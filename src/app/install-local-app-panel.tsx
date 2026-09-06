import type { Dispatch, SetStateAction } from "react";
import { ArrowLeft, AlertTriangle, CheckCircle2, Package, RefreshCw, Search, ShieldCheck, Wrench, X } from "lucide-react";
import { resolveMarketHostUpgradeAction } from "../local-app-host-upgrade";
import { formatAppUpdateProgressButton } from "./formatters";
import { ApplicationIcon, Field } from "./ui-primitives";
import type { AppUpdateCheckState, AppUpdateProgress, AppUpdateStatus, InstallSourceMode, MarketConnector } from "./types";

interface InstallLocalAppPanelProps {
  appUpdate: AppUpdateStatus | null;
  appUpdateCheckState: AppUpdateCheckState;
  appUpdateProgress: AppUpdateProgress | null;
  customInstallConfirmed: boolean;
  installBusy: boolean;
  installLocalApp: () => Promise<void>;
  installPanelOpen: boolean;
  installSourceMode: InstallSourceMode;
  installableMarketConnectors: MarketConnector[];
  marketAppQuery: string;
  marketLoadError: string;
  marketLoading: boolean;
  refreshMarketConnectorApps: () => Promise<MarketConnector[]>;
  registeredInstallAppId: string;
  registeredInstallVersion: string;
  selectedMarketAppId: string;
  setCustomInstallConfirmed: Dispatch<SetStateAction<boolean>>;
  setInstallPanelOpen: Dispatch<SetStateAction<boolean>>;
  setInstallSourceMode: Dispatch<SetStateAction<InstallSourceMode>>;
  setMarketAppQuery: Dispatch<SetStateAction<string>>;
  setRegisteredInstallAppId: Dispatch<SetStateAction<string>>;
  setRegisteredInstallVersion: Dispatch<SetStateAction<string>>;
  setSelectedMarketAppId: Dispatch<SetStateAction<string>>;
  updateBusy: boolean;
  upgradeClientForMarketApp: (app: MarketConnector) => Promise<void>;
  visibleMarketConnectors: MarketConnector[];
}

export function InstallLocalAppPanel(props: InstallLocalAppPanelProps) {
  const { appUpdate, appUpdateCheckState, appUpdateProgress, customInstallConfirmed, installBusy, installLocalApp, installPanelOpen, installSourceMode, installableMarketConnectors, marketAppQuery, marketLoadError, marketLoading, refreshMarketConnectorApps, registeredInstallAppId, registeredInstallVersion, selectedMarketAppId, setCustomInstallConfirmed, setInstallPanelOpen, setInstallSourceMode, setMarketAppQuery, setRegisteredInstallAppId, setRegisteredInstallVersion, setSelectedMarketAppId, updateBusy, upgradeClientForMarketApp, visibleMarketConnectors } = props;
    if (!installPanelOpen) {
      return null;
    }
    const selectedMarket = installableMarketConnectors.find((app) => app.appId === selectedMarketAppId);
    const selectedMarketIncompatible = selectedMarket?.compatible === false;
    const marketPrimaryAction = selectedMarket
      ? resolveMarketHostUpgradeAction(
          selectedMarket.name,
          !selectedMarketIncompatible,
          appUpdate,
          appUpdateCheckState === "checking",
          updateBusy,
          formatAppUpdateProgressButton(appUpdateProgress)
        )
      : null;
    const closeInstallPanel = () => {
      setInstallPanelOpen(false);
      setInstallSourceMode("market");
      setMarketAppQuery("");
      setCustomInstallConfirmed(false);
    };

    return (
      <div className="modal-backdrop" role="presentation" onClick={closeInstallPanel}>
        <section
          className="install-panel"
          role="dialog"
          aria-modal="true"
          aria-labelledby="install-panel-title"
          onClick={(event) => event.stopPropagation()}
        >
          <div className="install-panel-head">
            <div className="install-panel-title">
              <span className={`install-panel-title-icon ${installSourceMode === "custom" ? "custom" : ""}`}>
                {installSourceMode === "market" ? <Package size={20} /> : <Wrench size={20} />}
              </span>
              <div>
                <h3 id="install-panel-title">
                  {installSourceMode === "market" ? "应用市场" : "注册版本安装"}
                </h3>
                <p>
                  {installSourceMode === "market"
                    ? "发现并安装经过平台验证的本地应用"
                    : "按平台注册的 appId 和精确版本安装"}
                </p>
              </div>
            </div>
            <div className="install-panel-head-actions">
              {installSourceMode === "market" ? (
                <button
                  className="secondary button-with-icon custom-install-entry"
                  onClick={() => {
                    setCustomInstallConfirmed(false);
                    setInstallSourceMode("custom");
                  }}
                  disabled={installBusy}
                >
                  <Wrench size={15} aria-hidden="true" />
                  注册版本安装
                </button>
              ) : (
                <button
                  className="secondary button-with-icon"
                  onClick={() => setInstallSourceMode("market")}
                  disabled={installBusy}
                >
                  <ArrowLeft size={15} aria-hidden="true" />
                  返回市场
                </button>
              )}
              <button
                className="icon-button install-panel-close"
                onClick={closeInstallPanel}
                aria-label="关闭应用市场"
                title="关闭"
              >
                <X size={18} aria-hidden="true" />
              </button>
            </div>
          </div>

          {installSourceMode === "market" ? (
            <div className="install-panel-body market-browser">
              <div className="market-toolbar">
                <label className="market-search">
                  <Search size={16} aria-hidden="true" />
                  <input
                    value={marketAppQuery}
                    onChange={(event) => setMarketAppQuery(event.target.value)}
                    placeholder="搜索应用、能力或版本"
                    aria-label="搜索市场应用"
                  />
                </label>
                <span>{marketLoading ? "正在更新市场…" : `${visibleMarketConnectors.length} 个可用应用`}</span>
              </div>

              <div className="market-browser-layout">
                <div className="market-app-list" aria-label="市场应用列表">
                  {marketLoading && installableMarketConnectors.length === 0 ? (
                    <div className="market-list-state" role="status">
                      <RefreshCw className="spin" size={20} aria-hidden="true" />
                      <strong>正在加载应用市场</strong>
                      <span>马上就好</span>
                    </div>
                  ) : null}
                  {!marketLoading && marketLoadError ? (
                    <div className="market-list-state error" role="alert">
                      <AlertTriangle size={20} aria-hidden="true" />
                      <strong>市场加载失败</strong>
                      <span>{marketLoadError}</span>
                      <button className="secondary" onClick={() => void refreshMarketConnectorApps()}>
                        重新加载
                      </button>
                    </div>
                  ) : null}
                  {!marketLoadError && visibleMarketConnectors.map((app) => (
                    <button
                      className={`market-app-card ${selectedMarketAppId === app.appId ? "active" : ""} ${app.compatible ? "" : "incompatible"}`}
                      key={app.appId}
                      onClick={() => setSelectedMarketAppId(app.appId)}
                      aria-pressed={selectedMarketAppId === app.appId}
                    >
                      <ApplicationIcon
                        className="market-app-icon"
                        name={app.name}
                        src={app.iconDataUrl}
                      />
                      <span className="market-app-card-copy">
                        <span className="market-app-card-title">
                          <strong>{app.name}</strong>
                          {!app.compatible ? <em>不兼容</em> : null}
                        </span>
                        <span>{app.description}</span>
                        <small>{app.capability} · {app.version}</small>
                      </span>
                    </button>
                  ))}
                  {!marketLoading && !marketLoadError && visibleMarketConnectors.length === 0 ? (
                    <div className="market-list-state">
                      <Search size={20} aria-hidden="true" />
                      <strong>{marketAppQuery.trim() ? "没有找到相关应用" : "市场暂时没有应用"}</strong>
                      <span>{marketAppQuery.trim() ? "试试应用名称或能力关键词" : "也可安装已注册但尚未公开上架的版本"}</span>
                    </div>
                  ) : null}
                </div>

                <div className="market-app-detail" aria-live="polite">
                  {selectedMarket ? (
                    <>
                      <div className="market-detail-hero">
                        <ApplicationIcon
                          className="market-detail-icon"
                          name={selectedMarket.name}
                          src={selectedMarket.iconDataUrl}
                        />
                        <div>
                          <div className="market-detail-title-row">
                            <h4>{selectedMarket.name}</h4>
                            <span className="market-verified-badge">
                              <CheckCircle2 size={13} aria-hidden="true" />
                              平台验证
                            </span>
                          </div>
                          <p>{selectedMarket.description}</p>
                        </div>
                      </div>
                      <div className="market-detail-meta">
                        <div><span>版本</span><strong>{selectedMarket.version}</strong></div>
                        <div><span>主要能力</span><strong>{selectedMarket.capability}</strong></div>
                      </div>
                      <div className={`market-permission-card ${selectedMarketIncompatible ? "incompatible" : ""}`}>
                        {selectedMarketIncompatible ? (
                          <AlertTriangle size={18} aria-hidden="true" />
                        ) : (
                          <ShieldCheck size={18} aria-hidden="true" />
                        )}
                        <div>
                          <strong>{selectedMarketIncompatible ? "当前版本不可安装" : "安装前须知"}</strong>
                          <p>
                            {selectedMarketIncompatible
                              ? selectedMarket.compatibilityMessage ||
                                `需要百积木 ${selectedMarket.minimumHostVersion ?? "更高版本"}`
                              : selectedMarket.risk}
                          </p>
                        </div>
                      </div>
                      {selectedMarket.requiredHostCapabilities.length > 0 ? (
                        <div className="market-requirements">
                          <strong>所需主机能力</strong>
                          <div>
                            {selectedMarket.requiredHostCapabilities.map((capability) => (
                              <span key={capability}>{capability}</span>
                            ))}
                          </div>
                        </div>
                      ) : null}
                    </>
                  ) : (
                    <div className="market-detail-empty">
                      <Package size={28} aria-hidden="true" />
                      <strong>选择一个应用查看详情</strong>
                      <span>安装前可确认版本、能力和权限说明</span>
                    </div>
                  )}
                </div>
              </div>
            </div>
          ) : (
            <div className="install-panel-body custom-install-view">
              <div className="custom-install-form">
                <div className="custom-install-intro">
                  <strong>安装已注册但未公开上架的应用</strong>
                  <p>输入 appId 和精确版本；客户端会从平台注册中心解析安装内容，并在下载前核验登记状态。</p>
                </div>
                <Field label="appId" wide>
                  <input
                    value={registeredInstallAppId}
                    onChange={(event) => setRegisteredInstallAppId(event.target.value)}
                    placeholder="已在平台注册的 appId"
                    autoFocus
                  />
                </Field>
                <Field label="版本" wide>
                  <input
                    value={registeredInstallVersion}
                    onChange={(event) => setRegisteredInstallVersion(event.target.value)}
                    placeholder="例如 3.0.1"
                  />
                </Field>
                <div className="install-risk-note">
                  <AlertTriangle size={18} aria-hidden="true" />
                  <div>
                    <strong>注册不等于公开审核</strong>
                    <span>
                      平台会拒绝未注册或已撤销的应用版本，但未公开审核的版本仍可能执行其声明的本机命令，请先确认开发者和权限。
                    </span>
                  </div>
                </div>
                <label className="check-row wide custom-trust-confirmation">
                  <input
                    type="checkbox"
                    checked={customInstallConfirmed}
                    onChange={(event) => setCustomInstallConfirmed(event.target.checked)}
                  />
                  <span>我已确认开发者和权限，同意安装这个已注册但未公开审核的版本</span>
                </label>
              </div>
            </div>
          )}

          <div className="install-panel-actions">
            <div className="install-action-context">
              {installSourceMode === "market" ? (
                selectedMarket ? (
                  <>
                    <strong>{selectedMarket.name}</strong>
                    <span>{selectedMarket.version} · {selectedMarket.capability}</span>
                  </>
                ) : (
                  <span>请选择一个应用后安装</span>
                )
              ) : (
                <>
                  <strong>已注册版本</strong>
                  <span>未注册、已撤销或身份不匹配的版本会被拒绝</span>
                </>
              )}
            </div>
            <button
              className="primary install-primary-action"
              onClick={() => {
                if (
                  installSourceMode === "market" &&
                  selectedMarket &&
                  marketPrimaryAction?.kind !== "install_app"
                ) {
                  void upgradeClientForMarketApp(selectedMarket);
                  return;
                }
                void installLocalApp();
              }}
              disabled={
                installBusy ||
                (installSourceMode === "market" && !selectedMarket) ||
                (installSourceMode === "market" && marketPrimaryAction?.disabled === true) ||
                (installSourceMode === "custom" &&
                  (!registeredInstallAppId || !registeredInstallVersion || !customInstallConfirmed))
              }
            >
              {installBusy
                ? "正在安装…"
                : installSourceMode === "market" && marketPrimaryAction
                  ? marketPrimaryAction.label
                  : installSourceMode === "market" && selectedMarket
                    ? `安装 ${selectedMarket.name}`
                    : "安装应用"}
            </button>
          </div>
        </section>
      </div>
    );
  }
