## 来源与证据边界

观察日期：2026-10-28。原实现仓库：https://github.com/TokenNotIncluded/lmm-api ，行为观察提交 `d468affecfd76a5123358732fb9a4e0cc1f63566`，迁出基准 `ff01f8456b978089bce08175cbd357c3d22c3983`；CLI 固定为 https://github.com/TokenNotIncluded/lmm 的 `23de1322d68a021707eda876ed477e71d72f9544`。实现已迁到 https://github.com/TokenNotIncluded/lmm-api-source ，初次迁出提交 `06d8b4b7c9630106c6ef47425f9e67d333bd1548`；当前实现版本以主仓库 gitlink 和 STATUS.md 为准，由当前主仓库 `generated/lmm-api-source/` 子模块接入。它不是配方依赖，不进入归档。

初次提取配方时只做源码、文档及已有测试定义的静态观察，未重跑应用、数据库、浏览器、支付或性能验收；后续修复和验证按版本单独记录。源码存在、测试定义、历史通过说明均不代表固定版本已通过验收。原开发工作区未提交内容全部排除；包括 Passkey、模型预检、共享读配额、价格修改和旧单体回迁准备脚本，不能将其当成已发布功能。

旧单体 `传统` 与 `traditional` 固定于 `3948a0a1ee1cefdd016a7aa45a11851545bb4b85`；当前微服务默认分支为 `main`。固定源码中仍有旧分支名的文档和工作流引用，应按提交识别观察对象，不能按旧文案取错源码。

项目基于 QuantumNous/new-api，保留上游及其他贡献者归属、AGPL-3.0-or-later、NOTICE 和第三方许可证。目标不要求复制实现；若复用代码、素材、字体或其他受保护材料，逐项保留许可证和归属。商业许可状态未知，不据此扩大授权。

## 0.1.1：2026-10-29 流失败分类

依据 [#945](https://github.com/TokenNotIncluded/lmm-api/issues/945) 与 [#962](https://github.com/TokenNotIncluded/lmm-api/issues/962)，保留当前增量 SSE 分帧，补充 `upstream_truncated` 的公开语义与精确失败回归。观察基线为源码 `86e51b0708086fb78605ef9f519275149ac47873`；未改变付款方、预留或结算策略。真实 TCP 测试、内存账务观察与 PostgreSQL、真实供应商验收分别登记，详见主项目 STATUS.md 与源码 PR。

## 0.1.2：2026-10-29 邮件正式接线

行为实现来源为 [lmm-api-source `5733972`](https://github.com/TokenNotIncluded/lmm-api-source/commit/57339728292d522ca7a4f926f1624301b5cc9350)：正式 identity v6 新装与运行权限、加密出箱、配置及 Core/Web 邮件调用。固定输入的实际非 test Core、本地受限 PostgreSQL 和真实 SMTP 传输已完成独立注册/重置/撤销/重放/启动拒绝/退出等待验收；Web 定向测试与类型检查分别记录。它不证明真实浏览器、外部邮箱、存量升级、完整资源门槛或生产切换。完整范围、原始失败与未运行检查继续由主项目 STATUS.md 保留，不能将邮件切片推定为整站完成。
