import { createServiceHealthCheck, createServiceStartCommand, serviceCapabilitiesJson } from "./config-conversion";
import { formatDesktopPermissionValue, formatRegisteredServiceDetail, formatRegisteredServiceStatus, formatTime } from "./formatters";
import { InfoRow } from "./ui-primitives";
import type { ConnectorSummary, DesktopPermissionStatus, RegisteredServiceStatus, UiServiceConfig } from "./types";

export interface ServiceRuntimeView { status: RegisteredServiceStatus | undefined; statusClass: string | null; statusLabel: string | null; detail: string; }
interface ServiceRuntimePanelProps { isSystem: boolean; refreshRegisteredServiceStatuses: () => Promise<unknown>; registeredServiceStatuses: RegisteredServiceStatus[]; service: UiServiceConfig; serviceIndex: number; serviceStartBusy: string | null; startRegisteredService: (serviceName: string) => Promise<boolean>; stopRegisteredService: (serviceName: string) => Promise<boolean>; updateService: (serviceIndex: number, updater: (service: UiServiceConfig) => UiServiceConfig) => void; }
interface ComputerPermissionPanelProps { desktopPermissionBusy: "accessibility" | "screen_recording" | null; desktopPermissions: DesktopPermissionStatus | null; openDesktopPermissionSettings: (permission: "accessibility" | "screen_recording" | "full_disk_access") => Promise<void>; requestDesktopPermission: (permission: "accessibility" | "screen_recording") => Promise<void>; service: UiServiceConfig; }
interface ConnectorPermissionPanelProps { connector: ConnectorSummary; desktopPermissions: DesktopPermissionStatus | null; openDesktopPermissionSettings: (permission: "accessibility" | "screen_recording" | "full_disk_access") => Promise<void>; }
interface ServiceDefinitionJsonPanelProps { applyServiceJsonDraft: (serviceIndex: number) => void; resetServiceJsonDraft: (serviceIndex: number) => void; service: UiServiceConfig; serviceIndex: number; serviceJsonDrafts: Record<number, string>; serviceJsonErrors: Record<number, string>; updateServiceJsonDraft: (serviceIndex: number, value: string) => void; }

export function resolveServiceRuntimeView(service: UiServiceConfig, registeredServiceStatuses: RegisteredServiceStatus[]): ServiceRuntimeView {
    const runtimeStatus = registeredServiceStatuses.find((status) => status.service === service.name);
    if (
      !service.health_check &&
      !service.start_command &&
      !service.stop_command &&
      (!runtimeStatus || runtimeStatus.status === "not_configured")
    ) {
      return {
        status: undefined,
        statusClass: null,
        statusLabel: null,
        detail: formatRegisteredServiceDetail(service, undefined)
      };
    }
    const statusIsStartableOnly =
      (runtimeStatus != null && !runtimeStatus.healthCheckConfigured && runtimeStatus.startCommandConfigured) ||
      (runtimeStatus == null && service.health_check == null && service.start_command != null);
    const runtimeStatusClass = statusIsStartableOnly ? "startable" : runtimeStatus?.status ?? "unknown";
    const runtimeStatusLabel = statusIsStartableOnly
      ? "可启动"
      : formatRegisteredServiceStatus(runtimeStatus?.status ?? "unknown");
    return {
      status: runtimeStatus,
      statusClass: runtimeStatusClass,
      statusLabel: runtimeStatusLabel,
      detail: formatRegisteredServiceDetail(service, runtimeStatus)
    };
  }

export function ServiceRuntimePanel(props: ServiceRuntimePanelProps) {
  const { isSystem, refreshRegisteredServiceStatuses, registeredServiceStatuses, service, serviceIndex, serviceStartBusy, startRegisteredService, stopRegisteredService, updateService } = props;
    const serviceRuntimeView = (current: UiServiceConfig) => resolveServiceRuntimeView(current, registeredServiceStatuses);
    const runtimeView = serviceRuntimeView(service);
    const serviceIsHealthy = runtimeView.status?.status === "healthy";
    const canStopService = service.stop_command != null;

    return (
      <div className="registered-service-runtime">
        <div className="registered-service-runtime-main">
          <div>
            <strong>运行</strong>
            <p>{runtimeView.detail}</p>
            {runtimeView.status ? (
              <span className="registered-service-runtime-meta">
                上次检查 {formatTime(runtimeView.status.checkedAtMs)}
              </span>
            ) : null}
          </div>
          {runtimeView.statusLabel && runtimeView.statusClass ? (
            <div className={`status-pill status-${runtimeView.statusClass}`}>
              {runtimeView.statusLabel}
            </div>
          ) : null}
        </div>
        <div className="registered-service-runtime-actions">
          {service.health_check ? (
            <button className="ghost" onClick={() => void refreshRegisteredServiceStatuses()}>
              检查状态
            </button>
          ) : !isSystem ? (
            <button
              className="ghost"
              onClick={() =>
                updateService(serviceIndex, (current) => ({
                  ...current,
                  health_check: createServiceHealthCheck()
                }))
              }
            >
              配置检查
            </button>
          ) : null}
          {serviceIsHealthy && canStopService ? (
            <button
              className="secondary danger"
              onClick={() => void stopRegisteredService(service.name)}
              disabled={serviceStartBusy != null}
            >
              {serviceStartBusy === service.name ? "停止中" : "停止应用"}
            </button>
          ) : service.start_command ? (
            <button
              className="secondary"
              onClick={() => void startRegisteredService(service.name)}
              disabled={serviceStartBusy != null}
            >
              {serviceStartBusy === service.name ? "启动中" : "启动应用"}
            </button>
          ) : !isSystem ? (
            <button
              className="secondary"
              onClick={() =>
                updateService(serviceIndex, (current) => ({
                  ...current,
                  start_command: createServiceStartCommand()
                }))
              }
            >
              配置启动
            </button>
          ) : null}
        </div>
      </div>
    );
  }

export function ComputerPermissionPanel(props: ComputerPermissionPanelProps) {
  const { desktopPermissionBusy, desktopPermissions, openDesktopPermissionSettings, requestDesktopPermission, service } = props;
    if (!service.enabled) {
      return null;
    }

    return (
      <div className="service-permission-panel">
        <div className="runtime-config-head">
          <div>
            <strong>桌面控制权限</strong>
            <small>桌面控制启用后，截图需要屏幕录制；点击、输入和拖拽需要辅助功能。</small>
          </div>
        </div>
        <div className="status-detail-grid">
          <InfoRow
            label="屏幕录制"
            value={formatDesktopPermissionValue(desktopPermissions, "screen_recording", "用于截图")}
            tone={
              desktopPermissions?.screenRecordingSupported &&
              !desktopPermissions.screenRecordingGranted
                ? "danger"
                : "normal"
            }
          />
          <InfoRow
            label="辅助功能"
            value={formatDesktopPermissionValue(desktopPermissions, "accessibility", "用于点击、输入和拖拽")}
            tone={
              desktopPermissions?.accessibilitySupported &&
              !desktopPermissions.accessibilityGranted
                ? "danger"
                : "normal"
            }
          />
        </div>
        {desktopPermissions?.platform === "macos" ? (
          <div className="permission-actions">
            {!desktopPermissions.screenRecordingGranted ? (
              <button
                className="secondary"
                onClick={() => void requestDesktopPermission("screen_recording")}
                disabled={desktopPermissionBusy != null}
              >
                {desktopPermissionBusy === "screen_recording" ? "请求中" : "请求屏幕录制"}
              </button>
            ) : null}
            {!desktopPermissions.accessibilityGranted ? (
              <button
                className="secondary"
                onClick={() => void requestDesktopPermission("accessibility")}
                disabled={desktopPermissionBusy != null}
              >
                {desktopPermissionBusy === "accessibility" ? "请求中" : "请求辅助功能"}
              </button>
            ) : null}
            {!desktopPermissions.screenRecordingGranted ? (
              <button
                className="ghost"
                onClick={() => void openDesktopPermissionSettings("screen_recording")}
              >
                打开屏幕录制设置
              </button>
            ) : null}
            {!desktopPermissions.accessibilityGranted ? (
              <button
                className="ghost"
                onClick={() => void openDesktopPermissionSettings("accessibility")}
              >
                打开辅助功能设置
              </button>
            ) : null}
          </div>
        ) : null}
      </div>
    );
  }

export function ConnectorPermissionPanel(props: ConnectorPermissionPanelProps) {
  const { connector, desktopPermissions, openDesktopPermissionSettings } = props;
    const platform = desktopPermissions?.platform;
    const permissions = (connector.permissions ?? []).filter(
      (permission) =>
        permission.platforms.length === 0 || !platform || permission.platforms.includes(platform)
    );
    if (permissions.length === 0) {
      return null;
    }

    return (
      <div className="service-permission-panel">
        <div className="runtime-config-head">
          <div>
            <strong>本机权限</strong>
            <small>这些权限授予百积木桌面应用，由它作为 Connector 的可信启动宿主。</small>
          </div>
        </div>
        <div className="status-detail-grid">
          {permissions.map((permission) => (
            <InfoRow
              key={permission.id}
              label={permission.title}
              value={permission.description || "需要在系统设置中授权"}
            />
          ))}
        </div>
        {permissions.some((permission) => permission.id === "macos.fullDiskAccess") ? (
          <div className="permission-actions">
            <button
              className="secondary"
              onClick={() => void openDesktopPermissionSettings("full_disk_access")}
            >
              打开完全磁盘访问设置
            </button>
            <small>请把“百积木”加入并开启，随后重启百积木，再从这里启动应用。</small>
          </div>
        ) : null}
      </div>
    );
  }

export function ServiceDefinitionJsonPanel(props: ServiceDefinitionJsonPanelProps) {
  const { applyServiceJsonDraft, resetServiceJsonDraft, service, serviceIndex, serviceJsonDrafts, serviceJsonErrors, updateServiceJsonDraft } = props;
    const draft = serviceJsonDrafts[serviceIndex] ?? serviceCapabilitiesJson(service);
    const error = serviceJsonErrors[serviceIndex];

    return (
      <div className="service-json-section">
        <div className="method-advanced-head">
          <strong>能力定义 JSON</strong>
          <small>统一编辑 methods；保存应用时会按同一套配置校验写入。</small>
        </div>
        <textarea
          className="json-editor"
          rows={18}
          value={draft}
          onChange={(event) => updateServiceJsonDraft(serviceIndex, event.target.value)}
        />
        {error ? <div className="json-error">{error}</div> : null}
        <div className="json-editor-actions">
          <button className="secondary" onClick={() => applyServiceJsonDraft(serviceIndex)}>
            应用 JSON
          </button>
          <button className="ghost" onClick={() => resetServiceJsonDraft(serviceIndex)}>
            重置
          </button>
        </div>
      </div>
    );
  }
