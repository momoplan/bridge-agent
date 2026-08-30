import { readFileSync } from "node:fs";

import { describe, expect, test } from "vitest";

const workflow = readFileSync(
  ".github/workflows/release-bridge-agent.yml",
  "utf8",
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
  test("all Bridge Agent release package versions remain aligned", () => {
    const expected = packageJson.version;

    expect(tauriConfig.version).toBe(expected);
    expect(packageLock.version).toBe(expected);
    expect(packageLock.packages[""].version).toBe(expected);
    for (const manifest of cargoManifests) {
      expect(cargoPackageVersion(manifest)).toBe(expected);
    }
  });

  test("bundled CLI market gate resolves the versioned app identity", () => {
    const body = jobBody("prepare-domestic-release", "quality-gate");

    expect(bundledCliAppId).toBe("baijimu-cli");
    expect(body).toContain("tools/baijimu-cli/APP_ID");
    expect(body).toContain("select(.appId == $app_id)");
    expect(body).toContain("select(.latestVersion.manifest.appId == $app_id)");
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
    expect(mirrorBody).toContain("register-release-manifest.mjs");
    expect(mirrorBody.indexOf("register-release-manifest.mjs")).toBeLessThan(
      mirrorBody.indexOf('upload_asset "Windows x64"'),
    );
    expect(verifyBody).toContain(
      "needs.mirror-domestic-release.result == 'success'",
    );
    expect(workflow).not.toContain("/publish");
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

  test("published client metadata is verified through the canonical CDN", () => {
    const body = jobBody("verify-update-service");

    expect(body).toContain(
      "^https://download\\\\.baijimu\\\\.com/lowcode/direct-uploads/bridge-agent-release/",
    );
    expect(body).not.toContain(
      "^https://[a-z0-9][a-z0-9-]*\\\\.oss-[a-z0-9-]+\\\\.aliyuncs\\\\.com/lowcode/direct-uploads/bridge-agent-release/",
    );
  });

  test("Linux dependency installation has bounded retries and an official mirror fallback", () => {
    expect(
      workflow.match(/install-linux-release-dependencies\.sh/g),
    ).toHaveLength(2);
    expect(linuxDependencyInstaller).toContain("APT_COMMAND_TIMEOUT_SECONDS");
    expect(linuxDependencyInstaller).toContain("Acquire::Retries=");
    expect(linuxDependencyInstaller).toContain("Acquire::http::Timeout=15");
    expect(linuxDependencyInstaller).toContain("Acquire::https::Timeout=15");
    expect(linuxDependencyInstaller).toContain("https://archive.ubuntu.com/ubuntu");
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
