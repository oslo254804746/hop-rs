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

Admin Web 在当前 MVP 中强制绑定 loopback 地址。远程管理请通过宿主机系统 SSH 或管理网络建立隧道，不要把 Admin Web 直接暴露到公网。

## 部署前验证

在发布前运行：

```bash
cargo test --workspace
cargo build --release --bin hop-server --bin hop
```

构建产物：

```text
target/release/hop-server
target/release/hop
```

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
sudo install -m 0755 target/release/hop-server /usr/local/bin/hop-server
sudo install -m 0755 target/release/hop /usr/local/bin/hop
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
- `credential add`：保存目标主机凭证，供 Hop 在 TUI 或 `hop connect <asset>` 时从服务器侧连接目标资产。

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
ssh hop-host -p 2222
```

开发者本地安装 `hop` 后：

```bash
hop --host hop-host --port 2222 ls
hop --host hop-host --port 2222 connect web-prod-01
hop --host hop-host --port 2222 ssh-config
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

当前 Dockerfile 使用 `rust:1.90-bookworm` 构建，并把 `hop-server`、`hop` 和默认配置复制到最终 `debian:bookworm-slim` 镜像。

### 2. 准备数据目录

```bash
mkdir -p data
cp config.example.toml config.toml
```

容器内工作目录是 `/var/lib/hop`，因此 `config.toml` 中的相对路径会落到挂载的数据目录中。

### 3. 推荐模式：Linux host network

因为 Admin Web 强制绑定 `127.0.0.1:8080`，在 Linux 上使用 host network 可以同时保留 loopback 约束并访问管理界面：

```bash
docker run -d \
  --name hop \
  --restart unless-stopped \
  --network host \
  -v "$PWD/data:/var/lib/hop" \
  -v "$PWD/config.toml:/etc/hop/config.toml:ro" \
  -e RUST_LOG=info \
  hop:0.1.0
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

### 4. Bridge 模式：仅发布 SSH 端口

如果使用 Docker bridge 网络：

```bash
docker run -d \
  --name hop \
  --restart unless-stopped \
  -p 2222:2222 \
  -v "$PWD/data:/var/lib/hop" \
  -v "$PWD/config.toml:/etc/hop/config.toml:ro" \
  -e RUST_LOG=info \
  hop:0.1.0
```

这种模式下，SSH 端口可用，但 Admin Web 仍绑定在容器内部 loopback 上。不要把 `admin_bind` 改成 `0.0.0.0:8080`，当前服务会拒绝非 loopback 管理地址。

Bridge 模式建议用 `docker exec` 执行本机管理 CLI：

```bash
docker exec hop hop-server --config /etc/hop/config.toml reset-admin

docker exec hop hop-server --config /etc/hop/config.toml key list
docker exec hop hop-server --config /etc/hop/config.toml asset list
docker exec hop hop-server --config /etc/hop/config.toml credential list
```

需要传入 stdin 时：

```bash
printf '%s' 'secret' | docker exec -i hop hop-server --config /etc/hop/config.toml credential add \
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
sudo install -m 0755 target/release/hop-server /usr/local/bin/hop-server
sudo install -m 0755 target/release/hop /usr/local/bin/hop
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
- Admin Web 无法远程访问：这是预期安全约束；通过宿主机系统 SSH 隧道访问 loopback 管理端口。
- Docker bridge 模式无法访问 Admin Web：当前 MVP 要求 `admin_bind` 是 loopback；使用 Linux host network 或本机管理 CLI。
- DB locked：Hop 已启用 WAL 和 busy timeout；持续锁定通常意味着另一个进程长期占用 SQLite 文件。
