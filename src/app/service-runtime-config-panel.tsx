import { createServiceHealthCheck, createServiceStartCommand, createServiceStopCommand } from "./config-conversion";
import { Field } from "./ui-primitives";
import type { UiServiceConfig, UiServiceHealthCheck, UiServiceStartCommand } from "./types";

interface ServiceRuntimeConfigPanelProps {
  isSystem: boolean;
  service: UiServiceConfig;
  serviceIndex: number;
  updateService: (serviceIndex: number, updater: (service: UiServiceConfig) => UiServiceConfig) => void;
  updateServiceHealthCheck: (serviceIndex: number, updater: (healthCheck: UiServiceHealthCheck) => UiServiceHealthCheck) => void;
  updateServiceStartCommand: (serviceIndex: number, updater: (startCommand: UiServiceStartCommand) => UiServiceStartCommand) => void;
  updateServiceStopCommand: (serviceIndex: number, updater: (stopCommand: UiServiceStartCommand) => UiServiceStartCommand) => void;
}

export function ServiceRuntimeConfigPanel(props: ServiceRuntimeConfigPanelProps) {
  const { isSystem, service, serviceIndex, updateService, updateServiceHealthCheck, updateServiceStartCommand, updateServiceStopCommand } = props;
    if (isSystem) {
      return null;
    }

    return (
      <div className="runtime-config-section">
        <div className="method-advanced-head">
          <strong>运行配置</strong>
          <small>健康检查决定状态展示；启动/停止命令决定应用详情里的运行按钮。</small>
        </div>

        <div className="runtime-config-block">
          <div className="runtime-config-head">
            <div>
              <strong>健康检查</strong>
              <small>{service.health_check ? "已启用 HTTP 状态检查" : "未配置，无法自动判断是否可用"}</small>
            </div>
            {service.health_check ? (
              <button
                className="ghost danger"
                onClick={() =>
                  updateService(serviceIndex, (current) => ({
                    ...current,
                    health_check: null
                  }))
                }
              >
                移除检查
              </button>
            ) : (
              <button
                className="secondary"
                onClick={() =>
                  updateService(serviceIndex, (current) => ({
                    ...current,
                    health_check: createServiceHealthCheck()
                  }))
                }
              >
                添加检查
              </button>
            )}
          </div>
          {service.health_check ? (
            <div className="form-grid">
              <Field label="检查 URL" wide>
                <input
                  value={service.health_check.url}
                  onChange={(event) =>
                    updateServiceHealthCheck(serviceIndex, (current) => ({
                      ...current,
                      url: event.target.value
                    }))
                  }
                />
              </Field>
              <Field label="HTTP 方法">
                <input
                  value={service.health_check.http_method}
                  onChange={(event) =>
                    updateServiceHealthCheck(serviceIndex, (current) => ({
                      ...current,
                      http_method: event.target.value.toUpperCase()
                    }))
                  }
                />
              </Field>
              <Field label="期望状态码">
                <input
                  value={service.health_check.expect_status}
                  onChange={(event) =>
                    updateServiceHealthCheck(serviceIndex, (current) => ({
                      ...current,
                      expect_status: event.target.value
                    }))
                  }
                  placeholder="默认任意 2xx"
                />
              </Field>
              <Field label="超时秒数">
                <input
                  value={service.health_check.timeout_secs}
                  onChange={(event) =>
                    updateServiceHealthCheck(serviceIndex, (current) => ({
                      ...current,
                      timeout_secs: event.target.value
                    }))
                  }
                  placeholder="默认 3"
                />
              </Field>
              <Field label="响应包含">
                <input
                  value={service.health_check.body_contains}
                  onChange={(event) =>
                    updateServiceHealthCheck(serviceIndex, (current) => ({
                      ...current,
                      body_contains: event.target.value
                    }))
                  }
                  placeholder="可选"
                />
              </Field>
              <Field label="请求头" wide>
                <textarea
                  rows={4}
                  value={service.health_check.headers_text}
                  onChange={(event) =>
                    updateServiceHealthCheck(serviceIndex, (current) => ({
                      ...current,
                      headers_text: event.target.value
                    }))
                  }
                  placeholder={"Authorization: Bearer xxx\nX-App: local-service"}
                />
              </Field>
            </div>
          ) : null}
        </div>

        <div className="runtime-config-block">
          <div className="runtime-config-head">
            <div>
              <strong>启动命令</strong>
              <small>{service.start_command ? "已启用应用启动按钮" : "未配置，应用详情不会执行启动动作"}</small>
            </div>
            {service.start_command ? (
              <button
                className="ghost danger"
                onClick={() =>
                  updateService(serviceIndex, (current) => ({
                    ...current,
                    start_command: null
                  }))
                }
              >
                移除启动
              </button>
            ) : (
              <button
                className="secondary"
                onClick={() =>
                  updateService(serviceIndex, (current) => ({
                    ...current,
                    start_command: createServiceStartCommand()
                  }))
                }
              >
                添加启动
              </button>
            )}
          </div>
          {service.start_command ? (
            <div className="form-grid">
              <Field label="命令参数" hint="每行一个参数；第一行是可执行文件。" wide>
                <textarea
                  rows={5}
                  value={service.start_command.command_text}
                  onChange={(event) =>
                    updateServiceStartCommand(serviceIndex, (current) => ({
                      ...current,
                      command_text: event.target.value
                    }))
                  }
                  placeholder={"npm\nrun\ndev"}
                />
              </Field>
              <Field label="工作目录">
                <input
                  value={service.start_command.cwd}
                  onChange={(event) =>
                    updateServiceStartCommand(serviceIndex, (current) => ({
                      ...current,
                      cwd: event.target.value
                    }))
                  }
                  placeholder="留空继承 Agent 进程目录"
                />
              </Field>
              <Field label="超时秒数">
                <input
                  value={service.start_command.timeout_secs}
                  onChange={(event) =>
                    updateServiceStartCommand(serviceIndex, (current) => ({
                      ...current,
                      timeout_secs: event.target.value
                    }))
                  }
                  placeholder="默认 20"
                />
              </Field>
              <Field label="环境变量" wide>
                <textarea
                  rows={4}
                  value={service.start_command.env_text}
                  onChange={(event) =>
                    updateServiceStartCommand(serviceIndex, (current) => ({
                      ...current,
                      env_text: event.target.value
                    }))
                  }
                  placeholder={"NODE_ENV: development\nPORT: 8081"}
                />
              </Field>
            </div>
          ) : null}
        </div>

        <div className="runtime-config-block">
          <div className="runtime-config-head">
            <div>
              <strong>停止命令</strong>
              <small>{service.stop_command ? "运行中会显示停止应用按钮" : "未配置，运行中无法从应用详情停止"}</small>
            </div>
            {service.stop_command ? (
              <button
                className="ghost danger"
                onClick={() =>
                  updateService(serviceIndex, (current) => ({
                    ...current,
                    stop_command: null
                  }))
                }
              >
                移除停止
              </button>
            ) : (
              <button
                className="secondary"
                onClick={() =>
                  updateService(serviceIndex, (current) => ({
                    ...current,
                    stop_command: createServiceStopCommand()
                  }))
                }
              >
                添加停止
              </button>
            )}
          </div>
          {service.stop_command ? (
            <div className="form-grid">
              <Field label="命令参数" hint="每行一个参数；第一行是可执行文件。" wide>
                <textarea
                  rows={5}
                  value={service.stop_command.command_text}
                  onChange={(event) =>
                    updateServiceStopCommand(serviceIndex, (current) => ({
                      ...current,
                      command_text: event.target.value
                    }))
                  }
                  placeholder={"npm\nrun\nstop"}
                />
              </Field>
              <Field label="工作目录">
                <input
                  value={service.stop_command.cwd}
                  onChange={(event) =>
                    updateServiceStopCommand(serviceIndex, (current) => ({
                      ...current,
                      cwd: event.target.value
                    }))
                  }
                  placeholder="留空继承 Agent 进程目录"
                />
              </Field>
              <Field label="超时秒数">
                <input
                  value={service.stop_command.timeout_secs}
                  onChange={(event) =>
                    updateServiceStopCommand(serviceIndex, (current) => ({
                      ...current,
                      timeout_secs: event.target.value
                    }))
                  }
                  placeholder="默认 20"
                />
              </Field>
              <Field label="环境变量" wide>
                <textarea
                  rows={4}
                  value={service.stop_command.env_text}
                  onChange={(event) =>
                    updateServiceStopCommand(serviceIndex, (current) => ({
                      ...current,
                      env_text: event.target.value
                    }))
                  }
                  placeholder={"NODE_ENV: development\nPORT: 8081"}
                />
              </Field>
            </div>
          ) : null}
        </div>
      </div>
    );
  }
