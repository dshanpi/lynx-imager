//! Host SD reader: authorize one enumerated removable disk, write the expanded
//! image from byte zero and verify its exact written range from the device.
mod host;
mod worker;

use flate2::read::MultiGzDecoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};
use tauri::{AppHandle, Manager};
use xz2::read::XzDecoder;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Card {
    key: String,
    name: String,
    bytes: u64,
    sector: u64,
    identity: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Image {
    path: String,
    bytes: u64,
    sha256: String,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Request {
    card: Card,
    image: Image,
}

fn source(path: &str) -> Result<Box<dyn Read>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("请选择普通镜像文件".into());
    }
    let name = path.to_ascii_lowercase();
    if name.ends_with(".img.gz") || name.ends_with(".wic.gz") {
        Ok(Box::new(MultiGzDecoder::new(file)))
    } else if name.ends_with(".img.xz") || name.ends_with(".wic.xz") {
        Ok(Box::new(XzDecoder::new_multi_decoder(file)))
    } else if name.ends_with(".img") || name.ends_with(".raw") || name.ends_with(".wic") {
        Ok(Box::new(file))
    } else {
        Err("请选择 .img、.raw、.wic 或对应的 .img/.wic 压缩整盘镜像".into())
    }
}

fn inspect(path: String) -> Result<Image, String> {
    let mut reader = source(&path)?;
    let mut hash = Sha256::new();
    let mut bytes = 0u64;
    let mut head = Vec::new();
    let mut buf = vec![0; 1024 * 1024];
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        if head.len() < 512 {
            head.extend_from_slice(&buf[..n.min(512 - head.len())]);
        }
        bytes += n as u64;
        hash.update(&buf[..n]);
    }
    if bytes < 512 || bytes % 512 != 0 {
        return Err("镜像长度须为完整的 512 字节扇区".into());
    }
    if [
        b"RKFW".as_slice(),
        b"RKAF",
        b"IMAGEWTY",
        b"ANDROID!",
        b"\x3a\xff\x26\xed",
    ]
    .iter()
    .any(|sig| head.starts_with(sig))
    {
        return Err("所选文件是升级包或分区格式，请选择板卡的 SD 整盘镜像。".into());
    }
    Ok(Image {
        path,
        bytes,
        sha256: format!("{:x}", hash.finalize()),
    })
}

fn fits(card: &Card, image: &Image) -> Result<(), String> {
    if !matches!(card.sector, 512 | 1024 | 2048 | 4096)
        || image.bytes == 0
        || image.bytes > card.bytes
        || image.bytes % card.sector != 0
    {
        return Err("镜像大小超出卡容量或与逻辑扇区不对齐".into());
    }
    Ok(())
}

// Use full blocks even when a decompressor returns short reads.
fn transfer(
    mut input: impl Read,
    disk: &mut (impl Read + Write + Seek),
    image: &Image,
    mut sync: impl FnMut() -> std::io::Result<()>,
    mut progress: impl FnMut(&str, u64),
) -> Result<(), String> {
    let mut buf = vec![0; 1024 * 1024];
    let mut hash = Sha256::new();
    let mut done = 0;
    disk.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    while done < image.bytes {
        let n = (image.bytes - done).min(buf.len() as u64) as usize;
        input.read_exact(&mut buf[..n]).map_err(|e| e.to_string())?;
        disk.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        hash.update(&buf[..n]);
        done += n as u64;
        progress("写入", done);
    }
    if input.read(&mut buf[..1]).map_err(|e| e.to_string())? != 0
        || format!("{:x}", hash.finalize()) != image.sha256
    {
        return Err("写入期间源镜像发生变化，请重新选取镜像".into());
    }
    disk.flush()
        .and_then(|_| sync())
        .map_err(|e| e.to_string())?;
    disk.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    done = 0;
    while done < image.bytes {
        let n = (image.bytes - done).min(buf.len() as u64) as usize;
        disk.read_exact(&mut buf[..n]).map_err(|e| e.to_string())?;
        hash.update(&buf[..n]);
        done += n as u64;
        progress("回读校验", done);
    }
    if format!("{:x}", hash.finalize()) != image.sha256 {
        return Err("写卡后的 SHA-256 与镜像不一致".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn imager_card_list() -> Result<Vec<Card>, String> {
    tauri::async_runtime::spawn_blocking(host::list)
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn imager_card_image(path: String) -> Result<Image, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let p = Path::new(&path).canonicalize().map_err(|e| e.to_string())?;
        inspect(p.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn imager_card_write(app: AppHandle, card: Card, image: Image) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = app
            .state::<crate::hardware_safety::HardwareSafetyState>()
            .try_begin("Imager", "TF / SD 写卡", card.key.clone())?;
        fits(&card, &image)?;
        if !host::list()?.contains(&card) {
            return Err("读卡器或卡片已更换，请重新选择".into());
        }
        worker::launch(&app, Request { card, image })
    })
    .await
    .map_err(|e| e.to_string())?
}
pub use worker::entry;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    #[test]
    fn simulated_media_writes_full_image_and_verifies_readback() {
        let data = vec![137u8; 1024 * 1024 + 512];
        let image = Image {
            path: String::new(),
            bytes: data.len() as u64,
            sha256: format!("{:x}", Sha256::digest(&data)),
        };
        let mut disk = Cursor::new(vec![77; data.len() + 512]);
        transfer(&data[..], &mut disk, &image, || Ok(()), |_, _| {}).unwrap();
        assert_eq!(&disk.get_ref()[..data.len()], &data);
        assert_eq!(disk.get_ref()[data.len()], 77);
        assert!(transfer(&data[..256], &mut disk, &image, || Ok(()), |_, _| {}).is_err());
    }
    #[test]
    fn mismatched_readback_and_insufficient_media_are_rejected() {
        struct BadRead(Cursor<Vec<u8>>);
        impl Read for BadRead {
            fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
                let n = self.0.read(b)?;
                if n > 0 {
                    b[0] ^= 1;
                }
                Ok(n)
            }
        }
        impl Write for BadRead {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.0.write(b)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        impl Seek for BadRead {
            fn seek(&mut self, p: SeekFrom) -> std::io::Result<u64> {
                self.0.seek(p)
            }
        }
        let data = [1u8; 1024];
        let img = Image {
            path: String::new(),
            bytes: 1024,
            sha256: format!("{:x}", Sha256::digest(data)),
        };
        let small = Card {
            key: "test".into(),
            name: "test".into(),
            identity: "test".into(),
            bytes: 512,
            sector: 512,
        };
        assert!(fits(&small, &img).is_err());
        let mut disk = BadRead(Cursor::new(vec![0; 1024]));
        assert!(transfer(&data[..], &mut disk, &img, || Ok(()), |_, _| {})
            .unwrap_err()
            .contains("SHA-256"));
    }
    #[test]
    fn unwraps_gzip_and_rejects_vendor_update() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("disk.img.gz");
        let mut gz = flate2::write::GzEncoder::new(File::create(&p).unwrap(), Default::default());
        gz.write_all(&[0x42; 1024]).unwrap();
        gz.finish().unwrap();
        assert_eq!(inspect(p.to_str().unwrap().into()).unwrap().bytes, 1024);
        let p = dir.path().join("rk.img");
        let mut d = [0; 512];
        d[..4].copy_from_slice(b"RKFW");
        std::fs::write(&p, d).unwrap();
        assert!(inspect(p.to_str().unwrap().into()).is_err());
    }
}
