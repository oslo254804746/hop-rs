# Hop 当前产品方向

## 定位

Hop 是小型、自托管、默认安全的 SSH/TCP 跳板：入口公钥认证、按资产授权、加密目标凭据、统一会话记录，以及无需数据库知识的官方网页面板。

## 两条清晰路径

1. **Compose + 面板（推荐）**：把 `examples/panel-first.yaml` 复制为唯一的 `hop.yaml`、更换网页管理 Token、启动 Compose，在空 Catalog 中从网页创建本地资源。
2. **单 YAML、无面板**：把 `examples/config-first.yaml` 复制为同一个 `hop.yaml`，在其中声明运行参数与资源，验证后直接启动。

两条路径共享同一个 Catalog 与权限模型，不要求用户理解内部数据库或 Apply 实现。

## 安全边界

- 入口 SSH 只宣告 `publickey`；
- Control API 默认关闭，Compose 中仅在私有网络可达；
- 面板同源代理只开放 `/api/v1`；
- 网页管理 Token 只在浏览器内存；
- 目标 secret 加密保存且永不回读；
- YAML secret 采用直接字符串，因此文件权限与备份被视为部署安全边界。

## 管理边界

- `local` 资源来自面板或本地命令，可在面板编辑；
- `config` 资源来自 `hop.yaml`，面板只读；
- API 在资源响应中提供最小 ownership，UI 在展示操作前做出判断；
- 后端事务与冲突检查始终是最终一致性防线。

## 用户体验

- 默认连接当前面板 Origin，只输入 Token；
- 独立远程 API URL 是高级选项；
- 设置页关注连接安全与 ownership，不暴露内部配置编排；
- 面板提供 English / 简体中文切换；
- 面板展示 Known Hosts 完整指纹，并通过显式确认保护 TOFU 信任重置；
- `change-me` 在后端和面板都有清晰告警。

## 交付标准

前后端使用匹配版本标签，文档以 Compose 面板路径为首，单 YAML 为第二路径。发布必须通过 Rust、Node、Docker、Compose、桌面/移动浏览器和真实 OpenSSH 全链路验证。
