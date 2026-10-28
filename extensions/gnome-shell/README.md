# Scribiva GNOME Shell 扩展

GNOME Wayland 不向普通应用提供焦点窗口信息，也不允许它们直接向其他窗口输入文字（ADR-012）。本扩展在 GNOME Shell 内部完成这两件事：

1. 告诉 Scribiva 停止录音时焦点在哪个窗口；
2. 把听写结果直接输入那个窗口的光标位置：优先通过输入法提交（与 GNOME 屏幕键盘相同的方式，不经过剪贴板，Unicode 完整，一次撤销），应用不支持时改用剪贴板加一次 Ctrl+V。确认焦点和输入在同一步完成，文字不会落进中途切换到的其他窗口。

另外在顶栏显示录音状态图标（不可点击、不抢焦点）。

支持 GNOME 45–50。

## 暴露的内容

总线名 `app.scribiva.Shell`，对象路径 `/app/scribiva/Shell`，接口 `app.scribiva.Shell1`：

- `GetFocus() -> (s app_id, s wm_class, t window_id, u pid)`：焦点窗口的应用 ID（如 `org.gnome.Terminal.desktop`）、WM class、Mutter 窗口 ID、进程 ID。没有焦点窗口，或 Shell 自身处于模态（概览、运行对话框、顶栏菜单）时返回 `('', '', 0, 0)`。
- `InsertText(s text, t expected_window_id, a{sv} options) -> s outcome`：焦点窗口仍是 `expected_window_id` 时输入 `text`。`options`：`single_line`（b，换行改为空格，用于终端）、`paste_shortcut`（s，`ctrl+v` 或 `ctrl+shift+v`）、`restore_clipboard`（b）、`paste_fallback`（b）。返回 `committed`（输入法提交）、`pasted`（粘贴兜底）、`focus_changed`、`no_focus`、`no_text_input`、`busy`、`invalid` 或 `failed`。从不发送回车。
- `SetState(s phase)`：`idle | recording | processing | error`。`idle` 隐藏图标；未知值忽略；非 `idle` 状态 10 分钟内没有更新会自动回到 `idle`。
- 属性 `Version`（s）= `"2"`（v1 没有 `InsertText`，客户端会退回 eitype 粘贴）。

不读取、不暴露窗口标题或任何窗口内容。扩展被禁用或锁屏时总线名消失，客户端应降级为只复制。

## 安装

```bash
./build.sh
gnome-extensions install --force dist/scribiva@scribiva.app.shell-extension.zip
# Wayland 下需注销并重新登录，Shell 才会发现新扩展
gnome-extensions enable scribiva@scribiva.app
```

## 测试

```bash
gdbus call --session --dest app.scribiva.Shell \
  --object-path /app/scribiva/Shell \
  --method app.scribiva.Shell1.GetFocus

gdbus call --session --dest app.scribiva.Shell \
  --object-path /app/scribiva/Shell \
  --method app.scribiva.Shell1.SetState recording

gdbus introspect --session --dest app.scribiva.Shell --object-path /app/scribiva/Shell
```

调试日志：`journalctl -f -o cat /usr/bin/gnome-shell`。

## 真机输入检查

在 GNOME 会话里运行，5 秒内点进任意输入框：

```bash
bash test/manual-check.sh
```

`test/run.sh` 是无头 GNOME Shell 测试：扩展能加载、D-Bus 接口可用；但在开发机上 GTK 测试窗口在无头会话里无法完成 Wayland 初始化，输入部分需用上面的真机检查。
