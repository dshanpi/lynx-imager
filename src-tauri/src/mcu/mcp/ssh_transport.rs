//! MCU tunnel SSH transport.
//!
//! This module owns only authentication and creation of an opaque SSH byte
//! transport. It deliberately has no dependency on Linux Workbench host,
//! project, credential, tool-router, or tunnel-lifecycle types.

use std::{
    net::{TcpStream, ToSocketAddrs},
    path::{Path, PathBuf},
    time::Duration,
};

use ssh2::{HashType, Session};

#[derive(Clone)]
pub(super) struct RemoteTarget {
    address: String,
    port: u16,
    username: String,
    auth_type: AuthType,
    private_key_path: String,
    host_fingerprint: String,
    password: Option<String>,
    key_passphrase: Option<String>,
}

pub(super) struct RemoteConnectionConfig {
    pub address: String,
    pub port: u16,
    pub username: String,
    pub auth_type: String,
    pub private_key_path: String,
    pub host_fingerprint: String,
}

#[derive(Clone, Copy)]
enum AuthType {
    PrivateKey,
    Password,
}

pub(super) fn configured_target(
    config: RemoteConnectionConfig,
    password: Option<String>,
    key_passphrase: Option<String>,
) -> Result<RemoteTarget, String> {
    if config.address.trim().is_empty() || config.username.trim().is_empty() || config.port == 0 {
        return Err("MCU_SSH_CONFIG:主机地址、SSH 端口和用户名不能为空".to_owned());
    }
    let auth_type = match config.auth_type.as_str() {
        "password" => AuthType::Password,
        "private_key" => AuthType::PrivateKey,
        _ => return Err("MCU_SSH_CONFIG:认证方式必须是 password 或 private_key".to_owned()),
    };
    Ok(RemoteTarget {
        address: config.address,
        port: config.port,
        username: config.username,
        auth_type,
        private_key_path: config.private_key_path,
        host_fingerprint: config.host_fingerprint,
        password,
        key_passphrase,
    })
}

pub(super) fn connect(target: &RemoteTarget) -> Result<Session, String> {
    let _native_guard = crate::diagnostics::begin_native_ssh_operation("mcu_ssh_connect");
    let address = format!("{}:{}", target.address, target.port);
    let socket_address = address
        .to_socket_addrs()
        .map_err(|error| format!("MCU_SSH_ADDRESS:{error}"))?
        .next()
        .ok_or_else(|| "MCU_SSH_ADDRESS:no address resolved".to_owned())?;
    let tcp = TcpStream::connect_timeout(&socket_address, Duration::from_secs(10))
        .map_err(|error| format!("MCU_SSH_CONNECT:{error}"))?;
    tcp.set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|error| format!("MCU_SSH_SOCKET:{error}"))?;
    tcp.set_write_timeout(Some(Duration::from_secs(10)))
        .map_err(|error| format!("MCU_SSH_SOCKET:{error}"))?;

    let mut session = Session::new().map_err(|error| format!("MCU_SSH_SESSION:{error}"))?;
    session.set_tcp_stream(tcp);
    session
        .handshake()
        .map_err(|error| format!("MCU_SSH_HANDSHAKE:{error}"))?;

    let fingerprint = session
        .host_key_hash(HashType::Sha256)
        .map(hex_fingerprint)
        .ok_or_else(|| "MCU_SSH_FINGERPRINT:server did not provide a host key".to_owned())?;
    verify_fingerprint(&target.host_fingerprint, &fingerprint)?;

    match target.auth_type {
        AuthType::PrivateKey => authenticate_with_available_key(&mut session, target)?,
        AuthType::Password => {
            let password = target
                .password
                .as_deref()
                .ok_or_else(|| "MCU_SSH_AUTH:password is required".to_owned())?;
            session
                .userauth_password(&target.username, password)
                .map_err(|error| format!("MCU_SSH_AUTH:{error}"))?;
        }
    }
    if !session.authenticated() {
        return Err("MCU_SSH_AUTH:authentication rejected".to_owned());
    }
    Ok(session)
}

fn verify_fingerprint(expected: &str, actual: &str) -> Result<(), String> {
    if expected.is_empty() {
        // Preserve the current MCU settings flow, which does not yet expose a
        // separate trust-host action. A saved fingerprint, when present, is
        // always enforced below.
        return Ok(());
    }
    if !constant_time_equal(expected, actual) {
        return Err(format!(
            "MCU_HOST_KEY_CHANGED:expected={expected},actual={actual}"
        ));
    }
    Ok(())
}

fn authenticate_with_available_key(
    session: &mut Session,
    target: &RemoteTarget,
) -> Result<(), String> {
    if session.userauth_agent(&target.username).is_ok() && session.authenticated() {
        return Ok(());
    }

    let mut candidates = Vec::new();
    if !target.private_key_path.trim().is_empty() {
        candidates.push(PathBuf::from(target.private_key_path.trim()));
    }
    if let Some(user_dir) = std::env::var_os("HOME").map(PathBuf::from) {
        for file_name in ["id_ed25519", "id_ecdsa", "id_rsa"] {
            candidates.push(user_dir.join(".ssh").join(file_name));
        }
    }
    for candidate in candidates {
        if !candidate.is_file() {
            continue;
        }
        if session
            .userauth_pubkey_file(
                &target.username,
                None,
                Path::new(&candidate),
                target.key_passphrase.as_deref(),
            )
            .is_ok()
            && session.authenticated()
        {
            return Ok(());
        }
    }
    Err("MCU_SSH_AUTH:未发现可用的 SSH Agent 或私钥".to_owned())
}

fn hex_fingerprint(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

fn constant_time_equal(left: &str, right: &str) -> bool {
    left.len() == right.len()
        && left
            .bytes()
            .zip(right.bytes())
            .fold(0u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_auth_type() {
        let result = configured_target(
            RemoteConnectionConfig {
                address: "127.0.0.1".to_owned(),
                port: 22,
                username: "ubuntu".to_owned(),
                auth_type: "linux_magic".to_owned(),
                private_key_path: String::new(),
                host_fingerprint: String::new(),
            },
            None,
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn fingerprint_comparison_is_exact() {
        assert!(constant_time_equal("aa:bb", "aa:bb"));
        assert!(!constant_time_equal("aa:bb", "aa:bc"));
        assert!(!constant_time_equal("aa", "aa:bb"));
    }

    #[test]
    fn empty_fingerprint_preserves_current_first_connect_flow() {
        assert!(verify_fingerprint("", "aa:bb").is_ok());
        assert!(verify_fingerprint("aa:bb", "aa:bc").is_err());
    }
}
