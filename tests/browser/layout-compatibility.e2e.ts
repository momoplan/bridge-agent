import { test, expect } from "@playwright/test";
import { installDesktopRuntime } from "../desktop-runtime";

test("Flex spacing works when Grid gap is supported but Flex gap is absent", async ({ page }) => {
  await page.addInitScript(installDesktopRuntime);
  await page.addInitScript(() => {
    const scrollHeight = Object.getOwnPropertyDescriptor(Element.prototype, "scrollHeight")!;
    Object.defineProperty(Element.prototype, "scrollHeight", { ...scrollHeight, get() {
      if (this instanceof HTMLElement && this.style.rowGap === "1px" && this.style.visibility === "hidden") return 2;
      return scrollHeight.get!.call(this);
    } });
  });
  await page.goto("/");
  await expect(page.getByText("测试应用", { exact: true })).toBeVisible();
  expect(await page.evaluate(() => CSS.supports("gap", "1px"))).toBe(true);
  await expect(page.locator("html")).toHaveClass(/flex-gap-not-supported/);
  const spacing = await page.evaluate(() => {
    const fixture = document.createElement("section");
    fixture.style.cssText = "position:fixed;top:100px;left:400px;width:190px;background:white;z-index:9999;";
    fixture.innerHTML = '<div class="loading-actions"><button style="width:80px;height:20px;padding:0">A</button><button style="width:80px;height:20px;padding:0">B</button><button style="width:80px;height:20px;padding:0">C</button></div><div class="python-runtime-fields"><span>A</span><span>B</span></div><div class="button-with-icon"><svg width="15" height="15"></svg><span>label</span></div><div class="startup-component-list" id="grid-spacing"><span>A</span><span>B</span></div><div class="loading-actions"><div class="startup-component-list" id="nested-grid"><span>A</span><span>B</span></div></div><div class="startup-component"><span>Title</span><span>Status</span></div>';
    document.body.appendChild(fixture);
    const children = (selector: string) => Array.from(fixture.querySelector(selector)!.children, (child) => child.getBoundingClientRect());
    const row = children(".loading-actions"), column = children(".python-runtime-fields"), icon = children(".button-with-icon"), grid = children("#grid-spacing"), nested = children("#nested-grid");
    return { row: row[1].left - row[0].right, wrap: row[2].top - row[0].bottom, column: column[1].top - column[0].bottom, icon: icon[1].left - icon[0].right, grid: grid[1].top - grid[0].bottom, nestedGrid: nested[1].top - nested[0].bottom, background: fixture.querySelector(".startup-component")!.getBoundingClientRect().left - fixture.getBoundingClientRect().left };
  });
  expect(spacing).toEqual({ row: 10, wrap: 10, column: 10, icon: 7, grid: 10, nestedGrid: 10, background: 0 });
});

test("modern Flex layouts keep native spacing and keyboard focus", async ({ page }) => {
  await page.addInitScript(installDesktopRuntime);
  await page.goto("/");
  await expect(page.getByText("测试应用", { exact: true })).toBeVisible();
  await expect(page.locator("html")).not.toHaveClass(/flex-gap-not-supported/);
  const button = page.getByRole("button", { name: "安装应用", exact: true });
  await button.focus();
  const values = await button.evaluate((element) => {
    const [icon, label] = Array.from(element.children, (child) => child.getBoundingClientRect());
    return { gap: label.left - icon.right, focus: getComputedStyle(element).boxShadow };
  });
  expect(values.gap).toBe(7);
  expect(values.focus).not.toBe("none");
});
