use windows::core::{HRESULT, PCWSTR, PWSTR};
use windows::Win32::Foundation::ERROR_NOT_FOUND;
use windows::Win32::Security::Credentials::{
    CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE,
    CRED_TYPE_GENERIC,
};

const RIOT_API_TARGET: &str = "Aura/RiotApiKey";
const CREDENTIAL_USER: &str = "Aura";

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn is_not_found(error: &windows::core::Error) -> bool {
    error.code() == HRESULT::from_win32(ERROR_NOT_FOUND.0)
}

/// Loads Aura's Riot credential from Windows Credential Manager. The value is
/// returned only to Rust and is never serialized to the webview.
pub fn load_riot_api_key() -> Result<Option<String>, String> {
    let target = wide(RIOT_API_TARGET);
    let mut raw: *mut CREDENTIALW = std::ptr::null_mut();

    let read_result = unsafe {
        CredReadW(
            PCWSTR::from_raw(target.as_ptr()),
            CRED_TYPE_GENERIC,
            0,
            &mut raw,
        )
    };
    if let Err(error) = read_result {
        return if is_not_found(&error) {
            Ok(None)
        } else {
            Err(format!("Windows Credential Manager read failed: {error}"))
        };
    }
    if raw.is_null() {
        return Err("Windows Credential Manager returned an empty credential".into());
    }

    let result = unsafe {
        let credential = &*raw;
        if credential.CredentialBlob.is_null() || credential.CredentialBlobSize == 0 {
            Err("stored Riot credential is empty".to_string())
        } else {
            let bytes = std::slice::from_raw_parts(
                credential.CredentialBlob,
                credential.CredentialBlobSize as usize,
            );
            String::from_utf8(bytes.to_vec())
                .map(Some)
                .map_err(|_| "stored Riot credential is not valid UTF-8".to_string())
        }
    };
    unsafe { CredFree(raw.cast()) };
    result
}

/// Stores the Riot API key as a generic per-user Windows credential. Windows
/// protects this value; Aura never writes it to its project files or logs.
pub fn save_riot_api_key(api_key: &str) -> Result<(), String> {
    let mut target = wide(RIOT_API_TARGET);
    let mut username = wide(CREDENTIAL_USER);
    let mut blob = api_key.as_bytes().to_vec();

    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(target.as_mut_ptr()),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        UserName: PWSTR(username.as_mut_ptr()),
        ..Default::default()
    };

    unsafe { CredWriteW(&credential, 0) }
        .map_err(|error| format!("Windows Credential Manager write failed: {error}"))
}

pub fn delete_riot_api_key() -> Result<(), String> {
    let target = wide(RIOT_API_TARGET);
    match unsafe { CredDeleteW(PCWSTR::from_raw(target.as_ptr()), CRED_TYPE_GENERIC, 0) } {
        Ok(()) => Ok(()),
        Err(error) if is_not_found(&error) => Ok(()),
        Err(error) => Err(format!("Windows Credential Manager delete failed: {error}")),
    }
}
