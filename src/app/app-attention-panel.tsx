import type { Dispatch, SetStateAction } from "react";
import { Terminal } from "lucide-react";
import { isComputerService } from "./config-conversion";
import { formatAppUpdateProgressButton } from "./formatters";
import type { AppPage, AppUpdateProgress, AppUpdateStatus, LocalAppDetailTab, RuntimeSnapshot, UiAgentConfig } from "./types";

interface AppAttentionPanelProps { appUpdate: AppUpdateStatus | null; appUpdateProgress: AppUpdateProgress | null; busy: boolean; config: UiAgentConfig | null; hasDesktopPermissionGap: boolean; installAppUpdate: (update?: AppUpdateStatus | null) => Promise<void>; openAppUpdateReleasePage: (update: AppUpdateStatus) => Promise<void>; runtime: RuntimeSnapshot | null; runtimeCanStop: boolean; setActiveLocalAppDetailTab: Dispatch<SetStateAction<LocalAppDetailTab>>; setActivePage: Dispatch<SetStateAction<AppPage>>; setExpandedServiceIndex: Dispatch<SetStateAction<number | null>>; setSelectedLocalAppId: Dispatch<SetStateAction<string | null>>; startActionLabel: string; startActionLocked: boolean; startAgent: () => Promise<void>; statusLabel: string; stopAgent: () => Promise<void>; updateBusy: boolean; }

export function AppAttentionPanel(props: AppAttentionPanelProps) {
  const { appUpdate, appUpdateProgress, busy, config, hasDesktopPermissionGap, installAppUpdate, openAppUpdateReleasePage, runtime, runtimeCanStop, setActiveLocalAppDetailTab, setActivePage, setExpandedServiceIndex, setSelectedLocalAppId, startActionLabel, startActionLocked, startAgent, statusLabel, stopAgent, updateBusy } = props;
    if (!config) {
      return null;
    }
    const actionableRuntime =
      runtime != null &&
      (runtime.status !== "online" || !runtime.relay_registered)
        ? runtime
        : null;
    const hasClientUpdate = appUpdate?.updateAvailable === true && !appUpdate.forceUpdateRequired;

    if (!actionableRuntime && !hasDesktopPermissionGap && !hasClientUpdate) {
      return null;
    }

    return (
      <div className="app-attention-stack" aria-label="需要处理">
        {actionableRuntime ? (
          <div className="notice-banner warning" role="status">
            <div>
              <strong>{actionableRuntime.last_error ? "Agent 连接异常" : `Agent ${statusLabel}`}</strong>
              <span>
                {actionableRuntime.last_error ||
                  (actionableRuntime.status === "stopped"
                    ? "启动 Agent 后，工作区才能调用这些本地应用与能力。"
                    : "连接尚未完成，可以稍后刷新或前往诊断页查看详情。")}
              </span>
            </div>
            <div className="notice-banner-actions">
              <button
                className="primary"
                onClick={() => void startAgent()}
                disabled={busy || startActionLocked}
              >
                {startActionLabel}
              </button>
              {runtimeCanStop ? (
                <button className="secondary" onClick={() => void stopAgent()} disabled={busy}>
                  停止
                </button>
              ) : null}
              <button
                className="ghost button-with-icon"
                onClick={() => setActivePage("diagnostics")}
              >
                <Terminal size={15} aria-hidden="true" />
                查看诊断
              </button>
            </div>
          </div>
        ) : null}

        {hasDesktopPermissionGap ? (
          <div className="notice-banner warning" role="status">
            <div>
              <strong>桌面控制需要系统权限</strong>
              <span>授予屏幕录制和辅助功能权限后，桌面控制能力才能正常工作。</span>
            </div>
            <button
              className="secondary"
              onClick={() => {
                const computerIndex = config.services.findIndex(isComputerService);
                if (computerIndex >= 0) {
                  setExpandedServiceIndex(computerIndex);
                  setSelectedLocalAppId("built-in:desktop-control");
                  setActiveLocalAppDetailTab("overview");
                }
              }}
            >
              打开桌面控制
            </button>
          </div>
        ) : null}

        {hasClientUpdate && appUpdate ? (
          <div className="notice-banner warning" role="status">
            <div>
              <strong>百积木客户端可升级到 {appUpdate.latestVersion}</strong>
              <span>更新包含新的桌面体验和客户端改进。</span>
            </div>
            {appUpdate.autoDownloadAvailable ? (
              <button className="secondary" onClick={() => void installAppUpdate()} disabled={updateBusy}>
                {updateBusy ? formatAppUpdateProgressButton(appUpdateProgress) : "安装更新"}
              </button>
            ) : appUpdate.releaseUrl ? (
              <button className="secondary" onClick={() => void openAppUpdateReleasePage(appUpdate)}>
                打开下载页
              </button>
            ) : null}
          </div>
        ) : null}
      </div>
    );
  }
