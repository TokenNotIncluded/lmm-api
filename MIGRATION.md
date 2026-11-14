# 仓库拆分记录

2026-10-28，`TokenNotIncluded/lmm-api` 调整为 Recipkg 配方项目，可编译的实现迁入独立仓库 `TokenNotIncluded/lmm-api-source`。本页只说明仓库的来源与职责分工；最新工作统一见 [源码进度列表](https://github.com/TokenNotIncluded/lmm-api-source/blob/main/docs/progress.md)。

| 内容 | 位置与来源 |
| --- | --- |
| 需求、公开接口、验收 | 本仓库 `recipes/lmm-api/` 及五个组件/合同包 |
| 实现 | 源码仓库；父项目通过 `generated/lmm-api-source/` 的 gitlink 固定版本 |
| 原实现迁出基准 | `lmm-api@ff01f8456b978089bce08175cbd357c3d22c3983` |
| 初次迁入快照 | `lmm-api-source@06d8b4b7c9630106c6ef47425f9e67d333bd1548` |
| CLI/providers | 仍是源码仓库内的嵌套子模块，保留各自独立的 Git 历史 |
| 共享技能 | 由本仓库以子模块固定；自定义技能的源码路径继续在实现仓库中使用 |

初次迁入以原先已跟踪的文件树为基础，只调整了迁移入口说明；原提交历史和未提交的工作按当时的记录保留。构建命令请在源码根目录读取，旧的绝对缓存路径不再是可用环境。当前确切的实现版本用 `git ls-tree HEAD generated/lmm-api-source` 读取；表中的迁入 SHA 仅表示历史来源。

迁移时的结构检查、未提交内容保全、远端回读和 Actions 设置，见[固定迁移原始记录](https://github.com/TokenNotIncluded/lmm-api/blob/8f11ef82ac3a20e33406f2cc7478b419e74bfbe6/MIGRATION.md)。这些记录不能代替当前的应用构建、业务验收或生产状态。旧单体可由 [go-v0.2.118](https://github.com/TokenNotIncluded/lmm-api/releases/tag/go-v0.2.118) 复现。

文档精简前的[状态快照](https://github.com/TokenNotIncluded/lmm-api/blob/8f11ef82ac3a20e33406f2cc7478b419e74bfbe6/STATUS.md)仅供历史核查；当前进度不再写入父仓库。
