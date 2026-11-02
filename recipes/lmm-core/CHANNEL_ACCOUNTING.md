# Channel balances, consumption and copies

Core owns channel identity and publishes configuration, independent encrypted
credentials and display snapshots in the same audited transaction. Deleting a
channel retires its ID permanently. Native admission also reserves primary,
fallback and image channel IDs inside its existing reservation transaction,
before funding writes. An unfunded admission rolls these metadata writes back;
later settlement or refund of an admitted request remains available after retirement. An explicitly requested previously used ID
is rejected; generated copies allocate above every recorded ID. Sensitive
configuration reads expose the read-only `channel_id_high_water` hint so the
create form proposes a free ID after deletions. The hint is not publication input
and never substitutes for the transactional uniqueness check.

## Consumption

`GET /core/v1/admin/routing/channels/{id}/usage` requires the current
administrator Session and ChannelRead permission. Optional `start_timestamp`
and `end_timestamp` are Unix seconds; the default and maximum window is 31 days.
The Native projection validates frozen request provenance, actual dispatch,
settlement commands and refunds in one SQL snapshot. A failover is attributed to
the dispatched upstream, not today's routing configuration. Holds and uncertain
requests are reported separately from settled consumption.

`GET /core/v1/admin/routing/channels/{id}/counters` also returns lifetime Native
consumption, an immutable inherited display baseline, their confirmed subtotal,
and the cached provider balance. Window consumption excludes inherited baselines.
Credit amounts and counts are decimal strings; 500000 Native credits equal one
USD. Provider balances retain their configured unit and are never converted into
credits, tokens or supplier cost.

`historical_coverage=false` preserves the missing historical opening. In that
case `used_quota` is null and the combined amount is only the confirmed subtotal.
Missing provenance and a reused historical ID prevent a completeness claim.
Native channel totals and windows must remain available with a million valid
receipts, including a small channel sharing a store with a much larger one.
An owner rebuild validates every original receipt without a global request or
per-request receipt truncation. Live updates share the existing funding
transaction, and reads use the derived channel/time projection. Unready or dirty
projections fail explicitly; reads and startup never repair them. General usage
pagination and other statistics retain their independent acceptance scope.

## Reviewed historical counters

Legacy `used_quota` is a signed 64-bit recorded counter. It can contain refund
adjustments and is neither a proven gross amount nor a complete financial net.
Preserve its original integer, verified historical unit, source archive/export,
mapping, observation time and unit evidence. Today's configuration and a log's
unit do not prove the accumulated counter's unit. Missing evidence stays unknown.

The owner workflow binds the exact source material and target snapshot, shows a
reviewable plan and performs one explicit apply. Source coverage must distinguish
disjoint counters from counters including Native amounts at the seal. Deduplication
uses the exact opaque Native request ID set and confirmed Native display amount,
including inherited Native baselines; timestamps, ID prefixes and maximum IDs
are not cutover evidence. Replay keeps the original seal and offset. A changed
target or unproven overlap rejects confirmation; an uncertain submitted result
stays UNKNOWN until resolved from the original receipt.

An additive `lifetime` view exposes `basis`, `complete`, `total_usd`, exact decimal
string `exact_usd_numerator`/`exact_usd_denominator`, and nullable `legacy_opening`.
The historical offset and Native net are added as exact rational amounts and
rounded only once for the displayed USD string. Negative and unsafe-in-JavaScript
integers remain exact. Original Native credit/gross/refund fields keep their
Native meaning. Incomplete totals are null, not zero. This does not authorize a
wallet import or prove missing historical provider, sorting or statistics parity.

Copies with preservation retain the same historical rational offset and capture
the source Native display subtotal once. Copy chains do not add the opening
again; a subsequent source refund cannot change an earlier child's display.
Reset copies inherit neither opening nor counters. Imported facts are owner-only
and immutable; runtime copies preserve the parent's source/unit/offset and cannot
promote unknown coverage. No opening or copy adds to global financial totals.

## Provider balance

`PUT /core/v1/admin/routing/channels/{id}/balance-config` takes
`{expected_version,probe}` under the existing sensitive-write permission.
`probe` is null or `{path,json_pointer,unit_label,subtract_json_pointer?,require?,require_all?,auth?,cny_per_usd?,deepseek_currency_array?,openai_subscription?}`: a
same-origin relative GET path, an RFC 6901 JSON pointer, an optional unit label,
and optionally a second RFC 6901 pointer. When the second pointer is present,
the observed finite number is the first selected numeric value minus the second;
missing, invalid, non-finite or credential-reflecting values fail without changing
the stored observation. Optional `require` is `{json_pointer,equals}`; the
pointer must select exactly the supplied non-null JSON boolean, number or bounded
string before any balance is accepted. Missing or unequal status fails without
changing the observation. Optional `require_all` is a nonempty array of at most eight such requirements; every predicate must match, in addition to `require` when supplied. Each pointer and scalar has the same validation and credential-confidentiality rules. This allows a Moonshot-shaped response to require both `/status == true` and `/code == 0`. An optional explicit positive finite decimal-string `cny_per_usd` divides the selected CNY value by that operator-supplied rate and rounds to 16 decimal places as the legacy provider conversion did; it requires `unit_label` to be `USD` and cannot be combined with subtraction. The rate is publication configuration, not an inferred historical rate or financial ledger conversion. Optional `deepseek_currency_array: true` requires `json_pointer: /balance_infos`, USD output and no subtraction. It selects the first USD entry (ignoring surrounding currency whitespace/case); otherwise the first CNY entry requires an explicitly supplied positive `cny_per_usd` and is converted as above. Selected amounts must be finite nonnegative strings; an invalid first USD entry is not skipped in favour of CNY. Optional `openai_subscription: true` selects an explicit legacy OpenAI-style two-GET plan: first `/v1/dashboard/billing/subscription`, then same-origin `/v1/dashboard/billing/usage` with generated `start_date` and `end_date` in the host local calendar. Start is the first of the current month when `has_payment_method` is true, otherwise 100 calendar days earlier; end is today. It computes `hard_limit_usd - total_usage / 100`, requires explicit USD output, Bearer auth, those fixed fields and paths, and rejects malformed or non-finite responses without altering the previous observation. No user-controlled query, redirect, credentials or arbitrary second URL is allowed. It cannot be combined with other selectors or transformations. No automatic adapter discovery is implied. No credential may appear in this configuration. For
example, a manually configured SiliconFlow-shaped probe may require `/code` to
be the number `20000` and read `/data/totalBalance`; this is not automatic
provider discovery. Optional `auth` may be `api_key` to send the stored credential
as `Api-Key` instead of protocol-default authentication; arbitrary header names,
credential values and query secrets are not accepted. An AIProxy-shaped response
can be configured with `auth:api_key`, `/success == true` and
`/data/totalPoints`, without making any external call during configuration.
Existing probes omit these optional fields and retain their meaning. This explicit plan can express OpenRouter's credits minus usage on a
configured same-origin endpoint, but is not automatic provider-specific balance
discovery or complete legacy adapter parity. A publication containing these new
fields requires a Core version that understands them; older Core versions must not
be used to recover that publication.

`GET /core/v1/admin/routing/channels/{id}/balance` requires ChannelOperate and
uses the channel's existing server-side credential. The request uses the normal
gateway's endpoint, timeout and body limits. No database transaction or actor
lock spans network I/O. The second authorization and publication fence reject a
stale configuration or key before updating the observation and audit atomically.
Failure leaves the previous observation intact. The legacy
`GET /api/channel/update_balance/{id}` retains its numeric `data` and adds the
typed observation. An absent configured probe fails explicitly.

Native observations carry a nullable round-trippable balance string, unit,
server observation time, kind, source channel and record time. Missing values and
times remain unknown. A real zero remains visible. Neither the browser clock nor
a copied/reset snapshot represents a fresh provider observation.

## Copy semantics

Native `POST /core/v1/admin/routing/channels/{id}/copy` accepts
`{expected_version,suffix,reset_balance?}`; the flag defaults to true. Legacy
`POST /api/channel/copy/{id}?suffix=...&reset_balance=...` retains its default
`_复制` suffix and explicit empty suffix. Copy always creates an independently
encrypted credential, published routing bindings and metadata.

With `reset_balance=true`, the new display balance and consumption start at
zero. With false, Core captures the source's cached balance and confirmed gross
and refunded display counters. Both retain the original balance observation
time, including an unknown time; the snapshot kind records reset or copy.
Unknown historical coverage remains unknown. Later charges, refunds or balance
refreshes on either channel do not rewrite the other's inherited snapshot.

For older managed Core versions, the Web default copy omits the new optional
flag. Preserve-copy and new balance/statistics actions require fresh, explicit
backend capabilities; absent capabilities never imply support.

Neither mode creates a charge, refund, wallet entry or subscription usage.
Global financial statistics continue reading the original ledger once. Failed
CAS, audit or publication rolls back the new ID, baseline, observation and key.
An unknown commit result retains the existing explicit recovery behavior and
must not trigger an automatic second copy.

## Installation and evidence

The current 44-component manifest is installed only by explicit fresh
initialization or the exact offline upgrade from frozen PostgreSQL 18
predecessors 43, 42, 41 or 32. Component 43 and its frozen 42→43 contract remain
unchanged predecessor facts; component 44 adds the channel projection and
reviewed opening facts without replacing prior DDL. Runtime startup performs no
DDL and cannot rewrite inherited baselines, delete identities or access
owner-only upgrade receipts.

The change retains the full completion contract. Local synthetic database/API
checks, Web tests/build, independent acceptance, browser rendering, historical
migration and production deployment are separate evidence. Tests are not proof
that all old provider adapters, historical channel openings or production
cutover have passed.

## 旧渠道类型的余额探测

保留明确导入或管理员设置的旧渠道 type；没有 type 时不从域名、协议或密钥猜测。手工 probe 配置优先；显式清空同时停用自动计划。按固定旧源 type 1/8、10、12、13、20、25、40、43 分别选择 OpenAI/custom subscription、AIProxy、API2GPT、AIGC2D、OpenRouter、Moonshot、SiliconFlow、DeepSeek 的查询合同，未知、退休及不相容认证协议返回不支持。复制保留类型选择，删除移除选择，观察前后重验发布版本及最终计划。

所有请求仍限管理员配置的同源目的地，保留认证头、成功状态、大小/超时、秘密保护及失败时旧观察不变的约束。AIProxy 固定外部域名和 Advanced Custom 完整请求能力不由本切片声明支持。OpenAI/OpenRouter 为 USD，AIProxy 为 points；旧源未证明币种的 API2GPT/AIGC2D/SiliconFlow 保留未知。Moonshot 无显式汇率时显示 CNY，有明确正 CNY/USD 比率才展示 USD；DeepSeek 优先 USD，只有明确比率才换算 CNY fallback。显示转换不能写账本、改变历史资金或推断历史汇率。

验收以真实隔离库和本地 HTTP 上游验证自动选择、手工覆盖、重载/复制/停用、拒绝、发布并发及观察保存；静态映射单元测试不代替这些链路，也不代表实际外部供应商已经验收。


## 列表排序与探测观测

历史列表的余额、累计消费、响应时间和测试时间排序必须使用可信来源。累计消费保留原完整性判断，以未舍入的精确值比较；缺少完整历史的值进入未知组，不当零。余额只在明确相同单位内比较，混合 USD/CNY/积分时要求选择单位，不隐式换汇。未知值无论升降序均放末尾，相同值固定以渠道 ID 升序。观测须保留真实发生时间、来源、凭据版本和适用范围；复制余额不更新其真实观察时间。

当前安全渠道测试只是模型目录探测，响应时间和测试时间必须明确标注这一范围，不宣称已完成模型推理。通过独立 channel-observations 组件保存目录探测结果、时间、耗时与原凭据版本，和资金账本、路由发布分离。失败或未知结果不得造出成功耗时；换密钥后的旧结果不得冒充当前密钥有效性，复制不制造新探测记录。

过滤、统计读取、排序与分页在同一一致数据库快照下完成，批量读取，不逐渠道扫描请求历史。Native 列表返回绑定排序/过滤和观测数据的快照摘要；后续页携带原摘要，数据变化时明确拒绝旧快照并刷新分页，不静默重排。未携带摘要的旧客户端只保证单请求内一致，不能宣称跨页稳定。验收覆盖同值、未知、多单位、过时及复制观测、并发变化、分页边界和权限拒绝。
