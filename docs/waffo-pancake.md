# Waffo Pancake（测试环境）

当前 Go 后端使用官方 `github.com/waffo-com/waffo-pancake-sdk-go` 完成服务端集成。不要把 `@waffo/pancake-ts` 放进 Web bundle：Waffo 的私钥只能留在服务端。

## 配置

只需要把这两个凭据注入 API 进程（不要提交到 Git）：

```sh
export WAFFO_MERCHANT_ID='从 Dashboard → API & Development 顶部复制的 Merchant ID'
export WAFFO_PRIVATE_KEY='API Key 对应的 RSA 私钥 PEM'
```

`WAFFO_MERCHANT_ID` 不是 `storeId`。Store/Product ID 是运行时配置：可以在管理员的 Waffo Pancake 配置中从 Dashboard 选择，或使用后端的 pair/catalog 路由创建/保存。

## 端点

- 已登录用户创建钱包 checkout：`POST /api/user/self/waffo-pancake/pay`
- 测试 webhook：`POST /api/waffo-pancake/webhook/test`
- 正式 webhook：`POST /api/waffo-pancake/webhook/prod`

在 Pancake Dashboard 的 Webhooks 中注册测试 URL，并至少订阅：
`order.completed`、`refund.succeeded`、`refund.failed`。请求体会先验签，再按订单的 merchant external ID 绑定本地订单。

## 测试卡与验收

测试模式使用 Visa `4576 7500 0000 0110`，任意未来有效期和三位 CVC。成功后应看到：

1. checkout session 返回 `checkout_url`；
2. webhook 收到并处理 `order.completed`，本地充值订单变为成功；
3. 退款成功事件写入幂等的财务收入冲销记录；退款失败只记录审计，不扣用户额度；
4. 重复投递不会重复记账。

退款不会自动从用户余额扣除。部分退款、余额已消费和多次退款需要单独的业务政策；当前实现先保证签名、身份绑定、可追溯和财务一致性。

本地回归：

```sh
cd apps/api-go
go test ./service ./controller -run 'WaffoPancake|PaymentWebhook' -count=1
```

