---
name: serial-diagnostics
version: 1.0.0
domain: linux
systems: [common, linux]
category: diagnostics/serial
chip-vendors: []
required-tools: [lynx_workbench_status, lynx_serial_open, lynx_serial_read, lynx_serial_close]
description: Safely inspect a channel-bound Linux board UART without mixing it into flash protocol logs.
---
# Linux UART diagnostics

Start with `lynx_workbench_status` and use only the serial port bound to the current SDK channel.
Open the UART once, reuse its handle for bounded reads, and close only the handle created by the current workflow.
UART output belongs in the serial terminal; FEL/FES transport and verification output belongs in the flash console.
Never infer a successful boot from USB transfer completion alone.
