import { expect, test } from "vitest";

import { buildManifest } from "./register-release-manifest.mjs";

test("buildManifest keeps release format ownership in the workflow", () => {
  expect(buildManifest([
      "future-platform::release/client.pkg::true",
      "future-platform::release/client-manual.zip::false",
    ])).toEqual({
      assets: [
        { target: "future-platform", name: "client.pkg", signatureRequired: true },
        { target: "future-platform", name: "client-manual.zip", signatureRequired: false },
      ],
    });
});

test("buildManifest rejects duplicate identities", () => {
  expect(() =>
    buildManifest(["target::a/client.pkg::true", "target::b/client.pkg::false"]),
  ).toThrow(/Duplicate manifest asset/);
});

test("macOS manifests carry the versioned per-architecture operating-system requirements", () => {
  const minimumSystemVersions = { x86_64: "10.15.5", aarch64: "11.0" };
  const manifest = buildManifest(["macOS Universal::a/client.dmg::false", "Windows x64::a/client.msi::true"], minimumSystemVersions);
  expect(manifest.assets[0].minimumSystemVersions).toEqual(minimumSystemVersions);
  expect(manifest.assets[1]).not.toHaveProperty("minimumSystemVersions");
});
