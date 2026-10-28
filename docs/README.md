# Scribiva v1.0 — 开发文档索引

> 版本：v1.0-draft.2 ｜ 日期：2026-10-28 ｜ 状态：**开发基线草案（未完成技术验证）** ｜ 默认语言：简体中文

> 当前可运行实现为 `0.2.0-alpha.1`，见仓库根 [README](../README.md) 与 [server/README.md](../server/README.md)。本目录仍是未来 v1.0 的产品/工程规范，不代表下列平台矩阵与商业能力均已交付。

Scribiva 是一款面向 **Windows + Linux** 的系统级、跨应用 AI 语音输入工具：用户通过全局快捷键录音，在任意**已验证支持**的编辑场景完成多语言语音识别、自动语法修正并向原光标位置插入文本。特别保证 **Obsidian 的 Live Preview 与 Source Mode** 在支持矩阵中单独验收。

## 阅读顺序

| 文件 | 面向谁 | 重点 |
|---|---|---|
| [01-PRD.md](01-PRD.md) | 产品、开发、测试 | 目标用户、流程、需求与明确的验收条件 |
| [02-ARCHITECTURE.md](02-ARCHITECTURE.md) | 技术负责人 | 分层架构、边界、状态机、时序与技术选型 |
| [03-PLATFORM-ADAPTERS.md](03-PLATFORM-ADAPTERS.md) | Rust / 系统工程师 | Windows、X11、Wayland 的快捷键和文本注入策略 |
| [04-ASR-GRAMMAR-I18N.md](04-ASR-GRAMMAR-I18N.md) | AI / 后端 | 20 语言、自动识别、语法修正、保护规则与回退 |
| [05-API-AND-DATA.md](05-API-AND-DATA.md) | 前后端 | API 契约、数据结构、幂等、额度与订阅 |
| [06-UX-SPEC.md](06-UX-SPEC.md) | UI / 前端 | 状态栏、首次授权、操作反馈、设置与错误态 |
| [07-SECURITY-PRIVACY.md](07-SECURITY-PRIVACY.md) | 安全 / 全员 | 权限、密钥、音频处理、日志和威胁模型 |
| [08-QA-ACCEPTANCE.md](08-QA-ACCEPTANCE.md) | QA / 开发 | 平台矩阵、20 语言、Obsidian、性能、发布门槛 |
| [09-DELIVERY-PLAN.md](09-DELIVERY-PLAN.md) | 执行者 | 技术风险前置、Sprint、任务依赖与退出条件 |
| [10-RELEASE-OPERATIONS.md](10-RELEASE-OPERATIONS.md) | DevOps | CI、安装签名、自动更新、运行手册 |
| [11-METRICS-ECONOMICS.md](11-METRICS-ECONOMICS.md) | 产品 / 运营 | $1,000 MRR 模型、漏斗、成本与埋点 |
| [12-ADR-AND-RISKS.md](12-ADR-AND-RISKS.md) | 技术负责人 | 架构决策记录（ADR-001～012）、风险台账和待定事项 |
| [13-DEVELOPER-SETUP.md](13-DEVELOPER-SETUP.md) | 开发 | 仓库建议、环境变量、开发启动、质量检查 |
| [14-SOURCES.md](14-SOURCES.md) | 全员 | 官方资料、参考项目及核验日期 |

**alpha 实现记录**（英文，描述已交付的 `0.2.0-alpha.1`，不是 v1.0 规范）：[ARCHITECTURE.md](ARCHITECTURE.md) · [SECURITY.md](SECURITY.md) · [ROADMAP.md](ROADMAP.md) · [TEST_REPORT.md](TEST_REPORT.md) · [SOURCES.md](SOURCES.md)

## 已确认的产品约束

1. 首发平台：**Windows + Linux 同时首发**；开发与验证 **Linux 先行**，Windows 不推迟到 v1.1。macOS 不在 v1.0 范围内。
2. 核心体验：**全局快捷键、录音、转写、自动语法修正、插入当前应用**，无需跳到单独网页。
3. Linux 参考项目：**Voxtype**，只参考/按许可证复用适合的子模块，不宣称 Scribiva 与 Voxtype 有官方关系。
4. 重点应用：**Obsidian**，验证其 Live Preview、Source Mode、中文和 Markdown。
5. 语言：**至少 20 种可选择并通过验收的语音语言**，含自动检测和手动指定。
6. 商业目标：**$1,000 MRR**；商业模式为**每月免费层 + 订阅**；支付以 Stripe 为主，Paddle 为第二选项评估。
7. 平台和语言只有进入 **已通过 QA 矩阵** 的组合，才能对外宣称正式支持；其他只能标明 Beta 或不支持。
8. 目标市场包含**中国大陆**与海外；大陆网络可用性、支付与合规是首发前置风险。
9. 首发必须有 **landing page**（产品、价格、下载、兼容列表、隐私/条款/退款政策、联系方式）。

## 特别重要：不是既成事实的内容

- **Wayland 支持是风险项**：Portal 可用性、系统授权、合成器和输入方式因环境而异。优先完成平台 PoC 后再承诺发布日期。
- **20 种语言是验收目标**，模型提供商宣传“99+ 语言”并非对质量的证明。
- **云服务及价格可变**：模型、支付与基础设施均做成可替换适配器；本包示例价格只用于成本测算，不构成报价。
- **已创建开发实现，但远未完成商业验证**：仓库包含 Linux GNOME Wayland 客户端与 stdlib/SQLite 服务端骨架；Windows、X11、KDE、生产支付、部署与 20 语言验收仍未交付。

## 定义

- **P0**：v1.0 正式发布必须满足；**P1**：后续迭代；**PoC**：为验证技术路线而写的最小原型。
- **注入成功（verified）**：应用能够观察到可靠的提交成功证据。多数桌面系统无法从外部真正确认目标编辑器内容已更改；此时只可返回 `dispatched_unverified`，不得假称成功。
- **受支持环境**：版本、操作系统、桌面会话、权限与目标应用组合均通过兼容性测试。
- **公开发布日期**：未确定，遵循验收标准而不是固定工期。

## 开工前的 Gate 0

**必须先成功：** 在 Windows、Linux X11、GNOME Wayland、KDE Wayland 各一组受控环境中，将 `Hello 中文 العربية\n- **Markdown**` 插入 Obsidian 两种编辑模式，保留换行、文字顺序与撤销能力；记录失败配置。做不到时缩小支持声明，而不是悄悄移除 Windows/Linux 或 Obsidian 需求。

顺序上 Linux 先行，但 Windows PoC 与 Linux 并行启动（ADR-008）；GNOME/KDE Wayland 还需同时验证焦点信息来源（GNOME Shell 扩展 / KWin 脚本，ADR-012）。

## 版本变更流程

任何改变 P0、用户数据处理、平台支持、订阅规则的决定都必须修改对应规范，新增 ADR，并在 README 更新版本。本文档中的“建议”“假设”“待验证”不能作为已经实现的功能发布。
