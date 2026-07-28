# Release A：Admin Usability 实施计划

状态：首个开发切片已完成（2026-07-28）

范围：`Phase 0 Admin 基础壳` + `Phase 1 登录/资产高频体验` 的首个切片
总路线图：[Hop Admin 改版总路线图](admin-redesign-roadmap.md)

## 本轮目标

1. 未登录页面不再暴露完整 Admin 导航。
2. 登录表单具备正确语义、自动填充和错误关联。
3. 资产列表提供真实文本搜索与标签组合筛选。
4. 添加资产从常驻右栏改为按需打开的 Drawer。
5. 批量标签操作只有在选中资产后出现。
6. `hostname:port` 保持完整、可复制，不再逐字换行。
7. 保持现有单管理员密码、CSRF、资产 CRUD 和导入导出行为兼容。

## 实施结果

- Workstream A/B/C 已完成集成。
- 登录页使用独立未认证壳，不再泄露内部导航；密码输入支持密码管理器与可访问错误提示。
- 资产页支持文本搜索、标签组合筛选、URL 状态恢复、按需新增 Drawer、渐进式批量操作和区分后的空状态。
- 资产表格在 1440px、1024px 与 520px 下完成实测；移动端宽表只在自身容器内滚动。
- `cargo fmt --all -- --check` 通过。
- `cargo test --workspace` 通过，共 111 项测试。
- 本切片未改变数据库模型、现有单管理员密码、SSH 访问、CSRF 或资产 CRUD 协议。

仍留在总路线图后续切片：

- 编辑资产 Drawer 与创建表单复用。
- 目标地址一键复制按钮；本切片先完成地址完整展示和文本选择。
- 登录失败恢复/锁定策略。
- 凭据 Drawer、凭据影响范围与 Known Host 风险文案。

## 并行工作流

### Workstream A：登录与未认证壳

负责文件：

- `crates/hop-server/src/admin/html.rs` 中的 layout/login 区域。
- `crates/hop-server/src/admin/release_a_login.css`。

任务：

- 为登录页提供独立、无侧边栏的页面壳。
- 保留品牌、语言切换和 loopback 安全语境。
- 为密码输入增加 `autocomplete="current-password"`。
- 将登录错误与输入字段通过 `aria-describedby` 关联。
- 保持登录成功、失败和语言切换现有路由兼容。
- 补充 HTML 渲染测试。

### Workstream B：资产列表与 Drawer

负责文件：

- `crates/hop-server/src/admin/html.rs` 中的 assets 区域。
- `crates/hop-server/src/admin/release_a_assets.css`。

任务：

- 重组资产页工具栏和状态摘要。
- 增加搜索输入，提交到 `/assets?q=...`。
- 标签链接保留当前搜索条件。
- 添加资产表单放入 `dialog`/Drawer，并支持打开、关闭、Esc 与点击遮罩。
- 表格目标地址使用不可逐字换行的复制友好结构。
- 批量工具栏根据选中项数量显示。
- 空状态区分“没有资产”和“当前筛选无结果”。
- 补充 HTML 渲染测试。

### Workstream C：资产查询后端

负责文件：

- `crates/hop-server/src/admin/routes.rs`。

任务：

- `AssetsQuery` 增加可选 `q`。
- 文本搜索覆盖名称、主机、端口、描述、标签、协议和 preset。
- 文本搜索与标签筛选可组合。
- 搜索比较忽略 ASCII 大小写并忽略首尾空格。
- 将当前搜索词传入 `html::assets`。
- 补充筛选 helper 单元测试。

## 集成顺序

1. 合并 Workstream C，确定 `html::assets` 的搜索参数契约。
2. 合并 Workstream A，验证登录页不影响已认证 layout。
3. 合并 Workstream B，解决 `html::assets` 签名和搜索 URL。
4. 运行 `cargo fmt --all`。
5. 运行 `cargo test -p hop-server admin`。
6. 运行 `cargo test --workspace`。
7. 启动临时 Admin Web，在 1440px 与窄屏检查登录和资产页。

## 完成定义

- 现有单管理员无需迁移或重新设置密码。
- 未登录 HTML 中没有资产、凭据、密钥、审计和设置导航链接。
- 搜索和标签筛选刷新后仍可恢复。
- 资产 Drawer 可通过键盘打开和关闭，关闭后焦点返回触发按钮。
- 没有选择资产时，批量标签控件不可见。
- 删除资产仍使用原有 CSRF 保护。
- 关键 HTML 结构有自动化测试。
- `cargo test --workspace` 通过。
