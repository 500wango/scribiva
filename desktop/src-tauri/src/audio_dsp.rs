//! Platform-independent audio processing: downmix, resample to 16 kHz mono,
//! WAV/FLAC encoding, abnormal-signal detection and silence trimming before
//! upload. The capture adapters feed this; the logic is unit-tested on every
//! platform.

use std::io::Cursor;

/// Target format required by the upload path (mono, 16 kHz, 16-bit PCM).
pub const TARGET_RATE: u32 = 16_000;
/// Recordings shorter than this are dropped without upload (FR-008).
pub const MIN_RECORDING_MS: u64 = 1_000;

/// Average interleaved frames into one mono channel.
pub fn downmix_to_mono(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// Streaming sample-rate converter for mono `f32` input.
///
/// Downsampling uses box-filter averaging over each output period (a cheap
/// anti-alias low-pass that is adequate for speech). Upsampling uses linear
/// interpolation. State carries across `push` calls so callback-sized chunks
/// produce the same output as one large buffer.
#[derive(Debug, Clone)]
pub struct Resampler {
    src_rate: u32,
    dst_rate: u32,
    // downsampling state
    phase: u64,
    acc: f32,
    count: u32,
    // upsampling state
    prev: Option<f32>,
    pos: f64,
}

impl Resampler {
    pub fn new(src_rate: u32, dst_rate: u32) -> Self {
        Self {
            src_rate: src_rate.max(1),
            dst_rate: dst_rate.max(1),
            phase: 0,
            acc: 0.0,
            count: 0,
            prev: None,
            pos: 0.0,
        }
    }

    pub fn push(&mut self, input: &[f32], out: &mut Vec<f32>) {
        use std::cmp::Ordering;
        match self.src_rate.cmp(&self.dst_rate) {
            Ordering::Equal => out.extend_from_slice(input),
            Ordering::Greater => self.push_down(input, out),
            Ordering::Less => self.push_up(input, out),
        }
    }

    fn push_down(&mut self, input: &[f32], out: &mut Vec<f32>) {
        let src = u64::from(self.src_rate);
        let dst = u64::from(self.dst_rate);
        for &sample in input {
            self.acc += sample;
            self.count += 1;
            self.phase += dst;
            if self.phase >= src {
                self.phase -= src;
                out.push(self.acc / self.count as f32);
                self.acc = 0.0;
                self.count = 0;
            }
        }
    }

    fn push_up(&mut self, input: &[f32], out: &mut Vec<f32>) {
        let step = f64::from(self.src_rate) / f64::from(self.dst_rate);
        for &sample in input {
            let Some(prev) = self.prev else {
                self.prev = Some(sample);
                out.push(sample);
                self.pos = step;
                continue;
            };
            // Emit every output position that falls in (prev, sample].
            while self.pos <= 1.0 {
                let t = self.pos as f32;
                out.push(prev + (sample - prev) * t);
                self.pos += step;
            }
            self.pos -= 1.0;
            self.prev = Some(sample);
        }
    }
}

/// Convert a float sample in [-1, 1] to signed 16-bit with clamping.
pub fn f32_to_i16(sample: f32) -> i16 {
    let s = if sample.is_finite() { sample } else { 0.0 };
    (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16
}

/// Duration in milliseconds of `samples` mono samples at `rate`.
pub fn duration_ms(samples: usize, rate: u32) -> u64 {
    (samples as u64 * 1000) / u64::from(rate.max(1))
}

/// Peak and RMS level (0..1) for the microphone test meter.
pub fn levels(samples: &[i16]) -> (f32, f32) {
    if samples.is_empty() {
        return (0.0, 0.0);
    }
    let mut peak = 0f32;
    let mut sum = 0f64;
    for &s in samples {
        let v = f32::from(s) / f32::from(i16::MAX);
        peak = peak.max(v.abs());
        sum += f64::from(v) * f64::from(v);
    }
    (
        peak.min(1.0),
        ((sum / samples.len() as f64).sqrt() as f32).min(1.0),
    )
}

/// Encode mono 16-bit PCM samples as a WAV file in memory.
pub fn encode_wav(samples: &[i16], rate: u32) -> Result<Vec<u8>, String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = Cursor::new(Vec::with_capacity(44 + samples.len() * 2));
    {
        let mut writer =
            hound::WavWriter::new(&mut cursor, spec).map_err(|e| format!("WAV encode: {e}"))?;
        let mut i16_writer = writer.get_i16_writer(samples.len() as u32);
        for &s in samples {
            i16_writer.write_sample(s);
        }
        i16_writer.flush().map_err(|e| format!("WAV encode: {e}"))?;
        writer.finalize().map_err(|e| format!("WAV encode: {e}"))?;
    }
    Ok(cursor.into_inner())
}

/// Encode mono 16-bit PCM samples as FLAC in memory (pure Rust, lossless).
/// Typically 40–60 % of the WAV size for speech (PRD §11.2: smaller
/// cross-border uploads).
pub fn encode_flac(samples: &[i16], rate: u32) -> Result<Vec<u8>, String> {
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;
    if samples.is_empty() {
        return Err("FLAC encode: no samples".into());
    }
    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|(_, e)| format!("FLAC config: {e:?}"))?;
    let wide: Vec<i32> = samples.iter().map(|&s| i32::from(s)).collect();
    let source = flacenc::source::MemSource::from_samples(&wide, 1, 16, rate as usize);
    let stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size)
        .map_err(|e| format!("FLAC encode: {e:?}"))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream
        .write(&mut sink)
        .map_err(|e| format!("FLAC write: {e:?}"))?;
    Ok(sink.into_inner())
}

/// One upload body: FLAC when encoding works, otherwise the WAV unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadAudio {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    pub file_name: &'static str,
}

/// Build the upload body from the WAV kept in memory for retry. Re-encoding
/// on every attempt keeps the retained copy a single, simple format.
pub fn upload_audio(wav: &[u8]) -> UploadAudio {
    let flac = hound::WavReader::new(Cursor::new(wav))
        .map_err(|e| e.to_string())
        .and_then(|r| {
            let spec = r.spec();
            if spec.channels != 1 || spec.bits_per_sample != 16 {
                return Err("unexpected WAV format".to_string());
            }
            let samples: Result<Vec<i16>, _> = r.into_samples::<i16>().collect();
            let samples = samples.map_err(|e| e.to_string())?;
            encode_flac(&samples, spec.sample_rate)
        });
    match flac {
        Ok(bytes) => UploadAudio {
            bytes,
            mime: "audio/flac",
            file_name: "audio.flac",
        },
        Err(_) => UploadAudio {
            bytes: wav.to_vec(),
            mime: "audio/wav",
            file_name: "audio.wav",
        },
    }
}

// ------------------------------------------------------------ silence trim

/// Energy frame for silence trimming.
pub const TRIM_FRAME_MS: u32 = 30;
/// Context kept before the first and after the last voiced frame.
pub const TRIM_PAD_MS: u32 = 300;
/// Absolute RMS floor for "voiced" (16-bit scale).
pub const TRIM_MIN_RMS: f64 = 120.0;

/// Voiced range `[first, last)` in samples, or `None` when nothing is voiced
/// or nothing would be cut. Port of `scribiva.audio.trim_silence`: energy
/// based, the threshold adapts to the quietest tenth of the recording.
pub fn voiced_range(samples: &[i16], rate: u32) -> Option<(usize, usize)> {
    let step = ((rate * TRIM_FRAME_MS / 1000) as usize).max(1);
    let levels: Vec<f64> = samples
        .chunks(step)
        .map(|c| (c.iter().map(|&x| f64::from(x).powi(2)).sum::<f64>() / c.len() as f64).sqrt())
        .collect();
    if levels.is_empty() {
        return None;
    }
    let mut sorted = levels.clone();
    sorted.sort_by(f64::total_cmp);
    let floor = sorted[sorted.len() / 10];
    let threshold = TRIM_MIN_RMS.max(floor * 3.0);
    let first_voiced = levels.iter().position(|&l| l >= threshold)?;
    let last_voiced = levels.iter().rposition(|&l| l >= threshold)?;
    let pad = (TRIM_PAD_MS / TRIM_FRAME_MS).max(1) as usize;
    let first = first_voiced.saturating_sub(pad) * step;
    let last = ((last_voiced + 1 + pad) * step).min(samples.len());
    if first == 0 && last >= samples.len() {
        return None;
    }
    Some((first, last))
}

/// Trim leading/trailing silence (FR-017: less input for phantom text).
/// Returns the samples unchanged when nothing is voiced, so the server can
/// still judge (and flag) a silent recording.
pub fn trim_silence(samples: Vec<i16>, rate: u32) -> (Vec<i16>, bool) {
    match voiced_range(&samples, rate) {
        Some((first, last)) => (samples[first..last].to_vec(), true),
        None => (samples, false),
    }
}

// ------------------------------------------------------------ abnormal signal

/// A sample at or beyond this magnitude counts as clipped.
pub const CLIP_LEVEL: i16 = 32_000;
/// Analysis window for clipping / DC offset.
pub const SIGNAL_WINDOW_MS: u32 = 200;
/// A window is "clipped" when at least this share of its samples is clipped.
pub const WINDOW_CLIPPED_SHARE: f64 = 0.10;
/// A window is "pinned" when its mean (DC offset) exceeds this magnitude.
pub const PINNED_DC: f64 = 16_000.0;
/// Refuse to upload when more than this share of the recording is abnormal.
pub const ABNORMAL_SHARE: f64 = 0.40;
/// A wake-up transient at the start of at most this length is cut instead.
pub const MAX_TRANSIENT_MS: u32 = 2_500;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SignalCheck {
    /// Upload, after skipping `skip` leading samples (start transient).
    Normal { skip: usize },
    /// Mostly clipped or pinned: do not upload (`MIC_SIGNAL_ABNORMAL`).
    Abnormal { share: f64 },
}

fn window_abnormal(w: &[i16]) -> bool {
    let clipped = w
        .iter()
        .filter(|&&x| x.unsigned_abs() >= CLIP_LEVEL as u16)
        .count();
    let mean = w.iter().map(|&x| f64::from(x)).sum::<f64>() / w.len().max(1) as f64;
    clipped as f64 >= WINDOW_CLIPPED_SHARE * w.len() as f64 || mean.abs() > PINNED_DC
}

/// Detect a broken microphone signal (DC offset or clipping bursts, as seen
/// on a faulty jack or a gain set far too high). A leading run of abnormal
/// windows up to `MAX_TRANSIENT_MS` is treated as a device wake-up transient
/// and skipped; the rest decides.
pub fn check_signal(samples: &[i16], rate: u32) -> SignalCheck {
    let step = ((rate * SIGNAL_WINDOW_MS / 1000) as usize).max(1);
    let flags: Vec<(usize, bool)> = samples
        .chunks(step)
        .map(|w| (w.len(), window_abnormal(w)))
        .collect();
    let lead_windows = flags.iter().take_while(|(_, bad)| *bad).count();
    let lead_samples: usize = flags[..lead_windows].iter().map(|(n, _)| n).sum();
    let max_transient = (rate * MAX_TRANSIENT_MS / 1000) as usize;
    let (skip, rest) = if lead_windows > 0 && lead_samples <= max_transient {
        (lead_samples, &flags[lead_windows..])
    } else {
        (0, &flags[..])
    };
    let total: usize = rest.iter().map(|(n, _)| n).sum();
    if total == 0 {
        // Empty, or nothing but a transient.
        return if lead_windows > 0 {
            SignalCheck::Abnormal { share: 1.0 }
        } else {
            SignalCheck::Normal { skip: 0 }
        };
    }
    let bad: usize = rest.iter().filter(|(_, b)| *b).map(|(n, _)| n).sum();
    let share = bad as f64 / total as f64;
    if share > ABNORMAL_SHARE {
        SignalCheck::Abnormal { share }
    } else {
        SignalCheck::Normal { skip }
    }
}

/// Recording after the local checks, ready to encode and upload.
#[derive(Debug, Clone, PartialEq)]
pub enum Prepared {
    Abnormal,
    /// Nothing above the speech threshold once a start transient is cut:
    /// uploading would only invite phantom text (FR-017).
    NoSpeech,
    Ready {
        samples: Vec<i16>,
        /// Length after cutting a start transient, before silence trimming;
        /// used for the "too short" rule.
        spoken_ms: u64,
    },
}

/// Signal check, transient cut and silence trim, in that order.
/// Speech needs at least `SPEECH_MIN_MS` of frames above `TRIM_MIN_RMS`
/// (AC energy, DC removed so a slow offset drift is not mistaken for voice).
pub const SPEECH_MIN_MS: u32 = 400;

pub fn has_speech(samples: &[i16], rate: u32) -> bool {
    let step = ((rate * TRIM_FRAME_MS / 1000) as usize).max(1);
    let needed = (SPEECH_MIN_MS / TRIM_FRAME_MS).max(1) as usize;
    let voiced = samples
        .chunks(step)
        .filter(|c| {
            let n = c.len() as f64;
            let mean = c.iter().map(|&v| f64::from(v)).sum::<f64>() / n;
            let ac = (c
                .iter()
                .map(|&v| (f64::from(v) - mean).powi(2))
                .sum::<f64>()
                / n)
                .sqrt();
            ac >= TRIM_MIN_RMS
        })
        .count();
    voiced >= needed
}

pub fn prepare_for_upload(mut samples: Vec<i16>, rate: u32) -> Prepared {
    match check_signal(&samples, rate) {
        SignalCheck::Abnormal { .. } => Prepared::Abnormal,
        SignalCheck::Normal { skip } => {
            samples.drain(..skip.min(samples.len()));
            if !has_speech(&samples, rate) {
                return Prepared::NoSpeech;
            }
            let spoken_ms = duration_ms(samples.len(), rate);
            let (samples, _) = trim_silence(samples, rate);
            Prepared::Ready { samples, spoken_ms }
        }
    }
}

/// Accumulates captured audio of any device format into 16 kHz mono i16.
#[derive(Debug, Clone)]
pub struct Converter {
    channels: usize,
    resampler: Resampler,
    scratch: Vec<f32>,
    pub samples: Vec<i16>,
    max_samples: usize,
}

impl Converter {
    pub fn new(src_rate: u32, channels: u16, max_seconds: u32) -> Self {
        Self {
            channels: usize::from(channels.max(1)),
            resampler: Resampler::new(src_rate, TARGET_RATE),
            scratch: Vec::new(),
            samples: Vec::new(),
            max_samples: TARGET_RATE as usize * max_seconds.max(1) as usize,
        }
    }

    /// Push interleaved float frames. Returns false once the max duration is reached.
    pub fn push_interleaved(&mut self, interleaved: &[f32]) -> bool {
        if self.samples.len() >= self.max_samples {
            return false;
        }
        let mono = downmix_to_mono(interleaved, self.channels);
        self.scratch.clear();
        self.resampler.push(&mono, &mut self.scratch);
        let room = self.max_samples - self.samples.len();
        self.samples
            .extend(self.scratch.iter().take(room).map(|&s| f32_to_i16(s)));
        self.samples.len() < self.max_samples
    }
}

#[cfg(test)]
mod speech_tests {
    use super::*;

    fn tone(ms: u32, amp: f64) -> Vec<i16> {
        (0..(TARGET_RATE * ms / 1000))
            .map(|i| (amp * (f64::from(i) / 9.0).sin()) as i16)
            .collect()
    }

    #[test]
    fn silence_after_a_start_transient_is_no_speech() {
        // Developer's broken mic: pinned for 2 s, then only room noise.
        let mut s = vec![i16::MAX; (TARGET_RATE * 2) as usize];
        s.extend(tone(8000, 60.0));
        assert!(matches!(
            prepare_for_upload(s, TARGET_RATE),
            Prepared::NoSpeech
        ));
    }

    #[test]
    fn slow_dc_drift_is_not_speech() {
        let s: Vec<i16> = (0..TARGET_RATE * 4)
            .map(|i| -8000 + (i / 8) as i16)
            .collect();
        assert!(!has_speech(&s, TARGET_RATE));
    }

    #[test]
    fn short_word_counts_as_speech() {
        let mut s = tone(1000, 40.0);
        s.extend(tone(600, 5000.0));
        s.extend(tone(1000, 40.0));
        assert!(matches!(
            prepare_for_upload(s, TARGET_RATE),
            Prepared::Ready { .. }
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, freq: f32, seconds: f32, amp: f32) -> Vec<f32> {
        let n = (rate as f32 * seconds) as usize;
        (0..n)
            .map(|i| amp * (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin())
            .collect()
    }

    #[test]
    fn downmix_averages_channels() {
        let stereo = [1.0, 0.0, 0.5, 0.5, -1.0, 1.0];
        assert_eq!(downmix_to_mono(&stereo, 2), vec![0.5, 0.5, 0.0]);
        assert_eq!(downmix_to_mono(&[0.1, 0.2], 1), vec![0.1, 0.2]);
    }

    #[test]
    fn resample_48k_to_16k_keeps_duration_and_level() {
        let input = sine(48_000, 440.0, 1.0, 0.5);
        let mut out = Vec::new();
        Resampler::new(48_000, 16_000).push(&input, &mut out);
        assert_eq!(out.len(), 16_000);
        let peak = out.iter().fold(0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.45 && peak <= 0.5, "peak {peak}");
    }

    #[test]
    fn resample_44100_chunked_matches_single_pass() {
        let input = sine(44_100, 300.0, 2.0, 0.8);
        let mut whole = Vec::new();
        Resampler::new(44_100, 16_000).push(&input, &mut whole);
        let mut chunked = Vec::new();
        let mut r = Resampler::new(44_100, 16_000);
        for chunk in input.chunks(441) {
            r.push(chunk, &mut chunked);
        }
        assert_eq!(whole, chunked);
        assert!((whole.len() as i64 - 32_000).abs() <= 1, "{}", whole.len());
    }

    #[test]
    fn resample_upsamples_8k() {
        let input = sine(8_000, 200.0, 1.0, 0.5);
        let mut out = Vec::new();
        let mut r = Resampler::new(8_000, 16_000);
        for chunk in input.chunks(100) {
            r.push(chunk, &mut out);
        }
        assert!((out.len() as i64 - 16_000).abs() <= 2, "{}", out.len());
        // Interpolated midpoints lie between neighbours.
        assert!((out[1] - (input[0] + input[1]) / 2.0).abs() < 1e-6);
    }

    #[test]
    fn i16_conversion_clamps() {
        assert_eq!(f32_to_i16(2.0), i16::MAX);
        assert_eq!(f32_to_i16(-2.0), -i16::MAX);
        assert_eq!(f32_to_i16(f32::NAN), 0);
        assert_eq!(f32_to_i16(0.0), 0);
    }

    #[test]
    fn short_recordings_are_rejected() {
        assert!(duration_ms(15_999, TARGET_RATE) < MIN_RECORDING_MS);
        assert!(duration_ms(16_000, TARGET_RATE) >= MIN_RECORDING_MS);
    }

    #[test]
    fn converter_stops_at_max_duration() {
        let mut c = Converter::new(48_000, 2, 1);
        let frames = vec![0.25f32; 48_000 * 2 * 2]; // 2 s stereo
        assert!(!c.push_interleaved(&frames));
        assert_eq!(c.samples.len(), 16_000);
        assert!(!c.push_interleaved(&frames));
        assert_eq!(c.samples.len(), 16_000);
    }

    #[test]
    fn wav_header_is_mono_16k_16bit() {
        let samples: Vec<i16> = (0..16_000).map(|i| (i % 100) as i16).collect();
        let bytes = encode_wav(&samples, TARGET_RATE).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(bytes.len(), 44 + samples.len() * 2);
        let reader = hound::WavReader::new(Cursor::new(bytes)).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.channels, 1);
        assert_eq!(spec.sample_rate, 16_000);
        assert_eq!(spec.bits_per_sample, 16);
        let decoded: Vec<i16> = reader.into_samples::<i16>().map(Result::unwrap).collect();
        assert_eq!(decoded, samples);
    }

    #[test]
    fn levels_report_peak_and_rms() {
        let (peak, rms) = levels(&[i16::MAX, -i16::MAX]);
        assert!((peak - 1.0).abs() < 1e-6 && (rms - 1.0).abs() < 1e-6);
        assert_eq!(levels(&[]), (0.0, 0.0));
    }

    // ------------------------------------------------------------ FLAC

    fn speechlike(seconds: f32, amp: f32) -> Vec<i16> {
        // Two partials with a slow envelope: compressible but not trivial.
        let n = (TARGET_RATE as f32 * seconds) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / TARGET_RATE as f32;
                let env = 0.6 + 0.4 * (2.0 * std::f32::consts::PI * 3.0 * t).sin();
                let v = env
                    * ((2.0 * std::f32::consts::PI * 180.0 * t).sin()
                        + 0.4 * (2.0 * std::f32::consts::PI * 720.0 * t).sin());
                (v * amp) as i16
            })
            .collect()
    }

    #[test]
    fn flac_header_and_round_trip() {
        let samples = speechlike(2.3, 6000.0);
        let flac = encode_flac(&samples, TARGET_RATE).unwrap();
        assert_eq!(&flac[0..4], b"fLaC");
        // First metadata block is STREAMINFO (type 0), 34 bytes long.
        assert_eq!(flac[4] & 0x7f, 0);
        assert_eq!(u32::from_be_bytes([0, flac[5], flac[6], flac[7]]), 34);
        let mut reader = claxon::FlacReader::new(Cursor::new(&flac)).unwrap();
        let info = reader.streaminfo();
        assert_eq!(info.sample_rate, 16_000);
        assert_eq!(info.channels, 1);
        assert_eq!(info.bits_per_sample, 16);
        assert_eq!(info.samples, Some(samples.len() as u64));
        let decoded: Vec<i16> = reader.samples().map(|s| s.unwrap() as i16).collect();
        assert_eq!(decoded, samples);
        assert!(
            flac.len() < samples.len() * 2,
            "FLAC should be smaller than PCM"
        );
    }

    #[test]
    fn upload_prefers_flac_and_falls_back_to_wav() {
        let samples = speechlike(1.5, 4000.0);
        let wav = encode_wav(&samples, TARGET_RATE).unwrap();
        let up = upload_audio(&wav);
        assert_eq!((up.mime, up.file_name), ("audio/flac", "audio.flac"));
        assert_eq!(&up.bytes[0..4], b"fLaC");
        // Not a WAV: uploaded unchanged as WAV (the server validates it).
        let junk = b"RIFF0000WAVEjunk".to_vec();
        let up = upload_audio(&junk);
        assert_eq!((up.mime, up.file_name), ("audio/wav", "audio.wav"));
        assert_eq!(up.bytes, junk);
        assert!(encode_flac(&[], TARGET_RATE).is_err());
    }

    // ------------------------------------------------------------ trim

    fn silence(seconds: f32, noise: i16) -> Vec<i16> {
        let n = (TARGET_RATE as f32 * seconds) as usize;
        (0..n)
            .map(|i| if i % 2 == 0 { noise } else { -noise })
            .collect()
    }

    #[test]
    fn trim_keeps_speech_with_padding() {
        let mut s = silence(2.0, 20);
        let speech_start = s.len();
        s.extend(speechlike(1.0, 5000.0));
        let speech_end = s.len();
        s.extend(silence(3.0, 20));
        let (first, last) = voiced_range(&s, TARGET_RATE).unwrap();
        let pad = (TARGET_RATE * TRIM_PAD_MS / 1000) as usize;
        let frame = (TARGET_RATE * TRIM_FRAME_MS / 1000) as usize;
        assert!(
            first <= speech_start && first + pad + frame >= speech_start,
            "{first}"
        );
        assert!(
            last >= speech_end && last <= speech_end + pad + 2 * frame,
            "{last}"
        );
        let (trimmed, did) = trim_silence(s.clone(), TARGET_RATE);
        assert!(did);
        assert_eq!(trimmed.len(), last - first);
        assert!(duration_ms(trimmed.len(), TARGET_RATE) < 1_800);
    }

    #[test]
    fn trim_skips_silent_and_full_recordings() {
        let quiet = silence(3.0, 30);
        let (out, did) = trim_silence(quiet.clone(), TARGET_RATE);
        assert!(!did);
        assert_eq!(out, quiet);
        let speech = speechlike(2.0, 5000.0);
        assert_eq!(voiced_range(&speech, TARGET_RATE), None);
        assert_eq!(voiced_range(&[], TARGET_RATE), None);
    }

    // ------------------------------------------------------------ signal

    fn square(seconds: f32, amp: i16) -> Vec<i16> {
        let n = (TARGET_RATE as f32 * seconds) as usize;
        (0..n)
            .map(|i| if (i / 40) % 2 == 0 { amp } else { -amp })
            .collect()
    }

    fn pinned(seconds: f32, level: i16) -> Vec<i16> {
        vec![level; (TARGET_RATE as f32 * seconds) as usize]
    }

    #[test]
    fn clean_speech_is_normal() {
        assert_eq!(
            check_signal(&speechlike(4.0, 8000.0), TARGET_RATE),
            SignalCheck::Normal { skip: 0 }
        );
        // Occasional peaks at full scale are not a broken signal.
        let mut s = speechlike(4.0, 8000.0);
        for i in (0..s.len()).step_by(997) {
            s[i] = i16::MAX;
        }
        assert_eq!(
            check_signal(&s, TARGET_RATE),
            SignalCheck::Normal { skip: 0 }
        );
        assert_eq!(
            check_signal(&[], TARGET_RATE),
            SignalCheck::Normal { skip: 0 }
        );
    }

    #[test]
    fn full_scale_square_is_abnormal() {
        assert!(matches!(
            check_signal(&square(3.0, i16::MAX), TARGET_RATE),
            SignalCheck::Abnormal { share } if share > 0.99
        ));
        assert_eq!(
            prepare_for_upload(square(3.0, -32_767), TARGET_RATE),
            Prepared::Abnormal
        );
    }

    #[test]
    fn all_pinned_is_abnormal() {
        assert!(matches!(
            check_signal(&pinned(5.0, 20_000), TARGET_RATE),
            SignalCheck::Abnormal { .. }
        ));
        assert!(matches!(
            check_signal(&pinned(5.0, -24_000), TARGET_RATE),
            SignalCheck::Abnormal { .. }
        ));
        // Short and entirely pinned: nothing left after the transient.
        assert!(matches!(
            check_signal(&pinned(1.2, 30_000), TARGET_RATE),
            SignalCheck::Abnormal { share } if share == 1.0
        ));
    }

    #[test]
    fn start_transient_is_cut() {
        let mut s = pinned(1.4, 30_000);
        let transient = s.len();
        s.extend(speechlike(3.0, 6000.0));
        match check_signal(&s, TARGET_RATE) {
            SignalCheck::Normal { skip } => {
                let window = (TARGET_RATE * SIGNAL_WINDOW_MS / 1000) as usize;
                assert!(
                    skip >= transient - window && skip <= transient + window,
                    "{skip}"
                );
            }
            other => panic!("{other:?}"),
        }
        match prepare_for_upload(s, TARGET_RATE) {
            Prepared::Ready { samples, spoken_ms } => {
                assert!((2_900..=3_100).contains(&spoken_ms), "{spoken_ms}");
                assert!(samples.iter().all(|x| x.unsigned_abs() < CLIP_LEVEL as u16));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn long_or_dominant_bursts_are_abnormal() {
        // Transient longer than 2.5 s is not cut; at 50 % it rejects.
        let mut s = square(3.0, i16::MAX);
        s.extend(speechlike(3.0, 6000.0));
        assert!(matches!(
            check_signal(&s, TARGET_RATE),
            SignalCheck::Abnormal { .. }
        ));
        // Bursts spread through the recording (not a start transient).
        let mut s = Vec::new();
        for _ in 0..5 {
            s.extend(speechlike(0.4, 6000.0));
            s.extend(square(0.6, i16::MAX));
        }
        assert!(matches!(
            check_signal(&s, TARGET_RATE),
            SignalCheck::Abnormal { .. }
        ));
        // A long clean recording with a short burst in the middle is fine.
        let mut s = speechlike(5.0, 6000.0);
        s.extend(square(0.4, i16::MAX));
        s.extend(speechlike(5.0, 6000.0));
        assert_eq!(
            check_signal(&s, TARGET_RATE),
            SignalCheck::Normal { skip: 0 }
        );
    }
}
