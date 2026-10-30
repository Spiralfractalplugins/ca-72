//! The plug-in's parameters: the front panel's and left hand controller's controls in their
//! dials' units, MIDI BEND RANGE, and the plug-in's own POLY, VOICES, ENTROPY, SPREAD and INNER.
//! Their ids are stable (a saved session's settings are found by them): never rename one.

use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use ca72::freq_dial::{DIAL_END, Law, Pot};
use ca72::modulation::PITCH_WHEEL_SEMITONES;
use ca72::tuning::Range;
use ca72::voice::{
    ContourKnobs, ModDirection, OscPanel, Panel, Priority, Quality, Shape, Waveform,
};
use nih_plug::prelude::*;

use crate::character::Placement;
use crate::drive::Calibration;
use crate::engine::{Controls, DOUBLE_CENTS, DRIVE_TOP, LEVEL_RANGE, POLY_VOICES, Reach};
use crate::learn::MidiMap;

/// RANGE's positions.
#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Footage {
    #[id = "lo"]
    #[name = "LO"]
    Lo,
    #[id = "32"]
    #[name = "32'"]
    R32,
    #[id = "16"]
    #[name = "16'"]
    R16,
    #[id = "8"]
    #[name = "8'"]
    R8,
    #[id = "4"]
    #[name = "4'"]
    R4,
    #[id = "2"]
    #[name = "2'"]
    R2,
}

impl From<Footage> for Range {
    fn from(f: Footage) -> Range {
        match f {
            Footage::Lo => Range::Lo,
            Footage::R32 => Range::R32,
            Footage::R16 => Range::R16,
            Footage::R8 => Range::R8,
            Footage::R4 => Range::R4,
            Footage::R2 => Range::R2,
        }
    }
}

/// WAVEFORM on oscillators 1 and 2.
#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wave {
    #[id = "triangle"]
    Triangle,
    #[id = "shark_tooth"]
    #[name = "Shark Tooth"]
    SharkTooth,
    #[id = "sawtooth"]
    Sawtooth,
    #[id = "square"]
    Square,
    #[id = "wide_rectangle"]
    #[name = "Wide Rectangle"]
    WideRectangle,
    #[id = "narrow_rectangle"]
    #[name = "Narrow Rectangle"]
    NarrowRectangle,
}

impl From<Wave> for Waveform {
    fn from(w: Wave) -> Waveform {
        match w {
            Wave::Triangle => Waveform::Triangle,
            Wave::SharkTooth => Waveform::SharkTooth,
            Wave::Sawtooth => Waveform::Sawtooth,
            Wave::Square => Waveform::Square,
            Wave::WideRectangle => Waveform::WideRectangle,
            Wave::NarrowRectangle => Waveform::NarrowRectangle,
        }
    }
}

/// WAVEFORM on oscillator 3: its second position the reverse sawtooth.
#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wave3 {
    #[id = "triangle"]
    Triangle,
    #[id = "reverse_sawtooth"]
    #[name = "Reverse Sawtooth"]
    ReverseSawtooth,
    #[id = "sawtooth"]
    Sawtooth,
    #[id = "square"]
    Square,
    #[id = "wide_rectangle"]
    #[name = "Wide Rectangle"]
    WideRectangle,
    #[id = "narrow_rectangle"]
    #[name = "Narrow Rectangle"]
    NarrowRectangle,
}

/// Where SCATTER puts the voices (decisions.md R45, the CA-74's R30): evenly from edge
/// to edge, or out from the centre. The CA-74's third, EDGES, is not offered: INNER moves the
/// voices out from the centre instead, all of them to the edges at 100 % (the owner,
/// 2026-10-10; decisions.md R49). In this order (the strip's tabs'): a preset's value is
/// its index, 0 EVEN and 1 CENTER.
#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scatter {
    #[id = "even"]
    #[name = "EVEN"]
    Even,
    #[id = "centre"]
    #[name = "CENTER"]
    Centre,
}

/// The kit's placement of each choice.
impl From<Scatter> for Placement {
    fn from(s: Scatter) -> Placement {
        match s {
            Scatter::Even => Placement::Even,
            Scatter::Centre => Placement::Centre,
        }
    }
}

impl From<Wave3> for Waveform {
    fn from(w: Wave3) -> Waveform {
        match w {
            Wave3::Triangle => Waveform::Triangle,
            Wave3::ReverseSawtooth => Waveform::ReverseSawtooth,
            Wave3::Sawtooth => Waveform::Sawtooth,
            Wave3::Square => Waveform::Square,
            Wave3::WideRectangle => Waveform::WideRectangle,
            Wave3::NarrowRectangle => Waveform::NarrowRectangle,
        }
    }
}

/// The NOISE switch: white (pink to the modulation) or pink (red to the modulation).
#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoiseColor {
    #[id = "white"]
    White,
    #[id = "pink"]
    Pink,
}

/// An oscillator's waveform (decisions.md R-CONTINUOUS-WAVE): STEPPED, the WAVEFORM switch as
/// the instrument has it; CONTINUOUS, which the original does not have, the switch turned
/// without steps through every shape, each blending into the next, and on to the narrowest
/// pulse, with MOD.
#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaveMode {
    #[id = "stepped"]
    #[name = "Stepped"]
    Stepped,
    #[id = "continuous"]
    #[name = "Continuous"]
    Continuous,
}

/// GLIDE's range (decisions.md R-GLIDE-LONG): the circuit's hold capacitor, or five times it.
#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlideRange {
    #[id = "normal"]
    #[name = "Normal"]
    Normal,
    #[id = "long"]
    #[name = "Long (×5)"]
    Long,
}

/// Which way MOD moves a CONTINUOUS shape from its switch (its key Command-clicked).
#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeModDirection {
    #[id = "clockwise"]
    #[name = "Clockwise"]
    Clockwise,
    #[id = "anticlockwise"]
    #[name = "Anticlockwise"]
    Anticlockwise,
    #[id = "both"]
    #[name = "Both Ways"]
    Both,
}

impl From<ShapeModDirection> for ModDirection {
    fn from(d: ShapeModDirection) -> ModDirection {
        match d {
            ShapeModDirection::Clockwise => ModDirection::Onward,
            ShapeModDirection::Anticlockwise => ModDirection::Back,
            ShapeModDirection::Both => ModDirection::Both,
        }
    }
}

/// The names of oscillator `n`'s (0 to 2) four places before the rectangle's stretch:
/// oscillator 3's second and third its reverse shark tooth and reverse sawtooth in CONTINUOUS.
pub fn shape_names(n: usize) -> [&'static str; 4] {
    if n == 2 {
        [
            "Triangle",
            "Reverse Shark Tooth",
            "Reverse Sawtooth",
            "Square",
        ]
    } else {
        ["Triangle", "Shark Tooth", "Sawtooth", "Square"]
    }
}

/// Oscillator `n`'s CONTINUOUS shape at `t` of its travel, as the host and the panel's tip show
/// it: a place's name ("Sawtooth"); between two, both and how far ("Sawtooth to Square 40 %");
/// past SQUARE the rectangle's duty cycle, named at the switch's own three ("Wide Rectangle"
/// with its current circuit duty, or "Rectangle 24 %" between the places).
pub fn shape_text(n: usize, t: f64) -> String {
    use ca72::voice::{SHAPE_PLACES, shape_at, shape_bias};
    let at = shape_at(
        Shape {
            continuous: true,
            at: t.clamp(0.0, 1.0) * SHAPE_PLACES,
            modulated: false,
            direction: ModDirection::Onward,
        },
        Default::default(),
    );
    let names = shape_names(n);
    if at >= 3.0 {
        let duty = (100.0 * ca72::voice::rect_duty(shape_bias(at))).round();
        let name = [
            (3.0, "Square"),
            (4.0, "Wide Rectangle"),
            (5.0, "Narrow Rectangle"),
        ]
        .iter()
        .find(|(place, _)| *place == at)
        .map_or("Rectangle", |(_, name)| *name);
        return format!("{name} {duty:.0} %");
    }
    let k = at.floor() as usize;
    if at == k as f64 {
        return names[k].to_owned();
    }
    let share = (100.0 * (at - k as f64)).round();
    format!("{} to {} {share:.0} %", names[k], names[k + 1])
}

/// The travel at which the rectangle's duty cycle is `d` (held to the rectangle's stretch).
pub fn travel_at_duty(d: f64) -> f64 {
    use ca72::voice::{SHAPE_PLACES, rect_duty, shape_bias};
    // (The duty falls as the travel goes on: a bisection.)
    let (mut lo, mut hi) = (3.0, SHAPE_PLACES);
    for _ in 0..40 {
        let mid = 0.5 * (lo + hi);
        if rect_duty(shape_bias(mid)) > d {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi) / SHAPE_PLACES
}

/// Inverse of the waveform's displayed names, blends and rectangle duties. Shared by
/// oscillator and LFO host parameters; numeric input retains its existing duty-percent units.
pub(crate) fn parse_shape_text(n: usize, text: &str) -> Option<f32> {
    let s = text.trim().to_ascii_lowercase();
    let names = shape_names(n).map(str::to_ascii_lowercase);
    let places = ca72::voice::SHAPE_PLACES;
    // Rounded blend endpoints and rectangle duties may coincide with a named place.
    // Pick a representative inside that display bin so text -> value -> text is stable.
    let displayed = |guess: f64| {
        [0.0, 0.002, -0.002, 0.004, -0.004]
            .into_iter()
            .map(|offset| (guess + offset / places).clamp(0.0, 1.0) as f32)
            .find(|v| shape_text(n, f64::from(*v)).eq_ignore_ascii_case(&s))
            .unwrap_or(guess as f32)
    };
    let percent = |text: &str| {
        text.trim_end_matches('%')
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
    };
    if let Some(k) = names.iter().position(|name| *name == s) {
        return Some((k as f64 / places) as f32);
    }
    for k in 0..3 {
        let prefix = format!("{} to {} ", names[k], names[k + 1]);
        if let Some(share) = s.strip_prefix(&prefix) {
            return percent(share).map(|v| displayed((k as f64 + v / 100.0) / places));
        }
    }
    for (name, at) in [("wide rectangle", 4.0), ("narrow rectangle", 5.0)] {
        if s == name {
            return Some((at / places) as f32);
        }
    }
    for (name, at) in [
        ("square ", 3.0),
        ("wide rectangle ", 4.0),
        ("narrow rectangle ", 5.0),
        ("rectangle ", 3.0),
    ] {
        if let Some(duty) = s.strip_prefix(name) {
            return percent(duty).map(|v| {
                let named = (at / places) as f32;
                if shape_text(n, f64::from(named)).eq_ignore_ascii_case(&s) {
                    named
                } else {
                    displayed(travel_at_duty(v / 100.0))
                }
            });
        }
    }
    percent(&s).map(|v| travel_at_duty(v / 100.0) as f32)
}

/// Oscillator `n`'s CONTINUOUS shape: its travel, 0 (TRIANGLE) to 1 (the narrowest pulse), at
/// SAWTOOTH to begin with, as WAVEFORM is; shown by [`shape_text`]; a place's name, or a duty
/// cycle ("30 %"), typed.
fn shape_param(name: &str, n: usize) -> FloatParam {
    let places = ca72::voice::SHAPE_PLACES;
    FloatParam::new(
        name,
        (2.0 / places) as f32,
        FloatRange::Linear { min: 0.0, max: 1.0 },
    )
    .with_value_to_string(std::sync::Arc::new(move |t| shape_text(n, f64::from(t))))
    .with_string_to_value(std::sync::Arc::new(move |s| parse_shape_text(n, s)))
}

/// FILTER MODE, which the original does not have (decisions.md R46): LO, the filter as
/// drawn; HI, the hardware reference's high-pass (the mixer's output less the filter's).
#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterMode {
    #[id = "lo"]
    #[name = "LO"]
    Lo,
    #[id = "hi"]
    #[name = "HI"]
    Hi,
}

/// QUALITY (decisions.md R47, R48): LO, the light model (Potato mode); HI, the
/// circuit's model at its real-time setting; ULTRA, the circuit's model with no compromises
/// (the model's own No Compromises), for offline renders and small projects on powerful
/// computers. In this order, low to high, as a host lists them.
#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityMode {
    #[id = "lo"]
    #[name = "LO"]
    Lo,
    #[id = "hi"]
    #[name = "HI"]
    Hi,
    #[id = "ultra"]
    #[name = "ULTRA"]
    Ultra,
}

/// NOTE PRIORITY (decisions.md R50): which of the keys held sounds while POLY is off
/// (and in UNISON, each voice the same). LOW, the keyboard circuit's own; LAST, the key pressed
/// last; HIGH, the highest. In this order, as the toggle's positions run left to right.
#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotePriority {
    #[id = "low"]
    #[name = "LOW"]
    Low,
    #[id = "last"]
    #[name = "LAST"]
    Last,
    #[id = "high"]
    #[name = "HIGH"]
    High,
}

impl From<NotePriority> for Priority {
    fn from(p: NotePriority) -> Priority {
        match p {
            NotePriority::Low => Priority::Low,
            NotePriority::Last => Priority::Last,
            NotePriority::High => Priority::High,
        }
    }
}

/// The arpeggiator's DIRECTION (decisions.md R-ARP): UP, DOWN, UP/DOWN, RANDOM or ORDER, the
/// drawer's tabs left to right (ORDER the last, so that the others keep their saved values).
#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArpDirection {
    #[id = "up"]
    #[name = "UP"]
    Up,
    #[id = "down"]
    #[name = "DOWN"]
    Down,
    #[id = "up_down"]
    #[name = "UP/DOWN"]
    UpDown,
    #[id = "random"]
    #[name = "RANDOM"]
    Random,
    #[id = "order"]
    #[name = "ORDER"]
    Order,
}

impl From<ArpDirection> for crate::arp::Direction {
    fn from(d: ArpDirection) -> crate::arp::Direction {
        match d {
            ArpDirection::Up => crate::arp::Direction::Up,
            ArpDirection::Down => crate::arp::Direction::Down,
            ArpDirection::UpDown => crate::arp::Direction::UpDown,
            ArpDirection::Random => crate::arp::Direction::Random,
            ArpDirection::Order => crate::arp::Direction::Order,
        }
    }
}

/// The arpeggiator's RATE (decisions.md R-ARP): a note value, slow to fast as its dial turns,
/// each straight value's dotted and triplet ones between it and the next
/// (`crate::arp::RATES`, in the same order).
#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArpRate {
    #[id = "1/2"]
    #[name = "1/2"]
    Half,
    #[id = "1/4d"]
    #[name = "1/4 dotted"]
    QuarterDotted,
    #[id = "1/2t"]
    #[name = "1/2 triplet"]
    HalfTriplet,
    #[id = "1/4"]
    #[name = "1/4"]
    Quarter,
    #[id = "1/8d"]
    #[name = "1/8 dotted"]
    EighthDotted,
    #[id = "1/4t"]
    #[name = "1/4 triplet"]
    QuarterTriplet,
    #[id = "1/8"]
    #[name = "1/8"]
    Eighth,
    #[id = "1/16d"]
    #[name = "1/16 dotted"]
    SixteenthDotted,
    #[id = "1/8t"]
    #[name = "1/8 triplet"]
    EighthTriplet,
    #[id = "1/16"]
    #[name = "1/16"]
    Sixteenth,
    #[id = "1/32d"]
    #[name = "1/32 dotted"]
    ThirtySecondDotted,
    #[id = "1/16t"]
    #[name = "1/16 triplet"]
    SixteenthTriplet,
    #[id = "1/32"]
    #[name = "1/32"]
    ThirtySecond,
}

/// An oscillator's DIVISION, timed (decisions.md R-OSC-TIMED): its cycle a note value, slow to
/// fast as its dial turns, 4, 2 and 1 bars, then each straight value's dotted and triplet ones
/// between it and the next (`crate::timed::DIVISIONS`, in the same order); its ids the
/// arpeggiator's RATE's form.
#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Division {
    #[id = "4bars"]
    #[name = "4 bars"]
    FourBars,
    #[id = "2bars"]
    #[name = "2 bars"]
    TwoBars,
    #[id = "1bar"]
    #[name = "1 bar"]
    Bar,
    #[id = "1/2d"]
    #[name = "1/2 dotted"]
    HalfDotted,
    #[id = "1/1t"]
    #[name = "1/1 triplet"]
    WholeTriplet,
    #[id = "1/2"]
    #[name = "1/2"]
    Half,
    #[id = "1/4d"]
    #[name = "1/4 dotted"]
    QuarterDotted,
    #[id = "1/2t"]
    #[name = "1/2 triplet"]
    HalfTriplet,
    #[id = "1/4"]
    #[name = "1/4"]
    Quarter,
    #[id = "1/8d"]
    #[name = "1/8 dotted"]
    EighthDotted,
    #[id = "1/4t"]
    #[name = "1/4 triplet"]
    QuarterTriplet,
    #[id = "1/8"]
    #[name = "1/8"]
    Eighth,
    #[id = "1/16d"]
    #[name = "1/16 dotted"]
    SixteenthDotted,
    #[id = "1/8t"]
    #[name = "1/8 triplet"]
    EighthTriplet,
    #[id = "1/16"]
    #[name = "1/16"]
    Sixteenth,
    #[id = "1/32d"]
    #[name = "1/32 dotted"]
    ThirtySecondDotted,
    #[id = "1/16t"]
    #[name = "1/16 triplet"]
    SixteenthTriplet,
    #[id = "1/32"]
    #[name = "1/32"]
    ThirtySecond,
}

/// The arpeggiator's OCTAVES: 1 to 5 (the owner, 2026-10-19: "let's add 5 octaves as an
/// option"; a value saved as 1 to 4 is the same octaves still, saved plain).
pub const ARP_OCTAVES: (i32, i32) = (1, crate::arp::MOST_OCTAVES as i32);

#[derive(Params)]
pub struct Ca72Params {
    /// The noise generator's seed, saved with the session so that it renders the same noise
    /// again.
    #[persist = "noise_seed"]
    pub noise_seed: Arc<AtomicU64>,
    /// The standalone's TEMPO (beats a minute, f64 bits; decisions.md R-ARP), this computer's
    /// (`settings.rs`), the arpeggiator's while the host gives none.
    pub tempo: Arc<AtomicU64>,
    /// The editor's width in logical pixels (its height follows the panel's proportions); 0
    /// until it is resized, when it opens at a share of the screen.
    #[persist = "editor_width"]
    pub editor_width: Arc<AtomicU32>,
    /// Whether the lower panel's controls are shown, saved with the session, outside presets.
    #[persist = "editor_panel"]
    pub editor_panel: Arc<AtomicBool>,
    /// The window's current height: presets can temporarily reveal a hidden lower panel.
    /// Set before asking the host to resize, so its size query sees the requested height.
    pub editor_expanded: Arc<AtomicBool>,
    /// The last preference the open editor applied, to recognise project state recalled later.
    pub editor_panel_applied: Arc<AtomicBool>,
    /// Whether presets are temporarily occupying the lower panel, for the host's size query.
    pub editor_presets: Arc<AtomicBool>,
    pub(crate) editor_resizing: Arc<crate::resizing::Resizing>,
    /// The preset the plug-in was last set to (decisions.md R10), empty when none: saved
    /// with the session, so the bar names it again.
    #[persist = "preset"]
    pub preset: Arc<RwLock<String>>,
    /// The selected preset’s opaque effects, carried through plug-in sessions (R-FX-PRESETS).
    #[persist = "preset_effects"]
    pub preset_effects: Arc<RwLock<Option<String>>>,
    /// The MIDI controllers assigned to parameters by MIDI Learn, and the learning (decisions.md
    /// R34): saved with the session (`crate::learn::Saved`), not with a sound preset.
    #[persist = "midi_map"]
    pub midi_map: Arc<MidiMap>,

    /// AUTO GAIN's curve for the sound (decisions.md R45; `drive.rs`): measured when DRIVE or
    /// AUTO GAIN is changed in the editor, a preset's taken as it is chosen (R-AUTO-GAIN-DRIVE),
    /// kept with the session so that it plays and renders the same again.
    #[persist = "drive_curve"]
    pub drive_curve: Calibration,
    /// How the FREQUENCY knobs' dials read on this computer (`settings.rs`; decisions.md
    /// R54): 0 a semitone a mark, 1 the hardware's. Not the session's: a learned
    /// controller turns the knob through it ([`Ca72Params::freq_law`]).
    pub freq_law: Arc<AtomicU8>,
    /// Where MPE's zones come from on this computer (`settings.rs`, `expression::Mode`;
    /// decisions.md R-MPE). Not the session's.
    pub mpe: Arc<AtomicU8>,
    /// Where a timbre rests on this computer (`settings.rs`, `expression::TimbreRest`;
    /// decisions.md R-MPE), read by the audio thread each block. Not the session's.
    pub timbre_rest: Arc<AtomicU8>,
    /// What expression has arrived, per note and on ordinary channels, counted by the audio
    /// thread for the editor's INPUT screen (`expression::Heard`; R-MPE). Not the
    /// session's.
    pub heard: Arc<crate::expression::Heard>,

    /// The knobs' values a preset or Init is setting, marked so the voices take them at once
    /// instead of gliding there (`crate::smooth`; decisions.md R-KNOB-GLIDE). Not the session's.
    pub lands: crate::smooth::Lands,

    /// The LFOs' assignments and EDGES (decisions.md R-LFO): the patch's, saved with the session
    /// and with presets (`lfoN_to_<target>`, `lfoN_edges`), not host parameters. A session or
    /// a preset saved before has none (`Ca72::filter_state`, `presets::targets`).
    #[persist = "lfo"]
    pub lfo_patch: Arc<crate::lfo::Patch>,
    /// Each LFO's present value and phase from the audio thread, for the editor (R-LFO). Not
    /// the session's.
    pub lfo_scope: Arc<crate::lfo::Scope>,
    /// Whether this is the standalone application (its host's API, told at initialisation and
    /// every block; a plug-in until then): its TEMPO a modulation target there alone
    /// (`lfo::Target::Tempo`, decisions.md R-LFO), the plug-ins following their host's.
    pub standalone: Arc<AtomicBool>,
    /// TEMPO's travel as a knob's (`TEMPO_RANGE`, straight along it), for its modulation's
    /// depths (a share of its travel: `lfo::Target::Tempo`). Not a host parameter: never
    /// registered, its value never read.
    pub tempo_knob: FloatParam,

    /// POWER off: the host's bypass, the output faded out.
    #[id = "bypass"]
    pub bypass: BoolParam,

    // CONTROLLERS
    /// TUNE in its dial's units (printed -2..+2, -2.5..+2.5 at its ends).
    #[id = "tune"]
    pub tune: FloatParam,
    #[id = "glide"]
    pub glide: FloatParam,
    /// MODULATION MIX: oscillator 3 (0) to the noise (10).
    #[id = "mod_mix"]
    pub mod_mix: FloatParam,
    #[id = "osc_mod"]
    pub osc_mod: BoolParam,
    #[id = "osc3_control"]
    pub osc3_control: BoolParam,

    // OSCILLATOR BANK
    #[id = "osc1_range"]
    pub osc1_range: EnumParam<Footage>,
    #[id = "osc1_waveform"]
    pub osc1_waveform: EnumParam<Wave>,
    #[id = "osc2_range"]
    pub osc2_range: EnumParam<Footage>,
    /// FREQUENCY (decisions.md R54): its pot's position in the dial's units, -7.5 to
    /// +7.5, the wiper at (value + 7.5) / 15 of its track, as every version has saved it; shown
    /// and typed in semitones ([`frequency`]). The knob reads it through the computer's law
    /// (`ca72::freq_dial`): the dial a semitone a mark (7 printed at 140 degrees, 7.5 at the
    /// ends of the knob's travel), or the hardware's.
    #[id = "osc2_frequency"]
    pub osc2_frequency: FloatParam,
    #[id = "osc2_waveform"]
    pub osc2_waveform: EnumParam<Wave>,
    #[id = "osc3_range"]
    pub osc3_range: EnumParam<Footage>,
    #[id = "osc3_frequency"]
    pub osc3_frequency: FloatParam,
    #[id = "osc3_waveform"]
    pub osc3_waveform: EnumParam<Wave3>,

    // MIXER
    #[id = "osc1_on"]
    pub osc1_on: BoolParam,
    #[id = "osc1_volume"]
    pub osc1_volume: FloatParam,
    /// EXTERNAL INPUT: the side chain, through the preamplifier.
    #[id = "ext_on"]
    pub ext_on: BoolParam,
    #[id = "ext_volume"]
    pub ext_volume: FloatParam,
    #[id = "osc2_on"]
    pub osc2_on: BoolParam,
    #[id = "osc2_volume"]
    pub osc2_volume: FloatParam,
    #[id = "noise_on"]
    pub noise_on: BoolParam,
    #[id = "noise_volume"]
    pub noise_volume: FloatParam,
    #[id = "noise_type"]
    pub noise_type: EnumParam<NoiseColor>,
    #[id = "osc3_on"]
    pub osc3_on: BoolParam,
    #[id = "osc3_volume"]
    pub osc3_volume: FloatParam,

    // MODIFIERS
    #[id = "filter_mode"]
    pub filter_mode: EnumParam<FilterMode>,
    #[id = "filter_mod"]
    pub filter_mod: BoolParam,
    #[id = "keyboard_control_1"]
    pub keyboard_control_1: BoolParam,
    #[id = "keyboard_control_2"]
    pub keyboard_control_2: BoolParam,
    #[id = "cutoff"]
    pub cutoff: FloatParam,
    #[id = "emphasis"]
    pub emphasis: FloatParam,
    #[id = "contour_amount"]
    pub contour_amount: FloatParam,
    #[id = "filter_attack"]
    pub filter_attack: FloatParam,
    #[id = "filter_decay"]
    pub filter_decay: FloatParam,
    #[id = "filter_sustain"]
    pub filter_sustain: FloatParam,
    #[id = "loudness_attack"]
    pub loudness_attack: FloatParam,
    #[id = "loudness_decay"]
    pub loudness_decay: FloatParam,
    #[id = "loudness_sustain"]
    pub loudness_sustain: FloatParam,

    // OUTPUT
    #[id = "volume"]
    pub volume: FloatParam,
    #[id = "main_output"]
    pub main_output: BoolParam,
    #[id = "a440"]
    pub a440: BoolParam,

    // LEFT HAND CONTROLLER
    #[id = "glide_on"]
    pub glide_on: BoolParam,
    /// On, the contours decay after a key's release.
    #[id = "decay_on"]
    pub decay_on: BoolParam,
    /// The PITCH wheel, -1..1, 0 in its detent.
    #[id = "pitch_wheel"]
    pub pitch_wheel: FloatParam,
    /// The MODULATION wheel, 1 fully forward.
    #[id = "mod_wheel"]
    pub mod_wheel: FloatParam,

    // MIDI
    /// How far a MIDI keyboard's pitch bend, fully up or down, moves the PITCH wheel, in
    /// semitones: up to the wheel's whole travel.
    #[id = "midi_bend_range"]
    pub midi_bend_range: FloatParam,

    // The plug-in's own, not the instrument's (decisions.md, "POLY", "ANALOG and SPREAD").
    /// POLY: one instrument per voice (VOICES of them); off, the one, NOTE PRIORITY choosing
    /// which of the keys held sounds (LOW, its keyboard circuit's own).
    #[id = "poly"]
    pub poly: BoolParam,
    /// VOICES: how many POLY plays, 2 to 10; with POLY and UNISON off, how many places the one
    /// instrument goes round, a place an attack (decisions.md R62).
    #[id = "voices"]
    pub voices: IntParam,
    /// ENTROPY: each voice its own parts and its oscillators drifting, 0 to 100 %.
    #[id = "entropy"]
    pub entropy: FloatParam,
    /// SPREAD: POLY's voices across the stereo field, 0 to 100 %.
    #[id = "spread"]
    pub spread: FloatParam,
    /// FEEDBACK, the phones' VOLUME knob (0 to 10): the output cabled into EXTERNAL INPUT,
    /// the level sent following the knob's taper (`ca72::voice::feedback_law`).
    #[id = "feedback"]
    pub feedback: FloatParam,
    /// LOCK: the oscillators identical, the circuit as drawn, its level matched (off: each
    /// keeps a small mismatch of its own; decisions.md R9).
    #[id = "lock"]
    pub lock: BoolParam,
    /// SCATTER's placement, EVEN (the default) or CENTER: where SPREAD puts the voices
    /// (decisions.md R45, R49). The last parameter: a session saved before it reads
    /// EVEN, and one saved with EDGES opens with EVEN too (`Ca72::filter_state`).
    #[id = "placement"]
    pub placement: EnumParam<Scatter>,
    /// UNISON: VOICES instruments on every key together (decisions.md R45). After the
    /// placement: a session saved before it reads it off.
    #[id = "unison"]
    pub unison: BoolParam,
    /// DOUBLE: each note two voices, detuned up to 100 cents, either side of the centre, 0 to
    /// 100 % (decisions.md R45). The last parameter: a session saved before it reads 0.
    #[id = "double"]
    pub double: FloatParam,
    /// DRIVE (0 to 24 dB): the mixer's signal into the filter raised, its input pair driven
    /// harder (decisions.md R45). After DOUBLE: a session saved before reads it off.
    #[id = "drive"]
    pub drive: FloatParam,
    /// LEVEL (dB): the output's gain, the plug-in's own, after MAIN OUTPUT's (decisions.md
    /// R45). After DRIVE: a session saved before reads 0 dB.
    #[id = "level"]
    pub level: FloatParam,
    /// AUTO GAIN (on by default): the output brought back down by as much as DRIVE made the
    /// sound louder, as measured for it (decisions.md R45). After LEVEL: a session saved
    /// before reads it on.
    #[id = "auto_gain"]
    pub auto_gain: BoolParam,
    /// INNER (0 to 100 %): the inner edge of each side's band, a share of SPREAD's way out, the
    /// centre cleared of voices (decisions.md R49). After AUTO GAIN: a session saved before
    /// reads 0.
    #[id = "inner"]
    pub inner: FloatParam,
    /// QUALITY, LO by default for a new instance (R-LO-DEFAULT; HI until 0.2.4): at LO the
    /// voices are played by the light model, Potato mode, for computers the circuit's is too
    /// heavy for (decisions.md R47); at ULTRA by the circuit's model with no compromises (R48).
    /// Not a preset's (`library::KEPT`): it suits the computer, not the sound; saved with the
    /// session. After AUTO GAIN: a session saved before it reads it HI (`filter_state`).
    #[id = "quality"]
    pub quality: EnumParam<QualityMode>,
    /// DOUBLE (the strip's SCATTER | DOUBLE): each note two voices, DETUNE apart, either side
    /// of the centre; off, SCATTER, DETUNE unused (the owner, 2026-10-10: DOUBLE had been
    /// DETUNE above 0, so DETUNE at 0 "instantly switches to scatter"; "make that knob
    /// unresponsive unless the user is in double mode"). After QUALITY: a session or a preset
    /// saved before reads it on wherever its DETUNE is above 0 (`Ca72::filter_state`,
    /// `presets::targets`).
    #[id = "doubled"]
    pub doubled: BoolParam,
    /// NOTE PRIORITY, LOW by default: which key held sounds (the toggle under GLIDE and DECAY).
    /// A preset's, as any panel control (the owner, 2026-10-10: "This item should be saved per
    /// patch"). The last parameter: a session or a preset saved before reads it LOW, the
    /// instrument's own.
    #[id = "note_priority"]
    pub note_priority: EnumParam<NotePriority>,
    /// RETRIGGER, off by default: each key pressed starts the contours again, other keys held
    /// or not (the switch under GLIDE and DECAY; off, single triggering, the instrument's). A
    /// preset's, as NOTE PRIORITY. LAST brings the new pitch with the attack; LOW or HIGH can
    /// keep a held key's old pitch until it is lifted. Key lifts do not restart the contours.
    /// After it: a session or a preset saved before reads it off.
    #[id = "retrigger"]
    pub retrigger: BoolParam,

    // The arpeggiator (decisions.md R-ARP), the plug-in's own, after everything else, so that
    // no parameter before it moves. A preset's, as any sound parameter: a preset or a session
    // saved before reads it off (`Ca72::filter_state`, `presets::targets`).
    /// ON: the keys held played in turn (off: as played).
    #[id = "arp_on"]
    pub arp_on: BoolParam,
    /// LATCH: the chord last played kept playing after its keys are let go.
    #[id = "arp_latch"]
    pub arp_latch: BoolParam,
    #[id = "arp_direction"]
    pub arp_direction: EnumParam<ArpDirection>,
    #[id = "arp_rate"]
    pub arp_rate: EnumParam<ArpRate>,
    #[id = "arp_octaves"]
    pub arp_octaves: IntParam,
    /// GATE: each note's length, a share of what its step leaves between the shortest note and
    /// the shortest gap ([`crate::arp::SHORTEST`], [`crate::arp::REARM`]).
    #[id = "arp_gate"]
    pub arp_gate: FloatParam,
    /// SWING: every second step late, up to three quarters of the way through the pair.
    #[id = "arp_swing"]
    pub arp_swing: FloatParam,

    // Each note's own pressure and timbre (MPE's, a host's note expressions, or every voice's
    // alike from channel pressure and control change 74) moving its voice (decisions.md
    // R-MPE), the plug-in's own, after everything else. A preset's, as any sound parameter: a
    // preset or a session saved before reads them 0, not at all (`Ca72::filter_state`,
    // `presets::targets`).
    /// PRESSURE's reach on the voice's MODULATION wheel, -100 to 100 %: at 100 %, full pressure
    /// moves it as a keyboard's wheel pushed fully forward does.
    #[id = "pressure_mod"]
    pub pressure_mod: FloatParam,
    /// PRESSURE's reach on the voice's filter's cutoff, octaves at full pressure.
    #[id = "pressure_filter"]
    pub pressure_filter: FloatParam,
    /// TIMBRE's (MPE's slide, control change 74) on the voice's MODULATION wheel.
    #[id = "timbre_mod"]
    pub timbre_mod: FloatParam,
    /// TIMBRE's on the voice's filter's cutoff, octaves at its top.
    #[id = "timbre_filter"]
    pub timbre_filter: FloatParam,
    // The oscillators timed (decisions.md R-OSC-TIMED), the plug-in's own, after the
    // arpeggiator's, so that no parameter before them moves. A preset's, as any sound
    // parameter: a preset or a session saved before reads them off (`Ca72::filter_state`,
    // `presets::targets`).
    /// Oscillators 2's and 3's TIMED (oscillator 1 has no FREQUENCY to turn into note values:
    /// the owner): at its RANGE's LO, its cycle its DIVISION of the host's tempo, its resets on
    /// the host's beats and bars, its FREQUENCY keeping its own place for when it is off; at
    /// any other RANGE, nothing. DIVISION: the cycle's note value, a parameter of its own (not
    /// FREQUENCY's knob read otherwise): stepped, so that automation and presets name a value,
    /// never a place between two.
    #[id = "osc2_timed"]
    pub osc2_timed: BoolParam,
    #[id = "osc2_division"]
    pub osc2_division: EnumParam<Division>,
    #[id = "osc3_timed"]
    pub osc3_timed: BoolParam,
    #[id = "osc3_division"]
    pub osc3_division: EnumParam<Division>,
    // Each oscillator's waveform (decisions.md R-CONTINUOUS-WAVE), the plug-in's own, after
    // everything else, so that no parameter before it moves: STEPPED or CONTINUOUS (WAVEFORM's
    // menu), CONTINUOUS's shape (the switch turned), its MOD key and MOD's direction (the key
    // Command-clicked). A preset's, as any sound parameter; a preset or a session saved before
    // reads STEPPED.
    #[id = "osc1_shape_mode"]
    pub osc1_shape_mode: EnumParam<WaveMode>,
    #[id = "osc1_shape"]
    pub osc1_shape: FloatParam,
    #[id = "osc1_shape_mod"]
    pub osc1_shape_mod: BoolParam,
    #[id = "osc1_shape_mod_dir"]
    pub osc1_shape_mod_dir: EnumParam<ShapeModDirection>,
    #[id = "osc2_shape_mode"]
    pub osc2_shape_mode: EnumParam<WaveMode>,
    #[id = "osc2_shape"]
    pub osc2_shape: FloatParam,
    #[id = "osc2_shape_mod"]
    pub osc2_shape_mod: BoolParam,
    #[id = "osc2_shape_mod_dir"]
    pub osc2_shape_mod_dir: EnumParam<ShapeModDirection>,
    #[id = "osc3_shape_mode"]
    pub osc3_shape_mode: EnumParam<WaveMode>,
    #[id = "osc3_shape"]
    pub osc3_shape: FloatParam,
    #[id = "osc3_shape_mod"]
    pub osc3_shape_mod: BoolParam,
    #[id = "osc3_shape_mod_dir"]
    pub osc3_shape_mod_dir: EnumParam<ShapeModDirection>,
    #[id = "glide_range"]
    pub glide_range: EnumParam<GlideRange>,
    // The four LFOs (decisions.md R-LFO), the plug-in's own, after everything else, so that no
    // parameter before them moves: `lfo1_waveform` to `lfo4_mono`. A preset's, as any sound
    // parameter; a preset or a session saved before reads each at its default, none
    // assigned, so none of them does anything.
    #[nested(id_prefix = "lfo1", group = "LFO 1")]
    pub lfo1: crate::lfo::LfoParams,
    #[nested(id_prefix = "lfo2", group = "LFO 2")]
    pub lfo2: crate::lfo::LfoParams,
    #[nested(id_prefix = "lfo3", group = "LFO 3")]
    pub lfo3: crate::lfo::LfoParams,
    #[nested(id_prefix = "lfo4", group = "LFO 4")]
    pub lfo4: crate::lfo::LfoParams,
    // The six macros (decisions.md R-MACROS), after the LFOs, so that no parameter before them
    // moves: 0 to 100 %, named MACRO 1 to MACRO 6 in the host whatever their typed names. A
    // preset or a session saved before has each at 0, none assigned.
    #[id = "macro1"]
    pub macro1: FloatParam,
    #[id = "macro2"]
    pub macro2: FloatParam,
    #[id = "macro3"]
    pub macro3: FloatParam,
    #[id = "macro4"]
    pub macro4: FloatParam,
    #[id = "macro5"]
    pub macro5: FloatParam,
    #[id = "macro6"]
    pub macro6: FloatParam,
    // Each LFO's POLARITY, REVERSE and ON (decisions.md R-LFO's corrections), after the macros,
    // so that no parameter before them moves: `lfo1_polarity` to `lfo4_on`. A preset or a
    // session saved before reads BI, not reversed and ON.
    #[nested(id_prefix = "lfo1", group = "LFO 1")]
    pub lfo1_keys: crate::lfo::LfoKeys,
    #[nested(id_prefix = "lfo2", group = "LFO 2")]
    pub lfo2_keys: crate::lfo::LfoKeys,
    #[nested(id_prefix = "lfo3", group = "LFO 3")]
    pub lfo3_keys: crate::lfo::LfoKeys,
    #[nested(id_prefix = "lfo4", group = "LFO 4")]
    pub lfo4_keys: crate::lfo::LfoKeys,
    // Each LFO's LEVEL (decisions.md R-LFO's addendum), after everything, so that no parameter
    // before it moves: `lfo1_level` to `lfo4_level`. A preset or a session saved before reads
    // 100 %.
    #[nested(id_prefix = "lfo1", group = "LFO 1")]
    pub lfo1_level: crate::lfo::LfoLevel,
    #[nested(id_prefix = "lfo2", group = "LFO 2")]
    pub lfo2_level: crate::lfo::LfoLevel,
    #[nested(id_prefix = "lfo3", group = "LFO 3")]
    pub lfo3_level: crate::lfo::LfoLevel,
    #[nested(id_prefix = "lfo4", group = "LFO 4")]
    pub lfo4_level: crate::lfo::LfoLevel,
    // The velocity's response (decisions.md R-VELOCITY), after everything, so that no parameter
    // before it moves. A preset's, as any sound parameter: a preset or a session saved before
    // reads a straight line from 0 (`Ca72::filter_state`, `presets::targets`).
    /// CURVE: -100 % (SOFT: light notes give more) to 100 % (HARD: they give less), 0 a
    /// straight line ([`crate::lfo::Response`]).
    #[id = "velocity_curve"]
    pub velocity_curve: FloatParam,
    /// FLOOR: the least the velocity gives, the softest note's, 0 to 100 %.
    #[id = "velocity_floor"]
    pub velocity_floor: FloatParam,
}

/// The velocity's CURVE, -100 (SOFT) to 100 % (HARD), 0 by default.
fn velocity_curve() -> FloatParam {
    FloatParam::new(
        "Velocity Curve",
        0.0,
        FloatRange::Linear {
            min: -100.0,
            max: 100.0,
        },
    )
    .with_unit(" %")
    .with_value_to_string(formatters::v2s_f32_rounded(0))
}

/// The velocity's FLOOR, 0 to 100 %, 0 by default.
fn velocity_floor() -> FloatParam {
    FloatParam::new(
        "Velocity Floor",
        0.0,
        FloatRange::Linear {
            min: 0.0,
            max: 100.0,
        },
    )
    .with_unit(" %")
    .with_value_to_string(formatters::v2s_f32_rounded(0))
}

/// How far PRESSURE and TIMBRE move a voice's cutoff at most, either way, octaves.
pub const REACH_OCTAVES: f32 = 4.0;

/// A reach on the MODULATION wheel, -100 to 100 %, 0 by default.
fn reach_mod(name: &str) -> FloatParam {
    FloatParam::new(
        name,
        0.0,
        FloatRange::Linear {
            min: -100.0,
            max: 100.0,
        },
    )
    .with_unit(" %")
    .with_value_to_string(formatters::v2s_f32_rounded(0))
}

/// A reach on the cutoff, octaves either way, 0 by default.
fn reach_filter(name: &str) -> FloatParam {
    FloatParam::new(
        name,
        0.0,
        FloatRange::Linear {
            min: -REACH_OCTAVES,
            max: REACH_OCTAVES,
        },
    )
    .with_unit(" octaves")
    .with_value_to_string(formatters::v2s_f32_rounded(2))
}

/// DOUBLE on at DETUNE's 0, as the engine is told it: a share of DETUNE's travel so small that
/// the pair play as one pitch (a ten-thousandth of a cent: a beat a day), the engine's DOUBLE
/// being on while its detune is above 0.
pub const DOUBLED_AT_NONE: f64 = 1e-6;

/// Cents as the strip's DETUNE reads them: a number, whole or to a tenth ("12", "7.2"), as an
/// instrument's display shows it, without a sign (the CA-74's R34).
pub fn cents(c: f64) -> String {
    let c = (c * 10.0).round() / 10.0;
    if c.fract() == 0.0 {
        format!("{c:.0}")
    } else {
        format!("{c:.1}")
    }
}

/// An amount, 0 to 100 %.
fn percent(name: &str) -> FloatParam {
    FloatParam::new(
        name,
        0.0,
        FloatRange::Linear {
            min: 0.0,
            max: 100.0,
        },
    )
    .with_unit(" %")
    .with_value_to_string(formatters::v2s_f32_rounded(0))
}

/// A FREQUENCY pot's offset as the host shows it: cents to a tenth under a semitone ("+3.4
/// cents"), else semitones to a hundredth ("+7.00 semitones").
pub fn semitones_text(st: f64) -> String {
    let c = (st * 1000.0).round() / 10.0;
    if c == 0.0 {
        String::from("0 semitones")
    } else if c.abs() < 100.0 {
        format!("{c:+.1} cents")
    } else {
        format!("{st:+.2} semitones")
    }
}

/// Semitones as typed: a number, "semitones" or "st" after it or none; "cents" or "c" after it
/// for cents.
pub fn semitones_typed(s: &str) -> Option<f64> {
    let t = s.trim();
    let (t, per) = match ["semitones", "semitone", "st"]
        .iter()
        .find_map(|u| t.strip_suffix(u))
    {
        Some(t) => (t, 1.0),
        None => match ["cents", "cent", "\u{a2}", "c"]
            .iter()
            .find_map(|u| t.strip_suffix(u))
        {
            Some(t) => (t, 0.01),
            None => (t, 1.0),
        },
    };
    let v: f64 = t.trim().trim_start_matches('+').parse().ok()?;
    v.is_finite().then_some(v * per)
}

/// The wiper (0..1 of its track) of a FREQUENCY pot at `plain` (the parameter's value).
pub fn wiper_of(plain: f32) -> f64 {
    ((f64::from(plain) + DIAL_END) / (2.0 * DIAL_END)).clamp(0.0, 1.0)
}

/// Oscillator 2's or 3's FREQUENCY ([`Ca72Params::osc2_frequency`]): the pot's position, shown
/// and typed as its oscillator's offset from the centre (beyond the pot's reach, its end).
fn frequency(name: &str, pot: Pot) -> FloatParam {
    let end = DIAL_END as f32;
    FloatParam::new(
        name,
        0.0,
        FloatRange::Linear {
            min: -end,
            max: end,
        },
    )
    .with_value_to_string(Arc::new(move |v| {
        semitones_text(pot.semitones(wiper_of(v)))
    }))
    .with_string_to_value(Arc::new(move |s| {
        let w = pot.wiper(semitones_typed(s)?);
        Some(((2.0 * w - 1.0) * DIAL_END) as f32)
    }))
}

/// A knob over `lo..hi` in its dial's units.
fn dial(name: &str, lo: f32, hi: f32, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: lo, max: hi })
        .with_value_to_string(formatters::v2s_f32_rounded(2))
}

/// A 0 to 10 knob.
fn ten(name: &str, default: f32) -> FloatParam {
    dial(name, 0.0, 10.0, default)
}

/// A fresh seed for a new instance's noise generator.
fn fresh_seed() -> u64 {
    std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish()
}

impl Default for Ca72Params {
    /// The panel as the instrument's default patch has it: oscillator 1 alone on a sawtooth
    /// at 8', the filter half open, MAIN OUTPUT at 10.
    fn default() -> Self {
        Ca72Params {
            noise_seed: Arc::new(AtomicU64::new(fresh_seed())),
            tempo: Arc::new(AtomicU64::new(DEFAULT_TEMPO.to_bits())),
            editor_width: Arc::new(AtomicU32::new(0)),
            editor_panel: Arc::new(AtomicBool::new(true)),
            editor_expanded: Arc::new(AtomicBool::new(true)),
            editor_panel_applied: Arc::new(AtomicBool::new(true)),
            editor_presets: Arc::new(AtomicBool::new(false)),
            editor_resizing: Arc::default(),
            preset: Arc::new(RwLock::new(String::new())),
            preset_effects: Arc::new(RwLock::new(None)),
            midi_map: Arc::new(if cfg!(test) {
                MidiMap::default()
            } else {
                MidiMap::for_computer()
            }),
            freq_law: Arc::new(AtomicU8::new(0)),
            mpe: Arc::new(AtomicU8::new(0)),
            timbre_rest: Arc::new(AtomicU8::new(0)),
            heard: Arc::default(),

            lands: crate::smooth::Lands::default(),
            lfo_patch: Arc::default(),
            lfo_scope: Arc::default(),
            standalone: Arc::new(AtomicBool::new(false)),
            tempo_knob: FloatParam::new(
                "Tempo",
                DEFAULT_TEMPO as f32,
                FloatRange::Linear {
                    min: TEMPO_RANGE.0 as f32,
                    max: TEMPO_RANGE.1 as f32,
                },
            ),
            drive_curve: Calibration::default(),
            bypass: BoolParam::new("Bypass", false).make_bypass(),
            tune: dial("Tune", -2.5, 2.5, 0.0),
            glide: ten("Glide", 0.0),
            mod_mix: ten("Modulation Mix", 0.0),
            osc_mod: BoolParam::new("Oscillator Modulation", false),
            osc3_control: BoolParam::new("Osc 3 Control", true),
            osc1_range: EnumParam::new("Osc 1 Range", Footage::R8),
            osc1_waveform: EnumParam::new("Osc 1 Waveform", Wave::Sawtooth),
            osc2_range: EnumParam::new("Osc 2 Range", Footage::R8),
            osc2_frequency: frequency("Osc 2 Frequency", Pot::Two),
            osc2_waveform: EnumParam::new("Osc 2 Waveform", Wave::Sawtooth),
            osc3_range: EnumParam::new("Osc 3 Range", Footage::R8),
            osc3_frequency: frequency("Osc 3 Frequency", Pot::Three),
            osc3_waveform: EnumParam::new("Osc 3 Waveform", Wave3::Sawtooth),
            osc1_on: BoolParam::new("Osc 1 On", true),
            osc1_volume: ten("Osc 1 Volume", 8.0),
            ext_on: BoolParam::new("External Input On", false),
            ext_volume: ten("External Input Volume", 5.0),
            osc2_on: BoolParam::new("Osc 2 On", false),
            osc2_volume: ten("Osc 2 Volume", 8.0),
            noise_on: BoolParam::new("Noise On", false),
            noise_volume: ten("Noise Volume", 8.0),
            noise_type: EnumParam::new("Noise Color", NoiseColor::White),
            osc3_on: BoolParam::new("Osc 3 On", false),
            osc3_volume: ten("Osc 3 Volume", 8.0),
            filter_mode: EnumParam::new("Filter Mode", FilterMode::Lo),
            filter_mod: BoolParam::new("Filter Modulation", false),
            keyboard_control_1: BoolParam::new("Keyboard Control 1", true),
            keyboard_control_2: BoolParam::new("Keyboard Control 2", false),
            cutoff: dial("Cutoff Frequency", -5.0, 5.0, 0.0),
            emphasis: ten("Emphasis", 0.0),
            contour_amount: ten("Amount of Contour", 5.0),
            filter_attack: ten("Filter Attack Time", 0.0),
            filter_decay: ten("Filter Decay Time", 4.0),
            filter_sustain: ten("Filter Sustain Level", 5.0),
            loudness_attack: ten("Loudness Attack Time", 0.0),
            loudness_decay: ten("Loudness Decay Time", 4.0),
            loudness_sustain: ten("Loudness Sustain Level", 8.0),
            volume: ten("Volume", 10.0),
            main_output: BoolParam::new("Main Output", true),
            a440: BoolParam::new("A-440", false),
            glide_on: BoolParam::new("Glide Switch", false),
            decay_on: BoolParam::new("Decay Switch", true),
            pitch_wheel: dial("Pitch Wheel", -1.0, 1.0, 0.0),
            mod_wheel: dial("Modulation Wheel", 0.0, 1.0, 0.0),
            midi_bend_range: dial("MIDI Bend Range", 0.0, PITCH_WHEEL_SEMITONES as f32, 2.0)
                .with_unit(" semitones"),
            poly: BoolParam::new("Poly", false),
            voices: IntParam::new(
                "Voices",
                POLY_VOICES.1 as i32,
                IntRange::Linear {
                    min: POLY_VOICES.0 as i32,
                    max: POLY_VOICES.2 as i32,
                },
            ),
            entropy: FloatParam::new(
                "Entropy",
                5.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 100.0,
                },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),
            // (The strip's WIDTH: the id is SPREAD's, as sessions and presets have it; the
            // CA-74's R31.)
            spread: percent("Width"),
            feedback: ten("Feedback", 0.0),
            lock: BoolParam::new("Lock (oscillators identical)", false),
            placement: EnumParam::new("Scatter Placement", Scatter::Even),
            unison: BoolParam::new("Unison", false),
            // (The strip's DETUNE, DOUBLE's: its amount in cents, none SCATTER; the CA-74's
            // R31.)
            double: percent("Double Detune")
                .with_unit("")
                .with_value_to_string(Arc::new(|v| {
                    if v > 0.0 {
                        format!("{} cents", cents(f64::from(v) / 100.0 * DOUBLE_CENTS))
                    } else {
                        String::from("Off")
                    }
                }))
                .with_string_to_value(Arc::new(|s| {
                    let t = s.trim();
                    let t = t
                        .strip_suffix("cents")
                        .or_else(|| t.strip_suffix("cent"))
                        .or_else(|| t.strip_suffix('\u{a2}'))
                        .unwrap_or(t)
                        .trim();
                    if t.eq_ignore_ascii_case("off") {
                        return Some(0.0);
                    }
                    let c: f64 = t.parse().ok()?;
                    Some((c / DOUBLE_CENTS * 100.0).clamp(0.0, 100.0) as f32)
                })),
            drive: FloatParam::new(
                "Drive",
                0.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: DRIVE_TOP as f32,
                },
            )
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            level: FloatParam::new(
                "Level",
                0.0,
                FloatRange::Linear {
                    min: LEVEL_RANGE.0 as f32,
                    max: LEVEL_RANGE.1 as f32,
                },
            )
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            auto_gain: BoolParam::new("Auto Gain", true),
            inner: percent("Inner"),
            doubled: BoolParam::new("Double", false),
            quality: EnumParam::new("Quality", QualityMode::Lo),
            note_priority: EnumParam::new("Note Priority", NotePriority::Low),
            retrigger: BoolParam::new("Retrigger", false),
            arp_on: BoolParam::new("Arp", false),
            // (HOLD, once LATCH: the owner, 2026-10-19; its id kept, so that what was saved
            // loads.)
            arp_latch: BoolParam::new("Arp Hold", false),
            arp_direction: EnumParam::new("Arp Direction", ArpDirection::Up),
            arp_rate: EnumParam::new("Arp Rate", ArpRate::Sixteenth),
            arp_octaves: IntParam::new(
                "Arp Octaves",
                1,
                IntRange::Linear {
                    min: ARP_OCTAVES.0,
                    max: ARP_OCTAVES.1,
                },
            ),
            arp_gate: FloatParam::new(
                "Arp Gate",
                50.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 100.0,
                },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),
            arp_swing: percent("Arp Swing"),
            pressure_mod: reach_mod("Pressure to Modulation"),
            pressure_filter: reach_filter("Pressure to Filter"),
            timbre_mod: reach_mod("Timbre to Modulation"),
            timbre_filter: reach_filter("Timbre to Filter"),
            osc2_timed: BoolParam::new("Osc 2 Timed", false),
            osc2_division: EnumParam::new("Osc 2 Division", Division::Quarter),
            osc3_timed: BoolParam::new("Osc 3 Timed", false),
            osc3_division: EnumParam::new("Osc 3 Division", Division::Quarter),
            osc1_shape_mode: EnumParam::new("Osc 1 Waveform Mode", WaveMode::Stepped),
            osc1_shape: shape_param("Osc 1 Shape", 0),
            osc1_shape_mod: BoolParam::new("Osc 1 Shape Mod", false),
            osc1_shape_mod_dir: EnumParam::new(
                "Osc 1 Shape Mod Direction",
                ShapeModDirection::Clockwise,
            ),
            osc2_shape_mode: EnumParam::new("Osc 2 Waveform Mode", WaveMode::Stepped),
            osc2_shape: shape_param("Osc 2 Shape", 1),
            osc2_shape_mod: BoolParam::new("Osc 2 Shape Mod", false),
            osc2_shape_mod_dir: EnumParam::new(
                "Osc 2 Shape Mod Direction",
                ShapeModDirection::Clockwise,
            ),
            osc3_shape_mode: EnumParam::new("Osc 3 Waveform Mode", WaveMode::Stepped),
            osc3_shape: shape_param("Osc 3 Shape", 2),
            osc3_shape_mod: BoolParam::new("Osc 3 Shape Mod", false),
            osc3_shape_mod_dir: EnumParam::new(
                "Osc 3 Shape Mod Direction",
                ShapeModDirection::Clockwise,
            ),
            glide_range: EnumParam::new("Glide Range", GlideRange::Normal),
            lfo1: crate::lfo::LfoParams::new(1),
            lfo2: crate::lfo::LfoParams::new(2),
            lfo3: crate::lfo::LfoParams::new(3),
            lfo4: crate::lfo::LfoParams::new(4),
            macro1: crate::lfo::macro_param(1),
            macro2: crate::lfo::macro_param(2),
            macro3: crate::lfo::macro_param(3),
            macro4: crate::lfo::macro_param(4),
            macro5: crate::lfo::macro_param(5),
            macro6: crate::lfo::macro_param(6),
            lfo1_keys: crate::lfo::LfoKeys::new(1),
            lfo2_keys: crate::lfo::LfoKeys::new(2),
            lfo3_keys: crate::lfo::LfoKeys::new(3),
            lfo4_keys: crate::lfo::LfoKeys::new(4),
            lfo1_level: crate::lfo::LfoLevel::new(1),
            lfo2_level: crate::lfo::LfoLevel::new(2),
            lfo3_level: crate::lfo::LfoLevel::new(3),
            lfo4_level: crate::lfo::LfoLevel::new(4),
            velocity_curve: velocity_curve(),
            velocity_floor: velocity_floor(),
        }
    }
}

impl std::fmt::Debug for Ca72Params {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ca72Params")
            .field("noise_seed", &self.seed())
            .field("controls", &self.controls())
            .finish()
    }
}

/// The standalone's TEMPO for a new computer (the owner, 2026-10-18: "default 120").
pub const DEFAULT_TEMPO: f64 = 120.0;
/// How slow and fast TEMPO goes, beats a minute.
pub const TEMPO_RANGE: (f64, f64) = (30.0, 300.0);

impl Ca72Params {
    /// Where MPE's zones come from on this computer (decisions.md R-MPE; `settings.rs`).
    pub fn mpe_mode(&self) -> crate::expression::Mode {
        crate::expression::Mode::of_code(self.mpe.load(Ordering::Relaxed))
    }

    pub fn set_mpe_mode(&self, mode: crate::expression::Mode) {
        self.mpe.store(mode.code(), Ordering::Relaxed);
    }

    /// Where a timbre rests on this computer (decisions.md R-MPE; `settings.rs`).
    pub fn timbre_rest(&self) -> crate::expression::TimbreRest {
        crate::expression::TimbreRest::from_index(self.timbre_rest.load(Ordering::Relaxed))
    }

    pub fn set_timbre_rest(&self, rest: crate::expression::TimbreRest) {
        self.timbre_rest.store(rest.index(), Ordering::Relaxed);
    }

    /// The standalone's TEMPO, beats a minute.
    pub fn tempo(&self) -> f64 {
        let t = f64::from_bits(self.tempo.load(Ordering::Relaxed));
        if t.is_finite() {
            t.clamp(TEMPO_RANGE.0, TEMPO_RANGE.1)
        } else {
            DEFAULT_TEMPO
        }
    }

    pub fn set_tempo(&self, bpm: f64) {
        let t = if bpm.is_finite() {
            bpm.clamp(TEMPO_RANGE.0, TEMPO_RANGE.1)
        } else {
            DEFAULT_TEMPO
        };
        self.tempo.store(t.to_bits(), Ordering::Relaxed);
    }

    /// Whether this is the standalone application ([`Ca72Params::standalone`]).
    pub fn is_standalone(&self) -> bool {
        self.standalone.load(Ordering::Relaxed)
    }

    pub fn set_standalone(&self, standalone: bool) {
        if self.is_standalone() != standalone {
            self.standalone.store(standalone, Ordering::Relaxed);
        }
    }

    /// The arpeggiator's settings, as its parameters have them.
    pub fn arp(&self) -> crate::arp::Settings {
        crate::arp::Settings {
            on: self.arp_on.value(),
            latch: self.arp_latch.value(),
            direction: self.arp_direction.value().into(),
            step: crate::arp::RATES[self.arp_rate.value().to_index()].1,
            octaves: self.arp_octaves.value().clamp(ARP_OCTAVES.0, ARP_OCTAVES.1) as u8,
            gate: f64::from(self.arp_gate.value()) / 100.0,
            swing: f64::from(self.arp_swing.value()) / 100.0,
        }
    }

    /// Oscillators 2's and 3's TIMED, DIVISION and RANGE (decisions.md R-OSC-TIMED), by the
    /// oscillator's index (1, 2); none for oscillator 1, which is never timed.
    pub fn timed_params(
        &self,
        n: usize,
    ) -> Option<(&BoolParam, &EnumParam<Division>, &EnumParam<Footage>)> {
        match n {
            1 => Some((&self.osc2_timed, &self.osc2_division, &self.osc2_range)),
            2 => Some((&self.osc3_timed, &self.osc3_division, &self.osc3_range)),
            _ => None,
        }
    }

    /// Each oscillator's timed cycle, if its TIMED is on (whatever its RANGE is: the engine
    /// applies it at LO only); never oscillator 1's.
    pub fn timed(&self) -> [Option<crate::timed::Length>; 3] {
        std::array::from_fn(|n| {
            let (on, division, _) = self.timed_params(n)?;
            on.value()
                .then(|| crate::timed::DIVISIONS[division.value().to_index()].1)
        })
    }

    /// Whether oscillator `n` (0, 1, 2) is timed now: its TIMED on and its RANGE at LO (at any
    /// other RANGE it does nothing; oscillator 1, never).
    pub fn timed_now(&self, n: usize) -> bool {
        self.timed_params(n)
            .is_some_and(|(on, _, range)| on.value() && range.value() == Footage::Lo)
    }

    /// How the FREQUENCY knobs' dials read on this computer.
    pub fn freq_law(&self) -> Law {
        match self.freq_law.load(Ordering::Relaxed) {
            1 => Law::Hardware,
            _ => Law::Semitones,
        }
    }

    pub fn set_freq_law(&self, law: Law) {
        let v = match law {
            Law::Semitones => 0,
            Law::Hardware => 1,
        };
        self.freq_law.store(v, Ordering::Relaxed);
    }

    pub fn seed(&self) -> u64 {
        self.noise_seed.load(Ordering::Relaxed)
    }

    /// Oscillator `n`'s (0 to 2) waveform's parameters: STEPPED or CONTINUOUS, its shape, its
    /// MOD and MOD's direction (decisions.md R-CONTINUOUS-WAVE).
    pub fn shape_params(
        &self,
        n: usize,
    ) -> (
        &EnumParam<WaveMode>,
        &FloatParam,
        &BoolParam,
        &EnumParam<ShapeModDirection>,
    ) {
        match n {
            0 => (
                &self.osc1_shape_mode,
                &self.osc1_shape,
                &self.osc1_shape_mod,
                &self.osc1_shape_mod_dir,
            ),
            1 => (
                &self.osc2_shape_mode,
                &self.osc2_shape,
                &self.osc2_shape_mod,
                &self.osc2_shape_mod_dir,
            ),
            _ => (
                &self.osc3_shape_mode,
                &self.osc3_shape,
                &self.osc3_shape_mod,
                &self.osc3_shape_mod_dir,
            ),
        }
    }

    /// LFO `n`'s (0 to 3; past 3, the fourth) parameters (decisions.md R-LFO).
    pub fn lfo(&self, n: usize) -> &crate::lfo::LfoParams {
        match n {
            0 => &self.lfo1,
            1 => &self.lfo2,
            2 => &self.lfo3,
            _ => &self.lfo4,
        }
    }

    /// LFO `n`'s (0 to 3; past 3, the fourth) POLARITY, REVERSE and ON (R-LFO's corrections).
    pub fn lfo_keys(&self, n: usize) -> &crate::lfo::LfoKeys {
        match n {
            0 => &self.lfo1_keys,
            1 => &self.lfo2_keys,
            2 => &self.lfo3_keys,
            _ => &self.lfo4_keys,
        }
    }

    /// LFO `n`'s (0 to 3; past 3, the fourth) LEVEL (R-LFO's addendum).
    pub fn lfo_level(&self, n: usize) -> &crate::lfo::LfoLevel {
        match n {
            0 => &self.lfo1_level,
            1 => &self.lfo2_level,
            2 => &self.lfo3_level,
            _ => &self.lfo4_level,
        }
    }

    /// Macro `n`'s (0 to 5; past 5, the sixth) parameter (decisions.md R-MACROS).
    pub fn macro_param(&self, n: usize) -> &FloatParam {
        match n {
            0 => &self.macro1,
            1 => &self.macro2,
            2 => &self.macro3,
            3 => &self.macro4,
            4 => &self.macro5,
            _ => &self.macro6,
        }
    }

    /// Whether oscillator `n`'s waveform is CONTINUOUS.
    pub fn continuous(&self, n: usize) -> bool {
        self.shape_params(n).0.value() == WaveMode::Continuous
    }

    /// The engine's controls from the parameters (dial units in, the voice's 0..1 out).
    pub fn controls(&self) -> Controls {
        self.controls_with(&|p: &FloatParam| p.value())
    }

    /// The engine's controls with each knob where `knob` has it (its dial's units): the
    /// parameter's value, or where the voices have it while it glides (`crate::smooth::Smooth`;
    /// decisions.md R34, R-KNOB-GLIDE). Every other conversion is [`Ca72Params::controls`]'.
    pub fn controls_with(&self, knob: &dyn Fn(&FloatParam) -> f32) -> Controls {
        let value = |p: &FloatParam| f64::from(knob(p));
        let ten = |p: &FloatParam| value(p) / 10.0;
        let freq = |p: &FloatParam| (value(p) + 7.5) / 15.0;
        let osc = [
            OscPanel {
                range: self.osc1_range.value().into(),
                waveform: self.osc1_waveform.value().into(),
                on: self.osc1_on.value(),
                volume: ten(&self.osc1_volume),
                freq: ca72::tuning::FREQ_CENTRE,
            },
            OscPanel {
                range: self.osc2_range.value().into(),
                waveform: self.osc2_waveform.value().into(),
                on: self.osc2_on.value(),
                volume: ten(&self.osc2_volume),
                freq: freq(&self.osc2_frequency),
            },
            OscPanel {
                range: self.osc3_range.value().into(),
                waveform: self.osc3_waveform.value().into(),
                on: self.osc3_on.value(),
                volume: ten(&self.osc3_volume),
                freq: freq(&self.osc3_frequency),
            },
        ];
        let panel = Panel {
            osc,
            osc3_control: self.osc3_control.value(),
            cutoff: (value(&self.cutoff) + 5.0) / 10.0,
            emphasis: ten(&self.emphasis),
            contour_amount: ten(&self.contour_amount),
            keyboard_control_1: self.keyboard_control_1.value(),
            keyboard_control_2: self.keyboard_control_2.value(),
            filter_contour: ContourKnobs {
                attack: ten(&self.filter_attack),
                decay: ten(&self.filter_decay),
                sustain: ten(&self.filter_sustain),
            },
            loudness_contour: ContourKnobs {
                attack: ten(&self.loudness_attack),
                decay: ten(&self.loudness_decay),
                sustain: ten(&self.loudness_sustain),
            },
            glide: ten(&self.glide),
            glide_on: self.glide_on.value(),
            glide_long: self.glide_range.value() == GlideRange::Long,
            decay: self.decay_on.value(),
            noise_on: self.noise_on.value(),
            noise_volume: ten(&self.noise_volume),
            noise_pink: self.noise_type.value() == NoiseColor::Pink,
            mod_mix: ten(&self.mod_mix),
            osc_mod: self.osc_mod.value(),
            filter_mod: self.filter_mod.value(),
            filter_hi: self.filter_mode.value() == FilterMode::Hi,
            pitch_wheel: value(&self.pitch_wheel),
            mod_wheel: value(&self.mod_wheel),
            ext_volume: ten(&self.ext_volume),
            ext_on: self.ext_on.value(),
            a440: self.a440.value(),
            tune: value(&self.tune) / 2.5,
            quality: Quality::default(),
            priority: self.note_priority.value().into(),
            retrigger: self.retrigger.value(),
            shape: std::array::from_fn(|n| {
                let (mode, shape, modulated, direction) = self.shape_params(n);
                Shape {
                    continuous: mode.value() == WaveMode::Continuous,
                    at: ca72::voice::shape_snapped(value(shape) * ca72::voice::SHAPE_PLACES),
                    modulated: modulated.value(),
                    direction: direction.value().into(),
                }
            }),
        };
        Controls {
            panel,
            volume: ten(&self.volume),
            main_output: self.main_output.value(),
            bend_range: value(&self.midi_bend_range),
            power: !self.bypass.value(),
            poly: self.poly.value(),
            voices: usize::try_from(self.voices.value()).unwrap_or(POLY_VOICES.1),
            entropy: value(&self.entropy) / 100.0,
            spread: value(&self.spread) / 100.0,
            inner: value(&self.inner) / 100.0,
            placement: self.placement.value().into(),
            unison: self.unison.value(),
            double: if self.doubled.value() {
                (value(&self.double) / 100.0).max(DOUBLED_AT_NONE)
            } else {
                0.0
            },
            drive: value(&self.drive),
            level: value(&self.level),
            auto_gain: self.auto_gain.value(),
            // The knob's travel through its taper (decisions.md R8).
            feedback: ca72::voice::feedback_law(value(&self.feedback) / 10.0),
            lock: self.lock.value(),
            potato: self.quality.value() == QualityMode::Lo,
            ultra: self.quality.value() == QualityMode::Ultra,
            reach: Reach {
                pressure_mod: value(&self.pressure_mod) / 100.0,
                pressure_filter: value(&self.pressure_filter),
                timbre_mod: value(&self.timbre_mod) / 100.0,
                timbre_filter: value(&self.timbre_filter),
            },
            timed: self.timed(),
            lfo: crate::lfo::controls(self, knob),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The host reads DOUBLE's amount as the strip does, DETUNE in cents (Off at none), and
    /// takes it back so; SPREAD's is the strip's WIDTH. Their ids are as they were (the
    /// CA-74's test).
    #[test]
    fn double_reads_in_cents_and_spread_is_width() {
        let p = Ca72Params::default();
        let d = &p.double;
        let at = |percent: f32| d.normalized_value_to_string(d.preview_normalized(percent), true);
        assert_eq!(at(0.0), "Off");
        assert_eq!(at(60.0), "60 cents");
        assert_eq!(at(7.2), "7.2 cents");
        let back = |s: &str| {
            d.string_to_normalized_value(s)
                .map(|n| (d.preview_plain(n) * 1000.0).round() / 1000.0)
        };
        assert_eq!(back("60 cents"), Some(60.0));
        assert_eq!(back("60 \u{a2}"), Some(60.0));
        assert_eq!(back("7.2"), Some(7.2));
        assert_eq!(back("off"), Some(0.0));
        assert_eq!(back("140"), Some(100.0), "past 100 cents: the most");
        assert_eq!((d.name(), p.spread.name()), ("Double Detune", "Width"));
        assert_eq!(p.inner.name(), "Inner");
        let ids: Vec<String> = p.param_map().into_iter().map(|(id, ..)| id).collect();
        assert!(ids.iter().any(|i| i == "double") && ids.iter().any(|i| i == "spread"));
        assert!(ids.iter().any(|i| i == "inner"));
    }

    /// The host reads FREQUENCY in semitones from the centre (cents under one) and takes them
    /// back so, the value the pot's position as every version saved it (decisions.md
    /// R54): 0.1.x's +5 is its pot five sixths along, +5.52 semitones on oscillator 2.
    #[test]
    fn frequency_reads_in_semitones_and_keeps_the_pot() {
        let p = Ca72Params::default();
        let f = &p.osc2_frequency;
        let shown = |plain: f32| f.normalized_value_to_string(f.preview_normalized(plain), true);
        assert_eq!(shown(0.0), "0 semitones");
        assert_eq!(shown(5.0), "+5.52 semitones");
        assert_eq!(shown(-7.5), "-8.56 semitones");
        assert_eq!(shown(0.026), "+2.9 cents");
        let back = |s: &str| {
            let n = f.string_to_normalized_value(s)?;
            Some(Pot::Two.semitones(wiper_of(f.preview_plain(n))))
        };
        let near = |got: Option<f64>, want: f64| got.is_some_and(|g| (g - want).abs() < 1e-4);
        assert!(near(back("7"), 7.0));
        assert!(near(back("+7.00 semitones"), 7.0));
        assert!(near(back("-5 st"), -5.0));
        assert!(near(back("+3.4 cents"), 0.034));
        assert!(near(back("12c"), 0.12));
        assert!(
            near(back("20"), Pot::Two.semitones(1.0)),
            "beyond the pot: its end"
        );
        assert_eq!(back("up"), None);
        // The engine takes the pot's position as the value has it, whatever the law.
        let c = p.controls();
        assert_eq!(c.panel.osc[1].freq, 0.5);
        assert_eq!(
            (p.freq_law(), Law::default()),
            (Law::Semitones, Law::Semitones)
        );
        p.set_freq_law(Law::Hardware);
        assert_eq!(p.freq_law(), Law::Hardware);
    }

    /// The arpeggiator's parameters (R-ARP): their ids after every other, off by default at a
    /// sixteenth, UP, an octave, GATE 50 % and no SWING; RATE's choices are the scheduler's
    /// CONTINUOUS's travel as the panel draws it is the engine's (decisions.md R-CONTINUOUS-WAVE);
    /// its shape shows its places' names, blends and duty cycles, and takes either typed; a new
    /// instance is STEPPED, at SAWTOOTH, MOD off and clockwise.
    #[test]
    fn continuous_waveforms_are_off_by_default_and_their_travel_the_engines() {
        use ca72::voice::SHAPE_PLACES;
        assert_eq!(ca72_panel::art::RECT_STEPS, ca72::voice::RECT_STEPS);
        let p = Ca72Params::default();
        for n in 0..3 {
            let (mode, shape, modulated, direction) = p.shape_params(n);
            assert_eq!(mode.value(), WaveMode::Stepped);
            assert!((f64::from(shape.value()) * SHAPE_PLACES - 2.0).abs() < 1e-6);
            assert!(!modulated.value());
            assert_eq!(direction.value(), ShapeModDirection::Clockwise);
            assert!(!p.continuous(n));
        }
        assert!(p.controls().panel.shape.iter().all(|s| !s.continuous));
        let at = |k: f64| (k / SHAPE_PLACES) as f32;
        let (one, three) = (&p.osc1_shape, &p.osc3_shape);
        assert_eq!(one.normalized_value_to_string(at(2.0), true), "Sawtooth");
        assert_eq!(
            three.normalized_value_to_string(at(2.0), true),
            "Reverse Sawtooth"
        );
        assert_eq!(
            three.normalized_value_to_string(at(1.0), true),
            "Reverse Shark Tooth"
        );
        assert_eq!(
            one.normalized_value_to_string(at(2.4), true),
            "Sawtooth to Square 40 %"
        );
        assert_eq!(one.normalized_value_to_string(at(3.0), true), "Square 48 %");
        assert_eq!(
            one.normalized_value_to_string(at(4.0), true),
            "Wide Rectangle 29 %"
        );
        // The calibrated circuit's 220 Hz MIX duty at the wide selector, not the old
        // profile's rounded 30 %. Display rounding cannot preserve the exact selector
        // position; typing the measured duty can (docs/calibration/pulse-source-20261022.md).
        let t = one
            .string_to_normalized_value("29.378574 %")
            .expect("a duty cycle");
        assert!((f64::from(t) * SHAPE_PLACES - 4.0).abs() < 2e-2, "{t}");
        let t = three
            .string_to_normalized_value("reverse sawtooth")
            .expect("a name");
        assert_eq!(t, at(2.0));
    }

    /// note values in order, its names a host shows.
    #[test]
    fn the_arpeggiator_is_off_by_default_its_ids_last() {
        let p = Ca72Params::default();
        let ids: Vec<String> = p.param_map().into_iter().map(|(id, ..)| id).collect();
        let at = |id: &str| ids.iter().position(|i| i == id).expect(id);
        for id in [
            "arp_on",
            "arp_latch",
            "arp_direction",
            "arp_rate",
            "arp_octaves",
            "arp_gate",
            "arp_swing",
        ] {
            assert!(at(id) > at("retrigger"), "{id}");
        }
        let a = p.arp();
        assert!(!a.on && !a.latch);
        assert_eq!(a.direction, crate::arp::Direction::Up);
        assert_eq!((a.step, a.octaves, a.gate, a.swing), (0.25, 1, 0.5, 0.0));
        assert_eq!(ArpRate::variants().len(), crate::arp::RATES.len());
        assert_eq!(ArpRate::variants()[1], "1/4 dotted");
        assert_eq!(ArpRate::variants()[2], "1/2 triplet");
        assert_eq!(
            ArpDirection::variants(),
            ["UP", "DOWN", "UP/DOWN", "RANDOM", "ORDER"]
        );
        assert_eq!(p.arp_latch.name(), "Arp Hold");
        assert_eq!(ARP_OCTAVES, (1, 5));
        assert_eq!(p.tempo(), DEFAULT_TEMPO);
        p.set_tempo(1000.0);
        assert_eq!(p.tempo(), TEMPO_RANGE.1);
    }

    /// The expression's reaches (R-MPE): their ids after every other, 0 by default (none, the
    /// sound as before), the engine told them in its units.
    #[test]
    fn the_expression_reaches_are_last_and_none_by_default() {
        let p = Ca72Params::default();
        let ids: Vec<String> = p.param_map().into_iter().map(|(id, ..)| id).collect();
        let at = |id: &str| ids.iter().position(|i| i == id).expect(id);
        for id in [
            "pressure_mod",
            "pressure_filter",
            "timbre_mod",
            "timbre_filter",
        ] {
            assert!(at(id) > at("arp_swing"), "{id}");
        }
        assert_eq!(p.controls().reach, Reach::default());
        let c = p.controls_with(&|k| {
            if std::ptr::eq(k, &p.pressure_mod) {
                -50.0
            } else if std::ptr::eq(k, &p.timbre_filter) {
                REACH_OCTAVES
            } else {
                k.value()
            }
        });
        assert_eq!(c.reach.pressure_mod, -0.5);
        assert_eq!(c.reach.timbre_filter, f64::from(REACH_OCTAVES));
        let shown =
            |q: &FloatParam, v: f32| q.normalized_value_to_string(q.preview_normalized(v), true);
        assert_eq!(shown(&p.pressure_mod, 100.0), "100 %");
        assert_eq!(shown(&p.pressure_filter, -1.5), "-1.50 octaves");
    }

    /// SCATTER's placement offers two choices, EVEN (the default) and CENTER, the strip's tabs
    /// in their order, their ids as they were; each the kit's placement of its name, never its
    /// EDGES (decisions.md R49). The host steps between the two.
    #[test]
    fn the_placement_offers_even_and_center() {
        let p = Ca72Params::default();
        assert_eq!(Scatter::variants(), ["EVEN", "CENTER"]);
        assert_eq!(Scatter::ids(), Some(&["even", "centre"][..]));
        assert_eq!(
            ca72_panel::strip::Bank::Placement.words(),
            Scatter::variants()
        );
        assert_eq!(Placement::from(Scatter::Even), Placement::Even);
        assert_eq!(Placement::from(Scatter::Centre), Placement::Centre);
        let q = &p.placement;
        assert_eq!((q.name(), q.value()), ("Scatter Placement", Scatter::Even));
        assert_eq!(q.step_count(), Some(1));
        let shown = |n: f32| q.normalized_value_to_string(n, false);
        assert_eq!((shown(0.0), shown(1.0)), ("EVEN".into(), "CENTER".into()));
        assert_eq!(p.controls().placement, Placement::Even);
    }
}
