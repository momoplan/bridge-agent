import type { ReactNode } from "react";
import {
  Activity,
  Blocks,
  Home,
  RefreshCw,
  Settings,
  SlidersHorizontal
} from "lucide-react";
import bjmLogoLight from "../assets/brand/bjm-logo-light.svg";

export type DesktopPage = "overview" | "apps" | "diagnostics" | "settings";

interface DesktopSidebarProps {
  activePage: DesktopPage;
  deviceName: string;
  statusClass: string;
  statusLabel: string;
  onNavigate: (page: DesktopPage) => void;
}

const NAV_ITEMS: Array<{
  id: DesktopPage;
  label: string;
  description: string;
  icon: typeof Home;
}> = [
  { id: "overview", label: "概览", description: "连接与运行状态", icon: Home },
  { id: "apps", label: "本地应用", description: "应用与开放能力", icon: Blocks },
  { id: "diagnostics", label: "诊断", description: "系统、日志与清单", icon: Activity }
];

export function DesktopSidebar({
  activePage,
  deviceName,
  statusClass,
  statusLabel,
  onNavigate
}: DesktopSidebarProps) {
  return (
    <aside className="desktop-sidebar">
      <div className="desktop-brand">
        <img src={bjmLogoLight} alt="" aria-hidden="true" />
        <div>
          <strong>百积木</strong>
          <span>本地连接客户端</span>
        </div>
      </div>

      <nav className="desktop-nav" aria-label="主导航">
        <span className="desktop-nav-label">工作台</span>
        {NAV_ITEMS.map((item) => {
          const Icon = item.icon;
          return (
            <button
              key={item.id}
              className={`desktop-nav-item ${activePage === item.id ? "active" : ""}`}
              onClick={() => onNavigate(item.id)}
              aria-current={activePage === item.id ? "page" : undefined}
            >
              <Icon size={18} strokeWidth={1.8} aria-hidden="true" />
              <span>
                <strong>{item.label}</strong>
                <small>{item.description}</small>
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
        >
          <Settings size={18} strokeWidth={1.8} aria-hidden="true" />
          <span>
            <strong>设置</strong>
            <small>设备、连接与运行参数</small>
          </span>
        </button>
      </nav>

      <div className="desktop-device">
        <div className="desktop-device-icon">
          <SlidersHorizontal size={17} strokeWidth={1.8} aria-hidden="true" />
        </div>
        <div>
          <strong title={deviceName}>{deviceName}</strong>
          <span>
            <i className={`status-dot status-${statusClass}`} aria-hidden="true" />
            {statusLabel}
          </span>
        </div>
      </div>
    </aside>
  );
}

interface DesktopHeaderProps {
  title: string;
  description?: string;
  busy?: boolean;
  onRefresh?: () => void;
  actions?: ReactNode;
}

export function DesktopHeader({
  title,
  description,
  busy = false,
  onRefresh,
  actions
}: DesktopHeaderProps) {
  return (
    <header className="desktop-header">
      <div className="desktop-header-copy">
        <h1>{title}</h1>
        {description ? <p>{description}</p> : null}
      </div>
      <div className="desktop-header-actions">
        {onRefresh ? (
          <button
            className="icon-button"
            onClick={onRefresh}
            disabled={busy}
            aria-label="刷新当前状态"
            title="刷新"
          >
            <RefreshCw size={17} className={busy ? "spin" : undefined} aria-hidden="true" />
          </button>
        ) : null}
        {actions}
      </div>
    </header>
  );
}

interface DesktopStatusBarProps {
  statusClass: string;
  statusLabel: string;
  workspace: string;
  version: string;
  lastEvent: string;
}

export function DesktopStatusBar({
  statusClass,
  statusLabel,
  workspace,
  version,
  lastEvent
}: DesktopStatusBarProps) {
  return (
    <footer className="desktop-statusbar">
      <span>
        <i className={`status-dot status-${statusClass}`} aria-hidden="true" />
        Agent {statusLabel}
      </span>
      <span>工作区 {workspace || "未授权"}</span>
      <span className="desktop-statusbar-spacer" />
      <span>最近事件 {lastEvent}</span>
      <span>v{version}</span>
    </footer>
  );
}
