# 来源环境身份升级（Bridge 0.9.2）

应用由 `environmentKey + appId` 标识，精确安装增加 `version`。分发 listing 只用于定位经过审核的对象，
不构成另一套市场身份。公开安装由宿主从消费者环境的授权市场接口解析 listing，完整校验来源和精确版本。
私有安装必须显式提供环境 key，并使用该环境已授权的凭据读取冻结版本；不允许无来源的匿名登记接口。

CLI 0.90.0 与宿主控制协议 3.0.0 配套使用：

```sh
baijimu local-app install <app-id> --environment-key <source-environment-key> --version <semver>
```

0.9.2 修复 0.8.9 直接升级的自动迁移缺口。客户端先完成更新检查和旧应用 ID 迁移，
再调用发行包内已签名的来源身份迁移程序；成功后才开放配置读取、应用控制服务、CLI 引导和业务启动。
新安装无需用户执行命令。没有待转换记录时迁移幂等返回，不停止已运行应用。

存在旧记录时先预检所有来源，再使用本机配置已有的停止命令关闭应用（沿用生命周期超时和用户 PATH），
确认旧宿主与安装目录没有活跃写入进程，再备份、原子转换。停止或迁移失败会显示“配置迁移失败”并阻断业务启动；
排除原因后重启即可重试。不会忽略未知字段、猜来源或修改用户应用数据。

0.9.2 的升级验收范围是 0.8.9 直接升级及全新安装，不提供专门的 0.9.1 恢复路径。
0.9.2 发布后由按平台版本选择替代 0.9.1 默认下载/更新推荐，旧标签和签名制品保留用于审计。

如需离线运维，停止 Bridge 和本地应用后仍可使用发行包内的迁移程序：

```sh
bridge-agent-environment-identity-migration --config-dir <bridge-config-dir> --local-apps-dir <local-apps-root> --managed-apps-dir <managed-apps-root> --host-already-stopped
```

迁移程序随 Bridge 版本构建，无独立版本。它预检全部安装记录、managed tool 当前/回滚来源和制品旁的
`.market.json`，只删除旧分发来源中的 `marketKey`，保留已有 `source.application.environmentKey`，
原始记录保存为 `.json.before-environment-identity-3`，逐文件原子替换，允许失败后重复执行。
离线模式不终止进程；启动模式仅调用配置声明的应用停止命令。两种模式均不连接网络、不猜测来源、不改写清单或用户应用数据。运行中的宿主和应用会阻止迁移。
旧的 source-less 安装保持未认领；从明确选择的来源重新安装时必须加 `--replace --migrate-source-identity`。
已知来源不能通过该开关换到其他环境。回滚宿主前应停止写入并恢复对应原始记录，不用新记录启动旧宿主。

公开读取合同改为 3.0.0，已冻结内容 1.0.0 和市场提交协议 2.0.0 保持不变。
服务端 local-app-service 3.0.0、local-app-market 5.0.0 应与客户端配套切换，旧客户端不兼容新目录协议。
