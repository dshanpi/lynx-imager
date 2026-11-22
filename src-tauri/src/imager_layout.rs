//! Read-only layout metadata. Offsets here never become flashing arguments.
use serde::Serialize;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
};

#[derive(Debug, Serialize)]
pub(crate) struct Region {
    pub name: String,
    pub offset: u64,
    pub bytes: u64,
}
#[derive(Debug, Serialize)]
pub(crate) struct Layout {
    pub source: String,
    pub coordinate: &'static str,
    pub regions: Vec<Region>,
    pub extent: Option<u64>,
}
impl Layout {
    pub fn new(source: impl Into<String>, coordinate: &'static str) -> Self {
        Self {
            source: source.into(),
            coordinate,
            regions: Vec::new(),
            extent: None,
        }
    }
    pub fn add(&mut self, name: String, offset: u64, bytes: u64) {
        self.regions.push(Region {
            name,
            offset,
            bytes,
        });
    }
}

fn read(file: &mut File, at: u64, buf: &mut [u8]) -> Option<()> {
    file.seek(SeekFrom::Start(at)).ok()?;
    file.read_exact(buf).ok()
}
fn word(buf: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(buf[at..at + 4].try_into().unwrap())
}
fn wide(buf: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(buf[at..at + 8].try_into().unwrap())
}

/// Classic RKAF item directory, either standalone or referenced by the RKFW header.
pub fn rockchip(f: &mut File, length: u64) -> Option<Layout> {
    let mut header = [0u8; 140];
    read(f, 0, &mut header)?;
    let (base, extent) = if &header[..4] == b"RKFW" {
        (u64::from(word(&header, 33)), u64::from(word(&header, 37)))
    } else {
        (0, length)
    };
    if extent < 140 || base + extent > length {
        return None;
    }
    read(f, base, &mut header)?;
    if &header[..4] != b"RKAF" {
        return None;
    }
    let count = u64::from(word(&header, 136));
    if !(1..=4096).contains(&count) || 140 + 112 * count > extent {
        return None;
    }
    let mut layout = Layout::new("RKAF 包目录", "包内偏移");
    for i in 0..count {
        let mut item = [0u8; 112];
        read(f, base + 140 + 112 * i, &mut item)?;
        let name = String::from_utf8_lossy(&item[..32])
            .split('\0')
            .next()?
            .to_owned();
        if item[32..96].starts_with(b"SELF\0") {
            continue;
        }
        let offset = u64::from(word(&item, 96));
        let bytes = u64::from(word(&item, 108));
        if bytes == 0 {
            continue;
        }
        if offset < 140 + 112 * count || offset + bytes > extent {
            return None;
        }
        layout.add(name, base + offset, bytes);
    }
    ordered(layout)
}

fn gpt(f: &mut File, length: u64, sector: u64) -> Option<Layout> {
    let mut h = vec![0u8; sector as usize];
    read(f, sector, &mut h)?;
    if &h[..8] != b"EFI PART" || wide(&h, 24) != 1 {
        return None;
    }
    let header_size = word(&h, 12) as usize;
    if !(92..=h.len()).contains(&header_size) {
        return None;
    }
    let crc = word(&h, 16);
    h[16..20].fill(0);
    if crc32fast::hash(&h[..header_size]) != crc {
        return None;
    }
    let count = word(&h, 80) as usize;
    let stride = word(&h, 84) as usize;
    if !(1..=4096).contains(&count) || !(128..=1024).contains(&stride) || stride % 128 != 0 {
        return None;
    }
    let offset = wide(&h, 72).checked_mul(sector)?;
    let mut entries = vec![0u8; count * stride];
    if offset < sector * 2 || offset.checked_add(entries.len() as u64)? > length {
        return None;
    }
    read(f, offset, &mut entries)?;
    if crc32fast::hash(&entries) != word(&h, 88) {
        return None;
    }
    let mut layout = Layout::new("GPT 分区表", "磁盘偏移");
    layout.extent = Some(length);
    for (index, e) in entries.chunks_exact(stride).enumerate() {
        if e[..16].iter().all(|b| *b == 0) {
            continue;
        }
        let first = wide(e, 32);
        let last = wide(e, 40);
        if first < wide(&h, 40) || last > wide(&h, 48) || first > last || last >= length / sector {
            return None;
        }
        let chars: Vec<_> = e[56..128]
            .chunks_exact(2)
            .map(|u| u16::from_le_bytes([u[0], u[1]]))
            .take_while(|u| *u != 0)
            .collect();
        let mut name = String::from_utf16_lossy(&chars);
        if name.is_empty() {
            name = format!("分区 {}", index + 1)
        }
        layout.add(name, first * sector, (last - first + 1) * sector);
    }
    ordered(layout)
}
fn ordered(mut layout: Layout) -> Option<Layout> {
    layout.regions.sort_by_key(|r| r.offset);
    if layout.regions.is_empty()
        || layout
            .regions
            .windows(2)
            .any(|p| p[0].offset + p[0].bytes > p[1].offset)
    {
        None
    } else {
        Some(layout)
    }
}
pub fn disk(f: &mut File, length: u64) -> Option<Layout> {
    let mut mbr = [0u8; 512];
    read(f, 0, &mut mbr)?;
    for sector in [512, 4096] {
        let mut sig = [0; 8];
        if read(f, sector, &mut sig).is_some() && &sig == b"EFI PART" {
            return gpt(f, length, sector);
        }
    }
    if mbr[510..] != [0x55, 0xaa] {
        return None;
    }
    let mut layout = Layout::new("MBR 主分区表", "磁盘偏移");
    layout.extent = Some(length);
    for (i, e) in mbr[446..510].chunks_exact(16).enumerate() {
        let kind = e[4];
        if kind == 0 {
            continue;
        }
        if kind == 0xee || !matches!(e[0], 0 | 0x80) {
            return None;
        }
        let offset = u64::from(word(e, 8)) * 512;
        let bytes = u64::from(word(e, 12)) * 512;
        if offset == 0 || bytes == 0 || offset + bytes > length {
            return None;
        }
        layout.add(format!("主分区 {} · 0x{:02X}", i + 1, kind), offset, bytes);
    }
    ordered(layout)
}

pub fn sunxi(data: &[u8]) -> Option<Layout> {
    if data.len() != 16384 || crc32fast::hash(&data[4..]) != word(data, 0) || word(data, 4) != 0x200
    {
        return None;
    }
    let table = crate::firmware::mbr::parse_sunxi_mbr(data).ok()?;
    if table.part_count as usize > table.array.len() {
        return None;
    }
    let mut layout = Layout::new("sunxi_mbr.fex", "介质偏移");
    for p in table.array.iter().take(table.part_count as usize) {
        let sector = (u64::from(p.addrhi) << 32) | u64::from(p.addrlo);
        let length = (u64::from(p.lenhi) << 32) | u64::from(p.lenlo);
        let offset = sector.checked_mul(512)?;
        let bytes = length.checked_mul(512)?;
        offset.checked_add(bytes)?;
        layout.add(p.name.clone(), offset, bytes);
    }
    ordered(layout)
}

/// Limit the optional boot-region probe; larger or invalid headers still retain their magic marker.
pub fn boot0_bytes(file: &mut File, head: &[u8], length: u64) -> Option<u64> {
    let boot = crate::firmware::boot0::parse_boot0(head).ok()?;
    let size = u64::from(boot.length);
    if !(48..=1024 * 1024).contains(&size) || size > length || !size.is_multiple_of(4) {
        return None;
    }
    let mut buf = vec![0u8; size as usize];
    read(file, 0, &mut buf)?;
    buf[12..16].copy_from_slice(&0x5f0a6c39u32.to_le_bytes());
    let sum = buf
        .chunks_exact(4)
        .fold(0u32, |sum, bytes| sum.wrapping_add(word(bytes, 0)));
    (sum == boot.check_sum).then_some(size)
}
