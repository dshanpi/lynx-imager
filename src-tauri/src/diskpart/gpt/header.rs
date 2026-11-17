use std::mem::size_of;

pub const GPT_SIGNATURE: &[u8; 8] = b"EFI PART";
pub const GPT_HEADER_SIZE: usize = 92;
pub const GPT_HEADER_LBA: u64 = 1;
pub const GPT_MIN_HEADER_SIZE: usize = 512 + GPT_HEADER_SIZE;
pub const SECTOR_SIZE: u64 = 512;
#[allow(dead_code)]
pub const DEFAULT_PARTITION_COUNT: u32 = 128;
#[allow(dead_code)]
pub const DEFAULT_PARTITION_ENTRY_SIZE: u32 = 128;

#[derive(Debug, Clone, Copy, Default)]
#[repr(C, packed)]
pub struct GptHeaderRaw {
    pub signature: [u8; 8],
    pub revision: u32,
    pub header_size: u32,
    pub header_crc32: u32,
    pub reserved: u32,
    pub my_lba: u64,
    pub alternate_lba: u64,
    pub first_usable_lba: u64,
    pub last_usable_lba: u64,
    pub disk_guid: [u8; 16],
    pub partition_entry_lba: u64,
    pub num_partition_entries: u32,
    pub size_of_partition_entry: u32,
    pub partition_entry_crc32: u32,
}

impl GptHeaderRaw {
    pub fn is_valid(&self) -> bool {
        &self.signature == GPT_SIGNATURE
            && self.header_size >= 92
            && self.header_size as usize <= GPT_HEADER_SIZE
    }

    pub fn calculate_crc32(&self) -> u32 {
        let mut header_copy = *self;
        header_copy.header_crc32 = 0;
        let bytes = unsafe {
            std::slice::from_raw_parts(
                &header_copy as *const GptHeaderRaw as *const u8,
                size_of::<GptHeaderRaw>(),
            )
        };
        crc32fast::hash(bytes)
    }

    pub fn update_crc32(&mut self) {
        self.header_crc32 = self.calculate_crc32();
    }

    pub fn as_bytes(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                self as *const GptHeaderRaw as *const u8,
                size_of::<GptHeaderRaw>(),
            )
        }
    }

    #[allow(dead_code)]
    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        unsafe {
            std::slice::from_raw_parts_mut(
                self as *mut GptHeaderRaw as *mut u8,
                size_of::<GptHeaderRaw>(),
            )
        }
    }
}

#[derive(Debug, Clone)]
pub struct GptHeader {
    pub revision: u32,
    pub header_size: u32,
    pub my_lba: u64,
    pub alternate_lba: u64,
    pub first_usable_lba: u64,
    pub last_usable_lba: u64,
    pub disk_guid: [u8; 16],
    pub partition_entry_lba: u64,
    pub num_partition_entries: u32,
    pub size_of_partition_entry: u32,
    pub partition_entry_crc32: u32,
}

impl GptHeader {
    pub fn from_raw(raw: &GptHeaderRaw) -> Self {
        Self {
            revision: raw.revision,
            header_size: raw.header_size,
            my_lba: raw.my_lba,
            alternate_lba: raw.alternate_lba,
            first_usable_lba: raw.first_usable_lba,
            last_usable_lba: raw.last_usable_lba,
            disk_guid: raw.disk_guid,
            partition_entry_lba: raw.partition_entry_lba,
            num_partition_entries: raw.num_partition_entries,
            size_of_partition_entry: raw.size_of_partition_entry,
            partition_entry_crc32: raw.partition_entry_crc32,
        }
    }

    pub fn to_raw(&self) -> GptHeaderRaw {
        let mut raw = GptHeaderRaw {
            signature: *GPT_SIGNATURE,
            revision: self.revision,
            header_size: self.header_size,
            header_crc32: 0,
            reserved: 0,
            my_lba: self.my_lba,
            alternate_lba: self.alternate_lba,
            first_usable_lba: self.first_usable_lba,
            last_usable_lba: self.last_usable_lba,
            disk_guid: self.disk_guid,
            partition_entry_lba: self.partition_entry_lba,
            num_partition_entries: self.num_partition_entries,
            size_of_partition_entry: self.size_of_partition_entry,
            partition_entry_crc32: self.partition_entry_crc32,
        };
        raw.update_crc32();
        raw
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct GptData {
    pub protective_mbr: Vec<u8>,
    pub primary_header: GptHeader,
    pub partition_entries: Vec<u8>,
    pub backup_header: Option<GptHeader>,
}

#[allow(dead_code)]
impl GptData {
    pub fn from_bytes(data: &[u8]) -> Result<Self, String> {
        if data.len() < GPT_MIN_HEADER_SIZE {
            return Err(format!(
                "Data too small: {} bytes, need at least {}",
                data.len(),
                GPT_MIN_HEADER_SIZE
            ));
        }

        let protective_mbr = data[0..512].to_vec();

        let raw_header =
            unsafe { &*(data[512..512 + GPT_HEADER_SIZE].as_ptr() as *const GptHeaderRaw) };

        if !raw_header.is_valid() {
            return Err(format!("Invalid GPT signature: {:?}", raw_header.signature));
        }

        let primary_header = GptHeader::from_raw(raw_header);

        let partition_entries_size = (primary_header.num_partition_entries as u64
            * primary_header.size_of_partition_entry as u64)
            as usize;
        let partition_entries_start = (primary_header.partition_entry_lba * SECTOR_SIZE) as usize;

        let partition_entries = if partition_entries_start + partition_entries_size <= data.len() {
            data[partition_entries_start..partition_entries_start + partition_entries_size].to_vec()
        } else {
            vec![0u8; partition_entries_size]
        };

        Ok(Self {
            protective_mbr,
            primary_header,
            partition_entries,
            backup_header: None,
        })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let total_size = 512 + GPT_HEADER_SIZE + self.partition_entries.len();
        let mut data = Vec::with_capacity(total_size);

        data.extend_from_slice(&self.protective_mbr);

        let raw_header = self.primary_header.to_raw();
        data.extend_from_slice(raw_header.as_bytes());

        let padding = 512 - GPT_HEADER_SIZE;
        data.extend(std::iter::repeat_n(0u8, padding));

        data.extend_from_slice(&self.partition_entries);

        data
    }

    pub fn calculate_backup_parameters(&self, total_size_bytes: u64) -> (u64, u64, u64) {
        let total_sectors = total_size_bytes / SECTOR_SIZE;
        let backup_gpt_lba = total_sectors - 1;
        let partition_entries_sectors = (self.partition_entries.len() as u64).div_ceil(SECTOR_SIZE);
        let last_usable_lba = backup_gpt_lba - partition_entries_sectors - 1;
        (total_sectors, backup_gpt_lba, last_usable_lba)
    }

    pub fn adjust_for_size(&mut self, total_size_bytes: u64) -> Result<u64, String> {
        let (_, backup_gpt_lba, last_usable_lba) =
            self.calculate_backup_parameters(total_size_bytes);

        if last_usable_lba >= self.primary_header.last_usable_lba {
            return Ok(self.primary_header.last_usable_lba);
        }

        let old_last_usable_lba = self.primary_header.last_usable_lba;
        self.primary_header.last_usable_lba = last_usable_lba;
        self.primary_header.alternate_lba = backup_gpt_lba;

        Ok(old_last_usable_lba)
    }

    pub fn create_backup_header(&self, total_size_bytes: u64) -> Result<GptHeader, String> {
        let (_, backup_gpt_lba, last_usable_lba) =
            self.calculate_backup_parameters(total_size_bytes);
        let partition_entries_sectors = (self.partition_entries.len() as u64).div_ceil(SECTOR_SIZE);
        let backup_partition_entries_lba = backup_gpt_lba - partition_entries_sectors;

        Ok(GptHeader {
            revision: self.primary_header.revision,
            header_size: self.primary_header.header_size,
            my_lba: backup_gpt_lba,
            alternate_lba: GPT_HEADER_LBA,
            first_usable_lba: self.primary_header.first_usable_lba,
            last_usable_lba,
            disk_guid: self.primary_header.disk_guid,
            partition_entry_lba: backup_partition_entries_lba,
            num_partition_entries: self.primary_header.num_partition_entries,
            size_of_partition_entry: self.primary_header.size_of_partition_entry,
            partition_entry_crc32: self.primary_header.partition_entry_crc32,
        })
    }

    pub fn to_backup_bytes(&self, total_size_bytes: u64) -> Result<Vec<u8>, String> {
        let backup_header = self.create_backup_header(total_size_bytes)?;
        let partition_entries_sectors = (self.partition_entries.len() as u64).div_ceil(SECTOR_SIZE);
        let total_backup_size = (partition_entries_sectors + 1) * SECTOR_SIZE;

        let mut data = Vec::with_capacity(total_backup_size as usize);

        data.extend_from_slice(&self.partition_entries);

        let padding =
            (partition_entries_sectors * SECTOR_SIZE) as usize - self.partition_entries.len();
        data.extend(std::iter::repeat_n(0u8, padding));

        let raw_header = backup_header.to_raw();
        data.extend_from_slice(raw_header.as_bytes());

        let final_padding = SECTOR_SIZE as usize - GPT_HEADER_SIZE;
        data.extend(std::iter::repeat_n(0u8, final_padding));

        Ok(data)
    }
}

#[allow(dead_code)]
pub fn read_gpt_header_from_bytes(data: &[u8]) -> Result<GptHeader, String> {
    if data.len() < GPT_MIN_HEADER_SIZE {
        return Err(format!("Data too small: {} bytes", data.len()));
    }

    let raw_header =
        unsafe { &*(data[512..512 + GPT_HEADER_SIZE].as_ptr() as *const GptHeaderRaw) };

    if !raw_header.is_valid() {
        return Err(format!("Invalid GPT signature: {:?}", raw_header.signature));
    }

    Ok(GptHeader::from_raw(raw_header))
}

pub fn modify_gpt_header_in_place(data: &mut [u8], total_size_bytes: u64) -> Result<u64, String> {
    if data.len() < GPT_MIN_HEADER_SIZE {
        return Err(format!("Data too small: {} bytes", data.len()));
    }

    let (mbr_part, rest) = data.split_at_mut(512);
    let header_slice = &mut rest[..GPT_HEADER_SIZE];

    let raw_header = unsafe { &mut *(header_slice.as_mut_ptr() as *mut GptHeaderRaw) };

    if !raw_header.is_valid() {
        return Err(format!(
            "Invalid GPT signature: {:?}",
            &mbr_part[0..8.min(mbr_part.len())]
        ));
    }

    let total_sectors = total_size_bytes / SECTOR_SIZE;
    let backup_gpt_lba = total_sectors - 1;
    let partition_entries_size =
        raw_header.num_partition_entries as u64 * raw_header.size_of_partition_entry as u64;
    let partition_entries_sectors = partition_entries_size.div_ceil(SECTOR_SIZE);
    let last_usable_lba = backup_gpt_lba - partition_entries_sectors - 1;

    let old_last_usable_lba = raw_header.last_usable_lba;

    if last_usable_lba >= old_last_usable_lba {
        return Ok(old_last_usable_lba);
    }

    raw_header.last_usable_lba = last_usable_lba;
    raw_header.alternate_lba = backup_gpt_lba;
    raw_header.update_crc32();

    Ok(last_usable_lba)
}

#[allow(dead_code)]
pub fn create_backup_gpt_bytes(
    primary_data: &[u8],
    total_size_bytes: u64,
) -> Result<Vec<u8>, String> {
    if primary_data.len() < GPT_MIN_HEADER_SIZE {
        return Err(format!("Data too small: {} bytes", primary_data.len()));
    }

    let gpt_data = GptData::from_bytes(primary_data)?;
    gpt_data.to_backup_bytes(total_size_bytes)
}

pub fn modify_backup_gpt_in_place(
    data: &mut [u8],
    total_size_bytes: u64,
    primary_data: &[u8],
) -> Result<(), String> {
    if data.len() < GPT_MIN_HEADER_SIZE {
        return Err(format!("Data too small for backup: {} bytes", data.len()));
    }

    let primary_gpt = GptData::from_bytes(primary_data)?;
    let backup_header = primary_gpt.create_backup_header(total_size_bytes)?;
    let raw_backup = backup_header.to_raw();

    let (_, rest) = data.split_at_mut(512);
    let header_slice = &mut rest[..GPT_HEADER_SIZE];
    header_slice.copy_from_slice(raw_backup.as_bytes());

    Ok(())
}

#[allow(dead_code)]
pub fn guid_to_string(guid: &[u8; 16]) -> String {
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        guid[3], guid[2], guid[1], guid[0],
        guid[5], guid[4],
        guid[7], guid[6],
        guid[8], guid[9],
        guid[10], guid[11], guid[12], guid[13], guid[14], guid[15]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gpt_header_raw_size() {
        assert_eq!(size_of::<GptHeaderRaw>(), GPT_HEADER_SIZE);
    }

    #[test]
    fn test_gpt_signature() {
        let mut raw = GptHeaderRaw::default();
        raw.signature.copy_from_slice(GPT_SIGNATURE);
        raw.header_size = GPT_HEADER_SIZE as u32;
        assert!(raw.is_valid());
    }
}
