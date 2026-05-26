# bridge-agent

`bridge-agent` 是安装在用户自己电脑上的本地代理。

它的职责不是替代顶层 agent，而是把这台机器上经过用户授权的本地能力，安全地暴露给外部 agent 调用。

先把最容易混淆的一点说清楚：

- 任何人都可以使用这个开源项目
- 但如果你想让外部 agent 使用你自己电脑上的能力，你必须先在自己的机器上安装并运行 `bridge-agent`
- 外部 agent 不能在“用户什么都不装”的前提下直接获得本地 shell 或本地服务能力

## 下载

给最终用户分发时，使用平台自己的下载页或更新服务返回的国内下载地址：

- 最新版本页：由 `BRIDGE_AGENT_RELEASE_PAGE_URL` 指向平台下载页
- macOS：优先下载 universal `.dmg`
- Windows：下载 `.msi` 或安装器
- Linux：下载 `.AppImage` / `.deb`

如果你只是普通用户，直接下载对应平台安装包即可，不需要本地安装 Rust 或 Node 环境。

如果你拿到的是旧版分发物，需要额外注意：

- Intel Mac 只能运行 `x64` 或 universal 的 macOS 安装包
- Apple Silicon Mac 可以运行 `arm64` 或 universal 的 macOS 安装包
- 如果误装了另一种架构，Finder 里能看到应用，但启动会失败

## 它解决什么问题

`bridge-agent` 解决的是“外部 agent 如何安全地调用用户自己电脑上的本地能力”这个问题。

典型场景：

- 让外部 ChatGPT / Claude 调用本机桌面控制能力，例如 `computer.screenshot` / `computer.click`
- 让外部 agent 调用本地已经存在的业务服务，例如本机 Java / Node / Python 服务
- 让本地机器不暴露公网入站端口，仍然能被远端授权访问

## 架构关系

系统里有三个角色：

- `bridge-agent`
  - 安装在用户自己的机器上
  - 管理“我这台机器对外开放哪些服务和方法”
- `relay`
  - 负责转发和鉴权
  - 不直接执行本地命令
- 外部 agent / app
  - 通过 `relay` 调用某个设备上的 `service.method`

调用链路：

1. 用户在本机安装并启动 `bridge-agent`
2. `bridge-agent` 打开授权页面，用户确认授权
3. `bridge-agent` 获取 `agent token`，主动连接 `relay`
4. 用户把某个设备服务授权给外部 app / agent
5. 外部 app / agent 拿到 `client token`
6. 外部 app / agent 通过 `relay` 调用本机暴露的 `service.method`

## 谁需要安装

- 如果你只是调用别人已经开放出来的服务：不需要安装 `bridge-agent`
- 如果你想让外部 agent 使用你自己电脑上的能力：需要安装 `bridge-agent`
- 如果你是平台运营方：需要部署 `relay`

所以它应该是一个可审计、可安装、可分发的开源本地项目，而不是一个纯云端工具。

## 当前工程形态

`bridge-agent` 现在是一个完整的本地端工程，不再只是单个 CLI。

它包含三层：

- Rust core library：负责配置、服务注册、WebSocket 长连、调用转发、日志和本地安全策略
- CLI：适合服务器、脚本或纯命令行场景
- Tauri desktop app：适合最终用户安装、管理本地服务并打包分发

## 当前能力

- 通过 WebSocket 主动连接 relay
- 上报最小协议 `agent_id + services[]`
- 按 `service + method + arguments` 接收调用
- 大截图支持先申请上传槽位、再直传对象存储/文件服务、最后只返回文件引用
- 本地配置里支持三种方法绑定
  - `computer_use`
  - `shell_command`
  - `http`
- 本地管理端可编辑服务、方法、超时、allowlist、日志保留等配置
- 已接浏览器授权启动和轮询，授权成功后会把 `agent token` 自动写回本地配置
- 可打包为桌面应用分发

## 对外暴露的模型

外部看到的是业务服务模型，而不是本地实现细节。

产品语境里可以把概念分成两层：

- 服务：本机配置和协议里的对象，例如 `computer`、`shellExec`、`local-java-service`
- 方法：服务下面的具体动作，例如 `screenshot`、`click`、`invokeApi`
- 对外能力：启用后的 `service.method`，也就是外部 agent 最终能调用的能力

所以桌面端配置页以“服务”为主概念；“能力”只用于描述已经对外开放的调用结果。

当前桌面端把 `computer` 和 `shellExec` 作为系统服务展示：它们由应用默认配置维护，不能删除或改名；用户可以启停服务，其中 `shellExec` 还允许调整命令权限、根目录和超时。其他由用户新增的 HTTP / Shell 服务属于自定义服务。

服务配置按服务独立保存。点击“保存服务”只把当前服务合并回本地配置文件，不会覆盖其他服务的未保存草稿；点击“保存并应用”会在保存后刷新正在运行的 runtime registry，并通过当前 WebSocket 连接重新上报 capabilities。Agent 未运行时，保存仍会落盘，下一次启动后生效。

例如：

- `computer.screenshot`
- `computer.click`
- `shellExec.shellExec`
- `local-java-service.invokeApi`
- `local-java-service.jobFinished`（事件）

这里：

- `computer` / `shellExec` / `local-java-service` 是服务
- `screenshot` / `click` / `shellExec` / `invokeApi` 是方法
- `jobFinished` 是事件

外部不会看到：

- 这是 shell 实现的
- 还是 HTTP 转发实现的

这些都只是本地 `bridge-agent` 的内部 binding 细节。

注意：

- `computer_use` / `shell` / `http` 都不在 agent-relay 协议里暴露
- relay 看到的是 `services[].methods[]` 和 `services[].events[]`，例如 `computer.screenshot`、`local-java-service.jobFinished`
- `computer.screenshot` 超过阈值后不应继续把整张图 base64 内联到 WebSocket 消息里，而应走“prepare upload -> direct upload -> asset ref”
- 自定义本地服务发送事件时，不直接连 relay；它请求 bridge-agent 的本机事件入口，由 bridge-agent 校验事件声明后通过现有 websocket 上报 relay

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

默认配置文件会写到系统配置目录下的 `agent-config.json`。

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
- `relay.token`
- `runtime.default_timeout_secs`
- `runtime.log_file_enabled`
- `runtime.log_file_dir`（可选；留空时使用系统默认日志目录）
- `runtime.event_server_enabled`（默认启用）
- `runtime.event_server_bind`（默认 `127.0.0.1:18081`）
- `runtime.event_server_token`（可选；如果监听非 loopback 地址则必须配置）
- `services[].methods[].binding`
- `services[].events[]`

`binding.type` 只存在于本地配置里，用来决定本机怎么执行方法，不会进入 relay 协议。

## 自定义服务事件

设备上的服务可以在配置里声明事件：

```json
{
  "name": "local-java-service",
  "description": "Example business service backed by a local HTTP endpoint.",
  "enabled": true,
  "methods": [],
  "events": [
    {
      "name": "jobFinished",
      "description": "Emitted when a local job finishes.",
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

```bash
curl -X POST http://127.0.0.1:18081/v1/events \
  -H 'Content-Type: application/json' \
  -d '{
    "service": "local-java-service",
    "event": "jobFinished",
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
  -d '{"service":"local-java-service","event":"jobFinished","payload":{}}'
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
    "methods": [
      {
        "name": "generate",
        "description": "Generate a report.",
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
- 使用默认的 `shellExec.shellExec`
- 或者开一个映射本地 Java 服务的 `local-java-service.invokeApi`

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

## Windows 后台服务

如果需要安装后长期后台运行，并且不依赖用户登录，可以使用新增的 `bridge-agent-service` 二进制，把核心 runtime 作为 Windows Service 托管。

服务入口：

- `bridge-agent-service.exe`

调试运行：

```bash
cargo run --bin bridge-agent-service -- --console
```

也可以显式指定配置文件：

```bash
cargo run --bin bridge-agent-service -- --console --config /path/to/agent-config.json
```

Windows 正式安装时，建议把服务注册成固定服务名 `BridgeAgent`，并把配置文件放到共享路径：

- `C:\ProgramData\Baijimu\BridgeAgent\agent-config.json`

当前默认行为：

- Windows 桌面端 / CLI 如果发现上面的共享配置文件已存在，会优先读取它
- 否则仍然回退到当前用户自己的配置目录
- Windows Service 在没有显式传入 `--config` 时，会默认使用上面的共享路径

一个典型的服务注册命令示例：

```powershell
sc.exe create BridgeAgent binPath= "\"C:\Program Files\Bridge Agent\bridge-agent-service.exe\" --config \"C:\ProgramData\Baijimu\BridgeAgent\agent-config.json\"" start= delayed-auto
```

实现和打包时要注意：

- 不要把 Tauri 桌面程序直接改成服务，服务化的是 Rust runtime/CLI 这一层
- MSI 里需要负责注册服务、升级时先停服务再替换文件、完成后重新拉起
- 如果桌面端也要编辑同一份共享配置，安装器需要给 `C:\ProgramData\Baijimu\BridgeAgent` 配置合适 ACL，让服务账号和交互用户都能读写
- `bridge-agent-service.exe`、桌面端 exe、安装器和后续升级器都要做正式代码签名

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

## 发布与国内分发

`bridge-agent` 继续通过 GitHub Actions 自动产出各平台安装包，但客户端运行时不再依赖 GitHub 或 Gitee 判断更新。正式发布链路是：

1. GitHub Actions 负责构建、签名、公证和生成安装包
2. 发布流程向平台 release service 申请 OSS 预签名上传地址
3. GitHub Actions 把安装包直接上传到国内 OSS/CDN
4. 发布流程把 OSS 下载地址、对象 key、sha256 回写给 release service
5. 所有平台产物上传完成后，发布流程调用 release service 的 publish 接口
6. 客户端只请求 `BRIDGE_AGENT_UPDATE_API_URL` 判断是否有新版本，并从服务返回的国内下载地址下载安装包

Gitee 可以作为代码镜像或国内发布说明入口，但不参与客户端更新判断；即使 Gitee 不可用，也不影响应用内检查更新和下载安装。

仓库里的工作流文件是：

- `.github/workflows/release-bridge-agent.yml`

触发方式：

- 推送 tag：`bridge-agent-v0.1.12`
- 或者在 GitHub Actions 页面手动执行 `workflow_dispatch`

macOS 自动签名和公证前，需要先在仓库的 GitHub Secrets 里配置这些值：

- `BRIDGE_AGENT_UPDATE_API_URL`
  - 客户端检查更新的公开接口：`https://relay.baijimu.com/api/bridge-agent/releases/latest`
- `BRIDGE_AGENT_RELEASE_PAGE_URL`
  - 可选，展示给用户手动打开的下载页，例如 `https://baijimu.com/bridge-agent/download`
- `BRIDGE_AGENT_RELEASE_API_URL`
  - 发布流程调用的内部 release service 地址：`https://relay.baijimu.com/api/bridge-agent`
- `BRIDGE_AGENT_RELEASE_API_TOKEN`
  - 发布流程调用 release service 的 Bearer token，值应与 relay 的 `WS_BRIDGE_ADMIN_TOKEN` 对齐
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
- 通过平台 release service 申请预签名地址，并把构建产物直接上传到国内 OSS/CDN
- 所有平台上传完成后发布最新版本元数据

如果这些 secrets 没配齐，发布任务会直接失败，避免产物只存在 GitHub 或未进入国内更新链路。

### 更新服务接口约定

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
  "releaseName": "Bridge Agent bridge-agent-v0.1.28",
  "releaseUrl": "https://baijimu.com/bridge-agent/download",
  "publishedAt": "2026-05-22T10:00:00Z",
  "assets": [
    {
      "name": "Bridge Agent_0.1.28_universal.dmg",
      "downloadUrl": "https://download.baijimu.com/bridge-agent/releases/bridge-agent-v0.1.28/Bridge%20Agent_0.1.28_universal.dmg",
      "sha256": "..."
    }
  ]
}
```

`updateAvailable` 由自有更新服务决定；如果省略，客户端会按 `version > currentVersion` 判断。这样服务端可以做灰度、暂停发布、按平台返回不同最新版，而客户端不需要依赖 GitHub 或 Gitee 的 release 状态。

发布流程调用内部 release service：

- `POST /releases/{tag}`：创建或更新待发布版本
- `POST /releases/{tag}/assets/prepare`：为单个平台安装包申请 OSS 预签名上传地址
- `POST /releases/{tag}/assets/complete`：安装包上传 OSS 成功后，回写固定公开 `downloadUrl`、`objectKey`、`sha256`、`sizeBytes`
- `POST /releases/{tag}/publish`：在所有平台产物上传完成后，把这个版本设为可被客户端检查到的最新版本

`assets/prepare` 请求体示例：

```json
{
  "tagName": "bridge-agent-v0.1.28",
  "version": "0.1.28",
  "target": "macOS Universal",
  "name": "Bridge Agent_0.1.28_universal.dmg",
  "sha256": "...",
  "contentType": "application/x-apple-diskimage",
  "sizeBytes": 120000000
}
```

`assets/prepare` 响应示例：

```json
{
  "uploadUrl": "https://oss-example/put-signed-url",
  "method": "PUT",
  "headers": {
    "x-oss-content-sha256": "UNSIGNED-PAYLOAD"
  },
  "objectKey": "bridge-agent/releases/bridge-agent-v0.1.28/Bridge%20Agent_0.1.28_universal.dmg",
  "downloadUrl": "https://download.baijimu.com/bridge-agent/releases/bridge-agent-v0.1.28/Bridge%20Agent_0.1.28_universal.dmg",
  "resourceUrl": "https://download.baijimu.com/bridge-agent/releases/bridge-agent-v0.1.28/Bridge%20Agent_0.1.28_universal.dmg"
}
```

`downloadUrl` / `resourceUrl` 必须是长期可访问的公开固定地址；预签名 URL 只用于 `uploadUrl`，不能写入 release 元数据。

## 打包分发

推荐的正式分发方式不是手工发二进制，而是通过 GitHub Actions 构建后向平台 release service 申请预签名地址，再把安装包直接上传到国内 OSS/CDN，并由 release service 发布最新版本元数据。

macOS 推荐直接构建 universal 安装包，这样最终用户不需要自己区分 Intel 和 Apple Silicon：

```bash
npm run tauri:build:macos-universal
```

如果只是在当前机器本地验证，也可以先跑调试包：

```bash
npm run tauri:build:macos-universal -- --debug
```

发布步骤：

1. 同步更新版本号
   - `package.json`
   - `Cargo.toml`
   - `src-tauri/Cargo.toml`
   - `src-tauri/tauri.conf.json`
2. 推送版本 tag，例如 `bridge-agent-v0.1.12`
3. GitHub Actions 会自动构建各平台安装包
4. 工作流通过 release service 申请预签名地址，并把安装包直接上传到国内 OSS/CDN
5. release service 保存安装包下载地址和 sha256，并在 publish 后对客户端开放最新版本
6. 最终用户从平台下载页下载，或在应用内通过自有更新服务自动下载安装

对应 workflow：

- `.github/workflows/release-bridge-agent.yml`

调试打包：

```bash
npm run tauri build -- --debug
```

本机验证过的产物路径：

- `src-tauri/target/universal-apple-darwin/debug/bundle/macos/Bridge Agent.app`
- `src-tauri/target/universal-apple-darwin/release/bundle/dmg/Bridge Agent_0.1.12_universal.dmg`

后续如果要做 Windows / Linux 分发，直接在对应平台执行同样的 `tauri build` 即可。

注意：

- macOS 正式对外分发建议补代码签名和 notarization
- Windows 正式对外分发建议补代码签名
- 没签名也可以先做内测发布，但安装体验会差一些

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

- `shellExec.shellExec`

本地策略包括：

- `root_dir`
- `allow_commands`
- 超时限制
- 环境变量白名单

对外调用参数统一使用 argv 数组形式：

- Windows 查询 PATH 或执行 shell 内建命令时，例如 `{"command":["cmd","/C","where","wechat-decrypt"]}`
- 其他平台需要 shell 语义时，例如 `{"command":["sh","-lc","which wechat-decrypt"]}`

### 3. `http`

适合把本地 Java / Node / Python 服务映射成业务方法，例如：

- `local-java-service.invokeApi`

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
