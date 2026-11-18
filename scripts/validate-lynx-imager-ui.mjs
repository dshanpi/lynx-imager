import { readFile } from 'node:fs/promises';
import path from 'node:path';

const root = path.resolve(import.meta.dirname, '..');
const files = {
  main: path.join(root, 'src/main.tsx'),
  page: path.join(root, 'src/Components/Imager/ImagerPage.tsx'),
  shell: path.join(root, 'src/Components/Imager/ImagerShell.css'),
  skill: path.join(root, 'skills/lynx-imager-ui/SKILL.md'),
  reference: path.join(root, 'skills/lynx-imager-ui/references/design-contract.md'),
};

const source = Object.fromEntries(
  await Promise.all(Object.entries(files).map(async ([name, file]) => [name, await readFile(file, 'utf8')]))
);
const failures = [];

function includes(file, value, reason) {
  if (!source[file].includes(value)) failures.push(`${file}: ${reason}`);
}

function excludes(file, value, reason) {
  if (source[file].includes(value)) failures.push(`${file}: ${reason}`);
}

for (const label of ['基础烧录', '高级烧录', '连接 AI']) {
  includes('main', label, `missing workspace ${label}`);
}
for (const label of ['选择板子', '选择系统', '配置接口', '烧写系统']) {
  includes('page', label, `missing basic step ${label}`);
}
for (const marker of ['imager-board-search', 'imager-board-filters', 'imager-board-picker', 'imager-board-detail']) {
  includes('page', marker, `missing board-selection interaction ${marker}`);
}
for (const vendor of ["chipVendor: 'allwinner'", "chipVendor: 'rockchip'", "chipVendor: 'nxp'", "chipVendor: 'st'", "chipVendor: 'kendryte'"]) {
  includes('page', vendor, `missing supported vendor route ${vendor}`);
}
for (const marker of ['imager-app-header', 'imager-app-tabs', 'imager-app-main', 'imager-app-status']) {
  includes('shell', marker, `missing shell contract ${marker}`);
}
for (const marker of ['name: lynx-imager-ui', 'Product boundary', 'Control and state rules', 'Backend boundary']) {
  includes('skill', marker, `missing skill rule ${marker}`);
}
for (const marker of ['Shared geometry', 'Action hierarchy', 'Workflow state matrix']) {
  includes('reference', marker, `missing design contract ${marker}`);
}
for (const forbidden of ['LearningCenterPage', '学习中心', 'import Layout from']) {
  excludes('main', forbidden, `legacy product shell marker remains: ${forbidden}`);
}

if (failures.length) {
  console.error('LYNX Imager UI constraint gate failed:');
  for (const failure of failures) console.error(`- ${failure}`);
  process.exit(1);
}

console.log('LYNX Imager UI constraint gate passed: shell, four-step flow, vendor routes, controls, and skill contract are present.');
