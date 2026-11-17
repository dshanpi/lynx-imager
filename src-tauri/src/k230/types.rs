//! K230 烧录公共类型。

use serde::{Deserialize, Serialize};

/// K230 USB 设备 VID/PID。
///
/// K230 BootROM 与 UBOOT(loader)阶段共用同一 VID/PID,靠 CPU info 字符串区分阶段。
pub const K230_USB_VID: u16 = 0x29F1;
pub const K230_USB_PID: u16 = 0x0230;

/// K230 烧录目标介质类型。
///
/// 对应 K230 USB BootROM loader 支持的五种存储介质,与参考源码
/// `kburnUsbIspCommandTaget` 枚举一致。序列化为 snake_case 供前端使用。
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MediumType {
    /// 接在 K230 SDIO0 接口的 EMMC 或 SD 卡
    #[default]
    Emmc = 0x01,
    /// 接在 K230 SDIO1 接口的 EMMC 或 SD 卡
    SdCard = 0x02,
    /// SPI 总线上的 NAND Flash
    SpiNand = 0x03,
    /// SPI 总线上的 NOR Flash
    SpiNor = 0x04,
    /// 芯片内置 OTP 设备
    Otp = 0x05,
}

impl MediumType {
    /// 根据介质选择对应的 loader 二进制。
    ///
    /// EMMC/SDCARD 共用 mmc loader;SPI_NAND 单独;SPI_NOR/OTP 共用 spi_nor loader。
    pub fn loader(&self) -> &'static [u8] {
        match self {
            MediumType::Emmc | MediumType::SdCard => super::LOADER_MMC,
            MediumType::SpiNand => super::LOADER_SPI_NAND,
            MediumType::SpiNor | MediumType::Otp => super::LOADER_SPI_NOR,
        }
    }
}

/// K230 USB 设备启动阶段。
///
/// 设备插入后先处于 BootROM 阶段,下载 loader 并跳转后重新枚举为 UBOOT 阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[allow(dead_code)]
pub enum DeviceStage {
    /// BootROM 阶段:裸芯片,需下载 loader 到 SRAM(0x80360000)并跳转
    Brom,
    /// UBOOT 阶段:loader 已运行,可进行介质探测与烧录
    Uboot,
    /// 未知阶段(握手未识别)
    Unknown,
}

/// 扫描到的 K230 设备信息。
///
/// 对应前端 `FlashDevice` 契约(device_id/bus/port/mode),保持与原 efex_scan_devices
/// 返回结构兼容,前端改动最小化。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct K230Device {
    /// 设备句柄 ID(全局单调递增,对应原 device_id)
    pub device_id: u32,
    /// USB 总线号
    pub bus: u8,
    /// USB 端口号
    pub port: u8,
    /// 设备路径(总线-端口格式,如 "1-1",用于多设备精确寻址)
    pub path: String,
    /// 当前启动阶段
    pub stage: DeviceStage,
}
