## 来源与证据边界

观察日期：2026-10-28。原实现仓库：https://github.com/TokenNotIncluded/lmm-api ，行为观察提交 `d468affecfd76a5123358732fb9a4e0cc1f63566`，迁出基准 `ff01f8456b978089bce08175cbd357c3d22c3983`；CLI 固定为 https://github.com/TokenNotIncluded/lmm 的 `23de1322d68a021707eda876ed477e71d72f9544`。实现已迁到 https://github.com/TokenNotIncluded/lmm-api-source ，初次迁出提交 `06d8b4b7c9630106c6ef47425f9e67d333bd1548`；当前实现版本以主仓库 gitlink 和 STATUS.md 为准，由当前主仓库 `generated/lmm-api-source/` 子模块接入。它不是配方依赖，不进入归档。

初次提取配方时只做源码、文档及已有测试定义的静态观察，未重跑应用、数据库、浏览器、支付或性能验收；后续修复和验证按版本单独记录。源码存在、测试定义、历史通过说明均不代表固定版本已通过验收。原开发工作区未提交内容全部排除；包括 Passkey、模型预检、共享读配额、价格修改和旧单体回迁准备脚本，不能将其当成已发布功能。

旧单体 `传统` 与 `traditional` 固定于 `3948a0a1ee1cefdd016a7aa45a11851545bb4b85`；当前微服务默认分支为 `main`。固定源码中仍有旧分支名的文档和工作流引用，应按提交识别观察对象，不能按旧文案取错源码。

项目基于 QuantumNous/new-api，保留上游及其他贡献者归属、AGPL-3.0-or-later、NOTICE 和第三方许可证。目标不要求复制实现；若复用代码、素材、字体或其他受保护材料，逐项保留许可证和归属。商业许可状态未知，不据此扩大授权。

## 0.1.1：2026-10-29 定价问题回写

依据本项目 [#911](https://github.com/TokenNotIncluded/lmm-api/issues/911) 的价格排序需求、[#961](https://github.com/TokenNotIncluded/lmm-api/issues/961) 的动态计费分类要求与 [#920 最新定位](https://github.com/TokenNotIncluded/lmm-api/issues/920#issuecomment-6456789544) 更新行为及验收。本轮在当前源码上增加六种价格排序和对应回归；分类规则已存在，不重复实现。保留未知动态价格、真实浏览器与旧单体生产目录的验收边界，具体执行结果见主仓库 STATUS.md。

## 0.1.2：2026-10-29 邮件正式接线

行为实现来源为 [lmm-api-source `5733972`](https://github.com/TokenNotIncluded/lmm-api-source/commit/57339728292d522ca7a4f926f1624301b5cc9350)：正式 identity v6 新装与运行权限、加密出箱、配置及 Core/Web 邮件调用。固定输入的实际非 test Core、本地受限 PostgreSQL 和真实 SMTP 传输已完成独立注册/重置/撤销/重放/启动拒绝/退出等待验收；Web 定向测试与类型检查分别记录。它不证明真实浏览器、外部邮箱、存量升级、完整资源门槛或生产切换。完整范围、原始失败与未运行检查继续由主项目 STATUS.md 保留，不能将邮件切片推定为整站完成。

## 0.1.3：2026-10-30 Notice 保存失败处理

[lmm-api-source `90c442d`](https://github.com/TokenNotIncluded/lmm-api-source/commit/90c442d8cb6d8811d0cdeaa6e806a324770f67f4) 修复 Notice 表单使用异步 mutation 时未消费拒绝、导致正确错误提示之外仍抛页面异常的问题。实际新 bundle 复验登录、Notice 保存刷新、真实存储失败及 COMMIT UNKNOWN；公告/只读/窄屏/普通用户四组按输入未变边界复用原独立证据，合计限定范围 8 组。新输入 pageerror=0，两故障 3.5 秒均仅一次写；UNKNOWN 随后读回和刷新确认真实提交。固定 Bun1.3.14 执行一次构建、完整类型检查及变更文件 lint/format，通过；复用依赖最初由同锁文件的 canary 安装，未宣称工具链完全相同。其余设置、旧库/新 Core 组合、全站和生产验收不由此通过。

## 0.1.4：2026-10-30 原生性能显示

实现来源 `57c71549a5596eeb5f9c105df73634841f888977`，统计优先aggregate并使用实际分子/分母，未知保持null、部分覆盖及缺样本可见，七语言各新增同4键。主线程固定Bun1.3.14/Node26.10.0、2670项输入逐一匹配提交，34项定向与25项overview、完整类型、变更文件lint、隔离build和初始包预算通过；初始gzip550112/786432 bytes。旧资源夹具缺字段的失败和作者canary/事后补hash记录均保留，不把它们伪称固定工具链证据。新bundle尚未完成真实浏览器验收；完整登录/历史数据、全站和生产切换未由此通过。

独立捕获Native响应→Web复验随后发现，模型页PerformanceOverview空结果提前返回时漏掉部分覆盖提示（wi1030a2：7过/1失败）。`24066ba4c2204a9bf6173c97f4ff2d8196512401` 仅补该分支notice，保留无数据文案；完整类型/组件lint/overview25项已复验，原失败与断言保留。最终构建和独立复验固定输入以STATUS.md为准。

最终 `24066ba4` 的新冻结2670项Web输入逐blob匹配，完整类型/组件lint/overview25、一次新build与包体预算通过（初始gzip550115/786432 bytes，533份输出）。独立wi1030a3保持原断言，8/8组通过；以归档实际Native JSON/SQL校验真实API适配与React DOM，HTTP传输/canvas为替身，未启动新后端，也不是浏览器/登录验收。原wi1030a1夹具错误、wi1030a2产品失败及冻结输入保留，不能只报告最后通过。最终制品SHA清单和清理情况见STATUS.md。

## 0.1.6：2026-10-30 HTML-in-Canvas 与全站视觉更新

实现来源为 [lmm-api-source `6523c4b`](https://github.com/TokenNotIncluded/lmm-api-source/commit/6523c4b4b831643d69de3874c2e35933c91bfc2b)，测试源码树为 `f55d28d7e32314d427aa682c5fcbb722246e26a1`。首页新增按浏览器能力启用的 HTML-in-Canvas 连接场景与普通 DOM 回退，同时更新全局主题、工作台、导航、认证、定价、指南和共享控件。炭黑、暖白与克制金色用于布局层级和交互提示；现有身份、权限、资金、API 和确认行为保持原合同。

开发自测共 70 个不同定向用例通过：共享控件与主题 17 项、公开页/认证/定价 47 项、控制台 3 项、Canvas 3 项。测试夹具类型修正后完整类型检查通过，最终文案与窄屏样式修正后的构建及包体预算通过，文档检查通过。定向 lint 遵循仓库排除配置，`src/components/ui` 未由该 lint 覆盖。

云端 Chrome 实际检查了桌面首页及连接场景的明暗主题、浅色登录页，并在 390/320 CSS 像素 iframe 中检查窄屏首页、连接场景、暗色登录和浅色指南。场景实际标记为 `data-renderer=dom`，键盘焦点可见、暂停可切换；首次窄屏节点 05 被裁切后已修复并截图复核，图片节点复用七语言 `Drawing` 文案，受影响的 3 项测试重跑通过且不重复计数。截图保留在本轮对话，详细观察范围见 [STATUS.md](../../STATUS.md)。本地静态预览没有后端，定价页跳回首页；认证后的控制台与数据、浏览器原生 HTML-in-Canvas 绘制及独立业务验收未执行，本轮没有部署或真实付费调用。

## 0.1.7：2026-10-30 原生钱包报价确认与 LinuxDO 表单支付

原生财务投影可用时显示实际启用渠道，使用十进制字符串和整数金额，先报价再明确确认；固定 Creem 产品金额只读，LDC／法币和测试／正式环境明确区分。保存原请求键以恢复未知结果，写入不自动重放，状态查询有界。`checkout_method` 明确控制用户点击后的 GET 链接或 POST 表单；LinuxDO 表单保留原字段与签名，不由 URL 推测方法。七语言文案已同步。七项定向 Web 测试、相关 lint 与完整类型检查通过；支付 POST 补丁前的 Web build 已通过，补丁后未重跑 bundler。未进行真实商户或浏览器支付验收。

实现来源为 [lmm-api-source `7abe06d`](https://github.com/TokenNotIncluded/lmm-api-source/commit/7abe06d5d369b2ccefe4364099450bfae4534953)，详见 [STATUS.md](../../STATUS.md)；对应源码的 `docs/payment-gateways.md` 与支付模块 `VERIFICATION.md` 记录实际检查、保留失败和未运行范围。开发自测不等于独立验收或生产切换；本轮未部署、未访问真实支付账户或产生真实费用。
