# 原生 Key 只读合同

固定来源 services/core/src/native_key_read.rs。两个 GET 返回 Cache-Control: no-store，不写身份、钱包或授权。

| 条件 | HTTP | JSON error |
| --- | --- | --- |
| 缺少/无效/重复 Authorization 或别名认证头 | 401 | invalid_api_key |
| IP 不允许/无可信 peer | 403 | ip_not_allowed |
| 未同意余额访问 | 403 | account_balance_access_required |
| 读请求超限 | 429 | quota_query_rate_limited |
| 余额依赖不可用 | 503 | account_balance_unavailable |
| usage 依赖不可用 | 503 | quota_query_unavailable |

失败对象为 {"valid":false,"error":"<上述代码>"}。成功不是假的零值；unknown 不按余额 0 展示。GET /v1/usage 成功至少含 valid=true、scope="token"、currency="USD"、remaining、used_today、used_total、used_quota、normalized_used_quota、usage_projection_available=true、total_quota、unlimited、updated_at、day_timezone="UTC"、used_today_source="retained_consumption_logs"、consistency="persisted_snapshot"。used_quota/normalized_used_quota 为整数 credits；美元字段按每 USD 500000 单位换算。unlimited 时 remaining 与 total_quota 可为 null，不能误判为零余额。UTC 日界限与历史保留日志来源必须清楚。

余额 V2、读同意、IP 验证、legacy/native 余额来源一致性和 Key 有效性必须结合真实 provider/CLI 联合验收，不能只测试上述 JSON 样例。未提交 /v1/pricing 与共享 ReadQuota 变更不纳入本轮固定版本。
