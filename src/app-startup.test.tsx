// @vitest-environment jsdom
import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import startupHtml from "../index.html?raw";
import { installDesktopRuntime } from "../tests/desktop-runtime";
import { startRecoveryShell } from "./recovery-shell";
import { mountBusiness } from "./business-entry";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
let stop: (() => void) | undefined;
let shell: ReturnType<typeof startRecoveryShell> | undefined;
afterEach(async () => {
  await act(async () => { stop?.(); });
  shell?.dispose();
  vi.restoreAllMocks();
});

async function start(options: Parameters<typeof installDesktopRuntime>[0] = {}) {
  document.documentElement.innerHTML = startupHtml;
  const fixture = installDesktopRuntime(options);
  shell = startRecoveryShell(document);
  await act(async () => {
    stop = mountBusiness(document.getElementById("root")!, shell!.ready, shell!.fail);
  });
  return fixture;
}

describe("complete desktop startup and independent recovery", () => {
  it("loads nonempty business config, built-ins, installed apps and managed tools", async () => {
    const fixture = await start();
    const root = document.getElementById("root")!;
    expect(root.textContent).toContain("测试应用");
    expect(root.textContent).toContain("测试 CLI");
    expect(root.textContent).toContain("桌面控制");
    expect(root.textContent).toContain("Shell");
    expect(document.getElementById("desktop-recovery")!.hidden).toBe(true);
    expect(fixture.calls.indexOf("mark_frontend_ready")).toBeGreaterThan(fixture.calls.indexOf("load_config"));
  });
  it("keeps an unregistered device usable for authorization", async () => {
    await start({ authorized: false });
    expect(document.getElementById("root")!.textContent).toContain("设备尚未授权");
  });
  it("continues to render with DNS failure and nonempty config", async () => {
    await start({ dnsError: true });
    expect(document.getElementById("root")!.textContent).toContain("测试应用");
    expect(document.getElementById("root")!.hidden).toBe(false);
  });
  it("retains update actions when configuration cannot load", async () => {
    const fixture = await start({ configError: true });
    expect(document.getElementById("root")!.textContent).toContain("配置读取失败");
    expect(fixture.calls).not.toContain("mark_frontend_ready");
  });
  it("catches render failure and installs updates without loading business state", async () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    const fixture = await start({ crash: true });
    expect(document.getElementById("desktop-recovery")!.hidden).toBe(false);
    expect(document.getElementById("recovery-title")!.textContent).toContain("无法显示");
    expect(fixture.calls).toContain("report_frontend_failure");
    expect(fixture.calls).not.toContain("mark_frontend_ready");
    await act(async () => { document.getElementById("recovery-install")!.click(); });
    expect(fixture.calls).toContain("install_app_update");
    expect(document.getElementById("recovery-status")!.textContent).toContain("更新已安装");
  });
  it("keeps recovery visible and retryable after an update-network error", async () => {
    const fixture = await start({ dnsError: true });
    shell!.fail(new Error("injected runtime failure"));
    await act(async () => { document.getElementById("recovery-check")!.click(); });
    expect(document.getElementById("recovery-status")!.textContent).toContain("dns error");
    fixture.options.dnsError = false;
    await act(async () => { document.getElementById("recovery-check")!.click(); });
    expect(document.getElementById("recovery-status")!.textContent).toContain("发现新版本");
    expect(document.getElementById("desktop-recovery")!.hidden).toBe(false);
  });
});
