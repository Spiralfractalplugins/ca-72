//! Two voices' filters solved side by side (`Voice::before_filter`, `Voice::filter_pair`,
//! `Voice::after_filter`; decisions.md R-HI-PAIRS) give each voice the same samples to the bit
//! as `Voice::tick_jacks` alone: at HI, where the pair is solved together, with the voices
//! set apart (notes, cutoffs, emphasis, FEEDBACK, the jacks, a waveform changed, the filter's
//! mode and poles switched), and where the pair is not (ULTRA, and the two at different qualities); and
//! with FEEDBACK's loop closed, where each voice runs its whole audio path alone.

use ca72::voice::{Jacks, Quality, Voice};

/// Two voices, each played alone and played as a pair, `seconds` long: the same samples.
fn pair_matches_alone(qa: Quality, qb: Quality, seconds: f64, one_emphasis: bool) {
    let rate = 48_000.0;
    let tau = 2.0 * core::f64::consts::PI;
    let mut a = Voice::prototype(rate);
    a.set_seed(3);
    a.panel.quality = qa;
    a.panel.cutoff = 0.35;
    a.panel.emphasis = 0.6;
    let mut b = Voice::prototype(rate);
    b.set_seed(11);
    b.panel.quality = qb;
    b.panel.cutoff = 0.55;
    // (One EMPHASIS, as POLY's voices and a twin share it: one circuit, so their loops are
    // evaluated together too.)
    b.panel.emphasis = if one_emphasis { 0.6 } else { 0.95 };
    b.panel.osc[2].on = true;
    let (mut a1, mut b1) = (a.clone(), b.clone());
    let jacks = |t: f64, k: f64| Jacks {
        ext: 0.0,
        detune: [
            3.0 * libm::sin(tau * 0.5 * t + k),
            -2.0,
            1.5 * libm::cos(tau * 0.7 * t),
        ],
        cutoff: 0.03 * libm::sin(tau * 0.3 * t + k),
        ..Jacks::default()
    };
    let n = (seconds * rate) as usize;
    let mut compared = 0usize;
    for i in 0..n {
        let t = i as f64 / rate;
        // Notes and controls at moments of their own on each voice, the same on both copies.
        let mut each = |f: &dyn Fn(&mut Voice)| {
            f(&mut a);
            f(&mut a1);
        };
        match i {
            100 => each(&|v| v.note(45, true)),
            9_000 => each(&|v| v.note(52, true)),
            20_000 => each(&|v| v.panel.filter_mode = ca72::vcf::FilterMode::Hi),
            30_000 => each(&|v| v.note(52, false)),
            _ => {}
        }
        let mut each = |f: &dyn Fn(&mut Voice)| {
            f(&mut b);
            f(&mut b1);
        };
        match i {
            2_500 => each(&|v| v.note(60, true)),
            12_000 => each(&|v| v.feedback = 0.4),
            // (FILTER POLES and FILTER MODE's mixes, each change crossfading: R-POLES.)
            15_000 => each(&|v| v.panel.filter_poles = 2),
            18_000 => each(&|v| v.panel.filter_mode = ca72::vcf::FilterMode::High),
            21_000 => each(&|v| v.panel.filter_mode = ca72::vcf::FilterMode::Band),
            24_000 if !one_emphasis => each(&|v| v.panel.emphasis = 0.2),
            36_000 => each(&|v| v.note(60, false)),
            48_000 => each(&|v| v.note(64, true)),
            _ => {}
        }
        let (ja, jb) = (jacks(t, 0.0), jacks(t, 1.0));
        let ya = a.tick_jacks(&ja);
        let yb = b.tick_jacks(&jb);
        let ha = a1.before_filter(&ja);
        let hb = b1.before_filter(&jb);
        let (fa, fb) = Voice::filter_pair(&mut a1, &ha, &mut b1, &hb);
        let za = a1.after_filter(&ha, fa);
        let zb = b1.after_filter(&hb, fb);
        assert_eq!(
            ya.to_bits(),
            za.to_bits(),
            "{qa:?}/{qb:?}: voice a at sample {i}: {ya} against {za}"
        );
        assert_eq!(
            yb.to_bits(),
            zb.to_bits(),
            "{qa:?}/{qb:?}: voice b at sample {i}: {yb} against {zb}"
        );
        compared += 2;
    }
    assert!(
        a.probe() == a1.probe() && b.probe() == b1.probe(),
        "the voices' last state"
    );
    println!(
        "{qa:?}/{qb:?} (one emphasis: {one_emphasis}): {compared} samples the same to the bit"
    );
}

#[test]
fn a_hi_pair_gives_each_voice_its_own_samples() {
    pair_matches_alone(Quality::Potato, Quality::Potato, 1.25, false);
    pair_matches_alone(Quality::Potato, Quality::Potato, 1.25, true);
}

/// QUALITY switched under held notes, ULTRA to HI and back, the pair solved together only
/// while both play HI: each voice the same to the bit as alone. (The paired path takes over the
/// 4x VCA's states at the switch to HI and leaves its last output for the switch back, as
/// `Voice::tick_jacks` does; decisions.md R-BLEP, ULTRA's VCA at 4x.)
#[test]
fn a_pair_switched_between_ultra_and_hi_plays_each_voice_alone() {
    let rate = 48_000.0;
    let mut a = Voice::prototype(rate);
    a.set_seed(5);
    a.panel.quality = Quality::NoCompromises;
    a.panel.cutoff = 0.4;
    a.panel.emphasis = 0.7;
    let mut b = Voice::prototype(rate);
    b.set_seed(9);
    b.panel.quality = Quality::NoCompromises;
    b.panel.cutoff = 0.6;
    b.panel.emphasis = 0.7;
    let (mut a1, mut b1) = (a.clone(), b.clone());
    let jacks = Jacks::default();
    for i in 0..(0.3 * rate) as usize {
        // Both voices to HI, then voice a alone to ULTRA and back (a mixed pair, played one
        // after the other), then both to ULTRA.
        let (both, a_only) = match i {
            4_000 => (Some(Quality::Potato), None),
            9_000 => (None, Some(Quality::NoCompromises)),
            11_000 => (None, Some(Quality::Potato)),
            12_000 => (Some(Quality::NoCompromises), None),
            _ => (None, None),
        };
        for v in [&mut a, &mut a1, &mut b, &mut b1] {
            if i == 50 {
                v.note(48, true);
            }
            if let Some(q) = both {
                v.panel.quality = q;
            }
        }
        if let Some(q) = a_only {
            a.panel.quality = q;
            a1.panel.quality = q;
        }
        let ya = a.tick_jacks(&jacks);
        let yb = b.tick_jacks(&jacks);
        let ha = a1.before_filter(&jacks);
        let hb = b1.before_filter(&jacks);
        let (fa, fb) = Voice::filter_pair(&mut a1, &ha, &mut b1, &hb);
        let za = a1.after_filter(&ha, fa);
        let zb = b1.after_filter(&hb, fb);
        assert_eq!(
            ya.to_bits(),
            za.to_bits(),
            "voice a at sample {i}: {ya} against {za}"
        );
        assert_eq!(
            yb.to_bits(),
            zb.to_bits(),
            "voice b at sample {i}: {yb} against {zb}"
        );
    }
    assert!(
        a.probe() == a1.probe() && b.probe() == b1.probe(),
        "the voices' last state"
    );
}

/// Two voices with FEEDBACK's loop closed (decisions.md R-HI-FEEDBACK), each played alone and
/// played as a pair: the same samples, one leaving the loop and coming back beside the other.
fn looped_pair_matches_alone(qa: Quality, qb: Quality, seconds: f64) {
    let rate = 48_000.0;
    let tau = 2.0 * core::f64::consts::PI;
    let mut a = Voice::prototype(rate);
    a.set_seed(3);
    a.panel.quality = qa;
    a.panel.cutoff = 0.35;
    a.panel.emphasis = 0.6;
    a.panel.ext_on = true;
    a.panel.ext_volume = 0.8;
    a.feedback = 0.3;
    let mut b = Voice::prototype(rate);
    b.set_seed(11);
    b.panel.quality = qb;
    b.panel.cutoff = 0.55;
    b.panel.emphasis = 0.95;
    b.panel.osc[2].on = true;
    b.panel.ext_on = true;
    b.panel.ext_volume = 1.0;
    b.feedback = 0.6;
    let (mut a1, mut b1) = (a.clone(), b.clone());
    let n = (seconds * rate) as usize;
    for i in 0..n {
        let t = i as f64 / rate;
        let mut each = |f: &dyn Fn(&mut Voice)| {
            f(&mut a);
            f(&mut a1);
        };
        match i {
            100 => each(&|v| v.note(45, true)),
            8_000 => each(&|v| v.panel.filter_mode = ca72::vcf::FilterMode::Hi),
            14_000 => each(&|v| v.panel.ext_on = false),
            20_000 => each(&|v| v.panel.ext_on = true),
            _ => {}
        }
        let mut each = |f: &dyn Fn(&mut Voice)| {
            f(&mut b);
            f(&mut b1);
        };
        match i {
            2_500 => each(&|v| v.note(60, true)),
            10_000 => each(&|v| v.panel.ext_volume = 0.5),
            12_000 => each(&|v| v.feedback = 0.0),
            17_000 => each(&|v| v.feedback = 1.0),
            26_000 => each(&|v| v.note(60, false)),
            _ => {}
        }
        let (ja, jb) = (
            Jacks {
                ext: 0.05 * libm::sin(tau * 110.0 * t),
                ..Jacks::default()
            },
            Jacks::default(),
        );
        let ya = a.tick_jacks(&ja);
        let yb = b.tick_jacks(&jb);
        let ha = a1.before_filter(&ja);
        let hb = b1.before_filter(&jb);
        let (fa, fb) = Voice::filter_pair(&mut a1, &ha, &mut b1, &hb);
        let za = a1.after_filter(&ha, fa);
        let zb = b1.after_filter(&hb, fb);
        assert_eq!(
            ya.to_bits(),
            za.to_bits(),
            "{qa:?}/{qb:?}: voice a at sample {i}: {ya} against {za}"
        );
        assert_eq!(
            yb.to_bits(),
            zb.to_bits(),
            "{qa:?}/{qb:?}: voice b at sample {i}: {yb} against {zb}"
        );
    }
    assert!(
        a.probe() == a1.probe() && b.probe() == b1.probe(),
        "the voices' last state"
    );
    for v in [&a, &b, &a1, &b1] {
        assert_eq!(v.preamp_failed(), 0, "the closed loop's preamplifier");
    }
}

#[test]
fn a_pair_with_the_loop_closed_plays_each_voice_as_alone() {
    looped_pair_matches_alone(Quality::Potato, Quality::Potato, 0.7);
}

#[test]
fn ultra_and_mixed_pairs_with_the_loop_closed_play_each_voice_alone() {
    looped_pair_matches_alone(Quality::NoCompromises, Quality::NoCompromises, 0.3);
    looped_pair_matches_alone(Quality::Potato, Quality::NoCompromises, 0.3);
}

#[test]
fn ultra_and_mixed_pairs_play_each_voice_alone() {
    pair_matches_alone(Quality::HighFidelity, Quality::HighFidelity, 0.2, true);
    pair_matches_alone(Quality::Potato, Quality::HighFidelity, 0.2, true);
    pair_matches_alone(Quality::NoCompromises, Quality::NoCompromises, 0.2, true);
    pair_matches_alone(Quality::Potato, Quality::NoCompromises, 0.2, true);
}
