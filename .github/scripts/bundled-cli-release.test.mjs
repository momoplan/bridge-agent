import {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { afterEach, describe, expect, it } from "vitest";

const scriptPath = "tools/baijimu-cli/prepare-bundled-cli.sh";
const pinnedCliVersion = readFileSync(
  "tools/baijimu-cli/VERSION",
  "utf8",
).trim();
const workflowPath = ".github/workflows/release-bridge-agent.yml";
const defenderScriptPath =
  "src-tauri/scripts/verify-windows-defender.ps1";
const defenderUpdateScriptPath =
  "src-tauri/scripts/update-defender-signatures.ps1";
const prepareScript = readFileSync(scriptPath, "utf8");
const temporaryDirectories = [];

function createFixture() {
  const root = mkdtempSync(join(tmpdir(), "baijimu-cli-release-test-"));
  temporaryDirectories.push(root);
  const assetsDirectory = join(root, "assets");
  const packageDirectory = join(root, "package");
  const packageBinDirectory = join(packageDirectory, "bin");
  const resourceDirectory = join(root, "resources");
  mkdirSync(assetsDirectory, { recursive: true });
  mkdirSync(packageBinDirectory, { recursive: true });

  const platform =
    process.platform === "darwin" ? "macos-universal" : "linux-x64";
  const assetName = `baijimu-cli-${pinnedCliVersion}-${platform}.zip`;
  const binaryPath = join(packageBinDirectory, "baijimu");
  writeFileSync(
    binaryPath,
    [
      "#!/usr/bin/env bash",
      'test "$1" = "--version"',
      'test "$2" = "--json"',
      `printf '{"version":"${pinnedCliVersion}"}\\n'`,
      "",
    ].join("\n"),
  );
  chmodSync(binaryPath, 0o755);

  const archivePath = join(assetsDirectory, assetName);
  const zipped = spawnSync("zip", ["-qr", archivePath, "bin"], {
    cwd: packageDirectory,
    encoding: "utf8",
  });
  if (zipped.status !== 0) {
    throw new Error(zipped.stderr || "zip failed");
  }
  const archiveSha256 = createHash("sha256")
    .update(readFileSync(archivePath))
    .digest("hex");
  const checksumPath = join(root, "SHA256SUMS");
  writeFileSync(checksumPath, `${archiveSha256}  ${assetName}\n`);

  return {
    archivePath,
    assetsDirectory,
    checksumPath,
    resourceDirectory,
  };
}

function runPrepare(fixture) {
  return spawnSync("bash", [scriptPath], {
    encoding: "utf8",
    env: {
      ...process.env,
      BAIJIMU_CLI_USE_RELEASE_ASSET: "true",
      BAIJIMU_CLI_RELEASE_ASSETS_DIR: fixture.assetsDirectory,
      BAIJIMU_CLI_CHECKSUM_FILE: fixture.checksumPath,
      BAIJIMU_CLI_RESOURCE_DIR: fixture.resourceDirectory,
    },
  });
}

afterEach(() => {
  while (temporaryDirectories.length > 0) {
    rmSync(temporaryDirectories.pop(), {
      force: true,
      recursive: true,
    });
  }
});

describe("bundled Baijimu CLI release provenance", () => {
  it.runIf(process.platform !== "win32")(
    "copies the pinned, checksum-verified release binary",
    () => {
      const fixture = createFixture();
      const result = runPrepare(fixture);

      expect(result.status, result.stderr).toBe(0);
      expect(result.stdout).toContain(`"version":"${pinnedCliVersion}"`);
      expect(result.stdout).toContain(
        "Prepared pinned Baijimu CLI OSS asset",
      );
      const preparedBinary = join(fixture.resourceDirectory, "baijimu");
      expect(existsSync(preparedBinary)).toBe(true);
      expect(readFileSync(preparedBinary)).toEqual(
        readFileSync(
          join(
            fixture.assetsDirectory,
            "..",
            "package",
            "bin",
            "baijimu",
          ),
        ),
      );
    },
  );

  it.runIf(process.platform !== "win32")(
    "rejects an asset whose bytes do not match the pinned checksum",
    () => {
      const fixture = createFixture();
      writeFileSync(fixture.archivePath, "tampered release asset");
      const result = runPrepare(fixture);

      expect(result.status).not.toBe(0);
      expect(result.stderr).toContain(
        "Baijimu CLI release checksum mismatch",
      );
      expect(
        existsSync(join(fixture.resourceDirectory, "baijimu")),
      ).toBe(false);
    },
  );

  it("uses the immutable CLI release and scans final Windows artifacts", () => {
    const workflow = readFileSync(workflowPath, "utf8");
    const defenderScript = readFileSync(defenderScriptPath, "utf8");
    const defenderUpdateScript = readFileSync(
      defenderUpdateScriptPath,
      "utf8",
    );

    expect(workflow).toContain(
      'BAIJIMU_CLI_USE_RELEASE_ASSET: "true"',
    );
    expect(workflow).toContain(
      "tools/baijimu-cli/prepare-bundled-cli.sh",
    );
    expect(workflow).toContain(
      "Scan signed Windows release with Microsoft Defender",
    );
    expect(workflow).toContain(
      "src-tauri/scripts/verify-windows-defender.ps1",
    );
    expect(workflow).toContain(
      "src-tauri/scripts/update-defender-signatures.test.ps1",
    );
    expect(defenderScript).toContain("Update-DefenderSignaturesWithRetry");
    expect(defenderScript).toContain("SignatureUpdateMaxAttempts = 3");
    expect(defenderScript).toContain("SignatureUpdateRetrySeconds = 10");
    expect(defenderUpdateScript).toContain(
      "Update-MpSignature -ErrorAction Stop",
    );
    expect(defenderUpdateScript).toContain(
      "Microsoft Defender signature update failed after {0} attempts",
    );
    expect(defenderUpdateScript).toMatch(
      /if \(\$attempt -eq \$MaxAttempts\) \{\s*throw/s,
    );
    expect(defenderScript).toContain("Get-AuthenticodeSignature");
    expect(defenderScript).toContain('signature.Status -ne "Valid"');
    expect(defenderScript).toContain("ZoneId=3");
    expect(defenderScript).toContain("Start-MpScan");
    expect(defenderScript).toContain("Get-MpThreatDetection");
    expect(defenderScript).toContain("Get-MpThreatCatalog");
  });

  it("downloads the pinned content-addressed OSS asset without repository access", () => {
    expect(prepareScript).toContain(
      "https://lowcode-common.oss-cn-beijing.aliyuncs.com/managed-tool-artifacts/baijimu-cli/releases",
    );
    expect(prepareScript).toContain(
      'source_url="${release_base_url}/v${pinned_cli_version}/${expected_sha256}/${asset_name}"',
    );
    expect(prepareScript).toContain('"${source_url}" -o "${temporary_dir}/${asset_name}"');
    expect(prepareScript).not.toContain("api.github.com");
    expect(prepareScript).not.toContain("gh release download");
    expect(prepareScript).not.toContain("gitee.com");
  });
});
