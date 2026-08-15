# SSH 与 TCP 代理

Hop 当前只向用户提供两种资产类型：`ssh` 和 `tcp`。类型描述的是 Hop 如何处理连接，不是目标服务的名称。

| 类型 | Hop 的处理方式 | 适合的目标 |
|---|---|---|
| `ssh` | Hop 可以使用托管凭据连接目标 SSH 服务，也可以作为 ProxyJump 转发入口 | Linux 服务器、NAS、交换机等 SSH 服务 |
| `tcp` | Hop 只建立目标 TCP 连接并双向转发字节，不解析上层协议 | RDP、MySQL、PostgreSQL、Redis、VNC 和其他 TCP 服务 |

RDP、MySQL 等服务不需要单独的资产类型。资产名称和端口已经能表达用途，例如 `windows-rdp:3389`、`home-mysql:3306`。

Hop 暂不支持 UDP 资产。

## SSH 资产

SSH 资产可以引用一个托管凭据：

```yaml
credentials:
  nas-root:
    username: root
    private_key: |
      -----BEGIN OPENSSH PRIVATE KEY-----
      replace-with-target-private-key
      -----END OPENSSH PRIVATE KEY-----
    passphrase: "optional-passphrase"

assets:
  nas:
    type: ssh
    host: 192.168.1.20
    port: 22
    credential: nas-root
```

这是主 `hop.yaml` 的片段，不是单独的资源文件。目标私钥与密码一样直接保存在受保护的主配置中；请使用 `chmod 0600 hop.yaml`，不要提交到版本控制。

设置凭据后，Hop 可以提供以下能力：

- 在 TUI 中选择资产并建立交互式 Shell。
- 使用资产名直连。
- 执行远程命令。
- 传输 SCP 和 SFTP 文件。

```bash
ssh -p 2222 menu@hop.example.com
ssh -p 2222 nas@hop.example.com
ssh -p 2222 nas@hop.example.com 'uname -a'
scp -P 2222 backup.tar nas@hop.example.com:/tmp/backup.tar
sftp -P 2222 nas@hop.example.com
```

这里的 `nas` 是 Hop 中的资产名。目标服务器的 SSH 用户名来自 `nas-root` 凭据。

SSH 资产也可以不配置托管凭据。此时 Hop 仍允许经过授权的 TCP 转发和 ProxyJump，但不能代替用户建立托管 Shell、远程命令或 SFTP 会话。

## ProxyJump

ProxyJump 让本地 OpenSSH 客户端通过 Hop 连接目标 SSH 服务。目标认证由本地客户端完成，不使用 Hop 中保存的目标凭据。

```sshconfig
Host hop
  HostName hop.example.com
  Port 2222
  User menu
  IdentityFile ~/.ssh/id_ed25519

Host nas-via-hop
  HostName nas.hop
  Port 22
  User root
  ProxyJump hop
  IdentityFile ~/.ssh/nas_ed25519
```

```bash
ssh nas-via-hop
```

`nas.hop` 不需要公共 DNS 记录。Hop 收到转发请求后会去掉 `.hop` 后缀，并在 Catalog 中查找名为 `nas` 的资产。建议始终使用这种写法，避免把内网地址暴露在本地 SSH 配置中。

## TCP 资产

TCP 资产不保存服务自身的登录凭据。RDP 密码、数据库账号等信息仍由原生客户端管理。

```yaml
assets:
  windows-rdp:
    type: tcp
    host: 192.168.1.30
    port: 3389
    description: Windows desktop

  home-mysql:
    type: tcp
    host: 192.168.1.40
    port: 3306
    description: Home MySQL
```

使用 SSH 本地端口转发访问 RDP：

```bash
ssh -N -T -p 2222 \
  -o ExitOnForwardFailure=yes \
  -L 13389:windows-rdp.hop:3389 \
  menu@hop.example.com
```

随后让 RDP 客户端连接 `127.0.0.1:13389`。

MySQL 的使用方式相同：

```bash
ssh -N -T -p 2222 \
  -o ExitOnForwardFailure=yes \
  -L 13306:home-mysql.hop:3306 \
  menu@hop.example.com

mysql --host 127.0.0.1 --port 13306 --user app
```

本地监听端口由用户选择，目标端口应与资产配置保持一致。如果希望隧道长时间运行，可以在客户端增加 `ServerAliveInterval` 和 `ServerAliveCountMax`。

## 授权规则

每一条连接路径都使用同一套 Access Key 资产白名单：

- TUI 只显示当前 Key 可以访问的资产。
- 资产名直连、远程命令和 SFTP 会再次检查授权。
- ProxyJump 和 TCP 转发只接受 Catalog 中存在且已授权的目标。
- 手工构造 `host:port` 不能绕过白名单。

Access Key 未设置 `assets` 时可以访问全部资产。`assets: []` 表示认证成功，但不能发现或连接任何资产。非空数组是严格白名单。

## 类型选择

创建资产时按以下规则选择：

1. Hop 需要建立并管理目标 SSH 会话时，使用 `ssh`。
2. 用户通过原生客户端访问某个 TCP 服务时，使用 `tcp`。
3. 不要根据 RDP、MySQL、Redis 等应用名称增加类型别名。

新的资产类型只有在传输方式或 Hop 的处理行为发生变化时才有意义。
