//! The parameters as an agent reads and sets them: each by its id (as sessions and presets
//! save it), its label as the panel prints it, and its value in the units the panel and the
//! host print it in (a knob's dial, percent, dB, semitones, cents; a selector's positions by
//! name; a switch on or off), each range checked rather than clamped, so that an agent learns
//! that it asked for what the panel cannot do.

use ca72::freq_dial::Pot;
use nih_plug::prelude::*;
use serde_json::{Value, json};

use super::ApiError;
use crate::engine::DOUBLE_CENTS;
use crate::params::{self, Ca72Params};

/// What a parameter's value is, to an agent.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// A number from `min` to `max` in `unit` (`step` apart, for VOICES).
    Number {
        unit: &'static str,
        min: f64,
        max: f64,
        step: Option<f64>,
    },
    /// One of these positions, by name (or by index from 0).
    Choice(Vec<&'static str>),
    /// On or off.
    Switch,
}

/// How a number is read: the parameter's own value, a FREQUENCY pot's position as the
/// oscillator's semitones from its centre, DETUNE's share as cents, or an LFO's FADE in
/// seconds, signed (in above 0, out below, off at 0: decisions.md R-LFO-FADE).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Units {
    Plain,
    Semitones(Pot),
    Cents,
    Fade,
}

/// A parameter as the agent operates it, whatever its type.
pub trait Agentic {
    fn normalized(&self) -> f32;
    fn default_normalized(&self) -> f32;
    fn text(&self, normalized: f32) -> String;
    fn plain_of(&self, normalized: f32) -> f64;
    fn normalized_of(&self, plain: f64) -> f32;
    fn host_name(&self) -> &str;
    fn unit_text(&self) -> &'static str;
    /// The choices' names and ids, for a selector.
    fn choices(&self) -> Option<(&'static [&'static str], Option<&'static [&'static str]>)> {
        None
    }
    fn is_switch(&self) -> bool {
        false
    }
    fn is_integer(&self) -> bool {
        false
    }
    /// One change, a gesture of its own (as the panel's click or typed value is).
    fn set_once(&self, s: &ParamSetter<'_>, normalized: f32);
}

macro_rules! common {
    () => {
        fn normalized(&self) -> f32 {
            self.unmodulated_normalized_value()
        }
        fn default_normalized(&self) -> f32 {
            self.default_normalized_value()
        }
        fn text(&self, normalized: f32) -> String {
            self.normalized_value_to_string(normalized, true)
        }
        fn host_name(&self) -> &str {
            self.name()
        }
        fn unit_text(&self) -> &'static str {
            self.unit()
        }
        fn set_once(&self, s: &ParamSetter<'_>, normalized: f32) {
            s.begin_set_parameter(self);
            s.set_parameter_normalized(self, normalized);
            s.end_set_parameter(self);
        }
    };
}

impl Agentic for FloatParam {
    common!();
    fn plain_of(&self, normalized: f32) -> f64 {
        f64::from(self.preview_plain(normalized))
    }
    fn normalized_of(&self, plain: f64) -> f32 {
        self.preview_normalized(plain as f32)
    }
}

impl Agentic for IntParam {
    common!();
    fn plain_of(&self, normalized: f32) -> f64 {
        f64::from(self.preview_plain(normalized))
    }
    fn normalized_of(&self, plain: f64) -> f32 {
        self.preview_normalized(plain.round() as i32)
    }
    fn is_integer(&self) -> bool {
        true
    }
}

impl Agentic for BoolParam {
    common!();
    fn plain_of(&self, normalized: f32) -> f64 {
        f64::from(u8::from(self.preview_plain(normalized)))
    }
    fn normalized_of(&self, plain: f64) -> f32 {
        self.preview_normalized(plain >= 0.5)
    }
    fn is_switch(&self) -> bool {
        true
    }
}

impl<T: Enum + PartialEq + 'static> Agentic for EnumParam<T> {
    common!();
    fn plain_of(&self, normalized: f32) -> f64 {
        self.preview_plain(normalized).to_index() as f64
    }
    fn normalized_of(&self, plain: f64) -> f32 {
        let last = T::variants().len().saturating_sub(1);
        self.preview_normalized(T::from_index((plain.max(0.0) as usize).min(last)))
    }
    fn choices(&self) -> Option<(&'static [&'static str], Option<&'static [&'static str]>)> {
        Some((T::variants(), T::ids()))
    }
}

/// One parameter: its id, the panel's label, where it is, and the parameter.
pub struct Entry<'a> {
    pub id: &'static str,
    pub label: &'static str,
    pub section: &'static str,
    pub param: &'a dyn Agentic,
    units: Units,
}

impl std::fmt::Debug for Entry<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Entry").field("id", &self.id).finish()
    }
}

/// Every parameter, in the panel's order (its sections left to right), then the left hand's,
/// the strip's, the arpeggiator's drawer's, and those the editor has no control for.
pub fn table(p: &Ca72Params) -> Vec<Entry<'_>> {
    use Units::*;
    fn e<'a>(
        id: &'static str,
        label: &'static str,
        section: &'static str,
        param: &'a dyn Agentic,
        units: Units,
    ) -> Entry<'a> {
        Entry {
            id,
            label,
            section,
            param,
            units,
        }
    }
    const C: &str = "CONTROLLERS";
    const O: &str = "OSCILLATOR BANK";
    const M: &str = "MIXER";
    const F: &str = "MODIFIERS";
    const OUT: &str = "OUTPUT";
    const L: &str = "LEFT HAND CONTROLLER";
    const SV: &str = "STRIP: VOICES";
    const SS: &str = "STRIP: STEREO";
    const SO: &str = "STRIP: OUTPUT";
    const A: &str = "ARPEGGIATOR";
    const H: &str = "NOT ON THE PANEL";
    // (Each note's own pressure and timbre's reach, decisions.md R-MPE: the MPE drawer's.)
    const X: &str = "EXPRESSION";
    let mut all = vec![
        e("bypass", "POWER", OUT, &p.bypass, Plain),
        e("tune", "TUNE", C, &p.tune, Plain),
        e("osc_mod", "OSCILLATOR MODULATION", C, &p.osc_mod, Plain),
        e("glide", "GLIDE", C, &p.glide, Plain),
        e("glide_range", "GLIDE RANGE", C, &p.glide_range, Plain),
        e("mod_mix", "MODULATION MIX", C, &p.mod_mix, Plain),
        e("osc1_range", "OSCILLATOR-1 RANGE", O, &p.osc1_range, Plain),
        e("osc2_range", "OSCILLATOR-2 RANGE", O, &p.osc2_range, Plain),
        e("osc3_range", "OSCILLATOR-3 RANGE", O, &p.osc3_range, Plain),
        e(
            "osc2_frequency",
            "OSCILLATOR-2 FREQUENCY",
            O,
            &p.osc2_frequency,
            Semitones(Pot::Two),
        ),
        e(
            "osc3_frequency",
            "OSCILLATOR-3 FREQUENCY",
            O,
            &p.osc3_frequency,
            Semitones(Pot::Three),
        ),
        e(
            "osc1_waveform",
            "OSCILLATOR-1 WAVEFORM",
            O,
            &p.osc1_waveform,
            Plain,
        ),
        e(
            "osc2_waveform",
            "OSCILLATOR-2 WAVEFORM",
            O,
            &p.osc2_waveform,
            Plain,
        ),
        e(
            "osc3_waveform",
            "OSCILLATOR-3 WAVEFORM",
            O,
            &p.osc3_waveform,
            Plain,
        ),
        e("osc3_control", "OSC. 3 CONTROL", O, &p.osc3_control, Plain),
        // (The oscillators timed, R-OSC-TIMED: host parameters until their keys are on the
        // panel.)
        e("osc2_timed", "OSC. 2 TIMED", H, &p.osc2_timed, Plain),
        e(
            "osc2_division",
            "OSC. 2 DIVISION",
            H,
            &p.osc2_division,
            Plain,
        ),
        e("osc3_timed", "OSC. 3 TIMED", H, &p.osc3_timed, Plain),
        e(
            "osc3_division",
            "OSC. 3 DIVISION",
            H,
            &p.osc3_division,
            Plain,
        ),
        e(
            "osc1_shape_mode",
            "OSCILLATOR-1 WAVEFORM MODE",
            O,
            &p.osc1_shape_mode,
            Plain,
        ),
        e("osc1_shape", "OSCILLATOR-1 SHAPE", O, &p.osc1_shape, Plain),
        e(
            "osc1_shape_mod",
            "OSCILLATOR-1 MOD",
            O,
            &p.osc1_shape_mod,
            Plain,
        ),
        e(
            "osc1_shape_mod_dir",
            "OSCILLATOR-1 MOD DIRECTION",
            O,
            &p.osc1_shape_mod_dir,
            Plain,
        ),
        e(
            "osc2_shape_mode",
            "OSCILLATOR-2 WAVEFORM MODE",
            O,
            &p.osc2_shape_mode,
            Plain,
        ),
        e("osc2_shape", "OSCILLATOR-2 SHAPE", O, &p.osc2_shape, Plain),
        e(
            "osc2_shape_mod",
            "OSCILLATOR-2 MOD",
            O,
            &p.osc2_shape_mod,
            Plain,
        ),
        e(
            "osc2_shape_mod_dir",
            "OSCILLATOR-2 MOD DIRECTION",
            O,
            &p.osc2_shape_mod_dir,
            Plain,
        ),
        e(
            "osc3_shape_mode",
            "OSCILLATOR-3 WAVEFORM MODE",
            O,
            &p.osc3_shape_mode,
            Plain,
        ),
        e("osc3_shape", "OSCILLATOR-3 SHAPE", O, &p.osc3_shape, Plain),
        e(
            "osc3_shape_mod",
            "OSCILLATOR-3 MOD",
            O,
            &p.osc3_shape_mod,
            Plain,
        ),
        e(
            "osc3_shape_mod_dir",
            "OSCILLATOR-3 MOD DIRECTION",
            O,
            &p.osc3_shape_mod_dir,
            Plain,
        ),
        e(
            "osc1_volume",
            "OSCILLATOR-1 VOLUME",
            M,
            &p.osc1_volume,
            Plain,
        ),
        e("osc1_on", "OSCILLATOR-1 ON", M, &p.osc1_on, Plain),
        e(
            "ext_volume",
            "EXTERNAL INPUT VOLUME",
            M,
            &p.ext_volume,
            Plain,
        ),
        e("ext_on", "EXTERNAL INPUT ON", M, &p.ext_on, Plain),
        e(
            "osc2_volume",
            "OSCILLATOR-2 VOLUME",
            M,
            &p.osc2_volume,
            Plain,
        ),
        e("osc2_on", "OSCILLATOR-2 ON", M, &p.osc2_on, Plain),
        e("noise_volume", "NOISE VOLUME", M, &p.noise_volume, Plain),
        e("noise_on", "NOISE ON", M, &p.noise_on, Plain),
        e(
            "osc3_volume",
            "OSCILLATOR-3 VOLUME",
            M,
            &p.osc3_volume,
            Plain,
        ),
        e("osc3_on", "OSCILLATOR-3 ON", M, &p.osc3_on, Plain),
        e(
            "noise_type",
            "NOISE (WHITE or PINK)",
            M,
            &p.noise_type,
            Plain,
        ),
        e(
            "filter_mode",
            "FILTER MODE (LO or HI)",
            F,
            &p.filter_mode,
            Plain,
        ),
        e(
            "filter_shape",
            "FILTER SHAPE (LO, BAND, STEEP HIGH or NOTCH)",
            F,
            &p.filter_shape,
            Plain,
        ),
        e("filter_poles", "FILTER POLES", F, &p.filter_poles, Plain),
        e("filter_mod", "FILTER MODULATION", F, &p.filter_mod, Plain),
        e(
            "keyboard_control_1",
            "KEYBOARD CONTROL 1",
            F,
            &p.keyboard_control_1,
            Plain,
        ),
        e(
            "keyboard_control_2",
            "KEYBOARD CONTROL 2",
            F,
            &p.keyboard_control_2,
            Plain,
        ),
        e("cutoff", "CUTOFF FREQUENCY", F, &p.cutoff, Plain),
        e("emphasis", "EMPHASIS", F, &p.emphasis, Plain),
        e(
            "contour_amount",
            "AMOUNT OF CONTOUR",
            F,
            &p.contour_amount,
            Plain,
        ),
        e(
            "filter_attack",
            "FILTER CONTOUR ATTACK TIME",
            F,
            &p.filter_attack,
            Plain,
        ),
        e(
            "filter_decay",
            "FILTER CONTOUR DECAY TIME",
            F,
            &p.filter_decay,
            Plain,
        ),
        e(
            "filter_sustain",
            "FILTER CONTOUR SUSTAIN LEVEL",
            F,
            &p.filter_sustain,
            Plain,
        ),
        e(
            "loudness_attack",
            "LOUDNESS CONTOUR ATTACK TIME",
            F,
            &p.loudness_attack,
            Plain,
        ),
        e(
            "loudness_decay",
            "LOUDNESS CONTOUR DECAY TIME",
            F,
            &p.loudness_decay,
            Plain,
        ),
        e(
            "loudness_sustain",
            "LOUDNESS CONTOUR SUSTAIN LEVEL",
            F,
            &p.loudness_sustain,
            Plain,
        ),
        e("volume", "MAIN OUTPUT VOLUME", OUT, &p.volume, Plain),
        e("main_output", "MAIN OUTPUT", OUT, &p.main_output, Plain),
        e("a440", "A-440", OUT, &p.a440, Plain),
        e("feedback", "FEEDBACK", OUT, &p.feedback, Plain),
        e("quality", "QUALITY", OUT, &p.quality, Plain),
        e("glide_on", "GLIDE (switch)", L, &p.glide_on, Plain),
        e("decay_on", "DECAY (switch)", L, &p.decay_on, Plain),
        e("note_priority", "NOTE PRIORITY", L, &p.note_priority, Plain),
        e("retrigger", "RETRIGGER", L, &p.retrigger, Plain),
        e("pitch_wheel", "PITCH (wheel)", L, &p.pitch_wheel, Plain),
        e("mod_wheel", "MODULATION (wheel)", L, &p.mod_wheel, Plain),
        e("poly", "POLY", SV, &p.poly, Plain),
        e("unison", "UNISON", SV, &p.unison, Plain),
        e("voices", "VOICES", SV, &p.voices, Plain),
        e("entropy", "ENTROPY", SV, &p.entropy, Plain),
        e(
            "doubled",
            "DOUBLE (VOICES PLAYED AS)",
            SS,
            &p.doubled,
            Plain,
        ),
        e("placement", "PLACEMENT", SS, &p.placement, Plain),
        e("spread", "WIDTH", SS, &p.spread, Plain),
        e("inner", "INNER", SS, &p.inner, Plain),
        e("double", "DETUNE (DOUBLE)", SS, &p.double, Cents),
        e("drive", "DRIVE", SO, &p.drive, Plain),
        e("auto_gain", "AUTO GAIN", SO, &p.auto_gain, Plain),
        e("level", "LEVEL", SO, &p.level, Plain),
        e("arp_on", "ARP ON", A, &p.arp_on, Plain),
        e("arp_latch", "ARP HOLD", A, &p.arp_latch, Plain),
        e("arp_direction", "ARP DIRECTION", A, &p.arp_direction, Plain),
        e("arp_rate", "ARP RATE", A, &p.arp_rate, Plain),
        e("arp_octaves", "ARP OCTAVES", A, &p.arp_octaves, Plain),
        e("arp_gate", "ARP GATE", A, &p.arp_gate, Plain),
        e("arp_swing", "ARP SWING", A, &p.arp_swing, Plain),
        e(
            "pressure_mod",
            "PRESSURE TO MODULATION",
            X,
            &p.pressure_mod,
            Plain,
        ),
        e(
            "pressure_filter",
            "PRESSURE TO FILTER",
            X,
            &p.pressure_filter,
            Plain,
        ),
        e(
            "timbre_mod",
            "TIMBRE TO MODULATION",
            X,
            &p.timbre_mod,
            Plain,
        ),
        e(
            "timbre_filter",
            "TIMBRE TO FILTER",
            X,
            &p.timbre_filter,
            Plain,
        ),
        e(
            "velocity_curve",
            "VELOCITY CURVE (-100 SOFT, 100 HARD)",
            X,
            &p.velocity_curve,
            Plain,
        ),
        e(
            "velocity_floor",
            "VELOCITY FLOOR",
            X,
            &p.velocity_floor,
            Plain,
        ),
        e("lock", "LOCK (oscillators identical)", H, &p.lock, Plain),
        e(
            "midi_bend_range",
            "MIDI BEND RANGE",
            H,
            &p.midi_bend_range,
            Plain,
        ),
    ];
    // The LFOs' (decisions.md R-LFO): what they modulate is `modulation.*`'s, not a parameter.
    for n in 0..crate::lfo::LFOS {
        let (q, keys) = (p.lfo(n), p.lfo_keys(n));
        let params: [&dyn Agentic; crate::lfo::PARTS] = [
            &q.waveform,
            &q.shape_mode,
            &q.shape,
            &q.rate,
            &q.timed,
            &q.division,
            &q.phase,
            &q.fade,
            &q.mode,
            &q.mono,
            &keys.polarity,
            &keys.reverse,
            &keys.on,
            &p.lfo_level(n).level,
        ];
        for (k, param) in params.into_iter().enumerate() {
            all.push(e(
                crate::lfo::PARAM_IDS[n][k],
                crate::lfo::PARAM_LABELS[n][k],
                crate::lfo::SECTIONS[n],
                param,
                if k == crate::lfo::FADE_PART {
                    Fade
                } else {
                    Plain
                },
            ));
        }
    }
    // The macros' (decisions.md R-MACROS): their names and what they modulate are
    // `modulation.*`'s.
    for n in 0..crate::lfo::MACROS {
        all.push(e(
            crate::lfo::MACRO_IDS[n],
            crate::lfo::MACRO_LABELS[n],
            "MACROS",
            p.macro_param(n),
            Plain,
        ));
    }
    all
}

/// The parameter of this id, label or host name (any case).
pub fn find<'a>(table: &'a [Entry<'a>], key: &str) -> Option<&'a Entry<'a>> {
    let k = key.trim();
    table.iter().find(|e| e.id == k).or_else(|| {
        table.iter().find(|e| {
            e.label.eq_ignore_ascii_case(k) || e.param.host_name().eq_ignore_ascii_case(k)
        })
    })
}

/// The ids, in the table's order.
pub fn ids(p: &Ca72Params) -> Vec<&'static str> {
    table(p).iter().map(|e| e.id).collect()
}

impl Entry<'_> {
    /// What its value is.
    pub fn shape(&self) -> Shape {
        if self.param.is_switch() {
            return Shape::Switch;
        }
        if let Some((names, _)) = self.param.choices() {
            return Shape::Choice(names.to_vec());
        }
        let (unit, min, max) = match self.units {
            Units::Semitones(pot) => ("semitones", pot.semitones(0.0), pot.semitones(1.0)),
            Units::Cents => ("cents", 0.0, DOUBLE_CENTS),
            Units::Fade => (
                "s",
                -f64::from(crate::lfo::FADE_MOST),
                f64::from(crate::lfo::FADE_MOST),
            ),
            Units::Plain => {
                let (min, max) = (self.param.plain_of(0.0), self.param.plain_of(1.0));
                (self.param.unit_text().trim(), min, max)
            }
        };
        let step = self.param.is_integer().then_some(1.0);
        Shape::Number {
            unit,
            min: round(min),
            max: round(max),
            step,
        }
    }

    /// Its value (normalized) in the shape's terms: a number in its unit, a position's name,
    /// on or off.
    pub fn number_of(&self, normalized: f32) -> f64 {
        match self.units {
            Units::Semitones(pot) => pot.semitones(f64::from(normalized)),
            Units::Cents => self.param.plain_of(normalized) / 100.0 * DOUBLE_CENTS,
            Units::Plain | Units::Fade => self.param.plain_of(normalized),
        }
    }

    fn json_of(&self, normalized: f32) -> Value {
        match self.shape() {
            Shape::Switch => json!(normalized >= 0.5),
            Shape::Choice(names) => {
                let i = self.param.plain_of(normalized) as usize;
                json!(names.get(i).copied().unwrap_or(""))
            }
            Shape::Number { step, .. } => {
                let n = self.number_of(normalized);
                if step.is_some() {
                    json!(n.round() as i64)
                } else {
                    json!(round(n))
                }
            }
        }
    }

    /// As an agent reads it: the value as printed, and as a number, name or switch.
    pub fn read(&self) -> Value {
        let n = self.param.normalized();
        json!({
            "id": self.id,
            "label": self.label,
            "value": self.json_of(n),
            "printed": self.param.text(n),
        })
    }

    /// What it is: its label, section, host name, value, shape and default.
    pub fn describe(&self) -> Value {
        let n = self.param.normalized();
        let d = self.param.default_normalized();
        let mut v = json!({
            "id": self.id,
            "label": self.label,
            "section": self.section,
            "name": self.param.host_name(),
            "value": self.json_of(n),
            "printed": self.param.text(n),
            "default": self.json_of(d),
        });
        match self.shape() {
            Shape::Switch => v["type"] = json!("switch"),
            Shape::Choice(names) => {
                v["type"] = json!("choice");
                v["choices"] = json!(names);
            }
            Shape::Number {
                unit,
                min,
                max,
                step,
            } => {
                v["type"] = json!(if step.is_some() { "integer" } else { "number" });
                v["unit"] = json!(unit);
                v["min"] = json!(min);
                v["max"] = json!(max);
            }
        }
        v
    }

    /// The JSON Schema of a value an agent may give it (`parameters.set`).
    pub fn schema(&self) -> Value {
        match self.shape() {
            Shape::Switch => json!({
                "type": ["boolean", "string"],
                "description": format!("{}: true (on) or false (off)", self.label),
            }),
            Shape::Choice(names) => json!({
                "type": ["string", "integer"],
                "enum": names.iter().map(|n| json!(n)).chain((0..names.len()).map(|i| json!(i))).collect::<Vec<_>>(),
                "description": format!("{}: {}", self.label, names.join(", ")),
            }),
            Shape::Number {
                unit,
                min,
                max,
                step,
            } => {
                let unit_said = if unit.is_empty() {
                    " (its dial's numbers)".to_owned()
                } else {
                    format!(" {unit}")
                };
                json!({
                    "type": [if step.is_some() { "integer" } else { "number" }, "string"],
                    "description": format!("{}: {min} to {max}{unit_said}", self.label),
                })
            }
        }
    }

    /// The normalized value an agent's `v` asks for, or why it cannot be.
    pub fn parse(&self, v: &Value) -> Result<f32, ApiError> {
        let shape = self.shape();
        let bad = |why: String| {
            ApiError::params(format!("{} ({}): {why}", self.label, self.id))
                .with_data(self.describe())
        };
        match (&shape, v) {
            (Shape::Switch, Value::Bool(b)) => {
                Ok(self.param.normalized_of(f64::from(u8::from(*b))))
            }
            (Shape::Switch, Value::Number(n))
                if n.as_f64() == Some(0.0) || n.as_f64() == Some(1.0) =>
            {
                Ok(self.param.normalized_of(n.as_f64().unwrap_or(0.0)))
            }
            (Shape::Switch, Value::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
                "on" | "true" | "yes" | "1" => Ok(self.param.normalized_of(1.0)),
                "off" | "false" | "no" | "0" => Ok(self.param.normalized_of(0.0)),
                _ => Err(bad(format!("`{s}` is neither on nor off"))),
            },
            (Shape::Choice(names), Value::Number(n)) => match n.as_u64() {
                Some(i) if (i as usize) < names.len() => Ok(self.param.normalized_of(i as f64)),
                _ => Err(bad(format!(
                    "position {n} is not one of 0 to {}",
                    names.len() - 1
                ))),
            },
            (Shape::Choice(names), Value::String(s)) => {
                let ids = self.param.choices().and_then(|(_, ids)| ids).unwrap_or(&[]);
                let t = normal_choice(s);
                names
                    .iter()
                    .position(|n| normal_choice(n) == t)
                    .or_else(|| ids.iter().position(|i| normal_choice(i) == t))
                    .map(|i| self.param.normalized_of(i as f64))
                    .ok_or_else(|| bad(format!("`{s}` is not one of {}", names.join(", "))))
            }
            (Shape::Number { .. }, Value::Number(n)) => {
                let x = n.as_f64().ok_or_else(|| bad("not a number".into()))?;
                self.number(x).map_err(bad)
            }
            (Shape::Number { unit, .. }, Value::String(s)) => {
                let x = self.typed(s, unit).ok_or_else(|| {
                    bad(format!(
                        "`{s}` is not a number{}",
                        if unit.is_empty() {
                            String::new()
                        } else {
                            format!(" of {unit}")
                        }
                    ))
                })?;
                self.number(x).map_err(bad)
            }
            _ => Err(bad(format!("{v} is not a value it takes"))),
        }
    }

    /// A number in the shape's unit, within its range.
    fn number(&self, x: f64) -> Result<f32, String> {
        let Shape::Number {
            unit,
            min,
            max,
            step,
        } = self.shape()
        else {
            return Err("not a number".into());
        };
        let tolerance = 5e-3;
        if !x.is_finite() || x < min - tolerance || x > max + tolerance {
            let unit = if unit.is_empty() {
                String::new()
            } else {
                format!(" {unit}")
            };
            return Err(format!("{x} is outside {min} to {max}{unit}"));
        }
        if step.is_some() && (x - x.round()).abs() > 1e-9 {
            return Err(format!("{x} is not a whole number"));
        }
        let x = x.clamp(min.min(max), max.max(min));
        Ok(match self.units {
            Units::Semitones(pot) => {
                let w = pot.wiper(x);
                self.param
                    .normalized_of((2.0 * w - 1.0) * ca72::freq_dial::DIAL_END)
            }
            Units::Cents => self.param.normalized_of(x / DOUBLE_CENTS * 100.0),
            Units::Plain | Units::Fade => self.param.normalized_of(x),
        })
    }

    /// A number typed with its unit or without ("7 st", "+3.4 cents", "-6 dB", "50 %"), in
    /// the shape's unit; DETUNE's "off" its 0.
    fn typed(&self, s: &str, unit: &str) -> Option<f64> {
        let t = s.trim();
        match self.units {
            Units::Semitones(_) => params::semitones_typed(t),
            Units::Cents => {
                if t.eq_ignore_ascii_case("off") {
                    return Some(0.0);
                }
                let t = ["cents", "cent", "\u{a2}", "c"]
                    .iter()
                    .find_map(|u| t.strip_suffix(u))
                    .unwrap_or(t);
                t.trim().trim_start_matches('+').parse().ok()
            }
            // ("Off", "In 1.2", "Out 1.2", "-1.2 s".)
            Units::Fade => crate::lfo::fade_typed(t).map(f64::from),
            Units::Plain => {
                let t = if unit.is_empty() {
                    t
                } else {
                    t.strip_suffix(unit)
                        .or_else(|| {
                            (t.len() >= unit.len()
                                && t[t.len() - unit.len()..].eq_ignore_ascii_case(unit))
                            .then(|| &t[..t.len() - unit.len()])
                        })
                        .unwrap_or(t)
                };
                t.trim().trim_start_matches('+').parse().ok()
            }
        }
    }
}

/// A choice's name compared loosely: case, spaces, underscores and dashes aside, and the
/// footage's mark (`8'` or `8`).
fn normal_choice(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, ' ' | '_' | '-' | '\''))
        .flat_map(char::to_lowercase)
        .collect()
}

/// A number to six significant decimals (no float noise in what an agent reads).
fn round(x: f64) -> f64 {
    (x * 1e6).round() / 1e6
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{DRIVE_TOP, LEVEL_RANGE};
    use ca72::modulation::PITCH_WHEEL_SEMITONES;

    fn entry<'a>(t: &'a [Entry<'a>], id: &str) -> &'a Entry<'a> {
        find(t, id).expect(id)
    }

    /// Every parameter the host sees is in the table once, and nothing else.
    #[test]
    fn the_table_is_every_parameter_once() {
        let p = Ca72Params::default();
        let mut ours = ids(&p);
        let mut hosts: Vec<String> = p.param_map().into_iter().map(|(id, ..)| id).collect();
        ours.sort_unstable();
        hosts.sort_unstable();
        assert_eq!(ours, hosts);
    }

    /// Each parameter's shape is its dial's: the knobs in their numbers, FREQUENCY in
    /// semitones, DETUNE in cents, the strip's in %, dB and voices.
    #[test]
    fn shapes_are_the_panels_units() {
        let p = Ca72Params::default();
        let t = table(&p);
        let num = |id: &str| match entry(&t, id).shape() {
            Shape::Number { unit, min, max, .. } => (unit, min, max),
            s => panic!("{id}: {s:?}"),
        };
        assert_eq!(num("cutoff"), ("", -5.0, 5.0));
        assert_eq!(num("emphasis"), ("", 0.0, 10.0));
        assert_eq!(num("tune"), ("", -2.5, 2.5));
        assert_eq!(num("drive"), ("dB", 0.0, DRIVE_TOP));
        assert_eq!(num("level"), ("dB", LEVEL_RANGE.0, LEVEL_RANGE.1));
        assert_eq!(num("entropy"), ("%", 0.0, 100.0));
        assert_eq!(num("double"), ("cents", 0.0, DOUBLE_CENTS));
        assert_eq!(num("voices"), ("", 2.0, 10.0));
        assert_eq!(num("midi_bend_range").2, round(PITCH_WHEEL_SEMITONES));
        let (unit, lo, hi) = num("osc2_frequency");
        assert_eq!(unit, "semitones");
        assert!(lo < -8.0 && hi > 7.0, "{lo} {hi}");
        assert_eq!(entry(&t, "a440").shape(), Shape::Switch);
        assert_eq!(
            entry(&t, "osc1_range").shape(),
            Shape::Choice(vec!["LO", "32'", "16'", "8'", "4'", "2'"])
        );
    }

    /// Values are taken in the units printed, as numbers or typed with their units, and by a
    /// choice's name, id or index; anything outside the range is refused, not clamped.
    #[test]
    fn values_are_parsed_and_range_checked() {
        let p = Ca72Params::default();
        let t = table(&p);
        let n = |id: &str, v: Value| entry(&t, id).parse(&v);
        let c = entry(&t, "cutoff");
        assert_eq!(n("cutoff", json!(2.5)), Ok(c.param.normalized_of(2.5)));
        assert_eq!(n("cutoff", json!("2.5")), Ok(c.param.normalized_of(2.5)));
        assert!(n("cutoff", json!(6)).is_err());
        let err = n("emphasis", json!(11)).expect_err("out of range");
        assert!(err.message.contains("0 to 10"), "{}", err.message);
        assert_eq!(
            err.data.as_ref().and_then(|d| d.get("max")),
            Some(&json!(10.0))
        );
        let d = entry(&t, "drive");
        assert_eq!(n("drive", json!("6 dB")), Ok(d.param.normalized_of(6.0)));
        assert_eq!(n("drive", json!("6dB")), Ok(d.param.normalized_of(6.0)));
        assert!(n("level", json!(-31)).is_err());
        assert!(n("voices", json!(4.5)).is_err());
        assert!(n("voices", json!(11)).is_err());
        let e = entry(&t, "entropy");
        assert_eq!(n("entropy", json!("50 %")), Ok(e.param.normalized_of(50.0)));
        // FREQUENCY in semitones (or cents), DETUNE in cents.
        let f = entry(&t, "osc2_frequency");
        let seven = n("osc2_frequency", json!(7)).expect("7 st");
        assert!((f.number_of(seven) - 7.0).abs() < 1e-4);
        let cents = n("osc2_frequency", json!("+12 cents")).expect("cents");
        assert!((f.number_of(cents) - 0.12).abs() < 1e-4);
        assert!(n("osc2_frequency", json!(20)).is_err());
        let dt = entry(&t, "double");
        let x = n("double", json!(25)).expect("25 cents");
        assert!((dt.number_of(x) - 25.0).abs() < 1e-4);
        assert_eq!(n("double", json!("off")), Ok(dt.param.normalized_of(0.0)));
        // Choices by name, id or index; switches by booleans or words.
        let r = entry(&t, "osc1_range");
        assert_eq!(
            n("osc1_range", json!("16'")),
            Ok(r.param.normalized_of(2.0))
        );
        assert_eq!(n("osc1_range", json!("16")), Ok(r.param.normalized_of(2.0)));
        assert_eq!(n("osc1_range", json!(0)), Ok(r.param.normalized_of(0.0)));
        assert!(n("osc1_range", json!(6)).is_err());
        let w = entry(&t, "osc1_waveform");
        assert_eq!(
            n("osc1_waveform", json!("shark tooth")),
            Ok(w.param.normalized_of(1.0))
        );
        assert_eq!(
            n("osc1_waveform", json!("narrow_rectangle")),
            Ok(w.param.normalized_of(5.0))
        );
        let err = n("osc1_waveform", json!("sine")).expect_err("no sine");
        assert!(err.message.contains("Sawtooth"), "{}", err.message);
        assert_eq!(n("a440", json!(true)), Ok(1.0));
        assert_eq!(n("a440", json!("OFF")), Ok(0.0));
        assert!(n("a440", json!(0.5)).is_err());
        assert!(n("cutoff", json!(null)).is_err());
    }

    /// A parameter is found by its id, its panel label or its host name, and read as printed.
    #[test]
    fn found_by_id_label_or_name_and_read_as_printed() {
        let p = Ca72Params::default();
        let t = table(&p);
        assert_eq!(find(&t, "WIDTH").map(|e| e.id), Some("spread"));
        assert_eq!(find(&t, "cutoff frequency").map(|e| e.id), Some("cutoff"));
        assert_eq!(
            find(&t, "Osc 2 Frequency").map(|e| e.id),
            Some("osc2_frequency")
        );
        assert!(find(&t, "nothing").is_none());
        let r = entry(&t, "osc1_volume").read();
        assert_eq!(r["value"], json!(8.0));
        assert_eq!(r["printed"], json!("8.00"));
        let f = entry(&t, "osc1_range").read();
        assert_eq!(f["value"], json!("8'"));
        let s = entry(&t, "osc1_on").read();
        assert_eq!(s["value"], json!(true));
        let d = entry(&t, "double").describe();
        assert_eq!(
            (d["type"].clone(), d["unit"].clone()),
            (json!("number"), json!("cents"))
        );
        assert_eq!(entry(&t, "voices").describe()["type"], json!("integer"));
    }
}
