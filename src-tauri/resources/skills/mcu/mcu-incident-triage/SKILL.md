---
name: mcu-incident-triage
version: 1.0.0
domain: mcu
systems: [common, baremetal, freertos]
category: diagnostics/incidents
chip-vendors: []
required-tools: [lynx_mcu_capabilities, lynx_mcu_board_info]
description: Triage a sanitized LYNX MCU-channel failure without inheriting Linux, ADB, shell or generic flashing capabilities.
---
# LYNX MCU incident triage

Keep the saved MCU board scope and the `lynx_mcu_*` boundary intact.

1. Read the stable failure fields and confirm the current board with `lynx_mcu_board_info`.
2. Use `lynx_mcu_capabilities` to determine which read-only observations are actually available.
3. Classify probe, UART, target-power, configuration and software failures separately.
4. Read a more specific MCU Skill when one matches the RTOS or peripheral.
5. Prefer bounded status and UART observation. Do not reset, erase, flash, power-cycle or switch probes automatically.
6. Report missing evidence and request operator approval for any physical or destructive next step.

Never call Linux shell, ADB, SDK filesystem, download or Linux vendor-flashing tools from this channel.
