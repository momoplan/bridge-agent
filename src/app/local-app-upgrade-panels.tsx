import type { Dispatch, ReactNode, SetStateAction } from "react";
import { AlertTriangle, ShieldCheck, X } from "lucide-react";
import { formatLocalAppInstallTaskPhase, type LocalAppInstallTask } from "../local-app-install-tasks";
import { describeLocalAppUpdate } from "../local-app-updates";
import { formatPublishedAt } from "./formatters";
import { legacyEventContracts, legacyMethodContracts } from "./legacy-contracts";
import { ApplicationIcon } from "./ui-primitives";
import type { LocalAppItem, LocalAppUpdateStatus, MarketConnector } from "./types";

interface LocalAppInstallProgressProps { compact?: boolean; task: LocalAppInstallTask; }
interface LocalAppUpdateChangesProps { app: LocalAppItem; marketApp: MarketConnector; }
interface LocalAppUpgradeDialogProps { app: LocalAppItem; connectorUpdateBusy: string | null; localAppUpdateStatus: (app: LocalAppItem) => LocalAppUpdateStatus | undefined; managedToolBusy: boolean; marketAppForLocalApp: (app: LocalAppItem) => MarketConnector | undefined; renderLocalAppUpdateChanges: (app: LocalAppItem, marketApp: MarketConnector) => ReactNode; setPendingUpgradeAppId: Dispatch<SetStateAction<string | null>>; upgradeLocalAppVersion: (app: LocalAppItem) => Promise<void>; }

export function LocalAppInstallProgress(props: LocalAppInstallProgressProps) {
  const { compact = false, task } = props;
    const percent = task.phase === "succeeded" ? 100 : task.progressPercent;
    const indeterminate = percent == null && task.phase !== "failed";
    return (
      <div
        className={`local-app-install-progress ${compact ? "compact" : ""} ${task.phase === "failed" ? "failed" : ""}`}
        role="status"
        aria-label={`${task.name}：${task.error || task.message}`}
      >
        <div className="local-app-install-progress-head">
          <strong>{formatLocalAppInstallTaskPhase(task)}</strong>
          <span>{percent == null ? "" : `${Math.round(percent)}%`}</span>
        </div>
        {task.phase !== "failed" ? (
          <div className="local-app-install-progress-track" aria-hidden="true">
            <div
              className={`local-app-install-progress-bar ${indeterminate ? "indeterminate" : ""}`}
              style={indeterminate ? undefined : { width: `${Math.max(0, Math.min(100, percent ?? 0))}%` }}
            />
          </div>
        ) : null}
        <p>{task.error || task.message}</p>
      </div>
    );
  }
export function LocalAppUpdateChanges(props: LocalAppUpdateChangesProps) {
  const { app, marketApp } = props;
    const changes = describeLocalAppUpdate(
      app.connector
        ? {
            configSchema: app.connector.configSchema ?? null,
            database: app.connector.database ?? null,
            methods: app.connector.methods ?? legacyMethodContracts(app.connector.methodNames),
            events: app.connector.events ?? legacyEventContracts(app.connector.eventNames),
            permissions: app.connector.permissions
          }
        : null,
      marketApp
    );
    const hasStructuralChanges = changes.sections.some((section) => section.changes.length > 0);
    const riskLabel = {
      breaking: "包含破坏性变化",
      attention: "需要确认",
      compatible: hasStructuralChanges ? "兼容性变化" : "未发现结构变化",
      unknown: "变更声明不完整"
    }[changes.highestRisk];

    return (
      <section className="local-app-update-changes" aria-label="版本改动详情">
        <div className="local-app-update-changes-head">
          <div>
            <strong>本次更新内容</strong>
            <span>
              {app.managedTool?.installedVersion ?? app.connector?.version ?? "未安装"}
              {" → "}
              {marketApp.version}
            </span>
          </div>
          <div className="local-app-update-risk-summary">
            <span className={`local-app-update-risk ${changes.highestRisk}`}>{riskLabel}</span>
            {marketApp.publishedAt ? <small>发布于 {formatPublishedAt(marketApp.publishedAt)}</small> : null}
          </div>
        </div>
        {changes.releaseNotes.length > 0 ? (
          <div className="local-app-release-notes">
            <strong>版本说明</strong>
            <ul>
              {changes.releaseNotes.map((note, index) => <li key={`${note}:${index}`}>{note}</li>)}
            </ul>
          </div>
        ) : (
          <div className="local-app-update-no-notes compact">
            <AlertTriangle size={16} aria-hidden="true" />
            <span>发布方未提供版本说明，请以以下结构化契约差异为准。</span>
          </div>
        )}
        <div className="local-app-update-contracts">
          {changes.sections.map((contractSection) => (
            <article
              className={`local-app-update-contract ${!contractSection.applicable ? "not-applicable" : contractSection.declared ? "declared" : "undeclared"}`}
              key={contractSection.id}
            >
              <div className="local-app-update-contract-head">
                <strong>{contractSection.title}</strong>
                <span>
                  {!contractSection.applicable
                    ? "不适用"
                    : !contractSection.declared
                    ? "未声明"
                    : contractSection.unchanged
                      ? "无变化"
                      : `${contractSection.changes.length} 项`}
                </span>
              </div>
              {contractSection.changes.length > 0 ? (
                <ul className="local-app-update-contract-list">
                  {contractSection.changes.map((change) => (
                    <li className={change.tone} key={change.id}>
                      <span className="local-app-update-change-marker" aria-hidden="true" />
                      <div>
                        <strong>{change.title}</strong>
                        {change.detail ? <p>{change.detail}</p> : null}
                      </div>
                    </li>
                  ))}
                </ul>
              ) : (
                <p className={`local-app-update-contract-empty ${contractSection.applicable && !contractSection.declared ? "warning" : ""}`}>
                  {contractSection.emptyMessage}
                </p>
              )}
            </article>
          ))}
        </div>
        {!changes.hasSpecificChanges && changes.undeclaredSections.length > 0 ? (
          <div className="local-app-update-no-notes">
            <AlertTriangle size={17} aria-hidden="true" />
            <span>无法完整审查 {changes.undeclaredSections.join("、")}；这不代表这些部分没有变化。</span>
          </div>
        ) : null}
      </section>
    );
  }
export function LocalAppUpgradeDialog(props: LocalAppUpgradeDialogProps) {
  const { app, connectorUpdateBusy, localAppUpdateStatus, managedToolBusy, marketAppForLocalApp, renderLocalAppUpdateChanges, setPendingUpgradeAppId, upgradeLocalAppVersion } = props;
    const marketApp = marketAppForLocalApp(app);
    const updateStatus = localAppUpdateStatus(app);
    if (!marketApp || !updateStatus?.updateAvailable) {
      return null;
    }
    const upgradeBusy = connectorUpdateBusy === app.id || (app.kind === "managed_tool" && managedToolBusy);
    const closeUpgrade = () => setPendingUpgradeAppId(null);

    return (
      <div className="modal-backdrop local-app-upgrade-backdrop" role="presentation" onClick={closeUpgrade}>
        <section
          className="local-app-upgrade-dialog"
          role="dialog"
          aria-modal="true"
          aria-labelledby="local-app-upgrade-title"
          onClick={(event) => event.stopPropagation()}
        >
          <div className="install-panel-head">
            <div className="app-detail-heading">
              <ApplicationIcon
                className="app-detail-icon"
                name={app.name}
                src={app.connector?.iconDataUrl ?? marketApp.iconDataUrl}
              />
              <div>
                <p className="eyebrow">确认升级</p>
                <h3 id="local-app-upgrade-title">{app.name}</h3>
                <p>确认具体改动后，再安装 {updateStatus.latestVersion}。</p>
              </div>
            </div>
            <button className="icon-button" onClick={closeUpgrade} aria-label="关闭升级确认">
              <X size={18} aria-hidden="true" />
            </button>
          </div>
          <div className="local-app-upgrade-body">
            {renderLocalAppUpdateChanges(app, marketApp)}
            <div className={`market-permission-card ${marketApp.compatible ? "" : "incompatible"}`}>
              {marketApp.compatible ? <ShieldCheck size={18} aria-hidden="true" /> : <AlertTriangle size={18} aria-hidden="true" />}
              <div>
                <strong>{marketApp.compatible ? "升级包已通过平台校验" : "当前版本不可升级"}</strong>
                <p>{marketApp.compatible ? marketApp.risk : marketApp.compatibilityMessage}</p>
              </div>
            </div>
          </div>
          <div className="install-panel-actions">
            <button className="secondary" onClick={closeUpgrade}>{upgradeBusy ? "关闭" : "取消"}</button>
            <button
              className="primary accent"
              onClick={() => void upgradeLocalAppVersion(app)}
              disabled={upgradeBusy || !marketApp.compatible}
            >
              {upgradeBusy ? "正在升级…" : `确认升级到 ${updateStatus.latestVersion}`}
            </button>
          </div>
        </section>
      </div>
    );
  }
