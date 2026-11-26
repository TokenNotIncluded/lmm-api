# LMM API

[English](README_EN.md)

**LMM API 把工具市场、AI 助手和团队工作区连接在一起。** 你可以在市场发现 MCP 工具，为助手授权，并设置调用预算。团队成员共用一套身份、权限和用量记录。

本仓库维护 Recipkg 配方。应用开发在 [lmm-api-source](https://github.com/TokenNotIncluded/lmm-api-source) 进行。

| 想查什么 | 入口 |
| --- | --- |
| 产品行为与支付范围 | [产品要求](recipes/lmm-api/PRODUCT_REQUIREMENTS.md) · [支付矩阵](recipes/lmm-api/PAYMENT_PARITY.md) |
| 如何装配和验收 | [集成配方](recipes/lmm-api/BUILD_PROMPT.md) · [集成场景](recipes/lmm-api/ACCEPTANCE.md) · [最终验收条件](recipes/lmm-api/PRODUCTION_ACCEPTANCE.md) |
| 哪些能力已经验证 | [源码进度与验收证据](https://github.com/TokenNotIncluded/lmm-api-source/blob/main/docs/progress.md) |
| 如何核对已发布配方 | 在[发布页](https://github.com/TokenNotIncluded/lmm-api/releases)查找 `recipes-v<版本>` 预发布。其 `source.json` 记录实现提交和发布地址，`SHA256SUMS` 用于校验下载文件。见[发布流程](.agents/RECIPKG.md)。 |

## 使用配方或固定源码

[Recipkg](https://github.com/recipkg/recipkg) 检查配方结构并生成重建提示词。

```sh
recipkg check recipes/lmm-api
recipkg build recipes/lmm-api --lang zh-CN
```

要构建或测试应用，先取出本仓库固定的实现，再按源码仓库的说明操作。

```sh
git ls-tree HEAD generated/lmm-api-source
git submodule update --init --recursive -- generated/lmm-api-source
cd generated/lmm-api-source
```

OpenCode / Magpie 的 LMM OAuth 插件以 Git 子模块固定在 `generated/lmm-api-source/packages/opencode-lmm-auth`。该插件在 TokenNotIncluded 独立开源。 [插件源码与使用说明](https://github.com/TokenNotIncluded/opencode-lmm-auth)

[来源](recipes/lmm-api/SOURCE.md) · [迁移记录](MIGRATION.md) · [AGPL-3.0-or-later](LICENSE) · [NOTICE](NOTICE) · [第三方归属](THIRD-PARTY-LICENSES.md)。保留 QuantumNous/new-api 及各贡献者署名。
