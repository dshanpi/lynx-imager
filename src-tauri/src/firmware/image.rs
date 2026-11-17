use crate::firmware::common::{read_packed, string_from_bytes};
use crate::firmware::types::{
    FileHeaderDto, FileHeaderV1Dto, FileHeaderV3Dto, FileInfoDto, ImageHeaderDto, ImageHeaderV1Dto,
    ImageHeaderV3Dto, ImageInfoDto, ParseImageResultDto,
};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

const IMAGEWTY_MAGIC: &[u8; 8] = b"IMAGEWTY";
const IMAGEWTY_FILEHDR_LEN: usize = 1024;
const IMAGEWTY_FHDR_MAINTYPE_LEN: usize = 8;
const IMAGEWTY_FHDR_SUBTYPE_LEN: usize = 16;
const IMAGEWTY_FHDR_FILENAME_LEN: usize = 256;

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct ImageHeaderV1Raw {
    magic: [u8; 8],
    header_version: u32,
    header_size: u32,
    ram_base: u32,
    version: u32,
    image_size: u32,
    image_header_size: u32,
    pid: u32,
    vid: u32,
    hardware_id: u32,
    firmware_id: u32,
    val1: u32,
    val1024: u32,
    num_files: u32,
    val1024_2: u32,
    val0: u32,
    val0_2: u32,
    val0_3: u32,
    val0_4: u32,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct ImageHeaderV3Raw {
    magic: [u8; 8],
    header_version: u32,
    header_size: u32,
    ram_base: u32,
    version: u32,
    image_size: u32,
    image_header_size: u32,
    unknown: u32,
    pid: u32,
    vid: u32,
    hardware_id: u32,
    firmware_id: u32,
    val1: u32,
    val1024: u32,
    num_files: u32,
    val1024_2: u32,
    val0: u32,
    val0_2: u32,
    val0_3: u32,
    val0_4: u32,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct FileHeaderV1Raw {
    filename_len: u32,
    total_header_size: u32,
    maintype: [u8; IMAGEWTY_FHDR_MAINTYPE_LEN],
    subtype: [u8; IMAGEWTY_FHDR_SUBTYPE_LEN],
    unknown_3: u32,
    stored_length: u32,
    original_length: u32,
    offset: u32,
    unknown: u32,
    filename: [u8; IMAGEWTY_FHDR_FILENAME_LEN],
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct FileHeaderV3Raw {
    filename_len: u32,
    total_header_size: u32,
    maintype: [u8; IMAGEWTY_FHDR_MAINTYPE_LEN],
    subtype: [u8; IMAGEWTY_FHDR_SUBTYPE_LEN],
    unknown_0: u32,
    filename: [u8; IMAGEWTY_FHDR_FILENAME_LEN],
    stored_length: u32,
    pad1: u32,
    original_length: u32,
    pad2: u32,
    offset: u32,
}

fn to_image_header_dto(header_bytes: &[u8]) -> Result<ImageHeaderDto, String> {
    if header_bytes.len() < IMAGEWTY_FILEHDR_LEN {
        return Err("image header too small".to_string());
    }
    let magic = string_from_bytes(&header_bytes[..8]);
    let header_version = u32::from_le_bytes(header_bytes[8..12].try_into().unwrap());
    if header_version == 0x0300 {
        let raw: ImageHeaderV3Raw = read_packed(header_bytes)?;
        Ok(ImageHeaderDto {
            magic,
            header_version: raw.header_version,
            header_size: raw.header_size,
            ram_base: raw.ram_base,
            version: raw.version,
            image_size: raw.image_size,
            image_header_size: raw.image_header_size,
            v1: None,
            v3: Some(ImageHeaderV3Dto {
                unknown: raw.unknown,
                pid: raw.pid,
                vid: raw.vid,
                hardware_id: raw.hardware_id,
                firmware_id: raw.firmware_id,
                val1: raw.val1,
                val1024: raw.val1024,
                num_files: raw.num_files,
                val1024_2: raw.val1024_2,
                val0: raw.val0,
                val0_2: raw.val0_2,
                val0_3: raw.val0_3,
                val0_4: raw.val0_4,
            }),
        })
    } else if header_version == 0x0100 {
        let raw: ImageHeaderV1Raw = read_packed(header_bytes)?;
        Ok(ImageHeaderDto {
            magic,
            header_version: raw.header_version,
            header_size: raw.header_size,
            ram_base: raw.ram_base,
            version: raw.version,
            image_size: raw.image_size,
            image_header_size: raw.image_header_size,
            v1: Some(ImageHeaderV1Dto {
                pid: raw.pid,
                vid: raw.vid,
                hardware_id: raw.hardware_id,
                firmware_id: raw.firmware_id,
                val1: raw.val1,
                val1024: raw.val1024,
                num_files: raw.num_files,
                val1024_2: raw.val1024_2,
                val0: raw.val0,
                val0_2: raw.val0_2,
                val0_3: raw.val0_3,
                val0_4: raw.val0_4,
            }),
            v3: None,
        })
    } else {
        Err(format!("unknown image header version: {header_version:#x}"))
    }
}

fn num_files_from_header(header: &ImageHeaderDto) -> u32 {
    header
        .v3
        .as_ref()
        .map(|v| v.num_files)
        .or_else(|| header.v1.as_ref().map(|v| v.num_files))
        .unwrap_or(0)
}

fn to_file_header_dto(bytes: &[u8], header_version: u32) -> Result<FileHeaderDto, String> {
    if header_version == 0x0300 {
        let raw: FileHeaderV3Raw = read_packed(bytes)?;
        Ok(FileHeaderDto {
            filename_len: raw.filename_len,
            total_header_size: raw.total_header_size,
            maintype: string_from_bytes(&raw.maintype),
            subtype: string_from_bytes(&raw.subtype),
            v1: None,
            v3: Some(FileHeaderV3Dto {
                unknown_0: raw.unknown_0,
                filename: string_from_bytes(&raw.filename),
                stored_length: raw.stored_length,
                pad1: raw.pad1,
                original_length: raw.original_length,
                pad2: raw.pad2,
                offset: raw.offset,
            }),
        })
    } else if header_version == 0x0100 {
        let raw: FileHeaderV1Raw = read_packed(bytes)?;
        Ok(FileHeaderDto {
            filename_len: raw.filename_len,
            total_header_size: raw.total_header_size,
            maintype: string_from_bytes(&raw.maintype),
            subtype: string_from_bytes(&raw.subtype),
            v1: Some(FileHeaderV1Dto {
                unknown_3: raw.unknown_3,
                stored_length: raw.stored_length,
                original_length: raw.original_length,
                offset: raw.offset,
                unknown: raw.unknown,
                filename: string_from_bytes(&raw.filename),
            }),
            v3: None,
        })
    } else {
        Err(format!("unknown image header version: {header_version:#x}"))
    }
}

fn file_info_from_header(header: &FileHeaderDto) -> FileInfoDto {
    if let Some(v3) = &header.v3 {
        FileInfoDto {
            filename: v3.filename.clone(),
            maintype: header.maintype.clone(),
            subtype: header.subtype.clone(),
            stored_length: v3.stored_length,
            original_length: v3.original_length,
            offset: v3.offset,
        }
    } else if let Some(v1) = &header.v1 {
        FileInfoDto {
            filename: v1.filename.clone(),
            maintype: header.maintype.clone(),
            subtype: header.subtype.clone(),
            stored_length: v1.stored_length,
            original_length: v1.original_length,
            offset: v1.offset,
        }
    } else {
        FileInfoDto {
            filename: String::new(),
            maintype: header.maintype.clone(),
            subtype: header.subtype.clone(),
            stored_length: 0,
            original_length: 0,
            offset: 0,
        }
    }
}

pub fn parse_image(file_path: &str) -> Result<ParseImageResultDto, String> {
    let mut file = File::open(file_path).map_err(|e| format!("failed to open image: {e}"))?;

    let mut magic = [0u8; 8];
    file.read_exact(&mut magic)
        .map_err(|e| format!("failed to read image magic: {e}"))?;
    if &magic != IMAGEWTY_MAGIC {
        return Ok(ParseImageResultDto {
            image_info: None,
            file_headers: Vec::new(),
            is_encrypted: true,
            last_error: None,
        });
    }

    file.seek(SeekFrom::Start(0))
        .map_err(|e| format!("failed to seek image: {e}"))?;
    let mut header_bytes = vec![0u8; IMAGEWTY_FILEHDR_LEN];
    file.read_exact(&mut header_bytes)
        .map_err(|e| format!("failed to read image header: {e}"))?;

    let image_header = to_image_header_dto(&header_bytes)?;
    let header_version = image_header.header_version;
    let num_files = num_files_from_header(&image_header);

    let mut file_headers = Vec::with_capacity(num_files as usize);
    for index in 0..num_files {
        let offset = IMAGEWTY_FILEHDR_LEN as u64 + (index as u64) * IMAGEWTY_FILEHDR_LEN as u64;
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| format!("failed to seek file header: {e}"))?;
        let mut file_header_bytes = vec![0u8; IMAGEWTY_FILEHDR_LEN];
        file.read_exact(&mut file_header_bytes)
            .map_err(|e| format!("failed to read file header: {e}"))?;
        file_headers.push(to_file_header_dto(&file_header_bytes, header_version)?);
    }

    let image_info = ImageInfoDto {
        header: image_header,
        files: file_headers.iter().map(file_info_from_header).collect(),
        is_encrypted: false,
    };

    Ok(ParseImageResultDto {
        image_info: Some(image_info),
        file_headers,
        is_encrypted: false,
        last_error: None,
    })
}

fn read_entry_data(
    file_path: &str,
    matcher: impl Fn(&FileHeaderDto) -> bool,
    range: Option<(usize, usize)>,
) -> Result<Option<Vec<u8>>, String> {
    let parsed = parse_image(file_path)?;
    if parsed.is_encrypted {
        return Ok(None);
    }
    let file_header = parsed.file_headers.iter().find(|entry| matcher(entry));
    let Some(file_header) = file_header else {
        return Ok(None);
    };

    let (offset, length) = if let Some(v3) = &file_header.v3 {
        (v3.offset as usize, v3.original_length as usize)
    } else if let Some(v1) = &file_header.v1 {
        (v1.offset as usize, v1.original_length as usize)
    } else {
        return Ok(None);
    };

    let (start, read_len) = range.unwrap_or((0, length));
    if start.saturating_add(read_len) > length {
        return Ok(None);
    }

    let mut file = File::open(file_path).map_err(|e| format!("failed to open image: {e}"))?;
    file.seek(SeekFrom::Start((offset + start) as u64))
        .map_err(|e| format!("failed to seek entry data: {e}"))?;
    let mut data = vec![0u8; read_len];
    file.read_exact(&mut data)
        .map_err(|e| format!("failed to read entry data: {e}"))?;
    Ok(Some(data))
}

pub fn read_entry_by_filename(file_path: &str, filename: &str) -> Result<Option<Vec<u8>>, String> {
    read_entry_data(
        file_path,
        |entry| {
            entry
                .v3
                .as_ref()
                .map(|v| v.filename == filename)
                .or_else(|| entry.v1.as_ref().map(|v| v.filename == filename))
                .unwrap_or(false)
        },
        None,
    )
}

pub fn read_entry_by_maintype_subtype(
    file_path: &str,
    maintype: &str,
    subtype: &str,
) -> Result<Option<Vec<u8>>, String> {
    read_entry_data(
        file_path,
        |entry| entry.maintype == maintype && entry.subtype == subtype,
        None,
    )
}

pub fn read_entry_range_by_filename(
    file_path: &str,
    filename: &str,
    start: usize,
    length: usize,
) -> Result<Option<Vec<u8>>, String> {
    read_entry_data(
        file_path,
        |entry| {
            entry
                .v3
                .as_ref()
                .map(|v| v.filename == filename)
                .or_else(|| entry.v1.as_ref().map(|v| v.filename == filename))
                .unwrap_or(false)
        },
        Some((start, length)),
    )
}

pub fn read_entry_range_by_maintype_subtype(
    file_path: &str,
    maintype: &str,
    subtype: &str,
    start: usize,
    length: usize,
) -> Result<Option<Vec<u8>>, String> {
    read_entry_data(
        file_path,
        |entry| entry.maintype == maintype && entry.subtype == subtype,
        Some((start, length)),
    )
}
