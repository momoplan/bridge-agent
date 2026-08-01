#!/usr/bin/env node

import { createHash } from 'node:crypto';
import { mkdir, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

function option(name) {
  const index = process.argv.indexOf(name);
  if (index === -1 || !process.argv[index + 1]) {
    throw new Error(`missing ${name}`);
  }
  return process.argv[index + 1];
}

const tag = option('--tag');
const expectedSkillSha256 = option('--skill-sha256').toLowerCase();
const archiveSha256 = option('--archive-sha256').toLowerCase();
for (const [name, value] of [
  ['skill SHA-256', expectedSkillSha256],
  ['archive SHA-256', archiveSha256],
]) {
  if (!/^[0-9a-f]{64}$/.test(value)) {
    throw new Error(`invalid ${name}`);
  }
}
if (!/^v\d+\.\d+\.\d+$/.test(tag)) {
  throw new Error('tag must be an immutable vMAJOR.MINOR.PATCH release');
}

const repository = 'https://github.com/momoplan/baijimu-platform-skill';
const sourceUrl = `https://raw.githubusercontent.com/momoplan/baijimu-platform-skill/${tag}/SKILL.md`;
const archiveUrl = `${repository}/releases/download/${tag}/baijimu-platform.zip`;
const archiveResponse = await fetch(archiveUrl);
if (!archiveResponse.ok) {
  throw new Error(`failed to download ${archiveUrl}: HTTP ${archiveResponse.status}`);
}
const archive = Buffer.from(await archiveResponse.arrayBuffer());
const actualArchiveSha256 = createHash('sha256').update(archive).digest('hex');
if (actualArchiveSha256 !== archiveSha256) {
  throw new Error(
    `archive SHA-256 mismatch: expected ${archiveSha256}, got ${actualArchiveSha256}`,
  );
}

const response = await fetch(sourceUrl, { redirect: 'error' });
if (!response.ok) {
  throw new Error(`failed to download ${sourceUrl}: HTTP ${response.status}`);
}
const contents = Buffer.from(await response.arrayBuffer());
const actualSkillSha256 = createHash('sha256').update(contents).digest('hex');
if (actualSkillSha256 !== expectedSkillSha256) {
  throw new Error(
    `skill SHA-256 mismatch: expected ${expectedSkillSha256}, got ${actualSkillSha256}`,
  );
}

const scriptDir = dirname(fileURLToPath(import.meta.url));
const targetDir = resolve(scriptDir, '../src-tauri/resources/skills/baijimu-platform');
await mkdir(targetDir, { recursive: true });
await writeFile(resolve(targetDir, 'SKILL.md'), contents);
await writeFile(
  resolve(targetDir, 'PROVENANCE.json'),
  `${JSON.stringify(
    {
      repository,
      release: tag,
      skillSha256: actualSkillSha256,
      archiveSha256,
    },
    null,
    2,
  )}\n`,
);

console.log(
  `synced baijimu-platform ${tag} (skill ${actualSkillSha256}, archive ${actualArchiveSha256})`,
);
