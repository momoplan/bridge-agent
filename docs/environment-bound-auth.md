# 环境绑定授权

授权服务返回 `environmentKey`，取自环境投影并经初始化验证的认证配置。客户端以此保存环境身份，API 地址只负责连接；不得按 CLI 当前工作区推断 Bridge 的授权。

共享授权环境记录保存 `environmentKey` 和 `baseUrl`；凭据保存 `environmentKey`、工作区和设备/用户信息。CLI 保留自身的 currentEnvironment/currentWorkspaceId，Bridge 使用自己的平台配置。没有环境字段的旧凭据仅归属官方环境；私有环境必须完成明确绑定。官方目录由版本化 `config/official-environment.json` 定义。

CLI 首次连接使用 `baijimu auth login --base-url https://environment.example.test`，需要指定工作区时加 `--workspace-id`。授权成功才保存地址、凭据和当前环境。`baijimu auth environment` 列出已登记环境；`baijimu auth environment --use-key <environmentKey>` 切换 CLI，保持 Bridge 的连接。CLI 退出登录保留其他环境及 Bridge 设备凭据。

Bridge 在设置中编辑目标 API 地址后发起浏览器授权，在目标环境选择工作区。授权取消或失败不覆盖原连接；成功后保存新的环境/工作区/设备绑定。修改地址时不会把旧工作区 ID 带入新环境。环境身份与既有同地址绑定冲突时拒绝保存。

发布顺序：access-credential-service 0.9.0、device-service 0.6.0 → CLI 0.55.0 → 内置该 CLI 的 Bridge 0.8.0。已有官方授权按缺省规则继续读取；旧 CLI 不具备环境字段保存和隔离能力，需要随客户端升级。
