//! Cross-platform Workbench secret storage backed by the operating-system keyring.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use keyring::Entry;
use std::sync::Arc;
use tauri::{command, AppHandle, State};

use super::types::SshCredentials;

const SERVICE: &str = "com.dshanpi.lynx.workbench";
static SESSION_CREDENTIALS: OnceLock<Mutex<HashMap<i64, SshCredentials>>> = OnceLock::new();

#[command]
pub fn workbench_save_credentials(
    app: AppHandle,
    state: State<'_, Arc<super::WorkbenchState>>,
    host_id: i64,
    credentials: SshCredentials,
) -> Result<(), String> {
    if host_id <= 0 || credentials.is_empty() {
        return Err("CREDENTIALS_INVALID:host and at least one secret are required".to_owned());
    }
    {
        let db = state.db.lock().map_err(|_| "WORKBENCH_STORE_UNAVAILABLE")?;
        super::storage::host_by_id(&db, host_id).map_err(|_| "SSH_HOST_NOT_FOUND")?;
    }
    let audit = crate::config_audit::begin(
        &app,
        "ssh_host",
        format!("ssh-host:{host_id}"),
        "ssh_credential",
        "update",
        vec!["credential".to_owned()],
        serde_json::json!({"credentialChanged": true}),
    )?;
    let result = (|| {
        let value =
            serde_json::to_string(&credentials).map_err(|e| format!("CREDENTIALS_ENCODE:{e}"))?;
        entry(host_id)?
            .set_password(&value)
            .map_err(|e| format!("CREDENTIALS_STORE:{e}"))?;
        cache_credentials(host_id, credentials);
        Ok(())
    })();
    crate::config_audit::finish(&app, &audit, &result);
    result
}

#[command]
pub fn workbench_delete_credentials(app: AppHandle, host_id: i64) -> Result<(), String> {
    let audit = crate::config_audit::begin(
        &app,
        "ssh_host",
        format!("ssh-host:{host_id}"),
        "ssh_credential",
        "delete",
        vec!["credential".to_owned()],
        serde_json::json!({"credentialChanged": true}),
    )?;
    let result = match entry(host_id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(format!("CREDENTIALS_DELETE:{error}")),
    };
    if result.is_ok() {
        if let Ok(mut cache) = session_credentials().lock() {
            cache.remove(&host_id);
        }
    }
    crate::config_audit::finish(&app, &audit, &result);
    result
}

pub fn resolve(host_id: Option<i64>, supplied: SshCredentials) -> Result<SshCredentials, String> {
    // A password-authenticated host and a private-key-authenticated host each
    // need only one secret. Never unlock the OS keyring when the caller has
    // already supplied the credential required for this operation.
    if !supplied.is_empty() {
        return Ok(supplied);
    }
    let Some(host_id) = host_id else {
        return Ok(supplied);
    };
    if let Some(cached) = cached_credentials(host_id) {
        return Ok(cached);
    }
    let Ok(entry) = entry(host_id) else {
        return Ok(supplied);
    };
    let stored = match entry.get_password() {
        Ok(value) => serde_json::from_str::<SshCredentials>(&value)
            .map_err(|e| format!("CREDENTIALS_DECODE:{e}"))?,
        // Headless Linux sessions may not have a Secret Service. In that case
        // retain the supplied ephemeral values and let SSH authentication
        // report whether a secret was actually required.
        Err(_) => return Ok(supplied),
    };
    let resolved = SshCredentials {
        password: non_empty(supplied.password).or(stored.password),
        key_passphrase: non_empty(supplied.key_passphrase).or(stored.key_passphrase),
    };
    cache_credentials(host_id, resolved.clone());
    Ok(resolved)
}

fn entry(host_id: i64) -> Result<Entry, String> {
    Entry::new(SERVICE, &format!("ssh-host-{host_id}"))
        .map_err(|e| format!("CREDENTIALS_KEYRING:{e}"))
}

fn serial_login_entry(device_id: i64) -> Result<Entry, String> {
    Entry::new(SERVICE, &format!("serial-login-{device_id}"))
        .map_err(|e| format!("SERIAL_LOGIN_KEYRING:{e}"))
}

#[command]
pub fn workbench_serial_login_password_status(
    state: State<'_, Arc<super::WorkbenchState>>,
    device_id: i64,
) -> Result<bool, String> {
    ensure_device(&state, device_id)?;
    Ok(serial_login_entry(device_id)?
        .get_password()
        .is_ok_and(|value| !value.is_empty()))
}

#[command]
pub fn workbench_save_serial_login_password(
    app: AppHandle,
    state: State<'_, Arc<super::WorkbenchState>>,
    device_id: i64,
    password: Option<String>,
) -> Result<(), String> {
    ensure_device(&state, device_id)?;
    let operation = if password.as_ref().is_some_and(|value| !value.is_empty()) {
        "update"
    } else {
        "delete"
    };
    let audit = crate::config_audit::begin(
        &app,
        "linux_board",
        format!("linux:{device_id}"),
        "serial_login_credential",
        operation,
        vec!["serialLoginCredential".to_owned()],
        serde_json::json!({"credentialChanged": true}),
    )?;
    let result = match password.filter(|value| !value.is_empty()) {
        Some(password) => serial_login_entry(device_id)?
            .set_password(&password)
            .map_err(|e| format!("SERIAL_LOGIN_CREDENTIAL_STORE:{e}")),
        None => match serial_login_entry(device_id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(format!("SERIAL_LOGIN_CREDENTIAL_DELETE:{error}")),
        },
    };
    crate::config_audit::finish(&app, &audit, &result);
    result
}

pub(crate) fn resolve_serial_login_password(device_id: i64) -> Result<Option<String>, String> {
    match serial_login_entry(device_id)?.get_password() {
        Ok(value) if !value.is_empty() => Ok(Some(value)),
        Ok(_) | Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(format!("SERIAL_LOGIN_CREDENTIAL_READ:{error}")),
    }
}

pub(crate) fn delete_serial_login_password(device_id: i64) -> Result<(), String> {
    match serial_login_entry(device_id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(format!("SERIAL_LOGIN_CREDENTIAL_DELETE:{error}")),
    }
}

fn ensure_device(state: &super::WorkbenchState, device_id: i64) -> Result<(), String> {
    if device_id <= 0 {
        return Err("SERIAL_LOGIN_DEVICE_INVALID".to_owned());
    }
    let db = state.db.lock().map_err(|_| "WORKBENCH_STORE_UNAVAILABLE")?;
    super::storage::device_by_id(&db, device_id)
        .map(|_| ())
        .map_err(|_| "SERIAL_LOGIN_DEVICE_NOT_FOUND".to_owned())
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|item| !item.is_empty())
}

fn session_credentials() -> &'static Mutex<HashMap<i64, SshCredentials>> {
    SESSION_CREDENTIALS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cache_credentials(host_id: i64, credentials: SshCredentials) {
    if let Ok(mut cache) = session_credentials().lock() {
        cache.insert(host_id, credentials);
    }
}

fn cached_credentials(host_id: i64) -> Option<SshCredentials> {
    session_credentials()
        .lock()
        .ok()
        .and_then(|cache| cache.get(&host_id).cloned())
}
