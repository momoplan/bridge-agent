import { pathToFileURL } from "node:url";
import { readFileSync } from "node:fs";
import { releasePlatforms } from "./release-platforms.mjs";
import { compareReleaseVersions } from "./release-version.mjs";
const compatibility = JSON.parse(readFileSync(new URL("../../config/client-compatibility.json", import.meta.url), "utf8"));

export function assertOsSelection(response) {
  if (response.headers.get("x-bridge-release-os-selection") !== "os-v1") {
    throw new Error("Release service must enforce operating-system compatibility before publishing a higher macOS baseline");
  }
}

async function get(url) {
  const response = await fetch(url, { signal: AbortSignal.timeout(30000) });
  if (!response.ok) throw new Error(`Release verification returned HTTP ${response.status}: ${url}`);
  return response;
}

export function assertPlatformSelection(response) {
  if (response.headers.get("x-bridge-release-selection") !== "platform-v1") {
    throw new Error("Release service must support platform-specific latest selection before publishing selected platforms");
  }
}

export async function verifyPlatformRelease(base, version, platforms, fetchResponse = get, fetchBinary = fetch, repair = false) {
  for (const platform of platforms) {
    for (const probe of platform.updaters) {
      const url = new URL(base);
      url.searchParams.set("platform", platform.id);
      url.searchParams.set("arch", probe.arch);
      url.searchParams.set("currentVersion", "0.0.0");
      if (platform.id === "macos") url.searchParams.set("osVersion", compatibility.macos[probe.arch]);
      const response = await fetchResponse(url);
      assertPlatformSelection(response);
      if (platform.id === "macos") assertOsSelection(response);
      const metadata = await response.json();
      if ((repair ? compareReleaseVersions(metadata.version, version) < 0 : metadata.version !== version) || metadata.assets.length !== platform.assets.length) throw new Error(`Incorrect ${platform.id} release metadata`);
      for (const format of platform.assets) {
        const assets = metadata.assets.filter((asset) => asset.target === platform.name && asset.name.endsWith(format.suffix));
        if (assets.length !== 1 || !/^[a-f0-9]{64}$/i.test(assets[0].sha256)) throw new Error(`Invalid ${platform.id} ${format.suffix} metadata`);
        if (platform.id === "macos" && !repair && Object.entries(compatibility.macos).some(([arch, minimum]) => assets[0].minimumSystemVersions?.[arch] !== minimum)) {
          throw new Error("Published macOS assets must retain the exact per-architecture system requirements");
        }
        const download = new URL(assets[0].downloadUrl);
        if (download.protocol !== "https:") throw new Error("Release download must use HTTPS");
      }
      const updater = new URL(`${base}/tauri`);
      updater.search = new URLSearchParams({ target: probe.target, arch: probe.arch, currentVersion: "0.0.0" }).toString();
      if (platform.id === "macos") updater.searchParams.set("osVersion", compatibility.macos[probe.arch]);
      const updaterResponse = await fetchResponse(updater);
      if (platform.id === "macos") assertOsSelection(updaterResponse);
      const update = await updaterResponse.json();
      if (update.version !== metadata.version || !update.signature?.trim() || !metadata.assets.some((asset) => asset.downloadUrl === update.url && asset.signature === update.signature)) throw new Error(`Incorrect ${platform.id} updater metadata`);
      const binary = await fetchBinary(update.url, { headers: { Range: "bytes=0-0" }, signal: AbortSignal.timeout(30000) });
      try { if (!binary.ok) throw new Error(`Public download returned HTTP ${binary.status}`); }
      finally { await binary.body?.cancel(); }
    }
  }
}

export async function verifyMacOsExclusions(base, version, fetchResponse = get) {
  for (const arch of Object.keys(compatibility.macos)) {
    const minimum = compatibility.macos[arch].split(".").map(Number);
    const index = minimum.findLastIndex((part) => part > 0);
    minimum[index] -= 1;
    const olderParts = minimum.slice(0, index + 1);
    while (olderParts.length < 3) olderParts.push(0);
    const olderSystem = olderParts.join(".");
    for (const osVersion of [undefined, olderSystem]) {
      for (const tauri of [false, true]) {
        const url = new URL(tauri ? `${base}/tauri` : base);
        url.search = new URLSearchParams({ [tauri ? "target" : "platform"]: tauri ? "darwin" : "macos", arch, currentVersion: "0.0.0" }).toString();
        if (osVersion) url.searchParams.set("osVersion", osVersion);
        const response = await fetchResponse(url);
        if (response.status === 204) continue;
        const metadata = await response.json();
        if (compareReleaseVersions(metadata.version, version) >= 0) {
          throw new Error(`Incompatible macOS update selected for ${arch}/${osVersion ?? "unreported OS"}`);
        }
      }
    }
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [mode, base] = process.argv.slice(2);
  if (mode === "preflight") {
    const url = new URL(base);
    url.searchParams.set("platform", releasePlatforms(process.env.RELEASE_PLATFORMS)[0].id);
    assertPlatformSelection(await get(url));
    if (releasePlatforms(process.env.RELEASE_PLATFORMS).some((platform) => platform.id === "macos")) {
      url.searchParams.set("platform", "macos");
      url.searchParams.set("arch", "x86_64");
      url.searchParams.set("osVersion", compatibility.macos.x86_64);
      assertOsSelection(await get(url));
    }
  } else if (mode === "verify") {
    const platforms = releasePlatforms(process.env.RELEASE_PLATFORMS);
    const version = process.env.RELEASE_TAG.replace(/^bridge-agent-v/, "");
    await verifyPlatformRelease(base, version, platforms, get, fetch, process.env.REPAIR_ASSETS_ONLY === "true");
    if (platforms.some((platform) => platform.id === "macos") && process.env.REPAIR_ASSETS_ONLY !== "true") await verifyMacOsExclusions(base, version);
  } else throw new Error(`Unknown release verification mode: ${mode}`);
}
