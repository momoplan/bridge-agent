import { isFullShellAccess } from "./config-conversion";
import { Field } from "./ui-primitives";
import type { UiMethodConfig } from "./types";

interface CapabilityMethodConfigPanelProps {
  grantFullShellAccess: (serviceIndex: number, methodIndex: number) => void;
  isMethodAdvancedOpen: (serviceIndex: number, methodIndex: number) => boolean;
  method: UiMethodConfig;
  methodIndex: number;
  restoreSafeShellAccess: (serviceIndex: number, methodIndex: number) => void;
  serviceIndex: number;
  updateMethod: (serviceIndex: number, methodIndex: number, updater: (method: UiMethodConfig) => UiMethodConfig) => void;
}

export function CapabilityMethodConfigPanel(props: CapabilityMethodConfigPanelProps) {
  const { grantFullShellAccess, isMethodAdvancedOpen, method, methodIndex, restoreSafeShellAccess, serviceIndex, updateMethod } = props;
    if (!isMethodAdvancedOpen(serviceIndex, methodIndex)) {
      return null;
    }
    const configuresShellExecution =
      method.binding.type === "shell_command" &&
      ["exec", "startExecution"].includes(method.name);

    return (
      <div className="method-advanced capability-config-panel">
        <div className="method-advanced-head">
          <strong>方法配置</strong>
          <small>这里直接配置当前能力的方法权限和本地绑定。</small>
        </div>
        <div className="form-grid">
          <Field label="方法状态">
            <label className="switch inline-switch">
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
          </Field>

          {method.binding.type === "computer_use" ? (
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
          ) : null}

          {method.binding.type === "shell_command" ? (
            <>
              {configuresShellExecution ? (
                <>
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
                  <Field label="根目录" hint="默认相对配置目录，填 / 表示整机根目录。">
                    <input
                      value={method.binding.root_dir}
                      onChange={(event) =>
                        updateMethod(serviceIndex, methodIndex, (current) => ({
                          ...current,
                          binding:
                            current.binding.type === "shell_command"
                              ? {
                                  ...current.binding,
                                  root_dir: event.target.value
                                }
                              : current.binding
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
                          binding:
                            current.binding.type === "shell_command"
                              ? {
                                  ...current.binding,
                                  allow_commands_text: event.target.value
                                }
                              : current.binding
                        }))
                      }
                      placeholder="echo, pwd, git 或 *"
                    />
                  </Field>
                </>
              ) : null}
              <Field label="默认超时">
                <input
                  value={method.binding.default_timeout_secs}
                  onChange={(event) =>
                    updateMethod(serviceIndex, methodIndex, (current) => ({
                      ...current,
                      binding:
                        current.binding.type === "shell_command"
                          ? {
                              ...current.binding,
                              default_timeout_secs: event.target.value
                            }
                          : current.binding
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
                      binding:
                        current.binding.type === "shell_command"
                          ? {
                              ...current.binding,
                              max_timeout_secs: event.target.value
                            }
                          : current.binding
                    }))
                  }
                  placeholder="留空则使用全局"
                />
              </Field>
            </>
          ) : null}

          {method.binding.type === "http" ? (
            <>
              <Field label="本地 URL" wide>
                <input
                  value={method.binding.url}
                  onChange={(event) =>
                    updateMethod(serviceIndex, methodIndex, (current) => ({
                      ...current,
                      binding:
                        current.binding.type === "http"
                          ? {
                              ...current.binding,
                              url: event.target.value
                            }
                          : current.binding
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
                      binding:
                        current.binding.type === "http"
                          ? {
                              ...current.binding,
                              http_method: event.target.value.toUpperCase()
                            }
                          : current.binding
                    }))
                  }
                />
              </Field>
              <Field label="超时">
                <input
                  value={method.binding.timeout_secs}
                  onChange={(event) =>
                    updateMethod(serviceIndex, methodIndex, (current) => ({
                      ...current,
                      binding:
                        current.binding.type === "http"
                          ? {
                              ...current.binding,
                              timeout_secs: event.target.value
                            }
                          : current.binding
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
                      binding:
                        current.binding.type === "http"
                          ? {
                              ...current.binding,
                              headers_text: event.target.value
                            }
                          : current.binding
                    }))
                  }
                  placeholder={"Authorization: Bearer xxx\nX-App: local-java"}
                />
              </Field>
            </>
          ) : null}
        </div>
      </div>
    );
  }
