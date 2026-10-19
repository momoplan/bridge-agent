import { readFileSync, mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { join, resolve } from "node:path";
import { tmpdir } from "node:os";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";

// Exercise released Linux bytes through the client's real managed installer.
// Only public immutable downloads and captured Owner contracts are used; no user PAT.
if (process.platform !== "linux" || process.arch !== "x64") throw new Error("Linux x64 runner required");
const root = resolve(import.meta.dirname, "..");
const fixture = join(root, "tests/fixtures/cli-upgrade");
const baseline = JSON.parse(readFileSync(join(fixture, "baseline.json"), "utf8"));
const listing = JSON.parse(readFileSync(join(fixture, "market-listing.json"), "utf8"));
const current = JSON.parse(readFileSync(join(root, "tools/baijimu-cli/SOURCE.json"), "utf8"));
if (JSON.stringify(current.source) !== JSON.stringify(listing.frozenVersion.source)
    || current.listingId !== listing.listingId) throw new Error("Upgrade fixture differs from bundled release");
const directory = mkdtempSync(join(tmpdir(), "baijimu-linux-cli-upgrade-"));
try {
  for (const [name, artifacts] of [["previous", baseline.artifacts], ["current", listing.frozenVersion.content.manifest.artifacts]]) {
    const artifact = artifacts.find((item) => item.platform === process.platform && item.arch === "x86_64");
    if (!artifact) throw new Error(`Missing ${name} Linux artifact`);
    const response = await fetch(artifact.source, { signal: AbortSignal.timeout(120_000), redirect: "error" });
    if (!response.ok) throw new Error(`${name} download failed: ${response.status}`);
    const bytes = Buffer.from(await response.arrayBuffer());
    const checksum = `sha256:${createHash("sha256").update(bytes).digest("hex")}`;
    if (checksum !== artifact.checksum) throw new Error(`${name} checksum mismatch`);
    writeFileSync(join(directory, `${name}.zip`), bytes);
  }
  const result = spawnSync("cargo", ["test", "--locked", "--manifest-path", "src-tauri/Cargo.toml",
    "linux_cli_", "--", "--ignored", "--nocapture"], {
    cwd: root, stdio: "inherit", timeout: 20 * 60 * 1000,
    env: { ...process.env, BAIJIMU_CLI_UPGRADE_FIXTURE_DIR: directory }
  });
  if (result.status !== 0) throw result.error ?? new Error(`Linux CLI upgrade failed: ${result.status}`);
} finally {
  rmSync(directory, { recursive: true, force: true });
}
