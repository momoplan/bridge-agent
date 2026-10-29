# Relay 连接改为 gRPC 双向流（Bridge Agent 0.16.0）

2026-10-29 当前用户确认把设备与 Relay 之间的原生 WebSocket 换成 gRPC 双向流，并确认依赖清单和版本 Bridge Agent 0.15.0 → 0.16.0（MINOR）。平台记录见 `lowcode/docs/decisions/2026-10-29-device-grpc-channel.md`，协议权威说明见 relay 仓库 `docs/adr/2026-10-29-device-grpc-channel.md`。

- **连接**：每次连接打开一条 `baijimu.relay.device.v1.DeviceChannel/Connect` 流。socket、HTTP/2 分帧、关闭和半开连接由 tonic 与 h2 处理，客户端不再手写 Ping/Pong 和 Close。
- **地址**：gRPC 地址取已配置 relay 地址的协议、域名和端口（`wss://relay.baijimu.com/ws/agent` → `https://relay.baijimu.com/`），已有配置和授权下发的 `relayWsUrl` 不用改，设备不用重新授权。`https` 使用 rustls 与内置 webpki 根证书。
- **凭证**：设备 token 不再出现在 URL 里，改放在请求元数据 `authorization: Bearer <token>`，设备 ID 放在 `x-baijimu-device-id`。
- **授权失效**：`UNAUTHENTICATED`、`PERMISSION_DENIED` 进入“需要重新授权”并暂停重连；`UNAVAILABLE` 等其他错误按普通退避重连，Relay token 存储故障不会让设备丢弃凭证。
- **存活**：HTTP/2 PING 只在相邻两跳之间有效，所以双方各自每 25 秒发一个心跳帧，75 秒收不到 Relay 的任何帧就断开重连，和原来的超时一致。
- **消息**：帧按方向区分类型，与原有 `AgentMessage` 一一对应，协议版本 4.0.0 和 feature 协商不变。方法参数、结果、事件载荷、schema 和流数据仍是原样的 JSON 字节。单帧上限 64 MiB，与原 WebSocket 一致（需要 relay ≥ 0.35.1）。Relay 发来本版本不认识的帧类型时只记日志并忽略，与原来忽略未知消息类型的行为相同。
- **不回退**：只走 gRPC，连不上就按上面的规则报错和重连，不自动退回 WebSocket。未批准兼容回退，因此要等公网入口支持 gRPC 后才能发布。
- **依赖**：直接依赖 tonic 0.14.8（`channel`、`codegen`、`tls-ring`、`tls-webpki-roots`）；relay 依赖升到 `8831f35`（0.35.1）并打开 `device-channel` 特性，生成代码和合同类型转换来自 relay。传递依赖 webpki-roots 1.0.6 → 1.0.11；测试构建为 tonic 打开 `server` 特性以运行真实 gRPC 假 Relay。`tokio-tungstenite` 已不再被代码使用，按已确认的计划在第三阶段（Relay 删除 `/ws/agent`）一并移除。
