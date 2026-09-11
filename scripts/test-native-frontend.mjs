import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve, join } from "node:path";
import { spawnSync } from "node:child_process";

const root = resolve(import.meta.dirname, "..");
const target = join(root, "src-tauri", "target");
const temporary = mkdtempSync(join(tmpdir(), "baijimu-frontend-smoke-"));
const config = JSON.parse(readFileSync(join(root, "src-tauri/tauri.conf.json"), "utf8"));
const environment = { ...process.env,
  TAURI_CONFIG: JSON.stringify({ identifier: "com.baijimu.bridgeagent.frontend-smoke", productName: "Baijimu Frontend Smoke",
    app: { security: { devCsp: config.app.security.csp } } }),
  CARGO_TARGET_DIR: target,
};
delete environment.BRIDGE_AGENT_UPDATE_API_URL;
// Keep the declared registries and platform compiler flags even with an isolated working directory.
const build = spawnSync("cargo", ["build", "--locked", "--config", join(root, ".cargo/config.toml"), "--manifest-path", join(root, "src-tauri/Cargo.toml")], {
  cwd: temporary, env: environment, stdio: "inherit", timeout: 20 * 60 * 1000
});
if (build.status !== 0) throw build.error ?? new Error(`native smoke build failed: ${build.status}`);
const executable = join(target, "debug", `bridge-agent-desktop${process.platform === "win32" ? ".exe" : ""}`);
const result = spawnSync(executable, [], {
  cwd: temporary,
  env: { ...environment, WS_BRIDGE_CONFIG: join(temporary, "agent-config.json"), BRIDGE_AGENT_FRONTEND_SMOKE: "1",
    BAIJIMU_MANAGED_TOOL_ROOT: join(temporary, "managed-tools"), BAIJIMU_MANAGED_BIN_DIR: join(temporary, "bin") },
  stdio: "inherit", timeout: 45000
});
if (result.status !== 0) throw result.error ?? new Error(`native frontend smoke failed: ${result.status}`);
console.log(`Native WebView recovery passed. Isolated diagnostics: ${temporary}`);
