//! Cross-platform CH347 backend.
//!
//! Windows uses the vendor DLL. On Linux/macOS, device discovery, GPIO, I2C and
//! the bounded SPI write path use the adapter's native bulk protocol through
//! libusb. SPI read/duplex and uncommon configuration remain explicit
//! capability errors instead of pretending that an operation succeeded.

use rusb::{Context, Device, DeviceHandle, UsbContext};
use std::sync::Arc;
use std::time::Duration;
use tauri::{command, AppHandle, Manager, State};

use super::Ch347State;

const VID_WCH: u16 = 0x1a86;
const CH347F_PID: u16 = 0x55de;
const CH347F_DEVICE_VERSION: rusb::Version = rusb::Version(1, 2, 0);
const GPIO_COMMAND: u8 = 0xcc;
const GPIO_COUNT: usize = 8;
const ENDPOINT_OUT: u8 = 0x06;
const ENDPOINT_IN: u8 = 0x86;
const USB_TIMEOUT: Duration = Duration::from_millis(500);
const CH347_PACKET_LENGTH: usize = 512;
const I2C_STREAM_COMMAND: u8 = 0xaa;
const I2C_STREAM_START: u8 = 0x74;
const I2C_STREAM_STOP: u8 = 0x75;
const I2C_STREAM_END: u8 = 0x00;
const I2C_STREAM_OUT: u8 = 0x80;
const I2C_STREAM_IN: u8 = 0xc0;
const SPI_CONFIG_COMMAND: u8 = 0xc0;
const SPI_WRITE_COMMAND: u8 = 0xc4;
const MAX_NATIVE_I2C_WRITE: usize = 63;
const MAX_NATIVE_I2C_READ: usize = 32;
const MAX_NATIVE_SPI_WRITE: usize = 480;

fn unavailable<T>() -> Result<T, String> {
    Err(
        "CH347_UNSUPPORTED_OPERATION:I2C/SPI is not implemented by the native USB backend"
            .to_owned(),
    )
}

fn ensure_open(state: &Ch347State, index: u32) -> Result<(), String> {
    if state
        .open_count
        .lock()
        .map_err(|_| "CH347_STATE_LOCK".to_owned())?
        .contains_key(&index)
    {
        Ok(())
    } else {
        Err(format!("CH347_NOT_OPEN:index={index}"))
    }
}

fn bulk_write(
    handle: &DeviceHandle<Context>,
    packet: &[u8],
    operation: &str,
) -> Result<(), String> {
    let written = handle
        .write_bulk(ENDPOINT_OUT, packet, USB_TIMEOUT)
        .map_err(|error| usb_error(&format!("{operation} write"), error))?;
    if written == packet.len() {
        Ok(())
    } else {
        Err(format!(
            "CH347_SHORT_WRITE:{operation}:{written}/{}",
            packet.len()
        ))
    }
}

fn bulk_read(handle: &DeviceHandle<Context>, operation: &str) -> Result<Vec<u8>, String> {
    let mut reply = vec![0u8; CH347_PACKET_LENGTH];
    let read = handle
        .read_bulk(ENDPOINT_IN, &mut reply, USB_TIMEOUT)
        .map_err(|error| usb_error(&format!("{operation} read"), error))?;
    reply.truncate(read);
    Ok(reply)
}

fn command_exchange(
    handle: &DeviceHandle<Context>,
    packet: &[u8],
    operation: &str,
) -> Result<Vec<u8>, String> {
    bulk_write(handle, packet, operation)?;
    bulk_read(handle, operation)
}

fn validate_command_ack(reply: &[u8], command: u8, operation: &str) -> Result<(), String> {
    if reply.len() == 4 && reply[0] == command && reply[1] == 1 && reply[2] == 0 && reply[3] == 0 {
        Ok(())
    } else {
        Err(format!(
            "CH347_REPLY_INVALID:{operation}:{}",
            hex::encode(reply)
        ))
    }
}

fn with_claimed<T>(
    state: &Ch347State,
    index: u32,
    operation: &str,
    action: impl FnOnce(&DeviceHandle<Context>) -> Result<T, String>,
) -> Result<T, String> {
    ensure_open(state, index)?;
    let _guard = state
        .io_lock
        .lock()
        .map_err(|_| "CH347_IO_LOCK".to_owned())?;
    let (handle, interface) = open_claimed(index)?;
    let result = action(&handle);
    let release = handle
        .release_interface(interface)
        .map_err(|error| usb_error(&format!("{operation} release"), error));
    match (result, release) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

fn usb_error(context: &str, error: rusb::Error) -> String {
    if error == rusb::Error::Access {
        return format!(
            "CH347_USB_PERMISSION:{context}:access denied; install the LYNX udev rule and reconnect the controller"
        );
    }
    format!("CH347_USB:{context}:{error}")
}

fn devices() -> Result<Vec<Device<Context>>, String> {
    let context = Context::new().map_err(|error| usb_error("initialize", error))?;
    let list = context
        .devices()
        .map_err(|error| usb_error("enumerate", error))?;
    let mut matches = list
        .iter()
        .filter_map(|device| {
            let descriptor = device.device_descriptor().ok()?;
            is_ch347f(
                descriptor.vendor_id(),
                descriptor.product_id(),
                descriptor.device_version(),
            )
            .then_some(device)
        })
        .collect::<Vec<_>>();
    matches.sort_by_key(|device| (device.bus_number(), device.address()));
    Ok(matches)
}

fn is_ch347f(vid: u16, pid: u16, device_version: rusb::Version) -> bool {
    vid == VID_WCH && pid == CH347F_PID && device_version == CH347F_DEVICE_VERSION
}

fn device_at(index: u32) -> Result<Device<Context>, String> {
    devices()?
        .into_iter()
        .nth(index as usize)
        .ok_or_else(|| format!("CH347_DEVICE_NOT_FOUND:index={index}"))
}

fn interface_number(_device: &Device<Context>) -> Result<u8, String> {
    Ok(4)
}

fn open_claimed(index: u32) -> Result<(DeviceHandle<Context>, u8), String> {
    let device = device_at(index)?;
    let interface = interface_number(&device)?;
    let handle = device.open().map_err(|error| usb_error("open", error))?;
    let _ = handle.set_auto_detach_kernel_driver(true);
    handle
        .claim_interface(interface)
        .map_err(|error| usb_error(&format!("claim interface {interface}"), error))?;
    Ok((handle, interface))
}

fn gpio_packet(enable: u8, dir_out: u8, data_out: u8) -> [u8; 11] {
    let mut packet = [0u8; 11];
    packet[0] = GPIO_COMMAND;
    packet[1] = GPIO_COUNT as u8;
    for gpio in 0..GPIO_COUNT {
        let mask = 1u8 << gpio;
        if enable & mask != 0 {
            packet[3 + gpio] |= 0xc0;
        }
        if dir_out & mask != 0 {
            packet[3 + gpio] |= 0x30;
        }
        if data_out & mask != 0 {
            packet[3 + gpio] |= 0x08;
        }
    }
    packet
}

fn validate_gpio_reply(reply: &[u8], enable: u8, dir_out: u8, data_out: u8) -> Result<(), String> {
    if reply.len() != 11
        || reply[0] != GPIO_COMMAND
        || reply[1] != GPIO_COUNT as u8
        || reply[2] != 0
    {
        return Err(format!("CH347_GPIO_REPLY_INVALID:{}", hex::encode(reply)));
    }
    for gpio in 0..GPIO_COUNT {
        let mask = 1u8 << gpio;
        if enable & dir_out & mask == 0 {
            continue;
        }
        let actual_high = reply[3 + gpio] & 0x40 != 0;
        let expected_high = data_out & mask != 0;
        if actual_high != expected_high {
            return Err(format!(
                "CH347_GPIO_VERIFY_FAILED:gpio={gpio}:expected={}:reply={:02x}",
                u8::from(expected_high),
                reply[3 + gpio]
            ));
        }
    }
    Ok(())
}

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

pub fn list_devices_core(state: &Ch347State) -> Vec<serde_json::Value> {
    let _guard = match state.io_lock.lock() {
        Ok(guard) => guard,
        Err(_) => return Vec::new(),
    };
    devices()
        .unwrap_or_default()
        .into_iter()
        .enumerate()
        .filter_map(|(index, device)| {
            let descriptor = device.device_descriptor().ok()?;
            let serial_number = device
                .open()
                .ok()
                .and_then(|handle| handle.read_serial_number_string_ascii(&descriptor).ok())
                .unwrap_or_default();
            let interface_number = interface_number(&device).ok()?;
            Some(serde_json::json!({
                "index": index,
                "name": format!("Lynx Power #{} · {:03}:{:03}", index, device.bus_number(), device.address()),
                "chipType": 2,
                "chipName": "CH347F",
                "desc": format!("USB {:04x}:{:04x}", descriptor.vendor_id(), descriptor.product_id()),
                "vid": descriptor.vendor_id(),
                "pid": descriptor.product_id(),
                "serialNumber": serial_number,
                "interfaceNumber": interface_number,
            }))
        })
        .collect()
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

#[command]
pub fn ch347_open(
    app: AppHandle,
    state: State<'_, Arc<Ch347State>>,
    index: u32,
) -> Result<(), String> {
    crate::hardware_safety::ensure_idle(&app, "CH347 打开设备")?;
    open_core(&state, index)
}

pub fn open_core(state: &Ch347State, index: u32) -> Result<(), String> {
    crate::flash_owner::check_ch347()?;
    let mut counts = state
        .open_count
        .lock()
        .map_err(|_| "CH347_STATE_LOCK".to_owned())?;
    if let Some(count) = counts.get_mut(&index) {
        *count += 1;
        return Ok(());
    }
    let _guard = state
        .io_lock
        .lock()
        .map_err(|_| "CH347_IO_LOCK".to_owned())?;
    let (handle, interface) = open_claimed(index)?;
    handle
        .release_interface(interface)
        .map_err(|error| usb_error("release", error))?;
    counts.insert(index, 1);
    Ok(())
}

#[command]
pub fn ch347_close(
    app: AppHandle,
    state: State<'_, Arc<Ch347State>>,
    index: u32,
) -> Result<(), String> {
    crate::hardware_safety::ensure_idle(&app, "CH347 关闭设备")?;
    close_core(&state, index)
}

pub fn close_core(state: &Ch347State, index: u32) -> Result<(), String> {
    let mut counts = state
        .open_count
        .lock()
        .map_err(|_| "CH347_STATE_LOCK".to_owned())?;
    match counts.get_mut(&index) {
        Some(count) if *count > 1 => *count -= 1,
        Some(_) => {
            counts.remove(&index);
        }
        None => return Err(format!("CH347_NOT_OPEN:index={index}")),
    }
    Ok(())
}

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

pub fn i2c_transfer_core(
    state: &Ch347State,
    index: u32,
    write_data: Vec<u8>,
    read_len: u32,
    speed_khz: Option<u32>,
    scl_stretch: Option<bool>,
    delay_ms: Option<u32>,
) -> Result<Vec<u8>, String> {
    validate_i2c_options(&write_data, read_len, scl_stretch, delay_ms)?;
    let mode = i2c_speed_mode(speed_khz.unwrap_or(100));
    with_claimed(state, index, "I2C transfer", |handle| {
        configure_i2c(handle, mode)?;
        i2c_transaction(handle, &write_data, read_len as usize)
    })
}

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

pub fn i2c_scan_core(
    state: &Ch347State,
    index: u32,
    speed_khz: Option<u32>,
    scl_stretch: Option<bool>,
    delay_ms: Option<u32>,
) -> Result<Vec<u32>, String> {
    validate_i2c_options(&[0x02], 0, scl_stretch, delay_ms)?;
    let mode = i2c_speed_mode(speed_khz.unwrap_or(100));
    with_claimed(state, index, "I2C scan", |handle| {
        configure_i2c(handle, mode)?;
        let mut found = Vec::new();
        for address in 1u8..=0x7f {
            if i2c_transaction(handle, &[address << 1], 0).is_ok() {
                found.push(u32::from(address));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(found)
    })
}

fn i2c_speed_mode(speed_khz: u32) -> u8 {
    match speed_khz {
        0..=20 => 0,
        21..=50 => 4,
        51..=100 => 1,
        101..=200 => 5,
        201..=400 => 2,
        401..=750 => 3,
        _ => 6,
    }
}

fn validate_i2c_options(
    write_data: &[u8],
    read_len: u32,
    scl_stretch: Option<bool>,
    delay_ms: Option<u32>,
) -> Result<(), String> {
    if write_data.is_empty() || write_data.len() > MAX_NATIVE_I2C_WRITE {
        return Err(format!(
            "CH347_I2C_WRITE_LENGTH_INVALID:{}/{}",
            write_data.len(),
            MAX_NATIVE_I2C_WRITE
        ));
    }
    if read_len as usize > MAX_NATIVE_I2C_READ {
        return Err(format!(
            "CH347_I2C_READ_LENGTH_INVALID:{read_len}/{}",
            MAX_NATIVE_I2C_READ
        ));
    }
    if scl_stretch == Some(true) {
        return Err("CH347_UNSUPPORTED_OPTION:I2C clock stretching on native backend".to_owned());
    }
    if delay_ms.unwrap_or(0) > 0 {
        return Err("CH347_UNSUPPORTED_OPTION:I2C adapter delay on native backend".to_owned());
    }
    Ok(())
}

fn configure_i2c(handle: &DeviceHandle<Context>, mode: u8) -> Result<(), String> {
    // Preserve the official CH347 open-drain/no-stretch stream defaults, then
    // select the requested SCL divider. Neither command writes an I2C target.
    let reply = command_exchange(
        handle,
        &[
            0xe2, 0x08, 0x00, 0x00, 0x00, 0x81, 0x81, 0x00, 0x00, 0x00, 0x00,
        ],
        "I2C configure",
    )?;
    validate_command_ack(&reply, 0xe2, "I2C configure")?;
    bulk_write(
        handle,
        &[I2C_STREAM_COMMAND, 0x60 | (mode & 0x07), I2C_STREAM_END],
        "I2C speed",
    )
}

fn i2c_packet(write_data: &[u8], read_len: usize) -> Result<Vec<u8>, String> {
    validate_i2c_options(write_data, read_len as u32, Some(false), Some(0))?;
    let mut packet = Vec::with_capacity(write_data.len() + read_len + 10);
    packet.extend_from_slice(&[
        I2C_STREAM_COMMAND,
        I2C_STREAM_START,
        I2C_STREAM_OUT | write_data.len() as u8,
    ]);
    packet.extend_from_slice(write_data);
    if read_len > 0 {
        packet.extend_from_slice(&[I2C_STREAM_START, I2C_STREAM_OUT | 1, write_data[0] | 1]);
        if read_len > 1 {
            packet.push(I2C_STREAM_IN | (read_len as u8 - 1));
        }
        packet.push(I2C_STREAM_IN);
    }
    packet.extend_from_slice(&[I2C_STREAM_STOP, I2C_STREAM_END]);
    Ok(packet)
}

fn i2c_transaction(
    handle: &DeviceHandle<Context>,
    write_data: &[u8],
    read_len: usize,
) -> Result<Vec<u8>, String> {
    let packet = i2c_packet(write_data, read_len)?;
    let reply = command_exchange(handle, &packet, "I2C transaction")?;
    let ack_count = write_data.len() + usize::from(read_len > 0);
    let expected = ack_count + read_len;
    if reply.len() != expected || reply[..ack_count].contains(&0) {
        return Err(format!(
            "CH347_I2C_NACK_OR_REPLY_INVALID:{}",
            hex::encode(reply)
        ));
    }
    Ok(reply[ack_count..].to_vec())
}

#[command]
#[allow(clippy::too_many_arguments)]
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
    let packet = spi_config_packet(
        mode.unwrap_or(0),
        speed_mhz.unwrap_or(4),
        cs,
        data_bits.unwrap_or(8),
        byte_order.unwrap_or(1),
        write_read_interval.unwrap_or(0),
        out_default_data.unwrap_or(0xff),
        cs1_polarity.unwrap_or(0),
        cs2_polarity.unwrap_or(0),
        is_auto_deactive_cs.unwrap_or(1),
        active_delay.unwrap_or(0),
        delay_deactive.unwrap_or(0),
    )?;
    with_claimed(state, index, "SPI configure", |handle| {
        let reply = command_exchange(handle, &packet, "SPI configure")?;
        validate_command_ack(&reply, SPI_CONFIG_COMMAND, "SPI configure")
    })
}

#[allow(clippy::too_many_arguments)]
fn spi_config_packet(
    mode: u8,
    speed_mhz: u32,
    cs: Option<u32>,
    data_bits: u8,
    byte_order: u8,
    write_read_interval: u16,
    out_default_data: u8,
    cs1_polarity: u8,
    cs2_polarity: u8,
    is_auto_deactive_cs: u16,
    active_delay: u16,
    delay_deactive: u32,
) -> Result<Vec<u8>, String> {
    if mode > 3 || data_bits != 8 || byte_order > 1 {
        return Err("CH347_SPI_CONFIG_UNSUPPORTED:mode/dataBits/byteOrder".to_owned());
    }
    if !matches!(cs, None | Some(0)) {
        return Err(
            "CH347_SPI_CONFIG_UNSUPPORTED:native backend supports grounded CS only".to_owned(),
        );
    }
    if write_read_interval != 0
        || cs1_polarity != 0
        || cs2_polarity != 0
        || is_auto_deactive_cs != 1
        || active_delay != 0
        || delay_deactive != 0
    {
        return Err("CH347_SPI_CONFIG_UNSUPPORTED:timing or CS options".to_owned());
    }
    // StreamHwCfgS payload captured from WCH's official CH347 Linux library.
    // The two mode words below are the only mode-dependent fields.
    let mut payload = [
        0x00, 0x00, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x30, 0x00, 0x00,
        0x00, 0x07, 0x00, 0x00, 0x00, 0xff, 0x00, 0x00, 0x00, 0x06, 0x00,
    ];
    payload[6] = if mode >= 2 { 2 } else { 0 };
    payload[8] = if mode == 1 || mode == 3 { 1 } else { 0 };
    payload[14] = if byte_order == 1 { 0 } else { 0x80 };
    payload[20] = out_default_data;
    payload[24] = match speed_mhz {
        0..=1 => 6,
        2..=4 => 4,
        5..=10 => 2,
        11..=20 => 1,
        _ => 0,
    };
    let mut packet = vec![SPI_CONFIG_COMMAND, payload.len() as u8, 0x00];
    packet.extend_from_slice(&payload);
    Ok(packet)
}

#[command]
pub fn ch347_spi_get_config(
    app: AppHandle,
    _state: State<'_, Arc<Ch347State>>,
    _index: u32,
) -> Result<serde_json::Value, String> {
    crate::hardware_safety::ensure_idle(&app, "CH347 SPI 读取配置")?;
    unavailable()
}

#[command]
pub fn ch347_spi_set_frequency(
    app: AppHandle,
    _state: State<'_, Arc<Ch347State>>,
    _index: u32,
    _frequency_hz: u32,
) -> Result<(), String> {
    crate::hardware_safety::ensure_idle(&app, "CH347 SPI 设置频率")?;
    unavailable()
}

#[command]
pub fn ch347_spi_set_data_bits(
    app: AppHandle,
    _state: State<'_, Arc<Ch347State>>,
    _index: u32,
    _data_bits: u8,
) -> Result<(), String> {
    crate::hardware_safety::ensure_idle(&app, "CH347 SPI 设置数据位")?;
    unavailable()
}

#[command]
pub fn ch347_spi_change_cs(
    app: AppHandle,
    _state: State<'_, Arc<Ch347State>>,
    _index: u32,
    _status: u8,
) -> Result<(), String> {
    crate::hardware_safety::ensure_idle(&app, "CH347 SPI 切换片选")?;
    unavailable()
}

#[command]
#[allow(clippy::too_many_arguments)]
pub fn ch347_spi_set_chip_select(
    app: AppHandle,
    _state: State<'_, Arc<Ch347State>>,
    _index: u32,
    _enable_select: u16,
    _chip_select: u16,
    _is_auto_deactive_cs: u32,
    _active_delay: u32,
    _delay_deactive: u32,
) -> Result<(), String> {
    crate::hardware_safety::ensure_idle(&app, "CH347 SPI 配置片选")?;
    unavailable()
}

#[command]
#[allow(clippy::too_many_arguments)]
pub fn ch347_spi_write(
    app: AppHandle,
    state: State<'_, Arc<Ch347State>>,
    index: u32,
    tx_data: Vec<u8>,
    cs: Option<u32>,
    _mode: Option<u8>,
    _speed_mhz: Option<u32>,
    _data_bits: Option<u8>,
) -> Result<(), String> {
    crate::hardware_safety::ensure_idle(&app, "CH347 SPI 写入")?;
    spi_write_core(&state, index, tx_data, cs)
}

pub fn spi_write_core(
    state: &Ch347State,
    index: u32,
    tx_data: Vec<u8>,
    cs: Option<u32>,
) -> Result<(), String> {
    let packet = spi_write_packet(&tx_data, cs)?;
    with_claimed(state, index, "SPI write", |handle| {
        let reply = command_exchange(handle, &packet, "SPI write")?;
        validate_command_ack(&reply, SPI_WRITE_COMMAND, "SPI write")
    })
}

/// Write a larger SPI buffer while keeping the native USB interface claimed.
///
/// The CH347 native stream packet is bounded, so callers choose a packet-sized
/// chunk. Keeping one claim for the whole frame avoids hundreds of open/claim
/// cycles when refreshing an RGB display.
pub fn spi_write_chunks_core(
    state: &Ch347State,
    index: u32,
    tx_data: &[u8],
    chunk_size: usize,
    cs: Option<u32>,
) -> Result<(), String> {
    if tx_data.is_empty() {
        return Ok(());
    }
    if chunk_size == 0 || chunk_size > MAX_NATIVE_SPI_WRITE {
        return Err(format!(
            "CH347_SPI_CHUNK_LENGTH_INVALID:{chunk_size}/{MAX_NATIVE_SPI_WRITE}"
        ));
    }
    if !matches!(cs, None | Some(0)) {
        return Err(
            "CH347_SPI_CONFIG_UNSUPPORTED:native backend supports grounded CS only".to_owned(),
        );
    }
    with_claimed(state, index, "SPI write buffer", |handle| {
        for chunk in tx_data.chunks(chunk_size) {
            let packet = spi_write_packet(chunk, cs)?;
            let reply = command_exchange(handle, &packet, "SPI write buffer")?;
            validate_command_ack(&reply, SPI_WRITE_COMMAND, "SPI write buffer")?;
        }
        Ok(())
    })
}

fn spi_write_packet(tx_data: &[u8], cs: Option<u32>) -> Result<Vec<u8>, String> {
    if tx_data.is_empty() || tx_data.len() > MAX_NATIVE_SPI_WRITE {
        return Err(format!(
            "CH347_SPI_WRITE_LENGTH_INVALID:{}/{}",
            tx_data.len(),
            MAX_NATIVE_SPI_WRITE
        ));
    }
    if !matches!(cs, None | Some(0)) {
        return Err(
            "CH347_SPI_CONFIG_UNSUPPORTED:native backend supports grounded CS only".to_owned(),
        );
    }
    let length = tx_data.len() as u16;
    let mut packet = Vec::with_capacity(tx_data.len() + 3);
    packet.extend_from_slice(&[SPI_WRITE_COMMAND, length as u8, (length >> 8) as u8]);
    packet.extend_from_slice(tx_data);
    Ok(packet)
}

#[command]
pub async fn ch347_spi_fill(
    app: AppHandle,
    _state: State<'_, Arc<Ch347State>>,
    _index: u32,
    _color: Vec<u8>,
    _pixel_count: u32,
    _cs: Option<u32>,
) -> Result<(), String> {
    crate::hardware_safety::ensure_idle(&app, "CH347 SPI 填充")?;
    unavailable()
}

#[command]
pub async fn ch347_spi_write_buffer(
    app: AppHandle,
    state: State<'_, Arc<Ch347State>>,
    index: u32,
    data: Vec<u8>,
    cs: Option<u32>,
) -> Result<(), String> {
    crate::hardware_safety::ensure_idle(&app, "CH347 SPI 缓冲写入")?;
    spi_write_chunks_core(&state, index, &data, MAX_NATIVE_SPI_WRITE, cs)
}

#[command]
#[allow(clippy::too_many_arguments)]
pub fn ch347_spi_read(
    app: AppHandle,
    _state: State<'_, Arc<Ch347State>>,
    _index: u32,
    _read_len: u32,
    _cs: Option<u32>,
    _mode: Option<u8>,
    _speed_mhz: Option<u32>,
    _data_bits: Option<u8>,
) -> Result<Vec<u8>, String> {
    crate::hardware_safety::ensure_idle(&app, "CH347 SPI 读取")?;
    unavailable()
}

#[allow(dead_code)]
pub fn spi_read_core(
    _state: &Ch347State,
    _index: u32,
    _read_len: u32,
    _cs: Option<u32>,
) -> Result<Vec<u8>, String> {
    unavailable()
}

#[command]
#[allow(clippy::too_many_arguments)]
pub fn ch347_spi_transfer(
    app: AppHandle,
    _state: State<'_, Arc<Ch347State>>,
    _index: u32,
    _tx_data: Vec<u8>,
    _cs: Option<u32>,
    _mode: Option<u8>,
    _speed_mhz: Option<u32>,
    _data_bits: Option<u8>,
) -> Result<Vec<u8>, String> {
    crate::hardware_safety::ensure_idle(&app, "CH347 SPI 全双工传输")?;
    unavailable()
}

#[allow(dead_code)]
pub fn spi_transfer_core(
    _state: &Ch347State,
    _index: u32,
    _tx_data: Vec<u8>,
    _cs: Option<u32>,
) -> Result<Vec<u8>, String> {
    unavailable()
}

#[command]
#[allow(clippy::too_many_arguments)]
pub fn ch347_spi_stream4(
    app: AppHandle,
    _state: State<'_, Arc<Ch347State>>,
    _index: u32,
    _tx_data: Vec<u8>,
    _cs: Option<u32>,
    _mode: Option<u8>,
    _speed_mhz: Option<u32>,
    _data_bits: Option<u8>,
) -> Result<Vec<u8>, String> {
    crate::hardware_safety::ensure_idle(&app, "CH347 SPI 流传输")?;
    unavailable()
}

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
    if !state
        .open_count
        .lock()
        .map_err(|_| "CH347_STATE_LOCK".to_owned())?
        .contains_key(&index)
    {
        return Err(format!("CH347_NOT_OPEN:index={index}"));
    }
    let _guard = state
        .io_lock
        .lock()
        .map_err(|_| "CH347_IO_LOCK".to_owned())?;
    // GPIO0 is handled entirely in hardware and must never be configured by
    // software, even if a legacy caller still includes bit 0.
    let (enable, dir_out, data_out) = without_gpio0(enable, dir_out, data_out);
    let (handle, interface) = open_claimed(index)?;
    let packet = gpio_packet(enable, dir_out, data_out);
    let written = handle
        .write_bulk(ENDPOINT_OUT, &packet, USB_TIMEOUT)
        .map_err(|error| usb_error("GPIO write", error))?;
    if written != packet.len() {
        return Err(format!("CH347_GPIO_SHORT_WRITE:{written}/{}", packet.len()));
    }
    let mut reply = [0u8; 11];
    let read = handle
        .read_bulk(ENDPOINT_IN, &mut reply, USB_TIMEOUT)
        .map_err(|error| usb_error("GPIO read", error))?;
    let result = validate_gpio_reply(&reply[..read], enable, dir_out, data_out);
    let release = handle
        .release_interface(interface)
        .map_err(|error| usb_error("release", error));
    result.and(release)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpio_packet_encodes_enable_direction_and_level_masks() {
        let packet = gpio_packet(0b0001_0011, 0b0001_0001, 0b0001_0000);
        assert_eq!(&packet[..3], &[0xcc, 8, 0]);
        assert_eq!(packet[3], 0xf0);
        assert_eq!(packet[4], 0xc0);
        assert_eq!(packet[7], 0xf8);
    }

    #[test]
    fn gpio_reply_checks_output_level() {
        let mut reply = gpio_packet(0x01, 0x01, 0x01);
        reply[3] = 0x40;
        assert!(validate_gpio_reply(&reply, 0x01, 0x01, 0x01).is_ok());
        reply[3] = 0;
        assert!(validate_gpio_reply(&reply, 0x01, 0x01, 0x01).is_err());
    }

    #[test]
    fn gpio0_is_always_removed_from_software_commands() {
        assert_eq!(without_gpio0(0x11, 0x11, 0x01), (0x10, 0x10, 0x00));
        assert_eq!(without_gpio0(0x01, 0x01, 0x01), (0x00, 0x00, 0x00));
    }

    #[test]
    fn only_lynx_ch347f_usb_identity_is_supported() {
        assert!(is_ch347f(0x1a86, 0x55de, rusb::Version(1, 2, 0)));
        assert!(!is_ch347f(0x1a86, 0x55de, rusb::Version(4, 4, 1)));
        assert!(!is_ch347f(0x1a86, 0x55db, rusb::Version(1, 2, 0)));
    }

    #[test]
    fn ina226_register_read_uses_repeated_start_and_final_nack() {
        assert_eq!(
            i2c_packet(&[0x80, 0xfe], 2).unwrap(),
            vec![0xaa, 0x74, 0x82, 0x80, 0xfe, 0x74, 0x81, 0x81, 0xc1, 0xc0, 0x75, 0x00]
        );
    }

    #[test]
    fn st7789_mode3_packet_matches_wch_stream_configuration() {
        assert_eq!(
            spi_config_packet(3, 1, Some(0), 8, 1, 0, 0xff, 0, 0, 1, 0, 0).unwrap(),
            vec![
                0xc0, 0x1a, 0x00, 0x00, 0x00, 0x04, 0x01, 0x00, 0x00, 0x02, 0x00, 0x01, 0x00, 0x00,
                0x02, 0x30, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0xff, 0x00, 0x00, 0x00, 0x06,
                0x00,
            ]
        );
    }

    #[test]
    fn native_spi_write_is_bounded_and_grounded_cs_only() {
        assert_eq!(
            spi_write_packet(&[0x11, 0x29], Some(0)).unwrap(),
            vec![0xc4, 0x02, 0x00, 0x11, 0x29]
        );
        assert!(spi_write_packet(&vec![0; MAX_NATIVE_SPI_WRITE + 1], Some(0)).is_err());
        assert!(spi_write_packet(&[0x11], Some(1)).is_err());
    }
}
