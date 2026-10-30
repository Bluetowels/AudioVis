//! Preset cycle: runs through the saved presets by itself, holding each for
//! a while and then morphing every setting smoothly into the next.

use crate::params::Params;
use serde::{Deserialize, Serialize};

/// The choices in the panel, kept with the app's settings rather than in presets.
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct CycleSettings {
    pub on: bool,
    /// Pick the next preset at random instead of going down the list.
    pub random: bool,
    /// Seconds each preset is held before the morph to the next begins.
    pub hold: f32,
    /// Seconds the morph from one preset to the next takes.
    pub morph: f32,
}

impl Default for CycleSettings {
    fn default() -> Self {
        Self { on: false, random: false, hold: 20.0, morph: 10.0 }
    }
}

/// A morph under way.
pub struct Morph {
    pub name: String,
    pub from: Params,
    pub to: Params,
}

pub struct Cycle {
    pub settings: CycleSettings,
    /// Seconds into the hold, or into the morph while there is one.
    pub clock: f32,
    pub morph: Option<Morph>,
    rng: u64,
}

impl Cycle {
    pub fn new(settings: CycleSettings) -> Self {
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
        Self { settings, clock: 0.0, morph: None, rng: seed | 1 }
    }

    /// Start the hold again from now, dropping any morph under way.
    pub fn restart(&mut self) {
        self.clock = 0.0;
        self.morph = None;
    }

    /// How far through the morph, 0..1, eased so it starts and ends gently.
    pub fn progress(&self) -> f32 {
        let t = if self.settings.morph < 0.05 { 1.0 } else { (self.clock / self.settings.morph).clamp(0.0, 1.0) };
        t * t * (3.0 - 2.0 * t)
    }

    /// The preset to go to after `current`: the next one down the list, or
    /// any other at random. Never `current` itself while there is a choice.
    pub fn next(&mut self, names: &[String], current: &str) -> Option<String> {
        if names.is_empty() {
            return None;
        }
        let at = names.iter().position(|n| n == current);
        if !self.settings.random {
            return Some(names[at.map_or(0, |i| (i + 1) % names.len())].clone());
        }
        let others: Vec<&String> = names.iter().filter(|n| *n != current).collect();
        if others.is_empty() {
            return Some(names[0].clone());
        }
        // xorshift: plenty for picking a preset.
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        Some(others[(self.rng % others.len() as u64) as usize].clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Vec<String> {
        ["A", "B", "C"].map(String::from).to_vec()
    }

    #[test]
    fn sequential_goes_down_the_list_and_round() {
        let mut c = Cycle::new(CycleSettings::default());
        assert_eq!(c.next(&names(), "A").as_deref(), Some("B"));
        assert_eq!(c.next(&names(), "C").as_deref(), Some("A"));
        assert_eq!(c.next(&names(), "unsaved").as_deref(), Some("A"));
    }

    #[test]
    fn random_never_repeats() {
        let mut c = Cycle::new(CycleSettings { random: true, ..Default::default() });
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..200 {
            let n = c.next(&names(), "B").unwrap();
            assert_ne!(n, "B");
            seen.insert(n);
        }
        assert_eq!(seen.len(), 2);
    }
}
