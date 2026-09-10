# 市场 listing 消费者迁移准备

状态：基于已发布 Bridge 0.6.21（35f99f4），不改变客户端版本，未切换桌面市场请求或安装流程，未构建发布制品。候选独立客户端发布 0.7.0 是 MINOR，须由当前用户准确确认；旧认证迁移分支里的历史版本说明不能代替本次授权。

## 已实现的无状态边界

`src/market_distribution` 直接依赖已发布共享合同 local-app-contract 1.0.0 的不可变提交 534803e56ab8b9bf79633ead4709356c2bcd231a。没有复制市场 DTO，没有新增安装记录或缓存。

- 使用部署配置提供的 marketKey 与 HTTPS API 基址；不从域名推导来源环境。
- 页、精确版本和制品路径按市场合同构造。制品 URL 不取自作者环境或清单中的来源 URL。
- 精确响应校验完整 market/listing/source/version；SemVer build metadata 属于精确身份。
- 升级选择只能沿已证明的市场目录和来源应用，不能把同名 appId 的其他环境内容认作升级；只选择高于已安装版本的市场指定目标，不代替市场 latest 策略。
- 无来源的旧记录或私有来源不能回退市场。该层不授权安装、不创建身份、不修改设备数据。
- 页内重复来源、重复目录、错误市场、非前进分页游标失败关闭。

`tests/market_distribution.rs` 使用市场生产者同版本 CModel 5.0.0 作为 dev-dependency，验证真实 HTTP 响应封装、原始 manifest 字节及共享合同解码。现有生产 CModel 1.1.7 和所有认证调用保持原依赖。正式接入前须统一升级响应 owner 依赖并完整验证认证与其他消费者；不能保留两套生产响应协议或通过 JSON Value 重写冻结清单。

## 正式安装接入仍需完成

1. 桌面目录/本机控制 API：替换裸 appId/latestVersion DTO，消费 MarketPage、分页和稳定 listing 引用，并保留名称、图标、应用类型与宿主兼容性展示。
2. 精确版本与制品：使用 MarketListing/FrozenVersion 和市场制品 API，保持来源离线可安装，禁止向制品来源发送平台凭据；现有下载校验的迁移须由制品安全合同 owner 明确，不能临时补造摘要。
3. 安装记录：ConnectorInstallRecord/ConnectorInstallProvenance、managed-tool 状态、安装任务目前未持有完整来源身份。新记录的唯一 owner 仍为 Bridge；版本化记录和正式迁移制品要先明确输入证据、生命周期、失败行为及授权。
4. 本地隔离：connector_data_dir、进程/服务注册、更新匹配和卸载都仍以裸 appId 为键。正式迁移须覆盖全链，不能只改市场 DTO 后允许多来源同名安装。
5. 旧记录：从权威历史映射绑定来源及 listing；来源无法证明则阻断自动升级并说明原因，不能默认当前环境，不能按 appId 或已登录工作区认领。保留用户数据与可审计迁移结果。
6. device-service 授权/调用：需同一来源身份贯穿授权、事件和调用。用户此前延期的是私有环境/多连接凭据迁移，当前工作不得扩展到该认证重构，也不能把缺失的多来源授权声称已完成。
7. 联合验收：相同 appId/version 的两个来源不串升级/数据/权限；旧记录无证据拒绝；跨市场与错误精确版本拒绝；来源断开后市场制品可安装；升级/回退/卸载恢复正确；现有授权响应与 CLI 免重复登录行为回归。

## 中心切换门禁

0.6.21 实际仍调用 `/api/local-app-market/apps` 及 `/apps/{appId}/versions/{version}`。中心 2.0.0 仅开放 `/api/local-app-distribution/listings` 会破坏现有消费者。

本准备代码并不解除该门禁。必须完成客户端接入、身份迁移、准确版本确认及三平台正式发布验收，并处理仍在使用旧客户端的用户后，才能退役旧读取合同。若安排过渡合同，必须由市场 owner 明确所有权、身份映射、支持窗口及退役条件，不能在网关猜测身份、简单重定向或重新开放作者写入。

## 版本确认清单

对象类型：独立客户端项目（不是平台 Component、Bundle 或市场本地应用）。

Bridge Agent，GitHub momoplan/bridge-agent：0.6.21 → 0.7.0，MINOR。范围为稳定 listing/来源身份消费及安装升级映射，保留现有授权响应；未完成多来源设备授权前不得宣称支持同名应用跨来源并装。准确版本授权和安装状态模型授权分别核实，准备阶段不代替上线许可。

## 本轮验证结果

- `cargo test --workspace --locked`：169 项现有核心测试 + 15 项新消费者测试全部通过。
- `cargo clippy --lib --test market_distribution --locked -- -D warnings`：通过。
- `cargo fmt --all -- --check`、`git diff --check`：通过。
- macOS 桌面依赖图可解析，桌面锁文件已同步共享合同；未执行后继版本发布构建。
- Connector 与 managed_tool golden 文件逐字节对照共享合同上述固定提交，无漂移。
- 新增代码未接入桌面调用/安装状态，未改变任何客户端版本、标签或线上配置；不宣称完成市场安装迁移。
