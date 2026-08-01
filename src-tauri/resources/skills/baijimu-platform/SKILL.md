---
name: baijimu-platform
description: 通过 `baijimu` CLI 使用百积木企业 AI 操作系统。用于登录认证，管理工作区、项目文件和 Git、智能体会话、模型凭证、Bundle-first 模块开发与统一发布、运行时服务与应用、托管服务、数据库配置、平台应用、本地 Connector，或通过公开 Partner API 补充 CLI 尚未封装的能力。适用于 Codex、WorkBuddy、钉钉悟空等能够读取 SKILL.md 并执行本机命令的智能体平台。
---

# 百积木平台

通过本机 `baijimu` CLI 操作百积木。把已安装 CLI 的能力输出和帮助视为执行事实来源；本技能只保存跨版本稳定的工作流，不复制会随版本变化的命令参数、资源清单或接口定义。

## 能力发现

1. 运行 `baijimu --version`，确认本机版本。命令不在当前进程 `PATH` 时，先检查并直接使用官方固定安装位置：Windows 为 `$env:LOCALAPPDATA\Baijimu\bin\baijimu.exe`，macOS/Linux 为 `~/.local/bin/baijimu`。固定位置也不存在时才报告 CLI 未安装；Windows 上固定位置可用但短命令不可用时，提示用户完成当前任务后重启 Agent 或终端以读取更新后的用户 `PATH`。
2. 使用已确认的 CLI 入口运行 `capabilities --help`。若帮助包含 `--offline`，运行 `capabilities --offline --json`，取得本机完整命令树和版本固定的官方文档入口；旧版 CLI 则直接依赖各级 `--help`。
3. 查询精确参数时，优先使用本机 `baijimu <command> --help`；需要详细流程或机器结构时，只访问能力输出中 `documentation.version`、`documentation.commandSchema` 或 `documentation.offlineCapabilities` 指向的固定版本 URL。
4. 不用通用搜索结果或官网“最新版本”页面覆盖本机 CLI 行为。固定入口缺失时，报告 CLI/文档版本不匹配。
5. 需要账号或工作区动态资源时，运行 `baijimu auth status --verify`，再运行 `baijimu capabilities --json`；已知工作区时按本机帮助增加工作区参数。

百积木官方文档站为 <https://docs.baijimu.com/>：CLI 索引为 <https://docs.baijimu.com/cli/>，Bundle 开发规范为 <https://docs.baijimu.com/development/bundle-development/>，Partner API 为 <https://docs.baijimu.com/integration/api/>。`https://www.baijimu.com/docs/` 是兼容重定向入口；不要据此手工拼接版本 URL。索引只用于发现，执行仍服从本机 CLI 返回的固定入口；固定版本页面或 JSON 不可访问时，明确报告该 CLI 版本的文档尚未发布，不得改用其他版本猜测参数。

如果 `baijimu` 不存在，告知用户先安装官方 CLI，不要静默下载。未登录时运行 `baijimu auth login`，由用户在浏览器中完成授权。

## 标准工作流

1. 用本机能力输出和 `--help` 确认目标命令确实存在。
2. 先读取目标对象和当前状态。
3. 使用 CLI 的资源解析能力或命令自身的精确名称解析，把展示名转换为稳定 ID；零匹配或多匹配时停止并请求稳定 ID。
4. 明确目标、参数、权限和副作用后再写入。
5. 执行后用对应的 `get`、`list`、`status`、`messages`、`resources` 或审计命令回查；发布和服务调用还要做端到端验证。
6. 汇报业务结果、稳定 ID、验证证据和仍未解决的版本、认证或权限问题。

## 能力路由

- 认证与工作区：`auth`、`workspace`、`resource`。
- 可发现能力、Bundle-first 开发与安装：`capabilities`、`bundle`。
- 项目文件与 Git：`project file`、`project git`。
- 智能体与消息：`agent session`、`agent chat`、`llm-credential`。
- 模块源码项目与方法：`module project`、`module method`。
- Bundle 内模块定义、模块版本和统一发布：以本机帮助中的 `bundle module`、`bundle version`、`bundle review`、`bundle market` 为准。
- 托管服务和构建：`hosted-service`、`rust-build`、`db-profile`。
- 平台应用：`platform-app`。
- 本地 Connector：`local-app`。设备、桌面、本地 shell 和 Connector 运行面仅在本机能力输出或帮助确认存在，且用户已完成本地端、设备、工作区与服务授权时使用。
- CLI 未封装的公开能力：`baijimu api <METHOD> <PATH>`。调用前必须确认 Partner API 路径、参数、权限和返回结构。

## 执行规则

- 不编造 workspaceId、projectId、businessId、method、connectorId、moduleId、versionId、sessionId 或发布 ID。
- 不把展示名、模糊搜索结果第一项或缓存状态当作协议标识。
- 不熟悉命令时先运行相应的 `--help`；帮助中不存在的命令不得执行。
- Runtime 调用先列服务，再读取方法定义，最后调用；所有业务参数放入 `--params` 对象，无参数也显式传 `{}`。
- 复杂 JSON 优先写入临时文件并使用 `@file`，完成后清理不含用户资产的临时文件。
- 不直接编辑 CLI 认证文件、Bridge Agent 配置、Connector 安装目录或 management token。
- 模块源码项目可以独立存在；模块定义必须在 Bundle 内创建。模块冻结只生成不可变内部资源，不得把它描述成独立发布、审核、市场上架或安装。
- Bundle Manifest 必须引用精确模块版本；对外审核、市场发布、安装、升级和卸载均以 Bundle 为对象。若本机 CLI 尚未提供 `bundle module`，报告版本过旧并使用其固定版本文档，不要猜测新命令参数或退回独立模块发布。
- 本地能力排查顺序固定为：本地端运行与授权、Relay 连接、Connector 安装与启用、健康检查、服务和方法上报、调用方权限、审计与日志印证。能力不存在时报告版本或授权缺失，不用手工配置绕过。
- 不输出 PAT、模型密钥、服务令牌、cookie 或完整认证响应。除非用户明确要求，不使用任何显示 secret 的选项。

## 风险与确认

查询、列表、帮助、状态和审计属于只读操作，可直接执行。

创建、更新、安装、升级、发布、提交审核和调用可能产生业务副作用的方法，必须确保目标与参数明确；执行后回查。

删除、卸载、回滚、重置、撤回发布、释放数据库、覆盖远端内容，以及可能产生费用或向外部发送消息的操作，必须获得用户对准确目标的明确授权。不要用临时兼容分支、缓存态或手工改服务器文件绕过失败。

## 完成标准

只有在目标操作成功且状态源回查一致时才报告完成。若失败，保留原始错误码和可公开的错误信息，区分 CLI 版本缺失、认证失败、权限不足、资源解析失败、平台业务错误和本地 Connector 不健康。
