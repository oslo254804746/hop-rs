# Hop 部署文档

本文覆盖两种部署方式：

- 二进制直接部署：适合生产环境，推荐配合 systemd。
- Docker 部署：适合快速验证、单机部署和隔离运行。

Hop 服务端目标运行环境是 Linux。可以在 Windows/macOS 上开发，但生产发布建议在 Linux、WSL 或 Docker/Linux builder 中构建。

## 端口与持久化文件

默认配置来自 `config.example.toml`：

```toml
[server]
ssh_bind = "0.0.0.0:2222"
# Prefer 127.0.0.1 for Admin Web. If you bind 0.0.0.0, protect it with
# a host-local port mapping, firewall, VPN, or trusted management network.
admin_bind = "127.0.0.1:8080"

[database]
path = "./hop.db"

[ssh]
host_key_file = "./hop_host_key"

[security]
secret_key_file = "./hop.secret"
```

生产环境必须持久化并备份：

- `hop.db`：资产、公钥、会话、known hosts、加密后的凭证。
- `hop.secret`：凭证解密主密钥，丢失后已保存凭证不可恢复。
- `hop_host_key`：Hop SSH 服务 host key，丢失后客户端会看到 host-key 变化告警。

Admin Web 默认绑定 loopback 地址。远程管理请通过宿主机系统 SSH 或管理网络建立隧道；如果你主动把 `admin_bind` 改成 `0.0.0.0:8080`，服务会打印 warning 但不会阻止启动。务必用防火墙、VPN、宿主机本地端口映射或可信管理网络限制访问，不要把 Admin Web 直接暴露到公网。

Docker 镜像默认把运行态文件集中在 `/data`，其中包含 `config.toml`、`hop.db`、`hop.secret` 和 `hop_host_key`。生产运行时应挂载 `/data` 到宿主机目录或 Docker volume。

## 部署前验证

在发布前运行：

```bash
cargo test --workspace
cargo build --release -p hop-server
```

构建产物只有 release profile 下的 `hop-server` 服务端二进制。

如果在 Windows 上遇到 `could not execute process ... build-script-build` 这类 release build-script 启动错误，优先在 Linux、WSL 或 Docker builder 中构建。Hop 服务端本身按 Linux 部署设计。

## 二进制直接部署

以下示例假设部署到 Linux 主机，运行用户为 `hop`，数据目录为 `/var/lib/hop`，配置文件为 `/etc/hop/config.toml`。

### 1. 创建用户与目录

```bash
sudo useradd --system --create-home --home-dir /var/lib/hop --shell /usr/sbin/nologin hop
sudo install -d -o hop -g hop -m 0750 /var/lib/hop
sudo install -d -o root -g root -m 0755 /etc/hop
```

### 2. 安装二进制和配置

```bash
SERVER_BIN=hop-server
sudo install -m 0755 "./target/release/${SERVER_BIN}" /usr/local/bin/hop-server
sudo install -m 0644 config.example.toml /etc/hop/config.toml
```

检查 `/etc/hop/config.toml`：

```toml
[server]
ssh_bind = "0.0.0.0:2222"
admin_bind = "127.0.0.1:8080"

[database]
path = "./hop.db"

[ssh]
host_key_file = "./hop_host_key"

[security]
secret_key_file = "./hop.secret"
```

这些相对路径会以 systemd 中的 `WorkingDirectory=/var/lib/hop` 为基准。

### 3. 安装 systemd service

```bash
sudo install -m 0644 systemd/hop.service /etc/systemd/system/hop.service
sudo systemctl daemon-reload
sudo systemctl enable --now hop
```

查看首次启动日志和初始管理员密码：

```bash
sudo journalctl -u hop -n 100 --no-pager
```

常用运维命令：

```bash
sudo systemctl status hop
sudo systemctl restart hop
sudo journalctl -u hop -f
```

### 4. 访问 Admin Web

在 Hop 主机本机访问：

```text
http://127.0.0.1:8080
```

远程访问时，通过宿主机系统 SSH 建立隧道：

```bash
ssh -N -L 8080:127.0.0.1:8080 root@hop-host
```

然后在本机浏览器打开：

```text
http://127.0.0.1:8080
```

这里使用的是宿主机系统 SSH，不是 Hop 自身的 `2222` 端口。

### 5. 初始化数据

也可以不用 Web，直接在服务器上用本机 CLI 初始化：

这里的 `key` 和 `credential` 分别负责两段认证：

- `key add`：允许开发者用自己的 SSH 公钥登录 Hop 的 `2222` 端口。
- `credential add`：保存目标主机凭证，供 Hop 在 TUI 或资产名直连时从服务器侧连接目标资产。

```bash
sudo -u hop /usr/local/bin/hop-server --config /etc/hop/config.toml reset-admin

sudo -u hop /usr/local/bin/hop-server --config /etc/hop/config.toml key add \
  --name "alice laptop" \
  --public-key-file /tmp/id_ed25519.pub

printf '%s' 'secret' | sudo -u hop /usr/local/bin/hop-server --config /etc/hop/config.toml credential add \
  --name deploy-password \
  --username deploy \
  --auth-type password \
  --password-stdin

sudo -u hop /usr/local/bin/hop-server --config /etc/hop/config.toml asset add \
  --name web-prod-01 \
  --hostname 10.0.1.10 \
  --port 22 \
  --tags prod,web \
  --credential-id <credential-id>
```

### 6. 验证 SSH 入口

用户公钥加入白名单后：

```bash
ssh -p 2222 hop-host
ssh -p 2222 web-prod-01@hop-host
ssh -J hop-host:2222 web-prod-01.hop
```

ProxyJump 示例：

```bash
ssh -J hop-host:2222 web-prod-01.hop
scp -J hop-host:2222 ./file web-prod-01.hop:/tmp/
```

ProxyJump 不使用 Hop 托管凭证，用户本机必须拥有目标主机认证能力。

## Docker 部署

### 1. 构建镜像

```bash
docker build -t hop:0.1.0 .
```

当前 Dockerfile 使用 `rust:1.90-bookworm` 构建，并把 `hop-server` 和默认配置复制到最终 `debian:bookworm-slim` 镜像。

### 2. 准备数据目录

```bash
mkdir -p data
```

容器内默认数据目录是 `/data`。首次启动时，如果 `/data/config.toml` 不存在，镜像 entrypoint 会从内置模板创建它。Docker 模板使用绝对路径，因此以下文件都会落在同一个挂载目录中：

- `/data/config.toml`
- `/data/hop.db`
- `/data/hop.secret`
- `/data/hop_host_key`
- `/data/hop.db-wal` 和 `/data/hop.db-shm`

### 3. Linux 推荐模式：host network

因为默认配置让 Admin Web 绑定 `127.0.0.1:8080`，在 Linux 上使用 host network 可以同时保留 loopback 约束并访问管理界面：

```bash
docker run -d \
  --name hop \
  --restart unless-stopped \
  --network host \
  -v "$PWD/data:/data" \
  -e RUST_LOG=info \
  ghcr.io/oslo254804746/hop-rs:latest
```

查看初始管理员密码：

```bash
docker logs hop
```

本机访问：

```text
http://127.0.0.1:8080
```

远程访问仍建议通过宿主机系统 SSH 隧道：

```bash
ssh -N -L 8080:127.0.0.1:8080 root@hop-host
```

### 4. Docker Desktop / 非 Linux：bridge 模式

Docker Desktop 的 `--network=host` 不等价于真实 Linux 宿主机网络；容器内的 `127.0.0.1:8080` 仍可能只属于 Docker Desktop 的 Linux VM，Windows/macOS 宿主机浏览器无法访问。

如果需要在 Docker Desktop 上本机访问 Admin Web，请先初始化配置：

```bash
docker run --rm \
  -v "$PWD/data:/data" \
  ghcr.io/oslo254804746/hop-rs:latest \
  hop-server --help >/dev/null
```

然后编辑 `data/config.toml`，把 Admin Web 监听地址改成容器网卡：

```toml
[server]
admin_bind = "0.0.0.0:8080"
```

再用 bridge 网络启动，并把 Admin Web 只发布到宿主机 loopback：

```bash
docker run -d \
  --name hop \
  --restart unless-stopped \
  -p 2222:2222 \
  -p 127.0.0.1:8080:8080 \
  -v "$PWD/data:/data" \
  -e RUST_LOG=info \
  ghcr.io/oslo254804746/hop-rs:latest
```

真正的访问边界来自 `-p 127.0.0.1:8080:8080`，它让管理端只在宿主机本机可访问。不要使用 `-p 8080:8080` 把 Admin Web 绑定到所有宿主机网卡，除非你已经有可信管理网络或防火墙策略。

如果只需要 SSH 入口，不需要 Web 管理界面，可以保持默认配置，只发布 SSH 端口：

```bash
docker run -d \
  --name hop \
  --restart unless-stopped \
  -p 2222:2222 \
  -v "$PWD/data:/data" \
  -e RUST_LOG=info \
  ghcr.io/oslo254804746/hop-rs:latest
```

Bridge 模式建议用 `docker exec` 执行本机管理 CLI：

```bash
docker exec --user hop hop hop-server --config /data/config.toml reset-admin

docker exec --user hop hop hop-server --config /data/config.toml key list
docker exec --user hop hop hop-server --config /data/config.toml asset list
docker exec --user hop hop hop-server --config /data/config.toml credential list
```

需要传入 stdin 时：

```bash
printf '%s' 'secret' | docker exec -i --user hop hop hop-server --config /data/config.toml credential add \
  --name deploy-password \
  --username deploy \
  --auth-type password \
  --password-stdin
```

### 5. Docker 运维命令

```bash
docker logs -f hop
docker restart hop
docker stop hop
docker rm hop
```

进入容器检查：

```bash
docker exec -it hop sh
```

## 升级

二进制部署：

```bash
sudo systemctl stop hop
sudo tar -C /var/lib -czf hop-backup-$(date +%Y%m%d%H%M%S).tgz hop
SERVER_BIN=hop-server
sudo install -m 0755 "./target/release/${SERVER_BIN}" /usr/local/bin/hop-server
sudo systemctl start hop
sudo journalctl -u hop -n 100 --no-pager
```

Docker 部署：

```bash
docker build -t hop:0.1.1 .
tar -czf hop-data-backup-$(date +%Y%m%d%H%M%S).tgz data
docker stop hop
docker rm hop
```

然后用新镜像按原 `docker run` 参数启动。

## 备份与恢复

备份数据目录即可覆盖数据库、主密钥和 SSH host key：

```bash
sudo systemctl stop hop
sudo tar -C /var/lib -czf hop-backup-$(date +%Y%m%d%H%M%S).tgz hop
sudo systemctl start hop
```

Docker：

```bash
docker stop hop
tar -czf hop-data-backup-$(date +%Y%m%d%H%M%S).tgz data
docker start hop
```

恢复时先停止服务，解压备份到原数据目录，再启动服务。

## 排障

- Unknown SSH key：确认用户公钥已通过 Admin Web 或 `hop-server key add` 加入白名单，并且处于 active 状态。
- Permission denied (publickey)：这是 Hop 入口认证失败。请添加开发者本机公钥到 authorized keys；不要把 `credential add` 创建的目标主机用户名/密码当作 Hop 登录密码。
- Target auth failure：确认资产绑定了正确 credential，目标用户名、密码或私钥可用。
- Host key mismatch：先核实目标主机是否重装或 key 是否变更，再从 known hosts 中清理旧记录。
- 默认配置下 Admin Web 无法远程访问：这是 loopback 绑定的结果；通过宿主机系统 SSH 隧道访问，或在明确访问边界后调整 `admin_bind`。
- Docker Desktop 下 `--network=host` 无法访问 Admin Web：使用 bridge 模式、`-p 127.0.0.1:8080:8080`，并在 `/data/config.toml` 中显式设置 `admin_bind = "0.0.0.0:8080"`。
- DB locked：Hop 已启用 WAL 和 busy timeout；持续锁定通常意味着另一个进程长期占用 SQLite 文件。
