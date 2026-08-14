# Hop

Hop v0.2 是一个面向个人开发者、Homelab 和单一管理信任域小团队的轻量 SSH 跳板机。它使用原生 OpenSSH 客户端，覆盖 TUI、资产直连、远程命令、SCP/SFTP、ProxyJump 和通用 TCP 转发；不要求浏览器、专属客户端、外部数据库或容器运行时。

[English](README-EN.md)

## v0.2 边界

- 单一 Rust 二进制和单一 SQLite Catalog。
- 多把入口 Access Key，可独立启用、禁用或删除。
- Key 未声明 `assets` 时访问全部当前及未来资产；`assets: []` 表示认证成功但不能发现或访问资产；非空数组是严格白名单。
- YAML/TOML、CLI 和可选 Control API 都写入同一个 Catalog。
- HTTP Control API 默认关闭；核心不包含 Admin Web、管理员账号、Cookie、CSRF、角色或 RBAC。
- OpenWrt 是一等发行目标：轻量 LuCI/procd 包按架构下载经 SHA256 校验的静态核心，不在 IPK/APK 内编译或内置 Rust 后端。
- 资产类型只有 `ssh` 和 `tcp`。SSH 由 Hop 管理协议与凭据，RDP、数据库等服务使用通用 TCP 转发。

## 快速开始

```bash
cargo build --release --locked -p hop-server
cp config.example.toml config.toml
```

启动配置也可以使用 [`config.example.yaml`](config.example.yaml)。[`resources.example.yaml`](resources.example.yaml) 提供了可以直接离线校验的 SSH/TCP 资源样例。

添加入口公钥、目标凭据和资产：

```bash
./target/release/hop-server --config config.toml key add \
  --name laptop --public-key-file ~/.ssh/id_ed25519.pub

printf '%s' 'target-password' | \
  ./target/release/hop-server --config config.toml credential add \
  --name homelab-root --username root --auth-type password --password-stdin

credential_id=$(./target/release/hop-server --config config.toml credential list | awk 'NR == 1 { print $1 }')
./target/release/hop-server --config config.toml asset add \
  --name nas --hostname 192.168.1.20 --port 22 --credential-id "$credential_id"

./target/release/hop-server --config config.toml serve
```

默认 SSH 监听 `0.0.0.0:2222`，Control API 不启动。

## 原生 SSH 使用

```bash
# TUI 资产选择器
ssh -p 2222 menu@hop-host

# 资产名直连与远程命令
ssh -p 2222 nas@hop-host
ssh -p 2222 nas@hop-host 'uname -a'

# SCP / SFTP 使用 Hop 托管的目标凭据
scp -P 2222 file.txt nas@hop-host:/tmp/file.txt
sftp -P 2222 nas@hop-host

# 将已登记的 RDP 资产映射到本机
ssh -p 2222 -L 13389:desktop.hop:3389 menu@hop-host
```

ProxyJump 示例：

```sshconfig
Host hop
  HostName hop-host
  Port 2222
  IdentityFile ~/.ssh/id_ed25519

Host *.hop
  ProxyJump hop
```

Hop 在 TUI 列表、资产名直连、托管交互会话、远程命令、SCP/SFTP、ProxyJump 和 TCP 转发入口使用同一 Key-to-Asset 授权查询；手工构造目标不会绕过白名单。普通白名单变更只影响新连接，紧急阻断使用 Control API 的显式会话终止接口。

## 声明式资源

资源 manifest 支持严格 YAML 或 TOML，拒绝未知字段和重复资源。Secret 默认只允许 `file` 或 `env` 来源，解析后使用 Master Key 加密写入 SQLite。

```yaml
api_version: hop/v1alpha1

credentials:
  homelab:
    type: password
    username: root
    password:
      file: /etc/hop/secrets/homelab.password

assets:
  nas:
    type: ssh
    host: 192.168.1.20
    port: 22
    credential: homelab

  desktop:
    type: tcp
    host: 192.168.1.30
    port: 3389

access:
  laptop:
    public_key:
      file: /etc/hop/keys/laptop.pub
    assets: [nas, desktop]
```

```bash
# 纯离线语法、secret 和引用校验
hop-server config validate -f resources.yaml --offline --json

# 针对现有 Catalog 的只读校验与 diff
hop-server --config config.toml config validate -f resources.yaml --json
hop-server --config config.toml config diff -f resources.yaml --source home --json

# 原子提交；可选 revision 乐观锁、dry-run 和显式 prune
hop-server --config config.toml apply -f resources.yaml \
  --source home --base-revision 0 --json
hop-server --config config.toml apply -f resources.yaml --source home --dry-run
hop-server --config config.toml apply -f resources.yaml --source home --prune

hop-server --config config.toml config status --json
```

同一内容重复 apply 不增加 Catalog revision。完整 source 扫描中缺失的资源默认只标记 orphan 并继续可用；只有显式 `state: absent` 或 `--prune` 才删除。启动配置中的 watcher 同样调用这套 Apply engine，等待 scope 稳定后重扫；错误只记录状态并保留上一代有效 Catalog，且默认不 prune。

## 启动配置与 Control API

`config.example.toml` 展示所有启动边界：SSH listen、数据库、Host Key、Master Key、超时、keepalive、banner、proxy policy、API、inventory source/watcher 和运行设置。YAML 配置使用相同字段。

Catalog 资源可以动态 apply；监听地址、数据库和密钥路径等启动字段需要重启；白名单和凭据变化只影响新连接。

启用 API 时必须显式创建 Token 文件：

```toml
[api]
enabled = true
listen = "127.0.0.1:8083"
token_file = "/etc/hop/control-api.token"
cors_allowlist = []
```

所有 `/api/v1` 请求使用 `Authorization: Bearer <token>`。API 提供状态、Catalog revision、资源和本地 CRUD、会话终止、source/status、validate、diff、apply 与 reload；凭据响应只包含 `configured`/`missing` 状态，不返回密文或明文。非 loopback 监听必须配置明确的 CORS allowlist。

## 验证

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
./scripts/manual_e2e_key_asset_access.sh
./scripts/e2e_local_openssh.sh
```

第二个 E2E 会启动隔离的 OpenSSH 目标，验证远程命令、stdin/stdout/stderr、退出码、PTY、SCP、SFTP、ProxyJump、Host Key 首次记录与变更拒绝，以及凭据密文。

## 文档

- [SSH 与 TCP 代理](docs/proxying.zh-CN.md)
- [配置参考](docs/configuration.zh-CN.md)
- [部署指南](docs/deployment.zh-CN.md)
- [Deployment, backup, rebuild, and recovery](docs/deployment.md)
- [Control API 与本地管理](docs/admin-guide.zh-CN.md)
- [声明式 Apply 规范](docs/product/declarative-apply-spec.md)
- [Access Key 与资产白名单](docs/product/lightweight-access-control.md)
- [OpenWrt 打包与资源测量](docs/openwrt.md)
- [v0.2 产品方向](docs/product/product-direction-v0.2.md)
- [文档索引](docs/README.md)

## License

MIT
