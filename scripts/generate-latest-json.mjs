import { readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const [assetsDir, version, tag] = process.argv.slice(2);

if (!assetsDir || !version || !tag) {
  console.error('Usage: generate-latest-json.mjs <assets-dir> <version> <tag>');
  process.exit(1);
}

const repo = process.env.GITHUB_REPOSITORY ?? 'BADtochka/yet-another-player';
const files = readdirSync(assetsDir);

function platform(suffix) {
  const installers = files.filter((file) => file.endsWith(suffix));
  if (installers.length !== 1) {
    throw new Error(`Expected exactly one ${suffix} installer in ${assetsDir}, found ${installers.length}`);
  }
  const [installer] = installers;
  const signature = readFileSync(join(assetsDir, `${installer}.sig`), 'utf8').trim();
  if (!signature) {
    throw new Error(`Empty updater signature for ${installer}`);
  }
  return {
    signature,
    url: `https://github.com/${repo}/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(installer)}`,
  };
}

const macos = platform('.app.tar.gz');
const manifest = {
  version,
  pub_date: new Date().toISOString(),
  platforms: {
    'windows-x86_64': platform('-setup.exe'),
    'linux-x86_64-appimage': platform('_amd64.AppImage'),
    'linux-x86_64-deb': platform('_amd64.deb'),
    'linux-x86_64-pacman': platform('-x86_64.pkg.tar.zst'),
    'darwin-aarch64': macos,
    'darwin-x86_64': macos,
  },
};

writeFileSync(join(assetsDir, 'latest.json'), `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`Generated latest.json for ${tag} (${version})`);
