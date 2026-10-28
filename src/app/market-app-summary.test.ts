import { describe, expect, it } from "vitest";
import { marketCapabilitySummary } from "./market-app-summary";

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
