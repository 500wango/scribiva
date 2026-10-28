# Scribiva API — Groq 代理与额度骨架

**状态：`0.2.0-alpha.1` · 可运行骨架，不是生产服务。**

服务端只使用 Python 3.12 标准库与 SQLite，目的不是替代最终后端选型，而是先把**密钥不落客户端、账户鉴权、额度预留、幂等、失败不计费、Stripe Webhook 验签**这些不可延后的边界做成可测试的实现。v1.0 沿用本实现，单实例部署在美国服务器，在反向代理终止 TLS 并做定时备份（ADR-011）；只有需要多实例时才迁移 PostgreSQL 和共享限流。

## 快速启动

```bash
export GROQ_API_KEY='gsk_...'
export SCRIBIVA_DB="$PWD/server/data/scribiva.sqlite3"
export PYTHONPATH=server

python3 -m scribiva_server init-db
python3 -m scribiva_server create-user --email you@example.com   # 只显示一次 token
python3 -m scribiva_server serve --host 127.0.0.1 --port 8787
```

`serve` 默认只允许回环地址。若确实需要监听其它地址，必须显式设置 `SCRIBIVA_ALLOW_REMOTE=1`，并且应在 TLS 反向代理之后运行；服务本身不终止 TLS。

客户端登录（token 通过隐藏输入读取，保存为本机 `0600` 文件）：

```bash
scribiva service login --url http://127.0.0.1:8787
scribiva service status
scribiva service billing --plan monthly   # 输出 Stripe Checkout URL
scribiva service portal                   # 输出 Stripe Billing Portal URL
scribiva service delete --confirm         # 删除服务端任务/转写/token，并清理本机凭据
scribiva service logout
```

配置 `service_url` 后，客户端不再需要本机 Groq Key；录音、语言和模式仍由客户端选择，识别与语法处理在服务端完成。`service billing` / `service portal` 只打印 Stripe 托管页面 URL，不在客户端处理银行卡或订阅状态；权益变更只以服务端验签后的 Stripe Webhook 为准。本机仪表盘（`scribiva ui`）也提供额度显示和同样的 Checkout/Portal 入口。

## API

| 方法 | 路径 | 鉴权 | 说明 |
|---|---|---|---|
| GET | `/healthz` | 无 | 健康检查；不返回密钥 |
| GET | `/v1/config/public` | 无 | 公开配置、模型名、限额与隐私说明 |
| GET | `/v1/me` | Bearer | 账户、套餐、订阅状态、当月用量 |
| GET | `/v1/me/usage` | Bearer | 当月已用、预留与剩余秒数 |
| GET | `/v1/me/dictations` | Bearer | 最近任务元数据（不含转写文本） |
| POST | `/v1/dictations` | Bearer | `multipart/form-data` 上传 WAV；同步返回结果 |
| GET | `/v1/dictations/{id}` | Bearer | 查询任务；结果过期返回 410 |
| POST | `/v1/dictations/{id}/cancel` | Bearer | 取消处理中的任务 |
| POST | `/v1/billing/checkout-session` | Bearer | 创建 Stripe Checkout（`{"plan":"monthly"}`） |
| POST | `/v1/billing/portal-session` | Bearer | 创建 Stripe Billing Portal |
| POST | `/v1/webhooks/stripe` | 签名 | Stripe Webhook；只接受验签事件 |
| DELETE | `/v1/me/data` | Bearer | 删除转写、任务与 token；需 `{"confirm":"DELETE"}` |

上传字段：`audio`（文件）、`language`、`grammar_mode`（`raw/standard/polished`）、`client_job_id`、`client_version`。`Idempotency-Key` 请求头必填，且必须与 `client_job_id` 一致。示例：

```bash
curl -sS http://127.0.0.1:8787/v1/dictations \
  -H "Authorization: Bearer $SCRIBIVA_TOKEN" \
  -H "Idempotency-Key: job-2026-10-26-001" \
  -F audio=@sample.wav \
  -F language=zh \
  -F grammar_mode=standard \
  -F client_job_id=job-2026-10-26-001 \
  -F client_version=0.2.0-alpha.1
```

成功响应包含 `raw_text`、`output_text`、`billing.charged_audio_seconds`、`billing.remaining_seconds`、`warnings`。错误统一为：

```json
{"error": {"code": "QUOTA_EXCEEDED", "message": "...", "retryable": false, "request_id": "req_..."}}
```

## 额度与幂等语义

- 免费套餐默认 `1800` 秒/UTC 自然月；Pro 默认 `36000` 秒/UTC 自然月，可用环境变量调整。
- 音频时长由服务端从 WAV 头或 `ffprobe` 计算，**不信任客户端声明**；按秒向上取整，每段至少 1 秒。
- 请求在调用 Groq 前先以事务创建 `processing` 任务并预留额度；只有成功写入结果时才记录 `usage_ledger` 扣费。识别失败、取消、超时都不会扣费。
- `(user_id, client_job_id)` 唯一。相同 key 重放返回已完成结果或处理中状态，不会重复计费；失败任务用相同 key 重试返回 `409 IDEMPOTENCY_CONFLICT`，需换新 key。
- 同一用户同时只允许一个处理中任务；超过 `JOB_TIMEOUT` 的陈旧任务会在下一次预留时标记为失败。
- 结果默认保留 `900` 秒；过期后读取返回 `410 RESULT_EXPIRED`，文本字段被清空，但账单秒数保留。
- 限流为进程内滑动窗口（默认 `30` 次/分钟/用户）。多实例部署必须改为共享限流。

## Stripe 骨架

```bash
export STRIPE_SECRET_KEY='sk_...'
export STRIPE_WEBHOOK_SECRET='whsec_...'
export STRIPE_PRICE_ID_MONTHLY='price_...'
export STRIPE_PRICE_ID_ANNUAL='price_...'
export SCRIBIVA_PUBLIC_BASE_URL='https://api.example.com'
```

- Webhook 使用 `Stripe-Signature` 的 `t=...,v1=...` HMAC-SHA256 验签，默认容忍 300 秒时钟偏差；未配置 secret 时返回 503。
- `checkout.session.completed` 只把订阅标记为 `incomplete`；只有验签后的 `customer.subscription.created/updated/deleted` 才能改变 entitlement。
- 事件 ID 去重写入 `processed_webhook_events`。`invoice.paid` 不单独授予权益，避免把付款邮件当授权来源。
- Checkout/Portal 通过 Stripe REST API 创建；服务端不保存卡数据，也不信任跳转 URL 作为授权依据。
- 开发环境可用本地命令模拟权益，不走真实支付：

```bash
python3 -m scribiva_server grant-pro --user you@example.com --days 30
```

## 配置

| 环境变量 | 默认值 | 说明 |
|---|---|---|
| `SCRIBIVA_DB` | `server/data/scribiva.sqlite3` | SQLite 路径 |
| `SCRIBIVA_HOST` | `127.0.0.1` | 监听地址 |
| `SCRIBIVA_PORT` | `8787` | 监听端口 |
| `GROQ_API_KEY` | 空 | 必填；服务端持有，不返回客户端 |
| `SCRIBIVA_FREE_SECONDS` | `1800` | 免费月额度 |
| `SCRIBIVA_PRO_SECONDS` | `36000` | Pro 月额度 |
| `SCRIBIVA_MAX_AUDIO_BYTES` | `25000000` | 上传字节上限 |
| `SCRIBIVA_MAX_AUDIO_SECONDS` | `180` | 单段音频秒数上限 |
| `SCRIBIVA_RESULT_TTL_SECONDS` | `900` | 结果文本保留时间 |
| `SCRIBIVA_JOB_TIMEOUT_SECONDS` | `120` | 处理中超时回收 |
| `SCRIBIVA_RATE_LIMIT` | `30` | 每分钟/用户请求数 |
| `SCRIBIVA_TOKEN_TTL_DAYS` | `30` | token 有效期 |
| `SCRIBIVA_PUBLIC_BASE_URL` | 空 | Stripe 成功/取消/Portal 返回地址 |
| `SCRIBIVA_ALLOW_REMOTE` | `false` | 允许非回环绑定 |
| `GROQ_BASE_URL` | `https://api.groq.com/openai/v1` | 仅测试/兼容用途 |

## 隐私与删除

- 音频仅在内存中处理，不写入数据库或磁盘；WAV 由标准库校验，FLAC/WebM 需要本机 `ffprobe`，缺失时拒绝而不是猜测时长。
- 转写文本只在结果 TTL 内保存；`DELETE /v1/me/data` 删除任务、文本和全部 token，并标记 `deletion_requested_at`。用量账本与订阅状态作为最小计费元数据保留。
- 日志只记录请求 ID、状态码、用户 ID、耗时和错误码，不记录 Authorization、音频、转写或 Stripe 密钥。
- token 只存 SHA-256 哈希；数据库与配置目录以 `0600/0700` 权限创建。

## 已知非生产边界

- 单进程 SQLite：适合单实例开发和早期部署，不适合多副本、高并发或跨区域。
- 没有邮箱验证、密码找回、OAuth、管理员 Web 控制台、审计导出或密钥轮换流程。
- 限流与任务状态在进程内/单库中；崩溃恢复依赖超时回收，不依赖队列。
- Stripe 流程没有处理所有事件类型（如退款、争议、税务），也未提供前端支付页面。
- 未实现对象存储、音频留存开关、内容审核、滥用检测或多租户隔离。
- 服务端不终止 TLS；生产必须在反向代理上启用 HTTPS、请求体上限和超时。

## 测试

服务端测试完全离线：使用临时 SQLite、模拟 Provider、临时端口和签名自造的 Stripe 事件，不访问 Groq 或 Stripe。

```bash
PYTHONPATH=src:server python3 -m unittest discover -s tests -p 'test_*.py' -v
```

覆盖：Bearer 鉴权与过期/撤销、额度预留与并发、幂等重放、取消竞态、结果过期、限流、音频校验、multipart 限制、文本保护、Provider 失败不计费、Stripe 验签与事件幂等、隐私删除、客户端到真实本地服务的 curl 集成。
