use log::{debug, error, warn};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use tauri::command;

const CHUNK_SIZE: usize = 256 * 1024 * 1024;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagerFileIdentity {
    pub size_bytes: u64,
    pub sha256: String,
}

pub(crate) fn hash_reader(reader: &mut impl Read) -> std::io::Result<ImagerFileIdentity> {
    let mut hasher = Sha256::new();
    let size_bytes = std::io::copy(reader, &mut hasher)?;
    Ok(ImagerFileIdentity {
        size_bytes,
        sha256: hex::encode(hasher.finalize()),
    })
}

#[command]
pub async fn imager_file_identity(path: String) -> Result<ImagerFileIdentity, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let metadata = std::fs::metadata(&path).map_err(|e| e.to_string())?;
        if !metadata.is_file() {
            return Err("请选择普通镜像文件".to_string());
        }
        let mut file = File::open(path).map_err(|e| e.to_string())?;
        let before = file.metadata().map_err(|e| e.to_string())?;
        if !before.is_file() {
            return Err("请选择普通镜像文件".to_string());
        }
        let identity = hash_reader(&mut file).map_err(|e| e.to_string())?;
        let after = file.metadata().map_err(|e| e.to_string())?;
        if identity.size_bytes != before.len() || identity.size_bytes != after.len()
            || before.modified().ok() != after.modified().ok()
        {
            return Err("文件读取期间发生变化，请重新选择".to_string());
        }
        Ok(identity)
    }).await.map_err(|e| e.to_string())?
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ExtractResult {
    pub success: bool,
    pub message: String,
    pub bytes_written: u64,
}

#[command]
pub fn extract_file_chunked(
    source_path: String,
    dest_path: String,
    offset: u64,
    length: u64,
) -> Result<ExtractResult, String> {
    debug!(
        "extract_file_chunked called: source={}, dest={}, offset={}, length={}",
        source_path, dest_path, offset, length
    );

    let source = PathBuf::from(&source_path);
    let dest = PathBuf::from(&dest_path);

    if !source.exists() {
        error!("Source file does not exist: {}", source_path);
        return Err(format!("Source file does not exist: {}", source_path));
    }

    let mut source_file = File::open(&source).map_err(|e| {
        error!("Failed to open source file: {}", e);
        format!("Failed to open source file: {}", e)
    })?;

    let mut dest_file = File::create(&dest).map_err(|e| {
        error!("Failed to create destination file: {}", e);
        format!("Failed to create destination file: {}", e)
    })?;

    source_file.seek(SeekFrom::Start(offset)).map_err(|e| {
        error!("Failed to seek to offset {}: {}", offset, e);
        format!("Failed to seek to offset {}: {}", offset, e)
    })?;

    debug!(
        "Starting file extraction with chunk size: {} bytes",
        CHUNK_SIZE
    );

    let mut buffer = vec![0u8; CHUNK_SIZE];
    let mut remaining = length as usize;
    let mut total_written: u64 = 0;

    while remaining > 0 {
        let read_size = std::cmp::min(remaining, CHUNK_SIZE);
        let read_buffer = &mut buffer[..read_size];

        let bytes_read = source_file.read(read_buffer).map_err(|e| {
            error!("Failed to read from source: {}", e);
            format!("Failed to read from source: {}", e)
        })?;

        if bytes_read == 0 {
            warn!("Unexpected end of file at offset {}", total_written);
            break;
        }

        dest_file
            .write_all(&read_buffer[..bytes_read])
            .map_err(|e| {
                error!("Failed to write to destination: {}", e);
                format!("Failed to write to destination: {}", e)
            })?;

        remaining -= bytes_read;
        total_written += bytes_read as u64;
    }

    dest_file.flush().map_err(|e| {
        error!("Failed to flush destination file: {}", e);
        format!("Failed to flush destination file: {}", e)
    })?;

    debug!("Successfully extracted {} bytes", total_written);

    Ok(ExtractResult {
        success: true,
        message: format!("Successfully extracted {} bytes", total_written),
        bytes_written: total_written,
    })
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct FileExtractOptions {
    pub source_path: String,
    pub dest_path: String,
    pub offset: u64,
    pub length: u64,
}

#[command]
pub fn extract_files_batch(files: Vec<FileExtractOptions>) -> Result<Vec<ExtractResult>, String> {
    debug!("extract_files_batch called with {} files", files.len());

    let mut results = Vec::with_capacity(files.len());

    for (index, file_opt) in files.into_iter().enumerate() {
        debug!(
            "Processing file {}/{}: {} -> {}",
            index + 1,
            results.capacity(),
            file_opt.source_path,
            file_opt.dest_path
        );

        let result = extract_file_chunked(
            file_opt.source_path,
            file_opt.dest_path,
            file_opt.offset,
            file_opt.length,
        )?;
        results.push(result);
    }

    debug!(
        "Batch extraction completed: {} files processed",
        results.len()
    );

    Ok(results)
}

#[command]
pub fn get_file_size(path: String) -> Result<u64, String> {
    debug!("get_file_size called for: {}", path);

    let path = PathBuf::from(&path);

    if !path.exists() {
        error!("File does not exist: {}", path.display());
        return Err(format!("File does not exist: {}", path.display()));
    }

    let metadata = std::fs::metadata(&path).map_err(|e| {
        error!("Failed to get file metadata: {}", e);
        format!("Failed to get file metadata: {}", e)
    })?;

    let size = metadata.len();
    debug!("File size: {} bytes", size);

    Ok(size)
}
