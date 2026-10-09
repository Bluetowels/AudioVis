"""Offline 2D previews: the whole frame is the spectrum, colour is amplitude.

    python render2d.py --mode grid
    python render2d.py --mode cross --bass --mirror

grid:  x = note within the octave (A at the left edge), y = octave (bass at
       the bottom). Octaves of a note stack in a vertical column.
cross: x = frequency and y = frequency (both low to high). A pixel lights
       when both of its frequencies are sounding.

Silence renders pure black; there is no fixed geometry.
"""
import argparse
import subprocess
import time
from pathlib import Path

import moderngl
import numpy as np
from scipy.signal import butter, sosfiltfilt

import analysis

HERE = Path(__file__).resolve().parent
TILT_DB_PER_OCTAVE = 4.5
RANGE_DB = 60.0

# Bass drive: the level of everything below BASS_HZ, measured on the waveform
# (a 23 ms window, so it is far quicker than the 200 ms bass analysis bins),
# with instant attack and a short release. It scales the whole picture.
BASS_HZ = 150.0
BASS_WINDOW = 1024
BASS_RELEASE_S = 0.06
BASS_RANGE_DB = 30.0
BASS_GAIN = (0.50, 1.45)    # picture brightness with no bass / at a full hit
BASS_LIFT = 0.30

SOFT_FLOOR = 0.35           # soft combine: how far one frequency alone lights its row/column

QUAD_VS = """
#version 330
in vec2 in_pos;
out vec2 uv;
void main() { uv = in_pos * 0.5 + 0.5; gl_Position = vec4(in_pos, 0.0, 1.0); }
"""

# Pass 1: the brightness of every pixel, 0..1, with optional persistence.
FIELD_FS = """
#version 330
uniform sampler2D level;     // grid: 36 x 9 (note, octave); cross: 324 x 2 (x axis row, y axis row)
uniform sampler2D previous;  // last frame's output of this pass
uniform int mode;            // 0 = grid, 1 = cross
uniform int mirror;          // cross only: fold both axes so high frequencies meet in the centre
uniform int flip_y;          // cross only: reverse the vertical frequency axis (within each quadrant)
uniform int soft;            // cross only: one loud frequency lights its whole row and column
uniform float soft_floor;
uniform float gain;          // overall brightness, driven by bass energy (1 = off)
uniform float lift;          // on a bass hit, rows and columns of sounding bins glow
uniform float keep;          // fraction of the previous frame that persists (0 = none)
in vec2 uv;
out float f_value;

// Bilinear lookup with eased weights, so cells blend without visible creases.
float smooth_level(vec2 st, vec2 size) {
    vec2 p = st * size - 0.5;
    vec2 i = floor(p), f = p - i;
    f = f * f * (3.0 - 2.0 * f);
    return texture(level, (i + 0.5 + f) / size).r;
}

void main() {
    float v, reach;
    if (mode == 0) {
        v = smooth_level(uv, vec2(36.0, 9.0));
        reach = v;
    } else {
        vec2 f = uv;                                     // low to high, from the bottom-left corner
        if (mirror == 1) f = 1.0 - abs(2.0 * f - 1.0);   // low at every edge, high in the centre
        if (flip_y == 1) f.y = 1.0 - f.y;
        float a = smooth_level(vec2(f.x, 0.25), vec2(324.0, 2.0));
        float b = smooth_level(vec2(f.y, 0.75), vec2(324.0, 2.0));
        reach = max(a, b);
        v = soft == 1 ? reach * mix(soft_floor, 1.0, min(a, b)) : a * b;
    }
    v = v * gain + lift * reach * (1.0 - v);
    f_value = max(clamp(v, 0.0, 1.0), keep * texture(previous, uv).r);
}
"""

# Pass 2: colour.
COLOUR_FS = """
#version 330
uniform sampler2D value;
in vec2 uv;
out vec4 f_color;

vec3 ramp(float t) {         // black -> violet -> red -> orange -> pale yellow
    const vec3 c1 = vec3(0.16, 0.03, 0.42);
    const vec3 c2 = vec3(0.80, 0.10, 0.30);
    const vec3 c3 = vec3(1.00, 0.55, 0.05);
    const vec3 c4 = vec3(1.00, 0.98, 0.80);
    if (t < 0.25) return mix(vec3(0.0), c1, t * 4.0);
    if (t < 0.50) return mix(c1, c2, (t - 0.25) * 4.0);
    if (t < 0.75) return mix(c2, c3, (t - 0.50) * 4.0);
    return mix(c3, c4, (t - 0.75) * 4.0);
}

void main() {
    // Rows leave top-first for ffmpeg, so the video's top row is uv.y = 1.
    float v = texture(value, vec2(uv.x, 1.0 - uv.y)).r;
    f_color = vec4(ramp(pow(v, 1.3)), 1.0);
}
"""


def bass_drive(audio, start, duration, sr, times):
    """Bass level 0..1 at each of `times` (seconds from `start`)."""
    x = analysis.load_mono(audio, start, duration, sr).astype(np.float64)
    low = sosfiltfilt(butter(4, BASS_HZ, btype="low", fs=sr, output="sos"), x)
    power = np.concatenate([[0.0], np.cumsum(low * low)])
    ends = np.clip((np.asarray(times) * sr).astype(int), BASS_WINDOW, len(x))
    rms = np.sqrt((power[ends] - power[ends - BASS_WINDOW]) / BASS_WINDOW)
    db = 20.0 * np.log10(np.maximum(rms, 1e-9))
    e = np.clip((db - (np.percentile(db, 99.5) - BASS_RANGE_DB)) / BASS_RANGE_DB, 0.0, 1.0)
    decay = np.exp(-(times[1] - times[0]) / BASS_RELEASE_S)
    for i in range(1, len(e)):
        e[i] = max(e[i], e[i - 1] * decay)
    return e


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--audio", default=None, help="defaults to the only mp3 in the project folder")
    ap.add_argument("--mode", choices=["grid", "cross"], default="grid")
    ap.add_argument("--start", type=float, default=60.0)
    ap.add_argument("--dur", type=float, default=30.0)
    ap.add_argument("--size", default="1920x1080")
    ap.add_argument("--fps", type=int, default=60)
    ap.add_argument("--mirror", action="store_true",
                    help="cross only: the picture becomes the bottom-left quadrant, mirrored into the other three")
    ap.add_argument("--flip-y", action="store_true",
                    help="cross only: reverse the vertical frequency axis (within each quadrant when mirrored)")
    ap.add_argument("--soft", action="store_true",
                    help="cross only: softer combine, one loud frequency lights its whole row and column")
    ap.add_argument("--stereo", action="store_true",
                    help="cross only: left channel on x, right channel on y")
    ap.add_argument("--decay-ms", type=float, default=0.0,
                    help="per-pixel persistence: time for a pixel to fade to 37%% (0 = none)")
    ap.add_argument("--bass", action="store_true", help="bass hits drive the brightness of the whole frame")
    ap.add_argument("--out", default=None)
    args = ap.parse_args()

    audio = Path(args.audio) if args.audio else next(HERE.parent.glob("*.mp3"))
    width, height = (int(v) for v in args.size.split("x"))
    out_path = (HERE / (args.out or f"out/preview_2D_{args.mode}_30s.mp4")).resolve()
    out_path.parent.mkdir(parents=True, exist_ok=True)

    a_start = max(0.0, args.start - 1.0)
    a_dur = args.start + args.dur - a_start
    cache = HERE / "out" / "cache"
    channels = (0, 1) if args.stereo and args.mode == "cross" else (None,)
    results = [analysis.analyse(audio, a_start, a_dur, cache, channel=c) for c in channels]
    meta = results[0][1]
    slice_dt = meta["hop"] / meta["sr"]
    n_bins = results[0][0].shape[1]

    # Level 0..1 over RANGE_DB below the loudest bins of the excerpt, after a
    # treble tilt. Anything below the range, including silence, is exactly 0.
    # Shape (sources, slices, bins); both stereo channels share one reference.
    tilt = TILT_DB_PER_OCTAVE / -analysis.FLOOR_DB
    loud = np.stack([feats[..., 0].astype(np.float32) for feats, _ in results])
    tilted = loud + tilt * np.arange(n_bins) / analysis.BINS_PER_OCTAVE
    hi = float(np.percentile(tilted, 99.9))
    span = RANGE_DB / -analysis.FLOOR_DB
    level = np.where(loud > 0.0, np.clip((tilted - (hi - span)) / span, 0.0, 1.0), 0.0).astype("f4")

    ctx = moderngl.create_standalone_context(require=330)
    print("renderer:", ctx.info["GL_RENDERER"])
    tex_size = (analysis.BINS_PER_OCTAVE, analysis.OCTAVES) if args.mode == "grid" else (n_bins, 2)
    tex = ctx.texture(tex_size, 1, dtype="f4")
    tex.filter = (moderngl.LINEAR, moderngl.LINEAR)
    tex.repeat_x = tex.repeat_y = False

    quad = ctx.buffer(np.array([-1, -1, 1, -1, -1, 1, 1, 1], dtype="f4").tobytes())
    field = ctx.program(vertex_shader=QUAD_VS, fragment_shader=FIELD_FS)
    colour = ctx.program(vertex_shader=QUAD_VS, fragment_shader=COLOUR_FS)
    field_vao = ctx.vertex_array(field, [(quad, "2f", "in_pos")])
    colour_vao = ctx.vertex_array(colour, [(quad, "2f", "in_pos")])
    field["level"].value, field["previous"].value = 0, 1
    field["mode"].value = 0 if args.mode == "grid" else 1
    field["mirror"].value = 1 if args.mirror else 0
    field["flip_y"].value = 1 if args.flip_y else 0
    field["soft"].value = 1 if args.soft else 0
    field["soft_floor"].value = SOFT_FLOOR
    field["keep"].value = np.exp(-1000.0 / args.fps / args.decay_ms) if args.decay_ms > 0 else 0.0

    # Two brightness targets, swapped each frame, so a frame can read the last one.
    values = [ctx.texture((width, height), 1, dtype="f2") for _ in range(2)]
    for v in values:
        v.filter = (moderngl.NEAREST, moderngl.NEAREST)
    value_fbos = [ctx.framebuffer([v]) for v in values]
    for f in value_fbos:
        f.clear(0.0, 0.0, 0.0, 0.0)
    out_fbo = ctx.framebuffer([ctx.texture((width, height), 3)])

    encoder = subprocess.Popen(
        ["ffmpeg", "-y", "-v", "error",
         "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", f"{width}x{height}", "-r", str(args.fps), "-i", "-",
         "-ss", f"{args.start:.6f}", "-t", f"{args.dur:.6f}", "-i", str(audio),
         "-map", "0:v:0", "-map", "1:a:0",
         "-c:v", "libx264", "-preset", "fast", "-crf", "18", "-pix_fmt", "yuv420p",
         "-c:a", "aac", "-b:a", "256k", "-shortest", "-movflags", "+faststart", str(out_path)],
        stdin=subprocess.PIPE)

    n_frames = int(round(args.dur * args.fps))
    drive = None
    if args.bass:
        drive = bass_drive(audio, a_start, a_dur, meta["sr"],
                           args.start - a_start + np.arange(n_frames) / args.fps)
        np.save(out_path.with_suffix(".bass.npy"), drive)
    frame_bytes = bytearray(width * height * 3)
    prev = -1
    t0 = time.perf_counter()
    for n in range(n_frames):
        t = args.start + n / args.fps - a_start
        newest = min(int(t / slice_dt), level.shape[1] - 1)
        # No averaging: the peak of the slices that arrived since the last
        # frame (about three at 60 fps), so no transient is skipped.
        now = level[:, max(prev + 1, newest - 3):newest + 1].max(axis=1)
        prev = newest
        if args.mode == "cross":
            now = now[[0, -1]]          # x axis row, y axis row (the same row twice in mono)
        tex.write(np.ascontiguousarray(now).tobytes())
        e = drive[n] if drive is not None else None
        field["gain"].value = 1.0 if e is None else BASS_GAIN[0] + (BASS_GAIN[1] - BASS_GAIN[0]) * e
        field["lift"].value = 0.0 if e is None else BASS_LIFT * e * e

        cur, last = n % 2, (n + 1) % 2
        value_fbos[cur].use()
        tex.use(0)
        values[last].use(1)
        field_vao.render(moderngl.TRIANGLE_STRIP)
        out_fbo.use()
        values[cur].use(0)
        colour_vao.render(moderngl.TRIANGLE_STRIP)
        out_fbo.read_into(frame_bytes, components=3)
        encoder.stdin.write(frame_bytes)

    encoder.stdin.close()
    render_s = time.perf_counter() - t0
    if encoder.wait() != 0:
        raise SystemExit("ffmpeg failed")
    print(f"rendered {n_frames} frames in {render_s:.1f} s = {n_frames / render_s:.1f} frames/s (including encode)")
    print("wrote", out_path)


if __name__ == "__main__":
    main()
