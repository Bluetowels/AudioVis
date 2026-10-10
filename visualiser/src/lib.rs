//! AudioVis: a full-screen audio visualiser. Every pixel is a pair of
//! frequencies; its colour is how loud they are together. Silence is black.

mod analysis;
#[cfg(target_os = "android")]
mod android;
mod audio;
#[cfg_attr(not(windows), allow(dead_code))]
mod cover;
#[cfg_attr(not(windows), allow(dead_code))]
mod lyrics;
mod midi;
#[cfg_attr(not(windows), allow(dead_code))]
mod nowplaying;
mod params;
mod render;
mod surface;

use analysis::{BINS_PER_OCTAVE, BassMeter, F_MIN, N_BINS, Vqt};
use audio::{Capture, Channel, Source};
use eframe::egui;
use midi::{Bindings, Controller};
use params::{Backdrop, BassStyle, DEFS, LyricsPlace, Material, Mirror, P, PALETTES, Params, SavedParams, Shape, def_of};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Instant;

/// Bins quieter than this are treated as silent whatever the gain.
const GATE_DB: f32 = -90.0;
/// Bins this far below the loudest bin are also treated as silent. Without
/// this, a steep slope lifts the faint hiss at the top of the spectrum above
/// the music, auto-gain follows the hiss, and the picture goes dark.
const RELATIVE_GATE_DB: f32 = 70.0;
/// Auto-gain never turns up further than this reference level.
const AUTO_GAIN_FLOOR_DB: f32 = -55.0;
/// Stereo position is averaged over this long and this many bins either side
/// (six bins is two semitones), so whole instruments lean together.
const STEREO_SMOOTH_S: f32 = 0.15;
const STEREO_SMOOTH_BINS: f32 = 6.0;
const BASS_RANGE_DB: f32 = 30.0;
const BASS_GAIN: (f32, f32) = (0.50, 1.45);
const SOFT_FLOOR: f32 = 0.35;

#[derive(Serialize, Deserialize)]
struct Settings {
    params: SavedParams,
    bindings: Bindings,
    #[serde(default)]
    source: Option<Source>,
    #[serde(default = "params::yes")]
    show_hints: bool,
    #[serde(default)]
    show_fps: bool,
    #[serde(default)]
    show_fps_graph: bool,
    /// Names of the settings sections that are folded away.
    #[serde(default)]
    collapsed: std::collections::BTreeSet<String>,
    #[serde(default = "params::yes")]
    vsync: bool,
    #[serde(default)]
    show_surface: bool,
    /// Ask for HDR output on the next start.
    #[serde(default)]
    hdr: bool,
    /// Show lyrics. Off until asked for, because finding them sends the
    /// title and artist of what is playing to lrclib.net.
    #[serde(default)]
    lyrics: bool,
    /// The lyrics sync offset in ms for each music app, by the app's id.
    #[serde(default)]
    lyrics_offsets: std::collections::BTreeMap<String, f32>,
    /// Show the title and artist for a few seconds when a track starts.
    #[serde(default = "params::yes")]
    track_card: bool,
    /// Bring the title in from the distance instead of showing it in the corner.
    #[serde(default)]
    track_card_fly: bool,
}

#[cfg(windows)]
fn config_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("AudioVis")
}

#[cfg(target_os = "macos")]
fn config_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    home.join("Library/Application Support/AudioVis")
}

/// The app's own private storage, which Android tells it about at start.
#[cfg(target_os = "android")]
fn config_dir() -> PathBuf {
    android::data_dir()
}

#[cfg(not(any(windows, target_os = "macos", target_os = "android")))]
fn config_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("AudioVis")
}

/// The graphics API the app asks for, and its name for error messages.
#[cfg(windows)]
const GRAPHICS: (Option<&str>, &str) = (Some("vulkan"), "Vulkan");
#[cfg(target_os = "macos")]
const GRAPHICS: (Option<&str>, &str) = (Some("metal"), "Metal");
#[cfg(target_os = "android")]
const GRAPHICS: (Option<&str>, &str) = (Some("vulkan"), "Vulkan");
#[cfg(not(any(windows, target_os = "macos", target_os = "android")))]
const GRAPHICS: (Option<&str>, &str) = (None, "Vulkan or OpenGL");

/// What the operating system is called in the panel.
#[cfg(windows)]
const SYSTEM_NAME: &str = "Windows";
#[cfg(target_os = "macos")]
const SYSTEM_NAME: &str = "macOS";
#[cfg(not(any(windows, target_os = "macos")))]
#[cfg_attr(target_os = "android", allow(dead_code))]
const SYSTEM_NAME: &str = "system";

/// With no saved settings the app listens to this. On Android every real
/// source needs a permission, so a first run shows the test signal instead of
/// opening on a permission prompt.
#[cfg(not(target_os = "android"))]
const FIRST_SOURCE: Source = Source::SystemOutput;
#[cfg(target_os = "android")]
const FIRST_SOURCE: Source = Source::TestSignal;

/// The controller looked for when none is named. Android has no command
/// line to name another, so there it is whichever controller is plugged in.
#[cfg(not(target_os = "android"))]
const MIDI_PORT: &str = "nanoKONTROL2";
#[cfg(target_os = "android")]
const MIDI_PORT: &str = "";

fn preset_names() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(config_dir().join("presets"))
        .map(|dir| {
            dir.filter_map(|e| e.ok())
                .filter_map(|e| e.path().file_stem().map(|s| s.to_string_lossy().into_owned()))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

struct Options {
    /// Overrides the saved audio source.
    source: Option<Source>,
    vsync: bool,
    fullscreen: bool,
    show_panel: bool,
    /// Start in the circle view (for self-tests, which ignore saved settings).
    circle: bool,
    /// Self-test: show the FPS counter and graph.
    fps: bool,
    /// Ask for HDR output on this run.
    hdr: bool,
    /// Start with the picture of the controller showing.
    surface: bool,
    stereo: bool,
    /// Self-test: draw bass as a fill of the dark areas.
    underlay: bool,
    overrides: Vec<(String, f32)>,
    /// Self-test: show this slider's description as if the pointer were resting on it.
    demo_hint: Option<String>,
    /// Show lyrics on this run, whatever the saved setting.
    lyrics: bool,
    /// Where to draw them on this run.
    lyrics_place: Option<LyricsPlace>,
    lyrics_justify: bool,
    /// Self-test: colours from the cover, a background and a 3D material.
    cover_colours: bool,
    track_card: bool,
    track_card_fly: bool,
    /// Self-test: fly the title in at the start, as F5 does.
    fly_title_now: bool,
    stars: bool,
    backdrop: Option<Backdrop>,
    material: Option<Material>,
    /// Show the lyrics in this LRC file (or the one beside this audio file,
    /// with the same name), timed from when the app starts,
    /// instead of looking up what is playing.
    lyrics_file: Option<PathBuf>,
    midi_port: String,
    /// Take a screenshot to this path after `seconds`, print timings and exit.
    selftest: Option<PathBuf>,
    seconds: f32,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            source: None,
            vsync: true,
            fullscreen: false,
            show_panel: true,
            circle: false,
            fps: false,
            hdr: false,
            surface: false,
            stereo: false,
            underlay: false,
            overrides: Vec::new(),
            demo_hint: None,
            lyrics: false,
            lyrics_place: None,
            lyrics_justify: false,
            cover_colours: false,
            track_card: false,
            track_card_fly: false,
            fly_title_now: false,
            stars: false,
            backdrop: None,
            material: None,
            lyrics_file: None,
            midi_port: MIDI_PORT.into(),
            selftest: None,
            seconds: 3.0,
        }
    }
}

#[cfg(not(target_os = "android"))]
fn parse_options() -> Options {
    let mut o = Options::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--test-signal" => o.source = Some(Source::TestSignal),
            "--list-devices" => {
                println!("Output devices:");
                audio::output_device_names().iter().for_each(|n| println!("  {n}"));
                println!("Input devices:");
                audio::input_device_names().iter().for_each(|n| println!("  {n}"));
                std::process::exit(0);
            }
            "--default-output" => o.source = Some(Source::SystemOutput),
            // Output devices to capture, by exact name, separated by semicolons.
            "--outputs" => {
                o.source = args.next().map(|s| Source::Outputs(s.split(';').map(|n| n.trim().to_string()).collect()))
            }
            "--no-vsync" => o.vsync = false,
            "--circle" => o.circle = true,
            "--fps" => o.fps = true,
            "--hdr" => o.hdr = true,
            "--controller" => o.surface = true,
            "--stereo" => o.stereo = true,
            "--underlay" => o.underlay = true,
            "--show-hint" => o.demo_hint = args.next(),
            "--lyrics" => o.lyrics = true,
            "--lyrics-justify" => o.lyrics_justify = true,
            // Self-test helpers for the switches that are not sliders.
            "--cover-colours" => o.cover_colours = true,
            "--track-card" => o.track_card = true,
            "--track-card-fly" => o.track_card_fly = true,
            "--fly-title" => o.fly_title_now = true,
            "--stars" => o.stars = true,
            "--background" => {
                o.backdrop = args.next().and_then(|name| Backdrop::ALL.into_iter().find(|b| b.label().eq_ignore_ascii_case(&name)));
            }
            "--material" => {
                o.material = args.next().and_then(|name| Material::ALL.into_iter().find(|m| m.label().eq_ignore_ascii_case(&name)));
            }
            "--lyrics-place" => {
                o.lyrics_place = args.next().and_then(|name| LyricsPlace::ALL.into_iter().find(|place| place.label().eq_ignore_ascii_case(&name)));
            }
            "--lyrics-file" => {
                o.lyrics_file = args.next().map(PathBuf::from);
                o.lyrics = true;
            }
            // Self-test helpers: set any slider by key, e.g. --set bass_amount=2
            "--set" => {
                if let Some((key, value)) = args.next().as_deref().and_then(|s| s.split_once('=')) {
                    o.overrides.push((key.to_string(), value.parse().unwrap_or(0.0)));
                }
            }
            "--fullscreen" => o.fullscreen = true,
            "--hide-panel" => o.show_panel = false,
            "--midi-port" => o.midi_port = args.next().unwrap_or_default(),
            "--selftest" => o.selftest = args.next().map(PathBuf::from),
            "--seconds" => o.seconds = args.next().and_then(|s| s.parse().ok()).unwrap_or(3.0),
            other => eprintln!("ignoring unknown option {other}"),
        }
    }
    o
}

/// The desktop program: read the command line, open the window and run
/// until it is closed.
#[cfg(not(target_os = "android"))]
pub fn run() -> eframe::Result {
    // Started from a terminal, print there (--list-devices, --selftest),
    // unless the output is already going to a file or a pipe.
    // SAFETY: plain Win32 calls; attaching fails harmlessly when there is no terminal.
    #[cfg(windows)]
    unsafe {
        if GetStdHandle(STD_OUTPUT_HANDLE).is_null() {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
    // Without a console a panic would otherwise close the app without a word.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_hook(info);
        message_box(&format!("AudioVis hit a problem and has to close.

{info}"));
    }));
    start(parse_options(), eframe::NativeOptions::default())
}

/// Android starts the app here, on a thread of its own, each time it opens
/// the app's screen.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: android_activity::AndroidApp) {
    android::attach(&app);
    std::panic::set_hook(Box::new(|info| android::log(&format!("AudioVis hit a problem and has to close: {info}"))));
    let mut native = eframe::NativeOptions { android_app: Some(app), ..Default::default() };
    // Phones differ in the largest picture their graphics chip can hold, so
    // ask for what this one has instead of the desktop figure.
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut native.wgpu_options.wgpu_setup {
        use eframe::egui_wgpu::wgpu;
        setup.device_descriptor = std::sync::Arc::new(|adapter| wgpu::DeviceDescriptor {
            label: Some("audiovis device"),
            required_limits: wgpu::Limits {
                max_texture_dimension_2d: adapter.limits().max_texture_dimension_2d.min(8192),
                ..Default::default()
            },
            ..Default::default()
        });
    }
    let _ = start(Options::default(), native);
}

/// Open the window and run the app in it until it is closed.
fn start(options: Options, mut native: eframe::NativeOptions) -> eframe::Result {
    // The app is built for Vulkan on a discrete GPU on Windows and on Android, and Metal on macOS.
    if let (Some(backend), None) = (GRAPHICS.0, std::env::var_os("WGPU_BACKEND")) {
        // SAFETY: nothing else is running yet.
        unsafe { std::env::set_var("WGPU_BACKEND", backend) };
    }
    if std::env::var_os("WGPU_POWER_PREF").is_none() {
        unsafe { std::env::set_var("WGPU_POWER_PREF", "high") };
    }

    native.viewport = egui::ViewportBuilder::default().with_title("AudioVis").with_inner_size([1280.0, 720.0]);
    native.renderer = eframe::Renderer::Wgpu;
    // Vsync is fixed when the window is created, so the saved choice is read here.
    let saved_vsync = std::fs::read_to_string(config_dir().join("settings.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|v| v.get("vsync").and_then(|v| v.as_bool()))
        .unwrap_or(true);
    let vsync_active = options.vsync && (saved_vsync || options.selftest.is_some());

    // HDR is also fixed when the window is created. The copy of egui-wgpu in
    // vendor/ reads this when it chooses the surface.
    let saved: Option<serde_json::Value> = std::fs::read_to_string(config_dir().join("settings.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok());
    let saved_hdr = saved.as_ref().and_then(|v| v.get("hdr")).and_then(|v| v.as_bool()).unwrap_or(false);
    if cfg!(windows) && (options.hdr || (saved_hdr && options.selftest.is_none())) {
        let base = saved
            .as_ref()
            .and_then(|v| v.pointer("/params/values/hdr_base"))
            .and_then(|v| v.as_f64())
            .unwrap_or(200.0);
        eframe::egui_wgpu::HDR_UI_SCALE.store(((base / 80.0) as f32).to_bits(), std::sync::atomic::Ordering::Relaxed);
        // SAFETY: nothing else is running yet.
        unsafe { std::env::set_var("AUDIOVIS_HDR", "1") };
    }
    native.wgpu_options.surface.present_mode = if vsync_active {
        // Plain Fifo, not AutoVsync: that prefers a mode which shows a late
        // frame immediately, tearing the picture whenever a frame runs over.
        eframe::egui_wgpu::wgpu::PresentMode::Fifo
    } else {
        eframe::egui_wgpu::wgpu::PresentMode::AutoNoVsync
    };

    let result = eframe::run_native(
        "AudioVis",
        native,
        Box::new(|cc| {
            if cc.wgpu_render_state.is_none() {
                return Err("the wgpu renderer is not available".into());
            }
            Ok(Box::new(App::new(cc, options, saved_vsync, vsync_active, saved_hdr)))
        }),
    );
    if let Err(e) = &result {
        // Started from a shortcut there is no console to read the error in.
        message_box(&format!(
            "AudioVis couldn't start the graphics card. It needs a GPU with {} support and an up-to-date driver.\n\n{e}",
            GRAPHICS.1
        ));
    }
    result
}

#[cfg(windows)]
#[link(name = "user32")]
unsafe extern "system" {
    fn MessageBoxW(window: *mut std::ffi::c_void, text: *const u16, caption: *const u16, kind: u32) -> i32;
}

#[cfg(windows)]
const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
#[cfg(windows)]
const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn AttachConsole(process: u32) -> i32;
    fn GetStdHandle(which: u32) -> *mut std::ffi::c_void;
}

/// Show an error in a standard Windows message box and wait for OK.
#[cfg(windows)]
fn message_box(text: &str) {
    const MB_ICONERROR: u32 = 0x10;
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (text, caption) = (wide(text), wide("AudioVis"));
    // SAFETY: both strings are NUL-terminated and outlive the call.
    unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), MB_ICONERROR) };
}

/// Show an error in a standard macOS alert and wait for OK.
#[cfg(target_os = "macos")]
fn message_box(text: &str) {
    // The text is passed as an argument, so nothing in it is read as script.
    let script = ["on run argv", r#"display alert "AudioVis" message (item 1 of argv) as critical"#, "end run"];
    let _ = std::process::Command::new("/usr/bin/osascript")
        .args(script.into_iter().flat_map(|line| ["-e", line]))
        .arg("--")
        .arg(text)
        .status();
}

/// Android has no simple message box; the error goes to the system log.
#[cfg(target_os = "android")]
fn message_box(text: &str) {
    android::log(text);
}

/// The error has already gone to the terminal, which is all there is here.
#[cfg(not(any(windows, target_os = "macos", target_os = "android")))]
fn message_box(_text: &str) {}

/// The description shown beside whichever control the pointer is resting on.
#[derive(Default)]
struct Hint {
    /// Control under the pointer this frame, if it has a description.
    hovered: Option<(egui::Id, String, egui::Rect)>,
    /// The control being timed or shown, and when the pointer arrived on it.
    current: Option<(egui::Id, String, egui::Rect, Instant)>,
}

/// Frames kept for the FPS graph (two seconds at 120 fps).
const FPS_HISTORY: usize = 240;

const HINT_DELAY_S: f32 = 0.45;
const HINT_FADE_S: f32 = 0.18;

struct App {
    options: Options,
    params: Params,
    bindings: Bindings,
    capture: Capture,
    controller: Controller,
    vqt: Vqt,
    /// A bass window and detail the sliders have asked for, and when they last moved.
    window_change: Option<(f32, f32, Instant)>,
    sharpen_scratch: Vec<f32>,
    /// 3D camera angle round the centre, in radians.
    orbit_angle: f32,
    /// Seconds the last frame took, for the rain simulation.
    frame_dt: f32,
    /// 3D camera position and the point it looks at.
    camera: ([f32; 3], [f32; 3]),
    /// Smoothed height of the ground the flight has to stay above.
    clearance: f32,
    /// Width over height of the picture area, from the last frame drawn.
    aspect: f32,
    bass: BassMeter,
    samples: Vec<f32>,
    fresh: Vec<f32>,
    db: [Vec<f32>; 2],
    left: Vec<analysis::Complex32>,
    right: Vec<analysis::Complex32>,
    /// Per bin, smoothed over time: left power, right power, their geometric
    /// mean, and how much they move together.
    stereo_energy: [Vec<f32>; 4],
    /// The same, also smoothed across neighbouring bins.
    stereo_blurred: [Vec<f32>; 4],
    /// Stereo position per bin, -1 left .. 1 right.
    pan: Vec<f32>,
    /// How far left and right are out of step per bin, 0 .. 1.
    wide: Vec<f32>,
    shown: [Vec<f32>; 2],
    scratch: Vec<f32>,
    reference_db: f32,
    bass_reference_db: f32,
    bass_env: f32,
    last_frame: Instant,
    started: Instant,
    frame_ms: f32,
    analysis_ms: f32,
    frames: u64,
    show_panel: bool,
    fullscreen: bool,
    learning: Option<P>,
    hint: Hint,
    show_hints: bool,
    show_fps: bool,
    show_fps_graph: bool,
    collapsed: std::collections::BTreeSet<String>,
    /// Whether the picture of the controller is showing.
    show_surface: bool,
    /// Vsync as chosen in the panel, and as this run was started with.
    vsync: bool,
    vsync_active: bool,
    /// HDR output as chosen in the panel, and whether this run is drawing in HDR.
    hdr: bool,
    hdr_active: bool,
    /// Show the HDR test pattern over the picture.
    hdr_pattern: bool,
    /// Seconds of 3D flight flown so far, scaled by the flight speed.
    flight_time: f32,
    /// What another app is playing, as of this frame, if anything here wants to know.
    playing: Option<nowplaying::Now>,
    /// The colours the picture is being drawn in.
    look: cover::Look,
    /// The colours taken from a cover, and which cover (by its address).
    cover_look: Option<(usize, cover::Look)>,
    /// How far palette drift has gone, in palettes.
    palette_drift: f32,
    /// How far the flight through the stars has gone; 0 when it is off.
    star_travel: f32,
    /// How fast that flight is going, in sheets of stars passed each second.
    star_rate: f32,
    /// Where the flight is heading, from the middle of the picture in picture
    /// heights, and how far the stars have slid sideways in its turns.
    star_heading: [f32; 2],
    star_slide: [f32; 2],
    /// Bass hits so far, each of which turns the flight a new way, and the
    /// bass level last frame, to tell when a hit starts.
    star_hits: u32,
    star_bass_was: f32,
    /// How hard the flight is turning (0 to 1), and how long it has been held there.
    star_turn_level: f32,
    star_turn_held: f32,
    /// How far the flight is being held steady for a title flying in (0 to 1).
    star_steady: f32,
    /// Which cover the renderer was last given (by its address; 0 for none).
    cover_sent: usize,
    /// Whether the title and artist are shown as a track starts.
    track_card: bool,
    track_card_fly: bool,
    /// The track on the card, and when it came on.
    card: Option<(nowplaying::Track, Instant)>,
    /// How near a title flying in has come: 0 a dot in the distance, 1 filling the picture.
    card_near: f32,
    /// Whether the card now showing is flying in or sitting in the corner.
    card_flying: bool,
    /// A fly-in has been asked for (F5 or a controller button) and not yet begun.
    card_fly_asked: bool,
    /// The cover as a picture the interface can draw, and which cover (by its address).
    card_cover: Option<(usize, egui::TextureHandle)>,
    /// Whether lyrics are shown.
    lyrics: bool,
    /// Watches what other apps are playing; started the first time lyrics are on.
    now_playing: Option<nowplaying::NowPlaying>,
    lookup: lyrics::Lookup,
    /// Lyrics given on the command line, shown instead of looking any up.
    lyrics_file: Option<std::sync::Arc<[lyrics::Line]>>,
    /// The track lyrics are being shown for, for the panel.
    lyrics_track: Option<nowplaying::Track>,
    /// The app whose sync offset is on the slider, and every app's offset.
    lyrics_app: String,
    lyrics_offsets: std::collections::BTreeMap<String, f32>,
    /// Recent frame times in ms, newest last, for the graph.
    frame_history: std::collections::VecDeque<f32>,
    preset_name: String,
    presets: Vec<String>,
    inputs: Vec<String>,
    outputs: Vec<String>,
    status: Vec<audio::TapStatus>,
    status_at: Instant,
    adapter: String,
    shot_requested: bool,
    /// Frame times in ms over the last three seconds of a self-test.
    recent: Vec<f32>,
    /// How far across the picture a finger has swiped so far, in points.
    #[cfg(target_os = "android")]
    swipe: f32,
    /// The settings as last written to storage, and when they were last checked.
    #[cfg(target_os = "android")]
    saved: (String, Instant),
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, options: Options, vsync: bool, vsync_active: bool, hdr: bool) -> Self {
        let options_hdr = options.hdr;
        let hdr_active = cc.wgpu_render_state.as_ref().is_some_and(|r| r.target_format == eframe::egui_wgpu::wgpu::TextureFormat::Rgba16Float);
        let render_state = cc.wgpu_render_state.as_ref().expect("the wgpu renderer is required");
        render::init(render_state);
        let info = render_state.adapter.get_info();
        let adapter = format!("{} ({:?})", info.name, info.backend);

        let saved: Option<Settings> = std::fs::read_to_string(config_dir().join("settings.json"))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok());
        let show_surface = options.surface || (options.selftest.is_none() && saved.as_ref().is_some_and(|s| s.show_surface));
        let track_card_fly = options.track_card_fly || (options.selftest.is_none() && saved.as_ref().is_some_and(|s| s.track_card_fly));
        let track_card = options.track_card || options.track_card_fly || (options.selftest.is_none() && saved.as_ref().is_none_or(|s| s.track_card));
        let lyrics = options.lyrics || (options.selftest.is_none() && saved.as_ref().is_some_and(|s| s.lyrics));
        let lyrics_offsets = saved.as_ref().filter(|_| options.selftest.is_none()).map(|s| s.lyrics_offsets.clone()).unwrap_or_default();
        let lyrics_file = options.lyrics_file.as_deref().and_then(lyrics::sidecar).map(Into::into);
        let (mut params, mut bindings, saved_source, show_hints, show_fps, show_fps_graph, collapsed) = match saved {
            // A self-test always runs on defaults so results are comparable.
            Some(s) if options.selftest.is_none() => {
                (Params::from_saved(&s.params), s.bindings, s.source, s.show_hints, s.show_fps, s.show_fps_graph, s.collapsed)
            }
            _ => (Params::default(), Bindings::default(), None, true, options.fps, options.fps, Default::default()),
        };

        bindings.add_new_defaults();
        if options.circle {
            params.shape = Shape::Circle;
        }
        if options.stereo {
            params.stereo = true;
        }
        if let Some(place) = options.lyrics_place {
            params.lyrics_place = place;
        }
        params.palette_from_cover |= options.cover_colours;
        params.lyrics_justify |= options.lyrics_justify;
        params.stars |= options.stars;
        params.backdrop = options.backdrop.unwrap_or(params.backdrop);
        params.material = options.material.unwrap_or(params.material);
        if options.underlay {
            params.bass_style = BassStyle::Underlay;
        }
        for (key, value) in &options.overrides {
            if let Some(d) = DEFS.iter().find(|d| d.key == key) {
                params.set(d.id, *value);
            }
        }

        let capture = Capture::open(options.source.clone().or(saved_source).unwrap_or(FIRST_SOURCE));
        let sample_rate = capture.sample_rate();
        let (bass_window_s, detail) = (params.target(P::BassWindow) / 1000.0, params.target(P::Detail));
        let look = palette_look(&params);
        let fly_title_now = options.fly_title_now;
        crawl_font(&cc.egui_ctx);
        Self {
            show_panel: options.show_panel,
            fullscreen: options.fullscreen,
            controller: Controller::open(&options.midi_port),
            options,
            params,
            bindings,
            capture,
            vqt: Vqt::new(sample_rate, bass_window_s, detail),
            window_change: None,
            sharpen_scratch: Vec::new(),
            orbit_angle: 0.0,
            frame_dt: 0.0,
            camera: ([0.0, 0.0, 1.3], [0.0; 3]),
            clearance: 0.0,
            aspect: 16.0 / 9.0,
            bass: BassMeter::new(sample_rate),
            samples: Vec::new(),
            fresh: Vec::new(),
            db: [vec![analysis::SILENCE_DB; N_BINS], vec![analysis::SILENCE_DB; N_BINS]],
            left: vec![analysis::Complex32::default(); N_BINS],
            right: vec![analysis::Complex32::default(); N_BINS],
            stereo_energy: std::array::from_fn(|_| vec![0.0; N_BINS]),
            stereo_blurred: std::array::from_fn(|_| vec![0.0; N_BINS]),
            pan: vec![0.0; N_BINS],
            wide: vec![0.0; N_BINS],
            shown: [vec![0.0; N_BINS], vec![0.0; N_BINS]],
            scratch: vec![0.0; N_BINS],
            reference_db: AUTO_GAIN_FLOOR_DB,
            bass_reference_db: AUTO_GAIN_FLOOR_DB,
            bass_env: 0.0,
            last_frame: Instant::now(),
            started: Instant::now(),
            frame_ms: 0.0,
            analysis_ms: 0.0,
            frames: 0,
            learning: None,
            hint: Hint::default(),
            show_hints,
            show_fps,
            show_fps_graph,
            collapsed,
            show_surface,
            vsync,
            vsync_active,
            hdr: hdr || (options_hdr && hdr_active),
            hdr_active,
            hdr_pattern: false,
            flight_time: 0.0,
            playing: None,
            look,
            cover_look: None,
            palette_drift: 0.0,
            star_travel: 0.0,
            star_rate: 0.0,
            star_heading: [0.0; 2],
            star_slide: [0.0; 2],
            star_hits: 0,
            star_bass_was: 0.0,
            star_turn_level: 0.0,
            star_turn_held: 0.0,
            star_steady: 0.0,
            cover_sent: 0,
            track_card,
            track_card_fly,
            card: None,
            card_near: 0.0,
            card_flying: false,
            card_fly_asked: fly_title_now,
            card_cover: None,
            lyrics,
            now_playing: None,
            lookup: lyrics::Lookup::new(config_dir().join("lyrics")),
            lyrics_file,
            lyrics_track: None,
            lyrics_app: String::new(),
            lyrics_offsets,
            frame_history: std::collections::VecDeque::with_capacity(FPS_HISTORY),
            preset_name: String::new(),
            presets: preset_names(),
            inputs: audio::input_device_names(),
            outputs: audio::output_device_names(),
            status: Vec::new(),
            status_at: Instant::now(),
            adapter,
            shot_requested: false,
            recent: Vec::new(),
            #[cfg(target_os = "android")]
            swipe: 0.0,
            #[cfg(target_os = "android")]
            saved: (String::new(), Instant::now()),
        }
    }

    fn set_source(&mut self, source: Source) {
        self.capture = Capture::open(source);
        self.status.clear();
    }

    fn settings_text(&self) -> Option<String> {
        let settings = Settings {
            params: self.params.to_saved(),
            bindings: self.bindings.clone(),
            source: Some(self.capture.source.clone()),
            show_hints: self.show_hints,
            show_fps: self.show_fps,
            show_fps_graph: self.show_fps_graph,
            collapsed: self.collapsed.clone(),
            vsync: self.vsync,
            show_surface: self.show_surface,
            hdr: self.hdr,
            lyrics: self.lyrics,
            lyrics_offsets: self.lyrics_offsets.clone(),
            track_card: self.track_card,
            track_card_fly: self.track_card_fly,
        };
        serde_json::to_string_pretty(&settings).ok()
    }

    fn save_settings(&self) {
        let dir = config_dir();
        if std::fs::create_dir_all(&dir).is_ok() {
            if let Some(text) = self.settings_text() {
                let _ = std::fs::write(dir.join("settings.json"), text);
            }
        }
    }

    /// Android can end the app without a word, so there the settings are
    /// written whenever they have changed, checked every few seconds.
    #[cfg(target_os = "android")]
    fn save_changes(&mut self) {
        if self.saved.1.elapsed().as_secs_f32() < 3.0 {
            return;
        }
        self.saved.1 = Instant::now();
        if let Some(text) = self.settings_text().filter(|text| *text != self.saved.0) {
            self.save_settings();
            self.saved.0 = text;
        }
    }

    /// Touch stands in for the keyboard and mouse: a tap on the picture hides
    /// or shows the panel, and a swipe across it steps through the palettes.
    /// (Pressing and holding is a right-click, which egui sees to itself.)
    #[cfg(target_os = "android")]
    fn touch(&mut self, picture: &egui::Response) {
        const SWIPE_POINTS: f32 = 60.0;
        if picture.clicked() {
            self.show_panel = !self.show_panel;
        }
        self.swipe += picture.drag_delta().x;
        if picture.drag_stopped() {
            if self.swipe.abs() > SWIPE_POINTS {
                self.params.step_palette(if self.swipe < 0.0 { 1 } else { -1 });
                self.controller.release(&self.bindings, P::Palette);
            }
            self.swipe = 0.0;
        }
    }

    /// The choice of where the sound comes from, which on Android is a short
    /// fixed list: there are no devices to pick between.
    #[cfg(target_os = "android")]
    fn android_sources(&mut self, ui: &mut egui::Ui) {
        let current = self.capture.source.clone();
        let mut chosen = current.clone();
        for source in [Source::SystemOutput, Source::Input(String::new()), Source::File, Source::TestSignal] {
            let label = source.label();
            ui.radio_value(&mut chosen, source, label);
        }
        if chosen != current {
            self.set_source(chosen);
        }
        match current {
            Source::SystemOutput => {
                ui.small("Android asks each time before letting the app hear other apps, and some apps (many music streaming services) do not allow it. If the picture stays black, use the microphone or a file.");
            }
            Source::File => {
                if ui.button("Choose a file").clicked() {
                    android::pick_file();
                }
            }
            _ => {}
        }
    }

    fn save_preset(&mut self) {
        let name: String = self.preset_name.chars().filter(|c| c.is_alphanumeric() || " -_".contains(*c)).collect();
        if name.trim().is_empty() {
            return;
        }
        let dir = config_dir().join("presets");
        if std::fs::create_dir_all(&dir).is_ok() {
            if let Ok(text) = serde_json::to_string_pretty(&self.params.to_saved()) {
                let _ = std::fs::write(dir.join(format!("{}.json", name.trim())), text);
            }
        }
        self.presets = preset_names();
    }

    fn load_preset(&mut self, name: &str) {
        let path = config_dir().join("presets").join(format!("{name}.json"));
        if let Some(saved) = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str::<SavedParams>(&t).ok()) {
            // The lyrics offset belongs to the music app, not the preset.
            let offset = self.params.target(P::LyricsOffset);
            self.params = Params::from_saved(&saved);
            self.params.set(P::LyricsOffset, offset);
            self.controller.release_all();
            self.preset_name = name.to_string();
        }
    }

    /// Turn the newest audio into one level per bin for each axis, plus the
    /// bass-driven gain and glow. Returns (gain, glow).
    fn analyse(&mut self, dt: f32) -> (f32, f32) {
        let t0 = Instant::now();
        self.capture.pad_silence();
        let stereo = self.params.stereo;
        let sample_rate = self.capture.sample_rate();
        // Rebuilding the transform takes a moment, so wait until the slider has settled.
        let window_s = self.params.target(P::BassWindow) / 1000.0;
        let detail = self.params.target(P::Detail);
        let mut rebuild = sample_rate != self.vqt.sample_rate;
        if (window_s - self.vqt.max_window_s).abs() > 0.0005 || (detail - self.vqt.detail).abs() > 0.001 {
            match self.window_change {
                Some((w, d, since)) if (w - window_s).abs() < 0.0005 && (d - detail).abs() < 0.001 => {
                    rebuild |= since.elapsed().as_secs_f32() > 0.3
                }
                _ => self.window_change = Some((window_s, detail, Instant::now())),
            }
        }
        if rebuild {
            if sample_rate != self.vqt.sample_rate {
                self.bass = BassMeter::new(sample_rate);
            }
            self.vqt = Vqt::new(sample_rate, window_s, detail);
            self.window_change = None;
        }
        self.capture.read_fresh(&mut self.fresh);
        let n = self.vqt.max_n;
        if stereo {
            // Both channels separately, then per bin: combined loudness, where it
            // sits between left and right, and how far the two are out of step.
            self.capture.read_last(n, Channel::Left, &mut self.samples);
            self.vqt.analyse_complex(&self.samples, &mut self.left);
            self.capture.read_last(n, Channel::Right, &mut self.samples);
            self.vqt.analyse_complex(&self.samples, &mut self.right);
            // Position and out-of-step are taken from energy averaged over about
            // 150 ms and a few semitones either side. Measured per bin and per
            // frame they jitter, and neighbouring bins lean opposite ways.
            let keep = (-dt / STEREO_SMOOTH_S).exp();
            for b in 0..N_BINS {
                let (l, r) = (self.left[b], self.right[b]);
                let (pl, pr) = (l.norm_sqr(), r.norm_sqr());
                self.db[0][b] = (10.0 * (0.5 * (pl + pr)).max(1e-14).log10()).max(analysis::SILENCE_DB);
                let now = [pl, pr, (pl * pr).sqrt(), (l * r.conj()).re];
                for (smoothed, value) in self.stereo_energy.iter_mut().zip(now) {
                    smoothed[b] = value + (smoothed[b] - value) * keep;
                }
            }
            for (smoothed, blurred) in self.stereo_energy.iter().zip(self.stereo_blurred.iter_mut()) {
                blurred.copy_from_slice(smoothed);
                blur(blurred, STEREO_SMOOTH_BINS);
            }
            let [pl, pr, both, together] = &self.stereo_blurred;
            for b in 0..N_BINS {
                let total = pl[b] + pr[b];
                (self.pan[b], self.wide[b]) = if total > 1e-9 {
                    ((pr[b] - pl[b]) / total, ((both[b] - together[b]) / total).clamp(0.0, 1.0))
                } else {
                    (0.0, 0.0)
                };
            }
        } else {
            self.capture.read_last(n, Channel::Mono, &mut self.samples);
            self.vqt.analyse(&self.samples, &mut self.db[0]);
            self.pan.fill(0.0);
            self.wide.fill(0.0);
        }
        let (x, y) = self.db.split_at_mut(1);
        y[0].copy_from_slice(&x[0]);

        let p = &self.params;
        let slope = p.get(P::Slope) / BINS_PER_OCTAVE as f32;
        let range = p.get(P::Range);
        let fall = p.get(P::AutoGainSpeed) * dt;

        // The slope tilts about the middle of the spectrum, so it lifts treble
        // and lowers bass by the same amount instead of only adding level.
        let tilt = |b: usize| slope * (b as f32 - 0.5 * N_BINS as f32);
        let loudest = self.db.iter().flatten().copied().fold(f32::MIN, f32::max);
        let gate = GATE_DB.max(loudest - RELATIVE_GATE_DB);

        // Reference level: the loudest tilted bin that is really sounding,
        // falling back slowly, or the manual setting.
        let peak = self
            .db
            .iter()
            .flat_map(|row| row.iter().enumerate().filter(|(_, db)| **db > gate).map(|(b, db)| db + tilt(b)))
            .fold(f32::MIN, f32::max);
        self.reference_db = if p.auto_gain {
            peak.max(self.reference_db - fall).max(AUTO_GAIN_FLOOR_DB)
        } else {
            p.get(P::Reference)
        };

        let decay_ms = p.get(P::Decay);
        let keep = if decay_ms > 1.0 { (-dt * 1000.0 / decay_ms).exp() } else { 0.0 };
        let sigma = p.get(P::Smoothing);
        for (row, shown) in self.db.iter().zip(self.shown.iter_mut()) {
            for (b, db) in row.iter().enumerate() {
                let tilted = db + tilt(b);
                let level = ((tilted - (self.reference_db - range)) / range).clamp(0.0, 1.0);
                self.scratch[b] = if *db > gate { level } else { 0.0 };
            }
            analysis::sharpen(
                &mut self.scratch,
                &self.vqt.blur,
                &self.vqt.cap_blur,
                p.get(P::BassSharpen),
                p.get(P::Sharpen),
                &mut self.sharpen_scratch,
            );
            if sigma > 0.05 {
                blur(&mut self.scratch, sigma);
            }
            for (s, level) in shown.iter_mut().zip(&self.scratch) {
                *s = level.max(*s * keep);
            }
        }

        // Bass drive, measured on the waveform so it is quick.
        self.bass.set_cutoff(p.get(P::BassCutoff));
        let bass_db = self.bass.feed(&self.fresh);
        self.bass_reference_db = if p.auto_gain {
            bass_db.max(self.bass_reference_db - fall).max(AUTO_GAIN_FLOOR_DB)
        } else {
            p.get(P::Reference)
        };
        let hit = ((bass_db - (self.bass_reference_db - BASS_RANGE_DB)) / BASS_RANGE_DB).clamp(0.0, 1.0);
        let release = (-dt * 1000.0 / p.get(P::BassRelease)).exp();
        self.bass_env = hit.max(self.bass_env * release);

        let amount = if p.bass_boost { p.get(P::BassAmount) } else { 0.0 };
        // The bass pulse from the middle is drawn by the shader from `bass_env`
        // (see `uniforms`); this is the optional brightening of the whole picture.
        let flare = (1.0 + amount * (BASS_GAIN.0 + (BASS_GAIN.1 - BASS_GAIN.0) * self.bass_env - 1.0)).max(0.0);
        let (gain, glow) = match p.shape {
            _ if !p.bass_flare => (1.0, 0.0),
            Shape::Cross => (flare, (amount * p.get(P::BassGlow) * self.bass_env * self.bass_env).min(1.0)),
            Shape::Circle => (flare, 0.0),
        };
        self.analysis_ms += (t0.elapsed().as_secs_f32() * 1000.0 - self.analysis_ms) * 0.05;
        (gain, glow)
    }

    /// How much treble is showing, 0 to 1: the stars twinkle with it.
    fn treble(&self) -> f32 {
        let top = &self.shown[0][2 * N_BINS / 3..];
        (3.0 * top.iter().sum::<f32>() / top.len() as f32).min(1.0)
    }

    /// How far out and how strong the bass pulse is this frame.
    fn pulse(&self) -> (f32, f32) {
        let p = &self.params;
        let amount = if p.bass_boost { p.get(P::BassAmount) } else { 0.0 };
        let radius = 0.05 + (0.45 * p.get(P::BassGlow) + 1.2 * (amount - 1.0).max(0.0)) * self.bass_env;
        (amount * self.bass_env, radius)
    }

    /// Height of the 3D surface at a ground point, worked out the same way the
    /// shader does (leaving out stereo, which can only lower it). The flight
    /// uses it to stay clear of the surface.
    fn surface_height(&self, x: f32, y: f32, gain: f32, glow: f32) -> f32 {
        let p = &self.params;
        let aspect = self.aspect;
        let low = p.get(P::FreqLow) / analysis::OCTAVES as f32;
        let high = (p.get(P::FreqHigh) / analysis::OCTAVES as f32).max(low + 0.02).min(1.0);
        let level = |row: &[f32], f: f32| {
            let pos = (low + f.clamp(0.0, 1.0) * (high - low)) * N_BINS as f32 - 0.5;
            let i = pos.floor();
            let t = pos - i;
            let t = t * t * (3.0 - 2.0 * t);
            let at = |k: f32| row[(k.max(0.0) as usize).min(N_BINS - 1)];
            at(i) + (at(i + 1.0) - at(i)) * t
        };
        let blend = p.get(P::Combine);
        let combine = |a: f32, b: f32| {
            let either = a.max(b) * (SOFT_FLOOR + (1.0 - SOFT_FLOOR) * a.min(b));
            a * b + (either - a * b) * blend
        };
        let corner = 0.5 * aspect.hypot(1.0);
        let r = x.hypot(y) / corner;
        let v = if p.shape == Shape::Circle {
            if r > 1.03 {
                0.0
            } else {
                let a = level(&self.shown[0], if p.flip_x { 1.0 - r } else { r });
                let mut v = combine(a, a);
                let angular = p.get(P::Angular);
                if angular > 0.0 {
                    let turn = x.atan2(y) / std::f32::consts::TAU + 0.5;
                    let g = 1.0 - ((turn * 2.0).fract() * 2.0 - 1.0).abs();
                    let b = level(&self.shown[0], if p.flip_y { 1.0 - g } else { g });
                    v += (combine(a, b) - v) * angular;
                }
                v * gain
            }
        } else {
            let fold = |u: f32| 1.0 - (1.0 - u.rem_euclid(2.0)).abs();
            let mirror = |u: f32| 1.0 - (2.0 * u - 1.0).abs();
            let (mut fx, mut fy) = (fold(x / aspect + 0.5), fold(y + 0.5));
            if matches!(p.mirror, Mirror::Quadrants | Mirror::LeftRight) {
                fx = mirror(fx);
            }
            if matches!(p.mirror, Mirror::Quadrants | Mirror::TopBottom) {
                fy = mirror(fy);
            }
            let a = level(&self.shown[0], if p.flip_x { fx } else { 1.0 - fx });
            let b = level(&self.shown[1], if p.flip_y { fy } else { 1.0 - fy });
            let v = combine(a, b);
            v * gain + glow * a.max(b) * (1.0 - v)
        };
        let relief = 0.35 * p.get(P::ReliefHeight);
        let (strength, radius) = self.pulse();
        let dome = if p.bass_style == BassStyle::Pulse {
            0.8 * (strength * (-(r / radius.max(1e-3)).powi(2)).exp()).min(1.5)
        } else {
            0.0
        };
        relief * ((v * p.get(P::Brightness)).clamp(0.0, 1.0).powf(p.get(P::Contrast)) + dome)
    }

    /// Where the 3D camera is and what it looks at. Normally it looks at the
    /// centre from a tilt and an orbit angle. In flight it wanders on slow,
    /// overlapping curves that never repeat exactly: at depth 0 above and
    /// around the picture, towards depth 1 down among the peaks and across the
    /// middle, held clear of the surface as that moves with the music.
    fn update_camera(&mut self, dt: f32, gain: f32, glow: f32) {
        let p = &self.params;
        let height = 0.35 * p.get(P::ReliefHeight);
        if p.get(P::Flight) <= 0.0 {
            let (tilt, orbit, distance) = (p.get(P::Tilt).to_radians(), self.orbit_angle, 1.3f32);
            self.camera = (
                [distance * tilt.sin() * orbit.sin(), -distance * tilt.sin() * orbit.cos(), distance * tilt.cos()],
                [0.0; 3],
            );
            self.clearance = 0.0;
            return;
        }

        let depth = p.get(P::FlightDepth);
        let path = |t: f32| {
            let angle = 0.23 * t + 0.9 * (0.11 * t).sin();
            let around = 0.95 + 0.45 * (0.17 * t + 1.0).sin();
            let through = 0.80 * (0.21 * t + 0.5).sin();        // swings across the middle
            let radius = around + (through - around) * depth;
            [radius * angle.sin(), -radius * angle.cos()]
        };
        let t = self.flight_time;
        let here = path(t);
        let next = path(t + 0.6);
        let (dx, dy) = (next[0] - here[0], next[1] - here[1]);
        let step = dx.hypot(dy).max(1e-4);
        let heading = [dx / step, dy / step];

        // The highest ground under the camera and a short way ahead of it.
        // The camera rises to clear it quickly and settles back slowly, so it
        // skims over the music instead of jolting on every beat.
        let ground = (0..5)
            .map(|k| {
                let d = 0.06 * k as f32;
                self.surface_height(here[0] + heading[0] * d, here[1] + heading[1] * d, gain, glow)
            })
            .fold(0.0, f32::max);
        let rate = if ground > self.clearance { 0.12 } else { 0.9 };
        self.clearance += (ground - self.clearance) * (1.0 - (-dt / rate).exp());

        let high = height + 0.16 + 0.30 * (0.5 + 0.5 * (0.13 * t + 2.0).sin());
        let low = self.clearance + 0.05 + 0.07 * (0.5 + 0.5 * (0.31 * t).sin());
        let lift = (high + (low - high) * depth).max(self.clearance + 0.04);

        let look = p.get(P::LookAhead);
        let centre = [0.25 * (0.19 * t).sin(), 0.25 * (0.14 * t).cos(), 0.4 * height];
        let ahead = [here[0] + heading[0] * 0.7, here[1] + heading[1] * 0.7, lift - 0.10];
        self.camera = (
            [here[0], here[1], lift],
            std::array::from_fn(|i| centre[i] + (ahead[i] - centre[i]) * look),
        );
    }

    fn uniforms(&self, gain: f32, glow: f32, aspect: f32) -> render::Uniforms {
        let p = &self.params;
        let mut low = p.get(P::FreqLow) / analysis::OCTAVES as f32;
        let mut high = p.get(P::FreqHigh) / analysis::OCTAVES as f32;
        if high < low + 0.02 {
            (low, high) = (low.min(0.98), (low + 0.02).min(1.0));
        }
        let look = &self.look;
        let count = look.stops.len().min(params::MAX_STOPS);
        let mut stops = [[0.0, 0.0, 0.0, 1.0]; 9];
        for i in 0..count {
            let c = look.stops[if p.reverse_palette { count - 1 - i } else { i }];
            stops[i + 1] = [c[0], c[1], c[2], 1.0];
        }
        let accent = p.bass_colour.unwrap_or(look.accent);

        // 3D camera. Normally it looks at the centre from a tilt and an orbit
        // angle. In flight it wanders on slow, overlapping curves that never
        // repeat exactly, staying above the surface and looking near the centre.
        let height = 0.35 * p.get(P::ReliefHeight);
        let (eye, target) = self.camera;
        let surround = p.surround_colour.unwrap_or(look.surround);
        let amount = if p.bass_boost { p.get(P::BassAmount) } else { 0.0 };
        render::Uniforms {
            layout: [p.mirror as u8 as f32, p.flip_x as u8 as f32, p.flip_y as u8 as f32, p.get(P::Combine)],
            tone: [SOFT_FLOOR, gain, glow, p.get(P::Contrast)],
            view: [p.get(P::Brightness), low, high, N_BINS as f32],
            circle: [
                (p.shape == Shape::Circle) as u8 as f32,
                amount * self.bass_env,
                // The pulse swells with the hit; "Bass glow" sets how far it reaches.
                // Amounts above 1 push it out past the corners so it floods the frame.
                0.05 + (0.45 * p.get(P::BassGlow) + 1.2 * (amount - 1.0).max(0.0)) * self.bass_env,
                aspect,
            ],
            accent: [accent[0], accent[1], accent[2], p.get(P::BassGlow)],
            extra: [count as f32, p.get(P::Banding), p.get(P::Angular), (p.bass_style == BassStyle::Underlay) as u8 as f32],
            stereo: [p.stereo as u8 as f32, p.get(P::StereoEmphasis), p.get(P::SurroundAmount), p.material as u8 as f32],
            surround: [surround[0], surround[1], surround[2], 1.0],
            relief: [
                (p.get(P::Tilt) > 0.05 || p.get(P::Flight) > 0.0) as u8 as f32,
                height,
                p.get(P::Storm),
                self.started.elapsed().as_secs_f32() % 3600.0,
            ],
            cam_eye: [eye[0], eye[1], eye[2], (3.0 * (self.star_rate - 0.3)).clamp(0.0, 4.0)],
            cam_target: [target[0], target[1], target[2], 0.0],
            sim: [self.frame_dt, (p.get(P::Storm) * render::MAX_DROPS as f32).floor(), p.get(P::SurfaceAmount), self.star_travel],
            hdr: [
                self.hdr_active as u8 as f32,
                p.get(P::HdrBase) / 80.0,
                p.get(P::HdrPeak).max(p.get(P::HdrBase)) / 80.0,
                (self.hdr_active && self.hdr_pattern) as u8 as f32,
            ],
            post: [p.get(P::Bloom), if p.backdrop == Backdrop::Cover { 2.0 } else { 0.0 }, p.get(P::BackdropAmount), self.treble()],
            star_turn: [self.star_heading[0], self.star_heading[1], self.star_slide[0], self.star_slide[1]],
            stars: [if p.stars { p.get(P::StarBrightness).max(1e-4) } else { 0.0 }, p.get(P::StarDensity), p.get(P::StarVariety), 0.0],
            stops,
        }
    }

    /// Ask what other apps are playing, if anything here uses it: the lyrics,
    /// colours from the cover, the cover as a background, or the track card.
    fn update_playing(&mut self) {
        let wanted = (self.lyrics && self.lyrics_file.is_none())
            || self.params.palette_from_cover
            || self.params.backdrop == Backdrop::Cover
            || self.track_card
            || self.card_fly_asked
            // One asked for by hand is still on its way.
            || (self.card.is_some() && self.card_flying && self.card_near < 1.0);
        self.playing = if wanted { self.now_playing.get_or_insert_with(nowplaying::NowPlaying::start).now() } else { None };
    }

    /// The chosen palette's colours, or with palette drift on, a blend on the
    /// way from one palette to the next of its kind.
    fn drifting_look(&mut self, dt: f32) -> cover::Look {
        let seconds = self.params.get(P::PaletteDrift);
        if seconds < 0.5 {
            self.palette_drift = 0.0;
            return palette_look(&self.params);
        }
        self.palette_drift += dt / seconds;
        // Smooth palettes take turns with the smooth, abrupt ones with the abrupt.
        let chosen = self.params.palette();
        let abrupt = |i: usize| PALETTES[i].stops.len() > 4;
        let turns: Vec<usize> = (0..PALETTES.len()).map(|k| (chosen + k) % PALETTES.len()).filter(|i| abrupt(*i) == abrupt(chosen)).collect();
        let at = self.palette_drift.rem_euclid(turns.len() as f32);
        let (from, to) = (turns[at as usize % turns.len()], turns[(at as usize + 1) % turns.len()]);
        // Each palette is held for a while, then blends into the next.
        let t = ((at.fract() - 0.35) / 0.65).clamp(0.0, 1.0);
        let t = t * t * (3.0 - 2.0 * t);
        let mix = |a: [f32; 3], b: [f32; 3]| std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t);
        // Palettes have four or eight colours; as eight they can be blended one for one.
        let eight = |i: usize| -> Vec<[f32; 3]> {
            let stops = PALETTES[i].stops;
            (1..=8)
                .map(|k| {
                    let s = k as f32 / 8.0 * stops.len() as f32;
                    let upper = (s.ceil() as usize).clamp(1, stops.len());
                    let below = if upper == 1 { [0.0; 3] } else { stops[upper - 2] };
                    let part = s - (upper - 1) as f32;
                    std::array::from_fn(|c| below[c] + (stops[upper - 1][c] - below[c]) * part)
                })
                .collect()
        };
        cover::Look {
            stops: eight(from).into_iter().zip(eight(to)).map(|(a, b)| mix(a, b)).collect(),
            accent: mix(PALETTES[from].accent, PALETTES[to].accent),
            surround: mix(params::SURROUND_COLOURS[from], params::SURROUND_COLOURS[to]),
        }
    }

    /// Move the colours in use towards the ones wanted: the chosen palette's,
    /// or the playing track's cover's. A new track's colours fade in.
    fn update_look(&mut self, dt: f32) {
        let cover = self.playing.as_ref().and_then(|now| now.cover.clone()).filter(|_| self.params.palette_from_cover);
        let wanted = match &cover {
            Some(cover) => {
                let address = std::sync::Arc::as_ptr(cover) as usize;
                if self.cover_look.as_ref().is_none_or(|(of, _)| *of != address) {
                    self.cover_look = Some((address, cover::look(cover)));
                }
                self.cover_look.as_ref().map(|(_, look)| look.clone()).unwrap_or_else(|| palette_look(&self.params))
            }
            None => self.drifting_look(dt),
        };
        if cover.is_none() || wanted.stops.len() != self.look.stops.len() {
            // Changing palette by hand is immediate.
            self.look = wanted;
            return;
        }
        let k = 1.0 - (-dt / 0.5).exp();
        let ease = |from: &mut [f32; 3], to: &[f32; 3]| (0..3).for_each(|i| from[i] += (to[i] - from[i]) * k);
        self.look.stops.iter_mut().zip(&wanted.stops).for_each(|(from, to)| ease(from, to));
        ease(&mut self.look.accent, &wanted.accent);
        ease(&mut self.look.surround, &wanted.surround);
    }

    /// The playing track's cover, the first time it is seen, for the renderer
    /// to take a copy of. A track with none gets a black one.
    fn new_cover(&mut self) -> Option<std::sync::Arc<nowplaying::Cover>> {
        let cover = self.playing.as_ref().and_then(|now| now.cover.clone());
        let address = cover.as_ref().map_or(0, |c| std::sync::Arc::as_ptr(c) as usize);
        if address == self.cover_sent {
            return None;
        }
        self.cover_sent = address;
        let blank = || nowplaying::Cover { rgba: vec![0; (4 * nowplaying::COVER_SIZE * nowplaying::COVER_SIZE) as usize] };
        Some(cover.unwrap_or_else(|| std::sync::Arc::new(blank())))
    }

    /// The track as the lyrics search wants it.
    fn lyrics_query(track: &nowplaying::Track) -> lyrics::Query {
        lyrics::Query {
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: track.album.clone(),
            duration: track.duration,
        }
    }

    /// One line for the panel: what is playing and whether it has lyrics.
    #[cfg(windows)]
    fn lyrics_status(&self) -> String {
        if self.lyrics_file.is_some() {
            return "Showing the lyrics file given on the command line.".into();
        }
        let Some(track) = &self.lyrics_track else {
            return "Nothing is playing in an app that reports to Windows' media controls.".into();
        };
        // "Spotify.exe" or "WiMPMusic.27241E05630EA_kn85bz84x7te4!TIDAL".
        let app = track.app.rsplit('!').next().unwrap_or(&track.app).trim_end_matches(".exe");
        let state = match self.lookup.state(&Self::lyrics_query(track)) {
            _ if track.title.is_empty() => "no title reported".to_string(),
            lyrics::State::Looking => "looking for lyrics…".to_string(),
            lyrics::State::Synced(lines) => format!("{} lines of lyrics", lines.len()),
            lyrics::State::Missing(why) => why,
        };
        format!("{app}: {} by {}: {state}", track.title, track.artist)
    }

    /// Work out which lyrics are wanted and how far through them the music
    /// is, then draw them in the chosen place.
    fn draw_lyrics(&mut self, painter: &egui::Painter, picture: egui::Rect) {
        if !self.lyrics {
            return;
        }
        let (lines, position, app) = match &self.lyrics_file {
            Some(lines) => (Some(lines.clone()), Some(self.started.elapsed().as_secs_f64()), "file".to_string()),
            None => {
                let Some(now) = &self.playing else {
                    self.lyrics_track = None;
                    return;
                };
                let query = Self::lyrics_query(&now.track);
                if !query.title.is_empty() {
                    self.lookup.want(&query);
                }
                let lines = match self.lookup.state(&query) {
                    lyrics::State::Synced(lines) => Some(lines),
                    _ => None,
                };
                let (app, position) = (now.track.app.clone(), now.position);
                if self.lyrics_track.as_ref() != Some(&now.track) {
                    self.lyrics_track = Some(now.track.clone());
                }
                (lines, position, app)
            }
        };

        // Each music app has its own sync offset; the slider shows the current app's.
        let offset = self.params.target(P::LyricsOffset);
        if app != self.lyrics_app {
            let first = self.lyrics_app.is_empty();
            let remembered = self.lyrics_offsets.get(&app).copied().unwrap_or(if first { offset } else { 0.0 });
            self.params.set(P::LyricsOffset, remembered);
            self.controller.release(&self.bindings, P::LyricsOffset);
            self.lyrics_app = app;
        } else if self.lyrics_offsets.get(&app).copied().unwrap_or(0.0) != offset {
            self.lyrics_offsets.insert(app, offset);
        }

        let (Some(lines), Some(position)) = (lines, position) else { return };
        let now = position - self.params.get(P::LyricsOffset) as f64 / 1000.0;
        // Whole half points only, so a size is drawn from the same glyphs every frame.
        let size = ((self.params.get(P::LyricsSize) / 100.0 * picture.height()) * 2.0).round().max(16.0) / 2.0;
        match self.params.lyrics_place {
            LyricsPlace::Circle => self.draw_lyrics_arc(painter, picture, &lines, now, size),
            LyricsPlace::Crawl => self.draw_lyrics_crawl(painter, picture, &lines, now, size),
            place => self.draw_lyrics_block(painter, picture, &lines, now, size, place),
        }
    }

    /// Two colours for the lyrics from the palette in use, so the words
    /// belong to the picture: one for words waiting or already sung, and a
    /// brighter one for the word being sung.
    fn lyric_colours(&self) -> ([f32; 3], [f32; 3]) {
        let mut stops = self.look.stops.clone();
        let light = |c: &[f32; 3]| 0.2 * c[0] + 0.7 * c[1] + 0.1 * c[2];
        stops.sort_by(|a, b| light(b).total_cmp(&light(a)));
        // Lifted towards white far enough to read over the picture.
        let lift = |c: [f32; 3], by: f32| c.map(|v| v + (1.0 - v) * by);
        (lift(stops[1.min(stops.len() - 1)], 0.35), lift(stops[0], 0.6))
    }

    /// Draw laid-out lyrics with every letter moved to where `place` puts it
    /// (given the middle of the letter and the corner in question, both as laid out) and faded by `opacity`. With
    /// `words` (lyrics that time every word), the word being sung comes forward: it swells, lifts, takes
    /// the bright colour and glows, then settles back as the next one starts.
    #[allow(clippy::too_many_arguments)]
    fn lyric_text(
        &self,
        painter: &egui::Painter,
        galley: &egui::Galley,
        words: &[LyricWord],
        now: f64,
        size: f32,
        dim: f32,
        tint: Option<[f32; 3]>,
        place: &dyn Fn(egui::Pos2, egui::Pos2) -> Option<egui::Pos2>,
        opacity: &dyn Fn(egui::Pos2) -> f32,
    ) {
        let p = &self.params;
        // A set colour (and a word being sung a little paler), or the palette's.
        let (mid, hot) = match tint {
            Some(colour) => (colour, colour.map(|v| v + (1.0 - v) * 0.4)),
            None => self.lyric_colours(),
        };
        // How much the words that are not being sung show.
        let strength = p.get(P::LyricsStrength);
        let quiet = if tint.is_some() { (2.2 * strength).min(1.0) } else { 0.80 * strength };
        // At nothing, nothing shows at all, a word being sung included.
        let shown = (strength / 0.1).min(1.0);
        // Brightness follows the bass. In HDR the words stay at the panel's
        // steady white, well under the picture's peaks.
        let bass = if p.bass_boost { self.bass_env } else { 0.0 };
        let level = if self.hdr_active || !p.bass_boost { 1.0 } else { 0.7 + 0.3 * bass };
        let smooth = |x: f64| {
            let x = x.clamp(0.0, 1.0) as f32;
            x * x * (3.0 - 2.0 * x)
        };
        let colour = |c: [f32; 3], alpha: f32| {
            let byte = |v: f32| (255.0 * v * level).round().clamp(0.0, 255.0) as u8;
            egui::Color32::from_rgb(byte(c[0]), byte(c[1]), byte(c[2])).gamma_multiply(alpha.clamp(0.0, 1.0))
        };
        // A laid-out letter gives its place in the font picture in pixels; a mesh wants 0 to 1.
        let atlas = painter.ctx().fonts(|fonts| fonts.font_image_size());

        let (mut shadow, mut glow, mut letters) = (egui::Mesh::default(), egui::Mesh::default(), egui::Mesh::default());
        for (r, row) in galley.rows.iter().enumerate() {
            let source = &row.visuals.mesh;
            (shadow.texture_id, glow.texture_id, letters.texture_id) = (source.texture_id, source.texture_id, source.texture_id);
            let foot = row.pos.y + 0.9 * size;
            // Each corner of each letter: where it goes, its colour, how
            // dark its shadow is and how far forward its word has come.
            let mut corners: Vec<Option<(egui::Pos2, egui::Color32, f32, f32)>> = Vec::with_capacity(source.vertices.len());
            for letter in source.vertices.chunks(4) {
                let middle = letter.iter().fold(egui::Vec2::ZERO, |sum, v| sum + v.pos.to_vec2()) / letter.len() as f32;
                let middle = row.pos + middle;
                let word = words.iter().find(|w| w.row == r && middle.x >= w.left && middle.x <= w.right);
                // `forward` rises as the word starts and falls away once the
                // next has begun; `kick` is the extra push at its first moment.
                let (forward, kick, sung) = word.map_or((0.0, 0.0, 0.0), |w| {
                    let begun = smooth((now - w.start) / 0.07);
                    let over = if now < w.end { 1.0 } else { (-(now - w.end) / 0.12).exp() as f32 };
                    (begun * over, if now >= w.start { (-(now - w.start) / 0.22).exp() as f32 } else { 0.0 }, begun)
                });
                let seen = opacity(middle) * dim * shown;
                let rgb: [f32; 3] = std::array::from_fn(|i| mid[i] + (hot[i] - mid[i]) * forward);
                // In a line with word times, the words still to come hang back a little.
                let alpha = quiet * if word.is_some() { 0.75 + 0.25 * sung } else { 1.0 };
                let text = colour(rgb, (alpha + (1.0 - alpha) * forward) * seen);
                let swell = 1.0 + forward * (0.05 + 0.12 * kick + 0.04 * bass);
                for v in letter {
                    let mut at = row.pos + v.pos.to_vec2();
                    if let Some(w) = word {
                        let centre = 0.5 * (w.left + w.right);
                        // It grows taller freely but only a little wider, so it does not run into its neighbours.
                        let wider = swell.min(1.0 + 0.12 * size / (w.right - w.left).max(1.0));
                        at = egui::pos2(centre + (at.x - centre) * wider, foot + (at.y - foot) * swell - 0.06 * size * forward);
                    }
                    corners.push(if seen > 0.0 { place(middle, at).map(|at| (at, text, 0.75 * strength * seen, forward * seen)) } else { None });
                }
            }
            let d = (size * 0.045).max(1.0);
            let halo = size * 0.035 * (1.0 + bass);
            for triangle in source.indices.chunks_exact(3) {
                let [Some(a), Some(b), Some(c)] = [0, 1, 2].map(|k| corners[triangle[k] as usize]) else { continue };
                let uv = |k: usize| {
                    let uv = source.vertices[triangle[k] as usize].uv;
                    egui::pos2(uv.x / atlas[0] as f32, uv.y / atlas[1] as f32)
                };
                let add = |mesh: &mut egui::Mesh, offset: egui::Vec2, tint: &dyn Fn(&(egui::Pos2, egui::Color32, f32, f32)) -> egui::Color32| {
                    let first = mesh.vertices.len() as u32;
                    for (k, corner) in [a, b, c].iter().enumerate() {
                        mesh.vertices.push(egui::epaint::Vertex { pos: corner.0 + offset, uv: uv(k), color: tint(corner) });
                    }
                    mesh.indices.extend([first, first + 1, first + 2]);
                };
                add(&mut letters, egui::Vec2::ZERO, &|corner| corner.1);
                // A dark copy behind, on each diagonal, keeps them readable over a bright picture.
                for offset in [egui::vec2(d, d), egui::vec2(-d, d), egui::vec2(d, -d), egui::vec2(-d, -d)] {
                    add(&mut shadow, offset, &|corner| egui::Color32::from_black_alpha((255.0 * corner.2) as u8));
                }
                // The word being sung has a halo in its own colour, which swells with the bass.
                if a.3 > 0.03 {
                    // Two rings of faint copies, close enough together to read as a soft edge.
                    for k in 0..16 {
                        let (sin, cos) = (k as f32 * std::f32::consts::TAU / 8.0 + 0.4 * (k / 8) as f32).sin_cos();
                        let reach = halo * if k < 8 { 1.0 } else { 0.5 };
                        add(&mut glow, egui::vec2(cos, sin) * reach, &|corner| colour(hot, 0.06 * corner.3));
                    }
                }
            }
        }
        painter.add(shadow);
        painter.add(glow);
        painter.add(letters);
    }

    /// Lyrics as lines across the picture, at the top, middle or bottom: the
    /// line being sung, the one before it fading out above and the one to
    /// come small below.
    fn draw_lyrics_block(&self, painter: &egui::Painter, picture: egui::Rect, lines: &[lyrics::Line], now: f64, size: f32, place: LyricsPlace) {
        /// A line is brought in this long before it is sung, so it can be read in time.
        const LEAD_S: f64 = 0.15;
        /// How long the line before takes to fade away.
        const LEAVE_S: f64 = 0.5;
        let t = now + LEAD_S;
        let started = lines.partition_point(|line| line.start <= t);
        let current = started.checked_sub(1);
        let previous = started.checked_sub(2).map(|i| &lines[i]);
        let next = lines.get(started);
        let smooth = |x: f64| {
            let x = x.clamp(0.0, 1.0) as f32;
            x * x * (3.0 - 2.0 * x)
        };
        // Draw a line centred, with `y` at its top (anchor 0), middle (0.5)
        // or bottom (1). Returns its height.
        let text = |line: &lyrics::Line, timed: Option<f64>, size: f32, y: f32, anchor: f32, opacity: f32| -> f32 {
            let mut job = egui::text::LayoutJob::default();
            job.wrap.max_width = picture.width() * 0.86;
            job.halign = egui::Align::Center;
            job.append(&line.text, 0.0, egui::TextFormat { font_id: egui::FontId::proportional(size), color: egui::Color32::WHITE, ..Default::default() });
            let galley = painter.layout_job(job);
            let height = galley.size().y;
            let origin = egui::vec2(picture.center().x, y - anchor * height);
            let words = timed.map(|next_start| lyric_words(&galley, line, next_start)).unwrap_or_default();
            let dim = if timed.is_some() { 1.0 } else { 0.7 };
            self.lyric_text(painter, &galley, &words, now, size, dim, None, &|_, at| Some(at + origin), &|_| opacity);
            height
        };

        let middle = match place {
            LyricsPlace::Top => picture.top() + (0.16 * picture.height()).max(2.6 * size),
            LyricsPlace::Bottom => picture.bottom() - (0.16 * picture.height()).max(2.6 * size),
            _ => picture.center().y,
        };
        let age = current.map_or(f64::MAX, |i| t - lines[i].start);
        // The new line comes in over the second half of its lead, once the
        // line before has moved out of its way.
        let arrive = smooth(age / (0.5 * LEAD_S) - 1.0);
        let mut height = size * 1.2;
        if let Some(i) = current.filter(|i| !lines[*i].text.is_empty()) {
            // A line left standing through a long gap dims, so it is not taken for the one being sung.
            let held = 1.0 - 0.65 * smooth((age - 10.0) / 2.0);
            let next_start = lines.get(i + 1).map_or(f64::MAX, |line| line.start);
            height = text(&lines[i], Some(next_start), size, middle + (1.0 - arrive) * 0.3 * size, 0.5, arrive * held);
        }
        let gap = 0.35 * size;
        if let Some(line) = previous.filter(|line| !line.text.is_empty() && age < LEAVE_S) {
            // It moves up out of the way of the new line as it fades.
            let rise = smooth(age / (0.6 * LEAD_S));
            let bottom = middle + 0.6 * size + (-height / 2.0 - gap - 0.6 * size) * rise;
            text(line, None, size, bottom, 1.0, 1.0 - smooth(age / LEAVE_S));
        }
        if let Some(line) = next.filter(|line| self.params.lyrics_preview && !line.text.is_empty()) {
            let opacity = if current.is_some() { arrive } else { 1.0 };
            text(line, None, (size * 0.5 * 2.0).round() / 2.0, middle + height / 2.0 + gap, 0.0, opacity);
        }
    }

    /// Lyrics as the opening titles of a space film: yellow, laid back on a
    /// plane, coming in at the bottom of the picture and rolling away up it
    /// into the distance. The line being sung is at an easy distance to read.
    fn draw_lyrics_crawl(&self, painter: &egui::Painter, picture: egui::Rect, lines: &[lyrics::Line], now: f64, size: f32) {
        const YELLOW: [f32; 3] = [1.0, 0.86, 0.16];
        /// How many times further away than the bottom edge the line being sung is.
        const READING: f32 = 1.2;
        /// Lines fade away between these distances: from just past the one
        /// being sung, so the page thins steadily as it goes.
        const FADE: (f32, f32) = (1.45, 3.8);
        let started = lines.partition_point(|line| line.start <= now);
        let Some(current) = started.checked_sub(1) else { return };

        // Text at the bottom edge is drawn this size; it shrinks with distance.
        let near_size = (1.7 * size * 2.0).round() / 2.0;
        let font = egui::FontId::new(near_size, egui::FontFamily::Name(CRAWL_FONT.into()));
        let block = 0.8 * picture.width();
        let justify = self.params.lyrics_justify;
        let layout = |line: &lyrics::Line| {
            let set = |wrap: f32| {
                let mut job = egui::text::LayoutJob::default();
                job.wrap.max_width = wrap;
                job.halign = egui::Align::Center;
                job.append(&line.text, 0.0, egui::TextFormat { font_id: font.clone(), color: egui::Color32::WHITE, ..Default::default() });
                painter.layout_job(job)
            };
            if !justify {
                let galley = set(block);
                let fit = Fit::of(&galley, None, near_size);
                return (galley, fit);
            }
            // Justified, a line is broken into rows of about equal length,
            // few enough that none has to be shrunk far to fit the block.
            let whole = set(f32::INFINITY);
            let rows = (0.8 * whole.size().x / block).ceil().max(1.0);
            let galley = if rows > 1.0 { set(1.08 * whole.size().x / rows) } else { whole };
            let fit = Fit::of(&galley, Some(block), near_size);
            (galley, fit)
        };
        let gap = 0.45 * near_size;
        // Everything rolls away from a point above the top of the picture.
        let horizon = picture.top() - 0.08 * picture.height();
        let fall = picture.bottom() - horizon;
        // Distance up the page that doubles how far away a line is.
        let stretch = 0.75 * picture.height();
        let smooth = |x: f32| {
            let x = x.clamp(0.0, 1.0);
            x * x * (3.0 - 2.0 * x)
        };

        // How far the page has rolled past the top of the current line: by one
        // line and its gap in the time until the next line starts (or a few
        // seconds, where the next is a long way off).
        let (galley, fit) = layout(&lines[current]);
        let next_start = lines.get(current + 1).map_or(f64::MAX, |line| line.start);
        let until_next = (next_start - lines[current].start).clamp(0.3, 6.0);
        let rolled = (fit.height + gap) * ((now - lines[current].start) / until_next).clamp(0.0, 1.0) as f32;

        // Draw a line whose top is `down` the page from the top of the current one.
        let draw = |galley: &egui::Galley, fit: &Fit, words: &[LyricWord], down: f32, dim: f32| {
            let distance = |y: f32| READING + (rolled - (down + y)) / stretch;
            self.lyric_text(
                painter,
                galley,
                words,
                now,
                near_size,
                dim,
                Some(YELLOW),
                &|middle, at| {
                    let on_page = fit.place(middle, at);
                    let away = distance(on_page.y);
                    // Nearer than this it is off the bottom of the picture anyway.
                    (away > 0.6).then(|| egui::pos2(picture.center().x + on_page.x / away, horizon + fall / away))
                },
                &|middle| 1.0 - smooth((distance(fit.place(middle, middle).y) - FADE.0) / (FADE.1 - FADE.0)),
            );
        };

        // The line being sung stands out the way a word being sung does: it
        // lifts, turns pale and bright, and glows. Lyrics that time every
        // word pick out the word instead.
        let mut words = lyric_words(&galley, &lines[current], next_start);
        if words.is_empty() {
            let until = lines[current].start + until_next;
            words = galley
                .rows
                .iter()
                .enumerate()
                .map(|(r, row)| LyricWord { start: lines[current].start, end: until, row: r, left: row.pos.x - 1.0, right: row.pos.x + row.size.x + 1.0 })
                .collect();
        }
        draw(&galley, &fit, &words, 0.0, 1.0);
        // The lines already sung, further up the page and further away.
        let mut down = 0.0;
        for earlier in lines[..current].iter().rev() {
            let (galley, fit) = layout(earlier);
            down -= fit.height + gap;
            if READING + (rolled - down - fit.height) / stretch > FADE.1 {
                break;
            }
            draw(&galley, &fit, &[], down, 0.5);
        }
        // The lines to come, lower down and nearer, until they are off the bottom.
        let mut down = fit.height + gap;
        for later in &lines[current + 1..] {
            if READING + (rolled - down) / stretch < 0.9 {
                break;
            }
            let (galley, fit) = layout(later);
            draw(&galley, &fit, &[], down, 0.5);
            down += fit.height + gap;
        }
    }

    /// Lyrics run round the far side of a circle about the middle of the
    /// picture and scroll past its top as they are sung, each line following
    /// the last. They lie on the picture, so in 3D they tilt and turn with it.
    fn draw_lyrics_arc(&self, painter: &egui::Painter, picture: egui::Rect, lines: &[lyrics::Line], now: f64, size: f32) {
        /// Letters fade out between these angles either side of the top.
        const FADE: (f32, f32) = (55.0, 80.0);
        let font = egui::FontId::proportional(size);
        // To the foot of the letters, kept clear of the top of the picture.
        let shorter = picture.width().min(picture.height());
        let radius = (0.40 * shorter).min(0.5 * shorter - 1.35 * size).max(2.0 * size);
        let gap = 1.6 * size;
        let reach = FADE.1.to_radians() * radius;
        let centre = picture.center();
        let smooth = |x: f32| {
            let x = x.clamp(0.0, 1.0);
            x * x * (3.0 - 2.0 * x)
        };

        // Where a point on the picture is on screen: `angle` clockwise from the
        // top of the circle and `r` points out from its centre. Flat, that is
        // plain geometry. In 3D the point is on the ground the surface rises from
        // (see `relief_view` in the shader), and "the top" is the far side
        // from wherever the camera is, so the words always face it.
        let p = &self.params;
        let relief = p.get(P::Tilt) > 0.05 || p.get(P::Flight) > 0.0;
        let (eye, target) = self.camera;
        let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
        let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let cross = |a: [f32; 3], b: [f32; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
        let unit = |a: [f32; 3]| {
            let length = dot(a, a).sqrt().max(1e-6);
            [a[0] / length, a[1] / length, a[2] / length]
        };
        let forward = unit(sub(target, eye));
        let right = unit(cross(forward, [0.0, 0.0, 1.0]));
        let up = cross(right, forward);
        let away = forward[0].hypot(forward[1]);
        let far = if away > 1e-3 { [forward[0] / away, forward[1] / away] } else { [0.0, 1.0] };
        let spread = 0.5 / 1.3;
        let aspect = picture.width() / picture.height().max(1.0);
        let on_picture = |angle: f32, r: f32| -> Option<egui::Pos2> {
            let (sin, cos) = angle.sin_cos();
            if !relief {
                return Some(centre + egui::vec2(sin, -cos) * r);
            }
            let r = r / picture.height();
            let from_eye = sub([r * (far[0] * cos + far[1] * sin), r * (far[1] * cos - far[0] * sin), 0.0], eye);
            let depth = dot(from_eye, forward);
            (depth > 0.05).then(|| {
                let x = dot(from_eye, right) / (depth * aspect * spread);
                let y = dot(from_eye, up) / (depth * spread);
                centre + egui::vec2(x * picture.width(), -y * picture.height()) * 0.5
            })
        };

        // A line is laid out straight, then every corner of every letter is
        // moved to its place on the picture.
        let layout = |line: &lyrics::Line| painter.layout_no_wrap(line.text.clone(), font.clone(), egui::Color32::WHITE);
        // Draw a line with its left end `x` points clockwise along the arc from the top.
        let draw = |galley: &egui::Galley, words: &[LyricWord], x: f32| {
            let tall = galley.size().y;
            let dim = if words.is_empty() { 0.7 } else { 1.0 };
            self.lyric_text(
                painter,
                galley,
                words,
                now,
                size,
                dim,
                None,
                &|_, at| on_picture((x + at.x) / radius, radius + tall - at.y),
                &|at| 1.0 - smooth((((x + at.x) / radius).abs().to_degrees() - FADE.0) / (FADE.1 - FADE.0)),
            );
        };

        let started = lines.partition_point(|line| line.start <= now);
        let Some(current) = started.checked_sub(1) else {
            // Before the first line: it waits with its first letter at the top.
            if let Some(first) = lines.first() {
                draw(&layout(first), &[], 0.0);
            }
            return;
        };
        let line = &lines[current];
        let galley = layout(line);
        let whole = galley.size().x;
        let next_start = lines.get(current + 1).map_or(f64::MAX, |next| next.start);
        let words = lyric_words(&galley, line, next_start);

        // How far the line has moved past the top. It crosses in about the
        // time it takes to sing, then the next line waits there for its turn.
        // With word timings, each word reaches the top as it is sung.
        let mut marks = vec![(line.start, 0.0)];
        marks.extend(words.iter().map(|w| (w.start, w.left)));
        let end = words.last().map_or(line.start + (0.15 * line.text.chars().count() as f64).max(2.0), |w| w.end);
        marks.push((end.min(next_start), whole + gap));
        let mut moved = whole + gap;
        for pair in marks.windows(2) {
            let ((t0, x0), (t1, x1)) = (pair[0], pair[1]);
            if now < t1 && t1 > t0 {
                moved = x0 + (x1 - x0) * ((now - t0) / (t1 - t0)).clamp(0.0, 1.0) as f32;
                break;
            }
        }

        draw(&galley, &words, -moved);
        // The lines already sung, going away anticlockwise.
        let mut right_end = -moved - gap;
        for earlier in lines[..current].iter().rev() {
            if right_end < -reach {
                break;
            }
            let galley = layout(earlier);
            right_end -= galley.size().x;
            draw(&galley, &[], right_end);
            right_end -= gap;
        }
        // The lines to come, arriving clockwise.
        let mut left_end = -moved + whole + gap;
        for later in &lines[current + 1..] {
            if left_end > reach {
                break;
            }
            let galley = layout(later);
            draw(&galley, &[], left_end);
            left_end += galley.size().x + gap;
        }
    }

    /// The title, artist and cover of a track for a few seconds as it starts,
    /// in the top-right corner of the picture.
    fn draw_track_card(&mut self, painter: &egui::Painter, picture: egui::Rect) {
        const SHOWN_S: f32 = 7.0;
        let Some(now) = &self.playing else { return };
        let same = |a: &nowplaying::Track, b: &nowplaying::Track| a.app == b.app && a.title == b.title && a.artist == b.artist;
        if self.card_fly_asked && !now.track.title.is_empty() {
            // Asked for by hand: fly it in now, whatever the settings say.
            (self.card, self.card_near, self.card_flying, self.card_fly_asked) = (Some((now.track.clone(), Instant::now())), 0.0, true, false);
        } else if !self.track_card {
            // Left alone only while one asked for by hand is still on its way.
            if !(self.card_flying && self.card_near < 1.0) {
                self.card = None;
                return;
            }
        } else if !now.track.title.is_empty() && self.card.as_ref().is_none_or(|(track, _)| !same(track, &now.track)) {
            (self.card, self.card_near, self.card_flying) = (Some((now.track.clone(), Instant::now())), 0.0, self.track_card_fly);
        }
        let Some((track, since)) = &self.card else { return };
        let age = since.elapsed().as_secs_f32();
        if if self.card_flying { self.card_near >= 1.0 } else { age > SHOWN_S } {
            return;
        }
        let seen = (age / 0.4).min(1.0) * ((SHOWN_S - age) / 1.2).min(1.0);

        // The cover arrives a moment after the title; it is added when it does.
        if let Some(cover) = &now.cover {
            let address = std::sync::Arc::as_ptr(cover) as usize;
            if self.card_cover.as_ref().is_none_or(|(of, _)| *of != address) {
                let side = nowplaying::COVER_SIZE as usize;
                let pixels = cover.rgba.chunks_exact(4).map(|p| egui::Color32::from_rgb(p[0], p[1], p[2])).collect();
                let image = egui::ColorImage::new([side, side], pixels);
                self.card_cover = Some((address, painter.ctx().load_texture("cover", image, egui::TextureOptions::LINEAR)));
            }
        } else {
            self.card_cover = None;
        }

        if self.card_flying {
            // It comes on with the stars: at their pace when they are flying,
            // but never so fast that it cannot be read, nor so slowly that it lingers.
            let pace = if self.star_travel > 0.0 { self.star_rate.clamp(0.1, 0.2) } else { 0.14 };
            self.card_near = (self.card_near + pace * self.frame_dt).min(1.0);
            let near = self.card_near;
            // Its share of the picture's height. It grows by the same factor
            // each moment, as anything approached at a steady speed does: a dot
            // for a while, readable for a second or two, then on top of you.
            let share = 1.15 * 0.003f32.powf(1.0 - near);
            let smooth = |x: f32| {
                let x = x.clamp(0.0, 1.0);
                x * x * (3.0 - 2.0 * x)
            };
            // Out of the dark, and thinning away as it comes to fill the picture.
            let seen = smooth(near / 0.12) * (1.0 - smooth((near - 0.78) / 0.22));
            // Laid out once at a fixed size, then drawn at whatever size it has reached.
            let wrap = 700.0;
            let centred = |text: &str, size: f32| {
                let mut job = egui::text::LayoutJob::default();
                job.wrap.max_width = wrap;
                job.halign = egui::Align::Center;
                job.append(text, 0.0, egui::TextFormat { font_id: egui::FontId::proportional(size), color: egui::Color32::WHITE, ..Default::default() });
                painter.layout_job(job)
            };
            let (title, artist) = (centred(&track.title, 56.0), centred(&track.artist, 36.0));
            let side = if self.card_cover.is_some() { 420.0 } else { 0.0 };
            let whole = side + 24.0 + title.size().y + 8.0 + artist.size().y;
            let scale = share * picture.height() / whole * 1.0;
            // Dead ahead: where the stars are streaming from.
            let ahead = picture.center() + egui::vec2(self.star_heading[0], -self.star_heading[1]) * picture.height();
            let top = ahead.y - 0.5 * whole * scale;
            if let Some((_, texture)) = &self.card_cover {
                let image = egui::Rect::from_center_size(egui::pos2(ahead.x, top + 0.5 * side * scale), egui::vec2(side, side) * scale);
                let all = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                painter.image(texture.id(), image, all, egui::Color32::WHITE.gamma_multiply(seen));
            }
            let atlas = painter.ctx().fonts(|fonts| fonts.font_image_size());
            let mut y = top + (side + 24.0) * scale;
            for (galley, grey) in [(title, 255u8), (artist, 200)] {
                for (offset, colour) in [
                    (egui::vec2(1.5, 1.5) * scale.max(0.3), egui::Color32::from_black_alpha((200.0 * seen) as u8)),
                    (egui::Vec2::ZERO, egui::Color32::from_gray(grey).gamma_multiply(seen)),
                ] {
                    let mut mesh = egui::Mesh::default();
                    for row in &galley.rows {
                        let source = &row.visuals.mesh;
                        mesh.texture_id = source.texture_id;
                        let first = mesh.vertices.len() as u32;
                        mesh.vertices.extend(source.vertices.iter().map(|v| egui::epaint::Vertex {
                            pos: egui::pos2(ahead.x, y) + (row.pos.to_vec2() + v.pos.to_vec2()) * scale + offset,
                            uv: egui::pos2(v.uv.x / atlas[0] as f32, v.uv.y / atlas[1] as f32),
                            color: colour,
                        }));
                        mesh.indices.extend(source.indices.iter().map(|i| first + i));
                    }
                    painter.add(mesh);
                }
                y += (galley.size().y + 8.0) * scale;
            }
            return;
        }

        let wrap = (0.3 * picture.width()).max(160.0);
        let line = |text: &str, size: f32, colour: egui::Color32| {
            painter.layout(text.to_string(), egui::FontId::proportional(size), colour.gamma_multiply(seen), wrap)
        };
        let lines = [
            line(&track.title, 20.0, egui::Color32::WHITE),
            line(&track.artist, 15.0, egui::Color32::from_gray(210)),
            line(&track.album, 12.0, egui::Color32::from_gray(150)),
        ];
        let text_width = lines.iter().map(|g| g.size().x).fold(0.0, f32::max);
        let text_height: f32 = lines.iter().map(|g| g.size().y + 2.0).sum();
        let side = if self.card_cover.is_some() { text_height.max(56.0) } else { 0.0 };
        let pad = 10.0;
        let size = egui::vec2(side + if side > 0.0 { pad } else { 0.0 } + text_width + 2.0 * pad, side.max(text_height) + 2.0 * pad);
        // It slides in from the edge as it appears.
        let corner = picture.right_top() + egui::vec2(-size.x - 14.0 + 20.0 * (1.0 - (age / 0.4).min(1.0)), 14.0);
        let card = egui::Rect::from_min_size(corner, size);
        painter.rect_filled(card, 8.0, egui::Color32::from_black_alpha((150.0 * seen) as u8));
        let mut at = card.min + egui::vec2(pad, pad);
        if let Some((_, texture)) = &self.card_cover {
            let image = egui::Rect::from_min_size(at, egui::vec2(side, side));
            let whole = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            painter.image(texture.id(), image, whole, egui::Color32::WHITE.gamma_multiply(seen));
            at.x += side + pad;
        }
        at.y += 0.5 * (size.y - 2.0 * pad - text_height);
        for galley in lines {
            let height = galley.size().y + 2.0;
            painter.galley(at, galley, egui::Color32::WHITE);
            at.y += height;
        }
    }

    /// FPS counter and graph in the top-left corner of the picture.
    fn draw_fps(&self, painter: &egui::Painter, picture: egui::Rect) {
        let origin = picture.left_top() + egui::vec2(10.0, 8.0);
        let mut graph_top = origin.y;
        if self.show_fps {
            let text = format!("{:.0} fps   {:.1} ms", 1000.0 / self.frame_ms.max(0.01), self.frame_ms);
            let font = egui::FontId::monospace(16.0);
            // A dark copy underneath keeps it readable over a bright picture.
            painter.text(origin + egui::vec2(1.0, 1.0), egui::Align2::LEFT_TOP, &text, font.clone(), egui::Color32::BLACK);
            painter.text(origin, egui::Align2::LEFT_TOP, &text, font, egui::Color32::WHITE);
            graph_top += 24.0;
        }
        if self.show_fps_graph && self.frame_history.len() > 1 {
            let area = egui::Rect::from_min_size(egui::pos2(origin.x, graph_top), egui::vec2(FPS_HISTORY as f32, 72.0));
            painter.rect_filled(area, 3.0, egui::Color32::from_black_alpha(170));
            let fps: Vec<f32> = self.frame_history.iter().map(|ms| 1000.0 / ms.max(0.1)).collect();
            let mean = fps.iter().sum::<f32>() / fps.len() as f32;
            // Leave headroom above the usual rate so a steady line is not glued to the top.
            let top = (mean * 1.25).max(30.0);
            let y = |v: f32| area.bottom() - (v / top).clamp(0.0, 1.0) * area.height();
            let points: Vec<egui::Pos2> = fps
                .iter()
                .enumerate()
                .map(|(i, v)| egui::pos2(area.right() - (fps.len() - 1 - i) as f32, y(*v)))
                .collect();
            painter.hline(area.x_range(), y(mean), egui::Stroke::new(1.0, egui::Color32::from_white_alpha(60)));
            painter.add(egui::Shape::line(points, egui::Stroke::new(1.5, egui::Color32::from_rgb(120, 255, 140))));
            let small = egui::FontId::monospace(10.0);
            let lowest = fps.iter().copied().fold(f32::MAX, f32::min);
            painter.text(area.left_top() + egui::vec2(4.0, 2.0), egui::Align2::LEFT_TOP, format!("{top:.0}"), small.clone(), egui::Color32::GRAY);
            painter.text(
                area.left_bottom() + egui::vec2(4.0, -2.0),
                egui::Align2::LEFT_BOTTOM,
                format!("last 2 s: mean {mean:.0}, lowest {lowest:.0}"),
                small,
                egui::Color32::LIGHT_GRAY,
            );
        }
    }

    /// A settings section heading that folds its contents away when clicked.
    /// Returns whether the section is open.
    fn section(&mut self, ui: &mut egui::Ui, name: &str) -> bool {
        let open = !self.collapsed.contains(name);
        let heading = egui::RichText::new(format!("{} {name}", if open { "-" } else { "+" })).strong();
        let response = ui.add(egui::Label::new(heading).sense(egui::Sense::click()));
        if response.clicked() {
            if open {
                self.collapsed.insert(name.to_string());
            } else {
                self.collapsed.remove(name);
            }
        }
        self.describe(&response, "Click to fold this section away or open it again. Folded sections are remembered between runs.");
        open
    }

    /// Attach a description to a control; it fades in beside the control
    /// after the pointer has rested there a moment.
    fn describe(&mut self, response: &egui::Response, text: &str) {
        if response.hovered() {
            self.hint.hovered = Some((response.id, text.to_string(), response.rect));
        }
    }

    fn show_hint(&mut self, ctx: &egui::Context) {
        if !self.show_hints {
            self.hint = Hint::default();
            return;
        }
        let hovered = self.hint.hovered.take();
        match (&hovered, &self.hint.current) {
            (Some((id, ..)), Some((current, ..))) if id == current => {}
            (Some((id, text, rect)), _) => self.hint.current = Some((*id, text.clone(), *rect, Instant::now())),
            (None, _) => {}
        }
        let Some((id, text, rect, since)) = self.hint.current.clone() else { return };
        let resting = hovered.as_ref().is_some_and(|(h, ..)| *h == id) && since.elapsed().as_secs_f32() > HINT_DELAY_S;
        let opacity = ctx.animate_bool_with_time(egui::Id::new("hint-opacity"), resting, HINT_FADE_S);
        if opacity <= 0.0 {
            if hovered.is_none() {
                self.hint.current = None;
            }
            return;
        }
        // To the left of the control, over the picture, so it never covers the panel.
        let width = 320.0;
        let x = (rect.left() - width - 28.0).max(8.0);
        egui::Area::new(egui::Id::new("hint"))
            .order(egui::Order::Tooltip)
            .fixed_pos(egui::pos2(x, rect.top() - 6.0))
            .interactable(false)
            .show(ctx, |ui| {
                ui.set_opacity(opacity);
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(width);
                    ui.label(text);
                });
            });
    }

    fn slider(&mut self, ui: &mut egui::Ui, id: P) {
        let d = def_of(id);
        let mut value = self.params.target(id);
        let mut slider = egui::Slider::new(&mut value, d.min..=d.max).logarithmic(d.log);
        if id == P::LyricsOffset {
            slider = slider.step_by(10.0);
        }
        slider = match id {
            P::Palette => slider.custom_formatter(|v, _| PALETTES[(v.round() as usize).min(PALETTES.len() - 1)].name.to_string()),
            P::FreqLow | P::FreqHigh => slider.custom_formatter(|v, _| format!("{:.0} Hz", F_MIN as f64 * 2f64.powf(v))),
            _ => slider.suffix(if d.unit.is_empty() { String::new() } else { format!(" {}", d.unit) }),
        };
        let bound = self.bindings.control_for(id);
        let label = match (self.learning == Some(id), bound) {
            (true, _) => format!("{}  [move a control…]", d.name),
            (false, Some(cc)) => format!("{}  [CC {cc}]", d.name),
            (false, None) => d.name.to_string(),
        };
        let heading = ui.label(label);
        let response = ui.add(slider);
        let mut help = d.help.to_string();
        if let Some(cc) = bound {
            help += &match cc {
                0..=7 => format!(" On the controller: fader {}.", cc + 1),
                16..=23 => format!(" On the controller: knob {}.", cc - 15),
                _ => format!(" On the controller: control {cc}."),
            };
        }
        help += " The small mark on the slider is its default; double-click the slider to go back to it. Right-click for MIDI learn.";

        // The slider's track is the left part of the widget; the value box follows it.
        let track = egui::Rect::from_min_size(response.rect.min, egui::vec2(ui.spacing().slider_width, response.rect.height()));
        let inset = track.height() / 2.5;       // the handle stops this far short of each end
        let x = track.left() + inset + d.to_norm(d.default) * (track.width() - 2.0 * inset);
        let mark = egui::Stroke::new(1.5, ui.visuals().strong_text_color().gamma_multiply(0.6));
        ui.painter().vline(x, track.top()..=track.top() + 4.0, mark);
        ui.painter().vline(x, track.bottom() - 4.0..=track.bottom(), mark);
        let on_track = response.interact_pointer_pos().or(response.hover_pos()).is_some_and(|p| track.contains(p));
        // A slider only listens for drags, so it never reports a double-click
        // itself; ask the pointer directly.
        let reset = on_track && ui.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary));
        self.describe(&heading, &help);
        self.describe(&response, &help);
        if self.options.demo_hint.as_deref() == Some(d.key) {
            self.hint.hovered = Some((response.id, help.clone(), response.rect));
        }
        if reset {
            self.params.set(id, d.default);
            self.controller.release(&self.bindings, id);
        } else if response.changed() {
            self.params.set(id, value);
            self.controller.release(&self.bindings, id);
        }
        response.context_menu(|ui| {
            if ui.button("MIDI learn").clicked() {
                self.learning = Some(id);
                ui.close();
            }
            if bound.is_some() && ui.button("Clear MIDI binding").clicked() {
                self.bindings.unbind(id);
                ui.close();
            }
            if ui.button("Reset to default").clicked() {
                self.params.set(id, d.default);
                self.controller.release(&self.bindings, id);
                ui.close();
            }
        });
    }

    fn panel(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading("AudioVis");
            #[cfg(not(target_os = "android"))]
            ui.small("Tab: hide panel   F11: fullscreen   F5: fly the title in   right-click picture: palette");
            #[cfg(target_os = "android")]
            ui.small("Tap the picture to hide or show this panel, swipe across it for the next palette, and press and hold it (or any slider) for more.");
            let label = if self.show_hints { "Descriptions on hover: on" } else { "Descriptions on hover: off" };
            ui.toggle_value(&mut self.show_hints, label);
            ui.horizontal(|ui| {
                let r = ui.toggle_value(&mut self.show_fps, "FPS counter");
                self.describe(&r, "Show the frame rate and frame time in the top-left corner of the picture. Also on F2.");
                let r = ui.toggle_value(&mut self.show_surface, "Controller");
                self.describe(&r, "Show a picture of the nanoKONTROL2 across the bottom of the screen, with every knob, fader and button labelled with what it does. It moves with the hardware, and right-clicking a control on it reassigns that control. Also on F4.");
                let r = ui.toggle_value(&mut self.show_fps_graph, "FPS graph");
                self.describe(&r, "Show a graph of the frame rate over the last two seconds in the top-left corner of the picture, so dips and stutters are visible. Also on F3.");
            });
            // What other apps are playing is only known on Windows.
            #[cfg(windows)]
            {
            let r = ui.checkbox(&mut self.track_card, "Show each track's title as it starts");
            self.describe(&r, "When a new track starts in a music app, show its title, artist, album and cover in the top-right corner of the picture for a few seconds. These are read from Windows on this PC; nothing is sent anywhere.");
            if self.track_card {
                let r = ui.checkbox(&mut self.track_card_fly, "Fly the title in from the distance");
                self.describe(&r, "Instead of the corner, the cover and title start as a dot far ahead and come towards you, growing until they fill the picture and thinning away as they do. With the stars flying they come at the stars' pace, from the point the stars stream out of, and the flight holds a steady course and speed until they have passed.");
            }
            }
            // HDR output has only been built and tried on Windows.
            #[cfg(windows)]
            {
            let label = match (self.hdr, self.hdr_active) {
                (true, true) => "HDR output (on)",
                (false, false) => "HDR output",
                (true, false) => "HDR output (applies when the app is restarted, if the display offers it)",
                (false, true) => "HDR output (turns off when the app is restarted)",
            };
            let r = ui.checkbox(&mut self.hdr, label);
            self.describe(&r, "Draw in high dynamic range, so the loudest parts of the picture are brighter than ordinary white. Needs an HDR display with HDR switched on in Windows. A change takes effect the next time the app starts. The two HDR brightness sliders are in the Colour section.");
            }
            let label = if self.vsync != self.vsync_active { "Vsync (applies when the app is restarted)" } else { "Vsync" };
            let r = ui.checkbox(&mut self.vsync, label);
            self.describe(&r, "On: each frame waits for the display, so the picture never tears and the frame rate matches the screen. Off: frames are drawn as fast as possible, which can tear. A change takes effect the next time the app starts.");
            ui.small(format!("{:.0} fps   analysis {:.2} ms   {}", 1000.0 / self.frame_ms.max(0.01), self.analysis_ms, self.adapter));
            ui.separator();

            if self.section(ui, "Audio source") {
            #[cfg(target_os = "android")]
            self.android_sources(ui);
            #[cfg(not(target_os = "android"))]
            {
            let current = self.capture.source.clone();
            let mut chosen = current.clone();
            let source_box = egui::ComboBox::from_id_salt("source").selected_text(current.label()).width(240.0).show_ui(ui, |ui| {
                ui.selectable_value(&mut chosen, Source::SystemOutput, Source::SystemOutput.label());
                let ticked = match &current {
                    Source::Outputs(names) => names.clone(),
                    _ => Vec::new(),
                };
                ui.selectable_value(&mut chosen, Source::Outputs(ticked), "Chosen output devices");
                for name in &self.inputs {
                    let s = Source::Input(name.clone());
                    let label = s.label();
                    ui.selectable_value(&mut chosen, s, label);
                }
                ui.selectable_value(&mut chosen, Source::TestSignal, Source::TestSignal.label());
            });
            self.describe(&source_box.response, &format!("Where the sound comes from. Default output device: whatever is sent to the {SYSTEM_NAME} default device. Chosen output devices: tick one or more devices and their sound is added together. Input: a microphone or line input. Test signal: a built-in kick, chord and hi-hat for checking the picture."));
            if let Source::Outputs(names) = &mut chosen {
                ui.small("Captures what apps send to each ticked device, added together:");
                egui::ScrollArea::vertical().id_salt("outputs").max_height(140.0).show(ui, |ui| {
                    for name in &self.outputs.clone() {
                        let mut on = names.contains(name);
                        let tick = ui.checkbox(&mut on, name);
                        self.describe(&tick, "Capture what programs send to this output device. Only its first two channels (front left and right) are used, so a surround upmix further down the chain is never seen.");
                        if tick.changed() {
                            if on {
                                names.push(name.clone());
                            } else {
                                names.retain(|n| n != name);
                            }
                        }
                    }
                });
            }
            if chosen != current {
                self.set_source(chosen);
            }
            }
            if self.status_at.elapsed().as_secs_f32() > 0.25 || self.status.is_empty() {
                self.status = self.capture.status();
                self.status_at = Instant::now();
            }
            for s in &self.status {
                let meters: Vec<String> = s.peaks_db.iter().map(|p| if *p < -99.0 { "–".into() } else { format!("{p:.0}") }).collect();
                ui.small(format!("{}: {}", s.name, s.format));
                ui.small(format!("   {}   peak per channel (dB): {}", if s.receiving { "receiving" } else { "silent" }, meters.join("  ")));
            }
            ui.small("Only the first two channels (front left and right) of each device are used.");
            for e in &self.capture.errors {
                ui.colored_label(egui::Color32::LIGHT_RED, e);
            }
            let r = ui.checkbox(&mut self.params.stereo, "Stereo");
            self.describe(&r, "Off: left and right are mixed and the picture is symmetric. On: each sound is drawn toward the side it is panned to (left of the screen for left, right for right; in the circle view, toward nine or three o'clock), and wide, out-of-step sound takes the surround colour. On the controller: S button 4.");
            if self.params.stereo {
                self.slider(ui, P::StereoEmphasis);
                self.slider(ui, P::SurroundAmount);
                let mut custom = self.params.surround_colour.is_some();
                ui.horizontal(|ui| {
                    let r = ui.checkbox(&mut custom, "Custom surround colour");
                    self.describe(&r, "Unticked: surround sound uses a colour chosen to stand apart from the current palette and the bass colour. Ticked: pick your own with the swatch.");
                    if r.changed() {
                        self.params.surround_colour = custom.then(|| params::SURROUND_COLOURS[self.params.palette()]);
                    }
                    if let Some(colour) = &mut self.params.surround_colour {
                        ui.color_edit_button_rgb(colour);
                    }
                });
            }
            }
            ui.separator();

            if self.section(ui, "Level") {
            let r = ui.checkbox(&mut self.params.auto_gain, format!("Auto-gain (now {:.0} dBFS)", self.reference_db));
            self.describe(&r, "On: full brightness follows the loudest part of the music, so quiet and loud tracks both fill the range. Off: full brightness is fixed at the Reference level. The number is the level currently treated as full brightness. On the controller: S button 5.");
            for id in [P::Reference, P::AutoGainSpeed, P::Range, P::Slope, P::Contrast, P::Brightness] {
                self.slider(ui, id);
            }
            }
            ui.separator();

            if self.section(ui, "Notes and timing") {
            for id in [P::BassSharpen, P::BassWindow, P::Sharpen, P::Detail] {
                self.slider(ui, id);
            }
            }
            ui.separator();

            if self.section(ui, "Bass boost") {
            let r = ui.checkbox(&mut self.params.bass_boost, "On");
            self.describe(&r, "Bass hits drive a pulse in a contrasting colour that grows from the middle of the picture, and optionally brighten the whole picture too. On the controller: S button 6.");
            for id in [P::BassAmount, P::BassCutoff, P::BassRelease, P::BassGlow] {
                self.slider(ui, id);
            }
            ui.horizontal(|ui| {
                for s in BassStyle::ALL {
                    let r = ui.selectable_value(&mut self.params.bass_style, s, s.label());
                    self.describe(&r, "How a bass hit is drawn. Pulse from the middle: a glowing disc in the bass colour that grows from the centre, over the picture. Fill dark areas: the bass colour fills only the black and nearly black parts of the picture, so it runs underneath everything else. Bass amount sets the strength and Bass glow how dim an area can be and still fill.");
                }
            });
            let r = ui.checkbox(&mut self.params.bass_flare, "Bass also flares the whole frame");
            self.describe(&r, "As well as the pulse in the middle, bass hits brighten the whole picture and dim it between hits.");
            let mut custom = self.params.bass_colour.is_some();
            ui.horizontal(|ui| {
                let r = ui.checkbox(&mut custom, "Custom bass colour");
                self.describe(&r, "Unticked: the bass pulse uses a colour chosen to contrast with the current palette. Ticked: pick your own colour with the swatch.");
                if r.changed() {
                    self.params.bass_colour = custom.then(|| PALETTES[self.params.palette()].accent);
                }
                if let Some(colour) = &mut self.params.bass_colour {
                    ui.color_edit_button_rgb(colour);
                }
            });
            }
            ui.separator();

            if self.section(ui, "Picture") {
            ui.horizontal(|ui| {
                ui.label("Shape");
                for s in Shape::ALL {
                    let r = ui.selectable_value(&mut self.params.shape, s, s.label());
                    self.describe(&r, "Cross: across and up are both frequency, and a pixel lights when both of its frequencies are sounding. Circle: frequency runs outward from the centre, so each note is a ring and a falling sweep is a shrinking circle. On the controller: R button 1.");
                }
            });
            if self.params.shape == Shape::Circle {
                ui.small("Circle: frequency runs outward from the centre. Flip x turns it inside out.");
                self.slider(ui, P::Angular);
            }
            let mirror_box = egui::ComboBox::from_label("Mirror").selected_text(self.params.mirror.label()).show_ui(ui, |ui| {
                for m in Mirror::ALL {
                    ui.selectable_value(&mut self.params.mirror, m, m.label());
                }
            });
            self.describe(&mirror_box.response, "Cross view only. Four quadrants: the picture is mirrored so bass meets in the centre and treble sits in the corners (the flip switches turn that round). Left / right and Top / bottom mirror one direction only. Off: no mirroring, bass at the top-right unless flipped. On the controller: S button 3 steps through them.");
            ui.horizontal(|ui| {
                let r = ui.checkbox(&mut self.params.flip_x, "Flip x");
                self.describe(&r, "Unticked, bass is in the middle. Cross view: ticked puts treble in the middle horizontally and bass at the left and right edges. Circle view: ticked turns it inside out, treble in the middle and bass at the edges. On the controller: S button 1.");
                let r = ui.checkbox(&mut self.params.flip_y, "Flip y");
                self.describe(&r, "Unticked, bass is in the middle. Cross view: ticked puts treble in the middle vertically and bass at the top and bottom edges. Circle view: reverses the angular pattern, if that is turned up. On the controller: S button 2.");
            });
            for id in [P::Combine, P::Decay, P::Smoothing, P::FreqLow, P::FreqHigh] {
                self.slider(ui, id);
            }
            }
            ui.separator();

            if self.section(ui, "3D (experimental)") {
            for id in [P::Tilt, P::ReliefHeight, P::Orbit, P::Flight, P::FlightDepth, P::LookAhead, P::Storm] {
                self.slider(ui, id);
            }
            ui.horizontal(|ui| {
                ui.label("Surface");
                for m in Material::ALL {
                    let r = ui.selectable_value(&mut self.params.material, m, m.label());
                    self.describe(&r, "What the 3D surface looks as if it is made of. Surface strength, below, sets how far it goes. Matte: plain light and shade. Gloss: shiny plastic, with white highlights that slide over the ridges as the camera moves. Metal: polished, mirroring a bright sky in its own colour. Glass: dim face on and bright at the edges, with sharp glints. Highlights only appear where there is sound.");
                }
            });
            if self.params.material != Material::Matte {
                self.slider(ui, P::SurfaceAmount);
            }
            }
            ui.separator();

            if self.section(ui, "Colour") {
            self.slider(ui, P::Palette);
            #[cfg(windows)]
            {
            let r = ui.checkbox(&mut self.params.palette_from_cover, "Colours from the album cover");
            self.describe(&r, "Take the colours from the cover of whatever track is playing, so each track has its own look: its two strongest colours from dark to bright, with a bass colour chosen to stand apart from them. The cover is read from Windows on this PC; nothing is sent anywhere. With nothing playing, or a player that shows no cover, the palette above is used.");
            }
            self.slider(ui, P::PaletteDrift);
            self.slider(ui, P::Banding);
            self.slider(ui, P::Bloom);
            ui.horizontal(|ui| {
                ui.label("Background");
                for b in Backdrop::ALL {
                    // The cover is only known on Windows.
                    if b == Backdrop::Cover && !cfg!(windows) {
                        continue;
                    }
                    let r = ui.selectable_value(&mut self.params.backdrop, b, b.label());
                    self.describe(&r, "What shows where the picture is dark. Black: nothing, so silence is black. Album cover: the cover of the track that is playing, blurred and dim, read from Windows on this PC. Stars have a section of their own, below.");
                }
            });
            if self.params.backdrop != Backdrop::Off {
                self.slider(ui, P::BackdropAmount);
            }
            if self.hdr_active {
                self.slider(ui, P::HdrBase);
                self.slider(ui, P::HdrPeak);
                let r = ui.checkbox(&mut self.hdr_pattern, "HDR test pattern");
                self.describe(&r, "Show seven white patches across the picture. Five are at fixed brightness, from 80 to 1600 nits; patches at or above what the display can reach look the same as each other. The last two follow the base and peak brightness sliders.");
            }
            let r = ui.checkbox(&mut self.params.reverse_palette, "Reverse palette");
            self.describe(&r, "Swaps the palette end for end, so its loudest colour becomes its quietest. Silence stays black. On the controller: S button 7.");
            }
            ui.separator();

            if self.section(ui, "Starfield") {
            let r = ui.checkbox(&mut self.params.stars, "Show stars");
            self.describe(&r, "A field of stars behind the picture, showing where it is dark. They twinkle with the treble. Still, they drift slowly when the picture is flat, and in 3D they surround it and move with the camera. With a flight speed above 0 the view flies forward through them.");
            if self.params.stars {
                for id in [P::StarBrightness, P::StarDensity, P::StarVariety, P::StarSpeed, P::StarBass, P::StarTurn, P::StarTurnHold] {
                    self.slider(ui, id);
                }
            }
            }
            ui.separator();

            // Lyrics need to know what is playing, which only the Windows build can ask.
            #[cfg(windows)]
            {
            if self.section(ui, "Lyrics") {
            let r = ui.checkbox(&mut self.lyrics, "Show lyrics");
            self.describe(&r, "Shows the words of the song over the picture, in time with the music. AudioVis reads the title and artist of what is playing from Windows and looks the lyrics up on lrclib.net, a free, crowd-sourced lyrics site. So while this is on, the title, artist, album and length of each track you play are sent to that site; nothing is sent while it is off. Results are kept on this PC, so each track is only asked about once. On the controller: the marker SET button.");
            if self.lyrics {
                ui.small(self.lyrics_status());
                ui.horizontal(|ui| {
                    for place in LyricsPlace::ALL {
                        let r = ui.selectable_value(&mut self.params.lyrics_place, place, place.label());
                        self.describe(&r, "Where the lyrics go. Top, Middle and Bottom show the line being sung across the picture, with the line before fading out above it and the line to come below. Circle runs them round the far side of a circle about the middle of the picture, scrolling past the top as they are sung; in 3D they lie on the picture and tilt and turn with it. Crawl lays them back in yellow like the opening titles of a space film: they come in at the bottom and roll away up the picture into the distance.");
                    }
                });
                if self.params.lyrics_place == LyricsPlace::Crawl {
                    let r = ui.checkbox(&mut self.params.lyrics_justify, "Justify the crawl");
                    self.describe(&r, "Make every row of the crawl the same width, as the text of a film's opening crawl is. Each line is made larger or smaller to fill the width, a long one is broken into even rows, and what is left is taken up between the words and letters. A very short line is only stretched so far, and then sits in the middle.");
                }
                self.slider(ui, P::LyricsSize);
                self.slider(ui, P::LyricsStrength);
                let r = ui.checkbox(&mut self.params.lyrics_preview, "Show the next line");
                self.describe(&r, "Show the line to come, small and dim, under the one being sung.");
                self.slider(ui, P::LyricsOffset);
                ui.small("The offset is kept separately for each music app. On the controller: the marker < and > buttons move it 10 ms.");
            }
            ui.small("Lyrics come from LRCLIB (lrclib.net) and remain the copyright of their owners.");
            }
            ui.separator();
            }

            if self.section(ui, "Presets") {
            ui.horizontal(|ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut self.preset_name).hint_text("name").desired_width(150.0));
                self.describe(&r, "Name for a preset. Presets store every slider and switch, but not the audio source or the controller mapping.");
                let r = ui.button("Save");
                self.describe(&r, "Save the current settings under this name. An existing preset with the same name is replaced.");
                if r.clicked() {
                    self.save_preset();
                }
            });
            for name in self.presets.clone() {
                let r = ui.button(&name);
                self.describe(&r, "Load this preset. Faders on the controller then need to be moved to the new positions before they take over.");
                if r.clicked() {
                    self.load_preset(&name);
                }
            }
            let r = ui.button("Reset everything to defaults");
            self.describe(&r, "Put every slider and switch back to how the app first started. Saved presets are not touched.");
            if r.clicked() {
                self.params = Params::default();
                self.controller.release_all();
            }
            }
            ui.separator();

            if self.section(ui, "MIDI controller") {
            match (&self.controller.port, &self.controller.error) {
                (Some(port), _) => ui.label(format!("Connected: {port}")),
                (None, Some(e)) => ui.colored_label(egui::Color32::LIGHT_RED, e),
                _ => ui.label("Not connected"),
            };
            if let Some((cc, value)) = self.controller.last_message {
                ui.small(format!("last message: CC {cc} = {value}"));
            }
            ui.small("Right-click any slider for MIDI learn. A fader takes over once it reaches the slider's position.");
            let r = ui.button("Restore default controller mapping");
            self.describe(&r, "Undo any reassignments. Strips 1 to 5, fader then knob: range and contrast; note sharpening and speed vs pitch detail; bass amount and bass glow; decay and smoothing; combine blend and highest frequency; palette and colour banding; 3D tilt and 3D height; 3D orbit speed and 3D flight speed.");
            if r.clicked() {
                self.bindings = Bindings::default();
                self.controller.release_all();
            }
            }
        });
    }
}

/// The name the crawl's typeface goes by.
const CRAWL_FONT: &str = "crawl";

/// Set up the typeface for the lyrics crawl: a bold condensed gothic, as near
/// as the system has to the one on film. It is read from the system's own
/// fonts where there is one, and otherwise the app's usual typeface is used.
fn crawl_font(ctx: &egui::Context) {
    #[cfg(windows)]
    const CANDIDATES: [&str; 3] = ["C:/Windows/Fonts/framd.ttf", "C:/Windows/Fonts/arialbd.ttf", "C:/Windows/Fonts/segoeuib.ttf"];
    #[cfg(target_os = "macos")]
    const CANDIDATES: [&str; 2] = ["/System/Library/Fonts/Supplemental/Arial Bold.ttf", "/Library/Fonts/Arial Bold.ttf"];
    #[cfg(not(any(windows, target_os = "macos")))]
    const CANDIDATES: [&str; 2] = ["/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf", "/system/fonts/Roboto-Bold.ttf"];
    let mut fonts = egui::FontDefinitions::default();
    let mut family = fonts.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default();
    if let Some(bytes) = CANDIDATES.iter().find_map(|path| std::fs::read(path).ok()) {
        fonts.font_data.insert(CRAWL_FONT.to_string(), egui::FontData::from_owned(bytes).into());
        family.insert(0, CRAWL_FONT.to_string());
    }
    fonts.families.insert(egui::FontFamily::Name(CRAWL_FONT.into()), family);
    ctx.set_fonts(fonts);
}

/// How one row of laid-out text is moved to fill a block: made larger or
/// smaller, then its words and letters spread to take up what is left.
struct RowFit {
    /// The row's top and left as laid out, and after fitting.
    top: f32,
    left: f32,
    new_top: f32,
    new_left: f32,
    scale: f32,
    /// Extra width shared out along the row.
    slack: f32,
    /// Each letter's left edge as laid out, and the share of the slack before it.
    stops: Vec<(f32, f32)>,
}

/// How laid-out text is fitted to a block. Justified, every row is the full
/// width of the block, like the text of a film's opening crawl.
struct Fit {
    rows: Vec<RowFit>,
    height: f32,
}

impl Fit {
    /// Fit `galley` (laid out centred) to a block `width` wide, or with no
    /// width leave it exactly as it is. `size` is the size of its type.
    fn of(galley: &egui::Galley, width: Option<f32>, size: f32) -> Self {
        /// A row is made at most this much smaller or larger to fit.
        const SCALE: (f32, f32) = (0.7, 1.6);
        let mut rows = Vec::new();
        let mut new_top = 0.0;
        for (r, row) in galley.rows.iter().enumerate() {
            let top = row.pos.y;
            let tall = galley.rows.get(r + 1).map_or(galley.size().y, |next| next.pos.y) - top;
            // The row from its first letter to its last, leaving out spaces at the ends.
            let inked: Vec<usize> = (0..row.glyphs.len()).filter(|i| !row.glyphs[*i].chr.is_whitespace()).collect();
            let (Some(&first), Some(&last), Some(width)) = (inked.first(), inked.last(), width) else {
                rows.push(RowFit { top, left: 0.0, new_top: top, new_left: 0.0, scale: 1.0, slack: 0.0, stops: Vec::new() });
                new_top = top + tall;
                continue;
            };
            let left = row.pos.x + row.glyphs[first].pos.x;
            let right = row.pos.x + row.glyphs[last].pos.x + row.glyphs[last].advance_width;
            let scale = (width / (right - left).max(1.0)).clamp(SCALE.0, SCALE.1);
            let used = (right - left) * scale;
            // What is left over goes between the words first and the letters
            // second, and only so far: a very short line stays short of the
            // edges and centred instead of being pulled apart.
            let share = |i: usize| if row.glyphs[i].chr.is_whitespace() { 5.0 } else { 1.0 };
            let total: f32 = (first..last).map(share).sum();
            let most = (first..last).map(|i| if row.glyphs[i].chr.is_whitespace() { 1.0 } else { 0.09 }).sum::<f32>() * size * scale;
            let slack = (width - used).clamp(0.0, most);
            let mut before = 0.0;
            let stops = (0..row.glyphs.len())
                .map(|i| {
                    let stop = (row.pos.x + row.glyphs[i].pos.x, if total > 0.0 { (before / total).min(1.0) } else { 0.0 });
                    if (first..last).contains(&i) {
                        before += share(i);
                    }
                    stop
                })
                .collect();
            rows.push(RowFit { top, left, new_top, new_left: -0.5 * (used + slack), scale, slack, stops });
            new_top += tall * scale;
        }
        Self { rows, height: new_top }
    }

    /// Where a corner `at` of the letter whose middle is `middle` goes.
    fn place(&self, middle: egui::Pos2, at: egui::Pos2) -> egui::Pos2 {
        let Some(row) = self.rows.iter().rev().find(|row| row.top <= middle.y).or(self.rows.first()) else { return at };
        if row.stops.is_empty() {
            return at;
        }
        let share = row.stops.iter().rev().find(|(left, _)| *left <= middle.x).map_or(0.0, |(_, share)| *share);
        egui::pos2(row.new_left + (at.x - row.left) * row.scale + row.slack * share, row.new_top + (at.y - row.top) * row.scale)
    }
}

/// The colours of the palette chosen in the panel.
fn palette_look(params: &Params) -> cover::Look {
    let palette = &PALETTES[params.palette()];
    cover::Look { stops: palette.stops.to_vec(), accent: palette.accent, surround: params::SURROUND_COLOURS[params.palette()] }
}

/// One word of a line of lyrics as laid out on screen: when it is sung and
/// where it sits in the layout.
struct LyricWord {
    start: f64,
    /// When the next word starts, or the line is over.
    end: f64,
    row: usize,
    left: f32,
    right: f32,
}

/// The words of a laid-out line and when each is sung, for lyrics that carry
/// a time for every word. Most only time whole lines, and for those there
/// are no words to pick out: guessing the times does not stay with the singer.
fn lyric_words(galley: &egui::Galley, line: &lyrics::Line, next_start: f64) -> Vec<LyricWord> {
    let Some((last, _)) = line.words.last() else { return Vec::new() };
    // Each word's row, left and right edges, and where its first letter is in the text.
    let mut found: Vec<(usize, f32, f32, usize)> = Vec::new();
    let mut letter = 0;
    for (r, row) in galley.rows.iter().enumerate() {
        let mut inside = false;
        for glyph in &row.glyphs {
            let left = row.pos.x + glyph.pos.x;
            let right = left + glyph.advance_width;
            if glyph.chr.is_whitespace() {
                inside = false;
            } else if let (true, Some(word)) = (inside, found.last_mut()) {
                word.2 = right;
            } else {
                found.push((r, left, right, letter));
                inside = true;
            }
            letter += 1;
        }
    }

    // The letter each timed piece ends before, and when the piece starts.
    let mut pieces = Vec::new();
    let mut letters = 0;
    for (start, piece) in &line.words {
        letters += piece.chars().count();
        pieces.push((letters, *start));
    }
    let starts: Vec<f64> = found.iter().map(|word| pieces.iter().find(|(end, _)| word.3 < *end).map_or(*last, |(_, start)| *start)).collect();
    let end = (last + 1.0).min(next_start);
    found
        .iter()
        .enumerate()
        .map(|(i, &(row, left, right, _))| LyricWord { start: starts[i], end: starts.get(i + 1).copied().unwrap_or(end), row, left, right })
        .collect()
}

/// Gaussian blur across neighbouring bins.
fn blur(levels: &mut [f32], sigma: f32) {
    let radius = (sigma * 2.5).ceil() as i32;
    let weights: Vec<f32> = (-radius..=radius).map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp()).collect();
    let total: f32 = weights.iter().sum();
    let source = levels.to_vec();
    let last = source.len() as i32 - 1;
    for (b, out) in levels.iter_mut().enumerate() {
        let acc: f32 = weights
            .iter()
            .enumerate()
            .map(|(k, w)| w * source[(b as i32 + k as i32 - radius).clamp(0, last) as usize])
            .sum();
        *out = acc / total;
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let dt = self.last_frame.elapsed().as_secs_f32().min(0.1);
        self.last_frame = Instant::now();
        self.frames += 1;
        if self.frames == 30 && self.fullscreen {
            // Fullscreen is requested once the window is up; asking at creation was not honoured.
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
        }
        self.frame_ms += (dt * 1000.0 - self.frame_ms) * 0.05;
        if self.frame_history.len() == FPS_HISTORY {
            self.frame_history.pop_front();
        }
        self.frame_history.push_back(dt * 1000.0);

        // Keyboard. Typing a preset name must not trigger shortcuts.
        if !ctx.egui_wants_keyboard_input() {
            if ctx.input(|i| i.key_pressed(egui::Key::Tab) || i.key_pressed(egui::Key::F1)) {
                self.show_panel = !self.show_panel;
            }
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && self.fullscreen {
                self.fullscreen = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::F2)) {
            self.show_fps = !self.show_fps;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::F4)) {
            self.show_surface = !self.show_surface;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::F3)) {
            self.show_fps_graph = !self.show_fps_graph;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::F5)) {
            self.card_fly_asked = true;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::F11)) {
            self.fullscreen = !self.fullscreen;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
        }

        #[cfg(target_os = "android")]
        self.controller.reconnect(&self.options.midi_port);
        self.controller.apply(
            &mut self.params,
            &mut self.bindings,
            &mut self.learning,
            &mut self.show_panel,
            &mut self.show_surface,
            &mut self.lyrics,
            &mut self.card_fly_asked,
        );
        self.params.glide(dt);
        // The panel is drawn at the picture's base brightness.
        eframe::egui_wgpu::HDR_UI_SCALE.store((self.params.get(P::HdrBase) / 80.0).to_bits(), std::sync::atomic::Ordering::Relaxed);
        self.frame_dt = dt.min(0.05);
        self.flight_time += dt * self.params.get(P::Flight);
        let orbit = self.params.get(P::Orbit);
        self.orbit_angle = if orbit.abs() < 0.05 { 0.0 } else { (self.orbit_angle + orbit.to_radians() * dt) % std::f32::consts::TAU };
        self.update_playing();
        self.update_look(dt);
        let (gain, glow) = self.analyse(dt);
        self.update_camera(dt, gain, glow);
        // The flight through the stars: a bass hit surges it forward or holds
        // it back. Kept small so it stays exact, and never quite 0 while flying.
        let star_speed = self.params.get(P::StarSpeed);
        // While a title is flying in, the flight holds a steady course and
        // speed, so the title comes straight on; both ease back afterwards.
        let title_coming = self.card.is_some() && self.card_flying && self.card_near < 1.0;
        self.star_steady += (if title_coming { 1.0 } else { 0.0 } - self.star_steady) * (1.0 - (-dt / 0.35).exp());
        let free = 1.0 - self.star_steady;
        let with_bass = self.params.get(P::StarBass) * free;
        let pace = if with_bass >= 0.0 { 1.0 + 4.0 * with_bass * self.bass_env } else { 1.0 + with_bass * self.bass_env };
        // The slider gathers pace as it goes up: 1 is a gentle drift, 6 over twenty times that.
        self.star_rate = 0.06 * star_speed * (1.0 + star_speed) * pace;
        // Turns: each hit swings the heading a new way (successive hits are
        // spread round the compass), as far as the hit is strong, and it
        // straightens as the hit dies away. While it is off centre the stars
        // slide across, faster the faster the flight.
        if self.bass_env > 0.45 && self.star_bass_was <= 0.45 {
            self.star_hits = self.star_hits.wrapping_add(1);
        }
        self.star_bass_was = self.bass_env;
        let way = self.star_hits as f32 * 2.399_963;
        // The turn is as hard as the hit, held there for the hold time, then
        // let go as quickly as the bass itself dies away.
        if self.bass_env >= self.star_turn_level {
            (self.star_turn_level, self.star_turn_held) = (self.bass_env, 0.0);
        } else {
            self.star_turn_held += dt;
            if self.star_turn_held > self.params.get(P::StarTurnHold) {
                let release = (-dt * 1000.0 / self.params.get(P::BassRelease)).exp();
                self.star_turn_level = self.bass_env.max(self.star_turn_level * release);
            }
        }
        let reach = 0.45 * self.params.get(P::StarTurn) * self.star_turn_level * free;
        let ease = 1.0 - (-dt / 0.12).exp();
        for (i, wanted) in [reach * way.cos(), reach * way.sin()].into_iter().enumerate() {
            self.star_heading[i] += (wanted - self.star_heading[i]) * ease;
            self.star_slide[i] = (self.star_slide[i] - 14.0 * self.star_heading[i] * self.star_rate * dt) % 4096.0;
        }
        self.star_travel = if star_speed > 0.001 && self.params.stars { (self.star_travel + self.star_rate * dt) % 4096.0 + 1e-6 } else { 0.0 };

        if self.show_panel {
            egui::Panel::right("controls").resizable(false).default_size(300.0).show(ui, |ui| self.panel(ui));
        }
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            // A finger swipes the picture as well as tapping it.
            let sense = if cfg!(target_os = "android") { egui::Sense::click_and_drag() } else { egui::Sense::click() };
            let (rect, response) = ui.allocate_exact_size(ui.available_size(), sense);
            let mut levels = Vec::with_capacity(render::ROWS as usize * N_BINS);
            levels.extend_from_slice(&self.shown[0]);
            levels.extend_from_slice(&self.shown[1]);
            levels.extend_from_slice(&self.pan);
            levels.extend_from_slice(&self.wide);
            render::push_maxima(&mut levels, &self.shown[0]);
            render::push_maxima(&mut levels, &self.shown[1]);
            render::push_bin_maxima(&mut levels, &self.shown[0]);
            render::push_bin_maxima(&mut levels, &self.shown[1]);
            self.aspect = rect.width() / rect.height().max(1.0);
            ui.painter().add(eframe::egui_wgpu::Callback::new_paint_callback(
                rect,
                render::Frame {
                    uniforms: self.uniforms(gain, glow, rect.width() / rect.height().max(1.0)),
                    levels,
                    size: [(rect.width() * ctx.pixels_per_point()).round() as u32, (rect.height() * ctx.pixels_per_point()).round() as u32],
                    cover: self.new_cover(),
                },
            ));
            self.draw_lyrics(ui.painter(), rect);
            self.draw_track_card(ui.painter(), rect);
            self.draw_fps(ui.painter(), rect);
            if self.hdr_active && self.hdr_pattern {
                let labels = [
                    "80".to_string(),
                    "200".to_string(),
                    "400".to_string(),
                    "800".to_string(),
                    "1600".to_string(),
                    format!("base {:.0}", self.params.get(P::HdrBase)),
                    format!("peak {:.0}", self.params.get(P::HdrPeak).max(self.params.get(P::HdrBase))),
                ];
                for (i, label) in labels.iter().enumerate() {
                    let at = egui::pos2(rect.left() + (0.2 + 0.1 * i as f32) * rect.width(), rect.center().y - 0.09 * rect.height() - 6.0);
                    ui.painter().text(at, egui::Align2::CENTER_BOTTOM, format!("{label} nits"), egui::FontId::proportional(14.0), egui::Color32::WHITE);
                }
            }
            if self.show_surface {
                let hovered = surface::Surface {
                    params: &mut self.params,
                    bindings: &mut self.bindings,
                    controller: &mut self.controller,
                    show_panel: self.show_panel,
                    show_surface: true,
                    lyrics: self.lyrics,
                }
                .show(ui, rect);
                if hovered.is_some() {
                    self.hint.hovered = hovered;
                }
            }
            response.context_menu(|ui| {
                ui.strong("Palette");
                for (i, palette) in PALETTES.iter().enumerate() {
                    if ui.radio(self.params.palette() == i, palette.name).clicked() {
                        self.params.set(P::Palette, i as f32);
                        self.controller.release(&self.bindings, P::Palette);
                    }
                }
                ui.separator();
                ui.checkbox(&mut self.params.reverse_palette, "Reverse");
                ui.separator();
                ui.checkbox(&mut self.show_panel, "Show controls (Tab)");
            });
            #[cfg(target_os = "android")]
            self.touch(&response);
        });

        // Self-test: screenshot, report timings, exit.
        if let Some(path) = self.options.selftest.clone() {
            let elapsed = self.started.elapsed().as_secs_f32();
            if elapsed > self.options.seconds - 3.0 && !self.shot_requested {
                self.recent.push(dt * 1000.0);
            }
            if elapsed > self.options.seconds && !self.shot_requested {
                self.shot_requested = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            let shot = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
            if let Some(image) = shot {
                let [w, h] = image.size;
                let saved = image::save_buffer(&path, image.as_raw(), w as u32, h as u32, image::ColorType::Rgba8);
                println!(
                    "selftest: {w}x{h} px, {} frames in {elapsed:.2} s = {:.1} fps, frame {:.2} ms, analysis {:.3} ms, sample rate {}, adapter {}, midi {:?}, audio error {:?}, saved {:?}",
                    self.frames,
                    self.frames as f32 / elapsed,
                    self.frame_ms,
                    self.analysis_ms,
                    self.vqt.sample_rate,
                    self.adapter,
                    self.controller.port,
                    self.capture.errors,
                    saved.is_ok()
                );
                for s in self.capture.status() {
                    println!("selftest: source {} | {} | receiving {} | peak per channel dB {:.0?}", s.name, s.format, s.receiving, s.peaks_db);
                }
                self.recent.sort_by(|a, b| a.total_cmp(b));
                let n = self.recent.len().max(1);
                println!(
                    "selftest: last 3 s: {} frames, median {:.2} ms, 99th percentile {:.2} ms, worst {:.2} ms, over 9 ms: {}",
                    self.recent.len(),
                    self.recent.get(n / 2).copied().unwrap_or(0.0),
                    self.recent.get(n * 99 / 100).copied().unwrap_or(0.0),
                    self.recent.last().copied().unwrap_or(0.0),
                    self.recent.iter().filter(|t| **t > 9.0).count()
                );
                #[cfg(windows)]
                if self.lyrics {
                    println!("selftest: lyrics: {}", self.lyrics_status());
                }
                // The cover as read from the player, beside the screenshot, to compare the colours with.
                if let Some(cover) = self.playing.as_ref().and_then(|now| now.cover.as_ref()) {
                    let _ = image::save_buffer(path.with_extension("cover.png"), &cover.rgba, nowplaying::COVER_SIZE, nowplaying::COVER_SIZE, image::ColorType::Rgba8);
                }
                if self.params.palette_from_cover {
                    let hex = |c: &[f32; 3]| format!("#{:02x}{:02x}{:02x}", (c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8);
                    println!(
                        "selftest: colours from cover {}: {:?}, bass {}, surround {}",
                        self.playing.as_ref().is_some_and(|now| now.cover.is_some()),
                        self.look.stops.iter().map(hex).collect::<Vec<_>>(),
                        hex(&self.look.accent),
                        hex(&self.look.surround)
                    );
                }
                let (ppp, info) = (ctx.pixels_per_point(), ctx.input(|i| i.viewport().clone()));
                println!(
                    "selftest: pixels per point {ppp}, fullscreen {:?}, monitor {:?}, inner {:?}",
                    info.fullscreen, info.monitor_size, info.inner_rect
                );
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }

        self.show_hint(&ctx);
        #[cfg(target_os = "android")]
        self.save_changes();
        ctx.request_repaint();
    }

    fn on_exit(&mut self) {
        if self.options.selftest.is_none() {
            self.save_settings();
        }
        // The screen is going away, and the sound source with it.
        #[cfg(target_os = "android")]
        android::detach();
    }
}

#[allow(dead_code)]
fn _all_settings_have_a_slider() {
    // Compile-time reminder: DEFS drives presets and MIDI; the panel lists each one by hand.
    let _ = DEFS.len();
}
