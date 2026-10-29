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

## 安装和运维

同一程序提供 --help、--version、schema-manifest、init-db、check-db、create-first-owner、upgrade-db；bootstrap-user 仅开发。--help/--version 不要求服务配置；--config 支持显式 TOML。默认 /etc/lmm.conf.d/core.toml，秘密 /etc/lmm.conf.d/secrets/core/，状态 /var/lib/lmm/core/，临时 /run/lmm/core/。不输出秘密。

init-db 仅新空隔离库，owner 单事务安装声明组件。普通 runtime 校验完整 schema/权限后启动，不建表、不创建角色、不授予权限；错误 schema、owner 误用、权限缺失应拒绝。upgrade-db 只接受明确固定前置契约，不能用普通启动或全新安装例子升级已有站点。

Core 可完全无 Go 独立工作；内部 RPC 默认不开放，明确认证并按模块/用户授权。公开 status 准确报告启用能力。按 ACCEPTANCE.md 用真实隔离 PostgreSQL 验证资金和拒绝路径，不以 mock 替代。

升级历史：同一 Core 的 `upgrade-history` 显式安装或查询独立 PostgreSQL 18 历史库，不能在普通启动初始化。独立管理员和存储不能随目标库备份恢复或切换到旧副本；本机两实例不证明生产故障域独立。固定 authority ID、物理实例与无 owner 权限的 maintenance writer，拒绝将目标同一物理集群当独立历史。原站点/操作/请求/候选/目标绑定不可替换，closing→held→started→committed 单向 CAS 消费，不因租约、重连、目标物理恢复或数据库停启清空。COMMIT 回执丢失保持 UNKNOWN，按原操作观察，禁止重做。存储记录由控制器另行验证的围栏与回执，不把任意 hash、历史状态或命令成功当正式升级/开放权限；查询始终无 retry/open 授权。同一 Core 还须提供 `upgrade-offline` v3：固定且不同的 v5/26、Console v1 v5/26 与 v6/27 前驱，核对实际候选二进制、目标实例/OID及固定结构；HBA 必须由 PostgreSQL 实际加载且在目标 PGDATA 恢复范围之外，仅允许本地 SCRAM 安装连接并拒绝其他目标连接。显式部署单元按实际制品核对、持久 mask 并停止，排空目标写者后，以原专用连接加锁并执行串行化事务。外部 STARTED 先于目标变更；旧列/行/多重性/序列/权限保持，固定 DDL、合同更新及不可变本地回执原子提交。ACK 丢失仅恢复观察；原始绑定的外部历史和本地回执共同判定，旧物理备份不能把已提交变成可重做。recover 会重施运行围栏，不重复业务或 schema 写入。目标 PG 与列出的单元不是全站围栏证明；完整历史资金迁移、部署清单、最终开放、finalization 和生产验收继续按总项目范围交付。
