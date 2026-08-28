---
name: baijimu-platform
description: 通过 `baijimu` CLI 使用百积木企业 AI 操作系统的基础入口。用于登录认证、能力发现、工作区、项目文件与 Git、智能体会话、模型凭证、平台应用、本地 Connector 和公开 Partner API，并把 Bundle 开发或 Hosted Service 后端开发路由到对应场景技能。适用于能够读取 SKILL.md 并执行本机命令的智能体平台。
---

# 百积木平台

通过本机 `baijimu` CLI 操作百积木。把已安装 CLI 的分级帮助视为命令事实来源；本技能只保存跨版本稳定的工作流，不复制会随版本变化的命令参数、资源清单或接口定义。

## 能力发现

1. 运行 `baijimu --version`，确认本机版本。命令不在当前进程 `PATH` 时，先检查并直接使用官方固定安装位置：Windows 为 `$env:LOCALAPPDATA\Baijimu\bin\baijimu.exe`，macOS/Linux 为 `~/.local/bin/baijimu`。固定位置也不存在时才报告 CLI 未安装；Windows 上固定位置可用但短命令不可用时，提示用户完成当前任务后重启 Agent 或终端以读取更新后的用户 `PATH`。
2. 运行 `baijimu --help`，只定位当前任务需要的顶层命令组；再沿目标路径逐级运行 `baijimu <command> --help` 和更深层子命令的 `--help`。不要递归导出或一次加载完整命令树。
3. 查询精确参数时，以目标子命令的本机帮助为准。需要稳定业务流程时打开对应官方文档页面；只有尚不知道页面路径时才从 `llms.txt` 或 `docs-manifest.json` 定位，不读取无关页面。
4. 不用通用搜索结果、官网“最新版本”页面或历史命令快照覆盖本机 CLI 行为。帮助中缺少目标命令时，报告当前 CLI 版本不支持。
5. 需要账号或工作区动态资源时，运行 `baijimu auth status --verify`，再按目标资源运行对应的 `list`、`get`、`current` 或 `status`；已知工作区时按目标命令帮助增加工作区参数。
6. 不自行拼接或探测未由当前 CLI 能力输出、帮助或固定版本文档返回的域名。Bundle 市场操作通过当前 CLI 和 `https://api.baijimu.com` 的统一 Partner API 完成；`bundle-market.baijimu.com` 是已退役入口，其 DNS 不解析不是服务故障，也不能作为升级 CLI 的依据。只有本机命令面、固定版本文档或实际命令明确显示版本不兼容时，才报告需要升级。
7. 把工作区选择与平台健康分开判断。已有项目属于哪个工作区就使用哪个工作区；只有用户明确需要独立成员、权限、计费、数据隔离或产品归属，或者没有合适的目标工作区时，才建议新建。不得根据 DNS、网络探测或 CLI 健康状态推断需要新建工作区。

百积木官方文档站为 <https://docs.baijimu.com/>。已知业务场景时直接打开对应公开页面；路径未知时再从 <https://docs.baijimu.com/llms.txt> 或 <https://docs.baijimu.com/docs-manifest.json> 定位公开 Markdown、版本化 JSON Schema 和示例，不抓取 HTML 内嵌渲染数据。CLI 索引为 <https://docs.baijimu.com/cli/>，Partner API 为 <https://docs.baijimu.com/integration/api/>。`https://www.baijimu.com/docs/` 是兼容重定向入口。执行始终服从本机目标命令的分级 `--help`，不得改用其他版本或历史快照猜测参数。

面向普通用户和开发者的稳定产品契约只以官方文档站为准。场景技能只保存跨版本边界和执行顺序，必须继续按需读取当前 CLI 的分级帮助，不能用技能正文覆盖实时命令面。

如果 `baijimu` 不存在，告知用户先安装官方 CLI，不要静默下载。未登录时运行 `baijimu auth login`，由用户在浏览器中完成授权。

## 标准工作流

1. 沿目标命令路径逐级运行 `--help`，确认目标命令确实存在。
2. 先读取目标对象和当前状态。
3. 使用 CLI 的资源解析能力或命令自身的精确名称解析，把展示名转换为稳定 ID；零匹配或多匹配时停止并请求稳定 ID。
4. 明确目标、参数、权限和副作用后再写入。
5. 执行后用对应的 `get`、`list`、`status`、`messages`、`resources` 或审计命令回查；发布和服务调用还要做端到端验证。
6. 汇报业务结果、稳定 ID、验证证据和仍未解决的版本、认证或权限问题。

## 能力路由

- 认证与工作区：`auth`、`workspace`、`resource`。
- 项目文件与 Git：`project file` 仅用于 list/read/grep/download；修改必须通过 `project checkout` 检出 canonical 仓库。操作前先用本机帮助确认并运行 `project branch-policy get` 读取实际策略：`DIRECT` 允许有项目 Git 写权限的成员以快进方式直推 `main`；`PROTECTED` 必须推送 `codex/<userId>/<branch>` 个人分支，再用 `project merge` 合入 `main`，用户可以合并自己的分支。两种策略都禁止删除、强推或非快进覆盖 `main`。不要根据成员角色或历史默认值猜测策略；本机 CLI 没有 `branch-policy` 时，报告版本不匹配并使用该版本固定文档，不得猜测。权威设计见 <https://docs.baijimu.com/concepts/projects/>。
- 智能体与消息：`agent session`、`agent chat`、`llm-credential`。
- Bundle、Module、平台应用和 Runtime Bundle 生命周期：切换到 `$baijimu-bundle-development`。
- Hosted Service 后端项目、构建、数据库迁移、Environment、Deployment 和 Endpoint：切换到
  `$baijimu-hosted-service-development`。
- 平台应用：`platform-app`。
- 本地 Connector：`local-app`。设备、桌面、本地 shell 和 Connector 运行面仅在目标命令帮助确认存在，且用户已完成本地端、设备、工作区与服务授权时使用。
- CLI 未封装的公开能力：`baijimu api <METHOD> <PATH>`。调用前必须确认 Partner API 路径、参数、权限和返回结构。

## 执行规则

- 不编造 workspaceId、projectId、businessId、method、connectorId、moduleId、versionId、sessionId 或发布 ID。
- 不把展示名、模糊搜索结果第一项或缓存状态当作协议标识。
- 不熟悉命令时先运行相应的 `--help`；帮助中不存在的命令不得执行。
- 不把未登记域名的 DNS 结果描述成平台、Bundle 市场、认证或工作区故障；必须保留实际 CLI 命令、错误码和固定版本入口作为判断证据。
- Runtime 调用先列服务，再读取方法定义，最后调用；所有业务参数放入 `--params` 对象，无参数也显式传 `{}`。
- 复杂 JSON 优先写入临时文件并使用 `@file`，完成后清理不含用户资产的临时文件。
- 不直接编辑 CLI 认证文件、Bridge Agent 配置、Connector 安装目录或 management token。
- 项目文档和本技能不授予工作区、发布或审核权限。发布者与审核者边界由平台状态源执行，不得用同一身份自行批准、伪造审核状态或直接修改服务器绕过流程。
- 本地能力排查顺序固定为：本地端运行与授权、Relay 连接、Connector 安装与启用、健康检查、服务和方法上报、调用方权限、审计与日志印证。能力不存在时报告版本或授权缺失，不用手工配置绕过。
- 不输出 PAT、模型密钥、服务令牌、cookie 或完整认证响应。除非用户明确要求，不使用任何显示 secret 的选项。

## 风险与确认

查询、列表、帮助、状态和审计属于只读操作，可直接执行。

创建、更新、安装、升级、发布、提交审核和调用可能产生业务副作用的方法，必须确保目标与参数明确；执行后回查。

删除、卸载、回滚、重置、撤回发布、释放数据库、覆盖远端内容，以及可能产生费用或向外部发送消息的操作，必须获得用户对准确目标的明确授权。不要用临时兼容分支、缓存态或手工改服务器文件绕过失败。

## 完成标准

只有在目标操作成功且状态源回查一致时才报告完成。若失败，保留原始错误码和可公开的错误信息，区分 CLI 版本缺失、认证失败、权限不足、资源解析失败、平台业务错误和本地 Connector 不健康。
