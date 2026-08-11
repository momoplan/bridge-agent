import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const tauriDirectory = resolve(repositoryRoot, "src-tauri");
const windowsConfig = JSON.parse(
  readFileSync(resolve(tauriDirectory, "tauri.windows.conf.json"), "utf8")
);

function expectBitmap(relativePath, width, height) {
  expect(relativePath).toBeTruthy();
  const bitmap = readFileSync(resolve(tauriDirectory, relativePath));
  expect(bitmap.subarray(0, 2).toString("ascii")).toBe("BM");
  expect(bitmap.readInt32LE(18)).toBe(width);
  expect(Math.abs(bitmap.readInt32LE(22))).toBe(height);
  expect(bitmap.readUInt16LE(28)).toBe(24);
}

describe("Windows installer branding", () => {
  it("uses release-ready WiX banner and dialog bitmaps", () => {
    const wix = windowsConfig.bundle.windows.wix;
    expectBitmap(wix.bannerPath, 493, 58);
    expectBitmap(wix.dialogImagePath, 493, 312);
  });
});
