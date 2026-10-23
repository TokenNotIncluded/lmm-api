# 📖 文档导航

[项目首页](../README.md) · [贡献指南](contributing.md)

欢迎来到 LMM Forge 文档中心！这里汇集了项目的架构设计、开发指南和运维参考，帮助你快速找到所需内容。

我们当前的架构以微服务分支为基础：可执行程序包括 `lmm` 客户端、`lmm-core` 核心和 `lmm-extensions` 扩展；Web 前端作为独立构建的静态文件单独产出。如果你在其他地方看到线上 Go 单体的历史说明，那是旧架构的参考资料，并非新架构的安装步骤。

日常开发请基于 `wip/rust-core-go-extensions` 分支；`main` 与 `traditional` 已冻结，不再接受新提交——详情请参阅[分支交接规则](development-branches.md)。如果你想了解传统 main 的增量是否已迁移到新分支，可以查看[固定源码对齐核查](main-wip-parity-audit.md)。另外，[Extore 商品同步](extore-shop-binding-followup.md)目前尚未完成验收，暂时请不要将其部署到生产环境。

## 主要入口

从这里找到你需要的内容：

| 主题 | 文档 |
| --- | --- |
| 架构、模块状态与剩余工作 | [架构与状态](core-migration.md) |
| 注册滥用、蜜罐、独立限频与封禁通知 | [行为与接线范围](abuse-controls.md) |
| 低频用户管理与批量预览 | [Python 管理客户端](admin-cli.md) |
| 架构收尾的并行分工与串行验收 | [10 个并行任务与 1 个串行收尾提示词](architecture-completion-prompts.md) |
| 新安装、服务隔离与日常启停 | [Docker 开发栈](../deployment/docker/README.md) |
| 环境、命令与测试 | [开发指南](development.md)、[本地质量门禁](quality.md) |
| TOML 配置、文件路径与升级保留规则 | [配置与文件路径](filesystem-layout.md) |
| 可编辑协议与发布规则 | [用户、隐私、退款模板](legal/README.md) |
| 独立分发包、校验与发布边界 | [打包与分发](release-architecture.md) |
| 历史资料的使用范围 | [历史文档](archive.md) |

## 详细设计

如果你想深入了解各模块的设计细节，以下文档会是很好的起点：

[核心身份](core-identity.md) · [跨进程接口](core-protocol.md) · [业务验收范围](microkernel-business-acceptance.md)

[Rust 路由与计费合同](../services/core/src/routing/README.md)包含 [OpenAI 12 小时缓存写入计费（revision 4）](../services/core/src/routing/README.md#openai-twelve-hour-cache-writes-revision-4)的用量分区、示例价格、恢复和回退边界。

[通知与邮件](../services/core/src/notifications/README.md)说明个人偏好、余额阈值、渠道发布事件、SMTP／Resend 配置，以及事务回滚和投递状态不明时的处理边界；源码候选仍需独立 Rust／PostgreSQL 验证。

[RT16 Markdown 隔离安全复现](../scripts/security/rt16/README.md)用于验证组件层面的安全性。不过请注意，它不能替代商品、客服、个人资料和助手的存储读写验收——那些需要单独确认。

[部署研究结论与待接入项](deployment-research.md)保留了已移除实验目录中的有效结论与历史来源，供参考学习使用，但请不要将其作为正式部署步骤。

Go 模块的接口与存储规则以各自源码目录为准：[扩展宿主](../services/extensions/README.md)、[助手](../services/extensions/internal/modules/assistant/README.md)。[CLI 子项目文档](https://github.com/TokenNotIncluded/lmm#readme)与服务端部署分开。

品牌与产品：[品牌设计](brand-design.md)、[产品记录](product.md)、[分支来源与归属](fork.md)。前端资料：[设计](../apps/web/DESIGN.md)、[Logo](../.github/assets/README.md)、[翻译术语](translation-glossary.md)。[安全报告](../.github/SECURITY.md)与[支持渠道](SUPPORT.md)不属于运行时用户协议。

旧部署、OpenAPI、审计和已合并专题可以从[历史索引](archive.md)查找原文。它们作为历史参考保留，但不代表当前服务能力或测试通过状态。

## 文档维护指南

为了让文档保持清晰和实用，我们遵循以下几条简单的原则：

- **聚焦当前**：入口文档只描述当前可执行的步骤，避免过时信息造成困惑。
- **就近原则**：实现细节留在拥有该接口的模块中，让信息和代码住在一起。
- **整洁归档**：删除实现后，同步删除相关的生成依赖，并将历史来源集中到 [archive.md](archive.md)，而不是新增逐文件的占位页。
- **始终保留**：许可证、安全记录和仍有使用方的接口约定不应删除。
