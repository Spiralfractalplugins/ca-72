//! The factory presets' AUTO GAIN curves (decisions.md R-AUTO-GAIN-DRIVE): each preset carries
//! the curve the plug-in measures for it at 48 kHz, at QUALITY HI (`drive_curve`, ULTRA's too)
//! and LO (`drive_curve_lo`), so that a preset chosen plays at its level at once and nothing is
//! measured. A test measures one preset afresh and checks every preset has both curves; by
//! hand, every preset is measured again and compared to the bit:
//! `cargo test --release -p ca72-plugin --test factory_curves -- --ignored --nocapture`.
//! `CA72_WRITE_CURVES=<presets.toml>` writes the measured curves into that file in place.

#![allow(clippy::unwrap_used)]

mod common;

use ca72_plugin::drive::{Calibration, STEPS};
use ca72_plugin::library::{Sound, factory};
use common::{controls_of, played_as};

/// The plug-in's own measurement of `s` at QUALITY LO (`lo`) or HI, with the voices at the
/// rate an engine prepared at 48 kHz gives them.
fn measured(s: &Sound, lo: bool) -> [f32; STEPS] {
    let mut c = played_as(s, controls_of(s));
    (c.potato, c.ultra) = (lo, false);
    let cal = Calibration::default();
    let mut e = ca72_plugin::engine::Engine::new();
    e.prepare(48_000.0, 1);
    e.calibrate_with(cal.clone());
    cal.ask(&c);
    cal.measure_now();
    let saved = cal.saved();
    assert_eq!(saved.sound, ca72_plugin::drive::sound(&c), "{}", s.name);
    saved.db
}

/// Every factory preset carries a curve at both qualities, each correction finite and none a
/// boost past a decibel (DRIVE makes a sound louder, or leaves it).
#[test]
fn every_factory_preset_carries_its_curves() {
    assert!(!factory().is_empty());
    for s in factory() {
        let hi = controls_of(s);
        let ultra = ca72_plugin::engine::Controls { ultra: true, ..hi };
        assert_eq!(
            ca72_plugin::drive::sound(&hi),
            ca72_plugin::drive::sound(&ultra),
            "{}: ULTRA keeps HI's curve",
            s.name
        );
        for (q, db) in [("HI", s.drive_curve), ("LO", s.drive_curve_lo)] {
            let db = db.unwrap_or_else(|| panic!("{}: no {q} curve", s.name));
            assert!(
                db.iter().all(|d| d.is_finite() && *d <= 1.0 && *d > -40.0),
                "{} {q}: {db:?}",
                s.name
            );
        }
    }
}

/// One preset measured afresh, as the plug-in measures it: its curves are the ones it carries,
/// to the bit. (Tenor Pluck: short, so that the debug build's voices measure it in time.)
#[test]
fn a_factory_presets_curves_are_its_measurement() {
    let s = factory().iter().find(|s| s.name == "Tenor Pluck").unwrap();
    assert_eq!(Some(measured(s, true)), s.drive_curve_lo, "LO");
    assert_eq!(Some(measured(s, false)), s.drive_curve, "HI");
}

/// Every factory preset measured afresh: its curves the ones it carries, to the bit; or, with
/// `CA72_WRITE_CURVES`, written into that presets' file.
#[test]
#[ignore]
fn every_factory_presets_curves_are_its_measurement() {
    let write = std::env::var_os("CA72_WRITE_CURVES");
    let mut curves = Vec::new();
    for s in factory() {
        let (hi, lo) = (measured(s, false), measured(s, true));
        println!("{:24} HI {hi:?} LO {lo:?}", s.name);
        if write.is_none() {
            assert_eq!(Some(hi), s.drive_curve, "{} HI", s.name);
            assert_eq!(Some(lo), s.drive_curve_lo, "{} LO", s.name);
        }
        curves.push((s.name.clone(), hi, lo));
    }
    let Some(file) = write else { return };
    let text = std::fs::read_to_string(&file).unwrap();
    let list = |db: &[f32; STEPS]| {
        db.iter()
            .map(|d| {
                let t = d.to_string();
                if t.contains('.') { t } else { format!("{t}.0") }
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut out = String::new();
    let mut name: Option<String> = None;
    for line in text.lines() {
        if line.starts_with("drive_curve") {
            continue;
        }
        out.push_str(line);
        out.push('\n');
        if line == "[[preset]]" {
            name = None;
        } else if let Some(n) = line.strip_prefix("name = ") {
            name = Some(n.trim_matches('"').to_owned());
        } else if line.starts_with("tags = ") {
            let n = name.as_deref().unwrap();
            let (_, hi, lo) = curves.iter().find(|(c, _, _)| c == n).unwrap();
            out.push_str(&format!("drive_curve = [{}]\n", list(hi)));
            out.push_str(&format!("drive_curve_lo = [{}]\n", list(lo)));
        }
    }
    std::fs::write(&file, out).unwrap();
}
