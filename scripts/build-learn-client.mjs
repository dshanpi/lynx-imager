// Public trust roots only. Private signing keys remain on the course publisher.
import { readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { fileURLToPath, pathToFileURL } from 'node:url';

const fixtureKeys = JSON.parse(
  readFileSync(
    new URL('../docs/contracts/learn-v2/fixtures/test-trust-registry.json', import.meta.url),
    'utf8'
  )
);
export function validateLearnBuildKeys(raw) {
  const invalid = () =>
    new Error(
      '课程公钥文件无效：只接受最多 16 个唯一 keyId 和 32 字节 Ed25519 publicKey，禁止私钥和仓库测试密钥。'
    );
  if (Buffer.byteLength(raw) > 8192) throw invalid();
  let entries;
  try {
    entries = JSON.parse(raw);
  } catch {
    throw invalid();
  }
  if (!Array.isArray(entries) || !entries.length || entries.length > 16) throw invalid();
  const seen = new Set();
  for (const entry of entries) {
    if (
      !entry ||
      Object.keys(entry).sort().join(',') !== 'keyId,publicKey' ||
      typeof entry.keyId !== 'string' ||
      !/^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$/.test(entry.keyId) ||
      typeof entry.publicKey !== 'string' ||
      !/^[A-Za-z0-9+/]{43}=$/.test(entry.publicKey) ||
      seen.has(entry.keyId)
    )
      throw invalid();
    const bytes = Buffer.from(entry.publicKey, 'base64');
    if (
      bytes.length !== 32 ||
      bytes.toString('base64') !== entry.publicKey ||
      bytes.every((b) => b === 0) ||
      (bytes[0] === 1 && bytes.subarray(1).every((b) => b === 0)) ||
      fixtureKeys.some((k) => k.publicKey === entry.publicKey || k.keyId === entry.keyId)
    )
      throw invalid();
    seen.add(entry.keyId);
  }
  return JSON.stringify(entries);
}

export const learnBuildInvocation = (args) => ['run', 'tauri', 'build', '--', ...args];

function main() {
  const args = process.argv.slice(2);
  const index = args.indexOf('--keys');
  if (index < 0 || !args[index + 1] || args[index + 1].startsWith('--'))
    throw new Error(
      '用法：node scripts/build-learn-client.mjs --keys <正式公开公钥.json> [--check] [-- <Tauri 构建参数>]'
    );
  const filename = args[index + 1];
  args.splice(index, 2);
  let raw;
  try {
    raw = readFileSync(filename, 'utf8');
  } catch {
    throw new Error('无法读取课程公开公钥文件，请核对文件位置。');
  }
  const keys = validateLearnBuildKeys(raw);
  if (args[0] === '--check' && args.length === 1) {
    console.log(
      `课程公开公钥格式检查通过：${JSON.parse(keys).length} 个；发布方身份须独立确认，Rust 在读取课程时仍执行完整验签。`
    );
    return;
  }
  if (args[0] === '--') args.shift();
  const npmCli = process.env.npm_execpath;
  if (process.platform === 'win32' && !npmCli)
    throw new Error('Windows 请使用 npm run build:learn-client -- --keys <公开公钥.json> 启动。');
  const result = spawnSync(
    npmCli ? process.execPath : 'npm',
    npmCli ? [npmCli, ...learnBuildInvocation(args)] : learnBuildInvocation(args),
    {
      cwd: fileURLToPath(new URL('..', import.meta.url)),
      env: { ...process.env, LYNX_LEARN_TRUSTED_KEYS_JSON: keys },
      stdio: 'inherit',
      shell: false,
    }
  );
  if (result.error) throw new Error('无法启动 Tauri 构建，请检查 Node/npm 环境。');
  process.exitCode = result.status ?? 1;
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main();
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
