# Hop 配置参考

Hop 有两类配置文件，它们解决的问题不同：

| 配置 | 内容 | 生效方式 |
|---|---|---|
| 启动配置 | 监听地址、SQLite 路径、Host Key、Master Key、日志和资源来源 | 启动时读取，修改后重启 Hop |
| 资源清单 | 凭据、SSH/TCP 资产、Access Key 和资产白名单 | validate、diff、apply 后写入 SQLite，可热更新 |

SQLite 是资源的运行事实源。SSH 请求不会临时读取 YAML 或 TOML。资源清单、CLI 和 Control API 都通过同一个 Catalog 修改 SQLite。

## 启动配置

使用 `--config` 指定启动配置：

```bash
hop-server --config /etc/hop/config.toml serve
```

文件扩展名必须是 `.toml`、`.yaml` 或 `.yml`。两种格式使用相同的字段，未知字段会导致启动失败。不传 `--config` 时，Hop 使用内置默认值。

完整 TOML 示例：

```toml
[server]
ssh_listen = "0.0.0.0:2222"

[database]
path = "/var/lib/hop/hop.db"

[api]
enabled = false
listen = "127.0.0.1:8083"
token_file = "/var/lib/hop/control-api.token"
cors_allowlist = []

[ssh]
host_key_file = "/var/lib/hop/hop_host_key"
host_key_type = "ed25519"
banner = "Welcome to Hop"
keepalive_interval = 30
connect_timeout = 10
proxy_policy = "assets_only"

[security]
master_key_file = "/var/lib/hop/hop.secret"

[inventory]
sources = []

[runtime]
temp_dir = "/tmp/hop"
log_level = "info"
session_retention_days = 30
```

等价的 YAML 写法：

```yaml
server:
  ssh_listen: 0.0.0.0:2222

database:
  path: /var/lib/hop/hop.db

api:
  enabled: false
  listen: 127.0.0.1:8083
  token_file: /var/lib/hop/control-api.token
  cors_allowlist: []

ssh:
  host_key_file: /var/lib/hop/hop_host_key
  host_key_type: ed25519
  banner: Welcome to Hop
  keepalive_interval: 30
  connect_timeout: 10
  proxy_policy: assets_only

security:
  master_key_file: /var/lib/hop/hop.secret

inventory:
  sources: []

runtime:
  temp_dir: /tmp/hop
  log_level: info
  session_retention_days: 30
```

相对路径按进程工作目录解析，不按配置文件所在目录解析。systemd 示例的工作目录是 `/var/lib/hop`，容器的工作目录是 `/data`。正式部署建议使用绝对路径。

### server

| 字段 | 默认值 | 含义 |
|---|---|---|
| `ssh_listen` | `0.0.0.0:2222` | Hop SSH 服务的监听地址，格式为 `IP:PORT` |

修改监听地址后需要重启。对公网开放前，应同时配置主机防火墙或云防火墙。

### database

| 字段 | 默认值 | 含义 |
|---|---|---|
| `path` | `./hop.db` | SQLite Catalog 路径 |

Catalog 保存资产、凭据密文、Access Key、资产白名单、Known Hosts、会话和资源来源状态。备份数据库时必须同时备份 Master Key。

### api

| 字段 | 默认值 | 含义 |
|---|---|---|
| `enabled` | `false` | 是否启动 HTTP Control API |
| `listen` | `127.0.0.1:8083` | API 监听地址 |
| `token_file` | `./hop-api.token` | Bearer Token 文件路径 |
| `cors_allowlist` | `[]` | 允许访问 API 的浏览器 Origin 列表 |

API 默认关闭。启用后，`token_file` 必须存在并包含 Token。Hop 不自动生成 API Token。

当 `listen` 不是 loopback 地址时，`cors_allowlist` 不能为空。CORS 只约束浏览器请求，不能替代 TLS、防火墙或反向代理认证。

```toml
[api]
enabled = true
listen = "127.0.0.1:8083"
token_file = "/var/lib/hop/control-api.token"
cors_allowlist = []
```

### ssh

| 字段 | 默认值 | 含义 |
|---|---|---|
| `host_key_file` | `./hop_host_key` | Hop 对入口 SSH 客户端使用的服务器 Host Key |
| `host_key_type` | `ed25519` | Host Key 类型，当前只接受 `ed25519` |
| `banner` | `Welcome to Hop` | 认证阶段显示的文本，空字符串表示关闭 |
| `keepalive_interval` | `30` | SSH keepalive 间隔，单位为秒，`0` 表示关闭 |
| `connect_timeout` | `10` | Hop 连接目标 SSH 服务的超时，单位为秒 |
| `proxy_policy` | `assets_only` | TCP 转发策略，当前只接受 `assets_only` |

Hop 在 `host_key_file` 不存在时生成新密钥。应长期保留该文件，否则客户端会看到 Host Key 变化警告。

`assets_only` 表示 ProxyJump 和本地 TCP 转发只能连接 Catalog 中存在且当前 Access Key 有权访问的资产。

### security

| 字段 | 默认值 | 含义 |
|---|---|---|
| `master_key_file` | `./hop.secret` | 加密目标凭据的 Master Key 文件 |

Hop 在文件不存在时生成 Master Key。丢失该文件后，数据库中的目标密码和私钥无法解密。不要把 Master Key 提交到 Git。

### inventory

`inventory.sources` 声明启动时加载的资源清单。每个 source 有以下字段：

| 字段 | 必填 | 含义 |
|---|---|---|
| `id` | 是 | 稳定来源名称，只允许 ASCII 字母、数字、`.`、`-` 和 `_`，最长 128 个字符 |
| `path` | 是 | 单个文件路径或 glob，例如 `/etc/hop/resources.d/*.yaml` |
| `watch` | 否 | 是否监控文件变化并自动 apply，默认 `false` |
| `prune` | 否 | 是否删除本次完整扫描中消失的资源，默认 `false` |

TOML 示例：

```toml
[[inventory.sources]]
id = "home"
path = "/etc/hop/resources.d/*.yaml"
watch = true
prune = false
```

所有 source 都会在 Hop 启动时 apply 一次。`watch = true` 的 source 会继续监控文件。Hop 等待文件范围连续两次保持稳定后再 apply，解析失败时保留上一份有效 Catalog。

`prune = false` 时，成功扫描中消失的资源会标记为 orphan，但仍可使用。只有显式 `state: absent` 或启用 prune 才会删除资源。建议保持默认值，先通过 `config diff` 检查删除范围。

### runtime

| 字段 | 默认值 | 含义 |
|---|---|---|
| `temp_dir` | `/tmp/hop` | Hop 启动时创建的临时目录 |
| `log_level` | `info` | tracing 日志过滤级别，`RUST_LOG` 环境变量优先 |
| `session_retention_days` | `30` | 启动时清理已结束会话的保留天数，`0` 表示不自动清理 |

## 资源清单

资源清单也支持 YAML 和 TOML。下面的 YAML 同时定义了目标凭据、一个 SSH 资产、两个 TCP 资产和一把入口 Access Key：

```yaml
api_version: hop/v1alpha1

credentials:
  nas-root:
    type: ssh_key
    username: root
    private_key:
      file: /etc/hop/secrets/nas-root

assets:
  nas:
    type: ssh
    host: 192.168.1.20
    port: 22
    display_name: Home NAS
    credential: nas-root

  windows-rdp:
    type: tcp
    host: 192.168.1.30
    port: 3389
    description: Windows desktop

  home-mysql:
    type: tcp
    host: 192.168.1.40
    port: 3306

access:
  laptop:
    public_key:
      file: /etc/hop/keys/laptop.pub
    assets:
      - nas
      - windows-rdp
      - home-mysql
```

当前公开配置只使用 `ssh` 和 `tcp` 两种资产类型。RDP、MySQL、Redis 等都是 TCP 服务，不需要类型别名。

资源名称必须以 ASCII 字母或数字开头，只能包含 ASCII 字母、数字、`.`、`-` 和 `_`，最长 128 个字符。名称在同一种资源中不能重复。

### api_version

| 字段 | 值 |
|---|---|
| `api_version` | 当前必须是 `hop/v1alpha1` |

Hop 会拒绝未知版本和未知字段。

### credentials

`credentials` 是以稳定名称为 key 的 map。名称用于资产引用。

| 字段 | 适用类型 | 含义 |
|---|---|---|
| `state` | 全部 | 默认 `present`；使用 `absent` 删除资源，删除声明不能再带其他字段 |
| `type` | 全部 | `password` 或 `ssh_key` |
| `username` | 全部 | 连接目标 SSH 服务时使用的用户名 |
| `password` | `password` | 密码来源 |
| `private_key` | `ssh_key` | OpenSSH 私钥来源 |
| `passphrase` | `ssh_key` | 私钥口令来源，可选 |

Secret 来源必须且只能设置 `file` 或 `env`：

```yaml
password:
  file: /etc/hop/secrets/nas.password
```

```yaml
password:
  env: HOP_NAS_PASSWORD
```

Hop 读取 secret 后使用 Master Key 加密写入 SQLite。diff、API 响应、日志和审计不会返回 secret 内容。使用 `env` 时，执行 validate 或 apply 的进程必须能读取对应环境变量。

### assets

`assets` 也是以稳定名称为 key 的 map。名称用于 SSH 直连、`.hop` 转发目标和 Access Key 白名单。

| 字段 | 必填 | 含义 |
|---|---|---|
| `state` | 否 | 默认 `present`；`absent` 表示删除 |
| `type` | 是 | `ssh` 或 `tcp` |
| `host` | 是 | Hop 所在网络能够访问的目标主机名或 IP |
| `port` | 是 | `1..=65535` 的目标端口 |
| `display_name` | 否 | 界面中显示的名称 |
| `description` | 否 | 资产说明 |
| `credential` | SSH 资产可选 | 引用 `credentials` 中的名称 |

SSH 资产配置凭据后支持托管 Shell、远程命令和 SCP/SFTP。不配置凭据时仍可用于 ProxyJump。TCP 资产不能引用 SSH 凭据，只提供透明 TCP 转发。完整用法见 [SSH 与 TCP 代理](proxying.zh-CN.md)。

### access

`access` 中的每个条目表示一把入口 SSH 公钥，不是用户账号。

| 字段 | 必填 | 含义 |
|---|---|---|
| `state` | 否 | 默认 `present`；`absent` 表示删除 |
| `public_key` | 是 | 单把 OpenSSH 公钥的 `file` 或 `env` 来源 |
| `enabled` | 否 | 默认 `true` |
| `assets` | 否 | 可访问的资产名称列表 |

`assets` 有三种语义：

```yaml
# 省略 assets，访问当前和未来的全部资产
access:
  owner:
    public_key: { file: /etc/hop/keys/owner.pub }
```

```yaml
# 空数组，允许公钥认证，但不能发现或访问资产
access:
  suspended:
    public_key: { file: /etc/hop/keys/suspended.pub }
    assets: []
```

```yaml
# 非空数组，使用严格白名单
access:
  automation:
    public_key: { file: /etc/hop/keys/automation.pub }
    assets: [nas]
```

## TOML 资源清单

相同资源也可以使用 TOML：

```toml
api_version = "hop/v1alpha1"

[credentials.nas-root]
type = "ssh_key"
username = "root"

[credentials.nas-root.private_key]
file = "/etc/hop/secrets/nas-root"

[assets.nas]
type = "ssh"
host = "192.168.1.20"
port = 22
credential = "nas-root"

[assets.windows-rdp]
type = "tcp"
host = "192.168.1.30"
port = 3389

[access.laptop.public_key]
file = "/etc/hop/keys/laptop.pub"

[access.laptop]
assets = ["nas", "windows-rdp"]
```

## 校验、预览和提交

离线校验只检查语法、secret 和清单内部引用，不打开 SQLite：

```bash
hop-server config validate -f resources.yaml --offline
```

在线校验会同时检查当前 Catalog 中的引用和资源归属：

```bash
hop-server --config /etc/hop/config.toml \
  config validate -f resources.yaml
```

提交前查看 diff：

```bash
hop-server --config /etc/hop/config.toml \
  config diff -f resources.yaml --source home
```

执行原子 apply：

```bash
hop-server --config /etc/hop/config.toml \
  apply -f resources.yaml --source home
```

多个文件可以组成一个 apply scope。使用 glob 时应加引号，让 Hop 自己检查完整范围：

```bash
hop-server --config /etc/hop/config.toml \
  apply -f '/etc/hop/resources.d/*.yaml' --source home
```

任意资源校验失败时，整个 apply 不会写入。重复提交相同内容不会增加 Catalog revision。

## 热更新边界

| 变更 | 是否需要重启 |
|---|---|
| Access Key、白名单、凭据、SSH/TCP 资产 | 不需要，apply 或 watcher 成功后新连接使用新值 |
| `server`、`database`、`api`、`ssh`、`security`、`runtime` | 需要 |
| `inventory.sources` 本身 | 需要 |

普通资源变更不会主动中断已经建立的连接。紧急情况下，可以通过 Control API 的会话终止接口关闭指定活动会话。

## 配置文件权限

建议按用途设置权限：

| 文件 | 建议权限 |
|---|---|
| 启动配置、资源清单、公钥 | root 可写，Hop 服务用户可读 |
| 密码、目标私钥、API Token | 仅 root 和 Hop 服务用户可读 |
| SQLite、Master Key、Host Key | Hop 服务用户可读写 |

不要只备份 `hop.db`。恢复托管凭据需要与数据库匹配的 `master_key_file`。
