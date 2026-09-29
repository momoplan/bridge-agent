# Bridge Agent 0.9.3 与 CLI 0.92.0

本次客户端版本为 0.9.3，发布 macOS Universal、Windows x64 和 Linux x64。
内置 Baijimu CLI 从 0.90.0 更新到 0.92.0，使用 CLI 发布清单中的精确版本、来源身份和三平台 SHA-256。
客户端仅消费已签名 CLI 制品，不重新构建 CLI。

首次安装使用内置版本；启动时如果内置版本高于当前托管 CLI，则升级至内置版本。
已安装的更高版本保留。0.92.0 提供本地应用精确版本的私有分发授权管理命令，
保留现有安装协议和宿主控制协议 3.0.0。现有应用安装与凭据继续保留。

## 发布顺序与强制更新

1. CLI 0.92.0 完成独立中心市场审核，并由其正式发布工作流完成公开来源及制品验证。
2. 从市场读取该版本 listing 身份，更新 Bridge 仓库的 BAIJIMU_CLI_MARKET_SOURCE 发行配置。
3. 从远程 main 的不可变 bridge-agent-v0.9.3 标签，通过 release-bridge-agent 工作流发布三平台。
4. 验证各平台 GitHub Release、签名、公开下载、更新元数据以及包内 CLI 版本。
5. 平台管理员通过正式客户端发布管理入口将 minimumSupportedVersion 设为 0.9.3，保持 forceUpdate=false。
6. 各平台分别验证旧版本请求需要强更，0.9.3 及更高版本请求不需要强更。

最低支持版本目前是跨平台全局配置，因此不得在某个平台缺少目标制品时提高门槛。
客户端更新策略不由外部发布 Token 或 GitHub Actions 修改。失败时保持原最低支持版本；
已发布不可变制品不得覆盖，分发修复仅使用声明的 repair_assets_only 流程。
