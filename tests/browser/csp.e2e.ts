import { readFileSync } from "node:fs";
import { test, expect } from "@playwright/test";
import { installDesktopRuntime } from "../desktop-runtime";

const config = JSON.parse(readFileSync(new URL("../../src-tauri/tauri.conf.json", import.meta.url), "utf8"));

test("production CSP allows bundled module preload on older WebViews", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.route("**/", async (route) => {
    const response = await route.fetch();
    await route.fulfill({ response, headers: { ...response.headers(), "content-security-policy": config.app.security.csp } });
  });
  await page.addInitScript(installDesktopRuntime);
  await page.addInitScript(() => {
    const supports = DOMTokenList.prototype.supports;
    DOMTokenList.prototype.supports = function (token) {
      return token === "modulepreload" ? false : supports.call(this, token);
    };
    Object.assign(window, { cspViolations: [] });
    document.addEventListener("securitypolicyviolation", (event) => {
      (window as unknown as { cspViolations: string[] }).cspViolations.push(`${event.effectiveDirective}: ${event.blockedURI}`);
    });
  });
  await page.goto("/");
  await expect(page.getByText("测试应用", { exact: true })).toBeVisible();
  await expect(page.locator("#desktop-recovery")).toBeHidden();
  expect(await page.evaluate(() => (window as unknown as { cspViolations: string[] }).cspViolations)).toEqual([]);
  expect(errors).toEqual([]);
});
