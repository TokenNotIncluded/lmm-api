# 独立应用授权与轮换恢复

此合同补齐桌面 CLI 与 Core 之间的 managed application 接口。它是必须实现和验收的要求，不是当前能力或上线状态声明。Core 继续独占身份、权限、授权预算与账本；复用现有 OAuth、浏览器 Session/MFA、PKCE 和账务事务，不增加运行服务或另设付款方。

## 客户端与发现

- 独立客户端包括 `client_id=lmm-cc-switch`、`application=cc-switch`，以及 `client_id=lmm-magpie`、`application=magpie`。client/application 必须匹配注册的组合，不得互相冒用或复用只读 `lmm`、Pi 家族。每个安装使用独立随机 `installation_id`，编码为 64 位小写十六进制；它是绑定标识，不是认证秘密。Magpie 原生 provider 的额外合同见 [MAGPIE.md](MAGPIE.md)。
- 每个客户端默认不启用，分别配置有限额度上限，不因开启另一客户端而自动启用。仅当显式配置有限额度、专用恢复加密密钥、实际模型目录与网关，以及匹配数据库组件和运行权限后，才发布对应能力。缺少前提须明确不可用，不公布无法完成的流程。
- OAuth 授权服务器及资源发现绑定同一 issuer。`lmm_managed_clients` 条目包含上述 client/application、同 issuer 的 `/api/oauth2/application` 状态地址，以及 `refresh_recovery={contract: lmm.refresh-recovery.v1, window_seconds: 600}`。
- issuer 为具有正常受信任证书的 HTTPS 域名来源；不得通过关闭验证、复用生产 TLS 私钥、使用其他应用 token 或伪造 grant 完成验收。CLI 原生 PKCE 回调仍使用随机端口的回环 HTTP 地址。

## 授权、令牌与实时状态

授权请求保留现有严格参数校验，另接收安装 ID、application 与 `display_label`。标签非空、UTF-8 不超过 160 字节且无控制字符，浏览器显示时转义。只读/Pi 请求不得接受这些字段。安装、client、应用、用户、Session/MFA、scope、分组及有限预算在同意、code 和 family 各阶段一致；不因客户端自报字段获得权限。

用户实际同意独立应用、安装、分组、scope 和预算后才签发。预算为 `credit_500k_usd` 整数，范围为 1 至 9007199254740991，且不超过服务端当前上限；付款方保持该用户的个人账户。模型消费仍通过既有预留、当前开发者/分组策略和账本，家族所有 token 代际共同计算待处理与已消费额度。

每个应用及授权家族单独计算预算；同一用户在 Magpie、CC Switch 与 Pi 上的额度不能相互挪用。个人中心的授权清单按当前用户及浏览器会话隔离缓存，托管授权返回其真实 `application`、`installation_id` 和 `display_label`，使用户能够区分设备并单独撤销。旧只读/Pi 清单保持兼容，不伪造设备信息。

code 交换及 refresh 表单必须带匹配的 `installation_id`。成功 JSON 有九个字段：`access_token`、`refresh_token`、`token_type=Bearer`、`expires_in`、`scope`、`client_id`、`installation_id`、`grant_id`、`budget_credits`。两个 token 保持现有规范编码；grant 为 `lmm_gr_` 加 64 位小写十六进制；`expires_in` 为 1 至 86400 的整数，预算为上述安全整数。旧客户端响应形状保持原合同。

`GET /api/oauth2/application` 只接受当前有效的该应用 Bearer，返回 `contract=lmm.application-status.v1`、`client_id`、`application`、`installation_id`、`grant_id`、整数 `budget_credits`、`scopes` 数组及 Unix 秒 `access_expires_at`。服务端重新验证家族、用户、当前权限与撤销状态；不能相信 CLI 本地状态。时间为该 access token 的真实绝对到期，scope 不扩大且包含实际获准的 `catalog:read`、`models:invoke`。状态读取不触发供应商调用或扣款。关闭新签发不得使已有家族无法撤销。

## 六百秒精确轮换恢复

CLI 在发出 refresh 前，先将旧凭据、随机 64 位小写十六进制 `rotation_id` 和开始时间持久保存至该安装的系统凭据库。响应丢失或本地后继保存失败时，只能恢复原操作，不得另建 rotation 或改用另一应用的凭据。

Core 在同一事务中校验当前用户、client、安装、family、scope、预算和有效期，锁定原 lineage，消费旧 refresh，创建唯一后继并保存可恢复回执。回执绑定原 refresh 摘要、rotation、请求内容及同一 family，使用专用 AEAD 密钥保存同一后继秘密与原绝对到期；不得将明文 token 存表或输出日志。普通运行角色不具有安装、任意改写或删除 lineage 的权限。服务器窗口以首次成功轮换事务内保存的数据库时间为起点，固定 `recover_until=created_at+600秒`；最终授权与恢复读取按数据库时钟重检。CLI 仍坚持原本地开始时间的严格小于 600 秒限制，不借服务端起点延长本机期限。

- 同一操作在窗口内、后继仍有效且未继续消费时，返回同一后继 token、grant、scope 和预算；`expires_in` 按原 access 绝对到期计算剩余正整数秒，不能重放原完整 TTL。不要求响应字节相同，不新建 token、不延长窗口、有效期或预算。
- 同一已知操作超过窗口、后继已消费/过期或授权已撤销时，明确拒绝恢复，不能返回貌似可用的旧凭据，也不因该幂等读取再次撤销其他有效代际。
- client/安装不匹配时，在任何副作用前拒绝；不得借不匹配请求撤销另一安装。匹配绑定的已消费旧 token 改用另一 rotation，按重放攻击撤销其自身家族；不影响其他安装或只读/Pi 家族。
- 真正 COMMIT 确认丢失时，只按原 lineage/rotation 查询已提交结果；不得假定回滚并再次签发。并发相同或冲突操作均最多产生一个后继。
- 恢复密钥按 key ID 管理，活动密钥与旧解密密钥分离；保留所有尚在窗口内回执所需密钥。缺密钥、错误密钥或密文/AAD 损坏须明确不可用，不生成新后继、不释放预算、不降低验证。不得使用邮箱、Web 会话或生产环境的其他用途密钥。

响应缓存可在窗口结束后由有界、显式授权的维护操作清理；仍需防重放、撤销、计费或审计的 lineage/tombstone 保留。缓存过期不等于删掉家族或账务证据。维护可观测且不能由普通启动隐式执行 DDL。

## 数据库与兼容

既有 69 组件的历史 SQL、封印源码快照和历史证据保持原样。新增状态及既有 client/source 约束的扩展通过追加组件及精确 69→70 owner 升级完成；不能靠普通启动修复目录摘要或放宽未知前驱。新装、显式升级、运行角色授权、目录合同、撤销脚本与提交不明回执共同更新，旧 grant、资金、ACL 和回执不得丢失。此前 69 组件验收不自动覆盖新组件。

Magpie 纳入尚未发布的第 70 组件，必须更新这一候选的目录摘要和准入材料，并重新验证新装、69→70、权限及两个托管客户端的隔离。此前仅覆盖 CC Switch 的第 70 组件候选证据保留原输入身份，不能改名为包含 Magpie 的通过证据。

## 必须独立验收

真实隔离 PostgreSQL、受限 runtime 和 Core HTTP 覆盖浏览器同意、code 交换、实时状态、模型有限预算、跨安装/跨 client 拒绝、撤销及旧只读/Pi 兼容。轮换另覆盖提交后丢响应、同操作并发、不同操作重放、599/600 秒边界、后继再消费、撤销/权限变化、重启及密钥轮换/缺失/损坏，回读仅一个后继、原预算与窗口、账务不变及秘密不泄露。

新装和精确升级分别验证真实 owner CLI、受限角色、旧数据/ACL、错误前驱及提交丢回执恢复。最后使用真实受信任 QA HTTPS、独立测试身份、系统凭据库和实际 CLI 完成授权、add-only 接入、sync、响应/凭据保存失败恢复及配置回执恢复；保留用户选择，不激活付费模型调用。合成 Store/Exchange、结构检查或安装成功均不能替代该端到端结果；平台、组合资源、真实供应商与生产切换分别列明。
