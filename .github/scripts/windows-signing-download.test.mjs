import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const workflowPaths = [
  ".github/workflows/release-baijimu-cli.yml",
  ".github/workflows/release-bridge-agent.yml",
  ".github/workflows/release-codex-local-app.yml",
  ".github/workflows/release-codex-completion-local-app.yml",
];

const bridgeSigningScriptPath = "src-tauri/scripts/sign-windows-artifact.ps1";
const unicodeLauncherPath =
  "src-tauri/scripts/CodeSignToolUnicodeLauncher.java";
const tauriConfigPath = "src-tauri/tauri.conf.json";
const windowsTauriConfigPath = "src-tauri/tauri.windows.conf.json";

describe("Windows signing tool download", () => {
  for (const workflowPath of workflowPaths) {
    it(`${workflowPath} uses the pinned archive with an accepted user agent and retries`, () => {
      const workflow = readFileSync(workflowPath, "utf8");

      expect(workflow).toContain(
        "https://ssl.com/wp-content/uploads/2024/10/CodeSignTool-v1.3.1-windows.zip",
      );
      expect(workflow).toContain(
        'Mozilla/5.0 (Windows NT 10.0; Win64; x64) GitHub-Actions',
      );
      expect(workflow).toMatch(/-MaximumRetryCount 3(?: `)?/);
      expect(workflow).toMatch(
        /e45a9e6c2aac4cae16c114eb590a2196406681357eb587507c65cd3646b5330d/,
      );
    });
  }

  it("reconstructs the Chinese MSI program name inside Java without cmd.exe", () => {
    const signingScript = readFileSync(bridgeSigningScriptPath, "utf8");
    const unicodeLauncher = readFileSync(unicodeLauncherPath, "utf8");

    expect(signingScript).toContain("$brandProgramName");
    expect(signingScript).toContain('"-Dfile.encoding=UTF-8"');
    expect(signingScript).toContain("& $javaExecutable.FullName");
    expect(signingScript).toContain("[Convert]::ToBase64String");
    expect(signingScript).toContain("CodeSignToolUnicodeLauncher");
    expect(signingScript).not.toContain(
      '$arguments += "-program_name=$programName"',
    );
    expect(unicodeLauncher).toContain("Base64.getDecoder().decode");
    expect(unicodeLauncher).toContain("StandardCharsets.UTF_8");
    expect(unicodeLauncher).toContain('"-program_name=" + programName');
    expect(signingScript).toContain("$stagedInputFile");
    expect(signingScript).toContain("bridge-agent-signing-input");
    expect(signingScript).toContain("$unsignedBackupFile");
    expect(signingScript).not.toContain(
      "File]::Replace($replacementFile, $resolvedFile.Path, $null)",
    );
    expect(signingScript).toContain("WINDOWS_SIGNING_LOG_PATH");
    expect(signingScript).not.toContain('& ".\\CodeSignTool.bat" @arguments');
  });

  it("preserves Windows signing diagnostics when the Tauri bundle step fails", () => {
    const workflow = readFileSync(
      ".github/workflows/release-bridge-agent.yml",
      "utf8",
    );

    expect(workflow).toContain("WINDOWS_SIGNING_LOG_PATH");
    expect(workflow).toContain(
      "failure() && runner.os == 'Windows' && steps.build_release_bundles.outcome == 'failure'",
    );
    expect(workflow).toContain("Windows signing diagnostics:");
  });

  it("pins an Authenticode metadata parser for the Chinese program name", () => {
    const workflow = readFileSync(
      ".github/workflows/release-bridge-agent.yml",
      "utf8",
    );

    expect(workflow).toContain(
      "https://github.com/mtrojnar/osslsigncode/releases/download/2.14/osslsigncode-2.14-windows-x64-mingw.zip",
    );
    expect(workflow).toContain(
      "9a1722aaf62a27852c4eb9c35749a0248065052d0ae0a93d4ed6bb49def027f2",
    );
    expect(workflow).toContain("OSSLSIGNCODE_PATH");
    expect(workflow).toContain("Text description:\\s*百积木");
    expect(workflow).not.toContain(
      "signatureDetails -notmatch '(?m)^\\s*Description:",
    );
  });

  it("uses the canonical Chinese product name while preserving the MSI upgrade identity", () => {
    const tauriConfig = JSON.parse(readFileSync(tauriConfigPath, "utf8"));
    const windowsTauriConfig = JSON.parse(
      readFileSync(windowsTauriConfigPath, "utf8"),
    );

    expect(tauriConfig.productName).toBe("百积木");
    expect(windowsTauriConfig).not.toHaveProperty("productName");
    expect(windowsTauriConfig.bundle.windows.wix.upgradeCode).toBe(
      "94895101-CD67-53B8-BB30-F95026802DF2",
    );
    expect(windowsTauriConfig.bundle.windows.wix.language).toBe("zh-CN");
  });
});
