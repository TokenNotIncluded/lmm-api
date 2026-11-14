# Magpie 原生模型客户端

这是新增能力的行为与验收合同，不声明当前实现或生产部署已经完成。Magpie 通过其官方 provider 插件接口接入 LMM；不要求用户复制 CC Switch、Pi、浏览器或其他提供商的凭据，也不新增服务端守护进程。

## 来源与接入

- 目标产品为 [yetone/magpie](https://github.com/yetone/magpie)，官方站点为 [usemagpie.ai](https://usemagpie.ai/)。公开协议参考其 [provider 插件文档](https://usemagpie.ai/docs/plugins)。观察基线是 `v0.1.2512`、提交 `7da5b34aa497858bef9b6823a72d499ab375e055`；其 [MIT LICENSE](https://github.com/yetone/magpie/blob/7da5b34aa497858bef9b6823a72d499ab375e055/LICENSE) 属于上游，复用代码时保留归属。单纯对接公开协议不声称复用了上游实现。
- 提供独立的原生 LMM provider：浏览器 OAuth 授权、模型发现、带 Bearer 的模型调用、刷新与额度显示使用官方插件生命周期。通用 URL/API Key 导入可以保留，但不能代替本合同的原生授权验收。
- 使用 `client_id=lmm-magpie`、`application=magpie`，遵守 [托管应用合同](MANAGED_APPLICATIONS.md) 的 issuer、PKCE、回环回调、安装绑定、明确同意、最小 scope、撤销和六百秒恢复。配置与发现中显式区分 Magpie、CC Switch、Pi 和只读客户端。
- Core 以注册的 client/application 组合生成执行计划；插件自报的名称、安装或额度不授予权限。未配置 Magpie 上限、必要数据库权限、恢复密钥、目录或网关时不发布 Magpie 能力。开启 CC Switch 不能隐式开启 Magpie。

## 预算与额度读取

- 服务端单独配置 Magpie 的有限预算上限；用户在同意页明确选择本次授权的额度与分组。预算单位为 `credit_500k_usd`，必须为 1 至 9007199254740991 的整数，不能默认无限，也不能把 credits 当美元展示。
- 按 grant/family 统计完整生命周期内的已消费和待处理预留，所有 access/refresh 代际共用同一额度。不同应用、安装、家族之间不能转移或复用额度；账户余额、现有策略与实际价格仍由 Core 统一裁定。
- 新增只读 `GET /api/oauth2/application/budget`。只接受当前有效托管应用 Bearer 及 `usage:read`，无调用者指定 grant/用户参数。返回 `contract=lmm.application-budget.v1`、`unit=credit_500k_usd`、真实 `client_id`、`application`、`installation_id`、`grant_id`，以及整数 `budget_credits`、`consumed_credits`、`reserved_credits`、`remaining_credits`。所有数值必须精确且处于 JSON 安全整数范围；剩余额度与实际准入使用同一预算口径，不能另造客户端扣款规则。
- 预算读取采用一致的权限与账务快照，重验当前用户、scope、family、安装、到期和撤销状态，只暴露该家族的投影；不调用上游、不刷新 token、不预留或扣款。未知结算仍计入待处理占用，不能伪装成零消费或释放额度。证据损坏或暂不可用须返回明确失败。
- Magpie 的 `auth.usage` 展示这份真实家族预算，明确是授权生命周期额度，不伪造每日/月度重置时间。不可用显示错误或 unknown，不能显示成 0% 已用。既有 `/api/oauth2/application`、token JSON 及 CC Switch/Pi 响应形状保持兼容；不得把近段时间的用量列表当终身预算。
- 到达额度、权限缩小、撤销或过期时，Core 拒绝后续模型准入；插件显示具体可恢复状态，不轮换其他账户或应用凭据绕过。普通安全刷新不增加预算；新增额度必须重新取得用户明确同意。

## 本机状态与刷新

- 使用 Magpie 官方的账户级 auth store 和钩子；多账户状态以 issuer/client/installation/grant 绑定，不能只按 provider 名称共享模块全局 token。保留原有其他 provider、账户、所选模型及用户配置。
- 在发出首次轮换前，把随机 rotation ID、本地开始时间及恢复原操作所需状态持久化到该账户可恢复的私有存储；目录 0700、秘密文件 0600，拒绝软链接、路径穿越或不安全权限，原子保存并同步落盘。不能把日志、URL、插件 metadata 展示或普通配置导出当秘密存储。
- 宿主的普通刷新锁、15 秒等待或失败重试不等于 LMM 的六百秒恢复。插件必须在响应丢失、宿主/插件重启、保存失败或并发调用后恢复同一 rotation；缺少持久化确认时不发送新的轮换，不借宿主重试换用新 rotation，也不把待确认状态当已成功。
- 只有在服务端实时状态、凭据与安装绑定均验证成功后，才向模型端点发送请求。关闭、超时、取消或过期的授权不会在迟到回调中保存成功或替换当前账户；不能在未验证的重定向、跨源 URL 或另一账户请求中发送 Bearer。

## 必须执行的验收

1. 真实隔离 PostgreSQL、受限 runtime、Core HTTP 验证 Magpie 新装授权、有限额度、usage 读取、预留/消费、耗尽、撤销及权限变化；并发请求不能超预算，失败/取消/UNKNOWN 的占用符合真实账务合同。
2. Magpie、CC Switch、Pi、只读及同应用多安装交叉测试，证明 token、rotation、状态和额度均不能串用；旧客户端响应及既有授权仍兼容。
3. 第 70 组件新装及精确 69→70 owner 升级保留历史数据、资金、ACL 与回执，缺权限、错误前驱、错误 client/application 组合与提交不明均明确拒绝或沿原操作恢复。
4. 使用上述固定提交的真实 Magpie、独立私有 HOME/账户、受信任 QA HTTPS 与 Core，完成官方插件加载、浏览器 PKCE、目录发现、真实预算展示、撤销、重启及同操作刷新恢复。实际用户配置和凭据不作为夹具，供应商不发真实付费模型调用；如需模型传输验证，使用明确隔离的合成上游。
5. 分别记录单元、宿主协议、真实客户端、数据库、受信任 TLS、故障与平台结果。模拟宿主或复制 auth JSON 不能替代实际 Magpie 验收；未执行和 UNKNOWN 不记为通过。
