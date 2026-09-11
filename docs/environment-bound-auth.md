# 环境绑定授权

授权服务返回 `environmentKey`，取自环境投影并经初始化验证的认证配置。客户端以此保存环境身份，API 地址只负责连接；不得按 CLI 当前工作区推断 Bridge 的授权。

共享授权环境记录保存 `environmentKey` 和 `baseUrl`；凭据保存 `environmentKey`、工作区和设备/用户信息。CLI 保留自身的 currentEnvironment/currentWorkspaceId，Bridge 使用自己的平台配置。没有环境字段的旧凭据仅归属官方环境；私有环境必须完成明确绑定。官方目录由版本化 `config/official-environment.json` 定义。

CLI 首次连接使用 `baijimu auth login --base-url https://environment.example.test`，需要指定工作区时加 `--workspace-id`。授权成功才保存地址、凭据和当前环境。`baijimu auth environment` 列出已登记环境；`baijimu auth environment --use-key <environmentKey>` 切换 CLI，保持 Bridge 的连接。CLI 退出登录保留其他环境及 Bridge 设备凭据。

Bridge 在设置中编辑目标 API 地址后发起浏览器授权，在目标环境选择工作区。授权取消或失败不覆盖原连接；成功后保存新的环境/工作区/设备绑定。修改地址时不会把旧工作区 ID 带入新环境。环境身份与既有同地址绑定冲突时拒绝保存。

发布顺序：access-credential-service 0.9.0、device-service 0.6.0 → CLI 0.55.0 → 内置该 CLI 的 Bridge 0.8.0。已有官方授权按缺省规则继续读取；旧 CLI 不具备环境字段保存和隔离能力，需要随客户端升级。

## 市场读取凭证

Bridge 按自身绑定的环境和工作区选择共享授权文件中的 PAT，来源 `source` 和设备 `clientId` 不作为市场读取的权限条件。CLI 登录、手工登记和设备授权产生的 PAT 均可参与选择；服务端验证令牌有效性、用户及工作区读取权限。Relay 的设备授权独立处理。

存在多条匹配凭证时，沿用 CLI 的排序：依次比较 `issuedAtEpochSeconds`、`issuedAt`、`credentialId`，优先较新记录。不会受 CLI 当前环境或工作区切换影响，也不会在选中凭证失败后尝试其他身份。官方环境仍支持缺少环境字段的历史凭证；私有环境必须具备明确的环境绑定。
