# LMM API 配方

[English](README_EN.md) · [应用源码](https://github.com/TokenNotIncluded/lmm-api-source)

这里维护 LMM API 的 Recipkg 配方和固定源码版本。LMM API 提供模型网关、AI 助手和 MCP 工具授权。

```sh
recipkg check recipes/lmm-api
recipkg build recipes/lmm-api --lang zh-CN
git submodule update --init --recursive -- generated/lmm-api-source
```

- [产品要求](recipes/lmm-api/PRODUCT_REQUIREMENTS.md) · [生产验收](recipes/lmm-api/PRODUCTION_ACCEPTANCE.md)
- [配方发布](.agents/RECIPKG.md) · [源码进度](https://github.com/TokenNotIncluded/lmm-api-source/blob/main/docs/progress.md)

源自 [QuantumNous/new-api](https://github.com/QuantumNous/new-api)。[AGPL-3.0](LICENSE) · [NOTICE](NOTICE) · [第三方许可证](THIRD-PARTY-LICENSES.md)
