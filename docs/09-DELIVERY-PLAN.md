# Scribiva v1.0 — 开发实施计划与任务分解

**计划方法**：以技术 Gate 为优先级，不承诺特定发布日期；人力和工期需在团队实际能力确认后估算。

## 1. 基础路线

```mermaid
flowchart LR
  G0[Gate 0: 注入 PoC] --> G1[Core Dictation]
  G1 --> G2[20 Languages + Grammar]
  G2 --> G3[Billing + Security]
  G3 --> G4[QA Beta]
  G4 --> G5[Windows / Linux Release]
```

Windows 与 Linux **同时首发**（ADR-008）。工程上 **Linux 先行**：每个 Gate 先在 Linux 通过，Windows 按同一标准完成后 Gate 才关闭；Windows PoC 须与 Linux 并行启动，不能等 Linux 完成后再开始。

## 2. Gate 0：风险原型（必须最先）

**范围**：不接 ASR、不做支付、不做完整 UI，只测试跨应用文本注入。

| 任务 | 交付物 | DoD |
|---|---|---|
| G0-01 | Windows Unicode 注入 PoC | Obsidian 两种模式/Chrome/VS Code 正确插入、撤销 |
| G0-02 | Linux X11 PoC | Obsidian 两模式写入多脚本文本 |
| G0-03 | GNOME Wayland PoC | Portal/eitype 或明确的合规后备路径通过；失败有诊断 |
| G0-04 | KDE Wayland PoC | 同上，记录 portal 授权与后端 |
| G0-05 | Focus Guard PoC | 目标窗口变化时停止自动写入；GNOME Shell 扩展经 D-Bus 返回焦点窗口 ID 的原型 |
| G0-06 | Injection Idempotency PoC | 后端不确定时不自动二次注入 |
| G0-07 | Obsidian Undo PoC | 一次撤销结果清晰且历史笔记无改变 |

**Gate 0 输出**：可运行最小 PoC、兼容记录、输入策略与 ADR。失败时先缩小 Linux 支持环境或重构注入方案，不进入大规模开发。

## 3. 建议 Sprint 分解（顺序，不是固定周数）

### Sprint A — 客户端基础
- A-01 初始化 Tauri 2/React/Rust workspace 和代码质量工具。
- A-02 Tray、HUD、设置页、OS 平台检测、诊断页。
- A-03 麦克风选择、权限、录音缓冲、短音频编码。
- A-04 Rust 状态机/取消 token/错误码。
- A-05 将 Gate 0 输入适配封装为 `TextInjector` trait。

### Sprint B — AI 管线
- B-01 认证与用户身份。
- B-02 服务端用量预留和音频上传限制。
- B-03 ASR 适配器 + 20 语言枚举。
- B-04 raw/standard/polished 语法模块、保护器与失败回退。
- B-05 客户端短期恢复结果与不重复插入。
- B-06 逐语言自动/手动识别测试框架。

### Sprint C — 订阅与安全
- C-01 Stripe Checkout / Billing Portal / Webhook。
- C-02 服务端额度 ledger、状态重放和幂等。
- C-03 配置/机密安全存储、隐私告知、删除入口。
- C-04 错误码日志与可选匿名诊断。
- C-05 编写用户帮助文档和 Linux 授权引导。
- C-06 Landing page：产品、价格、下载、兼容列表、隐私/条款/退款政策、联系方式，中英双语，大陆与海外可访问（FR-014）；须在真实收费前上线以通过 Stripe 网站审核。
- C-07 中国大陆可达性：大陆网络到美国服务端的实测、音频压缩上传、备用域名、支付路径实测（FR-015）。
- C-09 美国生产部署：服务端与 landing page 上线、TLS、在生产服务器上验证 Groq 调用、接入现有 Stripe 账户的生产与测试密钥。
- C-08 Paddle 评估：按 PRD §11.3 输出 ADR（D-010）。

### Sprint D — 验收与发布
- D-01 应用 × 平台矩阵自动化/人工执行。
- D-02 Obsidian Markdown/Unicode/Undo 专项。
- D-03 20 语言标注样本与质量回归。
- D-04 安装包、签名、更新、回滚。
- D-05 Beta 真实用户试验、关键问题修复、发布说明。

## 4. 首批工程 Issue（可复制至 GitHub）

| Issue | 优先级 | 依赖 | 简明验收 |
|---|---|---|---|
| SCR-001 Create workspace | P0 | 无 | lint/test/build 运行 |
| SCR-002 Windows injection adapter | P0 | 001 | Obsidian × 2 模式 |
| SCR-003 Linux X11 adapter | P0 | 001 | Unicode + Undo |
| SCR-004 Linux Wayland portal probe | P0 | 001 | GNOME/KDE 检测与许可 |
| SCR-005 Linux eitype / wtype fallback | P0 | 004 | 多后端不重复注入 |
| SCR-006 Focus guard & cancellation | P0 | 002/003 | 切窗口停止插入 |
| SCR-007 Audio capture + HUD | P0 | 001 | 麦克风与状态可见 |
| SCR-008 ASR backend | P0 | 007 | 20 语言配置 |
| SCR-009 Grammar pipeline | P0 | 008 | 保护结构与原文回退 |
| SCR-010 Dictation orchestrator | P0 | 006/009 | 完整端到端流程 |
| SCR-011 Auth + entitlements | P0 | 001 | 服务器权限真相源 |
| SCR-012 Stripe billing | P0 | 011 | 幂等 webhook |
| SCR-013 Security + data deletion | P0 | 008/011 | 无敏感日志，清理有效 |
| SCR-014 QA language + Obsidian | P0 | 010 | QA 矩阵通过 |
| SCR-015 Signed releases | P0 | 014 | 安装/升级/回滚验证 |
| SCR-016 Settings UI and diagnostics | P0 | 001/005 | 不成功时可解释 |
| SCR-017 Landing page | P0 | 012 | FR-014 内容齐全，通过 Stripe 网站审核 |
| SCR-018 China mainland availability | P0 | 008/012 | FR-015 全流程在大陆网络实测 |
| SCR-019 Paddle evaluation | P1 | 012/017 | ADR 记录结论 |
| SCR-020 GNOME Shell extension + KWin script | P0 | 006 | 焦点信息与录音指示；未安装时默认只复制（ADR-012） |
| SCR-021 Hallucination filter | P0 | 008 | QA-AI-004 通过（FR-017） |
| SCR-022 Dictation commands | P0 | 009 | QA-AI-005 通过（FR-016） |
| SCR-023 Personal dictionary | P0 | 008/011 | QA-AI-006 通过（FR-018） |
| SCR-024 Magic-link + device auth | P0 | 011 | QA-AUTH-002 通过（FR-019） |
| SCR-025 Obsidian plugin | P1 | 010 | Obsidian 内 `verified` 插入、一步撤销 |

## 5. 开发量预估方法

建议先为 SCR-002/003/004/005 做 1–2 个短周期探索，估算每个 Linux 组合器需要的权限引导、驱动依赖和兼容修复量，再决定整体工期。**不建议凭当前规格承诺“30 天双平台正式发布”**。

## 6. 风险优先级

1. Wayland 系统级快捷键和输入注入不可预测。
2. Obsidian 撤销、光标与富文本边界。
3. 非拉丁脚本与语法修正语义漂移。
4. 录音取消、异常网络、双次发送。
5. Linux 安装依赖以及商用用户权限接受度。
6. 真实用户获客/订阅续费，不是功能完成后自动发生。

## 7. 项目进度模板

```markdown
## 本周期目标：<...>
- 已完成/证据：<commit / QA case>
- 未完成/原因：<...>
- 最严重风险：<...>
- 下个 Gate 需要的证据：<...>
- 需要负责人决定：<...>
```

见 [测试](08-QA-ACCEPTANCE.md)、[风险/ADR](12-ADR-AND-RISKS.md)。
