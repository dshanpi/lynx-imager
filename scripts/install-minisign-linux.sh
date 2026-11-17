#!/usr/bin/env bash

set -euo pipefail

MINISIGN_VERSION=0.12
MINISIGN_LINUX_SHA256=9a599b48ba6eb7b1e80f12f36b94ceca7c00b7a5173c95c3efc88d9822957e73

task_root="$(mktemp -d "${RUNNER_TEMP:-/tmp}/lynx-minisign.XXXXXX")"
archive="$task_root/minisign-${MINISIGN_VERSION}-linux.tar.gz"
install_dir="$task_root/bin"

curl --fail --location --retry 3 \
  "https://github.com/jedisct1/minisign/releases/download/${MINISIGN_VERSION}/minisign-${MINISIGN_VERSION}-linux.tar.gz" \
  --output "$archive"
echo "$MINISIGN_LINUX_SHA256  $archive" | sha256sum --check --strict
tar --extract --gzip --file "$archive" --directory "$task_root" \
  "minisign-linux/x86_64/minisign"
install -D -m 0755 "$task_root/minisign-linux/x86_64/minisign" "$install_dir/minisign"
"$install_dir/minisign" -v

if [ -n "${GITHUB_PATH:-}" ]; then
  echo "$install_dir" >> "$GITHUB_PATH"
else
  echo "$install_dir"
fi
