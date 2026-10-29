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
