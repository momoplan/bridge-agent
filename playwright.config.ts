import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/browser",
  testMatch: "**/*.e2e.ts",
  use: { baseURL: "http://127.0.0.1:1421", trace: "retain-on-failure" },
  webServer: { command: "npm run preview -- --host 127.0.0.1", url: "http://127.0.0.1:1421", reuseExistingServer: false },
});
