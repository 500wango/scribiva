//! Microphone capture through PipeWire's `pw-record`, which writes headerless
//! 16 kHz mono s16le PCM to stdout. A reader thread turns the bytes into
//! samples. The child exists only while recording: stop sends SIGTERM and
//! drains the pipe, and `Drop` kills it, so no recorder outlives its job.
//! Devices come from one read-only `pw-dump` call (stable `node.name` ids).

use crate::audio_dsp::TARGET_RATE;
use crate::native::{DeviceList, InputDevice};
use serde_json::Value;
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------- PCM

/// Append little-endian s16 samples from `bytes`, carrying an odd trailing
/// byte to the next chunk. Stops storing at `max_samples`; returns false once
/// the limit is reached (the rest is discarded).
pub fn push_pcm(
    carry: &mut Option<u8>,
    bytes: &[u8],
    out: &mut Vec<i16>,
    max_samples: usize,
) -> bool {
    let mut i = 0;
    if let Some(lo) = carry.take() {
        match bytes.first() {
            Some(&hi) => {
                if out.len() < max_samples {
                    out.push(i16::from_le_bytes([lo, hi]));
                }
                i = 1;
            }
            None => {
                *carry = Some(lo);
                return out.len() < max_samples;
            }
        }
    }
    let rest = &bytes[i..];
    let pairs = rest.chunks_exact(2);
    if let [last] = pairs.remainder() {
        *carry = Some(*last);
    }
    for pair in pairs {
        if out.len() >= max_samples {
            break;
        }
        out.push(i16::from_le_bytes([pair[0], pair[1]]));
    }
    out.len() < max_samples
}

/// A PipeWire node name is passed as a single `--target=` argument; refuse
/// anything that is not a plain identifier-like string.
pub fn valid_node_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 256
        && !name.starts_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-:@+".contains(c))
}

pub fn record_args(target: Option<&str>) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(t) = target {
        args.push(format!("--target={t}"));
    }
    for a in ["--rate", "16000", "--channels", "1", "--format", "s16", "-"] {
        args.push(a.to_string());
    }
    args
}

// ---------------------------------------------------------------- devices

fn text(v: Option<&Value>, limit: usize) -> String {
    v.and_then(Value::as_str)
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .map(|s| s.chars().take(limit).collect())
        .unwrap_or_default()
}

fn truthy(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => {
            matches!(s.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes")
        }
        _ => false,
    }
}

fn int(v: Option<&Value>) -> i64 {
    match v {
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0),
        Some(Value::String(s)) => s.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

/// `default.audio.source` value: `{"name": "..."}` as an object or a JSON string.
fn default_name(v: Option<&Value>) -> Option<String> {
    let name = match v? {
        Value::Object(o) => o.get("name").cloned(),
        Value::String(s) => match serde_json::from_str::<Value>(s) {
            Ok(Value::Object(o)) => o.get("name").cloned(),
            Ok(_) => None,
            Err(_) => Some(Value::String(s.clone())),
        },
        _ => None,
    };
    Some(text(name.as_ref(), 200)).filter(|s| !s.is_empty())
}

struct Candidate {
    device: InputDevice,
    priority: i64,
    id: i64,
}

/// Lower is better: physical before virtual, high session priority, low id.
fn rank(c: &Candidate) -> (bool, i64, i64) {
    (c.device.is_virtual, -c.priority, c.id)
}

/// Port of `devices.parse_pw_dump`: Audio/Source(/Virtual) nodes plus the
/// default source from the `default` metadata object.
pub fn parse_pw_dump(json: &str) -> Result<(Vec<InputDevice>, Option<String>), String> {
    let data: Value =
        serde_json::from_str(json).map_err(|e| format!("Cannot parse pw-dump output: {e}"))?;
    let list = data.as_array().ok_or("pw-dump returned unexpected JSON")?;
    let mut sources: Vec<Candidate> = Vec::new();
    let mut default: Option<String> = None;
    for obj in list {
        let kind = obj.get("type").and_then(Value::as_str).unwrap_or("");
        if kind == "PipeWire:Interface:Node" {
            let Some(props) = obj
                .get("info")
                .and_then(|i| i.get("props"))
                .filter(|p| p.is_object())
            else {
                continue;
            };
            let class = props
                .get("media.class")
                .and_then(Value::as_str)
                .unwrap_or("");
            if class != "Audio/Source" && class != "Audio/Source/Virtual" {
                continue;
            }
            let node = text(props.get("node.name"), 200);
            if node.is_empty() {
                continue;
            }
            let label = ["node.description", "node.nick", "device.description"]
                .iter()
                .map(|k| text(props.get(*k), 160))
                .find(|s| !s.is_empty())
                .unwrap_or_else(|| node.clone());
            let candidate = Candidate {
                device: InputDevice {
                    id: node.clone(),
                    name: label,
                    default: false,
                    is_virtual: class == "Audio/Source/Virtual"
                        || truthy(props.get("node.virtual")),
                },
                priority: int(props.get("priority.session")),
                id: obj.get("id").and_then(Value::as_i64).unwrap_or(1 << 31),
            };
            match sources.iter_mut().find(|c| c.device.id == node) {
                Some(existing) if rank(&candidate) < rank(existing) => *existing = candidate,
                Some(_) => {}
                None => sources.push(candidate),
            }
        } else if kind == "PipeWire:Interface:Metadata" && default.is_none() {
            let is_default = obj
                .get("props")
                .and_then(|p| p.get("metadata.name"))
                .and_then(Value::as_str)
                == Some("default");
            if !is_default {
                continue;
            }
            for item in obj
                .get("metadata")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if item.get("key").and_then(Value::as_str) == Some("default.audio.source") {
                    default = default_name(item.get("value")).or(default);
                }
            }
        }
    }
    sources.sort_by(|a, b| {
        let key = |c: &Candidate| {
            (
                Some(&c.device.id) != default.as_ref(),
                c.device.is_virtual,
                -c.priority,
                c.device.name.to_lowercase(),
                c.device.id.clone(),
            )
        };
        key(a).cmp(&key(b))
    });
    let devices = sources
        .into_iter()
        .map(|mut c| {
            c.device.default = Some(&c.device.id) == default.as_ref();
            c.device
        })
        .collect();
    Ok((devices, default))
}

fn pw_dump() -> Result<(Vec<InputDevice>, Option<String>), String> {
    let out = super::proc::run("pw-dump", &[], None, Duration::from_secs(5), true)
        .map_err(|e| format!("MIC_ENUMERATION_FAILED: {e} (install pipewire-bin)"))?;
    if !out.success {
        return Err(format!(
            "MIC_ENUMERATION_FAILED: pw-dump failed: {}",
            out.stderr.chars().take(160).collect::<String>()
        ));
    }
    parse_pw_dump(&String::from_utf8_lossy(&out.stdout))
}

/// Node that a recording with `configured` will use: the configured node if
/// it is present right now, otherwise the system default.
pub fn active_node(
    devices: &[InputDevice],
    default: Option<&str>,
    configured: &str,
) -> Option<String> {
    if !configured.is_empty() && devices.iter().any(|d| d.id == configured) {
        return Some(configured.to_string());
    }
    default.map(str::to_string)
}

pub fn device_list(configured: &str) -> DeviceList {
    match pw_dump() {
        Ok((devices, default)) => DeviceList {
            active: active_node(&devices, default.as_deref(), configured),
            devices,
            error: None,
        },
        Err(e) => DeviceList {
            devices: Vec::new(),
            active: None,
            error: Some(e),
        },
    }
}

/// `--target` for `configured`, or None (system default) when it is unset,
/// unsafe or not present right now.
fn target_for(configured: &str) -> Option<String> {
    if configured.is_empty() || !valid_node_name(configured) {
        return None;
    }
    match pw_dump() {
        Ok((devices, _)) if devices.iter().any(|d| d.id == configured) => {
            Some(configured.to_string())
        }
        Ok(_) => None,
        // Enumeration failed: still try the node the user picked.
        Err(_) => Some(configured.to_string()),
    }
}

// ---------------------------------------------------------------- recorder

pub struct Recorder {
    child: Option<Child>,
    reader: Option<JoinHandle<Vec<i16>>>,
    stderr: Arc<Mutex<String>>,
}

fn sigterm(child: &Child) {
    if let Ok(pid) = i32::try_from(child.id()) {
        // SAFETY: plain kill(2) on our own child's pid; the child has not been reaped yet.
        unsafe { libc::kill(pid, libc::SIGTERM) };
    }
}

impl Recorder {
    /// Start `pw-record`. `on_full` runs once (on the reader thread) when
    /// `max_seconds` is reached; it must not block on this recorder.
    pub fn start(
        device_id: &str,
        max_seconds: u32,
        on_full: Box<dyn FnOnce() + Send>,
    ) -> Result<Self, String> {
        let exe = super::proc::which("pw-record").ok_or_else(|| {
            "MIC_NOT_FOUND: pw-record is missing (install pipewire-bin)".to_string()
        })?;
        let target = target_for(device_id);
        let mut child = Command::new(exe)
            .args(record_args(target.as_deref()))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("MIC_ERROR: pw-record failed to start: {e}"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or("MIC_ERROR: no pw-record output")?;
        let stderr = Arc::new(Mutex::new(String::new()));
        if let Some(mut err) = child.stderr.take() {
            let sink = stderr.clone();
            std::thread::spawn(move || {
                let mut buf = Vec::new();
                let _ = (&mut err).take(4096).read_to_end(&mut buf);
                if let Ok(mut s) = sink.lock() {
                    *s = String::from_utf8_lossy(&buf)
                        .trim()
                        .chars()
                        .take(200)
                        .collect();
                }
                // Keep draining so pw-record never blocks on a full stderr pipe.
                let _ = std::io::copy(&mut err, &mut std::io::sink());
            });
        }
        let max_samples = (max_seconds as usize).max(1) * TARGET_RATE as usize;
        let reader = std::thread::Builder::new()
            .name("scribiva-capture".into())
            .spawn(move || {
                let mut stdout = stdout;
                let mut samples = Vec::with_capacity(TARGET_RATE as usize * 30);
                let mut carry = None;
                let mut on_full = Some(on_full);
                let mut buf = [0u8; 8192];
                loop {
                    match stdout.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if !push_pcm(&mut carry, &buf[..n], &mut samples, max_samples) {
                                if let Some(cb) = on_full.take() {
                                    cb();
                                }
                            }
                        }
                    }
                }
                samples
            })
            .map_err(|e| format!("MIC_ERROR: {e}"))?;
        let mut rec = Self {
            child: Some(child),
            reader: Some(reader),
            stderr,
        };
        // pw-record exits right away when the node or the daemon is missing.
        let deadline = Instant::now() + Duration::from_millis(250);
        while Instant::now() < deadline {
            let exited = rec.child.as_mut().and_then(|c| c.try_wait().ok()).flatten();
            if let Some(status) = exited {
                rec.child = None;
                let _ = rec.reader.take().map(JoinHandle::join);
                std::thread::sleep(Duration::from_millis(20));
                let detail = rec.stderr.lock().map(|s| s.clone()).unwrap_or_default();
                return Err(format!(
                    "MIC_ERROR: pw-record exited ({status}){}",
                    if detail.is_empty() {
                        String::new()
                    } else {
                        format!(": {detail}")
                    }
                ));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        Ok(rec)
    }

    /// Stop capturing and return 16 kHz mono samples.
    pub fn finish(mut self) -> Vec<i16> {
        // pw-record flushes and exits on SIGTERM; its stdout then hits EOF.
        // Reap the child first (SIGKILL after 2 s) so the reader can never hang.
        if let Some(mut child) = self.child.take() {
            sigterm(&child);
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                match child.try_wait() {
                    Ok(Some(_)) | Err(_) => break,
                    Ok(None) if Instant::now() >= deadline => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break;
                    }
                    Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
        }
        self.reader
            .take()
            .and_then(|r| r.join().ok())
            .unwrap_or_default()
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        // Cancel path: kill the recorder and discard samples.
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(r) = self.reader.take() {
            let _ = r.join();
        }
    }
}

/// Short local microphone test: never uploaded, never stored.
pub fn mic_test(device_id: &str, millis: u64) -> Result<(f32, f32), String> {
    let rec = Recorder::start(device_id, 5, Box::new(|| {}))?;
    std::thread::sleep(Duration::from_millis(millis.clamp(300, 4000)));
    let samples = rec.finish();
    Ok(crate::audio_dsp::levels(&samples))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_bytes_to_i16_with_odd_chunks() {
        let mut out = Vec::new();
        let mut carry = None;
        // 0x0001 = 1, 0xFFFF = -1, 0x8000 = i16::MIN, split across chunks.
        assert!(push_pcm(&mut carry, &[0x01], &mut out, 100));
        assert!(out.is_empty() && carry == Some(0x01));
        assert!(push_pcm(
            &mut carry,
            &[0x00, 0xFF, 0xFF, 0x00],
            &mut out,
            100
        ));
        assert_eq!(out, vec![1, -1]);
        assert_eq!(carry, Some(0x00));
        assert!(push_pcm(&mut carry, &[0x80], &mut out, 100));
        assert_eq!(out, vec![1, -1, i16::MIN]);
        assert_eq!(carry, None);
        assert!(push_pcm(&mut carry, &[], &mut out, 100));
    }

    #[test]
    fn pcm_stops_at_max() {
        let mut out = Vec::new();
        let mut carry = None;
        let bytes: Vec<u8> = (0..10i16).flat_map(i16::to_le_bytes).collect();
        assert!(!push_pcm(&mut carry, &bytes, &mut out, 4));
        assert_eq!(out, vec![0, 1, 2, 3]);
        assert!(!push_pcm(&mut carry, &bytes, &mut out, 4));
        assert_eq!(out.len(), 4);
    }

    #[test]
    fn record_args_and_node_names() {
        assert_eq!(
            record_args(None),
            ["--rate", "16000", "--channels", "1", "--format", "s16", "-"]
        );
        assert_eq!(
            record_args(Some("alsa_input.usb-X.mono")).first().unwrap(),
            "--target=alsa_input.usb-X.mono"
        );
        assert!(valid_node_name("alsa_input.pci-0000_00_1f.3.analog-stereo"));
        assert!(valid_node_name("bluez_input.00:11:22:33:44:55.0"));
        assert!(!valid_node_name("--rate=8000"));
        assert!(!valid_node_name("a b"));
        assert!(!valid_node_name("x;rm"));
        assert!(!valid_node_name(""));
    }

    const FIXTURE: &str = r#"[
      {"id": 30, "type": "PipeWire:Interface:Node", "info": {"props": {
        "media.class": "Audio/Sink", "node.name": "alsa_output.speakers", "node.description": "Speakers"}}},
      {"id": 41, "type": "PipeWire:Interface:Node", "info": {"props": {
        "media.class": "Audio/Source", "node.name": "alsa_input.usb-mic", "node.description": "USB   Microphone",
        "priority.session": 2000}}},
      {"id": 42, "type": "PipeWire:Interface:Node", "info": {"props": {
        "media.class": "Audio/Source", "node.name": "alsa_input.pci-internal", "node.nick": "Internal Mic",
        "priority.session": "1900"}}},
      {"id": 43, "type": "PipeWire:Interface:Node", "info": {"props": {
        "media.class": "Audio/Source/Virtual", "node.name": "echo-cancel-source", "node.description": "Echo-Cancel Source"}}},
      {"id": 44, "type": "PipeWire:Interface:Node", "info": {"props": {
        "media.class": "Audio/Source", "node.name": "alsa_input.usb-mic", "node.description": "Dup", "node.virtual": "true"}}},
      {"id": 45, "type": "PipeWire:Interface:Node", "info": {"props": {"media.class": "Audio/Source"}}},
      {"id": 46, "type": "PipeWire:Interface:Node", "info": null},
      {"id": 0, "type": "PipeWire:Interface:Metadata", "props": {"metadata.name": "settings"}, "metadata": [
        {"key": "default.audio.source", "value": {"name": "wrong"}}]},
      {"id": 1, "type": "PipeWire:Interface:Metadata", "props": {"metadata.name": "default"}, "metadata": [
        {"key": "default.audio.sink", "value": {"name": "alsa_output.speakers"}},
        {"key": "default.audio.source", "type": "Spa:String:JSON", "value": {"name": "alsa_input.pci-internal"}}]}
    ]"#;

    #[test]
    fn pw_dump_sources_and_default() {
        let (devices, default) = parse_pw_dump(FIXTURE).unwrap();
        assert_eq!(default.as_deref(), Some("alsa_input.pci-internal"));
        let ids: Vec<&str> = devices.iter().map(|d| d.id.as_str()).collect();
        // Default first, then physical by priority, virtual last; sinks and nameless nodes skipped.
        assert_eq!(
            ids,
            [
                "alsa_input.pci-internal",
                "alsa_input.usb-mic",
                "echo-cancel-source"
            ]
        );
        assert!(devices[0].default && !devices[1].default);
        assert_eq!(devices[0].name, "Internal Mic");
        // Duplicate node name: the physical, better-ranked entry wins; whitespace collapsed.
        assert_eq!(devices[1].name, "USB Microphone");
        assert!(!devices[1].is_virtual);
        assert!(devices[2].is_virtual);
        assert_eq!(
            active_node(&devices, default.as_deref(), "alsa_input.usb-mic").as_deref(),
            Some("alsa_input.usb-mic")
        );
        assert_eq!(
            active_node(&devices, default.as_deref(), "gone").as_deref(),
            Some("alsa_input.pci-internal")
        );
    }

    #[test]
    fn pw_dump_default_as_json_string_and_errors() {
        let json = r#"[{"type": "PipeWire:Interface:Metadata", "props": {"metadata.name": "default"},
            "metadata": [{"key": "default.audio.source", "value": "{\"name\":\"mic-a\"}"}]}]"#;
        let (devices, default) = parse_pw_dump(json).unwrap();
        assert!(devices.is_empty());
        assert_eq!(default.as_deref(), Some("mic-a"));
        assert!(parse_pw_dump("{}").is_err());
        assert!(parse_pw_dump("not json").is_err());
    }
}
