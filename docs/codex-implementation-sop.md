# 企业 Codex 初始用户实施 SOP

本文面向平台实施、客户成功、平台运维和企业管理员。普通最终用户不需要阅读本文。

实施 SOP 的目标是完成企业初始用户的组织开通、设备授权、Codex App 自动安装、Codex 终端能力可选开通、端到端验证和交付验收。实施人员不应手工为用户生成 Codex 密钥；标准流程应由平台后台、Agent 对话安装流程和百积木本机命令能力自动完成安装、凭证生成、模型绑定、router 配置和校验。

## 0. CLI、router 与用户自主安装目标

现有 `baijimu-codex-windows-install` 技能里的 CLI 和 router 是实施/运维视角的底层概念，不是普通用户需要理解或操作的产品入口。

| 概念 | 在现有技能中的含义 | 在用户自主安装目标中的定位 |
| --- | --- | --- |
| Codex App | OpenAI 官方桌面应用。 | 用户使用 Codex 设备安装能力时的首选安装对象。由平台 Agent 通过百积木的 `shellExec.shellExec` 自动安装，用户不手工下载安装包。 |
| Codex CLI/Terminal | 用户设备上的终端能力，用于企业 router、计量、CLI 验证或终端工作流。 | 仅在用户要求“Codex 终端”或企业产品流需要 router-backed 终端能力时配置。普通用户不手工执行 CLI 安装命令。 |
| router | `https://router.baijimu.com/api/claudecode/v1`，Codex 终端能力访问模型服务的统一 API 网关。它负责接收用户凭证、校验用户/工作区/项目归属、选择系统模型密钥、转发到真实模型服务，并写入审计和计量链路。 | 对普通用户不可见。仅在启用终端能力时由平台自动配置到终端工具或客户端认证机制中。 |
| lowcode-apikey | 底层用户凭证和系统模型密钥绑定服务。 | 对普通用户不可见。只在配置 Codex 终端能力时由平台自动开通流程调用，或由运维在异常排查时查看脱敏绑定状态。Codex App 安装本身不需要调用它。 |
| relay `shellExec` | 通过已连接的百积木在用户设备上执行安装和验证命令。 | 是普通用户自助安装 Codex App/终端能力的标准执行通道，但必须经过用户授权和 allowlist 控制。 |

用户自主安装的目标流程应是：

1. 用户安装并打开 bridge-agent。
2. 用户完成企业账号授权。
3. 百积木获得设备授权上下文和本机能力授权。
4. 授权完成后，用户在百积木首页点击“打开控制台”，进入百积木控制台并打开 Agent 对话页面，输入“请使用 Codex 设备安装能力，给当前工作区的设备安装 Codex。”
5. 系统自动搜索并加载公开市场中的 Codex 设备安装能力，通过 `shellExec.shellExec` 完成 Codex App 安装和验证。
6. 如果企业需要 Codex 终端能力，系统继续自动完成凭证写入、router 配置、启动和健康检查。
7. 用户在对话中输入“确认 Codex 是否安装成功”完成首次验证。

因此，App 安装命令、CLI、router、lowcode-apikey 和 relay `shellExec` 都应该被产品化流程封装起来。普通用户只看到 百积木、设备授权、平台 Agent 对话和 Codex 安装状态。

### 0.1 当前链路能否走通

按现有设计，这条链路可以走通，但必须同时满足以下条件：

| 环节 | 必须具备的能力 | 责任方 |
| --- | --- | --- |
| 用户授权 | bridge-agent 授权成功后拿到 `desktopChatSession`、`projectId`、`agentConfigId` 和设备上下文。 | bridge-agent 与平台授权服务 |
| AI 识别安装意图 | 用户输入“请使用 Codex 设备安装能力，给当前工作区的设备安装 Codex。”或“安装 Codex 终端”后，对话服务能自动搜索并加载 Codex 设备安装能力，而不是普通聊天回复。 | 对话服务/agent 编排 |
| 安装 Codex App | AI 通过当前设备暴露的 `shellExec.shellExec` 方法执行官方 Codex App 安装、启动和验证命令。 | bridge-agent/relay/安装技能 |
| 写入百积木 CLI token | 仅当需要 Codex 终端能力时，bridge-agent 客户端在设备授权阶段把当前工作区的百积木本地 CLI token 写入 `~/.config/baijimu/auth.json`。 | bridge-agent 客户端 |
| 配置 router | 仅当需要 Codex 终端能力时，router 地址固定为 `https://router.baijimu.com/api/claudecode/v1`，由安装脚本消费本机 CLI token 并自动写入 Codex 终端配置；`model`、`model_provider`、权限默认值必须写在 `~/.codex/config.toml` 根节点顶部。 | 安装工具链/技能 |
| 设备执行配置 | AI 通过当前设备暴露的 `shellExec.shellExec` 方法执行配置写入、启动和健康检查。 | bridge-agent/relay |
| 结果校验 | 安装后校验 Codex App 状态和可见窗口；需要终端能力时，再通过 router `/responses`、凭证、计量日志确认归属一致。 | 安装工具链/平台日志 |

如果上述能力都已上线，用户自主安装可以不需要实施人员介入。实施人员只需要在失败时看链路卡在哪一层。

如果还没有完全上线，最可能的缺口通常是：

- lowcode-apikey 还没有以市场模块方法暴露给 AI 调用。
- “Codex 设备安装能力”无法从公开市场搜索或会话内加载。
- `shellExec.shellExec` 没有授权给当前对话 agent，或命令 allowlist 不允许安装命令。
- 安装工具链还没有把 Codex App 安装、router、凭证和终端配置写成一个幂等流程。

### 0.2 平台技能与 `shellExec` 权限边界

“Codex 设备安装能力”应该沉淀为平台市场技能或平台工具链能力，而不是写死在百积木前端，也不是让普通用户照着下载页或 CLI 文档手工执行。平台技能负责把用户意图编排成可审计的自动流程：

1. Agent 根据用户话术搜索公开市场中的 Codex 设备安装能力，并在当前会话内加载技能说明。
2. 读取当前授权上下文，包括用户、工作区、项目、agent 配置、对话 session 和设备。
3. 检查当前设备是否已暴露 `shellExec.shellExec`，以及安装所需命令是否在 allowlist 内。
4. 在用户明确授权后，通过 `shellExec.shellExec` 安装 Codex App，并执行启动和安装验证。
5. 如果需要 Codex 终端能力，由客户端授权流程先把当前工作区的百积木本地 CLI token 写入 `~/.config/baijimu/auth.json`。
6. 由 Codex 安装技能消费这个本机 CLI token，并使用固定 router 地址 `https://router.baijimu.com/api/claudecode/v1` 生成终端配置。
7. 重启或打开 Codex App，并确认主进程、app-server 和可见窗口都存在。
8. 返回安装结果；涉及终端能力时，在平台侧校验 router、凭证、计量和审计归属。

`shellExec.shellExec` 是本机命令执行能力，必须按用户授权和 allowlist 运行。安装技能不应尝试绕过权限。如果当前 Agent 没有执行权限，或者 allowlist 不允许安装命令，技能应该明确提示用户在百积木中开启对应本机能力，并说明这次授权用于安装和验证 Codex。

权限引导应包含：

- 需要开启的本机方法：`shellExec.shellExec`。
- 需要允许的命令范围：Codex App 官方安装器下载/启动、OSS fallback 下载、SHA256 校验、应用安装/启动、可见窗口验证；如果需要终端能力，再包括终端配置写入、版本检查、router `/responses` 健康检查。
- 不展示、不复制、不要求用户保存 API Key。
- macOS 的“完全磁盘访问”“辅助功能”“屏幕录制”等系统隐私权限不能由平台静默开启。如果需要这些权限，应提示用户到系统设置中授权。
- 如果企业安全策略禁止本机命令执行，应停止自动安装，转为企业 IT 或实施人员处理，不临时放宽权限。

第一阶段建议把 allowlist 控制在安装必需命令内。Windows App 安装通常需要允许 `powershell`、`winget`、安装器下载/启动命令和安装验证命令；macOS App 安装通常需要允许 `uname`、`curl`、`hdiutil`、`ditto`、`open`、`mdls`、`xattr`、`shasum`、`pkill`、`pgrep`、`lsappinfo`、`osascript`、`screencapture`、`sips` 等命令。具体命令清单应由平台安装技能维护，并随 Codex 安装方式更新。

## 1. 实施前信息确认

安装前必须确认身份映射，不应猜测用户、工作区、设备或项目归属。

| 项目 | 说明 | 来源 |
| --- | --- | --- |
| 企业名称 | 客户组织名称 | 商务合同或企业管理员 |
| 组织 ID | 平台内组织标识 | 平台后台 |
| 工作区 ID | 初始用户所在工作区 | 平台后台 |
| 项目 ID | Codex 请求要归属和计量的项目 | 平台后台 |
| 用户 ID | 初始用户在平台内的用户标识 | 平台后台 |
| 用户邮箱/手机号 | 登录、通知和成员匹配使用 | 企业管理员 |
| 设备 ID | bridge-agent 或连接设备标识 | 设备授权记录 |
| 本机系统用户名 | Windows/macOS/Linux 用户名 | 终端命令或客户提供 |
| 网络策略 | 是否允许访问平台域名、relay、router 和模型服务 | 企业 IT |

如果上述映射存在歧义，应先与客户管理员确认，再启动自动开通。

## 2. 平台侧准备

实施人员在平台后台完成：

1. 创建或确认企业组织。
2. 创建或确认默认工作区。
3. 添加初始用户，并赋予组织管理员、工作区管理员或试点用户权限。
4. 创建或确认 Codex 归属项目。
5. 确认该企业有可用模型、额度和计量策略。
6. 确认百积木公共下载页、百积木平台 Agent 对话页和公开市场中的 Codex 设备安装能力可用。

验收标准：

- 初始用户可以登录平台。
- 初始用户在目标工作区内可见。
- 目标项目存在且状态可用。
- 企业模型策略和额度策略已生效。

## 3. 自动开通 Codex 凭证与模型绑定

企业版标准流程应由平台自动完成：

1. 管理员或实施人员在平台后台选择企业、工作区、项目和初始用户。
2. 平台校验用户、工作区、项目、设备授权关系是否一致。
3. 平台自动为该用户创建或复用 Codex 专用凭证。
4. 平台自动把凭证绑定到正确的用户、工作区和项目。
5. 平台按企业策略自动绑定可用的系统模型密钥、模型范围和额度策略。
6. 平台自动调用凭证校验能力，确认返回的用户、工作区、项目和模型路由均正确。
7. 平台把凭证写入对话安装流程、Codex 终端配置或客户端认证机制。

lowcode-apikey 是底层鉴权服务，不是实施人员的常规操作入口。只有自动开通失败、身份映射异常、模型绑定异常或需要吊销/轮换凭证时，平台运维或授权实施人员才进入后台排查。

安全要求：

- 不在交付文档、截图、聊天记录或工单中粘贴完整密钥。
- 对客户展示时只展示脱敏凭证标识。
- 后台排查只查看脱敏密钥标识、绑定关系、状态和校验结果。
- 如果发现用户、工作区、项目不一致，停止开通并修正映射，不创建临时默认项目或兜底账号。

## 4. 指导用户安装 bridge-agent

工作区用户侧动作详见 [Codex 工作区用户安装与使用指南](./codex-user-quickstart.md)。个人用户只需要连接设备时，使用 [百积木个人用户安装指南](./bridge-agent-user-install.md)。

实施人员需要确认：

1. 用户从百积木公共下载页获取正确安装包。
2. 用户完成 bridge-agent 安装和浏览器授权。
3. 设备出现在正确工作区下。
4. 设备在线或最近在线。
5. relay 能看到该设备连接或心跳记录。

验收标准：

- bridge-agent 正常启动。
- 设备归属到正确用户和工作区。
- 设备授权状态正确。

## 5. 指导用户安装 Codex

推荐路径是用户打开百积木完成设备授权，然后在百积木首页点击“打开控制台”，进入百积木控制台并打开 Agent 对话页面，输入“请使用 Codex 设备安装能力，给当前工作区的设备安装 Codex。”系统会自动搜索并会话内加载公开市场技能，再通过 `shellExec.shellExec` 完成 Codex App 安装、启动和验证。百积木当前没有内置对话输入框，“打开控制台”按钮会打开平台控制台入口。

如果企业环境还需要百积木 router-backed 的 Codex 终端能力，用户或管理员可以继续输入“安装 Codex 终端”，由同一技能自动完成终端工具安装、凭证写入、router 配置和健康检查。

实施人员需要确认：

- 用户授权后可以在百积木首页点击“打开控制台”，并在百积木控制台进入 Agent 对话页面。
- 用户输入“请使用 Codex 设备安装能力，给当前工作区的设备安装 Codex。”后，系统能搜索并加载 Codex 设备安装能力，启动安装流程。
- Codex App 已安装、可启动，并且有可见窗口。仅有进程或菜单栏不算完成。
- 如果启用 Codex 终端能力，终端工具已安装、启用并通过健康检查。
- 如果启用 Codex 终端能力，终端配置指向企业 router。
- 如果启用 Codex 终端能力，对话安装流程、终端工具或客户端认证机制使用初始用户专属凭证。
- 配置中没有平台管理员密钥、系统密钥或其他客户密钥。

企业 router 地址：

```text
https://router.baijimu.com/api/claudecode/v1
```

现有 Codex 设备安装能力应作为平台市场技能使用：用户不需要手动安装技能，也不需要把技能预装到默认助理；Agent 应在会话中搜索、加载并执行该能力。技能优先通过 `shellExec.shellExec` 安装 Codex App；需要终端能力时，再创建用户专属凭证、配置 router 并验证计量归属。

终端配置的关键验收点：

```toml
model = "gpt-5.4"
model_provider = "baijimu-router"
sandbox_mode = "danger-full-access"
approval_policy = "on-request"

[model_providers.baijimu-router]
base_url = "https://router.baijimu.com/api/claudecode/v1"
wire_api = "responses"
requires_openai_auth = true
```

这些根级配置必须位于 `~/.codex/config.toml` 的第一个 TOML 表之前；不能追加到 `[marketplaces.*]`、`[desktop]` 或 `[projects.*]` 表内部。Codex 终端使用的本地凭证由安装脚本从客户端已写入的百积木本地 CLI token 转写到 `~/.codex/auth.json`，权限为 `600`，不得在日志或对话中显示完整值。

命令行试点或排障时，可以由授权实施人员在用户设备上写入自动生成的凭证：

```bat
setx OPENAI_API_KEY "<generated-user-key>"
```

这是试点或排障路径。企业版普通用户的标准入口是平台 Agent 对话；当前标准安装对象包括 Codex App，终端能力按企业产品流按需启用。

## 6. 端到端验证

用户侧验证：

```text
确认 Codex 是否安装成功
```

平台侧验证：

1. 查询 router 请求日志。
2. 查询平台凭证校验记录；必要时再进入底层鉴权后台排查绑定。
3. 查询模型调用计量记录。
4. 查询 bridge-agent 或设备在线状态。
5. 确认请求解析到同一个企业、工作区、用户、设备和项目。

验收标准：

- 对话返回 Codex 已安装、已启用或健康检查通过。
- router 记录显示请求来自初始用户专属凭证。
- 工作区、用户、项目归属正确。
- 计量记录写入目标项目。
- 没有落到平台管理员账号、默认测试账号或其他企业账号。

## 7. 问题排查

| 问题 | 优先排查点 | 处理方式 |
| --- | --- | --- |
| 平台 Agent 对话页不可用 | 授权结果、desktopChatSession、projectId、agentConfigId | 先修复授权结果，再让用户重进对话。 |
| Codex 设备安装能力无响应 | 对话通道、市场技能搜索、会话内技能加载、Codex 安装任务 | 检查对话服务、`search_skill`/`use_skill` 调用和安装任务日志。 |
| Codex App 安装失败 | `shellExec` 授权、allowlist、官方安装器下载、系统安装策略 | 先确认百积木权限，再看企业终端管控或系统安装提示。 |
| 官方下载失败 | 用户网络到 GitHub、ChatGPT、oaistatic 的连通性；OSS fallback manifest | 切换百积木 OSS 官方资产缓存，校验 SHA256 后安装，不使用第三方镜像。 |
| Codex App 启动后无窗口 | App 主进程、app-server、`lsappinfo windows`、桌面截图 | 清理残留进程后重开；如果 `windows=[ NULL ]`，记录为 App 窗口创建问题，不算安装完成。 |
| Codex 终端健康检查失败 | 终端安装状态、启动命令、router 地址、凭证写入状态 | 先校验平台凭证，再确认终端配置。 |
| 认证失败 | 用户凭证、router 地址、对话安装流程或终端配置写入状态 | 先用用户专属 key 直接请求 router `/responses`；如果 HTTP 200，再检查本机 TOML 根配置和 App 状态。 |
| Codex 忽略 router | `~/.codex/config.toml` 顶部根配置 | 确认 `model` 和 `model_provider` 位于第一个 `[table]` 之前，不在 marketplace/project 表下。 |
| macOS 配置脚本弹出 Xcode 工具安装 | 安装脚本是否调用 `python3` | 改用 shell/awk/plutil/osascript，不要求用户安装开发者工具。 |
| 请求归属错误 | 用户/工作区/项目映射、凭证绑定 | 停止继续试用，修正身份映射后重新自动开通或轮换凭证。 |
| bridge-agent 不在线 | 授权状态、relay 网络、设备 token | 重新授权或检查企业网络放行。 |
| 企业网络拦截 | 域名、代理、TLS 检查、防火墙 | 提交网络放行清单给企业 IT。 |
| 模型不可用 | 系统模型密钥、额度、供应商状态 | 修复模型绑定或额度后再次验证。 |

排查原则：

- 先确认身份映射和路由归属，再处理体验层问题。
- 不使用平台管理员密钥帮用户通过验证。
- 不在根因不清楚时增加兜底账号、默认项目或缓存状态。
- 如果涉及企业网络、安全策略或终端管控，应明确是客户 IT 侧配置还是平台侧配置问题。

## 8. 实施验收清单

交付完成后，实施人员和客户管理员共同确认：

- 企业组织已创建。
- 初始用户已加入正确工作区。
- Codex 项目已创建并可计量。
- 初始用户凭证已自动生成或复用，并通过校验。
- 初始用户设备已安装 bridge-agent。
- 设备已完成授权并在线。
- Codex App 已通过对话安装流程安装并可启动。
- Codex App 已打开并显示可见窗口；如果需要登录、授权或系统权限，已记录具体阻塞点。
- 如启用 Codex 终端能力，终端配置指向企业 router。
- 如启用 Codex 终端能力，`~/.codex/config.toml` 根配置、`~/.codex/auth.json` 权限和 router `/responses` 健康检查均通过。
- 用户侧对话验证通过。
- router、凭证校验、计量、审计日志归属一致。
- 企业管理员已收到管理员使用说明。
- 初始用户已收到 Codex 工作区用户安装与使用指南。
