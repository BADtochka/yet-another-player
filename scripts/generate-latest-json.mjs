import { readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const [assetsDir, version, tag] = process.argv.slice(2);

if (!assetsDir || !version || !tag) {
  console.error('Usage: generate-latest-json.mjs <assets-dir> <version> <tag>');
  process.exit(1);
}

const repo = process.env.GITHUB_REPOSITORY ?? 'BADtochka/yet-another-player';
const files = readdirSync(assetsDir);
const installer = files.find((file) => file.endsWith('-setup.exe'));
const signatureFile = installer ? `${installer}.sig` : files.find((file) => file.endsWith('-setup.exe.sig'));

if (!installer || !signatureFile || !files.includes(signatureFile)) {
  console.error('Could not find NSIS installer and signature in', assetsDir, files);
  process.exit(1);
}

const signature = readFileSync(join(assetsDir, signatureFile), 'utf8').trim();
const url = `https://github.com/${repo}/releases/download/${tag}/${installer}`;

const manifest = {
  version,
  pub_date: new Date().toISOString(),
  platforms: {
    'windows-x86_64': {
      signature,
      url,
    },
  },
};

writeFileSync(join(assetsDir, 'latest.json'), `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`Generated latest.json for ${tag} (${version})`);
