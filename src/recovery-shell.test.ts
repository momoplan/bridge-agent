// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import startupHtml from "../index.html?raw";
import { installDesktopRuntime } from "../tests/desktop-runtime";
import { startRecoveryShell } from "./recovery-shell";

let shell: ReturnType<typeof startRecoveryShell>;
let fixture: ReturnType<typeof installDesktopRuntime>;
beforeEach(() => {
  vi.useFakeTimers();
  document.documentElement.innerHTML = startupHtml;
  fixture = installDesktopRuntime();
  shell = startRecoveryShell(document);
});
afterEach(() => { shell.dispose(); vi.useRealTimers(); });

it("reports a stalled business bootstrap without removing recovery controls", async () => {
  await vi.advanceTimersByTimeAsync(20000);
  expect(document.getElementById("recovery-detail")!.textContent).toContain("启动超时");
  expect(document.getElementById("desktop-recovery")!.hidden).toBe(false);
  expect(fixture.calls).toContain("report_frontend_failure");
  shell.ready();
  expect(document.getElementById("desktop-recovery")!.hidden).toBe(false);
});

it("detects a disappeared business root while the recovery event loop is alive", async () => {
  document.getElementById("root")!.innerHTML = "<div>rendered</div>";
  shell.ready();
  await vi.advanceTimersByTimeAsync(3000);
  expect(document.getElementById("desktop-recovery")!.hidden).toBe(true);
  document.getElementById("root")!.replaceChildren();
  await vi.advanceTimersByTimeAsync(3000);
  expect(document.getElementById("desktop-recovery")!.hidden).toBe(false);
  expect(document.getElementById("recovery-detail")!.textContent).toContain("意外退出");
});

it("allows retry after a failed signed installation without erasing the error", async () => {
  shell.fail(new Error("render failure"));
  fixture.options.installError = true;
  const install = document.getElementById("recovery-install") as HTMLButtonElement;
  install.click();
  expect(install.disabled).toBe(true);
  await vi.advanceTimersByTimeAsync(0);
  expect(install.disabled).toBe(false);
  expect(document.getElementById("recovery-status")!.textContent).toContain("signature verification failed");
  fixture.options.installError = false;
  install.click();
  await vi.advanceTimersByTimeAsync(0);
  expect(document.getElementById("recovery-status")!.textContent).toContain("更新已安装");
  expect(document.getElementById("desktop-recovery")!.hidden).toBe(false);
});

it("renders fatal error payloads as text and ignores failure after disposal", () => {
  shell.fail(new Error("<img src=x onerror=alert(1)>"));
  const detail = document.getElementById("recovery-detail")!;
  expect(detail.querySelector("img")).toBeNull();
  expect(detail.textContent).toContain("<img");
  shell.dispose();
  shell.fail(new Error("after disposal"));
  expect(detail.textContent).not.toContain("after disposal");
});

it("does not replace a fatal error with a later startup timeout", async () => {
  shell.fail(new Error("business module failed"));
  await vi.advanceTimersByTimeAsync(21000);
  expect(document.getElementById("recovery-detail")!.textContent).toBe("business module failed");
  expect(fixture.calls.filter((call) => call === "report_frontend_failure")).toHaveLength(1);
});

it("keeps a mounted desktop visible after unrelated global errors before and after readiness", async () => {
  const notify = () => {
    window.dispatchEvent(new ErrorEvent("error", { message: "optional resource error" }));
    const rejection = new Event("unhandledrejection");
    Object.assign(rejection, { reason: new Error("optional request failed") });
    window.dispatchEvent(rejection);
  };
  notify();
  document.getElementById("root")!.innerHTML = "<div>rendered</div>";
  shell.ready();
  notify();
  await vi.advanceTimersByTimeAsync(21000);
  expect(document.getElementById("desktop-recovery")!.hidden).toBe(true);
  expect(document.getElementById("root")!.hidden).toBe(false);
  expect(fixture.calls).not.toContain("report_frontend_failure");
});
