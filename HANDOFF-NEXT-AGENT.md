# 下一位 agent 接手说明

用户于 2026-11-04 明确要求组织子 agent 接手并继续完成任务，原 STOP 已解除。当前正进行本机 Core55/OAuth 修复及独立验收；仍不访问 DMIT、生产数据或真实付费服务。下方旧检查点和失败证据保留，不能把开发者自测视为独立验收。

## 入口与边界

- 工作区 `/home/lightjunction/Documents/GITHUB/lmm-api`，实现位于 `generated/lmm-api-source/`。先读两层 `AGENTS.md`、本文件及原 `HANDOFF-2026-11-03.md`；后者仍是本地未跟踪文件，不随本提交分发。
- 单一 `main`；不新建分支或 worktree，不 reset、clean、强推。先检查实际 HEAD 和工作区，保留他人工作。
- 不触碰 DMIT、生产数据、生产账号或真实付费调用。生产没有切换；最终切流必须取得用户明确确认。
- 遵守用户排除私人归档目录及其引用的要求。仅使用当前仓库和下列新建 `.local/state/lmm-api/` 证据，不搜索其他归档或从旧缓存恢复输入路径。
- 父仓库 `recipes/lmm-api/COMPLETION_SCOPE.md` 的既有两行修改，以及 `.pi/`、原交接文件和 `INTEGRATION-2026-11-03.md`，本轮未纳入提交；不要误删或覆盖。

## 2026-11-04 接手进展

- 源码 `7d1b0ba` 已提交并推送：consent CSP 仅增加当前已验证的精确 loopback 回调；Core55 正式 7 列授权、撤权与启动 guard 同步；导入器新增独立、严格的 fresh55 profile，不重标旧 receipt。
- 开发者自测：OAuth 3 项通过；importer 124 项通过、1 项跳过；真实合成 PG 上逐列撤权拒绝、正式 revoke/regrant 和 Core→Go 四类历史漏斗通过。首轮遗漏 Go DB 环境变量导致的失败日志保留，不能写为首次全绿。
- Go acquisition/community 包测试、Go 构建、Web 构建及 bundle 检查通过。完整 Core debug 候选正在构建；不是 release 或部署产物。
- 独立验收尚未运行。新夹具必须实际收到浏览器 allow/deny 回调，禁止从 Location 头回退取 code 后冒充浏览器成功。历史异常时间混合有效事件的漏报疑点待实际验证。
- 新证据入口：`/home/lightjunction/.local/state/lmm-api/core55-acceptance-20261104/`、`connection-acceptance/run-02-prep/`、`connection-filter/core55/permissions-20261104-complete/`。冻结候选生成后以其清单为准，不能使用可变 target 替代。
- 广告付费仍未实现：当前新广告返回 503，隐藏退款为 pending_core。Core 现有 Commerce 仅表达商店订单，广告合同草案在本机 `ad-payments-20261104/CONTRACT.md`，不是已实现功能。

## 提交状态（原暂停检查点）

- 源码 `5194aca` 已推送：明确的 Core54 fresh import 合同。`4c7f337` 已推送：Core54 实际 CLI manifest 测试修正。
- 源码 `a49db97` 为 STOP 时的 **WIP 检查点**：20 个 Core55 历史连接导出、导入、查询及漏斗文件。保存现场不等于联测通过；不可部署。
- 父仓库本交接提交固定到该 WIP gitlink。接手时读取实际 Git 提交，不将冻结 Core54 二进制当成当前 Core55 编译结果。

## 恢复后的优先顺序

1. 修复真实浏览器 OAuth consent 的 CSP 失败。`services/core/src/legacy_auth/oauth2.rs` 的 consent 页 `form-action 'self'` 阻止表单提交后跳转到已验证的 loopback OAuth 回调。尚未改代码。检查 `identity/grants/issuer.rs`、`identity/grants.rs`、`identity/grants/protocol.rs` 的数据流；保留既有 CSRF、PKCE 及严格回调验证，不能宽泛移除 CSP。修复后必须由独立验收重跑同一浏览器流程，协议成功不能替代浏览器验收。
2. 审阅 Core55 WIP，补齐正式最小列授权和启动权限检查，再继续 Core→Go 历史漏斗联测。目前撤销新增列权限后 guard 仍返回 `Ok(())`，预期 `MissingPrivileges`，因此完整历史联测失败。此前仅在开发者私有 PG 临时授予拟定 7 列后证明过实际 SQL 可查询。
3. 冻结完整 Core55 候选，显式审阅 importer 的 Core55 profile。当前 Core54 合同必须拒绝未知55，不能放宽后缀或重标旧 receipt。完成后重新进行独立验收及 Core、Go、Web 组合构建。
4. 按原交接和配方完整验收范围继续资金、历史权益、获客、排名、支付、图像、商店、助手、工具及回调等缺口，最后完整 fresh DB 演练。安装与切流仍未完成，现阶段不访问服务器。

## 可用的新证据

根目录 `/home/lightjunction/.local/state/lmm-api/`：

- `connection-acceptance/REPORT.md`：独立 Native 验收，四种非空合成连接分组与 SQL/Core/Go/Web 对齐；29 个 API 断言及四组 UI 筛选通过。OAuth 浏览器 consent 的 CSP 错误仍为 FAIL。验收服务已停止。
- `community-runtime/frozen/candidate.json`：冻结 **Core54** 候选及源绑定。配套 `lmm-core`、`lmm-extensions`、`grant-core-runtime.sql`。不能被可变 debug 产物替换。
- `manifest54/tests.log`：冻结 Core54 实际 CLI 的4项测试通过；不证明当前55全套 Cargo 通过。
- `contract54/audit.json`、`source-freeze.json`、`unit-results.json` 及 final 日志：开发者报告 importer 123项（1跳过）、financial 40项、credential 37项通过。私有 receipt 不要公开打印。
- `connection-filter/native-YinxQrEQ/SELFTEST.md`、`connection-filter/core55/progress.json`：Core55 开发者进度与失败现场。

Core55 开发者已报告通过：catalog pin、CLI4项、Python3项、Native 回归，以及合成历史快照导出导入、回读、错误绑定、原子性和重放拒绝。前54 SQL 未改。这些不是独立验收；真实旧源迁移、55完整导入 profile、正式权限联测及历史 Go 漏斗仍未完成。对应私有 PG 55484 已停止。

Core54 profile 只覆盖 fresh import；旧49合同历史校验保留，但当前命令不执行49导入，也没有49→54已安装站点升级方案。不得把旧 receipt 重标54。

## 总体状态

项目未完成，不报告完成百分比。原先验证过的登录、余额、API key 等结论不能自动扩展为本轮资金功能、历史迁移或端到端验收通过。缺少允许使用的真实输入时保留 UNKNOWN，不伪造对账。自测、独立验收、提交推送、发布和部署必须分开报告。
