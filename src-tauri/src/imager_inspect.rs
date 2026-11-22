use crate::{file::hash_reader, firmware::image::parse_open_image};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageEntry {
    name: String,
    main_type: String,
    subtype: String,
    size_bytes: u64,
    stored_bytes: u64,
    offset: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageInspection {
    size_bytes: u64,
    sha256: String,
    known_f101_loader: bool,
    header_hex: String,
    app_hex: String,
    egon_at_zero: bool,
    boot0_bytes: Option<u64>,
    spl_at8192: bool,
    container_version: Option<u32>,
    entries: Vec<ImageEntry>,
    layout: Option<crate::imager_layout::Layout>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageReference {
    document_sha256: String,
    sha256: String,
    size_bytes: Option<u64>,
    application_offset: Option<u64>,
    parameters_offset: Option<u64>,
    application_bytes: Option<u64>,
    board_model: Option<String>,
    file_name: Option<String>,
}

fn regular_file(path: &Path) -> Result<File, String> {
    if !path.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("请选择普通文件".into());
    }
    let f = File::open(path).map_err(|e| e.to_string())?;
    if !f.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("请选择普通文件".into());
    }
    Ok(f)
}

fn sample(f: &mut File, offset: u64, len: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    f.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
    f.take(len)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    Ok(bytes)
}

fn inspect(path: &Path) -> Result<ImageInspection, String> {
    let mut f = regular_file(path)?;
    let before = f.metadata().map_err(|e| e.to_string())?;
    let head = sample(&mut f, 0, 64)?;
    if head.is_empty() {
        return Err("文件为空".into());
    }
    let spl = sample(&mut f, 8192, 24)?;
    let app = sample(&mut f, 0x80000, 16)?;
    let mut result = ImageInspection {
        size_bytes: before.len(),
        sha256: String::new(),
        known_f101_loader: false,
        header_hex: hex::encode(&head[..head.len().min(16)]),
        app_hex: hex::encode(app),
        boot0_bytes: crate::imager_layout::boot0_bytes(&mut f, &head, before.len()),
        egon_at_zero: head.get(4..12) == Some(b"eGON.BT0"),
        spl_at8192: spl.get(4..12) == Some(b"eGON.BT0") && spl.get(20..23) == Some(b"SPL"),
        container_version: None,
        entries: Vec::new(),
        layout: None,
    };
    if head.starts_with(b"IMAGEWTY") {
        let version = head.get(8..12).ok_or("IMAGEWTY 文件头截断")?;
        let count_offset = match u32::from_le_bytes(version.try_into().unwrap()) {
            0x100 => 56,
            0x300 => 60,
            _ => return Err("IMAGEWTY 容器版本尚不支持".into()),
        };
        let count = head
            .get(count_offset..count_offset + 4)
            .ok_or("IMAGEWTY 文件头截断")?;
        let count = u32::from_le_bytes(count.try_into().unwrap());
        if count == 0 || count > 4096 {
            return Err("IMAGEWTY 目录条目数量无效或超出分析范围".into());
        }
        let info = parse_open_image(&mut f)?
            .image_info
            .ok_or("无法解析 IMAGEWTY 目录")?;
        result.container_version = Some(info.header.header_version >> 8);
        for (index, item) in info.files.into_iter().enumerate() {
            // IMAGEWTY v3 stores lengths and offsets as three little-endian u64s.
            // The existing firmware editor DTO exposes their low words for compatibility.
            let (stored, original, offset) = if info.header.header_version == 0x0300 {
                let fields = sample(&mut f, (index as u64 + 1) * 1024 + 292, 24)?;
                if fields.len() != 24 {
                    return Err("IMAGEWTY 目录截断".into());
                }
                (
                    u64::from_le_bytes(fields[0..8].try_into().unwrap()),
                    u64::from_le_bytes(fields[8..16].try_into().unwrap()),
                    u64::from_le_bytes(fields[16..24].try_into().unwrap()),
                )
            } else {
                (
                    u64::from(item.stored_length),
                    u64::from(item.original_length),
                    u64::from(item.offset),
                )
            };
            if stored.max(original) > 0
                && (offset < (u64::from(count) + 1) * 1024
                    || offset > before.len()
                    || stored.max(original) > before.len() - offset)
            {
                return Err(format!("IMAGEWTY 条目范围无效：{}", item.filename));
            }
            if item.filename.eq_ignore_ascii_case("sunxi_mbr.fex") && stored.min(original) >= 16384 {
                let content = sample(&mut f, offset, 16384)?;
                result.layout = crate::imager_layout::sunxi(&content);
            }
            result.entries.push(ImageEntry {
                stored_bytes: stored,
                name: item.filename,
                main_type: item.maintype.trim().to_owned(),
                subtype: item.subtype.trim().to_owned(),
                offset,
                size_bytes: original,
            });
        }
    }
    if result.container_version.is_none() { result.layout = crate::imager_layout::disk(&mut f, before.len()); }
    f.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let digest = hash_reader(&mut f).map_err(|e| e.to_string())?;
    let after = f.metadata().map_err(|e| e.to_string())?;
    if digest.size_bytes != before.len()
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
    {
        return Err("文件在分析期间发生变化，请重新选择".into());
    }
    result.known_f101_loader = digest.sha256 == crate::workbench::firmware_lifecycle::F101_LOADER_SHA;
    result.sha256 = digest.sha256;
    Ok(result)
}

fn unsigned(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        value
            .as_str()
            .and_then(|s| s.strip_prefix("0x"))
            .and_then(|s| u64::from_str_radix(s, 16).ok())
    })
}

fn optional_number(value: Option<&Value>, field: &str) -> Result<Option<u64>, String> {
    value
        .map(|v| unsigned(v).ok_or_else(|| format!("清单字段 {field} 不是有效的非负整数")))
        .transpose()
}

fn read_reference(path: &Path) -> Result<ImageReference, String> {
    let f = regular_file(path)?;
    let mut bytes = Vec::new();
    f.take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 1024 * 1024 {
        return Err("清单超过 1 MiB".into());
    }
    let doc: Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("无法读取 JSON 清单：{e}"))?;
    let img = doc.get("image").unwrap_or(&doc);
    let hash = img["sha256"]
        .as_str()
        .filter(|s| s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit()))
        .ok_or("缺少系统镜像的 SHA-256 参考值")?;
    Ok(ImageReference {
        document_sha256: hex::encode(Sha256::digest(&bytes)),
        sha256: hash.to_ascii_lowercase(),
        size_bytes: optional_number(
            img.get("sizeBytes").or_else(|| doc.get("image_bytes")),
            "镜像大小",
        )?,
        parameters_offset: doc.pointer("/layout/parametersOffset").and_then(unsigned),
        application_offset: optional_number(
            doc.pointer("/layout/applicationOffset")
                .or_else(|| doc.get("physical_app_offset")),
            "应用偏移",
        )?,
        application_bytes: optional_number(doc.get("app_bytes"), "应用大小")?,
        board_model: doc["boardModel"].as_str().map(str::to_owned),
        file_name: img["fileName"].as_str().map(str::to_owned),
    })
}

#[tauri::command]
pub async fn imager_inspect_image(path: String) -> Result<ImageInspection, String> {
    tauri::async_runtime::spawn_blocking(move || inspect(Path::new(&path)))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn imager_read_image_reference(path: String) -> Result<ImageReference, String> {
    tauri::async_runtime::spawn_blocking(move || read_reference(Path::new(&path)))
        .await
        .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppComparison {
    matches: bool,
    compared_bytes: u64,
    application_sha256: String,
    sample_hex: String,
}

fn compare_application(
    image_path: &Path,
    application_path: &Path,
    offset: u64,
    expected_bytes: Option<u64>,
) -> Result<AppComparison, String> {
    let mut image = regular_file(image_path)?;
    let mut application = regular_file(application_path)?;
    let image_before = image.metadata().map_err(|e| e.to_string())?;
    let app_before = application.metadata().map_err(|e| e.to_string())?;
    let length = app_before.len();
    if length == 0
        || expected_bytes.is_some_and(|n| n != length)
        || offset > image_before.len()
        || length > image_before.len() - offset
    {
        return Err("应用文件大小与清单或系统镜像中的应用区范围不符".into());
    }
    image
        .seek(SeekFrom::Start(offset))
        .map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut remaining = length;
    let mut matches = true;
    let mut sample_hex = String::new();
    let (mut a, mut b) = (vec![0u8; 64 * 1024], vec![0u8; 64 * 1024]);
    while remaining > 0 {
        let count = remaining.min(a.len() as u64) as usize;
        image
            .read_exact(&mut a[..count])
            .map_err(|e| e.to_string())?;
        application
            .read_exact(&mut b[..count])
            .map_err(|e| e.to_string())?;
        if remaining == length {
            sample_hex = hex::encode(&b[..count.min(16)]);
        }
        matches &= a[..count] == b[..count];
        hash.update(&b[..count]);
        remaining -= count as u64;
    }
    for (file, before) in [(&image, image_before), (&application, app_before)] {
        let after = file.metadata().map_err(|e| e.to_string())?;
        if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
            return Err("比对期间文件发生变化，请重新选择".into());
        }
    }
    Ok(AppComparison {
        matches,
        compared_bytes: length,
        application_sha256: hex::encode(hash.finalize()),
        sample_hex,
    })
}

#[tauri::command]
pub async fn imager_compare_application(
    image_path: String,
    application_path: String,
    offset: u64,
    expected_bytes: Option<u64>,
) -> Result<AppComparison, String> {
    tauri::async_runtime::spawn_blocking(move || {
        compare_application(
            Path::new(&image_path),
            Path::new(&application_path),
            offset,
            expected_bytes,
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
    };
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new(data: &[u8]) -> Self {
            let path = std::env::temp_dir().join(format!(
                "imager-inspect-{}-{}",
                std::process::id(),
                SEQ.fetch_add(1, Ordering::Relaxed)
            ));
            fs::write(&path, data).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    #[test]
    fn arbitrary_and_raw_files_report_observed_bytes_only() {
        let unrecognized = Fixture::new(b"example");
        let info = inspect(&unrecognized.0).unwrap();
        assert_eq!(info.size_bytes, 7);
        assert_eq!(info.header_hex, "6578616d706c65");
        assert!(!info.egon_at_zero && !info.spl_at8192 && info.container_version.is_none());
        let mut bin = vec![0; 0x80010];
        bin[4..12].copy_from_slice(b"eGON.BT0");
        bin[0x80000..].fill(0x5a);
        let raw = Fixture::new(&bin);
        let result = inspect(&raw.0).unwrap();
        assert!(result.egon_at_zero && !result.spl_at8192);
        assert_eq!(result.app_hex, "5a".repeat(16));
        assert_eq!(result.sha256, hash_reader(&mut &bin[..]).unwrap().sha256);
        bin[0x2004..0x200c].copy_from_slice(b"eGON.BT0");
        bin[0x2014..0x2017].copy_from_slice(b"SPL");
        let emmc = Fixture::new(&bin);
        assert!(inspect(&emmc.0).unwrap().spl_at8192);
    }

    #[test]
    fn independently_compares_selected_application_range() {
        let image = Fixture::new(b"bootabcdefpadding");
        let app = Fixture::new(b"abcdef");
        assert!(
            compare_application(&image.0, &app.0, 4, Some(6))
                .unwrap()
                .matches
        );
        assert!(
            !compare_application(&image.0, &app.0, 5, None)
                .unwrap()
                .matches
        );
        assert!(compare_application(&image.0, &app.0, 4, Some(7)).is_err());
        assert!(compare_application(&image.0, &app.0, u64::MAX, None).is_err());
    }

    fn imagewty() -> Vec<u8> {
        let mut bytes = vec![0u8; 2049];
        bytes[..8].copy_from_slice(b"IMAGEWTY");
        bytes[8..12].copy_from_slice(&0x300u32.to_le_bytes());
        bytes[60..64].copy_from_slice(&1u32.to_le_bytes());
        let file = &mut bytes[1024..2048];
        file[8..16].copy_from_slice(b"FES     ");
        file[16..32].copy_from_slice(b"FES_1-0000000000");
        file[36..44].copy_from_slice(b"fes1.fex");
        file[292..296].copy_from_slice(&1u32.to_le_bytes());
        file[300..304].copy_from_slice(&1u32.to_le_bytes());
        file[308..312].copy_from_slice(&2048u32.to_le_bytes());
        bytes
    }

    #[test]
    fn reads_catalog_entries_and_rejects_corrupt_data_ranges_counts_and_truncation() {
        let mut bytes = imagewty();
        let source = Fixture::new(&bytes);
        let info = inspect(&source.0).unwrap();
        assert_eq!(info.container_version, Some(3));
        assert_eq!(info.entries[0].main_type, "FES");
        assert_eq!(info.entries[0].name, "fes1.fex");
        assert_eq!(info.entries[0].offset, 2048);
        bytes[1024 + 308..1024 + 312].copy_from_slice(&4096u32.to_le_bytes());
        assert!(inspect(&Fixture::new(&bytes).0).is_err());
        bytes[60..64].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(inspect(&Fixture::new(&bytes).0).is_err());
        assert!(inspect(&Fixture::new(b"IMAGEWTY").0).is_err());
        assert!(inspect(&Fixture::new(b"").0).is_err());
    }

    #[test]
    fn accepts_build_and_publication_references_but_rejects_loader_only_or_invalid() {
        let hash = "ab".repeat(32);
        let build=Fixture::new(serde_json::json!({"sha256":hash,"image_bytes":614400,"physical_app_offset":"0x80000","app_bytes":89584}).to_string().as_bytes());
        let r = read_reference(&build.0).unwrap();
        assert_eq!(r.application_offset, Some(0x80000));
        assert_eq!(r.size_bytes, Some(614400));
        let release=Fixture::new(serde_json::json!({"boardModel":"yuzukineko","image":{"fileName":"app.img","sizeBytes":4096,"sha256":hash},"loader":{"sha256":"de".repeat(32)}}).to_string().as_bytes());
        let r = read_reference(&release.0).unwrap();
        assert_eq!(r.sha256, hash);
        assert_eq!(r.file_name.as_deref(), Some("app.img"));
        assert!(read_reference(&Fixture::new(br#"{"loader":{"sha256":"abc"}}"#).0).is_err());
        assert!(read_reference(
            &Fixture::new(
                serde_json::json!({"sha256":hash,"image_bytes":-1})
                    .to_string()
                    .as_bytes()
            )
            .0
        )
        .is_err());
        assert!(read_reference(&Fixture::new(&vec![b' '; 1024 * 1024 + 1]).0).is_err());
    }
}
