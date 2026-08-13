use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, WindowEvent,
};

fn log_ok(msg: &str) {
    println!("[AURA::TRAY][OK] {}", msg);
}
fn log_err(msg: &str) {
    eprintln!("[AURA::TRAY][ERR] {}", msg);
}

pub fn init(app: &AppHandle) -> tauri::Result<()> {
    // Falls back to skipping tray creation (rest of the app still runs) rather
    // than a hard crash if the bundle icon is missing from tauri.conf.json.
    let icon = match app.default_window_icon() {
        Some(icon) => icon.clone(),
        None => {
            log_err(
                "no default window icon found (check tauri.conf.json bundle.icon) — skipping tray",
            );
            return Ok(());
        }
    };

    let show_dash = MenuItem::with_id(app, "show_dashboard", "Open Dashboard", true, None::<&str>)?;
    let toggle_over = MenuItem::with_id(
        app,
        "toggle_overlay",
        "Toggle HUD Overlay",
        true,
        None::<&str>,
    )?;
    // This native escape hatch remains clickable even while the overlay WebView
    // itself is locked and passing every pointer event through to the game.
    let toggle_interaction = MenuItem::with_id(
        app,
        "toggle_overlay_interaction",
        "Toggle HUD Interaction Lock",
        true,
        None::<&str>,
    )?;
    let quit_aura = MenuItem::with_id(app, "quit", "Exit Aura", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&show_dash, &toggle_over, &toggle_interaction, &quit_aura],
    )?;

    let _tray = TrayIconBuilder::new()
        .icon(icon)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            // Tauri v2 passes a TrayIcon here, not an AppHandle. Resolve the
            // application handle from the tray before accessing windows.
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app_handle = tray.app_handle();
                if let Some(win) = app_handle.get_webview_window("main") {
                    let _ = win.unminimize();
                    let _ = win.show();
                    let _ = win.set_focus();
                }
            }
        })
        .on_menu_event(|app_handle, event| match event.id.as_ref() {
            "show_dashboard" => {
                if let Some(win) = app_handle.get_webview_window("main") {
                    let _ = win.show();
                    let _ = win.set_focus();
                }
            }
            "toggle_overlay" => {
                if let Some(win) = app_handle.get_webview_window("overlay") {
                    match win.is_visible() {
                        Ok(true) => {
                            if let Err(error) = crate::overlay::hide(app_handle) {
                                log_err(&error);
                            }
                        }
                        Ok(false) => {
                            if let Err(error) = crate::overlay::show(app_handle) {
                                log_err(&error);
                            }
                        }
                        Err(e) => log_err(&format!("failed overlay visibility poll: {}", e)),
                    }
                } else if let Err(error) = crate::overlay::show(app_handle) {
                    log_err(&error);
                }
            }
            "toggle_overlay_interaction" => match crate::overlay::toggle_interaction(app_handle) {
                Ok(config) => log_ok(if config.locked {
                    "HUD interaction locked; clicks pass through to the game"
                } else {
                    "HUD interaction unlocked; overlay controls accept clicks"
                }),
                Err(error) => log_err(&format!("failed to toggle HUD interaction: {error}")),
            },
            "quit" => {
                log_ok("termination requested via context menu, cleaning up allocations");
                app_handle.exit(0);
            }
            _ => {}
        })
        .build(app)?;

    // Confirmed default Tauri behavior: closing the last window exits the whole
    // process unless CloseRequested is intercepted. Without this, the X button
    // kills the tray icon and all background loops with it — only "Exit Aura"
    // should actually terminate the app.
    if let Some(main_window) = app.get_webview_window("main") {
        let main_window_clone = main_window.clone();
        main_window.on_window_event(move |event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                match main_window_clone.hide() {
                    Ok(_) => log_ok("dashboard close intercepted, hidden to tray"),
                    Err(e) => log_err(&format!("hide-on-close failed: {}", e)),
                }
            }
        });
    } else {
        log_err("main window not found — close-to-tray not registered");
    }

    log_ok("system tray manager mounted");
    Ok(())
}
