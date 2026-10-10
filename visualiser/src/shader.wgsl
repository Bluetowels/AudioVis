// The whole picture. Cross view: every pixel is a pair of frequencies (x, y)
// and its colour is how loud the two are together. Circle view: distance
// from the centre is frequency. With relief on, brightness is also height
// and the picture is seen from a tilted camera.

struct Uniforms {
    // mirror mode (0 off, 1 quadrants, 2 left/right, 3 top/bottom), flip x, flip y, combine blend
    shape: vec4<f32>,
    // soft-combine floor, bass gain, bass glow, contrast
    tone: vec4<f32>,
    // master brightness, lowest frequency (0..1), highest frequency (0..1), bin count
    view: vec4<f32>,
    // circle view on (1) or off (0), bass pulse strength, bass pulse radius, picture width / height
    circle: vec4<f32>,
    // bass colour (rgb) and the bass glow setting (a)
    accent: vec4<f32>,
    // number of palette colours, banding (0 smooth .. 1 hard steps), circle angular pattern,
    // bass style (0 pulse from the middle, 1 fill dark areas)
    extra: vec4<f32>,
    // stereo on (1) or off (0), stereo emphasis, surround colour amount,
    // 3D material (0 matte, 1 gloss, 2 metal, 3 glass)
    stereo: vec4<f32>,
    // colour for sound where left and right move against each other
    surround: vec4<f32>,
    // 3D view on (1) or off (0), height of full brightness, storm amount, time in seconds
    relief: vec4<f32>,
    // 3D camera position and the point it looks at (ground units; z is height)
    cam_eye: vec4<f32>,
    cam_target: vec4<f32>,
    // seconds since the last frame, number of raindrops in use, 3D material strength (0 matte .. 1 full), unused
    sim: vec4<f32>,
    // HDR output on (1) or off (0), base brightness and peak brightness in units of 80 nits, test pattern on (1)
    hdr: vec4<f32>,
    // bloom amount, background (0 black, 1 stars, 2 album cover), background brightness, treble level (0..1)
    post: vec4<f32>,
    // black, then up to eight colours from quiet to loud
    stops: array<vec4<f32>, 9>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
// Row 0: level per bin for the x axis. Row 1: for the y axis. Row 2: stereo
// position per bin (-1 left .. 1 right). Row 3: how far left and right are out of step (0..1).
// Rows 4-12 and 13-21: for rows 0 and 1, the loudest level within a widening span of each bin.
// Rows 22 and 23: for rows 0 and 1, the loudest level anywhere inside each single bin.
@group(0) @binding(1) var levels: texture_2d<f32>;
// The picture as a square map, drawn by `fs_map` at the start of each 3D frame:
// red is brightness, green is the out-of-step amount.
// The cover of the track that is playing, as a small square.
@group(0) @binding(2) var cover: texture_2d<f32>;
@group(0) @binding(3) var cover_sampler: sampler;
@group(1) @binding(0) var field_map: texture_2d<f32>;
@group(1) @binding(1) var field_sampler: sampler;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,   // (0, 0) at the bottom-left of the picture
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VertexOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: VertexOut;
    out.position = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    out.uv = p;
    return out;
}

// Set while drawing the 3D picture map: levels are then blended with a smooth
// curve through four neighbouring bins. The eased blend used for the flat
// picture is level at every bin centre, which in 3D turns every slope into
// a staircase and every peak into a dimpled ball.
var<private> smooth_levels: bool = false;

// Level at a fractional bin position, blended between neighbours with eased
// weights so stripes have no visible creases.
fn level_at(f: f32, row: i32) -> f32 {
    let n = i32(u.view.w);
    let x = f * u.view.w - 0.5;
    let i = i32(floor(x));
    var t = x - floor(x);
    if (smooth_levels && row < 2) {
        let p0 = textureLoad(levels, vec2<i32>(clamp(i - 1, 0, n - 1), row), 0).r;
        let p1 = textureLoad(levels, vec2<i32>(clamp(i, 0, n - 1), row), 0).r;
        let p2 = textureLoad(levels, vec2<i32>(clamp(i + 1, 0, n - 1), row), 0).r;
        let p3 = textureLoad(levels, vec2<i32>(clamp(i + 2, 0, n - 1), row), 0).r;
        let curve = p1 + 0.5 * t * (p2 - p0 + t * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3 + t * (3.0 * (p1 - p2) + p3 - p0)));
        // Kept within the four bins, so the search limits still hold.
        return clamp(curve, 0.0, max(max(p0, p1), max(p2, p3)));
    }
    t = t * t * (3.0 - 2.0 * t);
    let a = textureLoad(levels, vec2<i32>(clamp(i, 0, n - 1), row), 0).r;
    let b = textureLoad(levels, vec2<i32>(clamp(i + 1, 0, n - 1), row), 0).r;
    return mix(a, b, t);
}

// Position 0..1 on an axis to a position within the displayed frequency range.
fn in_range(f: f32) -> f32 {
    return u.view.y + f * (u.view.z - u.view.y);
}

fn ramp(t: f32) -> vec3<f32> {
    let n = u.extra.x;
    let s = clamp(t, 0.0, 1.0) * n;
    let i = min(u32(floor(s)), u32(n) - 1u);
    // Banding squeezes each blend toward a hard edge halfway between colours.
    // The fade up from black is left smooth so quiet sound still shows.
    var k = s - f32(i);
    if (i > 0u) { k = clamp((k - 0.5) / (1.0 - u.extra.y + 1e-3) + 0.5, 0.0, 1.0); }
    return mix(u.stops[i].rgb, u.stops[i + 1u].rgb, k);
}

// Stereo: how brightly a sound panned to `pan` shows at a point whose
// left-right position is `side` (both -1 left .. 1 right). Centred sound
// shows evenly; panned sound fades toward the opposite side.
fn steer(pan: f32, side: f32) -> f32 {
    if (u.stereo.x < 0.5) { return 1.0; }
    let toward = select(-1.0, 1.0, pan >= 0.0);
    let away = (1.0 - side * toward) * 0.5;
    // Nearly centred sound is left alone; the fade grows gently with how far it is panned.
    return 1.0 - u.stereo.y * smoothstep(0.05, 0.85, abs(pan)) * away;
}

// Stereo: recolour sound where left and right move against each other.
fn surround_tint(colour: vec3<f32>, t: f32, out_of_step: f32) -> vec3<f32> {
    if (u.stereo.x < 0.5) { return colour; }
    let w = smoothstep(0.12, 0.55, out_of_step) * u.stereo.z;
    let alt = clamp(u.surround.rgb * t * 1.2 + vec3<f32>(pow(t, 4.0) * 0.4), vec3<f32>(0.0), vec3<f32>(1.0));
    return mix(colour, alt, w);
}

// Two levels into one brightness: both needed (0) or either is enough (1).
fn combine(a: f32, b: f32) -> f32 {
    let either = max(a, b) * mix(u.tone.x, 1.0, min(a, b));
    return mix(a * b, either, u.shape.w);
}

// Bass pulse: a disc from the middle in a contrasting colour, laid over the
// picture. Past full strength it floods the frame and pushes to white.
fn bass_pulse(colour: vec3<f32>, uv: vec2<f32>) -> vec3<f32> {
    if (u.extra.w > 0.5) {
        // Fill style: the bass colour shows only where the picture is black or
        // nearly black, so it sits underneath everything else.
        let strength = u.circle.y;
        let brightest = max(colour.r, max(colour.g, colour.b));
        let dark = 1.0 - smoothstep(0.0, 0.04 + 0.5 * u.accent.a, brightest);
        let fill = clamp(u.accent.rgb * strength + vec3<f32>(max(strength - 1.0, 0.0) * 0.7), vec3<f32>(0.0), vec3<f32>(1.0));
        return colour + fill * dark * (1.0 - colour);
    }
    let aspect = vec2<f32>(u.circle.w, 1.0);
    let r = length((uv - 0.5) * aspect) / length(0.5 * aspect);
    let reach = r / max(u.circle.z, 1e-3);
    let pulse = u.circle.y * exp(-reach * reach);
    let wash = clamp(u.accent.rgb * pulse + vec3<f32>(max(pulse - 1.0, 0.0) * 0.7), vec3<f32>(0.0), vec3<f32>(1.0));
    return 1.0 - (1.0 - colour) * (1.0 - wash);
}

// Circle view: low frequencies in the middle, high at the corners, so a
// falling sweep is a shrinking ring. Returns (brightness 0..1, out-of-step 0..1).
fn circle_field(uv: vec2<f32>) -> vec2<f32> {
    let aspect = vec2<f32>(u.circle.w, 1.0);
    let p = (uv - 0.5) * aspect;
    let r = length(p) / length(0.5 * aspect);
    var f = r;
    if (u.shape.y > 0.5) { f = 1.0 - f; }
    f = in_range(clamp(f, 0.0, 1.0));

    // Stereo: each ring is brightest toward the side its sound is panned to
    // (nine o'clock for left, three for right); centred sound is an even ring.
    let side = p.x / max(length(p), 1e-5);
    let a = level_at(f, 0) * steer(level_at(f, 2), side);
    var v = combine(a, a);

    // Angular pattern: a second frequency runs round the ring (low at twelve
    // and six o'clock, high at three and nine), a polar form of the cross view.
    if (u.extra.z > 0.0) {
        let turn = atan2(p.x, p.y) / 6.2831853 + 0.5;
        var g = 1.0 - abs(fract(turn * 2.0) * 2.0 - 1.0);
        if (u.shape.z > 0.5) { g = 1.0 - g; }
        let b = level_at(in_range(g), 0) * steer(level_at(in_range(g), 2), side);
        v = mix(v, combine(a, b), u.extra.z);
    }
    // Past the corners of the picture there is no frequency left to show, so
    // the surface falls away to nothing. (Only the 3D view can see out there.)
    let inside = 1.0 - smoothstep(1.0, 1.03, r);
    return vec2<f32>(clamp(v * u.tone.y * u.view.x, 0.0, 1.0) * inside, level_at(f, 3));
}

// Cross view. Returns (brightness 0..1, out-of-step 0..1).
fn cross_field(uv: vec2<f32>) -> vec2<f32> {
    var f = uv;
    let mode = u.shape.x;
    if (mode == 1.0 || mode == 2.0) { f.x = 1.0 - abs(2.0 * f.x - 1.0); }
    if (mode == 1.0 || mode == 3.0) { f.y = 1.0 - abs(2.0 * f.y - 1.0); }
    // Unflipped, bass is where the mirror folds meet (the centre, with four
    // quadrants), matching the circle view. The flip switches reverse that.
    if (u.shape.y < 0.5) { f.x = 1.0 - f.x; }
    if (u.shape.z < 0.5) { f.y = 1.0 - f.y; }

    // Stereo: left-panned sound shows on the left half of the screen, right on the right.
    let side = uv.x * 2.0 - 1.0;
    let fx = in_range(f.x);
    let fy = in_range(f.y);
    let a = level_at(fx, 0) * steer(level_at(fx, 2), side);
    let b = level_at(fy, 1) * steer(level_at(fy, 2), side);
    var v = combine(a, b);
    v = v * u.tone.y + u.tone.z * max(a, b) * (1.0 - v);
    let out_of_step = (level_at(fx, 3) * a + level_at(fy, 3) * b) / max(a + b, 1e-4);
    return vec2<f32>(clamp(v * u.view.x, 0.0, 1.0), out_of_step);
}

// Brightness and out-of-step amount at a point of the picture. Outside the
// 0..1 square (seen only with relief) the cross repeats as mirror images and
// the circle simply continues outward.
fn field(uv: vec2<f32>) -> vec2<f32> {
    if (u.circle.x > 0.5) { return circle_field(uv); }
    return cross_field(1.0 - abs(1.0 - (uv - 2.0 * floor(uv * 0.5))));
}

fn shade(uv: vec2<f32>, value: vec2<f32>) -> vec3<f32> {
    let t = pow(value.x, u.tone.w);
    return bass_pulse(surround_tint(ramp(t), t, value.y), uv);
}

// ---------------------------------------------------------------------------
// 3D view. The picture is first drawn into a square map (`fs_map`); the 3D
// view and the rain then read brightness, and so height, from that map.

// In the circle view the map covers the ground out to just past the corner of
// the picture, where the highest frequency sits.
const MAP_MARGIN: f32 = 1.04;

// Distance from the centre of the picture to its corner.
fn corner() -> f32 {
    return length(0.5 * vec2<f32>(u.circle.w, 1.0));
}

// The map stores brightness in two parts, a coarse value and the remainder,
// because one 16-bit channel is too coarse for heights: its steps showed as
// ripples in the lighting.
@fragment
fn fs_map(in: VertexOut) -> @location(0) vec4<f32> {
    smooth_levels = true;
    var value: vec2<f32>;
    if (u.circle.x > 0.5) {
        let ground = (in.uv - 0.5) * 2.0 * corner() * MAP_MARGIN;
        value = circle_field(vec2<f32>(ground.x / u.circle.w + 0.5, ground.y + 0.5));
    } else {
        value = cross_field(in.uv);
    }
    let coarse = floor(value.x * 256.0) / 256.0;
    return vec4<f32>(coarse, value.y, (value.x - coarse) * 256.0, 1.0);
}

// Brightness and out-of-step amount at a ground point (x across, y up the
// picture, centre at the origin). Beyond the picture the cross repeats as
// mirror images; the circle carries on outward.
fn field_at(ground: vec2<f32>) -> vec2<f32> {
    var m = vec2<f32>(ground.x / u.circle.w + 0.5, ground.y + 0.5);
    if (u.circle.x > 0.5) {
        m = ground / (2.0 * corner() * MAP_MARGIN) + 0.5;
        // Off the map there is only flat, empty ground. (The camera itself is
        // out here at steep tilts.)
        if (any(m < vec2<f32>(0.0)) || any(m > vec2<f32>(1.0))) { return vec2<f32>(0.0); }
    }
    let stored = textureSampleLevel(field_map, field_sampler, vec2<f32>(m.x, 1.0 - m.y), 0.0);
    return vec2<f32>(stored.r + stored.b / 256.0, stored.g);
}

// How far the bass pulse lifts the surface at a distance from the centre
// (0 = centre, 1 = corner). The fill style stays on the ground.
fn pulse_height(r: f32) -> f32 {
    if (u.extra.w > 0.5) { return 0.0; }
    let reach = r / max(u.circle.z, 1e-3);
    return u.relief.y * 0.8 * min(u.circle.y * exp(-reach * reach), 1.5);
}

// Surface height at a ground point.
fn height_at(ground: vec2<f32>) -> f32 {
    return u.relief.y * pow(field_at(ground).x, u.tone.w) + pulse_height(length(ground) / corner());
}

// The loudest level within `half_bins` bins of position `f` on an axis
// (0 = x, 1 = y), from the precomputed rows of widening maxima.
fn max_level(f: f32, half_bins: f32, axis: i32) -> f32 {
    let n = i32(u.view.w);
    if (half_bins <= 0.5) {
        // A span of a bin or less: the exact maximum over the one or two bins it touches.
        let reach = half_bins / u.view.w;
        let lo = clamp(i32((f - reach) * u.view.w), 0, n - 1);
        let hi = clamp(i32((f + reach) * u.view.w), 0, n - 1);
        return max(textureLoad(levels, vec2<i32>(lo, 22 + axis), 0).r, textureLoad(levels, vec2<i32>(hi, 22 + axis), 0).r);
    }
    let k = clamp(i32(ceil(log2(half_bins + 1.5))) - 1, 0, 8);
    return textureLoad(levels, vec2<i32>(clamp(i32(f * u.view.w), 0, n - 1), 4 + axis * 9 + k), 0).r;
}

// A height the surface cannot exceed anywhere along the ground segment from
// `a` to `b`. Lets the march skip empty space without stepping over a wall.
// In the circle view only the span of distances from the centre matters, so
// a ray running alongside a ring is not held up by it.
fn height_limit(a: vec2<f32>, b: vec2<f32>) -> f32 {
    let aspect = u.circle.w;
    let range = u.view.z - u.view.y;
    let d = b - a;
    let nearest = length(a + d * clamp(-dot(a, d) / max(dot(d, d), 1e-12), 0.0, 1.0)) / corner();
    var v: f32;
    if (u.circle.x > 0.5) {
        let furthest = max(length(a), length(b)) / corner();
        var f = clamp(0.5 * (nearest + furthest), 0.0, 1.0);
        if (u.shape.y > 0.5) { f = 1.0 - f; }
        let level = max_level(in_range(f), 0.5 * (furthest - nearest) * range * u.view.w, 0);
        v = combine(level, level);
        if (u.extra.z > 0.0) {
            // Angular pattern: the second frequency depends on the direction
            // from the centre, so bound it over the directions this segment spans.
            let mid = 0.5 * (a + b);
            var g = 1.0 - abs(fract((atan2(mid.x, mid.y) / 6.2831853 + 0.5) * 2.0) * 2.0 - 1.0);
            if (u.shape.z > 0.5) { g = 1.0 - g; }
            let turn = atan2(abs(a.x * b.y - a.y * b.x), dot(a, b)) / 6.2831853;
            v = mix(v, combine(level, max_level(in_range(g), 4.0 * turn * range * u.view.w, 0)), u.extra.z);
        }
        v = v * u.tone.y * u.view.x;
        if (nearest > 1.03) { v = 0.0; }    // wholly outside the picture
    } else {
        let mid = 0.5 * (a + b);
        var uv = vec2<f32>(mid.x / aspect + 0.5, mid.y + 0.5);
        uv = 1.0 - abs(1.0 - (uv - 2.0 * floor(uv * 0.5)));
        var f = uv;
        let mode = u.shape.x;
        if (mode == 1.0 || mode == 2.0) { f.x = 1.0 - abs(2.0 * f.x - 1.0); }
        if (mode == 1.0 || mode == 3.0) { f.y = 1.0 - abs(2.0 * f.y - 1.0); }
        if (u.shape.y < 0.5) { f.x = 1.0 - f.x; }
        if (u.shape.z < 0.5) { f.y = 1.0 - f.y; }
        // Bins covered on each axis by this segment.
        let fold = vec2<f32>(select(1.0, 2.0, mode == 1.0 || mode == 2.0) / aspect, select(1.0, 2.0, mode == 1.0 || mode == 3.0));
        let half_bins = 0.5 * abs(d) * fold * range * u.view.w;
        let x = max_level(in_range(f.x), half_bins.x, 0);
        let y = max_level(in_range(f.y), half_bins.y, 1);
        v = (combine(x, y) * u.tone.y + u.tone.z * max(x, y)) * u.view.x;
    }
    return u.relief.y * pow(clamp(v, 0.0, 1.0), u.tone.w) + pulse_height(nearest);
}

fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

// Storm: what rain does to the look of the surface. Water runs down the
// slopes in moving streaks and low ground becomes dark, rippling pools.
// (The drops themselves are separate; see the rain section below.)
fn wet(colour: vec3<f32>, ground: vec2<f32>, h: f32, slope: vec2<f32>) -> vec3<f32> {
    let storm = u.relief.z;
    let time = u.relief.w;
    let water = vec3<f32>(0.60, 0.80, 1.00);

    // Ripples: each small patch of a pool is disturbed at its own rhythm.
    let g = ground * 26.0;
    let cell = floor(g);
    let rnd = hash(cell);
    let phase = fract(time * (0.7 + 0.9 * rnd) + rnd * 7.0);
    let centre = vec2<f32>(hash(cell + 3.1), hash(cell + 7.7)) * 0.4 - 0.2;
    let ring = smoothstep(0.05, 0.0, abs(length(fract(g) - 0.5 - centre) - phase * 0.25)) * (1.0 - phase) * (1.0 - phase);

    // Run-off: streaks that travel downhill, stronger on steeper ground.
    let steep = length(slope);
    let uphill = slope / max(steep, 1e-4);
    let along = dot(ground, uphill);
    let across = dot(ground, vec2<f32>(-uphill.y, uphill.x));
    let streak = smoothstep(0.55, 1.0, sin(across * 240.0 + 6.0 * hash(floor(vec2<f32>(across * 38.0, 0.0)))))
        * (0.5 + 0.5 * sin(along * 70.0 + time * 10.0));
    let runoff = streak * smoothstep(0.4, 2.5, steep);

    // Pools: the lowest ground fills with dark water.
    let low = 1.0 - smoothstep(0.02, 0.10, h / max(u.relief.y, 1e-3));
    var c = colour * (1.0 - 0.25 * storm);
    c = mix(c, vec3<f32>(0.010, 0.030, 0.055) + c * 0.35, low * storm * 0.8);
    c += water * storm * (0.30 * runoff + 0.25 * low * ring);
    return c;
}

// The picture lies on the ground with brightness as height, seen from the
// 3D camera.
fn relief_view(screen: vec2<f32>) -> vec3<f32> {
    let aspect = u.circle.w;
    let distance = 1.3;
    let spread = 0.5 / distance;
    let eye = u.cam_eye.xyz;
    let forward = normalize(u.cam_target.xyz - eye);
    let right = normalize(cross(forward, vec3<f32>(0.0, 0.0, 1.0)));
    let up = cross(right, forward);
    let s = screen * 2.0 - 1.0;
    let ray = normalize(forward + right * (s.x * aspect * spread) + up * (s.y * spread));
    if (ray.z > -0.02) { return vec3<f32>(0.0); }   // at or above the horizon

    // From where the ray drops below the highest possible surface to the
    // ground, or to the far limit where the picture has faded out anyway.
    let reach = 5.0;
    let top = u.relief.y + pulse_height(0.0);
    let t_start = max((eye.z - top) / -ray.z, 0.0);
    let t_ground = eye.z / -ray.z;
    let t_end = min(t_ground, t_start + reach);

    // Ground width of the narrowest frequency bin.
    let range = u.view.z - u.view.y;
    let mode = u.shape.x;
    var bins_per_unit = max(
        select(1.0, 2.0, mode == 1.0 || mode == 2.0) / aspect,
        select(1.0, 2.0, mode == 1.0 || mode == 3.0)) * range * u.view.w;
    if (u.circle.x > 0.5) { bins_per_unit = range * u.view.w / corner(); }
    let bin_width = 1.0 / bins_per_unit;
    let flat_speed = max(length(ray.xy), 1e-4);     // ground distance covered per unit along the ray

    // Take the longest step that provably stays above the surface; where none
    // does, shorten to a fraction of a bin and test the surface itself. A
    // wall is never stepped over, however thin.
    var t = t_start;
    var dt = (t_end - t_start) / 6.0;
    var t_hit = t_end;
    var found = false;
    for (var i = 0; i < 400; i++) {
        if (t >= t_end - 1e-5) { break; }
        dt = min(dt, t_end - t);
        // Far away a pixel covers more ground, so steps need not be as fine.
        let fine = max(0.5 * bin_width, 0.0030 * t) / flat_speed;
        let p0 = eye + ray * t;
        let p1 = eye + ray * (t + dt);
        if (p1.z > height_limit(p0.xy, p1.xy)) {
            t += dt;
            dt *= 2.0;
            continue;
        }
        if (dt > fine) {
            dt *= 0.5;
            continue;
        }
        // Test the end of the step and its middle, so the tip of a thin peak
        // is not passed through between two samples.
        let half = eye + ray * (t + 0.5 * dt);
        let at_half = half.z <= height_at(half.xy);
        if (at_half || p1.z <= height_at(p1.xy)) {
            var lo = t;
            var hi = t + select(dt, 0.5 * dt, at_half);
            for (var k = 0; k < 6; k++) {
                let m = 0.5 * (lo + hi);
                let r = eye + ray * m;
                if (r.z <= height_at(r.xy)) { hi = m; } else { lo = m; }
            }
            t_hit = hi;
            found = true;
            break;
        }
        t += dt;
    }
    // Past the far limit, or out of steps on a grazing view across dense
    // detail in the distance, there is nothing reliable to draw.
    if (!found) { return vec3<f32>(0.0); }

    let hit = eye + ray * t_hit;
    let e = 0.5 * bin_width;
    let h = height_at(hit.xy);
    let slope = vec2<f32>(
        height_at(hit.xy + vec2<f32>(e, 0.0)) - height_at(hit.xy - vec2<f32>(e, 0.0)),
        height_at(hit.xy + vec2<f32>(0.0, e)) - height_at(hit.xy - vec2<f32>(0.0, e))) / (2.0 * e);
    let normal = normalize(vec3<f32>(-slope, 1.0));

    // A wall takes the colour of the sound at its top most of the way down
    // its side. Coloured by its own height instead, every wall fades to black
    // at the foot, and a tilted view fills with those dark lower sides.
    var value = field_at(hit.xy);
    let steep = length(slope);
    if (steep > 0.5) {
        let uphill = slope / steep;
        var peak = value.x;
        for (var j = 1; j <= 5; j++) {
            peak = max(peak, field_at(hit.xy + uphill * (bin_width * 1.2 * f32(j))).x);
        }
        value.x = mix(value.x, peak, 0.7 * smoothstep(0.5, 2.5, steep));
    }
    var colour = shade(vec2<f32>(hit.x / aspect + 0.5, hit.y + 0.5), value);

    // Light from the upper left so slopes facing it are brighter; distance dims gently.
    let sun = normalize(vec3<f32>(-0.35, 0.45, 0.8));
    let light = max(dot(normal, sun), 0.0);
    // What the surface is made of. Highlights only show where there is
    // sound, so silence stays black whatever the material.
    let material = u.stereo.w;
    let present = smoothstep(0.02, 0.25, max(colour.r, max(colour.g, colour.b)));
    let glint = max(dot(normal, normalize(sun - ray)), 0.0);
    let matte = colour * (0.55 + 0.75 * light);
    if (material < 0.5) {
        colour = matte;
    } else if (material < 1.5) {
        // Gloss: shiny plastic, with a white highlight where a slope catches the light.
        colour = colour * (0.50 + 0.70 * light) + vec3<f32>(0.75 * pow(glint, 48.0) * present);
    } else if (material < 2.5) {
        // Metal: little light of its own; it mirrors a bright sky in its own colour.
        let mirrored = reflect(ray, normal);
        let sky = smoothstep(-0.25, 0.75, mirrored.z) * (0.75 + 0.25 * sin(mirrored.x * 7.0 + mirrored.y * 5.0));
        colour = colour * (0.30 + 0.35 * light + 1.00 * sky) + mix(colour, vec3<f32>(1.0), 0.45) * (1.3 * pow(glint, 90.0) * present);
    } else {
        // Glass: dim face on, bright where it is seen edge on, with sharp glints.
        let rim = pow(1.0 - max(dot(normal, -ray), 0.0), 3.0);
        colour = colour * (0.35 + 0.40 * light) + mix(colour, vec3<f32>(1.0), 0.7) * (0.85 * rim * present) + vec3<f32>(pow(glint, 200.0) * present);
    }
    colour = mix(matte, colour, u.sim.z);
    if (u.relief.z > 0.0) { colour = wet(colour, hit.xy, h, slope); }
    colour *= exp(-0.22 * max(t_hit - distance, 0.0)) * (1.0 - smoothstep(0.5 * reach, 0.95 * reach, t_hit - t_start));
    return clamp(colour, vec3<f32>(0.0), vec3<f32>(1.0));
}

// Final step for every colour. On a standard display, nothing. On an HDR
// display the surface is linear and 1.0 means 80 nits: ordinary colours are
// shown at the base brightness and the brightest ones climb to the peak.
fn to_display(colour: vec3<f32>) -> vec3<f32> {
    if (u.hdr.x < 0.5) { return colour; }
    let c = max(colour, vec3<f32>(0.0));
    let linear = select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
    let brightest = min(max(c.r, max(c.g, c.b)), 1.0);
    return linear * mix(u.hdr.y, u.hdr.z, pow(brightest, 3.0));
}

// HDR test pattern: a band of white patches across the middle of the
// picture. Five are at fixed brightness (80, 200, 400, 800 and 1600 nits);
// the last two follow the base and peak brightness settings.
fn test_pattern(uv: vec2<f32>) -> vec4<f32> {
    if (abs(uv.y - 0.5) > 0.09 || uv.x < 0.15 || uv.x > 0.85) { return vec4<f32>(0.0); }
    let x = (uv.x - 0.15) / 0.1;
    let tile = i32(floor(x));
    if (abs(fract(x) - 0.5) > 0.42 || abs(uv.y - 0.5) > 0.075) { return vec4<f32>(0.0, 0.0, 0.0, 1.0); }
    var level = array<f32, 7>(1.0, 2.5, 5.0, 10.0, 20.0, u.hdr.y, u.hdr.z)[tile];
    return vec4<f32>(vec3<f32>(level), 1.0);
}

// ---------------------------------------------------------------------------
// Background: what shows where the picture is dark.

// Stars at a point of a flat sky, in three sizes. They twinkle, more so
// with treble in the music.
fn starfield(p: vec2<f32>) -> f32 {
    let time = u.relief.w;
    var light = 0.0;
    for (var layer = 0; layer < 3; layer++) {
        let scale = 26.0 * pow(2.1, f32(layer));
        let g = p * scale + f32(layer) * 17.3;
        let cell = floor(g);
        let rnd = hash(cell);
        if (rnd < 0.90) { continue; }
        let centre = (vec2<f32>(hash(cell + 3.1), hash(cell + 7.7)) - 0.5) * 0.6;
        let d = length(fract(g) - 0.5 - centre);
        let size = (rnd - 0.90) * 10.0;
        let twinkle = 0.65 + 0.35 * sin(time * (1.5 + 5.0 * hash(cell + 1.7)) + 40.0 * rnd) * (0.35 + 0.65 * u.post.w);
        light += (0.35 + 0.65 * size) * twinkle * smoothstep(0.05 + 0.07 * size, 0.0, d) / (1.0 + 0.6 * f32(layer));
    }
    return light;
}

// The direction a screen point looks in, in 3D.
fn view_ray(screen: vec2<f32>) -> vec3<f32> {
    let spread = 0.5 / 1.3;
    let forward = normalize(u.cam_target.xyz - u.cam_eye.xyz);
    let right = normalize(cross(forward, vec3<f32>(0.0, 0.0, 1.0)));
    let up = cross(right, forward);
    let s = screen * 2.0 - 1.0;
    return normalize(forward + right * (s.x * u.circle.w * spread) + up * (s.y * spread));
}

// The background behind a screen point.
fn backdrop(uv: vec2<f32>) -> vec3<f32> {
    let mode = u.post.y;
    if (mode < 0.5) { return vec3<f32>(0.0); }
    if (mode < 1.5) {
        // Flat, the stars drift slowly. In 3D they are fixed in the sky all
        // round, and seen through the black ground, so they move with the camera.
        var p = (uv - 0.5) * vec2<f32>(u.circle.w, 1.0) + u.relief.w * vec2<f32>(0.004, 0.0012);
        if (u.relief.x > 0.5) {
            let ray = view_ray(uv);
            p = vec2<f32>(atan2(ray.x, ray.y) * 0.6, ray.z * 1.2);
        }
        return vec3<f32>(0.80, 0.88, 1.00) * starfield(p) * u.post.z;
    }
    // The cover, filling the picture, blurred by averaging a ring of samples.
    var c = (uv - 0.5) * vec2<f32>(1.0, -1.0);
    if (u.circle.w > 1.0) { c.y /= u.circle.w; } else { c.x *= u.circle.w; }
    var sum = textureSampleLevel(cover, cover_sampler, c + 0.5, 0.0).rgb;
    for (var k = 0; k < 12; k++) {
        let a = 6.2831853 * f32(k) / 12.0;
        let r = select(0.035, 0.07, k % 2 == 0);
        sum += textureSampleLevel(cover, cover_sampler, c + 0.5 + r * vec2<f32>(cos(a), sin(a)), 0.0).rgb;
    }
    return sum / 13.0 * 0.30 * u.post.z;
}

// The picture before it is fitted to the display: flat or 3D, over the background.
fn scene(uv: vec2<f32>) -> vec3<f32> {
    var colour: vec3<f32>;
    if (u.relief.x > 0.5) { colour = relief_view(uv); } else { colour = shade(uv, field(uv)); }
    if (u.post.y > 0.5) {
        let brightest = max(colour.r, max(colour.g, colour.b));
        colour += backdrop(uv) * (1.0 - smoothstep(0.0, 0.45, brightest));
    }
    return colour;
}

@fragment
fn fs(in: VertexOut) -> @location(0) vec4<f32> {
    if (u.hdr.w > 0.5) {
        let tile = test_pattern(in.uv);
        if (tile.a > 0.5) { return tile; }
    }
    return vec4<f32>(to_display(scene(in.uv)), 1.0);
}

// With bloom on, the picture is drawn to a texture of its own first and
// fitted to the display afterwards (see post.wgsl).
@fragment
fn fs_scene(in: VertexOut) -> @location(0) vec4<f32> {
    return vec4<f32>(scene(in.uv), 1.0);
}

// ---------------------------------------------------------------------------
// Storm rain. Each drop falls until it meets the surface, bounces up as a
// short splash, then evaporates and starts again as a new drop.

struct Drop {
    pos: vec3<f32>,
    life: f32,      // splashes only: seconds left
    vel: vec3<f32>,
    kind: f32,      // -1 unused, 0 falling, 1 splash
};

@group(2) @binding(0) var<storage, read_write> falling: array<Drop>;
@group(3) @binding(0) var<storage, read> drops: array<Drop>;

// A well-mixed random number in 0..1 for drop `i`, different for each `salt`
// and each frame.
fn chance(i: u32, salt: u32) -> f32 {
    var x = (i * 747796405u + salt * 2891336453u) ^ bitcast<u32>(u.relief.w);
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    x = (x >> 22u) ^ x;
    return f32(x >> 8u) / 16777216.0;
}

@compute @workgroup_size(64)
fn cs_rain(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (i >= arrayLength(&falling)) { return; }
    let n = f32(i);
    let dt = u.sim.x;
    var d = falling[i];

    if (n >= u.sim.y) {             // more drops than the storm setting calls for
        d.kind = -1.0;
        falling[i] = d;
        return;
    }

    var renew = d.kind < -0.5;
    if (!renew && d.kind < 0.5) {
        d.pos += d.vel * dt;
        let ground = height_at(d.pos.xy);
        if (d.pos.z <= ground) {
            let angle = 6.2831853 * chance(i, 1u);
            let speed = 0.10 + 0.25 * chance(i, 2u);
            d.pos.z = ground + 0.003;
            d.vel = vec3<f32>(cos(angle) * speed, sin(angle) * speed, 0.45 + 0.5 * chance(i, 3u));
            d.kind = 1.0;
            d.life = 0.30;
        }
    } else if (!renew) {
        d.vel.z -= 4.5 * dt;
        d.pos += d.vel * dt;
        d.life -= dt;
        renew = d.life <= 0.0 || d.pos.z < height_at(d.pos.xy) - 0.01;
    }

    if (renew) {
        // Rain falls over a wide disc around where the camera is looking.
        let angle = 6.2831853 * chance(i, 4u);
        let radius = 1.8 * sqrt(chance(i, 5u));
        let top = 2.2 * u.relief.y + 0.35;
        d.pos = vec3<f32>(u.cam_target.xy + radius * vec2<f32>(cos(angle), sin(angle)), top + 1.4 * chance(i, 6u));
        d.vel = vec3<f32>(0.12, 0.05, -(2.4 + 0.8 * chance(i, 7u)));
        d.kind = 0.0;
        d.life = 1.0;
    }
    falling[i] = d;
}

struct DropOut {
    @builtin(position) position: vec4<f32>,
    @location(0) glow: f32,
    @location(1) side: f32,
};

@vertex
fn vs_drop(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> DropOut {
    var out: DropOut;
    out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);     // off screen unless shown below
    let d = drops[instance];
    if (d.kind < -0.5) { return out; }

    let aspect = u.circle.w;
    let spread = 0.5 / 1.3;
    let eye = u.cam_eye.xyz;
    let forward = normalize(u.cam_target.xyz - eye);
    let right = normalize(cross(forward, vec3<f32>(0.0, 0.0, 1.0)));
    let up = cross(right, forward);

    // A short streak from where the drop is back along the way it came.
    let tail = d.pos - d.vel * select(0.035, 0.020, d.kind > 0.5);
    let a = d.pos - eye;
    let b = tail - eye;
    let depth_a = dot(a, forward);
    let depth_b = dot(b, forward);
    if (depth_a < 0.05 || depth_b < 0.05) { return out; }
    let sa = vec2<f32>(dot(a, right) / (depth_a * aspect * spread), dot(a, up) / (depth_a * spread));
    let sb = vec2<f32>(dot(b, right) / (depth_b * aspect * spread), dot(b, up) / (depth_b * spread));

    // Hidden if the surface rises between the drop and the camera.
    let toward = -a;
    let span = min(1.0, 1.5 / max(length(toward), 1e-3));
    for (var k = 1; k <= 8; k++) {
        let q = d.pos + toward * (span * f32(k) / 8.0);
        if (q.z < height_at(q.xy)) { return out; }
    }

    let ends = array<f32, 6>(0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let sides = array<f32, 6>(-1.0, 1.0, -1.0, -1.0, 1.0, 1.0);
    let line = (sb - sa) * vec2<f32>(aspect, 1.0);
    let across = normalize(vec2<f32>(-line.y, line.x) + vec2<f32>(1e-6, 0.0));
    let width = clamp(0.0030 / depth_a, 0.0008, 0.0040);
    let at = mix(sa, sb, ends[vertex]) + across * sides[vertex] * width / vec2<f32>(aspect, 1.0);
    out.position = vec4<f32>(at, 0.5, 1.0);
    out.side = sides[vertex];
    let fade = select(0.55, 1.2 * clamp(d.life / 0.30, 0.0, 1.0), d.kind > 0.5);
    out.glow = fade * (1.0 - 0.7 * ends[vertex]) * exp(-0.25 * depth_a);
    return out;
}

@fragment
fn fs_drop(in: DropOut) -> @location(0) vec4<f32> {
    let soft = 1.0 - in.side * in.side;
    return vec4<f32>(to_display(vec3<f32>(0.62, 0.80, 1.00) * in.glow * soft), 0.0);
}
