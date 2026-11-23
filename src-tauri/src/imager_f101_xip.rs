use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tauri::Manager;

// Layout and boot image from F101-zephyr / YuzukiNeko's mkimage.py build.
const APP_OFFSET: usize = 0x80000;
const FLASH_BYTES: usize = 16 * 1024 * 1024;
const SECTOR: usize = 4096;
const BOOT_BYTES: usize = 40960;
const BOOT_SHA256: &str = "135cb3c2eedf93a9d2d983b7394cfe12120c1e9d27c74fea85bf03762030f044";

fn read_prefix(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("请选择普通文件。".into());
    }
    let mut bytes = Vec::new();
    file.take(limit as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    Ok(bytes)
}

fn assemble(app: &[u8], boot: &[u8]) -> Result<Vec<u8>, String> {
    if app.is_empty() || app.len() > FLASH_BYTES - APP_OFFSET {
        return Err("应用为空或超出 16 MiB SPI NOR 的应用区。".into());
    }
    if app.starts_with(b"IMAGEWTY")
        || app.starts_with(b"\x7fELF")
        || app.get(4..12) == Some(b"eGON.BT0")
    {
        return Err("这里需要 Zephyr 原始应用 BIN，请检查文件类型。".into());
    }
    let mut image = vec![0xff; APP_OFFSET + app.len().div_ceil(SECTOR) * SECTOR];
    image[..boot.len()].copy_from_slice(boot);
    image[APP_OFFSET..APP_OFFSET + app.len()].copy_from_slice(app);
    Ok(image)
}

fn prepare(
    cache: &Path,
    app: &Path,
    expected_sha: &str,
    boot_source: &Path,
) -> Result<PathBuf, String> {
    let boot = read_prefix(boot_source, BOOT_BYTES)?;
    if hex::encode(Sha256::digest(&boot)) != BOOT_SHA256 {
        return Err("未找到此 F101 XIP 配置的启动段。请选择工程配套的 xip-boot_spi.bin，或以该启动段开头的整片镜像。".into());
    }
    let app = read_prefix(app, FLASH_BYTES - APP_OFFSET + 1)?;
    if hex::encode(Sha256::digest(&app)) != expected_sha {
        return Err("应用文件已变化或超出容量，请重新选择。".into());
    }
    let image = assemble(&app, &boot)?;
    let digest = hex::encode(Sha256::digest(&image));
    let dir = cache.join("f101-xip").join(digest);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("zephyr-f101-full.img");
    let mut temp = tempfile::NamedTempFile::new_in(dir).map_err(|e| e.to_string())?;
    temp.write_all(&image).map_err(|e| e.to_string())?;
    temp.flush().map_err(|e| e.to_string())?;
    // Independent preparations of the same image may finish together.
    if let Err(e) = temp.persist_noclobber(&path) {
        if e.error.kind() != std::io::ErrorKind::AlreadyExists {
            return Err(e.error.to_string());
        }
        if read_prefix(&path, image.len() + 1)? != image {
            return Err("已缓存镜像与生成内容不符，请清理对应缓存后重试。".into());
        }
    }
    Ok(path)
}

#[tauri::command]
pub async fn imager_prepare_f101_xip(
    app: tauri::AppHandle,
    application_path: String,
    application_sha256: String,
    boot_source_path: String,
) -> Result<String, String> {
    let cache = app.path().app_cache_dir().map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        prepare(
            &cache,
            Path::new(&application_path),
            &application_sha256,
            Path::new(&boot_source_path),
        )
        .map(|path| path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f101_app_offset_padding_and_limit() {
        let app = vec![0xab; SECTOR + 1];
        let image = assemble(&app, &[1, 2]).unwrap();
        assert_eq!(image.len(), APP_OFFSET + 2 * SECTOR);
        assert_eq!(&image[..2], &[1, 2]);
        assert!(image[2..APP_OFFSET].iter().all(|&v| v == 0xff));
        assert_eq!(&image[APP_OFFSET..APP_OFFSET + app.len()], &app);
        assert!(image[APP_OFFSET + app.len()..].iter().all(|&v| v == 0xff));
        assert!(assemble(&[], &[1]).is_err());
        assert!(assemble(&vec![0; FLASH_BYTES - APP_OFFSET + 1], &[1]).is_err());
        assert!(assemble(b"IMAGEWTY", &[1]).is_err());
    }
}
