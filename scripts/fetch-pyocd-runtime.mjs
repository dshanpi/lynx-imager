import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

const PYOCD_VERSION = '0.37.0';
const PACK_VERSION = '6.5.1';
const PACK_SHA256 = 'c5f3a3308426cb7667d599d9b88c810c4f4c6c2627baf33b8f7d050b8a72afd9';
const PACK_NAME = `Renesas.RA_DFP.${PACK_VERSION}.pack`;
const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(scriptDir, '..');

function hostTarget() {
  return {
    'win32-x64': 'windows-x86_64',
    'linux-x64': 'linux-x86_64',
    'darwin-arm64': 'darwin-aarch64',
  }[`${process.platform}-${process.arch}`];
}

function requestedTarget() {
  const argument = process.argv.find((value) => value.startsWith('--target='));
  return argument?.slice('--target='.length) || process.env.LYNX_RUNTIME_TARGET || hostTarget();
}

function sha256(path) {
  return createHash('sha256').update(readFileSync(path)).digest('hex');
}

function run(command, args) {
  const result = spawnSync(command, args, { encoding: 'utf8', stdio: 'inherit' });
  if (result.status !== 0) throw new Error(`${command} failed with exit code ${result.status}`);
}

async function main() {
  const target = requestedTarget();
  if (!['linux-x86_64', 'windows-x86_64', 'darwin-aarch64'].includes(target)) {
    throw new Error(`Unsupported pyOCD runtime target: ${target || 'unknown'}`);
  }
  const runtimeRoot = join(repoRoot, 'src-tauri', 'resources', 'runtime', target);
  const pyembed = join(runtimeRoot, 'pyembed');
  const python = join(
    pyembed,
    ...(target.startsWith('windows') ? ['python.exe'] : ['bin', 'python3'])
  );
  if (!existsSync(python)) throw new Error(`Python runtime is missing: ${python}`);

  const markerPath = join(pyembed, '.lynx-pyocd.json');
  const expectedMarker = { version: PYOCD_VERSION, target };
  const pyocd = join(
    pyembed,
    ...(target.startsWith('windows') ? ['Scripts', 'pyocd.exe'] : ['bin', 'pyocd'])
  );
  const pyocdModule = join(
    pyembed,
    ...(target.startsWith('windows')
      ? ['Lib', 'site-packages', 'pyocd', '__main__.py']
      : ['lib', 'python3.12', 'site-packages', 'pyocd', '__main__.py'])
  );
  const markerMatches =
    existsSync(markerPath) &&
    JSON.stringify(JSON.parse(readFileSync(markerPath, 'utf8'))) === JSON.stringify(expectedMarker);
  if (!markerMatches || (!existsSync(pyocd) && !existsSync(pyocdModule))) {
    if (target === hostTarget()) {
      run(python, [
        '-m',
        'pip',
        'install',
        '--disable-pip-version-check',
        '--no-compile',
        `pyocd==${PYOCD_VERSION}`,
      ]);
      run(pyocd, ['--version']);
    } else if (target === 'windows-x86_64') {
      const buildPython = join(
        repoRoot,
        'src-tauri',
        'resources',
        'runtime',
        hostTarget(),
        'pyembed',
        ...(process.platform === 'win32' ? ['python.exe'] : ['bin', 'python3'])
      );
      if (!existsSync(buildPython)) {
        throw new Error(`Host Python runtime is missing for cross-target pyOCD: ${buildPython}`);
      }
      const sitePackages = join(pyembed, 'Lib', 'site-packages');
      rmSync(sitePackages, { recursive: true, force: true });
      mkdirSync(sitePackages, { recursive: true });
      run(buildPython, [
        '-m',
        'pip',
        'install',
        '--disable-pip-version-check',
        '--no-compile',
        '--only-binary=:all:',
        '--platform',
        'win_amd64',
        '--implementation',
        'cp',
        '--python-version',
        '3.12',
        '--target',
        sitePackages,
        `pyocd==${PYOCD_VERSION}`,
      ]);
      if (!existsSync(pyocdModule)) {
        throw new Error('Cross-target pyOCD installation did not produce the pyocd module');
      }
    } else {
      throw new Error(`Cross-target pyOCD preparation is not supported for ${target}`);
    }
    writeFileSync(markerPath, JSON.stringify(expectedMarker, null, 2));
  }

  const packDirectory = join(runtimeRoot, 'cmsis-packs');
  const packPath = join(packDirectory, PACK_NAME);
  mkdirSync(packDirectory, { recursive: true });
  if (!existsSync(packPath) || sha256(packPath) !== PACK_SHA256) {
    const response = await fetch(`https://www2.renesas.eu/Keil_MDK_Packs/${PACK_NAME}`, {
      redirect: 'follow',
      signal: AbortSignal.timeout(5 * 60_000),
    });
    if (!response.ok) throw new Error(`Renesas RA_DFP download failed: HTTP ${response.status}`);
    const data = Buffer.from(await response.arrayBuffer());
    const actual = createHash('sha256').update(data).digest('hex');
    if (actual !== PACK_SHA256) {
      throw new Error(
        `Renesas RA_DFP checksum mismatch: expected ${PACK_SHA256}, received ${actual}`
      );
    }
    writeFileSync(packPath, data);
  }
  console.log(`Prepared pyOCD ${PYOCD_VERSION} and ${PACK_NAME} for ${target}.`);
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : error);
  process.exitCode = 1;
});
