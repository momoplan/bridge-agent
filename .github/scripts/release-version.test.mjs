import { expect, test } from "vitest";
import { compareReleaseVersions, parseReleaseVersion } from "./release-version.mjs";

test.each(["0.0.0", "1.2.3", "1.2.3-rc.1", "1.2.3+build.01", "1.2.3-rc.1+build.2"])("accepts SemVer %s", (value) => {
  expect(() => parseReleaseVersion(value)).not.toThrow();
});
test.each(["01.2.3", "1.02.3", "1.2.03", "1.2.3.4", "1.2.3-01", "1.2.3-rc..1", "1.2.3+", "1.2"])("rejects invalid release version %s", (value) => {
  expect(() => parseReleaseVersion(value)).toThrow();
});
test("uses SemVer precedence, ignoring build metadata and comparing numeric prereleases numerically", () => {
  const versions = ["1.0.0-alpha", "1.0.0-alpha.1", "1.0.0-alpha.beta", "1.0.0-beta", "1.0.0-beta.2", "1.0.0-beta.11", "1.0.0-rc.1", "1.0.0", "1.0.1", "1.1.0", "2.0.0"];
  for (let i = 1; i < versions.length; i++) {
    expect(compareReleaseVersions(versions[i], versions[i - 1])).toBe(1);
    expect(compareReleaseVersions(versions[i - 1], versions[i])).toBe(-1);
  }
  expect(compareReleaseVersions("1.2.3+one", "1.2.3+two")).toBe(0);
});
