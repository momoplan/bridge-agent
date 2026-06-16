# 百积木 Local Connector 规范草案

本文定义百积木 Local 的 Connector 安装、运行、注册和远程暴露规范。

目标是把 `wechat-bridge-collector`、Codex Adapter、Claude Code Adapter、桌面控制、浏览器控制等本机能力统一成可安装、可升级、可审计的本地 Connector，而不是依赖 skill 带用户逐条执行安装命令。

## 命名

- 百积木 Local：安装在用户电脑上的本地宿主，工程内部可继续沿用 `bridge-agent`。
- Connector：安装到百积木 Local 的本地连接器，负责连接一个本机应用、服务或能力。
- Connector Package：Connector 的分发包，建议后缀为 `.bjmconnector`。
- Connector Registry：官方或企业维护的 Connector 索引。

示例：

- Codex Connector：连接本机 `codex app-server`。
- WeChat Connector：连接本机微信采集器。
- Claude Code Connector：连接本机 Claude Code。
- Desktop Connector：暴露本机截图、鼠标、键盘等桌面能力。

## 设计原则

1. 百积木 Local 是宿主，不内置具体业务能力。
2. Connector 单独发布、安装、升级和卸载。
3. Connector 可以来自官方市场、GitHub 仓库或本地开发包。
4. 安装不等于远程开放；用户必须显式授权 workspace、service 和高风险 method。
5. Connector 不直接连接 relay；所有远程访问都经过百积木 Local。
6. 百积木 Local 负责鉴权、relay 长连、服务上报、调用审计、生命周期和健康检查。
7. Connector 本身可以开源或闭源，但 manifest、能力声明、签名和校验信息必须可审计。

## 包结构

目录式包：

```text
codex-connector.bjmconnector/
  connector.json
  README.md
  LICENSE
  bin/
    codex-connector
  schemas/
    config.schema.json
    events.schema.json
  hooks/
    install.sh
    uninstall.sh
```

归档式包可以使用同样结构压缩分发，百积木 Local 安装时解包到本机 Connector 目录。

## connector.json

`connector.json` 是唯一必需文件。

```json
{
  "schemaVersion": "1.0",
  "id": "com.baijimu.connector.codex",
  "name": "Codex Connector",
  "version": "0.1.0",
  "description": "Connect local Codex app-server to Baijimu Local.",
  "publisher": {
    "name": "Baijimu",
    "homepage": "https://baijimu.com"
  },
  "source": {
    "type": "github",
    "repo": "baijimu/baijimu-connector-codex",
    "revision": "v0.1.0"
  },
  "runtime": {
    "type": "process",
    "command": "bin/codex-connector",
    "args": [],
    "env": {},
    "healthCheck": {
      "type": "http",
      "url": "http://127.0.0.1:${PORT}/healthz",
      "timeoutSecs": 2,
      "expectStatus": 200
    }
  },
  "configSchema": {
    "type": "object",
    "properties": {
      "codexBinary": {
        "type": "string",
        "default": "codex"
      },
      "defaultCwd": {
        "type": "string"
      }
    },
    "additionalProperties": false
  },
  "remoteCapabilities": [
    {
      "name": "codex.thread.read",
      "risk": "medium",
      "description": "Read local Codex thread metadata and output."
    },
    {
      "name": "codex.turn.write",
      "risk": "medium",
      "description": "Send prompts and follow-up instructions to local Codex."
    },
    {
      "name": "codex.action.approve",
      "risk": "high",
      "description": "Approve Codex actions from a remote client."
    }
  ],
  "services": [
    {
      "name": "codexSession",
      "description": "Local Codex session control.",
      "transport": {
        "type": "http",
        "baseUrl": "http://127.0.0.1:${PORT}"
      },
      "methods": [
        {
          "name": "startThread",
          "description": "Start a Codex thread.",
          "path": "/invoke/startThread",
          "httpMethod": "POST",
          "timeoutSecs": 30,
          "input_schema": {
            "type": "object",
            "additionalProperties": true
          }
        }
      ],
      "events": [
        {
          "name": "messageDelta",
          "description": "Streaming Codex assistant output.",
          "payload_schema": {
            "type": "object",
            "additionalProperties": true
          }
        }
      ]
    }
  ],
  "hooks": {
    "install": "hooks/install.sh",
    "uninstall": "hooks/uninstall.sh"
  }
}
```

## 字段要求

- `schemaVersion`：规范版本，当前为 `1.0`。
- `id`：全局唯一 ID，推荐反向域名格式。
- `name`：展示名。
- `version`：语义化版本。
- `publisher`：发布方信息。
- `source`：来源信息，用于审计和升级。
- `runtime`：启动方式和健康检查。
- `configSchema`：用户配置 schema。
- `remoteCapabilities`：远程能力声明，用于安装确认和 workspace 授权。
- `services`：Connector 安装后向百积木 Local 注册的服务、方法和事件。
- `hooks`：可选安装/卸载脚本。高风险 hook 需要本地用户确认。

## 安装来源

### 官方市场

默认入口。市场返回已审核版本、签名、checksum、兼容范围和下载地址。

### GitHub 仓库

高级入口。支持：

```bash
bjm-local connector install github:baijimu/baijimu-connector-codex
bjm-local connector install https://github.com/baijimu/baijimu-connector-codex
```

仓库必须提供：

- 根目录 `connector.json`，或 GitHub Release asset 中的 `.bjmconnector`。
- 可校验的版本、checksum 和 source revision。
- README，说明本机依赖和安全边界。

如果没有百积木认可签名，安装 UI 必须标记为未验证来源。

### 本地开发包

用于开发和调试：

```bash
bjm-local connector install ./dist/codex-connector.bjmconnector
```

本地开发包默认只对当前设备启用，不自动开放到 workspace。

## 生命周期

百积木 Local 负责：

1. 下载并校验包。
2. 展示远程能力和高风险行为。
3. 写入 Connector 安装目录。
4. 渲染配置。
5. 执行 install hook。
6. 启动 Connector runtime。
7. 运行 health check。
8. 注册 services、methods、events。
9. 重新上报 capabilities 到 relay。
10. 记录安装、启动、调用、事件和卸载日志。

Connector 状态至少包含：

- `installed`
- `configured`
- `starting`
- `running`
- `unhealthy`
- `stopped`
- `update_available`
- `failed`

## 服务注册

Connector 可以通过两种方式注册服务：

1. 静态注册：百积木 Local 从 `connector.json.services` 生成本地 service 配置。
2. 动态注册：Connector 启动后调用本机 `/v1/services` 注册或替换 service。

动态注册适合运行期端口、path、事件 schema 会变化的 Connector。静态注册适合固定 HTTP/stdio/socket 适配器。

无论哪种方式，对 relay 上报的仍然只有：

```text
services[].methods[]
services[].events[]
```

Connector 的内部 transport、进程、hook、安装来源不暴露给远端调用方。

## 权限和授权

Connector 权限分两类：

- 本机依赖：例如启动 `codex app-server`、读取微信本地数据库、连接本机 HTTP 端口。
- 远程能力：例如远程发 prompt、读取消息、审批 action、控制桌面。

百积木 Local 不替代 Codex、微信或 Claude Code 自己的权限系统。它只控制这些能力通过 relay 暴露给哪个 workspace、哪个远端 app、哪些 service/method。

安装后默认策略：

1. Connector 已安装但不自动开放给任何 workspace。
2. 用户必须选择开放的 workspace。
3. 用户必须选择开放的 service/method。
4. 高风险 method 默认需要确认或显式策略。
5. 所有远程调用本地留审计日志。

## WeChat Connector 映射

现有 `wechat-bridge-collector` 已经具备 Connector 雏形：

- 本机 HTTP method server：`http://127.0.0.1:18082/invoke/*`
- 本机事件入口：`POST http://127.0.0.1:18081/v1/events`
- 服务注册文件：`service-registration.json`
- health check：`GET /health`
- start command：`wechat-bridge-collector start`
- 自启：`wechat-bridge-collector install-autostart`

迁移后：

- `wechat-bridge-collector` 仓库增加 `connector.json`。
- 安装入口从 skill 改为百积木 Local Connector 安装器。
- skill 只保留为诊断、权限异常处理和 legacy 安装 fallback。
- `service-registration.json` 可以继续作为服务定义来源，或被 `connector.json.services` 引用。

## Codex Connector 映射

Codex Connector 不应通过 PTY 作为主协议。优先使用 `codex app-server`：

```text
百积木 Local
  -> Codex Connector
  -> codex app-server --listen stdio:// 或 unix://
  -> JSON-RPC thread/turn/item/event
```

远端调用映射：

- `codexSession.startThread` -> `thread/start`
- `codexSession.resumeThread` -> `thread/resume`
- `codexSession.startTurn` -> `turn/start`
- `codexSession.steerTurn` -> `turn/steer`
- `codexSession.interruptTurn` -> `turn/interrupt`

事件映射：

- `thread/started` -> `codexSession.threadStarted`
- `turn/started` -> `codexSession.turnStarted`
- `item/agentMessage/delta` -> `codexSession.messageDelta`
- `item/started` -> `codexSession.itemStarted`
- `item/completed` -> `codexSession.itemCompleted`
- `turn/completed` -> `codexSession.turnCompleted`

Codex app-server transport 不应直接暴露到公网。Connector 应使用本机 stdio、Unix socket 或 loopback，并通过百积木 Local relay 暴露结构化能力。

## CLI 草案

```bash
bjm-local connector search codex
bjm-local connector install baijimu/codex
bjm-local connector install github:baijimu/baijimu-connector-codex
bjm-local connector install ./dist/wechat-connector.bjmconnector
bjm-local connector list
bjm-local connector status com.baijimu.connector.wechat
bjm-local connector start com.baijimu.connector.wechat
bjm-local connector stop com.baijimu.connector.wechat
bjm-local connector update com.baijimu.connector.wechat
bjm-local connector uninstall com.baijimu.connector.wechat
bjm-local connector logs com.baijimu.connector.wechat
```

## 实施路线

### 阶段 1：规范和兼容层

- 定义 `connector.json` schema。
- 在百积木 Local 中加入 Connector 安装目录和状态表。
- 支持从本地目录安装 Connector。
- 支持从 `connector.json.services` 生成现有 `ServiceConfig`。
- 保持现有 `/v1/services` 动态注册能力。

### 阶段 2：WeChat Connector 迁移

- 给 `wechat-bridge-collector` 增加 `connector.json`。
- 把 `setup`、`probe`、`install-autostart`、`start`、`register` 编排到安装器。
- 安装完成后自动 health check 和 service 上报。
- 将 `agent-skill` 改为诊断和 legacy fallback。

### 阶段 3：Codex Connector

- 新建 `baijimu-connector-codex`。
- 对接 `codex app-server` JSON-RPC。
- 支持 thread、turn、streamed events、approval 和 interrupt。
- 通过百积木 Local relay 实现远程控制。

### 阶段 4：市场和 GitHub 安装

- 增加官方 Connector Registry。
- 支持 GitHub repo/release 安装。
- 增加签名、checksum、兼容性校验。
- 增加更新和回滚。

## 与 skill 的关系

Connector 安装器接管标准安装路径。

Skill 不再负责常规安装，而负责：

- 诊断安装失败原因。
- 引导系统权限异常处理。
- 解释安全边界。
- 处理旧版本迁移。
- 支持没有百积木 Local 安装器的 legacy 场景。

这可以避免每个本机能力都写一套 skill 安装流程，也避免安装行为散落在不可审计的自然语言执行步骤里。
