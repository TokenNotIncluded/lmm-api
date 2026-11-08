# LMM API · 配方项目

本仓库维护 `recipes/lmm-api/` 及其递归组件的需求、接口与验收合同，默认分支为 `main`。实现由 [lmm-api-source](https://github.com/TokenNotIncluded/lmm-api-source) 的固定 gitlink 接入 `generated/lmm-api-source/`。

## 工作位置

- 需求和合同只放 UTF-8 Markdown，读取对应 `RECIPE.md`、`BUILD_PROMPT.md`、`ACCEPTANCE.md` 与声明依赖；不放源码、二进制、软链接、执行钩子或秘密。
- 产品范围读 [PRODUCT_REQUIREMENTS.md](recipes/lmm-api/PRODUCT_REQUIREMENTS.md)，完成证据读 [PRODUCTION_ACCEPTANCE.md](recipes/lmm-api/PRODUCTION_ACCEPTANCE.md)。动态进度只维护源码仓库 [docs/progress.md](https://github.com/TokenNotIncluded/lmm-api-source/blob/main/docs/progress.md)，本仓库不复制状态列表或逐轮通过数。
- 实现任务进入 `generated/lmm-api-source/`，先读其 `AGENTS.md` 和适用技能，再执行源码内命令。Core 身份、权限、资金和数据库边界不得因文档整理改变。
- 源码自测并正常提交/推送后，再显式更新父 gitlink；用 `git ls-tree HEAD generated/lmm-api-source` 读取固定版本。CLI/providers 保持嵌套子模块归属，不提交不可达指针。
- 保留并行改动；不 reset/clean/强推覆盖他人工作。`generated/` 只作源码子模块容器，不恢复平铺旧单体。

## 检查与证据

配方文档修改运行 `recipkg check recipes/lmm-api`。依赖或归档结构改变时，再定向检查临时归档和生成提示词；记录工具缺失或失败，不把结构检查当应用构建。纯文档整理不启动数据库、浏览器、全量应用构建或生产流程。

共享技能由 `.agents/skills/` 的 gitlink 固定；自定义技能的实现路径在源码仓库内读取。运行结果区分开发自测、独立业务验收、提交推送、发布与部署，保留失败、UNKNOWN 和未运行范围。生产读取按用户授权范围；写入、重启、部署、生产账号或真实付费调用须有相应明确授权，资金动作保留预览确认。

保留 AGPL-3.0-or-later、QuantumNous/new-api、贡献者和第三方归属。旧单体使用固定 [go-v0.2.118](https://github.com/TokenNotIncluded/lmm-api/releases/tag/go-v0.2.118) 或具体提交追溯，不假定历史分支仍存在，不改写历史。
