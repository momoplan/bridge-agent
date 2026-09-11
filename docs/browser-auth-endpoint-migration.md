# 浏览器授权地址迁移（0.8.3）

修复旧版本点击“浏览器授权”时请求已退役 `/lowcode3` 路径并收到 HTML 404 的问题。

客户端的 `platform.base_url` 表示环境 API 根地址。官方默认值与历史官方入口来自
`config/official-environment.json`，前端和 Rust 共同读取，业务逻辑不再维护独立官方域名分支。
加载、保存和前端编辑都会规范化旧配置，现有 `/lowcode3` 服务后缀会迁移掉，保留环境域名、
端口和部署路径前缀。设备 ID、工作区、环境身份、Relay 凭据和应用配置保持不变。

启动和轮询仅使用 `device-service/api/external-workspace-device-auth/{start,poll}`。
配置页显示实际连接的平台地址。错误提示包含请求 URL，不再引导用户填写已退役地址。

服务端仍拥有 verificationUri / verificationUriComplete。客户端不改写服务端返回的授权页面链接。
发布前必须验证环境的 `device-service` 配置 `external-auth-public-base-url` 指向当前设备服务公网入口，
并验证 `external-auth-manager-base-url` 指向目标环境管理前端。客户端迁移不能替代环境配置发布。

回归覆盖旧配置文件持久化迁移与幂等性、设备/凭据保留、私有环境端口/前缀保留、前端编辑保存，
以及通过本地 HTTP 服务对真实启动/轮询请求路径和参数进行验证。
