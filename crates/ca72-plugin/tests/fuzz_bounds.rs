//! Every control at either end at once, as clap-validator's `param-fuzz-bounds` sets them,
//! with notes over the whole MIDI range: the engine plays on without panicking and its
//! output stays finite. POLY, VOICES, ENTROPY and SPREAD are among the controls.
//!
//! It found two faults (decisions.md R7): a key far beyond the 44 with an oscillator's
//! RANGE, FREQUENCY and the PITCH wheel high drove the converter's solve to NaN, and a POLY
//! voice resumed after resting with DECAY moved to its end sent the decay's Newton steps
//! below the -10 V rail (a panic on the audio thread). `FUZZ_SEED` and `FUZZ_ROUNDS` (72,
//! 40) run it wider.

#![allow(clippy::unwrap_used)]

use ca72::tuning::Range;
use ca72::voice::{Panel, Waveform};
use ca72_plugin::character::Placement;
use ca72_plugin::engine::{BEND_MOST, Controls, Engine, Event, Expression, Reach};

/// A small generator, the same sequence on every run.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
    fn end(&mut self) -> bool {
        self.next() & 1 == 1
    }
    fn either(&mut self, lo: f64, hi: f64) -> f64 {
        if self.end() { hi } else { lo }
    }
}

fn at_the_ends(r: &mut Lcg) -> Controls {
    let mut p = Panel::default();
    for o in &mut p.osc {
        o.range = if r.end() { Range::Lo } else { Range::R2 };
        o.waveform = if r.end() {
            Waveform::Triangle
        } else {
            Waveform::Square
        };
        o.on = r.end();
        o.volume = r.either(0.0, 1.0);
        o.freq = r.either(0.0, 1.0);
    }
    p.osc3_control = r.end();
    p.cutoff = r.either(0.0, 1.0);
    p.emphasis = r.either(0.0, 1.0);
    p.contour_amount = r.either(0.0, 1.0);
    p.keyboard_control_1 = r.end();
    p.keyboard_control_2 = r.end();
    for c in [&mut p.filter_contour, &mut p.loudness_contour] {
        c.attack = r.either(0.0, 1.0);
        c.decay = r.either(0.0, 1.0);
        c.sustain = r.either(0.0, 1.0);
    }
    p.glide = r.either(0.0, 1.0);
    p.glide_on = r.end();
    p.decay = r.end();
    p.noise_on = r.end();
    p.noise_volume = r.either(0.0, 1.0);
    p.noise_pink = r.end();
    p.mod_mix = r.either(0.0, 1.0);
    p.osc_mod = r.end();
    p.filter_mod = r.end();
    {
        use ca72::vcf::FilterMode as M;
        p.filter_mode =
            [M::Lo, M::Hi, M::Band, M::High][usize::from(r.end()) * 2 + usize::from(r.end())];
    }
    // (FILTER POLES at its ends.)
    p.filter_poles = if r.end() { 1 } else { 4 };
    p.pitch_wheel = r.either(-1.0, 1.0);
    p.mod_wheel = r.either(0.0, 1.0);
    p.ext_volume = r.either(0.0, 1.0);
    p.ext_on = r.end();
    p.a440 = r.end();
    p.tune = r.either(-1.0, 1.0);
    Controls {
        panel: p,
        volume: r.either(0.0, 1.0),
        main_output: r.end(),
        bend_range: r.either(0.0, 24.0),
        power: r.end(),
        poly: r.end(),
        voices: if r.end() { 10 } else { 2 },
        entropy: r.either(0.0, 1.0),
        spread: r.either(0.0, 1.0),
        inner: r.either(0.0, 1.0),
        // (The placement's ends: the plug-in's two choices.)
        placement: if r.end() {
            Placement::Centre
        } else {
            Placement::Even
        },
        unison: r.end(),
        double: r.either(0.0, 1.0),
        drive: r.either(0.0, 24.0),
        level: r.either(-30.0, 12.0),
        auto_gain: r.end(),
        feedback: r.either(0.0, 1.0),
        lock: r.end(),
        potato: r.end(),
        ultra: r.end(),
        reach: Reach::default(),
        // (Oscillators 2 and 3 timed at their slowest and fastest divisions, or not; 1 never.)
        timed: std::array::from_fn(|n| {
            (n > 0 && r.end()).then(|| {
                if r.end() {
                    ca72_plugin::timed::Length::Bars(4.0)
                } else {
                    ca72_plugin::timed::Length::Beats(0.125)
                }
            })
        }),
        // (No LFO: the fuzz's sequence as it was; `lfos_at_the_ends` adds them.)
        lfo: Default::default(),
    }
}

/// The expression's reaches at their ends (R-MPE), from a generator of their own, so that
/// the controls' sequence is as it was.
fn reach_at_the_ends(x: &mut Lcg) -> Reach {
    Reach {
        pressure_mod: x.either(-1.0, 1.0),
        pressure_filter: x.either(-4.0, 4.0),
        timbre_mod: x.either(-1.0, 1.0),
        timbre_filter: x.either(-4.0, 4.0),
    }
}

#[test]
fn every_control_at_its_ends_plays_on() {
    let seed = std::env::var("FUZZ_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(72);
    let rounds = std::env::var("FUZZ_ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(40);
    let mut r = Lcg(seed);
    let mut x = Lcg(seed ^ 0x81);
    for rate in [44_100.0, 48_000.0] {
        let mut e = Engine::new();
        e.set(&at_the_ends(&mut r));
        e.prepare(rate, 7);
        for round in 0..rounds {
            let mut c = at_the_ends(&mut r);
            c.reach = reach_at_the_ends(&mut x);
            e.set(&c);
            for _ in 0..4 {
                let key = (r.next() % 128) as u8;
                // (Each note's own expression at its ends too, its bend as far as MPE's goes.)
                let expression = Expression {
                    pressure: x.either(0.0, 1.0) as f32,
                    timbre: x.either(0.0, 1.0) as f32,
                    bend: x.either(-f64::from(BEND_MOST), f64::from(BEND_MOST)) as f32,
                };
                e.event(Event::NoteExpression { key, expression });
                e.event(Event::ChannelPressure(x.either(0.0, 1.0) as f32));
                e.event(Event::Note { key, on: true });
                let off = r.end().then(|| (r.next() % 128) as u8);
                if let Some(k) = off {
                    e.event(Event::Note { key: k, on: false });
                }
                for i in 0..512 {
                    let ext = if c.panel.ext_on { 0.5 } else { 0.0 };
                    let (left, right) =
                        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            e.tick_stereo(ext)
                        })) {
                            Ok(x) => x,
                            Err(_) => {
                                panic!("round {round} at {rate} Hz, sample {i}, key {key}: {c:#?}")
                            }
                        };
                    assert!(
                        left.is_finite() && right.is_finite(),
                        "round {round} at {rate} Hz, sample {i}: {left} {right} with {c:?}"
                    );
                }
                e.end_block(512);
            }
            e.event(Event::AllNotesOff);
        }
    }
}
