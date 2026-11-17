---
name: linux-incident-triage
version: 1.0.0
domain: linux
systems: [common, linux]
category: diagnostics/incidents
chip-vendors: []
required-tools: [lynx_workbench_status]
description: Triage a sanitized LYNX Linux-channel failure before proposing any retry, device action or code change.
---
# LYNX Linux incident triage

Treat the supplied incident packet as a pointer, not complete proof. Keep the saved project, device and board scope unchanged.

1. Read `errorCode`, `component`, `failureClass`, `resolutionKey`, `taskPhase` and `retryable`.
2. Call `lynx_workbench_status` and inspect only the current scoped channel. Do not scan or select another board.
3. Separate software defects from driver ownership, device disconnects, permissions, configuration and expected control flow.
4. If another specific Skill matches, read it before proposing detailed steps.
5. Propose the smallest read-only confirmation first. Never retry flashing, power, reset, serial writes or destructive commands automatically.
6. State what evidence is missing. Ask the operator before any physical intervention or hardware action.

For Rockchip open failures, distinguish access denied, incompatible driver, exclusive ownership and disconnect. Closing another flasher or changing a Windows driver is an operator action; do not claim that LYNX performed it.
