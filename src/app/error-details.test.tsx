// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { ErrorDetails } from "./error-details";
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
let root: ReturnType<typeof createRoot>;
afterEach(async () => { await act(async () => root.unmount()); document.body.innerHTML = ""; });
it("keeps the summary visible and expands/copies complete diagnostic text safely", async () => {
  document.body.innerHTML = '<div id="root"></div>';
  root = createRoot(document.getElementById("root")!);
  const writeText = vi.fn(async () => {});
  Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
  const error = '依赖服务暂不可用\n错误码：TEST_UNAVAILABLE\n请求 ID：request-1\n<script>unsafe</script>';
  await act(async () => root.render(<ErrorDetails error={error} />));
  expect(document.querySelector("p")?.textContent).toBe("依赖服务暂不可用");
  expect(document.querySelector("details")?.open).toBe(false);
  expect(document.querySelector("pre")?.textContent).toContain("request-1");
  expect(document.querySelector("script")).toBeNull();
  await act(async () => document.querySelector("button")!.click());
  expect(writeText).toHaveBeenCalledWith(error);
  expect(document.querySelector('[role="status"]')?.textContent).toBe("已复制");
});
