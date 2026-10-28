# CLI 验收要求

- 离线禁网运行 presets/catalog/status/doctor，预设 56 项且可筛选，错误/不支持目标明确非零退出；doctor 摘要不含敏感路径和密钥。
- --json、--non-interactive 在无终端及有终端环境都不等待输入；JSON 结构稳定，stdout 不混普通日志。
- setup --dry-run 不写配置、不装应用、不读秘密；阻塞原因可定位，不把空动作显示成功。
- 合成 issuer/测试凭据库验证 PKCE、错误 state、取消、token 撤销与 issuer 隔离；失败不得删除仍需恢复的本地凭据。
- models/balance 对 401/403、网络错误、未知币种与单位、服务不可用准确失败；模型查询不触发付费模型调用，余额失败不显示 0。
- Linux/Pi 1.2.0 的 add-only 必须针对显式实例和 Node，预览后只修改授权 Provider 范围，不安装 Pi、不复制 CLI token。真实 Pi 测试本轮未执行。
- 各平台分别记录构建/凭据库/OAuth 与行为结果，不从支持表推断已验收。

所有执行项本轮 NOT RUN。未测启动延迟、包体积和系统库性能。
