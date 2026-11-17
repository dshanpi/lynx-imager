//! K230 BROM 阶段 USB 协议。
//!
//! BROM(BootROM)阶段:芯片裸启动,通过 USB 控制+批量传输下载 loader 到 SRAM
//! (0x80360000),然后跳转执行。loader 运行后设备重新枚举为 UBOOT 阶段。
//!
//! 协议:
//! - 控制传输(vendor request)设置地址/跳转
//! - 批量传输端点 0x01 传输 loader 数据(每页 1000 字节)
//! - 握手:控制传输读 32 字节 CPU info,"K230"=BROM,"Uboot Stage for K230"=UBOOT
//!
//! 参考:`refer/k230_flash/src/kburn/burner_k230/burner_brom.cpp`、
//! `refer/k230_burning_tool/canaan-burn/burn/k230/k230_burn.isp.c`。

use super::super::error::K230Error;
use super::usb::{K230UsbDevice, DEFAULT_TIMEOUT};

/// BROM 阶段 OUT 端点(批量传输写 loader 数据)。
pub const KENDRYTE_OUT_ENDPOINT: u8 = 0x01;
/// BROM 阶段 IN 端点。
#[allow(dead_code)]
pub const KENDRYTE_IN_ENDPOINT: u8 = 0x81;

/// BROM 控制传输 vendor request 命令码。
///
/// 通过 `bmRequestType = OUT|VENDOR|DEVICE` 的控制传输发送,`wValue`/`wIndex` 编码地址
/// 的高低 16 位(`wValue = addr >> 16`,`wIndex = addr & 0xffff`)。
pub const EP0_GET_CPU_INFO: u8 = 0;
pub const EP0_SET_DATA_ADDRESS: u8 = 1;
#[allow(dead_code)]
pub const EP0_SET_DATA_LENGTH: u8 = 2;
pub const EP0_PROG_START: u8 = 4;

/// loader 默认加载地址(K230 SRAM 区域)。
///
/// 允许范围 0x80300000 - 0x80400000,默认 0x80360000。
pub const K230_SRAM_APP_ADDR: u32 = 0x80360000;

/// BROM 每页传输大小(字节,十进制 1000,非 1024)。
pub const K230_SRAM_PAGE_SIZE: usize = 1000;

/// CPU info 握手标识字符串。
pub const BROM_CPU_INFO: &str = "K230";
pub const UBOOT_CPU_INFO: &str = "Uboot Stage for K230";

/// BROM 烧录器,持有已打开的 BROM 阶段设备句柄。
pub struct BromBurner<'a> {
    pub dev: &'a K230UsbDevice,
}

impl<'a> BromBurner<'a> {
    pub fn new(dev: &'a K230UsbDevice) -> Self {
        Self { dev }
    }

    /// 读取 CPU info(32 字节),判定设备阶段。
    ///
    /// 返回握手字符串前缀对应的阶段判定:
    /// - "K230" -> BROM 阶段
    /// - "Uboot Stage for K230" -> UBOOT 阶段
    pub fn get_cpu_info(&self) -> Result<[u8; 32], K230Error> {
        let mut buf = [0u8; 32];
        self.dev.control_in(EP0_GET_CPU_INFO, 0, &mut buf)?;
        Ok(buf)
    }

    /// 设置数据写入地址(控制传输 SET_DATA_ADDRESS)。
    pub fn set_data_addr(&self, addr: u32) -> Result<(), K230Error> {
        self.dev.control_out(EP0_SET_DATA_ADDRESS, addr)
    }

    /// 跳转到指定地址执行 loader(控制传输 PROG_START)。
    pub fn boot_from(&self, addr: u32) -> Result<(), K230Error> {
        self.dev.control_out(EP0_PROG_START, addr)
    }

    /// 下载 loader 到 SRAM 并跳转执行。
    ///
    /// 流程(对应参考 `K230BROMBurner::write` + `boot_from`):
    /// 1. `set_data_addr(K230_SRAM_APP_ADDR)` 设置写入地址
    /// 2. 按 1000 字节分页,批量传输到端点 0x01
    /// 3. `boot_from(K230_SRAM_APP_ADDR)` 跳转执行
    ///
    /// `on_progress(current, total)` 回调用于报告 loader 下载进度。
    pub fn download_and_boot<F>(&self, loader: &[u8], on_progress: F) -> Result<(), K230Error>
    where
        F: Fn(usize, usize),
    {
        let addr = K230_SRAM_APP_ADDR;
        let total = loader.len();

        // 1. 设置写入地址
        self.set_data_addr(addr)?;

        // 2. 按 1000B 分页批量写入
        let pages = total.div_ceil(K230_SRAM_PAGE_SIZE);
        for page in 0..pages {
            let offset = page * K230_SRAM_PAGE_SIZE;
            let end = (offset + K230_SRAM_PAGE_SIZE).min(total);
            let chunk = &loader[offset..end];
            let written = self
                .dev
                .bulk_write(KENDRYTE_OUT_ENDPOINT, chunk, DEFAULT_TIMEOUT)?;
            if written != chunk.len() {
                return Err(K230Error::Usb(format!(
                    "bulk write short: {} != {}",
                    written,
                    chunk.len()
                )));
            }
            on_progress(end, total);
        }

        // 3. 跳转执行 loader
        self.boot_from(addr)?;

        Ok(())
    }
}

/// 判定 CPU info 缓冲对应的设备阶段。
///
/// 返回 Some(true) 表示 UBOOT(loader 已运行),Some(false) 表示 BROM(裸芯片),
/// None 表示未识别。
pub fn detect_stage(cpu_info: &[u8]) -> Option<bool> {
    let end = cpu_info
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(cpu_info.len());
    let s = std::str::from_utf8(&cpu_info[..end]).ok()?;
    if s.starts_with(UBOOT_CPU_INFO) {
        Some(true) // UBOOT
    } else if s.starts_with(BROM_CPU_INFO) {
        Some(false) // BROM
    } else {
        None
    }
}
