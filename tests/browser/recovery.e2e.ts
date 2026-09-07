import { test, expect } from "@playwright/test";
import { installDesktopRuntime } from "../desktop-runtime";

test("production bundle renders populated desktop after startup", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.addInitScript(installDesktopRuntime);
  await page.goto("/");
  await expect(page.getByText("测试应用", { exact: true })).toBeVisible();
  await expect(page.getByText("测试 CLI", { exact: true })).toBeVisible();
  await expect(page.locator("#desktop-recovery")).toBeHidden();
  expect(errors).toEqual([]);
});

test("missing business JavaScript leaves independent update and log actions", async ({ page }) => {
  await page.addInitScript(installDesktopRuntime);
  await page.route("**/assets/business-entry-*.js", (route) => route.abort());
  await page.goto("/");
  await expect(page.locator("#recovery-title")).toHaveText("界面暂时无法显示");
  await page.getByRole("button", { name: "检查更新", exact: true }).click();
  await expect(page.locator("#recovery-status")).toContainText("发现新版本");
  await page.getByRole("button", { name: "安装官方更新", exact: true }).click();
  await expect(page.locator("#recovery-status")).toContainText("更新已安装");
  await page.getByRole("button", { name: "打开启动日志", exact: true }).click();
  expect(await page.evaluate(() => (window as unknown as { desktopFixture: { calls: string[] } }).desktopFixture.calls)).toContain("open_startup_log");
});

test("render crash leaves a visible recovery panel", async ({ page }) => {
  await page.addInitScript(installDesktopRuntime, { crash: true });
  await page.goto("/");
  await expect(page.locator("#recovery-title")).toHaveText("界面暂时无法显示");
  await page.getByRole("button", { name: "打开系统修复窗口", exact: true }).click();
  expect(await page.evaluate(() => (window as unknown as { desktopFixture: { calls: string[] } }).desktopFixture.calls)).toContain("open_native_recovery");
});

test("missing bootstrap still displays static recovery instructions", async ({ page }) => {
  await page.route("**/assets/*.js", (route) => route.abort());
  await page.goto("/");
  await expect(page.locator("#desktop-recovery")).toBeVisible();
  await expect(page.getByText("若按钮无响应，请从系统托盘选择“检查更新与修复”。")).toBeVisible();
});
