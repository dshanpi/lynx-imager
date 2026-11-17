//! Native Canaan Kendryte K230 USB flashing support.
//!
//! The protocol implementation is derived from Canaanburn and the official
//! Kendryte K230 flashing tools. It is embedded so desktop users do not need
//! to install or configure a separate command-line flasher.

pub mod burner;
pub mod error;
pub mod kdimage;
pub mod protocol;
pub mod types;

pub const LOADER_MMC: &[u8] = include_bytes!("loaders/loader_mmc.bin");
pub const LOADER_SPI_NAND: &[u8] = include_bytes!("loaders/loader_spi_nand.bin");
pub const LOADER_SPI_NOR: &[u8] = include_bytes!("loaders/loader_spi_nor.bin");
