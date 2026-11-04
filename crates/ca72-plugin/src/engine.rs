//! The plug-in's audio engine, apart from any plug-in format: the circuit model's voice in its
//! real-time (Potato) quality, played by MIDI notes, pitch bend and the modulation wheel, with
//! the side chain at EXTERNAL INPUT. Once the voices are built nothing here allocates
//! (`tests/realtime.rs`).
//!
//! POLY off, the one instrument takes every key through its keyboard circuit (NOTE PRIORITY's
//! choice of the keys held, LOW by default, the circuit's own; single triggering), as before. UNISON (outranking POLY) plays VOICES whole
//! instruments together, each taking every key through its own keyboard circuit as the one
//! does, each its own parts under ENTROPY and its own place under SPREAD, turned down by the
//! square root of VOICES (decisions.md R45, the CA-74's R25). POLY on, one voice per note, up to VOICES (2 to
//! 10) of the ten built: each the whole instrument with its own keyboard circuit, which its
//! note reaches as one key (`PolyKey`; decisions.md, "POLY"). A POLY note takes the voice
//! that last played its key if that voice is free or letting go, else the free voice whose
//! last note began longest ago, else the one let go longest ago, else the one held longest
//! (the CA-74's R27; decisions.md R45), so a chord struck again stays where it was and a
//! melody goes round the voices. ENTROPY makes each voice its own parts, SPREAD puts the voices
//! across the stereo field where the placement has them (`character.rs`); the one instrument
//! (without DOUBLE) at the next of VOICES's places at each attack, a note played legato staying
//! where its phrase began (decisions.md R62); ENTROPY at 5 % and SPREAD at 0 by default (R-ENTROPY-FLOOR).

use crate::timed::{Length, Position};
use ca72::light::Light;
use ca72::modulation::PITCH_WHEEL_SEMITONES;
use ca72::resample::{Decimator, Interpolator};
use ca72::vco::Timed;
use ca72::voice::{
    Jacks, LINE_UP, Panel, Priority, Quality, RETRIGGER_GAP, Swept, Voice, audio_taper,
    cutoff_track,
};

use crate::character::{Character, Placement};
use crate::drive::{AVERAGE, Calibration, Curve, HEARD, HELD, KEYS, KWeighted, STEPS};
use crate::pool::{Ask, Crew, EVERY, MAX, Need, Pool, Shared, Split};
use ca72::threaded::Helpers;
use plugin_kit_stereo::place::{
    ALONE, DOUBLE_TRIM, glide_share, pair_gains, unison_trim, voice_gains,
};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How far ENTROPY goes at 100 %, in the character's figures (`character.rs`, the DAW's):
/// twice them, as in the DAW's instrument (the owner, 2026-10-02, found them too subtle at
/// once).
pub const ENTROPY_DEPTH: f64 = 2.0;

/// The oscillators' own mismatch and drift with LOCK off, in the character's figures
/// (decisions.md R9; the DAW's): a tuning spread of 3 cents and 2.25 of drift, so oscillators
/// at one pitch never lock in phase (locked, three in unison sounded 3.3 dB louder, not
/// better). ENTROPY adds to it above 5 %; below that it falls to none (R-ENTROPY-FLOOR).
pub const ENTROPY_FLOOR: f64 = 0.75;

/// The previous minimum's place on ENTROPY's new travel (R-ENTROPY-FLOOR).
pub const ENTROPY_KNEE: f64 = 0.05;

/// The old ENTROPY above its minimum, in its original units. Below the knee only the
/// oscillator floor changes; panel tolerances and cutoff movement remain absent.
pub fn entropy_depth(amount: f64) -> f64 {
    ((amount - ENTROPY_KNEE) / (1.0 - ENTROPY_KNEE)).clamp(0.0, 1.0) * ENTROPY_DEPTH
}

/// How much of the ideal gain of locking (`10 log10(coherent / incoherent)`) shows at the
/// output, the mixer's overdrive and the filter taking the rest (fitted in the DAW).
pub const LOCK_SHARE: f64 = 0.68;

/// The output's gain with LOCK on: down by the panel's switched-on oscillators that share a
/// pitch (RANGE, FREQUENCY and, for oscillator 3, OSC. 3 CONTROL alike) and their volumes, so
/// locked oscillators sound as loud as drifting ones. With no two at one pitch, 1.
pub fn lock_trim(p: &Panel) -> f64 {
    let keys = |n: usize| (p.osc[n].range, p.osc[n].freq, n < 2 || p.osc3_control);
    let on = |n: usize| p.osc[n].on && p.osc[n].volume > 0.0;
    let incoherent: f64 = (0..3)
        .filter(|&n| on(n))
        .map(|n| p.osc[n].volume.powi(2))
        .sum();
    let mut coherent = 0.0;
    for n in (0..3).filter(|&n| on(n)) {
        if (0..n).any(|m| on(m) && keys(m) == keys(n)) {
            continue;
        }
        let a: f64 = (0..3)
            .filter(|&m| on(m) && keys(m) == keys(n))
            .map(|m| p.osc[m].volume)
            .sum();
        coherent += a * a;
    }
    if incoherent > 0.0 && coherent > incoherent {
        ca72::ulp::pow(coherent / incoherent, -0.5 * LOCK_SHARE)
    } else {
        1.0
    }
}

/// The voices POLY may play: the fewest, by default, the most (all of them built).
pub const POLY_VOICES: (usize, usize, usize) = (2, 4, 10);

/// The voices whose levels the editor is told: as many as POLY can have.
pub const LEVELS: usize = POLY_VOICES.2;

/// DRIVE's top, dB (decisions.md R45, the CA-74's R29: past it the filter is saturated
/// and the note stops being one).
pub const DRIVE_TOP: f64 = 24.0;

/// How long a curve measured after DRIVE was turned takes to come in, s (its glide's time
/// constant; decisions.md R-AUTO-GAIN-DRIVE): slow enough that a refinement arriving while a
/// note plays is not heard as a step, quick beside the measurement itself.
pub const REFINE_GLIDE: f64 = 0.3;

/// LEVEL's range, dB (the CA-74's).
pub const LEVEL_RANGE: (f64, f64) = (-30.0, 12.0);

/// ENTROPY's tolerances on the panel: EMPHASIS, the filter contour's ATTACK and DECAY, the
/// loudness contour's, GLIDE.
const ENTROPY_KNOBS: usize = 6;

/// What the parameters set: the panel (its wheels where the parameters leave them), MAIN
/// OUTPUT's VOLUME (0..1 of its travel) and switch, how far a MIDI keyboard's full pitch
/// bend moves the PITCH wheel, in semitones, POWER (off: bypassed, silent), and POLY,
/// VOICES, ENTROPY, SPREAD and INNER (0..1), where SPREAD places the voices, UNISON and DOUBLE.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Controls {
    pub panel: Panel,
    pub volume: f64,
    pub main_output: bool,
    pub bend_range: f64,
    pub power: bool,
    pub poly: bool,
    pub voices: usize,
    pub entropy: f64,
    pub spread: f64,
    /// INNER: the inner edge of each side's band, a share of SPREAD's way out (decisions.md
    /// R49).
    pub inner: f64,
    /// SCATTER's placement: where SPREAD puts POLY's voices (decisions.md R45), EVEN or
    /// CENTER as the plug-in offers them (`params::Scatter`; R49).
    pub placement: Placement,
    /// UNISON: VOICES instruments on every key together (decisions.md R45).
    pub unison: bool,
    /// DOUBLE (0..1): each note played by two voices, detuned from each other by up to
    /// [`DOUBLE_CENTS`], SPREAD putting them to either side of the centre (decisions.md
    /// R45); 0, one voice a note.
    pub double: f64,
    /// DRIVE (0..[`DRIVE_TOP`] dB): the mixer's signal into the filter raised by so much, its
    /// input pair driven past where the panel's mixer can take it; 0, the circuit
    /// (decisions.md R45).
    pub drive: f64,
    /// LEVEL (dB, [`LEVEL_RANGE`]): the output's gain after MAIN OUTPUT's, the plug-in's own;
    /// 0, none (decisions.md R45).
    pub level: f64,
    /// AUTO GAIN: the output brought back down by as much as DRIVE made the sound louder, as
    /// measured for it (`drive.rs`; decisions.md R45).
    pub auto_gain: bool,
    /// FEEDBACK (0..1): the voices' output patched back into their EXTERNAL INPUT, a sample
    /// late, by this much of its level (the phones' VOLUME knob; decisions.md R8).
    pub feedback: f64,
    /// LOCK: the oscillators identical, the circuit as drawn, its level matched
    /// ([`lock_trim`]); off, each oscillator keeps its own floor of mismatch
    /// ([`ENTROPY_FLOOR`]; decisions.md R9).
    pub lock: bool,
    /// QUALITY at LO, Potato mode: the voices played by the light model ([`Light`]), not the
    /// circuit's, for computers the circuit is too heavy for (decisions.md R47).
    pub potato: bool,
    /// QUALITY at ULTRA: the circuit's model with High Fidelity numerics (`Quality::HighFidelity`),
    /// not at its real-time setting (decisions.md R48). Switched while it plays.
    pub ultra: bool,
    /// How far each voice's own pressure and timbre move it (decisions.md R-MPE): all 0, not
    /// at all, the circuit as before to the bit.
    pub reach: Reach,
    /// Each oscillator timed (decisions.md R-OSC-TIMED): its cycle's length on the host's
    /// clock ([`Engine::set_position`]) while its RANGE is at LO; none, or at any other RANGE,
    /// its own.
    pub timed: [Option<Length>; 3],
    /// The LFOs (decisions.md R-LFO): their settings, assignments and the assigned knobs'
    /// travel; none assigned, nothing more runs.
    pub lfo: crate::lfo::Lfos,
}

/// How far a voice's note's pressure (0..1) and timbre (-1..1, 0 at rest) move it (decisions.md
/// R-MPE): its MODULATION wheel's MIDI share, as a keyboard's wheel moves it (-1..1, 1 adding a
/// wheel pushed fully forward at full pressure), and its filter's cutoff, octaves at full.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Reach {
    pub pressure_mod: f64,
    pub pressure_filter: f64,
    pub timbre_mod: f64,
    pub timbre_filter: f64,
}

impl Reach {
    /// Whether it moves the voices' MODULATION wheels.
    fn moves_wheel(&self) -> bool {
        self.pressure_mod != 0.0 || self.timbre_mod != 0.0
    }
}

/// A note's own expression, as MPE or a host's note expressions give it (decisions.md R-MPE):
/// its pressure, 0..1; its timbre, -1..1 and 0 at rest, as the computer's TIMBRE REST measures
/// it (`expression::TimbreRest`, R-MPE: 0..1 from 0 at ZERO); and its own pitch bend,
/// semitones (on top of the PITCH wheel's, which every voice shares).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Expression {
    pub pressure: f32,
    pub timbre: f32,
    pub bend: f32,
}

impl Expression {
    pub const NONE: Expression = Expression {
        pressure: 0.0,
        timbre: 0.0,
        bend: 0.0,
    };
}

/// A note's velocity as a voice has it, the velocity source's (decisions.md R-VELOCITY): its
/// note's, 0..1 (`to`), and where it has got gliding there (`now`): at once for a new note,
/// over [`EXPRESSION_GLIDE`] as the key the instruments sound changes under a phrase.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Velocity {
    to: f32,
    now: f64,
}

impl Velocity {
    /// A key's velocity before it is told one: full (a note from a source without one).
    const FULL: f32 = 1.0;

    fn new() -> Velocity {
        Velocity {
            to: Velocity::FULL,
            now: f64::from(Velocity::FULL),
        }
    }

    /// Velocity `v` from now, at once or gliding.
    fn take(&mut self, v: f32, at_once: bool) {
        self.to = v;
        if at_once {
            self.now = f64::from(v);
        }
    }

    /// A share `share` of the way on to its note's.
    fn glide(&mut self, share: f64) {
        self.now += (f64::from(self.to) - self.now) * share;
    }

    /// Where [`Velocity::glide`] by `share` will have it, without moving it (the LFOs' next
    /// step, decisions.md R-LFO-STRESS).
    fn ahead(&self, share: f64) -> f64 {
        self.now + (f64::from(self.to) - self.now) * share
    }
}

impl Default for Controls {
    fn default() -> Self {
        Controls {
            panel: Panel::default(),
            volume: 1.0,
            main_output: true,
            bend_range: 2.0,
            power: true,
            poly: false,
            voices: POLY_VOICES.1,
            entropy: ENTROPY_KNEE,
            spread: 0.0,
            inner: 0.0,
            placement: Placement::Even,
            unison: false,
            double: 0.0,
            drive: 0.0,
            level: 0.0,
            auto_gain: true,
            feedback: 0.0,
            lock: false,
            potato: false,
            ultra: false,
            reach: Reach::default(),
            timed: [None; 3],
            lfo: crate::lfo::Lfos::default(),
        }
    }
}

/// MIDI input, as the engine takes it (any channel).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    Note {
        key: u8,
        on: bool,
    },
    /// The pitch bend, -1..1, 0 at rest.
    PitchBend(f32),
    /// The modulation wheel (control change 1), 0..1.
    Modulation(f32),
    /// Reset All Controllers (control change 121): the bend and modulation wheel at rest.
    ResetControllers,
    /// All Notes Off (control change 123): every key released (the contours then run out as
    /// the DECAY switch has them).
    AllNotesOff,
    /// All Sound Off (control change 120): every key released and the sound gone at once, the
    /// output faded out over [`HUSH_FADE`] and every voice put to rest (decisions.md R18).
    AllSoundOff,
    /// A key's velocity (0..1) for its next note (decisions.md R-VELOCITY): sent before the
    /// note, the note starts with it; what the velocity source gives the voice playing it.
    NoteVelocity {
        key: u8,
        velocity: f32,
    },
    /// A key's own expression from now (decisions.md R-MPE): the voice playing it while it is
    /// held, and the next note on it. Sent before the key's note, its note starts with it.
    NoteExpression {
        key: u8,
        expression: Expression,
    },
    /// The pressure (0..1) and timbre (-1..1, 0 at rest) every voice shares, ordinary MIDI's
    /// channel pressure and control change 74 (decisions.md R-MPE): a voice takes
    /// the larger of its own pressure and this, and of its own timbre and this the one farther
    /// from rest ([`farther`]).
    ChannelPressure(f32),
    Timbre(f32),
}

/// The control change of a keyboard's modulation wheel.
pub const MODULATION_WHEEL: u8 = 1;
pub const ALL_SOUND_OFF: u8 = 120;
pub const RESET_CONTROLLERS: u8 = 121;
pub const ALL_NOTES_OFF: u8 = 123;

/// How fast the reported OVERLOAD lamp falls from a peak, s: the lamp's own thermal lag is
/// not modelled, and a display polling now and then should still see a flash.
const LAMP_FALL: f32 = 0.05;

/// The lowest rate the voice runs at, Hz: below it, at 2, 4 or 8 times the host's. The top
/// key's fundamental on 2' (4.2 kHz) stays under its Nyquist frequency, and the model's
/// factory calibration fails at much lower rates (it gives NaN at 1.2 kHz).
pub const MIN_VOICE_RATE: f64 = 8_000.0;

/// The most samples [`OutputDelay`] holds back.
const LINE_UP_MOST: usize = 64;

/// The output held back so that every QUALITY mode's sound comes out together, ULTRA's
/// the latest ([`LINE_UP`]): a ring of each channel's last samples, made with the engine
/// (nothing allocated on the audio thread).
#[derive(Debug, Clone)]
struct OutputDelay {
    ring: [[f32; LINE_UP_MOST]; 2],
    at: usize,
}

impl OutputDelay {
    fn new() -> OutputDelay {
        OutputDelay {
            ring: [[0.0; LINE_UP_MOST]; 2],
            at: 0,
        }
    }

    /// `left` and `right` held back `late` samples (less than [`LINE_UP_MOST`]).
    fn hold(&mut self, late: usize, left: &mut [f32], right: &mut [f32]) {
        let late = late.min(LINE_UP_MOST - 1);
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            self.ring[0][self.at] = *l;
            self.ring[1][self.at] = *r;
            let k = (self.at + LINE_UP_MOST - late) % LINE_UP_MOST;
            (*l, *r) = (self.ring[0][k], self.ring[1][k]);
            self.at = (self.at + 1) % LINE_UP_MOST;
        }
    }

    fn clear(&mut self) {
        *self = OutputDelay::new();
    }
}

/// How long POWER takes to fade the output out or in, s (switched at once, it clicks).
const POWER_FADE: f64 = 0.01;

/// How long All Sound Off takes to fade the output out before the voices are put to rest, s
/// (at once, it clicks; decisions.md R18). It fades back in as POWER does.
pub const HUSH_FADE: f64 = 0.005;

/// The longest All Sound Off keeps the output silent waiting for clean voices to put in the
/// sounding ones' places ([`Spares`]), s; they are normally ready long before.
const HUSH_WAIT: f64 = 0.1;

/// A POLY voice falls silent, and is free for another note, once its key is up and its
/// output has stayed under this for a whole block (blocks shorter than [`SILENT_SPAN`] reckoned
/// together).
const SILENT: f64 = 1e-6;

/// The fewest samples over which a POLY voice is found silent: a host's blocks shorter than
/// this (a call of one frame, a block split at its automation) are reckoned together until
/// they span it. A voice's output under [`SILENT`] for one sample is a zero crossing, not
/// silence: reckoned a block of one frame at a time, a voice at -15 dBFS was cut off in its
/// release (decisions.md R-SHORT-BLOCKS). Blocks this long or longer are reckoned one at a
/// time, as before.
const SILENT_SPAN: usize = 32;

/// A twin let go (DOUBLE turned off) falls silent once its output has stayed under [`SILENT`]
/// for this many samples running, counted across runs (the CA-74's).
const QUIET: usize = 256;

/// Without POLY or UNISON the one instrument sleeps (is not played, the output silent) once no
/// key is down, its output has stayed under [`SILENT`] and its slow state (`slow_state`: the
/// keyboard's hold capacitor and the contours, its twin's too with DOUBLE) has moved less than
/// [`SETTLED`] over this, s; the next note then starts where it would have (decisions.md
/// R68, the development line's R30).
const SLEEP_WINDOW: f64 = 0.05;

/// The stack [`Engine::prepare`] builds the voices on, and [`Engine::start_workers`] starts
/// the workers and ULTRA's crews on, bytes: a main thread's on macOS (decisions.md
/// R-PREPARE-STACK; it needed more than 512 KB in October 2026).
const PREPARE_STACK: usize = 8 << 20;

/// How little the slow state may move over [`SLEEP_WINDOW`] for the instrument to sleep, V: a
/// contour falling with a time constant of 10 s is then within 0.2 mV of where it settles.
const SETTLED: f64 = 1e-6;

/// Whether the one instrument sleeps, and how long it has been silent with its slow state
/// still (`quiet`, of `window` samples at the host's rate) since `anchor`; the output's peak
/// over the block so far.
#[derive(Debug, Clone, Copy, Default)]
struct Rest {
    asleep: bool,
    quiet: usize,
    window: usize,
    anchor: [f64; 6],
    peak: f64,
    /// Sleep not allowed (measurements that play on as before: [`Engine::never_sleep`]).
    never: bool,
}

impl Rest {
    /// The output's samples, as they are made.
    fn heard(&mut self, l: &[f32], r: &[f32]) {
        for y in l.iter().chain(r) {
            self.peak = self.peak.max(f64::from(y.abs()));
        }
    }

    /// After a block of `n` samples: the instrument's slow state then (None while a key is
    /// down or its voice is out of reach), the block's peak reckoned.
    fn after(&mut self, state: Option<[f64; 6]>, n: usize) {
        let silent = self.peak < SILENT;
        self.peak = 0.0;
        match state {
            Some(st) if silent && !self.never && !self.asleep => {
                if self.quiet == 0 {
                    self.anchor = st;
                }
                self.quiet += n;
                if self.quiet >= self.window {
                    self.asleep = st
                        .iter()
                        .zip(&self.anchor)
                        .all(|(a, b)| (a - b).abs() < SETTLED);
                    self.quiet = 0;
                }
            }
            _ => self.quiet = 0,
        }
    }

    fn wake(&mut self) {
        (self.asleep, self.quiet, self.peak) = (false, 0, 0.0);
    }
}

/// The one instrument's slow state (its voice's, and its twin's while DOUBLE plays it), the
/// model QUALITY plays.
fn slow_state(p: &Playing) -> Option<[f64; 6]> {
    let v = if p.potato {
        p.light.slow_state()?
    } else {
        p.voice.slow_state()?
    };
    let t = if p.double || p.twin.live {
        if p.potato {
            p.twin.light.slow_state()?
        } else {
            p.twin.voice.slow_state()?
        }
    } else {
        [0.0; 3]
    };
    Some([v[0], v[1], v[2], t[0], t[1], t[2]])
}

/// The time constant, seconds, with which a voice's place follows SPREAD, INNER, the
/// placement, VOICES and DOUBLE: plugin-kit's (its K6; decisions.md R45, the CA-74's R27
/// and R28).
pub use plugin_kit_stereo::place::GLIDE;

/// The time constant, seconds, with which the one instrument's place follows when it scatters
/// (POLY, UNISON and DOUBLE off: decisions.md R62): a fifth of the time its contours
/// take to start after its trigger contact closes (the light model's `TRIGGER_ON`, fitted to
/// the circuit's), so that at an attack it is within 1 % of its new place before its contours
/// start (with [`GLIDE`] it would still be 46 % of the way short).
pub const TURN_GLIDE: f64 = ca72::light::laws::TRIGGER_ON / 5.0;

/// The detune between DOUBLE's two voices of a note at its full amount, cents: the CA-72's own
/// (the owner, 2026-10-10: "let's make our maximum detune 100"; the kit's, the CA-74's, is 20).
pub const DOUBLE_CENTS: f64 = 100.0;

/// The voices at a multiple of the host's rate: the side chain interpolated up, the left and
/// right outputs decimated down.
#[derive(Debug)]
struct Oversampled {
    factor: usize,
    up: Interpolator,
    down: [Decimator; 2],
}

/// One POLY voice's key (decisions.md, "POLY"): its keyboard circuit gets its note as a key
/// down while the voice's gate is high. A new note on the voice while its gate was high
/// (the gate dropped for a sample between them) lifts the key and presses the new one once
/// the trigger contact has been open [`RETRIGGER_GAP`]: the circuit re-arms its trigger only
/// then, so the contours retrigger; the new note's pitch and attack come that much later.
#[derive(Debug, Clone, Copy)]
struct PolyKey {
    down: Option<i32>,
    /// Samples the trigger contact has been open, and how many it must be.
    open: u32,
    gap: u32,
}

impl PolyKey {
    fn new(rate: f64) -> PolyKey {
        let gap = (RETRIGGER_GAP * rate).ceil() as u32;
        PolyKey {
            down: None,
            open: gap,
            gap,
        }
    }

    /// The voice's gate and key for the next sample, played on its keys by `note` (the
    /// circuit's voice's, or the light one's).
    fn step(&mut self, gate: bool, key: i32, mut note: impl FnMut(i32, bool)) {
        if gate {
            match self.down {
                None if self.open >= self.gap => {
                    note(key, true);
                    self.down = Some(key);
                }
                None => {}
                Some(old) if old != key => {
                    note(key, true);
                    note(old, false);
                    self.down = Some(key);
                }
                Some(_) => {}
            }
        } else if let Some(old) = self.down.take() {
            note(old, false);
            self.open = 0;
        }
        if self.down.is_none() {
            self.open = self.open.saturating_add(1);
        }
    }

    /// Its key lifted at once (POLY switched, the voices rebuilt), on the keys `note` plays.
    fn lift(&mut self, mut note: impl FnMut(i32, bool)) {
        if let Some(old) = self.down.take() {
            note(old, false);
        }
    }

    /// As on a voice at rest: no key down, the trigger contact open long enough.
    fn rest(&mut self) {
        self.down = None;
        self.open = self.gap;
    }
}

/// A POLY voice's place: its note, gate and age, whether it still sounds, kept by the
/// engine (events change them without touching the voice); and the voice as it plays,
/// shared with the workers ([`crate::pool`]).
#[derive(Debug)]
struct Slot {
    play: Shared,
    note: u8,
    gate: bool,
    /// The gate the voice saw at its last sample, and a one-sample drop pending (a new note
    /// on a gate that was high).
    shown: bool,
    drop: bool,
    age: u64,
    /// Whether it is played (from its note until it falls silent), and its output's peak
    /// over the block so far.
    active: bool,
    peak: f64,
    /// Its output's peak over the blocks since it was last found sounding or silent, and how
    /// many samples they span ([`SILENT_SPAN`]).
    heard: f64,
    span: usize,
    /// What waits for the voice while a worker late from an earlier run holds it: its key
    /// lifted, and the one instrument's keys pressed or released (POLY off), in order.
    lift: bool,
    notes: [(i32, bool); 32],
    pending: usize,
    /// To be put to rest once All Sound Off has faded the output out.
    rest: bool,
    /// Its note's own expression (decisions.md R-MPE), and whether the voice takes it at once
    /// (a new note on it), not gliding there from the last note's.
    expression: Expression,
    fresh: bool,
    /// Its note's velocity (decisions.md R-VELOCITY).
    velocity: Velocity,
}

impl Slot {
    /// The run's gate, note and drop to its voice. A drop it was given for a run it did not
    /// play (a worker late, the run closed before it began) stays with it for its next.
    fn hand(&self, p: &mut Playing) {
        (p.gate, p.note) = (self.gate, self.note);
        p.drop |= self.drop;
    }

    /// After a run its voice played by the deadline (`ok`) or not: the drop is the voice's
    /// now; the gate it shows is the run's, or, not known to have played it, either that or the
    /// one before, so that a new note on it retriggers whichever it saw (decisions.md R18).
    fn ran(&mut self, ok: bool) {
        self.drop = false;
        self.shown = if ok {
            self.gate
        } else {
            self.shown || self.gate
        };
    }
}

/// DOUBLE's second voice of a note (decisions.md R45, the CA-74's R28): a voice with
/// parts of its own (its character, noise and oscillators' start, a place's past the voices'),
/// its own key, its gains at the last run's end (None before its first run, or while it does
/// not play), and whether it still sounds: with DOUBLE turned off it plays its tail out, its
/// key lifted, until silent for [`QUIET`] samples (how many so far).
#[derive(Debug, Clone)]
struct Twin {
    voice: Voice,
    light: Light,
    character: Character,
    key: PolyKey,
    pan: Option<(f64, f64)>,
    live: bool,
    quiet: usize,
}

impl Twin {
    /// After a run of `n` samples whose loudest was `peak`: live while DOUBLE is on; let go,
    /// until it has been silent long enough; its gains where the run left them.
    fn ran(&mut self, n: usize, doubled: bool, peak: f64, pan: Option<(f64, f64)>) {
        if doubled {
            (self.live, self.quiet) = (true, 0);
        } else if self.live {
            self.quiet = if peak < SILENT { self.quiet + n } else { 0 };
            if self.quiet >= QUIET {
                (self.live, self.quiet) = (false, 0);
            }
        }
        self.pan = pan;
    }
}

/// Voice `k`'s twin for DOUBLE, of an instance seeded `seed`, at `rate` Hz: made as the voice
/// of place `k` past the voices' is. (Boxed, as a spare twin is: a voice is some 34 KB, and a
/// debug build copies what is made about the stack; ten spares' twins unboxed overflowed a
/// test thread's 2 MB. A voice's place stays the size it was.)
fn made_twin(rate: f64, seed: u64, k: usize) -> Box<Twin> {
    let j = k + POLY_VOICES.2;
    Box::new(Twin {
        voice: made_voice(rate, seed, j),
        light: made_light(rate, seed, j),
        character: Character::new(voice_seed(seed, j), j, 3, ENTROPY_KNOBS),
        key: PolyKey::new(rate),
        pan: None,
        live: false,
        quiet: 0,
    })
}

/// A voice as it plays: the voice, its character and its key, and a run's gate, note and
/// drop in, its samples out; with DOUBLE, its twin too.
#[derive(Debug, Clone)]
pub struct Playing {
    voice: Voice,
    /// The light model's voice, and whether it is the one played (QUALITY at LO).
    light: Light,
    potato: bool,
    character: Character,
    key: PolyKey,
    gate: bool,
    note: u8,
    drop: bool,
    /// The last run's samples (left, right) and its output's peak.
    out: [[f64; CHUNK]; 2],
    peak: f64,
    /// Its gains, left and right, at the last run's end, gliding to its place (None before
    /// its first run: there from the start).
    pan: Option<(f64, f64)>,
    /// The engine's panel's generation it plays with ([`Engine::apply`]).
    panel_gen: u64,
    /// Its output went non-finite: silent until the engine puts a clean voice in its place
    /// ([`Engine::restore`]; decisions.md R18).
    broken: bool,
    /// DOUBLE's second voice of the note, and whether DOUBLE is on as the engine's panel
    /// last had it (the twin takes the keys pressed only then).
    twin: Box<Twin>,
    double: bool,
    /// The one instrument's place among VOICES's when it scatters (decisions.md R62):
    /// the place its last attack took (None before its first, or since the pattern started
    /// again: the first place); and whether its trigger contact was closed at its last sample.
    turn: Option<usize>,
    closed: bool,
    /// ULTRA's split crew lent to it (decisions.md R-THREADS), whether this run plays on it,
    /// and what a split run works out before and after (kept here: some 40 KB, too much for
    /// a host's audio thread's stack).
    split: Option<Split>,
    split_now: bool,
    scratch: Option<Box<Scratch>>,
    /// Runs it has played split, for the tests and measurements.
    split_runs: u64,
    /// Its note's expression as it plays it (decisions.md R-MPE).
    expressed: Expressed,
    /// Its glide across the LFOs' step (decisions.md R-LFO-STRESS).
    sweep: Sweep,
}

/// A voice's glide across an LFO step (decisions.md R-LFO-STRESS): each of its samples' move of
/// what the circuit reads of a pitch or CUTOFF ([`Swept`]: CUTOFF's wiper, TUNE, the FREQUENCY
/// wipers), or the light voice (each oscillator's cents, added as ENTROPY's are, and CUTOFF's
/// current into the control node, [`Light::set_cutoff_swept`]), and its samples since the
/// step's start. None (the default): the jacks as
/// they were, to the bit.
#[derive(Debug, Clone, Copy, Default)]
struct Sweep {
    on: bool,
    light: bool,
    circuit: Swept,
    cents: [f64; 3],
    current: f64,
    at: f64,
}

impl Sweep {
    /// From panel `a`, the step's start, in a straight line to `b`, the next step's, over `len`
    /// samples: in what the light voice reads with `light`, else the circuit. Each is after
    /// its knob's law, so the line is a chord of the LFO's curve through it, exact at each step.
    fn between(a: &Panel, b: &Panel, len: f64, light: bool) -> Sweep {
        let mut s = Sweep {
            light,
            ..Sweep::default()
        };
        if light {
            let ((pa, ca), (pb, cb)) = (ca72::light::swept_laws(a), ca72::light::swept_laws(b));
            s.cents = std::array::from_fn(|o| (pb[o] - pa[o]) * 100.0 / len);
            s.current = (cb - ca) / len;
            s.on = s.cents != [0.0; 3] || s.current != 0.0;
        } else {
            let wiper = |x: f64| x.clamp(0.0, 1.0);
            s.circuit = Swept {
                cutoff: (cutoff_track(b.cutoff) - cutoff_track(a.cutoff)) / len,
                tune: (b.tune.clamp(-1.0, 1.0) - a.tune.clamp(-1.0, 1.0)) / len,
                freq: [1, 2].map(|o| (wiper(b.osc[o].freq) - wiper(a.osc[o].freq)) / len),
            };
            s.on = s.circuit != Swept::default();
        }
        s
    }

    /// The next sample's, and it a sample on: None while there is no glide.
    #[inline]
    fn step(&mut self) -> Option<Sweep> {
        if !self.on {
            return None;
        }
        let now = *self;
        self.at += 1.0;
        Some(now)
    }

    /// The light voice's CUTOFF current moved so far, A.
    #[inline]
    fn light_current(&self) -> f64 {
        if self.light {
            self.current * self.at
        } else {
            0.0
        }
    }

    /// Its pitches' and the circuit's CUTOFF's moves so far added to a sample's jacks `j`.
    #[inline]
    fn add(&self, j: &mut Jacks) {
        let at = self.at;
        if self.light {
            for (d, c) in j.detune.iter_mut().zip(self.cents) {
                *d += c * at;
            }
        } else {
            let c = &self.circuit;
            j.swept = Swept {
                cutoff: c.cutoff * at,
                tune: c.tune * at,
                freq: c.freq.map(|f| f * at),
            };
        }
    }
}

/// The most a note's own bend goes either way, semitones (an MPE member channel's bend range
/// at its widest, 96: decisions.md R-MPE).
pub const BEND_MOST: f32 = 96.0;

/// How long a voice's expression takes to follow its note's, s (the time constant of its
/// glide): a controller's steps, a few milliseconds apart, are not heard as steps.
pub const EXPRESSION_GLIDE: f64 = 0.004;

/// A voice's expression as it plays (decisions.md R-MPE): its note's pressure, timbre and bend
/// (semitones) as the engine hands them, where they have glided to, and whether to take them at
/// once; the MODULATION wheel's panel position and MIDI share (0..1) as the engine's panel
/// has them, and whether this voice's wheel is moved from there now.
#[derive(Debug, Clone, Copy, Default)]
struct Expressed {
    to: [f64; 3],
    at: [f64; 3],
    fresh: bool,
    wheel: f64,
    midi: f64,
    moved: bool,
}

impl Expressed {
    /// The share a sample of the way the expression glides, at `rate` Hz.
    fn share(rate: f64) -> f64 {
        1.0 - (-1.0 / (EXPRESSION_GLIDE * rate)).exp()
    }

    /// A sample on, gliding by `share`: what the sample's jacks take, the oscillators' cents
    /// and the cutoff's octaves under `reach`; None while there is no expression at all (the
    /// jacks then as they were, to the bit).
    fn step(&mut self, share: f64, reach: &Reach) -> Option<(f64, f64)> {
        if self.fresh {
            self.fresh = false;
            self.at = self.to;
        }
        if self.at == [0.0; 3] && self.to == [0.0; 3] {
            return None;
        }
        for (at, to) in self.at.iter_mut().zip(self.to) {
            *at += (to - *at) * share;
            if (to - *at).abs() < 1e-9 {
                *at = to;
            }
        }
        let [pressure, timbre, bend] = self.at;
        Some((
            bend * 100.0,
            reach.pressure_filter * pressure + reach.timbre_filter * timbre,
        ))
    }

    /// The MODULATION wheel this voice plays at with `reach`, its own pressure and timbre
    /// moving the wheel's MIDI share as a keyboard's wheel moves it (the panel's position the
    /// least, as ever): with none, the engine's panel's to the bit.
    fn wheel(&self, reach: &Reach) -> f64 {
        let [pressure, timbre, _] = self.at;
        let midi = self.midi + reach.pressure_mod * pressure + reach.timbre_mod * timbre;
        self.wheel
            .max(ca72::modulation::midi_wheel(midi.clamp(0.0, 1.0)))
    }
}

/// Of a note's own timbre and the one every voice shares, the one farther from rest (0), the
/// note's where they are as far (decisions.md R-MPE): a note with none of its own takes
/// the shared, a shared one at rest leaves the note's alone. Timbres measured from 0 (TIMBRE
/// REST at ZERO) are never below it, and this is then the larger, as it had been, to the bit.
pub fn farther(own: f32, shared: f32) -> f32 {
    if shared.abs() > own.abs() {
        shared
    } else {
        own
    }
}

/// The expression's cents and octaves (`e`, [`Expressed::step`]) added to jacks `j`: the bend
/// to oscillators 1 and 2, and to 3 with OSC. 3 CONTROL on (the bend's bus reaches it only
/// then), the octaves to the cutoff.
fn express(j: &mut Jacks, e: Option<(f64, f64)>, osc3: bool) {
    if let Some((cents, octaves)) = e {
        if cents != 0.0 {
            j.detune[0] += cents;
            j.detune[1] += cents;
            if osc3 {
                j.detune[2] += cents;
            }
        }
        if octaves != 0.0 {
            j.cutoff += octaves;
        }
    }
}

/// The most notes a split run carries for a voice (a key's change at most twice a sample).
const RUN_NOTES: usize = 2 * CHUNK;

/// What a split run works out for a voice and its twin: the notes at their samples, the jacks
/// each sample, the samples given and the trigger contact after each.
#[derive(Debug, Clone)]
struct Scratch {
    notes: [[(usize, i32, bool); RUN_NOTES]; 2],
    count: [usize; 2],
    jacks: [[Jacks; CHUNK]; 2],
    out: [[f64; CHUNK]; 2],
    closed: [[bool; CHUNK]; 2],
}

impl Scratch {
    fn new() -> Box<Scratch> {
        Box::new(Scratch {
            notes: [[(0, 0, false); RUN_NOTES]; 2],
            count: [0; 2],
            jacks: [[Jacks::default(); CHUNK]; 2],
            out: [[0.0; CHUNK]; 2],
            closed: [[false; CHUNK]; 2],
        })
    }
}

/// Whether voice `p` plays this run on `crew`, lent it now: unless it holds a crew whose pool
/// has gone (the last of it, which is not dropped on the audio thread: a later pool takes it
/// back, [`crate::pool::Play::recall`]).
fn lend(p: &mut Playing, crew: &Split) -> bool {
    match &p.split {
        Some(a) if Arc::ptr_eq(a, crew) => true,
        Some(a) if Arc::strong_count(a) == 1 => false,
        _ => {
            p.split = Some(crew.clone());
            true
        }
    }
}

/// Where a run puts a voice and its twin: their gains (left, right), DOUBLE's half detune,
/// cents, whether DOUBLE is on and whether the twin plays.
struct Places {
    voice: (f64, f64),
    twin: (f64, f64),
    half: f64,
    doubled: bool,
    twin_on: bool,
}

/// A voice's run in [`Playing::play_pair`]: its places, its pan as it glides, its peak, its
/// key.
struct Lane {
    p: Places,
    gl: f64,
    gr: f64,
    peak: f64,
    note: i32,
    /// The expression's glide a sample ([`Expressed::share`]).
    glide: f64,
}

impl Playing {
    /// A run of `n` samples with `ext` at EXTERNAL INPUT: its key moved on a sample at a time
    /// (the drop at the first), its character's offsets and its place; with DOUBLE its twin
    /// too; the samples in `out`, the peak in `peak`. A sample that is not finite marks the
    /// voice broken, and it is silent from there.
    pub fn play(&mut self, n: usize, ext: &[f64; CHUNK], rate: f64, m: &Mix) {
        self.play_from(&ext[..n], rate, m, glide_share(rate));
    }

    /// As [`Playing::play`], for as many samples as `ext` has, its place gliding by `share`
    /// a sample ([`glide_share`] at `rate`).
    fn play_from(&mut self, ext: &[f64], rate: f64, m: &Mix, share: f64) {
        self.express_wheel(m);
        if self.potato {
            self.play_light_from(ext, rate, m, share);
            return;
        }
        if self.split_now
            && let Some(crew) = self.split.take()
        {
            let played = match crew.try_lock() {
                Ok(mut h) => self.play_split_from(&mut h, ext, rate, m, share),
                Err(_) => false,
            };
            self.split = Some(crew);
            if played {
                return;
            }
        }
        let mut p = self.places(m);
        self.voice.set_drive(m.drive);
        self.twin.voice.set_drive(m.drive);
        let (mut gl, mut gr) = self.pan.unwrap_or(p.voice);
        let (mut hl, mut hr) = self.twin.pan.unwrap_or(p.twin);
        let (mut peak, mut twin_peak) = (0.0f64, 0.0f64);
        let note = i32::from(self.note);
        let (glide, osc3) = (Expressed::share(rate), self.voice.panel.osc3_control);
        for (k, &x) in ext.iter().enumerate() {
            let gate = if self.drop {
                self.drop = false;
                false
            } else {
                self.gate
            };
            gl += (p.voice.0 - gl) * share;
            gr += (p.voice.1 - gr) * share;
            if p.twin_on {
                hl += (p.twin.0 - hl) * share;
                hr += (p.twin.1 - hr) * share;
            }
            if self.broken {
                (self.out[0][k], self.out[1][k]) = (0.0, 0.0);
                continue;
            }
            if p.twin_on && self.voice.pairs() && self.twin.voice.pairs() {
                // HI with the twin sounding: the voice's and its twin's filters solved side by
                // side (decisions.md R-HI-PAIRS), each the same samples to the bit. Their keys
                // and jacks first (neither hangs on the other's output), then the two to the
                // filter, the pair, and each on from it.
                let voice = &mut self.voice;
                self.key.step(gate, note, |k, on| voice.note(k, on));
                let e = self.expressed.step(glide, &m.reach);
                let sw = self.sweep.step();
                let mut j = jacks(&mut self.character, x, rate, m, k);
                if let Some(sw) = &sw {
                    sw.add(&mut j);
                }
                if p.doubled {
                    j.detune = j.detune.map(|d| d - p.half);
                }
                express(&mut j, e, osc3);
                let t = &mut self.twin;
                let twin = &mut t.voice;
                t.key
                    .step(gate && p.doubled, note, |k, on| twin.note(k, on));
                let mut jt = jacks(&mut t.character, x, rate, m, k);
                if let Some(sw) = &sw {
                    sw.add(&mut jt);
                }
                if p.doubled {
                    jt.detune = jt.detune.map(|d| d + p.half);
                }
                express(&mut jt, e, osc3);
                let hv = self.voice.before_filter(&j);
                let ht = self.twin.voice.before_filter(&jt);
                let (fv, ft) = Voice::filter_pair(&mut self.voice, &hv, &mut self.twin.voice, &ht);
                let mut y = self.voice.after_filter(&hv, fv);
                if !y.is_finite() {
                    self.broken = true;
                    y = 0.0;
                }
                self.turned(self.voice.trigger_closed(), m, &mut p);
                let (mut left, mut right, mut most) = (y * gl, y * gr, y.abs());
                let mut z = self.twin.voice.after_filter(&ht, ft);
                if !z.is_finite() {
                    self.broken = true;
                    z = 0.0;
                }
                left += z * hl;
                right += z * hr;
                most = most.max(z.abs());
                twin_peak = twin_peak.max(z.abs());
                peak = peak.max(most);
                self.out[0][k] = left;
                self.out[1][k] = right;
                continue;
            }
            let voice = &mut self.voice;
            self.key.step(gate, note, |k, on| voice.note(k, on));
            let e = self.expressed.step(glide, &m.reach);
            let sw = self.sweep.step();
            let mut j = jacks(&mut self.character, x, rate, m, k);
            if let Some(sw) = &sw {
                sw.add(&mut j);
            }
            if p.doubled {
                j.detune = j.detune.map(|d| d - p.half);
            }
            express(&mut j, e, osc3);
            let mut y = self.voice.tick_jacks(&j);
            if !y.is_finite() {
                self.broken = true;
                y = 0.0;
            }
            self.turned(self.voice.trigger_closed(), m, &mut p);
            let (mut left, mut right, mut most) = (y * gl, y * gr, y.abs());
            if p.twin_on {
                let t = &mut self.twin;
                // (A twin let go plays its tail out, its key lifted.)
                let voice = &mut t.voice;
                t.key
                    .step(gate && p.doubled, note, |k, on| voice.note(k, on));
                let mut j = jacks(&mut t.character, x, rate, m, k);
                if let Some(sw) = &sw {
                    sw.add(&mut j);
                }
                if p.doubled {
                    j.detune = j.detune.map(|d| d + p.half);
                }
                express(&mut j, e, osc3);
                let mut z = t.voice.tick_jacks(&j);
                if !z.is_finite() {
                    self.broken = true;
                    z = 0.0;
                }
                left += z * hl;
                right += z * hr;
                most = most.max(z.abs());
                twin_peak = twin_peak.max(z.abs());
            }
            peak = peak.max(most);
            self.out[0][k] = left;
            self.out[1][k] = right;
        }
        self.peak = peak;
        self.pan = Some((gl, gr));
        self.twin.ran(
            ext.len(),
            p.doubled,
            twin_peak,
            p.twin_on.then_some((hl, hr)),
        );
    }

    /// Two voices' runs ([`Playing::play`]) side by side, a sample of each in turn, their
    /// filters solved together (decisions.md R-HI-PAIRS) where both play HI on their own (no
    /// twin sounding, not split, not broken at the start): each voice the same samples to the
    /// bit as played alone; else one run after the other.
    pub fn play_pair(
        a: &mut Playing,
        b: &mut Playing,
        n: usize,
        ext: &[f64; CHUNK],
        rate: f64,
        m: &Mix,
    ) {
        let share = glide_share(rate);
        let ext = &ext[..n];
        let alone = |p: &Playing| {
            p.potato || p.split_now || p.broken || !p.voice.pairs() || p.places(m).twin_on
        };
        if alone(a) || alone(b) {
            a.play_from(ext, rate, m, share);
            b.play_from(ext, rate, m, share);
            return;
        }
        let (mut la, mut lb) = (a.lane(m, rate), b.lane(m, rate));
        for (k, &x) in ext.iter().enumerate() {
            let ja = a.lane_in(&mut la, k, x, rate, m, share);
            let jb = b.lane_in(&mut lb, k, x, rate, m, share);
            match (ja, jb) {
                (Some(ja), Some(jb)) => {
                    let ha = a.voice.before_filter(&ja);
                    let hb = b.voice.before_filter(&jb);
                    let (fa, fb) = Voice::filter_pair(&mut a.voice, &ha, &mut b.voice, &hb);
                    let ya = a.voice.after_filter(&ha, fa);
                    a.lane_out(&mut la, k, ya, m);
                    let yb = b.voice.after_filter(&hb, fb);
                    b.lane_out(&mut lb, k, yb, m);
                }
                (Some(ja), None) => {
                    let y = a.voice.tick_jacks(&ja);
                    a.lane_out(&mut la, k, y, m);
                }
                (None, Some(jb)) => {
                    let y = b.voice.tick_jacks(&jb);
                    b.lane_out(&mut lb, k, y, m);
                }
                (None, None) => {}
            }
        }
        a.lane_end(&la, n);
        b.lane_end(&lb, n);
    }

    /// [`Playing::play_from`]'s start without a twin sounding: its places, DRIVE and pan.
    fn lane(&mut self, m: &Mix, rate: f64) -> Lane {
        self.express_wheel(m);
        let p = self.places(m);
        self.voice.set_drive(m.drive);
        self.twin.voice.set_drive(m.drive);
        let (gl, gr) = self.pan.unwrap_or(p.voice);
        Lane {
            p,
            gl,
            gr,
            peak: 0.0,
            note: i32::from(self.note),
            glide: Expressed::share(rate),
        }
    }

    /// A sample's start in [`Playing::play_from`] without a twin sounding: the gate, the pan
    /// gliding, the key and the jacks; None (the sample silent) if the voice is broken.
    fn lane_in(
        &mut self,
        l: &mut Lane,
        k: usize,
        x: f64,
        rate: f64,
        m: &Mix,
        share: f64,
    ) -> Option<Jacks> {
        let gate = if self.drop {
            self.drop = false;
            false
        } else {
            self.gate
        };
        l.gl += (l.p.voice.0 - l.gl) * share;
        l.gr += (l.p.voice.1 - l.gr) * share;
        if self.broken {
            (self.out[0][k], self.out[1][k]) = (0.0, 0.0);
            return None;
        }
        let voice = &mut self.voice;
        self.key.step(gate, l.note, |k, on| voice.note(k, on));
        let e = self.expressed.step(l.glide, &m.reach);
        let sw = self.sweep.step();
        let mut j = jacks(&mut self.character, x, rate, m, k);
        if let Some(sw) = &sw {
            sw.add(&mut j);
        }
        if l.p.doubled {
            j.detune = j.detune.map(|d| d - l.p.half);
        }
        express(&mut j, e, self.voice.panel.osc3_control);
        Some(j)
    }

    /// A sample's end in [`Playing::play_from`] without a twin sounding: the voice's output
    /// `y` placed.
    fn lane_out(&mut self, l: &mut Lane, k: usize, y: f64, m: &Mix) {
        let mut y = y;
        if !y.is_finite() {
            self.broken = true;
            y = 0.0;
        }
        self.turned(self.voice.trigger_closed(), m, &mut l.p);
        let (left, right, most) = (y * l.gl, y * l.gr, y.abs());
        l.peak = l.peak.max(most);
        self.out[0][k] = left;
        self.out[1][k] = right;
    }

    /// [`Playing::play_from`]'s end without a twin sounding, `n` samples played.
    fn lane_end(&mut self, l: &Lane, n: usize) {
        self.peak = l.peak;
        self.pan = Some((l.gl, l.gr));
        self.twin.ran(n, l.p.doubled, 0.0, None);
    }

    /// [`Playing::play_from`] on helpers (ULTRA, decisions.md R-THREADS): the voice, then its
    /// twin, each on the crew's six threads, the same samples to the bit. The gates, keys and
    /// jacks are worked out first (none hangs on the voices' output), then the voices played,
    /// then their places and SCATTER's turn sample by sample as the whole run has them. False,
    /// and nothing played, if the voices cannot be split (broken, FEEDBACK on).
    fn play_split_from(
        &mut self,
        crew: &mut Helpers,
        ext: &[f64],
        rate: f64,
        m: &Mix,
        share: f64,
    ) -> bool {
        let n = ext.len().min(CHUNK);
        let mut p = self.places(m);
        if self.broken
            || n < ext.len()
            || !Helpers::take(&self.voice)
            || (p.twin_on && !Helpers::take(&self.twin.voice))
        {
            return false;
        }
        let Some(mut s) = self.scratch.take() else {
            return false;
        };
        self.voice.set_drive(m.drive);
        self.twin.voice.set_drive(m.drive);
        let note = i32::from(self.note);
        let (glide, osc3) = (Expressed::share(rate), self.voice.panel.osc3_control);
        s.count = [0; 2];
        for (k, &x) in ext.iter().enumerate() {
            let gate = if self.drop {
                self.drop = false;
                false
            } else {
                self.gate
            };
            let (notes, count) = (&mut s.notes[0], &mut s.count[0]);
            self.key.step(gate, note, |key, on| {
                if *count < RUN_NOTES {
                    notes[*count] = (k, key, on);
                    *count += 1;
                }
            });
            let e = self.expressed.step(glide, &m.reach);
            let sw = self.sweep.step();
            let mut j = jacks(&mut self.character, x, rate, m, k);
            if let Some(sw) = &sw {
                sw.add(&mut j);
            }
            if p.doubled {
                j.detune = j.detune.map(|d| d - p.half);
            }
            express(&mut j, e, osc3);
            s.jacks[0][k] = j;
            if p.twin_on {
                let t = &mut *self.twin;
                let (notes, count) = (&mut s.notes[1], &mut s.count[1]);
                t.key.step(gate && p.doubled, note, |key, on| {
                    if *count < RUN_NOTES {
                        notes[*count] = (k, key, on);
                        *count += 1;
                    }
                });
                let mut j = jacks(&mut t.character, x, rate, m, k);
                if let Some(sw) = &sw {
                    sw.add(&mut j);
                }
                if p.doubled {
                    j.detune = j.detune.map(|d| d + p.half);
                }
                express(&mut j, e, osc3);
                s.jacks[1][k] = j;
            }
        }
        {
            let Scratch {
                notes,
                count,
                jacks,
                out,
                closed,
            } = &mut *s;
            let [o0, o1] = out;
            let [c0, c1] = closed;
            crew.play(
                &mut self.voice,
                &notes[0][..count[0]],
                &jacks[0][..n],
                &mut o0[..n],
                &mut c0[..n],
            );
            if p.twin_on {
                crew.play(
                    &mut self.twin.voice,
                    &notes[1][..count[1]],
                    &jacks[1][..n],
                    &mut o1[..n],
                    &mut c1[..n],
                );
            }
        }
        let (mut gl, mut gr) = self.pan.unwrap_or(p.voice);
        let (mut hl, mut hr) = self.twin.pan.unwrap_or(p.twin);
        let (mut peak, mut twin_peak) = (0.0f64, 0.0f64);
        for k in 0..n {
            gl += (p.voice.0 - gl) * share;
            gr += (p.voice.1 - gr) * share;
            if p.twin_on {
                hl += (p.twin.0 - hl) * share;
                hr += (p.twin.1 - hr) * share;
            }
            if self.broken {
                (self.out[0][k], self.out[1][k]) = (0.0, 0.0);
                continue;
            }
            let mut y = s.out[0][k];
            if !y.is_finite() {
                self.broken = true;
                y = 0.0;
            }
            self.turned(s.closed[0][k], m, &mut p);
            let (mut left, mut right, mut most) = (y * gl, y * gr, y.abs());
            if p.twin_on {
                let mut z = s.out[1][k];
                if !z.is_finite() {
                    self.broken = true;
                    z = 0.0;
                }
                left += z * hl;
                right += z * hr;
                most = most.max(z.abs());
                twin_peak = twin_peak.max(z.abs());
            }
            peak = peak.max(most);
            self.out[0][k] = left;
            self.out[1][k] = right;
        }
        self.scratch = Some(s);
        self.split_runs += 1;
        self.peak = peak;
        self.pan = Some((gl, gr));
        self.twin
            .ran(n, p.doubled, twin_peak, p.twin_on.then_some((hl, hr)));
        true
    }

    /// The split crew it holds, handed to `into` while it has room ([`crate::pool::Play`]).
    pub(crate) fn recall(&mut self, into: &mut Vec<Split>) {
        if into.len() < into.capacity()
            && let Some(a) = self.split.take()
        {
            into.push(a);
        }
        self.split_now = false;
    }

    /// [`Playing::play_from`] at QUALITY's LO: the light voices played in the circuit's
    /// voices' place, with their keys, characters, places and DOUBLE's twin as theirs (the
    /// character's offsets read each sample, as the circuit's voice reads them, so that a
    /// voice's character is the same whichever plays it).
    fn play_light_from(&mut self, ext: &[f64], rate: f64, m: &Mix, share: f64) {
        let mut p = self.places(m);
        self.light.set_drive(m.drive);
        self.twin.light.set_drive(m.drive);
        let (mut gl, mut gr) = self.pan.unwrap_or(p.voice);
        let (mut hl, mut hr) = self.twin.pan.unwrap_or(p.twin);
        let (mut peak, mut twin_peak) = (0.0f64, 0.0f64);
        let note = i32::from(self.note);
        let (glide, osc3) = (Expressed::share(rate), self.voice.panel.osc3_control);
        for (k, &x) in ext.iter().enumerate() {
            let gate = if self.drop {
                self.drop = false;
                false
            } else {
                self.gate
            };
            gl += (p.voice.0 - gl) * share;
            gr += (p.voice.1 - gr) * share;
            if p.twin_on {
                hl += (p.twin.0 - hl) * share;
                hr += (p.twin.1 - hr) * share;
            }
            if self.broken {
                (self.out[0][k], self.out[1][k]) = (0.0, 0.0);
                continue;
            }
            let light = &mut self.light;
            self.key.step(gate, note, |k, on| light.note(k, on));
            let e = self.expressed.step(glide, &m.reach);
            let sw = self.sweep.step();
            let mut j = jacks(&mut self.character, x, rate, m, k);
            if let Some(sw) = &sw {
                sw.add(&mut j);
            }
            if p.doubled {
                j.detune = j.detune.map(|d| d - p.half);
            }
            express(&mut j, e, osc3);
            self.light.set_offsets(j.detune, j.cutoff);
            self.light
                .set_cutoff_swept(sw.as_ref().map_or(0.0, Sweep::light_current));
            self.light.set_timed(j.timed);
            let mut y = self.light.tick(x);
            if !y.is_finite() {
                self.broken = true;
                y = 0.0;
            }
            self.turned(self.light.trigger_closed(), m, &mut p);
            let (mut left, mut right, mut most) = (y * gl, y * gr, y.abs());
            if p.twin_on {
                let t = &mut *self.twin;
                let light = &mut t.light;
                t.key
                    .step(gate && p.doubled, note, |k, on| light.note(k, on));
                let mut j = jacks(&mut t.character, x, rate, m, k);
                if let Some(sw) = &sw {
                    sw.add(&mut j);
                }
                if p.doubled {
                    j.detune = j.detune.map(|d| d + p.half);
                }
                express(&mut j, e, osc3);
                t.light.set_offsets(j.detune, j.cutoff);
                t.light
                    .set_cutoff_swept(sw.as_ref().map_or(0.0, Sweep::light_current));
                t.light.set_timed(j.timed);
                let mut z = t.light.tick(x);
                if !z.is_finite() {
                    self.broken = true;
                    z = 0.0;
                }
                left += z * hl;
                right += z * hr;
                most = most.max(z.abs());
                twin_peak = twin_peak.max(z.abs());
            }
            peak = peak.max(most);
            self.out[0][k] = left;
            self.out[1][k] = right;
        }
        self.peak = peak;
        self.pan = Some((gl, gr));
        self.twin.ran(
            ext.len(),
            p.doubled,
            twin_peak,
            p.twin_on.then_some((hl, hr)),
        );
    }

    /// Where a run with `m` puts the voice and its twin: with DOUBLE the pair about the
    /// centre, the voice half the detune flat to the right and its twin half sharp as far to
    /// the left, SPREAD's way out as far as the placement puts the voice (the CA-74's R28 and
    /// R41); else the voice at its own place, or the one instrument scattering at its turn's
    /// (a twin let go plays its tail where it was).
    fn places(&self, m: &Mix) -> Places {
        let doubled = m.double > 0.0;
        let twin_on = doubled || self.twin.live;
        // (One voice alone, no placement among others: its pair all of SPREAD's way out; a twin
        // let go as the one instrument starts to scatter, there still.)
        let out = match m.placement {
            _ if !twin_on => 0.0,
            Some(p) if !m.scatter => self.character.pair(p, m.voices),
            _ => ALONE,
        };
        let wide = |(l, r): (f32, f32)| (f64::from(l), f64::from(r));
        let [voice, twin] = pair_gains(m.spread, m.inner, out);
        Places {
            voice: if doubled {
                wide(voice)
            } else if m.scatter {
                self.scattered(m)
            } else {
                let placement = m.placement.unwrap_or_default();
                wide(self.character.gains(m.spread, m.inner, placement, m.voices))
            },
            twin: if twin_on { wide(twin) } else { (0.0, 0.0) },
            half: 0.5 * m.double,
            doubled,
            twin_on,
        }
    }

    /// The one instrument's gains at its turn's place among VOICES's, scattering (decisions.md
    /// R62): before its first attack, the first place.
    fn scattered(&self, m: &Mix) -> (f64, f64) {
        let placement = m.placement.unwrap_or_default();
        let at = self.turn.unwrap_or(0);
        let (l, r) = voice_gains(m.spread, m.inner, placement, at, m.voices);
        (f64::from(l), f64::from(r))
    }

    /// Its trigger contact `closed` at the sample just played: closing (an attack's start) with
    /// the one instrument scattering, its place the next of VOICES's, from the next sample
    /// (decisions.md R62). A note played legato, its contact staying closed, stays.
    fn turned(&mut self, closed: bool, m: &Mix, p: &mut Places) {
        if m.scatter && closed && !self.closed {
            self.turn = Some(self.turn.map_or(0, |t| (t + 1) % m.voices.max(1)));
            p.voice = self.scattered(m);
        }
        self.closed = closed;
    }

    /// A run's start: a new note's expression taken at once, and the MODULATION wheel, its
    /// twin's too, where the voice's own pressure and timbre move it under `m`'s reach
    /// (decisions.md R-MPE); back at the engine's panel's once they no longer move it, and
    /// untouched while they never have.
    fn express_wheel(&mut self, m: &Mix) {
        let x = &mut self.expressed;
        if std::mem::take(&mut x.fresh) {
            x.at = x.to;
        }
        let moves = m.reach.moves_wheel();
        if !moves && !x.moved {
            return;
        }
        x.moved = moves;
        let w = x.wheel(&m.reach);
        let potato = self.potato;
        let t = &mut *self.twin;
        for (voice, light) in [
            (&mut self.voice, &mut self.light),
            (&mut t.voice, &mut t.light),
        ] {
            if voice.panel.mod_wheel != w {
                voice.panel.mod_wheel = w;
                if potato {
                    light.set_panel(&voice.panel);
                }
            }
        }
    }

    /// A key pressed or released on the voice's keyboard (the one instrument's, or UNISON's),
    /// and on its twin's: pressed there only with DOUBLE on, so that a twin let go plays its
    /// tail out.
    fn note(&mut self, key: i32, on: bool) {
        let twin = self.double || !on;
        if self.potato {
            self.light.note(key, on);
            if twin {
                self.twin.light.note(key, on);
            }
        } else {
            self.voice.note(key, on);
            if twin {
                self.twin.voice.note(key, on);
            }
        }
    }

    /// Its key and its twin's lifted at once, on the voices played.
    fn lift(&mut self) {
        let t = &mut *self.twin;
        if self.potato {
            let (light, twin) = (&mut self.light, &mut t.light);
            self.key.lift(|k, on| light.note(k, on));
            t.key.lift(|k, on| twin.note(k, on));
        } else {
            let (voice, twin) = (&mut self.voice, &mut t.voice);
            self.key.lift(|k, on| voice.note(k, on));
            t.key.lift(|k, on| twin.note(k, on));
        }
    }
}

/// A clean voice for each voice's place, made off the audio thread, to take a voice's place
/// on it without allocating (decisions.md R18): a voice whose output went non-finite, and
/// every sounding voice at All Sound Off. The voice taken out is dropped off the audio thread
/// too, when the spares are next mended ([`Spares::mend`]). The audio thread only tries their
/// locks.
#[derive(Debug, Default)]
pub struct Spares {
    places: [Mutex<Spare>; POLY_VOICES.2],
}

/// A place's spare voice and twin (DOUBLE's): clean, or the ones they replaced. (Each boxed,
/// as [`made_twin`]'s twin is: ten voices inline made [`Spares`] some 440 KB, which `Arc::new`
/// built on the stack of whatever thread made the engine, a host's making an instance:
/// decisions.md R-HOST-STACK.)
#[derive(Debug, Default)]
struct Spare {
    voice: Option<Box<Voice>>,
    twin: Option<Box<Voice>>,
    clean: bool,
}

impl Spares {
    /// Each place whose voice was taken given a clean one again, at its rate, and the one
    /// taken out dropped. Not on the audio thread: the plug-in has its helper thread do it when
    /// the engine asks ([`Engine::take_mending`]; decisions.md R23).
    pub fn mend(&self) {
        self.mend_until(|| false);
    }

    /// As [`Spares::mend`], stopping before the next voice once `stop` (the plug-in dropped:
    /// decisions.md R23), so that waiting for it takes at most one voice's making.
    pub fn mend_until(&self, stop: impl Fn() -> bool) {
        for place in &self.places {
            if stop() {
                return;
            }
            let used = place
                .lock()
                .ok()
                .filter(|s| !s.clean)
                .and_then(|s| s.voice.as_ref().map(|v| v.rate()));
            if let Some(rate) = used {
                fill(place, rate, |s| {
                    !s.clean && s.voice.as_ref().map(|v| v.rate()) == Some(rate)
                });
            }
        }
    }

    /// Every place given a clean voice at `rate` Hz (the voices' own) unless it has one.
    fn make(&self, rate: f64) {
        for place in &self.places {
            fill(place, rate, |s| {
                !(s.clean && s.voice.as_ref().map(|v| v.rate()) == Some(rate))
            });
        }
    }
}

/// A clean voice and twin at `rate` Hz put in `place` while it is `wanted` there: made before
/// the lock is taken, and the ones they replace dropped after.
fn fill(place: &Mutex<Spare>, rate: f64, wanted: impl Fn(&Spare) -> bool) {
    if !place.lock().is_ok_and(|s| wanted(&s)) {
        return;
    }
    let (mut voice, mut twin) = (
        Some(Box::new(Voice::prototype(rate))),
        Some(Box::new(Voice::prototype(rate))),
    );
    if let Ok(mut s) = place.lock()
        && wanted(&s)
    {
        std::mem::swap(&mut s.voice, &mut voice);
        std::mem::swap(&mut s.twin, &mut twin);
        s.clean = true;
    }
    drop((voice, twin));
}

/// The voices, the MIDI input's state and the controls they play with.
#[derive(Debug)]
pub struct Engine {
    slots: Vec<Slot>,
    /// Each voice's output's peak over the last block (the editor's drops and lamps follow
    /// them; each slot's peak is kept anyway, for knowing when its voice falls silent).
    levels: [f32; LEVELS],
    /// The one instrument asleep, or how near it is (decisions.md R68).
    rest: Rest,
    over: Option<Oversampled>,
    /// HI's and LO's output held back to come out with ULTRA's ([`LINE_UP`]).
    line_up: OutputDelay,
    rate: f64,
    seed: u64,
    controls: Controls,
    gain: f64,
    /// Diagnostics for detached renders: recovered or sanitised output cannot be used as
    /// a calibration measurement. Live recovery's audio behaviour is unchanged.
    render_recoveries: u64,
    render_sanitised: u64,
    /// Keys held (MIDI numbers), played on by voices built for a new rate (POLY off).
    held: [bool; 128],
    /// POLY's notes taken so far (each voice's age).
    counter: u64,
    bend: f32,
    modulation: f32,
    /// The notes' expression (decisions.md R-MPE): each key's own as last told, the pressure
    /// and timbre every voice shares, when each key was last pressed (NOTE PRIORITY's LAST)
    /// and the key the instruments sound (POLY off, UNISON), whose expression they take.
    keys: [Expression; 128],
    pressure: f32,
    timbre: f32,
    pressed: [u64; 128],
    presses: u64,
    sounding: Option<u8>,
    /// Each key's velocity as last told, the velocity every voice shares (the latest note's,
    /// the instruments' sounding key's: decisions.md R-VELOCITY) and the latest note's as it
    /// came (for the editor's meter).
    velocities: [f32; 128],
    velocity: Velocity,
    last_velocity: f32,
    lamp: f32,
    /// A voice's place's glide a sample at the voices' rate ([`glide_share`]), and the one
    /// instrument's while it scatters ([`TURN_GLIDE`]).
    glide: f64,
    turn_glide: f64,
    /// The place the one instrument's voice took at its last attack, scattering (the editor
    /// shows it), and whether its pattern starts again from the first place at its next run
    /// (every key let go by the host: decisions.md R62).
    turn: usize,
    restart: bool,
    /// AUTO GAIN (decisions.md R45): the sound's curve (`drive.rs`), the correction the
    /// controls ask for from it (1: none), the correction gliding to it a sample at a time,
    /// and the glide's share a sample at the host's rate: the controls' (DRIVE turned, a
    /// preset's or a session's curve, [`GLIDE`]'s), a measured curve's ([`REFINE_GLIDE`]'s,
    /// R-AUTO-GAIN-DRIVE), and the one gliding now. Only it glides: the output's other gains
    /// step as they did.
    curve: Curve<STEPS>,
    auto: f64,
    auto_now: f64,
    auto_share: f64,
    auto_refine: f64,
    auto_glide: f64,
    /// Where AUTO GAIN's measurements are asked for and kept, told the voices' rate.
    calibration: Option<Calibration>,
    /// The output's gain under POWER (0 off, 1 on), and its step a sample while it fades.
    fade: f64,
    fade_step: f64,
    /// All Sound Off under way (the output fading out, or out and the voices being put to
    /// rest), its fade's step a sample and the samples it has waited, faded out.
    hush: bool,
    hush_step: f64,
    hush_waited: usize,
    /// The clean voices put in voices' places, and whether some are to be made again.
    spares: Arc<Spares>,
    mend: bool,
    /// The panel's generation (each [`Engine::apply`] a new one), and the voices'
    /// preamplifier inside FEEDBACK's loop.
    panel_gen: u64,
    in_loop: ca72::preamp::InLoop,
    /// The workers sharing POLY's voices (none: every voice on the caller's thread), how many
    /// were asked for and the block period they were made audio threads for; and the
    /// present block's deadline for them.
    pool: Option<Pool>,
    workers: (usize, Option<Duration>),
    deadline: Option<Instant>,
    /// Voices' runs not heard (a worker past the deadline, or the voice still held by one), for
    /// the measurements ([`Engine::silent_runs`]).
    silent_runs: u64,
    /// The host's thread's load, block by block, and whether LO shares its voices for it where
    /// this system shares them by load ([`LoSharing::Load`], decisions.md R-LO-WORKERS); when
    /// the present block began, and the runs not heard by then.
    busy: Busy,
    block_start: Option<Instant>,
    block_silent: u64,
    /// Where POLY's workers are started and stopped off the audio thread, the plan they are
    /// started for there (0: none), this thread's ask for them and whether the helper
    /// thread is to serve it (decisions.md R21, R23).
    crew: Arc<Crew>,
    plan: u64,
    ask: Ask,
    serve: bool,
    /// How often the workers were started (for the tests).
    #[cfg(test)]
    pool_starts: u64,
    /// The clock oscillator 3 timed keeps (decisions.md R-OSC-TIMED): where the music was as
    /// the block began ([`Engine::set_position`]; until told, 120 BPM from the start), the host
    /// samples played since, and the host sample the voices are at now, from it.
    position: Position,
    since: f64,
    cursor: f64,
    /// The controls as the plug-in gave them, before the LFOs move their knobs (decisions.md
    /// R-LFO; `controls` is them moved, and what everything plays by); the LFOs' generators;
    /// and how long since the modulation's last step, s (EDGES' slew).
    base: Controls,
    bank: Box<crate::lfo::Bank>,
    lfo_dt: f64,
    /// The present block's points for the effects' pins (decisions.md R-FX-PARAMS).
    fx: Box<FxPoints>,
    /// The host samples played since the engine was prepared at its rate, and where the
    /// present block began among them: the time the LFOs' ticks keep (R-LFO's addendum).
    played: u64,
    block_from: u64,
    /// The output's gain on its way, a sample at a time, across a modulation's step from where
    /// it was to `gain` (decisions.md R-LFO-STRESS): where it is, its change a sample and the
    /// samples left. None left, `gain` itself.
    gain_now: f64,
    gain_ramp: f64,
    gain_left: usize,
    /// The step's end, where the voices glide a pitch or CUTOFF an LFO moves to
    /// (decisions.md R-LFO-STRESS).
    ahead: Box<Ahead>,
}

/// Where the LFOs will have the knobs at the next step (decisions.md R-LFO-STRESS): whether the
/// voices glide to it, over how many of their samples, the engine's panel there and the LFOs'
/// values there, shared and each voice's own; and whether the step jumps (a SHARP edge in it,
/// [`crate::lfo::JUMP`]), the shared LFOs' and each voice's own, which then hold the step.
#[derive(Debug, Clone, Default)]
struct Ahead {
    on: bool,
    len: f64,
    panel: Panel,
    shared: [f64; crate::lfo::LFOS],
    voices: [[f64; crate::lfo::LFOS]; crate::lfo::VOICES],
    jumps: bool,
    voice_jumps: [bool; crate::lfo::VOICES],
    /// A copy of the generators to run the next step's ticks on, where the LFOs move each
    /// other or TEMPO (R-LFO's addendum): kept here so that the step allocates nothing.
    bank: crate::lfo::Bank,
    /// The share of the way the velocity source glides on at the next step (R-VELOCITY; 0
    /// while nothing takes it): the glide's end reads each voice's there.
    velocity: f64,
}

impl Default for Engine {
    fn default() -> Self {
        Engine::new()
    }
}

#[cfg(test)]
mod offline_health_tests {
    use super::*;
    #[test]
    fn sanitised_output_is_observable_by_the_offline_host() {
        let mut engine = Engine::new();
        assert!(!engine.render_faulted());
        assert_eq!(engine.faded(f64::NAN, 0.0), (0.0, 0.0));
        assert!(
            engine.render_faulted(),
            "zeroed NaN must not become successful calibration audio"
        );
    }
}

/// How a voice's place reaches its voice: `f` on it at once, unless a worker late from an
/// earlier run holds it.
fn with_voice<T>(s: &Slot, f: impl FnOnce(&mut Playing) -> T) -> Option<T> {
    s.play.try_lock().ok().map(|mut p| f(&mut p))
}

impl Engine {
    /// An engine without voices (silent until [`Engine::prepare`]).
    pub fn new() -> Engine {
        let controls = Controls::default();
        Engine {
            slots: Vec::new(),
            levels: [0.0; LEVELS],
            rest: Rest::default(),
            over: None,
            line_up: OutputDelay::new(),
            rate: 0.0,
            position: Position::start(crate::timed::FALLBACK_TEMPO, 48_000.0),
            since: 0.0,
            cursor: 0.0,
            seed: 0,
            gain: output_gain(controls.volume, controls.main_output),
            base: controls,
            bank: Box::default(),
            lfo_dt: 0.0,
            fx: Box::default(),
            played: 0,
            block_from: 0,
            gain_now: 0.0,
            gain_ramp: 0.0,
            gain_left: 0,
            ahead: Box::default(),
            controls,
            held: [false; 128],
            counter: 0,
            bend: 0.0,
            modulation: 0.0,
            keys: [Expression::NONE; 128],
            pressure: 0.0,
            timbre: 0.0,
            pressed: [0; 128],
            presses: 0,
            sounding: None,
            velocities: [Velocity::FULL; 128],
            velocity: Velocity::new(),
            last_velocity: 0.0,
            lamp: 0.0,
            render_recoveries: 0,
            render_sanitised: 0,
            glide: 1.0,
            turn_glide: 1.0,
            turn: 0,
            restart: false,
            curve: AVERAGE,
            auto: 1.0,
            auto_now: 1.0,
            auto_share: 1.0,
            auto_refine: 1.0,
            auto_glide: 1.0,
            calibration: None,
            fade: 1.0,
            fade_step: 1.0,
            hush: false,
            hush_step: 1.0,
            hush_waited: 0,
            spares: Arc::new(Spares::default()),
            mend: false,
            panel_gen: 0,
            in_loop: ca72::preamp::InLoop::default(),
            pool: None,
            workers: (0, None),
            deadline: None,
            silent_runs: 0,
            busy: Busy::default(),
            block_start: None,
            block_silent: 0,
            crew: Arc::new(Crew::new()),
            plan: 0,
            ask: Ask::Not,
            serve: false,
            #[cfg(test)]
            pool_starts: 0,
        }
    }

    /// The voices for `rate` Hz, their noise seeded from `seed` (the first voice's is `seed`
    /// itself, as with one voice), playing the keys held, and their spares ([`Spares`]). Not
    /// on the audio thread: voices at a rate not yet built take up to a second (the
    /// oscillators' factory tuning, the tables); later ones are copies. At the rate they were
    /// built for, the voices are kept, their noise and (for a new seed) their characters
    /// drawn from `seed` again, as a session's saved seed loaded into a running instance has
    /// them (decisions.md R18). New voices stop the workers, here ([`Engine::start_workers`]
    /// plans them again).
    pub fn prepare(&mut self, rate: f64, seed: u64) {
        // On a thread of its own with a main thread's stack (decisions.md R-PREPARE-STACK):
        // a voice made for a rate the first time solves the keyboard circuit, and with the
        // slots' voices, twins and spares built in place that took more than 512 KB, all a
        // macOS host's secondary thread has (pluginval initialising the Audio Unit on its own
        // thread overflowed it). Not the audio thread: hosts prepare a plug-in apart from it.
        let mut once = Some((&mut *self, rate, seed));
        let ran = std::thread::scope(|s| {
            std::thread::Builder::new()
                .name("ca72-prepare".to_owned())
                .stack_size(PREPARE_STACK)
                .spawn_scoped(s, || {
                    if let Some((e, rate, seed)) = once.take() {
                        e.prepare_here(rate, seed);
                    }
                })
                .map(|h| h.join())
        });
        match ran {
            Ok(Ok(())) => {}
            // A panic in it is the caller's, as it would have been here.
            Ok(Err(panic)) => std::panic::resume_unwind(panic),
            // No thread to be had: here, as before.
            Err(_) => {
                if let Some((e, rate, seed)) = once.take() {
                    e.prepare_here(rate, seed);
                }
            }
        }
    }

    /// [`Engine::prepare`]'s work, on whichever thread runs it.
    fn prepare_here(&mut self, rate: f64, seed: u64) {
        if rate != self.rate {
            self.position = Position::start(crate::timed::FALLBACK_TEMPO, rate);
            (self.since, self.cursor) = (0.0, 0.0);
            // (The LFOs' ticks from the start again, at the new rate.)
            (self.played, self.block_from) = (0, 0);
            self.bank.cross.on = false;
        }
        // (The machine's cores, which ULTRA's split crews are sized by, read here, off the
        // audio thread: an instance made at LO read them at ULTRA's first block, a file read
        // and memory taken on the audio thread, once a process.)
        let _ = split_voices();
        self.rest.wake();
        self.rest.window = (SLEEP_WINDOW * rate).ceil() as usize;
        let reseeded = seed != self.seed;
        self.seed = seed;
        if rate == self.rate && !self.slots.is_empty() {
            let voice_rate = self.voice_rate();
            for (k, s) in self.slots.iter_mut().enumerate() {
                if let Ok(mut p) = s.play.lock() {
                    if reseeded {
                        // (Made again, as an instance opened with the seed makes them: its
                        // voices' oscillators start where it says.)
                        p.voice = made_voice(voice_rate, seed, k);
                        p.light = made_light(voice_rate, seed, k);
                        p.character = Character::new(voice_seed(seed, k), k, 3, ENTROPY_KNOBS);
                        p.twin = made_twin(voice_rate, seed, k);
                        p.panel_gen = u64::MAX;
                    } else {
                        p.voice.set_seed(voice_seed(seed, k));
                        p.light.set_seed(voice_seed(seed, k));
                    }
                }
            }
            if reseeded {
                self.apply();
                self.press_held();
            }
            self.auto_now = self.auto;
            self.spares.make(voice_rate);
            return;
        }
        self.stop_workers();
        self.rate = rate;
        self.line_up.clear();
        self.fade_step = (1.0 / (POWER_FADE * rate)).min(1.0);
        self.hush_step = (1.0 / (HUSH_FADE * rate)).min(1.0);
        self.hush = false;
        let factor = [1, 2, 4, 8]
            .into_iter()
            .find(|f| rate * *f as f64 >= MIN_VOICE_RATE)
            .unwrap_or(8);
        self.over = (factor > 1).then(|| Oversampled {
            factor,
            up: Interpolator::new(factor),
            down: [Decimator::new(factor), Decimator::new(factor)],
        });
        let voice_rate = rate * factor as f64;
        self.glide = glide_share(voice_rate);
        self.turn_glide = 1.0 - (-1.0 / (TURN_GLIDE * voice_rate)).exp();
        (self.auto_now, self.auto_share) = (self.auto, glide_share(rate));
        self.auto_refine = 1.0 - (-1.0 / (REFINE_GLIDE * rate)).exp();
        self.auto_glide = self.auto_share;
        if let Some(c) = &self.calibration {
            c.set_rate(voice_rate);
        }
        self.slots = (0..POLY_VOICES.2)
            .map(|k| Slot {
                play: Arc::new(Mutex::new(Playing {
                    voice: made_voice(voice_rate, seed, k),
                    light: made_light(voice_rate, seed, k),
                    potato: false,
                    character: Character::new(voice_seed(seed, k), k, 3, ENTROPY_KNOBS),
                    key: PolyKey::new(voice_rate),
                    gate: false,
                    note: 0,
                    drop: false,
                    out: [[0.0; CHUNK]; 2],
                    peak: 0.0,
                    pan: None,
                    panel_gen: u64::MAX,
                    broken: false,
                    twin: made_twin(voice_rate, seed, k),
                    double: false,
                    turn: None,
                    closed: false,
                    split: None,
                    split_now: false,
                    scratch: Some(Scratch::new()),
                    split_runs: 0,
                    expressed: Expressed::default(),
                    sweep: Sweep::default(),
                })),
                note: 0,
                gate: false,
                shown: false,
                drop: false,
                age: 0,
                active: false,
                peak: 0.0,
                heard: 0.0,
                span: 0,
                lift: false,
                notes: [(0, false); 32],
                pending: 0,
                rest: false,
                expression: Expression::NONE,
                fresh: false,
                velocity: Velocity::new(),
            })
            .collect();
        // (Each voice's lock taken once here, off the audio thread: on macOS the standard
        // library may make a mutex's lock on its first use; history.md, "The worker locks".
        // The spares' are taken as they are made.)
        for s in &self.slots {
            drop(s.play.lock());
        }
        self.spares.make(voice_rate);
        self.apply();
        self.press_held();
    }

    /// The keys held pressed again on the instruments' voices just made (POLY's voices take
    /// the next keys).
    fn press_held(&self) {
        for k in 0..self.instruments() {
            if let Ok(mut p) = self.slots[k].play.lock() {
                for (key, _) in self.held.iter().enumerate().filter(|(_, h)| **h) {
                    p.note(key as i32, true);
                }
            }
        }
    }

    /// Up to `workers` threads of the plug-in's own ([`AUTO`]: as many as the instance's share
    /// of the process's budget and its cap give, decisions.md R-THREADS) to share POLY's voices
    /// with the caller's thread, and at ULTRA this machine's split crews, made audio threads for
    /// blocks of `period` (None: left ordinary, for measurements that run as fast as they go;
    /// decisions.md R11); 0 workers, none. Held only while the instance needs threads (R21, as
    /// R-THREADS changed it: POLY or UNISON on, or ULTRA chosen where this machine splits
    /// voices): started here if it does, else when it comes to, off the audio thread
    /// ([`Crew::serve`]); let go when it no longer does, or started again when its share
    /// moves. Not on the audio thread. Threads already running for these voices, as many and
    /// for the same period, are kept while they are needed (a host's state loaded into a
    /// running instance prepares it again: decisions.md R18). Returns how many workers run
    /// (none with POLY off, or if the system would not start them).
    pub fn start_workers(&mut self, workers: usize, period: Option<Duration>) -> usize {
        if self.pool.is_some() && self.workers == (workers, period) && self.need().is_some() {
            return self.workers();
        }
        // On a thread of its own with a main thread's stack, as `prepare` (decisions.md
        // R-PREPARE-STACK, R-ULTRA-STACK): ULTRA's split crews are made from a voice's parts,
        // and that took some 250 KB of the stack (`Parts::spawn` 177 KB, `Helpers::new` 64 KB)
        // of the host's thread loading a state saved at ULTRA, more than a thread of 256 KB
        // has. Not the audio thread: it starts threads anyway.
        let mut once = Some(&mut *self);
        let ran = std::thread::scope(|s| {
            std::thread::Builder::new()
                .name("ca72-start".to_owned())
                .stack_size(PREPARE_STACK)
                .spawn_scoped(s, || {
                    once.take().map(|e| e.start_workers_here(workers, period))
                })
                .map(|h| h.join())
        });
        match ran {
            Ok(Ok(started)) => started.unwrap_or(0),
            // A panic in it is the caller's, as it would have been here.
            Ok(Err(panic)) => std::panic::resume_unwind(panic),
            // No thread to be had: here, as before.
            Err(_) => once
                .take()
                .map_or(0, |e| e.start_workers_here(workers, period)),
        }
    }

    /// [`Engine::start_workers`]'s work, on whichever thread runs it.
    fn start_workers_here(&mut self, workers: usize, period: Option<Duration>) -> usize {
        self.stop_workers();
        self.workers = (workers, period);
        if (workers == 0 && split_voices() == 0) || self.slots.is_empty() {
            return 0;
        }
        let voices: Vec<Shared> = self.slots.iter().map(|s| s.play.clone()).collect();
        // (ULTRA's split crews made from a voice's parts, spare between runs.)
        let template = self.slots[0]
            .play
            .lock()
            .ok()
            .map(|p| p.voice.boxed_clone());
        self.plan = self.crew.plan(&voices, workers, period, template);
        if let Some(need) = self.need() {
            self.pool = self.crew.start(self.plan, need);
            self.ask = Ask::Answered;
            #[cfg(test)]
            {
                self.pool_starts += 1;
            }
        }
        self.workers()
    }

    /// POLY's workers stopped, here, their share of the process's budget given back, and none
    /// started again until [`Engine::start_workers`] (a plug-in deactivated: decisions.md
    /// R21). Not on the audio thread.
    pub fn stop_workers(&mut self) {
        self.pool = None;
        self.crew.clear();
        (self.plan, self.ask) = (0, Ask::Not);
    }

    /// At a block's start, on the audio thread: POLY's workers as POLY asks (decisions.md
    /// R21), without waiting. POLY switched on, a pool asked of the helper thread
    /// ([`Engine::take_serving`]) and taken once it is ready, every voice played here until
    /// then; switched off, the pool let go, to be stopped there.
    fn follow_poly(&mut self) {
        let on = self.need();
        if self
            .crew
            .follow(on, self.plan, &mut self.pool, &mut self.ask)
        {
            self.serve = true;
        }
    }

    /// Where POLY's workers are started and stopped off the audio thread ([`Crew::serve`]).
    pub fn crew(&self) -> Arc<Crew> {
        self.crew.clone()
    }

    /// Whether POLY's workers are to be started or stopped ([`Crew::serve`], off the audio
    /// thread) since this was last asked.
    pub fn take_serving(&mut self) -> bool {
        std::mem::take(&mut self.serve)
    }

    /// The clean voices put in voices' places ([`Spares`]): for mending off the audio thread.
    pub fn spares(&self) -> Arc<Spares> {
        self.spares.clone()
    }

    /// Whether the spares are to be mended ([`Spares::mend`], off the audio thread) since this
    /// was last asked.
    pub fn take_mending(&mut self) -> bool {
        std::mem::take(&mut self.mend)
    }

    /// Voices' runs not heard so far: a worker not done by the block's deadline, or the voice
    /// still held by a late one (a measurement's count; the plug-in does not read it).
    pub fn silent_runs(&self) -> u64 {
        self.silent_runs
    }

    /// The voices' runs played split so far (ULTRA, decisions.md R-THREADS), and the split crews
    /// held: for the tests and measurements, not the audio thread (it takes the voices' locks).
    #[doc(hidden)]
    pub fn split_runs(&self) -> (u64, usize) {
        let runs = self
            .slots
            .iter()
            .filter_map(|s| s.play.lock().ok().map(|p| p.split_runs))
            .sum();
        (runs, self.pool.as_ref().map_or(0, |p| p.splits().len()))
    }

    /// The workers sharing POLY's voices.
    pub fn workers(&self) -> usize {
        self.pool.as_ref().map_or(0, Pool::workers)
    }

    /// When the present block's voices must be done by: a voice a worker has not finished
    /// then is silent for the run, not waited for (None: waited for). The plug-in sets it at
    /// each block's start, when POLY's workers also follow POLY ([`Engine::follow_poly`]).
    /// Where the music is as a block begins, for oscillator 3 timed (decisions.md
    /// R-OSC-TIMED): its samples' places on the clock are counted from here, each from this,
    /// never added up from the blocks before.
    pub fn set_position(&mut self, p: Position) {
        self.position = p;
        (self.since, self.cursor) = (0.0, 0.0);
        self.block_from = self.played;
        self.fx.clear();
    }

    /// The present block's points for the effects' pins (decisions.md R-FX-PARAMS): where the
    /// sources moving them were read, and each pin's offset there.
    pub fn fx_points(&self) -> &FxPoints {
        &self.fx
    }

    /// The block's place on the clock from here on, the tempo having changed within it (the
    /// standalone's TEMPO moved by a modulation source: decisions.md R-LFO): `p` the block's
    /// start as the samples from here have it ([`crate::timed::Clock::retime`]), the samples'
    /// count since the block began kept.
    pub fn retime(&mut self, p: Position) {
        self.position = p;
    }

    /// The controls with the sources' moves in them now (their shared copies'): what the
    /// strip's screens read (decisions.md R-LFO). Asleep too (R68), when the voices' are not
    /// moved but the generators run on.
    pub fn controls_now(&self) -> Controls {
        let l = &self.base.lfo;
        if l.active() {
            l.modulated(
                &self.base,
                &l.source_values(&self.bank.shared_values, self.velocity.now),
            )
        } else {
            self.base
        }
    }

    /// The tempo the modulation sources have the standalone's TEMPO at now (BPM: in force since
    /// the latest of the LFOs' ticks), if any moves it (decisions.md R-LFO). Never in a plug-in,
    /// whose controls leave TEMPO's assignments out.
    pub fn tempo(&self) -> Option<f64> {
        if self.bank.cross.on && self.base.lfo.tempo_moved() {
            self.bank.cross.tempo
        } else {
            None
        }
    }

    /// Where in the present block the LFOs' next tick falls (host samples from its start: the
    /// first sample on or after its time), while a source moves TEMPO: the tempo in force
    /// changes there.
    pub fn next_tempo_tick(&self) -> Option<usize> {
        self.tempo()?;
        let rate = self.lfo_rate();
        let next = self.bank.cross.latest() + 1;
        let mut s = (next as f64 * rate / crate::lfo::TICKS_A_SECOND).ceil() as u64;
        while crate::lfo::tick_at(s, rate) < next {
            s += 1;
        }
        Some(s.saturating_sub(self.block_from) as usize)
    }

    /// Each oscillator's place on the clock `at` host samples after the block's start, and its
    /// cycles a voice's sample, if it is timed and its RANGE is at LO (at any other RANGE timed
    /// mode does nothing).
    fn timed_at(&self, at: f64) -> [Option<Timed>; 3] {
        let factor = self.over.as_ref().map_or(1, |o| o.factor) as f64;
        std::array::from_fn(|n| {
            let length = self.controls.timed[n]?;
            if self.controls.panel.osc[n].range != ca72::tuning::Range::Lo {
                return None;
            }
            let (phase, per) = self.position.cycle(length);
            let now = phase + at * per;
            Some(Timed {
                phase: now - now.floor(),
                per: per / factor,
            })
        })
    }

    /// [`Engine::timed_at`], for the tests.
    #[cfg(test)]
    pub(crate) fn timed_probe(&self, at: f64) -> [Option<Timed>; 3] {
        self.timed_at(at)
    }

    /// The one instrument's oscillator `n`'s latest reset, how long before its latest sample and
    /// how many it has made (the model QUALITY plays; the light voice's while it is timed), for
    /// the tests.
    #[cfg(test)]
    pub(crate) fn osc_reset(&self, n: usize) -> Option<(f64, u64)> {
        let p = self.slots.first()?.play.try_lock().ok()?;
        Some(if p.potato {
            p.light.probe_timed_reset(n)
        } else {
            p.voice.probe_reset_ago(n)
        })
    }

    pub fn set_deadline(&mut self, deadline: Option<Instant>) {
        self.deadline = deadline;
        // (The host's thread's time in the plug-in is counted from here to the block's end.)
        let now = Instant::now();
        self.block_start = Some(now);
        self.block_silent = self.silent_runs;
        self.follow_poly();
        if let Some(p) = &mut self.pool {
            // (The block's samples, from its deadline, a share of its period from now: what
            // the workers' runs are due within on macOS, decisions.md R-MAC-512. 0 offline.)
            let len = deadline.map_or(0, |d| {
                (d.saturating_duration_since(now).as_secs_f64() / DEADLINE * self.rate).round()
                    as usize
            });
            // (Runs past the deadline still shared until the period's end: R-DROPOUTS.)
            let grace = deadline.map(|d| now + d.saturating_duration_since(now).div_f64(DEADLINE));
            p.block(len, grace);
        }
    }

    /// The controls for the samples that follow. POLY, UNISON or DOUBLE switched lifts every
    /// key first (the notes then sounding stop; the next notes play in the new way).
    pub fn set(&mut self, c: &Controls) {
        if *c != self.base {
            self.rest.wake();
            self.base = *c;
            // (Set at once: no glide until the LFOs' next step.)
            self.ahead.on = false;
            // (The knobs where the LFOs have them now: decisions.md R-LFO. None assigned, the
            // controls as given.)
            let m = if c.lfo.active() {
                let values = c
                    .lfo
                    .source_values(&self.bank.shared_values, self.velocity.now);
                c.lfo.modulated(c, &values)
            } else {
                *c
            };
            self.take(&m, true);
        }
    }

    /// The controls `c` (the LFOs' moves in them) in force: their gains, and the voices' panel,
    /// at once (`eager`) or as each voice next plays.
    fn take(&mut self, c: &Controls, eager: bool) {
        {
            if c.poly != self.controls.poly
                || c.potato != self.controls.potato
                || c.unison != self.controls.unison
                || (c.unison && c.voices != self.controls.voices)
                || (c.double > 0.0) != (self.controls.double > 0.0)
            {
                self.lift_all();
            }
            if c.voices < self.controls.voices {
                // The voices past the new count let go (they run on until silent).
                for s in self.slots.iter_mut().skip(c.voices.max(1)) {
                    s.gate = false;
                }
            }
            self.controls = *c;
            self.gain = output_gain(c.volume, c.main_output)
                * if c.lock { lock_trim(&c.panel) } else { 1.0 }
                * unison_trim_of(c)
                * double_trim(c)
                * level_gain(c.level);
            self.follow_auto(auto_trim(c, &self.curve), false);
            if eager {
                // (The controls set from outside: the gain at once, as ever.)
                self.gain_left = 0;
                self.apply();
            } else {
                // (Each voice takes it as it next plays: those not playing, never.)
                self.panel_gen = self.panel_gen.wrapping_add(1);
            }
        }
    }

    /// The LFOs a step on (decisions.md R-LFO): each one's value at the samples about to be
    /// played, `n` of them, the knobs they modulate moved there, and each voice's own where MONO
    /// is off; the generators moved on by `n`. Never wakes the sleeping instrument (R68): asleep,
    /// the generators run on and nothing more.
    fn modulate(&mut self, n: usize) {
        let rate = self.lfo_rate();
        // (The velocity on to the key the instruments sound now: R-VELOCITY.)
        if self.base.lfo.velocity_active() {
            let share = 1.0 - (-(n as f64) / (rate * EXPRESSION_GLIDE)).exp();
            self.velocity.glide(share);
            for s in &mut self.slots {
                s.velocity.glide(share);
            }
        }
        let l = &self.base.lfo;
        let voices = if self.many() { self.slots.len() } else { 1 };
        // (A pitch or CUTOFF an LFO moves: where the next step will have the knobs, which the
        // voices glide to across this one's samples, unless a SHARP edge in it holds it: an
        // LFO's value halfway across off the line between its ends, R-LFO-STRESS.)
        let glides = n > 1 && l.fast() && modulation_glides() && !self.rest.asleep;
        let mut mid = [0.0; crate::lfo::LFOS];
        let mut mid_voices = [[0.0; crate::lfo::LFOS]; crate::lfo::VOICES];
        if l.cross() {
            // (A source moves an LFO's own knob: the generators on their ticks, fixed in time,
            // each value for the sound read between them: R-LFO's addendum.)
            let clocks = self.lfo_clocks(rate);
            self.bank.cross_to(l, &clocks, self.played, rate, voices);
            self.bank
                .peek(l, &clocks, self.played, rate, self.lfo_dt, voices);
            if glides {
                // (Halfway and at the step's end, on a copy of the generators: the ticks
                // between run and read as the next step will, the knobs they move and TEMPO's
                // timed LFOs with them.)
                let a = &mut *self.ahead;
                for k in [n / 2, n] {
                    a.bank.clone_from(&self.bank);
                    let at = self.played + k as u64;
                    a.bank.cross_to(l, &clocks, at, rate, voices);
                    a.bank.peek(l, &clocks, at, rate, k as f64 / rate, voices);
                    if k == n {
                        (a.shared, a.voices) = (a.bank.shared_values, a.bank.voice_values);
                    } else {
                        (mid, mid_voices) = (a.bank.shared_values, a.bank.voice_values);
                    }
                }
            }
        } else {
            self.bank.cross.on = false;
            let (per, clock) = crate::lfo::Bank::clock(l, &self.position, self.since, rate);
            self.bank.step(l, &clock, self.lfo_dt, voices);
            if glides {
                use crate::lfo::Bank;
                let at = |k: usize| Bank::clock(l, &self.position, self.since + k as f64, rate).1;
                let (b, a) = (&self.bank, &mut *self.ahead);
                mid = b.ahead(l, &per, &at(n / 2), n / 2, rate, voices, &mut mid_voices);
                a.shared = b.ahead(l, &per, &at(n), n, rate, voices, &mut a.voices);
            }
            self.bank.advance(l, &per, n, rate, voices);
        }
        self.lfo_dt = n as f64 / rate;
        // The effects' pins' offsets at this step, the sources' shared values (an effect has
        // no voices), asleep or not: an effect's tail plays on (R-FX-PARAMS).
        let moved = l.fx_moved();
        if moved != 0 {
            let values = l.source_values(&self.bank.shared_values, self.velocity.now);
            let at = self.played.saturating_sub(self.block_from) as u32;
            self.fx.push(moved, at, l.fx_offsets(&values));
        }
        if glides {
            use crate::lfo::{JUMP, LFOS};
            let (b, a) = (&self.bank, &mut *self.ahead);
            let off = |x: f64, m: f64, y: f64| (m - 0.5 * (x + y)).abs() > JUMP;
            a.jumps = (0..LFOS).any(|k| off(b.shared_values[k], mid[k], a.shared[k]));
            for (v, jumps) in a.voice_jumps.iter_mut().enumerate().take(voices) {
                *jumps = (0..LFOS).any(|k| {
                    !l.lfo[k].mono && off(b.voice_values[v][k], mid_voices[v][k], a.voices[v][k])
                });
            }
            // (The velocity where the next step will have it, gliding under a phrase: R-VELOCITY.)
            a.velocity = if l.velocity_active() {
                1.0 - (-(n as f64) / (rate * EXPRESSION_GLIDE)).exp()
            } else {
                0.0
            };
            let velocity = self.velocity.ahead(a.velocity);
            let next = l.modulated(&self.base, &l.source_values(&a.shared, velocity));
            let panel = self.panel_of(&next);
            let a = &mut *self.ahead;
            a.panel = panel;
            a.len = (n * self.over.as_ref().map_or(1, |o| o.factor)) as f64;
        }
        self.ahead.on = false;
        if self.rest.asleep {
            return;
        }
        let values = l.source_values(&self.bank.shared_values, self.velocity.now);
        let m = l.modulated(&self.base, &values);
        if m != self.controls {
            let (was, sharp) = (self.gain_at(), l.sharp_on(crate::lfo::Target::Volume));
            self.take(&m, false);
            // (A step of the output's gain made a ramp across the step's samples: EDGES SOFT's
            // 2 ms slew on VOLUME is then smooth, not a few clicks; SHARP's alone stay steps,
            // R-LFO-STRESS.)
            if self.gain != was && n > 1 && !sharp && modulation_gain_ramps() {
                (self.gain_now, self.gain_left) = (was, n);
                self.gain_ramp = (self.gain - was) / n as f64;
            } else {
                self.gain_left = 0;
            }
        } else if l.per_voice() || glides {
            self.panel_gen = self.panel_gen.wrapping_add(1);
        }
        // (Every voice takes its glide as it next plays: the panel's new generation.)
        self.ahead.on = glides;
    }

    /// Each LFO's present reading, for the editor's scopes (decisions.md R-LFO).
    pub fn lfo_readings(&self) -> [crate::lfo::Reading; crate::lfo::LFOS] {
        self.bank.readings(&self.base.lfo)
    }

    /// The rate the LFOs keep time by.
    fn lfo_rate(&self) -> f64 {
        if self.rate > 0.0 { self.rate } else { 48_000.0 }
    }

    /// The LFOs' clocks for the present block, on their ticks' time.
    fn lfo_clocks(&self, rate: f64) -> crate::lfo::Clocks {
        crate::lfo::Clocks::of(
            &self.base.lfo,
            &self.position,
            self.block_from as f64 / rate,
            rate,
        )
    }

    /// The LFOs' ticks run up to now, where they run on them (R-LFO's addendum): before a note
    /// starts them, and at a run's end, so that what follows takes them from the time it is.
    fn lfo_catch_up(&mut self) {
        if !self.bank.cross.on || !self.base.lfo.cross() {
            return;
        }
        let rate = self.lfo_rate();
        let clocks = self.lfo_clocks(rate);
        let voices = if self.many() { self.slots.len() } else { 1 };
        self.bank
            .cross_to(&self.base.lfo, &clocks, self.played, rate, voices);
    }

    /// The LFOs' ticks, for the tests (R-LFO's addendum).
    #[cfg(test)]
    pub fn lfo_cross(&self) -> &crate::lfo::Cross {
        &self.bank.cross
    }

    /// The one instrument's key pressed or released (POLY off), on the first voice (once it
    /// is free, if a worker late from an earlier run holds it); with UNISON, on each of its
    /// instruments.
    fn one_note(&mut self, key: i32, on: bool) {
        for k in 0..self.instruments().max(1) {
            let s = &mut self.slots[k];
            if with_voice(s, |p| p.note(key, on)).is_none() && s.pending < s.notes.len() {
                s.notes[s.pending] = (key, on);
                s.pending += 1;
            }
            if self.controls.unison && on {
                s.active = true;
            }
        }
    }

    /// Whether LO shares its voices with the workers now (decisions.md R-LO-WORKERS): always,
    /// never, or while the host's thread is busy, as this system has it ([`lo_sharing`]).
    pub fn lo_shares(&self) -> bool {
        match lo_sharing() {
            LoSharing::Never => false,
            LoSharing::Always => true,
            LoSharing::Load => self.busy.on,
        }
    }

    /// Whether more than the one instrument plays: POLY's voices or UNISON's.
    fn many(&self) -> bool {
        self.controls.poly || self.controls.unison
    }

    /// What the instance asks threads for (decisions.md R-THREADS): POLY's or UNISON's voices
    /// to share, and at ULTRA its voices to split where this machine splits any, the one
    /// instrument's too (so it holds threads with POLY off, R21 as changed there); none else.
    fn need(&self) -> Option<Need> {
        let split = self.controls.ultra && split_voices() > 0;
        match (self.many(), split) {
            (true, _) => Some(Need {
                workers: true,
                split,
            }),
            (false, true) => Some(Need {
                workers: false,
                split: true,
            }),
            (false, false) => None,
        }
    }

    /// The split crews lent this run (at ULTRA), if any.
    fn crews(&self) -> &[Split] {
        match &self.pool {
            Some(p) if self.controls.ultra => p.splits(),
            _ => &[],
        }
    }

    /// The voices that take every key through their keyboard circuits, as the one instrument
    /// does: UNISON's VOICES, POLY off's one, POLY's none.
    fn instruments(&self) -> usize {
        if self.controls.unison {
            self.voices()
        } else if self.controls.poly {
            0
        } else {
            1.min(self.slots.len())
        }
    }

    pub fn event(&mut self, e: Event) {
        // (Expression alone never wakes the instrument: asleep, no key is down to hear it. A
        // velocity neither: its note does.)
        match e {
            Event::NoteVelocity { key, velocity } => {
                if let Some(v) = self.velocities.get_mut(usize::from(key)) {
                    *v = if velocity.is_finite() {
                        velocity.clamp(0.0, 1.0)
                    } else {
                        Velocity::FULL
                    };
                }
                return;
            }
            Event::NoteExpression { key, expression } => {
                return self.note_expression(key, expression);
            }
            Event::ChannelPressure(v) => {
                self.pressure = if v.is_finite() {
                    v.clamp(0.0, 1.0)
                } else {
                    0.0
                };
                return;
            }
            Event::Timbre(v) => {
                self.timbre = if v.is_finite() {
                    v.clamp(-1.0, 1.0)
                } else {
                    0.0
                };
                return;
            }
            _ => {}
        }
        self.rest.wake();
        match e {
            Event::Note { key, on } => {
                if usize::from(key) >= self.held.len() {
                    return;
                }
                // (A key pressed with none held starts a phrase: its expression at once.)
                let phrase = on && !self.held.contains(&true);
                self.held[usize::from(key)] = on;
                if on {
                    self.presses += 1;
                    self.pressed[usize::from(key)] = self.presses;
                    self.last_velocity = self.velocities[usize::from(key)];
                }
                // (Each LFO's RETRIG resets on every key, independently of the contours.
                // FREE's FADE and shared ENV still follow the main trigger. Bring the
                // generators up to this event before restarting: R-LFO-NOTE-RETRIG.)
                let new = on && (phrase || self.controls.panel.retrigger);
                if on {
                    self.lfo_catch_up();
                    self.bank.start_shared(&self.base.lfo, new);
                }
                if self.slots.is_empty() {
                    return;
                }
                if self.controls.unison || !self.controls.poly {
                    if on {
                        for k in 0..self.instruments() {
                            self.bank.start_voice(&self.base.lfo, k, new);
                        }
                    }
                    self.one_note(i32::from(key), on);
                    self.follow_sounding(phrase);
                } else if on {
                    self.poly_on(key);
                } else {
                    for s in &mut self.slots {
                        if s.gate && s.note == key {
                            s.gate = false;
                        }
                    }
                }
                if on {
                    self.velocity_moved();
                }
            }
            Event::PitchBend(b) => {
                self.bend = b.clamp(-1.0, 1.0);
                self.apply();
            }
            Event::Modulation(m) => {
                self.modulation = m.clamp(0.0, 1.0);
                self.apply();
            }
            Event::ResetControllers => {
                (self.bend, self.modulation) = (0.0, 0.0);
                // (Pressure and bend at rest; timbre, a sound controller, as it was: MIDI's
                // RP-015.)
                self.pressure = 0.0;
                let rest = |e: &mut Expression| (e.pressure, e.bend) = (0.0, 0.0);
                self.keys.iter_mut().for_each(rest);
                self.slots.iter_mut().for_each(|s| rest(&mut s.expression));
                self.apply();
            }
            Event::AllNotesOff => self.release_all(),
            Event::AllSoundOff => self.all_sound_off(),
            Event::NoteVelocity { .. }
            | Event::NoteExpression { .. }
            | Event::ChannelPressure(_)
            | Event::Timbre(_) => {}
        }
    }

    /// Key `key`'s own expression from now (decisions.md R-MPE): POLY's voice playing it while
    /// it is held (one let go keeps what it had), the instruments' while it is the key they
    /// sound, and its next note's.
    fn note_expression(&mut self, key: u8, e: Expression) {
        let Some(k) = self.keys.get_mut(usize::from(key)) else {
            return;
        };
        let finite = |x: f32| if x.is_finite() { x } else { 0.0 };
        *k = Expression {
            pressure: finite(e.pressure).clamp(0.0, 1.0),
            timbre: finite(e.timbre).clamp(-1.0, 1.0),
            bend: finite(e.bend).clamp(-BEND_MOST, BEND_MOST),
        };
        let e = *k;
        if self.instruments() > 0 {
            if self.sounding == Some(key) && self.held[usize::from(key)] {
                for k in 0..self.instruments() {
                    self.slots[k].expression = e;
                }
            }
        } else {
            for s in &mut self.slots {
                if s.gate && s.note == key {
                    s.expression = e;
                }
            }
        }
    }

    /// The key the instruments sound (POLY off, UNISON) as NOTE PRIORITY chooses among those
    /// held, its expression theirs (decisions.md R-MPE): at once for a key pressed with none
    /// held, gliding there from the last as the keys change under a phrase. With none held,
    /// the last one's stays, its release playing out with it.
    fn follow_sounding(&mut self, phrase: bool) {
        let held = || (0..128u8).filter(|&k| self.held[usize::from(k)]);
        let key = match self.controls.panel.priority {
            Priority::Low => held().min(),
            Priority::High => held().max(),
            Priority::Last => held().max_by_key(|&k| self.pressed[usize::from(k)]),
        };
        let Some(key) = key else {
            return;
        };
        let fresh = phrase || self.sounding.is_none();
        let e = self.keys[usize::from(key)];
        let v = self.velocities[usize::from(key)];
        self.sounding = Some(key);
        self.velocity.take(v, fresh);
        for k in 0..self.instruments() {
            let s = &mut self.slots[k];
            s.expression = e;
            s.fresh |= fresh;
            s.velocity.take(v, fresh);
        }
    }

    /// The knobs the velocity moves where a note has just put it (decisions.md R-VELOCITY), so
    /// that the note starts there, not a step later: as [`Engine::modulate`] moves them.
    fn velocity_moved(&mut self) {
        let l = &self.base.lfo;
        if !l.velocity_active() || self.rest.asleep {
            return;
        }
        let values = l.source_values(&self.bank.shared_values, self.velocity.now);
        let m = l.modulated(&self.base, &values);
        if m != self.controls {
            self.take(&m, false);
        } else if l.per_voice() {
            self.panel_gen = self.panel_gen.wrapping_add(1);
        }
    }

    /// The latest note's velocity as it came, 0..1 (the editor's meter: decisions.md
    /// R-VELOCITY), none before a note.
    pub fn last_velocity(&self) -> Option<f32> {
        (self.presses > 0).then_some(self.last_velocity)
    }

    /// Key `key`'s own expression as last told (for the plug-in's tests).
    #[cfg(test)]
    pub(crate) fn key_expression(&self, key: u8) -> Expression {
        self.keys[usize::from(key)]
    }

    /// The MODULATION wheel the voice in slot `k` plays at, if it is sounding (for the plug-in's
    /// tests).
    #[cfg(test)]
    pub(crate) fn voice_wheel(&self, k: usize) -> Option<f64> {
        let s = self.slots.get(k).filter(|s| s.active)?;
        with_voice(s, |p| p.voice.panel.mod_wheel)
    }

    /// The pressure and timbre every voice shares (for the plug-in's tests).
    #[cfg(test)]
    pub(crate) fn shared_expression(&self) -> (f32, f32) {
        (self.pressure, self.timbre)
    }

    /// A voice's expression as slot `k` hands it on: its note's own, its pressure at least the
    /// shared one, its timbre the shared one where that is farther from rest ([`farther`]).
    fn handed(&self, k: usize) -> [f64; 3] {
        let e = self.slots[k].expression;
        [
            f64::from(e.pressure.max(self.pressure)),
            f64::from(farther(e.timbre, self.timbre)),
            f64::from(e.bend),
        ]
    }

    /// All Sound Off (decisions.md R18): every key released and the output faded out over
    /// [`HUSH_FADE`]; then each voice that sounded is put to rest ([`Engine::settle`]) and the
    /// output fades back in. A voice taken for a note meanwhile plays on.
    fn all_sound_off(&mut self) {
        self.release_all();
        let (many, instruments) = (self.many(), self.instruments());
        for (k, s) in self.slots.iter_mut().enumerate() {
            s.rest = if many {
                s.active || k < instruments
            } else {
                k == 0
            };
        }
        self.hush = !self.slots.is_empty();
        self.hush_waited = 0;
    }

    /// Once All Sound Off has faded the output out, the voices it marked put to rest, a clean
    /// voice in each one's place ([`Engine::restore`]), and the output let back in; a voice
    /// whose clean voice is not yet made is waited for, silent, up to [`HUSH_WAIT`]. At the
    /// start of each run (one sample, or the samples between events).
    fn settle(&mut self) {
        if !self.hush || self.fade > 0.0 {
            return;
        }
        let poly = self.many();
        let mut waiting = false;
        for k in 0..self.slots.len() {
            if !self.slots[k].rest {
                continue;
            }
            if poly && self.slots[k].gate {
                self.slots[k].rest = false;
                continue;
            }
            let play = self.slots[k].play.clone();
            let rested = play.try_lock().is_ok_and(|mut p| self.restore(k, &mut p));
            let s = &mut self.slots[k];
            if rested {
                s.rest = false;
                if poly {
                    (s.active, s.peak, s.heard, s.span) = (false, 0.0, 0.0, 0);
                }
            } else {
                waiting = true;
            }
        }
        if !waiting || self.hush_waited as f64 >= HUSH_WAIT * self.rate {
            self.hush = false;
            for s in &mut self.slots {
                s.rest = false;
            }
        }
    }

    /// Voice `k` put back to rest without allocating (decisions.md R18): a clean voice in its
    /// place from its spare ([`Spares`]), its noise seeded, the engine's panel, its key up
    /// (pressed again if its gate is high; POLY off, the keys held pressed again). False if
    /// the spare has not been made again since it was last used. Either way the spares are
    /// then to be mended ([`Engine::take_mending`]).
    fn restore(&mut self, k: usize, p: &mut Playing) -> bool {
        if p.broken {
            self.render_recoveries = self.render_recoveries.saturating_add(1);
        }
        if p.potato {
            // (The light voices are put to rest in place: nothing to make.)
            p.light.rest();
            p.twin.light.rest();
            light_seeded(&mut p.light, self.seed, k);
            light_seeded(&mut p.twin.light, self.seed, k + POLY_VOICES.2);
        } else if !self.restore_voices(k, p) {
            return false;
        }
        p.key.rest();
        p.twin.key.rest();
        p.expressed.fresh = true;
        (p.twin.live, p.twin.quiet, p.twin.pan) = (false, 0, None);
        (p.broken, p.drop) = (false, false);
        let s = &mut self.slots[k];
        (s.lift, s.pending, s.shown) = (false, 0, false);
        // (The panel before the keys: DOUBLE's, which the twin takes keys by.)
        self.voice_panel(k, p, &self.panel());
        if k < self.instruments() {
            for (key, _) in self.held.iter().enumerate().filter(|(_, h)| **h) {
                p.note(key as i32, true);
            }
        }
        true
    }

    /// Place `k`'s circuit voice and twin swapped for clean spares, seeded as made: whether
    /// they were (not if the spares are not clean or not at the voices' rate).
    fn restore_voices(&mut self, k: usize, p: &mut Playing) -> bool {
        self.mend = true;
        let rate = p.voice.rate();
        let Ok(mut spare) = self.spares.places[k].try_lock() else {
            return false;
        };
        if !spare.clean {
            return false;
        }
        let Some(clean) = spare.voice.as_mut().filter(|v| v.rate() == rate) else {
            return false;
        };
        std::mem::swap(&mut p.voice, &mut **clean);
        if let Some(twin) = spare.twin.as_mut().filter(|v| v.rate() == rate) {
            std::mem::swap(&mut p.twin.voice, &mut **twin);
        }
        spare.clean = false;
        drop(spare);
        seeded(&mut p.voice, self.seed, k);
        seeded(&mut p.twin.voice, self.seed, k + POLY_VOICES.2);
        true
    }

    /// A POLY note: the voice that last played the key if it is free or letting go (a chord
    /// struck again stays where it was in the field), else the free voice whose last note began
    /// longest ago (the notes go round the voices, to either side in turn), else the one let
    /// go longest ago, else the one held longest (its key lifted and pressed again: `PolyKey`)
    /// (the CA-74's R27).
    fn poly_on(&mut self, key: u8) {
        let n = self.voices();
        let pick = (0..n)
            .filter(|&k| {
                let s = &self.slots[k];
                s.note == key && s.age > 0 && !s.gate
            })
            .max_by_key(|&k| self.slots[k].age)
            .or_else(|| {
                (0..n)
                    .filter(|&k| !self.slots[k].active)
                    .min_by_key(|&k| self.slots[k].age)
            })
            .or_else(|| {
                (0..n)
                    .filter(|&k| !self.slots[k].gate)
                    .min_by_key(|&k| self.slots[k].age)
            })
            .or_else(|| (0..n).min_by_key(|&k| self.slots[k].age))
            .unwrap_or(0);
        self.counter += 1;
        let e = self.keys[usize::from(key)];
        let v = self.velocities[usize::from(key)];
        self.velocity.take(v, true);
        let s = &mut self.slots[pick];
        s.drop = s.active && s.shown;
        (s.note, s.gate, s.age, s.active, s.peak) = (key, true, self.counter, true, 0.0);
        (s.heard, s.span) = (0.0, 0);
        (s.expression, s.fresh) = (e, true);
        s.velocity.take(v, true);
        // (Its own LFOs start with each note it takes: decisions.md R-LFO.)
        self.bank.start_voice(&self.base.lfo, pick, true);
    }

    /// The voices POLY plays.
    fn voices(&self) -> usize {
        self.controls
            .voices
            .clamp(POLY_VOICES.0, POLY_VOICES.2)
            .min(self.slots.len())
    }

    /// Every held key released (the contours then run out as the DECAY switch has them), and
    /// the one instrument's pattern started again from the first place (decisions.md
    /// R62): the host resets the plug-in, and sends All Notes Off as its transport
    /// stops, so that a part plays and renders its places alike each time.
    pub fn release_all(&mut self) {
        self.rest.wake();
        for key in 0..self.held.len() {
            if std::mem::take(&mut self.held[key]) && self.instruments() > 0 {
                self.one_note(key as i32, false);
            }
        }
        for s in &mut self.slots {
            s.gate = false;
        }
        self.restart = true;
    }

    /// Every voice's key lifted at once and the held keys forgotten (POLY switched).
    fn lift_all(&mut self) {
        for k in 0..self.slots.len() {
            let s = &mut self.slots[k];
            if with_voice(s, Playing::lift).is_none() {
                s.lift = true;
            }
            s.gate = false;
        }
        if self.instruments() > 0 {
            for key in 0..self.held.len() {
                if self.held[key] {
                    self.one_note(key as i32, false);
                }
            }
        }
        self.held = [false; 128];
    }

    /// One output sample (the left and right channels together, as on a mono output) with
    /// `ext` at EXTERNAL INPUT (1.0 the same volts as the output's 1.0).
    pub fn tick(&mut self, ext: f32) -> f32 {
        let (l, r) = self.tick_stereo(ext);
        0.5 * (l + r)
    }

    /// One output sample, left and right, with `ext` at EXTERNAL INPUT (a sample that is not
    /// finite taken as 0: decisions.md R18); silence before [`Engine::prepare`]. With POWER
    /// off, once faded out, the voices rest (the MIDI input still reaches them). Held back as
    /// [`Engine::render`] holds it ([`LINE_UP`]).
    pub fn tick_stereo(&mut self, ext: f32) -> (f32, f32) {
        self.cursor = self.since;
        self.since += 1.0;
        self.played += 1;
        let (l, r) = self.voices_stereo(ext);
        let (mut l, mut r) = ([l], [r]);
        let late = self.lined_up();
        self.line_up.hold(late, &mut l, &mut r);
        (l[0], r[0])
    }

    /// [`Engine::tick_stereo`]'s sample before it is held back.
    fn voices_stereo(&mut self, ext: f32) -> (f32, f32) {
        if self.rest.asleep {
            if self.dormant(&[ext]) {
                return (0.0, 0.0);
            }
            self.rest.wake();
        }
        self.settle();
        let on = self.controls.power;
        if (!on && self.fade == 0.0) || self.slots.is_empty() {
            return (0.0, 0.0);
        }
        let ext = if ext.is_finite() { ext } else { 0.0 };
        let rate = self.voice_rate();
        let (l, r) = match self.over.take() {
            None => self.sample(f64::from(ext), rate),
            Some(mut o) => {
                let mut ins = [0.0; 8];
                o.up.push(f64::from(ext), &mut ins[..o.factor]);
                let (mut l, mut r) = (0.0, 0.0);
                let at = self.cursor;
                for (j, x) in ins[..o.factor].iter().enumerate() {
                    // (Each of the voices' samples at its own place on the clock.)
                    self.cursor = at + j as f64 / o.factor as f64;
                    let (a, b) = self.sample(*x, rate);
                    if let Some(d) = o.down[0].push(a) {
                        l = d;
                    }
                    if let Some(d) = o.down[1].push(b) {
                        r = d;
                    }
                }
                self.cursor = at;
                self.over = Some(o);
                (l, r)
            }
        };
        let (l, r) = self.faded(l, r);
        if !self.many() {
            self.rest.heard(&[l], &[r]);
        }
        (l, r)
    }

    /// `left.len()` output samples, left and right, with `ext[i]` at EXTERNAL INPUT (an empty
    /// `ext`: nothing there; a sample that is not finite, 0) and no event among them: the same
    /// samples, to the bit, as as many [`Engine::tick_stereo`]. With POLY the voices play
    /// [`CHUNK`] samples at a time, one after another (a voice's state, some 30 KB, then stays
    /// in the processor's nearest cache), or shared with the workers, and are summed in their
    /// order (decisions.md R11). The one instrument asleep, and nothing at EXTERNAL INPUT:
    /// silence, as as many samples would give, without them (decisions.md R68).
    pub fn render(&mut self, ext: &[f32], left: &mut [f32], right: &mut [f32]) {
        if !self.base.lfo.active() {
            self.bank.cross.on = false;
            return self.render_part(ext, left, right);
        }
        // (The LFOs modulating: a step every `lfo::STEP` samples, decisions.md R-LFO; a pitch
        // or CUTOFF gliding across each, R-LFO-STRESS.)
        let len = left.len().min(right.len());
        let step = modulation_step();
        let mut i = 0;
        while i < len {
            let n = (len - i).min(step);
            self.modulate(n);
            let x = &ext[i.min(ext.len())..(i + n).min(ext.len())];
            self.render_part(x, &mut left[i..i + n], &mut right[i..i + n]);
            i += n;
        }
        self.lfo_catch_up();
    }

    /// [`Engine::render`]'s samples, with the controls as they are.
    fn render_part(&mut self, ext: &[f32], left: &mut [f32], right: &mut [f32]) {
        let len = left.len().min(right.len());
        let from = self.since;
        self.since += len as f64;
        self.played += len as u64;
        if self.rest.asleep {
            if self.dormant(&ext[..ext.len().min(len)]) {
                left[..len].fill(0.0);
                right[..len].fill(0.0);
                // (What the output still holds back comes out, as with the voices playing.)
                let late = self.lined_up();
                self.line_up.hold(late, &mut left[..len], &mut right[..len]);
                return;
            }
            self.rest.wake();
        }
        self.render_voices(ext, &mut left[..len], &mut right[..len], from);
        let late = self.lined_up();
        self.line_up.hold(late, &mut left[..len], &mut right[..len]);
        if !self.many() {
            self.rest.heard(&left[..len], &right[..len]);
        }
    }

    /// The plug-in's latency, host samples (decisions.md R-LATENCY): how late ULTRA's sound
    /// comes out, the voices' ([`ca72::voice::latency`]) and, below [`MIN_VOICE_RATE`], the
    /// engine's resamplers'; rounded. The same in every QUALITY mode (HI's and LO's sound
    /// held back to come out with ULTRA's: [`LINE_UP`]), so a host's compensation never
    /// changes with it.
    pub fn latency(&self) -> u32 {
        let voices = ca72::voice::latency(Quality::HighFidelity);
        let late = match &self.over {
            Some(o) => voices / o.factor as f64 + o.up.delay() + o.down[0].delay(),
            None => voices,
        };
        late.round() as u32
    }

    /// How many host samples the output is held back in the QUALITY mode playing
    /// ([`LINE_UP`]).
    fn lined_up(&self) -> usize {
        let at_voices = if self.controls.ultra {
            0
        } else if self.controls.potato {
            LINE_UP.1
        } else {
            LINE_UP.0
        };
        let factor = self.over.as_ref().map_or(1, |o| o.factor);
        (at_voices as f64 / factor as f64).round() as usize
    }

    /// Whether the one instrument, asleep, stays so for samples with `ext` at EXTERNAL INPUT:
    /// POWER on and faded in, no All Sound Off fading, and nothing there.
    fn dormant(&self, ext: &[f32]) -> bool {
        !self.many()
            && self.controls.power
            && !self.hush
            && self.fade == 1.0
            && ext
                .iter()
                .all(|x| !x.is_finite() || f64::from(x.abs()) < SILENT)
    }

    /// [`Engine::render`]'s samples, played, the first `from` host samples after the block's
    /// start.
    fn render_voices(&mut self, ext: &[f32], left: &mut [f32], right: &mut [f32], from: f64) {
        let len = left.len().min(right.len());
        let at = |i: usize| ext.get(i).copied().filter(|x| x.is_finite()).unwrap_or(0.0);
        // (At QUALITY's LO the one instrument too plays a run at a time: its light voice costs
        // too little for a lock and a place worked out each sample to be nothing beside it.)
        // (At ULTRA with a split crew, too: a run is the crew's pass.)
        let runs = self.many() || self.controls.potato || !self.crews().is_empty();
        if !runs || self.over.is_some() || self.slots.is_empty() {
            for i in 0..len {
                self.cursor = from + i as f64;
                (left[i], right[i]) = self.voices_stereo(at(i));
            }
            return;
        }
        let rate = self.voice_rate();
        let mut mix = self.mix();
        let mut start = 0;
        while start < len {
            self.settle();
            // (The oscillators timed: the run's first sample's places on the clock.)
            mix.timed = self.timed_at(from + start as f64);
            let mut n = (len - start).min(CHUNK);
            // The samples the voices play: all of them, unless POWER is off, when they rest
            // from the sample the fade reaches 0. All Sound Off's run ends at that sample, its
            // voices put to rest from the next, as a sample at a time has them.
            let mut run = n;
            if !self.controls.power || self.hush {
                let (mut fade, mut out) = (self.fade, 0);
                while out < n && fade != 0.0 {
                    fade = self.fade_on(fade);
                    out += 1;
                }
                if self.hush && self.fade > 0.0 {
                    n = out;
                }
                run = if self.controls.power { n } else { out };
            }
            let mut ins = [0.0f64; CHUNK];
            for (k, x) in ins.iter_mut().enumerate().take(run) {
                *x = f64::from(at(start + k));
            }
            let (mut l, mut r) = ([0.0f64; CHUNK], [0.0f64; CHUNK]);
            if run > 0 {
                if self.many() {
                    self.run(run, &ins, rate, &mix, true, &mut l, &mut r);
                } else {
                    self.run_one(run, &ins, rate, &mix, &mut l, &mut r);
                }
            }
            for k in 0..n {
                (left[start + k], right[start + k]) = if k < run {
                    self.faded(l[k], r[k])
                } else {
                    (0.0, 0.0)
                };
            }
            start += n;
        }
    }

    /// The one instrument's voice (POLY and UNISON off) over `n` samples, as as many samples
    /// of [`Engine::tick_stereo`] play it, its lock taken once: silent for the run if a worker
    /// late from an earlier run still holds it.
    fn run_one(
        &mut self,
        n: usize,
        ext: &[f64; CHUNK],
        rate: f64,
        mix: &Mix,
        l: &mut [f64; CHUNK],
        r: &mut [f64; CHUNK],
    ) {
        let play = self.slots[0].play.clone();
        let Ok(mut p) = play.try_lock() else {
            return;
        };
        self.bring_up(0, &mut p);
        p.split_now = self.crews().first().is_some_and(|c| lend(&mut p, c));
        p.play_from(&ext[..n], rate, mix, self.one_glide(mix));
        self.turn = p.turn.unwrap_or(0);
        l[..n].copy_from_slice(&p.out[0][..n]);
        r[..n].copy_from_slice(&p.out[1][..n]);
    }

    /// The one instrument's place's glide a sample: [`TURN_GLIDE`]'s while it scatters (its
    /// knobs' moves too), else [`GLIDE`]'s.
    fn one_glide(&self, mix: &Mix) -> f64 {
        if mix.scatter {
            self.turn_glide
        } else {
            self.glide
        }
    }

    /// POLY's sounding voices over `n` samples, summed into `l` and `r` in their order: each
    /// given its gate, note and drop and brought up to the engine's panel, then played here,
    /// or shared with the workers (with `share`, and a run long enough to be worth it). A
    /// voice a worker late from an earlier run still holds sits the run out.
    #[allow(clippy::too_many_arguments)]
    fn run(
        &mut self,
        n: usize,
        ext: &[f64; CHUNK],
        rate: f64,
        mix: &Mix,
        share: bool,
        l: &mut [f64; CHUNK],
        r: &mut [f64; CHUNK],
    ) {
        let mut list = [0usize; MAX];
        let mut count = 0;
        for k in 0..self.slots.len() {
            if !self.slots[k].active || count == MAX {
                continue;
            }
            let play = self.slots[k].play.clone();
            let Ok(mut p) = play.try_lock() else {
                self.slots[k].peak = self.slots[k].peak.max(SILENT);
                self.silent_runs += 1;
                continue;
            };
            self.bring_up(k, &mut p);
            self.slots[k].hand(&mut p);
            drop(p);
            list[count] = k;
            count += 1;
        }
        // At ULTRA, every sounding voice split if there are crews for all of them, each its own
        // (splitting only some gains nothing: decisions.md R-THREADS); else every one whole.
        let crews = self.crews();
        let split = share && count > 0 && crews.len() >= count;
        for (j, &k) in list[..count].iter().enumerate() {
            if let Ok(mut p) = self.slots[k].play.try_lock() {
                p.split_now = split && lend(&mut p, &crews[j]);
            }
        }
        let deadline = self.deadline;
        // (At QUALITY's LO only as many threads as the run's light voices repay waking:
        // decisions.md R-LO-WORKERS. On macOS no more than take as many turns; at ULTRA every one,
        // its voices often past the deadline: R-MAC-512.)
        let threads = if self.controls.ultra {
            EVERY
        } else if !self.controls.potato {
            MAX
        } else if self.lo_shares() {
            lo_threads(count)
        } else {
            1
        };
        // (Two voices' filters solved together only at HI without DOUBLE: decisions.md
        // R-HI-PAIRS, R-POLY-CRACKLE.)
        let pairs = !self.controls.ultra && !self.controls.potato && mix.double <= 0.0;
        let played = match &mut self.pool {
            Some(pool) if share && threads > 1 && count > 1 && n >= SHARED_RUN => {
                pool.play(&list[..count], n, ext, rate, mix, pairs, threads, deadline)
            }
            _ => {
                // (Two at a time, their filters solved together at HI: decisions.md
                // R-HI-PAIRS. Each writes its own samples; they are summed in order below.)
                let mut played = [false; MAX];
                let mut i = 0;
                while i < count {
                    let a = self.slots[list[i]].play.try_lock();
                    let b = (i + 1 < count).then(|| self.slots[list[i + 1]].play.try_lock());
                    match (a, b) {
                        (Ok(mut a), Some(Ok(mut b))) => {
                            Playing::play_pair(&mut a, &mut b, n, ext, rate, mix);
                            (played[i], played[i + 1]) = (true, true);
                        }
                        (a, b) => {
                            if let Ok(mut a) = a {
                                a.play(n, ext, rate, mix);
                                played[i] = true;
                            }
                            if let Some(Ok(mut b)) = b {
                                b.play(n, ext, rate, mix);
                                played[i + 1] = true;
                            }
                        }
                    }
                    i += 2;
                }
                played
            }
        };
        for (&ok, &k) in played.iter().zip(&list[..count]) {
            self.silent_runs += u64::from(!ok);
            let s = &mut self.slots[k];
            s.ran(ok);
            // (A voice not heard this run is not known to have fallen silent: it is not freed
            // for it at the block's end. decisions.md R18.)
            let p = if ok { s.play.try_lock().ok() } else { None };
            let Some(p) = p else {
                s.peak = s.peak.max(SILENT);
                continue;
            };
            for i in 0..n {
                l[i] += p.out[0][i];
                r[i] += p.out[1][i];
            }
            s.peak = s.peak.max(p.peak);
        }
    }

    /// A voice brought up to what the engine asked of it meanwhile: a clean voice in its place
    /// if it broke ([`Engine::restore`]), the panel, its key lifted and the one instrument's
    /// keys.
    fn bring_up(&mut self, k: usize, p: &mut Playing) {
        if p.broken {
            self.restore(k, p);
        }
        // (The panel before the keys: DOUBLE's, which the twin takes keys by.)
        if p.panel_gen != self.panel_gen {
            self.voice_panel(k, p, &self.panel());
        }
        if k == 0 && std::mem::take(&mut self.restart) {
            p.turn = None;
        }
        p.expressed.to = self.handed(k);
        let s = &mut self.slots[k];
        p.expressed.fresh |= std::mem::take(&mut s.fresh);
        if std::mem::take(&mut s.lift) {
            p.lift();
        }
        for &(key, on) in &s.notes[..s.pending] {
            p.note(key, on);
        }
        s.pending = 0;
    }

    /// Whether a detached render encountered a recovered voice, sanitised sample, or
    /// a voice still broken. Called only with offline work stopped; a held lock also fails.
    pub fn render_faulted(&self) -> bool {
        self.render_recoveries != 0
            || self.render_sanitised != 0
            || self
                .slots
                .iter()
                .any(|s| s.play.try_lock().map_or(true, |p| p.broken))
    }

    /// A sample of the voices' sum, left and right, through MAIN OUTPUT's gain and POWER's
    /// or All Sound Off's fade (moved a sample on). Never a sample that is not finite
    /// (decisions.md R18): 0 instead, and the resamplers, which may hold one, start again.
    fn faded(&mut self, l: f64, r: f64) -> (f32, f32) {
        self.fade = self.fade_on(self.fade);
        if self.hush && self.fade == 0.0 {
            self.hush_waited += 1;
        }
        // (As the one voice was: the gain, then the fade; AUTO GAIN's correction, gliding,
        // between them, 1 without DRIVE.)
        self.auto_now += (self.auto - self.auto_now) * self.auto_glide;
        let gain = if self.gain_left > 0 {
            self.gain_left -= 1;
            self.gain_now += self.gain_ramp;
            if self.gain_left == 0 {
                self.gain
            } else {
                self.gain_now
            }
        } else {
            self.gain
        };
        let (l, r) = (
            (l * gain * self.auto_now * self.fade) as f32,
            (r * gain * self.auto_now * self.fade) as f32,
        );
        if l.is_finite() && r.is_finite() {
            return (l, r);
        }
        self.render_sanitised = self.render_sanitised.saturating_add(1);
        if let Some(o) = &mut self.over {
            o.up.reset();
            o.down.iter_mut().for_each(Decimator::reset);
        }
        (0.0, 0.0)
    }

    /// The output's gain at the next sample: on its way across a step ([`Engine::faded`]), else
    /// `gain`.
    fn gain_at(&self) -> f64 {
        if self.gain_left > 0 {
            self.gain_now
        } else {
            self.gain
        }
    }

    /// The output's gain under POWER and All Sound Off a sample after `fade`: rising while
    /// POWER is on, else falling (All Sound Off's faster).
    fn fade_on(&self, fade: f64) -> f64 {
        if self.controls.power && !self.hush {
            (fade + self.fade_step).min(1.0)
        } else if self.hush {
            (fade - self.hush_step).max(0.0)
        } else {
            (fade - self.fade_step).max(0.0)
        }
    }

    /// How ENTROPY, SPREAD, DOUBLE and DRIVE stand for the voices' samples, and oscillator 3
    /// timed at the sample the voices are at.
    fn mix(&self) -> Mix {
        Mix {
            timed: self.timed_at(self.cursor),
            ..mix_of(&self.controls, self.voices())
        }
    }

    /// AUTO GAIN's curve for the sound (`drive.rs`), the correction following it: a preset's
    /// or a session's, the sound's at once ([`GLIDE`]).
    pub fn set_curve(&mut self, curve: Curve<STEPS>) {
        self.curve = curve;
        self.follow_auto(auto_trim(&self.controls, &curve), false);
    }

    /// A curve measured for the sound after DRIVE was turned (decisions.md R-AUTO-GAIN-DRIVE):
    /// the correction glides to it over [`REFINE_GLIDE`], so that a refinement arriving while a
    /// note plays moves its level unheard.
    pub fn refine_curve(&mut self, curve: Curve<STEPS>) {
        self.curve = curve;
        self.follow_auto(auto_trim(&self.controls, &curve), true);
    }

    /// The correction's new target, glided to slowly (a measured curve) or as the controls
    /// glide; a target unchanged leaves a glide under way as it is.
    fn follow_auto(&mut self, auto: f64, refine: bool) {
        if auto != self.auto {
            self.auto = auto;
            self.auto_glide = if refine {
                self.auto_refine
            } else {
                self.auto_share
            };
        }
    }

    /// AUTO GAIN's curve the engine plays with.
    pub fn curve(&self) -> Curve<STEPS> {
        self.curve
    }

    /// Where AUTO GAIN's measurements are asked for and kept (`drive.rs`), told the voices'
    /// rate now and whenever the engine is prepared. Not on the audio thread.
    pub fn calibrate_with(&mut self, c: Calibration) {
        if self.rate > 0.0 {
            c.set_rate(self.voice_rate());
        }
        self.calibration = Some(c);
    }

    /// One sample of the voices at their rate (`rate` Hz): left and right.
    fn sample(&mut self, ext: f64, rate: f64) -> (f64, f64) {
        let mix = self.mix();
        if !self.many() {
            let play = self.slots[0].play.clone();
            let Ok(mut p) = play.try_lock() else {
                return (0.0, 0.0);
            };
            self.bring_up(0, &mut p);
            p.split_now = false;
            // (The one voice at its turn's place, or DOUBLE's pair; at SPREAD 0 in the centre,
            // both sides whole. The voice's key is the keyboard's.)
            p.play_from(&[ext], rate, &mix, self.one_glide(&mix));
            self.turn = p.turn.unwrap_or(0);
            return (p.out[0][0], p.out[1][0]);
        }
        let mut ins = [0.0f64; CHUNK];
        ins[0] = ext;
        let (mut l, mut r) = ([0.0f64; CHUNK], [0.0f64; CHUNK]);
        self.run(1, &ins, rate, &mix, false, &mut l, &mut r);
        (l[0], r[0])
    }

    fn voice_rate(&self) -> f64 {
        self.rate * self.over.as_ref().map_or(1, |o| o.factor) as f64
    }

    /// After a block of `len` samples: the OVERLOAD lamp (0 off, 1 fully lit), its peak held
    /// and falling with a time constant of [`LAMP_FALL`]; POLY's voices let go and fallen
    /// silent over the block freed.
    pub fn end_block(&mut self, len: usize) -> f32 {
        // (The block's runs are played: the workers may park as they finish.)
        if let Some(p) = &self.pool {
            p.rest();
        }
        if let Some(t) = self.block_start.take()
            && self.rate > 0.0
            && len > 0
        {
            let period = len as f64 / self.rate;
            let unheard = self.silent_runs > self.block_silent;
            self.busy.tally(t.elapsed().as_secs_f64(), period, unheard);
        }
        if !self.many() && !self.rest.asleep {
            let state = self
                .slots
                .first()
                .and_then(|s| with_voice(s, |p| slow_state(p)));
            self.rest.after(state.flatten(), len);
        }
        let mut peak = 0.0f64;
        // (UNISON's instruments, which take their keys straight, are held while a key is.)
        let held = if self.controls.unison && self.held.iter().any(|h| *h) {
            self.instruments()
        } else {
            0
        };
        for (k, s) in self.slots.iter_mut().enumerate() {
            let overload = |p: &mut Playing| {
                if p.potato {
                    let twin = p.twin.light.take_overload_peak();
                    return p.light.take_overload_peak().max(twin);
                }
                let twin = p.twin.voice.take_overload_peak();
                p.voice.take_overload_peak().max(twin)
            };
            if let Some(p) = with_voice(s, overload) {
                peak = peak.max(p);
            }
            (s.heard, s.span) = (s.heard.max(s.peak), s.span + len);
            if s.span >= SILENT_SPAN {
                if s.active && !s.gate && s.heard < SILENT && k >= held {
                    s.active = false;
                }
                (s.heard, s.span) = (0.0, 0);
            }
            if let Some(l) = self.levels.get_mut(k) {
                *l = if s.peak.is_finite() {
                    s.peak as f32
                } else {
                    0.0
                };
            }
            s.peak = 0.0;
        }
        let fall = (-(len as f32) / (self.rate as f32 * LAMP_FALL)).exp();
        let fall = if fall.is_finite() { fall } else { 0.0 };
        self.lamp = (self.lamp * fall).max(peak.clamp(0.0, 1.0) as f32);
        self.lamp
    }

    pub fn lamp(&self) -> f32 {
        self.lamp
    }

    /// Each voice's output's peak over the last block, by its slot (POLY's and UNISON's voices;
    /// the one instrument's is the first).
    pub fn levels(&self) -> &[f32; LEVELS] {
        &self.levels
    }

    /// Each oscillator's CONTINUOUS shape where the newest sounding voice had it at its last
    /// sample (MOD's orbs: decisions.md R-CONTINUOUS-WAVE), its switch's steps; none while no
    /// voice sounds, for a STEPPED oscillator, or while a worker holds the voice (the last
    /// block's kept by the editor then).
    pub fn shapes(&self) -> Option<[Option<f64>; 3]> {
        if self.sounding_mask() == 0 {
            return Some([None; 3]);
        }
        let k = if self.many() {
            (0..self.slots.len())
                .filter(|&k| self.slots[k].active)
                .max_by_key(|&k| self.slots[k].age)?
        } else {
            0
        };
        let play = self.slots.get(k)?.play.try_lock().ok()?;
        Some(if play.potato {
            play.light.probe_shapes()
        } else {
            play.voice.probe_shapes()
        })
    }

    /// Which voices sound now, a bit a voice (the editor's display of where they sound, A6):
    /// POLY's and UNISON's sounding, the one instrument while a key is held (scattering, the
    /// bit of the place its last attack took).
    pub fn sounding_mask(&self) -> u32 {
        if self.many() {
            self.slots
                .iter()
                .enumerate()
                .filter(|(_, s)| s.active)
                .fold(0, |m, (k, _)| m | 1 << k)
        } else {
            let at = if self.mix().scatter { self.turn } else { 0 };
            u32::from(self.held.iter().any(|h| *h)) << at.min(LEVELS - 1)
        }
    }

    /// Whether the one instrument sleeps: silent and settled, not played until an event, a
    /// control changed or a sound at EXTERNAL INPUT wakes it (decisions.md R68).
    pub fn asleep(&self) -> bool {
        self.rest.asleep
    }

    /// The one instrument never to sleep (or to again): for measuring what it costs waiting.
    pub fn never_sleep(&mut self, never: bool) {
        self.rest.never = never;
        self.rest.wake();
    }

    /// The voices sounding now (POLY or UNISON on), or 1 (off).
    pub fn sounding(&self) -> usize {
        if self.many() {
            self.slots.iter().filter(|s| s.active).count()
        } else {
            1
        }
    }

    /// The wheels where the parameters and the MIDI input leave them: PITCH -1..1 and
    /// MODULATION 0..1.
    pub fn wheels(&self) -> (f64, f64) {
        let p = self.panel();
        (p.pitch_wheel, p.mod_wheel)
    }

    /// Where a MIDI keyboard's wheels move the panel's, as the panel draws them: its pitch
    /// bend's share of the PITCH wheel's travel (0..1, 0.5 at rest) and its modulation wheel
    /// (0..1).
    pub fn midi_wheels(&self) -> (f32, f32) {
        let share = (self.controls.bend_range / PITCH_WHEEL_SEMITONES).clamp(0.0, 1.0);
        let share = if share.is_finite() { share as f32 } else { 0.0 };
        (
            0.5 + self.bend * share / 2.0,
            ca72::modulation::midi_wheel(f64::from(self.modulation)) as f32,
        )
    }

    /// The panel the voices play: the controls', in Potato (at ULTRA, High Fidelity), the
    /// MIDI input's bend moving
    /// the PITCH wheel by its share of the wheel's travel and its modulation wheel holding
    /// MODULATION up to where the hardware reference's MIDI puts it
    /// (`ca72::modulation::midi_wheel`).
    fn panel(&self) -> Panel {
        self.panel_of(&self.controls)
    }

    /// [`Engine::panel`] for the controls `c`.
    fn panel_of(&self, c: &Controls) -> Panel {
        let share = (c.bend_range / PITCH_WHEEL_SEMITONES).clamp(0.0, 1.0);
        let share = if share.is_finite() { share } else { 0.0 };
        let mut p = c.panel;
        p.quality = if c.ultra {
            Quality::HighFidelity
        } else {
            Quality::Potato
        };
        p.pitch_wheel = (p.pitch_wheel + f64::from(self.bend) * share).clamp(-1.0, 1.0);
        p.mod_wheel = p
            .mod_wheel
            .max(ca72::modulation::midi_wheel(f64::from(self.modulation)));
        p
    }

    /// How every voice's preamplifier is solved inside FEEDBACK's loop (for comparing the
    /// models: decisions.md R11).
    pub fn set_in_loop(&mut self, m: ca72::preamp::InLoop) {
        self.in_loop = m;
        self.apply();
    }

    /// The panel to every voice (each with its tolerances under ENTROPY): a new generation,
    /// which a voice a worker still holds takes before it next plays.
    fn apply(&mut self) {
        self.panel_gen = self.panel_gen.wrapping_add(1);
        let p = self.panel();
        for k in 0..self.slots.len() {
            let play = self.slots[k].play.clone();
            if let Ok(mut v) = play.try_lock() {
                self.voice_panel(k, &mut v, &p);
            }
        }
    }

    /// A voice's panel `p` with its tolerances, FEEDBACK and its loop's preamplifier; its
    /// twin's with the twin's tolerances; and whether DOUBLE is on.
    fn voice_panel(&self, k: usize, v: &mut Playing, p: &Panel) {
        let entropy = entropy_depth(self.controls.entropy);
        let mut feedback = self.controls.feedback;
        // (Its own LFOs on the knobs it has of its own, where MONO is off: decisions.md R-LFO.)
        let own;
        let p = if self.base.lfo.per_voice() && k < crate::lfo::VOICES {
            let mut q = *p;
            self.base.lfo.voice(
                &mut q,
                &mut feedback,
                &self
                    .base
                    .lfo
                    .source_values(&self.bank.shared_values, self.velocity.now),
                &self
                    .base
                    .lfo
                    .source_values(&self.bank.voice_values[k], self.slots[k].velocity.now),
            );
            own = q;
            &own
        } else {
            p
        };
        let feedback = feedback.clamp(0.0, 1.0);
        for (voice, character) in [
            (&mut v.voice, &v.character),
            (&mut v.twin.voice, &v.twin.character),
        ] {
            voice.feedback = feedback;
            voice.in_loop = self.in_loop;
            let pv = if entropy > 0.0 {
                entropy_panel(character, *p, entropy)
            } else {
                *p
            };
            if voice.panel != pv {
                voice.panel = pv;
            }
        }
        // (The light voices only at LO: their laws are worked out again as their panel
        // changes, which the circuit's voices have no need of.)
        v.potato = self.controls.potato;
        if v.potato {
            for (light, voice) in [(&mut v.light, &v.voice), (&mut v.twin.light, &v.twin.voice)] {
                light.feedback = feedback;
                light.set_panel(&voice.panel);
            }
        }
        // (Its glide to the next step's knobs, R-LFO-STRESS: ENTROPY's tolerances leave the
        // knobs that glide alone, so the twin's is the voice's.)
        let a = &*self.ahead;
        let jumps = a.jumps || (self.base.lfo.per_voice() && a.voice_jumps.get(k) == Some(&true));
        v.sweep = if a.on && !jumps {
            let mut next = a.panel;
            if self.base.lfo.per_voice() && k < crate::lfo::VOICES {
                let mut feedback = 0.0;
                self.base.lfo.voice(
                    &mut next,
                    &mut feedback,
                    &self
                        .base
                        .lfo
                        .source_values(&a.shared, self.velocity.ahead(a.velocity)),
                    &self
                        .base
                        .lfo
                        .source_values(&a.voices[k], self.slots[k].velocity.ahead(a.velocity)),
                );
            }
            Sweep::between(p, &next, a.len, v.potato)
        } else {
            Sweep::default()
        };
        v.double = self.controls.double > 0.0;
        v.panel_gen = self.panel_gen;
        (v.expressed.wheel, v.expressed.midi) =
            (self.controls.panel.mod_wheel, f64::from(self.modulation));
    }
}

/// How often the LFOs move their knobs, samples: [`crate::lfo::STEP`] (decisions.md R-LFO).
#[cfg(not(feature = "lfo-lab"))]
#[inline(always)]
const fn modulation_step() -> usize {
    crate::lfo::STEP
}

/// A measurement's own step for the LFOs (feature `lfo-lab` only; 0: as built).
#[cfg(feature = "lfo-lab")]
static MODULATION_STEP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(feature = "lfo-lab")]
fn modulation_step() -> usize {
    match MODULATION_STEP.load(std::sync::atomic::Ordering::Relaxed) {
        0 => crate::lfo::STEP,
        n => n,
    }
}

/// The LFOs moving their knobs every `samples` (a sample: the reference the 32-sample step is
/// measured against, decisions.md R-LFO-STRESS), every instance in the process; 0 puts back
/// the built steps. A measurement's only: the feature `lfo-lab`.
#[cfg(feature = "lfo-lab")]
pub fn set_modulation_step_for_measurement(samples: usize) {
    MODULATION_STEP.store(samples, std::sync::atomic::Ordering::Relaxed);
}

/// The behaviour before R-LFO-STRESS: 32-sample steps, no output-gain ramp and no glide. An
/// explicit step still wins, so that the per-sample reference is the same on either side.
#[cfg(feature = "lfo-lab")]
static MODULATION_LEGACY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Measure the behaviour at 1374b0f, before both LFO fixes. Process-wide, for serial
/// measurements only; absent from a plug-in build.
#[cfg(feature = "lfo-lab")]
pub fn set_modulation_legacy_for_measurement(legacy: bool) {
    MODULATION_LEGACY.store(legacy, std::sync::atomic::Ordering::Relaxed);
}

/// Whether the voices glide a pitch or CUTOFF an LFO moves across each step (feature `lfo-lab`
/// only; true: as built).
#[cfg(feature = "lfo-lab")]
static MODULATION_GLIDE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Measure the LFOs with the voices' glide across each step (as built) or without it (each step
/// held, as before R-LFO-STRESS; the output-gain ramp kept). Process-wide, for serial
/// measurements only; absent from a plug-in build.
#[cfg(feature = "lfo-lab")]
pub fn set_modulation_glide_for_measurement(glide: bool) {
    MODULATION_GLIDE.store(glide, std::sync::atomic::Ordering::Relaxed);
}

#[inline(always)]
fn modulation_glides() -> bool {
    #[cfg(feature = "lfo-lab")]
    {
        use std::sync::atomic::Ordering::Relaxed;
        MODULATION_GLIDE.load(Relaxed) && !MODULATION_LEGACY.load(Relaxed)
    }
    #[cfg(not(feature = "lfo-lab"))]
    {
        true
    }
}

#[inline(always)]
fn modulation_gain_ramps() -> bool {
    #[cfg(feature = "lfo-lab")]
    {
        !MODULATION_LEGACY.load(std::sync::atomic::Ordering::Relaxed)
    }
    #[cfg(not(feature = "lfo-lab"))]
    {
        true
    }
}

/// Today's count, the least the process's budget holds (decisions.md R11, R39, R-THREADS):
/// [`workers_for`] this machine's hardware threads.
pub fn default_workers() -> usize {
    workers_for(ca72_rt::cores().all)
}

/// [`Engine::start_workers`]'s ask for as many workers as this machine gives an instance (the
/// plug-in's, decisions.md R-THREADS).
pub const AUTO: usize = usize::MAX;

/// A measurement's own size of the process's budget of threads (0: none, the plug-in's way:
/// [`budget_workers`]'s rule). Only the benches set it.
static BUDGET_OVERRIDE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// A measurement's or a test's own count of ULTRA's split voices (`usize::MAX`: none,
/// [`split_voices_for`] this machine's fast cores).
static SPLIT_OVERRIDE: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(usize::MAX);

/// The process's budget of threads made audio threads ([`crate::pool`]) while `instances` of
/// the plug-in's hold threads (decisions.md R-THREADS, approved 2026-10-16): this machine's
/// hardware threads less two (the system's and the host's) and less one for each instance's
/// host thread, and never fewer than today's count ([`default_workers`]); on macOS, too,
/// every instance at least one worker and two over ([`budget_for_macos`]); a measurement's
/// own if it has set one ([`set_budget_for_measurement`]).
pub fn budget_workers(instances: usize) -> usize {
    match BUDGET_OVERRIDE.load(std::sync::atomic::Ordering::Relaxed) {
        0 if cfg!(target_os = "macos") => budget_for_macos(ca72_rt::cores().all, instances),
        0 => budget_for(ca72_rt::cores().all, instances),
        n => n,
    }
}

/// [`budget_workers`] on a machine of `all` hardware threads (Linux and Windows).
pub fn budget_for(all: usize, instances: usize) -> usize {
    workers_for(all).max(all.saturating_sub(2 + instances))
}

/// [`budget_workers`] on a Mac of `all` hardware threads (decisions.md R-THREADS, the macOS
/// rule approved 2026-10-16): [`budget_for`]'s, or the instances and two over if more, never
/// more than the hardware threads less two. macOS demotes a time-constraint thread that runs
/// far past its computation, so the margin for the instances' host threads is not needed there
/// as on Linux at `SCHED_FIFO`; with eight instances on the Mac, every instance given a worker
/// made a quarter of the blocks late that four workers did (the others none, late every block).
pub fn budget_for_macos(all: usize, instances: usize) -> usize {
    budget_for(all, instances).max((instances + 2).min(all.saturating_sub(2)))
}

/// The most workers an instance's pool holds (decisions.md R-THREADS): this machine's fast
/// cores less two (so that one instance's voices stay on them: on a hybrid processor, workers
/// past its performance cores made one instance slower), never fewer than today's count;
/// none fewer than its share while a measurement sets the budget.
pub fn instance_cap() -> usize {
    if BUDGET_OVERRIDE.load(std::sync::atomic::Ordering::Relaxed) != 0 {
        return usize::MAX;
    }
    cap_for(ca72_rt::cores())
}

/// [`instance_cap`] on a machine of these cores.
pub fn cap_for(c: ca72_rt::Cores) -> usize {
    workers_for(c.all).max(c.fast.saturating_sub(2))
}

/// How many of an instance's voices ULTRA plays split, each on six threads (decisions.md
/// R-THREADS): [`split_voices_for`] this machine's fast cores, or a measurement's own.
pub fn split_voices() -> usize {
    match SPLIT_OVERRIDE.load(std::sync::atomic::Ordering::Relaxed) {
        usize::MAX => split_voices_for(ca72_rt::cores().fast),
        n => n,
    }
}

/// ULTRA's split voices on a machine of `fast` fast cores: none below 6 (on 4 cores or fewer
/// ULTRA stays as it was, the owner's rule; 5 was not measured), one from 6 to 9, two from 10.
/// Three were not clean where measured, so not more.
pub fn split_voices_for(fast: usize) -> usize {
    match fast {
        0..=5 => 0,
        6..=9 => 1,
        _ => 2,
    }
}

/// What a lone instance with POLY on holds at HI: its workers.
pub fn workers_alone() -> usize {
    budget_workers(1).min(instance_cap()).min(MAX - 1)
}

/// Sets the process's budget of threads for a measurement (0: back to [`budget_workers`]'s
/// rule), an instance's pool then bounded by its share alone. Not for the plug-in: the benches'
/// way to try more workers than this machine is given.
#[doc(hidden)]
pub fn set_budget_for_measurement(workers: usize) {
    BUDGET_OVERRIDE.store(workers, std::sync::atomic::Ordering::Relaxed);
}

/// Sets how many voices ULTRA splits, for a measurement or a test (None: back to this
/// machine's, [`split_voices_for`]). Not for the plug-in.
#[doc(hidden)]
pub fn set_split_for_measurement(voices: Option<usize>) {
    SPLIT_OVERRIDE.store(
        voices.unwrap_or(usize::MAX),
        std::sync::atomic::Ordering::Relaxed,
    );
}

/// The workers for a machine of `n` processors before R-THREADS (decisions.md R11, R39), now
/// the least the process's budget holds ([`budget_workers`]): a third of them less two, at most
/// four (four on the Mac's 14, two on 8), and at least one from 3 up, so that on a machine of 3
/// or 4 (an older quad-core) POLY's voices are not all on the host's thread while its other
/// processors idle; none on 1 or 2, where the host's thread and a worker would be all the
/// machine has.
pub fn workers_for(n: usize) -> usize {
    if n < 3 {
        0
    } else {
        (n.saturating_sub(2) / 3).clamp(1, 4)
    }
}

/// How LO shares its voices with the workers on a system (decisions.md R-LO-WORKERS).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoSharing {
    /// Every voice on the host's thread, as before R-LO-WORKERS (0.2.8).
    Never,
    /// Whenever [`lo_threads`] gives more than one thread.
    Always,
    /// Only while the host's thread is busy ([`Busy`]): below that, as [`LoSharing::Never`].
    Load,
}

/// A measurement's own [`LoSharing`] (0: this system's).
static LO_SHARING_OVERRIDE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// How LO shares its voices on this system (decisions.md R-LO-WORKERS, the owner's decision
/// of 2026-10-19): by load on Windows, where the workers' short runs cost two to four times the
/// voices' own processor time and the host's thread gained little while it was not busy; by
/// load on macOS too (R-MAC-LO-LOAD): in CoreAudio's own IO thread a light LO load waiting for
/// the workers made a block late about once a minute under background load, where the IO
/// thread playing every voice itself, as 0.2.8 does, made none; always on Linux, where waking a
/// worker costs little. A measurement's own, if it has set one.
pub fn lo_sharing() -> LoSharing {
    match LO_SHARING_OVERRIDE.load(std::sync::atomic::Ordering::Relaxed) {
        1 => LoSharing::Never,
        2 => LoSharing::Always,
        3 => LoSharing::Load,
        _ if cfg!(any(windows, target_os = "macos")) => LoSharing::Load,
        _ => LoSharing::Always,
    }
}

/// Sets how LO shares its voices, for a measurement or a test (None: back to this system's,
/// [`lo_sharing`]). Not for the plug-in.
#[doc(hidden)]
pub fn set_lo_sharing_for_measurement(sharing: Option<LoSharing>) {
    let v = match sharing {
        None => 0,
        Some(LoSharing::Never) => 1,
        Some(LoSharing::Always) => 2,
        Some(LoSharing::Load) => 3,
    };
    LO_SHARING_OVERRIDE.store(v, std::sync::atomic::Ordering::Relaxed);
}

/// The host's thread's load, its time in the plug-in a block against the block's period,
/// smoothed, and whether it is busy enough for LO to share its voices ([`LoSharing::Load`],
/// decisions.md R-LO-WORKERS): busy from a smoothed load above [`Busy::ON`], or at once after a
/// block late or one with a voice's run not heard; no longer once the load has stayed below
/// [`Busy::OFF`] for the hold, which starts at [`Busy::HOLD`] and doubles (to [`Busy::MOST`])
/// each time it is busy again within [`Busy::AGAIN`] of ceasing, so that it does not flap (the
/// load falls when the voices are shared); back to [`Busy::HOLD`] after [`Busy::AGAIN`] not
/// busy. Seconds throughout; plain arithmetic, for the audio thread.
#[derive(Debug, Clone, Copy)]
pub struct Busy {
    load: f64,
    on: bool,
    /// How long the load has been below [`Busy::OFF`] while busy, the hold now, and how long
    /// since it was last busy.
    quiet: f64,
    hold: f64,
    since: f64,
}

impl Default for Busy {
    fn default() -> Self {
        Busy {
            load: 0.0,
            on: false,
            quiet: 0.0,
            hold: Busy::HOLD,
            since: f64::INFINITY,
        }
    }
}

impl Busy {
    /// The smoothed load above which LO shares: half the block's period.
    pub const ON: f64 = 0.5;
    /// The smoothed load below which it stops, after the hold: well below [`Busy::ON`], and
    /// above the 24 to 28 % a light load's sharing left on Windows, where it hardly lowers the
    /// host's thread's time (so that a light load that went busy for one late block returns to
    /// the host's thread).
    pub const OFF: f64 = 0.3;
    /// The smoothing's time constant.
    pub const SMOOTH: f64 = 0.2;
    /// The first hold, the most, and how soon being busy again doubles it.
    pub const HOLD: f64 = 2.0;
    pub const MOST: f64 = 30.0;
    pub const AGAIN: f64 = 10.0;

    /// A block of `period` seconds that took the host's thread `took`, with or without a
    /// voice's run not heard.
    pub fn tally(&mut self, took: f64, period: f64, unheard: bool) {
        if !(took.is_finite() && period > 0.0) {
            return;
        }
        let share = took / period;
        self.load += (period / Busy::SMOOTH).min(1.0) * (share - self.load);
        if share > 1.0 || unheard || self.load > Busy::ON {
            if !self.on {
                self.hold = if self.since < Busy::AGAIN {
                    (2.0 * self.hold).min(Busy::MOST)
                } else {
                    Busy::HOLD
                };
                self.on = true;
            }
            self.quiet = 0.0;
        } else if self.on {
            if self.load < Busy::OFF {
                self.quiet += period;
                if self.quiet >= self.hold {
                    (self.on, self.quiet, self.since) = (false, 0.0, 0.0);
                }
            } else {
                self.quiet = 0.0;
            }
        } else {
            self.since += period;
        }
    }

    /// Whether it is busy.
    pub fn on(&self) -> bool {
        self.on
    }

    /// The smoothed load.
    pub fn load(&self) -> f64 {
        self.load
    }
}

/// LO's sharing for a measurement (`usize::MAX`: [`lo_threads`]'s rule; else at most this many
/// threads, the host's among them, for any run of two voices or more: 1, every voice on the
/// host's thread, as before R-LO-WORKERS).
static LO_THREADS_OVERRIDE: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(usize::MAX);

/// Sets how many threads LO's voices are played on, for a measurement (None: back to
/// [`lo_threads`]'s rule). Not for the plug-in.
#[doc(hidden)]
pub fn set_lo_threads_for_measurement(threads: Option<usize>) {
    LO_THREADS_OVERRIDE.store(
        threads.unwrap_or(usize::MAX),
        std::sync::atomic::Ordering::Relaxed,
    );
}

/// The most threads, the host's among them, a run of `count` of LO's light voices is played
/// on (decisions.md R-LO-WORKERS): a thread a voice up to four, then a thread for each two.
/// Each thread then takes from the host's thread at least what it costs the computer (the
/// host's time falling by at least half the workers' processor time), by the measurements on
/// the Ryzen 7 7800X3D; a thread a voice past four fell by less than a third of it. The host's
/// thread waits for the longest of the threads' shares, so no more threads than make it
/// shorter. 1, the host's alone, for one voice; as many workers as the pool holds at most (the
/// machine's, as at HI). DOUBLE's twins, played with their voices, change neither.
pub fn lo_threads(count: usize) -> usize {
    match LO_THREADS_OVERRIDE.load(std::sync::atomic::Ordering::Relaxed) {
        usize::MAX if count <= 4 => count.max(1),
        usize::MAX => count.div_ceil(2),
        k => k,
    }
}

/// When a block's voices must be done by, as a share of its period from its start: a voice a
/// worker has not finished then is silent for its run (decisions.md R11). Normally they are
/// done long before.
pub const DEADLINE: f64 = 0.75;

/// The shortest run worth sharing with the workers, samples (waking them takes some
/// microseconds).
const SHARED_RUN: usize = 8;

/// Samples [`Engine::render`] plays a voice for at a time.
pub const CHUNK: usize = 128;

/// ENTROPY, SPREAD, DOUBLE and DRIVE as the voices' samples take them: ENTROPY's depth, the
/// oscillators' (their floor with it, unless LOCK), the depth the character's offsets are
/// drawn at, SPREAD and INNER, SCATTER's placement (none for one voice alone with DOUBLE, its
/// pair at the edge) and the voices it places among (VOICES), DOUBLE's detune between a note's
/// two voices, cents (0: one voice a note), DRIVE's gain into the filter (1: the circuit), and
/// whether the one instrument scatters (POLY, UNISON and DOUBLE off: its voice at the next of
/// VOICES's places at each attack; decisions.md R62).
#[derive(Debug, Clone, Copy)]
pub struct Mix {
    entropy: f64,
    oscillators: f64,
    at: f64,
    spread: f64,
    inner: f64,
    placement: Option<Placement>,
    voices: usize,
    double: f64,
    drive: f64,
    scatter: bool,
    /// How far the voices' own pressure and timbre move them (decisions.md R-MPE).
    reach: Reach,
    /// Each oscillator timed (decisions.md R-OSC-TIMED): where its cycle stands at the run's
    /// first sample, and its cycles a sample (the voices'); none unless it is timed at LO.
    timed: [Option<Timed>; 3],
}

/// [`Mix`]'s values, as the workers are handed them.
pub const MIX_LEN: usize = 20;

impl Mix {
    pub fn to_array(self) -> [f64; MIX_LEN] {
        [
            self.entropy,
            self.oscillators,
            self.at,
            self.spread,
            self.inner,
            self.placement.map_or(-1.0, |p| p.index() as f64),
            self.voices as f64,
            self.double,
            self.drive,
            f64::from(u8::from(self.scatter)),
            self.reach.pressure_mod,
            self.reach.pressure_filter,
            self.reach.timbre_mod,
            self.reach.timbre_filter,
            self.timed[0].map_or(-1.0, |t| t.phase),
            self.timed[0].map_or(0.0, |t| t.per),
            self.timed[1].map_or(-1.0, |t| t.phase),
            self.timed[1].map_or(0.0, |t| t.per),
            self.timed[2].map_or(-1.0, |t| t.phase),
            self.timed[2].map_or(0.0, |t| t.per),
        ]
    }

    pub fn from_array(
        [
            entropy,
            oscillators,
            at,
            spread,
            inner,
            placement,
            voices,
            double,
            drive,
            scatter,
            pressure_mod,
            pressure_filter,
            timbre_mod,
            timbre_filter,
            phase1,
            per1,
            phase2,
            per2,
            phase3,
            per3,
        ]: [f64; MIX_LEN],
    ) -> Mix {
        Mix {
            entropy,
            oscillators,
            at,
            spread,
            inner,
            placement: (placement >= 0.0).then(|| Placement::from_index(placement as usize)),
            voices: voices as usize,
            double,
            drive,
            scatter: scatter > 0.0,
            reach: Reach {
                pressure_mod,
                pressure_filter,
                timbre_mod,
                timbre_filter,
            },
            timed: [(phase1, per1), (phase2, per2), (phase3, per3)]
                .map(|(phase, per)| (phase >= 0.0).then_some(Timed { phase, per })),
        }
    }
}

/// A voice's jacks for its next sample, the run's `k`th: `ext` at EXTERNAL INPUT, its
/// character's offsets (a sample on), and each oscillator's place on the clock if it is timed.
fn jacks(c: &mut Character, ext: f64, rate: f64, m: &Mix, k: usize) -> Jacks {
    let mut j = Jacks {
        ext,
        timed: m.timed.map(|t| {
            t.map(|t| {
                let at = t.phase + k as f64 * t.per;
                Timed {
                    phase: at - at.floor(),
                    per: t.per,
                }
            })
        }),
        ..Jacks::default()
    };
    if m.at > 0.0 {
        let (cents, cutoff) = c.offsets(m.at, rate);
        j.detune = cents.map(|c| c * m.oscillators / m.at);
        j.cutoff = cutoff * m.entropy / m.at;
    }
    j
}

/// The share of a block's period the voices may take: the rest is the host's, its other
/// plug-ins' and the machine's own jitter (the DAW's figure).
pub const HEADROOM: f64 = 0.6;

/// How many POLY voices this machine plays in real time at `rate` Hz in blocks of `block`
/// frames with `workers` workers (decisions.md, "POLY in real time"; R11): the engine itself,
/// its workers made audio threads, played paced at the block's period on an audio thread of
/// its own for a moment, as a host plays it: chords of as many notes as voices tried, a new
/// one every few blocks (each taking voices still sounding, as legato), gliding, oscillators
/// 2 and 3 on, oscillator 3 on the filter through the MODULATION wheel, ENTROPY at half. The
/// count is the most voices whose blocks where the keys change take, at their median, no
/// more than [`HEADROOM`] of the period. With `feedback`, FEEDBACK on (at half, into the
/// external input at half VOLUME). About a fifth of a second a count tried, ten first, then
/// the estimate from it downward; once a process for each rate, block, FEEDBACK and number
/// of workers. Not on the audio thread.
pub fn realtime_voices(rate: f64, block: usize, feedback: bool, workers: usize) -> u32 {
    /// A measurement: the rate's bits, the block, FEEDBACK, the workers, the voices.
    type Measured = (u64, usize, bool, usize, u32);
    static MEASURED: std::sync::OnceLock<Mutex<Vec<Measured>>> = std::sync::OnceLock::new();
    let cache = MEASURED.get_or_init(|| Mutex::new(Vec::new()));
    let key = (rate.to_bits(), block, feedback, workers);
    if let Some(n) = cache
        .lock()
        .ok()
        .and_then(|c| c.iter().find(|e| (e.0, e.1, e.2, e.3) == key).map(|e| e.4))
    {
        return n;
    }
    let n = std::thread::scope(|s| {
        std::thread::Builder::new()
            .name("ca72-measure".to_owned())
            .spawn_scoped(s, || measure_voices(rate, block, feedback, workers))
            .ok()
            .and_then(|h| h.join().ok())
            .unwrap_or(0)
    });
    if let Ok(mut c) = cache.lock() {
        c.push((key.0, key.1, key.2, key.3, n));
    }
    n
}

/// [`realtime_voices`]' measurement, on a thread of its own that it makes an audio thread.
fn measure_voices(rate: f64, block: usize, feedback: bool, workers: usize) -> u32 {
    let block = block.max(1);
    let period = Duration::from_secs_f64(block as f64 / rate.max(1.0));
    let mut c = Controls {
        poly: true,
        voices: POLY_VOICES.2,
        entropy: 0.5,
        ..Controls::default()
    };
    c.panel.osc[1].on = true;
    c.panel.osc[2].on = true;
    c.panel.filter_mod = true;
    c.panel.mod_wheel = 0.4;
    c.panel.glide = 0.15;
    c.panel.glide_on = true;
    c.panel.emphasis = 0.5;
    if feedback {
        c.panel.ext_on = true;
        c.panel.ext_volume = 0.5;
        c.feedback = 0.5;
    }
    // (Paced and bounded: a fifth of a second a count.)
    let _ = ca72_rt::promote(period, period / 2);
    let (mut l, mut r) = (vec![0.0f32; block], vec![0.0f32; block]);
    let mut held: Vec<u8> = Vec::with_capacity(POLY_VOICES.2);
    let mut chord = 0usize;
    // The median time of a count's blocks where the keys change, on an engine of its own
    // (no voices of the last count still sounding).
    let mut trial = |voices: usize| -> Duration {
        c.voices = voices;
        let mut e = Engine::new();
        e.set(&c);
        e.prepare(rate, 1);
        e.start_workers(workers, Some(period));
        held.clear();
        let blocks = ((0.2 / period.as_secs_f64()) as usize).clamp(12, 64);
        let every = (blocks / 6).max(2);
        let mut changes = Vec::with_capacity(8);
        let start = Instant::now();
        for i in 0..blocks + every {
            if let Some(wait) = (start + period * i as u32).checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            }
            let t = Instant::now();
            e.set_deadline(Some(t + period.mul_f64(DEADLINE)));
            let change = i % every == 0;
            if change {
                for key in held.drain(..) {
                    e.event(Event::Note { key, on: false });
                }
                const ROOTS: [u8; 6] = [45, 41, 43, 48, 40, 46];
                const SHAPE: [u8; 10] = [0, 7, 12, 16, 19, 24, 28, 31, 34, 36];
                let root = ROOTS[chord % ROOTS.len()];
                chord += 1;
                for d in SHAPE.iter().take(voices) {
                    e.event(Event::Note {
                        key: root + d,
                        on: true,
                    });
                    held.push(root + d);
                }
            }
            e.render(&[], &mut l, &mut r);
            e.end_block(block);
            // (The first chord's blocks warm the voices and the caches up.)
            if change && i >= every {
                changes.push(t.elapsed());
            }
        }
        std::hint::black_box((&l, &r));
        changes.sort();
        changes
            .get(changes.len() / 2)
            .copied()
            .unwrap_or(Duration::MAX)
    };
    let room = period.mul_f64(HEADROOM);
    let most = POLY_VOICES.2;
    let t = trial(most);
    if t <= room {
        return most as u32;
    }
    let mut voices =
        ((most as f64 * room.as_secs_f64() / t.as_secs_f64()).floor() as usize).min(most - 1);
    while voices > 0 {
        if trial(voices) <= room {
            break;
        }
        voices -= 1;
    }
    voices as u32
}

/// UNISON's trim on the output: its VOICES voices, not in phase, add in power, so it is turned
/// down by the square root of VOICES's setting and sounds about as loud as one voice (the
/// CA-74's R25).
fn unison_trim_of(c: &Controls) -> f64 {
    if c.unison {
        unison_trim(c.voices.clamp(POLY_VOICES.0, POLY_VOICES.2))
    } else {
        1.0
    }
}

/// Where voice `k` of seed `s` starts its oscillators on their ramps (decisions.md R45,
/// the CA-74's R25): the first voice at the circuit's initial condition, as the reference
/// starts (so one voice, and every preset with POLY and UNISON off, is the model's to the bit);
/// every other voice where its seed says, as a real instrument's free-running oscillators are
/// wherever they are when a note comes. A voice's three start together, as the reference's do,
/// only the voices apart (the CA-74 starts each oscillator apart: here LOCK's trim, R9, is for
/// three started together, and each voice's would otherwise have its own level at LOCK). Only a
/// voice made or put back to rest starts so: one that has played stops wherever its oscillators
/// were, and the next note finds them there.
fn out_of_step(k: usize, s: u64) -> Option<f64> {
    if k == 0 {
        return None;
    }
    let x = voice_seed(s, k)
        .wrapping_add(3)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15);
    Some(((x ^ (x >> 29)) >> 11) as f64 / (1u64 << 53) as f64)
}

/// Voice `k` of an instance seeded `seed`, at `rate` Hz: its own copy from the prototype (its
/// key lists' room reserved: a key press must not allocate on the audio thread), [`seeded`].
fn made_voice(rate: f64, seed: u64, k: usize) -> Voice {
    let mut voice = Voice::prototype(rate);
    seeded(&mut voice, seed, k);
    voice
}

/// Voice `k`'s light model (QUALITY's LO) of an instance seeded `seed`, at `rate` Hz,
/// [`light_seeded`]. (Made with the voice, off the audio thread; it allocates nothing after.)
fn made_light(rate: f64, seed: u64, k: usize) -> Light {
    let mut light = Light::new(rate);
    light_seeded(&mut light, seed, k);
    light
}

/// A light voice at rest made voice `k` of an instance seeded `seed`, as [`seeded`] makes the
/// circuit's: its noise seeded, its oscillators started where [`out_of_step`] says.
fn light_seeded(light: &mut Light, seed: u64, k: usize) {
    light.set_seed(voice_seed(seed, k));
    if let Some(at) = out_of_step(k, seed) {
        light.start_oscillators_at(at);
    }
}

/// A clean voice made voice `k` of an instance seeded `seed`: its noise seeded, its
/// oscillators started where [`out_of_step`] says.
fn seeded(voice: &mut Voice, seed: u64, k: usize) {
    voice.set_seed(voice_seed(seed, k));
    if let Some(at) = out_of_step(k, seed) {
        voice.start_oscillators_at(at);
    }
}

/// LEVEL's gain at `db` (within [`LEVEL_RANGE`]; not finite, 0 dB). At 0 dB, 1 exactly.
pub fn level_gain(db: f64) -> f64 {
    let db = if db.is_finite() {
        db.clamp(LEVEL_RANGE.0, LEVEL_RANGE.1)
    } else {
        0.0
    };
    10f64.powf(db / 20.0)
}

/// How ENTROPY, SPREAD, DOUBLE and DRIVE stand for the voices' samples under controls `c`,
/// `voices` (VOICES) of them placed.
fn mix_of(c: &Controls, voices: usize) -> Mix {
    let many = c.poly || c.unison;
    // (The one instrument without DOUBLE scatters: its places are VOICES's, one an attack.)
    let scatter = !many && c.double <= 0.0;
    let entropy = entropy_depth(c.entropy);
    // Below 5 % the former floor falls to none; above it the old range, unless LOCK.
    let oscillators = if c.lock {
        0.0
    } else {
        ENTROPY_FLOOR * (c.entropy / ENTROPY_KNEE).clamp(0.0, 1.0) + entropy
    };
    Mix {
        entropy,
        oscillators,
        at: oscillators.max(entropy),
        spread: c.spread,
        inner: c.inner.clamp(0.0, 1.0),
        // (The one instrument with DOUBLE has no place among others: none, and its pair as far
        // out as SPREAD, the kit's `ALONE`.)
        placement: (many || scatter).then_some(c.placement),
        voices,
        double: if c.double > 0.0 {
            c.double.min(1.0) * DOUBLE_CENTS
        } else {
            0.0
        },
        drive: if c.drive > 0.0 {
            10f64.powf(c.drive.min(DRIVE_TOP) / 20.0)
        } else {
            1.0
        },
        scatter,
        reach: c.reach,
        timed: [None; 3],
    }
}

/// AUTO GAIN's correction under controls `c` from the sound's `curve` (decisions.md
/// R45): 1 without DRIVE or with AUTO GAIN off.
fn auto_trim(c: &Controls, curve: &Curve<STEPS>) -> f64 {
    if c.auto_gain && c.drive > 0.0 {
        10f64.powf(curve.at(c.drive, DRIVE_TOP) / 20.0)
    } else {
        1.0
    }
}

/// The seed of AUTO GAIN's measuring voices: their own, not the instance's, so that a sound
/// measures the same in any instance (the CA-74's R29).
const PROBE_SEED: u64 = 0x0CA7_2D21_7E00_0001;

/// The voices a sound's panel plays (ENTROPY's tolerances on each, FEEDBACK), one for each of
/// the notes AUTO GAIN's measurement plays ([`KEYS`]), for measuring DRIVE's loudness off the
/// audio thread (decisions.md R45; `drive.rs`): made as an engine's first voices are, each
/// its own (its parts, its oscillators' start, its noise), POLY's, one a note, in the centre,
/// without the output's gain (MAIN OUTPUT's, LEVEL's, the trims', AUTO GAIN's).
#[derive(Debug)]
pub struct Probe {
    voices: Vec<Playing>,
    controls: Controls,
    rate: f64,
}

impl Probe {
    /// For the sound of controls `c` with the voices at `rate` Hz.
    pub fn new(c: &Controls, rate: f64) -> Probe {
        let mut controls = *c;
        (
            controls.poly,
            controls.unison,
            controls.double,
            controls.spread,
        ) = (true, false, 0.0, 0.0);
        let entropy = entropy_depth(controls.entropy);
        let mut panel = c.panel;
        panel.quality = Quality::Potato;
        let voices = (0..KEYS.len())
            .map(|k| {
                let character = Character::new(voice_seed(PROBE_SEED, k), k, 3, ENTROPY_KNOBS);
                let mut voice = made_voice(rate, PROBE_SEED, k);
                voice.feedback = controls.feedback.clamp(0.0, 1.0);
                voice.panel = if entropy > 0.0 {
                    entropy_panel(&character, panel, entropy)
                } else {
                    panel
                };
                // (At QUALITY's LO the light voices measured, as they play.)
                let mut light = made_light(rate, PROBE_SEED, k);
                if controls.potato {
                    light.feedback = voice.feedback;
                    light.set_panel(&voice.panel);
                }
                Playing {
                    voice,
                    light,
                    potato: controls.potato,
                    character,
                    key: PolyKey::new(rate),
                    gate: false,
                    note: 0,
                    drop: false,
                    out: [[0.0; CHUNK]; 2],
                    peak: 0.0,
                    pan: None,
                    panel_gen: 0,
                    broken: false,
                    twin: made_twin(rate, PROBE_SEED, k),
                    double: false,
                    turn: None,
                    closed: false,
                    split: None,
                    split_now: false,
                    scratch: None,
                    split_runs: 0,
                    expressed: Expressed::default(),
                    sweep: Sweep::default(),
                }
            })
            .collect();
        Probe {
            voices,
            controls,
            rate,
        }
    }

    /// The K-weighted energy of [`KEYS`] played together, held [`HELD`] s and heard for
    /// [`HEARD`] s, at DRIVE `db` (dB): each time from the voices as they were made.
    pub fn energy(&self, db: f64) -> f64 {
        let mut c = self.controls;
        c.drive = db;
        let m = mix_of(&c, KEYS.len());
        let mut voices = self.voices.clone();
        for (v, key) in voices.iter_mut().zip(KEYS) {
            v.note = key;
        }
        let mut k = KWeighted::new(self.rate);
        let held = (HELD * self.rate) as usize;
        let heard = (HEARD * self.rate) as usize;
        let ext = [0.0; CHUNK];
        let share = glide_share(self.rate);
        let mut at = 0;
        while at < heard {
            let until = if at < held { held } else { heard };
            let n = CHUNK.min(until - at);
            for v in &mut voices {
                v.gate = at < held;
                v.play_from(&ext[..n], self.rate, &m, share);
            }
            for i in 0..n {
                k.push(voices.iter().map(|v| v.out[0][i]).sum());
            }
            at += n;
        }
        k.energy
    }
}

/// DOUBLE's trim on the output: a note's two voices, each its own, add in power, so it is
/// turned down by two's square root and sounds about as loud as one voice a note (the CA-74's
/// R28).
fn double_trim(c: &Controls) -> f64 {
    if c.double > 0.0 { DOUBLE_TRIM } else { 1.0 }
}

/// Voice `k`'s noise seed from the instance's: the first voice's the instance's own.
fn voice_seed(seed: u64, k: usize) -> u64 {
    if k == 0 {
        seed
    } else {
        seed ^ ((k as u64 + 1) << 48)
    }
}

/// The panel with voice `c`'s tolerances, by `amount` (0..1) of ENTROPY.
fn entropy_panel(c: &Character, mut p: Panel, amount: f64) -> Panel {
    let k = |i: usize, x: f64| c.knob(i, x, amount);
    p.emphasis = k(0, p.emphasis);
    p.filter_contour.attack = k(1, p.filter_contour.attack);
    p.filter_contour.decay = k(2, p.filter_contour.decay);
    p.loudness_contour.attack = k(3, p.loudness_contour.attack);
    p.loudness_contour.decay = k(4, p.loudness_contour.decay);
    p.glide = k(5, p.glide);
    p
}

/// MAIN OUTPUT VOLUME: R20 (5K, audio taper) from the output into the jack's 10K load, and
/// its switch; `volume` 0..1 of the knob's travel.
pub fn output_gain(volume: f64, on: bool) -> f64 {
    if !on {
        return 0.0;
    }
    let t = audio_taper(volume);
    let (top, bottom) = (5e3 * (1.0 - t), 5e3 * t);
    let load = bottom * 10e3 / (bottom + 10e3);
    if load <= 0.0 {
        0.0
    } else {
        load / (top + load)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::Budget;
    use ca72::voice::Priority;

    /// The workers are handed each timed oscillator's place with the run's other values
    /// (decisions.md R-OSC-TIMED), and none where it is not timed.
    #[test]
    fn the_workers_mix_carries_the_oscillators_timed() {
        let mut m = mix_of(&Controls::default(), 4);
        assert_eq!(Mix::from_array(m.to_array()).timed, [None; 3]);
        for n in 0..3 {
            m.timed[n] = Some(Timed {
                phase: 0.123 + 0.1 * n as f64,
                per: 1e-4 * (n + 1) as f64,
            });
            assert_eq!(Mix::from_array(m.to_array()).timed, m.timed);
        }
    }

    #[test]
    fn entropy_fades_the_old_floor_to_zero_below_five_percent() {
        for amount in [0.0, 0.01, 0.025, 0.05] {
            let m = mix_of(
                &Controls {
                    entropy: amount,
                    ..Controls::default()
                },
                4,
            );
            assert_eq!(m.entropy, 0.0);
            assert!((m.oscillators - ENTROPY_FLOOR * amount / 0.05).abs() < 1e-15);
            let mut c = Character::new(7, 0, 3, ENTROPY_KNOBS);
            for _ in 0..1024 {
                let j = jacks(&mut c, 0.0, 48_000.0, &m, 0);
                assert_eq!(j.cutoff, 0.0);
                if amount == 0.0 {
                    assert_eq!(j.detune, [0.0; 3]);
                }
            }
        }
        for amount in [0.0, 0.05, 0.5, 1.0] {
            let m = mix_of(
                &Controls {
                    entropy: amount,
                    lock: true,
                    ..Controls::default()
                },
                4,
            );
            let mut c = Character::new(9, 0, 3, ENTROPY_KNOBS);
            for _ in 0..1024 {
                assert_eq!(jacks(&mut c, 0.0, 48_000.0, &m, 0).detune, [0.0; 3]);
            }
        }
    }

    /// Compare the engine's output with the old mix calculated independently, including
    /// the panel's tolerances, RNG progression and both quality paths. Other voice controls
    /// take their usual path; both engines render on the caller without a deadline.
    #[test]
    fn translated_entropy_renders_identically_to_the_legacy_mix() {
        for potato in [false, true] {
            for old in [0.0, 20.0, 40.0, 100.0] {
                for lock in [false, true] {
                    let c = Controls {
                        entropy: crate::compat::entropy_percent(old) / 100.0,
                        potato,
                        lock,
                        poly: true,
                        voices: 4,
                        spread: 0.5,
                        ..Controls::default()
                    };
                    let mut actual = Engine::new();
                    actual.set(&c);
                    actual.prepare(48_000.0, 1);
                    let mut reference = Engine::new();
                    reference.set(&c);
                    reference.prepare(48_000.0, 1);
                    let depth = old / 100.0 * 2.0;
                    let oscillators = if lock { 0.0 } else { 0.75 + depth };
                    let mut legacy = reference.mix();
                    legacy.entropy = depth;
                    legacy.oscillators = oscillators;
                    legacy.at = oscillators.max(depth);
                    for slot in &reference.slots {
                        let mut playing = slot.play.lock().unwrap();
                        let panel = entropy_panel(&playing.character, reference.panel(), depth);
                        playing.voice.panel = panel;
                        playing.light.set_panel(&panel);
                        let panel =
                            entropy_panel(&playing.twin.character, reference.panel(), depth);
                        playing.twin.voice.panel = panel;
                        playing.twin.light.set_panel(&panel);
                    }
                    for key in [45, 52, 57, 64] {
                        actual.event(Event::Note { key, on: true });
                        reference.event(Event::Note { key, on: true });
                    }
                    let mut heard = false;
                    for _ in 0..40 {
                        let (mut l, mut r) = ([0.0f32; CHUNK], [0.0f32; CHUNK]);
                        actual.render(&[], &mut l, &mut r);
                        reference.settle();
                        let (mut a, mut b) = ([0.0; CHUNK], [0.0; CHUNK]);
                        reference.run(
                            CHUNK,
                            &[0.0; CHUNK],
                            48_000.0,
                            &legacy,
                            true,
                            &mut a,
                            &mut b,
                        );
                        // (The legacy mix's output held back as the engine's is, to come
                        // out with ULTRA's: decisions.md R-LATENCY.)
                        let (mut fa, mut fb) = ([0.0f32; CHUNK], [0.0f32; CHUNK]);
                        for i in 0..CHUNK {
                            (fa[i], fb[i]) = reference.faded(a[i], b[i]);
                        }
                        let late = reference.lined_up();
                        reference.line_up.hold(late, &mut fa, &mut fb);
                        for i in 0..CHUNK {
                            let (a, b) = (fa[i], fb[i]);
                            assert_eq!(
                                (l[i].to_bits(), r[i].to_bits()),
                                (a.to_bits(), b.to_bits()),
                                "old {old}, LO {potato}, LOCK {lock}, sample {i}"
                            );
                            heard |= l[i] != 0.0 || r[i] != 0.0;
                        }
                        actual.end_block(CHUNK);
                        reference.end_block(CHUNK);
                    }
                    assert!(heard);
                }
            }
        }
    }

    /// Reset-to-reset frequency removes the sampling grid's apparent period jitter. With
    /// modulation at rest there is no held-note pitch movement at 0 %, unlocked too.
    #[test]
    fn zero_entropy_has_steady_pitch_in_every_oscillator() {
        for seed in [1, 7] {
            let mut c = Controls {
                entropy: 0.0,
                ..Controls::default()
            };
            for osc in &mut c.panel.osc {
                osc.on = true;
            }
            let mut e = Engine::new();
            e.set(&c);
            e.prepare(48_000.0, seed);
            e.event(Event::Note { key: 69, on: true });
            for _ in 0..24_000 {
                e.tick(0.0);
            }
            let resets = |e: &Engine| {
                let p = e.slots[0].play.lock().unwrap();
                std::array::from_fn::<_, 3, _>(|n| p.voice.probe_resets(n))
            };
            let mut previous = resets(&e);
            let mut first = [0.0; 3];
            for window in 0..8 {
                for _ in 0..12_000 {
                    e.tick(0.0);
                }
                let now = resets(&e);
                for n in 0..3 {
                    let frequency =
                        (now[n].1 - previous[n].1) as f64 / (now[n].0 - previous[n].0) * 48_000.0;
                    assert!(frequency.is_finite() && frequency > 400.0);
                    if window == 0 {
                        first[n] = frequency;
                    }
                    let cents = 1200.0 * (frequency / first[n]).log2();
                    assert!(
                        cents.abs() < 0.01,
                        "osc {}, seed {seed}, window {window}: {cents} cents",
                        n + 1
                    );
                }
                previous = now;
            }
        }
    }

    /// Issue 57: inspect both contours themselves, as well as the CLAP host's audio. A
    /// separated key must start and hold every instrument's contours; overlapping keys keep
    /// them held, whether RETRIGGER starts a fresh attack or ordinary legato carries on.
    #[test]
    fn separated_and_overlapping_keys_start_and_hold_both_contours() {
        for (potato, ultra) in [(true, false), (false, false), (false, true)] {
            for mode in 0..3 {
                for priority in [Priority::Low, Priority::Last, Priority::High] {
                    for retrigger in [false, true] {
                        let mut c = Controls {
                            potato,
                            ultra,
                            poly: mode == 1,
                            unison: mode == 2,
                            voices: 2,
                            lock: true,
                            ..Controls::default()
                        };
                        c.panel.priority = priority;
                        c.panel.retrigger = retrigger;
                        c.panel.decay = false;
                        let mut e = Engine::new();
                        e.set(&c);
                        e.prepare(48_000.0, 51826);
                        for key in [60, 64, 60] {
                            e.event(Event::Note { key, on: true });
                            for _ in 0..40 {
                                let (mut l, mut r) = ([0.0; 256], [0.0; 256]);
                                e.render(&[], &mut l, &mut r);
                                e.end_block(256);
                            }
                            let check = |e: &Engine, keys: &[u8]| {
                                let mut count = 0;
                                for (k, s) in e.slots.iter().enumerate() {
                                    if !(k < e.instruments() || mode == 1 && s.gate) {
                                        continue;
                                    }
                                    if mode == 1 {
                                        assert!(
                                            keys.contains(&s.note),
                                            "unexpected POLY key {}",
                                            s.note
                                        );
                                    }
                                    let p = s.play.lock().unwrap();
                                    let (f, l) = if potato {
                                        p.light.contours()
                                    } else {
                                        let (f, l, _) = p.voice.envelopes();
                                        (f, l)
                                    };
                                    assert!(
                                        f > 0.3 && l > 0.3,
                                        "LO {potato}, ULTRA {ultra}, mode {mode}, {priority:?}, retrigger {retrigger}, key {key}: {f}, {l} V"
                                    );
                                    count += 1;
                                }
                                let expected = if mode == 1 {
                                    keys.len()
                                } else {
                                    e.instruments()
                                };
                                assert_eq!(count, expected, "mode {mode}: held voices");
                            };
                            check(&e, &[key]);
                            e.event(Event::Note { key: 67, on: true });
                            let (mut l, mut r) = ([0.0; 2048], [0.0; 2048]);
                            e.render(&[], &mut l, &mut r);
                            e.end_block(2048);
                            check(&e, &[key, 67]);
                            if key == 64 {
                                e.event(Event::AllNotesOff);
                            } else {
                                e.event(Event::Note { key: 67, on: false });
                                e.render(&[], &mut l, &mut r);
                                e.end_block(2048);
                                check(&e, &[key]);
                                e.event(Event::Note { key, on: false });
                            }
                            for _ in 0..5 {
                                e.render(&[], &mut l, &mut r);
                                e.end_block(2048);
                            }
                            assert!(e.held.iter().all(|&held| !held));
                            for s in &e.slots {
                                let p = s.play.lock().unwrap();
                                let (f, l) = if potato {
                                    p.light.contours()
                                } else {
                                    let (f, l, _) = p.voice.envelopes();
                                    (f, l)
                                };
                                assert!(
                                    f < 0.3 && l < 0.3,
                                    "contours did not release: {f}, {l} V, LO {potato}, ULTRA {ultra}, mode {mode}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    /// A voice's output's peak as the engine reports it, playing the panel as it opens and
    /// louder: printed, for the editor's level scale (`-- --ignored --nocapture`).
    #[test]
    #[ignore = "prints levels"]
    fn voice_levels() {
        for (name, c) in [
            ("as it opens", Controls::default()),
            (
                "POLY, four voices",
                Controls {
                    poly: true,
                    voices: 4,
                    ..Controls::default()
                },
            ),
        ] {
            let mut e = Engine::new();
            e.set(&c);
            e.prepare(48_000.0, 1);
            for key in [45, 52, 57, 64] {
                e.event(Event::Note { key, on: true });
            }
            let (mut l, mut r) = ([0.0f32; 256], [0.0f32; 256]);
            let mut most = [0.0f32; LEVELS];
            let mut out = 0.0f32;
            for _ in 0..200 {
                e.render(&[], &mut l, &mut r);
                e.end_block(256);
                for (m, v) in most.iter_mut().zip(e.levels()) {
                    *m = m.max(*v);
                }
                out = l.iter().chain(r.iter()).fold(out, |m, v| m.max(v.abs()));
            }
            println!("{name}: voices' peaks {most:?}, the output's {out}");
        }
    }

    /// The workers [`Engine::start_workers`] starts for `asked` without a period on this
    /// machine: as many as asked, within an instance's cap (decisions.md R-THREADS: one on CI's
    /// three-core Mac, two on its Windows and Linux machines) and the pool's most.
    fn started(asked: usize) -> usize {
        asked.min(instance_cap()).min(MAX - 1)
    }

    fn four_voices(workers: usize) -> Engine {
        let mut e = Engine::new();
        e.set(&Controls {
            poly: true,
            voices: 4,
            ..Controls::default()
        });
        e.prepare(48_000.0, 1);
        assert_eq!(e.start_workers(workers, None), started(workers));
        for key in [45, 52, 57, 64] {
            e.event(Event::Note { key, on: true });
        }
        let (mut l, mut r) = ([0.0f32; 256], [0.0f32; 256]);
        e.render(&[], &mut l, &mut r);
        e.end_block(256);
        e
    }

    /// The workers by the machine's processors (decisions.md R11, R39): none on 1 or 2, one
    /// from 3 (a quad-core's 4 among them: before R39, none below 5), then a third of them
    /// less two, at most four.
    #[test]
    fn a_machine_of_three_processors_or_more_has_a_worker() {
        let by: Vec<usize> = (1..=20).map(workers_for).collect();
        assert_eq!(
            by,
            [0, 0, 1, 1, 1, 1, 1, 2, 2, 2, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4]
        );
        assert_eq!(workers_for(0), 0);
        assert_eq!(workers_for(256), 4);
    }

    /// The count by the machine (decisions.md R-THREADS) on the three measured: the budget the
    /// hardware threads less two and less the instances, never below today's; an instance's
    /// cap the fast cores less two, never below today's; ULTRA's split voices by the fast
    /// cores, none on 4 cores or fewer.
    #[test]
    fn the_count_follows_the_machine() {
        use ca72_rt::Cores;
        // The M4 Pro (10 fast of 14), the i5-13600K (6 of 20), the 7800X3D (16 of 16).
        let mac = Cores { all: 14, fast: 10 };
        let windows = Cores { all: 20, fast: 6 };
        let linux = Cores { all: 16, fast: 16 };
        let budget =
            |c: Cores| -> Vec<usize> { [1, 2, 4, 8].map(|a| budget_for(c.all, a)).to_vec() };
        // (The Mac's own rule: every instance a worker and two over, at most N - 2.)
        let macos = [1, 2, 4, 6, 8, 10, 12, 16].map(|a| budget_for_macos(mac.all, a));
        assert_eq!(macos, [11, 10, 8, 8, 10, 12, 12, 12]);
        assert_eq!(budget_for_macos(4, 8), 2);
        assert_eq!(budget(windows), [17, 16, 14, 10]);
        assert_eq!(budget(linux), [13, 12, 10, 6]);
        assert_eq!([mac, windows, linux].map(cap_for), [8, 4, 14]);
        assert_eq!(
            [mac, windows, linux].map(|c| split_voices_for(c.fast)),
            [2, 1, 2]
        );
        // Today's count the least: a machine of 4 (CI's) keeps its one worker (its budget,
        // below its cap of 2) and splits none.
        let quad = Cores { all: 4, fast: 4 };
        assert_eq!(
            (budget_for(4, 1), budget_for(4, 8), cap_for(quad)),
            (1, 1, 2)
        );
        assert_eq!((0..=5).map(split_voices_for).max(), Some(0));
        assert_eq!((split_voices_for(9), split_voices_for(64)), (1, 2));
    }

    /// The host's thread's load (decisions.md R-LO-WORKERS): a light load leaves LO on the
    /// host's thread; half the period, smoothed, or a late block or a run not heard, shares;
    /// sharing stops only once the load has stayed below 30 % for the hold, which grows when it
    /// would flap.
    #[test]
    fn lo_shares_while_the_host_s_thread_is_busy() {
        let p = 256.0 / 48_000.0;
        let run = |b: &mut Busy, share: f64, seconds: f64| {
            for _ in 0..(seconds / p) as usize {
                b.tally(share * p, p, false);
            }
        };
        let mut b = Busy::default();
        run(&mut b, 0.45, 5.0);
        assert!(!b.on(), "busy at 45 %");
        run(&mut b, 0.6, 1.0);
        assert!(b.on(), "not busy at 60 %");
        // Shared, the load falls: busy still between the two, however long.
        run(&mut b, 0.35, 10.0);
        assert!(b.on());
        // Below a quarter: busy for the hold, then not.
        run(&mut b, 0.1, 1.9);
        assert!(b.on(), "stopped before the hold");
        run(&mut b, 0.1, 0.3);
        assert!(!b.on(), "busy after the hold");
        // A late block, or a run not heard: busy at once.
        let mut late = Busy::default();
        late.tally(1.1 * p, p, false);
        assert!(late.on());
        let mut unheard = Busy::default();
        unheard.tally(0.1 * p, p, true);
        assert!(unheard.on());
        // Busy again soon: the hold doubles, so it does not flap; it is back to 2 s after a while.
        run(&mut b, 0.6, 1.0);
        assert!(b.on());
        run(&mut b, 0.1, 3.0);
        assert!(b.on(), "the hold did not grow");
        run(&mut b, 0.1, 1.5);
        assert!(!b.on());
        run(&mut b, 0.1, 11.0);
        run(&mut b, 0.6, 1.0);
        run(&mut b, 0.1, 2.3);
        assert!(!b.on(), "the hold did not come back to 2 s");
        // A block that cannot be timed is left out.
        let before = b.load();
        b.tally(f64::NAN, p, false);
        b.tally(0.5, 0.0, false);
        assert_eq!(b.load().to_bits(), before.to_bits());
    }

    /// LO's voices on as many threads as repay waking (decisions.md R-LO-WORKERS): one a voice
    /// up to four, then one for each two; the host's alone for one.
    #[test]
    fn lo_s_threads_follow_its_voices() {
        let t: Vec<usize> = (0..=10).map(lo_threads).collect();
        assert_eq!(t, [1, 1, 2, 3, 4, 3, 3, 4, 4, 5, 5]);
    }

    /// A voice that a worker late from an earlier run still holds sits the next runs out:
    /// the block is not held up for it (it is still held when the block is done), the other
    /// voices play, and it plays again once let go. (Bounded by what happens, not by time: on
    /// a loaded machine any thread of the test may be held up for milliseconds.)
    #[test]
    fn a_voice_still_held_sits_out_and_nothing_waits_for_it() {
        let mut e = four_voices(2);
        let held = e.slots[1].play.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let (done, until) = std::sync::mpsc::channel::<()>();
        let late = std::thread::spawn(move || {
            let g = held.lock();
            tx.send(()).unwrap();
            let _ = until.recv_timeout(Duration::from_secs(30));
            drop(g);
        });
        rx.recv().unwrap();
        let (mut l, mut r) = ([0.0f32; 256], [0.0f32; 256]);
        // (No deadline: the voices the workers take are waited for, however late they are.)
        e.set_deadline(None);
        e.render(&[], &mut l, &mut r);
        assert!(!late.is_finished(), "the block waited for the held voice");
        assert!(l.iter().any(|x| x.abs() > 0.0), "the other voices silent");
        done.send(()).unwrap();
        late.join().unwrap();
        e.render(&[], &mut l, &mut r);
        assert!(e.slots[1].play.try_lock().is_ok());
    }

    /// A worker that has taken a voice and does not finish it by the deadline: the audio
    /// thread waits no longer than the deadline (it returns while the voice is still held, far
    /// past it), and that voice is silent for the run. The deadline is generous so that a
    /// loaded machine holding a worker up does not silence the others too.
    #[test]
    fn a_worker_late_past_the_deadline_is_not_waited_for() {
        let e = four_voices(1);
        let voices: Vec<Shared> = e.slots.iter().map(|s| s.play.clone()).collect();
        let mut pool = Pool::start(1, &voices, None).unwrap();
        let held = voices[0].clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let (done, until) = std::sync::mpsc::channel::<()>();
        let stall = std::thread::spawn(move || {
            let g = held.lock();
            tx.send(()).unwrap();
            let _ = until.recv_timeout(Duration::from_secs(30));
            drop(g);
        });
        rx.recv().unwrap();
        let mix = e.mix();
        let played = pool.play(
            &[0, 1, 2, 3],
            64,
            &[0.0; CHUNK],
            48_000.0,
            &mix,
            true,
            MAX,
            Some(Instant::now() + Duration::from_secs(1)),
        );
        assert!(!stall.is_finished(), "waited for the held voice");
        assert!(!played[0], "the held voice played");
        assert!(played[1..4].iter().all(|&p| p), "{played:?}");
        done.send(()).unwrap();
        stall.join().unwrap();
    }

    /// A voice let go that sits a run out (a worker late from an earlier run holds it) is not
    /// freed for it at the block's end: its tail is not known to have ended (R18).
    #[test]
    fn a_voice_that_misses_a_run_is_not_freed_for_it() {
        let mut e = four_voices(0);
        let (mut l, mut r) = ([0.0f32; 256], [0.0f32; 256]);
        for _ in 0..40 {
            e.render(&[], &mut l, &mut r);
            e.end_block(256);
        }
        for key in [45, 52, 57, 64] {
            e.event(Event::Note { key, on: false });
        }
        let held = e.slots[1].play.clone();
        let g = held.lock().unwrap();
        e.render(&[], &mut l, &mut r);
        e.end_block(256);
        drop(g);
        assert!(e.slots.iter().take(4).all(|s| s.active && !s.gate));
    }

    /// A retrigger's drop handed to a voice for a run it did not play is kept for its next run,
    /// not overwritten there, and played once; the gate it shows, not known to have played the
    /// run, keeps a new note on it retriggering (R18).
    #[test]
    fn a_drop_for_a_run_not_played_is_kept_for_the_next() {
        let mut e = four_voices(0);
        let play = e.slots[0].play.clone();
        let mut p = play.lock().unwrap();
        let s = &mut e.slots[0];
        (s.drop, s.shown) = (true, true);
        s.hand(&mut p);
        s.ran(false);
        assert!(!s.drop && p.drop);
        s.hand(&mut p);
        assert!(p.drop, "the drop lost");
        p.play(1, &[0.0; CHUNK], 48_000.0, &Mix::from_array([0.0; MIX_LEN]));
        assert!(!p.drop);
        s.gate = false;
        s.ran(false);
        assert!(s.shown, "a gate the voice may not have seen");
        s.ran(true);
        assert!(!s.shown);
    }

    /// A voice whose output goes non-finite (its preamplifier's states poisoned, as one such
    /// sample at EXTERNAL INPUT left them) is put back to rest: the output stays finite and
    /// the held key sounds again; and again once the spares are mended (R18). POLY off and on.
    #[test]
    fn a_voice_gone_non_finite_is_put_back_to_rest() {
        for poly in [false, true] {
            let mut c = Controls {
                poly,
                ..Controls::default()
            };
            c.panel.ext_on = true;
            c.panel.ext_volume = 1.0;
            let mut e = Engine::new();
            e.set(&c);
            e.prepare(48_000.0, 1);
            e.event(Event::Note { key: 57, on: true });
            let (mut l, mut r) = ([0.0f32; 256], [0.0f32; 256]);
            for round in 0..3 {
                if let Ok(mut p) = e.slots[0].play.lock() {
                    p.voice.tick_jacks(&ca72::voice::Jacks {
                        ext: f64::NAN,
                        ..ca72::voice::Jacks::default()
                    });
                }
                let mut last = 0.0f32;
                for _ in 0..40 {
                    e.render(&[], &mut l, &mut r);
                    e.end_block(256);
                    assert!(l.iter().chain(&r).all(|x| x.is_finite()), "poly {poly}");
                    last = l.iter().fold(0.0, |m, x| m.max(x.abs()));
                }
                assert!(e.take_mending(), "poly {poly}, round {round}: not restored");
                assert!(last > 1e-2, "poly {poly}, round {round}: silent ({last})");
                e.spares().mend();
            }
        }
    }

    /// Workers running for the voices, as many and for the same period, are kept when the
    /// engine is prepared again at the same rate (a host's state loaded into a running
    /// instance), and started once for voices built at a new rate (R18); a state with POLY off
    /// stops them (R21).
    #[test]
    fn workers_running_for_the_voices_are_kept() {
        let mut e = Engine::new();
        let mut c = Controls {
            poly: true,
            ..Controls::default()
        };
        e.set(&c);
        e.prepare(48_000.0, 1);
        assert_eq!(e.start_workers(2, None), started(2));
        e.prepare(48_000.0, 2);
        assert_eq!(e.start_workers(2, None), started(2));
        assert_eq!(e.pool_starts, 1, "started again at the same rate");
        e.prepare(44_100.0, 2);
        assert_eq!(e.workers(), 0, "kept with the old voices");
        assert_eq!(e.start_workers(2, None), started(2));
        assert_eq!(e.start_workers(1, None), started(1));
        assert_eq!(e.pool_starts, 3);
        c.poly = false;
        e.set(&c);
        e.prepare(44_100.0, 2);
        assert_eq!(e.start_workers(1, None), 0, "kept with POLY off");
    }

    /// A mend stops before its next voice once told to (the plug-in dropped: decisions.md
    /// R23): told after one, it makes one clean voice; the next mend makes the rest.
    #[test]
    fn a_mend_stops_before_its_next_voice() {
        let spares = Spares::default();
        spares.make(48_000.0);
        for place in &spares.places[..3] {
            place.lock().unwrap().clean = false;
        }
        let asked = std::cell::Cell::new(0);
        spares.mend_until(|| {
            asked.set(asked.get() + 1);
            asked.get() > 1
        });
        let clean = || {
            spares
                .places
                .iter()
                .filter(|p| p.lock().unwrap().clean)
                .count()
        };
        assert_eq!(clean(), POLY_VOICES.2 - 2);
        spares.mend();
        assert_eq!(clean(), POLY_VOICES.2);
    }

    /// An engine whose workers come out of `budget`, its voices built, POLY as `poly`, and
    /// workers asked of it as the plug-in asks them (for 5 ms blocks).
    fn planned(budget: &'static Budget, poly: bool, workers: usize) -> Engine {
        let mut e = Engine::new();
        e.crew = Arc::new(Crew::with_budget(budget));
        e.set(&Controls {
            poly,
            voices: 4,
            ..Controls::default()
        });
        e.prepare(48_000.0, 1);
        e.start_workers(workers, Some(Duration::from_millis(5)));
        e
    }

    /// POLY switched on or off, and the next block's start; returns whether serving is due
    /// (which the plug-in asks of its helper: decisions.md R23).
    fn switch(e: &mut Engine, poly: bool) -> bool {
        let c = Controls { poly, ..e.controls };
        e.set(&c);
        e.set_deadline(None);
        e.take_serving()
    }

    /// The helper thread's serving, run here (decisions.md R21, R23).
    fn serve(e: &Engine) {
        e.crew().serve();
    }

    /// An instance holds workers only while POLY is on (R21): activated with POLY off, none;
    /// switched on, none at once (its voices play on the caller's thread) and its workers once
    /// the helper thread has started them; switched off, let go at the next block and
    /// back in the budget once the helper thread has stopped them.
    #[test]
    fn an_instance_holds_workers_only_while_poly_is_on() {
        static THREE: Budget = Budget::new(|_| 3);
        let mut e = planned(&THREE, false, 2);
        assert_eq!((e.workers(), THREE.held()), (0, 0));
        assert!(!switch(&mut e, false));
        assert!(switch(&mut e, true), "no workers asked for");
        assert_eq!(e.workers(), 0);
        for key in [45, 52, 57, 64] {
            e.event(Event::Note { key, on: true });
        }
        let (mut l, mut r) = ([0.0f32; 256], [0.0f32; 256]);
        e.render(&[], &mut l, &mut r);
        assert!(l.iter().any(|x| x.abs() > 0.0), "silent while waiting");
        serve(&e);
        assert_eq!(THREE.held(), 2);
        e.set_deadline(None);
        assert_eq!(e.workers(), 2);
        e.render(&[], &mut l, &mut r);
        e.end_block(256);
        assert!(switch(&mut e, false), "not let go");
        assert_eq!(e.workers(), 0);
        assert_eq!(THREE.held(), 2, "stopped on the audio thread");
        serve(&e);
        assert_eq!(THREE.held(), 0);
    }

    /// The bug R21 fixes: an instance with POLY off, activated first, held its share of the
    /// budget, and one with POLY on, activated later, got none. Now the second takes them all;
    /// the first, switched on, finds none left and plays its voices itself, and asks again
    /// when POLY is next switched on, by which time the second has given them back.
    #[test]
    fn an_instance_with_poly_off_leaves_its_workers_to_one_with_it_on() {
        static FOUR: Budget = Budget::new(|_| 4);
        let mut off = planned(&FOUR, false, 4);
        let mut on = planned(&FOUR, true, 4);
        assert_eq!((off.workers(), on.workers()), (0, 4));
        assert!(switch(&mut off, true));
        serve(&off);
        off.set_deadline(None);
        assert_eq!(off.workers(), 0);
        assert!(!switch(&mut off, true), "asked again while POLY stays on");
        assert!(switch(&mut on, false));
        serve(&on);
        assert_eq!(FOUR.held(), 0);
        switch(&mut off, false);
        assert!(switch(&mut off, true));
        serve(&off);
        off.set_deadline(None);
        assert_eq!(off.workers(), 4);
        drop((off, on));
        assert_eq!(FOUR.held(), 0);
    }

    /// Voices built at a new rate while a pool started for the old ones waits to be taken, or
    /// the plug-in deactivated then: that pool is stopped, and the new voices' own pool gets
    /// every worker; nothing is kept from the budget.
    #[test]
    fn a_pool_for_voices_gone_is_stopped() {
        static TWO: Budget = Budget::new(|_| 2);
        let mut e = planned(&TWO, false, 2);
        assert!(switch(&mut e, true));
        serve(&e);
        assert_eq!(TWO.held(), 2);
        e.prepare(44_100.0, 1);
        assert_eq!(TWO.held(), 0);
        assert_eq!(e.start_workers(2, Some(Duration::from_millis(5))), 2);
        e.set_deadline(None);
        assert_eq!(e.workers(), 2);
        assert!(switch(&mut e, false));
        assert!(switch(&mut e, true));
        serve(&e);
        e.stop_workers();
        assert_eq!(TWO.held(), 0);
        e.set_deadline(None);
        assert_eq!(e.workers(), 0);
        assert!(!e.take_serving());
    }

    /// A new seed at the rate the voices were built for draws their characters again, as
    /// voices built with it have them (R18).
    #[test]
    fn a_new_seed_draws_the_characters_again() {
        let mut a = Engine::new();
        a.prepare(48_000.0, 1);
        a.prepare(48_000.0, 2);
        let mut b = Engine::new();
        b.prepare(48_000.0, 2);
        for (k, (x, y)) in a.slots.iter().zip(&b.slots).enumerate() {
            let (x, y) = (x.play.lock().unwrap(), y.play.lock().unwrap());
            assert_eq!(
                format!("{:?}", x.character),
                format!("{:?}", y.character),
                "voice {k}"
            );
        }
    }

    /// Left and right power of `keys` held for a second with `c`, and the largest difference
    /// of a sample's two sides.
    fn sides(c: &Controls, keys: &[u8]) -> (f64, f64, f64) {
        let mut e = Engine::new();
        e.set(c);
        e.prepare(48_000.0, 9);
        for &key in keys {
            e.event(Event::Note { key, on: true });
        }
        let (mut l, mut r) = ([0.0f32; 256], [0.0f32; 256]);
        let (mut mid, mut side, mut diff) = (0.0f64, 0.0f64, 0.0f64);
        for _ in 0..(48_000 / 256) {
            e.render(&[], &mut l, &mut r);
            e.end_block(256);
            for (&a, &b) in l.iter().zip(&r) {
                let (a, b) = (f64::from(a), f64::from(b));
                mid += (0.5 * (a + b)).powi(2);
                side += (0.5 * (a - b)).powi(2);
                diff = diff.max((a - b).abs());
            }
        }
        (mid, side, diff)
    }

    /// At full SPREAD a three-note chord fills the field (side within 3 dB of mid; the old
    /// places and pan law, a voice in the centre weighed double, gave less), and one voice
    /// alone without SPREAD stays in the centre, left and right the same (decisions.md R45).
    #[test]
    fn a_chord_fills_the_field_and_one_voice_without_spread_stays_in_the_centre() {
        let c = Controls {
            poly: true,
            voices: 8,
            spread: 1.0,
            ..Controls::default()
        };
        let (mid, side, _) = sides(&c, &[57, 61, 64]);
        let db = 10.0 * (side / mid).log10();
        assert!(db > -3.0, "side {db:.1} dB against mid");
        let mono = Controls {
            poly: false,
            spread: 0.0,
            ..c
        };
        let (_, _, diff) = sides(&mono, &[57]);
        assert_eq!(diff, 0.0, "one voice off the centre");
    }

    /// INNER at 100 % clears the centre: a chord on EVEN's three voices, one of them in the
    /// centre at INNER 0, is all at the edges, its side nearer its mid; one voice alone without
    /// SPREAD stays in the centre, whatever INNER (decisions.md R49).
    #[test]
    fn inner_clears_the_centre_and_one_voice_stays_there() {
        let c = Controls {
            poly: true,
            voices: 3,
            spread: 1.0,
            ..Controls::default()
        };
        let share = |c: &Controls| {
            let (mid, side, _) = sides(c, &[57, 61, 64]);
            side / mid
        };
        let (open, cleared) = (share(&c), share(&Controls { inner: 1.0, ..c }));
        assert!(
            cleared > 1.3 * open,
            "side against mid: {open:.3} then {cleared:.3}"
        );
        let mono = Controls {
            poly: false,
            spread: 0.0,
            inner: 1.0,
            ..c
        };
        let (_, _, diff) = sides(&mono, &[57]);
        assert_eq!(diff, 0.0, "one voice off the centre");
    }

    /// Where the one instrument sounds for each of `keys` played in turn with `c`, each held
    /// 0.15 s and let go for 0.25 s: the right side's share of its power while held, from 20 ms
    /// in (its place reached); and the editor's bits for it then.
    fn scattered(c: &Controls, keys: &[u8]) -> Vec<(f64, u32)> {
        let mut e = Engine::new();
        e.set(c);
        e.prepare(48_000.0, 9);
        let mut out = Vec::new();
        for &key in keys {
            e.event(Event::Note { key, on: true });
            let (l, r) = run_for(&mut e, 0.15);
            let from = (0.02 * 48_000.0) as usize;
            let power = |x: &[f64]| x[from..].iter().map(|v| v * v).sum::<f64>();
            let (pl, pr) = (power(&l), power(&r));
            assert!(pl + pr > 1e-3, "key {key} silent");
            out.push((pr / (pl + pr), e.sounding_mask()));
            e.event(Event::Note { key, on: false });
            run_for(&mut e, 0.25);
        }
        out
    }

    /// `e` rendered for `seconds`, left and right.
    fn run_for(e: &mut Engine, seconds: f64) -> (Vec<f64>, Vec<f64>) {
        let (mut l, mut r) = ([0.0f32; 240], [0.0f32; 240]);
        let (mut ls, mut rs) = (Vec::new(), Vec::new());
        for _ in 0..(seconds * 48_000.0 / 240.0).round() as usize {
            e.render(&[], &mut l, &mut r);
            e.end_block(240);
            ls.extend(l.iter().map(|&x| f64::from(x)));
            rs.extend(r.iter().map(|&x| f64::from(x)));
        }
        (ls, rs)
    }

    /// The right side's share of a voice's power at place `k` of `n` (the kit's gains).
    fn share_at(c: &Controls, k: usize, n: usize) -> f64 {
        let (l, r) = voice_gains(c.spread, c.inner, c.placement, k, n);
        let (l, r) = (f64::from(l), f64::from(r));
        r * r / (l * l + r * r)
    }

    /// The one instrument (POLY, UNISON and DOUBLE off) scatters: each attack at the next of
    /// VOICES's places, round and round, where the placement puts POLY's voices (EVEN; CENTER
    /// with INNER), at QUALITY's LO the same; the editor told the place (decisions.md
    /// R62).
    #[test]
    fn the_one_instrument_takes_the_next_place_at_each_attack() {
        let keys = [57, 60, 62, 64, 65, 67, 69];
        let even = Controls {
            voices: 4,
            spread: 1.0,
            ..Controls::default()
        };
        let centre = Controls {
            voices: 5,
            spread: 0.8,
            inner: 0.3,
            placement: Placement::Centre,
            ..Controls::default()
        };
        let light = Controls {
            potato: true,
            ..even
        };
        for c in [even, centre, light] {
            let got = scattered(&c, &keys);
            for (i, &(share, mask)) in got.iter().enumerate() {
                let k = i % c.voices;
                let want = share_at(&c, k, c.voices);
                assert!(
                    (share - want).abs() < 1e-6,
                    "{:?} (LO {}) note {i}: {share:.6} against place {k}'s {want:.6}",
                    c.placement,
                    c.potato
                );
                assert_eq!(mask, 1 << k, "note {i}: the editor's bit");
            }
            // (Not all in one place.)
            assert!(got.iter().any(|g| (g.0 - got[0].0).abs() > 0.1));
        }
    }

    /// A note played legato (its trigger contact staying closed, single triggering) stays at
    /// the place its phrase began; with RETRIGGER, a key pressed among others held starts the
    /// contours again, and moves to the next place (decisions.md R62).
    #[test]
    fn a_legato_note_stays_and_retrigger_moves_it() {
        for retrigger in [false, true] {
            let mut c = Controls {
                voices: 4,
                spread: 1.0,
                ..Controls::default()
            };
            c.panel.retrigger = retrigger;
            c.panel.priority = Priority::Last;
            let mut e = Engine::new();
            e.set(&c);
            e.prepare(48_000.0, 9);
            e.event(Event::Note { key: 57, on: true });
            run_for(&mut e, 0.15);
            e.event(Event::Note { key: 64, on: true });
            let (l, r) = run_for(&mut e, 0.15);
            let from = (0.04 * 48_000.0) as usize;
            let power = |x: &[f64]| x[from..].iter().map(|v| v * v).sum::<f64>();
            let share = power(&r) / (power(&l) + power(&r));
            let k = usize::from(retrigger);
            assert!(
                (share - share_at(&c, k, 4)).abs() < 1e-6,
                "retrigger {retrigger}: {share:.6} against place {k}'s"
            );
        }
    }

    /// All Notes Off (and the host's reset, which lets every key go so) starts the one
    /// instrument's pattern again from the first place, so that a part plays and renders its
    /// places alike each time (decisions.md R62).
    #[test]
    fn all_notes_off_starts_the_pattern_again() {
        let c = Controls {
            voices: 4,
            spread: 1.0,
            ..Controls::default()
        };
        let mut e = Engine::new();
        e.set(&c);
        e.prepare(48_000.0, 9);
        for key in [57, 60] {
            e.event(Event::Note { key, on: true });
            run_for(&mut e, 0.1);
            e.event(Event::Note { key, on: false });
            run_for(&mut e, 0.1);
        }
        assert_eq!(e.turn, 1);
        e.event(Event::AllNotesOff);
        e.event(Event::Note { key: 62, on: true });
        let (l, r) = run_for(&mut e, 0.1);
        let from = (0.02 * 48_000.0) as usize;
        let power = |x: &[f64]| x[from..].iter().map(|v| v * v).sum::<f64>();
        let share = power(&r) / (power(&l) + power(&r));
        assert!((share - share_at(&c, 0, 4)).abs() < 1e-6, "{share:.6}");
        assert_eq!(e.turn, 0);
    }

    /// At an attack the one instrument reaches its new place before its contours start
    /// ([`TURN_GLIDE`], `TRIGGER_ON` after its trigger contact closes): within 1 % of the way
    /// there, from the edge to the other (with [`GLIDE`], 46 % short), so that the note is
    /// heard where it sounds, not sliding there (decisions.md R62).
    #[test]
    fn the_one_instrument_is_at_its_new_place_before_its_contours_start() {
        let c = Controls {
            voices: 2,
            spread: 1.0,
            ..Controls::default()
        };
        let mut e = Engine::new();
        e.set(&c);
        e.prepare(48_000.0, 9);
        e.event(Event::Note { key: 57, on: true });
        run_for(&mut e, 0.1);
        e.event(Event::Note { key: 57, on: false });
        run_for(&mut e, 0.2);
        e.event(Event::Note { key: 60, on: true });
        for _ in 0..(ca72::light::laws::TRIGGER_ON * 48_000.0).round() as usize {
            e.tick_stereo(0.0);
        }
        let (gl, _) = e.slots[0].play.lock().unwrap().pan.unwrap();
        let ((from, _), (to, _)) = (
            voice_gains(1.0, 0.0, Placement::Even, 0, 2),
            voice_gains(1.0, 0.0, Placement::Even, 1, 2),
        );
        let short = (gl - f64::from(to)) / f64::from(from - to);
        assert!(
            short.abs() < 0.01,
            "{:.1} % of the way short",
            100.0 * short
        );
    }

    /// POLY's notes go round the voices: a note after one let go and fallen silent takes the
    /// next voice, not the first again.
    #[test]
    fn poly_notes_go_round_the_voices() {
        let c = Controls {
            poly: true,
            voices: 4,
            ..Controls::default()
        };
        let mut e = Engine::new();
        e.set(&c);
        e.prepare(48_000.0, 9);
        let (mut l, mut r) = ([0.0f32; 256], [0.0f32; 256]);
        for (i, key) in [48u8, 50, 52, 53, 55].into_iter().enumerate() {
            e.event(Event::Note { key, on: true });
            let k = e
                .slots
                .iter()
                .position(|s| s.gate)
                .expect("a voice for the note");
            assert_eq!(k, i % 4, "note {i}");
            e.event(Event::Note { key, on: false });
            for _ in 0..(3 * 48_000 / 256) {
                e.render(&[], &mut l, &mut r);
                e.end_block(256);
            }
            assert!(e.slots.iter().all(|s| !s.active), "note {i} not silent");
        }
    }

    /// A key played again takes the voice it last had, letting go or fallen silent, so a chord
    /// struck again stays where it was in the field; a new key still takes the next voice.
    #[test]
    fn a_key_played_again_keeps_its_voice() {
        let c = Controls {
            poly: true,
            voices: 8,
            ..Controls::default()
        };
        let mut e = Engine::new();
        e.set(&c);
        e.prepare(48_000.0, 9);
        let (mut l, mut r) = ([0.0f32; 256], [0.0f32; 256]);
        let chord = [60u8, 64, 67];
        let voices = |e: &Engine| -> Vec<usize> {
            chord
                .iter()
                .map(|&key| {
                    e.slots
                        .iter()
                        .position(|s| s.gate && s.note == key)
                        .expect("a voice for the key")
                })
                .collect()
        };
        let strike = |e: &mut Engine, on: bool| {
            for key in chord {
                e.event(Event::Note { key, on });
            }
        };
        strike(&mut e, true);
        let first = voices(&e);
        strike(&mut e, false);
        // Struck again while letting go, then again after falling silent.
        for wait in [2usize, 3 * 48_000 / 256] {
            for _ in 0..wait {
                e.render(&[], &mut l, &mut r);
                e.end_block(256);
            }
            strike(&mut e, true);
            assert_eq!(voices(&e), first, "after {wait} blocks");
            strike(&mut e, false);
        }
        e.event(Event::Note { key: 69, on: true });
        let new = e.slots.iter().position(|s| s.gate).unwrap();
        assert!(
            !first.contains(&new),
            "a new key on voice {new}, one of the chord's"
        );
    }

    /// A held note's power over `seconds` (after `skip` of them) with `c`, on key `key`.
    fn held_power(c: &Controls, key: u8, skip: f64, seconds: f64) -> f64 {
        let mut e = Engine::new();
        e.set(c);
        e.prepare(48_000.0, 9);
        e.event(Event::Note { key, on: true });
        let (mut l, mut r) = ([0.0f32; 256], [0.0f32; 256]);
        let mut sum = 0.0f64;
        for k in 0..((skip + seconds) * 48_000.0 / 256.0) as usize {
            e.render(&[], &mut l, &mut r);
            e.end_block(256);
            if k as f64 >= skip * 48_000.0 / 256.0 {
                sum += l
                    .iter()
                    .chain(&r)
                    .map(|&x| f64::from(x).powi(2))
                    .sum::<f64>();
            }
        }
        sum
    }

    /// UNISON's voices come in anywhere in their cycles, not in step: with LOCK (every voice
    /// the circuit as drawn, nothing to pull them apart) a held note is still about as loud
    /// with eight of them as with one (within 2 dB), where eight in step would sum 9 dB louder
    /// (decisions.md R45, the CA-74's R25).
    #[test]
    fn unison_voices_come_in_out_of_step() {
        let c = Controls {
            voices: 8,
            lock: true,
            ..Controls::default()
        };
        let one = held_power(&c, 40, 0.0, 1.0);
        let all = held_power(&Controls { unison: true, ..c }, 40, 0.0, 1.0);
        let db = 10.0 * (all / one).log10();
        assert!(db.abs() < 2.0, "UNISON {db:+.1} dB against one voice");
    }

    /// UNISON's voices, each its own, are trimmed by their count's square root: a held note
    /// is about as loud with eight of them as with one (within 2 dB, over its sustain).
    #[test]
    fn unison_is_about_as_loud_as_one_voice() {
        let c = Controls {
            voices: 8,
            entropy: 0.3,
            ..Controls::default()
        };
        let one = held_power(&c, 45, 1.0, 2.0);
        let all = held_power(&Controls { unison: true, ..c }, 45, 1.0, 2.0);
        let db = 10.0 * (all / one).log10();
        assert!(db.abs() < 2.0, "UNISON {db:+.1} dB against one voice");
    }

    /// Every UNISON voice takes the keys as the one instrument's keyboard does (NOTE PRIORITY,
    /// LOW by default, and single triggering): its keyboard voltage the one instrument's after each of a
    /// run of keys pressed and let go, legato and not; and every voice sounds.
    #[test]
    fn unison_voices_take_the_keys_as_the_instrument_does() {
        let events: [(u8, bool); 8] = [
            (60, true),
            (64, true),
            (55, true),
            (55, false),
            (60, false),
            (64, false),
            (67, true),
            (62, true),
        ];
        let run = |unison: bool| -> Vec<Vec<f64>> {
            let c = Controls {
                unison,
                voices: 4,
                ..Controls::default()
            };
            let mut e = Engine::new();
            e.set(&c);
            e.prepare(48_000.0, 9);
            let (mut l, mut r) = ([0.0f32; 256], [0.0f32; 256]);
            let mut out = Vec::new();
            for (key, on) in events {
                e.event(Event::Note { key, on });
                for _ in 0..8 {
                    e.render(&[], &mut l, &mut r);
                    e.end_block(256);
                }
                let n = if unison { 4 } else { 1 };
                out.push(
                    (0..n)
                        .map(|k| e.slots[k].play.lock().unwrap().voice.probe().0)
                        .collect(),
                );
            }
            if unison {
                assert_eq!(e.sounding(), 4, "every UNISON voice sounding");
            }
            out
        };
        let (one, all) = (run(false), run(true));
        for (i, (o, a)) in one.iter().zip(&all).enumerate() {
            for (k, v) in a.iter().enumerate() {
                assert!(
                    (v - o[0]).abs() < 1e-9,
                    "after event {i}, voice {k}'s keyboard at {v} V, the instrument's {}",
                    o[0]
                );
            }
        }
    }

    /// NOTE PRIORITY reaches the voices (decisions.md R50): with POLY off, and in each
    /// of UNISON's voices, the keyboard after 60, 67, 55 and 64 are pressed is where the key the
    /// priority chooses puts it alone (LOW the circuit's own, its lowest a little sharp with
    /// higher keys held: within 1.5 mV).
    #[test]
    fn note_priority_reaches_the_one_instrument_and_unisons_voices() {
        let keyboard = |priority: Priority, unison: bool, keys: &[u8]| -> Vec<f64> {
            let mut c = Controls {
                unison,
                voices: 4,
                ..Controls::default()
            };
            c.panel.priority = priority;
            let mut e = Engine::new();
            e.set(&c);
            e.prepare(48_000.0, 9);
            let (mut l, mut r) = ([0.0f32; 256], [0.0f32; 256]);
            for &key in keys {
                e.event(Event::Note { key, on: true });
                for _ in 0..8 {
                    e.render(&[], &mut l, &mut r);
                    e.end_block(256);
                }
            }
            let n = if unison { 4 } else { 1 };
            (0..n)
                .map(|k| e.slots[k].play.lock().unwrap().voice.probe().0)
                .collect()
        };
        for (priority, sounding, within) in [
            (Priority::Low, 55, 1.5e-3),
            (Priority::Last, 64, 5e-5),
            (Priority::High, 67, 5e-5),
        ] {
            let alone = keyboard(Priority::Low, false, &[sounding])[0];
            for unison in [false, true] {
                for (k, v) in keyboard(priority, unison, &[60, 67, 55, 64])
                    .into_iter()
                    .enumerate()
                {
                    assert!(
                        (v - alone).abs() < within,
                        "{priority:?}, UNISON {unison}: voice {k}'s keyboard {:.3} mV from {sounding}'s",
                        (v - alone) * 1e3
                    );
                }
            }
        }
    }

    /// Switching UNISON lets every key go: no voice's keyboard holds a key after it.
    #[test]
    fn switching_unison_lets_every_key_go() {
        let c = Controls {
            unison: true,
            voices: 4,
            ..Controls::default()
        };
        let mut e = Engine::new();
        e.set(&c);
        e.prepare(48_000.0, 9);
        let (mut l, mut r) = ([0.0f32; 256], [0.0f32; 256]);
        e.event(Event::Note { key: 60, on: true });
        e.render(&[], &mut l, &mut r);
        e.set(&Controls { unison: false, ..c });
        e.render(&[], &mut l, &mut r);
        for k in 0..4 {
            assert!(
                !e.slots[k].play.lock().unwrap().voice.envelopes().2,
                "voice {k} still holds a key"
            );
        }
    }

    /// Left and right of `c` with `keys` held for `seconds`, as `sides` plays them.
    fn played(c: &Controls, keys: &[u8], seconds: f64) -> (Vec<f64>, Vec<f64>) {
        let mut e = Engine::new();
        e.set(c);
        e.prepare(48_000.0, 9);
        for &key in keys {
            e.event(Event::Note { key, on: true });
        }
        let (mut l, mut r) = ([0.0f32; 256], [0.0f32; 256]);
        let (mut ls, mut rs) = (Vec::new(), Vec::new());
        for _ in 0..(seconds * 48_000.0 / 256.0) as usize {
            e.render(&[], &mut l, &mut r);
            e.end_block(256);
            ls.extend(l.iter().map(|&x| f64::from(x)));
            rs.extend(r.iter().map(|&x| f64::from(x)));
        }
        (ls, rs)
    }

    /// DOUBLE: a note's twin half the detune sharp on the left, its voice half flat on the
    /// right (at full SPREAD each alone on its side), the two apart by the detune
    /// (decisions.md R45, the CA-74's R28): 7 % of 100 cents. One oscillator, LOCK.
    #[test]
    fn double_puts_the_sharp_twin_left_and_the_flat_voice_right() {
        let mut c = Controls {
            double: 0.07,
            spread: 1.0,
            lock: true,
            ..Controls::default()
        };
        c.panel.osc[1].on = false;
        c.panel.osc[2].on = false;
        let (ls, rs) = played(&c, &[57], 2.0);
        let hz = |x: &[f64]| {
            ca72_analysis::pitch::pitch(&x[24_000..], 48_000)
                .median_hz
                .expect("a pitch")
        };
        let cents = 1200.0 * (hz(&ls) / hz(&rs)).log2();
        assert!(
            (cents - 7.0).abs() < 1.5,
            "left against right {cents:.1} cents"
        );
    }

    /// DOUBLE's two voices a note, each its own, are trimmed by two's square root: a chord is
    /// about as loud with DOUBLE as without (within 2 dB); with SPREAD at 0 the pair stays in
    /// the centre, left and right the same.
    #[test]
    fn double_is_about_as_loud_and_stays_in_the_centre_without_spread() {
        let loud = |double: f64| {
            let c = Controls {
                poly: true,
                double,
                entropy: 0.3,
                spread: 1.0,
                ..Controls::default()
            };
            let (mid, side, _) = sides(&c, &[57, 61, 64]);
            mid + side
        };
        let db = 10.0 * (loud(0.35) / loud(0.0)).log10();
        assert!(
            db.abs() < 2.0,
            "DOUBLE {db:+.1} dB against one voice a note"
        );
        let centred = Controls {
            double: 0.35,
            ..Controls::default()
        };
        let (_, _, diff) = sides(&centred, &[57]);
        assert_eq!(diff, 0.0, "the pair off the centre with SPREAD at 0");
    }

    /// DOUBLE takes the placement (the CA-74's R41): with CENTER a note on the first voice
    /// has its pair in the centre, left and right the same; with EVEN out to the sides (its
    /// first voice's place is an edge). INNER at 100 % puts every pair at the edges, as EDGES
    /// did (R49): CENTER's first pair is then where EVEN's is, to the bit. One voice alone
    /// (POLY off) has its pair as far out as SPREAD whatever the placement (the kit's `ALONE`),
    /// there too.
    #[test]
    fn double_takes_the_placement_its_pairs_mirrored() {
        let doubled = |poly: bool, placement: Placement, inner: f64| Controls {
            poly,
            voices: 4,
            double: 0.35,
            spread: 1.0,
            inner,
            placement,
            ..Controls::default()
        };
        let (_, _, diff) = sides(&doubled(true, Placement::Centre, 0.0), &[57]);
        assert_eq!(diff, 0.0, "CENTER's first pair in the centre");
        let even = sides(&doubled(true, Placement::Even, 0.0), &[57]);
        assert!(even.1 > 0.1 * even.0, "EVEN: the pair out to the sides");
        let inner = sides(&doubled(true, Placement::Centre, 1.0), &[57]);
        assert_eq!(
            inner, even,
            "INNER at 100 %: CENTER's first pair at the edges"
        );
        let alone = sides(&doubled(false, Placement::Centre, 0.0), &[57]);
        assert_eq!(alone, even, "POLY off: the pair at the edges");
    }

    /// DOUBLE's pairs played by the workers are the same to the bit as on the caller's thread
    /// (a pair is one place's: whoever takes it plays both).
    #[test]
    fn doubled_voices_on_the_workers_are_the_same_to_the_bit() {
        let c = Controls {
            poly: true,
            double: 0.35,
            spread: 1.0,
            entropy: 0.3,
            ..Controls::default()
        };
        let run = |workers: usize| -> Vec<f32> {
            let mut e = Engine::new();
            e.set(&c);
            e.prepare(48_000.0, 9);
            if workers > 0 {
                e.start_workers(workers, None);
            }
            for key in [48, 55, 60, 64, 67] {
                e.event(Event::Note { key, on: true });
            }
            let (mut l, mut r) = ([0.0f32; 256], [0.0f32; 256]);
            let mut out = Vec::new();
            for _ in 0..100 {
                e.render(&[], &mut l, &mut r);
                e.end_block(256);
                out.extend_from_slice(&l);
                out.extend_from_slice(&r);
            }
            out
        };
        let (alone, shared) = (run(0), run(2));
        assert!(alone.iter().any(|x| x.abs() > 1e-3), "silence compared");
        assert!(
            alone
                .iter()
                .zip(&shared)
                .all(|(a, b)| a.to_bits() == b.to_bits())
        );
    }

    /// A voice taken for a new note, free, let go or still held (stolen), keeps its place, and
    /// its twin its own: with WIDTH and DOUBLE unmoved, their gains at every run's end are their
    /// places' to the bit, never a step (the crackles reported in POLY at HI, 2026-10-19,
    /// cured by SCATTER at WIDTH 0: R-POLY-CRACKLE). At LO and at HI (its voices in pairs).
    #[test]
    fn a_voice_taken_for_a_new_note_keeps_its_place() {
        for potato in [true, false] {
            let c = Controls {
                poly: true,
                voices: 3,
                double: 0.12,
                spread: 1.0,
                inner: 0.3,
                potato,
                ..Controls::default()
            };
            let mut e = Engine::new();
            e.set(&c);
            e.prepare(48_000.0, 9);
            let (mut l, mut r) = ([0.0f32; 64], [0.0f32; 64]);
            // (Eight keys on three voices, each pressed before the one four before it is let
            // go: from the fourth on, a key takes a voice still held.)
            let keys = [48u8, 52, 55, 59, 62, 64, 60, 57];
            for (i, &key) in keys.iter().enumerate() {
                e.event(Event::Note { key, on: true });
                if i >= 4 {
                    e.event(Event::Note {
                        key: keys[i - 4],
                        on: false,
                    });
                }
                for _ in 0..6 {
                    e.render(&[], &mut l, &mut r);
                    e.end_block(64);
                    let m = e.mix();
                    for (k, s) in e.slots.iter().enumerate() {
                        let p = s.play.lock().unwrap();
                        let at = p.places(&m);
                        assert!(
                            p.pan.is_none_or(|g| g == at.voice),
                            "LO {potato}, key {i}, voice {k}: {:?} for {:?}",
                            p.pan,
                            at.voice
                        );
                        assert!(
                            p.twin.pan.is_none_or(|g| g == at.twin),
                            "LO {potato}, key {i}, twin {k}: {:?} for {:?}",
                            p.twin.pan,
                            at.twin
                        );
                    }
                }
            }
            assert!(l.iter().any(|x| x.abs() > 1e-4), "silence checked");
        }
    }

    /// DOUBLE turned off lets the note go, and the twin plays its tail out on its side (INNER at
    /// 100 %: every pair at the edges, as EDGES had them), then falls silent and stops; a key
    /// pressed after reaches the voice alone (POLY off, at its first place: it scatters now).
    /// POLY off and on.
    #[test]
    fn a_twin_let_go_plays_its_tail_out_and_takes_no_new_key() {
        for poly in [false, true] {
            let c = Controls {
                poly,
                double: 0.35,
                spread: 1.0,
                inner: 1.0,
                ..Controls::default()
            };
            let mut e = Engine::new();
            e.set(&c);
            e.prepare(48_000.0, 9);
            e.event(Event::Note { key: 57, on: true });
            let (mut l, mut r) = ([0.0f32; 256], [0.0f32; 256]);
            // (POLY off, the voice alone scatters once DOUBLE is off: its first note at the
            // first of VOICES's places. R62.)
            let (gl, gr) = voice_gains(1.0, 1.0, Placement::Even, 0, c.voices);
            let mut block = |e: &mut Engine| {
                e.render(&[], &mut l, &mut r);
                e.end_block(256);
                l.iter().zip(&r).fold((0.0, 0.0), |(d, off), (&a, &b)| {
                    (
                        f64::max(d, f64::from(a - b).abs()),
                        f64::max(off, f64::from(a * gr - b * gl).abs()),
                    )
                })
            };
            for _ in 0..40 {
                block(&mut e);
            }
            e.set(&Controls { double: 0.0, ..c });
            assert!(
                block(&mut e).0 > 1e-3,
                "poly {poly}: the tail not to either side"
            );
            for _ in 0..(10 * 48_000 / 256) {
                block(&mut e);
            }
            let live = |e: &Engine| e.slots.iter().any(|s| s.play.lock().unwrap().twin.live);
            assert!(!live(&e), "poly {poly}: a twin still live after its tail");
            e.event(Event::Note { key: 60, on: true });
            let (mut most, mut heard) = (0.0f64, 0.0f64);
            for _ in 0..40 {
                let (d, off) = block(&mut e);
                if !poly {
                    (most, heard) = (most.max(off), heard.max(d));
                }
            }
            assert!(
                most < 1e-6 * heard.max(1.0),
                "poly {poly}: a new key off its place"
            );
            assert!(poly || heard > 1e-3, "the new key silent");
            assert!(!live(&e), "poly {poly}: a twin took the new key");
            assert!(
                e.slots
                    .iter()
                    .all(|s| !s.play.lock().unwrap().twin.voice.envelopes().2),
                "poly {poly}: a twin holds a key"
            );
        }
    }

    /// A held note's energy (left and right) below and above `hz`, `skip` to `skip + 1` s in,
    /// with `c`; and the samples. A sustained sound: oscillator 1's sawtooth alone, the
    /// filter half open.
    fn bands(c: &Controls, hz: f64, skip: f64) -> (f64, f64, Vec<f64>) {
        let (l, r) = played(c, &[45], skip + 1.0);
        let from = (skip * 48_000.0) as usize;
        let y: Vec<f64> = l[from..]
            .iter()
            .zip(&r[from..])
            .map(|(a, b)| a + b)
            .collect();
        let n = y.len();
        let (mut low, mut high) = (0.0, 0.0);
        // (A plain DFT on whole-hertz bins: one second, 1 Hz apart.)
        for f in (5..4_000).step_by(5) {
            let w = 2.0 * std::f64::consts::PI * f as f64 / 48_000.0;
            let (mut re, mut im) = (0.0, 0.0);
            for (k, &v) in y.iter().enumerate() {
                re += v * (w * k as f64).cos();
                im += v * (w * k as f64).sin();
            }
            let e = (re * re + im * im) / n as f64;
            if (f as f64) < hz {
                low += e;
            } else {
                high += e;
            }
        }
        (low, high, y)
    }

    fn sustained() -> Controls {
        let mut c = Controls {
            lock: true,
            ..Controls::default()
        };
        c.panel.osc[1].on = false;
        c.panel.osc[2].on = false;
        c.panel.loudness_contour.sustain = 1.0;
        c
    }

    /// DRIVE drives the filter's input pair harder: a held note grows louder with it, and by
    /// less the higher it goes, as the pair saturates (on a sawtooth +2.7, +5.1, +8.5 and +10.7
    /// dB at 3, 6, 12 and 24 dB; decisions.md R45). (Not brighter: the ladder's stages,
    /// slew-limited by their currents, take the highs the clipping adds.)
    #[test]
    fn drive_drives_the_filter_harder() {
        let c = sustained();
        let (lo0, hi0, _) = bands(&c, 1_000.0, 0.5);
        // (AUTO GAIN off: DRIVE alone.)
        let louder = |drive: f64| {
            let c = Controls {
                drive,
                auto_gain: false,
                ..c
            };
            let (lo, hi, _) = bands(&c, 1_000.0, 0.5);
            10.0 * ((lo + hi) / (lo0 + hi0)).log10()
        };
        let (a, b, d) = (louder(6.0), louder(12.0), louder(24.0));
        assert!(a > 3.0, "DRIVE 6 dB: {a:+.1} dB");
        assert!(
            a < b && b < d,
            "DRIVE 6, 12, 24 dB: {a:+.1}, {b:+.1}, {d:+.1} dB"
        );
        assert!(d < 18.0, "DRIVE 24 dB: {d:+.1} dB, the pair not saturating");
    }

    /// LEVEL moves the output by its decibels, and at 0 dB is no gain at all.
    #[test]
    fn level_moves_the_output_by_its_decibels() {
        assert_eq!(level_gain(0.0), 1.0);
        let c = sustained();
        let (l0, _) = played(&c, &[45], 0.3);
        let (l, _) = played(&Controls { level: -6.0, ..c }, &[45], 0.3);
        let (a, b) = (
            l0.iter().map(|x| x * x).sum::<f64>(),
            l.iter().map(|x| x * x).sum::<f64>(),
        );
        let db = 10.0 * (b / a).log10();
        assert!((db + 6.0).abs() < 1e-3, "LEVEL -6 dB: {db:+.4} dB");
    }

    /// AUTO GAIN's correction glides on the output: switched off at DRIVE 12 dB, it takes the
    /// output up by the curve's correction there, a sample at a time, without a step (the
    /// ratio of the outputs every sample from 1 to the correction, by at most a hundredth a
    /// sample). Without DRIVE it asks for none: AUTO GAIN on or off, the same to the bit
    /// (decisions.md R45, the CA-74's R29).
    #[test]
    fn autos_correction_glides_and_without_drive_there_is_none() {
        let run = |c: Controls, then: Controls| -> Vec<f32> {
            let mut e = Engine::new();
            e.set(&c);
            e.prepare(48_000.0, 9);
            e.event(Event::Note { key: 45, on: true });
            let (mut l, mut r) = ([0.0f32; 256], [0.0f32; 256]);
            let mut y = Vec::new();
            for k in 0..60 {
                if k == 40 {
                    e.set(&then);
                }
                e.render(&[], &mut l, &mut r);
                e.end_block(256);
                y.extend_from_slice(&l);
            }
            y
        };
        let c = Controls {
            drive: 12.0,
            ..sustained()
        };
        let off = Controls {
            auto_gain: false,
            ..c
        };
        let (on, switched) = (run(c, c), run(c, off));
        let at = 40 * 256;
        assert_eq!(on[..at], switched[..at]);
        let want = 10f64.powf(-AVERAGE.at(12.0, DRIVE_TOP) / 20.0);
        // (Each sample's ratio where the output is not near nought, and its index.)
        let ratios: Vec<(usize, f64)> = on[at..]
            .iter()
            .zip(&switched[at..])
            .enumerate()
            .filter(|(_, (a, _))| a.abs() > 1e-3)
            .map(|(i, (a, b))| (i, f64::from(*b) / f64::from(*a)))
            .collect();
        assert!(ratios[0].1 < 1.01, "a step: {}", ratios[0].1);
        assert!(
            ratios
                .windows(2)
                .filter(|w| w[1].0 == w[0].0 + 1)
                .all(|w| (w[1].1 - w[0].1).abs() < 0.01),
            "a step within the glide"
        );
        let last = ratios[ratios.len() - 1].1;
        assert!((last - want).abs() < 1e-3 * want, "{last} against {want}");
        let plain = sustained();
        let (a, b) = (
            run(plain, plain),
            run(
                plain,
                Controls {
                    auto_gain: false,
                    ..plain
                },
            ),
        );
        assert_eq!(a, b);
    }

    /// R-AUTO-GAIN-DRIVE: a curve measured while a note plays glides in over REFINE_GLIDE (5.6
    /// dB here, at most 0.02 dB a millisecond, 1 - 1/e of the way after REFINE_GLIDE); a
    /// preset's or a session's is the sound's within the controls' glide (10 ms; within a hundredth
    /// of a decibel after 0.1 s).
    #[test]
    fn a_measured_curve_glides_in_and_a_presets_is_the_sounds_at_once() {
        let c = Controls {
            drive: 12.0,
            ..sustained()
        };
        let quieter = Curve([-8.0, -14.0, -18.0, -20.0]);
        let run = |refine: bool| -> Vec<f32> {
            let mut e = Engine::new();
            e.set(&c);
            e.prepare(48_000.0, 9);
            e.event(Event::Note { key: 45, on: true });
            let (mut l, mut r) = ([0.0f32; 256], [0.0f32; 256]);
            let mut y = Vec::new();
            for k in 0..200 {
                if k == 40 {
                    if refine {
                        e.refine_curve(quieter);
                    } else {
                        e.set_curve(quieter);
                    }
                }
                e.render(&[], &mut l, &mut r);
                e.end_block(256);
                y.extend_from_slice(&l);
            }
            y
        };
        let off = {
            let mut e = Engine::new();
            e.set(&Controls {
                auto_gain: false,
                ..c
            });
            e.prepare(48_000.0, 9);
            e.event(Event::Note { key: 45, on: true });
            let (mut l, mut r) = ([0.0f32; 256], [0.0f32; 256]);
            let mut y = Vec::new();
            for _ in 0..200 {
                e.render(&[], &mut l, &mut r);
                e.end_block(256);
                y.extend_from_slice(&l);
            }
            y
        };
        let at = 40 * 256;
        // The correction in dB, sample by sample, where the output is not near nought.
        let db = |y: &[f32]| -> Vec<(usize, f64)> {
            y.iter()
                .zip(&off)
                .enumerate()
                .skip(at)
                .filter(|(_, (_, b))| b.abs() > 1e-3)
                .map(|(i, (a, b))| (i, 20.0 * (f64::from(*a) / f64::from(*b)).log10()))
                .collect()
        };
        let (from, to) = (AVERAGE.at(12.0, DRIVE_TOP), quieter.at(12.0, DRIVE_TOP));
        let refined = db(&run(true));
        // (At most the glide's first slope: the step spread over REFINE_GLIDE.)
        let steepest = (to - from).abs() / (REFINE_GLIDE * 1000.0) * 1.05;
        for w in refined.windows(2) {
            let per_ms = (w[1].1 - w[0].1).abs() / (w[1].0 - w[0].0) as f64 * 48.0;
            assert!(per_ms < steepest, "{per_ms} dB a millisecond at {}", w[1].0);
        }
        let tau = at + (REFINE_GLIDE * 48_000.0) as usize;
        let near = refined.iter().find(|(i, _)| *i >= tau).unwrap().1;
        // (The glide is the gain's, not its decibels'.)
        let gain = |db: f64| 10f64.powf(db / 20.0);
        let gone = (gain(near) - gain(from)) / (gain(to) - gain(from));
        assert!((gone - (1.0 - (-1.0f64).exp())).abs() < 0.02, "{gone}");
        let loaded = db(&run(false));
        let soon = loaded.iter().find(|(i, _)| *i >= at + 48 * 100).unwrap().1;
        assert!((soon - to).abs() < 0.01, "{soon} against {to}");
    }

    /// Blocks of `e` for `seconds`, the loudest sample's level.
    fn loudest(e: &mut Engine, seconds: f64) -> f32 {
        let (mut l, mut r) = ([0.0f32; 256], [0.0f32; 256]);
        let mut most = 0.0f32;
        for _ in 0..(seconds * 48_000.0 / 256.0) as usize {
            e.render(&[], &mut l, &mut r);
            e.end_block(256);
            most = l.iter().chain(&r).fold(most, |a, x| a.max(x.abs()));
        }
        most
    }

    /// QUALITY at LO (decisions.md R47): the one instrument and POLY's voices play the
    /// light model, not the circuit's, and switching QUALITY lets go of the keys held, as
    /// switching POLY does.
    #[test]
    fn quality_lo_plays_the_light_voices_and_switching_it_lets_go_of_the_keys() {
        let mut c = Controls {
            potato: true,
            ..Controls::default()
        };
        let mut e = Engine::new();
        e.set(&c);
        e.prepare(48_000.0, 1);
        e.event(Event::Note { key: 57, on: true });
        assert!(loudest(&mut e, 0.5) > 0.01, "the light voice silent");
        {
            let p = e.slots[0].play.lock().unwrap();
            assert!(p.light.held(), "the key not on the light voice");
            assert!(!p.voice.envelopes().2, "the circuit's voice got the key");
        }
        c.potato = false;
        e.set(&c);
        assert!(
            !e.slots[0].play.lock().unwrap().light.held(),
            "the key still held on the light voice"
        );
        // POLY at LO: a chord's voices sound, and fall silent and free after (the DECAY
        // switch off: the contours let go at once).
        (c.potato, c.poly, c.panel.decay) = (true, true, false);
        e.set(&c);
        for key in [48, 55, 64] {
            e.event(Event::Note { key, on: true });
        }
        assert!(loudest(&mut e, 0.5) > 0.01, "POLY's light voices silent");
        assert_eq!(e.sounding(), 3);
        e.event(Event::AllNotesOff);
        // (Two seconds: the VCA's thump dies away over about one, as the circuit's does.)
        loudest(&mut e, 2.0);
        assert_eq!(e.sounding(), 0, "POLY's light voices not freed");
    }

    /// The voices built on a thread of their own (decisions.md R-PREPARE-STACK): an engine
    /// prepared, at a rate no voice has been made at yet, from a thread with no more stack than
    /// a macOS host's secondary thread has (512 KB), as pluginval initialises an Audio Unit.
    #[test]
    fn prepare_fits_a_hosts_secondary_thread() {
        // (The engine made here, on the test's thread: in a debug build its construction alone
        // takes more than 512 KB, which a release build does not; `prepare` is what a host
        // calls on its own thread.)
        let mut e = Box::new(Engine::new());
        let made = std::thread::Builder::new()
            .stack_size(512 << 10)
            .spawn(move || {
                e.prepare(47_250.0, 1);
                e.slots.len()
            })
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(made, POLY_VOICES.2);
    }

    /// QUALITY at ULTRA (decisions.md R-ULTRA-HIFI): the circuit's voices use High Fidelity,
    /// and switching to HI and back changes the model's setting under the key held, without
    /// letting go of it. (A short while: ULTRA is slow, more so in a debug build.)
    #[test]
    fn quality_ultra_plays_the_circuit_with_high_fidelity_and_keeps_the_key() {
        let mut c = Controls {
            ultra: true,
            ..Controls::default()
        };
        let mut e = Engine::new();
        e.set(&c);
        e.prepare(48_000.0, 1);
        e.event(Event::Note { key: 57, on: true });
        assert!(loudest(&mut e, 0.05) > 0.001, "ULTRA's voice silent");
        {
            let p = e.slots[0].play.lock().unwrap();
            assert_eq!(p.voice.panel.quality, Quality::HighFidelity);
            assert!(p.voice.envelopes().2, "the key not on the circuit's voice");
        }
        c.ultra = false;
        e.set(&c);
        loudest(&mut e, 0.01);
        let p = e.slots[0].play.lock().unwrap();
        assert_eq!(p.voice.panel.quality, Quality::Potato);
        assert!(p.voice.envelopes().2, "switching to HI let go of the key");
    }

    /// At LO the one instrument plays a run at a time ([`Engine::run_one`]): the same samples,
    /// to the bit, as a sample at a time, ENTROPY's movements and the glide of its place
    /// among them.
    #[test]
    fn the_one_voice_at_lo_plays_a_run_as_a_sample_at_a_time() {
        let c = Controls {
            potato: true,
            entropy: 0.5,
            ..Controls::default()
        };
        let make = || {
            let mut e = Engine::new();
            e.set(&c);
            e.prepare(48_000.0, 3);
            e.event(Event::Note { key: 60, on: true });
            e
        };
        let (mut runs, mut samples) = (make(), make());
        let (mut l, mut r) = ([0.0f32; 256], [0.0f32; 256]);
        let mut heard = 0.0f32;
        for block in 0..40 {
            runs.render(&[], &mut l, &mut r);
            for i in 0..256 {
                let (a, b) = samples.tick_stereo(0.0);
                assert!(
                    a.to_bits() == l[i].to_bits() && b.to_bits() == r[i].to_bits(),
                    "block {block}, sample {i}"
                );
                heard = heard.max(a.abs());
            }
            runs.end_block(256);
            samples.end_block(256);
        }
        assert!(heard > 0.01, "silent");
    }

    /// At LO DOUBLE's pair and UNISON's voices are placed and levelled as at HI: the pair to
    /// either side at full SPREAD, the unison about as loud as one voice.
    #[test]
    fn lo_places_double_and_levels_unison_as_hi() {
        let doubled = Controls {
            potato: true,
            poly: true,
            double: 1.0,
            spread: 1.0,
            ..Controls::default()
        };
        let (mid, side, _) = sides(&doubled, &[57]);
        assert!(side > 0.05 * mid && mid > 0.0, "mid {mid}, side {side}");
        let c = Controls {
            potato: true,
            voices: 8,
            entropy: 0.3,
            ..Controls::default()
        };
        let one = held_power(&c, 45, 1.0, 2.0);
        let all = held_power(&Controls { unison: true, ..c }, 45, 1.0, 2.0);
        let db = 10.0 * (all / one).log10();
        assert!(db.abs() < 2.0, "UNISON {db:+.1} dB against one voice");
    }

    /// AUTO GAIN measures a sound at LO with the light voices (`drive.rs`), its key the sound's
    /// at LO; at HI keyed as before.
    #[test]
    fn auto_gain_measures_lo_with_the_light_voices() {
        let hi = Controls::default();
        let lo = Controls { potato: true, ..hi };
        assert_ne!(crate::drive::sound(&lo), crate::drive::sound(&hi));
        let probe = Probe::new(&lo, MIN_VOICE_RATE);
        assert!(probe.voices.iter().all(|v| v.potato));
        let e = probe.energy(0.0);
        assert!(e.is_finite() && e > 0.0, "{e}");
        assert!(
            Probe::new(&hi, MIN_VOICE_RATE)
                .voices
                .iter()
                .all(|v| !v.potato)
        );
    }

    /// The one instrument at 48 kHz with `c`.
    fn one_instrument(c: &Controls) -> Engine {
        let mut e = Engine::new();
        e.set(c);
        e.prepare(48_000.0, 1);
        e
    }

    /// `seconds` of blocks of 256 samples, `ext` at EXTERNAL INPUT: the output's peak.
    fn played_for(e: &mut Engine, seconds: f64, ext: f32) -> f32 {
        let (mut l, mut r) = ([0.0f32; 256], [0.0f32; 256]);
        let x = [ext; 256];
        let mut peak = 0.0f32;
        for _ in 0..(seconds * 48_000.0 / 256.0).ceil() as usize {
            e.render(&x, &mut l, &mut r);
            e.end_block(256);
            peak = l.iter().chain(&r).fold(peak, |m, y| m.max(y.abs()));
        }
        peak
    }

    /// Played until asleep, at most `seconds`: whether it fell asleep.
    fn sleeps_within(e: &mut Engine, seconds: f64) -> bool {
        let mut t = 0.0;
        while !e.asleep() && t < seconds {
            played_for(e, 0.1, 0.0);
            t += 0.1;
        }
        e.asleep()
    }

    /// The one instrument sleeps once silent and settled, never while a key is down, and wakes
    /// for a note, a control moved, a wheel, All Notes Off and a sound at EXTERNAL INPUT; asleep,
    /// the output is silent. So at QUALITY LO and with DOUBLE; never with POLY or UNISON, whose
    /// voices are freed instead, nor while A-440 sounds (decisions.md R68).
    #[test]
    fn a_silent_settled_instrument_sleeps_and_wakes() {
        let mut e = one_instrument(&Controls::default());
        assert!(sleeps_within(&mut e, 30.0), "awake with nothing played");
        assert_eq!(played_for(&mut e, 0.1, 0.0), 0.0);
        e.event(Event::Note { key: 60, on: true });
        assert!(!e.asleep());
        assert!(played_for(&mut e, 1.0, 0.0) > 0.01, "the note silent");
        played_for(&mut e, 4.0, 0.0);
        assert!(!e.asleep(), "asleep with a key down");
        e.event(Event::Note { key: 60, on: false });
        assert!(
            sleeps_within(&mut e, 30.0),
            "awake 30 s after the key was let go"
        );
        let mut c = Controls::default();
        c.panel.cutoff = 0.7;
        e.set(&c);
        assert!(!e.asleep(), "a control moved left it asleep");
        assert!(sleeps_within(&mut e, 30.0));
        e.set(&c);
        assert!(e.asleep(), "the same controls again woke it");
        e.event(Event::Modulation(0.5));
        assert!(!e.asleep(), "the wheel left it asleep");
        assert!(sleeps_within(&mut e, 30.0));
        e.release_all();
        assert!(!e.asleep(), "All Notes Off left it asleep");
        assert!(sleeps_within(&mut e, 30.0));
        let (mut l, mut r) = ([0.0f32; 256], [0.0f32; 256]);
        e.render(&[0.0, 0.0, 0.25], &mut l, &mut r);
        assert!(!e.asleep(), "a sound at EXTERNAL INPUT left it asleep");
        e.never_sleep(true);
        assert!(!sleeps_within(&mut e, 5.0), "asleep when told never to");
        for (what, c) in [
            (
                "QUALITY LO",
                Controls {
                    potato: true,
                    ..Controls::default()
                },
            ),
            (
                "DOUBLE",
                Controls {
                    double: 0.3,
                    ..Controls::default()
                },
            ),
        ] {
            let mut e = one_instrument(&c);
            assert!(
                sleeps_within(&mut e, 30.0),
                "{what}: awake with nothing played"
            );
            e.event(Event::Note { key: 60, on: true });
            assert!(
                played_for(&mut e, 0.5, 0.0) > 0.01,
                "{what}: the note silent"
            );
            e.event(Event::Note { key: 60, on: false });
            assert!(sleeps_within(&mut e, 30.0), "{what}: awake after the note");
        }
        for (what, c) in [
            (
                "POLY",
                Controls {
                    poly: true,
                    ..Controls::default()
                },
            ),
            (
                "UNISON",
                Controls {
                    unison: true,
                    ..Controls::default()
                },
            ),
        ] {
            let mut e = one_instrument(&c);
            assert!(
                !sleeps_within(&mut e, 5.0),
                "{what}: the one instrument's sleep"
            );
        }
        let mut c = Controls::default();
        c.panel.a440 = true;
        let mut e = one_instrument(&c);
        assert!(!sleeps_within(&mut e, 5.0), "asleep while A-440 sounds");
    }

    /// An instrument woken from sleep plays its next note as one that never slept: at rest its
    /// contours within 20 uV of the other's, through the note its keyboard voltage the same,
    /// its contours within 0.5 mV of their 5 V (a sample or two of time: they tick at a
    /// sub-rate whose cycle stopped while it slept), its level within 0.1 dB. Otherwise only
    /// its free-running oscillators' and noise's phases differ (decisions.md R68).
    #[test]
    fn a_woken_instrument_plays_its_next_note_as_one_that_never_slept() {
        let (mut a, mut b) = (
            one_instrument(&Controls::default()),
            one_instrument(&Controls::default()),
        );
        b.never_sleep(true);
        for e in [&mut a, &mut b] {
            e.event(Event::Note { key: 48, on: true });
            played_for(e, 0.5, 0.0);
            e.event(Event::Note { key: 48, on: false });
        }
        assert!(sleeps_within(&mut a, 30.0));
        played_for(&mut b, 30.0, 0.0);
        assert!(!b.asleep());
        let probe = |e: &Engine| {
            let p = e.slots[0].play.lock().unwrap();
            let (f, l, _) = p.voice.envelopes();
            (f, l, p.voice.probe().0)
        };
        let (x, y) = (probe(&a), probe(&b));
        let rested = (x.0 - y.0).abs().max((x.1 - y.1).abs());
        assert!(rested < 2e-5, "the contours {rested:e} V apart at rest");
        for e in [&mut a, &mut b] {
            e.event(Event::Note { key: 55, on: true });
        }
        assert!(!a.asleep());
        let (mut contours, mut keyboard, mut power) = (0.0f64, 0.0f64, [0.0f64; 2]);
        let (mut l, mut r) = ([0.0f32; 256], [0.0f32; 256]);
        for _ in 0..(0.5 * 48_000.0 / 256.0) as usize {
            for (k, e) in [&mut a, &mut b].into_iter().enumerate() {
                e.render(&[], &mut l, &mut r);
                e.end_block(256);
                power[k] += l.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>();
            }
            let (x, y) = (probe(&a), probe(&b));
            contours = contours.max((x.0 - y.0).abs()).max((x.1 - y.1).abs());
            keyboard = keyboard.max((x.2 - y.2).abs());
        }
        assert!(keyboard < 1e-9, "the keyboard {keyboard:e} V apart");
        assert!(contours < 5e-4, "the contours {contours:e} V apart");
        let db = 10.0 * (power[0] / power[1]).log10();
        assert!(db.abs() < 0.1, "levels {db:.3} dB apart");
    }
}

/// Each note's own expression (decisions.md R-MPE): it reaches its own voice only, follows the
/// note to its voice, is taken afresh by a new note and kept by one let go, and without any the
/// voices play as before to the bit.
#[cfg(test)]
mod expression_tests {
    use super::*;

    const RATE: f64 = 48_000.0;

    fn engine(c: &Controls) -> Engine {
        let mut e = Engine::new();
        e.set(c);
        e.prepare(RATE, 5);
        e
    }

    fn poly(voices: usize) -> Controls {
        Controls {
            poly: true,
            voices,
            ..Controls::default()
        }
    }

    fn reach() -> Reach {
        Reach {
            pressure_mod: 1.0,
            pressure_filter: 2.0,
            timbre_mod: 0.5,
            timbre_filter: -1.0,
        }
    }

    fn x(pressure: f32, timbre: f32, bend: f32) -> Expression {
        Expression {
            pressure,
            timbre,
            bend,
        }
    }

    fn on(e: &mut Engine, key: u8, expression: Expression) {
        e.event(Event::NoteExpression { key, expression });
        e.event(Event::Note { key, on: true });
    }

    fn off(e: &mut Engine, key: u8) {
        e.event(Event::Note { key, on: false });
    }

    /// The slot playing `key` (held), if any.
    fn slot_of(e: &Engine, key: u8) -> Option<usize> {
        e.slots.iter().position(|s| s.gate && s.note == key)
    }

    fn block(e: &mut Engine) -> (Vec<f32>, Vec<f32>) {
        let (mut l, mut r) = ([0.0f32; 256], [0.0f32; 256]);
        e.render(&[], &mut l, &mut r);
        e.end_block(256);
        (l.to_vec(), r.to_vec())
    }

    /// The voice in slot `k`'s last run, left and right.
    fn voice_out(e: &Engine, k: usize) -> Vec<u64> {
        let p = e.slots[k].play.lock().unwrap();
        p.out[0]
            .iter()
            .chain(&p.out[1])
            .map(|x| x.to_bits())
            .collect()
    }

    /// Two notes, one of them with pressure, timbre and its own bend: the other's voice plays
    /// the same samples to the bit as with none, at LO, HI (whose voices' filters are solved in
    /// pairs) and ULTRA; the expressive one's does not.
    #[test]
    fn simultaneous_notes_move_only_their_own_voices() {
        for (potato, ultra) in [(true, false), (false, false), (false, true)] {
            let c = Controls {
                potato,
                ultra,
                reach: reach(),
                ..poly(4)
            };
            let (mut plain, mut moved) = (engine(&c), engine(&c));
            for e in [&mut plain, &mut moved] {
                on(e, 48, Expression::NONE);
                on(e, 55, Expression::NONE);
            }
            moved.event(Event::NoteExpression {
                key: 48,
                expression: x(0.9, 0.3, 2.0),
            });
            let (a, b) = (slot_of(&moved, 48).unwrap(), slot_of(&moved, 55).unwrap());
            assert_eq!(
                (slot_of(&plain, 48), slot_of(&plain, 55)),
                (Some(a), Some(b))
            );
            let mut differs = false;
            for i in 0..24 {
                let (pl, _) = block(&mut plain);
                let (ml, _) = block(&mut moved);
                // (Runs of 128: the last run of each block is compared, every block.)
                assert_eq!(
                    voice_out(&plain, b),
                    voice_out(&moved, b),
                    "LO {potato} ULTRA {ultra}, block {i}"
                );
                differs |= voice_out(&plain, a) != voice_out(&moved, a);
                assert!(ml.iter().all(|y| y.is_finite()) && pl.iter().all(|y| y.is_finite()));
            }
            assert!(
                differs,
                "the expressive note is moved (LO {potato}, ULTRA {ultra})"
            );
        }
    }

    /// With every reach at 0, pressure and timbre (each note's and the shared) change nothing:
    /// the output to the bit as without them; and a reach with no expression likewise. Each
    /// QUALITY mode, the one instrument and POLY.
    #[test]
    fn without_expression_or_reach_the_voices_play_as_before_to_the_bit() {
        for (potato, ultra) in [(true, false), (false, false), (false, true)] {
            for poly_on in [false, true] {
                let base = Controls {
                    potato,
                    ultra,
                    poly: poly_on,
                    voices: 4,
                    ..Controls::default()
                };
                let reached = Controls {
                    reach: reach(),
                    ..base
                };
                let mut before = engine(&base);
                let mut unreached = engine(&base);
                let mut untouched = engine(&reached);
                for key in [48, 52, 55] {
                    before.event(Event::Note { key, on: true });
                    on(&mut unreached, key, x(1.0, 0.7, 0.0));
                    untouched.event(Event::Note { key, on: true });
                }
                unreached.event(Event::ChannelPressure(0.8));
                unreached.event(Event::Timbre(0.4));
                for i in 0..12 {
                    if i == 6 {
                        for e in [&mut before, &mut unreached, &mut untouched] {
                            off(e, 52);
                        }
                    }
                    let want = block(&mut before);
                    let bits = |v: &(Vec<f32>, Vec<f32>)| {
                        v.0.iter()
                            .chain(&v.1)
                            .map(|y| y.to_bits())
                            .collect::<Vec<_>>()
                    };
                    let tag = format!("LO {potato} ULTRA {ultra} POLY {poly_on} block {i}");
                    assert_eq!(bits(&want), bits(&block(&mut unreached)), "{tag}");
                    assert_eq!(bits(&want), bits(&block(&mut untouched)), "{tag}");
                }
            }
        }
    }

    /// A note's expression sent before it reaches it at its attack, at once (not gliding from
    /// the voice's last note's); a note let go keeps what it had while its key's next note
    /// takes its own; the voice that last played a key, free, takes it again with the new.
    #[test]
    fn expression_follows_the_note_through_release_and_reuse() {
        let mut e = engine(&Controls {
            reach: reach(),
            ..poly(4)
        });
        on(&mut e, 60, x(0.5, 0.2, 1.0));
        let k = slot_of(&e, 60).unwrap();
        assert_eq!(e.slots[k].expression, x(0.5, 0.2, 1.0));
        block(&mut e);
        assert_eq!(
            e.slots[k].play.lock().unwrap().expressed.at,
            [0.5, f64::from(0.2f32), 1.0]
        );
        off(&mut e, 60);
        e.event(Event::NoteExpression {
            key: 60,
            expression: x(1.0, 1.0, -3.0),
        });
        assert_eq!(e.slots[k].expression, x(0.5, 0.2, 1.0), "let go, kept");
        block(&mut e);
        on(&mut e, 60, x(0.1, 0.0, 0.0));
        assert_eq!(slot_of(&e, 60), Some(k), "its voice again");
        assert_eq!(
            e.slots[k].expression,
            x(0.1, 0.0, 0.0),
            "the new note's own"
        );
        block(&mut e);
        let at = e.slots[k].play.lock().unwrap().expressed.at;
        assert_eq!(at, [f64::from(0.1f32), 0.0, 0.0], "at once, not gliding");
        // While held, a change glides there.
        e.event(Event::NoteExpression {
            key: 60,
            expression: x(0.9, 0.0, 0.0),
        });
        let mut last = f64::from(0.1f32);
        for _ in 0..3 {
            let (l, _) = (block(&mut e), ());
            assert!(l.0.iter().all(|y| y.is_finite()));
            let now = e.slots[k].play.lock().unwrap().expressed.at[0];
            assert!(now > last && now <= f64::from(0.9f32), "{now}");
            last = now;
        }
        for _ in 0..20 {
            block(&mut e);
        }
        assert_eq!(
            e.slots[k].play.lock().unwrap().expressed.at[0],
            f64::from(0.9f32)
        );
    }

    /// A held voice stolen for a new note takes the new note's expression; the stolen note's
    /// later expression reaches no voice; the order notes are let go in changes nothing of the
    /// others'.
    #[test]
    fn a_stolen_voice_takes_the_new_notes_expression() {
        let mut e = engine(&Controls {
            reach: reach(),
            ..poly(2)
        });
        on(&mut e, 60, x(0.1, 0.0, 0.0));
        on(&mut e, 62, x(0.2, 0.0, 0.0));
        block(&mut e);
        let (a, b) = (slot_of(&e, 60).unwrap(), slot_of(&e, 62).unwrap());
        on(&mut e, 64, x(0.3, 0.0, 5.0));
        assert_eq!(slot_of(&e, 64), Some(a), "the oldest held");
        assert_eq!(slot_of(&e, 60), None);
        assert_eq!(e.slots[a].expression, x(0.3, 0.0, 5.0));
        e.event(Event::NoteExpression {
            key: 60,
            expression: x(1.0, 1.0, 1.0),
        });
        assert_eq!(e.slots[a].expression, x(0.3, 0.0, 5.0));
        assert_eq!(e.slots[b].expression, x(0.2, 0.0, 0.0));
        block(&mut e);
        assert_eq!(
            e.slots[a].play.lock().unwrap().expressed.at,
            [f64::from(0.3f32), 0.0, 5.0]
        );
        // Let go in either order: each keeps its own.
        off(&mut e, 64);
        e.event(Event::NoteExpression {
            key: 62,
            expression: x(0.6, 0.0, 0.0),
        });
        assert_eq!(e.slots[b].expression, x(0.6, 0.0, 0.0));
        assert_eq!(e.slots[a].expression, x(0.3, 0.0, 5.0));
        off(&mut e, 62);
        e.event(Event::NoteExpression {
            key: 64,
            expression: x(0.9, 0.0, 0.0),
        });
        assert_eq!(e.slots[a].expression, x(0.3, 0.0, 5.0));
    }

    /// The one instrument and UNISON's voices take the expression of the key NOTE PRIORITY
    /// sounds: LOW the lowest held, HIGH the highest, LAST the last pressed; back to a key
    /// still held as the sounding one is let go; the last kept once none is held.
    #[test]
    fn mono_and_unison_take_the_sounding_notes_expression() {
        for (unison, priority) in [
            (false, Priority::Low),
            (false, Priority::High),
            (false, Priority::Last),
            (true, Priority::Low),
        ] {
            let mut c = Controls {
                unison,
                voices: 3,
                reach: reach(),
                ..Controls::default()
            };
            c.panel.priority = priority;
            let mut e = engine(&c);
            let (lo, mid, hi) = (x(0.1, 0.0, 0.0), x(0.2, 0.0, 0.0), x(0.3, 0.0, 0.0));
            on(&mut e, 60, mid);
            on(&mut e, 48, lo);
            on(&mut e, 72, hi);
            let want = match priority {
                Priority::Low => lo,
                Priority::High | Priority::Last => hi,
            };
            let n = e.instruments();
            assert_eq!(n, if unison { 3 } else { 1 });
            for k in 0..n {
                assert_eq!(e.slots[k].expression, want, "{priority:?} UNISON {unison}");
            }
            // A change to a key not sounding reaches nothing; to the sounding one, every voice.
            let other = if priority == Priority::Low { 72 } else { 48 };
            e.event(Event::NoteExpression {
                key: other,
                expression: x(0.9, 0.9, 0.0),
            });
            assert_eq!(e.slots[0].expression, want);
            let sounding = if priority == Priority::Low { 48 } else { 72 };
            e.event(Event::NoteExpression {
                key: sounding,
                expression: x(0.5, 0.0, 0.0),
            });
            for k in 0..n {
                assert_eq!(e.slots[k].expression, x(0.5, 0.0, 0.0));
            }
            block(&mut e);
            off(&mut e, sounding);
            assert_eq!(
                e.slots[0].expression,
                if priority == Priority::Low {
                    mid
                } else if priority == Priority::Last {
                    // (LAST: the last pressed of those still held, 48, as changed above.)
                    x(0.9, 0.9, 0.0)
                } else {
                    mid
                },
                "{priority:?}"
            );
            for key in [48, 60, 72] {
                off(&mut e, key);
            }
            let kept = e.slots[0].expression;
            e.event(Event::NoteExpression {
                key: 60,
                expression: x(1.0, 1.0, 1.0),
            });
            assert_eq!(e.slots[0].expression, kept, "none held: the last kept");
        }
    }

    /// Ordinary MIDI's channel pressure and timbre reach every voice alike, a note's own the
    /// larger where it is larger; Reset All Controllers rests the pressures and bends, not the
    /// timbre (RP-015).
    #[test]
    fn shared_pressure_and_timbre_reach_every_voice() {
        let mut e = engine(&Controls {
            reach: reach(),
            ..poly(4)
        });
        on(&mut e, 48, Expression::NONE);
        on(&mut e, 55, x(0.9, 0.0, 2.0));
        e.event(Event::ChannelPressure(0.6));
        e.event(Event::Timbre(0.25));
        let (a, b) = (slot_of(&e, 48).unwrap(), slot_of(&e, 55).unwrap());
        let p6 = f64::from(0.6f32);
        assert_eq!(e.handed(a), [p6, 0.25, 0.0]);
        assert_eq!(e.handed(b), [f64::from(0.9f32), 0.25, 2.0]);
        e.event(Event::ResetControllers);
        assert_eq!(e.handed(a), [0.0, 0.25, 0.0]);
        assert_eq!(e.handed(b), [0.0, 0.25, 0.0]);
        assert_eq!(e.keys[55], x(0.0, 0.0, 0.0));
    }

    /// A timbre about its rest (decisions.md R-MPE): a voice takes of its note's and the
    /// shared one the one farther from rest, its note's where as far; a note with none takes
    /// the shared, and a shared one at rest leaves each note's alone. Timbres from 0 (ZERO)
    /// take the larger, as before.
    #[test]
    fn a_voice_takes_the_timbre_farther_from_rest() {
        let mut e = engine(&Controls {
            reach: reach(),
            ..poly(4)
        });
        on(&mut e, 48, Expression::NONE);
        on(&mut e, 55, x(0.0, -0.5, 0.0));
        let (a, b) = (slot_of(&e, 48).unwrap(), slot_of(&e, 55).unwrap());
        assert_eq!((e.handed(a)[1], e.handed(b)[1]), (0.0, -0.5));
        e.event(Event::Timbre(0.25));
        assert_eq!((e.handed(a)[1], e.handed(b)[1]), (0.25, -0.5));
        e.event(Event::Timbre(-0.75));
        assert_eq!((e.handed(a)[1], e.handed(b)[1]), (-0.75, -0.75));
        e.event(Event::Timbre(0.5));
        assert_eq!(e.handed(b)[1], -0.5, "as far: the note's own");
        for (own, shared) in [(0.0f32, 0.3f32), (0.6, 0.3), (0.3, 0.3), (1.0, 0.0)] {
            assert_eq!(farther(own, shared), own.max(shared));
        }
    }

    /// The pressure's reach on the MODULATION wheel moves its own voice's wheel only, as far
    /// as a keyboard's wheel would at full; turned back to 0, the voice's wheel is the
    /// panel's again.
    #[test]
    fn pressure_moves_its_own_voices_modulation_wheel() {
        let mut c = Controls {
            reach: Reach {
                pressure_mod: 1.0,
                ..Reach::default()
            },
            ..poly(4)
        };
        let mut e = engine(&c);
        on(&mut e, 48, x(1.0, 0.0, 0.0));
        on(&mut e, 55, Expression::NONE);
        block(&mut e);
        let (a, b) = (slot_of(&e, 48).unwrap(), slot_of(&e, 55).unwrap());
        let wheel = |e: &Engine, k: usize| e.slots[k].play.lock().unwrap().voice.panel.mod_wheel;
        assert_eq!(wheel(&e, a), 1.0);
        assert_eq!(wheel(&e, b), 0.0);
        // With the keyboard's wheel half up, half pressure adds to it.
        e.event(Event::Modulation(0.25));
        e.event(Event::NoteExpression {
            key: 48,
            expression: x(0.25, 0.0, 0.0),
        });
        for _ in 0..30 {
            block(&mut e);
        }
        assert_eq!(
            wheel(&e, a),
            ca72::modulation::midi_wheel(0.25 + f64::from(0.25f32))
        );
        assert_eq!(wheel(&e, b), ca72::modulation::midi_wheel(0.25));
        c.reach = Reach::default();
        e.set(&c);
        block(&mut e);
        assert_eq!(wheel(&e, a), ca72::modulation::midi_wheel(0.25));
    }

    /// A note's own bend moves its own voice's pitch, by its semitones (an octave up doubles
    /// it), at LO and HI; the PITCH wheel's bus, and so the bend, reaches oscillator 3 only with
    /// OSC. 3 CONTROL on.
    #[test]
    fn a_notes_own_bend_moves_its_voice_by_its_semitones() {
        let crossings = |potato: bool, bend: f32| {
            let mut c = Controls { potato, ..poly(2) };
            c.panel.osc[0].waveform = ca72::voice::Waveform::Sawtooth;
            c.panel.loudness_contour.sustain = 1.0;
            c.panel.cutoff = 1.0;
            let mut e = engine(&c);
            on(&mut e, 57, x(0.0, 0.0, bend));
            let k = slot_of(&e, 57).unwrap();
            // (Blocks of one run each: the voice's every sample.)
            let (mut l, mut r) = ([0.0f32; CHUNK], [0.0f32; CHUNK]);
            let mut v = Vec::new();
            for i in 0..400 {
                e.render(&[], &mut l, &mut r);
                e.end_block(CHUNK);
                if i >= 25 {
                    let p = e.slots[k].play.lock().unwrap();
                    v.extend_from_slice(&p.out[0]);
                }
            }
            let mean = v.iter().sum::<f64>() / v.len() as f64;
            v.windows(2).filter(|w| w[0] < mean && w[1] >= mean).count() as f64
        };
        for potato in [true, false] {
            let (plain, up) = (crossings(potato, 0.0), crossings(potato, 12.0));
            assert!((up / plain - 2.0).abs() < 0.05, "LO {potato}: {plain} {up}");
        }
        let mut j = Jacks::default();
        express(&mut j, Some((150.0, 0.0)), false);
        assert_eq!(j.detune, [150.0, 150.0, 0.0]);
        express(&mut j, Some((-150.0, 1.5)), true);
        assert_eq!(j.detune, [0.0, 0.0, -150.0]);
        assert_eq!(j.cutoff, 1.5);
        express(&mut j, None, true);
        assert_eq!((j.detune, j.cutoff), ([0.0, 0.0, -150.0], 1.5));
    }

    /// The expression itself never wakes the one instrument asleep (it has no key down to be
    /// heard), and values out of range or not finite are kept in range.
    #[test]
    fn expression_is_kept_in_range_and_does_not_wake_a_sleeping_instrument() {
        let mut c = Controls::default();
        c.panel.decay = false;
        let mut e = engine(&c);
        on(&mut e, 60, Expression::NONE);
        block(&mut e);
        off(&mut e, 60);
        let mut asleep = false;
        for _ in 0..400 {
            block(&mut e);
            if e.asleep() {
                asleep = true;
                break;
            }
        }
        assert!(asleep);
        e.event(Event::NoteExpression {
            key: 60,
            expression: x(f32::NAN, 7.0, 1e9),
        });
        e.event(Event::ChannelPressure(f32::INFINITY));
        e.event(Event::Timbre(-2.0));
        assert!(e.asleep());
        assert_eq!(e.keys[60], x(0.0, 1.0, BEND_MOST));
        // (A timbre runs -1..1 about its rest, R-MPE.)
        assert_eq!((e.pressure, e.timbre), (0.0, -1.0));
    }
}

#[cfg(test)]
#[path = "engine_lfo_tests.rs"]
mod lfo_tests;

#[cfg(test)]
#[path = "engine_lfo_cross_tests.rs"]
mod lfo_cross_tests;

#[cfg(test)]
#[path = "engine_velocity_tests.rs"]
mod velocity_tests;

/// The most points a block keeps for the effects' pins: one each [`crate::lfo::STEP`] samples
/// of an 8192-sample block.
pub const FX_POINTS: usize = 256;

/// A block's points for the effects' pins (decisions.md R-FX-PARAMS): at each step of the LFOs,
/// its frame in the block and each pin's offset on its travel (the sum of depth × source), and
/// which pins some source moves. Fixed in size: nothing is allocated on the audio thread.
#[derive(Clone, Debug)]
pub struct FxPoints {
    pub n: usize,
    pub moved: u32,
    pub at: [u32; FX_POINTS],
    pub offsets: [[f32; crate::lfo::FX_PINS]; FX_POINTS],
}

impl Default for FxPoints {
    fn default() -> Self {
        FxPoints {
            n: 0,
            moved: 0,
            at: [0; FX_POINTS],
            offsets: [[0.0; crate::lfo::FX_PINS]; FX_POINTS],
        }
    }
}

impl FxPoints {
    fn clear(&mut self) {
        self.n = 0;
        self.moved = 0;
    }

    fn push(&mut self, moved: u32, at: u32, offsets: [f32; crate::lfo::FX_PINS]) {
        self.moved |= moved;
        if self.n < FX_POINTS {
            self.at[self.n] = at;
            self.offsets[self.n] = offsets;
            self.n += 1;
        }
    }
}
