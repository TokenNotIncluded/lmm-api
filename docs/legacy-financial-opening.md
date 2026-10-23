# 历史余额 opening 导入

`migrate-legacy-financial.py` 的 opening 路径仅支持**经独立审核、全用户预留为零的期初余额**，写入一个全新、离线、由安装 owner 控制的目标库。
入口是 [财务脚本](../scripts/migrate-legacy-financial.py)，具体约束在
[opening 模块](../scripts/legacy_financial_opening.py)。普通 `--apply` 仍明确拒绝；其他金融领域仍由 planner 保留和分类。

当前真实 06 批次仍为 `BLOCKED`、`target_writes=0`：没有可用的完整零预留审阅材料和已审核 opening 输入，旧工具指纹也不能直接作为当前授权。
隔离合成库的有限验收不代表真实历史资金、历史 MFA、生产迁移或[完整仓库验收](architecture-completion-prompts.md)完成。

可复现的合成验收入口是 [CLI 验收脚本](../scripts/test_legacy_financial_opening_cli.py)。
它只创建新的本机 PostgreSQL 集群与合成用户，接受明确的冻结源码和对应 Core 二进制指纹，
覆盖预览、审核材料、整批写入、严格重放，以及真实 COMMIT 成功后丢弃 psql 确认的原 attempt 对账。
验收只适用于记录中的源码、schema 和二进制；后续 Pi schema 的签字不能复用旧目标或旧 approval。

## 可选：同事务保留原始历史计数

三个 opening 模式可显式增加 `--include-opening-counters`，选择独立的
`financial-opening-with-historical-counters` profile。默认 opening 仍只写余额；
普通 `--apply` 仍拒绝。新 profile 不能用于 source-rights preview，也不能为已使用的旧目标补写计数。

该 profile 要求新的 reservation review：`format_version=2`、
`scope=opening-with-historical-counters-economic-policy-review`，
`included_families=["wallet-opening","historical-counters"]`，
`excluded_families` 为原排除集合去掉 `historical-counters`。
`historical_counter_policy=preserve-source-integers-no-pricing-or-usage-normalization`，
`counter_source_facts_sha256` 绑定完整逐账户来源用户、个人账户、原行 hash 与
`used_quota/request_count/aff_history` 的带类型向量。其余全表、全用户零预留审核保持不变；
仅填写 hash 或状态不能证明经济事实。

新 approval 使用版本 `3` 和上述独立 scope，另绑定完整 counter payload、来源事实向量和
包含实际 counter 原行的完整读回；旧版本 `2` 的批准不能授权这个 profile，反向混用也拒绝。
预览额外输出 `opening-counter-payloads.private.json`。apply 在同一 SERIALIZABLE 事务中
先完成全部钱包 opening，再调用 owner-only `import_account_opening_counters`，计数不额外取号或造账。
严格 replay、CAS、所有无关表/序列守恒与原 attempt 的 UNKNOWN 对账也同时包含这些计数。
执行和对账必须保留相同显式 profile，不可用更换模式或 migration ID 处理不确定结果。

计数始终保留来源有符号整数，不归零、不按当前价格换算、不伪造模型请求或历史充值。
非零历史 used 的归一化金额仍为 unknown；Key、订阅、历史支付退款、托管和其他权利仍在排除范围。
该路径已在独立的新 PostgreSQL 合成目标验证正常 apply、严格 replay、丢弃真实 COMMIT 回包后的原 attempt 对账、拒绝与 CAS，以及后段故障的业务整笔回滚。
故障实验仅在实际发送的 SQL 中显式加入一条 RAISE；原 SQL、差异和序列前进记录分别保留。
这些结果绑定当次 13 组件 schema、冻结工具和 Core 二进制，不是后续 schema 组合或所有资金流程的签字。
真实 06 批次的完整经济审核、source 单位证明和全新离线目标要求没有因此完成。

## 先定位待处理的经济权利

`--source-rights-preview` 只读取已经核验的冻结导出和身份映射，列出钱包、债务、affiliate、订阅、Key 额度、悬赏托管等来源事实，帮助逐项审核。它不查询来源或目标数据库，不生成 opening payload、approval 或零预留证明。

该模式要求 `--reuse-export-proof`；不能同时选择三个 opening 模式，也不能传目标 receipt、预留审核或 opening approval/attempt。其他共用来源参数与 planner 一致，输出仍须为 `~/Backups` 下一个新的私有目录。凭据和用户事实不放进 argv 或仓库。

详细结果保存在 `source-rights-reconciliation.private.json`，manifest 绑定它的实际文件 hash 和类型 hash，以及当前工具源码。该模式保留冻结来源引用、字节 hash、source proof 和非秘密投影，不生成包含完整原行的 `financial-full-source-manifest.private.json`，避免额外复制 Key bearer；manifest 明确 `full_source_manifest_created=false`、`raw_source_rows_copied=false`。原始完整行继续在原冻结私有导出中保留；普通金融 planner 的私有 full-source DTO 协议本轮不变。汇总按表、字段、观察状态分别统计，不把重叠来源或不同币种相加。未知单位、缺失关联、未映射用户和未投影的表继续保留为待审核。核对范围仅限这份选定导出，来源 catalog 中未导出的表也不因此获得放行；`reserved_zero_proven=false`、`financial_rows_migrated=0` 是正常结果，命令成功不代表经济审核或迁移成功。

非零托管金额必须单独保留其归属、原始单位和后续释放/退款权利；不能加入钱包余额、再扣一次钱包，或因为将其排除在 opening writer 之外就视为已处理。来源行定位、状态名称或代码版本本身都不能证明历史部署执行过某种扣款规则。

源码定位入口：`markref context migration.financial-rights --max-bytes 8000`。同一标签串联 CLI、离线投影和测试，不能代替来源审核。

### 悬赏金额的缩放核对

离线预览的 `bounty_rebase_lineage` 将冻结的悬赏项目、挑战和争议与原 `wallet_credit_rebases` 母计划核对。它按固定版本的历史规则重新生成允许的金额变更，验证精确比例、舍入、归属及完整来源字段，区分当前记录匹配变更前、变更后、两者、混合、缺失或快照后变更。

匹配变更后值只证明当前记录与母计划一致，不证明历史审批、实际部署版本、COMMIT 或美元单位已经验收。零值及舍入后未变的金额同时匹配前后值，不能单独作为已缩放证明。当前金额不再缩放，平台费、tip 和争议历史快照不随项目金额一起改写；历史拒绝记录也继续作为申诉守卫保留。

输出中的明确 blocker 和未知权利仍须逐项处理。该核对不生成导入 payload 或释放托管的授权，不能替代原子导入与付款、退款、争议恢复的独立验收。

源码与独立反例入口：`markref context migration.bounty-basis --max-bytes 8000`。

### API Key 剩余额度与历史用量的独立对账

`source-rights-reconciliation.private.json` 的 `key_quota_reconciliation` 在冻结导出校验后，
从 `tokens` 原行重新计算并逐类型核对 planner 的完整 `key_limits`，绑定来源 Key、原用户、
目标个人账户、来源行 hash 和导出位置。缺项、多项、重复、改归属或改计数的计划即使重新计算了 hash 也拒绝；
未映射 owner 保留原行事实并标记 blocker，不改绑其他账户。不会导出 Key 密文或把来源限制当成新授权。

每条 Key 独立保留 `remain_quota`、`used_quota` 和 `unlimited_quota`，单位仍是
`source-field-unit-unproven`。`display_basis_integer = used_quota + remain_quota` 仅为原整数的
显示基数核对，不是美元换算或钱包金额；无限额 Key 也保留有符号的显示基数，但候选 lifetime cap 为 null。
负的剩余额度不截断为零，负历史用量、有限 cap 越界及显示基数越界继续阻断。
Key 之间及 Key 与用户的历史用量可能重叠，不汇成新资金，也不再次扣钱包。

离线函数 `legacy_financial_reconciliation.build(..., target_key_inventory=inventory)` 接收目标库
**只读导出的完整迁移范围 Key 清单**，不建立数据库连接。JSON 对象固定为
`{"format_version":1,"scope":"legacy-api-keys","records":[...]}`。每行必须且只能包含：
`source_key_id`、`target_key_id`、`account_id`（均为正整数），`status`（整数 1 启用、2 停用、
3 过期、4 耗尽），`deleted_at`、`expired_time`、`unlimited`、`remaining_units`、`used_units`、
`lifetime_limit_units`、`secret_sha256`、`restrictions`。`secret_sha256` 是原 bearer UTF-8 字节的
小写 SHA-256，不是来源行 hash；导出中不得含 bearer。`restrictions` 包含原字段
`model_limits_enabled`、`model_limits`、`allow_ips`、`group`、`auto_groups`、`cross_group_retry`、
`oauth_managed`、`account_balance_read`、`one_time_reveal`、`creation_source`，缺失的来源字段按 null 比较。
该格式保留原整数与状态语义；实际目标 schema 的只读适配及导出范围完整性仍需独立审核，不能从计划伪造目标清单。

目标比较独立投影来源行，不从 planner 的 `map_key_limits` 生成目标期望值；共用 mapper 仅检查
来源与计划是否一致。逐条比较 owner、状态（含删除与过期）、所有额度与限制、secret hash，输出
`matched / missing_in_target / unexpected_in_target / field_mismatch`；匹配或字段不符的行均绑定实际
`target_key_id`。任一非 matched 使 `key_reconciliation_passed=false`、顶层状态为
`failed-key-reconciliation`，并保留逐条差异字段名，不复制可疑目标字段值。未提供清单或缺项也是失败；
只有缺失目标的记录才没有目标 ID。

`--source-rights-preview --target-key-inventory PATH` 可读取这一份已独立准备的离线 JSON；该参数不能用于
普通 planner 或三个 opening 模式，也不会连接目标数据库。文件沿用 `~/Backups` 私有路径规则，拒绝
符号链接、不安全权限、重复 JSON 属性和非有限数。通过格式及已知 bearer 检查后，原始字节保留为
`target-key-inventory.private.json`；manifest 分别记录 `target_key_inventory_file_sha256` 与
`target_key_inventory_typed_sha256`，前者绑定文件字节，后者绑定带类型的内容。
`target_key_inventory_provenance_verified=false`、`target_inventory_scope_completeness_verified=false`：
本地 hash 不能证明目标导出来自哪个数据库，或导出范围完整。

提供清单后，任一缺失、多余或字段不符使命令退出 `1`，同时保留完整对账报告与 manifest；格式或
私有路径检查失败则退出 `1`，记录 failure，且不生成目标清单副本。未提供清单时仍可成功生成来源预览
并退出 `0`，但 Key 状态保持失败。stdout、report 与私有对账均明确给出 `key_reconciliation_passed`
和状态；全部 matched 也只到 `blocked-for-opening-economic-review`，不能把退出 `0` 当作迁移验收。

不同来源 ID 共用 bearer、缺失/空 bearer、未知或非整数 status 均直接拒绝，错误信息不包含原值。
该模式先在内存中完成所有输出的已知 Key bearer 检查，再写出派生文件；检查包括 plan 中的来源限制与
删除状态、source proof 的路径及元数据、目标清单、最终 manifest 和序列化内容。夹带会以固定错误码拒绝，
输出只保留无秘密的 failure，不写出提前生成的 plan 或 proof 副本。发布阶段逐文件校验字节 hash；
多文件输出不是原子事务，I/O 失败时可能留下已经检查过的部分文件，须保留失败证据。
原始冻结导出仍是私有敏感材料。这是合成纯函数及 CLI 文件路径测试覆盖的**离线对账边界**，
不是 PostgreSQL 导入或运行进程验收。
Core 已有 `core_charging.budget_opening_consumption` 的历史用量预算合同；本片不写该表，
也不生成其导入 payload。`credential_activation_allowed=false`、`target_writes=0`；即使所有导出字段
matched，也不授权激活或证明单位与历史部署规则正确。实际导入的原子性、重放、故障恢复、运行时有效权限
及不重复计费仍待后续独立验收，06 批次保持 BLOCKED。

实现与反例沿用 `migration.financial-rights`，开发者测试见
[`scripts/test_legacy_key_reconciliation.py`](../scripts/test_legacy_key_reconciliation.py)。

### 钱包、债务和返佣的目标比较

`--source-rights-preview --target-wallet-inventory PATH` 可增加一份独立准备的私有目标观察，
逐用户比较钱包、债务和返佣三个桶。此参数只用于离线来源预览，不连接或写入数据库，
不能用于普通 planner 或 opening 模式。文件遵守同样的 `~/Backups` 私有路径和 JSON 检查规则。

清单根字段为 `format_version=1`、`scope=legacy-wallet-positions`、`unit=credit_500k_usd`
及 `records`。每条记录只含 `source_user_id`、`account_id`、`wallet_units`、
`debt_units_signed`、`affiliate_units`；身份必须是正整数且用户、账户各自唯一。
钱包不得为负，债务不得为正，返佣保留有符号整数；布尔值或浮点数不能充当整数。
不得从迁移计划生成这份目标观察来代替真实读回。

对账直接从冻结来源整数计算期望值，并与已有的来源到计划核对组合使用；逐条输出
`matched / missing_in_target / unexpected_in_target / field_mismatch`。
提供清单后，任何非 matched 使 `wallet_reconciliation_passed=false`，命令退出 `1`，
保留差异报告。未提供清单仍可生成来源预览，但钱包比较不能判为通过。
完整结果位于 `wallet_position_reconciliation`，清单字节与带类型内容的 hash 分别绑定到 manifest。

即使全部整数匹配，状态也仅为 `matched-source-integers-only`：目标观察的数据库来源与完整性、
来源单位、预留和托管权利仍未证明，`target_state_complete=false`。
这不生成经济批准或导入 payload，也不替代真实 PostgreSQL 迁移和独立业务验收。

目标清单可用 [`export_financial_target_inventory.py`](../scripts/export_financial_target_inventory.py)
从已审核的本地 Unix PostgreSQL 目标实际读取。它要求 `--target-receipt`、`--identity-manifest`、
`--identity-map` 和新的私有 `--output` 目录，在同一只读事务快照内核对实例、schema、身份导入
receipt、账户归属和实际余额。输入文件、密码文件及工具源码在读取前冻结，输出前再次校验；
查询期间发生变化就拒绝发布清单。

完整的映射内观察退出 `0` 并写出 `target-wallet-inventory.private.json`。缺桶、缺余额或单位错误
保留不完整观察并退出 `2`，不会补零或生成可消费清单；实例、输入或连接不符退出 `1`。
该工具只观察指定身份映射范围；完整托管权利和预留读回由组合导入另行校验。

## 支持范围

单位固定为整数 `credit_500k_usd`，`1 USD = 500000 credits`。
来源 `users.quota` 的正值进入 wallet，负值保留为 debt；`users.aff_quota` 独立进入 affiliate，不转成可消费余额。
不再扣一次历史 used，也不通过重放支付回调或伪造模型消费补余额。

预留证据必须覆盖冻结映射中的每个用户，以及冻结财务导出中的每张选定表。非零 reserved、pending transfer、未知权利或缺失覆盖均拒绝整批。
没有 pending 分类不能证明 reserved 为零；金额为零也不能代替审核。当前 opening 不恢复原 reservation 的 capture/release 操作权利。

历史计数、Key 额度、订阅、旧支付退款基准、escrow 和在途权利不在这个 writer 的迁移范围内。
opening journal 不授权历史退款，也不激活原来停用的身份。空 opening 批次暂不支持。

## 三个入口与参数

三个 opening 模式互斥。不选择它们时，脚本沿用财务 planner 流程。
参数名称可用只读帮助核对：

```sh
python3 -B scripts/migrate-legacy-financial.py --help
```

| 模式 | 行为 | 额外要求 |
| --- | --- | --- |
| `--opening-preview` | 重新核对冻结来源和目标，生成待审核的 opening candidate；目标写入为零 | 下表中的全部共用输入 |
| `--apply-opening` | 只执行与重新计算结果完全一致、已单独审核的 opening 批次 | `--opening-approval` |
| `--reconcile-opening` | 只确认原不确定 attempt 的结果，不重试导入 | `--opening-approval`、`--opening-attempt` |

所有 opening 模式都需要以下参数；文件参数只能指向已准备、受保护的本地材料。
这里列参数而不提供自动生成 approval 或初始化目标库的命令。

| 参数 | 必须指向或表达的内容 |
| --- | --- |
| `--source-receipt` | 冻结来源 receipt，绑定原始 archive、导出和元数据 |
| `--identity-map` | 完整、已审核的 source user 到原生 user/personal account 映射 |
| `--identity-manifest` | 与同一来源、同一映射文件绑定的 identity-only manifest |
| `--reuse-export-proof` | 原冻结财务导出的 proof；opening 不重新查询来源库 |
| `--target-receipt` | 资源 owner 为这一个新离线目标准备的身份、合同、前态和独占 lease receipt |
| `--reservation-evidence` | 全表、全用户的真实预留核对证据文件 |
| `--reservation-review` | 独立审核记录，绑定上述证据的实际文件字节、来源、映射与排除范围 |
| `--migration-id` | 本 opening 批次固定 ID，必须与其引用的 identity import ID 不同 |
| `--output` | `~/Backups` 下一个尚不存在的新输出目录，权限为 `0700`；每次预览、执行或对账各用新目录 |
| `--source-timezone` | 可选：有证据的历史应用 IANA 时区；不得从数据库时区推断，冻结后须保持一致 |

`--opening-approval` 是审核完成后单独保留的 approval 文件。
`--opening-attempt` 必须指向**原执行输出中的** `opening-attempt.private.json`，不能指向新造的 attempt。
文件内容、凭据和用户数据不要放进命令参数、Git 或公开日志；连接凭据由私有 target receipt 引用，runtime 不获得导入凭据。

## 审核后才能执行

预览会生成 `opening-approval-candidate.private.json` 和 `opening-payloads.private.json`，状态为 `opening-preview-review-required`。
生成 candidate 不等于审核通过。审核至少要确认：

1. 原 archive、导出字节、metadata、行数基线和原始用户行 hash 匹配；身份映射完整且一一对应。
2. 零预留证据确实来自同一 source/mapping，逐表原始行 hash 和逐用户覆盖完整；独立 reviewer 已审查经济权利，而不只是填写 hash 或状态字符串。
3. 每个 payload 的原始行、账号、整数金额和观察时间均绑定；观察时间只是冻结 dump 的捕获起点，不是历史支付或结算时间。
4. 当前 planner、identity helper、opening writer、全部 Core schema 和实际 catalog 指纹一致；目标的原 identity receipt 与原生用户映射实际读回一致。
5. 目标确为新离线库，只有初始化 seeds 和已核实的 identity-only 导入；未运行 app/worker，没有其他业务材料。

当前 identity helper 要求 schema v4 和完整的 13 组件集合，初始化合同为
`ledger-zero-events-empty-routing-unset-abuse-default-oauth-contract-v3`。除零账本、空事件容量和未选择路由外，
只允许 `core_abuse.settings` 的唯一默认记录：`revision=0`、`document={"version":1,"policies":[]}`。
另外核验初始化生成的唯一 `core_oauth.contract` 记录，包括版本、DDL 指纹和实际 OAuth catalog 指纹。
缺失、修改或新增的业务记录均拒绝；身份导入还会读回确认这些默认记录未改变。
旧 v1/v2 seed receipt 不适用于当前目标，不能只改 receipt 中的版本字符串来复用授权。

审核保留的 approval 必须与执行时重新计算的对象逐类型一致；`False` 不等于整数 `0`，`4.0` 也不能代替版本整数 `4`。
来源、映射、工具、目标或审核材料变化后，不得复用旧 approval。

目标使用受保护的本地 Unix socket，绑定 cluster system identifier、数据库 OID、启动时间、nonce、版本和 catalog。
实际数据库/schema owner 必须为 NOLOGIN，连接它的 LOGIN 角色单独配置并显式切换；runtime 的危险继承、SET/admin 路径、直接账本/receipt/序列权限均拒绝。
资源 owner 提供稳定的目标独占 lease；不同输出目录不能各造一个锁代替它。
执行期间必须持续保持离线独占：advisory lock 和一次无其他客户端检查不能排除不遵守该约定的 owner、DDL 或直接取号操作。

## 原子写入、重放与序列

全部用户在同一个 SERIALIZABLE、持久提交事务中调用 Core 的 owner-only opening 函数，完成完整读回后写一个整批 receipt。
同批 ID、同一审核材料的严格 replay 返回 `opening-replay-noop`，不追加 opening、不改变表或任何序列。
同 ID 改材料、换 ID 重导、映射冲突或向已封存批次追加账户均拒绝；整批冲突在任何 opening/取号前检查。

成功导入只允许两个账本 identity sequences 按预期取号，无关表和序列保持原状。
中途 SQL 失败回滚金融行，但已取走的账本 ID 可能留下空号；这不等于金融数据部分提交。
writer 记录触达序列的影响，不使用 `RESTART` 或 `setval` 抹平空号，也不宣称普通回滚能恢复所有序列。

## UNKNOWN：查原 attempt，不重试

超时、连接中断或 COMMIT 确认丢失后，`opening-outcome-unknown` 表示执行结果未确认，不能理解为“没有写入”。
不得换 ID 重导、修改原材料、重新执行 `--apply-opening`，或因为缺少 receipt 就断言已经回滚。
保留原输出目录中的以下材料，并用新输出目录运行 `--reconcile-opening`：

- 原 `opening-attempt.private.json`、approval 与全部冻结输入。
- 原 `opening-import.private.sql`、`opening-session-prefix.private.sql` 及其 hash。
- 原 `opening-backend.private.json`：同一执行连接的真实 PID、backend start、数据库 OID 和 system ID。
- 原 stdout/stderr、前态与其他故障证据；不得手改 metadata 的时间或身份。

writer 在同一 psql 连接中临时 `SET ROLE NONE`，以实际 LOGIN 读取自身真实 backend start，再恢复已审核的 NOLOGIN owner 后执行业务事务。
这解决 owner 角色下 stats 字段隐藏的问题，不授予监控角色，也不补造时间。

对账先验证原 attempt、SQL 字节和完整 backend metadata。
同一个对账连接先取得 session advisory fence，再建立新的只读事务快照，并确认原 PID 已不在；PID 仍存在或复用不明时继续拒绝判定。
不能在取得 fence 前建立旧快照，也不能释放 fence 后换连接读取结果。

| 对账结果 | 含义与后续 |
| --- | --- |
| `opening-confirmed-committed` | 整批 receipt、完整账户/账本读回和所有序列 seal 均匹配；对账不再写入资金 |
| `opening-confirmed-business-rollback` | 原业务前态和无关序列匹配，触达账本序列的合法前移单独记录；`new_reviewed_sequence_envelope_required=true` |
| 仍为 UNKNOWN / 拒绝判定 | 原 backend 未结束、metadata 缺失、材料/读回冲突或实例身份不符；保留证据并继续人工核对 |

确认业务回滚后，旧 sequence envelope 也不能直接重用；若要另一次执行，必须保留原经济 payload 和 ID，重新测量并审核执行前态。
所有路径均无自动重试。重启导致 boot 身份变化时，需资源 owner 审核恢复证据链，不能把重启当成回滚证明。


## 历史悬赏资金托管

Core 的 `billing::legacy_escrow` 为历史悬赏提供单独的资金池：一个项目可逐个支付冻结的挑战奖励，
关闭时只退还剩余款给原付款账户。它不能用普通钱包 Reserve 或商户订单代替；平台费不重复收取。

离线安装 owner 调用 `core_billing.import_legacy_escrow_batch(jsonb)`。整批材料绑定同一 wallet opening 的
migration、来源 archive、身份 mapping、观察时间及独立审核 proof；函数要求原 wallet opening 的预留为零，
并拒绝已有运行期资金活动。导入把项目金额从 clearing 记入原付款账户的 reserved，不再扣钱包。
这是独立的 owner-only SQL 原语；它的存在不表示真实历史资金已经组合导入或验收。
proof hash 仅绑定审核材料，不能代替原单位、经济权利和完整批次的人工独立审核。

### 只读生成托管候选

在 `--source-rights-preview` 上显式增加 `--escrow-candidate-preview`，从同一已核验的冻结导出、
身份映射和财务计划生成 `escrow-candidate.private.json`。它要求完整覆盖项目、挑战、争议和
母计划来源表，逐行核对母计划的变更后记录，保留原付款人与收款账户。已支付记录不能变成新的待付款；
历史拒绝保护记录的原数额也不能直接变成当前单位的可付金额。

`--escrow-unit-evidence PATH` 可提供独立的来源单位材料，绑定 archive、export、mapping、身份记录、
母计划、审计行与原执行代码证据。它只用于候选，不能作为经济批准。没有单位材料或存在来源 blocker 时，
`batch_candidate=null`，仍保留逐项候选事实和明确的 blocker。退出 `0` 只表示预览成功生成。

stdout 只给计数、状态和拒绝原因；详细资料仅写入私有 Backups 目录。manifest 分别绑定候选和单位
材料的原文件字节 hash、带类型内容 hash 及 mapper 源码。预览没有数据库连接，不写入资金，
`approval_created=false`、`source_unit_authority_verified=false`、`inflight_reservations_verified=false`。

### 钱包与托管的显式组合导入

[`financial_import.py`](../scripts/financial_import.py) 的 `--prepare --include-legacy-escrow` 使用独立
profile `financial-opening-with-legacy-escrow-v1`。它调用现有 Core 原语，将钱包、历史计数与全部托管项目
放进同一事务；原 opening-only profile 不自动加入托管。新 profile 的材料为版本 `3`，
来源经济审阅为版本 `2`，旧审阅和旧 SQL 批准不能授权这个新范围。

除原输入外，prepare 必须显式读取以下私有文件：

| 参数 | 绑定内容 |
| --- | --- |
| `--escrow-candidate` | 从原冻结来源与身份映射重新计算的完整托管候选 |
| `--escrow-unit-evidence` | 来源单位及母计划依据 |
| `--source-review-evidence` | `source_review.evidence_sha256` 对应的经济审阅原文 |
| `--escrow-policy-evidence` | 单位材料中 `source_policy_evidence_sha256` 对应的原文 |
| `--inflight-evidence` | 绑定来源、映射、导出、进程身份和完整用户/表范围的排空核对材料 |
| `--inflight-process-evidence` | 排空核对中 `process_evidence_sha256` 对应的原始进程取证报告 |

来源经济审阅须明确批准 `restore-clearing-to-reserved-without-wallet-debit` 策略。钱包 opening 的 reserved
为零只表示不把该批托管再次并入钱包预留；组合后每个付款账户的 reserved 必须精确等于其所有恢复项目之和。
仍须单独证明其他在途款已排空；在线数据库快照、终态计数、一个 hash 或策略枚举不能代替进程证据。
所需原文也按实际字节绑定；工具只核对材料一致性，不替代独立审阅者对事实的判断。

最终 SQL 批准另用版本 `2`、决策 `approve-offline-opening-with-legacy-escrow`，绑定 scope、material、SQL
和来源审阅。prepare 生成的模板保持 `REVIEW_REQUIRED`。同 ID 的严格重放、冲突拒绝、完整前态核对及
UNKNOWN 原 attempt 对账继续有效；不会因为提交回包丢失就换 ID 或自动补执行。

真实来源缺少单位或排空材料时，继续拒绝 prepare/apply。合成 PostgreSQL 演练及产物构建不能抵扣
真实源经济审核、金融导入对账、运行期付款/退款与整站验收；当前真实批次不能写成已迁移。

### 导入后的提交、付款与原操作核查

运行期 [`LegacyEscrowStore`](../services/core/src/billing/legacy_escrow.rs) 在同一事务中重新校验
当前个人账户 Session：提交证据须是冻结挑战的原参与者，付款和关闭须是原项目 owner。
调用方不能指定金额、收款账户或代替他人的 actor；这些值来自冻结导入记录和已验证的 Session。

原状态为 `accepted`、尚未付款且具有已审核 payout basis 的挑战，由参与者调用 `submit`，提供
`migration_id`、`source_project_id`、`source_challenge_id`、`operation_key`，以及至少一个 GitHub HTTPS
issue 或 PR URL。`issue_url`、`pull_request_url` 各最多 512 字节，可选 `submission_note` 最多 2000 字节。
这些链接是参与者的证据声明；Core 不抓取链接，也不据此证明仓库归属或 PR 作者身份，owner 仍须审阅。

成功提交只追加一条 `legacy_escrow_operations` 事件并增加项目 revision，不产生资金 journal。
原挑战的 `status`、来源行、来源 hash 和导入金额保持不变，不能把 `accepted` 改为 `submitted` 代替提交。
每个挑战只允许一次成功提交；换新 key 修改证据或重复提交会拒绝，同项目重复使用同一 PR URL 也会拒绝。
PR 去重仅覆盖新增的成功提交事件；导入记录没有历史 PR、issue 或仓库字段，不能据此与来源已提交记录去重。
owner 对这类挑战付款时，必须在 `payout` 命令中用 `submission_operation_key` 精确绑定该成功提交。
来源本来就是 `submitted` 的挑战保留原付款命令，不接受额外的 `submission_operation_key`。

每个挑战最多付款一次，同一项目可多次部分付款。收款及关闭后的退款优先归还账户欠款，再进入钱包；
普通 Capture、Release 和 Reverse 不能使用此 opening。成功操作、业务拒绝都保留原 key 的结果；
相同 Session 身份、相同 key 和完全相同的命令可重放，改材料或重复支付拒绝。

超时、断连或无法确认 COMMIT 时结果为 UNKNOWN，不能当成回滚，也不能换 key 补付。
`query` 按原 `migration_id`、`source_project_id`、`operation_key` 返回原命令、actor 和 receipt；
只有项目 owner 或该操作原 actor 可查，可用 `expected_command` 精确核对原命令。
查询暂时返回 missing 只表示此次读取未看到结果，不能证明原请求已经回滚；继续保留原命令和原 key 核查或重放。

accepted/submitted 挑战、未解决争议和仍在七日申诉期的拒绝记录会保护项目余额。
历史拒绝记录的原始 reward quota 不自动视为当前单位的可付金额；没有已审核 payout basis 的记录不可付款。
存在尚未解决的近期权利但缺少 payout basis 时，整批导入拒绝。开放争议没有原生裁决接口，仍保持冻结。

### 受保护的内部接口与部署边界

[`HTTP adapter`](../services/core/src/billing/legacy_escrow/http.rs) 复用现有 bounties 专用 token 和受保护
UDS，固定使用 `POST /internal/bounties/v1/legacy-escrow/{submit,payout,close,query}`。
请求同时携带单值 `Authorization: Bearer <host token>`、`X-Lmm-Protocol: 1` 和
`X-Lmm-User-Credential: <Session>`；host token 不能代替用户 Session。请求体上限 16 KiB，响应为 `no-store`。
receipt 区分 `submitted`、`posted`、`rejected`；成功提交不带 journal，零余额关闭也可能不带 journal。

Go 的[类型化客户端](../services/extensions/internal/modules/bounties/legacy_escrow_client.go) 提供
`SubmitLegacyEscrow`、`PayoutLegacyEscrow`、`CloseLegacyEscrow`、`QueryLegacyEscrowForCommand`。
核查调用携带完整原命令，不会执行付款；资金写请求遇到断连、5xx 或无效回执时返回 `ErrUnknown`，
不会自动重试或生成新 key。这是私有内部适配，未增加公开 Go handler、产品 UI 或新的浏览器确认 token。
用户侧公开付款流程与浏览器确认仍未验收，新悬赏和争议裁决的完整生命周期也未因此完成。

legacy escrow 业务表仅授 runtime SELECT，只有 `legacy_escrow_finalize` 函数有 EXECUTE；导入、直接表写入和账本序列
不授 runtime。DDL 参与 ledger 组件指纹。全新目标使用匹配版本 `init-db`；已有精确旧 Core 26 库须按
[显式 ledger 升级流程](archlinux-install-update.md#core-26-ledger-升级)单独演练，不能修改版本字符串沿用旧 receipt。
资金导入仍只接受符合原前置条件的全新离线目标，并拒绝非空升级回执；schema 升级不会放宽资金候选或授权。
当前真实目标未执行该升级，真实资金也未 apply；经济审核、来源进程排空及完整业务验收继续作为未完成项。
