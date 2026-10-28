//! Signed auto-update (FR-012) through `tauri-plugin-updater`.
//!
//! - Checks at startup and every 24 h while `auto_update_check` is on.
//! - Disabled entirely while `plugins.updater.pubkey` in `tauri.conf.json`
//!   is still the placeholder: without a real minisign key nothing could be
//!   verified, and the plugin refuses unsigned packages anyway.
//! - The package is downloaded (and its signature verified by the plugin)
//!   in the background, but only installed when the user presses "Restart
//!   to install", and never while recording, uploading or inserting.
//! - Linux `.deb` installs live under `/usr`: the updater cannot replace
//!   system files without root (it would prompt for a password), so there
//!   Scribiva only says a new version exists and links the release page.
//!
//! The webview has no access to the plugin's own commands (no `updater:*`
//! capability); everything goes through Scribiva's commands below.

use crate::native::Native;
use serde::Serialize;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::AppHandle;
use tauri_plugin_updater::{Update, UpdaterExt};

pub const PUBKEY_PLACEHOLDER: &str = "REPLACE_WITH_MINISIGN_PUBLIC_KEY";
/// Opened for installs the updater cannot replace (Linux system packages).
pub const RELEASE_PAGE: &str = "https://github.com/500wango/scribiva/releases/latest";
const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
/// First check shortly after startup, so it does not compete with launch.
const FIRST_CHECK_AFTER: Duration = Duration::from_secs(20);
const BUSY_RETRY: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateState {
    /// Downloading and verifying the signed package.
    Downloading,
    /// Verified and ready: "restart to install".
    Ready,
    /// The updater cannot replace this install: download from the website.
    Manual,
    /// Download or verification failed; retried at the next check.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpdateView {
    pub version: String,
    pub state: UpdateState,
    /// Release page for `Manual` (fixed constant, never from the network).
    pub url: Option<String>,
    pub error: Option<String>,
}

/// Downloaded, signature-verified package waiting for the user.
struct Pending {
    update: Update,
    bytes: Vec<u8>,
}

static PENDING: Mutex<Option<Pending>> = Mutex::new(None);
static RUNNING: Mutex<bool> = Mutex::new(false);

pub fn is_placeholder(pubkey: &str) -> bool {
    let k = pubkey.trim();
    k.is_empty() || k == PUBKEY_PLACEHOLDER
}

/// Installed as a system package (Linux `.deb` puts the binary in /usr):
/// self-update would need root, so the user updates from the website.
pub fn is_system_install(exe: &Path) -> bool {
    cfg!(target_os = "linux") && exe.starts_with("/usr")
}

fn configured_pubkey(app: &AppHandle) -> String {
    app.config()
        .plugins
        .0
        .get("updater")
        .and_then(|u| u.get("pubkey"))
        .and_then(|k| k.as_str())
        .unwrap_or_default()
        .to_string()
}

/// Whether update checks can run at all in this build.
pub fn enabled(app: &AppHandle) -> bool {
    !is_placeholder(&configured_pubkey(app))
}

/// Start the startup + 24 h check loop (no-op without a real public key).
pub fn start(native: &Arc<Native>) {
    if !enabled(&native.app) {
        return;
    }
    let n = native.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK_AFTER).await;
        loop {
            if n.settings().auto_update_check {
                check(&n).await;
            }
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

/// Check now (setting turned on, or "check for updates" in the UI).
pub fn check_soon(native: &Arc<Native>) {
    if !enabled(&native.app) {
        return;
    }
    let n = native.clone();
    tauri::async_runtime::spawn(async move {
        check(&n).await;
    });
}

async fn check(native: &Arc<Native>) {
    {
        let mut running = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
        if *running {
            return;
        }
        *running = true;
    }
    run_check(native).await;
    *RUNNING.lock().unwrap_or_else(|e| e.into_inner()) = false;
}

async fn run_check(native: &Arc<Native>) {
    // Already downloaded and waiting for a restart.
    if matches!(
        native.update_state(),
        Some(UpdateView {
            state: UpdateState::Ready,
            ..
        })
    ) {
        return;
    }
    let Ok(updater) = native.app.updater() else {
        return;
    };
    let update = match updater.check().await {
        Ok(Some(u)) => u,
        // Up to date, or offline / endpoint down: try again next time.
        Ok(None) | Err(_) => return,
    };
    let version = update.version.clone();
    let manual = std::env::current_exe()
        .map(|p| is_system_install(&p))
        .unwrap_or(false);
    if manual {
        native.set_update(Some(UpdateView {
            version,
            state: UpdateState::Manual,
            url: Some(RELEASE_PAGE.to_string()),
            error: None,
        }));
        return;
    }
    // Do not compete with a dictation upload for bandwidth.
    while native.is_busy() {
        tokio::time::sleep(BUSY_RETRY).await;
    }
    native.set_update(Some(UpdateView {
        version: version.clone(),
        state: UpdateState::Downloading,
        url: None,
        error: None,
    }));
    // `download` verifies the minisign signature against the configured key.
    match update.download(|_, _| {}, || {}).await {
        Ok(bytes) => {
            *PENDING.lock().unwrap_or_else(|e| e.into_inner()) = Some(Pending { update, bytes });
            native.set_update(Some(UpdateView {
                version,
                state: UpdateState::Ready,
                url: None,
                error: None,
            }));
        }
        Err(e) => native.set_update(Some(UpdateView {
            version,
            state: UpdateState::Failed,
            url: None,
            error: Some(e.to_string()),
        })),
    }
}

/// "Restart to install". Refused while recording, uploading or inserting.
pub fn install(native: &Arc<Native>) -> Result<(), String> {
    if native.is_busy() {
        return Err("UPDATE_BUSY: finish or cancel the dictation first".into());
    }
    let pending = PENDING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
        .ok_or("NO_UPDATE: no downloaded update")?;
    // Verifies the signature again, then installs. On Windows the NSIS
    // installer is launched and the app exits; it restarts afterwards.
    if let Err(e) = pending.update.install(&pending.bytes) {
        native.set_update(Some(UpdateView {
            version: pending.update.version.clone(),
            state: UpdateState::Failed,
            url: None,
            error: Some(e.to_string()),
        }));
        return Err(format!("UPDATE_FAILED: {e}"));
    }
    native.app.restart();
}

/// Open the fixed release page (system installs).
pub fn open_release_page(app: &AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(RELEASE_PAGE, None::<&str>)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_key_disables_updates() {
        assert!(is_placeholder(PUBKEY_PLACEHOLDER));
        assert!(is_placeholder("  "));
        assert!(is_placeholder(""));
        assert!(!is_placeholder(
            "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk="
        ));
    }

    #[test]
    fn system_installs_are_manual() {
        let usr = is_system_install(Path::new("/usr/bin/scribiva"));
        assert_eq!(usr, cfg!(target_os = "linux"));
        assert!(!is_system_install(Path::new(
            "/home/u/Apps/Scribiva.AppImage"
        )));
        assert!(!is_system_install(Path::new(
            "C:\\Users\\u\\AppData\\Local\\Scribiva\\scribiva.exe"
        )));
    }

    #[test]
    fn release_page_is_https() {
        assert!(RELEASE_PAGE.starts_with("https://"));
    }

    #[test]
    fn view_serialises_state() {
        let v = UpdateView {
            version: "1.0.1".into(),
            state: UpdateState::Ready,
            url: None,
            error: None,
        };
        let j = serde_json::to_value(&v).unwrap();
        assert_eq!(j["state"], "ready");
        assert_eq!(j["version"], "1.0.1");
    }
}
