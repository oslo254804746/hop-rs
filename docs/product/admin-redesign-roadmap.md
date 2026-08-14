# Hop Admin 改版记录（历史）

状态：Archived

本文只记录 v0.1.5 已经交付的 Admin Web 工作，不再包含未来实施任务。活跃产品方向见：

- [Hop v0.2 产品方向](product-direction-v0.2.md)
- [Access Key 与资产白名单方案](lightweight-access-control.md)
- [声明式资源 Apply 与 SQLite 事实源规范](declarative-apply-spec.md)

历史设计稿：[Hop Admin — Product & UX Direction](https://www.figma.com/design/N2bXdpZomgzmICU164L6AH)

## v0.1.5 已交付内容

v0.1.5 曾围绕内置 Admin Web 完成：

- 独立登录壳。
- 资产搜索、标签筛选和 Drawer 编辑。
- 凭据创建、轮换和使用范围保护。
- Known Hosts 指纹检查与信任重置保护。
- 基于真实数据的 Dashboard。
- SSH 会话和管理操作记录。
- 本地多管理员。
- Owner、Operator、Viewer 访问级别。
- Access Key 的 all/restricted 资产范围。
- 活动会话终止。

这些历史行为的记录以以下文档为准：

- [Hop v0.1.5 发布说明](../releases/v0.1.5.zh-CN.md)
- [Release A 实施记录](release-a-implementation-plan.md)

## 被终止的后续方向

以下方向不再进入 v0.2 路线：

- 继续扩展多管理员和访问级别。
- 自定义 RBAC 或 capability 配置。
- People 聚合实体。
- MFA、OIDC 和组织身份同步。
- 审批、JIT Access 和临时提权。
- 面向合规的复杂审计工作台。
- 将内置 Admin Web 作为核心产品入口。

## v0.2 处理原则

v0.2 是 clean break：

- 不实现 v0.1 数据迁移。
- 不保留旧 Admin Web、多管理员和角色兼容层。
- 旧数据库只检测并安全拒绝，不自动修改或删除。
- 连接协议的核心行为通过新架构和回归测试重新保证。
- 多 Access Key 与可选资产白名单继续作为核心能力。

本文不得再追加未来阶段、任务列表或验收门槛。
