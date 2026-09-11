import { appendFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { platformsForAssets, releaseAssets } from "./release-platforms.mjs";

const platforms = platformsForAssets("release-assets", process.env.RELEASE_PLATFORMS, process.env.REPAIR_ASSETS_ONLY === "true");
const api = process.argv[2];
const tag = process.env.RELEASE_TAG;
const version = tag.replace(/^bridge-agent-v/, "");
const assets = releaseAssets("release-assets", platforms, version);
const run = (script, args) => execFileSync(process.execPath, [`.github/scripts/${script}`, ...args], { stdio: "inherit" });

run("register-release-manifest.mjs", [api, tag, ...assets.map((asset) => `${asset.target}::${asset.file}::${asset.signatureRequired}`)]);
for (const asset of assets) {
  run("upload-oss-release-asset.mjs", [api, tag, version, asset.target, asset.file, ...(asset.signatureRequired ? [`${asset.file}.sig`] : [])]);
}

if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `platforms=${platforms.map(({ id }) => id).join(",")}\n`);
