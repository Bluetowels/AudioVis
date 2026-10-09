"""Offline preview of mapping A (pitch spiral tunnel, continuous surface).

    python render.py --start 60 --dur 30 --out out/preview_A_30s.mp4
"""
import argparse
import subprocess
import time
from pathlib import Path

import moderngl
import numpy as np

import analysis

HERE = Path(__file__).resolve().parent
HISTORY_S = 4.0

# Geometry: r = R0 + K * octave, z = -V * age.
R0, K, V = 0.30, 0.20, 2.0
SUBDIV = 3

# Per-mode display settings. "v1" is the first preview (history surface only).
# "push" and "thick" add the newest slice as a bold line and dim the history;
# their tilt and range match a SPAN-style analyser, and the camera sits a
# little further back so the widened outer ring stays in frame.
MODES = {
    "v1":    dict(tilt_db=2.0, range_db=40.0, history_exposure=1.0, line=0, eye_z=1.55),
    "push":  dict(tilt_db=4.5, range_db=60.0, history_exposure=0.30, line=1, eye_z=1.85),
    "thick": dict(tilt_db=4.5, range_db=60.0, history_exposure=0.30, line=2, eye_z=1.85),
}
LINE_PUSH = 0.90 * K        # push: radial travel at full loudness
LINE_HALF_WIDTH = 0.42 * K  # thick: half-width at full loudness

SCENE_VS = """
#version 330
uniform sampler2D feat;      // x = bin, y = slice age index (0 = newest)
uniform mat4 mvp;
uniform float frac_age;      // seconds since the newest slice
uniform float slice_dt;
uniform vec3 geom;           // r0, k, v
uniform int n_bins;
uniform float tilt_per_octave;   // display-only loudness tilt, in loudness units
in vec2 in_grid;             // (bin, age index); bin is fractional between analysis bins
out vec3 v_feat;
out float v_age;
out float v_tilt;
void main() {
    v_tilt = tilt_per_octave * in_grid.x / 36.0;
    int b = int(floor(in_grid.x));
    int row = int(in_grid.y);
    v_feat = mix(texelFetch(feat, ivec2(b, row), 0).rgb,
                 texelFetch(feat, ivec2(min(b + 1, n_bins - 1), row), 0).rgb, fract(in_grid.x));
    v_age = frac_age + in_grid.y * slice_dt;
    float p = in_grid.x / 3.0;                       // semitones above A0
    float theta = 6.28318530718 * (p - 3.0) / 12.0;  // C at 12 o'clock
    float r = geom.x + geom.y * p / 12.0;
    gl_Position = mvp * vec4(r * sin(theta), r * cos(theta), -geom.z * v_age, 1.0);
}
"""

SCENE_FS = """
#version 330
uniform vec2 loud_range;
uniform float history;
uniform float exposure;
in vec3 v_feat;
in float v_age;
in float v_tilt;
out vec4 f_color;
void main() {
    float a = smoothstep(loud_range.x, loud_range.y, v_feat.x + v_tilt);
    float inten = a * a * a;
    vec3 warm = vec3(1.00, 0.33, 0.07);
    vec3 cool = vec3(0.08, 0.62, 1.00);
    vec3 col = mix(warm, cool, smoothstep(0.18, 0.42, v_feat.y));
    float fade = (1.0 - smoothstep(history * 0.7, history, v_age)) * mix(1.0, 0.55, v_age / history);
    float rim = 1.0 + 2.0 * exp(-v_age / 0.06);       // marks "now"
    float flash = 1.0 + 2.5 * v_feat.z;
    f_color = vec4(min(col * inten * fade * rim * flash * exposure, vec3(8.0)), 1.0);
}
"""

# The newest slice as a ribbon in the z = 0 plane: two vertices per column,
# in_col = (bin, side) with side 0 = inner edge, 1 = outer edge.
LINE_VS = """
#version 330
uniform sampler2D feat;
uniform mat4 mvp;
uniform vec3 geom;
uniform int n_bins;
uniform float tilt_per_octave;
uniform vec2 loud_range;
uniform int line_mode;       // 1 = push outward, 2 = thickness
uniform vec2 line_size;      // push travel, thick half-width
in vec2 in_col;
out vec3 v_feat;
out float v_level;
out float v_side;
void main() {
    int b = int(floor(in_col.x));
    v_feat = mix(texelFetch(feat, ivec2(b, 0), 0).rgb,
                 texelFetch(feat, ivec2(min(b + 1, n_bins - 1), 0), 0).rgb, fract(in_col.x));
    float tilted = v_feat.x + tilt_per_octave * in_col.x / 36.0;
    v_level = clamp((tilted - loud_range.x) / (loud_range.y - loud_range.x), 0.0, 1.0);
    v_side = in_col.y;
    float p = in_col.x / 3.0;
    float theta = 6.28318530718 * (p - 3.0) / 12.0;
    float r = geom.x + geom.y * p / 12.0;
    const float hair = 0.004;   // quiet bins keep a hairline so the spiral stays visible
    float travel = v_level * v_level;   // still a 60 dB range, but peaks stand clear of the bed
    if (line_mode == 1) r += in_col.y * (hair + line_size.x * travel);
    else r += (in_col.y * 2.0 - 1.0) * (hair + line_size.y * travel);
    gl_Position = mvp * vec4(r * sin(theta), r * cos(theta), 0.0, 1.0);
}
"""

LINE_FS = """
#version 330
uniform int line_mode;
uniform float exposure;
in vec3 v_feat;
in float v_level;
in float v_side;
out vec4 f_color;
void main() {
    vec3 warm = vec3(1.00, 0.33, 0.07);
    vec3 cool = vec3(0.08, 0.62, 1.00);
    vec3 col = mix(warm, cool, smoothstep(0.18, 0.42, v_feat.y));
    float shape, bright;
    if (line_mode == 1) {    // filled from the ring up to a bright top edge
        shape = 0.22 + 0.78 * pow(v_side, 5.0);
        bright = 0.25 + 0.75 * v_level;
    } else {                 // soft-edged band, brighter as well as wider when loud
        float d = abs(v_side * 2.0 - 1.0);
        shape = 1.0 - d * d;
        bright = 0.10 + 0.90 * v_level * v_level;
    }
    float flash = 1.0 + 1.5 * v_feat.z;
    f_color = vec4(col * shape * bright * flash * exposure * 2.2, 1.0);
}
"""

QUAD_VS = """
#version 330
in vec2 in_pos;
out vec2 uv;
void main() { uv = in_pos * 0.5 + 0.5; gl_Position = vec4(in_pos, 0.0, 1.0); }
"""

BRIGHT_FS = """
#version 330
uniform sampler2D src;
uniform float threshold;
in vec2 uv;
out vec4 f_color;
void main() {
    vec3 c = clamp(textureLod(src, uv, 2.0).rgb, vec3(0.0), vec3(64.0));
    float l = max(c.r, max(c.g, c.b));
    f_color = vec4(c * max(l - threshold, 0.0) / max(l, 1e-4), 1.0);
}
"""

BLUR_FS = """
#version 330
uniform sampler2D src;
uniform vec2 step_uv;
in vec2 uv;
out vec4 f_color;
void main() {
    const float w[5] = float[](0.227027, 0.194595, 0.121622, 0.054054, 0.016216);
    vec3 c = texture(src, uv).rgb * w[0];
    for (int i = 1; i < 5; i++) {
        c += texture(src, uv + step_uv * float(i)).rgb * w[i];
        c += texture(src, uv - step_uv * float(i)).rgb * w[i];
    }
    f_color = vec4(c, 1.0);
}
"""

COMPOSITE_FS = """
#version 330
uniform sampler2D hdr;
uniform sampler2D bloom;
uniform float bloom_strength;
in vec2 uv;
out vec4 f_color;
vec3 filmic(vec3 x) {   // ACES fit
    return clamp(x * (2.51 * x + 0.03) / (x * (2.43 * x + 0.59) + 0.14), 0.0, 1.0);
}
void main() {
    vec2 st = vec2(uv.x, 1.0 - uv.y);   // rows leave top-first for ffmpeg
    vec3 c = min(texture(hdr, st).rgb, vec3(64.0)) + bloom_strength * texture(bloom, st).rgb;
    f_color = vec4(pow(filmic(c), vec3(1.0 / 2.2)), 1.0);
}
"""


def perspective(fovy_deg, aspect, near, far):
    f = 1.0 / np.tan(np.radians(fovy_deg) / 2)
    m = np.zeros((4, 4), dtype=np.float32)
    m[0, 0], m[1, 1] = f / aspect, f
    m[2, 2], m[2, 3] = (far + near) / (near - far), 2 * far * near / (near - far)
    m[3, 2] = -1.0
    return m


def look_at(eye, target, up=(0.0, 1.0, 0.0)):
    eye, target, up = (np.asarray(a, dtype=np.float32) for a in (eye, target, up))
    f = target - eye
    f /= np.linalg.norm(f)
    s = np.cross(f, up)
    s /= np.linalg.norm(s)
    u = np.cross(s, f)
    m = np.eye(4, dtype=np.float32)
    m[0, :3], m[1, :3], m[2, :3] = s, u, -f
    m[:3, 3] = -m[:3, :3] @ eye
    return m


def camera_mvp(aspect, eye_z):
    # Fixed, slightly right of and above the axis, in front of the newest slice.
    r_outer = R0 + K * analysis.OCTAVES
    eye = (0.50 * r_outer, 0.30 * r_outer, eye_z * r_outer)
    target = (0.0, 0.0, -0.22 * V * HISTORY_S)
    return perspective(64.0, aspect, 0.1, 100.0) @ look_at(eye, target)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--audio", default=None, help="defaults to the only mp3 in the project folder")
    ap.add_argument("--start", type=float, default=60.0)
    ap.add_argument("--dur", type=float, default=30.0)
    ap.add_argument("--size", default="1920x1080")
    ap.add_argument("--fps", type=int, default=60)
    ap.add_argument("--mode", choices=sorted(MODES), default="v1")
    ap.add_argument("--out", default="out/preview_A_30s.mp4")
    ap.add_argument("--exposure", type=float, default=0.55)
    args = ap.parse_args()

    mode = MODES[args.mode]
    audio = Path(args.audio) if args.audio else next(HERE.parent.glob("*.mp3"))
    width, height = (int(v) for v in args.size.split("x"))
    out_path = (HERE / args.out).resolve()
    out_path.parent.mkdir(parents=True, exist_ok=True)

    # Analysis covers the excerpt plus enough lead-in to fill the history.
    a_start = max(0.0, args.start - HISTORY_S - 1.0)
    t0 = time.perf_counter()
    feats, meta = analysis.analyse(audio, a_start, args.start + args.dur - a_start, HERE / "out" / "cache")
    print(f"analysis: {feats.shape[0]} slices x {feats.shape[1]} bins in {time.perf_counter() - t0:.1f} s")

    slice_dt = meta["hop"] / meta["sr"]
    n_hist = int(np.ceil(HISTORY_S / slice_dt)) + 1
    n_bins = feats.shape[1]

    # Display only: a treble tilt so the bass does not set the whole range,
    # then top = loudest bins of the excerpt, bottom range_db below.
    tilt = mode["tilt_db"] / -analysis.FLOOR_DB
    tilted = feats[..., 0].astype(np.float32) + tilt * np.arange(n_bins) / analysis.BINS_PER_OCTAVE
    hi = float(np.percentile(tilted, 99.9))
    loud_range = (hi - mode["range_db"] / -analysis.FLOOR_DB, hi)

    ctx = moderngl.create_standalone_context(require=330)
    print("renderer:", ctx.info["GL_RENDERER"])

    # Vertices are subdivided between bins so the rings are not visibly faceted.
    n_cols = (n_bins - 1) * SUBDIV + 1
    grid = np.stack(np.meshgrid(np.arange(n_cols) / SUBDIV, np.arange(n_hist)), axis=-1).astype("f4")
    i = (np.arange(n_hist - 1)[:, None] * n_cols + np.arange(n_cols - 1)[None, :]).ravel()
    index = np.stack([i, i + 1, i + n_cols, i + 1, i + n_cols + 1, i + n_cols], axis=-1).astype("i4")

    scene = ctx.program(vertex_shader=SCENE_VS, fragment_shader=SCENE_FS)
    scene_vao = ctx.vertex_array(scene, [(ctx.buffer(grid.tobytes()), "2f", "in_grid")],
                                 index_buffer=ctx.buffer(index.tobytes()))
    feat_tex = ctx.texture((n_bins, n_hist), 3, dtype="f2")
    feat_tex.filter = (moderngl.NEAREST, moderngl.NEAREST)
    scene["mvp"].write(camera_mvp(width / height, mode["eye_z"]).T.tobytes())
    scene["slice_dt"].value = slice_dt
    scene["n_bins"].value = n_bins
    scene["geom"].value = (R0, K, V)
    scene["loud_range"].value = loud_range
    scene["tilt_per_octave"].value = tilt
    scene["history"].value = HISTORY_S
    scene["exposure"].value = args.exposure * mode["history_exposure"]

    line_vao = None
    if mode["line"]:
        cols = np.repeat(np.arange(n_cols) / SUBDIV, 2)
        sides = np.tile([0.0, 1.0], n_cols)
        line = ctx.program(vertex_shader=LINE_VS, fragment_shader=LINE_FS)
        line_vao = ctx.vertex_array(
            line, [(ctx.buffer(np.stack([cols, sides], axis=-1).astype("f4").tobytes()), "2f", "in_col")])
        line["mvp"].write(camera_mvp(width / height, mode["eye_z"]).T.tobytes())
        line["geom"].value = (R0, K, V)
        line["n_bins"].value = n_bins
        line["tilt_per_octave"].value = tilt
        line["loud_range"].value = loud_range
        line["line_mode"].value = mode["line"]
        line["line_size"].value = (LINE_PUSH, LINE_HALF_WIDTH)
        line["exposure"].value = args.exposure

    # 32-bit float here: where the walls are seen exactly edge-on, thousands of
    # additive layers land on one pixel and overflow a 16-bit float target.
    msaa = ctx.framebuffer([ctx.renderbuffer((width, height), 4, samples=4, dtype="f4")])
    hdr = ctx.texture((width, height), 4, dtype="f4")
    hdr.filter = (moderngl.LINEAR_MIPMAP_LINEAR, moderngl.LINEAR)
    hdr.repeat_x = hdr.repeat_y = False
    hdr_fbo = ctx.framebuffer([hdr])

    bw, bh = width // 4, height // 4
    bloom_tex = [ctx.texture((bw, bh), 4, dtype="f2") for _ in range(2)]
    for t in bloom_tex:
        t.repeat_x = t.repeat_y = False
    bloom_fbo = [ctx.framebuffer([t]) for t in bloom_tex]
    out_fbo = ctx.framebuffer([ctx.texture((width, height), 3)])

    quad = ctx.buffer(np.array([-1, -1, 1, -1, -1, 1, 1, 1], dtype="f4").tobytes())
    bright = ctx.program(vertex_shader=QUAD_VS, fragment_shader=BRIGHT_FS)
    blur = ctx.program(vertex_shader=QUAD_VS, fragment_shader=BLUR_FS)
    comp = ctx.program(vertex_shader=QUAD_VS, fragment_shader=COMPOSITE_FS)
    bright_vao, blur_vao, comp_vao = (
        ctx.vertex_array(p, [(quad, "2f", "in_pos")]) for p in (bright, blur, comp))
    bright["threshold"].value = 0.6
    comp["hdr"].value, comp["bloom"].value = 0, 1
    comp["bloom_strength"].value = 0.35

    encoder = subprocess.Popen(
        ["ffmpeg", "-y", "-v", "error",
         "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", f"{width}x{height}", "-r", str(args.fps), "-i", "-",
         "-ss", f"{args.start:.6f}", "-t", f"{args.dur:.6f}", "-i", str(audio),
         "-map", "0:v:0", "-map", "1:a:0",
         "-c:v", "libx264", "-preset", "fast", "-crf", "20", "-pix_fmt", "yuv420p",
         "-c:a", "aac", "-b:a", "256k", "-shortest", "-movflags", "+faststart", str(out_path)],
        stdin=subprocess.PIPE)

    n_frames = int(round(args.dur * args.fps))
    window = np.zeros((n_hist, n_bins, 3), dtype=np.float16)
    frame_bytes = bytearray(width * height * 3)
    t0 = time.perf_counter()
    for n in range(n_frames):
        t = args.start + n / args.fps - a_start       # seconds into the analysis
        newest = min(int(t / slice_dt), feats.shape[0] - 1)
        take = min(n_hist, newest + 1)
        window[:take] = feats[newest::-1][:take]
        window[take:] = 0
        feat_tex.write(window.tobytes())
        scene["frac_age"].value = t - newest * slice_dt

        msaa.use()
        ctx.clear(0.0, 0.0, 0.0, 1.0)
        ctx.enable(moderngl.BLEND)
        ctx.disable(moderngl.DEPTH_TEST | moderngl.CULL_FACE)
        ctx.blend_func = moderngl.ONE, moderngl.ONE
        feat_tex.use(0)
        scene_vao.render(moderngl.TRIANGLES)
        if line_vao:
            line_vao.render(moderngl.TRIANGLE_STRIP)
        ctx.disable(moderngl.BLEND)
        ctx.copy_framebuffer(hdr_fbo, msaa)
        hdr.build_mipmaps()

        bloom_fbo[0].use()
        hdr.use(0)
        bright_vao.render(moderngl.TRIANGLE_STRIP)
        for radius in (1.0, 2.5):
            for src, dst, step in ((0, 1, (radius / bw, 0.0)), (1, 0, (0.0, radius / bh))):
                bloom_fbo[dst].use()
                bloom_tex[src].use(0)
                blur["step_uv"].value = step
                blur_vao.render(moderngl.TRIANGLE_STRIP)

        out_fbo.use()
        hdr.use(0)
        bloom_tex[0].use(1)
        comp_vao.render(moderngl.TRIANGLE_STRIP)
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
