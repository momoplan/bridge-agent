import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { it, expect } from "vitest";

it("delivers only the verified source matching the pinned bundled binary", () => {
  const root = mkdtempSync(join(tmpdir(), "market-bundled-source-"));
  try {
    mkdirSync(join(root, "tools/baijimu-cli"), { recursive: true });
    writeFileSync(join(root, "tools/baijimu-cli/VERSION"), "1.2.3");
    writeFileSync(join(root, "tools/baijimu-cli/APP_ID"), "fixture-cli");
    const binary = join(root, "fixture-cli");
    const run = () => spawnSync(process.execPath, [resolve(".github/scripts/copy-bundled-market-source.mjs"), root, binary], {
      env: { ...process.env, BAIJIMU_CLI_REQUIRE_MARKET_SOURCE: "true" }, encoding: "utf8"
    });
    expect(run().status).not.toBe(0);
    const source = { kind: "market", marketKey: "fixture-market", listingId: "00000000-0000-0000-0000-000000000001",
      version: "1.2.3", source: { application: { environmentKey: "fixture-env", appId: "fixture-cli" }, version: "1.2.3" } };
    const path = join(root, "tools/baijimu-cli/INSTALL_SOURCE.json");
    writeFileSync(path, JSON.stringify(source));
    expect(run().status).toBe(0);
    expect(JSON.parse(readFileSync(`${binary}.market.json`, "utf8"))).toEqual(source);
    rmSync(`${binary}.market.json`);
    source.source.version = "1.2.4";
    writeFileSync(path, JSON.stringify(source));
    expect(run().status).not.toBe(0);
    expect(existsSync(`${binary}.market.json`)).toBe(false);
  } finally { rmSync(root, { recursive: true, force: true }); }
});
