# 百积木

百积木是安装在用户自己电脑上的本地连接客户端。当前仓库、CLI、服务二进制和部分接口路径仍沿用内部工程名 `bridge-agent`。

它的职责不是替代顶层 agent，而是把这台机器上经过用户授权的本地能力，安全地暴露给外部 agent 调用。

先把最容易混淆的一点说清楚：

- 任何人都可以使用这个开源项目
- 但如果你想让外部 agent 使用你自己电脑上的能力，你必须先在自己的机器上安装并运行百积木
- 外部 agent 不能在“用户什么都不装”的前提下直接获得本地 shell 或本地服务能力

## 下载

给最终用户分发时，使用平台自己的下载页或更新服务返回的国内下载地址：

- 最新版本页：由更新服务的 `releaseUrl` 发布数据指向平台下载页
- macOS：优先下载 universal `.dmg`
- Windows：下载 `.msi` 安装包；安装时会检测 WebView2，缺失时自动下载并安装
- Linux：下载 `.AppImage` / `.deb`

如果你只是普通用户，直接下载对应平台安装包即可，不需要本地安装 Rust 或 Node 环境。

如果你拿到的是旧版分发物，需要额外注意：

- Intel Mac 只能运行 `x64` 或 universal 的 macOS 安装包
- Apple Silicon Mac 可以运行 `arm64` 或 universal 的 macOS 安装包
- 如果误装了另一种架构，Finder 里能看到应用，但启动会失败

## 它解决什么问题

百积木解决的是“外部 agent 如何安全地调用用户自己电脑上的本地能力”这个问题。

典型场景：

- 让外部 ChatGPT / Claude 调用本机桌面控制能力，例如 `computer.screenshot` / `computer.click`
- 让外部 agent 调用本地已经存在的业务服务，例如本机 Java / Node / Python 服务
- 让本地机器不暴露公网入站端口，仍然能被远端授权访问

## 架构关系

系统里有三个角色：

- 百积木（内部工程名 `bridge-agent`）
  - 安装在用户自己的机器上
  - 管理“我这台机器对外开放哪些系统服务、本地应用、方法和事件”
- `relay`
  - 负责转发和鉴权
  - 不直接执行本地命令
- 外部 agent / app
  - 通过 `relay` 调用某个设备上的系统 `service.method`，或某个
    `connectorId + method`

调用链路：

1. 用户在本机安装并启动百积木
2. 百积木打开授权页面，用户确认授权
3. 百积木获取 `agent token`，主动连接 `relay`
4. 用户把某个系统服务或本地应用授权给外部 app / agent
5. 外部 app / agent 拿到 `client token`
6. 外部 app / agent 通过 `relay` 调用本机暴露的系统服务或本地应用方法

## 谁需要安装

- 如果你只是调用别人已经开放出来的设备能力：不需要安装 `bridge-agent`
- 如果你想让外部 agent 使用你自己电脑上的能力：需要安装百积木
- 如果你是平台运营方：需要部署 `relay`

所以它应该是一个可审计、可安装、可分发的开源本地项目，而不是一个纯云端工具。

## 当前工程形态

百积木现在是一个完整的本地端工程，不再只是单个 CLI。

它包含三层：

- Rust core library：负责系统服务、本地应用目录、WebSocket 长连、调用转发、事件 outbox、日志和本地安全策略
- CLI：适合服务器、脚本或纯命令行场景
- Tauri desktop app：适合最终用户安装、管理本地应用并打包分发

## 当前能力

- 通过 WebSocket 主动连接 relay
- 上报 `agent_id + services[] + local_apps[]`
- 系统服务按 `service + method + arguments` 调用；本地应用按
  `connectorId + method + arguments` 调用
- 大截图支持先申请上传槽位、再直传对象存储/文件服务、最后只返回文件引用
- 本地配置里支持三种方法绑定
  - `computer_use`
  - `shell_command`
  - `http`
- 本地管理端可管理本地应用生命周期；内置系统服务仍可编辑方法、超时、allowlist 和日志保留等配置
- 已接浏览器授权启动和轮询，授权成功后会把 `agent token` 自动写回本地配置
- 可打包为桌面应用分发

## 本地应用和 Connector

百积木桌面端把最终用户看到的概念统一为“本地应用”：本地能力宿主负责设备授权、relay 长连、Connector 安装、生命周期、健康检查和调用审计。

需要宿主精确控制生命周期的 Connector 使用 `runtime.processOwnership: "host"`：清单的
`args[]` 是持续运行的前台入口，桌面端持有进程句柄并在停止、升级、卸载和退出时回收
完整进程树。Windows 的运行、停止和环境准备命令统一无控制台窗口；macOS 仍由已签名的
百积木桌面宿主直接派生进程，避免把受保护数据权限漂移到临时 Python 启动器。

Codex、Claude Code、WeChat、Desktop Control 等能力不应都硬编码进宿主，而应优先作为可安装 Connector 或内置应用：

- Codex：作为独立 Rust 本地应用连接本机 `codex app-server`，同时提供结构化 session/thread/turn/event 能力和“账户与工作区”管理；LLM credential 签发、归属校验、本机配置切换均由 Codex 应用自身执行，Bridge Agent 只通过 Connector 声明的本机管理接口展示结果。
- WeChat Connector：连接本机微信采集器，注册 `wechatLocal` 查询方法和消息事件。
- Desktop Control：提供截图、点击、键盘、滚动等桌面能力，当前作为内置应用随客户端分发。

Baijimu CLI 作为“官方托管工具”显示在本地应用页：客户端内置一个首次安装基线版本，后续版本由本地应用管理器独立升级和回滚，不跟随客户端覆盖或降级。CLI 不注册远程能力，也不经过 relay。

客户端安装或修复 Baijimu CLI 时，会同时把 Codex、WorkBuddy 和钉钉悟空共用的
`baijimu-platform` Skill 幂等安装到当前用户的
`~/.agents/skills/baijimu-platform/SKILL.md`。其唯一上游是
[`momoplan/baijimu-platform-skill`](https://github.com/momoplan/baijimu-platform-skill) 的
固定发行版本，Bridge 内置文件必须通过版本和 SHA-256 校验，不再维护独立的
`baijimu-docs` 副本。安装时会把旧 `baijimu-docs` 以及旧 Codex 专属目录里的同名技能
迁移到 `~/.agents/skill-backups/`，避免重复发现。

Agent 在新任务中遇到百积木、Bundle、模块、运行时、平台应用、Connector 或 Partner
API 需求时，会先通过本机 `baijimu capabilities --offline --json` 取得版本固定的官方
文档入口，再查询 `https://docs.baijimu.com/`；不需要用户手工提供百积木规范。CLI
升级、回滚或客户端重启都会修复缺失或被修改的托管 Skill。

稳定命令在 macOS/Linux 安装到 `~/.local/bin/baijimu`，在 Windows 安装到
`%LOCALAPPDATA%\Baijimu\bin\baijimu.exe`。Windows 客户端会把该目录幂等置于当前用户
`PATH` 首位并广播环境更新，不覆盖其他 PATH 项；安装前已经启动的 Codex 或终端无法被
外部进程修改既有环境，文档发现 Skill 会在当次任务直接使用上述绝对路径，用户重启
Codex/终端后即可直接调用 `baijimu`。

Baijimu CLI 由其私有 GitHub 仓库 `momoplan/baijimu-cli-rs` 的唯一
`release.yml` 工作流独立发布。公开下载统一使用内容寻址 OSS、npm 或本地应用市场；
Bridge Agent 只按固定版本和 SHA-256 消费 OSS 制品，不再构建、签名、镜像或发布 CLI。

Codex 的运行状态与账户状态彼此独立：应用状态来自 Connector 的 health check，账户页单独显示当前工作区、项目和凭证有效性。LLM key 只由 Codex 本地应用处理，不进入 Bridge Agent、前端状态或 relay，也不会作为 `codexSession` 方法暴露给远端。

本地应用、官方托管工具和 Connector 的正式规范见 [BRIDGE_LOCAL_CONNECTOR_SPEC.md](BRIDGE_LOCAL_CONNECTOR_SPEC.md)。标准安装机制成熟后，skill 不再承担常规 Connector 安装职责，只保留诊断、权限异常处理和 legacy fallback。

Connector 安装采用分级信任：只有从应用市场选择、由后端重新读取市场记录，并通过 HTTPS、SHA-256、Connector ID 和版本一致性校验的发布包才标记为“平台信任”。本地目录、Git 仓库以及其他直接来源始终标记为“用户信任、平台未验证”，即使它们使用了与市场应用相同的 Connector ID，也不会自动获得市场身份或市场升级入口。桌面端要求安装前确认风险，后续重新同步也会再次确认；CLI 安装需显式传入 `--accept-untrusted`。安装记录会保存来源、信任等级以及安装内容摘要。

桌面端安装采用后台任务：应用市场只负责发起任务，可随时关闭；下载、校验、安装和启动进度显示在本地应用面板。Connector 声明的 `setup` 属于应用自身初始化能力，不再阻塞宿主安装；例如 Codex 的下载配置、凭证、路由验证和重试都在 Codex 应用界面中执行并展示。

运行中的桌面端会发布仅当前用户可读的本机控制发现文件，`baijimu` CLI 通过随机令牌和 loopback HTTP 管理本地应用。AI 和自动化工具不需要直接修改 `agent-config.json` 或 Connector 安装目录：

```bash
baijimu local-app device status
baijimu local-app device market
baijimu local-app install codex --market
baijimu local-app install /path/to/connector --accept-untrusted
baijimu local-app device list
baijimu local-app device get com.baijimu.connector.codex
baijimu local-app device start com.baijimu.connector.codex
baijimu local-app device stop com.baijimu.connector.codex
baijimu local-app device sync com.baijimu.connector.codex
baijimu local-app device invoke com.baijimu.connector.codex credentialState
baijimu local-app device uninstall com.baijimu.connector.codex --yes
```

CLI 默认自动发现并在需要时启动百积木桌面端；特殊部署可用 `BAIJIMU_LOCAL_APP_CONTROL_FILE` 或 `--control-file` 指定发现文件。非市场安装必须显式传 `--accept-untrusted`，卸载必须显式传 `--yes`。

## 对外暴露的模型

外部协议同时支持两类能力：内置/兼容能力继续使用 `services[]`，Connector 安装的能力
使用一等的 `local_apps[]`。本地应用不需要为每个 Connector 再创建 service，也不依赖
runtime `businessId`。

产品语境里可以把概念分成两层：

- 本地应用：用户安装、启动、卸载和授权的对象，例如 Codex Connector、WeChat Connector、桌面控制。
- 服务：内置能力和兼容自定义能力的内部对象，例如 `computer`、`shell`。
- 方法：服务下面的具体动作，例如 `screenshot`、`click`、`exec`、`queryExecution`。
- 对外能力：内置服务使用 `service.method`；本地应用使用
  `connectorId + method/event`。

所以桌面端默认以“应用”为主概念；“服务”只在开发者配置、CLI、本机注册 API 和协议说明里出现。

当前桌面端把 `computer` 和 `shell` 归到内置应用：它们由应用默认配置维护，不能删除或改名；用户可以启停应用，其中 `shell` 还允许在开发者配置里调整命令权限、根目录和超时。其他由用户新增的 HTTP / Shell 服务会显示为自定义应用。

开发者配置仍按内部服务独立保存。点击“保存配置”只把当前运行项合并回本地配置文件，不会覆盖其他运行项的未保存草稿；点击“保存并应用”会在保存后刷新正在运行的 runtime registry，并通过当前 WebSocket 连接重新上报 capabilities。Agent 未运行时，保存仍会落盘，下一次启动后生效。

例如：

- `computer.screenshot`
- `computer.click`
- `shell.exec`
- `shell.queryExecution`

这里：

- `computer` / `shell` 是默认系统服务
- `screenshot` / `click` / `exec` / `queryExecution` 是方法

外部不会看到：

- 这是 shell 实现的
- 还是 HTTP 转发实现的

这些都只是本地 `bridge-agent` 的内部 binding 细节。

注意：

- `computer_use` / `shell` / `http` 都不在 agent-relay 协议里暴露
- relay 同时看到 `services[]` 和 `local_apps[]`；本地应用定义包含稳定
  `connectorId + methods[] + events[]`。同一设备上一个 `connectorId` 只允许一个实例，
  平台用 `deviceId + connectorId` 唯一识别本地应用
- `computer.screenshot` 超过阈值后不应继续把整张图 base64 内联到 WebSocket 消息里，而应走“prepare upload -> direct upload -> asset ref”
- Connector 发送事件时不直接连 relay；它使用安装时生成的独立事件凭证请求
  Bridge Agent 本机入口。事件先写本地 outbox，再通过现有 WebSocket 上报；只有
  Event Center 持久化成功后的 ACK 才会删除 outbox 文件。

## 项目结构

- `src/lib.rs`
- `src/config.rs`
- `src/event_server.rs`
- `src/runtime.rs`
- `src/services.rs`
- `src/main.rs`
- `src-tauri/src/main.rs`
- `src/App.tsx`

## 本地配置模型

正式构建的默认配置文件会写到系统配置目录下的 `agent-config.json`。Debug 构建使用独立的
`agent-config.development.json`，不会加载或修改正式配置；显式传入 `--config` 或
`WS_BRIDGE_CONFIG` 时仍以指定路径为准。

示例配置可以用下面命令生成：

```bash
cargo run -- init-config
```

核心字段：

- `platform.base_url`
- `platform.workspace_id`（授权成功后自动写回）
- `upload.prepare_url`（可选；默认使用 relay 同域的 `/api/bridge-agent/uploads/prepare`）
- `upload.inline_limit_bytes`（截图内联阈值，默认 262144 字节；超过后改走上传，避免大图 base64 进入同步调用响应）
- `upload.timeout_secs`
- `relay.url`
- `relay.agent_id`
- `relay.token`（兼容字段；持久化时始终留空，实际凭证由系统安全存储管理）
- `runtime.default_timeout_secs`
- `runtime.log_file_enabled`
- `runtime.log_file_dir`（可选；留空时使用系统默认日志目录）
- `runtime.event_server_enabled`（默认启用）
- `runtime.event_server_bind`（默认 `127.0.0.1:18081`）
- `runtime.event_server_token`（可选；如果监听非 loopback 地址则必须配置）
- `services[].methods[].binding`
- `services[].events[]`

`binding.type` 只存在于本地配置里，用来决定本机怎么执行方法，不会进入 relay 协议。

### 设备凭证存储

Relay token 不再明文写入 `agent-config.json`，也不会返回给 WebView 前端。桌面界面只接收“已配置/未配置”状态：

- macOS 使用 Keychain
- Linux 使用 Secret Service
- Windows 使用 CNG DPAPI-NG 的机器级保护，使交互桌面进程和 LocalSystem 服务可以读取同一份加密凭证

Debug 构建不会访问正式凭据存储。macOS/Linux 开发凭据写入配置目录下独立的
`.bridge-agent-development/*.credentials` 文件，目录和文件权限分别为 `0700`、`0600`；正式构建仍只
使用 Keychain 或 Secret Service。开发凭据不会从正式 token 自动迁移，开发版需要独立完成一次设备授权。
这样本地重新编译产生的新代码哈希不会触发 macOS 钥匙串身份确认，也不会让未签名调试程序获得正式
relay token。

旧版本配置中的明文 token 会在首次加载时自动迁移到系统安全存储并从 JSON 中删除。Unix 配置目录和配置文件同时收紧为 `0700`、`0600`。Windows 的加密凭证文件与共享配置放在同一 ProgramData 目录，但文件内容不能作为明文读取。

## Connector 设备事件

Schema `2.0` Connector 在 `connector.json` 顶层声明 `events[]`。Bridge Agent 启动
Connector 时会注入：

- `BAIJIMU_CONNECTOR_EVENT_ENDPOINT`：默认
  `http://127.0.0.1:18081/v1/local-app-events`
- `BAIJIMU_CONNECTOR_EVENT_TOKEN_FILE`：该安装实例独享、权限为 `0600` 的事件凭证文件

Connector 使用 Bearer token 发布：

```bash
curl -X POST "$BAIJIMU_CONNECTOR_EVENT_ENDPOINT" \
  -H "Authorization: Bearer $(tr -d '\\r\\n' < "$BAIJIMU_CONNECTOR_EVENT_TOKEN_FILE")" \
  -H 'Content-Type: application/json' \
  -d '{
    "connectorId": "com.baijimu.connector.wechat",
    "event": "messageReceived",
    "eventId": "evt-01J...",
    "payload": {"conversationId": "c-1"}
  }'
```

返回 `202` 表示事件已经进入本机持久化 outbox。断线或重启后会自动重传；Relay 只有在
Event Center 完成去重和事务持久化后才 ACK，收到 ACK 后本机才删除该事件。这个链路
不创建 runtime service，也不生成 `businessId`。

## 自定义服务事件

设备上的服务可以在配置里声明事件：

```json
{
  "name": "reportTool",
  "description": "Local report generation service.",
  "enabled": true,
  "methods": [],
  "events": [
    {
      "name": "finished",
      "description": "Emitted when report generation finishes.",
      "enabled": true,
      "payload_schema": {
        "type": "object",
        "additionalProperties": true
      }
    }
  ]
}
```

运行时 bridge-agent 会在本机启动事件入口，默认地址是 `127.0.0.1:18081`。自定义服务发送事件：

本地事件入口与 relay WebSocket 是两个独立子系统。事件端口绑定失败不会阻止 Agent 连接 relay；
运行时会记录 `event_server.bind_failed` 并持续重试。端口恢复后会记录 `event_server.recovered`，随后本地
服务和 Connector 可以继续向固定入口发送事件，无需重启 Agent。

```bash
curl -X POST http://127.0.0.1:18081/v1/events \
  -H 'Content-Type: application/json' \
  -d '{
    "service": "reportTool",
    "event": "finished",
    "payload": {
      "jobId": "job-1",
      "status": "success"
    }
  }'
```

如果配置了 `runtime.event_server_token`，请求需要带：

```bash
curl -X POST http://127.0.0.1:18081/v1/events \
  -H 'Authorization: Bearer <event-server-token>' \
  -H 'Content-Type: application/json' \
  -d '{"service":"reportTool","event":"finished","payload":{}}'
```

bridge-agent 只接受已声明且已启用的 `service.event`，接收后返回 `202 Accepted`，并通过 agent 与 relay 的 websocket 发送 `event_emitted` 消息。后续由 relay 按订阅关系把事件投递到订阅方 URL。

## 本机服务注册

bridge-agent 支持本机程序把自己注册成 bridge-agent 服务。这个入口只给 bridge-agent 所在机器上的本地程序、脚本或 AI 生成工具使用，不给 relay 反向调用。

新生成的默认配置会开启本机服务注册，并写入 `runtime.service_registration_token`。已有配置如果要开启，需要手动增加：

```json
{
  "runtime": {
    "service_registration_enabled": true,
    "service_registration_token": "replace-with-a-local-secret"
  }
}
```

服务注册复用本机 API server，默认地址仍是 `127.0.0.1:18081`。注册一个本地 HTTP 程序：

```bash
curl -X POST http://127.0.0.1:18081/v1/services \
  -H "Authorization: Bearer $BRIDGE_AGENT_SERVICE_REGISTRATION_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "reportTool",
    "description": "AI generated report service.",
    "transport": {
      "type": "http",
      "baseUrl": "http://127.0.0.1:39127"
    },
    "healthCheck": {
      "type": "http",
      "path": "/health",
      "timeoutSecs": 2,
      "expectStatus": 200
    },
    "startCommand": {
      "type": "shell_command",
      "command": ["report-tool", "start"],
      "timeoutSecs": 15
    },
    "methods": [
      {
        "name": "generate",
        "description": "Generate a report.",
        "responseMode": "plain",
        "path": "/invoke/generate",
        "httpMethod": "POST",
        "timeoutSecs": 60,
        "input_schema": {
          "type": "object",
          "additionalProperties": true
        }
      }
    ],
    "events": [
      {
        "name": "finished",
        "description": "Report generation finished."
      }
    ],
    "replace": true
  }'
```

注册成功后，bridge-agent 会把服务写入 `agent-config.json`，刷新正在运行的 runtime registry，并通过现有 WebSocket 重新上报 capabilities。外部 agent 看到的是普通的 `reportTool.generate`，不会看到本机 HTTP binding 细节。

`methods[].responseMode` 是方法响应契约，支持 `cmodel`、`plain`、`passthrough`，缺失时为 `cmodel`。Bridge Agent 会把该字段随 capabilities 上报；调用请求不携带也不能覆盖它。`plain` 返回本机 HTTP JSON，`passthrough` 保留本机 HTTP 状态、响应头和原始响应字节，由 Relay 恢复为最终 HTTP 响应。

`healthCheck` 和 `startCommand` 是本机客户端使用的注册服务运行信息，不会上报给 relay capabilities：

- `healthCheck.type=http`：支持 `path` 相对 `transport.baseUrl`，也支持直接传完整 `url`；`timeoutSecs` 默认由客户端控制，`expectStatus` 默认按 200 判断。
- `startCommand.type=shell_command`：`command` 是 argv 数组，命令应当是“触发启动后退出”的动作，例如调用系统服务管理器；不要注册一个长期前台运行且不退出的命令。

管理接口：

- `GET /v1/services`：列出本机配置里的服务
- `POST /v1/services`：新增服务；同名服务默认拒绝，`replace: true` 时覆盖
- `PUT /v1/services/{name}`：按名称覆盖服务
- `DELETE /v1/services/{name}`：删除服务并热刷新 capabilities

也可以用 CLI 脚本化修改配置：

```bash
bridge-agent register-service --file service-registration.json --replace
bridge-agent list-services
bridge-agent unregister-service reportTool
```

CLI 直接修改配置文件，适合安装脚本或 agent 未运行时使用；如果需要正在运行的 agent 立即上报 relay，优先调用本机 `/v1/services` API。

## 运行日志

运行时日志会同时保存在桌面端“诊断 -> 日志”和本地文件里。文件日志默认开启，按大小轮转，适合排查 Windows service 或用户机器上的联调问题。

桌面壳、启动流程和 WebView 前端日志统一通过 Tauri 官方日志插件记录，按 5 MiB 轮转并保留 5 个文件；不再通过自定义前端 IPC 命令拼接日志。

默认日志路径：

- Windows：`C:\ProgramData\Baijimu\BridgeAgent\logs\bridge-agent.log`
- macOS / Linux：系统应用数据目录下的 `bridge-agent/logs/bridge-agent.log`

可通过本地配置调整：

- `runtime.log_file_enabled`
- `runtime.log_file_dir`
- `runtime.log_file_max_bytes`
- `runtime.log_file_max_files`

## 快速开始

1. 生成配置文件

```bash
cargo run -- init-config
```

2. 编辑本地配置，声明你要开放的服务和方法

例如：

- 开一个 `computer.screenshot`
- 再开一个 `computer.click`
- 使用默认的 `shell.exec`
- 或者注册一个映射本地 HTTP 服务的自定义方法，例如 `reportTool.generate`

3. 启动 agent

```bash
cargo run -- run
```

4. 点击浏览器授权，在网页中选择目标工作区并完成批准

5. 授权成功后，外部 app / agent 才能通过 relay 调用这台机器上的服务

如果你要给最终用户分发，一般不是让用户跑 `cargo run`，而是直接分发 Tauri 打包后的桌面应用。

## 大截图上传协议

当 `computer.screenshot` 结果超过 `upload.inline_limit_bytes` 时，`bridge-agent` 不再把整张图内联到 WebSocket 消息里，而是改走上传。默认阈值是 262144 字节，常规桌面截图通常会返回文件引用而不是完整 base64：

1. `bridge-agent -> prepare upload`
2. `bridge-agent -> 直传对象存储 / 文件服务`
3. `bridge-agent -> relay` 只回文件引用

默认的上传准备接口：

- `POST {relay-origin}/api/bridge-agent/uploads/prepare`

其中 `relay-origin` 会从 `relay.url` 自动推导：

- `wss://relay.baijimu.com/ws/agent` -> `https://relay.baijimu.com/api/bridge-agent/uploads/prepare`
- `ws://127.0.0.1:8080/ws/agent` -> `http://127.0.0.1:8080/api/bridge-agent/uploads/prepare`（旧默认，仅兼容迁移）

也可以通过 `upload.prepare_url` 显式覆盖。

请求头建议：

- `Authorization: Bearer {relay.token}`

请求体示例：

```json
{
  "agent_id": "dev_8f5b7bb6308f4b6f8c0d2cb4b5f8a1a4",
  "workspace_id": 642,
  "purpose": "computer_screenshot",
  "content_type": "image/png",
  "file_name": "bridge-agent-screenshot-1744718123456.png",
  "size_bytes": 19905790
}
```

上传准备响应示例：

```json
{
  "file_id": "file_123",
  "upload_url": "https://oss-example/put-signed-url",
  "method": "PUT",
  "headers": {
    "x-oss-content-sha256": "UNSIGNED-PAYLOAD"
  },
  "object_key": "bridge-agent/screenshots/file_123.png",
  "download_url": "https://download.example.com/file_123",
  "expires_at": "2026-04-15T20:00:00+08:00"
}
```

截图最终通过 relay 返回给上层的结果示例：

```json
{
  "result_type": "asset_ref",
  "asset_id": "file_123",
  "object_key": "bridge-agent/screenshots/file_123.png",
  "download_url": "https://download.example.com/file_123",
  "expires_at": "2026-04-15T20:00:00+08:00",
  "mime_type": "image/png",
  "width": 3024,
  "height": 1964,
  "display_id": null,
  "size_bytes": 19905790
}
```

如果没有可用上传接口，同时截图又超过阈值，`bridge-agent` 会返回：

- `error.code = "PAYLOAD_TOO_LARGE"`

这样可以避免继续把 relay 的 WebSocket 单消息上限打爆。

## Connector 私有资源上传

声明 `assets.upload` 权限并要求宿主能力 `connector.asset-upload.v1` 的 Connector，
会获得 Connector 专属的本机上传端点与凭证文件：

- `BAIJIMU_CONNECTOR_ASSET_UPLOAD_ENDPOINT`
- `BAIJIMU_CONNECTOR_ASSET_UPLOAD_TOKEN_FILE`

Connector 向 `POST /v1/local-app-assets` 提交其专属数据目录内的图片路径。Bridge Agent
校验 Connector 安装状态、启用状态、权限、独立凭证、文件真实路径、图片魔数和 5 MB
上限后，使用宿主自己的 relay token 申请私有 OSS 上传槽位并完成直传。Connector 从不
获得 relay token。接口返回 `assetId`、`objectKey`、短期 `downloadUrl`、`expiresAt`、
`sha256` 等统一 `asset_ref` 字段。

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

## 桌面运行与登录启动

Bridge Agent 当前只支持有用户会话的桌面运行模式，不提供开机后、用户登录前运行的无人值守模式。

所有平台都由 `bridge-agent-desktop` 独占 Agent Runtime：

- 用户登录后，Tauri Autostart 当前用户登录项启动桌面宿主
- 关闭主窗口只隐藏到系统托盘，桌面宿主和 Agent Runtime 继续运行
- 在托盘选择“退出”时，桌面宿主先停止 Agent Runtime、释放 `127.0.0.1:18081`，再退出进程
- 用户注销或系统关机后桌面宿主停止；下次用户登录时重新启动
- Windows MSI 不安装 `BridgeAgent` Windows Service，也不包含 `bridge-agent-service.exe`
- 从 0.2.22 及更早版本升级时，MSI 会在安装阶段停止并删除历史 `BridgeAgent` 服务，再由新版桌面宿主接管 Runtime
- 应用内更新会先完成更新包下载和签名校验，再停止 Agent Runtime，随后启动安装器；安装器启动失败时会尝试恢复 Runtime

Windows 继续使用共享设备配置：

- `C:\ProgramData\Baijimu\BridgeAgent\agent-config.json`

“登录启动”和“无人值守”的边界不同：登录启动依赖交互用户会话，适合托盘、桌面控制和用户主动退出；无人值守则必须在没有任何用户登录时仍能在线，需要独立服务作为唯一 Runtime 所有者。当前产品只实现前者。

## 桌面应用开发

安装前端依赖：

```bash
npm install
```

启动桌面开发版：

```bash
npm run tauri dev
```

开发版使用独立配置和凭据，不会复用已安装正式版的设备身份。需要连接 relay 时，请在开发版中单独授权；
不要通过 `--config` 或 `WS_BRIDGE_CONFIG` 把正式配置复制成开发配置。

构建前端：

```bash
npm run build
```

执行与正式发布一致的本地质量门禁：

```bash
npm test
cargo fmt --all -- --check
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo test --workspace
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

### 客户端唤起协议

桌面安装包注册 `baijimu` URL Scheme。分享承接页只能在用户点击后调用以下固定路由：

```text
baijimu://codex/install
baijimu://codex/install?shareId=<opaque-share-id>
```

`shareId` 可选，长度不超过 256，只接受 URL-safe 字符。客户端把所有 Deep Link 当作不可信输入：拒绝
其他 host、path、查询参数、重复 `shareId`、fragment 和跳转 URL，不允许 Deep Link 直接触发自定义来源
安装。合法路由按当前本机状态进入 Codex 已安装详情、正在安装进度或平台市场中的 Codex 安装确认页。

Windows/Linux 的二次唤起由 Single Instance 插件转发给已有进程；macOS 的协议声明固化在正式 `.app`
的 `CFBundleURLTypes` 中。网页不能可靠读取客户端安装状态，应采用“尝试唤起，失败后显示客户端下载”的
交互，不得把超时推测展示成确定的未安装结论。

### 启动恢复与安全模式

桌面端采用“基础壳先启动、业务组件后启动”的顺序。系统托盘、本地应用 UI 服务、内置 CLI 或 Agent runtime 启动失败时，只把对应组件标记为降级，不再终止 Tauri 主进程。单实例插件确认当前进程是主实例后才会写入本次启动记录；重复启动只唤醒已有窗口，不会累计启动失败。前端完成 IPC 握手后会清除本次未完成启动标记；连续两次真正未完成握手时，下一次自动进入安全模式。

安全模式不自动启动本地 UI、CLI 和 Agent runtime，但仍保留官方签名更新、启动日志、配置归档恢复和普通模式重启。也可以显式执行：

```bash
open -a "百积木" --args --safe-mode
```

启动健康状态保存在配置目录的 `startup-state.json`，启动诊断日志路径会直接显示在安全模式界面。

## 发布与国内分发

`bridge-agent` 独立使用 GitHub Actions 发布，不经过 Jenkins。正式发布的唯一自动化入口是
`.github/workflows/release-bridge-agent.yml`，由 GitHub `bridge-agent-vX.Y.Z` tag 触发。
客户端运行时不依赖 GitHub 或 Gitee 判断更新。正式发布链路是：

1. 发布提交先合入并推送到 GitHub `main`，四处桌面版本号和内置 CLI 固定版本保持一致
2. 在该提交上创建不可变 `bridge-agent-vX.Y.Z` tag，并只把 tag 推送到 GitHub
3. GitHub Actions 校验 tag、精确提交、`main` 归属、桌面版本和内置 CLI tag，并串行执行发布
4. GitHub Actions 执行前端、Rust 和 Windows 质量门禁
5. GitHub Actions 负责 macOS、Windows、Linux 构建、代码签名、公证，并用 Tauri updater 私钥生成更新包签名
6. GitHub Actions 从 release service 获取短时 OSS PUT 地址，把安装包上传到百积木公共 OSS；GitHub Release 只保留完整历史和构建摘要
7. 每个 OSS 对象通过匿名完整下载和 SHA-256 校验后，把永久 OSS URL、对象键、sha256 与 minisign 签名登记到 release service
8. 所有平台产物上传完成后，工作流发布版本元数据，并校验公开更新接口只返回 `download.baijimu.com` CDN 地址
9. 客户端先用版本策略接口判断强制更新，再由 Tauri 官方 updater 请求动态更新接口，校验签名后原子安装并重启

百积木公共 OSS 是客户端二进制的权威存储源，release service 是版本、校验和、签名与下载选择的唯一事实源。GitHub Actions 不保存 OSS 长期凭据，而是通过 release service 和 project-service 获取短时、单对象上传地址。`www.baijimu.com/download/` 是面向用户的官网客户端下载页；公开 latest/Tauri 元数据把已验证的不可变 OSS `objectKey` 投影到 `download.baijimu.com` CDN，Gitee 不参与 Bridge Agent 客户端安装包分发。

独立 Baijimu CLI 的版本策略由 `local-app-market` 的 `managed_tool` manifest 管理，
不登记到桌面端 Tauri 更新服务。CLI 的私有 GitHub Release 保存构建归档，内容寻址 OSS
保存公开 ZIP 与校验和；Bridge Agent 的新版本只从 OSS 获取固定版本，不再承载 CLI Release。

仓库里的工作流文件是：

- `.github/workflows/release-bridge-agent.yml`

Bridge Agent 只发布自身。Baijimu CLI、Codex Connector 与 Codex Completion Connector
分别在自己的私有 GitHub 仓库使用各自唯一的 `release.yml` 发布。

标准触发方式：

```bash
git switch main
git pull --ff-only origin main
git tag -a bridge-agent-v0.1.115 -m "Release bridge-agent v0.1.115"
git push origin bridge-agent-v0.1.115
```

只能给已经存在于 GitHub `main` 的提交打正式 tag。工作流使用全局
`bridge-agent-release` concurrency group，前一个版本未结束时，后一个版本会在
GitHub Actions 队列中等待，不占用其他项目的发布系统。

如果签名、runner 或工作流本身的问题导致已有 tag 发布失败，修复提交合入 `main` 后，
可以从 GitHub Actions 页面选择 `main`，用原 `release_tag` 执行
`workflow_dispatch`；命令行等价操作是：

```bash
gh workflow run release-bridge-agent.yml \
  --ref main \
  -f release_tag=bridge-agent-v0.1.110
```

已有版本需要重新同步签名制品、OSS 和更新元数据，并同时提升最低支持版本时，必须使用同一工作流的
`repair_assets_only` 入口；工作流会先读取并保留 release service 当前的 `releasePageUrl`，再通过受支持的
策略 API 完整更新和回读策略，不直接修改数据库：

```bash
gh workflow run release-bridge-agent.yml \
  --ref main \
  -f release_tag=bridge-agent-v0.2.33 \
  -f repair_assets_only=true \
  -f minimum_supported_version=0.2.33 \
  -f force_update_message='必须升级到 0.2.33 后继续使用。'
```

`minimum_supported_version` 必须是稳定的三段版本号，且不得高于 `release_tag`。策略保持
`forceUpdate=false`，由服务端仅对低于最低支持版本的客户端返回强制更新；工作流会验证旧版本被强更、
最低支持版本自身不被强更。

修复发布要求新提交是原 tag 提交的后代，且版本号不变；工作流不会移动或覆盖既有
GitHub tag。普通正式发布不要使用 `workflow_dispatch`，直接推送新 tag。

macOS 自动签名和公证前，需要先在仓库的 GitHub Secrets 里配置这些值：

- `BRIDGE_AGENT_UPDATE_API_URL`
  - 客户端检查更新的正式公开接口：`https://updates.baijimu.com/api/bridge-agent/releases/latest`
  - 旧版客户端仍会访问 `https://relay.baijimu.com/api/bridge-agent/releases/latest`，该入口只作为 OpenResty 兼容转发保留
- `BRIDGE_AGENT_RELEASE_API_URL`
  - 发布流程调用的 release service 地址：`https://updates.baijimu.com/api/bridge-agent`
- `BRIDGE_AGENT_RELEASE_API_TOKEN`
  - 发布流程调用 release service 的 Bearer token
- `TAURI_SIGNING_PRIVATE_KEY`
  - Tauri updater minisign 私钥，只在 release runner 中使用
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
  - 上述 updater 私钥密码
- `APPLE_CERTIFICATE`
  - `Developer ID Application` 证书导出的 `.p12` 文件内容，先转成 base64
- `APPLE_CERTIFICATE_PASSWORD`
  - 导出 `.p12` 时设置的密码
- `APPLE_API_ISSUER`
  - App Store Connect API Key 的 Issuer ID
- `APPLE_API_KEY`
  - App Store Connect API Key 的 Key ID
- `APPLE_API_PRIVATE_KEY`
  - 下载得到的 `.p8` 私钥全文内容
- `SSL_COM_USERNAME`
  - SSL.com 账号用户名
- `SSL_COM_PASSWORD`
  - SSL.com 账号密码
- `SSL_COM_CREDENTIAL_ID`
  - eSigner 证书 credential ID，可用 CodeSignTool 的 `get_credential_ids` 查询
- `SSL_COM_TOTP_SECRET`
  - eSigner 自动签名用的 TOTP secret

导出证书并转成 base64 的示例命令：

```bash
openssl base64 -A -in /path/to/developer-id-application.p12 -out certificate-base64.txt
```

这里的 `Developer ID Application` 证书用于面向外部分发的 macOS 应用签名；如果以后需要给 `.pkg` 安装器签名，还要额外申请 `Developer ID Installer` 证书。

工作流在 macOS runner 上会自动完成这些事情：

- 导入 `Developer ID Application` 证书
- 用 `Developer ID Application: Xiaofeng Zhang (H82D8SYZ94)` 给 Tauri 的 macOS 产物签名
- 用 App Store Connect API key 提交 notarization
- 等待公证通过后生成构建产物
- 把构建产物上传到百积木公共 OSS，验证匿名下载后登记到 release service
- 所有平台上传完成后发布最新版本元数据

工作流在 Windows runner 上会自动完成这些事情：

- 生成小体积 MSI，安装时检测 WebView2，缺失时自动下载并安装
- 校验 SSL.com secrets 是否齐全，缺失时直接失败
- 下载并解压 SSL.com eSigner CodeSignTool
- 在 Tauri 打包过程中签名桌面端 exe 和 MSI 安装包
- 用 `Get-AuthenticodeSignature` 校验 Windows 产物签名有效
- 上传 GitHub Release 与百积木公共 OSS，并登记国内更新元数据

Windows 对外发布必须完成代码签名。如果这些 secrets 没配齐，Windows release 会直接失败，不会生成或上传未签名安装包。

Windows eSigner 资源准备：

1. 在 SSL.com 购买 IV/OV/EV Code Signing 证书，正式公司主体优先选 OV
2. 证书签发后把订单 enroll 到 eSigner for Code
3. 设置 eSigner PIN 和 TOTP，用于自动化签名
4. 用 CodeSignTool 查询 `SSL_COM_CREDENTIAL_ID`
5. 把上面的 SSL.com 信息写入 GitHub Secrets

### 更新服务接口约定

Tauri 官方 updater 请求动态协议端点：

```text
GET https://updates.baijimu.com/api/bridge-agent/releases/latest/tauri?target=darwin&arch=aarch64&currentVersion=0.1.101
```

有更新时返回 `200`：

```json
{
  "version": "0.1.105",
  "url": "https://lowcode-common.oss-cn-beijing.aliyuncs.com/lowcode/direct-uploads/bridge-agent-release/20260729/anonymous/uuid-Baijimu_0.1.105_universal.app.tar.gz",
  "signature": "<minisign signature>",
  "notes": "百积木 bridge-agent-v0.1.105",
  "pub_date": "2026-07-19T10:00:00Z"
}
```

没有更新时返回 `204`。macOS updater 使用 `.app.tar.gz`，Windows 使用 `.msi`，Linux 使用 `.AppImage`；三个 updater 产物都必须带签名，缺少签名时服务不会把它返回给客户端。DMG 和 DEB 继续作为手工安装产物。

客户端检查更新时会请求 `BRIDGE_AGENT_UPDATE_API_URL`，并附带查询参数：

- `platform`：`macos` / `windows` / `linux`
- `arch`：例如 `x86_64` / `aarch64`
- `currentVersion`：当前客户端版本

更新服务返回 JSON：

```json
{
  "tagName": "bridge-agent-v0.1.28",
  "version": "0.1.28",
  "updateAvailable": true,
  "forceUpdate": false,
  "minimumSupportedVersion": "0.1.20",
  "releaseName": "百积木 bridge-agent-v0.1.28",
  "releaseUrl": "https://www.baijimu.com/download/",
  "publishedAt": "2026-05-22T10:00:00Z",
  "assets": [
    {
      "name": "百积木_0.1.28_universal.dmg",
      "provider": "baijimu-oss",
      "downloadUrl": "https://download.baijimu.com/lowcode/direct-uploads/bridge-agent-release/20260729/anonymous/uuid-Baijimu_0.1.28_universal.dmg",
      "sha256": "..."
    }
  ]
}
```

`updateAvailable` 由自有更新服务决定；如果省略，客户端会按 `version > currentVersion` 判断。这样服务端可以做灰度、暂停发布、按平台返回不同最新版，二进制由百积木公共 OSS 提供。

`forceUpdate` 为 `true` 时，客户端会显示不可关闭的强制更新界面，只允许安装更新或打开下载页；也可以只返回 `minimumSupportedVersion`，客户端会在 `currentVersion < minimumSupportedVersion` 时自动进入强制更新。可选返回 `forceUpdateMessage` 覆盖默认提示文案。

发布流程调用内部 release service：

- `POST /releases/{tag}`：创建或更新待发布版本
- `POST /releases/{tag}/assets/prepare`：由 release service 向 project-service 申请单对象短时 OSS PUT 地址
- `POST /releases/{tag}/assets/complete`：匿名完整下载与 SHA-256 校验通过后，登记永久 OSS URL、对象键和 updater 签名
- `POST /releases/{tag}/assets/register`：仅用于受控迁移已有永久 OSS/Gitee 资产，不是新版本发布主链
- `POST /releases/{tag}/publish`：在所有平台产物上传完成后，把这个版本设为可被客户端检查到的最新版本

`assets/complete` 请求体示例：

```json
{
  "tagName": "bridge-agent-v0.1.28",
  "version": "0.1.28",
  "target": "macOS Universal",
  "name": "Baijimu_0.1.28_universal.dmg",
  "sha256": "...",
  "contentType": "application/x-apple-diskimage",
  "sizeBytes": 90000000,
  "objectKey": "lowcode/direct-uploads/bridge-agent-release/20260729/anonymous/uuid-Baijimu_0.1.28_universal.dmg",
  "downloadUrl": "https://lowcode-common.oss-cn-beijing.aliyuncs.com/lowcode/direct-uploads/bridge-agent-release/20260729/anonymous/uuid-Baijimu_0.1.28_universal.dmg"
}
```

`assets/complete.downloadUrl` 必须是环境公共 OSS 中与 `objectKey` 精确一致的永久公开地址，不能包含 token、签名查询参数或其他临时凭证；release service 对外查询时再投影成同路径的 `download.baijimu.com` CDN 地址。release service 和 GitHub Actions 都不持有 OSS 长期凭据。

## 本地打包验证

正式分发统一使用前述 GitHub Actions 工作流。本节命令只用于本机验证，不用于发布正式二进制。

macOS 推荐直接构建 universal 安装包，这样最终用户不需要自己区分 Intel 和 Apple Silicon：

```bash
npm run tauri:build:macos-universal
```

如果只是在当前机器本地验证，也可以先跑调试包：

```bash
npm run tauri:build:macos-universal -- --debug
```

正式发布前需要同步更新：

- `package.json`
- `Cargo.toml`
- `src-tauri/Cargo.toml`
- `src-tauri/tauri.conf.json`
- `tools/baijimu-cli/VERSION`

版本提交合入 GitHub `main` 后，再按“发布与国内分发”一节创建并推送正式 tag；
工作流会拒绝版本不一致、tag 不属于 `main`、内置 CLI tag 不存在、内置 CLI 不是本地应用
市场当前稳定版或既有 tag 被移动的发布。修复既有不可变版本的 `workflow_dispatch` 不重新要求
历史内置 CLI 等于当前市场版本，避免后续 CLI 发布阻断同版本制品恢复。

调试打包：

```bash
npm run tauri build -- --debug
```

本机验证过的产物路径：

- `src-tauri/target/universal-apple-darwin/debug/bundle/macos/百积木.app`
- `src-tauri/target/universal-apple-darwin/release/bundle/dmg/百积木_0.1.12_universal.dmg`

macOS、Windows、Linux 正式包只由 GitHub Actions 的受控 runner 构建。正式工作流强制
执行 Apple 签名与公证、Windows Authenticode 签名和 Tauri updater 签名；缺少任何
必需凭证都会终止发布，不会降级生成未签名正式包。

## 方法绑定

### 1. `computer_use`

适合 GPT-5.4 这类模型驱动的桌面控制服务，例如：

- `computer.screenshot`
- `computer.click`
- `computer.type`

当前首版实现：

- 只在 macOS 上启用
- 依赖系统的辅助功能权限和屏幕录制权限
- 内建动作包括截图、单击、双击、移动、拖拽、滚动、输入文本、按键和等待

### 2. `shell_command`

适合终端类服务，例如：

- `shell.exec`
- `shell.queryExecution`

本地策略包括：

- `root_dir`
- `allow_commands`
- 超时限制
- 环境变量白名单

桌面应用从 Finder / 启动器启动时，系统给它的 `PATH` 往往比终端登录 shell 更短。`shell` 会在保留安全环境白名单的同时补入常见本机工具链目录，包括 Homebrew、Volta、nvm、fnm、pyenv、asdf、mise、conda、Cargo、Bun、Deno 和用户本地 `bin` 目录，避免 `node`、`python3` 这类命令因为 GUI 环境缺少 PATH 而找不到。macOS 如果只有 Apple 的 `/usr/bin/python3` shim，且本机尚未同意 Xcode license，该系统命令仍会被 Xcode license 拦截；此时需要用户同意 Xcode license，或安装 Homebrew / pyenv / conda 等独立 Python。

对外调用参数统一使用 argv 数组形式：

- Windows 查询 PATH 或执行 shell 内建命令时，例如 `{"command":["cmd","/C","where","wechat-decrypt"]}`
- 其他平台需要 shell 语义时，例如 `{"command":["sh","-lc","which wechat-decrypt"]}`

### 3. `http`

适合把本地 Java / Node / Python 服务映射成业务方法，例如：

- `reportTool.generate`

当前行为：

- `POST/PUT/PATCH`：把 `arguments` 作为 JSON body 转发
- `GET/DELETE`：把 `arguments` 转成 query string
- 返回状态码、响应头和响应体

## 安全边界

- 本地机器不开放入站端口给外网
- 所有调用都通过本地 agent 主动外连 relay
- `computer_use` 不等于任意 shell，它只执行受控的桌面动作
- shell 方法必须显式 allowlist
- cwd 不能逃逸 root_dir
- 每个方法调用都有超时

如果要进一步提高隔离级别，仍然建议搭配单独用户、容器或系统沙箱使用。

## 这个仓库还应该继续补什么

如果它要作为公开项目给外部用户使用，后续还应该持续补这些文档：

- 安装说明：macOS / Windows / Linux 各自怎么安装
- 授权流程：用户第一次启动后会发生什么
- 配置说明：每个字段的含义和安全影响
- 服务模型说明：什么叫 service、什么叫 method
- 安全模型：哪些能力默认不开、哪些风险需要用户自己确认
- 发布说明：如何下载桌面包、如何校验版本、如何查看源码

当前 README 先把最关键的产品边界写清楚了：`bridge-agent` 是一个需要安装在本机的开源本地代理，而不是一个无需安装就能直接获得本地能力的云工具。
