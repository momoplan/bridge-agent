import type { ReactNode } from "react";
import { COMPUTER_METHOD_PRESETS, HTTP_SCHEMA, SHELL_SCHEMA } from "./constants";
import { createHttpMethod, createShellMethod, describeMethodBinding, formatMethodTypeLabel, isComputerService, isFullShellAccess, isSystemService } from "./config-conversion";
import { prettyJson, serviceSignature } from "./formatters";
import { Card, Field } from "./ui-primitives";
import type { UiMethodConfig, UiServiceConfig } from "./types";

interface ServiceEditorPanelProps {
  busy: boolean;
  deleteSavedService: (serviceIndex: number, applyToRuntime?: boolean) => Promise<void>;
  grantFullShellAccess: (serviceIndex: number, methodIndex: number) => void;
  isMethodAdvancedOpen: (serviceIndex: number, methodIndex: number) => boolean;
  removeMethod: (serviceIndex: number, methodIndex: number) => void;
  removeService: (serviceIndex: number) => void;
  renderComputerPermissionPanel: (service: UiServiceConfig) => ReactNode;
  renderServiceDefinitionJson: (service: UiServiceConfig, serviceIndex: number) => ReactNode;
  renderServiceRuntimeConfig: (service: UiServiceConfig, serviceIndex: number, isSystem: boolean) => ReactNode;
  renderServiceRuntimePanel: (service: UiServiceConfig, serviceIndex: number, isSystem: boolean) => ReactNode;
  restoreSafeShellAccess: (serviceIndex: number, methodIndex: number) => void;
  saveService: (serviceIndex: number, applyToRuntime?: boolean) => Promise<void>;
  savedServiceSignatures: string[];
  service: UiServiceConfig;
  serviceIndex: number;
  serviceNotices: Record<number, string>;
  toggleMethodAdvanced: (serviceIndex: number, methodIndex: number) => void;
  updateMethod: (serviceIndex: number, methodIndex: number, updater: (method: UiMethodConfig) => UiMethodConfig) => void;
  updateService: (serviceIndex: number, updater: (service: UiServiceConfig) => UiServiceConfig) => void;
}

export function ServiceEditorPanel(props: ServiceEditorPanelProps) {
  const { busy, deleteSavedService, grantFullShellAccess, isMethodAdvancedOpen, removeMethod, removeService, renderComputerPermissionPanel, renderServiceDefinitionJson, renderServiceRuntimeConfig, renderServiceRuntimePanel, restoreSafeShellAccess, saveService, savedServiceSignatures, service, serviceIndex, serviceNotices, toggleMethodAdvanced, updateMethod, updateService } = props;
    const isComputer = isComputerService(service);
    const isSystem = isSystemService(service);
    const serviceDirty = savedServiceSignatures[serviceIndex] !== serviceSignature(service);
    const servicePersisted = savedServiceSignatures[serviceIndex] != null;
    const hasRuntimeControls =
      service.health_check != null || service.start_command != null || service.stop_command != null;
    const serviceNotice = serviceNotices[serviceIndex];

    return (
      <Card
        title={service.name || "未命名应用"}
        description={
          isSystem
            ? "系统内置"
            : service.description || "自定义本地应用"
        }
        action={
          <div className="service-actions">
            <span className={`service-save-state ${serviceDirty ? "dirty" : "clean"}`}>
              {serviceDirty ? "未保存" : "已保存"}
            </span>
            <label className="switch">
              <input
                type="checkbox"
                checked={service.enabled}
                onChange={(event) =>
                  updateService(serviceIndex, (current) => ({
                    ...current,
                    enabled: event.target.checked
                  }))
                }
              />
              启用
            </label>
            <button className="secondary" onClick={() => void saveService(serviceIndex)} disabled={busy}>
              保存配置
            </button>
            <button className="primary" onClick={() => void saveService(serviceIndex, true)} disabled={busy}>
              保存并应用
            </button>
            {!isSystem ? (
              <button
                className="ghost danger"
                onClick={() =>
                  servicePersisted
                    ? void deleteSavedService(serviceIndex, true)
                    : removeService(serviceIndex)
                }
                disabled={busy}
              >
                {servicePersisted ? "删除并应用" : "移除草稿"}
              </button>
            ) : null}
          </div>
        }
      >
        {serviceNotice ? <div className="service-local-notice">{serviceNotice}</div> : null}
        {isComputer ? renderComputerPermissionPanel(service) : null}
        {hasRuntimeControls ? renderServiceRuntimePanel(service, serviceIndex, isSystem) : null}
        <div className="service-editor-layout">
          {!isSystem ? (
            <>
              <div className="form-grid">
                <Field label="内部服务名">
                  <input
                    value={service.name}
                    onChange={(event) =>
                      updateService(serviceIndex, (current) => ({
                        ...current,
                        name: event.target.value
                      }))
                    }
                  />
                </Field>
                <Field label="应用说明">
                  <input
                    value={service.description}
                    onChange={(event) =>
                      updateService(serviceIndex, (current) => ({
                        ...current,
                        description: event.target.value
                      }))
                    }
                  />
                </Field>
              </div>
              {renderServiceRuntimeConfig(service, serviceIndex, isSystem)}
              {renderServiceDefinitionJson(service, serviceIndex)}
            </>
          ) : null}
          <div className="method-list">
            {service.methods.map((method, methodIndex) => (
              <div className="method-card" key={`${service.name}-${method.name}-${methodIndex}`}>
                <div className="method-topline">
                  <div className="method-copy">
                    <div className="method-title-row">
                      <h4>{method.name || "未命名方法"}</h4>
                      <span className="method-badge">{formatMethodTypeLabel(method.binding.type)}</span>
                      {method.enabled != null ? (
                        <span className={`service-badge ${method.enabled ? "enabled" : "disabled"}`}>
                          {method.enabled ? "启用" : "停用"}
                        </span>
                      ) : null}
                    </div>
                    <p>{describeMethodBinding(method)}</p>
                    {isComputer && method.binding.type === "computer_use" ? (
                      <div className="method-facts">
                        <span>动作：{COMPUTER_METHOD_PRESETS[method.binding.action].label}</span>
                        <span>说明：{method.description || COMPUTER_METHOD_PRESETS[method.binding.action].description}</span>
                      </div>
                    ) : null}
                  </div>
                  {isSystem && !isComputer ? (
                    <div className="service-actions">
                      <label className="switch">
                        <input
                          type="checkbox"
                          checked={method.enabled}
                          onChange={(event) =>
                            updateMethod(serviceIndex, methodIndex, (current) => ({
                              ...current,
                              enabled: event.target.checked
                            }))
                          }
                        />
                        启用
                      </label>
                      <button
                        className={isMethodAdvancedOpen(serviceIndex, methodIndex) ? "secondary" : "ghost"}
                        onClick={() => toggleMethodAdvanced(serviceIndex, methodIndex)}
                      >
                        {isMethodAdvancedOpen(serviceIndex, methodIndex) ? "收起高级设置" : "高级设置"}
                      </button>
                      {!isSystem ? (
                        <button
                          className="ghost danger"
                          onClick={() => removeMethod(serviceIndex, methodIndex)}
                        >
                          删除方法
                        </button>
                      ) : null}
                    </div>
                  ) : null}
                </div>

                {isSystem && !isComputer ? (
                  <div className="form-grid">
                    {!isSystem ? (
                      <>
                        <Field label="方法名">
                          <input
                            value={method.name}
                            onChange={(event) =>
                              updateMethod(serviceIndex, methodIndex, (current) => ({
                                ...current,
                                name: event.target.value
                              }))
                            }
                          />
                        </Field>
                        <Field label="方法描述">
                          <input
                            value={method.description}
                            onChange={(event) =>
                              updateMethod(serviceIndex, methodIndex, (current) => ({
                                ...current,
                                description: event.target.value
                              }))
                            }
                          />
                        </Field>
                        <Field label="绑定类型">
                          <select
                            value={method.binding.type}
                            disabled={method.binding.type === "computer_use"}
                            onChange={(event) =>
                              updateMethod(serviceIndex, methodIndex, (current) => ({
                                ...current,
                                input_schema_text:
                                  event.target.value === "shell_command"
                                    ? prettyJson(SHELL_SCHEMA)
                                    : prettyJson(HTTP_SCHEMA),
                                binding:
                                  event.target.value === "shell_command"
                                    ? createShellMethod().binding
                                    : createHttpMethod().binding
                              }))
                            }
                          >
                            {method.binding.type === "computer_use" ? (
                              <option value="computer_use">computer_use（内置）</option>
                            ) : null}
                            <option value="shell_command">shell_command</option>
                            <option value="http">http</option>
                          </select>
                        </Field>
                      </>
                    ) : null}

                    {method.binding.type === "computer_use" ? (
                      <Field
                        label="桌面控制能力"
                        hint="内置能力由系统应用维护，不在自定义应用里编辑。"
                      >
                        <input value={COMPUTER_METHOD_PRESETS[method.binding.action].label} readOnly />
                      </Field>
                    ) : method.binding.type === "shell_command" ? (
                      <Field label="权限模式">
                        <div className="mode-toggle">
                          <button
                            className={!isFullShellAccess(method.binding) ? "secondary active-toggle" : "ghost"}
                            onClick={() => restoreSafeShellAccess(serviceIndex, methodIndex)}
                          >
                            受限
                          </button>
                          <button
                            className={isFullShellAccess(method.binding) ? "secondary active-toggle" : "ghost"}
                            onClick={() => grantFullShellAccess(serviceIndex, methodIndex)}
                          >
                            全部权限
                          </button>
                        </div>
                      </Field>
                    ) : (
                      <>
                        <Field label="本地 URL" wide>
                          <input
                            value={method.binding.url}
                            onChange={(event) =>
                              updateMethod(serviceIndex, methodIndex, (current) => ({
                                ...current,
                                binding: {
                                  ...current.binding,
                                  url: event.target.value
                                }
                              }))
                            }
                          />
                        </Field>
                        <Field label="HTTP 方法">
                          <input
                            value={method.binding.http_method}
                            onChange={(event) =>
                              updateMethod(serviceIndex, methodIndex, (current) => ({
                                ...current,
                                binding: {
                                  ...current.binding,
                                  http_method: event.target.value.toUpperCase()
                                }
                              }))
                            }
                          />
                        </Field>
                      </>
                    )}
                  </div>
                ) : null}

                {isSystem && !isComputer && isMethodAdvancedOpen(serviceIndex, methodIndex) ? (
                  <div className="method-advanced">
                    <div className="method-advanced-head">
                      <strong>高级设置</strong>
                      <small>只有在需要细调本地实现时才展开这里。</small>
                    </div>

                    {method.binding.type === "shell_command" ? (
                      <>
                        <div className="permission-banner compact-banner">
                          <div>
                            <strong>
                              {isFullShellAccess(method.binding) ? "当前是全部权限模式" : "当前是受限模式"}
                            </strong>
                            <p>这里决定命令白名单、根目录和超时上限。</p>
                          </div>
                        </div>
                        <div className="form-grid">
                          <Field label="根目录" hint="默认相对配置目录，填 / 表示整机根目录。">
                            <input
                              value={method.binding.root_dir}
                              onChange={(event) =>
                                updateMethod(serviceIndex, methodIndex, (current) => ({
                                  ...current,
                                  binding: {
                                    ...current.binding,
                                    root_dir: event.target.value
                                  }
                                }))
                              }
                            />
                          </Field>
                          <Field label="允许命令" hint="逗号分隔；填 * 表示任意命令。">
                            <input
                              value={method.binding.allow_commands_text}
                              onChange={(event) =>
                                updateMethod(serviceIndex, methodIndex, (current) => ({
                                  ...current,
                                  binding: {
                                    ...current.binding,
                                    allow_commands_text: event.target.value
                                  }
                                }))
                              }
                              placeholder="echo, pwd, git 或 *"
                            />
                          </Field>
                          <Field label="默认超时">
                            <input
                              value={method.binding.default_timeout_secs}
                              onChange={(event) =>
                                updateMethod(serviceIndex, methodIndex, (current) => ({
                                  ...current,
                                  binding: {
                                    ...current.binding,
                                    default_timeout_secs: event.target.value
                                  }
                                }))
                              }
                              placeholder="留空则使用全局"
                            />
                          </Field>
                          <Field label="最大超时">
                            <input
                              value={method.binding.max_timeout_secs}
                              onChange={(event) =>
                                updateMethod(serviceIndex, methodIndex, (current) => ({
                                  ...current,
                                  binding: {
                                    ...current.binding,
                                    max_timeout_secs: event.target.value
                                  }
                                }))
                              }
                              placeholder="留空则使用全局"
                            />
                          </Field>
                        </div>
                      </>
                    ) : method.binding.type === "computer_use" ? (
                      <div className="form-grid">
                        <Field
                          label="显示器 ID"
                          hint="留空表示主显示器；截图时可指定其他显示器。"
                        >
                          <input
                            value={method.binding.display_id}
                            onChange={(event) =>
                              updateMethod(serviceIndex, methodIndex, (current) => ({
                                ...current,
                                binding:
                                  current.binding.type === "computer_use"
                                    ? {
                                        ...current.binding,
                                        display_id: event.target.value
                                      }
                                    : current.binding
                              }))
                            }
                            placeholder="留空使用主屏"
                          />
                        </Field>
                      </div>
                    ) : (
                      <div className="form-grid">
                        <Field label="超时">
                          <input
                            value={method.binding.timeout_secs}
                            onChange={(event) =>
                              updateMethod(serviceIndex, methodIndex, (current) => ({
                                ...current,
                                binding: {
                                  ...current.binding,
                                  timeout_secs: event.target.value
                                }
                              }))
                            }
                            placeholder="留空则使用全局"
                          />
                        </Field>
                        <Field label="请求头" wide>
                          <textarea
                            rows={4}
                            value={method.binding.headers_text}
                            onChange={(event) =>
                              updateMethod(serviceIndex, methodIndex, (current) => ({
                                ...current,
                                binding: {
                                  ...current.binding,
                                  headers_text: event.target.value
                                }
                              }))
                            }
                            placeholder={"Authorization: Bearer xxx\nX-App: local-java"}
                          />
                        </Field>
                      </div>
                    )}

                    {!isSystem ? (
                      <Field label="输入 Schema JSON" wide>
                        <textarea
                          rows={8}
                          value={method.input_schema_text}
                          onChange={(event) =>
                            updateMethod(serviceIndex, methodIndex, (current) => ({
                              ...current,
                              input_schema_text: event.target.value
                            }))
                          }
                        />
                      </Field>
                    ) : null}
                  </div>
                ) : null}
              </div>
            ))}
          </div>
        </div>
      </Card>
    );
  }
