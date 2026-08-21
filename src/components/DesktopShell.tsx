import { useEffect, useRef, useState } from "react";
import {
  Activity,
  Blocks,
  ChevronDown,
  House,
  Settings,
  X
} from "lucide-react";
import bjmLogoLight from "../assets/brand/bjm-logo-light.svg";
import type { DeviceAuthorizationState } from "../device-authorization-state";

export type DesktopPage = "apps" | "diagnostics" | "settings";

interface DesktopSidebarProps {
  activePage: DesktopPage;
  deviceName: string;
  statusClass: string;
  statusLabel: string;
  workspace: string;
  relay: string;
  lastEvent: string;
  version: string;
  lastError?: string | null;
  authorizationState: DeviceAuthorizationState;
  onNavigate: (page: DesktopPage) => void;
}

const NAV_ITEMS: Array<{
  id: DesktopPage;
  label: string;
  description: string;
  icon: typeof Blocks;
}> = [
  { id: "apps", label: "应用", description: "本机应用与开放能力", icon: Blocks },
  { id: "diagnostics", label: "诊断", description: "系统、日志与清单", icon: Activity }
];

export function DesktopSidebar({
  activePage,
  deviceName,
  statusClass,
  statusLabel,
  workspace,
  relay,
  lastEvent,
  version,
  lastError,
  authorizationState,
  onNavigate
}: DesktopSidebarProps) {
  const [connectionOpen, setConnectionOpen] = useState(false);
  const connectionAreaRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!connectionOpen) {
      return;
    }

    function closeOnOutsidePointer(event: PointerEvent) {
      if (event.target instanceof Node && !connectionAreaRef.current?.contains(event.target)) {
        setConnectionOpen(false);
      }
    }

    function closeOnEscape(event: KeyboardEvent) {
      if (event.key === "Escape") {
        setConnectionOpen(false);
      }
    }

    document.addEventListener("pointerdown", closeOnOutsidePointer);
    document.addEventListener("keydown", closeOnEscape);
    return () => {
      document.removeEventListener("pointerdown", closeOnOutsidePointer);
      document.removeEventListener("keydown", closeOnEscape);
    };
  }, [connectionOpen]);

  useEffect(() => {
    setConnectionOpen(false);
  }, [activePage]);

  const connectionTitle = `Agent ${statusLabel} · 点击查看连接详情`;

  function toggleConnectionDetails() {
    setConnectionOpen((current) => !current);
  }

  function openDiagnostics() {
    setConnectionOpen(false);
    onNavigate("diagnostics");
  }

  const workspaceLabel = workspace || "未授权";
  const authorizationLocked = authorizationState !== "authorized";
  const hasKnownVersion = Boolean(version && version !== "-");
  const versionLabel = hasKnownVersion ? `v${version}` : "版本未知";
  const versionDescription = hasKnownVersion ? version : "未知";

  return (
    <aside className="desktop-sidebar">
      <div className="desktop-brand" ref={connectionAreaRef}>
        <button
          className="desktop-brand-home"
          onClick={() => onNavigate("apps")}
          aria-label={authorizationLocked ? "打开设备授权页" : "打开应用首页"}
          title={authorizationLocked ? "打开设备授权页" : "打开应用首页"}
        >
          <img src={bjmLogoLight} alt="" aria-hidden="true" />
          <House className="desktop-brand-home-icon" size={12} aria-hidden="true" />
        </button>
        <div className="desktop-brand-copy">
          <strong>百积木</strong>
          <button
            className="desktop-connection-trigger"
            onClick={toggleConnectionDetails}
            aria-expanded={connectionOpen}
            aria-controls="desktop-connection-popover"
            aria-haspopup="dialog"
            title={connectionTitle}
          >
            <i className={`status-dot status-${statusClass}`} aria-hidden="true" />
            <span>Agent {statusLabel}</span>
            <ChevronDown
              className={connectionOpen ? "open" : undefined}
              size={12}
              strokeWidth={2}
              aria-hidden="true"
            />
          </button>
        </div>
        <button
          className="desktop-connection-compact"
          onClick={toggleConnectionDetails}
          aria-expanded={connectionOpen}
          aria-controls="desktop-connection-popover"
          aria-haspopup="dialog"
          aria-label={connectionTitle}
          title={connectionTitle}
        >
          <i className={`status-dot status-${statusClass}`} aria-hidden="true" />
        </button>

        {connectionOpen ? (
          <section
            className="desktop-connection-popover"
            id="desktop-connection-popover"
            role="dialog"
            aria-labelledby="desktop-connection-title"
          >
            <header className="desktop-connection-head">
              <div>
                <span id="desktop-connection-title">连接详情</span>
                <strong>
                  <i className={`status-dot status-${statusClass}`} aria-hidden="true" />
                  Agent {statusLabel}
                </strong>
              </div>
              <button
                className="desktop-connection-close"
                onClick={() => setConnectionOpen(false)}
                aria-label="关闭连接详情"
                title="关闭"
              >
                <X size={15} aria-hidden="true" />
              </button>
            </header>

            <dl className="desktop-connection-list">
              <div>
                <dt>设备</dt>
                <dd title={deviceName}>{deviceName}</dd>
              </div>
              <div>
                <dt>工作区</dt>
                <dd>{workspace || "未授权"}</dd>
              </div>
              <div>
                <dt>Relay</dt>
                <dd title={relay}>{relay || "-"}</dd>
              </div>
              <div>
                <dt>最近事件</dt>
                <dd>{lastEvent}</dd>
              </div>
              <div>
                <dt>客户端版本</dt>
                <dd>v{version}</dd>
              </div>
            </dl>

            {lastError ? (
              <div className="desktop-connection-error" role="status">
                <span>最近错误</span>
                <p>{lastError}</p>
              </div>
            ) : null}

            <footer className="desktop-connection-actions">
              <button className="secondary button-with-icon" onClick={openDiagnostics}>
                <Activity size={14} aria-hidden="true" />
                打开诊断
              </button>
            </footer>
          </section>
        ) : null}
      </div>

      <nav className="desktop-nav" aria-label="主导航">
        <span className="desktop-nav-label">工作台</span>
        {NAV_ITEMS.map((item) => {
          const Icon = item.icon;
          const authorizationHome = authorizationLocked && item.id === "apps";
          return (
            <button
              key={item.id}
              className={`desktop-nav-item ${activePage === item.id ? "active" : ""}`}
              onClick={() => onNavigate(item.id)}
              aria-current={activePage === item.id ? "page" : undefined}
            >
              <Icon size={18} strokeWidth={1.8} aria-hidden="true" />
              <span>
                <strong>{authorizationHome ? "授权" : item.label}</strong>
                <small>{authorizationHome ? "授权后开放本地能力" : item.description}</small>
              </span>
            </button>
          );
        })}
      </nav>

      <div className="desktop-sidebar-spacer" />

      <nav className="desktop-nav desktop-nav-secondary" aria-label="客户端">
        <span className="desktop-nav-label">客户端</span>
        <button
          className={`desktop-nav-item ${activePage === "settings" ? "active" : ""}`}
          onClick={() => onNavigate("settings")}
          aria-current={activePage === "settings" ? "page" : undefined}
          disabled={authorizationLocked}
          title={authorizationLocked ? "完成设备授权后开放设置" : undefined}
        >
          <Settings size={18} strokeWidth={1.8} aria-hidden="true" />
          <span>
            <strong>设置</strong>
            <small>设备、连接与运行参数</small>
          </span>
        </button>
      </nav>

      <footer
        className="desktop-environment-meta"
        aria-label={`当前设备 ${deviceName}，当前工作区 ${workspaceLabel}，客户端版本 ${versionDescription}`}
        title={`设备 ${deviceName} · 工作区 ${workspaceLabel} · 客户端 ${versionLabel}`}
      >
        <span className="desktop-environment-device">
          <span className="desktop-environment-label">设备</span>
          <strong title={deviceName}>{deviceName}</strong>
        </span>
        <span className="desktop-environment-context">
          <span className="desktop-environment-workspace">
            <span className="desktop-environment-label">工作区</span>
            <strong>{workspace ? `#${workspace}` : workspaceLabel}</strong>
          </span>
          <span className="desktop-environment-separator" aria-hidden="true">
            ·
          </span>
          <span className="desktop-environment-version">{versionLabel}</span>
        </span>
      </footer>
    </aside>
  );
}
