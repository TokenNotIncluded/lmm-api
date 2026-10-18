<div align="center">
  <img src=".github/assets/lmm-logo.svg" alt="LMM API · Language Model Mesh" width="520" />
  <h1>LMM API</h1>
  <p>LMM API · Connecting models and tools, letting intelligence flow freely</p>
  <p>
    <a href="https://github.com/TokenNotIncluded/lmm-api/actions/workflows/core-protocol.yml"><img src="https://github.com/TokenNotIncluded/lmm-api/actions/workflows/core-protocol.yml/badge.svg?branch=wip%2Frust-core-go-extensions" alt="Microkernel checks" /></a>
    <a href="LICENSE"><img src="https://img.shields.io/badge/license-AGPL--3.0-blue" alt="License: AGPL-3.0" /></a>
  </p>
  <p><a href="README.md">简体中文</a> · <a href="README_EN.md">English</a></p>
</div>

**LMM — Language Model Mesh (语言模型网络)** — helps you connect language models, tools, and services through a single, intelligently-routed gateway. A lean Rust core handles identity, permissions, billing, and model routing, while Go-based extensions carry the store, tool market, assistant, operations, and payment adapters. The "Mesh" in our name reflects what we care about most: multi-model routing, channel sharing, and seamless component collaboration.

> [!WARNING]
> **You're looking at our next-generation microservice rewrite — exciting work that's still in progress.**
> This branch is a fresh-install architecture and is not yet a drop-in replacement for the existing site. Passing module tests does not prove complete model, payment, or funds flows. Historical funds, subscriptions, and API keys are still being reconciled on isolated copies and are not accepted here.
> The runtime does not connect to the old application database, automatically upgrade schemas, or fall back to the legacy server. Check our [architecture and status](docs/core-migration.md) document for a full picture of the remaining work.

## Three programs

LMM is organized as three focused programs that work together. Here's how they relate to each other:

```text
lmm client / Web console
        │
        ├──► lmm-core (Rust) ────────► core PostgreSQL, model providers
        │        ▲
        │        │ authorized internal calls
        └──► lmm-extensions (Go) ────► its own PostgreSQL
```

| Program | Source | Responsibility |
| --- | --- | --- |
| `lmm` | `apps/cli` | User client that connects to services and calls their available features |
| `lmm-core` | `services/core` | Identity, permissions, billing, ledger and model routing; `init-db`, `bootstrap-user` and others are subcommands of this program |
| `lmm-extensions` | `services/extensions` | Business extensions for the store, tools, assistant, operations and payment adapters |

- The core runs without Go. Extensions own their records only and cannot edit core balances or the core database.
- `apps/web` produces a static Web archive served by a Web server; it is not an extra executable.
- The Rust `lmm` client is maintained in [TokenNotIncluded/lmm](https://github.com/TokenNotIncluded/lmm) and pinned here as the `apps/cli` submodule. See the [client README](https://github.com/TokenNotIncluded/lmm#readme) for features and Magpie parity progress.
- `contracts` holds cross-process contracts, `config/legal` holds editable policy drafts, and `packaging/distribution.json` defines archive contents.

## Quick start

Ready to dive in? Tool versions come from [mise.toml](mise.toml); run tools through `mise exec --`. You only need to prepare what your task requires — see [Development](docs/development.md) for the full walkthrough.

```sh
# Initialize the client submodule only when you need the CLI
git submodule update --init --depth 1 -- apps/cli

# Initialize a new core database once with the installation-owner config; normal startup never creates or alters tables
# Prepare owner and runtime credentials separately; see deployment/postgres/README.md
lmm-core --config /absolute/private/core-owner.toml init-db

# Read-only check of the installed database and runtime configuration
lmm-core --config /absolute/private/core.toml check-db
```

Start a full isolated environment with the [Docker development stack](deployment/docker/README.md). Core and extensions are separate Compose projects: updating extensions must not restart the core or its database, and a responding core health route does not mean model routes are ready.

## Configuration and directories

We use a clean, predictable layout so you always know where things live. New installations use TOML: `/etc/lmm.conf.d/core.toml` and `/etc/lmm.conf.d/extensions.toml`, or an explicit `--config` path.

| Content | Path |
| --- | --- |
| Secrets | `/etc/lmm.conf.d/secrets/<service>/` |
| Persistent state | `/var/lib/lmm/<service>/` |
| Runtime files | `/run/lmm/<service>/` |

See [configuration and filesystem layout](docs/filesystem-layout.md) for ownership, upgrades and legacy package boundaries. This convention does not automatically migrate an existing installation.

## Documentation

Not sure where to start? Here's a quick map to help you find what you need:

| Task | Entry point |
| --- | --- |
| Understand ownership and remaining work | [Architecture and status](docs/core-migration.md) |
| Start an isolated environment | [Docker development stack](deployment/docker/README.md) |
| Develop and test with mise | [Development](docs/development.md) |
| Configure TOML, service directories and upgrade preservation | [Configuration and filesystem layout](docs/filesystem-layout.md) |
| Configure user, privacy and refund policies | [Policy templates](docs/legal/README.md) |
| Build and inspect separate archives | [Packaging and distribution](docs/release-architecture.md) |
| Find detailed contracts and historical material | [Documentation](docs/README.md) |

## Branches

All active development happens on the `wip/rust-core-go-extensions` branch — that's the one you're reading now. The legacy `main` and `traditional` branches are preserved as historical snapshots and are no longer updated. Please note that [branch promotion](docs/development-branches.md) is a development milestone, not a deployment or acceptance decision. The [Extore product sync](docs/extore-shop-binding-followup.md) remains incomplete and is not ready for deployment.

## Contribution and license

We'd love to have you contribute! Start by reading the [contribution guide](docs/contributing.md). If you discover a security vulnerability, please follow the [security policy](.github/SECURITY.md) — never disclose keys or user data publicly. Keep in mind that merging, packaging, publishing, and deploying are separate decisions in our workflow.

The project builds on [QuantumNous/new-api](https://github.com/QuantumNous/new-api) under [PERSON_NAME]. Please preserve [NOTICE](NOTICE), [fork provenance](docs/fork.md) and [third-party notices](THIRD-PARTY-LICENSES.md). The [hosted service](https://api.lmm.best) and this branch have different implementation states.
