# Issue 修复与旧 PR 迁移合同

本文件把问题单和旧候选中仍需实现的行为纳入可重建需求，补充 COMPLETION_SCOPE.md 的 A04、A05、A07、A09、A10 与 S11。生产、权限、资金和证据门槛继续以 PRODUCTION_ACCEPTANCE.md 及 contracts 配方为准。总跟踪入口为 [#895](https://github.com/TokenNotIncluded/lmm-api/issues/895)；下列合同编号可直接用于其待办与后续源码 PR。

配方在 `TokenNotIncluded/lmm-api/main`，实现位于 `TokenNotIncluded/lmm-api-source/main`，由 `generated/lmm-api-source/` 固定提交接入。旧 PR 的 `services/`、`apps/` 路径属于当时的源码布局；后续实现进入源码仓库，再更新配方和 gitlink。

## 维护闭环

1. 阅读 issue 原文、评论、相关 PR 和当前实际源码，把用户可观察的错误及其验收条件固定下来。旧路径消失、实现换语言、默认分支或仓库变化，均不能代替行为验收。
2. 以最小完整行为修复问题，同时在对应配方保留输入、输出、权限、单位、失败和恢复条件。复用当前实现，保留其他任务的改动；历史候选按功能提取，不能整文件覆盖后来加入的授权、选路或资金保护。
3. 在冻结输入上执行能证明该问题已解决的定向检查，记录命令、退出码、实际源码版本及真实限制。已经通过且输入未变的检查可以复用；只对剩余具体风险补验。代码、工具链、依赖或目标合同变化时重新判断证据适用范围。
4. 修复完成的 issue 必须链接实际源码提交和对应验收。配方更新、文档修订、专项单测和完整产品验收分别登记；遇到缺失环境或未执行的业务链路，保留 BLOCKED/NOT RUN。
5. 旧 PR 可以在完整承接残留需求后以 `superseded / migrated` 关闭：保留原正文、候选 SHA、失败证据及已抽取范围，指向本文件的合同编号和 #895。这种关闭代表工作位置迁移；只有相关行为在当前输入上通过验收，才把对应合同标为完成。不得因关闭 PR 顺带关闭它关联的尚未完成 issue。
6. 每次完成当前源码 PR 后，核实远端提交，显式更新父项目 gitlink，再核对最终配方引用。构建、合并、发布、部署与真实外部服务验收各自记录。

## 旧 PR 的承接范围

| 原候选 | 保留的既有成果 | 仍需完成的合同 |
| --- | --- | --- |
| [#879](https://github.com/TokenNotIncluded/lmm-api/pull/879)，`cb3276d423bf2d81d4451180dd1e3ff18c1dd6df` | 已接纳的 inbox、渠道、收据和结算恢复；当前源码已有宿主接线，实际 Core/Go 重启的两项恢复历史验收及真实 Core 授权的订单状态读取验收 | PR879-R1 至 R3：支付 readiness、已验证恢复行为的持续保留、其余退款/供应商/资源验收 |
| [#875](https://github.com/TokenNotIncluded/lmm-api/pull/875)，`0d44fb95c1bb7d11f9e8bc74e963b936f7a0542e` | 当前按归属校验的历史用量读取、有限 JSON/CSV 导出、未知金额与 CSV 防注入保护 | PR875-R1 至 R3：全来源合同、同快照完整财务 ZIP 与真实 HTTP/权限/单位验收 |
| [#942](https://github.com/TokenNotIncluded/lmm-api/pull/942)，`a7c122ffbade62d7cba38569eca3cf273f0b3a8f` | 已抽取的 12 小时缓存用量、精确价格、结算恢复和展示基础；当前主线的 selected-route、资金预留与来源约束 | PR942-R1 至 R7：搜索与原生供应商、导出/看板、通知、价格管理、schema 升级及未完成候选 |

这是需求映射，不能从“保留的既有成果”推断相应领域已整体验收。原 #875 的专项运行曾修正临时锁文件和格式；原 #942 的测试编译、源码边界以及 schema 证据阻断仍属于原输入，历史专项结果不能覆盖当前组合验证。

## PR879：支付恢复与宿主生命周期

### PR879-R1：支付能力的真实就绪状态

把支付可用性接入现有宿主生命周期。支付就绪必须同时证明：支付独立数据库的运行角色具有所需权限、实际 Core 支付能力及窄 RPC 凭据可用、恢复 worker 已装配且至少成功完成一次有界批次。普通宿主 HTTP 健康不能单独证明支付可用。

就绪结果有明确采样时间和有效期。探针或批次失败、父任务取消、worker 停止、结果过期或进入 drain 时立即撤销支付就绪；恢复成功后才重新开放。批次串行、有截止时间及数量上限，HTTP 准入和回调入口有明确并发上限。请求或批次超时但尚未退出时继续持有其名额，不能由计时器提前释放而积累失控任务。

验收覆盖首次未成功、依赖失效后恢复、重复启动、并发饱和、取消但仍在执行的 handler、worker 异常及 drain 竞争。拒绝新工作后等待已接纳请求和批次实际退出，再关闭数据库和 Core 客户端；等待超时保留恢复状态。公开/内部 capability 与 Web 显示消费同一事实，停用支付不能影响 Core 独立模型服务。复用当前 composition 与 host 准入，不并存两套无人调用的支付 Runtime。

### PR879-R2：真实 Rust/Go 双进程故障恢复

保留已经通过的真实进程回归。[当前固定源码的支付验收说明](https://github.com/TokenNotIncluded/lmm-api-source/blob/86e51b0708086fb78605ef9f519275149ac47873/tests/payments/README.md) 和 [质量指南](https://github.com/TokenNotIncluded/lmm-api-source/blob/86e51b0708086fb78605ef9f519275149ac47873/docs/quality.md) 已记录两项独立通过：Core 入账 COMMIT 后、Go 本地收据持久化前双产品重启恢复；accepted-proof 实际等待至少 302 秒过期后双产品重启，由真实 Go worker 使用 Core 持久证明完成首次入账。真实 Core 授权的 PREPARED 订单状态读取也有独立验收。上述历史证据按原冻结输入和模拟供应商边界保留，不重新描述成“从未验证 Rust/Go 恢复”，也不转作本轮新运行结果。

重建时在隔离 PostgreSQL 中运行实际 Core 与 Go 宿主，分别冻结二进制、配置、数据库角色和进程身份；供应商可以先使用明确的合成端点。保留已通过场景，补齐尚未覆盖的 inbox/job 交接、精确 RPC 网络回执丢失、退款 hold、provider dispatch 或终态记录结果不明等故障点。原快速用例证明的是 COMMIT 到 Go 本地记录之间中断，不能扩写为已证明精确丢失 Core 网络回包或完整 checkout/refund。

每个场景读取同一原操作的 proof、订单、inbox/job、租约、dispatch、journal/entry、hold 和预算；重新启动及重复回调后只能形成唯一资金终态。缺失 Go adapter 不得阻止 Core 已接纳 proof 和既有收据的恢复；新的供应商 IO 仍要求原渠道、商户和环境配置。未接纳的陈旧事件不可在恢复时被当成已验签事实。COMMIT 结果不明先按原合同 fence，再查询原 operation；UNKNOWN 不换幂等键、不再收费、不提前释放 hold。

旧 Go+PG 子进程测试中的 Core 替身保留为包级回归；后续真实进程 runner 使用实际 Core RPC、Rust 验签与账本读回。完整回调入口、checkout/refund/hold release、浏览器与边缘代理流程及支付资源验收仍需按各自范围完成。真实供应商验收按 PRODUCTION_ACCEPTANCE.md 单独授权并记录，不从合成成功推定 Stripe/ePay 已兼容。

### PR879-R3：供应商与历史退款完整性

延续 A05 的 provider/支付/订阅矩阵，逐项标明合成、sandbox、真实、未实现及所需验收。新支付退款引用原 Core intent 与 accepted proof；历史退款引用 A03 的不可变 `LegacyPaymentBasis`，绑定原 provider namespace、transaction、商户、环境、币种、归属及累计整数额度。Opening balance 不能充当原充值，新的 native intent 不能替代历史支付依据。

覆盖部分退款累计、重复/乱序事件、扣款与退款竞争、签名错误、金额/币种冲突及渠道删除。只有可靠绑定的 dispatch/结果证据才允许释放退款 hold。缺少历史来源或尚未实现的供应商验签保持明确不可用，相关金额继续可追踪。

## PR875：完整财务导出

### PR875-R1：来源、身份与金额合同

完整导出需要 A03 提供真实只读来源、稳定身份/账户/Key 映射、历史/native 去重及退款/作废依据。旧候选使用 `core_history.source_contract_v1`、`log_records_v1`、`quota_data_v1`、`finance_records_v1`、`finance_options_v1` 五个视图；重建时可用当前已审核的等价接口，但必须明确字段、类型、版本、归属和每类来源的唯一完整回执，不能把合成 fixture 当生产 DDL 或迁移证明。

未证明映射与权威证明为空是两种结果。缺失、重复、矛盾或不可验证的来源回执应拒绝完整导出；运行角色对来源整表、单列以及回执有写权限时也拒绝。保留原始 quota，历史美元金额只采用来源冻结的单位与计价基准；未知金额、截断结果、退款和多基准不能由当前汇率或当前价格倒算，也不能把部分小计说成完整总计。

### PR875-R2：同快照 15 文档 ZIP

管理员完整财务导出在一次授权和一个只读数据库快照中生成下列 JSON 文档，保留用户/session/API Key 各自的只读范围，普通用户无权取得全站财务 ZIP：

| 文件 | 内容 |
| --- | --- |
| `users-balances.json` | 用户余额、归属与独立的累计用量投影 |
| `channels-pricing.json` | 渠道价格相关配置，剔除秘密与 URL 凭据 |
| `subscription-plans.json` | 订阅计划和明确的额度/币种/周期 |
| `topups.json` | 充值及原始结算金额、币种与状态 |
| `subscription-orders.json` | 订阅订单、供应商状态与累计退款 |
| `subscription-payment-events.json` | 订阅付款事件和原周期 |
| `bounty-ledger.json` | 悬赏账目、关联对象与金额 |
| `checkins.json` | 签到奖励及来源 |
| `redemptions.json` | 兑换记录与状态，保留可见字段边界 |
| `user-subscriptions.json` | 用户订阅、额度使用与重置周期 |
| `usage-billing-records.json` | 授权范围内的消费与历史金额依据 |
| `financial-options.json` | 允许导出的财务选项 |
| `model-prices-and-ratios.json` | 财务选项派生的明确模型价格/倍率 |
| `effective-model-pricing.json` | 单独来源证明的有效模型价格 |
| `manifest.json` | 版本、快照时间、筛选范围、各文档行数、脱敏与截断说明 |

所有记录采用字段白名单，秘密、原始请求、任意嵌套 provider payload、opaque log metadata 和未经授权个人信息不进入归档。敏感 URL 仅允许已审核的来源标识，不暴露 userinfo、路径、query 或 fragment。来源缺失不可生成假空数组。当前较窄导出继续准确报告 `complete_financial_export=false` 和实际 `included_families`；完整合同全部满足后才可声明完整。

### PR875-R3：有界资源、错误与集成验收

行数、时间范围、每条 metadata/anchor、总输出字节、并发与执行期限均有上限；ZIP 目录和尾部字节也计算在总上限内。分页、统计和每份文档读取同一快照及相同过滤条件。中途序列化、IO 或 ZIP 尾部失败不能返回貌似完整的成功归档；资源限额错误保留限额类别，与来源不可用、格式错误区分。

验收包括真实 PG 的来源缺失/冲突、列级写权限拒绝、并发修改时快照一致、跨账户/团队/Key 越权、native/history 去重、金额单位对账、空/未知/混合基准、只差一个尾部字节的限额和中途 IO 失败。保留 JSON 对 NUL 的合法转义与 CSV 对不安全内容的拒绝；CSV 需处理 BOM、Unicode 空白/控制前缀、公式、编码和换行。实际 HTTP 路由与 Web 下载在固定后端版本下验证，不能由格式函数通过替代。A03 真实来源未交付时，完整历史导出保留依赖阻断。

## PR942：上游计费与管理能力拆分

各项实现仍由 Core 裁定身份、价格、付款方、路由、账本和渠道秘密；Web 只显示真实能力并提交可审阅操作。保持当前已接纳的来源、selected-route、凭据版本和资金预留保护。以下功能可以分别提交源码 PR，但每个提交都必须形成可测试的完整接口/状态/消费链路。

### PR942-R1：实际用量与精确计费

保留已抽取的独立 12 小时 cache-write 维度：完成响应中的实际用量、冻结的有理数单价、缓存与正文不重复计数、结算/恢复、价格及日志展示必须一致。明确零、缺失、未知及与其他缓存量矛盾的情况，不得因旧版本缺少字段便按零结算。

延续 [#822](https://github.com/TokenNotIncluded/lmm-api/issues/822) 的实际搜索计费：逐 provider/protocol 固定搜索编码、单位、成功终态证据与价格来源，覆盖候选声明的 OpenAI/Azure/xAI/DashScope Responses、Gemini grounding、Anthropic/DashScope Messages 和 Zhipu。Gemini 的查询次数与“请求使用过搜索”是独立计价维度；不能相互替代或从模型名字估算。缺失、冲突或非最终 usage 保留 pending/UNKNOWN。端到端验证实际请求、终态 evidence、唯一结算、重放、取消/断流及 UI 费用；不因供应商不收费的某种模式推广为全部免费。

### PR942-R2：原生供应商操作

DeepSeek search/fetch 及 reasoning/request 参数按其原生合同接入，保留已付费、明确零和未知结果；Ali multimodal embeddings 保留媒体输入、显式 dimensions、text/image 的独立单位与精确价格。验证真实路由选择、原始参数、响应形状、非法维度、错误与 usage 缺失，只有可信完成证据才能结算。

[#922](https://github.com/TokenNotIncluded/lmm-api/issues/922) 的 AWS Mantle Messages 使用审阅并锁定的官方 SigV4 signer，绑定实际 region/host/path、private AWS JSON credential 及临时 session token。不同 credential reference 不得把 AWS 文档或其中 secret 解释为普通模型 API key，签名失败不能自动降级为 API-key 认证。覆盖 canonical request、凭据格式/过期、配置冲突、顶层 cache-control、流式结果及计费。源码候选、离线签名测试和真实供应商接受分别登记。

### PR942-R3：用量导出和缓存看板

按 [#900](https://github.com/TokenNotIncluded/lmm-api/issues/900) 完成 native self/admin CSV 导出：同一过滤、时间范围、语言列头、scope 与分页，用户只能导出本人，管理员需独立授权。行数、总字节、耗时及每用户/跨节点并发有界，取消和中断可靠释放下载名额。明确没有共享协调服务时的实际并发范围，不能把进程内限制称为集群限制。

缓存看板必须在完整 API/UI 链路保留未知与零的区别，区分 lifetime 总数和已包含的分项，避免把子集再次加入总数。验证空、未知、已知混合、筛选范围、租户隔离、时间边界和重复记录。12 小时缓存基础或历史 CSV 已存在，不代表这两项完整交付。

### PR942-R4：通知策略与出站结果

按 [#951](https://github.com/TokenNotIncluded/lmm-api/issues/951) 保留低额度阈值跨越/重新武装、跨节点去重、用户通知偏好、独立管理员渠道订阅及明确 SMTP/Resend 传输。余额变化与通知事实在同一 Core 事务中记录，移动到/释放 hold 不误当真实经济损益；历史 journal 重放不得按今天余额生成旧告警。并发、重复及同事务再次观察不重复入队。

邮件发送前持久化唯一 attempt，结果/事件确认保持可恢复。若采用旧候选的至多一次尝试合同，必须明确网络未知和“记录 attempt 后崩溃”可能丢信；UNKNOWN 后不自动换通道或再发，不能承诺 exactly-once/inbox delivery。送达前重新检查当前收件人归属、已验证地址、权限及偏好，队列上限与停机后的事务行为有明确合同。验证阈值交叉、credit 后重触发、hold 不误报、竞争、队列满时回滚、撤权/改偏好后抑制、确认丢失不发生第二次连接。邮件服务真实验收单独记录。

### PR942-R5：原生价格与优先级管理

管理员可读取当前 native catalog、导入/导出价格、选择每模型精确 FX 的 keep/multiply/divide，审阅完整前后金额后确认；优先级支持表格/卡片排序并绑定同一 native catalog。价格来源标识、USD 单位、组/模型/speed、tier 边界和 provider metering 合同保持明确，不用浮点舍入把不可表示的标量硬转成合法价格。Multiply/divide 必须在提交时重新核对原汇率值和修订；缺少汇率不能猜默认值。

初始可信 routing baseline 只能由明确 owner 离线命令建立；在线 runtime 缺少 baseline 时拒绝，不能用客户端提供的 before 文档自举。已有 baseline 必须与完整 before snapshot 匹配。原生管理员会话、当前 MFA/策略在服务端再次校验；预览有有效期、每管理员/全局上限，确认绑定预览、来源/FX 证据、版本和快照摘要。发布用稳定 operation ID 和 CAS，撤销产生新预览及更高版本，不改写历史。

每节点接受新模型工作前刷新 durable release；在途请求保持原冻结价格。重启不能用旧文件配置覆盖已发布价格。运行角色只取得审阅的窄 wrapper，不能直写管理表、创建 baseline、替换 origin/credential reference 或扩张组/协议。验证无权限、过期预览、版本/FX 冲突、并发发布、ACK 丢失按原 ID 查询、重启与撤销；Web 必须保留资金预览确认。

### PR942-R6：显式 schema 升级

旧候选 component 27 的 26→27 升级尚有 `ROUTING_ADMIN_26_TO_27_PIN_PENDING` 硬阻断。后续迁移必须从当前已审核的真实 schema 建立 predecessor/target/source fingerprint、组件版本、目录证据、owner-only DDL/ACL 和已安装数据库升级路径；旧编号、旧 pin 及全新安装成功不能代替当前既有库升级证明。

在隔离 PostgreSQL 上验证正确前置升级、错误前置拒绝、重复执行、权限和 catalog 全状态读回、失败回滚及 COMMIT 结果不明恢复。运行角色不能 DDL 或自授权限，普通启动仅校验；已存在业务记录与钱不被覆盖。保留原失败到真实证据收齐并纳入新 regression，不能单删 pending 断言、修改受保护 hash 或扩大 runtime 权限让 CI 变绿。

### PR942-R7：候选中未交付的剩余项

旧 PR 明确未交付的 bounded JS image-task host 与 legacy scalar-price import bridge 继续由 #895 的 A07/A09/A10 承接。图像任务需要受限执行、原始 actor/payer/selected route/credential 固定、任务终态/轮询/取消、可信 usage 与同操作恢复；不能把通用聊天转发等同图像任务宿主。价格桥接必须按冻结来源/单位/模型映射进行精确转换、逐项隔离非法报价、可审阅预览及显式发布；缺失或不支持的旧价格表达式明确阻断，不能以当前默认价替代。

这些目标需先核对当前已实现接口与 A07 原完整要求，复用既有模块。关联的采购成本与销售定价隔离继续按 [#845](https://github.com/TokenNotIncluded/lmm-api/issues/845)、[#850](https://github.com/TokenNotIncluded/lmm-api/issues/850) 跟踪；本文件不将其折叠为“价格管理已完成”。

## 状态与追踪要求

旧 PR 迁移后，#895 保留 PR879-R1～R3、PR875-R1～R3、PR942-R1～R7 的独立待办及原关联 issue。每条记录采用“固定输入 → 实际行为证据 → 剩余条件 → 源码/配方提交”的顺序。适用的旧证据可引用原版本，不为整理状态反复运行未变化的重型验收。

源码和配方中已有的安全拒绝属于有效产品行为；缺失能力仍是待完成目标。任何未运行的真实 PG、实际进程、浏览器、供应商或生产验证继续分别标明。维护 issue 的完成条件是其自身范围得到证明；项目完成条件仍是完整 A01–A10/S11 和生产切换合同。
