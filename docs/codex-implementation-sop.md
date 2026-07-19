# 企业 Codex 初始用户实施 SOP

本文面向平台实施、客户成功、平台运维和企业管理员。普通最终用户不需要阅读本文。

实施目标是完成企业初始用户的组织开通、设备授权、ChatGPT desktop app 与 Codex 终端自动安装、百积木路由配置、端到端验证和交付验收。安装流程不依赖任何 Codex 市场技能；公开执行文档和它引用的受控脚本是唯一执行规范。

## 0. 唯一安装入口与责任边界

用户入口：

```text
https://www.baijimu.com/docs/features/codex-user-install/
```

Agent 执行入口：

```text
https://www.baijimu.com/docs/features/codex-device-install/index.html
```

责任边界：

| 能力 | 定位 | 责任方 |
| --- | --- | --- |
| ChatGPT desktop app | OpenAI 官方桌面应用，Codex 位于其中，不保证显示为单独的 `Codex.app`。 | 公开安装脚本、用户设备 |
| Codex CLI/Terminal | 企业 router、计量、CLI 验证和终端工作流。 | 公开安装脚本、用户设备 |
| router | 固定为 `https://router.baijimu.com/api/claudecode/v1`，负责鉴权、路由、审计和计量。 | 百积木平台 |
| 本地 CLI token | bridge-agent 在设备授权阶段写入 `~/.config/baijimu/auth.json`，安装脚本只在设备本地消费。 | bridge-agent |
| `shell` | 在授权设备上执行安装和验证命令。 | bridge-agent/relay |
| 安装编排 | 按公开执行文档选择平台脚本、传入上下文并读取脱敏 JSON 结果。 | 平台 Agent |

禁止重新引入以下链路：

- 搜索、安装或会话内加载 Codex 设备安装市场技能。
- 把旧技能、模型记忆或历史聊天中的安装步骤作为执行依据。
- 让用户复制 API Key、CLI token 或完整密钥。
- 调用 Codex 专用 Partner API、Codex 专用 CLI 命令或 `workspace-agent.createUserApiKey`。

## 1. 标准用户流程

1. 用户安装并打开百积木客户端。
2. 用户登录百积木账号并授权当前设备。
3. bridge-agent 写入当前工作区的本地 CLI token，并通过 relay 上报设备与本机能力。
4. 用户在百积木客户端首页点击“打开控制台”，进入 Agent 对话。
5. 用户复制个人安装页提供的指令；该指令要求 Agent 读取固定的 Codex 设备安装执行文档。
6. Agent 检查工作区、项目、用户、会话、设备和 `shell` 权限。
7. Agent 在目标设备用后台执行运行对应平台的一体化脚本，并轮询最终脱敏 JSON。
8. App、终端、路由、账号、窗口和归属全部验证通过后，Agent 才能报告完成。

## 2. 实施前信息确认

安装前必须确认身份映射，不应猜测用户、工作区、设备或项目归属。

| 项目 | 说明 | 来源 |
| --- | --- | --- |
| 企业名称和组织 ID | 客户组织及平台标识 | 商务合同、平台后台 |
| 工作区 ID | 初始用户所在工作区 | 当前授权上下文 |
| 项目 ID | Codex 请求归属和计量项目 | 当前 Agent 会话上下文 |
| 用户 ID | 当前认证用户 | 平台认证上下文 |
| Agent 配置和会话 ID | 安装请求来源 | 当前 Agent 会话上下文 |
| 设备 ID、系统和架构 | 目标设备和安装包选择 | bridge-agent、检测脚本 |
| 网络策略 | 是否允许访问百积木文档、缓存、relay 和 router | 企业 IT |

如果映射有歧义，先修正平台上下文再安装；不得使用默认账号、默认项目或其他客户上下文兜底。

## 3. 平台和设备前置条件

平台侧必须满足：

- 用户已加入目标工作区，目标项目状态可用。
- 企业模型、额度、计量和审计策略已生效。
- 个人安装页、设备安装执行页及其脚本均可访问。
- 当前模型键为执行文档规定的 `gpt-5.6-sol`，或平台显式传入受支持的 `CODEX_MODEL`。

设备侧必须满足：

- bridge-agent 在线并归属正确用户和工作区。
- 当前 Agent 已获授权调用 `shell.exec`、`shell.startExecution` 和 `shell.queryExecution`。
- 安装所需命令在 allowlist 内。
- `~/.config/baijimu/auth.json` 中存在当前工作区的设备本地 CLI token。

缺少任一 `shell` 方法、命令权限或本地授权文件时停止安装，向用户说明准确阻塞点，不降低系统安全策略。

## 4. 安装执行

Agent 读取公开执行文档后：

1. 获取 `workspaceId`、`projectId`、`userId`、`agentConfigId`、`agentSessionId`、`sessionId` 和已连接设备。
2. 通过检测脚本确认操作系统和 CPU 架构。
3. 使用 `shell.startExecution` 运行一体化脚本；不要用单次长阻塞 `shell.exec` 承载下载和安装。
4. 至少传入 `CODEX_WORKSPACE_ID` 和 `CODEX_PROJECT_ID`；有会话上下文时一并传入。
5. 使用 `shell.queryExecution` 轮询，并以脚本最终 stdout 的脱敏 JSON 为判断依据。
6. 脚本失败时报告真实阶段、错误和用户下一步，不自行添加临时兼容分支。

标准脚本：

```text
macOS:
https://www.baijimu.com/docs/scripts/codex-device-install/macos-configure-terminal-and-login.sh

Windows:
https://www.baijimu.com/docs/scripts/codex-device-install/windows-configure-terminal-and-login.ps1
```

Windows 多行 PowerShell 通过 `stdin` 传给 `powershell -File -`，不得把完整脚本拼进 `-Command`。

Linux 当前不能完成 ChatGPT desktop app 安装；目标设备为 Linux 时停止完整流程，不能只安装 CLI 后报告成功。

## 5. 凭证、路由和配置

安装脚本从设备本地读取当前工作区 token：

```text
~/.config/baijimu/auth.json
```

并写入 Codex 本地配置：

```text
~/.codex/auth.json
~/.codex/config.toml
```

关键配置：

```toml
model = "gpt-5.6-sol"
model_provider = "baijimu-router"
sandbox_mode = "danger-full-access"
approval_policy = "on-request"

[model_providers.baijimu-router]
base_url = "https://router.baijimu.com/api/claudecode/v1"
wire_api = "responses"
requires_openai_auth = true
```

根级字段必须位于第一个 TOML 表之前，并保留已有的 `[marketplaces.*]`、`[desktop]`、`[projects.*]` 等无关配置。`auth.json` 权限必须符合执行文档要求，完整 token 和 API Key 不得进入聊天、日志、截图、状态文件或最终 JSON。

## 6. 端到端验收

完成标准必须同时满足：

- ChatGPT desktop app 已安装、能启动、有可见窗口，并可进入 Codex。
- Windows 路由登录场景中，`account/read.account.type` 为 `apiKey`。
- `codex --version` 通过，并完成一次小型 smoke test。
- 使用设备本地凭证调用 router `POST /responses` 返回 HTTP 200。
- 服务端追踪确认请求归属到正确 `userId`、`workspaceId` 和 `projectId`。
- 没有落到平台管理员账号、默认测试账号或其他客户账号。

只完成 App、只写配置、只看到进程、只通过 router 请求，都不能报告安装成功。

## 7. 问题排查

| 问题 | 优先排查点 | 处理方式 |
| --- | --- | --- |
| 执行文档无法读取 | `/docs/features/codex-device-install/index.html`、网络和文档发布状态 | 恢复公开文档访问后重试，不回退到旧技能。 |
| 本机执行能力不可用 | 设备在线状态、`shell.exec/startExecution/queryExecution` 授权 | 让用户授权准确方法和命令范围。 |
| App 安装失败 | 受控缓存、SHA256、系统安装策略 | 使用脚本返回的阶段和错误定位，不使用未知镜像。 |
| App 启动后无窗口 | 主进程、app-server、窗口句柄、桌面截图 | `windows=[ NULL ]` 不算成功，按脚本收集日志。 |
| Windows 仍显示登录页 | app-server 登录事件和 `account/read` | 三个 API Key 登录条件全部通过后再重启 App。 |
| Codex 忽略 router | `config.toml` 根级字段位置 | 修复根级配置后重新验证。 |
| 终端或 smoke test 失败 | CLI 版本、本地 auth、router 响应 | 从脚本 JSON 的失败阶段继续排查。 |
| 请求归属错误 | 用户、工作区、项目和 token 映射 | 停止使用，修正身份映射后重新安装。 |
| 企业网络拦截 | 文档、OSS、relay、router 域名和 TLS | 由企业 IT 按网络放行清单处理。 |

排查时不使用平台管理员密钥，不增加默认账号、默认项目、缓存态或兼容分支掩盖根因。

## 8. 实施验收清单

- 企业组织、初始用户、工作区和 Codex 项目映射正确。
- bridge-agent 已安装、授权、在线并上报三个 `shell` 方法。
- 个人安装页和设备安装执行页可通过带或不带尾斜杠的地址访问。
- Agent 按公开执行文档和一体化脚本执行，未安装或加载 Codex 市场技能。
- ChatGPT desktop app、可见窗口、Codex 终端、路由、账号和 smoke test 全部通过。
- router、计量和审计日志归属一致。
- 安装过程和最终回复未泄露完整密钥或 token。
- 企业管理员和初始用户已收到对应安装与使用指南。
