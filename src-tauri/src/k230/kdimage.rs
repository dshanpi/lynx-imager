//! `*.kdimg` 镜像格式解析。
//!
//! kdimg 是嘉楠 K230 烧录镜像容器格式,结构:
//! - 512 字节头(`kd_img_hdr_t`):magic、CRC32、版本、分区数、image/chip/board info
//! - 分区表:N * 256 字节(`kd_img_part_t`):每项含介质偏移、大小、SHA256、名称
//! - 分区数据:各分区实际内容(`part_content_offset` 指向)
//!
//! 校验:头部 CRC32 + 分区表 CRC32 + 每分区内容 SHA256。
//!
//! 参考:`refer/k230_flash/src/kburn/include/kdimage.h`、`kdimage.cpp`。
//! 目前支持 v2 版本(`part_flag` 为 uint64);v1 兼容后续添加。

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crc32fast::Hasher as Crc32Hasher;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::error::K230Error;

/// kdimg 头部 magic。
pub const KDIMG_HEADER_MAGIC: u32 = 0x27CB8F93;
/// kdimg 分区项 magic。
#[allow(dead_code)]
pub const KDIMG_PART_MAGIC: u32 = 0x91DF6DA4;
/// kdimg 头部大小(字节)。
pub const KDIMG_HEADER_SIZE: usize = 512;
/// kdimg 分区项大小(字节)。
pub const KDIMG_PART_SIZE: usize = 256;

/// kdimg 镜像头(512 字节,手动从 buffer 解析以避免 C 结构体 padding 问题)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KdImgHeader {
    pub magic: u32,
    pub crc32: u32,
    pub flag: u32,
    /// 版本:>= 0x02 为 v2(part_flag 是 uint64)
    pub version: u32,
    pub part_tbl_num: u32,
    pub part_tbl_crc32: u32,
    pub image_info: String,
    pub chip_info: String,
    pub board_info: String,
}

/// kdimg 分区项(256 字节)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KdImgPart {
    pub magic: u32,
    /// 介质偏移(4KB 对齐)
    pub part_offset: u32,
    /// 写入大小(4KB 对齐)
    pub part_size: u32,
    /// 额外擦除范围
    pub part_erase_size: u32,
    /// 分区最大容量
    pub part_max_size: u32,
    /// 分区标志(v2 为 uint64;SPI-NAND OOB 等编码在此)
    pub part_flag: u64,
    /// 在 .kdimg 文件内的数据偏移
    pub part_content_offset: u32,
    /// 在 .kdimg 文件内的数据大小(不足 part_size 部分 0xFF 填充)
    pub part_content_size: u32,
    /// 分区内容 SHA256 校验值
    pub part_content_sha256: [u8; 32],
    /// 分区名称
    pub part_name: String,
}

/// 解析后的 kdimg 镜像。
pub struct KdImage {
    pub path: String,
    #[allow(dead_code)]
    pub header: KdImgHeader,
    pub parts: Vec<KdImgPart>,
}

impl KdImage {
    /// 从文件路径解析 kdimg。
    ///
    /// 读取并校验头部 magic + CRC32,读取并校验分区表 CRC32。
    pub fn parse<P: AsRef<Path>>(path: P) -> Result<Self, K230Error> {
        let path = path.as_ref();
        let mut file = File::open(path)?;
        let path_str = path.to_string_lossy().to_string();
        let file_size = file.metadata()?.len();

        // 读头部 512 字节
        let mut hdr_buf = [0u8; KDIMG_HEADER_SIZE];
        file.read_exact(&mut hdr_buf)?;
        let header = parse_header(&hdr_buf);

        if header.magic != KDIMG_HEADER_MAGIC {
            // 非 kdimg 格式,当作普通 raw 镜像(单分区)处理。
            // 参考 k230_flash.cpp:356-368:普通 .img/.bin 当单分区(partName="image",
            // partOffset=0, partFlag=0),整体写入介质起始地址。
            let part = KdImgPart {
                magic: 0,
                part_offset: 0,
                part_size: file_size as u32,
                part_erase_size: 0,
                part_max_size: file_size as u32,
                part_flag: 0,
                part_content_offset: 0,
                part_content_size: file_size as u32,
                part_content_sha256: [0u8; 32], // raw 镜像无 SHA256,跳过校验
                part_name: "image".to_string(),
            };
            return Ok(Self {
                path: path_str,
                header,
                parts: vec![part],
            });
        }

        // 校验头部 CRC32:crc32 字段(偏移 4..8)置 0 后计算整 512 字节
        let mut crc_buf = hdr_buf;
        crc_buf[4..8].copy_from_slice(&0u32.to_le_bytes());
        let mut crc = Crc32Hasher::new();
        crc.update(&crc_buf);
        let hdr_crc = crc.finalize();
        if hdr_crc != header.crc32 {
            return Err(K230Error::KdImage(format!(
                "header CRC32 mismatch: computed 0x{:08X} != expected 0x{:08X}",
                hdr_crc, header.crc32
            )));
        }

        // 读分区表
        let tbl_size = header.part_tbl_num as usize * KDIMG_PART_SIZE;
        let mut tbl_buf = vec![0u8; tbl_size];
        file.read_exact(&mut tbl_buf)?;

        // 校验分区表 CRC32
        let mut crc = Crc32Hasher::new();
        crc.update(&tbl_buf);
        let tbl_crc = crc.finalize();
        if tbl_crc != header.part_tbl_crc32 {
            return Err(K230Error::KdImage(format!(
                "partition table CRC32 mismatch: 0x{:08X} != 0x{:08X}",
                tbl_crc, header.part_tbl_crc32
            )));
        }

        // 解析各分区项
        let mut parts = Vec::with_capacity(header.part_tbl_num as usize);
        for i in 0..header.part_tbl_num as usize {
            let part_buf = &tbl_buf[i * KDIMG_PART_SIZE..(i + 1) * KDIMG_PART_SIZE];
            parts.push(parse_part(part_buf));
        }

        Ok(Self {
            path: path_str,
            header,
            parts,
        })
    }

    /// 读取指定分区的内容数据,并校验 SHA256。
    ///
    /// 不足 `part_size` 的部分由设备端用 0xFF 填充,此处只读取 `part_content_size` 字节。
    pub fn read_part_data(&self, part: &KdImgPart) -> Result<Vec<u8>, K230Error> {
        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(part.part_content_offset as u64))?;
        let mut data = vec![0u8; part.part_content_size as usize];
        file.read_exact(&mut data)?;

        // SHA256 校验(全 0 表示 raw 镜像无校验值,跳过)
        if part.part_content_sha256.iter().any(|&b| b != 0) {
            let mut hasher = Sha256::new();
            hasher.update(&data);
            let digest = hasher.finalize();
            if digest.as_slice() != part.part_content_sha256 {
                return Err(K230Error::KdImage(format!(
                    "partition '{}' SHA256 mismatch",
                    part.part_name
                )));
            }
        }

        Ok(data)
    }
}

/// 从 512 字节 buffer 解析头部字段。
fn parse_header(buf: &[u8]) -> KdImgHeader {
    KdImgHeader {
        magic: u32_le(&buf[0..4]),
        crc32: u32_le(&buf[4..8]),
        flag: u32_le(&buf[8..12]),
        version: u32_le(&buf[12..16]),
        part_tbl_num: u32_le(&buf[16..20]),
        part_tbl_crc32: u32_le(&buf[20..24]),
        image_info: cstr_to_string(&buf[24..56]),
        chip_info: cstr_to_string(&buf[56..88]),
        board_info: cstr_to_string(&buf[88..152]),
    }
}

/// 从 256 字节 buffer 解析分区项字段。
fn parse_part(buf: &[u8]) -> KdImgPart {
    let mut sha256 = [0u8; 32];
    sha256.copy_from_slice(&buf[36..68]);
    KdImgPart {
        magic: u32_le(&buf[0..4]),
        part_offset: u32_le(&buf[4..8]),
        part_size: u32_le(&buf[8..12]),
        part_erase_size: u32_le(&buf[12..16]),
        part_max_size: u32_le(&buf[16..20]),
        part_flag: u64_le(&buf[20..28]),
        part_content_offset: u32_le(&buf[28..32]),
        part_content_size: u32_le(&buf[32..36]),
        part_content_sha256: sha256,
        part_name: cstr_to_string(&buf[68..100]),
    }
}

#[inline]
fn u32_le(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

#[inline]
fn u64_le(b: &[u8]) -> u64 {
    u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

/// C 风格字符串(以 0 结尾)转 Rust String。
fn cstr_to_string(buf: &[u8]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).to_string()
}
