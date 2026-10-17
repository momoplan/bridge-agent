# Connector 截图归属与上传完成合同（2026-10-17）

当前用户明确确认：Connector 图片上传从 Relay → project-service 改为 Relay → workspace-file-service，沿用已存在的设备授权和个人文件服务关系。文件归属为设备有效授权导出的 (workspaceId, userId) 个人空间，不能由调用者指定用户，也不能默认写入工作区共享空间。文件字节保持客户端直传 OSS。

用户分别确认平台 Component workspace-file-service 8.0.6 → 8.1.0、独立客户端 Bridge Agent 0.12.1 → 0.13.0，以及私有本地应用 wechat-sender 3.1.4 → 3.1.5。Relay 最初拟议 0.20.0 已被占用；核对目录后用户再次明确确认 Relay Component 0.22.0，并确认继承 0.21.0 主线相对生产 0.19.1 的准确依赖差异：新增 bundle-service、runtime-config-service、backend-instance、gateway2 服务依赖，新增 backend-property-configuration-contract 1.2.0、backend-instance-contract 12.0.0，event-center 17.0.0 → 18.0.0，bundle-runtime-contract 29.0.0 → 34.0.0，删除 bundle-owner-catalog-contract（bundle-runtime-contract 33.2.0 的别名）。本次不改变这些已登记依赖，也不改变其锁定的来源或传递解析。

## 合同与职责

- 新 Connector 云上传使用 `/api/bridge-agent/local-app-assets/prepare` 和 `/complete`，请求显式 `contractVersion: 1.0.0`，返回标准 CModel。Bridge 仍向 Connector 提供 `/v1/local-app-assets` 及既有 `asset_ref` 成功表示。
- Relay 校验现有设备 token 与工作区绑定，传递既有 DeviceAuthorization 给文件 owner；个人身份从当前设备授权解析，绝不生成、缓存或转发用户 JWT/PAT。
- 文件 owner 新增 `/inner/device-assets/direct-uploads/prepare` 和 `/complete`，复用其版本化 DeviceAuthorization 与上传票据/完成合同。只允许 PNG/JPEG/WebP，source 为稳定协议类型 `connector_asset`；路径和 UUID 由 owner 生成，应用与用途是 Relay 审计标签，不是授权凭据。
- Bridge 执行 prepare → OSS PUT → complete，核对完成文件 ID、大小和 MIME；完成前不得返回成功。owner 检查来源、个人空间、配额、上传对象、版本及撤权，重复完成保持同一文件身份。
- 现有 `computer.screenshot` 合同保持；Connector 的旧上传请求明确拒绝并提示升级，不能回退 project-service 或在未完成登记时返回临时文件。
- 服务端 CModel 业务失败保留错误码、公开 message 和 retryable。Bridge 本机接口用 422 表示上游业务拒绝并原样携带身份；真实连接/超时用 502/504，协议错误与存储拒绝各自明确。Connector 解析结构化错误并透传 CModel，不能只打印 urllib 的 HTTP 状态。没有上游 retryable 时保持未知。
- 不在错误、日志中输出 token、原始代理页面或 OSS 签名 URL。文件服务内部诊断留在服务端，公开接口只发安全业务说明。

## 发布、迁移与恢复

无新增数据库字段、凭据、存储资源或文件迁移。已有文件保留。先发布文件 owner 与 Relay 组件，再发布客户端与私有 Connector；版本及依赖必须使用本段精确批准目标。旧客户端的 Connector 上传仍需升级，新客户端连接旧 Relay 应明确失败，无兼容回退。回滚不能把已经登记的文件切回 Project Service；暂停推广并协调回退客户端/服务端合同，不删除个人文件。Relay 同时继承的事件/属性协议需按已登记消费者合同验证，不能仅凭截图成功宣称整次部署安全。

验收覆盖申请/PUT/完成、重复完成、缺少 PUT、文件身份不符、截图与 Connector 来源混用、跨工作区/个人空间、撤权、格式不合法、错误原码和可重试性、代理 HTML/敏感信息不外泄。生产验收使用工作区 1390 的已授权 majunyao 设备，确认版本、查询截图可读与后续匹配分析；不测试发送消息。

批准与实现不是发布证据；实际流水线、制品、Operation 和真机结果需另行记录。
