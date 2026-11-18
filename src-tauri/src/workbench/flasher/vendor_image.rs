//! Read-only file inspection and bounded options for the offline vendor wizard.
use super::{
    adapter_override_args, parse_extra_args, AdapterFlashOptions, DeviceConfig,
    FlasherDeviceCandidate, LocalFlashOptions,
};
use crate::k230::kdimage::{KdImage, KDIMG_HEADER_MAGIC, KDIMG_PART_MAGIC};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VendorSetup {
    pub route: String,
    medium: Option<String>,
    alt: Option<u8>,
    address: Option<String>,
    target_key: Option<String>,
    erase_confirmed: Option<bool>,
    dependencies: Option<Vec<FileReference>>,
}
#[derive(Debug, Serialize)]
pub struct InspectionItem {
    label: String,
    detail: String,
    ok: bool,
}
#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct FileReference {
    path: String,
    bytes: u64,
    sha256: String,
}

#[derive(Debug, Serialize)]
pub struct Inspection {
    dependencies: Vec<FileReference>,
    route: String,
    path: String,
    bytes: u64,
    format: String,
    items: Vec<InspectionItem>,
    pub(super) blockers: Vec<String>,
    alternates: Vec<u8>,
}
impl Inspection {
    fn item(&mut self, label: impl Into<String>, detail: impl Into<String>, ok: bool) {
        self.items.push(InspectionItem {
            label: label.into(),
            detail: detail.into(),
            ok,
        });
    }
}

#[tauri::command]
pub async fn imager_inspect_vendor_image(
    route: String,
    path: String,
) -> Result<Inspection, String> {
    tauri::async_runtime::spawn_blocking(move || inspect(&route, Path::new(&path)))
        .await
        .map_err(|e| e.to_string())?
}

pub(super) fn inspect(route: &str, path: &Path) -> Result<Inspection, String> {
    if !std::fs::metadata(path)
        .map_err(|e| format!("无法读取文件：{e}"))?
        .is_file()
    {
        return Err("请选择普通镜像文件".into());
    }
    let mut f = File::open(path).map_err(|e| format!("无法打开镜像：{e}"))?;
    let metadata = f.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err("请选择非空的普通文件".into());
    }
    let mut head = [0u8; 512];
    let n = (metadata.len().min(512)) as usize;
    f.read_exact(&mut head[..n]).map_err(|e| e.to_string())?;
    let h = &head[..n];
    let filename = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    let mut r = Inspection {
        route: route.into(),
        path: path.to_string_lossy().into_owned(),
        bytes: metadata.len(),
        format: String::new(),
        items: Vec::new(),
        blockers: Vec::new(),
        alternates: Vec::new(),
        dependencies: Vec::new(),
    };
    let rk = h.starts_with(b"RKFW") || h.starts_with(b"RKAF");
    let kd = h.starts_with(&KDIMG_HEADER_MAGIC.to_le_bytes());
    let dfu = h.starts_with(b"DfuSe");
    let gzip = h.starts_with(&[0x1f, 0x8b]);
    let xz = h.starts_with(&[0xfd, 0x37, 0x7a, 0x58, 0x5a, 0]);
    let known_container =
        rk || kd || dfu || h.starts_with(b"IMAGEWTY") || h.starts_with(b"PK\x03\x04");
    match route {
        "rk-update" => {
            r.format = "Rockchip update.img".into();
            if !rk {
                r.blockers.push("文件头未识别为 RKFW / RKAF 厂商包".into());
            } else {
                r.item("包头", String::from_utf8_lossy(&h[..4]), true);
            }
        }
        "rk-emmc" => {
            r.format = if xz {
                "XZ 压缩整盘镜像"
            } else if gzip {
                "Gzip 压缩整盘镜像"
            } else {
                "eMMC 整盘 RAW"
            }
            .into();
            let supported = if xz {
                filename.ends_with(".img.xz")
            } else if gzip {
                filename.ends_with(".img.gz")
            } else {
                filename.ends_with(".img") && r.bytes % 512 == 0
            };
            if known_container || !supported {
                r.blockers
                    .push("请选择扇区对齐的 .img，或与压缩格式相符的 .img.xz / .img.gz".into());
            } else {
                r.item(
                    "写入范围",
                    "eMMC 用户区整盘；解压和布局复检在写入前完成",
                    true,
                );
            }
        }
        "nxp-uuu" => {
            r.format = "UUU 烧录脚本".into();
            if metadata.len() > 4 * 1024 * 1024 {
                return Err("UUU 输入超出可解析的脚本大小".into());
            }
            let script =
                std::fs::read_to_string(path).map_err(|_| "无法按文本读取 UUU 脚本".to_string())?;
            let script = script.trim_start_matches('\u{feff}');
            if !script
                .lines()
                .map(str::trim)
                .any(|s| s.starts_with("uuu_version "))
            {
                r.blockers.push("缺少 uuu_version 声明".into());
            }
            uuu_references(script, path.parent().unwrap_or(Path::new(".")), &mut r);
        }
        "nxp-configured" => {
            r.format = "单板 UUU 配置所需文件".into();
            if known_container || filename.ends_with(".uuu") || h.starts_with(b"uuu_version") {
                r.blockers
                    .push("请使用与单板 UUU 参数配套的输入文件".into());
            } else {
                r.item(
                    "执行依据",
                    "由已保存的单板 UUU 参数决定目标、顺序与分区",
                    true,
                );
            }
        }
        "st-dfuse" => {
            r.format = "DfuSe 多元素容器".into();
            if !dfu {
                r.blockers
                    .push("未找到 DfuSe 文件头，请核对烧录路线".into());
            } else if let Err(e) = dfuse(&mut f, h, &mut r) {
                r.blockers.push(e);
            }
        }
        "st-raw" | "k230-raw" => {
            r.format = "原始二进制镜像".into();
            if known_container
                || gzip
                || xz
                || !(filename.ends_with(".bin") || filename.ends_with(".img"))
            {
                r.blockers
                    .push("请选择未封装、未压缩的 .bin 或 .img 文件".into());
            } else if route == "k230-raw" && !(512..=u32::MAX as u64).contains(&r.bytes) {
                r.blockers
                    .push("K230 RAW 大小须在 512 字节至 4 GiB 以下".into());
            } else {
                r.item(
                    "写入起点",
                    if route == "st-raw" {
                        "由用户依据板卡文档输入"
                    } else {
                        "所选介质的偏移 0"
                    },
                    true,
                );
            }
        }
        "k230-kdimage" => {
            r.format = "嘉楠 KDImage".into();
            if !kd || h.len() < 512 {
                r.blockers.push("未找到完整 KDImage 文件头".into());
            } else {
                let count = read_u32(h, 16) as u64;
                if count == 0 || count > 4096 || 512 + count * 256 > r.bytes {
                    r.blockers
                        .push("KDImage 分区表长度无效或超出解析范围".into());
                } else {
                    let img = KdImage::parse(path).map_err(|e| format!("KDImage 校验失败：{e}"))?;
                    if img.header.version < 2 {
                        r.blockers
                            .push("当前 KDImage 解析器需要 v2 分区字段".into());
                    }
                    for p in img.parts {
                        let ok = p.magic == KDIMG_PART_MAGIC
                            && u64::from(p.part_content_offset) + u64::from(p.part_content_size)
                                <= r.bytes
                            && (p.part_content_size == 0
                                || u64::from(p.part_content_offset) >= 512 + count * 256);
                        if !ok {
                            r.blockers
                                .push(format!("分区 {} 的表项或数据范围无效", p.part_name));
                        }
                        r.item(
                            p.part_name,
                            format!("目标 0x{:X} · {} 字节", p.part_offset, p.part_content_size),
                            ok,
                        );
                    }
                }
            }
        }
        _ => return Err("未知的厂商烧录路线".into()),
    }
    Ok(r)
}

fn read_u32(buf: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap())
}
fn dfuse(file: &mut File, h: &[u8], result: &mut Inspection) -> Result<(), String> {
    if h.len() < 11 || h[5] != 1 || h[10] == 0 {
        return Err("DfuSe 前缀或目标数量无效".into());
    }
    let limit = u64::from(read_u32(h, 6));
    if limit.checked_add(16) != Some(result.bytes) {
        return Err("DfuSe 声明长度与文件不符".into());
    }
    let mut pos = 11;
    for _ in 0..h[10] {
        if pos + 274 > limit {
            return Err("目标头被截断".into());
        }
        file.seek(SeekFrom::Start(pos)).map_err(|e| e.to_string())?;
        let mut t = [0; 274];
        file.read_exact(&mut t).map_err(|e| e.to_string())?;
        if &t[..6] != b"Target" {
            return Err("目标标记无效".into());
        }
        let alt = t[6];
        let end = pos + 274 + u64::from(read_u32(&t, 266));
        let elements = read_u32(&t, 270);
        if end > limit || elements == 0 || elements > 4096 {
            return Err("目标元素数量或边界无效".into());
        }
        if !result.alternates.contains(&alt) {
            result.alternates.push(alt);
        }
        pos += 274;
        for _ in 0..elements {
            if pos + 8 > end {
                return Err("元素表截断".into());
            }
            file.seek(SeekFrom::Start(pos)).map_err(|e| e.to_string())?;
            let mut e = [0; 8];
            file.read_exact(&mut e).map_err(|e| e.to_string())?;
            let addr = read_u32(&e, 0);
            let size = read_u32(&e, 4);
            pos += 8 + u64::from(size);
            if size == 0 || pos > end || addr as u64 + size as u64 > u32::MAX as u64 + 1 {
                return Err("元素地址或数据超出范围".into());
            }
            result.item(
                format!("Alternate {alt}"),
                format!("0x{addr:08X} · {size} 字节"),
                true,
            );
        }
        if pos != end {
            return Err("DfuSe 目标数据长度不符".into());
        }
    }
    if pos != limit {
        return Err("DfuSe 元素范围与镜像长度不符".into());
    }
    file.seek(SeekFrom::Start(limit))
        .map_err(|e| e.to_string())?;
    let mut suffix = [0; 16];
    file.read_exact(&mut suffix).map_err(|e| e.to_string())?;
    if &suffix[8..11] != b"UFD" || suffix[11] != 16 {
        return Err("DFU 后缀无效".into());
    }
    Ok(())
}

// Parse file operands of common UUU data-transfer commands without executing the script.
fn uuu_references(script: &str, folder: &Path, result: &mut Inspection) {
    let mut refs = std::collections::BTreeSet::new();
    let root = match std::fs::canonicalize(folder) {
        Ok(p) => p,
        Err(e) => {
            result.blockers.push(format!("无法打开脚本目录：{e}"));
            return;
        }
    };
    for (i, line) in script.lines().enumerate() {
        let Some((protocol, command)) = line.trim().split_once(':') else {
            continue;
        };
        if protocol.trim_start().starts_with('#') {
            continue;
        }
        let Some(words) = script_words(command) else {
            result
                .blockers
                .push(format!("第 {} 行 · 文件名双引号未闭合", i + 1));
            continue;
        };
        if words.iter().filter(|w| w.as_str() == "-f").count() > 1 {
            result
                .blockers
                .push(format!("第 {} 行 · 重复的文件参数无法唯一核对", i + 1));
            continue;
        }
        let cmd = words
            .first()
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();
        let protocol = protocol
            .split('[')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        // UUU also registers host shell/indirection verbs for every USB protocol.
        // Only native transfers and target-side operations belong in this wizard.
        let supported = match protocol.as_str() {
            "sdp" | "sdps" | "sdpu" | "sdpv" => matches!(
                cmd.as_str(),
                "boot" | "dcd" | "jump" | "write" | "status" | "blog" | "delay" | "done"
            ),
            "fb" | "fastboot" | "fbk" => matches!(
                cmd.as_str(),
                "flash"
                    | "download"
                    | "ucmd"
                    | "acmd"
                    | "getvar"
                    | "erase"
                    | "reboot"
                    | "set_active"
                    | "done"
                    | "delay"
                    | "sync"
                    | "ucp"
            ),
            _ => false,
        };
        if !supported {
            result.blockers.push(format!(
                "第 {} 行 · 基础流程无法核对该 UUU 命令，需审阅脚本",
                i + 1
            ));
            continue;
        }
        if cmd == "ucp" && !words.get(2).is_some_and(|s| s.starts_with("T:")) {
            result
                .blockers
                .push(format!("第 {} 行 · 基础流程仅支持向设备传入文件", i + 1));
            continue;
        }
        let explicit = words
            .windows(2)
            .find(|v| v[0] == "-f")
            .map(|v| v[1].as_str());
        let file = explicit.or_else(|| match cmd.as_str() {
            "flash" | "download" | "ucp" if words.len() > 1 => {
                if cmd == "ucp" {
                    words
                        .get(1)
                        .map(String::as_str)
                        .filter(|s| !s.starts_with("T:"))
                } else {
                    words.last().map(String::as_str)
                }
            }
            _ => None,
        });
        if file.is_none()
            && (matches!(
                cmd.as_str(),
                "boot" | "dcd" | "write" | "flash" | "download" | "ucp"
            ) || words.last().is_some_and(|w| w == "-f"))
        {
            result
                .blockers
                .push(format!("第 {} 行 · 未找到待传入的本地文件", i + 1));
            continue;
        }
        if let Some(file) = file {
            if !refs.insert(file.to_owned()) {
                continue;
            }
            let dynamic =
                file.contains(['$', '*', '%']) || file.starts_with('@') || file.starts_with('-');
            let evidence = (|| -> Result<FileReference, String> {
                if dynamic {
                    return Err("动态引用无法在离线预检中解析".into());
                }
                let input = Path::new(file);
                if input.is_absolute()
                    || input.components().any(|part| {
                        matches!(
                            part,
                            std::path::Component::ParentDir
                                | std::path::Component::Prefix(_)
                                | std::path::Component::RootDir
                        )
                    })
                {
                    return Err("文件必须位于配套脚本目录内".into());
                }
                let absolute = folder
                    .join(input)
                    .canonicalize()
                    .map_err(|_| "缺失或为空".to_owned())?;
                if !absolute.starts_with(&root) {
                    return Err("引用的实际文件位于脚本目录外".into());
                }
                let metadata = absolute.metadata().map_err(|e| e.to_string())?;
                if !metadata.is_file() || metadata.len() == 0 {
                    return Err("缺失或为空".into());
                }
                let mut source = File::open(&absolute).map_err(|e| e.to_string())?;
                let digest = crate::file::hash_reader(&mut source)
                    .map_err(|e| format!("无法完整读取：{e}"))?;
                let after = source.metadata().map_err(|e| e.to_string())?;
                if metadata.len() != digest.size_bytes
                    || after.len() != digest.size_bytes
                    || metadata.modified().ok() != after.modified().ok()
                {
                    return Err("计算指纹时文件已变化".into());
                }
                Ok(FileReference {
                    path: file.into(),
                    bytes: digest.size_bytes,
                    sha256: digest.sha256,
                })
            })();
            match evidence {
                Ok(reference) => {
                    result.item(
                        file,
                        format!(
                            "第 {} 行 · {} 字节 · SHA-256 {}",
                            i + 1,
                            reference.bytes,
                            reference.sha256
                        ),
                        true,
                    );
                    result.dependencies.push(reference);
                }
                Err(detail) => {
                    result.item(file, format!("第 {} 行 · {detail}", i + 1), false);
                    result
                        .blockers
                        .push(format!("第 {} 行 · {file} · {detail}", i + 1));
                }
            }
        }
    }
}
fn script_words(s: &str) -> Option<Vec<String>> {
    let mut result = Vec::new();
    let mut token = String::new();
    let mut quote = None;
    for c in s.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => token.push(c),
            None if c == '"' => quote = Some(c),
            None if c == '#' && token.is_empty() => break,
            None if c.is_whitespace() => {
                if !token.is_empty() {
                    result.push(std::mem::take(&mut token));
                }
            }
            None => token.push(c),
        }
    }
    if !token.is_empty() {
        result.push(token);
    }
    quote.is_none().then_some(result)
}

impl VendorSetup {
    pub(super) fn check_evidence(&self, report: &Inspection) -> Result<(), String> {
        if self.route == "nxp-uuu" && self.dependencies.as_ref() != Some(&report.dependencies) {
            return Err("UUU 引用文件的指纹与确认时不符，请重新分析".into());
        }
        Ok(())
    }
    pub(super) fn validate_dfu(
        &self,
        device: &DeviceConfig,
        info: &Inspection,
        targets: &[FlasherDeviceCandidate],
    ) -> Result<(), String> {
        let alt = self.alt.ok_or("请选择设备实际列出的 Alternate")?;
        if self.target_key.as_deref() != Some(device.usb_binding.as_str()) {
            return Err("DFU 目标已变更，请重新选择 Alternate".into());
        }
        let mut found = targets
            .iter()
            .filter(|u| u.key == device.usb_binding && u.flasher_kind == "dfu-util");
        let target = found.next().ok_or("绑定的 DFU 设备已断开")?;
        if found.next().is_some()
            || target
                .dfu_alternates
                .iter()
                .filter(|a| a.number == alt)
                .count()
                != 1
        {
            return Err("所选 Alternate 未在当前唯一 DFU 设备上得到确认".into());
        }
        if self.route == "st-dfuse" && (info.alternates.len() != 1 || info.alternates[0] != alt) {
            return Err("单次 dfu-util 下载仅支持与所选 Alternate 相符的单目标包".into());
        }
        Ok(())
    }
    pub(super) fn arguments(
        &self,
        device: &DeviceConfig,
        options: &LocalFlashOptions,
    ) -> Result<Option<Vec<String>>, String> {
        let expected = match self.route.as_str() {
            "rk-update" | "rk-emmc" => ("rockchip", "openrockcli"),
            "st-dfuse" | "st-raw" => ("st", "dfu-util"),
            "nxp-uuu" | "nxp-configured" => ("nxp", "uuu"),
            "k230-raw" | "k230-kdimage" => ("kendryte", "k230"),
            _ => return Err("未知厂商路线".into()),
        };
        let raw_rk = self.route == "rk-emmc";
        if options.system_type != if raw_rk { "mainline" } else { "vendor" }
            || (raw_rk && options.mainline_route.as_deref() != Some("rockchip_raw_emmc"))
            || (self.medium.is_some() && expected.0 != "kendryte")
            || (self.alt.is_some() && expected.0 != "st")
            || (self.address.is_some() && self.route != "st-raw")
            || (self.dependencies.is_some() && self.route != "nxp-uuu")
            || (self.erase_confirmed.is_some() && !raw_rk)
        {
            return Err("输入选项与当前烧录路线不匹配".into());
        }
        if (device.chip_vendor.as_str(), device.flasher_kind.as_str()) != expected {
            return Err("路线与所选单板的烧录器不匹配".into());
        }
        if matches!(self.route.as_str(), "rk-emmc" | "k230-kdimage" | "k230-raw")
            && (device.usb_binding.is_empty()
                || self.target_key.as_deref() != Some(device.usb_binding.as_str()))
        {
            return Err("目标与所选 USB 绑定不符，请重新核对".into());
        }
        if self.route == "rk-emmc" && self.erase_confirmed != Some(true) {
            return Err("尚未确认覆盖所选 eMMC 用户区".into());
        }
        if self.route != "rk-emmc"
            && (options.loader_path.is_some() || options.mainline_route.is_some())
        {
            return Err("该路线不接受独立 Loader".into());
        }
        if expected.0 != "rockchip"
            && (options.mode.is_some()
                || options.verify.is_some()
                || options.post_flash_action.is_some())
        {
            return Err("当前路线使用专用烧录选项".into());
        }
        let attrs = match self.route.as_str() {
            "rk-update" => None,
            "rk-emmc" => Some(json!({"raw":true,"loaderPath":options.loader_path,"medium":"emmc"})),
            "nxp-uuu" => return Ok(Some(Vec::new())),
            "nxp-configured" => {
                let args = parse_extra_args(&device.flasher_args)?;
                if !matches!(args.as_slice(), [option,recipe] if option=="-b" && matches!(recipe.as_str(),"emmc"|"sd"|"emmc_all"|"sd_all"|"spl"|"qspi"|"nand"|"nvme_all"|"fspinand"))
                {
                    return Err(
                        "请选择与单个输入文件匹配的 UUU 内置 -b 配方；自定义脚本请使用脚本路线"
                            .into(),
                    );
                }
                return Ok(Some(args));
            }
            "st-dfuse" | "st-raw" => {
                let alt = self.alt.ok_or("请选择设备实际列出的 Alternate")?;
                let address = if self.route == "st-raw" {
                    let addr = self.address.as_deref().unwrap_or("");
                    let hex = addr
                        .strip_prefix("0x")
                        .or_else(|| addr.strip_prefix("0X"))
                        .ok_or("RAW 地址须为 0x 开头的十六进制值")?;
                    if hex.is_empty()
                        || hex.len() > 8
                        || !hex.bytes().all(|b| b.is_ascii_hexdigit())
                    {
                        return Err("RAW 地址超出 32 位范围".into());
                    }
                    addr
                } else {
                    ""
                };
                Some(json!({"alt":alt.to_string(),"address":address}))
            }
            "k230-kdimage" | "k230-raw" => {
                let medium = self.medium.as_deref().unwrap_or("");
                if !matches!(medium, "sdcard" | "emmc" | "spi_nor" | "spi_nand") {
                    return Err("请选择板卡实际安装的 K230 存储介质".into());
                }
                Some(json!({"medium":medium}))
            }
            _ => unreachable!(),
        };
        adapter_override_args(
            device,
            AdapterFlashOptions {
                mode: options.mode.clone(),
                verify: options.verify,
                post_flash_action: options.post_flash_action.clone(),
                vendor_options: attrs,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    fn tempdir() -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("imager-vendor-{:016x}", rand::random::<u64>()));
        fs::create_dir(&path).unwrap();
        path
    }
    #[test]
    fn checks_uuu_dependencies_relative_to_script_with_line_numbers() {
        let dir = tempdir();
        fs::write(dir.join("boot blob.bin"), b"payload").unwrap();
        let path = dir.join("flash.uuu");
        fs::write(
            &path,
            "uuu_version 1.5.0\nSDPS: boot -f \"boot blob.bin\"\nFB: flash -raw2sparse all os.wic",
        )
        .unwrap();
        let r = inspect("nxp-uuu", &path).unwrap();
        assert_eq!(r.items.len(), 2);
        assert_eq!(r.blockers.len(), 1);
        assert!(r.blockers[0].contains("第 3 行"));
        fs::write(dir.join("os.wic"), b"image").unwrap();
        let before = inspect("nxp-uuu", &path).unwrap();
        assert!(before.blockers.is_empty());
        fs::write(dir.join("os.wic"), b"other").unwrap();
        let after = inspect("nxp-uuu", &path).unwrap();
        assert_eq!(before.dependencies[1].bytes, after.dependencies[1].bytes);
        assert_ne!(before.dependencies[1].sha256, after.dependencies[1].sha256);
        fs::write(&path, "uuu_version 1.5.0\nFB: shell").unwrap();
        assert!(!inspect("nxp-uuu", &path).unwrap().blockers.is_empty());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn refuses_vendor_containers_as_raw_and_rejects_oversized_partition_table() {
        let dir = tempdir();
        let p = dir.join("system.img");
        let mut data = vec![0; 512];
        data[..4].copy_from_slice(b"RKFW");
        fs::write(&p, &data).unwrap();
        assert!(inspect("rk-update", &p).unwrap().blockers.is_empty());
        assert!(!inspect("rk-emmc", &p).unwrap().blockers.is_empty());
        data[..4].copy_from_slice(&KDIMG_HEADER_MAGIC.to_le_bytes());
        data[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        fs::write(&p, &data).unwrap();
        assert!(!inspect("k230-raw", &p).unwrap().blockers.is_empty());
        assert!(!inspect("k230-kdimage", &p).unwrap().blockers.is_empty());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn dfuse_target_and_element_come_from_container() {
        let dir = tempdir();
        let p = dir.join("device.dfu");
        let mut d = vec![0u8; 11 + 274 + 8 + 4 + 16];
        d[0..5].copy_from_slice(b"DfuSe");
        d[5] = 1;
        d[6..10].copy_from_slice(&297u32.to_le_bytes());
        d[10] = 1;
        d[11..17].copy_from_slice(b"Target");
        d[17] = 4;
        d[277..281].copy_from_slice(&12u32.to_le_bytes());
        d[281..285].copy_from_slice(&1u32.to_le_bytes());
        d[285..289].copy_from_slice(&0x1234u32.to_le_bytes());
        d[289..293].copy_from_slice(&4u32.to_le_bytes());
        d[305..308].copy_from_slice(b"UFD");
        d[308] = 16;
        fs::write(&p, &d).unwrap();
        let i = inspect("st-dfuse", &p).unwrap();
        assert!(i.blockers.is_empty());
        assert_eq!(i.alternates, vec![4]);
        assert!(i.items[0].detail.contains("0x00001234"));
        d[289..293].copy_from_slice(&100u32.to_le_bytes());
        fs::write(&p, &d).unwrap();
        assert!(!inspect("st-dfuse", &p).unwrap().blockers.is_empty());
        fs::remove_dir_all(dir).unwrap();
    }
}
