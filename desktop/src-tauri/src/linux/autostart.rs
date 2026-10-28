//! XDG autostart entry (`~/.config/autostart/scribiva.desktop`) that starts
//! Scribiva in the tray at login. Written only after onboarding, removed when
//! `start_at_login` is off.

use crate::storage::Settings;
use std::path::{Path, PathBuf};

const FILE: &str = "scribiva.desktop";

fn autostart_dir() -> Option<PathBuf> {
    if let Some(x) = std::env::var_os("XDG_CONFIG_HOME").filter(|x| !x.is_empty()) {
        return Some(PathBuf::from(x).join("autostart"));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/autostart"))
}

/// Desktop Entry `Exec` value: reserved characters quoted per the spec.
fn exec_word(path: &str) -> String {
    if path
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-+".contains(c))
    {
        return path.to_string();
    }
    let mut out = String::from("\"");
    for c in path.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

pub fn entry(exe: &Path) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=Scribiva\nComment=Voice typing\nExec={} --hidden\nIcon=scribiva\nTerminal=false\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n",
        exec_word(&exe.to_string_lossy())
    )
}

pub fn wanted(settings: &Settings) -> bool {
    settings.onboarding_done && settings.start_at_login
}

/// Write or remove the entry in `dir`. Only rewrites when the content changed.
pub fn apply(dir: &Path, want: bool, exe: &Path) -> Result<(), String> {
    let path = dir.join(FILE);
    if !want {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        };
    }
    let body = entry(exe);
    if std::fs::read_to_string(&path).ok().as_deref() == Some(body.as_str()) {
        return Ok(());
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("desktop.tmp");
    std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

pub fn sync(settings: &Settings) {
    if let Some(dir) = autostart_dir() {
        let _ = apply(&dir, wanted(settings), &super::setup::app_exe());
    }
}

pub fn installed() -> bool {
    autostart_dir().is_some_and(|d| d.join(FILE).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_and_apply() {
        assert!(
            entry(Path::new("/usr/bin/scribiva")).contains("\nExec=/usr/bin/scribiva --hidden\n")
        );
        assert!(entry(Path::new("/opt/my app/scribiva"))
            .contains("Exec=\"/opt/my app/scribiva\" --hidden"));
        assert!(entry(Path::new("/x/$y")).contains("Exec=\"/x/\\$y\" --hidden"));

        let mut s = Settings::default();
        assert!(!wanted(&s)); // before onboarding
        s.onboarding_done = true;
        assert!(wanted(&s));
        s.start_at_login = false;
        assert!(!wanted(&s));

        let dir = std::env::temp_dir().join(format!("scribiva-autostart-{}", std::process::id()));
        let exe = Path::new("/usr/bin/scribiva");
        apply(&dir, true, exe).unwrap();
        let body = std::fs::read_to_string(dir.join(FILE)).unwrap();
        assert!(body.starts_with("[Desktop Entry]\n") && body.contains("--hidden"));
        apply(&dir, true, exe).unwrap();
        apply(&dir, false, exe).unwrap();
        assert!(!dir.join(FILE).exists());
        apply(&dir, false, exe).unwrap(); // removing twice is fine
        let _ = std::fs::remove_dir_all(&dir);
    }
}
