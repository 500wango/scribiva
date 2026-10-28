# Scribiva landing page

纯静态 HTML，无构建步骤、无外部字体/CDN/统计脚本。任意 HTTPS 静态托管直接上传本目录即可（全部为相对链接）。

```
index.html        简体中文首页（默认）
en/index.html     英文首页
privacy.html      隐私政策（中文 + #en 英文，草案）
terms.html        服务条款（中文 + #en 英文，草案）
refund.html       退款与取消政策（中文 + #en 英文，草案）
assets/config.js  所有可替换值的唯一来源
assets/style.css  样式（含深色模式）
assets/main.js    读取 config.js、下载按钮状态、首屏演示
assets/logo.svg   图标
```

本地预览：`python3 -m http.server 8080 -d landing`

## 上线前必须替换的占位值

以下都在 `assets/config.js` 修改，页面会自动套用：

| 键 | 当前占位 | 说明 |
|---|---|---|
| `contact_email` | `support@[域名]` | 联系邮箱 |
| `legal_entity` | `[经营主体名称 / Legal entity name]` | 条款、隐私政策中的经营主体 |
| `legal_address` | `[经营主体地址 / Registered address]` | 注册地址 |
| `legal_effective_date` | `[生效日期 / Effective date]` | 法律文本生效日期 |
| `free_quota_*` | 30 分钟/月 | **待定价确认**（服务端默认 1,800 秒） |
| `price_monthly_*` / `price_yearly_*` | $8/月、$69/年 | **待定价确认**，须与 Stripe 价格一致 |
| `pro_quota_*` | `[Pro 月度额度待定]` | Pro 月度额度 |
| `version` | `[版本号 / version]` | 发布版本号 |
| `release_state` | `coming_soon` | 改为 `available` 后按钮变为下载链接 |
| `windows_url` / `linux_deb_url` | 空 | 安装包地址（建议同域名，保证大陆可访问） |
| `windows_sha256` / `linux_deb_sha256` | `[SHA-256 待发布 / pending]` | 安装包校验值 |

HTML 中与上述同名的文字只是脚本未运行时的后备显示，改完 config.js 后建议在各页面同步替换一遍（`grep -rn "\[域名\]\|\$8\|\$69\|30 分钟\|30 min" landing`）。

以下占位只写在页面里，需直接编辑 HTML：

- `privacy.html`：Groq 留存政策链接、Stripe 隐私政策链接、服务器托管商、邮件发送服务、是否用于训练/出售个人信息、中英文版本冲突时以哪个为准、按 PIPL/GDPR/CCPA 补充的权利条款（HTML 注释标出）。
- `terms.html`：适用法律与争议解决地；责任限制条款（HTML 注释标出）。
- `refund.html`：退款申请期限（N 天）、回复时限（N 个工作日）、不退款情形、按比例退款与年付处理。
- 首页兼容性表格：所有环境当前标为 “Beta / 测试中”，QA 通过后按 `docs/08-QA-ACCEPTANCE.md` 结论更新。
- 下载区的校验命令中示例文件名 `Scribiva-Setup.exe`、`scribiva_*.deb`，按实际文件名修改。

法律页面顶部的“草案，待法务审核”横幅在法务审核通过后删除。
