# Recipkg 主项目迁移 · 2026-10-28

## 最终归属

| 内容 | 仓库 / 路径 |
| --- | --- |
| 原名与主仓库 | TokenNotIncluded/lmm-api，main；当前目录即配方项目 |
| 需求、接口与验收 | recipes/lmm-api 及其五个递归组件/合同包 |
| 实现源码 | https://github.com/TokenNotIncluded/lmm-api-source |
| 固定源码指针 | generated/lmm-api-source @ 06d8b4b7c9630106c6ef47425f9e67d333bd1548 |
| CLI 与 Providers | 原固定子模块保留在源码仓库 apps/cli 与 packages/* |
| 共享技能 | 当前主仓库 .agents/skills/ 的 11 个原有上游子模块，固定提交不变 |
| 原自定义技能 | 当前主仓库保留原文件；源码仓库有完整副本，原相对源码/文档路径在源码仓库中继续有效 |

实现由原主仓库 ff01f8456b978089bce08175cbd357c3d22c3983 的已跟踪树导出成新源码仓库初始快照。仅 AGENTS.md、README.md 与新增 SOURCE_ORIGIN.md 有迁移说明变化，业务实现文件与迁出基准完全相同。原提交历史保留在主仓库现有历史中，没有重写或强推；旧单体分支仍为 传统 与 traditional。

原工作区服务、应用、脚本、契约、配置、部署/打包、测试、示例与其实际本地目录已移动到源码子模块；根不再保留平铺实现源码、根构建文件或旧源码子模块指针。需要构建时进入 generated/lmm-api-source，原 mise.toml、justfile、package.json 路径在该源码根不变。构建缓存/本地产物随目录移动，没有上传；旧绝对构建缓存路径不代表新位置可直接使用，实际应用构建未重跑。

15 项原有 dirty/删除/未跟踪路径迁移前后按内容 SHA256/存在状态核对，全部保留在源码工作区，没有提交到任一远端。迁移期间出现的其他未跟踪文件也留在其原目录中。原 CLI 与七个 Provider 的 Git 元数据、工作树位置和远端已核对修复；源码和根技能子模块各自保持独立所有权。

## 实际自测

| 检查 | 结果 |
| --- | --- |
| 源码与迁出基准比较 | 只改变迁移入口文档，业务实现未改 |
| 原 15 项未提交内容 | 哈希/删除状态一致 |
| 主仓库布局 | 根无 apps/services/scripts/contracts 等旧源码平铺；generated 只有命名源码子模块 |
| .gitmodules / index | 11 个原共享技能 + 1 个源码，均为正确远端、SHA 与 mode 160000 |
| 源码 CLI / Providers | 原提交和远端保持；路径修复后可正常读取 Git 状态 |
| 源码文档检查 | python3 -B scripts/check-docs-brand.py --docs-only：PASS |
| recipkg check recipes/lmm-api | PASS |
| 临时 pack / 归档 check | PASS，62 个 UTF-8 Markdown 归档文件，无源码/二进制/软链接/执行钩子 |
| 完整 build 提示词 | PASS，466194 UTF-8 字节，含全部六个逻辑配方与固定源码身份 |
| 临时归档摘要 | 7b442c15b2ea63189301550d523f528e31188e755e34e3ae9f6a313f714a7605；临时目录已删除，不保留交付归档 |
| 新源码远端回读 | main 为 06d8b4b7c9630106c6ef47425f9e67d333bd1548 |
| GitHub 发现 | 原主仓库已添加 recipkg topic 并回读，保留原 topic |

本轮是仓库迁移，自测通过；独立业务验收未执行。没有重新构建应用或运行数据库、浏览器、真实支付/模型、旧数据资金对账、性能或生产切换。原失败和待验收边界继续记录在配方及源码文档，不能以迁移成功代替产品完成。

新源码仓库 Actions 已显式关闭，不触发 CI/部署；迁移提交使用 skip ci。GitHub 默认分支、配方 topic 与源码提交分别回读，不以本地状态推断远端结果。没有访问或操作任何生产服务器。
