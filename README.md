<div align="center">
  <img src="https://raw.githubusercontent.com/TokenNotIncluded/lmm-api-source/7abe06d5d369b2ccefe4364099450bfae4534953/.github/assets/lmm-logo.svg" width="520" alt="LMM API · Language Model Mesh">
  <h1>LMM API</h1>
  <p><strong>连接模型与工具，让智能自由流动。</strong></p>
  <p>Rust Core · Go Extensions · React Web · Rust CLI · Recipkg reconstruction contracts</p>

  [![Recipe](https://img.shields.io/badge/recipe-0.1.11-111111?style=flat-square)](recipes/lmm-api/RECIPE.md)
  [![Source](https://img.shields.io/badge/source-7abe06d5-111111?style=flat-square)](https://github.com/TokenNotIncluded/lmm-api-source/tree/7abe06d5d369b2ccefe4364099450bfae4534953)
  [![Status](https://img.shields.io/badge/production%20switch-not%20proven-111111?style=flat-square)](STATUS.md)
  [![License](https://img.shields.io/badge/license-AGPL--3.0--or--later-111111?style=flat-square)](LICENSE)

  [English](README_EN.md) · [Rebuild](#从配方开始) · [Source](#固定实现源码) · [Acceptance](recipes/lmm-api/ACCEPTANCE.md) · [Status](STATUS.md)
</div>

> [!WARNING]
> 这个仓库现在是 **LMM API 的 Recipkg 配方、接口合同与验收主项目**。可编译实现已经迁到独立的 [`TokenNotIncluded/lmm-api-source`](https://github.com/TokenNotIncluded/lmm-api-source)，并以固定 Git 子模块接入。配方检查通过不等于软件已经完整重建，更不等于新架构已经可以切换生产。

## 一句话理解

LMM 是 Language Model Mesh：用户通过一个统一入口管理身份、团队、权限、余额、预算和模型调用；商店、助手、工具市场、运营与支付适配则放在可插拔扩展中。

```text
CLI / OpenAI-compatible API / Web console
                    │
          ┌─────────┴─────────┐
          ▼                   ▼
   lmm-core · Rust     lmm-extensions · Go
身份 · 权限 · 账本      商店 · 工具 · 助手
预算 · 订阅 · 路由      运营 · 支付商 I/O
          │                   │
          └──── 明确授权合同 ──┘
```

关键原则很简单：**钱和权限只有 Core 一个主人。** 扩展拥有自己的业务状态，但不能直接改核心余额、账本或核心数据库。

## 先分清三个层次

| 层次 | 在哪里 | 它证明什么 |
| --- | --- | --- |
| **产品合同** | 本仓库 `recipes/` | 需求、边界、接口、失败行为和验收条件被明确记录 |
| **固定实现** | `generated/lmm-api-source/` | 当前合同对应一个可追溯的源码提交 |
| **生产事实** | 独立部署与验收证据 | 真实数据、供应商、支付、浏览器、故障恢复和切换流程实际成立 |

```text
recipe check passed
        ≠
source builds
        ≠
integration accepted
        ≠
production can switch
```

当前状态明确停在最后一个等号之前：已有大量开发者自测和隔离实验，但尚不能证明可切换生产。完整证据与未完成项见 [STATUS.md](STATUS.md)。

## 六个配方包

| 配方 | 本质职责 | 对应实现 |
| --- | --- | --- |
| [`lmm-api`](recipes/lmm-api/) | 集成入口和完整产品验收 | 整体分发与部署组合 |
| [`lmm-core`](recipes/lmm-core/) | 身份、团队、权限、账本、预算、订阅、路由与转发 | `services/core`，Rust |
| [`lmm-extensions`](recipes/lmm-extensions/) | 商店、工具、助手、运营和支付适配 | `services/extensions`，Go |
| [`lmm-web`](recipes/lmm-web/) | 静态控制台、认证、钱包、使用记录与管理界面 | `apps/web`，React/TypeScript |
| [`lmm-cli`](recipes/lmm-cli/) | 用户与运维命令行合同 | `apps/cli`，Rust |
| [`lmm-contracts`](recipes/lmm-contracts/) | 跨进程 RPC、HTTP、事件、配置和兼容性合同 | 共享叶子依赖 |

主配方依赖 Core、Extensions、Web 和 CLI；各组件再依赖 Contracts。运行时只有三个产品程序：`lmm-core`、`lmm-extensions` 和 `lmm` CLI。Web 是静态产物，不是第四个常驻后端。

## 从配方开始

安装 [Recipkg](https://github.com/recipkg/recipkg) 后：

```bash
# 检查所有引用、依赖和配方结构
recipkg check recipes/lmm-api

# 生成完整中文重建任务
recipkg build recipes/lmm-api --lang zh-CN
```

打包并复核一个可交付的配方归档：

```bash
set -eu
out="$(mktemp -d)"
trap 'rm -rf "$out"' EXIT

recipkg pack recipes/lmm-api \
  -o "$out/lmm-api.recipe.tar.gz"
recipkg check "$out/lmm-api.recipe.tar.gz"
```

`recipkg build` 生成的是实现任务，不会调用模型、编译代码、启动 PostgreSQL 或验证生产数据。真正实现必须逐项执行 [`ACCEPTANCE.md`](recipes/lmm-api/ACCEPTANCE.md) 和 [`PRODUCTION_ACCEPTANCE.md`](recipes/lmm-api/PRODUCTION_ACCEPTANCE.md)。

## 固定实现源码

当前主仓库固定：

```text
TokenNotIncluded/lmm-api-source
7abe06d5d369b2ccefe4364099450bfae4534953
```

获取完整源码及嵌套子模块：

```bash
git clone --recurse-submodules \
  https://github.com/TokenNotIncluded/lmm-api.git
cd lmm-api

git submodule update \
  --init \
  --recursive \
  -- generated/lmm-api-source
```

进入实现仓库后，以其固定的 `mise.toml`、`justfile` 和开发文档为准：

```bash
cd generated/lmm-api-source
mise install
mise exec -- just --list
```

不要把主仓库工作区里未提交的源码、其他分支或本地数据库实验算进这个固定快照。子模块提交是发布边界。

## 实现架构

```text
                         ┌──────────────────────┐
                         │ PostgreSQL · Core    │
                         │ 身份 / 账本 / 路由   │
                         └──────────▲───────────┘
                                    │
client ── HTTP/OpenAI API ──► lmm-core ◄── provider APIs
   │                                ▲
   │                                │ 授权内部 RPC
   ├── static web console ──────────┤
   │                                │
   └────────────────────► lmm-extensions
                                    │
                         ┌──────────▼───────────┐
                         │ PostgreSQL · Ext     │
                         │ 商店 / 支付 / 助手   │
                         └──────────────────────┘
```

- Core 与 Extensions 使用独立数据库和独立运行身份。
- 普通启动不得创建、升级或偷偷修复数据库结构。
- 初始化、检查和升级是显式命令，安装 owner 与运行凭据分离。
- Extensions 失败不得破坏模型转发、核心余额或账本一致性。
- Web 只能通过公开/授权接口工作，不能直接接触数据库。
- 健康接口有响应不代表模型供应商、账本、支付或浏览器业务已可用。

实现细节从 [固定源码 README](https://github.com/TokenNotIncluded/lmm-api-source/blob/7abe06d5d369b2ccefe4364099450bfae4534953/README.md) 开始。

## 当前已验证到哪里

截至 `2026-10-30` 的固定状态包括：

- 主配方 `0.1.11` 与固定源码 `7abe06d5…`；
- Rust Core、Go Extensions、React Web 和 RPC 合同的多组定向测试；
- 隔离 PostgreSQL 升级、账本/支付幂等、部分浏览器财务页面和 nginx 路由夹具；
- Stripe、EPay、Creem、Waffo Global 和 Waffo Pancake 钱包充值协议的开发者实现与局部验证；
- FastPay 明确退役，配置与 provider 路径拒绝继续启用；
- Recipkg 递归检查、临时归档检查和完整提示词渲染。

这些证据证明特定代码路径在特定隔离条件下成立。它们没有证明：

- 所有真实商户账号和真实支付回调；
- 完整浏览器、Core、Extensions 和供应商联合验收；
- 历史身份、资金、订阅、API Key、日志和凭据迁移全部对账；
- 所有 writer 已停写且故障恢复达到生产要求；
- 生产开放、回滚与灰度切换已完成。

所以项目不发布一个误导性的“总体完成百分比”。分母与依赖关键路径在 [`COMPLETION_SCOPE.md`](recipes/lmm-api/COMPLETION_SCOPE.md) 和 [`PRODUCTION_ACCEPTANCE.md`](recipes/lmm-api/PRODUCTION_ACCEPTANCE.md) 中逐项保存。

## 生产切换门槛

生产候选必须同时回答五类问题：

```text
1. 运行的源码、二进制、SBOM 和 schema 是否属于同一提交？
2. 旧数据是否完整迁移、对账且能证明来源？
3. 所有写入者是否停写，旧会话是否排空？
4. 模型、资金、支付、身份、浏览器和边缘路由是否联合通过？
5. 灰度、回滚和失败恢复是否在同一候选上验证？
```

缺一项就只能标记为 `BLOCKED` / `UNKNOWN`，不能靠“测试很多”推导为可上线。

权威入口：

| 目标 | 文档 |
| --- | --- |
| 完整产品验收 | [ACCEPTANCE.md](recipes/lmm-api/ACCEPTANCE.md) |
| 生产切换 | [PRODUCTION_ACCEPTANCE.md](recipes/lmm-api/PRODUCTION_ACCEPTANCE.md) |
| 全范围分母 | [COMPLETION_SCOPE.md](recipes/lmm-api/COMPLETION_SCOPE.md) |
| 支付对照 | [PAYMENT_PARITY.md](recipes/lmm-api/PAYMENT_PARITY.md) |
| 问题与维护合同 | [ISSUE_MAINTENANCE.md](recipes/lmm-api/ISSUE_MAINTENANCE.md) |
| 当前证据与限制 | [STATUS.md](STATUS.md) |

## 仓库地图

```text
recipes/                    产品合同与验收
  lmm-api/                  集成入口
  lmm-core/                 Rust 核心
  lmm-extensions/           Go 扩展
  lmm-web/                  Web 控制台
  lmm-cli/                  Rust 客户端
  lmm-contracts/            共享接口合同
generated/lmm-api-source/   固定实现 Git 子模块
.agents/skills/             固定上游与项目技能
STATUS.md                   当前证据、失败和剩余工作
MIGRATION.md                从旧单体迁移到配方/源码双仓结构
```

共享第三方库、操作系统、PostgreSQL、模型供应商、支付商、密钥库和外部账号是环境前提，不会被伪装成配方内部依赖。

## 分支与历史

- `main`：当前 Recipkg 主项目与固定源码指针。
- `traditional` / `传统`：旧 Go 单体历史。
- 原实现迁出基准：`ff01f8456b978089bce08175cbd357c3d22c3983`。

迁移仓库结构不会自动迁移生产数据库，也不会让旧服务切换到新程序。线上 [`api.lmm.best`](https://api.lmm.best) 的运行状态与本仓库开发进度必须分别核验。

## 贡献与安全

修改行为前先更新相应配方、合同和验收条件，再修改固定实现。合并、打包、发布和部署是四个独立决定。

不要把 API Key、支付密钥、用户数据、生产数据库、私有日志或真实商户回调上传到 Issue、PR、配方归档或模型提示词。

## License and attribution

LMM API 使用 [AGPL-3.0-or-later](LICENSE)。保留 [NOTICE](NOTICE)、[第三方许可证](THIRD-PARTY-LICENSES.md)、QuantumNous/new-api 与所有贡献者归属。
