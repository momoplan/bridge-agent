import type { Dispatch, ReactNode, SetStateAction } from "react";
import { ExternalLink, Plus, RefreshCw, Search } from "lucide-react";
import { defaultLocalAppDetailTab } from "./embedded-local-app";
import { formatLocalAppInstallTaskPhase, type LocalAppInstallTask } from "../local-app-install-tasks";
import { ApplicationIcon, Card } from "./ui-primitives";
import type { InstallSourceMode, LocalAppDetailTab, LocalAppItem, LocalAppKind, LocalAppLifecycle, LocalAppUpdateStatus, MarketConnector, UiAgentConfig } from "./types";

interface LocalAppListPanelProps {
  availableLocalAppUpdates: Array<{ app: LocalAppItem; status: LocalAppUpdateStatus }>; busy: boolean; config: UiAgentConfig | null; formatLocalAppKind: (kind: LocalAppKind) => string; localAppKindFilter: LocalAppKind | "all"; localAppQuery: string; localApps: LocalAppItem[]; needsAuthorization: boolean; openConsole: () => Promise<void>; refreshAll: () => Promise<void>; refreshing: boolean; refreshMarketConnectorApps: () => Promise<MarketConnector[]>; renderAppAttentionPanel: () => ReactNode; renderLocalAppCard: (app: LocalAppItem) => ReactNode; setCustomInstallConfirmed: Dispatch<SetStateAction<boolean>>; setInstallPanelOpen: Dispatch<SetStateAction<boolean>>; setInstallSourceMode: Dispatch<SetStateAction<InstallSourceMode>>; setLocalAppKindFilter: Dispatch<SetStateAction<LocalAppKind | "all">>; setLocalAppQuery: Dispatch<SetStateAction<string>>; setMarketAppQuery: Dispatch<SetStateAction<string>>; setSelectedLocalAppId: Dispatch<SetStateAction<string | null>>;
}
interface LocalAppCardProps {
  app: LocalAppItem; config: UiAgentConfig | null; countLocalAppCapabilities: (app: LocalAppItem, config: UiAgentConfig) => number; formatLocalAppKind: (kind: LocalAppKind) => string; hasLocalAppStartCommand: (app: LocalAppItem) => boolean; localAppLifecycle: (app: LocalAppItem) => LocalAppLifecycle; localAppUpdateStatus: (app: LocalAppItem) => LocalAppUpdateStatus | undefined; marketAppForLocalApp: (app: LocalAppItem) => MarketConnector | undefined; renderLocalAppInstallProgress: (task: LocalAppInstallTask, compact?: boolean) => ReactNode; setActiveLocalAppDetailTab: Dispatch<SetStateAction<LocalAppDetailTab>>; setExpandedServiceIndex: Dispatch<SetStateAction<number | null>>; setSelectedLocalAppId: Dispatch<SetStateAction<string | null>>;
}

export function LocalAppListPanel(props: LocalAppListPanelProps) {
  const { availableLocalAppUpdates, busy, config, formatLocalAppKind, localAppKindFilter, localAppQuery, localApps, needsAuthorization, openConsole, refreshAll, refreshing, refreshMarketConnectorApps, renderAppAttentionPanel, renderLocalAppCard, setCustomInstallConfirmed, setInstallPanelOpen, setInstallSourceMode, setLocalAppKindFilter, setLocalAppQuery, setMarketAppQuery, setSelectedLocalAppId } = props;
    if (!config) {
      return <div />;
    }
    const normalizedQuery = localAppQuery.trim().toLocaleLowerCase();
    const visibleApps = localApps.filter((app) => {
      if (localAppKindFilter !== "all" && app.kind !== localAppKindFilter) {
        return false;
      }
      if (!normalizedQuery) {
        return true;
      }
      return `${app.name} ${app.description} ${formatLocalAppKind(app.kind)}`
        .toLocaleLowerCase()
        .includes(normalizedQuery);
    });
    const groupedApps: Array<{ title: string; apps: LocalAppItem[] }> = [
      { title: "官方工具", apps: visibleApps.filter((app) => app.kind === "managed_tool") },
      { title: "已安装应用", apps: visibleApps.filter((app) => app.kind === "connector") },
      { title: "内置应用", apps: visibleApps.filter((app) => app.kind === "built_in") },
      { title: "自定义应用", apps: visibleApps.filter((app) => app.kind === "custom") }
    ].filter((group) => group.apps.length > 0);

    return (
      <div className="local-app-panel">
        {renderAppAttentionPanel()}
        <div className="app-toolbar">
          <div className="app-toolbar-group">
            <label className="app-search">
              <Search size={16} strokeWidth={1.8} aria-hidden="true" />
              <span className="sr-only">搜索本地应用</span>
              <input
                type="search"
                value={localAppQuery}
                onChange={(event) => setLocalAppQuery(event.target.value)}
                placeholder="搜索应用或能力"
              />
            </label>
            <select
              className="app-kind-filter"
              value={localAppKindFilter}
              onChange={(event) => setLocalAppKindFilter(event.target.value as LocalAppKind | "all")}
              aria-label="按应用类型筛选"
            >
              <option value="all">全部类型</option>
              <option value="managed_tool">官方工具</option>
              <option value="connector">已安装应用</option>
              <option value="built_in">内置应用</option>
              <option value="custom">自定义应用</option>
            </select>
            <span className="app-result-count">{visibleApps.length} 个应用</span>
          </div>
          <div className="app-toolbar-actions">
            {!needsAuthorization ? (
              <button
                className="ghost button-with-icon"
                onClick={() => void openConsole()}
                disabled={busy}
              >
                <ExternalLink size={15} aria-hidden="true" />
                打开控制台
              </button>
            ) : null}
            <button
              className="icon-button"
              onClick={() => void refreshAll()}
              disabled={refreshing}
              aria-label="刷新应用状态"
              title="刷新"
            >
              <RefreshCw size={17} className={refreshing ? "spin" : undefined} aria-hidden="true" />
            </button>
            <button
              className="primary button-with-icon"
              onClick={() => {
                setInstallSourceMode("market");
                setMarketAppQuery("");
                setCustomInstallConfirmed(false);
                setInstallPanelOpen(true);
                void refreshMarketConnectorApps();
              }}
            >
              <Plus size={16} strokeWidth={2} aria-hidden="true" />
              安装应用
            </button>
          </div>
        </div>
        {availableLocalAppUpdates.length > 0 ? (
          <div className="notice-banner warning" role="status">
            <div>
              <strong>{availableLocalAppUpdates.length} 个应用有可用更新</strong>
              <span>
                {availableLocalAppUpdates
                  .map(({ app, status }) => `${app.name} ${status.currentVersion} → ${status.latestVersion}`)
                  .join("；")}
              </span>
            </div>
            <button
              className="secondary"
              onClick={() => setSelectedLocalAppId(availableLocalAppUpdates[0].app.id)}
            >
              查看更新
            </button>
          </div>
        ) : null}
        {groupedApps.length > 0 ? (
          <div className="local-app-groups">
            {groupedApps.map((group) => (
              <section className="local-app-group" key={group.title}>
                <div className="method-advanced-head">
                  <strong>{group.title}</strong>
                  <small>{group.apps.length} 个应用</small>
                </div>
                <div className="local-app-grid">
                  {group.apps.map((app) => renderLocalAppCard(app))}
                </div>
              </section>
            ))}
          </div>
        ) : (
          <Card
            title={localApps.length === 0 ? "还没有应用" : "没有匹配的应用"}
            description={
              localApps.length === 0
                ? "从应用市场安装本地应用，或安装已注册但尚未公开上架的 Git 版本。"
                : "调整搜索词或应用类型筛选后重试。"
            }
          >
            <div className="empty-state">
              {localApps.length === 0 ? "还没有安装应用。" : "当前筛选条件下没有结果。"}
            </div>
          </Card>
        )}
      </div>
    );
  }
export function LocalAppCard(props: LocalAppCardProps) {
  const { app, config, countLocalAppCapabilities, formatLocalAppKind, hasLocalAppStartCommand, localAppLifecycle, localAppUpdateStatus, marketAppForLocalApp, renderLocalAppInstallProgress, setActiveLocalAppDetailTab, setExpandedServiceIndex, setSelectedLocalAppId } = props;
    if (!config) {
      return null;
    }
    const hasStartCommand = hasLocalAppStartCommand(app);
    const lifecycle = localAppLifecycle(app);
    const updateStatus = localAppUpdateStatus(app);
    const iconDataUrl = app.connector?.iconDataUrl ?? marketAppForLocalApp(app)?.iconDataUrl;
    return (
      <button
        className="local-app-card"
        key={app.id}
        onClick={() => {
          setSelectedLocalAppId(app.id);
          setActiveLocalAppDetailTab(defaultLocalAppDetailTab(app));
          if (app.serviceIndexes.length > 0) {
            setExpandedServiceIndex(app.serviceIndexes[0]);
          }
        }}
      >
        <div className="local-app-card-top">
          <span className="local-app-card-identity">
            <ApplicationIcon className="local-app-card-icon" name={app.name} src={iconDataUrl} />
            <strong>{app.name}</strong>
          </span>
          <span className={`sidebar-service-status status-${lifecycle.statusClass}`}>
            {lifecycle.label}
          </span>
        </div>
        <p>{app.description}</p>
        {app.installTask ? renderLocalAppInstallProgress(app.installTask, true) : null}
        <div className="local-app-card-meta">
          <span>{formatLocalAppKind(app.kind)}</span>
          {app.managedTool ? (
            <span>版本 {app.managedTool.installedVersion ?? "未安装"}</span>
          ) : app.installTask && !app.connector ? (
            <span>{formatLocalAppInstallTaskPhase(app.installTask)}</span>
          ) : (
            <span>{countLocalAppCapabilities(app, config)} 项能力</span>
          )}
          {hasStartCommand ? <span>可启动</span> : null}
          {updateStatus?.updateAvailable ? (
            <span className="status-pill status-warning">可更新到 {updateStatus.latestVersion}</span>
          ) : null}
        </div>
      </button>
    );
  }
