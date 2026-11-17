#!/usr/bin/env bash
set -euo pipefail

remote=${DL_REMOTE:?DL_REMOTE is required}
expected='dl-release-command protocol=4'

command -v ssh >/dev/null || {
  echo "ssh is required to verify the release deployment channel" >&2
  exit 2
}

actual=$(ssh -o BatchMode=yes -o IdentitiesOnly=yes "$remote" release-channel-probe-v4)
if [[ "$actual" != "$expected" ]]; then
  echo "restricted release deployment channel returned an unexpected protocol response" >&2
  exit 1
fi

echo "$expected verified"
