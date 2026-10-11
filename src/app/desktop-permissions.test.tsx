import { describe, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { formatDesktopPermissionValue } from "./formatters";
import { ComputerPermissionPanel } from "./service-status-panels";
import type { DesktopPermissionStatus, UiServiceConfig } from "./types";

const denied: DesktopPermissionStatus = {
  platform: "macos",
  accessibilityGranted: false,
  accessibilitySupported: true,
  screenRecordingGranted: false,
  screenRecordingSupported: true,
};

function renderPermissions(status: DesktopPermissionStatus) {
  const service: UiServiceConfig = {
    name: "computer", description: "桌面控制", enabled: true, methods: [],
    health_check: null, start_command: null, stop_command: null,
  };
  return renderToStaticMarkup(<ComputerPermissionPanel
    service={service}
    desktopPermissions={status}
    desktopPermissionBusy={null}
    requestDesktopPermission={vi.fn()}
    openDesktopPermissionSettings={vi.fn()}
  />);
}

describe("macOS permission compatibility", () => {
  it("keeps missing detection distinct from permission denial", () => {
    const status = { ...denied, screenRecordingSupported: false };
    expect(formatDesktopPermissionValue(status, "screen_recording", "用于截图")).toContain("无法自动检测");
    expect(formatDesktopPermissionValue(denied, "screen_recording", "用于截图")).toBe("未授权");
    expect(formatDesktopPermissionValue({ ...denied, screenRecordingGranted: true }, "screen_recording", "用于截图")).toBe("已授权");
  });

  it("keeps manual settings and accessibility authorization on older macOS", () => {
    const html = renderPermissions({ ...denied, screenRecordingSupported: false });
    expect(html).toContain("无法自动检测");
    expect(html).toContain("打开屏幕录制设置");
    expect(html).toContain("请求辅助功能");
    expect(html).not.toContain("请求屏幕录制");
  });

  it("retains screen permission requests when the system API is available", () => {
    expect(renderPermissions(denied)).toContain("请求屏幕录制");
  });
});
