//! Exact values at the knobs, in the panel's printed units (R-KNOB-ENTRY).

use ca72_panel::controls::{Dial, Legends, TIME_MARKS};
use ca72_panel::value_entry::Field;
use nih_plug::prelude::Param;

use super::{CONTROLS, Editing, Key, KeyState, KeyboardEvent, Kind, Modifiers, Readout, art};

#[derive(Debug)]
pub(super) struct Entry {
    control: usize,
    text: String,
    selected: bool,
    edited: bool,
    /// A modulation source's depth on the knob, typed in percent (DEPTH…, decisions.md R-LFO),
    /// rather than the knob's own value.
    depth: Option<(crate::lfo::Source, crate::lfo::Target)>,
}

impl Entry {
    /// Whether it is control `i`'s own value being typed (not a source's depth on it).
    pub(super) fn is_value_of(&self, i: usize) -> bool {
        self.control == i && self.depth.is_none()
    }

    /// What it holds now.
    #[cfg(test)]
    pub(super) fn text(&self) -> &str {
        &self.text
    }
}

pub(super) fn shortcut(modifiers: Modifiers) -> bool {
    modifiers.contains(if cfg!(target_os = "macos") {
        Modifiers::META
    } else {
        Modifiers::CONTROL
    })
}

fn readout(control: usize) -> Option<Readout> {
    Some(match CONTROLS[control].param {
        "voices" => Readout::Voices,
        "entropy" => Readout::Entropy,
        "spread" => Readout::Width,
        "inner" => Readout::Inner,
        "double" => Readout::Detune,
        "drive" => Readout::Drive,
        "level" => Readout::Level,
        _ => return None,
    })
}

// Printed times in ms at their angles (art::marks). Between them the printed scale is
// straight; below the first mark it meets zero at the stop, past the last it continues at
// that slope to the stop. These are dial readings, not measured contour durations.
const TIMES: [(f64, f64); 7] = [
    (-150.0, 0.0),
    TIME_MARKS[0],
    TIME_MARKS[1],
    TIME_MARKS[2],
    TIME_MARKS[3],
    TIME_MARKS[4],
    TIME_MARKS[5],
];

fn time_at(knob: f64) -> f64 {
    let angle = knob.clamp(0.0, 1.0) * 300.0 - 150.0;
    let pair = TIMES
        .windows(2)
        .find(|p| angle <= p[1].0)
        .unwrap_or(&TIMES[5..]);
    let [(a, ta), (b, tb)] = pair else {
        unreachable!()
    };
    ta + (tb - ta) * (angle - a) / (b - a)
}

fn time_knob(ms: f64) -> f64 {
    let pair = TIMES
        .windows(2)
        .find(|p| ms <= p[1].1)
        .unwrap_or(&TIMES[5..]);
    let [(a, ta), (b, tb)] = pair else {
        unreachable!()
    };
    ((a + (b - a) * (ms - ta) / (tb - ta) + 150.0) / 300.0).clamp(0.0, 1.0)
}

const FOOTAGES: [f64; 6] = [0.0, 32.0, 16.0, 8.0, 4.0, 2.0];

/// An LFO's FADE (s, signed) with its knob at `knob` (0..1): RISE's cube-root travel each way
/// from the centre, as its parameter's range has it.
fn fade_at(knob: f64) -> f64 {
    let t = 2.0 * knob.clamp(0.0, 1.0) - 1.0;
    10.0 * t.powi(3)
}

/// Its knob's travel (0..1) at FADE `s` seconds (signed): [`fade_at`]'s inverse.
fn fade_knob(s: f64) -> f64 {
    0.5 + 0.5 * (s / 10.0).clamp(-1.0, 1.0).cbrt()
}

fn unit(control: usize) -> &'static str {
    if let Some(r) = readout(control) {
        return match r {
            Readout::Entropy | Readout::Width | Readout::Inner => "%",
            Readout::Detune => "cents",
            Readout::Drive | Readout::Level => "dB",
            _ => "voices",
        };
    }
    match CONTROLS[control].kind {
        Kind::Knob {
            dial: Dial::Time, ..
        } => "ms",
        Kind::Knob {
            dial: Dial::Tune, ..
        } => "semitones",
        Kind::Knob {
            dial: Dial::Freq, ..
        } => "dial",
        // An LFO's (decisions.md R-LFO).
        Kind::Knob {
            dial: Dial::LfoRate,
            ..
        } => "Hz",
        Kind::Knob {
            dial: Dial::LfoPhase,
            ..
        } => "degrees",
        Kind::Knob {
            dial: Dial::LfoFade,
            ..
        } => "s \u{b7} \u{2013} out \u{b7} + in",
        Kind::Selector(Legends::Ranges) => "ft · 0 = LO",
        Kind::Selector(Legends::Poles) => "poles 1–4",
        Kind::Selector(_) => "position 1–6",
        _ => "dial",
    }
}

fn plain(control: usize, knob: f64) -> f64 {
    match CONTROLS[control].kind {
        Kind::Knob { dial, .. } => match dial {
            Dial::Tune => knob * 5.0 - 2.5,
            Dial::Cutoff => knob * 10.0 - 5.0,
            Dial::Freq => knob * 15.0 - 7.5,
            Dial::Time => time_at(knob),
            // An LFO's, through its parameter's travel (decisions.md R-LFO).
            Dial::LfoRate => 0.01 + 49.99 * knob.powi(5),
            Dial::LfoPhase => knob * 360.0,
            // (FADE signed: out to the left of its centre, in to the right.)
            Dial::LfoFade => fade_at(knob),
            _ => knob * 10.0,
        },
        Kind::Selector(Legends::Ranges) => FOOTAGES[(knob * 5.0).round() as usize],
        // FILTER POLES: its place is its poles (decisions.md R-POLES).
        Kind::Selector(Legends::Poles) => (knob * 5.0).round(),
        Kind::Selector(_) => (knob * 5.0).round() + 1.0,
        _ => unreachable!("only knobs and selectors take values"),
    }
}

fn normalized(control: usize, text: &str) -> Option<f64> {
    let time = matches!(
        CONTROLS[control].kind,
        Kind::Knob {
            dial: Dial::Time,
            ..
        }
    );
    let text = text.trim();
    // An LFO's FADE: "Off", "In 1.2", "Out 1.2", or seconds signed (decisions.md R-LFO-FADE).
    if let Kind::Knob {
        dial: Dial::LfoFade,
        ..
    } = CONTROLS[control].kind
    {
        let s = crate::lfo::fade_typed(text)?;
        return Some(fade_knob(f64::from(s)));
    }
    let (text, scale) = if time {
        if let Some(t) = text.strip_suffix("ms") {
            (t.trim(), 1.0)
        } else if let Some(t) = text.strip_suffix('s') {
            (t.trim(), 1000.0)
        } else {
            (text, 1.0)
        }
    } else {
        (text, 1.0)
    };
    let lfo_dial = match CONTROLS[control].kind {
        Kind::Knob { dial, .. } => matches!(dial, Dial::LfoRate | Dial::LfoPhase),
        _ => false,
    };
    let text = if lfo_dial {
        text.trim_end_matches(['s', '\u{b0}'])
            .trim_end_matches("Hz")
            .trim_end_matches("hz")
            .trim()
    } else {
        text
    };
    let v = text.parse::<f64>().ok()?;
    if !v.is_finite() {
        return None;
    }
    Some(
        match CONTROLS[control].kind {
            Kind::Knob { dial, .. } => match dial {
                Dial::Tune => (v + 2.5) / 5.0,
                Dial::Cutoff => (v + 5.0) / 10.0,
                Dial::Freq => (v + 7.5) / 15.0,
                Dial::Time => time_knob(v * scale),
                Dial::LfoRate => ((v - 0.01) / 49.99).clamp(0.0, 1.0).powf(0.2),
                Dial::LfoPhase => v / 360.0,
                _ => v / 10.0,
            },
            Kind::Selector(Legends::Ranges) => {
                let position = if v <= 0.0 {
                    0
                } else {
                    (1..6).min_by(|&a, &b| {
                        (FOOTAGES[a] - v.clamp(2.0, 32.0))
                            .abs()
                            .total_cmp(&(FOOTAGES[b] - v.clamp(2.0, 32.0)).abs())
                    })?
                };
                position as f64 / 5.0
            }
            Kind::Selector(Legends::Poles) => v.round().clamp(1.0, 4.0) / 5.0,
            Kind::Selector(_) => (v.round().clamp(1.0, 6.0) - 1.0) / 5.0,
            _ => return None,
        }
        .clamp(0.0, 1.0),
    )
}

/// A division as the timed dial prints it (decisions.md R-OSC-TIMED), by its place among the
/// eighteen: `4x` and `2x` the bars, `1` a bar, then a note value, `.` dotted and `t` triplet
/// (as the arpeggiator's RATE is typed).
fn note_text(division: usize) -> String {
    match division {
        0 => "4x".into(),
        1 => "2x".into(),
        d => {
            // (From 1 bar on, threes: dotted, triplet, straight; `params::Division`'s order.)
            let (j, r) = ((d - 2) / 3, (d - 2) % 3);
            match r {
                0 => format!("{}", 1u32 << j),
                1 => format!("{}.", 2u32 << j),
                _ => format!("{}t", 1u32 << j),
            }
        }
    }
}

/// The division typed: a note value as the dial prints it (`8`, `8.`, `8t`, `4x`, `2x`), or as
/// the host names it (`1/8`, `1/8 dotted`, `1/8d`, `1/4 triplet`, `4 bars`, `1 bar`); none for
/// anything else (a dotted whole note, a value off the dial).
fn note_index(text: &str) -> Option<usize> {
    let t: String = text
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let t = t.strip_prefix("1/").unwrap_or(&t);
    let bars = |n: &str| match n {
        "4" => Some(0),
        "2" => Some(1),
        "1" => Some(2),
        _ => None,
    };
    for s in ["bars", "bar", "x"] {
        if let Some(n) = t.strip_suffix(s) {
            return bars(n);
        }
    }
    let (n, mark) = if let Some(n) = t.strip_suffix("dotted").or(t.strip_suffix('.')) {
        (n, 0)
    } else if let Some(n) = t.strip_suffix("d") {
        (n, 0)
    } else if let Some(n) = t.strip_suffix("triplet").or(t.strip_suffix('t')) {
        (n, 1)
    } else {
        (t, 2)
    };
    let m = [1, 2, 4, 8, 16, 32]
        .iter()
        .position(|v| n == v.to_string())?;
    let i = match mark {
        // Dotted: a half to a thirty-second.
        0 if m >= 1 => 3 * m,
        // Triplet: a whole to a sixteenth.
        1 if m <= 4 => 4 + 3 * m,
        2 => 2 + 3 * m,
        _ => return None,
    };
    (i <= ca72_panel::controls::TIMED_STEPS).then_some(i)
}

impl Editing {
    pub(super) fn start_knob_entry(&mut self, control: usize) -> bool {
        if !self.operable(control)
            || !matches!(
                CONTROLS[control].kind,
                Kind::Knob { .. } | Kind::Selector(_)
            )
        {
            return false;
        }
        let p = &self.params;
        let value = match readout(control) {
            Some(Readout::Voices) => f64::from(p.voices.unmodulated_plain_value()),
            Some(Readout::Entropy) => f64::from(p.entropy.unmodulated_plain_value()),
            Some(Readout::Width) => f64::from(p.spread.unmodulated_plain_value()),
            Some(Readout::Inner) => f64::from(p.inner.unmodulated_plain_value()),
            Some(Readout::Detune) => {
                f64::from(p.double.unmodulated_plain_value()) * crate::engine::DOUBLE_CENTS / 100.0
            }
            Some(Readout::Drive) => f64::from(p.drive.unmodulated_plain_value()),
            Some(Readout::Level) => f64::from(p.level.unmodulated_plain_value()),
            _ if self.timed_dial(control) => 0.0,
            _ => plain(control, self.value_of(control)),
        };
        let value = if value.abs() < 0.00005 { 0.0 } else { value };
        let text = if self.timed_dial(control) {
            // (A timed FREQUENCY: its division, as the dial prints it.)
            let steps = ca72_panel::controls::TIMED_STEPS as f64;
            note_text((self.value_of(control) * steps).round() as usize)
        } else {
            format!("{value:.4}")
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_owned()
        };
        self.playing.release();
        self.last_press = None;
        self.last_strip_press = None;
        self.knob_entry = Some(Entry {
            control,
            text,
            selected: true,
            edited: false,
            depth: None,
        });
        self.typing_keys = true;
        true
    }

    /// Source `s`'s depth on knob `control` (target `t`) to be typed, in percent of its travel,
    /// in the knob's entry field (DEPTH…, decisions.md R-LFO).
    pub(super) fn start_depth_entry(
        &mut self,
        control: usize,
        s: crate::lfo::Source,
        t: crate::lfo::Target,
    ) {
        let depth = self.params.lfo_patch.depth(s, t).unwrap_or(0.0);
        self.playing.release();
        self.last_press = None;
        self.last_strip_press = None;
        self.knob_entry = Some(Entry {
            control,
            text: format!("{:.0}", f64::from(depth) * 100.0),
            selected: true,
            edited: false,
            depth: Some((s, t)),
        });
        self.typing_keys = true;
    }

    pub(super) fn entry_field(&self, entry: &Entry) -> Field {
        Field {
            bounds: [0.0; 4],
            size: self.renderer.text_size() * 1.15,
            text: entry.text.clone(),
            unit: if entry.depth.is_some() {
                "% of its travel, \u{2013}100 to 100"
            } else if self.timed_dial(entry.control) {
                "note · . dotted · t triplet"
            } else if CONTROLS
                .get(entry.control)
                .is_some_and(|c| c.param.ends_with("_frequency"))
                && self.params.freq_law() == ca72::freq_dial::Law::Semitones
            {
                "semitones"
            } else {
                unit(entry.control)
            },
            selected: entry.selected,
        }
        .placed(
            self.renderer.fonts(),
            // (A depth on the standalone's TEMPO typed over its screen: R-LFO.)
            match super::fx_pins::target(entry.control).and_then(|n| self.fx_pin_at(n)) {
                Some(at) => at,
                None => match CONTROLS.get(entry.control) {
                    Some(c) => c.centre(),
                    None => {
                        let (x, y, w, h) =
                            art::tempo_window(self.renderer.arrangement().plate_at());
                        (x + w / 2.0, y + h / 2.0)
                    }
                },
            },
            [
                0.0,
                0.0,
                art::W,
                f64::from(self.physical_height()) / self.renderer.scale(),
            ],
        )
    }

    pub(super) fn knob_key(&mut self, k: &KeyboardEvent) -> bool {
        if k.state != KeyState::Down {
            return true;
        }
        let Some(entry) = &mut self.knob_entry else {
            return false;
        };
        match &k.key {
            Key::Enter => {
                let entry = self.knob_entry.take().expect("an entry");
                // A source's depth (decisions.md R-LFO): within -100 to 100 %, none at 0.
                if let Some((s, t)) = entry.depth {
                    let v = entry
                        .text
                        .trim()
                        .trim_end_matches('%')
                        .trim()
                        .replace('\u{2013}', "-");
                    if entry.edited
                        && let Ok(v) = v.parse::<f64>()
                        && v.is_finite()
                    {
                        let d = (v / 100.0).clamp(-1.0, 1.0) as f32;
                        if d == 0.0 {
                            self.params.lfo_patch.remove(s, t);
                        } else {
                            let _ = self.params.lfo_patch.assign(s, t, d);
                        }
                    }
                    return true;
                }
                if !entry.edited || !self.operable(entry.control) {
                    return true;
                }
                if let Some(r) = readout(entry.control) {
                    self.set_typed(r, &entry.text);
                } else if self.timed_dial(entry.control) {
                    // A timed FREQUENCY takes a note value, its DIVISION's (R-OSC-TIMED).
                    if let Some(d) = note_index(&entry.text) {
                        let steps = ca72_panel::controls::TIMED_STEPS as f64;
                        self.set_once(entry.control, d as f64 / steps);
                    }
                } else if let Some(v) = normalized(entry.control, &entry.text) {
                    self.set_once(entry.control, v);
                }
            }
            Key::Escape => self.knob_entry = None,
            Key::Character(c) if shortcut(k.modifiers) && c.eq_ignore_ascii_case("a") => {
                entry.selected = true
            }
            Key::Character(c)
                if !k
                    .modifiers
                    .intersects(Modifiers::CONTROL | Modifiers::META | Modifiers::ALT) =>
            {
                entry.edited = true;
                if entry.selected {
                    entry.text.clear();
                    entry.selected = false;
                }
                for ch in c.chars() {
                    if !ch.is_ascii() || ch.is_control() || entry.text.len() >= 32 {
                        entry.text = "Invalid value".to_owned();
                        break;
                    }
                    entry.text.push(ch);
                }
            }
            Key::Backspace | Key::Delete => {
                entry.edited = true;
                if entry.selected {
                    entry.text.clear();
                    entry.selected = false;
                } else {
                    entry.text.pop();
                }
            }
            _ => {}
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn control(id: &str) -> usize {
        ca72_panel::controls::index(id).expect("a control")
    }

    #[test]
    fn panel_dials_parse_their_numbers_and_clamp_at_the_stops() {
        for (id, text, expected) in [
            ("volume", "7.25", 0.725),
            ("mod_mix", "3.5", 0.35),
            ("tune", "-2", 0.1),
            ("cutoff", "4", 0.9),
            ("osc2_frequency", "-5", 1.0 / 6.0),
            ("osc3_frequency", "7.5", 1.0),
            ("filter_attack", "200", 0.2),
            ("filter_decay", "600 ms", 0.4),
            ("loudness_attack", "1 s", 0.6),
            ("loudness_decay", "5000", 217.0 / 300.0),
            ("filter_attack", "10 s", 258.0 / 300.0),
            ("filter_attack", "1e300 s", 1.0),
            ("osc1_range", "32", 0.2),
            ("osc1_range", "8", 0.6),
            ("osc1_range", "0", 0.0),
            ("osc1_range", "999", 0.2),
            ("osc1_range", "1", 1.0),
            ("osc1_waveform", "3.6", 0.6),
            ("osc3_waveform", "999", 1.0),
        ] {
            let got = normalized(control(id), text).expect("a number");
            assert!((got - expected).abs() < 1e-12, "{id} {text}: {got}");
        }
        for id in [
            "volume",
            "mod_mix",
            "tune",
            "cutoff",
            "osc2_frequency",
            "filter_attack",
            "osc3_waveform",
        ] {
            assert_eq!(normalized(control(id), "-1e30"), Some(0.0), "{id}");
            assert_eq!(normalized(control(id), "1e30"), Some(1.0), "{id}");
            for text in ["", "x", "2x", "--1", "1..2", "NaN", "inf", "-inf", "1e999"] {
                assert_eq!(normalized(control(id), text), None, "{id} {text}");
            }
        }
    }

    /// A timed FREQUENCY's typed note values (decisions.md R-OSC-TIMED): each division's text
    /// as the dial prints it comes back as that division, and the host's names do too.
    #[test]
    fn a_timed_dial_takes_its_note_values() {
        for d in 0..=ca72_panel::controls::TIMED_STEPS {
            assert_eq!(note_index(&note_text(d)), Some(d), "{d}: {}", note_text(d));
            let name = crate::timed::DIVISIONS[d].0;
            assert_eq!(note_index(name), Some(d), "{name}");
        }
        assert_eq!(note_text(0), "4x");
        assert_eq!(note_text(2), "1");
        assert_eq!(note_text(9), "8.");
        assert_eq!(note_text(16), "16t");
        assert_eq!(note_text(17), "32");
        for (text, d) in [
            ("4×", None),
            ("4 X", Some(0)),
            ("1/8d", Some(9)),
            ("1/8 dotted", Some(9)),
            (" 8T ", Some(13)),
            ("1 bar", Some(2)),
            ("1x", Some(2)),
        ] {
            assert_eq!(note_index(text), d, "{text}");
        }
        for text in ["", "3", "1.", "32t", "64", "8x", "1/3", "x", "..", "8.t"] {
            assert_eq!(note_index(text), None, "{text}");
        }
    }

    #[test]
    fn the_time_scale_returns_every_printed_mark_and_both_stops() {
        for (angle, ms) in TIMES {
            let knob = (angle + 150.0) / 300.0;
            assert!((time_at(knob) - ms).abs() < 1e-9);
            assert!((time_knob(ms) - knob).abs() < 1e-12);
        }
        for k in 0..=1000 {
            let knob = f64::from(k) / 1000.0;
            assert!((time_knob(time_at(knob)) - knob).abs() < 1e-12);
        }
        assert_eq!(time_knob(-1000.0), 0.0);
        assert_eq!(time_knob(1e20), 1.0);
    }
}
