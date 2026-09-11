import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const catalog = JSON.parse(readFileSync(new URL("../release-platforms.json", import.meta.url), "utf8"));

export function resolveReleasePlan(selection, version, platforms = catalog) {
  if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/.test(version)) {
    throw new Error("Invalid release version");
  }
  const keys = selection?.trim() ? selection.split(",").map((key) => key.trim()) : Object.keys(platforms);
  if (!keys.length || new Set(keys).size !== keys.length || keys.some((key) => !Object.hasOwn(platforms, key))) {
    throw new Error("Release platforms must be distinct catalog keys");
  }
  const selected = keys.map((key) => platforms[key]);
  const assets = selected.flatMap((platform) => platform.assets.map((asset) => ({
    target: platform.name,
    name: `Baijimu_${version}_${asset.suffix}`,
    signatureRequired: asset.signatureRequired,
    updaters: asset.updaters ?? [],
  })));
  return {
    matrix: { include: selected.map(({ name, runner, tauri_args, rust_targets }) => ({ name, runner, tauri_args, rust_targets })) },
    qualityRunner: selected[0].qualityRunner,
    assets,
    updaters: assets.flatMap((asset) => asset.updaters.map((updater) => ({ ...updater, name: asset.name }))),
  };
}

export function validateReleaseFiles(plan, directory) {
  const expected = new Set();
  for (const asset of plan.assets) {
    expected.add(asset.name);
    expected.add(`${asset.name}.sig`);
    const paths = asset.signatureRequired ? [asset.name, `${asset.name}.sig`] : [asset.name];
    for (const name of paths) {
      const path = join(directory, name);
      if (!statSync(path, { throwIfNoEntry: false })?.isFile() || statSync(path).size === 0) {
        throw new Error(`Missing release bundle or required signature: ${name}`);
      }
    }
  }
  for (const name of readdirSync(directory)) {
    if (!expected.has(name)) throw new Error(`Unexpected release asset outside selected platforms: ${name}`);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const version = process.env.RELEASE_TAG?.replace(/^bridge-agent-v/, "");
  const plan = resolveReleasePlan(process.env.RELEASE_PLATFORMS, version);
  if (process.argv[2]) validateReleaseFiles(plan, process.argv[2]);
  console.log(JSON.stringify(plan));
}
