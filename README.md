# LMM API

LMM API 面向个人、团队和运营者，提供统一的模型入口、身份与权限、资金账本以及业务扩展。本仓库维护 Recipkg 产品配方，具体实现位于 [lmm-api-source](https://github.com/TokenNotIncluded/lmm-api-source)。

**最新进展与完成证据见 [源码进度列表](https://github.com/TokenNotIncluded/lmm-api-source/blob/main/docs/progress.md)。** 本仓库只维护需求和验收合同，不再另存一份状态表。

[English](README_EN.md) · [产品要求](recipes/lmm-api/PRODUCT_REQUIREMENTS.md) · [集成配方](recipes/lmm-api/BUILD_PROMPT.md) · [验收条件](recipes/lmm-api/PRODUCTION_ACCEPTANCE.md)

## 架构

| 组件 | 职责 |
| --- | --- |
| Rust `lmm-core` | 身份、权限、账本、预算、模型路由与计费的唯一权威 |
| Go `lmm-extensions` | 商店、客服、助手、工具、运营与外部支付 IO；使用独立数据库和窄 RPC |
| Rust `lmm` | 面向用户的客户端，以及经授权的软件接入 |
| 静态 Web | 用户、团队与管理界面，直接使用真实的后端能力 |

Core 可以独立运行。扩展不能直接读写 Core 数据库，也不能自行授权扣款。产品只包含上述三个可执行程序，Web 不需要第四个常驻后端。

## 使用配方

先安装 [Recipkg](https://github.com/recipkg/recipkg)，再检查并构建集成包：

```sh
recipkg check recipes/lmm-api
recipkg build recipes/lmm-api --lang zh-CN
```

`build` 只生成重建提示词，不会编译应用，也不会执行验收。集成包递归依赖 [Core](recipes/lmm-core/)、[Extensions](recipes/lmm-extensions/)、[Web](recipes/lmm-web/) 和 [CLI](recipes/lmm-cli/)，各组件共用 [Contracts](recipes/lmm-contracts/)。支付的详细范围见 [支付矩阵](recipes/lmm-api/PAYMENT_PARITY.md)。

## 使用固定源码

```sh
git submodule update --init --recursive -- generated/lmm-api-source
git ls-tree HEAD generated/lmm-api-source
```

该 gitlink 指向当前配方固定的实现提交。构建、测试和运行命令请在 `generated/lmm-api-source/` 内查阅并执行；CLI 与 providers 仍作为嵌套子模块保留在源码仓库中。源码变更、构建、独立验收、发布和生产部署须分别报告，配方检查不能代替业务证据。

## 来源与许可

[来源说明](recipes/lmm-api/SOURCE.md)和[仓库迁移记录](MIGRATION.md)保存了重建依据。旧单体可通过固定的 [go-v0.2.118](https://github.com/TokenNotIncluded/lmm-api/releases/tag/go-v0.2.118) 及其提交复现，不依赖历史分支名。

请保留 [AGPL-3.0-or-later](LICENSE)、[NOTICE](NOTICE)、[第三方归属](THIRD-PARTY-LICENSES.md)、QuantumNous/new-api 以及全部贡献者的署名。
