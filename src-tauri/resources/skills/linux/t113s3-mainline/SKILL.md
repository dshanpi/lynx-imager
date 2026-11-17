---
name: t113s3-mainline-nand
version: 1.0.0
domain: linux
systems: [linux, buildroot, uboot]
category: boards/t113s3
chip-vendors: [allwinner]
required-tools: [lynx_workbench_status, lynx_allwinner_enter_fel, lynx_flash_capabilities, lynx_start_flash, lynx_workbench_task_status, lynx_serial_open, lynx_serial_read]
description: Build, flash and verify the DshanPi T113S3 Pro pure-mainline system on SPI-NAND.
---
# DshanPi T113S3 Pro mainline NAND

Target facts: T113S3/R528, 128 MiB DDR3, Winbond 256 MiB SPI-NAND, UART3 PB6/PB7 at 115200.
The installed system is mainline U-Boot/Linux/Buildroot. The production NAND flashing path may use a RAM-only Tina/IMAGEWTY loader to enter FES.
Keep U-Boot SPI bus-width policy separate from Linux, preserve `ENV_IS_NOWHERE`, recognize boot source 4, and keep the FES layout consistent with the mainline DTS and UBI arguments.
After flashing, require a power-loss cold boot and UART evidence through the login prompt.
