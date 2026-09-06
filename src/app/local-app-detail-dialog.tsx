import type { Dispatch, ReactNode, SetStateAction } from "react";
import { embeddedLocalAppView } from "../local-app-runtime";
import { isComputerService, isShellService } from "./config-conversion";
import { LocalAppEmbeddedUi } from "./embedded-local-app";
import { formatTime } from "./formatters";
import { ApplicationIcon, handleTabListKeyDown, InfoRow } from "./ui-primitives";
import type { LocalAppInstallTask } from "../local-app-install-tasks";
import type { LocalAppDetailTab, LocalAppItem, LocalAppKind, LocalAppLifecycle, LocalAppUpdateStatus, MarketConnector, UiAgentConfig, UiServiceConfig } from "./types";

interface LocalAppDetailDialogProps {
  activeLocalAppDetailTab: LocalAppDetailTab;
  app: LocalAppItem;
  checkLocalAppVersion: (app: LocalAppItem) => Promise<void>;
  config: UiAgentConfig | null;
  connectorBusy: string | null;
  connectorSourceKind: (app: LocalAppItem, marketApp?: MarketConnector) => string;
  connectorSyncSource: (app: LocalAppItem) => string;
  connectorUninstalling: string | null;
  connectorUpdateBusy: string | null;
  countLocalAppCapabilities: (app: LocalAppItem, config: UiAgentConfig) => number;
  formatLocalAppKind: (kind: LocalAppKind) => string;
  hasLocalAppStartCommand: (app: LocalAppItem) => boolean;
  hasLocalAppStopCommand: (app: LocalAppItem) => boolean;
  localAppLifecycle: (app: LocalAppItem) => LocalAppLifecycle;
  localAppUpdateStatus: (app: LocalAppItem) => LocalAppUpdateStatus | undefined;
  managedToolBusy: boolean;
  marketAppForLocalApp: (app: LocalAppItem) => MarketConnector | undefined;
  renderComputerPermissionPanel: (service: UiServiceConfig) => ReactNode;
  renderConnectorPermissionPanel: (connector: NonNullable<LocalAppItem["connector"]>) => ReactNode;
  renderLocalAppAbilityList: (app: LocalAppItem, canShowConfig: boolean) => ReactNode;
  renderLocalAppInstallProgress: (task: LocalAppInstallTask, compact?: boolean) => ReactNode;
  renderLocalAppRuntime: (app: LocalAppItem) => ReactNode;
  renderLocalAppUpdateChanges: (app: LocalAppItem, marketApp: MarketConnector) => ReactNode;
  renderServiceEditor: (service: UiServiceConfig, serviceIndex: number) => ReactNode;
  rollbackManagedTool: (app: LocalAppItem) => Promise<void>;
  setActiveLocalAppDetailTab: Dispatch<SetStateAction<LocalAppDetailTab>>;
  setPendingUpgradeAppId: Dispatch<SetStateAction<string | null>>;
  setSelectedLocalAppId: Dispatch<SetStateAction<string | null>>;
  showAdvancedSettings: boolean;
  startLocalApp: (app: LocalAppItem) => Promise<void>;
  stopLocalApp: (app: LocalAppItem) => Promise<void>;
  syncLocalApp: (app: LocalAppItem) => Promise<void>;
  uninstallLocalApp: (app: LocalAppItem) => Promise<void>;
  upgradeManagedTool: (app: LocalAppItem) => Promise<void>;
}

export function LocalAppDetailDialog(props: LocalAppDetailDialogProps) {
  const { activeLocalAppDetailTab, app, checkLocalAppVersion, config, connectorBusy, connectorSourceKind, connectorSyncSource, connectorUninstalling, connectorUpdateBusy, countLocalAppCapabilities, formatLocalAppKind, hasLocalAppStartCommand, hasLocalAppStopCommand, localAppLifecycle, localAppUpdateStatus, managedToolBusy, marketAppForLocalApp, renderComputerPermissionPanel, renderConnectorPermissionPanel, renderLocalAppAbilityList, renderLocalAppInstallProgress, renderLocalAppRuntime, renderLocalAppUpdateChanges, renderServiceEditor, rollbackManagedTool, setActiveLocalAppDetailTab, setPendingUpgradeAppId, setSelectedLocalAppId, showAdvancedSettings, startLocalApp, stopLocalApp, syncLocalApp, uninstallLocalApp, upgradeManagedTool } = props;
    if (!config) {
      return null;
    }
    const appComputerService = app
      .serviceIndexes.map((serviceIndex) => config.services[serviceIndex])
      .find((service): service is UiServiceConfig => Boolean(service && isComputerService(service)));
    const hasShellCapability = app.serviceIndexes.some((serviceIndex) => {
      const service = config.services[serviceIndex];
      return service ? isShellService(service) : false;
    });
    const embeddedUi = app.connector?.ui?.type === "embedded" ? app.connector.ui : null;
    const isManagedTool = app.kind === "managed_tool" && Boolean(app.managedTool);
    const canConfigureCapabilities =
      !isManagedTool &&
      app.kind !== "connector" &&
      (showAdvancedSettings || app.kind === "custom" || hasShellCapability);
    const canShowDeveloperConfig =
      !isManagedTool &&
      app.kind !== "connector" &&
      (showAdvancedSettings || app.kind === "custom");
    const marketApp = marketAppForLocalApp(app);
    const updateStatus = localAppUpdateStatus(app);
    const updateBusy = connectorUpdateBusy === app.id || (isManagedTool && managedToolBusy);
    const lifecycle = localAppLifecycle(app);
    const embeddedView = embeddedLocalAppView(
      lifecycle.state,
      connectorUninstalling === app.id,
    );
    const appIsRunning = lifecycle.state === "ready";
    const appCanStop = hasLocalAppStopCommand(app);
    const syncSource = connectorSyncSource(app);
    const iconDataUrl = app.connector?.iconDataUrl ?? marketApp?.iconDataUrl;
    const closeDetail = () => setSelectedLocalAppId(null);

    return (
      <div className="modal-backdrop app-detail-backdrop" role="presentation" onClick={closeDetail}>
        <section
          className="app-detail-dialog"
          role="dialog"
          aria-modal="true"
          onClick={(event) => event.stopPropagation()}
        >
          <div className="install-panel-head">
            <div className="app-detail-heading">
              <ApplicationIcon className="app-detail-icon" name={app.name} src={iconDataUrl} />
              <div>
                <p className="eyebrow">{formatLocalAppKind(app.kind)}</p>
                <h3>{app.name}</h3>
                <p>{app.description}</p>
              </div>
            </div>
            <button className="ghost" onClick={closeDetail}>
              关闭
            </button>
          </div>

          {app.installTask ? renderLocalAppInstallProgress(app.installTask) : null}

          <div className="app-detail-toolbar">
            <div className="section-tabs" role="tablist" aria-label={`${app.name} 应用详情`}>
              {embeddedUi && app.connector ? (
                <button
                  id="local-app-detail-app-tab"
                  className={`section-tab ${activeLocalAppDetailTab === "app" ? "active" : ""}`}
                  role="tab"
                  aria-selected={activeLocalAppDetailTab === "app"}
                  aria-controls="local-app-detail-app-panel"
                  tabIndex={activeLocalAppDetailTab === "app" ? 0 : -1}
                  onClick={() => setActiveLocalAppDetailTab("app")}
                  onKeyDown={handleTabListKeyDown}
                >
                  {embeddedUi.title?.trim() || "应用"}
                </button>
              ) : null}
              <button
                id="local-app-detail-overview-tab"
                className={`section-tab ${activeLocalAppDetailTab === "overview" ? "active" : ""}`}
                role="tab"
                aria-selected={activeLocalAppDetailTab === "overview"}
                aria-controls="local-app-detail-overview-panel"
                tabIndex={activeLocalAppDetailTab === "overview" ? 0 : -1}
                onClick={() => setActiveLocalAppDetailTab("overview")}
                onKeyDown={handleTabListKeyDown}
              >
                概览
              </button>
              {!isManagedTool && (app.kind !== "connector" || Boolean(app.connector)) ? (
                <button
                  id="local-app-detail-capabilities-tab"
                  className={`section-tab ${activeLocalAppDetailTab === "capabilities" ? "active" : ""}`}
                  role="tab"
                  aria-selected={activeLocalAppDetailTab === "capabilities"}
                  aria-controls="local-app-detail-capabilities-panel"
                  tabIndex={activeLocalAppDetailTab === "capabilities" ? 0 : -1}
                  onClick={() => setActiveLocalAppDetailTab("capabilities")}
                  onKeyDown={handleTabListKeyDown}
                >
                  能力
                </button>
              ) : null}
              {canShowDeveloperConfig && !isManagedTool ? (
                <button
                  id="local-app-detail-config-tab"
                  className={`section-tab ${activeLocalAppDetailTab === "config" ? "active" : ""}`}
                  role="tab"
                  aria-selected={activeLocalAppDetailTab === "config"}
                  aria-controls="local-app-detail-config-panel"
                  tabIndex={activeLocalAppDetailTab === "config" ? 0 : -1}
                  onClick={() => setActiveLocalAppDetailTab("config")}
                  onKeyDown={handleTabListKeyDown}
                >
                  配置
                </button>
              ) : null}
            </div>
            <div className="service-actions">
              {app.installTask && !["succeeded", "failed"].includes(app.installTask.phase) ? null : isManagedTool && app.managedTool?.state !== "ready" && !managedToolBusy ? (
                <>
                  <button
                    className="primary accent"
                    onClick={() => void upgradeManagedTool(app)}
                    disabled={!marketApp}
                  >
                    安装应用
                  </button>
                </>
              ) : marketApp ? (
                updateStatus?.updateAvailable ? (
                  <button
                    className="primary accent"
                    onClick={() => setPendingUpgradeAppId(app.id)}
                    disabled={connectorBusy != null || connectorUpdateBusy != null || managedToolBusy}
                  >
                    {updateBusy ? "升级中" : `升级到 ${updateStatus.latestVersion}`}
                  </button>
                ) : (
                  <button
                    className="secondary"
                    onClick={() => void checkLocalAppVersion(app)}
                    disabled={connectorBusy != null || connectorUpdateBusy != null || managedToolBusy}
                  >
                    {updateBusy ? "检查中" : "检查更新"}
                  </button>
                )
              ) : app.connector && app.connector.reviewStatus !== "PUBLISHED" ? (
                <button
                  className="secondary"
                  onClick={() => void syncLocalApp(app)}
                  disabled={connectorBusy != null || connectorUpdateBusy != null}
                >
                  {updateBusy ? "同步中" : "拉取最新"}
                </button>
              ) : null}
              {app.managedTool?.canRollback ? (
                <button
                  className="secondary"
                  onClick={() => void rollbackManagedTool(app)}
                  disabled={managedToolBusy}
                >
                  回滚到 {app.managedTool.previousVersion}
                </button>
              ) : null}
              {appIsRunning && appCanStop ? (
                <button
                  className="primary danger"
                  onClick={() => void stopLocalApp(app)}
                  disabled={connectorBusy != null || connectorUpdateBusy != null}
                >
                  {connectorBusy === app.id ? "停止中" : "停止应用"}
                </button>
              ) : hasLocalAppStartCommand(app) ? (
                <button
                  className="primary"
                  onClick={() => void startLocalApp(app)}
                  disabled={connectorBusy != null || connectorUpdateBusy != null}
                >
                  {connectorBusy === app.id ? "启动中" : "启动应用"}
                </button>
              ) : null}
              {app.connector ? (
                <button
                  className="ghost danger"
                  onClick={() => void uninstallLocalApp(app)}
                  disabled={connectorBusy != null || connectorUpdateBusy != null}
                >
                  卸载
                </button>
              ) : null}
            </div>
          </div>

          {activeLocalAppDetailTab === "app" && embeddedUi && app.connector && embeddedView === "mounted" ? (
            <div
              id="local-app-detail-app-panel"
              className="app-detail-tab-panel embedded-local-app-panel"
              role="tabpanel"
              aria-labelledby="local-app-detail-app-tab"
            >
              <LocalAppEmbeddedUi
                appId={app.connector.appId}
                title={embeddedUi.title?.trim() || app.name}
              />
            </div>
          ) : activeLocalAppDetailTab === "app" && embeddedView === "uninstalling" ? (
            <div id="local-app-detail-app-panel" className="app-detail-tab-panel embedded-local-app-panel" role="tabpanel" aria-labelledby="local-app-detail-app-tab">
              <div className="embedded-local-app-loading">正在关闭应用界面并准备卸载…</div>
            </div>
          ) : activeLocalAppDetailTab === "app" && embeddedUi && embeddedView === "transitioning" ? (
            <div id="local-app-detail-app-panel" className="app-detail-tab-panel embedded-local-app-panel" role="tabpanel" aria-labelledby="local-app-detail-app-tab">
              <div className="embedded-local-app-loading">{lifecycle.detail}</div>
            </div>
          ) : activeLocalAppDetailTab === "app" && embeddedUi ? (
            <div id="local-app-detail-app-panel" className="app-detail-tab-panel embedded-local-app-panel" role="tabpanel" aria-labelledby="local-app-detail-app-tab">
              <div className="empty-state">
                <strong>{lifecycle.label}</strong>
                <p>{lifecycle.detail}</p>
                {hasLocalAppStartCommand(app) ? (
                  <button
                    className="primary"
                    onClick={() => void startLocalApp(app)}
                    disabled={connectorBusy != null || connectorUpdateBusy != null}
                  >
                    {connectorBusy === app.id ? "启动中" : "启动应用"}
                  </button>
                ) : null}
              </div>
            </div>
          ) : null}

          {activeLocalAppDetailTab === "overview" ? (
            <div id="local-app-detail-overview-panel" className="app-detail-tab-panel" role="tabpanel" aria-labelledby="local-app-detail-overview-tab">
              {marketApp && updateStatus?.updateAvailable
                ? renderLocalAppUpdateChanges(app, marketApp)
                : null}
              <div className="status-detail-grid">
                <InfoRow label="类型" value={formatLocalAppKind(app.kind)} />
                <InfoRow
                  label="来源类型"
                  value={isManagedTool ? "官方独立发行" : app.connector ? connectorSourceKind(app, marketApp) : "内置"}
                />
                {app.connector ? (
                  <InfoRow
                    label="信任状态"
                    value={app.connector.reviewStatus === "PUBLISHED" ? "已注册 · 已公开审核" : "已注册 · 未公开审核"}
                  />
                ) : null}
                <InfoRow label="安装来源" value={isManagedTool ? marketApp?.source ?? "等待市场版本" : syncSource || "内置"} />
                <InfoRow label="安装位置" value={app.managedTool?.activePath ?? app.connector?.packagePath ?? "随客户端发布"} />
                <InfoRow label="版本" value={app.managedTool?.installedVersion ?? app.connector?.version ?? "随客户端发布"} />
                {app.managedTool ? (
                  <>
                    <InfoRow label="稳定命令" value={app.managedTool.launcherPath} />
                    <InfoRow label="命令 PATH" value={app.managedTool.pathConfigured ? "已配置" : "未配置"} />
                    {app.managedTool.restartRequired ? (
                      <InfoRow label="生效提示" value="请重新启动已打开的 Codex 或终端" />
                    ) : null}
                    <InfoRow label="随包基线" value={app.managedTool.bundledVersion ?? "无"} />
                    <InfoRow label="上一版本" value={app.managedTool.previousVersion ?? "无"} />
                    <InfoRow label="状态" value={app.managedTool.detail} />
                  </>
                ) : null}
                {app.connector ? (
                  <>
                    <InfoRow label="上次同步" value={formatTime(app.connector.lastSyncedAtEpochMs)} />
                    <InfoRow label="内容摘要" value={app.connector.packageChecksum ?? "旧版本未记录"} />
                    <InfoRow
                      label="启动策略"
                      value={app.connector.startPolicy === "manual" ? "用户授权后手动启动" : "自动启动"}
                    />
                    <InfoRow
                      label="进程管理"
                      value={
                        app.connector.processOwnership === "host"
                          ? "百积木宿主托管（停止时回收完整进程树）"
                          : "应用自行管理"
                      }
                    />
                  </>
                ) : null}
                {updateStatus ? (
                  <InfoRow
                    label="更新"
                    value={
                      updateStatus.updateAvailable
                        ? `可升级到 ${updateStatus.latestVersion}`
                        : `已是最新版本 ${updateStatus.currentVersion}`
                    }
                  />
                ) : null}
                {!isManagedTool ? (
                  <InfoRow label="能力数" value={String(countLocalAppCapabilities(app, config))} />
                ) : null}
              </div>
              {appComputerService ? renderComputerPermissionPanel(appComputerService) : null}
              {app.connector ? renderConnectorPermissionPanel(app.connector) : null}
              {renderLocalAppRuntime(app)}
            </div>
          ) : null}

          {activeLocalAppDetailTab === "capabilities" && !isManagedTool ? (
            <div id="local-app-detail-capabilities-panel" className="app-detail-tab-panel" role="tabpanel" aria-labelledby="local-app-detail-capabilities-tab">
              <div className="method-advanced-head">
                <strong>能力</strong>
                <small>这些能力会在授权后开放给工作区调用。</small>
              </div>
              {renderLocalAppAbilityList(app, canConfigureCapabilities)}
            </div>
          ) : null}

          {activeLocalAppDetailTab === "config" && canShowDeveloperConfig ? (
            <div id="local-app-detail-config-panel" className="app-detail-tab-panel developer-config-stack" role="tabpanel" aria-labelledby="local-app-detail-config-tab">
              <div className="method-advanced-head">
                <strong>开发者配置</strong>
                <small>内部运行项、启动命令、HTTP 绑定和 JSON 定义。</small>
              </div>
              {app.serviceIndexes.map((serviceIndex) =>
                config.services[serviceIndex] ? renderServiceEditor(config.services[serviceIndex], serviceIndex) : null
              )}
            </div>
          ) : null}
        </section>
      </div>
    );
  }
