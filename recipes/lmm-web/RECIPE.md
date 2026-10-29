# lmm-web

```toml
format = "recipkg/1"

[package]
name = "lmm-web"
version = "0.1.4"
kind = "software"
description = "保留多语言与功能可用性边界的静态 Web 控制台"
entry = "BUILD_PROMPT.md"

[dependencies.contracts]
source = "../lmm-contracts"
```

此包只含行为、契约和验收要求。第三方库、操作系统与外部服务不是隐藏的配方依赖。
来源为 AGPL-3.0-or-later 的 LMM API；参见 SOURCE.md。
