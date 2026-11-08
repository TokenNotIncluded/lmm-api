# LMM API

LMM API provides a shared model gateway, identity and permissions, a financial ledger, and business extensions for individuals, teams, and operators. This repository maintains the Recipkg product recipes. Implementation lives in [lmm-api-source](https://github.com/TokenNotIncluded/lmm-api-source).

**Current work and completion evidence: [source progress tracker](https://github.com/TokenNotIncluded/lmm-api-source/blob/main/docs/progress.md).** This repository maintains requirements and acceptance contracts without a second status list.

[中文](README.md) · [Product requirements](recipes/lmm-api/PRODUCT_REQUIREMENTS.md) · [Integrated recipe](recipes/lmm-api/BUILD_PROMPT.md) · [Acceptance criteria](recipes/lmm-api/PRODUCTION_ACCEPTANCE.md)

## Architecture

| Component | Responsibility |
| --- | --- |
| Rust `lmm-core` | Sole authority for identity, permissions, ledger, budgets, model routing, and charging |
| Go `lmm-extensions` | Store, support, assistant, tools, operations, and external payment IO through separate databases and scoped RPC |
| Rust `lmm` | User client and authorized application integration |
| Static Web | User, team, and administrator interfaces using actual backend capabilities |

Core can run independently. Extensions cannot directly access the Core database or authorize charges. There are three product executables; Web does not require another resident backend.

## Use the recipes

Install [Recipkg](https://github.com/recipkg/recipkg), then check the integrated package:

```sh
recipkg check recipes/lmm-api
recipkg build recipes/lmm-api --lang en
```

`build` generates reconstruction instructions; it does not compile or accept the application. The integrated recipe depends on [Core](recipes/lmm-core/), [Extensions](recipes/lmm-extensions/), [Web](recipes/lmm-web/), and [CLI](recipes/lmm-cli/), with shared [Contracts](recipes/lmm-contracts/). See the [payment matrix](recipes/lmm-api/PAYMENT_PARITY.md) for provider and business scope.

## Use the pinned implementation

```sh
git submodule update --init --recursive -- generated/lmm-api-source
git ls-tree HEAD generated/lmm-api-source
```

The gitlink identifies the implementation pinned by this recipe project. Read and run implementation build, test, and runtime commands inside `generated/lmm-api-source/`. CLI/provider submodules remain nested there. Source changes, builds, independent acceptance, releases, and production deployment require separate evidence.

## Provenance and license

[Source provenance](recipes/lmm-api/SOURCE.md) and the [repository migration record](MIGRATION.md) preserve the reconstruction basis. The historical monolith is reproducible from [go-v0.2.118](https://github.com/TokenNotIncluded/lmm-api/releases/tag/go-v0.2.118) and its commit; no historical branch name is required.

Retain [AGPL-3.0-or-later](LICENSE), [NOTICE](NOTICE), [third-party attribution](THIRD-PARTY-LICENSES.md), QuantumNous/new-api, and all contributor credits.
