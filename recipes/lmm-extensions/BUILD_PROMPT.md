# Go 扩展宿主

使用 Go 构建一个 lmm-extensions 程序。contracts 依赖定义 actor/payer/scope、Core RPC 和事件语义，所有资金与身份操作通过 Core provider；不可直接读写核心数据库。PostgreSQL/HTTP/TLS、支付商、MCP 服务和 SMTP 为外部前提；gRPC/Protobuf、TOML 和维护中的 SDK 可作为锁定且许可证允许的库。Go 各模块有自己的记录、权限和窄接口，不共享一个可任意访问全部业务的全局数据库对象。

模块按显式配置启用：商店商品/订单/购物车与完整购买退款，客服消息与归属，助手预设/会话/脱敏历史/归档、工具市场和 MCP 接入、运营和支付适配、公开法律文档快照。模块存在不代表启用。未装配模型执行的助手返回不可用，不伪造对话。扩展不得根据模型输出、浏览器角色或 host token 自行授权付款、管理动作或 sponsored payer。

支付适配管理供应商交互和记录，Core 保持支付订单资金权威。验签、金额/币种、事件与订单归属、授权用户、幂等消费以及退款/扣款恢复必须保持原 operation；网络未知按原键查询，不自动重建重复支付。合成供应商测试不能冒充真实 Stripe/ePay 兼容。配置不完整或缺少必要身份/Commerce/支付权限必须明确拒绝。

Core 事件经持久投递，consumer 按事件 ID 去重；重启或 Go 停机后恢复，不阻塞 Core 的模型链路。支持明确准入/排空与停止，已接纳记录保持可恢复；不声称模块内内存限制具有进程隔离能力。

默认配置 /etc/lmm.conf.d/extensions.toml，秘密 /etc/lmm.conf.d/secrets/extensions/，状态 /var/lib/lmm/extensions/，临时 /run/lmm/extensions/。installer/owner 和 runtime 分离；普通启动不改结构，不创建数据库角色。公开法律文档只读快照不带服务凭证、不调用 Core。Web compat 仅显式装配，不能成为绕过 Core 的身份或资金捷径。

Extore 同步保留为待验收集成，不自动推送其 main 或触发部署。未启用模块、超限、失败和依赖不可用要反馈准确错误，不返回空操作成功。
