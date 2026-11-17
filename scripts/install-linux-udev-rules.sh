#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
rule_source="${script_dir}/../packaging/udev/70-lynx-cmsis-dap.rules"
rule_target="/etc/udev/rules.d/70-lynx-cmsis-dap.rules"

install -m 0644 "${rule_source}" "${rule_target}"
udevadm control --reload-rules
udevadm trigger --subsystem-match=usb --attr-match=idVendor=1a86 --attr-match=idProduct=8011
