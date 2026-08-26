import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { ConfigLoadFailurePanel, RecoveryUpdateCard } from "./RecoveryUpdateCard";

const baseProps = {
  checkState: "ready" as const,
  checkError: null,
  currentVersion: "0.6.3",
  targetVersion: "0.6.4",
  updateAvailable: true,
  autoDownloadAvailable: true,
  releaseUrl: "https://example.invalid/release",
  updateBusy: false,
  updateBusyLabel: "正在安装",
  onInstall: vi.fn(),
  onOpenRelease: vi.fn(),
  onCheck: vi.fn()
};

describe("RecoveryUpdateCard", () => {
  it("offers the signed updater without requiring a loaded business config", () => {
    const markup = renderToStaticMarkup(<RecoveryUpdateCard {...baseProps} />);

    expect(markup).toContain("安装 0.6.4（保留配置）");
    expect(markup).toContain("现有配置会保留并由新版本迁移");
    expect(markup).toContain("重新检查");
  });

  it("keeps the independent update check available when metadata loading fails", () => {
    const markup = renderToStaticMarkup(
      <RecoveryUpdateCard
        {...baseProps}
        checkState="error"
        checkError="network unavailable"
        updateAvailable={false}
        autoDownloadAvailable={false}
        targetVersion={null}
      />
    );

    expect(markup).toContain("检查失败：network unavailable");
    expect(markup).toContain("重新检查");
    expect(markup).not.toContain("恢复默认配置");
  });

  it("places the independent updater before destructive config recovery", () => {
    const markup = renderToStaticMarkup(
      <ConfigLoadFailurePanel
        error="failed to parse config"
        recoveryUpdateCard={<RecoveryUpdateCard {...baseProps} />}
        recoveryBusy={false}
        onRetry={vi.fn()}
        onRecoverDefaults={vi.fn()}
      />
    );

    expect(markup).toContain("配置读取失败不会阻止官方更新");
    expect(markup).toContain("安装 0.6.4（保留配置）");
    expect(markup.indexOf("安装 0.6.4（保留配置）")).toBeLessThan(
      markup.indexOf("归档并恢复默认配置")
    );
  });
});
