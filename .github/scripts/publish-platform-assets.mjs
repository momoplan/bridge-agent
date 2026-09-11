import { appendFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { pathToFileURL } from "node:url";
import { platformsForAssets, releaseAssets } from "./release-platforms.mjs";
import { parseReleaseVersion } from "./release-version.mjs";

const runScript = (script, args) => execFileSync(process.execPath, [`.github/scripts/${script}`, ...args], { stdio: "inherit" });

export function publishPlatformAssets({ api, tag, selection = "", repair = false, directory = "release-assets" }, run = runScript) {
  const platforms = platformsForAssets(directory, selection, repair);
  const version = tag.replace(/^bridge-agent-v/, "");
  parseReleaseVersion(version);
  const assets = releaseAssets(directory, platforms, version);
  run("register-release-manifest.mjs", [api, tag, ...assets.map((asset) => `${asset.target}::${asset.file}::${asset.signatureRequired}`)]);
  for (const asset of assets) {
    run("upload-oss-release-asset.mjs", [api, tag, version, asset.target, asset.file, ...(asset.signatureRequired ? [`${asset.file}.sig`] : [])]);
  }
  return platforms.map(({ id }) => id).join(",");
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const platforms = publishPlatformAssets({ api: process.argv[2], tag: process.env.RELEASE_TAG,
    selection: process.env.RELEASE_PLATFORMS, repair: process.env.REPAIR_ASSETS_ONLY === "true" });
  if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `platforms=${platforms}\n`);
}
