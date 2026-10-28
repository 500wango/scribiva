# Scribiva v1.0 — 技术来源与核验记录

**核验日期**：2026-10-26。链接是独立官方资料和开源项目，不代表这些项目认可/背书 Scribiva。供应商价格、接口、兼容性和许可证可能变动；上线前须再次核查。

## A. 官方 API 与框架

| 资料 | 链接 | 支持的结论 |
|---|---|---|
| Tauri 2 Global Shortcut | https://v2.tauri.app/plugin/global-shortcut/ | 官方提供桌面全局快捷键插件，但需验证具体 Wayland 环境 |
| XDG GlobalShortcuts Portal | https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.GlobalShortcuts.html | Portal 的会话和全局快捷键接口 |
| XDG RemoteDesktop Portal | https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.RemoteDesktop.html | 输入可通过 EIS/libei 的 ConnectToEIS 或 D-Bus 通道（需授权） |
| Groq Speech to Text | https://console.groq.com/docs/speech-to-text | 多语言 Whisper 模型、接口、文件限制、最低计费时长和文档价格 |
| Groq API reference | https://console.groq.com/docs/api-reference | `/audio/transcriptions` 参数以及 ISO-639-1 语言指定 |
| Obsidian Editing Views | https://obsidian.md/help/edit-and-read | Live Preview 与 Source Mode 的区别 |
| Stripe Billing Docs | https://docs.stripe.com/billing | 订阅管理与服务端权益同步参考 |
| Stripe Webhooks | https://docs.stripe.com/webhooks | 签名验证、重试及 webhook 事件处理 |
| Stripe WeChat Pay | https://docs.stripe.com/payments/wechat-pay | Checkout 订阅模式不支持微信支付；循环扣款为私有预览（2026-10-28 查阅） |
| Stripe Alipay recurring | https://support.stripe.com/questions/enabling-recurring-charges-for-alipay | 支付宝循环扣款需申请并经账户审核 |
| Paddle Alipay | https://developer.paddle.com/concepts/payment-methods/alipay | 支付宝订阅仅限人民币定价、中国账单地址，需审批，续费上限 1,600 CNY |
| Paddle WeChat Pay | https://developer.paddle.com/concepts/payment-methods/wechat-pay | 微信支付仅一次性购买，不支持订阅 |
| Paddle domain review | https://www.paddle.com/help/start/account-verification/what-is-domain-verification | 网站审核要求：产品说明、价格、条款/退款/隐私政策、HTTPS |
| Windows SendInput API | https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput | Win32 模拟输入和 UIPI 权限约束 |

## B. 开源 Linux 参考

| 资料 | 链接 | 使用方式 |
|---|---|---|
| Voxtype 代码库 | https://github.com/peteonrails/voxtype | Linux 语音输入架构参考（MIT） |
| Voxtype 安装文档 | https://github.com/peteonrails/voxtype/blob/dev/docs/INSTALL.md | Linux 依赖、权限、系统差异参考 |
| Voxtype 用户手册 | https://github.com/peteonrails/voxtype/blob/dev/docs/USER_MANUAL.md | X11/Wayland 多后端回退与组合器兼容 |
| Voxtype 配置文档 | https://github.com/peteonrails/voxtype/blob/dev/docs/CONFIGURATION.md | `wtype/eitype/dotool/ydotool/clipboard/xclip` 与驱动顺序 |
| Voxtype LICENSE（在 repo 内） | https://github.com/peteonrails/voxtype/blob/dev/LICENSE | 当前参考许可证；复制具体代码前再逐文件确认 |

## C. 技术结论与置信度

- **[KNOWN]** Tauri 有 Global Shortcut 插件；**[INFERRED]** 它在所有 Linux Wayland 桌面都直接可用是错误推论。
- **[KNOWN]** XDG Portal 规范定义了全局快捷键和远程桌面输入机制；**[INFERRED]** 每个用户系统都完整实现这些规范需要实测。
- **[KNOWN]** Voxtype 公布了多种 Linux 输出驱动和回退次序；**[INFERRED]** Scribiva 应参考机制，但不能保证简单复制就能正常运行。
- **[KNOWN]** Groq Whisper 多语言模型支持大量语种；**[INFERRED]** Scribiva 的“20 种正式支持”必须由测试而不是供应商列表决定。
- **[KNOWN]** Obsidian 有两种编辑模式；**[INFERRED]** 应分别做文本插入和撤销测试。

## D. 仍需获取的第一手证据

1. 每个平台的实际 PoC build、测试录屏、权限/驱动状态、失败日志（去敏）。
2. 20 种语言的有授权语料与盲测质量统计。
3. 所选 LLM 的价格、合同、安全/数据处理条款和实际输出质量。
4. Stripe 生产账户、地区、税费和退款规则的确定配置。
5. Windows/Linux 更新制品的签名和安全测试结果。

**使用约束**：以上来源为技术设计依据，不构成 Scribiva 已实现、已验证或已发布功能的证明。
