# Hop v0.2 产品方向：小而全的 SSH 跳板机

状态：Active，取代此前以 Admin Web、多管理员和角色能力为中心的后续路线。

确认日期：2026-08-14

相关规范：

- [声明式资源 Apply 与 SQLite 事实源规范](declarative-apply-spec.md)
- [Access Key 与资产白名单方案](lightweight-access-control.md)

## 1. 产品定位

Hop 面向个人开发者、Homelab 使用者和共享同一管理信任边界的小团队，目标是提供：

- 简单部署。
- 快速接入家中或内网资产。
- 覆盖常用 SSH 跳板机使用方式。
- 不要求外部数据库、缓存、消息队列或专属客户端。

一句话定位：

> 一个可以快速部署到家庭或内网、使用原生 SSH 客户端访问多种资产的小而全跳板机。

“小而全”中的“小”指部署模型、管理模型和用户心智足够小；“全”指连接与运维能力完整，而不是企业组织治理能力完整。

## 2. 核心用户假设

一个 Hop 实例通常由以下主体部署和管理：

- 一名个人使用者；或者
- 一个共享同一信任边界的小团队。

因此，一个实例对应一个管理信任域。Hop 不再将“同一实例内存在互不信任的多类管理员”作为核心场景。

当确实存在不同管理边界时，推荐部署多个轻量 Hop 实例，而不是在一个实例中建立企业 RBAC、组织和策略系统。

## 3. 产品原则

### 3.1 原生协议优先

用户通过已有工具使用 Hop：

- `ssh`
- `scp`
- `sftp`
- `ProxyJump`
- SSH 本地端口转发

不要求安装 Hop 专属终端客户端。

### 3.2 默认路径必须最短

首次可用路径应保持为：

```text
部署一个二进制或软件包
→ 添加一把入口 SSH Key
→ 添加凭据和资产
→ 使用原生 ssh 连接
```

默认路径不出现管理员角色、权限矩阵、组织、策略、身份源或审批概念。

### 3.3 功能完整不等于治理复杂

Hop 优先完善：

- 交互式 TUI。
- 资产名直连。
- 托管远程命令。
- SCP/SFTP。
- ProxyJump。
- 通用 TCP 转发及 RDP/VNC/数据库预设。
- 凭据托管。
- Host Key 信任。
- 多入口 SSH Key。
- 可选的按 Key 资产白名单。
- 声明式资源 apply。
- 可选 Control API。
- Docker、Linux 二进制和 OpenWrt 分发。

Hop 不以企业人员治理、合规工作台或 IAM/PAM 平台为产品完成度标准。

## 4. 管理面与访问面

Hop 明确区分两个层面：

| 层面 | 解决的问题 | v0.2 模型 |
|---|---|---|
| 管理面 | 谁可以修改 Hop | 一个管理信任域，管理凭据等权 |
| 访问面 | 哪把 SSH Key 可以访问哪些资产 | 多 Key，默认全部，可选白名单 |

### 4.1 管理面

管理面保持简单：

- 默认只有部署者需要管理 Hop。
- Control API 使用一个管理 Token；后续如支持多个 Token，它们也只用于独立吊销，不区分角色。
- 不再继续发展 Owner、Operator、Viewer。
- 不提供自定义角色、capability 编辑器或策略语言。
- 独立面板和 LuCI 都接入同一个版本化 Control API。

v0.2 是明确的 clean break。多管理员、访问级别和内置 Admin Web 直接退出新架构，不实现旧数据迁移或兼容层。进程检测到 v0.1 数据库时必须安全拒绝启动并提示备份或删除，绝不能自动覆盖、DROP 或改写旧库。

### 4.2 访问面

Hop 允许添加多把入口 SSH Key。每把 Key 是一个独立访问入口，可以单独启用、禁用和吊销。

默认行为：

- 未配置资产范围时访问当前和未来的全部资产。
- 用户主动选择限制范围时，才配置资产白名单。
- 空白名单表示可以通过公钥认证，但不能发现或访问任何资产。

这是一层简单 ACL，不是角色系统。它应统一作用于 TUI、直连、远程命令、SFTP、ProxyJump 和 TCP 转发。

## 5. 配置与状态

Hop 使用不对称混合模型：

- 启动参数以 YAML/TOML 文件为事实源。
- 资产、凭据、Access Key 和资产白名单 apply 后以 SQLite 为运行事实源。
- YAML/TOML、CLI 和 Control API 是写入同一 Catalog 的不同入口。
- 会话、审计、资产健康和 Known Hosts 属于 SQLite 或内存运行状态。

SQLite 是嵌入式实现细节，不引入外部服务，也不违背简单部署目标。

## 6. Control API 与面板

### 6.1 Control API

- 默认关闭。
- 默认监听 loopback。
- 使用版本化 `/api/v1` 协议。
- 使用管理 Token 认证，不引入管理员角色。
- 提供状态、资源、validate、diff、apply 和 reload 能力。
- 对 secret 只返回 configured/missing/changed 等状态。

### 6.2 管理面板

- 核心发行默认不包含完整 Admin Web。
- 面板拆分为独立项目和独立发布物。
- 可存在多个社区或官方面板，共同使用稳定 Control API。
- OpenWrt LuCI 插件是轻量服务与核心下载入口，不改变核心配置模型。
- 可以保留可选静态 UI 托管能力，但不得让 UI 成为核心运行前提。

## 7. OpenWrt 分发

OpenWrt 是 v0.2 的一等发行目标，而不是发布后手工打包：

- `hop-rs` Release：按 x86_64/aarch64 发布静态 musl 核心与统一 `SHA256SUMS`。
- `luci-app-hop`：`all` 架构的 LuCI/procd/UCI 控制包，缺少核心时按架构下载并验证，不在 IPK/APK 内编译或内置 Rust。
- UCI 只负责 enabled、config path、日志等服务外壳。
- 资产、凭据和 Access Key 不在 UCI 与 Hop manifest 中重复维护。
- 首批优先验证 x86_64 和 aarch64，再依据依赖兼容性扩展其他架构。
- 必须评估二进制大小、常驻内存和 SQLite 日志对路由器闪存的写入影响。

## 8. 明确不继续发展的能力

- 多管理员角色和访问级别。
- Owner、Operator、Viewer 产品模型。
- 自定义 RBAC 或 capability 编辑器。
- People 聚合实体和组织通讯录。
- OIDC、SCIM 和组织身份同步。
- 审批、JIT Access、临时提权。
- 多层组织、项目、空间和资源继承。
- 以合规报表为中心的审计工作台。
- 多节点 HA 控制平面。

保留轻量会话和变更记录，用于排错和解释运行行为；它们不扩展为合规平台。

## 9. v0.2 优先级

### P0：重新建立核心边界

1. 将本文件设为活跃产品方向。
2. 提取统一 Catalog service。
3. 实现声明式 validate/diff/apply 与 SQLite 资源归属。
4. 用回归测试保留 SSH、SFTP、远程命令、ProxyJump 和 TCP 转发的核心行为。
5. 保留多 Access Key 和可选资产白名单。

### P1：控制面解耦

1. 建立版本化 Control API。
2. 默认关闭 HTTP 管理接口。
3. 从 v0.2 核心删除内置 Admin Web、多管理员和角色代码，不保留兼容层。
4. 管理认证简化为单一信任域，不增加角色能力。

### P2：分发体验

1. Linux x86_64/aarch64 静态核心 Release。
2. OpenWrt 轻量 LuCI/procd 控制包与校验下载器。
3. Docker 发行。
4. 安装、备份、重建和故障恢复文档。

## 10. 产品完成标准

当以下体验成立时，Hop 才算符合新的“小而全”定位：

- 新用户可以在数分钟内完成部署和第一条真实 SSH 连接。
- 单人用户不需要理解角色、管理员账号或权限矩阵。
- 添加第二把 SSH Key 不需要创建用户或组织。
- Key 默认访问全部资产，限制范围是可选操作。
- 资产清单修改可以安全 validate、diff、apply，失败不影响上一份有效数据。
- 不启用 Control API 和面板时，全部核心连接能力仍可使用。
- OpenWrt 安装不要求 Docker、Node.js 或外部数据库。
- 文档首先解释连接与部署，不以 Dashboard 或团队治理作为主卖点。

## 11. 当前实现状态（2026-08-14）

已经落地：

- 全新的单一 v0.2 SQLite baseline，核心启动不再执行 v0.1 migration chain。
- v0.1 数据库在建立可写连接前只读拒绝，并有字节和修改时间不变测试。
- 统一 Catalog、资源归属、revision、source generation、orphan 和显式 prune。
- 严格 YAML/TOML manifest、离线/在线 validate、diff、dry-run 和原子 apply。
- 多 Access Key 的 `all` / `restricted` / 空白名单语义继续覆盖 SSH 运行路径。
- 核心已删除 Admin Web、管理员密码、Cookie、CSRF、角色与 capability 代码。
- `/api/v1` Control API 默认关闭；启用时使用单一等权 Bearer Token。
- Control API 已覆盖资源、本地 CRUD、会话终止、source/status、validate、diff、apply 与 reload，secret 响应只暴露状态。
- inventory source 在启动时调用统一 Apply engine；watcher 等待 scope 稳定后重扫，失败保留上一代有效 Catalog，且默认不 prune。
- OpenWrt `all` 架构 LuCI/procd/UCI 包、校验下载器，以及 x86_64/aarch64 静态核心 Release 契约。
- OpenWrt 24.10.4 IPK 与 25.12.5 APK 已由各自官方 SDK 完成云端实际打包验证。
- x86_64/aarch64 musl 核心已完成云端静态构建、ELF/运行自检、归档、统一 `SHA256SUMS` 与完整候选包验证。
- v0.2 schema 上的隔离 OpenSSH E2E 已覆盖远程命令流与退出码、PTY、SCP、SFTP、ProxyJump、Host Key 首次记录与变更拒绝，以及凭据密文。

后续真实硬件复测：

- 在真实 x86_64 与 aarch64 OpenWrt 设备上复测 RSS；获得设备前，以隔离的 x86_64 GNU/Linux 基线持续观察资源趋势。该复测不改变已经验证的首批静态核心与轻量控制包边界。
