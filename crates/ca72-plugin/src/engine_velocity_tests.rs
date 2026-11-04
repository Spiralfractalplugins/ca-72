//! The velocity in the engine (decisions.md R-VELOCITY): nothing assigned, a note's velocity
//! changes nothing, to the bit; assigned, each POLY voice takes its own note's at its attack,
//! from the attack's first sample; the one instrument the key NOTE PRIORITY sounds, at once for a
//! new phrase, gliding to a legato key's; the controls every voice shares the latest note's;
//! CURVE and FLOOR between.

use super::*;
use crate::lfo::{self, Assignment, Response, Target};

const RATE: f64 = 48_000.0;

fn controls() -> Controls {
    Controls {
        entropy: 0.0,
        potato: true,
        ..Controls::default()
    }
}

/// `c` with the velocity moving `t` by `depth` from travel `base`.
fn with(mut c: Controls, t: Target, depth: f32, base: f32) -> Controls {
    let slot = c.lfo.to[lfo::VELOCITY]
        .iter_mut()
        .find(|a| a.is_none())
        .expect("room");
    *slot = Some(Assignment { target: t, depth });
    c.lfo.base[t.index()] = base;
    c
}

fn engine(c: &Controls) -> Engine {
    let mut e = Engine::new();
    e.set(c);
    e.prepare(RATE, 3);
    e
}

fn run(e: &mut Engine, n: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(n);
    let (mut l, mut r) = ([0.0f32; 256], [0.0f32; 256]);
    let mut left = n;
    while left > 0 {
        let k = left.min(256);
        e.render(&[], &mut l[..k], &mut r[..k]);
        e.end_block(k);
        out.extend_from_slice(&l[..k]);
        left -= k;
    }
    out
}

/// Key `key` struck at `velocity` (or let go).
fn strike(e: &mut Engine, key: u8, velocity: f32) {
    e.event(Event::NoteVelocity { key, velocity });
    e.event(Event::Note { key, on: true });
}

fn lift(e: &mut Engine, key: u8) {
    e.event(Event::Note { key, on: false });
}

fn panel(e: &Engine, k: usize) -> Panel {
    e.slots[k].play.try_lock().expect("free").voice.panel
}

fn slot_of(e: &Engine, key: u8) -> usize {
    e.slots
        .iter()
        .position(|s| s.gate && s.note == key)
        .expect("a voice")
}

/// CUTOFF where travel `travel` puts it.
fn cutoff_at(c: &Controls, travel: f64) -> f64 {
    let mut m = *c;
    Target::Cutoff.write(&mut m, travel, c.lfo.law.into());
    m.panel.cutoff
}

/// With nothing assigned, or a depth of 0, the notes' velocities change nothing: the samples
/// are the same to the bit as with no velocity told at all, the one instrument and POLY.
#[test]
fn nothing_assigned_plays_as_before_to_the_bit() {
    for poly in [false, true] {
        let base = Controls {
            poly,
            voices: 3,
            ..controls()
        };
        let zero = with(base, Target::Cutoff, 0.0, 0.5);
        let play = |c: &Controls, told: bool| {
            let mut e = engine(c);
            let mut out = Vec::new();
            for (k, v) in [(60u8, 0.1f32), (64, 0.9), (67, 0.4)] {
                if told {
                    strike(&mut e, k, v);
                } else {
                    e.event(Event::Note { key: k, on: true });
                }
                out.extend(run(&mut e, 2400));
            }
            lift(&mut e, 64);
            out.extend(run(&mut e, 4800));
            out
        };
        let before = play(&base, false);
        assert_eq!(play(&base, true), before, "poly {poly}: velocities told");
        assert_eq!(play(&zero, true), before, "poly {poly}: a depth of 0");
    }
}

/// Each POLY voice takes its own note's velocity, from its attack's first sample.
#[test]
fn each_poly_voice_takes_its_notes_velocity() {
    let c = with(
        Controls {
            poly: true,
            voices: 3,
            ..controls()
        },
        Target::Cutoff,
        0.5,
        0.2,
    );
    let mut e = engine(&c);
    strike(&mut e, 60, 0.2);
    run(&mut e, 1);
    let soft = slot_of(&e, 60);
    assert!((panel(&e, soft).cutoff - cutoff_at(&c, 0.3)).abs() < 1e-6);
    strike(&mut e, 64, 1.0);
    run(&mut e, 1);
    let hard = slot_of(&e, 64);
    assert_ne!(soft, hard);
    assert!((panel(&e, hard).cutoff - cutoff_at(&c, 0.7)).abs() < 1e-6);
    // (The first voice keeps its own as it plays on.)
    run(&mut e, 4800);
    assert!((panel(&e, soft).cutoff - cutoff_at(&c, 0.3)).abs() < 1e-6);
    assert!((panel(&e, hard).cutoff - cutoff_at(&c, 0.7)).abs() < 1e-6);
}

/// The one instrument takes the key NOTE PRIORITY sounds: a new phrase's at once, a legato
/// key's over the expression's glide (4 ms), and back to the held key's when that one lets go;
/// with LOW, a higher key pressed under a phrase does not sound, and its velocity is not taken.
#[test]
fn the_one_instrument_glides_to_a_legato_keys_velocity() {
    // (NOTE PRIORITY's LAST, so that the legato key sounds.)
    let mut c = with(controls(), Target::Cutoff, 0.5, 0.2);
    c.panel.priority = Priority::Last;
    let mut e = engine(&c);
    strike(&mut e, 60, 0.2);
    run(&mut e, 1);
    assert_eq!(
        e.slots[0].velocity.now,
        f64::from(0.2f32),
        "a phrase at once"
    );
    strike(&mut e, 64, 1.0);
    run(&mut e, lfo::STEP);
    let mid = e.slots[0].velocity.now;
    assert!(mid > 0.3 && mid < 0.99, "gliding: {mid}");
    run(&mut e, (0.04 * RATE) as usize);
    assert!((e.slots[0].velocity.now - 1.0).abs() < 1e-3);
    assert!((panel(&e, 0).cutoff - cutoff_at(&c, 0.7)).abs() < 1e-3);
    lift(&mut e, 64);
    run(&mut e, (0.04 * RATE) as usize);
    assert!((e.slots[0].velocity.now - f64::from(0.2f32)).abs() < 1e-3);
    // (LOW: the higher key never sounds.)
    c.panel.priority = Priority::Low;
    let mut e = engine(&c);
    strike(&mut e, 60, 0.2);
    strike(&mut e, 64, 1.0);
    run(&mut e, (0.04 * RATE) as usize);
    assert_eq!(e.slots[0].velocity.now, f64::from(0.2f32));
}

/// A control every voice shares (VOLUME) takes the latest note's velocity.
#[test]
fn shared_controls_take_the_latest_notes_velocity() {
    let c = with(
        Controls {
            poly: true,
            voices: 3,
            ..controls()
        },
        Target::Volume,
        0.4,
        0.3,
    );
    let law = c.lfo.law.into();
    let want = |v: f64| {
        let mut m = c;
        Target::Volume.write(&mut m, 0.3 + 0.4 * v, law);
        m.volume
    };
    let mut e = engine(&c);
    strike(&mut e, 60, 0.25);
    run(&mut e, 1);
    assert!((e.controls.volume - want(f64::from(0.25f32))).abs() < 1e-6);
    strike(&mut e, 64, 0.75);
    run(&mut e, 1);
    assert!((e.controls.volume - want(f64::from(0.75f32))).abs() < 1e-6);
}

/// CURVE and FLOOR shape what a note's velocity gives.
#[test]
fn curve_and_floor_shape_the_value() {
    let mut c = with(
        Controls {
            poly: true,
            voices: 2,
            ..controls()
        },
        Target::Cutoff,
        0.5,
        0.2,
    );
    c.lfo.velocity = Response {
        curve: 1.0,
        floor: 0.2,
    };
    let mut e = engine(&c);
    strike(&mut e, 60, 0.5);
    run(&mut e, 1);
    let v = c.lfo.velocity.at(f64::from(0.5f32));
    assert!((v - (0.2 + 0.8 * 0.5f64.powi(3))).abs() < 1e-6, "{v}");
    let k = slot_of(&e, 60);
    assert!((panel(&e, k).cutoff - cutoff_at(&c, 0.2 + 0.5 * v)).abs() < 1e-6);
}

/// A velocity told never wakes the sleeping instrument, nor plays a note.
#[test]
fn a_velocity_alone_changes_nothing() {
    let c = with(controls(), Target::Cutoff, 0.5, 0.2);
    let mut e = engine(&c);
    let before = run(&mut e, 2400);
    e.event(Event::NoteVelocity {
        key: 60,
        velocity: 0.5,
    });
    let mut f = engine(&c);
    run(&mut f, 2400);
    assert_eq!(run(&mut e, 2400), run(&mut f, 2400));
    assert!(before.iter().all(|s| s.abs() < 1e-3));
    assert_eq!(e.last_velocity(), None, "no note yet");
    strike(&mut e, 60, 0.5);
    assert_eq!(e.last_velocity(), Some(0.5));
}

/// The velocity's assignments are the patch's: saved in a preset as `velocity_to_<target>` and
/// in a session as any source's, and read back.
#[test]
fn the_velocitys_assignments_are_saved_with_the_patch() {
    use crate::lfo::{Patch, Source};
    let p = Patch::default();
    p.assign(Source::Velocity, Target::Cutoff, 0.4)
        .expect("room");
    p.assign(Source::Velocity, Target::Volume, -0.25)
        .expect("room");
    let values = p.values();
    assert!(
        values.contains(&("velocity_to_cutoff".into(), 40.0)),
        "{values:?}"
    );
    let q = Patch::default();
    q.load_values(&values);
    assert_eq!(q.depth(Source::Velocity, Target::Cutoff), Some(0.4));
    assert_eq!(q.depth(Source::Velocity, Target::Volume), Some(-0.25));
    let r = Patch::default();
    r.load(&p.saved());
    assert_eq!(
        r.assignments(Source::Velocity),
        p.assignments(Source::Velocity)
    );
    assert_eq!(Source::of_id("velocity"), Some(Source::Velocity));
    assert_eq!(Source::of_index(lfo::VELOCITY), Some(Source::Velocity));
}

/// The velocity moves neither an LFO's own knobs nor TEMPO (they run on the generators' ticks):
/// such an assignment, if one is ever made, is kept and never applied.
#[test]
fn the_velocity_never_moves_an_lfos_knobs_or_tempo() {
    use crate::lfo::Source;
    assert!(!Source::Velocity.takes(Target::LfoRate(0)));
    assert!(!Source::Velocity.takes(Target::LfoLevel(3)));
    assert!(!Source::Velocity.takes(Target::Tempo));
    assert!(Source::Velocity.takes(Target::Cutoff));
    assert!(Source::Lfo(0).takes(Target::LfoRate(1)));
    let p = crate::params::Ca72Params::default();
    p.lfo_patch
        .assign(Source::Velocity, Target::LfoRate(0), 0.5)
        .expect("room");
    p.lfo_patch
        .assign(Source::Velocity, Target::Cutoff, 0.5)
        .expect("room");
    let l = p.controls().lfo;
    let applied: Vec<_> = l.to[lfo::VELOCITY]
        .iter()
        .flatten()
        .map(|a| a.target)
        .collect();
    assert_eq!(applied, [Target::Cutoff]);
    assert!(!l.cross(), "no LFO's knob moved");
    assert_eq!(
        p.lfo_patch.depth(Source::Velocity, Target::LfoRate(0)),
        Some(0.5),
        "kept"
    );
}

/// CURVE and FLOOR are the last parameters, a straight line from 0 by default, and reach the
/// engine only while the velocity moves something.
#[test]
fn curve_and_floor_keep_their_positions_and_reach_the_engine_only_when_assigned() {
    use crate::lfo::Source;
    use nih_plug::prelude::Params;
    let p = crate::params::Ca72Params::default();
    let ids: Vec<String> = p.param_map().into_iter().map(|(id, ..)| id).collect();
    let n = ids.len();
    assert_eq!(
        &ids[n - 3..],
        ["velocity_curve", "velocity_floor", "filter_shape"]
    );
    assert_eq!(p.velocity_curve.value(), 0.0);
    assert_eq!(p.velocity_floor.value(), 0.0);
    let c = p.controls_with(&|k| {
        if std::ptr::eq(k, &p.velocity_curve) {
            -60.0
        } else {
            k.value()
        }
    });
    assert_eq!(c.lfo.velocity, Response::default(), "nothing assigned");
    p.lfo_patch
        .assign(Source::Velocity, Target::Cutoff, 0.5)
        .expect("room");
    let r = p.controls().lfo.velocity;
    assert_eq!(r, Response::default(), "a straight line from 0");
    assert_eq!(r.at(0.3), 0.3);
}
