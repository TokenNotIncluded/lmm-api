# LMM API

LMM API 为个人、团队和运营者提供统一的模型入口、身份权限、资金账本与业务扩展。本仓库维护 Recipkg 产品配方；实现位于 [lmm-api-source](https://github.com/TokenNotIncluded/lmm-api-source)。

**查看最新工作与完成证据：[源码进度列表](https://github.com/TokenNotIncluded/lmm-api-source/blob/main/docs/progress.md)。** 本仓库只维护需求和验收合同，不另存一份状态表。

[English](README_EN.md) · [产品要求](recipes/lmm-api/PRODUCT_REQUIREMENTS.md) · [集成配方](recipes/lmm-api/BUILD_PROMPT.md) · [验收条件](recipes/lmm-api/PRODUCTION_ACCEPTANCE.md)

## 架构

| 组件 | 职责 |
| --- | --- |
| Rust `lmm-core` | 身份、权限、账本、预算、模型路由与计费的唯一权威 |
| Go `lmm-extensions` | 商店、客服、助手、工具、运营与外部支付 IO；独立数据库和窄 RPC |
| Rust `lmm` | 用户客户端与已授权的软件接入 |
| 静态 Web | 用户、团队与管理界面，消费真实后端能力 |

Core 可独立运行；扩展不能直接读写 Core 数据库或自行授权扣款。产品仅有上述三个可执行程序，Web 不需要第四个常驻后端。

## 使用配方

安装 [Recipkg](https://github.com/recipkg/recipkg) 后检查集成包：

```sh
recipkg check recipes/lmm-api
recipkg build recipes/lmm-api --lang zh-CN
```

`build` 生成重建提示词，不编译应用或执行验收。集成包递归依赖 [Core](recipes/lmm-core/)、[Extensions](recipes/lmm-extensions/)、[Web](recipes/lmm-web/)、[CLI](recipes/lmm-cli/)，组件共用 [Contracts](recipes/lmm-contracts/)。详细支付范围见 [支付矩阵](recipes/lmm-api/PAYMENT_PARITY.md)。

## 使用固定源码

```sh
git submodule update --init --recursive -- generated/lmm-api-source
git ls-tree HEAD generated/lmm-api-source
```

以上 gitlink 是当前配方固定的实现提交；构建、测试和运行命令在 `generated/lmm-api-source/` 中读取并执行。CLI/providers 保持源码仓库内的嵌套子模块。源码、构建、独立验收、发布和生产部署分别报告，配方检查不替代业务证据。

## 来源与许可

[来源说明](recipes/lmm-api/SOURCE.md)与[仓库迁移记录](MIGRATION.md)保留重建依据。旧单体可从固定 [go-v0.2.118](https://github.com/TokenNotIncluded/lmm-api/releases/tag/go-v0.2.118) 及其提交重现，不依赖历史分支名。

保留 [AGPL-3.0-or-later](LICENSE)、[NOTICE](NOTICE)、[第三方归属](THIRD-PARTY-LICENSES.md)、QuantumNous/new-api 及全部贡献者署名。
