export interface ClientUpdateSummary {
  latestVersion: string | null;
  updateAvailable: boolean;
  autoDownloadAvailable: boolean;
  releaseUrl: string | null;
}

export type MarketHostUpgradeAction =
  | { kind: "install_app"; label: string; disabled: false }
  | { kind: "checking"; label: string; disabled: true }
  | { kind: "install"; label: string; disabled: false }
  | { kind: "download"; label: string; disabled: false }
  | { kind: "check"; label: string; disabled: false };

export function resolveMarketHostUpgradeAction(
  appName: string,
  compatible: boolean,
  update: ClientUpdateSummary | null,
  checking: boolean,
  installing: boolean,
  progressLabel: string
): MarketHostUpgradeAction {
  if (compatible) {
    return { kind: "install_app", label: `安装 ${appName}`, disabled: false };
  }
  if (installing) {
    return { kind: "checking", label: progressLabel, disabled: true };
  }
  if (checking) {
    return { kind: "checking", label: "正在检查客户端更新…", disabled: true };
  }
  if (update?.updateAvailable && update.autoDownloadAvailable) {
    return {
      kind: "install",
      label: update.latestVersion ? `升级客户端到 ${update.latestVersion}` : "立即升级客户端",
      disabled: false
    };
  }
  if (update?.updateAvailable && update.releaseUrl) {
    return {
      kind: "download",
      label: update.latestVersion ? `下载客户端 ${update.latestVersion}` : "下载新版客户端",
      disabled: false
    };
  }
  return {
    kind: "check",
    label: update ? "重新检查客户端更新" : "检查并升级客户端",
    disabled: false
  };
}
