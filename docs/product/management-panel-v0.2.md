# Hop 管理面板交付契约

## 当前交付

Hop 官方面板由相邻的 `hop-rs-frontend` 仓库提供，并由后端仓库根目录的 `compose.yaml` 组合交付。生产容器是静态 Nginx 运行时，不包含 Node。

默认拓扑：

```text
browser :8080 -> panel /api/v1 -> hop:8083
ssh client :2222 -------------> hop:2222
```

宿主机不发布 8083。静态服务器仅代理 `/api/v1`，其他 `/api` 路径拒绝，前端深链回退到 `index.html`。

## 普通用户工作流

1. 打开面板；
2. 输入 `hop.yaml` 的网页管理 Token；
3. 在空 Catalog 中添加入口公钥、目标凭据和资产；
4. 查看与终止活跃会话。

API 地址默认是当前 Origin。连接独立远程实例是折叠的高级选项。Token 只在页面内存中持有，不进入 localStorage、静态产物、Nginx 配置或访问日志。

## 资源归属

资源 API 响应包含最小字段：

```json
{ "ownership": "local" }
```

或：

```json
{ "ownership": "config" }
```

面板/CLI 创建的本地资源可编辑；`hop.yaml` 声明的资源在操作按钮渲染前即为只读。面板不展示内部管理标识，也不提供启动配置 Apply 工作台。

## 设置页

设置页只解释连接模式、Token 内存边界、同源代理、浏览器安全头与资源归属。它不承担 Catalog 声明文件的编辑器角色。

## 国际化

面板提供 English / 简体中文即时切换。语言偏好是唯一写入 localStorage 的面板状态；管理 Token 永不持久化。

## 发布与验证

前后端镜像共享 `HOP_VERSION` 标签。发布门槛包括 lint、typecheck、unit、production build、桌面/移动 Playwright，以及生产容器的首页、SPA 深链、认证代理、Bearer 转发、未知 API 路径和上游不可用测试。
