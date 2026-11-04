//! The engine as hosts play it in the ways the reports of issue #53 (artifacts bouncing in
//! Bitwig at 96 kHz, clean once the project's other plug-ins were removed) could come from: in
//! blocks of any length, and on a host's thread whose floating-point controls another plug-in
//! has left as it likes (decisions.md R-SHORT-BLOCKS).
//!
//! - `released_voices_ring_out_in_blocks_of_any_length`: POLY's chords let go, rendered in
//!   blocks of 1, 2, 3, 7 and 31 frames, the same to the bit as in blocks of 512. A voice let go
//!   was freed once its output stayed under `SILENT` for a block: in a block of one frame, at a
//!   zero crossing, its release cut off.
//! - `flush_bits_left_by_another_plug_in_change_nothing`: flush-to-zero off, and
//!   denormals-are-zero on, on the host's thread: the same to the bit.
//! - `floating_point_controls` (by hand): the factory presets Felt Bass, Brass Choir and Feedback
//!   Funk at 96 kHz at each QUALITY with the host's thread's flush bits and rounding as another
//!   plug-in might leave them: every sample finite, the loudness and the high band (the third
//!   difference) within a fifth of a decibel of the render as nih-plug's process call has it.
//!   `cargo test --release -p ca72-plugin --test engine_robustness -- --ignored --nocapture`.

#![allow(clippy::unwrap_used, unsafe_code)]

mod common;

use ca72_plugin::engine::{Controls, Engine, Event};
use common::{controls_of, played_as, preset};

/// The host's thread's floating-point controls: MXCSR on x86-64, FPCR on AArch64.
#[cfg(target_arch = "x86_64")]
mod controls {
    /// Flush-to-zero (nih-plug's process call sets it), denormals-are-zero.
    pub const FTZ: u32 = 1 << 15;
    pub const DAZ: u32 = 1 << 6;
    /// Rounding: to nearest, down, up and toward zero.
    pub const ROUNDING: [u32; 4] = [0, 1 << 13, 2 << 13, 3 << 13];
    pub const MASK: u32 = FTZ | DAZ | 3 << 13;

    pub fn get() -> u32 {
        let mut csr = 0u32;
        // SAFETY: stores MXCSR into a writable u32.
        unsafe { std::arch::asm!("stmxcsr [{}]", in(reg) &mut csr, options(nostack)) };
        csr
    }

    pub fn set(csr: u32) {
        // SAFETY: loads MXCSR with only its control bits changed from what it held.
        unsafe { std::arch::asm!("ldmxcsr [{}]", in(reg) &csr, options(nostack, readonly)) };
    }
}

#[cfg(target_arch = "aarch64")]
mod controls {
    /// Flush-to-zero (nih-plug's process call sets it); AArch64 has no denormals-are-zero
    /// apart from it.
    pub const FTZ: u32 = 1 << 24;
    pub const DAZ: u32 = 0;
    /// Rounding: to nearest, up, down and toward zero (FPCR's RMode).
    pub const ROUNDING: [u32; 4] = [0, 1 << 22, 2 << 22, 3 << 22];
    pub const MASK: u32 = FTZ | 3 << 22;

    pub fn get() -> u32 {
        let fpcr: u64;
        // SAFETY: reads FPCR, as any thread may.
        unsafe { std::arch::asm!("mrs {}, fpcr", out(reg) fpcr, options(nomem, nostack)) };
        fpcr as u32
    }

    pub fn set(fpcr: u32) {
        // SAFETY: writes FPCR with only its control bits changed from what it held.
        unsafe { std::arch::asm!("msr fpcr, {}", in(reg) u64::from(fpcr), options(nostack)) };
    }
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
mod controls {
    pub const FTZ: u32 = 0;
    pub const DAZ: u32 = 0;
    pub const ROUNDING: [u32; 4] = [0; 4];
    pub const MASK: u32 = 0;
    pub fn get() -> u32 {
        0
    }
    pub fn set(_: u32) {}
}

/// Two chords at `rate` (sample, event): four notes from 0.05 s to 0.9 s, four more from 1.0 s
/// to 1.6 s, each taking voices still releasing.
fn chords(rate: f64) -> Vec<(usize, Event)> {
    let mut e = Vec::new();
    for (on, off, keys) in [
        (0.05, 0.9, &[48u8, 55, 60, 64][..]),
        (1.0, 1.6, &[50, 57, 62, 65][..]),
    ] {
        for &key in keys {
            e.push(((on * rate) as usize, Event::Note { key, on: true }));
            e.push(((off * rate) as usize, Event::Note { key, on: false }));
        }
    }
    e.sort_by_key(|x| (x.0, matches!(x.1, Event::Note { on: true, .. })));
    e
}

/// [`chords`] played for `seconds` in blocks of `block` frames, the host's thread's
/// floating-point controls `bits` ([`controls::MASK`]'s) meanwhile, offline (no deadline), the
/// two channels interleaved.
fn render(c: &Controls, rate: f64, seconds: f64, block: usize, bits: u32) -> Vec<f32> {
    let was = controls::get();
    controls::set((was & !controls::MASK) | bits);
    let mut e = Engine::new();
    e.set(c);
    e.prepare(rate, 1);
    let n = (seconds * rate) as usize;
    let events = chords(rate);
    let mut next = events.iter().peekable();
    let (mut l, mut r) = (vec![0.0f32; n], vec![0.0f32; n]);
    let mut b = 0;
    while b < n {
        let len = block.min(n - b);
        let mut k = b;
        while k < b + len {
            while let Some((_, x)) = next.next_if(|(at, _)| *at <= k) {
                e.event(*x);
            }
            let stop = next
                .peek()
                .map_or(b + len, |(at, _)| (*at).clamp(k + 1, b + len));
            e.render(&[], &mut l[k..stop], &mut r[k..stop]);
            k = stop;
        }
        e.end_block(len);
        assert!(!e.take_mending(), "a voice put back clean after sample {b}");
        if e.take_serving() {
            e.crew().serve();
        }
        b += len;
    }
    drop(e);
    controls::set(was);
    l.into_iter().zip(r).flat_map(|(a, b)| [a, b]).collect()
}

/// The first frame two renders differ at, if any.
fn first_difference(x: &[f32], y: &[f32]) -> Option<usize> {
    x.iter()
        .zip(y)
        .position(|(a, b)| a.to_bits() != b.to_bits())
        .map(|i| i / 2)
}

/// POLY's released voices play their release out in blocks of any length: the same to the bit
/// as in blocks of 512. The sounds and rates the blocks of one frame cut off before the fix
/// (decisions.md R-SHORT-BLOCKS): Felt Bass at HI (48 and 96 kHz), the panel a new instance
/// has at LO (96 kHz), POLY's four voices.
#[test]
fn released_voices_ring_out_in_blocks_of_any_length() {
    let felt = {
        let s = preset("Felt Bass");
        played_as(&s, controls_of(&s))
    };
    let init = ca72_plugin::params::Ca72Params::default().controls();
    for (name, mut c, potato, rate) in [
        ("init, LO", init, true, 96_000.0),
        ("Felt Bass, HI", felt, false, 48_000.0),
        ("Felt Bass, HI", felt, false, 96_000.0),
    ] {
        (c.poly, c.voices, c.potato, c.ultra) = (true, 4, potato, false);
        let reference = render(&c, rate, 1.8, 512, controls::FTZ);
        for block in [1, 2, 3, 7, 31] {
            let x = render(&c, rate, 1.8, block, controls::FTZ);
            assert!(x.iter().all(|v| v.is_finite()));
            if let Some(f) = first_difference(&x, &reference) {
                // (What was cut off: the difference over the next 2 ms.)
                let span = 2 * f..(2 * (f + (0.002 * rate) as usize)).min(x.len());
                let cut = x[span.clone()]
                    .iter()
                    .zip(&reference[span])
                    .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
                panic!(
                    "{name}, {rate} Hz, blocks of {block}: differs from {:.4} s, by {:.1} dBFS over the next 2 ms",
                    f as f64 / rate,
                    20.0 * f64::from(cut).log10()
                );
            }
        }
    }
}

/// Flush-to-zero off and denormals-are-zero on, as another plug-in on the host's thread may
/// leave them (nih-plug's process call sets flush-to-zero alone, and restores it after): the
/// same to the bit at LO and HI.
#[test]
fn flush_bits_left_by_another_plug_in_change_nothing() {
    let s = preset("Brass Choir");
    let mut c = played_as(&s, controls_of(&s));
    c.voices = c.voices.min(4);
    for potato in [true, false] {
        (c.potato, c.ultra) = (potato, false);
        let reference = render(&c, 48_000.0, 1.2, 256, controls::FTZ);
        for bits in [0, controls::FTZ | controls::DAZ, controls::DAZ] {
            let x = render(&c, 48_000.0, 1.2, 256, bits);
            assert_eq!(
                first_difference(&x, &reference),
                None,
                "LO {potato}, controls {bits:#x}"
            );
        }
    }
}

/// The energy of a channel of interleaved stereo and of its third difference, in dB.
fn energies(x: &[f32]) -> (f64, f64) {
    let l: Vec<f64> = x.iter().step_by(2).map(|v| f64::from(*v)).collect();
    let e = l.iter().map(|v| v * v).sum::<f64>();
    let h = l
        .windows(4)
        .map(|q| (q[3] - 3.0 * q[2] + 3.0 * q[1] - q[0]).powi(2))
        .sum::<f64>();
    (10.0 * e.log10(), 10.0 * h.log10())
}

#[test]
#[ignore = "a measurement, run by hand"]
fn floating_point_controls() {
    let rate = 96_000.0;
    let mut worst = (0.0f64, 0.0f64);
    for name in ["Felt Bass", "Brass Choir", "Feedback Funk"] {
        let s = preset(name);
        let mut c = played_as(&s, controls_of(&s));
        c.voices = c.voices.min(4);
        for (q, potato, ultra) in [
            ("LO", true, false),
            ("HI", false, false),
            ("ULTRA", false, true),
        ] {
            (c.potato, c.ultra) = (potato, ultra);
            let seconds = if ultra { 0.6 } else { 1.8 };
            let reference = render(&c, rate, seconds, 256, controls::FTZ);
            let (le, he) = energies(&reference);
            for (label, bits) in [
                ("no flush-to-zero", 0),
                ("denormals-are-zero", controls::FTZ | controls::DAZ),
                ("rounding 1", controls::FTZ | controls::ROUNDING[1]),
                ("rounding 2", controls::FTZ | controls::ROUNDING[2]),
                (
                    "rounding toward zero",
                    controls::FTZ | controls::ROUNDING[3],
                ),
            ] {
                let x = render(&c, rate, seconds, 256, bits);
                let non_finite = x.iter().filter(|v| !v.is_finite()).count();
                let most = x
                    .iter()
                    .zip(&reference)
                    .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
                let (lx, hx) = energies(&x);
                eprintln!(
                    "{name}, {q}, {label}: {non_finite} not finite, at most {most:.3e} from the render with flush-to-zero alone, loudness {:+.3} dB, high band {:+.3} dB",
                    lx - le,
                    hx - he
                );
                assert_eq!(non_finite, 0);
                worst = (worst.0.max((lx - le).abs()), worst.1.max((hx - he).abs()));
            }
        }
    }
    eprintln!(
        "worst: loudness {:.3} dB, high band {:.3} dB",
        worst.0, worst.1
    );
    assert!(worst.0 < 0.2 && worst.1 < 0.2);
}
