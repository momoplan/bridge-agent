import { describe, expect, it } from "vitest";
import { marketSelectionKey, sameMarketApplication, type InstallSource } from "./market-identity";

const selection = (environmentKey: string, listingId = "catalog-1", version = "1.0.0"): InstallSource => ({
  kind: "market", marketKey: "market-a", listingId, version,
  source: { application: { environmentKey, appId: "example.app" }, version }
});

describe("market source identity", () => {
  it("matches versions only within the same complete market source", () => {
    expect(sameMarketApplication(selection("env-a"), selection("env-a", "catalog-1", "1.1.0"))).toBe(true);
    expect(sameMarketApplication(selection("env-a"), selection("env-b"))).toBe(false);
    expect(sameMarketApplication(selection("env-a"), selection("env-a", "catalog-2"))).toBe(false);
    expect(sameMarketApplication(selection("env-a"), { ...selection("env-a"), marketKey: "market-b" } as InstallSource)).toBe(false);
  });
  it("does not infer provenance for historical or environment installations", () => {
    expect(sameMarketApplication(undefined, selection("env-a"))).toBe(false);
    expect(sameMarketApplication({ kind: "environment", source: selection("env-a").source }, selection("env-a"))).toBe(false);
  });
  it("keys market cards by market and catalog instead of the repeated app id", () => {
    expect(marketSelectionKey({ installSource: selection("env-a") }))
      .not.toBe(marketSelectionKey({ installSource: selection("env-b", "catalog-2") }));
    expect(marketSelectionKey({ installSource: selection("env-a") }))
      .toBe(marketSelectionKey({ installSource: selection("env-a", "catalog-1", "1.1.0") }));
  });
});
