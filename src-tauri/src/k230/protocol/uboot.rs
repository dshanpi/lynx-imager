//! K230 UBOOT 阶段 USB 协议。
//!
//! UBOOT(loader)阶段:loader 已运行,通过 60 字节 CBW/CSW 封包协议进行介质烧录
//! (probe/get_info/erase/write_lba/read_lba/reboot)。
//!
//! 协议:
//! - 封包固定 60 字节(6B 头 cmd/result/data_size + 54B 数据)
//! - 命令响应:响应 cmd = 原始 cmd | 0x8000
//! - 写数据:WRITE_LBA 携带 [offset, size, max, flag](loader_version>=1 带 4 个 uint64,否则 3 个)
//! - ZLP:当 length % ep_out_mps == 0 时需补发零长度包
//! - 端点:动态扫描 config descriptor 找 bulk IN/OUT(不再硬编码 0x01/0x81)
//!
//! 参考:`refer/k230_flash/src/kburn/burner_k230/burner_uboot.cpp`、
//! `refer/k230_burning_tool/canaan-burn/burn/k230/k230_burn.c`。
//!
//! 实现用 `[u8; 60]` buffer 手动序列化,避免 `#[repr(C)]` 结构体在不同编译器下的
//! 位域/对齐布局问题(参考源码 `kburn_medium_info` 使用 C 位域)。

use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use super::super::error::K230Error;
use super::super::types::MediumType;
use super::usb::K230UsbDevice;

/// UBOOT 封包总大小(6B 头 + 54B 数据)。
pub const KBUNR_USB_PKT_SIZE: usize = 60;
/// 封包数据段大小。
pub const KBUNR_USB_DATA_SIZE: usize = 54;
/// 响应标志:响应 cmd = 原始 cmd | 0x8000。
pub const CMD_FLAG_DEV_TO_HOST: u16 = 0x8000;

/// UBOOT 命令码。
pub const KBURN_CMD_NONE: u16 = 0;
pub const KBURN_CMD_REBOOT: u16 = 0x01;
pub const KBURN_CMD_DEV_PROBE: u16 = 0x10;
pub const KBURN_CMD_DEV_GET_INFO: u16 = 0x11;
pub const KBURN_CMD_ERASE_LBA: u16 = 0x20;
pub const KBURN_CMD_WRITE_LBA: u16 = 0x21;
#[allow(dead_code)]
pub const KBURN_CMD_WRITE_LBA_CHUNK: u16 = 0x22;
#[allow(dead_code)]
pub const KBURN_CMD_READ_LBA: u16 = 0x23;
#[allow(dead_code)]
pub const KBURN_CMD_READ_LBA_CHUNK: u16 = 0x24;

/// 结果码。
pub const KBURN_RESULT_OK: u16 = 1;
#[allow(dead_code)]
pub const KBURN_RESULT_ERROR: u16 = 2;
/// 错误且附带错误字符串(data 区)。
pub const KBURN_RESULT_ERROR_MSG: u16 = 0xFF;

/// Reboot magic("Rbot"),作为 uint64 跟 KBURN_CMD_REBOOT 发送。
pub const KBURN_REBOOT_MAGIC: u64 = 0x52626F74;

/// 介质信息(DEV_GET_INFO 返回,32 字节)。
///
/// 注意:参考源码 `kburn_medium_info` 用 C 位域(timeout_ms:32, wp:8, type:7, valid:1),
/// 这里用显式字段,从 32 字节 buffer 手动解包(小端布局)。
#[derive(Debug, Clone, Default)]
pub struct MediumInfo {
    pub capacity: u64,
    pub blk_size: u64,
    pub erase_size: u64,
    pub timeout_ms: u32,
    pub wp: u8,
    #[allow(dead_code)]
    pub medium_type: u8,
    #[allow(dead_code)]
    pub valid: u8,
}

/// UBOOT 烧录器,持有设备句柄引用、端点、loader 版本与介质信息。
///
/// 生命周期:绑定到 `K230UsbDevice` 的引用,每次烧录创建一个。
pub struct UbootBurner<'a> {
    pub dev: &'a K230UsbDevice,
    pub ep_in: u8,
    pub ep_out: u8,
    pub ep_out_mps: u16,
    pub loader_version: u32,
    pub medium_info: MediumInfo,
    pub error_msg: String,
}

impl<'a> UbootBurner<'a> {
    /// 创建 UBOOT 烧录器:发现端点 + 探测 loader 版本 + nop 清状态。
    pub fn new(dev: &'a K230UsbDevice) -> Result<Self, K230Error> {
        let (ep_in, ep_out, ep_out_mps) = dev.find_bulk_endpoints()?;
        let mut burner = Self {
            dev,
            ep_in,
            ep_out,
            ep_out_mps,
            loader_version: 0,
            medium_info: MediumInfo {
                timeout_ms: 10,
                ..Default::default()
            },
            error_msg: String::new(),
        };
        burner.loader_version = burner.probe_loader_version().unwrap_or(0);
        burner.nop()?;
        burner.medium_info.timeout_ms = 10000; // probe/get_info 用长超时
        Ok(burner)
    }

    fn timeout(&self) -> Duration {
        Duration::from_millis(self.medium_info.timeout_ms.max(1000) as u64)
    }

    /// 探测 loader 版本(控制传输 bRequest=0, wValue=1 读 4 字节)。
    fn probe_loader_version(&self) -> Result<u32, K230Error> {
        let mut buf = [0u8; 4];
        self.dev.control_in(0, 0x0001, &mut buf)?;
        Ok(u32::from_le_bytes(buf))
    }

    /// 批量写 + ZLP 处理(length % ep_out_mps == 0 时补发零包)。
    fn write_data(&self, data: &[u8]) -> Result<(), K230Error> {
        let timeout = self.timeout();
        let n = self.dev.bulk_write(self.ep_out, data, timeout)?;
        if n != data.len() {
            return Err(K230Error::Usb(format!(
                "bulk write short: {} != {}",
                n,
                data.len()
            )));
        }
        // ZLP:长度是端点 MaxPacketSize 整数倍时补发零长度包
        if self.ep_out_mps > 0 && data.len().is_multiple_of(self.ep_out_mps as usize) {
            self.dev.bulk_write(self.ep_out, &[], timeout)?;
        }
        Ok(())
    }

    /// 批量读。
    fn read_data(&self, buf: &mut [u8]) -> Result<usize, K230Error> {
        self.dev.bulk_read(self.ep_in, buf, self.timeout())
    }

    /// 构造 CBW(60 字节)。
    fn make_cbw(cmd: u16, data: &[u8]) -> Result<[u8; KBUNR_USB_PKT_SIZE], K230Error> {
        if data.len() > KBUNR_USB_DATA_SIZE {
            return Err(K230Error::Protocol(format!(
                "cmd data too large: {}",
                data.len()
            )));
        }
        let mut cbw = [0u8; KBUNR_USB_PKT_SIZE];
        cbw[0..2].copy_from_slice(&cmd.to_le_bytes());
        cbw[4..6].copy_from_slice(&(data.len() as u16).to_le_bytes());
        cbw[6..6 + data.len()].copy_from_slice(data);
        Ok(cbw)
    }

    /// 发送命令并接收响应(CBW -> write -> read CSW -> parse)。
    fn send_cmd(&mut self, cmd: u16, data: &[u8]) -> Result<[u8; KBUNR_USB_PKT_SIZE], K230Error> {
        let cbw = Self::make_cbw(cmd, data)?;
        self.write_data(&cbw)?;
        let mut csw = [0u8; KBUNR_USB_PKT_SIZE];
        self.read_data(&mut csw)?;
        self.parse_resp(&csw, cmd)?;
        Ok(csw)
    }

    /// 校验响应(cmd | 0x8000,result OK),错误时填充 error_msg。
    fn parse_resp(&mut self, csw: &[u8; KBUNR_USB_PKT_SIZE], cmd: u16) -> Result<(), K230Error> {
        let resp_cmd = u16::from_le_bytes([csw[0], csw[1]]);
        let result = u16::from_le_bytes([csw[2], csw[3]]);
        let data_size = u16::from_le_bytes([csw[4], csw[5]]) as usize;

        if resp_cmd != (cmd | CMD_FLAG_DEV_TO_HOST) {
            self.error_msg = "cmd recv resp cmd error".into();
            return Err(K230Error::Protocol(format!(
                "resp cmd mismatch: {:#06x} (expected {:#06x})",
                resp_cmd,
                cmd | CMD_FLAG_DEV_TO_HOST
            )));
        }
        if result != KBURN_RESULT_OK {
            if result == KBURN_RESULT_ERROR_MSG && data_size <= KBUNR_USB_DATA_SIZE {
                let msg = String::from_utf8_lossy(&csw[6..6 + data_size]).to_string();
                self.error_msg = msg.clone();
                return Err(K230Error::Protocol(msg));
            }
            self.error_msg = "cmd recv resp result error".into();
            return Err(K230Error::Protocol(format!("resp result: {}", result)));
        }
        Ok(())
    }

    /// 发 NOP 命令清设备错误状态。
    ///
    /// 全程短超时:nop 是清状态辅助操作,不应阻塞烧录流程。参考 `kburn_nop`:
    /// 先用 50ms 短读排空可能残留的响应包,再发 NONE 命令(用当前操作超时)。
    ///
    /// 关键:排空读直接用 50ms 单次 `bulk_read`,**不走 `read_data`/重试**。之前经
    /// `read_data -> bulk_read`,50ms 被 `timeout()` 的 `.max(1000)` 钳位到 1s、再 ×6 重试,
    /// 在无残留(常规情况)时每个分区白等 ~7s;且 `clear_halt` 反复打断 loader 端点,
    /// 实测会触发设备复位(write uboot 后 bulk_read 反复超时 + 设备重新枚举)。
    pub fn nop(&mut self) -> Result<(), K230Error> {
        // 排空残留响应包:50ms 短读,单次不重试(无残留时 50ms 即返回,不阻塞)。
        let mut buf = [0u8; KBUNR_USB_PKT_SIZE];
        let _ = self
            .dev
            .bulk_read(self.ep_in, &mut buf, Duration::from_millis(50));
        // 发 NONE 清设备状态,用当前操作超时(参考 kburn_nop:restore 后用原 timeout_ms)。
        // 失败不致命(nop 仅用于清状态)。
        let _ = self.send_cmd(KBURN_CMD_NONE, &[]);
        Ok(())
    }

    /// 探测目标介质,返回 (out_chunk_size, in_chunk_size)。
    pub fn probe(&mut self, target: MediumType) -> Result<(u64, u64), K230Error> {
        let data = [target as u8, 0xFF];
        let csw = self.send_cmd(KBURN_CMD_DEV_PROBE, &data)?;
        let result = &csw[6..];
        if result.len() < 16 {
            return Err(K230Error::Protocol("probe result too short".into()));
        }
        let out_chunk = u64_le(&result[0..8]);
        let in_chunk = u64_le(&result[8..16]);
        Ok((out_chunk, in_chunk))
    }

    /// 读取介质信息(DEV_GET_INFO),更新 self.medium_info。
    pub fn get_medium_info(&mut self) -> Result<&MediumInfo, K230Error> {
        let csw = self.send_cmd(KBURN_CMD_DEV_GET_INFO, &[])?;
        let data = &csw[6..];
        if data.len() < 32 {
            return Err(K230Error::Protocol("medium info too short".into()));
        }
        self.medium_info = MediumInfo {
            capacity: u64_le(&data[0..8]),
            blk_size: u64_le(&data[8..16]),
            erase_size: u64_le(&data[16..24]),
            timeout_ms: u32_le(&data[24..28]),
            wp: data[28],
            medium_type: data[29] & 0x7F,
            valid: (data[29] >> 7) & 1,
        };
        Ok(&self.medium_info)
    }

    /// 擦除介质(offset, size),支持超时重试(erase 是长操作)。
    ///
    /// `cancel` 在轮询间隙检查,使擦除期间取消能在单次读超时(最长 timeout_ms)后生效。
    pub fn erase(
        &mut self,
        offset: u64,
        size: u64,
        max_retry: u32,
        cancel: &AtomicBool,
    ) -> Result<(), K230Error> {
        if offset + size > self.medium_info.capacity {
            return Err(K230Error::Medium("erase exceed capacity".into()));
        }
        if self.medium_info.wp == 1 {
            return Err(K230Error::Medium("write protected".into()));
        }
        let cfg = make_u64_pair(offset, size);
        let cbw = Self::make_cbw(KBURN_CMD_ERASE_LBA, &cfg)?;
        self.write_data(&cbw)?;

        // 轮询读响应(超时重试,参考源码 do_sleep(3000))
        let mut csw = [0u8; KBUNR_USB_PKT_SIZE];
        let mut retry = 0;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(K230Error::Cancelled);
            }
            match self.read_data(&mut csw) {
                Ok(_) => break,
                Err(K230Error::Usb(_)) => {
                    retry += 1;
                    if retry > max_retry {
                        self.error_msg = "erase timeout".into();
                        return Err(K230Error::Medium("erase timeout".into()));
                    }
                    thread::sleep(Duration::from_millis(3000));
                }
                Err(e) => return Err(e),
            }
        }
        self.parse_resp(&csw, KBURN_CMD_ERASE_LBA)?;
        Ok(())
    }

    /// 开始写入(WRITE_LBA,携带 offset/size/max/flag)。
    ///
    /// loader_version >= 1 时带 4 个 uint64(含 part_flag),否则 3 个。
    pub fn write_start(
        &mut self,
        offset: u64,
        size: u64,
        max: u64,
        part_flag: u64,
    ) -> Result<(), K230Error> {
        if offset + size > self.medium_info.capacity {
            return Err(K230Error::Medium("write exceed capacity".into()));
        }
        if self.medium_info.wp == 1 {
            return Err(K230Error::Medium("write protected".into()));
        }
        if self.medium_info.erase_size > 0 && !offset.is_multiple_of(self.medium_info.erase_size) {
            return Err(K230Error::Medium(format!(
                "write offset {} not aligned to erase_size {}",
                offset, self.medium_info.erase_size
            )));
        }
        let mut cfg = [0u8; 32];
        cfg[0..8].copy_from_slice(&offset.to_le_bytes());
        cfg[8..16].copy_from_slice(&size.to_le_bytes());
        cfg[16..24].copy_from_slice(&max.to_le_bytes());
        let cfg_len = if self.loader_version >= 1 {
            cfg[24..32].copy_from_slice(&part_flag.to_le_bytes());
            32
        } else {
            24
        };
        self.send_cmd(KBURN_CMD_WRITE_LBA, &cfg[..cfg_len])?;
        Ok(())
    }

    /// 写入一个数据块(批量传输,不读响应)。
    pub fn write_chunk(&mut self, data: &[u8]) -> Result<(), K230Error> {
        self.write_data(data)
    }

    /// 结束写入(读 CSW 确认 + nop 清状态)。
    pub fn write_end(&mut self) -> Result<(), K230Error> {
        let mut csw = [0u8; KBUNR_USB_PKT_SIZE];
        self.read_data(&mut csw)?;
        self.parse_resp(&csw, KBURN_CMD_WRITE_LBA)?;
        self.nop()?;
        Ok(())
    }

    /// 重启芯片(发 REBOOT 命令 + magic)。
    pub fn reboot(&mut self) -> Result<(), K230Error> {
        let mut cbw = [0u8; KBUNR_USB_PKT_SIZE];
        cbw[0..2].copy_from_slice(&KBURN_CMD_REBOOT.to_le_bytes());
        cbw[4..6].copy_from_slice(&8u16.to_le_bytes());
        cbw[6..14].copy_from_slice(&KBURN_REBOOT_MAGIC.to_le_bytes());
        self.write_data(&cbw)?;
        Ok(())
    }
}

/// 构造两个 u64 拼成的 16 字节 buffer。
fn make_u64_pair(a: u64, b: u64) -> [u8; 16] {
    let mut buf = [0u8; 16];
    buf[0..8].copy_from_slice(&a.to_le_bytes());
    buf[8..16].copy_from_slice(&b.to_le_bytes());
    buf
}

#[inline]
fn u64_le(b: &[u8]) -> u64 {
    u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

#[inline]
fn u32_le(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}
