# Hop 部署指南

## 路径一：官方 Compose + 网页面板（推荐）

### 1. 准备

- Docker Engine 24+ 与 Docker Compose v2
- 相邻的 `hop-rs/` 和 `hop-rs-frontend/` 源码目录，或可访问同版本 GHCR 镜像
- 宿主机上未占用的 SSH 与面板端口

### 2. 创建配置

```bash
cd hop-rs
cp hop.yaml hop.local.yaml
python3 -c 'import secrets; print(secrets.token_urlsafe(32))'
```

把生成值写入 `hop.local.yaml` 的 `api.token`，然后：

```bash
chmod 0600 hop.local.yaml
HOP_CONFIG_FILE=./hop.local.yaml docker compose config
HOP_CONFIG_FILE=./hop.local.yaml docker compose up -d --build
```

Compose 只向宿主机发布 `${HOP_SSH_PORT:-2222}` 与 `${HOP_PANEL_PORT:-8080}`。Control API 的 8083 端口只在 Compose 网络内可见，面板严格把 `/api/v1` 转发给 `http://hop:8083`；其他 `/api` 路径返回 404。

### 3. 首次使用

打开 `http://<hop-host>:8080`，输入 `hop.local.yaml` 中的网页管理 Token。无需填写 API 地址。初始 Catalog 为空，可依次创建：

1. 入口访问公钥；
2. SSH 目标凭据（TCP 资产不需要）；
3. SSH 或 TCP 资产。

默认 `change-me` 会在后端日志与网页中产生警告。它不是安全值。

### 4. 持久化与升级

- `/data` 使用 `hop-data` 命名卷，保存数据库、主密钥和 SSH 主机密钥。
- `hop.local.yaml` 只读挂载，容器不会修改它。
- 两个镜像使用同一个 `HOP_VERSION` 标签，避免 API/面板版本漂移。

```bash
HOP_VERSION=v0.2.1 HOP_CONFIG_FILE=./hop.local.yaml docker compose pull
HOP_VERSION=v0.2.1 HOP_CONFIG_FILE=./hop.local.yaml docker compose up -d
```

升级前备份配置与数据卷。回滚时同时回滚 hop 与 panel 镜像。

## 路径二：单 YAML、无面板

### 二进制

```bash
cp config.example.yaml hop.local.yaml
chmod 0600 hop.local.yaml
hop-server --config ./hop.local.yaml config validate
hop-server --config ./hop.local.yaml serve
```

若 `api.token` 缺失或为空，Control API 会禁用但 SSH 仍启动。这适合只用 SSH 与本地 CLI 的部署。

### 单容器

```bash
docker run -d --name hop --restart unless-stopped \
  -p 2222:2222 \
  -v "$PWD/hop.local.yaml:/etc/hop/hop.yaml:ro" \
  -v hop-data:/data \
  ghcr.io/oslo254804746/hop-rs:v0.2.1 \
  hop-server --config /etc/hop/hop.yaml serve
```

只有需要直接从浏览器跨域访问独立 Control API 时，才应额外发布 8083，并在 `api.cors_allowlist` 中列出完整的 `http://` 或 `https://` Origin。裸主机名、裸 IP、包含路径的 URL 都会得到带字段位置的配置错误。`["*"]` 可用但只适合明确接受任意 Origin 的环境，且不能与其他条目混用。

## systemd

```ini
[Unit]
Description=Hop SSH jump server
After=network-online.target

[Service]
User=hop
Group=hop
ExecStart=/usr/local/bin/hop-server --config /etc/hop/hop.yaml serve
Restart=on-failure
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ReadWritePaths=/var/lib/hop

[Install]
WantedBy=multi-user.target
```

配置中使用 `data_dir: /var/lib/hop`，并让 `/etc/hop/hop.yaml` 权限为 `0600`、属主为 `hop`。

## 运维检查

```bash
docker compose ps
docker compose logs hop panel
ssh -p 2222 <asset-name>@<hop-host>
```

未匹配的入口公钥必须只得到 `Permission denied (publickey)`。面板刷新后 Token 从浏览器内存清除，这是预期行为；重新输入即可。
