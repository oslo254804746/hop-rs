# Hop 单 YAML 配置参考

Hop 使用一份严格的 YAML（也兼容 `.toml`）同时配置进程运行参数和启动 Catalog。未知字段、错误类型、交叉引用失败或不合法公钥都会在启动监听器之前报错。

## 完整示例

```yaml
listen: 0.0.0.0:2222
data_dir: ./data

api:
  enabled: true
  listen: 127.0.0.1:8083
  token: change-me
  cors_allowlist: []

ssh:
  host_key_type: ed25519
  banner: Welcome to Hop
  keepalive_interval: 30
  connect_timeout: 10
  proxy_policy: assets_only

runtime:
  temp_dir: /tmp/hop
  log_level: info
  session_retention_days: 30

credentials:
  password-login:
    username: root
    password: replace-this-password
  key-login:
    username: deploy
    private_key: |
      -----BEGIN OPENSSH PRIVATE KEY-----
      replace-this-key
      -----END OPENSSH PRIVATE KEY-----
    passphrase: replace-this-passphrase

assets:
  nas:
    type: ssh
    host: 192.168.1.20
    port: 22
    display_name: Home NAS
    description: Storage host
    credential: password-login
  metrics:
    type: tcp
    host: 192.168.1.30
    port: 9090

access_keys:
  laptop:
    public_key_file: ./keys/laptop.pub
    enabled: true
    assets: [nas]
  operator:
    public_key: "ssh-ed25519 AAAAC3... operator@device"
```

运行：

```bash
chmod 0600 hop.yaml
hop-server --config ./hop.yaml config validate
hop-server --config ./hop.yaml serve
```

所有相对路径（`data_dir`、`runtime.temp_dir`、`public_key_file`）均以配置文件所在目录解析，而不是当前工作目录。

## 顶层字段

| 字段 | 默认值 | 说明 |
|---|---|---|
| `listen` | `0.0.0.0:2222` | 入口 SSH 监听地址 |
| `data_dir` | `.` | 数据库 `hop.db`、加密主密钥 `hop.secret`、主机密钥 `hop_host_key` 的目录 |
| `api` | 禁用 | Control API 与网页管理认证 |
| `ssh` | 见下表 | SSH 运行参数 |
| `runtime` | 见下表 | 临时目录、日志与会话保留参数 |
| `credentials` | `{}` | 以名称为键的目标凭据 |
| `assets` | `{}` | 以名称为键的 SSH/TCP 资产 |
| `access_keys` | `{}` | 以名称为键的入口公钥 |

## `api`

| 字段 | 默认值 | 说明 |
|---|---|---|
| `enabled` | `false` | 是否尝试启动 Control API |
| `listen` | `127.0.0.1:8083` | API 监听地址 |
| `token` | 缺失 | 直接 Bearer Token 字符串 |
| `cors_allowlist` | `[]` | 仅供浏览器直接跨域访问的 Origin 列表 |

即使 `enabled: true`，`token` 缺失、空字符串或只有空白时也只会禁用 API，SSH 不受影响。`change-me` 会启动 API，但打印不包含 Token 内容的安全警告。Token 的 Debug 表示始终为 `[REDACTED]`，不会出现在日志、API 响应或面板构建产物中。

Compose 面板采用同源反向代理，`cors_allowlist` 保持空数组即可。直接跨域时必须填写完整 Origin，例如：

```yaml
api:
  enabled: true
  listen: 0.0.0.0:8083
  token: replace-me
  cors_allowlist:
    - https://panel.example.com
```

裸 IP `192.0.2.10`、裸主机名 `panel.example.com`、含路径或凭据的 URL 都会以 `api.cors_allowlist[index]` 报错。`"*"` 支持任意 Origin，但必须是唯一条目。

## `ssh`

| 字段 | 默认值 | 约束 |
|---|---|---|
| `host_key_type` | `ed25519` | 当前只能是 `ed25519` |
| `banner` | `Welcome to Hop` | TUI 欢迎语 |
| `keepalive_interval` | `30` | 秒 |
| `connect_timeout` | `10` | 目标连接超时秒数 |
| `proxy_policy` | `assets_only` | 当前只能是 `assets_only` |

入口认证只宣告公钥，不提供 password 或 keyboard-interactive 回退。

## `runtime`

| 字段 | 默认值 | 说明 |
|---|---|---|
| `temp_dir` | `/tmp/hop` | 临时文件目录 |
| `log_level` | `info` | 日志级别 |
| `session_retention_days` | `30` | 会话记录保留天数 |

## `credentials`

每项必须包含 `username`，并在 `password` 与 `private_key` 中二选一。`passphrase` 只能与 `private_key` 一起使用。

三个 secret 字段都直接保存字符串，不接受环境变量/文件/命令联合类型。YAML 文件包含真实目标机 secret，请始终使用 `0600` 权限、受控备份和 secret 扫描排除规则。

## `assets`

| 字段 | 必需性 | 说明 |
|---|---|---|
| `type` | 可选 | `ssh`（默认）或 `tcp` |
| `host` | 必需 | 主机名或 IP |
| `port` | SSH 可选、TCP 必需 | SSH 默认 22 |
| `display_name` | 可选 | 展示名称 |
| `description` | 可选 | 描述 |
| `credential` | 可选 | 必须引用同文件 `credentials` 中的名称；TCP 不允许凭据 |

## `access_keys`

每项必须在 `public_key` 与 `public_key_file` 中二选一：

- `public_key`：完整 OpenSSH 公钥行；
- `public_key_file`：读取公钥文件，支持相对路径；
- `enabled`：可选，默认启用；
- `assets`：省略表示所有资产，空数组表示拒绝所有资产，非空数组表示仅允许列出的资产。

Hop 只保存公钥及其指纹，不生成或返回入口私钥。

## 启动应用与归属

配置在每次启动时通过内部原子 Apply 引擎应用：

- 全部验证成功后才提交；失败时数据库保持不变；
- 重复启动同一文件是幂等的；
- 修改 secret 或主机字段会更新相应资源；
- 从 YAML 移除的配置归属资源会删除；
- 面板/本地创建的 `local` 资源不会被删除。

Control API 列表响应只公开 `ownership: "local" | "config"`，不公开内部管理标识。
