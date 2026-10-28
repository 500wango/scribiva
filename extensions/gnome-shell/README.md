# Scribiva GNOME Shell 扩展

GNOME Wayland 不向普通应用提供焦点窗口信息（ADR-012）。本扩展通过会话 D-Bus 把当前焦点窗口的最少信息提供给本机 Scribiva 客户端，用来确认粘贴目标；并在顶栏显示录音状态图标（不可点击、不抢焦点）。

支持 GNOME 45–50。

## 暴露的内容

总线名 `app.scribiva.Shell`，对象路径 `/app/scribiva/Shell`，接口 `app.scribiva.Shell1`：

- `GetFocus() -> (s app_id, s wm_class, t window_id, u pid)`：焦点窗口的应用 ID（如 `org.gnome.Terminal.desktop`）、WM class、Mutter 窗口 ID、进程 ID。没有焦点窗口，或 Shell 自身处于模态（概览、运行对话框、顶栏菜单）时返回 `('', '', 0, 0)`。
- `SetState(s phase)`：`idle | recording | processing | error`。`idle` 隐藏图标；未知值忽略；非 `idle` 状态 10 分钟内没有更新会自动回到 `idle`。
- 属性 `Version`（s）= `"1"`。

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
