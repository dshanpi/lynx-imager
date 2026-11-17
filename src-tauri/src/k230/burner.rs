//! K230 烧录器状态机。
//!
//! 编排 BROM(loader 下载)-> UBOOT(介质烧录)完整流程:
//! 1. 打开 BROM 设备 -> 握手确认 "K230"
//! 2. 按介质选择 loader -> 下载到 SRAM(0x80360000) -> boot_from 跳转
//! 3. 等待设备重新枚举为 UBOOT(同 VID/PID,CPU info 变为 "Uboot Stage for K230")
//! 4. UBOOT 阶段:probe 介质 -> get_info -> 逐分区 erase + write_stream
//! 5. 可选 reboot
//!
//! 参考:`refer/k230_flash/src/kburn/burner_k230/burner.cpp`、
//! `refer/k230_burning_tool/canaan-burn/burn/k230/k230_burn.c`。

use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use log::{info, warn};

use super::error::K230Error;
use super::kdimage::KdImage;
use super::protocol::brom::{detect_stage, BromBurner};
use super::protocol::uboot::UbootBurner;
use super::protocol::usb::K230UsbDevice;
use super::types::MediumType;

/// 烧录进度回调:`(current, total, stage_label)`。
pub type ProgressFn = Box<dyn Fn(usize, usize, &str) + Send>;

/// K230 烧录器,编排 BROM -> UBOOT 完整流程。
pub struct Burner {
    pub target: MediumType,
}

impl Burner {
    pub fn new(target: MediumType) -> Self {
        Self { target }
    }

    /// 执行完整烧录流程:打开 BROM -> 下载 loader -> 等待 UBOOT -> 烧录分区。
    ///
    /// `bus`/`port` 定位目标设备(支持多设备并行时各自独立寻址)。
    /// `cancel` 为取消标志:在各阶段间隙(握手后/loader 下载前/等待 UBOOT/逐分区/分块)轮询,
    /// 被置位时立即以 `K230Error::Cancelled` 中止,避免取消后仍跑完整轮烧录。
    pub fn burn(
        &self,
        bus: u8,
        port: u8,
        image: &KdImage,
        on_progress: &ProgressFn,
        reboot: bool,
        cancel: &AtomicBool,
    ) -> Result<(), K230Error> {
        // 1. 打开 BROM 设备并握手
        on_progress(0, 1, "handshake");
        let dev = K230UsbDevice::open(bus, port)?;
        let brom = BromBurner::new(&dev);
        let cpu_info = brom.get_cpu_info()?;
        on_progress(1, 1, "handshake");

        match detect_stage(&cpu_info) {
            Some(false) => {} // BROM 阶段,继续下载 loader
            Some(true) => {
                // 已是 UBOOT(loader 已运行),直接进入烧录
                return self.burn_uboot_with_recovery(
                    bus,
                    port,
                    dev,
                    image,
                    on_progress,
                    reboot,
                    cancel,
                );
            }
            _ => {
                let end = cpu_info
                    .iter()
                    .position(|&b| b == 0)
                    .unwrap_or(cpu_info.len());
                let info = String::from_utf8_lossy(&cpu_info[..end]).to_string();
                return Err(K230Error::Protocol(format!(
                    "unknown device stage (cpu_info={:?})",
                    info
                )));
            }
        }

        // 2. 下载 loader(ISP)到 SRAM 并跳转
        check_cancel(cancel)?;
        let loader = self.target.loader();
        on_progress(0, loader.len(), "load_loader");
        brom.download_and_boot(loader, |cur, total| {
            on_progress(cur, total, "load_loader");
        })?;

        // 3. 释放 BROM 句柄,等待设备重新枚举为 UBOOT(同 bus/port)
        // K230 BROM→UBOOT 复位实测 ~300-500ms,先固定等待避免无谓的全量 USB 枚举
        drop(dev);
        on_progress(0, 1, "wait_uboot");
        thread::sleep(Duration::from_millis(300));

        // 4. 等待并打开 UBOOT 设备(固定等待后最多 5 次快速重试)
        let dev = self.wait_uboot(bus, port, cancel)?;
        on_progress(1, 1, "wait_uboot");

        // 5. UBOOT 阶段烧录(带重开恢复)
        self.burn_uboot_with_recovery(bus, port, dev, image, on_progress, reboot, cancel)
    }

    /// UBOOT 烧录,带一次重开恢复。
    ///
    /// 同控制器多设备并行时,其他设备 BROM->UBOOT 切换的瞬时 `libusb_get_device_list`
    /// 枚举可能打断本设备 USB 传输,导致设备状态 desync(写数据/CSW 丢失,bulk_read 超时)。
    /// Fix 6 的 clear_halt 重试只能恢复 STALL,不能恢复 desync。此处遇到 USB 错误时
    /// 重新打开 UBOOT 设备(拿到干净状态)并重试一次烧录(覆盖写,对 Flash 安全)。
    #[allow(clippy::too_many_arguments)]
    fn burn_uboot_with_recovery(
        &self,
        bus: u8,
        port: u8,
        dev: K230UsbDevice,
        image: &KdImage,
        on_progress: &ProgressFn,
        reboot: bool,
        cancel: &AtomicBool,
    ) -> Result<(), K230Error> {
        match self.burn_uboot(&dev, image, on_progress, reboot, cancel) {
            Ok(()) => Ok(()),
            Err(e) if Self::is_recoverable_usb_err(&e) => {
                // 取消则不再重试(避免取消后还重开设备重烧一次)
                check_cancel(cancel)?;
                warn!(
                    "burn_uboot failed ({}), re-opening UBOOT device to retry once",
                    e
                );
                drop(dev);
                on_progress(0, 1, "resync");
                // 设备仍在线只是 desync,重新等待并打开 UBOOT 设备,再烧一次(覆盖写)
                match self.wait_uboot(bus, port, cancel) {
                    Ok(dev2) => self.burn_uboot(&dev2, image, on_progress, reboot, cancel),
                    Err(wait_err) => {
                        warn!("re-open after recovery failed: {}", wait_err);
                        Err(e)
                    }
                }
            }
            Err(e) => Err(e),
        }
    }

    /// 是否可通过重开设备恢复的 USB 错误(超时/IO/STALL 等,非协议逻辑错误)。
    fn is_recoverable_usb_err(e: &K230Error) -> bool {
        matches!(e, K230Error::Usb(_))
    }

    /// 等待 UBOOT 设备出现(固定等待后少量重试,最多 ~2s)。
    ///
    /// 调用前已 sleep 300ms 等待设备复位。此处最多 5 次重试,间隔 100ms,
    /// 每次 `open()` 内部的全量 USB 枚举本身耗时即充当自然退避。
    /// 参考 `k230_burning_tool` 事件驱动架构:UBOOT 设备由 USB monitor 异步推送,
    /// 无需长时间轮询;我们的固定等待+少量重试模拟同等效果。
    fn wait_uboot(
        &self,
        bus: u8,
        port: u8,
        cancel: &AtomicBool,
    ) -> Result<K230UsbDevice, K230Error> {
        for attempt in 0..5 {
            check_cancel(cancel)?;
            if attempt > 0 {
                thread::sleep(Duration::from_millis(100));
            }
            if let Ok(dev) = K230UsbDevice::open(bus, port) {
                let brom = BromBurner::new(&dev);
                if let Ok(cpu_info) = brom.get_cpu_info() {
                    if detect_stage(&cpu_info) == Some(true) {
                        return Ok(dev);
                    }
                }
            }
        }
        Err(K230Error::Protocol(
            "wait uboot re-enumerate timeout".into(),
        ))
    }

    /// UBOOT 阶段烧录:probe 介质 -> get_info -> 逐分区 erase + write。
    fn burn_uboot(
        &self,
        dev: &K230UsbDevice,
        image: &KdImage,
        on_progress: &ProgressFn,
        reboot: bool,
        cancel: &AtomicBool,
    ) -> Result<(), K230Error> {
        let t = Instant::now();
        let mut uboot = UbootBurner::new(dev)?;
        info!(
            "[K230] UBOOT init (endpoints/loader_ver/nop) done in {:?}",
            t.elapsed()
        );

        // probe 介质并读取介质信息(probe 返回 loader 建议的 out/in chunk size)
        let t = Instant::now();
        let (out_chunk, _in_chunk) = uboot.probe(self.target)?;
        info!(
            "[K230] UBOOT probe done in {:?} (out_chunk={}, in_chunk={})",
            t.elapsed(),
            out_chunk,
            _in_chunk
        );
        let t = Instant::now();
        uboot.get_medium_info()?;
        info!(
            "[K230] UBOOT get_medium_info done in {:?} (capacity={}, blk_size={}, erase_size={}, timeout_ms={})",
            t.elapsed(),
            uboot.medium_info.capacity,
            uboot.medium_info.blk_size,
            uboot.medium_info.erase_size,
            uboot.medium_info.timeout_ms
        );

        let blk_size = uboot.medium_info.blk_size.max(1) as usize;
        // 用 probe 返回的 out_chunk_size 分块(参考 write_stream: chunk_size = out_chunk_size),
        // 避免单次写入超过 loader 的 USB 接收缓冲引发 STALL;0 时回退 blk_size*512。
        let chunk_size = if out_chunk > 0 {
            out_chunk as usize
        } else {
            blk_size * 512
        };

        // 逐分区烧录
        for (i, part) in image.parts.iter().enumerate() {
            check_cancel(cancel)?;
            let label = format!("write_partition_{}", i);

            // 读取分区数据(含 SHA256 校验)--先读以获得实际写入大小,用于计算尾部擦除范围
            let data = image.read_part_data(part)?;
            info!(
                "[K230] part {} '{}': offset={} data_size={} aligned={} part_erase_size={} part_max={} flag={:#x}",
                i,
                part.part_name,
                part.part_offset,
                data.len(),
                data.len().div_ceil(blk_size) * blk_size,
                part.part_erase_size,
                part.part_max_size,
                part.part_flag
            );

            // 擦除"剩余尾部":仅擦除 [offset+data_size, offset+part_erase_size) 对齐到 erase_size
            // 的部分。参考 k230_flash.cpp:数据区由 loader 在 WRITE_LBA 内部自擦除,这里只补擦写入
            // 数据之后的剩余空间。旧实现擦除 [offset, offset+part_erase_size) 整段(含数据区),在数据
            // 填满擦除范围的大分区上会冗余擦除整段数据区,耗时数十秒(进度卡在分区起点 7% 久不动)。
            if part.part_erase_size > 0 && uboot.medium_info.erase_size > 0 {
                let base = part.part_offset as u64;
                let es = uboot.medium_info.erase_size;
                let erase_start = round_up(base + data.len() as u64, es);
                let erase_end = round_down(base + part.part_erase_size as u64, es);
                if erase_end > erase_start {
                    on_progress(0, 1, &format!("erase_partition_{}", i));
                    let t = Instant::now();
                    info!(
                        "[K230] part {} erase tail [{}..{}) size={} start",
                        i,
                        erase_start,
                        erase_end,
                        erase_end - erase_start
                    );
                    uboot.erase(erase_start, erase_end - erase_start, 30, cancel)?;
                    info!("[K230] part {} erase done in {:?}", i, t.elapsed());
                } else {
                    info!("[K230] part {} erase skipped (tail empty)", i);
                }
            }

            // 对齐到块大小(不足部分用 0 填充)
            let aligned_size = data.len().div_ceil(blk_size) * blk_size;

            // 开始写入
            let t = Instant::now();
            uboot.write_start(
                part.part_offset as u64,
                aligned_size as u64,
                part.part_max_size as u64,
                part.part_flag,
            )?;
            info!("[K230] part {} write_start done in {:?}", i, t.elapsed());
            // 参考源码 write_start 后 do_sleep(100)
            thread::sleep(Duration::from_millis(100));

            on_progress(0, data.len(), &label);

            // 分块写入(chunk_size 在循环外按 probe 的 out_chunk_size 计算)
            let t = Instant::now();
            let mut sent = 0;
            while sent < data.len() {
                check_cancel(cancel)?;
                let end = (sent + chunk_size).min(data.len());
                uboot.write_chunk(&data[sent..end])?;
                sent = end;
                on_progress(sent, data.len(), &label);
            }
            info!(
                "[K230] part {} write {} chunks (chunk_size={}) done in {:?}",
                i,
                (data.len() + chunk_size - 1) / chunk_size.max(1),
                chunk_size,
                t.elapsed()
            );

            // 填充对齐部分(0xFF,参考源码用 0 填充,这里用 0xFF 符合 Flash 语义)
            let pad = aligned_size - data.len();
            if pad > 0 {
                let pad_buf = vec![0xFFu8; pad];
                uboot.write_chunk(&pad_buf)?;
            }

            // 结束写入(读 CSW 确认 + nop)
            let t = Instant::now();
            uboot.write_end()?;
            info!("[K230] part {} write_end done in {:?}", i, t.elapsed());
        }

        // 烧录完成后重启(参考 canaan-burn kburn_reset_chip:REBOOT 命令 + magic 0x52626F74)
        if reboot {
            on_progress(0, 1, "reboot");
            uboot.reboot()?;
            on_progress(1, 1, "reboot");
        }

        Ok(())
    }
}

/// 取消标志检查:被置位时返回 `K230Error::Cancelled` 中止烧录。
///
/// 在各阶段间隙(分区切换、分块之间、erase 轮询、等待 UBOOT)调用,使取消能在
/// 当前 USB 传输(单次最长 timeout_ms)结束后即刻生效,而非跑完整轮烧录。
fn check_cancel(cancel: &AtomicBool) -> Result<(), K230Error> {
    if cancel.load(Ordering::Relaxed) {
        return Err(K230Error::Cancelled);
    }
    Ok(())
}

/// 向上对齐到 multiple 的整数倍(multiple > 0)。参考源码 round_up。
fn round_up(v: u64, m: u64) -> u64 {
    v.div_ceil(m) * m
}

/// 向下对齐到 multiple 的整数倍(multiple > 0)。参考源码 round_down。
fn round_down(v: u64, m: u64) -> u64 {
    v - v % m
}
