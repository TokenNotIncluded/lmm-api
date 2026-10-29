# lmm-core

```toml
format = "recipkg/1"

[package]
name = "lmm-core"
version = "0.1.8"
kind = "software"
description = "Rust 身份权限资金账本与多协议模型网关"
entry = "BUILD_PROMPT.md"

[dependencies.contracts]
source = "../lmm-contracts"
```

此包只含行为、契约和验收要求。第三方库、操作系统与外部服务不是隐藏的配方依赖。
来源为 AGPL-3.0-or-later 的 LMM API；参见 SOURCE.md。
