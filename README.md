# bridge-agent

`bridge-agent` 是安装在用户自己电脑上的本地代理。

它的职责不是替代顶层 agent，而是把这台机器上经过用户授权的本地能力，安全地暴露给外部 agent 调用。

先把最容易混淆的一点说清楚：

- 任何人都可以使用这个开源项目
- 但如果你想让外部 agent 使用你自己电脑上的能力，你必须先在自己的机器上安装并运行 `bridge-agent`
- 外部 agent 不能在“用户什么都不装”的前提下直接获得本地 shell 或本地服务能力

## 下载

给最终用户分发时，直接使用 GitHub Releases 里的安装包：

- 最新版本页：[`Releases / latest`](../../releases/latest)
- macOS：优先下载 universal `.dmg`
- Windows：下载 `.msi` 或安装器
- Linux：下载 `.AppImage` / `.deb`

如果你只是普通用户，直接下载对应平台安装包即可，不需要本地安装 Rust 或 Node 环境。

如果你拿到的是旧版分发物，需要额外注意：

- Intel Mac 只能运行 `x64` 或 universal 的 macOS 安装包
- Apple Silicon Mac 可以运行 `arm64` 或 universal 的 macOS 安装包
- 如果误装了另一种架构，Finder 里能看到应用，但启动会失败

## 它解决什么问题

`bridge-agent` 解决的是“外部 agent 如何安全地调用用户自己电脑上的本地能力”这个问题。

典型场景：

- 让外部 ChatGPT / Claude 调用本机桌面控制能力，例如 `computer.screenshot` / `computer.click`
- 让外部 agent 调用本地已经存在的业务服务，例如本机 Java / Node / Python 服务
- 让本地机器不暴露公网入站端口，仍然能被远端授权访问

## 架构关系

系统里有三个角色：

- `bridge-agent`
  - 安装在用户自己的机器上
  - 管理“我这台机器对外开放哪些服务和方法”
- `relay`
  - 负责转发和鉴权
  - 不直接执行本地命令
- 外部 agent / app
  - 通过 `relay` 调用某个设备上的 `service.method`

调用链路：

1. 用户在本机安装并启动 `bridge-agent`
2. `bridge-agent` 打开授权页面，用户确认授权
3. `bridge-agent` 获取 `agent token`，主动连接 `relay`
4. 用户把某个设备服务授权给外部 app / agent
5. 外部 app / agent 拿到 `client token`
6. 外部 app / agent 通过 `relay` 调用本机暴露的 `service.method`

## 谁需要安装

- 如果你只是调用别人已经开放出来的服务：不需要安装 `bridge-agent`
- 如果你想让外部 agent 使用你自己电脑上的能力：需要安装 `bridge-agent`
- 如果你是平台运营方：需要部署 `relay`

所以它应该是一个可审计、可安装、可分发的开源本地项目，而不是一个纯云端工具。

## 当前工程形态

`bridge-agent` 现在是一个完整的本地端工程，不再只是单个 CLI。

它包含三层：

- Rust core library：负责配置、服务注册、WebSocket 长连、调用转发、日志和本地安全策略
- CLI：适合服务器、脚本或纯命令行场景
- Tauri desktop app：适合最终用户安装、管理本地服务并打包分发

## 当前能力

- 通过 WebSocket 主动连接 relay
- 上报最小协议 `agent_id + services[]`
- 按 `service + method + arguments` 接收调用
- 本地配置里支持三种方法绑定
  - `computer_use`
  - `shell_command`
  - `http`
- 本地管理端可编辑服务、方法、超时、allowlist、日志保留等配置
- 已接浏览器授权启动和轮询，授权成功后会把 `agent token` 自动写回本地配置
- 可打包为桌面应用分发

## 对外暴露的模型

外部看到的是业务服务模型，而不是本地实现细节。

例如：

- `computer.screenshot`
- `computer.click`
- `local-java-service.invokeApi`

这里：

- `computer` / `local-java-service` 是服务
- `screenshot` / `click` / `invokeApi` 是方法

外部不会看到：

- 这是 shell 实现的
- 还是 HTTP 转发实现的

这些都只是本地 `bridge-agent` 的内部 binding 细节。

注意：

- `computer_use` / `shell` / `http` 都不在 agent-relay 协议里暴露
- relay 看到的是 `services[].methods[]`，例如 `computer.screenshot`

## 项目结构

- `src/lib.rs`
- `src/config.rs`
- `src/runtime.rs`
- `src/services.rs`
- `src/main.rs`
- `src-tauri/src/main.rs`
- `src/App.tsx`

## 本地配置模型

默认配置文件会写到系统配置目录下的 `agent-config.json`。

示例配置可以用下面命令生成：

```bash
cargo run -- init-config
```

核心字段：

- `platform.base_url`
- `platform.workspace_id`（授权成功后自动写回）
- `relay.url`
- `relay.agent_id`
- `relay.token`
- `runtime.default_timeout_secs`
- `services[].methods[].binding`

`binding.type` 只存在于本地配置里，用来决定本机怎么执行方法，不会进入 relay 协议。

## 快速开始

1. 生成配置文件

```bash
cargo run -- init-config
```

2. 编辑本地配置，声明你要开放的服务和方法

例如：

- 开一个 `computer.screenshot`
- 再开一个 `computer.click`
- 或者开一个映射本地 Java 服务的 `local-java-service.invokeApi`

3. 启动 agent

```bash
cargo run -- run
```

4. 点击浏览器授权，在网页中选择目标工作区并完成批准

5. 授权成功后，外部 app / agent 才能通过 relay 调用这台机器上的服务

如果你要给最终用户分发，一般不是让用户跑 `cargo run`，而是直接分发 Tauri 打包后的桌面应用。

## CLI

初始化配置：

```bash
cargo run -- init-config
```

打印示例配置：

```bash
cargo run -- print-example-config
```

启动 agent：

```bash
cargo run -- run
```

也可以指定配置文件：

```bash
cargo run -- run --config /path/to/agent-config.json
```

## Windows 后台服务

如果需要安装后长期后台运行，并且不依赖用户登录，可以使用新增的 `bridge-agent-service` 二进制，把核心 runtime 作为 Windows Service 托管。

服务入口：

- `bridge-agent-service.exe`

调试运行：

```bash
cargo run --bin bridge-agent-service -- --console
```

也可以显式指定配置文件：

```bash
cargo run --bin bridge-agent-service -- --console --config /path/to/agent-config.json
```

Windows 正式安装时，建议把服务注册成固定服务名 `BridgeAgent`，并把配置文件放到共享路径：

- `C:\ProgramData\Baijimu\BridgeAgent\agent-config.json`

当前默认行为：

- Windows 桌面端 / CLI 如果发现上面的共享配置文件已存在，会优先读取它
- 否则仍然回退到当前用户自己的配置目录
- Windows Service 在没有显式传入 `--config` 时，会默认使用上面的共享路径

一个典型的服务注册命令示例：

```powershell
sc.exe create BridgeAgent binPath= "\"C:\Program Files\Bridge Agent\bridge-agent-service.exe\" --config \"C:\ProgramData\Baijimu\BridgeAgent\agent-config.json\"" start= delayed-auto
```

实现和打包时要注意：

- 不要把 Tauri 桌面程序直接改成服务，服务化的是 Rust runtime/CLI 这一层
- MSI 里需要负责注册服务、升级时先停服务再替换文件、完成后重新拉起
- 如果桌面端也要编辑同一份共享配置，安装器需要给 `C:\ProgramData\Baijimu\BridgeAgent` 配置合适 ACL，让服务账号和交互用户都能读写
- `bridge-agent-service.exe`、桌面端 exe、安装器和后续升级器都要做正式代码签名

## 桌面应用开发

安装前端依赖：

```bash
npm install
```

启动桌面开发版：

```bash
npm run tauri dev
```

构建前端：

```bash
npm run build
```

## GitHub 发布

`bridge-agent` 现在可以继续通过 GitHub Actions 自动产出 macOS 安装包，但 macOS 这条线必须带上 `Developer ID Application` 签名和 Apple 公证。

仓库里的工作流文件是：

- `.github/workflows/release-bridge-agent.yml`

触发方式：

- 推送 tag：`bridge-agent-v0.1.12`
- 或者在 GitHub Actions 页面手动执行 `workflow_dispatch`

macOS 自动签名和公证前，需要先在仓库的 GitHub Secrets 里配置这些值：

- `APPLE_CERTIFICATE`
  - `Developer ID Application` 证书导出的 `.p12` 文件内容，先转成 base64
- `APPLE_CERTIFICATE_PASSWORD`
  - 导出 `.p12` 时设置的密码
- `APPLE_API_ISSUER`
  - App Store Connect API Key 的 Issuer ID
- `APPLE_API_KEY`
  - App Store Connect API Key 的 Key ID
- `APPLE_API_PRIVATE_KEY`
  - 下载得到的 `.p8` 私钥全文内容

导出证书并转成 base64 的示例命令：

```bash
openssl base64 -A -in /path/to/developer-id-application.p12 -out certificate-base64.txt
```

这里的 `Developer ID Application` 证书用于 GitHub 下载分发；如果以后需要给 `.pkg` 安装器签名，还要额外申请 `Developer ID Installer` 证书。

工作流在 macOS runner 上会自动完成这些事情：

- 导入 `Developer ID Application` 证书
- 用 `Developer ID Application: Xiaofeng Zhang (H82D8SYZ94)` 给 Tauri 的 macOS 产物签名
- 用 App Store Connect API key 提交 notarization
- 等待公证通过后再把构建产物上传到 GitHub Release

如果这些 secrets 没配齐，macOS 任务会直接失败，避免把未签名或未公证的安装包发布出去。

## 打包分发

推荐的正式分发方式不是手工发二进制，而是通过 GitHub Releases 自动上传各平台安装包。

macOS 推荐直接构建 universal 安装包，这样最终用户不需要自己区分 Intel 和 Apple Silicon：

```bash
npm run tauri:build:macos-universal
```

如果只是在当前机器本地验证，也可以先跑调试包：

```bash
npm run tauri:build:macos-universal -- --debug
```

发布步骤：

1. 同步更新版本号
   - `package.json`
   - `Cargo.toml`
   - `src-tauri/Cargo.toml`
   - `src-tauri/tauri.conf.json`
2. 推送版本 tag，例如 `bridge-agent-v0.1.12`
3. GitHub Actions 会自动构建并把安装包上传到当前 tag 对应的 Release
4. 最终用户从仓库的 [`Releases / latest`](../../releases/latest) 直接下载

对应 workflow：

- `.github/workflows/release-bridge-agent.yml`

调试打包：

```bash
npm run tauri build -- --debug
```

本机验证过的产物路径：

- `src-tauri/target/universal-apple-darwin/debug/bundle/macos/Bridge Agent.app`
- `src-tauri/target/universal-apple-darwin/release/bundle/dmg/Bridge Agent_0.1.12_universal.dmg`

后续如果要做 Windows / Linux 分发，直接在对应平台执行同样的 `tauri build` 即可。

注意：

- macOS 正式对外分发建议补代码签名和 notarization
- Windows 正式对外分发建议补代码签名
- 没签名也可以先做内测发布，但安装体验会差一些

## 方法绑定

### 1. `computer_use`

适合 GPT-5.4 这类模型驱动的桌面控制服务，例如：

- `computer.screenshot`
- `computer.click`
- `computer.type`

当前首版实现：

- 只在 macOS 上启用
- 依赖系统的辅助功能权限和屏幕录制权限
- 内建动作包括截图、单击、双击、移动、拖拽、滚动、输入文本、按键和等待

### 2. `shell_command`

适合终端类服务，例如：

- `terminal.exec`

本地策略包括：

- `root_dir`
- `allow_commands`
- 超时限制
- 环境变量白名单

### 3. `http`

适合把本地 Java / Node / Python 服务映射成业务方法，例如：

- `local-java-service.invokeApi`

当前行为：

- `POST/PUT/PATCH`：把 `arguments` 作为 JSON body 转发
- `GET/DELETE`：把 `arguments` 转成 query string
- 返回状态码、响应头和响应体

## 安全边界

- 本地机器不开放入站端口给外网
- 所有调用都通过本地 agent 主动外连 relay
- `computer_use` 不等于任意 shell，它只执行受控的桌面动作
- shell 方法必须显式 allowlist
- cwd 不能逃逸 root_dir
- 每个方法调用都有超时

如果要进一步提高隔离级别，仍然建议搭配单独用户、容器或系统沙箱使用。

## 这个仓库还应该继续补什么

如果它要作为公开项目给外部用户使用，后续还应该持续补这些文档：

- 安装说明：macOS / Windows / Linux 各自怎么安装
- 授权流程：用户第一次启动后会发生什么
- 配置说明：每个字段的含义和安全影响
- 服务模型说明：什么叫 service、什么叫 method
- 安全模型：哪些能力默认不开、哪些风险需要用户自己确认
- 发布说明：如何下载桌面包、如何校验版本、如何查看源码

当前 README 先把最关键的产品边界写清楚了：`bridge-agent` 是一个需要安装在本机的开源本地代理，而不是一个无需安装就能直接获得本地能力的云工具。
