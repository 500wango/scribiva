# Scribiva v1.0 — 增长指标、MRR 与成本模型

## 1. 商业目标与指标定义

**目标：$1,000 MRR**。仅计算活跃、可重复的订阅收入，年付按合同期摊入月，不把终身买断、一次性服务费、欠费或累计销售额当作 MRR。

- `MRR = Σ(活跃订阅的周期合同价值 / 对应月数)`（按统一会计口径处理折扣、税、退款和汇率）。
- `ARR = MRR × 12` 仅为年化速率，不代表已收年收入。
- `New MRR + Expansion MRR - Churned MRR - Contraction MRR = Net New MRR`。
- `Paid conversion = 新付费账户数 / 合格免费层账户数`，必须定义免费层队列（如注册后 30 天内完成 ≥1 次听写）与观察窗口。
- `Activation = 首次安装后七天内 ≥3 次服务端 completed 听写`（建议初始定义；插入结果作为辅助指标）。
- `North Star = 每周 ≥3 次服务端 completed 听写的账户数`（见 PRD §2）。

## 2. 定价模型：决策建议而非已定价

| 假设平均每有效订阅贡献 | 目标订阅账户数 | 说明 |
|---:|---:|---|
| $5 MRR | 200 | 低价、需要更多用户 |
| $8 MRR | 125 | 小团队基准示例 |
| $10 MRR | 100 | 对价值验证要求更高 |
| $69/年 | 174 | $69 / 12 = $5.75 MRR；174 × $5.75 ≈ $1,000.50 |

建议测试 `$8/月`、`$69/年`，但价格需结合推理成本、竞品、支付费和目标用户访谈批准。年付的**现金收入**集中在成交日，不可直接等同 MRR。

## 3. 漏斗模型（情景而非真实数据）

| 免费层→付费转化 | 获取 125 付费账户所需累计免费层账户 |
|---:|---:|
| 3% | 4,167 |
| 5% | 2,500 |
| 10% | 1,250 |
| 15% | 834 |

真实数据还要减去取消订阅、退款和重复注册。若无可持续新用户流入或留存不足，目标很难保持。

## 4. 语音成本模型

以 Groq 官方在 2026-10-26 文档列出的 `whisper-large-v3-turbo` `$0.04/音频小时` 为说明：

- 125 个付费用户 × 每人 10 小时/月 = **1,250 小时/月**。
- 理论 ASR 基础费用 = 1,250 × $0.04 = **$50/月**。
- 这是**理想小时计费粗估**：供应商文档有最短计费长度（例如 10 秒），很多短请求会导致实际费用不同；请求失败、重试、模型切换等也改变账单。
- 此费用**不包含**语法模型、托管服务、存储、监控、带宽、支付费、税、退款和开发者支持成本。
- **模型选择会改变成本**：当前客户端与服务端代码默认 `whisper-large-v3`（$0.111/小时），同样用量约 **$139/月**，是 turbo 的约 2.8 倍。上线前按 `04-ASR-GRAMMAR-I18N.md` 的建议把默认改为 turbo，或按此口径重算毛利。
- **免费层也产生成本**：每个免费层账户按月度额度上限计；例如 1,800 秒/月 × 2,000 个免费层账户 = 1,000 小时 ≈ $40/月（turbo），需与付费收入合并核算。

### 更准确的模型

```text
BilledSeconds = Σ over recordings max(recording_duration_seconds, provider_minimum_billed_seconds)
ASRCost = BilledSeconds / 3600 × provider_price_per_hour
GrammarCost = Σ(prompt_tokens/1e6×input_price + completion_tokens/1e6×output_price)
GrossMargin = (NetSubscriptionRevenue - ASRCost - GrammarCost - variable_hosting - payment_processing) / NetSubscriptionRevenue
```

若供应商计费机制与上述模型不一致，按实际账单修正。每月检查分布，而不是只看平均分钟数；重度用户可能显著影响单位经济模型。

## 5. 必须采集的分析事件（最小化）

**原则**：事件只包含元数据，不上传音频、转写文本、窗口标题、剪贴板内容或可识别的文件路径。客户端诊断默认 opt-in，服务端必要计费元数据除外。

| 事件名 | 属性（示例） | 目的 |
|---|---|---|
| `install_first_open` | os, app_version | 分平台新增 |
| `onboarding_step_completed` | step_id, os | 找安装障碍 |
| `recording_started` | language_mode, os | 漏斗 |
| `recording_cancelled` | reason, stage | 取消问题 |
| `dictation_completed` | duration_bucket, asr_ms, grammar_ms, language_code | 可靠性 |
| `insertion_dispatched` | backend, os, session, target_app_category | 系统输入稳定性 |
| `insertion_failed` | backend, error_code, os, session | 回退与兼容性 |
| `free_quota_exceeded` | plan | 付费入口 |
| `landing_page_download` | os, region_bucket | 官网下载转化（不含 IP 或精确位置） |
| `checkout_started` | plan | 收费漏斗 |
| `subscription_activated` | plan, billing_interval | 付费转化 |
| `subscription_cancelled` | reason_category | 流失分析 |

`target_app_category` 仅允许粗分类（browser/editor/chat/office/other），不包含应用窗口标题或文档路径。`device_id` 不作为可被第三方追踪的稳定标识符。

## 6. 每周增长复盘

- **产品**：每个平台首次成功率、7 日留存、每用户每周听写次数、输入失败 Top 5。
- **商业**：月活免费层用户、新订阅、取消数、净增 MRR、有效订阅、回款及折扣；按支付通道（Stripe/Paddle）与地区（中国大陆/海外）分组。
- **免费层成本**：免费层用户的语音分钟与 ASR/LLM 成本单独核算，作为 D-004 额度调整依据。
- **成本**：每有效付费账户语音分钟、ASR/LLM 成本、支持工单、支付费、毛利。
- **渠道**：Chrome 等浏览器生态不是主产品，因此获客应通过 SEO、开源社区、Linux 社区、Obsidian 场景内容、用户转介绍等验证。

## 7. 早期阶段建议停止/继续规则

- 当付费用户 <10 且高比例用户不能完成主流程时：**优先修复产品，不扩张获客**。
- 当 30 位定向用户中有明显重复使用、至少 5 位付费且少量支持成本可控时：继续进行小规模增长试验。
- 连续出现焦点误贴、数据丢失、系统权限安全风险时：**立即暂停发布/推广**，先修复。
- 若 20 语言质量差异大，按真实成绩披露 Beta，不能用“模型支持 99+ 种”替代产品验证。

所有门槛均是内部行动规则，**不代表统计学意义上的成功证明**。
