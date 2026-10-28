// Small UI string map: English + Simplified Chinese (docs/06 §8). Speech
// languages are a separate list; UI translation covers only these two.
export type UiLang = 'en' | 'zh';

const en = {
  appTagline: 'Voice typing for Windows and Linux',
  nav_status: 'Status', nav_settings: 'Settings', nav_dictionary: 'Dictionary', nav_account: 'Account & billing', nav_onboarding: 'Setup guide',
  uiLanguage: 'Interface language',
  // phases
  phase_idle: 'Ready', phase_recording: 'Recording', phase_processing: 'Transcribing…', phase_injecting: 'Inserting…',
  phase_ready: 'Paste sent (not verified)', phase_focus_changed: 'Window changed: paused to avoid a wrong paste',
  phase_clipboard_only: 'Copied to clipboard', phase_suspected_noise: 'No clear speech detected; result not inserted',
  phase_quota: 'Monthly free quota used up', phase_error: 'Dictation failed: audio kept for retry',
  phase_cancelled: 'Cancelled', phase_unknown: 'Unknown state',
  hint_idle: 'Keep the cursor in your target app and press {hotkey}.',
  hint_recording: 'Press {hotkey} again to stop, Esc to cancel.',
  hint_ready: 'Ctrl+V was sent once. Check the target app; it is never re-sent automatically.',
  notice_focus_changed: 'The foreground window changed after you stopped recording. The text is on the clipboard.',
  notice_focus_unknown: 'Scribiva could not identify the target window, so it only copied the text.',
  notice_terminal: 'Terminals are copy-only: pasting into a shell could run commands. Paste manually if intended.',
  notice_elevated: 'The target runs as administrator. Windows blocks input from normal apps (UIPI), so the text was only copied. Scribiva never asks for admin rights.',
  notice_own_window: 'Scribiva\'s own window was focused, so nothing was pasted.',
  notice_shell: 'No text field was focused (taskbar or Start). The text was copied.',
  notice_auto_insert_off: 'Auto-insert is off; the text was copied.',
  notice_modifiers_held: 'Keys were still held down, so no paste was sent. The text is on the clipboard.',
  notice_input_blocked: 'Windows refused the simulated keystrokes. The text is on the clipboard.',
  notice_too_short: 'Recording shorter than 1 second was dropped and not uploaded.',
  // actions
  start: 'Start', stop: 'Stop', cancel: 'Cancel', retry: 'Retry', discard: 'Discard', dismiss: 'Dismiss',
  copy: 'Copy', copyRaw: 'Copy original', showRaw: 'View original transcript', showFinal: 'View edited text', clear: 'Clear result',
  lastResult: 'Last result', noResult: 'No result yet', memoryOnly: 'Kept in memory only until you clear it or quit.',
  warnings: 'Notes',
  // settings
  sec_writing: 'Language & writing', sec_audio: 'Microphone', sec_input: 'Hotkey & insertion', sec_privacy: 'Privacy',
  speechLanguage: 'Speech language', autoDetect: 'Auto-detect',
  mode: 'Writing mode', mode_raw: 'Raw (no AI edit)', mode_standard: 'Smart edit', mode_polished: 'Polished',
  polishedWarn: 'Polished may change wording. Please proofread.',
  script: 'Chinese output', script_auto: 'As recognised', script_hans: 'Simplified 简体', script_hant: 'Traditional 繁體',
  dictationCommands: 'Spoken formatting commands ("new line", "new paragraph", "换行")',
  mic: 'Input device', micDefault: 'System default', micTest: 'Test microphone', micLevel: 'Level',
  micTestHint: 'Speak for 1.5 seconds. The audio stays on this device and is discarded.',
  maxDuration: 'Maximum recording length (seconds)',
  hotkey: 'Toggle hotkey', hotkeyHelp: 'Example: Ctrl+Alt+V, Ctrl+Shift+Space, F13. Press Enter to apply.', hotkeyReset: 'Reset to default',
  autoInsert: 'Paste automatically into the focused app',
  autoInsertRisk: 'Risk: a keystroke is sent to whatever window has focus. Scribiva checks the window did not change and skips terminals and admin windows, but cannot confirm the text landed. When off, results are only copied.',
  insertWithoutFocus: 'Insert even when the focused window cannot be verified',
  insertWithoutFocusRisk: 'Risk: on desktops without focus information (e.g. GNOME Wayland without the Scribiva extension), text may be pasted into the wrong window, including chats or terminals. Leave off unless you accept that.',
  restoreClipboard: 'Restore my previous plain-text clipboard after pasting',
  notifications: 'Show Windows notifications for results that need attention',
  saved: 'Saved', apply: 'Apply',
  privacyNote: 'Audio is uploaded only after you stop recording, processed in the United States by Scribiva and Groq, and not stored by the server. Results stay on the server for a short time (about 15 minutes) for recovery.',
  diagnostics: 'Diagnostics',
  // account
  serviceUrl: 'Service URL', signIn: 'Sign in with email', signOut: 'Sign out', signedInAs: 'Signed in as',
  notSignedIn: 'Not signed in', loginCode: 'Your login code', loginOpened: 'A browser window opened. Enter your email there and click the link we send.',
  loginWaiting: 'Waiting for confirmation…', loginExpired: 'The login code expired. Start again.',
  plan: 'Plan', usage: 'This month', remaining: 'Remaining', resetsAt: 'Resets', grace: 'Month-end grace for one recording',
  upgradeMonthly: 'Upgrade (monthly)', upgradeAnnual: 'Upgrade (annual)', manage: 'Manage subscription', refresh: 'Refresh',
  billingNote: 'Payment happens on Stripe in your browser. Your plan updates after Stripe confirms it.',
  quotaHint: 'Your free quota for this month is used up. It resets {date}.',
  deleteData: 'Delete my server data', deleteConfirm: 'Type DELETE to confirm deletion of transcripts, jobs and sign-in tokens on the server.',
  deleteDone: 'Server data deleted. You are signed out.',
  // dictionary
  dictHelp: 'Names, products and code identifiers that should be recognised and kept as written. Stored with your account.',
  dictAdd: 'Add term', dictRemove: 'Remove', dictSave: 'Save dictionary', dictImport: 'Import', dictExport: 'Export',
  dictCount: '{n} of {max} terms', dictNeedsLogin: 'Sign in to edit your dictionary.',
  // onboarding
  ob_title: 'Welcome to Scribiva', ob_value: 'Dictate into any supported editor: 20 languages with automatic grammar editing.',
  ob_privacy_title: 'Privacy', ob_privacy: 'To transcribe, your recording is uploaded to Scribiva servers in the United States and processed by Groq (speech recognition and text editing). For users in mainland China this is a cross-border transfer. Scribiva only records while you press the hotkey.',
  ob_agree: 'I understand and agree', ob_next: 'Next', ob_back: 'Back', ob_finish: 'Finish',
  ob_login_title: 'Sign in', ob_mic_title: 'Microphone test', ob_hotkey_title: 'Hotkey',
  ob_hotkey_body: 'Press {hotkey} anywhere to start, again to stop. Esc cancels. Scribiva stays in the tray and will not take focus.',
  ob_linux_hint: 'On Linux, the hotkey and input backend are configured by the Scribiva CLI (see Diagnostics).',
  error: 'Error',
};

export type Key = keyof typeof en;

const zh: Record<Key, string> = {
  appTagline: 'Windows 与 Linux 语音输入',
  nav_status: '状态', nav_settings: '设置', nav_dictionary: '个人词典', nav_account: '账户与订阅', nav_onboarding: '新手引导',
  uiLanguage: '界面语言',
  phase_idle: '准备就绪', phase_recording: '正在录音', phase_processing: '正在转写…', phase_injecting: '正在输入…',
  phase_ready: '已发送粘贴（未验证）', phase_focus_changed: '输入位置已变化，为避免误贴，已暂停',
  phase_clipboard_only: '已复制到剪贴板', phase_suspected_noise: '未检测到清晰语音，结果未插入',
  phase_quota: '本月免费额度已用完', phase_error: '转写失败，录音未丢失',
  phase_cancelled: '已取消', phase_unknown: '未知状态',
  hint_idle: '保持目标应用的光标位置，按 {hotkey} 开始说话。',
  hint_recording: '再按 {hotkey} 停止，按 Esc 取消。',
  hint_ready: '已发送一次 Ctrl+V。请检查目标应用；不会自动重发。',
  notice_focus_changed: '停止录音后前台窗口发生了变化，文字已放入剪贴板。',
  notice_focus_unknown: '无法确认输入位置，已只复制到剪贴板。',
  notice_terminal: '终端窗口只复制不粘贴：粘贴到命令行可能执行命令。如需要请手动粘贴。',
  notice_elevated: '目标窗口以管理员权限运行，Windows（UIPI）会阻止普通程序输入，因此只复制。Scribiva 不会请求管理员权限。',
  notice_own_window: '当前焦点是 Scribiva 自己的窗口，未粘贴。',
  notice_shell: '当前没有输入框（任务栏或开始菜单），文字已复制。',
  notice_auto_insert_off: '自动插入已关闭，文字已复制。',
  notice_modifiers_held: '按键仍被按住，未发送粘贴。文字已在剪贴板中。',
  notice_input_blocked: 'Windows 拒绝了模拟按键。文字已在剪贴板中。',
  notice_too_short: '录音不足 1 秒，已丢弃且未上传。',
  start: '开始', stop: '停止', cancel: '取消', retry: '重试', discard: '丢弃', dismiss: '关闭',
  copy: '复制', copyRaw: '复制原文', showRaw: '查看 ASR 原文', showFinal: '查看修正后文本', clear: '清除结果',
  lastResult: '最近一次结果', noResult: '尚无结果', memoryOnly: '仅保存在内存中，清除或退出后即删除。',
  warnings: '提示',
  sec_writing: '语言与写作', sec_audio: '麦克风', sec_input: '快捷键与插入', sec_privacy: '隐私',
  speechLanguage: '听写语言', autoDetect: '自动检测',
  mode: '写作模式', mode_raw: '原文（不经 AI 修改）', mode_standard: '智能修正', mode_polished: '流畅表达',
  polishedWarn: '流畅表达可能改变措辞，请校对。',
  script: '中文输出字形', script_auto: '按识别结果', script_hans: '简体', script_hant: '繁體',
  dictationCommands: '口述格式命令（“换行”“新段落”“new line”）',
  mic: '输入设备', micDefault: '系统默认', micTest: '测试麦克风', micLevel: '音量',
  micTestHint: '请说话 1.5 秒。音频只在本机处理并立即丢弃。',
  maxDuration: '最长录音时长（秒）',
  hotkey: '开始/停止快捷键', hotkeyHelp: '示例：Ctrl+Alt+V、Ctrl+Shift+Space、F13。按回车生效。', hotkeyReset: '恢复默认',
  autoInsert: '自动粘贴到当前应用',
  autoInsertRisk: '风险：按键会发送到当前焦点窗口。Scribiva 会确认窗口未变化，并跳过终端和管理员窗口，但无法确认文字确实写入。关闭后结果只复制。',
  insertWithoutFocus: '无法校验焦点时也自动插入',
  insertWithoutFocusRisk: '风险：在无法获得焦点信息的桌面（如未安装 Scribiva 扩展的 GNOME Wayland），文字可能被粘贴到错误窗口，包括聊天或终端。除非接受该风险，否则请保持关闭。',
  restoreClipboard: '粘贴后恢复原有的纯文本剪贴板',
  notifications: '需要处理时显示 Windows 通知',
  saved: '已保存', apply: '应用',
  privacyNote: '只有在停止录音后才上传音频，由 Scribiva 与 Groq 在美国处理，服务器不保存音频。结果在服务器上短暂保留（约 15 分钟）以便恢复。',
  diagnostics: '环境诊断',
  serviceUrl: '服务地址', signIn: '使用邮箱登录', signOut: '退出登录', signedInAs: '已登录',
  notSignedIn: '未登录', loginCode: '登录码', loginOpened: '已打开浏览器。请在网页中输入邮箱，并点击邮件中的链接。',
  loginWaiting: '等待确认…', loginExpired: '登录码已过期，请重新开始。',
  plan: '套餐', usage: '本月用量', remaining: '剩余', resetsAt: '重置时间', grace: '月末单段宽限',
  upgradeMonthly: '升级（月付）', upgradeAnnual: '升级（年付）', manage: '管理订阅', refresh: '刷新',
  billingNote: '在浏览器中通过 Stripe 付款。Stripe 确认后套餐才会更新。',
  quotaHint: '本月免费额度已用完，将于 {date} 重置。',
  deleteData: '删除我的服务器数据', deleteConfirm: '输入 DELETE 以确认删除服务器上的转写、任务和登录凭据。',
  deleteDone: '服务器数据已删除，已退出登录。',
  dictHelp: '需要正确识别并保持原样的人名、产品名和代码标识符。随账户保存在服务器。',
  dictAdd: '添加术语', dictRemove: '删除', dictSave: '保存词典', dictImport: '导入', dictExport: '导出',
  dictCount: '{n} / {max} 个术语', dictNeedsLogin: '登录后才能编辑个人词典。',
  ob_title: '欢迎使用 Scribiva', ob_value: '在受支持的编辑器中直接语音输入：20 种语言，自动语法修正。',
  ob_privacy_title: '隐私说明', ob_privacy: '为完成转写，录音会上传到位于美国的 Scribiva 服务器，并由 Groq 进行语音识别与文本处理。对中国大陆用户而言这属于数据出境。Scribiva 只在你按下快捷键后录音。',
  ob_agree: '我已了解并同意', ob_next: '下一步', ob_back: '上一步', ob_finish: '完成',
  ob_login_title: '登录', ob_mic_title: '麦克风测试', ob_hotkey_title: '快捷键',
  ob_hotkey_body: '在任意位置按 {hotkey} 开始，再按一次停止，Esc 取消。Scribiva 常驻托盘，不会抢占焦点。',
  ob_linux_hint: 'Linux 上的快捷键与输入后端由 Scribiva CLI 配置（见环境诊断）。',
  error: '错误',
};

const tables: Record<UiLang, Record<Key, string>> = { en, zh };

export function translate(lang: UiLang, key: Key, vars?: Record<string, string | number>): string {
  let s = tables[lang][key] ?? en[key];
  if (vars) for (const [k, v] of Object.entries(vars)) s = s.split(`{${k}}`).join(String(v));
  return s;
}

export function phaseKey(phase: string): Key {
  const k = `phase_${phase}` as Key;
  return k in en ? k : 'phase_unknown';
}

export function noticeKey(notice: string): Key | null {
  const k = `notice_${notice}` as Key;
  return k in en ? k : null;
}

/** Speech languages: auto + 20 (PRD §5), labelled in their own language. */
export const SPEECH_LANGUAGES: [string, string][] = [
  ['en', 'English'], ['zh', '中文'], ['ja', '日本語'], ['ko', '한국어'], ['es', 'Español'], ['fr', 'Français'],
  ['de', 'Deutsch'], ['pt', 'Português'], ['it', 'Italiano'], ['ru', 'Русский'], ['ar', 'العربية'], ['hi', 'हिन्दी'],
  ['tr', 'Türkçe'], ['nl', 'Nederlands'], ['pl', 'Polski'], ['id', 'Bahasa Indonesia'], ['vi', 'Tiếng Việt'],
  ['th', 'ไทย'], ['sv', 'Svenska'], ['uk', 'Українська'],
];

/** Server timestamps are UTC ISO; show them in the user's local time zone. */
export function formatLocal(iso: string | null | undefined, lang: UiLang): string {
  if (!iso) return '-';
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '-';
  return d.toLocaleString(lang === 'zh' ? 'zh-CN' : undefined, { dateStyle: 'medium', timeStyle: 'short' });
}

export function formatSeconds(s: number | null | undefined, lang: UiLang): string {
  const v = Math.max(0, Math.round(Number(s) || 0));
  const m = Math.floor(v / 60);
  const r = v % 60;
  return lang === 'zh' ? `${m} 分 ${r} 秒` : `${m} min ${r} s`;
}
