import { describe, expect, it } from "vitest";
import { verifyConsumerMarket } from "./verify-consumer-market.mjs";

function fixture() {
  const options = {
    baseUrl: "https://consumer.example.test/partner/v1/local-app-service/api/local-app-market/",
    workspaceId: "7", token: "test-pat", appId: "example-cli", version: "1.2.3",
    selection: { kind: "market", marketKey: "public-market", listingId: "catalog-1", version: "1.2.3",
      source: { application: { environmentKey: "author-a", appId: "example-cli" }, version: "1.2.3" } }
  };
  const listing = { marketKey: "public-market", listingId: "catalog-1", frozenVersion: {
    source: structuredClone(options.selection.source), content: { applicationType: "managed_tool", artifacts: [{ artifactId: "artifact-1" }] }
  } };
  const calls = [];
  const request = async (url, init) => {
    calls.push({ url, init });
    const data = url.pathname.endsWith("/context") ? "public-market" : { items: [listing], nextCursor: null };
    return { status: 200, json: async () => ({ contractVersion: "1.0.0", errorCode: "0", data }) };
  };
  return { options, listing, calls, request };
}
describe("consumer market release verification", () => {
  it("reads only the configured consumer owner with a workspace credential", async () => {
    const { options, calls, request } = fixture();
    await expect(verifyConsumerMarket(options, request)).resolves.toEqual({ listingId: "catalog-1", version: "1.2.3" });
    expect(calls).toHaveLength(2);
    for (const { url, init } of calls) {
      expect(url.origin).toBe("https://consumer.example.test");
      expect(url.searchParams.get("workspaceId")).toBe("7");
      expect(init.headers.Authorization).toBe("Bearer test-pat");
      expect(init.redirect).toBe("error");
    }
  });
  it("does not accept another environment's same-named app", async () => {
    const { options, listing, request } = fixture();
    listing.frozenVersion.source.application.environmentKey = "author-b";
    await expect(verifyConsumerMarket(options, request)).rejects.toThrow("bound market source");
  });
  it("requires the currently published version of the bound catalog", async () => {
    const { options, listing, request } = fixture();
    listing.frozenVersion.source.version = "1.2.4";
    await expect(verifyConsumerMarket(options, request)).rejects.toThrow("bound market source");
  });
  it("rejects absent credentials before sending any request", async () => {
    const { options, calls, request } = fixture();
    options.token = "";
    await expect(verifyConsumerMarket(options, request)).rejects.toThrow();
    expect(calls).toHaveLength(0);
  });
});
