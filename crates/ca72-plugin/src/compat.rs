//! Older sessions and presets translated into the current controls (R-TRANSLATE,
//! R-ENTROPY-FLOOR).
//!
//! From 0.2.0 the circuit is calibrated to the owner's hardware reference (R42): the laws of a
//! dozen knobs were measured on it in place of the drawing's tapers and straight tracks, so the
//! same position no longer sets the same wiper or resistance. A session 0.1 saved has each of
//! those knobs moved to the position at which 0.2.0's law sets what 0.1's set at the position
//! it held (a position beyond 0.2.0's reach at the knob's end), so that it plays as near to how
//! it sounded as the knobs can bring it. What R42 changed in the circuit's own parts (the
//! contours' resistors, capacitors and diodes, KEYBOARD CONTROL's resistors, the VCA's trims,
//! the keyboard's 0 V on C2) is no knob's to give back, and is left.

use std::collections::BTreeMap;

use ca72::modulation::{mod_wheel_r, mod_wheel_r_drawn};
use ca72::voice::{
    FILTER_ATTACK, FILTER_DECAY, LOUDNESS_ATTACK, LOUDNESS_DECAY, audio_taper,
    contour_amount_track, cutoff_track, emphasis_r14, emphasis_r14_drawn, ext_taper, glide_pot,
    glide_pot_drawn, sustain_track, time_pot, time_pot_drawn, volume_track,
};
use nih_plug::prelude::{Enum, Param};
use nih_plug::wrapper::state::ParamValue;

use crate::params::Ca72Params;

/// Whether a state saved by `version` (nih-plug's: the plug-in's version then, empty from the
/// oldest) is 0.1's, whose knobs follow the drawing's laws.
pub fn before_0_2(version: &str) -> bool {
    if version.is_empty() {
        return true;
    }
    let mut n = version.split('.').map(|x| x.trim().parse::<u32>().ok());
    matches!((n.next().flatten(), n.next().flatten()), (Some(0), Some(minor)) if minor < 2)
}

/// Before the extended ENTROPY travel (0.2.7); an absent writer version is legacy.
pub fn before_entropy_floor(version: &str) -> bool {
    let version = version.trim();
    if version.is_empty() {
        return true;
    }
    let mut n = version.split('.');
    let major = n.next().and_then(|v| v.parse::<u32>().ok());
    let minor = n.next().and_then(|v| v.parse::<u32>().ok());
    let patch = n
        .next()
        .and_then(|v| v.split(['-', '+']).next()?.parse::<u32>().ok())
        .unwrap_or(0);
    matches!((major, minor), (Some(0), Some(m)) if m < 2 || (m == 2 && patch < 7))
}

/// The new percent for a legacy ENTROPY percent (R-ENTROPY-FLOOR).
pub fn entropy_percent(old: f64) -> f64 {
    5.0 + 0.95 * old.clamp(0.0, 100.0)
}

/// A legacy preset's ENTROPY, including the old default if it did not name one.
pub fn translate_entropy_preset(values: &mut Vec<(String, f64)>) {
    if let Some((_, v)) = values.iter_mut().find(|(k, _)| k == "entropy") {
        *v = entropy_percent(*v);
    } else {
        values.push(("entropy".to_owned(), 5.0));
    }
}

/// A knob whose law 0.2.0 changed: its parameter's ID, its plain value at the knob's two ends,
/// and 0.1's and 0.2.0's law, each from the knob's travel (0 to 1) to what it sets there.
struct Changed {
    id: &'static str,
    ends: (f64, f64),
    old: fn(f64) -> f64,
    new: fn(f64) -> f64,
}

fn straight(p: f64) -> f64 {
    p.clamp(0.0, 1.0)
}
fn filter_attack(p: f64) -> f64 {
    time_pot(p, &FILTER_ATTACK)
}
fn filter_decay(p: f64) -> f64 {
    time_pot(p, &FILTER_DECAY)
}
fn loudness_attack(p: f64) -> f64 {
    time_pot(p, &LOUDNESS_ATTACK)
}
fn loudness_decay(p: f64) -> f64 {
    time_pot(p, &LOUDNESS_DECAY)
}

const TEN: (f64, f64) = (0.0, 10.0);

/// Every knob whose law changed between 0.1.5 and 0.2.0: the mixer's VOLUME pots (a straight
/// track, now [`volume_track`]), EXTERNAL INPUT VOLUME (the drawing's audio taper, now
/// [`ext_taper`]), CUTOFF, AMOUNT OF CONTOUR and the two SUSTAINs (straight, now their
/// tracks), EMPHASIS (R14), the four ATTACK and DECAY pots and GLIDE (the drawing's tapers,
/// now measured), and the MODULATION wheel's (its drawn law, now measured).
const CHANGED: [Changed; 16] = [
    Changed {
        id: "osc1_volume",
        ends: TEN,
        old: straight,
        new: volume_track,
    },
    Changed {
        id: "osc2_volume",
        ends: TEN,
        old: straight,
        new: volume_track,
    },
    Changed {
        id: "osc3_volume",
        ends: TEN,
        old: straight,
        new: volume_track,
    },
    Changed {
        id: "noise_volume",
        ends: TEN,
        old: straight,
        new: volume_track,
    },
    Changed {
        id: "ext_volume",
        ends: TEN,
        old: audio_taper,
        new: ext_taper,
    },
    Changed {
        id: "cutoff",
        ends: (-5.0, 5.0),
        old: straight,
        new: cutoff_track,
    },
    Changed {
        id: "emphasis",
        ends: TEN,
        old: emphasis_r14_drawn,
        new: emphasis_r14,
    },
    Changed {
        id: "contour_amount",
        ends: TEN,
        old: straight,
        new: contour_amount_track,
    },
    Changed {
        id: "filter_attack",
        ends: TEN,
        old: time_pot_drawn,
        new: filter_attack,
    },
    Changed {
        id: "filter_decay",
        ends: TEN,
        old: time_pot_drawn,
        new: filter_decay,
    },
    Changed {
        id: "filter_sustain",
        ends: TEN,
        old: straight,
        new: sustain_track,
    },
    Changed {
        id: "loudness_attack",
        ends: TEN,
        old: time_pot_drawn,
        new: loudness_attack,
    },
    Changed {
        id: "loudness_decay",
        ends: TEN,
        old: time_pot_drawn,
        new: loudness_decay,
    },
    Changed {
        id: "loudness_sustain",
        ends: TEN,
        old: straight,
        new: sustain_track,
    },
    Changed {
        id: "glide",
        ends: TEN,
        old: glide_pot_drawn,
        new: glide_pot,
    },
    Changed {
        id: "mod_wheel",
        ends: (0.0, 1.0),
        old: mod_wheel_r_drawn,
        new: mod_wheel_r,
    },
];

/// The travel (0 to 1) at which monotonic `law` gives `want`; its nearer end where it never
/// does.
fn travel_for(law: fn(f64) -> f64, want: f64) -> f64 {
    let (at0, at1) = (law(0.0), law(1.0));
    let rising = at1 >= at0;
    let (lo, hi) = if rising { (at0, at1) } else { (at1, at0) };
    if want <= lo {
        return if rising { 0.0 } else { 1.0 };
    }
    if want >= hi {
        return if rising { 1.0 } else { 0.0 };
    }
    let (mut a, mut b) = (0.0f64, 1.0f64);
    for _ in 0..64 {
        let m = 0.5 * (a + b);
        if (law(m) < want) == rising {
            a = m;
        } else {
            b = m;
        }
    }
    0.5 * (a + b)
}

/// 0.2.0's plain value for knob `id` held at 0.1's `plain`: where 0.2.0's law sets what
/// 0.1's set there. None for a knob whose law did not change.
pub fn translated(id: &str, plain: f64) -> Option<f64> {
    let k = CHANGED.iter().find(|k| k.id == id)?;
    let (lo, hi) = k.ends;
    let at = ((plain - lo) / (hi - lo)).clamp(0.0, 1.0);
    let travel = travel_for(k.new, (k.old)(at));
    Some(lo + travel * (hi - lo))
}

/// The IDs of the knobs [`translated`] moves.
pub fn changed_ids() -> impl Iterator<Item = &'static str> {
    CHANGED.iter().map(|k| k.id)
}

/// LEVEL for what 0.1 saved, dB: R42's VCA trims (its R43 and balance) made the voice 0.2 to
/// 0.6 dB louder, and 0.1.5's presets, translated, played 0.53 dB louder than in 0.1.5 at the
/// median (`tests/preset_render.rs`, `CA72_FROM_0_1`).
pub const LEVEL_FROM_0_1: f64 = -0.5;

/// A state's parameters as 0.1 saved them (plain values), each knob whose law changed moved
/// to 0.2.0's position for it, and each parameter 0.1 did not have, which it would otherwise
/// keep from the instance, at its default (as 0.1 sounded: no DRIVE, UNISON or INNER, FILTER
/// MODE LO), LEVEL at [`LEVEL_FROM_0_1`]. QUALITY, the computer's, is left as it is.
pub fn translate(params: &mut BTreeMap<String, ParamValue>) {
    for (id, v) in params.iter_mut() {
        if let ParamValue::F32(x) = v
            && let Some(t) = translated(id, f64::from(*x))
        {
            *x = t as f32;
        }
    }
    let d = Ca72Params::default();
    let mut put = |id: &str, v: ParamValue| {
        params.entry(id.to_owned()).or_insert(v);
    };
    put("drive", ParamValue::F32(d.drive.default_plain_value()));
    put(
        "auto_gain",
        ParamValue::Bool(d.auto_gain.default_plain_value()),
    );
    put("inner", ParamValue::F32(d.inner.default_plain_value()));
    put("unison", ParamValue::Bool(d.unison.default_plain_value()));
    put("double", ParamValue::F32(d.double.default_plain_value()));
    put(
        "filter_mode",
        ParamValue::I32(d.filter_mode.default_plain_value().to_index() as i32),
    );
    put(
        "filter_poles",
        ParamValue::I32(d.filter_poles.default_plain_value()),
    );
    put(
        "placement",
        ParamValue::I32(d.placement.default_plain_value().to_index() as i32),
    );
    put("level", ParamValue::F32(LEVEL_FROM_0_1 as f32));
}

/// A preset of 0.1's (plain values by ID) as a session of 0.1's is translated: each knob whose
/// law changed at its value, or its default where the preset does not name it (0.1's defaults
/// are 0.2.0's), moved; LEVEL at [`LEVEL_FROM_0_1`]. The others are left to be their defaults,
/// as a preset's are.
pub fn translate_preset(values: &mut Vec<(String, f64)>) {
    let defaults = crate::presets::values_now(&Ca72Params::default());
    for id in changed_ids() {
        if !values.iter().any(|(k, _)| k == id)
            && let Some((_, d)) = defaults.iter().find(|(k, _)| k == id)
        {
            values.push((id.to_owned(), *d));
        }
    }
    for (k, v) in values.iter_mut() {
        if let Some(t) = translated(k, *v) {
            *v = t;
        }
    }
    values.retain(|(k, _)| k != "level");
    values.push(("level".to_owned(), LEVEL_FROM_0_1));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entropy_moves_the_old_range_to_five_through_one_hundred() {
        assert_eq!(entropy_percent(0.0), 5.0);
        assert_eq!(entropy_percent(40.0), 43.0);
        assert_eq!(entropy_percent(100.0), 100.0);
        for i in 0..=1000 {
            let old = f64::from(i) / 10.0;
            let new = entropy_percent(old);
            let depth = crate::engine::entropy_depth(new / 100.0);
            assert!((depth - old / 100.0 * 2.0).abs() < 1e-15);
        }
        for version in [
            "",
            "0.1.5",
            "0.2",
            "0.2.0",
            "0.2.6",
            "0.2.6+build",
            " 0.2.6 ",
        ] {
            assert!(before_entropy_floor(version), "{version}");
        }
        for version in [
            "0.2.7",
            "0.2.7-beta.1",
            "0.2.7+build.1",
            "0.2.8",
            "0.3.0",
            "1.0.0",
            "nightly",
        ] {
            assert!(!before_entropy_floor(version), "{version}");
        }
    }

    #[test]
    fn the_versions_before_0_2_are_0_1s() {
        for v in ["", "0.1.0", "0.1.5", "0.1"] {
            assert!(before_0_2(v), "{v:?}");
        }
        for v in ["0.2.0", "0.2.1", "0.10.0", "1.0.0", "nightly"] {
            assert!(!before_0_2(v), "{v:?}");
        }
    }

    /// Each knob moved sets in 0.2.0 what it set in 0.1, to a part in a million of its range,
    /// wherever 0.2.0's law reaches it; elsewhere it stops at the end nearer.
    #[test]
    fn a_translated_knob_sets_what_it_set() {
        for k in &CHANGED {
            let (lo, hi) = k.ends;
            let (n0, n1) = ((k.new)(0.0), (k.new)(1.0));
            let (min, max) = (n0.min(n1), n0.max(n1));
            for i in 0..=100 {
                let at = f64::from(i) / 100.0;
                let plain = lo + at * (hi - lo);
                let t = translated(k.id, plain).unwrap();
                assert!((lo..=hi).contains(&t), "{}: {t}", k.id);
                let want = (k.old)(at);
                let got = (k.new)((t - lo) / (hi - lo));
                if want > min && want < max {
                    assert!(
                        (got - want).abs() <= 1e-6 * (max - min),
                        "{} at {at}: {got} for {want}",
                        k.id
                    );
                } else {
                    let end = if (want <= min) == (n1 >= n0) {
                        0.0
                    } else {
                        1.0
                    };
                    assert_eq!((t - lo) / (hi - lo), end, "{} at {at}", k.id);
                }
            }
        }
    }

    /// The knobs at their ends stay there, and moving through 0.1's travel never turns a knob
    /// back.
    #[test]
    fn translation_keeps_the_ends_and_the_order() {
        for k in &CHANGED {
            let (lo, hi) = k.ends;
            let mut last = f64::NEG_INFINITY;
            for i in 0..=200 {
                let plain = lo + f64::from(i) / 200.0 * (hi - lo);
                let t = translated(k.id, plain).unwrap();
                assert!(t >= last - 1e-9, "{} turned back at {plain}", k.id);
                last = t;
            }
        }
        assert_eq!(
            translated("cutoff", 0.0).map(|t| (t * 1e6).round()),
            Some(0.0)
        );
        assert!(translated("tune", 1.0).is_none());
    }

    /// A state 0.1 saved opens with what 0.1 did not have at its default, whatever the instance
    /// holds, LEVEL trimmed; QUALITY is the computer's.
    #[test]
    fn a_state_of_0_1_gets_0_2s_additions_at_their_defaults() {
        let mut p = BTreeMap::new();
        p.insert("cutoff".to_owned(), ParamValue::F32(0.0));
        translate(&mut p);
        assert!(matches!(p["drive"], ParamValue::F32(x) if x == 0.0));
        assert!(matches!(p["unison"], ParamValue::Bool(false)));
        assert!(matches!(p["filter_mode"], ParamValue::I32(0)));
        assert!(matches!(p["filter_poles"], ParamValue::I32(4)));
        assert!(matches!(p["level"], ParamValue::F32(x) if x == LEVEL_FROM_0_1 as f32));
        assert!(!p.contains_key("quality"));
        let mut v = vec![("emphasis".to_owned(), 4.0), ("level".to_owned(), 3.0)];
        translate_preset(&mut v);
        assert_eq!(v.iter().filter(|(k, _)| k == "level").count(), 1);
        assert!(v.contains(&("level".to_owned(), LEVEL_FROM_0_1)));
        assert_eq!(v.len(), CHANGED.len() + 1, "{v:?}");
    }

    #[test]
    fn a_state_translates_only_its_changed_knobs() {
        let mut p = BTreeMap::new();
        p.insert("emphasis".to_owned(), ParamValue::F32(5.0));
        p.insert("tune".to_owned(), ParamValue::F32(1.0));
        p.insert("poly".to_owned(), ParamValue::Bool(true));
        translate(&mut p);
        assert!(matches!(p["emphasis"], ParamValue::F32(x) if (x - 5.0).abs() > 0.01));
        assert!(matches!(p["tune"], ParamValue::F32(x) if x == 1.0));
        assert!(matches!(p["poly"], ParamValue::Bool(true)));
    }
}
