import type { Dispatch, SetStateAction } from "react";
import { DeviceAuthorizationGate } from "../components/DeviceAuthorizationGate";
import { deriveDeviceAuthorizationState } from "../device-authorization-state";
import type { ServiceRuntimeView } from "./service-status-panels";
import type { AppPage, BrowserAuthStartResponse, LocalAppItem, LocalAppLifecycle, LogEntry, RuntimeLockConflict, UiAgentConfig, UiServiceConfig } from "./types";

interface RuntimeConflictPanelProps { busy: boolean; runtimeConflict: RuntimeLockConflict | null; setRuntimeConflict: Dispatch<SetStateAction<RuntimeLockConflict | null>>; stopConflictingRuntimeAndStart: () => Promise<void>; }
interface AuthorizationGatePanelProps { authorizationState: ReturnType<typeof deriveDeviceAuthorizationState>; beginBrowserAuth: () => Promise<void>; browserAuth: BrowserAuthStartResponse | null; busy: boolean; config: UiAgentConfig | null; copyText: (text: string, label: string) => Promise<void>; openExternalUrl: (url: string) => Promise<void>; openExternalUrlInEdge: (url: string) => Promise<void>; setActivePage: Dispatch<SetStateAction<AppPage>>; }
interface LocalAppRuntimePanelProps { app: LocalAppItem; config: UiAgentConfig | null; localAppLifecycle: (app: LocalAppItem) => LocalAppLifecycle; serviceRuntimeView: (service: UiServiceConfig) => ServiceRuntimeView; }
interface LogMetadataProps { entry: LogEntry; }

export function RuntimeConflictPanel(props: RuntimeConflictPanelProps) {
  const { busy, runtimeConflict, setRuntimeConflict, stopConflictingRuntimeAndStart } = props;
    if (!runtimeConflict) {
      return null;
    }

    const processName = runtimeConflict.process.name || "未知进程";
    const processPath =
      runtimeConflict.process.executable_path || runtimeConflict.process.command_line || "无法读取进程路径";

    return (
      <div className="runtime-conflict-panel" role="alert">
        <div className="runtime-conflict-copy">
          <strong>百积木已经在运行</strong>
          <p>
            当前配置被 PID {runtimeConflict.pid} 占用。确认这是旧实例后，可以停止旧实例并重新启动。
          </p>
          <dl>
            <div>
              <dt>进程</dt>
              <dd>{processName}</dd>
            </div>
            <div>
              <dt>路径</dt>
              <dd>{processPath}</dd>
            </div>
            <div>
              <dt>锁文件</dt>
              <dd>{runtimeConflict.lock_path}</dd>
            </div>
          </dl>
        </div>
        <div className="runtime-conflict-actions">
          <button className="primary danger" onClick={() => void stopConflictingRuntimeAndStart()} disabled={busy}>
            停止旧实例并启动
          </button>
          <button className="secondary" onClick={() => setRuntimeConflict(null)} disabled={busy}>
            暂不处理
          </button>
        </div>
      </div>
    );
  }
export function AuthorizationGatePanel(props: AuthorizationGatePanelProps) {
  const { authorizationState, beginBrowserAuth, browserAuth, busy, config, copyText, openExternalUrl, openExternalUrlInEdge, setActivePage } = props;
    if (authorizationState === "authorized" || !config) {
      return null;
    }

    return (
      <DeviceAuthorizationGate
        state={authorizationState}
        workspaceId={config.platform.workspace_id}
        pendingAuthorization={browserAuth}
        busy={busy}
        onAuthorize={() => void beginBrowserAuth()}
        onCopyAuthorizationUrl={() => {
          if (browserAuth) {
            void copyText(browserAuth.verificationUriComplete, "授权链接");
          }
        }}
        onOpenAuthorizationUrl={() => {
          if (browserAuth) {
            void openExternalUrl(browserAuth.verificationUriComplete);
          }
        }}
        onOpenAuthorizationUrlInEdge={() => {
          if (browserAuth) {
            void openExternalUrlInEdge(browserAuth.verificationUriComplete);
          }
        }}
        onOpenDiagnostics={() => setActivePage("diagnostics")}
      />
    );
  }
export function LocalAppRuntimePanel(props: LocalAppRuntimePanelProps) {
  const { app, config, localAppLifecycle, serviceRuntimeView } = props;
    if (!config) {
      return null;
    }
    const services = app.serviceIndexes
      .map((serviceIndex) => config.services[serviceIndex])
      .filter((service): service is UiServiceConfig => Boolean(service));
    const localApp =
      app.localAppIndex == null ? null : config.local_apps[app.localAppIndex] ?? null;
    if (services.length === 0 && !localApp) {
      return null;
    }
    const lifecycle = localAppLifecycle(app);

    return (
      <div className="local-app-runtime-list">
        <div className="local-app-lifecycle-row">
          <div>
            <strong>应用生命周期</strong>
            <p>{lifecycle.detail}</p>
          </div>
          <div className={`status-pill status-${lifecycle.statusClass}`}>{lifecycle.label}</div>
        </div>
        {services.map((service) => {
          const runtimeView = serviceRuntimeView(service);
          return (
            <div className="local-app-runtime-row" key={service.name}>
              <div>
                <strong>{service.description || service.name}</strong>
                <p>{runtimeView.detail}</p>
              </div>
              {runtimeView.statusLabel && runtimeView.statusClass ? (
                <div className={`status-pill status-${runtimeView.statusClass}`}>
                  {runtimeView.statusLabel}
                </div>
              ) : null}
            </div>
          );
        })}
        {localApp ? (
          <div className="local-app-runtime-row" key={localApp.appId}>
            <div>
              <strong>{localApp.name}</strong>
              <p>
                {localApp.healthCheck
                  ? "按应用级 healthCheck 检查运行状态"
                  : "应用未声明 healthCheck"}
              </p>
            </div>
          </div>
        ) : null}
      </div>
    );
  }
export function LogMetadata(props: LogMetadataProps) {
  const { entry } = props;
    const items = [
      entry.category,
      entry.service,
      entry.method,
      entry.event,
      entry.outcome,
      entry.request_id ? `request ${entry.request_id}` : null,
      entry.event_id ? `event ${entry.event_id}` : null,
      entry.duration_ms != null ? `${entry.duration_ms}ms` : null,
      entry.http_method && entry.path ? `${entry.http_method} ${entry.path}` : null,
      entry.status_code != null ? `${entry.status_code}` : null
    ].filter((item): item is string => Boolean(item));

    if (items.length === 0) {
      return null;
    }

    return (
      <div className="log-meta">
        {items.map((item) => (
          <span key={item}>{item}</span>
        ))}
      </div>
    );
  }
