# Hop v0.2 Control API 与本地管理

Hop v0.2 不包含 Admin Web、管理员账号或角色协议。一个实例属于一个管理信任域，管理入口是本机 CLI、声明式 Apply，以及一枚等权管理 Token 保护的可选 Control API。

## 启用 API

API 默认关闭，关闭时不会创建 HTTP listener。先创建高熵、权限受控的 Token 文件，再配置 loopback 地址并重启：

```toml
[api]
enabled = true
listen = "127.0.0.1:8083"
token_file = "/var/lib/hop/control-api.token"
cors_allowlist = []
```

所有请求必须携带：

```http
Authorization: Bearer <token>
```

非 loopback 监听必须声明非空 `cors_allowlist`，并应由 TLS 与网络访问控制保护；CORS 本身不是安全边界。

## 接口

| 方法 | 路径 | 用途 |
|---|---|---|
| GET | `/api/v1/status` | 健康、版本、Catalog revision |
| GET | `/api/v1/catalog/revision` | 乐观并发 revision |
| GET/POST | `/api/v1/assets` | 列出/创建本地资产 |
| PUT/DELETE | `/api/v1/assets/{id}` | 更新/删除本地资产 |
| GET/POST | `/api/v1/credentials` | 列出 secret 状态/创建本地凭据 |
| PUT/DELETE | `/api/v1/credentials/{id}` | 更新/删除本地凭据 |
| GET/POST | `/api/v1/access-keys` | 列出/创建本地 Access Key |
| DELETE | `/api/v1/access-keys/{id}` | 吊销并删除本地 Key |
| PUT | `/api/v1/access-keys/{id}/enabled` | 启用或禁用 Key |
| PUT | `/api/v1/access-keys/{id}/access` | 替换 all/restricted 资产范围 |
| GET | `/api/v1/sessions` | 最近会话 |
| POST | `/api/v1/sessions/{id}/terminate` | 显式终止已登记的活动会话 |
| GET | `/api/v1/config/sources` | source 成功/失败与 generation |
| GET | `/api/v1/config/status` | source、orphan、schema 和 revision |
| POST | `/api/v1/config/validate` | 校验 manifest 内容 |
| POST | `/api/v1/config/diff` | 只读 diff |
| POST | `/api/v1/config/apply` | 携带 base revision 的原子 apply |
| POST | `/api/v1/config/reload` | 通过同一 Apply engine 重载启动配置中的 source |

凭据响应不会返回明文、加密 envelope、私钥或密码，只返回 `configured`/`missing`。Access Key 响应不返回公钥正文，只返回名称、指纹、状态、模式和已分配资产 ID。

## 本地 CRUD

创建密码凭据：

```json
{
  "name": "root",
  "username": "root",
  "auth_type": "password",
  "password": "request-only-secret"
}
```

`auth_type` 支持 `password`、`key` 和 `key_passphrase`。更新时省略 secret 会保留与认证类型兼容的现有值；切换认证类型必须提交新类型需要的材料。

创建资产：

```json
{
  "name": "server",
  "protocol": "ssh",
  "hostname": "192.0.2.10",
  "port": 22,
  "credential_id": "<credential-id>",
  "tags": ["home"]
}
```

TCP 资产使用 `protocol: "tcp"`，不带 SSH 凭据。RDP、VNC、MySQL、PostgreSQL 和 Redis 都使用同一种 TCP 资产，不需要额外的类型别名。

创建 Access Key：

```json
{
  "name": "laptop",
  "public_key": "ssh-ed25519 AAAA...",
  "assets": []
}
```

创建/更新 Key 范围时，省略 `assets` 表示全部资产，`[]` 表示不能访问任何资产，非空数组是内部资产 ID 的严格白名单。

普通 CRUD 只能修改 `local` 资源。修改声明式资源会返回 HTTP 409 和 `managed_by_source`；应修改拥有该资源的 manifest 后重新 apply。

## Validate、diff 与 apply

API payload 传 manifest 内容，不接受服务器任意文件路径：

```json
{
  "content": "api_version: hop/v1alpha1\nassets: {}\n",
  "format": "yaml",
  "source_id": "panel",
  "base_revision": 12,
  "prune": false,
  "dry_run": false
}
```

Apply 必须携带 `/api/v1/catalog/revision` 返回的 revision。过期值返回 `409 revision_conflict`。错误使用稳定 code 与资源 path；失败只记录非敏感 source/审计摘要，不产生部分资源修改。Dry-run 不写 Catalog。

## 会话语义

禁用 Key、收紧白名单、修改凭据或删除资产会影响新连接，默认不终止已有 SSH stream。需要紧急阻断时调用显式 terminate 接口。

## 面板现状与边界

v0.2.0 当前没有可管理 Catalog 资源的官方图形面板。`hop-rs` 已删除旧 Admin Web；`luci-app-hop` 当前只提供服务启停、核心下载和日志等 OpenWrt 外壳设置，不调用 Control API。需要管理资产、凭据、Access Key 或白名单时，当前请使用本机 CLI、manifest 或 `/api/v1`。

后续通用面板和 LuCI 资源页面都必须调用 `/api/v1`，不得直接读写 SQLite，也不得在 UCI 中重复资产、凭据或 Access Key。核心不托管面板静态资源，也不依赖前端仓库。已确认的交付与安全边界见 [v0.2 管理面板交付边界](product/management-panel-v0.2.md)。
