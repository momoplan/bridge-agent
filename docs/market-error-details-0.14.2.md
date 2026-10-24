# 认证错误与请求追踪（11.0.0）

2026-10-24 当前用户明确确认完整方案、依赖变更和准确版本：contracts 10.3.0 → 11.0.0，local-app-service 10.0.4 → 11.0.0，Bridge Agent 0.14.1 → 0.14.2（macOS）。停机迁移策略不变。

认证 SDK 的 client feature 增加同版本公共 baijimu-cmodel-http 依赖，复用唯一 RequestContext，默认捕获当前请求上下文，并允许显式传入。它只能设置请求与追踪 ID，不开放认证头或目标修改能力。发送前继续检查 audience、有效期、workspace 约束和重定向限制。

ClientError::Transport 从无数据变体变为持有 reqwest::Error 的变体，Error::source 保留完整底层原因。错误携带的 URL 在边界移除，避免查询参数泄漏；Display 保持安全摘要。消费者需迁移枚举模式匹配，此 Rust API 不兼容变化要求 MAJOR；CModel JSON 协议未变。

已批准消费者改动：local-app-service 的 auth/core/http/axum 从 Git 5.0.0（213e7ef）改为 crates.io 精确 11.0.0，增加内部 registry catalog 11.0.0、tracing 0.1.44、tracing-subscriber 0.3.23 JSON；Bridge 的 core/http 同样改为 crates.io 精确 11.0.0。只解析必要的 lock 依赖变化。其他服务关系、认证协议和停机迁移策略不变。SDK 先由登记 Jenkins 发布，再由消费者锁定真实 registry 制品验证。

不在服务端复制 HTTP 认证客户端或 CModel 解析逻辑，不增加旧协议兼容回退。验证覆盖传输错误 source、查询 URL 不泄漏、并发上下文隔离、显式上下文、原有认证与工作区约束。发布失败遵循不可变制品恢复流程；服务回退使用此前已验证制品，不能覆盖版本。

客户端通过标准 CModel SDK 区分 HTTP 200 业务失败、非 200 失败与非法响应；版本、目录与制品下载共用此解析边界。ConsumerFailure 保留公开错误码、消息、重试建议、请求/追踪 ID 与完整公开详情；CLI 升级命令通过结构化 Tauri CommandError 保留这些字段；其余现有字符串命令边界通过 Display 保留这些字段，错误提示可展开和复制。底层 reqwest 原因只进入本地日志并移除 URL。

安装包接口返回成功 JSON 或响应头与冻结清单不符时，同样携带请求/追踪 ID 和 HTTP 状态，并记录具体违反项；保留既有拒绝安装行为。
