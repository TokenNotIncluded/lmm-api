# 当前状态 · 2026-11-07

实现版本以本仓库 gitlink 为准。当前源码是 [lmm-api-source 9250ef0](https://github.com/TokenNotIncluded/lmm-api-source/commit/9250ef01399afc5cedde8413b0e897ed4a0a0503)。生产没有切换。

## 本轮 OAuth / 广告推进

- 自 `06ca6ac` 编译 Core56 调试二进制（SHA-256 `2c50ebf1…b3d480`）。库单测 `consent_csp_allows_only_the_current_validated_callback` 通过。
- 隔离 PostgreSQL 上对活 Core 做 issuer Host 的 HTTP 表单旅程：consent CSP 为 `form-action 'self'` 加精确回调；Allow 换到 code 并完成 token；Deny 回 `access_denied`。证据在本机 `~/.local/state/lmm-api/oauth-core-20261107/evidence/http-oauth-results.json`。这是开发自测，不是独立浏览器验收。
- Chromium 完整 Allow/Deny 未跑完：本机可用内存约 300MiB 时启动 Chromium 会挤掉 Core。无 Core 的 CSP 形状核对仍在 `~/.local/state/lmm-api/oauth-csp-20261107/result.json`。
- 广告丢响应 PostgreSQL 故障注入仍是开发自测；独立签收和浏览器广告闭环未做。
- 已退役误导性的 `GET /api/ai-directory/ads/quote`（410 `AI_DIRECTORY_AD_QUOTE_RETIRED`），正式路径仍是 `POST .../prepare`。
- 商店可选 `store.stock_key_file`（私密原始 32 字节）已从 extensions 配置接入 `WithStockKey`；`PublicConfig.stock_items_supported` 随密封密钥就绪状态广告，前端在未就绪时不提供库存导入。Extore 商店绑定仍在独立候选分支，未并入微服务宿主。
- 本机仍无旧站迁移源库。真实迁移、资金零和、Core56 全业务验收、生产隔离安装和切流都没有执行。

验收合同仍是 [PRODUCTION_ACCEPTANCE.md](recipes/lmm-api/PRODUCTION_ACCEPTANCE.md) 与 [COMPLETION_SCOPE.md](recipes/lmm-api/COMPLETION_SCOPE.md)。
