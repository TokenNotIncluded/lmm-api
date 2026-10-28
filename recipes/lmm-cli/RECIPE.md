# lmm-cli

```toml
format = "recipkg/1"

[package]
name = "lmm-cli"
version = "0.1.0"
kind = "software"
description = "Rust 桌面 OAuth、只读发现与显式应用接入预览客户端"
entry = "BUILD_PROMPT.md"

[dependencies.contracts]
source = "../lmm-contracts"
```

此包只含行为、契约和验收要求。第三方库、操作系统与外部服务不是隐藏的配方依赖。
来源为 AGPL-3.0-or-later 的 LMM API；参见 SOURCE.md。
