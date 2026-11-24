#!/usr/bin/env python3
"""Stage Apple Silicon sidecars from pinned public sources; never run a device task.

See docs/engineering/imager-macos.md for source builds and prerequisites.
Only generated resources/binaries files are replaced. Source checkouts are read-only.
"""
import argparse
import hashlib
import json
import shutil
import subprocess
from pathlib import Path

PINS = {
    'openixcli': '4f869200b72d8fecbf6d420326495ff6baa42810',
    'openrockcli': 'c081a0987c0b4dde02fe104894c7c3d8989b9368',
}
UUU_SHA256 = '7f2ce10d5e738af635b339696709e2c3df4984801cc6c3764cd1c207c7aac9b0'


def run(*args):
    return subprocess.check_output(args, text=True, stderr=subprocess.STDOUT).strip()


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sources', type=Path, required=True)
    parser.add_argument('--brew-prefix', type=Path, default=Path('/opt/homebrew'))
    args = parser.parse_args()
    sources = args.sources.resolve(strict=True)
    prefix = args.brew_prefix.resolve(strict=True)
    output = Path(__file__).resolve().parents[1] / 'src-tauri/resources/binaries'
    inputs = {}
    for project, commit in PINS.items():
        checkout = sources / project
        if run('git', '-C', str(checkout), 'rev-parse', 'HEAD') != commit:
            raise SystemExit(f'{project}: unexpected source commit')
        if run('git', '-C', str(checkout), 'diff', 'HEAD', '--'):
            raise SystemExit(f'{project}: source has local changes')
    uuu = sources / 'uuu/uuu_mac_arm'
    if sha(uuu) != UUU_SHA256:
        raise SystemExit('UUU upstream digest mismatch')
    inputs.update({
        'openixcli': sources / 'openixcli/target/release/openixcli',
        'openrockcli': sources / 'openrockcli/openrockcli',
        'uuu': uuu,
        'dfu-util': prefix / 'opt/dfu-util/bin/dfu-util',
        'libusb-1.0.dylib': prefix / 'opt/libusb/lib/libusb-1.0.0.dylib',
        'libssl.3.dylib': prefix / 'opt/openssl@3/lib/libssl.3.dylib',
        'libcrypto.3.dylib': prefix / 'opt/openssl@3/lib/libcrypto.3.dylib',
        'libzstd.1.dylib': prefix / 'opt/zstd/lib/libzstd.1.dylib',
        'libtinyxml2.11.dylib': prefix / 'opt/tinyxml2/lib/libtinyxml2.11.dylib',
    })
    licenses = {
        'openixcli-LICENSE': sources / 'openixcli/LICENSE',
        'openrockcli-LICENSE': sources / 'openrockcli/LICENSE',
        'uuu-LICENSE': sources / 'uuu/LICENSE',
        'dfu-util-COPYING': prefix / 'opt/dfu-util/COPYING',
        'libusb-COPYING': prefix / 'opt/libusb/COPYING',
        'openssl-LICENSE': prefix / 'opt/openssl@3/LICENSE.txt',
        'zstd-LICENSE': prefix / 'opt/zstd/LICENSE',
        'tinyxml2-LICENSE': prefix / 'opt/tinyxml2/LICENSE.txt',
    }
    # Validate all inputs before changing generated resources.
    for path in [*inputs.values(), *licenses.values()]:
        if not path.is_file():
            raise SystemExit(f'Missing prerequisite: {path}')
    for path in inputs.values():
        if 'arm64' not in run('lipo', '-archs', str(path)).split():
            raise SystemExit(f'Not an arm64 runtime: {path.name}')
    output.mkdir(parents=True, exist_ok=True)
    aliases = {p.name: name for name, p in inputs.items() if name.endswith('.dylib')}
    for name, source in inputs.items():
        target = output / name
        shutil.copy2(source, target)
        target.chmod(0o755)
        dependencies = run('otool', '-L', str(target)).splitlines()[1:]
        for line in dependencies:
            dependency = line.strip().split(' (', 1)[0]
            if not dependency.startswith('/') or dependency.startswith(('/System/Library/', '/usr/lib/')):
                continue
            mapped = aliases.get(Path(dependency).name)
            if not mapped:
                raise SystemExit(f'Unbundled library: {name}: {dependency}')
            run('install_name_tool', '-change', dependency, '@loader_path/' + mapped, str(target))
        if name.endswith('.dylib'):
            run('install_name_tool', '-id', '@loader_path/' + name, str(target))
        run('codesign', '--force', '--sign', '-', str(target))
    for name, source in licenses.items():
        shutil.copy2(source, output / name)
    manifest = {'architecture': 'arm64', 'sourceCommits': PINS,
                'uuuRelease': 'uuu_1.5.243', 'uuuUpstreamSha256': UUU_SHA256,
                'files': {name: {'sourceSha256': sha(path), 'bundledSha256': sha(output / name)}
                          for name, path in {**inputs, **licenses}.items()}}
    (output / 'macos-runtime.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print(json.dumps(manifest, indent=2))


if __name__ == '__main__':
    main()
