# lmm Rust 客户端

独立 Rust CLI，可执行名 lmm（Windows 为 lmm.exe），crate 名 lmm-cli。contracts 定义服务端身份、余额/价格单位和错误；客户端不裁定资金权限。目标 Linux/macOS/Windows amd64/arm64；Rust 文档最低 1.88，实际平台构建与系统凭据库需独立验收。Linux 桌面 OAuth 需要运行中 Secret Service，Windows/macOS 用系统凭据库；OS、浏览器、凭据库、目标应用和 Node 是外部前提。

离线 presets [关键词] [--json] 查询内置 56 项服务商预设；不得读密钥或联网。catalog 搜索有明确项目身份的软件，区分安装/接入/维护能力。status 只读检测当前 OS/架构、WSL/容器/SSH 线索、PATH 候选和显式实例；doctor --report 给脱敏摘要，移除实例路径且不读配置内容。

setup <目标> --dry-run 生成可审阅预览和阻塞原因；终端下无目标时可选择。--json 与 --non-interactive 禁止等待交互；非法/不支持动作必须明确失败，不做空操作成功。未实现的安装/同步/更新/恢复/解除/卸载/启动保持真实 blocked，不能因配方出现变成可用。

login 使用桌面浏览器 PKCE，仅将凭据放系统凭据库；绑定 issuer，避免 token 输出。models [--json] 只读当前授权模型与价格，不发模型请求或改应用配置。balance [--json] 读取余额 V2，保留整数积分/美元单位；失败或未知单位不能输出 0。logout 撤销当前 issuer 的 CLI token family，服务端成功后才删除本地凭据，不影响 Pi/DSH 等其他应用。

已定义的限定接入适配合同为 Linux、官方 Pi 1.2.0、显式项目实例，管理该项目 LMM Provider，不安装或卸载 Pi 本体，也不复制 CLI 凭据。固定 Provider 观察参考为 TokenNotIncluded/pi-lmm-provider@f0c54764eb3d73e50790346f0dbab834587fe27b；这是外部实现/测试参考，不是未提炼的配方依赖。本配方的重建范围仅为 CLI 适配与其 dry-run/失败边界，不要求重建 Pi 或 Provider。真实接入须独立验证；若将 Provider 纳入交付，先提炼完整配方和验收，不能只拉源码当依赖完成。

CLI 不包装 Magpie、不依赖 Go runtime。用户配置遵循 XDG 的 lmm/config.toml、state/lmm、cache/lmm；具体凭据实现以系统库为准。输出说明对象、动作、错误和下一步；JSON 保持机器可读，诊断不泄露路径/秘密。保留未完成完整产品要求为后续目标，不能把预览版声明当所有软件适配已完成。

## 独立授权的应用接入

实现 contracts 依赖中的[独立应用授权与轮换恢复](../lmm-contracts/MANAGED_APPLICATIONS.md)。cc-switch 使用其独立安装身份与有限模型授权，经过真实浏览器 PKCE 同意后持久保存在系统凭据库；接入、sync 和配置恢复保留用户选择且不复制只读 CLI/Pi token。请求前持久化 Refreshing 状态，响应或凭据保存丢失只恢复同一 rotation；实时状态必须来自 Core。仅在相应服务端能力完整、正常 TLS 和目标原生实例均有效时执行；缺少任一前提明确 blocked，不伪造支持。官方安装及配置回执恢复与应用授权/sync 分别验收，真实供应商和平台验收另列。
