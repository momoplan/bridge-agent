import { describe, expect, it } from "vitest";
import { parseBaijimuDeepLink } from "./deep-link";

describe("Baijimu desktop deep links", () => {
  it("accepts a registered local app install route with or without a share id", () => {
    expect(parseBaijimuDeepLink("baijimu://apps/install?appId=app-01")).toEqual({
      kind: "local_app_install",
      appId: "app-01",
      shareId: null
    });
    expect(parseBaijimuDeepLink("baijimu://apps/install?appId=app-01&shareId=share_01-test"))
      .toEqual({
        kind: "local_app_install",
        appId: "app-01",
        shareId: "share_01-test"
      });
  });

  it("accepts a generic app activation route without asserting business state", () => {
    expect(parseBaijimuDeepLink("baijimu://open")).toEqual({ kind: "app_open" });
  });

  it.each([
    "https://apps/install?appId=app-01",
    "baijimu://apps/install",
    "baijimu://apps/install?appId=one&appId=two",
    "baijimu://apps/install?appId=contains%2Fslash",
    "baijimu://other/install",
    "baijimu://codex/open",
    "baijimu://codex/install#fragment",
    "baijimu://codex/install?next=https%3A%2F%2Fevil.example",
    "baijimu://codex/install?shareId=one&shareId=two",
    "baijimu://codex/install?shareId=contains%2Fslash",
    "baijimu://open/",
    "baijimu://open?authorized=true",
    "baijimu://open#fragment",
    "baijimu://bridge-agent/device-authorization-complete?user_code=secret",
    "baijimu://bridge-agent/device-authorization-complete#fragment",
    "baijimu://bridge-agent/device-authorization-complete",
    "baijimu://bridge-agent/other"
  ])("rejects unsupported or unsafe routes: %s", (url) => {
    expect(parseBaijimuDeepLink(url)).toBeNull();
  });
});
