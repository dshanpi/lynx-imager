use keyring::Entry;

const SERVICE: &str = "com.dshanpi.lynx.ai-supervisor";

fn service_for_namespace(namespace: Option<&str>, debug_build: bool) -> Result<String, String> {
    if !debug_build {
        return Ok(SERVICE.to_owned());
    }
    let Some(namespace) = namespace else {
        return Ok(SERVICE.to_owned());
    };
    if namespace.is_empty()
        || namespace.len() > 64
        || !namespace
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
    {
        return Err("AI_TEST_CREDENTIAL_NAMESPACE_INVALID".to_owned());
    }
    Ok(format!("{SERVICE}.test.{namespace}"))
}

fn service() -> Result<String, String> {
    #[cfg(debug_assertions)]
    let namespace = std::env::var("LYNX_TEST_SUPERVISOR_KEYRING_NAMESPACE").ok();
    #[cfg(not(debug_assertions))]
    let namespace: Option<String> = None;
    service_for_namespace(namespace.as_deref(), cfg!(debug_assertions))
}

fn entry(provider_id: i64) -> Result<Entry, String> {
    Entry::new(&crate::keyring_service(&service()?), &format!("ai-provider-{provider_id}"))
        .map_err(|error| format!("AI_CREDENTIAL_KEYRING:{error}"))
}

fn webhook_entry(task_id: i64) -> Result<Entry, String> {
    Entry::new(&crate::keyring_service(&service()?), &format!("automatic-webhook-{task_id}"))
        .map_err(|error| format!("SUPERVISOR_WEBHOOK_KEYRING:{error}"))
}

pub fn save(provider_id: i64, api_key: &str) -> Result<(), String> {
    let api_key = api_key.trim();
    if provider_id <= 0 || api_key.is_empty() {
        return Err("AI_CREDENTIAL_INVALID".to_owned());
    }
    entry(provider_id)?
        .set_password(api_key)
        .map_err(|error| format!("AI_CREDENTIAL_STORE:{error}"))
}

pub fn resolve(provider_id: i64) -> Result<String, String> {
    entry(provider_id)?
        .get_password()
        .map_err(|error| match error {
            keyring::Error::NoEntry => "AI_CREDENTIAL_MISSING".to_owned(),
            other => format!("AI_CREDENTIAL_READ:{other}"),
        })
}

pub fn exists(provider_id: i64) -> bool {
    resolve(provider_id).is_ok_and(|secret| !secret.is_empty())
}

pub fn delete(provider_id: i64) -> Result<(), String> {
    match entry(provider_id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(format!("AI_CREDENTIAL_DELETE:{error}")),
    }
}

pub fn save_webhook(task_id: i64, secret: &str) -> Result<(), String> {
    let secret = secret.trim();
    if task_id <= 0 || secret.chars().count() < 16 {
        return Err("SUPERVISOR_WEBHOOK_SECRET_TOO_SHORT".to_owned());
    }
    webhook_entry(task_id)?
        .set_password(secret)
        .map_err(|error| format!("SUPERVISOR_WEBHOOK_SECRET_STORE:{error}"))
}

pub fn resolve_webhook(task_id: i64) -> Result<String, String> {
    webhook_entry(task_id)?
        .get_password()
        .map_err(|error| match error {
            keyring::Error::NoEntry => "SUPERVISOR_WEBHOOK_SECRET_MISSING".to_owned(),
            other => format!("SUPERVISOR_WEBHOOK_SECRET_READ:{other}"),
        })
}

pub fn delete_webhook(task_id: i64) -> Result<(), String> {
    match webhook_entry(task_id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(format!("SUPERVISOR_WEBHOOK_SECRET_DELETE:{error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires an unlocked desktop OS keyring"]
    fn live_keyring_round_trip_and_delete() {
        let id = 8_000_000_000_i64 + (rand::random::<u32>() as i64);
        let value = format!("live-keyring-test-{}", rand::random::<u64>());
        save(id, &value).expect("store temporary credential");
        assert_eq!(resolve(id).expect("read temporary credential"), value);
        assert!(exists(id));
        delete(id).expect("delete temporary credential");
        assert_eq!(resolve(id).unwrap_err(), "AI_CREDENTIAL_MISSING");
    }
}

#[cfg(test)]
mod test_namespace_tests {
    use super::*;

    #[test]
    fn debug_fixture_service_is_distinct_from_real_and_other_test_profiles() {
        let left = service_for_namespace(Some("ui-check-a"), true).unwrap();
        let right = service_for_namespace(Some("ui-check-b"), true).unwrap();
        assert_ne!(left, SERVICE);
        assert_ne!(left, right);
        assert_eq!(left, "com.dshanpi.lynx.ai-supervisor.test.ui-check-a");
    }

    #[test]
    fn malformed_test_namespace_never_falls_back_to_real_credentials() {
        for value in ["", "../default", "a.b", "real/profile"] {
            assert!(service_for_namespace(Some(value), true).is_err());
        }
        assert!(service_for_namespace(Some(&"x".repeat(65)), true).is_err());
    }

    #[test]
    fn normal_debug_and_all_release_clients_preserve_real_service() {
        assert_eq!(service_for_namespace(None, true).unwrap(), SERVICE);
        assert_eq!(
            service_for_namespace(Some("ui-check"), false).unwrap(),
            SERVICE
        );
        assert_eq!(
            service_for_namespace(Some("../invalid"), false).unwrap(),
            SERVICE
        );
    }
}
