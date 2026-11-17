---
name: freertos-cherryusb
version: 1.0.0
domain: mcu
systems: [freertos]
category: rtos/packages/usb
chip-vendors: []
required-tools: [lynx_mcu_capabilities, lynx_mcu_board_info, lynx_mcu_uart_open, lynx_mcu_uart_read, lynx_mcu_uart_close]
description: Diagnose CherryUSB CDC stalls on a fixed MCU board using bounded UART observation and no implicit reset or flash.
---
# FreeRTOS CherryUSB CDC diagnostics

Read board capabilities and identity first. Observe the bound UART before changing USB initialization, interrupt priorities, cache/DMA ownership or task scheduling.
Separate enumeration failure, endpoint callback starvation, blocked transmit completion and application task deadlock. Capture bounded evidence around USB IRQ and CDC callbacks.
This skill is diagnostic only: it does not authorize target reset, firmware flashing, power control or arbitrary shell access.
