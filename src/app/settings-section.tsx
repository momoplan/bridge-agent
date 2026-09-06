import type { Dispatch, SetStateAction } from "react";
import { DEFAULT_INLINE_LIMIT_BYTES, DEFAULT_PLATFORM_BASE_URL } from "./constants";
import { emptyToNull } from "./config-conversion";
import { safeNumber } from "./formatters";
import { Field } from "./ui-primitives";
import type { PythonRuntimeStatus, RelayConfig, RuntimeConfig, SettingsSection, UiAgentConfig } from "./types";

interface SettingsSectionProps {
  activeSettingsSection: SettingsSection;
  config: UiAgentConfig | null;
  configPath: string;
  openExternalUrl: (url: string) => Promise<void>;
  pythonCheckBusy: boolean;
  pythonStatus: PythonRuntimeStatus | null;
  refreshPythonRuntime: (pythonPath?: string | null) => Promise<void>;
  setPythonStatus: Dispatch<SetStateAction<PythonRuntimeStatus | null>>;
  setShowAdvancedSettings: Dispatch<SetStateAction<boolean>>;
  showAdvancedSettings: boolean;
  updateDevice: <K extends "name" | "description" | "tags_text">(key: K, value: UiAgentConfig["device"][K]) => void;
  updatePlatform: <K extends "base_url" | "workspace_id">(key: K, value: UiAgentConfig["platform"][K]) => void;
  updateRelay: <K extends keyof RelayConfig>(key: K, value: RelayConfig[K]) => void;
  updateRuntime: <K extends keyof RuntimeConfig>(key: K, value: RuntimeConfig[K]) => void;
  updateUpload: <K extends "prepare_url" | "inline_limit_bytes" | "timeout_secs">(key: K, value: UiAgentConfig["upload"][K]) => void;
  useDetectedPython: () => void;
}

export function SettingsSectionPanel(props: SettingsSectionProps) {
  const { activeSettingsSection, config, configPath, openExternalUrl, pythonCheckBusy, pythonStatus, refreshPythonRuntime, setPythonStatus, setShowAdvancedSettings, showAdvancedSettings, updateDevice, updatePlatform, updateRelay, updateRuntime, updateUpload, useDetectedPython } = props;
    if (!config) {
      return <div />;
    }

    switch (activeSettingsSection) {
      case "identity":
        return (
          <div className="form-grid">
            <Field label="设备名称" hint="显示给平台和授权页。">
              <input
                value={config.device.name}
                onChange={(event) => updateDevice("name", event.target.value)}
              />
            </Field>
            <Field label="运行名称" hint="Relay 侧的唯一标识。默认会自动生成唯一值，不建议多台机器共用。">
              <input
                value={config.relay.agent_id}
                onChange={(event) => updateRelay("agent_id", event.target.value)}
              />
            </Field>
            <Field label="设备标签">
              <input
                value={config.device.tags_text}
                onChange={(event) => updateDevice("tags_text", event.target.value)}
                placeholder="desktop, local"
              />
            </Field>
            <Field label="配置文件" hint="相对路径都基于这里解析。">
              <input value={configPath} readOnly />
            </Field>
            <Field label="设备描述" wide>
              <textarea
                rows={3}
                value={config.device.description}
                onChange={(event) => updateDevice("description", event.target.value)}
              />
            </Field>
          </div>
        );
      case "connection":
        return (
          <>
            <div className="section-inline-actions">
              <button
                className="secondary"
                onClick={() => setShowAdvancedSettings((current) => !current)}
              >
                {showAdvancedSettings ? "收起高级设置" : "展开高级设置"}
              </button>
            </div>
            <div className="form-grid">
              <Field
                label="默认平台"
                hint="默认使用正式环境。"
              >
                <input value={DEFAULT_PLATFORM_BASE_URL} readOnly />
              </Field>
              <Field
                label="授权后工作区"
                hint="浏览器授权成功后自动写回。"
              >
                <input
                  value={config.platform.workspace_id || ""}
                  readOnly
                  placeholder="在浏览器授权页里选择后自动写回"
                />
              </Field>
              {showAdvancedSettings ? (
                <Field
                  label="Baijimu Base URL"
                  hint="仅在测试环境修改。"
                  wide
                >
                  <input
                    value={config.platform.base_url}
                    onChange={(event) => updatePlatform("base_url", event.target.value)}
                    placeholder={DEFAULT_PLATFORM_BASE_URL}
                  />
                </Field>
              ) : null}
              <Field label="Relay WebSocket URL">
                <input
                  value={config.relay.url}
                  onChange={(event) => updateRelay("url", event.target.value)}
                />
              </Field>
              <Field
                label="设备凭证"
                hint="凭证保存在操作系统安全存储中，不会进入配置文件或前端页面。"
              >
                <input
                  value={config.credential_status.relay_token_configured ? "已安全保存" : "未授权"}
                  readOnly
                />
              </Field>
              <Field label="重连秒数">
                <input
                  type="number"
                  min={1}
                  value={config.relay.reconnect_secs}
                  onChange={(event) =>
                    updateRelay("reconnect_secs", safeNumber(event.target.value, 3))
                  }
                />
              </Field>
              <Field
                label="上传准备接口"
                wide
                hint="大截图会先调用这个接口申请上传槽位。留空时默认使用 relay 同域的 /api/bridge-agent/uploads/prepare。"
              >
                <input
                  value={config.upload.prepare_url}
                  onChange={(event) => updateUpload("prepare_url", event.target.value)}
                  placeholder="https://relay.baijimu.com/api/bridge-agent/uploads/prepare"
                />
              </Field>
              <Field
                label="内联上限字节"
                hint="截图超过这个阈值后不再直接走 WebSocket 内联。"
              >
                <input
                  type="number"
                  min={1024}
                  value={config.upload.inline_limit_bytes}
                  onChange={(event) =>
                    updateUpload("inline_limit_bytes", safeNumber(event.target.value, DEFAULT_INLINE_LIMIT_BYTES))
                  }
                />
              </Field>
              <Field label="上传超时秒数">
                <input
                  type="number"
                  min={1}
                  value={config.upload.timeout_secs}
                  onChange={(event) =>
                    updateUpload("timeout_secs", safeNumber(event.target.value, 60))
                  }
                />
              </Field>
            </div>
          </>
        );
      case "runtime":
        return (
          <div className="form-grid">
            <Field
              label="首选 Python 路径"
              wide
              hint="可留空；只有启动 Python Connector 或手动点击检测时才会发现解释器。每个 Connector 通过 requires-python 声明版本要求，并使用独立虚拟环境。"
            >
              <div className="python-runtime-fields">
              <input
                value={config.runtime.python_path ?? ""}
                onChange={(event) => {
                  updateRuntime("python_path", emptyToNull(event.target.value));
                  setPythonStatus(null);
                }}
                placeholder="例如 /opt/homebrew/bin/python3 或 C:\Python\python.exe"
              />
              <div className="python-runtime-panel">
                <span className={`status-pill status-${pythonStatus?.available ? "online" : "stopped"}`}>
                  {pythonStatus == null ? "按需检测" : pythonStatus.available ? "Python 可用" : "Python 未就绪"}
                </span>
                <span>
                  {pythonStatus?.message ?? "点击“检测 Python”检查首选路径或自动发现已安装的 Python。"}
                </span>
              </div>
              <div className="python-runtime-actions">
                <button
                  type="button"
                  className="secondary"
                  onClick={() => void refreshPythonRuntime(config.runtime.python_path)}
                  disabled={pythonCheckBusy}
                >
                  {pythonCheckBusy ? "检测中…" : "检测 Python"}
                </button>
                {pythonStatus?.available &&
                pythonStatus.detectedPath &&
                config.runtime.python_path !== pythonStatus.detectedPath ? (
                  <button type="button" className="secondary" onClick={useDetectedPython}>
                    使用检测路径
                  </button>
                ) : null}
                <button
                  type="button"
                  className="secondary"
                  onClick={() =>
                    void openExternalUrl("https://www.python.org/downloads/")
                  }
                >
                  安装 Python
                </button>
              </div>
              {pythonStatus?.detectedPath ? (
                <code className="python-runtime-path">{pythonStatus.detectedPath}</code>
              ) : null}
              </div>
            </Field>
            <Field label="Node 路径" hint="留空时自动从 PATH、登录 shell 和桌面 App 内置 runtime 查找。">
              <input
                value={config.runtime.node_path ?? ""}
                onChange={(event) => updateRuntime("node_path", emptyToNull(event.target.value))}
                placeholder="/opt/homebrew/bin/node"
              />
            </Field>
            <Field label="默认超时秒数">
              <input
                type="number"
                min={1}
                value={config.runtime.default_timeout_secs}
                onChange={(event) =>
                  updateRuntime("default_timeout_secs", safeNumber(event.target.value, 30))
                }
              />
            </Field>
            <Field label="最大超时秒数">
              <input
                type="number"
                min={1}
                value={config.runtime.max_timeout_secs}
                onChange={(event) =>
                  updateRuntime("max_timeout_secs", safeNumber(event.target.value, 120))
                }
              />
            </Field>
            <Field label="日志上限" hint="仅保留本地日志。">
              <input
                type="number"
                min={50}
                value={config.runtime.log_limit}
                onChange={(event) =>
                  updateRuntime("log_limit", safeNumber(event.target.value, 500))
                }
              />
            </Field>
            <Field label="文件日志">
              <label className="checkbox-row">
                <input
                  type="checkbox"
                  checked={config.runtime.log_file_enabled}
                  onChange={(event) => updateRuntime("log_file_enabled", event.target.checked)}
                />
                启用
              </label>
            </Field>
            <Field label="日志目录" hint="留空使用系统默认目录。">
              <input
                value={config.runtime.log_file_dir ?? ""}
                onChange={(event) => updateRuntime("log_file_dir", emptyToNull(event.target.value))}
                placeholder="C:\\ProgramData\\Baijimu\\BridgeAgent\\logs"
              />
            </Field>
            <Field label="单文件上限字节">
              <input
                type="number"
                min={1024}
                value={config.runtime.log_file_max_bytes}
                onChange={(event) =>
                  updateRuntime("log_file_max_bytes", safeNumber(event.target.value, 5 * 1024 * 1024))
                }
              />
            </Field>
            <Field label="轮转文件数">
              <input
                type="number"
                min={1}
                value={config.runtime.log_file_max_files}
                onChange={(event) =>
                  updateRuntime("log_file_max_files", safeNumber(event.target.value, 5))
                }
              />
            </Field>
            <Field label="本地事件入口">
              <label className="checkbox-row">
                <input
                  type="checkbox"
                  checked={config.runtime.event_server_enabled}
                  onChange={(event) => updateRuntime("event_server_enabled", event.target.checked)}
                />
                启用
              </label>
            </Field>
            <Field label="事件入口监听地址">
              <input
                value={config.runtime.event_server_bind}
                onChange={(event) => updateRuntime("event_server_bind", event.target.value)}
                placeholder="127.0.0.1:18081"
              />
            </Field>
          </div>
        );
    }
  }
