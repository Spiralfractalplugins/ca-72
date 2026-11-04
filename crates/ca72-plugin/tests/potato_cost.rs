//! What Potato mode's voice costs by what it plays (decisions.md R31): `ca72::light::Light`
//! alone at 48 kHz, oscillators 1 to 3 on at EMPHASIS 4, a key held, then the noise, each
//! modulation, both at MODULATION MIX 5, the external input and FEEDBACK, DRIVE, FILTER
//! MODE HI, and everything at once: nanoseconds a sample on this machine's clock, the least
//! of `REPS` (default 7) plays of two seconds (the least, so that the machine's other work
//! counts as little as it can).
//!
//! By hand: `cargo test --release -p ca72-plugin --test potato_cost -- --ignored
//! --nocapture` (`ONLY`, one case by its name).

#![allow(clippy::unwrap_used)]

use ca72::light::Light;
use ca72::voice::Panel;
use std::time::Instant;

/// A case: its name, what it sets on the panel, FEEDBACK, and a tone at EXTERNAL INPUT or not.
type Case = (&'static str, Box<dyn Fn(&mut Panel)>, f64, bool);

/// Nanoseconds a sample: the least over the plays.
fn cost(f: &dyn Fn(&mut Panel), feedback: f64, ext: bool) -> f64 {
    let mut l = Light::new(48_000.0);
    let mut p = Panel::default();
    p.osc[1].on = true;
    p.osc[2].on = true;
    p.emphasis = 0.4;
    f(&mut p);
    l.set_panel(&p);
    l.feedback = feedback;
    l.note(48, true);
    let mut sink = 0.0;
    for _ in 0..48_000 {
        sink += l.tick(0.0);
    }
    let x = if ext { 0.1 } else { 0.0 };
    let reps: usize = std::env::var("REPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(7);
    let mut least = f64::MAX;
    for _ in 0..reps {
        let clock = Instant::now();
        for _ in 0..96_000 {
            sink += l.tick(std::hint::black_box(x));
        }
        least = least.min(clock.elapsed().as_secs_f64() * 1e9 / 96_000.0);
    }
    std::hint::black_box(sink);
    least
}

#[test]
#[ignore = "a measurement, run by hand"]
fn the_light_voice_by_what_it_plays() {
    let cases: Vec<Case> = vec![
        ("base", Box::new(|_| {}), 0.0, false),
        ("noise", Box::new(|p| p.noise_on = true), 0.0, false),
        (
            "osc mod",
            Box::new(|p| {
                p.osc_mod = true;
                p.mod_wheel = 0.6;
            }),
            0.0,
            false,
        ),
        (
            "filter mod",
            Box::new(|p| {
                p.filter_mod = true;
                p.mod_wheel = 0.6;
            }),
            0.0,
            false,
        ),
        (
            "mods, mix .5",
            Box::new(|p| {
                p.osc_mod = true;
                p.filter_mod = true;
                p.mod_wheel = 0.6;
                p.mod_mix = 0.5;
            }),
            0.0,
            false,
        ),
        ("ext on", Box::new(|p| p.ext_on = true), 0.0, true),
        ("feedback", Box::new(|p| p.ext_on = true), 0.5, false),
        (
            "filter HI",
            Box::new(|p| p.filter_mode = ca72::vcf::FilterMode::Hi),
            0.0,
            false,
        ),
        (
            "everything",
            Box::new(|p| {
                p.noise_on = true;
                p.osc_mod = true;
                p.filter_mod = true;
                p.mod_wheel = 0.6;
                p.mod_mix = 0.5;
                p.ext_on = true;
            }),
            0.5,
            false,
        ),
    ];
    let only = std::env::var("ONLY").ok();
    for (n, f, fb, ext) in cases {
        if only.as_ref().is_some_and(|o| o != n) {
            continue;
        }
        let ns = cost(&*f, fb, ext);
        eprintln!(
            "{n:<14} {ns:7.1} ns a sample ({:.2} % of a core at 48 kHz)",
            ns * 48e3 / 1e7
        );
    }
}
