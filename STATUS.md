# 当前推进状态 · 2026-10-30

目标：迭代到完整可用，并证明生产环境具备切换到新架构的条件。**整体尚未完成，当前不能证明可切换生产。** 完整分母保留在 [原始完成范围](recipes/lmm-api/COMPLETION_SCOPE.md) 与 [生产验收合同](recipes/lmm-api/PRODUCTION_ACCEPTANCE.md)，不按测试数量估计完成百分比。

用户已清理本机此前保存隔离实验的 `~/.local/state/lmm-options-completion-20261028/` 目录。下文保留已观察到的结果，但该目录中的原始日志、测试数据库和构建产物目前无法重新核对。

持久围栏的合成实验未完成：第二组在 PostgreSQL 崩溃重启时超过 15 秒等待限制并退出，后续提交确认中断场景未运行；这不能证明围栏可用于生产。

新增的隔离合成实验使用 PostgreSQL 18.6 私有 Unix socket：禁止新连接后，分别终止正在写入的事务或保持事务存活，随后立即停库重启；两种情况下禁连状态及原有已提交数据保持，未提交数据回滚。另一合成库停用旧登录并排空会话后，导入账号仍可读取，重启后状态保持；其他登录通过成员关系切换为旧角色后可以实际写入，即使撤销成员关系，已经切换的会话仍能继续写入并提交，新会话则被拒绝。停写必须排空这类会话。实验未覆盖 COMMIT 回执丢失或断电，也不证明站点所有写入者已停写。崩溃及撤销后存活会话场景通过独立复验；停库失败注入由开发者验证。

当前通过 `generated/lmm-api-source/` 固定源码提交 `084b50e703794e2e739856c802a9d7efd353f36e`：包含开账连接统计快照修复、旧转账离线来源映射、Pi-Core 测试的 Provider pin 修正，缺少财务、旧开账、历史日志、凭据、订阅或路由导入回执时保持 UNKNOWN 的修正，正式新装 identity v6 的邮箱挑战、业务、加密出箱、HTTP/SMTP worker 与 Web 调用接线，迟到退款成功/容量冲突后的原证明自动恢复及新邮箱测试运行权限，Notice/公告的受限运行账户写入、原子审计与 COMMIT_UNKNOWN，独立升级历史库 CLI/客户端和独立复验，修复后的 CLI Pi 配置只读诊断、Go 助手 agent 必选启动校验、显式离线钱包请求键约束升级入口，以及本轮价格排序、流截断诊断和质量计划修复，新增 Core agent 显式配置与新装第 27 个组件、正式 Core 进程启动验证、运维报告对 agent 状态的兼容检查，以及暂存邮件组件的配置站点重置链接。新增真实 relay 性能观测/查询、Web 分母加权/null/部分覆盖显示，以及 ChannelRead 动作判定修复。并行工作的未提交候选不包含在此固定提交中，也不当成最终候选。

本机源码分支已通过普通合并同步远端并推送；七个语言文件的远端价格文案和本地邮箱文案都已按逐键检查保留，CLI 指针已更新。已验收的迟到退款恢复已提交；其他未提交的配置升级、路由预检等候选仍在工作树中；历史邮件 staged 集成材料也保留，不能把这些目录或局部测试自动计入已固定提交的最终组合。

事务内路由发布原语已固定：外层 Core 用例可以把业务写入和路由发布放进同一 PostgreSQL 事务，保留原公开 publish 接口。最终冻结库的真实 PostgreSQL 自测 3/3 通过，覆盖外层失败回滚、并发版本冲突和公开接口；当前库/main 编译与库 Clippy 通过。它尚不提供完整渠道 CRUD、Session 授权或跨进程激活，不能据此关闭渠道故障项。

全站财务已在源码 `be9b687` 及显示修复 `084b50e` 提交、推送并回读。R7 Native 的独立完整矩阵20/20及正式首管CLI→真实密码登录→财务200/零值1/1通过，缺来源、晚结算旧请求等503反例保留。R7库/main编译和库Clippy通过，1316份实际编译输入与提交逐字节绑定；未运行完整Cargo测试或all-target Clippy。作者writer386的78项纯测试、10项基础及6项封存检查分属R6组合，两套共享2项，合计14个不同场景，不能算R7重跑。

R6真实旧27→财务28升级验证旧行/密码/序列/ACL保存、拒绝异常默认授权、精确重放、check-db/登录及实际数据库COMMIT确认丢失后的UNKNOWN和原请求恢复。R7只改变首管读取动作识别，schema及升级输入与R6相同；R6升级证据不标成R7重新执行。writer的提交后抑制回执与升级器的真实协议截断分别报告，不混为同一故障注入。

实际Chrome/CUA已用真实合成管理员登录，同一R7数据库切换新静态构建并刷新：全站使用25 USD/12500000归一化credits、余额10 USD/5000000，CNY125/25/100、USD12.50/2.50/10、LDC88/8/80及缺证据2/异常1均匹配。真实页面发现并修复美元卡片依赖钱包全局单位导致空显示；32项金额/组件回归、完整类型、lint/format、实际构建及包体检查通过，2672份输入与提交绑定。截图与浏览器范围在 `~/Documents/Codex/lmm-pro-20261030-followup/finance-ui-after.jpg` 及 `finance-browser-acceptance.json`。本次仅验收财务正向读取/刷新/警示；overview中的个人钱包、历史概览及问候语仍Unavailable，不能算完整控制台恢复。

两个Pro交付均已取得。财务复核更正报告31撤回旧的时间cutoff缺陷与补丁建议；渠道实现归档已校验、尚未合入，需保留财务28并实现明确28→29升级，不能覆盖旧routing或财务catalog校验。原12类事故、A01–A10/S11及生产切换结论均不缩小；真实旧来源、完整经济权利/持续停写及最终开放仍缺证据。

## 10 月 27 日部署故障回查 · 2026-10-29

按当时部署、报错和回滚对话，与上述已提交源码逐项核对。此前“全量迁移 / ALL PASS”的部署表述被后续页面、接口和资金故障推翻，不能作为完成证据。本表不按新增测试数计算完成率，也不将明确返回不可用算作功能恢复。这里只核对本地源码与已有证据，未访问生产。

| 当时故障或缺项 | 当前结论 | 已提交实现与剩余边界 |
| --- | --- | --- |
| 模型价格页、模型调用 503，缺少历史信任价格及净支付来源 | 部分修复，完整链路未验收 | 已有价格、路由及来源校验；最终组合的完整旧库导入、模型调用、定价和结算仍缺联合证据。历史某次价格页恢复不证明当前组合可以切换。 |
| 系统资源页面显示 Not reported | 字段与 DOM 用例通过，实际浏览器未验收 | Web 使用实际 `lmm-core` / `lmm-extensions` 键；本轮固定 Bun1.3.14 发现旧 overview 夹具缺少必填字段，保存失败后补齐夹具，25项 overview 通过，含进程键和未知值。产品组件/schema未放宽；原缺依赖与本轮失败保留。 |
| 管理概览、流量及用户统计 503 | 部分补齐 | `/api/data`、`/api/data/flow`、`/api/data/users` 已有原生请求只读投影，明确 `availability=partial`、`historical_imports=false`；不能当历史统计完整恢复。 |
| 全站财务统计、性能健康 503 | 原生财务及完整合成历史来源读取已交付；历史性能与生产迁移未完成 | R7独立21组、作者R6来源/升级切片及财务真实浏览器正向读取通过；缺来源仍503，未导入真实旧站。Native性能仍只含实际relay观测；历史性能未恢复，不能把财务页面通过算完整全站验收。 |
| 系统设置无法加载或保存 | 公开内容与限定浏览器流程已独立复验，完整设置仍未完成 | 新装实际 Core 的 GET/PUT/bulk/validate、Notice/双公告别名、权限、CAS、审计及 COMMIT 回执丢失通过；只开放三项内容键。限定浏览器 8 组已验（本轮4组、边界内复用4组）；主机/秘密/资金策略等设置、全页覆盖、旧库与最终组合仍未验收。 |
| 渠道列表、搜索、增删改及测试不可用 | 部分补齐 | 已有活动路由只读快照、搜索及能力说明；本轮修正 ChannelRead 被误用 AdminUsersRead 的判定，独立实际Native 7组/45次HTTP通过，171表/22序列不变。增删改、测试、余额等仍未实现，`unsupported` 返回501；真实导入策略链仍待验。 |
| 充值、在线支付及退款恢复 | 恢复切片已提交，完整支付链仍未完成 | 迟到退款成功、额度释放后自动恢复、原 proof/请求键、双进程重启及账本幂等已通过本地独立复验；实际 Go 宿主的充值提交断点/302 秒过期恢复开发测试通过。真实供应商、退款的完整宿主/浏览器入口、历史支付及全部 A05 流程仍未验收。 |
| 订阅权利、兑换、返利、红包及历史绘图/任务等旧功能 | 未完成全量迁移 | 完整业务分母继续保留；历史来源、全部经济权利和对应页面不能因占位路由或局部读取被算作完成。 |
| 登录、旧凭据和邮件相关业务完整迁移 | 邮件新装及限定 v5 升级后身份已验，完整迁移仍未完成 | 正式 identity v6、本地 SMTP 链路已验；两种合成 v5 前驱在实际 CLI 升级后，通过原密码/旧别名登录及未验证邮箱无重置权限。真实旧 v5 制品、所有旧凭据、实际浏览器、外部邮箱和完整历史迁移仍未验收。 |
| 助手入口、扩展及工具链 | 部分补齐 | 已通过受限账户下真实 Core 启动与认证控制 RPC 测试；Go 工具委托、对话资金闭环、存量升级和最终制品仍未验收。 |
| 异常调用后额度冻结、结算不明 | 未完成根治证明 | 已改进流截断诊断、资金恢复和 UNKNOWN 判定；历史补偿不等于未决成本或冻结款已结算。当前最终组合的中断、重启、对账及资金释放仍缺完整验收。 |
| 从旧版本平滑升级、失败回滚 | 正式 v6 与两种合成 v5 升级切片已交付，整体未完成 | v6 实际 CLI、库外历史/外置 HBA、明确单元停机、提交回执丢失、SIGKILL 窗口与目标物理恢复已限定独立验收。两种 v5 数据库前驱及升级后实际新 Core 另获独立通过，但没有真实旧 v5 制品，services=[]。全部写者持续停写、完整历史资金迁移、多主机存储、最终开放及 S11 仍缺证据。 |

因此当前仍不能宣称旧部署故障已全部解决，或证明生产可以切换。下文的局部自测、独立复验与候选进展保持各自范围。

## 原生性能观测与渠道权限 · 2026-10-30

源码产品提交 `0b54719`（性能）、`e106a74`（渠道）、`57c71549a5596eeb5f9c105df73634841f888977`（Web）均本地验证。真实 relay 在 execute 后记录单调耗时，流式首个非空文本/思考/工具Delta的TTFT，以及终结后的成功状态；非流TTFT保持null。SERIALIZABLE最佳努力追加 `usage_events/performance-v1`，失败显示缺样本，不能影响或重试模型/资金操作。只读个人Session查询受当前分组/模型可见性限制，使用请求中冻结的模型/组，UTC半开窗口、最近三非空桶、分子分母总和；不平均均值。此观测不是整段HTTP延迟、供应商纯生成速率、历史性能或所有入口的覆盖。

最终R2真实Core SHA256 `d6d356311de8cf6589c2ec4429603a0d9308ca634b1aed20b78a6c42b7c76c10`，库 `693984ccec2e06da3500dd5e128e19f0874fa3cf838a9d90d40079c1cc416945`；1310项冻结输入清单SHA `73a3bf0dc53ace138631306b09d3b11a60bbe66ff534704eb7f71067f6bcd41d`、编译配置SHA `60b83892afb73985beb17c6ed8938ffb7a1c0038df221760e2e63855b28953cc`。完整dev图 library/main与库Clippy通过，未重跑all-targets/旧单元总集。绑定已提交源码时仅 `history_http/statistics_tests.rs` 的cfg(test)改动晚于冻结，dev产品文件完全一致；不把未编译测试计为通过。manifest/catalog仍47ad3692/557d9775，未改schema或runtime授权。

性能开发实际Native/PG/延迟TCP供应商 `~/Documents/Codex/pm1030a4/` PASS：JSON/SSE/断流、加权/null/权限、仅观测失败仍完成模型、重启不重派，共4次真实本地供应商请求。独立所有者 `~/Documents/Codex/pi1030a2/REPORT.md` 8/8限定组PASS：两模型/两组、真实较窄用户可见性、延迟SSE、终态200但结算失败返回503且不算成功、崩溃缺样本和重启不重派；334项证据hash主线程复核一致。没有历史、全入口、>10000/高负载、完整取消/背压/重试分解、三个不同时间桶边界或浏览器验收。

渠道开发受限Router/PG门禁通过，随后另一个owner在 `~/Documents/Codex/ci1030a2/REPORT.md` 用实际R2进程独立完成7/7组、45次HTTP：默认admin读取、完整策略ChannelRead deny五端点403但同Session目录200、普通用户/Key/匿名/revoked拒绝、有效root canonical优先。171张表与22个序列全部保持，供应商调用0，三项不支持写操作仍501。真实历史导入权限链、并发策略轮换、完整渠道CRUD/测试/余额与浏览器未验收。公开重跑代码已提交f655a06，入口为 `tests/performance/independent_channel_run.py`；184项证据hash和执行前/发布后文档来源分开核对一致，不调用作者测试作为独立通过。

Web `~/Documents/Codex/wp1030a1/` 固定Bun1.3.14/Node26.10.0，2670项Web输入与提交57c7154逐项一致。34项性能定向加25项overview全部通过，完整类型检查、变更文件lint（保留原循环import警告）、一次隔离build及初始包预算通过：gzip550112/786432 bytes，533个dist文件，dist清单SHA `c2bed954b80682c049ef2db21604b1ed0b3d9a9e389cd903c5ea6a89626af884`。构建Version为该完整提交，正式工作树dist未覆盖。七语言各仅新增同4键，未知指标不归零、显示部分覆盖/采样/缺样本。没有把DOM或构建当浏览器、完整登录、历史升级或整站验收。随后独立Native响应→Web复验wi1030a2中7组通过、空结果覆盖提示1组失败：模型页PerformanceOverview提前返回丢失notice。产品修复 `24066ba4c2204a9bf6173c97f4ff2d8196512401` 仅补该提示，主线程wp1030a2完整类型/组件lint/overview25项通过；修后wp1030a2一次新build/包体预算通过，完整Version=24066ba4，2670项输入逐blob匹配该提交；初始gzip550115/786432 bytes，533份输出清单SHA `7a5189a0aa3e6fec8e426a57321152bad50210aba91832b07b0281b2f686e3b9`。独立所有者新根wi1030a3原8组全部通过，2715项hash主线程复核一致，公开代码提交e3b1f1d；使用归档实际Native JSON/SQL、真实API适配与React组件，只有HTTP传输和canvas替身，不是活后端/浏览器/登录验收。原wi1030a1模块导出夹具错误、wi1030a2真实产品失败及断言均保留。

保留原失败：开发R1实际观测因未用SERIALIZABLE而写入失败，产品已修；R2旧fixture误以为撤销分组不使Session失效，实际401符合合同；早期socket过长未运行产品。独立性能首轮误期望结算失败HTTP200、独立渠道首轮误把序列作复合表，各修独立夹具后通过，原件未删除。Web作者原34项使用canary而非固定Bun且hash为事后补录，不作固定工具证据；主线程按固定版本重新冻结/59项运行，并保留旧overview与固定版本补齐前失败。原失败不能被外层脚本退出0覆盖。

本轮成功PG/副本分别清理，主线程已将两次Web冻结测试源码/依赖副本与被替代bundle移到系统回收站，保留最终24066ba的533份静态制品、冻结变化输入、命令/哈希及所有失败原件；普通工作树dist未动。不将逻辑字节移除当物理空间回收。

本轮配方 lmm-api/core0.1.8、Web0.1.5、contracts0.1.2；递归check、临时pack、归档check及完整中文提示词build通过。归档SHA256 `8a7c11dc758d5a72ffe71cef3dcf76fdd63326549af7a45ee984d675802dfb7e`，提示词SHA256 `1fdc805365c0c3013c130ba1631406b88c2fd6b1ca9e1e27ac70fe61a7d79e5e`；临时归档和提示词已删除。配方检查不等于完整重建或生产验收。

上一轮配方 lmm-api/core0.1.7、Web0.1.4、contracts0.1.2，保留完整A01–A10/S11及原部署12类分母。递归check、临时pack、归档check和完整中文提示词build均通过：本地40份、递归归档65份UTF-8 Markdown，无链接。归档SHA256 `b17a94b5fd5bea60c3a5310397b1dce80a454fe63c31fc339c2e10c9f95d15d0`，提示词SHA256 `4f6b311c458d2ec17e20fea9d5dcc1e2495c0e2068caf5b85fac60eb8136dc1a`，临时归档/提示词已删除；不等价于完整重建。全站财务、历史经济权利、完整渠道、最终开放和生产切换仍未完成；未访问生产/DMIT或进行真实付费调用。

## 升级后真实启动与部署故障复核 · 2026-10-30

再次读取10月27日原始部署/回滚对话，核对08:53Z的四页面报错报告与09:08Z的全功能缺项报告，并检查当前固定源码。上表保留12类完整故障范围；目前没有一类达到固定最终组合、全部原业务及完整独立验收的全量关闭标准，不将局部修复折算成完成百分比。本节复核时 `history_http.rs` 的财务统计/性能汇总仍返回503；性能部分已由后续原生观测替代，财务仍503。`channel_http.rs` 的写入/测试仍返回501。之前的“全量迁移 / ALL PASS”不能恢复为有效结论。

新增真实运行缺陷已修复并推送：`fcb29cd98950053ab3a76e29548d8752366baa22`。旧候选2d982f的正式 CLI 虽通过，但升级后实际 `serve` 因错误要求 SELECT 私有 `core_meta.offline_upgrades` 而报 `MissingPrivileges`。本次只修 runtime 角色验证：六张公开合同和账本读取仍为必需，私有 `offline_upgrades` / `console_revisions` 的整表及单列 SELECT 都拒绝；未给 runtime 增权。manifest/catalog 保持47ad3692/557d9775。最终实际 Core SHA256 `dea60909b00bed72db09772200db99233b9eb68bdb9a62197aa38288786d71aa`，完整 dev 图库/main 编译及库 Clippy 通过，1308项编译输入逐项对应修复提交；没有独立重编或旧单元总集的新结果。

开发真实 CLI/HTTP 检查 `~/Documents/Codex/2026-10-30-runtime-authority-q29f_kip/runtime-run-02/` PASS：升级后明确夹具开放、实际受限新 Core、原密码登录、Notice 保存/读回、缺公开合同 SELECT 和私有回执表/列误授权拒绝。随后重新封闭，再完成原物理恢复/历史分歧场景；失败旧候选现场 `~/Documents/Codex/2026-10-30-upgraded-runtime-dve487un/` 已停库保留。

v5 独立所有者最终证据 `~/Documents/Codex/2026-10-30-offline-v5-fixed-runtime-fc90431b/`：fresh_v5_core26 为160表，console_v1_v5_core26为161表，两者各21条序列。实际新 CLI apply/recover、外部 Committed、原数据/ACL/密码保持、目标6881项catalog定义固定匹配；实际新 Core 以原密码/合法旧别名登录，错误/禁用/重复别名拒绝，未验证邮箱不产生重置挑战，另设合成已验证正对照通过。公开合同缺读拒绝，两张私有回执表/列读取误授均拒绝。259项SHA核验通过，公开验证代码在 `tests/upgrade_offline_independent/v5_*`，已推送 `1fc679ab95fce10d6f254eee9c92c542c2917ead`。旧2d Native只计数据库结果；最终runtime使用dea60909。没有真实旧v5制品，services=[]及手工开放仅夹具，不是完整旧站升级/最终开放。

另一个独立所有者 `~/Documents/Codex/ur1030a2/` 的8组v6升级后运行检查通过，176项哈希主线程复核无差异，公开runtime_*三文件已提交/推送 `ae58fdc2af2912ec5ddbe1112e59cfd43215fa53`：真实旧Core建库/服务停机/正式升级，人工夹具开放，最小角色与两张私有表13项真实SQL拒绝（42501），实际新进程身份、原密码登录及Notice HTTP/SQL一致；撤销公开合同或账本读权限均 MissingPrivileges，私有表/列SELECT与数据库CREATE误授权均 UnsafePrivileges。随后重新封闭并停止进程。首轮 `ur1030a1` 已升级成功，但独立夹具漏放行 postgres 管理连接，被 HBA 拒绝，后续组未跑；下一轮仅补夹具管理规则后通过，原件停库保留，没有给 runtime 增加元数据权限。开发与独立检查均未把手工开放当作产品finalization，未跑新的完整浏览器或S11。

本轮开发成功PGDATA/备份及过期compile-r1的三个产物已移到系统回收站，普通文件逻辑量915,297,070字节；不宣称已回收物理磁盘。最终r2制品/库、冻结输入、失败现场和日志保留。v5独立所有者另已清理其成功PGDATA；v6独立所有者已清理成功副本/秘密。全量资金迁移、未决成本、最终开放和原12类完整业务继续待验；未访问生产/DMIT，未做真实付费调用。

配方 lmm-api / lmm-core 现为0.1.6，Web保持0.1.3；递归check、临时pack、归档check、完整中文提示词build均通过，65个UTF-8 Markdown文件且无软链接。归档SHA256 `a045aa57052f1de1a3de882dbfdf48ea90f0730796cbc39b2850e3dcb5506513`，提示词SHA256 `6244d0bf907e39b4b3c6723fb4d9c0a4ac3fac64fad091395fb438903b638c8d`；临时归档/提示词已删除。此检查不等价于完整配方重建或业务验收。

## 正式离线升级及浏览器收尾 · 2026-10-30

源码 `90c442d8cb6d8811d0cdeaa6e806a324770f67f4` 和 `67dc5bac6667ad796bdcc496ec7de9952dbabf07` 已提交、推送并回读。后者只纳入正式 `upgrade-offline` v3，不夹带旧 control-v2/其他并行候选；已逐个核对 1308 个真实编译输入与该 Git 提交完全一致。新装 metadata 明确改变：manifest `47ad36927c16ccc4237cf16c9889705ae4d53db184c77aaac4a264c4e5203078`、catalog `557d9775042b94356f752ae4993bc73be645e9956e9fe1110ed5aa1cd531ee2d`。catalog 参考由旧实际 Core init-db 加精确新 metadata DDL 独立生成，再由新实际 Core fresh init 核对；不是升级观察后修改预期。

真实非 test Core SHA256 `2d982f5368f984241cc26bcbf60c38ddf56cf243c2400387a811b6e0cd4ef9f8`；同一完整 Cargo dev 图编出库/main，库 Clippy warnings denied 通过，未运行旧单元总集。开发三场景均 PASS：普通实际升级、目标真实 COMMIT 已成功但回执丢失、历史库 COMMIT 已成功但回执丢失；每次均包含旧数据保持、原绑定恢复、同/换操作拒绝、历史库进程停启及目标 PGDATA 物理恢复后 DIVERGED、无 replay/open。测试代码在 `scripts/test-upgrade-offline.py`。证据 `~/Documents/Codex/2026-10-30-offline-v3-lfg3lf4u/`。首轮编译错误、账号 owner/CONNECT、错误库的函数权限夹具失败保留；真实运行捕获 v6 比 v5 多一个序列，已修为 v6=22/v5=21，未放宽结构 pin。

独立所有者证据 `~/Documents/Codex/2026-10-30-offline-v3-independent-loq18ady/`，代码 `tests/upgrade_offline_independent/`：实际目标 COMMIT ACK 丢失，外部 Started+本地 receipt；恢复在真实 ALLOW_CONNECTIONS=true 窗口遭 SIGKILL 后，服务端原始 ErrorResponse 的 C=28000/HBA 仍拒绝 runtime。新连接恢复重新禁连并推进 Committed；物理恢复旧目标后独立历史仍 Committed，明确 DIVERGED 并拒绝重放/开放。独立 SQL oracle 检查 160 张旧表（22 张有数据）、22 条原序列、2001 条 ACL/owner；业务行/凭据/序列/授权保持，runtime_contract 仅精确更新固定 catalog。旧 Core 实际停止且持久 mask，重启拒绝；缺/错绑定拒绝。psql 默认未显示 SQLSTATE、oracle 首先误把预期 runtime_contract 更新计作数据变化，两次夹具误报保留，随后用原始 wire ErrorResponse 及精确字段转换复验。没有独立重编；v5 CLI、其他平台/版本、多主机、全站资金、最终开放和 S11 均未验收。

浏览器 `~/Documents/Codex/cb1030a10/REPORT.md`：限定 8 组满足，4 组用新 bundle 实跑（登录、Notice 成功保存刷新、真实503失败、真实 COMMIT UNKNOWN），4 组因除 Notice 外输入不变复用原独立证据（公告、其它设置只读、390×844、普通用户入口）。a8 捕获正确错误提示之外的未处理 Promise 拒绝，现 Notice 使用 mutation 的统一错误处理，页面异常 0；故障3.5秒均仅1次写，UNKNOWN 随后真实读回/刷新确认。Bun1.3.14 构建一次、完整类型检查与单文件 lint/format 通过；复用同锁依赖最初由 canary 安装，未混同工具链。Core 仍是旧实际 6eca... 程序，此浏览器结果不证明新 metadata Core 的最终组合。原定位失败和产品失败现场保留。

本轮成功开发测试的 PGDATA/备份清理，删除普通文件逻辑字节 1,109,394,039；失败现场、实际 Core/库、冻结输入、回执和日志保留。浏览器成功副本也已清理；其旧重复目录清理逻辑量和物理空间观测分别记录，不等同。当前 /home 可用约159GiB。整体目标继续，未部署/访问生产或 DMIT、未调用付费上游。

配方 `lmm-api` / `lmm-core` 升至0.1.5，Web升至0.1.3；递归 check、临时 pack、归档 check、完整中文提示词 build 通过。归档65个 UTF-8 Markdown 文件、无软链接；归档 SHA256 `d9c6c4eae945ef3f024cc8f83ebdaedee8c4696340f21656b4dd3a4db939373d`，提示词 SHA256 `8a9106d353b1b0e8e63807d471c8003616d52897b9af60b67b4c46938e49b2c1`。临时归档/提示词已删除；首次 pack 因扩展名错误被拒绝，改为工具要求的 `.recipe.tar.gz` 后通过，未改配方规则。这不是从配方完整重建的软件验收。

## Issue 与配方维护 · 2026-10-28 UTC

[源码 PR #1](https://github.com/TokenNotIncluded/lmm-api-source/pull/1) 已合并，正常保留主分支的邮件暂存与 CLI pin 并行提交。`lmm-api`、`lmm-core`、`lmm-contracts`、`lmm-web` 配方升至 `0.1.1`；扩展与 CLI 配方版本保持 `0.1.0`。本轮状态为 **self-tested; independent acceptance not run**，不替代下文原有独立验收的固定输入与适用范围。

- #911：新增输入、输出、按次价格各自升降序，使用 schema-v2 USD 实际价格；零有效，缺失/无效/上下文相关动态价格排后，稳定排序且兼容原 URL。七种语言同步更新。
- #961 / #962：现有动态价格分类、SSE 同块多帧及跨块解析已经具备，本轮验证相关实现和测试证据，没有重复移植旧实现。
- #786：原 staged snapshot 绑定已有回归覆盖；补齐完整质量计划中的 `mocklab-selftest` 及计划检查。
- #945：缺失协议终止标记的流现使用 `upstream_truncated`，与破损 HTTP 连接、协议格式错误和用户取消分别记录；输出前 HTTP 502、输出后流内错误、UNKNOWN 及预留资金语义保持。真实 PostgreSQL 的不完整流、掉线、重启和 hold/reconcile 仍需专门验收，issue 保持开放。

开发验证：Web 五组定向测试最终共 71 项通过，完整类型检查及变更文件 lint/format 通过。首轮组合运行 70 通过、1 个新增 UI 用例因未等待菜单挂载失败；调整等待方式后受影响文件 5/5 通过，其余 66 项未变更。Rust 1.99.0、`--locked`：真实 TCP `relay_http` 20 项、honeypot 10 项、SSE parser 7 项通过；实际 Go 生命周期用例明确过滤 1 项，没有运行真实数据库资金验收。质量门禁自测 31 项通过；Mocklab 首次受限沙箱因 socket 权限得到 12 PASS / 27 FAIL / 1 BLOCKED，允许 localhost socket 后同组自测 36 PASS / 0 FAIL / 0 BLOCKED / 0 NOT RUN。该复跑是本地合成组件验证，网络命名空间未隔离。文档品牌检查与差异空白检查通过。上述行为测试绑定源码树 `ccf135748c9cb27ed4309d98425a52e123c59438`，合并时保留的并行变更不属于这些行为测试的验收范围。

保留并行变更后的实际合并提交 `27068288bf4d7733da3884b6b11023fb171ef476` 额外通过 `cargo check --manifest-path services/core/Cargo.toml --locked --lib`；这是合并组合的编译检查，未扩展为邮件或 CLI 的运行验收。

配方递归 `check`、临时 `pack`、归档 `check` 和完整中文提示词 `build` 通过；归档 65 个文件均为 UTF-8 Markdown，无软链接。归档 SHA-256 为 `545ef980ffc097e0d412abf890415cdb61c69085a075a84ede1367968847e385`；完整提示词 SHA-256 为 `4a4941d171d3b6e8747629761eab8feb957059150e4893dbb6b7bee76f6100cc`。临时归档与展开提示词在检查后删除；这不证明从配方成功重建产品。

旧 PR #879、#875、#942 的剩余要求收录于 [维护与迁移合同](recipes/lmm-api/ISSUE_MAINTENANCE.md)，使用 `PR879-R1..R3`、`PR875-R1..R3`、`PR942-R1..R7` 共 13 个稳定编号，由 [#895](https://github.com/TokenNotIncluded/lmm-api/issues/895) 继续跟踪。旧布局 PR 按迁移取代处理不表示整项能力完成；历史通过、失败、UNKNOWN、schema 阻断和完整财务导出仍未完成的状态全部保留。#920 的旧生产目录分类、#943 的不完整流结算策略及其他未证实完成项继续开放。本轮未部署生产、未调用付费上游，也未进行独立业务验收。

## 最新进展 · 2026-10-29

支付恢复已在源码 `a853ea8f0186c948a62206005931efc1c47004d8` 提交：同一退款先失败后成功不再被 Go 当作已处理而丢失入账任务；Core 仅对原 dispatch/refund/amount/proof 恢复，并继续检查剩余退款容量。受其他退款占用时保留不可变 blocked 事实；收到充分绑定的失败证明、Core 确认释放后，Go 在保存释放回执的同一事务里重排原 accepted-proof settlement job。正式宿主的 Work 调度无需手工 Reconcile，不重新 POST 退款、不改原 key，也不授予额外资金权限。原 blocked 事实保留，匹配的 provider-refund receipt 才是解决见证；累计成功超过原付款仍拒绝，不编造净结算或损失分摊合同。

独立验收 `~/Documents/Codex/2026-10-29-a05-independent-_mju3n5z/`：原本轮完整正式 main 邮件/agent 组合冻结 1296 文件，manifest `2317680de66748510c3f3ef50f735fe42ae684b4c22a3774f2edf00505d0f929`、实际非 test Core `2ad4650d8516c11eca1c34c65c088c9c7f4155b521f3ccffe4227f7e0369d77c` 上，实等 302.369 秒，过期新接纳拒绝、原 proof lookup 不变、自动 Work、Core 入账后 Go 本地回执丢失、杀停/重启两个 OS 进程、重复回调、精确账本与仅两次供应商 POST 均通过。旧独立失败由 15 字符 hold key 和已提交后状态应为 awaiting_evidence 的夹具错误导致，旧断言/失败不改写。最终仅将 Core 的一个嵌套 if-let 等价折叠为 let-chain 以通过 Clippy；独立者审阅比较顺序和拒绝行为等价，旧 302 秒证据不改贴新二进制。最终实际 Core `70f8d903346803ef058a327dd977b6a50495a90137e3f498e6c69453c0722d4b` 另在 attempt-06/07 完成原 blocked proof 的容量释放恢复、16 路跨入口竞争仅一次 posted、双成功超额拒绝复验。可复制独立测试已提交在源码 `tests/payments/independent_a05/`。

最终组合开发证据 `~/Documents/Codex/2026-10-29-payment-host-recovery-j0yx0mjs/rust-tests-r2/`：Cargo.lock 绑定完整 dev-feature 依赖图（SQLx migrate）编译库、实际 src/main.rs 和三个测试目标，库 Clippy warnings denied/格式通过；不能称为 release 或 S11 资源验收。受限角色测试 helper 只从正式授权脚本取新 identity v6 邮件窄授权，未混入 Console 候选。真实 PostgreSQL Store 29 PASS/1 ignored；实际 Core 与实际 Go main `24f686e1a8367571e343416142506ec1a20d2cb14c5aae0263b0b5f09003fb0c` 的提交后本地回执丢失/双 OS 重启测试通过；另显式执行原 ignored 的宿主 proof-expiry 用例，实际单调等待 302.034 秒、签名年龄 306 秒、62 次观察，完整用例 366.96 秒通过且仅执行目标 1 项。旧配置和秘密不变，原 proof/key、单次充值及精确账本恢复；这不是供应商退款 HTTP 入口的整链验收。原独立生产依赖图与最终开发依赖图/源码哈希分别保存。

Go 支付包自测保留 7 项外部夹具跳过；初次 rustbridge 两例因临时路径过长失败，单改 TMPDIR 后 Go driver 仍使用长 GOTMPDIR，日志保留；两者均设 /tmp 后受影响包 4 项及绑定子例通过。首轮 Clippy 因新增嵌套 if-let 失败，修复后新冻结输入重建/复验，未放宽 lint、资金断言或产品门槛。新 Go main 在无 Git 的冻结副本首轮 VCS stamping 失败，仅用 buildvcs=false 修正构建来源，源码/产物另由 manifest 绑定，不伪造 commit。成功的自有 PG 数据和临时进程目录已停库清理，进程证据移入上述私有目录；主线程另删除约 800.6 MiB 已失效编译中间产物，独立方约 308 MiB 中间库及 262.5 MiB 成功测试数据，未动共享缓存、未完成工作和原失败记录。

历史 Console 并行候选的 COMMIT_UNKNOWN 和新装 Rust/PG/实际进程部分已由下文正式交付取代；旧库桥接/持续围栏仍不齐。数据库信任边界与 HTTP 用户审计分别说明，桥接与持续围栏仍是未提交候选。原交付草稿在 `~/Documents/Codex/2026-10-29-console-delivery/`，不能将支付通过抵扣设置、升级或其他 A01–A10/S11 验收。

正式邮件接线已在源码 `57339728292d522ca7a4f926f1624301b5cc9350` 提交：identity v6 新装 SQL、对应窄权限和启动检查，独立 proof/rate/AEAD 密钥配置、Core/Web 路由、原子安全通知、自动 SMTP worker 及退出等待。Web 注册匿名不自动刷新，绑定固定发起凭据/会话，重置 fragment 在应用加载前清除；原生注册使用入口的真实身份策略，不能绕过 Web 的关闭状态。存量 v5 仍拒绝启动，不把 staged delta 当正式升级。

邮件实际非 test debug Core 二进制 SHA256 `89280f89f4da28afcdb0aa4f43cbb31afa68f53e08d5d0a89569e8565a126144` 绑定 538 项冻结输入；对应暂存实现与这些输入逐一一致，额外 Web/文档独立记录。正式库/主程序/HTTP 业务目标编译和库/主程序 Clippy（warnings denied）、格式检查通过。真实 PostgreSQL 的 17 项业务/HTTP 测试首轮 16 通过、1 项限流记录 INSERT 超过既有 5 秒限制；保留产品和断言，用同二进制在增加测试 WAL 容量的全新夹具中单项 4.64 秒通过。原超时保留，不据此声称性能问题根治或 S11 通过。

独立验收在 `~/Documents/Codex/2026-10-29-core-email-main-v3jf71jx/independent-smtp-n0fl_tc1/evidence-04/` 的六组均 PASS、退出 0：实际 Core 初始化/受限角色启动、公开开关、真实 SMTP 收码注册和登录、重置链接/新旧密码与会话/refresh 撤销及 proof 重放拒绝、原生注册限制、缺邮件权限启动拒绝和 SMTP DATA 未结束时等待退出。此前三轮夹具失败全部保留；第三轮立即申请 reset 与同地址跨用途 60 秒冷却相符，第四轮等待真实窗口后通过，没有修改产品限流或数据库时间戳。该验收只证明本地私有 PG/SMTP 与实际 Core 进程；临时进程、数据和 socket 均已停止清理。

Web 15 个输入哈希与检查时一致；启动/回调 JS 8 项、邮箱 API 4 项、重置 UI 6 项通过，完整类型检查、相关 lint/格式检查通过。测试使用固定 Bun 1.3.14；依赖在隔离网络命名空间通过缓存硬链接恢复，未改锁文件。真实浏览器、正式静态包、外部 SMTP/收件箱、存量升级和生产切换仍未验收。

本轮 `lmm-api`、`lmm-core`、`lmm-web` 配方升至 0.1.2，递归 check、临时 pack、归档 check 与完整中文提示词 build 均通过；归档 65 项均为 UTF-8 Markdown、无软链接。最终归档 SHA256 `db1e3ed65652b1a7a21d1f30210f6f0896d136d1ba9bbb6e66f103dc407c9b20`，提示词 SHA256 `9d0c137731fd6cc4c73b3ffcf689be62a6dd4dd0ad1b35c78085cea7a71cf4ed`，检查后临时归档和提示词已删除。这是配方结构验证，不是从配方重建成功。

五项新增邮件配置单元测试现全部通过：密钥分域与文件安全、显式 SMTP、站点 origin、缺配置不公布邮箱验证、旧配置启动拒绝。测试在完整 Cargo dev-feature 图的真实库内执行，1110 项无关测试被过滤，不能声称全库测试通过。前序私有接口包装、launcher、资源门禁和编译失败均保留；可用内存恢复后通过既有 2 GiB 门禁，完整 test 库编译发现两个旧测试模块不匹配新接口，修正测试命名空间和缺少的 email 字段，补齐冻结快照漏带的既有 contracts 夹具后才通过。仅这两项测试源码修正另提交为 `84100a7573cfa02c223785901cc6fdca3d0dc1ae`，正式邮件运行代码与已验收非 test 二进制未变。完整输入和命令见 `~/Documents/Codex/2026-10-29-core-email-main-v3jf71jx/attempt-03/lib-config-test/launch-06/`；本轮整体依然不能标为完整可用。

此前六名并行协作者因 Codex 刷新令牌被撤销而中断；默认 `group/smart-split` 路由随后仍不可用。本轮改用工具支持的 `gpt-6-astra` 分工完成邮件 schema、配置、Web 及独立实际进程验收，结果见本节前述记录。A05、钱包和围栏等候选继续保留各自未完成状态，开发自测不替代独立验收。

`financial_import.py` 的只读恢复在回执缺失、即使当前数据匹配前态时也返回 UNKNOWN；旧备份可能保留数据库身份但丢失之后的提交，不能仅由库内数据证明未执行。同一材料的准确回执仍可确认批次已提交，但当前实现没有原 attempt 的持久绑定或独立历史见证，也没有持续停写；UNKNOWN 并非重试授权。以 `0b446dc` 冻结源码叠加本次三个改动文件，14 项单元检查和 3 项隔离 PostgreSQL 18 检查通过；独立验收在隔离 PostgreSQL 中先保存旧前态、提交开账、恢复旧备份，随后实际 resolver 返回 UNKNOWN 且不写入；存在准确回执仍返回 COMMITTED。逻辑恢复产生的默认 ACL 表示差异按原权限清单复原后才通过目录校验；这不是持续停写或生产恢复验收。本次开发测试直接在含未提交 Console 候选的工作树中先因已冻结的 Core 源码指纹不匹配而失败，随后改用冻结源码，没有修改或放宽指纹。

历史日志导入的无回执结果同样保持 UNKNOWN；13 项单元检查和隔离 PostgreSQL 的实际无回执/已有回执用例通过。该 PostgreSQL 测试首次因旧夹具丢弃现有来源合同字段失败，保留字段后重跑通过；另补齐恢复 SQL 返回的迁移 ID，真实数据库回读响应与驱动合同一致。尚未建立独立外部历史见证。

历史凭据导入在回执缺失时也保持 UNKNOWN，合法回执仍可确认；开发者在隔离 PostgreSQL 18.6 上完成 15 项测试，含旧物理备份恢复和只读核对，另有 37 项单元检查通过。测试以冻结旧源码叠加本次三份文件，未进行最终组合的独立验收。

旧钱包/计数器开账对账的无回执分支现返回 `opening-outcome-unknown`，不再声称已确认回滚；隔离 PostgreSQL 钱包和计数器两种模式共六个场景及 40 项相关检查通过，未运行完整 CLI 或独立业务验收。下表关于“回滚确认”的旧证据反映当时版本，其无回执的结果判定现已收紧。订阅和路由恢复也在无回执时返回 UNKNOWN；冻结源码上的隔离 PostgreSQL 分别通过 8 项和 3 项测试，包含物理备份恢复。其订阅单元测试曾有 PG17 镜像预期与现有 PG18 不符的失败，修正该测试后单项通过；跨恢复的持久停写与外部历史见证仍未实现。

持久升级控制 v2 的未提交隔离切片在 PostgreSQL 18.6 上通过 8 项开发自测，覆盖持久 STARTED、并发、响应丢失、数据库重启与权限拒绝；它尚未接入任何现有升级器。原 catalog 与 owner 校验会拒绝其新增对象，正在实现逐对象验证、兼容投影与真正升级接线；不能忽略控制 schema 或把该测试当成持续站点停写、外部历史见证或生产验收。证据在源码子模块 `.local-tests/upgrade-control-v2-p0-run01/`。

独立复验对同一冻结切片完成 9 项隔离检查，并重放开发测试 8/8；首轮两项因测试夹具失败，修正夹具后分别通过，原始失败保留。业务所有者在 READY 和 STARTED 时仍能直接写入，原升级器也未使用该槽；因此这不是持续停写或产品验收。复验期间开发工作区继续变化，结论不适用于后续接线版本。证据为 `~/Documents/Codex/2026-10-29-core-v2-slot-acceptance-064133/acceptance.md`。

工作树中进一步接入升级控制的候选以冻结源码编译，首次 PostgreSQL 测试因测试集群默认 SQL_ASCII 而失败；显式使用 UTF-8 后复用同一测试二进制，定向用例 1/1 通过，验证 Console DDL 与原业务回执、绑定操作回执共同提交，故障回滚后 STARTED 保留，以及换 ID 和控制权限篡改拒绝。该候选未提交，正式 CLI 未构建或验证，ledger/additive 的既有历史门槛仍存在；持续站点停写和库外历史见证仍缺失，正式写入保持拒绝。证据在源码子模块 `.local-tests/upgrade-v2-core-run05/` 和 `.local-tests/upgrade-v2-core-run06-pg/`。

邮箱挑战存储切片已提交：开发自测 9 项通过（其中 8 项使用隔离 PostgreSQL），含并发单次消费、错误码预算、事务回滚和历史邮箱不可直接恢复；工作树组合的 `cargo check --lib` 通过。该切片没有接入正式安装或升级、权限、注册/绑定/重置 HTTP、Web 或 SMTP，不得应用其测试 SQL 到现有站点；当前 issue 先提交，不能在之后另行入队邮件来声称安全，正式启用前须把挑战与加密 outbox 原子提交。独立验收在 PostgreSQL 18.6 完成 16 项真实数据库用例，覆盖未验证历史邮箱拒绝、并发单次消费、次数限制、事务失败与权限边界；完整密码变更、HTTP、Web、SMTP 和正式安装升级仍未验收。证据在 `~/Documents/Codex/2026-10-29-lmm-email-challenge-independent/`。

历史切片记录（本轮正式接线前；当前状态见本节开头）：已提交但未启用的加密邮件 outbox 切片在隔离 PostgreSQL 与本地 SMTP sink 的开发自测中通过 18 项；此前 SMTP 收件阶段拒绝时错误断言应收到正文，修正测试后重跑通过，失败记录保留。冻结的 11 个文件与验收记录的 SHA-256 一致；挑战签发、限流预算与加密出箱可由同一事务提交，无需客户端挑战 ID 的验证也在该切片自测。独立测试以固定输入运行实际代码，14 项通过，包括事务回滚、12 路并发、猜测预算和 8 种 SMTP 响应；确认租约过期后迟到的 250 不能覆盖 UNKNOWN，显式重发仍可能重复投递。真实邮件、HTTP 和正式 schema/权限/启动均未接入。账户变更的安全通知尚无法通过现有公开接口原子接线：`EmailMutation::finish` 同时消费挑战并提交，而通知入队需要已消费状态及仍打开的事务；这需要修正并验证失败回滚。证据为 `~/Documents/Codex/2026-10-29-lmm-email-outbox-selftest-attempt-6/freeze-receipt.json` 和 `~/Documents/Codex/2026-10-29-lmm-email-outbox-independent-acceptance/acceptance-report.md`。

历史切片记录（本轮正式接线前；当前状态见本节开头）：邮箱注册、绑定和密码重置的已提交但未启用业务切片使用服务端选择的有效挑战，不要求客户端提供挑战 ID。隔离 PostgreSQL 开发自测首轮 8 项中 7 项通过；剩余用例因测试用户没有开发者授权而在创建 API Key 时被拒绝，补齐测试前提后该用例单独通过。两次运行的源码快照不同，不是同一最终组合的 8/8 验收；中间一次编译被主动中断且未运行数据库，失败及中断记录保留。随后独立测试固定源码与依赖，在隔离 PostgreSQL 通过 11 项，覆盖真实旧凭据撤销、并发消费、MFA 保留和事务失败回滚。证据为 `~/Documents/Codex/2026-10-29-lmm-email-business-independent-acceptance/acceptance-report.md`；完整 HTTP、真实邮件和安全通知事务仍未接入。

独立 CLI 仓库提交 `4e6e351` 增加 Pi 1.2.0 显式实例配置的只读诊断。固定源码的完整 CLI 开发自测 68 项、clippy、格式及隔离只读命令检查通过；其中新增诊断测试 9 项，初始旧版测试的失败日志保留。**后续独立验收未通过：**并发替换符号链接可使检查后的打开操作读取合成凭据文件；另有 4 种 Pi 原生支持的来源写法未被识别。126 个有效用例中 121 项通过、5 项失败，失败已交回实现者修正，不能将其声称为安全的完整只读诊断；已有测试二进制与源码的独立构建溯源也不足。未验证真实已安装 Pi 的完整接入或 Windows/macOS。开发证据为 `~/Documents/Codex/2026-10-29-lmm-cli-pi-status-41dkue3u/evidence/summary.json`，独立验收报告为 `~/Documents/Codex/2026-10-29-lmm-cli-pi-independent-cqmz5w1d/ACCEPTANCE.md`。

CLI 修复候选 2 在独立命令测试中针对原问题的 17 项及 3 项混合来源用例通过；旧版基础回归 120/121，剩余断言期望本地来源为 `no_known_source`，候选按新文档合同返回更保守的 `unknown`，原失败与单独核对均保留。作者的 Linux 自测 70 项和 clippy 通过，36 项输入哈希与提交前工作树一致；修复已提交并推送至 CLI 仓库 `wip/pi-native-integration` 的 `f86ec5e`，源码 main 已固定该 CLI 提交。验收者核对了冻结源码、作者构建日志与产物哈希，但没有独立重建证明；平台和真实 Pi 限制仍在。证据在 `~/Documents/Codex/2026-10-29-lmm-cli-pi-retest-2w2vgxrd/`。

新的旧转账映射仅整理和校验离线来源事实，15 项合成测试通过；`can_apply=false`，未实现双库导入与激活。Pi-Core runner 的 Provider 固定提交已对齐当前 gitlink，真实 Provider 源码绑定自测通过；实际 CLI、浏览器和 Core 联合验收尚未运行。

四个 Pro 分别完成数据库升级、财务迁移、邮箱身份和全项目验收方案；财务修订版的参考 PostgreSQL 实验确认须先取得原执行锁再建立新读取快照，并揭示旧备份的历史连续性问题。相关报告和交付在 [交接目录](https://drive.google.com/drive/folders/1QqRY8YHkT7faUq5nlZnkmaCAHIcXyxyq)；这些参考模型不是产品集成或生产验收。

## 本轮已修复并独立复验

| 问题 | 修复后结果 | 实际证据范围 |
| --- | --- | --- |
| PostgreSQL 事务内活动统计快照可能隐藏新连接 | 开账的初始、最终及回滚确认检查刷新活动快照，发现其他客户端时中止事务 | 独立 PostgreSQL 18.6 合成库执行完整回滚确认 SQL，在钱包和历史计数两种配置下验证失败回滚、已有连接拒绝和晚到连接拒绝；移除末尾检查的反向对照错误通过。并发时序使用测试通知和暂停；未覆盖完整 CLI 的 attempt 校验或最终检查后的连接，仍需外部停写和排空 |
| 扩展宿主凭据 absent 却能 safe_to_cutover=true | 五个扩展业务门禁全部 BLOCKED；reviewed 对照保留原 READY | 独立实际 Python CLI 回归；preflight/身份构建器/本地回环 edge 合成测试 |
| 普通财务计划在 Key 限制元数据夹带 bearer | 规划阶段拒绝所有 Key 的已知 bearer，异常及 stdout/stderr 不泄露，不发布计划 | 独立同 Key/跨 Key/嵌套反例及真实普通 CLI 调用；原始冻结私有来源归档当前状态未核实 |
| 源和目标同样非法的 Key 限制类型误报 matched | 限制字段不再匹配，整体对账失败，仍禁止激活 | 独立九类字段域反例与既有 typed-fact 对账测试；不证明目标清单真实来源与完整性 |
| 旧转账状态与领取或取消事实矛盾 | 标记阻断，禁止据此生成余额开账数据；待领取转账仍需专门导入合同 | 基于已提交源码的隔离环境 34 项相关测试通过；独立 512 组状态组合通过。未执行真实历史数据导入或生产验收 |

## 本轮真实 PostgreSQL 验收

测试方在全新 Unix socket、无 TCP 的 PostgreSQL18 和合成用户 1–4 上执行钱包及历史原始计数器 CLI。预览/审批/实际导入通过；正常重放、真实 COMMIT 成功但外部 psql 确认丢失后的原操作查询恢复，均核对 160 张非系统表和 21 个序列的快照，未发生重复变更。没有用成功 mock 替换业务 SQL；测试自己的集群已停止。

只覆盖合成开账与原始计数器 profile。二进制 schema-manifest 与当前安装 schema 匹配只证明 schema 身份，不证明完整运行二进制来自当前最终组合源码。外部 psql 回执丢弃不等于数据库网络层断连。真实历史资金、订阅、Key、持续托管权利及其他业务仍须独立组合迁移和对账。

当前工作树（含保留的未提交 Rust 工作）执行了 Rust1.99 cargo check --locked --all-targets，通过，输入文件哈希前后一致；这是编译检查，不是测试、业务通过或已提交候选的构建证明。

## 独立升级历史库 · 2026-10-29

主配方与 Core 配方 0.1.4 递归 check、pack、归档 check 和完整中文 build 通过；65 文件全为 UTF-8 Markdown、无软链接；归档 SHA256 `dac85b0e5ba1575fbde64d7ec4af9a17186e9c8aa63a17a92d8df99b98d78e14`、提示词 `b70846d340bee9a8965132190711d3d1178452b5621e63fe8f10ff5b4f62a7f1`，临时文件已删除。这不证明从配方重建成功。

已提交源码 dddde75 并在 `7c3ebb86cf406c5f0a268ea2eb90edb985271751` 加入独立测试。`lmm-core upgrade-history` 只安装或查询独立 PostgreSQL 18 authority；实际 Rust History API 保存原 binding、CAS 消费及回执观察，不新增产品程序，不在普通启动或目标 init-db 中安装。拒绝错误 authority、目标同一物理集群、owner/超额 writer 权限。历史状态和传入的围栏/回执 hash 不单独授权 DDL、retry 或 site open。无自动过期、接管、清空或新一代升级；完整 finalize 仍未完成。

开发证据 `~/Documents/Codex/2026-10-29-upgrade-history-5h209odr/`：完整既有 Cargo dev 图编译 native 库/实际 main/目标测试，Clippy warnings denied、格式与文档检查通过。实际 Core SHA256 `6eca9a6e8b270bc7368057469dbd8185ad287db74589df3f3a223f610a53ec31`；新装 schema-manifest 仍为 `59bc164463df00df11a253feecbf9e08dd914e6e3cc7fcb3af68d193287eb005`。 提交后 1295 个 native/未变基线文件逐个与编译输入匹配；成功测试 exe、Clippy 中间物、临时 index/pyc 已清理 99758522 字节，实际 Core/库及必要冻结证据保留。r1/r2/r3 按测试输入递增，native 产品输入未变、实际库/main复用；新增输入分别是故障代理/测试、历史库进程停启后 CLI 读回、输出路径拒绝。并发仅一次、目标物理恢复、原绑定/角色拒绝通过；代理观察真实 COMMIT 和 ReadyForQuery 后截断，新客户端得到 COMMIT_UNKNOWN，新连接仍读到 Started 且重做拒绝。历史库停启后实际 CLI 读回 Started/Committed，retry/open 均 false。原 Console 代理默认行为自测通过。

独立 owner 在 `~/Documents/Codex/2026-10-29-upgrade-history-independent-side-a/` 使用同一实际库/main、独立 Rust 程序和 SQL oracle，未执行作者业务用例。八组检查 PASS：逐字段原绑定拒绝，错误 authority/同实例拒绝，四消费者仅一个成功，真实 ACK 丢失/死连接与新连接，合成扣账后的物理目标恢复，authority PostgreSQL 实际进程停启，writer 15 项真实 SQL 权限拒绝，真实 CLI 各阶段只读且无 open/retry。不是宿主机 OS reboot；合成扣账和 hash 不是完整资金或围栏证据。成功库、备份副本、exe 和秘密配置已清理，独立源码在 `tests/upgrade_history_independent/`。生产独立存储/凭据未落实或访问；最终 apply、连续部署围栏、恢复控制器、browser、release/S11 和生产切换未验收。

Console bridge r4 独立数据保持也通过，证据 `~/Documents/Codex/2026-10-29-console-bridge-r4-independent-side-a/`：两种前驱旧数据/email/flag/21 序列/ACL 保持，真实密码/alias 与 reset 正反对照通过。首次独立 SQL 序列整行转换 42809 的夹具失败保留，修正为显式序列字段后通过；失败库停止保留，所有运行材料已移出源码目录。它仍是私有 stage 的合成测试，桥接产品/正式 apply 未提交，不能计为升级完整交付。

## 公开设置内容写入 · 2026-10-29

`lmm-api` 与 `lmm-core` 配方升至 0.1.3，补齐上述内容写入与实际进程验收合同，完整 A01–A10/S11 分母不变。递归 check、临时 pack、归档 check 和完整中文提示词 build 通过；65 个文件全部 UTF-8 Markdown、无软链接。归档 SHA256 `a6bee28e770eb0cd51daf4007aadd15bafb07c9fa728156dc73dfb86ffcb8fb2`、完整提示词 SHA256 `6020cfdf7c9637eddc73191a1cb3f57fd8fa142b668a72ebf917c303e0f40e6c`；临时归档与提示词已删除。这不证明从配方重建软件。

源码 `f993be2c860fded95af706527ff5b8705c52cd1c` 已提交、推送并回读。Notice 和公告两别名由有效最高管理员 Session 写入；API Key、普通管理员和撤销凭据拒绝。比较后保存、别名、版本与 actor 审计同事务；审计失败全部回滚，相同内容不增加版本。COMMIT 回执丢失返回 503 `SYSTEM_OPTIONS_COMMIT_UNKNOWN`，不声称回滚、不自动重试。其余设置继续明确部分可用，不能计为完整设置功能恢复。

正式冻结输入在 `~/Documents/Codex/2026-10-29-console-formal-50jdzil7/`：基源码 a853ea8 加 Console overlay，1293 文件；CoreSchema 只纳入 Console 元数据，未夹带控制 v2 或桥接候选公共入口。完整 Cargo dev-feature 图编译实际非 test main、库和两个测试目标，库 Clippy warnings denied/格式通过；实际 Core SHA256 `8d760cf7012eb4436001469406e4afeb6b6161001a64dd23d67ae824019692b7`。提交后逐个核对全部 1293 文件与 Git 内容一致。隔离 PostgreSQL 18 下七项库测试与一项受限账户 Router TCP 测试通过；这是开发自测，不是 release/S11。

独立实际 main 验收在 `~/Documents/Codex/2026-10-29-console-independent-attempt-05/`：十组全部执行 PASS，使用正式 init-db、create-first-owner、grant，受限运行账户、真实 HTTP 登录/保存/并发/撤销、审计失败回滚与两次 OS main 启动。代理将真实 writer COMMIT 送到 PostgreSQL，观察 CommandComplete(COMMIT) 和 ReadyForQuery(I) 后截断回执；HTTP 返回 COMMIT_UNKNOWN，另一个 SQL 连接证实内容和审计已提交，后续 HTTP 可读取且没有自动重试。测试代码已提交到 `tests/console/independent_process/`。 成功测试的两个可执行文件及 Clippy 中间物经桥接开发者确认无依赖后删除，共 288194422 字节；保留实际 Core、待桥接使用的库、冻结源码、完整命令、日志与哈希。

首轮独立记录依次保留用户 bus 环境、共享锁 BLOCKED、TOML 必填字段、显式注册策略和原生注册入口 403 的夹具失败；最终通过使用真实 Web 注册/登录准备普通用户，保留原生注册 403 边界。没有改产品输入或放宽 Console 断言。每次原 runner、日志和诊断均保留。测试进程、秘密配置、临时数据库已清理；十组 PASS 不扩展为浏览器或旧站点验收。

SQL 信任边界：runtime 无 options 直接 DML/DDL 或 owner/元数据权限；允许函数限制键、固定 search_path 并由 owner 持有。实测持 runtime 凭据可直接写允许键，绕过 HTTP 内容、别名和 actor 审计；这是现有受信任 Core SQL 身份的能力，不是数据库强制用户审计。HTTP 事务的审计、权限及内容已验，与直接 SQL 分开记载。配方保留完整 A10 分母。

旧库升级仍未完成。另一个 v2 bridge 在两个明确 v5/26 前驱到 v6/27 目标的隔离 PostgreSQL 上通过预览、结构核对及回滚开发用例；它没有已提交升级、持续围栏或持久恢复，产品未集成；r4 数据保持已完成前节所述独立复验。r4 已修正“所有历史邮箱一律拒绝”的错误前提：原 email/flag 值与旧密码别名保持，不创建 verified 地址或 reset 权限。真实冻结 Core 库及正式授权后的受限角色在两种前驱上完成别名登录、错误/重复/禁用别名拒绝、旧行/序列/ACL 保持及 reset 正反对照；一项合同测试、两项 PostgreSQL 场景开发测试通过。仅测试内部提交私有 stage，让真实池方法读取，不是产品 apply/围栏/恢复验收。证据在 `~/Documents/Codex/2026-10-29-console-bridge-v2-r4/`，r1–r3 的 212 个证据文件哈希未变；一条编译警告保留，未声称 Clippy、独立验收或 S11 通过。更早的账本/路由历史前驱仍须自己的桥接。

## 设置升级的历史候选及遗留失败

以下记录绑定当时的旧输入和候选，保留失败及缺失证据；新装 Console 当前状态以上节为准，不能将旧候选通过挪用到当前组合，也不将其旧新装未交付状态重复当成当前事实。


源码工作树已接通 Notice 和公告写路由，并为受限运行账户增加专用数据库函数。隔离 PostgreSQL 18 上，设置路由的七项本地测试通过；另一份冻结源码通过一项独立编写的 TCP 测试，覆盖实际安装、正式授权脚本、受限运行账户、管理员鉴权、公告别名、审计、并发冲突和失败回滚。这些是合成新装数据库和单个路由的测试，不是完整服务或生产验收。测试及输入清单原位于已清理的本机目录。

该受限账户用例已整理为源码项目内的 `scripts/test-console-runtime.sh`，运行通过；脚本只创建独立临时数据库。它与用例文件均尚未提交。

旧数据库升级仍未完成：旧 26 组件升级测试在新候选上实际报 `UPGRADE_DB_COMPILED_PREDECESSOR_DRIFT`，旧导入工具也因结构指纹变化拒绝候选。升级过程、历史合同兼容、受限账户完整启动和最终组合验收仍需完成。运行账户可直接调用允许键的数据库函数，绕过 HTTP 层审计与内容校验；其信任边界需要审查。候选尚未提交、推送或部署。

新的 Console 升级候选第二轮隔离 PostgreSQL 测试在回执篡改断言处失败，测试未打印实际返回结果；原日志和冻结输入已保留，但当时数据库及编译产物被运行器清理。修正测试诊断并增加离线连接检查后，后续一次测试越过该断言，最终在角色撤销场景失败：测试角色仍有 CONNECT 权限，新检查因此拒绝升级。该次失败数据库、编译产物和日志均保留。随后修正测试夹具撤销 CONNECT 的遗漏，使用隔离 PostgreSQL 18 重跑通过：真实候选测试、实际 CLI 升级及同请求重放、Clippy 均通过，冻结输入未变化；日志在 `~/Documents/Codex/2026-10-29-core-console-upgrade/attempt-5/`。该结果针对未提交候选，未证明持续停写、完整运行账户权限或提交结果不明时的持久恢复。

已从固定提交重新构建历史安装器，在全新隔离 PostgreSQL 18 中执行安装并核对全部 26 个组件和 6301 条结构定义，结果与此前记录的特定前驱一致。重建产物不是此前被替换的二进制，也不证明任何现有站点的状态。升级候选的实际新安装和受限账户 `check-db` 通过；其 6352 条结构定义与历史安装器执行安装后手工暂存候选 DDL 的结果一致。这尚不是升级命令或重放验收。

另一条历史路径已核对旧提交 `a398fa77b` 的实际安装：26 个组件、157 张表、21 个序列和结构指纹与旧合同一致。当前 `095c` 安装器拒绝这条旧账本升级，要求保留旧路由安装器；改用从历史提交 `0b4f40996` 重建且核对过的中间安装器后，合成库升级提交及同请求重放通过，旧表数据、序列和样例余额保持一致。升级后的 6165 条结构定义与当前新装库的 6301 条不同，尚未证明到设置升级前驱的桥接；这也不是现有站点或生产切换验收。首次隔离库启动因套接字路径过长失败，构建缓存曾误复用旧二进制；两者均已修正，原日志现不可用。

另一份隔离测试在历史安装器创建的数据库上执行候选升级，按预期拒绝提交并回滚；旧结构及 160 张表、21 个序列的状态保持不变。此前两次测试因目标库的 PostgreSQL 默认函数权限不符合离线角色合同而被拒绝，修正目标库的测试角色授权后回滚用例通过；原失败日志现不可用。随后，一份私有实验版在另一座合成旧库实际提交升级，新结构、旧数据、序列和授权核对通过；受限运行账户 `check-db`、写入后按同一请求重放和部分冲突拒绝也通过。带原会话身份的版本首次因角色无法读取会话启动时间失败；修正后的 v5 通过实际提交、截断 PostgreSQL COMMIT 确认后的回执恢复，以及提交前断线由专用恢复命令保持未确定且不自动重做的测试。但隔离测试复现了原操作结果未决时，另一个普通升级进程仍可重新提交；缺少持久围栏，候选不能用于现有库。v5 的旧组件升级与账本升级前驱检查都报 `UPGRADE_DB_COMPILED_PREDECESSOR_DRIFT`，按规则停止；实验版尚未合入源码，最终组合和现有站点授权仍未验证。

## 仍阻断完整目标

- 公开设置三项内容键的新装实际进程已独立复验并提交；完整设置功能、旧数据库升级和最终组合仍未通过。窄数据库函数不代替 HTTP 用户审计。
- 旧版待领取钱包转账创建时已扣除发送方余额；隔离账本实验确认，新版普通 reserve 再扣一次，opening 流水也不能直接充当可领取的 hold。私有账本候选在合成 PostgreSQL 中通过领取、取消、竞争及不重复扣款测试；正式安装与授权后的受限运行账户也通过实际 Rust Store 领取、取消和直接写入拒绝检查。该用例首次因测试用户名超长失败，修正后通过。两笔合成转账的 Core 批量入账与重放测试、Go 双库分享链接恢复及冲突测试也通过；它们仍是测试夹具。真实来源绑定、停写证明、提交结果不明的恢复及实际导入仍未实现。Core 保存令牌摘要，Go 扩展另存原令牌；迁移需要协调两侧记录、停写和失败恢复，仅导入 Core 不足以恢复发送者历史。
- 旧版请求键可包含句点、冒号及 UTF-8。Core 和 Go 已修复历史读取并保留新建请求原规则；独立测试使用真实 Go Handler、Core 进程和两个隔离 PostgreSQL 库，33 个 HTTP 请求覆盖历史状态、待领取链接恢复、用户隔离及账本余额不变。测试使用替代的 Go 会话提取器，Core 验证合成 Session；未执行实际领取、取消或完整浏览器登录。验收输入是 `999c189` 加这组修改和其他未提交工作，五个钱包文件与提交一致，但不是干净 `833a06e` 的完整组合验收。旧键按字节限长，而两侧现有数据库约束按字符限长；历史数据导入、数据库结构升级和排序规则仍未完成。
- Go 表升级的私有测试此前两次遇到 `operator is not unique: text || "char"`；本轮修正目录查询的类型转换后，在正常 Go 校验配置和合成 PostgreSQL 中通过 DDL 事务、回滚、记录及表权限保持、重复请求和运行账户 DDL 拒绝测试。原失败日志现不可用；该夹具的回执和停写条件不具备真实迁移所需的可信保证，真实升级仍未实现。
- 已提交的 Go 钱包请求键约束升级入口（`8098f68`）此前验证中：补齐夹具的合成加密密钥后，真实 PostgreSQL 复测失败，夹具没有切换 schema owner；冻结副本还漏带工作树的字节限长 schema，因此不能用该轮“接受当前结构”断言判定产品缺陷。原始结果保存在 `~/Documents/Codex/2026-10-29-wallet-upgrade-x8wg2va6/`。现已修正 owner、离线重放和目标库回执检查，并完整冻结当前 Go 输入至 `~/Documents/Codex/2026-10-29-wallet-upgrade-fixture-fix-39lv3xxk/`，后续结果见下一项。更早记录的日志 `~/.local/share/rtk/tee/1793217956_go_test.log` 当前不存在，无法重新核验。候选的事务锁不证明持续停写，也不包含原始转账令牌的双库导入。
- 上述正确组合复测中，错误输入/运行账户拒绝和提交回应丢失恢复两项通过；另两项因测试把不安全权限状态下的恢复拒绝误当成应返回“原操作不存在”而失败。按离线操作合同保留拒绝权限检查，修正期望后的两项定向复测在 `~/Documents/Codex/2026-10-29-wallet-upgrade-recovery-49b1m2_m/` 通过，冻结输入未漂移；上述结果分属明确的夹具版本，实际 Go 主程序随后编译通过；冻结副本无 Git 元数据导致的首轮版本标记错误已保留，显式关闭版本标记后的构建成功。独立验收仍未完成，原失败未覆盖。
- 使用上述实际 Go 宿主二进制，在隔离 PostgreSQL 中执行 `upgrade-wallet-db`，实际升级、同请求重放、原请求恢复通过；未知参数、换请求键和恢复前重新开放 runtime 登录均拒绝。证据在 `~/Documents/Codex/2026-10-29-wallet-upgrade-host-ofuo69_b/`，593 文件冻结与二进制摘要绑定并在运行后保持；这是开发自测，不提供外部持续围栏或历史 bearer 双库导入。
- 除上一条采用正常 Go 校验的重跑外，此前私有 Go 合成测试脚本设置了 `GOSUMDB=off`，不符合项目的依赖验证要求；这些结果只作为受限的开发证据，最终组合必须在正常校验配置下重新验证。
- 新增的私有旧转账来源检查草稿尚未验证：测试目录设置触发了同一个 `private_artifact_outside_backups` 拒绝，未运行到检查逻辑；按项目规则停止重试；原失败日志现不可用。它不提供实际导入能力。
- A01/A02 完整权限/登录/旧凭证/特权批准，A03 全部历史经济权利及单位/预留依据，A04 全量日志/用量/财务，A05–A08 真实支付/商店/供应商/助手工具和 A10 全页面/CLI 联合验收，尚无本轮最终组合证据。
- 历史 A05 候选记录（当前正式恢复结果见本页最新进展）：退款恢复曾存在跨进程断点：已提交源码中的 `services/extensions/internal/modules/payments/service.go` 对同一退款先失败后成功只记录冲突并返回，未安排 Core 入账任务；事件处理路径可能因此将该通知记为已处理。工作树候选涉及 Go 入队/旧 inbox 重放及 Core 两条退款入口；冻结输入的 Go 本地测试 101 项通过、7 项外部夹具跳过，Rust 实际构建和隔离 PostgreSQL Store 测试 27 项通过、1 项长时用例忽略。真实 Core main 与 Go 支付模块 worker 的退款响应丢失及重启恢复测试，在合成支付端和隔离库中通过；首次因缺少证据目录在启动前失败的记录仍保留。这是开发测试，不是 Go main、真实供应商或独立业务验收。失败后重新退款与迟到成功相撞时的对账解除流程仍未完成，不能把候选称为支付恢复完成。证据在 `/tmp/lmm-delegate/a05-late-success-4t2830_6/`，保留至待验完成。
- 历史 A05 容量候选记录（未包含本轮自动 Work）：`a05-capacity-v2-lruhcspr` 已完成开发自测：Go 101 通过/7 跳过，真实 PostgreSQL 29 通过/1 忽略，两个 Core/Go 模块 worker 联合恢复场景通过。其冻结源码清单 SHA256 `3117334bc9303c21034f4bedaa6c2de7397653e793a78a5dbad6f51ccd4b9e7b`；独立复验已安排。两笔成功退款合计超过原付款时仍缺权威净结算与账本合同并继续拒绝；302 秒过期、Go 宿主 main 和最终提交组合仍未验收。
- A08 Core agent 的显式配置、固定摘要工具注册表、`with_agent` 配置接线、新装第 27 个组件和列级运行授权已在 `67b4b90` 提交并推送；普通文本模式保留。显式启用时，错误注册表、策略漂移、禁用工具或不足权限会拒绝。535 个封存输入对应提交树 `c360f25a870285cbb4408487c660fa36f3f285f8`：真实受限 PostgreSQL 账户的配置加载检查执行七个场景，全部通过，模型供应商调用为零；实际主程序类型检查、定向 Clippy 和格式检查通过。证据在 `~/Documents/Codex/2026-10-29-core-agent-staged-agcoj9es/`。整包库内测试曾触及隔离内存/交换区上限，后续改用定向集成测试，旧失败保留。
- `8f829ef` 随后修复运维脚本的严格报告解析：旧 v1 记录缺少 `assistant_agent` 时仅视为未启用，显式要求 agent 时必须有 true 证明，未知字段和非布尔值仍拒绝。46 项脚本测试通过；首轮 `/tmp` 路径被祖先权限检查拒绝，第二轮证据目录被“仅允许 Backups 输出”规则拒绝，改用私有 Backups 夹具目录后通过，未放宽检查。证据在 `~/Documents/Codex/2026-10-29-core-agent-ops-33o9zx3d/`；对应 Rust `check-db` JSON 断言已更新且格式检查通过，该 CLI 测试本轮未运行。A08 仍是开发自测，独立验收未完成；这次脚本修复时尚未运行真实 Core main 启动/UDS，后续结果见下一项。Go 工具委托、资金链路和存量库升级仍未验收。先前冻结 26 组件的升级结果不适用于本次正式 27 组件的最终组合。
- `516fef3` 新增的正式 Core 进程测试已提交并推送。535 个冻结输入对应提交树 `9627bfbed77711bd9ef7c8064d370525a9bf8146`：真实非 test 的 debug Core 二进制摘要 `94a427baa6bf6bd17c9dd8adee92205cb88551034867d52cea64429753ff6f38`，受限账户下 HTTP 就绪并报告 models/events，未认证 UDS 请求拒绝，已认证控制 RPC 返回 identity 与 agent 执行能力；撤销 agent 必需 INSERT 后，第二个真实 Core 进程明确以 MissingPrivileges 拒绝启动。七个原配置场景、四项真实 `check-db` 测试、定向 Clippy 和格式检查全部通过，上游模型调用为零。首轮测试引用了未生成的 RPC 客户端而编译失败，改用已有 protobuf 消息与 tonic 客户端传输后通过，产品代码未因此改动。原失败源码/清单/位置映射及全部日志保留在 `~/Documents/Codex/2026-10-29-core-agent-main-42g893vz/`；成功的临时 PostgreSQL 数据与套接字已停止并清理。这是开发自测，不证明 Go 工具、对话资金闭环、存量升级、release 制品、完整资源门槛或生产切换；独立验收未运行。
- Go 宿主的 `assistant.agent_required` 启动校验已提交：显式要求 agent 时拒绝缺失依赖和仅提供普通文本能力的 Core。配置解析测试通过；真实 Unix socket 定向测试首次因测试路径超过 100 字节限制失败，缩短夹具路径后通过。首次运行还因 Go 临时目录不存在在编译前退出，原始错误与修复后结果分别保留在 `~/Documents/Codex/2026-10-29-assistant-agent-gate-q9dswm_i/` 和 `~/Documents/Codex/2026-10-29-assistant-agent-gate-fix-w8_swn2s/`。这是局部开发测试，尚未独立验收或证明完整 agent 链路可用。
- Go 助手启动门禁后续已在 `6ae3081` 固定输入上独立通过 12 组/62 个子用例，包括 6 次实际 Go 进程启动及真实合成 UDS；未失败、未跳过。覆盖凭据复用、能力/registry 不匹配、缺依赖、默认可选行为及启动后断线拒绝。证据在 `~/Documents/Codex/2026-10-29-go-agent-gate-u7wk2vag/`；这不证明真实 Core agent 已装配，也不代表当前整体组合验收。
> 以下 A02 条目保留正式接线前的过程和失败证据。当前新装、Core/Web 接线及本地 SMTP 进程验收状态见本页“最新进展”开头；存量升级等未验收范围仍保留。

- A02 的密码注册尚不支持邮箱验证码：配置要求邮箱验证或域名白名单时，当前密码注册会被拒绝；公开状态仍报告 `email_verification=false`。旧版的发码、绑定、找回密码、限流及验证码消费行为需要在 Core 实现，并与 Web 调用对接；邮件服务真实配置和投递未验证。
- A02 HTTP 与重置邮件/Web 兼容候选保存在源码 `services/core/staged/email-http-integration/v2/`：从配置的站点 origin 生成固定重置链接，页面读取 fragment 后清除凭据地址；8 项实际页面启动 JavaScript 测试通过。Rust 编译、HTTP/PostgreSQL 用例、正式启动和 SMTP 闭环尚未运行；补丁未应用，已安排独立验收。
- 该 HTTP v2 候选的独立文件依赖检查发现两个 `include_str!` 相对路径失效，当时后台测试构建尚未执行。后续 v3 已修正路径并重新生成补丁；8 个 Core 与 3 个 Web 目标均精确应用，450 个冻结基线输入未漂移，临时副本已清理；后端后续结果见下一项。
- A02 v3 冻结候选后续完成库目标和测试目标编译、定向 Clippy 与格式检查。同一未改动二进制的两项重置链接纯测试先通过，七项 HTTP/真实受限 PostgreSQL 测试首次被夹具路径守卫拒绝，修正自有临时路径映射后分别通过，每次准确执行一项。覆盖注册、绑定、重置、凭据撤销、MFA 边界、隐藏资格和通知入队失败回滚；这是进程内 Axum 请求，不是正式 Core HTTP 服务。首轮编译 OOM、启动器准备失败与七项夹具失败均保留。候选输入清单 SHA256 为 `55b485e8396833b46b121d4f4723a48ff7ff6b68f7edf1762be8bdcd16b3c879`，未提交或启用该整包候选。
- `a5c0512` 单独提交并推送现有暂存邮件组件的重置链接实现，要求显式验证的站点配置，沿用已有证明和过期时间。实际 lettre 向本地 SMTP sink 发送，解码真实 DATA 后验证链接及凭据单次消费；精确暂存树 `7c212d3a5a28f8f7e1dfca7c0439bf6a1dab079d` 的组件测试 19/19、Clippy、格式、runner 语法和维护文档检查通过，排除并行通知/schema 改动。独立验收未运行，未接正式 Core 邮件启动或完整 HTTP-to-SMTP 进程链路；临时 PostgreSQL 均停止并清理。完整证据与原始失败在 `~/Documents/Codex/2026-10-29-email-http-backend-ozs8x_et/REPORT.md`。
- A02 安全通知已有待集成补丁：消费验证码、业务修改和加密通知入队在同一事务内完成，绑定时当前与原已验证地址使用不同收件槽。开发者在固定输入的隔离 PostgreSQL 上通过 8 项测试，包括入队失败回滚和两处进程终止；证据在 `~/.local/state/api-lmm-microservices-20261011/coordination/architecture-completion-v1/A02-EMAIL-NOTICE-INTEGRATION-20261029/`。五个目标文件的补丁已应用到本地源码工作树，哈希与测试快照一致；组合接线仍只在隔离测试中，未独立验收，也未接正式 schema、权限、HTTP 或邮件投递。已提交接口中的上述事务缺口依然存在。
- 独立源码审查发现上述通知的测试用 `schema-delta.sql` 对已有通知统一回填 `current`，而旧接口可能向原已验证邮箱发送通知；带数据升级会错误占用槽位。修正候选现在锁内拒绝全部旧安全通知，且只允许 READ COMMITTED；该隔离级别与当前正式升级的 SERIALIZABLE 不兼容，不能直接接入。新候选的 PostgreSQL/Rust 测试尚未运行，原 8/8 不证明新输入通过。该 delta 本来就未授权用于正式库；证据在 `~/.local/state/a02-notice-independent-20261029-icw2i5pq/`。
- A02 的 identity v6 新装 schema 与运行权限补丁位于源码的 `services/core/staged/email-schema-integration/`，尚未应用。隔离 PostgreSQL 的开发测试覆盖新装与测试用 v5 delta 的结构一致性、历史邮箱不自动变成已验证、失败回滚及受限角色权限。首次权限检查因现有脚本缺少 `core_meta.console_revisions` 的 SELECT 而失败，候选修正后针对该失败的复测通过，原失败保留。实际 Rust 新装测试未启动；正式升级分发、持续停写、旧二进制行为和独立验收仍未证明，测试 delta 不可直接用于存量库。
- 上述 v6 候选后续独立 PostgreSQL 验收未通过：给 runtime 额外授予 `console_revisions` SELECT 与现有 Console owner-only 合同冲突；两份旧邮箱夹具重复应用新装后已存在的列而失败；列级 `REFERENCES(credential_id)` 超额授权没有被候选检查识别。作者正在修复。新版通知测试 delta 的锁后并发检查和对有旧安全通知的拒绝已独立验证，但其 READ COMMITTED 限制与正式 SERIALIZABLE 升级仍不兼容，不能将拒绝当成迁移成功。证据在 `~/.local/state/lmm-acceptance/a02-identity-v6-20261029-u3z0myjc/`。
- identity v6 revision-2 后续独立真实 PostgreSQL 复验为 98 条断言中 97 通过/1 失败：原 Console 冲突、REFERENCES 漏检和重复 DDL 已修复；challenge-only 夹具消费合法 challenge 时触发器因 outbox 权限不足回滚，delivery 夹具相同操作成功，仍不放行。原作者继续修复；Rust、实际 Core 启动和正式升级未验。证据在 `~/.local/state/lmm-acceptance/a02-identity-v6-r2-20261029-3488672t/`。
- A09/S11 最终源码与制品/进程绑定、授权只读来源与目标、最终写入隔离及 drain、备份实际恢复、1CPU 限额资源验收、生产环境自身条件和切换演练，尚未在本轮证明。
- 升级控制候选的独立 release 二进制构建失败：冻结源码的 `src/admin.rs:115` 调用 `UpgradeCommand.clone()`，该类型没有实现 `Clone`，因此未生成可验收二进制。此前通过的是库的定向测试，不覆盖实际 CLI 编译；原始日志在源码的 `.local-tests/upgrade-v2-independent-release-20261029T0750/release-build.log`。作者补充 `Clone` 后，在固定快照运行 `cargo check --release --bin lmm-core` 通过；这是类型检查，尚未链接或运行正式 CLI，独立复验仍待完成。当前工作树另有 9 个路径与该快照不同，不能把检查结果当最终组合通过。
- 上述 Clone 修复版本后续已完成实际 non-test release 链接与运行；563 项冻结输入的清单 SHA256 为 `269cb3bdc636fa58c4296dfc50807126a0ffefd1ef20100809e8972b088ece49`，实际 CLI/PG 第四轮 189 项及编译/事务日志 16 项断言通过。READY/STARTED apply 都拒绝，recovery 保持 UNKNOWN；不存在产品生成的 COMMITTED 双回执。其业务 owner 仍能写入，未证明持续外部围栏、独立历史见证或当前新装组合。证据在源码 `.local-tests/upgrade-v2-independent-release-recheck-20261029T0909/`，前三轮夹具失败保留。
- 同次独立 PostgreSQL 检查确认，升级控制处于 READY 或 STARTED 时业务 owner 仍可直接写表；事务中断与已提交 STARTED 的隔离结果不能替代外部持续停写围栏和独立历史见证。CLI 未生成，因此 apply、recover、权限攻击和断线矩阵均未执行。完整结果在源码的 `.local-tests/upgrade-v2-independent-release-20261029T0750/REPORT.md`。
- A09 围栏/见证候选在源码 `services/core/staged/upgrade-fence-integration/` 有 23 项开发自测证据；其中 owner 重新开放数据库和可信 UID 回滚本地见证的反例继续保留。后续独立审查冻结了 165 个候选文件并运行文件见证观察，但 18 组独立 PostgreSQL 时序因资源条件未执行；不能抵扣为通过。审查指出临时开放维护连接后管理连接死亡时的重新禁连恢复风险，以及部分禁连探针先被 HBA 拒绝的归因问题；作者继续修正。正式 CLI、全部写者 drain、独立外部见证和完整 S11 仍未验收，正式发布不放行。
- 预检 READY 仍只是受信任输入断言的一致性，不能把合格形状的 hash 或手填 passed 当生产 go/no-go。

## 证据与限制

完整命令、冻结输入 SHA256、原始失败、开发自测和独立复验原位于本机私有 `~/.local/state/lmm-api-production-readiness-20261028/`，该目录当前不可见。`~/Backups/lmm-prod-20261028/opening/` 目录仍存在合成 PostgreSQL 回执和 dump；尚未重新核验其完整性。没有将私密材料放入配方或 Git。

此前记录了首次 CLI 缺必需参数、直接使用 Backups 根父目录被路径检查拒绝的失败；后者未创建数据库，修正为新的私有嵌套输出后独立执行一次通过。它们不被计为产品通过。

本轮没有运行 CI、访问 DMIT 或其他生产服务器、操作真实资金、创建生产测试账号、部署或切换流量。目标保持 active；局部复验通过不代替 A01–A10/S11 的完整完成审计。
