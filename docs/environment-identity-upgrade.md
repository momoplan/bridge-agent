# 来源环境身份升级（Bridge 0.9.1）

应用由 `environmentKey + appId` 标识，精确安装增加 `version`。分发 listing 只用于定位经过审核的对象，
不构成另一套市场身份。公开安装由宿主从消费者环境的授权市场接口解析 listing，完整校验来源和精确版本。
私有安装必须显式提供环境 key，并使用该环境已授权的凭据读取冻结版本；不允许无来源的匿名登记接口。

CLI 0.90.0 与宿主控制协议 3.0.0 配套使用：

```sh
baijimu local-app install <app-id> --environment-key <source-environment-key> --version <semver>
```

升级前停止 Bridge 和本地应用，使用发行包内已签名的所有者迁移程序：

```sh
bridge-agent-environment-identity-migration --config-dir <bridge-config-dir> --local-apps-dir <local-apps-root> --managed-apps-dir <managed-apps-root> --host-already-stopped
```

迁移程序随 Bridge 版本构建，无独立版本。它预检全部安装记录、managed tool 当前/回滚来源和制品旁的
`.market.json`，只删除旧分发来源中的 `marketKey`，保留已有 `source.application.environmentKey`，
原始记录保存为 `.json.before-environment-identity-3`，逐文件原子替换，允许失败后重复执行。
程序不终止进程、不连接网络、不猜测来源、不改写清单或用户应用数据。运行中的宿主和应用会阻止迁移。
旧的 source-less 安装保持未认领；从明确选择的来源重新安装时必须加 `--replace --migrate-source-identity`。
已知来源不能通过该开关换到其他环境。回滚宿主前应停止写入并恢复对应原始记录，不用新记录启动旧宿主。

公开读取合同改为 3.0.0，已冻结内容 1.0.0 和市场提交协议 2.0.0 保持不变。
服务端 local-app-service 3.0.0、local-app-market 5.0.0 应与客户端配套切换，旧客户端不兼容新目录协议。
