//! FILTER POLES and FILTER MODE's mixes in the circuit's filter (decisions.md R-POLES): LO from
//! each of the ladder's stages, falling 6 dB an octave a pole above the corner at the drawn
//! filter's passband; STEEP HIGH rising 6 dB an octave a pole below it to the same passband;
//! BAND peaking at the corner; every setting still ringing, the loop's emphasis taken from the
//! fourth stage as drawn; the loop itself untouched by the setting, so that LO at 4 poles is
//! the filter as drawn to the bit after any other; and a change crossfading, not clicking.

use ca72::vcf::{FilterMode, Vcf};
use ca72::voice::Quality;
use std::f64::consts::PI;

const SR: f64 = 48_000.0;
/// One mixer channel's 33K and the test signal's size (small signal).
const G_MIX: f64 = 1.0 / 33e3;
const A_SMALL: f64 = 0.02;
/// The ladder's current for a corner near 500 Hz, and EMPHASIS 0 and 7.5's R14.
const I0: f64 = 6e-6;
const I0_HIGH: f64 = 40e-6;
const R14_0: f64 = 50e3;

fn filter(mode: FilterMode, poles: u8, r14: f64, quality: Quality) -> Vcf {
    let mut f = Vcf::new(SR, 4);
    f.circuit = ca72::filter_cal::CALIBRATED.circuit();
    f.circuit.r14 = r14;
    f.set_quality(quality);
    f.set_shape(mode, poles);
    f
}

/// The response's magnitude, dB, at each frequency: a small impulse on one mixer channel,
/// `n` samples of the output, each frequency's bin by a DFT of its own.
fn response(
    mode: FilterMode,
    poles: u8,
    r14: f64,
    i0: f64,
    quality: Quality,
    freqs: &[f64],
) -> Vec<f64> {
    let n = 1 << 16;
    let mut f = filter(mode, poles, r14, quality);
    let x: Vec<f64> = (0..n)
        .map(|i| {
            f.tick(
                if i == 0 { A_SMALL * G_MIX } else { 0.0 },
                ca72::filter_cal::G_BUS_OFF,
                i0,
            ) / A_SMALL
        })
        .collect();
    freqs
        .iter()
        .map(|&hz| {
            let w = 2.0 * PI * hz / SR;
            let (mut re, mut im) = (0.0, 0.0);
            for (i, v) in x.iter().enumerate() {
                re += v * (w * i as f64).cos();
                im -= v * (w * i as f64).sin();
            }
            10.0 * (re * re + im * im).log10()
        })
        .collect()
}

fn octaves(lo: f64, hi: f64, per: usize) -> Vec<f64> {
    let n = ((hi / lo).log2() * per as f64).round() as usize;
    (0..=n)
        .map(|k| lo * 2f64.powf(k as f64 / per as f64))
        .collect()
}

/// NOTCH's response near the corner, for the record (`-- --ignored --nocapture`).
#[test]
#[ignore]
fn print_notch() {
    let freqs = octaves(250.0, 4_000.0, 16);
    for r14 in [R14_0, 3e3] {
        for n in 1..=4u8 {
            let r = response(
                FilterMode::Notch,
                n,
                r14,
                I0_HIGH,
                Quality::HighFidelity,
                &freqs,
            );
            let (k, low) = r
                .iter()
                .enumerate()
                .fold((0, f64::MAX), |a, (k, &d)| if d < a.1 { (k, d) } else { a });
            let ends = response(
                FilterMode::Notch,
                n,
                r14,
                I0_HIGH,
                Quality::HighFidelity,
                &[62.5, 16_000.0],
            );
            eprintln!(
                "R14 {r14:>6} NOTCH {n}: deepest {low:.1} dB at {:.0} Hz; 62 Hz {:.1}, 16 kHz {:.1}",
                freqs[k], ends[0], ends[1]
            );
        }
    }
}

/// Prints every setting's response (`cargo test -p ca72 --test filter_poles -- --ignored
/// --nocapture`), for the record.
#[test]
#[ignore]
fn print_responses() {
    let freqs = octaves(31.25, 16_000.0, 2);
    for (i0, r14) in [(I0, R14_0), (I0, 3e3), (I0_HIGH, R14_0)] {
        for mode in [FilterMode::Lo, FilterMode::Band, FilterMode::High] {
            for poles in 1..=4 {
                let r = response(mode, poles, r14, i0, Quality::HighFidelity, &freqs);
                let line: Vec<String> = r.iter().map(|d| format!("{d:6.1}")).collect();
                eprintln!(
                    "I0 {:2.0} uA R14 {r14:>6} {mode:?} {poles}: {}",
                    i0 * 1e6,
                    line.join(" ")
                );
            }
        }
    }
    let line: Vec<String> = freqs.iter().map(|f| format!("{f:6.0}")).collect();
    eprintln!("Hz                {}", line.join(" "));
}

/// The responses at a corner near 800 Hz (EMPHASIS 0), at ULTRA's numerics and HI's.
fn at_800(mode: FilterMode, poles: u8, quality: Quality, freqs: &[f64]) -> Vec<f64> {
    response(mode, poles, R14_0, I0_HIGH, quality, freqs)
}

const QUALITIES: [Quality; 2] = [Quality::HighFidelity, Quality::Potato];

/// LO from each stage: 6 dB an octave a pole above the corner (5.7 to 11.3 kHz, asymptotic
/// there), its passband (125 Hz) within a dB of the drawn filter's.
#[test]
fn lo_falls_six_db_an_octave_a_pole_at_the_drawn_passband() {
    let freqs = [125.0, 5_657.0, 11_314.0];
    for q in QUALITIES {
        let drawn = at_800(FilterMode::Lo, 4, q, &freqs)[0];
        for n in 1..=4u8 {
            let r = at_800(FilterMode::Lo, n, q, &freqs);
            let slope = r[1] - r[2];
            assert!(
                (slope - 6.0 * f64::from(n)).abs() < 1.0,
                "{q:?} LO {n}: {slope:.1} dB an octave"
            );
            assert!(
                (r[0] - drawn).abs() < 1.0,
                "{q:?} LO {n}: passband {:.1} dB against the drawn {drawn:.1}",
                r[0]
            );
        }
    }
}

/// STEEP HIGH: from two octaves to one below the corner (250 to 500 Hz) it rises as n poles
/// do there (5.3 dB an octave a pole, 6 an octave further down); its passband (16 kHz) the
/// same at every setting and within 3 dB of LO's (the loop's feedback takes nothing from a
/// high-pass's passband, as it does from the low-pass's).
#[test]
fn steep_high_rises_six_db_an_octave_a_pole() {
    let freqs = [125.0, 250.0, 500.0, 16_000.0];
    for q in QUALITIES {
        let drawn = at_800(FilterMode::Lo, 4, q, &freqs)[0];
        let mut pass = Vec::new();
        for n in 1..=4u8 {
            let r = at_800(FilterMode::High, n, q, &freqs);
            let slope = r[2] - r[1];
            let n = f64::from(n);
            assert!(
                slope > 5.0 * n - 0.5 && slope < 6.5 * n,
                "{q:?} STEEP HIGH {n}: {slope:.1} dB an octave"
            );
            assert!(
                (r[3] - drawn).abs() < 3.0,
                "{q:?} STEEP HIGH {n}: passband {:.1} dB against LO's {drawn:.1}",
                r[3]
            );
            pass.push(r[3]);
        }
        let spread = pass.iter().fold(f64::MIN, |a, &b| a.max(b))
            - pass.iter().fold(f64::MAX, |a, &b| a.min(b));
        assert!(spread < 0.2, "{q:?}: STEEP HIGH's passbands {pass:?}");
    }
}

/// STEEP HIGH at 4 poles keeps falling under a low corner (near 130 Hz): 50 dB down at 31 Hz,
/// where a mix with the drawn (loaded) fourth stage stood at 32.
#[test]
fn steep_high_keeps_falling_under_a_low_corner() {
    let r = response(
        FilterMode::High,
        4,
        R14_0,
        I0,
        Quality::HighFidelity,
        &[31.25, 16_000.0],
    );
    assert!(r[0] - r[1] < -48.0, "{:.1} dB at 31 Hz", r[0] - r[1]);
}

/// BAND: its peak within half an octave of the corner (where LO 4 is 12 dB down, each stage
/// 3; at 3 poles half an octave under it), within a dB of STEEP HIGH's passband (both the loop's drive, which its feedback takes
/// from LO's passband only); 6 dB an octave each side at 1 and 2 poles, 6 below and 12 above
/// at 3, 12 each side at 4.
#[test]
fn band_peaks_at_the_corner() {
    let freqs = octaves(62.5, 16_000.0, 4);
    for q in QUALITIES {
        let lo4 = at_800(FilterMode::Lo, 4, q, &freqs);
        let drawn = lo4[0];
        let high = at_800(FilterMode::High, 1, q, &freqs)[freqs.len() - 1];
        // (Between the grid's points, in octaves.)
        let k = lo4.iter().position(|&d| d < drawn - 12.0).unwrap();
        let t = (drawn - 12.0 - lo4[k - 1]) / (lo4[k] - lo4[k - 1]);
        let corner = freqs[k - 1] * (freqs[k] / freqs[k - 1]).powf(t);
        for n in 1..=4u8 {
            let r = at_800(FilterMode::Band, n, q, &freqs);
            let (k, peak) = r
                .iter()
                .enumerate()
                .fold((0, f64::MIN), |a, (k, &d)| if d > a.1 { (k, d) } else { a });
            // (G^2 (1 - G) peaks half an octave under the corner.)
            let at = if n == 3 { corner / 2f64.sqrt() } else { corner };
            assert!(
                (freqs[k] / at).log2().abs() <= 0.5,
                "{q:?} BAND {n}: peak at {:.0} Hz, the corner {corner:.0}",
                freqs[k]
            );
            assert!(
                (peak - high).abs() < 1.0,
                "{q:?} BAND {n}: peak {peak:.1} dB, STEEP HIGH's passband {high:.1}"
            );
            // Two to three octaves each side of the peak.
            let below = r[k - 8] - r[k - 12];
            let above = r[k + 8] - r[k + 12];
            let want = match n {
                1 | 2 => (6.0, 6.0),
                3 => (6.0, 12.0),
                _ => (12.0, 12.0),
            };
            assert!(
                (below - want.0).abs() < 1.5 && (above - want.1).abs() < 1.5,
                "{q:?} BAND {n}: {below:.1} dB an octave below, {above:.1} above"
            );
        }
    }
}

/// The emphasis is taken from the fourth stage at every setting: at EMPHASIS 10 every one
/// sings on its own once its input has stopped.
#[test]
fn every_setting_sings_at_full_emphasis() {
    let r14 = ca72::voice::emphasis_r14(1.0);
    for mode in [FilterMode::Lo, FilterMode::Band, FilterMode::High] {
        for n in 1..=4u8 {
            let mut f = filter(mode, n, r14, Quality::HighFidelity);
            f.circuit = ca72::filter_cal::CALIBRATED.circuit();
            f.circuit.r14 = r14;
            for i in 0..48_000 {
                let kick = if i < 48 { 0.5 * G_MIX } else { 0.0 };
                f.tick(kick, ca72::filter_cal::G_BUS_OFF, I0_HIGH);
            }
            let x: Vec<f64> = (0..24_000)
                .map(|_| f.tick(0.0, ca72::filter_cal::G_BUS_OFF, I0_HIGH))
                .collect();
            let rms = (x.iter().map(|v| v * v).sum::<f64>() / x.len() as f64).sqrt();
            assert!(rms > 0.05, "{mode:?} {n}: RMS {rms:.2e} V");
        }
    }
}

/// A sawtooth-like test signal on one mixer channel (110 Hz, a little under 1 V).
fn saw(i: usize) -> f64 {
    let t = (i as f64 * 110.0 / SR).fract();
    0.8 * (2.0 * t - 1.0) * G_MIX
}

/// The setting changes only the output: after LO at 1 pole, STEEP HIGH and BAND and back to LO
/// at 4, each change's crossfade over, the filter gives the drawn filter's samples to the
/// bit, and its loop's state is the drawn one's.
#[test]
fn the_loop_is_the_same_whatever_the_setting() {
    let mut drawn = filter(FilterMode::Lo, 4, 10e3, Quality::Potato);
    let mut moved = filter(FilterMode::Lo, 4, 10e3, Quality::Potato);
    for i in 0..60_000 {
        match i {
            5_000 => moved.set_shape(FilterMode::Lo, 1),
            15_000 => moved.set_shape(FilterMode::High, 2),
            25_000 => moved.set_shape(FilterMode::Band, 4),
            35_000 => moved.set_shape(FilterMode::Lo, 4),
            _ => {}
        }
        let a = drawn.tick(saw(i), ca72::filter_cal::G_BUS_OFF, I0_HIGH);
        let b = moved.tick(saw(i), ca72::filter_cal::G_BUS_OFF, I0_HIGH);
        if i > 36_000 {
            assert_eq!(a.to_bits(), b.to_bits(), "sample {i}");
        }
    }
    assert_eq!(drawn.state(), moved.state());
}

/// A change crossfades: switching LO at 4 poles to STEEP HIGH at 4 under a steady tone, no
/// sample moves further from the one before than the larger of the two settings' own does.
#[test]
fn a_change_crossfades() {
    let tone = |i: usize| 0.5 * G_MIX * (2.0 * PI * 220.0 * i as f64 / SR).sin();
    let steepest = |x: &[f64]| {
        x.windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f64, f64::max)
    };
    let run = |shapes: &[(usize, FilterMode)]| {
        let mut f = filter(FilterMode::Lo, 4, R14_0, Quality::HighFidelity);
        (0..36_000)
            .map(|i| {
                if let Some(&(_, m)) = shapes.iter().find(|s| s.0 == i) {
                    f.set_shape(m, 4);
                }
                f.tick(tone(i), ca72::filter_cal::G_BUS_OFF, I0_HIGH)
            })
            .collect::<Vec<_>>()
    };
    let lo = run(&[]);
    let high = run(&[(1, FilterMode::High)]);
    let switched = run(&[(24_000, FilterMode::High)]);
    let own = steepest(&lo[12_000..]).max(steepest(&high[12_000..]));
    for delay in [1, 120] {
        let interrupted = run(&[
            (24_000, FilterMode::High),
            (24_000 + delay, FilterMode::Lo),
            (24_001 + delay, FilterMode::Band),
        ]);
        let step = steepest(&interrupted[23_000..26_000]);
        assert!(step <= 1.1 * own, "interrupted at {delay}: {step} vs {own}");
    }
    let at = steepest(&switched[23_000..26_000]);
    assert!(
        at <= 1.05 * own,
        "{at:.3e} V a sample across the change, {own:.3e} V in either"
    );
}

/// NOTCH: at 1, 2 and 4 poles its deepest within a third of an octave of BAND's peak, 30 dB
/// under its passband (16 kHz, STEEP HIGH's within half a dB) at 2 poles and 40 at 4; at
/// EMPHASIS 7.5 still there, within a third of an octave, 25 and 35 dB down: the loop's feedback
/// moves the poles, not the zeros.
#[test]
fn notch_stays_at_the_corner() {
    let freqs = octaves(250.0, 4_000.0, 16);
    let deepest = |r: &[f64]| {
        r.iter()
            .enumerate()
            .fold((0, f64::MAX), |a, (k, &d)| if d < a.1 { (k, d) } else { a })
    };
    for q in QUALITIES {
        let band = at_800(FilterMode::Band, 2, q, &freqs);
        let peak = band
            .iter()
            .enumerate()
            .fold((0, f64::MIN), |a, (k, &d)| if d > a.1 { (k, d) } else { a })
            .0;
        let high = at_800(FilterMode::High, 2, q, &[16_000.0])[0];
        for (n, depth, rung) in [(1u8, 30.0, 25.0), (2, 30.0, 25.0), (4, 40.0, 35.0)] {
            let pass = at_800(FilterMode::Notch, n, q, &[16_000.0])[0];
            assert!(
                (pass - high).abs() < 0.5,
                "{q:?} NOTCH {n}: passband {pass:.1} dB, STEEP HIGH's {high:.1}"
            );
            for (r14, want) in [(R14_0, depth), (3e3, rung)] {
                let r = response(FilterMode::Notch, n, r14, I0_HIGH, q, &freqs);
                let (k, low) = deepest(&r);
                let octaves = (freqs[k] / freqs[peak]).log2();
                assert!(
                    octaves.abs() <= 1.0 / 3.0 && low < pass - want,
                    "{q:?} NOTCH {n}, R14 {r14}: {low:.1} dB at {:.0} Hz, BAND's peak at {:.0}",
                    freqs[k],
                    freqs[peak]
                );
            }
        }
    }
}
