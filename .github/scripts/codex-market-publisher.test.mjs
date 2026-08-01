import { readFileSync } from "node:fs";

import { describe, expect, test } from "vitest";

const publisher = readFileSync(
  "tools/release/publish-codex-local-app-market.sh",
  "utf8",
);

describe("Codex local app market publisher", () => {
  test("promotes the published version above the current application rank", () => {
    expect(publisher).toContain("COALESCE(MAX(rank_order), 0) + 1");
    expect(publisher).toContain("rank_order=VALUES(rank_order)");
    expect(publisher).not.toMatch(/\n\s*400, '\$\{published_at\}'/);
  });

  test("publishes and verifies the Codex host compatibility contract", () => {
    expect(publisher).toContain('minimumVersion: "0.2.21"');
    expect(publisher).toContain('capabilities: ["connector.setup.v1"]');
    expect(publisher).toContain("hostVersion=0.2.21");
    expect(publisher).toContain("hostCapabilities=connector.setup.v1");
    expect(publisher).toContain(".latestVersion.compatibility.compatible == true");
  });
});
