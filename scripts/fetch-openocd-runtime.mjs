import { createHash } from 'node:crypto';
import {
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

const VERSION = '0.12.0-7';
const DISTRIBUTIONS = {
  'linux-x86_64': {
    archive: `xpack-openocd-${VERSION}-linux-x64.tar.gz`,
    sha256: '94b3790983beaf8ed57e646c0620dd66d705fddae03d290823a6ed3b439468d6',
    executable: 'bin/openocd',
  },
  'windows-x86_64': {
    archive: `xpack-openocd-${VERSION}-win32-x64.zip`,
    sha256: '6bfd3c97135aafef8affc9af1acf34fd0e2b9ca26044506f6abd7f95b7630052',
    executable: 'bin/openocd.exe',
  },
  'darwin-aarch64': {
    archive: `xpack-openocd-${VERSION}-darwin-arm64.tar.gz`,
    sha256: '667342c086984f3e5a55b4e0d5f711add13fb04de040fca493303000e6c19327',
    executable: 'bin/openocd',
  },
};

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(scriptDir, '..');

function hostTarget() {
  return {
    'win32-x64': 'windows-x86_64',
    'linux-x64': 'linux-x86_64',
    'darwin-arm64': 'darwin-aarch64',
  }[`${process.platform}-${process.arch}`];
}

function targetName() {
  const argument = process.argv.find((value) => value.startsWith('--target='));
  return argument?.slice('--target='.length) || process.env.LYNX_RUNTIME_TARGET || hostTarget();
}

function sha256(path) {
  return createHash('sha256').update(readFileSync(path)).digest('hex');
}

function run(command, args, cwd, environment = {}) {
  const result = spawnSync(command, args, {
    cwd,
    encoding: 'utf8',
    env: { ...process.env, ...environment },
  });
  if (result.status !== 0) {
    throw new Error(`${command} failed: ${result.stderr || result.stdout || result.status}`);
  }
}

function extract(distribution, archive, staged, targetRoot, suffix) {
  if (!distribution.archive.endsWith('.zip')) {
    mkdirSync(staged, { recursive: true });
    run('tar', ['-xf', archive, '-C', staged, '--strip-components=1'], targetRoot);
    return;
  }

  const unpacked = join(targetRoot, `.openocd-unpacked-${suffix}`);
  mkdirSync(unpacked, { recursive: true });
  try {
    if (process.platform === 'win32') {
      run(
        'powershell.exe',
        [
          '-NoProfile',
          '-NonInteractive',
          '-Command',
          'Expand-Archive -LiteralPath $env:LYNX_OPENOCD_ARCHIVE -DestinationPath $env:LYNX_OPENOCD_DESTINATION -Force',
        ],
        targetRoot,
        {
          LYNX_OPENOCD_ARCHIVE: archive,
          LYNX_OPENOCD_DESTINATION: unpacked,
        }
      );
    } else {
      run('unzip', ['-q', archive, '-d', unpacked], targetRoot);
    }
    const entries = readdirSync(unpacked).filter((name) => name !== '__MACOSX');
    if (entries.length !== 1 || !statSync(join(unpacked, entries[0])).isDirectory()) {
      throw new Error('OpenOCD ZIP must contain exactly one distribution root directory');
    }
    renameSync(join(unpacked, entries[0]), staged);
  } finally {
    rmSync(unpacked, { recursive: true, force: true });
  }
}

function prepareForBundle(target, destination) {
  if (target !== 'linux-x86_64') return;

  // xPack ships an older private libudev for OpenOCD. linuxdeploy otherwise
  // promotes it into the AppImage's global usr/lib, where it is too old for
  // the host libgudev used by the desktop application. Use the system libudev
  // on Linux; the package metadata declares it as a runtime dependency.
  rmSync(join(destination, 'libexec', 'libudev.so.1'), { force: true });
  rmSync(join(destination, 'libexec', 'libudev.so.1.6.13'), { force: true });
}

async function main() {
  const target = targetName();
  const distribution = DISTRIBUTIONS[target];
  if (!distribution) throw new Error(`Unsupported OpenOCD runtime target: ${target || 'unknown'}`);

  const targetRoot = join(repoRoot, 'src-tauri', 'resources', 'runtime', target);
  const destination = join(targetRoot, 'openocd');
  const marker = join(destination, '.lynx-runtime.json');
  const expected = JSON.stringify({
    source: 'xpack-dev-tools/openocd-xpack',
    version: VERSION,
    target,
    sha256: distribution.sha256,
  });
  if (existsSync(marker) && existsSync(join(destination, distribution.executable))) {
    if (JSON.stringify(JSON.parse(readFileSync(marker, 'utf8'))) === expected) {
      prepareForBundle(target, destination);
      console.log(`OpenOCD ${VERSION} runtime already prepared for ${target}.`);
      return;
    }
  }

  mkdirSync(targetRoot, { recursive: true });
  const suffix = `${process.pid}-${Date.now()}`;
  const archive = join(
    targetRoot,
    `.openocd-${suffix}${distribution.archive.endsWith('.zip') ? '.zip' : '.tar.gz'}`
  );
  const staged = join(targetRoot, `.openocd-${suffix}`);
  try {
    const url = `https://github.com/xpack-dev-tools/openocd-xpack/releases/download/v${VERSION}/${distribution.archive}`;
    const response = await fetch(url, {
      redirect: 'follow',
      signal: AbortSignal.timeout(5 * 60_000),
    });
    if (!response.ok) throw new Error(`OpenOCD download failed: HTTP ${response.status}`);
    writeFileSync(archive, Buffer.from(await response.arrayBuffer()));
    const actual = sha256(archive);
    if (actual !== distribution.sha256) {
      throw new Error(
        `OpenOCD checksum mismatch: expected ${distribution.sha256}, received ${actual}`
      );
    }
    extract(distribution, archive, staged, targetRoot, suffix);
    if (!existsSync(join(staged, distribution.executable))) {
      throw new Error(`OpenOCD archive is missing ${distribution.executable}`);
    }
    prepareForBundle(target, staged);
    writeFileSync(join(staged, '.lynx-runtime.json'), `${expected}\n`);
    rmSync(destination, { recursive: true, force: true });
    renameSync(staged, destination);
    console.log(`Prepared verified OpenOCD ${VERSION} runtime for ${target}.`);
  } finally {
    rmSync(archive, { force: true });
    rmSync(staged, { recursive: true, force: true });
  }
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : error);
  process.exitCode = 1;
});
