import { describe, expect, it } from "vitest";
import { parseBaijimuDeepLink } from "./deep-link";

describe("Baijimu desktop deep links", () => {
  it("accepts the Codex install route with or without a share id", () => {
    expect(parseBaijimuDeepLink("baijimu://codex/install")).toEqual({
      kind: "codex_install",
      shareId: null
    });
    expect(parseBaijimuDeepLink("baijimu://codex/install?shareId=share_01-test"))
      .toEqual({
        kind: "codex_install",
        shareId: "share_01-test"
      });
  });

  it("accepts the device authorization completion route without authorization data", () => {
    expect(
      parseBaijimuDeepLink("baijimu://bridge-agent/device-authorization-complete")
    ).toEqual({ kind: "device_authorization_complete" });
  });

  it.each([
    "https://codex/install",
    "baijimu://other/install",
    "baijimu://codex/open",
    "baijimu://codex/install#fragment",
    "baijimu://codex/install?next=https%3A%2F%2Fevil.example",
    "baijimu://codex/install?shareId=one&shareId=two",
    "baijimu://codex/install?shareId=contains%2Fslash",
    "baijimu://bridge-agent/device-authorization-complete?user_code=secret",
    "baijimu://bridge-agent/device-authorization-complete#fragment",
    "baijimu://bridge-agent/other"
  ])("rejects unsupported or unsafe routes: %s", (url) => {
    expect(parseBaijimuDeepLink(url)).toBeNull();
  });
});
