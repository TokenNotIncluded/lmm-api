# LMM API

[中文](README.md)

**LMM API brings a tool marketplace, AI assistants, and team workspaces together.** Discover MCP tools, grant an assistant access, and set a budget for its calls. Team members share identity, permissions, and usage records.

This repository contains the Recipkg recipes. Application development takes place in [lmm-api-source](https://github.com/TokenNotIncluded/lmm-api-source).

| Question | Where to look |
| --- | --- |
| What should the product do? | [Product requirements](recipes/lmm-api/PRODUCT_REQUIREMENTS.md) and [payment matrix](recipes/lmm-api/PAYMENT_PARITY.md) |
| How do the components fit together, and what must pass? | [Integration recipe](recipes/lmm-api/BUILD_PROMPT.md), [integration scenarios](recipes/lmm-api/ACCEPTANCE.md) and [final acceptance criteria](recipes/lmm-api/PRODUCTION_ACCEPTANCE.md) |
| Which capabilities have been verified? | [Implementation progress and acceptance evidence](https://github.com/TokenNotIncluded/lmm-api-source/blob/main/docs/progress.md) |
| Which source shipped with a recipe release? | Look for a `recipes-v<version>` prerelease on the [releases page](https://github.com/TokenNotIncluded/lmm-api/releases). Its `source.json` identifies the implementation commit and release. Use `SHA256SUMS` to verify the downloads. See the [release procedure](.agents/RECIPKG.md). |

## Use the recipes or pinned source

Use [Recipkg](https://github.com/recipkg/recipkg) to check the recipes and generate reconstruction instructions.

```sh
recipkg check recipes/lmm-api
recipkg build recipes/lmm-api --lang en
```

To build or test the application, check out the pinned implementation and follow its development guide.

```sh
git ls-tree HEAD generated/lmm-api-source
git submodule update --init --recursive -- generated/lmm-api-source
cd generated/lmm-api-source
```

[Provenance](recipes/lmm-api/SOURCE.md) · [Migration history](MIGRATION.md) · [AGPL-3.0-or-later](LICENSE) · [NOTICE](NOTICE) · [Third-party attribution](THIRD-PARTY-LICENSES.md). Retain the credits for QuantumNous/new-api and all contributors.
