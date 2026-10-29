## 来源与证据边界

观察日期：2026-10-28。原实现仓库：https://github.com/TokenNotIncluded/lmm-api ，行为观察提交 `d468affecfd76a5123358732fb9a4e0cc1f63566`，迁出基准 `ff01f8456b978089bce08175cbd357c3d22c3983`；CLI 固定为 https://github.com/TokenNotIncluded/lmm 的 `23de1322d68a021707eda876ed477e71d72f9544`。实现已迁到 https://github.com/TokenNotIncluded/lmm-api-source ，初次迁出提交 `06d8b4b7c9630106c6ef47425f9e67d333bd1548`；当前实现版本以主仓库 gitlink 和 STATUS.md 为准，由当前主仓库 `generated/lmm-api-source/` 子模块接入。它不是配方依赖，不进入归档。

初次提取配方时只做源码、文档及已有测试定义的静态观察，未重跑应用、数据库、浏览器、支付或性能验收；后续修复和验证按版本单独记录。源码存在、测试定义、历史通过说明均不代表固定版本已通过验收。原开发工作区未提交内容全部排除；包括 Passkey、模型预检、共享读配额、价格修改和旧单体回迁准备脚本，不能将其当成已发布功能。

旧单体 `传统` 与 `traditional` 固定于 `3948a0a1ee1cefdd016a7aa45a11851545bb4b85`；当前微服务默认分支为 `main`。固定源码中仍有旧分支名的文档和工作流引用，应按提交识别观察对象，不能按旧文案取错源码。

项目基于 QuantumNous/new-api，保留上游及其他贡献者归属、AGPL-3.0-or-later、NOTICE 和第三方许可证。目标不要求复制实现；若复用代码、素材、字体或其他受保护材料，逐项保留许可证和归属。商业许可状态未知，不据此扩大授权。

## 0.1.1：2026-10-29 SSE 错误合同

从当前 Core 的 `RelayError`、流内错误编码和真实回环 HTTP 回归提炼 [#945](https://github.com/TokenNotIncluded/lmm-api/issues/945) 的失败分类。`upstream_truncated` 从过去共用的 `invalid_upstream_response` 中分出，传输错误与主动取消的现有类别保持。该变化是公开错误码的细分，HTTP 已发送状态、资金 UNKNOWN 以及 Protobuf wire 合同不变；消费者按新错误码识别并保留原操作恢复。

## 0.1.2：2026-10-30 性能观测与渠道读取

来源为 lmm-api-source 的 `0b54719`（真实 relay 观测和读取）与 `e106a74`（ChannelRead gate），没有新的配方依赖或产品进程。新增 nullable/加权/覆盖状态及个人 Session 当前可见性合同，保持金融权威不变。原生性能的实际 Core/PG/计数供应商开发与独立8组通过；历史性能、完整渠道控制面和最终整站验收尚未完成。结果以 STATUS.md 固定输入及限制为准。
