# Go 扩展宿主

使用 Go 构建一个 lmm-extensions 程序。contracts 依赖定义 actor/payer/scope、Core RPC 和事件语义，所有资金与身份操作通过 Core provider；不可直接读写核心数据库。PostgreSQL/HTTP/TLS、支付商、MCP 服务和 SMTP 为外部前提；gRPC/Protobuf、TOML 和维护中的 SDK 可作为锁定且许可证允许的库。Go 各模块有自己的记录、权限和窄接口，不共享一个可任意访问全部业务的全局数据库对象。

模块按显式配置启用：商店商品/订单/购物车与完整购买退款，客服消息与归属，助手预设/会话/脱敏历史/归档、工具市场和 MCP 接入、运营和支付适配、公开法律文档快照。模块存在不代表启用。未装配模型执行的助手返回不可用，不伪造对话。扩展不得根据模型输出、浏览器角色或 host token 自行授权付款、管理动作或 sponsored payer。

## 支付网关与业务范围

支付适配负责向支付商下单、查询和接收回调，Core 决定这笔钱属于谁、能否入账以及能退多少。恢复旧单体 `3948a0a1ee1cefdd016a7aa45a11851545bb4b85` 的五类 provider：EPay、Stripe、Creem、Waffo Global、Waffo Pancake。EPay 包含支付宝、微信、LINUX DO Credit 和管理员明确配置的自定义 type；这些是同一协议下的支付方式。保留各网关独立的配置、凭据、环境和启用条件，不能以一个 Stripe 用例或一个 provider 枚举代替全部接入。

钱包充值、套餐购买/续费、商店付款和退款分别实现并验收。旧钱包七个预设入口是 EPay 支付宝、EPay 微信、LINUX DO Credit、Stripe、Creem、Waffo、Waffo Pancake；Waffo Global 的卡、Apple Pay、Google Pay 是服务端配置的子方式。订阅恢复 EPay 一次性购买、Stripe/Creem 的周期支付、Pancake 一次性及订阅产品映射和平台余额支付。商店恢复平台余额、商户自有 EPay、商户自有 Pancake、平台 Pancake、平台 Linux DO；旧钱包有某个网关，不自动授予它商店结算能力。

Linux DO 使用 EPay 协议并以 LDC 结算，保留显式单位与定价。普通支付宝、微信仍为 CNY，不能沿用同名币种标签把 LDC 当现金。商店平台 Linux DO 绑定 `https://credit.linux.do/epay`；任意 EPay 主机不继承 Linux DO 的退款 API。Pancake 的商店退款包含申请、查询、签名通知与实际执行证据，退款申请受理不等于退款成功；Linux DO 仅按其已核实的全额退款合同，任意商户 EPay 保留人工退款状态。

FastPay 已由用户明确淘汰：不实现 adapter、不注册路由或运行配置、不提供 UI 入口，也不把历史配置迁成启用网关。历史账目和拒绝未知 provider 的负向 fixtures 可保留；保留记录不授予新付款能力。

Core 保持支付订单资金权威。原始回调先由 Core 验证签名、商户、环境、金额、币种、事件与订单归属，持久接纳后再按唯一证明消费；Go 只持有外部 IO 所需凭据、独立业务记录和窄 RPC 权限，不接触 Core 数据库。网络未知、幂等消费、退款和扣款恢复保持原 operation，不换 key、付款方或金额重新执行。关闭新下单后，已存在订单的查询、合法回调和退款继续按原凭据/配置快照处理。配置不完整或缺少必要身份、Commerce、支付权限时明确拒绝。

Web 用户入口先取得服务端渠道和精确报价，明确确认后才创建外部 checkout，再按原订单读取结果。固定商品金额不可由客户端修改；金额和 credits 使用精确十进制整数及明确单位。401、响应丢失、刷新页面不能自动重放金融写请求；支付商已收款和 Core 已入账分别展示，只有后者表示钱包到账。capability 与实际装配一致，通过三者联合验收核实。

完整产品矩阵与固定源码证据由集成配方 `lmm-api/PAYMENT_PARITY.md` 维护；本包自身保留上述全部目标，不以该集成文档作为隐藏依赖。当前多 provider 钱包充值候选不代表订阅、商店外部付款、退款闭环或真实支付商验收已完成。

## 宿主与运行边界

Core 事件经持久投递，consumer 按事件 ID 去重；重启或 Go 停机后恢复，不阻塞 Core 的模型链路。支持明确准入/排空与停止，已接纳记录保持可恢复；不声称模块内内存限制具有进程隔离能力。

默认配置 /etc/lmm.conf.d/extensions.toml，秘密 /etc/lmm.conf.d/secrets/extensions/，状态 /var/lib/lmm/extensions/，临时 /run/lmm/extensions/。installer/owner 和 runtime 分离；普通启动不改结构，不创建数据库角色。公开法律文档只读快照不带服务凭证、不调用 Core。Web compat 仅显式装配，不能成为绕过 Core 的身份或资金捷径。

Extore 同步保留为待验收集成，不自动推送其 main 或触发部署。未启用模块、超限、失败和依赖不可用要反馈准确错误，不返回空操作成功。
