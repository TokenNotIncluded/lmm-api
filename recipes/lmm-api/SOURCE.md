## 来源与证据边界

观察日期：2026-10-28。原实现仓库：https://github.com/TokenNotIncluded/lmm-api ，行为观察提交 `d468affecfd76a5123358732fb9a4e0cc1f63566`，迁出基准 `ff01f8456b978089bce08175cbd357c3d22c3983`；CLI 固定为 https://github.com/TokenNotIncluded/lmm 的 `23de1322d68a021707eda876ed477e71d72f9544`。实现已迁到 https://github.com/TokenNotIncluded/lmm-api-source ，初次迁出提交 `06d8b4b7c9630106c6ef47425f9e67d333bd1548`；当前实现版本以主仓库 gitlink 和 STATUS.md 为准，由当前主仓库 `generated/lmm-api-source/` 子模块接入。它不是配方依赖，不进入归档。

初次提取配方时只做源码、文档及已有测试定义的静态观察，未重跑应用、数据库、浏览器、支付或性能验收；后续修复和验证按版本单独记录。源码存在、测试定义、历史通过说明均不代表固定版本已通过验收。原开发工作区未提交内容全部排除；包括 Passkey、模型预检、共享读配额、价格修改和旧单体回迁准备脚本，不能将其当成已发布功能。

旧单体 `传统` 与 `traditional` 固定于 `3948a0a1ee1cefdd016a7aa45a11851545bb4b85`；当前微服务默认分支为 `main`。固定源码中仍有旧分支名的文档和工作流引用，应按提交识别观察对象，不能按旧文案取错源码。

项目基于 QuantumNous/new-api，保留上游及其他贡献者归属、AGPL-3.0-or-later、NOTICE 和第三方许可证。目标不要求复制实现；若复用代码、素材、字体或其他受保护材料，逐项保留许可证和归属。商业许可状态未知，不据此扩大授权。

## 0.1.1：2026-10-29 Issue 与 PR 维护

本轮从源码 `86e51b0708086fb78605ef9f519275149ac47873` 核对当前问题，补齐定价排序、上游流截断的公开错误分类和完整质量计划的 Mocklab 入口。依据为本项目 [#911](https://github.com/TokenNotIncluded/lmm-api/issues/911)、[#945](https://github.com/TokenNotIncluded/lmm-api/issues/945)、[#786](https://github.com/TokenNotIncluded/lmm-api/issues/786)；已有动态分类和 SSE 分帧分别按 [#961](https://github.com/TokenNotIncluded/lmm-api/issues/961)、[#962](https://github.com/TokenNotIncluded/lmm-api/issues/962) 重新核对。实际自测、原始失败和未运行范围由 STATUS.md 及关联源码 PR 记录。

ISSUE_MAINTENANCE.md 完整承接旧 #879/#875/#942 的剩余需求和验收。旧 PR 关闭为迁移取代不代表完整财务导出、支付供应商或 schema 升级完成；原候选与已通过的历史场景均保留。
