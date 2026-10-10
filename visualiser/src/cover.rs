//! Colours for the picture taken from a track's cover, so each track has
//! its own look.

use crate::nowplaying::Cover;

/// The colours the picture is drawn in.
#[derive(Clone, PartialEq)]
pub struct Look {
    /// From quiet to loud; black is always below the first.
    pub stops: Vec<[f32; 3]>,
    /// For the bass pulse.
    pub accent: [f32; 3],
    /// For out-of-step (surround) sound in stereo.
    pub surround: [f32; 3],
}

/// A colour from its hue (0..1 round the colour wheel), saturation and brightness.
fn hsv(hue: f32, saturation: f32, value: f32) -> [f32; 3] {
    let h = hue.rem_euclid(1.0) * 6.0;
    std::array::from_fn(|i| {
        let k = (h + [5.0, 3.0, 1.0][i]) % 6.0;
        value * (1.0 - saturation * k.min(4.0 - k).clamp(0.0, 1.0))
    })
}

/// Hues counted in this many steps round the colour wheel.
const HUES: usize = 24;

/// A palette from a cover: its two strongest colours, from dark to bright,
/// with bass and surround colours chosen to stand apart from both.
pub fn look(cover: &Cover) -> Look {
    // How much of the cover is each hue, counting vivid pixels more, and how
    // saturated that hue is where it appears.
    let mut amount = [0.0f32; HUES];
    let mut saturation = [0.0f32; HUES];
    let mut vivid = 0.0;
    let pixels = cover.rgba.chunks_exact(4);
    let count = pixels.len().max(1) as f32;
    for pixel in pixels {
        let [r, g, b] = [pixel[0], pixel[1], pixel[2]].map(|v| v as f32 / 255.0);
        let (high, low) = (r.max(g).max(b), r.min(g).min(b));
        let spread = high - low;
        if spread < 0.02 {
            continue;
        }
        let hue = if high == r { ((g - b) / spread).rem_euclid(6.0) } else if high == g { (b - r) / spread + 2.0 } else { (r - g) / spread + 4.0 } / 6.0;
        let weight = spread * high.sqrt();
        let bin = ((hue * HUES as f32) as usize).min(HUES - 1);
        amount[bin] += weight;
        saturation[bin] += weight * spread / high;
        vivid += spread / high;
    }
    // A black and white cover gives a grey picture with a red bass.
    if vivid / count < 0.08 {
        return Look {
            stops: vec![[0.12, 0.12, 0.13], [0.40, 0.40, 0.42], [0.72, 0.72, 0.74], [1.0, 1.0, 1.0]],
            accent: [1.0, 0.2, 0.15],
            surround: [0.15, 0.75, 1.0],
        };
    }

    // Neighbouring steps count together, so a colour that straddles two is not split.
    let smoothed: Vec<f32> = (0..HUES).map(|i| amount[(i + HUES - 1) % HUES] * 0.5 + amount[i] + amount[(i + 1) % HUES] * 0.5).collect();
    let apart = |a: usize, b: usize| {
        let d = a.abs_diff(b);
        d.min(HUES - d)
    };
    let strongest = |skip: Option<usize>| {
        (0..HUES)
            .filter(|i| skip.is_none_or(|s| apart(*i, s) > 3))
            .max_by(|a, b| smoothed[*a].total_cmp(&smoothed[*b]))
            .unwrap_or(0)
    };
    let first = strongest(None);
    let mut second = strongest(Some(first));
    // A cover in one colour: the second is a near neighbour of the first.
    if smoothed[second] < 0.12 * smoothed[first] {
        second = (first + 2) % HUES;
    }
    let hue_of = |bin: usize| (bin as f32 + 0.5) / HUES as f32;
    let strength = |bin: usize| (saturation[bin] / amount[bin].max(1e-6)).clamp(0.5, 0.95);
    let (a, b) = (hue_of(first), hue_of(second));

    // The bass colour sits opposite the two; the surround colour to one side of that.
    let (sum_x, sum_y) = [a, b].iter().fold((0.0f32, 0.0f32), |(x, y), h| (x + (h * std::f32::consts::TAU).cos(), y + (h * std::f32::consts::TAU).sin()));
    let opposite = sum_y.atan2(sum_x) / std::f32::consts::TAU + 0.5;
    Look {
        stops: vec![hsv(a, strength(first), 0.32), hsv(a, strength(first), 0.80), hsv(b, strength(second) * 0.9, 1.0), hsv(b, 0.22, 1.0)],
        accent: hsv(opposite, 0.85, 1.0),
        surround: hsv(opposite + 0.17, 0.75, 1.0),
    }
}
