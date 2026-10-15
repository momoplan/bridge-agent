# 设备调用公开错误详情

2026-10-15 用户确认独立修复设备错误链路，准确批准 Bridge Agent 0.11.3 → 0.12.0（MINOR，macOS）与 Relay 0.15.1 → 0.16.0（MINOR）。依赖、来源及锁文件解析保持不变；不发布 Bundle 或平台应用。

设备回执 InvokeError 新增可选 public_details（标准 CModel message/retryable JSON）。只有本地 HTTP 返回通过现有 CModel 解码器校验的公开失败详情才填充该字段。内部 message 继续保存诊断，不从内部异常、命令输出或旧 success/error 包装提升出公开提示。合法但没有公开详情的旧设备错误仍不带该字段；这不是新兼容分支或文案推断。

Relay 独立校验该字段，只在 CModel HTTP 响应中输出标准公开详情；字段畸形则明确协议错误。单节点及跨节点的原有 InvokeResult 序列化均保留该字段。先升级全部 Relay，再在授权验收设备安装客户端。旧 Relay 不保证保留新增字段，故不能倒置部署顺序。不得将 error.message 无条件作为用户提示。

回归覆盖标准中文详情、畸形详情拒绝、无公开详情不泄露内部文本、设备 wire 往返及 Relay 跨节点回执往返。客户端唯一 GitHub Actions 工作流完成质量门禁和签名分发；签名发布成功与设备升级、真实调用验收分别记录，不以更新元数据替代设备版本事实。
