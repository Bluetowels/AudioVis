//! Variable-Q spectrum of the most recent audio: 36 bins per octave over nine
//! octaves from A0, with bass windows capped at 200 ms. Every bin's window
//! ends at the newest sample, so nothing waits on the long bass windows.

pub use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};
use std::sync::Arc;

pub const F_MIN: f32 = 27.5;
pub const OCTAVES: usize = 9;
pub const BINS_PER_OCTAVE: usize = 36;
pub const N_BINS: usize = OCTAVES * BINS_PER_OCTAVE;
pub const SILENCE_DB: f32 = -140.0;

struct Group {
    n: usize,
    fft: Arc<dyn Fft<f32>>,
    buf: Vec<Complex32>,
    scratch: Vec<Complex32>,
}

struct Bin {
    group: usize,
    first: usize,
    kernel: Vec<Complex32>,
}

pub struct Vqt {
    pub sample_rate: u32,
    /// Samples needed by the longest window.
    pub max_n: usize,
    /// Longest analysis window in seconds; bass bins that want more are capped here.
    pub max_window_s: f32,
    /// Window length as a fraction of what full pitch detail needs (1 = full
    /// detail, 0.25 = four times quicker and four times blurrier).
    pub detail: f32,
    /// How many bins wide each bin's response is: 1 where the window is as
    /// long as the pitch resolution needs, more where it has been shortened.
    pub blur: Vec<f32>,
    /// The part of `blur` caused by the bass window cap alone.
    pub cap_blur: Vec<f32>,
    groups: Vec<Group>,
    bins: Vec<Bin>,
}

impl Vqt {
    pub fn new(sample_rate: u32, max_window_s: f32, detail: f32) -> Self {
        let sr = sample_rate as f64;
        let mut blur = Vec::with_capacity(N_BINS);
        let mut cap_blur = Vec::with_capacity(N_BINS);
        let q = 1.0 / (2f64.powf(1.0 / BINS_PER_OCTAVE as f64) - 1.0);
        let mut planner = FftPlanner::<f32>::new();
        let mut groups: Vec<Group> = Vec::new();
        let mut bins = Vec::with_capacity(N_BINS);

        for b in 0..N_BINS {
            let f = F_MIN as f64 * 2f64.powf(b as f64 / BINS_PER_OCTAVE as f64);
            let wanted = q * detail as f64 * sr / f;
            let len = (wanted.min(max_window_s as f64 * sr).round() as usize).max(32);
            blur.push((q * sr / f / len as f64).max(1.0) as f32);
            cap_blur.push((wanted / len as f64).max(1.0) as f32);
            let n = len.next_power_of_two();
            let group = match groups.iter().position(|g| g.n == n) {
                Some(i) => i,
                None => {
                    let fft = planner.plan_fft_forward(n);
                    let scratch = vec![Complex32::default(); fft.get_inplace_scratch_len()];
                    groups.push(Group { n, fft, buf: vec![Complex32::default(); n], scratch });
                    groups.len() - 1
                }
            };

            // Hann-windowed complex tone, right-aligned in the frame and scaled
            // so a full-scale sine at the bin centre reads 0 dB.
            let window: Vec<f64> = (0..len)
                .map(|j| 0.5 - 0.5 * (std::f64::consts::TAU * j as f64 / len as f64).cos())
                .collect();
            let sum: f64 = window.iter().sum();
            let mut k = vec![Complex32::default(); n];
            for (j, w) in window.iter().enumerate() {
                let phase = std::f64::consts::TAU * f * j as f64 / sr;
                let a = 2.0 * w / sum;
                k[n - len + j] = Complex32::new((a * phase.cos()) as f32, (a * phase.sin()) as f32);
            }
            groups[group].fft.process(&mut k);

            // Keep only the part of the kernel's spectrum around the bin.
            let centre = f * n as f64 / sr;
            let margin = 4.0 * n as f64 / len as f64 + 2.0;
            let first = (centre - margin).floor().max(0.0) as usize;
            let last = ((centre + margin).ceil() as usize).min(n / 2);
            let kernel = k[first..=last].iter().map(|c| c.conj() / n as f32).collect();
            bins.push(Bin { group, first, kernel });
        }

        let max_n = groups.iter().map(|g| g.n).max().unwrap_or(0);
        Self { sample_rate, max_n, max_window_s, detail, blur, cap_blur, groups, bins }
    }

    /// Level of each bin in dBFS from `samples` (newest last, at least `max_n` long).
    pub fn analyse(&mut self, samples: &[f32], out_db: &mut [f32]) {
        self.transform(samples);
        for (b, out) in out_db.iter_mut().enumerate() {
            *out = (20.0 * self.bin(b).norm().max(1e-7).log10()).max(SILENCE_DB);
        }
    }

    /// Each bin as a complex value (size and timing), for comparing channels.
    pub fn analyse_complex(&mut self, samples: &[f32], out: &mut [Complex32]) {
        self.transform(samples);
        for (b, out) in out.iter_mut().enumerate() {
            *out = self.bin(b);
        }
    }

    fn bin(&self, b: usize) -> Complex32 {
        let bin = &self.bins[b];
        let spectrum = &self.groups[bin.group].buf[bin.first..bin.first + bin.kernel.len()];
        spectrum.iter().zip(&bin.kernel).map(|(x, k)| x * k).sum()
    }

    fn transform(&mut self, samples: &[f32]) {
        for g in &mut self.groups {
            let tail = &samples[samples.len() - g.n..];
            for (c, s) in g.buf.iter_mut().zip(tail) {
                *c = Complex32::new(*s, 0.0);
            }
            g.fft.process_with_scratch(&mut g.buf, &mut g.scratch);
        }
    }
}

/// Replace each bump in the spectrum with a narrow line at its centre.
///
/// One note lights several neighbouring bins (many more where the bass window
/// cap smears it), but the centre of the bump is still the note. Each local
/// peak is located between bins and redrawn about one bin wide. `everywhere`
/// blends from the original (0) to the sharpened version (1) across the whole
/// spectrum; `bass` does the same only where the cap smears the bins.
pub fn sharpen(levels: &mut [f32], blur: &[f32], cap_blur: &[f32], bass: f32, everywhere: f32, scratch: &mut Vec<f32>) {
    if bass <= 0.0 && everywhere <= 0.0 {
        return;
    }
    let n = levels.len();
    let end = if everywhere > 0.0 {
        n
    } else {
        match cap_blur.iter().rposition(|b| *b > 1.02) {
            Some(last) => (last + 8).min(n),
            None => return,
        }
    };
    scratch.clear();
    scratch.resize(end, 0.0);

    for b in 0..end {
        let level = levels[b];
        if level <= 0.0 {
            continue;
        }
        // A peak is the highest bin within its own main lobe...
        let half = (2.0 * blur[b]).round().max(1.0) as usize;
        let (lo, hi) = (b.saturating_sub(half), (b + half).min(n - 1));
        if (lo..b).any(|i| levels[i] >= level) || (b + 1..=hi).any(|i| levels[i] > level) {
            continue;
        }
        // ...and not a side lobe of a much stronger neighbour (0.3 of the level
        // range is about 18 dB at the default 60 dB range).
        let wide = 2 * half;
        let strongest = levels[b.saturating_sub(wide)..=(b + wide).min(n - 1)].iter().copied().fold(0.0, f32::max);
        if level < strongest - 0.3 {
            continue;
        }
        // Parabola through the peak and its neighbours for the position between bins.
        let left = levels[b.saturating_sub(1)];
        let right = levels[(b + 1).min(n - 1)];
        let curve = left - 2.0 * level + right;
        let offset = if curve < 0.0 { (0.5 * (left - right) / curve).clamp(-0.5, 0.5) } else { 0.0 };
        let centre = b as f32 + offset;
        let (from, to) = (b.saturating_sub(3), (b + 3).min(end - 1));
        for (i, out) in scratch.iter_mut().enumerate().take(to + 1).skip(from) {
            let d = (i as f32 - centre) / 0.8;
            *out = out.max(level * (-0.5 * d * d).exp());
        }
    }

    for b in 0..end {
        let smeared = ((cap_blur[b] - 1.0) / 0.6).clamp(0.0, 1.0);
        levels[b] += (scratch[b] - levels[b]) * (bass * smeared).max(everywhere);
    }
}

/// Level of everything below a cutoff, measured on the waveform over a short
/// window so it reacts far faster than the bass bins of the spectrum.
pub struct BassMeter {
    sample_rate: f32,
    cutoff: f32,
    stages: [Biquad; 2],
    squares: Vec<f64>,
    pos: usize,
    sum: f64,
}

#[derive(Default, Clone, Copy)]
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    z: [f32; 2],
}

impl Biquad {
    fn lowpass(&mut self, sample_rate: f32, cutoff: f32, q: f32) {
        let w = std::f32::consts::TAU * cutoff / sample_rate;
        let alpha = w.sin() / (2.0 * q);
        let a0 = 1.0 + alpha;
        let b1 = (1.0 - w.cos()) / a0;
        self.b = [b1 / 2.0, b1, b1 / 2.0];
        self.a = [-2.0 * w.cos() / a0, (1.0 - alpha) / a0];
    }

    fn run(&mut self, x: f32) -> f32 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

impl BassMeter {
    /// Measurement window in seconds (1024 samples at 44.1 kHz).
    const WINDOW_S: f32 = 0.0232;

    pub fn new(sample_rate: u32) -> Self {
        let mut m = Self {
            sample_rate: sample_rate as f32,
            cutoff: 0.0,
            stages: [Biquad::default(); 2],
            squares: vec![0.0; (Self::WINDOW_S * sample_rate as f32) as usize],
            pos: 0,
            sum: 0.0,
        };
        m.set_cutoff(150.0);
        m
    }

    pub fn set_cutoff(&mut self, cutoff: f32) {
        if (cutoff - self.cutoff).abs() > 0.01 {
            self.cutoff = cutoff;
            // Fourth-order Butterworth as two second-order stages.
            self.stages[0].lowpass(self.sample_rate, cutoff, 0.541_196_1);
            self.stages[1].lowpass(self.sample_rate, cutoff, 1.306_563);
        }
    }

    /// Feed new samples; returns the current level in dBFS.
    pub fn feed(&mut self, samples: &[f32]) -> f32 {
        for &x in samples {
            let first = self.stages[0].run(x);
            let y = self.stages[1].run(first) as f64;
            self.sum += y * y - self.squares[self.pos];
            self.squares[self.pos] = y * y;
            self.pos = (self.pos + 1) % self.squares.len();
            if self.pos == 0 {
                self.sum = self.squares.iter().sum(); // stop rounding error building up
            }
        }
        let rms = (self.sum.max(0.0) / self.squares.len() as f64).sqrt() as f32;
        (20.0 * rms.max(1e-7).log10()).max(SILENCE_DB)
    }
}
