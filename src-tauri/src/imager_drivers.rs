//! USB driver setup is initiated only by the device-connection screen.
use serde::Serialize;
use tauri::{AppHandle, Manager};

#[derive(Serialize)]
pub struct DriverInfo {
    platform: &'static str,
    ids: &'static [&'static str],
}

fn ids(vendor: &str) -> Result<&'static [&'static str], String> {
    Ok(match vendor {
        "allwinner" => &["1f3a:efe8"],
        "rockchip" => &[
            "2207:110c",
            "2207:180a",
            "2207:281a",
            "2207:290a",
            "2207:292a",
            "2207:292c",
            "2207:300a",
            "2207:300b",
            "2207:301a",
            "2207:310a",
            "2207:310b",
            "2207:310c",
            "2207:320a",
            "2207:320b",
            "2207:320c",
            "2207:330a",
            "2207:330c",
            "2207:330d",
            "2207:330e",
            "2207:350a",
            "2207:350b",
            "2207:350d",
            "2207:350e",
            "2207:350f",
        ],
        "st" => &["0483:df11"],
        "kendryte" => &["29f1:0230"],
        // ROM, SPL and Fastboot enumerate independently. Limit rules to download IDs.
        "nxp" => &[
            "15a2:0080",
            "1fc9:012b",
            "1fc9:012f",
            "1fc9:0134",
            "1fc9:013e",
            "1fc9:0146",
            "1fc9:014e",
            "1fc9:0159",
            "1fc9:015c",
            "1fc9:015d",
            "0525:b4a4",
            "1fc9:0151",
            "0525:a4a5",
            "1fc9:0152",
        ],
        _ => return Err("未知的 USB 烧录平台".into()),
    })
}

#[tauri::command]
pub fn imager_driver_info(vendor: String) -> Result<DriverInfo, String> {
    Ok(DriverInfo {
        platform: std::env::consts::OS,
        ids: if cfg!(windows) && vendor == "nxp" {
            &["0525:a4a5", "1fc9:0152"]
        } else {
            ids(&vendor)?
        },
    })
}

#[tauri::command]
pub async fn imager_install_driver(
    app: AppHandle,
    vendor: String,
    inf_path: Option<String>,
) -> Result<String, String> {
    let devices = ids(&vendor)?;
    #[cfg(not(windows))]
    let _ = inf_path;
    let safety = app.state::<crate::hardware_safety::HardwareSafetyState>();
    let _guard = safety.try_begin("LYNX Imager", "安装 USB 驱动", &vendor)?;
    crate::flash_owner::check_start("")?;
    #[cfg(target_os = "linux")]
    {
        linux_install(&vendor, devices).await
    }
    #[cfg(windows)]
    {
        let _ = devices;
        if vendor == "rockchip" {
            let path = std::path::PathBuf::from(inf_path.ok_or("请选择 Rockusb 驱动 INF 文件")?);
            return windows_setup::install_rockusb(path).await;
        }
        windows_setup::install(app.path().app_cache_dir().map_err(|e| e.to_string())?).await
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = devices;
        Err("当前操作系统没有驱动安装入口".into())
    }
}

#[cfg(target_os = "linux")]
async fn linux_install(vendor: &str, devices: &[&str]) -> Result<String, String> {
    let mut cmd = tokio::process::Command::new("/usr/bin/pkexec");
    cmd.args([
        "/bin/sh",
        "-c",
        include_str!("imager_usb_rules.sh"),
        "lynx-imager",
        vendor,
    ])
    .args(devices);
    let result = cmd
        .output()
        .await
        .map_err(|e| format!("无法打开系统授权窗口，请检查 PolicyKit：{e}"))?;
    match result.status.code() {
        Some(0) => Ok("USB 访问规则已安装。请重新插拔开发板的下载口。".into()),
        Some(126) => Err("已取消系统授权，可重新安装。".into()),
        _ => Err(format!(
            "USB 规则安装失败：{}",
            String::from_utf8_lossy(&result.stderr).trim()
        )),
    }
}

#[cfg(windows)]
mod windows_setup {
    use sha2::{Digest, Sha256};
    use std::{
        fs::{File, OpenOptions},
        io::Write,
        os::windows::fs::OpenOptionsExt,
        path::{Path, PathBuf},
        time::Duration,
    };
    const URL: &str = "https://github.com/pbatard/libwdi/releases/download/v1.5.1/zadig-2.9.exe";
    const SIZE: u64 = 5334088;
    const SHA: &str = "4ecaa95df3da3621486a043aef8b3050b8bafe7c901402871e816229ef82039b";

    // Keep this handle open through UAC and the installer's lifetime.
    fn verified(path: &Path) -> Result<File, String> {
        let mut f = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(path)
            .map_err(|e| e.to_string())?;
        if f.metadata().map_err(|e| e.to_string())?.len() != SIZE {
            return Err("安装器长度校验失败".into());
        }
        let mut hash = Sha256::new();
        std::io::copy(&mut f, &mut hash).map_err(|e| e.to_string())?;
        if hex::encode(hash.finalize()) != SHA {
            return Err("安装器 SHA-256 校验失败".into());
        }
        Ok(f)
    }
    async fn fetch(dir: &Path, path: &Path) -> Result<File, String> {
        if let Ok(f) = verified(path) {
            return Ok(f);
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(90))
            .build()
            .map_err(|e| e.to_string())?;
        let mut response = client
            .get(URL)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("无法下载官方 Zadig 安装器，请检查网络后重试：{e}"))?;
        let mut bytes = Vec::with_capacity(SIZE as usize);
        while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
            if bytes.len() + chunk.len() > SIZE as usize {
                return Err("安装器大小与官方版本不符".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        if bytes.len() != SIZE as usize || hex::encode(Sha256::digest(&bytes)) != SHA {
            return Err("下载的安装器校验失败，请重试".into());
        }
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let tmp = dir.join(format!("download-{:016x}.tmp", rand::random::<u64>()));
        let save = (|| {
            let mut out = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
            out.write_all(&bytes)?;
            drop(out);
            std::fs::rename(&tmp, path)
        })();
        if let Err(e) = save {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("无法保存驱动安装器：{e}"));
        }
        verified(path)
    }
    pub async fn install_rockusb(inf: PathBuf) -> Result<String, String> {
        if !inf.is_absolute()
            || !inf
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.eq_ignore_ascii_case("inf"))
        {
            return Err("请选择有效的 Rockusb 驱动 INF 文件".into());
        }
        let mut file = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&inf)
            .map_err(|e| format!("无法读取驱动文件：{e}"))?;
        let text = read_rockusb_inf(&mut file)?;
        if !text.contains("usb\\vid_2207&pid_") || !text.contains("rockusb.sys") {
            return Err("所选 INF 不包含瑞芯微 Rockusb 驱动和匹配的 USB 设备。".into());
        }
        run_installer(inf, include_str!("imager_rockusb.ps1"), file).await
    }
    fn read_rockusb_inf(file: &mut File) -> Result<String, String> {
        use std::io::Read;
        let mut bytes = Vec::new();
        file.take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 1024 * 1024 {
            return Err("INF 文件过大".into());
        }
        let text = if bytes.starts_with(&[0xff, 0xfe]) || bytes.get(1) == Some(&0) {
            let words: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|p| u16::from_le_bytes([p[0], p[1]]))
                .collect();
            String::from_utf16_lossy(&words)
        } else {
            String::from_utf8_lossy(&bytes).into_owned()
        };
        Ok(text.to_ascii_lowercase())
    }
    pub async fn install(cache: PathBuf) -> Result<String, String> {
        let dir = cache.join("usb-drivers");
        let exe = dir.join("zadig-2.9.exe");
        let locked = fetch(&dir, &exe).await?;
        run_installer(exe, include_str!("imager_zadig.ps1"), locked).await
    }
    async fn run_installer(
        path: PathBuf,
        script: &'static str,
        locked: File,
    ) -> Result<String, String> {
        tauri::async_runtime::spawn_blocking(move || {
            let _locked = locked;
            crate::flash_owner::check_start("")?;
            let mut buf = [0u16; 32768];
            let n = unsafe {
                windows::Win32::System::SystemInformation::GetSystemDirectoryW(Some(&mut buf))
            } as usize;
            if n == 0 || n >= buf.len() {
                return Err("无法定位 Windows 系统目录".into());
            }
            let system = PathBuf::from(String::from_utf16(&buf[..n]).map_err(|e| e.to_string())?);
            let mut command =
                std::process::Command::new(system.join("WindowsPowerShell/v1.0/powershell.exe"));
            crate::process::configure_background_command(&mut command);
            let status = command
                .current_dir(&system)
                .env("LYNX_DRIVER_PATH", path)
                .args(["-NoProfile", "-NonInteractive", "-Command", script])
                .status()
                .map_err(|e| format!("无法启动 Windows 驱动安装器：{e}"))?;
            match status.code() {
                Some(0) => Ok("安装器已关闭，正在重新检测下载口。".into()),
                Some(4) => Ok("Rockusb 驱动包已添加，需要重启 Windows 后生效。".into()),
                Some(5) => Ok("Rockusb 驱动包已添加，正在重新检测下载口。".into()),
                Some(2) => Err("安装器发布者签名未通过 Windows 验证。".into()),
                Some(3) => Err("已取消 Windows 管理员授权。".into()),
                _ => Err("驱动安装器返回错误，请检查设备连接并重试。".into()),
            }
        })
        .await
        .map_err(|e| e.to_string())?
    }
}

#[derive(Serialize)]
pub struct UsbTarget {
    vendor: &'static str,
    key: String,
    pid: u16,
    loader: bool,
}

#[tauri::command]
pub async fn imager_usb_targets() -> Result<Vec<UsbTarget>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let mut targets = Vec::new();
        for device in rusb::devices().map_err(|e| e.to_string())?.iter() {
            let Ok(desc) = device.device_descriptor() else {
                continue;
            };
            let id = format!("{:04x}:{:04x}", desc.vendor_id(), desc.product_id());
            let known_vendor = ["allwinner", "rockchip", "nxp", "st", "kendryte"]
                .into_iter()
                .find(|v| ids(v).is_ok_and(|known| known.contains(&id.as_str())));
            let rockusb = desc.vendor_id() == 0x2207
                && device.active_config_descriptor().is_ok_and(|config| {
                    config.interfaces().any(|iface| {
                        iface.descriptors().any(|d| {
                            (d.class_code(), d.sub_class_code(), d.protocol_code()) == (0xff, 6, 5)
                        })
                    })
                });
            let Some(vendor) = known_vendor.or(if rockusb { Some("rockchip") } else { None })
            else {
                continue;
            };
            let ports = device.port_numbers().unwrap_or_default();
            let key = format!(
                "{}:{}",
                device.bus_number(),
                ports
                    .iter()
                    .map(u8::to_string)
                    .collect::<Vec<_>>()
                    .join(".")
            );
            targets.push(UsbTarget {
                vendor,
                key,
                pid: desc.product_id(),
                loader: vendor == "rockchip"
                    && desc.usb_version().sub_minor() & 1 == 1
                    && !ports.is_empty(),
            });
        }
        Ok(targets)
    })
    .await
    .map_err(|e| e.to_string())?
}
