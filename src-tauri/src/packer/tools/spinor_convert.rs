use log::{debug, error, info, warn};
use std::fs::File;
use std::io::Write;
use std::path::Path;

use crate::packer::tools::{
    build_partition_subtype, extract_file_data, find_file_by_subtype, parse_image,
};
use crate::packer::types::{SpinorMergeConfig, SpinorMergeResult};

const GPT_SIZE: u64 = 16 * 1024;

pub fn merge_spinor_firmware(config: SpinorMergeConfig) -> Result<SpinorMergeResult, String> {
    info!("Starting SPI NOR firmware merge...");
    debug!(
        "Config: output={}, logic_start={} sectors, uboot_start={} sectors, nor_size={} MB",
        config.output_path,
        config.logic_start / 512,
        config.uboot_start / 512,
        config.nor_size / (1024 * 1024)
    );

    let nor_size = config.nor_size as usize;

    let firmware_path = Path::new(&config.firmware_path);
    if !firmware_path.exists() {
        error!("Firmware file does not exist: {}", config.firmware_path);
        return Err(format!(
            "Firmware file does not exist: {}",
            config.firmware_path
        ));
    }

    let mut firmware_file = File::open(firmware_path).map_err(|e| {
        error!("Failed to open firmware file: {}", e);
        format!("Failed to open firmware file: {}", e)
    })?;

    let (image_header, file_entries) = parse_image(&mut firmware_file)?;
    info!("Parsed image: {} files", image_header.num_files);

    let mut output_buffer = vec![0xFFu8; nor_size];

    if let Some(boot0_entry) = find_file_by_subtype(&file_entries, "1234567890BNOR_0") {
        match extract_file_data(&mut firmware_file, boot0_entry) {
            Ok(data) => {
                output_buffer[0..data.len()].copy_from_slice(&data);
                info!("Boot0 NOR loaded: {} bytes at offset 0", data.len());
            }
            Err(e) => {
                warn!("Failed to extract Boot0 NOR: {}", e);
            }
        }
    } else {
        warn!("Boot0 NOR not found in firmware");
    }

    if let Some(uboot_entry) = find_file_by_subtype(&file_entries, "BOOTPKG-NOR00000") {
        match extract_file_data(&mut firmware_file, uboot_entry) {
            Ok(data) => {
                let uboot_offset = config.uboot_start as usize;
                if uboot_offset + data.len() > nor_size {
                    error!("U-Boot exceeds NOR size");
                    return Err("U-Boot exceeds NOR size".to_string());
                }
                output_buffer[uboot_offset..uboot_offset + data.len()].copy_from_slice(&data);
                info!(
                    "U-Boot NOR loaded: {} bytes at offset {}",
                    data.len(),
                    uboot_offset
                );
            }
            Err(e) => {
                warn!("Failed to extract U-Boot NOR: {}", e);
            }
        }
    } else {
        warn!("U-Boot NOR not found in firmware");
    }

    if let Some(gpt_entry) = find_file_by_subtype(&file_entries, "1234567890___GPT") {
        match extract_file_data(&mut firmware_file, gpt_entry) {
            Ok(data) => {
                let gpt_offset = config.logic_start as usize;
                if gpt_offset + data.len() > nor_size {
                    error!("GPT exceeds NOR size");
                    return Err("GPT exceeds NOR size".to_string());
                }
                output_buffer[gpt_offset..gpt_offset + data.len()].copy_from_slice(&data);
                info!("GPT loaded: {} bytes at offset {}", data.len(), gpt_offset);
            }
            Err(e) => {
                warn!("Failed to extract GPT: {}", e);
            }
        }
    } else {
        warn!("GPT not found in firmware");
    }

    let mut partition_offset = config.logic_start + GPT_SIZE;

    for partition in &config.partitions {
        if partition.download_file.is_empty() {
            debug!(
                "Partition {} has no download file, skipping",
                partition.name
            );
            partition_offset += partition.size;
            continue;
        }

        let partition_subtype = build_partition_subtype(&partition.download_file);

        if let Some(partition_entry) = find_file_by_subtype(&file_entries, &partition_subtype) {
            match extract_file_data(&mut firmware_file, partition_entry) {
                Ok(data) => {
                    let partition_size = partition.size as usize;

                    if data.len() > partition_size {
                        error!(
                            "Partition {} file size ({}) exceeds partition size ({})",
                            partition.name,
                            data.len(),
                            partition_size
                        );
                        return Err(format!(
                            "Partition {} file size exceeds partition size",
                            partition.name
                        ));
                    }

                    let offset = partition_offset as usize;
                    if offset + data.len() > nor_size {
                        error!("Partition {} exceeds NOR size", partition.name);
                        return Err(format!("Partition {} exceeds NOR size", partition.name));
                    }

                    output_buffer[offset..offset + data.len()].copy_from_slice(&data);
                    info!(
                        "Partition {} loaded: {} bytes at offset {}",
                        partition.name,
                        data.len(),
                        offset
                    );
                }
                Err(e) => {
                    warn!("Failed to extract partition {}: {}", partition.name, e);
                }
            }
        } else {
            warn!(
                "Partition file not found in firmware: {} -> {} (subtype: {})",
                partition.name, partition.download_file, partition_subtype
            );
        }

        partition_offset += partition.size;
    }

    let output_path = Path::new(&config.output_path);
    let mut output_file = File::create(output_path).map_err(|e| {
        error!("Failed to create output file: {}", e);
        format!("Failed to create output file: {}", e)
    })?;

    output_file.write_all(&output_buffer).map_err(|e| {
        error!("Failed to write output file: {}", e);
        format!("Failed to write output file: {}", e)
    })?;

    output_file.flush().map_err(|e| {
        error!("Failed to flush output file: {}", e);
        format!("Failed to flush output file: {}", e)
    })?;

    info!("SPI NOR firmware merge completed: {} bytes", nor_size);

    Ok(SpinorMergeResult {
        success: true,
        message: format!("Successfully created SPI NOR firmware: {} bytes", nor_size),
        output_size: nor_size as u64,
    })
}
