---
name: t527-sdk-emmc
version: 1.0.0
domain: linux
systems: [linux, buildroot, tina]
category: boards/t527
chip-vendors: [allwinner]
required-tools: [lynx_workbench_status]
description: Work inside the fixed Avaota A1 T527 SDK channel and keep eMMC assumptions separate from T113 NAND.
---
# Avaota A1 T527 SDK and eMMC

Use the saved SDK channel and fixed remote SDK directory. Treat eMMC as block storage and do not reuse T113 SPI-NAND physical layout or bad-block assumptions.
Before any future flashing workflow, identify the exact image format, eMMC capacity, boot partitions and board-specific loader. Skills do not authorize arbitrary SSH shell or hardware writes.
