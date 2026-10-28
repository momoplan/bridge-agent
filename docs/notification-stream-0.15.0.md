# 本地应用实时输出流（Bridge Agent 0.15.0）

2026-10-28 当前用户确认：Connector 对外协议新增 `notificationStream` 与能力 `connector.notification-stream.v1`；Bridge 的 relay 依赖从 `ca7cfc8` 升级到包含流合同的 relay 提交（relay 先把仅服务端使用的 8 个合同 crate 改为 `server` feature 下的可选依赖，Bridge 锁文件只有 relay 一项变化）；版本 Bridge Agent 0.14.2 → 0.15.0（MINOR）。平台记录见 `lowcode/docs/decisions/2026-10-28-local-app-notification-stream.md`。

- Agent 协议新增 feature `local_app_streams_v1`，处理 `stream_attach` / `stream_detach`，转发 `stream_output` / `stream_resync` / `stream_closed`。
- 只有 Relay 挂接时才拉取 Connector 的 SSE，数据不落 SQLite，不重试；Relay 连接结束时全部停止。
- 未声明 `notificationStream` 的 Connector 回复 `STREAM_UNSUPPORTED`，其事件上传路径不变。
- 合同细节见 `BRIDGE_LOCAL_CONNECTOR_SPEC.md` 的“实时输出流”。
