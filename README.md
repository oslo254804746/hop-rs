<div align="center">

**中文** | [English](README-EN.md)

# 🦀 Hop

**极简 SSH 跳板机，极致掌控。**

一个 Rust 单二进制文件，用公钥认证、TUI 资产选择器、托管凭证和代理转发，替代你臃肿的跳板机方案 —— 全部由 SQLite 驱动。

[![CI](https://github.com/oslo254804746/hop-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/oslo254804746/hop-rs/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

</div>

---

## 为什么选 Hop？

大多数跳板机/堡垒机方案是臃肿的 Java/Python 全家桶，需要数据库、缓存、消息队列和各种管理面板，部署一套要一周。Hop 反其道而行：

- **单二进制** —— `hop-server` 一个文件搞定一切
- **零外部依赖** —— SQLite 内嵌，不需要 Redis/Postgres/RabbitMQ
- **默认安全** —— Admin Web 仅监听本地回环，凭证使用 ChaCha20-Poly1305 加密
- **SSH 原生** —— 用户只需要 `ssh`，无需专属客户端

## 功能一览

```text
┌────────────────────────────────────────────────────────┐
│  公钥白名单        仅受信密钥可进入 Hop               │
│  TUI 资产选择器    模糊搜索，秒级连接                 │
│  托管凭证          服务端代理认证目标主机             │
│  ProxyJump 转发    受资产白名单约束的 TCP 转发        │
│  Admin Web         轻量管理界面                       │
│  批量导入/导出     资产与凭证批量迁移                 │
│  TOFU 主机密钥     首次连接自动信任                   │
│  i18n 管理界面     多语言支持                         │
└────────────────────────────────────────────────────────┘
```

## 快速开始

```bash
# 构建
cargo build --release -p hop-server

# 运行
cp config.example.toml config.toml
./target/release/hop-server serve --config config.toml
```

首次启动自动生成：
- SQLite 数据库
- Ed25519 主机密钥
- ChaCha20-Poly1305 主密钥（`hop.secret`）
- 一次性管理员密码（输出到终端）

默认端口：**SSH `0.0.0.0:2222`** | **Admin Web `127.0.0.1:8080`**

## 使用方式

```bash
# 交互式 TUI —— 模糊搜索你的服务器
ssh -p 2222 hop-host

# 直连模式 —— 资产名作为 SSH 用户名
ssh -p 2222 web-prod-01@hop-host

# ProxyJump —— Hop 作为透明 TCP 中继
ssh -J hop-host:2222 web-prod-01.hop
```

## 项目结构

```text
crates/
├── hop-core/       配置、模型、SQLite、凭证加密
└── hop-server/     SSH 服务、TUI、Admin Web、本机 CLI
migrations/         SQLite schema 迁移
systemd/            生产环境 systemd 服务单元
```

**技术栈：** `russh` · `ratatui` · `axum` · `sqlx` · `chacha20poly1305` · `maud`

## CLI 参考

```bash
hop-server serve                    # 启动服务（默认）
hop-server reset-admin              # 重置管理员密码
hop-server key add|list|activate|deactivate
hop-server credential add|list|delete
hop-server asset add|list|delete
hop-server export --kind assets --format csv --output dump.csv
hop-server import --file dump.csv --on-conflict skip
```

## Docker

```bash
# Linux（推荐）：host 网络保持回环绑定
docker run -d --name hop --network host \
  -v "$PWD/data:/data" ghcr.io/oslo254804746/hop-rs:latest

# Docker Desktop：bridge 网络 + 仅本地回环映射管理端口
docker run -d --name hop \
  -p 2222:2222 -p 127.0.0.1:8080:8080 \
  -v "$PWD/data:/data" ghcr.io/oslo254804746/hop-rs:latest
```

查看初始管理员密码：`docker logs hop`

## 部署

完整部署指南（二进制、systemd、Docker、升级、备份、排障）：

**→ [docs/deployment.md](docs/deployment.md)**

## 安全模型

| 层级 | 机制 |
|------|------|
| Hop 入口认证 | 仅 SSH 公钥白名单 |
| 凭证存储 | ChaCha20-Poly1305 + HKDF-SHA256 |
| Admin Web 认证 | Argon2 密码哈希 |
| ProxyJump 目标 | 资产白名单强制校验 |
| Admin Web 暴露面 | 默认仅监听回环地址 |

> **`hop.secret` 是你的命根子。** 丢了它，所有已存储的凭证将无法恢复。务必备份。

## 备份

三个文件，一次原子快照：

```bash
hop.db          # 所有数据：资产、密钥、会话、加密凭证
hop.secret      # 主密钥 —— 丢失不可恢复
hop_host_key    # SSH 主机身份
```

## 许可证

[MIT](LICENSE)
