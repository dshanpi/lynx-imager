//! RockUSB Loader maintenance. Keep the probed device handle through confirmation:
//! a replacement board on the same hub port must never inherit an erase request.
use rusb::{DeviceHandle, Direction, GlobalContext, TransferType};
use serde::Serialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

const CONFIRM_WINDOW: Duration = Duration::from_secs(60);
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(30);
const CHUNK_SECTORS: u32 = 16384;

struct Loader {
    handle: DeviceHandle<GlobalContext>,
    interface: u8,
    input: u8,
    output: u8,
    tag: u32,
}

impl Loader {
    fn open(key: &str, pid: u16) -> Result<Self, String> {
        for device in rusb::devices().map_err(|e| e.to_string())?.iter() {
            let Ok(desc) = device.device_descriptor() else {
                continue;
            };
            if desc.vendor_id() != 0x2207
                || desc.product_id() != pid
                || desc.usb_version().sub_minor() & 1 == 0
            {
                continue;
            }
            let ports = device.port_numbers().unwrap_or_default();
            if ports.is_empty()
                || format!(
                    "{}:{}",
                    device.bus_number(),
                    ports
                        .iter()
                        .map(u8::to_string)
                        .collect::<Vec<_>>()
                        .join(".")
                ) != key
            {
                continue;
            }
            let config = device
                .active_config_descriptor()
                .map_err(|e| e.to_string())?;
            for iface in config.interfaces().flat_map(|i| i.descriptors()) {
                if (
                    iface.class_code(),
                    iface.sub_class_code(),
                    iface.protocol_code(),
                    iface.setting_number(),
                ) != (0xff, 6, 5, 0)
                {
                    continue;
                }
                let bulk = |direction| {
                    iface
                        .endpoint_descriptors()
                        .find(|ep| {
                            ep.transfer_type() == TransferType::Bulk && ep.direction() == direction
                        })
                        .map(|ep| ep.address())
                };
                if let (Some(input), Some(output)) = (bulk(Direction::In), bulk(Direction::Out)) {
                    return Ok(Self {
                        handle: device
                            .open()
                            .map_err(|e| format!("无法打开所选 Loader：{e}"))?,
                        interface: iface.interface_number(),
                        input,
                        output,
                        tag: 0,
                    });
                }
            }
        }
        Err("未找到所选 RockUSB Loader 接口，请检查连接和驱动。".into())
    }

    // RockUSB CBW/CSW framing, as in Rockchip rkdeveloptool and OpenRockCLI rock.c.
    fn command(
        &mut self,
        opcode: u8,
        start: u32,
        count: u16,
        data: &mut [u8],
    ) -> Result<(), String> {
        self.tag = self.tag.wrapping_add(1);
        let mut cbw = [0u8; 31];
        cbw[..4].copy_from_slice(b"USBC");
        cbw[4..8].copy_from_slice(&self.tag.to_le_bytes());
        cbw[8..12].copy_from_slice(&(data.len() as u32).to_le_bytes());
        cbw[12] = if data.is_empty() { 0 } else { 0x80 };
        cbw[14] = if opcode == 0x25 { 10 } else { 6 };
        cbw[15] = opcode;
        cbw[17..21].copy_from_slice(&start.to_be_bytes());
        cbw[22..24].copy_from_slice(&count.to_be_bytes());
        let io = |e: rusb::Error| format!("RockUSB 通信失败：{e}");
        if self
            .handle
            .write_bulk(self.output, &cbw, TRANSFER_TIMEOUT)
            .map_err(io)?
            != cbw.len()
        {
            return Err("RockUSB 命令发送不完整".into());
        }
        if !data.is_empty()
            && self
                .handle
                .read_bulk(self.input, data, TRANSFER_TIMEOUT)
                .map_err(io)?
                != data.len()
        {
            return Err("设备返回信息不完整".into());
        }
        let mut status = [0; 13];
        if self
            .handle
            .read_bulk(self.input, &mut status, TRANSFER_TIMEOUT)
            .map_err(io)?
            != status.len()
            || !valid_response(&status, self.tag)
        {
            return Err(format!("设备未确认 RockUSB 命令 0x{opcode:02X}"));
        }
        Ok(())
    }

    fn sectors(&mut self) -> Result<u32, String> {
        let mut medium = [0; 4];
        self.command(0x2b, 0, 0, &mut medium)?;
        if u32::from_le_bytes(medium) != 2 {
            return Err("当前 Loader 报告的存储介质不是 eMMC。".into());
        }
        let mut info = [0; 11];
        self.command(0x1a, 0, 0, &mut info)?;
        let sectors = u32::from_le_bytes(info[..4].try_into().unwrap());
        if sectors == 0 || sectors == u32::MAX {
            return Err("设备没有返回有效的 eMMC 用户区容量。".into());
        }
        Ok(sectors)
    }

    fn claim(&self) -> Result<(), String> {
        self.handle
            .claim_interface(self.interface)
            .map_err(|e| format!("RockUSB 接口不可独占：{e}"))
    }
}

fn valid_response(status: &[u8; 13], tag: u32) -> bool {
    &status[..4] == b"USBS" && status[4..8] == tag.to_le_bytes() && status[8..] == [0; 5]
}

fn erase_range(
    sectors: u32,
    mut erase: impl FnMut(u32, u16) -> Result<(), String>,
) -> Result<u64, String> {
    let mut start = 0;
    while start < sectors {
        let count = (sectors - start).min(CHUNK_SECTORS) as u16;
        erase(start, count)
            .map_err(|e| format!("擦除在扇区 {start} 处中断，用户区可能已部分擦除：{e}"))?;
        start += u32::from(count);
    }
    Ok(u64::from(start) * 512)
}

struct Pending {
    token: String,
    created: Instant,
    loader: Loader,
    sectors: u32,
    key: String,
}
static PENDING: Mutex<Option<Pending>> = Mutex::new(None);

#[derive(Serialize)]
pub struct Preview {
    token: String,
    bytes: u64,
}

#[tauri::command]
pub async fn imager_emmc_prepare(app: AppHandle, key: String, pid: u16) -> Result<Preview, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = app
            .state::<crate::hardware_safety::HardwareSafetyState>()
            .try_begin("Imager", "检查 eMMC", key.clone())?;
        crate::flash_owner::check_start("openrockcli")?;
        let mut loader = Loader::open(&key, pid)?;
        loader.claim()?;
        let sectors = loader.sectors()?;
        loader
            .handle
            .release_interface(loader.interface)
            .map_err(|e| e.to_string())?;
        let token = format!("{:032x}", rand::random::<u128>());
        *PENDING.lock().map_err(|e| e.to_string())? = Some(Pending {
            token: token.clone(),
            created: Instant::now(),
            loader,
            sectors,
            key,
        });
        let expiry = token.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(CONFIRM_WINDOW).await;
            let _ = imager_emmc_cancel(expiry);
        });
        Ok(Preview {
            token,
            bytes: u64::from(sectors) * 512,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn imager_emmc_cancel(token: String) -> Result<(), String> {
    let mut pending = PENDING.lock().map_err(|e| e.to_string())?;
    if pending.as_ref().is_some_and(|p| p.token == token) {
        pending.take();
    }
    Ok(())
}

#[tauri::command]
pub async fn imager_emmc_erase(app: AppHandle, token: String) -> Result<u64, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut pending = PENDING.lock().map_err(|e| e.to_string())?;
        if !pending
            .as_ref()
            .is_some_and(|p| p.token == token && p.created.elapsed() < CONFIRM_WINDOW)
        {
            return Err("确认已失效，请重新检查设备。".into());
        }
        let mut target = pending.take().unwrap();
        drop(pending);
        let _guard = app
            .state::<crate::hardware_safety::HardwareSafetyState>()
            .try_begin("Imager", "擦除 eMMC 用户区", target.key)?;
        crate::flash_owner::check_start("openrockcli")?;
        target.loader.claim()?;
        if target.loader.sectors()? != target.sectors {
            return Err("设备容量已改变，请重新检查。".into());
        }
        let mut last_update = Instant::now();
        erase_range(target.sectors, |sector, n| {
            target.loader.command(0x25, sector, n, &mut [])?;
            let done = sector + u32::from(n);
            if done == target.sectors || last_update.elapsed() >= Duration::from_millis(350) {
                let _ = app.emit("imager-emmc-progress", u64::from(done) * 512);
                last_update = Instant::now();
            }
            Ok(())
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_user_sectors_including_tail_are_erased_exactly_once() {
        let mut calls = Vec::new();
        assert_eq!(
            erase_range(32769, |s, n| {
                calls.push((s, n));
                Ok(())
            })
            .unwrap(),
            32769 * 512
        );
        assert_eq!(calls, [(0, 16384), (16384, 16384), (32768, 1)]);
    }
    #[test]
    fn failed_chunk_is_never_retried_or_skipped() {
        let mut calls = 0;
        assert!(erase_range(32769, |s, _| {
            calls += 1;
            if s == 16384 {
                Err("断开".into())
            } else {
                Ok(())
            }
        })
        .unwrap_err()
        .contains("16384"));
        assert_eq!(calls, 2);
    }
    #[test]
    fn device_must_acknowledge_the_complete_matching_command() {
        let mut status = *b"USBS\x01\0\0\0\0\0\0\0\0";
        assert!(valid_response(&status, 1));
        assert!(!valid_response(&status, 2));
        status[12] = 1;
        assert!(!valid_response(&status, 1));
        status[12] = 0;
        status[8] = 1;
        assert!(!valid_response(&status, 1));
    }
}
