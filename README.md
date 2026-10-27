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

**LMM 是 Language Model Mesh，「语言模型网络」。** 简单说，它是一个模型网关：你的账号、权限、余额和模型调用都从这里走；商店、工具市场、助手、运营和支付这些生意上的东西，则交给可插拔的扩展。之所以叫 Mesh，是因为我们希望多个模型、多条渠道和多个组件能像网一样互相连通，而不是绑死在一处。

> [!WARNING]
> **先说清楚：这个分支是一次推倒重来的微服务重构，还在施工中（WIP），现在还不能替代线上网站。**
> 模块测试通过了，并不代表整站的调用、支付和资金链路已经串起来。历史资金、订阅和 API Key 的迁移还在隔离副本里一笔笔对账，没有验收。
> 新程序不会连接旧业务库，不会偷偷升级表结构，也不会悄悄退回旧单体。还剩哪些活，写在[架构与状态](docs/core-migration.md)里。

## 它由三个程序组成

```text
lmm 客户端 / Web 控制台
        │
        ├──► lmm-core（Rust）───────► 核心 PostgreSQL、模型供应商
        │        ▲
        │        │ 授权后的内部调用
        └──► lmm-extensions（Go）───► 扩展自己的 PostgreSQL
```

| 程序 | 源码 | 它负责什么 |
| --- | --- | --- |
| `lmm` | `apps/cli` | 你手里的客户端，连上服务后调用已经提供的功能 |
| `lmm-core` | `services/core` | 身份、权限、计费、账本和模型路由；`init-db`、`bootstrap-user` 这些都是它的子命令 |
| `lmm-extensions` | `services/extensions` | 商店、工具、助手、运营与支付适配等业务扩展 |

我们刻意这样划分，是想让最要紧的东西——钱和权限——只有一个主人：

- 核心离开 Go 也能独立跑。扩展只管自己的记录，碰不到核心的余额，也碰不到核心数据库。
- `apps/web` 只产出一包静态文件，交给 Web 服务器托管，不是又一个要运维的程序。
- `lmm` 客户端单独维护在 [TokenNotIncluded/lmm](https://github.com/TokenNotIncluded/lmm)，这里通过 `apps/cli` 子模块固定版本。它能做什么、和 Magpie 对齐到哪一步，见[客户端说明](https://github.com/TokenNotIncluded/lmm#readme)。
- `contracts` 放跨进程契约，`config/legal` 放可以自己改的协议草稿，`packaging/distribution.json` 决定每个分发包里装什么。

## 上手

工具版本统一写在 [mise.toml](mise.toml)，命令都用 `mise exec --` 跑。做什么就准备什么，不用一上来把所有工具装齐，细节见[开发指南](docs/development.md)。

```sh
# 需要 CLI 时才初始化客户端子模块
git submodule update --init --depth 1 -- apps/cli

# 全新核心数据库必须用安装 owner 配置显式初始化一次；普通启动不建表、不改表
# owner 与运行凭据分开准备，见 deployment/postgres/README.md
lmm-core --config /absolute/private/core-owner.toml init-db

# 只读检查已安装数据库与运行配置
lmm-core --config /absolute/private/core.toml check-db
```

想一次拉起完整的隔离环境，用 [Docker 开发栈](deployment/docker/README.md)。核心和扩展是两个独立的 Compose 项目：更新扩展时不该顺带重启核心或它的数据库。另外提醒一句，核心健康接口有响应，不等于模型接口已经能用了。

## 配置放在哪

新安装一律使用 TOML：`/etc/lmm.conf.d/core.toml` 和 `/etc/lmm.conf.d/extensions.toml`，也可以用 `--config` 显式指定。

| 内容 | 路径 |
| --- | --- |
| 私密文件 | `/etc/lmm.conf.d/secrets/<服务>/` |
| 持久状态 | `/var/lib/lmm/<服务>/` |
| 运行时文件 | `/run/lmm/<服务>/` |

完整规则和旧包迁移的边界见[配置与文件路径](docs/filesystem-layout.md)。路径统一了，不代表旧安装会自动转换过来。

## 接下来看什么

| 你想…… | 去这里 |
| --- | --- |
| 弄清边界和还剩什么没做 | [架构与状态](docs/core-migration.md) |
| 拉起一套隔离环境 | [Docker 开发栈](deployment/docker/README.md) |
| 用 mise 开发、跑测试 | [开发指南](docs/development.md) |
| 配置 TOML、服务目录和升级保留规则 | [配置与文件路径](docs/filesystem-layout.md) |
| 改用户、隐私、退款协议 | [协议模板](docs/legal/README.md) |
| 构建和检查分发包 | [打包与分发](docs/release-architecture.md) |
| 翻详细接口和历史资料 | [文档目录](docs/README.md) |

## 分支

日常开发都在 `wip/rust-core-go-extensions` 上；旧单体的 `main` 和 `traditional` 已经冻结，只作历史留档。[主线交接](docs/development-branches.md)不等于已经上线或验收。[Extore 商品同步](docs/extore-shop-binding-followup.md)还没做完，请不要部署。

## 一起参与

欢迎来帮忙！动手前先看一眼[贡献指南](docs/contributing.md)。发现漏洞请按[安全指南](.github/SECURITY.md)私下告诉我们，别把密钥或用户数据贴到公开的地方。合并、打包、发布和部署是四件不同的事，各自单独决定。

## 致谢与许可证

这个项目站在 [QuantumNous/new-api](https://github.com/QuantumNous/new-api) 的肩膀上发展而来，感谢上游作者和所有贡献者。代码按 [AGPL-3.0](LICENSE) 发布；请保留 [NOTICE](NOTICE)、[分支来源与归属](docs/fork.md) 和[第三方许可证](THIRD-PARTY-LICENSES.md)。[线上服务](https://api.lmm.best)和本分支的实现进度并不一样。
