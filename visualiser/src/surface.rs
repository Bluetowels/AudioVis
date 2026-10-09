//! A picture of the Korg nanoKONTROL2 drawn over the bottom of the
//! visualisation: every knob, fader and button labelled with what it does,
//! moving with the hardware. Right-click a control on it to reassign it.

use crate::midi::{Action, Bindings, Controller};
use crate::params::{DEFS, P, Params, Toggle};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

/// Transport block, as laid out on the unit: (control number, legend, column, row).
const TRANSPORT: [(u8, &str, f32, f32); 11] = [
    (58, "<", 0.0, 0.0),
    (59, ">", 1.0, 0.0),
    (46, "CYC", 0.0, 1.0),
    (60, "SET", 2.0, 1.0),
    (61, "<", 3.0, 1.0),
    (62, ">", 4.0, 1.0),
    (43, "<<", 0.0, 2.0),
    (44, ">>", 1.0, 2.0),
    (42, "STOP", 2.0, 2.0),
    (41, "PLAY", 3.0, 2.0),
    (45, "REC", 4.0, 2.0),
];

/// A name short enough to sit beside a control.
fn short(id: P) -> &'static str {
    match id {
        P::Reference => "Reference",
        P::AutoGainSpeed => "Gain speed",
        P::Contrast => "Contrast",
        P::Brightness => "Brightness",
        P::BassAmount => "Bass amount",
        P::Decay => "Decay",
        P::Combine => "Combine",
        P::FreqLow => "Lowest freq",
        P::FreqHigh => "Highest freq",
        P::Smoothing => "Smoothing",
        P::Angular => "Angular",
        P::Banding => "Banding",
        P::BassSharpen => "Bass sharpen",
        P::Sharpen => "Sharpen",
        P::Detail => "Speed/detail",
        P::StereoEmphasis => "Stereo emph.",
        P::SurroundAmount => "Surround",
        P::ReliefHeight => "3D height",
        P::Orbit => "3D orbit",
        P::Flight => "Flight speed",
        P::FlightDepth => "Flight depth",
        P::LookAhead => "Look ahead",
        other => crate::params::def_of(other).name,
    }
}

/// Whether a button's switch is on, for switches that have a state.
fn lit(action: Action, params: &Params, show_panel: bool, show_surface: bool) -> Option<bool> {
    Some(match action {
        Action::Toggle(Toggle::FlipX) => params.flip_x,
        Action::Toggle(Toggle::FlipY) => params.flip_y,
        Action::Toggle(Toggle::Stereo) => params.stereo,
        Action::Toggle(Toggle::AutoGain) => params.auto_gain,
        Action::Toggle(Toggle::BassBoost) => params.bass_boost,
        Action::Toggle(Toggle::ReversePalette) => params.reverse_palette,
        Action::TogglePanel => show_panel,
        Action::ToggleSurface => show_surface,
        _ => return None,
    })
}

const BODY: Color32 = Color32::from_rgba_premultiplied(14, 14, 16, 225);
const PART: Color32 = Color32::from_rgb(58, 58, 64);
const EDGE: Color32 = Color32::from_rgb(96, 96, 104);
const TEXT: Color32 = Color32::from_rgb(214, 214, 220);
const FAINT: Color32 = Color32::from_rgb(120, 120, 128);
const ON: Color32 = Color32::from_rgb(255, 86, 60);
const SETTING: Color32 = Color32::from_rgb(90, 200, 255);

pub struct Surface<'a> {
    pub params: &'a mut Params,
    pub bindings: &'a mut Bindings,
    pub controller: &'a mut Controller,
    pub show_panel: bool,
    pub show_surface: bool,
}

impl Surface<'_> {
    /// Draw the controller across the bottom of `picture`. Returns a
    /// description for whichever control the pointer is over.
    pub fn show(&mut self, ui: &mut egui::Ui, picture: Rect) -> Option<(egui::Id, String, Rect)> {
        // The unit is about four times as wide as it is deep.
        let width = (picture.width() * 0.94).min(1180.0);
        let height = width * 0.25;
        let body = Rect::from_min_size(pos2(picture.center().x - width / 2.0, picture.bottom() - height - 14.0), vec2(width, height));
        let painter = ui.painter().clone();
        painter.rect_filled(body, 10.0, BODY);
        painter.rect_stroke(body, 10.0, Stroke::new(1.0, EDGE), StrokeKind::Inside);
        let small = FontId::proportional((width / 105.0).clamp(8.0, 11.5));
        let mut hovered = None;

        let status = match (&self.controller.port, &self.controller.error) {
            (Some(port), _) => format!("{port}: connected"),
            (None, Some(e)) => format!("Controller not connected ({e})"),
            _ => "Controller not connected".to_string(),
        };
        painter.text(body.left_top() + vec2(12.0, 8.0), Align2::LEFT_TOP, status, small.clone(), FAINT);
        painter.text(
            body.right_top() + vec2(-12.0, 8.0),
            Align2::RIGHT_TOP,
            "Right-click a control to reassign it   blue = the setting, white = the hardware",
            small.clone(),
            FAINT,
        );

        // Transport block on the left.
        let block = Rect::from_min_max(body.left_top() + vec2(12.0, 34.0), pos2(body.left() + width * 0.19, body.bottom() - 10.0));
        let cell = vec2(block.width() / 5.0, block.height() / 3.0);
        for (cc, legend, column, row) in TRANSPORT {
            let centre = block.left_top() + vec2((column + 0.5) * cell.x, (row + 0.32) * cell.y);
            let rect = Rect::from_center_size(centre, vec2(cell.x * 0.72, cell.y * 0.36));
            let label_at = centre + vec2(0.0, cell.y * 0.24);
            if let Some(h) = self.button(ui, &painter, cc, rect, legend, label_at, Align2::CENTER_TOP, &small) {
                hovered = Some(h);
            }
        }

        // Eight channel strips.
        let strips = Rect::from_min_max(pos2(block.right() + 10.0, body.top() + 26.0), body.right_bottom() - vec2(10.0, 8.0));
        let strip_width = strips.width() / 8.0;
        for i in 0..8u8 {
            let strip = Rect::from_min_size(strips.left_top() + vec2(i as f32 * strip_width, 0.0), vec2(strip_width, strips.height()));
            if i > 0 {
                painter.vline(strip.left(), strip.y_range(), Stroke::new(1.0, PART));
            }

            // Knob at the top, with its label beside it.
            let radius = (strip.height() * 0.085).min(strip_width * 0.13);
            let knob = pos2(strip.left() + 8.0 + radius, strip.top() + 6.0 + radius);
            let cc = 16 + i;
            let area = Rect::from_center_size(knob, vec2(radius * 2.4, radius * 2.4));
            let (setting, hardware, id) = self.continuous(cc);
            painter.circle_filled(knob, radius, PART);
            painter.circle_stroke(knob, radius, Stroke::new(1.0, EDGE));
            // A knob turns through 300 degrees, from seven o'clock round to five.
            let dial = |norm: f32, from: f32, to: f32, stroke: Stroke| {
                let angle = (-150.0 + 300.0 * norm).to_radians();
                let dir = vec2(angle.sin(), -angle.cos());
                painter.line_segment([knob + dir * radius * from, knob + dir * radius * to], stroke);
            };
            if let Some(s) = setting {
                dial(s, 1.05, 1.45, Stroke::new(2.0, SETTING));
            }
            if let Some(h) = hardware {
                dial(h, 0.15, 0.95, Stroke::new(2.0, TEXT));
            }
            let label = id.map(short).unwrap_or("unassigned");
            painter.text(knob + vec2(radius * 1.7, 0.0), Align2::LEFT_CENTER, label, small.clone(), if id.is_some() { TEXT } else { FAINT });
            if let Some(h) = self.continuous_menu(ui, cc, area.union(Rect::from_min_size(area.right_top(), vec2(strip_width * 0.55, area.height()))), "Knob", i) {
                hovered = Some(h);
            }

            // S, M and R buttons down the left, labels beside them.
            let top = strip.top() + radius * 2.0 + 16.0;
            let rows = (strip.bottom() - top - small.size - 8.0) / 3.0;
            let size = vec2((strip_width * 0.17).min(22.0), (rows * 0.62).min(18.0));
            for (row, (base, letter)) in [(32u8, "S"), (48, "M"), (64, "R")].into_iter().enumerate() {
                let rect = Rect::from_min_size(pos2(strip.left() + 8.0, top + row as f32 * rows + (rows - size.y) / 2.0), size);
                let label_at = rect.right_center() + vec2(5.0, 0.0);
                if let Some(h) = self.button(ui, &painter, base + i, rect, letter, label_at, Align2::LEFT_CENTER, &small) {
                    hovered = Some(h);
                }
            }

            // Fader on the right, its label along the bottom of the strip.
            let cc = i;
            let slot = Rect::from_min_max(pos2(strip.right() - 18.0, top + 2.0), pos2(strip.right() - 14.0, strip.bottom() - small.size - 12.0));
            let (setting, hardware, id) = self.continuous(cc);
            painter.rect_filled(slot, 2.0, PART);
            let y = |norm: f32| slot.bottom() + (slot.top() - slot.bottom()) * norm;
            if let Some(s) = setting {
                painter.hline(slot.left() - 9.0..=slot.right() + 9.0, y(s), Stroke::new(2.0, SETTING));
            }
            if let Some(h) = hardware {
                let cap = Rect::from_center_size(pos2(slot.center().x, y(h)), vec2(16.0, 8.0));
                painter.rect_filled(cap, 2.0, TEXT);
                painter.hline(cap.x_range(), cap.center().y, Stroke::new(1.0, BODY));
            }
            let label = id.map(short).unwrap_or("unassigned");
            painter.text(strip.left_bottom() + vec2(8.0, -4.0), Align2::LEFT_BOTTOM, label, small.clone(), if id.is_some() { TEXT } else { FAINT });
            let area = Rect::from_min_max(pos2(slot.left() - 12.0, slot.top()), pos2(slot.right() + 12.0, strip.bottom()));
            let label_area = Rect::from_min_max(pos2(strip.left(), strip.bottom() - small.size - 8.0), strip.right_bottom());
            if let Some(h) = self.continuous_menu(ui, cc, area.union(label_area), "Fader", i) {
                hovered = Some(h);
            }
        }
        // The description goes just above the unit, over the control, kept on screen.
        // (`show_hint` draws it 348 points to the left of the rectangle it is given.)
        hovered.map(|(id, text, area)| {
            let left = area.left().min(picture.right() - 340.0).max(picture.left() + 8.0);
            (id, text, Rect::from_min_size(pos2(left + 348.0, body.top() - 86.0), area.size()))
        })
    }

    /// For a knob or fader: where its setting is, where the hardware is, and which setting it drives.
    fn continuous(&self, cc: u8) -> (Option<f32>, Option<f32>, Option<P>) {
        let id = self.bindings.continuous.get(&cc).and_then(|key| DEFS.iter().find(|d| d.key == key)).map(|d| d.id);
        let setting = id.map(|id| crate::params::def_of(id).to_norm(self.params.target(id)));
        (setting, self.controller.position(cc), id)
    }

    fn continuous_menu(&mut self, ui: &mut egui::Ui, cc: u8, area: Rect, kind: &str, index: u8) -> Option<(egui::Id, String, Rect)> {
        let response = ui.interact(area, ui.id().with(("surface", cc)), Sense::click());
        let assigned = self.bindings.continuous.get(&cc).cloned();
        response.context_menu(|ui| {
            ui.strong(format!("{kind} {}", index + 1));
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                for d in &DEFS {
                    if ui.radio(assigned.as_deref() == Some(d.key), d.name).clicked() {
                        self.bindings.bind(cc, d.id);
                        self.controller.release(self.bindings, d.id);
                        ui.close();
                    }
                }
            });
            ui.separator();
            if ui.button("Unassign").clicked() {
                self.bindings.continuous.remove(&cc);
                ui.close();
            }
        });
        let d = assigned.and_then(|key| DEFS.iter().find(|d| d.key == key));
        response.hovered().then(|| {
            let text = match d {
                Some(d) => format!("{kind} {}: {}. {} Right-click to assign a different setting.", index + 1, d.name, d.help),
                None => format!("{kind} {}: not assigned. Right-click to choose a setting for it.", index + 1),
            };
            (response.id, text, area)
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn button(
        &mut self,
        ui: &mut egui::Ui,
        painter: &egui::Painter,
        cc: u8,
        rect: Rect,
        legend: &str,
        label_at: Pos2,
        align: Align2,
        font: &FontId,
    ) -> Option<(egui::Id, String, Rect)> {
        let action = self.bindings.buttons.get(&cc).copied();
        let state = action.and_then(|a| lit(a, self.params, self.show_panel, self.show_surface));
        let pressed = self.controller.pressed_recently(cc);
        let fill = if state == Some(true) || pressed { ON } else { PART };
        painter.rect_filled(rect, 3.0, fill);
        painter.rect_stroke(rect, 3.0, Stroke::new(1.0, EDGE), StrokeKind::Inside);
        let legend_font = FontId::proportional((rect.height() * 0.62).min(font.size));
        painter.text(rect.center(), Align2::CENTER_CENTER, legend, legend_font, if fill == ON { Color32::BLACK } else { FAINT });
        if let Some(a) = action {
            painter.text(label_at, align, a.label(), font.clone(), TEXT);
        }

        let area = rect.expand(3.0);
        let response = ui.interact(area, ui.id().with(("surface", cc)), Sense::click());
        response.context_menu(|ui| {
            ui.strong(format!("Button (control {cc})"));
            for a in Action::ALL {
                if ui.radio(action == Some(a), a.label()).clicked() {
                    self.bindings.continuous.remove(&cc);
                    self.bindings.buttons.insert(cc, a);
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("Unassign").clicked() {
                self.bindings.buttons.remove(&cc);
                ui.close();
            }
        });
        response.hovered().then(|| {
            let text = match action {
                Some(a) => format!("{}. Right-click to give this button a different job.", a.help()),
                None => "Not assigned. Right-click to give this button a job.".to_string(),
            };
            (response.id, text, area)
        })
    }
}
