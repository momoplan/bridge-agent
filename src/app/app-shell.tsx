import type { Dispatch, ReactNode, SetStateAction } from "react";
import { AlertTriangle } from "lucide-react";
import { ConfigLoadFailurePanel } from "../components/RecoveryUpdateCard";
import { DesktopSidebar } from "../components/DesktopShell";
import { deriveDeviceAuthorizationState } from "../device-authorization-state";
import type { StartupUpdateGateState } from "../startup-update-gate";
import { formatTime } from "./formatters";
import type { AppPage, AppUpdateStatus, AppVersionInfo, DetailPanel, RuntimeSnapshot, StartupComponentHealth, StartupHealthSnapshot, UiAgentConfig } from "./types";

interface StartupLoadingViewProps { busy: boolean; error: string; openStartupLog: () => Promise<void>; recoverInvalidConfig: () => Promise<void>; refreshAll: () => Promise<void>; renderRecoveryUpdateCard: () => ReactNode; renderStartupRecoveryPanel: () => ReactNode; renderToastStack: () => ReactNode; restartInNormalMode: () => Promise<void>; startupConfigGateReady: boolean; startupConfigMigrationFailure: string | null; startupHealth: StartupHealthSnapshot | null; startupRecoveryBusy: boolean; startupUpdateGate: StartupUpdateGateState; }
interface DesktopApplicationShellProps { activeDetailPanel: DetailPanel; activePage: AppPage; appUpdate: AppUpdateStatus | null; appVersion: AppVersionInfo | null; authorizationState: ReturnType<typeof deriveDeviceAuthorizationState>; config: UiAgentConfig; degradedStartupComponents: StartupComponentHealth[]; needsAuthorization: boolean; openStartupLog: () => Promise<void>; renderAppsPage: () => ReactNode; renderDeviceAuthorizationGate: () => ReactNode; renderDiagnosticsPage: () => ReactNode; renderForceUpdateOverlay: () => ReactNode; renderInstallLocalAppPanel: () => ReactNode; renderRuntimeConflictPanel: () => ReactNode; renderSettingsPage: () => ReactNode; renderStartupRecoveryPanel: () => ReactNode; renderToastStack: () => ReactNode; runtime: RuntimeSnapshot | null; setActiveDetailPanel: Dispatch<SetStateAction<DetailPanel>>; setActivePage: Dispatch<SetStateAction<AppPage>>; setSelectedLocalAppId: Dispatch<SetStateAction<string | null>>; startupHealth: StartupHealthSnapshot | null; statusLabel: string; }

export function StartupLoadingView(props: StartupLoadingViewProps) {
  const { busy, error, openStartupLog, recoverInvalidConfig, refreshAll, renderRecoveryUpdateCard, renderStartupRecoveryPanel, renderToastStack, restartInNormalMode, startupConfigGateReady, startupConfigMigrationFailure, startupHealth, startupRecoveryBusy, startupUpdateGate } = props;
    if (startupHealth?.safeMode && startupConfigGateReady) {
      return (
        <main className="app-shell app-loading startup-recovery-shell">
          {renderStartupRecoveryPanel()}
          {renderToastStack()}
        </main>
      );
    }
    return (
      <main className="app-shell app-loading">
        {startupUpdateGate === "update_required" ? (
          <section className="loading-panel" aria-labelledby="startup-update-required-title">
            <p className="eyebrow">百积木</p>
            <h1 id="startup-update-required-title">需要先升级客户端</h1>
            <p>配置迁移、配置读取和业务组件均未启动。请先安装官方签名更新。</p>
            {renderRecoveryUpdateCard()}
          </section>
        ) : startupConfigMigrationFailure ? (
          <section className="loading-panel" aria-labelledby="startup-migration-failed-title">
            <p className="eyebrow">百积木</p>
            <h1 id="startup-migration-failed-title">配置迁移未完成</h1>
            <p>业务配置尚未读取，Agent、Connector 和本地服务均未启动。</p>
            <div className="alert error">{startupConfigMigrationFailure}</div>
            {renderRecoveryUpdateCard()}
            <div className="loading-actions">
              <button
                className="primary"
                onClick={() => void restartInNormalMode()}
                disabled={startupRecoveryBusy}
              >
                {startupRecoveryBusy ? "正在重启" : "重启并重试迁移"}
              </button>
              <button className="secondary" onClick={() => void openStartupLog()}>
                打开启动日志
              </button>
              <button
                className="secondary danger"
                onClick={() => void recoverInvalidConfig()}
                disabled={busy}
              >
                {busy ? "恢复中" : "归档并恢复默认配置"}
              </button>
            </div>
            <p className="loading-hint">
              恢复默认配置只应在迁移无法修复时使用；操作前会先归档当前配置文件。
            </p>
          </section>
        ) : error ? (
          <ConfigLoadFailurePanel
            error={error}
            recoveryUpdateCard={renderRecoveryUpdateCard()}
            recoveryBusy={busy}
            onRetry={() => void refreshAll()}
            onRecoverDefaults={() => void recoverInvalidConfig()}
          />
        ) : (
          <section className="loading-panel">
            <p className="eyebrow">百积木</p>
            <h1>{startupUpdateGate === "checking" ? "正在检查更新" : "正在准备启动"}</h1>
            <p>
              {startupUpdateGate === "checking"
                ? "完成更新判定前，不会读取配置或启动业务组件。"
                : "等待桌面壳完成更新门禁和配置迁移。"}
            </p>
            {renderRecoveryUpdateCard()}
          </section>
        )}
      </main>
    );
}

export function DesktopApplicationShell(props: DesktopApplicationShellProps) {
  const { activeDetailPanel, activePage, appUpdate, appVersion, authorizationState, config, degradedStartupComponents, needsAuthorization, openStartupLog, renderAppsPage, renderDeviceAuthorizationGate, renderDiagnosticsPage, renderForceUpdateOverlay, renderInstallLocalAppPanel, renderRuntimeConflictPanel, renderSettingsPage, renderStartupRecoveryPanel, renderToastStack, runtime, setActiveDetailPanel, setActivePage, setSelectedLocalAppId, startupHealth, statusLabel } = props;
  const pageTitleMap: Record<AppPage, string> = {
    apps: "应用",
    diagnostics: "诊断",
    settings: "设置"
  };

  const runtimeStatusClass = runtime?.status ?? "stopped";
  const currentVersion = appVersion?.currentVersion ?? appUpdate?.currentVersion ?? "-";

  function navigateToPage(page: AppPage) {
    if (needsAuthorization && page === "settings") {
      setActivePage("apps");
      return;
    }
    setActivePage(page);
    if (page === "apps") {
      setSelectedLocalAppId(null);
    }
    if (page === "diagnostics" && activeDetailPanel === "settings") {
      setActiveDetailPanel("system");
    }
  }

  return (
    <main className="app-shell">
      <div className="desktop-shell">
        <DesktopSidebar
          activePage={activePage}
          deviceName={config.device.name}
          statusClass={runtimeStatusClass}
          statusLabel={statusLabel}
          workspace={config.platform.workspace_id}
          relay={runtime?.relay_url ?? config.relay.url}
          lastEvent={runtime ? formatTime(runtime.last_event_at) : "-"}
          version={currentVersion}
          lastError={needsAuthorization ? null : runtime?.last_error}
          authorizationState={authorizationState}
          onNavigate={navigateToPage}
        />

        <section className="main-panel">
          <div className="desktop-workspace">
            <div className="desktop-content">
              <div className="desktop-content-inner">
                <h1 className="sr-only">{pageTitleMap[activePage]}</h1>
                {!needsAuthorization ? renderRuntimeConflictPanel() : null}
                {degradedStartupComponents.length > 0 && !startupHealth?.safeMode ? (
                  <div className="alert warning startup-health-alert">
                    <span>
                      部分启动组件处于降级状态：
                      {degradedStartupComponents.map((component) => component.label).join("、")}。桌面客户端仍可使用。
                    </span>
                    <button className="ghost" onClick={() => void openStartupLog()}>
                      查看启动日志
                    </button>
                  </div>
                ) : null}
                {runtime?.last_error &&
                !needsAuthorization &&
                activePage !== "diagnostics" &&
                activePage !== "apps" ? (
                  <div className="alert warning">
                    <AlertTriangle size={16} aria-hidden="true" />
                    {runtime.last_error}
                  </div>
                ) : null}
                <div className="page-body">
                  {needsAuthorization ? (
                    activePage === "diagnostics" ? renderDiagnosticsPage() : renderDeviceAuthorizationGate()
                  ) : (
                    <>
                      {activePage === "apps" ? renderAppsPage() : null}
                      {activePage === "diagnostics" ? renderDiagnosticsPage() : null}
                      {activePage === "settings" ? renderSettingsPage() : null}
                    </>
                  )}
                </div>
              </div>
            </div>
          </div>
        </section>
      </div>
      {renderToastStack()}
      {!needsAuthorization ? renderInstallLocalAppPanel() : null}
      {renderForceUpdateOverlay()}
      {startupHealth?.safeMode ? (
        <div className="startup-recovery-overlay" role="dialog" aria-modal="true">
          {renderStartupRecoveryPanel()}
        </div>
      ) : null}
    </main>
  );
}
