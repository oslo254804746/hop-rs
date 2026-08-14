# Hop v0.2 管理面板交付边界

状态：Active

确认日期：2026-08-14

相关契约：

- [Hop v0.2 产品方向](product-direction-v0.2.md)
- [Control API 与本地管理](../admin-guide.zh-CN.md)
- [声明式资源 Apply 与 SQLite 事实源规范](declarative-apply-spec.md)

## 1. 当前缺口

v0.2.0 已交付无浏览器依赖的 Hop 核心和版本化 Control API，但没有交付可用的资源管理面板：

- `hop-rs` 不再包含 v0.1 Admin Web。
- `luci-app-hop` 当前 LuCI 页面只负责服务启停、启动配置路径、日志选项和核心下载。
- 资产、凭据、Access Key、白名单和会话仍只能通过 CLI、manifest 或直接调用 Control API 管理。

因此，Control API 已经解耦不等于管理体验已经完成。文档和发布说明在面板真正交付前不得暗示 Hop 或 `luci-app-hop` 已提供资源管理 UI。

## 2. 产品决策

v0.2 必须提供官方、可选的图形管理入口，但不恢复 v0.1 的内置 Admin Web 架构：

1. Hop 核心继续可以完全无面板运行，不嵌入管理员账号、登录态、角色或前端构建链。
2. 通用 Hop 面板作为独立项目和独立发布物，通过 `/api/v1` 管理一个 Hop 实例。
3. `luci-app-hop` 增加 Catalog 资源管理页面；它与通用面板遵循同一信息架构和 Control API 行为。
4. 两个入口都不得直接读写 SQLite，也不得在 UCI 中复制资产、凭据、Access Key 或白名单。

这两个入口是同一产品能力的两种交付形态，不应发展成两套不同的资源模型。

## 3. 连接与认证边界

### 3.1 通用面板

通用面板直接调用用户配置的 Hop Control API：

- 使用一枚等权 Bearer Token，不增加面板专属账号或角色。
- Token 默认只保存在当前会话内，不写入浏览器长期存储。
- 远程访问必须由部署者提供 TLS 和网络访问控制；CORS 不是安全边界。
- 面板不代理 SSH/TCP 数据面流量。

### 3.2 LuCI 面板

LuCI 浏览器页面不得读取 Hop Token。`luci-app-hop` 应通过受 LuCI ACL 保护的 rpcd 后端，把允许的管理请求转发到 loopback Control API：

```text
LuCI 页面
→ LuCI session 与 rpcd ACL
→ luci-app-hop 管理代理
→ 127.0.0.1 上的 Hop /api/v1
```

管理 Token 只保存在路由器上、由 `hop` 服务和受限代理读取。代理必须限制目标地址、HTTP 方法和 `/api/v1` 路径，不能成为通用 HTTP 转发器。

## 4. 首个可用版本范围

两个面板都应覆盖相同的高频任务：

- 概览：版本、运行状态、Catalog revision 和配置 source 状态。
- 资产：查看、新增、编辑和删除本地 `ssh` / `tcp` 资产。
- 凭据：创建、轮换和删除；已有 secret 只显示 configured/missing。
- Access Key：新增、启用、禁用、删除，以及 all/restricted/空白名单配置。
- 会话：查看最近会话并显式终止仍在运行的会话。
- 声明式配置：validate、diff、apply、reload、revision conflict 和 source 错误。

声明式资源在面板中必须清楚标记管理来源。面板不能把 `managed_by_source` 伪装成普通保存失败，也不能提供隐式 takeover。

首个版本不包含：

- 面板自身的用户、团队或角色系统。
- Owner、Operator、Viewer 或 capability 分配。
- OIDC、SCIM、审批、JIT Access 或组织层级。
- 合规报表和多节点控制平面。

## 5. 信息架构

通用面板和 LuCI 页面使用同一组任务名称：

| 区域 | 主要任务 |
|---|---|
| Overview | 确认实例、版本、运行状态和需要处理的问题 |
| Assets | 管理 SSH/TCP 目标和目标凭据关联 |
| Credentials | 写入或轮换目标 SSH 凭据，不回显 secret |
| Access | 管理入口 Key、启用状态和资产范围 |
| Sessions | 查看连接结果并终止活动会话 |
| Configuration | 查看 source、validate/diff/apply/reload |
| Service | 仅 LuCI：服务启停、核心版本、下载和日志 |

`Service` 属于 OpenWrt 外壳，不进入 Hop Catalog。其余区域都通过 Control API 工作。

## 6. 发布与完成标准

通用面板和 LuCI 资源面板可以分阶段交付，但在发布物存在前必须明确标记为未交付。v0.2 的图形管理体验在以下条件同时成立后才算完成：

1. Linux/Docker 用户可以安装官方面板发布物，不需要 Node.js 构建环境。
2. OpenWrt 用户可以从 `Services -> Hop` 完成首条 Access Key、凭据和资产配置。
3. 两种入口对 ownership、secret、revision 和会话终止具有一致语义。
4. LuCI 浏览器上下文拿不到 Control API Token。
5. 面板不可用或 Control API 关闭时，CLI、manifest 和全部 SSH/TCP 核心路径不受影响。
6. 发布文档明确区分 Hop 核心、通用面板和 `luci-app-hop` 的版本与安装方式。

视觉设计与前端实现需要另行选择视觉目标；本文件只确定产品、数据和安全边界。
