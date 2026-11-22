set -eu
# pkexec receives only the vendor and the fixed ID list from the Rust allowlist.
PATH=/usr/sbin:/usr/bin:/sbin:/bin
export PATH
vendor=$1
shift
target="/etc/udev/rules.d/70-lynx-imager-${vendor}.rules"
tmp=$(mktemp "${target}.XXXXXX")
trap 'rm -f "$tmp"' EXIT
for id do
  printf 'SUBSYSTEM=="usb", ENV{DEVTYPE}=="usb_device", ATTR{idVendor}=="%s", ATTR{idProduct}=="%s", TAG+="uaccess"\n' "${id%:*}" "${id#*:}"
done > "$tmp"
chmod 0644 "$tmp"
mv -fT "$tmp" "$target"
udevadm control --reload-rules
