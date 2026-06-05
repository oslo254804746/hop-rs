# Hop — 轻量跳板机系统架构设计

## 设计哲学

- **SSH 进来就能用** — 公钥在白名单里就有权限，不搞内部用户/权限体系
- **TUI 是主界面** — 开发者日常使用 TUI 交互（fuzzy search → 连接 → 返回列表）
- **Web 只给管理员** — 管理资产、凭证、SSH 公钥白名单
- **端口分离** — 服务端口（SSH）对外暴露，管理端口仅内网/本地访问
- **单二进制，SQLite only，零依赖部署** — 对外只暴露 SSH 端口，管理端口默认仅本机/内网可见

## MVP 边界

MVP 只实现三件事：

1. 公钥白名单进入 hop
2. TUI 搜索资产并用服务器托管凭证连接目标主机
3. ProxyJump/ProxyCommand 作为纯 TCP 跳板，带资产 allowlist

明确推迟：TUI 文件浏览器、ZMODEM、细粒度权限、审批流、会话录像、TOTP、复杂前端 SPA。

## 架构概览

```
┌─────────────────────────────────────────────────────────────┐
│                         hop-server                            │
├────────────────────────────┬────────────────────────────────┤
│  SSH Server (对外)          │  Admin API (内网)               │
│  0.0.0.0:2222              │  127.0.0.1:8080                │
│                            │                                 │
│  验证 SSH 公钥 →            │  首次启动生成随机密码            │
│  白名单通过 → 启动 TUI      │  资产/凭证/公钥 CRUD            │
│  ProxyJump → 直通转发       │  连接日志查看                   │
└────────────────────────────┴────────────────────────────────┤
│                        hop-core                               │
│  SQLite · 配置 · 凭证加密 · 模型                              │
└─────────────────────────────────────────────────────────────┘
```

## 鉴权模型

```
                    ┌──────────────────┐
                    │   管理员          │
                    │   (Web 界面)      │
                    └────────┬─────────┘
                             │ 添加/移除
                             ▼
                    ┌──────────────────┐
                    │  SSH 公钥白名单    │  ← 数据库 authorized_keys 表
                    │  (公钥 + 备注名)  │
                    └────────┬─────────┘
                             │ 验证
                             ▼
          用户 SSH → hop:2222 → 公钥匹配？
                 ├─ Yes → 进入 TUI（可见所有资产）
                 └─ No  → 拒绝连接

管理界面认证：首次启动生成随机密码，只在控制台输出一次，不写入持久日志
```

### 未来扩展预留

MVP 不在 `authorized_keys` 里预留 `allowed_assets` JSON 字段。所有通过认证的公钥默认可见全部资产；如果未来需要细粒度权限，通过新迁移增加 `key_asset_grants` 这类关系表，而不是让一个未使用的 JSON 字段长期悬空。

## 技术栈

| 维度 | 选型 |
|------|------|
| 后端 | Rust (tokio + axum + russh) |
| TUI | ratatui + crossterm backend + termwiz InputParser |
| 模糊搜索 | nucleo |
| SSH | russh；russh-sftp 仅 Post-MVP 文件传输使用 |
| 数据库 | SQLite only (sqlx)，不支持其他 DB |
| Web 管理 | axum + server-rendered HTML (maud 或 askama)，不做 SPA |
| CLI | clap |
| 加密 | chacha20poly1305 + hkdf + sha2 (凭证加密)，argon2 (管理密码) |
| 配置 | toml + serde |
| 日志 | tracing |

版本基线以 `Cargo.lock` 为准。2026-06 初始实现建议从 `russh 0.61.x`、`ratatui 0.30.x`、`axum 0.8.x`、`sqlx 0.9.x` 起步；如果 `sqlx 0.9` 在实现期遇到生态兼容问题，可保守 pin 到 `0.8.6`。`anstyle` 只描述 ANSI 样式，不替代 SSH 场景下的输入解析；`tui-input` 可作为文本输入控件辅助，也不替代 raw byte parser。

## 项目结构

```
hop/
├── Cargo.toml                    # Workspace 根
├── config.example.toml
├── migrations/
│   └── 001_init.sql
├── crates/
│   ├── hop-core/                 # 模型、DB、配置、凭证加密
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── models.rs         # Asset, Credential, AuthorizedKey, Session
│   │       ├── db.rs             # SQLite CRUD
│   │       ├── config.rs         # TOML 配置
│   │       └── crypto.rs         # XChaCha20-Poly1305 凭证加密
│   ├── hop-server/               # 最终二进制：SSH + outbound client + TUI + Admin
│   │   └── src/
│   │       ├── main.rs           # 启动 SSH + Admin API
│   │       ├── ssh/
│   │       │   ├── server.rs     # russh server，公钥白名单验证
│   │       │   ├── client.rs     # outbound SSH client
│   │       │   ├── bridge.rs     # server channel ↔ client channel raw bridge
│   │       │   └── proxy.rs      # direct-tcpip 纯 TCP 转发
│   │       ├── tui/
│   │       │   ├── app.rs        # TUI 主循环
│   │       │   ├── input.rs      # termwiz InputParser 适配
│   │       │   └── views.rs      # 资产列表 + fuzzy search
│   │       └── admin/
│   │           ├── routes.rs     # 管理后台 CRUD
│   │           ├── auth.rs       # admin 密码认证
│   │           └── html.rs       # maud/askama 服务端渲染
└── docs/
```

MVP 不单独拆 `hop-ssh`、`hop-tui`、`hop-api` crate，先放在 `hop-server` 内部模块，避免 workspace 过早膨胀。等 API 边界稳定后再拆 crate。

## 数据库设计

SQLite only，共 6 张表：`authorized_keys`、`assets`、`credentials`、`sessions`、`known_hosts`、`settings`。

```sql
-- SSH 公钥白名单
CREATE TABLE authorized_keys (
    id             TEXT PRIMARY KEY,
    name           TEXT NOT NULL,          -- 备注名（如 "张三的 MacBook"）
    public_key     TEXT NOT NULL UNIQUE,   -- OpenSSH 格式公钥
    fingerprint    TEXT NOT NULL,          -- SHA256 指纹，用于快速匹配
    is_active      BOOLEAN NOT NULL DEFAULT TRUE,
    created_at     TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_authorized_keys_fingerprint ON authorized_keys(fingerprint);

-- 资产（目标主机）
CREATE TABLE assets (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL UNIQUE,   -- 别名，用于搜索和连接
    hostname      TEXT NOT NULL,          -- IP 或域名
    port          INTEGER NOT NULL DEFAULT 22,
    description   TEXT,
    tags          TEXT,                    -- JSON array: ["prod", "web"]
    credential_id TEXT REFERENCES credentials(id),
    created_at    TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at    TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- 凭证（连接目标的 SSH 密钥/密码）
CREATE TABLE credentials (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    username        TEXT NOT NULL,        -- 目标主机用户名
    auth_type       TEXT NOT NULL,        -- 'password' | 'key' | 'key+passphrase'
    password_enc    TEXT,                 -- 加密 envelope，见"凭证加密密钥生命周期"
    private_key_enc TEXT,                 -- 加密 envelope
    passphrase_enc  TEXT,                 -- 加密 envelope
    created_at      TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- 连接日志
CREATE TABLE sessions (
    id          TEXT PRIMARY KEY,
    key_finger  TEXT NOT NULL,            -- 连接者的公钥指纹
    key_name    TEXT,                     -- 连接者备注名
    mode        TEXT NOT NULL,            -- 'tui' | 'tui-connect' | 'direct' | 'proxyjump'
    asset_name  TEXT,
    target_host TEXT,
    target_port INTEGER,
    client_ip   TEXT,
    status      TEXT NOT NULL DEFAULT 'started', -- 'started' | 'ok' | 'failed'
    error       TEXT,
    started_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    ended_at    TIMESTAMP
);

-- 目标主机密钥（TOFU）
CREATE TABLE known_hosts (
    hostname    TEXT NOT NULL,
    port        INTEGER NOT NULL DEFAULT 22,
    key_type    TEXT NOT NULL,            -- "ssh-ed25519", "ssh-rsa" 等
    fingerprint TEXT NOT NULL,
    first_seen  TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (hostname, port, key_type)
);

-- 系统设置
CREATE TABLE settings (
    key         TEXT PRIMARY KEY,
    value       TEXT NOT NULL
);
-- 存储: admin_password_hash, first_run_completed 等
```

SQLite 运行策略：

- 启动时执行 `PRAGMA journal_mode = WAL`
- 设置 `busy_timeout`（如 5s），避免管理后台写入与会话日志写入轻微并发时直接失败
- 所有 schema 迁移随二进制打包，优先用 `sqlx::migrate!`；若最终部署需要省掉外部 `migrations/` 目录，也可用 `include_str!` 内嵌 SQL
- 不支持 MySQL/Postgres，避免配置矩阵扩大

## 配置文件

```toml
[server]
ssh_bind = "0.0.0.0:2222"          # 服务端口，对外暴露
admin_bind = "127.0.0.1:8080"      # 管理端口，仅内网/本地

[database]
path = "./hop.db"

[ssh]
host_key_file = "./hop_host_key"   # 不存在时自动生成 Ed25519 密钥
host_key_type = "ed25519"
banner = "Welcome to Hop"
keepalive_interval = 30            # 秒，双向 keepalive
connect_timeout = 10               # 秒，连接目标超时
proxy_policy = "assets_only"        # direct-tcpip 只能访问 assets 表命中的 host:port

[security]
# 凭证加密密钥，首次启动自动生成并写入此文件，权限必须为 0600
secret_key_file = "./hop.secret"

[runtime]
temp_dir = "/tmp/hop"               # Post-MVP 文件暂存使用，MVP 不依赖
```

## 使用方式

### 管理员：Web 管理后台
```
1. 首次启动 hop-server → 控制台输出一次随机管理密码
2. 浏览器打开 http://127.0.0.1:8080 → 输入密码
3. 添加资产（主机名、IP、凭证）
4. 添加 SSH 公钥到白名单（粘贴公钥 + 起个备注名）
```

如果管理员不在 hop 机器本机，使用 SSH 隧道访问管理后台：

```bash
ssh -L 8080:127.0.0.1:8080 hop-server -p 2222
```

### 开发者：SSH 进入 TUI
```bash
ssh -p 2222 hop-server              # 公钥认证通过 → 进入 TUI
```

### 开发者：资产名直连
```bash
ssh -p 2222 myserver@hop-server     # 公钥认证通过 → Hop 使用托管凭证进入资产
```

SSH username 会被解释为资产名或资产 hostname。进入 Hop 的身份仍然来自公钥 fingerprint，目标主机用户名来自资产绑定的托管凭证。

### 高级：ProxyJump 直通
```bash
ssh -J hop:2222 target-host         # hop 作为纯 TCP 跳板
scp -J hop:2222 ./file target-host:/tmp/
```

ProxyJump/ProxyCommand 模式不使用 hop 存储的目标凭证。用户本地 SSH 客户端必须能完成目标主机认证；hop 只负责验证进入 hop 的公钥、检查目标是否命中 assets allowlist、建立 TCP 转发和记录日志。

当 `host_to_connect` 是 `web-prod-01.hop` 时，hop 可去掉 `.hop` 后缀并按 asset name 查找真实 `hostname:port`；也可以直接接受 assets 表中的真实 `hostname:port`。两种形式都必须命中 assets 表，避免 hop 变成内网 open proxy。

### SSH Remote Command

Hop 不提供面向开发者的 SSH remote command API。任何 `ssh hop-server <command>` 请求都会返回不支持消息并以非零状态退出；开发者入口只保留 TUI、资产名直连和 ProxyJump。

## TUI 界面

```
┌─────────────────────────────────────────────────┐
│  Hop v0.1.0                          [?] Help   │
├─────────────────────────────────────────────────┤
│  > web_                                         │  ← 实时 fuzzy search
├─────────────────────────────────────────────────┤
│  ▸ web-prod-01    10.0.1.10   [prod][web]       │
│    web-prod-02    10.0.1.11   [prod][web]       │
│    web-staging    10.0.2.10   [staging][web]    │
│                                                  │
├─────────────────────────────────────────────────┤
│  Enter:连接  /:搜索  g:标签  q:退出              │
└─────────────────────────────────────────────────┘
```

文件浏览器不进入 MVP。文件传输优先使用标准 `scp -J` / `sftp -J`，但这属于纯 TCP 跳板模式，要求用户本地拥有目标主机认证能力。使用服务器托管凭证的文件传输另行设计。

## 关键技术决策

### TUI-over-SSH 桥接架构

ratatui/crossterm 通常直接操作本地终端，但 hop 的 TUI 运行在 SSH server 内部。桥接策略：

```
russh Channel (async)
    │
    ├─ 写方向 (server→client): 
    │   tokio::sync::mpsc → TUI 渲染线程 write 到 channel
    │   ratatui Backend 的 Write impl 写入 mpsc sender
    │   一个 tokio task 从 receiver 读取并调用 handle.data()
    │
    ├─ 读方向 (client→server):
    │   russh Handler::data() 回调收到原始字节
    │   不能用 crossterm::event::read()（它读 stdin）
    │   需要自己解析 ANSI 转义序列为按键事件
    │   可用 termwiz 的 InputParser 或手写解析器
    │
    └─ PTY Resize:
        russh Handler::window_change_request() 回调
        通过 channel 通知 TUI 更新 terminal size
```

**实现要点：**
- TUI 渲染跑在独立的 blocking thread（`tokio::task::spawn_blocking`）
- 用 `tokio::io::DuplexStream` 或 mpsc 通道桥接 async↔sync
- 输入解析用 `termwiz::input::InputParser` 替代 crossterm 的事件读取
- 终端大小变更通过共享的 `Arc<AtomicU16>` 或 watch channel 传递
- Ctrl+C 在 TUI 前台时按普通按键处理，不能让 hop 进程退出；进入目标会话 raw bridge 后再原样转发给目标 SSH channel
- TUI 进入目标会话前退出 alternate screen/清屏，目标断开后重新初始化 ratatui backend，避免目标程序残留 ANSI 状态污染资产列表

### ProxyJump 语义

ProxyJump 模式是**纯 TCP 转发**（标准行为）：
- hop 收到 `direct-tcpip` 请求，只做 TCP 连接转发
- **不做凭证查找，不做认证代理**
- 用户的 SSH 客户端自己通过隧道认证目标主机
- hop 必须检查目标地址是否在 assets 表中，MVP 不允许任意内网地址转发

凭证查找只在 TUI 或资产名直连这种 server-side managed connection 中使用。`ssh -J`、`scp -J`、`sftp -J` 永远不使用 hop 托管凭证。

资产匹配规则：

1. `host_to_connect:port` 直接命中 `assets.hostname:assets.port`
2. `host_to_connect` 命中 `assets.name`
3. `host_to_connect` 形如 `<asset>.hop`，去掉 `.hop` 后命中 `assets.name`

不命中则拒绝 `direct-tcpip`，并写入 sessions 日志，避免 hop 变成内网 open proxy。

### 目标主机密钥验证

outbound SSH 连接（hop→目标）的 host key 验证策略：**TOFU (Trust On First Use)**
- 首次连接目标：接受并存储 host key fingerprint（存入 SQLite `known_hosts` 表）
- 后续连接：校验 fingerprint 是否匹配
- 不匹配时拒绝连接并在 TUI 显示警告
- 管理后台可查看/清除已知指纹

```sql
CREATE TABLE known_hosts (
    hostname    TEXT NOT NULL,
    port        INTEGER NOT NULL DEFAULT 22,
    key_type    TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    first_seen  TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (hostname, port, key_type)
);
```

### 会话模型

TUI 中选择目标连接时：
1. TUI **退出前台**，终端完全交给目标 SSH 会话（raw terminal bridge）
2. 用户在目标上的操作直接通过 hop 桥接，hop 不解析中间数据
3. 用户断开目标连接（exit/Ctrl-D）后，**自动返回 TUI 界面**
4. 连接失败时显示错误信息，按任意键返回资产列表

### 文件传输实现

MVP 不实现 TUI 文件浏览器，也不实现 ZMODEM。原因：

- TUI 文件浏览器需要 SFTP 客户端、远端目录 UI、上传下载状态、错误恢复，工作量接近一个极简文件管理器
- 暂存方案并不天然成立：用户执行 `scp hop:/tmp/hop-downloads/file .` 时，hop 自身还必须实现 SFTP subsystem 或 scp exec，否则文件取不回来
- 标准 `scp -J` / `sftp -J` 已能覆盖“用户本地有目标凭证”的场景

Post-MVP 再设计 server-managed 文件传输：

- **下载**：hop 用托管凭证从目标 SFTP 读取 → hop 暂存 → 通过 hop 自己的 SFTP subsystem 或本地 CLI 拉回
- **上传**：本地 CLI 推到 hop 暂存 → hop 用托管凭证写入目标
- **ZMODEM**：只作为可选增强，不作为默认路径；不同终端支持度不稳定

### 凭证加密密钥生命周期

- `hop.secret` 文件存储一个随机生成的 32 字节 master key（base64 编码）
- 首次启动由 `OsRng` 生成，写入文件，Linux 权限强制为 `0600`
- 凭证加密使用 HKDF-SHA256 从 master key 派生出 per-credential 的加密密钥
- 加密算法默认 XChaCha20-Poly1305；如果未来需要 FIPS 合规，再增加 AES-GCM 作为可选算法
- 每个加密字段存储完整 envelope：`v1:xchacha20poly1305:<nonce_b64>:<ciphertext_b64>`
- nonce 每次加密随机生成，严禁复用；认证 tag 包含在 AEAD ciphertext 中
- **如果 hop.secret 丢失，所有存储的凭证不可恢复** — 文档需明确告知用户备份此文件
- 不支持密钥轮换（极简设计）

### 管理密码存储与重置

- 首次启动生成随机密码，argon2 哈希后存入 SQLite `settings` 表
- 终端打印明文密码（仅一次）
- 重置方式：`hop-server reset-admin` 子命令 → 生成新密码并打印
- 或者删除数据库中 settings 记录 → 下次启动重新生成
- Admin API 默认只绑定 `127.0.0.1:8080`；远程管理通过 SSH tunnel，不直接对公网开放
- TOTP 不进入 MVP。若后续需要增强后台登录，可优先做 `hop-server admin-token --ttl 10m` 这种一次性临时登录 token，再考虑长期 TOTP 配置

### 资产-凭证关系

- 一个 asset 对应一个 credential（schema: `credential_id` 外键）
- 多个 asset 可共享同一个 credential
- 如果同一台机器需要不同用户登录，创建多个 asset 条目（如 `web-prod-root`、`web-prod-deploy`）
- `credential_id` 允许为空：这类资产只能用于 ProxyJump/ProxyCommand allowlist，不能通过 TUI 使用托管凭证连接
- 初期不做一对多，保持简单

### 目标部署平台

- **Server 运行环境：Linux (x86_64/aarch64)**
- 开发可在 Windows/macOS 进行，但 hop-server 设计为 Linux 部署
- PTY 处理、信号处理等使用 Unix API
- 为了匹配“单二进制、零依赖部署”，SQLite 采用 bundled 构建路径；不要启用动态链接宿主 `libsqlite3.so` 的配置
- release profile 放在 `Cargo.toml`，建议：`opt-level = "z"`、`lto = true`、`codegen-units = 1`、`panic = "abort"`

## SSH Server 内部流程

```
用户 SSH 连接 hop:2222
  → russh server 握手
  → 公钥认证：计算 fingerprint → 查 authorized_keys 表
     ├─ 匹配且 is_active = true → 认证通过
     └─ 不匹配 → 拒绝
  → 判断请求类型：
     ├─ shell/pty 请求 → 启动 TUI（传入 PTY handle）
     ├─ direct-tcpip (ProxyJump/ProxyCommand -W) → 查 asset allowlist → 纯 TCP 桥接
     └─ exec 请求 → 返回不支持 SSH remote commands，并以非零状态退出
```

## 关键依赖

| 用途 | Crate |
|------|-------|
| 异步运行时 | tokio |
| SSH server/client | russh, russh-sftp |
| TUI 框架 | ratatui, crossterm |
| SSH 输入解析 | termwiz (InputParser，替代 crossterm 事件读取) |
| 模糊搜索 | nucleo |
| HTTP 框架 | axum |
| 数据库 | sqlx (sqlite) |
| CLI 解析 | clap |
| 凭证加密 | chacha20poly1305, hkdf, sha2 |
| 配置解析 | toml, serde |
| 日志 | tracing, tracing-subscriber |
| UUID | uuid |
| 密码哈希 | argon2 (admin 密码) |
| HTML 渲染 | maud 或 askama |

SQLite 零依赖部署要求使用 bundled SQLite。实现时确认 `sqlx`/`libsqlite3-sys` 的 feature 组合走静态 bundled 路径；如果选择 `sqlite-unbundled`，就违背了单二进制零依赖目标。

## 实现阶段

1. **Core + DB** — 模型、SQLite 迁移、配置解析、凭证加密 envelope、WAL/busy_timeout
2. **最小管理入口** — `reset-admin`、本机 CRUD CLI 或极简 Admin API，用来录入公钥/资产/凭证
3. **SSH Server** — russh 服务端、公钥白名单认证、PTY/shell/direct-tcpip 路由，exec 请求统一拒绝
4. **Outbound SSH + Raw Bridge** — 先用硬编码/DB asset 打通 hop server channel ↔ 目标 SSH channel，覆盖 resize、Ctrl+C、EOF、exit status
5. **TUI** — ratatui 资产列表 + fuzzy search + 键盘交互，Enter 后切入 raw bridge，断开后返回 TUI
6. **ProxyJump/ProxyCommand** — direct-tcpip 纯 TCP 转发 + assets allowlist + 日志，支持 `<asset>.hop`
7. **Admin Web** — axum + server-rendered HTML 管理资产/凭证/公钥/known_hosts/sessions
8. **Polish** — 错误处理、首次启动引导、Docker、文档、release profile、备份提示
9. **Post-MVP File Transfer** — SFTP subsystem / 暂存目录 / TUI 文件浏览器 / ZMODEM 可选增强

## 验证方式

1. `hop-server` 首次启动 → 终端打印管理密码 → Web 可登录
2. Web 添加 SSH 公钥 + 资产
3. `ssh hop -p 2222` → 进入 TUI → 看到资产列表
4. TUI 中搜索并 Enter 连接目标
5. 目标连接退出后自动返回 TUI，终端状态正常
6. `ssh -p 2222 web-prod-01@hop` 资产名直连，使用 Hop 托管凭证进入目标
7. `ssh -J hop:2222 target` ProxyJump 直通，仅当 target 命中 assets allowlist
8. `scp -J hop:2222` 在用户本地具备目标凭证时正常工作
