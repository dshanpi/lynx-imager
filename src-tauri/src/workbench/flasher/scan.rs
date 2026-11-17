//! Pure parsers for vendor discovery output.

use super::super::types::FlasherDeviceCandidate;

pub(super) fn parse_dfu_util_scan(output: &str) -> Vec<FlasherDeviceCandidate> {
    let mut devices = Vec::new();
    for line in output.lines().filter(|line| line.contains("Found DFU:")) {
        let Some(ids) = extract_quoted_or_bracketed(line, '[', ']') else {
            continue;
        };
        let Some((vid, pid)) = ids.split_once(':') else {
            continue;
        };
        let (Ok(vid), Ok(pid)) = (u16::from_str_radix(vid, 16), u16::from_str_radix(pid, 16))
        else {
            continue;
        };
        if vid != 0x0483 || pid != 0xdf11 {
            continue;
        }
        let Some(path) = extract_field(line, "path=\"") else {
            continue;
        };
        let Some((bus, port)) = path.split_once('-') else {
            continue;
        };
        let Ok(bus) = bus.parse::<u16>() else {
            continue;
        };
        let key = format!("{bus}:{port}");
        if devices
            .iter()
            .any(|candidate: &FlasherDeviceCandidate| candidate.key == key)
        {
            continue;
        }
        devices.push(FlasherDeviceCandidate {
            key,
            flasher_kind: "dfu-util".to_owned(),
            bus,
            port: port.to_owned(),
            vid,
            pid,
            mode: "dfu".to_owned(),
            chip: "STM32 DFU".to_owned(),
        });
    }
    devices
}

fn extract_quoted_or_bracketed(value: &str, start: char, end: char) -> Option<&str> {
    let start_index = value.find(start)? + start.len_utf8();
    let end_index = value[start_index..].find(end)? + start_index;
    Some(&value[start_index..end_index])
}

fn extract_field<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    let start = value.find(prefix)? + prefix.len();
    let end = value[start..].find('"')? + start;
    Some(&value[start..end])
}

pub(super) fn parse_openrockcli_scan(output: &str) -> Result<Vec<FlasherDeviceCandidate>, String> {
    let mut devices = Vec::new();
    for line in output.lines() {
        let columns: Vec<_> = line.split_whitespace().collect();
        if columns.len() < 5 || columns[2].len() != 9 || !columns[2].contains(':') {
            continue;
        }
        let Ok(bus) = columns[0].parse::<u16>() else {
            continue;
        };
        let Some((vid, pid)) = columns[2].split_once(':') else {
            continue;
        };
        let (Ok(vid), Ok(pid)) = (u16::from_str_radix(vid, 16), u16::from_str_radix(pid, 16))
        else {
            continue;
        };
        let port = columns[1].to_owned();
        devices.push(FlasherDeviceCandidate {
            key: format!("{bus}:{port}"),
            flasher_kind: "openrockcli".to_owned(),
            bus,
            port,
            vid,
            pid,
            mode: columns[3].to_owned(),
            chip: columns[4..].join(" "),
        });
    }
    Ok(devices)
}

pub(super) fn parse_uuu_scan(output: &str) -> Result<Vec<FlasherDeviceCandidate>, String> {
    let mut devices = Vec::new();
    for line in output.lines() {
        let columns: Vec<_> = line.split_whitespace().collect();
        if columns.len() < 6 || !columns[0].contains(':') {
            continue;
        }
        let Some((bus, port)) = columns[0].split_once(':') else {
            continue;
        };
        let Ok(bus) = bus.parse::<u16>() else {
            continue;
        };
        let (Ok(vid), Ok(pid)) = (
            parse_prefixed_hex(columns[3]),
            parse_prefixed_hex(columns[4]),
        ) else {
            continue;
        };
        let port = port.to_owned();
        devices.push(FlasherDeviceCandidate {
            key: format!("{bus}:{port}"),
            flasher_kind: "uuu".to_owned(),
            bus,
            port,
            vid,
            pid,
            mode: columns[2].trim_end_matches(':').to_ascii_lowercase(),
            chip: columns[1].to_owned(),
        });
    }
    Ok(devices)
}

fn parse_prefixed_hex(value: &str) -> Result<u16, std::num::ParseIntError> {
    u16::from_str_radix(
        value
            .strip_prefix("0x")
            .or_else(|| value.strip_prefix("0X"))
            .unwrap_or(value),
        16,
    )
}
