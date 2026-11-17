import { createHash } from 'node:crypto';
import {
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

const RELEASE = '20260718';
const VERSION = '3.12.13';
const DISTRIBUTIONS = {
  'windows-x86_64': {
    triple: 'x86_64-pc-windows-msvc',
    sha256: '0d422a1439ec308e03f47df551bc30f5994727c456e414b026d202bcda9b7c1c',
    interpreter: 'python.exe',
  },
  'linux-x86_64': {
    triple: 'x86_64-unknown-linux-gnu',
    sha256: '5854aa6ec71cad00334d5065633c210b2e7feb40956767a59a91791cadcf0b79',
    interpreter: 'bin/python3',
  },
  'darwin-aarch64': {
    triple: 'aarch64-apple-darwin',
    sha256: '9a1e9e06175c10efd8378b904b07fa21bd791ab3345d7cdffeb4a76c9ff55903',
    interpreter: 'bin/python3',
  },
};

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(scriptDir, '..');

function hostTarget() {
  const key = `${process.platform}-${process.arch}`;
  const targets = {
    'win32-x64': 'windows-x86_64',
    'linux-x64': 'linux-x86_64',
    'darwin-arm64': 'darwin-aarch64',
  };
  return targets[key];
}

function parseTarget() {
  const targetArg = process.argv.find((value) => value.startsWith('--target='));
  return targetArg?.slice('--target='.length) || process.env.LYNX_RUNTIME_TARGET || hostTarget();
}

async function download(url, output) {
  const response = await fetch(url, {
    redirect: 'follow',
    signal: AbortSignal.timeout(5 * 60_000),
  });
  if (!response.ok) {
    throw new Error(
      `Python runtime download failed: HTTP ${response.status} ${response.statusText}`
    );
  }
  writeFileSync(output, Buffer.from(await response.arrayBuffer()));
}

function sha256(path) {
  return createHash('sha256').update(readFileSync(path)).digest('hex');
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    encoding: 'utf8',
    stdio: options.capture ? 'pipe' : 'inherit',
    ...options,
  });
  if (result.status !== 0) {
    throw new Error(
      `${command} failed with exit code ${result.status}: ${result.stderr || result.stdout || ''}`
    );
  }
  return result;
}

function validateCrossTargetInterpreter(target, interpreterPath) {
  const header = readFileSync(interpreterPath).subarray(0, 4);
  const valid =
    (target === 'windows-x86_64' && header.subarray(0, 2).equals(Buffer.from('MZ'))) ||
    (target === 'linux-x86_64' && header.equals(Buffer.from([0x7f, 0x45, 0x4c, 0x46]))) ||
    (target === 'darwin-aarch64' &&
      (header.equals(Buffer.from([0xcf, 0xfa, 0xed, 0xfe])) ||
        header.equals(Buffer.from([0xfe, 0xed, 0xfa, 0xcf]))));
  if (!valid) {
    throw new Error(`Extracted runtime interpreter has the wrong binary format for ${target}`);
  }
}

function pruneUnsupportedModules(target, runtimeRoot) {
  if (target !== 'linux-x86_64') {
    return;
  }

  // Tauri's Linux bundler dereferences resource symlinks. Keeping the
  // python/python3 -> python3.12 aliases would therefore put three physical
  // copies of the 30 MiB interpreter in AppImage/deb/rpm, and the unversioned
  // libpython development link would duplicate the shared library as well.
  // LYNX resolves only bin/python3, so normalize that path to the one real
  // executable before bundling and discard build-time aliases/config helpers.
  const binRoot = join(runtimeRoot, 'bin');
  const versionedInterpreter = join(binRoot, `python${VERSION.slice(0, 4)}`);
  const bundledInterpreter = join(binRoot, 'python3');
  rmSync(join(binRoot, 'python'), { force: true });
  rmSync(bundledInterpreter, { force: true });
  renameSync(versionedInterpreter, bundledInterpreter);
  rmSync(join(binRoot, 'python3-config'), { force: true });
  rmSync(join(binRoot, `python${VERSION.slice(0, 4)}-config`), { force: true });
  rmSync(join(runtimeRoot, 'lib', `libpython${VERSION.slice(0, 4)}.so`), { force: true });

  // linuxdeploy scans every ELF in the AppDir. This optional Tk module links
  // to Tcl 9, which Ubuntu 22.04 does not provide and LYNX does not use.
  const extensionRoot = join(runtimeRoot, 'lib', `python${VERSION.slice(0, 4)}`, 'lib-dynload');
  for (const entry of readdirSync(extensionRoot)) {
    if (entry.startsWith('_tkinter.')) {
      rmSync(join(extensionRoot, entry), { force: true });
    }
  }
}

async function main() {
  const target = parseTarget();
  const distribution = DISTRIBUTIONS[target];
  if (!distribution) {
    throw new Error(
      `Unsupported Python runtime target "${target || 'unknown'}". ` +
        `Supported targets: ${Object.keys(DISTRIBUTIONS).join(', ')}`
    );
  }

  const destination = join(repoRoot, 'src-tauri', 'resources', 'runtime', target, 'pyembed');
  const markerPath = join(destination, '.lynx-runtime.json');
  const legacyPyocdMarkerPath = join(destination, '.lynx-pyocd.json');
  const interpreterPath = join(destination, ...distribution.interpreter.split('/'));
  const expectedMarker = {
    source: 'astral-sh/python-build-standalone',
    release: RELEASE,
    version: VERSION,
    target,
    sha256: distribution.sha256,
    layout: target === 'linux-x86_64' ? 4 : 3,
  };

  if (existsSync(markerPath) && existsSync(interpreterPath)) {
    const marker = JSON.parse(readFileSync(markerPath, 'utf8'));
    if (
      JSON.stringify(marker) === JSON.stringify(expectedMarker) &&
      !existsSync(legacyPyocdMarkerPath)
    ) {
      console.log(`Python ${VERSION} runtime already prepared for ${target}.`);
      return;
    }
    if (existsSync(legacyPyocdMarkerPath)) {
      console.log('Python runtime contains legacy pyOCD packages; rebuilding the core runtime.');
    }
  }

  const archiveName =
    `cpython-${VERSION}+${RELEASE}-${distribution.triple}` + '-install_only_stripped.tar.gz';
  const url =
    `https://github.com/astral-sh/python-build-standalone/releases/download/${RELEASE}/` +
    encodeURIComponent(archiveName);
  const targetRoot = dirname(destination);
  const uniqueSuffix = `${process.pid}-${Date.now()}`;
  const stagedName = `.pyembed-${uniqueSuffix}`;
  const stagedDestination = join(targetRoot, stagedName);
  const stagedInterpreter = join(stagedDestination, ...distribution.interpreter.split('/'));
  const stagedArchiveName = `.python-runtime-${uniqueSuffix}.tar.gz`;
  const archivePath = join(targetRoot, stagedArchiveName);

  mkdirSync(stagedDestination, { recursive: true });
  try {
    console.log(`Downloading Python ${VERSION} runtime for ${target}...`);
    await download(url, archivePath);
    const actualHash = sha256(archivePath);
    if (actualHash !== distribution.sha256) {
      throw new Error(
        `Python runtime checksum mismatch for ${target}: expected ` +
          `${distribution.sha256}, received ${actualHash}`
      );
    }

    // Keep every tar argument relative to its working directory. This avoids GNU
    // tar treating a Windows drive letter as a remote host, and extracting in
    // place preserves the runtime's relative symlinks on Unix.
    run('tar', ['-xzf', stagedArchiveName, '-C', stagedName, '--strip-components=1'], {
      cwd: targetRoot,
    });
    if (!existsSync(stagedInterpreter)) {
      throw new Error(`Extracted runtime is missing ${distribution.interpreter}`);
    }

    pruneUnsupportedModules(target, stagedDestination);
    if (target === hostTarget()) {
      run(stagedInterpreter, [
        '-c',
        'import multiprocessing, sqlite3, ssl, sys; assert sys.version_info[:2] == (3, 12)',
      ]);
    } else {
      validateCrossTargetInterpreter(target, stagedInterpreter);
    }
    writeFileSync(
      join(stagedDestination, '.lynx-runtime.json'),
      JSON.stringify(expectedMarker, null, 2)
    );

    rmSync(destination, { recursive: true, force: true });
    renameSync(stagedDestination, destination);
    console.log(`Prepared Python ${VERSION} runtime at ${destination}.`);
  } finally {
    rmSync(archivePath, { force: true });
    rmSync(stagedDestination, { recursive: true, force: true });
  }
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : error);
  process.exitCode = 1;
});
