use std::mem;
use std::sync::Arc;
use tauri::{command, AppHandle, Manager, State};
use windows::core::{s, w, PCSTR};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

use crate::ch347::Ch347State;

// --- CH347 FFI types ---

type FnOpen = unsafe extern "system" fn(u32) -> HANDLE;
type FnClose = unsafe extern "system" fn(u32) -> i32;
type FnStreamI2C = unsafe extern "system" fn(u32, u32, *const u8, u32, *mut u8) -> i32;
type FnStreamI2cRetAck =
    unsafe extern "system" fn(u32, u32, *const u8, u32, *mut u8, *mut u32) -> i32;
type FnI2CSet = unsafe extern "system" fn(u32, u32) -> i32;
type FnI2CSetStretch = unsafe extern "system" fn(u32, i32) -> i32;
type FnI2CSetDelaymS = unsafe extern "system" fn(u32, u32) -> i32;
type FnSpiInit = unsafe extern "system" fn(u32, *const SpiCfg) -> i32;
type FnSpiWriteRead = unsafe extern "system" fn(u32, u32, u32, *mut u8) -> i32;
type FnSpiWrite = unsafe extern "system" fn(u32, u32, u32, u32, *mut u8) -> i32;
type FnSpiRead = unsafe extern "system" fn(u32, u32, u32, *mut u32, *mut u8) -> i32;
type FnSpiSetFrequency = unsafe extern "system" fn(u32, u32) -> i32;
type FnSpiSetDataBits = unsafe extern "system" fn(u32, u8) -> i32;
type FnSpiChangeCS = unsafe extern "system" fn(u32, u8) -> i32;
type FnSpiSetChipSelect = unsafe extern "system" fn(u32, u16, u16, u32, u32, u32) -> i32;
type FnSpiGetCfg = unsafe extern "system" fn(u32, *mut SpiCfg) -> i32;
type FnStreamSPI4 = unsafe extern "system" fn(u32, u32, u32, *mut u8) -> i32;
type FnGpioSet = unsafe extern "system" fn(u32, u8, u8, u8) -> i32;
type FnSetTimeout = unsafe extern "system" fn(u32, u32, u32) -> i32;
type FnGetDeviceInfor = unsafe extern "system" fn(u32, *mut DevInfor) -> i32;
type FnGetChipType = unsafe extern "system" fn(u32) -> u8;

// 对应 CH347DLL.H 的 mSpiCfgS，官方用 #pragma pack(1) 1字节对齐。
// 必须用 packed，否则 #[repr(C)] 默认按自然对齐填充，chip_select 等字段会错位，
// 导致 CH347SPI_Init 收到错误的 CS/autoDeactiveCS 配置 → SPI 行为异常 → 屏幕乱码。
#[repr(C, packed)]
#[derive(Clone)]
struct SpiCfg {
    mode: u8,
    clock: u8,
    byte_order: u8,
    write_read_interval: u16,
    out_default_data: u8,
    chip_select: u32,
    cs1_polarity: u8,
    cs2_polarity: u8,
    is_auto_deactive_cs: u16,
    active_delay: u16,
    delay_deactive: u32,
}

#[repr(C)]
struct DevInfor {
    index: u8,
    device_path: [u8; 260],
    usb_class: u8,
    func_type: u8,
    device_id: [u8; 64],
    chip_mode: u8,
    dev_handle: HANDLE,
    bulk_out_max_size: u16,
    bulk_in_max_size: u16,
    usb_speed_type: u8,
    ch347_if_num: u8,
    data_up_endp: u8,
    data_dn_endp: u8,
    product_string: [u8; 64],
    manufacturer_string: [u8; 64],
    write_timeout: u32,
    read_timeout: u32,
    func_desc_str: [u8; 64],
    firmware_ver: u8,
}

fn ptr_to_str(bytes: &[u8]) -> String {
    String::from_utf8_lossy(
        &bytes
            .iter()
            .take_while(|&&b| b != 0)
            .copied()
            .collect::<Vec<u8>>(),
    )
    .to_string()
}

struct Ch347Ffi {
    open: FnOpen,
    close: FnClose,
    stream_i2c: FnStreamI2C,
    stream_i2c_ret_ack: FnStreamI2cRetAck,
    i2c_set: FnI2CSet,
    i2c_set_stretch: FnI2CSetStretch,
    i2c_set_delay_ms: FnI2CSetDelaymS,
    spi_init: FnSpiInit,
    spi_write_read: FnSpiWriteRead,
    spi_write: FnSpiWrite,
    spi_read: FnSpiRead,
    spi_set_frequency: FnSpiSetFrequency,
    spi_set_data_bits: FnSpiSetDataBits,
    spi_change_cs: FnSpiChangeCS,
    spi_set_chip_select: FnSpiSetChipSelect,
    spi_get_cfg: FnSpiGetCfg,
    spi_stream4: FnStreamSPI4,
    gpio_set: FnGpioSet,
    set_timeout: FnSetTimeout,
    get_device_infor: FnGetDeviceInfor,
    get_chip_type: FnGetChipType,
}

impl Ch347Ffi {
    unsafe fn load() -> Option<Self> {
        // Try 64-bit DLL first, then 32-bit
        let lib = LoadLibraryW(w!("CH347DLLA64.dll"))
            .or_else(|_| LoadLibraryW(w!("CH347DLL.dll")))
            .ok()?;
        log::info!("CH347 DLL loaded, resolving functions...");
        let get = |name: PCSTR| -> Option<unsafe extern "system" fn() -> isize> {
            unsafe { GetProcAddress(lib, name) }
        };
        Some(Self {
            open: mem::transmute(get(s!("CH347OpenDevice"))?),
            close: mem::transmute(get(s!("CH347CloseDevice"))?),
            stream_i2c: mem::transmute(get(s!("CH347StreamI2C"))?),
            stream_i2c_ret_ack: mem::transmute(get(s!("CH347StreamI2C_RetACK"))?),
            i2c_set: mem::transmute(get(s!("CH347I2C_Set"))?),
            i2c_set_stretch: mem::transmute(get(s!("CH347I2C_SetStretch"))?),
            i2c_set_delay_ms: mem::transmute(get(s!("CH347I2C_SetDelaymS"))?),
            spi_init: mem::transmute(get(s!("CH347SPI_Init"))?),
            spi_write_read: mem::transmute(get(s!("CH347SPI_WriteRead"))?),
            spi_write: mem::transmute(get(s!("CH347SPI_Write"))?),
            spi_read: mem::transmute(get(s!("CH347SPI_Read"))?),
            spi_set_frequency: mem::transmute(get(s!("CH347SPI_SetFrequency"))?),
            spi_set_data_bits: mem::transmute(get(s!("CH347SPI_SetDataBits"))?),
            spi_change_cs: mem::transmute(get(s!("CH347SPI_ChangeCS"))?),
            spi_set_chip_select: mem::transmute(get(s!("CH347SPI_SetChipSelect"))?),
            spi_get_cfg: mem::transmute(get(s!("CH347SPI_GetCfg"))?),
            spi_stream4: mem::transmute(get(s!("CH347StreamSPI4"))?),
            gpio_set: mem::transmute(get(s!("CH347GPIO_Set"))?),
            set_timeout: mem::transmute(get(s!("CH347SetTimeout"))?),
            get_device_infor: mem::transmute(get(s!("CH347GetDeviceInfor"))?),
            get_chip_type: mem::transmute(get(s!("CH347GetChipType"))?),
        })
    }
}

fn to_bool(v: i32) -> bool {
    v != 0
}

/// 读取一个【已打开】设备的描述信息，构造统一的设备条目 JSON。
///
/// GetDeviceInfor/GetChipType 都是对已打开句柄的只读查询（见 CH347Demo EnumDevice：
/// 在 Open 与 Close 之间调用），不 open/close、不触碰 SPI/I2C 数据管线，可安全用于
/// 扫描和热插拔心跳轮询。
///
/// 关键：扫描分支与"已打开设备"分支都用这个函数生成条目，保证设备打开前后名字一致 ——
/// 否则打开后名字会从 "CH347T: <desc>" 退化成 "CH347 #i"，让用户误以为选择被重置成了 #0。
///
/// # Safety
/// 调用方必须保证 `i` 对应的设备已经 open（句柄有效），且 `ffi` 指向有效的 DLL 函数表。
unsafe fn build_device_entry(ffi: &Ch347Ffi, i: u32) -> serde_json::Value {
    let mut info = DevInfor {
        index: 0,
        device_path: [0u8; 260],
        usb_class: 0,
        func_type: 0,
        device_id: [0u8; 64],
        chip_mode: 0,
        dev_handle: HANDLE::default(),
        bulk_out_max_size: 0,
        bulk_in_max_size: 0,
        usb_speed_type: 0,
        ch347_if_num: 0,
        data_up_endp: 0,
        data_dn_endp: 0,
        product_string: [0u8; 64],
        manufacturer_string: [0u8; 64],
        write_timeout: 0,
        read_timeout: 0,
        func_desc_str: [0u8; 64],
        firmware_ver: 0,
    };
    let _ = (ffi.get_device_infor)(i, &mut info);
    let desc = ptr_to_str(&info.func_desc_str);
    let chip_type = (ffi.get_chip_type)(i);
    let chip_name = match chip_type {
        0 => "CH341",
        1 => "CH347T",
        2 => "CH347F",
        3 => "CH339W",
        _ => "?",
    };
    let name = if desc.is_empty() {
        format!("{} #{}", chip_name, i)
    } else {
        format!("{}: {}", chip_name, desc)
    };
    let serial_number = ptr_to_str(&info.device_id);
    let pid = if chip_type == 2 { 0x55de } else { 0x55dd };
    serde_json::json!({
        "index": i,
        "name": name,
        "chipType": chip_type,
        "chipName": chip_name,
        "desc": desc,
        "vid": 0x1a86,
        "pid": pid,
        "serialNumber": serial_number,
        "interfaceNumber": info.ch347_if_num,
    })
}

fn is_ch347f_entry(entry: &serde_json::Value) -> bool {
    entry["chipType"].as_u64() == Some(2)
}

/// List available CH347 devices (scans indices 0-15, matches reference code EnumDevice)
///
/// 注意：扫描时会对每个索引调 open(i)+close(i) 来探测设备是否存在。
/// 但对**已经被应用打开的设备**（在 active_devices 里），绝不能再 open/close ——
/// 否则会破坏当前进行中的 SPI/I2C 会话（导致后续 spi_init 等操作失败，
/// 表现为前端轮询热插拔时把正在用的设备 close 掉）。
/// 这些已打开的设备直接当作"存在"加入结果，用简化名字 "CH347 #{i}"。
#[command]
pub fn ch347_list_devices(
    app: AppHandle,
    state: State<'_, Arc<Ch347State>>,
) -> Result<Vec<serde_json::Value>, String> {
    crate::hardware_safety::ensure_idle(&app, "CH347 USB 设备扫描")?;
    let safety = app.state::<crate::hardware_safety::HardwareSafetyState>();
    let _hardware_guard = safety.try_begin("CH347", "USB 设备扫描", "CH347F")?;
    crate::flash_owner::check_ch347()?;
    Ok(list_devices_core(&state))
}

/// 与传输无关的核心实现（Tauri 命令与 HTTP 服务共用）。
pub fn list_devices_core(state: &Ch347State) -> Vec<serde_json::Value> {
    if !state.is_available() {
        log::info!("ch347_list_devices: DLL not available");
        return vec![];
    }
    // 取出当前已打开的设备索引集合 —— 这些索引扫描时跳过 open/close
    let active_set: std::collections::HashSet<u32> = state
        .open_count
        .lock()
        .map(|m| m.keys().copied().collect())
        .unwrap_or_default();

    let mut devices = Vec::new();
    unsafe {
        if let Some(ffi) = Ch347Ffi::load() {
            log::info!(
                "ch347_list_devices: scanning indices 0-15 (skipping {} active)...",
                active_set.len()
            );
            for i in 0..16u32 {
                // 已打开的设备：不 open/close（会破坏会话），改用 CH347SPI_GetCfg 做心跳探测。
                // GetCfg 不 open/close 设备，只读已打开设备的 SPI 配置寄存器：
                //   设备还在 → 返回 true，加入结果
                //   设备被拔 → USB 通信失败返回 false，不加入结果（前端轮询会发现消失 → 自动断开）
                if active_set.contains(&i) {
                    let mut cfg = SpiCfg {
                        mode: 0,
                        clock: 0,
                        byte_order: 0,
                        write_read_interval: 0,
                        out_default_data: 0,
                        chip_select: 0,
                        cs1_polarity: 0,
                        cs2_polarity: 0,
                        is_auto_deactive_cs: 1,
                        active_delay: 0,
                        delay_deactive: 0,
                    };
                    let alive = (ffi.spi_get_cfg)(i, &mut cfg);
                    if to_bool(alive) {
                        log::info!("  device {} open & alive (GetCfg ok)", i);
                        // 设备仍在 —— 复用与扫描分支相同的命名逻辑，避免打开后名字退化成
                        // "CH347 #i"（让用户误以为选择被重置成了 #0）。GetDeviceInfor/GetChipType
                        // 是对已打开句柄的只读查询，不会破坏正在进行的 SPI/I2C 会话。
                        let entry = build_device_entry(&ffi, i);
                        if is_ch347f_entry(&entry) {
                            devices.push(entry);
                        }
                    } else {
                        log::info!("  device {} was open but GetCfg failed — treat as gone", i);
                    }
                    continue;
                }
                let handle = (ffi.open)(i);
                if !handle.is_invalid() {
                    log::info!("  device {} opened successfully", i);
                    // 用与"已打开设备"分支相同的命名逻辑构造条目，保证名字一致。
                    let entry = build_device_entry(&ffi, i);
                    log::info!("    {}", entry["name"]);
                    if is_ch347f_entry(&entry) {
                        devices.push(entry);
                    }
                    (ffi.close)(i);
                }
            }
            log::info!("ch347_list_devices: found {} device(s)", devices.len());
        } else {
            log::error!("ch347_list_devices: failed to resolve CH347DLL functions");
        }
    }
    devices
}

pub fn resolve_device_index_core(
    state: &Ch347State,
    preferred_index: u32,
    serial_number: &str,
) -> Result<u32, String> {
    crate::flash_owner::check_ch347()?;
    list_devices_core(state)
        .into_iter()
        .find(|device| {
            if serial_number.trim().is_empty() {
                device["index"].as_u64() == Some(preferred_index as u64)
            } else {
                device["serialNumber"].as_str() == Some(serial_number)
            }
        })
        .and_then(|device| device["index"].as_u64())
        .map(|index| index as u32)
        .ok_or_else(|| {
            if serial_number.trim().is_empty() {
                format!("CH347F_DEVICE_NOT_FOUND:index={preferred_index}")
            } else {
                format!("CH347F_DEVICE_NOT_FOUND:serial={serial_number}")
            }
        })
}

/// Open a CH347 device by index
#[command]
pub fn ch347_open(
    app: AppHandle,
    state: State<'_, Arc<Ch347State>>,
    index: u32,
) -> Result<(), String> {
    crate::hardware_safety::ensure_idle(&app, "CH347 打开设备")?;
    open_core(&state, index)
}

/// 与传输无关的核心实现（Tauri 命令与 HTTP 服务共用）。
///
/// 引用计数：同一 index 可被多次 open（计数 +1），仅当计数从 0→1 时才真正调 DLL open。
/// 这样前端工具与 Python 脚本可共享同一设备，不会因重复 open 破坏 DLL 状态。
pub fn open_core(state: &Ch347State, index: u32) -> Result<(), String> {
    crate::flash_owner::check_ch347()?;
    if !state.is_available() {
        return Err("CH347DLL not available".into());
    }
    // 先判断是否已打开：若已打开（计数>0）只增计数，不重复调 DLL open（否则破坏 DLL 状态）
    let already_open = {
        let mut counts = state
            .open_count
            .lock()
            .map_err(|e| format!("Lock error: {e}"))?;
        let c = counts.entry(index).or_insert(0);
        *c += 1;
        *c > 1 // 之前就有打开者
    };
    if already_open {
        log::info!("CH347 device {} already open (refcount incremented)", index);
        return Ok(());
    }
    unsafe {
        if let Some(ffi) = Ch347Ffi::load() {
            let handle = (ffi.open)(index);
            if handle.is_invalid() {
                // open 失败：回滚计数
                if let Ok(mut counts) = state.open_count.lock() {
                    if let Some(c) = counts.get_mut(&index) {
                        *c = c.saturating_sub(1);
                        if *c == 0 {
                            counts.remove(&index);
                        }
                    }
                }
                return Err(format!("Failed to open device {}", index));
            }
            (ffi.set_timeout)(index, 500, 500);
            log::info!("CH347 device {} opened", index);
            return Ok(());
        }
    }
    // DLL 加载失败：回滚计数
    if let Ok(mut counts) = state.open_count.lock() {
        if let Some(c) = counts.get_mut(&index) {
            *c = c.saturating_sub(1);
            if *c == 0 {
                counts.remove(&index);
            }
        }
    }
    Err("CH347DLL load failed".into())
}

/// Close a CH347 device
#[command]
pub fn ch347_close(
    app: AppHandle,
    state: State<'_, Arc<Ch347State>>,
    index: u32,
) -> Result<(), String> {
    crate::hardware_safety::ensure_idle(&app, "CH347 关闭设备")?;
    close_core(&state, index)
}

/// 与传输无关的核心实现（Tauri 命令与 HTTP 服务共用）。
///
/// 引用计数：仅当计数减到 0（最后一个使用者关闭）才真正调 DLL close，
/// 避免脚本 close 把前端还在用的设备关掉导致后续操作崩溃。
pub fn close_core(state: &Ch347State, index: u32) -> Result<(), String> {
    let should_close = {
        let mut counts = state
            .open_count
            .lock()
            .map_err(|e| format!("Lock error: {e}"))?;
        match counts.get_mut(&index) {
            Some(c) => {
                *c = c.saturating_sub(1);
                if *c == 0 {
                    counts.remove(&index);
                    true
                } else {
                    false
                }
            }
            None => false, // 本来就没记录为打开，安全地 no-op
        }
    };
    if should_close {
        unsafe {
            if let Some(ffi) = Ch347Ffi::load() {
                (ffi.close)(index);
            }
        }
        log::info!("CH347 device {} closed", index);
    } else {
        log::info!(
            "CH347 device {} close deferred (other users still holding it)",
            index
        );
    }
    Ok(())
}

/// I2C transfer
#[command]
#[allow(clippy::too_many_arguments)]
pub fn ch347_i2c_transfer(
    app: AppHandle,
    state: State<'_, Arc<Ch347State>>,
    index: u32,
    write_data: Vec<u8>,
    read_len: u32,
    speed_khz: Option<u32>,
    scl_stretch: Option<bool>,
    delay_ms: Option<u32>,
) -> Result<Vec<u8>, String> {
    crate::hardware_safety::ensure_idle(&app, "CH347 I2C 传输")?;
    i2c_transfer_core(
        &state,
        index,
        write_data,
        read_len,
        speed_khz,
        scl_stretch,
        delay_ms,
    )
}

/// 与传输无关的核心实现（Tauri 命令与 HTTP 服务共用）。
pub fn i2c_transfer_core(
    state: &Ch347State,
    index: u32,
    write_data: Vec<u8>,
    read_len: u32,
    speed_khz: Option<u32>,
    scl_stretch: Option<bool>,
    delay_ms: Option<u32>,
) -> Result<Vec<u8>, String> {
    if !state.is_available() {
        return Err("CH347DLL not available".into());
    }
    unsafe {
        if let Some(ffi) = Ch347Ffi::load() {
            if let Some(enable) = scl_stretch {
                (ffi.i2c_set_stretch)(index, enable as i32);
            }
            if let Some(delay) = delay_ms {
                (ffi.i2c_set_delay_ms)(index, delay);
            }
            let mode = match speed_khz.unwrap_or(100) {
                0..=20 => 0,
                21..=50 => 4,
                51..=100 => 1,
                101..=200 => 5,
                201..=400 => 2,
                401..=750 => 3,
                _ => 6,
            };
            (ffi.i2c_set)(index, mode);
            let mut read_buf = vec![0u8; read_len as usize];
            let ok = (ffi.stream_i2c)(
                index,
                write_data.len() as u32,
                write_data.as_ptr(),
                read_len,
                if read_len > 0 {
                    read_buf.as_mut_ptr()
                } else {
                    std::ptr::null_mut()
                },
            );
            if to_bool(ok) {
                Ok(read_buf)
            } else {
                Err("I2C transfer failed".into())
            }
        } else {
            Err("CH347DLL load failed".into())
        }
    }
}

/// I2C address scan
#[command]
pub fn ch347_i2c_scan(
    app: AppHandle,
    state: State<'_, Arc<Ch347State>>,
    index: u32,
    speed_khz: Option<u32>,
    scl_stretch: Option<bool>,
    delay_ms: Option<u32>,
) -> Result<Vec<u32>, String> {
    crate::hardware_safety::ensure_idle(&app, "CH347 I2C 扫描")?;
    i2c_scan_core(&state, index, speed_khz, scl_stretch, delay_ms)
}

/// 与传输无关的核心实现（Tauri 命令与 HTTP 服务共用）。
pub fn i2c_scan_core(
    state: &Ch347State,
    index: u32,
    speed_khz: Option<u32>,
    scl_stretch: Option<bool>,
    delay_ms: Option<u32>,
) -> Result<Vec<u32>, String> {
    if !state.is_available() {
        return Err("CH347DLL not available".into());
    }
    let mut found = Vec::new();
    unsafe {
        if let Some(ffi) = Ch347Ffi::load() {
            if let Some(enable) = scl_stretch {
                (ffi.i2c_set_stretch)(index, enable as i32);
            }
            if let Some(delay) = delay_ms {
                (ffi.i2c_set_delay_ms)(index, delay);
            }
            let mode = match speed_khz.unwrap_or(100) {
                0..=20 => 0,
                21..=50 => 4,
                51..=100 => 1,
                101..=200 => 5,
                201..=400 => 2,
                401..=750 => 3,
                _ => 6,
            };
            (ffi.i2c_set)(index, mode);
            for addr in 1u8..128u8 {
                let write_addr = addr << 1;
                // 扫描稳定性改进：每个地址最多重试 2 次 + 地址之间留总线恢复时间。
                // 原因：CH347 对单字节写事务的 ACK 检测在高速率/总线电容较大时余量不足，
                // 加之 EEPROM 处于内部写周期(tWR)时会 NACK，导致"上电能扫到、反复扫有时扫不到"。
                // 重试 + 延时能显著降低漏检概率。
                let mut acked = false;
                for _attempt in 0..2 {
                    let mut ack_count: u32 = 0;
                    (ffi.stream_i2c_ret_ack)(
                        index,
                        1,
                        &write_addr,
                        0,
                        std::ptr::null_mut(),
                        &mut ack_count,
                    );
                    // Device exists only if the address byte was ACKed (ack_count >= 1)
                    if ack_count >= 1 {
                        acked = true;
                        break;
                    }
                    // 重试前给总线一点恢复时间（约 1ms）
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                if acked {
                    found.push(addr as u32);
                }
                // 地址之间的总线恢复间隔（约 1ms），降低连续 START/STOP 的时序压力
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
    }
    Ok(found)
}

/// Map user-friendly MHz to CH347 clock index
fn mhz_to_clock(speed_mhz: u32) -> u8 {
    match speed_mhz {
        0..=1 => 6,   // 937.5 KHz
        2..=4 => 4,   // 3.75 MHz
        5..=10 => 2,  // 15 MHz
        11..=20 => 1, // 30 MHz
        _ => 0,       // 60 MHz
    }
}

/// Initialize SPI with full configuration
#[command]
pub fn ch347_spi_init(
    app: AppHandle,
    state: State<'_, Arc<Ch347State>>,
    index: u32,
    mode: Option<u8>,
    speed_mhz: Option<u32>,
    cs: Option<u32>,
    data_bits: Option<u8>,
    byte_order: Option<u8>,
    write_read_interval: Option<u16>,
    out_default_data: Option<u8>,
    cs1_polarity: Option<u8>,
    cs2_polarity: Option<u8>,
    is_auto_deactive_cs: Option<u16>,
    active_delay: Option<u16>,
    delay_deactive: Option<u32>,
) -> Result<(), String> {
    crate::hardware_safety::ensure_idle(&app, "CH347 SPI 初始化")?;
    spi_init_core(
        &state,
        index,
        mode,
        speed_mhz,
        cs,
        data_bits,
        byte_order,
        write_read_interval,
        out_default_data,
        cs1_polarity,
        cs2_polarity,
        is_auto_deactive_cs,
        active_delay,
        delay_deactive,
    )
}

/// 与传输无关的核心实现（Tauri 命令与 HTTP 服务共用）。
#[allow(clippy::too_many_arguments)]
pub fn spi_init_core(
    state: &Ch347State,
    index: u32,
    mode: Option<u8>,
    speed_mhz: Option<u32>,
    cs: Option<u32>,
    data_bits: Option<u8>,
    byte_order: Option<u8>,
    write_read_interval: Option<u16>,
    out_default_data: Option<u8>,
    cs1_polarity: Option<u8>,
    cs2_polarity: Option<u8>,
    is_auto_deactive_cs: Option<u16>,
    active_delay: Option<u16>,
    delay_deactive: Option<u32>,
) -> Result<(), String> {
    if !state.is_available() {
        return Err("CH347DLL not available".into());
    }
    unsafe {
        if let Some(ffi) = Ch347Ffi::load() {
            let cfg = SpiCfg {
                mode: mode.unwrap_or(0).min(3),
                clock: mhz_to_clock(speed_mhz.unwrap_or(4)),
                byte_order: byte_order.unwrap_or(1),
                write_read_interval: write_read_interval.unwrap_or(0),
                out_default_data: out_default_data.unwrap_or(0xff),
                chip_select: 0x80 | (cs.unwrap_or(0) & 0x03),
                cs1_polarity: cs1_polarity.unwrap_or(0),
                cs2_polarity: cs2_polarity.unwrap_or(0),
                is_auto_deactive_cs: is_auto_deactive_cs.unwrap_or(1),
                active_delay: active_delay.unwrap_or(0),
                delay_deactive: delay_deactive.unwrap_or(0),
            };
            // Apply data bits before init (convert from byte-width to DLL format: 0=8bit, 1=16bit)
            if let Some(bits) = data_bits {
                (ffi.spi_set_data_bits)(index, if bits == 16 { 1 } else { 0 });
            }
            let ok = (ffi.spi_init)(index, &cfg);
            if to_bool(ok) {
                Ok(())
            } else {
                Err("SPI init failed".into())
            }
        } else {
            Err("CH347DLL load failed".into())
        }
    }
}

/// Get current SPI configuration
#[command]
pub fn ch347_spi_get_config(
    app: AppHandle,
    state: State<'_, Arc<Ch347State>>,
    index: u32,
) -> Result<serde_json::Value, String> {
    crate::hardware_safety::ensure_idle(&app, "CH347 SPI 读取配置")?;
    if !state.is_available() {
        return Err("CH347DLL not available".into());
    }
    unsafe {
        if let Some(ffi) = Ch347Ffi::load() {
            let mut cfg = SpiCfg {
                mode: 0,
                clock: 0,
                byte_order: 0,
                write_read_interval: 0,
                out_default_data: 0,
                chip_select: 0,
                cs1_polarity: 0,
                cs2_polarity: 0,
                is_auto_deactive_cs: 1,
                active_delay: 0,
                delay_deactive: 0,
            };
            let ok = (ffi.spi_get_cfg)(index, &mut cfg);
            if to_bool(ok) {
                // packed 结构字段不能直接引用，先拷贝到局部变量（按值读取安全）
                let (mode, clock, byte_order, write_read_interval) =
                    (cfg.mode, cfg.clock, cfg.byte_order, cfg.write_read_interval);
                let (out_default_data, chip_select) = (cfg.out_default_data, cfg.chip_select);
                let (cs1_polarity, cs2_polarity) = (cfg.cs1_polarity, cfg.cs2_polarity);
                let (is_auto_deactive_cs, active_delay, delay_deactive) = (
                    cfg.is_auto_deactive_cs,
                    cfg.active_delay,
                    cfg.delay_deactive,
                );
                Ok(serde_json::json!({
                    "mode": mode,
                    "clock": clock,
                    "byteOrder": byte_order,
                    "writeReadInterval": write_read_interval,
                    "outDefaultData": out_default_data,
                    "chipSelect": chip_select,
                    "cs1Polarity": cs1_polarity,
                    "cs2Polarity": cs2_polarity,
                    "isAutoDeactiveCS": is_auto_deactive_cs,
                    "activeDelay": active_delay,
                    "delayDeactive": delay_deactive,
                }))
            } else {
                Err("SPI get config failed".into())
            }
        } else {
            Err("CH347DLL load failed".into())
        }
    }
}

/// Set SPI clock frequency (call before spi_init)
#[command]
pub fn ch347_spi_set_frequency(
    app: AppHandle,
    state: State<'_, Arc<Ch347State>>,
    index: u32,
    frequency_hz: u32,
) -> Result<(), String> {
    crate::hardware_safety::ensure_idle(&app, "CH347 SPI 设置频率")?;
    if !state.is_available() {
        return Err("CH347DLL not available".into());
    }
    unsafe {
        if let Some(ffi) = Ch347Ffi::load() {
            let ok = (ffi.spi_set_frequency)(index, frequency_hz);
            if to_bool(ok) {
                Ok(())
            } else {
                Err("SPI set frequency failed".into())
            }
        } else {
            Err("CH347DLL load failed".into())
        }
    }
}

/// Set SPI data bits (8-bit or 16-bit)
#[command]
pub fn ch347_spi_set_data_bits(
    app: AppHandle,
    state: State<'_, Arc<Ch347State>>,
    index: u32,
    data_bits: u8,
) -> Result<(), String> {
    crate::hardware_safety::ensure_idle(&app, "CH347 SPI 设置数据位")?;
    if !state.is_available() {
        return Err("CH347DLL not available".into());
    }
    unsafe {
        if let Some(ffi) = Ch347Ffi::load() {
            let ok = (ffi.spi_set_data_bits)(index, if data_bits == 16 { 1 } else { 0 });
            if to_bool(ok) {
                Ok(())
            } else {
                Err("SPI set data bits failed".into())
            }
        } else {
            Err("CH347DLL load failed".into())
        }
    }
}

/// Manual chip select control
#[command]
pub fn ch347_spi_change_cs(
    app: AppHandle,
    state: State<'_, Arc<Ch347State>>,
    index: u32,
    status: u8,
) -> Result<(), String> {
    crate::hardware_safety::ensure_idle(&app, "CH347 SPI 切换片选")?;
    if !state.is_available() {
        return Err("CH347DLL not available".into());
    }
    unsafe {
        if let Some(ffi) = Ch347Ffi::load() {
            let ok = (ffi.spi_change_cs)(index, status);
            if to_bool(ok) {
                Ok(())
            } else {
                Err("SPI change CS failed".into())
            }
        } else {
            Err("CH347DLL load failed".into())
        }
    }
}

/// Configure chip select behavior
#[command]
#[allow(clippy::too_many_arguments)]
pub fn ch347_spi_set_chip_select(
    app: AppHandle,
    state: State<'_, Arc<Ch347State>>,
    index: u32,
    enable_select: u16,
    chip_select: u16,
    is_auto_deactive_cs: u32,
    active_delay: u32,
    delay_deactive: u32,
) -> Result<(), String> {
    crate::hardware_safety::ensure_idle(&app, "CH347 SPI 配置片选")?;
    if !state.is_available() {
        return Err("CH347DLL not available".into());
    }
    unsafe {
        if let Some(ffi) = Ch347Ffi::load() {
            let ok = (ffi.spi_set_chip_select)(
                index,
                enable_select,
                chip_select,
                is_auto_deactive_cs,
                active_delay,
                delay_deactive,
            );
            if to_bool(ok) {
                Ok(())
            } else {
                Err("SPI set chip select failed".into())
            }
        } else {
            Err("CH347DLL load failed".into())
        }
    }
}

/// SPI write-only transfer (MOSI only, no MISO read).
/// Matches CH347Demo CH347SpiStream(Cmd=0xC4): directly calls CH347SPI_Write
/// without re-initializing SPI or changing data bits. SPI must be configured
/// first via ch347_spi_init (called once, like CH347InitSpi).
#[command]
#[allow(clippy::too_many_arguments)]
pub fn ch347_spi_write(
    app: AppHandle,
    state: State<'_, Arc<Ch347State>>,
    index: u32,
    tx_data: Vec<u8>,
    cs: Option<u32>,
    mode: Option<u8>,
    speed_mhz: Option<u32>,
    data_bits: Option<u8>,
) -> Result<(), String> {
    crate::hardware_safety::ensure_idle(&app, "CH347 SPI 写入")?;
    let _ = (mode, speed_mhz, data_bits);
    spi_write_core(&state, index, tx_data, cs)
}

/// 与传输无关的核心实现（Tauri 命令与 HTTP 服务共用）。
pub fn spi_write_core(
    state: &Ch347State,
    index: u32,
    tx_data: Vec<u8>,
    cs: Option<u32>,
) -> Result<(), String> {
    if !state.is_available() {
        return Err("CH347DLL not available".into());
    }
    if tx_data.is_empty() {
        return Err("txData is empty".into());
    }
    unsafe {
        if let Some(ffi) = Ch347Ffi::load() {
            let chip_select = cs.unwrap_or(0x80);
            let len = tx_data.len() as u32;
            let mut buf = tx_data;
            // CH347Demo: CH347SPI_Write(iIndex, 0x80, iLength, 512, ioBuffer)
            let ok = (ffi.spi_write)(index, chip_select, len, 512, buf.as_mut_ptr());
            if to_bool(ok) {
                Ok(())
            } else {
                Err("SPI write failed".into())
            }
        } else {
            Err("CH347DLL load failed".into())
        }
    }
}

/// 与传输无关的大缓冲分块实现（HTTP RGB565 帧与 Tauri 命令共用）。
pub fn spi_write_chunks_core(
    state: &Ch347State,
    index: u32,
    tx_data: &[u8],
    chunk_size: usize,
    cs: Option<u32>,
) -> Result<(), String> {
    if !state.is_available() {
        return Err("CH347DLL not available".into());
    }
    if tx_data.is_empty() {
        return Ok(());
    }
    if chunk_size == 0 {
        return Err("CH347_SPI_CHUNK_LENGTH_INVALID:0".into());
    }
    unsafe {
        if let Some(ffi) = Ch347Ffi::load() {
            let chip_select = cs.unwrap_or(0x80);
            for chunk in tx_data.chunks(chunk_size) {
                let mut buffer = chunk.to_vec();
                let ok = (ffi.spi_write)(
                    index,
                    chip_select,
                    buffer.len() as u32,
                    512,
                    buffer.as_mut_ptr(),
                );
                if !to_bool(ok) {
                    return Err("SPI write buffer failed".into());
                }
            }
            Ok(())
        } else {
            Err("CH347DLL load failed".into())
        }
    }
}

/// SPI fill: 在 Rust 端循环写入指定颜色字节，填充 pixel_count 个像素。
///
/// 用于 RGB 屏（如 ST7796U2 320x480）整屏填色点亮。若用前端按块多次 IPC 调用
/// ch347_spi_write，307200 字节会被拆成数十次 IPC + JSON 序列化，刷新极慢且易花屏。
/// 本命令在后端一次性循环调用 CH347SPI_Write（32KB/块，DLL 内部再按 512B 分 USB 包），
/// 整屏填色 <1s 完成。
///
/// 前置条件：SPI 已通过 ch347_spi_init 配置；已发 RAMWR(0x2C) 命令进入显存写入态；
/// DC 已拉高（数据模式）。color 为单像素的字节序列（RGB565 = 2 字节，高字节在前）。
#[command]
pub async fn ch347_spi_fill(
    app: AppHandle,
    state: State<'_, Arc<Ch347State>>,
    index: u32,
    color: Vec<u8>,
    pixel_count: u32,
    cs: Option<u32>,
) -> Result<(), String> {
    crate::hardware_safety::ensure_idle(&app, "CH347 SPI 填充")?;
    if !state.is_available() {
        return Err("CH347DLL not available".into());
    }
    if color.is_empty() {
        return Err("color is empty".into());
    }
    if pixel_count == 0 {
        return Ok(());
    }
    unsafe {
        if let Some(ffi) = Ch347Ffi::load() {
            let chip_select = cs.unwrap_or(0x80);
            let bpp = color.len();
            // 每次写给 DLL 的块大小（512B = 1 个 USB 包）。实测 CH347 写超时 500ms 是
            // 按"单次 CH347SPI_Write 调用"计的，USB 吞吐低时大块会超时报
            // "SPI fill write failed"。512B 即使在 ~1KB/s 也 <500ms，最稳；DLL 不再分包子。
            const CHUNK_BYTES: usize = 512;
            let pixels_per_chunk = (CHUNK_BYTES / bpp) as u32;
            let chunk_len = (pixels_per_chunk as usize) * bpp;
            // 预生成一个满块的缓冲区，后续按需截取前 len 字节写出
            let mut chunk = Vec::with_capacity(chunk_len);
            for _ in 0..pixels_per_chunk {
                chunk.extend_from_slice(&color);
            }
            let mut written: u32 = 0;
            while written < pixel_count {
                let n = std::cmp::min(pixels_per_chunk, pixel_count - written);
                let len = (n as usize) * bpp;
                let ok = (ffi.spi_write)(
                    index,
                    chip_select,
                    len as u32,
                    512,
                    chunk[..len].as_mut_ptr(),
                );
                if !to_bool(ok) {
                    return Err("SPI fill write failed".into());
                }
                written += n;
            }
            Ok(())
        } else {
            Err("CH347DLL load failed".into())
        }
    }
}

/// SPI 写任意字节缓冲（分块）。用于 RGB 屏（如 ST7796U2）推一帧/一区域的像素数据。
/// 与 ch347_spi_fill 的区别：fill 用单色循环填，本函数直接把传入的 data 切块写出。
/// 1KB/块（CH347 写超时 500ms，低 USB 吞吐时大块会超时；1KB 即使 ~3KB/s 也 ≈333ms 安全）。
/// 1 次 IPC，Rust 端循环，避免前端按块多次 IPC。
#[command]
pub async fn ch347_spi_write_buffer(
    app: AppHandle,
    state: State<'_, Arc<Ch347State>>,
    index: u32,
    data: Vec<u8>,
    cs: Option<u32>,
) -> Result<(), String> {
    crate::hardware_safety::ensure_idle(&app, "CH347 SPI 缓冲写入")?;
    spi_write_chunks_core(&state, index, &data, 1024, cs)
}

/// SPI read-only transfer.
/// Matches reference CH347SpiStream(Cmd=0xC3): sends cmd bytes,
/// then reads requested length from MISO.
/// SPI must be configured first via ch347_spi_init (called once, like CH347InitSpi).
#[command]
#[allow(clippy::too_many_arguments)]
pub fn ch347_spi_read(
    app: AppHandle,
    state: State<'_, Arc<Ch347State>>,
    index: u32,
    read_len: u32,
    cs: Option<u32>,
    mode: Option<u8>,
    speed_mhz: Option<u32>,
    data_bits: Option<u8>,
) -> Result<Vec<u8>, String> {
    crate::hardware_safety::ensure_idle(&app, "CH347 SPI 读取")?;
    let _ = (mode, speed_mhz, data_bits);
    spi_read_core(&state, index, read_len, cs)
}

/// 与传输无关的核心实现（Tauri 命令与 HTTP 服务共用）。
#[allow(dead_code)]
pub fn spi_read_core(
    state: &Ch347State,
    index: u32,
    read_len: u32,
    cs: Option<u32>,
) -> Result<Vec<u8>, String> {
    if !state.is_available() {
        return Err("CH347DLL not available".into());
    }
    if read_len == 0 {
        return Err("readLen must be > 0".into());
    }
    unsafe {
        if let Some(ffi) = Ch347Ffi::load() {
            let chip_select = cs.unwrap_or(0x80);
            let mut buf = vec![0u8; read_len as usize];
            let mut actual_len: u32 = read_len;
            let ok = (ffi.spi_read)(index, chip_select, 0, &mut actual_len, buf.as_mut_ptr());
            if to_bool(ok) {
                buf.truncate(actual_len as usize);
                Ok(buf)
            } else {
                Err("SPI read failed".into())
            }
        } else {
            Err("CH347DLL load failed".into())
        }
    }
}

/// SPI full-duplex transfer (WriteRead — MOSI write, MISO read in same buffer).
/// Matches CH347Demo CH347SpiStream(Cmd=0xC2): directly calls CH347SPI_WriteRead.
/// SPI must be configured first via ch347_spi_init (called once, like CH347InitSpi).
#[command]
#[allow(clippy::too_many_arguments)]
pub fn ch347_spi_transfer(
    app: AppHandle,
    state: State<'_, Arc<Ch347State>>,
    index: u32,
    tx_data: Vec<u8>,
    cs: Option<u32>,
    mode: Option<u8>,
    speed_mhz: Option<u32>,
    data_bits: Option<u8>,
) -> Result<Vec<u8>, String> {
    crate::hardware_safety::ensure_idle(&app, "CH347 SPI 全双工传输")?;
    let _ = (mode, speed_mhz, data_bits);
    spi_transfer_core(&state, index, tx_data, cs)
}

/// 与传输无关的核心实现（Tauri 命令与 HTTP 服务共用）。
#[allow(dead_code)]
pub fn spi_transfer_core(
    state: &Ch347State,
    index: u32,
    tx_data: Vec<u8>,
    cs: Option<u32>,
) -> Result<Vec<u8>, String> {
    if !state.is_available() {
        return Err("CH347DLL not available".into());
    }
    unsafe {
        if let Some(ffi) = Ch347Ffi::load() {
            let chip_select = cs.unwrap_or(0x80);
            let len = tx_data.len() as u32;
            let mut buf = tx_data;
            let ok = (ffi.spi_write_read)(index, chip_select, len, buf.as_mut_ptr());
            if to_bool(ok) {
                Ok(buf)
            } else {
                Err("SPI transfer failed".into())
            }
        } else {
            Err("CH347DLL load failed".into())
        }
    }
}

/// SPI 4-wire stream transfer (high-efficiency full-duplex).
/// SPI must be configured first via ch347_spi_init (called once, like CH347InitSpi).
#[command]
#[allow(clippy::too_many_arguments)]
pub fn ch347_spi_stream4(
    app: AppHandle,
    state: State<'_, Arc<Ch347State>>,
    index: u32,
    tx_data: Vec<u8>,
    cs: Option<u32>,
    mode: Option<u8>,
    speed_mhz: Option<u32>,
    data_bits: Option<u8>,
) -> Result<Vec<u8>, String> {
    crate::hardware_safety::ensure_idle(&app, "CH347 SPI 流传输")?;
    let _ = (mode, speed_mhz, data_bits);
    if !state.is_available() {
        return Err("CH347DLL not available".into());
    }
    unsafe {
        if let Some(ffi) = Ch347Ffi::load() {
            let chip_select = cs.unwrap_or(0x80);
            let len = tx_data.len() as u32;
            let mut buf = tx_data;
            let ok = (ffi.spi_stream4)(index, chip_select, len, buf.as_mut_ptr());
            if to_bool(ok) {
                Ok(buf)
            } else {
                Err("SPI stream4 failed".into())
            }
        } else {
            Err("CH347DLL load failed".into())
        }
    }
}

/// GPIO set — control GPIO1-GPIO7 pins. GPIO0 is reserved by hardware.
#[command]
pub fn ch347_gpio_set(
    app: AppHandle,
    state: State<'_, Arc<Ch347State>>,
    index: u32,
    enable: u8,
    dir_out: u8,
    data_out: u8,
) -> Result<(), String> {
    crate::hardware_safety::ensure_idle(&app, "CH347 GPIO 设置")?;
    gpio_set_core(&state, index, enable, dir_out, data_out)
}

/// 与传输无关的核心实现（Tauri 命令与 HTTP 服务共用）。
fn without_gpio0(enable: u8, dir_out: u8, data_out: u8) -> (u8, u8, u8) {
    (enable & !0x01, dir_out & !0x01, data_out & !0x01)
}

pub fn gpio_set_core(
    state: &Ch347State,
    index: u32,
    enable: u8,
    dir_out: u8,
    data_out: u8,
) -> Result<(), String> {
    if !state.is_available() {
        return Err("CH347DLL not available".into());
    }
    let (enable, dir_out, data_out) = without_gpio0(enable, dir_out, data_out);
    unsafe {
        if let Some(ffi) = Ch347Ffi::load() {
            let ok = (ffi.gpio_set)(index, enable, dir_out, data_out);
            if to_bool(ok) {
                Ok(())
            } else {
                Err("GPIO set failed".into())
            }
        } else {
            Err("CH347DLL load failed".into())
        }
    }
}
