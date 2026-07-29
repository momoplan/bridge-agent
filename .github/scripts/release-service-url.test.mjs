import { describe, expect, test } from "vitest";

import {
  normalizeReleaseApiBase,
  normalizeUpdateApiUrl,
} from "./release-service-url.mjs";

describe("release service public URL contract", () => {
  test("accepts only the canonical release API base", () => {
    expect(
      normalizeReleaseApiBase(
        "https://updates.baijimu.com/api/bridge-agent/",
      ),
    ).toBe("https://updates.baijimu.com/api/bridge-agent");
  });

  test.each([
    "http://updates.baijimu.com/api/bridge-agent",
    "https://relay.baijimu.com/api/bridge-agent",
    "https://updates.baijimu.com:443/api/bridge-agent",
    "https://user:pass@updates.baijimu.com/api/bridge-agent",
    "https://updates.baijimu.com/api/bridge-agent/releases",
    "https://updates.baijimu.com/api/bridge-agent?next=relay",
    "https://updates.baijimu.com/api/bridge-agent#fragment",
  ])("rejects non-canonical release API URL %s", (value) => {
    expect(() => normalizeReleaseApiBase(value)).toThrow(
      /must be exactly https:\/\/updates\.baijimu\.com\/api\/bridge-agent/,
    );
  });

  test("accepts only the canonical updater URL", () => {
    expect(
      normalizeUpdateApiUrl(
        "https://updates.baijimu.com/api/bridge-agent/releases/latest/",
      ),
    ).toBe(
      "https://updates.baijimu.com/api/bridge-agent/releases/latest",
    );
    expect(() =>
      normalizeUpdateApiUrl(
        "https://updates.baijimu.com/api/bridge-agent/releases/latest/tauri",
      ),
    ).toThrow(
      /must be exactly https:\/\/updates\.baijimu\.com\/api\/bridge-agent\/releases\/latest/,
    );
  });
});
