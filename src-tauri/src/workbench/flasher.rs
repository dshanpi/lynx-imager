//! Vendor-neutral command-line flasher adapter.
//!
//! The adapter launches a configured executable with an argument array. It
//! never constructs a shell command, so firmware paths and user options cannot
//! escape into a shell. Raw output and process exit status remain authoritative.

mod arguments;
mod rockchip_raw;
mod scan;

use arguments::{build_flash_args, validate_dfu_args, version_args};
use scan::{parse_dfu_util_scan, parse_openrockcli_scan, parse_uuu_scan};

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};
use tauri::{command, AppHandle, Emitter, Manager, State};

use crate::k230::burner::{Burner as K230Burner, ProgressFn as K230ProgressFn};
use crate::k230::kdimage::KdImage;
use crate::k230::protocol::usb::K230UsbDevice;
use crate::k230::types::{MediumType, K230_USB_PID, K230_USB_VID};
use crate::process::configure_background_command;

use super::storage;
use super::types::{DeviceConfig, FlasherDeviceCandidate, TaskEvent};
use super::WorkbenchState;

pub(crate) fn cleanup_rockchip_raw_cache(app: &AppHandle) {
    let result = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())
        .and_then(|root| rockchip_raw::cleanup(&root.join("rockchip-raw-v1")));
    if let Err(error) = result {
        log::warn!("RK raw stale cache cleanup: {error}");
    }
}

const FLASHER_PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const RAW_PREFLIGHT_STALL_TIMEOUT: Duration = Duration::from_secs(30);
const RAW_BOOTSTRAP_STALL_TIMEOUT: Duration = Duration::from_secs(90);
const FLASHER_TERMINATE_TIMEOUT: Duration = Duration::from_secs(5);
const FLASHER_COOPERATIVE_CANCEL_TIMEOUT: Duration = Duration::from_secs(12);
const FLASHER_STREAM_RECORD_LIMIT: usize = 64 * 1024;
const FLASHER_PROBE_OUTPUT_LIMIT: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FlashCapabilities {
    pub chip_vendor: String,
    pub flasher_kind: String,
    pub modes: Vec<&'static str>,
    pub default_mode: Option<&'static str>,
    pub post_flash_actions: Vec<&'static str>,
    pub supports_verify: bool,
    pub artifact_formats: Vec<&'static str>,
    pub mainline_workflow: &'static str,
    pub audit_required: bool,
    pub vendor_options: Value,
    pub safety_policy: FlashSafetyPolicy,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FlashSafetyPolicy {
    pub boot_protocol: &'static str,
    pub boot_entry: &'static str,
    pub erase_strategy: &'static str,
    pub software_boot_transition_allowed: bool,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct AdapterFlashOptions {
    pub mode: Option<String>,
    pub verify: Option<bool>,
    pub post_flash_action: Option<String>,
    pub vendor_options: Option<Value>,
}

enum FlashBackend {
    Allwinner,
    Rockchip,
    Nxp,
    Kendryte,
    StDfu,
}

#[derive(Clone, Copy)]
pub(crate) enum FlashAuditActor {
    User,
    Workflow,
    Gateway,
}

impl FlashAuditActor {
    fn identity(self) -> (&'static str, &'static str) {
        match self {
            Self::User => ("user", "local-user"),
            Self::Workflow => ("service", "workbench-workflow"),
            Self::Gateway => ("ai", "linux-mcp-gateway"),
        }
    }
}

impl FlashBackend {
    fn for_device(device: &DeviceConfig) -> Result<Self, String> {
        match (device.chip_vendor.as_str(), device.flasher_kind.as_str()) {
            ("allwinner", "openixcli") => Ok(Self::Allwinner),
            ("rockchip", "openrockcli") => Ok(Self::Rockchip),
            ("nxp", "uuu") => Ok(Self::Nxp),
            ("kendryte", "k230") => Ok(Self::Kendryte),
            ("st", "dfu-util") => Ok(Self::StDfu),
            (vendor, flasher) => Err(format!(
                "FLASH_BACKEND_MISMATCH:chip vendor {vendor} cannot use flasher {flasher}"
            )),
        }
    }

    fn capabilities(&self, device: &DeviceConfig) -> FlashCapabilities {
        let safety_policy = self.safety_policy();
        match self {
            Self::Allwinner => FlashCapabilities {
                chip_vendor: device.chip_vendor.clone(),
                flasher_kind: device.flasher_kind.clone(),
                modes: vec!["command", "partition", "partition_erase", "full_erase"],
                default_mode: Some("full_erase"),
                post_flash_actions: vec!["none", "reboot", "poweroff"],
                supports_verify: true,
                artifact_formats: vec![
                    "allwinner_vendor_package",
                    "mainline_raw_disk_image",
                    "mainline_nand_component_package",
                ],
                mainline_workflow: "Use a board-matched loader only in RAM to enter FES. NAND and SPI NAND require a component package, partition_erase (FES policy 0x1) for MBR/layout replacement, and strict Boot0/Boot1/partition verification; SD/SD NAND, eMMC and UFS may use whole-disk raw after the runtime storage-type preflight. SPI NOR requires the supported F101 RAM Loader, a runtime-verified physical-address contract and independent full-flash readback; unsupported or unverified profiles are blocked before destructive commands.",
                audit_required: true,
                vendor_options: json!({
                    "raw": {
                        "type": "boolean",
                        "description": "Treat firmwarePath as the mainline whole-disk logical raw image"
                    },
                    "medium": {"enum": ["spinor"], "description": "Explicit physical SPI NOR route; requires a supported loader, verified physical mapping and independent readback"},
                    "loaderPath": {
                        "type": "string",
                        "description": "Board-matched loader used only in RAM to initialize DRAM and enter FES for either raw or NAND component flashing"
                    }
                }),
                safety_policy,
            },
            Self::Rockchip => FlashCapabilities {
                chip_vendor: device.chip_vendor.clone(),
                flasher_kind: device.flasher_kind.clone(),
                modes: vec!["full_erase"],
                default_mode: Some("full_erase"),
                post_flash_actions: if cfg!(windows) {
                    vec!["reboot", "none", "poweroff"]
                } else {
                    vec!["reboot", "poweroff"]
                },
                supports_verify: true,
                artifact_formats: vec!["rockchip_update_image", "rockchip_raw_emmc", "rockchip_raw_emmc_xz", "rockchip_raw_emmc_gz"],
                mainline_workflow: "Use a complete board-matched .img/.img.xz/.img.gz with vendorOptions raw=true, loaderPath and medium=emmc; mandatory verification and full erase. RAM Loader is not written to media. Validated update.img packages remain supported.",
                audit_required: true,
                vendor_options: json!({"raw": {"type":"boolean"}, "loaderPath": {"type":"string", "description":"Board-matched RAM Loader; required for raw"}, "medium": {"enum":["emmc"]}}),
                safety_policy,
            },
            Self::Nxp => FlashCapabilities {
                chip_vendor: device.chip_vendor.clone(),
                flasher_kind: device.flasher_kind.clone(),
                modes: vec![],
                default_mode: None,
                post_flash_actions: vec![],
                supports_verify: false,
                artifact_formats: vec!["uuu_script", "imx_boot_container"],
                mainline_workflow: "use the board's saved UUU script; arbitrary caller arguments remain prohibited",
                audit_required: true,
                vendor_options: json!({"configuredArgsOnly": true}),
                safety_policy,
            },
            Self::Kendryte => FlashCapabilities {
                chip_vendor: device.chip_vendor.clone(),
                flasher_kind: device.flasher_kind.clone(),
                modes: vec![],
                default_mode: None,
                post_flash_actions: vec!["reboot", "none"],
                supports_verify: false,
                artifact_formats: vec!["kdimage", "raw_image"],
                mainline_workflow: "use the built-in K230 backend with an advertised target medium",
                audit_required: true,
                vendor_options: json!({
                    "medium": ["emmc", "sdcard", "spi_nand", "spi_nor", "otp"],
                }),
                safety_policy,
            },
            Self::StDfu => FlashCapabilities {
                chip_vendor: device.chip_vendor.clone(),
                flasher_kind: device.flasher_kind.clone(),
                modes: vec![],
                default_mode: None,
                post_flash_actions: vec!["reset", "none"],
                supports_verify: false,
                artifact_formats: vec!["dfu", "dfuse", "raw_binary"],
                mainline_workflow: "use dfu-util with validated alternate/address vendor options",
                audit_required: true,
                vendor_options: json!({
                    "alt": "DFU alternate setting number or name",
                    "address": "optional DfuSe address expression"
                }),
                safety_policy,
            },
        }
    }

    fn safety_policy(&self) -> FlashSafetyPolicy {
        match self {
            Self::Allwinner => FlashSafetyPolicy {
                boot_protocol: "FEL / FES",
                boot_entry: "physical FEL key/boot strap, or configured reliable hardware reset/power controller",
                erase_strategy: "full_erase by default; verify enabled; override only when preserving data is intentional",
                software_boot_transition_allowed: false,
            },
            Self::Rockchip => FlashSafetyPolicy {
                boot_protocol: "Maskrom / Loader",
                boot_entry: "lynx_rockchip_enter_loader: fixed bound ADB or Linux serial reboot loader, followed by OpenRockCLI Loader confirmation; physical Maskrom key/boot strap for recovery",
                erase_strategy: "full_erase by default; verify enabled; avoids stale UBI/UBIFS journal data",
                software_boot_transition_allowed: true,
            },
            Self::Nxp => FlashSafetyPolicy {
                boot_protocol: "SDP / Fastboot",
                boot_entry: "physical boot-mode straps/switches, or configured reliable hardware reset/power controller",
                erase_strategy: "the saved UUU script/arguments are authoritative; LYNX does not invent a generic erase command",
                software_boot_transition_allowed: false,
            },
            Self::Kendryte => FlashSafetyPolicy {
                boot_protocol: "BROM / U-Boot",
                boot_entry: "physical boot key/strap, or configured reliable hardware reset/power controller",
                erase_strategy: "the built-in K230 backend erases each image target before writing; no generic full_erase option",
                software_boot_transition_allowed: false,
            },
            Self::StDfu => FlashSafetyPolicy {
                boot_protocol: "USB DFU / DfuSe",
                boot_entry: "physical BOOT switches/straps, or configured reliable hardware reset/power controller",
                erase_strategy: "DfuSe target/alternate/address controls erase and write; no generic full_erase option",
                software_boot_transition_allowed: false,
            },
        }
    }

    fn override_args(
        &self,
        device: &DeviceConfig,
        options: AdapterFlashOptions,
    ) -> Result<Option<Vec<String>>, String> {
        match self {
            Self::Allwinner => {
                let capabilities = self.capabilities(device);
                let mode = options
                    .mode
                    .unwrap_or_else(|| capabilities.default_mode.unwrap_or("partition").to_owned());
                if !capabilities.modes.contains(&mode.as_str()) {
                    return Err(format!("FLASH_OPTION_UNSUPPORTED:mode={mode}"));
                }
                let action = options
                    .post_flash_action
                    .unwrap_or_else(|| "reboot".to_owned());
                if !capabilities.post_flash_actions.contains(&action.as_str()) {
                    return Err(format!("FLASH_OPTION_UNSUPPORTED:postFlashAction={action}"));
                }
                let raw = if let Some(value) = options.vendor_options.as_ref() {
                    let object = value.as_object().ok_or_else(|| {
                        "FLASH_OPTION_INVALID:vendorOptions must be an object".to_owned()
                    })?;
                    if object
                        .keys()
                        .any(|key| !matches!(key.as_str(), "raw" | "loaderPath" | "medium"))
                    {
                        return Err(
                            "FLASH_OPTION_UNSUPPORTED:unknown Allwinner vendor option".to_owned()
                        );
                    }
                    object.get("raw").and_then(Value::as_bool).unwrap_or(false)
                } else {
                    false
                };
                if raw && mode != "command" {
                    return Err(
                        "FLASH_OPTION_INVALID:Allwinner raw requires mode=command".to_owned()
                    );
                }
                if !raw && mode == "command" {
                    return Err("FLASH_OPTION_INVALID:mode=command requires raw=true".to_owned());
                }
                let mut args = Vec::new();
                if let Some(medium) = options
                    .vendor_options
                    .as_ref()
                    .and_then(|value| value.get("medium"))
                {
                    if !raw || medium.as_str() != Some("spinor") {
                        return Err("FLASH_OPTION_INVALID:Allwinner medium requires raw=true and medium=spinor".to_owned());
                    }
                    args.push("--lynx-spinor".to_owned());
                }
                if raw {
                    args.push("--raw".to_owned());
                }
                let bootstrap = options
                    .vendor_options
                    .as_ref()
                    .and_then(Value::as_object)
                    .and_then(|object| object.get("loaderPath"));
                if raw && bootstrap.is_none() {
                    return Err("FLASH_OPTION_INVALID:Allwinner raw requires loaderPath".to_owned());
                }
                if let Some(bootstrap) = bootstrap {
                    let bootstrap = bootstrap.as_str().ok_or_else(|| {
                        "FLASH_OPTION_INVALID:loaderPath must be a string".to_owned()
                    })?;
                    if bootstrap.trim().is_empty() || !Path::new(bootstrap).is_file() {
                        return Err(format!("BOOTSTRAP_FIRMWARE_NOT_FOUND:{}", bootstrap.trim()));
                    }
                    args.extend(["--lynx-loader".to_owned(), bootstrap.to_owned()]);
                }
                args.extend([
                    "--verify".to_owned(),
                    options.verify.unwrap_or(!raw).to_string(),
                    "--mode".to_owned(),
                    mode,
                    "--post-action".to_owned(),
                    action,
                ]);
                Ok(Some(args))
            }
            Self::Rockchip => {
                let capabilities = self.capabilities(device);
                let mode = options
                    .mode
                    .unwrap_or_else(|| capabilities.default_mode.unwrap_or("partition").to_owned());
                if !capabilities.modes.contains(&mode.as_str()) {
                    return Err(format!("FLASH_OPTION_UNSUPPORTED:mode={mode}"));
                }
                let action = options
                    .post_flash_action
                    .unwrap_or_else(|| "reboot".to_owned());
                if !capabilities.post_flash_actions.contains(&action.as_str()) {
                    return Err(format!("FLASH_OPTION_UNSUPPORTED:postFlashAction={action}"));
                }
                let mut raw_args = Vec::new();
                if let Some(value) = options.vendor_options.as_ref() {
                    let object = value
                        .as_object()
                        .ok_or("FLASH_OPTION_INVALID:vendorOptions must be an object")?;
                    if !object.is_empty() {
                        if object
                            .keys()
                            .any(|k| !matches!(k.as_str(), "raw" | "loaderPath" | "medium"))
                            || object.get("raw").and_then(Value::as_bool) != Some(true)
                            || object.get("medium").and_then(Value::as_str) != Some("emmc")
                        {
                            return Err(
                                "FLASH_OPTION_INVALID:RK raw requires raw=true and medium=emmc"
                                    .to_owned(),
                            );
                        }
                        let loader = object
                            .get("loaderPath")
                            .and_then(Value::as_str)
                            .filter(|p| !p.trim().is_empty())
                            .ok_or("FLASH_OPTION_INVALID:RK raw requires loaderPath")?;
                        let metadata = std::fs::metadata(loader)
                            .map_err(|_| "FLASH_OPTION_INVALID:RK Loader unavailable")?;
                        if !metadata.is_file()
                            || metadata.len() == 0
                            || metadata.len() > 64 * 1024 * 1024
                        {
                            return Err("FLASH_OPTION_INVALID:RK Loader must be a nonempty file up to 64 MiB".to_owned());
                        }
                        if options.verify == Some(false) {
                            return Err(
                                "FLASH_OPTION_UNSUPPORTED:RK raw requires verification".to_owned()
                            );
                        }
                        raw_args.extend([
                            "--raw".to_owned(),
                            "--loader".to_owned(),
                            loader.to_owned(),
                            "--storage".to_owned(),
                            "emmc".to_owned(),
                        ]);
                    }
                }
                raw_args.extend([
                    "--verify".to_owned(),
                    options.verify.unwrap_or(true).to_string(),
                    "--mode".to_owned(),
                    mode,
                    "--post-action".to_owned(),
                    action,
                ]);
                Ok(Some(raw_args))
            }
            Self::Nxp => {
                if options.mode.is_some()
                    || options.verify.is_some()
                    || options.post_flash_action.is_some()
                    || options.vendor_options.is_some()
                {
                    return Err(
                        "FLASH_OPTION_UNSUPPORTED:NXP uses the board's saved UUU arguments"
                            .to_owned(),
                    );
                }
                Ok(None)
            }
            Self::Kendryte => {
                if options.mode.is_some() || options.verify.is_some() {
                    return Err("FLASH_OPTION_UNSUPPORTED:K230 mode/verify".to_owned());
                }
                let mut args = parse_extra_args(&device.flasher_args)?;
                if let Some(value) = options.vendor_options {
                    let object = value.as_object().ok_or_else(|| {
                        "FLASH_OPTION_INVALID:vendorOptions must be an object".to_owned()
                    })?;
                    if object.keys().any(|key| key != "medium") {
                        return Err(
                            "FLASH_OPTION_UNSUPPORTED:unknown K230 vendor option".to_owned()
                        );
                    }
                    if let Some(medium) = object.get("medium").and_then(Value::as_str) {
                        let mut filtered = Vec::with_capacity(args.len());
                        let mut index = 0;
                        while index < args.len() {
                            if args[index] == "--medium" {
                                index += 2;
                            } else {
                                filtered.push(args[index].clone());
                                index += 1;
                            }
                        }
                        args = filtered;
                        args.extend(["--medium".to_owned(), medium.to_owned()]);
                    }
                }
                match options.post_flash_action.as_deref() {
                    Some("none") if !args.iter().any(|arg| arg == "--no-reboot") => {
                        args.push("--no-reboot".to_owned())
                    }
                    Some("reboot") => args.retain(|arg| arg != "--no-reboot"),
                    None => {}
                    Some(other) => {
                        return Err(format!("FLASH_OPTION_UNSUPPORTED:postFlashAction={other}"))
                    }
                }
                Ok(Some(args))
            }
            Self::StDfu => {
                if options.mode.is_some() || options.verify.is_some() {
                    return Err("FLASH_OPTION_UNSUPPORTED:STM32 DFU mode/verify".to_owned());
                }
                let mut args = parse_extra_args(&device.flasher_args)?;
                if let Some(value) = options.vendor_options {
                    let object = value.as_object().ok_or_else(|| {
                        "FLASH_OPTION_INVALID:vendorOptions must be an object".to_owned()
                    })?;
                    if object
                        .keys()
                        .any(|key| !matches!(key.as_str(), "alt" | "address"))
                    {
                        return Err(
                            "FLASH_OPTION_UNSUPPORTED:unknown STM32 DFU vendor option".to_owned()
                        );
                    }
                    if let Some(alt) = object.get("alt").and_then(Value::as_str) {
                        replace_option(&mut args, &["-a", "--alt"], "-a", alt);
                    }
                    if let Some(address) = object.get("address").and_then(Value::as_str) {
                        replace_option(&mut args, &["-s", "--dfuse-address"], "-s", address);
                    }
                }
                match options.post_flash_action.as_deref() {
                    Some("reset") => {
                        if !args
                            .iter()
                            .any(|arg| matches!(arg.as_str(), "-R" | "--reset"))
                        {
                            args.push("-R".to_owned());
                        }
                    }
                    Some("none") => {
                        args.retain(|arg| !matches!(arg.as_str(), "-R" | "--reset"));
                    }
                    None => {}
                    Some(other) => {
                        return Err(format!("FLASH_OPTION_UNSUPPORTED:postFlashAction={other}"))
                    }
                }
                validate_dfu_args(&args)?;
                Ok(Some(args))
            }
        }
    }
}

fn replace_option(args: &mut Vec<String>, names: &[&str], canonical: &str, value: &str) {
    let mut index = 0;
    while index < args.len() {
        if names.contains(&args[index].as_str()) {
            args.drain(index..usize::min(index + 2, args.len()));
        } else {
            index += 1;
        }
    }
    if !value.trim().is_empty() {
        args.extend([canonical.to_owned(), value.to_owned()]);
    }
}

pub(crate) fn flash_capabilities(device: &DeviceConfig) -> Result<FlashCapabilities, String> {
    Ok(FlashBackend::for_device(device)?.capabilities(device))
}

pub(crate) fn flash_safety_policy(chip_vendor: &str) -> Option<FlashSafetyPolicy> {
    match chip_vendor {
        "allwinner" => Some(FlashBackend::Allwinner.safety_policy()),
        "rockchip" => Some(FlashBackend::Rockchip.safety_policy()),
        "nxp" => Some(FlashBackend::Nxp.safety_policy()),
        "kendryte" => Some(FlashBackend::Kendryte.safety_policy()),
        "st" => Some(FlashBackend::StDfu.safety_policy()),
        _ => None,
    }
}

pub(crate) fn adapter_override_args(
    device: &DeviceConfig,
    options: AdapterFlashOptions,
) -> Result<Option<Vec<String>>, String> {
    FlashBackend::for_device(device)?.override_args(device, options)
}

#[command]
pub fn workbench_test_flasher(
    kind: String,
    path: String,
    app: AppHandle,
) -> Result<String, String> {
    if kind == "k230" {
        return Ok(
            "LYNX 内置嘉楠 K230 烧录协议（BootROM / UBOOT，支持 EMMC、SD、SPI NAND、SPI NOR、OTP）"
                .to_owned(),
        );
    }
    let executable = resolve_executable(&app, &kind, &path)?;
    let mut command = Command::new(&executable);
    command.args(version_args(&kind)?);
    let output = command_output_with_timeout(&mut command, FLASHER_PROBE_TIMEOUT)?;
    if !output.status.success() {
        return Err(format!(
            "FLASHER_EXIT:{}:{}",
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    Ok(if stdout.trim().is_empty() {
        stderr.trim().to_owned()
    } else {
        stdout.trim().to_owned()
    })
}

#[command]
pub async fn workbench_scan_flasher_devices(
    kind: String,
    path: String,
    app: AppHandle,
    state: State<'_, Arc<WorkbenchState>>,
) -> Result<Vec<FlasherDeviceCandidate>, String> {
    crate::hardware_safety::ensure_idle(&app, "Linux 工作台 USB 烧录设备扫描")?;
    let safety = app.state::<crate::hardware_safety::HardwareSafetyState>();
    let _hardware_guard = safety.try_begin("Linux 工作台", "USB 烧录设备扫描", kind.clone())?;
    let flasher_lock = state.flasher_lock(&kind)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = flasher_lock.try_lock().map_err(|_| {
            format!("FLASHER_BUSY:{kind} is currently performing a hardware operation")
        })?;
        scan_flasher_devices(&app, &kind, &path)
    })
    .await
    .map_err(|error| error.to_string())?
}

pub(crate) fn scan_flasher_devices(
    app: &AppHandle,
    kind: &str,
    path: &str,
) -> Result<Vec<FlasherDeviceCandidate>, String> {
    let result = match kind {
        "openixcli" => match libefex::Context::scan_usb_devices() {
            Ok(devices) => Ok(devices
                .into_iter()
                .map(|device| FlasherDeviceCandidate {
                    key: device.location_path.as_ref().map_or_else(
                        || format!("{}:{}", device.bus, device.port),
                        |location| stable_allwinner_binding(location),
                    ),
                    flasher_kind: "openixcli".to_owned(),
                    bus: u16::from(device.bus),
                    port: device.port.to_string(),
                    vid: device.vid,
                    pid: device.pid,
                    mode: "fel/fes".to_owned(),
                    chip: "Allwinner".to_owned(),
                })
                .collect()),
            Err(error)
                if error
                    .to_string()
                    .to_ascii_lowercase()
                    .contains("device not found") =>
            {
                Ok(Vec::new())
            }
            Err(error) => Err(format!("USB_SCAN:{error}")),
        },
        "openrockcli" => {
            let executable = resolve_executable(app, kind, path)?;
            let mut command = Command::new(&executable);
            command.arg("scan");
            let output = command_output_with_timeout(&mut command, FLASHER_PROBE_TIMEOUT)?;
            if !output.status.success() {
                let detail = format!(
                    "{}\n{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
                if detail.to_ascii_lowercase().contains("device not found") {
                    Ok(Vec::new())
                } else {
                    Err(format!(
                        "FLASHER_EXIT:{}:{}",
                        output.status.code().unwrap_or(-1),
                        detail.trim()
                    ))
                }
            } else {
                parse_openrockcli_scan(&String::from_utf8_lossy(&output.stdout))
            }
        }
        "uuu" => {
            let executable = resolve_executable(app, kind, path)?;
            let mut command = Command::new(&executable);
            command.arg("-lsusb");
            let output = command_output_with_timeout(&mut command, FLASHER_PROBE_TIMEOUT)?;
            if !output.status.success() {
                return Err(format!(
                    "FLASHER_EXIT:{}:{}",
                    output.status.code().unwrap_or(-1),
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
            parse_uuu_scan(&String::from_utf8_lossy(&output.stdout))
        }
        "k230" => K230UsbDevice::list_all()
            .map(|devices| {
                devices
                    .into_iter()
                    .map(|(bus, port)| FlasherDeviceCandidate {
                        key: format!("{bus}:{port}"),
                        flasher_kind: "k230".to_owned(),
                        bus: u16::from(bus),
                        port: port.to_string(),
                        vid: K230_USB_VID,
                        pid: K230_USB_PID,
                        mode: "bootrom / loader".to_owned(),
                        chip: "K230 / K230D".to_owned(),
                    })
                    .collect()
            })
            .map_err(|error| format!("USB_SCAN:{error}")),
        "dfu-util" => {
            let executable = resolve_executable(app, kind, path)?;
            let mut command = Command::new(&executable);
            command.arg("--list");
            let output = command_output_with_timeout(&mut command, FLASHER_PROBE_TIMEOUT)?;
            if !output.status.success() {
                return Err(format!(
                    "FLASHER_EXIT:{}:{}",
                    output.status.code().unwrap_or(-1),
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
            let listing = format!(
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            Ok(parse_dfu_util_scan(&listing))
        }
        other => Err(format!("FLASHER_UNSUPPORTED:{other}")),
    };
    match &result {
        Ok(devices) => log::debug!(
            "workbench flasher scan kind={kind} path={} found={}",
            if path.trim().is_empty() {
                "<bundled>"
            } else {
                path
            },
            devices.len()
        ),
        Err(error) => log::warn!(
            "workbench flasher scan kind={kind} path={} failed: {error}",
            if path.trim().is_empty() {
                "<bundled>"
            } else {
                path
            }
        ),
    }
    result
}

fn stable_allwinner_binding(location_path: &str) -> String {
    format!("location:{}", location_path.trim().to_ascii_lowercase())
}

pub(crate) fn allwinner_binding_matches(expected: &str, candidate: &str) -> bool {
    if expected.eq_ignore_ascii_case(candidate) {
        return true;
    }
    let Some(expected) = expected.strip_prefix("location:libusb:") else {
        return false;
    };
    let Some((expected_bus, expected_leaf)) = expected.split_once(':') else {
        return false;
    };
    if expected_leaf.contains('.') {
        return false;
    }
    let Some(candidate) = candidate.strip_prefix("location:libusb:") else {
        return false;
    };
    let Some((candidate_bus, candidate_path)) = candidate.split_once(':') else {
        return false;
    };
    expected_bus == candidate_bus
        && candidate_path
            .rsplit('.')
            .next()
            .is_some_and(|leaf| leaf == expected_leaf)
}

#[command]
pub fn workbench_start_flash(
    device_id: i64,
    firmware_path: String,
    options: Option<LocalFlashOptions>,
    app: AppHandle,
    state: State<'_, Arc<WorkbenchState>>,
) -> Result<String, String> {
    if let Some(options) = options {
        let device = {
            let db = state.db.lock().map_err(|error| error.to_string())?;
            storage::device_by_id(&db, device_id).map_err(|error| error.to_string())?
        };
        let args = local_flash_args(&device, options)?;
        return start_flash_with_args(
            device_id,
            firmware_path,
            args,
            app,
            state.inner(),
            FlashAuditActor::User,
        );
    }
    start_flash_with_actor(
        device_id,
        firmware_path,
        app,
        state.inner(),
        FlashAuditActor::User,
    )
}

/// The offline UI exposes bounded options, never a USB selector or arbitrary arguments.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalFlashOptions {
    system_type: String,
    loader_path: Option<String>,
    mainline_route: Option<String>,
    mode: Option<String>,
    verify: Option<bool>,
    post_flash_action: Option<String>,
}

fn local_flash_args(
    device: &DeviceConfig,
    options: LocalFlashOptions,
) -> Result<Option<Vec<String>>, String> {
    match options.system_type.as_str() {
        "mainline" => {
            if device.chip_vendor == "rockchip" {
                // Existing local callers used mainline to submit packaged update.img.
                if options.mainline_route.is_none()
                    && options.loader_path.is_none()
                    && options.mode.is_none()
                    && options.verify.is_none()
                    && options.post_flash_action.is_none()
                {
                    return Ok(None);
                }
                if options.mainline_route.as_deref() != Some("rockchip_raw_emmc") {
                    return Err("FLASH_OPTION_INVALID:请选择 RK raw eMMC 路线".to_owned());
                }
                return adapter_override_args(
                    device,
                    AdapterFlashOptions {
                        mode: options.mode,
                        verify: options.verify,
                        post_flash_action: options.post_flash_action,
                        vendor_options: Some(
                            json!({"raw":true,"loaderPath":options.loader_path,"medium":"emmc"}),
                        ),
                    },
                );
            }
            // Only enumerated storage routes and a local Loader are selectable.
            // Saved bindings and the execution/resource gate remain authoritative.
            if options.mode.is_some()
                || options.verify.is_some()
                || (options.post_flash_action.is_some()
                    && options.mainline_route.as_deref() != Some("mainline_spinor"))
            {
                return Err("FLASH_OPTION_UNSUPPORTED:主线路线使用已保存的烧录选项".to_owned());
            }
            if device.chip_vendor == "allwinner" {
                if device.flasher_kind != "openixcli" {
                    return Err(
                        "FLASH_OPTION_UNSUPPORTED:FES Loader 需要 Allwinner 烧录器".to_owned()
                    );
                }
                let mut args = parse_extra_args(&device.flasher_args)?;
                let saved_route = if args.iter().any(|value| value == "--raw")
                    && args.iter().any(|value| value == "--lynx-spinor")
                {
                    "mainline_spinor"
                } else if args.iter().any(|value| value == "--raw")
                    && args.iter().any(|value| value == "--emmc-boot0-from-image")
                {
                    "mainline_emmc"
                } else if args.iter().any(|value| value == "--lynx-loader") {
                    "mainline_nand"
                } else {
                    ""
                };
                let route = options.mainline_route.as_deref().unwrap_or(saved_route);
                if !matches!(route, "mainline_nand" | "mainline_emmc" | "mainline_spinor") {
                    return Err(
                        "FLASH_OPTION_INVALID:请选择主线 NAND、eMMC 或 SPI NOR 存储路线".to_owned(),
                    );
                }
                let loader = options
                    .loader_path
                    .as_deref()
                    .or_else(|| argument_value(&args, "--lynx-loader"))
                    .or_else(|| argument_value(&args, "--lynx-raw-loader"))
                    .filter(|path| !path.trim().is_empty())
                    .ok_or_else(|| "FLASH_OPTION_INVALID:请选择 RAM FES Loader".to_owned())?
                    .to_owned();
                let metadata = std::fs::metadata(&loader).map_err(|_| {
                    "BOOTSTRAP_FIRMWARE_NOT_FOUND:FES Loader 不存在或无法读取".to_owned()
                })?;
                if !metadata.is_file() || metadata.len() == 0 {
                    return Err("BOOTSTRAP_FIRMWARE_NOT_FOUND:FES Loader 不是非空文件".to_owned());
                }
                if route == "mainline_spinor" {
                    let action = options
                        .post_flash_action
                        .as_deref()
                        .or_else(|| argument_value(&args, "--post-action"))
                        .unwrap_or("none");
                    if !matches!(action, "none" | "reboot" | "poweroff") {
                        return Err(format!("FLASH_OPTION_UNSUPPORTED:postFlashAction={action}"));
                    }
                    // NOR owns its exact argument set; stale eMMC Boot0 flags
                    // must never cross from a previously selected route.
                    args = vec![
                        "--raw",
                        "--lynx-raw-loader",
                        "",
                        "--lynx-spinor",
                        "--post-action",
                        action,
                    ]
                    .into_iter()
                    .map(str::to_owned)
                    .collect();
                } else if route != saved_route {
                    // Match the existing settings presets; never accept arbitrary CLI flags.
                    args = if route == "mainline_emmc" {
                        vec!["--raw", "--lynx-raw-loader", "", "--emmc-boot0-from-image"]
                    } else {
                        vec![
                            "--lynx-loader",
                            "",
                            "--verify",
                            "true",
                            "--mode",
                            "full_erase",
                            "--post-action",
                            "none",
                        ]
                    }
                    .into_iter()
                    .map(str::to_owned)
                    .collect();
                }
                let mut index = 0;
                let mut replaced = false;
                while index < args.len() {
                    if matches!(args[index].as_str(), "--lynx-loader" | "--lynx-raw-loader") {
                        if index + 1 >= args.len() {
                            return Err("FLASH_OPTION_INVALID:FES Loader 参数缺少路径".to_owned());
                        }
                        args[index + 1] = loader.clone();
                        replaced = true;
                        index += 2;
                    } else {
                        index += 1;
                    }
                }
                if !replaced {
                    return Err("FLASH_OPTION_INVALID:主线路线缺少 FES Loader 参数".to_owned());
                }
                return Ok(Some(args));
            }
            if options.loader_path.is_some() || options.mainline_route.is_some() {
                return Err("FLASH_OPTION_UNSUPPORTED:此平台不使用 Allwinner FES Loader".to_owned());
            }
            Ok(None)
        }
        "vendor" => {
            if options.loader_path.is_some() || options.mainline_route.is_some() {
                return Err("FLASH_OPTION_UNSUPPORTED:厂商镜像不接受主线 FES 选项".to_owned());
            }
            if matches!(device.flasher_kind.as_str(), "openixcli" | "openrockcli") {
                // command/raw requires a loader and is not a vendor-package route.
                if options.mode.as_deref() == Some("command") {
                    return Err(
                        "FLASH_OPTION_UNSUPPORTED:厂商镜像不能使用 raw command 模式".to_owned()
                    );
                }
                adapter_override_args(
                    device,
                    AdapterFlashOptions {
                        mode: options.mode,
                        verify: options.verify,
                        post_flash_action: options.post_flash_action,
                        vendor_options: None,
                    },
                )
            } else if options.mode.is_some()
                || options.verify.is_some()
                || options.post_flash_action.is_some()
            {
                Err("FLASH_OPTION_UNSUPPORTED:此烧录器使用已保存的专用配置".to_owned())
            } else {
                Ok(None)
            }
        }
        _ => Err("FLASH_OPTION_INVALID:未知系统类型".to_owned()),
    }
}

fn start_flash_with_actor(
    device_id: i64,
    firmware_path: String,
    app: AppHandle,
    state: &Arc<WorkbenchState>,
    actor: FlashAuditActor,
) -> Result<String, String> {
    start_flash_with_args(device_id, firmware_path, None, app, state, actor)
}

pub(crate) fn start_flash_with_args(
    device_id: i64,
    firmware_path: String,
    override_args: Option<Vec<String>>,
    app: AppHandle,
    state: &Arc<WorkbenchState>,
    actor: FlashAuditActor,
) -> Result<String, String> {
    start_flash_with_args_and_guard(
        device_id,
        firmware_path,
        override_args,
        app,
        state,
        actor,
        StartFlashExecutionContext::default(),
    )
}

#[derive(Default)]
pub(crate) struct StartFlashExecutionContext {
    pub(crate) hardware_guard: Option<crate::hardware_safety::HardwareActivityGuard>,
    pub(crate) preserve_serial_monitor: bool,
}

pub(crate) fn start_flash_with_args_and_guard(
    device_id: i64,
    firmware_path: String,
    override_args: Option<Vec<String>>,
    app: AppHandle,
    state: &Arc<WorkbenchState>,
    actor: FlashAuditActor,
    execution: StartFlashExecutionContext,
) -> Result<String, String> {
    if !Path::new(&firmware_path).is_file() {
        return Err(format!("FIRMWARE_NOT_FOUND:{firmware_path}"));
    }
    let device = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        storage::device_by_id(&db, device_id).map_err(|e| e.to_string())?
    };
    FlashBackend::for_device(&device)?;
    if device.usb_binding.trim().is_empty() {
        return Err("FLASH_TARGET_UNBOUND:请先绑定当前单板的 USB 烧录接口".to_owned());
    }
    ensure_stable_allwinner_binding(&device.flasher_kind, &device.usb_binding)?;
    let hardware_guard = match execution.hardware_guard {
        Some(guard) => guard,
        None => {
            let safety = app.state::<crate::hardware_safety::HardwareSafetyState>();
            safety.try_begin_scoped(
                "Linux 工作台",
                "烧录",
                format!("device:{device_id}:usb:{}", device.usb_binding),
                flasher_resource_key(&device.flasher_kind, &device.usb_binding),
            )?
        }
    };
    let task_id = format!(
        "flash-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    publish_flash_intent(&app, &task_id, &device, &firmware_path, actor)?;
    let is_rk_raw = device.flasher_kind == "openrockcli"
        && override_args
            .as_ref()
            .is_some_and(|args| args.iter().any(|a| a == "--raw"));
    let worker_owns_geometry_serial = device.flasher_kind == "openixcli"
        && override_args.as_ref().is_some_and(|args| {
            args.iter()
                .any(|argument| argument == "--bootstrap-firmware")
        });
    let serial = app.state::<Arc<crate::serial::SerialState>>();
    if !is_rk_raw
        && !device.serial_port.trim().is_empty()
        && crate::serial::commands::serial_is_open_core(
            serial.inner().as_ref(),
            &device.serial_port,
        )
    {
        if execution.preserve_serial_monitor
            && device.flasher_kind == "openixcli"
            && !worker_owns_geometry_serial
        {
            log::info!(
                "Retained target serial monitor during Allwinner FES bootstrap device={device_id}: {}",
                device.serial_port
            );
        } else {
            crate::serial::commands::serial_close_core(
                serial.inner().as_ref(),
                &device.serial_port,
            )?;
            log::info!(
                "Closed target LYNX serial before flash device={device_id}: {}",
                device.serial_port
            );
        }
    }
    if !is_rk_raw {
        let preflight_lock =
            state.flasher_target_lock(&device.flasher_kind, &device.usb_binding)?;
        let detected_targets = {
            let _guard = preflight_lock.try_lock().map_err(|_| {
                format!(
                    "FLASHER_BUSY:{} is currently performing a hardware operation",
                    device.flasher_kind
                )
            })?;
            scan_flasher_devices(&app, &device.flasher_kind, &device.flasher_path)
        }
        .map_err(|error| {
            format!(
                "FLASH_TARGET_SCAN_FAILED:{} 协议未发现可烧录设备（{error}）",
                device.flasher_kind
            )
        })?;
        let matching_targets = detected_targets
            .iter()
            .filter(|candidate| {
                if device.flasher_kind == "openixcli" {
                    allwinner_binding_matches(&device.usb_binding, &candidate.key)
                } else {
                    candidate.key == device.usb_binding
                }
            })
            .count();
        if matching_targets != 1 {
            return Err(format!(
            "FLASH_TARGET_NOT_FOUND:绑定目标 {} 未以 {} 协议唯一在线（匹配 {} 个），已拒绝启动烧录",
            device.usb_binding, device.flasher_kind, matching_targets
        ));
        }
    }
    let executable = if device.flasher_kind == "k230" {
        None
    } else {
        Some(resolve_executable(
            &app,
            &device.flasher_kind,
            &device.flasher_path,
        )?)
    };
    let extra_args = match override_args {
        Some(args) => args,
        None => parse_extra_args(&device.flasher_args)?,
    };
    if device.flasher_kind == "dfu-util" {
        validate_dfu_args(&extra_args)?;
    }
    let cancellation = Arc::new(AtomicBool::new(false));
    let device_lock = state.device_lock(device_id)?;
    let flasher_lock = state.flasher_target_lock(&device.flasher_kind, &device.usb_binding)?;
    insert_task(state, &task_id, device_id)?;
    match state.cancellations.lock() {
        Ok(mut tasks) => {
            tasks.insert(task_id.clone(), cancellation.clone());
        }
        Err(error) => {
            let detail = format!("FLASH_CANCELLATION_STATE:{error}");
            finish_task(state, &task_id, "failed", None, &detail);
            return Err(detail);
        }
    }

    let state = state.clone();
    let return_id = task_id.clone();
    std::thread::spawn(move || {
        let _hardware_guard = hardware_guard;
        let _guard = match device_lock.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                let error = "DEVICE_BUSY:another device operation is running".to_owned();
                finish_task(&state, &task_id, "failed", None, &error);
                emit(&app, &task_id, "system", error, None, "failed");
                if let Ok(mut tasks) = state.cancellations.lock() {
                    tasks.remove(&task_id);
                }
                return;
            }
        };
        let _flasher_guard = match flasher_lock.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                let error = format!(
                    "FLASHER_BUSY:{} is currently performing a hardware operation",
                    device.flasher_kind
                );
                finish_task(&state, &task_id, "failed", None, &error);
                emit(&app, &task_id, "system", error, None, "failed");
                if let Ok(mut tasks) = state.cancellations.lock() {
                    tasks.remove(&task_id);
                }
                return;
            }
        };
        let result = if device.flasher_kind == "k230" {
            run_k230_flasher(
                &app,
                &task_id,
                &device,
                &firmware_path,
                &extra_args,
                &cancellation,
            )
        } else {
            let result = run_flasher(
                &app,
                &task_id,
                &device,
                &firmware_path,
                extra_args.clone(),
                &cancellation,
                executable
                    .as_deref()
                    .expect("non-K230 flashers always resolve an executable"),
                Some(&state),
            );
            match result {
                Ok(0) => ensure_allwinner_reboot_fallback(&device, &extra_args, &state).map(|_| 0),
                other => other,
            }
        };
        let (mut status, exit_code, mut error) = match result {
            Ok(0) => ("success", Some(0), String::new()),
            Ok(code) => (
                "failed",
                Some(code),
                format!("{} exited with {code}", device.flasher_kind),
            ),
            Err(error) if cancellation.load(Ordering::Relaxed) => ("cancelled", None, error),
            Err(error) => ("failed", None, error),
        };
        if let Err(audit_error) =
            publish_flash_result(&app, &task_id, &device, status, exit_code, actor, &error)
        {
            let hardware_status = status;
            status = "failed";
            error =
                format!("AUDIT_RESULT_WRITE_FAILED:hardwareStatus={hardware_status}:{audit_error}");
        }
        finish_task(&state, &task_id, status, exit_code, &error);
        emit(&app, &task_id, "system", error.clone(), None, status);
        if let Ok(mut tasks) = state.cancellations.lock() {
            tasks.remove(&task_id);
        }
        crate::task_lifecycle::publish_workbench_completion(
            &app,
            crate::task_lifecycle::WorkbenchTaskCompletion {
                task_id,
                device_id,
                kind: "flash".to_owned(),
                status: status.to_owned(),
                error,
            },
        );
    });
    Ok(return_id)
}

pub(crate) fn publish_flash_intent(
    app: &AppHandle,
    task_id: &str,
    device: &DeviceConfig,
    firmware_path: &str,
    actor: FlashAuditActor,
) -> Result<(), String> {
    let firmware_size = std::fs::metadata(firmware_path)
        .map_err(|_| "FLASH_FIRMWARE_METADATA_UNAVAILABLE".to_owned())?
        .len();
    let (actor_type, actor_id) = actor.identity();
    crate::audit::publish_required(
        app,
        crate::audit::AuditEventDraft {
            actor_type: actor_type.to_owned(),
            actor_id: actor_id.to_owned(),
            category: "flash".to_owned(),
            action: "flash_intended".to_owned(),
            scope_type: "linux_board".to_owned(),
            scope_id: format!("linux:{}", device.id.unwrap_or_default()),
            session_id: Some(task_id.to_owned()),
            session_kind: None,
            session_title: None,
            end_session: false,
            correlation_id: Some(task_id.to_owned()),
            causation_id: None,
            result: "authorized".to_owned(),
            error_code: None,
            summary: "准备启动受控固件烧录".to_owned(),
            metadata: serde_json::json!({
                "taskId": task_id,
                "chipVendor": device.chip_vendor,
                "flasherKind": device.flasher_kind,
                "firmwareSizeBytes": firmware_size,
            }),
        },
    )?;
    Ok(())
}

pub(crate) fn publish_flash_result(
    app: &AppHandle,
    task_id: &str,
    device: &DeviceConfig,
    status: &str,
    exit_code: Option<i32>,
    actor: FlashAuditActor,
    error: &str,
) -> Result<(), String> {
    let (actor_type, actor_id) = actor.identity();
    let error_code = if status == "success" {
        None
    } else {
        Some(crate::audit::machine_error_code(
            error,
            "FLASH_ACTION_FAILED",
        ))
    };
    crate::audit::publish_required(
        app,
        crate::audit::AuditEventDraft {
            actor_type: actor_type.to_owned(),
            actor_id: actor_id.to_owned(),
            category: "flash".to_owned(),
            action: "flash_completed".to_owned(),
            scope_type: "linux_board".to_owned(),
            scope_id: format!("linux:{}", device.id.unwrap_or_default()),
            session_id: Some(task_id.to_owned()),
            session_kind: None,
            session_title: None,
            end_session: false,
            correlation_id: Some(task_id.to_owned()),
            causation_id: None,
            result: status.to_owned(),
            error_code,
            summary: if status == "success" {
                "固件烧录完成并通过既有成功判定".to_owned()
            } else {
                "固件烧录未通过既有成功判定".to_owned()
            },
            metadata: serde_json::json!({
                "taskId": task_id,
                "chipVendor": device.chip_vendor,
                "flasherKind": device.flasher_kind,
                "exitCode": exit_code,
            }),
        },
    )?;
    Ok(())
}

fn ensure_allwinner_reboot_fallback(
    device: &DeviceConfig,
    args: &[String],
    state: &WorkbenchState,
) -> Result<(), String> {
    if device.flasher_kind != "openixcli" || argument_value(args, "--post-action") != Some("reboot")
    {
        return Ok(());
    }
    // The embedded libefex implementation owns process-global payload state.
    // It may coexist with an external openixcli process on another endpoint,
    // but two in-process libefex operations must never overlap.
    let libefex_lock = state.flasher_lock("libefex-in-process")?;
    let Ok(_libefex_guard) = libefex_lock.try_lock() else {
        // openixcli already returned success after the requested post-action.
        // Do not overwrite verified flash success merely because the optional
        // in-process reboot fallback is occupied by another board.
        return Ok(());
    };
    let location = device.usb_binding.strip_prefix("location:");
    let legacy_pair = if location.is_none() {
        Some(parse_k230_binding(&device.usb_binding).map_err(|_| {
            format!(
                "ALLWINNER_DEVICE:invalid USB binding {}",
                device.usb_binding
            )
        })?)
    } else {
        None
    };
    let is_present = || {
        libefex::Context::scan_usb_devices()
            .map(|devices| {
                devices.iter().any(|candidate| {
                    location.map_or_else(
                        || {
                            legacy_pair.is_some_and(|(bus, port)| {
                                candidate.bus == bus && candidate.port == port
                            })
                        },
                        |expected| candidate.location_path.as_deref() == Some(expected),
                    )
                })
            })
            .unwrap_or(true)
    };
    if !is_present() {
        return Ok(());
    }

    let mut context = libefex::Context::new();
    let opened = if let Some(location) = location {
        context.scan_usb_device_by_location(location)
    } else {
        let (bus, port) = legacy_pair.expect("legacy pair");
        context.scan_usb_device_at(bus, port)
    };
    if let Err(error) = opened {
        // The endpoint can disappear between the presence snapshot above and
        // opening it here because the vendor post-action already rebooted the
        // board. The earlier openixcli success is authoritative for the flash
        // and verification phases; this optional reopen cannot reverse it.
        if reboot_open_race_is_success(&error.to_string()) {
            return Ok(());
        }
        return Err(format!("POST_FLASH_REBOOT_OPEN:{error}"));
    }
    context
        .usb_init()
        .map_err(|error| format!("POST_FLASH_REBOOT_USB:{error}"))?;
    context
        .efex_init()
        .map_err(|error| format!("POST_FLASH_REBOOT_EFEX:{error}"))?;
    context
        .fes_tool_mode(libefex::FesToolMode::Normal, libefex::FesToolMode::Reboot)
        .map_err(|error| format!("POST_FLASH_REBOOT_COMMAND:{error}"))?;
    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(250));
        if !is_present() {
            return Ok(());
        }
    }
    Err("POST_FLASH_REBOOT_FAILED:device remained in FEL/FES after fallback".to_owned())
}

fn reboot_open_race_is_success(error: &str) -> bool {
    // This fallback runs only after openixcli has exited successfully, which
    // means partition writes and the requested verification have completed.
    // USB enumeration may remain stale for a short time after reboot; failure
    // to reopen the old FEL endpoint must not overwrite the verified result.
    error.to_ascii_lowercase().contains("device not found")
}

pub(crate) fn flasher_resource_key(flasher_kind: &str, usb_binding: &str) -> String {
    format!("{}:{}", flasher_kind.trim(), usb_binding.trim())
}

pub(crate) fn ensure_stable_allwinner_binding(
    flasher_kind: &str,
    usb_binding: &str,
) -> Result<(), String> {
    if flasher_kind == "openixcli" && !usb_binding.starts_with("location:") {
        return Err(
            "USB_BINDING_LEGACY_REBIND_REQUIRED:旧版 0:N 是 Windows 枚举序号，会在 FEL/FES 重枚举后变化；请在单板设置中重新扫描并选择物理 USB 口"
                .to_owned(),
        );
    }
    Ok(())
}

pub(crate) fn run_k230_flasher(
    app: &AppHandle,
    task_id: &str,
    device: &DeviceConfig,
    firmware_path: &str,
    extra_args: &[String],
    cancellation: &AtomicBool,
) -> Result<i32, String> {
    let (bus, port) = parse_k230_binding(&device.usb_binding)?;
    let (medium, reboot) = parse_k230_options(extra_args)?;
    let image = KdImage::parse(firmware_path).map_err(|error| format!("K230_IMAGE:{error}"))?;
    emit(
        app,
        task_id,
        "system",
        format!(
            "启动内置 K230 烧录：Bus {bus} / Port {port}，目标介质 {}",
            medium_label(medium)
        ),
        Some(0),
        "running",
    );

    let progress_app = app.clone();
    let progress_task_id = task_id.to_owned();
    let progress: K230ProgressFn = Box::new(move |current, total, stage| {
        let percent = if total == 0 {
            None
        } else {
            Some(((current as f64 / total as f64) * 100.0).clamp(0.0, 100.0) as u64)
        };
        emit(
            &progress_app,
            &progress_task_id,
            "stdout",
            format!("K230 {stage}: {current}/{total}"),
            percent,
            "running",
        );
        let phase = if stage.starts_with("erase") {
            "erase"
        } else if stage == "reboot" {
            "post_flash_action"
        } else if matches!(stage, "handshake" | "load_loader" | "wait_uboot" | "resync") {
            "preflight"
        } else {
            "write"
        };
        persist_simple_progress(
            &progress_app,
            &progress_task_id,
            phase,
            percent.map(|value| value as f64),
            json!({
                "backendStage": stage,
                "current": current,
                "total": total,
            }),
        );
    });
    K230Burner::new(medium)
        .burn(bus, port, &image, &progress, reboot, cancellation)
        .map_err(|error| format!("K230_FLASH:{error}"))?;
    Ok(0)
}

fn persist_simple_progress(
    app: &AppHandle,
    task_id: &str,
    phase: &str,
    progress: Option<f64>,
    details: Value,
) {
    let state = app.state::<Arc<WorkbenchState>>();
    let Ok(details_json) = serde_json::to_string(&details) else {
        return;
    };
    if let Ok(db) = state.db.lock() {
        let _ = db.execute(
            "UPDATE task_history SET phase=?1,progress=?2,details_json=?3 WHERE task_id=?4",
            rusqlite::params![phase, progress, details_json, task_id],
        );
    };
}

fn parse_k230_binding(value: &str) -> Result<(u8, u8), String> {
    let (bus, port) = value
        .trim()
        .split_once(':')
        .ok_or_else(|| "K230_DEVICE:请选择已连接的 BootROM / Loader 接口".to_owned())?;
    let bus = bus
        .parse::<u8>()
        .map_err(|_| format!("K230_DEVICE:无效 USB Bus：{bus}"))?;
    let port = port
        .parse::<u8>()
        .map_err(|_| format!("K230_DEVICE:无效 USB Port：{port}"))?;
    Ok((bus, port))
}

fn parse_k230_options(args: &[String]) -> Result<(MediumType, bool), String> {
    let mut medium = MediumType::Emmc;
    let mut reboot = true;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--medium" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "K230_ARGS:--medium 缺少介质名称".to_owned())?;
                medium = match value.to_ascii_lowercase().replace('-', "_").as_str() {
                    "emmc" => MediumType::Emmc,
                    "sd" | "sdcard" | "sd_card" => MediumType::SdCard,
                    "spinand" | "spi_nand" => MediumType::SpiNand,
                    "spinor" | "spi_nor" => MediumType::SpiNor,
                    "otp" => MediumType::Otp,
                    _ => return Err(format!("K230_ARGS:不支持的目标介质 {value}")),
                };
                index += 2;
            }
            "--no-reboot" => {
                reboot = false;
                index += 1;
            }
            value => return Err(format!("K230_ARGS:不支持的内置选项 {value}")),
        }
    }
    Ok((medium, reboot))
}

fn medium_label(value: MediumType) -> &'static str {
    match value {
        MediumType::Emmc => "EMMC",
        MediumType::SdCard => "SD Card",
        MediumType::SpiNand => "SPI NAND",
        MediumType::SpiNor => "SPI NOR",
        MediumType::Otp => "OTP",
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_flasher(
    app: &AppHandle,
    task_id: &str,
    device: &DeviceConfig,
    firmware_path: &str,
    extra_args: Vec<String>,
    cancellation: &AtomicBool,
    executable: &Path,
    state: Option<&WorkbenchState>,
) -> Result<i32, String> {
    let mut extra_args = extra_args;
    let mut telemetry = FlashTelemetry::new(
        &device.usb_binding,
        argument_value(&extra_args, "--post-action").unwrap_or("unknown"),
    );
    let prepared =
        if device.flasher_kind == "openrockcli" && extra_args.iter().any(|a| a == "--raw") {
            let mut probe = Command::new(executable);
            probe.args(["flash", "--help"]);
            let output = command_output_with_timeout(&mut probe, FLASHER_PROBE_TIMEOUT)?;
            if !output.status.success()
                || !String::from_utf8_lossy(&output.stdout).contains("--raw --loader")
            {
                return Err(
                    "ROCKCHIP_RAW_TOOL_VERSION:当前 OpenRockCLI 不支持 raw eMMC，请更新配套烧录器"
                        .to_owned(),
                );
            }
            let loader = argument_value(&extra_args, "--loader")
                .ok_or("ROCKCHIP_RAW_LOADER:missing Loader")?;
            let root = app
                .path()
                .app_cache_dir()
                .map_err(|e| e.to_string())?
                .join("rockchip-raw-v1");
            let mut last_update = Instant::now() - Duration::from_secs(1);
            let prepared = rockchip_raw::prepare(
                Path::new(firmware_path),
                Path::new(loader),
                &root,
                cancellation,
                |phase, bytes| {
                    if bytes == 0 || last_update.elapsed() >= Duration::from_millis(250) {
                        telemetry.phase = phase.to_owned();
                        telemetry.transferred_bytes = Some(bytes);
                        telemetry.progress_pct = None;
                        persist_telemetry(state, task_id, &telemetry);
                        emit_telemetry(app, task_id, &telemetry);
                        last_update = Instant::now();
                    }
                },
            )?;
            telemetry.phase = "raw_metadata".to_owned();
            telemetry.raw_artifacts = Some(json!({"sourcePath":firmware_path,"loaderPath":loader,
            "sourceSha256":prepared.source_sha256,"imageSha256":prepared.image_sha256,
            "loaderSha256":prepared.loader_sha256,"imageBytes":prepared.bytes}));
            persist_telemetry(state, task_id, &telemetry);
            emit_telemetry(app, task_id, &telemetry);
            let mut inspect = Command::new(executable);
            inspect
                .arg("inspect-raw")
                .arg(&prepared.image)
                .arg("--loader")
                .arg(&prepared.loader);
            let output = command_output_with_timeout(&mut inspect, FLASHER_PROBE_TIMEOUT)?;
            if !output.status.success() {
                return Err(format!(
                    "ROCKCHIP_RAW_FORMAT:{}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
            if cancellation.load(Ordering::Relaxed) {
                return Err("ROCKCHIP_RAW_CANCELLED".to_owned());
            }
            crate::audit::publish_required(
                app,
                crate::audit::AuditEventDraft {
                    actor_type: "system".to_owned(),
                    actor_id: "rockchip-raw-preparation".to_owned(),
                    category: "flash".to_owned(),
                    action: "raw_artifacts_prepared".to_owned(),
                    scope_type: "linux_board".to_owned(),
                    scope_id: format!("linux:{}", device.id.unwrap_or_default()),
                    session_id: Some(task_id.to_owned()),
                    session_kind: None,
                    session_title: None,
                    end_session: false,
                    correlation_id: Some(task_id.to_owned()),
                    causation_id: None,
                    result: "verified".to_owned(),
                    error_code: None,
                    summary: "RK raw 镜像准备与文件哈希记录完成，尚未擦写".to_owned(),
                    metadata: telemetry.raw_artifacts.clone().unwrap_or(Value::Null),
                },
            )?;
            let candidates = scan_flasher_devices(app, &device.flasher_kind, &device.flasher_path)?;
            if candidates
                .iter()
                .filter(|c| c.key == device.usb_binding)
                .count()
                != 1
            {
                return Err("FLASH_TARGET_NOT_FOUND:绑定 RK 目标未唯一在线".to_owned());
            }
            let serial = app.state::<Arc<crate::serial::SerialState>>();
            if !device.serial_port.trim().is_empty()
                && crate::serial::commands::serial_is_open_core(
                    serial.inner().as_ref(),
                    &device.serial_port,
                )
            {
                crate::serial::commands::serial_close_core(
                    serial.inner().as_ref(),
                    &device.serial_port,
                )?;
            }
            let index = extra_args
                .iter()
                .position(|a| a == "--loader")
                .ok_or("ROCKCHIP_RAW_LOADER:missing Loader")?;
            extra_args[index + 1] = prepared.loader.to_string_lossy().into_owned();
            Some(prepared)
        } else {
            None
        };
    let prepared_path = prepared
        .as_ref()
        .map(|p| p.image.to_string_lossy().into_owned());
    let firmware_path = prepared_path.as_deref().unwrap_or(firmware_path);
    let mut args = build_flash_args(
        &device.flasher_kind,
        &device.usb_binding,
        firmware_path,
        extra_args,
        device.flasher_kind != "uuu" && supports_jsonl(executable),
    );
    let raw_geometry_environment = openix_raw_geometry_environment(device, &args);
    if device.flasher_kind == "openixcli"
        && argument_value(&args, "--bootstrap-firmware").is_some()
        && !device.serial_port.trim().is_empty()
    {
        args.extend([
            "--storage-geometry-serial-port".to_owned(),
            device.serial_port.clone(),
            "--storage-geometry-serial-baud".to_owned(),
            device.baud_rate.max(1_200).to_string(),
        ]);
    }
    // Derive defaults from the actual command, including legacy callers whose
    // empty extra_args gain --post-action reboot in build_flash_args.
    telemetry.post_flash_action = argument_value(&args, "--post-action")
        .unwrap_or("unknown")
        .to_owned();
    telemetry.post_flash_state = if telemetry.post_flash_action == "none" {
        "skipped"
    } else {
        "pending"
    }
    .to_owned();
    telemetry.phase = "preflight".to_owned();
    telemetry.progress_pct = Some(0.0);
    telemetry.transferred_bytes = None;
    telemetry.executable = executable.display().to_string();
    telemetry.executable_version = probe_flasher_version(&device.flasher_kind, executable);
    telemetry.arguments = args.clone();
    if argument_value(&args, "--mode") == Some("spinor-physical") {
        telemetry.mark_spinor();
    }
    persist_telemetry(state, task_id, &telemetry);
    emit_telemetry(app, task_id, &telemetry);
    emit(
        app,
        task_id,
        "system",
        format!(
            "Starting {} with {} argument(s)",
            executable.display(),
            args.len()
        ),
        None,
        "running",
    );
    let mut command = Command::new(executable);
    command
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(prepared) = prepared.as_ref() {
        prepared.protect_child(&mut command)?;
    }
    if let Some((port, baud)) = raw_geometry_environment {
        command.env("OPENIXCLI_STORAGE_GEOMETRY_SERIAL_PORT", port);
        command.env("OPENIXCLI_STORAGE_GEOMETRY_SERIAL_BAUD", baud);
    }
    let cancellation_marker = FlasherCancellationMarker::new(task_id);
    command.env("LYNX_FLASH_CANCEL_FILE", cancellation_marker.path());
    configure_background_command(&mut command);
    let mut child = command.spawn().map_err(|e| format!("FLASHER_START:{e}"))?;
    let process_job = match crate::process::ProcessJob::attach(&child) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "FLASHER_STDOUT:pipe unavailable".to_owned())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "FLASHER_STDERR:pipe unavailable".to_owned())?;
    let (sender, receiver) = mpsc::channel::<(&'static str, Vec<u8>)>();
    spawn_reader(stdout, "stdout", sender.clone());
    spawn_reader(stderr, "stderr", sender);
    let raw_command = args.first().is_some_and(|argument| argument == "raw");
    let raw_flash = raw_command || args.iter().any(|argument| argument == "--raw");
    let vendor_bootstrap = raw_command || argument_value(&args, "--bootstrap-firmware").is_some();
    let raw_preflight_timeout = if vendor_bootstrap {
        RAW_BOOTSTRAP_STALL_TIMEOUT
    } else {
        RAW_PREFLIGHT_STALL_TIMEOUT
    };
    let mut raw_preflight_complete = false;
    let mut last_executor_output = Instant::now();

    loop {
        if cancellation.load(Ordering::Relaxed) {
            cancellation_marker.request()?;
            if wait_for_flasher_exit(&mut child, FLASHER_COOPERATIVE_CANCEL_TIMEOUT)? {
                return Err("Flashing cancelled by user".to_owned());
            }
            terminate_flasher_process_tree(&process_job, &mut child)?;
            return Err("Flashing cancelled by user".to_owned());
        }
        match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok((stream, bytes)) => {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                last_executor_output = Instant::now();
                if text.contains("RAW_ERASE_START") || text.contains("RAW_WRITE_START") {
                    raw_preflight_complete = true;
                }
                let progress = update_flash_telemetry(&text, &mut telemetry);
                persist_telemetry_if_due(state, task_id, &mut telemetry);
                emit_telemetry(app, task_id, &telemetry);
                emit(app, task_id, stream, text, progress, "running");
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {}
        }
        if raw_flash
            && !raw_preflight_complete
            && last_executor_output.elapsed() >= raw_preflight_timeout
        {
            terminate_flasher_process_tree(&process_job, &mut child)?;
            telemetry.phase = "preflight_timeout".to_owned();
            telemetry.urgent = true;
            persist_telemetry(state, task_id, &telemetry);
            emit_telemetry(app, task_id, &telemetry);
            return Err(format!(
                "RAW_PREFLIGHT_TIMEOUT: {} produced no output for {} seconds; process tree terminated before media write",
                device.flasher_kind, raw_preflight_timeout.as_secs()
            ));
        }
        if let Some(status) = child.try_wait().map_err(|e| format!("FLASHER_WAIT:{e}"))? {
            let drained =
                drain_flasher_output(&receiver, Duration::from_secs(2), |stream, bytes| {
                    let text = String::from_utf8_lossy(&bytes).into_owned();
                    let progress = update_flash_telemetry(&text, &mut telemetry);
                    persist_telemetry(state, task_id, &telemetry);
                    emit_telemetry(app, task_id, &telemetry);
                    emit(app, task_id, stream, text, progress, "running");
                });
            if let Err(error) = drained {
                emit(app, task_id, "system", error, None, "running");
            }
            let expected_post_action_disconnect = !status.success()
                && post_action_disconnect_is_success(&device.flasher_kind, &args, &telemetry);
            if status.success() && telemetry.is_spinor() {
                telemetry.phase = telemetry.spinor_completion_phase().to_owned();
                emit_telemetry(app, task_id, &telemetry);
            } else if status.success() || expected_post_action_disconnect {
                telemetry.phase = "complete".to_owned();
                telemetry.progress_pct = Some(100.0);
                telemetry.committed_bytes = telemetry.total_bytes;
                telemetry.written_bytes = telemetry.committed_bytes;
                if telemetry.current_partition.is_some() {
                    telemetry.partition_progress_pct = Some(100.0);
                }
                if argument_value(&args, "--verify") == Some("true") {
                    telemetry.verify_state = "success".to_owned();
                    telemetry.verify_error_code = Some(0);
                }
                if telemetry.post_flash_state != "skipped" {
                    telemetry.post_flash_state = "success".to_owned();
                    if expected_post_action_disconnect {
                        telemetry.post_flash_error = None;
                    }
                }
                emit_telemetry(app, task_id, &telemetry);
            } else {
                mark_telemetry_process_failure(&mut telemetry);
                if device.flasher_kind == "openrockcli" {
                    if let Some(error_code) = telemetry.diagnostic_error_code.as_deref() {
                        let diagnostics = app
                            .state::<Arc<crate::diagnostics::DiagnosticState>>()
                            .inner()
                            .clone();
                        crate::diagnostics::capture_rockchip_open_error(
                            diagnostics.as_ref(),
                            error_code,
                            &telemetry.phase,
                            task_id,
                        );
                        crate::diagnostics::schedule_rockchip_open_error_sync(app.clone());
                    }
                }
                emit_telemetry(app, task_id, &telemetry);
            }
            persist_telemetry(state, task_id, &telemetry);
            return Ok(if expected_post_action_disconnect {
                0
            } else {
                status.code().unwrap_or(-1)
            });
        }
    }
}

fn openix_raw_geometry_environment(
    device: &DeviceConfig,
    args: &[String],
) -> Option<(String, String)> {
    (device.flasher_kind == "openixcli"
        && args.first().is_some_and(|argument| argument == "raw")
        && !device.serial_port.trim().is_empty())
    .then(|| {
        (
            device.serial_port.clone(),
            device.baud_rate.max(1_200).to_string(),
        )
    })
}

struct FlasherCancellationMarker {
    path: PathBuf,
}

impl FlasherCancellationMarker {
    fn new(task_id: &str) -> Self {
        let safe_task_id = task_id
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || character == '-' {
                    character
                } else {
                    '_'
                }
            })
            .collect::<String>();
        let path = std::env::temp_dir().join(format!(
            "lynx-flasher-cancel-{}-{safe_task_id}-{:016x}.flag",
            std::process::id(),
            rand::random::<u64>()
        ));
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn request(&self) -> Result<(), String> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options
            .open(&self.path)
            .and_then(|mut file| file.write_all(b"cancel"))
            .map_err(|_| "FLASHER_CANCEL_SIGNAL_FAILED".to_owned())
    }
}

impl Drop for FlasherCancellationMarker {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn wait_for_flasher_exit(child: &mut Child, timeout: Duration) -> Result<bool, String> {
    let deadline = Instant::now() + timeout;
    loop {
        match child
            .try_wait()
            .map_err(|error| format!("FLASHER_CANCEL_WAIT:{error}"))?
        {
            Some(_) => return Ok(true),
            None if Instant::now() >= deadline => return Ok(false),
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

fn terminate_flasher_process_tree(
    process_job: &crate::process::ProcessJob,
    child: &mut Child,
) -> Result<(), String> {
    let _ = process_job.cancel_blocking_io();
    std::thread::sleep(Duration::from_millis(100));
    if child
        .try_wait()
        .map_err(|error| format!("FLASHER_KILL_WAIT:{error}"))?
        .is_some()
    {
        return Ok(());
    }
    let job_result = process_job.terminate();
    let child_result = child.kill();
    let deadline = Instant::now() + FLASHER_TERMINATE_TIMEOUT;
    loop {
        match child
            .try_wait()
            .map_err(|error| format!("FLASHER_KILL_WAIT:{error}"))?
        {
            Some(_) => {
                job_result?;
                if let Err(error) = child_result {
                    if child.try_wait().ok().flatten().is_none() {
                        return Err(format!("FLASHER_KILL:{error}"));
                    }
                }
                return Ok(());
            }
            None if Instant::now() >= deadline => {
                return Err(format!(
                    "FLASHER_KILL_TIMEOUT:{}ms",
                    FLASHER_TERMINATE_TIMEOUT.as_millis()
                ));
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

fn post_action_disconnect_is_success(
    flasher_kind: &str,
    args: &[String],
    telemetry: &FlashTelemetry,
) -> bool {
    let firmware_validation_finished = match telemetry.verify_state.as_str() {
        "success" => telemetry.verify_error_code == Some(0),
        // openixcli emits an explicit skipped result only after every partition
        // and Boot0/Boot1 have been written successfully. A later reboot can
        // legitimately remove the FES endpoint before its final reopen attempt.
        "skipped" => telemetry.verify_error_code.is_none(),
        _ => false,
    };

    !telemetry.is_spinor()
        && flasher_kind == "openixcli"
        && argument_value(args, "--post-action") == Some("reboot")
        && telemetry.phase == "post_flash_action"
        && firmware_validation_finished
        && telemetry
            .post_flash_error
            .as_deref()
            .is_some_and(|error| error.to_ascii_lowercase().contains("device not found"))
}

fn supports_jsonl(executable: &Path) -> bool {
    let mut command = Command::new(executable);
    command.arg("--help");
    command_output_with_timeout(&mut command, FLASHER_PROBE_TIMEOUT)
        .ok()
        .is_some_and(|output| String::from_utf8_lossy(&output.stdout).contains("--output"))
}

fn probe_flasher_version(kind: &str, executable: &Path) -> String {
    let Ok(args) = version_args(kind) else {
        return "unsupported".to_owned();
    };
    let mut command = Command::new(executable);
    command.args(args);
    let Ok(output) = command_output_with_timeout(&mut command, FLASHER_PROBE_TIMEOUT) else {
        return "unavailable".to_owned();
    };
    let value = if output.stdout.is_empty() {
        &output.stderr
    } else {
        &output.stdout
    };
    String::from_utf8_lossy(value)
        .lines()
        .next()
        .unwrap_or("unavailable")
        .trim()
        .chars()
        .take(160)
        .collect()
}

fn command_output_with_timeout(command: &mut Command, timeout: Duration) -> Result<Output, String> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    configure_background_command(command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("FLASHER_START:{error}"))?;
    let process_job = match crate::process::ProcessJob::attach(&child) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "FLASHER_PIPE:stdout unavailable".to_owned())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "FLASHER_PIPE:stderr unavailable".to_owned())?;
    let stdout_reader = std::thread::spawn(move || {
        let mut output = Vec::new();
        let mut stdout = stdout;
        stdout
            .by_ref()
            .take(FLASHER_PROBE_OUTPUT_LIMIT + 1)
            .read_to_end(&mut output)
            .map(|_| {
                output.truncate(FLASHER_PROBE_OUTPUT_LIMIT as usize);
                output
            })
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut output = Vec::new();
        let mut stderr = stderr;
        stderr
            .by_ref()
            .take(FLASHER_PROBE_OUTPUT_LIMIT + 1)
            .read_to_end(&mut output)
            .map(|_| {
                output.truncate(FLASHER_PROBE_OUTPUT_LIMIT as usize);
                output
            })
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child
            .try_wait()
            .map_err(|error| format!("FLASHER_WAIT:{error}"))?
        {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let termination = terminate_flasher_process_tree(&process_job, &mut child);
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                termination?;
                return Err(format!("FLASHER_TIMEOUT:{}ms", timeout.as_millis()));
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| "FLASHER_PIPE:stdout reader panicked".to_owned())?
        .map_err(|error| format!("FLASHER_PIPE:stdout: {error}"))?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| "FLASHER_PIPE:stderr reader panicked".to_owned())?
        .map_err(|error| format!("FLASHER_PIPE:stderr: {error}"))?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn drain_flasher_output(
    receiver: &mpsc::Receiver<(&'static str, Vec<u8>)>,
    timeout: Duration,
    mut consume: impl FnMut(&'static str, Vec<u8>),
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(
                "FLASHER_OUTPUT_DRAIN_TIMEOUT: final process logs may be incomplete".to_owned(),
            );
        }
        match receiver.recv_timeout(remaining) {
            Ok((stream, bytes)) => consume(stream, bytes),
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                return Err(
                    "FLASHER_OUTPUT_DRAIN_TIMEOUT: final process logs may be incomplete".to_owned(),
                );
            }
        }
    }
}

fn spawn_reader<R: Read + Send + 'static>(
    mut reader: R,
    stream: &'static str,
    sender: mpsc::Sender<(&'static str, Vec<u8>)>,
) {
    std::thread::spawn(move || {
        let mut pending = Vec::with_capacity(4096);
        let mut buffer = [0u8; 4096];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(size) => {
                    for record in take_stream_records(&mut pending, &buffer[..size]) {
                        if sender.send((stream, record)).is_err() {
                            return;
                        }
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    let _ = sender.send((
                        stream,
                        format!("FLASHER_STREAM_READ_FAILED:{error}").into_bytes(),
                    ));
                    break;
                }
            }
        }
        if !pending.is_empty() {
            let _ = sender.send((stream, pending));
        }
    });
}

/// Split terminal-style output on both newline and carriage return. Vendor
/// flashers commonly redraw a single progress line with `\r`; waiting only for
/// `\n` hides all progress until the child process exits.
fn take_stream_records(pending: &mut Vec<u8>, bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut records = Vec::new();
    for &byte in bytes {
        if matches!(byte, b'\r' | b'\n') {
            if !pending.is_empty() {
                records.push(std::mem::take(pending));
            }
        } else {
            if pending.len() < FLASHER_STREAM_RECORD_LIMIT {
                pending.push(byte);
            } else if pending.len() == FLASHER_STREAM_RECORD_LIMIT {
                pending.extend_from_slice(b"...[truncated]");
            }
        }
    }
    records
}

fn validate_executable(path: &str) -> Result<(), String> {
    if path.trim().is_empty() || !Path::new(path).is_file() {
        return Err(format!("FLASHER_NOT_FOUND:{path}"));
    }
    Ok(())
}

pub(crate) fn resolve_executable(
    app: &AppHandle,
    kind: &str,
    configured: &str,
) -> Result<std::path::PathBuf, String> {
    if !configured.trim().is_empty() {
        validate_executable(configured)?;
        return Ok(configured.into());
    }
    let binary = match (kind, cfg!(windows)) {
        ("openixcli", true) => "openixcli.exe",
        ("openixcli", false) => "openixcli",
        ("openrockcli", true) => "openrockcli.exe",
        ("openrockcli", false) => "openrockcli",
        ("uuu", true) => "uuu.exe",
        ("uuu", false) => "uuu",
        ("dfu-util", true) => "dfu-util.exe",
        ("dfu-util", false) => "dfu-util",
        (other, _) => return Err(format!("FLASHER_UNSUPPORTED:{other}")),
    };
    let mut candidates = Vec::new();
    if let Ok(resources) = app.path().resource_dir() {
        candidates.push(resources.join("bin").join(binary));
        candidates.push(resources.join(binary));
    }
    if let Ok(current) = std::env::current_exe() {
        if let Some(parent) = current.parent() {
            candidates.push(parent.join(binary));
        }
    }
    if let Some(paths) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&paths).map(|directory| directory.join(binary)));
    }
    candidates
        .into_iter()
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| format!("FLASHER_NOT_FOUND:{binary} is not bundled or available in PATH"))
}

pub(crate) use arguments::parse_extra_args;

fn parse_percent(text: &str) -> Option<u64> {
    for line in text.lines() {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
            if value.get("type").and_then(|item| item.as_str()) == Some("progress") {
                if let Some(percent) = value.get("percent").and_then(|item| item.as_f64()) {
                    return Some(percent.clamp(0.0, 100.0) as u64);
                }
            }
        }
    }
    text.split(|character: char| {
        character.is_whitespace() || matches!(character, '[' | ']' | '(' | ')' | '\r')
    })
    .find_map(|token| {
        let value = token.strip_suffix('%')?.trim();
        value
            .parse::<f64>()
            .ok()
            .map(|percent| percent.clamp(0.0, 100.0) as u64)
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FlashTelemetry {
    phase: String,
    progress_pct: Option<f64>,
    current_partition: Option<String>,
    completed_partitions: Vec<String>,
    partition_progress_pct: Option<f64>,
    transferred_bytes: Option<u64>,
    queued_bytes: Option<u64>,
    committed_bytes: Option<u64>,
    /// Compatibility projection for existing UI consumers. This is set only
    /// from media-confirmed bytes, never from USB transfer progress.
    written_bytes: Option<u64>,
    total_bytes: Option<u64>,
    bytes_per_second: Option<f64>,
    backend_bytes_per_second: Option<f64>,
    usb_speed: String,
    usb_theoretical_mbps: Option<f64>,
    executable: String,
    executable_version: String,
    arguments: Vec<String>,
    output_tail: Vec<String>,
    verify_state: String,
    verify_method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    storage_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    physical_verify_state: Option<String>,
    physical_image_sha256: Option<String>,
    write_state: Option<String>,
    outside_image_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    boot_verify_state: Option<String>,
    loader_capabilities: Option<String>,
    raw_artifacts: Option<Value>,
    verify_error_code: Option<i64>,
    post_flash_action: String,
    post_flash_state: String,
    post_flash_attempts: u32,
    post_flash_error: Option<String>,
    diagnostic_error_code: Option<String>,
    #[serde(skip)]
    last_persisted: Instant,
    #[serde(skip)]
    last_persisted_progress: Option<f64>,
    #[serde(skip)]
    last_persisted_phase: String,
    #[serde(skip)]
    urgent: bool,
}

impl FlashTelemetry {
    fn is_spinor(&self) -> bool {
        self.storage_type.as_deref() == Some("spinor")
    }

    fn spinor_completion_phase(&self) -> &'static str {
        if self.physical_verify_state.as_deref() == Some("success") {
            "spinor_awaiting_boot"
        } else {
            "spinor_awaiting_acceptance"
        }
    }

    fn mark_spinor(&mut self) {
        self.storage_type = Some("spinor".to_owned());
        self.physical_verify_state
            .get_or_insert_with(|| "not_verified".to_owned());
        self.boot_verify_state
            .get_or_insert_with(|| "not_verified".to_owned());
    }

    fn new(usb_binding: &str, post_flash_action: &str) -> Self {
        let (usb_speed, usb_theoretical_mbps) = usb_diagnostics(usb_binding);
        Self {
            phase: "preflight".to_owned(),
            progress_pct: Some(0.0),
            current_partition: None,
            completed_partitions: Vec::new(),
            partition_progress_pct: None,
            transferred_bytes: None,
            queued_bytes: None,
            committed_bytes: None,
            written_bytes: None,
            total_bytes: None,
            bytes_per_second: None,
            backend_bytes_per_second: None,
            usb_speed,
            usb_theoretical_mbps,
            executable: String::new(),
            executable_version: String::new(),
            arguments: Vec::new(),
            output_tail: Vec::new(),
            verify_state: "pending".to_owned(),
            verify_method: None,
            storage_type: None,
            physical_verify_state: None,
            physical_image_sha256: None,
            write_state: None,
            outside_image_state: None,
            boot_verify_state: None,
            loader_capabilities: None,
            raw_artifacts: None,
            verify_error_code: None,
            post_flash_action: post_flash_action.to_owned(),
            post_flash_state: if post_flash_action == "none" {
                "skipped".to_owned()
            } else {
                "pending".to_owned()
            },
            post_flash_attempts: 0,
            post_flash_error: None,
            diagnostic_error_code: None,
            last_persisted: Instant::now(),
            last_persisted_progress: None,
            last_persisted_phase: String::new(),
            urgent: true,
        }
    }
}

fn argument_value<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].as_str())
}

fn stage_phase(stage: &str) -> &'static str {
    match stage {
        "DRAM Init" | "U-Boot Download" => "raw_bootstrap_vendor_fes",
        "Reconnecting" => "reconnecting",
        "RK Loader Reconnecting" => "rk_reconnecting",
        "Downloading Loader" => "rk_loader",
        "Checking Raw eMMC" => "rk_raw_preflight",
        "Query Device" => "query_device",
        "Erasing" => "erase",
        "Writing MBR" => "mbr_write",
        "Flashing Partitions" | "Writing Boot" => "write",
        "Setting Mode" => "post_flash_action",
        _ => "preflight",
    }
}

fn mark_telemetry_process_failure(telemetry: &mut FlashTelemetry) {
    if telemetry.physical_verify_state.as_deref() == Some("running") {
        telemetry.physical_verify_state = Some("failed".to_owned());
    }
    if telemetry.verify_state == "pending" || telemetry.verify_state == "running" {
        telemetry.verify_state = "not_run".to_owned();
        telemetry.verify_error_code = None;
    }
    if telemetry.post_flash_state == "pending" || telemetry.post_flash_state == "running" {
        telemetry.post_flash_state = "not_run".to_owned();
    }
    telemetry.urgent = true;
}

fn observe_openix_log_message(message: &str, telemetry: &mut FlashTelemetry) -> bool {
    let plain = message
        .trim()
        .strip_prefix("[INFO] ")
        .unwrap_or(message.trim());
    if telemetry.is_spinor() {
        if plain.starts_with("RAW_WRITE_START") {
            telemetry.write_state = Some("running".into());
        }
        if plain.contains("RAW_SPINOR_WRITE_FAILED:") {
            telemetry.write_state = Some("failed".into());
        }
        if plain == "RAW_WRITE_COMPLETE:medium=spinor" {
            telemetry.write_state = Some("success".into());
            telemetry.phase = "spinor_write_complete".to_owned();
            telemetry.urgent = true;
            return true;
        }
        if plain == "RAW_SPINOR_PHYSICAL_VERIFY_START" {
            telemetry.phase = "spinor_physical_verify".to_owned();
            telemetry.physical_verify_state = Some("running".to_owned());
            telemetry.verify_state = "running".to_owned();
            telemetry.urgent = true;
            return true;
        }
        if message.contains("RAW_SPINOR_PHYSICAL_VERIFY_FAILED:") {
            telemetry.phase = "spinor_physical_verify".to_owned();
            telemetry.physical_verify_state = Some("failed".to_owned());
            telemetry.verify_state = "failed".to_owned();
            telemetry.diagnostic_error_code = Some("RAW_SPINOR_PHYSICAL_VERIFY_FAILED".to_owned());
            mark_telemetry_process_failure(telemetry);
            return true;
        }
        if let Some(fields) = plain.strip_prefix("RAW_SPINOR_PHYSICAL_VERIFY_SUCCESS:") {
            let field = |key: &str| {
                fields.split(',').find_map(|part| {
                    let (name, value) = part.split_once('=')?;
                    (name == key).then_some(value)
                })
            };
            let image_bytes = field("imageBytes").and_then(|v| v.parse::<u64>().ok());
            let full_bytes = field("fullFlashBytes").and_then(|v| v.parse::<u64>().ok());
            let valid_hash = field("imageSha256").is_some_and(|hash| {
                hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())
            });
            if let (Some(image), Some(full)) = (image_bytes, full_bytes) {
                if valid_hash
                    && image > 0
                    && image % 512 == 0
                    && full >= image
                    && field("outsideImage") == Some("unchanged")
                {
                    telemetry.physical_verify_state = Some("success".to_owned());
                    telemetry.physical_image_sha256 = field("imageSha256").map(str::to_owned);
                    telemetry.outside_image_state = Some("unchanged".to_owned());
                    telemetry.verify_method = Some("physical-readback-sha256".to_owned());
                    telemetry.verify_state = "success".to_owned();
                    telemetry.committed_bytes = Some(image);
                    telemetry.written_bytes = Some(image);
                    telemetry.total_bytes = Some(image);
                    telemetry.partition_progress_pct = Some(100.0);
                    telemetry.phase = "spinor_physical_verify".to_owned();
                    telemetry.urgent = true;
                    return true;
                }
            }
        }
    }
    const SPINOR_LAYOUT_UNVERIFIED: &str = "RAW_SPINOR_PHYSICAL_LAYOUT_UNVERIFIED";
    if message.contains(SPINOR_LAYOUT_UNVERIFIED) {
        // The refusal explains that erase/write did not run. Consume this
        // diagnostic before the legacy free-text phase keyword inference.
        telemetry.phase = "spinor_preflight_blocked".to_owned();
        telemetry.diagnostic_error_code = Some(SPINOR_LAYOUT_UNVERIFIED.to_owned());
        mark_telemetry_process_failure(telemetry);
        return true;
    }
    if message.trim() == "RAW_TARGET_STORAGE:SPI NOR" {
        telemetry.mark_spinor();
        telemetry.urgent = true;
        return true;
    }
    if let Some(stage) = message.split("RAW_PREFLIGHT_STAGE:").nth(1) {
        telemetry.phase = if telemetry.is_spinor() && stage.trim() == "probe_capacity" {
            "spinor_probe_capacity".to_owned()
        } else {
            format!("raw_{}", stage.trim())
        };
        telemetry.urgent = true;
        return true;
    }
    if message.contains("RAW_ERASE_START") {
        telemetry.phase = "erase".to_owned();
        telemetry.urgent = true;
        return true;
    }
    if message.contains("RAW_MBR_DOWNLOAD_START") {
        telemetry.phase = if telemetry.is_spinor() {
            "spinor_mbr_write"
        } else {
            "raw_mbr_write"
        }
        .to_owned();
        telemetry.current_partition = Some("MBR".to_owned());
        telemetry.partition_progress_pct = Some(0.0);
        telemetry.urgent = true;
        return true;
    }
    if message.contains("RAW_MBR_DOWNLOAD_READY") {
        telemetry.phase = "raw_mbr_verify".to_owned();
        telemetry.partition_progress_pct = Some(100.0);
        telemetry.urgent = true;
        return true;
    }
    if message.contains("RAW_WRITE_START") {
        telemetry.phase = "write".to_owned();
        telemetry.current_partition = Some(
            if telemetry.is_spinor() {
                "SPI NOR"
            } else {
                "eMMC user area"
            }
            .to_owned(),
        );
        telemetry.partition_progress_pct = Some(0.0);
        telemetry.urgent = true;
        return true;
    }
    if message.contains("EMMC_BOOT0_ARTIFACT:") {
        telemetry.phase = "emmc_boot0_preflight".to_owned();
        telemetry.urgent = true;
        return true;
    }
    if message.contains("EMMC_BOOT0_WRITE_START") {
        telemetry.phase = "emmc_boot0_write".to_owned();
        telemetry.current_partition = Some("eMMC Boot0".to_owned());
        telemetry.partition_progress_pct = Some(0.0);
        // Boot0 is installed before the raw user-area write. Its FES status
        // only confirms component acceptance; the later whole-image verify is
        // the first point where the complete eMMC task can become successful.
        telemetry.verify_state = "running".to_owned();
        telemetry.verify_error_code = None;
        telemetry.urgent = true;
        return true;
    }
    if message.contains("EMMC_BOOT0_VERIFIED:") {
        telemetry.phase = "emmc_boot0_verify".to_owned();
        telemetry.partition_progress_pct = Some(100.0);
        telemetry.verify_state = "running".to_owned();
        telemetry.verify_error_code = None;
        telemetry.urgent = true;
        return true;
    }
    false
}

const ROCKCHIP_OPEN_ERROR_CODES: [&str; 5] = [
    "ROCKCHIP_USB_ACCESS_DENIED",
    "ROCKCHIP_USB_DEVICE_BUSY",
    "ROCKCHIP_USB_DRIVER_UNSUPPORTED",
    "ROCKCHIP_USB_DEVICE_DISCONNECTED",
    "ROCKCHIP_USB_OPEN_FAILED",
];

fn rockchip_open_error_code(message: &str) -> Option<&'static str> {
    for code in ROCKCHIP_OPEN_ERROR_CODES {
        if message.contains(code) {
            return Some(code);
        }
    }
    message
        .to_ascii_lowercase()
        .contains("cannot open selected rockchip device")
        .then_some("ROCKCHIP_USB_OPEN_FAILED")
}

fn observe_rockchip_open_error(message: &str, telemetry: &mut FlashTelemetry) -> bool {
    let Some(error_code) = rockchip_open_error_code(message) else {
        return false;
    };
    telemetry.phase = "query_device".to_owned();
    telemetry.diagnostic_error_code = Some(error_code.to_owned());
    telemetry.urgent = true;
    true
}

fn update_flash_telemetry(text: &str, telemetry: &mut FlashTelemetry) -> Option<u64> {
    let mut parsed_progress = None;
    for line in text.lines() {
        if !line.trim().is_empty() {
            telemetry.output_tail.push(line.to_owned());
            if telemetry.output_tail.len() > 80 {
                telemetry
                    .output_tail
                    .drain(..telemetry.output_tail.len() - 80);
            }
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            if observe_rockchip_open_error(line, telemetry) {
                continue;
            }
            if observe_openix_log_message(line, telemetry) {
                continue;
            }
            if let Some(method) = line.trim().strip_prefix("Verification method: ") {
                if matches!(method, "host-readback" | "device-checksum") {
                    telemetry.verify_method = Some(method.to_owned());
                    telemetry.urgent = true;
                }
                continue;
            }
            if line.trim().starts_with("Device-checksum verification:") {
                continue;
            }
            if let Some(capabilities) = line.trim().strip_prefix("Loader capabilities: ") {
                telemetry.loader_capabilities = Some(capabilities.chars().take(64).collect());
                telemetry.urgent = true;
                continue;
            }
            if line.contains("ROCKCHIP_VERIFY_UNAVAILABLE") {
                // This may precede erase, or report an unrepresentable checksum
                // during writing. Preserve the actual phase instead of claiming
                // that a write/verification stage necessarily ran.
                telemetry.verify_state = "failed".to_owned();
                telemetry.diagnostic_error_code = Some("ROCKCHIP_VERIFY_UNAVAILABLE".to_owned());
                telemetry.urgent = true;
                continue;
            }
            if let Some(stage) = line.trim().strip_prefix("Stage: ") {
                telemetry.phase = stage_phase(stage).to_owned();
                if telemetry.phase == "post_flash_action" {
                    telemetry.post_flash_state = "running".to_owned();
                }
                telemetry.urgent = true;
                continue;
            }
            if line.contains("ROCKCHIP_VERIFY_FAILED") {
                telemetry.phase = "verify".to_owned();
                telemetry.verify_state = "failed".to_owned();
                telemetry.verify_error_code = None;
                telemetry.urgent = true;
                continue;
            }
            if let Some(start) = line.find("ROCKCHIP_RAW_") {
                telemetry.diagnostic_error_code = Some(
                    line[start..]
                        .split([':', ' '])
                        .next()
                        .unwrap_or("ROCKCHIP_RAW_ERROR")
                        .to_owned(),
                );
                telemetry.urgent = true;
                continue;
            }
            if (telemetry.phase == "rk_loader" && line.starts_with("Downloading "))
                || line.starts_with("Already in Loader mode:")
            {
                continue;
            }
            let lower = line.to_ascii_lowercase();
            if lower.trim() == "flash failed." || telemetry.verify_state == "failed" {
                continue;
            }
            if lower.contains("verify") {
                telemetry.phase = "verify".to_owned();
            } else if lower.contains("erase") {
                telemetry.phase = "erase".to_owned();
            } else if lower.contains("writ")
                || lower.contains("flash")
                || lower.contains("download")
            {
                telemetry.phase = "write".to_owned();
            }
            continue;
        };
        match value.get("type").and_then(Value::as_str) {
            Some("log") => {
                if let Some(message) = value.get("message").and_then(Value::as_str) {
                    observe_openix_log_message(message, telemetry);
                }
            }
            Some("stage") if value.get("status").and_then(Value::as_str) == Some("running") => {
                if let Some(stage) = value.get("stage").and_then(Value::as_str) {
                    telemetry.phase = stage_phase(stage).to_owned();
                    if telemetry.phase == "post_flash_action" {
                        telemetry.post_flash_state = "running".to_owned();
                    }
                    telemetry.urgent = true;
                }
            }
            Some("partition") => {
                if let Some(name) = value.get("name").and_then(Value::as_str) {
                    if telemetry.current_partition.as_deref() != Some(name) {
                        if let Some(previous) = telemetry.current_partition.replace(name.to_owned())
                        {
                            if !telemetry.completed_partitions.contains(&previous) {
                                telemetry.completed_partitions.push(previous);
                            }
                        }
                        telemetry.partition_progress_pct = Some(0.0);
                        telemetry.phase = "write".to_owned();
                        telemetry.urgent = true;
                    }
                }
            }
            Some("progress") => {
                if let Some(percent) = value.get("percent").and_then(Value::as_f64) {
                    let percent = percent.clamp(0.0, 100.0);
                    telemetry.progress_pct = Some(percent);
                    parsed_progress = Some(percent as u64);
                }
                telemetry.transferred_bytes = value
                    .get("transferredBytes")
                    .or_else(|| value.get("current"))
                    .and_then(Value::as_u64);
                telemetry.queued_bytes = value.get("queuedBytes").and_then(Value::as_u64);
                telemetry.committed_bytes = value.get("committedBytes").and_then(Value::as_u64);
                telemetry.written_bytes = telemetry.committed_bytes;
                telemetry.total_bytes = value.get("total").and_then(Value::as_u64);
                telemetry.backend_bytes_per_second =
                    value.get("bytesPerSecond").and_then(Value::as_f64);
                telemetry.bytes_per_second = clamp_backend_speed(
                    telemetry.backend_bytes_per_second,
                    telemetry.usb_theoretical_mbps,
                );
                telemetry.partition_progress_pct =
                    match (telemetry.transferred_bytes, telemetry.total_bytes) {
                        (Some(current), Some(total)) if total > 0 => {
                            Some((current as f64 / total as f64 * 100.0).clamp(0.0, 100.0))
                        }
                        _ => telemetry.partition_progress_pct,
                    };
            }
            Some("verify") => {
                // OpenixCLI also emits a generic verify failure when MBR
                // transport fails, before any image partitions were written.
                if telemetry.phase == "mbr_write"
                    && value.get("status").and_then(Value::as_str) == Some("failed")
                    && value.get("errorCode").and_then(Value::as_i64) == Some(-1)
                {
                    telemetry.verify_state = "not_run".to_owned();
                    telemetry.verify_error_code = None;
                    telemetry.urgent = true;
                    continue;
                }
                telemetry.phase = "verify".to_owned();
                telemetry.verify_state = value
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("running")
                    .to_owned();
                telemetry.verify_error_code = value.get("errorCode").and_then(Value::as_i64);
                telemetry.urgent = true;
            }
            Some("post_action") => {
                if telemetry.is_spinor() {
                    if let Some(action @ ("none" | "reboot" | "poweroff" | "shutdown")) =
                        value.get("action").and_then(Value::as_str)
                    {
                        telemetry.post_flash_action = action.to_owned();
                    }
                }
                telemetry.phase = "post_flash_action".to_owned();
                telemetry.post_flash_state = value
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("running")
                    .to_owned();
                telemetry.post_flash_attempts = value
                    .get("attempt")
                    .and_then(Value::as_u64)
                    .unwrap_or(u64::from(telemetry.post_flash_attempts))
                    as u32;
                telemetry.post_flash_error = value
                    .get("error")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                telemetry.urgent = true;
            }
            Some("finished") => {
                if telemetry.is_spinor() {
                    // A process-level finish is not evidence of media placement or boot.
                    telemetry.phase = telemetry.spinor_completion_phase().to_owned();
                    telemetry.urgent = true;
                    continue;
                }
                telemetry.phase = "complete".to_owned();
                telemetry.progress_pct = Some(100.0);
                telemetry.committed_bytes = telemetry.total_bytes;
                telemetry.written_bytes = telemetry.committed_bytes;
                if telemetry.verify_state == "running" || telemetry.verify_state == "pending" {
                    telemetry.verify_state = "success".to_owned();
                    telemetry.verify_error_code = Some(0);
                }
                if telemetry.post_flash_state != "skipped" {
                    telemetry.post_flash_state = "success".to_owned();
                }
                telemetry.urgent = true;
                parsed_progress = Some(100);
            }
            _ => {}
        }
    }
    if parsed_progress.is_none() {
        if let Some(percent) = parse_percent(text) {
            telemetry.progress_pct = Some(percent as f64);
            parsed_progress = Some(percent);
        }
    }
    parsed_progress
}

fn clamp_backend_speed(reported: Option<f64>, theoretical_mbps: Option<f64>) -> Option<f64> {
    match (reported, theoretical_mbps) {
        (Some(value), Some(mbps)) => Some(value.min(mbps * 1_000_000.0 / 8.0)),
        (value, None) => value,
        (None, _) => None,
    }
}

fn persist_telemetry_if_due(
    state: Option<&WorkbenchState>,
    task_id: &str,
    telemetry: &mut FlashTelemetry,
) {
    let progress_delta = match (telemetry.progress_pct, telemetry.last_persisted_progress) {
        (Some(current), Some(previous)) => (current - previous).abs(),
        (Some(_), None) => 100.0,
        _ => 0.0,
    };
    if telemetry.urgent
        || telemetry.phase != telemetry.last_persisted_phase
        || progress_delta >= 1.0
        || telemetry.last_persisted.elapsed() >= Duration::from_millis(250)
    {
        persist_telemetry(state, task_id, telemetry);
        telemetry.last_persisted = Instant::now();
        telemetry.last_persisted_progress = telemetry.progress_pct;
        telemetry.last_persisted_phase = telemetry.phase.clone();
        telemetry.urgent = false;
    }
}

fn persist_telemetry(state: Option<&WorkbenchState>, task_id: &str, telemetry: &FlashTelemetry) {
    let Some(state) = state else { return };
    let Ok(details_json) = serde_json::to_string(telemetry) else {
        return;
    };
    if let Ok(db) = state.db.lock() {
        let _ = db.execute(
            "UPDATE task_history SET phase=?1,progress=?2,details_json=?3 WHERE task_id=?4",
            rusqlite::params![
                telemetry.phase,
                telemetry.progress_pct,
                details_json,
                task_id
            ],
        );
    }
}

fn emit_telemetry(app: &AppHandle, task_id: &str, telemetry: &FlashTelemetry) {
    let _ = app.emit(
        "workbench-flash-telemetry",
        serde_json::json!({
            "taskId": task_id,
            "telemetry": telemetry,
        }),
    );
}

pub(crate) fn usb_diagnostics(binding: &str) -> (String, Option<f64>) {
    let normalized = normalize_libusb_binding(binding);
    let Some((bus, port_path)) = normalized.split_once(':') else {
        return ("unknown".to_owned(), None);
    };
    let Ok(bus) = bus.parse::<u8>() else {
        return ("unknown".to_owned(), None);
    };
    // A Windows Rockusb location is not a libusb topology. Do not enumerate
    // libusb devices merely to populate optional speed telemetry for it.
    if !port_path.split('.').all(|part| part.parse::<u8>().is_ok()) {
        return ("unknown".to_owned(), None);
    }
    let Ok(devices) = rusb::devices() else {
        return ("unknown".to_owned(), None);
    };
    let Some(device) = devices.iter().find(|device| {
        if device.bus_number() != bus {
            return false;
        }
        let path = device
            .port_numbers()
            .ok()
            .filter(|items| !items.is_empty())
            .map(|items| {
                items
                    .iter()
                    .map(u8::to_string)
                    .collect::<Vec<_>>()
                    .join(".")
            })
            .unwrap_or_else(|| device.port_number().to_string());
        path == port_path
    }) else {
        return ("unknown".to_owned(), None);
    };
    match device.speed() {
        rusb::Speed::Low => ("low".to_owned(), Some(1.5)),
        rusb::Speed::Full => ("full".to_owned(), Some(12.0)),
        rusb::Speed::High => ("high".to_owned(), Some(480.0)),
        rusb::Speed::Super => ("super".to_owned(), Some(5_000.0)),
        rusb::Speed::SuperPlus => ("super_plus".to_owned(), Some(10_000.0)),
        _ => ("unknown".to_owned(), None),
    }
}

fn normalize_libusb_binding(binding: &str) -> &str {
    binding
        .trim()
        .strip_prefix("location:")
        .unwrap_or(binding.trim())
        .strip_prefix("libusb:")
        .unwrap_or_else(|| {
            binding
                .trim()
                .strip_prefix("location:")
                .unwrap_or(binding.trim())
        })
}

#[cfg(test)]
mod scan_tests {
    use super::{
        normalize_libusb_binding, parse_dfu_util_scan, parse_openrockcli_scan, parse_uuu_scan,
        take_stream_records,
    };

    #[test]
    fn flasher_exit_drains_late_reader_output() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(30));
            sender
                .send(("stderr", b"ROCKCHIP_VERIFY_FAILED final".to_vec()))
                .unwrap();
        });
        let mut records = Vec::new();
        super::drain_flasher_output(
            &receiver,
            std::time::Duration::from_secs(1),
            |stream, bytes| records.push((stream, bytes)),
        )
        .unwrap();
        thread.join().unwrap();
        assert_eq!(
            records,
            vec![("stderr", b"ROCKCHIP_VERIFY_FAILED final".to_vec())]
        );
    }

    #[test]
    fn flasher_exit_drain_reports_a_pipe_kept_open() {
        let (_sender, receiver) = std::sync::mpsc::channel();
        assert!(super::drain_flasher_output(
            &receiver,
            std::time::Duration::from_millis(20),
            |_, _| {}
        )
        .unwrap_err()
        .contains("FLASHER_OUTPUT_DRAIN_TIMEOUT"));
    }

    #[test]
    fn normalizes_saved_allwinner_physical_binding_for_usb_diagnostics() {
        assert_eq!(normalize_libusb_binding("location:libusb:3:3.1"), "3:3.1");
        assert_eq!(normalize_libusb_binding("libusb:3:3.1"), "3:3.1");
        assert_eq!(normalize_libusb_binding("3:3.1"), "3:3.1");
    }

    #[test]
    fn splits_live_progress_on_carriage_return_and_newline() {
        let mut pending = Vec::new();
        let first = take_stream_records(&mut pending, b"Writing 1%\rWriting 2");
        assert_eq!(first, [b"Writing 1%".to_vec()]);
        assert_eq!(pending, b"Writing 2");

        let second = take_stream_records(&mut pending, b"%\r\nDone\n");
        assert_eq!(second, [b"Writing 2%".to_vec(), b"Done".to_vec()]);
        assert!(pending.is_empty());
    }

    #[test]
    fn parses_openrockcli_scan_table() {
        let devices = parse_openrockcli_scan(
            "Scanning USB devices...\r\n\r\n\
             Bus  Port  VID:PID    Mode      Chip\r\n\
             ---  ----  ---------  --------  --------\r\n\
             001  2.3   2207:350b  maskrom   RK3568\r\n\
             002  4     2207:350a  loader    RK3566\r\n",
        )
        .expect("parse scan output");
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].key, "1:2.3");
        assert_eq!(devices[0].flasher_kind, "openrockcli");
        assert_eq!(devices[0].vid, 0x2207);
        assert_eq!(devices[0].mode, "maskrom");
        assert_eq!(devices[1].chip, "RK3566");
    }

    #[test]
    fn parses_uuu_known_usb_table() {
        let devices = parse_uuu_scan(
            "uuu (Universal Update Utility) for nxp imx chips -- libuuu_1.5.243\n\
             Connected Known USB Devices\n\
             \tPath\t Chip\t Pro\t Vid\t Pid\t BcdVersion\t Serial_no\n\
             \t====================================================================\n\
             \t1:2\t MX6ULL\t SDP:\t 0x15A2\t0x0080\t 0x0001\t board-a\n\
             \t3:24\t MX8MM\t SDPV:\t 0x1FC9\t0x0134\t 0x0500\t board-b\n",
        )
        .expect("parse UUU scan output");
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].key, "1:2");
        assert_eq!(devices[0].flasher_kind, "uuu");
        assert_eq!(devices[0].chip, "MX6ULL");
        assert_eq!(devices[0].mode, "sdp");
        assert_eq!(devices[0].vid, 0x15a2);
        assert_eq!(devices[0].pid, 0x0080);
        assert_eq!(devices[1].key, "3:24");
        assert_eq!(devices[1].mode, "sdpv");
    }

    #[test]
    fn parses_and_deduplicates_stm32_dfu_alternates() {
        let devices = parse_dfu_util_scan(
            "Found DFU: [0483:df11] ver=0200, devnum=6, cfg=1, intf=0, path=\"1-5.2\", alt=1, name=\"@virtual\", serial=\"ABC\"\n\
             Found DFU: [0483:df11] ver=0200, devnum=6, cfg=1, intf=0, path=\"1-5.2\", alt=0, name=\"@FSBL\", serial=\"ABC\"\n\
             Found DFU: [1234:5678] ver=0100, devnum=7, cfg=1, intf=0, path=\"2-3\", alt=0, name=\"other\", serial=\"XYZ\"\n",
        );
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].key, "1:5.2");
        assert_eq!(devices[0].flasher_kind, "dfu-util");
        assert_eq!(devices[0].vid, 0x0483);
        assert_eq!(devices[0].pid, 0xdf11);
        assert_eq!(devices[0].mode, "dfu");
    }
}

fn emit(
    app: &AppHandle,
    task_id: &str,
    stream: &str,
    message: String,
    progress: Option<u64>,
    status: &str,
) {
    let _ = app.emit(
        "workbench-task-event",
        TaskEvent {
            task_id: task_id.to_owned(),
            phase: "flash".to_owned(),
            stream: stream.to_owned(),
            message,
            current: progress,
            total: progress.map(|_| 100),
            status: status.to_owned(),
        },
    );
}

fn insert_task(state: &WorkbenchState, task_id: &str, device_id: i64) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.execute(
        "INSERT INTO task_history(task_id,kind,device_id,status,started_at,phase,progress)\
         VALUES(?1,'flash',?2,'running',unixepoch(),'preflight',0)",
        rusqlite::params![task_id, device_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn finish_task(
    state: &WorkbenchState,
    task_id: &str,
    status: &str,
    exit_code: Option<i32>,
    error: &str,
) {
    if let Ok(db) = state.db.lock() {
        let _ = super::storage::transition_task_status(&db, task_id, status, exit_code, error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_rockchip_windows_location_roundtrips_without_truncation() {
        let port = "PCIROOT(0)#PCI(1400)#USBROOT(0)#USB(3)#USB(2)";
        let output = format!("000  {port}  2207:350e  maskrom  RK3576\r\n");
        let devices = parse_openrockcli_scan(&output).unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].key, format!("0:{port}"));
        let args = build_flash_args(
            "openrockcli",
            &devices[0].key,
            "C:\\固件 目录\\update.img",
            vec!["--post-action".into(), "none".into()],
            false,
        );
        assert_eq!(
            args,
            vec![
                "flash",
                "C:\\固件 目录\\update.img",
                "--post-action",
                "none",
                "--bus",
                "0",
                "--port",
                port
            ]
        );
    }

    #[test]
    fn native_rockchip_stages_and_verification_failure_remain_visible() {
        let mut telemetry = FlashTelemetry::new("0:PCIROOT(0)#USB(2)", "none");
        update_flash_telemetry("Stage: Erasing\n", &mut telemetry);
        assert_eq!(telemetry.phase, "erase");
        assert_eq!(
            update_flash_telemetry(
                "Stage: Flashing Partitions\nFlashing: [####] 37%\r",
                &mut telemetry
            ),
            Some(37)
        );
        assert_eq!(telemetry.phase, "write");
        let error =
            "Error: ROCKCHIP_VERIFY_FAILED at LBA 0x10000; verification cannot be downgraded";
        update_flash_telemetry(error, &mut telemetry);
        update_flash_telemetry(
            "Error: write or verification failed for partition boot\nFlash failed.\n",
            &mut telemetry,
        );
        mark_telemetry_process_failure(&mut telemetry);
        assert_eq!(telemetry.phase, "verify");
        assert_eq!(telemetry.verify_state, "failed");
        assert_eq!(telemetry.post_flash_state, "skipped");
        assert!(telemetry.output_tail.iter().any(|line| line == error));
    }

    #[test]
    fn native_rockchip_verification_method_and_preflight_failure_are_explicit() {
        let mut telemetry = FlashTelemetry::new("", "none");
        update_flash_telemetry(
            "Loader capabilities: 05 04 00 00 00 00 00 00\nVerification method: device-checksum\n",
            &mut telemetry,
        );
        assert_eq!(
            serde_json::to_value(&telemetry).unwrap()["verifyMethod"],
            "device-checksum"
        );
        assert_eq!(telemetry.verify_state, "pending");
        update_flash_telemetry("Device-checksum verification: Loader reads media after each write and checks the Rockchip 32-bit additive checksum; not a host byte-for-byte comparison.\n", &mut telemetry);
        assert_eq!(telemetry.phase, "preflight");
        for _ in 0..100 {
            update_flash_telemetry("progress log\n", &mut telemetry);
        }
        assert_eq!(
            serde_json::to_value(&telemetry).unwrap()["verifyMethod"],
            "device-checksum"
        );
        update_flash_telemetry("Stage: Checking Readback\nError: ROCKCHIP_VERIFY_UNAVAILABLE before erase: read-failed\nFlash failed.\n", &mut telemetry);
        mark_telemetry_process_failure(&mut telemetry);
        assert_eq!(telemetry.phase, "preflight");
        assert_eq!(telemetry.verify_state, "failed");
        assert_eq!(
            telemetry.diagnostic_error_code.as_deref(),
            Some("ROCKCHIP_VERIFY_UNAVAILABLE")
        );
        assert_eq!(telemetry.post_flash_state, "skipped");
    }

    #[test]
    fn native_rockchip_capabilities_allow_no_reset() {
        let board = device("rockchip", "openrockcli");
        let capabilities = flash_capabilities(&board).unwrap();
        assert_eq!(
            capabilities.post_flash_actions.contains(&"none"),
            cfg!(windows)
        );
    }

    #[test]
    fn failed_reconnect_preserves_stage_and_marks_unreached_steps_not_run() {
        let mut telemetry = FlashTelemetry::new("", "reboot");
        update_flash_telemetry(
            "{\"stage\":\"Reconnecting\",\"status\":\"running\",\"type\":\"stage\"}\n",
            &mut telemetry,
        );
        assert_eq!(telemetry.phase, "reconnecting");
        mark_telemetry_process_failure(&mut telemetry);
        assert_eq!(telemetry.phase, "reconnecting");
        assert_eq!(telemetry.verify_state, "not_run");
        assert_eq!(telemetry.post_flash_state, "not_run");
    }

    fn device(vendor: &str, flasher: &str) -> DeviceConfig {
        DeviceConfig {
            id: Some(1),
            board_uid: "linux-00000000000000000000000000000001".to_owned(),
            name: "test board".to_owned(),
            board_image_path: String::new(),
            chip_vendor: vendor.to_owned(),
            serial_port: String::new(),
            serial_vid: 0,
            serial_pid: 0,
            serial_number: String::new(),
            usb_binding: "1:2".to_owned(),
            camera_binding: String::new(),
            adb_serial: String::new(),
            serial_interface_number: None,
            baud_rate: 115_200,
            serial_console_type: "auto".to_owned(),
            serial_login_mode: "none".to_owned(),
            serial_login_username: "root".to_owned(),
            serial_login_prompt: "login:".to_owned(),
            serial_password_prompt: "Password:".to_owned(),
            serial_shell_prompt: "#".to_owned(),
            serial_auto_login: false,
            flasher_kind: flasher.to_owned(),
            flasher_path: String::new(),
            flasher_args: String::new(),
            power_controller: String::new(),
            power_controller_vid: 0,
            power_controller_pid: 0,
            power_controller_serial: String::new(),
            power_controller_interface_number: None,
            power_baud_rate: 115_200,
            power_profile_json: String::new(),
            power_channel: String::new(),
            boot_channel: String::new(),
            reset_channel: String::new(),
            power_off_wait_ms: 1_000,
            power_on_wait_ms: 2_000,
            serial_reconnect_timeout_ms: 30_000,
            boot_timeout_ms: 60_000,
        }
    }

    #[test]
    fn vendor_backends_advertise_distinct_capabilities() {
        let allwinner = flash_capabilities(&device("allwinner", "openixcli")).unwrap();
        assert!(allwinner.supports_verify);
        assert_eq!(allwinner.default_mode, Some("full_erase"));
        assert_eq!(
            allwinner.modes,
            vec!["command", "partition", "partition_erase", "full_erase"]
        );
        assert!(allwinner.post_flash_actions.contains(&"none"));
        assert!(allwinner.audit_required);
        assert!(allwinner
            .artifact_formats
            .contains(&"mainline_raw_disk_image"));
        assert!(allwinner
            .artifact_formats
            .contains(&"mainline_nand_component_package"));
        assert!(!allwinner.artifact_formats.contains(&"raw_disk_image"));
        assert!(allwinner.vendor_options.get("raw").is_some());
        assert!(allwinner.vendor_options.get("loaderPath").is_some());
        assert!(allwinner.mainline_workflow.contains("NAND"));
        assert!(allwinner.mainline_workflow.contains("eMMC"));
        assert!(!allwinner.safety_policy.software_boot_transition_allowed);

        let rockchip = flash_capabilities(&device("rockchip", "openrockcli")).unwrap();
        assert!(rockchip.audit_required);
        assert!(rockchip.artifact_formats.contains(&"rockchip_update_image"));
        assert!(rockchip.artifact_formats.contains(&"rockchip_raw_emmc"));

        let nxp = flash_capabilities(&device("nxp", "uuu")).unwrap();
        assert!(nxp.audit_required);
        assert!(nxp.artifact_formats.contains(&"uuu_script"));

        let k230 = flash_capabilities(&device("kendryte", "k230")).unwrap();
        assert!(!k230.supports_verify);
        assert!(k230.audit_required);
        assert!(k230.artifact_formats.contains(&"raw_image"));
        assert!(k230.vendor_options.get("medium").is_some());
        assert!(k230
            .safety_policy
            .erase_strategy
            .contains("erases each image target"));

        let stm32 = flash_capabilities(&device("st", "dfu-util")).unwrap();
        assert!(!stm32.supports_verify);
        assert!(stm32.audit_required);
        assert!(stm32.artifact_formats.contains(&"raw_binary"));
        assert!(stm32.post_flash_actions.contains(&"reset"));
        assert!(stm32.vendor_options.get("alt").is_some());
        assert!(flash_capabilities(&device("rockchip", "openixcli")).is_err());
    }

    #[test]
    fn allwinner_and_rockchip_default_to_full_erase() {
        let rockchip = adapter_override_args(
            &device("rockchip", "openrockcli"),
            AdapterFlashOptions::default(),
        )
        .unwrap()
        .unwrap();
        assert!(rockchip
            .windows(2)
            .any(|pair| pair == ["--mode", "full_erase"]));

        let allwinner = adapter_override_args(
            &device("allwinner", "openixcli"),
            AdapterFlashOptions::default(),
        )
        .unwrap()
        .unwrap();
        assert!(allwinner
            .windows(2)
            .any(|pair| pair == ["--mode", "full_erase"]));
    }

    #[test]
    fn spinor_local_route_discards_emmc_flags_and_preserves_requested_action() {
        let loader =
            std::env::temp_dir().join(format!("lynx-spinor-route-{}.img", std::process::id()));
        std::fs::write(&loader, b"IMAGEWTY test-only RAM loader fixture").unwrap();
        let mut board = device("allwinner", "openixcli");
        board.flasher_args =
            r#"["--raw","--lynx-raw-loader","stale","--emmc-boot0-from-image"]"#.to_owned();
        for action in ["none", "reboot", "poweroff"] {
            let args = local_flash_args(
                &board,
                serde_json::from_value(json!({
                    "systemType":"mainline", "mainlineRoute":"mainline_spinor",
                    "loaderPath":loader.to_str().unwrap(), "postFlashAction":action
                }))
                .unwrap(),
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                args,
                [
                    "--raw",
                    "--lynx-raw-loader",
                    loader.to_str().unwrap(),
                    "--lynx-spinor",
                    "--post-action",
                    action
                ]
            );
        }
        let invalid = local_flash_args(
            &board,
            serde_json::from_value(json!({
                "systemType":"mainline", "mainlineRoute":"mainline_spinor",
                "loaderPath":loader.to_str().unwrap(), "postFlashAction":"invalid"
            }))
            .unwrap(),
        )
        .unwrap_err();
        assert!(invalid.contains("postFlashAction=invalid"));
        std::fs::remove_file(loader).unwrap();
    }

    #[test]
    fn switching_from_spinor_restores_vendor_nand_and_emmc_routes() {
        let loader =
            std::env::temp_dir().join(format!("lynx-route-isolation-{}.img", std::process::id()));
        std::fs::write(&loader, b"test-only RAM loader fixture").unwrap();
        let path = loader.to_str().unwrap();
        let mut board = device("allwinner", "openixcli");
        board.flasher_args = serde_json::to_string(&[
            "--raw",
            "--lynx-raw-loader",
            path,
            "--lynx-spinor",
            "--post-action",
            "poweroff",
        ])
        .unwrap();
        for (route, expected) in [
            (
                "mainline_nand",
                vec![
                    "--lynx-loader",
                    path,
                    "--verify",
                    "true",
                    "--mode",
                    "full_erase",
                    "--post-action",
                    "none",
                ],
            ),
            (
                "mainline_emmc",
                vec![
                    "--raw",
                    "--lynx-raw-loader",
                    path,
                    "--emmc-boot0-from-image",
                ],
            ),
        ] {
            let args = local_flash_args(
                &board,
                serde_json::from_value(json!({
                    "systemType": "mainline", "mainlineRoute": route, "loaderPath": path
                }))
                .unwrap(),
            )
            .unwrap()
            .unwrap();
            assert_eq!(args, expected, "route={route}");
        }
        for mode in ["full_erase", "partition", "partition_erase"] {
            for verify in [false, true] {
                for action in ["none", "reboot", "poweroff"] {
                    let args = local_flash_args(
                        &board,
                        serde_json::from_value(json!({
                            "systemType": "vendor", "mode": mode,
                            "verify": verify, "postFlashAction": action
                        }))
                        .unwrap(),
                    )
                    .unwrap()
                    .unwrap();
                    assert_eq!(
                        args,
                        vec![
                            "--verify",
                            if verify { "true" } else { "false" },
                            "--mode",
                            mode,
                            "--post-action",
                            action
                        ]
                    );
                }
            }
        }
        std::fs::remove_file(loader).unwrap();
    }

    #[test]
    fn allwinner_raw_disk_image_is_explicit_and_translated() {
        let bootstrap = std::env::temp_dir().join(format!(
            "lynx-openix-bootstrap-{:032x}.img",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&bootstrap, b"IMAGEWTY bootstrap fixture").unwrap();
        let bootstrap_text = bootstrap.to_string_lossy().into_owned();
        let args = adapter_override_args(
            &device("allwinner", "openixcli"),
            AdapterFlashOptions {
                post_flash_action: Some("none".to_owned()),
                vendor_options: Some(json!({
                    "raw": true,
                    "loaderPath": bootstrap_text,
                })),
                mode: Some("command".to_owned()),
                ..Default::default()
            },
        )
        .unwrap()
        .unwrap();
        assert!(args.iter().any(|arg| arg == "--raw"));
        assert!(args.windows(2).any(|pair| pair == ["--verify", "false"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--post-action", "none"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--lynx-loader", bootstrap_text.as_str()]));
        std::fs::remove_file(bootstrap).unwrap();
    }

    #[test]
    fn allwinner_bootstrap_supports_nand_components_and_requires_existing_file() {
        let missing = adapter_override_args(
            &device("allwinner", "openixcli"),
            AdapterFlashOptions {
                vendor_options: Some(json!({
                    "raw": true,
                    "loaderPath": "/definitely/missing/loader.img",
                })),
                mode: Some("command".to_owned()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(missing.contains("BOOTSTRAP_FIRMWARE_NOT_FOUND"));

        let component_loader = std::env::temp_dir().join(format!(
            "lynx-openix-component-bootstrap-{:032x}.img",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&component_loader, b"IMAGEWTY component bootstrap fixture").unwrap();
        let component_loader_text = component_loader.to_string_lossy().into_owned();
        let without_raw = adapter_override_args(
            &device("allwinner", "openixcli"),
            AdapterFlashOptions {
                vendor_options: Some(json!({
                    "loaderPath": component_loader_text,
                })),
                mode: Some("partition".to_owned()),
                ..Default::default()
            },
        )
        .unwrap()
        .unwrap();
        assert!(without_raw
            .windows(2)
            .any(|pair| pair == ["--lynx-loader", component_loader_text.as_str()]));
        std::fs::remove_file(component_loader).unwrap();

        let without_bootstrap = adapter_override_args(
            &device("allwinner", "openixcli"),
            AdapterFlashOptions {
                vendor_options: Some(json!({"raw": true})),
                mode: Some("command".to_owned()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(without_bootstrap.contains("raw requires loaderPath"));
    }

    #[test]
    fn mcp_adapter_allows_allwinner_components_but_keeps_rockchip_policy() {
        assert!(adapter_override_args(
            &device("allwinner", "openixcli"),
            AdapterFlashOptions {
                mode: Some("partition".to_owned()),
                ..Default::default()
            },
        )
        .is_ok());
        assert!(adapter_override_args(
            &device("rockchip", "openrockcli"),
            AdapterFlashOptions {
                mode: Some("partition".to_owned()),
                ..Default::default()
            },
        )
        .unwrap_err()
        .starts_with("FLASH_OPTION_UNSUPPORTED:mode=partition"));
    }

    #[test]
    fn rockchip_raw_local_and_mcp_are_identical_and_reject_bypass() {
        let loader =
            std::env::temp_dir().join(format!("rk-loader-{:016x}.bin", rand::random::<u64>()));
        std::fs::write(&loader, b"loader fixture").unwrap();
        let board = device("rockchip", "openrockcli");
        let vendor = json!({"raw":true,"loaderPath":loader.to_str().unwrap(),"medium":"emmc"});
        let mcp = adapter_override_args(
            &board,
            AdapterFlashOptions {
                vendor_options: Some(vendor.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        let local=local_flash_args(&board,serde_json::from_value(json!({"systemType":"mainline","loaderPath":loader.to_str().unwrap(),"mainlineRoute":"rockchip_raw_emmc","verify":true})).unwrap()).unwrap();
        assert_eq!(mcp, local);
        let args = build_flash_args(
            "openrockcli",
            "0:PCIROOT(0)#USBROOT(0)#USB(1)",
            "中文 image.img",
            mcp.unwrap(),
            false,
        );
        assert_eq!(&args[..2], ["flash", "中文 image.img"]);
        assert!(args
            .windows(2)
            .any(|v| v == ["--port", "PCIROOT(0)#USBROOT(0)#USB(1)"]));
        assert!(args
            .windows(2)
            .any(|v| v == ["--loader", loader.to_str().unwrap()]));
        for bad in [
            json!({"raw":true,"medium":"emmc"}),
            json!({"raw":true,"loaderPath":loader,"medium":"sd"}),
            json!({"raw":true,"loaderPath":loader,"medium":"emmc","offset":64}),
            json!({"raw":"true","loaderPath":loader,"medium":"emmc"}),
        ] {
            assert!(adapter_override_args(
                &board,
                AdapterFlashOptions {
                    vendor_options: Some(bad),
                    ..Default::default()
                }
            )
            .is_err());
        }
        assert!(adapter_override_args(
            &board,
            AdapterFlashOptions {
                verify: Some(false),
                vendor_options: Some(vendor.clone()),
                ..Default::default()
            }
        )
        .is_err());
        assert!(adapter_override_args(
            &device("allwinner", "openixcli"),
            AdapterFlashOptions {
                vendor_options: Some(vendor),
                ..Default::default()
            }
        )
        .is_err());
        std::fs::remove_file(loader).unwrap();
    }

    #[test]
    fn rk_loader_telemetry_does_not_report_fes_or_success_at_one_hundred() {
        let mut t = FlashTelemetry::new("invalid", "reboot");
        update_flash_telemetry("Stage: Downloading Loader\n", &mut t);
        assert_eq!(t.phase, "rk_loader");
        update_flash_telemetry("Downloading Download\n", &mut t);
        assert_eq!(t.phase, "rk_loader");
        update_flash_telemetry("Stage: Checking Raw eMMC\nError: ROCKCHIP_RAW_STORAGE: not eMMC; no erase or write issued\n",&mut t);
        assert_eq!(t.phase, "rk_raw_preflight");
        assert_eq!(
            t.diagnostic_error_code.as_deref(),
            Some("ROCKCHIP_RAW_STORAGE")
        );

        update_flash_telemetry("Stage: RK Loader Reconnecting\n", &mut t);
        assert_eq!(t.phase, "rk_reconnecting");
        update_flash_telemetry("Flashing: [####] 100%\n", &mut t);
        assert_ne!(t.verify_state, "success");
        t.raw_artifacts = Some(json!({"imageSha256":"test"}));
        assert_eq!(
            serde_json::to_value(&t).unwrap()["rawArtifacts"]["imageSha256"],
            "test"
        );
    }

    #[test]
    fn rockchip_advertises_only_controlled_loader_transition() {
        let policy = flash_safety_policy("rockchip").unwrap();
        assert!(policy.software_boot_transition_allowed);
        assert!(policy.boot_entry.contains("lynx_rockchip_enter_loader"));
        assert!(policy.boot_entry.contains("physical"));
    }

    #[test]
    fn vendors_without_controlled_loader_recipe_keep_physical_boot_policy() {
        for vendor in ["allwinner", "nxp", "kendryte", "st"] {
            let policy = flash_safety_policy(vendor).unwrap();
            assert!(!policy.software_boot_transition_allowed);
            assert!(policy.boot_entry.contains("physical"));
        }
    }

    #[test]
    fn builds_stm32_dfu_arguments_with_bound_usb_path() {
        let args = build_flash_args(
            "dfu-util",
            "1:5.2",
            "/tmp/rootfs.bin",
            vec![
                "-a".to_owned(),
                "rootfs".to_owned(),
                "-s".to_owned(),
                "0x08000000:leave".to_owned(),
                "-R".to_owned(),
            ],
            false,
        );
        assert_eq!(
            args,
            [
                "-p",
                "1-5.2",
                "-a",
                "rootfs",
                "-s",
                "0x08000000:leave",
                "-R",
                "-D",
                "/tmp/rootfs.bin"
            ]
        );
    }

    #[test]
    fn stm32_dfu_rejects_target_and_upload_overrides() {
        for args in [
            vec!["-p".to_owned(), "2-1".to_owned()],
            vec!["-D".to_owned(), "/tmp/other.bin".to_owned()],
            vec!["-U".to_owned(), "/tmp/readback.bin".to_owned()],
        ] {
            assert!(validate_dfu_args(&args)
                .unwrap_err()
                .starts_with("STM32_DFU_ARGS:"));
        }
    }

    #[test]
    fn nxp_rejects_allwinner_style_options() {
        let result = adapter_override_args(
            &device("nxp", "uuu"),
            AdapterFlashOptions {
                mode: Some("partition".to_owned()),
                ..Default::default()
            },
        );
        assert!(result.unwrap_err().starts_with("FLASH_OPTION_UNSUPPORTED"));
    }

    #[test]
    fn k230_validates_and_translates_vendor_options() {
        let args = adapter_override_args(
            &device("kendryte", "k230"),
            AdapterFlashOptions {
                post_flash_action: Some("none".to_owned()),
                vendor_options: Some(json!({"medium": "spi_nor"})),
                ..Default::default()
            },
        )
        .unwrap()
        .unwrap();
        assert!(args.windows(2).any(|pair| pair == ["--medium", "spi_nor"]));
        assert!(args.iter().any(|arg| arg == "--no-reboot"));
    }

    #[test]
    fn parses_structured_flash_telemetry() {
        let mut telemetry = FlashTelemetry::new("invalid", "reboot");
        update_flash_telemetry(
            "{\"type\":\"stage\",\"status\":\"running\",\"stage\":\"Flashing Partitions\"}\n\
             {\"type\":\"partition\",\"name\":\"rootfs\"}\n\
             {\"type\":\"progress\",\"percent\":42.5,\"current\":425,\"total\":1000,\"bytesPerSecond\":12000000}\n",
            &mut telemetry,
        );
        assert_eq!(telemetry.phase, "write");
        assert_eq!(telemetry.current_partition.as_deref(), Some("rootfs"));
        assert_eq!(telemetry.progress_pct, Some(42.5));
        assert_eq!(telemetry.partition_progress_pct, Some(42.5));
        assert_eq!(telemetry.transferred_bytes, Some(425));
        assert_eq!(telemetry.committed_bytes, None);
        assert_eq!(telemetry.written_bytes, None);
        assert_eq!(telemetry.bytes_per_second, Some(12_000_000.0));

        update_flash_telemetry(
            "{\"type\":\"verify\",\"status\":\"success\",\"errorCode\":0}\n\
             {\"type\":\"post_action\",\"status\":\"success\",\"attempt\":1}\n\
             {\"type\":\"finished\",\"postAction\":\"reboot\"}\n",
            &mut telemetry,
        );
        assert_eq!(telemetry.phase, "complete");
        assert_eq!(telemetry.verify_state, "success");
        assert_eq!(telemetry.post_flash_state, "success");
        assert_eq!(telemetry.progress_pct, Some(100.0));
        assert_eq!(telemetry.committed_bytes, Some(1000));
        assert_eq!(telemetry.written_bytes, Some(1000));
    }

    #[test]
    fn classifies_new_and_legacy_rockchip_open_failures() {
        for (line, expected) in [
            (
                "Error: ROCKCHIP_USB_ACCESS_DENIED (LIBUSB_ERROR_ACCESS)",
                "ROCKCHIP_USB_ACCESS_DENIED",
            ),
            (
                "Error: ROCKCHIP_USB_DEVICE_BUSY (LIBUSB_ERROR_BUSY): close other tools",
                "ROCKCHIP_USB_DEVICE_BUSY",
            ),
            (
                "Error: ROCKCHIP_USB_DRIVER_UNSUPPORTED (LIBUSB_ERROR_NOT_SUPPORTED)",
                "ROCKCHIP_USB_DRIVER_UNSUPPORTED",
            ),
            (
                "Error: ROCKCHIP_USB_DEVICE_DISCONNECTED (LIBUSB_ERROR_NO_DEVICE)",
                "ROCKCHIP_USB_DEVICE_DISCONNECTED",
            ),
            (
                "Error: ROCKCHIP_USB_OPEN_FAILED (LIBUSB_ERROR_OTHER)",
                "ROCKCHIP_USB_OPEN_FAILED",
            ),
            (
                "Error: cannot open selected Rockchip device",
                "ROCKCHIP_USB_OPEN_FAILED",
            ),
        ] {
            let mut telemetry = FlashTelemetry::new("1:10.1", "reboot");
            update_flash_telemetry(
                &format!("Stage: Query Device\n{line}\nFlash failed.\n"),
                &mut telemetry,
            );
            assert_eq!(telemetry.phase, "query_device");
            assert_eq!(telemetry.diagnostic_error_code.as_deref(), Some(expected));
        }
    }

    #[test]
    fn unrelated_flasher_failures_do_not_receive_rockchip_open_codes() {
        let mut telemetry = FlashTelemetry::new("1:10.1", "reboot");
        update_flash_telemetry(
            "Stage: Flashing Partitions\nError: write failed\n",
            &mut telemetry,
        );
        assert_eq!(telemetry.phase, "write");
        assert_eq!(telemetry.diagnostic_error_code, None);
    }

    #[test]
    fn queued_transfer_is_never_reported_as_media_written() {
        let mut telemetry = FlashTelemetry::new("invalid", "none");
        update_flash_telemetry(
            "{\"type\":\"progress\",\"percent\":94.0,\"current\":29097984,\"transferredBytes\":29097984,\"queuedBytes\":29097984,\"committedBytes\":null,\"total\":29097984}\n",
            &mut telemetry,
        );
        assert_eq!(telemetry.transferred_bytes, Some(29_097_984));
        assert_eq!(telemetry.queued_bytes, Some(29_097_984));
        assert_eq!(telemetry.committed_bytes, None);
        assert_eq!(telemetry.written_bytes, None);
    }

    #[test]
    fn persists_plain_terminal_progress_in_telemetry() {
        let mut telemetry = FlashTelemetry::new("invalid", "reboot");
        assert_eq!(
            update_flash_telemetry("Writing firmware 37%", &mut telemetry),
            Some(37)
        );
        assert_eq!(telemetry.phase, "write");
        assert_eq!(telemetry.progress_pct, Some(37.0));
    }

    #[test]
    fn raw_flash_reports_the_exact_preflight_stage_before_media_write() {
        let mut telemetry = FlashTelemetry::new("0:4", "reboot");
        update_flash_telemetry(
            "{\"type\":\"log\",\"level\":\"info\",\"message\":\"RAW_PREFLIGHT_STAGE:fes_handshake\"}\n",
            &mut telemetry,
        );
        assert_eq!(telemetry.phase, "raw_fes_handshake");

        update_flash_telemetry(
            "{\"type\":\"log\",\"level\":\"info\",\"message\":\"RAW_WRITE_START\"}\n",
            &mut telemetry,
        );
        assert_eq!(telemetry.phase, "write");
        assert_eq!(
            telemetry.current_partition.as_deref(),
            Some("eMMC user area")
        );
    }

    #[test]
    fn spinor_preflight_rejection_does_not_report_erasing_from_error_text() {
        let message = "Raw disk flash failed: Invalid firmware format: RAW_SPINOR_PHYSICAL_LAYOUT_UNVERIFIED: SPI NOR physical read/write mapping and independent physical readback are not verified for this loader; refusing erase/MBR/image writes. No guessed offset is permitted.";
        for line in [
            serde_json::json!({"type":"log", "level":"error", "message":message}).to_string(),
            format!("Error: {message}"),
        ] {
            let mut telemetry = FlashTelemetry::new("", "reboot");
            update_flash_telemetry("RAW_TARGET_STORAGE:SPI NOR\n", &mut telemetry);
            telemetry.phase = "query_device".to_owned();
            telemetry.transferred_bytes = Some(0);
            update_flash_telemetry(&line, &mut telemetry);
            mark_telemetry_process_failure(&mut telemetry);
            assert_eq!(telemetry.phase, "spinor_preflight_blocked");
            assert_eq!(
                telemetry.diagnostic_error_code.as_deref(),
                Some("RAW_SPINOR_PHYSICAL_LAYOUT_UNVERIFIED")
            );
            assert_eq!(telemetry.transferred_bytes, Some(0));
            assert_eq!(telemetry.committed_bytes, None);
            assert_eq!(telemetry.verify_state, "not_run");
            assert_eq!(telemetry.post_flash_state, "not_run");
        }
    }

    #[test]
    fn emmc_boot0_status_never_claims_whole_image_verification() {
        let mut telemetry = FlashTelemetry::new("location:libusb:3:3.2", "none");
        update_flash_telemetry(
            "{\"type\":\"log\",\"level\":\"info\",\"message\":\"EMMC_BOOT0_WRITE_START:method=fes_boot0_component\"}\n",
            &mut telemetry,
        );
        assert_eq!(telemetry.phase, "emmc_boot0_write");
        assert_eq!(telemetry.current_partition.as_deref(), Some("eMMC Boot0"));
        assert_eq!(telemetry.verify_state, "running");
        assert_eq!(telemetry.verify_error_code, None);

        update_flash_telemetry(
            "{\"type\":\"log\",\"level\":\"info\",\"message\":\"EMMC_BOOT0_VERIFIED:bytes=49152,sha256=abc\"}\n",
            &mut telemetry,
        );
        assert_eq!(telemetry.phase, "emmc_boot0_verify");
        assert_eq!(telemetry.partition_progress_pct, Some(100.0));
        assert_eq!(telemetry.verify_state, "running");
        assert_eq!(telemetry.verify_error_code, None);

        update_flash_telemetry(
            "{\"type\":\"verify\",\"status\":\"success\",\"errorCode\":0}\n",
            &mut telemetry,
        );
        assert_eq!(telemetry.verify_state, "success");
        assert_eq!(telemetry.verify_error_code, Some(0));
    }

    #[test]
    fn raw_mbr_has_distinct_write_and_ready_phases() {
        let mut telemetry = FlashTelemetry::new("0:4", "none");
        update_flash_telemetry("RAW_MBR_DOWNLOAD_START\n", &mut telemetry);
        assert_eq!(telemetry.phase, "raw_mbr_write");
        assert_eq!(telemetry.current_partition.as_deref(), Some("MBR"));
        update_flash_telemetry("RAW_MBR_DOWNLOAD_READY\n", &mut telemetry);
        assert_eq!(telemetry.phase, "raw_mbr_verify");
        assert_eq!(telemetry.partition_progress_pct, Some(100.0));
    }

    #[test]
    fn spinor_protocol_completion_does_not_prove_physical_layout_or_boot() {
        let mut telemetry = FlashTelemetry::new("", "reboot");
        observe_openix_log_message("RAW_TARGET_STORAGE:SPI NOR", &mut telemetry);
        observe_openix_log_message("RAW_MBR_DOWNLOAD_START", &mut telemetry);
        assert_eq!(telemetry.phase, "spinor_mbr_write");
        observe_openix_log_message("RAW_WRITE_START", &mut telemetry);
        assert_eq!(telemetry.current_partition.as_deref(), Some("SPI NOR"));
        update_flash_telemetry(
            "{\"type\":\"verify\",\"status\":\"success\"}\n{\"type\":\"finished\"}\n",
            &mut telemetry,
        );
        let details = serde_json::to_value(&telemetry).unwrap();
        assert_eq!(details["verifyState"], "success");
        assert_eq!(details["physicalVerifyState"], "not_verified");
        assert_eq!(details["bootVerifyState"], "not_verified");
        assert_eq!(details["postFlashState"], "pending");
        assert_eq!(details["phase"], "spinor_awaiting_acceptance");
    }

    #[test]
    fn spinor_finished_cannot_overwrite_failure_or_invent_media_commit() {
        let mut telemetry = FlashTelemetry::new("", "reboot");
        observe_openix_log_message("RAW_TARGET_STORAGE:SPI NOR", &mut telemetry);
        telemetry.total_bytes = Some(4096);
        update_flash_telemetry(
            "{\"type\":\"verify\",\"status\":\"failed\"}\n{\"type\":\"post_action\",\"action\":\"reboot\",\"status\":\"failed\",\"error\":\"USB error\"}\n{\"type\":\"finished\"}\n",
            &mut telemetry,
        );
        assert_eq!(telemetry.verify_state, "failed");
        assert_eq!(telemetry.post_flash_state, "failed");
        assert_eq!(telemetry.post_flash_error.as_deref(), Some("USB error"));
        assert_eq!(telemetry.committed_bytes, None);
        assert!(!post_action_disconnect_is_success(
            "openixcli",
            &[],
            &telemetry
        ));
    }

    #[test]
    fn spinor_physical_readback_confirms_media_but_not_boot_or_reboot() {
        let mut telemetry = FlashTelemetry::new("", "reboot");
        update_flash_telemetry(
            "RAW_TARGET_STORAGE:SPI NOR\nRAW_SPINOR_PHYSICAL_VERIFY_START\n",
            &mut telemetry,
        );
        assert_eq!(telemetry.physical_verify_state.as_deref(), Some("running"));
        update_flash_telemetry("RAW_SPINOR_PHYSICAL_VERIFY_SUCCESS:imageSha256=e22ef5250bb58415fa48de8fb5795da519d804a3145f0bd1632bb8e6b8451f15,imageBytes=4890624,fullFlashBytes=16777216,outsideImage=unchanged\n{\"type\":\"finished\"}\n", &mut telemetry);
        assert_eq!(telemetry.physical_verify_state.as_deref(), Some("success"));
        assert_eq!(telemetry.committed_bytes, Some(4890624));
        assert_eq!(
            telemetry.physical_image_sha256.as_deref(),
            Some("e22ef5250bb58415fa48de8fb5795da519d804a3145f0bd1632bb8e6b8451f15")
        );
        assert_eq!(telemetry.outside_image_state.as_deref(), Some("unchanged"));
        assert_eq!(telemetry.boot_verify_state.as_deref(), Some("not_verified"));
        assert_eq!(telemetry.post_flash_state, "pending");
        assert_eq!(telemetry.phase, "spinor_awaiting_boot");
    }

    #[test]
    fn spinor_write_completion_is_independent_of_failed_readback_and_reboot() {
        let mut telemetry = FlashTelemetry::new("", "reboot");
        update_flash_telemetry("RAW_TARGET_STORAGE:SPI NOR\nRAW_WRITE_START:medium=spinor,physicalStart=0\nRAW_WRITE_COMPLETE:medium=spinor\nRAW_SPINOR_PHYSICAL_VERIFY_START\nError: RAW_SPINOR_PHYSICAL_VERIFY_FAILED: byte 0 mismatch\n",&mut telemetry);
        assert_eq!(telemetry.write_state.as_deref(), Some("success"));
        assert_eq!(telemetry.physical_verify_state.as_deref(), Some("failed"));
        assert_eq!(telemetry.physical_image_sha256, None);
        assert_eq!(telemetry.outside_image_state, None);
        assert_eq!(telemetry.post_flash_state, "not_run");
        assert_eq!(telemetry.boot_verify_state.as_deref(), Some("not_verified"));
        let mut other = FlashTelemetry::new("", "reboot");
        update_flash_telemetry(
            "RAW_TARGET_STORAGE:eMMC\nRAW_WRITE_COMPLETE:medium=spinor\n",
            &mut other,
        );
        assert_eq!(other.write_state, None);
    }
    #[test]
    fn spinor_physical_failure_never_becomes_preflight_rejection_or_completion() {
        let mut telemetry = FlashTelemetry::new("", "reboot");
        update_flash_telemetry("RAW_TARGET_STORAGE:SPI NOR\nRAW_WRITE_START\nRAW_SPINOR_PHYSICAL_VERIFY_START\nError: RAW_SPINOR_PHYSICAL_VERIFY_FAILED: byte 0 mismatch\n", &mut telemetry);
        assert_eq!(telemetry.physical_verify_state.as_deref(), Some("failed"));
        assert_eq!(telemetry.phase, "spinor_physical_verify");
        assert_eq!(telemetry.post_flash_state, "not_run");
        assert_eq!(telemetry.committed_bytes, None);
        assert_ne!(telemetry.phase, "spinor_preflight_blocked");
    }

    #[test]
    fn flasher_cancel_marker_is_scoped_and_removed_on_drop() {
        let marker = FlasherCancellationMarker::new("flash-test:raw");
        assert!(!marker.path().exists());
        marker.request().expect("write cooperative cancel marker");
        let path = marker.path().to_owned();
        assert!(path.is_file());
        drop(marker);
        assert!(!path.exists());
    }

    #[test]
    fn flasher_stream_records_are_bounded_without_losing_the_next_record() {
        let mut pending = Vec::new();
        let mut input = vec![b'x'; FLASHER_STREAM_RECORD_LIMIT + 4096];
        input.extend_from_slice(b"\nnext\n");
        let records = take_stream_records(&mut pending, &input);
        assert_eq!(records.len(), 2);
        assert!(records[0].len() <= FLASHER_STREAM_RECORD_LIMIT + "...[truncated]".len());
        assert!(records[0].ends_with(b"...[truncated]"));
        assert_eq!(records[1], b"next");
        assert!(pending.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn ordinary_flasher_termination_kills_the_helper_process_group() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "sleep 60 & wait"]);
        configure_background_command(&mut command);
        let mut child = command.spawn().unwrap();
        let process_job = crate::process::ProcessJob::attach(&child).unwrap();
        let started = Instant::now();

        terminate_flasher_process_tree(&process_job, &mut child).unwrap();

        assert!(started.elapsed() < FLASHER_TERMINATE_TIMEOUT);
        assert!(child.try_wait().unwrap().is_some());
    }

    #[test]
    fn keeps_backend_speed_and_caps_display_speed_to_usb_link() {
        let mut telemetry = FlashTelemetry::new("3:1", "reboot");
        // This is a pure telemetry test. Do not let a real device currently
        // attached at bus 3 / port 1 change the expected USB link speed.
        telemetry.usb_speed = "full".to_owned();
        telemetry.usb_theoretical_mbps = Some(12.0);
        update_flash_telemetry(
            "{\"type\":\"progress\",\"current\":11300000,\"total\":22600000,\"bytesPerSecond\":11300000}\n",
            &mut telemetry,
        );
        assert_eq!(telemetry.backend_bytes_per_second, Some(11_300_000.0));
        assert_eq!(telemetry.bytes_per_second, Some(1_500_000.0));
    }

    #[test]
    fn retains_only_the_latest_raw_flasher_output_lines() {
        let mut telemetry = FlashTelemetry::new("invalid", "reboot");
        let output = (0..100)
            .map(|index| format!("raw line {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        update_flash_telemetry(&output, &mut telemetry);
        assert_eq!(telemetry.output_tail.len(), 80);
        assert_eq!(
            telemetry.output_tail.first().map(String::as_str),
            Some("raw line 20")
        );
        assert_eq!(
            telemetry.output_tail.last().map(String::as_str),
            Some("raw line 99")
        );
    }

    #[test]
    fn allwinner_reboot_disconnect_after_verify_is_success() {
        let mut telemetry = FlashTelemetry::new("3:1", "reboot");
        update_flash_telemetry(
            "{\"type\":\"verify\",\"status\":\"success\",\"errorCode\":0}\n\
             {\"type\":\"post_action\",\"status\":\"failed\",\"attempt\":2,\"error\":\"Failed to open device: Device not found\"}\n",
            &mut telemetry,
        );
        let args = vec!["--post-action".to_owned(), "reboot".to_owned()];
        assert!(post_action_disconnect_is_success(
            "openixcli",
            &args,
            &telemetry
        ));
        assert!(!post_action_disconnect_is_success(
            "openrockcli",
            &args,
            &telemetry
        ));
    }

    #[test]
    fn allwinner_reboot_disconnect_after_skipped_verify_is_success() {
        let mut telemetry = FlashTelemetry::new("3:2", "reboot");
        update_flash_telemetry(
            "{\"type\":\"verify\",\"status\":\"skipped\",\"errorCode\":null}\n\
             {\"type\":\"post_action\",\"status\":\"failed\",\"attempt\":2,\"error\":\"Failed to open device: Device not found\"}\n",
            &mut telemetry,
        );
        let args = vec!["--post-action".to_owned(), "reboot".to_owned()];
        assert!(post_action_disconnect_is_success(
            "openixcli",
            &args,
            &telemetry
        ));
    }

    #[test]
    fn allwinner_reboot_disconnect_does_not_hide_verify_failure() {
        let mut telemetry = FlashTelemetry::new("3:2", "reboot");
        update_flash_telemetry(
            "{\"type\":\"verify\",\"status\":\"failed\",\"errorCode\":-1}\n\
             {\"type\":\"post_action\",\"status\":\"failed\",\"attempt\":2,\"error\":\"Failed to open device: Device not found\"}\n",
            &mut telemetry,
        );
        let args = vec!["--post-action".to_owned(), "reboot".to_owned()];
        assert!(!post_action_disconnect_is_success(
            "openixcli",
            &args,
            &telemetry
        ));
    }

    #[test]
    fn allwinner_reboot_fallback_accepts_verified_device_disappearance() {
        assert!(reboot_open_race_is_success("Device not found"));
        assert!(!reboot_open_race_is_success("Access denied"));
    }

    #[test]
    fn builds_rockchip_arguments_with_dotted_usb_port() {
        let args = build_flash_args("openrockcli", "1:2.3", "/tmp/update.img", Vec::new(), true);
        assert_eq!(
            &args[..4],
            ["flash", "/tmp/update.img", "--output", "jsonl"]
        );
        assert!(args.windows(2).any(|pair| pair == ["--bus", "1"]));
        assert!(args.windows(2).any(|pair| pair == ["--port", "2.3"]));
        assert!(args.windows(2).any(|pair| pair == ["--mode", "full_erase"]));
    }

    #[test]
    fn applies_allwinner_binding_to_the_selected_board() {
        let args = build_flash_args(
            "openixcli",
            "1:2",
            "/tmp/image.img",
            vec!["--verify".to_owned(), "false".to_owned()],
            false,
        );
        assert_eq!(
            args,
            [
                "flash",
                "/tmp/image.img",
                "--verify",
                "false",
                "--bus",
                "1",
                "--port",
                "2"
            ]
        );
    }

    #[test]
    fn allwinner_physical_binding_survives_case_and_enumeration_changes() {
        let first = stable_allwinner_binding("PCIROOT(0)#USBROOT(0)#USB(4)");
        let reenumerated = stable_allwinner_binding("pciROOT(0)#usbROOT(0)#usb(4)");
        assert_eq!(first, reenumerated);
        assert_ne!(
            first,
            stable_allwinner_binding("PCIROOT(0)#USBROOT(0)#USB(5)")
        );
        assert!(!first.contains("0:8"));
        assert!(!first.contains("0:10"));
    }

    #[test]
    fn legacy_libusb_binding_resolves_to_full_hub_path_without_crossing_bus_or_leaf() {
        assert!(allwinner_binding_matches(
            "location:libusb:2:2",
            "location:libusb:2:3.2"
        ));
        assert!(allwinner_binding_matches(
            "location:libusb:2:3.2",
            "location:libusb:2:3.2"
        ));
        assert!(!allwinner_binding_matches(
            "location:libusb:2:2",
            "location:libusb:2:3.1"
        ));
        assert!(!allwinner_binding_matches(
            "location:libusb:2:2",
            "location:libusb:3:3.2"
        ));
    }

    #[test]
    fn allwinner_flash_uses_location_instead_of_transient_bus_port() {
        let args = build_flash_args(
            "openixcli",
            "location:pciroot(0)#usbroot(0)#usb(4)",
            "C:/image.raw",
            Vec::new(),
            true,
        );
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--device-location", "pciroot(0)#usbroot(0)#usb(4)"]));
        assert!(!args.iter().any(|arg| arg == "--bus" || arg == "--port"));
    }

    #[test]
    fn command_raw_keeps_cli_layout_exact_and_passes_geometry_out_of_band() {
        let mut device = device("allwinner", "openixcli");
        device.serial_port = "/dev/ttyACM0".to_owned();
        device.baud_rate = 115_200;
        let args = build_flash_args(
            "openixcli",
            "location:libusb:3:2",
            "/tmp/mainline.raw",
            vec![
                "--raw".to_owned(),
                "--lynx-raw-loader".to_owned(),
                "/tmp/loader.bin".to_owned(),
            ],
            true,
        );

        assert_eq!(
            args,
            [
                "raw",
                "/tmp/loader.bin",
                "/tmp/mainline.raw",
                "--mode",
                "command",
                "--output",
                "jsonl",
                "--device-location",
                "libusb:3:2",
            ]
        );
        assert_eq!(
            openix_raw_geometry_environment(&device, &args),
            Some(("/dev/ttyACM0".to_owned(), "115200".to_owned()))
        );
    }

    #[test]
    fn legacy_allwinner_enumeration_binding_requires_explicit_rebind() {
        assert!(ensure_stable_allwinner_binding("openixcli", "0:10")
            .unwrap_err()
            .starts_with("USB_BINDING_LEGACY_REBIND_REQUIRED:"));
        assert!(ensure_stable_allwinner_binding(
            "openixcli",
            "location:pciroot(0)#usbroot(0)#usb(4)"
        )
        .is_ok());
        assert!(ensure_stable_allwinner_binding("openrockcli", "1:2.3").is_ok());
    }

    #[test]
    fn builds_nxp_uuu_arguments_with_usb_path_filter() {
        let args = build_flash_args(
            "uuu",
            "1:24",
            "/tmp/image.wic",
            vec!["-b".to_owned(), "emmc_all".to_owned()],
            false,
        );
        assert_eq!(args, ["-m", "1:24", "-b", "emmc_all", "/tmp/image.wic"]);
    }

    #[test]
    fn builds_nxp_uuu_direct_script_arguments() {
        let args = build_flash_args("uuu", "", "/tmp/flash.uuu", Vec::new(), false);
        assert_eq!(args, ["/tmp/flash.uuu"]);
    }

    #[test]
    fn selects_vendor_specific_version_arguments() {
        assert_eq!(version_args("openrockcli").unwrap(), ["--version"]);
        assert_eq!(version_args("uuu").unwrap(), ["-h"]);
    }

    #[test]
    fn parses_k230_device_and_medium_options() {
        assert_eq!(parse_k230_binding("3:7").unwrap(), (3, 7));
        assert_eq!(
            parse_k230_options(&["--medium".to_owned(), "spi_nand".to_owned()]).unwrap(),
            (MediumType::SpiNand, true)
        );
        assert_eq!(
            parse_k230_options(&[
                "--medium".to_owned(),
                "sd_card".to_owned(),
                "--no-reboot".to_owned()
            ])
            .unwrap(),
            (MediumType::SdCard, false)
        );
    }

    #[test]
    fn rejects_invalid_k230_device_and_options() {
        assert!(parse_k230_binding("").is_err());
        assert!(parse_k230_options(&["--medium".to_owned(), "unknown".to_owned()]).is_err());
    }

    #[test]
    fn parses_json_argument_array_without_shell() {
        let args = parse_extra_args(r#"["--bus","1","--port","5"]"#).unwrap();
        assert_eq!(args, ["--bus", "1", "--port", "5"]);
    }

    #[test]
    fn extracts_progress_percentage() {
        assert_eq!(parse_percent("Writing system [42.5%]\r"), Some(42));
        assert_eq!(
            parse_percent(r#"{"type":"progress","percent":73.8}"#),
            Some(73)
        );
        assert_eq!(parse_percent("No progress here"), None);
    }
}
