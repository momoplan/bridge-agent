# 按平台发布 Bridge Agent

正式发布使用唯一 `release-bridge-agent` workflow 的手动入口。推送提交或标签不发布客户端。
默认平台来自 `.github/release-platforms.json` 的 `defaultPlatforms`，当前为 macOS Universal。
Windows、Linux 只在需要时选择；普通 PR 的跨平台质量检查继续运行，不调用 SSL.com。

## 发布

1. 实现和验证后将源码合入远程 main。遵守版本确认规则；MINOR/MAJOR 需要用户确认准确目标。
2. 从对应 main 提交创建不可变 `bridge-agent-v<SemVer>` 标签。版本文件必须一致。
3. 运行 `release-bridge-agent`，选择 main，填写标签与 `release_platforms`。留空发布 macOS；也可填写
   `windows`、`linux` 或逗号分隔组合，例如 `macos,windows`。本次选择是完整的平台集合。
4. 发布入口先校验平台和更新服务能力，再运行现有质量检查、选中平台的构建与签名、制品上传、更新验证。
5. 校验标签必须对应本次运行的精确源码，不能用更新的 main 内容覆盖旧版本。

每次发布只登记选中平台的完整格式集合。该集合在上传前由同一配置生成 manifest；漏文件、漏更新签名、
重复文件或混入未选择平台的文件都会失败。未发布平台保持各自已有的最新版本，不复制旧安装包冒充新版本。
macOS 的 Intel 和 Apple Silicon 使用同一 Universal 制品，两种架构均验证。

## 恢复

分发失败使用原标签和 `repair_assets_only=true`。`release_platforms` 留空时从原 GitHub Release 资产识别完整
平台集合，历史三平台版本继续按原集合恢复。填写平台时必须与已有资产一致，不能借修复增加或删减平台。
修复验证使用实际上传的完整平台集合；修复旧版本不会要求回退各平台已发布的新版本。
新平台需要一个新的不可变版本。恢复复用已有已签名制品，不重新调用签名服务。

## 部署顺序与验收

先发布 `bridge-agent-release-service` 的按平台查询修复及官网按平台下载查询，再启用此工作流。发布前要求 `/latest` 返回
`X-Bridge-Release-Selection: platform-v1`，旧服务不能接收新单平台发布流程。
更新服务必须先匹配平台/架构/格式，再选择其最新已发布版本；无平台参数仍表示全局最近一次发布。
动态下载入口和 Tauri 更新入口必须使用相同规则。发布不能改变全局强制升级策略。

验收包括默认 macOS、Windows 独立发布、平台组合、无效输入、历史三平台恢复、遗漏/多余制品，
以及新版 macOS 发布后 Windows/Linux 查询继续返回各自原版本。不得要求所有平台版本相同。

本次平台选择不等于提供未签名测试包：选中 Windows 正式发布仍必须签名。
