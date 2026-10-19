import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { prepareBundledMarketSource } from "./prepare-bundled-market-source.mjs";
const source = { kind: "market", listingId: "listing-a", version: "1.2.3",
  source: { application: { environmentKey: "author-a", appId: "cli-a" }, version: "1.2.3" } };
it("preserves pinned provenance without network access or credentials", () => {
  expect(prepareBundledMarketSource(source, "cli-a", "1.2.3")).toEqual(source);
});
it("rejects missing provenance or mismatched app and version", () => {
  for (const candidate of [null, { ...source, listingId: "" }, { ...source, version: "1.2.4" }])
    expect(() => prepareBundledMarketSource(candidate, "cli-a", "1.2.3")).toThrow();
  expect(() => prepareBundledMarketSource(source, "cli-b", "1.2.3")).toThrow();
  expect(() => prepareBundledMarketSource(source, "cli-a", "1.2.4")).toThrow();
});
it("does not include extra configuration or credentials in the bundle", () => {
  expect(prepareBundledMarketSource({ ...source, token: "not-for-bundle" }, "cli-a", "1.2.3")).toEqual(source);
});

it("versioned release provenance matches VERSION and the captured immutable Owner response", () => {
  const pinned = JSON.parse(readFileSync("tools/baijimu-cli/SOURCE.json", "utf8"));
  const version = readFileSync("tools/baijimu-cli/VERSION", "utf8").trim();
  const appId = readFileSync("tools/baijimu-cli/APP_ID", "utf8").trim();
  expect(prepareBundledMarketSource(pinned, appId, version)).toEqual(pinned);
  const owner = JSON.parse(readFileSync("tests/fixtures/cli-upgrade/market-listing.json", "utf8"));
  expect(owner.frozenVersion.source).toEqual(pinned.source);
  expect(owner.listingId).toBe(pinned.listingId);
});
