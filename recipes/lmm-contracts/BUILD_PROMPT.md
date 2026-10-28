# LMM 共享接口与权限合同

此叶子配方产出可由四个组件消费的接口规范和合成测试向量，不要求额外服务进程。没有配方依赖。JSON/HTTP、Protobuf/gRPC、TOML、UTC 时间和整数金额编码为外部标准；只实现产品需用的部分，不自行创造密码协议。

## 统一边界

| 事实 | 契约 |
| --- | --- |
| 身份 | 用户、团队、账户、操作人、Key owner、付款方分别建模，不因数值相同而视为同一 namespace |
| 权限 | 平台等级、平台动作权限、团队角色、开发者与 OAuth scope 各自检查；显式拒绝优先；浏览器/模型/Go 自报角色无授权效力 |
| 付款 | 个人 Key 默认仅个人资金；团队 Key 使用所属团队；管理员或团队成员身份本身不是消费授权 |
| 单位 | 账本单位 credit_500k_usd，500000 整数单位 = 1 USD；精确整数 credits 与明确货币单位；不把积分当美元；未知单位必须报错，不显示为零；报价版本固定 |
| 操作 | 接纳时固定 actor、key owner、payer、价格版本、钱包/订阅分项及预算周期；后续重试、退款和恢复引用原操作 |
| UNKNOWN | 未知提交/结算结果按原幂等键查询；不得重新换付款人、重复扣费或释放未确认预留 |
| 事件 | Core 持久 outbox，至少一次投递，扩展按事件 ID 去重；Go 停机不丢失已接受资金事实 |
| RPC | 明确 host 身份、每模块权限、用户/团队权限、消息大小/截止时间、幂等键与审计；不可公开无认证 mutation |
| 秘密 | Core 独占渠道和核心数据库秘密；扩展只拿自身角色及窄 RPC 凭据；所有输出脱敏 |

## 公共可观察接口

Core 提供 /v1/models，POST /v1/chat/completions、/v1/responses、/v1/messages、/v1beta/models/{action}；模型关闭必须拒绝，不能假定所有协议转换、模型能力和供应商已验收。JSON 常规响应与 SSE 完成/错误、流中断及最终 usage 必须明确区分。未知/不支持参数按所选协议清楚拒绝，不悄悄改写资金含义。

流失败码：正常 HTTP EOF 但缺上游协议终态或残留半帧为 `upstream_truncated`；HTTP 传输中断/损坏为 `upstream_connection_error`；非法 UTF-8、JSON 或协议为 `invalid_upstream_response`；客户端主动取消为 `request_cancelled`。前三者在响应尚未开始时为 HTTP 502；已开始 SSE 时使用流内错误表达，不能声称改写已经发送的 HTTP 状态。主动取消保持现有 499 语义，取消后客户端可能已无法接收错误。日志和原操作恢复保留失败类型；这些分类不授权重发已经输出的请求、不把未知用量改成零、不释放未经确认的预留。

原生只读接口 GET /v1/balance、/v1/usage：单一 Bearer opaque Key，验证有效性、允许 IP、scope/同意与读限制；不接受 OAuth token 或别名头冒充该 Key。账号级共享读限制和 /v1/pricing Key 定价读取属于原开发目录的未提交目标，本次不规定其实现已接线。

/core/v1/identity、/core/v1/keys、/core/v1/credentials/{id}、/core/v1/teams 与 invite 操作用于核心身份及团队管理。公开 /core/v1/pricing/public、会话 /core/v1/pricing/session、POST /core/v1/pricing/runtime 按各自身份与授权区分结果，不能把私密价格从公开接口泄露。

/health/ready 表达当前已启用能力就绪；/health/model-ready 在模型关闭时为 503；/api/status 保留 capability 信息、版本与请求 ID。公共头不得泄露内部部署/归档/schema 身份。匿名访问受保护资源应拒绝；状态码、结构与具体客户端消费场景一起验收。

内部领域包含 control/commands、ledger、payments、commerce、events 和 assistant；RPC 应分别表达 operation/event/idempotency、actor/payer/scope、金额单位、版本、成功/拒绝/未知结果。固定源码的 contracts/proto/lmm/core/v1/*.proto 只是观察来源；本包 PROTOCOL.md 内含完整固定接口声明，保留服务名、方法、消息字段号、enum 值、metadata 身份边界和错误语义，不要自由改动既有 wire 合同。用黄金字节向量与固定旧 provider/consumer 验证直接互通；本次仅保全声明，没有执行二进制互通验收。

TOML 配置显式引用绝对秘密文件路径，不展开环境变量或 shell；runtime 不能用 owner 权限。安装、启动、升级、迁移分开。跨进程请求不得持有整个上游流期间的数据库长事务。
