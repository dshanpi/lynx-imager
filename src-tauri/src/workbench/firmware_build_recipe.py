"""Fixed NekoBox recipe; invoked inside the existing Workbench project lease."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time
import selectors
import signal

workspace = Path.cwd().resolve()
attempt = sys.argv[1]
if not attempt.startswith('fwbuild-') or not attempt.replace('-', '').isalnum():
    raise SystemExit('FIRMWARE_BUILD_ID_INVALID')
root = workspace / '.lynx-firmware' / attempt
root.mkdir(parents=True, exist_ok=False)
receipt = {'schemaVersion': 1, 'id': attempt, 'recipe': 'nekobox-xip-v1',
           'status': 'running', 'stages': [], 'artifacts': [], 'startedAt': int(time.time()),
           'hardwareTest': 'not_verified'}

def sha(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for b in iter(lambda: f.read(1024 * 1024), b''):
            h.update(b)
    return h.hexdigest()

def save():
    temporary = root / 'receipt.json.part'
    temporary.write_text(json.dumps(receipt, ensure_ascii=False, indent=2), encoding='utf8')
    temporary.replace(root / 'receipt.json')

def capture(args):
    return subprocess.check_output(args, text=True, stderr=subprocess.STDOUT, timeout=60).strip()

def sources():
    result = {}
    for name in ['zephyr', 'zephyr-components', 'SyterKit']:
        path = workspace / name
        if capture(['git', '-C', str(path), 'status', '--porcelain', '--untracked-files=normal']):
            raise RuntimeError('FIRMWARE_SOURCE_DIRTY:' + name)
        result[name] = {'commit': capture(['git', '-C', str(path), 'rev-parse', 'HEAD']),
                        'tree': capture(['git', '-C', str(path), 'rev-parse', 'HEAD^{tree}'])}
        origin = capture(['git', '-C', str(path), 'remote', 'get-url', 'origin'])
        public_sources = {'zephyr': 'https://github.com/YuzukiHD/zephyr.git', 'zephyr-components': 'https://github.com/YuzukiHD/zephyr-components.git', 'SyterKit': 'https://github.com/YuzukiHD/SyterKit.git'}
        result[name]['origin'] = origin if origin == public_sources[name] else 'other-origin-sha256:' + hashlib.sha256(origin.encode()).hexdigest()
    # Lock every actually cloned west module, and explicitly record missing modules.
    # A minimal SDK legitimately omits unrelated boards' modules; never invent their SHA.
    modules = []
    for line in capture(['west', 'list', '--all', '-f', '{name}|{path}|{cloned}']).splitlines():
        name, relative, cloned = line.split('|')
        entry = {'name': name, 'path': relative, 'state': cloned}
        if cloned == 'cloned':
            path = (workspace / relative).resolve()
            path.relative_to(workspace)
            if capture(['git', '-C', str(path), 'status', '--porcelain', '--untracked-files=normal']):
                raise RuntimeError('FIRMWARE_MODULE_DIRTY:' + name)
            entry['commit'] = capture(['git', '-C', str(path), 'rev-parse', 'HEAD'])
            entry['tree'] = capture(['git', '-C', str(path), 'rev-parse', 'HEAD^{tree}'])
        modules.append(entry)
    frozen = json.dumps({'modules': modules, 'manifestSha256': sha(workspace / 'zephyr/west.yml')}, sort_keys=True, indent=2)
    (root / 'west-frozen.json').write_text(frozen, encoding='utf8')
    result['westManifestSha256'] = sha(root / 'west-frozen.json')
    result['environmentSha256'] = sha(workspace / 'env.sh')
    return result

def step(name, args):
    stage = {'name': name, 'status': 'running', 'startedAt': int(time.time())}
    receipt['stages'].append(stage)
    save()
    print('LYNX_FIRMWARE_STAGE:' + name, flush=True)
    try:
        with (root / 'build.log').open('ab') as log:
            p = subprocess.Popen(args, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, start_new_session=True)
            selector = selectors.DefaultSelector()
            selector.register(p.stdout, selectors.EVENT_READ)
            total = 0
            deadline = time.monotonic() + 1800
            try:
                while selector.get_map():
                    if time.monotonic() > deadline:
                        raise RuntimeError('FIRMWARE_BUILD_STAGE_TIMEOUT')
                    for key, _ in selector.select(timeout=1):
                        data = os.read(key.fd, 16384)
                        if not data:
                            selector.unregister(key.fileobj)
                            continue
                        total += len(data)
                        if total > 128 * 1024 * 1024:
                            raise RuntimeError('FIRMWARE_BUILD_LOG_LIMIT')
                        log.write(data)
                        sys.stdout.buffer.write(data)
                        sys.stdout.buffer.flush()
                code = p.wait(timeout=10)
                if code:
                    raise RuntimeError('FIRMWARE_BUILD_EXIT:' + str(code))
            finally:
                selector.close()
                if p.poll() is None:
                    os.killpg(p.pid, signal.SIGTERM)
                    try:
                        p.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        os.killpg(p.pid, signal.SIGKILL)
                        p.wait()
        stage['status'] = 'success'
    except Exception:
        stage['status'] = 'failed'
        raise
    finally:
        stage['finishedAt'] = int(time.time())
        save()

try:
    save()
    receipt['inputs'] = sources()
    compiler = os.environ['CROSS_COMPILE'] + 'gcc'
    receipt['toolchain'] = {'version': capture([compiler, '--version']).splitlines()[0],
                            'sha256': sha(compiler)}
    receipt['stages'].append({'name': 'source_lock', 'status': 'success', 'finishedAt': int(time.time())})
    save()
    os.environ.setdefault('CMAKE_BUILD_PARALLEL_LEVEL', '4')
    os.environ['PATH'] = str(Path.home() / '.cargo/bin') + os.pathsep + os.environ['PATH']
    app_build = root / 'app-build'
    boot_build = root / 'boot-build'
    step('application', ['west', 'build', '-b', 'yuzukineko', '-d', str(app_build),
                         str(workspace / 'zephyr-components/nekobox')])
    step('boot_config', ['make', '-C', str(workspace / 'SyterKit'), 'O=' + str(boot_build),
                         'CROSS_COMPILE=' + os.environ['CROSS_COMPILE'], 'yuzukineko_rv32_sram_defconfig'])
    step('boot', ['make', '-C', str(workspace / 'SyterKit'), '-j4', 'O=' + str(boot_build),
                 'CROSS_COMPILE=' + os.environ['CROSS_COMPILE'], 'xip-boot'])
    boot = boot_build / 'build/yuzukineko/app_sram/xip-boot/xip-boot_spi.bin'
    app = app_build / 'zephyr/zephyr.bin'
    image = root / 'nekobox-full.img'
    step('package', [sys.executable, str(workspace / 'zephyr-components/nekobox/tools/mkimage.py'),
                     '--boot', str(boot), '--app', str(app), '-o', str(image)])
    data = image.read_bytes()
    boot_bytes = boot.read_bytes()
    app_bytes = app.read_bytes()
    if not (0x80000 < len(data) <= 0x1000000) or data[:len(boot_bytes)] != boot_bytes or data[0x80000:0x80000+len(app_bytes)] != app_bytes:
        raise RuntimeError('FIRMWARE_IMAGE_LAYOUT_MISMATCH')
    if sources() != receipt['inputs']:
        raise RuntimeError('FIRMWARE_SOURCE_CHANGED_DURING_BUILD')
    for source, name in [(boot, 'xip-boot_spi.bin'), (app, 'nekobox.bin'),
                         (app_build / 'zephyr/zephyr.elf', 'nekobox.elf'),
                         (app_build / 'zephyr/zephyr.map', 'nekobox.map'),
                         (app_build / 'zephyr/.config', 'nekobox.config'),
                         (app_build / 'zephyr/zephyr.dts', 'nekobox.dts')]:
        shutil.copyfile(source, root / name)
    for name in ['nekobox-full.img', 'xip-boot_spi.bin', 'nekobox.bin', 'nekobox.elf', 'nekobox.map', 'nekobox.config', 'nekobox.dts', 'west-frozen.json']:
        p = root / name
        receipt['artifacts'].append({'fileName': name, 'sizeBytes': p.stat().st_size, 'sha256': sha(p)})
    receipt['stages'].append({'name': 'structure', 'status': 'success', 'finishedAt': int(time.time())})
    receipt['status'] = 'success'
except Exception as error:
    receipt['status'] = 'failed'
    receipt['error'] = type(error).__name__ + ':' + str(error)[:500]
    raise
finally:
    receipt['finishedAt'] = int(time.time())
    save()
