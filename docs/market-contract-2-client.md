# 0.8.4 市场协议适配

客户端 0.8.3 使用 local-app-contract 1.1.0，无法解析中心新版条目中的
contractVersion 和 presentation。0.8.4 将核心与桌面依赖一起锁定到已发布的
local-app-contract 2.0.0、不可变提交 53d8520c09b567e8b611beb2c125d6fee41086a1。

市场卡片的名称、描述、能力、风险和图标来自审核后的 presentation。冻结 manifest
继续用于安装、制品选择和宿主兼容性，不能代替独立展示信息。没有可选能力或风险说明时不显示空行。

目录顺序由中心市场决定；UUID 只作为条目身份和分页游标，不比较 UUID 大小推断目录顺序。
客户端仍拒绝重复条目、重复来源应用、当前游标再次出现在页面中，以及不指向最后条目的后继游标。
跨页重复同样终止读取，避免循环请求或合并错误目录。

普通质量检查覆盖协议解析、展示投影、来源隔离和分页。真实工作区的只读验收可以显式指定已有
设备配置，复用客户端自己的凭据选择、目录和准确版本读取代码：

```sh
BRIDGE_AGENT_LIVE_CONFIG=/absolute/path/to/agent-config.json \
  cargo test --manifest-path src-tauri/Cargo.toml \
  current_device_market_reads_and_renders_reviewed_listings -- --ignored --nocapture
```

该验收不安装应用、不更改配置或凭据；常规 CI 默认不运行需要真实账号的测试。
客户端发布仅使用 release-bridge-agent 工作流，0.8.4 的发布平台为 macos,windows。
市场内应用及其冻结版本不因客户端协议适配而重新发布。
