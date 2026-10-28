//! Platform-neutral orchestrator (Windows and Linux): owns the state
//! machine, microphone, focus snapshot, upload and delivery. OS specifics go
//! through `crate::platform`. Locking rule: the `inner` mutex is never held
//! while calling into Tauri (emit, tray, shortcuts), because those calls
//! marshal to the main thread and wait.

use crate::api::{self, ApiClient, CallError, DictationRequest, DictationResponse, Me};
use crate::audio_dsp::{self, TARGET_RATE};
use crate::core::{
    CancelEffect, Core, ErrorInfo, FailureClass, LastResult, Phase, ResultAction, RetryDecision,
    StopDecision,
};
use crate::focus_policy::FocusSnapshot;
use crate::platform::{self, audio, clipboard, focus, hotkey};
use crate::storage::{self, Settings};
use crate::tray;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// `CODE: message` → (code, message).
fn split_code(e: &str) -> (String, String) {
    match e.split_once(": ") {
        Some((c, m)) if c.chars().all(|ch| ch.is_ascii_uppercase() || ch == '_') => {
            (c.to_string(), m.to_string())
        }
        _ => ("ERROR".to_string(), e.to_string()),
    }
}

pub fn local_error(e: &str) -> ErrorInfo {
    let (code, message) = split_code(e);
    ErrorInfo {
        code,
        message,
        class: FailureClass::Local,
        retryable: false,
    }
}

/// Audio kept in memory only, for retry after a failure (never on disk).
struct Retained {
    pub job_id: String,
    pub wav: Vec<u8>,
    pub duration_ms: u64,
    pub focus: Option<FocusSnapshot>,
}

/// Input device as shown in the UI. `id` is a cpal device id (Windows) or a
/// PipeWire `node.name` (Linux); empty = system default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InputDevice {
    pub id: String,
    pub name: String,
    pub default: bool,
    #[serde(rename = "virtual")]
    pub is_virtual: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceList {
    pub devices: Vec<InputDevice>,
    /// Device a recording would use right now (Linux), if known.
    pub active: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LoginView {
    pub user_code: String,
    pub verification_uri: String,
    pub expires_at_ms: u64,
    pub error: Option<String>,
}

struct Inner {
    pub core: Core,
    pub settings: Settings,
    pub config_dir: PathBuf,
    pub recorder: Option<audio::Recorder>,
    pub retained: Option<Retained>,
    pub token: Option<String>,
    pub account: Option<Me>,
    pub login: Option<LoginView>,
    pub login_gen: u64,
    pub hotkey_error: Option<String>,
    /// Shortcut shown in the UI when it is not `settings.hotkey` (Linux:
    /// the GNOME custom shortcut, empty when none is set up).
    pub hotkey_label: Option<String>,
    published: Option<Phase>,
}

pub struct Native {
    pub app: AppHandle,
    inner: Mutex<Inner>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusView {
    pub phase: Phase,
    pub job_id: Option<String>,
    pub recording_started_ms: Option<u64>,
    pub max_record_seconds: u32,
    pub error: Option<ErrorInfo>,
    pub notice: Option<String>,
    pub has_last: bool,
    pub can_retry: bool,
    pub signed_in: bool,
    pub service_url: String,
    pub hotkey: String,
    pub hotkey_error: Option<String>,
    pub login: Option<LoginView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LastView {
    pub available: bool,
    #[serde(flatten)]
    pub last: Option<LastResult>,
}

impl Native {
    /// The token is loaded separately (`load_token`), because the Linux
    /// Secret Service may block on an unlock prompt.
    pub fn new(app: AppHandle, config_dir: PathBuf) -> Arc<Self> {
        let settings = storage::load_settings(&config_dir);
        Arc::new(Self {
            app,
            inner: Mutex::new(Inner {
                core: Core::new(),
                settings,
                config_dir,
                recorder: None,
                retained: None,
                token: None,
                account: None,
                login: None,
                login_gen: 0,
                hotkey_error: None,
                hotkey_label: None,
                published: None,
            }),
        })
    }

    /// Read the token from the OS secret store. Returns whether one exists.
    pub fn load_token(&self) -> bool {
        let dir = self.lock().config_dir.clone();
        let token = storage::load_token(&dir);
        let found = token.is_some();
        self.lock().token = token;
        found
    }

    pub fn config_dir(&self) -> PathBuf {
        self.lock().config_dir.clone()
    }

    fn forget_token(&self) {
        let dir = self.config_dir();
        storage::delete_token(&dir);
        self.lock().token = None;
    }

    /// Poison-tolerant lock: a panicked worker must not brick the app.
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn settings(&self) -> Settings {
        self.lock().settings.clone()
    }

    pub fn view(&self) -> StatusView {
        let g = self.lock();
        let error = g.core.error().cloned();
        StatusView {
            phase: g.core.phase(),
            job_id: g.core.active_job().map(str::to_string),
            recording_started_ms: g.core.recording_started_ms(),
            max_record_seconds: g.settings.max_record_seconds,
            can_retry: g.core.phase() == Phase::Error
                && error.as_ref().is_some_and(|e| e.retryable)
                && g.core.retained_for().is_some()
                && g.retained.is_some(),
            error,
            notice: g.core.notice().map(str::to_string),
            has_last: g.core.last().is_some(),
            signed_in: g.token.is_some() && !g.settings.service_url.is_empty(),
            service_url: g.settings.service_url.clone(),
            hotkey: g
                .hotkey_label
                .clone()
                .unwrap_or_else(|| g.settings.hotkey.clone()),
            hotkey_error: g.hotkey_error.clone(),
            login: g.login.clone(),
        }
    }

    pub fn last(&self) -> LastView {
        let g = self.lock();
        LastView {
            available: g.core.last().is_some(),
            last: g.core.last().cloned(),
        }
    }

    /// Push state to the webview, tray and (for attention states) a toast.
    pub fn publish(&self) {
        let view = self.view();
        let (lang, notify, prev) = {
            let mut g = self.lock();
            let prev = g.published.replace(view.phase);
            (
                g.settings.ui_language.clone(),
                g.settings.notifications,
                prev,
            )
        };
        let _ = self.app.emit("status", &view);
        tray::update(&self.app, &view, &lang);
        if prev != Some(view.phase) {
            hotkey::on_phase(&self.app, prev, view.phase);
            platform::indicator(view.phase);
            if notify {
                tray::toast(&self.app, &view, &lang);
            }
        }
    }

    // ------------------------------------------------------------ recording

    pub fn toggle(self: &Arc<Self>) {
        let recording = self.lock().core.phase() == Phase::Recording;
        if recording {
            self.stop(None);
        } else {
            self.start();
        }
    }

    pub fn start(self: &Arc<Self>) {
        let (job, cancel_remote, device, max_seconds) = {
            let mut g = self.lock();
            if g.core.phase() == Phase::Recording {
                return;
            }
            if g.token.is_none() || g.settings.service_url.is_empty() {
                g.core.block_start(ErrorInfo {
                    code: "AUTH_REQUIRED".into(),
                    message: "Sign in to start dictating".into(),
                    class: FailureClass::Auth,
                    retryable: false,
                });
                drop(g);
                self.publish();
                return;
            }
            // PRD §7.7: warn before recording, not after the upload is rejected.
            if g.account
                .as_ref()
                .is_some_and(|a| a.usage.limit_seconds > 0 && a.usage.remaining_seconds <= 0)
            {
                g.core.block_start(ErrorInfo {
                    code: "QUOTA_EXCEEDED".into(),
                    message: "Monthly quota used up".into(),
                    class: FailureClass::Quota,
                    retryable: false,
                });
                drop(g);
                self.publish();
                return;
            }
            let job = uuid::Uuid::new_v4().to_string();
            let Ok(ticket) = g.core.start_recording(job.clone(), now_ms()) else {
                return;
            };
            g.retained = None;
            (
                job,
                ticket.cancel_remote,
                g.settings.mic_device.clone(),
                g.settings.max_record_seconds,
            )
        };
        // Show "recording" before the device opens so feedback is immediate.
        self.publish();
        if let Some(old) = cancel_remote {
            self.cancel_remote(old);
        }
        let weak = Arc::downgrade(self);
        let full_job = job.clone();
        let on_full: Box<dyn FnOnce() + Send> = Box::new(move || {
            // Runs on the capture thread: stopping joins that thread, so hop off it.
            std::thread::spawn(move || {
                if let Some(n) = weak.upgrade() {
                    n.stop(Some(full_job));
                }
            });
        });
        let started = audio::Recorder::start(&device, max_seconds, on_full);
        let mut g = self.lock();
        let still_ours = g.core.phase() == Phase::Recording && g.core.active_job() == Some(&job);
        match started {
            Ok(rec) if still_ours => g.recorder = Some(rec),
            Ok(rec) => {
                drop(g);
                drop(rec); // cancelled or stopped while the device was opening
            }
            Err(e) => {
                if still_ours {
                    g.core.failed(&job, local_error(&e));
                }
                drop(g);
                self.publish();
            }
        }
    }

    /// Stop the active recording (optionally only if it is `only`).
    pub fn stop(self: &Arc<Self>, only: Option<String>) {
        // Focus snapshot first: this is the target the user is dictating into.
        let focus_at_stop = focus::snapshot();
        let (job, recorder) = {
            let mut g = self.lock();
            if g.core.phase() != Phase::Recording {
                return;
            }
            let Some(job) = g.core.active_job().map(str::to_string) else {
                return;
            };
            if only.as_ref().is_some_and(|o| *o != job) {
                return;
            }
            (job, g.recorder.take())
        };
        let samples = recorder.map(audio::Recorder::finish).unwrap_or_default();
        let duration = audio_dsp::duration_ms(samples.len(), TARGET_RATE);
        let mut g = self.lock();
        match g.core.stop_recording(&job, duration) {
            StopDecision::Upload { job_id } => match audio_dsp::encode_wav(&samples, TARGET_RATE) {
                Ok(wav) => {
                    g.retained = Some(Retained {
                        job_id: job_id.clone(),
                        wav,
                        duration_ms: duration,
                        focus: focus_at_stop,
                    });
                    drop(g);
                    self.publish();
                    self.spawn_upload(job_id);
                }
                Err(e) => {
                    g.core.failed(&job_id, local_error(&e));
                    drop(g);
                    self.publish();
                }
            },
            StopDecision::TooShort | StopDecision::Ignore => {
                drop(g);
                self.publish();
            }
        }
    }

    pub fn cancel(self: &Arc<Self>) {
        let (effect, recorder) = {
            let mut g = self.lock();
            let effect = g.core.cancel();
            g.retained = None;
            (effect, g.recorder.take())
        };
        drop(recorder); // stops the stream and discards samples
        if let CancelEffect::CancelRemote { job_id } = effect {
            self.cancel_remote(job_id);
        }
        self.publish();
    }

    pub fn discard(self: &Arc<Self>) {
        let (effect, recorder) = {
            let mut g = self.lock();
            let effect = g.core.discard();
            g.retained = None;
            (effect, g.recorder.take())
        };
        drop(recorder);
        if let CancelEffect::CancelRemote { job_id } = effect {
            self.cancel_remote(job_id);
        }
        self.publish();
    }

    pub fn acknowledge(&self) {
        self.lock().core.acknowledge();
        self.publish();
    }

    pub fn retry(self: &Arc<Self>) {
        let job = {
            let mut g = self.lock();
            if g.retained.is_none() {
                return;
            }
            match g.core.retry(uuid::Uuid::new_v4().to_string()) {
                RetryDecision::Upload { job_id } => {
                    if let Some(r) = g.retained.as_mut() {
                        r.job_id = job_id.clone();
                    }
                    job_id
                }
                RetryDecision::Nothing => return,
            }
        };
        self.publish();
        self.spawn_upload(job);
    }

    pub fn copy_last(&self, raw: bool) -> Result<(), String> {
        let text = {
            let g = self.lock();
            let last = g.core.last().ok_or("NO_RESULT")?;
            if raw {
                last.raw.clone()
            } else {
                last.text.clone()
            }
        };
        clipboard::write_text(&text).map(|_| ())
    }

    pub fn clear_last(&self) {
        self.lock().core.clear_last();
        self.publish();
    }
}

// ---------------------------------------------------------------- upload / deliver

impl Native {
    fn client_and_token(&self) -> Result<(ApiClient, String), ErrorInfo> {
        let (url, token) = {
            let g = self.lock();
            (g.settings.service_url.clone(), g.token.clone())
        };
        let token = token.ok_or_else(|| ErrorInfo {
            code: "AUTH_REQUIRED".into(),
            message: "Sign in again".into(),
            class: FailureClass::Auth,
            retryable: true,
        })?;
        let client = ApiClient::new(&url).map_err(|e| local_error(&e))?;
        Ok((client, token))
    }

    fn cancel_remote(&self, job_id: String) {
        let Ok((client, token)) = self.client_and_token() else {
            return;
        };
        tauri::async_runtime::spawn(async move {
            client.cancel_job(&token, &job_id).await;
        });
    }

    fn spawn_upload(self: &Arc<Self>, job_id: String) {
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            this.upload(job_id).await;
        });
    }

    async fn upload(self: Arc<Self>, job_id: String) {
        let request = {
            let g = self.lock();
            let Some(r) = g.retained.as_ref().filter(|r| r.job_id == job_id) else {
                return;
            };
            DictationRequest {
                job_id: job_id.clone(),
                wav: r.wav.clone(),
                duration_ms: r.duration_ms,
                language: g.settings.language.clone(),
                grammar_mode: g.settings.mode.clone(),
                output_script: g.settings.output_script.clone(),
                dictation_commands: g.settings.dictation_commands,
            }
        };
        let (client, token) = match self.client_and_token() {
            Ok(v) => v,
            Err(e) => {
                self.lock().core.failed(&job_id, e);
                self.publish();
                return;
            }
        };
        let check = {
            let this = self.clone();
            let id = job_id.clone();
            move || this.lock().core.is_current(&id)
        };
        match client.dictate(&token, &request, check).await {
            Ok(resp) => self.on_result(&job_id, resp),
            Err(CallError::Cancelled) => {}
            Err(CallError::Http(err)) => {
                let auth = err.class == FailureClass::Auth;
                let quota = err.class == FailureClass::Quota;
                self.lock().core.failed(&job_id, err);
                if auth {
                    // Token rejected: forget it so the UI offers sign-in again.
                    self.forget_token();
                }
                self.publish();
                if quota {
                    self.spawn_refresh_account();
                }
            }
        }
    }

    fn on_result(self: &Arc<Self>, job_id: &str, resp: DictationResponse) {
        let text = resp.final_text();
        let raw = resp.raw_text.clone().unwrap_or_default();
        let noise = resp.is_suspected_noise();
        let (action, focus_at_stop, settings) = {
            let mut g = self.lock();
            let action = g
                .core
                .result_arrived(job_id, text, raw, resp.warnings.clone(), noise);
            if let (Some(b), Some(acc)) = (resp.billing.as_ref(), g.account.as_mut()) {
                if let Some(rem) = b.remaining_seconds {
                    acc.usage.remaining_seconds = rem;
                }
                if b.resets_at.is_some() {
                    acc.usage.resets_at = b.resets_at.clone();
                }
            }
            let focus = if matches!(action, ResultAction::Ignore) {
                None
            } else {
                g.retained.take().and_then(|r| r.focus)
            };
            (action, focus, g.settings.clone())
        };
        match action {
            ResultAction::Ignore => return,
            ResultAction::KeepOnly => {
                self.publish();
                return;
            }
            ResultAction::Deliver { text } => {
                let this = self.clone();
                let job = job_id.to_string();
                // Clipboard waits and key injection must not block the async runtime.
                std::thread::spawn(move || {
                    let is_current = || this.lock().core.is_current(&job);
                    let (outcome, notice) = platform::deliver::deliver(
                        &text,
                        focus_at_stop.as_ref(),
                        &settings,
                        &is_current,
                    );
                    {
                        let mut g = this.lock();
                        if g.core.delivered(&job, outcome) {
                            g.core.set_notice(notice);
                        }
                    }
                    this.publish();
                });
            }
        }
        self.spawn_refresh_account();
    }
}

// ---------------------------------------------------------------- account

fn err_text(e: ErrorInfo) -> String {
    format!("{}: {}", e.code, e.message)
}

impl Native {
    pub fn spawn_refresh_account(self: &Arc<Self>) {
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            let _ = this.refresh_account().await;
        });
    }

    pub async fn refresh_account(self: &Arc<Self>) -> Result<Me, String> {
        let (client, token) = self.client_and_token().map_err(err_text)?;
        match client.me(&token).await {
            Ok(me) => {
                self.lock().account = Some(me.clone());
                let _ = self.app.emit("account", &me);
                Ok(me)
            }
            Err(e) => {
                if e.class == FailureClass::Auth {
                    self.forget_token();
                    self.publish();
                }
                Err(err_text(e))
            }
        }
    }

    pub fn cached_account(&self) -> Option<Me> {
        self.lock().account.clone()
    }

    /// Device login (§8.1). Opens the browser; the token goes straight into
    /// the OS secret store and is never sent to the webview.
    pub async fn login(self: &Arc<Self>, base_url: String) -> Result<LoginView, String> {
        let base = api::validate_base_url(&base_url)?;
        let client = ApiClient::new(&base)?;
        let start = client.device_start().await?;
        let open = start
            .verification_uri_complete
            .clone()
            .unwrap_or_else(|| start.verification_uri.clone());
        if !api::is_safe_verification_url(&open) {
            return Err("INVALID_RESPONSE: unsafe login URL".into());
        }
        let view = LoginView {
            user_code: start.user_code.clone(),
            verification_uri: open.clone(),
            expires_at_ms: now_ms() + start.expires_in.min(3600) * 1000,
            error: None,
        };
        let gen = {
            let mut g = self.lock();
            g.login_gen += 1;
            g.login = Some(view.clone());
            g.login_gen
        };
        self.publish();
        {
            use tauri_plugin_opener::OpenerExt;
            let _ = self.app.opener().open_url(open, None::<&str>);
        }
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            this.poll_login(
                client,
                base,
                start.device_code,
                start.interval,
                start.expires_in,
                gen,
            )
            .await;
        });
        Ok(view)
    }

    async fn poll_login(
        self: Arc<Self>,
        client: ApiClient,
        base: String,
        device_code: String,
        interval: u64,
        expires_in: u64,
        gen: u64,
    ) {
        let mut interval = interval.clamp(1, 60);
        let deadline = std::time::Instant::now() + Duration::from_secs(expires_in.clamp(30, 3600));
        let finish = |this: &Arc<Self>, error: Option<String>| {
            {
                let mut g = this.lock();
                if g.login_gen != gen {
                    return;
                }
                match error {
                    Some(e) => {
                        if let Some(l) = g.login.as_mut() {
                            l.error = Some(e);
                        }
                    }
                    None => g.login = None,
                }
            }
            this.publish();
        };
        loop {
            tokio::time::sleep(Duration::from_secs(interval)).await;
            if self.lock().login_gen != gen {
                return; // cancelled or restarted
            }
            if std::time::Instant::now() > deadline {
                return finish(&self, Some("EXPIRED_TOKEN".into()));
            }
            match client.device_poll(&device_code).await {
                api::PollStep::Pending => {}
                api::PollStep::SlowDown => interval = (interval + 5).min(60),
                api::PollStep::Expired => return finish(&self, Some("EXPIRED_TOKEN".into())),
                api::PollStep::Failed(code) => return finish(&self, Some(code)),
                api::PollStep::Token(t) => {
                    // The Secret Service may block on an unlock prompt: off the runtime.
                    let (dir, token) = (self.config_dir(), t.access_token.clone());
                    let stored = tauri::async_runtime::spawn_blocking(move || {
                        storage::store_token(&dir, &token)
                    })
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|r| r);
                    if let Err(e) = stored {
                        return finish(&self, Some(e));
                    }
                    let saved = {
                        let mut g = self.lock();
                        if g.login_gen != gen {
                            return;
                        }
                        g.token = Some(t.access_token);
                        g.settings.service_url = base.clone();
                        // Clear a "sign in first" block, but keep a failed upload
                        // (audio retained) so the user can press Retry now.
                        if g.retained.is_none()
                            && g.core
                                .error()
                                .is_some_and(|e| e.class == FailureClass::Auth)
                        {
                            g.core.acknowledge();
                        }
                        storage::save_settings(&g.config_dir, &g.settings)
                    };
                    finish(&self, saved.err());
                    self.spawn_refresh_account();
                    return;
                }
            }
        }
    }

    pub fn cancel_login(&self) {
        {
            let mut g = self.lock();
            g.login_gen += 1;
            g.login = None;
        }
        self.publish();
    }

    pub fn logout(&self) {
        self.forget_token();
        self.lock().account = None;
        self.publish();
    }

    pub async fn open_billing(self: &Arc<Self>, portal: bool, plan: String) -> Result<(), String> {
        if !portal && plan != "monthly" && plan != "annual" {
            return Err("Unsupported plan".into());
        }
        let (client, token) = self.client_and_token().map_err(err_text)?;
        let url = if portal {
            client.portal(&token).await
        } else {
            client.checkout(&token, &plan).await
        }
        .map_err(err_text)?;
        if !api::is_trusted_billing_url(&url) {
            return Err("UNTRUSTED_BILLING_URL: refusing to open a non-Stripe page".into());
        }
        use tauri_plugin_opener::OpenerExt;
        self.app
            .opener()
            .open_url(url, None::<&str>)
            .map_err(|e| e.to_string())
    }

    pub async fn dictionary(self: &Arc<Self>) -> Result<api::Dictionary, String> {
        let (client, token) = self.client_and_token().map_err(err_text)?;
        client.dictionary(&token).await.map_err(err_text)
    }

    pub async fn set_dictionary(
        self: &Arc<Self>,
        terms: Vec<String>,
    ) -> Result<api::Dictionary, String> {
        let terms = api::clean_terms(&terms, 200, 64)?;
        let (client, token) = self.client_and_token().map_err(err_text)?;
        client
            .put_dictionary(&token, &terms)
            .await
            .map_err(err_text)
    }

    /// `DELETE /v1/me/data` (confirmation enforced by the command layer),
    /// then local sign-out and clearing of the last result.
    pub async fn delete_data(self: &Arc<Self>) -> Result<(), String> {
        let (client, token) = self.client_and_token().map_err(err_text)?;
        client.delete_data(&token).await.map_err(err_text)?;
        self.forget_token();
        {
            let mut g = self.lock();
            g.account = None;
            g.core.clear_last();
        }
        self.publish();
        Ok(())
    }

    // ------------------------------------------------------------ settings

    pub fn set_setting(self: &Arc<Self>, key: &str, value: &str) -> Result<Settings, String> {
        let (settings, old_hotkey) = {
            let mut g = self.lock();
            let old = g.settings.hotkey.clone();
            let mut next = g.settings.clone();
            next.apply(key, value)?;
            storage::save_settings(&g.config_dir, &next)?;
            g.settings = next.clone();
            (next, old)
        };
        #[cfg(windows)]
        if key == "hotkey" && settings.hotkey != old_hotkey {
            if let Err(e) = hotkey::rebind(self, &old_hotkey, &settings.hotkey) {
                // Roll back to the previous working binding.
                let mut g = self.lock();
                g.settings.hotkey = old_hotkey;
                let _ = storage::save_settings(&g.config_dir, &g.settings);
                drop(g);
                self.publish();
                return Err(e);
            }
        }
        #[cfg(not(windows))]
        let _ = old_hotkey;
        if key == "ui_language" {
            tray::rebuild_menu(&self.app, &settings.ui_language);
        }
        platform::setting_changed(&self.config_dir(), key, &settings);
        self.publish();
        Ok(settings)
    }

    #[cfg(windows)]
    pub fn set_hotkey_error(&self, error: Option<String>) {
        self.lock().hotkey_error = error;
    }

    #[cfg(not(windows))]
    pub fn set_hotkey_label(&self, label: Option<String>) {
        self.lock().hotkey_label = label;
    }

    pub fn is_recording(&self) -> bool {
        self.lock().core.phase() == Phase::Recording
    }

    /// Short local microphone test; never uploaded or stored.
    pub fn mic_test(&self, millis: u64) -> Result<(f32, f32), String> {
        if self.is_recording() {
            return Err("Recording in progress".into());
        }
        audio::mic_test(&self.settings().mic_device, millis)
    }
}

/// The managed orchestrator.
pub fn native(app: &AppHandle) -> Arc<Native> {
    app.state::<Arc<Native>>().inner().clone()
}

/// Command-line actions, from the first launch or forwarded by a second one
/// (`scribiva --toggle` from the GNOME shortcut, `--cancel`, `--hidden`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchAction {
    Toggle,
    Cancel,
    /// Start in the tray (autostart); nothing to do for a running instance.
    Hidden,
    /// Plain launch: show the window.
    Show,
}

pub fn launch_action<S: AsRef<str>>(argv: &[S]) -> LaunchAction {
    // argv[0] is the program; later flags win over earlier ones.
    let mut action = LaunchAction::Show;
    for a in argv.iter().skip(1) {
        match a.as_ref() {
            "--toggle" => action = LaunchAction::Toggle,
            "--cancel" => action = LaunchAction::Cancel,
            "--hidden" if action == LaunchAction::Show => action = LaunchAction::Hidden,
            _ => {}
        }
    }
    action
}

/// Run a launch action against the running instance. Recording work runs on
/// its own thread (callers may be on the event loop or a D-Bus thread).
pub fn run_launch_action(app: &AppHandle, action: LaunchAction) {
    match action {
        LaunchAction::Toggle | LaunchAction::Cancel => {
            let Some(n) = app.try_state::<Arc<Native>>().map(|s| s.inner().clone()) else {
                return;
            };
            std::thread::spawn(move || {
                if action == LaunchAction::Toggle {
                    n.toggle();
                } else {
                    n.cancel();
                }
            });
        }
        LaunchAction::Show => tray::show_main(app),
        LaunchAction::Hidden => {}
    }
}

/// Hide-to-tray on close; shared by both platforms.
pub fn hide_on_close(window: &tauri::WebviewWindow) {
    let w = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = w.hide();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_actions() {
        assert_eq!(launch_action(&["scribiva"]), LaunchAction::Show);
        assert_eq!(
            launch_action(&["scribiva", "--toggle"]),
            LaunchAction::Toggle
        );
        assert_eq!(
            launch_action(&["/usr/bin/scribiva", "--cancel"]),
            LaunchAction::Cancel
        );
        assert_eq!(
            launch_action(&["scribiva", "--hidden"]),
            LaunchAction::Hidden
        );
        assert_eq!(
            launch_action(&["scribiva", "--hidden", "--toggle"]),
            LaunchAction::Toggle
        );
        assert_eq!(
            launch_action(&["scribiva", "--toggle", "--hidden"]),
            LaunchAction::Toggle
        );
        // argv[0] is never interpreted, unknown flags are ignored.
        assert_eq!(launch_action(&["--toggle"]), LaunchAction::Show);
        assert_eq!(launch_action(&["scribiva", "--evil"]), LaunchAction::Show);
    }

    #[test]
    fn error_codes_are_split() {
        let e = local_error("MIC_BUSY: device in use");
        assert_eq!(
            (e.code.as_str(), e.message.as_str()),
            ("MIC_BUSY", "device in use")
        );
        let e = local_error("plain failure: details");
        assert_eq!(e.code, "ERROR");
    }
}
