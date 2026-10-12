import { test, expect } from "@playwright/test";
import { installDesktopRuntime } from "../desktop-runtime";

async function openSettings(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByRole("button", { name: /^设置/ }).click();
  await expect(page.getByRole("heading", { name: "通用", exact: true })).toBeVisible();
}

test("unauthorized users can disable autostart, navigate back and see the real system state", async ({ page }) => {
  await page.addInitScript(installDesktopRuntime, { authorized: false });
  await openSettings(page);
  const toggle = page.getByRole("switch", { name: "开机自启动" });
  await expect(toggle).toBeChecked();
  await toggle.click();
  await expect(toggle).not.toBeChecked();
  await expect(page.getByText("已关闭开机自启动", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "设备与账号", exact: true }).click();
  await page.getByRole("button", { name: "通用", exact: true }).click();
  await expect(toggle).not.toBeChecked();
  expect(await page.evaluate(() => (window as any).desktopFixture.calls.filter((c: string) => c === "save_config"))).toEqual([]);
});

test("independent saves send only changed fields and preserve concurrent data", async ({ page }) => {
  await page.addInitScript(installDesktopRuntime);
  await openSettings(page);
  await page.getByRole("button", { name: "设备与账号", exact: true }).click();
  const name = page.getByLabel("设备名称");
  await name.fill("我的工作电脑");
  await page.evaluate(() => { (window as any).desktopFixture.config.relay.reconnect_secs = 99; });
  await page.getByRole("button", { name: "保存更改", exact: true }).click();
  await expect(page.getByText("设备与账号已保存", { exact: true })).toBeVisible();
  expect(await page.evaluate(() => (window as any).desktopFixture.settingsWrites)).toEqual([{ section: "identity", changes: { "device.name": "我的工作电脑" } }]);
  expect(await page.evaluate(() => (window as any).desktopFixture.config.relay.reconnect_secs)).toBe(99);
  await page.getByRole("button", { name: "连接", exact: true }).click();
  await page.getByRole("button", { name: "展开高级设置" }).click();
  await expect(page.getByLabel("重连秒数")).toHaveValue("99");
});

test("unsaved changes can be cancelled, saved before switching, or discarded when leaving settings", async ({ page }) => {
  await page.addInitScript(installDesktopRuntime);
  await openSettings(page);
  await page.getByRole("button", { name: "设备与账号", exact: true }).click();
  await page.getByLabel("设备名称").fill("保存的新名称");
  await page.getByRole("button", { name: "连接", exact: true }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await page.getByRole("button", { name: "继续编辑", exact: true }).click();
  await expect(page.getByLabel("设备名称")).toHaveValue("保存的新名称");
  await page.getByRole("button", { name: "连接", exact: true }).click();
  await page.getByRole("button", { name: "保存并继续", exact: true }).click();
  await expect(page.getByRole("heading", { name: "连接", exact: true })).toBeVisible();
  await page.getByRole("button", { name: "设备与账号", exact: true }).click();
  await expect(page.getByLabel("设备名称")).toHaveValue("保存的新名称");
  await page.getByLabel("设备名称").fill("放弃的名称");
  await page.getByRole("button", { name: /^诊断/ }).click();
  await page.getByRole("button", { name: "放弃并继续", exact: true }).click();
  await expect(page.getByRole("heading", { name: "系统", exact: true })).toBeVisible();
  await page.getByRole("button", { name: /^设置/ }).click();
  await page.getByRole("button", { name: "设备与账号", exact: true }).click();
  await expect(page.getByLabel("设备名称")).toHaveValue("保存的新名称");
});

test("failed saves retain the draft and failed autostart writes show verified state", async ({ page }) => {
  await page.addInitScript(installDesktopRuntime, { settingsSaveError: true, autostartError: true });
  await openSettings(page);
  await page.getByRole("switch", { name: "开机自启动" }).click();
  await expect(page.getByRole("switch", { name: "开机自启动" })).toBeChecked();
  await expect(page.getByText("系统拒绝修改登录项")).toBeVisible();
  await page.getByRole("button", { name: "设备与账号", exact: true }).click();
  await page.getByLabel("设备名称").fill("保留的草稿");
  await page.getByRole("button", { name: "连接", exact: true }).click();
  await page.getByRole("button", { name: "保存并继续", exact: true }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await expect(page.getByRole("dialog").getByRole("alert")).toContainText("保存失败");
  await page.getByRole("button", { name: "继续编辑", exact: true }).click();
  await expect(page.getByLabel("设备名称")).toHaveValue("保留的草稿");
});

test("event policy and runtime have separate save scopes and settings fit a narrow window", async ({ page }) => {
  await page.addInitScript(installDesktopRuntime);
  await page.setViewportSize({ width: 1024, height: 768 });
  await openSettings(page);
  await page.screenshot({ path: "/tmp/bridge-settings-general.png", fullPage: true });
  await page.getByRole("button", { name: "事件与存储", exact: true }).click();
  await page.getByLabel("未确认事件保留天数").fill("14");
  await page.getByRole("button", { name: "保存更改", exact: true }).click();
  await expect(page.getByText("事件与存储已保存", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "运行", exact: true }).click();
  await page.getByLabel("默认超时秒数", { exact: true }).fill("45");
  await page.getByRole("button", { name: "保存更改", exact: true }).click();
  await expect(page.getByText("运行已保存", { exact: true })).toBeVisible();
  const writes = await page.evaluate(() => (window as any).desktopFixture.settingsWrites);
  expect(Object.keys(writes[0].changes)).toEqual(["runtime.event_delivery"]);
  expect(writes[1]).toEqual({ section: "runtime", changes: { "runtime.default_timeout_secs": 45 } });
  expect(await page.evaluate(() => (window as any).desktopFixture.config.runtime.event_delivery.retention_days)).toBe(14);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  await page.screenshot({ path: "/tmp/bridge-settings-runtime.png", fullPage: true });
});
