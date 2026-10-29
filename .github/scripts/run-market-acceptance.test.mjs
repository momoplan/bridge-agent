import { describe, expect, it } from "vitest";
import { runMarketAcceptance } from "./run-market-acceptance.mjs";

const commit = "a".repeat(40);
const env = { BRIDGE_AGENT_MARKET_ACCESS_JSON: "private-configuration", GITHUB_SHA: commit };
const passed = (receipt = { contractVersion: "4.0.0", listings: 7, versions: 7 }) => ({
  status: 0,
  stdout: `MARKET_READ_ACCEPTANCE ${JSON.stringify(receipt)}\ntest result: ok. 1 passed; 0 failed; 0 ignored; 40 filtered out;\n`,
});

describe("real market release acceptance", () => {
  it("runs the actual desktop consumer explicitly, independently of bundled CLI provenance", () => {
    const receipt = runMarketAcceptance(env, commit, (command, args, options) => {
      expect(command).toBe("cargo");
      expect(args).toContain("--locked");
      expect(args).toContain("desktop::market_live_tests::current_device_market_reads_and_renders_reviewed_listings");
      expect(args.slice(-3)).toEqual(["--ignored", "--exact", "--nocapture"]);
      expect(options.env).toBe(env);
      return passed();
    });
    expect(receipt).toEqual({ sourceCommit: commit, contractVersion: "4.0.0", listings: 7, versions: 7 });
    expect(JSON.stringify(receipt)).not.toContain(env.BRIDGE_AGENT_MARKET_ACCESS_JSON);
  });
  it("blocks a missing credential or mismatched release commit before execution", () => {
    const never = () => { throw new Error("must not execute"); };
    expect(() => runMarketAcceptance({}, commit, never)).toThrow("Missing release market access");
    expect(() => runMarketAcceptance(env, "b".repeat(40), never)).toThrow("exact release source");
  });
  it("does not mistake a zero-test or skipped run for acceptance", () => {
    for (const stdout of ["test result: ok. 0 passed; 0 failed; 0 ignored;", "test result: ok. 0 passed; 0 failed; 1 ignored;"]) {
      expect(() => runMarketAcceptance(env, commit, () => ({ status: 0, stdout }))).toThrow("exactly one");
    }
  });
  it("blocks transport/contract failures and incomplete or wrong-version receipts", () => {
    expect(() => runMarketAcceptance(env, commit, () => ({ status: 101, stderr: "private-data" }))).toThrow("release is blocked");
    for (const receipt of [
      { contractVersion: "3.0.0", listings: 7, versions: 7 },
      { contractVersion: "4.0.0", listings: 0, versions: 0 },
      { contractVersion: "4.0.0", listings: 7, versions: 6 },
    ]) expect(() => runMarketAcceptance(env, commit, () => passed(receipt))).toThrow("complete catalog");
  });
});
