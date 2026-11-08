# 仓库拆分记录

2026-10-28，`TokenNotIncluded/lmm-api` 调整为 Recipkg 配方项目，可编译实现迁入独立仓库 `TokenNotIncluded/lmm-api-source`。此页只说明仓库来源和职责；最新工作统一见 [源码进度列表](https://github.com/TokenNotIncluded/lmm-api-source/blob/main/docs/progress.md)。

| 内容 | 位置与来源 |
| --- | --- |
| 需求、公开接口、验收 | 本仓库 `recipes/lmm-api/` 及五个组件/合同包 |
| 实现 | 源码仓库；父项目 `generated/lmm-api-source/` gitlink 固定版本 |
| 原实现迁出基准 | `lmm-api@ff01f8456b978089bce08175cbd357c3d22c3983` |
| 初次迁入快照 | `lmm-api-source@06d8b4b7c9630106c6ef47425f9e67d333bd1548` |
| CLI/providers | 保持源码仓库内嵌套子模块及其独立 Git 历史 |
| 共享技能 | 本仓库固定技能子模块；自定义技能的源码路径在实现仓库继续使用 |

初次迁入以原已跟踪树为基础，只调整迁移入口说明，原提交历史及未提交工作按当时记录保留。构建命令在源码根读取，旧绝对缓存路径不构成当前可用环境。精确当前实现由 `git ls-tree HEAD generated/lmm-api-source` 读取；表中的迁入 SHA 仅是历史来源。

迁移时的结构检查、未提交内容保全、远端回读和 Actions 设置见 [固定迁移原始记录](https://github.com/TokenNotIncluded/lmm-api/blob/8f11ef82ac3a20e33406f2cc7478b419e74bfbe6/MIGRATION.md)。这些记录不替代当前应用构建、业务验收或生产状态。旧单体可由 [go-v0.2.118](https://github.com/TokenNotIncluded/lmm-api/releases/tag/go-v0.2.118) 重现。

文档精简前的 [状态快照](https://github.com/TokenNotIncluded/lmm-api/blob/8f11ef82ac3a20e33406f2cc7478b419e74bfbe6/STATUS.md) 仅供历史核查；当前进度不再写入父仓库。
