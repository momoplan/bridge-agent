# 百积木 Local App and Connector Specification

本文定义 `bridge-agent` 桌面端里的“本地应用”规范，用来约束 Codex、WeChat、Desktop Control 以及后续第三方本地能力如何接入、安装、启动、更新、卸载和对外暴露能力。

## 术语

- 本地应用：用户在 `bridge-agent` 桌面端看到和管理的对象。除随客户端发布的内置能力外，可安装对象都必须先在平台登记并取得唯一 `appId`。
- 官方托管工具：由百积木维护、在应用页独立显示并按版本安装、升级和回滚的本机工具。它不必注册远程服务，例如 Baijimu CLI。
- Connector：可安装的本地应用包。它通过 `connector.json` 声明身份、版本、运行方式、方法和设备事件。
- 市场应用：由平台 `local-app-market` 返回的 Connector 分发记录。市场只描述可安装版本，真正安装后仍以 Connector 包为准。
- 服务：内置能力和 legacy 自定义能力的协议对象，例如 `computer`、`shell`。
- 方法：本地应用下的可调用动作，例如 `wechat.searchMessages`。
- 设备事件：由某台设备上的某个 Connector 安装实例产生的事件，例如
  `wecom + messageReceived`。唯一安装实例身份是 `deviceId + appId`。

## 分类

本地应用分为三类：

| 类型 | 来源 | 是否可市场更新 | 典型例子 |
| --- | --- | --- | --- |
| 官方托管工具 | 客户端基线版本 + `local-app-market` 独立版本 | 是 | Baijimu CLI |
| 内置应用 | 随 `bridge-agent` 客户端发布 | 否，跟随客户端版本 | Desktop Control |
| 已注册 Connector | `local-app-market` 精确版本登记返回的 Git / 包版本 | 是 | Codex Connector、WeCom Connector、Browser Connector |

UI 可以统一展示为“本地应用”，但实现和治理必须区分：

- Connector 必须使用服务端分配的 `local_app.id` 作为唯一 `appId`，并能从注册表精确解析版本、来源和 SHA-256。
- 官方托管工具必须有稳定应用 ID、SemVer 版本、按平台/架构区分的发布包、SHA-256 和平台签名。
- 官方托管工具由本地应用管理器维护版本目录和稳定命令入口；客户端内置副本只负责首次安装和离线修复，不能覆盖或降级已经托管的更高版本。
- 未登记的本地目录、压缩包或 Git 仓库一律拒绝安装。登记版本可以不经公开审核而通过精确 `appId + version` 直接安装；审核只决定是否出现在公共市场列表。
- 内置应用由 `bridge-agent` 客户端维护，不允许普通卸载，也不通过 Connector 安装目录管理。

## 应用图标

Connector 可以在 `connector.json` 顶层声明应用图标：

```json
{
  "icon": {
    "mediaType": "image/png",
    "data": "<256x256 PNG 原始字节的标准 Base64>"
  },
  "hostRequirements": {
    "minimumVersion": "0.2.95",
    "capabilities": ["connector.presentation.icon.v1"]
  }
}
```

`icon` 是本地应用品牌图标的唯一源事实。宿主会把它展示在应用市场、已安装应用卡片、应用详情和升级确认等
应用身份位置；内嵌页面不得再维护或展示一份应用图标。

- `mediaType` 当前必须为 `image/png`。
- `data` 必须是 PNG 原始字节的标准 Base64，不得包含 `data:` URI 前缀、空白或换行。
- PNG 必须是 `256 × 256` 正方形，解码后不得超过 `128 KiB`；建议保留透明背景并确保缩放到 32px 时仍可辨认。
- 图标缺失时宿主使用通用占位图形；不得在宿主运行时按具体 app ID、名称或市场条目硬编码图标。
- 声明图标的新版本必须要求宿主 `>= 0.2.95` 及能力 `connector.presentation.icon.v1`，确保所有身份位置一致展示。

## 市场应用的本机管理界面

一个市场 Connector 在用户界面中只能对应一个本地应用。对于涉及本机密钥、系统授权或应用私有配置文件的应用，Connector 可以同时声明受本机 token 保护的管理接口和随包发布的内嵌界面；`bridge-agent` 按稳定 `appId` 在同一个应用详情中加载该界面，但不能实现应用专用设置页，也不能为这部分功能再创建第二张内置应用卡片。

宿主管理面板遵循以下边界：

- Connector 负责声明和运行可授权的服务、方法、事件、健康检查、应用管理接口及应用专用界面。
- 应用自身负责凭证签发、密钥存储和配置文件原子更新；进程生命周期按
  `runtime.processOwnership` 明确归属。`connector` 模式由应用维护后台进程，`host` 模式
  由 `bridge-agent` 持有并回收前台进程。宿主同时负责安装、启停、健康检查、升级、回滚
  和经过清单校验的本机管理请求代理。
- 应用专用界面必须随 Connector 包发布和版本化。Bridge Agent 只提供隔离的 HTML 容器与受清单约束的调用桥，不得按应用 ID 写业务界面或业务状态。
- 宿主管理操作不得注册成 Connector 的远程方法，不得经过 relay，也不得出现在工作区可授权能力列表中。
- LLM key 不得进入 `bridge-agent`、前端状态或 relay，只能由对应本地应用进程签发、校验和写入。设备授权产生的本机工作区 token 仍由 Bridge Agent 写入共享 CLI 授权文件，应用只从该私有文件读取；两类密钥都不得返回前端，前端只接收脱敏后的归属、有效性和更新时间。
- 应用运行状态和账户配置状态必须分开。Connector 健康检查失败才表示应用运行故障；凭证未配置或无效只表示账户需要处理。
- 卸载 Connector 后，宿主管理面板随应用入口消失；本机凭证是否清理必须由用户单独确认，不能随卸载静默删除。

Codex 按状态所有权拆成三个本地应用：`codex` 负责桌面应用安装、桌面工作区凭证和用户显式切换；`codex-connector` 独立提供 session、thread、turn 和 event 能力；`codex-completion` 独立提供 OpenAI 兼容补全接口。三个值都是存量市场表主键，不是客户端另造的命名空间。平台项目不参与安装和凭证归属。Bridge Agent 只后台安装和启动应用、加载清单声明的 `ui/` 并代理管理操作；LLM credential 不经过 Bridge Agent。

管理接口必须满足：

- `management.type` 当前只能为 `http`，且 `baseUrl` 必须是 loopback HTTP 地址。
- `management.auth.type` 必须为 `connector_token`；token 由 Bridge Agent 按 `appId` 生成到应用私有数据目录，通过 `BAIJIMU_LOCAL_APP_TOKEN_FILE` 注入应用。Unix 文件权限必须为 `0600`。应用不得自行替换或返回 token。
- `operations` 只能声明 `GET` 或 `POST`，路径必须位于 `/management/` 下；宿主不得接受前端传入任意 URL、方法或路径。
- 宿主启动应用时通过 `BAIJIMU_LOCAL_APP_DATA_DIR` 传入独立数据目录。应用包升级不得覆盖该目录，卸载是否清理业务配置必须由用户确认。
- Bridge Agent 使用同一私有 token 为该应用的健康检查和全部 HTTP 能力调用添加 `Authorization: Bearer ...`。应用必须拒绝无 token 请求；只绑定 loopback 不能代替鉴权。

需要在应用内部提供本机初始化能力的 Connector 可以声明 `setup`：

```json
{
  "setup": {
    "operation": "setupRetry",
    "statusOperation": "setupState",
    "timeoutSecs": 1800
  }
}
```

依赖特定宿主协议的 Connector 还必须声明宿主要求：

```json
{
  "hostRequirements": {
    "minimumVersion": "0.2.21",
    "capabilities": ["connector.setup.v1"]
  }
}
```

- `minimumVersion` 是可安装该版本 Connector 的最低百积木客户端语义版本。
- `capabilities` 是宿主必须具备的协议能力；当前 setup 管理与状态代理契约对应 `connector.setup.v1`。
- 市场必须返回兼容判断；不兼容版本仍可展示，但不得下发安装源，客户端必须提示先升级。
- 客户端必须在 UI 和安装命令两层校验，不能依赖前端禁用按钮作为唯一保护。

- `setup` 仅允许用于 `schemaVersion: "3.0.0"`，并要求同时声明 `management`。
- 新发布且声明 `setup` 的 Connector 必须同时声明 `hostRequirements.minimumVersion >= 0.2.21` 和 `connector.setup.v1`；`0.2.21` 是首个会上报宿主版本/能力并展示升级提示的客户端版本。
- `operation` 必须引用一个 `POST` management operation；应用内界面通过宿主受控调用桥传入当前授权工作区，操作应立即返回并在应用内部启动幂等后台任务。
- `statusOperation` 必须引用一个 `GET` management operation，返回 `status`：`pending`、`running`、`succeeded` 或 `failed`。失败时可返回脱敏 `error`。
- `timeoutSecs` 范围为 30–3600，默认 1800，作为应用初始化任务的超时契约保留；Connector 自己负责执行和展示超时结果，宿主安装不等待该期限。
- setup 不得阻塞 Connector 下载、校验、安装、注册和启动。宿主完成这些阶段后立即报告“安装完成”，初始化状态只能显示在应用自身界面中。
- setup 不得要求用户再次选择本机设备或与能力无关的业务项目；需要凭证时由应用使用应用界面确认的当前工作区上下文，并精确校验本机授权。

### 用户命令环境

宿主必须在每次启动 Connector、停止 Connector 或执行本机服务生命周期命令时动态构造
当前用户的命令搜索环境，不能直接采用桌面 GUI 或后台进程启动时继承的 `PATH`：

- macOS/Linux 从操作系统账户记录解析实际登录 shell，并通过有超时、输出上限和进程组回收的
  交互式登录探测读取最终 `PATH`；启动文件的普通输出不得污染机器解析结果。
- Windows 从当前用户和机器环境登记动态读取 `PATH`；不得要求 Bridge Agent 重启后才识别
  已登记的用户命令目录。
- 最终顺序固定为入口可执行文件所在目录、Connector 清单声明的 `PATH`、当前用户规范
  `PATH`、宿主进程 `PATH`，并按首次出现去重。
- 宿主只注入 `PATH`，不得把交互式 shell 中的 Token、Key 或其他未声明环境变量复制给
  Connector。
- 当前用户 `PATH` 是设备动态状态，只能在进程启动边界使用，不能持久化到
  `agent-config.json`、Connector 安装记录或市场清单。
- Connector 派生的子进程自然继承该环境。关键官方托管工具仍必须使用下述声明式依赖和宿主
  注入的绝对路径，不能用通用 `PATH` 替代版本与来源校验。

### 官方托管工具依赖

Connector 需要调用 Baijimu CLI 等官方托管工具时，必须在清单中声明依赖，不能依赖桌面宿主进程启动时继承的 `PATH`：

```json
{
  "hostRequirements": {
    "minimumVersion": "0.2.82",
    "capabilities": ["connector.managed-tool-dependencies.v1"]
  },
  "managedToolDependencies": [
    {
      "id": "baijimu-cli",
      "minimumVersion": "0.1.45",
      "requiredFor": ["install", "start"],
      "executablePathEnv": "EXAMPLE_BAIJIMU_BINARY"
    }
  ]
}
```

- `managedToolDependencies` 仅允许用于 `schemaVersion: "3.0.0"`，并要求宿主能力 `connector.managed-tool-dependencies.v1`。
- `id` 引用已登记官方托管工具的 `appId`；`minimumVersion` 是 Connector 实际调用契约所需的最低 SemVer，不能写成随发布变化的“当前最新版”。
- `requiredFor` 可包含 `install` 和 `start`。宿主必须在进入对应阶段前串行完成工具安装或修复、版本校验和真实可执行性检查；失败时必须停止该阶段，不能继续安装或启动 Connector。
- `executablePathEnv` 是宿主在每次启动 Connector 时注入的变量。值必须是宿主动态解析并验证过的稳定 launcher 绝对路径；不得持久化到 Connector 配置，也不得允许 `runtime.env` 覆盖。
- `install` 门禁发生在包下载、签名和清单校验之后、停止或替换现有应用之前；因此依赖失败不会破坏已安装版本。`start` 门禁发生在创建 Connector 进程之前，覆盖首次安装后的自动启动、手动启动、升级重启和客户端恢复启动。
- 官方托管工具可随 Bridge Agent 安装包提供内置副本，也可独立升级；依赖解析始终采用托管工具状态与版本目录作为源事实。内置副本只负责首次引导和修复，不能覆盖更高版本。

## Connector 包结构

一个 Connector 包必须至少包含：

```text
connector-root/
  connector.json
```

可以包含：

```text
connector-root/
  package.json
  bin/
  dist/
  ui/
    index.html
    assets/
  README.md
  LICENSE
```

要求：

- `connector.json` 必须位于包根目录。
- 包内路径必须使用相对路径，不依赖安装前的源码绝对路径。
- 安装后，百积木会把包复制到本机 `local-apps` 目录；运行命令应以安装后的包路径为准。
- 不要把用户 token、cookie、数据库副本或机器私有配置提交进 Connector 包。

## 官方托管工具

官方托管工具不要求 `connector.json`、`service-registration.json` 或 `service.method`。市场版本通过 `latestVersion.manifest` 声明：

```json
{
  "applicationType": "managed_tool",
  "releaseNotes": ["新增批量升级命令。", "修复 Windows PATH 检测。"],
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

`connector.json` 是 Connector 的主清单。当前且唯一接受的 schema 版本为严格 SemVer
`3.0.0`。旧版 `id`、`connectorId`、`services` 和 `serviceRegistrationFiles` 不再解析；升级到
Bridge Agent `0.6.0` 前由迁移工具完成数据归档，应用必须重新安装新清单。

必填字段：

- `schemaVersion`
- `appId`
- `name`
- `version`
- `runtime`
- `transport`
- `methods` 或 `events` 至少一个

推荐字段：

- `description`
- `publisher`
- `source`
- `runtime`
- `management`
- `setup`
- `ui`
- `upgradeReview`
- `configSchema`
- `database`
- `remoteCapabilities`
- `hooks`
- `permissions`
- `legacyAutostartLabels`

示例：

```json
{
  "schemaVersion": "3.0.0",
  "appId": "wecom",
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
    "startPolicy": "automatic",
    "processOwnership": "host",
    "command": "wechat-bridge-collector",
    "args": ["run"],
    "stopArgs": ["stop"],
    "healthCheck": {
      "type": "http",
      "path": "/health",
      "timeoutSecs": 2,
      "expectStatus": 200
    }
  },
  "transport": {
    "type": "http",
    "baseUrl": "http://127.0.0.1:18082"
  },
  "methods": [
    {
      "name": "searchMessages",
      "description": "Search local WeChat messages.",
      "path": "/invoke/searchMessages",
      "httpMethod": "POST",
      "timeoutSecs": 30,
      "input_schema": {"type": "object"}
    }
  ],
  "events": [
    {
      "name": "messageReceived",
      "description": "A local WeChat message was received.",
      "enabled": true,
      "payload_schema": {"type": "object"}
    }
  ],
  "upgradeReview": {
    "configuration": "declared",
    "interfaces": "declared",
    "database": "declared"
  },
  "configSchema": {
    "type": "object",
    "required": ["databasePath"],
    "properties": {
      "databasePath": {"type": "string"},
      "syncIntervalSecs": {"type": "integer", "default": 30}
    }
  },
  "database": {
    "engine": "sqlite",
    "schemaVersion": "3",
    "migrations": [
      {
        "id": "003-add-message-status",
        "fromVersion": "2",
        "toVersion": "3",
        "description": "为消息增加同步状态",
        "changes": [
          {
            "operation": "add_column",
            "target": "messages.sync_status",
            "description": "新增非空状态字段并回填历史数据",
            "destructive": false
          }
        ],
        "destructive": false,
        "rollback": "automatic",
        "downtime": "none"
      }
    ]
  },
  "ui": {
    "type": "embedded",
    "entry": "ui/index.html",
    "title": "管理",
    "defaultView": true
  },
  "remoteCapabilities": [
    {
      "name": "wechat.events.messageReceived",
      "risk": "high",
      "description": "Emit message events from the user's local WeChat data."
    }
  ]
}
```

### 升级审查契约

Bridge Agent 使用当前已安装 package 的不可变清单快照和市场目标版本的 `latestVersion.manifest`
计算升级差异。发布方是三类契约的唯一源事实：

- **配置变化**：来自 `configSchema`。客户端比较配置路径、类型、必填性、默认值和枚举值；新增必填项、
  删除配置项、类型变化或枚举收窄会标记为破坏性变化。
- **接口变化**：来自 `methods[]`、`events[]` 及其 `input_schema`、`payload_schema`、`responseMode`、
  `httpMethod` 和 `path`。新增接口、删除接口和契约修改分别展示，不能只发布方法名称列表。
- **数据库变化**：来自 `database`。`schemaVersion` 是 package 数据库 Schema 的当前版本；
  `migrations[]` 必须保留从仍受支持的旧 Schema 到目标 Schema 的完整有向迁移链。

`database.migrations[]` 字段规则：

- `id`、`fromVersion`、`toVersion`、`description` 必填，且同一清单内 `id` 唯一。
- `changes[]` 至少一项；每项必须声明 `operation`、`target`、`description`，并准确设置 `destructive`。
- `rollback` 只能是 `automatic`、`manual`、`unsupported` 或 `not_declared`。
- `downtime` 只能是 `none`、`brief`、`required` 或 `not_declared`。
- 只要 migration 或任一 change 具有破坏性、禁止回滚或必须停机，升级页就按破坏性变化展示。
- 从当前 `schemaVersion` 无法解析到目标版本的完整 migration 链时，升级页必须明确报告契约缺失，
  不能显示“数据库无变化”。

未声明某类契约表示“发布方未声明，无法判断”，不表示“无变化”。版本业务摘要属于市场发布记录，
不属于 schema `3.0.0` 的 `connector.json`，也不能替代上述结构化契约。

`upgradeReview` 必须对三类契约逐项声明 `declared` 或 `not_applicable`：

- `configuration: declared` 要求同时存在 `configSchema`；没有运行配置时使用 `not_applicable`。
- Connector 的 `interfaces` 必须为 `declared`，其 methods/events 就是接口源事实；不提供接口的
  managed tool 可以使用 `not_applicable`。
- `database: declared` 要求同时存在 `database`；package 不使用持久化数据库时使用 `not_applicable`。
- 省略 `upgradeReview` 且又没有对应契约时，客户端只能显示“未声明”，不能推断为“不适用”。

`runtime.startPolicy` 支持：

- `automatic`（默认）：健康检查失败时，Bridge Agent 可执行 `runtime.command + args[]`
  恢复应用。
- `manual`：仅允许用户从本机应用页显式启动。运行时重建、重连和健康检查都不会在后台
  执行启动命令。访问 macOS 受保护数据的 Connector 必须使用该策略，避免在用户未完成
  系统授权时触发 TCC 提示。

`runtime.processOwnership` 支持两种明确的进程所有权：

- `connector`（默认，兼容旧包）：`runtime.command + args[]` 是执行后退出的启动命令，
  Connector 自己维护后台进程。宿主只能通过 `stopArgs[]` 和健康检查间接控制它。
- `host`：`runtime.command + args[]` 必须是持续运行的前台进程。桌面宿主持有子进程句柄，
  把标准输出和错误写入 Connector 私有运行日志，在停止、升级、卸载和客户端退出时回收
  完整进程树。Windows 创建的运行进程、停止命令和环境准备命令都必须使用无控制台窗口
  标志；macOS 进程必须由已签名的百积木桌面宿主直接派生，以维持稳定的 TCC 权限归属。

新发布且使用 `processOwnership: "host"` 的 Connector 必须声明
`hostRequirements.minimumVersion >= 0.2.40` 和能力
`connector.process.host-managed.v1`。它必须提供非空的 `args[]` 前台入口，以及幂等的
`stopArgs[]`，供优雅停止和跨版本升级清理使用；即使优雅停止失败，宿主仍必须按进程树
强制回收。

需要系统权限的 Connector 应在 manifest 顶层声明：

```json
{
  "permissions": [
    {
      "id": "macos.fullDiskAccess",
      "title": "完全磁盘访问",
      "description": "读取应用沙盒中的本地数据。",
      "platforms": ["macos"]
    }
  ],
  "legacyAutostartLabels": ["com.example.old-launch-agent"]
}
```

Bridge Agent 会在应用详情页展示权限说明，并可打开对应系统设置。`legacyAutostartLabels` 用于升级/同步/卸载时停止并删除 Connector 旧版本遗留的 LaunchAgent；不得把新的长期进程生命周期重新交给 Connector 自建的登录项。

## 内嵌应用界面

Connector 可以随应用包发布构建后的 Web 前端。Bridge Agent 只支持 `ui.type = embedded`，并把该页面作为应用详情中的独立 tab 加载：

```json
{
  "schemaVersion": "3.0.0",
  "ui": {
    "type": "embedded",
    "entry": "ui/index.html",
    "title": "个性化设置",
    "defaultView": true
  }
}
```

字段规则：

- `entry` 必须是应用包专用 UI 子目录内、使用 `/` 的相对 `.html` 路径，不能直接放在包根目录，也不能包含 `..`、绝对路径或符号链接逃逸。
- `entry` 所在目录是 UI 静态资源根目录；HTML 中应使用相对路径引用 JS、CSS、图片和字体。
- `title` 是宿主一级 tab 名称，长度为 1 到 64 个字符；使用“管理”“设置”“工作台”等简短名词，
  不重复应用名称；省略时显示“应用”。
- `defaultView = true` 时，用户点击应用默认进入自定义界面；否则仍默认进入“概览”。
- 前端可以使用 React、Vue、Svelte 等框架，但必须先构建为静态文件。客户端路由应使用 hash 路由。
- 页面 CSP 禁止内联脚本和直接网络请求；JavaScript 必须放在 UI 目录下并通过相对 `src` 引用。
- UI 文件随 Connector 安装、升级和回滚；运行期配置不得写回安装目录。

### 个性化主页展示规范

内嵌页面显示在宿主应用详情的 `ui.title` 一级 tab 内容区内，不是独立应用窗口。宿主拥有应用详情外壳，
包括应用图标、名称、类型、版本、能力数、描述、生命周期状态、更新、启动、停止、卸载以及“概览/能力/配置”
等宿主导航。个性化主页必须直接从当前业务状态、内容或操作开始，不得重复这些基础信息。

- 页面根部不得再次展示应用图标、应用名、版本、能力数、“已安装应用”等身份 Hero，也不得复制宿主的
  检查更新、启动、停止或卸载操作。
- 页面可以展示该应用专有的初始化、连接、账号、同步、诊断和业务操作；“刷新状态”只刷新页面拥有的业务
  状态，不替代宿主的生命周期刷新。
- 默认用同页 section/card 组织少量相关管理域。只有每个子视图都足够复杂且需要互斥切换时才使用二级
  tab；二级导航必须弱于宿主一级 tab，并使用 `tablist/tab/tabpanel`、`aria-selected`、`aria-controls`
  和键盘方向键语义。
- 存在先后依赖的安装或初始化流程使用步骤条，不使用 tab；tab 只表达可自由切换的平级内容。
- 页面背景、圆角和留白应与内容区协调，不再绘制覆盖整个 iframe 的第二层应用外壳；从 320px 宽度起保持
  可用，并避免固定视口高度造成双重滚动。
- 标题层级从业务 section 的 `h2` 开始；宿主中的应用名承担页面 `h1` 语义。弹窗标题按自身层级使用。

Bridge Agent 会给入口 HTML 自动注入 `window.baijimuLocalApp`：

```js
const settings = await window.baijimuLocalApp.invoke("getSettings");
await window.baijimuLocalApp.invoke("saveSettings", {
  theme: "dark",
  syncIntervalMinutes: 15
});
```

`invoke` 只能调用同一 Connector 在 `management.operations` 中声明的操作。应用页面不能直接获得 Tauri IPC、文件系统、relay 或任意本机 HTTP 权限。Bridge Agent 对页面消息来源、操作名、请求大小和响应大小进行校验，再使用 Connector 私有 token 调用应用后端。

应用后端负责校验并原子写入 `BAIJIMU_LOCAL_APP_DATA_DIR` 下的配置。安装目录视为只读；升级必须保留数据目录，卸载时是否清理数据必须由用户单独确认。

命名要求：

- `appId` 必须等于平台注册表 `local_app.id`。创建应用时调用方不得传入身份；服务端在同一事务中生成数据库主键并返回。
- 存量应用保留已有主键（例如 `codex`、`wecom`），新应用通常取得 UUID。两者都只是同一数据库主键类型，不存在第二套 Connector 身份或命名空间。
- `appId` 一旦登记不得因为仓库迁移、展示名称变化或实现重写而改变。
- `permissions[].id` 必须使用 ASCII 字母、数字、点、短横线或下划线；既有权限 ID
  `macos.fullDiskAccess` 属于稳定协议标识，宿主必须保持兼容。
- `name` 是展示名，可以变化。
- `version` 应使用 SemVer。市场版本和 Connector 包版本必须一致。

## 本地应用能力声明

Schema `3.0.0` 在 `connector.json` 顶层直接声明 `transport`、`methods` 和 `events`，
一个 Connector 安装实例就是一个本地应用，不再为了远程调用或事件订阅创建 service。
以下字段直接放在 `connector.json`：

```json
{
  "transport": {
    "type": "http",
    "baseUrl": "http://127.0.0.1:18082"
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

- `methods[].name` 和 `events[].name` 必须稳定；删除或改名属于破坏性变更。
- `transport.baseUrl` 默认应绑定 `127.0.0.1`，不要要求用户暴露公网端口。
- `runtime.healthCheck` 应能快速判断本地应用是否可用。
- `processOwnership: "connector"` 时，`runtime.command + args[]` 应在触发启动后退出。
- `processOwnership: "host"` 时，`runtime.command + args[]` 必须保持前台运行，禁止再次
  派生自守护进程或安装系统级自启动项。
- `runtime.command + stopArgs[]` 应尽量幂等；应用未运行时也应安全退出。
- `input_schema` 应尽量收紧，不要长期使用完全开放的 `additionalProperties: true` 作为正式能力接口。

同一设备上一个 `appId` 只允许一个实例。Bridge Agent 以 Connector 独立事件凭证
校验本机事件来源，Relay 和平台以 `deviceId + appId` 唯一识别本地应用，不再维护
额外的安装实例身份。

## 市场元数据

平台注册事务生成 `local_app.id`，并把它作为 API、清单、Bridge、设备授权和事件链路唯一的
`appId`。调用方创建应用时不提交 ID；服务端返回后，开发者把该值写入版本化应用目录和
`connector.json`。存量短 ID 原样保留，新建应用使用数据库生成的 UUID。

市场服务 `local-app-market` 提供两类读取：

```text
GET {platform.base_url}/api/local-app-market/apps?platform={macos|windows|linux}
GET {platform.base_url}/api/local-app-registry/apps/{appId}/versions/{version}
```

第一类只返回已审核且公开上架的版本；第二类精确解析 `ACTIVE` 的已登记版本，供 GitHub
直接安装、测试和未审核分发。撤销应用或版本注册后，精确解析也必须立即拒绝。

公开列表可以返回 lowcode 包装结构：

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
  "appId": "wecom",
  "name": "微信",
  "description": "安装微信本地采集 connector，把微信相关本地能力接入工作区。",
  "publisher": "Baijimu",
  "risk": "需要读取本机微信数据库、联系人和消息记录目录，只在用户本机运行。",
  "riskLevel": "high",
  "capability": "本地微信消息查询、搜索和消息事件采集。",
  "platforms": ["macos"],
  "latestVersion": {
    "version": "0.2.3",
    "sourceType": "https",
    "source": "https://download.example.com/local-apps/wecom-0.2.3.zip",
    "repo": "momoplan/wechat-bridge-collector",
    "revision": "v0.2.3",
    "checksum": "<64 lowercase hexadecimal SHA-256>",
    "capabilities": [
      "wechat.messages.read",
      "wechat.messages.search",
      "wechat.events.messageReceived"
    ],
    "manifest": {
      "schemaVersion": "3.0.0",
      "appId": "wecom",
      "name": "微信",
      "version": "0.2.3",
      "runtime": {
        "type": "process",
        "command": "wecom-connector",
        "args": ["start"],
        "stopArgs": ["stop"],
        "processOwnership": "host"
      },
      "hostRequirements": {
        "minimumVersion": "0.6.0",
        "capabilities": ["connector.process.host-managed.v1"]
      },
      "transport": {"type": "http", "baseUrl": "http://127.0.0.1:18082"},
      "upgradeReview": {"configuration": "declared", "interfaces": "declared", "database": "declared"},
      "configSchema": {"type": "object", "properties": {}},
      "methods": [{"name": "searchMessages", "description": "Search messages.", "path": "/invoke/searchMessages", "httpMethod": "POST", "input_schema": {"type": "object"}}],
      "events": [{"name": "messageReceived", "description": "Message received.", "payload_schema": {"type": "object"}}],
      "database": {"engine": "sqlite", "schemaVersion": "3", "migrations": []}
    },
    "publishedAt": "2026-06-18T10:00:00Z"
  }
}
```

要求：

- `appId` 就是市场表主键，不再存在另一套市场 ID、Connector ID 或安装实例 ID。
- `appId` 必须等于 Connector 包内 `connector.json.appId`。
- `latestVersion.version` 必须等于 Connector 包内 `connector.json.version`。
- `latestVersion.revision` 推荐指向不可变 tag，例如 `v0.2.3`。
- GitHub 直接安装只负责发现和打包指定 revision；Bridge 随后仍须通过精确注册接口核对
  `appId + version + repo + revision + checksum`，并从登记来源重新下载。未登记内容不得执行。
- 正式制品必须使用 HTTPS、精确 revision 和 SHA-256；按平台/架构分发时由
  `manifest.artifacts[]` 声明唯一匹配制品。
- `latestVersion.manifest` 必须是该不可变 package 版本的完整清单快照，不能只保留启动命令摘要；
  配置、接口和数据库升级审查都以此为目标版本源事实。
- `platforms` 必须准确表达支持平台；不要把只支持 macOS 的 Connector 标成 Windows/Linux 可用。
- `riskLevel` 建议使用 `low`、`medium`、`high`。

## 安装和更新

安装流程：

1. 用户从公开市场选择应用，或提供已登记版本的 `appId + version` / Git revision。
2. Bridge 调用注册表精确解析版本，确认应用和版本均为 `ACTIVE`。
3. Bridge 从登记的 HTTPS 制品源重新下载并验证 SHA-256；macOS/Windows 继续验证平台签名。
4. Bridge 读取 `connector.json`，严格校验 schema `3.0.0`、`appId`、版本、来源和宿主能力。
5. Bridge 安装到本机 `local-apps/{appId}/{version}`，私有数据始终位于 `app-data/{appId}`。
6. Bridge 把本地应用定义写入 `agent-config.json.localApps[]`，生成私有 token，准备运行环境。
7. Bridge 刷新 runtime registry，并通过已有 WebSocket 重新上报 capabilities。

更新规则：

- 用市场 `appId` 找到本机已安装 Connector。
- 比较本机 `connector.json.version` 与市场 `latestVersion.version`。
- 更新时重新安装同一个 `appId`，并替换该 Connector 的本地应用定义。
- 更新不得悄悄迁移到另一个 `appId`。
- 如果服务名、方法名、事件名发生破坏性变更，必须升级主版本，并在市场风险说明中写清楚。
- 升级前必须展示配置、接口、数据库和权限的结构化差异；缺少声明时必须展示“无法判断”。

来源同步规则：

- `sourceReference` 只记录已登记的来源信息；它不能成为绕过注册表的信任依据。
- Git 同步重新发现指定 revision 后，必须再次精确解析注册表并核对 repo、revision 和 checksum。
- 重新同步只允许安装同一 `appId` 的已登记版本，并更新 `lastSyncedAtEpochMs`，保留首次 `installedAtEpochMs`。

卸载规则：

- 卸载 Connector 时，删除安装记录和该 Connector 的本地应用定义。
- 应用私有数据是否删除必须由用户单独确认；默认保留登录态和授权配置。
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

1. Connector 在 schema `3.0.0` 清单顶层声明 `events[]`。
2. Bridge Agent 启动 Connector 时注入
   `BAIJIMU_LOCAL_APP_EVENT_ENDPOINT` 和
   `BAIJIMU_LOCAL_APP_EVENT_TOKEN_FILE`。
3. Connector 读取独立事件凭证，调用
   `POST /v1/local-app-events`，传入 `appId`、`event`、`payload`，可选
   `eventId` 和 `occurredAt`。
4. Bridge Agent 校验凭证、安装实例和事件声明，通过 WebSocket 同步转发
   `local_app_event_emitted`；它不保存事件。
5. Relay 校验设备和 capability 后同步转发给 Event Center，也不保存事件。
6. Event Center 先匹配订阅：没有匹配项时确认忽略且不保存 payload；存在匹配项时，
   在一个事务中完成事件去重和投递任务持久化后返回 `event_ack`。
7. Bridge Agent 收到 `event_ack` 后才返回 `202 Accepted`。断线、超时或下游失败返回
   非 2xx；Connector 必须保留自己的源游标，并使用同一个 `eventId` 重试。
8. 订阅、指定设备范围、Webhook 重试和投递记录全部由 Event Center 维护，Relay
   不保存设备事件订阅表，也不直接触发 Webhook。

Bridge Agent 启动时会删除旧版本遗留的专用 `event-outbox` 目录；该目录中的事件不再
重放。Connector 自己拥有的源游标和源数据不属于此清理范围。

调用示例：

```bash
curl -X POST "$BAIJIMU_LOCAL_APP_EVENT_ENDPOINT" \
  -H "Authorization: Bearer $(tr -d '\\r\\n' < "$BAIJIMU_LOCAL_APP_EVENT_TOKEN_FILE")" \
  -H "Content-Type: application/json" \
  -d '{
    "appId": "wecom",
    "event": "messageReceived",
    "eventId": "evt-01J...",
    "payload": {"conversationId": "c-1"}
  }'
```

事件 payload 应保持结构化，并避免发送无边界的大对象。需要传大文件时，应走文件引用或上传协议。

## 兼容性

稳定接口：

- `connector.json.appId`（即 `local_app.id`）
- `connector.json.version`
- `deviceId + appId` 安装实例语义
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

- 改名或删除 Connector、方法、事件。
- 收紧输入 schema 导致旧调用失败。
- 改变事件 payload 必填字段。
- 改变权限边界，例如从只读查询变成消息监听。

破坏性变更必须提升主版本。

## 本地开发和 GitHub 直接安装

本地开发不再等同于“未登记安装”。开发者先在平台创建应用取得服务端 `appId`，再为每个测试版本
登记精确 repo、revision、HTTPS 制品和 SHA-256。版本可以保持未审核状态，因此不出现在公共市场；
Bridge 仍可通过精确注册接口安装和同步。

GitHub URL 是输入方式，不是信任来源：Bridge 可以浅克隆 revision 读取元数据，但在执行任何包内代码前
必须用清单中的 `appId + version` 查询注册表，校验 repo/revision/checksum，并从登记源重新下载。
本地目录、任意压缩包和没有注册记录的 Git revision 都必须拒绝。这样保留开发效率，同时保证所有
可执行本地应用都有可撤销的服务端身份和版本记录。

## 发布前验收清单

Connector 发布到市场前至少确认：

- `connector.json` 可以被百积木解析。
- `connector.json.appId` 与市场表主键 `local_app.id` 一致。
- `connector.json.version` 与市场 `latestVersion.version` 一致。
- 市场版本摘要逐条说明该版本对用户可感知的新增、修复和破坏性变化；它不写入严格 schema `3.0.0` 清单。
- `configSchema`、`methods/events` 和 `database` 与真实 package 行为一致；数据库版本变化时存在从所有受支持旧版本到目标版本的完整 migration 链。
- Git tag 或 revision 存在、可被浅克隆，且与注册表中的 repo、revision、制品 SHA-256 完全一致。
- 安装后本地应用定义能写入 `agent-config.json.localApps[]`，不会写入 runtime service。
- `runtime.command + args[]` 在 `processOwnership: host` 下是持续运行的前台进程，并可由幂等 `stopArgs[]` 优雅停止。
- `healthCheck` 通过。
- 方法能通过百积木调用。
- 事件能通过百积木本机事件入口上报。
- 卸载只删除该 Connector 的本地应用定义。
- `risk`、`riskLevel`、`platforms` 与真实行为一致。

## 当前实现约束

当前百积木实现中：

- Connector 清单文件名固定为 `connector.json`。
- 清单只接受 `schemaVersion: 3.0.0` 和 `appId`；旧身份字段及 service-registration 模型全部拒绝。
- 安装只接受注册表精确解析成功的版本。Git URL 可以用于发现 revision，但不能绕过登记和 checksum 校验。
- 市场列表既支持 lowcode 包装结构，也支持直接返回数组。
- 客户端比较新旧 `configSchema`、methods/events 完整契约、database migration 和权限声明，并按兼容、需确认、破坏性三档展示。
- 安装包写入 `local-apps`，安装记录包含 `sourceReference`、`sourcePath`、注册状态、审核状态、来源 checksum、包 checksum、`installedAtEpochMs` 和 `lastSyncedAtEpochMs`。
- Connector 顶层至少声明一个方法或事件；`transport` 目前支持 loopback HTTP。
- Bridge Agent 为每个 appId 在 `app-data/{appId}` 生成私有 token，并为健康检查和 HTTP 方法自动注入 Bearer 鉴权。
- 桌面端在配置目录写入权限为 `0600` 的 `local-app-control.json`，内容包含进程 ID、随机令牌和 loopback HTTP 地址；控制服务正常停止时删除，重启时覆盖并轮换令牌。
- `baijimu local-app install` 和 `baijimu local-app device ...` 必须通过该本机控制面调用桌面端实现，不得自行写 Connector 目录或复制安装算法。
- 本机控制面支持市场查询、已安装应用查询、安装、启动、停止、来源同步、卸载以及清单声明的 management operation；所有请求必须携带发现文件中的 Bearer token。
- 本机控制面只允许绑定 loopback；CLI 读取发现文件后必须再次校验 URL，禁止向远端主机发送本机控制 token。

后续如果新增包签名、checksum、压缩包分发或沙箱运行，应在本文中扩展对应章节。
