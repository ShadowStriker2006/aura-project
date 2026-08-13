// Self-throttle: when League is detected running, Aura drops its OWN priority
// and trims its OWN working set. Never touches League's process — get out of
// its way, don't fight it for scheduling.

use std::time::Duration;
use tokio::time::sleep;
use windows::Win32::Foundation::{CloseHandle, GetLastError};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::ProcessStatus::K32EmptyWorkingSet;
use windows::Win32::System::Threading::{
    GetCurrentProcess, SetPriorityClass, BELOW_NORMAL_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS,
};

#[derive(Debug)]
pub enum PerfError {
    ProcessNotFound(String),
    SnapshotFailed(u32),
    SetPriorityFailed(u32, u32),
    TrimFailed(u32, u32),
}

impl std::fmt::Display for PerfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PerfError::ProcessNotFound(name) => write!(f, "target '{}' not running", name),
            PerfError::SnapshotFailed(code) => {
                write!(f, "toolhelp snapshot failed, os_err={}", code)
            }
            PerfError::SetPriorityFailed(pid, code) => {
                write!(f, "SetPriorityClass failed pid={} os_err={}", pid, code)
            }
            PerfError::TrimFailed(pid, code) => {
                write!(f, "EmptyWorkingSet failed pid={} os_err={}", pid, code)
            }
        }
    }
}
impl std::error::Error for PerfError {}

fn log_ok(msg: &str) {
    println!("[AURA::PERF][OK] {}", msg);
}
fn log_err(msg: &str) {
    eprintln!("[AURA::PERF][ERR] {}", msg);
}

/// Scans running processes for `target` (case-insensitive exe name). Returns its PID if found.
///
/// FIXED: previously used `.trim_end_matches('\0')` on the whole fixed-size
/// szExeFile buffer, which only strips trailing nulls. Since the same buffer
/// is reused across loop iterations, a shorter name overwriting a longer
/// previous one can leave stale characters *after* the real null terminator —
/// trim_end does nothing for that case. This now truncates at the first null
/// byte, matching proper C-string semantics.
pub fn find_pid_by_name(target: &str) -> Result<u32, PerfError> {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)
            .map_err(|_| PerfError::SnapshotFailed(GetLastError().0))?;

        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };

        let mut found: Option<u32> = None;
        let target_lower = target.to_lowercase();

        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let null_pos = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let exe_name =
                    String::from_utf16_lossy(&entry.szExeFile[..null_pos]).to_lowercase();

                if exe_name == target_lower {
                    found = Some(entry.th32ProcessID);
                    break;
                }
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }

        let _ = CloseHandle(snapshot);
        found.ok_or_else(|| PerfError::ProcessNotFound(target.to_string()))
    }
}

/// Drops THIS process to BELOW_NORMAL. Called once League is confirmed running.
pub fn deprioritize_self() -> Result<(), PerfError> {
    unsafe {
        SetPriorityClass(GetCurrentProcess(), BELOW_NORMAL_PRIORITY_CLASS)
            .map_err(|_| PerfError::SetPriorityFailed(std::process::id(), GetLastError().0))
    }
}

/// Restores THIS process to NORMAL once League exits, so the dashboard stays snappy.
pub fn restore_self() -> Result<(), PerfError> {
    unsafe {
        SetPriorityClass(GetCurrentProcess(), NORMAL_PRIORITY_CLASS)
            .map_err(|_| PerfError::SetPriorityFailed(std::process::id(), GetLastError().0))
    }
}

/// Forces THIS process to hand unused RAM pages back to the OS.
///
/// CORRECTED: confirmed directly against windows-rs's own generated docs
/// (microsoft.github.io/windows-docs-rs) that K32EmptyWorkingSet returns raw
/// `BOOL`, not `Result<()>` — the earlier assumption that it followed the
/// same Result-ified convention as Process32FirstW/NextW was wrong for this
/// specific function. Uses `.as_bool()` accordingly.
pub fn purge_own_working_set() -> Result<(), PerfError> {
    unsafe {
        if K32EmptyWorkingSet(GetCurrentProcess()).as_bool() {
            Ok(())
        } else {
            Err(PerfError::TrimFailed(std::process::id(), GetLastError().0))
        }
    }
}

/// Background watch loop. Interval is caller's call — the caller controls the low-frequency match interval.
///
/// FIXED: previously flipped `engaged` regardless of whether the OS call
/// actually succeeded. If SetPriorityClass failed once (permissions hiccup,
/// etc.), the loop believed it was already throttled/restored and would
/// never retry, since it only acts on state *transitions*. Now `engaged`
/// only flips inside the Ok arm, so a failed attempt gets retried next tick.
pub async fn run_guard_loop(watch_for: &'static str, interval_secs: u64) {
    log_ok(&format!("guard loop started, watching for '{}'", watch_for));
    let mut engaged = false;

    loop {
        match find_pid_by_name(watch_for) {
            Ok(pid) if !engaged => {
                match deprioritize_self() {
                    Ok(_) => {
                        log_ok(&format!(
                            "game detected (pid={}), self dropped to BELOW_NORMAL",
                            pid
                        ));
                        if let Err(e) = purge_own_working_set() {
                            log_err(&format!("one-time working-set trim failed: {}", e));
                        } else {
                            log_ok("one-time working-set trim completed");
                        }
                        engaged = true;
                    }
                    Err(e) => log_err(&e.to_string()), // engaged stays false, retried next tick
                }
            }
            Err(_) if engaged => {
                match restore_self() {
                    Ok(_) => {
                        log_ok("game exited, self restored to NORMAL");
                        engaged = false;
                    }
                    Err(e) => log_err(&e.to_string()), // stays engaged, retried next tick
                }
            }
            _ => {}
        }

        sleep(Duration::from_secs(interval_secs)).await;
    }
}
