use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};
use tauri::Manager;
const HASH: &str = "bd7d13b36df2bc0d5e7b41af97de9240ee22f271590e7c49b2e9168d5eae4cf6";
const DATA: &[u8] = include_bytes!("../vendor/board-loaders/dshanpi-a1.bin.gz");
fn install(root: &Path) -> Result<PathBuf, String> {
    let dir = root.join("loaders").join("dshanpi-a1").join(HASH);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("MiniLoaderAll.bin");
    if path.metadata().is_ok_and(|m| m.len() == 754169)
        && std::fs::read(&path).is_ok_and(|b| format!("{:x}", Sha256::digest(b)) == HASH)
    {
        return Ok(path);
    }
    let mut buffer = Vec::new();
    GzDecoder::new(DATA)
        .read_to_end(&mut buffer)
        .map_err(|e| e.to_string())?;
    if format!("{:x}", Sha256::digest(&buffer)) != HASH {
        return Err("内置 Loader 校验失败".into());
    }
    let mut tmp = tempfile::NamedTempFile::new_in(&dir).map_err(|e| e.to_string())?;
    tmp.write_all(&buffer).map_err(|e| e.to_string())?;
    tmp.persist(&path).map_err(|e| e.to_string())?;
    Ok(path)
}

#[tauri::command]
pub async fn imager_board_loader(app: tauri::AppHandle, board: String) -> Result<String, String> {
    if board != "DshanPI-A1" {
        return Err("当前板型没有内置 Loader".into());
    }
    let dir = app.path().app_local_data_dir().map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        install(&dir).map(|p| p.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shipped_loader_matches_board_package_and_recovers_damaged_cache() {
        let root = tempfile::tempdir().unwrap();
        let p = install(root.path()).unwrap();
        let bytes = std::fs::read(&p).unwrap();
        assert_eq!(&bytes[..4], b"LDR ");
        assert_eq!(&bytes[21..25], b"6753");
        std::fs::write(&p, vec![0; bytes.len()]).unwrap();
        assert_eq!(std::fs::read(install(root.path()).unwrap()).unwrap(), bytes);
    }
}
