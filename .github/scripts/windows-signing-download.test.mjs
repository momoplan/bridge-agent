import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const workflowPaths = [
  ".github/workflows/release-baijimu-cli.yml",
  ".github/workflows/release-bridge-agent.yml",
  ".github/workflows/release-codex-local-app.yml",
  ".github/workflows/release-codex-completion-local-app.yml",
];

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
});
