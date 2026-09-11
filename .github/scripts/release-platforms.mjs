import { appendFileSync, readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

export const catalogPath = fileURLToPath(new URL("../release-platforms.json", import.meta.url));

export function releasePlatforms(input = "", catalog = JSON.parse(readFileSync(catalogPath, "utf8"))) {
  const requested = input.trim() ? input.split(",").map((id) => id.trim()) : catalog.defaultPlatforms;
  if (!requested.length || new Set(requested).size !== requested.length) {
    throw new Error("Select one or more distinct release platforms");
  }
  return requested.map((id) => {
    const platform = catalog.platforms.find((entry) => entry.id === id);
    if (!platform) throw new Error(`Unknown release platform: ${id}`);
    return platform;
  });
}

export function releaseAssets(directory, platforms, version) {
  const files = readdirSync(directory).filter((name) => statSync(join(directory, name)).isFile());
  const assets = platforms.flatMap((platform) => platform.assets.map((format) => {
    const matches = files.filter((name) => version ? name === `Baijimu_${version}_${format.filenameSuffix}` : name.endsWith(format.suffix));
    if (matches.length !== 1) throw new Error(`Expected exactly one ${platform.id} ${format.suffix} asset, found ${matches.length}`);
    const name = matches[0];
    const file = join(directory, name);
    if (!statSync(file).size) throw new Error(`Empty release asset: ${name}`);
    if (format.signatureRequired && (!files.includes(`${name}.sig`) || !readFileSync(`${file}.sig`, "utf8").trim())) {
      throw new Error(`Missing updater signature: ${name}`);
    }
    return { target: platform.name, name, file, signatureRequired: format.signatureRequired };
  }));
  const expected = new Set(assets.flatMap((asset) => [asset.name, ...(asset.signatureRequired ? [`${asset.name}.sig`] : [])]));
  const unexpected = files.filter((name) => !expected.has(name));
  if (unexpected.length) throw new Error(`Release contains unselected or unexpected assets: ${unexpected.join(", ")}`);
  return assets;
}

export function platformsForAssets(directory, input = "", repair = false) {
  if (!repair || input.trim()) return releasePlatforms(input);
  // Repair preserves the original platform set, including historical all-platform releases.
  const catalog = JSON.parse(readFileSync(catalogPath, "utf8"));
  const files = readdirSync(directory);
  return releasePlatforms(catalog.platforms.filter((platform) =>
    platform.assets.some(({ suffix }) => files.some((name) => name.endsWith(suffix))),
  ).map(({ id }) => id).join(","), { ...catalog, defaultPlatforms: [] });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const platforms = releasePlatforms(process.env.RELEASE_PLATFORMS);
  const matrix = { include: platforms.map(({ id, name, runner, tauri_args, rust_targets }) => ({ id, name, runner, tauri_args, rust_targets })) };
  if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `matrix=${JSON.stringify(matrix)}\nquality_runner=${platforms[0].qualityRunner}\nplatforms=${platforms.map(({ id }) => id).join(",")}\n`);
  console.log(JSON.stringify(matrix));
}
