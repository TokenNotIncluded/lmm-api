# 产品要求

本文件定义 LMM API 必须保留的行为、拒绝与恢复条件。具体接口和验收向量由各组件配方补充，完成证据按 [PRODUCTION_ACCEPTANCE.md](PRODUCTION_ACCEPTANCE.md) 核对。**唯一动态进度入口是源码仓库的 [docs/progress.md](https://github.com/TokenNotIncluded/lmm-api-source/blob/main/docs/progress.md)**；这里不保存任务状态、负责人分工或逐轮测试计数。

## 范围与来源

旧架构的可重现参考是 [go-v0.2.118](https://github.com/TokenNotIncluded/lmm-api/releases/tag/go-v0.2.118) 及其源码提交 `cc52aa195c6f2a01d703a1a407be73c31b89867d`；支付使用 [PAYMENT_PARITY.md](PAYMENT_PARITY.md) 单独固定的来源。按实际路由、页面、权限和资金行为对齐；路径迁移、能力暂时关闭或新目录缺失，都不能删除原需求。新增 CLI 目标与旧服务端兼容范围分别验收。

当前实现位于 `TokenNotIncluded/lmm-api-source`，精确版本读取父仓库 `git ls-tree HEAD generated/lmm-api-source`，嵌套 CLI/providers 再按该源码提交的 gitlink 固定。历史来源、当前实现、可安装制品和生产运行是不同证据。

## 架构与数据权威

- 仅有三个产品程序：Rust `lmm-core`、Go `lmm-extensions`、Rust 客户端 `lmm`。Web 是 React/TypeScript 静态产物；Core 保持单 Cargo 项目，扩展模块属于同一 Go 宿主，不新增部署守护进程或通用调度平台。
- Core 独占身份、权限、渠道模型秘密、价格、付款方、预算、账本和模型请求权威。Go 拥有独立模块数据库、必要外部 IO 凭据和窄 RPC 权限；不取得 Core DSN，不通过 root Key 代用户调用模型，不依据浏览器、模型输出或 host token 自行授予资金权限。
- 用户、操作人、Key owner、账户、团队、付款方和资源所有者分别建模。合同固定字段类型、单位、版本、scope、幂等键、UNKNOWN 与错误语义；provider/consumer 不用任意 JSON 透传绕过合同。Protobuf 既有字段号、enum 值及 wire 兼容由 contracts 配方保护。
- 运行配置为 TOML，默认 `/etc/lmm.conf.d/core.toml`、`/etc/lmm.conf.d/extensions.toml`；秘密为独立私有文件。HTTP/合同/证据 JSON 不属于运行配置。配置不执行 shell 或隐式环境展开；秘密、原始用户数据和敏感付款材料不进入参数、日志、Git、截图或公开输出。
- Core 在 Go 完全停止时仍能提供已授权身份与模型服务；已接纳流不依赖 Go 存活。事件持久投递，消费者按事件 ID 去重；能力、readiness、worker、准入和排空必须反映同一实际装配结果。

## 模型协议转换依赖

Core 使用 [zconv](https://crates.io/crates/zconv) 承担模型协议转换，依赖版本由源码 Cargo.lock 固定。小步接入须实际生成目标协议请求，并核对现有文本、图片、工具调用及提供商专属字段的行为；不接受仅增加依赖或调用身份转换作为完成。协议转换不取得凭据、价格或资金权限；准入、传输、用量证据及结算仍由 Core 负责。请求、完整响应和流式事件的接入范围及未完成部分分别记录并验收。 路由目标可显式限定客户端协议；未配置限制时保留既有行为。限制在选路及上游发送前生效，不能仅隐藏目录；同一渠道的其他目标不受牵连。首发暂缓的协议组合须明确列出，不能静默改模型或丢弃字段。

## 身份与权限

### 动作授权与特权激活

完整动作矩阵覆盖 root/admin/普通用户、显式 allow/deny、团队 owner/admin/member、developer 手动审批/付费激活、信任等级与折扣，以及渠道、采购、用户、团队、密钥、账本、订单、工具和财务读取。显式 deny 优先；每个真实操作检查 action/resource/tenant，拒绝未知动作、空资源、跨账户/团队、自报角色或付款方。菜单和 DTO 不授予权限。

旧特权账号逐个离线批准。材料绑定同一 source archive、原用户与身份事实、权限合同版本、策略摘要和目标实例；策略或来源不完整时保持停用。完整核验后在同一事务内最后激活，写审计、提升 auth_version、执行 CAS 并撤销原凭据。运行角色不能批准或导入；不能通过置 active、另建 root、关闭 MFA 或伪造 setup 成功绕开审批。

验证所有角色、资源和撤权组合；两个批准竞争只能发生一次；COMMIT 回执丢失按原批准 ID 恢复。冲突/拒绝后无关表和序列不变，旧会话与 Key 按冻结合同失效，管理入口不能扩大财务或目录可见性。

### 认证、会话与客户端

保留注册、bcrypt/Argon2 密码、邮箱绑定/验证/重置、验证码/Turnstile/邀请码/条款、登录/刷新/注销、会话列表与定向撤销、旧个人 token 轮换、个人/团队 API Key 创建/完整更新/显示/撤销，以及 Key 的组、IP、别名、预算限制。高权限动作要求真实当前 Session；API Key 和 host token 不能替代。密码重置撤销旧会话，旧 token 轮换要求 Session。

MFA 的密码通过阶段只发短期 challenge；绑定 purpose/user/auth_version/期限/尝试次数。TOTP 窗口、一次性 backup code、恢复、启停代际撤销及并发消费分别验证。Passkey 核对 RP ID、origin、challenge、签名计数、用户验证与会话绑定；过期、重放及错误用途拒绝。

OAuth 登录、OAuth2 授权服务器、OIDC 和远控客户端授权分别覆盖。保留 provider 登录与绑定/解绑、catalog/authorize/token/revoke、discovery/JWKS/userinfo、consent/scope、state/nonce/PKCE、精确 redirect 白名单、授权码单用及 refresh family 轮换/撤销。检查 issuer/audience、密钥轮换、账号链接冲突与接管、内网 discovery/SSRF；公开 client 不冒充 confidential client。单个 provider 通过不代表其他 provider 完成。

Cookie 按 SameSite/Secure/HttpOnly、CSRF、origin 和受信代理合同隔离，并有实际限流/审计。Key secret 仅存 hash 或已批准的可恢复密文，主密钥轮换/重加密保留版本且不泄露、不清库。邮箱出箱接纳、SMTP 接受与收件箱送达分别记录；邮件配置缺失、身份代际变化、过期验证码均不得放行。

### HumanGrant

HumanGrant 保留明确人工 consent、Ed25519 请求签名与 nonce、原 AI 单次 claim、持久授权/撤销、策略 revision 和原人类 Session 代际绑定。`account.profile.read` 仅允许读取批准者本人的数字 account ID；不得交换成人类 Session/API Key，也不授权模型、余额、团队资金或管理动作。

Web 保留 “Continue with human beings”、enabled/ready 显示规则、root 真实 Cookie 设置、独立审批页和本人撤销。purpose 作为不可信文本展示，权限默认不勾选，开关恢复不能复活旧 grant。真实 Cookie 登录至 consent/claim/profile/revoke、PG 并发/进程重启/撤销均须验证，测试登录 seam 不代替真实主机流程。

## 资金、订阅与历史迁移

账本使用受界整数，`credit_500k_usd` 的 500000 单位对应 1 USD；历史单位必须由冻结来源及转换证明确定。wallet、debt、affiliate、预留、收入和持续托管权利分别保留。金额未知不按零、当前汇率、当前目录或时间猜测；原始整数、unknown 原因与来源一并保留。

来源合同包括余额/欠款/返佣、已用计数、Key 剩余额度与历史使用、订阅额度/周期/无限/never、充值退款、bounty escrow、净付费/激活、原价格基准和 rebase。离线材料按只读 plan → mapping → 原 SQL → 完整 manifest 形成，绑定归档/观测点、完整 ID map、逐行摘要、目标 system-id/OID/boot/nonce、NOLOGIN owner、全部 schema/catalog 和空库/预置种子合同。

导入由独立 owner 在有明确授权的隔离目标内原子执行；运行服务和 worker 停止，无并发 client，runtime/Go 不能导入或重写来源。同 ID 同材料可精确重放，同 ID 异 mapping 或异 ID 冲突拒绝；目标事实 CAS、无关表/序列和账本守恒。COMMIT 结果未知先结束原连接、取得 advisory fence，再查原 ID；不得换 ID 或盲重试。后续导入阶段或应用写入开始后，不能重跑早期严格回放。

同一账本为模型、商店、支付及工具提供有来源、受权和幂等的预留/结算/退款、订阅消费、转账、兑换/礼品、affiliate 收益/追偿、bounty escrow/释放。Go 不选择任意账户执行通用 post-json；历史开账不伪造成模型请求、新充值或旧回调重放。Key hash 不能推导明文，无法等价恢复时执行明确轮换/撤销策略。

原生余额订阅、兑换、签到和红包按已确认整数 credits 工作；未知历史法币估值不禁用这些已有明确价格和授权的业务。订阅购买/续期/取消/未用退款及管理绑定、失效、删除、重置券/批量重置保留各自规则。价格、额度、回执不随 USD/CNY 展示偏好改写；外部支付仍须真实渠道与明确报价。

验证负余额和先偿债、affiliate 分离、整数溢出/舍入、订阅时区/DST/月末/never/无限、共用预算、历史 Key 与新预留、撤权期间合法退款、并发/重复导入、错误来源/映射/归属/角色/实例/catalog、部分 SQL 回滚及 ACK 丢失。实际应用余额、逐账户/币种/周期与总账对账同时成立；纯 RecordingLedger 或 plan 比较不能证明已迁移。

## 历史读取与财务导出

覆盖日志、本人/Key/管理员统计、看板、用户数据、flow 和财务导出，包括旧 `/api/log/`、`/api/log/self`、`/api/log/token`、`/api/log/stat`、`/api/log/self/stat`、`/api/data/`、`/api/data/users`、`/api/data/self`、`/api/data/flow`、`/api/data/flow/self`、`/api/finance/export` 的行为。每种 Session/API Key/管理员身份、筛选、排序、分页、时间区间和字段可见性必须明确。

分页、总数、聚合和导出使用同一鉴权、过滤与数据库快照；来源归属和历史/native 去重、退款/作废规则一致。只读不补写账本或重放奖励。保留原 quota，以及 `quota_usd/available/basis`、unpriced count、truncated；金额基准只接受来源证明的受界正整数 `pricing_credits_per_usd`。基准缺失、非法、越界或 bucket 缺组成项时明确 unknown，含未知项的总计不能冒充完整金额。

用户 normalized_used 使用独立 UsageProjector，不能与逐日志 captured-anchor 互换。opening、native 净使用和退款按同一来源核对；有界 decimal/整数累积，最后按指定精度取整。全站来源、Native 共存和导入期间未决意图按 Core 配方的封存合同处理，不凭 observed_at 判断新旧不重叠。

CSV/JSON/完整财务 ZIP 按字段白名单，禁止秘密、opaque metadata、未经授权个人信息和 provider 原始 payload；防公式注入、编码/NUL/换行问题。行数、anchor、metadata、输出总字节、并发和期限受限，ZIP 尾部计入上限，中途 IO 失败不返回完整成功。完整 ZIP 文档集合与其来源见 [Issue 行为合同](ISSUE_MAINTENANCE.md#pr875-r2同快照-15-文档-zip)。

真实 PG 验证跨用户/团队/Key scope、同快照并发、已知/未知混合、empty_result、多基准、负数退款、非法 anchor、排序、输出上限和读取后全状态不变。来源尚未证明时保留依赖阻断，不能用合成格式测试关闭真实历史验收。

## 支付与退款

[PAYMENT_PARITY.md](PAYMENT_PARITY.md) 是 provider 及旧行为来源的细化合同；钱包、订阅、商店、退款、管理分别实现和验收。

| 对象 | 保留范围 |
| --- | --- |
| EPay | 支付宝/微信 CNY、Linux DO LDC、自定义 type、钱包充值、一次性套餐购买；通用 EPay 的查询/退款按具体上游能力或人工处理 |
| Stripe | 钱包 Checkout/异步支付/过期，订阅首购/周期/失败/状态，钱包原 PaymentIntent 退款；订阅退款是独立产品目标 |
| Creem | 服务端固定 one-time 商品与金额；订阅商品/价格/币种/周期、首购/续费/生命周期；旧版没有自动退款不能描述为已有能力 |
| Waffo Global | 独立 SDK/凭据/环境/服务端子方式/企业账单、钱包支付及全额/部分退款；旧固定路由没有订阅 checkout |
| Waffo Pancake | 钱包、one-time/recurring 套餐映射、周期与生命周期、原交易退款、地区/语言/币种、商品管理、customer portal、超时上游查询 |
| Coinbase Business | 新增个人钱包：Core USD 报价与同额 USDC，独立 test/live、持久原 idempotency key、Core 验签及一次入账；默认关闭、自动退款关闭 |
| FastPay | 明确退役，不注册/启用/迁成可用渠道；历史标记和未知 provider 拒绝 fixtures 保留 |

五类旧 provider 下的七个钱包预设为 EPay 支付宝、EPay 微信、Linux DO Credit、Stripe、Creem、Waffo Global、Waffo Pancake。卡、Apple Pay、Google Pay 是 Global 的子方式。Linux DO 绑定 `https://credit.linux.do/epay`，LDC 与现金分列；普通 EPay 不继承其网关、退款或单位合同。

渠道独立配置启用条件、秘密/环境、商品/支付方式和单位；秘密只写，公开目录仅返回本人可用的非敏感字段。受众、注册等待、限额和权限在服务端执行；钱包与订阅独立判断。Pancake catalog/pair/save、subscription-product/product-options 和 Session-only portal 绑定服务端 customer/store/environment，登录证明不落日志/缓存。关闭新下单后，旧单按原配置继续查询、回调和退款；本地 checkout 超时不能直接关单，须查询上游并保留 pending/未知/已付义务。

入账链为原始回调 → Core 独立验签与 intent/商户/环境/订单/交易/币种/金额/产品/时间窗绑定 → 持久 accepted proof → Go 持久队列 → Core 幂等账本。可靠 ACK 只能在 Core 接纳后发送；未接纳的陈旧事件仍拒绝，已接纳证明不退化成短时签名缓存。EPay 共用签名密钥只留 Core，Go 只能取得已存在 intent 的 GET/POST checkout，不能任意代签；Global 除签名外还需 Core 持私有凭据对原交易独立 inquiry。

Web 先读服务端渠道和精确十进制整数报价，再明确确认。原用户/会话、request key、quote、order、环境和金额固定；GET/POST handoff 保留原字段与签名，401、刷新、丢响应和 UNKNOWN 不自动重放金融 POST。状态仅有限 GET 查询；provider_paid 与 core_credit_confirmed 分开，只有账本确认表示到账。

新退款绑定 Core intent/accepted proof；历史退款绑定离线导入的不可变 LegacyPaymentBasis、独立验签、provider namespace/transaction 和累计额度。两个来源共享交易去重，不能用 opening/new intent 伪造旧付款。dispatch/结果 UNKNOWN 持久保存，外部 IO 不在金融事务中；只有充分绑定的失败证明才释放 hold。

覆盖重复/乱序、同事件不同 payload、先退款后入账、部分退款累计与取整、资金不足转债、零 credits 差额留有效 receipt 而不造零 journal、取消/超时/重放以及消费退款竞争。故障注入包含接纳后 ACK 前、Go 队列后、Core COMMIT 后和退款 dispatch 后响应前；真实停止并重启 Core/Go OS 进程，按原 key 核唯一入账/退款、账本、hold 和外部调用计数。保留至少 302 秒跨签名窗口的已接纳证明恢复验证。host token、跨用户查询、篡改归属/环境/币种/金额不得取得资金权威。

## 商店、客服与商业扩展

商店按独立矩阵支持 `balance`、`external:epay`、`external:waffo_pancake`、`platform:waffo_pancake`、`platform:linuxdo`。冻结买卖双方、actor/payer、quantity、单价单位、报价版本、command hash；USD 整数分与 credits 明确互斥。外部钱包收款不能自动授权商品交付。

保留商品/类别/集合/购物车/收藏/点赞、public/registered/private 可见性、TestMode/草稿预览、访客购买/邮件提货、购买限制/条款、优惠、库存/密封交付/退货、全额/按数量/金额退款。提货链接/码、付款凭证和退款后领取权限分别验证。Extore 的商户密钥、授权、连接/撤销、目录/多规格/版本/文件同步、只读编辑和绑定按原版本及可验证 receipt 恢复，不凭 HTTP 200 认定交付。

Go 状态机与 Core reserve/capture/refund/reconcile 通过窄 RPC 衔接；CompleteDelivery 只消费原订单已批准 capture grant。外部交付不占账本事务；UNKNOWN 保留原订单/键并查原命令，不自动退款、补扣或重建订单。余额、商户 EPay 人工退款、Pancake ticket/执行查询/签名通知、Linux DO 全额退款按各自合同处理；受理不等于实际资金完成。

客服用户/管理者/历史消息按所有者和参与者隔离；平台人工支持与商店客服各自授权。保留运营、推广/分享/广告/推荐/游戏/RSS、HeroSMS 邮箱/SMS、远端模型部署管理、平台充值折扣、内容审核、通知/签名 webhook、汇率，以及旧版实际存在的相关页面/API。按固定旧源码确认各行为，不自造旧版没有的要求；新增目标明确标明来源。

真实 Core+Go+独立 PG 验证付费购买、库存提交失败、退款/结算竞争、重复/乱序交付、撤权、买家停用后的合法退款、卖家先偿债、累计退款/数量上限和原价/收款人/payer/hash 篡改。条款缺失/过期拒绝后订单、预留和库存均不变；访客和关闭 capability、跨客服参与者、模块 PG 持久化与 OS 重启分别验证。免费订单、内存 funds 或直接改 balance 不抵扣付费验收。

## 模型、渠道与计费

逐 provider × 入口 × 格式覆盖 Chat、Responses、Messages、Gemini、模型列表、Embeddings/Rerank、独立音频/翻译、图像、聊天多模态、Realtime/WebSocket、异步媒体/任务/批处理，以及原有渠道和模型管理。格式不支持时明确拒绝，不能静默丢字段；独立图像生成不抵扣视觉聊天。搜索/grounding、原生供应商、12 小时缓存、价格导入和异步图像任务的细节继续见 [Issue 行为合同](ISSUE_MAINTENANCE.md)。

服务端固定渠道/endpoint 白名单、秘密引用、别名、组、优先级/权重、首选/排除、健康重试、attempt 计费归属、受信代理和 Key/IP/alias 限制。客户端不能选择任意上游、价格或付款方；防 redirect、DNS 重绑定、私网/metadata SSRF 和秘密泄露。渠道 tag/multi-key/原生 provider 操作及余额/真实推理探测属于各自业务，不用目录探测耗时冒充推理延迟。

计价覆盖输入/输出、缓存读写、按次及其他经确认维度；用精确整数/decimal，报价版本与汇率依据只捕获一次，重试不改价。上下文与请求预留上界必须有验证过的外部模型合同，包含消息、工具定义和其他输入；合成 UTF-8 字节上界不自动成为真实模型保证。异步结果与资金原操作持久关联。

### 音频用量合同

音频分别计文本输入、文本输出、音频输入、音频输出。旧倍率分支为 `(文本输入 + 文本输出×补全倍率 + 音频输入×音频倍率 + 音频输出×音频倍率×音频补全倍率)×模型倍率×分组倍率`，并保留固定价格、阶梯、舍入与上限；音频 token 不按文本价格静默结算。

OpenAiChatV1 仍要求显式 text/audio，旧收据按原冻结版本恢复。显式配置的 OpenAiChatV2 将请求投影限于已审阅 text/input_audio 输入与 text/audio 输出，排除 image、工具及其他模态；格式和 provider_contract_revision 属于冻结 AudioRule。V2 仍要求原始 prompt/completion/total 及两侧显式 audio_tokens，只在该请求与 provider 合同内用总量减 audio 推导缺失 text_tokens，显式 text 必须相等。

缺 image 只在上述冻结边界中表示不适用；非零 image、未知维度、负数、浮点、null、重复权威键、不一致总数或溢出均不结算。cache_read/cache_write、reasoning、prediction 等子集不重复扣除；该适配不新增缓存差价或改变既有精确价格。

合法 service_tier、默认 obfuscation 等 envelope 在内存校验后丢弃，不通过关闭默认字段绕过兼容。新不可变财务证据只留计数型 usage、实际帧 SHA/序号、必要语义/哈希链和响应 SHA；不落盘 envelope 或文本/音频 delta。可信 worker 在处理实际帧时建立来源关系，散列不冒充原字节逆向证明；旧 v3 envelope journal 兼容只读，不改写。

终值仍须有界读到 HTTP body EOF；usage/DONE 后尾部字节、重复终值、截断、超时或传输错误保持 hold。DONE/EOF、receipt 与规范化 usage 同事务持久化；恢复用原格式、价格和 dispatch。分别验证非流/流式正常官方字段、跨 chunk 等价、非法反例、事务故障及原价恢复；独立 STT/TTS/translation/realtime 继续单列，合成测试不宣称 provider 认证。

### 流式、授权与恢复

先持久化输出 checkpoint 再发送，保留背压、慢客户端、取消、tool call、usage 尾帧、跨块 UTF-8/JSON、压缩/超大帧的边界。缺可信终态用量或 COMMIT 不明时保留 hold；错误分类按 contracts，已发送 HTTP 状态不伪装可重写。

委托模型请求绑定原 Session、conversation/owner/tenant、可信付款/赞助政策、模型/工具/预算/expiry、用途和幂等键，支持撤销与团队隔离。Session 委托不变成 API Key 绕行，Go 不自选平台付款。

真实 Core OS 故障覆盖 dispatch 前后、partial 输出、终态持久后 finalize 前及 COMMIT ACK 丢失；逐 attempt 记录外发次数，恢复不得再调用不确定操作、重复结算或丢 checkpoint。验证各格式 golden/差分、真实 PG 预算、取消/重连、group/IP/alias、同 key 异请求、个人/团队/平台资金隔离；停止 Go 后已接纳模型流继续按原合同处理。

## 助手、工具市场与远控

普通对话、平台客服/赞助、预设复制、历史/归档、系统 prompt、工具范围、附件、setup guide；市场发布版本、发现/价格、用户授权、预算/次数/期限、OAuth 绑定、MCP 连接、执行/退款及持久 call-status 均须有真实入口。价格/版本变化不能沿用不相容的旧同意。

市场工具与商店商品的 AI 审核各有独立 `off`、`assist`、`auto` 模式，默认 off。assist 只留下私密建议并等待人工；auto 只在可信分类成功、当前仍为 auto 且提交版本匹配时决定。旧 assist 作业不能因开关变更自动批准。路由/供应商故障、超限、私有工具或技术校验失败均保留人工审核；工具还要求新鲜的远程验证、`ValidationDigest == Digest` 与计量检查。市场审核不继承聊天处罚、钱包扣费或账号制裁。

审核只提交脱敏的公开标题/说明/工具描述/链接标题，不发送链接目标、图片、schema、凭据、网关、库存密钥、价格订单或个人信息；超长输入明确失败，不能截断后批准。提交与入队同事务，每次重新提交生成新 token；作业绑定版本、文本摘要、路由与模式，完成时锁定并复核启用 owner、token、pending 状态和当前设置。管理员可审核自己的提交并覆盖当前自动决定；人工决定或编辑使旧 token 失效，迟到/重复结果不能覆盖。当前 AI 批准/拒绝在私有队列保持可达，pending 优先；人工覆盖或新草稿移出旧决定。结果区分 stale/overridden；临时供应商故障最多尝试三次，之后持久保留 failed/manual_required。终态清除文本，只暴露稳定错误码，不保存供应商响应正文。

设置读取限管理员，写入限超级管理员，两个模式字段均必填，未知字段、尾随 JSON 和非法模式拒绝；启用需实际审核路由，修改不改变聊天罚款策略。历史仅当前启用的 owner 或管理员可读，按 job ID 倒序最多 20 条，工具默认当前草稿/线上版本，显式版本必须属于原服务；缺分类为 null，公开目录不含审核元数据。市场任务使用独立完成事务，旧聊天完成入口拒绝市场来源，聊天审核列表和统计不混入市场任务。审核不证明商品质量、库存合法或链接安全。升级先保证全部 worker 支持市场来源再显式启用；回退先关两模式并排空市场作业，未知 worker 不得吞掉未决审核。详细来源见 [冻结的旧审核合同](https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/docs/market-ai-review.md)。

`assistant.agent_required` 是 AgentChat 唯一显式启用项。省略/false 不启用 AgentChat/agent journal；若独立 assistant 凭据及 PersonalText 就绪，可运行 PersonalChat，仅选择 delegation journal bundle。true 必须选择 delegation+agent bundle，核验独立凭据、AgentTurn、schema 与最小权限；任一缺失启动失败，不回退 PersonalChat。启动无 DDL/授权，运行中不自动切模式，capability/chat/journal 权限消费同一启动结果。

调用绑定 conversation owner/tenant、actor、payer/sponsor 政策、tool/price version、预算、expiry、nonce、operation key。metadata 发现与有副作用 invoke 分离；先持久 operation 再请求外部，超时/ACK 丢失按原 call-id 查证，不盲重试购买、发信、转账。退款有原 call/失败证据且只能一次。

MCP HTTP/SSE/Streamable、OAuth 及远控保持固定协议。限制 redirect/DNS/内网、响应大小/期限、工具 schema 与结果秘密；网页/仓库/工具输出是不可信数据，不升级为服务指令。保留来源归属和许可。Pi/远控持久记录设备、会话、所有者、授权、恢复与断线撤销，不形成无 scope 任意命令执行器。

验证真实 GoPG 历史 + CorePG 委托模型 + 计数 MCP 的联合对话，真实 Go OS 重启后状态恢复；跨用户/会话/租户、伪造 payer/sponsor、撤权、版本/价格篡改、重复 execute、UNKNOWN、并发预算耗尽、退款重复和恶意输出。缺桥接/权限/账本时明确拒绝且无扣款；MemoryStore 或 controlledFunds 不能代替最终证据。

## 安装、升级与恢复

安装 owner 与网络 runtime 分离，owner 使用 NOLOGIN 及受控离线通道；普通启动/check-db 只校验完整 schema、catalog、角色/ACL，不 DDL、建角色或自授权。新安装、明确前驱的升级、离线迁移分别验收；旧组件指纹及已冻结升级终点不可随新二进制改写。

维护状态机覆盖 schema-only 操作、所有 writer fence、旧后台/回调/请求/退款 drain、加密完整备份、可恢复密钥、独立 restore、失败处理与死 holder 的一次 CAS 接管。接管须证明原 holder 已死，不能凭超时推定；回退二进制不代表新账本可逆。独立升级历史存储不随目标物理备份回退，原请求及单向状态不因租约、重连或目标恢复清空，查询不给 retry/open 权限。

备份位于明确指定的私有目录，目录 0700、文件 0600；脱敏 metadata 与内容 hash 绑定来源。解密恢复到新独立库，逐表、行、多重性、序列、合同、角色/ACL 比对，再检查恢复后真实业务。backup 成功不代表 restore 成功，本地演练不等于生产切换。

pgroll 单独证明 Core 限定 schema、函数/触发器/SECURITY DEFINER、catalog 指纹、角色/search_path 的兼容；在一次性合成库验证旧新版本视图读写/权限、rollback/complete 与数据守恒。未证明就保留实验，不能接普通启动；其 status 也可能改元数据，不能拿受保护源库试运行。

Docker 冷构建和实际启动检查无 Go 的 Core、模块 DB/UID/secret/RPC、容器越权拒绝、重启和资源。原生 deb/rpm/apk 等实际 encoder/隔离安装验证路径/TOML、config noreplace、sysusers/OpenRC；安装不自动生成秘密、初始化 DB、启用/重启服务，升级/卸载保留配置和数据。依赖 checksum/sumdb 必须核官方来源并解决冲突，不能关闭校验。

测试资源独立记录 nonce/system-id/boot/datadir/socket/unit/PID；不同目标/角色/凭据隔离，重型构建有明确资源归属，冻结测试副本与 target 分离。仅清理本轮拥有且无其他 client/用途的 fixture，保留既有服务、源归档、失败证据和并行改动。

## Web 与 CLI

Web 验收清单按页面 × API × 角色 × 数据来源 × 失败/恢复生成，覆盖全部真实路由和动态子页：公开页/定价/法律、注册/协议/认证、管理与团队、Key/会话、模型/历史费用、钱包/订阅/转账/礼品、充值/退款、商店/客服、助手/工具及运营。能力关闭显示明确不可用；unknown 不显示零，401 不当空数据，隐藏菜单不抵扣后端。

界面简约、留白、层级和金额单位清楚；保留简体中文、繁体中文、英语、法语、俄语、日语、越南语。桌面与窄屏/键盘/可见焦点、加载/空/失败/无权限、明暗主题及减少动效分别验证；XSS、URL Key、CSRF、cookie/redirect、401/403/503/UNKNOWN 和账号切换不得泄露或跨会话重派资金操作。截图脱敏，完整 typecheck、范围 lint/format、组件、build/bundle 和真实浏览器结果分别记录。

真实浏览器使用固定静态产物、真实网络与持久后端，核实际前后端版本、角色、请求/结果、Console/Network、刷新及进程重启后读回；拦截整个 API 的 fixture 不是端到端。外部身份、支付、邮箱等依赖的合成/sandbox/真实结果分列。

CLI 为独立 Rust 子项目，保持 `lmm` 名称，XDG config/state/cache、TOML、秘密权限、server selection、认证/Session/OAuth/Key、模型流/取消/错误、工具/客户端操作、help、JSON/非交互和系统凭据库。具体命令与阻塞行为见 cli 配方，不为填表虚构管理/部署引擎。Linux/macOS/Windows 及 amd64/arm64 各自验收；非法 XDG/权限、shell 展开、秘密输出、stdout/stderr/退出码、真实 TCP 取消/auth 过期均核对。

与 [Magpie](https://usemagpie.ai/) 对齐属于独立新增目标：先核产品、真实来源和 LICENSE，再按目标软件/版本验证可恢复接入、配置预览/确认、失败/更新/撤销。只复用明确许可的源码并保留 attribution，无公开源码不能声称复用；客户端不能自行裁定扣款或复制其他应用凭据。CLI 子仓库提交须可达，父 gitlink、测试与制品绑定同一提交。

## 最终验证与资源门槛

各领域共享固定源码、依赖/工具链、接口版本、schema、配置、来源/目标和制品清单；孤立 PASS 不能拼成最终组合 PASS。Core 全 all-targets/真实 PG、clippy/fmt；Go readonly modules/race/vet/fmt/build 与模块 pgtest；完整 Web suite/typecheck/相关 lint/format/build/bundle；CLI 固定提交 fmt/clippy/test；协议、边界、工作流、分发、路径、文档及法律配置门槛按实际仓库命令执行。保留唯一测试数、失败、skip 与真实子进程版本，工具与依赖遵守锁定配置。

二进制复制前后与原构建产物 SHA 一致；Web 保存实际完整文件清单。源码、编译参数、版本、平台和运行 PID/exe/config 身份可追溯；可变 target/debug 不作为最终身份。源码/工具/依赖/目标变化时按影响重验；未提交组合或旧提交的通过记录不自动证明新候选。第三方许可证与归属完整保留。

资源验收固定真实 cgroup 硬配额：单 Core 1 CPU/256 MiB，Core+Go 共享 1 CPU/512 MiB，数据库/负载器/上游预算独立说明；整机 1C1G 另验。完整 HTTP/SSE 场景为 60 秒预热、300 秒稳态、60 秒过载、60 秒恢复、60 秒排空；正常 5 RPS、80% JSON/20% SSE，完整成功率 ≥99%、P95≤1 秒、P99≤2 秒。检查慢消费者、取消、过载、重启、排空、RSS/PSS、峰值、FD/连接及账本；不能靠全部拒绝、缩功能、提前关客户端或提高阈值通过。

新旧性能比较使用同 CPU/内存、数据规模、功能、计费语义、预热和负载的优化构建，保存成功/拒绝/超时分别计数、吞吐、分位延迟、内存增长和故障恢复。未测量不写提速/内存比例；产品 cgroup 不代替整机预算，合成上游不代替真实 provider。

真实来源迁移、最终主机安装/切流、writer fence 与 active writers/unknown liabilities、实际备份恢复、同一候选全站业务及恢复证据按 [生产验收合同](PRODUCTION_ACCEPTANCE.md) 逐项关闭。必要项缺证据、未执行、输入漂移或资金/权限问题时保留明确未完成；不删功能、断言、失败记录或手填回执来消除差距。

## 历史保全

旧任务提示词、当时的工作区/分工与逐轮结果保留在 [固定原文快照](https://github.com/TokenNotIncluded/lmm-api/blob/8f11ef82ac3a20e33406f2cc7478b419e74bfbe6/recipes/lmm-api/COMPLETION_SCOPE.md)。本文件按领域重组其有效要求，并继续纳入 [PAYMENT_PARITY.md](PAYMENT_PARITY.md)、[ISSUE_MAINTENANCE.md](ISSUE_MAINTENANCE.md) 和组件配方的细化合同；历史提示词不提供当前操作授权。
