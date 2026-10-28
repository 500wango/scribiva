//! Microphone capture with cpal (WASAPI). The stream exists only while
//! recording; a dedicated thread owns it (cpal streams are not `Send` on every
//! backend). Samples are converted to 16 kHz mono i16 on the fly.

use crate::audio_dsp::Converter;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

#[derive(Debug, Clone, Serialize)]
pub struct InputDevice {
    pub id: String,
    pub name: String,
    pub default: bool,
}

pub fn list_devices() -> Result<Vec<InputDevice>, String> {
    let host = cpal::default_host();
    let default_id = host
        .default_input_device()
        .and_then(|d| d.id().ok())
        .map(|id| id.to_string());
    let devices = host
        .input_devices()
        .map_err(|e| format!("MIC_ENUMERATION_FAILED: {e}"))?;
    Ok(devices
        .filter_map(|d| {
            let id = d.id().ok()?.to_string();
            let name = d
                .description()
                .map(|desc| desc.name().to_string())
                .unwrap_or_else(|_| id.clone());
            Some(InputDevice {
                default: default_id.as_deref() == Some(id.as_str()),
                id,
                name,
            })
        })
        .collect())
}

fn pick_device(device_id: &str) -> Result<cpal::Device, String> {
    let host = cpal::default_host();
    if !device_id.is_empty() {
        if let Ok(id) = device_id.parse::<cpal::DeviceId>() {
            if let Some(device) = host.device_by_id(&id) {
                return Ok(device);
            }
        }
        // Configured device is absent right now: fall back to the default.
    }
    host.default_input_device()
        .ok_or_else(|| "MIC_NOT_FOUND: no input device".to_string())
}

fn mic_error(e: cpal::Error) -> String {
    match e.kind() {
        cpal::ErrorKind::PermissionDenied => format!("MIC_PERMISSION_DENIED: {e}"),
        cpal::ErrorKind::DeviceBusy => format!("MIC_BUSY: {e}"),
        cpal::ErrorKind::DeviceNotAvailable => format!("MIC_NOT_FOUND: {e}"),
        _ => format!("MIC_ERROR: {e}"),
    }
}

fn build_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    converter: Arc<Mutex<Converter>>,
    full: Arc<AtomicBool>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let mut scratch: Vec<f32> = Vec::new();
    device
        .build_input_stream::<T, _, _>(
            config,
            move |data: &[T], _info: &cpal::InputCallbackInfo| {
                scratch.clear();
                scratch.extend(data.iter().map(|s| s.to_sample::<f32>()));
                if let Ok(mut c) = converter.lock() {
                    if !c.push_interleaved(&scratch) {
                        full.store(true, Ordering::SeqCst);
                    }
                }
            },
            |_err| {
                // Stream errors (device unplugged) end up as a short or empty
                // recording; the state machine reports it to the user.
            },
            Some(Duration::from_secs(3)),
        )
        .map_err(mic_error)
}

pub struct Recorder {
    stop_tx: mpsc::Sender<()>,
    join: Option<JoinHandle<Vec<i16>>>,
}

impl Recorder {
    /// Open the device and start capturing. `on_full` runs once (on the
    /// capture thread) when `max_seconds` is reached; it must not block on
    /// this recorder (spawn work elsewhere).
    pub fn start(
        device_id: &str,
        max_seconds: u32,
        on_full: Box<dyn FnOnce() + Send>,
    ) -> Result<Self, String> {
        let device_id = device_id.to_string();
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(), String>>(1);
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let join = std::thread::Builder::new()
            .name("scribiva-capture".into())
            .spawn(move || {
                let setup = (|| {
                    let device = pick_device(&device_id)?;
                    let supported = device.default_input_config().map_err(mic_error)?;
                    let config: cpal::StreamConfig = supported.config();
                    let converter = Arc::new(Mutex::new(Converter::new(
                        config.sample_rate,
                        config.channels,
                        max_seconds,
                    )));
                    let full = Arc::new(AtomicBool::new(false));
                    let (c, f) = (converter.clone(), full.clone());
                    let stream = match supported.sample_format() {
                        SampleFormat::F32 => build_stream::<f32>(&device, config, c, f),
                        SampleFormat::I16 => build_stream::<i16>(&device, config, c, f),
                        SampleFormat::I32 => build_stream::<i32>(&device, config, c, f),
                        SampleFormat::U16 => build_stream::<u16>(&device, config, c, f),
                        SampleFormat::U8 => build_stream::<u8>(&device, config, c, f),
                        SampleFormat::I8 => build_stream::<i8>(&device, config, c, f),
                        SampleFormat::F64 => build_stream::<f64>(&device, config, c, f),
                        other => Err(format!("MIC_UNSUPPORTED_FORMAT: {other:?}")),
                    }?;
                    stream.play().map_err(mic_error)?;
                    Ok::<_, String>((stream, converter, full))
                })();
                let (stream, converter, full) = match setup {
                    Ok(v) => {
                        let _ = ready_tx.send(Ok(()));
                        v
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return Vec::new();
                    }
                };
                let mut on_full = Some(on_full);
                loop {
                    match stop_rx.recv_timeout(Duration::from_millis(50)) {
                        Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
                        Err(RecvTimeoutError::Timeout) => {
                            if full.load(Ordering::SeqCst) {
                                if let Some(cb) = on_full.take() {
                                    cb();
                                }
                            }
                        }
                    }
                }
                drop(stream);
                let samples = converter
                    .lock()
                    .map(|mut c| std::mem::take(&mut c.samples))
                    .unwrap_or_default();
                samples
            })
            .map_err(|e| format!("MIC_ERROR: {e}"))?;
        match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(Self {
                stop_tx,
                join: Some(join),
            }),
            Ok(Err(e)) => {
                let _ = join.join();
                Err(e)
            }
            Err(_) => {
                let _ = stop_tx.send(());
                Err("MIC_ERROR: the input device did not start".into())
            }
        }
    }

    /// Stop capturing and return 16 kHz mono samples.
    pub fn finish(mut self) -> Vec<i16> {
        let _ = self.stop_tx.send(());
        self.join
            .take()
            .and_then(|j| j.join().ok())
            .unwrap_or_default()
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        // Cancel path: stop the stream and discard samples.
        let _ = self.stop_tx.send(());
        if let Some(j) = self.join.take() {
            let _ = j.join();
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
