# 按 Bundle Locator 查询 Connector 市场更新

当前用户批准 Bridge Agent 从 0.22.0 升至 0.23.0（MINOR），并单独批准核心与桌面壳的同一 Gitee 依赖 `local-app-contract` 从 `6.0.1@fa6b44e7ce3cda780ded57e9178a7c045539f6dc` 更新至 `6.2.0@d0bce0722285a30280a953d0405cd9cc4acfaa78`，传递引入 crates.io `baijimu-bundle-plugin =27.0.0`。先验证共享合同正式发布。

已具备市场安装来源的 Connector，通过已记录的市场条目取得正式 Bundle Locator，再按该 Locator、平台和架构查询更新。市场上仍使用旧合同的应用继续按已有来源规则查询。来源未知的安装保持原来的来源验证要求。查询无需写入设备的安装记录。
