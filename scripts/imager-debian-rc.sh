#!/usr/bin/env bash
# Normalize the Debian ordering of a Tauri SemVer prerelease before OTA signing.
set -euo pipefail
input=${1:?expected Tauri .deb}
output=${2:?expected output .deb}
version=$(dpkg-deb -f "$input" Version)
[[ $version =~ ^([0-9]+\.[0-9]+\.[0-9]+)-([a-z]+\.[0-9]+)$ ]]
debian="${BASH_REMATCH[1]}~${BASH_REMATCH[2]}"
dpkg --compare-versions "$debian" lt "${version%%-*}"
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT
dpkg-deb --raw-extract "$input" "$work"
sed -i "s/^Version:.*/Version: $debian/" "$work/DEBIAN/control"
dpkg-deb -Zxz -z4 --threads-max=3 --root-owner-group --build "$work" "$output"
test "$(dpkg-deb -f "$output" Version)" = "$debian"
