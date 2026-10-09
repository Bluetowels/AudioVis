//! Audio capture into ring buffers: what one or more output devices are
//! playing (WASAPI loopback on Windows, a Core Audio tap on macOS 14.6 or
//! later), an input device, or a built-in test signal.
//! Only the front left and right channels of a device are used.
//! On Android the sound is gathered by the Java side of the app instead:
//! what other apps are playing, the microphone, or an audio file.

#[cfg(not(target_os = "android"))]
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Samples kept per device: long enough for a one-second window at 192 kHz.
const CAPACITY: usize = 1 << 18;
pub const MAX_METERED_CHANNELS: usize = 8;

pub struct Ring {
    left: Vec<f32>,
    right: Vec<f32>,
    pos: usize,
    /// Samples written since the stream started.
    pub total: u64,
    pub sample_rate: u32,
    /// Loudest sample seen on each device channel since the meters were last read.
    peaks: [f32; MAX_METERED_CHANNELS],
}

#[derive(Clone, Copy, PartialEq)]
pub enum Channel {
    Mono,
    Left,
    Right,
}

impl Ring {
    fn new(sample_rate: u32) -> Self {
        Self {
            left: vec![0.0; CAPACITY],
            right: vec![0.0; CAPACITY],
            pos: 0,
            total: 0,
            sample_rate,
            peaks: [0.0; MAX_METERED_CHANNELS],
        }
    }

    pub fn push(&mut self, l: f32, r: f32) {
        self.left[self.pos] = l;
        self.right[self.pos] = r;
        self.pos = (self.pos + 1) % CAPACITY;
        self.total += 1;
    }

    /// One frame as delivered by the device, with any number of channels.
    pub fn push_frame(&mut self, frame: &[f32]) {
        for (peak, s) in self.peaks.iter_mut().zip(frame) {
            *peak = peak.max(s.abs());
        }
        self.push(frame[0], frame[frame.len().min(2) - 1]);
    }

    /// Add the newest `n` samples of a channel to `out` (oldest first).
    fn add_last(&self, channel: Channel, out: &mut [f32]) {
        let n = out.len().min(CAPACITY);
        let start = (self.pos + CAPACITY - n) % CAPACITY;
        for (i, o) in out.iter_mut().enumerate().take(n) {
            let j = (start + i) % CAPACITY;
            *o += match channel {
                Channel::Mono => 0.5 * (self.left[j] + self.right[j]),
                Channel::Left => self.left[j],
                Channel::Right => self.right[j],
            };
        }
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum Source {
    /// Whatever the system's default output device is playing.
    SystemOutput,
    /// What the named output devices are playing, added together.
    Outputs(Vec<String>),
    Input(String),
    TestSignal,
    /// An audio file the user picked, played by the app itself.
    #[cfg(target_os = "android")]
    File,
}

impl Source {
    #[cfg(target_os = "android")]
    pub fn label(&self) -> String {
        match self {
            Source::SystemOutput => "Other apps' sound".into(),
            Source::Outputs(_) => "Chosen output devices".into(),
            Source::Input(_) => "Microphone".into(),
            Source::File => "Audio file".into(),
            Source::TestSignal => "Test signal".into(),
        }
    }

    #[cfg(not(target_os = "android"))]
    pub fn label(&self) -> String {
        match self {
            Source::SystemOutput => "Default output device".into(),
            Source::Outputs(names) if names.is_empty() => "Chosen output devices (none ticked)".into(),
            Source::Outputs(names) => format!("Chosen output devices ({})", names.len()),
            Source::Input(name) => format!("Input: {name}"),
            Source::TestSignal => "Test signal".into(),
        }
    }
}

/// Whatever keeps a device's sound arriving for as long as it is held.
#[cfg(not(target_os = "android"))]
type Stream = cpal::Stream;
#[cfg(target_os = "android")]
type Stream = crate::android::Stream;

struct Tap {
    name: String,
    format: String,
    channels: usize,
    ring: Arc<Mutex<Ring>>,
    _stream: Option<Stream>,
    last_total: u64,
    last_data: Instant,
}

/// What one captured device is doing, for display.
pub struct TapStatus {
    pub name: String,
    pub format: String,
    pub receiving: bool,
    /// Peak level per device channel in dBFS since the last call.
    pub peaks_db: Vec<f32>,
}

pub struct Capture {
    pub source: Source,
    pub errors: Vec<String>,
    taps: Vec<Tap>,
    consumed: u64,
}

#[cfg(not(target_os = "android"))]
fn device_name(device: &cpal::Device) -> String {
    device.description().map(|d| d.name().to_string()).unwrap_or_else(|_| "unknown".into())
}

/// Android has no list of devices to choose from: it routes sound itself.
#[cfg(target_os = "android")]
pub fn input_device_names() -> Vec<String> {
    Vec::new()
}

#[cfg(target_os = "android")]
pub fn output_device_names() -> Vec<String> {
    Vec::new()
}

#[cfg(not(target_os = "android"))]
pub fn input_device_names() -> Vec<String> {
    let mut names: Vec<String> =
        cpal::default_host().input_devices().map(|d| d.map(|d| device_name(&d)).collect()).unwrap_or_default();
    names.sort();
    names
}

#[cfg(not(target_os = "android"))]
pub fn output_device_names() -> Vec<String> {
    let mut names: Vec<String> =
        cpal::default_host().output_devices().map(|d| d.map(|d| device_name(&d)).collect()).unwrap_or_default();
    names.sort();
    names
}

impl Capture {
    /// On Android the Java side of the app gathers the sound and hands it
    /// over a block at a time (see `android.rs`), always as one device.
    #[cfg(target_os = "android")]
    pub fn open(source: Source) -> Self {
        use crate::android::Feed;
        let ring = Arc::new(Mutex::new(Ring::new(48_000)));
        let mut errors = Vec::new();
        let (name, format, feed) = match &source {
            Source::TestSignal => {
                spawn_test_signal(Arc::downgrade(&ring));
                ("Test signal", "generated, 48 kHz", Feed::None)
            }
            Source::SystemOutput => ("Other apps", "waiting for permission", Feed::Playback),
            Source::Input(_) => ("Microphone", "waiting for permission", Feed::Microphone),
            Source::File => ("Audio file", "no file chosen yet", Feed::File),
            Source::Outputs(_) => {
                errors.push("Android has no separate output devices to choose from".to_string());
                ("No source", "", Feed::None)
            }
        };
        let stream = Stream::start(feed, &ring);
        let taps = vec![Tap::new(name.into(), format.into(), 2, ring, Some(stream))];
        Self { source, errors, taps, consumed: 0 }
    }

    #[cfg(not(target_os = "android"))]
    pub fn open(source: Source) -> Self {
        let mut taps = Vec::new();
        let mut errors = Vec::new();
        let host = cpal::default_host();
        let mut add = |result: Result<Tap, String>| match result {
            Ok(tap) => taps.push(tap),
            Err(e) => errors.push(e),
        };
        match &source {
            Source::TestSignal => {
                let ring = Arc::new(Mutex::new(Ring::new(48_000)));
                spawn_test_signal(Arc::downgrade(&ring));
                add(Ok(Tap::new("Test signal".into(), "generated, 48 kHz".into(), 2, ring, None)));
            }
            Source::SystemOutput => add(host
                .default_output_device()
                .ok_or_else(|| "no default output device".to_string())
                .and_then(|d| Tap::open(d, true))),
            Source::Outputs(names) => {
                for name in names {
                    add(host
                        .output_devices()
                        .map_err(|e| e.to_string())
                        .and_then(|mut all| all.find(|d| device_name(d) == *name).ok_or_else(|| format!("output device not found: {name}")))
                        .and_then(|d| Tap::open(d, true)));
                }
            }
            Source::Input(name) => add(host
                .input_devices()
                .map_err(|e| e.to_string())
                .and_then(|mut all| all.find(|d| device_name(d) == *name).ok_or_else(|| format!("input device not found: {name}")))
                .and_then(|d| Tap::open(d, false))),
        }

        // Devices are added sample for sample, so they must share a rate.
        if let Some(rate) = taps.first().map(|t| t.ring.lock().unwrap().sample_rate) {
            taps.retain(|t| {
                let same = t.ring.lock().unwrap().sample_rate == rate;
                if !same {
                    errors.push(format!("{} skipped: its sample rate differs from {}", t.name, rate));
                }
                same
            });
        }
        Self { source, errors, taps, consumed: 0 }
    }

    pub fn sample_rate(&self) -> u32 {
        self.taps.first().map(|t| t.ring.lock().unwrap().sample_rate).unwrap_or(48_000)
    }

    /// Loopback capture delivers nothing while a device is silent, which would
    /// leave the last sound frozen in the buffer. Fill gaps with silence.
    pub fn pad_silence(&mut self) {
        for tap in &mut self.taps {
            let mut ring = tap.ring.lock().unwrap();
            if ring.total != tap.last_total {
                tap.last_total = ring.total;
                tap.last_data = Instant::now();
                continue;
            }
            let gap = tap.last_data.elapsed();
            if gap > Duration::from_millis(60) {
                let n = ((gap.as_secs_f32() * ring.sample_rate as f32) as usize).min(CAPACITY);
                for _ in 0..n {
                    ring.push(0.0, 0.0);
                }
                tap.last_total = ring.total;
                tap.last_data = Instant::now();
            }
        }
    }

    /// The newest `n` samples of a channel, all devices added together.
    pub fn read_last(&self, n: usize, channel: Channel, out: &mut Vec<f32>) {
        out.clear();
        out.resize(n, 0.0);
        for tap in &self.taps {
            tap.ring.lock().unwrap().add_last(channel, out);
        }
    }

    /// The mono samples that arrived since the last call.
    pub fn read_fresh(&mut self, out: &mut Vec<f32>) {
        let total = self.taps.first().map(|t| t.ring.lock().unwrap().total).unwrap_or(0);
        let fresh = (total.saturating_sub(self.consumed) as usize).min(16_384);
        self.consumed = total;
        self.read_last(fresh, Channel::Mono, out);
    }

    pub fn status(&mut self) -> Vec<TapStatus> {
        self.taps
            .iter()
            .map(|tap| {
                let mut ring = tap.ring.lock().unwrap();
                let peaks = std::mem::take(&mut ring.peaks);
                // On Android the Java side says what the source is doing.
                #[cfg(target_os = "android")]
                let format = Some(crate::android::status()).filter(|s| !s.is_empty()).unwrap_or_else(|| tap.format.clone());
                #[cfg(not(target_os = "android"))]
                let format = tap.format.clone();
                TapStatus {
                    name: tap.name.clone(),
                    format,
                    receiving: tap.last_data.elapsed() < Duration::from_millis(500) && ring.total > 0,
                    peaks_db: peaks[..tap.channels.min(MAX_METERED_CHANNELS)]
                        .iter()
                        .map(|p| 20.0 * p.max(1e-6).log10())
                        .collect(),
                }
            })
            .collect()
    }
}

impl Tap {
    fn new(name: String, format: String, channels: usize, ring: Arc<Mutex<Ring>>, stream: Option<cpal::Stream>) -> Self {
        Self { name, format, channels, ring, _stream: stream, last_total: 0, last_data: Instant::now() }
    }

    /// Opening an input stream on an output device captures what it plays.
    #[cfg(not(target_os = "android"))]
    fn open(device: cpal::Device, is_output: bool) -> Result<Self, String> {
        let name = device_name(&device);
        let config = if is_output { device.default_output_config() } else { device.default_input_config() }
            .map_err(|e| format!("{name}: {e}"))?;
        let channels = config.channels() as usize;
        let ring = Arc::new(Mutex::new(Ring::new(config.sample_rate())));
        let on_error = |e| eprintln!("audio stream error: {e}");
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => {
                let ring = ring.clone();
                device.build_input_stream(
                    config.config(),
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        let mut ring = ring.lock().unwrap();
                        for frame in data.chunks_exact(channels) {
                            ring.push_frame(frame);
                        }
                    },
                    on_error,
                    None,
                )
            }
            cpal::SampleFormat::I16 => {
                let ring = ring.clone();
                let mut frame_f32 = vec![0.0f32; channels];
                device.build_input_stream(
                    config.config(),
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        let mut ring = ring.lock().unwrap();
                        for frame in data.chunks_exact(channels) {
                            for (f, s) in frame_f32.iter_mut().zip(frame) {
                                *f = *s as f32 / 32768.0;
                            }
                            ring.push_frame(&frame_f32);
                        }
                    },
                    on_error,
                    None,
                )
            }
            other => return Err(format!("{name}: unsupported sample format {other:?}")),
        }
        .map_err(|e| format!("{name}: {e}"))?;
        stream.play().map_err(|e| format!("{name}: {e}"))?;
        let format = format!("{channels} ch, {} Hz, {:?}", config.sample_rate(), config.sample_format());
        Ok(Self::new(name, format, channels, ring, Some(stream)))
    }
}

/// A repeating bar of kick, sustained chord and hi-hat, for checking the
/// picture without any audio playing. Stops when the ring is dropped.
fn spawn_test_signal(ring: std::sync::Weak<Mutex<Ring>>) {
    std::thread::spawn(move || {
        let sr = 48_000.0f32;
        let start = Instant::now();
        let mut written: u64 = 0;
        let mut noise: u32 = 0x1234_5678;
        let mut kick_phase = 0.0f32;
        while let Some(ring) = ring.upgrade() {
            let target = (start.elapsed().as_secs_f32() * sr) as u64;
            {
                let mut ring = ring.lock().unwrap();
                while written < target {
                    let t = written as f32 / sr;
                    let beat = t % 0.5;
                    let kick_freq = 45.0 + 90.0 * (-beat * 30.0).exp();
                    kick_phase += std::f32::consts::TAU * kick_freq / sr;
                    let kick = 0.7 * kick_phase.sin() * (-beat * 9.0).exp();
                    let chord: f32 = [220.0f32, 277.18, 329.63, 440.0, 880.0]
                        .iter()
                        .map(|f| 0.05 * (std::f32::consts::TAU * f * t).sin())
                        .sum();
                    noise = noise.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    let hat_t = (t + 0.25) % 0.25;
                    let hat = 0.12 * ((noise >> 8) as f32 / 8_388_608.0 - 1.0) * (-hat_t * 60.0).exp();
                    // A bass line: a new low note each second, with a second harmonic.
                    let note = [41.2f32, 49.0, 55.0, 61.74][(t as usize) % 4];
                    let bass = 0.22 * (std::f32::consts::TAU * note * t).sin() + 0.08 * (std::f32::consts::TAU * 2.0 * note * t).sin();
                    let kick = 0.6 * kick + bass;
                    let pan = (t * 0.7).sin() * 0.5;
                    ring.push_frame(&[kick + chord * (1.0 - pan) + hat, kick + chord * (1.0 + pan) - hat]);
                    written += 1;
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    });
}
