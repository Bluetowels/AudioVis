// Bloom: the bright parts of the finished picture are blurred and laid back
// over it as a glow. The picture arrives as a texture drawn by `fs_scene`.

struct Post {
    // HDR output on (1) or off (0), base and peak brightness in units of 80 nits, test pattern on (1)
    hdr: vec4<f32>,
    // bloom amount, unused
    bloom: vec4<f32>,
};

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;
@group(1) @binding(0) var<uniform> post: Post;
// The glow at a quarter and at a sixteenth of the picture's size.
@group(1) @binding(1) var glow_near: texture_2d<f32>;
@group(1) @binding(2) var glow_wide: texture_2d<f32>;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,   // (0, 0) at the top-left, as textures are
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VertexOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: VertexOut;
    out.position = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(p.x, 1.0 - p.y);
    return out;
}

// A quarter-size copy: each pixel is the average of a 4 x 4 block.
fn shrink(uv: vec2<f32>) -> vec3<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(source));
    return 0.25 * (textureSampleLevel(source, source_sampler, uv + texel * vec2<f32>(-1.0, -1.0), 0.0).rgb
        + textureSampleLevel(source, source_sampler, uv + texel * vec2<f32>(1.0, -1.0), 0.0).rgb
        + textureSampleLevel(source, source_sampler, uv + texel * vec2<f32>(-1.0, 1.0), 0.0).rgb
        + textureSampleLevel(source, source_sampler, uv + texel * vec2<f32>(1.0, 1.0), 0.0).rgb);
}

// Shrink the picture, keeping only its bright parts.
@fragment
fn fs_bright(in: VertexOut) -> @location(0) vec4<f32> {
    let c = shrink(in.uv);
    let brightest = max(c.r, max(c.g, c.b));
    return vec4<f32>(c * smoothstep(0.30, 0.95, brightest), 1.0);
}

@fragment
fn fs_shrink(in: VertexOut) -> @location(0) vec4<f32> {
    return vec4<f32>(shrink(in.uv), 1.0);
}

// A soft blur along one direction; done across then down, it is round.
fn blur(uv: vec2<f32>, step: vec2<f32>) -> vec3<f32> {
    let texel = step / vec2<f32>(textureDimensions(source));
    var sum = textureSampleLevel(source, source_sampler, uv, 0.0).rgb * 0.227027;
    sum += (textureSampleLevel(source, source_sampler, uv + texel * 1.384615, 0.0).rgb
        + textureSampleLevel(source, source_sampler, uv - texel * 1.384615, 0.0).rgb) * 0.316216;
    sum += (textureSampleLevel(source, source_sampler, uv + texel * 3.230769, 0.0).rgb
        + textureSampleLevel(source, source_sampler, uv - texel * 3.230769, 0.0).rgb) * 0.070270;
    return sum;
}

@fragment
fn fs_blur_across(in: VertexOut) -> @location(0) vec4<f32> {
    return vec4<f32>(blur(in.uv, vec2<f32>(1.0, 0.0)), 1.0);
}

@fragment
fn fs_blur_down(in: VertexOut) -> @location(0) vec4<f32> {
    return vec4<f32>(blur(in.uv, vec2<f32>(0.0, 1.0)), 1.0);
}

// The same last step as the main shader's: nothing on a standard display;
// on an HDR one, ordinary colours at the base brightness and the brightest
// climbing to the peak.
fn to_display(colour: vec3<f32>) -> vec3<f32> {
    if (post.hdr.x < 0.5) { return colour; }
    let c = max(colour, vec3<f32>(0.0));
    let linear = select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
    let brightest = min(max(c.r, max(c.g, c.b)), 1.0);
    return linear * mix(post.hdr.y, post.hdr.z, pow(brightest, 3.0));
}

// The HDR test pattern, as in the main shader.
fn test_pattern(uv: vec2<f32>) -> vec4<f32> {
    if (abs(uv.y - 0.5) > 0.09 || uv.x < 0.15 || uv.x > 0.85) { return vec4<f32>(0.0); }
    let x = (uv.x - 0.15) / 0.1;
    let tile = i32(floor(x));
    if (abs(fract(x) - 0.5) > 0.42 || abs(uv.y - 0.5) > 0.075) { return vec4<f32>(0.0, 0.0, 0.0, 1.0); }
    var level = array<f32, 7>(1.0, 2.5, 5.0, 10.0, 20.0, post.hdr.y, post.hdr.z)[tile];
    return vec4<f32>(vec3<f32>(level), 1.0);
}

// The picture with its glow added, fitted to the display.
@fragment
fn fs_final(in: VertexOut) -> @location(0) vec4<f32> {
    if (post.hdr.w > 0.5) {
        let tile = test_pattern(in.uv);
        if (tile.a > 0.5) { return tile; }
    }
    let picture = textureSampleLevel(source, source_sampler, in.uv, 0.0).rgb;
    let glow = textureSampleLevel(glow_near, source_sampler, in.uv, 0.0).rgb * 0.55
        + textureSampleLevel(glow_wide, source_sampler, in.uv, 0.0).rgb * 0.85;
    return vec4<f32>(to_display(min(picture + glow * post.bloom.x * 1.6, vec3<f32>(1.0))), 1.0);
}
