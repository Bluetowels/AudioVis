"""Variable-Q analysis for the preview renderer.

Produces, per slice (hop 256 samples) and per bin (36 per octave from A0),
three values in 0..1: loudness, tonality and onset strength.
"""
import hashlib
import json
import subprocess
from pathlib import Path

import numpy as np
import scipy.fft
from scipy.ndimage import maximum_filter1d, uniform_filter1d

F_MIN = 27.5            # A0
OCTAVES = 9
BINS_PER_OCTAVE = 36
N_BINS = OCTAVES * BINS_PER_OCTAVE
HOP = 256
MAX_WINDOW_S = 0.2      # bass window cap
FLOOR_DB = -80.0
PEAK_DB = 9.0          # peak-over-neighbours level that counts as fully tonal


def probe_sample_rate(path):
    out = subprocess.run(
        ["ffprobe", "-v", "error", "-select_streams", "a:0",
         "-show_entries", "stream=sample_rate", "-of", "csv=p=0", str(path)],
        capture_output=True, text=True, check=True).stdout
    return int(out.strip().split(",")[0])


def load_mono(path, start, duration, sr, channel=None):
    """Decode [start, start+duration) to mono float32 at the file's own rate.

    channel None mixes down to mono; 0 or 1 takes the left or right channel.
    """
    mix = ["-ac", "1"] if channel is None else ["-af", f"pan=mono|c0=c{channel}"]
    cmd = ["ffmpeg", "-v", "error", "-ss", f"{start:.6f}", "-t", f"{duration:.6f}",
           "-i", str(path), "-map", "0:a:0", *mix, "-ar", str(sr),
           "-f", "f32le", "-"]
    raw = subprocess.run(cmd, capture_output=True, check=True).stdout
    return np.frombuffer(raw, dtype=np.float32)


def bin_frequencies():
    return F_MIN * 2.0 ** (np.arange(N_BINS) / BINS_PER_OCTAVE)


def window_lengths(sr):
    q = 1.0 / (2.0 ** (1.0 / BINS_PER_OCTAVE) - 1.0)
    return np.minimum(q * sr / bin_frequencies(), MAX_WINDOW_S * sr).round().astype(int)


def vqt(x, sr):
    """Complex variable-Q transform, shape (slices, N_BINS).

    Bins are grouped by FFT size; each group is one STFT multiplied by a
    spectral kernel. Every window is centred on the slice time, so all bins
    line up in time. A full-scale sine at a bin centre gives magnitude 1.
    """
    freqs = bin_frequencies()
    lengths = window_lengths(sr)
    n_slices = 1 + len(x) // HOP
    out = np.zeros((n_slices, N_BINS), dtype=np.complex64)
    fft_sizes = 2 ** np.ceil(np.log2(lengths)).astype(int)

    for n in np.unique(fft_sizes):
        idx = np.nonzero(fft_sizes == n)[0]
        margin = int(np.ceil(8 * n / lengths[idx].min()))
        m_lo = max(0, int(freqs[idx].min() * n / sr) - margin)
        m_hi = min(n // 2 + 1, int(np.ceil(freqs[idx].max() * n / sr)) + margin + 1)
        kernel = np.zeros((len(idx), m_hi - m_lo), dtype=np.complex64)
        for row, b in enumerate(idx):
            length = lengths[b]
            w = np.hanning(length + 1)[:-1]
            k = np.zeros(n, dtype=np.complex128)
            s = (n - length) // 2
            t = np.arange(s, s + length) - n // 2
            k[s:s + length] = (2.0 * w / w.sum()) * np.exp(2j * np.pi * freqs[b] * t / sr)
            kernel[row] = np.conj(np.fft.fft(k)[m_lo:m_hi]) / n

        padded = np.pad(x, (n // 2, n // 2 + HOP))
        frames = np.lib.stride_tricks.sliding_window_view(padded, n)[::HOP][:n_slices]
        chunk = max(64, (1 << 25) // n)
        for a in range(0, n_slices, chunk):
            spec = scipy.fft.rfft(frames[a:a + chunk], axis=1, workers=-1)[:, m_lo:m_hi]
            out[a:a + chunk, idx] = spec.astype(np.complex64) @ kernel.T
    return out


def features(c, sr):
    """Loudness, tonality and onset in 0..1 from the complex transform."""
    db = 20.0 * np.log10(np.maximum(np.abs(c), 1e-10)).astype(np.float32)
    db = np.maximum(db, FLOOR_DB - 20.0)
    loud = np.clip((db - FLOOR_DB) / -FLOOR_DB, 0.0, 1.0)

    # Peakiness: bin level against the mean of its neighbours within about
    # +-2 semitones, leaving out the bin's own main lobe. Where the bass cap
    # widens the main lobe, both spans widen with it.
    lengths = window_lengths(sr)
    q = 1.0 / (2.0 ** (1.0 / BINS_PER_OCTAVE) - 1.0)
    blur = np.maximum(1.0, q * sr / bin_frequencies() / lengths)
    inner = np.minimum(np.round(blur).astype(int), 8)
    peak = np.zeros_like(db)
    for e in np.unique(inner):
        cols = np.nonzero(inner == e)[0]
        n_in, n_out = 2 * e + 1, 2 * (e + 5) + 1
        wide = uniform_filter1d(db, n_out, axis=1, mode="nearest")[:, cols] * n_out
        near = uniform_filter1d(db, n_in, axis=1, mode="nearest")[:, cols] * n_in
        top = maximum_filter1d(db, n_in, axis=1, mode="nearest")[:, cols]
        peak[:, cols] = np.clip((top - (wide - near) / (n_out - n_in)) / PEAK_DB, 0.0, 1.0)

    # Stability and flux are measured over half a window per bin, so heavily
    # overlapping bass windows are not mistaken for stable tones.
    lags = np.clip(np.round(lengths / HOP / 2).astype(int), 1, 16)
    change = np.zeros_like(db)
    flux = np.zeros_like(db)
    for lag in np.unique(lags):
        cols = np.nonzero(lags == lag)[0]
        d = np.zeros((db.shape[0], len(cols)), dtype=np.float32)
        d[lag:] = db[lag:, cols] - db[:-lag, cols]
        change[:, cols] = uniform_filter1d(np.abs(d), 2 * lag + 1, axis=0, mode="nearest")
        flux[:, cols] = np.maximum(d, 0.0) * loud[:, cols]
    stable = 1.0 - np.clip(change / 5.0, 0.0, 1.0)

    tonality = peak * (0.35 + 0.65 * stable)
    tonality = uniform_filter1d(tonality, 5, axis=0, mode="nearest")

    scale = np.percentile(flux, 99.9)
    onset = np.clip(flux / max(scale, 1e-6), 0.0, 1.0) ** 2
    return np.stack([loud, tonality, onset], axis=-1).astype(np.float16)


def analyse(path, start, duration, cache_dir, channel=None):
    """Return (features[slices, bins, 3], meta). Cached as .npy next to a .json."""
    path = Path(path)
    sr = probe_sample_rate(path)
    key = hashlib.sha1(
        f"{path.name}|{path.stat().st_size}|{start:.3f}|{duration:.3f}|{sr}|v5{"" if channel is None else f"|ch{channel}"}".encode()
    ).hexdigest()[:12]
    cache_dir = Path(cache_dir)
    cache_dir.mkdir(parents=True, exist_ok=True)
    npy, meta_path = cache_dir / f"vqt_{key}.npy", cache_dir / f"vqt_{key}.json"
    if npy.exists() and meta_path.exists():
        return np.load(npy), json.loads(meta_path.read_text())

    x = load_mono(path, start, duration, sr, channel)
    feats = features(vqt(x, sr), sr)
    meta = {"sr": sr, "hop": HOP, "start": start, "duration": duration,
            "bins": N_BINS, "bins_per_octave": BINS_PER_OCTAVE, "f_min": F_MIN}
    np.save(npy, feats)
    meta_path.write_text(json.dumps(meta, indent=2))
    return feats, meta
