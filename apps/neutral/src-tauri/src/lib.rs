const SERVICE: &str = "works.byteowlz.foxline.connections";

fn account(id: &str) -> Result<String, String> {
    if id.is_empty() || id.len() > 64 || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') {
        return Err("Invalid connection identifier".into());
    }
    Ok(format!("connection-{id}"))
}

#[tauri::command]
fn save_connection_secret(id: String, secret: String) -> Result<(), String> {
    let account = account(&id)?;
    if secret.len() < 32 || secret.len() > 128 || !secret.is_ascii() {
        return Err("Invalid device credential".into());
    }
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    { security_framework::passwords::set_generic_password(SERVICE, &account, secret.as_bytes())
        .map_err(|_| "Could not save the connection in Apple Keychain".into()) }
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    { let _ = (account, secret); Err("Secure credential storage is not implemented on this platform".into()) }
}

#[tauri::command]
fn read_connection_secret(id: String) -> Result<Option<String>, String> {
    let account = account(&id)?;
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    { match security_framework::passwords::get_generic_password(SERVICE, &account) {
        Ok(bytes) => String::from_utf8(bytes).map(Some).map_err(|_| "Invalid stored credential".into()),
        Err(error) if error.code() == -25300 => Ok(None),
        Err(_) => Err("Could not read Apple Keychain; unlock the device and try again".into()),
    } }
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    { let _ = account; Err("Secure credential storage is not implemented on this platform".into()) }
}

#[tauri::command]
fn forget_connection_secret(id: String) -> Result<(), String> {
    let account = account(&id)?;
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    { match security_framework::passwords::delete_generic_password(SERVICE, &account) {
        Ok(()) => Ok(()),
        Err(error) if error.code() == -25300 => Ok(()),
        Err(_) => Err("Could not remove the Keychain credential".into()),
    } }
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    { let _ = account; Err("Secure credential storage is not implemented on this platform".into()) }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![save_connection_secret, read_connection_secret, forget_connection_secret])
        .run(tauri::generate_context!())
        .expect("Foxline app could not start");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credentials_are_namespaced_and_not_paths() {
        assert_eq!(account("abc-123").unwrap(), "connection-abc-123");
        for invalid in ["", "../secret", "other:account", "foo/bar"] { assert!(account(invalid).is_err()); }
    }
}
