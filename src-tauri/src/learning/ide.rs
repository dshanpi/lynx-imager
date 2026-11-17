//! Shared IDE target validation. Never executes SSH configuration or curriculum text.
use crate::workbench;
use std::io::Read;

fn valid_alias(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.starts_with('-')
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}
fn project_path(value: &str) -> bool {
    value.starts_with('/')
        && value.len() <= 4096
        && !value.chars().any(char::is_control)
        && !value.split('/').any(|p| p == "..")
}

// Resolve a simple explicit OpenSSH alias without executing Match exec or ssh -G.
pub(super) fn alias_matches(
    config: &str,
    alias: &str,
    host: &workbench::types::HostConfig,
) -> bool {
    let mut active = true;
    let mut found = false;
    let mut address = None;
    let mut user = None;
    let mut port = None;
    for line in config.lines() {
        let parts: Vec<_> = line
            .split('#')
            .next()
            .unwrap_or_default()
            .split_whitespace()
            .collect();
        if parts.is_empty() {
            continue;
        }
        let key = parts[0].to_ascii_lowercase();
        if key == "include" || key == "match" {
            return false;
        }
        if key == "host" {
            // Complex/wildcard stanzas may change the effective target. Do not guess.
            if parts[1..].iter().any(|p| p.contains(['*', '?', '!'])) {
                return false;
            }
            active = parts[1..].contains(&alias);
            found |= active;
            continue;
        }
        if !active {
            continue;
        }
        if parts.len() != 2 {
            return false;
        }
        let v = parts[1].trim_matches('"');
        match key.as_str() {
            "hostname" => {
                address.get_or_insert(v);
            }
            "user" => {
                user.get_or_insert(v);
            }
            "port" => {
                port.get_or_insert(v);
            }
            _ => {}
        }
    }
    found
        && address == Some(host.address.as_str())
        && user == Some(host.username.as_str())
        && match port {
            None => host.port == 22,
            Some(value) => value.parse::<u16>().ok().filter(|p| *p > 0) == Some(host.port),
        }
}
pub(super) fn ide_url(
    host: &workbench::types::HostConfig,
    root: &str,
    alias: &str,
    file: &str,
) -> Result<String, String> {
    if !project_path(root)
        || file.starts_with('/')
        || file.split('/').any(|p| p == "..")
        || file.chars().any(char::is_control)
        || file.len() > 2048
    {
        return Err("LEARN_IDE_PATH_INVALID".into());
    }
    let authority = if !alias.is_empty() {
        if !valid_alias(alias) {
            return Err("LEARN_IDE_ALIAS_INVALID".into());
        }
        alias.into()
    } else {
        if host.port != 22 || !valid_alias(&host.username) || !valid_alias(&host.address) {
            return Err(
                "LEARN_IDE_ALIAS_REQUIRED:请配置与当前主机、用户名和端口匹配的 SSH 别名".into(),
            );
        }
        format!("{}@{}", host.username, host.address)
    };
    let path = if file.is_empty() {
        root.to_string()
    } else {
        // VS Code's remote protocol treats a plain path as a folder. An explicit
        // line suffix selects its file route (also for extensionless filenames).
        format!("{}/{}:1", root.trim_end_matches('/'), file)
    };
    let mut uri = url::Url::parse("vscode://vscode-remote").map_err(|_| "LEARN_IDE_URI_INVALID")?;
    {
        let mut segments = uri
            .path_segments_mut()
            .map_err(|_| "LEARN_IDE_URI_INVALID")?;
        segments.clear().push(&format!("ssh-remote+{authority}"));
        for segment in path.trim_start_matches('/').split('/') {
            segments.push(segment);
        }
    }
    Ok(uri.into())
}

/// Validate the default local config, without invoking ssh -G / Match exec.
pub(super) fn checked_url(
    host: &workbench::types::HostConfig,
    root: &str,
    alias: &str,
    relative_file: &str,
) -> Result<String, String> {
    let uri = ide_url(host, root, alias, relative_file)?;
    if !alias.is_empty() {
        let home = if cfg!(windows) {
            std::env::var_os("USERPROFILE")
        } else {
            std::env::var_os("HOME")
        }
        .ok_or("LEARN_IDE_CONFIG_MISSING:未找到本机用户目录")?;
        let file = std::fs::File::open(std::path::PathBuf::from(home).join(".ssh/config"))
            .map_err(|_| "LEARN_IDE_CONFIG_MISSING:请先在本机 ~/.ssh/config 保存 SSH 别名")?;
        checked_config(file, alias, host)?;
    }
    Ok(uri)
}

fn checked_config(
    reader: impl Read,
    alias: &str,
    host: &workbench::types::HostConfig,
) -> Result<(), String> {
    let mut config = String::new();
    reader
        .take(262145)
        .read_to_string(&mut config)
        .map_err(|_| "LEARN_IDE_CONFIG_INVALID:SSH 配置无法读取或不是 UTF-8")?;
    if config.len() > 262144 || !alias_matches(&config, alias, host) {
        return Err("LEARN_IDE_ALIAS_MISMATCH:SSH 别名须明确匹配已绑定主机、用户和端口；Include/Match/通配配置请先简化并核对".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ide_file_link_is_identified_as_file_by_vscode() {
        assert!(ide_url(&host(), "/sdk", "lesson", "src/main.c")
            .unwrap()
            .ends_with("/main.c:1"));
        assert!(!ide_url(&host(), "/sdk", "lesson", "")
            .unwrap()
            .ends_with(":1"));
    }
    fn host() -> workbench::types::HostConfig {
        serde_json::from_value(serde_json::json!({"id":1,"name":"test","address":"127.0.0.1","port":2222,"username":"student","authType":"password","privateKeyPath":"","hostFingerprint":"SHA256:test","defaultWorkDir":"/tmp"})).unwrap()
    }
    #[test]
    fn ide_custom_port_alias_and_literal_file_data() {
        let h = host();
        assert!(ide_url(&h, "/sdk", "", "")
            .unwrap_err()
            .starts_with("LEARN_IDE_ALIAS_REQUIRED"));
        assert_eq!(
            ide_url(&h, "/sdk project", "lesson", "src/a%2e#?.c").unwrap(),
            "vscode://vscode-remote/ssh-remote+lesson/sdk%20project/src/a%252e%23%3F.c:1"
        );
        let uri = ide_url(&h, "/sdk", "lesson", "示例.c").unwrap();
        assert!(uri.ends_with("/%E7%A4%BA%E4%BE%8B.c:1"));
    }
    #[test]
    fn ide_paths_reject_traversal_controls_and_excessive_input() {
        let h = host();
        for file in ["/etc/passwd", "../secret", "src/../../secret", "src/a\n.c"] {
            assert_eq!(
                ide_url(&h, "/sdk", "lesson", file).unwrap_err(),
                "LEARN_IDE_PATH_INVALID"
            );
        }
        for root in ["relative", "/sdk/../private", "/sdk\n"] {
            assert!(ide_url(&h, root, "lesson", "").is_err());
        }
        assert!(ide_url(&h, "/sdk", "lesson", &"a".repeat(2049)).is_err());
        assert!(ide_url(&h, "/sdk", "-oProxyCommand=bad", "").is_err());
    }
    #[test]
    fn ide_config_is_bounded_and_requires_exact_target() {
        let h = host();
        let valid = "Host lesson\n HostName 127.0.0.1\n User student\n Port 2222\n";
        checked_config(valid.as_bytes(), "lesson", &h).unwrap();
        for bad in [
            valid.replace("2222", "invalid"),
            valid.replace("2222", "0"),
            valid.replace("2222", "65536"),
            valid.replace("student", "root"),
            valid.replace("127.0.0.1", "other"),
            format!("{valid}Include other\n"),
            format!("{valid}Match exec bad\n"),
            format!("{valid}Host *\n"),
            "x".repeat(262145),
        ] {
            assert!(checked_config(bad.as_bytes(), "lesson", &h).is_err());
        }
        assert!(checked_config(&[0xff][..], "lesson", &h).is_err());
    }
    #[test]
    fn ide_config_honors_first_value_and_default_port_only_if_omitted() {
        let mut h = host();
        h.port = 22;
        assert!(alias_matches(
            "Host lesson\n HostName 127.0.0.1\n User student\n",
            "lesson",
            &h
        ));
        assert!(!alias_matches(
            "Host lesson\n HostName 127.0.0.1\n User student\n Port invalid\n Port 22\n",
            "lesson",
            &h
        ));
        assert!(!alias_matches(
            "HostName other\nHost lesson\n HostName 127.0.0.1\n User student\n",
            "lesson",
            &h
        ));
    }
}
