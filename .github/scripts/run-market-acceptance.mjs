import { execFileSync, spawnSync } from "node:child_process";
import { writeFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

const testName = "desktop::market_live_tests::current_device_market_reads_and_renders_reviewed_listings";

export function runMarketAcceptance(env, sourceCommit, execute = spawnSync) {
  if (!env.BRIDGE_AGENT_LIVE_CONFIG && !env.BRIDGE_AGENT_MARKET_ACCESS_JSON) {
    throw new Error("Missing release market access: configure BRIDGE_AGENT_MARKET_ACCESS_JSON");
  }
  if (!/^[a-f0-9]{40}$/.test(sourceCommit) || (env.GITHUB_SHA && env.GITHUB_SHA !== sourceCommit)) {
    throw new Error("Market acceptance must run against the exact release source commit");
  }
  const result = execute("cargo", ["test", "--locked", "--manifest-path", "src-tauri/Cargo.toml",
    testName, "--", "--ignored", "--exact", "--nocapture"], {
    env, encoding: "utf8", maxBuffer: 16 * 1024 * 1024,
  });
  const output = result.stdout || "";
  if (result.error || result.status !== 0) {
    throw new Error("Real desktop market reading failed; release is blocked");
  }
  // cargo exits successfully even when a renamed/missing test matches zero tests.
  const receipts = [...output.matchAll(/^MARKET_READ_ACCEPTANCE (\{[^\n]+\})\r?$/gm)];
  if (receipts.length !== 1 || !/test result: ok\. 1 passed; 0 failed; 0 ignored;/.test(output)) {
    throw new Error("Real market acceptance did not execute exactly one successful test");
  }
  const receipt = JSON.parse(receipts[0][1]);
  if (receipt.contractVersion !== "4.0.0" || !Number.isSafeInteger(receipt.listings) ||
      receipt.listings < 1 || receipt.versions !== receipt.listings) {
    throw new Error("Real market acceptance did not verify the complete catalog and exact versions");
  }
  return { sourceCommit, ...receipt };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const sourceCommit = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
    const receipt = runMarketAcceptance(process.env, sourceCommit);
    writeFileSync("market-read-acceptance.json", `${JSON.stringify(receipt, null, 2)}\n`);
    console.log(`Verified real market v4: ${receipt.listings} listings and ${receipt.versions} exact versions at ${sourceCommit}`);
  } catch (error) {
    // Child output may contain configured auth data. Never forward it to CI logs.
    console.error(error.message);
    process.exitCode = 1;
  }
}
