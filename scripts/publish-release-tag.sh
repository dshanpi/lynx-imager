#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]] || [[ ! "$1" =~ ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
  echo "usage: npm run release:stage -- vMAJOR.MINOR.PATCH" >&2
  exit 2
fi

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"
tag=$1
notes="docs/releases/$tag.md"

test -f "$notes" || {
  echo "release notes are missing: $notes" >&2
  exit 1
}
if grep -q 'TODO' "$notes"; then
  echo "release notes still contain TODO: $notes" >&2
  exit 1
fi
if [[ -n "$(git status --porcelain)" ]]; then
  echo "working tree is not clean; review and commit the release changes first" >&2
  exit 1
fi

npm run validate:release -- "$tag"
npm run verify:release -- "$tag"

branch=$(git branch --show-current)
test -n "$branch" || {
  echo "detached HEAD cannot be released" >&2
  exit 1
}
remote_url=$(git remote get-url origin)
case "$remote_url" in
  https://github.com/dshanpi/lynx.git | git@github.com:dshanpi/lynx.git) ;;
  *)
    echo "origin is not the private dshanpi/lynx source repository" >&2
    exit 1
    ;;
esac

git fetch --tags origin
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null ||
  git ls-remote --exit-code --tags origin "refs/tags/$tag" >/dev/null 2>&1; then
  echo "release tag already exists: $tag" >&2
  exit 1
fi

git push origin "HEAD:$branch"
git tag -a "$tag" -m "DshanPI LYNX｜芯桥 $tag"
if ! git push origin "refs/tags/$tag"; then
  git tag -d "$tag" >/dev/null
  echo "tag push failed; the local tag was removed and no release was triggered" >&2
  exit 1
fi

echo "$tag has been pushed. GitHub Actions now builds, signs and verifies a GitHub Draft only."
echo "Stable channels remain unchanged until promote-release.yml is started manually."
