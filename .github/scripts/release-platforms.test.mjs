import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { afterEach, expect, test } from "vitest";
import { resolveReleasePlan, validateReleaseFiles } from "./release-platforms.mjs";

const version = "1.2.3";
const roots = [];
afterEach(() => roots.splice(0).forEach((root) => rmSync(root, { recursive: true, force: true })));
function fixture(plan) {
  const root = mkdtempSync(join(tmpdir(), "bridge-release-platforms-"));
  roots.push(root);
  const assets = join(root, "release-assets");
  mkdirSync(assets);
  for (const asset of plan.assets) {
    writeFileSync(join(assets, asset.name), "signed bundle");
    if (asset.signatureRequired) writeFileSync(join(assets, `${asset.name}.sig`), "updater signature");
  }
  return { root, assets };
}

test("macOS and Windows selection owns the build, three assets, and both Mac architectures", () => {
  const plan = resolveReleasePlan("macos,windows", version);
  expect(plan.matrix.include.map((target) => target.name)).toEqual(["macOS Universal", "Windows x64"]);
  expect(plan.qualityRunner).toBe("macos-15");
  expect(plan.assets).toHaveLength(3);
  expect(plan.updaters.map(({ target, arch }) => `${target}:${arch}`)).toEqual(["darwin:aarch64", "darwin:x86_64", "windows:x86_64"]);
  expect(JSON.stringify(plan)).not.toContain("Linux");
  validateReleaseFiles(plan, fixture(plan).assets);
});

test("empty selection preserves the full registered release", () => {
  const plan = resolveReleasePlan("", version);
  expect(plan.assets).toHaveLength(5);
  expect(plan.matrix.include).toHaveLength(3);
  expect(plan.updaters).toHaveLength(4);
});

test.each(["unknown", "macos,macos", "macos,", ",", "macos,unknown"])("invalid platform selection %s fails before publication", (selection) => {
  expect(() => resolveReleasePlan(selection, version)).toThrow(/distinct catalog keys/);
});

test("a new catalog platform needs no selector branch", () => {
  const plan = resolveReleasePlan("future", version, {
    future: { name: "Future", runner: "future-runner", qualityRunner: "quality-runner", tauri_args: "--bundles pkg", rust_targets: "", assets: [{ suffix: "future.pkg", signatureRequired: true, updaters: [{ target: "future", arch: "arch" }] }] },
  });
  expect(plan.assets[0].name).toBe("Baijimu_1.2.3_future.pkg");
  expect(plan.updaters[0]).toEqual({ target: "future", arch: "arch", name: plan.assets[0].name });
});

test("incomplete signatures and unselected platform files fail preflight", () => {
  const plan = resolveReleasePlan("macos,windows", version);
  const { assets } = fixture(plan);
  writeFileSync(join(assets, `Baijimu_${version}_amd64.deb`), "unexpected");
  expect(() => validateReleaseFiles(plan, assets)).toThrow(/outside selected platforms/);
  rmSync(join(assets, `Baijimu_${version}_amd64.deb`));
  rmSync(join(assets, `${plan.assets[0].name}.sig`));
  expect(() => validateReleaseFiles(plan, assets)).toThrow(/required signature/);
});

const unixTest = test.runIf(process.platform !== "win32");
unixTest("publication registers the selected complete manifest before uploading any asset", () => {
  const plan = resolveReleasePlan("macos,windows", version);
  const { root, assets } = fixture(plan);
  const bin = join(root, "bin");
  mkdirSync(bin);
  const commands = join(root, "commands.jsonl");
  writeFileSync(join(bin, "node"), `#!/usr/bin/env bash\nexec "$REAL_NODE" "$NODE_MOCK" "$@"\n`, { mode: 0o755 });
  const mock = join(root, "node-mock.mjs");
  writeFileSync(mock, `import { appendFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
const [script, ...args] = process.argv.slice(2);
if (script.endsWith('/register-release-manifest.mjs') || script.endsWith('/upload-oss-release-asset.mjs')) {
  appendFileSync(process.env.COMMAND_LOG, JSON.stringify({script,args})+'\\n');
} else {
  const child = spawnSync(process.execPath, [process.env.REPO_ROOT+'/'+script, ...args], {stdio:'inherit'});
  process.exit(child.status ?? 1);
}
`);
  const env = { ...process.env, PATH: `${bin}${delimiter}${process.env.PATH}`, REAL_NODE: process.execPath, NODE_MOCK: mock, COMMAND_LOG: commands, REPO_ROOT: process.cwd(), RELEASE_TAG: `bridge-agent-v${version}`, RELEASE_PLATFORMS: "macos,windows", BRIDGE_AGENT_RELEASE_API_URL: "https://updates.baijimu.com/api/bridge-agent", BRIDGE_AGENT_RELEASE_API_TOKEN: "test-token" };
  const script = resolve(".github/scripts/release-steps/18-upload-release-bundles-to-oss-and-register-metadata.sh");
  const result = spawnSync("bash", [script], { cwd: root, env, encoding: "utf8" });
  expect(result.status, result.stderr).toBe(0);
  const calls = readFileSync(commands, "utf8").trim().split("\n").map(JSON.parse);
  expect(calls).toHaveLength(4);
  expect(calls[0].script).toContain("register-release-manifest");
  expect(calls[0].args.slice(2)).toEqual(plan.assets.map((asset) => `${asset.target}::release-assets/${asset.name}::${asset.signatureRequired}`));
  expect(calls.slice(1).map((call) => call.args[4])).toEqual(plan.assets.map((asset) => `release-assets/${asset.name}`));
  rmSync(commands);
  rmSync(join(assets, `${plan.assets[0].name}.sig`));
  const missing = spawnSync("bash", [script], { cwd: root, env, encoding: "utf8" });
  expect(missing.status).not.toBe(0);
  expect(() => readFileSync(commands)).toThrow();
});

unixTest("public verification checks only selected updaters and rejects an incomplete release", () => {
  const plan = resolveReleasePlan("macos,windows", version);
  const { root } = fixture(plan);
  const bin = join(root, "bin");
  mkdirSync(bin);
  const commands = join(root, "requests.jsonl");
  const latestPath = join(root, "latest.json");
  const base = "https://download.baijimu.com/lowcode/direct-uploads/bridge-agent-release/test/";
  const latest = { version, assets: plan.assets.map((asset) => ({ ...asset, provider: "baijimu-oss", downloadUrl: base + asset.name })) };
  writeFileSync(latestPath, JSON.stringify(latest));
  const mock = join(root, "curl-mock.mjs");
  writeFileSync(join(bin, "curl"), '#!/usr/bin/env bash\nexec "$REAL_NODE" "$CURL_MOCK" "$@"\n', { mode: 0o755 });
  writeFileSync(mock, `import { appendFileSync, readFileSync } from 'node:fs';
const url = process.argv.at(-1);
appendFileSync(process.env.COMMAND_LOG, JSON.stringify(url)+'\\n');
const latest = JSON.parse(readFileSync(process.env.LATEST_PATH, 'utf8'));
if (url.includes('/tauri?')) {
  const query = new URL(url).searchParams;
  const asset = latest.assets.find(a => a.updaters.some(u => u.target === query.get('target') && u.arch === query.get('arch')));
  if (!asset) process.exit(1);
  console.log(JSON.stringify({version:latest.version, signature:'verified-signature', url:asset.downloadUrl}));
} else if (url.includes('?currentVersion=')) console.log(JSON.stringify(latest));
`);
  const env = { ...process.env, PATH: `${bin}${delimiter}${process.env.PATH}`, REAL_NODE: process.execPath, CURL_MOCK: mock, COMMAND_LOG: commands, LATEST_PATH: latestPath, RELEASE_TAG: `bridge-agent-v${version}`, RELEASE_PLATFORMS: "macos,windows", BRIDGE_AGENT_UPDATE_API_URL: "https://updates.baijimu.com/api/bridge-agent/releases/latest" };
  const script = resolve(".github/scripts/release-steps/19-verify-published-updater-metadata-and-public-downloads.sh");
  const result = spawnSync("bash", [script], { env, encoding: "utf8" });
  expect(result.status, result.stderr).toBe(0);
  const calls = readFileSync(commands, "utf8").trim().split("\n").map(JSON.parse);
  expect(calls.filter(url => url.includes('/tauri?'))).toHaveLength(3);
  expect(calls.join("\n")).not.toContain("target=linux");
  latest.assets.pop();
  writeFileSync(latestPath, JSON.stringify(latest));
  const missing = spawnSync("bash", [script], { env, encoding: "utf8" });
  expect(missing.status).not.toBe(0);
});
