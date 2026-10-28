# LMM API

![Software and dependencies become a recipe for rebuilding software](https://raw.githubusercontent.com/recipkg/recipkg/main/assets/recipkg-cover.png)

This repository is now the Recipkg recipe project. [The integrated recipe](recipes/lmm-api/BUILD_PROMPT.md), its recursive component dependencies and [acceptance criteria](recipes/lmm-api/ACCEPTANCE.md) are the primary inputs.

Implementation lives in [TokenNotIncluded/lmm-api-source](https://github.com/TokenNotIncluded/lmm-api-source), pinned as the `generated/lmm-api-source/` Git submodule. Run implementation build/test commands from that directory. The CLI and provider submodules remain nested in the implementation repository.

```sh
recipkg check recipes/lmm-api
recipkg build recipes/lmm-api --lang en
git submodule update --init --recursive -- generated/lmm-api-source
```

The six recipes preserve Rust Core, Go extensions, the React/TypeScript static Web client, Rust CLI, and shared contracts. Shared project skills remain pinned under `.agents/skills/`; custom skills are preserved. Historical monolith branches are `传统` and `traditional`; `main` is the recipe project's default branch.

Repository migration and structural recipe checks do not prove successful reconstruction, business acceptance or production deployment. Uncommitted implementation work remains local to the implementation submodule. See [migration notes](MIGRATION.md), [source evidence](recipes/lmm-api/SOURCE.md) and [中文说明](README.md).

Retain AGPL-3.0-or-later, LICENSE, NOTICE, third-party licenses, QuantumNous/new-api and all contributor attribution.
