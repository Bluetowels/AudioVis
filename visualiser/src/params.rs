//! Every adjustable setting, in one table that the GUI, the MIDI controller
//! and presets all share.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum P {
    Slope,
    Range,
    Reference,
    AutoGainSpeed,
    Contrast,
    Brightness,
    BassAmount,
    BassCutoff,
    BassRelease,
    BassGlow,
    Decay,
    Combine,
    FreqLow,
    FreqHigh,
    Smoothing,
    Palette,
    Angular,
    Banding,
    BassWindow,
    BassSharpen,
    Sharpen,
    Detail,
    StereoEmphasis,
    SurroundAmount,
    Tilt,
    ReliefHeight,
    Orbit,
    Flight,
    Storm,
    FlightDepth,
    LookAhead,
    HdrBase,
    HdrPeak,
    LyricsSize,
    LyricsOffset,
}

pub struct Def {
    pub id: P,
    pub key: &'static str,
    pub name: &'static str,
    pub unit: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    /// Logarithmic travel, for frequencies and times.
    pub log: bool,
    pub help: &'static str,
}

const fn def(
    id: P,
    key: &'static str,
    name: &'static str,
    unit: &'static str,
    min: f32,
    max: f32,
    default: f32,
    log: bool,
    help: &'static str,
) -> Def {
    Def { id, key, name, unit, min, max, default, log, help }
}

pub const N_PARAMS: usize = 35;
pub const N_PALETTES: usize = 10;

pub const DEFS: [Def; N_PARAMS] = [
    def(P::Slope, "slope", "Slope", "dB/oct", -3.0, 3.0, 0.0, false, "Tilts the balance between bass and treble about the middle of the spectrum. 0 suits most music, because the analysis already treats each note equally. Positive brings out treble and negative brings out bass. The range is kept modest: a steeper tilt pushes most of the spectrum out of the brightness range and the picture goes dark."),
    def(P::Range, "range", "Range", "dB", 20.0, 90.0, 40.0, false, "How far below full brightness a sound can be and still show. Smaller gives more contrast and more black; larger shows quiet detail."),
    def(P::Reference, "reference", "Reference level", "dBFS", -60.0, 12.0, -12.0, false, "The level shown at full brightness. Only used when auto-gain is off."),
    def(P::AutoGainSpeed, "auto_gain_speed", "Auto-gain speed", "dB/s", 0.5, 40.0, 4.0, true, "How quickly auto-gain turns the picture back up after the music gets quieter. Slow keeps loud and quiet passages looking different; fast evens everything out."),
    def(P::Contrast, "contrast", "Contrast curve", "", 0.4, 3.0, 1.2, false, "How colour climbs from quiet to loud. Above 1 keeps quiet sound dark so peaks stand out; below 1 lifts quiet sound into the brighter colours."),
    def(P::Brightness, "brightness", "Master brightness", "x", 0.0, 2.0, 1.0, false, "Overall brightness of the picture, applied after everything else."),
    def(P::BassAmount, "bass_amount", "Bass boost amount", "", 0.0, 2.0, 0.5, false, "Strength of the bass effect. Above 1 is extreme: big hits flood the whole frame with the bass colour."),
    def(P::BassCutoff, "bass_cutoff", "Bass cutoff", "Hz", 40.0, 400.0, 150.0, true, "Sound below this frequency drives the bass effect. Lower it to respond to the kick drum only; raise it to include bass notes."),
    def(P::BassRelease, "bass_release", "Bass release", "ms", 10.0, 500.0, 60.0, true, "How long the bass effect takes to die away after a hit. Short is punchy; long gives a slow throb."),
    def(P::BassGlow, "bass_glow", "Bass glow", "", 0.0, 1.0, 0.3, false, "Pulse: how far it spreads from the middle. Fill dark areas: how dim a part of the picture can be and still be filled. In the cross view, with the whole-picture flare on, it also sets how much dark areas glow on a hit."),
    def(P::Decay, "decay", "Decay / persistence", "ms", 0.0, 600.0, 0.0, false, "How long a sound lingers on screen after it stops. 0 snaps off at once, like a fast analyser; higher leaves trails."),
    def(P::Combine, "combine", "Combine blend", "", 0.0, 1.0, 0.0, false, "At 0 a pixel lights only when both of its frequencies are sounding, which gives a compact, detailed picture. Towards 1, one loud frequency lights its whole row and column, which fills the screen. In the circle view it makes rings brighter and softer."),
    def(P::FreqLow, "freq_low", "Lowest frequency", "oct", 0.0, 8.0, 0.0, false, "The lowest frequency shown. Raise it to zoom in and leave out empty sub-bass."),
    def(P::FreqHigh, "freq_high", "Highest frequency", "oct", 1.0, 9.0, 9.0, false, "The highest frequency shown. Lower it to zoom in on the range where the music is."),
    def(P::Smoothing, "smoothing", "Smoothing between bins", "bins", 0.0, 6.0, 1.5, false, "Blurs neighbouring frequencies together. 0 keeps every stripe or ring sharp; higher gives broad, soft shapes."),
    def(P::Palette, "palette", "Palette", "", 0.0, (N_PALETTES - 1) as f32, 0.0, false, "The colours used from quiet to loud. The first six are smooth; Rainbow, Zigzag, Candy and Contour change colour abruptly. Also on the right-click menu of the picture."),
    def(P::Angular, "angular", "Circle: angular pattern", "", 0.0, 1.0, 0.5, false, "Circle view only. 0 is plain rings; higher runs a second frequency round the ring, like the cross view bent into a circle."),
    def(P::Banding, "banding", "Colour banding", "", 0.0, 1.0, 0.0, false, "0 blends smoothly between palette colours; 1 gives hard-edged bands."),
    def(P::BassWindow, "bass_window", "Bass window", "ms", 100.0, 1000.0, 200.0, true, "How much audio the lowest notes are measured over. Longer separates neighbouring bass notes better but they arrive later and linger; shorter is quicker and blurrier. 200 is the usual balance. Changes apply a moment after you stop moving it."),
    def(P::BassSharpen, "bass_sharpen", "Bass note sharpening", "", 0.0, 1.0, 0.3, false, "Low notes are smeared across several bins. This redraws each smear as a thin line at its centre, so a bass line is easy to follow. 0 leaves the smear; 1 is fully sharpened. Works best with one bass note at a time; two notes closer than about three semitones can merge into one."),
    def(P::Sharpen, "sharpen", "Note sharpening (whole spectrum)", "", 0.0, 1.0, 0.2, false, "Thins every note to a single fine line, at all frequencies. Good on clean, pitched music such as piano, voice and synths; on cymbals, snares and distortion it turns the texture into flickering thin lines. 0 is off."),
    def(P::Detail, "detail", "Speed vs pitch detail", "", 0.25, 1.0, 0.25, true, "1 measures each note over as long as its pitch needs: finest detail, slowest in the low mids. Lower values shorten every measurement, so the picture reacts faster but notes spread across more bins; add note sharpening to tidy that up. Changes apply a moment after you stop moving it."),
    def(P::StereoEmphasis, "stereo_emphasis", "Stereo emphasis", "", 0.0, 1.0, 1.0, false, "With Stereo on, how strongly each sound is pushed to the side of the screen it is panned to. 0 ignores panning; 1 leaves a hard-panned sound on its own side only. Centred and nearly centred sounds are always shown evenly."),
    def(P::SurroundAmount, "surround_amount", "Surround colour amount", "", 0.0, 1.0, 1.0, false, "With Stereo on, sound where left and right move against each other (wide, ambient sound, what a surround upmix sends to the rear speakers) is drawn in the surround colour. 0 turns that off."),
    def(P::Tilt, "tilt", "3D tilt", "deg", 0.0, 70.0, 0.0, false, "Experimental. 0 is the flat picture. Raising it tips the camera back so the picture is seen at an angle, with brightness standing up as height: ridges in the cross view, ripples and walls in the circle view."),
    def(P::ReliefHeight, "relief_height", "3D height", "", 0.0, 1.0, 0.6, false, "Experimental. How tall the brightest parts stand when 3D tilt is above 0. 0 keeps the tilted picture flat."),
    def(P::Orbit, "orbit", "3D orbit speed", "deg/s", -30.0, 30.0, 0.0, false, "With 3D tilt above 0, the camera circles the centre at this speed. 0 holds the front view; negative turns the other way."),
    def(P::Flight, "flight", "3D flight speed", "", 0.0, 2.0, 0.0, false, "Above 0, the camera leaves its fixed position and flies a smooth, wandering path around and over the picture, taking over from tilt and orbit. The value is how fast it flies. 0 is off."),
    def(P::Storm, "storm", "3D storm", "", 0.0, 1.0, 0.0, false, "Experimental. Raindrops fall on the 3D view, splash where they land and evaporate; the surface shows water running down slopes and pooling in the lowest areas. Higher values bring more rain. 0 is off."),
    def(P::FlightDepth, "flight_depth", "3D flight depth", "", 0.0, 1.0, 0.0, false, "How far the flight goes into the picture. 0 circles above and around it. Towards 1 the camera drops down among the walls and peaks and passes low across the middle, riding just above the surface as it moves with the music."),
    def(P::LookAhead, "look_ahead", "3D flight look ahead", "", 0.0, 1.0, 0.0, false, "Where the camera points in flight. 0 always looks towards the centre; 1 looks the way it is flying, like travelling through a landscape."),
    def(P::HdrBase, "hdr_base", "HDR base brightness", "nits", 80.0, 500.0, 200.0, true, "HDR output only. How bright ordinary parts of the picture are. 200 is close to a typical desktop; lower suits a dark room."),
    def(P::HdrPeak, "hdr_peak", "HDR peak brightness", "nits", 200.0, 2000.0, 1000.0, true, "HDR output only. How bright the very loudest parts of the picture go. Set it at or below what the display can reach; higher values are simply clipped by the display."),
    def(P::LyricsSize, "lyrics_size", "Lyrics size", "% of height", 2.0, 12.0, 5.0, false, "How tall the line being sung is, as a share of the picture's height. The line before and the line to come are drawn smaller."),
    def(P::LyricsOffset, "lyrics_offset", "Lyrics sync offset", "ms", -1000.0, 1000.0, 0.0, false, "Moves the lyrics earlier (negative) or later (positive) against the music, in steps of 10 ms, for when they run ahead of or behind what you hear. Remembered separately for each music app, and not stored in presets."),
];

pub fn def_of(id: P) -> &'static Def {
    &DEFS[id as usize]
}

impl Def {
    pub fn to_norm(&self, value: f32) -> f32 {
        let v = value.clamp(self.min, self.max);
        if self.log { (v / self.min).ln() / (self.max / self.min).ln() } else { (v - self.min) / (self.max - self.min) }
    }

    pub fn from_norm(&self, norm: f32) -> f32 {
        let n = norm.clamp(0.0, 1.0);
        let v = if self.log { self.min * (self.max / self.min).powf(n) } else { self.min + n * (self.max - self.min) };
        match self.id {
            P::Palette => v.round(),
            // Whole steps of 10 ms.
            P::LyricsOffset => (v / 10.0).round() * 10.0,
            _ => v,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Mirror {
    Off,
    Quadrants,
    LeftRight,
    TopBottom,
}

impl Mirror {
    pub const ALL: [Mirror; 4] = [Mirror::Off, Mirror::Quadrants, Mirror::LeftRight, Mirror::TopBottom];

    pub fn label(self) -> &'static str {
        match self {
            Mirror::Off => "Off",
            Mirror::Quadrants => "Four quadrants",
            Mirror::LeftRight => "Left / right",
            Mirror::TopBottom => "Top / bottom",
        }
    }

    pub fn next(self) -> Self {
        Self::ALL[(self as usize + 1) % Self::ALL.len()]
    }
}

/// How a bass hit is drawn.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum BassStyle {
    /// A glowing disc that grows from the middle, over the picture.
    #[default]
    Pulse,
    /// Colour that fills only the black and nearly black parts, underneath the picture.
    Underlay,
}

impl BassStyle {
    pub const ALL: [BassStyle; 2] = [BassStyle::Pulse, BassStyle::Underlay];

    pub fn label(self) -> &'static str {
        match self {
            BassStyle::Pulse => "Pulse from the middle",
            BassStyle::Underlay => "Fill dark areas",
        }
    }
}

/// How frequency is laid out on screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Shape {
    /// x and y are both frequency; a pixel shows the pair.
    #[default]
    Cross,
    /// Distance from the centre is frequency: a falling sweep is a shrinking circle.
    Circle,
}

impl Shape {
    pub const ALL: [Shape; 2] = [Shape::Cross, Shape::Circle];

    pub fn label(self) -> &'static str {
        match self {
            Shape::Cross => "Cross",
            Shape::Circle => "Circle",
        }
    }

    pub fn next(self) -> Self {
        Self::ALL[(self as usize + 1) % Self::ALL.len()]
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum Toggle {
    FlipX,
    FlipY,
    Stereo,
    AutoGain,
    BassBoost,
    ReversePalette,
}

#[derive(Clone)]
pub struct Params {
    /// Where each setting is heading.
    target: [f32; N_PARAMS],
    /// What is in use this frame; glides toward `target` after controller moves.
    current: [f32; N_PARAMS],
    pub shape: Shape,
    pub mirror: Mirror,
    pub flip_x: bool,
    pub flip_y: bool,
    pub stereo: bool,
    pub auto_gain: bool,
    pub bass_boost: bool,
    pub reverse_palette: bool,
    /// Bass hits brighten the whole picture, as well as driving the pulse from the middle.
    pub bass_flare: bool,
    pub bass_style: BassStyle,
    /// Colour of the circle view's bass pulse; `None` uses the palette's own contrast colour.
    pub bass_colour: Option<[f32; 3]>,
    /// Colour for out-of-step (surround) sound in stereo; `None` uses the palette's own.
    pub surround_colour: Option<[f32; 3]>,
    /// Lyrics: show the line to come, small, under the one being sung.
    pub lyrics_preview: bool,
}

impl Default for Params {
    fn default() -> Self {
        let values = std::array::from_fn(|i| DEFS[i].default);
        Self {
            target: values,
            current: values,
            shape: Shape::Cross,
            mirror: Mirror::Quadrants,
            flip_x: false,
            flip_y: false,
            stereo: true,
            auto_gain: true,
            bass_boost: true,
            reverse_palette: false,
            bass_flare: false,
            bass_style: BassStyle::Pulse,
            bass_colour: None,
            surround_colour: Some([1.0, 0.0, 0.0]),
            lyrics_preview: true,
        }
    }
}

impl Params {
    pub fn get(&self, id: P) -> f32 {
        self.current[id as usize]
    }

    pub fn target(&self, id: P) -> f32 {
        self.target[id as usize]
    }

    /// Set immediately (GUI, presets).
    pub fn set(&mut self, id: P, value: f32) {
        let d = def_of(id);
        let v = d.from_norm(d.to_norm(value));
        self.target[id as usize] = v;
        self.current[id as usize] = v;
    }

    /// Set from a controller: the value in use glides there over a few frames.
    pub fn set_target_norm(&mut self, id: P, norm: f32) {
        self.target[id as usize] = def_of(id).from_norm(norm);
    }

    pub fn glide(&mut self, dt: f32) {
        let k = 1.0 - (-dt / 0.03).exp();
        for (i, d) in DEFS.iter().enumerate() {
            if d.id == P::Palette {
                self.current[i] = self.target[i];
            } else {
                self.current[i] += (self.target[i] - self.current[i]) * k;
            }
        }
    }

    pub fn toggle(&mut self, t: Toggle) {
        let flag = match t {
            Toggle::FlipX => &mut self.flip_x,
            Toggle::FlipY => &mut self.flip_y,
            Toggle::Stereo => &mut self.stereo,
            Toggle::AutoGain => &mut self.auto_gain,
            Toggle::BassBoost => &mut self.bass_boost,
            Toggle::ReversePalette => &mut self.reverse_palette,
        };
        *flag = !*flag;
    }

    pub fn palette(&self) -> usize {
        (self.get(P::Palette).round() as usize).min(N_PALETTES - 1)
    }

    pub fn step_palette(&mut self, by: i32) {
        let n = N_PALETTES as i32;
        self.set(P::Palette, ((self.palette() as i32 + by).rem_euclid(n)) as f32);
    }

    pub fn to_saved(&self) -> SavedParams {
        SavedParams {
            values: DEFS.iter().map(|d| (d.key.to_string(), self.target(d.id))).collect(),
            shape: self.shape,
            mirror: self.mirror,
            flip_x: self.flip_x,
            flip_y: self.flip_y,
            stereo: self.stereo,
            auto_gain: self.auto_gain,
            bass_boost: self.bass_boost,
            reverse_palette: self.reverse_palette,
            bass_flare: self.bass_flare,
            bass_style: self.bass_style,
            bass_colour: self.bass_colour,
            surround_colour: self.surround_colour,
            lyrics_preview: self.lyrics_preview,
        }
    }

    pub fn from_saved(saved: &SavedParams) -> Self {
        let mut p = Self {
            shape: saved.shape,
            mirror: saved.mirror,
            flip_x: saved.flip_x,
            flip_y: saved.flip_y,
            stereo: saved.stereo,
            auto_gain: saved.auto_gain,
            bass_boost: saved.bass_boost,
            reverse_palette: saved.reverse_palette,
            bass_flare: saved.bass_flare,
            bass_style: saved.bass_style,
            bass_colour: saved.bass_colour,
            surround_colour: saved.surround_colour,
            lyrics_preview: saved.lyrics_preview,
            ..Self::default()
        };
        for d in &DEFS {
            if let Some(v) = saved.values.get(d.key) {
                p.set(d.id, *v);
            }
        }
        p
    }
}

pub fn yes() -> bool {
    true
}

/// On-disk form. Values are keyed by name so presets survive new settings.
#[derive(Clone, Serialize, Deserialize)]
pub struct SavedParams {
    pub values: BTreeMap<String, f32>,
    #[serde(default)]
    pub shape: Shape,
    pub mirror: Mirror,
    pub flip_x: bool,
    pub flip_y: bool,
    pub stereo: bool,
    pub auto_gain: bool,
    pub bass_boost: bool,
    pub reverse_palette: bool,
    #[serde(default, alias = "circle_flare")]
    pub bass_flare: bool,
    #[serde(default)]
    pub bass_style: BassStyle,
    #[serde(default)]
    pub bass_colour: Option<[f32; 3]>,
    #[serde(default)]
    pub surround_colour: Option<[f32; 3]>,
    #[serde(default = "yes")]
    pub lyrics_preview: bool,
}

pub struct Palette {
    pub name: &'static str,
    /// Up to eight colours from quiet to loud; black is always below the first.
    pub stops: &'static [[f32; 3]],
    /// A contrasting colour for the bass pulse in the circle view.
    pub accent: [f32; 3],
}

pub const MAX_STOPS: usize = 8;

/// Per palette, the colour for out-of-step (surround) sound in stereo. Chosen
/// to stand apart from both the palette and its bass colour.
pub const SURROUND_COLOURS: [[f32; 3]; N_PALETTES] = [
    [0.25, 1.00, 0.35], // Ember
    [1.00, 0.25, 0.60], // Ice
    [0.55, 0.35, 1.00], // Aurora
    [1.00, 0.55, 0.10], // Neon
    [0.45, 0.60, 1.00], // Sunset
    [0.15, 0.75, 1.00], // Mono
    [1.00, 0.35, 0.85], // Rainbow
    [1.00, 1.00, 1.00], // Zigzag
    [0.60, 0.60, 0.60], // Candy
    [0.20, 0.85, 0.75], // Contour
];

pub const PALETTES: [Palette; N_PALETTES] = [
    Palette { name: "Ember", stops: &[[0.16, 0.03, 0.42], [0.80, 0.10, 0.30], [1.00, 0.55, 0.05], [1.00, 0.98, 0.80]], accent: [0.10, 0.80, 1.00] },
    Palette { name: "Ice", stops: &[[0.02, 0.08, 0.35], [0.05, 0.40, 0.80], [0.30, 0.85, 0.95], [0.95, 1.00, 1.00]], accent: [1.00, 0.50, 0.10] },
    Palette { name: "Aurora", stops: &[[0.05, 0.10, 0.30], [0.00, 0.50, 0.45], [0.40, 0.90, 0.30], [0.95, 1.00, 0.70]], accent: [1.00, 0.20, 0.70] },
    Palette { name: "Neon", stops: &[[0.25, 0.00, 0.45], [0.85, 0.05, 0.65], [0.20, 0.75, 1.00], [0.90, 1.00, 1.00]], accent: [0.85, 1.00, 0.20] },
    Palette { name: "Sunset", stops: &[[0.30, 0.02, 0.20], [0.85, 0.15, 0.15], [1.00, 0.60, 0.25], [1.00, 0.92, 0.65]], accent: [0.10, 0.90, 0.80] },
    Palette { name: "Mono", stops: &[[0.12, 0.12, 0.12], [0.40, 0.40, 0.40], [0.72, 0.72, 0.72], [1.00, 1.00, 1.00]], accent: [1.00, 0.15, 0.10] },
    // The rest are not gradients: each level band is its own colour. Turn up
    // "Colour banding" to make the edges between them hard.
    Palette {
        name: "Rainbow",
        stops: &[[0.45, 0.00, 0.80], [0.10, 0.20, 1.00], [0.00, 0.80, 0.90], [0.10, 0.85, 0.20], [1.00, 0.90, 0.10], [1.00, 0.50, 0.00], [1.00, 0.10, 0.10], [1.00, 1.00, 1.00]],
        accent: [1.00, 1.00, 1.00],
    },
    Palette {
        name: "Zigzag",
        stops: &[[0.10, 0.00, 0.30], [0.00, 0.30, 0.70], [0.50, 0.00, 0.60], [0.00, 0.70, 0.60], [0.90, 0.10, 0.30], [1.00, 0.70, 0.00], [1.00, 0.30, 0.10], [1.00, 1.00, 0.80]],
        accent: [0.30, 1.00, 0.30],
    },
    Palette {
        name: "Candy",
        stops: &[[0.90, 0.10, 0.50], [0.10, 0.70, 0.90], [1.00, 0.80, 0.10], [0.50, 0.20, 0.90], [0.20, 0.90, 0.40], [1.00, 0.40, 0.10], [0.20, 0.40, 1.00], [1.00, 1.00, 1.00]],
        accent: [1.00, 1.00, 1.00],
    },
    Palette {
        name: "Contour",
        stops: &[[0.15, 0.15, 0.20], [0.50, 0.50, 0.60], [0.20, 0.20, 0.30], [0.70, 0.70, 0.80], [0.30, 0.30, 0.40], [0.90, 0.90, 0.95], [0.50, 0.50, 0.60], [1.00, 1.00, 1.00]],
        accent: [1.00, 0.45, 0.00],
    },
];