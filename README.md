# Hop

Hop 是一个约 3.7 MB、零目标端 Agent 的自托管 SSH 中继与凭据保险库：入口只接受已登记的公钥，目标 VPS 凭据加密保存，支持托管模式 `-L`/`-D` 端口转发与 Cursor / VS Code Remote-SSH 免密接入、`via_asset` 跨内网链式多级跳板、新机一键密钥洗密、定时协议延迟拨测与 Telegram / Bark / Webhook 告警通知，以及审计流水与 `~/.ssh/config` 生成器。

## 快速开始：网页面板（推荐）

不需要前端源码或本地构建。下载本仓库的 `compose.yaml` 和 `examples/`，然后执行：

```bash
cp examples/panel-first.yaml hop.yaml
sed -i 's/token: change-me/token: 请替换为随机长字符串/' hop.yaml
chmod 0600 hop.yaml
docker compose pull
docker compose up -d
```

默认打开：

- 面板：`http://localhost:8080`
- SSH：`localhost:2222`

首次打开面板时，只需输入 `hop.yaml` 中的网页管理 Token。面板与后端使用同源 `/api/v1`，后端管理端口不会发布到宿主机。初始 Catalog 为空，可直接在网页中添加入口公钥、目标凭据和资产（含 `via` 上游跳板），支持新机一键生成并下发 Ed25519 公钥洗密、按需与后台定时探测 SSH/TCP 协议握手延迟（含 RTT 火花线与 Telegram / Bark / Webhook 告警）、查看审计事件、导出 `~/.ssh/config`，以及在目标重装或 Host Key 轮换后核实并重置 Known Hosts 记录。

Compose 始终只读挂载当前目录中的一份 `hop.yaml`。`examples/panel-first.yaml` 和 `examples/config-first.yaml` 只是二选一的起始模板，不会同时加载、继承或合并。

## 单 YAML、无面板

复制完整示例并填写入口公钥与目标信息：

```bash
cp examples/config-first.yaml hop.yaml
chmod 0600 hop.yaml
cargo run --release -p hop-server -- --config ./hop.yaml config validate
cargo run --release -p hop-server -- --config ./hop.yaml serve
```

一份 `hop.yaml` 同时包含监听地址、数据目录、网页管理 Token、SSH 运行参数、定时拨测与告警通道、目标凭据、资产（含 `tags`、`display_name`、`via_asset` 与 `allow_tcp_forwarding`）和入口公钥。启动时 Hop 先用内部原子 Apply 引擎应用文件中的资源；任何错误都会阻止监听器启动，不会留下半套 Catalog。

## 配置文件与示例

- [`examples/panel-first.yaml`](examples/panel-first.yaml)：启用面板 API，Catalog 初始为空，资源由网页创建。
- [`examples/config-first.yaml`](examples/config-first.yaml)：在 YAML 中声明凭据、资产和入口公钥，适合无面板部署。
- `hop.yaml`：从其中一个示例复制出来的唯一运行配置；已被 `.gitignore` 排除。

`password`、`private_key`、`passphrase` 以及通知通道密钥都是 YAML 直接字符串；因此配置文件必须使用 `chmod 0600` 并排除在版本控制之外。入口公钥必须在 `public_key` 与 `public_key_file` 中二选一。相对路径以配置文件所在目录为基准。

`api.token` 缺失或为空时，仅禁用 Control API，SSH 仍正常启动。`change-me` 只用于首次体验；后端与面板都会警告，生产环境必须替换。

## 资源归属

- 面板或本地命令创建的资源归属为 `local`，可在面板编辑和删除。
- `hop.yaml` 声明的资源归属为 `config`，面板在显示操作按钮前就将其标为只读。
- 修改配置管理的资源后重启 Hop；启动 Apply 会更新它们，并删除配置中已移除的同归属资源，不影响本地资源。

## 连接与本地接入

```bash
# 直连托管 SSH 资产（Hop 自动使用保险库凭据登录目标机，同时支持 -L / -D 端口转发）
ssh -p 2222 <asset-name>@<hop-host>
ssh -p 2222 -L 15432:127.0.0.1:5432 <asset-name>@<hop-host>

# 打开交互式 TUI 选择器（支持 / 搜索、#标签过滤、r 键测延迟；选机后 GUI 客户端 SFTP 自动跟随并在退出时同步回收）
ssh -p 2222 hop@<hop-host>

# 新机密码登录一键洗密为 Ed25519 公钥凭据
printf '%s\n' "$ROOT_PASSWORD" | hop-server --config ./hop.yaml asset bootstrap-key <asset-name> --password-stdin

# 生成托管免密直连（支持 Cursor / VS Code Remote-SSH 与端口转发）或端到端 ProxyJump 的 ~/.ssh/config
hop-server --config ./hop.yaml ssh-config --host <hop-host> --mode relay
hop-server --config ./hop.yaml ssh-config --host <hop-host> --mode proxyjump
```

Hop 的入口 SSH 只宣告 `publickey` 认证。未登记的公钥会直接得到 `Permission denied (publickey)`，不会回退到密码提示。

## 文档

- [v0.3.0 发布说明](docs/releases/v0.3.0.zh-CN.md)
- [部署指南](docs/deployment.zh-CN.md)
- [SSH 与 TCP 代理](docs/proxying.zh-CN.md)
- [配置参考](docs/configuration.zh-CN.md)
- [管理员指南](docs/admin-guide.zh-CN.md)
- [English README](README-EN.md)

## 开发验证

```bash
cargo fmt --all --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
shellcheck docker-entrypoint.sh scripts/*.sh
docker compose config
```

许可证：MIT。
