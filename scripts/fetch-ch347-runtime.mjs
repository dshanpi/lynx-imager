import { createHash } from 'node:crypto';
import { existsSync, mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const output = resolve(scriptDir, '../src-tauri/resources/windows/CH347DLLA64.dll');
const sourceUrl = 'https://file.wch.cn/download/file?id=7';
const archiveSha256 = '1e50a624f3c2d77a5f7a10eed5afffd5efde927bf4cd91306e769e4cf866f0fd';
const dllSha256 = 'd08c1ccd0146376a55602d9f2038dabc39c36af95cf7a36f11b17d42f8420388';
const runtimeTarget = process.env.LYNX_RUNTIME_TARGET?.trim();

if (runtimeTarget ? !runtimeTarget.startsWith('windows-') : process.platform !== 'win32') {
  console.log('WCH CH347 DLL is only required for Windows targets; skipping.');
  process.exit(0);
}

function sha256(path) {
  return createHash('sha256').update(readFileSync(path)).digest('hex');
}

if (existsSync(output) && sha256(output) === dllSha256) {
  console.log('WCH CH347DLLA64.dll 2.6 already prepared.');
  process.exit(0);
}

const temporary = mkdtempSync(join(tmpdir(), 'lynx-ch347-'));
try {
  const archive = join(temporary, 'CH341PAR.ZIP');
  const response = await fetch(sourceUrl);
  if (!response.ok) throw new Error(`WCH download failed: HTTP ${response.status}`);
  writeFileSync(archive, Buffer.from(await response.arrayBuffer()));
  if (sha256(archive) !== archiveSha256) {
    throw new Error('WCH CH341PAR.ZIP checksum mismatch; refusing an unverified DLL.');
  }
  const extracted = spawnSync(
    'powershell.exe',
    [
      '-NoProfile',
      '-NonInteractive',
      '-Command',
      'Expand-Archive -LiteralPath $env:LYNX_CH347_ARCHIVE -DestinationPath $env:LYNX_CH347_DESTINATION -Force',
    ],
    {
      encoding: 'utf8',
      env: {
        ...process.env,
        LYNX_CH347_ARCHIVE: archive,
        LYNX_CH347_DESTINATION: temporary,
      },
    }
  );
  if (extracted.status !== 0) {
    throw new Error(`Failed to extract WCH DLL: ${extracted.stderr || extracted.stdout}`);
  }
  const source = join(temporary, 'CH341PAR/CH347DLLA64.DLL');
  if (sha256(source) !== dllSha256) throw new Error('Extracted WCH DLL checksum mismatch.');
  mkdirSync(dirname(output), { recursive: true });
  writeFileSync(output, readFileSync(source));
  console.log('Prepared verified WCH CH347DLLA64.dll 2.6.');
} finally {
  rmSync(temporary, { recursive: true, force: true });
}
