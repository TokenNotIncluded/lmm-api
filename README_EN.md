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

**LMM stands for Language Model Mesh.** Put simply, it's a model gateway: your account, permissions, balance and model calls all go through it, while the business side — the store, tool market, assistant, operations and payment adapters — lives in pluggable extensions. We call it a mesh because we want models, channels and components to connect to one another like a net, instead of being welded to a single place.

> [!WARNING]
> **Let's be upfront: this branch is a ground-up microservice rewrite. It's still under construction (WIP) and can't replace the live site yet.**
> Passing module tests doesn't mean model calls, payments and funds flows are wired together end to end. Historical funds, subscriptions and API keys are still being reconciled, line by line, on isolated copies, and haven't been accepted.
> The new runtime never connects to the old application database, never quietly upgrades schemas and never silently falls back to the legacy server. What's left to do lives in [architecture and status](docs/core-migration.md).

## Three programs

```text
lmm client / Web console
        │
        ├──► lmm-core (Rust) ────────► core PostgreSQL, model providers
        │        ▲
        │        │ authorized internal calls
        └──► lmm-extensions (Go) ────► its own PostgreSQL
```

| Program | Source | What it does |
| --- | --- | --- |
| `lmm` | `apps/cli` | The client in your hands: connects to services and calls the features they offer |
| `lmm-core` | `services/core` | Identity, permissions, billing, ledger and model routing; `init-db`, `bootstrap-user` and friends are its subcommands |
| `lmm-extensions` | `services/extensions` | Business extensions for the store, tools, assistant, operations and payment adapters |

We split things this way on purpose, so the things that matter most — money and permissions — have exactly one owner:

- The core runs fine without Go. Extensions own only their own records; they can't touch core balances or the core database.
- `apps/web` produces a bundle of static files for a Web server to host. It isn't one more program to operate.
- The Rust `lmm` client is maintained separately in [TokenNotIncluded/lmm](https://github.com/TokenNotIncluded/lmm) and pinned here as the `apps/cli` submodule. See the [client README](https://github.com/TokenNotIncluded/lmm#readme) for what it can do and how far Magpie parity has come.
- `contracts` holds cross-process contracts, `config/legal` holds policy drafts you can edit, and `packaging/distribution.json` decides what goes into each archive.

## Getting started

Tool versions live in [mise.toml](mise.toml); run everything through `mise exec --`. Set up only what your task needs — there's no need to install everything up front. See [Development](docs/development.md) for details.

```sh
# Initialize the client submodule only when you need the CLI
git submodule update --init --depth 1 -- apps/cli

# Initialize a new core database once with the installation-owner config; normal startup never creates or alters tables
# Prepare owner and runtime credentials separately; see deployment/postgres/README.md
lmm-core --config /absolute/private/core-owner.toml init-db

# Read-only check of the installed database and runtime configuration
lmm-core --config /absolute/private/core.toml check-db
```

To bring up a complete isolated environment in one go, use the [Docker development stack](deployment/docker/README.md). Core and extensions are separate Compose projects, so updating extensions shouldn't restart the core or its database. One more heads-up: a responding core health route doesn't mean the model routes are ready.

## Where configuration lives

New installations use TOML: `/etc/lmm.conf.d/core.toml` and `/etc/lmm.conf.d/extensions.toml`, or an explicit `--config` path.

| Content | Path |
| --- | --- |
| Secrets | `/etc/lmm.conf.d/secrets/<service>/` |
| Persistent state | `/var/lib/lmm/<service>/` |
| Runtime files | `/run/lmm/<service>/` |

See [configuration and filesystem layout](docs/filesystem-layout.md) for ownership, upgrades and legacy package boundaries. A shared layout doesn't mean an existing installation gets migrated automatically.

## Where to go next

| You want to… | Go here |
| --- | --- |
| Understand the boundaries and what's still missing | [Architecture and status](docs/core-migration.md) |
| Bring up an isolated environment | [Docker development stack](deployment/docker/README.md) |
| Develop and run tests with mise | [Development](docs/development.md) |
| Configure TOML, service directories and upgrade preservation | [Configuration and filesystem layout](docs/filesystem-layout.md) |
| Edit user, privacy and refund policies | [Policy templates](docs/legal/README.md) |
| Build and inspect separate archives | [Packaging and distribution](docs/release-architecture.md) |
| Dig into detailed contracts and historical material | [Documentation](docs/README.md) |

## Branches

Day-to-day development happens on `wip/rust-core-go-extensions`. The legacy monolith branches `main` and `traditional` are frozen and kept for history. [Branch promotion](docs/development-branches.md) isn't deployment or acceptance. [Extore product sync](docs/extore-shop-binding-followup.md) is still unfinished — please don't deploy it.

## Getting involved

Help is welcome! Take a quick look at the [contribution guide](docs/contributing.md) before you start. If you find a vulnerability, please tell us privately via the [security policy](.github/SECURITY.md), and never post keys or user data in public. Merging, packaging, publishing and deploying are four separate decisions, each made on its own.

## Thanks and license

This project grew out of [QuantumNous/new-api](https://github.com/QuantumNous/new-api) — many thanks to its authors and every contributor. It's released under [AGPL-3.0](LICENSE); please keep [NOTICE](NOTICE), [fork provenance](docs/fork.md) and [third-party notices](THIRD-PARTY-LICENSES.md) intact. The [hosted service](https://api.lmm.best) and this branch are at different stages of implementation.
