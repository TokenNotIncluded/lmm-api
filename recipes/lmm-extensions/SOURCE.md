## 来源与证据边界

观察日期：2026-10-28。原实现仓库：https://github.com/TokenNotIncluded/lmm-api ，行为观察提交 `d468affecfd76a5123358732fb9a4e0cc1f63566`，迁出基准 `ff01f8456b978089bce08175cbd357c3d22c3983`；CLI 固定为 https://github.com/TokenNotIncluded/lmm 的 `23de1322d68a021707eda876ed477e71d72f9544`。实现已迁到 https://github.com/TokenNotIncluded/lmm-api-source ，初次迁出提交 `06d8b4b7c9630106c6ef47425f9e67d333bd1548`；当前实现版本以主仓库 gitlink 和 STATUS.md 为准，由当前主仓库 `generated/lmm-api-source/` 子模块接入。它不是配方依赖，不进入归档。

初次提取配方时只做源码、文档及已有测试定义的静态观察，未重跑应用、数据库、浏览器、支付或性能验收；后续修复和验证按版本单独记录。源码存在、测试定义、历史通过说明均不代表固定版本已通过验收。原开发工作区未提交内容全部排除；包括 Passkey、模型预检、共享读配额、价格修改和旧单体回迁准备脚本，不能将其当成已发布功能。

旧单体 `传统` 与 `traditional` 固定于 `3948a0a1ee1cefdd016a7aa45a11851545bb4b85`；当前微服务默认分支为 `main`。固定源码中仍有旧分支名的文档和工作流引用，应按提交识别观察对象，不能按旧文案取错源码。

项目基于 QuantumNous/new-api，保留上游及其他贡献者归属、AGPL-3.0-or-later、NOTICE 和第三方许可证。目标不要求复制实现；若复用代码、素材、字体或其他受保护材料，逐项保留许可证和归属。商业许可状态未知，不据此扩大授权。

## 0.1.1：2026-10-30 五类支付适配与原生钱包宿主

新增 Creem 固定单次产品、Waffo Global、Waffo Pancake 适配；EPay 通过 Core 签名，不把商户 MD5 密钥交给 Go。Waffo 官方 SDK 固定为 `waffo-go/v2 v2.1.0` 与 `waffo-pancake-sdk-go v0.17.0`，许可证归属保留。v2 配置逐渠道定义协议／商户／环境，宿主挂载钱包目录、报价、确认和状态接口，回调保留原始签名及 GET／POST 传输。LinuxDO 明确使用 POST；新增四协议的自动退款保持未支持，FastPay 配置拒绝。

实现来源为 [lmm-api-source `7abe06d`](https://github.com/TokenNotIncluded/lmm-api-source/commit/7abe06d5d369b2ccefe4364099450bfae4534953)，详见 [STATUS.md](../../STATUS.md)；对应源码的 `docs/payment-gateways.md` 与支付模块 `VERIFICATION.md` 记录实际检查、保留失败和未运行范围。开发自测不等于独立验收或生产切换；本轮未部署、未访问真实支付账户或产生真实费用。
