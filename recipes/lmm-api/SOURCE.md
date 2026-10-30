## 来源与证据边界

观察日期：2026-10-28。原实现仓库：https://github.com/TokenNotIncluded/lmm-api ，行为观察提交 `d468affecfd76a5123358732fb9a4e0cc1f63566`，迁出基准 `ff01f8456b978089bce08175cbd357c3d22c3983`；CLI 固定为 https://github.com/TokenNotIncluded/lmm 的 `23de1322d68a021707eda876ed477e71d72f9544`。实现已迁到 https://github.com/TokenNotIncluded/lmm-api-source ，初次迁出提交 `06d8b4b7c9630106c6ef47425f9e67d333bd1548`；当前实现版本以主仓库 gitlink 和 STATUS.md 为准，由当前主仓库 `generated/lmm-api-source/` 子模块接入。它不是配方依赖，不进入归档。

初次提取配方时只做源码、文档及已有测试定义的静态观察，未重跑应用、数据库、浏览器、支付或性能验收；后续修复和验证按版本单独记录。源码存在、测试定义、历史通过说明均不代表固定版本已通过验收。原开发工作区未提交内容全部排除；包括 Passkey、模型预检、共享读配额、价格修改和旧单体回迁准备脚本，不能将其当成已发布功能。

旧单体 `传统` 与 `traditional` 固定于 `3948a0a1ee1cefdd016a7aa45a11851545bb4b85`；当前微服务默认分支为 `main`。固定源码中仍有旧分支名的文档和工作流引用，应按提交识别观察对象，不能按旧文案取错源码。

项目基于 QuantumNous/new-api，保留上游及其他贡献者归属、AGPL-3.0-or-later、NOTICE 和第三方许可证。目标不要求复制实现；若复用代码、素材、字体或其他受保护材料，逐项保留许可证和归属。商业许可状态未知，不据此扩大授权。

## 0.1.1：2026-10-29 Issue 与 PR 维护

本轮从源码 `86e51b0708086fb78605ef9f519275149ac47873` 核对当前问题，补齐定价排序、上游流截断的公开错误分类和完整质量计划的 Mocklab 入口。依据为本项目 [#911](https://github.com/TokenNotIncluded/lmm-api/issues/911)、[#945](https://github.com/TokenNotIncluded/lmm-api/issues/945)、[#786](https://github.com/TokenNotIncluded/lmm-api/issues/786)；已有动态分类和 SSE 分帧分别按 [#961](https://github.com/TokenNotIncluded/lmm-api/issues/961)、[#962](https://github.com/TokenNotIncluded/lmm-api/issues/962) 重新核对。实际自测、原始失败和未运行范围由 STATUS.md 及关联源码 PR 记录。

ISSUE_MAINTENANCE.md 完整承接旧 #879/#875/#942 的剩余需求和验收。旧 PR 关闭为迁移取代不代表完整财务导出、支付供应商或 schema 升级完成；原候选与已通过的历史场景均保留。

## 0.1.2：2026-10-29 邮件正式接线

行为实现来源为 [lmm-api-source `5733972`](https://github.com/TokenNotIncluded/lmm-api-source/commit/57339728292d522ca7a4f926f1624301b5cc9350)：正式 identity v6 新装与运行权限、加密出箱、配置及 Core/Web 邮件调用。固定输入的实际非 test Core、本地受限 PostgreSQL 和真实 SMTP 传输已完成独立注册/重置/撤销/重放/启动拒绝/退出等待验收；Web 定向测试与类型检查分别记录。它不证明真实浏览器、外部邮箱、存量升级、完整资源门槛或生产切换。完整范围、原始失败与未运行检查继续由主项目 STATUS.md 保留，不能将邮件切片推定为整站完成。

## 0.1.3：2026-10-29 公开设置写入

行为实现来源为 [lmm-api-source `f993be2`](https://github.com/TokenNotIncluded/lmm-api-source/commit/f993be2c860fded95af706527ff5b8705c52cd1c)：Notice/双公告别名、最高管理员 Session、CAS、原子审计与提交结果不确定处理。固定 1293 文件的实际 dev Core 在新装受限 PostgreSQL 上完成七项库测试、一项 Router TCP 开发测试，以及独立实际 main 的十组 HTTP/SQL 验收，含数据库已提交后丢弃回执。受信任 runtime 的直接 SQL 能力与 HTTP 用户审计分别记录；它不是数据库强制所有用户审计。只开放当前三项内容键，不缩小完整系统设置/A10 要求；浏览器、存量升级、release/S11 和生产切换未验收。原始失败、构建来源、哈希及清理由 STATUS.md 保留，不宣称从配方完成重建。

## 0.1.4：2026-10-29 独立升级历史库

实现来源为 [lmm-api-source `7c3ebb8`](https://github.com/TokenNotIncluded/lmm-api-source/commit/7c3ebb86cf406c5f0a268ea2eb90edb985271751)：同一 Core 的独立历史库安装/查询 CLI 和真实 SQLx CAS 客户端。固定开发图编译、Clippy、格式及隔离两实例测试通过；另一个 owner 独立验证实际 CLI/API、四消费者仅一次、物理目标恢复、真实 COMMIT 回执丢失、历史库进程停启和 writer 权限。历史库只保存原操作及观察，未接通完整升级 apply、部署围栏、最终恢复/开放；不据此宣称生产可切。新装组件/manifest 未改变，完整分母及失败继续由 STATUS.md 保留。

## 0.1.5：2026-10-30 正式离线升级与恢复

实现来源为 [lmm-api-source `67dc5ba`](https://github.com/TokenNotIncluded/lmm-api-source/commit/67dc5bac6667ad796bdcc496ec7de9952dbabf07)。同一 Core 的 `upgrade-offline` v3 接通库外历史、外置 HBA、明确部署单元的停机/持久 mask、固定前驱桥接、原子本地回执与恢复观察。v6/27 的实际 CLI 普通提交、目标/历史库真实 COMMIT 回执丢失及目标物理恢复开发测试通过；独立所有者另外验证控制进程在临时允许连接窗口被 SIGKILL 后，真实 PostgreSQL ErrorResponse 仍以 28000/HBA 拒绝 runtime，恢复后禁止重做或开放。独立 SQL oracle 核对 160 张旧表、22 条序列及 2001 条 ACL/owner 记录；runtime_contract 仅按固定目标更新。1308 个编译输入与该提交逐文件一致。

固定 manifest 为 `47ad36927c16ccc4237cf16c9889705ae4d53db184c77aaac4a264c4e5203078`，catalog 为 `557d9775042b94356f752ae4993bc73be645e9956e9fe1110ed5aa1cd531ee2d`。v5 的先前桥接数据保持证据不等于本轮正式 CLI 验收；v5 正式全流程、多主机存储、完整历史资金迁移、最终开放、发布资源/S11 与生产切换仍未验收。浏览器 Notice 的失败处理也在该提交的前序 `90c442d` 修复并完成限定范围复验。实际结果与保留失败见 STATUS.md。

## 0.1.6：2026-10-30 升级后的运行权限与旧身份

实现来源为 [lmm-api-source `fcb29cd`](https://github.com/TokenNotIncluded/lmm-api-source/commit/fcb29cd98950053ab3a76e29548d8752366baa22)。实际升级后的新 Core 曾因错误要求读取 owner-only 升级回执而以 `MissingPrivileges` 退出。现仅对六张公开合同保留明确读取要求，原账本读取及元数据写入/owner 限制保持；两张私有回执的表级或列级读取误授权均拒绝。manifest/catalog 与0.1.5一致，没有给 runtime 增加回执权限。

修复候选 SHA256 `dea60909b00bed72db09772200db99233b9eb68bdb9a62197aa38288786d71aa` 的完整 dev 图库/main 编译、库 Clippy 和实际升级后登录/公开内容读写通过；另一个 owner 独立验证实际 v6 升级后受限进程、SQL 拒绝矩阵及权限缺失/误授反例。两种合成 v5 前驱也独立通过真实 CLI apply/recover、旧行/密码/21条序列保持、旧别名登录与未验证邮箱无重置权；其 `services=[]`，没有运行真实旧 v5 制品。全部是本地 PG18 和明确夹具开放，不是产品 finalization、全站资金、完整浏览器、发布/S11 或生产切换证据。原失败和未运行范围见 STATUS.md；1308项真实编译输入与修复提交逐文件一致，没有独立重编。

## 0.1.7：2026-10-30 原生性能与渠道权限

实现来源 `0b54719` 的真实relay性能采样/个人只读查询和 `e106a74` 的ChannelRead gate；实际R2 Native SHA256 d6d356311de8cf6589c2ec4429603a0d9308ca634b1aed20b78a6c42b7c76c10，dev图编译与库Clippy通过。开发真实Native/PG/TCP供应商通过；独立性能8组、独立渠道7组/45次HTTP通过。采样为SERIALIZABLE最佳努力追加，不成为资金或重派权威；历史/完整入口未覆盖，财务仍503、渠道写/测试/余额仍501。cfg(test)后续改动未编译、三个不同时间桶边界/高负载、完整历史导入和浏览器/整站/生产切换仍待验。完整绑定、保留失败与命令在主仓库STATUS.md及其证据目录。

## 0.1.10：2026-10-30 前端视觉与 HTML-in-Canvas

集成 Web 配方 `0.1.6`，实现固定为 [lmm-api-source `6523c4b`](https://github.com/TokenNotIncluded/lmm-api-source/commit/6523c4b4b831643d69de3874c2e35933c91bfc2b)，测试源码树为 `f55d28d7e32314d427aa682c5fcbb722246e26a1`。首页连接场景、普通 DOM 回退及全站共享视觉层已更新；70 个不同定向用例、完整类型、最终构建和包体预算通过，云端 Chrome 的限定公开页面和窄屏检查完成。实际连接场景使用 DOM 回退，原生 Canvas、登录后的控制台及数据和独立业务验收未执行。具体范围、首次裁切修复和 lint 排除项见 [Web 来源](../lmm-web/SOURCE.md) 与 [STATUS.md](../../STATUS.md)。根配方检查与完整提示词构建的实际结果见 STATUS.md；原有业务缺项和生产切换结论保持。

## 0.1.11：2026-10-30 原生钱包五类支付网关

以旧单体 `3948a0a1ee1cefdd016a7aa45a11851545bb4b85` 的五 provider／七钱包模板为对照，集成 Core 独立验签与入账、Go 多渠道适配、Web 报价确认和既有原生库的固定 28→29 升级。FastPay 明确退役，不加入实现或可选配置。[支付对照矩阵](PAYMENT_PARITY.md) 分别保留订阅、店铺、Pancake 管理和退款缺项；本轮不能据此声称整个新架构已完成。

实现来源为 [lmm-api-source `7abe06d`](https://github.com/TokenNotIncluded/lmm-api-source/commit/7abe06d5d369b2ccefe4364099450bfae4534953)，详见 [STATUS.md](../../STATUS.md)；对应源码的 `docs/payment-gateways.md` 与支付模块 `VERIFICATION.md` 记录实际检查、保留失败和未运行范围。开发自测不等于独立验收或生产切换；本轮未部署、未访问真实支付账户或产生真实费用。

## 0.1.12：2026-10-30 Coinbase Business 钱包支付

实现来源为 [lmm-api-source `7520d0b`](https://github.com/TokenNotIncluded/lmm-api-source/commit/7520d0bc1d71c080e6d787bf5f7ad1bfa9434c54)。新增 USDC checkout 与同额 USD 钱包报价、请求绑定的 JWT、持久 UUIDv4、Core 独立 v0 验签、七语言入口及第 32 组件的显式 31→32 升级。渠道默认关闭，自动退款与订阅不包含在本轮适配中。

Go 竞态与接线、Web 9 项定向/完整类型检查、真实 PostgreSQL 请求键持久化及 schema SQL 等价检查通过。完整 Rust 编译/测试受固定依赖不可用阻塞，真实支付商与独立验收未运行；实际范围和原始失败见 [STATUS.md](../../STATUS.md) 与该源码的支付验证记录。提交和配方验证没有执行生产升级或部署。
