import type { Dispatch, ReactNode, SetStateAction } from "react";
import { RefreshCw } from "lucide-react";
import { DEFAULT_PLATFORM_BASE_URL } from "./constants";
import { formatAppUpdateProgressButton, formatRelayRegistration, formatRelaySeen, formatTime } from "./formatters";
import { Card, InfoRow } from "./ui-primitives";
import type { AppUpdateCheckState, AppUpdateProgress, AppUpdateStatus, AppVersionInfo, DetailPanel, LogEntry, RuntimeSnapshot, SettingsSection, UiAgentConfig } from "./types";

interface DetailPanelContentProps {
  activeDetailPanel: DetailPanel; activeSettingsSection: SettingsSection; appUpdate: AppUpdateStatus | null; appUpdateCheckState: AppUpdateCheckState; appUpdateProgress: AppUpdateProgress | null; appUpdateStatusLabel: string; appUpdateTone: "normal" | "warning" | "danger"; appVersion: AppVersionInfo | null; appVersionLabel: string; beginBrowserAuth: () => Promise<void>; busy: boolean; checkAppUpdate: (showLatestMessage?: boolean) => Promise<AppUpdateStatus | null>; clearLogs: () => Promise<void>; config: UiAgentConfig | null; configPath: string; filteredLogs: LogEntry[]; installAppUpdate: (update?: AppUpdateStatus | null) => Promise<void>; logServiceFilter: string; logServiceOptions: string[]; manifestPreview: string; needsAuthorization: boolean; openAppUninstaller: () => Promise<void>; openAppUpdateReleasePage: (update: AppUpdateStatus) => Promise<void>; renderAppUpdateProgress: () => ReactNode; renderLogMetadata: (entry: LogEntry) => ReactNode; renderSettingsSection: () => ReactNode; runtime: RuntimeSnapshot | null; saveConfig: () => Promise<void>; setActiveSettingsSection: Dispatch<SetStateAction<SettingsSection>>; setLogServiceFilter: Dispatch<SetStateAction<string>>; statusLabel: string; updateBusy: boolean;
}
interface DiagnosticsPageProps { activeDetailPanel: DetailPanel; busy: boolean; refreshAll: () => Promise<void>; refreshing: boolean; renderDetailPanel: () => ReactNode; resetExampleConfig: () => Promise<void>; setActiveDetailPanel: Dispatch<SetStateAction<DetailPanel>>; }
interface SettingsPageProps { activeSettingsSection: SettingsSection; beginBrowserAuth: () => Promise<void>; busy: boolean; config: UiAgentConfig | null; refreshAll: () => Promise<void>; refreshing: boolean; renderSettingsSection: () => ReactNode; runtime: RuntimeSnapshot | null; saveConfig: () => Promise<void>; setActiveSettingsSection: Dispatch<SetStateAction<SettingsSection>>; }
interface ToastStackProps { error: string; message: string; setError: Dispatch<SetStateAction<string>>; setMessage: Dispatch<SetStateAction<string>>; }

export function DetailPanelContent(props: DetailPanelContentProps) {
  const { activeDetailPanel, activeSettingsSection, appUpdate, appUpdateCheckState, appUpdateProgress, appUpdateStatusLabel, appUpdateTone, appVersion, appVersionLabel, beginBrowserAuth, busy, checkAppUpdate, clearLogs, config, configPath, filteredLogs, installAppUpdate, logServiceFilter, logServiceOptions, manifestPreview, needsAuthorization, openAppUninstaller, openAppUpdateReleasePage, renderAppUpdateProgress, renderLogMetadata, renderSettingsSection, runtime, saveConfig, setActiveSettingsSection, setLogServiceFilter, statusLabel, updateBusy } = props;
    if (!config) {
      return <div />;
    }

    if (activeDetailPanel === "logs") {
      return (
        <Card
          title="日志"
          description="最近运行记录。"
          action={
            <div className="service-actions log-actions">
              <select value={logServiceFilter} onChange={(event) => setLogServiceFilter(event.target.value)}>
                <option value="">全部服务</option>
                {logServiceOptions.map((serviceName) => (
                  <option value={serviceName} key={serviceName}>
                    {serviceName}
                  </option>
                ))}
              </select>
              <button className="ghost" onClick={() => void clearLogs()}>
                清空日志
              </button>
            </div>
          }
        >
          <div className="log-panel">
            {filteredLogs.length === 0 ? (
              <div className="empty-state">暂无日志</div>
            ) : (
              filteredLogs.map((entry, index) => (
                <div className={`log-line log-${entry.level}`} key={`${entry.timestamp_ms}-${index}`}>
                  <span>{formatTime(entry.timestamp_ms)}</span>
                  <strong>{entry.level.toUpperCase()}</strong>
                  <div>
                    <p>{entry.message}</p>
                    {renderLogMetadata(entry)}
                  </div>
                </div>
              ))
            )}
          </div>
        </Card>
      );
    }

    if (activeDetailPanel === "manifest") {
      return (
        <Card title="对外清单" description="联调时查看。">
          <pre className="code-panel">{manifestPreview}</pre>
        </Card>
      );
    }

    if (activeDetailPanel === "settings") {
      return (
        <Card
          title="高级设置"
          description="设备身份、授权连接和本机运行参数。"
          action={
            <div className="service-actions">
              <button className="secondary" onClick={() => void saveConfig()} disabled={busy}>
                保存配置
              </button>
              <button className="secondary" onClick={() => void beginBrowserAuth()} disabled={busy}>
                浏览器授权
              </button>
            </div>
          }
        >
          <div className="status-detail-grid connection-summary-grid">
            <InfoRow label="工作区" value={config.platform.workspace_id || "未授权"} />
            <InfoRow label="平台" value={DEFAULT_PLATFORM_BASE_URL} />
            <InfoRow label="Relay" value={runtime?.relay_url ?? config.relay.url} />
          </div>
          <div className="section-tabs">
            <button
              className={`section-tab ${activeSettingsSection === "identity" ? "active" : ""}`}
              onClick={() => setActiveSettingsSection("identity")}
            >
              设备
            </button>
            <button
              className={`section-tab ${activeSettingsSection === "connection" ? "active" : ""}`}
              onClick={() => setActiveSettingsSection("connection")}
            >
              连接
            </button>
            <button
              className={`section-tab ${activeSettingsSection === "runtime" ? "active" : ""}`}
              onClick={() => setActiveSettingsSection("runtime")}
            >
              运行
            </button>
          </div>
          {renderSettingsSection()}
        </Card>
      );
    }

    const isCheckingUpdate = appUpdateCheckState === "checking";

    return (
      <Card title="系统" description="版本与运行状态。">
        <div className="app-version-panel">
          <div>
            <span>当前版本</span>
            <strong>{appVersionLabel}</strong>
            <p className={appUpdateTone === "danger" ? "danger-text" : undefined}>{appUpdateStatusLabel}</p>
          </div>
          <div className="app-version-actions">
            {appUpdate?.updateAvailable ? (
              appUpdate.autoDownloadAvailable ? (
                <button className="primary" onClick={() => void installAppUpdate()} disabled={updateBusy}>
                  {updateBusy ? formatAppUpdateProgressButton(appUpdateProgress) : `升级到 ${appUpdate.latestVersion}`}
                </button>
              ) : appUpdate.releaseUrl ? (
                <button className="primary" onClick={() => void openAppUpdateReleasePage(appUpdate)}>
                  打开下载页
                </button>
              ) : null
            ) : null}
            <button
              className="secondary"
              onClick={() => void checkAppUpdate(true)}
              disabled={updateBusy || isCheckingUpdate}
            >
              {isCheckingUpdate ? "检查中" : "检查更新"}
            </button>
            {appVersion?.currentTarget.startsWith("windows-") ? (
              <button className="secondary danger" onClick={() => void openAppUninstaller()}>
                卸载百积木
              </button>
            ) : null}
          </div>
        </div>
        {renderAppUpdateProgress()}
        <div className="status-detail-grid">
          <InfoRow label="当前状态" value={statusLabel} />
          <InfoRow
            label="Relay 注册"
            value={formatRelayRegistration(runtime)}
            tone={runtime?.relay_registered ? "normal" : "warning"}
          />
          <InfoRow label="Relay 最近响应" value={formatRelaySeen(runtime)} />
          <InfoRow label="最近事件" value={runtime ? formatTime(runtime.last_event_at) : "-"} />
          <InfoRow label="运行名称" value={runtime?.agent_id ?? config.relay.agent_id} />
          <InfoRow label="Relay" value={runtime?.relay_url ?? config.relay.url} />
          <InfoRow label="日志文件" value={runtime?.log_file_path ?? "未启用"} />
          <InfoRow label="配置文件" value={configPath} />
          <InfoRow
            label="最近错误"
            value={needsAuthorization ? "无" : runtime?.last_error || "无"}
            tone={!needsAuthorization && runtime?.last_error ? "danger" : "normal"}
          />
        </div>
      </Card>
    );
  }
export function DiagnosticsPage(props: DiagnosticsPageProps) {
  const { activeDetailPanel, busy, refreshAll, refreshing, renderDetailPanel, resetExampleConfig, setActiveDetailPanel } = props;
    return (
      <div className="diagnostics-layout">
        <div className="page-action-bar">
          <div className="section-tabs">
            <button
              className={`section-tab ${activeDetailPanel === "system" ? "active" : ""}`}
              onClick={() => setActiveDetailPanel("system")}
            >
              系统
            </button>
            <button
              className={`section-tab ${activeDetailPanel === "logs" ? "active" : ""}`}
              onClick={() => setActiveDetailPanel("logs")}
            >
              日志
            </button>
            <button
              className={`section-tab ${activeDetailPanel === "manifest" ? "active" : ""}`}
              onClick={() => setActiveDetailPanel("manifest")}
            >
              清单
            </button>
          </div>
          <div className="page-action-bar-actions">
            <button
              className="icon-button"
              onClick={() => void refreshAll()}
              disabled={refreshing}
              aria-label="刷新诊断状态"
              title="刷新"
            >
              <RefreshCw size={17} className={refreshing ? "spin" : undefined} aria-hidden="true" />
            </button>
            <button className="ghost" onClick={() => void resetExampleConfig()} disabled={busy}>
              恢复示例
            </button>
          </div>
        </div>
        {renderDetailPanel()}
      </div>
    );
  }
export function SettingsPage(props: SettingsPageProps) {
  const { activeSettingsSection, beginBrowserAuth, busy, config, refreshAll, refreshing, renderSettingsSection, runtime, saveConfig, setActiveSettingsSection } = props;
    if (!config) {
      return <div />;
    }
    return (
      <div className="settings-page">
        <Card
          title="客户端设置"
          description="管理设备身份、工作区连接和本机运行参数。"
          action={
            <div className="service-actions">
              <button
                className="icon-button"
                onClick={() => void refreshAll()}
                disabled={refreshing}
                aria-label="刷新设置状态"
                title="刷新"
              >
                <RefreshCw size={17} className={refreshing ? "spin" : undefined} aria-hidden="true" />
              </button>
              <button className="secondary" onClick={() => void beginBrowserAuth()} disabled={busy}>
                浏览器授权
              </button>
              <button className="primary" onClick={() => void saveConfig()} disabled={busy}>
                {busy ? "保存中" : "保存配置"}
              </button>
            </div>
          }
        >
          <div className="status-detail-grid connection-summary-grid">
            <InfoRow label="工作区" value={config.platform.workspace_id || "未授权"} />
            <InfoRow label="平台" value={DEFAULT_PLATFORM_BASE_URL} />
            <InfoRow label="Relay" value={runtime?.relay_url ?? config.relay.url} />
          </div>
          <div className="section-tabs">
            <button
              className={`section-tab ${activeSettingsSection === "identity" ? "active" : ""}`}
              onClick={() => setActiveSettingsSection("identity")}
            >
              设备
            </button>
            <button
              className={`section-tab ${activeSettingsSection === "connection" ? "active" : ""}`}
              onClick={() => setActiveSettingsSection("connection")}
            >
              连接
            </button>
            <button
              className={`section-tab ${activeSettingsSection === "runtime" ? "active" : ""}`}
              onClick={() => setActiveSettingsSection("runtime")}
            >
              运行
            </button>
          </div>
          {renderSettingsSection()}
        </Card>
      </div>
    );
  }
export function ToastStack(props: ToastStackProps) {
  const { error, message, setError, setMessage } = props;
    if (!message && !error) {
      return null;
    }

    return (
      <div className="toast-stack" aria-live="polite" aria-atomic="true">
        {message ? (
          <div className="toast toast-success" role="status">
            <div>
              <strong>已完成</strong>
              <p>{message}</p>
            </div>
            <button className="toast-close" onClick={() => setMessage("")} aria-label="关闭成功提示">
              ×
            </button>
          </div>
        ) : null}
        {error ? (
          <div className="toast toast-error" role="alert">
            <div>
              <strong>操作失败</strong>
              <p>{error}</p>
            </div>
            <button className="toast-close" onClick={() => setError("")} aria-label="关闭错误提示">
              ×
            </button>
          </div>
        ) : null}
      </div>
    );
  }
