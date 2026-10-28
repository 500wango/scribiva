# 美国生产服务器部署（ADR-010、ADR-011）

单台 Linux 服务器（建议 Ubuntu 24.04 LTS，x86_64，美国区域），Caddy 终止 TLS，API 只监听回环地址。

## 1. 准备

```bash
sudo apt install -y python3 python3-venv ffmpeg caddy
sudo useradd --system --home /var/lib/scribiva --shell /usr/sbin/nologin scribiva
sudo install -d -o scribiva -g scribiva -m 0700 /var/lib/scribiva /var/backups/scribiva
sudo install -d -m 0750 -g scribiva /etc/scribiva
sudo git clone https://github.com/500wango/scribiva.git /opt/scribiva
sudo python3 -m venv /opt/scribiva/venv
sudo /opt/scribiva/venv/bin/pip install -r /opt/scribiva/server/requirements.txt
```

`requirements.txt` 只有 OpenCC（简繁转换，FR-003），版本已固定。没有它服务也能运行，只是 `output_script` 会返回 `SCRIPT_CONVERSION_UNAVAILABLE` 警告。

## 2. 配置

```bash
sudo cp /opt/scribiva/server/deploy/scribiva.env.example /etc/scribiva/scribiva.env
sudo chown root:scribiva /etc/scribiva/scribiva.env && sudo chmod 0640 /etc/scribiva/scribiva.env
sudoedit /etc/scribiva/scribiva.env       # 填 GROQ_API_KEY、STRIPE_*、SMTP_*、域名
```

- `SCRIBIVA_PUBLIC_BASE_URL` 必须是 API 的 HTTPS 地址；登录页 `/login` 和 Stripe 回跳都用它。
- Stripe Webhook 端点：`https://api.<域名>/v1/webhooks/stripe`，订阅事件：`customer.subscription.created`、`customer.subscription.updated`、`customer.subscription.deleted`、`checkout.session.completed`。
- 邮件：任意支持 SMTP 的发信服务。发件域需配置 SPF/DKIM，否则魔法链接容易进垃圾箱。

## 3. 上线前自检

```bash
sudo -u scribiva env $(sudo cat /etc/scribiva/scribiva.env | xargs) \
  PYTHONPATH=/opt/scribiva/server /opt/scribiva/venv/bin/python -m scribiva_server check-provider
```

必须在**这台美国服务器上**通过（R17）。返回 403 或 Cloudflare 1010 时联系 Groq 支持，不要尝试绕过。

## 4. 启动

```bash
sudo cp /opt/scribiva/server/deploy/scribiva-api.service /opt/scribiva/server/deploy/scribiva-maintenance.{service,timer} /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now scribiva-api scribiva-maintenance.timer
sudo cp /opt/scribiva/server/deploy/Caddyfile /etc/caddy/Caddyfile   # 先把 example.com 换成你的域名
sudo systemctl reload caddy
curl -fsS https://api.<域名>/healthz
```

`scribiva-maintenance.timer` 每 6 小时清理过期转写文本和登录记录，并写一份经完整性校验的备份到 `/var/backups/scribiva/`，保留 14 天。请另外把该目录同步到异地存储。

## 5. 恢复

见 `docs/10-RELEASE-OPERATIONS.md` §6「服务器故障或数据库损坏」。简要步骤：停服务 → 把最近的备份复制为 `/var/lib/scribiva/scribiva.sqlite3`（属主 scribiva，0600）→ 启动 → 在 Stripe 后台重放恢复点之后的订阅事件。

## 6. 中国大陆可达性

部署后从大陆网络（无代理）分别访问 landing page、`/healthz`、`/login`，并用客户端完成一次听写（QA-CN-001）。被干扰时准备一个备用域名，客户端可通过 `scribiva service login --url <新域名>` 切换。
