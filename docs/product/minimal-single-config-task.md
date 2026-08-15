# Hop 极简单配置重构任务

## 任务性质

本文件是下一开发会话的实施任务书，不是当前行为说明，也不是要求用户照做的部署文档。

目标是重新收紧 Hop 的产品边界：用户只准备一份配置文件，就能启动 Hop，并同时声明入口公钥、目标凭据和资产；不熟悉配置文件的用户则应通过官方网页面板完成日常管理。当前的“启动配置 + 资源 manifest + inventory source + secret source + apply 命令”虽然能力完整，但理解成本和部署步骤已经偏离个人开发者、Homelab 场景下的极简定位。

本任务跨两个仓库：

- 后端：`/home/oslo/projects/hop-rs`
- 前端：`/home/oslo/projects/hop-rs-frontend`

## 用户反馈整理

当前体验存在以下问题：

1. 用户需要理解“启动配置”和“资源清单”是两类文件。
2. 为了让资源随服务启动，还要理解并配置 `inventory.sources`、`id`、`path`、`watch` 和 `prune`。
3. Docker 路径又额外引入了 `HOP_BOOTSTRAP_FILE`，形成第三种表面用法。
4. 密码使用 `password.file` 或 `password.env` 间接引用，对家庭部署来说步骤过多。
5. 入口 Access Key、目标 SSH 凭据和 SSH Host Key 都包含“key”概念，现有文档没有在第一屏清楚地区分它们。
6. 快速开始包含复制 secret、查询数据库 ID、`docker exec`、apply/source 等实现概念，使用者很难判断哪些步骤是必需的。
7. 完整示例重复展示 TOML/YAML 和大量高级配置，掩盖了最小可运行路径。
8. Docker 示例只启动后端，没有把已经完成的官方网页面板放进默认体验。
9. 面板目前还要求用户理解 Control API URL、端口和 CORS；这对非技术用户没有必要。
10. `api.token_file` 要求预先创建第二个文件，文件缺失或为空还会让整个 server 退出。
11. `cors_allowlist = ["192.168.11.1"]` 看起来直观，实际却不是合法浏览器 Origin；Origin 必须包含 scheme，通常还包含端口。

## 产品结论

### 1. 对外只保留一份配置

`hop-server --config /path/to/hop.yaml serve` 必须同时完成：

- 加载监听和运行设置；
- 创建或打开 SQLite、Master Key 和 SSH Host Key；
- 校验并注册配置中声明的入口公钥、凭据和资产；
- 全部成功后再打开 SSH/API 监听端口。

不再要求用户准备单独的 `resources.yaml`，也不再把 `inventory.sources` 作为公开配置能力。修改配置后重启服务即可；本任务不需要文件 watcher 或热加载。

### 2. 复用原子 Apply 内核，但隐藏其复杂度

现有 Catalog Apply engine 已经提供严格校验、原子提交、稳定名称引用、加密保存和幂等 revision，应继续作为内部实现复用，不要重新写一套资源写库逻辑。

统一配置中的资源在启动时转换为内部 Manifest，并使用固定的内部 source ID，例如 `startup-config`。该 source ID 不出现在用户配置或快速文档中。

启动语义：

- 第一次启动创建资源；
- 相同配置重启不增加 revision；
- 修改配置后重启，原子更新对应资源；
- 从配置中删除资源时，隐式 prune 仅删除 `startup-config` 自己管理的资源；
- CLI/API 创建的本地资源不应被统一配置误删；
- 配置无效、引用无效或 secret 无效时，整个资源 scope 不写入，并且监听端口不得启动。

### 3. 密码直接写标量

目标 SSH 密码改成直接字符串：

```yaml
credentials:
  nas-root:
    username: root
    password: "target-password"
```

不再要求或支持以下公开写法：

```yaml
password:
  file: /data/secrets/nas.password
```

```yaml
password:
  env: HOP_NAS_PASSWORD
```

密码进入 SQLite 前仍必须使用 Master Key 加密。日志、diff、API、错误信息和 Debug 输出不得包含明文。

接受明文配置的安全代价是配置文件本身成为敏感文件。部署文档必须明确要求：

- 配置文件权限设为 `0600`；
- 不提交到 Git；
- Docker 以只读 bind mount 挂载配置；
- 示例只能使用明显的占位密码。

这是有意识的产品取舍：用一个受保护的文件换取更低的家庭部署认知成本。

### 4. 入口公钥可使用字符串或文件

“能连接 Hop 的密钥”在本任务中明确指入口 Access Key 的公钥，不是 Hop 登录目标资产使用的私钥，也不是 Hop 自身的 SSH Host Key。

推荐公开结构：

```yaml
access_keys:
  laptop:
    public_key: "ssh-ed25519 AAAA... laptop"
    assets: [nas]
```

或者：

```yaml
access_keys:
  laptop:
    public_key_file: ./laptop.pub
    assets: [nas]
```

规则：

- `public_key` 与 `public_key_file` 必须二选一；
- 公钥不是 secret，允许文件引用是为了直接复用现有 `*.pub`；
- 相对路径按主配置文件所在目录解析，不按进程工作目录解析；
- `assets` 省略表示可以访问全部资产，空数组表示不能访问任何资产，非空数组表示白名单；
- 文档统一使用“入口公钥”或“Access Key”，不要只写含义模糊的“密钥”。

### 5. 目标私钥保持单文件原则

目标 SSH 私钥凭据若继续支持，应使用 YAML 多行字符串直接写入同一配置文件：

```yaml
credentials:
  nas-key:
    username: root
    private_key: |
      -----BEGIN OPENSSH PRIVATE KEY-----
      ...
      -----END OPENSSH PRIVATE KEY-----
    passphrase: "optional-passphrase"
```

不要继续暴露通用的 `{ file, env }` `SecretSource` 联合类型。入口公钥是唯一明确允许字符串/文件二选一的用户字段。

### 6. Compose 默认交付后端与官方面板

官方面板已经位于 `/home/oslo/projects/hop-rs-frontend`，是 Vue 3 + TypeScript + Vite 的静态应用。Docker 快速开始应以 Compose 为主入口，一次启动：

- `hop` 后端；
- `panel` 静态站点与窄范围反向代理；
- 一个持久化数据目录；
- 一个只读主配置文件。

用户启动后直接打开面板地址，通过 Assets、Credentials 和 Access 页面完成配置。README 第一屏应明确告诉不熟悉 YAML 的用户：“启动后打开网页面板即可管理资产、目标凭据和入口公钥。”

单独的 `docker run` 和纯二进制启动仍可保留为无面板/高级路径，但不再承担最友好的新用户引导。

### 7. `api.token` 直接配置，Token 问题不能拖垮 SSH

删除公开的 `token_file`，改为直接字符串：

```yaml
api:
  enabled: true
  listen: 0.0.0.0:8083
  token: "change-me"
```

要求：

- 示例和首次生成的配置使用明显的提示值 `change-me`，旁边直接说明需要修改；
- 使用提示值时服务可以启动，但每次启动必须输出明确且不包含其他 secret 的安全警告；
- `token` 缺失或为空时，不能让 SSH server 一起退出；应记录可操作警告并保持 Control API 关闭；
- `token_file` 从新配置模型和普通文档中删除；
- token 不得出现在 Debug、错误、普通启动日志、API 响应或前端构建产物中；
- API 仍默认关闭，只有 Compose 面板方案显式启用；
- Compose 不把后端 `8083` 发布到宿主机，降低示例 token 未及时修改时的暴露范围。

这里的“默认提示”是降低首次启动失败率的产品选择，不代表 `change-me` 是安全凭据。正式部署文档必须要求修改，前端连接成功后也应对提示值显示醒目但不阻断的警告。

### 8. 默认同源代理，普通用户不配置 CORS

前端容器应同时提供静态资源和一个窄范围反向代理：

```text
browser -> http://hop-host:8080/        -> panel static files
browser -> http://hop-host:8080/api/v1  -> hop:8083/api/v1
```

因此默认 Compose 路径具备以下性质：

- 浏览器只访问一个 Origin；
- 面板不要求用户输入 API URL；
- 后端 `8083` 只在 Compose 内部网络暴露；
- `cors_allowlist` 可以保持空数组；
- 浏览器继续自己发送 Bearer Token，代理不得把 token 编译进静态文件或写入响应。

当前“非 loopback API 必须配置非空 CORS allowlist”的校验需要删除。监听在容器内部的 `0.0.0.0` 并不等于浏览器跨域，CORS 也不是网络访问控制。

直接跨域部署仍可支持 `cors_allowlist`，但必须校验完整 Origin，例如：

```yaml
cors_allowlist:
  - http://192.168.11.1:8080
  - https://hop.example.com
```

`192.168.11.1` 这种裸主机值应在启动前得到清晰错误，而不是悄悄配置成永远匹配不到的 HeaderValue。

### 9. 面板优先与配置优先是两条清楚路径

官方 Compose 示例采用“面板优先”：主配置只放运行参数和 API token，资产、凭据和入口公钥由面板写入 SQLite。这样面板创建的资源不会与 `startup-config` ownership 冲突。

需要完全声明式、无网页的用户可以采用“配置优先”：在同一主配置中声明 `credentials`、`assets` 和 `access_keys`，这些资源由启动配置管理，面板只读展示或明确提示“请修改 hop.yaml”。

不要让同一资源同时看起来既能在网页修改、又会在重启时被配置覆盖。后端 API 应向前端提供足够的 ownership 信息，前端在显示编辑按钮前就能区分：

- panel/local managed：允许网页编辑；
- config managed：只读，并给出修改主配置的提示。

普通用户不需要学习 source ID、revision、orphan 或 prune；界面只使用“由网页管理”和“由配置文件管理”这两个说法。

## 推荐的最小配置

下面应成为无面板/配置优先路径的主示例。字段名可以在实现前做一次一致性校对，但不能重新引入 source、manifest 或数据库 ID 概念。

```yaml
listen: 0.0.0.0:2222
data_dir: /data

credentials:
  nas-root:
    username: root
    password: "change-me"

assets:
  nas:
    host: 192.168.1.20
    credential: nas-root

access_keys:
  laptop:
    public_key: "ssh-ed25519 AAAA... laptop"
    assets: [nas]
```

默认值应承担大部分配置：

- `listen` 默认 `0.0.0.0:2222`；
- `data_dir` 默认当前目录，Docker 镜像中默认 `/data`；
- SSH 资产默认 `type: ssh` 和 `port: 22`；
- 凭据含 `password` 时自动判定为 password，含 `private_key` 时自动判定为 ssh key；
- `data_dir` 自动派生 SQLite、Master Key、SSH Host Key 等运行文件路径；网页管理 Token 直接来自 `api.token`；
- 只有 TCP 资产需要显式 `type: tcp`；
- 高级字段可以继续存在，但不得出现在最小示例中。

需要保留严格的冲突校验，例如同时提供 `password` 和 `private_key`、同时提供 `public_key` 和 `public_key_file`、TCP 资产引用 SSH 凭据等都应在监听前报出带字段路径的错误。

Compose 的面板优先示例应更短：

```yaml
listen: 0.0.0.0:2222
data_dir: /data

api:
  enabled: true
  listen: 0.0.0.0:8083
  token: "change-me"
```

用户启动后进入网页添加 Access Key、Credential 和 Asset。面板必须把三者分别解释为“谁能连接 Hop”“Hop 用什么登录目标”“Hop 可以连接到哪里”。

## API 配置评估

Control API 在纯后端模式下默认关闭，在官方 Compose 面板模式下默认启用。唯一公开的认证配置是 `api.token`，不再支持或展示 `token_file`。

`api.enabled = true` 且 `token` 缺失/为空时，只禁用 Control API 并记录警告，SSH/TCP 核心继续启动。`token = "change-me"` 时 API 可以启动，但后端和面板都必须提示用户修改。不能用 panic、找不到 token 文件或空文件错误让整个进程退出。

API token 是管理凭据，不是 Hop 入口 SSH 密码，也不是目标资产密码。文档和 UI 必须使用“网页管理 Token”这一名称。

## 建议的实现范围

### 配置模型

- 在 `HopConfig` 中直接加入 `credentials`、`assets` 和 `access_keys`。
- 引入 `data_dir`，为数据库、Master Key、Host Key 等提供派生默认路径。
- 相对文件路径统一相对于主配置文件目录解析。
- 将 `ApiConfig.token_file: PathBuf` 替换为不会泄漏 Debug 内容的 `token` 字段。
- 删除 `control_api::load_token` 和 token 文件存在性校验。
- 删除公开的 `InventoryConfig`、`InventorySourceConfig` 和 `inventory.sources`。
- 将资源配置转换为现有内部 Manifest/ResolvedManifest，再调用 Catalog Apply。
- 将 direct scalar secret 与 Catalog 加密边界连接起来，避免 secret 在不必要的中间结构中 Clone/Serialize/Debug。
- 对 CORS 明确 Origin 做 URI 级校验；拒绝裸 IP/主机名和带 path 的值。

### 启动流程

- `serve` 打开运行存储后，先 apply 主配置中的资源，再绑定监听端口。
- 使用固定内部 source 和隐式 source-scoped prune。
- 启动失败必须返回可操作的配置字段路径，不得只打印 `apply_failed`。
- 不实现 watcher；配置变更通过重启生效。
- API token 无效只能影响 Control API 子服务，不能阻止 SSH listener 启动。
- 删除“非 loopback listener 强制非空 CORS allowlist”的重复校验；空 allowlist 表示不授权跨域浏览器请求。

### CLI/API 边界

- CLI 和 Control API 的本地 CRUD 可以保留，作为可选高级管理方式。
- `config validate` 应直接校验 `--config` 指定的统一配置，不再要求额外 `-f resources.yaml`。
- 单独的 `apply -f`、`config diff -f`、source status/reload 是否彻底删除，可按代码依赖决定；即便暂时保留，也必须退出 README、部署指南和普通配置指南，标为内部/高级兼容能力。
- 不能再要求用户查询 credential UUID；资产始终通过稳定配置名引用凭据。
- 资源 list/detail API 应返回最小 ownership 枚举，让面板在操作前知道资源是否可编辑；不得只靠提交后 `409 managed_by_source` 猜测。
- 面板优先路径创建的资源保持 local ownership；配置优先路径创建的资源保持 config ownership。

### Docker

- 删除 `HOP_BOOTSTRAP_FILE` 和 `HOP_BOOTSTRAP_SOURCE` 这条即将被统一配置取代的路径。
- 后端镜像只需要挂载一个配置文件和一个持久化数据目录。
- 单后端的高级形态应接近：

```bash
docker run -d --name hop --restart unless-stopped \
  -p 2222:2222 \
  -v "$PWD/hop.yaml:/etc/hop/hop.yaml:ro" \
  -v "$PWD/data:/data" \
  hop:local hop-server --config /etc/hop/hop.yaml serve
```

- entrypoint 可以负责创建 `/data` 和修正运行用户权限，但不能再执行另一套资源 apply 命令。
- 仓库根目录新增官方 `compose.yaml`，源码 checkout 相邻时可分别使用 `.` 和 `../hop-rs-frontend` 作为 build context。
- 发布版本的 Compose 示例使用两个版本匹配的 GHCR 镜像，不要求普通用户 clone 两个源码仓库。
- Compose 只发布 `2222`（SSH）和面板端口（建议 `8080`）；后端 `8083` 只使用 `expose`。
- 前后端服务使用同一私有 Compose 网络；面板代理 upstream 固定为 `http://hop:8083`。
- 数据目录持久化，配置文件只读挂载；不得把 token 放进 Compose command、镜像 label 或前端 build arg。

概念示例：

```yaml
services:
  hop:
    build: .
    command: ["hop-server", "--config", "/etc/hop/hop.yaml", "serve"]
    ports:
      - "2222:2222"
    expose:
      - "8083"
    volumes:
      - ./hop.yaml:/etc/hop/hop.yaml:ro
      - ./data:/data

  panel:
    build: ../hop-rs-frontend
    depends_on:
      - hop
    ports:
      - "8080:80"
```

最终 Compose 应补充合理的 restart policy、只读静态容器文件系统、SPA fallback 和可执行的健康/启动验证，但不要为了形式堆叠用户必须理解的 YAML。

### 前端仓库

目标仓库：`/home/oslo/projects/hop-rs-frontend`。

- 新增多阶段生产 `Dockerfile`：Node 只负责构建，运行阶段只提供静态文件和反向代理，不保留 Node runtime。
- 新增静态服务器配置，仅代理 `/api/v1/` 到 `http://hop:8083/api/v1/`，其他未知前端路由 fallback 到 `index.html`；禁止通用 open proxy。
- Compose/same-origin 模式默认 API base 为当前 Origin，不再要求用户填写 `http://host:8083`。
- 连接层默认只要求网页管理 Token；API URL 放入折叠的“连接其他实例”高级入口。
- Bearer Token 由浏览器发给同源 `/api/v1`；静态服务器不保存、不注入、不记录 token。
- Token 可以在当前浏览器 session 中保留以避免每次刷新重新输入，但禁止写入 `localStorage`；实施时应补充 XSS/CSP 约束与测试。
- 删除或重构以 source/orphan/manifest validate/diff/apply/reload 为中心的 Configuration 页面；普通面板只呈现能帮助当前用户的设置和 ownership 提示。
- Assets、Credentials、Access、Sessions 页面继续使用真实 Control API；配置管理的资源在 UI 中明确只读。
- 前端 README、PRODUCT、设计规范和实施说明必须从“独立手填 URL 的面板”更新为“Compose 同源是默认、独立连接是高级能力”。
- 增加 production container smoke test，验证 `/`、SPA 深链、`/api/v1/status` 代理、401、Bearer 转发和 upstream 不可用状态。
- 为 `ghcr.io/oslo254804746/hop-rs-frontend` 增加与后端版本可配套的镜像发布流程；Compose 中避免不兼容的前后端 tag 组合。

## 本轮额外挖掘出的简化点

除用户明确提出的 Compose 和 `api.token` 外，本轮审计前端后确认还应纳入以下改动：

1. **默认路径不需要 CORS。** 同源反向代理比教用户填写 allowlist 更简单，也避免把 CORS 错当防火墙。
2. **默认不发布 8083。** 浏览器和宿主机只看到面板端口，Control API 留在 Compose 私有网络。
3. **面板不再要求 API URL。** 打开哪个面板 Origin，就默认连接该 Origin 下的 `/api/v1`；远程实例 URL 是高级选项。
4. **纠正 Origin 配置语义。** `192.168.11.1` 不是有效 Origin；直接跨域时必须包含 `http://` 或 `https://` 以及实际端口。
5. **Token 缺失不影响 SSH。** API 子服务配置错误不得让核心跳板能力整体退出。
6. **前端配置页需要收缩。** 单配置模型移除 source/watcher 后，现有 Sources、Orphans、Manifest Apply/Reload UI 不应继续占据主导航。
7. **前端需要提前知道 ownership。** config-managed 资源必须在显示编辑动作之前就标为只读，不能让非技术用户先填写整张表单再收到 409。
8. **Compose 采用 panel-first 空 Catalog。** 默认示例不预置一个随后无法在 UI 编辑的受管资产；用户在网页创建自己的资源。
9. **Token 不进入静态产物或代理配置。** 浏览器在 session 内持有 token 并发送 Authorization，避免为了“自动登录”把管理凭据交给所有能访问静态 JS 的人。
10. **前后端版本需要联动。** Compose 和发布工作流必须给出兼容 tag，而不是默认拉取两个独立变化的 `latest`。
11. **配置错误需要按边界处理。** API token 缺失只禁用可选 API 并保留 SSH；统一资源配置无效则在任何 listener 打开前整体失败，不能带着用户以为已生效的半套资源运行。
12. **README 应按用户能力分流。** “我想用网页”放在首位，“我想用单 YAML 管理”放在其次，CLI/API 细节放在参考文档。

## 文档重构要求

### 信息架构

只保留两条名称清楚、互不混写的学习路径，其中网页路径排在第一：

1. **网页管理（推荐）**：复制最小 `hop.yaml`、修改网页管理 Token、运行 `docker compose up -d`、打开 `http://hop-host:8080`，然后在网页添加入口公钥、目标凭据和资产。
2. **单配置管理（高级/无网页）**：在同一 `hop.yaml` 声明入口公钥、目标凭据和资产，启动二进制或单后端容器，再使用 `ssh -p 2222 menu@hop-host`。

主文档第一屏禁止出现：

- `inventory.sources`；
- `resources.yaml`；
- `HOP_BOOTSTRAP_FILE`；
- `source`、`watch`、`prune`、`revision`；
- credential UUID 查询；
- 一连串 `docker exec`；
- `password.file` / `password.env`；
- `token_file`；
- 要求用户填写后端 API URL；
- 默认 Compose 路径中的 CORS 配置；
- 同一个完整示例的 YAML/TOML 重复版本。

### 格式选择

- README 和快速部署只使用 YAML，作为唯一推荐格式。
- 若继续支持 TOML，只在配置参考中说明字段等价，不复制整份长示例。
- 高级 API、备份、ProxyJump、TCP 转发分别放到后续章节，不阻塞第一次连接。
- README 必须承认官方面板已经存在，删除“当前没有官方图形面板”的过时文字。
- Compose 示例紧邻最小 `hop.yaml`，并在启动命令后直接给出面板 URL 和第一次需要完成的三个网页动作。

### 需要审计的文件

- `README.md`
- `README-EN.md`
- `config.example.toml`
- `config.example.yaml`
- `config.docker.toml`
- `resources.example.yaml`
- `resources.docker.example.yaml`
- `docs/configuration.zh-CN.md`
- `docs/deployment.zh-CN.md`
- `docs/deployment.md`
- `docs/admin-guide.zh-CN.md`
- `docs/admin-guide.md`
- `docs/proxying.zh-CN.md`
- `docs/product/declarative-apply-spec.md`
- `CHANGELOG.md`
- `/home/oslo/projects/hop-rs-frontend/README.md`
- `/home/oslo/projects/hop-rs-frontend/PRODUCT.md`
- `/home/oslo/projects/hop-rs-frontend/docs/design-spec.md`
- `/home/oslo/projects/hop-rs-frontend/docs/implementation-plan.md`
- `/home/oslo/projects/hop-rs-frontend/src/stores/connection.ts`
- `/home/oslo/projects/hop-rs-frontend/src/layouts/AppShell.vue`
- `/home/oslo/projects/hop-rs-frontend/src/pages/ConfigurationPage.vue`

旧的产品设计文档可以保留历史背景，但所有“当前使用方式”必须与单配置模型一致。无用的示例文件应删除，不要同时保留新旧两套入口。

## 兼容性与迁移评估

这是一次公开配置契约简化，可能构成 breaking change。不要为了兼容旧写法而让新模型继续暴露两套概念。

推荐策略：

- 明确新版本不再接受 `inventory.sources`；
- 旧的分离 manifest 不再是启动输入；
- `api.token_file` 不再是启动输入，迁移时把文件内容复制到 `api.token`；
- 非 loopback API 不再强制要求 CORS allowlist；已有完整 Origin 列表仍可继续使用；
- 现有 SQLite 数据仍可打开；
- 主配置首次接管与本地或旧 source 同名的资源时，如果现有 ownership 会冲突，返回明确迁移错误，不静默抢占；
- 提供一段一次性迁移说明，但不要把迁移步骤放进新用户快速开始；
- 若自动 ownership 迁移会明显增加风险，宁可要求现有测试部署备份后使用新数据目录，也不要设计复杂的长期兼容层。

最终兼容决定需要写入 Changelog，并由测试固定。

## 当前工作区交接注意事项

编写本任务时，工作区已有未提交改动：

- CORS `cors_allowlist = ["*"]` panic 修复；
- SSH 入口只宣告 `publickey` 的修复；
- Docker `HOP_BOOTSTRAP_FILE` 启动 apply；
- `resources.docker.example.yaml` 及相关文档改动。

前端仓库当前位于 `dev` 分支，工作区干净，已有远端 `https://github.com/oslo254804746/hop-rs-frontend.git`，但尚无生产 Dockerfile 或镜像发布流程。

下一会话应：

- 保留并迁移独立的 CORS 修复和 SSH `publickey` 修复；
- 用单配置启动流程取代 `HOP_BOOTSTRAP_FILE`，不要在它上面继续扩展；
- 删除或重写 `resources.docker.example.yaml` 及 bootstrap 文档；
- 将 CORS 通配符修复保留为直接跨域的高级能力，同时删除非 loopback 强制 allowlist；
- 后端与前端是两个独立 Git 仓库，分别检查状态、分别提交，不要在一个仓库中执行覆盖另一个仓库的批量命令；
- 在修改前检查 dirty worktree，不要整仓 reset 或覆盖无关改动。

## 推荐实施顺序

1. **冻结新配置契约**：先用 serde 结构和解析测试固定单 YAML、直接 secret、`api.token`、路径解析、默认值与错误路径。
2. **重接启动流程**：复用 Apply engine 完成 config-managed 资源同步，删除 inventory watcher 和 Docker bootstrap；同时把 API token 错误收敛为 API 局部禁用。
3. **调整 API 契约**：为资源补充 ownership，删除面板不再需要的 source/reload 主路径，保留必要兼容时清楚标记高级接口。
4. **完成后端容器与 Compose**：先让 `hop` 在内部 `8083` 工作，再验证宿主机只暴露 `2222`。
5. **完成前端生产镜像与同源模式**：静态服务、SPA fallback、窄代理、默认当前 Origin、Token-only 首次连接。
6. **收缩前端产品模型**：移除旧 Configuration source 工作流，处理 config-managed 只读资源和默认 Token 警告。
7. **重写用户文档**：Compose 网页面板优先，单 YAML 次之，CLI/API 放参考章节；删除过时“没有官方面板”描述。
8. **跨仓验证**：Rust、Node、Docker、Compose、浏览器和 OpenSSH 全链路通过后，再分别提交两个仓库。

## 验收标准

### 用户体验

- 一个新用户只编辑一个 YAML 文件，运行一条启动命令，就能用已声明公钥进入 Hop。
- 最小密码 SSH 资产不需要额外 secret 文件、环境变量、资源 manifest、source 或数据库 ID。
- Docker 快速开始不包含启动后的 `docker exec` 配置步骤。
- `menu@`、资产名直连和目标凭据的区别在第一次使用示例中清楚可见。
- `docker compose up -d` 后访问一个面板 URL 即可管理空 Catalog，不需要输入 API URL 或配置 CORS。
- README 明确网页管理是推荐路径，同时保留单 YAML 的无面板路径。

### 行为

- 新数据目录首次启动会生成运行状态文件并原子创建配置资源。
- 相同配置重启幂等，Catalog revision 不变化。
- 修改密码、资产地址或入口公钥后重启会更新对应资源。
- 从配置删除资源只 prune 统一配置自己管理的资源。
- 配置中任一资源无效时不产生部分写入，也不打开监听。
- `public_key` 和 `public_key_file` 都有成功测试，二者同时存在或都不存在时有失败测试。
- `password: "value"` 可用；旧的 `password: { file: ... }` 和 `{ env: ... }` 按最终兼容决策被明确拒绝或只在迁移工具中读取。
- secret 不出现在日志、序列化响应、diff、错误或测试快照中。
- `api.token` 可直接使用，`token_file` 不再需要。
- API token 缺失或为空时 SSH listener 仍能启动，Control API 保持关闭并记录清晰警告。
- `token = "change-me"` 时 API 可启动且后端/面板均显示修改提示。
- Compose 中宿主机无法直接连接后端 `8083`，面板 `/api/v1` 同源代理可用且不需要 CORS response header。
- 直接跨域时完整 Origin 可匹配；裸 `192.168.11.1` 得到带字段路径的配置错误。
- panel/local 资源可以在网页编辑，config-managed 资源在操作前已显示为只读。

### 质量门槛

- `cargo fmt --all --check`
- `cargo test --workspace --locked`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `shellcheck docker-entrypoint.sh scripts/*.sh`
- 真实 Docker 空目录首次启动测试；随后重启验证幂等。
- OpenSSH E2E 使用配置中的入口公钥连接 TUI，并验证未匹配公钥只得到 `Permission denied (publickey)`，不出现密码提示。
- `docker compose config` 和从空目录启动成功。
- 前端执行 `npm run lint`、`npm run typecheck`、`npm run test:unit`、`npm run build` 和关键 Playwright E2E。
- 生产前端镜像验证静态首页、SPA fallback、同源 API 代理、Bearer 转发和 401 错误。

## 明确非目标

- 本任务不重新设计或重写图形面板；复用并调整 `/home/oslo/projects/hop-rs-frontend`，补齐生产容器、同源连接和已经变化的配置边界。
- 不实现多用户账号、RBAC、Cookie 或登录密码。
- 不实现配置 watcher 或复杂热更新。
- 不增加 Vault、KMS、Docker secrets、Kubernetes CRD 等企业 secret provider。
- 不重新设计 Catalog、SSH 代理或 Key-to-Asset 授权语义。
- 不因为简化配置而把目标密码误用为 Hop 入口密码；Hop 入口继续只接受 Access Key 公钥认证。

## 完成定义

代码、示例、Docker/Compose 入口、官方面板和当前用户文档必须讲同一套产品模型。网页用户执行 Compose 后不需要理解 API URL、CORS、source、manifest、数据库 ID 或 secret 文件；配置用户只需要一份 YAML。若任一路径仍要求用户先理解内部 Apply/Catalog 机制才能完成首次连接，则本任务尚未完成。
