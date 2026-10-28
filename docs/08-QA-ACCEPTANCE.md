# Scribiva v1.0 — 测试计划与上线验收

## 1. 测试对象与环境

**原则**：跨应用支持按系统环境和软件版本声明，不能只证明安装包能启动。

| Test env ID | 操作系统 | 显示会话 | 最低验证对象 |
|---|---|---|---|
| W11-01 | Windows 11 x64 | Native desktop | Obsidian、Word、VS Code、Chrome/Edge、聊天输入区 |
| LX11-01 | Linux Mint（Cinnamon）受测版 x64 | X11 | Obsidian、VS Code、浏览器 |
| LWG-01 | Ubuntu 24.04 x64（GNOME 46） | GNOME Wayland | Obsidian、VS Code、浏览器；无 GlobalShortcuts Portal |
| LWG-02 | Ubuntu 26.04 x64（GNOME 50） | GNOME Wayland | Obsidian、VS Code、浏览器；GlobalShortcuts Portal 可用 |
| LWK-01 | Fedora 受测稳定版本 x64（Plasma 6.8+） | KDE Plasma Wayland | Obsidian、VS Code、浏览器 |
| NET-CN-01 | 任一受支持环境 | 中国大陆网络（无代理） | landing page、下载、登录、听写、升级全流程（QA-CN-001）；延迟单独统计 |
| LWL-01 | Arch/受测版 | Hyprland 或 Sway | 技术验证，首版是否正式支持由 Gate 决定 |

> 以上为**拟测试矩阵**而非已通过的环境。Fedora/桌面/应用版本及具体硬件组合需在 QA 执行表登记，不能以“最新版”代替确切版本号。

## 2. 应用测试矩阵

| APP | 必验 | 输入行为 |
|---|---|---|
| Obsidian Live Preview | **P0** | 中文/阿拉伯文/列表/代码围栏/撤销/选区/焦点 |
| Obsidian Source Mode | **P0** | 与上项独立重复，保留 Markdown 原文 |
| Obsidian Reading View | **P0** | **不能**盲插，降级复制 |
| Chrome/Edge 普通输入框 | P0 | 单行/多行/UTF-8/emoji/RTL |
| Word | Windows P0 | 复杂文档/撤销/富文本 |
| VS Code / Cursor | P0 | 源代码面板、Markdown 文本、终端附近输入 |
| Slack/Discord | P0（支持矩阵选定版本） | 不自动发送；焦点失效时不贴入 |
| Google Docs/Notion | P1 或明确 Beta | 富文本兼容性独立评估 |
| PowerShell / Linux Terminal | 安全专项 | 默认不自动输入高风险命令/需确认 |
| 管理员/密码框 | 负面测试 | 禁止静默绕过系统权限 |

## 3. P0 可执行用例（示例）

| ID | 步骤 | 预期 |
|---|---|---|
| QA-INJ-001 | 将 `Hello 中文 العربية\n- **Markdown**` 注入 Obsidian Source Mode | 文字与换行顺序完全一致，插入正确光标位置 |
| QA-INJ-002 | 在 Live Preview 输入带列表和 Markdown 的内容 | 未破坏原有段落、插件和标记 |
| QA-INJ-003 | 完成一次插入后按一次撤销 | 可撤销整个本次输入；无原有笔记丢失 |
| QA-INJ-004 | 录音结束后切到另一聊天窗口 | 暂停自动注入；展示复制/选择目标 |
| QA-INJ-005 | 无粘贴权限，触发写入 | 明确错误、文本仍可复制 |
| QA-INJ-006 | 输入后端执行超时，用户点击重试 | 不出现不可控双重粘贴；不确定状态要求确认 |
| QA-INJ-007 | 写入时外部程序修改剪贴板 | 不覆盖后来写入的剪贴板内容 |
| QA-INJ-008 | 同一句包含中文、emoji、阿拉伯文 | 字符不丢失、不重新排序 |
| QA-REC-001 | 用户录音过程中取消 | 不上传音频，不调用语法修正、不插入 |
| QA-REC-002 | 录音已上传后取消 | 不应用延迟到达的结果，不自动插入 |
| QA-AI-001 | 智能修正修复明确错误 | 保持原意、数字、专名 |
| QA-AI-002 | 输入含代码围栏、文件路径、URL | 结构保护/校验成功，否则回退原文 |
| QA-AI-003 | LLM 超时但 ASR 完成 | 显示原文并可插入，不永久卡住 |
| QA-AI-004 | 录制 10 秒静音或环境噪声 | 不插入虚构文字；结果进入恢复区或报“未检测到语音”（FR-017） |
| QA-AI-005 | 口述“第一项 换行 列表项 第二项” | 输出确定的换行与 Markdown 列表，LLM 修正后结构不变（FR-016） |
| QA-AI-006 | 词典含 `Kubernetes`、`Scribiva`，在中文句中说出 | 拼写与词典一致，修正后不被改写（FR-018） |
| QA-AI-007 | 偏好简体，说一段普通话 | 输出全部为简体；偏好繁体时全部为繁体 |
| QA-INJ-009 | GNOME Wayland 未安装 Scribiva 扩展 | 默认只复制，不自动粘贴；提示安装扩展（ADR-012） |
| QA-INJ-010 | 焦点在终端窗口 | 默认只复制；不发送 Enter |
| QA-AUTH-001 | 禁用麦克风权限 | 无录音并有引导 |
| QA-BILL-001 | 免费额度用完仍发转写请求 | 服务端阻止并返回权益错误，不产生不可控供应商成本 |
| QA-BILL-002 | 模拟 Stripe 重复 webhook | 幂等，只更新一次权益状态 |
| QA-SEC-001 | 检索客户端及服务器日志 | 无音频正文、原始转写和密钥 |
| QA-UPDATE-001 | 更新包签名错误 | 拒绝安装并保留可运行旧版 |
| QA-BILL-003 | 跨自然月边界使用免费层 | 新月额度按服务端规则重置，客户端显示与服务端一致 |
| QA-BILL-004 | 订阅取消或欠费到期 | 账户回到免费层，不停用、不丢数据 |
| QA-BILL-005 | 本月剩余 20 秒时录 60 秒 | 本段完整处理并扣至 0；下一次录音开始前即提示升级 |
| QA-AUTH-002 | 新用户从桌面端点击登录 | 浏览器收到魔法链接完成登录后回到应用；同一邮箱不重复发放免费额度（FR-019） |
| QA-WEB-001 | 检查 landing page | FR-014 各项齐全、HTTPS、中英双语，价格与兼容列表和 QA 结论一致 |
| QA-CN-001 | 中国大陆网络（无代理）走官网→下载→注册→听写→升级 | 每步可用或有明确说明；延迟单独记录 |

## 4. 20 语言验收

为每种语言准备**独立标注集**：至少 30 条功能冒烟样例，并记录来源、口音、噪音、采样率、是否混说、标注者。必须覆盖表格中的 20 种且没有空白语言。

### 指标与判断

- ASR：按语种选择 WER（适合分词语言）、CER（中文/日文等）及人工语义审校。不能将某个单一 WER 阈值硬套全部 20 语言。
- 修正：检查语法错误修复率、**事实/数字/实体被误改率**、Markdown 保留率、空响应率和额外生成内容。
- 自动语种检测：短句与长句分开统计；置信不足时优先提示手动指定。
- 插入：每种语言在 Windows、GNOME Wayland 和 KDE Wayland 至少测试一次真实编辑器写入；关键文字体系需要扩充样本。

**建议的 Release Gate**：所有 20 语言都通过基础冒烟；任何单语严重误译、字符丢失或结构破坏为阻断项。具体 WER/CER 数值线由第一轮语料基线和目标人群共同确定，不能事先编造“准确率 ≥99%”。

## 5. 性能与可靠性

| 维度 | 初始目标/验收方法 |
|---|---|
| 录音状态响应 | 开始后 200ms 可见，QA 按平台测量 |
| 短录音完成到结果 | 15 秒以下录音 P95 ≤3 秒为初始目标，固定网络/设备后测试；失败则重新定基线 |
| 插入结果 | 记录 verified 与 dispatched_unverified，不混算 |
| 核心成功率 | 在受测应用矩阵中实际正确写入目标 ≥99%（P0 目标） |
| 无重复 | 100 次控制流/超时故障注入中 0 次双重自动插入 |
| 兼容稳定性 | 每套受支持 OS 连续 100 次录音任务无阻断崩溃（初始 smoke） |
| 麦克风异常 | 设备断开、独占、静音、权限撤销都有可恢复错误 |
| 更新 | 签名校验、升级、降级/回滚和配置迁移成功 |

## 6. Linux 专项

- 关闭 Portal、拒绝 Portal 授权、切换合成器、缺失 `wtype/eitype/dotool`、限制 `/dev/uinput`。
- 逐项测试不加入 `input` 组和未提供 root 权限时的行为；无特权应能进入安全的复制模式。
- Portal 授权后的恢复、重登、热键冲突及 KDE/GNOME 多屏。
- RTL/CJK 输入、远程桌面、非美式键盘布局、中文输入法开启时的输出行为。
- 测试记录必须注明 **发行版 + 内核 + 桌面环境/版本 + X11/Wayland + Portal 版本 + 注入后端**。

## 7. 发布检查表

- [ ] Windows + Linux 公开支持范围逐项 QA 通过，未通过环境标注 Beta/不支持。
- [ ] Obsidian 两种编辑模式通过，且 Reading View 能安全回退。
- [ ] 20 语言都完成 ASR 和语法修正基本测试；自动检测通过定义好的样本。
- [ ] 焦点变化、重复注入、异常取消、Clipboard 竞态通过。
- [ ] Stripe sandbox 及 production 小额真实支付与 webhook 通过。
- [ ] 更新包签名、安装、卸载、数据清理、旧版本回退通过。
- [ ] 隐私与第三方供应商披露完整，用户可删除临时结果。
- [ ] 核心用例有记录、失败截图和日志（无用户内容）。
- [ ] Landing page 上线：兼容列表、价格、下载与校验值、FAQ、已知限制、隐私/条款/退款政策与联系方式。
- [ ] Windows 与 Linux 同时通过本清单（ADR-008）。
- [ ] 中国大陆网络全流程（QA-CN-001）通过，或在官网明确列出限制。

## 8. 测试记录模板

```markdown
- Case ID: QA-INJ-001
- Build / commit: <sha>
- OS/version: <...>
- Session/compositor: <...>
- Target app/version/mode: <...>
- Input driver: <...>
- Language / sample ID: <...>
- Result: PASS | FAIL | BLOCKED | UNKNOWN
- Actual text vs expected: <redacted fixture reference>
- Screenshot/video: <safe private path>
- Error code: <...>
- Bug link / assignee: <...>
```
