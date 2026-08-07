# 本地应用 OSS 分发契约

百积木官方本地应用由各应用的专属流水线构建、签名和发布。GitHub 或 Gitee Release 只保留源码与
不可变签名制品归档；`local-app-market` 面向客户端返回的所有安装地址必须来自百积木控制的公共 OSS。

## Codex

Codex 本地应用由 GitHub Actions 通过 release service 的预签名 PUT 通道，直接上传到百积木公共 OSS；Jenkins
只下载 GitHub Release 中很小的 OSS manifest，并从其中的 OSS 地址校验和发布市场。生产制品地址使用
release service 返回的不可变 OSS 对象：

由于该 release service 的管理接口固定使用 Bridge Agent 的版本命名空间，Codex 上传使用同版本的内部
`bridge-agent-v<version>-codex.1` 影子记录；它不调用 Bridge Agent 的公开发布接口，也不会进入 Bridge Agent
更新目录。

```text
https://lowcode-common.oss-cn-beijing.aliyuncs.com/lowcode/direct-uploads/bridge-agent-release/<date>/<owner>/<id>-<asset>
```

对象由 release service 以不可变身份分配，并在上传完成后匿名完整回下载、校验 SHA-256 后登记。市场
只接受无查询参数的公共 OSS 地址，禁止登记 GitHub/Gitee 下载地址。

正式发布顺序固定为：

1. GitHub Actions 从不可变 Connector 标签完成三平台构建和平台签名，并生成 ZIP 与 SHA-256。
2. GitHub Actions 通过 release service 申请 OSS 预签名地址并直接 PUT 三个平台 ZIP；随后发布 OSS manifest
   到 GitHub Release 作为小型元数据归档。
3. Jenkins `codex-local-app-release` 只读取 OSS manifest，从公共 OSS 匿名下载三平台制品并核对 SHA-256。
4. 使用固定版本的 `baijimu` CLI 创建或复用精确市场版本并提交审核。返回
   `PENDING_REVIEW` 表示流水线提交阶段成功；流水线不得等待人工审核，也不得因此标记失败。
5. 独立审核完成后，以相同不可变提交和版本重跑发布任务。任务复用既有标签、Release、OSS 制品和市场
   版本，只执行幂等校验，并要求 macOS、Windows、Linux 公共市场记录和真实下载摘要全部一致。
6. 在受支持的 Bridge Agent 上完成后台安装、启动、应用内初始化和核心能力验证。

禁止把 GitHub/Gitee 下载地址登记为生产市场源，禁止从 MSE 读取数据库凭据或直接修改
`local_app_market` 表，禁止覆盖已发布版本或同名制品。
