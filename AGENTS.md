# LMM API · 配方项目

本仓库维护 `recipes/lmm-api/` 及其递归组件的需求、接口与验收合同，默认分支为 `main`。实现由 [lmm-api-source](https://github.com/TokenNotIncluded/lmm-api-source) 的固定 gitlink 接入 `generated/lmm-api-source/`。

## 工作位置

- 需求和合同只放 UTF-8 Markdown，读取对应 `RECIPE.md`、`BUILD_PROMPT.md`、`ACCEPTANCE.md` 与声明依赖；不放源码、二进制、软链接、执行钩子或秘密。
- 产品范围读 [PRODUCT_REQUIREMENTS.md](recipes/lmm-api/PRODUCT_REQUIREMENTS.md)，完成证据读 [PRODUCTION_ACCEPTANCE.md](recipes/lmm-api/PRODUCTION_ACCEPTANCE.md)。动态进度只维护源码仓库 [docs/progress.md](https://github.com/TokenNotIncluded/lmm-api-source/blob/main/docs/progress.md)，本仓库不复制状态列表或逐轮通过数。
- 实现任务进入 `generated/lmm-api-source/`，先读其 `AGENTS.md` 和适用技能，再执行源码内命令。Core 身份、权限、资金和数据库边界不得因文档整理改变。
- 源码自测并正常提交/推送后，再显式更新父 gitlink；用 `git ls-tree HEAD generated/lmm-api-source` 读取固定版本。CLI/providers 保持嵌套子模块归属，不提交不可达指针。
- 所有已获授权的开发者在现有项目的同一个 `main` 上直接读写、提交和快进推送。协调者组织分工和验收。文件重叠时先约定修改顺序，保留他人改动，不新建分支、工作树或项目外的开发副本。
- 在项目内约定的共享目录按任务记录目标、认领文件、基准 SHA、状态和交付 SHA。每位开发者单独回复或原子更新自己的记录。跨机器时先验证文件可见性。运行态记录须被 Git 忽略，且与安全规则、凭据及 `.git` 分开。`generated/` 只作源码子模块容器，不恢复平铺旧单体。

## 在同一主分支协作

所有已获授权的开发者都可以在当前项目及其规范源码目录的 `main` 读写并及时提交、推送。保持一个工作目录和一个主分支，正常快进推送并遵守分支保护。遇到文件交叠，先在任务文件约定修改次序并保留他人变更。

日常协作使用项目内已忽略的 `out/rollout-20261126-coordinator/tasks/<任务>/`。每位参与者使用独立回复文件，记录目标、认领路径、基准 SHA、状态和交付 SHA。跨设备先验证共享文件可读写。运行态对话留在该目录，凭据和安全配置保留在原有边界内。

编译、测试、类型检查、浏览器和发布验证统一使用云虚拟机或 GitHub 托管运行器。开发机用于编辑和查看回执，保持用户桌面与直播如常运行。业务操作和视觉设计各自保留独立验收。

## 检查与证据

编译、测试、类型检查和浏览器验收在云虚拟机或 GitHub 托管 runner 执行。用户开发机用于源码协作，保留用户桌面、直播和服务现场。
配方文档修改运行 `recipkg check recipes/lmm-api`。依赖或归档结构改变时，再定向检查临时归档和生成提示词；记录工具缺失或失败，不把结构检查当应用构建。纯文档整理不启动数据库、浏览器、全量应用构建或生产流程。

共享技能由 `.agents/skills/` 的 gitlink 固定；自定义技能的实现路径在源码仓库内读取。运行结果区分开发自测、独立业务验收、提交推送、发布与部署，保留失败、UNKNOWN 和未运行范围。生产读取按用户授权范围；写入、重启、部署、生产账号或真实付费调用须有相应明确授权，资金动作保留预览确认。

保留 AGPL-3.0-or-later、QuantumNous/new-api、贡献者和第三方归属。旧单体使用固定 [go-v0.2.118](https://github.com/TokenNotIncluded/lmm-api/releases/tag/go-v0.2.118) 或具体提交追溯，不假定历史分支仍存在，不改写历史。
