# NAND boundary

- NAND/SPI-NAND: use FES component mode, a declared erase policy, Boot0/Boot1 validation, and partition verification.
- eMMC, SD/SD-NAND, SPI-NOR and UFS: raw mode is allowed only after storage type and capacity preflight.
- A failed attempt is terminal. Do not retry automatically; require the operator to restore FEL.
- Keep UART boot output separate from FEL/FES protocol output.
