import type { Dispatch, ReactNode, SetStateAction } from "react";
import { formatMethodTypeLabel, toUiEvent, toUiMethod, toUiServiceHealthCheck, toUiServiceStartCommand } from "./config-conversion";
import { prettyJson } from "./formatters";
import type { CapabilityTestState, LocalAppItem, UiAgentConfig, UiEventConfig, UiMethodConfig, UiServiceConfig } from "./types";

interface LocalAppAbilityListProps {
  app: LocalAppItem;
  buildCapabilityTestKey: (serviceIndex: number, methodIndex: number) => string;
  busy: boolean;
  canShowConfig: boolean;
  capabilityTestBusy: string | null;
  capabilityTestDrafts: Record<string, string>;
  capabilityTestResults: Record<string, CapabilityTestState>;
  config: UiAgentConfig | null;
  defaultCapabilityArgumentsText: (method: UiMethodConfig) => string;
  isMethodAdvancedOpen: (serviceIndex: number, methodIndex: number) => boolean;
  openLocalAppCapabilityConfig: (serviceIndex: number, methodIndex?: number) => void;
  renderCapabilityMethodConfig: (method: UiMethodConfig, serviceIndex: number, methodIndex: number) => ReactNode;
  saveService: (serviceIndex: number, applyToRuntime?: boolean) => Promise<void>;
  savedServiceSignatures: string[];
  serviceSignature: (service: UiAgentConfig["services"][number]) => string;
  setCapabilityTestDrafts: Dispatch<SetStateAction<Record<string, string>>>;
  testCapability: (serviceIndex: number, methodIndex: number) => Promise<void>;
  testLocalAppCapability: (app: LocalAppItem, methodIndex: number) => Promise<void>;
}

export function LocalAppAbilityList(props: LocalAppAbilityListProps) {
  const { app, buildCapabilityTestKey, busy, canShowConfig, capabilityTestBusy, capabilityTestDrafts, capabilityTestResults, config, defaultCapabilityArgumentsText, isMethodAdvancedOpen, openLocalAppCapabilityConfig, renderCapabilityMethodConfig, saveService, savedServiceSignatures, serviceSignature, setCapabilityTestDrafts, testCapability, testLocalAppCapability } = props;
    if (!config) {
      return <div />;
    }
    const services: Array<{
      serviceIndex: number | null;
      service: UiServiceConfig;
      localAppEvents: UiEventConfig[];
    }> = app.serviceIndexes.flatMap((serviceIndex) => {
      const service = config.services[serviceIndex];
      return service
        ? [
            {
              serviceIndex,
              service,
              localAppEvents: []
            }
          ]
        : [];
    });
    if (app.localAppIndex != null) {
      const localApp = config.local_apps[app.localAppIndex];
      if (localApp) {
        services.push({
          serviceIndex: null,
          localAppEvents: localApp.events.map(toUiEvent),
          service: {
            name: localApp.appId,
            description: localApp.description || localApp.name,
            enabled: localApp.enabled,
            health_check: localApp.healthCheck ? toUiServiceHealthCheck(localApp.healthCheck) : null,
            start_command: localApp.startCommand ? toUiServiceStartCommand(localApp.startCommand) : null,
            stop_command: localApp.stopCommand ? toUiServiceStartCommand(localApp.stopCommand) : null,
            methods: localApp.methods.map(toUiMethod)
          }
        });
      }
    }

    if (services.length === 0) {
      return <div className="empty-state compact-empty">应用已安装，但当前没有写入能力。</div>;
    }

    return (
      <div className="app-ability-list">
        {services.map(({ serviceIndex, service, localAppEvents }) => (
          <div className="app-ability-group" key={service.name}>
            <div className="app-ability-group-head">
              <div>
                <strong>{service.description || service.name}</strong>
                <span className={`service-badge ${service.enabled ? "enabled" : "disabled"}`}>
                  {service.enabled ? "启用" : "停用"}
                </span>
              </div>
              {canShowConfig && serviceIndex != null ? (
                <div className="service-actions">
                  <span
                    className={`service-save-state ${
                      savedServiceSignatures[serviceIndex] !== serviceSignature(service) ? "dirty" : "clean"
                    }`}
                  >
                    {savedServiceSignatures[serviceIndex] !== serviceSignature(service) ? "未保存" : "已保存"}
                  </span>
                  <button
                    className="secondary compact-config-button"
                    onClick={() => void saveService(serviceIndex)}
                    disabled={busy}
                  >
                    保存配置
                  </button>
                  <button
                    className="primary compact-config-button"
                    onClick={() => void saveService(serviceIndex, true)}
                    disabled={busy}
                  >
                    保存并应用
                  </button>
                </div>
              ) : null}
            </div>
            <div className="method-list compact-method-list">
              {service.methods.map((method, methodIndex) => {
                const testKey =
                  serviceIndex == null
                    ? `local-app:${service.name}:${methodIndex}`
                    : buildCapabilityTestKey(serviceIndex, methodIndex);
                const testDraft = capabilityTestDrafts[testKey] ?? defaultCapabilityArgumentsText(method);
                const testResult = capabilityTestResults[testKey];
                const testDisabled = !service.enabled || !method.enabled || capabilityTestBusy != null;
                const configOpen =
                  serviceIndex != null && isMethodAdvancedOpen(serviceIndex, methodIndex);
                return (
                  <div className="method-card compact-method-card" key={`${service.name}-${method.name}-${methodIndex}`}>
                    <div className="method-topline compact-method-topline">
                      <div className="method-copy">
                        <div className="method-title-row">
                          <h4>{method.name || "未命名方法"}</h4>
                          <span className="method-badge">{formatMethodTypeLabel(method.binding.type)}</span>
                          <span className={`service-badge ${method.enabled ? "enabled" : "disabled"}`}>
                            {method.enabled ? "启用" : "停用"}
                          </span>
                        </div>
                        <p>{method.description || "本地方法"}</p>
                      </div>
                      <div className="capability-card-actions">
                        <button
                          className="primary compact-config-button"
                          onClick={() =>
                            serviceIndex == null
                              ? void testLocalAppCapability(app, methodIndex)
                              : void testCapability(serviceIndex, methodIndex)
                          }
                          disabled={testDisabled}
                        >
                          {capabilityTestBusy === testKey ? "测试中" : "测试"}
                        </button>
                        {canShowConfig && serviceIndex != null ? (
                          <button
                            className={
                              configOpen
                                ? "secondary compact-config-button active-toggle"
                                : "secondary compact-config-button"
                            }
                            onClick={() => openLocalAppCapabilityConfig(serviceIndex, methodIndex)}
                          >
                            {configOpen ? "收起配置" : "配置"}
                          </button>
                        ) : null}
                      </div>
                    </div>
                    {canShowConfig && serviceIndex != null
                      ? renderCapabilityMethodConfig(method, serviceIndex, methodIndex)
                      : null}
                    <div className="capability-test-panel">
                      <label>
                        <span>参数 JSON</span>
                        <small>请使用标准 JSON，属性名和字符串必须用英文双引号 "，不能用中文引号 “ ”。</small>
                        <textarea
                          rows={Math.max(3, Math.min(8, testDraft.split("\n").length))}
                          value={testDraft}
                          onChange={(event) =>
                            setCapabilityTestDrafts((current) => ({
                              ...current,
                              [testKey]: event.target.value
                            }))
                          }
                        />
                      </label>
                      {testResult ? (
                        <div className={`capability-test-result ${testResult.status}`}>
                          <div className="capability-test-result-head">
                            <strong>{testResult.message}</strong>
                            {testResult.result ? <span>{testResult.result.duration_ms}ms</span> : null}
                          </div>
                          {testResult.result ? (
                            <pre>{prettyJson(testResult.result.success ? testResult.result.data ?? {} : testResult.result.error ?? {})}</pre>
                          ) : null}
                        </div>
                      ) : null}
                    </div>
                  </div>
                );
              })}
              {localAppEvents.map((eventConfig, eventIndex) => (
                <div
                  className="method-card compact-method-card"
                  key={`${service.name}-${eventConfig.name}-event-${eventIndex}`}
                >
                  <div className="method-topline compact-method-topline">
                    <div className="method-copy">
                      <div className="method-title-row">
                        <h4>{eventConfig.name || "未命名事件"}</h4>
                        <span className="method-badge">Event</span>
                        <span className={`service-badge ${eventConfig.enabled ? "enabled" : "disabled"}`}>
                          {eventConfig.enabled ? "启用" : "停用"}
                        </span>
                      </div>
                      <p>{eventConfig.description || "本地事件"}</p>
                    </div>
                  </div>
                </div>
              ))}
              {service.methods.length === 0 && localAppEvents.length === 0 ? (
                <div className="empty-state compact-empty">当前没有开放能力。</div>
              ) : null}
            </div>
          </div>
        ))}
      </div>
    );
  }
