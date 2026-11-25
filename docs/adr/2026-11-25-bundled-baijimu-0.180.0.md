# Bridge Agent 内置 `baijimu` 0.180.0

2026-11-25，当前用户对逐项列出的依赖变更和发布范围回复“确认”。Bridge Agent `0.26.0`（MINOR）发布 macOS Universal、Windows x64 和 Linux x64，内置 `baijimu` 由 `0.121.0` 更新到 `0.180.0`。内置工具仍来自 `baijimu:baijimu-cli` 的同一市场条目，三平台制品及 SHA-256 取自该版本的正式发行记录。

Bridge Agent 维护安装与启动集成，`baijimu-cli-rs` 维护独立制品和市场来源。此次内置更新让新安装具备 BundleDefinition `24.0.0` 的作者入口。现有设备也可从客户端应用市场独立更新 `baijimu`。沿用现有托管工具更新和恢复机制。

发布前核对三平台原始制品、市场冻结清单及版本锁定。正式发布通过 `release-bridge-agent` 验收安装包签名、内置二进制及各平台更新。作者定义以生产 `baijimu bundle definition read` 验证。
