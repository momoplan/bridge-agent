---
name: baijimu-docs
description: 查询并使用百积木官方文档和本机 CLI 能力。用户提到百积木、baijimu、Bundle、模块、运行时、平台应用、Connector 或 Partner API 时使用。
---

# 百积木官方文档

处理百积木相关需求时，不要要求用户另行提供百积木规范、Bundle 格式或示例。先通过本机 CLI 和百积木官方网站自行发现当前版本对应的能力与文档。

1. 运行 `baijimu --version`，确认本机 CLI 版本。
2. 运行 `baijimu capabilities --help`。如果支持 `--offline`，运行 `baijimu capabilities --offline --json`。
3. 优先读取能力输出中以下版本固定文档入口：
   - `documentation.version`
   - `documentation.commandSchema`
   - `documentation.offlineCapabilities`
4. 文档入口不足时，只查询百积木官方网站：
   - CLI 文档：<https://www.baijimu.com/docs/cli/>
   - Bundle 开发文档：<https://www.baijimu.com/docs/development/bundle-development/>
   - Partner API：<https://www.baijimu.com/docs/integration/api/>
5. 涉及具体命令时，先运行 `baijimu <command> --help`，以本机已安装版本的帮助为准，不编造命令、参数、资源 ID 或 Bundle 格式。
6. 需要账号或工作区动态资源时，先运行 `baijimu auth status --verify`；未登录时运行 `baijimu auth login` 并让用户在浏览器完成授权。
7. 如果 CLI 与网站文档版本不一致，报告版本不匹配，并以本机 CLI 能力输出给出的固定版本文档为准。
