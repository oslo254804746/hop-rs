# Hop 声明式资源 Apply 与 SQLite 事实源规范

状态：Active，v0.2 已实现契约。

产品边界：[Hop v0.2 产品方向](product-direction-v0.2.md)

## 1. 决策摘要

Hop 采用“不对称混合”配置模型：

- 启动配置文件负责进程启动前必须知道的参数。
- SQLite 是资产、凭据、SSH 入口和授权关系的唯一运行事实源。
- YAML/TOML 资源清单、CLI 和 HTTP API 都是向 SQLite 提交变更的入口。
- YAML/TOML 清单不会在每次 SSH 请求中被直接读取，也不会与 SQLite 双向同步。
- 会话、审计、资产健康和 Known Hosts 只属于 SQLite 或内存，不进入资源清单。

核心数据流：

```text
YAML/TOML manifest ─┐
CLI CRUD ───────────┼─> validate/diff/apply ─> SQLite ─> SSH runtime
HTTP API ───────────┘
```

本规范首先固定四项行为：资源归属、覆盖优先级、删除语义和事务边界。

## 2. 目标与非目标

### 2.1 目标

1. 保持单二进制、单 SQLite 文件、无外部数据库的部署体验。
2. 允许开发者通过版本可控的 YAML/TOML 管理资源。
3. 允许 CLI、外部面板和后续 LuCI 在同一运行实例中安全管理资源。
4. 配置错误时继续使用上一份有效数据，不产生部分更新。
5. 明确每个资源由谁管理，避免文件、CLI 和 API 相互覆盖。
6. 让资源变更对新连接即时生效，同时定义现有连接的稳定行为。

### 2.2 非目标

- 不在 v1 中实现 SQLite 与资源文件的双向导出或自动回写。
- 不把监听地址、数据库路径等全部迁入 SQLite。
- 不允许多个资源文件通过加载顺序实现隐式 `last wins`。
- 不通过删除文件直接、立即删除线上资源。
- 不在资源清单中保存会话、审计、健康状态或缓存。
- 不在 v1 中实现跨 Hop 实例的集中式控制平面。
- 不在资源 Catalog 或 Control API 中引入管理员角色与 capability 分配。

## 3. 配置边界

### 3.1 启动配置

启动配置在打开数据库和监听端口前读取，文件是这部分配置的唯一事实源。

典型字段：

- SSH 监听地址。
- Control API 是否启用及监听地址。
- SQLite 路径。
- Host Key 路径和类型。
- Master Key 文件路径。
- 资源清单路径、是否监听和默认 prune 策略。
- 运行目录、日志级别和状态保留策略。

这些字段的变更默认需要进程重启。资源文件 apply 不得修改它们。

启动配置示例：

```yaml
server:
  ssh_listen: 0.0.0.0:12222

database:
  path: /data/hop.db

api:
  enabled: false
  listen: 127.0.0.1:8083
  token_file: /data/hop-api.token

ssh:
  host_key_file: /data/hop-host-key
  host_key_type: ed25519
  connect_timeout: 10
  keepalive_interval: 30
  proxy_policy: assets_only
  banner: ""

inventory:
  sources:
    - id: home
      path: /etc/hop/resources.d/*.yaml
      watch: true
      prune: false
```

### 3.2 资源清单

资源清单是提交给 apply 引擎的声明，不是 SSH 运行时直接读取的数据源。

v1 资源范围：

- `credentials`
- `assets`
- `access`，对应 SSH 公钥及其资产范围

资源清单示例：

```yaml
api_version: hop/v1alpha1

credentials:
  oslo:
    type: password
    username: root
    password:
      file: /data/secrets/oslo.password

assets:
  demo:
    type: ssh
    host: 192.168.11.133
    port: 22
    display_name: Demo Server
    credential: oslo

  demo_rdp:
    type: tcp
    host: 192.168.11.133
    port: 3389

access:
  oslo:
    public_key:
      file: /data/authorized_keys/oslo.pub
    assets:
      - demo
      - demo_rdp
```

v1 中每个 `access` 条目只包含一把公钥。未设置 `assets` 时默认访问全部资产；设置空数组表示允许认证但不能访问任何资产。Hop 不计划引入 People 聚合实体。

## 4. 资源标识与归属

### 4.1 稳定标识

资源清单中的 map key 是资源的外部稳定名称，例如 `assets.demo`。

- 同一资源类型内名称全局唯一。
- 名称用于 diff、apply 和引用解析，不依赖文件顺序。
- SQLite 继续使用内部 UUID；重命名默认视为创建新资源并处理旧资源。
- 后续如需无损重命名，应增加显式 `id` 或 `rename_from`，不得用模糊匹配猜测。

### 4.2 管理模式

每个可管理资源增加以下元数据：

| 字段 | 含义 |
|---|---|
| `management_mode` | `local` 或 `declarative` |
| `source_id` | 声明式来源，例如 `home`；本地资源为空 |
| `source_key` | 例如 `asset/demo` |
| `source_generation` | 最近成功 apply 的来源代数 |
| `last_applied_hash` | 规范化声明的摘要 |
| `last_applied_at` | 最近成功 apply 时间 |
| `orphaned_at` | 声明中消失但尚未删除的时间 |

CLI/API 直接创建的资源使用 `management_mode=local`。通过资源清单创建的资源使用 `management_mode=declarative`。

操作者来源仍单独进入审计事件，例如 `cli`、`api` 或 `watcher`，不要与资源管理模式混为一谈。

### 4.3 归属规则

1. 同一个资源只能有一个管理方。
2. 同一 apply 批次中出现重复资源名称时，整批失败。
3. 不同声明式来源声明同名资源时，返回 `ownership_conflict`，不使用加载顺序决定结果。
4. 声明式资源默认不能通过普通 CRUD API 或 CLI 修改。
5. 本地资源默认不能被资源清单静默接管。
6. v0.2 不暴露 takeover/adopt 操作；需要改变归属时，由当前管理入口显式删除资源，再通过新入口重新创建，避免隐式接管。

## 5. 覆盖优先级

Hop 不定义全局“文件优先”或“数据库优先”的字段级覆盖关系，而是使用资源级单一所有者。

| 当前归属 | 变更入口 | 默认结果 |
|---|---|---|
| `local` | CLI/API CRUD | 允许 |
| `local` | manifest apply | 冲突，不允许隐式接管 |
| `declarative:home` | 来源 `home` apply | 允许 |
| `declarative:home` | 其他来源 apply | 冲突 |
| `declarative` | CLI/API CRUD | 拒绝并返回管理来源 |
| 任意 | 运行状态更新 | 仅更新状态表，不改变资源归属 |

禁止字段级混合所有权。例如不允许资产地址来自 YAML、标签来自 API。字段级所有权会让 diff、删除和排错行为无法解释。

## 6. 删除与缺失语义

### 6.1 默认无 prune

一次完整、成功的来源扫描后，如果数据库中由该来源管理的资源没有出现在新声明中：

- 不立即删除资源。
- 设置 `orphaned_at`。
- 资源继续对运行时有效。
- `hop config status` 和管理 API 必须显示 orphaned 警告。

资源重新出现在同一来源中时，清除 `orphaned_at`。

### 6.2 显式删除

支持在清单中声明：

```yaml
assets:
  old_server:
    state: absent
```

显式删除必须满足：

- 当前来源拥有该资源。
- 所有引用同时在本批次中解除，或删除操作因 `resource_in_use` 失败。
- 删除资产时同步清理相关授权关系和健康记录。
- 删除凭据时仍保留“被资产引用则拒绝”的现有保护。

### 6.3 Prune

`hop apply --prune` 删除当前 apply scope 中已由同一来源管理、但未出现在成功声明中的资源。

- prune 必须先输出 diff。
- watcher 默认不得 prune。
- 只有启动配置对特定来源显式设置 `prune: true` 时，watcher 才可 prune。
- 文件不存在、glob 暂时无匹配、权限错误或解析失败都不是成功扫描，不得触发 orphan 或 prune。
- prune 不能跨 source 作用。

### 6.4 活动会话

删除或禁用资源后：

- 新连接立即拒绝使用该资源。
- 已建立连接默认保持到自然结束。
- 后续可增加 `--terminate-active` 或策略字段，但不得作为 v1 默认行为。

## 7. Apply 事务模型

### 7.1 Apply scope

一个 apply scope 是以下之一：

- 单个资源文件。
- 一次命令传入的多个资源文件。
- 一个声明式 source 的完整 glob 扫描结果。
- 一次 API apply 请求的完整 payload。

一个 scope 必须全有或全无，不能逐文件、逐资源部分成功。

### 7.2 执行顺序

```text
1. 读取完整 scope
2. 解析 YAML/TOML
3. 检查 api_version 和未知字段
4. 解析 file/env secret 引用
5. 校验名称、类型、端口和凭据材料
6. 解析所有跨资源引用
7. 检查归属冲突和删除影响
8. 读取当前 catalog revision 并生成 diff
9. 开启单个 SQLite 事务
10. 再次确认 revision/归属没有并发变化
11. 按依赖顺序写入资源、授权和审计事件
12. 更新 source generation 与 catalog revision
13. 提交事务
```

任意一步失败：

- SQLite 不产生资源变更。
- 当前 SSH 运行行为不变。
- watcher 继续保留上一代成功配置。
- 返回结构化错误，包含文件、资源路径和错误码，但不包含 secret 内容。

### 7.3 依赖顺序

创建和更新顺序：

1. credentials
2. assets
3. access keys
4. key-to-asset assignments

删除顺序相反。所有步骤仍位于同一数据库事务内。

### 7.4 并发与 revision

数据库维护单调递增的 `catalog_revision`。

- 每次成功资源变更只递增一次。
- dry-run 返回计算 diff 时使用的 revision。
- HTTP apply 使用请求字段携带 base revision。
- 如果校验完成后 catalog 已变化，apply 返回 `revision_conflict`，不得基于旧 diff 继续提交。
- CLI 可以提示用户重新执行 diff/apply；不得静默覆盖并发 API 修改。

## 8. 校验规则

### 8.1 通用规则

- 必须提供受支持的 `api_version`。
- 默认拒绝未知字段。
- map key 只允许稳定、可用于 CLI 的名称字符集。
- 同类型资源名称唯一。
- 引用必须在本批次声明或 SQLite 当前可见数据中存在。
- `state: absent` 资源不能被其他最终态资源引用。

### 8.2 资产规则

- 端口范围为 `1..=65535`。
- `ssh` 资产可以引用 SSH credential。
- `tcp` 资产不得引用 SSH credential；RDP、VNC 和 MySQL 等服务统一使用 `tcp` 类型。
- 同一资产的类型改变必须经过完整兼容性校验，不能保留不再适用的 credential 字段。

### 8.3 凭据规则

- `password` 必须有 username 和 password source。
- `ssh_key` 必须有 username 和 private key source，可选 passphrase。
- 出站 SSH key 不要求 public key 字段。
- 默认支持 `file` 和 `env` secret source。
- inline secret 默认拒绝；如后续支持，必须由启动配置显式开启并输出安全警告。
- secret 经校验后使用现有 Master Key 加密写入 SQLite，API、diff 和审计均不得返回明文。
- 为识别轮换，可保存基于 Master Key 的 secret HMAC，不保存明文摘要。

## 9. CLI 契约

```bash
hop-server config validate -f resources.yaml
hop-server config validate -f resources.yaml --offline
hop-server config diff -f resources.yaml --source home
hop-server apply -f resources.yaml --source home
hop-server apply -f '/etc/hop/resources.d/*.yaml' --source home
hop-server apply -f resources.yaml --source home --dry-run
hop-server apply -f resources.yaml --source home --prune
hop-server config status
```

要求：

- `validate` 默认只读 SQLite，以检查对现有资源的引用和归属，但不产生写入。
- `validate --offline` 不访问 SQLite，只检查 schema、资源清单内部引用和 secret material。
- `diff` 读取 SQLite，不写入。
- `apply --dry-run` 执行完整校验和归属检查，不提交事务。
- `apply` 输出 created/updated/deleted/orphaned/unchanged 数量和新 revision。
- 机器调用可选择 JSON 输出，并使用稳定错误码。

## 10. HTTP API 契约

版本化声明式接口：

- `POST /api/v1/config/validate`
- `POST /api/v1/config/diff`
- `POST /api/v1/config/apply`
- `GET /api/v1/config/sources`
- `GET /api/v1/config/status`
- `GET /api/v1/catalog/revision`

状态、资产、凭据、Access Key、会话终止和 local resource CRUD 见 [Control API 与本地管理](../admin-guide.zh-CN.md)。

约束：

- API payload 是 manifest 内容，不接受服务器本地任意文件路径。
- apply 必须携带稳定 `source_id` 和 base revision。
- API 返回的资源和 diff 对 secret 字段统一使用 `configured`、`missing`、`changed` 等状态。
- 普通资产/凭据 CRUD 只能修改 `local` 资源。
- 修改声明式资源时返回 `409 managed_by_source`，并指出 `source_id`。
- 管理 Token 等权，不定义 Owner、Operator、Viewer 或 capability scope。
- API 默认关闭并监听 loopback；启用远程访问时必须使用认证和明确 CORS allowlist。

## 11. Watcher 与热更新

watcher 是 apply 的触发器，不是另一套配置实现。

- watcher 每秒轮询 scope 的路径、大小与 mtime 快照；连续两次相同后才触发 apply，等价于 debounce 和稳定性等待。
- 每个配置 source 在进程启动时先 apply 一次；只有 `watch: true` 的 source 继续轮询。
- 每次触发重新扫描该 source 的完整 scope。
- 解析失败只记录失败状态，不清空数据库。
- 连续相同内容通过 `last_applied_hash` 保持幂等，不递增 revision。
- 成功 apply 后，新 SSH 认证、TUI 查询、直连、SFTP、ProxyJump 和 TCP 转发读取新数据。
- 已连接会话遵守第 6.4 节，不因普通 apply 自动断开。
- watcher 只观察 manifest scope；启动配置字段变化需要重启。

## 12. 审计与可观测性

每次产生资源变更的成功 apply 写入批次审计；失败的非 dry-run apply 通过独立安全路径记录失败摘要。完全幂等的 apply 和 dry-run 不写数据库：

```text
action: config.apply
actor: catalog-apply 或失败触发入口
source_id: home
base_revision: 41
new_revision: 42
created: 2
updated: 1
deleted: 0
orphaned: 1
result: success|failed
```

成功摘要与资源变更位于同一事务。失败事件不在资源事务中写入，只更新 source 错误状态和非敏感审计摘要。

必须提供：

- 每个 source 的最后成功时间、generation 和 revision。
- 最后失败时间及非敏感错误摘要。
- orphaned 资源列表。
- 当前 catalog revision。
- 当前运行版本支持的 manifest schema 版本。

## 13. 数据库实现

v0.2 baseline 使用独立 ownership 表，避免在每张资源表重复全部字段：

```sql
CREATE TABLE resource_ownership (
    resource_type       TEXT NOT NULL,
    resource_id         TEXT NOT NULL,
    management_mode     TEXT NOT NULL,
    source_id           TEXT,
    source_key          TEXT,
    source_generation   INTEGER,
    last_applied_hash   TEXT,
    last_applied_at     TIMESTAMP,
    orphaned_at         TIMESTAMP,
    PRIMARY KEY (resource_type, resource_id),
    UNIQUE (resource_type, source_id, source_key)
);

CREATE TABLE catalog_meta (
    singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
    revision     INTEGER NOT NULL
);

CREATE TABLE config_sources (
    source_id             TEXT PRIMARY KEY,
    generation            INTEGER NOT NULL DEFAULT 0,
    last_success_at       TIMESTAMP,
    last_success_revision INTEGER,
    last_error_at         TIMESTAMP,
    last_error_code       TEXT,
    last_error_message    TEXT
);
```

## 14. 错误码

v1 至少定义：

- `unsupported_api_version`
- `unknown_field`
- `duplicate_resource`
- `invalid_resource_name`
- `invalid_secret_source`
- `secret_unavailable`
- `invalid_credential_material`
- `unknown_reference`
- `resource_in_use`
- `ownership_conflict`
- `managed_by_source`
- `revision_conflict`
- `source_scan_incomplete`
- `apply_failed`

适用时错误响应包含资源路径，例如 `assets.demo.credential`；数据库级失败可能没有资源路径。任何错误都不得包含 password、private key 或 passphrase 内容。

## 15. 验收场景

以下场景必须有自动化测试：

1. 首次 apply 同时创建 credential、asset 和 access assignment。
2. 同一内容重复 apply 不产生数据库写入或 revision 变化。
3. 修改 credential 后只有新连接使用新材料。
4. 一个无效资源使整个批次回滚。
5. 两个文件声明同名资产时整批失败。
6. 文件试图覆盖 local 资源时返回 ownership conflict。
7. CRUD API 试图修改声明式资源时返回 managed-by-source。
8. 文件暂时消失或解析失败时不 orphan、不 prune。
9. 成功扫描缺失资源时标记 orphaned，但资源继续可用。
10. `state: absent` 删除被引用凭据时失败且无部分变更。
11. `--prune` 只影响指定 source。
12. 并发 API 修改导致 base revision 过期时 apply 失败。
13. apply 不会在日志、diff、审计或错误中泄露 secret。
14. 删除资产后新连接被拒绝，既有连接按默认策略保持。

## 16. 实现状态

v0.2 已实现统一 Catalog、严格 YAML/TOML、validate/diff/dry-run、单事务 apply、revision、ownership、absent/orphan/prune、CLI、稳定快照 watcher 和版本化 Control API。外部面板只能通过 API 提交操作，不得直接编辑数据库；SQLite 始终是资源运行事实源。
