# Rust 核心

依赖 contracts：消费其权限、资金、错误与 RPC 约束，作为 provider 为扩展和客户端提供一致接口。contracts 是规范/验收材料，不是另一运行时服务。使用 Rust 实现单个 Cargo 项目和一个 lmm-core 程序；可复用 Axum/Tokio/SQLx/Serde、维护的密码/TLS/Protobuf 库，锁定审阅过的版本并保留许可证，不自研密码算法。PostgreSQL、供应商 API、SMTP、反向代理和编译环境为外部前提。

## 用户行为和持久状态

身份：注册/登录、会话轮换与撤销、API Key 发放/撤销、个人/团队账户、邀请和成员权限、管理员与开发者动作 gate。OAuth/PKCE、旧密码验证、两因素、Telegram 和 Passkey 必须按明确能力状态交付；源码存在或未提交目录不意味着完整兼容。没有可安全映射的特权必须保持停用，不能直接置 active 恢复管理权。

邮箱：Core 内以独立证明密钥、限流密钥和加密出箱密钥支持验证码注册、当前会话绑定与找回密码。使用正式版本化身份 schema 和匹配运行权限，普通启动不做 DDL。`identity.email_config_file` 指向严格 TOML 侧配置，包含证明/限流密钥文件、活动出箱密钥及旧密钥文件映射、SMTP 和固定重置站点；秘密不从请求或日志取得。要求邮箱验证却缺少有效邮件配置时启动拒绝。消费证明、身份变更和必要通知在同一事务提交；SMTP worker 随主进程启动并在退出时等待当前发送。注册入口遵守各自启用状态与 Web 法律同意要求，不因共享邮件适配器绕过；重置不签发会话或替代 MFA。重置链接使用原证明及期限的 fragment，Web 启动前清除敏感地址；SMTP 250、UNKNOWN、外部收件箱送达分别记载，不能互相替代。验收应运行实际 Core、受限数据库角色、真实本地 SMTP 数据传输和 Web 行为，组件测试不抵扣完整流程。

资金：核心拥有用户、Key、账户成员/消费授权、钱包/订阅/预算、渠道秘密、价格、路由、账本与 charge 事实。事务性预留/预算与 append-only 账本，精确金额、幂等接纳、结算/退款、冲突与重启恢复。上游 streaming 不占长事务；durable checkpoint 提交后才发送对应输出。无最终 usage 不按零结算；未知结果通过原记录恢复，不再选付款方。

模型：按显式启用的 capabilities 开放 contracts 列出的 OpenAI Chat/Responses、Anthropic Messages、Gemini 模型入口。将模型能力、上游协议/参数、价格版本和可选路由绑定。鉴权、权限、余额/预算与必须依赖不可用时拒绝，模型默认关闭；错误、取消、超时、重试、流终止需要可解释结果和一致账本。不将渠道转换测试推定为真实供应商支持。

流式读取以完整 SSE 事件为单位逐帧解码：一次网络读取可能包含多帧，一帧也可能跨多次读取。保留尾帧 finish/usage，正确处理 CRLF、注释、event 字段和跨块 UTF-8；非 SSE JSON 不按网络 chunk 猜切。HTTP 正常结束但缺协议终态、或留有未闭合 SSE 帧时，返回 `upstream_truncated`；损坏的 HTTP 传输为 `upstream_connection_error`，非法编码/协议为 `invalid_upstream_response`，下游主动取消为 `request_cancelled`。公开响应、流内错误及日志保留同一分类，不合并成普通成功或客户端取消。错误分类不改变原付款方、预留和重放边界；缺终态或可信最终用量仍保持 UNKNOWN，不能仅因看见一次 usage 就结算。

领域：原生 pricing/余额/usage、历史与财务导出、通知与 setup、支付及 commerce 资金权威、工具市场 credit/委托、assistant model、礼品/红包、图片、悬赏、abuse 与信任价格等。先保持领域归属和现有明确接口，再对待完成能力登记阻塞；不能让 Go 接管权限或钱。

公开设置：`GET /api/option/` 返回生效配置及明确的 `partial`、`read_only` 和 `writable_keys`；当前内容写入范围为 `Notice`、`Announcements` 与 `console_setting.announcements`，两个公告键保持一致。PUT 接受单个字符串值，bulk 接受字符串映射及可选 `expected_values` 比较后保存，validate 只校验不保存。写入要求当前有效的最高管理员 Session，API Key、普通管理员及已撤销会话拒绝；公告内容格式、别名、版本和绑定 actor 的审计在同一事务处理，审计失败全部回滚，相同内容不增加版本或审计。并发比较冲突只允许一个请求胜出。COMMIT 确认丢失返回 HTTP 503 / `SYSTEM_OPTIONS_COMMIT_UNKNOWN`，不能声称回滚或盲目重试。运行数据库身份是受信任 Core，窄函数不独立替代 HTTP 用户鉴权、内容和审计；不得将这个边界描述成任意直接 SQL 都强制用户审计。主机配置、秘密、资金策略及未开放键不能伪装保存成功；完整系统设置功能仍按总项目 A10 和完整范围继续交付，不能用当前内容白名单缩小完成分母。

渠道读取：列表、搜索、详情和能力描述使用当前个人管理员 Session 及当前完整权限策略，按 ChannelRead 和 Channel 平台资源判定，不能拿 AdminUsersRead 代替。身份、策略及撤销在同一只读快照中核对；导入特权策略缺失/不完整/校验失败保持拒绝，原生管理员遵守现有默认目录规则，普通用户、API Key、团队及失效会话拒绝。此修复不将只读渠道等价为完整控制面，CRUD/测试/余额、动态秘密、持久发布与节点恢复继续按原始范围交付。

渠道控制面须保留传统站点的选择行为：原有符号优先级数值越大越优先，重试依次考虑更低的可用优先级；首选渠道必须属于本次选定的优先级和可用集合。同级在可用性过滤后按权重选择，权重全为零时均匀选择，否则零权重渠道不因平滑处理获得正权重。显式保留原请求的重试、首选和排除语义；对尚不支持的字段或供应商明确拒绝，不能静默丢弃或声称兼容。控制面写入、路由发布、授权快照与操作回执须有一致的事务和恢复语义；历史数据库升级须有明确的前驱、目标、围栏与原请求恢复，不以新安装成功代替升级验收。

## 支付网关与原生充值

Core 为 EPay、Stripe、Creem、Waffo Global、Waffo Pancake 分别实现独立供应商验签和付款归属验证；FastPay 明确排除。原始字节、签名、provider/商户/环境、Core intent、订单/交易、金额/币种和适用的商品/时间窗必须相符。按 provider namespace 对交易去重，可信 ingress proof 先持久化，后续入账只消费原证明一次；Go 的验证结果、网页回跳和支付商已收款状态都不能替代 Core 账本回执。保留旧单的原配置与证明恢复，订阅、商店和退款继续分别完成原完整验收。

消费 contracts 的 `PreparePersonalTopup` 与 `GetEPayCheckout`：个人账户由当前 Core 会话决定，原报价的金额、credits、环境与 key 固定；报价不发外部订单。EPay 的共享 MD5 密钥仅保留于 Core，Go 只获取已存在 intent 的签名 checkout 结果，不获得任意代签能力。显式返回 GET/POST 提交方式；Linux DO 保留 LDC、`type=epay` 和 POST `/epay/pay/submit.php`，不按 CNY 或旧钱包兑换示例计算。

Waffo Global 的验签公钥不能单独证明某个商户已收款。Core 还须使用自己持有的账户 API key 和私钥，向当前环境的固定官方 HTTPS inquiry 端点独立查询；调用方不能指定端点、查询体或查询结果。核对商户、原订单/请求编号、acquiring 交易号、成功状态、币种和精确金额后才接受 proof；失败、超时、状态未知和任何不匹配均不授予入账。正式与测试配置分别绑定，凭据不离开 Core；Pancake 使用自己的协议与配置，不能复用 Global 的信任判据。

支付结构通过正式 `payment-gateways` 组件加入，提供固定 **28 → 29 组件** 的 `core-payment-gateways-v1` 升级：核对确切前驱 manifest/catalog、物理数据库与角色、目标固定结构和原操作，拒绝未知前驱和既有重复 provider 交易。升级保持旧表数据、序列、权限与既有资金不变，不导入或伪造付款证据；新结构、运行合同及 owner-only 升级回执同事务提交。提交回执丢失只能以原请求观察已提交结果，不能换 ID 重做；新装与旧站升级各自验收，普通启动不执行 DDL。

### Coinbase Business Checkout

新增 `coinbase` 个人钱包充值，默认关闭，管理员配置完整后显式启用。Core 保留 USD 报价和账本金额，外部支付固定为同数额 USDC，两位小数按整数精确转换；这是产品的 1:1 计价规则。原金额、Core intent、商户、渠道和 test/live 环境在创建时固定，实际 USDC 支付单位随原证明保留。

Core 独立持有各环境的 webhook secret，对原始 body 与 `X-Webhook-Signature`（兼容同值旧头 `X-Hook0-Signature`）中的 v0 执行 HMAC-SHA256 验证，签名输入为时间戳、句点和原始 body；限定签名时间窗并拒绝未来超窗、歧义字段及错误秘密。只有 `checkout.payment.success` 与 `COMPLETED` 同时成立，且 checkout ID、metadata、金额、USDC 币种和原 intent 归属全部匹配，才能持久接纳证明。证明唯一消费，原订单重复回调和重启恢复只对应一次账本入账；网页回跳与 Go 状态不授予入账权。

Coinbase schema 作为独立第 32 组件追加，提供固定 31 → 32 组件离线升级，核对确切前驱、目标 manifest/catalog、角色和原操作回执；旧组件内容及其历史指纹保持。新装、既有库升级与提交不明恢复分别验收，普通启动不执行 DDL。自动退款保持关闭；已付款后的退款事件不能走充值入账路径。

## 性能观测

正式模型转发记录基于单调时钟的 relay 耗时，在 execute 结束时记录 UTC 秒，排除后续资金终结时长。流式 TTFT 是第一个非空文本、思考或工具参数 Delta，排除 Start、Usage、心跳和空 Delta；非流式 TTFT 未知。只有供应商完整终态、已确认结算、Completion 无错误的请求计成功；不能只看提前生成的 Report.outcome。TPS 定义为成功样本输出 token 总量除以其 relay 总秒数，不冒充供应商纯生成速率。性能事实与资金、重放和恢复授权分离，保存失败不重新外发或扣费。

在已有 charging usage_events 中以原 request_id 和固定事件种类保存一次，沿用原请求冻结的 canonical model/group，不能使用当前 Key 组重新归属。进程死亡或写入未确认时允许样本缺失，读取明确列出缺样本；历史性能未导入、非共享 relay 入口和未接纳 HTTP 不能假算完整。提供实际 /api/perf-metrics/summary 与 /api/perf-metrics：沿用当前个人 Session gate，在同一只读快照读取当前 enabled 组与模型可见性；匿名、Key、团队会话、撤销会话拒绝，不能泄露无权限模型/分组。hours 默认24、1至720；bucket_seconds 默认3600，仅60/300/3600，按 execute 结束 UTC 的半开窗口归桶。最近三个非空桶与均值使用实际样本和分母，不平均组/模型均值；未知值为 null，Web 不能 Number(null) 后显示成零。完整历史性能与原有全功能范围继续保留，不能把原生观测切片计为全量恢复。

## 安装和运维

同一程序提供 --help、--version、schema-manifest、init-db、check-db、create-first-owner、upgrade-db；bootstrap-user 仅开发。--help/--version 不要求服务配置；--config 支持显式 TOML。默认 /etc/lmm.conf.d/core.toml，秘密 /etc/lmm.conf.d/secrets/core/，状态 /var/lib/lmm/core/，临时 /run/lmm/core/。不输出秘密。

init-db 仅新空隔离库，owner 单事务安装声明组件。普通 runtime 校验完整 schema/权限后启动，不建表、不创建角色、不授予权限；错误 schema、owner 误用、权限缺失应拒绝。upgrade-db 只接受明确固定前置契约，不能用普通启动或全新安装例子升级已有站点。

Core 可完全无 Go 独立工作；内部 RPC 默认不开放，明确认证并按模块/用户授权。公开 status 准确报告启用能力。按 ACCEPTANCE.md 用真实隔离 PostgreSQL 验证资金和拒绝路径，不以 mock 替代。

升级历史：同一 Core 的 `upgrade-history` 显式安装或查询独立 PostgreSQL 18 历史库，不能在普通启动初始化。独立管理员和存储不能随目标库备份恢复或切换到旧副本；本机两实例不证明生产故障域独立。固定 authority ID、物理实例与无 owner 权限的 maintenance writer，拒绝将目标同一物理集群当独立历史。原站点/操作/请求/候选/目标绑定不可替换，closing→held→started→committed 单向 CAS 消费，不因租约、重连、目标物理恢复或数据库停启清空。COMMIT 回执丢失保持 UNKNOWN，按原操作观察，禁止重做。存储记录由控制器另行验证的围栏与回执，不把任意 hash、历史状态或命令成功当正式升级/开放权限；查询始终无 retry/open 授权。同一 Core 还须提供 `upgrade-offline` v3：固定且不同的 v5/26、Console v1 v5/26 与 v6/27 前驱，核对实际候选二进制、目标实例/OID及固定结构；HBA 必须由 PostgreSQL 实际加载且在目标 PGDATA 恢复范围之外，仅允许本地 SCRAM 安装连接并拒绝其他目标连接。显式部署单元按实际制品核对、持久 mask 并停止，排空目标写者后，以原专用连接加锁并执行串行化事务。外部 STARTED 先于目标变更；旧列/行/多重性/序列/权限保持，固定 DDL、合同更新及不可变本地回执原子提交。ACK 丢失仅恢复观察；原始绑定的外部历史和本地回执共同判定，旧物理备份不能把已提交变成可重做。recover 会重施运行围栏，不重复业务或 schema 写入。目标 PG 与列出的单元不是全站围栏证明；完整历史资金迁移、部署清单、最终开放、finalization 和生产验收继续按总项目范围交付。

## 全站财务统计与离线原始来源

实现 /api/finance/site-statistics 的完整既有语义：管理员在同一只读快照内读取所有保留用户（含停用/删除）的个人可用净余额、按原历史UsageProjector依据归一化的消费及新增Core净消费；余额、消费均用十进制整数字符串，规范credits_per_usd=500000。不能把预留、返佣、收入、团队余额或赠送当作原个人余额/现金充值。历史非零used没有完整原始归一化审计时整项不可用，不能直接除当前K，也不能从历史log金额倒推。

离线来源必须绑定相同归档、身份映射、来源各表/行及原开账counter：保留完整原始UsageProjector对rebase/divisor/rounding及费用撤销的证据；充值按成功wallet TopUp而非订阅镜像，保留原实付/退款微单位，按旧source分类区分现金、LDC虚拟单位、未确认、非法和排除。未知原币种/原金额不得用credits、Money浮点、预计订单或汇率补齐。来源写入只允许显式owner离线事务，全量验证后完成标记最后写、原键重放无改动、冲突拒绝；runtime/Go不能写来源或账本。新增来源schema须正式新装并提供精确27组件前驱的匹配升级，不可普通serve偷偷DDL或弱化既有升级校验。

历史 observed_at 只表示归档开始观察，不是完整快照的可靠结束时刻，不能单凭该时间证明新旧充值不重叠。来源导入在停写、排空其他连接后，由目标数据库封存原 Native 支付意图编号上界；配方材料不能伪造或覆盖该默认值，精确重放保持原封存值及全部序列。旧意图此后才确认付款仍需原始对应证明，未证明时保持 UNKNOWN；新意图需在封存之后创建。依赖的支付序列必须递增且不循环，回退或与已存在编号不符时拒绝。全站业务验收继续包含实际旧来源与既有未决付款，不能通过跳过或删除它们声称恢复。

Native 请求编号是字符串，不能假定递增。来源导入还必须由目标数据库封存当时已经存在的完整请求编号集合，配方和源计划不能覆盖它；精确重放保持原集合。该集合中的历史用户请求已结算或随后才结算时，统计保持 UNKNOWN，避免和历史已用额度重复相加。封存后产生的请求即便与导入处于同一秒，也可根据独立记录计入。完整旧归档之外、由 Core 实际创建且具有真实创建审计的 Native 用户，其历史基数为零；不能仅因创建早于旧库归档就拒绝，也不能通过伪造审计或缺失的旧用户来源绕过完整性检查。

上述读取规则不等于全量导入已支持任意 Native 共存次序。若声称支持 Native 先存在再首次导入旧站，必须提供完整身份、开账及全站财务的正式导入链路证据，并处理既有 fresh gate；不能由 SQL 拼装状态推导整条迁移路线已可用。

原生创建来源须覆盖正式安装 CLI 创建首个管理员的独立路径，不能仅识别普通注册和开发 bootstrap。正式首管的真实创建审计必须与该用户匹配；新装无历史账务时应能读取准确零统计。历史导入标记及完整来源校验继续生效，不能因首管身份而忽略缺失的历史。

历史口径见[上一版构建合同](https://github.com/TokenNotIncluded/lmm-api/blob/928a3ec03ede08f40467535bc74437f99c523d27/recipes/lmm-core/BUILD_PROMPT.md)：当时 Native 现金限定为已确认 Stripe 订单与其已核验退款，该来源保留为旧范围记录。

当前 Native 收款只统计已成功 Core 入账的 EPay、Stripe、Creem、Waffo Global、Waffo Pancake 原生充值，按原金额单位分类：USD、CNY、EUR 现金各自分列；LDC 进入虚拟单位列表，不能混同 USD、当作现金或用 credits 推算收入。退款仅使用各自已有合同下已核验并已在 Core 回冲的记录，同币种累计；新增充值网关不意味着其退款能力已经完成。退款重放、阻塞、延迟和未决结果均不得重复或隐去；纯 grant、钱包转账和商店款不算充值。原管理权限、无权限拒绝、无额外个人信息、总数越 JS/SQL bigint 仍精确。来源缺失、相冲突或不完整时保持明确 503/UNKNOWN，不能用仅 Native 的 partial 响应代替全站统计。
