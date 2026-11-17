#!/usr/bin/env python3
"""Synthetic BSP lesson embedded only in signed test packages. Never deploys."""
import hashlib
import json
from pathlib import Path
import shutil
import struct
import subprocess
import sys

TOOLS = ('dtc', 'fdtget', 'arm-linux-gnueabihf-gcc', 'arm-linux-gnueabihf-objcopy')
INPUTS = ('src/main.c', 'src/board.dts', 'scripts/verify.sh', 'scripts/bsp_lesson.py')
ARTIFACTS = ('board.dtb', 'config.o', 'config.bin')
VERIFICATION_FILES = ('roundtrip.dts', 'roundtrip.dtb', 'verified-config.bin')
MEMORY_BYTES = 64 * 1024 * 1024


def sha(data):
    return hashlib.sha256(data).hexdigest()


def read(path, limit=2 * 1024 * 1024):
    path = Path(path)
    if path.is_symlink() or not path.is_file() or path.stat().st_size > limit:
        raise ValueError('Missing, linked or oversized lesson file: ' + str(path))
    with path.open('rb') as file:
        raw = file.read(limit + 1)
    if len(raw) > limit:
        raise ValueError('Lesson file exceeds read limit')
    return raw


def command(args):
    try:
        result = subprocess.run(args, capture_output=True, timeout=20, check=False)
    except FileNotFoundError:
        raise ValueError('Required tool unavailable: ' + args[0]) from None
    if result.returncode:
        raise ValueError(result.stderr.decode(errors='replace')[-4096:])
    return result.stdout.decode()


def tool_facts():
    facts = {}
    for name in TOOLS:
        location = shutil.which(name)
        if location is None:
            raise ValueError('Required tool unavailable: ' + name)
        path = Path(location).resolve(strict=True)
        facts[name] = {'version': command([str(path), '--version']).splitlines()[0],
                       'executableSha256': sha(read(path, 64 * 1024 * 1024)),
                       'pathSha256': sha(str(path).encode())}
    target = command(['arm-linux-gnueabihf-gcc', '-dumpmachine']).strip()
    if target != 'arm-linux-gnueabihf':
        raise ValueError('Expected ARM Linux hard-float cross compiler')
    return {'target': target, 'tools': facts}


def source_facts():
    return {name: sha(read(name)) for name in INPUTS}


def artifact_facts():
    return {name: sha(read(Path('build') / name)) for name in ARTIFACTS}


def device_tree(path):
    model = command(['fdtget', '-t', 's', str(path), '/', 'model']).strip()
    compatible = command(['fdtget', '-t', 's', str(path), '/', 'compatible']).strip()
    if model != 'LYNX synthetic BSP fixture' or compatible != 'lynx,bsp-fixture':
        raise ValueError('Expected synthetic fixture model and compatible')
    memory = command(['fdtget', '-t', 'x', str(path), '/memory@40000000', 'reg']).split()
    if [int(value, 16) for value in memory] != [0x40000000, MEMORY_BYTES]:
        raise ValueError('Expected fixture memory base 0x40000000 and 64 MiB')
    return {'model': model, 'compatible': compatible, 'memoryBase': 0x40000000,
            'memoryBytes': MEMORY_BYTES}


def arm_configuration(output):
    elf = read('build/config.o')
    if len(elf) < 52 or elf[:6] != b'\x7fELF\x01\x01' or struct.unpack_from('<HH', elf, 16) != (1, 40):
        raise ValueError('Expected little-endian ARM32 relocatable ELF')
    command(['arm-linux-gnueabihf-objcopy', '-O', 'binary', '--only-section=.rodata',
             'build/config.o', str(output)])
    raw = read(output)
    if raw != struct.pack('<II', 0x4c594e58, MEMORY_BYTES):
        raise ValueError('ARM configuration differs from expected fixture memory')
    return {'architecture': 'ARM32', 'elfType': 'relocatable', 'elfMachine': 40,
            'configMemoryBytes': MEMORY_BYTES, 'configSha256': sha(raw)}


def remove(names):
    for name in names:
        (Path('build') / name).unlink(missing_ok=True)


def save(step, facts):
    text = json.dumps({'synthetic': True, 'hardwareActions': False, **facts}, sort_keys=True)
    temporary = Path('build') / (step + '-metrics.tmp')
    temporary.write_text(text + '\n')
    temporary.replace(Path('build') / (step + '-metrics.json'))
    print(text)


def main(step):
    if step not in ('source', 'build', 'run'):
        raise ValueError('Unknown lesson action')
    Path('build').mkdir(exist_ok=True)
    downstream = {'source': ('source', 'build', 'run'), 'build': ('build', 'run'), 'run': ('run',)}[step]
    remove(tuple(name + '-metrics.json' for name in downstream) + VERIFICATION_FILES)
    if step in ('source', 'build'):
        remove(ARTIFACTS)
    try:
        if step == 'run' and not Path('build/build-metrics.json').is_file():
            raise ValueError('Build evidence missing; compile the lesson first')
        tools = tool_facts()
        inputs = source_facts()
        if step == 'source':
            save(step, {'sourceSha256': inputs, 'toolchain': tools})
            return
        if step == 'build':
            command(['arm-linux-gnueabihf-gcc', '-ffreestanding', '-Wall', '-Wextra', '-Werror',
                     '-c', 'src/main.c', '-o', 'build/config.o'])
            command(['dtc', '-I', 'dts', '-O', 'dtb', '-o', 'build/board.dtb', 'src/board.dts'])
            facts = {**device_tree('build/board.dtb'), **arm_configuration('build/config.bin')}
            if source_facts() != inputs or tool_facts() != tools:
                raise ValueError('Build inputs changed during compilation')
            save(step, {**facts, 'sourceSha256': inputs, 'toolchain': tools, 'artifacts': artifact_facts()})
            return
        built = json.loads(read('build/build-metrics.json'))
        if built['sourceSha256'] != inputs:
            raise ValueError('Build inputs changed; compile again')
        if built['toolchain'] != tools:
            raise ValueError('Toolchain changed; compile again')
        artifacts = artifact_facts()
        if built['artifacts'] != artifacts:
            raise ValueError('Build artifact changed; compile again')
        facts = {**device_tree('build/board.dtb'), **arm_configuration('build/verified-config.bin')}
        if read('build/config.bin') != read('build/verified-config.bin'):
            raise ValueError('ARM extracted configuration changed')
        command(['dtc', '-I', 'dtb', '-O', 'dts', '-o', 'build/roundtrip.dts', 'build/board.dtb'])
        command(['dtc', '-I', 'dts', '-O', 'dtb', '-o', 'build/roundtrip.dtb', 'build/roundtrip.dts'])
        if device_tree('build/roundtrip.dtb') != device_tree('build/board.dtb'):
            raise ValueError('Device tree roundtrip changed properties')
        if source_facts() != inputs or artifact_facts() != artifacts or tool_facts() != tools:
            raise ValueError('Build inputs changed during verification')
        save(step, {**facts, 'sourceSha256': inputs, 'toolchain': tools, 'artifacts': artifacts,
                    'deviceTreeRoundtrip': True, 'roundtripSha256': sha(read('build/roundtrip.dtb'))})
    except Exception:
        remove(VERIFICATION_FILES)
        if step in ('source', 'build'):
            remove(ARTIFACTS)
        raise


if __name__ == '__main__':
    try:
        if len(sys.argv) != 2:
            raise ValueError('Exactly one lesson action is required')
        main(sys.argv[1])
    except (ValueError, KeyError, TypeError, OSError, subprocess.TimeoutExpired) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
