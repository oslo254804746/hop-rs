# Hop

Hop 是一个轻量级 SSH 跳板机 MVP。它把 SSH 公钥白名单、TUI 资产选择、服务器托管凭证连接目标主机，以及受资产 allowlist 限制的 ProxyJump/ProxyCommand 放在一个 Rust 服务里。

当前目标是保持部署简单：单服务、SQLite only、默认只暴露 SSH 服务端口，管理端口只绑定本机 loopback。

## 能力边界

已纳入 MVP：

- SSH 公钥白名单进入 Hop。
- SSH-over-TUI 资产搜索与连接。
- 服务器托管目标凭证，用于 TUI 或资产名直连。
- ProxyJump/ProxyCommand 纯 TCP 转发，并且只允许命中资产表的目标。
- SQLite 存储，凭证加密保存。
- 本机 `hop-server` 管理 CLI。

暂不纳入 MVP：TUI 文件浏览器、ZMODEM、细粒度资产授权、TOTP、审批流、会话录像和 SPA 前端。

## 快速验证

开发环境先跑：

```bash
cargo test --workspace
cargo build --workspace
```

Linux 发布构建：

```bash
cargo build --release -p hop-server
```

部署说明见 [docs/deployment.md](docs/deployment.md)，包含二进制直部署、systemd、Docker 部署、升级、备份和排障。

## 首次运行

```bash
cp config.example.toml config.toml
hop-server serve --config config.toml
```

首次启动时，Hop 会自动创建：

- SQLite 数据库：`database.path`
- Hop SSH host key：`ssh.host_key_file`
- 凭证加密主密钥：`security.secret_key_file`
- 初始管理员密码：只打印一次到终端或日志

默认端口：

- SSH 服务：`0.0.0.0:2222`
- Admin Web：`127.0.0.1:8080`

Admin Web 在 MVP 中强制绑定 loopback。远程访问时，请通过宿主机系统 SSH 或管理网络建立隧道，不要把 Admin Web 直接暴露到公网。

## 本机管理 CLI

在 Admin Web 完整录入数据前，可以直接在服务器上使用 `hop-server` 管理数据：

先分清两类认证数据：

- `key add` 添加的是“谁可以登录 Hop 的 2222 端口”。Hop 入口只接受 SSH 公钥白名单认证，不使用密码登录。
- `credential add` 添加的是“Hop 服务器连接目标资产时用什么用户名/密码或私钥”。它只在 TUI 选择资产或资产名直连时使用，不是 Hop 入口登录密码。

```bash
hop-server --config config.toml reset-admin

hop-server --config config.toml key add \
  --name "alice laptop" \
  --public-key-file ~/.ssh/id_ed25519.pub

printf '%s' 'secret' | hop-server --config config.toml credential add \
  --name deploy-password \
  --username deploy \
  --auth-type password \
  --password-stdin

hop-server --config config.toml asset add \
  --name web-prod-01 \
  --hostname 10.0.1.10 \
  --port 22 \
  --tags prod,web \
  --credential-id <credential-id>
```

列出凭证不会输出解密后的密码、私钥或 passphrase。生产环境优先使用 `--password-stdin`，避免把密码暴露在进程参数里。

## 开发者使用

Hop 入口身份只由管理员预先信任的 SSH 公钥决定；`credentials` 中保存的用户名、密码或私钥只用于 Hop 服务器侧连接目标资产，不用于登录 Hop 本身。

进入 Hop TUI：

```bash
ssh -p 2222 hop-host
```

用资产名作为 SSH username，直接进入某个服务器托管资产：

```bash
ssh -p 2222 web-prod-01@hop-host
```

通过 ProxyJump 使用 Hop 做纯 TCP 跳板：

```bash
ssh -J hop-host:2222 web-prod-01.hop
```

如果看到 `Permission denied (publickey...)`，先确认自己的公钥已经通过 `hop-server key add` 加入 Hop 的 authorized keys，并处于 active 状态。`credentials` 里的用户名和密码不会用于登录 Hop；Hop 入口认证始终以公钥 fingerprint 为准。

## Managed Connection 与 ProxyJump

TUI 中按 Enter 连接资产、以及资产名直连都属于服务器托管连接：Hop 会解密资产凭证，并从服务器侧发起到目标主机的 SSH 连接。

直连模式也属于服务器托管连接，适合跳过 TUI 直接进入某个资产：

```bash
ssh -p 2222 <asset_name>@hop-host
ssh -p 2222 <asset_hostname>@hop-host
```

其中 SSH username 会被 Hop 解释为直连目标，`<asset_name>` 或 `<asset_hostname>` 必须命中资产表，且该资产需要绑定托管凭据。Hop 入口身份仍由公钥 fingerprint 决定，审计中的 `key_name` 来自 authorized key 记录。直连会在会话审计中记录为 `mode=direct`。

`ssh -J hop:2222 target` 和 `ProxyCommand -W` 属于纯 TCP 转发：Hop 只检查目标是否命中资产 allowlist，不会使用托管凭证。用户本地 SSH 客户端必须能自行完成目标主机认证。

ProxyJump allowlist 支持：

- `assets.hostname:assets.port`
- `assets.name`，转发到该资产保存的 `hostname:port`
- `<asset>.hop`，去掉 `.hop` 后按资产名解析

## 备份

请把下面三个文件作为同一批次备份：

- SQLite 数据库
- `hop.secret`
- Hop SSH host key

如果 `hop.secret` 丢失，已保存的目标凭证无法恢复。

## 项目结构

```text
crates/hop-core/      配置、模型、SQLite、凭证加密
crates/hop-server/    SSH 服务、TUI、Admin Web、本机管理 CLI
migrations/           SQLite schema
systemd/              systemd service 示例
docs/                 设计与部署文档
```
