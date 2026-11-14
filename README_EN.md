# LMM API

LMM API gives individuals, teams, and operators a unified model gateway, identity and permissions, a financial ledger, and business extensions. This repository maintains the Recipkg product recipes; the implementation lives in [lmm-api-source](https://github.com/TokenNotIncluded/lmm-api-source).

**For current work and completion evidence, see the [source progress tracker](https://github.com/TokenNotIncluded/lmm-api-source/blob/main/docs/progress.md).** This repository maintains only requirements and acceptance contracts, and keeps no second status list.

[中文](README.md) · [Product requirements](recipes/lmm-api/PRODUCT_REQUIREMENTS.md) · [Integrated recipe](recipes/lmm-api/BUILD_PROMPT.md) · [Acceptance criteria](recipes/lmm-api/PRODUCTION_ACCEPTANCE.md)

## Architecture

| Component | Responsibility |
| --- | --- |
| Rust `lmm-core` | The sole authority for identity, permissions, ledger, budgets, model routing, and charging |
| Go `lmm-extensions` | Store, support, assistant, tools, operations, and external payment IO, using separate databases and narrow RPC |
| Rust `lmm` | User client and authorized application integration |
| Static Web | User, team, and administrator interfaces backed by real backend capabilities |

Core can run on its own. Extensions cannot read or write the Core database directly, and cannot authorize charges themselves. The product consists of exactly three executables; Web needs no additional resident backend.

## Use the recipes

Install [Recipkg](https://github.com/recipkg/recipkg), then check and build the integrated package:

```sh
recipkg check recipes/lmm-api
recipkg build recipes/lmm-api --lang en
```

`build` only generates reconstruction instructions. It does not compile the application or run acceptance. The integrated recipe depends on [Core](recipes/lmm-core/), [Extensions](recipes/lmm-extensions/), [Web](recipes/lmm-web/), and [CLI](recipes/lmm-cli/), with [Contracts](recipes/lmm-contracts/) shared across components. For provider and business scope, see the [payment matrix](recipes/lmm-api/PAYMENT_PARITY.md).

## Use the pinned implementation

```sh
git submodule update --init --recursive -- generated/lmm-api-source
git ls-tree HEAD generated/lmm-api-source
```

The gitlink points to the implementation commit this recipe project pins. Find and run build, test, and runtime commands inside `generated/lmm-api-source/`; the CLI and provider submodules remain nested in that repository. Report source changes, builds, independent acceptance, releases, and production deployment separately, since a recipe check is not business evidence.

## Provenance and license

[Source provenance](recipes/lmm-api/SOURCE.md) and the [repository migration record](MIGRATION.md) preserve the basis for reconstruction. The historical monolith can be reproduced from the pinned [go-v0.2.118](https://github.com/TokenNotIncluded/lmm-api/releases/tag/go-v0.2.118) release and its commit, without relying on any historical branch name.

Retain [AGPL-3.0-or-later](LICENSE), [NOTICE](NOTICE), [third-party attribution](THIRD-PARTY-LICENSES.md), QuantumNous/new-api, and all contributor credits.
