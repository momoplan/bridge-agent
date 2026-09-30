import { afterEach, describe, expect, it } from "vitest";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { readExpandedReleaseWorkflow } from "./release-workflow-source.mjs";

const fixtures = [];
afterEach(() => {
  for (const path of fixtures.splice(0)) rmSync(path, { recursive: true, force: true });
});

describe("release workflow script expansion", () => {
  it("expands Bash and PowerShell references identically for LF and CRLF checkouts", () => {
    const directory = mkdtempSync(join(tmpdir(), "release-workflow-newlines-"));
    fixtures.push(directory);
    const source = readFileSync(".github/workflows/release-bridge-agent.yml", "utf8")
      .replace(/\r\n/g, "\n");
    const lf = join(directory, "lf.yml");
    const crlf = join(directory, "crlf.yml");
    writeFileSync(lf, source);
    writeFileSync(crlf, source.replace(/\n/g, "\r\n"));
    const expected = readExpandedReleaseWorkflow(lf);
    expect(readExpandedReleaseWorkflow(crlf)).toBe(expected);
    expect(expected).toContain("Invalid release tag:");
    expect(expected).toContain("Windows desktop-owned MSI upgrade failed");
    expect(expected).not.toContain("\r");
  });
});
