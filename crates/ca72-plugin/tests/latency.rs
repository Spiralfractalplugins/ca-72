//! Every QUALITY mode's sound comes out with ULTRA's, and the plug-in reports one latency
//! (decisions.md R-LATENCY): the oscillators' sound and the attacks line up within half a
//! sample at 44.1, 48 and 96 kHz once HI's and LO's output is held back ([`LINE_UP`]), and the
//! engine's latency is the same in every mode. The measurements behind the figures run by
//! hand: `cargo test --release -p ca72-plugin --test latency -- --ignored --nocapture`.

#![allow(clippy::unwrap_used)]

use ca72::tuning::Range;
use ca72::voice::{LINE_UP, Quality, Voice, Waveform};
use ca72_plugin::engine::{Controls, Engine, Event};

const RATE: f64 = 48_000.0;
const BLOCK: usize = 256;

/// One sawtooth, 32', through the filter wide open; the contours' attack as short as it goes
/// and their sustain full; no noise, no external input unless `ext`.
fn patch(mode: &str, ext: bool) -> Controls {
    let mut c = Controls {
        potato: mode == "LO",
        ultra: mode == "ULTRA",
        entropy: 0.0,
        lock: true,
        ..Controls::default()
    };
    let p = &mut c.panel;
    for o in &mut p.osc {
        o.on = false;
    }
    if !ext {
        p.osc[0].on = true;
        p.osc[0].volume = 1.0;
        p.osc[0].waveform = Waveform::Sawtooth;
        p.osc[0].range = Range::R32;
    }
    p.noise_on = false;
    p.ext_on = ext;
    p.ext_volume = 1.0;
    p.cutoff = 1.0;
    p.emphasis = 0.0;
    p.contour_amount = 0.0;
    p.filter_contour.attack = 0.0;
    p.filter_contour.sustain = 1.0;
    p.loudness_contour.attack = 0.0;
    p.loudness_contour.sustain = 1.0;
    c
}

/// `seconds` of the engine's left output with a key pressed at `at` (samples) and held, and
/// `ext` at EXTERNAL INPUT.
fn play(c: &Controls, at: usize, seconds: f64, ext: &[f32]) -> Vec<f64> {
    let mut e = Engine::new();
    e.set(c);
    e.prepare(RATE, 1);
    let n = (seconds * RATE) as usize / BLOCK * BLOCK;
    let (mut l, mut r) = (vec![0.0f32; n], vec![0.0f32; n]);
    for b in (0..n).step_by(BLOCK) {
        let x: &[f32] = if ext.len() >= b + BLOCK {
            &ext[b..b + BLOCK]
        } else {
            &[]
        };
        if (b..b + BLOCK).contains(&at) {
            let k = at - b;
            let (x0, x1) = x.split_at(k.min(x.len()));
            e.render(x0, &mut l[b..at], &mut r[b..at]);
            e.event(Event::Note { key: 33, on: true });
            e.render(x1, &mut l[at..b + BLOCK], &mut r[at..b + BLOCK]);
        } else {
            e.render(x, &mut l[b..b + BLOCK], &mut r[b..b + BLOCK]);
        }
        e.end_block(BLOCK);
    }
    l.iter().map(|&v| f64::from(v)).collect()
}

/// The lag (samples, fractional) at which `b` best matches `a` delayed, within `reach`.
fn lag(a: &[f64], b: &[f64], reach: isize) -> f64 {
    let n = a.len() as isize;
    let score = |d: isize| -> f64 {
        let mut s = 0.0;
        for i in reach..n - reach {
            s += a[i as usize] * b[(i + d) as usize];
        }
        s
    };
    let scores: Vec<f64> = (-reach..=reach).map(score).collect();
    let k = (0..scores.len())
        .max_by(|&i, &j| scores[i].total_cmp(&scores[j]))
        .unwrap();
    let (y0, y1, y2) = (
        scores[k.saturating_sub(1)],
        scores[k],
        scores[(k + 1).min(scores.len() - 1)],
    );
    let frac = 0.5 * (y0 - y2) / (y0 - 2.0 * y1 + y2);
    k as f64 - reach as f64 + if frac.is_finite() { frac } else { 0.0 }
}

/// The first sample at which `x`'s magnitude reaches `share` of its peak after `from`.
fn onset(x: &[f64], from: usize, share: f64) -> f64 {
    onset_at(x, from, share * peak(x, from))
}

/// `x`'s largest magnitude after `from`.
fn peak(x: &[f64], from: usize) -> f64 {
    x[from..].iter().fold(0.0f64, |m, v| m.max(v.abs()))
}

/// The first sample at which `x`'s magnitude reaches `level` after `from`.
fn onset_at(x: &[f64], from: usize, level: f64) -> f64 {
    let i = (from..x.len()).find(|&i| x[i].abs() >= level).unwrap();
    // (Between the samples either side of the crossing.)
    let (a, b) = (x[i - 1].abs(), x[i].abs());
    (i - 1) as f64 + (level - a) / (b - a)
}

#[test]
#[ignore = "a measurement, run by hand"]
fn each_modes_latency() {
    let at = (0.25 * RATE) as usize;
    let modes = ["LO", "HI", "ULTRA"];
    let notes: Vec<Vec<f64>> = modes
        .iter()
        .map(|m| play(&patch(m, false), at, 1.5, &[]))
        .collect();
    // Onsets: a tenth and a half of the peak.
    for (m, x) in modes.iter().zip(&notes) {
        eprintln!(
            "{m:<6} onset at 10 % {:.2}, at 50 % {:.2} samples after the key",
            onset(x, at, 0.1) - at as f64,
            onset(x, at, 0.5) - at as f64
        );
    }
    // The waveform's lag behind ULTRA's, the note sustained.
    let seg = |x: &[f64]| x[(0.75 * RATE) as usize..(1.45 * RATE) as usize].to_vec();
    let u = seg(&notes[2]);
    for (m, x) in modes.iter().zip(&notes) {
        eprintln!(
            "{m:<6} waveform {:.2} samples behind ULTRA's",
            -lag(&seg(x), &u, 300)
        );
    }
    // An impulse at EXTERNAL INPUT a second in, a key held (the VCA open).
    let mut ext = vec![0.0f32; (1.5 * RATE) as usize];
    let t1 = RATE as usize;
    ext[t1] = 0.5;
    for m in modes {
        let y = play(&patch(m, true), at, 1.5, &ext);
        let w = &y[t1 - 200..t1 + 800];
        let k = (0..w.len())
            .max_by(|&i, &j| w[i].abs().total_cmp(&w[j].abs()))
            .unwrap();
        let (y0, y1, y2) = (w[k - 1].abs(), w[k].abs(), w[k + 1].abs());
        let peak = k as f64 - 200.0 + 0.5 * (y0 - y2) / (y0 - 2.0 * y1 + y2);
        eprintln!("{m:<6} EXTERNAL INPUT's impulse peaks {peak:.2} samples later");
    }
}

/// Each mode's oscillator path at `rate`: oscillator 1's sawtooth (16') through the filter
/// wide open, a key held; each of its resets' exact time (the oscillator's own account,
/// `probe_resets`) against where the output's jump is steepest. The mean over the resets
/// from half a second to a second and a half: samples.
fn oscillator_path(mode: &str, rate: f64) -> f64 {
    let mut c = patch("ULTRA", false);
    c.panel.osc[0].range = Range::R16;
    let n = (1.6 * rate) as usize;
    let mut y = Vec::with_capacity(n);
    let mut resets = Vec::new();
    let mut note = |tick: &mut dyn FnMut() -> (f64, (f64, u64))| {
        let mut last = 0;
        for i in 0..n {
            let (out, (t, count)) = tick();
            y.push(out);
            if count != last {
                last = count;
                resets.push((i, t));
            }
        }
    };
    if mode == "LO" {
        let mut l = ca72::light::Light::new(rate);
        l.set_panel(&c.panel);
        l.note(57, true);
        note(&mut || {
            let out = l.tick(0.0);
            (out, l.probe_resets())
        });
    } else {
        let mut v = Voice::new(rate, c.panel);
        v.panel.quality = if mode == "HI" {
            Quality::Potato
        } else {
            Quality::HighFidelity
        };
        v.note(57, true);
        let mut i = 0usize;
        note(&mut || {
            let out = v.tick();
            let (t, count) = v.probe_resets(0);
            i += 1;
            // (The voice's oscillator counts its own samples: the reset fell within this
            // sample's tick, `t` - ceil(`t`) of a sample before it.)
            (out, ((i - 1) as f64 + (t - t.ceil()), count))
        });
    }
    let mut sum = 0.0;
    let mut k = 0;
    for &(_, r) in resets
        .iter()
        .filter(|&&(_, r)| r > 0.5 * rate && r < 1.5 * rate)
    {
        // The jump's steepest point within 400 samples after the reset (the derivative's
        // peak between samples, interpolated).
        let from = r as usize;
        let d: Vec<f64> = (from..from + 400).map(|i| y[i + 1] - y[i]).collect();
        let j = (1..d.len() - 1)
            .max_by(|&a, &b| d[a].abs().total_cmp(&d[b].abs()))
            .unwrap();
        let (a, b, c) = (d[j - 1].abs(), d[j].abs(), d[j + 1].abs());
        let at = (from + j) as f64 + 0.5 + 0.5 * (a - c) / (a - 2.0 * b + c);
        sum += at - r;
        k += 1;
    }
    sum / k as f64
}

#[test]
#[ignore = "a measurement, run by hand"]
fn each_modes_oscillator_path_and_onset() {
    for rate in [44_100.0, 48_000.0, 96_000.0] {
        for m in ["LO", "HI", "ULTRA"] {
            eprintln!(
                "{rate} {m:<6} the oscillators' jumps reach the output {:.2} samples after them",
                oscillator_path(m, rate)
            );
        }
    }
    // Onsets: an A5 on 2', its envelope (the RMS over a period) reaching a tenth and half
    // of its held level.
    let at = (0.25 * RATE) as usize;
    for m in ["LO", "HI", "ULTRA"] {
        let mut c = patch(m, false);
        c.panel.osc[0].range = Range::R2;
        let x = play(&c, at, 1.0, &[]);
        let w = 55;
        let env: Vec<f64> = (0..x.len() - w)
            .map(|i| (x[i..i + w].iter().map(|v| v * v).sum::<f64>() / w as f64).sqrt())
            .collect();
        let held = env[(0.8 * RATE) as usize];
        let cross = |share: f64| {
            let i = (at - w..env.len())
                .find(|&i| env[i] >= share * held)
                .unwrap();
            let (a, b) = (env[i - 1], env[i]);
            // (The window's centre.)
            (i - 1) as f64 + (share * held - a) / (b - a) + w as f64 / 2.0 - at as f64
        };
        eprintln!(
            "{m:<6} envelope at 10 % {:.1}, 50 % {:.1}, 90 % {:.1} samples after the key",
            cross(0.1),
            cross(0.5),
            cross(0.9)
        );
    }
}

#[test]
fn the_oscillators_sound_lines_up_in_every_mode() {
    for rate in [44_100.0, 48_000.0, 96_000.0] {
        let ultra = oscillator_path("ULTRA", rate);
        for (m, held) in [("HI", LINE_UP.0), ("LO", LINE_UP.1)] {
            let off = oscillator_path(m, rate) + held as f64 - ultra;
            assert!(
                off.abs() <= 0.5,
                "{m} at {rate} Hz: {off:+.2} samples off ULTRA"
            );
        }
    }
}

/// A voice's output for `seconds` at `rate`, a key pressed at `at` and held: oscillator 1's
/// sawtooth on 8' (none with `quiet`, so that only the VCA's thump as it opens is heard), its
/// contours computed as ULTRA's whatever its quality (only the delays then differ).
fn attack(q: Quality, rate: f64, quiet: bool) -> Vec<f64> {
    let mut c = patch("ULTRA", false);
    c.panel.osc[0].range = Range::R8;
    c.panel.osc[0].on = !quiet;
    let mut v = Voice::new(rate, c.panel);
    v.panel.quality = q;
    v.hold_control_quality(Some(Quality::NoCompromises));
    let at = (0.25 * rate) as usize;
    (0..(0.4 * rate) as usize)
        .map(|i| {
            if i == at {
                v.note(57, true);
            }
            v.tick()
        })
        .collect()
}

#[test]
fn hi_s_attacks_come_out_with_ultra_s() {
    assert_eq!(
        ca72::voice::latency(Quality::HighFidelity),
        ca72::voice::latency(Quality::NoCompromises)
    );
    // Where each attack first reaches 1, 10 and 50 % of HI's peak (interpolated between
    // samples): a sawtooth, and the VCA's thump alone as it starts (its half way, half a
    // second on, is the VCAs' own: HI's bias from a table). The same level for both: ULTRA's
    // VCA takes in the filter's output above the band (decisions.md R-BLEP, ULTRA's VCA at
    // 4x), which raises a sawtooth's peaks by 0.15 dB at 44.1 and 48 kHz.
    for rate in [44_100.0, 48_000.0, 96_000.0] {
        let at = (0.25 * rate) as usize;
        for quiet in [true, false] {
            for q in [Quality::NoCompromises, Quality::HighFidelity] {
                let u = attack(q, rate, quiet);
                let h = attack(Quality::Potato, rate, quiet);
                let shares: &[f64] = if quiet {
                    &[0.01, 0.1]
                } else {
                    &[0.01, 0.1, 0.5]
                };
                for &share in shares {
                    let level = share * peak(&h, at);
                    let off = onset_at(&h, at, level) + LINE_UP.0 as f64 - onset_at(&u, at, level);
                    assert!(
                        off.abs() <= 0.5,
                        "{rate} Hz, {}, {} %: HI {off:+.2} samples off ULTRA",
                        if quiet {
                            "the VCA's thump"
                        } else {
                            "a sawtooth"
                        },
                        share * 100.0
                    );
                }
            }
        }
    }
}

#[test]
fn the_engine_reports_one_latency_in_every_mode() {
    for rate in [44_100.0, 48_000.0, 96_000.0] {
        let mut e = Engine::new();
        let mut reported = Vec::new();
        for m in ["LO", "HI", "ULTRA", "HI", "LO"] {
            e.set(&patch(m, false));
            e.prepare(rate, 1);
            reported.push(e.latency());
        }
        // ULTRA's sound's delay (85.75 samples), rounded.
        assert!(reported.iter().all(|&l| l == 86), "{rate} Hz: {reported:?}");
    }
}

#[test]
fn the_engine_lines_hi_and_lo_up_with_ultra() {
    // The engine's output in each mode against the voice's own: held back by LINE_UP in HI
    // and LO, not at all in ULTRA (their latest samples' lag, cross-correlated). The key goes
    // down within the engine's sleep window (decisions.md R68): a voice at rest from the start
    // (R-CONTOUR-SETTLE) would otherwise sleep first, its oscillators paused, and play at
    // another phase than the voice that ran on.
    let at = (0.02 * RATE) as usize;
    let n = (0.5 * RATE) as usize;
    for (m, q, held) in [
        ("HI", Quality::Potato, LINE_UP.0),
        ("ULTRA", Quality::HighFidelity, 0),
    ] {
        let c = patch(m, false);
        let e = play(&c, at, 0.5, &[]);
        let mut v = Voice::new(RATE, c.panel);
        v.panel.quality = q;
        let x: Vec<f64> = (0..n)
            .map(|i| {
                if i == at {
                    v.note(33, true);
                }
                v.tick()
            })
            .collect();
        let w = |x: &[f64]| x[n - 9_600..n - 200].to_vec();
        let off = lag(&w(&x), &w(&e), 60) - held as f64;
        assert!(off.abs() <= 0.5, "{m}: the engine {off:+.2} samples off");
    }
}
