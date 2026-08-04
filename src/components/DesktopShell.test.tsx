import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { DesktopSidebar } from "./DesktopShell";

function renderSidebar(workspace: string, version: string) {
  return renderToStaticMarkup(
    <DesktopSidebar
      activePage="apps"
      deviceName="测试设备"
      statusClass="online"
      statusLabel="在线"
      workspace={workspace}
      relay="wss://relay.example.com"
      lastEvent="刚刚"
      version={version}
      onNavigate={vi.fn()}
      onRefresh={vi.fn()}
    />
  );
}

describe("DesktopSidebar environment metadata", () => {
  it("keeps workspace and client version visible in the sidebar", () => {
    const markup = renderSidebar("433", "0.2.27");

    expect(markup).toContain("当前工作区 433，客户端版本 0.2.27");
    expect(markup).toContain("工作区</span><strong>#433</strong>");
    expect(markup).toContain("desktop-environment-version\">v0.2.27</span>");
  });

  it("uses explicit fallback labels instead of ambiguous punctuation", () => {
    const markup = renderSidebar("", "-");

    expect(markup).toContain("当前工作区 未授权，客户端版本 未知");
    expect(markup).toContain("desktop-environment-version\">版本未知</span>");
  });
});
