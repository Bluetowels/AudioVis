//! MIDI controller input: faders and knobs drive settings, buttons flip switches.
//! Defaults match a Korg nanoKONTROL2 on its factory mapping.

use crate::params::{P, Params, Toggle, def_of};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::mpsc::{Receiver, channel};

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub enum Action {
    Toggle(Toggle),
    MirrorNext,
    ShapeNext,
    PaletteNext,
    PalettePrevious,
    TogglePanel,
    /// Show or hide the picture of the controller.
    ToggleSurface,
}

impl Action {
    pub const ALL: [Action; 12] = [
        Action::Toggle(Toggle::FlipX),
        Action::Toggle(Toggle::FlipY),
        Action::MirrorNext,
        Action::ShapeNext,
        Action::Toggle(Toggle::Stereo),
        Action::Toggle(Toggle::AutoGain),
        Action::Toggle(Toggle::BassBoost),
        Action::Toggle(Toggle::ReversePalette),
        Action::PalettePrevious,
        Action::PaletteNext,
        Action::TogglePanel,
        Action::ToggleSurface,
    ];

    /// A name short enough to sit beside a button.
    pub fn label(self) -> &'static str {
        match self {
            Action::Toggle(Toggle::FlipX) => "Flip x",
            Action::Toggle(Toggle::FlipY) => "Flip y",
            Action::Toggle(Toggle::Stereo) => "Stereo",
            Action::Toggle(Toggle::AutoGain) => "Auto-gain",
            Action::Toggle(Toggle::BassBoost) => "Bass boost",
            Action::Toggle(Toggle::ReversePalette) => "Reverse",
            Action::MirrorNext => "Mirror",
            Action::ShapeNext => "Shape",
            Action::PaletteNext => "Pal +",
            Action::PalettePrevious => "Pal -",
            Action::TogglePanel => "Panel",
            Action::ToggleSurface => "Controller",
        }
    }

    pub fn help(self) -> &'static str {
        match self {
            Action::Toggle(Toggle::FlipX) => "Flip x on or off",
            Action::Toggle(Toggle::FlipY) => "Flip y on or off",
            Action::Toggle(Toggle::Stereo) => "Stereo on or off",
            Action::Toggle(Toggle::AutoGain) => "Auto-gain on or off",
            Action::Toggle(Toggle::BassBoost) => "Bass boost on or off",
            Action::Toggle(Toggle::ReversePalette) => "Reverse the palette",
            Action::MirrorNext => "Step to the next mirror mode",
            Action::ShapeNext => "Switch between the cross and circle views",
            Action::PaletteNext => "Next palette",
            Action::PalettePrevious => "Previous palette",
            Action::TogglePanel => "Show or hide the settings panel",
            Action::ToggleSurface => "Show or hide this picture of the controller",
        }
    }
}

/// Which control number does what. Saved with the settings.
#[derive(Clone, Serialize, Deserialize)]
pub struct Bindings {
    /// Control number -> setting key.
    pub continuous: BTreeMap<u8, String>,
    pub buttons: BTreeMap<u8, Action>,
}

impl Default for Bindings {
    fn default() -> Self {
        // One strip per pair of related settings: (fader, knob).
        let strips = [
            (Some(P::Range), Some(P::Contrast)),
            (Some(P::Sharpen), Some(P::Detail)),
            (Some(P::BassAmount), Some(P::BassGlow)),
            (Some(P::Decay), Some(P::Smoothing)),
            (Some(P::Combine), Some(P::FreqHigh)),
            (Some(P::Palette), Some(P::Banding)),
            (Some(P::Tilt), Some(P::ReliefHeight)),
            (Some(P::Orbit), Some(P::Flight)),
        ];
        let strips: [(Option<P>, Option<P>); 8] = strips;
        let mut continuous = BTreeMap::new();
        for (i, (fader, knob)) in strips.into_iter().enumerate() {
            if let Some(p) = fader {
                continuous.insert(i as u8, def_of(p).key.to_string());
            }
            if let Some(p) = knob {
                continuous.insert(16 + i as u8, def_of(p).key.to_string());
            }
        }
        // S buttons, left to right, then the track arrows for palettes.
        let buttons = BTreeMap::from([
            (32, Action::Toggle(Toggle::FlipX)),
            (33, Action::Toggle(Toggle::FlipY)),
            (34, Action::MirrorNext),
            (35, Action::Toggle(Toggle::Stereo)),
            (36, Action::Toggle(Toggle::AutoGain)),
            (37, Action::Toggle(Toggle::BassBoost)),
            (38, Action::Toggle(Toggle::ReversePalette)),
            (39, Action::TogglePanel),
            (64, Action::ShapeNext),
            (58, Action::PalettePrevious),
            (59, Action::PaletteNext),
        ]);
        Self { continuous, buttons }
    }
}

impl Bindings {
    /// Give actions added since a mapping was saved their default button, if it is free.
    pub fn add_new_defaults(&mut self) {
        for (cc, action) in Self::default().buttons {
            let known = self.buttons.values().any(|a| *a == action);
            if !known && !self.buttons.contains_key(&cc) && !self.continuous.contains_key(&cc) {
                self.buttons.insert(cc, action);
            }
        }
    }

    pub fn control_for(&self, id: P) -> Option<u8> {
        let key = def_of(id).key;
        self.continuous.iter().find(|(_, k)| k.as_str() == key).map(|(cc, _)| *cc)
    }

    pub fn bind(&mut self, cc: u8, id: P) {
        let key = def_of(id).key;
        self.continuous.retain(|_, k| k.as_str() != key);
        self.buttons.remove(&cc);
        self.continuous.insert(cc, key.to_string());
    }

    pub fn unbind(&mut self, id: P) {
        let key = def_of(id).key;
        self.continuous.retain(|_, k| k.as_str() != key);
    }
}

pub struct Controller {
    _connection: Option<midir::MidiInputConnection<()>>,
    events: Option<Receiver<(u8, u8)>>,
    pub port: Option<String>,
    pub error: Option<String>,
    /// Faders are not motorised: one only takes over once it reaches the
    /// setting's current position, so nothing jumps.
    picked_up: HashMap<u8, bool>,
    last_norm: HashMap<u8, f32>,
    /// The position before that, for telling when a fader crosses its setting.
    previous: HashMap<u8, f32>,
    /// Highest value each control has sent; some faders stop short of 127.
    top: HashMap<u8, u8>,
    pub last_message: Option<(u8, u8)>,
    /// When each button last sent a message, to flash it on the controller picture.
    pressed: HashMap<u8, std::time::Instant>,
}

impl Controller {
    pub fn open(name_contains: &str) -> Self {
        let mut c = Self {
            _connection: None,
            events: None,
            port: None,
            error: None,
            picked_up: HashMap::new(),
            last_norm: HashMap::new(),
            previous: HashMap::new(),
            top: HashMap::new(),
            last_message: None,
            pressed: HashMap::new(),
        };
        match Self::connect(name_contains) {
            Ok((connection, events, port)) => {
                c._connection = Some(connection);
                c.events = Some(events);
                c.port = Some(port);
            }
            Err(e) => c.error = Some(e),
        }
        c
    }

    fn connect(name_contains: &str) -> Result<(midir::MidiInputConnection<()>, Receiver<(u8, u8)>, String), String> {
        let input = midir::MidiInput::new("audiovis").map_err(|e| e.to_string())?;
        let port = input
            .ports()
            .into_iter()
            .find(|p| input.port_name(p).map(|n| n.contains(name_contains)).unwrap_or(false))
            .ok_or_else(|| format!("no MIDI input named like \"{name_contains}\""))?;
        let name = input.port_name(&port).map_err(|e| e.to_string())?;
        let (tx, rx) = channel();
        let connection = input
            .connect(
                &port,
                "audiovis-in",
                move |_, message, _| {
                    // Control change on any channel.
                    if message.len() == 3 && message[0] & 0xF0 == 0xB0 {
                        let _ = tx.send((message[1], message[2]));
                    }
                },
                (),
            )
            .map_err(|e| e.to_string())?;
        Ok((connection, rx, name))
    }

    /// A setting was changed from the GUI or a preset: its fader must catch up again.
    pub fn release(&mut self, bindings: &Bindings, id: P) {
        if let Some(cc) = bindings.control_for(id) {
            self.picked_up.insert(cc, false);
        }
    }

    /// Where the hardware knob or fader is, 0..1, if it has been moved this run.
    pub fn position(&self, cc: u8) -> Option<f32> {
        self.last_norm.get(&cc).copied()
    }

    pub fn pressed_recently(&self, cc: u8) -> bool {
        self.pressed.get(&cc).is_some_and(|at| at.elapsed().as_secs_f32() < 0.18)
    }

    pub fn release_all(&mut self) {
        self.picked_up.clear();
    }

    /// Record a knob or fader position from its raw value; returns it as 0..1.
    fn note_position(&mut self, cc: u8, value: u8) -> f32 {
        let top = self.top.entry(cc).or_insert(125);
        *top = (*top).max(value);
        let norm = (value as f32 / *top as f32).min(1.0);
        self.last_norm.insert(cc, norm);
        norm
    }

    /// Apply everything received since the last frame.
    pub fn apply(
        &mut self,
        params: &mut Params,
        bindings: &mut Bindings,
        learning: &mut Option<P>,
        show_panel: &mut bool,
        show_surface: &mut bool,
    ) {
        let Some(events) = &self.events else { return };
        let messages: Vec<(u8, u8)> = events.try_iter().collect();
        for (cc, value) in messages {
            self.last_message = Some((cc, value));
            if let Some(id) = learning.take() {
                bindings.bind(cc, id);
                self.picked_up.insert(cc, true);
            }

            if let Some(action) = bindings.buttons.get(&cc).copied() {
                // Buttons are treated as toggles: every message is one press.
                match action {
                    Action::Toggle(t) => params.toggle(t),
                    Action::MirrorNext => params.mirror = params.mirror.next(),
                    Action::ShapeNext => params.shape = params.shape.next(),
                    Action::PaletteNext => params.step_palette(1),
                    Action::PalettePrevious => params.step_palette(-1),
                    Action::TogglePanel => *show_panel = !*show_panel,
                    Action::ToggleSurface => *show_surface = !*show_surface,
                }
                self.pressed.insert(cc, std::time::Instant::now());
                continue;
            }

            if !bindings.continuous.contains_key(&cc) {
                // Unassigned: still remember it, so the controller picture can show it.
                if matches!(cc, 0..=7 | 16..=23) {
                    self.note_position(cc, value);
                } else {
                    self.pressed.insert(cc, std::time::Instant::now());
                }
                continue;
            }
            let Some(id) = bindings.continuous.get(&cc).and_then(|key| crate::params::DEFS.iter().find(|d| d.key == key)).map(|d| d.id)
            else {
                continue;
            };
            let norm = self.note_position(cc, value);
            let setting = def_of(id).to_norm(params.target(id));
            let previous = self.previous.insert(cc, norm);
            let picked = self.picked_up.entry(cc).or_insert(false);
            if !*picked {
                let crossed = previous.is_some_and(|p| (p - setting) * (norm - setting) <= 0.0);
                *picked = crossed || (norm - setting).abs() < 0.03;
            }
            if *picked {
                params.set_target_norm(id, norm);
            }
        }
    }
}
