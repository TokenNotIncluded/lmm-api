# LMM API recipes

[简体中文](README.md) · [Application source](https://github.com/TokenNotIncluded/lmm-api-source)

This repository contains the Recipkg recipes and pinned implementation for LMM API, a model gateway with AI assistants and MCP tool access.

```sh
recipkg check recipes/lmm-api
recipkg build recipes/lmm-api --lang en
git submodule update --init --recursive -- generated/lmm-api-source
```

- [Product requirements](recipes/lmm-api/PRODUCT_REQUIREMENTS.md) · [Production acceptance](recipes/lmm-api/PRODUCTION_ACCEPTANCE.md)
- [Recipe releases](.agents/RECIPKG.md) · [Implementation progress](https://github.com/TokenNotIncluded/lmm-api-source/blob/main/docs/progress.md)

Derived from [QuantumNous/new-api](https://github.com/QuantumNous/new-api). [AGPL-3.0](LICENSE) · [NOTICE](NOTICE) · [Third-party licenses](THIRD-PARTY-LICENSES.md)
