# Hop 部署指南

Hop 可以通过 Docker、Linux 二进制或 OpenWrt 软件包运行。先用 Docker 验证连接最省事；长期运行建议使用静态二进制和 systemd。路由器部署使用独立的 `luci-app-hop` 软件包。

## 部署前准备

开始前确认以下信息：

- 一把用于登录 Hop 的 SSH 公钥。
- Hop 服务器到目标内网资产的网络连通性。
- 一个对客户端开放的 SSH 端口，默认是 `2222`。
- 如果使用托管 SSH，需要目标服务器的密码或私钥。

RDP、数据库和其他 TCP 服务不需要把业务凭据交给 Hop。Hop 只转发 TCP 流量，用户仍通过原生客户端登录目标服务。

## 快速试用：Docker

从源码构建镜像：

```bash
git clone https://github.com/oslo254804746/hop-rs.git
cd hop-rs
docker build -t hop:local .
mkdir -p data
docker run -d --name hop --restart unless-stopped \
  -p 2222:2222 \
  -v "$PWD/data:/data" \
  hop:local
```

容器第一次启动时会在 `/data` 中创建启动配置、SQLite、Master Key 和 SSH Host Key。Control API 默认关闭，容器只发布 SSH 端口。

添加入口公钥：

```bash
docker exec -u hop hop hop-server --config /data/config.toml key add \
  --name laptop \
  --public-key "$(cat ~/.ssh/id_ed25519.pub)"
```

添加一个使用密码的 SSH 目标：

```bash
printf '%s' 'target-password' | docker exec -i -u hop hop \
  hop-server --config /data/config.toml credential add \
  --name nas-root \
  --username root \
  --auth-type password \
  --password-stdin

credential_id=$(docker exec -u hop hop \
  hop-server --config /data/config.toml credential list | awk 'NR == 1 { print $1 }')

docker exec -u hop hop hop-server --config /data/config.toml asset add \
  --name nas \
  --protocol ssh \
  --hostname 192.168.1.20 \
  --port 22 \
  --credential-id "$credential_id"
```

连接 Hop：

```bash
ssh -p 2222 menu@127.0.0.1
ssh -p 2222 nas@127.0.0.1
```

添加 TCP 资产时不需要凭据：

```bash
docker exec -u hop hop hop-server --config /data/config.toml asset add \
  --name windows-rdp \
  --protocol tcp \
  --hostname 192.168.1.30 \
  --port 3389

ssh -N -T -p 2222 \
  -L 13389:windows-rdp.hop:3389 \
  menu@127.0.0.1
```

测试完成后，RDP 客户端连接 `127.0.0.1:13389`。

## 推荐部署：Linux 二进制和 systemd

### 1. 下载并校验二进制

官方 Release 提供 `x86_64` 和 `aarch64` 的静态构建。选择当前架构：

```bash
case "$(uname -m)" in
  x86_64|amd64) hop_arch=x86_64 ;;
  aarch64|arm64) hop_arch=aarch64 ;;
  *) echo "unsupported architecture" >&2; exit 1 ;;
esac

hop_asset="hop-server-linux-${hop_arch}-musl.tar.gz"
hop_release="https://github.com/oslo254804746/hop-rs/releases/latest/download"

curl -fLO "$hop_release/SHA256SUMS"
curl -fLO "$hop_release/$hop_asset"
grep " $hop_asset$" SHA256SUMS | sha256sum -c -
tar -xzf "$hop_asset" hop-server
./hop-server --version
sudo install -m 0755 hop-server /usr/local/bin/hop-server
```

其他架构可以从源码构建：

```bash
cargo build --release --locked -p hop-server
sudo install -m 0755 target/release/hop-server /usr/local/bin/hop-server
```

### 2. 创建服务用户和目录

```bash
sudo useradd --system --home /var/lib/hop --shell /usr/sbin/nologin hop
sudo install -d -o hop -g hop -m 0700 /var/lib/hop
sudo install -d -o root -g hop -m 0750 \
  /etc/hop /etc/hop/resources.d /etc/hop/secrets /etc/hop/keys
```

如果系统中已经有 `hop` 用户，可以跳过 `useradd`。

### 3. 创建启动配置

将以下内容保存为 `/etc/hop/config.toml`：

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

[[inventory.sources]]
id = "home"
path = "/etc/hop/resources.d/*.yaml"
watch = true
prune = false

[runtime]
temp_dir = "/tmp/hop"
log_level = "info"
session_retention_days = 30
```

```bash
sudo chown root:hop /etc/hop/config.toml
sudo chmod 0640 /etc/hop/config.toml
```

字段说明见 [配置参考](configuration.zh-CN.md)。

### 4. 创建资源清单

将入口公钥和目标私钥放到 Hop 可以读取的位置：

```bash
sudo install -m 0644 ~/.ssh/id_ed25519.pub /etc/hop/keys/laptop.pub
sudo install -m 0640 -o root -g hop ~/.ssh/nas_ed25519 /etc/hop/secrets/nas-root
```

将以下内容保存为 `/etc/hop/resources.d/home.yaml`：

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
    credential: nas-root

  windows-rdp:
    type: tcp
    host: 192.168.1.30
    port: 3389

access:
  laptop:
    public_key:
      file: /etc/hop/keys/laptop.pub
```

```bash
sudo chown root:hop /etc/hop/resources.d/home.yaml
sudo chmod 0640 /etc/hop/resources.d/home.yaml
sudo -u hop hop-server config validate \
  -f /etc/hop/resources.d/home.yaml --offline
```

上面的 Access Key 没有设置 `assets`，因此可以访问全部当前及未来资产。如需限制范围，请阅读 [配置参考](configuration.zh-CN.md#access)。

### 5. 安装并启动 systemd 服务

仓库中的 [systemd/hop.service](../systemd/hop.service) 可以直接使用：

```bash
sudo install -m 0644 systemd/hop.service /etc/systemd/system/hop.service
sudo systemctl daemon-reload
sudo systemctl enable --now hop
sudo systemctl status hop
sudo journalctl -u hop -f
```

如果只下载了 Release 二进制，可以从仓库取得同一 service 文件，或按以下运行参数创建服务：

```text
User=hop
Group=hop
WorkingDirectory=/var/lib/hop
ExecStart=/usr/local/bin/hop-server --config /etc/hop/config.toml serve
```

### 6. 验证连接

```bash
ssh -p 2222 menu@hop.example.com
ssh -p 2222 nas@hop.example.com
ssh -p 2222 nas@hop.example.com 'uname -a'
```

如果服务器启用了防火墙，只开放 SSH 监听端口即可。Control API 未启用时不需要开放 `8083`。

## OpenWrt

OpenWrt 使用独立的 `luci-app-hop` 软件包。软件包提供 LuCI 页面、procd 服务和按架构下载核心的工具，不在路由器上编译 Rust。

安装 `.ipk` 或 `.apk` 后，在 LuCI 的 `Services -> Hop` 中启用服务。命令行写法：

```sh
uci set hop.main.enabled='1'
uci commit hop
/etc/init.d/hop enable
/etc/init.d/hop start
logread -e hop
```

第一次启用时，软件包会下载与路由器架构匹配的 Hop 核心并校验 SHA256。详细设置和资源清单示例见 [`luci-app-hop` 配置文档](https://github.com/oslo254804746/luci-app-hop/blob/dev/docs/configuration.zh-CN.md)。

## Control API

核心 SSH 与 TCP 代理不依赖 HTTP。只有外部面板或自动化需要管理 Hop 时才启用 Control API。

创建 Token：

```bash
umask 077
openssl rand -hex 32 | sudo tee /var/lib/hop/control-api.token >/dev/null
sudo chown hop:hop /var/lib/hop/control-api.token
```

修改启动配置后重启 Hop。建议保留 `127.0.0.1:8083`，通过本地代理或带 TLS 和认证的反向代理访问。接口说明见 [Control API 与本地管理](admin-guide.zh-CN.md)。

## 更新

更新二进制前先备份数据并保留旧二进制：

```bash
sudo systemctl stop hop
sudo cp /usr/local/bin/hop-server /usr/local/bin/hop-server.previous
sudo install -m 0755 ./hop-server /usr/local/bin/hop-server
sudo systemctl start hop
sudo systemctl status hop
```

更新后至少验证一次入口认证、一个 SSH 资产和一个 TCP 转发。出现问题时停止服务，恢复 `hop-server.previous`，再检查日志。

OpenWrt 可以在 LuCI 中执行核心下载，也可以使用：

```sh
/usr/share/hop/hop-core update
/etc/init.d/hop restart
```

## 备份与恢复

推荐停止服务后备份完整信任材料：

```bash
sudo systemctl stop hop
backup_dir="/srv/backups/hop-$(date +%F)"
sudo install -d -m 0700 "$backup_dir"
sudo cp -a /var/lib/hop "$backup_dir/"
sudo cp -a /etc/hop "$backup_dir/"
sudo systemctl start hop
```

至少需要保留：

| 文件 | 用途 |
|---|---|
| `hop.db` | Catalog 和运行状态 |
| `hop.secret` | 解密目标凭据 |
| `hop_host_key` | 保持 Hop 的 SSH 服务器身份稳定 |
| `config.toml` | 启动配置 |
| 资源清单和 secret 文件 | 重建声明式资源 |
| Control API Token | 仅在启用 API 时需要 |

恢复时停止 Hop，把这些文件放回原路径并恢复 `hop` 用户的读写权限，然后启动服务。不要只恢复数据库而遗漏与它匹配的 Master Key。

## 常见问题

| 现象 | 检查项 |
|---|---|
| `Permission denied (publickey)` | `key list` 中是否存在并启用了当前入口公钥 |
| TUI 看不到资产 | 当前 Access Key 的模式和白名单，`config status` 中是否有 apply 错误 |
| SSH 直连或 SFTP 失败 | 资产类型是否为 `ssh`，是否引用了有效凭据 |
| TCP 转发被拒绝 | 目标是否使用 `asset-name.hop:port`，资产是否在白名单中 |
| 修改资源文件后没有生效 | source 是否设置 `watch = true`，文件能否由 `hop` 用户读取 |
| API 端口不存在 | `api.enabled = false` 时属于正常行为 |
| 凭据无法解密 | 数据库与 Master Key 是否来自同一份备份 |
