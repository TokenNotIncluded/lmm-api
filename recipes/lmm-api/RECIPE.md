# lmm-api

```toml
format = "recipkg/1"

[package]
name = "lmm-api"
version = "0.1.10"
kind = "software"
description = "LMM 模型网关、身份资金核心、可插拔业务扩展与客户端的集成配方"
entry = "BUILD_PROMPT.md"

[dependencies.core]
source = "../lmm-core"

[dependencies.extensions]
source = "../lmm-extensions"

[dependencies.web]
source = "../lmm-web"

[dependencies.cli]
source = "../lmm-cli"
```

此包只含行为、契约和验收要求。第三方库、操作系统与外部服务不是隐藏的配方依赖。
来源为 AGPL-3.0-or-later 的 LMM API；参见 SOURCE.md。
