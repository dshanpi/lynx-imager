import { createHash } from 'node:crypto';
import { lstatSync, readFileSync, readdirSync, realpathSync, writeFileSync } from 'node:fs';
import { dirname, extname, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import YAML from 'yaml';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const root = resolve(scriptDir, '..', 'src-tauri', 'resources', 'skills');
const indexPath = join(root, 'index.json');
const allowedExtensions = new Set(['.md', '.txt', '.json', '.yaml', '.yml']);
const allowedTools = {
  linux: new Set([
    'lynx_workbench_status',
    'lynx_flash_capabilities',
    'lynx_allwinner_enter_fel',
    'lynx_scan_flash_devices',
    'lynx_start_flash',
    'lynx_workbench_task_status',
    'lynx_serial_open',
    'lynx_serial_read',
    'lynx_serial_close',
  ]),
  mcu: new Set([
    'lynx_mcu_capabilities',
    'lynx_mcu_board_info',
    'lynx_mcu_uart_open',
    'lynx_mcu_uart_read',
    'lynx_mcu_uart_close',
  ]),
};

const digest = (content) => createHash('sha256').update(content).digest('hex');
const posix = (path) => path.split(sep).join('/');

function filesBelow(directory) {
  const files = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    const status = lstatSync(path);
    if (status.isSymbolicLink()) throw new Error(`Skill symlink is prohibited: ${path}`);
    if (entry.isDirectory()) files.push(...filesBelow(path));
    else if (entry.isFile()) files.push(path);
  }
  return files;
}

function frontmatter(content, path) {
  if (!content.startsWith('---\n')) throw new Error(`Missing frontmatter: ${path}`);
  const end = content.indexOf('\n---\n', 4);
  if (end < 0) throw new Error(`Unclosed frontmatter: ${path}`);
  const value = YAML.parse(content.slice(4, end));
  if (!value || typeof value !== 'object') throw new Error(`Invalid frontmatter: ${path}`);
  return value;
}

function validateName(name, path) {
  if (typeof name !== 'string' || !/^[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?$/.test(name)) {
    throw new Error(`Invalid skill name in ${path}`);
  }
}

const entries = filesBelow(root)
  .filter((path) => path.endsWith(`${sep}SKILL.md`))
  .sort();
const names = new Set();
const skills = entries.map((entry) => {
  const directory = dirname(entry);
  const content = readFileSync(entry, 'utf8');
  const metadata = frontmatter(content, entry);
  validateName(metadata.name, entry);
  if (names.has(metadata.name)) throw new Error(`Duplicate skill name: ${metadata.name}`);
  names.add(metadata.name);
  if (!['linux', 'mcu'].includes(metadata.domain))
    throw new Error(`Invalid domain: ${metadata.name}`);
  if (!Array.isArray(metadata.systems) || metadata.systems.length === 0) {
    throw new Error(`Missing systems: ${metadata.name}`);
  }
  if (typeof metadata.category !== 'string' || !metadata.category) {
    throw new Error(`Missing category: ${metadata.name}`);
  }
  const requiredTools = metadata['required-tools'];
  if (!Array.isArray(requiredTools)) throw new Error(`Missing required-tools: ${metadata.name}`);
  for (const tool of requiredTools) {
    if (!allowedTools[metadata.domain].has(tool)) {
      throw new Error(`Cross-domain or unknown required tool ${tool}: ${metadata.name}`);
    }
  }
  const files = {};
  for (const path of filesBelow(directory).sort()) {
    const canonical = realpathSync(path);
    if (
      !canonical.startsWith(`${realpathSync(directory)}${sep}`) &&
      canonical !== realpathSync(directory)
    ) {
      throw new Error(`Skill path escapes its root: ${path}`);
    }
    if (!allowedExtensions.has(extname(path).toLowerCase())) {
      throw new Error(`Skill binary or executable is prohibited: ${path}`);
    }
    const relativePath = posix(relative(directory, path));
    if (!relativePath || relativePath.startsWith('../'))
      throw new Error(`Invalid skill path: ${path}`);
    files[relativePath] = digest(readFileSync(path));
  }
  return {
    name: metadata.name,
    version: metadata.version,
    domain: metadata.domain,
    systems: metadata.systems,
    category: metadata.category,
    chipVendors: metadata['chip-vendors'] || [],
    requiredTools,
    description: metadata.description,
    entryFile: 'SKILL.md',
    sha256: files['SKILL.md'],
    files,
  };
});

const generated = `${JSON.stringify({ schemaVersion: 1, skills }, null, 2)}\n`;
if (process.argv.includes('--write')) {
  writeFileSync(indexPath, generated);
  console.log(`Wrote ${skills.length} skills to ${indexPath}`);
} else {
  const current = readFileSync(indexPath, 'utf8');
  if (current !== generated)
    throw new Error('Skills index is stale; run validate-skills-index.mjs --write');
  console.log(
    `Skills index validated: ${skills.length} built-in skills, required-tools and sha256 agree.`
  );
}
