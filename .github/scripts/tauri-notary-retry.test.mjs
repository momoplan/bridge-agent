import { chmodSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { describe, expect, it } from "vitest";

const wrapper = ".github/scripts/tauri-build-with-notary-retry.sh";

function runWrapper(mode, maxAttempts = 4) {
  const directory = mkdtempSync(join(tmpdir(), "bridge-agent-tauri-notary-"));
  const state = join(directory, "attempts");
  const command = join(directory, "fake-tauri");
  const sleep = join(directory, "sleep");
  writeFileSync(command, `#!/usr/bin/env bash
set -euo pipefail
state="${state}"
attempts=0
if [ -f "$state" ]; then attempts="$(cat "$state")"; fi
attempts=$((attempts + 1))
printf '%s' "$attempts" > "$state"
case "${mode}" in
  transient-once)
    if [ "$attempts" -eq 1 ]; then
      echo 'failed to notarize app: NSURLErrorDomain Code=-1009 Internet connection appears to be offline; appstoreconnect.apple.com/notary' >&2
      exit 1
    fi
    ;;
  transient-always)
    echo 'failed to notarize app: network connection was lost; appstoreconnect.apple.com/notary' >&2
    exit 1
    ;;
  deterministic)
    echo 'failed to notarize app: invalid Apple credentials' >&2
    exit 1
    ;;
  unrelated-network)
    echo 'cargo download failed: connection reset' >&2
    exit 1
    ;;
esac
`);
  writeFileSync(sleep, "#!/usr/bin/env bash\nexit 0\n");
  chmodSync(command, 0o755);
  chmodSync(sleep, 0o755);

  const result = spawnSync("bash", [wrapper, command], {
    encoding: "utf8",
    env: {
      ...process.env,
      SLEEP_BIN: sleep,
      TAURI_NOTARY_MAX_ATTEMPTS: String(maxAttempts),
      TAURI_NOTARY_RETRY_DELAY_SECONDS: "0",
    },
  });
  return { ...result, attempts: Number(readFileSync(state, "utf8")) };
}

describe("Tauri Apple notarization recovery", () => {
  it("routes release builds through the notarization retry wrapper", () => {
    const workflow = readFileSync(".github/workflows/release-bridge-agent.yml", "utf8");
    expect(workflow).toContain("bash .github/scripts/tauri-build-with-notary-retry.sh");
  });

  it("retries a transient Apple notarization outage", () => {
    const result = runWrapper("transient-once");
    expect(result.status).toBe(0);
    expect(result.attempts).toBe(2);
  });

  it("does not retry deterministic notarization failures", () => {
    const result = runWrapper("deterministic");
    expect(result.status).toBe(1);
    expect(result.attempts).toBe(1);
    expect(result.stderr).toContain("not retrying");
  });

  it("does not treat unrelated network failures as notarization outages", () => {
    const result = runWrapper("unrelated-network");
    expect(result.status).toBe(1);
    expect(result.attempts).toBe(1);
  });

  it("fails after the bounded retry budget", () => {
    const result = runWrapper("transient-always", 3);
    expect(result.status).toBe(1);
    expect(result.attempts).toBe(3);
    expect(result.stderr).toContain("failed after 3 transient Apple notarization attempts");
  });
});
