# Scribiva v1.0 — 开发环境与协作规范

> 这是 v1.0 **建议的仓库结构/命令约定**。当前可运行的 `0.2.0-alpha.1`（Python 客户端 + stdlib/SQLite 服务端）结构和命令见仓库根 README；下列 npm/cargo 命令尚未在本仓库中运行通过。

## 1. 技术栈基线

- 桌面 UI：Tauri 2、React、TypeScript。
- 桌面核心：Rust，操作系统能力通过 trait 封装。
- 后端：Python 3.12 标准库 + SQLite（现有 `server/scribiva_server/`，ADR-011）；多实例需求出现时再迁移 PostgreSQL。
- 供应商：Groq Whisper 多语言 ASR 与 Groq LLM 后处理（已定，ADR-010），经适配层接入；支付 Stripe（现有账户），Paddle 待评估。
- 部署：服务端与 landing page 部署在美国服务器。
- 开发机器：Windows 11 x64 与至少 2 台独立 Linux 测试机/VM（GNOME/KDE Wayland 和 X11）。VM 录音/Portal/输入仿真存在偏差，最终须有真实桌面验证。

## 2. Repo 结构建议

```text
scribiva/
  README.md
  docs/                       # 规范文档（本目录）
  src/scribiva/               # 现有 Python Linux 客户端（alpha）
  server/scribiva_server/     # 服务端：Python + SQLite（ADR-011）
  desktop/                    # Tauri 2 + React 桌面端
    src/
    src-tauri/
  extensions/
    gnome-shell/              # 焦点信息与录音指示（ADR-012）
    kwin/
  web/                        # 本地 dashboard
  landing/                    # 官网 landing page（FR-014）
  tests/
  fixtures/
    multilingual/             # 有授权的人工语音标注材料
    markdown-injection/
  scripts/
  .github/workflows/
  desktop/package-lock.json   # 前端依赖锁定
  desktop/src-tauri/Cargo.lock
```

## 3. 初始化顺序

1. 在 `desktop/` 安装依赖并编译 Tauri 2 + React 项目；安装 Rust 与 Node LTS（使用各自官方受支持版本）。
2. 只先写 `src-tauri` 的 `HotkeyAdapter / FocusAdapter / TextInjector` trait 和最小驱动，验证真实桌面。
3. 在现有 Python 服务端上补齐魔法链接登录（FR-019）、数据库迁移脚本与备份；音频长度限制和幂等逻辑已有实现。
4. 接入 ASR、LLM 后处理；固定契约并编写离线测试 fixture。
5. 实现支付、更新、日志、兼容性测试及官网披露。

以下是**意图性命令范例**（需要开发者按脚手架和操作系统修订）：

```bash
# 在仓库根目录执行
(cd desktop && npm ci && npm run tauri dev)
PYTHONPATH=server python3 -m scribiva_server serve --host 127.0.0.1 --port 8787
PYTHONPATH=src:server python3 -m unittest discover -s tests -p 'test_*.py'
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all
(cd desktop && npm run build)
```

`npm ci` 需要先生成并提交 `desktop/package-lock.json`；cargo 命令在 `desktop/src-tauri/` 下执行。

## 4. 系统依赖

**Windows**：Rust toolchain、Visual Studio C++ 构建工具、WebView2/Tauri 所需运行时，麦克风权限与一个可编辑的 Obsidian 测试 Vault。安装包签名需要合法签名证书或对应发布服务。

**Linux**：Rust、WebKitGTK/Tauri 编译依赖、PipeWire/PulseAudio、X11/Wayland 开发依赖（按发行版官方说明）。输入后端按环境选装：`wtype`、`eitype`、`dotool`、`ydotool`、`wl-clipboard`、`xclip`，**并非要求全部安装**；优先以 Portal/合成器方式获得可用能力，不默认加 `input` 组。

**注意**：Voxtype 的安装说明不能直接作为 Scribiva 的系统依赖承诺，因为二者架构、发行包和 glibc 目标不同。

## 5. 代码规范与 PR 模板

- Rust/TypeScript 依赖锁定；CI 通过 lint、test 和静态分析。
- 每个变更一个 PR，描述需求 ID、具体测试系统与结果。
- 任何修改注入后端必须补充“焦点变化/Unicode/撤销/不重复发送”用例。
- 任何修改语言后处理必须补充“事实保护/Markdown 保护/多语言”回归。
- 数据库/订阅变更必须迁移可重放，webhook 处理幂等。

```markdown
## Purpose
- Requirement ID: FR-xxx
- What changed:
## Validation
- OS, session, target app/version:
- QA test IDs:
- Security/Privacy implications:
## Rollback
- How to revert safely:
## Screenshots or recordings
- Sanitized fixtures only, no real user audio/text
```

## 6. 本地调试流程

1. 先使用**硬编码静态测试文本**验证注入，不联接 API。
2. 再接本地 fixture 音频，模拟 ASR 成功/失败/超时。
3. 注入当前编辑器前对焦点进行快照和确认。
4. 对 Linux 记录 `session/compositor/portal/backend` 的**非敏感元数据**。
5. 开启网络后只传人工测试音频，正式用户录音不得进入开发环境。

## 7. 代码开源和许可证

Voxtype README 表明其仓库为 MIT 许可。复用具体代码时需要在仓库中保留原作者许可及版权声明，核对目标源码文件、打包资源和所有依赖的许可证；仅参考策略而重写实现也应在技术致谢中标明参考。

## 8. 第一条可合并 PR

Python alpha 已有 `doctor` 与 GNOME Wayland 注入实现。v1.0 的首批 PR 建议按风险顺序：

1. GNOME Shell 扩展原型：经 D-Bus 返回焦点窗口 ID，客户端在注入前比对；未安装时只复制（ADR-012，SCR-020）。
2. Windows 注入 PoC：静态文本写入 Obsidian 两种模式并验证撤销（G0-01）。与上一项并行，避免 Windows 拖后同时首发。
3. Tauri 端的 `TextInjector` / `FocusAdapter` trait 与 `doctor`，不执行注入。

每个 PR 只引入一类系统权限，便于审查。
