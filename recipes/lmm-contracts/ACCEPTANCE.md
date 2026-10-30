# 合同验收

- 用同一组向量在 Core provider 和每个 consumer 验证 actor/payer 区分、个人/团队隔离、拒绝优先、金额精度、重复幂等与 UNKNOWN。
- 空 funding order 与缺失/null 的语义不同：空列表无效，缺失/null 为 owner-only；跨账户付款失败不得遗留预留。
- 模型请求接纳后改变价格/预算周期不能改变原请求结算归属；断流无最终 usage 保留预留。
- SSE 多帧合并和单帧拆分不改变正文、终态或最终 usage；缺终态、半帧、传输损坏、协议错误、主动取消不能共用成功终止或错误类别。`upstream_truncated` 在 JSON/流内错误/日志中一致；HTTP 状态已经发出时不能假装返回新的状态。
- RPC host 凭据不能替代用户授权；未知模块、过期/撤销 scope、跨租户、伪造 event 均拒绝。
- 事件乱序/重复及 consumer 重启不能重复执行业务资金动作。
- 公开/私有价格与余额不串权；未知金额单位和不可用服务不可表示成功零值。
- 文本/API 新实现之间一致性与旧二进制 wire 互通分别验收；后者本次明确未覆盖。
- `PreparePersonalTopup` 用真实当前会话解析个人账户；宿主令牌、Web 用户 ID、跨会话请求和伪造付款方不能替代用户授权。相同 key 重试取得同一固定 intent，冲突金额/币种/渠道拒绝，报价阶段不产生外部支付或账本 credit。
- `GetEPayCheckout` 只能读取并签名已存在 intent 的固定值；EPay 密钥不进入 Go 配置、RPC、日志或浏览器。provider/商户/环境/订单/金额绑定正确，未知 intent 与无权限宿主拒绝；方法必须显式为 GET/POST，Linux DO POST 参数逐项保留，不能使用任意调用方参数代签。
- Rust provider 与 Go consumer 对新增方法、消息及 `EPayCheckout.method = 4` 编解码一致，原字段号和 enum 值没有改动；API 整数大于 JavaScript 安全整数仍无精度损失，LDC 不被解释为现金或隐式换汇。接收证明、支付商收款、Core 入账、退款各状态不能混为成功；GET/POST handoff、UNKNOWN 恢复与未启用 FastPay 由实际消费方分别验证。

本文件定义验收条件；当前执行结果和未运行项记录于主配方项目的 STATUS.md 及关联 PR。保留既有二进制 wire 互通与完整资金验收的独立门槛。
