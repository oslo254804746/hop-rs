# Hop 功能迭代任务清单

## 已完成 (MVP)

- [x] SSH 代理跳板功能
- [x] Admin Web UI（资产/凭据/密钥/会话管理）
- [x] TUI 交互连接
- [x] TOFU 主机密钥信任
- [x] ProxyJump 白名单
- [x] 本地 CLI 管理

---

## Phase 1: Admin UI i18n（中英文双语）

### 目标
Admin Web 界面支持中文/英文双语切换，浏览器 Accept-Language 自动检测 + 手动切换。

### 方案
自定义 Rust 静态 Struct，不引入外部 i18n 依赖。编译期安全，漏翻译即编译报错。

### 任务分解

- [x] **1.1 创建 `i18n.rs` 模块**
  - `Locale` 枚举 (En, Zh)
  - `L10n` struct（~100 个 `&'static str` 字段）
  - `static EN` / `static ZH` 翻译实例
  - `resolve_locale(headers)` — Cookie 优先 → Accept-Language → 默认 En
  - `LOCALE_COOKIE` 常量

- [x] **1.2 修改 `layout()` 函数**
  - 签名加 `t: &L10n` + 独立 `active: &str` 参数
  - `html lang=(t.locale.code())`
  - 侧栏导航文本国际化
  - 侧栏底部加语言切换链接

- [x] **1.3 修改所有页面函数**
  - `login()` — 5 个字符串
  - `overview()` — 15 个字符串
  - `assets()` + `edit_asset()` — 20+ 字符串
  - `credentials()` + `edit_credential()` — 20+ 字符串
  - `keys()` + `edit_key()` — 15+ 字符串
  - `known_hosts()` — 10 个字符串
  - `sessions()` — 10 个字符串

- [x] **1.4 修改 `routes.rs`**
  - 每个 handler 解析 locale 并传递给 html 函数
  - 新增 `GET /set-language?lang=xx&redirect=/path` 路由
  - 设置 `hop_lang` cookie（365天有效期）+ 302 重定向

- [x] **1.5 验证**
  - `cargo build` 编译通过
  - 中/英文界面均正确显示
  - 语言切换 cookie 生效
  - Accept-Language 检测生效

---

## Phase 2: 资产分组与标签筛选

### 目标
Admin UI 和 TUI 中支持按标签分组展示资产、快速筛选。

### 任务分解

- [x] **2.1 Admin UI 标签筛选**
  - 资产列表页顶部增加标签过滤器（点击标签筛选）
  - URL 参数 `?tag=prod` 支持
  - "全部" 重置按钮

- [x] **2.2 TUI 标签分组**
  - 资产列表按标签分组展示（类似 tree view）
  - 搜索支持 `tag:prod` 语法

- [x] **2.3 标签管理增强**
  - Admin UI 标签自动补全（基于已有标签）
  - 标签批量编辑（选中多个资产修改标签）

---

## Phase 3: 批量导入导出

### 目标
支持 CSV/JSON 格式批量导入导出资产和凭据，方便迁移和备份。

### 任务分解

- [x] **3.1 导出功能**
  - Admin UI 资产列表页 "导出" 按钮
  - 支持 CSV 和 JSON 格式
  - 凭据导出仅含元数据（不含加密密钥）

- [x] **3.2 导入功能**
  - Admin UI 文件上传表单
  - CSV/JSON 解析 + 校验
  - 冲突处理策略（跳过/覆盖/报错）
  - 导入结果摘要展示

- [x] **3.3 CLI 导入导出**
  - `hop-server export --format csv --output assets.csv`
  - `hop-server import --file assets.csv --on-conflict skip`

---

## Phase 4: SSH 直连模式

### 目标
支持类似 JumpServer 的直连语法：
```
ssh -p 2222 user@target_asset@hop_server
```
无需 TUI 交互，直接通过用户名编码目标信息实现直连。

### 任务分解

- [x] **4.1 用户名解析**
  - SSH 用户名格式：`<key_owner>@<asset_name>` 或 `<key_owner>@<asset_hostname>`
  - 解析逻辑放入 SSH auth handler

- [x] **4.2 直连路由**
  - 认证成功后判断是否为直连模式
  - 直连模式跳过 TUI，直接建立到目标的连接
  - 使用资产绑定的凭据或 agent forwarding

- [x] **4.3 权限控制**
  - 验证该 key 是否有权访问目标资产
  - 审计日志记录直连事件（mode: "direct"）

- [x] **4.4 文档与示例**
  - SSH config 配置示例
  - ProxyJump vs 直连模式对比说明

---

## 优先级排序

| Phase | 功能 | 优先级 | 预估工作量 |
|-------|------|--------|-----------|
| 1 | i18n 中英文 | P0 | 中 |
| 4 | SSH 直连模式 | P1 | 中 |
| 2 | 标签筛选 | P2 | 小 |
| 3 | 批量导入导出 | P3 | 中 |
