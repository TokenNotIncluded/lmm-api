# Assistant UI / 权限 / 隐私审计

审计日期：2026-08-13
审计范围：apps/web/src/features/assistant、相关 AI Elements、apps/api-go/router、assistant controller/model/middleware，以及现有 assistant 测试。
审计方式：静态证据审计 + 只读定向测试；没有修改现有源码、数据库或运行时配置。当前工作区本身存在未提交变更，本报告只评价审计时看到的当前文件内容，不将这些变更归因于本次审计。

## 结论摘要

| 项目 | 结论 |
| --- | --- |
| OpenAI 式输入框 | 基本具备：Enter 发送、Shift+Enter 换行、IME 保护、长度限制、加载态和模型重试；核心 textarea 的可访问名称仍不明确。 |
| 可折叠历史 | 有桌面/移动端历史视图切换、active/archived 筛选和 owner-only 归档；聊天轮次没有把服务端 conversation ID 续传，持久化历史会碎片化。 |
| 无障碍 | 移动端 Sheet、按钮名称、筛选的 aria-pressed 较好；全屏自制 dialog 缺焦点管理，消息错误/完成态和输入框命名仍有缺口。 |
| Anthropic 风格 | 全局默认主题确实是 warm cream / clay / editorial serif；assistant 自身主要使用通用 shadcn surface，风格一致性是部分满足。 |
| L0/L1/L2/管理员 | 服务端有二次工具授权和管理员路由保护；L0、已获 L1、管理员分支清楚，但 L2 语义分散在 trust rank、developer grant 和 capability 字段中，未形成一份明确的权限矩阵。 |
| 低等级历史与遮蔽 | secure card 的非所有者遮蔽、一次性 owner reveal、服务端入模/入库脱敏是有效控制；但当前“严格高等级”不只包含管理员，且正则脱敏不能保证覆盖所有敏感内容。 |
| 失败重试 | 聊天有前端五次瞬时错误重试和消息级 Retry；历史列表/详情的失败页写了“Try again”但没有重试操作。 |
| L1 申请不丢失 | 正确方向是先写 durable request、再创建短时确认 flow，且数据库层会合并 pending；但排队失败被吞掉，不能把“不会丢失”作为可靠保证。 |

P0 未发现。P1 需要在发布前处理的主要问题是历史会话连续性、跨用户读取政策与管理员入口、敏感内容边界、L1 durable 失败语义，以及核心输入/历史错误的可用性。

## 已验证的保护基线

- /api/assistant 普通入口由 UserAuth 保护，assistant admin 路由由 AdminAuth 保护：apps/api-go/router/relay-router.go:69-106；通用 middleware 定义见 apps/api-go/middleware/auth.go:101-110。
- L0 首轮工具目录只允许服务事实、成本、账户状态、引导、搜索、L1 推荐和人工支持；已获 developer access 可获得非管理员工具；管理员才进入 AdministratorMode：apps/api-go/controller/assistant_agent.go:321-369。
- 服务端在模型调用前和持久化前执行历史内容脱敏，历史模型测试验证了 API key、邮箱等不会以原文存储：apps/api-go/model/assistant_history.go:138-147,338-345、apps/api-go/model/assistant_history_test.go:37-59。
- secure card 对非所有者只返回 protected metadata；真正 reveal 同时要求 owner、浏览器 session、未过期且未消费：apps/api-go/model/assistant_history.go:500-509,714-778、apps/api-go/controller/assistant_history.go:145-180。
- 管理员变更仍是 assistant action + 显式确认，而不是模型输出即执行；L1 也是确认后才提交，UI 文案明确“AI recommendation 只是 draft”：apps/web/src/features/assistant/assistant-activation-tool.tsx:234-290。
- 现有定向测试结果：web assistant 三个测试文件共 36 pass、0 fail；Go history 测试命令 go test ./model -run 'TestAssistantHistory|TestAssistantConversation' -count=1 通过。测试覆盖了已存在的重试、隐私提示、移动 Sheet、低等级历史展示和 secure card owner 限制，但不覆盖下述缺口。

## P0

### P0-未发现立即性的账户接管或凭证明文越权

**证据**

管理员配置/定价写操作有单独 admin 路由，secure card reveal 没有把密文或 plaintext 放进非所有者历史视图；模型层测试也明确 L1 不能 reveal L0 的 card，只有 owner 可以 reveal：apps/api-go/router/relay-router.go:96-106、apps/api-go/model/assistant_history_test.go:169-183。

**建议**

把以下控制作为后续改动的 release gate：非所有者永远只能得到 protected card metadata；reveal 必须继续走浏览器 session、owner、过期和一次性消费检查；管理员 action 仍须显式确认。

**不应破坏**

不要为了补齐管理员历史入口而复用 card reveal 接口、返回 ciphertext，或让“能读低等级 transcript”继承“能读取低等级凭证”。

## P1

### P1-01：服务端 conversation ID 没有被前端续传，历史和 Retry 会产生碎片/重复

**证据**

- 前端 POST 只发送 message、messages 和 attempt header，没有 conversation_id；响应类型只解析 choices、intent、action，没有解析 lmm_assistant_history.conversation_id：apps/web/src/features/assistant/api.ts:26-48,512-552。
- panel 的 ConversationEntry/Retry 只保存本地 message 和 history，未保存 conversation ID：apps/web/src/features/assistant/assistant-panel.tsx:822-948。
- Go 在 input.ConversationID == 0 时把本轮重置成单条 user message，并创建新 conversation；只有传入既有 ID 才从服务端历史重建；成功响应随后才把 ID 放进 lmm_assistant_history：apps/api-go/controller/assistant.go:59-70,272-287,343-375。
- 前端已有测试验证了本地上下文和五次重试，但没有验证“同一逻辑 turn 始终使用同一 conversation ID”：apps/web/src/features/assistant/api.test.ts:166-267。

**影响**

用户在同一个可视聊天里连续提问，数据库可能得到“一轮一个 conversation”；历史折叠列表不能代表一个完整线程。若上游已经成功、浏览器只是在收响应时断线，panel Retry 会再次以 ID 0 提交，可能新增一轮和一条历史，而不是幂等重取。

**建议**

在 panel 持有当前 conversation ID，首轮从响应 metadata 设置，后续请求显式发送；清空会话时只清理本地 ID 并开始新线程。为网络重试引入稳定的 logical turn/idempotency key，attempt header 只能标识尝试次数，不能代替幂等键；服务端应能安全重放同一 turn。补充“响应丢失、点击 Retry、连续三轮、刷新历史”的前后端集成测试。

**不应破坏**

保留现有本地输入上下文长度限制、服务端以 conversation history 为权威的重建逻辑、消息级 Retry，以及 Clear conversation 不删除历史记录的行为。provider 内部已有的“只重试模型调用、不要重复执行 tool write”也应保持。

### P1-02：跨用户历史读取权限比“管理员查看低等级聊天”更宽，L2 规则也未被明确表达

**证据**

- conversation history list/detail 走普通 UserAuth，不是 AdminAuth：apps/api-go/router/relay-router.go:69-83。
- model 的授权规则是“viewer rank 严格高于 owner rank”，不是“viewer 必须是 administrator”：apps/api-go/model/assistant_history.go:230-287。
- 现有测试明确证明 L1、L2、admin 都可以读取 L0 transcript；L1 不能读取 L2：apps/api-go/model/assistant_history_test.go:61-83。
- 前端只按 is_admin、developer_access_granted 和 access level 文案分支，没有一份 L0/L1/L2/admin 的显式能力矩阵：apps/web/src/features/assistant/assistant-panel.tsx:685-742、apps/api-go/controller/assistant.go:534-583。

**结论**

如果产品要求是“只有管理员可以查看低等级聊天”，这是 P1 权限过宽；如果产品有意允许严格高等级用户查看，则现有 privacy notice 的 “Authorized higher-access users” 与实现一致，但需求、UI 文案和审计口径必须明确写成“高等级用户”，不能只写“管理员”。目前 L2 至少会影响 history rank，却不等同于已获 L1 developer access，容易造成前后端认知不一致。

**建议**

先固定并测试一张不可歧义的矩阵：L0、L1、L2、Admin、Root 分别能看什么、能调用什么、能写什么。若 cross-owner 真的是 admin-only policy，把跨用户 list/detail 改为 admin-only；若保留 higher-rank policy，给每种等级显示明确原因、最小字段和审计记录，并在服务端集中执行 deny-by-default，而不是依赖 UI 隐藏。

**不应破坏**

保留本人历史访问、L0 不能向上读取、读取权限不自动授予 archive/restore、非 owner 不能 reveal card，以及 forbidden/not-found 统一为 404 的 existence-oracle 防护。

### P1-03：后端有 user_id 读取能力，但 assistant UI 没有管理员选择低等级用户的入口

**证据**

- Go list controller 接受 user_id，并把最终授权交给 model；query parameter 本身不是 authority：apps/api-go/controller/assistant_history.go:33-71。
- web API 只暴露 getAssistantConversationHistory(archived)，参数只有 archived；history component 也只用 active/archived 调用，没有 user selector 或 admin-only user ID：apps/web/src/features/assistant/api.ts:701-716、apps/web/src/features/assistant/assistant-history.tsx:98-112。
- 现有 panel 测试 mock 了 owner: lower_level_user 的返回内容，但没有验证管理员从 UI 选择 owner 并发出 user_id：apps/web/src/features/assistant/assistant-panel.test.tsx:1140-1269。

**建议**

提供管理员专用、可分页且默认不暴露过多个人资料的低等级用户/待审队列入口；选择后由 API 带入 owner ID，服务端再次按最终权限判断，并记录 viewer、owner、时间、理由和结果。不要把任意可编辑 user ID 当作授权。

**不应破坏**

保留普通用户只看到自己的 active/archived history；保留 lower-level transcript 的只读属性；不要因新增管理员选择器而开放 archive、restore 或 card reveal。

### P1-04：低等级聊天的“关键内容遮蔽”只对已识别模式和 secure card 有保证

**证据**

- 服务端在入模/入库前脱敏，覆盖 API key、JWT、邮箱、Cookie、Bearer、secret 等常见模式：apps/api-go/model/assistant_history.go:46-50,138-147。
- lower-level viewer 仍会收到该 transcript 的自然语言 message；只有 card metadata 对非 owner 固定为 protected，controller 注释也明确“elevated users may inspect transcript, but never reveal card value”：apps/api-go/model/assistant_history.go:479-509、apps/api-go/controller/assistant_history.go:145-148。
- UI 自己也承认 pattern matching is not a guarantee，且 L1 action 的 user statement/recommendation 在 draft card 中直接渲染，没有套用同一 display redaction：apps/web/src/features/assistant/assistant-panel.tsx:962-1008、apps/web/src/features/assistant/assistant-activation-tool.tsx:268-280。

**结论**

secure card 的遮蔽满足“关键凭证不让管理员/高等级 viewer 看到”；但不能把“已存储文本没有命中正则”解释成“所有关键内容都已遮蔽”。管理员/高等级用户仍可看到经过正则过滤后的自然语言上下文，且 action draft 是一条不一致的 UI 显示路径。

**建议**

将敏感内容策略放在服务端并采用更保守的 field-level allowlist/分类结果：低等级 transcript 默认摘要化或遮蔽敏感段，card 只显示受控 metadata；任何 action draft 在 UI 展示前也走同一 display sanitizer。对管理员读取增加 reason、审计、最小字段和短时授权；将正则命中率和漏检案例作为测试样本，而不是只测几个固定 pattern。

**不应破坏**

保留现有“发送前阻止敏感输入”、服务端二次脱敏、card ciphertext 不入历史、非 owner 不返回 card ID/摘要、owner 一次性 reveal，以及明确告知对话不是私密通信的行为。

### P1-05：聊天强制浏览器 session，但历史和 self handoff 读取没有同一边界

**证据**

- chat prepare 明确拒绝 use_access_token：apps/api-go/controller/assistant.go:294-303。
- pricing、plan、key creation、handoff submit 和 card reveal 通过 requireAssistantBrowserSession；但 history list/detail/archive/unarchive 和 GET /api/assistant/handoffs/self 只经过 UserAuth，没有同一检查：apps/api-go/router/relay-router.go:77-83、apps/api-go/controller/assistant_history.go:33-96、apps/api-go/controller/assistant_tools.go:286-320。
- UserAuth 本身是通用 dashboard auth，既可承接内部 session 也可承接 PAT：apps/api-go/middleware/auth.go:101-110。

**结论**

若 assistant transcript/support 数据的隐私策略是“浏览器 UI-only”，当前边界不一致：PAT 可以读取/管理自己的历史和读取 handoff，而不能聊天。若 PAT 访问是有意的 API 合约，则必须在 privacy policy、OpenAPI 和 UI 中明确告知其持有者可访问的内容。

**建议**

优先让 history、archive/unarchive 和 self handoff read/write 统一经过 browser-session gate；或者正式定义 PAT 的数据访问范围、最小返回字段和撤销/审计规则。不要只在前端隐藏入口。

**不应破坏**

保留 chat、key、pricing、plan、handoff submit、card reveal 对 browser session 的现有保护；不要让 PAT 访问任何其他用户 history 或任何 secure-card plaintext。

### P1-06：AgentLoopEnabled=false 时，未评估的 L0 首轮可能被强制失败

**证据**

- 未获 L1、非管理员、access level L0 且未评估的用户会触发强制 interlocutor assessment：apps/api-go/controller/assistant_agent.go:373-378。
- agent loop 关闭时仍将 maxSteps 提高到至少 2，并在首步提供 assessment tool；但 tool call 后的统一条件对 !settings.AgentLoopEnabled 直接返回 ASSISTANT_AGENT_MAX_STEPS：apps/api-go/controller/assistant_agent.go:601-631,649-659。

**影响**

在 assistant enabled 但 agent loop disabled 的配置下，L1/L2/管理员或已评估用户可能还能走单轮答案；未评估 L0 首轮会因权限评估流程需要 tool 而不可用。这是 L0 可用性与权限引导的配置相关 P1。

**建议**

让“agent loop disabled”成为一致的能力状态：要么跳过 tool assessment 并采用无 tool 的受限 L0 answer，要么把 assessment 做成 loop 外的独立、可完成步骤；不要让 UI 看起来可聊天而服务端只返回 step-limit error。保留 loop 开启时的 L0 先评估策略。

**不应破坏**

保留 L0 不可调用 L1+工具、L1 不可调用 admin 工具、管理员 action 必须确认的服务端二次校验。

### P1-07：L1 durable request 的失败被记录后继续，浏览器关闭仍可能造成申请丢失

**证据**

- 普通 L1 关键词会在模型调用前尝试入队，但 DB 错误只写日志并继续聊天：apps/api-go/controller/assistant.go:382-390。
- 显式 prepare_l1_recommendation 会先提交 request 再创建短时 auth flow，这是正确方向；但 SubmitAssistantDeveloperAccessRequest 的错误也只写日志，随后继续创建确认 token：apps/api-go/controller/assistant_agent.go:1101-1122。
- model 层在 transaction 中锁 user、复用 pending、保留原 statement 并可后续补充 AI recommendation，具备幂等合并基础：apps/api-go/model/developer_access_request.go:150-224。
- UI 有无 AI 的手动申请、pending 状态、Refresh 和提交失败 toast；但 request query 的初始失败没有明确的持久化失败状态：apps/web/src/features/assistant/assistant-activation-tool.tsx:75-117,180-228,292-319。

**结论**

正常路径下“先入队再确认”能保护浏览器关闭/用户放弃确认；数据库故障路径并不满足“不丢失”，因为服务端会返回一个看似可继续的 flow，却没有确认 durable row 已成功写入。

**建议**

只有 durable commit 成功才返回 queued/pending；失败时返回稳定错误码，让 UI 保留 statement/recommendation 草稿并提供 Retry。为 assistant signal 和用户确认使用稳定 request key，服务端用唯一 pending 约束/transaction/outbox 保证重复提交合并；将“入队失败”和“模型失败”分别展示。

**不应破坏**

保留 AI recommendation 只是可选 review aid、无 AI 手动提交、同一用户 pending 合并、先写 durable request 后创建短时确认 flow，以及管理员最终审批才授予 L1 的行为。

### P1-08：历史错误页没有真正的 Retry 控件

**证据**

- AssistantHistory 与详情 query 都设置 retry: false；列表错误页只输出 “Unable to load conversation history. Try again.”，详情错误页也没有按钮：apps/web/src/features/assistant/assistant-history.tsx:106-175,291-325。
- 聊天本身有 5 次 retryable error 处理和消息级 Retry button：apps/web/src/features/assistant/api.ts:47-63,512-552、apps/web/src/features/assistant/assistant-panel.tsx:1110-1127。

**建议**

在列表/详情错误页提供可聚焦的 Retry/Reload，调用 query refetch，保留当前 active/archived filter、已选 conversation 和返回位置；对 403/404 继续使用不可探测的统一文案，不因 retry 暴露资源存在性。

**不应破坏**

保留 active/archived 的 aria-pressed 筛选、owner-only archive/restore、错误时不清除正在编辑的申请草稿，以及聊天已有的 bounded retry policy。

### P1-09：核心 textarea 没有明确 accessible name

**证据**

- panel 将 aria-label="Ask AI assistant" 传给 PromptInput：apps/web/src/features/assistant/assistant-panel.tsx:436-450。
- PromptInput 将这些 props 传给 form，不是 textarea；PromptInputTextarea 只把 props 传给底层 textarea，当前没有 label 或 textarea 自身的 aria-label：apps/web/src/components/ai-elements/prompt-input.tsx:797-803,902-913。
- Enter/Shift+Enter/IME 行为本身已有实现：apps/web/src/components/ai-elements/prompt-input.tsx:839-849；因此问题是字段命名，不是输入模型整体不可用。

**建议**

为 textarea 提供稳定的可见 label 或 aria-label，并让 privacy notice、L0 hint、validation message 通过明确的 aria-describedby 关联；补充 axe/屏幕阅读器 smoke test，验证 mobile、rail、fullscreen 三种模式。

**不应破坏**

保留 placeholder、4,000 字符限制、required/invalid 反馈、Enter 发送、Shift+Enter 换行、IME composition 保护和发送期间禁用。

## P2

### P2-01：全屏 rail 使用自制 dialog，缺少 Sheet/Dialog 的焦点生命周期

**证据**

fullscreen 分支只有 role=dialog aria-modal=true aria-label=Service guide，没有看到 focus trap、打开时聚焦、关闭时 focus restore、Escape 处理或与标题的 labelledby 关联：apps/web/src/features/assistant/assistant-panel.tsx:1303-1315。移动端则使用 Sheet primitive：apps/web/src/features/assistant/assistant-panel.tsx:1357-1368。

**建议**

复用现有 Dialog/Sheet primitive，或明确补齐 modal focus lifecycle；为 fullscreen 绑定可见 heading，保证键盘用户能回到触发按钮。

**不应破坏**

保留 rail collapse/expand、fullscreen 不显示 collapse button、mobile full dynamic viewport、safe-area padding 和 panel 内部滚动位置。

### P2-02：privacy disclosure 的折叠语义与可访问树不完全一致

**证据**

privacy notice 外层是 role=alert；折叠时 description 只变成 sr-only，仍可能留在可访问树中；toggle 同时使用 aria-controls 和 aria-describedby，textarea 还把外层 alert ID 放进 describedby：apps/web/src/features/assistant/assistant-panel.tsx:962-1008,1275-1281。

**建议**

使用标准 disclosure 结构：标题按钮控制 description 的 hidden/可访问状态，避免每次 focus 都重复朗读长隐私文案；保留首次展示和 footer 的短警示，且折叠状态仍能一键展开。

**不应破坏**

保留 privacy notice 初次展开、5 秒后折叠、可重新打开，以及“不发送个人信息、密码、API key、Cookie、credential”的明确文案。

### P2-03：消息 log 只声明 role=log，最终回答和错误不一定被读屏器宣布

**证据**

Conversation 设置了 role=log，但没有统一的 aria-live/aria-relevant；只有 thinking 状态的 MessageContent 有 aria-live=polite，最终 assistant answer 和 error entry 没有独立 announcement：apps/web/src/components/ai-elements/conversation.tsx:31-38、apps/web/src/features/assistant/assistant-panel.tsx:1080-1149。

**建议**

增加简短、去重的 status live region，并为每条消息提供稳定的“你/Service guide/error”语义；不要让完整 Markdown 在每次更新时被整段重复朗读。

**不应破坏**

保留可视 transcript、自动滚动/手动滚动按钮、Markdown 安全渲染、错误 Retry button 和 assistant thinking 状态。

### P2-04：Anthropic 风格是全局主题能力，assistant 组件自身仍偏通用

**证据**

全局样式导入 Lora，Anthropic preset 定义 warm cream、warm-neutral card、clay/coral accent 和 editorial serif，并且默认 preset 是 anthropic：apps/web/src/styles/index.css:19-33、apps/web/src/styles/theme-presets.css:474-560、apps/web/src/lib/theme-customization.ts:118-127。assistant panel/history 主要使用 bg-background、通用 Card/Alert/Message、secondary surface 和 rounded utility：apps/web/src/features/assistant/assistant-panel.tsx:950-1008、apps/web/src/features/assistant/assistant-history.tsx:222-283。

**建议**

为 assistant 定义少量语义 token 和稳定的 editorial hierarchy：暖中性对话 surface、克制 clay CTA、serif 标题、清晰的 user/assistant 节奏；跟随用户当前 theme preset，不在 feature 内硬编码单一颜色或品牌标志。

**不应破坏**

保留用户主题切换、暗色对比度、现有 responsive rail/mobile 布局、触控尺寸、代码块横向滚动和 reduced-motion 兼容。

### P2-05：L1 action draft 的展示脱敏与普通消息路径不一致

**证据**

普通 reply content 会经过 redactAssistantMessageForDisplay，但 reply.action 解析后直接放入 recommendation draft，draft 直接渲染 user_statement 和 recommendation：apps/web/src/features/assistant/assistant-panel.tsx:828-874、apps/web/src/features/assistant/assistant-activation-tool.tsx:268-280。最终 request model 层仍会对 reason/recommendation 做规范化/脱敏，这是后端持久化的正向保护：apps/api-go/model/developer_access_request.go:161-218。

**建议**

让 action draft 复用同一 display sanitizer，并对 preview 长度、URL、secret-like token 做 server-side validation；把“显示脱敏”和“最终存储脱敏”分别测试。

**不应破坏**

保留用户在提交前审阅原始意图的能力、显式确认、AI recommendation 不决定权限，以及最终 request 由管理员审核。

## 推荐修复顺序

1. 先修 P1-01：会话 ID、逻辑 turn 幂等和历史连续性；否则后续的历史管理员视图和 Retry 结果都不可信。
2. 同时定稿 P1-02/P1-03/P1-04：明确“管理员-only”还是“严格高等级”，再实现最小字段的管理员入口和服务端遮蔽策略。
3. 修复 P1-07/P1-08/P1-09：durable L1 失败要可见且可重试，历史错误页要可重试，核心输入要有 accessible name。
4. 完成 P2 无障碍/视觉细化，并补充真实浏览器键盘、NVDA/VoiceOver、暗色主题、移动 viewport 和权限矩阵集成测试。
