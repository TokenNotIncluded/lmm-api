# LMM API

![软件与依赖汇聚为可重建的配方](https://raw.githubusercontent.com/recipkg/recipkg/main/assets/recipkg-cover.png)

LMM API · Language Model Mesh · 连接模型与工具，让智能自由流动。

本仓库已作为 **Recipkg 配方主项目**：需求、行为、接口与验收条件在 `recipes/`；实现代码由独立的 [TokenNotIncluded/lmm-api-source](https://github.com/TokenNotIncluded/lmm-api-source) 维护，固定接入 `generated/lmm-api-source/`。

| 内容 | 入口 |
| --- | --- |
| 重建主配方 | [recipes/lmm-api/BUILD_PROMPT.md](recipes/lmm-api/BUILD_PROMPT.md) |
| 验收要求 | [recipes/lmm-api/ACCEPTANCE.md](recipes/lmm-api/ACCEPTANCE.md) |
| 来源与未知项 | [recipes/lmm-api/SOURCE.md](recipes/lmm-api/SOURCE.md) |
| 实现源码与构建 | [generated/lmm-api-source/README.md](generated/lmm-api-source/README.md) |
| 生产切换验收合同 | [recipes/lmm-api/PRODUCTION_ACCEPTANCE.md](recipes/lmm-api/PRODUCTION_ACCEPTANCE.md) |
| 当前推进状态 | [STATUS.md](STATUS.md) |
| 迁移记录 | [MIGRATION.md](MIGRATION.md) |

## 配方与实现

六个纯 Markdown 配方：`lmm-api` 集成、`lmm-core` Rust 核心、`lmm-extensions` Go 扩展、`lmm-web` React/TypeScript 静态控制台、`lmm-cli` Rust 客户端和 `lmm-contracts` 接口合同。主配方依赖四组件，各组件依赖完整的 contracts 叶子。只有三个产品程序，Web 是静态产物。

```sh
recipkg check recipes/lmm-api
recipkg build recipes/lmm-api --lang zh-CN

# 需要开发实现时才初始化源码及其嵌套子模块
git submodule update --init --recursive -- generated/lmm-api-source
cd generated/lmm-api-source
# 按源码仓库的 mise.toml 和 justfile 执行目标构建/测试
```

当前固定实现提交：`29e82295a80ca29747abd2c1b6490c7c01dc7315`。原实现迁出基准为 `ff01f8456b978089bce08175cbd357c3d22c3983`，原历史仍在本仓库。旧单体保留在 `传统` 与 `traditional` 分支；新开发主分支为 `main`。

## 状态

此迁移只改变仓库组织，不代表软件已经重建、完整业务验收或生产切换。真实供应商、支付、旧数据迁移对账和浏览器业务等未完成项继续保留在配方与源码文档。未提交实现工作保留在本地源码子模块，不计入发布快照。

共享技能保留在 `.agents/skills/` 的 11 个上游固定子模块，原自定义技能保留；实现任务使用源码子模块内自定义技能的相对路径。第三方库、PostgreSQL、OS、系统凭据库、模型和支付账号是外部前提，不冒充配方依赖。

AGPL-3.0-or-later；保留 [LICENSE](LICENSE)、[NOTICE](NOTICE)、[第三方许可证](THIRD-PARTY-LICENSES.md) 及 QuantumNous/new-api 和贡献者归属。
