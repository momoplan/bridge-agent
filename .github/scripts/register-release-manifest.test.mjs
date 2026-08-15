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
