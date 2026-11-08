# CLI 配方来源

本包定义独立 Rust 客户端、系统凭据、软件发现、预览确认和明确阻塞行为。要求和验收分别在本包 `BUILD_PROMPT.md`、`ACCEPTANCE.md`；当前工作与完成证据统一见[源码进度列表](https://github.com/TokenNotIncluded/lmm-api-source/blob/main/docs/progress.md)，本页只保存来源。

## 固定来源

| 来源 | 用途 |
| --- | --- |
| [旧单体 go-v0.2.118](https://github.com/TokenNotIncluded/lmm-api/releases/tag/go-v0.2.118)，源码 `cc52aa195c6f2a01d703a1a407be73c31b89867d` | 可重现的旧行为比较，不依赖历史分支存在 |
| 原行为观察 `d468affecfd76a5123358732fb9a4e0cc1f63566` | 早期配方的源码观察依据，不表示当前实现 |
| 迁出 `ff01f8456b978089bce08175cbd357c3d22c3983` → 源码仓初始 `06d8b4b7c9630106c6ef47425f9e67d333bd1548` | 仓库拆分来源 |
| [本包历史来源原文](https://github.com/TokenNotIncluded/lmm-api/blob/8f11ef82ac3a20e33406f2cc7478b419e74bfbe6/recipes/lmm-cli/SOURCE.md) | 逐轮变更提交、失败/通过范围及当时限制的固定记录 |

CLI 的历史观察版本与 Pi Provider 参考保留在固定来源原文；当前产品支持范围以客户端配方及真实目标软件/版本验收为准。Magpie 对齐是新增产品目标，不计为旧服务端回退。

## 当前实现与证据

父仓库 `git ls-tree HEAD generated/lmm-api-source` 给出实现提交；再从该源码提交读取 `apps/cli` 和 providers 的 gitlink。实现及其子模块不是隐藏的配方依赖，不进入 Markdown 配方归档。实施指南见[CLI 源码仓库](https://github.com/TokenNotIncluded/lmm)，执行命令以固定源码内的文件为准。

旧记录只证明其绑定输入和明确范围；代码存在、单项自测、合成上游、真实 PG/进程、浏览器、外部服务、发布和生产分别验收。来源文档不记录动态百分比或复制另一份待办。

保留 QuantumNous/new-api、其他贡献者、AGPL-3.0-or-later、NOTICE 和第三方归属。复用代码/素材/字体须按各自许可，不推断商业授权。
