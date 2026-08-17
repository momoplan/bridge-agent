# 本地应用 OSS 分发契约

百积木官方本地应用由各应用的专属流水线构建、签名和发布。GitHub 或 Gitee Release 只保留源码与
不可变签名制品归档；`local-app-market` 面向客户端返回的所有安装地址必须来自百积木控制的公共 OSS。

## Codex 所有权边界

Codex 由三个互不重叠的本地应用发布单元组成：`baijimu-codex-desktop` 负责桌面安装与工作区切换，
`baijimu-connector-codex` 负责 CLI 与 session/thread/turn 接口，`baijimu-connector-codex-completion`
负责 OpenAI 兼容补全入口。每个仓库分别拥有唯一 `v<version>` 标签、三平台构建与签名、GitHub Release、
OSS 上传、市场版本创建、提交审核和发布回查；三者不得共享标签、制品或 `local-app-market` 记录。
Bridge Agent 不保存这些应用的发布 workflow、Jenkinsfile 或市场发布脚本。

```text
oss://baijimu-lowcode-public-20260420/local-app-artifacts/codex/releases/v<version>/<sha256>/<asset>
https://download.baijimu.com/local-app-artifacts/codex/releases/v<version>/<sha256>/<asset>
```

对象键包含语义版本和制品 SHA-256；上传完成后由 GitHub Actions 匿名完整回下载并校验 SHA-256。市场
只接受无查询参数的公共 OSS 地址，禁止登记 GitHub/Gitee 下载地址。

Bridge Agent 在这条链路中只承担客户端宿主职责：从 `local-app-market` 读取已审核版本，匿名下载并校验 OSS
制品，完成安装、启动、应用内初始化和核心能力调用。它不参与 Codex 源码检出、标签创建、构建、签名、
OSS 上传或市场提交。

禁止把 GitHub/Gitee 下载地址登记为生产市场源，禁止从 MSE 读取数据库凭据或直接修改
`local_app_market` 表，禁止覆盖已发布版本或同名制品。
