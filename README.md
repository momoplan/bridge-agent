# bridge-agent

`bridge-agent` 现在是一个完整的本地端工程，不再只是单个 CLI。

它包含三层：

- Rust core library：负责配置、服务注册、WebSocket 长连、调用转发、日志和本地安全策略
- CLI：适合服务器、脚本或纯命令行场景
- Tauri desktop app：适合最终用户安装、管理本地服务并打包分发

## 当前能力

- 通过 WebSocket 主动连接 relay
- 上报最小协议 `agent_id + services[]`
- 按 `service + method + arguments` 接收调用
- 本地配置里支持两种方法绑定
  - `shell_command`
  - `http`
- 本地管理端可编辑服务、方法、超时、allowlist、日志保留等配置
- 可打包为桌面应用分发

注意：

- `shell/http` 只是本地实现细节，不在 agent-relay 协议里暴露
- relay 看到的是 `services[].methods[]`，例如 `computer.exec`
- 桌面端已经接了浏览器授权启动和轮询，授权成功后会把 `agent token` 自动写回本地配置

## 项目结构

- `src/lib.rs`
- `src/config.rs`
- `src/runtime.rs`
- `src/services.rs`
- `src/main.rs`
- `src-tauri/src/main.rs`
- `src/App.tsx`

## 本地配置模型

默认配置文件会写到系统配置目录下的 `agent-config.json`。

示例配置可以用下面命令生成：

```bash
cargo run -- init-config
```

核心字段：

- `platform.base_url`
- `platform.workspace_id`
- `relay.url`
- `relay.agent_id`
- `relay.token`
- `runtime.default_timeout_secs`
- `services[].methods[].binding`

`binding.type` 只存在于本地配置里，用来决定本机怎么执行方法，不会进入 relay 协议。

## CLI

初始化配置：

```bash
cargo run -- init-config
```

打印示例配置：

```bash
cargo run -- print-example-config
```

启动 agent：

```bash
cargo run -- run
```

也可以指定配置文件：

```bash
cargo run -- run --config /path/to/agent-config.json
```

## 桌面应用开发

安装前端依赖：

```bash
npm install
```

启动桌面开发版：

```bash
npm run tauri dev
```

构建前端：

```bash
npm run build
```

## 打包分发

调试打包：

```bash
npm run tauri build -- --debug
```

本机验证过的产物路径：

- `src-tauri/target/debug/bundle/macos/Bridge Agent.app`
- `src-tauri/target/debug/bundle/dmg/Bridge Agent_0.1.0_x64.dmg`

后续如果要做 Windows / Linux 分发，直接在对应平台执行同样的 `tauri build` 即可。

## 方法绑定

### 1. `shell_command`

适合电脑控制类服务，例如：

- `computer.exec`

本地策略包括：

- `root_dir`
- `allow_commands`
- 超时限制
- 环境变量白名单

### 2. `http`

适合把本地 Java / Node / Python 服务映射成业务方法，例如：

- `local-java-service.invokeApi`

当前行为：

- `POST/PUT/PATCH`：把 `arguments` 作为 JSON body 转发
- `GET/DELETE`：把 `arguments` 转成 query string
- 返回状态码、响应头和响应体

## 安全边界

- 本地机器不开放入站端口给外网
- 所有调用都通过本地 agent 主动外连 relay
- shell 方法必须显式 allowlist
- cwd 不能逃逸 root_dir
- 每个方法调用都有超时

如果要进一步提高隔离级别，仍然建议搭配单独用户、容器或系统沙箱使用。
