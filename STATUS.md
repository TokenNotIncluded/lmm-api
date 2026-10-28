# 当前推进状态 · 2026-10-28

目标：迭代到完整可用，并证明生产环境具备切换到新架构的条件。**整体尚未完成，当前不能证明可切换生产。** 完整分母保留在 [原始完成范围](recipes/lmm-api/COMPLETION_SCOPE.md) 与 [生产验收合同](recipes/lmm-api/PRODUCTION_ACCEPTANCE.md)，不按测试数量估计完成百分比。

本轮源码提交为 `095c5f8f0fe5d7a7bf158941b07eb3df75ddee5d`，通过 `generated/lmm-api-source/` 固定。原有未提交工作仍在子模块，未混入本轮提交或当成最终候选。

## 本轮已修复并独立复验

| 问题 | 修复后结果 | 实际证据范围 |
| --- | --- | --- |
| 扩展宿主凭据 absent 却能 safe_to_cutover=true | 五个扩展业务门禁全部 BLOCKED；reviewed 对照保留原 READY | 独立实际 Python CLI 回归；preflight/身份构建器/本地回环 edge 合成测试 |
| 普通财务计划在 Key 限制元数据夹带 bearer | 规划阶段拒绝所有 Key 的已知 bearer，异常及 stdout/stderr 不泄露，不发布计划 | 独立同 Key/跨 Key/嵌套反例及真实普通 CLI 调用；原始冻结私有来源归档单独保留 |
| 源和目标同样非法的 Key 限制类型误报 matched | 限制字段不再匹配，整体对账失败，仍禁止激活 | 独立九类字段域反例与既有 typed-fact 对账测试；不证明目标清单真实来源与完整性 |

## 本轮真实 PostgreSQL 验收

测试方在全新 Unix socket、无 TCP 的 PostgreSQL18 和合成用户 1–4 上执行钱包及历史原始计数器 CLI。预览/审批/实际导入通过；正常重放、真实 COMMIT 成功但外部 psql 确认丢失后的原操作查询恢复，均核对 160 张非系统表和 21 个序列的快照，未发生重复变更。没有用成功 mock 替换业务 SQL；测试自己的集群已停止。

只覆盖合成开账与原始计数器 profile。二进制 schema-manifest 与当前安装 schema 匹配只证明 schema 身份，不证明完整运行二进制来自当前最终组合源码。外部 psql 回执丢弃不等于数据库网络层断连。真实历史资金、订阅、Key、持续托管权利及其他业务仍须独立组合迁移和对账。

当前工作树（含保留的未提交 Rust 工作）执行了 Rust1.99 cargo check --locked --all-targets，通过，输入文件哈希前后一致；这是编译检查，不是测试、业务通过或已提交候选的构建证明。

## 仍阻断完整目标

- 设置写入当前未提交 patch 调用 lock_public_options/write_public_option，但安装 schema 没有这两个函数；新建隔离 PostgreSQL 已复现 42883。需要补齐与受限 runtime 匹配的安全写入和升级合同，并用真实 HTTP/数据库回读验收。
- A01/A02 完整权限/登录/旧凭证/特权批准，A03 全部历史经济权利及单位/预留依据，A04 全量日志/用量/财务，A05–A08 真实支付/商店/供应商/助手工具和 A10 全页面/CLI 联合验收，尚无本轮最终组合证据。
- A09/S11 最终源码与制品/进程绑定、授权只读来源与目标、最终写入隔离及 drain、备份实际恢复、1CPU 限额资源验收、生产环境自身条件和切换演练，尚未在本轮证明。
- 预检 READY 仍只是受信任输入断言的一致性，不能把合格形状的 hash 或手填 passed 当生产 go/no-go。

## 证据与限制

完整命令、冻结输入 SHA256、原始失败、开发自测和独立复验分别保存在本机私有 `~/.local/state/lmm-api-production-readiness-20261028/`；合成 PostgreSQL 回执和 dump 在 `~/Backups/lmm-prod-20261028/opening/`。没有将私密材料放入配方或 Git。

保留了首次 CLI 缺必需参数、直接使用 Backups 根父目录被路径检查拒绝的失败；后者未创建数据库，修正为新的私有嵌套输出后独立执行一次通过。它们不被计为产品通过。

本轮没有运行 CI、访问 DMIT 或其他生产服务器、操作真实资金、创建生产测试账号、部署或切换流量。目标保持 active；局部复验通过不代替 A01–A10/S11 的完整完成审计。
