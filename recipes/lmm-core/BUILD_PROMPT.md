# Rust 核心

依赖 contracts：消费其权限、资金、错误与 RPC 约束，作为 provider 为扩展和客户端提供一致接口。contracts 是规范/验收材料，不是另一运行时服务。使用 Rust 实现单个 Cargo 项目和一个 lmm-core 程序；可复用 Axum/Tokio/SQLx/Serde、维护的密码/TLS/Protobuf 库，锁定审阅过的版本并保留许可证，不自研密码算法。PostgreSQL、供应商 API、SMTP、反向代理和编译环境为外部前提。

## 用户行为和持久状态

身份：注册/登录、会话轮换与撤销、API Key 发放/撤销、个人/团队账户、邀请和成员权限、管理员与开发者动作 gate。OAuth/PKCE、旧密码验证、两因素、Telegram 和 Passkey 必须按明确能力状态交付；源码存在或未提交目录不意味着完整兼容。没有可安全映射的特权必须保持停用，不能直接置 active 恢复管理权。

资金：核心拥有用户、Key、账户成员/消费授权、钱包/订阅/预算、渠道秘密、价格、路由、账本与 charge 事实。事务性预留/预算与 append-only 账本，精确金额、幂等接纳、结算/退款、冲突与重启恢复。上游 streaming 不占长事务；durable checkpoint 提交后才发送对应输出。无最终 usage 不按零结算；未知结果通过原记录恢复，不再选付款方。

模型：按显式启用的 capabilities 开放 contracts 列出的 OpenAI Chat/Responses、Anthropic Messages、Gemini 模型入口。将模型能力、上游协议/参数、价格版本和可选路由绑定。鉴权、权限、余额/预算与必须依赖不可用时拒绝，模型默认关闭；错误、取消、超时、重试、流终止需要可解释结果和一致账本。不将渠道转换测试推定为真实供应商支持。

流式读取以完整 SSE 事件为单位逐帧解码：一次网络读取可能包含多帧，一帧也可能跨多次读取。保留尾帧 finish/usage，正确处理 CRLF、注释、event 字段和跨块 UTF-8；非 SSE JSON 不按网络 chunk 猜切。HTTP 正常结束但缺协议终态、或留有未闭合 SSE 帧时，返回 `upstream_truncated`；损坏的 HTTP 传输为 `upstream_connection_error`，非法编码/协议为 `invalid_upstream_response`，下游主动取消为 `request_cancelled`。公开响应、流内错误及日志保留同一分类，不合并成普通成功或客户端取消。错误分类不改变原付款方、预留和重放边界；缺终态或可信最终用量仍保持 UNKNOWN，不能仅因看见一次 usage 就结算。

领域：原生 pricing/余额/usage、历史与财务导出、通知与 setup、支付及 commerce 资金权威、工具市场 credit/委托、assistant model、礼品/红包、图片、悬赏、abuse 与信任价格等。先保持领域归属和现有明确接口，再对待完成能力登记阻塞；不能让 Go 接管权限或钱。

## 安装和运维

同一程序提供 --help、--version、schema-manifest、init-db、check-db、create-first-owner、upgrade-db；bootstrap-user 仅开发。--help/--version 不要求服务配置；--config 支持显式 TOML。默认 /etc/lmm.conf.d/core.toml，秘密 /etc/lmm.conf.d/secrets/core/，状态 /var/lib/lmm/core/，临时 /run/lmm/core/。不输出秘密。

init-db 仅新空隔离库，owner 单事务安装声明组件。普通 runtime 校验完整 schema/权限后启动，不建表、不创建角色、不授予权限；错误 schema、owner 误用、权限缺失应拒绝。upgrade-db 只接受明确固定前置契约，不能用普通启动或全新安装例子升级已有站点。

Core 可完全无 Go 独立工作；内部 RPC 默认不开放，明确认证并按模块/用户授权。公开 status 准确报告启用能力。按 ACCEPTANCE.md 用真实隔离 PostgreSQL 验证资金和拒绝路径，不以 mock 替代。
