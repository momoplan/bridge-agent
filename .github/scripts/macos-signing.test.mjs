import { mkdtempSync, readFileSync, writeFileSync, chmodSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { describe, expect, it } from "vitest";

const signer = ".github/scripts/sign-macos-with-retry.sh";
const releaseWorkflows = [
  ".github/workflows/release-baijimu-cli.yml",
  ".github/workflows/release-bridge-agent.yml",
  ".github/workflows/release-codex-completion-local-app.yml",
];

function runSigner(mode, maxAttempts = 4) {
  const directory = mkdtempSync(join(tmpdir(), "bridge-agent-macos-signing-"));
  const state = join(directory, "attempts");
  const binary = join(directory, "binary");
  const fakeCodesign = join(directory, "codesign");
  const fakeSleep = join(directory, "sleep");

  writeFileSync(binary, "fixture");
  writeFileSync(
    fakeCodesign,
    `#!/usr/bin/env bash
set -euo pipefail
state="${state}"
mode="${mode}"
if [ "$1" = "--display" ]; then
  attempts="$(cat "$state")"
  if [ "$mode" = "missing-timestamp-once" ] && [ "$attempts" -eq 1 ]; then
    echo "Timestamp=none" >&2
  else
    echo "Timestamp=Aug 2, 2026 at 12:00:00" >&2
  fi
  exit 0
fi
if [ "$1" = "--verify" ]; then
  exit 0
fi
attempts=0
if [ -f "$state" ]; then attempts="$(cat "$state")"; fi
attempts=$((attempts + 1))
printf '%s' "$attempts" > "$state"
case "$mode" in
  transient-twice)
    if [ "$attempts" -le 2 ]; then
      echo "A timestamp was expected but was not found." >&2
      exit 1
    fi
    ;;
  always-timestamp-failure)
    echo "The timestamp service is not available." >&2
    exit 1
    ;;
  deterministic-failure)
    echo "errSecInternalComponent" >&2
    exit 1
    ;;
esac
`,
  );
  writeFileSync(fakeSleep, "#!/usr/bin/env bash\nexit 0\n");
  chmodSync(fakeCodesign, 0o755);
  chmodSync(fakeSleep, 0o755);

  const result = spawnSync("bash", [signer, binary, "Developer ID Application: Test"], {
    encoding: "utf8",
    env: {
      ...process.env,
      CODESIGN_BIN: fakeCodesign,
      SLEEP_BIN: fakeSleep,
      CODESIGN_MAX_ATTEMPTS: String(maxAttempts),
      CODESIGN_RETRY_DELAY_SECONDS: "0",
    },
  });

  return {
    ...result,
    attempts: Number(readFileSync(state, "utf8")),
  };
}

describe("macOS signing timestamp recovery", () => {
  it("routes every macOS release workflow through the shared signer", () => {
    for (const workflowPath of releaseWorkflows) {
      const workflow = readFileSync(workflowPath, "utf8");
      expect(workflow).toContain("bash .github/scripts/sign-macos-with-retry.sh");
      expect(workflow).not.toMatch(/codesign[^\n]*--timestamp/);
    }
  });

  it("retries transient timestamp failures and verifies the final signature", () => {
    const result = runSigner("transient-twice");
    expect(result.status).toBe(0);
    expect(result.attempts).toBe(3);
    expect(result.stdout).toContain("macOS signature and trusted timestamp verified");
  });

  it("retries when codesign exits successfully without timestamp metadata", () => {
    const result = runSigner("missing-timestamp-once");
    expect(result.status).toBe(0);
    expect(result.attempts).toBe(2);
  });

  it("does not hide deterministic signing failures", () => {
    const result = runSigner("deterministic-failure");
    expect(result.status).toBe(1);
    expect(result.attempts).toBe(1);
    expect(result.stderr).toContain("non-timestamp error; not retrying");
  });

  it("fails after the configured timestamp retry budget is exhausted", () => {
    const result = runSigner("always-timestamp-failure", 3);
    expect(result.status).toBe(1);
    expect(result.attempts).toBe(3);
    expect(result.stderr).toContain("failed after 3 timestamp attempts");
  });
});
