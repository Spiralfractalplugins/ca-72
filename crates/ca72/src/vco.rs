//! One oscillator of board 1 in real time: the exponential converter ([`crate::expo`]),
//! the sawtooth core with its reset, and the wave shapers (docs/circuit/board1.md).
//!
//! How each part is derived:
//! - **Timing current**: [`ExpoCircuit`] solves the converter's circuit equations at the
//!   ramp's two ends once per output sample; the current is linear in the ramp's voltage
//!   (the Early effect, amplified slightly by R42's feedback), so the ramp integrates
//!   exactly between events.
//! - **Core**: C1 plus the ramp node's junction capacitances integrates the current; when
//!   the ramp falls through the Schmitt trigger's threshold the reset follows after the
//!   comparator's delay (it grows as the ramp slows), the ramp jumps to its top (which
//!   falls slightly as the ramp speeds up) and holds while Q10 releases. These constants
//!   are extracted from ngspice runs of the transcribed circuit ([`CoreParams::default`],
//!   `scripts/fit_core.py`); they explain ngspice's period within 0.11 cent from
//!   0.7 Hz to 4.2 kHz.
//! - **Shapers**: the buffer (Q7-Q3) is linear; the sawtooth is its divided output; the
//!   triangle is the DC transfer of Q2/Q1 tabulated from ngspice ([`crate::tables::TRI`]);
//!   the rectangle is Q11/Q12's comparator with its hysteresis, thresholds from ngspice's
//!   DC sweeps.
//! - **Band-limiting** (decisions.md R-BLEP): the core runs at `oversample` times the output
//!   rate; the jumps at the reset and the rectangle's edges get band-limited steps at their
//!   exact times, the reset's areas band-limited impulses, and the slope's jumps (the reset,
//!   the hold's end, the triangle's fold as a rounded corner) band-limited ramps, all from one
//!   windowed sinc ([`crate::resample::BlepLine`]); a halfband decimator brings each output
//!   down, its last stage long enough to keep 20 Hz to 20 kHz clean at 44.1 kHz
//!   ([`Decimator::for_oscillators`]). Only the outputs in use are corrected ([`Vco::set_outputs`]).
//!
//! Outputs are the board's open-circuit voltages (unloaded): the mixer's loading belongs
//! to the mixer's model. The reset's sub-microsecond shape is represented by what it does
//! below 20 kHz: its timing, and the areas it adds to the triangle (the glitch as the
//! buffer sweeps through Q2's fold) and to the sawtooth (the buffer's lag), measured on
//! the reference. Left out: the core's temperature dependence (its thresholds are 25 C's)
//! and noise.

use crate::expo::ExpoCircuit;
use crate::resample::{BLEP_DELAY, BlepLine, Decimator};
use crate::tables;

/// The core's reset constants (oscillator 1, 25 C).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CoreParams {
    /// C1, the timing capacitor (0.01 uF polystyrene), F.
    pub c1: f64,
    /// Capacitance the ramp node adds (Q10's collector junction, Q7's gate, IC2's
    /// collector and substrate junctions), F.
    pub c_par: f64,
    /// Leakage out of the ramp node, A (in the reference, ngspice's gmin across junctions).
    pub i_leak: f64,
    /// The ramp voltage at which the Schmitt trigger fires, for a slow ramp, V.
    pub v_threshold: f64,
    /// The ramp's top after a reset, and how much it falls per V/s of ramp slope.
    pub v_top0: f64,
    pub top_per_slope: f64,
    /// How long the ramp is held at its top while Q10 releases, s.
    pub t_hold: f64,
    /// The comparator's delay after the threshold, against ln(slope in V/s).
    pub delay_ln_slope: [f64; 8],
    pub delay: [f64; 8],
}

impl Default for CoreParams {
    /// Extracted from `circuits/boards/reference/vco1-core.json` by
    /// `scripts/fit_core.py` (ngspice-47, `mm-devices.lib` revision 1).
    fn default() -> Self {
        CoreParams {
            c1: 0.01e-6,
            c_par: 1.594822e-11,
            i_leak: 1.246916e-11,
            v_threshold: -3.970812,
            v_top0: 0.012771,
            top_per_slope: 2.824424e-07,
            t_hold: 1.203511e-06,
            delay_ln_slope: [
                4.4359, 5.18819, 5.94048, 6.69278, 7.44507, 8.19736, 8.94966, 9.70195,
            ],
            delay: [
                8.581449e-06,
                6.858305e-06,
                5.389057e-06,
                4.194429e-06,
                3.246414e-06,
                2.513420e-06,
                1.954638e-06,
                1.532251e-06,
            ],
        }
    }
}

impl CoreParams {
    fn delay_at(&self, slope: f64) -> f64 {
        let x = crate::ulp::log(slope.max(1e-9));
        let g = &self.delay_ln_slope;
        if x <= g[0] {
            return self.delay[0];
        }
        for i in 1..g.len() {
            if x <= g[i] {
                let a = (x - g[i - 1]) / (g[i] - g[i - 1]);
                return self.delay[i - 1] + (self.delay[i] - self.delay[i - 1]) * a;
            }
        }
        self.delay[g.len() - 1]
    }
}

/// The wave shapers' constants (oscillator 1, unloaded, 25 C).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShaperParams {
    /// The buffer: Vbuf = gain Vramp + offset (R24, R25, R4; ngspice DC sweep).
    pub buf_gain: f64,
    pub buf_offset: f64,
    /// The sawtooth output's share of the buffer: R33 / (R33 + R34).
    pub saw_ratio: f64,
    /// The rectangle's comparator: the buffer voltages at which it switches low (falling)
    /// and high (rising) for a width bias of 0 V, and how they move per volt of bias.
    pub rect_fall0: f64,
    pub rect_rise0: f64,
    pub rect_per_width: f64,
    /// The rectangle's two levels: Q12 off (0 V through R22) and on (R22/(R13+R22) of
    /// -10 V less Q12's saturation).
    pub rect_high: f64,
    pub rect_low: f64,
    /// Its edges' timing: it rises this long after the ramp's reset (Q12 leaving
    /// saturation), and falls this long after the ramp has passed the DC threshold by
    /// `rect_fall_overdrive` volts (Q11 and Q12 switching), s and V.
    pub rect_rise_delay: f64,
    pub rect_fall_delay: f64,
    pub rect_fall_overdrive: f64,
    /// What the reset adds beyond an ideal step at its instant, as areas (V s): the
    /// triangle's glitch while the buffer sweeps through Q2's fold, and the buffer's lag.
    pub tri_glitch: f64,
    pub buf_lag: f64,
}

impl Default for ShaperParams {
    fn default() -> Self {
        ShaperParams {
            buf_gain: 2.0346,
            buf_offset: 3.9098,
            saw_ratio: 4.7 / 9.0,
            rect_fall0: -0.0371,
            rect_rise0: 0.3738,
            rect_per_width: -0.9983,
            rect_high: 0.0,
            rect_low: -4.2954,
            rect_rise_delay: 2.74e-6,
            rect_fall_delay: 3.75e-6,
            rect_fall_overdrive: 4.94e-3,
            tri_glitch: -4.02e-7,
            buf_lag: -4.8e-7,
        }
    }
}

/// The triangle shaper's transfer at a buffer voltage (Catmull-Rom interpolation).
pub fn triangle(buf: f64) -> f64 {
    let t = &tables::TRI;
    let x = (buf - tables::TRI_MIN) / tables::TRI_STEP;
    let i = (x.floor() as isize).clamp(1, t.len() as isize - 3) as usize;
    let f = (x - i as f64).clamp(0.0, 1.0);
    let (p0, p1, p2, p3) = (t[i - 1], t[i], t[i + 1], t[i + 2]);
    p1 + 0.5
        * f
        * (p2 - p0 + f * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3 + f * (3.0 * (p1 - p2) + p3 - p0)))
}

/// The triangle shaper's slope against the buffer.
fn triangle_slope(buf: f64) -> f64 {
    const H: f64 = 1e-4;
    (triangle(buf + H) - triangle(buf - H)) / (2.0 * H)
}

/// The triangle's fold (Q2's, [`crate::tables::TRI`]) as a rounded corner (decisions.md
/// R-BLEP): not a sharp corner but a knee, sharp below its lowest point and rounded over
/// about 0.2 V above. The tangents 0.16 V below its lowest point and 0.19 V above meet at
/// its centre; it is taken as turning evenly over 0.15 V of the buffer about there (the
/// lab's sweep of 0 to 0.3 V: as a sharp corner its correction adds a kink where sampling
/// resolves the knee). Gives the ramp's voltage where it starts (the ramp falling), how
/// much the slope against the buffer grows across it, and its width.
fn fold_corner(sh: &ShaperParams) -> (f64, f64, f64) {
    const WIDTH: f64 = 0.15;
    let lowest = (0..tables::TRI.len())
        .min_by(|&i, &j| tables::TRI[i].total_cmp(&tables::TRI[j]))
        .map_or(0.0, |i| tables::TRI_MIN + i as f64 * tables::TRI_STEP);
    let (xl, xr) = (lowest - 0.16, lowest + 0.19);
    let (sl, sr) = (triangle_slope(xl), triangle_slope(xr));
    let (yl, yr) = (triangle(xl), triangle(xr));
    // Where yl + sl (x - xl) = yr + sr (x - xr).
    let x = (yr - yl + sl * xl - sr * xr) / (sl - sr);
    (
        (x + 0.5 * WIDTH - sh.buf_offset) / sh.buf_gain,
        sr - sl,
        WIDTH,
    )
}

/// The most timing current the converter's model gives, A: twice MIDI 127's at the panel's
/// centre (2.47 mA), an octave above the top key the plug-in plays, far past the audio band
/// on any range. Up to it the solve converges; above about 1e-2 A it runs away.
pub const TIMING_MAX: f64 = 5e-3;

/// The timing current at -4 V on the ramp over that at 0 V (the Early effect) at
/// [`TIMING_MAX`], as the converter's solve gives it there.
const EARLY_AT_MAX: f64 = 0.881;

/// One output sample of the oscillator.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct VcoOut {
    pub saw: f64,
    pub tri: f64,
    pub rect: f64,
}

/// Where a timed oscillator's cycle stands (decisions.md R-OSC-TIMED): its phase at the
/// sample (as the sample is given, the end of the time it covers), 0 where its ramp resets and
/// 1 at the next reset, and the cycles a sample takes. The
/// host's clock gives both, so that its resets fall on the clock's grid whatever the circuit
/// would have made of them.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Timed {
    pub phase: f64,
    pub per: f64,
}

/// The timing current's slope against the ramp's voltage, a share of the current a volt: the
/// converter's Early effect, R42's feedback amplifying it, at oscillator 3's LO currents, where
/// it is the same from 1e-11 to 1e-5 A (`the_timed_ramp_bends_as_the_converter_does`). A timed
/// ramp is bent by it as the circuit's is.
pub const TIMED_EARLY: f64 = 0.014493;

/// How far a timed oscillator may stand from its clock, cycles, and be brought back within its
/// cycle (its ramp's slope chosen so that it resets on time); further (the host's place jumped:
/// a locate, a loop, the transport started), it is put where the clock is at once.
pub const TIMED_TOLERANCE: f64 = 0.02;

/// An oscillator of board 1.
#[derive(Debug, Clone)]
pub struct Vco {
    pub expo: ExpoCircuit,
    pub core: CoreParams,
    pub shaper: ShaperParams,
    pulse: std::sync::Arc<crate::pulse::PulseModel>,
    width_network: bool,
    /// Circuit temperature, C (the converter follows it; the core's constants are 25 C's).
    pub celsius: f64,
    oversample: usize,
    dt: f64,
    /// Another oversampling, its decimators prepared ([`Vco::prepare`], [`Vco::set_oversample`]).
    other: usize,
    dec_other: [Decimator; 3],
    /// Potato's converter ([`ConverterTable`], made by [`Vco::prepare`]) and whether it is
    /// in use.
    table: Option<std::sync::Arc<crate::expo::ConverterTable>>,
    pub table_on: bool,
    /// Last table lookup, keyed by the exact drive, trims and temperature.
    table_last: Option<([u64; 4], (f64, f64))>,
    // State.
    v: f64,
    hold_left: f64,
    reset_in: Option<f64>,
    rise_in: Option<f64>,
    fall_in: Option<f64>,
    rect_is_low: bool,
    guess: Option<f64>,
    saw: BlepLine,
    tri: BlepLine,
    rect: BlepLine,
    /// Which outputs are corrected (sawtooth, triangle, rectangle; [`Vco::set_outputs`]).
    outputs: [bool; 3],
    /// Whether the width may move ([`Vco::set_width_moves`]), and the last sample's (NaN
    /// before the first and after a reset).
    width_moves: bool,
    last_width: f64,
    /// The triangle's fold as a rounded corner ([`fold_corner`]): the ramp's voltage where it
    /// starts, how much the triangle's slope against the buffer grows across it, and its
    /// width in the buffer's volts.
    fold: (f64, f64, f64),
    dec: [Decimator; 3],
    /// Output samples since the last `reset`, and the time of the latest ramp reset in
    /// output samples (fractional), with how many there have been.
    ticks: u64,
    last_reset: f64,
    resets: u64,
    sub: usize,
    /// The converter's last two solves with their exact arguments (the drive, the warm
    /// start, the circuit, the temperature): the same arguments give the same currents, so
    /// a steady drive, whose solves settle into a fixed point or a two-cycle, is not solved
    /// again (exactly; performance).
    solved: [Option<Solved>; 2],
    /// The comparator's delay at a timed ramp's last slope, s ([`Vco::tick_timed`]).
    timed_delay: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Solved {
    i_in: u64,
    guess: Option<u64>,
    celsius: u64,
    expo: ExpoCircuit,
    top: f64,
    bottom: f64,
}

impl Vco {
    /// An oscillator at `sample_rate`, running its core at `oversample` (1, 2, 4 or 8)
    /// times that rate.
    pub fn new(sample_rate: f64, oversample: usize) -> Vco {
        let oversample = match oversample {
            0 | 1 => 1,
            2 | 3 => 2,
            4..=7 => 4,
            _ => 8,
        };
        Vco {
            expo: ExpoCircuit::default(),
            core: CoreParams::default(),
            shaper: ShaperParams::default(),
            pulse: crate::pulse::PulseModel::shared(crate::pulse::PulseCircuit::instrument())
                .expect("prepared pulse comparator"),
            width_network: false,
            celsius: 25.0,
            oversample,
            dt: 1.0 / (sample_rate * oversample as f64),
            other: oversample,
            table: None,
            table_on: false,
            table_last: None,
            dec_other: [
                Decimator::for_oscillators(oversample),
                Decimator::for_oscillators(oversample),
                Decimator::for_oscillators(oversample),
            ],
            v: CoreParams::default().v_top0,
            hold_left: 0.0,
            reset_in: None,
            rise_in: None,
            fall_in: None,
            rect_is_low: false,
            guess: None,
            saw: BlepLine::default(),
            tri: BlepLine::default(),
            rect: BlepLine::default(),
            outputs: [true; 3],
            width_moves: false,
            last_width: f64::NAN,
            fold: fold_corner(&ShaperParams::default()),
            dec: [
                Decimator::for_oscillators(oversample),
                Decimator::for_oscillators(oversample),
                Decimator::for_oscillators(oversample),
            ],
            ticks: 0,
            last_reset: 0.0,
            resets: 0,
            sub: 0,
            solved: [None; 2],
            timed_delay: CoreParams::default().delay[0],
        }
    }

    /// Prepares Potato: a second oversampling factor (1, 2, 4 or 8) to switch to while it
    /// plays ([`Vco::set_oversample`]) and the converter's table for its trims as they are
    /// ([`crate::expo::ConverterTable`]; not on the audio thread: both are made here).
    pub fn prepare(&mut self, oversample: usize) {
        let d = || Decimator::for_oscillators(oversample);
        self.dec_other = [d(), d(), d()];
        self.other = oversample;
        // (One a process for each converter's circuit and temperature.)
        use crate::expo::{ConverterTable, ExpoCircuit};
        static TABLES: crate::shared::Shared<(ExpoCircuit, u64), ConverterTable> =
            crate::shared::Shared::new();
        let (expo, celsius) = (self.expo, self.celsius);
        self.table_last = None;
        self.table = Some(TABLES.get((expo, celsius.to_bits()), || {
            ConverterTable::new(&expo, celsius)
        }));
    }

    /// Runs at this oversampling from the next sample, if it is this one's or the prepared
    /// one's (otherwise nothing changes): the ramp and its events carry on; the decimators
    /// and the band-limited steps start again from silence (Potato).
    pub fn set_oversample(&mut self, oversample: usize) {
        if oversample == self.oversample || oversample != self.other {
            return;
        }
        let rate_dt = self.dt * self.oversample as f64;
        std::mem::swap(&mut self.dec, &mut self.dec_other);
        self.other = self.oversample;
        self.oversample = oversample;
        self.dt = rate_dt / oversample as f64;
        self.dec.iter_mut().for_each(Decimator::reset);
        self.saw.clear();
        self.tri.clear();
        self.rect.clear();
        self.sub = 0;
    }

    /// The latest ramp reset's time in output samples since [`Vco::reset`] (exact, before
    /// the output's latency), and how many resets there have been.
    pub fn last_reset(&self) -> (f64, u64) {
        (self.last_reset, self.resets)
    }

    /// How long before the latest output sample the latest ramp reset was, output samples
    /// (exact, before the output's latency), and how many resets there have been.
    pub fn reset_ago(&self) -> (f64, u64) {
        // (The latest sample's end, as `last_reset` counts time.)
        let end = self.ticks as f64 - 1.0 / self.oversample as f64;
        (end - self.last_reset, self.resets)
    }

    /// The outputs' delay through band-limiting and decimation, in output samples.
    pub fn latency(&self) -> f64 {
        self.dec[0].delay() + BLEP_DELAY as f64 / self.oversample as f64
    }

    /// Prepare a physical comparator component set during setup, never in audio processing.
    pub fn prepare_pulse(
        &mut self,
        circuit: crate::pulse::PulseCircuit,
    ) -> Result<(), crate::mna::NoConvergence> {
        self.pulse = crate::pulse::PulseModel::shared(circuit)?;
        Ok(())
    }
    /// Use the drawn10K width source (fixed taps or its continuous wiper) instead of
    /// an ideal bench voltage. All thresholds/delays are already prepared at construction.
    pub fn use_width_network(&mut self, on: bool) {
        self.width_network = on;
    }
    fn rect_thresholds(&self, width: f64) -> (f64, f64) {
        if self.width_network {
            let t = self.pulse.thresholds(width);
            (t.fall_buf, t.rise_buf)
        } else {
            (
                self.shaper.rect_fall0 + self.shaper.rect_per_width * width,
                self.shaper.rect_rise0 + self.shaper.rect_per_width * width,
            )
        }
    }
    fn rect_falling_delay(&self, slope: f64, width: f64) -> f64 {
        if self.width_network {
            self.pulse
                .falling_delay(width, self.shaper.buf_gain * slope)
        } else {
            self.shaper.rect_fall_delay
        }
    }
    fn rect_rising_delay(&self, width: f64) -> f64 {
        if self.width_network {
            self.pulse.rising_delay(width)
        } else {
            self.shaper.rect_rise_delay
        }
    }
    /// Which outputs (sawtooth, triangle, rectangle) to correct from now on: the voice's
    /// selected waveform's ([`crate::voice`]); all of them unless set. An output left out is
    /// still made, uncorrected (it aliases): taken up again, its corrections start with the
    /// next event (performance; decisions.md R-BLEP).
    pub fn set_outputs(&mut self, outputs: [bool; 3]) {
        self.outputs = outputs;
    }

    /// Starts again `at` (0 at the ramp's top after a reset, towards 1 at the threshold where
    /// it resets) down its ramp, as a free-running oscillator is wherever it has got to when a
    /// note comes, rather than at the reference's initial condition (decisions.md R45, the
    /// CA-74's R25). Its edges' state starts as a reset leaves it: an edge the place has already
    /// passed comes right at the next cycle.
    pub fn start_at(&mut self, at: f64) {
        self.reset();
        let f = at.clamp(0.0, 0.98);
        self.v = self.core.v_top0 + f * (self.core.v_threshold - self.core.v_top0);
    }

    /// Starts again from the top of the ramp.
    pub fn reset(&mut self) {
        self.v = self.core.v_top0;
        self.hold_left = 0.0;
        self.reset_in = None;
        self.rise_in = None;
        self.fall_in = None;
        self.rect_is_low = false;
        self.last_width = f64::NAN;
        self.guess = None;
        self.saw.clear();
        self.tri.clear();
        self.rect.clear();
        self.dec.iter_mut().for_each(Decimator::reset);
        self.ticks = 0;
        self.last_reset = 0.0;
        self.resets = 0;
    }

    /// The ramp's voltage now.
    pub fn ramp(&self) -> f64 {
        self.v
    }

    /// One output sample, for `i_in`, the current the control inputs send into the
    /// summing junction (see [`ExpoCircuit::input_current`]), and the rectangle's width
    /// bias in volts (0, -1.5 or -2.5 from the waveform switch).
    pub fn tick(&mut self, i_in: f64, width: f64) -> VcoOut {
        // The timing current is linear in the ramp voltage (the Early effect, amplified a
        // little by R42's feedback): I(v) = i0 + b v, from the converter at 0 V and -4 V.
        let key = (
            i_in.to_bits(),
            self.guess.map(f64::to_bits),
            self.celsius.to_bits(),
        );
        // (Potato: from the converter's table, where it has the drive; the solves' cache
        // looked in only without it.)
        let from_table = if self.table_on {
            let u = self.expo.drive_u(i_in, self.celsius);
            let key = [
                u.to_bits(),
                self.expo.r11.to_bits(),
                self.expo.a8.to_bits(),
                self.celsius.to_bits(),
            ];
            match self.table_last {
                Some((k, value)) if k == key => Some(value),
                _ => {
                    let value = self
                        .table
                        .as_ref()
                        .and_then(|t| t.lookup(&self.expo, self.celsius, u));
                    self.table_last = value.map(|v| (key, v));
                    value
                }
            }
        } else {
            None
        };
        let hit = || {
            self.solved
                .iter()
                .flatten()
                .find(|s| (s.i_in, s.guess, s.celsius) == key && s.expo == self.expo)
                .map(|s| (s.top, s.bottom))
        };
        let (top, bottom) = match from_table.or_else(hit) {
            Some(x) => x,
            None => {
                let top = self.expo.solve_current(i_in, 0.0, self.celsius, self.guess);
                let bottom = self
                    .expo
                    .solve_current(i_in, -4.0, self.celsius, Some(top.ic_exp));
                self.solved = [
                    Some(Solved {
                        i_in: key.0,
                        guess: key.1,
                        celsius: key.2,
                        expo: self.expo,
                        top: top.ic_exp,
                        bottom: bottom.ic_exp,
                    }),
                    self.solved[0],
                ];
                (top.ic_exp, bottom.ic_exp)
            }
        };
        // Far above the instrument's range (keys beyond its 44, the plug-in's extension, with
        // RANGE, FREQUENCY and the PITCH wheel high) the converter's model leaves its ground:
        // above about 1e-2 A its tail node runs past any rail and the solve diverges. The
        // timing current is held at [`TIMING_MAX`] there, the ramp's slope as the converter
        // gives it at that current; below it, as solved.
        let (top, bottom) = if top.is_finite() && bottom.is_finite() && top <= TIMING_MAX {
            (top, bottom)
        } else {
            (TIMING_MAX, TIMING_MAX * EARLY_AT_MAX)
        };
        self.guess = Some(top);
        let b = (top - bottom) / 4.0;
        self.tick_current(top, b, width)
    }

    /// The last sample's timing current with the ramp at 0 V, A: the converter's output,
    /// which the oscillator's frequency follows (a probe of the pitch path).
    pub fn timing_current(&self) -> f64 {
        self.guess.unwrap_or(0.0)
    }

    /// Whether the width may move from sample to sample (the plug-in's CONTINUOUS waveform,
    /// decisions.md R-CONTINUOUS-WAVE): then a new width the ramp has already passed switches
    /// the comparator at once, as the circuit's does ([`Vco::follow_width`]). Off (the
    /// instrument's switch, which gives three fixed widths), nothing changes.
    pub fn set_width_moves(&mut self, on: bool) {
        self.width_moves = on;
    }

    /// One output sample for a timing current given directly: `i0` at a ramp of 0 V and its
    /// slope `b` per volt (the Early effect). [`Vco::tick`] computes them from the inputs.
    pub fn tick_current(&mut self, i0: f64, b: f64, width: f64) -> VcoOut {
        self.tick_net(i0 - self.core.i_leak, b, width)
    }

    /// One output sample of the oscillator held to a clock (decisions.md R-OSC-TIMED): its
    /// ramp's slope chosen each sample so that it reaches the threshold, and resets after the
    /// comparator's delay, as the clock's next cycle starts (`t.phase` reaching 1); the slope
    /// bent by the converter's Early effect as the circuit's is ([`TIMED_EARLY`]). The
    /// converter, the pitch inputs and ENTROPY's drift have no part in it. An oscillator
    /// further from its clock than [`TIMED_TOLERANCE`] is put where the clock is at once, its
    /// outputs stepped, band-limited, as a reset steps them.
    pub fn tick_timed(&mut self, t: Timed, width: f64) -> VcoOut {
        let sample = self.dt * self.oversample as f64;
        let per = if t.per.is_finite() {
            t.per.clamp(1e-12, 0.25)
        } else {
            1e-12
        };
        let period = sample / per;
        // (Where the clock was as the sample's time began.)
        let phase = if t.phase.is_finite() {
            (t.phase - per).rem_euclid(1.0)
        } else {
            0.0
        };
        // Where its own cycle stands (at its start while the ramp is held at its top or about
        // to reset), and how far ahead of the clock that is.
        let at_top = self.reset_in.is_some() || self.hold_left > 0.0;
        let mut own = if at_top { 0.0 } else { self.ramp_phase(self.v) };
        let mut ahead = own - phase;
        ahead -= ahead.round();
        if ahead.abs() > TIMED_TOLERANCE {
            self.jump_to(phase, width);
            own = self.ramp_phase(self.v);
            ahead = own - phase;
            ahead -= ahead.round();
        }
        // The time left in its cycle, and before its ramp falls again: then the slope that
        // takes it to the threshold the comparator's delay before the cycle's end.
        let end = (1.0 - own + ahead) * period;
        let (from, wait) = match self.reset_in {
            Some(r) => (self.core.v_top0, r + self.core.t_hold),
            None => (self.v, self.hold_left),
        };
        // (Late, the ramp is sent down at once: the current the converter gives at most.)
        let run = (end - wait - self.timed_delay).max(1e-6 * sample);
        let c = self.core.c1 + self.core.c_par;
        let fall = self.ramp_log(self.core.v_threshold) - self.ramp_log(from);
        let a = (c * fall / (TIMED_EARLY * run)).clamp(1e-15, TIMING_MAX);
        let b = TIMED_EARLY * a;
        self.guess = Some(a + self.core.i_leak);
        self.timed_delay = self.core.delay_at((a + b * self.core.v_threshold) / c);
        self.tick_net(a, b, width)
    }

    /// How far down its ramp the voltage `v` is in a timed ramp's time from the top: the log
    /// of its distance from the ramp's asymptote under [`TIMED_EARLY`] (the time is this over
    /// the slope's share of the capacitance).
    fn ramp_log(&self, v: f64) -> f64 {
        let k = 1.0 / TIMED_EARLY;
        crate::ulp::log((self.core.v_top0 + k) / (v + k))
    }

    /// The share of a timed ramp's time from its top to the threshold that has passed at `v`.
    fn ramp_phase(&self, v: f64) -> f64 {
        (self.ramp_log(v) / self.ramp_log(self.core.v_threshold)).clamp(0.0, 1.0)
    }

    /// Puts the ramp `phase` of the way along a timed ramp's time, at once (a timed
    /// oscillator's clock jumped): its outputs stepped there, band-limited, the rectangle low
    /// if the ramp is past its falling edge's threshold; no reset, hold or edge pending.
    fn jump_to(&mut self, phase: f64, width: f64) {
        let k = 1.0 / TIMED_EARLY;
        // (At a cycle's very end, its next start: the ramp at its top.)
        let f = if phase < 0.999 { phase.max(0.0) } else { 0.0 };
        let v =
            (self.core.v_top0 + k) * crate::ulp::exp(-f * self.ramp_log(self.core.v_threshold)) - k;
        let sh = self.shaper;
        let (buf0, buf1) = (
            sh.buf_gain * self.v + sh.buf_offset,
            sh.buf_gain * v + sh.buf_offset,
        );
        if self.outputs[0] {
            self.saw.step(sh.saw_ratio * (buf1 - buf0), 1.0);
        }
        if self.outputs[1] {
            self.tri.step(triangle(buf1) - triangle(buf0), 1.0);
        }
        let (fall_buf, _) = self.rect_thresholds(width);
        let low = v <= (fall_buf - sh.buf_offset) / sh.buf_gain;
        if low != self.rect_is_low && self.outputs[2] {
            let (from, to) = if low {
                (sh.rect_high, sh.rect_low)
            } else {
                (sh.rect_low, sh.rect_high)
            };
            self.rect.step(to - from, 1.0);
        }
        self.rect_is_low = low;
        self.v = v;
        self.hold_left = 0.0;
        self.reset_in = None;
        self.rise_in = None;
        self.fall_in = None;
    }

    /// One output sample for the ramp's net current `a` at 0 V (the timing current less the
    /// leakage) and its slope `b` a volt.
    fn tick_net(&mut self, a: f64, b: f64, width: f64) -> VcoOut {
        let c = self.core.c1 + self.core.c_par;
        let sh = self.shaper;
        let (fall_buf, rise_buf) = self.rect_thresholds(width);
        let overdrive = if self.width_network {
            0.0
        } else {
            sh.rect_fall_overdrive
        };
        let v_fall = (fall_buf - sh.buf_offset) / sh.buf_gain - overdrive;
        if self.width_moves && width != self.last_width && !self.last_width.is_nan() {
            self.follow_width(v_fall, rise_buf, width, (a + b * self.v) / c);
        }
        self.last_width = width;
        let mut out = VcoOut::default();
        for k in 0..self.oversample {
            self.sub = k;
            self.step(a, b, c, v_fall, rise_buf);
            let buf = sh.buf_gain * self.v + sh.buf_offset;
            let level = if self.rect_is_low {
                sh.rect_low
            } else {
                sh.rect_high
            };
            let s = self.saw.push(sh.saw_ratio * buf);
            let t = self.tri.push(triangle(buf));
            let r = self.rect.push(level);
            if let Some(v) = self.dec[0].push(s) {
                out.saw = v;
            }
            if let Some(v) = self.dec[1].push(t) {
                out.tri = v;
            }
            if let Some(v) = self.dec[2].push(r) {
                out.rect = v;
            }
        }
        self.ticks += 1;
        out
    }

    /// One oversampled step: the ramp (C dv/dt = -(a + b v)) and its events, taken in
    /// time order: the threshold, the reset after the comparator's delay, the end of the
    /// hold, the ramp passing the rectangle's threshold, and the rectangle's delayed edges.
    fn step(&mut self, a: f64, b: f64, c: f64, v_fall: f64, rise_buf: f64) {
        let dt = self.dt;
        let mut t = 0.0;
        // v(t) = -A + (v0 + A) e^(-b t / C), A = a / b.
        let big_a = a / b;
        let advance = |v: f64, h: f64| -big_a + (v + big_a) * crate::ulp::exp(-b * h / c);
        let cross = |v0: f64, v1: f64| (c / b) * crate::ulp::log((v0 + big_a) / (v1 + big_a));
        let th = self.core.v_threshold;
        loop {
            let left = dt - t;
            let holding = self.hold_left > 0.0;
            let v_end = if holding {
                self.v
            } else {
                advance(self.v, left)
            };
            // The earliest event in what is left of the step.
            let mut next = left;
            let mut which = 0u8;
            let mut take = |at: f64, w: u8| {
                if at < next {
                    next = at;
                    which = w;
                }
            };
            if holding {
                take(self.hold_left, 1);
            }
            if let Some(r) = self.reset_in {
                take(r, 2);
            }
            if let Some(r) = self.rise_in {
                take(r, 3);
            }
            if let Some(f) = self.fall_in {
                take(f, 4);
            }
            if !holding && self.reset_in.is_none() && self.v > th && v_end <= th {
                take(cross(self.v, th).clamp(0.0, left), 5);
            }
            if !holding
                && !self.rect_is_low
                && self.fall_in.is_none()
                && self.v > v_fall
                && v_end <= v_fall
            {
                take(cross(self.v, v_fall).clamp(0.0, left), 6);
            }
            let v_fold = self.fold.0;
            if !holding && self.v > v_fold && v_end <= v_fold {
                take(cross(self.v, v_fold).clamp(0.0, left), 7);
            }
            // Move everything to it.
            if !holding {
                self.v = advance(self.v, next);
            }
            self.hold_left = (self.hold_left - next).max(0.0);
            for x in [&mut self.reset_in, &mut self.rise_in, &mut self.fall_in]
                .into_iter()
                .flatten()
            {
                *x -= next;
            }
            t += next;
            let frac = (t / dt).clamp(1e-9, 1.0);
            let sh = self.shaper;
            match which {
                1 => {
                    // The hold's end: the ramp falls again, at -s1 a sample in the buffer.
                    let s1 = (a + b * self.v) / c * dt * sh.buf_gain;
                    let buf = sh.buf_gain * self.v + sh.buf_offset;
                    if self.outputs[0] {
                        self.saw.ramp(-sh.saw_ratio * s1, frac);
                    }
                    if self.outputs[1] {
                        self.tri.ramp(-triangle_slope(buf) * s1, frac);
                    }
                }
                7 => {
                    // The triangle's fold: its slope against the buffer turns there, over
                    // its width.
                    self.v = v_fold;
                    if self.outputs[1] {
                        let s = (a + b * self.v) / c * dt * sh.buf_gain;
                        self.tri.fillet(self.fold.1 * s, frac, self.fold.2 / s);
                    }
                }
                2 => {
                    self.reset_in = None;
                    self.do_reset(a, b, c, frac, rise_buf);
                }
                3 => {
                    self.rise_in = None;
                    if self.rect_is_low {
                        self.rect_is_low = false;
                        if self.outputs[2] {
                            self.rect.step(sh.rect_high - sh.rect_low, frac);
                        }
                    }
                }
                4 => {
                    self.fall_in = None;
                    if !self.rect_is_low {
                        self.rect_is_low = true;
                        if self.outputs[2] {
                            self.rect.step(sh.rect_low - sh.rect_high, frac);
                        }
                    }
                }
                5 => {
                    self.v = th;
                    let slope = (a + b * th) / c;
                    self.reset_in = Some(self.core.delay_at(slope));
                }
                6 => {
                    self.v = v_fall;
                    self.fall_in =
                        Some(self.rect_falling_delay((a + b * self.v) / c, self.last_width));
                }
                _ => {}
            }
            if which == 0 {
                break;
            }
        }
    }

    /// The comparator against a width that has just moved: the falling ramp already past the
    /// new falling threshold while the rectangle is high, it falls (after its delay, as from
    /// a crossing); the buffer above the new rising threshold while it is low, it rises. The
    /// thresholds otherwise only meet the ramp as it crosses them ([`Vco::step`]) and the
    /// reset ([`Vco::do_reset`]), which a fixed width needs alone.
    fn follow_width(&mut self, v_fall: f64, rise_buf: f64, width: f64, slope: f64) {
        if self.hold_left > 0.0 {
            return;
        }
        let sh = self.shaper;
        let buf = sh.buf_gain * self.v + sh.buf_offset;
        if !self.rect_is_low
            && self.fall_in.is_none()
            && self.reset_in.is_none()
            && self.v <= v_fall
        {
            self.fall_in = Some(self.rect_falling_delay(slope, width));
        } else if self.rect_is_low && self.rise_in.is_none() && buf >= rise_buf {
            self.rise_in = Some(self.rect_rising_delay(width));
        }
    }

    fn do_reset(&mut self, a: f64, b: f64, c: f64, frac: f64, rise_buf: f64) {
        let sh = self.shaper;
        let slope = (a + b * self.v) / c;
        let v_top = self.core.v_top0 - self.core.top_per_slope * slope;
        let buf0 = sh.buf_gain * self.v + sh.buf_offset;
        let buf1 = sh.buf_gain * v_top + sh.buf_offset;
        // The ramp stops falling and holds: the buffer's slope (a sample) goes from -s0 to 0.
        let s0 = slope * self.dt * sh.buf_gain;
        if self.outputs[0] {
            self.saw.ramp(sh.saw_ratio * s0, frac);
            self.saw.step(sh.saw_ratio * (buf1 - buf0), frac);
            self.saw.impulse(sh.saw_ratio * sh.buf_lag / self.dt, frac);
        }
        if self.outputs[1] {
            self.tri.ramp(triangle_slope(buf0) * s0, frac);
            self.tri.step(triangle(buf1) - triangle(buf0), frac);
            self.tri.impulse(sh.tri_glitch / self.dt, frac);
        }
        if self.rect_is_low && buf1 >= rise_buf {
            self.rise_in = Some(self.rect_rising_delay(self.last_width));
        }
        // A fall still pending when the ramp resets is overtaken by the reset.
        self.fall_in = None;
        self.v = v_top;
        self.hold_left = self.core.t_hold;
        self.last_reset =
            self.ticks as f64 + (self.sub as f64 + frac - 1.0) / self.oversample as f64;
        self.resets += 1;
    }
}
