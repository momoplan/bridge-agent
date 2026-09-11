import { readFileSync } from "node:fs";

import { describe, expect, test } from "vitest";
import { readExpandedReleaseWorkflow } from "./release-workflow-source.mjs";

const workflow = readExpandedReleaseWorkflow(
  ".github/workflows/release-bridge-agent.yml",
);
const qualityWorkflow = readFileSync(".github/workflows/quality.yml", "utf8");
const windowsTauriConfig = JSON.parse(
  readFileSync("src-tauri/tauri.windows.conf.json", "utf8"),
);
const tauriConfig = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));
const packageJson = JSON.parse(readFileSync("package.json", "utf8"));
const packageLock = JSON.parse(readFileSync("package-lock.json", "utf8"));
const bundledCliAppId = readFileSync("tools/baijimu-cli/APP_ID", "utf8").trim();
const cargoManifests = [
  "Cargo.toml",
  "src-tauri/Cargo.toml",
  "tools/windows-uninstaller/Cargo.toml",
];
const windowsUninstallerPreparation = readFileSync(
  "scripts/prepare-windows-uninstaller.mjs",
  "utf8",
);
const linuxDependencyInstaller = readFileSync(
  ".github/scripts/install-linux-release-dependencies.sh",
  "utf8",
);
const giteeCargoConfigurator = readFileSync(
  ".github/scripts/configure-gitee-cargo.sh",
  "utf8",
);

function jobBody(jobName, nextJobName) {
  const startMarker = `  ${jobName}:\n`;
  const start = workflow.indexOf(startMarker);
  if (start < 0) {
    throw new Error(`workflow job not found: ${jobName}`);
  }

  const end =
    nextJobName === undefined
      ? workflow.length
      : workflow.indexOf(`  ${nextJobName}:\n`, start + startMarker.length);
  if (end < 0) {
    throw new Error(`next workflow job not found: ${nextJobName}`);
  }

  return workflow.slice(start, end);
}

function cargoPackageVersion(path) {
  const manifest = readFileSync(path, "utf8");
  const version = manifest.match(/^version = "([^"]+)"$/m)?.[1];
  if (version === undefined) {
    throw new Error(`package version not found: ${path}`);
  }
  return version;
}

describe("release workflow repository script availability", () => {
  test("only manual releases select platforms and preflight the service before signing", () => {
    const source = readFileSync(".github/workflows/release-bridge-agent.yml", "utf8");
    const triggers = source.slice(source.indexOf("on:"), source.indexOf("\nenv:"));
    expect(triggers).toContain("workflow_dispatch:");
    expect(triggers).not.toContain("push:");
    expect(triggers).toContain("release_platforms:");
    const selection = jobBody("select-platforms", "frontend-recovery-gate");
    expect(selection).toContain("verify-platform-release.mjs preflight");
    expect(selection).not.toContain("SSL_COM_");
    expect(jobBody("prepare-domestic-release", "quality-gate")).toContain("needs: select-platforms");
    expect(jobBody("release", "mirror-domestic-release")).toContain("fromJSON(needs.select-platforms.outputs.matrix)");
    expect(jobBody("verify-update-service")).toContain("RELEASE_PLATFORMS: ${{ needs.mirror-domestic-release.outputs.platforms }}");
  });

  test("white-screen repair compares against the signed MSI payload, not the unsigned build intermediate", () => {
    const smoke = readFileSync(".github/scripts/release-steps/16-smoke-test-windows-white-screen-repair.ps1", "utf8");
    expect(smoke).toContain('Get-AuthenticodeSignature -LiteralPath $installed');
    expect(smoke).toContain('$expected = (Get-FileHash $installed).Hash');
    expect(smoke).not.toContain('Get-FileHash "src-tauri/target/release/bridge-agent-desktop.exe"');
    expect(smoke.indexOf('"install-current-msi-baseline"')).toBeLessThan(smoke.indexOf('$expected ='));
    expect(smoke.indexOf('$expected =')).toBeLessThan(smoke.indexOf('"install-affected-version"'));
    expect(smoke.indexOf('"install-affected-version"')).toBeLessThan(smoke.indexOf('"repair-with-new-version"'));
  });

  test("frontend and native recovery are mandatory on both desktop platforms before release", () => {
    const gate = jobBody("frontend-recovery-gate", "prepare-domestic-release");
    const release = jobBody("release", "mirror-domestic-release");
    const action = readFileSync(".github/actions/frontend-recovery/action.yml", "utf8");
    expect(gate).toContain("runner: [macos-15, windows-latest]");
    expect(qualityWorkflow).toContain("runner: [macos-15, windows-latest]");
    expect(gate).toContain("uses: ./.github/actions/frontend-recovery");
    expect(qualityWorkflow).toContain("uses: ./.github/actions/frontend-recovery");
    expect(release).toContain("needs: [select-platforms, prepare-domestic-release, quality-gate, windows-quality-gate, frontend-recovery-gate]");
    expect(action).toContain("npm run test:browser");
    expect(action).toContain("npm run test:native");
    expect(release.indexOf("repair-with-new-version")).toBeGreaterThan(0);
    expect(release.indexOf("repair-with-new-version")).toBeLessThan(release.indexOf("Upload bundles to GitHub release"));
  });

  test("all Bridge Agent release package versions remain aligned", () => {
    const expected = packageJson.version;

    expect(tauriConfig.version).toBe(expected);
    expect(packageLock.version).toBe(expected);
    expect(packageLock.packages[""].version).toBe(expected);
    for (const manifest of cargoManifests) {
      expect(cargoPackageVersion(manifest)).toBe(expected);
    }
  });

  test("bundled CLI release validates pinned provenance without live market access", () => {
    const body = jobBody("prepare-domestic-release", "quality-gate");

    expect(bundledCliAppId).toBe("baijimu-cli");
    expect(body).toContain("tools/baijimu-cli/APP_ID");
    expect(body).toContain("prepare-bundled-market-source.mjs");
    expect(body).not.toContain("verify-consumer-market.mjs");
    expect(body).not.toContain("LOCAL_APP_CONSUMER_");
    expect(body).toContain("BAIJIMU_CLI_MARKET_SOURCE");
    expect(body).not.toContain(".latestVersion.repo ==");
  });

  test("verify-update-service checks out the dispatched workflow commit", () => {
    const body = jobBody("verify-update-service");
    const checkoutIndex = body.indexOf("uses: actions/checkout@v4");
    const helperIndex = body.indexOf(
      "node .github/scripts/release-service-url.mjs",
    );

    expect(checkoutIndex).toBeGreaterThanOrEqual(0);
    expect(body).toContain(
      "ref: ${{ github.event_name == 'workflow_dispatch' && github.sha || github.ref }}",
    );
    expect(helperIndex).toBeGreaterThan(checkoutIndex);
  });

  test("release upload never expands an empty array under Bash nounset", () => {
    expect(workflow).not.toContain("prerelease_flag=()");
    expect(workflow).toContain("release_create_args=(");
    expect(workflow).toContain(
      'retry_gh gh release create "${release_create_args[@]}"',
    );
  });

  test("complete asset manifest is registered before automatic publication", () => {
    const mirrorBody = jobBody("mirror-domestic-release", "verify-update-service");
    const verifyBody = jobBody("verify-update-service");

    expect(workflow).not.toContain("--draft");
    expect(workflow).not.toContain("publish_only:");
    expect(mirrorBody).toContain("publish-platform-assets.mjs");
    const publisher = readFileSync(".github/scripts/publish-platform-assets.mjs", "utf8");
    expect(publisher.indexOf('run("register-release-manifest.mjs"')).toBeLessThan(
      publisher.indexOf('run("upload-oss-release-asset.mjs"'),
    );
    expect(verifyBody).toContain(
      "needs.mirror-domestic-release.result == 'success'",
    );
    expect(workflow).not.toContain('"$api/releases/$RELEASE_TAG/publish"');
  });

  test("Windows quality gate runs workspace tests and real PATH registry writes", () => {
    const body = jobBody("windows-quality-gate", "release");
    const prepareUninstallerIndex = body.indexOf(
      "npm run prepare:windows-uninstaller:quality",
    );
    const desktopCheckIndex = body.indexOf(
      "cargo check --locked --manifest-path src-tauri/Cargo.toml --all-targets",
    );
    expect(prepareUninstallerIndex).toBeGreaterThanOrEqual(0);
    expect(desktopCheckIndex).toBeGreaterThan(prepareUninstallerIndex);
    expect(body).toContain("cargo test --locked --workspace");
    expect(body).toContain(
      "rustup toolchain install stable --component clippy,rustfmt",
    );
    expect(body).toContain(
      "managed_tool::tests::windows_registry_path_registration_round_trips",
    );
    expect(body).toContain("cargo test --locked --manifest-path src-tauri/Cargo.toml");
    expect(windowsTauriConfig.build.beforeBuildCommand).toBe(
      "node scripts/prepare-windows-uninstaller.mjs --with-frontend",
    );
    expect(windowsTauriConfig.build.beforeBundleCommand).toBeUndefined();
    expect(windowsUninstallerPreparation).toContain(
      'run(process.execPath, ["scripts/prepare-tauri-build.mjs"])',
    );
    expect(windowsUninstallerPreparation).not.toContain("npm.cmd");
    expect(workflow).toContain("WiX linker diagnostic exit code");
    expect(workflow).toContain("required signed sidecar");
    expect(workflow).toContain("Installed executable has an invalid Authenticode signature");
  });

  test("pull requests run the same Windows Rust quality contract before release", () => {
    expect(qualityWorkflow).toContain("runs-on: windows-latest");
    expect(qualityWorkflow).toContain(
      "npm run prepare:windows-uninstaller:quality",
    );
    expect(qualityWorkflow).toContain(
      "./.github/scripts/release-steps/03-check-windows-rust-targets.ps1",
    );
  });

  test("all Rust build jobs authenticate immutable CModel dependencies without embedding tokens", () => {
    const qualityBody = jobBody("quality-gate", "windows-quality-gate");
    const windowsBody = jobBody("windows-quality-gate", "release");
    const releaseBody = jobBody("release", "mirror-domestic-release");

    for (const body of [qualityBody, windowsBody, releaseBody]) {
      expect(body).toContain("CARGO_NET_GIT_FETCH_WITH_CLI");
      expect(body).toContain("secrets.GITEE_ACCESS_TOKEN");
      expect(body).toContain("configure-gitee-cargo.sh");
    }
    expect(qualityWorkflow).toContain("CARGO_NET_GIT_FETCH_WITH_CLI");
    expect(qualityWorkflow).toContain("secrets.GITEE_ACCESS_TOKEN");
    expect(qualityWorkflow).toContain("configure-gitee-cargo.sh");
    expect(giteeCargoConfigurator).toContain(
      'password=$GITEE_ACCESS_TOKEN',
    );
    expect(giteeCargoConfigurator).not.toMatch(/gitee\.com\/[^'" ]+@/);
  });

  test("external publisher token cannot mutate internal upgrade policy", () => {
    expect(workflow).not.toContain("minimum_supported_version:");
    expect(workflow).not.toContain("force_update_message:");
    expect(workflow).not.toContain("release-policy");
  });

  test("published metadata uses platform-scoped queries and registered download URLs", () => {
    const body = jobBody("verify-update-service");
    expect(body).toContain("verify-platform-release.mjs verify");
    const verifier = readFileSync(".github/scripts/verify-platform-release.mjs", "utf8");
    expect(verifier).toContain('url.searchParams.set("platform", platform.id)');
    expect(verifier).toContain('new URL(assets[0].downloadUrl)');
    expect(verifier).toContain('asset.downloadUrl === update.url');
  });

  test("Linux dependency installation keeps bounded retries and network timeouts", () => {
    expect(
      workflow.match(/install-linux-release-dependencies\.sh/g),
    ).toHaveLength(2);
    expect(linuxDependencyInstaller).toContain("APT_COMMAND_TIMEOUT_SECONDS");
    expect(linuxDependencyInstaller).toContain("Acquire::Retries=");
    expect(linuxDependencyInstaller).toContain("Acquire::http::Timeout=15");
    expect(linuxDependencyInstaller).toContain("Acquire::https::Timeout=15");
  });

  test("macOS DMG remains a quality-gated drag-to-install bundle", () => {
    const releaseBody = jobBody("release", "mirror-domestic-release");
    const dmg = tauriConfig.bundle.macOS.dmg;

    expect(dmg.background).toBe("./images/dmg-background.png");
    expect(dmg.windowSize).toEqual({ width: 660, height: 432 });
    expect(dmg.appPosition.x).toBeLessThan(dmg.applicationFolderPosition.x);
    expect(releaseBody).toContain(
      "TAURI_BUNDLER_DMG_IGNORE_CI: ${{ runner.os == 'macOS' && 'true' || 'false' }}",
    );
    expect(releaseBody).toContain('readlink "$applications_link"');
    expect(releaseBody).toContain('!= "/Applications"');
    expect(releaseBody).toContain('[ ! -s "$mount_dir/.DS_Store" ]');
    expect(releaseBody).toContain(
      'cmp -s "$mount_dir/.background/dmg-background.png" "src-tauri/images/dmg-background.png"',
    );
  });
});
