# Linux CLI 升级验收输入

`market-listing.json` 的 `frozenVersion` 来自 `baijimu workspace local-app version get` 的作者环境回读，市场条目 ID 来自 `baijimu` 0.180.0 的正式市场发布回执。外层按市场 3.0.0 构造测试输入。`market-listing-v4.json` 使用同一冻结版本，外层改为 4.0.0 并移除 `publisher`、`capability` 和 `risk`。

Linux 验收下载已发布的 0.121.0 和 0.180.0 制品，验证安装失败重试、升级、重启、回滚以及项目和 Bundle v4 作者入口调用。各平台的合同测试核对两种外层格式中的冻结内容完全一致。登记流水线的 Linux runner 使用实际发行二进制执行安装验收。
