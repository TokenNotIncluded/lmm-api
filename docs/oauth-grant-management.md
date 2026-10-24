# OAuth 授权管理

浏览器完成 OAuth 同意后，模型客户端拿到的是 Core 签发的访问令牌和刷新令牌。授权不再是“登录成功后永久存在”的隐形状态：个人资料的“Authorized clients”卡片读取当前账号的浏览器授权族，并可逐个撤销或一键撤销全部授权。

页面只看到客户端、用途、scope、预算和有效期。访问令牌、刷新令牌、token digest 和模型请求原文永远不返回浏览器。撤销请求使用当前 HttpOnly 浏览器会话重新锁定用户、个人账户和 `auth_version`，family ID 只是一次性定位符；换账号、换账户或旧会话不能撤销别人的授权。

Core 提供以下同源、带会话 Cookie 的接口：

| 方法 | 路径 | 作用 |
| --- | --- | --- |
| `GET` | `/api/user/auth/oauth2/grants` | 列出最多 100 个浏览器授权族的脱敏摘要 |
| `DELETE` | `/api/user/auth/oauth2/grants/{family_id}` | 撤销当前账号的一项授权 |
| `POST` | `/api/user/auth/oauth2/grants/revoke-all` | 紧急撤销当前账号全部浏览器授权 |

这些接口继承 Core 的同源 Origin/Referer 检查和 HttpOnly、SameSite 会话保护。撤销后，模型入口在下一次授权检查中拒绝该 family 的 access token 和 refresh token；已在执行中的请求仍按既有 Core 账本和排空规则收尾，不会伪造一个成功结果。

客户端应在收到 `401 invalid_token` 后删除本地 access/refresh token，并要求用户重新授权。页面管理是紧急控制面，不能把 bearer token 复制到页面、日志、URL 或客服消息中。
