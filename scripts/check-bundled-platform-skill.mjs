import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";

const repository = "https://github.com/momoplan/baijimu-platform-skill";
const bundledRoot = new URL("../src-tauri/resources/skills/baijimu-platform/", import.meta.url);
const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");

export function validateBundledSkill(contents, provenance) {
  if (provenance.repository !== repository || !/^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(provenance.release)) {
    throw new Error("Bundled platform skill must pin an official stable release");
  }
  for (const field of ["skillSha256", "archiveSha256"]) {
    if (!/^[a-f0-9]{64}$/.test(provenance[field])) throw new Error(`Invalid ${field}`);
  }
  if (digest(contents) !== provenance.skillSha256) throw new Error("Bundled platform skill checksum mismatch");
  const text = contents.toString("utf8");
  if (!text.startsWith("---\nname: baijimu-platform\n") || !text.includes("\ndescription:") || !text.includes("https://docs.baijimu.com/")) {
    throw new Error("Bundled platform skill is missing its official entrypoint metadata");
  }
}

export async function verifyUpstreamSkill(contents, provenance, fetcher = fetch) {
  validateBundledSkill(contents, provenance);
  const releaseBase = `${repository}/releases/download/${provenance.release}`;
  const urls = [
    `https://raw.githubusercontent.com/momoplan/baijimu-platform-skill/${provenance.release}/SKILL.md`,
    `${releaseBase}/baijimu-platform.zip`,
    `${releaseBase}/baijimu-platform.zip.sha256`,
  ];
  const upstream = await Promise.all(urls.map(async (url) => {
    const response = await fetcher(url);
    if (!response.ok) throw new Error(`Failed to verify platform skill upstream: HTTP ${response.status} (${url})`);
    return Buffer.from(await response.arrayBuffer());
  }));
  if (!contents.equals(upstream[0])) throw new Error("Bundled platform skill differs from the pinned upstream source");
  if (digest(upstream[1]) !== provenance.archiveSha256) throw new Error("Platform skill release archive checksum mismatch");
  const checksum = upstream[2].toString("utf8").trim().split(/\s+/);
  if (checksum.length !== 2 || checksum[0] !== provenance.archiveSha256 || checksum[1] !== "baijimu-platform.zip") {
    throw new Error("Platform skill provenance differs from the published release checksum");
  }
}

async function main() {
  const contents = await readFile(new URL("SKILL.md", bundledRoot));
  const provenance = JSON.parse(await readFile(new URL("PROVENANCE.json", bundledRoot), "utf8"));
  validateBundledSkill(contents, provenance);
  if (process.argv.includes("--upstream")) await verifyUpstreamSkill(contents, provenance);
  console.log(`Verified bundled platform skill ${provenance.release}${process.argv.includes("--upstream") ? " against its official release" : " locally"}.`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}
