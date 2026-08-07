# 本地应用 OSS 分发契约

百积木官方本地应用由各应用的专属流水线构建、签名和发布。GitHub 或 Gitee Release 只保留源码与
不可变签名制品归档；`local-app-market` 面向客户端返回的所有安装地址必须来自百积木控制的公共 OSS。

## Codex

Codex 本地应用使用以下生产前缀：

```text
oss://lowcode-common/local-app-artifacts/codex/releases/v<version>/<sha256>/<asset>
https://lowcode-common.oss-cn-beijing.aliyuncs.com/local-app-artifacts/codex/releases/v<version>/<sha256>/<asset>
```

对象键同时包含语义版本和制品 SHA-256。相同版本、相同摘要只能对应相同字节，禁止覆盖已发布版本或把
不同内容写入同一对象身份。ZIP 和对应 `.sha256` 文件都必须设置长期 immutable 缓存头。

正式发布顺序固定为：

1. GitHub Actions 从不可变 Connector 标签完成三平台构建和平台签名，并生成 ZIP 与 SHA-256。
2. Jenkins `codex-local-app-release` 下载 GitHub 归档，核对 GitHub 服务端摘要和 `.sha256`。
3. Jenkins 上传内容寻址对象到公共 OSS，随后匿名完整回下载并再次核对 SHA-256。
4. 使用固定版本的 `baijimu` CLI 创建或复用精确市场版本并提交审核。返回
   `PENDING_REVIEW` 表示流水线提交阶段成功；流水线不得等待人工审核，也不得因此标记失败。
5. 独立审核完成后，以相同不可变提交和版本重跑发布任务。任务复用既有标签、Release、OSS 制品和市场
   版本，只执行幂等校验，并要求 macOS、Windows、Linux 公共市场记录和真实下载摘要全部一致。
6. 在受支持的 Bridge Agent 上完成后台安装、启动、应用内初始化和核心能力验证。

禁止把 GitHub/Gitee 下载地址登记为生产市场源，禁止从 MSE 读取数据库凭据或直接修改
`local_app_market` 表，禁止覆盖已发布版本或同名制品。
