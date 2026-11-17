//! K230 USB 设备层(rusb)。
//!
//! 扫描/打开/claim K230 USB 设备(VID=0x29F1, PID=0x0230),提供控制传输与批量传输原语,
//! 以及端点动态发现(UBOOT 阶段用)。
//!
//! 与参考源码 libusb 调用 1:1 映射:`refer/k230_flash/src/kburn/kburn.cpp`(USB 枚举/打开)、
//! `burner_uboot.cpp::__get_endpoint`(端点发现)。

use std::thread;
use std::time::Duration;

use log::debug;
use rusb::{Direction, Recipient, RequestType, TransferType};

use super::super::error::K230Error;
use super::super::types::{K230_USB_PID, K230_USB_VID};

/// 默认 USB 传输超时(与参考源码 USB_TIMEOUT 一致)。
pub const DEFAULT_TIMEOUT: Duration = Duration::from_millis(1000);

/// bulk 传输错误重试次数(仅对 STALL/Pipe error 生效)。
///
/// 多设备并行烧录时,接入新设备会触发 `rusb::devices()` 全量枚举 + `device.open()`,
/// 在 Windows 上会瞬时打断同 USB 控制器下其他设备的 bulk 管道(表现为 Pipe error / STALL)。
/// 对 STALL 做 `clear_halt` + 重试可度过该瞬时干扰窗口。
///
/// 注意:**仅 STALL 重试**。超时与其他错误立即返回(与参考源码 `kburn_write_data`/
/// `kburn_read_data` 单次传输一致)——重试超时会放大延迟,且 `clear_halt` 会丢弃设备
/// 已发出的响应导致协议 desync(实测表现为 write uboot 后 bulk_read 反复超时 + 设备复位)。
const USB_BULK_MAX_RETRIES: u32 = 6;

/// 重试退避延迟(指数退避: 50/100/200/400/800/1600ms),覆盖新设备接入的 STALL 干扰窗口。
fn bulk_retry_delay(attempt: u32) -> Duration {
    let ms = 50u64 << attempt.min(5);
    Duration::from_millis(ms)
}

/// K230 USB 设备句柄,封装 rusb `DeviceHandle`。
///
/// 生命周期:打开后 claim interface 0,drop 时自动 release。BROM 与 UBOOT 阶段共用,
/// 靠 CPU info 握手区分(BROM 阶段端点硬编码 0x01/0x81,UBOOT 阶段动态发现)。
pub struct K230UsbDevice {
    pub handle: rusb::DeviceHandle<rusb::GlobalContext>,
    #[allow(dead_code)]
    pub bus: u8,
    #[allow(dead_code)]
    pub port: u8,
}

/// 控制传输 bmRequestType 构造(vendor, device)。
#[inline]
fn req_type_out() -> u8 {
    rusb::request_type(Direction::Out, RequestType::Vendor, Recipient::Device)
}

#[inline]
fn req_type_in() -> u8 {
    rusb::request_type(Direction::In, RequestType::Vendor, Recipient::Device)
}

impl K230UsbDevice {
    /// 扫描所有 K230 USB 设备,返回 (bus, port) 列表。
    pub fn list_all() -> Result<Vec<(u8, u8)>, K230Error> {
        let devices = rusb::devices().map_err(|e| K230Error::Usb(e.to_string()))?;
        let mut result = Vec::new();
        for device in devices.iter() {
            let desc = match device.device_descriptor() {
                Ok(d) => d,
                Err(_) => continue,
            };
            if desc.vendor_id() == K230_USB_VID && desc.product_id() == K230_USB_PID {
                result.push((device.bus_number(), device.port_number()));
            }
        }
        Ok(result)
    }

    /// 按 bus/port 打开设备并 claim interface 0。
    ///
    /// 使用 `set_auto_detach_kernel_driver` 自动 detach 内核驱动(Linux),Windows 上若已
    /// 装 WinUSB 驱动可直接 claim(参考 README 用 Zadig 安装)。
    pub fn open(bus: u8, port: u8) -> Result<Self, K230Error> {
        let devices = rusb::devices().map_err(|e| K230Error::Usb(e.to_string()))?;
        for device in devices.iter() {
            let desc = match device.device_descriptor() {
                Ok(d) => d,
                Err(_) => continue,
            };
            if desc.vendor_id() == K230_USB_VID
                && desc.product_id() == K230_USB_PID
                && device.bus_number() == bus
                && device.port_number() == port
            {
                let handle = device
                    .open()
                    .map_err(|e| K230Error::Usb(format!("open failed: {}", e)))?;
                // 自动 detach 内核驱动(失败可忽略,Windows WinUSB 不需要)
                let _ = handle.set_auto_detach_kernel_driver(true);
                handle
                    .claim_interface(0)
                    .map_err(|e| K230Error::Usb(format!("claim interface failed: {}", e)))?;
                return Ok(Self { handle, bus, port });
            }
        }
        Err(K230Error::DeviceNotFound(format!("{}-{}", bus, port)))
    }

    /// 控制传输(OUT,vendor request):设置地址/跳转等。
    ///
    /// 地址编码:`wValue = addr >> 16`(高 16 位),`wIndex = addr & 0xffff`(低 16 位),
    /// 与参考源码 `U32_HIGH_U16`/`U32_LOW_U16` 宏一致。
    pub fn control_out(&self, request: u8, addr: u32) -> Result<(), K230Error> {
        self.handle
            .write_control(
                req_type_out(),
                request,
                (addr >> 16) as u16,
                (addr & 0xffff) as u16,
                &[],
                DEFAULT_TIMEOUT,
            )
            .map_err(|e| K230Error::Usb(format!("control_out(req={}) failed: {}", request, e)))?;
        Ok(())
    }

    /// 控制传输(IN,vendor request):读 CPU info(握手)/ loader version 等。
    pub fn control_in(&self, request: u8, value: u16, buf: &mut [u8]) -> Result<usize, K230Error> {
        self.handle
            .read_control(req_type_in(), request, value, 0, buf, DEFAULT_TIMEOUT)
            .map_err(|e| K230Error::Usb(format!("control_in(req={}) failed: {}", request, e)))
    }

    /// 批量写(指定 OUT 端点)。
    ///
    /// 仅对 STALL(Pipe error)做 `clear_halt` + 重试,度过接入新设备导致的瞬时管道干扰;
    /// 超时/其他错误立即返回(参考 `kburn_write_data` 单次传输),避免重试超时放大延迟、
    /// 以及 `clear_halt` 丢弃设备已发响应造成协议 desync。
    pub fn bulk_write(
        &self,
        endpoint: u8,
        data: &[u8],
        timeout: Duration,
    ) -> Result<usize, K230Error> {
        for attempt in 0..USB_BULK_MAX_RETRIES {
            match self.handle.write_bulk(endpoint, data, timeout) {
                Ok(n) => return Ok(n),
                Err(rusb::Error::Pipe) => {
                    debug!(
                        "bulk_write(ep={:#04x}) STALL {}/{}, clear_halt+retry",
                        endpoint,
                        attempt + 1,
                        USB_BULK_MAX_RETRIES
                    );
                    // 清除端点 STALL(Pipe error)后重试
                    let _ = self.handle.clear_halt(endpoint);
                    thread::sleep(bulk_retry_delay(attempt));
                }
                Err(e) => {
                    return Err(K230Error::Usb(format!(
                        "bulk_write(ep={:#04x}) failed: {}",
                        endpoint, e
                    )));
                }
            }
        }
        Err(K230Error::Usb(format!(
            "bulk_write(ep={:#04x}) failed: STALL persisted (after {} retries)",
            endpoint, USB_BULK_MAX_RETRIES
        )))
    }

    /// 批量读(指定 IN 端点)。返回实际读取字节数。
    ///
    /// 仅对 STALL(Pipe error)做 `clear_halt` + 重试;超时/其他错误立即返回
    /// (参考 `kburn_read_data` 单次传输),避免重试超时放大延迟、`clear_halt` 丢响应。
    pub fn bulk_read(
        &self,
        endpoint: u8,
        buf: &mut [u8],
        timeout: Duration,
    ) -> Result<usize, K230Error> {
        for attempt in 0..USB_BULK_MAX_RETRIES {
            match self.handle.read_bulk(endpoint, buf, timeout) {
                Ok(n) => return Ok(n),
                Err(rusb::Error::Pipe) => {
                    debug!(
                        "bulk_read(ep={:#04x}) STALL {}/{}, clear_halt+retry",
                        endpoint,
                        attempt + 1,
                        USB_BULK_MAX_RETRIES
                    );
                    let _ = self.handle.clear_halt(endpoint);
                    thread::sleep(bulk_retry_delay(attempt));
                }
                Err(e) => {
                    return Err(K230Error::Usb(format!(
                        "bulk_read(ep={:#04x}) failed: {}",
                        endpoint, e
                    )));
                }
            }
        }
        Err(K230Error::Usb(format!(
            "bulk_read(ep={:#04x}) failed: STALL persisted (after {} retries)",
            endpoint, USB_BULK_MAX_RETRIES
        )))
    }

    /// 动态发现 bulk IN/OUT 端点及 OUT 端点 MaxPacketSize(UBOOT 阶段)。
    ///
    /// 遍历 active config descriptor 的所有 interface/altsetting/endpoint,挑选 bulk 传输的
    /// IN/OUT 端点。返回 (ep_in, ep_out, ep_out_mps)。
    pub fn find_bulk_endpoints(&self) -> Result<(u8, u8, u16), K230Error> {
        let device = self.handle.device();
        let config = device
            .active_config_descriptor()
            .map_err(|e| K230Error::Usb(format!("active_config_descriptor failed: {}", e)))?;
        let mut ep_in = 0u8;
        let mut ep_out = 0u8;
        let mut ep_out_mps = 0u16;
        for iface in config.interfaces() {
            for setting in iface.descriptors() {
                for ep in setting.endpoint_descriptors() {
                    if ep.transfer_type() != TransferType::Bulk {
                        continue;
                    }
                    match ep.direction() {
                        Direction::In => ep_in = ep.address(),
                        Direction::Out => {
                            ep_out = ep.address();
                            ep_out_mps = ep.max_packet_size();
                        }
                    }
                }
            }
        }
        if ep_in == 0 || ep_out == 0 {
            return Err(K230Error::Usb("bulk endpoint not found".into()));
        }
        Ok((ep_in, ep_out, ep_out_mps))
    }
}
