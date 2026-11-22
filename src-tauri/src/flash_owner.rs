use rusqlite::{Connection, OpenFlags};
use std::{path::Path, time::Duration};

/// Old LYNX releases do not participate in Imager's in-process hardware locks.
/// Inspect their live executor and task journal without signalling either process.
pub(crate) fn check(kind: &str) -> Result<(), String> {
    check_processes(kind, |kind| {
        let data = dirs::data_dir().ok_or("FLASH_OWNER_UNKNOWN:无法定位原版 LYNX 的数据目录")?;
        check_journal(&data.join("com.dshanpi.lynx/workbench.sqlite"), kind)
    })
}

/// A legacy GUI can accept a flash after an idle journal check. Refuse to
/// launch a new hardware task while that GUI is running.
pub(crate) fn check_start(kind: &str) -> Result<(), String> {
    check_processes(kind, |_| {
        Err("LEGACY_LYNX_ACTIVE:原版 LYNX 已打开。请等待原版任务结束并正常退出，再在 Imager 中执行硬件操作。".to_owned())
    })
}

// Legacy LYNX may hold its CH347 power monitor without a task_history entry.
pub(crate) fn check_ch347() -> Result<(), String> {
    check_processes("ch347", |_| {
        Err("CH347_OWNER_UNKNOWN:原版 LYNX 正在运行，电源控制器可能已由其接管。请先在原版中完成任务并退出，再连接 CH347。".to_owned())
    })
}

fn check_processes(
    kind: &str,
    journal: impl FnOnce(&str) -> Result<(), String>,
) -> Result<(), String> {
    let mut legacy_running = false;
    let mut external_helper = None;
    visit_processes(|name, pid, parent| {
        let name = name.strip_suffix(".exe").unwrap_or(name);
        if name.eq_ignore_ascii_case("lynx-app") {
            legacy_running = true;
        }
        let matches_helper = if kind.is_empty() {
            matches!(name, "openixcli" | "openrockcli" | "uuu" | "dfu-util")
        } else {
            !matches!(kind, "k230" | "ch347") && name.eq_ignore_ascii_case(kind)
        };
        if matches_helper && parent != std::process::id() {
            external_helper = Some(pid);
        }
    })
    .map_err(|e| format!("FLASH_OWNER_UNKNOWN:无法检查烧录进程（{e}），请重新检测"))?;
    if let Some(pid) = external_helper {
        return Err(format!(
            "FLASHER_IN_USE:另一个进程正在运行 {kind}（PID {pid}）。请等待它结束后重新检测"
        ));
    }
    if legacy_running {
        journal(kind)?;
    }
    Ok(())
}

fn check_journal(path: &Path, kind: &str) -> Result<(), String> {
    let read = || -> rusqlite::Result<bool> {
        let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        db.busy_timeout(Duration::from_millis(150))?;
        db.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM task_history AS t LEFT JOIN devices AS d ON d.id=t.device_id
                WHERE t.status='running' AND t.kind IN ('flash','workflow','mainline_boot')
                AND (?1='' OR COALESCE(d.flasher_kind,'') IN ('',?1) OR
                     (t.kind='mainline_boot' AND ?1='openixcli'))
            )",
            [kind],
            |row| row.get(0),
        )
    };
    match read() {
        Ok(false) => Ok(()),
        Ok(true) => Err(format!("FLASHER_IN_USE:原版 LYNX 有正在使用或准备使用 {kind} 的任务。请等待任务结束后重新检测")),
        Err(e) => Err(format!("FLASH_OWNER_UNKNOWN:原版 LYNX 正在运行，暂时无法读取其硬件任务状态（{e}）。请稍后重新检测")),
    }
}

#[cfg(target_os = "linux")]
fn visit_processes(mut visit: impl FnMut(&str, u32, u32)) -> std::io::Result<()> {
    for entry in std::fs::read_dir("/proc")? {
        let entry = entry?;
        let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse().ok()) else {
            continue;
        };
        let stat = match std::fs::read(entry.path().join("stat")) {
            Ok(data) => data,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
                ) =>
            {
                continue
            }
            Err(e) => return Err(e),
        };
        // comm is parenthesized and can contain spaces or ')' characters.
        let text = String::from_utf8_lossy(&stat);
        if let Some((head, fields)) = text.rsplit_once(") ") {
            if let (Some((_, name)), Some(parent)) = (
                head.split_once('('),
                fields
                    .split_whitespace()
                    .nth(1)
                    .and_then(|s| s.parse().ok()),
            ) {
                visit(name, pid, parent);
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
fn visit_processes(mut visit: impl FnMut(&str, u32, u32)) -> std::io::Result<()> {
    use windows::Win32::{
        Foundation::{CloseHandle, ERROR_NO_MORE_FILES},
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
    };
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }?;
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut next = unsafe { Process32FirstW(snapshot, &mut entry) };
    while next.is_ok() {
        let len = entry
            .szExeFile
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(entry.szExeFile.len());
        let name = String::from_utf16_lossy(&entry.szExeFile[..len]).to_ascii_lowercase();
        visit(&name, entry.th32ProcessID, entry.th32ParentProcessID);
        next = unsafe { Process32NextW(snapshot, &mut entry) };
    }
    unsafe { CloseHandle(snapshot) }?;
    match next {
        Err(e) if e.code() == ERROR_NO_MORE_FILES.to_hresult() => Ok(()),
        result => result.map_err(Into::into),
    }
}

#[cfg(target_os = "macos")]
fn visit_processes(mut visit: impl FnMut(&str, u32, u32)) -> std::io::Result<()> {
    let output = std::process::Command::new("/bin/ps")
        .args(["-A", "-o", "pid=,ppid=,comm="])
        .output()?;
    if !output.status.success() {
        return Err(std::io::Error::other("ps failed"));
    }
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let mut fields = line.split_whitespace();
        if let (Some(pid), Some(parent)) = (
            fields.next().and_then(|s| s.parse().ok()),
            fields.next().and_then(|s| s.parse().ok()),
        ) {
            let command = fields.collect::<Vec<_>>().join(" ");
            if let Some(name) = Path::new(&command).file_name().and_then(|s| s.to_str()) {
                visit(name, pid, parent);
            }
        }
    }
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn visit_processes(_visit: impl FnMut(&str, u32, u32)) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "hardware ownership inspection unavailable",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sibling_probe() {
        let Ok(kind) = std::env::var("IMAGER_TEST_OWNER_NAME") else {
            return;
        };
        let result = check_processes(&kind, |_| Ok(()));
        if let Ok(pid) = std::env::var("IMAGER_TEST_OWNER_PID") {
            let message = result.unwrap_err();
            assert!(
                message.starts_with("FLASHER_IN_USE:") && message.contains(&format!("PID {pid}")),
                "{message}"
            );
        } else {
            result.unwrap();
        }
    }

    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn external_owner_survives_rejection_and_releases_on_exit() {
        use std::process::{Command, Stdio};
        let folder =
            std::env::temp_dir().join(format!("imager-owner-{:016x}", rand::random::<u64>()));
        std::fs::create_dir(&folder).unwrap();
        // Linux comm is limited to 15 bytes. Use a private name to isolate parallel tests.
        let kind = format!("img{:08x}", rand::random::<u32>());
        #[cfg(windows)]
        let (exe, source, args) = (
            folder.join(format!("{kind}.exe")),
            std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
                .join("System32/ping.exe"),
            vec!["-n", "9", "127.0.0.1"],
        );
        #[cfg(target_os = "linux")]
        let (exe, source, args) = (
            folder.join(&kind),
            std::path::PathBuf::from("/bin/sleep"),
            vec!["8"],
        );
        std::fs::copy(source, &exe).unwrap();
        let probe = |pid: Option<u32>| {
            let mut cmd = Command::new(std::env::current_exe().unwrap());
            cmd.args(["--exact", "flash_owner::tests::sibling_probe"])
                .env("IMAGER_TEST_OWNER_NAME", &kind)
                .env_remove("IMAGER_TEST_OWNER_PID");
            if let Some(pid) = pid {
                cmd.env("IMAGER_TEST_OWNER_PID", pid.to_string());
            }
            let out = cmd.output().unwrap();
            assert!(
                out.status.success() && String::from_utf8_lossy(&out.stdout).contains("1 passed"),
                "{}",
                String::from_utf8_lossy(&out.stdout)
            );
        };
        probe(None);
        let mut fixture = Command::new(&exe)
            .args(args)
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        probe(Some(fixture.id()));
        assert!(fixture.try_wait().unwrap().is_none());
        fixture.kill().unwrap(); // Only our synthetic process.
        fixture.wait().unwrap();
        probe(None);
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn distinguishes_live_transactions_and_never_creates_a_missing_legacy_db() {
        let folder =
            std::env::temp_dir().join(format!("imager-owner-{:016x}", rand::random::<u64>()));
        std::fs::create_dir(&folder).unwrap();
        let path = folder.join("workbench.sqlite");
        assert!(check_journal(&path, "openixcli")
            .unwrap_err()
            .starts_with("FLASH_OWNER_UNKNOWN"));
        assert!(!path.exists());
        let db = Connection::open(&path).unwrap();
        db.execute_batch(
            "PRAGMA journal_mode=WAL;
            CREATE TABLE devices(id INTEGER PRIMARY KEY, flasher_kind TEXT);
            CREATE TABLE task_history(device_id INTEGER, kind TEXT, status TEXT);
            INSERT INTO devices VALUES(1,'openixcli');
            INSERT INTO task_history VALUES(1,'transfer','running'),(1,'flash','failed');",
        )
        .unwrap();
        assert!(check_journal(&path, "openixcli").is_ok());
        db.execute_batch("INSERT INTO task_history VALUES(1,'flash','running');")
            .unwrap();
        assert!(check_journal(&path, "openixcli")
            .unwrap_err()
            .starts_with("FLASHER_IN_USE"));
        assert!(check_journal(&path, "openrockcli").is_ok());
        db.execute_batch("UPDATE task_history SET status='success';")
            .unwrap();
        assert!(check_journal(&path, "openixcli").is_ok());
        drop(db);
        std::fs::remove_dir_all(folder).unwrap();
    }
}
