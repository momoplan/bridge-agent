# Linux CLI 升级验收输入

market-listing.json 是原始市场 3.0.0 响应样本，保留供来源证据核对。market-listing-v4.json 是本次合同升级后响应的测试投影，只将市场读取合同改为 4.0.0 并移除 publisher、capability、risk。它不声称现网已升级。冻结版本、manifest 原文、来源版本、制品及下载校验和均保持原始值。

当前客户端验收读取 v4 样本，并继续下载准确的已发布 Linux 制品，实际验证安装失败重试、升级、重启和回滚。root 合同测试在各平台验证投影与原始冻结内容逐字相同；真实 Linux 制品验收只在登记流水线的 Linux runner 执行。
