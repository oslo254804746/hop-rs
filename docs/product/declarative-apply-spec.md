# 单配置与内部原子 Apply 契约

## 用户模型

Hop 的公开启动模型是一份 `hop.yaml`。它同时描述运行参数、目标凭据、资产与入口公钥。启动不接受资源文件列表，不监视文件变化，也不依赖第二份资源清单。

配置必须在任何监听器开启前完整解析和校验。未知字段、重复资源名称、错误引用、无效 secret 组合、无效端口或公钥均使启动失败。

## 内部 Apply

统一配置会转换为内部 Manifest，并复用 Catalog 的事务 Apply 引擎。内部管理键固定，不属于公开配置或 API 响应。

事务保证：

- 解析、离线验证和数据库引用验证全部先于写入；
- 一个事务提交所有资源；
- 任意错误回滚全部变化；
- 相同配置重复启动幂等；
- 配置中删除的 `config` 资源会被清理；
- `local` 资源永不被启动清理影响。

高级 CLI/API validate、diff、apply 能力可作为兼容和调试接口继续复用同一引擎，但不构成普通部署路径，也不提供重新加载启动配置的接口。

## Secret

统一配置中的 `password`、`private_key`、`passphrase` 是直接字符串。入口公钥支持 `public_key` 或 `public_key_file`。

- secret 类型的 `Debug` 始终脱敏；
- 错误、日志、API 响应不得包含原值；
- 目标 secret 写入时加密，读取时只返回 configured/missing；
- 含 secret 的配置文件必须为 `0600`，且不得提交到版本控制。

## Ownership API

资产、目标凭据和入口公钥列表/写入响应仅公开：

```text
ownership = local | config
```

不公开内部管理键。对 `config` 资源发起本地变更时，后端仍以冲突拒绝，作为 UI 之外的最终防线。

## Control API Token

`api.token` 是直接字符串。缺失或空白只禁用 API；SSH 正常运行。`change-me` 允许首次体验但产生安全警告。Token 不得进入 Debug、日志、响应或前端产物。

## CORS

同源 Compose 不需要 allowlist。直接跨域访问时，每项必须是完整 HTTP(S) Origin，不能是裸主机、裸 IP、含路径或含凭据的 URL。`*` 被支持但必须单独出现。
