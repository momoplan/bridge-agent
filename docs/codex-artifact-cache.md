# ChatGPT/Codex 官方资产 OSS 缓存

国内用户设备直接访问 `chatgpt.com`、`github.com` 或 GitHub release 资产时容易超时。Codex 安装流程应优先尝试官方源；官方源在用户设备不可达时，切换到百积木控制的 OSS 缓存，并在执行前校验 SHA256。

OpenAI 当前桌面端口径是 ChatGPT desktop app，Codex 是其中的软件开发工作区/模式。缓存里的历史资产名仍保留 `codex-app-*`，用于兼容已经发布的安装脚本；其上游必须指向当前官方 ChatGPT desktop app 包。

Windows `get.microsoft.com` 下载项只是 Microsoft Store bootstrap installer，不是完整离线安装包。Windows 默认缓存资产必须是 Microsoft Store 的完整 MSIX/AppX 包，并通过 `Add-AppxPackage` 安装。

## 存储选择

二进制安装资产使用 OSS，不需要用户提供 Gitee 账号。

Gitee 可用于同步源码仓库、发布说明或元数据，但不作为 Codex 安装包的主分发链路。安装包、DMG、zip、tarball、npm tgz 等资产应放在百积木自有 OSS bucket，并通过 manifest 明确记录 upstream URL、版本、抓取时间、文件大小和 SHA256。

当前默认位置：

```text
oss://lowcode-common/codex-artifacts/
https://lowcode-common.oss-cn-beijing.aliyuncs.com/codex-artifacts/latest.json
```

## 同步

从一台能稳定访问 GitHub 的同步机运行：

```bash
OSS_BUCKET=lowcode-common \
OSS_PREFIX=codex-artifacts \
OSS_REGION=cn-beijing \
OSS_ENDPOINT=oss-cn-beijing.aliyuncs.com \
tools/codex-artifacts/sync-codex-artifacts.sh
```

默认同步生产安装常用资产：

- `install.sh`
- `install.ps1`
- `codex-package_SHA256SUMS`
- macOS Apple Silicon / Intel 的 Codex CLI tarball 和 ChatGPT desktop app DMG
- Windows x64 / arm64 的 Codex CLI zip
- Windows x64 / arm64 的 ChatGPT desktop app MSIX
- Codex npm 平台包

同步官方 ChatGPT desktop app DMG 时，加上：

```bash
INCLUDE_OFFICIAL_APP_DMG=1 \
OFFICIAL_APP_DMG_ARCHES="arm64 x86_64" \
tools/codex-artifacts/sync-codex-artifacts.sh
```

同步 Windows ChatGPT desktop app MSIX 时，默认会缓存 x64 和 arm64 包；如只需要 x64，可覆盖 `WINDOWS_APP_MSIX_ARCHES=x64`：

```bash
INCLUDE_WINDOWS_APP_MSIX=1 \
WINDOWS_APP_MSIX_ARCHES=x64 \
tools/codex-artifacts/sync-codex-artifacts.sh
```

该资产来源于 Microsoft Store 当前 ChatGPT desktop app 产品 `9PLM9XGG6VKS`。同步任务可以使用能解析 Microsoft Store FE3/CDN 元数据的受控来源获取原始 MSIX，但不得修改或重打包；发布到 OSS 前必须记录 upstream URL、文件大小和 SHA256。

当前如只需要给 Apple Silicon 用户补 App，可先同步 arm64：

```bash
CODEX_ASSET_REGEX='^codex-aarch64-apple-darwin\.tar\.gz$' \
INCLUDE_OFFICIAL_APP_DMG=1 \
OFFICIAL_APP_DMG_ARCHES=arm64 \
tools/codex-artifacts/sync-codex-artifacts.sh
```

需要临时只同步某个资产时，覆盖 `CODEX_ASSET_REGEX`：

```bash
CODEX_ASSET_REGEX='^codex-aarch64-apple-darwin\.tar\.gz$' \
tools/codex-artifacts/sync-codex-artifacts.sh
```

同步脚本会：

1. 调用 GitHub Release API 读取 `openai/codex` latest。
2. 按白名单下载官方 release 资产。
3. 校验下载文件大小。
4. 计算 SHA256。
5. 上传到 `codex-artifacts/releases/<tag>/`。
6. 发布 `codex-artifacts/latest.json`。

## 安装侧消费规则

安装流程在用户设备上执行时：

1. 先尝试官方 URL。
2. 官方 URL 超时、连接失败或 HTTP2 断流后，下载 OSS `latest.json`。
3. 从 manifest 中选择匹配当前 OS/arch 的资产。
4. 下载 `mirror_url`。
5. 用 manifest 中的 `sha256` 校验文件。
6. 校验通过后再解压、复制、挂载或执行。

用户设备上的安装脚本不要依赖 `python3` 解析 manifest；干净 macOS 可能会弹出 Xcode 命令行工具安装提示。优先使用系统自带 shell/awk/plutil/osascript，或者由平台侧预先解析后把明确的 URL、SHA256 和资产名传给设备。

不得使用未记录 upstream 和 SHA256 的第三方镜像。OSS 缓存只是官方资产的缓存，不是新的上游源。

Windows App 安装侧从 manifest 中选择兼容资产名：

```text
codex-app-windows-x64.msix
codex-app-windows-arm64.msix
```

安装时先下载 `mirror_url`，校验 SHA256，再运行：

```powershell
Add-AppxPackage -Path .\codex-app-windows-x64.msix
```

安装后验证当前 ChatGPT desktop app/Codex 包和开始菜单入口。不要把 `codex-app-windows-msstore-installer.exe` 当作默认安装资产；它只会打开 Microsoft Store 链路，无法保证静默安装。

## 当前 1393 问题对应资产

工作区 1393 的 macOS Apple Silicon 设备卡在：

```text
https://github.com/openai/codex/releases/latest/download/codex-aarch64-apple-darwin.tar.gz
```

对应 OSS fallback 资产应从 manifest 中选择：

```text
codex-aarch64-apple-darwin.tar.gz
codex-app-aarch64-apple-darwin.dmg
```
