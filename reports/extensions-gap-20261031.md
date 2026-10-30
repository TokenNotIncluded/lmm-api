# LMM API 扩展模块未完成项核对

日期：2026-10-31（Asia/Taipei）

源码基准：`TokenNotIncluded/lmm-api-source@7520d0bc1d71c080e6d787bf5f7ad1bfa9434c54`。
本次核对已提交源码、路由、宿主装配与调用方。没有运行产品测试或验证线上状态；本次交付是状态核对记录，没有实现这些待办。

## 修正后的八项清单

| 业务模块 | 已有能力 | 确切未完成项 |
| --- | --- | --- |
| 促销奖励 | 活动策略、预算、申请幂等、pending 恢复、PostgreSQL 记录与内部 HTTP | Core 奖励资格与入账合同、宿主装配；兑换码、邀请返佣、签到、活动红包的完整业务流程。[1] |
| 社区论坛 | 板块、主题、回复、锁定、置顶、隐藏与审计的领域接口 | 生产 Core 权限适配、PostgreSQL Repository 与 schema、配置、宿主装配和浏览器接口；工作量超过单纯注册模块。[2] |
| 支付 | 钱包充值；Stripe 钱包全额与部分退款及恢复路径 | 当前正式装配下的其他渠道退款、订阅支付、商店外部支付。EPay 的遗留退款方法未开放给当前 Core adapter；不能把方法存在算作运行可用。[3] |
| 商店 | 余额购买、库存与交付、全额本金退款、商品总量和单笔数量限制 | 优惠券、提货码/邮箱提货、游客购买、每买家累计限购、外部收款和部分退款。[4] |
| AI 助手 | 持久会话、多轮工具调用、恢复、服务端内置快捷问题和若干工具实现 | 真正逐 token 流式、自定义预设配置、搜索等工具的正式配置与装配；内置快捷问题发送的 preset_id 当前也被聊天入口拒绝。[5] |
| 工具市场 | 发布、人工审核、调用历史、恢复、投诉、收入及授权记录管理 | AI 自动审核、审核配置、AI 审核历史；普通调用历史与人工审核不是整体缺失。[6] |
| 运营管理 | 公告与已读记录、已审阅脚本的固定快照及公开下载 | 仓库配置、凭据管理、脚本拉取/同步和同步后的快照发布；现有三条仓库路由返回 501。[7] |
| 远程控制 | 加密消息转发、SSE、游标、撤销复验；Core 已有 OAuth 授权基础 | 公开代理与 Go 模块的 OAuth 接入，以及 OAuth 新执行请求的反滥用事务检查。[8] |

这里按八个业务域计数。“未完成”表示仍有具体用户流程缺失，不表示八个模块都没有实现。生产可用性需要另外的集成证据，本次不提供总体完成百分比。

## 关键断点与依赖

### 资金相关功能

促销的生产 Rewards 实现仍为 `UnavailableRewards`。注册 promotions 模块并不能使奖励入账。必须由 Core 校验资格、预算、资金来源和幂等，再出具可恢复的资金结果。[1]

当前 Payments RPC 的创建入口是 `PrepareTopup`，用途是钱包充值。订阅授予、商店收款需要各自冻结业务对象并绑定支付事实，不能直接复用充值来代替。商店现有 Core 合同冻结原始单价、数量和整笔资金操作；部分退款需要独立退款身份与累计退款限制，而非仅修改 Go 展示金额。[3][4]

每买家累计限购可在商店域内推进，但需复用事务和已有订单记录，并明确待支付、资金结果未知、取消和退款的计数口径。商品总量限制与单笔限制已经在下单事务中检查。[4]

### 助手与工具市场

Core 助手请求明确使用 `stream: false`，现有 RPC 是 unary，Go 在 turn 完成后取得 FinalText。前端虽然已有流式消费逻辑，但后端链路尚未输出模型增量事件。完整流式需要贯通 Core、RPC、Go 和前端，并保留同一 turn 的恢复和结算语义。[5]

四个内置快捷问题由后端返回，前端点击时会传 `preset_id`，但聊天入口目前拒绝所有非空 ID。这是可单独修复的用户路径问题，与自定义系统预设的配置和发布是两项工作。已有网页搜索适配器及 composition 注入点，正式启动代码尚未提供依赖配置。[5]

工具市场 AI 审核页面请求审核设置和 AI 审核历史，但新后端没有对应路由、任务与记录。已有人工审核、调用记录和投诉处理应继续保留为已实现能力。[6]

### 论坛、运营与远控

论坛的生产 Authority 和持久化 Repository 在源码中明确未实现。测试内存实现不能作为可重启的产品服务；需要同时补权限、数据库和宿主/浏览器接线。[2]

运营的 `GET/PUT /api/scripts/repository` 与 `POST /api/scripts/repository/pull` 返回 `SCRIPT_REPOSITORY_UNSUPPORTED`。Web 已有配置与拉取调用方，可以围绕实际仓库提交构建新的脚本快照；现有职责是内容分发，不包含执行远程脚本。[7]

Core 已能验证具有 `remote:control` scope 的 OAuth access token，内部 RPC 与 Go 客户端也识别 OAuth 来源。公开 Core proxy 仍只用 native authorizer，Go 再次要求 Native，新执行检查也仅支持 native。只删除 Go 的限制不足以完成 OAuth 接入。[8]

## 固定源码证据

[1]: https://github.com/TokenNotIncluded/lmm-api-source/blob/7520d0bc1d71c080e6d787bf5f7ad1bfa9434c54/services/extensions/internal/modules/promotions/promotions.go
[2]: https://github.com/TokenNotIncluded/lmm-api-source/blob/7520d0bc1d71c080e6d787bf5f7ad1bfa9434c54/services/extensions/internal/modules/forum/types.go
[3]: https://github.com/TokenNotIncluded/lmm-api-source/blob/7520d0bc1d71c080e6d787bf5f7ad1bfa9434c54/docs/payment-gateways.md
[4]: https://github.com/TokenNotIncluded/lmm-api-source/blob/7520d0bc1d71c080e6d787bf5f7ad1bfa9434c54/services/extensions/internal/modules/store/legacy_catalog.go
[5]: https://github.com/TokenNotIncluded/lmm-api-source/blob/7520d0bc1d71c080e6d787bf5f7ad1bfa9434c54/services/extensions/internal/webcompat/assistant.go
[6]: https://github.com/TokenNotIncluded/lmm-api-source/blob/7520d0bc1d71c080e6d787bf5f7ad1bfa9434c54/services/extensions/internal/webcompat/toolmarket.go
[7]: https://github.com/TokenNotIncluded/lmm-api-source/blob/7520d0bc1d71c080e6d787bf5f7ad1bfa9434c54/services/extensions/internal/webcompat/operations.go
[8]: https://github.com/TokenNotIncluded/lmm-api-source/blob/7520d0bc1d71c080e6d787bf5f7ad1bfa9434c54/services/core/src/remote_control.rs

补充定位：

- [宿主实际装配](https://github.com/TokenNotIncluded/lmm-api-source/blob/7520d0bc1d71c080e6d787bf5f7ad1bfa9434c54/services/extensions/internal/composition/runtime.go)
- [Stripe 退款能力](https://github.com/TokenNotIncluded/lmm-api-source/blob/7520d0bc1d71c080e6d787bf5f7ad1bfa9434c54/services/extensions/internal/modules/payments/stripe.go)
- [EPay Core adapter](https://github.com/TokenNotIncluded/lmm-api-source/blob/7520d0bc1d71c080e6d787bf5f7ad1bfa9434c54/services/extensions/internal/modules/payments/epay.go)
- [支付 RPC](https://github.com/TokenNotIncluded/lmm-api-source/blob/7520d0bc1d71c080e6d787bf5f7ad1bfa9434c54/contracts/proto/lmm/core/v1/payments.proto)
- [商店实际下单限制](https://github.com/TokenNotIncluded/lmm-api-source/blob/7520d0bc1d71c080e6d787bf5f7ad1bfa9434c54/services/extensions/internal/modules/store/orders.go)
- [商店高级订单输入](https://github.com/TokenNotIncluded/lmm-api-source/blob/7520d0bc1d71c080e6d787bf5f7ad1bfa9434c54/services/extensions/internal/modules/store/legacy_orders.go)
- [商店退款入口](https://github.com/TokenNotIncluded/lmm-api-source/blob/7520d0bc1d71c080e6d787bf5f7ad1bfa9434c54/services/extensions/internal/modules/store/legacy_refunds.go)
- [Core 商店资金合同](https://github.com/TokenNotIncluded/lmm-api-source/blob/7520d0bc1d71c080e6d787bf5f7ad1bfa9434c54/services/core/src/commerce.rs)
- [Core 助手模型执行](https://github.com/TokenNotIncluded/lmm-api-source/blob/7520d0bc1d71c080e6d787bf5f7ad1bfa9434c54/services/core/src/model_http/assistant_execution.rs)
- [助手 RPC](https://github.com/TokenNotIncluded/lmm-api-source/blob/7520d0bc1d71c080e6d787bf5f7ad1bfa9434c54/contracts/proto/lmm/core/v1/assistant.proto)
- [搜索 provider](https://github.com/TokenNotIncluded/lmm-api-source/blob/7520d0bc1d71c080e6d787bf5f7ad1bfa9434c54/services/extensions/internal/modules/assistant_services/search_provider.go)
- [助手工具装配](https://github.com/TokenNotIncluded/lmm-api-source/blob/7520d0bc1d71c080e6d787bf5f7ad1bfa9434c54/services/extensions/internal/composition/assistant.go)
- [正式应用入口](https://github.com/TokenNotIncluded/lmm-api-source/blob/7520d0bc1d71c080e6d787bf5f7ad1bfa9434c54/services/extensions/internal/app/run.go)
- [AI 审核前端调用](https://github.com/TokenNotIncluded/lmm-api-source/blob/7520d0bc1d71c080e6d787bf5f7ad1bfa9434c54/apps/web/src/features/market-ai-review/api.ts)
- [Core OAuth 远控授权](https://github.com/TokenNotIncluded/lmm-api-source/blob/7520d0bc1d71c080e6d787bf5f7ad1bfa9434c54/services/core/src/identity/grants.rs)
- [远控新增执行 gate](https://github.com/TokenNotIncluded/lmm-api-source/blob/7520d0bc1d71c080e6d787bf5f7ad1bfa9434c54/services/core/src/identity/resource.rs)
- [Go 远控鉴权](https://github.com/TokenNotIncluded/lmm-api-source/blob/7520d0bc1d71c080e6d787bf5f7ad1bfa9434c54/services/extensions/internal/modules/remotecontrol/module.go)
