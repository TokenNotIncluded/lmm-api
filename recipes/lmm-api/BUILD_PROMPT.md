# 重建 LMM API

面向个人、团队和运营者构建 Language Model Mesh：统一身份、权限、资金和模型入口，提供 Web 控制台、`lmm` 客户端及独立业务扩展。保留项目名 `lmm-api` 与品牌「LMM API · 连接模型与工具，让智能自由流动」。

完整范围读 [PRODUCT_REQUIREMENTS.md](PRODUCT_REQUIREMENTS.md)，细化接口消费递归组件配方；验收读 [ACCEPTANCE.md](ACCEPTANCE.md) 与 [PRODUCTION_ACCEPTANCE.md](PRODUCTION_ACCEPTANCE.md)。当前工作只维护[源码进度列表](https://github.com/TokenNotIncluded/lmm-api-source/blob/main/docs/progress.md)，构建提示词不保存已完成/未完成快照。

## 依赖与装配

| 依赖 | 职责 | 集成条件 |
| --- | --- | --- |
| core | Rust 身份、权限、资金、路由与模型请求；离线安装命令、内部 RPC | 无 Go 可独立运行；所有模型授权与计费归 Core |
| extensions | Go 商店、客服、工具、助手、运营与支付外部 IO | 独立业务库、窄 RPC，无 Core 数据库凭据 |
| web | React/TypeScript 静态控制台 | 与真实 Core/扩展联合，关闭能力明确不可用 |
| cli | Rust 用户客户端和授权软件接入 | 系统凭据库、PKCE、JSON/非交互、可审阅动作及准确阻塞 |

各组件依赖完整 contracts，统一类型、权限、单位、版本、幂等与错误。按合同依赖装配并冻结同一最终输入；隔离测试与最终联合验收分别记录。

仅三个产品程序 `lmm-core`、`lmm-extensions`、`lmm`；Web 为静态产物。Rust Core 单 Cargo 项目、Go 模块同一宿主，保留主要语言与职责。库使用经审阅且锁定的兼容版本，核对许可证；不额外制造产品守护进程。

## 产品范围

身份/团队/开发者权限、认证与凭据撤销、唯一账本/预算/订阅、模型矩阵和流式恢复、商店/订单/客服/运营、助手/工具/MCP、安全委托、公开接口、Web 与 CLI 按产品要求逐行为交付。旧路径消失或新能力关闭不能删减要求；客户端不能自行决定付款方或权限。

支付细节按 [PAYMENT_PARITY.md](PAYMENT_PARITY.md)：五类旧 provider、七个钱包预设、Linux DO LDC，分别完成钱包、订阅、商店、退款和配置管理。FastPay 退役，历史标记/拒绝 fixtures 保留。新增 Coinbase Business 钱包按组件合同实现 USD 报价、同额 USDC、独立验签与一次入账，默认关闭、自动退款关闭；不混入旧 provider 恢复矩阵。

Extore 以冻结商户、版本、文件和原操作完成授权/同步/撤销与真实两端验收；文档更新不授权推送其自动部署分支。真实供应商、历史身份/资金/Key/订阅迁移、全页面浏览器及生产切换分别证明，不用合成上游、健康 200 或配方结构抵扣。

## 环境与运行边界

服务器目标 Linux amd64/arm64，CLI 目标 Linux/macOS/Windows amd64/arm64；最低浏览器及各平台支持以实际验收明确。PostgreSQL、OS、浏览器、TLS、凭据库、反向代理和外部账号为前提，不伪装成隐藏配方依赖。已冻结升级合同中的 PostgreSQL 版本和前驱约束继续有效。

工具链和最低版本读取固定源码的声明与锁文件，并核实际子进程版本；不在入口复制易过期的开发版本表。运行配置 TOML，owner/runtime 分离，普通启动不建库、不 DDL、不自动迁移。新装、首任 owner 创建、既有库升级使用显式 Core 子命令；真实源只读、隔离目标、生产与付费操作按用户授权范围执行。

Web 按 web 配方提供简洁、留白、清晰层级，保留简繁中文、英语、法语、俄语、日语、越南语及准确时区/单位/错误。Core 独立、扩展停机、数据库故障、并发 SSE、慢客户端和大账本分页按生产验收合同的资源硬门槛及新旧对照执行。

## 交付与反馈

实现放入独立源码仓库，以可达提交的 gitlink 接入 `generated/lmm-api-source/`；CLI/providers 保持嵌套归属。先提交/推送并核远端，再更新父指针；配方不夹带源码。自测、独立复验、候选、安装、发布、部署各有自己的证据。

[ISSUE_MAINTENANCE.md](ISSUE_MAINTENANCE.md) 保留问题单与旧候选的具体行为合同。旧 PR 迁移或关闭不代替其行为验收；修复须关联源码、反例和固定输入。

质量快照分别绑定暂存模式/blob、工作区字节、未跟踪项及子模块，扫描基线到暂存区与工作区。部分暂存不能漏报，测试中只改暂存区也使证据失效。完整计划保留 Mocklab selftest 原始报告；失败、空集合、skip 和环境阻断不记成功，组件通过不替代真实业务。

## 站点配置合同

首页、公开页头与重复入口共享有效品牌、公开 API 地址及 HeaderNavModules。管理员保存后刷新与多视口实际生效，保留默认兼容、原权限和七语言，导航隐藏不取代授权。

Core 只向最高管理员 Session 开放 HeaderNavModules 写入，并声明可写能力。保留未知扩展字段，JSON 对象限 64 KiB；同事务核版本、批量 expected_values、审计与冲突。公开状态读取保存的导航和启动配置的公开地址，设置接口不能覆盖主机地址/OAuth issuer。数据库支持以追加组件和显式离线升级交付，不改旧 SQL/函数；验证数据/权限保留、精确重放、受限 runtime、并发及审计失败回滚。
