import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, test } from "vitest";
import { platformsForAssets, releaseAssets, releasePlatforms } from "./release-platforms.mjs";
import { assertPlatformSelection, verifyPlatformRelease } from "./verify-platform-release.mjs";

const directories = [];
afterEach(() => directories.splice(0).forEach((directory) => rmSync(directory, { recursive: true, force: true })));
function filesFor(platforms) {
  const directory = mkdtempSync(join(tmpdir(), "bridge-release-platforms-"));
  directories.push(directory);
  for (const platform of platforms) for (const format of platform.assets) {
    const file = join(directory, `client_${platform.id}${format.suffix}`);
    writeFileSync(file, "binary");
    if (format.signatureRequired) writeFileSync(`${file}.sig`, "signature");
  }
  return directory;
}

describe("platform-specific publication", () => {
  test("default release contains only macOS and no Windows signer runner", () => {
    const platforms = releasePlatforms();
    expect(platforms.map(({ id }) => id)).toEqual(["macos"]);
    expect(platforms.every(({ runner }) => runner.startsWith("macos-"))).toBe(true);
    expect(releaseAssets(filesFor(platforms), platforms)).toHaveLength(2);
  });
  test.each(["windows", "linux", "macos,windows", "macos,linux", "windows,linux", "macos,windows,linux"])("publishes exactly the chosen formats for %s", (input) => {
    const platforms = releasePlatforms(input);
    const assets = releaseAssets(filesFor(platforms), platforms);
    expect(assets.map(({ target }) => target)).toEqual(platforms.flatMap((platform) => platform.assets.map(() => platform.name)));
  });
  test.each(["windows,windows", "macos,", "unknown", ",", "MACOS"])("rejects invalid selection before a release starts: %s", (input) => {
    expect(() => releasePlatforms(input)).toThrow();
  });
  test("a catalog entry can add a target without a new selection branch", () => {
    const target = { id: "new-target", name: "New target" };
    expect(releasePlatforms("new-target", { platforms: [target], defaultPlatforms: [] })).toEqual([target]);
  });
  test("missing or unexpected artifacts fail before manifest registration", () => {
    const platforms = releasePlatforms("macos");
    const directory = filesFor(platforms);
    writeFileSync(join(directory, "unselected.msi"), "binary");
    expect(() => releaseAssets(directory, platforms)).toThrow(/unexpected/);
    rmSync(join(directory, "unselected.msi"));
    rmSync(join(directory, "client_macos.app.tar.gz.sig"));
    expect(() => releaseAssets(directory, platforms)).toThrow(/signature/);
  });
  test("repair discovers the complete original set instead of adopting the new macOS default", () => {
    const directory = filesFor(releasePlatforms("macos,windows,linux"));
    const original = platformsForAssets(directory, "", true);
    expect(original.map(({ id }) => id)).toEqual(["macos", "windows", "linux"]);
    expect(releaseAssets(directory, original)).toHaveLength(5);
    expect(() => releaseAssets(directory, platformsForAssets(directory, "macos", true))).toThrow(/unexpected/);
  });
  test("partial publication requires a platform-aware backend", () => {
    expect(() => assertPlatformSelection(new Response("{}"))).toThrow(/platform-specific/);
    expect(() => assertPlatformSelection(new Response("{}", { headers: { "x-bridge-release-selection": "platform-v1" } }))).not.toThrow();
  });
  test("macOS verification uses both architectures and never requires another platform's version", async () => {
    const platforms = releasePlatforms();
    const assets = releaseAssets(filesFor(platforms), platforms).map((asset) => ({ ...asset, sha256: "a".repeat(64), downloadUrl: `https://cdn.example.test/${asset.name}`, signature: asset.signatureRequired ? "signature" : undefined }));
    const requests = [];
    const fakeFetch = async (url) => {
      requests.push(url.toString());
      const payload = url.pathname.endsWith("/tauri")
        ? { version: "1.2.3", url: assets[0].downloadUrl, signature: "signature" }
        : { version: "1.2.3", assets };
      return new Response(JSON.stringify(payload), { headers: { "x-bridge-release-selection": "platform-v1" } });
    };
    await verifyPlatformRelease("https://update.example.test/latest", "1.2.3", platforms, fakeFetch, async () => new Response("b", { status: 206 }));
    expect(requests).toHaveLength(4);
    expect(requests.some((url) => url.includes("arch=aarch64"))).toBe(true);
    expect(requests.some((url) => url.includes("arch=x86_64"))).toBe(true);
    expect(requests.every((url) => !url.includes("windows") && !url.includes("linux"))).toBe(true);
    const binary = async () => new Response("b", { status: 206 });
    await expect(verifyPlatformRelease("https://update.example.test/latest", "1.2.2", platforms, fakeFetch, binary, true)).resolves.toBeUndefined();
    await expect(verifyPlatformRelease("https://update.example.test/latest", "1.2.2", platforms, fakeFetch, binary)).rejects.toThrow(/Incorrect macos/);
    await expect(verifyPlatformRelease("https://update.example.test/latest", "1.2.4", platforms, fakeFetch, binary, true)).rejects.toThrow(/Incorrect macos/);
  });
});
