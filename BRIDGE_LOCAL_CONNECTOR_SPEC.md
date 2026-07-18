# 百积木 Local App and Connector Specification

本文定义 `bridge-agent` 桌面端里的“本地应用”规范，用来约束 Codex、WeChat、Desktop Control 以及后续第三方本地能力如何接入、安装、启动、更新、卸载和对外暴露能力。

## 术语

- 本地应用：用户在 `bridge-agent` 桌面端看到和管理的对象。它可以是官方托管工具、内置应用、市场 Connector，也可以是用户手动注册的自定义应用。
- 官方托管工具：由百积木维护、在应用页独立显示并按版本安装、升级和回滚的本机工具。它不必注册远程服务，例如 Baijimu CLI。
- Connector：可安装的本地应用包。它通过 `connector.json` 声明身份、版本、运行方式、服务注册信息和能力。
- 市场应用：由平台 `local-app-market` 返回的 Connector 分发记录。市场只描述可安装版本，真正安装后仍以 Connector 包为准。
- 自定义应用：用户或本机开发工具通过开发者配置、本机服务注册 API、CLI 手动加入的服务。它没有市场更新源，默认不展示成市场应用。
- 服务：百积木协议内部的能力组，例如 `wechatLocal`、`computer`、`shell`。
- 方法：服务下的可调用动作，例如 `wechatLocal.searchMessages`。
- 事件：服务上报给外部订阅方的消息，例如 `wechatLocal.messageReceived`。

## 分类

本地应用分为四类：

| 类型 | 来源 | 是否可市场更新 | 典型例子 |
| --- | --- | --- | --- |
| 官方托管工具 | 客户端基线版本 + `local-app-market` 独立版本 | 是 | Baijimu CLI |
| 内置应用 | 随 `bridge-agent` 客户端发布 | 否，跟随客户端版本 | Desktop Control |
| 市场 Connector | `local-app-market` 返回的 Git / 包版本 | 是 | Codex Connector、WeChat Connector |
| 自定义应用 | 本机配置、注册 API、CLI | 否 | 本地报表工具、开发中的 HTTP 服务 |

UI 可以统一展示为“本地应用”，但实现和治理必须区分：

- 市场 Connector 必须有稳定 `connectorId`，并能从市场记录找到更新源。
- 官方托管工具必须有稳定应用 ID、SemVer 版本、按平台/架构区分的发布包、SHA-256 和平台签名。
- 官方托管工具由本地应用管理器维护版本目录和稳定命令入口；客户端内置副本只负责首次安装和离线修复，不能覆盖或降级已经托管的更高版本。
- 自定义应用不能伪装成市场应用；除非被市场收录并按本规范提供 Connector 包。
- 内置应用由 `bridge-agent` 客户端维护，不允许普通卸载，也不通过 Connector 安装目录管理。

## 市场应用的宿主管理面板

一个市场 Connector 在用户界面中只能对应一个本地应用。对于涉及本机密钥、系统授权或应用私有配置文件的第一方应用，Connector 可以声明受本机 token 保护的管理接口；`bridge-agent` 按稳定 `connectorId` 在同一个应用详情中挂载管理面板，但不能为这部分功能再创建第二张内置应用卡片。

宿主管理面板遵循以下边界：

- Connector 负责声明和运行可授权的服务、方法、事件、健康检查及其应用管理接口。
- 应用自身负责凭证签发、密钥存储、配置文件原子更新和应用进程重启；`bridge-agent` 只负责安装、启停、健康检查、升级、回滚和经过清单校验的本机管理请求代理。
- 宿主管理操作不得注册成 Connector 的远程方法，不得经过 relay，也不得出现在工作区可授权能力列表中。
- LLM key 不得进入 `bridge-agent`、前端状态或 relay，只能由对应本地应用进程签发、校验和写入。设备授权产生的本机工作区 token 仍由 Bridge Agent 写入共享 CLI 授权文件，应用只从该私有文件读取；两类密钥都不得返回前端，前端只接收脱敏后的归属、有效性和更新时间。
- 应用运行状态和账户配置状态必须分开。Connector 健康检查失败才表示应用运行故障；凭证未配置或无效只表示账户需要处理。
- 卸载 Connector 后，宿主管理面板随应用入口消失；本机凭证是否清理必须由用户单独确认，不能随卸载静默删除。

Codex 是这一模式的首个实现：独立 Rust 本地应用 `com.baijimu.connector.codex` 同时负责 `codex app-server` 的 session、thread、turn 和 event 能力，以及限定到 `workspaceId + projectId` 的 LLM credential 签发和本机 Codex 配置更新。Bridge Agent 只代理它声明的 `credentialState`、`listWorkspaceProjects` 和 `switchCredential` 管理操作；凭证切换不是 `codexSession` 的远程能力。

管理接口必须满足：

- `management.type` 当前只能为 `http`，且 `baseUrl` 必须是 loopback HTTP 地址。
- `management.auth.type` 必须为 `connector_token`；token 由应用首次启动时写入宿主为该 Connector 分配的私有数据目录，文件权限在 Unix 上必须为 `0600`。
- `operations` 只能声明 `GET` 或 `POST`，路径必须位于 `/management/` 下；宿主不得接受前端传入任意 URL、方法或路径。
- 宿主启动应用时通过 `BAIJIMU_CONNECTOR_DATA_DIR` 传入独立数据目录。应用包升级不得覆盖该目录，卸载是否清理业务配置必须由用户确认。

## Connector 包结构

一个 Connector 包必须至少包含：

```text
connector-root/
  connector.json
  service-registration.json
```

可以包含：

```text
connector-root/
  package.json
  bin/
  dist/
  README.md
  LICENSE
```

要求：

- `connector.json` 必须位于包根目录。
- 包内路径必须使用相对路径，不依赖安装前的源码绝对路径。
- 安装后，百积木会把包复制到本机 connectors 目录；运行命令应以安装后的包路径为准。
- 不要把用户 token、cookie、数据库副本或机器私有配置提交进 Connector 包。

## 官方托管工具

官方托管工具不要求 `connector.json`、`service-registration.json` 或 `service.method`。市场版本通过 `latestVersion.manifest` 声明：

```json
{
  "applicationType": "managed_tool",
  "artifacts": [
    {
      "platform": "macos",
      "arch": "universal",
      "source": "https://example.invalid/baijimu-cli-0.1.1-macos-universal.zip",
      "checksum": "sha256:...",
      "archivePath": "bin/baijimu"
    }
  ]
}
```

管理规则：

- 下载必须使用 HTTPS，并在解包前验证市场记录中的 SHA-256。
- macOS 和 Windows 正式产物必须通过系统代码签名验证；Linux 至少验证 SHA-256。
- 安装包中的 CLI 必须能通过 `baijimu --version --json` 返回与市场一致的版本和实现身份。
- 每个版本写入独立目录，稳定命令入口只指向当前激活版本。
- 更新采用同目录临时文件和原子切换；保留上一个有效版本用于回滚。
- Bridge Agent 启动时先读取托管状态；只有没有有效托管版本时才导入旧命令入口或客户端基线版本。
- 官方托管工具默认不经过 relay，也不对外暴露能力。需要远程调用时必须另行设计最小权限的 Connector 接口。

## connector.json

`connector.json` 是 Connector 的主清单。当前 schema 版本为 `1.0`。

必填字段：

- `schemaVersion`
- `id`
- `name`
- `version`
- `services` 或 `serviceRegistrationFiles` 至少一个

推荐字段：

- `description`
- `publisher`
- `source`
- `runtime`
- `configSchema`
- `remoteCapabilities`
- `hooks`

示例：

```json
{
  "schemaVersion": "1.0",
  "id": "com.baijimu.connector.wechat",
  "name": "WeChat Connector",
  "version": "0.2.3",
  "description": "Expose local WeChat search and message events to 百积木.",
  "publisher": {
    "name": "Baijimu",
    "homepage": "https://baijimu.com"
  },
  "source": {
    "type": "git",
    "repo": "momoplan/wechat-bridge-collector",
    "revision": "v0.2.3"
  },
  "runtime": {
    "type": "process",
    "command": "wechat-bridge-collector",
    "args": ["start"]
  },
  "serviceRegistrationFiles": [
    "service-registration.json"
  ],
  "remoteCapabilities": [
    {
      "name": "wechat.events.messageReceived",
      "risk": "high",
      "description": "Emit message events from the user's local WeChat data."
    }
  ]
}
```

命名要求：

- `id` 必须全局稳定，建议使用反域名格式，例如 `com.baijimu.connector.wechat`。
- `id` 一旦发布，不得因为仓库迁移、展示名称变化或实现重写而改变。
- `name` 是展示名，可以变化。
- `version` 应使用 SemVer。市场版本和 Connector 包版本必须一致。

## 服务注册

Connector 通过 `services` 内联声明服务，或通过 `serviceRegistrationFiles` 指向一个或多个服务注册文件。

推荐使用独立 `service-registration.json`：

```json
{
  "name": "wechatLocal",
  "description": "Local WeChat capability service.",
  "transport": {
    "type": "http",
    "baseUrl": "http://127.0.0.1:18082"
  },
  "healthCheck": {
    "type": "http",
    "path": "/health",
    "timeoutSecs": 2,
    "expectStatus": 200
  },
  "startCommand": {
    "type": "shell_command",
    "command": ["wechat-bridge-collector", "start", "--daemon"],
    "timeoutSecs": 15
  },
  "stopCommand": {
    "type": "shell_command",
    "command": ["wechat-bridge-collector", "stop"],
    "timeoutSecs": 10
  },
  "methods": [
    {
      "name": "searchMessages",
      "description": "Search local WeChat messages.",
      "path": "/invoke/searchMessages",
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
      "name": "messageReceived",
      "description": "A local WeChat message was received.",
      "enabled": true,
      "payload_schema": {
        "type": "object",
        "additionalProperties": true
      }
    }
  ]
}
```

要求：

- `name` 是对外协议里的服务名，必须稳定。
- `methods[].name` 和 `events[].name` 必须稳定；删除或改名属于破坏性变更。
- `transport.baseUrl` 默认应绑定 `127.0.0.1`，不要要求用户暴露公网端口。
- `healthCheck` 应能快速判断本地服务是否可用。
- `startCommand` 应是触发启动后退出的命令，不应是永久阻塞的前台进程。
- `stopCommand` 应尽量幂等；服务未运行时也应安全退出。
- `input_schema` 应尽量收紧，不要长期使用完全开放的 `additionalProperties: true` 作为正式能力接口。

## 市场元数据

市场服务 `local-app-market` 返回的是可安装版本列表。百积木当前请求：

```text
GET {platform.base_url}/api/local-app-market/apps?platform={macos|windows|linux}
```

可以返回 lowcode 包装结构：

```json
{
  "errorCode": "0",
  "value": "成功",
  "data": []
}
```

也可以直接返回数组。数组项格式：

```json
{
  "id": "wechat",
  "connectorId": "com.baijimu.connector.wechat",
  "name": "微信",
  "description": "安装微信本地采集 connector，把微信相关本地能力接入工作区。",
  "publisher": "Baijimu",
  "risk": "需要读取本机微信数据库、联系人和消息记录目录，只在用户本机运行。",
  "riskLevel": "high",
  "capability": "本地微信消息查询、搜索和消息事件采集。",
  "platforms": ["macos"],
  "latestVersion": {
    "version": "0.2.3",
    "sourceType": "git",
    "source": "https://github.com/momoplan/wechat-bridge-collector.git",
    "repo": "momoplan/wechat-bridge-collector",
    "revision": "v0.2.3",
    "checksum": null,
    "capabilities": [
      "wechat.messages.read",
      "wechat.messages.search",
      "wechat.events.messageReceived"
    ],
    "manifest": {
      "runtime": "process",
      "command": "wechat-bridge-collector",
      "args": ["start"]
    },
    "publishedAt": "2026-06-18T10:00:00Z"
  }
}
```

要求：

- `id` 是市场条目 ID，面向市场展示和路由。
- `connectorId` 必须等于 Connector 包内 `connector.json.id`。
- `latestVersion.version` 必须等于 Connector 包内 `connector.json.version`。
- `latestVersion.revision` 推荐指向不可变 tag，例如 `v0.2.3`。
- `latestVersion.source` 是安装源。若带 `revision`，百积木会按 `source#revision` 克隆。
- `platforms` 必须准确表达支持平台；不要把只支持 macOS 的 Connector 标成 Windows/Linux 可用。
- `riskLevel` 建议使用 `low`、`medium`、`high`。

## 安装和更新

安装流程：

1. 百积木从市场读取条目。
2. 用户选择市场应用。
3. 百积木下载 `latestVersion.source`，如果有 `revision` 则 checkout 对应分支或 tag。
4. 百积木读取 `connector.json` 并校验。
5. 百积木安装 Connector 包到本机 connectors 目录。
6. 百积木把服务注册写入本机 `agent-config.json`。
7. 百积木刷新 runtime registry，并通过已有 WebSocket 重新上报 capabilities。

更新规则：

- 用市场 `connectorId` 找到本机已安装 Connector。
- 比较本机 `connector.json.version` 与市场 `latestVersion.version`。
- 更新时重新安装同一个 `connectorId`，并替换该 Connector 管理的服务。
- 更新不得悄悄迁移到另一个 `connectorId`。
- 如果服务名、方法名、事件名发生破坏性变更，必须升级主版本，并在市场风险说明中写清楚。

自定义同步规则：

- 从本地目录或用户自己输入的 Git URL 安装的 Connector 不按市场版本判断升级。
- 百积木记录原始安装来源 `sourceReference`、解析后的本地路径 `sourcePath`、首次安装时间和最近同步时间。
- 用户点击“拉取最新”时，百积木使用 `sourceReference` 重新解析来源；没有 `sourceReference` 的历史安装记录回退使用 `sourcePath`。
- 重新同步会重新安装同一个 Connector 包，并替换该 Connector 管理的服务。
- 自定义应用同步完成后更新 `lastSyncedAtEpochMs`，但保留首次 `installedAtEpochMs`。

卸载规则：

- 卸载 Connector 时，删除安装记录和该 Connector 注册的服务。
- 不得删除用户手动创建的自定义服务。
- 不得删除其他 Connector 的服务。

## 权限和安全

Connector 默认运行在用户自己的机器上，因此规范重点是“清楚告知、最小暴露、可撤销”。

必须遵守：

- 本地服务默认只监听 `127.0.0.1`。
- 需要读取本机敏感数据时，必须在市场 `risk` 和 Connector README 中说明。
- 不得默认上传用户本机数据，除非用户明确授权且能力描述中写清楚。
- 不得要求用户关闭系统安全设置作为常规安装步骤。
- 不得把长期有效 token 写入仓库。
- 日志不得记录敏感消息正文、密钥、cookie、完整数据库路径等信息，除非用户显式开启诊断级别。

高风险能力示例：

- 读取聊天记录、联系人、浏览器数据、剪贴板、文件系统。
- 控制桌面、键盘、鼠标。
- 执行 shell 命令。
- 监听消息事件并转发给外部 agent。

高风险能力必须在市场条目中设置 `riskLevel: "high"`。

## 事件

Connector 不直接连接 relay，也不自己向外部订阅方投递事件。

正确流程：

1. Connector 本地服务声明 `events[]`。
2. Connector 运行时向百积木本机事件入口发送事件。
3. 百积木校验 `service.event` 已声明且启用。
4. 百积木通过已有 agent WebSocket 上报 relay。
5. relay 再按订阅关系投递给外部 app / agent。

事件 payload 应保持结构化，并避免发送无边界的大对象。需要传大文件时，应走文件引用或上传协议。

## 兼容性

稳定接口：

- `connector.json.id`
- `connector.json.version`
- 服务名
- 方法名
- 事件名
- 方法输入 schema
- 事件 payload schema

允许非破坏性变更：

- 增加新方法。
- 增加新事件。
- 扩展输入 schema 的可选字段。
- 增加更明确的健康检查。
- 改进启动/停止命令，只要行为兼容。

破坏性变更：

- 改名或删除服务、方法、事件。
- 收紧输入 schema 导致旧调用失败。
- 改变事件 payload 必填字段。
- 改变权限边界，例如从只读查询变成消息监听。

破坏性变更必须提升主版本。

## 本地开发和自定义应用

开发中的 Connector 可以从本地目录或 Git 仓库安装：

```text
/Users/me/connectors/my-app
https://gitee.com/org/my-connector.git#v0.1.0
```

自定义应用适合：

- 临时开发。
- 用户自己机器上的私有工具。
- AI 生成的本地脚本服务。
- 尚未进入市场审核的 Connector。

自定义应用不应被百积木标成“市场应用”。只有当它拥有稳定 `connectorId`、版本、风险说明、可安装源和市场条目后，才是市场 Connector。

自定义应用详情页应提供“拉取最新”或“重新同步”动作，不显示“升级到最新版本”。如果安装源是 Git URL，动作会重新 clone 指定仓库和 revision；如果安装源是本地目录，动作会重新读取该目录当前内容。没有市场条目的 Connector 不展示“检查更新”。

## 发布前验收清单

Connector 发布到市场前至少确认：

- `connector.json` 可以被百积木解析。
- `connector.json.id` 与市场 `connectorId` 一致。
- `connector.json.version` 与市场 `latestVersion.version` 一致。
- Git tag 或 revision 存在，且可被 `git clone --depth 1 --branch <revision>` 拉取。
- 安装后服务能写入 `agent-config.json`。
- `startCommand` 可执行，且不会永久阻塞。
- `healthCheck` 通过。
- 方法能通过百积木调用。
- 事件能通过百积木本机事件入口上报。
- 卸载只删除该 Connector 管理的服务。
- `risk`、`riskLevel`、`platforms` 与真实行为一致。

## 当前实现约束

当前百积木实现中：

- Connector 清单文件名固定为 `connector.json`。
- 安装源支持本地目录和 Git URL。
- Git URL 可以通过 `source#revision` 指定分支或 tag。
- 市场列表既支持 lowcode 包装结构，也支持直接返回数组。
- 安装后记录写入本机 connectors 目录下的 `install.json`，包括 `sourceReference`、`sourcePath`、`installedAtEpochMs` 和 `lastSyncedAtEpochMs`。
- Connector 至少要声明一个服务；服务至少要声明一个方法或事件。
- 服务注册 transport 目前支持 `http`。

后续如果新增包签名、checksum、压缩包分发或沙箱运行，应在本文中扩展对应章节。
