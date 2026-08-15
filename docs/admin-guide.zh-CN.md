# Hop 管理员指南

本文面向已启动 Hop 的日常管理员。推荐使用官方面板管理本地资源；声明在 `hop.yaml` 中的资源必须通过修改文件并重启来管理。

## 网页面板

打开 Compose 的 `http://<host>:8080`，输入 `api.token`。面板默认请求当前 Origin 的 `/api/v1`，Token 只保存在当前页面内存中；刷新页面后需重新输入。独立远程 API URL 位于连接对话框的高级入口。

使用 `change-me` 时面板和后端都会警告。更换 Token 后，重新打开连接对话框认证。

面板会根据 API 的 `ownership` 在渲染操作前区分：

- `local`：可编辑、删除、轮换或修改范围；
- `config`：显示为 `hop.yaml` 管理，只读且不显示变更按钮。

## 入口公钥

入口公钥决定谁能连接 Hop。可以在面板添加，也可使用 CLI：

```bash
hop-server --config /etc/hop/hop.yaml key add \
  --name oslo-laptop \
  --public-key-file ./oslo-laptop.pub
hop-server --config /etc/hop/hop.yaml key list
```

在 `hop.yaml` 中声明时，使用 `public_key` 或 `public_key_file`，并通过 `assets` 表达范围。省略为全部，`[]` 为全部拒绝。

## 目标凭据

目标凭据用于 Hop 登录 SSH 资产。面板和 API 永远只返回 secret 的 configured/missing 状态，不返回保存值。

```bash
hop-server --config /etc/hop/hop.yaml credential list
hop-server --config /etc/hop/hop.yaml credential add-password \
  --name nas-root --username root
```

交互输入不会回显。配置文件中的 `password`、`private_key` 与 `passphrase` 是直接字符串，因此配置文件必须为 `0600`。

## 资产

```bash
hop-server --config /etc/hop/hop.yaml asset add-ssh \
  --name nas --host 192.168.1.20 --port 22 --credential nas-root
hop-server --config /etc/hop/hop.yaml asset add-tcp \
  --name metrics --host 192.168.1.30 --port 9090
hop-server --config /etc/hop/hop.yaml asset list
```

用户通过资产名称连接：

```bash
ssh -p 2222 nas@hop.example.com
```

## 会话

面板展示最近 100 条会话记录。只有仍为 `started` 且存在活动内存传输的会话能收到终止信号；过期记录可能返回“不活跃”，但历史记录仍保留。

## 备份与恢复

至少备份：

- `hop.yaml` 及其引用的入口公钥文件；
- `data_dir/hop.db`；
- `data_dir/hop.secret`；
- `data_dir/hop_host_key`。

数据库与 `hop.secret` 必须成对恢复，否则已加密的目标 secret 无法解密。恢复时先停止 Hop，完整替换文件并保持权限，再启动并检查日志。

## 安全检查

- 配置文件权限为 `0600`，不进入版本控制；
- Control API 在 Compose 中不发布到宿主机；
- 非 Compose 的远程管理使用 TLS 反向代理；
- 定期轮换网页管理 Token 与目标凭据；
- 未登记公钥连接时只出现 `Permission denied (publickey)`；
- 日志、API 响应与前端静态产物中不应出现 Token 或目标 secret。
