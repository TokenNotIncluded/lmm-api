<div align="center">
  <img src=".github/assets/lmm-logo.svg" alt="LMM API · Language Model Mesh" width="520" />
  <h1>LMM API</h1>
  <p>LMM API · 连接模型与工具，让智能自由流动</p>
  <p>
    <a href="https://github.com/TokenNotIncluded/lmm-api/actions/workflows/core-protocol.yml"><img src="https://github.com/TokenNotIncluded/lmm-api/actions/workflows/core-protocol.yml/badge.svg?branch=wip%2Frust-core-go-extensions" alt="Microkernel checks" /></a>
    <a href="LICENSE"><img src="https://img.shields.io/badge/license-AGPL--3.0-blue" alt="License: AGPL-3.0" /></a>
  </p>
  <p><a href="README.md">简体中文</a> · <a href="README_EN.md">English</a></p>
</div>

**LMM 帮你把各种大语言模型统一管理起来** —— 身份认证、权限控制、计费账本、模型路由，一个网关全部搞定；商店、工具市场、助手、运营和支付适配则交给可插拔的扩展模块。**LMM 全称 Language Model Mesh（语言模型网络）**，Mesh 的含义正是多模型路由、渠道共享和组件之间的协作。

> [!WARNING]
> **这是我们正在精心打造的下一代微服务架构（WIP），目前还不能替代现有网站。**
> 各模块的单元测试已经通过，但整站调用、支付和资金链路的端到端联通仍在推进中。历史资金、订阅和 API Key 的迁移正在隔离副本中逐步对账，尚未最终验收。
> 程序设计上不会连接旧业务库、不会自动升级表结构，也不会回退到旧单体，请放心。剩余工作和详细状态请参阅[架构与状态](docs/core-migration.md)。

## 三个程序

LMM 由三个独立的程序协作完成工作：

```text
lmm 客户端 / Web 控制台
        │
        ├──► lmm-core（Rust）───────► 核心 PostgreSQL、模型供应商
        │        ▲
        │        │ 授权后的内部调用
        └──► lmm-extensions（Go）───► 扩展自己的 PostgreSQL
```

| 程序 | 源码 | 职责 |
| --- | --- | --- |
| `lmm` | `apps/cli` | 用户客户端，连接服务并调用已提供的功能 |
| `lmm-core` | `services/core` | 身份、权限、计费、账本与模型路由；`init-db`、`bootstrap-user` 等是它的子命令 |
| `lmm-extensions` | `services/extensions` | 商店、工具、助手、运营与支付适配等业务扩展 |

- 核心不依赖 Go 也能运行；扩展只管理自己的记录，不能直接改核心余额或数据库。
- `apps/web` 产出静态 Web 包，由 Web 服务器提供，不是额外的可执行程序。
- `lmm` 客户端独立维护于 [TokenNotIncluded/lmm](https://github.com/TokenNotIncluded/lmm)，这里以 `apps/cli` 子模块固定版本。能力及 Magpie 对齐进度见[客户端说明](https://github.com/TokenNotIncluded/lmm#readme)。
- `contracts` 保存跨进程契约，`config/legal` 保存可编辑协议草稿，`packaging/distribution.json` 定义各分发包内容。

## 快速开始

准备好了？按照下面的步骤开始吧！工具版本以 [mise.toml](mise.toml) 为准，命令通过 `mise exec --` 执行。按任务只准备需要的工具，详见[开发指南](docs/development.md)。

```sh
# 需要 CLI 时才初始化客户端子模块
git submodule update --init --depth 1 -- apps/cli

# 全新核心数据库必须用安装 owner 配置显式初始化一次；普通启动不建表、不改表
# owner 与运行凭据分开准备，见 deployment/postgres/README.md
lmm-core --config /absolute/private/core-owner.toml init-db

# 只读检查已安装数据库与运行配置
lmm-core --config /absolute/private/core.toml check-db
```

隔离的整套环境用 [Docker 开发栈](deployment/docker/README.md)启动。核心与扩展是独立 Compose 项目：更新扩展不应重启核心或其数据库；核心健康接口可响应不代表模型接口已就绪。

## 配置与目录

新安装统一使用 TOML 格式的配置文件，以下是默认路径一览：`/etc/lmm.conf.d/core.toml` 与 `/etc/lmm.conf.d/extensions.toml`，也可通过 `--config` 显式指定。

| 内容 | 路径 |
| --- | --- |
| 私密文件 | `/etc/lmm.conf.d/secrets/<服务>/` |
| 持久状态 | `/var/lib/lmm/<服务>/` |
| 运行时文件 | `/run/lmm/<服务>/` |

完整规则和旧包迁移边界见[配置与文件路径](docs/filesystem-layout.md)。统一路径不代表旧安装已自动转换。

## 文档入口

不确定从哪里开始？根据你想做的事情选择入口：

| 想做什么 | 入口 |
| --- | --- |
| 了解边界与剩余工作 | [架构与状态](docs/core-migration.md) |
| 启动隔离环境 | [Docker 开发栈](deployment/docker/README.md) |
| 使用 mise 开发、测试 | [开发指南](docs/development.md) |
| 统一 TOML 配置、服务目录与升级保留规则 | [配置与文件路径](docs/filesystem-layout.md) |
| 配置用户、隐私、退款协议 | [协议模板](docs/legal/README.md) |
| 构建和检查分发包 | [打包与分发](docs/release-architecture.md) |
| 查阅详细接口及历史资料 | [文档目录](docs/README.md) |

## 分支说明

我们的开发主线是 `wip/rust-core-go-extensions`；旧单体分支 `main` 与 `traditional` 已冻结，不再接受变更。需要注意的是，[主线交接](docs/development-branches.md)并不代表已上线或通过验收。此外，[Extore 商品同步](docs/extore-shop-binding-followup.md)功能尚未完成，目前还不具备部署条件。

## 贡献与许可证

欢迎参与贡献！在开始之前，请先阅读[贡献指南](docs/contributing.md)。如果发现安全漏洞，请按[安全指南](.github/SECURITY.md)私下报告，不要公开密钥或用户数据。合并、打包、发布和部署是分开的决定。

项目基于 [QuantumNous/new-api](https://github.com/QuantumNous/new-api) 发展，按 [AGPL-3.0](LICENSE) 发布。保留 [NOTICE](NOTICE)、[分支来源与归属](docs/fork.md) 和[第三方许可证](THIRD-PARTY-LICENSES.md)。[线上服务](https://api.lmm.best)与本分支的实现状态不同。
