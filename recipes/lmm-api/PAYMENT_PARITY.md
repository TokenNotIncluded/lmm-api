# 支付网关对齐与验收范围

支付接入本质上是保存一笔订单，把它交给支付商，再用可信收款凭证让 Core 只记一次账。网关名称、按钮和适配文件分别只是这条链路的一部分；钱包充值、订阅、商店和退款有不同的归属、状态和金额来源，必须分别完成。

## 范围与来源

对照基线为旧单体 `traditional` 的固定提交 [`3948a0a1ee1cefdd016a7aa45a11851545bb4b85`](https://github.com/TokenNotIncluded/lmm-api/tree/3948a0a1ee1cefdd016a7aa45a11851545bb4b85)。本文件基于该版本实际路由、控制器、服务、配置与 UI 模板，描述应恢复的行为和仍需完成的产品目标；源码存在只证明存在实现，不证明当前新架构运行通过。完整责任与恢复要求继续使用 [COMPLETION_SCOPE.md 的 A05](COMPLETION_SCOPE.md#a05)、A06 和 S11。

FastPay 已由用户明确淘汰。后续重建不实现 FastPay adapter，不注册其路由、运行配置或支付选项，不把旧配置迁成启用入口。历史账目可保留原 provider 标记并只读展示；旧迁移负向 fixtures 中的 `fastpay` 继续用于验证未知 provider 被拒绝。删除历史证据不是淘汰网关的条件。

## 旧能力与重建目标

### 钱包和订阅

| provider | 旧钱包能力 | 旧订阅能力 | 退款事实与重建边界 |
| --- | --- | --- | --- |
| EPay | 支付宝 `alipay`、微信 `wxpay`、Linux DO `epay`、管理员配置的自定义 type；本地订单先持久化，签名回调匹配原金额后入账 | EPay 一次性购买套餐 | EPay 是协议，不能由名字推断通用查询/退款接口；按具体上游能力实现，保留人工处理状态 |
| Stripe | Checkout、延迟付款成功/失败、过期；绑定环境、金额、币种与原交易 | 首次购买、invoice 周期收款、失败与订阅状态更新 | 钱包退款回调绑定原 PaymentIntent；旧钱包退款路径明确未覆盖订阅退款，后者保持独立目标 |
| Creem | 服务端商品目录、固定商品 checkout、paid/onetime 校验、原产品和交易绑定 | 远端产品金额/币种/周期校验，首次购买、续费与生命周期事件 | 旧 webhook 分发没有退款处理；自动退款不能记为旧版已有或本轮已完成 |
| Waffo Global | 独立 SDK、正式/测试凭据、服务端支付子方式、钱包下单及收款/退款回调、企业账单资料 | 固定旧路由没有 Global 订阅下单入口 | 钱包全额/部分退款回调；Global 与 Pancake 的协议和凭据分开，订阅需求另行实现和验收 |
| Waffo Pancake | hosted checkout、商品绑定、地区/语言/币种、固定环境验签、过期订单上游查询 | one-time 与 recurring 产品映射、首次购买、周期付款及生命周期事件、客户 portal | 钱包与订阅退款分别绑定原交易；商店退款还需执行查询，不能以 ticket 或通知替代资金结果 |

各 provider 的下单、周期事件与退款细节来自对应控制器，而不是仅依据路由名称。[S10] 旧 UI 的七个一键模板为 **EPay 支付宝、EPay 微信、LINUX DO Credit、Stripe、Creem、Waffo、Waffo Pancake**。这是五个 provider 下的七种预设入口，不是七套独立网关。自定义 EPay type 由管理员明确填写；Waffo 的 Card、Apple Pay、Google Pay 属于该 provider 的服务端子方式。平台余额可购买订阅，是内部账本路径，不计为外部支付网关。[S1–S3]

Linux DO 的支付单位是 LDC，使用 EPay 协议并不使它成为 CNY。恢复时保留显式单位、支付方式和服务端定价，LDC 收款与现金收入分别呈现；普通 `alipay`、`wxpay` 按 CNY 合同处理。旧商店平台 Linux DO 绑定精确地址 `https://credit.linux.do/epay`，商户自有普通 EPay 禁止把该主机当 CNY 网关。兑换率由已确认配置提供，不从名称或旧 UI 示例推导。[S2, S4]

### 商店付款与退款

| 商店 provider | 付款归属 | 旧退款合同 | 重建验收目标 |
| --- | --- | --- | --- |
| `balance` | 平台余额 | 内部余额退款 | 用户/付款方授权、库存与交付、Core 唯一结算和退款 |
| `external:epay` | 商户自己的 EPay 配置 | 人工退款，具体上游能力未知 | 商户隔离、CNY/type/订单绑定；未实现退款不得伪造成功 |
| `external:waffo_pancake` | 商户自己的 Pancake 配置 | refund ticket、查询、签名通知；部分退款有条件 | 冻结原商户/环境/订单/payment；申请与实际执行分别持久化 |
| `platform:waffo_pancake` | 平台 Pancake | 同上 | 平台资金权限、商户结算、执行结果与 Core 回冲一致 |
| `platform:linuxdo` | 平台 Linux DO，LDC | 仅全额退款；没有通用部分退款和执行查询能力 | 精确网关与原交易绑定，未知响应按原操作恢复，不借用其他 EPay 能力 |

商店使用自己的矩阵。Stripe、Creem 或 Waffo 在钱包可用，不代表旧商店已支持它们，也不能直接将钱包成功回调视为商品交付授权。新增商店网关需补明确的 Commerce 归属、外部付款、卖家收入、退款和恢复合同，原 A06 的完整购买退款目标继续保留。[S4–S5]

### 配置与持续订单处理

管理端应恢复各网关独立的启用条件、凭据、环境、商品/支付方式和金额单位。秘密只写，公开渠道只返回当前用户可用的非敏感字段；用户受众、注册等待期、充值限额和权限在服务端执行。钱包商品和订阅商品分别判断，不能因钱包商品未配置而隐藏合法订阅或丢弃其回调。[S2, S6]

Pancake 还包括 catalog、pair、save、subscription-product 和 product-options 管理行为，以及仅向合法网页会话发放的 customer portal。调用方不能选择其他 customer/store/environment；portal 登录证明不进入日志或持久缓存。过期订单在 checkout 时限后查询支付商账本，查询失败、仍 pending 或已付款时保留原单；不能只凭本地时间关单。[S3, S7]

## 本轮实现边界

本轮推进的是 **多 provider 钱包充值候选**：Core 对各 provider 验签并持久绑定支付证明，Go 宿主负责外部交互、队列与状态恢复，Web 按“服务端渠道 → 精确报价 → 明确确认 → 原订单读取”接入。具体 API、配置字段、源文件和执行结果在独立源码仓库维护；本配方不复制源码，不把尚未提交的组合称为固定发布版本。

钱包候选不能替代订阅、商店外部收款、全部退款、管理端配置迁移或真实支付商验收。每一项均按上面矩阵保留 owner、固定输入、开发者自测、真实 Core/Go/PG、OS 重启、浏览器、sandbox/真实支付商状态；测试数字与最终状态由本轮实际回执更新，不计算没有完整证据分母的总体百分比。用户明确排除的 FastPay 不进入待实现队列。

## 联合验收合同

| 阶段 | 需要证明的行为 | 可接受的完成证据 |
| --- | --- | --- |
| 渠道与报价 | 各网关显式装配，用户权限/金额/币种/商品匹配；固定商品不可改价；LDC 不串现金；FastPay 拒绝 | 服务端真实读取与拒绝请求、配置快照、精确报价及无额外资金写入 |
| 确认与下单 | 用户确认原报价；固定 actor/payer/渠道/环境/订单/金额；同 key 幂等 | Web 与实际后端联合；重复点击、响应丢失、401 和换账号不产生第二笔付款 |
| 回调与入账 | Core 验原始签名及商户/环境/时间窗/金额/币种/产品/交易归属，持久证明只消费一次 | 各 provider 正反向协议向量，真实 PG 唯一账本与余额读回；provider paid 与 Core credited 分离 |
| 异常恢复 | 接纳后 ACK 前、Go 持久队列后、Core COMMIT 后、外部请求后断链；旧单在配置变更后仍可恢复 | 真正重启 Core/Go OS 进程，核原 operation、调用次数、数据库与账本；UNKNOWN 不换 key、不提前释放 |
| 订阅 | 首次购买、续费周期、失败/取消/恢复、产品价格变更与退款分别绑定原合同 | 各网关独立生命周期记录；同一周期不重复授予权益；钱包用例不能抵扣 |
| 商店 | 商户/平台收款边界、库存、交付、卖家收入、用户权限与退款竞争 | Core+Go+独立 PG 的完整订单回读；所有者/跨商户/跨账户拒绝 |
| 退款 | 原支付来源、整数累计上限、部分退款取整、未知预留、与消费竞争、历史来源绑定 | 原 intent/proof 或不可变历史 basis；provider 执行证据、退款状态与 Core 回冲对账 |
| UI 与运行 | 七语言、桌面/窄屏、键盘、加载/错误/无权限、有限查询、会话恢复、模块排空/停机 | 固定静态产物及真实 Core/Go 的浏览器与进程证据；合成、sandbox、真实环境分列 |

新支付退款绑定原 Core intent 与 accepted proof；历史退款绑定 A03 离线导入的不可变付款来源，不能伪造新充值或重放旧回调补开账。两条来源共享 provider namespace/交易去重与退款上限。上述完整验收沿用 A05 的长期有效证明、乱序事件、真实进程重启和账本守恒要求。[S8]

## 论断与证据对应

| 论断 | 证据 | 状态 |
| --- | --- | --- |
| 旧钱包有五类 provider、七种预设入口 | S1 的模板、S2 的充值入口、S3 的注册路由 | 固定源码已核实；不表示当前运行通过 |
| Linux DO 是 EPay 协议下的 LDC，商店使用专门边界 | S2 的结算单位与 S4 的网关验证 | 固定源码已核实 |
| 订阅、钱包和商店的支持集合不同 | S3 的订阅路由、S4 的商店目录、S5 的退款能力表 | 固定源码已核实；缺口保留 |
| FastPay 不是需要恢复的可用网关 | 用户本轮明确排除；固定旧源码仅有 S9 的负向 fixture | 产品决定明确；历史拒绝证据保留 |
| 本轮充值候选不能宣告全部支付完成 | 上述业务矩阵与 S8 原始 A05/A06/S11 验收要求 | 联合验收及其余业务仍需分别提供证据 |

文档自查：范围是否具体——五 provider、七入口和四条业务路径已分开；表述是否可执行——各阶段列出动作与结果；结论是否有依据——旧行为绑定固定源码，候选不写成发布；验收是否完整——A05/A06/S11 原要求保留；设计是否一致——Core 独占身份与资金权威，Go 不直写核心库。未提供的运行证据留待实际验收，不从源码推断成功。

### 固定源码与合同索引

- **S1**：[钱包支付模板](https://github.com/TokenNotIncluded/lmm-api/blob/3948a0a1ee1cefdd016a7aa45a11851545bb4b85/apps/web/src/features/system-settings/integrations/payment-method-templates.ts)。
- **S2**：[钱包目录、EPay 及结算单位](https://github.com/TokenNotIncluded/lmm-api/blob/3948a0a1ee1cefdd016a7aa45a11851545bb4b85/apps/api-go/controller/topup.go)。
- **S3**：[充值、订阅、webhook 与配置路由](https://github.com/TokenNotIncluded/lmm-api/blob/3948a0a1ee1cefdd016a7aa45a11851545bb4b85/apps/api-go/router/api-router.go)。
- **S4**：[商店网关目录与 Linux DO 边界](https://github.com/TokenNotIncluded/lmm-api/blob/3948a0a1ee1cefdd016a7aa45a11851545bb4b85/apps/api-go/service/merchant_store_gateway.go)。
- **S5**：[商店退款能力](https://github.com/TokenNotIncluded/lmm-api/blob/3948a0a1ee1cefdd016a7aa45a11851545bb4b85/apps/api-go/service/merchant_store_refund_provider.go)及[实际退款执行查询](https://github.com/TokenNotIncluded/lmm-api/blob/3948a0a1ee1cefdd016a7aa45a11851545bb4b85/apps/api-go/service/merchant_store_refund_sync.go)。
- **S6**：[钱包/订阅/回调独立可用性条件](https://github.com/TokenNotIncluded/lmm-api/blob/3948a0a1ee1cefdd016a7aa45a11851545bb4b85/apps/api-go/controller/payment_webhook_availability.go)。
- **S7**：[Pancake SDK 适配与管理行为](https://github.com/TokenNotIncluded/lmm-api/blob/3948a0a1ee1cefdd016a7aa45a11851545bb4b85/apps/api-go/service/waffo_pancake.go)、[过期订单查询](https://github.com/TokenNotIncluded/lmm-api/blob/3948a0a1ee1cefdd016a7aa45a11851545bb4b85/apps/api-go/service/waffo_pancake_topup_expiry.go)、[customer portal](https://github.com/TokenNotIncluded/lmm-api/blob/3948a0a1ee1cefdd016a7aa45a11851545bb4b85/apps/api-go/controller/waffo_pancake_portal.go)。
- **S8**：[完整 A05/A06/S11 原始合同](COMPLETION_SCOPE.md)、[生产验收合同](PRODUCTION_ACCEPTANCE.md)。
- **S9**：[未知 provider 迁移拒绝 fixture](https://github.com/TokenNotIncluded/lmm-api/blob/3948a0a1ee1cefdd016a7aa45a11851545bb4b85/scripts/test-credit-balance-rebase.py)及[PostgreSQL 对应 fixture](https://github.com/TokenNotIncluded/lmm-api/blob/3948a0a1ee1cefdd016a7aa45a11851545bb4b85/scripts/test-credit-balance-rebase-postgres.py)。
- **S10**：[Stripe 收款/周期/钱包退款](https://github.com/TokenNotIncluded/lmm-api/blob/3948a0a1ee1cefdd016a7aa45a11851545bb4b85/apps/api-go/controller/topup_stripe.go)、[Creem 收款/周期](https://github.com/TokenNotIncluded/lmm-api/blob/3948a0a1ee1cefdd016a7aa45a11851545bb4b85/apps/api-go/controller/topup_creem.go)、[Creem 订阅商品核对](https://github.com/TokenNotIncluded/lmm-api/blob/3948a0a1ee1cefdd016a7aa45a11851545bb4b85/apps/api-go/controller/subscription_payment_creem.go)、[Waffo Global](https://github.com/TokenNotIncluded/lmm-api/blob/3948a0a1ee1cefdd016a7aa45a11851545bb4b85/apps/api-go/controller/topup_waffo.go)、[Waffo Pancake](https://github.com/TokenNotIncluded/lmm-api/blob/3948a0a1ee1cefdd016a7aa45a11851545bb4b85/apps/api-go/controller/topup_waffo_pancake.go)。
