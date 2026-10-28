//! Platform-independent audio conversion: downmix, resample to 16 kHz mono,
//! 16-bit PCM WAV encoding. The Windows capture thread feeds this; the logic is
//! unit-tested on every platform.

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
}
