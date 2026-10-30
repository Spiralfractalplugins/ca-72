//! The instrument, first voice (v0): the parts derived and tested so far, wired as the
//! instrument wires them (docs/circuit/voice.md).
//!
//! - **Keyboard** (board 2's keyboard circuit, [`crate::keyboard`]): 44 keys; the lowest
//!   and highest held join the pitch bus to the string (lowest-note priority follows; the
//!   plug-in's NOTE PRIORITY HIGH or LAST hands it the key it chooses alone: [`Priority`]); the
//!   trigger contact is closed while any key is held (single triggering); the hold,
//!   GLIDE, and the output's load (the oscillators' and the filter's keyboard inputs). A
//!   key's pitch contact opens [`RELEASE_LEAD`] after its trigger contact (A18).
//! - **Oscillators 1, 2 and 3** (board 1, each tuned by Folkman's 1973 procedure on the
//!   model, its keys played through the keyboard circuit): oscillator 2 with its FREQUENCY
//!   control, oscillator 3 with its FREQUENCY control, OSC. 3 CONTROL (its control stage IC8)
//!   and its reverse sawtooth (Q37). Each through its WAVEFORM switch (dwg 1448: triangle,
//!   shark tooth through R030 47K and R031 10K (oscillator 3: the reverse sawtooth),
//!   sawtooth, rectangle at 0, -1.5 and -2.5 V of width bias) and the mixer's VOLUME pot
//!   (25K linear) and 33K onto the bus, as a Norton source. The external input (33K) and the
//!   noise (11K) are switched off and load the bus.
//! - **Filter** (board 4) with its control node: CUTOFF (5K linear across +/-10 V) through
//!   R55 200K, KEYBOARD CONTROL 1 and 2 through R53 300K and R54 150K, AMOUNT OF CONTOUR (5K
//!   linear from the filter contour) through R74 47K; EMPHASIS (R14, 50K reverse audio,
//!   a rheostat).
//! - **VCA** (board 4, balanced by the factory procedure) on the loudness contour.
//! - **Contours** (board 2), at half the output rate, interpolated.
//!
//! The front panel's pot tapers are generic audio and reverse audio laws (assumptions A9;
//! GLIDE is "R2 5M AUDIO" on Figure 9-17).
//! The output is the main output's voltage across a 10K load, scaled 5 V to 1.0.

use crate::a440::A440;
use crate::contour::{ContourCircuit, Contours, Controls, Panel as ContourPanel};
use crate::expo::Input;
use crate::keyboard::{Keyboard, KeyboardCircuit, Load};
use crate::modulation::{LineLoads, MOD_WHEEL_FULL, Modulation, mod_wheel_r, pitch_wheel_volts};
use crate::noise::{Gauss, Noise, NoiseCircuit, NoiseLoads, NoiseOut};
use crate::preamp::{InLoop, Preamp, PreampCircuit};
use crate::revsaw::{RevSaw, RevSawCircuit, RevSawOut};
use crate::tuning::{
    Buses, FREQ_CENTRE, HIGH_A, LOW_A, OPEN_BUS, Osc, Range, SECOND_A, Tuning, folkman_1973_buses,
    folkman_1973_osc_buses, osc_drive_with,
};
use crate::vca::{OversampledVca, Vca, VcaCircuit, VcaDrive};
use crate::vcf::{ExpoTable, FilterExpo, Vcf};
use crate::vco::{Timed, Vco, VcoOut};

#[cfg(test)]
mod loop_lab;

/// The WAVEFORM switch's positions. The second is the shark tooth on oscillators 1 and 2
/// and the reverse sawtooth on oscillator 3 (dwg 1448); either name selects it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waveform {
    Triangle,
    SharkTooth,
    ReverseSawtooth,
    Sawtooth,
    Square,
    WideRectangle,
    NarrowRectangle,
}

impl Waveform {
    /// The width bias the switch's second pole sets (R034, R033, R032 from ground to
    /// -10 V), V.
    pub fn width(self) -> f64 {
        match self {
            Waveform::WideRectangle => -1.5,
            Waveform::NarrowRectangle => -2.5,
            _ => 0.0,
        }
    }
}

/// Whether a waveform is one of the rectangle's three positions.
pub fn is_rectangle(w: Waveform) -> bool {
    matches!(
        w,
        Waveform::Square | Waveform::WideRectangle | Waveform::NarrowRectangle
    )
}

/// The width bias at which CONTINUOUS's travel ends (decisions.md R-CONTINUOUS-WAVE): the
/// narrowest rectangle it gives, a little short of where the pulse stops coming (the reset
/// no longer lifts the ramp past the comparator's rising threshold), V.
pub const RECT_FLOOR: f64 = -3.45;

/// The rectangle's stretch of the travel past SQUARE in the switch's steps: square to wide,
/// wide to narrow, then on past narrow to the floor at the same volts a step as from wide to
/// narrow (1 V).
pub const RECT_STEPS: f64 = 2.0 + (-RECT_FLOOR - 2.5);

/// CONTINUOUS's whole travel in the switch's steps: TRIANGLE (0), the second place (1),
/// SAWTOOTH (2), SQUARE (3), WIDE (4), NARROW (5), then on to the floor.
pub const SHAPE_PLACES: f64 = 3.0 + RECT_STEPS;

/// An oscillator's waveform in the plug-in's CONTINUOUS mode, which the instrument does not have
/// (decisions.md R-CONTINUOUS-WAVE): its WAVEFORM switch turned without steps through the
/// whole travel, each shape blending into the next ([`shape_source`]) and past SQUARE the
/// rectangle's width bias anywhere down to [`RECT_FLOOR`] (the switch's second pole's three
/// taps a pot); with MOD, the place also moves on with the modulation line (taken through a
/// buffer: it loads nothing). STEPPED is the instrument's switch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shape {
    pub continuous: bool,
    /// Where the switch is, in its steps: 0 to [`SHAPE_PLACES`].
    pub at: f64,
    pub modulated: bool,
    pub direction: ModDirection,
}

impl Shape {
    pub const STEPPED: Shape = Shape {
        continuous: false,
        at: 2.0,
        modulated: false,
        direction: ModDirection::Onward,
    };
}

/// Which way MOD moves a CONTINUOUS shape from where its switch is (the owner, 2026-10-20: "a
/// command click ... to reverse the direction ... and then perhaps we could even have a third
/// mode where it travels to the left and to the right of it"): on (clockwise), back
/// (anticlockwise), or either way about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModDirection {
    #[default]
    Onward,
    Back,
    Both,
}

/// The modulation line as MOD takes it, a sample's: its depth (the line's gain against its
/// gain with the wheel fully forward, 0..1) and its swing (-1..1 over oscillator 3's
/// triangle at MODULATION MIX's oscillator 3; the noise's beyond, now and then).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ModSwing {
    pub depth: f64,
    pub swing: f64,
}

impl ModSwing {
    /// The share of the whole travel it moves a shape, signed (on is up): onward or back, as
    /// deep as the wheel is forward, from none at the swing's bottom to the depth at its top;
    /// either way, half that about none.
    pub fn share(self, direction: ModDirection) -> f64 {
        let one_way = (0.5 * self.depth * (1.0 + self.swing)).clamp(0.0, 1.0);
        match direction {
            ModDirection::Onward => one_way,
            ModDirection::Back => -one_way,
            ModDirection::Both => (0.5 * self.depth * self.swing).clamp(-0.5, 0.5),
        }
    }
}

impl Default for Shape {
    fn default() -> Self {
        Self::STEPPED
    }
}

/// Where a CONTINUOUS switch's shape is: its place, and with MOD moved its way by the
/// modulation's share of the whole travel, held to the travel, [`shape_snapped`].
pub fn shape_at(s: Shape, m: ModSwing) -> f64 {
    let at = if s.modulated {
        s.at + m.share(s.direction) * SHAPE_PLACES
    } else {
        s.at
    };
    shape_snapped(at.clamp(0.0, SHAPE_PLACES))
}

/// A place along CONTINUOUS's travel (its steps) within a thousandth of a step of one of the
/// switch's six places, that place exactly (a host's f32, or a preset's four decimals of the
/// travel: three ten-thousandths of a step at most); else where it is.
pub fn shape_snapped(at: f64) -> f64 {
    let near = at.round();
    if (at - near).abs() < 1e-3 && (0.0..=5.0).contains(&near) {
        near
    } else {
        at
    }
}

/// The rectangle's width bias with the switch `at` (its steps): past SQUARE, straight between
/// the switch's own three (which it gives exactly) and on to the floor; the square's below.
pub fn shape_bias(at: f64) -> f64 {
    let x = (at - 3.0).clamp(0.0, RECT_STEPS);
    if x <= 1.0 { -1.5 * x } else { -1.5 - (x - 1.0) }
}

/// Where along the travel (its steps) the rectangle has width bias `bias`: [`shape_bias`]'s
/// inverse past SQUARE.
pub fn bias_at(bias: f64) -> f64 {
    let b = bias.clamp(RECT_FLOOR, 0.0);
    3.0 + if b >= -1.5 { -b / 1.5 } else { 1.0 - 1.5 - b }
}

/// The rectangle's duty cycle (its share of a cycle high) at a width bias, as the light voice
/// plays it (the full voice's within about a percent: decisions.md R-CONTINUOUS-WAVE).
pub fn rect_duty(bias: f64) -> f64 {
    f64::from(crate::light::rect_law(bias).0)
}

/// The waveform the switch's place `k` (0 to 3) is on oscillator `n` in CONTINUOUS, as its own
/// switch names it, for what the place is made from; oscillator 3's second place, its reverse
/// shark tooth, which the switch does not have, `None` (the owner: "position two on oscillator
/// three would become position three and then position two would then turn into a reverse
/// Sharktooth").
pub fn shape_place(n: usize, k: usize) -> Option<Waveform> {
    Some(match (k, n == 2) {
        (0, _) => Waveform::Triangle,
        (1, false) => Waveform::SharkTooth,
        (1, true) => return None,
        (2, false) => Waveform::Sawtooth,
        (2, true) => Waveform::ReverseSawtooth,
        _ => Waveform::Square,
    })
}

/// A contour generator's three knobs, 0..1 (the panel's 0..10).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContourKnobs {
    pub attack: f64,
    pub decay: f64,
    pub sustain: f64,
}

/// One oscillator's controls: RANGE, WAVEFORM, its mixer switch and VOLUME, and FREQUENCY
/// (oscillators 2 and 3): its pot's wiper, 0..1 of its track, centred at 0.5. How the knob's
/// dial sets the pot is the player's choice ([`crate::freq_dial`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OscPanel {
    pub range: Range,
    pub waveform: Waveform,
    pub on: bool,
    pub volume: f64,
    pub freq: f64,
}

/// How closely the model follows its circuit, against what it costs (history.md, "Three
/// quality modes"): No Compromises within 1e-7 of full scale of the model as it stands;
/// High Fidelity within -120 dBFS, 0.01 cent and a sample of it, each measured on its own
/// path (`ca72-lab hifi`); Potato as close as a hundred voices at once allow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Quality {
    #[default]
    NoCompromises,
    HighFidelity,
    Potato,
}

/// Which of the keys held sounds (not the instrument's: decisions.md R50). The keyboard
/// circuit's own is the lowest; HIGH and LAST hand it the key chosen alone, as if it were the
/// only key down. In this order, as the toggle's positions run from left to right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Priority {
    /// The lowest key: the circuit's contacts as drawn (the lowest and highest keys held).
    #[default]
    Low,
    /// The key pressed last; once it is let go, the last of those still held.
    Last,
    /// The highest key.
    High,
}

/// The front panel and left hand controller: knobs 0..1 (the panel's 0..10; CUTOFF's
/// -5..+5 as 0..1), switches as booleans.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Panel {
    /// Oscillators 1, 2 and 3.
    pub osc: [OscPanel; 3],
    /// OSC. 3 CONTROL: on, the keyboard and the other inputs reach oscillator 3.
    pub osc3_control: bool,
    pub cutoff: f64,
    pub emphasis: f64,
    pub contour_amount: f64,
    pub keyboard_control_1: bool,
    pub keyboard_control_2: bool,
    pub filter_contour: ContourKnobs,
    pub loudness_contour: ContourKnobs,
    /// GLIDE (5M, a rheostat) and the left hand controller's GLIDE and DECAY switches.
    pub glide: f64,
    pub glide_on: bool,
    /// LONG: the hold capacitor C6 five times larger (decisions.md R-GLIDE-LONG).
    pub glide_long: bool,
    pub decay: bool,
    /// The noise's mixer switch and VOLUME, and the NOISE switch (SW14): WHITE (false) sends
    /// white noise to the mixer and pink to the modulation, PINK (true) pink to the mixer
    /// (through R50) and red to the modulation.
    pub noise_on: bool,
    pub noise_volume: f64,
    pub noise_pink: bool,
    /// MODULATION MIX: 0 oscillator 3 alone (counterclockwise), 1 the noise alone.
    pub mod_mix: f64,
    /// OSCILLATOR MODULATION and FILTER MODULATION.
    pub osc_mod: bool,
    pub filter_mod: bool,
    /// FILTER MODE, which the original does not have (decisions.md R46): HI, the hardware
    /// reference's high-pass (the mixer's output less the filter's); LO, the filter as drawn.
    pub filter_hi: bool,
    /// The wheels: pitch -1..1 (0 in its detent), MODULATION 0..1 (fully forward).
    pub pitch_wheel: f64,
    pub mod_wheel: f64,
    /// EXTERNAL INPUT: its VOLUME (R9, 1M audio, ahead of the preamplifier) and its mixer
    /// switch (SW10).
    pub ext_volume: f64,
    pub ext_on: bool,
    /// A-440 (SW18).
    pub a440: bool,
    /// TUNE, -1..1 from its centre (where the factory's tuning leaves it): R1 5K between
    /// +10 V and R22 5.1K, its wiper through R21 into each oscillator (5.35: "4-6
    /// semitones" over its travel).
    pub tune: f64,
    /// The model's quality mode (not a control of the instrument's).
    pub quality: Quality,
    /// Which key held sounds (not a control of the instrument's either).
    pub priority: Priority,
    /// RETRIGGER (not the instrument's: decisions.md R50): each key pressed opens the
    /// trigger contact for [`RETRIGGER_GAP`] before it comes down, so that the contours start
    /// again even with other keys held (off: single triggering, the instrument's).
    pub retrigger: bool,
    /// Each oscillator's waveform STEPPED (the instrument's switch) or CONTINUOUS, with or
    /// without MOD (not the instrument's: decisions.md R-CONTINUOUS-WAVE).
    pub shape: [Shape; 3],
}

impl Panel {
    /// Whether oscillator `n`'s shape follows the modulation line: CONTINUOUS, its MOD on.
    pub fn shape_mod_on(&self, n: usize) -> bool {
        let s = self.shape[n];
        s.continuous && s.modulated
    }

    /// Whether any oscillator's shape follows the modulation line.
    pub fn any_shape_mod(&self) -> bool {
        (0..3).any(|n| self.shape_mod_on(n))
    }
}

/// TUNE's wiper voltage over half its travel: R1 spans 10 V less R22's 5.05 V.
pub const TUNE_HALF_SPAN: f64 = 0.5 * (10.0 - 10.0 * 5.1 / 10.1);

impl Default for Panel {
    /// Oscillator 1 alone on a sawtooth at 8'; 2 and 3 off, their FREQUENCY centred.
    fn default() -> Self {
        let osc = |on| OscPanel {
            range: Range::R8,
            waveform: Waveform::Sawtooth,
            on,
            volume: 0.8,
            freq: FREQ_CENTRE,
        };
        Panel {
            osc: [osc(true), osc(false), osc(false)],
            osc3_control: true,
            cutoff: 0.5,
            emphasis: 0.0,
            contour_amount: 0.5,
            keyboard_control_1: true,
            keyboard_control_2: false,
            filter_contour: ContourKnobs {
                attack: 0.0,
                decay: 0.4,
                sustain: 0.5,
            },
            loudness_contour: ContourKnobs {
                attack: 0.0,
                decay: 0.4,
                sustain: 0.8,
            },
            glide: 0.0,
            glide_on: false,
            glide_long: false,
            decay: true,
            noise_on: false,
            noise_volume: 0.8,
            noise_pink: false,
            mod_mix: 0.0,
            osc_mod: false,
            filter_mod: false,
            filter_hi: false,
            pitch_wheel: 0.0,
            mod_wheel: 0.0,
            ext_volume: 0.5,
            ext_on: false,
            a440: false,
            tune: 0.0,
            quality: Quality::NoCompromises,
            priority: Priority::Low,
            retrigger: false,
            shape: [Shape::STEPPED; 3],
        }
    }
}

/// The mixer's VOLUME pots (25K linear) as the hardware reference's oscillator 1 VOLUME
/// sets its wiper: the fraction of the track at the dial's 2, 4, 6 and 8 through which the
/// channel gives the reference's MIX level against 10 (-16.51, -9.42, -5.19 and -2.10 dB;
/// docs/calibration, session C). A linear track's are 0.2, 0.4, 0.6 and 0.8.
const VOLUME_TRACK: [(f64, f64); 6] = [
    (0.0, 0.0),
    (0.2, 0.1548),
    (0.4, 0.3777),
    (0.6, 0.6226),
    (0.8, 0.8453),
    (1.0, 1.0),
];

/// A knob's wiper as a fraction of its track, straight between a law's points (knob, track;
/// the knob from 0 to 1), each point's own value exactly.
fn track(points: &[(f64, f64)], knob: f64) -> f64 {
    let p = knob.clamp(0.0, 1.0);
    for w in points.windows(2) {
        let ((pa, ta), (pb, tb)) = (w[0], w[1]);
        if p == pb {
            return tb;
        }
        if p <= pb {
            return ta + (tb - ta) * (p - pa) / (pb - pa);
        }
    }
    1.0
}

/// A mixer VOLUME knob's wiper as a fraction of its track: straight between
/// [`VOLUME_TRACK`]'s points.
pub fn volume_track(volume: f64) -> f64 {
    track(&VOLUME_TRACK, volume)
}

/// AMOUNT OF CONTOUR's pot (R12, 5K linear) as the hardware reference's knob sets its wiper:
/// the fraction of the track at 2.5, 4, 5 and 7.5 at which the voice's filter moves as the
/// reference's does for the same contour (the filter self-oscillating; docs/calibration):
/// 0.606, 1.506 and 2.389 octaves for 1.465 V at 2.5, 5 and 7.5 (session J, CUTOFF -2), 2.88
/// octaves for 3.755 V at 4 (session L, CUTOFF -1). A linear track's are 0.25, 0.4, 0.5 and
/// 0.75.
const CONTOUR_AMOUNT_TRACK: [(f64, f64); 6] = [
    (0.0, 0.0),
    (0.25, 0.2080),
    (0.4, 0.3796),
    (0.5, 0.5111),
    (0.75, 0.7950),
    (1.0, 1.0),
];

/// The AMOUNT OF CONTOUR knob's wiper as a fraction of its track.
pub fn contour_amount_track(amount: f64) -> f64 {
    track(&CONTOUR_AMOUNT_TRACK, amount)
}

/// The contours' SUSTAIN pots (R18 and R19, 5K linear across +10 V) as the hardware
/// reference's knobs set their wipers: the fraction of the track at 2, 4, 5, 6 and 8 at which
/// each contour holds the reference's level (sessions N and F; docs/calibration), the two
/// contours' fractions averaged (they part by 0.015 at most). A linear track's are 0.2, 0.4,
/// 0.5, 0.6 and 0.8; the ends hold where the reference's do.
const SUSTAIN_TRACK: [(f64, f64); 7] = [
    (0.0, 0.0),
    (0.2, 0.1611),
    (0.4, 0.3735),
    (0.5, 0.4924),
    (0.6, 0.6185),
    (0.8, 0.8375),
    (1.0, 1.0),
];

/// A SUSTAIN knob's wiper as a fraction of its track.
pub fn sustain_track(sustain: f64) -> f64 {
    track(&SUSTAIN_TRACK, sustain)
}

/// AMOUNT OF CONTOUR (R12, 5K linear from the filter contour's output to GND, through
/// [`contour_amount_track`]) and R74 ([`crate::vcf::R74`]) into the control node: the input it
/// makes with the contour at `env_f` volts.
pub fn contour_input(amount: f64, env_f: f64) -> Input {
    let t = contour_amount_track(amount);
    Input {
        r: crate::vcf::R74 + 5e3 * t * (1.0 - t),
        v: env_f * t,
    }
}

/// CUTOFF FREQUENCY (R11, 5K linear across +-10 V) as the hardware reference's knob sets its
/// wiper: the fraction of the track at the dial's -4, -2, 2 and 4 at which the voice's filter
/// sits where the reference's does, each at its CUT CV 0 (docs/calibration, session E). The
/// dial runs -5 to 5 between the knob's stops (photographed), where the two agree, as at 0. A
/// straight track's are 0.1, 0.3, 0.7 and 0.9.
const CUTOFF_TRACK: [(f64, f64); 7] = [
    (0.0, 0.0),
    (0.1, 0.0582),
    (0.3, 0.2669),
    (0.5, 0.5),
    (0.7, 0.7304),
    (0.9, 0.9346),
    (1.0, 1.0),
];

/// The CUTOFF knob's wiper as a fraction of its track: straight between [`CUTOFF_TRACK`]'s
/// points.
pub fn cutoff_track(cutoff: f64) -> f64 {
    track(&CUTOFF_TRACK, cutoff)
}

/// The input a mixer channel's VOLUME pot (25K linear, its wiper through `r_series` to the
/// bus, near GND) puts on its source, ohm.
fn channel_load(volume: f64, r_series: f64) -> f64 {
    let p = volume_track(volume);
    let bottom = 25e3 * p;
    25e3 * (1.0 - p) + bottom * r_series / (bottom + r_series)
}

/// MOD's measure (decisions.md R-CONTINUOUS-WAVE): the modulation amplifier's output at the
/// middle of oscillator 3's triangle and half its swing, MODULATION MIX at oscillator 3, the
/// triangle at its switch's output under the mixer's channel (VOLUME at 8) and the mix's
/// pot: the triangle's extremes over the ramp's travel, the buffer's top to the reset's
/// threshold.
fn mod_reference(modulation: &Modulation) -> (f64, f64) {
    let (core, sh) = (
        crate::vco::CoreParams::default(),
        crate::vco::ShaperParams::default(),
    );
    let (top, bottom) = (
        sh.buf_gain * core.v_top0 + sh.buf_offset,
        sh.buf_gain * core.v_threshold + sh.buf_offset,
    );
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for k in 0..=2000 {
        let t = crate::vco::triangle(bottom + (top - bottom) * k as f64 / 2000.0);
        lo = lo.min(t);
        hi = hi.max(t);
    }
    let r_tri = 750.0 * 8200.0 / 8950.0;
    let z_ch = channel_load(0.8, 33e3);
    let (_, z_mix) = Modulation::source_loads(0.0);
    let z = z_ch * z_mix / (z_ch + z_mix);
    let m = modulation.mix(0.0);
    let amp = |t: f64| m.offset + m.gain_osc3 * t * z / (r_tri + z);
    let (a, b) = (amp(lo), amp(hi));
    (0.5 * (a + b), 0.5 * (a - b).abs())
}

/// The noise generator's loads for the panel: the mixer's channel and the modulation mix on
/// the outputs the NOISE switch gives them.
fn noise_loads(p: &Panel) -> NoiseLoads {
    let audio = channel_load(p.noise_volume, 11e3);
    let (modulation, _) = Modulation::source_loads(p.mod_mix);
    if p.noise_pink {
        NoiseLoads {
            white: f64::INFINITY,
            pink: 24e3 + audio,
            red: modulation,
        }
    } else {
        NoiseLoads {
            white: audio,
            pink: modulation,
            red: f64::INFINITY,
        }
    }
}

/// The white noise's level against oscillator 1's triangle at the output, as the factory set
/// R26: the service manual's triangle at "1 +- 3dB" (5.8) and noise at "-5 +- 3dB" (5.27;
/// Folkman: "-5dB maximum in the white position"), dB (assumptions A23).
pub const NOISE_BELOW_TRIANGLE: f64 = 6.0;

/// Both channels' VOLUME for that measurement: 10. Folkman's white noise is "-5dB maximum"
/// (the noise at its maximum), and 5.8's triangle ("1 +- 3dB") is what the model's output
/// gives at VOLUME 10 (+1.2 dB). At 10, R50 puts the pink 5 dB under the white, 2 dB below
/// Norlin's later 5.27 window for it (A23; board3.md B3-2).
pub const NOISE_CALIBRATION_VOLUME: f64 = 1.0;

/// The band the meter read the noise over: the audio band, the filter fully open (5.22's
/// CUTOFF fully clockwise, EMPHASIS 0; A23), Hz.
pub const NOISE_CALIBRATION_BAND: (f64, f64) = (20.0, 20e3);

/// Calibrates the noise source's density (R26's job): the white noise's current into the bus
/// through its channel against oscillator 1's triangle's through its channel, both at
/// [`NOISE_CALIBRATION_VOLUME`], the noise over [`NOISE_CALIBRATION_BAND`] (the continuous
/// model's response). `key`: the keyboard's voltage with low A held.
fn calibrate_noise(vco: &mut Vco, rate: f64, tuning: &Tuning, key: f64) -> f64 {
    let vol = NOISE_CALIBRATION_VOLUME;
    // The triangle's current through its channel, low A on 2' (440 Hz), 200 periods.
    let d = osc_drive_with(Osc::One, tuning, key, &Buses::RESTING, Range::R2);
    let i_in = d.apply(&mut vco.expo);
    vco.reset();
    let n = (200.0 * rate / 440.0) as usize;
    let mut samples = Vec::with_capacity(n);
    for k in 0..n + 1000 {
        let o = vco.tick(i_in, 0.0);
        if k >= 1000 {
            let (v, r) = waveform_source(Waveform::Triangle, &o, None);
            samples.push(channel(v, r, vol, true, 33e3).0);
        }
    }
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    let i_tri = (samples.iter().map(|i| (i - mean) * (i - mean)).sum::<f64>()
        / samples.len() as f64)
        .sqrt();
    // The white noise at pin 5 into its channel, per unit density.
    let loads = NoiseLoads {
        white: channel_load(vol, 11e3),
        pink: Modulation::source_loads(0.0).0,
        red: f64::INFINITY,
    };
    let Ok(nz) = Noise::new(NoiseCircuit::default(), loads, rate, 0) else {
        return 0.0;
    };
    let (model, _, out) = nz.model_of();
    let (f0, f1) = NOISE_CALIBRATION_BAND;
    let points = 400usize;
    let mut power = 0.0;
    let mut prev: Option<(f64, f64)> = None;
    for k in 0..=points {
        let f = f0 * crate::ulp::pow(f1 / f0, k as f64 / points as f64);
        let z = model.response(out[0], f);
        let pw = z.0 * z.0 + z.1 * z.1;
        if let Some((fp, pp)) = prev {
            power += 0.5 * (pw + pp) * (f - fp);
        }
        prev = Some((f, pw));
    }
    // Its channel: the pin's voltage (a follower behind C18) through the pot and R48 11K.
    let (i_unit, _) = channel(libm::sqrt(power), 0.0, vol, true, 11e3);
    i_tri * crate::ulp::pow(10.0, -NOISE_BELOW_TRIANGLE / 20.0) / i_unit
}

/// An audio taper (generic: 10 % at half rotation), as a fraction of the track.
pub fn audio_taper(p: f64) -> f64 {
    (crate::ulp::pow(81.0, p.clamp(0.0, 1.0)) - 1.0) / 80.0
}

/// The ATTACK and DECAY pots on Figure 9-17: 1M audio rheostats (the generic taper), ohm.
pub fn time_pot_drawn(p: f64) -> f64 {
    1e6 * audio_taper(p)
}

/// The hardware reference's ATTACK and DECAY at its dial's printed marks, as the pot's
/// resistance through which the CA-72's contour takes the reference's time: 10 to 90 % of the
/// attack, 90 to 50 % of the final decay (docs/calibration, sessions F and J). The marks, on
/// the reference's dial (its manual's drawing and the owner's photographs): 200 ms at -90
/// degrees (0.2 of the travel), 600 ms at -30 (0.4), the top tick (0.5), 1 s at 30 (0.6), 5 s
/// at 60 (0.7), 10 s at 105 (0.85; the CA-72's panel prints 5 s at 67 and 10 s at 108), and
/// fully clockwise. The generic taper's are 17.6K, 53.9K, 100K, 162K, 268K, 535K and 1M.
///
/// With the timing capacitors as the reference's are (their series resistance and absorption,
/// `contour::C_ABSORPTION`), each resistance is the one at which the contour keeps the time
/// the ideal capacitor gave at the first fit, 0.88 to 0.90 of it; at 0.15, the generic taper's
/// 11.7K so scaled, and below it the generic taper's shape (docs/calibration, change 25).
pub const FILTER_ATTACK: [(f64, f64); 8] = [
    (0.15, 10.49e3),
    (0.2, 13.56e3),
    (0.4, 27.64e3),
    (0.5, 36.02e3),
    (0.6, 46.90e3),
    (0.7, 225.9e3),
    (0.85, 669.0e3),
    (1.0, 946.8e3),
];
pub const FILTER_DECAY: [(f64, f64); 8] = [
    (0.15, 10.47e3),
    (0.2, 15.94e3),
    (0.4, 33.12e3),
    (0.5, 43.46e3),
    (0.6, 52.19e3),
    (0.7, 236.8e3),
    (0.85, 581.7e3),
    (1.0, 768.2e3),
];
pub const LOUDNESS_ATTACK: [(f64, f64); 8] = [
    (0.15, 10.45e3),
    (0.2, 14.13e3),
    (0.4, 30.97e3),
    (0.5, 41.85e3),
    (0.6, 47.29e3),
    (0.7, 267.0e3),
    (0.85, 703.8e3),
    (1.0, 982.6e3),
];
pub const LOUDNESS_DECAY: [(f64, f64); 8] = [
    (0.15, 10.47e3),
    (0.2, 13.00e3),
    (0.4, 27.42e3),
    (0.5, 37.04e3),
    (0.6, 44.53e3),
    (0.7, 206.4e3),
    (0.85, 558.5e3),
    (1.0, 755.4e3),
];

/// An ATTACK or DECAY pot as the voice has it, ohm: to `law`'s first point (the dial's tick
/// past its 10 ms mark, up to which the reference's times agree with the generic taper within
/// where its knob was set) the generic taper's shape scaled to meet it, then straight in its
/// logarithm through `law`'s points.
pub fn time_pot(p: f64, law: &[(f64, f64)]) -> f64 {
    let p = p.clamp(0.0, 1.0);
    let (p0, r0) = law[0];
    if p <= p0 {
        return r0 * time_pot_drawn(p) / time_pot_drawn(p0);
    }
    let mut a = law[0];
    for &b in &law[1..] {
        if p == b.0 {
            return b.1;
        }
        if p < b.0 {
            return a.1 * crate::ulp::pow(b.1 / a.1, (p - a.0) / (b.0 - a.0));
        }
        a = b;
    }
    a.1
}

/// EMPHASIS on Figure 9-17: R14, 50K reverse audio used as a rheostat, 50K at 0 and 0 at 10
/// (the generic taper). Folkman's regeneration calibration and the service manual's checks run
/// on it (`filter_cal`).
pub fn emphasis_r14_drawn(p: f64) -> f64 {
    50e3 * audio_taper(1.0 - p)
}

/// R14's resistance at EMPHASIS 2.5, 5, 6, 7, 7.5 and 8.5 such that the filter's passband
/// falls as the hardware reference's does (-2.25, -9.53, -10.43, -11.92, -12.50 and -14.67
/// dB: docs/calibration, sessions E and J), and at 2, 3 and 4 such that its passband and peak
/// stand as the reference's do with its corner near 800 Hz (session N). The generic taper's
/// are 20.4K, 16.25K, 12.9K, 8.1K, 5.0K, 3.0K, 1.71K, 1.25K and 0.58K.
const EMPHASIS_R14: [(f64, f64); 10] = [
    (0.0, 50e3),
    (0.2, 30.68e3),
    (0.25, 18.82e3),
    (0.3, 9.47e3),
    (0.4, 3.64e3),
    (0.5, 2.91e3),
    (0.6, 2.30e3),
    (0.7, 1.548e3),
    (0.75, 1.35e3),
    (0.85, 834.0),
];

/// EMPHASIS as the voice has it: R14 through [`EMPHASIS_R14`], straight in its logarithm
/// between them, and from 8.5 to 10 the generic taper's shape scaled to meet it.
pub fn emphasis_r14(p: f64) -> f64 {
    let p = p.clamp(0.0, 1.0);
    let (pn, rn) = EMPHASIS_R14[EMPHASIS_R14.len() - 1];
    if p >= pn {
        return rn * audio_taper(1.0 - p) / audio_taper(1.0 - pn);
    }
    for w in EMPHASIS_R14.windows(2) {
        let ((pa, ra), (pb, rb)) = (w[0], w[1]);
        if p <= pb {
            return ra * crate::ulp::pow(rb / ra, (p - pa) / (pb - pa));
        }
    }
    rn
}

/// The mixer's bus as a Norton source: current and conductance.
fn channel(v: f64, r_src: f64, volume: f64, on: bool, r_series: f64) -> (f64, f64) {
    if !on {
        // Off, the switch grounds the series resistor's far end: it loads the bus only.
        return (0.0, 1.0 / r_series);
    }
    let p = volume_track(volume);
    let (up, down) = (r_src + 25e3 * (1.0 - p), (25e3 * p).max(1e-3));
    let v_w = v * down / (up + down);
    let r_w = up * down / (up + down);
    let g = 1.0 / (r_w + r_series);
    (v_w * g, g)
}

/// The outputs (sawtooth, triangle, rectangle) [`waveform_source`] takes from oscillator `n`
/// (0, 1, 2) for a waveform, which alone are band-limited in full (decisions.md R-BLEP):
/// oscillator 3's reverse sawtooth stage takes its sawtooth; the shark tooth of oscillators
/// 1 and 2 mixes the sawtooth and the triangle.
fn outputs_of(w: Waveform, n: usize) -> [bool; 3] {
    match w {
        Waveform::Triangle => [false, true, false],
        Waveform::Sawtooth => [true, false, false],
        Waveform::SharkTooth | Waveform::ReverseSawtooth => [true, n != 2, false],
        Waveform::Square | Waveform::WideRectangle | Waveform::NarrowRectangle => {
            [false, false, true]
        }
    }
}

/// Front-panel R030 on drawing1448, retained at its drawn47K value.
pub const SHARK_SAW_OHMS: f64 = 47e3;
/// Reference-equivalent R031; the drawing/BOM is10K±5% (Table7-2).
/// 9.53K is an inferred E96 equivalent, not a measured installed component.
pub const SHARK_TRIANGLE_OHMS: f64 = 9.53e3;

/// An oscillator's selected output as a Thevenin source (board1's output dividers:
/// sawtooth R34/R33, triangle R28/R23 behind Q1, rectangle R22 against R13 as Q12
/// switches), the hybrid through front-panel R030/R031. For oscillator3, `rev` is its
/// reverse-sawtooth stage. Public so the circuit lab can check the actual source model.
pub fn waveform_source(w: Waveform, o: &VcoOut, rev: Option<&RevSawOut>) -> (f64, f64) {
    let r_saw = 4300.0 * 4700.0 / 9000.0;
    let saw = match rev {
        Some(r) => (r.saw, r_saw * 33e3 / (r_saw + 33e3)),
        None => (o.saw, r_saw),
    };
    let tri = (o.tri, 750.0 * 8200.0 / 8950.0);
    // Rectangle: 4.7K to ground when Q12 is off (0 V), 4.7K || 6.2K when on (-4.3 V).
    let on = (-o.rect / 4.3).clamp(0.0, 1.0);
    let rect = (o.rect, 4700.0 + (4700.0 * 6200.0 / 10900.0 - 4700.0) * on);
    match (w, rev) {
        (Waveform::Triangle, _) => tri,
        (Waveform::Sawtooth, _) => saw,
        (Waveform::SharkTooth | Waveform::ReverseSawtooth, Some(r)) => (r.rev, r.r_rev),
        (Waveform::SharkTooth | Waveform::ReverseSawtooth, None) => {
            let (ga, gb) = (
                1.0 / (saw.1 + SHARK_SAW_OHMS),
                1.0 / (tri.1 + SHARK_TRIANGLE_OHMS),
            );
            ((saw.0 * ga + tri.0 * gb) / (ga + gb), 1.0 / (ga + gb))
        }
        _ => rect,
    }
}

/// The outputs (sawtooth, triangle, rectangle) a CONTINUOUS shape `at` takes from oscillator
/// `n`: a place's as the switch's own position's ([`outputs_of`]), a blend's both places'; all
/// of them while MOD may move it anywhere.
fn shape_outputs(n: usize, at: f64, modulated: bool) -> [bool; 3] {
    if modulated {
        return [true; 3];
    }
    let place = |k: usize| match shape_place(n, k) {
        Some(w) => outputs_of(w, n),
        // (The reverse shark tooth: the triangle, and the sawtooth the reverse stage takes.)
        None => [true, true, false],
    };
    if at >= 3.0 {
        return place(3);
    }
    let k = at.floor() as usize;
    if at == k as f64 {
        return place(k);
    }
    let (a, b) = (place(k), place(k + 1));
    [a[0] || b[0], a[1] || b[1], a[2] || b[2]]
}

/// A CONTINUOUS shape `at` (the switch's steps) as a Thevenin source: at a place, that place's
/// source exactly (the switch's own position's, [`waveform_source`]); between two, the two
/// mixed in turn, as a pot between them would (decisions.md R-CONTINUOUS-WAVE); past SQUARE
/// the rectangle. Oscillator 3's second place is a reverse shark tooth, its triangle and its
/// reverse sawtooth through the same calibrated front-panel network
/// ([`SHARK_TRIANGLE_OHMS`] and [`SHARK_SAW_OHMS`]); its third place is the reverse
/// sawtooth: `rev`, which CONTINUOUS runs while it is needed.
fn shape_source(n: usize, at: f64, o: &VcoOut, rev: Option<&RevSawOut>) -> (f64, f64) {
    let place = |k: usize| match shape_place(n, k) {
        Some(Waveform::ReverseSawtooth) => waveform_source(Waveform::ReverseSawtooth, o, rev),
        Some(w) => waveform_source(w, o, None),
        None => {
            let tri = waveform_source(Waveform::Triangle, o, None);
            let back = rev.map_or((0.0, 1e6), |r| (r.rev, r.r_rev));
            let (ga, gb) = (
                1.0 / (back.1 + SHARK_SAW_OHMS),
                1.0 / (tri.1 + SHARK_TRIANGLE_OHMS),
            );
            ((back.0 * ga + tri.0 * gb) / (ga + gb), 1.0 / (ga + gb))
        }
    };
    if at >= 3.0 {
        return place(3);
    }
    let k = at.floor() as usize;
    let w = at - k as f64;
    if w == 0.0 {
        return place(k);
    }
    let (a, b) = (place(k), place(k + 1));
    ((1.0 - w) * a.0 + w * b.0, (1.0 - w) * a.1 + w * b.1)
}

/// GLIDE's resistance in circuit: the 5M pot (a rheostat), shorted by the GLIDE switch
/// off, ohm.
pub fn glide_r(p: f64, on: bool) -> f64 {
    if on { glide_pot(p) } else { 0.0 }
}

/// GLIDE as Figure 9-17 draws it: 5M on the generic audio taper, ohm.
pub fn glide_pot_drawn(p: f64) -> f64 {
    5e6 * audio_taper(p)
}

/// GLIDE's resistance at 2.5, 5, 7.5 and 10 such that the keyboard's voltage slides an
/// octave, up and down, at the hardware reference's rates (20 to 80 % of C3 to C4 and back:
/// 61 and 22, 29 and 18, 3.8 and 2.1, 1.6 and 0.9 octaves a second; docs/calibration,
/// session J). The generic taper's are 125K, 500K, 1.63M and 5M.
const GLIDE_LAW: [(f64, f64); 4] = [(0.25, 162e3), (0.5, 257e3), (0.75, 2.1e6), (1.0, 5e6)];

/// GLIDE as the voice has it: the generic taper's shape to 2.5, scaled to meet
/// [`GLIDE_LAW`], then straight in its logarithm through it, ohm.
pub fn glide_pot(p: f64) -> f64 {
    let p = p.clamp(0.0, 1.0);
    let (p0, r0) = GLIDE_LAW[0];
    if p <= p0 {
        return r0 * audio_taper(p) / audio_taper(p0);
    }
    for w in GLIDE_LAW.windows(2) {
        let ((pa, ra), (pb, rb)) = (w[0], w[1]);
        if p <= pb {
            return ra * crate::ulp::pow(rb / ra, (p - pa) / (pb - pa));
        }
    }
    GLIDE_LAW[GLIDE_LAW.len() - 1].1
}

/// How long a released key's pitch contact stays closed after its trigger contact opens,
/// s (the keyboard is set up so the trigger goes off first: service manual 2.2.6; the time
/// is not documented: assumptions A18).
pub const RELEASE_LEAD: f64 = 2e-3;

/// How long a voice's trigger contact must be open before a key pressed again retriggers
/// its contours, s: Q20 holds the reset line about 12 ms after a release, until C7 has
/// drained (docs/circuit/board2.md, "Trigger section"; `tests/retrigger.rs` finds 11.5 to
/// 12 ms in every mode), and a millisecond more. POLY waits this long before a note that
/// takes a voice whose key is down (decisions.md, "POLY").
pub const RETRIGGER_GAP: f64 = 0.013;

/// The lowest key (F) as a MIDI note on 8' (A, the fifth key, is 110 Hz there).
pub const LOWEST_KEY_MIDI: i32 = 41;
pub const KEYS: i32 = 44;

/// The MIDI notes the voice plays: all 128. Those beyond the instrument's 44 keys continue
/// its key string (the plug-in's extension, not the circuit's: [`Keys::step`]).
pub const MIDI_NOTES: std::ops::RangeInclusive<i32> = 0..=127;

/// The keyboard circuit's load: the three oscillators' keyboard inputs and the filter's
/// KEYBOARD CONTROL 1 and 2 ([`Load::of`]).
fn keyboard_load(p: &Panel) -> Load {
    Load::of(3, p.keyboard_control_1, p.keyboard_control_2)
}

/// How far up FEEDBACK's knob (0..1 of its travel) sends how much of the output back
/// (0..1, [`Voice::feedback`]): a taper, as a volume pot's, so the knob's lower half is a
/// gentle onset and the loop's self-oscillation is near its top. 60 % of the travel sends
/// 30 % (the owner, 2026-10-02, found 30 % sent already very intense at 30 % of the knob);
/// a third sends 6 %, three quarters 51 %.
pub fn feedback_law(knob: f64) -> f64 {
    if knob > 0.0 {
        crate::ulp::pow(knob.min(1.0), FEEDBACK_TAPER)
    } else {
        0.0
    }
}

/// [`feedback_law`]'s exponent: ln 0.3 / ln 0.6, so that 60 % of the knob sends 30 %.
pub const FEEDBACK_TAPER: f64 = 2.356_915_448_856_724;

/// The instrument's voice: its panel and four parts. The keys' contacts, the keyboard
/// circuit and the contours (with the noise source) depend only on the keys and the panel,
/// not on the audio path, so a caller may run them on threads of their own ahead of it
/// ([`crate::threaded`]); [`Voice::tick_in`] runs them in turn.
#[derive(Debug, Clone)]
pub struct Voice {
    pub panel: Panel,
    keys: Keys,
    kbd: KeyboardPart,
    ctl: ControlPart,
    bias: BiasPart,
    audio: AudioPart,
    /// For measuring a quality mode's audio path apart (`ca72-lab hifi`): the pitch and
    /// contour paths (the keyboard circuit, the contours) held at this quality whatever
    /// the panel's.
    control_quality: Option<Quality>,
    /// The last sample's contours (filter, loudness), V.
    last_ctl: (f64, f64),
    /// FEEDBACK (the plug-in's, not the instrument's): how much of the voice's own output is
    /// patched back into its EXTERNAL INPUT jack, 0 to 1 (1: the output at its full level,
    /// before MAIN OUTPUT's VOLUME), at the next filter step where [`loops`], otherwise one
    /// sample late, as players cable the output into the
    /// external input to overdrive the preamplifier and the mixer. Heard through the
    /// MIXER's EXTERNAL INPUT switch and VOLUME, as the cable is. 0, nothing is added.
    pub feedback: f64,
    /// The historical preamplifier comparison ([`InLoop`]); the closed loop always uses
    /// the exact solve, `Circuit` also selecting the nodal lamp.
    pub in_loop: InLoop,
    /// The last output sample, for FEEDBACK.
    fed_back: f64,
}

/// A voice's parts lent for a run ([`Voice::lend`]).
pub(crate) struct Lent<'a> {
    pub panel: &'a Panel,
    pub keys: &'a mut Keys,
    pub kbd: &'a mut KeyboardPart,
    pub ctl: &'a mut ControlPart,
    pub bias: &'a mut BiasPart,
    pub pre: &'a mut PreampPart,
    pub front: &'a mut FrontPart,
    pub back: &'a mut BackPart,
}

/// The keys: which are held, and which released keys' pitch contacts are still closed.
#[derive(Debug, Clone)]
pub struct Keys {
    rate: f64,
    /// Keys held (as key numbers from the lowest), in the order pressed.
    held: Vec<i32>,
    /// Released keys whose pitch contacts are still closed, and the sample they open at.
    releasing: Vec<(i32, u64)>,
    /// When each MIDI note was last pressed, counted in presses (for LAST), and the count.
    pressed: [u64; 128],
    presses: u64,
    /// RETRIGGER's keys pressed, waiting for the trigger contact to have been open
    /// [`RETRIGGER_GAP`] (in samples, `gap`); the trigger contact as last set, and the
    /// sample it last opened at (`None`: never closed).
    waiting: Vec<i32>,
    gap: u64,
    gate: bool,
    opened: Option<u64>,
    samples: u64,
    /// The string's foot as last set (it stays while no key's contact is closed).
    foot: i32,
}

/// A sample's contacts: the lowest and highest keys whose pitch contacts are closed, the
/// trigger contact, and EXT. S-TRIG (the rear jack shorted to ground). With a key beyond
/// the string's ends sounding, `keys` is that end's key alone and `foot` how many keys
/// beyond it the key is ([`Keys::step`]); 0 otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Contacts {
    pub keys: Option<(usize, usize)>,
    pub gate: bool,
    pub s_trig: bool,
    pub foot: i32,
}

/// A sample at the instrument's jacks: the EXTERNAL INPUT (as a fraction of
/// [`INPUT_VOLTS`]) and the rear panel's control inputs, V, `None` for an empty jack (its
/// normal contact in place: the oscillators' external bus held by R156 33K, the filter's
/// R51 grounded, the loudness input tied to +10 V through 33K); EXT. S-TRIG closed or open.
/// A plugged source is ideal (A5).
///
/// With them, the plug-in's ENTROPY (decisions.md; not the instrument's): each oscillator off its
/// calibration by `detune` cents and the filter's cutoff off its by `cutoff` octaves, both
/// 0 for the circuit as drawn (the samples then the same to the bit). And each oscillator timed
/// (decisions.md R-OSC-TIMED; the plug-in's too): where its cycle stands on the host's clock, at
/// its RANGE's LO; none, the circuit's own.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Jacks {
    pub ext: f64,
    pub osc: Option<f64>,
    pub filter: Option<f64>,
    pub loudness: Option<f64>,
    pub s_trig: bool,
    pub detune: [f64; 3],
    pub cutoff: f64,
    pub timed: [Option<Timed>; 3],
    /// The plug-in's LFOs' knobs gliding across their steps ([`Swept`]).
    pub swept: Swept,
}

/// The plug-in's (decisions.md R-LFO-STRESS): how far the knobs an LFO moves have gone since
/// its last step, added where the circuit reads them, so that they glide in a straight line
/// from one step's value to the next instead of stepping: CUTOFF's wiper (a share of its
/// track), TUNE (its -1..1) and oscillators 2's and 3's FREQUENCY wipers, after their knobs'
/// laws. All 0: the panel's own, to the bit.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Swept {
    pub cutoff: f64,
    pub tune: f64,
    pub freq: [f64; 2],
}

/// The volts an octave at a 100K input to the filter's control node, as R51 (the FILTER
/// CONTROL jack's) is, measured on the model with CUTOFF at its centre: 0.97 V at -3 and
/// 1.03 V at +3, the converter's own curve (`tests/entropy.rs`). KEYBOARD CONTROL 1 and 2
/// together (100K) make the filter follow the keyboard's 1.013 V an octave about as closely.
pub const FILTER_VOLTS_PER_OCTAVE: f64 = 0.98;

/// The keyboard circuit (board 1), driven by the contacts: the pitch voltage.
#[derive(Debug, Clone)]
pub struct KeyboardPart {
    keyboard: Keyboard,
    /// The sample rate, Hz; the circuit's sub-rate (samples a tick, [`keyboard_every`]),
    /// the samples since the last tick and the outputs interpolated between.
    rate: f64,
    every: usize,
    count: usize,
    from: f64,
    to: f64,
    /// The keyboard's output's volts a key over the instrument's keys (the string's scale as
    /// its current, the amplifier and the load give it), and the keys beyond the string's
    /// ends in volts as they stand, gliding as the hold capacitor does (decisions.md, "The
    /// full MIDI range": the plug-in's extension).
    step: f64,
    beyond: f64,
}

/// The contour generators at their sub-rate, interpolated, and the noise source: what
/// depends only on the trigger contact and the panel.
#[derive(Debug, Clone)]
pub struct ControlPart {
    contours: Contours,
    contour_every: usize,
    /// The sample rate, Hz (the contours' sub-rate follows the quality mode).
    rate: f64,
    count: usize,
    env_from: (f64, f64),
    env_to: (f64, f64),
    /// The contours' outputs of the last samples, the newest at `env_at`: each reaches what
    /// it shapes as late as the sound does ([`contour_delays`]; decisions.md R-LATENCY).
    env_past: [(f64, f64); ENV_PAST],
    env_at: usize,
    noise: Noise,
    /// The ATTACK and DECAY pots' laws (filter, loudness).
    pots: [Memo<f64>; 4],
    /// The noise's last outputs, and the modulation mix's gain from it (its table, and its
    /// law at MODULATION MIX), for knowing when nothing hears it.
    last_noise: NoiseOut,
    mix: Modulation,
    noise_gain: Memo<f64>,
}

/// A knob law's last input and value: taken again only when its knob moves
/// (performance: their powers were a few percent of Potato's time). The same value either
/// way.
#[derive(Debug, Clone, Copy)]
struct Memo<T: Copy>(Option<(u64, T)>);

impl<T: Copy> Memo<T> {
    const NEW: Memo<T> = Memo(None);

    fn get(&mut self, x: f64, law: impl Fn(f64) -> T) -> T {
        match self.0 {
            Some((k, v)) if k == x.to_bits() => v,
            _ => {
                let v = law(x);
                self.0 = Some((x.to_bits(), v));
                v
            }
        }
    }
}

/// The VCA's bias ([`Vca::control`]) on the loudness contour and EXT. LOUDNESS. It reads
/// nothing of the signal path, so on threads it runs on one of its own, fed the contour
/// by the contours' as they go, ahead of the audio path (performance); the back's copy
/// of the VCA runs the signal path on it.
#[derive(Debug, Clone)]
pub struct BiasPart {
    vca: Vca,
    /// EXT. LOUDNESS's normal contact: the voltage and resistance J3 ties the input to.
    j3_normal: (f64, f64),
    /// The VCA's full bias solve's interval with the jack empty, samples.
    vca_every: usize,
}

/// A sample of [`ControlPart`]'s outputs: the filter and loudness contours, V, and the
/// noise.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ControlOut {
    pub env_f: f64,
    pub env_l: f64,
    pub noise: NoiseOut,
}

/// The audio path: the external input's preamplifier, then the front (modulation,
/// oscillators, mixer, the filter's control node), then the back (filter, VCA, the A-440).
/// Each depends only on what comes before it, so on threads they run side by side, each a
/// few samples behind the one before (`threaded.rs`), doing what [`AudioPart::tick`] does.
#[derive(Debug, Clone)]
pub struct AudioPart {
    pub(crate) pre: PreampPart,
    pub(crate) front: FrontPart,
    pub(crate) back: BackPart,
    /// The last sample's keyboard voltage, the mixer bus's Norton current, A, and the
    /// filter's output, V.
    last: (f64, f64, f64),
}

/// The external input's preamplifier and the OVERLOAD lamp's driver.
#[derive(Debug, Clone)]
pub struct PreampPart {
    preamp: Preamp,
    /// The OVERLOAD lamp (0 off, 1 fully lit), its brightest since it was last read
    /// ([`AudioPart::take_overload_peak`]), and the preamplifier's output, V.
    overload: f64,
    overload_peak: f64,
    ext_amp: f64,
    /// EXT. INPUT VOLUME's law.
    volume: Memo<(f64, f64)>,
    /// FEEDBACK on: the historical delayed-loop comparison (`in_loop`, R8, R11).
    /// The closed loop runs the exact preamplifier solve instead (R-HI-FEEDBACK).
    pub accurate: bool,
    pub in_loop: InLoop,
    /// FEEDBACK's loop at one rate ([`tick_looped`]): the preamplifier's current into the bus
    /// at the filter's next step, A.
    i_fed: f64,
}

/// The mixer's own noise as a Norton current on its bus, A RMS from 0 to 24 kHz (white):
/// the hardware reference's MIX jack carries 82.2 dB under a sawtooth at VOLUME 10 with
/// every source off (-103.5 against -21.3 dBFS, the interface's own floor 12 to 15 dB
/// lower; docs/calibration, sessions J and L), and the CA-72's bus 0.0319 mA RMS for that
/// sawtooth. The filter's self-oscillation starts from it, as the reference's does from its
/// circuit's noise; without it, nothing on the bus, the model's filter never starts.
pub const MIXER_HISS: f64 = 2.48e-9;

/// A seed for the mixer's noise, apart from the noise source's.
const HISS_SEED: u64 = 0x6869_7373;

/// The modulation line, the oscillators, the mixer and the filter's control node.
#[derive(Debug, Clone)]
pub struct FrontPart {
    rate: f64,
    /// The mixer's noise ([`MIXER_HISS`]): its generator, and its RMS a sample at the
    /// front's rate.
    hiss: Gauss,
    hiss_rms: f64,
    /// Each oscillator's tuning (TUNE and the octave step shared) and its oscillator.
    tunings: [Tuning; 3],
    vcos: [Vco; 3],
    revsaw: RevSaw,
    expo: FilterExpo,
    table: std::sync::Arc<ExpoTable>,
    modulation: Modulation,
    /// The filter's control node's voltage and current (the last sample's), and the
    /// modulation line's.
    filter_node: f64,
    i0: f64,
    line: f64,
    /// Oscillator 3's switch output, the last sample's (the modulation mix's input).
    last_osc3: f64,
    /// Each oscillator's timing current (the converter's output), the last sample's: its
    /// frequency follows it (a probe of the pitch path).
    last_i: [f64; 3],
    /// The MODULATION wheel's law.
    wheel: Memo<f64>,
    /// The modulation amplifier's output at the middle of oscillator 3's triangle and half
    /// its swing (MODULATION MIX at oscillator 3): what MOD's whole travel is measured
    /// against (decisions.md R-CONTINUOUS-WAVE); and the line as MOD took it, the last sample's.
    mod_ref: (f64, f64),
    mod_u: ModSwing,
    /// Each oscillator's CONTINUOUS shape where it was, the last sample's (MOD's orb on the
    /// panel: decisions.md R-CONTINUOUS-WAVE).
    shape_now: [Option<f64>; 3],
}

/// What the front gives the back each sample: the mixer bus's Norton current (A) and
/// conductance (S), and the filter's control current (A).
/// A sample's voice up to its filter ([`Voice::before_filter`]): the panel it played, the
/// front's output, the VCA's bias and the keyboard's voltage.
#[derive(Debug, Clone, Copy)]
pub struct Halfway {
    p: Panel,
    f: FrontOut,
    d: VcaDrive,
    v_kbd: f64,
    /// The filter's output when FEEDBACK ran the whole audio path already.
    looped: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FrontOut {
    pub i_bus: f64,
    pub g_bus: f64,
    pub i0: f64,
}

/// The filter, the VCA and the A-440.
#[derive(Debug, Clone)]
pub struct BackPart {
    vcf: Vcf,
    /// The VCA's signal path ([`Vca::signal`]) on the bias [`ControlPart`] works out: at the
    /// filter's rate in HI and ULTRA (decisions.md R-HI-FEEDBACK, R-BLEP).
    vca: Vca,
    vca_4x: OversampledVca,
    /// The filter's samples at its rate from [`Voice::filter_pair`], for
    /// [`BackPart::after_filter`].
    xs: [f64; 8],
    /// Whether the last sample ran the oversampled VCA, and the last output sample (V, before the
    /// output's scale): what a switch between the two takes over.
    ultra: bool,
    last_out: f64,
    a440: A440,
    /// EMPHASIS's law.
    emphasis: Memo<f64>,
    /// The output sample [`Voice::filter_pair`] worked out whole, by [`BackPart::tick`], for a
    /// voice it does not pair (ULTRA, whose VCA takes the filter's own samples, or two voices
    /// at different qualities), for [`Voice::after_filter`] to give.
    ready: Option<f64>,
}

impl BackPart {
    /// [`Voice::set_drive`] for the back on a thread of its own ([`crate::threaded`]).
    pub fn set_drive(&mut self, gain: f64) {
        self.vcf.set_drive(gain);
    }
}

/// The external input's scale: the jack's voltage for a sample of 1.0, the same as the
/// output's (so a render fed back to the input behaves as the instrument does; A25).
pub const INPUT_VOLTS: f64 = 5.0;

/// R9's law: the fraction of its track below the wiper at EXTERNAL INPUT VOLUME's marks 2, 4,
/// 5, 6, 8 and 10, solved so that the voice's gain from the jack to the mixer (the wiper loaded
/// by the preamplifier, about 97K) falls from 10 as the hardware reference's does: -39.5,
/// -33.1, -31.3, -29.9 and -16.9 dB (docs/calibration, session H). Figure 9-17's "1M audio"
/// with the generic taper gave -36.5, -28.4, -25.7, -23.4 and -18.6.
const EXT_TAPER: [(f64, f64); 6] = [
    (0.2, 0.011_81),
    (0.4, 0.028_45),
    (0.5, 0.037_10),
    (0.6, 0.046_45),
    (0.8, 0.509_38),
    (1.0, 1.0),
];

/// R9 at `p` (0..1), as a fraction of its track: between the measured marks, straight in the
/// fraction's logarithm; below 2, the generic audio taper's shape scaled to meet it.
pub fn ext_taper(p: f64) -> f64 {
    let p = p.clamp(0.0, 1.0);
    let (p0, t0) = EXT_TAPER[0];
    if p <= p0 {
        return t0 * audio_taper(p) / audio_taper(p0);
    }
    for w in EXT_TAPER.windows(2) {
        let ((pa, ta), (pb, tb)) = (w[0], w[1]);
        if p <= pb {
            let u = (p - pa) / (pb - pa);
            return ta * crate::ulp::pow(tb / ta, u);
        }
    }
    1.0
}

/// R9 (EXTERNAL INPUT VOLUME, 1M) at `volume`: its divider's ratio and source resistance, ohm.
fn ext_volume(volume: f64) -> (f64, f64) {
    let t = ext_taper(volume);
    (t, 1e6 * t * (1.0 - t))
}

impl Voice {
    /// A voice at `rate` Hz (48 kHz is what the models are tested at): tunes the three
    /// oscillators and balances the VCA as the factory did, and settles every state.
    pub fn new(rate: f64, panel: Panel) -> Voice {
        let mut keyboard = Keyboard::new(KeyboardCircuit::default(), rate);
        keyboard.set_load(keyboard_load(&panel));
        // The factory tuning plays its keys through the keyboard circuit, settled (the
        // keys it uses, each settled once).
        let played: Vec<(u32, f64)> = [LOW_A, SECOND_A, HIGH_A]
            .iter()
            .map(|&k| {
                let v = keyboard
                    .static_out(k as usize)
                    .unwrap_or(f64::from(k) * crate::tuning::KEY_STEP);
                (k, v)
            })
            .collect();
        let volts = |k: u32| {
            played
                .iter()
                .find(|&&(key, _)| key == k)
                .map_or(f64::from(k) * crate::tuning::KEY_STEP, |&(_, v)| v)
        };
        let os = oversampling(Quality::NoCompromises).0;
        let mut vcos = [Vco::new(rate, os), Vco::new(rate, os), Vco::new(rate, os)];
        // Tuned as the instrument stands for the procedure: the wheel in its detent, the
        // modulation off (its bus grounded, A21), the external input's jack empty.
        let buses = Buses {
            ext: OPEN_BUS,
            ..Buses::RESTING
        };
        let (t1, _) = folkman_1973_buses(&mut vcos[0], rate, Tuning::default(), &volts, &buses);
        let two = Osc::Two { freq: FREQ_CENTRE };
        let three = Osc::Three {
            freq: FREQ_CENTRE,
            control: true,
        };
        let (t2, _) = folkman_1973_osc_buses(&mut vcos[1], rate, t1, two, &volts, &buses);
        let (t3, _) = folkman_1973_osc_buses(&mut vcos[2], rate, t1, three, &volts, &buses);
        let tunings = [t1, t2, t3];
        for (v, t) in vcos.iter_mut().zip(&tunings) {
            v.expo.r11 = t.r11;
            v.expo.a8 = t.a8;
        }
        let revsaw = RevSaw::new(RevSawCircuit::default(), rate)
            .expect("the reverse sawtooth's operating point");
        let density = calibrate_noise(&mut vcos[0].clone(), rate, &t1, volts(LOW_A));
        for v in &mut vcos {
            v.prepare(oversampling(Quality::Potato).0);
        }
        let mut noise = Noise::new(
            NoiseCircuit::default(),
            noise_loads(&panel),
            rate,
            0x4D6F_6F67,
        )
        .expect("the noise generator's operating point");
        noise.set_density(density);
        let modulation = Modulation::new().expect("the modulation mix's operating points");
        let contour_every = contour_every(Quality::NoCompromises);
        // The balance trims as the hardware reference's, not the factory procedure's
        // (docs/calibration, change 32).
        let mut vca = Vca::new(VcaCircuit::default().reference_trims(), rate, 0.0);
        vca.prepare_potato();
        let mut ctl = ControlPart {
            contours: Contours::new(ContourCircuit::default(), rate / contour_every as f64),
            contour_every,
            rate,
            count: 0,
            env_from: (0.0, 0.0),
            env_to: (0.0, 0.0),
            env_past: [(0.0, 0.0); ENV_PAST],
            env_at: 0,
            noise,
            pots: [Memo::NEW; 4],
            last_noise: NoiseOut::default(),
            mix: modulation.clone(),
            noise_gain: Memo::NEW,
        };
        let bias = BiasPart {
            vca: vca.clone(),
            j3_normal: (vca.circuit.ext, vca.circuit.r_j3),
            vca_every: Vca::new(VcaCircuit::default(), rate, 0.0).control_every,
        };
        let cp = contour_panel(&panel, false, &mut ctl.pots);
        ctl.contours.settle(&cp);
        let o = ctl.contours.tick(false, &cp);
        ctl.env_from = (o.filter, o.loudness);
        ctl.env_to = ctl.env_from;
        ctl.env_past = [ctl.env_from; ENV_PAST];
        Voice {
            panel,
            keys: Keys {
                rate,
                // Room for every key: a key press must not allocate on the audio thread.
                held: Vec::with_capacity(128),
                releasing: Vec::with_capacity(128),
                pressed: [0; 128],
                presses: 0,
                waiting: Vec::with_capacity(128),
                gap: (RETRIGGER_GAP * rate).round() as u64,
                gate: false,
                opened: None,
                samples: 0,
                foot: 0,
            },
            kbd: KeyboardPart {
                to: keyboard.out(),
                from: keyboard.out(),
                keyboard,
                rate,
                every: 1,
                count: 0,
                step: (volts(HIGH_A) - volts(LOW_A)) / f64::from(HIGH_A - LOW_A),
                beyond: 0.0,
            },
            ctl,
            bias,
            audio: AudioPart {
                pre: PreampPart {
                    preamp: Preamp::new(
                        PreampCircuit::default(),
                        rate,
                        ext_volume(panel.ext_volume).1,
                    )
                    .expect("the preamplifier's operating point"),
                    overload: 0.0,
                    overload_peak: 0.0,
                    ext_amp: 0.0,
                    i_fed: 0.0,
                    volume: Memo::NEW,
                    accurate: false,
                    in_loop: InLoop::default(),
                },
                front: FrontPart {
                    rate,
                    hiss: Gauss::new(HISS_SEED),
                    hiss_rms: MIXER_HISS * libm::sqrt(rate / 48e3),
                    tunings,
                    vcos,
                    revsaw,
                    expo: crate::filter_cal::CALIBRATED.expo(),
                    table: ExpoTable::shared(crate::filter_cal::CALIBRATED.expo(), 25.0),
                    modulation: modulation.clone(),
                    filter_node: 0.0,
                    i0: 0.0,
                    line: 0.0,
                    last_osc3: 0.0,
                    last_i: [0.0; 3],
                    wheel: Memo::NEW,
                    mod_ref: mod_reference(&modulation),
                    mod_u: ModSwing::default(),
                    shape_now: [None; 3],
                },
                back: BackPart {
                    vcf: {
                        // The filter as calibrated (Folkman's procedure, RANGE where the
                        // hardware reference's sits: filter_cal::CALIBRATED).
                        let mut f = Vcf::new(rate, oversampling(Quality::NoCompromises).1);
                        f.circuit = crate::filter_cal::CALIBRATED.circuit();
                        f.prepare(oversampling(Quality::Potato).1);
                        f
                    },
                    vca_4x: OversampledVca::new(&vca, ULTRA_VCA_OVERSAMPLING),
                    xs: [0.0; 8],
                    vca,
                    ultra: false,
                    last_out: 0.0,
                    a440: A440::new(rate),
                    emphasis: Memo::NEW,
                    ready: None,
                },
                last: (0.0, 0.0, 0.0),
            },
            control_quality: None,
            last_ctl: (0.0, 0.0),
            feedback: 0.0,
            in_loop: InLoop::default(),
            fed_back: 0.0,
        }
    }

    /// A voice at `rate` Hz built once per rate and copied: its factory calibration (the
    /// oscillators' tuning, the tables, the noise's level) depends only on the circuit and
    /// the rate, and takes seconds. The panel is the default; set it after.
    pub fn prototype(rate: f64) -> Voice {
        use std::sync::{Mutex, OnceLock};
        static BUILT: OnceLock<Mutex<Vec<(u64, Voice)>>> = OnceLock::new();
        let cache = BUILT.get_or_init(|| Mutex::new(Vec::new()));
        let key = rate.to_bits();
        let cached = cache
            .lock()
            .ok()
            .and_then(|c| c.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone()));
        let mut v = match cached {
            Some(v) => v,
            None => {
                let v = Voice::new(rate, Panel::default());
                if let Ok(mut c) = cache.lock() {
                    c.push((key, v.clone()));
                }
                v
            }
        };
        // A clone keeps the key lists' contents, not their room: a key press must not
        // allocate on the audio thread.
        v.keys.reserve();
        v
    }

    /// The plug-in's DRIVE (its decisions.md R45, the CA-74's R29): the mixer's signal
    /// into the filter's input pair raised by `gain`, after C27, driving the pair harder than
    /// the panel's mixer can ([`crate::vcf::Drive::gain`]); 1 is the circuit.
    pub fn set_drive(&mut self, gain: f64) {
        self.audio.back.vcf.set_drive(gain);
    }

    /// The three oscillators started together `at` one share of the way down their ramps
    /// ([`Vco::start_at`]), as the reference starts them together at the top: for a voice made
    /// or put back to rest that is not the first, so that voices playing one note together do
    /// not start in step with each other (decisions.md R45). Never at a note.
    pub fn start_oscillators_at(&mut self, at: f64) {
        for v in &mut self.audio.front.vcos {
            v.start_at(at);
        }
    }

    /// Seeds the noise generator (each device its own stream; a render repeats).
    pub fn set_seed(&mut self, seed: u64) {
        self.ctl.noise.reseed(seed);
        self.audio.front.hiss = Gauss::new(seed ^ HISS_SEED);
    }

    pub fn rate(&self) -> f64 {
        self.audio.front.rate
    }

    /// The contours' latest values (filter, loudness), V, and whether a key is held.
    pub fn envelopes(&self) -> (f64, f64, bool) {
        (self.ctl.env_to.0, self.ctl.env_to.1, self.keys.held())
    }

    /// What moves slowly once no key is down: the keyboard's output (its hold capacitor, and
    /// the keys beyond the string's ends, as GLIDE has them), V, and the contours (filter,
    /// loudness), V, at their latest ticks; None while a key is held or waits to retrigger, a
    /// released key's pitch contact is still closed, or the trigger contact is. A voice whose
    /// output is silent and whose slow state stands still plays nothing that time would
    /// change but its free-running oscillators' and noise's phases (the plug-in lets such a
    /// voice sleep: decisions.md R68, the development line's R30).
    pub fn slow_state(&self) -> Option<[f64; 3]> {
        if self.keys.held() || !self.keys.releasing.is_empty() || self.keys.gate {
            return None;
        }
        Some([self.kbd.to, self.ctl.env_to.0, self.ctl.env_to.1])
    }

    /// Whether the trigger contact was closed at the last sample: a key down (RETRIGGER's
    /// once it has come down). It closing is a new attack's start, the contours following
    /// it.
    pub fn trigger_closed(&self) -> bool {
        self.keys.gate
    }

    /// Prepare a comparator component set during lab/setup, never on the audio thread.
    pub fn prepare_pulse(
        &mut self,
        circuit: crate::pulse::PulseCircuit,
    ) -> Result<(), crate::mna::NoConvergence> {
        for v in &mut self.audio.front.vcos {
            v.prepare_pulse(circuit)?;
        }
        Ok(())
    }
    /// The last sample's keyboard voltage, V, the mixer bus's Norton current, A, and the
    /// filter's output, V.
    pub fn probe(&self) -> (f64, f64, f64) {
        self.audio.last
    }

    /// The last sample's pitch path: the keyboard's voltage, V, and each oscillator's
    /// timing current, A.
    pub fn probe_pitch(&self) -> (f64, [f64; 3]) {
        (self.audio.last.0, self.audio.front.last_i)
    }

    /// Oscillator `n`'s (0, 1, 2) latest reset: how long before the latest sample, exact, and
    /// how many it has made (a probe of the oscillators timed: decisions.md R-OSC-TIMED).
    pub fn probe_reset_ago(&self, n: usize) -> (f64, u64) {
        self.audio.front.vcos[n].reset_ago()
    }

    /// Oscillator `n`'s (0, 1, 2) latest reset: its time in samples and how many it has
    /// made (a probe of its frequency at any pitch: the resets' spacing).
    pub fn probe_resets(&self, n: usize) -> (f64, u64) {
        self.audio.front.vcos[n].last_reset()
    }

    /// The last sample's MOD share of CONTINUOUS's travel, 0..1 (decisions.md
    /// R-CONTINUOUS-WAVE): 0 with no oscillator's MOD on.
    pub fn probe_shape_mod(&self) -> f64 {
        self.audio.front.mod_u.share(ModDirection::Onward)
    }

    /// Each oscillator's CONTINUOUS shape where the last sample had it (its switch's steps), MOD
    /// and all; `None` for a STEPPED one or one not computed.
    pub fn probe_shapes(&self) -> [Option<f64>; 3] {
        self.audio.front.shape_now
    }

    /// The last sample's contours (filter, loudness), V.
    pub fn probe_contours(&self) -> (f64, f64) {
        self.last_ctl
    }

    /// Holds the pitch and contour paths (the keyboard circuit, the contours) at a quality
    /// whatever the panel's, or lets them follow it again (`None`): for measuring a
    /// quality mode's audio path apart (`ca72-lab hifi`).
    pub fn hold_control_quality(&mut self, q: Option<Quality>) {
        self.control_quality = q;
    }

    /// Each oscillator's tuning (the factory procedure's result).
    pub fn tunings(&self) -> [Tuning; 3] {
        self.audio.front.tunings
    }

    /// The last sample's filter current (the ladder's I0, A), its control node's voltage and
    /// the modulation line's voltage, V.
    pub fn filter_probe(&self) -> (f64, f64, f64) {
        (
            self.audio.front.i0,
            self.audio.front.filter_node,
            self.audio.front.line,
        )
    }

    /// The noise source's calibrated density, V per root hertz.
    pub fn noise_density(&self) -> f64 {
        self.ctl.noise.density()
    }

    /// The reverse sawtooth stage (for its counter of failed steps).
    pub fn revsaw(&self) -> &RevSaw {
        &self.audio.front.revsaw
    }

    /// The keyboard circuit (for its counters of failed steps).
    pub fn keyboard(&self) -> &Keyboard {
        &self.kbd.keyboard
    }

    /// A MIDI note (0 to 127) pressed or released: beyond the 44 keys the string continues
    /// ([`Keys::step`]).
    pub fn note(&mut self, midi: i32, on: bool) {
        self.keys.note(midi, on, self.panel.retrigger);
    }

    /// One output sample.
    pub fn tick(&mut self) -> f64 {
        self.tick_in(0.0)
    }

    /// The external preamplifier (for profiles).
    pub fn preamp(&self) -> &crate::preamp::Preamp {
        &self.audio.pre.preamp
    }

    /// Failed solves in the external preamplifier (tests require none).
    pub fn preamp_failed(&self) -> usize {
        self.audio.pre.preamp.failed
    }

    /// The external preamplifier's output, the last sample's (V; for tests).
    pub fn ext_probe(&self) -> f64 {
        self.audio.pre.ext_amp
    }

    /// The OVERLOAD lamp: 0 off, 1 fully lit (it lights while the external input's
    /// preamplifier is driven near its limits, and holds a moment).
    pub fn overload(&self) -> f64 {
        self.audio.pre.overload
    }

    /// The OVERLOAD lamp's brightest since the last call (for a display that reads it now
    /// and then and should not miss a flash).
    pub fn take_overload_peak(&mut self) -> f64 {
        self.audio.take_overload_peak()
    }

    /// Whether [`crate::threaded::Helpers`] play the voice: at High Fidelity or No Compromises,
    /// with FEEDBACK off, its pitch and contour paths following its panel.
    pub fn splits(&self) -> bool {
        matches!(
            self.panel.quality,
            Quality::NoCompromises | Quality::HighFidelity
        ) && self.feedback == 0.0
            && self.control_quality.is_none()
    }

    /// The voice lent to [`crate::threaded::Helpers`] for a run: what its threads run (the
    /// preamplifier set as [`Voice::tick_jacks`] sets it with FEEDBACK off), and what the
    /// caller's does.
    pub(crate) fn lend(&mut self) -> Lent<'_> {
        self.audio.pre.accurate = false;
        self.audio.pre.in_loop = self.in_loop;
        Lent {
            panel: &self.panel,
            keys: &mut self.keys,
            kbd: &mut self.kbd,
            ctl: &mut self.ctl,
            bias: &mut self.bias,
            pre: &mut self.audio.pre,
            front: &mut self.audio.front,
            back: &mut self.audio.back,
        }
    }

    /// After a run lent ([`Voice::lend`]): the last sample's contours, probes and output, as
    /// [`Voice::tick_jacks`] keeps them.
    pub(crate) fn lent_ran(&mut self, ctl: (f64, f64), probe: (f64, f64, f64), y: f64) {
        self.last_ctl = ctl;
        self.audio.last = probe;
        self.fed_back = y;
    }

    /// The voice's parts, for running them on threads of their own
    /// ([`crate::threaded`]).
    pub fn into_parts(self) -> (Panel, Keys, KeyboardPart, ControlPart, BiasPart, AudioPart) {
        (
            self.panel, self.keys, self.kbd, self.ctl, self.bias, self.audio,
        )
    }

    /// One output sample with `ext` at the EXTERNAL INPUT jack (as a fraction of
    /// [`INPUT_VOLTS`]).
    pub fn tick_in(&mut self, ext: f64) -> f64 {
        self.tick_jacks(&Jacks {
            ext,
            ..Jacks::default()
        })
    }

    /// One output sample with these signals at the jacks (and, with [`Voice::feedback`],
    /// the last output sample added at EXTERNAL INPUT).
    pub fn tick_jacks(&mut self, j: &Jacks) -> f64 {
        self.audio.pre.accurate = self.feedback > 0.0;
        self.audio.pre.in_loop = self.in_loop;
        // (At one rate the loop is closed inside the audio path: decisions.md
        // R-FEEDBACK-LOOP.)
        let y = if self.feedback > 0.0 && !loops(&self.panel, self.feedback) {
            self.tick_at(&Jacks {
                ext: j.ext + self.feedback * self.fed_back,
                ..*j
            })
        } else {
            self.tick_at(j)
        };
        self.fed_back = y;
        y
    }

    /// Whether [`Voice::filter_pair`] solves this voice's filter beside another's (HI). Not
    /// with FEEDBACK's loop closed ([`tick_looped`]), which runs the filter a
    /// step at a time with its own preamplifier and VCA, so a FEEDBACK voice plays alone.
    pub fn pairs(&self) -> bool {
        self.panel.quality == Quality::Potato && !loops(&self.panel, self.feedback)
    }

    /// [`Voice::tick_jacks`] up to the filter (decisions.md R-HI-PAIRS): the keyboard, the
    /// contours, the VCA's bias, the preamplifier and the front, the filter set up; then
    /// [`Voice::filter_pair`] and [`Voice::after_filter`]. Together the same samples to the
    /// bit as [`Voice::tick_jacks`].
    pub fn before_filter(&mut self, j: &Jacks) -> Halfway {
        self.audio.pre.accurate = self.feedback > 0.0;
        self.audio.pre.in_loop = self.in_loop;
        let j = if self.feedback > 0.0 && !loops(&self.panel, self.feedback) {
            Jacks {
                ext: j.ext + self.feedback * self.fed_back,
                ..*j
            }
        } else {
            *j
        };
        let p = self.panel;
        let mut c = self.keys.step(p.priority);
        c.s_trig = j.s_trig;
        let pc = match self.control_quality {
            Some(quality) => Panel { quality, ..p },
            None => p,
        };
        let v_kbd = self.kbd.tick(&pc, c);
        // (The contours' delays follow the audio path's quality, as in `tick_at`.)
        let ctl = self.ctl.tick_for(&pc, c, p.quality);
        self.last_ctl = (ctl.env_f, ctl.env_l);
        let d = self.bias.tick(ctl.env_l, j.loudness, p.quality);
        if loops(&p, self.feedback) {
            let (f, y, out) = tick_looped(
                &mut self.audio.pre,
                &mut self.audio.front,
                &mut self.audio.back,
                (&p, &j, v_kbd, &ctl, &d),
                self.feedback,
            );
            self.audio.back.ready = Some(out);
            return Halfway {
                p,
                f,
                d,
                v_kbd,
                looped: Some(y),
            };
        }
        let i_pre = self.audio.pre.tick(&p, j.ext);
        let f = self.audio.front.tick(&p, &j, v_kbd, &ctl, i_pre);
        self.audio.back.set_filter(&p);
        Halfway {
            p,
            f,
            d,
            v_kbd,
            looped: None,
        }
    }

    /// Two voices' filters over the sample, from [`Voice::before_filter`]: solved side by
    /// side where both play HI. Otherwise each voice's back runs whole, as
    /// [`Voice::tick_jacks`] runs it (ULTRA's VCA takes the filter's own samples: decisions.md
    /// R-BLEP), its output kept for [`Voice::after_filter`].
    pub fn filter_pair(a: &mut Voice, ha: &Halfway, b: &mut Voice, hb: &Halfway) -> (f64, f64) {
        if !(a.pairs() && b.pairs() && ha.p.quality == hb.p.quality) {
            let alone = |v: &mut Voice, h: &Halfway| {
                if let Some(y) = h.looped {
                    return y;
                }
                let (y, out) = v.audio.back.tick(&h.p, &h.f, &h.d);
                v.audio.back.ready = Some(out);
                y
            };
            return (alone(a, ha), alone(b, hb));
        }
        let _fast = crate::ulp::FastScope::new(ha.p.quality != Quality::NoCompromises);
        let x = |h: &Halfway| (h.f.i_bus, h.f.g_bus, h.f.i0);
        let (ba, bb) = (&mut a.audio.back, &mut b.audio.back);
        // Each filter's own samples kept for its VCA, BackPart::after_filter.
        Vcf::tick_pair_oversampled(
            &mut ba.vcf,
            &mut bb.vcf,
            x(ha),
            x(hb),
            (&mut ba.xs, &mut bb.xs),
        )
    }

    /// [`Voice::tick_jacks`] from the filter's output `y` on: the VCA; the output sample.
    pub fn after_filter(&mut self, h: &Halfway, y: f64) -> f64 {
        let out = match self.audio.back.ready.take() {
            Some(out) => out,
            None => self.audio.back.after_filter(&h.p, y, &h.d),
        };
        self.audio.last = (h.v_kbd, h.f.i_bus, y);
        self.fed_back = out;
        out
    }

    fn tick_at(&mut self, j: &Jacks) -> f64 {
        let p = self.panel;
        let mut c = self.keys.step(p.priority);
        c.s_trig = j.s_trig;
        // (The pitch and contour paths at another quality, when it is held.)
        let pc = match self.control_quality {
            Some(quality) => Panel { quality, ..p },
            None => p,
        };
        let v_kbd = self.kbd.tick(&pc, c);
        let ctl = self.ctl.tick_for(&pc, c, p.quality);
        self.last_ctl = (ctl.env_f, ctl.env_l);
        let d = self.bias.tick(ctl.env_l, j.loudness, p.quality);
        if loops(&p, self.feedback) {
            self.audio
                .tick_looped(&p, j, v_kbd, &ctl, &d, self.feedback)
        } else {
            self.audio.tick(&p, j, v_kbd, &ctl, &d)
        }
    }
}

/// The contour generators' settings from the panel, EXT. S-TRIG open or closed.
fn contour_panel(p: &Panel, s_trig: bool, pots: &mut [Memo<f64>; 4]) -> ContourPanel {
    let [a0, d0, a1, d1] = pots;
    type Law = [(f64, f64)];
    let k = |c: ContourKnobs, a: &mut Memo<f64>, d: &mut Memo<f64>, la: &Law, ld: &Law| Controls {
        attack: a.get(c.attack, |x| time_pot(x, la)),
        decay: d.get(c.decay, |x| time_pot(x, ld)),
        sustain: sustain_track(c.sustain),
    };
    ContourPanel {
        filter: k(p.filter_contour, a0, d0, &FILTER_ATTACK, &FILTER_DECAY),
        loudness: k(
            p.loudness_contour,
            a1,
            d1,
            &LOUDNESS_ATTACK,
            &LOUDNESS_DECAY,
        ),
        decay_on: p.decay,
        s_trig,
    }
}

impl Keys {
    /// Whether the trigger contact was closed at the last step ([`Voice::trigger_closed`]).
    pub(crate) fn gate(&self) -> bool {
        self.gate
    }

    /// A MIDI note (0 to 127) pressed or released: the 44 keys F to C (41 to 84) are the
    /// instrument's, the rest continue its key string ([`Keys::step`]). With `retrigger`, a key
    /// pressed waits until the trigger contact has been open [`RETRIGGER_GAP`], opening it
    /// first if keys are held (as if every key were let go and pressed again).
    pub fn note(&mut self, midi: i32, on: bool, retrigger: bool) {
        if !MIDI_NOTES.contains(&midi) {
            return;
        }
        let k = midi - LOWEST_KEY_MIDI;
        if on {
            if !self.held.contains(&k) && !self.waiting.contains(&k) {
                if retrigger {
                    self.waiting.push(k);
                } else {
                    self.held.push(k);
                }
                self.presses += 1;
                self.pressed[midi as usize] = self.presses;
            }
            self.releasing.retain(|&(x, _)| x != k);
        } else if self.waiting.contains(&k) {
            // (Let go before it came down: it never sounded.)
            self.waiting.retain(|&x| x != k);
        } else if self.held.contains(&k) {
            self.held.retain(|&x| x != k);
            let until = self.samples + (RELEASE_LEAD * self.rate).round() as u64;
            self.releasing.push((k, until));
        }
    }

    /// The contacts for the next sample (the lowest and highest keys whose pitch contacts
    /// are closed: held, or released within the lead; and the trigger contact), and a
    /// sample on. With `priority` HIGH or LAST, the key it chooses among them alone, as if
    /// it were the only key down (the trigger contact as ever: closed while any is held).
    ///
    /// Keys beyond the string's ends (the plug-in's extension: decisions.md, "The full MIDI range"): the
    /// trigger contact closes for them as for the rest. While the lowest key whose contact
    /// is closed is below F, it sounds alone: the circuit plays F, and the keys between F
    /// and it are added to the keyboard's output ([`KeyboardPart::tick`]); while every such
    /// key is above C, the lowest of them likewise, from C. Otherwise the keys above C are
    /// left out, as the instrument's lowest-note priority leaves them silent, and the
    /// contacts are the instrument's own.
    pub fn step(&mut self, priority: Priority) -> Contacts {
        let now = self.samples;
        self.releasing.retain(|&(_, until)| until > now);
        // RETRIGGER's keys come down once the trigger contact has been open long enough for
        // the contours to start again; until then it is open, the keys held as they were.
        if !self.waiting.is_empty()
            && !self.gate
            && self.opened.is_none_or(|at| now - at >= self.gap)
        {
            self.held.append(&mut self.waiting);
        }
        let gate = !self.held.is_empty() && self.waiting.is_empty();
        if self.gate && !gate {
            self.opened = Some(now);
        }
        self.gate = gate;
        let keys = || {
            self.held
                .iter()
                .copied()
                .chain(self.releasing.iter().map(|&(k, _)| k))
        };
        let on = || keys().filter(|k| (0..KEYS).contains(k));
        let top = KEYS - 1;
        // A key alone: its own contact, or beyond the string's ends, that end's key.
        let alone = |k: i32| {
            let end = k.clamp(0, top);
            (Some((end as usize, end as usize)), k - end)
        };
        let chosen = match priority {
            Priority::Low => None,
            Priority::High => Some(keys().max()),
            Priority::Last => {
                Some(keys().max_by_key(|&k| self.pressed[(k + LOWEST_KEY_MIDI) as usize]))
            }
        };
        let (keys, foot) = match (chosen, keys().min(), on().min().zip(on().max())) {
            (Some(Some(k)), _, _) => alone(k),
            (Some(None), _, _) | (None, None, _) => (None, self.foot),
            (None, Some(lowest), _) if lowest < 0 => (Some((0, 0)), lowest),
            (None, _, Some((lo, hi))) => (Some((lo as usize, hi as usize)), 0),
            (None, Some(lowest), None) => alone(lowest),
        };
        self.foot = foot;
        let c = Contacts {
            keys,
            gate,
            s_trig: false,
            foot,
        };
        self.samples += 1;
        c
    }

    /// Room for every key, so a key press does not allocate (a clone keeps the lists'
    /// contents, not their room).
    pub fn reserve(&mut self) {
        self.held.reserve(128);
        self.releasing.reserve(128);
        self.waiting.reserve(128);
    }

    /// Whether a key is held (or pressed and waiting to come down).
    pub fn held(&self) -> bool {
        !self.held.is_empty() || !self.waiting.is_empty()
    }
}

impl KeyboardPart {
    /// One sample of the keyboard circuit's output, V, for the contacts, GLIDE and load (at
    /// the circuit's sub-rate the contacts are read at its ticks and the output
    /// interpolated between them).
    pub fn tick(&mut self, p: &Panel, c: Contacts) -> f64 {
        let _fast = crate::ulp::FastScope::new(p.quality != Quality::NoCompromises);
        let mut laps = crate::prof::Laps::start();
        if self.count.is_multiple_of(self.every) {
            // The sub-rate as the quality mode has it, changed at a tick.
            let every = keyboard_every(p.quality);
            if every != self.every {
                self.every = every;
                self.count = 0;
                self.keyboard.set_rate(self.rate / every as f64);
            }
            self.keyboard.set_quality(p.quality);
            self.keyboard.set_glide_long(p.glide_long);
            self.keyboard.set_glide(glide_r(p.glide, p.glide_on));
            self.keyboard.set_load(keyboard_load(p));
            self.from = self.to;
            self.to = self.keyboard.tick(c.keys, c.gate);
            if c.foot != 0 || self.beyond != 0.0 {
                self.to += self.beyond(p, c);
            }
        }
        self.count += 1;
        let v_kbd = if self.every == 1 {
            self.to
        } else {
            let a = ((self.count - 1) % self.every + 1) as f64 / self.every as f64;
            self.from + (self.to - self.from) * a
        };
        laps.lap(crate::prof::Part::Keyboard);
        v_kbd
    }

    /// The keys beyond the string's ends, in volts at the keyboard's output, a tick on
    /// (decisions.md, "The full MIDI range": the plug-in's extension, not the circuit's). The circuit plays the
    /// end key, and this adds the keys between it and the key sounding at the output's own
    /// volts a key, so the scale continues. While the trigger contact is closed it moves as
    /// the circuit's output does, through R61 and GLIDE onto the hold capacitor C6; while it
    /// is open, it holds, as C6 does.
    fn beyond(&mut self, p: &Panel, c: Contacts) -> f64 {
        let target = self.step * f64::from(c.foot);
        if c.gate {
            let k = &self.keyboard.circuit;
            let farads = if p.glide_long { k.c6 * 5.0 } else { k.c6 };
            let tau = (k.r61 + glide_r(p.glide, p.glide_on)) * farads;
            let h = self.every as f64 / self.rate;
            let a = crate::ulp::exp(-h / tau);
            self.beyond = target + (self.beyond - target) * a;
            if (self.beyond - target).abs() < 1e-12 {
                self.beyond = target;
            }
        }
        self.beyond
    }
}

/// The keyboard circuit's sub-rate (samples a tick) in each quality mode: in Potato an
/// eighth of the sample rate, a step of 167 us at 48 kHz, its output interpolated between
/// (glides and key changes lag by up to a tick).
fn keyboard_every(q: Quality) -> usize {
    match q {
        Quality::NoCompromises | Quality::HighFidelity => 1,
        Quality::Potato => 8,
    }
}

/// The oscillators' and the filter's oversampling in each quality mode: in Potato the
/// oscillators at the sample rate and the filter at four times it. At 2x, high EMPHASIS
/// and AMOUNT OF CONTOUR can turn ultrasonic regeneration into audible aliasing and
/// solver crackle. Keep the filter's rate fixed while the controls move.
fn oversampling(q: Quality) -> (usize, usize) {
    match q {
        Quality::NoCompromises | Quality::HighFidelity => (4, 4),
        Quality::Potato => (1, 4),
    }
}

/// How late the voice's sound comes out at a quality, samples: the oscillators'
/// band-limiting and decimators, and the filter's resamplers either side of it
/// (decisions.md R-LATENCY; the circuit's own delays, the filter's phase among them, are
/// the sound's, not this). 60.5 in Potato, 85.75 in the others.
pub fn latency(q: Quality) -> f64 {
    let (to_filter, out) = filter_resamplers(q);
    oscillator_latency(q) + to_filter + out
}

/// The oscillators' outputs' delay at a quality, samples (their [`crate::vco::Vco::latency`]):
/// 16 in Potato, 41.25 in the others.
fn oscillator_latency(q: Quality) -> f64 {
    use crate::resample::{BLEP_DELAY, Decimator, OSCILLATOR_LAST_HALFBAND};
    let os = oversampling(q).0;
    BLEP_DELAY as f64 / os as f64 + Decimator::delay_of(os, OSCILLATOR_LAST_HALFBAND)
}

/// The filter's resamplers' delays at a quality, samples (its input's interpolator, its
/// output's decimator: [`crate::vcf::Vcf::latency`] together): 22.25 and 22.25.
fn filter_resamplers(q: Quality) -> (f64, f64) {
    use crate::resample::{Decimator, Interpolator, LAST_HALFBAND};
    let os = oversampling(q).1;
    (
        Interpolator::delay_of(os),
        Decimator::delay_of(os, LAST_HALFBAND),
    )
}

/// How many samples HI's and LO's sound (Potato's and the light model's) is held back at the
/// plug-in's output so that it comes out with ULTRA's (decisions.md R-LATENCY): ULTRA's
/// oscillators' sound reaches the output 26.0 samples after HI's and 39.0 after LO's at
/// 44.1, 48 and 96 kHz, measured (the plug-in's `tests/latency.rs`, which holds it): 25.25
/// and 38.5 of it the oscillators' and the filters' resamplers ([`latency`], the light
/// model's filter's), the rest the filters' own phase, a little less in the cheaper models.
pub const LINE_UP: (usize, usize) = (26, 39);

/// How late the contours reach what they shape, samples (decisions.md R-LATENCY), less the
/// delay of their own sub-rate (`every` samples a tick, interpolated: `every` - 1):
/// - the filter contour meets its sound at the filter's input: the oscillators' delayed by
///   the filter's input resampler;
/// - the loudness contour reaches the VCA when ULTRA's does once the output is lined up
///   ([`LINE_UP`]): in ULTRA as late as its whole path ([`latency`]), in Potato that less the
///   samples its output is held back. So every mode's attack comes out with ULTRA's, and
///   each meets its sound as in the circuit (the filter's own phase still the circuit's).
///
/// In HI and ULTRA the VCA's bias reaches the output [`vca_bias_latency`] later, through its VCA's
/// decimator, so its loudness contour comes that much earlier.
///
/// At 48 kHz: 31.25 and 29.75 in Potato, 62.5 and 61.75 in the others.
fn contour_delays(audio: Quality, every: usize) -> (f64, f64) {
    let own = every as f64 - 1.0;
    let (to_filter, _) = filter_resamplers(audio);
    let held = if audio == Quality::Potato {
        LINE_UP.0 as f64
    } else {
        0.0
    };
    (
        (oscillator_latency(audio) + to_filter - own).max(0.0),
        (latency(Quality::NoCompromises) - held - vca_bias_latency(audio) - own).max(0.0),
    )
}

/// ULTRA's VCA's oversampling (decisions.md R-BLEP): the filter's.
const ULTRA_VCA_OVERSAMPLING: usize = 4;

/// Whether the VCA runs at the filter's rate: HI (Potato), ULTRA (High Fidelity),
/// and the No Compromises reference. LO uses the light model.
fn vca_oversampled(q: Quality) -> bool {
    matches!(
        q,
        Quality::NoCompromises | Quality::HighFidelity | Quality::Potato
    )
}

/// How much later than at the sample rate the VCA's bias reaches the output at a quality,
/// samples: in ULTRA through its VCA's decimator ([`OversampledVca::bias_latency`], 23), so
/// its loudness contour comes that much earlier; in the others none.
fn vca_bias_latency(q: Quality) -> f64 {
    if vca_oversampled(q) && oversampling(q).1 == ULTRA_VCA_OVERSAMPLING {
        OversampledVca::bias_latency(ULTRA_VCA_OVERSAMPLING)
    } else {
        0.0
    }
}

/// The contours' past outputs kept, samples (more than [`contour_delays`] in any mode).
const ENV_PAST: usize = 128;

/// The contours' sub-rate (samples a tick) in each quality mode: in Potato a quarter of the
/// others'.
fn contour_every(q: Quality) -> usize {
    match q {
        Quality::NoCompromises | Quality::HighFidelity => 2,
        Quality::Potato => 8,
    }
}

impl ControlPart {
    /// One sample of the contours (at their sub-rate, interpolated: a block's delay) and
    /// the noise (its loads follow the panel), for the trigger contact and EXT. S-TRIG.
    pub fn tick(&mut self, p: &Panel, c: Contacts) -> ControlOut {
        self.tick_for(p, c, p.quality)
    }

    /// [`ControlPart::tick`] for an audio path at `audio`'s quality, which the contours'
    /// delays follow ([`contour_delays`]), whatever quality the contours run at
    /// ([`Voice::hold_control_quality`]).
    pub fn tick_for(&mut self, p: &Panel, c: Contacts, audio: Quality) -> ControlOut {
        let _fast = crate::ulp::FastScope::new(p.quality != Quality::NoCompromises);
        let mut laps = crate::prof::Laps::start();
        if self.count.is_multiple_of(self.contour_every) {
            // The sub-rate as the quality mode has it, changed at a sub-sample's start.
            let every = contour_every(p.quality);
            if every != self.contour_every {
                self.contour_every = every;
                self.count = 0;
                self.contours.set_step(every as f64 / self.rate);
            }
            self.env_from = self.env_to;
            let cp = contour_panel(p, c.s_trig, &mut self.pots);
            self.contours.set_quality(p.quality);
            let o = self.contours.tick(c.gate, &cp);
            self.env_to = (o.filter, o.loudness);
        }
        laps.lap(crate::prof::Part::Contours);
        self.count += 1;
        let a = ((self.count - 1) % self.contour_every + 1) as f64 / self.contour_every as f64;
        let env_f = self.env_from.0 + (self.env_to.0 - self.env_from.0) * a;
        let env_l = self.env_from.1 + (self.env_to.1 - self.env_from.1) * a;
        // Each as late as the sound it shapes (interpolated between samples).
        self.env_at = (self.env_at + 1) % ENV_PAST;
        self.env_past[self.env_at] = (env_f, env_l);
        let (late_f, late_l) = contour_delays(audio, self.contour_every);
        let at = |late: f64| {
            let (whole, part) = (late as usize, late.fract());
            let k = |k: usize| self.env_past[(self.env_at + ENV_PAST - k) % ENV_PAST];
            let (e0, e1) = (k(whole), k(whole + 1));
            (e0, e1, part)
        };
        let (f0, f1, part) = at(late_f);
        let env_f = f0.0 + (f1.0 - f0.0) * part;
        let (l0, l1, part) = at(late_l);
        let env_l = l0.1 + (l1.1 - l0.1) * part;
        // Board 3: the noise. Not computed while nothing hears it: its mixer channel off and
        // the modulation mix taking none of it (its gain exactly 0) or neither modulation
        // switch on; it resumes where it stopped (decisions.md R11). The front then takes
        // its last outputs, which reach nothing: the channel switched off adds no current,
        // and nothing times them in the modulation mix is zero.
        let heard = p.noise_on
            || ((p.osc_mod || p.filter_mod || p.any_shape_mod())
                && self
                    .noise_gain
                    .get(p.mod_mix, |m| self.mix.mix(m).gain_noise)
                    != 0.0);
        let noise = if heard {
            self.noise.set_quality(p.quality);
            self.noise.set_loads(noise_loads(p));
            self.last_noise = self.noise.tick();
            self.last_noise
        } else {
            self.last_noise
        };
        laps.lap(crate::prof::Part::Noise);
        ControlOut {
            env_f,
            env_l,
            noise,
        }
    }
}

impl BiasPart {
    /// The VCA's bias as it stands (before the next sample).
    pub fn drive(&self) -> VcaDrive {
        self.vca.drive()
    }

    /// One sample of the VCA's bias on the loudness contour `env_l` (V) and EXT. LOUDNESS
    /// (`None` empty), in quality mode `q`.
    pub fn tick(&mut self, env_l: f64, loudness: Option<f64>, q: Quality) -> VcaDrive {
        let _fast = crate::ulp::FastScope::new(q != Quality::NoCompromises);
        let mut laps = crate::prof::Laps::start();
        // EXT. LOUDNESS (J3) from its jack's source, or its normal contact's 33K to +10 V.
        let (ext_l, r_j3) = match loudness {
            Some(v) => (v, 0.0),
            None => self.j3_normal,
        };
        if (self.vca.circuit.ext, self.vca.circuit.r_j3) != (ext_l, r_j3) {
            self.vca.circuit.ext = ext_l;
            self.vca.circuit.r_j3 = r_j3;
        }
        // With the jack plugged its tail can move at audio rate, which the VCA's own refresh
        // (it watches the contour) does not see: the bias is solved every sample
        // (vca_realtime.rs).
        // (In Potato from a table instead: `Vca::control`.)
        self.vca.potato_on = q == Quality::Potato;
        self.vca.control_every = if loudness.is_some() && q != Quality::Potato {
            1
        } else {
            self.vca_every
        };
        let d = self.vca.control(env_l);
        laps.lap(crate::prof::Part::VcaBias);
        d
    }
}

impl AudioPart {
    /// The OVERLOAD lamp after the last sample (0 off, 1 fully lit).
    pub fn overload(&self) -> f64 {
        self.pre.overload
    }

    /// The OVERLOAD lamp's brightest since the last call.
    pub fn take_overload_peak(&mut self) -> f64 {
        std::mem::take(&mut self.pre.overload_peak)
    }

    /// One output sample (see [`Voice::tick_jacks`]) on the contours `c` and the VCA's bias
    /// `d`: the preamplifier, the front, the back.
    pub fn tick(&mut self, p: &Panel, j: &Jacks, v_kbd: f64, c: &ControlOut, d: &VcaDrive) -> f64 {
        let i_pre = self.pre.tick(p, j.ext);
        let f = self.front.tick(p, j, v_kbd, c, i_pre);
        let (y, out) = self.back.tick(p, &f, d);
        self.last = (v_kbd, f.i_bus, y);
        out
    }

    /// [`AudioPart::tick`] with FEEDBACK's loop closed inside it ([`tick_looped`]), where
    /// [`loops`].
    pub fn tick_looped(
        &mut self,
        p: &Panel,
        j: &Jacks,
        v_kbd: f64,
        c: &ControlOut,
        d: &VcaDrive,
        feedback: f64,
    ) -> f64 {
        let (f, y, out) = tick_looped(
            &mut self.pre,
            &mut self.front,
            &mut self.back,
            (p, j, v_kbd, c, d),
            feedback,
        );
        self.last = (v_kbd, f.i_bus, y);
        out
    }
}

/// Whether FEEDBACK's loop runs at one rate ([`tick_looped`]; decisions.md R-FEEDBACK-LOOP):
/// in HI and ULTRA, whose preamplifier and VCA run at the filter's rate, FEEDBACK and the
/// mixer's EXTERNAL INPUT on.
pub(crate) fn loops(p: &Panel, feedback: f64) -> bool {
    feedback > 0.0
        && p.ext_on
        && vca_oversampled(p.quality)
        && oversampling(p.quality).1 == ULTRA_VCA_OVERSAMPLING
}

const _: () = assert!(crate::preamp::SUBSTEPS == ULTRA_VCA_OVERSAMPLING);

/// One sample with FEEDBACK's loop closed at the filter's rate (decisions.md
/// R-FEEDBACK-LOOP): the front without the preamplifier's current; then at each of the
/// filter's steps the VCA on the filter's output, and the preamplifier a substep on the jack's
/// voltage with the cable's share of the VCA's output added, its current into the bus at the
/// filter's next step. Only that step (a quarter of a sample) lies between the output and the
/// input, against the preamplifier's and the filter's resamplers (90 samples) when the output
/// is fed back a sample at a time. `at` is the panel, the jacks, the keyboard's voltage, the
/// contours and the VCA's bias, as [`AudioPart::tick`] takes them. The front's output, the
/// filter's output (V) and the output sample.
pub(crate) fn tick_looped(
    pre: &mut PreampPart,
    front: &mut FrontPart,
    back: &mut BackPart,
    at: (&Panel, &Jacks, f64, &ControlOut, &VcaDrive),
    feedback: f64,
) -> (FrontOut, f64, f64) {
    let (p, j, v_kbd, c, d) = at;
    let f = front.tick(p, j, v_kbd, c, 0.0);
    let _fast = crate::ulp::FastScope::new(p.quality != Quality::NoCompromises);
    pre.preamp.set_quality(match (p.quality, pre.in_loop) {
        (Quality::Potato, InLoop::Circuit) => Quality::HighFidelity,
        (q, _) => q,
    });
    pre.preamp.set_delayed(false);
    let (t, r) = pre.volume.get(p.ext_volume, ext_volume);
    pre.preamp.set_source_resistance(r);
    let ext = pre.preamp.upsample(j.ext * INPUT_VOLTS * t);
    back.set_filter(p);
    let a440 = back.a440.tick(p.a440);
    if !back.ultra {
        back.vca_4x.take_over(&back.vca, back.last_out);
        back.ultra = true;
    }
    back.vca_4x.set_a440(a440);
    back.vca_4x.set_plain(p.quality != Quality::NoCompromises);
    let ramp = back.vca_4x.start(d);
    let mut out = 0.0;
    let BackPart { vcf, vca_4x, .. } = back;
    let preamp = &mut pre.preamp;
    let y = vcf.tick_fed(f.i_bus, f.g_bus, f.i0, &mut pre.i_fed, |k, v| {
        let (y, o) = vca_4x.step(&ramp, k, v);
        if let Some(o) = o {
            out = o;
        }
        preamp.fine(ext[k] + t * feedback * y)
    });
    let o = pre.preamp.fine_outputs();
    pre.overload = o.lamp;
    pre.overload_peak = pre.overload_peak.max(o.lamp);
    pre.ext_amp = o.amp;
    back.last_out = out;
    (f, y, out / 5.0)
}

impl PreampPart {
    /// The external input's preamplifier over one sample with `ext` at the jack (a fraction
    /// of [`INPUT_VOLTS`]): the current SW10 and R46 put into the mixer's bus, A (none
    /// while SW10 is off, when the preamplifier does not run).
    pub fn tick(&mut self, p: &Panel, ext: f64) -> f64 {
        let _fast = crate::ulp::FastScope::new(p.quality != Quality::NoCompromises);
        let mut laps = crate::prof::Laps::start();
        self.i_fed = 0.0;
        self.preamp
            .set_quality(match (self.accurate, p.quality, self.in_loop) {
                (true, Quality::Potato, InLoop::Circuit) => Quality::HighFidelity,
                _ => p.quality,
            });
        self.preamp.set_delayed(
            self.accurate && p.quality == Quality::Potato && self.in_loop == InLoop::Delayed,
        );
        // The external input: R9's divider into the preamplifier, which runs while SW10 is
        // on; R46 loads the bus either way (the front counts its conductance).
        let i = if p.ext_on {
            let (t, r) = self.volume.get(p.ext_volume, ext_volume);
            self.preamp.set_source_resistance(r);
            let o = self.preamp.tick(ext * INPUT_VOLTS * t);
            self.overload = o.lamp;
            self.overload_peak = self.overload_peak.max(o.lamp);
            self.ext_amp = o.amp;
            o.i_bus
        } else {
            self.overload = 0.0;
            0.0
        };
        laps.lap(crate::prof::Part::Preamp);
        i
    }

    /// The OVERLOAD lamp after the last sample, its brightest left for [`PreampPart::lamp`].
    pub(crate) fn lamp_now(&self) -> f64 {
        self.overload
    }

    /// The OVERLOAD lamp after the last sample, and its brightest since the last call to
    /// this (for the threaded voice, which reads them from the preamplifier's thread).
    pub fn lamp(&mut self) -> (f64, f64) {
        (self.overload, std::mem::take(&mut self.overload_peak))
    }
}

impl FrontPart {
    /// The front over one sample: the keyboard's voltage `v_kbd`, the contours and noise
    /// `c`, the preamplifier's current into the bus `i_pre`.
    pub fn tick(
        &mut self,
        p: &Panel,
        j: &Jacks,
        v_kbd: f64,
        c: &ControlOut,
        i_pre: f64,
    ) -> FrontOut {
        let _fast = crate::ulp::FastScope::new(p.quality != Quality::NoCompromises);
        let mut laps = crate::prof::Laps::start();
        let p = *p;
        let (env_f, nz) = (c.env_f, c.noise);
        let mix = self.modulation.mix(p.mod_mix);
        let (noise_to_mod, noise_to_mixer) = if p.noise_pink {
            (nz.red, (nz.pink, 24e3))
        } else {
            (nz.pink, (nz.white, 0.0))
        };
        // The modulation line: the amplifier's output (from the last sample's oscillator 3,
        // computed below: a sample's delay in a path flat to 100 kHz) through R57 against the
        // wheel and the line's loads.
        let amp = mix.offset + mix.gain_noise * noise_to_mod + mix.gain_osc3 * self.last_osc3;
        let line_loads = LineLoads {
            osc_mod: p.osc_mod,
            filter_mod: p.filter_mod,
            filter_node: self.filter_node,
        };
        let r_wheel = self.wheel.get(p.mod_wheel, mod_wheel_r);
        let line = Modulation::line(amp, mix.r_out, r_wheel, &line_loads);
        self.line = line;
        let buses = Buses {
            bend: pitch_wheel_volts(p.pitch_wheel),
            // OSCILLATOR MODULATION off grounds the bus (A21).
            modulation: if p.osc_mod { line } else { 0.0 },
            // The oscillators' external control input (its jack's source holds the bus).
            ext: j.osc.unwrap_or(OPEN_BUS),
        };
        // MOD (the plug-in's, decisions.md R-CONTINUOUS-WAVE): a share of the shape's travel,
        // 0..1, from the line's swing about oscillator 3's triangle's middle, as deep as the
        // line's gain against its gain with the wheel fully forward: oscillator 3's triangle,
        // the wheel forward, sweeps the whole travel; nothing with the wheel back.
        // (The wheel fully back shorts the line: none at all.)
        self.mod_u = if p.any_shape_mod() && r_wheel > 0.1 {
            let g = Modulation::line_gain(mix.r_out, r_wheel, &line_loads);
            let g_full = Modulation::line_gain(mix.r_out, MOD_WHEEL_FULL, &line_loads);
            ModSwing {
                depth: g / g_full,
                swing: (amp - self.mod_ref.0) / self.mod_ref.1,
            }
        } else {
            ModSwing::default()
        };
        // The oscillators and the mixer: each channel a Norton source onto the bus, the
        // external input switched off loads it. Oscillator 3's switch output also feeds the
        // modulation mix through R23: its node is loaded by both.
        laps.lap(crate::prof::Part::Modulation);
        // The mixer's own noise, then the channels.
        let mut i_bus = self.hiss_rms * self.hiss.next_uniform();
        if p.ext_on {
            i_bus += i_pre;
        }
        let mut g_bus = 1.0 / 33e3;
        let (_, mod_load_b) = Modulation::source_loads(p.mod_mix);
        let mut osc3_node = 0.0;
        for n in 0..3 {
            let op = p.osc[n];
            // An oscillator nothing hears is not computed: switched off in the mixer (its
            // channel's 33K then only loads the bus) and, for oscillator 3, not feeding a
            // modulation switch that is on. It resumes where it stopped (a free-running
            // oscillator's phase is arbitrary; performance).
            if !(op.on || (n == 2 && (p.osc_mod || p.filter_mod || p.any_shape_mod()))) {
                g_bus += 1.0 / 33e3;
                continue;
            }
            self.vcos[n].set_oversample(oversampling(p.quality).0);
            self.vcos[n].table_on = p.quality == Quality::Potato;
            // CONTINUOUS (decisions.md R-CONTINUOUS-WAVE): where its shape is now.
            let shape = p.shape[n];
            let at = shape.continuous.then(|| shape_at(shape, self.mod_u));
            self.shape_now[n] = at;
            self.vcos[n].set_outputs(match at {
                None => outputs_of(op.waveform, n),
                Some(at) => shape_outputs(n, at, shape.modulated),
            });
            self.vcos[n].set_width_moves(shape.continuous);
            self.vcos[n].use_width_network(true);
            let width = at.map_or(op.waveform.width(), shape_bias);
            let o = match j.timed[n] {
                // An oscillator timed at LO (the plug-in's: decisions.md R-OSC-TIMED): its
                // resets on the host's clock, none of its pitch inputs or ENTROPY in it.
                Some(t) if op.range == Range::Lo => self.vcos[n].tick_timed(t, width),
                _ => {
                    // (An LFO's knobs where they have glided to since its step: R-LFO-STRESS.)
                    let freq = match n {
                        1 | 2 if j.swept.freq[n - 1] != 0.0 => {
                            op.freq.clamp(0.0, 1.0) + j.swept.freq[n - 1]
                        }
                        _ => op.freq,
                    };
                    let osc = match n {
                        0 => Osc::One,
                        1 => Osc::Two { freq },
                        _ => Osc::Three {
                            freq,
                            control: p.osc3_control,
                        },
                    };
                    let mut tuning = self.tunings[n];
                    let tune = if j.swept.tune != 0.0 {
                        p.tune.clamp(-1.0, 1.0) + j.swept.tune
                    } else {
                        p.tune.clamp(-1.0, 1.0)
                    };
                    tuning.tune += tune * TUNE_HALF_SPAN;
                    let mut d = osc_drive_with(osc, &tuning, v_kbd, &buses, op.range);
                    if j.detune[n] != 0.0 {
                        // ENTROPY (the plug-in's): the oscillator off its calibration, as an
                        // offset at its range tap (R43), which the octave trimmer set to an
                        // octave a step.
                        d.inputs[5].v += j.detune[n] / 1200.0 * tuning.octave_step;
                    }
                    let i_in = d.apply(&mut self.vcos[n].expo);
                    self.vcos[n].tick(i_in, width)
                }
            };
            self.last_i[n] = self.vcos[n].timing_current();
            // Oscillator 3's reverse sawtooth stage runs while its sawtooth or reverse
            // sawtooth is selected (R171 loads the sawtooth).
            // In CONTINUOUS, while its shape takes in the reverse sawtooth: always with MOD.
            let second = match at {
                None => matches!(
                    op.waveform,
                    Waveform::Sawtooth | Waveform::SharkTooth | Waveform::ReverseSawtooth
                ),
                Some(at) => shape.modulated || (at > 0.0 && at < 3.0),
            };
            let rev = if n == 2 && second {
                self.revsaw.set_quality(p.quality);
                Some(self.revsaw.tick(o.saw))
            } else {
                None
            };
            let (v_src, r_src) = match at {
                None => waveform_source(op.waveform, &o, rev.as_ref()),
                Some(at) => shape_source(n, at, &o, rev.as_ref()),
            };
            let (i, g) = if n == 2 {
                // The switch's output node under its two loads.
                let z_ch = channel_load(op.volume, 33e3);
                let z = z_ch * mod_load_b / (z_ch + mod_load_b);
                osc3_node = v_src * z / (r_src + z);
                channel(osc3_node, 0.0, op.volume, op.on, 33e3)
            } else {
                channel(v_src, r_src, op.volume, op.on, 33e3)
            };
            i_bus += i;
            g_bus += g;
        }
        laps.lap(crate::prof::Part::Oscillators);
        self.last_osc3 = osc3_node;
        // The noise's channel (R48 11K).
        let (i, g) = channel(
            noise_to_mixer.0,
            noise_to_mixer.1,
            p.noise_volume,
            p.noise_on,
            11e3,
        );
        i_bus += i;
        g_bus += g;
        // The filter's control node: CUTOFF, AMOUNT OF CONTOUR, KEYBOARD CONTROL 1 and 2,
        // R52 (the modulation line with FILTER MODULATION on, else grounded) and R51 (the
        // external control's jack, empty: grounded).
        let pos = if j.swept.cutoff != 0.0 {
            (cutoff_track(p.cutoff) + j.swept.cutoff).clamp(0.0, 1.0)
        } else {
            cutoff_track(p.cutoff)
        };
        let r_cut = 200e3 + 5e3 * pos * (1.0 - pos);
        let amt = p.contour_amount.clamp(0.0, 1.0);
        let open = Input {
            r: f64::INFINITY,
            v: 0.0,
        };
        let ins = [
            Input {
                r: r_cut,
                v: -10.0 + 20.0 * pos,
            },
            contour_input(amt, env_f),
            if p.keyboard_control_1 {
                Input {
                    r: crate::vcf::R53,
                    v: v_kbd,
                }
            } else {
                open
            },
            if p.keyboard_control_2 {
                Input {
                    r: crate::vcf::R54,
                    v: v_kbd,
                }
            } else {
                open
            },
            Input {
                r: 33e3,
                v: if p.filter_mod { line } else { 0.0 },
            },
            Input {
                r: 100e3,
                v: if j.cutoff == 0.0 {
                    j.filter.unwrap_or(0.0)
                } else {
                    // ENTROPY (the plug-in's): the cutoff off its calibration, as volts at R51.
                    j.filter.unwrap_or(0.0) + j.cutoff * FILTER_VOLTS_PER_OCTAVE
                },
            },
        ];
        let (i_node, g_node) = self.expo.node(&ins);
        let (i0, v_node) = self.table.current_and_node(i_node, g_node);
        self.i0 = i0;
        self.filter_node = v_node;
        laps.lap(crate::prof::Part::ControlNode);
        FrontOut { i_bus, g_bus, i0 }
    }
}

impl BackPart {
    /// The back over one sample: the front's `f` and the VCA's bias `d` (from
    /// [`ControlPart`]). The filter's output, V, and the output sample.
    pub fn tick(&mut self, p: &Panel, f: &FrontOut, d: &VcaDrive) -> (f64, f64) {
        let _fast = crate::ulp::FastScope::new(p.quality != Quality::NoCompromises);
        let mut laps = crate::prof::Laps::start();
        self.vcf.circuit.r14 = self.emphasis.get(p.emphasis, emphasis_r14);
        self.vcf.set_oversample(oversampling(p.quality).1);
        self.vcf.set_quality(p.quality);
        self.vcf.high_pass = p.filter_hi;
        let ultra = vca_oversampled(p.quality) && self.vcf.oversample() == ULTRA_VCA_OVERSAMPLING;
        let a440 = self.a440.tick(p.a440);
        if ultra != self.ultra {
            // QUALITY switched: the other signal path carries on from this one's states.
            if ultra {
                self.vca_4x.take_over(&self.vca, self.last_out);
            } else {
                self.vca_4x.hand_over(&mut self.vca);
            }
            self.ultra = ultra;
        }
        let (y, out) = if ultra {
            // ULTRA: the VCA at the filter's rate on the filter's own samples; the filter's
            // decimator still gives its output (the probe's, and HI's after a switch).
            let mut xs = [0.0f64; 8];
            let y = self.vcf.tick_oversampled(f.i_bus, f.g_bus, f.i0, &mut xs);
            laps.lap(crate::prof::Part::Filter);
            self.vca_4x.set_a440(a440);
            self.vca_4x.set_plain(p.quality != Quality::NoCompromises);
            (y, self.vca_4x.signal(&xs, d))
        } else {
            let y = self.vcf.tick(f.i_bus, f.g_bus, f.i0);
            laps.lap(crate::prof::Part::Filter);
            self.vca.a440 = a440;
            self.vca.plain_signal = p.quality != Quality::NoCompromises;
            (y, self.vca.signal(y, d))
        };
        laps.lap(crate::prof::Part::Vca);
        self.last_out = out;
        (y, out / 5.0)
    }

    /// [`BackPart::tick`] up to the filter: the filter set up for the panel.
    fn set_filter(&mut self, p: &Panel) {
        let _fast = crate::ulp::FastScope::new(p.quality != Quality::NoCompromises);
        self.vcf.circuit.r14 = self.emphasis.get(p.emphasis, emphasis_r14);
        self.vcf.set_oversample(oversampling(p.quality).1);
        self.vcf.set_quality(p.quality);
        self.vcf.high_pass = p.filter_hi;
    }

    /// [`BackPart::tick`] from the filter's output `y` on: the output sample.
    fn after_filter(&mut self, p: &Panel, y: f64, d: &VcaDrive) -> f64 {
        let _fast = crate::ulp::FastScope::new(p.quality != Quality::NoCompromises);
        // Only HI comes here (a pair). Its VCA at the filter's rate on the filter's
        // own samples ([`Vcf::tick_pair_oversampled`]), switching as in `tick`.
        let ultra = vca_oversampled(p.quality) && self.vcf.oversample() == ULTRA_VCA_OVERSAMPLING;
        if ultra != self.ultra {
            if ultra {
                self.vca_4x.take_over(&self.vca, self.last_out);
            } else {
                self.vca_4x.hand_over(&mut self.vca);
            }
            self.ultra = ultra;
        }
        let a440 = self.a440.tick(p.a440);
        let out = if ultra {
            self.vca_4x.set_a440(a440);
            self.vca_4x.set_plain(p.quality != Quality::NoCompromises);
            self.vca_4x.signal(&self.xs, d)
        } else {
            self.vca.a440 = a440;
            self.vca.plain_signal = p.quality != Quality::NoCompromises;
            self.vca.signal(y, d)
        };
        self.last_out = out;
        out / 5.0
    }
}

#[cfg(test)]
mod waveform_source_tests {
    use super::*;

    #[test]
    fn reverse_hybrid_obeys_the_calibrated_passive_network() {
        for (tri, back, source_ohms) in [(1.2, -0.9, 3500.0), (-1.4, 1.1, 1200.0)] {
            let o = VcoOut {
                tri,
                ..VcoOut::default()
            };
            let rev = RevSawOut {
                rev: back,
                r_rev: source_ohms,
                ..RevSawOut::default()
            };
            let (volts, resistance) = shape_source(2, 1.0, &o, Some(&rev));
            for load in [1000.0, 33000.0, 1e6] {
                let at_load = volts * load / (load + resistance);
                let triangle_current =
                    (tri - at_load) / (750.0 * 8200.0 / 8950.0 + SHARK_TRIANGLE_OHMS);
                let reverse_current = (back - at_load) / (source_ohms + SHARK_SAW_OHMS);
                // Kirchhoff's current law at the loaded physical mix node.
                assert!((triangle_current + reverse_current - at_load / load).abs() < 1e-12);
            }
        }
    }
}
