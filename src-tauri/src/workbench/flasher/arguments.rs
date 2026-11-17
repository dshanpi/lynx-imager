//! Pure vendor argument construction and validation.
//!
//! This module deliberately has no process, USB, Tauri, or device-session
//! dependencies. The parent adapter remains the sole owner of execution order.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RawMode {
    Command,
    SpinorPhysical,
}

impl RawMode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::SpinorPhysical => "spinor-physical",
        }
    }
}

pub(super) fn build_flash_args(
    flasher_kind: &str,
    usb_binding: &str,
    firmware_path: &str,
    extra_args: Vec<String>,
    jsonl: bool,
) -> Vec<String> {
    if flasher_kind == "dfu-util" {
        let path = usb_binding
            .split_once(':')
            .map(|(bus, port)| format!("{bus}-{port}"))
            .unwrap_or_else(|| usb_binding.to_owned());
        let mut args = vec!["-p".to_owned(), path];
        if extra_args.is_empty() {
            args.extend(["-a".to_owned(), "0".to_owned(), "-R".to_owned()]);
        } else {
            args.extend(extra_args);
        }
        args.extend(["-D".to_owned(), firmware_path.to_owned()]);
        return args;
    }
    if flasher_kind == "uuu" {
        let mut args = Vec::new();
        if !usb_binding.trim().is_empty() {
            args.extend(["-m".to_owned(), usb_binding.to_owned()]);
        }
        args.extend(extra_args);
        args.push(firmware_path.to_owned());
        return args;
    }

    if flasher_kind == "openixcli" && extra_args.iter().any(|arg| arg == "--raw") {
        if let Some(loader) = option_value(&extra_args, "--lynx-loader")
            .or_else(|| option_value(&extra_args, "--lynx-raw-loader"))
        {
            // Whole-disk raw images must use OpenixCLI's dedicated command
            // layout. The loader is the first positional input and remains a
            // RAM-only DRAM/FES bootstrap; it is never a flash artifact.
            let spinor = extra_args.iter().any(|arg| arg == "--lynx-spinor");
            let raw_mode = if spinor {
                RawMode::SpinorPhysical
            } else {
                RawMode::Command
            };
            let mut args = vec![
                "raw".to_owned(),
                loader.to_owned(),
                firmware_path.to_owned(),
                "--mode".to_owned(),
                raw_mode.as_str().to_owned(),
            ];
            if let Some(action) = option_value(&extra_args, "--post-action") {
                args.extend(["--post-action".to_owned(), action.to_owned()]);
            }
            if jsonl {
                args.extend(["--output".to_owned(), "jsonl".to_owned()]);
            }
            if !spinor
                && extra_args
                    .iter()
                    .any(|argument| argument == "--emmc-boot0-from-image")
            {
                args.push("--emmc-boot0-from-image".to_owned());
            }
            append_openix_selector(&mut args, usb_binding);
            return args;
        }
    }

    let mut args = vec!["flash".to_owned(), firmware_path.to_owned()];
    if jsonl {
        args.extend(["--output".to_owned(), "jsonl".to_owned()]);
    }
    if extra_args.is_empty() {
        args.extend([
            "--verify".to_owned(),
            "true".to_owned(),
            "--mode".to_owned(),
            "full_erase".to_owned(),
            "--post-action".to_owned(),
            "reboot".to_owned(),
        ]);
    } else if flasher_kind == "openixcli" {
        let loader = option_value(&extra_args, "--lynx-loader")
            .or_else(|| option_value(&extra_args, "--lynx-raw-loader"))
            .map(str::to_owned);
        if let Some(loader) = loader {
            args.extend(["--bootstrap-firmware".to_owned(), loader]);
            args.push("--nand-components".to_owned());
        }
        let mut index = 0;
        while index < extra_args.len() {
            if matches!(
                extra_args[index].as_str(),
                "--lynx-loader" | "--lynx-raw-loader"
            ) {
                index += 2;
                continue;
            }
            args.push(extra_args[index].clone());
            index += 1;
        }
    } else {
        args.extend(extra_args);
    }
    if matches!(flasher_kind, "openixcli" | "openrockcli") {
        if flasher_kind == "openixcli" {
            append_openix_selector(&mut args, usb_binding);
            return args;
        }
        if let Some((bus, port)) = usb_binding.split_once(':') {
            args.extend([
                "--bus".to_owned(),
                bus.to_owned(),
                "--port".to_owned(),
                port.to_owned(),
            ]);
        }
    }
    args
}

fn option_value<'a>(args: &'a [String], option: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|pair| pair[0] == option)
        .map(|pair| pair[1].as_str())
}

fn append_openix_selector(args: &mut Vec<String>, usb_binding: &str) {
    if let Some(location) = usb_binding.strip_prefix("location:") {
        args.extend([
            "--device-location".to_owned(),
            openixcli_compatible_location(location),
        ]);
    } else if let Some((bus, port)) = usb_binding.split_once(':') {
        args.extend([
            "--bus".to_owned(),
            bus.to_owned(),
            "--port".to_owned(),
            port.to_owned(),
        ]);
    }
}

fn openixcli_compatible_location(location: &str) -> String {
    #[cfg(not(target_os = "windows"))]
    {
        // OpenixCLI 0.2.11's pinned libefex exposes Linux locations as
        // libusb:<bus>:<leaf-port>. LYNX intentionally stores the complete
        // physical topology and performs a unique-target preflight first; the
        // child process receives the legacy leaf form only for its open call.
        if let Some(rest) = location.strip_prefix("libusb:") {
            if let Some((bus, path)) = rest.split_once(':') {
                if let Some(leaf) = path.rsplit('.').next() {
                    return format!("libusb:{bus}:{leaf}");
                }
            }
        }
    }
    location.to_owned()
}

pub(super) fn version_args(kind: &str) -> Result<&'static [&'static str], String> {
    match kind {
        "openixcli" | "openrockcli" | "dfu-util" => Ok(&["--version"]),
        "uuu" => Ok(&["-h"]),
        other => Err(format!("FLASHER_UNSUPPORTED:{other}")),
    }
}

pub(super) fn validate_dfu_args(args: &[String]) -> Result<(), String> {
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-a" | "--alt" | "-s" | "--dfuse-address" | "-t" | "--transfer-size" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| format!("STM32_DFU_ARGS:{} requires a value", args[index]))?;
                if value.trim().is_empty() || value.starts_with('-') {
                    return Err(format!("STM32_DFU_ARGS:{} has an invalid value", args[index]));
                }
                index += 2;
            }
            "-R" | "--reset" => index += 1,
            option => {
                return Err(format!(
                    "STM32_DFU_ARGS:unsupported or unsafe option {option}; LYNX owns device selection and download path"
                ))
            }
        }
    }
    Ok(())
}

pub(crate) fn parse_extra_args(value: &str) -> Result<Vec<String>, String> {
    if value.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str::<Vec<String>>(value)
        .map_err(|error| format!("FLASHER_ARGS:expected a JSON string array: {error}"))
}

#[cfg(test)]
mod tests {
    use super::build_flash_args;

    #[test]
    fn allwinner_raw_uses_command_layout_with_loader_first() {
        let args = build_flash_args(
            "openixcli",
            "0:2",
            "mainline-spinand-logical.raw",
            vec![
                "--raw".to_owned(),
                "--lynx-raw-loader".to_owned(),
                "t113s3-ddr3-spinand-dshanpi-t113s3pro-loader.bin".to_owned(),
                "--verify".to_owned(),
                "false".to_owned(),
                "--mode".to_owned(),
                "full_erase".to_owned(),
                "--post-action".to_owned(),
                "reboot".to_owned(),
            ],
            false,
        );

        assert_eq!(
            args,
            [
                "raw",
                "t113s3-ddr3-spinand-dshanpi-t113s3pro-loader.bin",
                "mainline-spinand-logical.raw",
                "--mode",
                "command",
                "--post-action",
                "reboot",
                "--bus",
                "0",
                "--port",
                "2",
            ]
        );
    }

    #[test]
    fn allwinner_nand_components_use_separate_ram_only_loader() {
        let args = build_flash_args(
            "openixcli",
            "location:libusb:3:2",
            "mainline-nand-components.img",
            vec![
                "--lynx-loader".to_owned(),
                "t113s3-loader.bin".to_owned(),
                "--verify".to_owned(),
                "true".to_owned(),
                "--mode".to_owned(),
                "full_erase".to_owned(),
                "--post-action".to_owned(),
                "none".to_owned(),
            ],
            true,
        );

        assert_eq!(
            args,
            [
                "flash",
                "mainline-nand-components.img",
                "--output",
                "jsonl",
                "--bootstrap-firmware",
                "t113s3-loader.bin",
                "--nand-components",
                "--verify",
                "true",
                "--mode",
                "full_erase",
                "--post-action",
                "none",
                "--device-location",
                "libusb:3:2",
            ]
        );
    }

    #[test]
    fn allwinner_emmc_raw_requests_mainline_spl_boot0_installation() {
        let expected_location = if cfg!(target_os = "windows") {
            "libusb:3:3.2"
        } else {
            "libusb:3:2"
        };
        let args = build_flash_args(
            "openixcli",
            "location:libusb:3:3.2",
            "avaota-a1-full-mainline.img",
            vec![
                "--raw".to_owned(),
                "--lynx-raw-loader".to_owned(),
                "t527-avaota-a1-loader.bin".to_owned(),
                "--emmc-boot0-from-image".to_owned(),
            ],
            true,
        );

        assert_eq!(
            args,
            [
                "raw",
                "t527-avaota-a1-loader.bin",
                "avaota-a1-full-mainline.img",
                "--mode",
                "command",
                "--output",
                "jsonl",
                "--emmc-boot0-from-image",
                "--device-location",
                expected_location,
            ]
        );
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn allwinner_linux_child_uses_legacy_leaf_after_full_path_preflight() {
        let args = build_flash_args(
            "openixcli",
            "location:libusb:3:3.1",
            "a133.img",
            Vec::new(),
            true,
        );
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--device-location", "libusb:3:1"]));
    }
    #[test]
    fn spinor_full_image_has_own_mode_no_emmc_boot0_and_exact_post_action() {
        for action in ["none", "reboot", "poweroff"] {
            let args = build_flash_args(
                "openixcli",
                "1:2",
                "nekobox-full.img",
                [
                    "--raw",
                    "--lynx-raw-loader",
                    "f101-ram-loader.img",
                    "--lynx-spinor",
                    "--emmc-boot0-from-image",
                    "--post-action",
                    action,
                ]
                .map(str::to_owned)
                .to_vec(),
                true,
            );
            assert_eq!(
                args,
                [
                    "raw",
                    "f101-ram-loader.img",
                    "nekobox-full.img",
                    "--mode",
                    "spinor-physical",
                    "--post-action",
                    action,
                    "--output",
                    "jsonl",
                    "--bus",
                    "1",
                    "--port",
                    "2"
                ]
            );
        }
    }
}
