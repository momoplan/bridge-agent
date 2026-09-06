import type { ReactNode } from "react";
import { RecoveryUpdateCard } from "../components/RecoveryUpdateCard";
import { formatAppUpdateProgressButton, formatStartupComponentStatus } from "./formatters";
import { InfoRow } from "./ui-primitives";
import type { AppUpdateProgress, AppUpdateStatus, AppUpdateCheckState, AppVersionInfo, StartupHealthSnapshot } from "./types";

interface RecoveryUpdateStatusCardProps {
  appUpdate: AppUpdateStatus | null; appUpdateCheckState: AppUpdateCheckState; appUpdateError: string | null; appUpdateProgress: AppUpdateProgress | null; appVersion: AppVersionInfo | null;
  checkStartupAppUpdate: (showLatestMessage?: boolean) => Promise<AppUpdateStatus | null>;
  installAppUpdate: (update?: AppUpdateStatus | null) => Promise<void>;
  openAppUpdateReleasePage: (update: AppUpdateStatus) => Promise<void>;
  renderAppUpdateProgress: () => ReactNode; updateBusy: boolean;
}
interface StartupRecoveryPanelProps {
  busy: boolean; openStartupLog: () => Promise<void>; recoverInvalidConfig: () => Promise<void>; renderRecoveryUpdateCard: () => ReactNode; restartInNormalMode: () => Promise<void>; startupHealth: StartupHealthSnapshot | null; startupRecoveryBusy: boolean;
}
interface ForceUpdateOverlayProps {
  appUpdate: AppUpdateStatus | null; appUpdateCheckState: AppUpdateCheckState; appUpdateProgress: AppUpdateProgress | null; checkAppUpdate: (showLatestMessage?: boolean) => Promise<AppUpdateStatus | null>; installAppUpdate: (update?: AppUpdateStatus | null) => Promise<void>; openAppUpdateReleasePage: (update: AppUpdateStatus) => Promise<void>; renderAppUpdateProgress: () => ReactNode; updateBusy: boolean;
}

export function RecoveryUpdateStatusCard(props: RecoveryUpdateStatusCardProps) {
  const { appUpdate, appUpdateCheckState, appUpdateError, appUpdateProgress, appVersion, checkStartupAppUpdate, installAppUpdate, openAppUpdateReleasePage, renderAppUpdateProgress, updateBusy } = props;
    return (
      <RecoveryUpdateCard
        checkState={appUpdateCheckState}
        checkError={appUpdateError}
        currentVersion={appVersion?.currentVersion ?? appUpdate?.currentVersion ?? null}
        targetVersion={appUpdate?.latestVersion ?? appUpdate?.minimumSupportedVersion ?? null}
        updateAvailable={appUpdate?.updateAvailable === true}
        autoDownloadAvailable={appUpdate?.autoDownloadAvailable === true}
        releaseUrl={appUpdate?.releaseUrl ?? null}
        updateBusy={updateBusy}
        updateBusyLabel={formatAppUpdateProgressButton(appUpdateProgress)}
        progress={renderAppUpdateProgress()}
        onInstall={() => void installAppUpdate()}
        onOpenRelease={() => {
          if (appUpdate) {
            void openAppUpdateReleasePage(appUpdate);
          }
        }}
        onCheck={() => void checkStartupAppUpdate(true)}
      />
    );
  }
export function StartupRecoveryPanel(props: StartupRecoveryPanelProps) {
  const { busy, openStartupLog, recoverInvalidConfig, renderRecoveryUpdateCard, restartInNormalMode, startupHealth, startupRecoveryBusy } = props;
    if (!startupHealth?.safeMode) {
      return null;
    }
    return (
      <section className="startup-recovery-panel" aria-labelledby="startup-recovery-title">
        <div className="startup-recovery-heading">
          <div>
            <p className="eyebrow">安全模式</p>
            <h1 id="startup-recovery-title">桌面基础壳已启动</h1>
            <p>
              业务组件本次未自动启动，你仍然可以检查并安装签名更新、查看启动日志，或修复配置后重启。
            </p>
          </div>
          <span className="status-pill status-backoff">受限运行</span>
        </div>

        <div className="startup-recovery-reason alert warning">
          {startupHealth.forcedSafeMode
            ? "当前进程通过 --safe-mode 明确启动。移除该启动参数后，才能恢复普通模式。"
            : startupHealth.consecutiveFailures > 0
              ? `检测到连续 ${startupHealth.consecutiveFailures} 次启动未完成，已停止自动拉起业务组件。`
              : "启动前置条件异常，已停止自动拉起业务组件，避免客户端反复退出。"}
        </div>

        <div className="startup-component-list">
          {startupHealth.components.map((component) => (
            <div className="startup-component" key={component.id}>
              <div>
                <strong>{component.label}</strong>
                {component.detail ? <p>{component.detail}</p> : null}
              </div>
              <span className={`startup-component-status status-${component.status}`}>
                {formatStartupComponentStatus(component.status)}
              </span>
            </div>
          ))}
        </div>

        {renderRecoveryUpdateCard()}

        <div className="startup-recovery-actions">
          <button
            className="primary"
            onClick={() => void restartInNormalMode()}
            disabled={startupRecoveryBusy || startupHealth.forcedSafeMode}
          >
            {startupRecoveryBusy ? "正在重启" : "退出安全模式并重启"}
          </button>
          <button className="secondary" onClick={() => void openStartupLog()}>
            打开启动日志
          </button>
          <button className="secondary" onClick={() => void recoverInvalidConfig()} disabled={busy}>
            {busy ? "恢复中" : "归档并恢复默认配置"}
          </button>
        </div>
        <p className="startup-log-path">启动日志：{startupHealth.startupLogPath}</p>
      </section>
    );
  }

export function ForceUpdateOverlay(props: ForceUpdateOverlayProps) {
  const { appUpdate, appUpdateCheckState, appUpdateProgress, checkAppUpdate, installAppUpdate, openAppUpdateReleasePage, renderAppUpdateProgress, updateBusy } = props;
    if (!appUpdate?.forceUpdateRequired) {
      return null;
    }
    const targetVersion = appUpdate.latestVersion ?? appUpdate.minimumSupportedVersion ?? "最新版本";
    const message =
      appUpdate.forceUpdateMessage ||
      `当前版本 ${appUpdate.currentVersion} 已停止支持，需要升级到 ${targetVersion} 后继续使用。`;

    return (
      <div className="force-update-overlay" role="dialog" aria-modal="true" aria-labelledby="force-update-title">
        <section className="force-update-panel">
          <p className="eyebrow">必须更新</p>
          <h2 id="force-update-title">请升级百积木本地连接客户端</h2>
          <p>{message}</p>
          <div className="force-update-meta">
            <InfoRow label="当前版本" value={appUpdate.currentVersion} tone="warning" />
            <InfoRow label="目标版本" value={targetVersion} />
          </div>
          <div className="force-update-actions">
            {appUpdate.autoDownloadAvailable ? (
              <button className="primary danger" onClick={() => void installAppUpdate()} disabled={updateBusy}>
                {updateBusy ? formatAppUpdateProgressButton(appUpdateProgress) : "立即更新"}
              </button>
            ) : appUpdate.releaseUrl ? (
              <button className="primary danger" onClick={() => void openAppUpdateReleasePage(appUpdate)}>
                打开下载页
              </button>
            ) : (
              <span className="danger-text">更新服务未提供可用安装包或下载页。</span>
            )}
            <button
              className="secondary"
              onClick={() => void checkAppUpdate(true)}
              disabled={updateBusy || appUpdateCheckState === "checking"}
            >
              {appUpdateCheckState === "checking" ? "检查中" : "重新检查"}
            </button>
          </div>
          {renderAppUpdateProgress()}
        </section>
      </div>
    );
  }
