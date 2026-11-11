# Hop 管理员指南

本文面向已启动 Hop 的日常管理员。推荐使用官方面板管理本地资源；声明在 `hop.yaml` 中的资源必须通过修改文件并重启来管理。

## 网页面板

打开 Compose 的 `http://<host>:8080`，输入 `api.token`。面板默认请求当前 Origin 的 `/api/v1`。验证成功后，Token 保存在当前标签页的 `sessionStorage` 中；刷新页面会先校验并自动重连。关闭标签页或选择“忘记连接”会清除 Token。独立远程 API URL 位于连接对话框的高级入口。

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
printf '%s\n' "$TARGET_PASSWORD" | hop-server --config /etc/hop/hop.yaml credential add \
  --name nas-root --username root --auth-type password --password-stdin
```

配置文件中的 `password`、`private_key` 与 `passphrase` 是直接字符串，因此配置文件必须为 `0600`。

## 资产、按需测延迟与 `~/.ssh/config`

```bash
hop-server --config /etc/hop/hop.yaml asset add \
  --name nas --protocol ssh --hostname 192.168.1.20 --port 22 \
  --display-name "Home NAS" --tags home,storage --credential nas-root
hop-server --config /etc/hop/hop.yaml asset add \
  --name metrics --protocol tcp --hostname 192.168.1.30 --port 9090
hop-server --config /etc/hop/hop.yaml asset list
hop-server --config /etc/hop/hop.yaml asset probe
```

用户可直接通过资产名称连接，或一键生成终端直连 / Cursor、VS Code Remote-SSH（`ProxyJump`）的 `~/.ssh/config`：

```bash
ssh -p 2222 nas@hop.example.com
hop-server --config /etc/hop/hop.yaml ssh-config --host hop.example.com --mode relay
hop-server --config /etc/hop/hop.yaml ssh-config --host hop.example.com --mode proxyjump
```

## 会话与审计事件

面板展示最近 100 条会话记录及审计事件流水（`GET /api/v1/audit-events` 或 `hop-server audit list`）。只有仍为 `started` 且存在活动内存传输的会话能收到终止信号；过期记录可能返回“不活跃”，但历史记录仍保留。已结束会话和审计事件按 `runtime.session_retention_days` 在启动时及运行期间每 6 小时自动清理。

Hop 启动时，会将上一个进程遗留的 `started` 会话标记为 `failed`，并写入
服务重启导致会话未完成的原因。该恢复发生在 SSH 和 Control API 监听器打开
之前，因此不会选中当前进程新建的会话。收到 SIGINT 或 SIGTERM 后，Hop 会停止
接受新的 HTTP/SSH 连接，通知活动传输退出，并最多等待 5 秒让 SSH 连接排空，
然后终止仍未退出的连接。

## 主机信任（Known Hosts）

Hop 首次连接 SSH 目标时通过 TOFU 保存目标的主机密钥指纹，之后每次托管连接都会核验该指纹。目标机器重装或有意轮换 SSH Host Key 后，原记录会阻止连接。

先通过目标机器的独立可信通道核实新指纹，再在面板打开“主机信任”（或执行 `hop-server known-hosts reset --hostname <host> --port <port> --key-type <type>`），选择准确的主机、端口和密钥算法，然后使用“重置信任”。服务端要求请求包含显式确认；删除后，下一次托管连接会自动建立新的 TOFU 指纹。

对应 Control API：

- `GET /api/v1/known-hosts`：列出当前信任记录；
- `DELETE /api/v1/known-hosts`：请求体必须包含 `hostname`、`port`、`key_type` 和 `confirm_reset: true`。

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
