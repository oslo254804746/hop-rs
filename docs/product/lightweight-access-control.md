# Hop Access Key 与资产白名单方案

状态：Active，取代原“轻量权限与多管理员方案”。

产品方向：[Hop v0.2 产品方向](product-direction-v0.2.md)

## 1. 决策摘要

Hop 不再将多管理员和角色分配作为核心产品能力。

访问控制简化为：

```text
一把入口 SSH Key
→ 默认访问全部资产
→ 可选限制为明确的资产白名单
```

这个模型只决定 SSH Key 通过 Hop 可以到达哪些资产，不决定谁可以管理 Hop。它是访问面的 ACL，不是管理面的 RBAC。

## 2. 目标

1. 支持部署者为个人设备、自动化任务或团队成员添加多把 SSH Key。
2. 每把 Key 可以独立启用、禁用和吊销。
3. 默认路径不要求用户配置权限。
4. 有需要时可以把一把 Key 限制到指定资产。
5. 同一规则统一覆盖全部 Hop 连接方式。
6. 不引入 User、People、Group、Role、Policy 等额外实体。

## 3. 产品模型

### 3.1 Access Key

每个 Access Key 只包含：

- 稳定内部 ID。
- 用户可理解的名称或用途，例如 `oslo-laptop`、`ci-deploy`。
- 一把 SSH 公钥。
- 指纹。
- 启用状态。
- 资产访问模式。
- 可选的资产白名单。

名称只是标签，不表示用户账号。一个人可以添加多把 Key，但 Hop 不需要把它们聚合成 People 实体。

### 3.2 访问模式

保留当前两种模式：

- `all`：访问当前及未来新增的全部资产。
- `restricted`：只访问明确分配的资产。

行为：

- 新建 Key 默认为 `all`。
- `restricted` 加空白名单表示认证成功但看不到、也无法连接任何资产。
- 切回 `all` 时清除旧白名单；运行时访问全部资产。
- 再次切回 `restricted` 时必须显式提交新的白名单；空数组即拒绝全部资产。

## 4. 默认交互

添加 Access Key 的默认流程只需要：

1. 名称或用途。
2. SSH 公钥。
3. 保存。

保存结果默认是“可访问全部资产”。

“限制资产范围”作为可选设置：

- 默认折叠或不进入首次路径。
- 展开后允许搜索并选择资产。
- 保存前展示一句话摘要，例如“这把 Key 可以访问 3 个资产”。
- 不展示角色、权限矩阵、策略表达式或组织结构。

## 5. 配置模型

声明式资源示例：

```yaml
api_version: hop/v1alpha1

access:
  oslo_laptop:
    public_key:
      file: /data/authorized_keys/oslo-laptop.pub

  ci_deploy:
    public_key:
      env: HOP_CI_PUBLIC_KEY
    assets:
      - production_api
      - production_worker

  suspended_key:
    public_key:
      file: /data/authorized_keys/old.pub
    enabled: false
```

规范：

- 未设置 `assets` 表示 `all`。
- 设置 `assets: []` 表示 `restricted` 且无资产。
- 设置非空数组表示 `restricted`。
- `assets` 中的名称必须解析到最终 apply 状态中的资产。
- v1 不支持 tag selector、表达式、通配策略或继承。
- `public_key` 必须解析为单把有效 SSH 公钥。

## 6. 统一执行边界

Key-to-Asset 校验必须统一作用于：

- 交互式 TUI 的资产发现。
- 使用资产名作为 SSH username 的直连。
- 托管远程命令。
- SCP/SFTP。
- ProxyJump。
- 本地 TCP 转发及 RDP/VNC/数据库预设。

规则必须同时控制“是否可发现”和“是否可连接”。不能只在 UI/TUI 隐藏资产而允许手工构造目标绕过。

## 7. 运行行为

### 7.1 新连接

- Key 被禁用后，新的 SSH 认证立即失败。
- Key 从 `all` 改为 `restricted` 后，新连接立即使用新范围。
- 从白名单移除资产后，新连接立即拒绝该资产。
- 添加资产到白名单后，新连接立即可用。

### 7.2 已有连接

普通 Key 或白名单变更默认不终止已建立连接。

原因：

- 配置 apply 不应意外中断文件传输和远程命令。
- 新连接已经受新策略保护。
- 如需紧急阻断，可使用现有活动会话终止能力。

后续可以增加显式 `--terminate-active`，但不得成为默认行为。

## 8. 管理面边界

Access Key 不授予 Hop 管理权限。

- 入口 SSH Key 只用于连接 Hop 和访问资产。
- Control API Token 用于修改 Hop。
- 两者不自动关联，也不共享角色模型。
- 管理 Token 等权，不区分 Owner、Operator、Viewer。
- 如果需要互不信任的管理边界，部署多个 Hop 实例。

v0.2 不导入或保留旧多管理员数据。检测到旧数据库时安全拒绝启动，由部署者自行备份或删除。

## 9. 明确不做

- People/User 聚合实体。
- 一个人的多 Key 详情页。
- 角色和 capability 分配。
- 资产组继承。
- 基于标签的动态策略。
- 允许/拒绝规则混合及优先级。
- 时间条件、来源 IP 条件和审批。
- OIDC/SCIM 身份映射。
- Key 与 Admin 账号自动绑定。

如果未来出现大量真实需求，再通过独立 RFC 评估；不得直接扩展当前简单模型。

## 10. v0.2 Clean Break

v0.2 重新建立精简 schema，不执行 v0.1 数据迁移。

- 不读取或导入旧 `admin_users`、角色和 capability 数据。
- 不读取或导入旧 Access Key 和资产分配。
- 不提供旧 Admin Web 或 People 文案兼容层。
- 检测到 v0.1 数据库时返回清晰错误，不自动改写或删除。
- 用户确认不需要旧数据后，删除旧数据库并由 v0.2 初始化新库。

可以复用当前 all/restricted 的行为设计和端到端测试思想，但实现必须围绕新的 Catalog/Apply 边界完成，而不是为旧表结构增加兼容代码。

## 11. 验收场景

1. 新增 Key 时未指定资产，能够发现和访问全部资产。
2. 新增 `restricted` Key 并指定两个资产，只能发现和访问这两个资产。
3. 空白名单 Key 可以认证，但不能发现或连接任何资产。
4. 手工构造 ProxyJump/TCP 目标无法绕过白名单。
5. SFTP 和远程命令使用与交互式 TUI 相同的访问结果。
6. 禁用 Key 后新认证失败。
7. 白名单变更不默认终止既有连接。
8. 普通管理 CRUD 不能修改由 manifest 管理的 Key。
9. manifest 不能静默接管本地 Key。
10. 错误和审计不记录完整公钥以外的敏感凭据材料。

## 12. 完成标准

- 单人默认路径不出现权限设置。
- 添加第二把 Key 不要求创建用户。
- 限制资产范围只需要一次可选操作。
- 所有连接方式使用同一授权函数和稳定 Key/Asset ID。
- 文档始终把它描述为资产白名单，而不是角色或团队权限系统。
