# LMM API · Recipkg project

本仓库的主要输入是 `recipes/lmm-api/` 及其递归依赖。实现代码在独立仓库 [TokenNotIncluded/lmm-api-source](https://github.com/TokenNotIncluded/lmm-api-source)，通过 `generated/lmm-api-source/` 的固定提交子模块接入。保持本项目及原 GitHub 仓库名 `lmm-api`，默认分支 `main`。

## 工作位置

- 改需求、公开接口和验收合同：编辑 `recipes/`，只放 UTF-8 Markdown，不放实现源码、二进制、软链接、执行钩子或秘密。
- 改实现：先进入 `generated/lmm-api-source/`，读取其 `AGENTS.md`，遵守原核心、权限、资金和数据库边界。构建及测试命令均在源码子模块内执行。
- 发布实现：在源码仓库本地自测并提交/推送，再显式更新本仓库 gitlink。不能将未提交工作误算为固定版本，禁止 reset/clean/强推覆盖并行改动。
- `generated/` 只作容器，不放平铺源码。CLI 与 providers 保持源码仓库内原子模块，不在主仓库重复装配。

## 技能与检查

共享 Recipkg 技能保留在 `.agents/skills/` 的 11 个固定上游子模块，目录和仓库身份不变。已有自定义技能保留原文；其原源码路径与文档引用在 `generated/lmm-api-source/.agents/skills/` 的完整对应副本中继续有效。实现任务从源码子模块读取对应自定义技能，不能把相对路径当主仓库代码位置。

配方修改运行 `recipkg check recipes/lmm-api`。递归依赖变更时临时 pack、check 归档并 build 完整提示词，随后删除临时归档。未明确要求不保留归档。共享技能更新先审阅其上游，再显式更新消费者提交。

自测、独立业务验收、推送、发布和部署分别报告。配方结构通过不代表重建成功。完整业务验收仍按配方和源码内完整验收范围执行；未运行、失败、UNKNOWN 和阻塞继续保留。保留金融预览确认，不注册生产测试账号，不访问生产数据或执行真实付费调用，不触碰或部署任何 DMIT 服务器。

保留 AGPL-3.0-or-later、QuantumNous/new-api、贡献者及第三方归属。旧单体历史在 `传统` / `traditional` 分支，历史提交不改写。
