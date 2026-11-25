use super::Card;
#[cfg(any(windows, target_os = "macos"))]
use serde_json::Value;
use std::{fs::File, process::Command};
pub(super) struct Target {
    pub disk: File,
    pub _volume_locks: Vec<File>,
}

fn output(mut cmd: Command) -> Result<Vec<u8>, String> {
    let out = cmd.output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(out.stdout)
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use std::{
        fs,
        os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
        path::Path,
    };
    fn text(p: &Path, n: &str) -> String {
        fs::read_to_string(p.join(n))
            .unwrap_or_default()
            .trim()
            .to_owned()
    }
    fn devno(m: u64) -> String {
        format!("{}:{}", libc::major(m), libc::minor(m))
    }
    pub fn list() -> Result<Vec<Card>, String> {
        let fs = fs::read_dir("/sys/block").map_err(|e| e.to_string())?;
        let mut cards = Vec::new();
        for entry in fs.flatten() {
            let p = entry.path();
            let key = entry.file_name().to_string_lossy().into_owned();
            let resolved = fs::canonicalize(&p).map_err(|e| e.to_string())?;
            let native_sd = text(&p, "device/type") == "SD";
            if !(native_sd
                || text(&p, "removable") == "1"
                    && resolved
                        .components()
                        .any(|c| c.as_os_str().to_string_lossy().starts_with("usb")))
                || text(&p, "ro") != "0"
            {
                continue;
            }
            if !key.bytes().all(|c| c.is_ascii_alphanumeric()) {
                continue;
            }
            let Some(bytes) = text(&p, "size")
                .parse::<u64>()
                .ok()
                .and_then(|n| n.checked_mul(512))
            else {
                continue;
            };
            let seq = text(&p, "diskseq");
            let dev = text(&p, "dev");
            let serial = text(&p, "device/cid");
            if bytes == 0 || dev.is_empty() || seq.is_empty() {
                continue;
            }
            let sector = text(&p, "queue/logical_block_size").parse().unwrap_or(0);
            let name = text(&p, "device/model");
            cards.push(Card {
                key: key.clone(),
                name: if name.is_empty() { key } else { name },
                bytes,
                sector,
                identity: format!("{}:{dev}:{seq}:{serial}", resolved.display()),
            });
        }
        Ok(cards)
    }
    pub fn open(card: &Card) -> Result<Target, String> {
        let p = std::path::PathBuf::from("/sys/block").join(&card.key);
        let mut nodes = vec![(card.key.clone(), text(&p, "dev"))];
        for ent in fs::read_dir(&p).map_err(|e| e.to_string())?.flatten() {
            if ent.path().join("partition").exists() {
                nodes.push((
                    ent.file_name().to_string_lossy().into_owned(),
                    text(&ent.path(), "dev"),
                ));
            }
        }
        let mounts = fs::read_to_string("/proc/self/mountinfo").map_err(|e| e.to_string())?;
        let mut release = Vec::new();
        for (name, dev) in &nodes {
            let hs = Path::new("/sys/class/block").join(name).join("holders");
            if fs::read_dir(hs)
                .map_err(|e| e.to_string())?
                .next()
                .is_some()
            {
                return Err("设备正在被卷管理器使用".into());
            }
            for line in mounts.lines() {
                let fields: Vec<_> = line.split_whitespace().take(6).collect();
                if fields.len() > 4 && fields[2] == dev {
                    if !fields[4].starts_with("/media/") && !fields[4].starts_with("/run/media/") {
                        return Err(format!("卡上有正在使用的挂载点 {}", fields[4]));
                    }
                    release.push(name);
                }
            }
        }
        for name in release {
            let mut c = Command::new("/usr/bin/udisksctl");
            c.args([
                "unmount",
                "--block-device",
                &format!("/dev/{name}"),
                "--no-user-interaction",
            ]);
            output(c)?;
        }
        let disk = File::options()
            .read(true)
            .write(true)
            .custom_flags(libc::O_EXCL | libc::O_NOFOLLOW)
            .open(format!("/dev/{}", card.key))
            .map_err(|e| format!("无法独占读卡器：{e}"))?;
        let meta = disk.metadata().map_err(|e| e.to_string())?;
        if !meta.file_type().is_block_device()
            || devno(meta.rdev()) != text(&p, "dev")
            || !list()?.contains(card)
        {
            return Err("卡片身份已改变".into());
        }
        Ok(Target {
            disk,
            _volume_locks: vec![],
        })
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::{
        ffi::c_void,
        os::windows::{fs::OpenOptionsExt, io::AsRawHandle, process::CommandExt},
        path::PathBuf,
    };
    #[link(name = "kernel32")]
    extern "system" {
        fn DeviceIoControl(
            h: *mut c_void,
            code: u32,
            ib: *const c_void,
            il: u32,
            ob: *mut c_void,
            ol: u32,
            n: *mut u32,
            ov: *mut c_void,
        ) -> i32;
    }
    fn ctl(f: &File, code: u32, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut n = 0;
        let ok = unsafe {
            DeviceIoControl(
                f.as_raw_handle(),
                code,
                std::ptr::null(),
                0,
                buf.as_mut_ptr().cast(),
                buf.len() as u32,
                &mut n,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(n as usize)
        }
    }
    const INVENTORY: &str = include_str!("card-inventory.ps1");
    fn inventory() -> Result<Vec<Value>, String> {
        use std::os::windows::ffi::OsStringExt;
        let mut path = [0u16; 512];
        let n = unsafe {
            windows::Win32::System::SystemInformation::GetSystemDirectoryW(Some(&mut path))
        } as usize;
        if n == 0 || n >= path.len() {
            return Err("无法定位 Windows 系统目录".into());
        }
        let system = PathBuf::from(std::ffi::OsString::from_wide(&path[..n]));
        let mut c = Command::new(system.join(r"WindowsPowerShell\v1.0\powershell.exe"));
        c.env_remove("PSModulePath")
            .creation_flags(0x08000000)
            .args(["-NoProfile", "-NonInteractive", "-Command", INVENTORY]);
        serde_json::from_slice(&output(c)?).map_err(|e| format!("无法解析 Windows 磁盘清单：{e}"))
    }
    fn entry(v: &Value) -> Option<Card> {
        Some(Card {
            key: v["key"].as_u64()?.to_string(),
            name: v["name"].as_str()?.into(),
            bytes: v["bytes"].as_u64()?,
            sector: v["sector"].as_u64()?,
            identity: v["identity"].as_str()?.into(),
        })
    }
    fn current_card(v: &Value) -> Option<Card> {
        let mut card = entry(v)?;
        let handle = File::options()
            .access_mode(0)
            .share_mode(3)
            .open(format!(r"\\.\PhysicalDrive{}", card.key))
            .ok()?;
        let mut generation = [0u8; 4];
        if ctl(&handle, 0x2d0800, &mut generation).is_ok_and(|n| n == 4) {
            card.identity
                .push_str(&format!("|media:{}", u32::from_le_bytes(generation)));
        }
        Some(card)
    }
    pub fn list() -> Result<Vec<Card>, String> {
        Ok(inventory()?.iter().filter_map(current_card).collect())
    }
    pub fn open(card: &Card) -> Result<Target, String> {
        let v = inventory()?
            .into_iter()
            .find(|v| current_card(v).as_ref() == Some(card))
            .ok_or("卡片已移除或身份改变")?;
        let mut locks = vec![];
        for vol in v["volumes"].as_array().ok_or("无法核对卡上的卷")? {
            let path = vol.as_str().ok_or("卷路径无效")?.trim_end_matches('\\');
            if !path.starts_with(r"\\?\Volume{")
                || !path.ends_with('}')
                || !path[11..path.len() - 1]
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() || c == '-')
            {
                return Err("卷标识无效".into());
            }
            let f = File::options()
                .read(true)
                .write(true)
                .share_mode(3)
                .open(path)
                .map_err(|e| e.to_string())?;
            let mut number = [0; 12];
            if ctl(&f, 0x2d1080, &mut number).map_err(|e| e.to_string())? != 12
                || u32::from_le_bytes(number[4..8].try_into().unwrap()).to_string() != card.key
            {
                return Err("卷已切换到其他磁盘".into());
            }
            ctl(&f, 0x90018, &mut []).map_err(|e| format!("卷正在使用，无法独占：{e}"))?;
            ctl(&f, 0x90020, &mut []).map_err(|e| e.to_string())?;
            locks.push(f);
        }
        if !list()?.contains(card) {
            return Err("卡片身份已改变".into());
        }
        let disk = File::options()
            .read(true)
            .write(true)
            .share_mode(3)
            .custom_flags(0x80000000)
            .open(format!(r"\\.\PhysicalDrive{}", card.key))
            .map_err(|e| e.to_string())?;
        let mut length = [0; 8];
        if ctl(&disk, 0x7405c, &mut length).map_err(|e| e.to_string())? != 8
            || u64::from_le_bytes(length) != card.bytes
        {
            return Err("读卡器容量已改变".into());
        }
        if !list()?.contains(card) {
            return Err("读卡器已重新枚举".into());
        }
        Ok(Target {
            disk,
            _volume_locks: locks,
        })
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use std::{
        io::Write,
        os::fd::AsRawFd,
        os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
        process::Stdio,
    };
    fn info(args: &[&str]) -> Result<Value, String> {
        let mut c = Command::new("/usr/sbin/diskutil");
        c.args(args);
        let plist = output(c)?;
        let mut p = Command::new("/usr/bin/plutil")
            .args(["-convert", "json", "-o", "-", "--", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        p.stdin
            .take()
            .unwrap()
            .write_all(&plist)
            .map_err(|e| e.to_string())?;
        let v = p.wait_with_output().map_err(|e| e.to_string())?;
        if !v.status.success() {
            return Err("无法读取磁盘描述".into());
        }
        serde_json::from_slice(&v.stdout).map_err(|e| e.to_string())
    }
    pub fn list() -> Result<Vec<Card>, String> {
        let disks = info(&["list", "-plist", "physical"])?;
        let mut cards = Vec::new();
        for d in disks["AllDisksAndPartitions"]
            .as_array()
            .ok_or("无法列出物理磁盘")?
        {
            let Some(key) = d["DeviceIdentifier"].as_str() else {
                continue;
            };
            let Some(n) = key.strip_prefix("disk") else {
                continue;
            };
            if n.is_empty() || !n.bytes().all(|c| c.is_ascii_digit()) {
                continue;
            }
            let v = info(&["info", "-plist", key])?;
            if v["Whole"] != true
                || v["Writable"] != true
                || v["RemovableMedia"] != true
                || v["VirtualOrPhysical"] != "Physical"
                || !matches!(v["BusProtocol"].as_str(), Some("USB" | "Secure Digital"))
            {
                continue;
            }
            if d["Partitions"].as_array().into_iter().flatten().any(|p| {
                p["Content"]
                    .as_str()
                    .is_some_and(|s| s.contains("APFS") || s.contains("CoreStorage"))
                    || p["MountPoint"]
                        .as_str()
                        .is_some_and(|m| !m.starts_with("/Volumes/"))
            }) {
                continue;
            }
            let Some(id) = v["DeviceTreePath"].as_str() else {
                continue;
            };
            if let (Some(bytes), Some(sector)) =
                (v["TotalSize"].as_u64(), v["DeviceBlockSize"].as_u64())
            {
                if bytes > 0 {
                    cards.push(Card {
                        key: key.into(),
                        name: v["MediaName"].as_str().unwrap_or(key).into(),
                        bytes,
                        sector,
                        identity: {
                            let Ok(node) = std::fs::metadata(format!("/dev/r{key}")) else {
                                continue;
                            };
                            format!(
                                "{id}:{}:{}:{}:{}",
                                node.ino(),
                                node.ctime(),
                                node.ctime_nsec(),
                                v["DiskUUID"].as_str().unwrap_or("")
                            )
                        },
                    });
                }
            }
        }
        Ok(cards)
    }
    pub fn open(card: &Card) -> Result<Target, String> {
        let mut c = Command::new("/usr/sbin/diskutil");
        c.args(["unmountDisk", &card.key]);
        output(c)?;
        let disk = File::options()
            .read(true)
            .write(true)
            .custom_flags(libc::O_EXLOCK | libc::O_NONBLOCK | libc::O_NOFOLLOW)
            .open(format!("/dev/r{}", card.key))
            .map_err(|e| e.to_string())?;
        if !disk
            .metadata()
            .map_err(|e| e.to_string())?
            .file_type()
            .is_char_device()
            || !list()?.contains(card)
        {
            return Err("卡片已更换".into());
        }
        if unsafe { libc::fcntl(disk.as_raw_fd(), libc::F_NOCACHE, 1) } < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(Target {
            disk,
            _volume_locks: vec![],
        })
    }
}
pub(super) use platform::{list, open};
