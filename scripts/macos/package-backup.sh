#!/bin/bash
# Package an already-built local app. Never modifies the app or running processes.
set -euo pipefail
if [ "$#" -ne 3 ]; then
  echo 'Usage: package-backup.sh /absolute/LYNX.app /absolute/output-directory package-name' >&2
  exit 2
fi
app_path="$1"
backup_dir="$2"
package_name="$3"
case "$app_path:$backup_dir" in /*:/*) ;; *) echo 'Use absolute paths.' >&2; exit 2;; esac
case "$package_name" in ''|*[!A-Za-z0-9._-]*) echo 'Invalid package name.' >&2; exit 2;; esac
[ "$(uname -s)" = Darwin ] || { echo 'macOS is required.' >&2; exit 2; }
[ -f "$app_path/Contents/Info.plist" ] || { echo 'LYNX.app is missing.' >&2; exit 2; }
[ ! -e "$backup_dir/$package_name.zip" ] && [ ! -e "$backup_dir/$package_name.dmg" ] || {
  echo 'Output already exists; choose a new package name.' >&2; exit 2;
}
codesign --verify --deep --strict "$app_path"
mkdir -p "$backup_dir"
staging_dir="$(mktemp -d "${TMPDIR:-/tmp}/lynx-macos-package.XXXXXX")"
trap 'rm -rf "$staging_dir"' EXIT
ditto "$app_path" "$staging_dir/LYNX.app"
ln -s /Applications "$staging_dir/Applications"
cat > "$staging_dir/安装说明.txt" <<'TXT'
LYNX macOS ARM64 本地备份版
将 LYNX.app 拖入 Applications。现有版本正在运行硬件任务时，请等待任务结束再退出/替换。
本包使用本地 ad-hoc 签名，未经过 Apple 公证，不是正式自动更新发布包。
不含用户数据库、串口记录、SSH 密码或系统钥匙串。
TXT
hdiutil create -quiet -volname 'LYNX macOS ARM64' -srcfolder "$staging_dir" -format UDZO "$backup_dir/$package_name.dmg"
ditto -c -k --sequesterRsrc --keepParent "$staging_dir/LYNX.app" "$backup_dir/$package_name.zip"
hdiutil verify "$backup_dir/$package_name.dmg"
unzip -tq "$backup_dir/$package_name.zip"
