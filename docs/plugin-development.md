# 扩展插件开发与管理

扩展插件属于 `lmm-extensions` 宿主。插件代码是经过审查后编译进宿主的 Go 模块，不能从浏览器或管理员请求上传 `.so`、脚本或任意二进制。这样保留了 Rust Core 的身份、权限和资金边界，也避免把不受信任代码放进同一进程。

## 插件契约

插件实现 `services/extensions/internal/modules.Module`：

```go
type Module interface {
    Name() string
    Handler() http.Handler
}
```

`Name()` 只能使用小写字母、数字和短横线，长度最多 32；服务令牌只负责进入宿主，插件仍须用 Core RPC 核验用户会话和权限。模块不能拿 Core 数据库连接、全局 ORM 或共享余额写入器。每个模块有独立的路由前缀和并发上限。

如果插件提供 `Version() string`，宿主会把它纳入注册摘要；未提供时使用 `1.0.0`。注册摘要是：

```text
sha256("lmm-extension-registration-v1\0" + name + "\0" + version)
```

它绑定管理员请求和当前编译版本，不能替代发布包签名或代码审查。新插件必须在源码中注册、通过 Go 测试和独立构建验收后，随新的 `lmm-extensions` 包发布。

## 管理 API

这些接口要求一次性的 Core session Bearer 凭据，并且 `platform_level` 为 5 或 6；宿主服务令牌和用户自带的身份头都不具备管理员权限。

| 方法 | 路径 | 作用 |
| --- | --- | --- |
| `GET` | `/api/extensions/plugins` | 查看当前编译注册、安装和启用状态 |
| `POST` | `/api/extensions/plugins/install` | 安装一个与当前注册摘要完全匹配的 manifest |
| `POST` | `/api/extensions/plugins/{name}/enable` | 启用插件路由 |
| `POST` | `/api/extensions/plugins/{name}/disable` | 禁止新的插件请求 |
| `DELETE` | `/api/extensions/plugins/{name}` | 删除安装状态；必须先禁用 |
| `POST` | `/api/extensions/plugins/reload` | 重新应用当前启用快照 |

安装请求只含 `name`、`version`、`api_version` 和 `registration_sha256`。启停和重载是热操作：路由表用原子快照切换，已进入宿主 admission 的请求不会被强行取消；关闭数据库或 worker 仍由正常进程排空流程负责。

Web 兼容路由也服从同一状态：`store`、`assistant`、`toolmarket`、`payments` 和 `operations` 被禁用后返回明确的不可用响应。Core 身份和健康接口不通过插件开关绕过权限。

## 开发、测试和发布

1. 在 `services/extensions/internal/modules/<name>` 实现模块及窄依赖，必要时在 `composition` 中显式装配；不要在插件中自行打开 Core 数据库。
2. 给 HTTP 路由、鉴权失败、停用状态、并发上限和关闭排空写行为测试。至少运行：

   ```sh
   (cd services/extensions && go test ./internal/modules ./internal/webcompat ./internal/app)
   ```

3. 更新宿主的显式注册/选择、配置示例、本文档和分发清单；重新构建并核对源码提交、包摘要和运行时 `BUILDINFO`。
4. 先在带有专用 Core RPC 和隔离数据库的候选宿主启用，验证 `/extensions/v1/modules`、Web 兼容路由、禁用后的 503 和重载期间的请求排空，再考虑生产灰度。

生产安装只替换经过校验的宿主包。`host.plugin_state_file` 是宿主用户可写的私有状态文件，目录应为 0700、文件为 0600；其中没有凭据、session、余额或插件代码。状态损坏或摘要不匹配时宿主拒绝启动，避免静默启用错误版本。

该设计支持管理员启停、卸载和热重载已发布插件；它不把网络请求变成代码发布渠道。真正的第三方插件若需要不受信任执行，必须先设计独立进程或容器协议并单独完成安全和资金验收，不能直接改成进程内动态加载。
