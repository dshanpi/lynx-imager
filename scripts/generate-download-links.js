import { readFileSync, writeFileSync } from 'fs';
import { fileURLToPath } from 'url';
import { dirname, join } from 'path';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

function generateDownloadLinks(version) {
  const releaseBase = `https://github.com/dshanpi/lynx-releases/releases/download/v${version}`;
  const lines = [
    '### Downloads',
    '',
    '#### Windows x64',
    '',
    '| Type | Download |',
    '|------|----------|',
    `| Installer | [LYNX_${version}_x64-setup.exe](${releaseBase}/LYNX_${version}_x64-setup.exe) |`,
    `| MSI | [LYNX_${version}_x64_en-US.msi](${releaseBase}/LYNX_${version}_x64_en-US.msi) |`,
    '',
    '**Dependencies:** https://developer.microsoft.com/zh-cn/microsoft-edge/webview2/?form=MA13LH#download',
    '',
    '#### Linux x64',
    '',
    '| Type | Download |',
    '|------|----------|',
    `| AppImage | [LYNX_${version}_amd64.AppImage](${releaseBase}/LYNX_${version}_amd64.AppImage) |`,
    `| DEB | [LYNX_${version}_amd64.deb](${releaseBase}/LYNX_${version}_amd64.deb) |`,
    `| RPM | [LYNX-${version}-1.x86_64.rpm](${releaseBase}/LYNX-${version}-1.x86_64.rpm) |`,
    '',
    '**Dependencies:** libusb-1.0-0-dev (Debian/Ubuntu: sudo apt-get install libusb-1.0-0-dev)',
    '',
    '#### macOS (Apple Silicon)',
    '',
    '| Type | Download |',
    '|------|----------|',
    `| DMG | [LYNX_${version}_aarch64.dmg](${releaseBase}/LYNX_${version}_aarch64.dmg) |`,
    '',
    '**Dependencies:** libusb (Homebrew: brew install libusb)',
    'macOS needs to disable Signing check for the app to run. by xattr -cr LYNX_aarch64.app',
  ];

  return lines.join('\n');
}

const version = process.argv[2];

if (!version) {
  console.error('Usage: node generate-download-links.js <version>');
  console.error('Example: node generate-download-links.js 1.0.0');
  process.exit(1);
}

const links = generateDownloadLinks(version);
console.log(links);

if (process.argv.includes('--output-file')) {
  const outputPath = join(__dirname, 'download-links.txt');
  writeFileSync(outputPath, links, 'utf-8');
  console.log(`\nDownload links saved to: ${outputPath}`);
}
