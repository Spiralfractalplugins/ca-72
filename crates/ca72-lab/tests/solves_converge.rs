//! Every iterative solve that should converge does, in every quality mode, under the worst
//! case's load (every knob, key and jack moving): none stops at its cap
//! (`ca72::unconverged`). No Compromises' contours once stopped short hundreds of
//! times a few seconds, their transistors' steps held too short to arrive.
//!
//! V-trig's root once stopped short at a release with SUSTAIN high: 21 times in a minute of
//! the worst case, 70 over the gate trains below and twice in perf-ext, in No Compromises and
//! High Fidelity alike, and once in Potato (decisions.md R-CONTOUR-VTRIG). The whole of those
//! loads takes minutes, so the tests that run by themselves reach the first of the stops, and
//! `cargo test --release -p ca72-lab --test solves_converge -- --ignored` runs the rest.

use ca72::contour::{ContourCircuit, Contours, Controls, Panel as ContourPanel};
use ca72::unconverged;
use ca72::voice::{ContourKnobs, Panel, Quality, Voice};
use ca72_lab::worst;

const MODES: [Quality; 3] = [
    Quality::NoCompromises,
    Quality::HighFidelity,
    Quality::Potato,
];

/// The worst case's load for each mode's `seconds`: what stopped short, by mode.
fn stopped_in_the_worst_load(modes: [(Quality, f64); 3]) -> Vec<String> {
    let rate = 48_000.0;
    let mut stopped = Vec::new();
    for (q, seconds) in modes {
        let mut v = Voice::prototype(rate);
        unconverged::take();
        worst::render_loop(&mut v, rate, seconds, q, false);
        let counts = unconverged::take();
        println!("{q:?}: {counts:?}");
        if !counts.is_empty() || v.keyboard().failed > 0 || v.revsaw().failed > 0 {
            stopped.push(format!(
                "{q:?}: {counts:?}, keyboard {}, reverse sawtooth {}",
                v.keyboard().failed,
                v.revsaw().failed
            ));
        }
    }
    stopped
}

/// (V-trig's first stops were at 4.96 s in No Compromises and High Fidelity, 33.96 s in Potato.)
#[test]
fn every_solve_converges_in_every_mode() {
    let stopped = stopped_in_the_worst_load([
        (Quality::NoCompromises, 6.0),
        (Quality::HighFidelity, 6.0),
        (Quality::Potato, 35.0),
    ]);
    assert!(stopped.is_empty(), "{stopped:?}");
}

#[test]
#[ignore = "minutes; by hand"]
fn every_solve_converges_over_a_minute_of_the_worst_case() {
    let stopped = stopped_in_the_worst_load(MODES.map(|q| (q, 60.0)));
    assert!(stopped.is_empty(), "{stopped:?}");
}

/// The case traced: DECAY off, the loudness contour's SUSTAIN up, the contours at 48 kHz's
/// sub-rate. V-trig takes a tick to fall from its high to 7.66 V, and the next starts from
/// there with CR2 [CR9] off: Newton's first step lands at 1.22 V, far up that diode's
/// exponential, and at a step of one n Vt from there 80 iterations ended at about 5.06 V, 43 mV
/// short, which stood on the contours for some 300 ms.
#[test]
fn a_release_from_sustain_high_finds_v_trig() {
    let c = |sustain: f64| Controls {
        attack: 0.0,
        decay: 20e3,
        sustain,
    };
    let panel = ContourPanel {
        filter: c(0.0),
        loudness: c(1.0),
        decay_on: false,
        s_trig: false,
    };
    let rate = 24_000.0;
    for q in [Quality::NoCompromises, Quality::HighFidelity] {
        let mut contours = Contours::new(ContourCircuit::default(), rate);
        contours.settle(&panel);
        contours.set_quality(q);
        for _ in 0..(0.05 * rate) as usize {
            contours.tick(true, &panel);
        }
        unconverged::take();
        let mut started = Vec::new();
        for _ in 0..(0.05 * rate) as usize {
            let before = contours.trigger_nodes()[2];
            contours.tick(false, &panel);
            if before - contours.trigger_nodes()[2] > 1.0 {
                started.push(before);
            }
        }
        let counts = unconverged::take();
        assert!(
            started.iter().any(|v| (v - 7.66).abs() < 0.01),
            "{q:?}: V-trig fell from {started:?}, not the case traced"
        );
        assert!(counts.is_empty(), "{q:?}: {counts:?}");
    }
}

/// perf-ext's contours, the scenario `ca72-lab hifi` failed on: two notes and their releases
/// (the patch's panel; the external input it plays does not reach the contours).
#[test]
fn perf_ext_converges() {
    let rate = 48_000.0;
    let mut panel = Panel {
        ext_on: true,
        ext_volume: 0.85,
        cutoff: 0.6,
        emphasis: 0.3,
        ..Panel::default()
    };
    panel.osc[0].on = false;
    panel.loudness_contour.sustain = 1.0;
    panel.filter_contour.sustain = 0.6;
    let events = [
        (0.1, 45, true),
        (1.6, 45, false),
        (2.0, 52, true),
        (3.5, 52, false),
    ];
    for q in MODES {
        let mut v = Voice::new(
            rate,
            Panel {
                quality: q,
                ..panel
            },
        );
        unconverged::take();
        for i in 0..(4.0 * rate) as usize {
            for &(t, key, on) in &events {
                if i == (t * rate) as usize {
                    v.note(key, on);
                }
            }
            v.tick();
        }
        let counts = unconverged::take();
        assert!(counts.is_empty(), "{q:?}: {counts:?}");
    }
}

/// What stopped short over fast gates, the triggers' hardest use: each (DECAY, SUSTAIN, DECAY's
/// knob) of `panels` with each time on and off below (s), a second and a bit of each.
fn stopped_in_gate_trains(panels: &[(bool, f64, f64)]) -> Vec<String> {
    let rate = 48_000.0;
    let gates = [
        (0.005, 0.005),
        (0.010, 0.010),
        (0.020, 0.005),
        (0.020, 0.011),
        (0.020, 0.013),
        (0.020, 0.020),
        (0.050, 0.008),
        (0.050, 0.011),
        (0.050, 0.013),
        (0.050, 0.020),
        (0.100, 0.050),
    ];
    let mut stopped = Vec::new();
    for q in MODES {
        for &(decay, sustain, knob) in panels {
            for (on, off) in gates {
                let contour = ContourKnobs {
                    attack: 0.0,
                    decay: knob,
                    sustain,
                };
                let panel = Panel {
                    decay,
                    loudness_contour: contour,
                    filter_contour: contour,
                    quality: q,
                    ..Panel::default()
                };
                let mut v = Voice::new(rate, panel);
                let (on, period) = ((on * rate) as usize, ((on + off) * rate) as usize);
                let start = (0.2 * rate) as usize;
                let mut held = false;
                unconverged::take();
                for i in 0..(1.2 * rate) as usize {
                    let gate = i >= start && (i - start) % period < on;
                    if gate != held {
                        v.note(60, gate);
                        held = gate;
                    }
                    v.tick();
                }
                let counts = unconverged::take();
                if !counts.is_empty() {
                    stopped.push(format!(
                        "{q:?}, DECAY {decay}, SUSTAIN {sustain}, {knob}, {on}/{period}: {counts:?}"
                    ));
                }
            }
        }
    }
    stopped
}

/// (The panel where the stops were: DECAY off, SUSTAIN up.)
#[test]
fn gate_trains_converge() {
    let stopped = stopped_in_gate_trains(&[(false, 1.0, 0.0)]);
    assert!(stopped.is_empty(), "{stopped:?}");
}

#[test]
#[ignore = "minutes; by hand"]
fn gate_trains_converge_on_every_panel() {
    let stopped = stopped_in_gate_trains(&[
        (true, 0.5, 0.4),
        (true, 0.0, 0.3),
        (false, 1.0, 0.0),
        (false, 0.5, 0.4),
        (false, 1.0, 0.8),
    ]);
    assert!(stopped.is_empty(), "{stopped:?}");
}
