---
name: allwinner-fes-flashing
version: 1.0.0
domain: linux
systems: [linux, allwinner, fes]
category: firmware/allwinner
chip-vendors: [allwinner]
required-tools: [lynx_workbench_status, lynx_flash_capabilities, lynx_allwinner_enter_fel, lynx_scan_flash_devices, lynx_start_flash, lynx_workbench_task_status]
description: Use a RAM-only loader and FES NAND component semantics while keeping the installed system mainline.
---
# Allwinner FES component flashing

Use the board-matched loader only for RAM DRAM/FES initialization. Detect the actual storage before selecting a write path.
NAND and SPI-NAND require component packages and FES bad-block/ECC/Boot0/Boot1 semantics; never use whole-disk raw mode for NAND.
After explicit destructive approval, poll the same task until a deterministic terminal state. A successful FES verification is not proof of cold boot.

See `references/nand-boundary.md` for the storage boundary.
