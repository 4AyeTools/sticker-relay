import { readFileSync, writeFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';

const [releaseDirectory, version, tag, repository, publicationDate] = process.argv.slice(2);

if (!releaseDirectory || !version || !tag || !repository || !publicationDate) {
  throw new Error('Usage: node scripts/generate-updater-manifest.mjs <release-dir> <version> <tag> <repository> <pub-date>');
}

const changelog = readFileSync('CHANGELOG.md', 'utf8');
const escapedVersion = version.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
const headingMatch = new RegExp(`^## ${escapedVersion}[^\\n]*$`, 'm').exec(changelog);
const sectionTail = headingMatch
  ? changelog.slice(headingMatch.index + headingMatch[0].length).replace(/^\\r?\\n/, '')
  : '';
const nextHeadingIndex = sectionTail.search(/^## /m);
const section = nextHeadingIndex >= 0 ? sectionTail.slice(0, nextHeadingIndex) : sectionTail;
const notes = section
  .trim()
  .replace(/^### /gm, '')
  .replace(/^- /gm, '• ')
  || `表情递 ${version} 已发布，包含功能改进和问题修复。`;

const assets = [
  ['windows-x86_64', `sticker-relay-${version}-windows-x64-setup.exe`],
  ['darwin-x86_64', `sticker-relay-${version}-macos-intel.app.tar.gz`],
  ['darwin-aarch64', `sticker-relay-${version}-macos-apple-silicon.app.tar.gz`],
];

const platforms = Object.fromEntries(assets.map(([platform, asset]) => {
  const assetPath = join(releaseDirectory, asset);
  const signaturePath = `${assetPath}.sig`;
  if (!existsSync(assetPath)) throw new Error(`Missing updater asset: ${asset}`);
  if (!existsSync(signaturePath)) throw new Error(`Missing updater signature: ${asset}.sig`);
  return [platform, {
    signature: readFileSync(signaturePath, 'utf8').trim(),
    url: `https://github.com/${repository}/releases/download/${tag}/${asset}`,
  }];
}));

const manifest = {
  version,
  notes,
  pub_date: publicationDate,
  platforms,
};

writeFileSync(join(releaseDirectory, 'latest.json'), `${JSON.stringify(manifest, null, 2)}\n`, 'utf8');
console.log(`Generated latest.json for ${Object.keys(platforms).join(', ')}`);
