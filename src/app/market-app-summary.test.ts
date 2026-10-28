import { describe, expect, it } from "vitest";
import { marketCapabilitySummary, matchesMarketAppQuery } from "./market-app-summary";

describe("market capability summary", () => {
  it("uses the structured version declarations and ignores unrelated author claims", () => {
    const app = {
      methods: [{ name: "read", description: "Read", inputSchema: {}, responseMode: "json", path: "/read", httpMethod: "GET" }],
      events: [{ name: "changed", description: "Changed", payloadSchema: {} }],
      permissions: [{ id: "files.read", title: "Read files", description: "Read", platforms: [] }],
      capability: "Everything", risk: "safe", publisher: "Claimed publisher",
    };
    expect(marketCapabilitySummary(app)).toBe("1 个方法 · 1 个事件 · 1 项权限");
    expect(marketCapabilitySummary({ methods: [], events: [], permissions: [] })).toBe("0 个方法 · 0 个事件 · 0 项权限");
  });
});

describe("market query", () => {
  it("finds exact-version declarations while excluding count labels and retired claims", () => {
    const app = {
      appId: "sample-tool", name: "Sample", description: "Example", version: "1.2.3",
      methods: [{ name: "ReadDocument", description: "Open a document", inputSchema: {}, responseMode: "json", path: "/read", httpMethod: "GET" }],
      events: [{ name: "document.changed", description: "Document saved", payloadSchema: {} }],
      permissions: [{ id: "files.read", title: "Read local files", description: "Selected folders only", platforms: [] }],
      capability: "forged capability", publisher: "forged publisher", risk: "forged risk",
    };
    for (const query of ["  READDOCUMENT  ", "document.changed", "files.read", "local files", "selected folders", "sample-tool", "1.2.3", ""]) {
      expect(matchesMarketAppQuery(app, query), query).toBe(true);
    }
    for (const query of ["forged", "方法", "事件", "项权限", "not-present"]) {
      expect(matchesMarketAppQuery(app, query), query).toBe(false);
    }
  });
});
