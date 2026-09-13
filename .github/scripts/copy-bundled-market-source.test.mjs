import { chmodSync, mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync, existsSync } from "node:fs";
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


it.skipIf(process.platform === "win32")("source builds enforce and copy the same pinned provenance as release assets", () => {
  const root = mkdtempSync(join(tmpdir(), "source-bundled-cli-"));
  try {
    const tools = join(root, "tools/baijimu-cli");
    const scripts = join(root, ".github/scripts");
    const cli = join(root, "cli");
    const bin = join(root, "commands");
    for (const dir of [tools, scripts, join(cli, "target/release"), bin]) mkdirSync(dir, { recursive: true });
    writeFileSync(join(tools, "VERSION"), "1.2.3");
    writeFileSync(join(tools, "APP_ID"), "fixture-cli");
    writeFileSync(join(cli, "Cargo.toml"), 'version = "1.2.3"\n');
    for (const file of ["prepare-bundled-cli.sh"]) {
      writeFileSync(join(tools, file), readFileSync(resolve("tools/baijimu-cli", file)));
    }
    writeFileSync(join(scripts, "copy-bundled-market-source.mjs"), readFileSync(resolve(".github/scripts/copy-bundled-market-source.mjs")));
    for (const [path, body] of [
      [join(bin, "cargo"), "#!/bin/sh\nexit 0\n"],
      [join(bin, "uname"), "#!/bin/sh\necho Linux\n"],
      [join(cli, "target/release/baijimu"), `#!/bin/sh\necho '{"version":"1.2.3"}'\n`]
    ]) { writeFileSync(path, body); chmodSync(path, 0o755); }
    const binary = join(root, "resources/baijimu");
    const run = () => spawnSync("bash", [join(tools, "prepare-bundled-cli.sh")], {
      encoding: "utf8", env: { ...process.env, PATH: `${bin}:${process.env.PATH}`,
        BAIJIMU_CLI_USE_RELEASE_ASSET: "false", BAIJIMU_CLI_RS_DIR: cli,
        BAIJIMU_CLI_RESOURCE_DIR: join(root, "resources"), BAIJIMU_CLI_REQUIRE_MARKET_SOURCE: "true" }
    });
    expect(run().status).not.toBe(0);
    expect(existsSync(`${binary}.market.json`)).toBe(false);
    const source = { kind: "market", marketKey: "fixture-market", listingId: "fixture-listing",
      version: "1.2.3", source: { application: { environmentKey: "fixture-env", appId: "fixture-cli" }, version: "1.2.3" } };
    writeFileSync(join(tools, "INSTALL_SOURCE.json"), JSON.stringify(source));
    const result = run();
    expect(result.stderr).toBe("");
    expect(result.status).toBe(0);
    expect(JSON.parse(readFileSync(`${binary}.market.json`, "utf8"))).toEqual(source);
  } finally { rmSync(root, { recursive: true, force: true }); }
});
