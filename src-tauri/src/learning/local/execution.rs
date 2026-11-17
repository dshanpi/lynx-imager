//! Process-local execution leases and bounded SSH output. No implicit retries.
use std::{
    collections::HashSet,
    io::Read,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub(super) const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
pub(super) const COMMAND_BUDGET: Duration = Duration::from_secs(60);

/// The package declares one script and literal argv, never a shell program.
/// Parse quotes for ordinary spaced/empty arguments, then quote each word again.
pub(super) fn render_command(command: &str) -> Result<String, String> {
    let forbidden = || "LOCAL_COURSE_COMMAND_FORBIDDEN".to_owned();
    if command.len() > 160
        || !command.starts_with("./scripts/")
        || command.chars().any(char::is_control)
        || command.contains([
            ';', '|', '&', '`', '>', '<', '$', '(', ')', '*', '?', '[', ']', '{', '}', '~', '#',
        ])
        || command.split_whitespace().any(|part| part.contains(".."))
    {
        return Err(forbidden());
    }
    let words = shlex::split(command).ok_or_else(forbidden)?;
    let program = words.first().ok_or_else(forbidden)?;
    if !program.starts_with("./scripts/")
        || program.ends_with('/')
        || words.iter().any(|word| word.contains(".."))
    {
        return Err(forbidden());
    }
    super::safe_relative(&program[2..]).map_err(|_| forbidden())?;
    Ok(words
        .iter()
        .map(|word| crate::workbench::ssh::shell_quote(word))
        .collect::<Vec<_>>()
        .join(" "))
}

#[derive(Default)]
pub(super) struct ActionGate(Arc<Mutex<HashSet<i64>>>);

pub(super) struct ActionLease {
    active: Arc<Mutex<HashSet<i64>>>,
    project_id: i64,
}

impl ActionGate {
    pub(super) fn acquire(&self, project_id: i64) -> Result<ActionLease, String> {
        let mut active = self
            .0
            .lock()
            .map_err(|_| "LEARNING_ACTION_GATE_UNAVAILABLE")?;
        if !active.insert(project_id) {
            return Err("LEARNING_ACTION_BUSY:该工程已有课程命令运行，请等待结果".into());
        }
        Ok(ActionLease {
            active: self.0.clone(),
            project_id,
        })
    }
}

impl Drop for ActionLease {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active.lock() {
            active.remove(&self.project_id);
        }
    }
}

pub(super) fn remaining_timeout(deadline: Instant, now: Instant) -> Result<u32, String> {
    let remaining = deadline
        .checked_duration_since(now)
        .filter(|d| !d.is_zero())
        .ok_or("LEARNING_ACTION_TIMEOUT:课程命令超时；请先确认远端任务状态，再手动重试")?;
    // libssh2 interprets zero as unlimited, so round up to at least one millisecond.
    Ok(remaining.as_millis().clamp(1, u32::MAX as u128) as u32)
}

pub(super) fn read_output(
    reader: &mut impl Read,
    limit: usize,
    mut check_budget: impl FnMut() -> Result<(), String>,
) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        check_budget()?;
        let size = buffer
            .len()
            .min(limit.saturating_sub(output.len()).saturating_add(1));
        let read = reader.read(&mut buffer[..size]);
        check_budget()?;
        let count = read.map_err(|_| "LEARNING_SSH_READ")?;
        if count == 0 {
            return Ok(output);
        }
        if output.len() + count > limit {
            return Err(
                "LEARNING_ACTION_OUTPUT_LIMIT:课程命令输出超过 1 MiB；请检查远端任务".into(),
            );
        }
        output.extend_from_slice(&buffer[..count]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Cursor};

    #[test]
    fn command_argv_preserves_spaces_empty_arguments_and_unicode() {
        let rendered =
            render_command("./scripts/check.sh 'a b' '' 中文 \"it's\" --flag=value").unwrap();
        assert_eq!(
            shlex::split(&rendered).unwrap(),
            vec![
                "./scripts/check.sh",
                "a b",
                "",
                "中文",
                "it's",
                "--flag=value"
            ]
        );
        for bad in [
            "./scripts/",
            "./scripts/x # ignored",
            "./scripts/x\targ",
            "./scripts/x {a,b}",
            "./scripts/x ~",
            "./scripts/x \\",
        ] {
            assert!(render_command(bad).is_err(), "accepted {bad:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn real_shell_receives_only_literal_argv_in_quoted_workspace() {
        use std::os::unix::fs::PermissionsExt;
        let root =
            std::env::temp_dir().join(format!("lynx-command-{} ' quoted", rand::random::<u64>()));
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join("scripts")).unwrap();
        let script = root.join("scripts/check.sh");
        std::fs::write(&script, "#!/bin/sh\nprintf '<%s>\\n' \"$@\"\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let rendered =
            render_command("./scripts/check.sh 'a b' '' 中文 \"it's\" --flag=value").unwrap();
        let line = format!(
            "cd {} && {}",
            crate::workbench::ssh::shell_quote(&root.to_string_lossy()),
            rendered
        );
        let output = std::process::Command::new("bash")
            .args(["--noprofile", "--norc", "-c", &line])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            "<a b>\n<>\n<中文>\n<it's>\n<--flag=value>\n"
        );
        assert!(output.stderr.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lease_serializes_project_and_releases_after_worker() {
        let gate = ActionGate::default();
        let lease = gate.acquire(1).unwrap();
        assert!(gate.acquire(1).is_err());
        let _other = gate.acquire(2).unwrap();
        let (release, wait) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _lease = lease;
            wait.recv().unwrap();
        });
        assert!(gate.acquire(1).is_err());
        release.send(()).unwrap();
        worker.join().unwrap();
        assert!(gate.acquire(1).is_ok());
    }

    #[test]
    fn output_exact_limit_passes_and_overflow_stops_at_one_extra_byte() {
        assert_eq!(
            read_output(&mut Cursor::new(b"1234"), 4, || Ok(())).unwrap(),
            b"1234"
        );
        let mut source = Cursor::new(b"123456789");
        assert!(read_output(&mut source, 4, || Ok(()))
            .unwrap_err()
            .starts_with("LEARNING_ACTION_OUTPUT_LIMIT"));
        assert_eq!(source.position(), 5);
        assert!(read_output(&mut Cursor::new([]), 4, || Ok(()))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn output_timeout_and_read_failure_never_return_partial_success() {
        let mut checks = 0;
        let result = read_output(&mut Cursor::new(b"1234"), 4, || {
            checks += 1;
            if checks == 2 {
                Err("deadline".into())
            } else {
                Ok(())
            }
        });
        assert_eq!(result.unwrap_err(), "deadline");
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("broken"))
            }
        }
        assert_eq!(
            read_output(&mut Broken, 4, || Ok(())).unwrap_err(),
            "LEARNING_SSH_READ"
        );
    }

    #[test]
    fn deadline_never_sets_unlimited_timeout() {
        let now = Instant::now();
        assert!(remaining_timeout(now, now).is_err());
        assert!(remaining_timeout(now, now + Duration::from_millis(1)).is_err());
        assert_eq!(
            remaining_timeout(now + Duration::from_nanos(1), now).unwrap(),
            1
        );
        assert_eq!(
            remaining_timeout(now + COMMAND_BUDGET, now).unwrap(),
            60_000
        );
    }
}
