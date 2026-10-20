# Arch Linux 安装和更新 LMM API

后端使用两个独立的 AUR 包：`lmm-api-core-bin` 是 Rust 核心，
`lmm-api-extensions-bin` 是可选的 Go 扩展。前端是静态文件，由 Nginx 提供。
核心可以单独运行；更新扩展不需要重启核心或 PostgreSQL。

本文使用软件包提供的 `/usr/bin` 程序和 systemd unit，
不使用可选的 `/opt/lmm` 程序版本仓库，也不使用旧单体的服务和环境变量配置。
已有站点的数据迁移、旧服务停止和首次流量切换按该站点的部署安排执行，
不要把下面的空库安装命令用于现有业务数据库。

## 本次发布版本

| 组件 | 安装目标 | 与本轮前端匹配的版本 |
| --- | --- | --- |
| 核心 | `lmm-api-core-bin` | **待本轮发布后填写** |
| 扩展 | `lmm-api-extensions-bin` | **待本轮发布后填写** |
| 前端 | 经核验的 Web 静态归档 | **待本轮发布后填写版本、来源 SHA 和归档 SHA-256** |

仓库中已有的 Core `0.1.0rc2-1`、扩展 `0.1.0rc1-1` 是之前的发布，
不能据此认定它们包含本轮新增的价格接口和前端接线。
发布者应在这里填写最终版本，再按这组产物安装。

下文的 `<…>`、`LMM_SOURCE_DIR`、`WEB_RELEASE_ID` 等都是待替换项，
不是可直接使用的凭据或版本。数据库连接串只写进私密文件，不放进命令参数。

## 一、首次安装

### 1. 安装软件包

在普通构建用户下读取、检查 AUR 配方，再打包。`-bin` 配方下载已构建的归档，
不编译应用；小内存服务器只接收最终包即可。不要以 root 运行 `makepkg`。

```sh
git clone https://aur.archlinux.org/lmm-api-core-bin.git
cd lmm-api-core-bin
less PKGBUILD
makepkg --syncdeps --cleanbuild
```

需要扩展时，在另一目录对它执行同样步骤：

```sh
git clone https://aur.archlinux.org/lmm-api-extensions-bin.git
cd lmm-api-extensions-bin
less PKGBUILD
makepkg --syncdeps --cleanbuild
```

将与上表版本、平台和可信归档摘要一致的 `.pkg.tar.zst` 传到服务机。
只安装选中的实际文件，不用宽泛通配符选中目录里的历史包：

```sh
sudo pacman -U /absolute/staging/<core-package>.pkg.tar.zst
sudo pacman -U /absolute/staging/<extensions-package>.pkg.tar.zst
sudo systemd-sysusers /usr/lib/sysusers.d/lmm-core.conf
sudo systemd-sysusers /usr/lib/sysusers.d/lmm-extensions.conf
```

只装核心时省略扩展的两条命令。包不会初始化数据库、启用服务或自动重启。
Arch 的系统 hook 可能已经创建账户，显式执行对应 sysusers 声明仍可用于确认。

| 内容 | 核心 | 扩展 |
| --- | --- | --- |
| 程序 | `/usr/bin/lmm-core` | `/usr/bin/lmm-extensions` |
| 服务 | `lmm-core.service` | `lmm-extensions.service` |
| 配置 | `/etc/lmm.conf.d/core.toml` | `/etc/lmm.conf.d/extensions.toml` |
| 私密文件 | `/etc/lmm.conf.d/secrets/core/` | `/etc/lmm.conf.d/secrets/extensions/` |
| 持久状态 | `/var/lib/lmm/core/` | `/var/lib/lmm/extensions/` |
| 临时 socket | `/run/lmm/core/` | `/run/lmm/extensions/` |

### 2. 准备配置和秘密

先查看安装的示例并按本机填写，服务只读这个固定文件，不扫描配置目录：
下面涉及扩展账户、目录和配置的命令，仅在已经安装扩展包时执行。

```sh
sudo install -d -m 0755 /etc/lmm.conf.d/secrets
sudo install -d -m 0700 -o lmm-core -g lmm-core /etc/lmm.conf.d/secrets/core
sudo install -d -m 0700 -o lmm-extensions -g lmm-extensions /etc/lmm.conf.d/secrets/extensions
sudo install -d -m 0700 /root/lmm-install
sudoedit /etc/lmm.conf.d/core.toml
sudoedit /etc/lmm.conf.d/extensions.toml
```

最小 Core 配置包含 `version = 1`、`server` 和 `database`；
`database.url_file` 指向 Core 的 **runtime** 连接串文件。
初始监听保留 `127.0.0.1:8080`，由 Nginx 对外提供 HTTPS。
扩展保留 `127.0.0.1:8081`，先用 `host.modules = ["none"]` 明确不启用业务模块。
这只能验证基础进程启动，不代表整站业务已配置。

私密文件使用对应服务账户持有的 `0600`，或它实际可读的 `0400`。
TOML 可为 root 持有的 `0644`，但不能允许其他用户写入。
私密目录不要授予共享 RPC 组读取权限。

浏览器登录还需要 `[identity.web]` 的真实 HTTPS `origin`、单独的 refresh key，
并明确站点是否发布用户协议、隐私政策。开启控制台使用 `[console] enabled = true`。
模型网关需另配 `[model] config_file` 和供应商凭据；空示例不会自动打开模型、支付或商城。
按[当前配置示例](../config/core.example.toml)填写，不把 TOML 当 shell `source`。
这一步只准备文件，不启动服务；数据库配置记录和内置目录在创建首位管理员后再安装。

需要生成原始 32 字节加密 key 时，可以在 root 私密目录生成后分别安装给服务：

```sh
sudo sh -c 'umask 077; openssl rand 32 > /root/lmm-install/web-refresh-key'
sudo install -m 0600 -o lmm-core -g lmm-core /root/lmm-install/web-refresh-key /etc/lmm.conf.d/secrets/core/web-refresh-key
```

可恢复 API Key 的 `api_key_encryption_key_file`、MFA key、市场快照 key 都要独立生成和保存，
不能复制 refresh key 充当它们。它们是持久秘密，更新时保留，不能每次启动重新生成。
RPC/host token 则是 32–256 字节的可见 ASCII 文本，可用 `openssl rand -hex 32` 生成；
不要把原始二进制加密 key 当 RPC token。

### 3. 创建全新的 Core 数据库

PostgreSQL 由操作者单独安装、配置。下面假定本机已有可用 PostgreSQL，
新库名选为 `lmm_core`；先确认这个名字没有现有业务数据。

软件包尚未安装数据库授权 SQL。把与所装二进制 **来源 SHA 一致** 的源码只读副本
放在绝对路径 `LMM_SOURCE_DIR`，使用其中这两个真实文件：

- `deployment/postgres/create-core-roles.sql`：创建 `lmm_core_owner NOLOGIN` 和受限 `lmm_core_runtime LOGIN`。
- `deployment/postgres/grant-core-runtime.sql`：初始化后，按实际启用的功能授权。

新库创建和角色准备由 PostgreSQL 管理员执行：

```sh
LMM_SOURCE_DIR='/absolute/pinned-source'
sudo -u postgres createdb lmm_core
sudo -u postgres psql -X -v ON_ERROR_STOP=1 -d lmm_core -f "$LMM_SOURCE_DIR/deployment/postgres/create-core-roles.sql"
sudo -u postgres psql -X -v ON_ERROR_STOP=1 -d lmm_core
```

在该私密 `psql` 会话中，为 runtime 设置密码；独立安装员也应预先创建，
只允许它离线切到 `lmm_core_owner`，不把这个成员关系授给 runtime：

```sql
ALTER DATABASE lmm_core OWNER TO lmm_core_owner;
\password lmm_core_runtime
CREATE ROLE lmm_core_installer LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
GRANT lmm_core_owner TO lmm_core_installer WITH INHERIT FALSE, SET TRUE;
GRANT CONNECT ON DATABASE lmm_core TO lmm_core_installer;
REVOKE CONNECT ON DATABASE lmm_core FROM PUBLIC;
\password lmm_core_installer
```

这里的 `ALTER DATABASE` 只针对上面刚创建、仍为空的 `lmm_core`，
使数据库、后续 schema 和表都由离线 `lmm_core_owner` 持有；runtime 不能是 owner。
`create-core-roles.sql` 只授权 CONNECT/CREATE，本身不会改变数据库 owner。
不要对旧站数据库照搬这个修改。

这里的撤销只针对刚创建的新库，避免 Go 的其他 runtime 因 PostgreSQL 默认的
`PUBLIC CONNECT` 获得 Core 连接权。已有集群的其他数据库权限另行核对，不能批量重置。

用私密编辑器创建两份连接串：

| 文件 | 内容 |
| --- | --- |
| `/root/lmm-install/core-owner-url` | 安装员登录到 `lmm_core`，包含 `options=-c%20role%3Dlmm_core_owner`，使新对象真正归 NOLOGIN owner |
| `/etc/lmm.conf.d/secrets/core/database-url` | `lmm_core_runtime` 登录同一个新库，不含 `SET ROLE` 选项 |

URL 中的密码须按 URL 规则编码；仅本机回环连接可按本机 PostgreSQL 配置选择非 TLS，
远程数据库使用经过验证的 TLS 配置。不要把真实 URL 放进 shell 历史、截图或问题反馈。

```sh
sudoedit /root/lmm-install/core-owner-url
sudoedit /etc/lmm.conf.d/secrets/core/database-url
sudo chmod 0600 /root/lmm-install/core-owner-url
sudo chown lmm-core:lmm-core /etc/lmm.conf.d/secrets/core/database-url
sudo chmod 0600 /etc/lmm.conf.d/secrets/core/database-url
```

离线安装配置 `/root/lmm-install/core-owner.toml` 可以只有：

```toml
version = 1
[database]
url_file = "/root/lmm-install/core-owner-url"
max_connections = 1
```

用 `sudoedit /root/lmm-install/core-owner.toml` 保存这份 TOML，并设置 `0600`。

确认 owner 文件仅 root 可读、runtime 文件归 `lmm-core` 后，**只对这个新空库执行一次**：

```sh
sudo /usr/bin/lmm-core --config /root/lmm-install/core-owner.toml init-db
```

接下来先创建首位正式管理员，不要提前写入配置、模型目录、内置工具或业务记录。

### 4. 创建第一位正式管理员

`init-db` 后使用正式的 `create-first-owner`，为新站创建可用密码登录的 superadmin。
它只接受刚初始化、尚无用户、配置目录或其他业务数据的库；
调用者必须是同时拥有数据库、schema 和应用表的离线 owner。
上面的独立 installer 登录并 `SET ROLE lmm_core_owner` 满足该权限模型，runtime 不满足。

用私密编辑器创建输入文件，**不要在 shell 命令或参数里拼接密码**：

```sh
sudoedit /root/lmm-install/first-owner.json
sudo chmod 0600 /root/lmm-install/first-owner.json
```

文件只包含以下三个字段。替换示例值；密码中的双引号、反斜杠等按 JSON 规则转义。
不要新增 `role`、余额或其他字段：

```json
{
  "username": "site-owner",
  "display_name": "Site Owner",
  "password": "<REPLACE_WITH_A_LONG_UNIQUE_PASSWORD>"
}
```

用户名为 3–20 字节的小写 ASCII 字母、数字或 `_.-`，首字符必须是字母或数字。
显示名可用中文，最多 80 个字符且不含首尾空白；密码至少 12 个字符。
输入文件最大 4096 字节。

确认文件内容后执行一次，密码经 stdin 进入程序，输出只保存无秘密的安装回执：

```sh
sudo sh -c 'umask 077; set -C; /usr/bin/lmm-core --config /root/lmm-install/core-owner.toml create-first-owner < /root/lmm-install/first-owner.json > /root/lmm-install/first-owner-receipt.json'
```

回执包含 `protocol = core.first-owner.v1`、用户 ID、用户名、`platform_role = superadmin`
和 `setup_completed = true`。这一步已同时记录初始化完成，**不再运行 `complete-setup`**。
它不充值、不生成 Session/API Key、不发奖励，也不插入额外 developer 授权行；
管理员访问权限使用现有角色规则。输入 JSON 含明文密码，不要打包、上传或放进日志。

现在再配置模型、安装需要的内置目录，并按实际功能给 runtime 授权。
例如，启用浏览器控制台、身份 RPC 和模型网关时：

```sh
sudo -u postgres psql -X -v ON_ERROR_STOP=1 -d lmm_core \
  -v runtime_role=lmm_core_runtime \
  -v enable_console=true -v enable_web_identity=true \
  -v enable_events=true -v enable_models=true \
  -f "$LMM_SOURCE_DIR/deployment/postgres/grant-core-runtime.sql"
```

图片、支付、商城、OAuth、助手、工具市场等有各自的 `enable_*` 开关，
按[离线授权说明](../deployment/postgres/README.md)和实际配置选择。
授权开关不会配置上游或启用 HTTP 功能；`false` 也不会撤销以前给过的权限。

**已有站点的数据迁移不使用 `create-first-owner`。**
导入旧管理员后，仍按 `plan-admin-activation` / `apply-admin-activation` 的预览与本人确认
激活，再执行一次 `complete-setup` 记录初始化完成；不能用创建新 owner 绕过导入或确认。
`bootstrap-user` 仍只是开发 bootstrap，不用于正式站点管理员安装。

### 5. 按需安装扩展存储与 RPC

Go 不得持有 Core 的 owner/runtime 连接串。每个已启用模块使用自己的数据库，
数据库 owner 为 NOLOGIN，离线 installer 可 `SET ROLE`，runtime 为无角色继承的独立 LOGIN。
各模块库撤销 `PUBLIC CONNECT/CREATE/TEMP` 后，只授权各自角色；
不能只在 TOML 中换个库名，却继续让所有 runtime 连接全部库。

例如，在 PostgreSQL 管理员的私密 `psql` 会话中，为全新的支付库建立独立角色：

```sql
CREATE ROLE lmm_payments_owner NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
CREATE ROLE lmm_payments_installer LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
CREATE ROLE lmm_payments_runtime LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
GRANT lmm_payments_owner TO lmm_payments_installer WITH INHERIT FALSE, SET TRUE;
CREATE DATABASE lmm_payments OWNER lmm_payments_owner;
REVOKE ALL ON DATABASE lmm_payments FROM PUBLIC;
GRANT CONNECT ON DATABASE lmm_payments TO lmm_payments_installer, lmm_payments_runtime;
\password lmm_payments_installer
\password lmm_payments_runtime
```

`CREATE DATABASE` 在事务外执行。将安装员登录 URL 写入
`/root/lmm-install/payments-owner-url`，设为 root 持有的 `0600`；
Go installer 自己执行 `SET ROLE`，这个 URL **不要**沿用 Core 的 `options` 查询参数。
模块 runtime 的 URL 则单独安装到扩展私密目录，归 `lmm-extensions` 持有且为 `0600`，
填写在该模块的 `database_url_file` 中。

当前真实的离线安装入口如下，以已经预建好角色和空库的支付模块为例：

```sh
sudo /usr/bin/lmm-extensions install-db \
  --owner-url-file /root/lmm-install/payments-owner-url \
  --database lmm_payments \
  --owner-role lmm_payments_owner \
  --runtime-role lmm_payments_runtime \
  --component payments
```

`install-db` 必须是第一个参数，不接受 `--config`。
它还支持 `assistant`、`toolmarket`、`toolmarket-builtins`；数据库名须匹配组件前缀。
后两者还要求 `--encryption-key-file` 指向持久的原始 32 字节市场快照 key。
`store/support/operations` 没有这个通用安装子命令，使用各自模块的实际离线 schema 过程。
普通 Go 启动不安装或升级 schema，也没有 `check-db` 子命令。

即使暂不启用模块，Go 的 `host.token_file` 也必须可读。例如生成其独立 host token：

```sh
sudo sh -c 'umask 077; openssl rand -hex 32 > /root/lmm-install/extensions-host-token'
sudo install -m 0600 -o lmm-extensions -g lmm-extensions /root/lmm-install/extensions-host-token /etc/lmm.conf.d/secrets/extensions/host-token
```

RPC 分为两条通道，不能把 socket 或 token 混用：

| 通道 | 示例 socket | 作用 |
| --- | --- | --- |
| 身份/事件 gRPC UDS | `/run/lmm/core/core.sock` | Core `[rpc]` 对应 Go `[core] socket`；身份、commerce、payments、assistant 各用途独立 token |
| 私有 HTTP UDS | `/run/lmm/core/private.sock` | tool market、wallet、bounties、images、support projection 共享这个 HTTP socket，各路由仍使用独立用途 token |

同一用途在 Core 和 Go 两端的 token 内容须相同，但各自保存为服务私有副本；
不同用途独立生成。Go 的 `host.token_file` 也是另一份秘密，绝不交给浏览器。
私有 HTTP socket 必须与身份 gRPC socket 不同；不能因为两者都是 Unix socket 就交换使用。
HTTP socket 只给内部进程，不能经 Nginx 对外代理。

跨服务账户的 socket 权限使用已随包提供的配置片段：

```sh
sudo install -d -m 0755 /etc/systemd/system/lmm-core.service.d
sudo install -d -m 0755 /etc/systemd/system/lmm-extensions.service.d
sudo install -m 0644 /usr/share/doc/lmm-core/rpc.service.conf.example /etc/systemd/system/lmm-core.service.d/rpc.conf
sudo install -m 0644 /usr/share/doc/lmm-extensions/rpc.service.conf.example /etc/systemd/system/lmm-extensions.service.d/rpc.conf
getent group lmm-rpc
```

将输出中的实际数字 GID 写入 Core `[rpc].shared_group_id`；
已启用的私有 HTTP 监听配置也须使用同一实际 GID。
前面的 Core `systemd-sysusers` 声明会创建 `lmm-rpc`；如果查不到，先检查账户准备是否完成。
这些 drop-in 让 Core 的主组为 `lmm-rpc`、RPC 目录可遍历，Go 加入补充组；
IPC 目录为 `0750`、socket 为 `0660`，秘密仍为 `0600`。
最后在 `host.modules` 中明确选择所需模块并填写它们的实际配置，
不要把 `modules = ["none"]` 当成整站部署配置。

### 6. 启动并核对运行中的程序

```sh
sudo systemctl daemon-reload
sudo -u lmm-core /usr/bin/lmm-core --config /etc/lmm.conf.d/core.toml check-db
sudo systemctl enable --now lmm-core.service
sudo systemctl enable --now lmm-extensions.service
```

只使用核心时省略扩展启动。Core `check-db` 使用服务 UID 和 runtime TOML，
成功表示当前 schema/受限权限检查通过，不意味着模型、支付和所有扩展都已验收。

```sh
systemctl show lmm-core.service lmm-extensions.service -p ActiveState -p SubState -p MainPID
curl --fail --silent --show-error http://127.0.0.1:8080/health/ready
curl --fail --silent --show-error http://127.0.0.1:8081/health/ready
```

启用 `[edge_access]` 后，Core 的健康接口也要求受信反代凭据。
此时普通直连会返回 `403 INTERNAL_ONLY`，不能据此判断进程不健康。
运维检查应通过已配置的 Nginx，或在仅运维可读的 `0600` curl 配置文件中提供
`X-LMM-Edge-Proof` 和 `X-LMM-Edge-Client-IP` 后访问回环监听。
前者读取本站独立 proof 文件，后者填写实际检查来源 IP；不要把 proof 放进命令参数、
浏览器或 Go 配置。用后文的 `curl --config` 方式读取该私密文件。

Go 的健康响应只表示宿主存活/接收请求，不能代替所选模块的数据库和 RPC 业务检查。
Core 模型未启用时 `/health/model-ready` 为 503；不要为改成 200 而虚开模型或花钱探测。

确认进程真正执行了包内程序，而不是旧 `/opt` 副本：

```sh
CORE_PID=$(systemctl show lmm-core.service -p MainPID --value)
test "$CORE_PID" -gt 0
sudo readlink "/proc/$CORE_PID/exe"
sudo sha256sum "/proc/$CORE_PID/exe" /usr/bin/lmm-core
```

扩展同样核对自己的 MainPID 和 `/usr/bin/lmm-extensions`。
若看到 `(deleted)` 或摘要不同，说明磁盘升级后进程仍在执行旧程序。

## 二、部署静态前端和 Nginx

前端在构建机生成并保留来源 SHA、版本和归档摘要，服务机不运行 Bun 或重新构建。
以下 `/srv/lmm-web` 是操作者管理的静态发布目录，不是 AUR 后端包的安装路径：

```text
/srv/lmm-web/releases/<WEB_RELEASE_ID>/dist/  # 完整、只读、不能覆盖同一版本
/srv/lmm-web/current                        # 指向当前 dist 的符号链接
/srv/lmm-web/shared/static/                 # 保留历次带 hash 的 JS/CSS/font
```

上传到新版本目录，核对归档及展开后的文件清单，再设为 root 持有、Nginx 可读。
将该版本 `dist/static/` 加入共享静态目录；同路径若已有不同字节，应中止并修复产物命名，
不能覆盖旧浏览器仍在使用的资源。保留归档随附的许可证文件。

在已经配置好 HTTPS 的站点中合并静态路由，保留原来的 API、模型、OAuth、
证书续期和流式连接配置。当前 Web 产物的代码资源前缀是 `/static/`：

```nginx
location ^~ /static/ {
    root /srv/lmm-web/shared;
    try_files $uri =404;
    add_header Cache-Control "public, max-age=31536000, immutable";
}
location / {
    root /srv/lmm-web/current;
    try_files $uri $uri/ /index.html;
    add_header Cache-Control "no-cache" always;
}
```

这两段仅负责前端。Core 的 `/core/`、`/v1/`、`/v1beta/`、身份和控制台接口，
以及 Go 的实际公共模块接口，必须按当前后端路由分别代理；不能把所有 `/api/`
一概交给 Go，也不能把错误的 API 请求降级成前端 `index.html`。
浏览器的 `/core/v1/pricing/session` 使用真实 Session；OAuth bearer 不是它的替代凭据。

准备完候选版本后，用同一文件系统内的 rename 原子替换链接：

```sh
WEB_RELEASE_ID='<FINAL_WEB_RELEASE_ID>'
sudo ln -s "/srv/lmm-web/releases/$WEB_RELEASE_ID/dist" "/srv/lmm-web/.current-$WEB_RELEASE_ID"
sudo mv -Tf "/srv/lmm-web/.current-$WEB_RELEASE_ID" /srv/lmm-web/current
sudo nginx -t
sudo systemctl reload nginx.service
readlink /srv/lmm-web/current
```

第一次部署先合并并检查 Nginx 配置，再对外开放；以后只换静态版本链接不需要重启后端。
`index.html` 应重新验证，带 hash 的旧资源继续可读。
原子换链接避免半上传页面，但不自动保证所有浏览器版本和新旧 API 都兼容。

需要维护窗口时，先准备好版本目录和配置，再使用
[现有维护页生成及切换工具](../deployment/native/README.md#cutover-after-migration-is-qualified)。
维护页应返回 503、`Retry-After` 和真实的预计完成时间，Nginx、SSH、PostgreSQL 保持运行。
这些维护工具可配合 AUR 服务使用；不要因此套用其可选 `/opt` 程序激活脚本。

## 三、后续更新

### 只更新 Core

提前核对新 Core 与现有 schema、配置和前端 API 的兼容性，准备好更新包。
如果 release 要求 schema 变化，先执行该版本明确的迁移流程；下面的普通更新不改数据库。

```sh
sudo pacman -U /absolute/staging/<new-core-package>.pkg.tar.zst
sudoedit /etc/lmm.conf.d/core.toml
sudo systemctl daemon-reload
sudo -u lmm-core /usr/bin/lmm-core --config /etc/lmm.conf.d/core.toml check-db
sudo systemctl restart lmm-core.service
```

配置没有要合并的变化时省略 `sudoedit`。检查 `.pacnew` 并按字段合并，不用新示例覆盖旧配置。
更新后重新核对 MainPID、`/proc/<PID>/exe` 摘要、readiness 和受影响接口。
这里不运行 `init-db`、`create-first-owner`、`bootstrap-user` 或 `complete-setup`，也不重建数据库。

### 只更新 Go 扩展

```sh
sudo pacman -U /absolute/staging/<new-extensions-package>.pkg.tar.zst
sudoedit /etc/lmm.conf.d/extensions.toml
sudo systemctl daemon-reload
sudo systemctl restart lmm-extensions.service
```

保留每个模块的数据库、持久 key、用途 token、RPC socket 和模块选择。
正常更新不运行 `install-db`，不重启 Core 或 PostgreSQL。
对所更新模块做实际只读验证；只看 Go 健康页不够。

### 只更新前端

构建机产生更高版本的静态归档，上传到新的 release 目录，核验并补齐共享 hash 资源，
确认新接口已由对应后端版本提供，再按上面的原子链接步骤切换。
不覆盖当前目录、不在服务器重建、不清理旧浏览器正在使用的资源。

### 凭据与非付费验证

`/v1/models` 是认证后的模型列表读取，不会发起付费模型请求。
使用已有、权限合适的 API Key，把 curl 配置放在服务机私密文件中，例如：

```text
url = "https://<YOUR_DOMAIN>/v1/models"
header = "Authorization: Bearer <EXISTING_API_KEY>"
silent
show-error
fail
```

由私密编辑器填入实际值，文件权限设为 `0600`，再执行：

```sh
curl --config /absolute/private/model-list.curl --output /absolute/private/model-list.json
```

不要用 `curl -H "Authorization: Bearer $KEY"` 把展开后的 Key 放到进程参数中，
也不要把 curl 配置、响应中的私有信息或原始 journal 整份发到公开问题里。
验证浏览器登录、价格目录和模块读取时，同样使用已有授权，不能为部署检查注册生产奖励账户。

## 四、失败时恢复什么

本项目的生产软件保持版本向前，失败修复发布更高补丁版本；
不使用 `pacman -U` 安装旧包，也不把 Web current 切回更低软件版本。
旧包和旧 release 目录保留作证据，不是自动软件降级入口。

Nginx 配置没有通过 `nginx -t` 时不 reload；修复候选配置。
已选维护页或发生部分切换时，先读回链接、运行 PID、数据库和事务状态，
再恢复正确配置或部署修复版。不要连续盲目重试不确定的支付、回调或迁移动作。

需要恢复数据库时，按这次实际备份和恢复安排处理，并保留备份后的有效写入、
未决操作和对账结果；不能把恢复较早快照当成普通更新的回滚步骤。
详见[生产更新规则](release-architecture.md#生产版本只升级)。

本文的命令按当前源码和包清单核对；写入文档不代表这些步骤已在某台机器完成。
实际交付要分别记录安装版本、运行程序、前端版本和可用接口。
