import { describe, expect, it } from "vitest";
import { resolveMarketHostUpgradeAction } from "./local-app-host-upgrade";

describe("market host prerequisite upgrade action", () => {
  it("offers the signed in-app updater when a compatible client update is available", () => {
    expect(
      resolveMarketHostUpgradeAction(
        "Codex",
        false,
        {
          latestVersion: "0.2.59",
          updateAvailable: true,
          autoDownloadAvailable: true,
          releaseUrl: "https://example.test/release"
        },
        false,
        false,
        "正在更新…"
      )
    ).toEqual({ kind: "install", label: "升级客户端到 0.2.59", disabled: false });
  });

  it("falls back to the official download when this platform cannot update in place", () => {
    expect(
      resolveMarketHostUpgradeAction(
        "Codex",
        false,
        {
          latestVersion: "0.2.59",
          updateAvailable: true,
          autoDownloadAvailable: false,
          releaseUrl: "https://example.test/release"
        },
        false,
        false,
        "正在更新…"
      )
    ).toEqual({ kind: "download", label: "下载客户端 0.2.59", disabled: false });
  });

  it("keeps the prerequisite actionable before or after a failed update check", () => {
    expect(resolveMarketHostUpgradeAction("Codex", false, null, false, false, "正在更新…")).toEqual({
      kind: "check",
      label: "检查并升级客户端",
      disabled: false
    });
    expect(
      resolveMarketHostUpgradeAction(
        "Codex",
        false,
        {
          latestVersion: "0.2.58",
          updateAvailable: false,
          autoDownloadAvailable: false,
          releaseUrl: null
        },
        false,
        false,
        "正在更新…"
      )
    ).toEqual({ kind: "check", label: "重新检查客户端更新", disabled: false });
  });

  it("locks the action while checking or installing", () => {
    expect(resolveMarketHostUpgradeAction("Codex", false, null, true, false, "正在更新…")).toEqual({
      kind: "checking",
      label: "正在检查客户端更新…",
      disabled: true
    });
    expect(resolveMarketHostUpgradeAction("Codex", false, null, false, true, "已下载 72%")).toEqual({
      kind: "checking",
      label: "已下载 72%",
      disabled: true
    });
  });

  it("changes back to the app installation action after the upgraded host is compatible", () => {
    expect(resolveMarketHostUpgradeAction("Codex", true, null, false, false, "正在更新…")).toEqual({
      kind: "install_app",
      label: "安装 Codex",
      disabled: false
    });
  });
});
