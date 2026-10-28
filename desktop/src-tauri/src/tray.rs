//! Tray icon, menu, tooltip and optional notification (Windows and Linux).
//! The tray is the main surface: the window is hidden on close and never
//! shown or focused when recording starts (no focus steal). On Linux the
//! icon is an AppIndicator (libayatana-appindicator3), whose menu is the
//! only interaction (no click events).

use crate::core::Phase;
use crate::native::{Native, StatusView};
use std::sync::Arc;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_notification::NotificationExt;

const TRAY_ID: &str = "scribiva-tray";

fn icon(kind: &str) -> Option<Image<'static>> {
    let bytes: &[u8] = match kind {
        "recording" => include_bytes!("../icons/tray-recording.png"),
        "processing" => include_bytes!("../icons/tray-processing.png"),
        "attention" => include_bytes!("../icons/tray-attention.png"),
        _ => include_bytes!("../icons/tray-idle.png"),
    };
    Image::from_bytes(bytes).ok().map(|i| i.to_owned())
}

/// Short UI strings for tray and toasts (`en` / `zh`).
fn t(lang: &str, key: &str) -> &'static str {
    let zh = lang == "zh";
    match key {
        "start" => {
            if zh {
                "开始录音"
            } else {
                "Start dictation"
            }
        }
        "stop" => {
            if zh {
                "停止录音"
            } else {
                "Stop dictation"
            }
        }
        "cancel" => {
            if zh {
                "取消"
            } else {
                "Cancel"
            }
        }
        "mode" => {
            if zh {
                "写作模式"
            } else {
                "Writing mode"
            }
        }
        "raw" => {
            if zh {
                "原文"
            } else {
                "Raw"
            }
        }
        "standard" => {
            if zh {
                "智能修正"
            } else {
                "Smart edit"
            }
        }
        "polished" => {
            if zh {
                "流畅表达"
            } else {
                "Polished"
            }
        }
        "copy" => {
            if zh {
                "复制最近结果"
            } else {
                "Copy last result"
            }
        }
        "insert" => {
            if zh {
                "插入最近结果"
            } else {
                "Insert last result"
            }
        }
        "discard_last" => {
            if zh {
                "丢弃最近结果"
            } else {
                "Discard last result"
            }
        }
        "update_ready" => {
            if zh {
                "重启以安装更新"
            } else {
                "Restart to install update"
            }
        }
        "update_manual" => {
            if zh {
                "有新版本：前往官网下载"
            } else {
                "New version: download from the website"
            }
        }
        "injecting" => {
            if zh {
                "正在输入…"
            } else {
                "Inserting…"
            }
        }
        "settings" => {
            if zh {
                "打开设置"
            } else {
                "Open settings"
            }
        }
        "quit" => {
            if zh {
                "退出"
            } else {
                "Quit"
            }
        }
        "idle" => {
            if zh {
                "按快捷键开始说话"
            } else {
                "Press the hotkey to start"
            }
        }
        "recording" => {
            if zh {
                "正在录音"
            } else {
                "Recording"
            }
        }
        "processing" => {
            if zh {
                "正在转写…"
            } else {
                "Transcribing…"
            }
        }
        "ready" => {
            if zh {
                "已发送粘贴（未验证）"
            } else {
                "Paste sent (not verified)"
            }
        }
        "focus_changed" => {
            if zh {
                "输入位置已变化，未粘贴；文字已复制"
            } else {
                "Window changed: not pasted, text copied"
            }
        }
        "clipboard_only" => {
            if zh {
                "已复制到剪贴板"
            } else {
                "Copied to clipboard"
            }
        }
        "suspected_noise" => {
            if zh {
                "未检测到清晰语音，结果未插入"
            } else {
                "No clear speech; result not inserted"
            }
        }
        "quota" => {
            if zh {
                "本月额度已用完"
            } else {
                "Monthly quota used up"
            }
        }
        "error" => {
            if zh {
                "需要处理：转写失败"
            } else {
                "Needs attention: dictation failed"
            }
        }
        "cancelled" => {
            if zh {
                "已取消"
            } else {
                "Cancelled"
            }
        }
        _ => "",
    }
}

/// Update entry shown in the tray, if any.
fn update_item(native: &Native) -> Option<&'static str> {
    use crate::updater::UpdateState;
    match native.update_state().map(|u| u.state) {
        Some(UpdateState::Ready) => Some("update_ready"),
        Some(UpdateState::Manual) => Some("update_manual"),
        _ => None,
    }
}

fn build_menu(
    app: &AppHandle,
    lang: &str,
    mode: &str,
    update: Option<&'static str>,
) -> tauri::Result<Menu<Wry>> {
    let toggle = MenuItem::with_id(app, "toggle", t(lang, "start"), true, None::<&str>)?;
    // Cancel works for recording, transcribing and inserting (on Linux this
    // and `scribiva --cancel` replace the global Esc of Windows).
    let cancel = MenuItem::with_id(app, "cancel", t(lang, "cancel"), true, None::<&str>)?;
    let raw = CheckMenuItem::with_id(
        app,
        "mode:raw",
        t(lang, "raw"),
        true,
        mode == "raw",
        None::<&str>,
    )?;
    let std_ = CheckMenuItem::with_id(
        app,
        "mode:standard",
        t(lang, "standard"),
        true,
        mode == "standard",
        None::<&str>,
    )?;
    let pol = CheckMenuItem::with_id(
        app,
        "mode:polished",
        t(lang, "polished"),
        true,
        mode == "polished",
        None::<&str>,
    )?;
    let modes = Submenu::with_items(app, t(lang, "mode"), true, &[&raw, &std_, &pol])?;
    let copy = MenuItem::with_id(app, "copy", t(lang, "copy"), true, None::<&str>)?;
    // Explicit user request, into the window that has focus once the menu
    // closes ("insert anyway" for suspected noise, "insert again").
    let insert = MenuItem::with_id(app, "insert", t(lang, "insert"), true, None::<&str>)?;
    let discard = MenuItem::with_id(
        app,
        "discard_last",
        t(lang, "discard_last"),
        true,
        None::<&str>,
    )?;
    let settings = MenuItem::with_id(app, "settings", t(lang, "settings"), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", t(lang, "quit"), true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[
            &toggle, &cancel, &sep1, &modes, &copy, &insert, &discard, &sep2, &settings,
        ],
    )?;
    if let Some(key) = update {
        menu.append(&MenuItem::with_id(
            app,
            key,
            t(lang, key),
            true,
            None::<&str>,
        )?)?;
    }
    menu.append(&quit)?;
    Ok(menu)
}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

pub fn setup(app: &AppHandle, native: &Arc<Native>) -> tauri::Result<()> {
    let s = native.settings();
    let menu = build_menu(app, &s.ui_language, &s.mode, update_item(native))?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip(format!("Scribiva · {}", t(&s.ui_language, "idle")))
        .on_menu_event(|app, event| {
            let Some(native) = app.try_state::<Arc<Native>>() else {
                return;
            };
            let native = native.inner().clone();
            let id = event.id().as_ref().to_string();
            let app = app.clone();
            std::thread::spawn(move || match id.as_str() {
                "toggle" => native.toggle(),
                "cancel" => native.cancel(),
                "copy" => {
                    let _ = native.copy_last(false);
                }
                "insert" => native.insert_last(false),
                "discard_last" => native.discard_last(),
                "update_ready" => {
                    if crate::updater::install(&native).is_err() {
                        let a = app.clone();
                        let _ = app.run_on_main_thread(move || show_main(&a));
                    }
                }
                "update_manual" => {
                    let _ = crate::updater::open_release_page(&app);
                }
                "settings" => {
                    let a = app.clone();
                    let _ = app.run_on_main_thread(move || show_main(&a));
                }
                "quit" => {
                    native.cancel();
                    app.exit(0);
                }
                m if m.starts_with("mode:") => {
                    let _ = native.set_setting("mode", &m[5..]);
                    rebuild_menu(&app, &native.settings().ui_language);
                }
                _ => {}
            });
        })
        .on_tray_icon_event(|tray, event| {
            // Left click opens settings; recording is started by hotkey or menu.
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        });
    if let Some(i) = icon("idle") {
        builder = builder.icon(i);
    }
    builder.build(app)?;
    Ok(())
}

pub fn rebuild_menu(app: &AppHandle, lang: &str) {
    let Some(native) = app.try_state::<Arc<Native>>() else {
        return;
    };
    let mode = native.settings().mode;
    let update = update_item(&native);
    if let (Some(tray), Ok(menu)) = (
        app.tray_by_id(TRAY_ID),
        build_menu(app, lang, &mode, update),
    ) {
        let _ = tray.set_menu(Some(menu));
    }
}

fn icon_kind(phase: Phase) -> &'static str {
    match phase {
        Phase::Recording => "recording",
        Phase::Processing | Phase::Injecting => "processing",
        Phase::FocusChanged | Phase::SuspectedNoise | Phase::Quota | Phase::Error => "attention",
        _ => "idle",
    }
}

/// Tooltip text always names the state, so status is never colour-only.
pub fn update(app: &AppHandle, view: &StatusView, lang: &str) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let mut text = format!("Scribiva · {}", t(lang, view.phase.as_str()));
    if view.phase == Phase::Idle && !view.hotkey.is_empty() {
        text = format!("Scribiva · {} ({})", t(lang, "idle"), view.hotkey);
    }
    if let Some(e) = view
        .hotkey_error
        .as_ref()
        .filter(|_| view.phase == Phase::Idle)
    {
        text = format!("Scribiva · {e}");
    }
    // Windows truncates tooltips at 127 characters.
    let text: String = text.chars().take(120).collect();
    let _ = tray.set_tooltip(Some(text));
    let _ = tray.set_icon(icon(icon_kind(view.phase)));
}

/// Toast only for outcomes that need the user's attention; never content.
pub fn toast(app: &AppHandle, view: &StatusView, lang: &str) {
    let key = match view.phase {
        Phase::FocusChanged
        | Phase::ClipboardOnly
        | Phase::SuspectedNoise
        | Phase::Quota
        | Phase::Error => view.phase.as_str(),
        _ => return,
    };
    let _ = app
        .notification()
        .builder()
        .title("Scribiva")
        .body(t(lang, key))
        .show();
}
