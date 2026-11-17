#!/usr/bin/env python3
"""Stage a new ad-hoc signed arm64 DMG. Never replace an installed/running app.

Requires a completed Tauri production app bundle; does not grant release approval,
notarize the app, generate updater signatures, or publish a stable manifest.
"""
import argparse
import hashlib
import json
import plistlib
import shutil
import subprocess
import tempfile
from pathlib import Path


def run(*args):
    return subprocess.check_output(args, text=True, stderr=subprocess.STDOUT)


def digest(path):
    checksum = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            checksum.update(block)
    return checksum.hexdigest()


def linked_libraries(path):
    return [line.strip().split(' (', 1)[0]
            for line in run('otool', '-L', str(path)).splitlines()[1:]]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--app', type=Path, required=True)
    parser.add_argument('--output-dir', type=Path, required=True)
    parser.add_argument('--version', required=True)
    args = parser.parse_args()
    original = args.app.resolve(strict=True)
    info = plistlib.loads((original / 'Contents/Info.plist').read_bytes())
    if info['CFBundleShortVersionString'] != args.version:
        raise SystemExit('App version does not match requested version')
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=False)
    stage = output / 'stage'
    stage.mkdir()
    app = stage / 'LYNX.app'
    shutil.copytree(original, app, symlinks=True)
    binary = app / 'Contents/MacOS' / info['CFBundleExecutable']
    if run('lipo', '-archs', str(binary)).strip() != 'arm64':
        raise SystemExit('Only verified Apple Silicon arm64 bundles are supported')

    # The libefex system build links Homebrew libusb; make that dependency portable.
    frameworks = app / 'Contents/Frameworks'
    frameworks.mkdir(exist_ok=True)
    for dependency in linked_libraries(binary):
        if dependency.startswith('/opt/homebrew/') and 'libusb-1.0' in dependency:
            source = Path(dependency)
            target = frameworks / source.name
            shutil.copy2(source, target)
            run('install_name_tool', '-change', dependency,
                '@executable_path/../Frameworks/' + target.name, str(binary))
            run('install_name_tool', '-id', '@rpath/' + target.name, str(target))
            license_file = source.parent.parent / 'COPYING'
            if not license_file.is_file():
                raise SystemExit('Missing libusb license')
            shutil.copy2(license_file, app / 'Contents/Resources/libusb-COPYING')
            run('codesign', '--force', '--sign', '-', str(target))

    magic = {b'\xcf\xfa\xed\xfe', b'\xfe\xed\xfa\xcf', b'\xca\xfe\xba\xbe', b'\xbe\xba\xfe\xca'}
    checked = 0
    for item in app.rglob('*'):
        if item.is_symlink() or not item.is_file():
            continue
        with item.open('rb') as stream:
            if stream.read(4) not in magic:
                continue
        checked += 1
        for dependency in linked_libraries(item):
            if dependency.startswith('/') and not dependency.startswith(('/System/Library/', '/usr/lib/')):
                raise SystemExit('External library in ' + str(item.relative_to(app)) + ': ' + dependency)

    run(str(app / 'Contents/Resources/pyembed/bin/python3'), '-B', '-c',
        'import multiprocessing, sqlite3, ssl')
    run('codesign', '--force', '--sign', '-', '--options', '0', str(app))
    run('codesign', '--verify', '--deep', '--strict', str(app))
    (stage / 'Applications').symlink_to('/Applications')
    (stage / '安装说明.txt').write_text(
        f'LYNX {args.version} / macOS Apple Silicon arm64 手动安装候选\n'
        '将 LYNX.app 拖入 Applications。替换或关闭旧应用前请确认没有运行中的硬件任务。\n'
        '本包使用 ad-hoc 签名，未 Apple 公证；系统可能阻止打开。请按组织的测试安装流程处理。\n'
        '本包未进入自动更新，不包含正式 updater 签名。真实硬件与升级验收请另行安排。\n', encoding='utf-8')
    dmg = output / f'LYNX_{args.version}_aarch64.dmg'
    run('hdiutil', 'create', '-volname', f'LYNX {args.version} arm64',
        '-srcfolder', str(stage), '-format', 'UDZO', str(dmg))
    run('hdiutil', 'verify', str(dmg))
    with tempfile.TemporaryDirectory(prefix='lynx-dmg-check-') as mount:
        run('hdiutil', 'attach', '-readonly', '-nobrowse', '-mountpoint', mount, str(dmg))
        try:
            mounted = Path(mount) / 'LYNX.app'
            run('codesign', '--verify', '--deep', '--strict', str(mounted))
            if digest(mounted / 'Contents/MacOS' / info['CFBundleExecutable']) != digest(binary):
                raise SystemExit('Mounted binary differs from staged app')
        finally:
            run('hdiutil', 'detach', mount)
    (output / 'SHA256SUMS.txt').write_text(digest(dmg) + '  ' + dmg.name + '\n')
    check = {'version':args.version, 'architecture':'arm64', 'dmg':dmg.name,
             'bytes':dmg.stat().st_size, 'sha256':digest(dmg), 'machoDependencyChecks':checked,
             'externalNonSystemAbsoluteLibraries':0, 'embeddedPythonImports':'PASS',
             'adHocSignature':'PASS', 'readOnlyDmgMount':'PASS', 'mountedBinaryDigest':'PASS',
             'notarization':'NOT VERIFIED', 'updaterSignature':'NOT VERIFIED',
             'nativeDesktopSmoke':'NOT VERIFIED', 'cleanInstallUpgrade':'NOT VERIFIED'}
    (output / 'PACKAGE-CHECK.json').write_text(json.dumps(check, ensure_ascii=False, indent=2)+'\n')
    print(json.dumps(check, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()
