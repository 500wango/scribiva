//! Child-process helpers for the Linux adapters. Programs are resolved on
//! `PATH` (or given as absolute paths) and run with an argv array; text is
//! passed through stdin or argv, never through a shell. Every call has a
//! timeout and a capped output size.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Upper bound for captured stdout (clipboard text, pw-dump JSON).
const MAX_OUTPUT: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

impl Output {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).trim().to_string()
    }
}

/// Executable `name` on `PATH`, or `name` itself when it is an absolute path.
pub fn which(name: &str) -> Option<PathBuf> {
    let candidate = Path::new(name);
    if candidate.is_absolute() {
        return is_executable(candidate).then(|| candidate.to_path_buf());
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|p| is_executable(p))
}

pub fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

fn read_capped(mut r: impl Read, cap: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = [0u8; 16 * 1024];
    loop {
        match r.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if out.len() < cap {
                    let take = n.min(cap - out.len());
                    out.extend_from_slice(&buf[..take]);
                }
                // Keep draining past the cap so the child never blocks on a full pipe.
            }
        }
    }
    out
}

/// Run `program args...`, optionally feeding `stdin`, and wait at most
/// `timeout`. On timeout the child is killed and an error returned.
///
/// `capture` = false sends stdout/stderr to /dev/null. That is required for
/// tools that fork a background server inheriting the pipes (wl-copy, xclip);
/// otherwise reading the pipe would wait for that server to exit.
pub fn run(
    program: &str,
    args: &[&str],
    stdin: Option<&[u8]>,
    timeout: Duration,
    capture: bool,
) -> Result<Output, String> {
    let exe = which(program).ok_or_else(|| format!("{program} not found"))?;
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(if capture {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(if capture {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("{program} failed to start: {e}"))?;
    if let (Some(data), Some(mut pipe)) = (stdin, child.stdin.take()) {
        let data = data.to_vec();
        // Separate thread: a large write must not deadlock against a full stdout pipe.
        std::thread::spawn(move || {
            let _ = pipe.write_all(&data);
        });
    }
    let out_reader = child
        .stdout
        .take()
        .map(|o| std::thread::spawn(move || read_capped(o, MAX_OUTPUT)));
    let err_reader = child
        .stderr
        .take()
        .map(|e| std::thread::spawn(move || read_capped(e, 4096)));
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{program} timed out"));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => return Err(format!("{program}: {e}")),
        }
    };
    let stdout = out_reader.and_then(|h| h.join().ok()).unwrap_or_default();
    let stderr = err_reader
        .and_then(|h| h.join().ok())
        .map(|b| String::from_utf8_lossy(&b).trim().to_string())
        .unwrap_or_default();
    Ok(Output {
        success: status.success(),
        stdout,
        stderr,
    })
}

/// Run and require success; returns trimmed stdout.
pub fn run_ok(program: &str, args: &[&str], timeout: Duration) -> Result<String, String> {
    let out = run(program, args, None, timeout, true)?;
    if out.success {
        Ok(out.text())
    } else if out.stderr.is_empty() {
        Err(format!("{program} failed"))
    } else {
        Err(format!(
            "{program}: {}",
            out.stderr.chars().take(200).collect::<String>()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_with_stdin_and_argv_only() {
        // `cat` echoes stdin; shell metacharacters stay inert data.
        let out = run(
            "cat",
            &[],
            Some(b"a; rm -rf / $(x)"),
            Duration::from_secs(5),
            true,
        )
        .unwrap();
        assert!(out.success);
        assert_eq!(out.stdout, b"a; rm -rf / $(x)");
    }

    #[test]
    fn times_out_and_kills() {
        let err = run("sleep", &["5"], None, Duration::from_millis(100), true).unwrap_err();
        assert!(err.contains("timed out"));
    }

    #[test]
    fn missing_program() {
        assert!(run_ok("scribiva-no-such-tool", &[], Duration::from_secs(1)).is_err());
        assert!(which("scribiva-no-such-tool").is_none());
        assert!(which("sh").is_some());
    }
}
