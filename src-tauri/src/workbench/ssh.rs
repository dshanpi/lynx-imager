use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::{
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD},
    Engine as _,
};
use ssh2::{HashType, Session};

use super::types::{HostConfig, SshAuthType, SshCredentials};

/// Connect, verify the server fingerprint, and authenticate.
pub fn connect(
    host: &HostConfig,
    credentials: &SshCredentials,
) -> Result<(Session, String, bool), String> {
    connect_with_policy(host, credentials, HostKeyPolicy::Strict)
}

/// User-opened Shell only: do not block on a saved SDK pin or update that pin.
/// SDK automation, MCP and transfers must continue using `connect`.
pub(super) fn connect_interactive_terminal(
    host: &HostConfig,
    credentials: &SshCredentials,
) -> Result<(Session, String, bool), String> {
    connect_with_policy(host, credentials, HostKeyPolicy::InteractiveTerminal)
}

#[derive(Clone, Copy)]
enum HostKeyPolicy {
    Strict,
    InteractiveTerminal,
}

impl HostKeyPolicy {
    fn verify(self, saved: &str, actual: &str) -> Result<bool, String> {
        match self {
            Self::Strict => {
                verify_host_fingerprint(saved, actual)?;
                Ok(true)
            }
            Self::InteractiveTerminal => Ok(constant_time_equal(saved, actual)),
        }
    }
}

fn connect_with_policy(
    host: &HostConfig,
    credentials: &SshCredentials,
    policy: HostKeyPolicy,
) -> Result<(Session, String, bool), String> {
    let _native_guard = crate::diagnostics::begin_native_ssh_operation("ssh_connect");
    let address = format!("{}:{}", host.address, host.port);
    let socket_address = address
        .to_socket_addrs()
        .map_err(|e| format!("SSH_ADDRESS:{e}"))?
        .next()
        .ok_or_else(|| "SSH_ADDRESS:no address resolved".to_owned())?;
    let tcp = TcpStream::connect_timeout(&socket_address, Duration::from_secs(10))
        .map_err(|e| format!("SSH_CONNECT:{e}"))?;
    tcp.set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|e| e.to_string())?;
    tcp.set_write_timeout(Some(Duration::from_secs(10)))
        .map_err(|e| e.to_string())?;

    let mut session = Session::new().map_err(|e| format!("SSH_SESSION:{e}"))?;
    session.set_tcp_stream(tcp);
    session
        .handshake()
        .map_err(|e| format!("SSH_HANDSHAKE:{e}"))?;

    let fingerprint = session
        .host_key_hash(HashType::Sha256)
        .map(|bytes| format!("SHA256:{}", STANDARD_NO_PAD.encode(bytes)))
        .ok_or_else(|| "SSH_FINGERPRINT:server did not provide a host key".to_owned())?;
    let trusted = policy.verify(&host.host_fingerprint, &fingerprint)?;

    match host.auth_type {
        SshAuthType::PrivateKey => {
            authenticate_with_available_key(&mut session, host, credentials)?;
        }
        SshAuthType::Password => {
            let password = credentials
                .password
                .as_deref()
                .ok_or_else(|| "SSH_AUTH:password is required".to_owned())?;
            session
                .userauth_password(&host.username, password)
                .map_err(|e| format!("SSH_AUTH:{e}"))?;
        }
    }
    if !session.authenticated() {
        return Err("SSH_AUTH:authentication rejected".to_owned());
    }
    Ok((session, fingerprint, trusted))
}

fn authenticate_with_available_key(
    session: &mut Session,
    host: &HostConfig,
    credentials: &SshCredentials,
) -> Result<(), String> {
    if session.userauth_agent(&host.username).is_ok() && session.authenticated() {
        return Ok(());
    }

    let mut candidates = Vec::new();
    if !host.private_key_path.trim().is_empty() {
        candidates.push(PathBuf::from(host.private_key_path.trim()));
    }
    if let Some(user_dir) = std::env::var_os("HOME").map(PathBuf::from) {
        for file_name in ["id_ed25519", "id_ecdsa", "id_rsa"] {
            candidates.push(user_dir.join(".ssh").join(file_name));
        }
    }

    let mut attempted_count = 0usize;
    for candidate in candidates {
        if !candidate.is_file() {
            continue;
        }
        attempted_count += 1;
        if session
            .userauth_pubkey_file(
                &host.username,
                None,
                Path::new(&candidate),
                credentials.key_passphrase.as_deref(),
            )
            .is_ok()
            && session.authenticated()
        {
            return Ok(());
        }
    }

    let detail = if attempted_count == 0 {
        "未发现 SSH Agent 或 ~/.ssh/id_ed25519、id_ecdsa、id_rsa".to_owned()
    } else {
        format!("自动尝试 {attempted_count} 个可用密钥失败")
    };
    Err(format!("SSH_AUTH:{detail}"))
}

fn verify_host_fingerprint(saved: &str, actual: &str) -> Result<(), String> {
    if saved.trim().is_empty() {
        return Err(format!("HOST_KEY_CONFIRM_REQUIRED:{actual}"));
    }
    if !constant_time_equal(saved, actual) {
        return Err(format!("HOST_KEY_CHANGED:expected={saved},actual={actual}"));
    }
    Ok(())
}

fn fingerprint_bytes(value: &str) -> Option<[u8; 32]> {
    let value = value.trim();
    let bytes = if let Some(encoded) = value.strip_prefix("SHA256:") {
        STANDARD_NO_PAD
            .decode(encoded)
            .or_else(|_| STANDARD.decode(encoded))
            .ok()?
    } else {
        let groups: Vec<_> = value.split(':').collect();
        if groups.len() != 32 || groups.iter().any(|group| group.len() != 2) {
            return None;
        }
        hex::decode(groups.concat()).ok()?
    };
    bytes.try_into().ok()
}

fn constant_time_equal(left: &str, right: &str) -> bool {
    let (Some(left), Some(right)) = (fingerprint_bytes(left), fingerprint_bytes(right)) else {
        return false;
    };
    left.iter()
        .zip(right.iter())
        .fold(0u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

/// Quote one literal value for a POSIX shell.
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interactive_shell_does_not_block_or_promote_changed_keys() {
        let actual = "SHA256:6wBcfry4NFRmKfhyCnpbQ08bArUUl9b/lhu1EPjZzY8";
        for saved in [
            "",
            "invalid",
            "SHA256:Jt0fvBVTjJsDOOcuQ1p8CfcMsQAeZVToLanNbK4K45k",
        ] {
            assert!(HostKeyPolicy::Strict.verify(saved, actual).is_err());
            assert_eq!(
                HostKeyPolicy::InteractiveTerminal.verify(saved, actual),
                Ok(false)
            );
        }
        assert_eq!(HostKeyPolicy::Strict.verify(actual, actual), Ok(true));
        assert_eq!(
            HostKeyPolicy::InteractiveTerminal.verify(actual, actual),
            Ok(true)
        );
    }

    #[test]
    fn sha256_fingerprint_formats_compare_digest_bytes() {
        let legacy = "eb:00:5c:7e:bc:b8:34:54:66:29:f8:72:0a:7a:5b:43:4f:1b:02:b5:14:97:d6:ff:96:1b:b5:10:f8:d9:cd:8f";
        let openssh = "SHA256:6wBcfry4NFRmKfhyCnpbQ08bArUUl9b/lhu1EPjZzY8";
        assert!(constant_time_equal(legacy, openssh));
        assert!(constant_time_equal(&legacy.to_uppercase(), openssh));
        assert!(constant_time_equal(openssh, &format!("{openssh}=")));
        assert!(!constant_time_equal(
            "SHA256:Jt0fvBVTjJsDOOcuQ1p8CfcMsQAeZVToLanNbK4K45k",
            openssh
        ));
        assert!(!constant_time_equal("", ""));
        assert!(!constant_time_equal("malformed", "malformed"));
        assert!(!constant_time_equal("SHA256:AA", "SHA256:AA"));
    }

    #[test]
    fn unknown_changed_and_malformed_keys_fail_before_authentication() {
        let key = "SHA256:6wBcfry4NFRmKfhyCnpbQ08bArUUl9b/lhu1EPjZzY8";
        assert_eq!(
            verify_host_fingerprint("", key),
            Err(format!("HOST_KEY_CONFIRM_REQUIRED:{key}"))
        );
        for saved in [
            "invalid",
            "SHA256:Jt0fvBVTjJsDOOcuQ1p8CfcMsQAeZVToLanNbK4K45k",
        ] {
            assert_eq!(
                verify_host_fingerprint(saved, key),
                Err(format!("HOST_KEY_CHANGED:expected={saved},actual={key}"))
            );
        }
        assert_eq!(verify_host_fingerprint(key, key), Ok(()));
    }

    #[test]
    fn quotes_shell_literals() {
        assert_eq!(shell_quote("/tmp/a b"), "'/tmp/a b'");
        assert_eq!(shell_quote("it's"), "'it'\"'\"'s'");
    }
}
