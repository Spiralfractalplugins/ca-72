//! What a pointer at a point of the drawing finds there, and what operating each kind of
//! control does to its value (normalized, 0..1).
//!
//! A knob turns with a vertical drag (fine: a tenth as fast), the mouse wheel nudges it a
//! notch at a time, a trackpad turns it as far as it travels, and a double click restores its
//! default; RANGE and WAVEFORM step between their positions (a
//! click on a legend picks it, a click on the knob steps toward the side clicked); a switch
//! flips at a click anywhere on it; QUALITY's toggle goes to the position clicked (its lever's
//! three places, or their legends); the wheels stay where they are left, the PITCH wheel
//! settling into its centre detent when let go near it (it has no spring), as an LFO's FADE
//! knob settles into its centre, off (decisions.md R-LFO-FADE).

use crate::arrangement::Arrangement;
use crate::art::{self, BIG, GRIP, Layout, POWER_BOUNDS, STD, WHEEL_SCALE};
use crate::controls::{CONTROLS, Control, Dial, Kind, Place};

/// What is under the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// A control, by its index in [`crate::CONTROLS`].
    Control(usize),
    /// A selector's legend: the selector's index and the legend's position.
    Legend(usize, usize),
    /// The POWER switch.
    Power,
    /// The editor's resize grip.
    Grip,
}

fn in_rect(x: f64, y: f64, r: [f64; 4]) -> bool {
    x >= r[0] && x <= r[2] && y >= r[1] && y <= r[3]
}

/// What is at (x, y) in the drawing, the panel laid out as `arrangement` has it (the drawers'
/// knobs and the LFO page's controls apart: [`hit_drawer`], [`hit_lfo`]).
pub fn hit(layout: &Layout, arrangement: &Arrangement, x: f64, y: f64) -> Option<Target> {
    for (i, c) in arrangement.controls.iter().enumerate() {
        if c.place.drawn_apart() {
            continue;
        }
        if let Some(t) = hit_control(layout, i, c, x, y) {
            return Some(t);
        }
    }
    let (px, py) = arrangement.power_at();
    if in_rect(x - px, y - py, POWER_BOUNDS) {
        return Some(Target::Power);
    }
    if in_rect(x, y, GRIP) {
        return Some(Target::Grip);
    }
    None
}

/// Control `i` (`c`) or one of its legends, if (x, y) in the drawing is on it.
fn hit_control(layout: &Layout, i: usize, c: &Control, x: f64, y: f64) -> Option<Target> {
    let (cx, cy) = c.centre();
    let (dx, dy) = (x - cx, y - cy);
    let found = match c.kind {
        Kind::Knob { big, .. } => dx.hypot(dy) <= if big { BIG.skirt } else { STD.skirt } + 8.0,
        Kind::Selector(set) => {
            for k in set.places() {
                let (lx, ly) = layout.legend_at(set, k);
                if in_rect(dx - lx, dy - ly, [-18.0, -14.0, 18.0, 14.0]) {
                    return Some(Target::Legend(i, k));
                }
            }
            dx.hypot(dy) <= 60.0
        }
        Kind::Rocker { w, h, .. } => in_rect(
            dx,
            dy,
            [-w / 2.0 - 4.0, -h / 2.0 - 4.0, w / 2.0 + 4.0, h / 2.0 + 4.0],
        ),
        Kind::Wheel { .. } => {
            let (hx, hy) = (36.0 * WHEEL_SCALE, 79.0 * WHEEL_SCALE);
            in_rect(dx, dy, [-hx, -hy, hx, hy])
        }
        Kind::Key => in_rect(dx, dy, KEY_HIT),
        // (As the page finds its jack.)
        Kind::Jack => dx.hypot(dy) <= crate::strip::lfo::JACK_R + 10.0,
        Kind::Toggle { across: false, .. } => in_rect(dx, dy, TOGGLE_HIT),
        Kind::Toggle { across: true, .. } => in_rect(dx, dy, TOGGLE_HIT_ACROSS),
    };
    found.then_some(Target::Control(i))
}

/// The LFO page's control (or WAVEFORM's legend) at (x, y) in the drawing, the page showing
/// (decisions.md R-LFO).
pub fn hit_lfo(layout: &Layout, x: f64, y: f64) -> Option<Target> {
    CONTROLS
        .iter()
        .enumerate()
        .filter(|(_, c)| c.place == Place::Lfo)
        .find_map(|(i, c)| hit_control(layout, i, c, x, y))
}

/// A macro's knob at (x, y) in the drawing, the macros showing (decisions.md R-MACROS).
pub fn hit_macro(x: f64, y: f64) -> Option<Target> {
    hit_drawer(Place::Macro, x, y)
}

/// The arpeggiator's drawer's knob at (x, y) in the drawing, the drawer open (decisions.md
/// R-ARP).
pub fn hit_arp(x: f64, y: f64) -> Option<Target> {
    hit_drawer(Place::Arp, x, y)
}

/// Drawer `place`'s knob at (x, y) in the drawing, the drawer open (the arpeggiator's,
/// decisions.md R-ARP; the expression's, R-MPE).
pub fn hit_drawer(place: Place, x: f64, y: f64) -> Option<Target> {
    CONTROLS.iter().enumerate().find_map(|(i, c)| {
        let (cx, cy) = c.centre();
        (c.place == place && (x - cx).hypot(y - cy) <= STD.skirt + 8.0)
            .then_some(Target::Control(i))
    })
}

/// Where a drag takes a control from `start`, the pointer moved by (dx, dy) logical pixels;
/// `None` for a switch, which a drag does not move. A knob of steps (VOICES) goes from step to
/// step, never between them (the owner, 2026-10-10: knobs with discrete options "shouldn't move
/// smoothly between discrete options when the user is clicking and dragging them").
pub fn drag(kind: &Kind, start: f64, dx: f64, dy: f64, fine: bool) -> Option<f64> {
    let rate = if fine { 0.1 } else { 1.0 };
    Some(match kind {
        // VOICES, 2 to 10, a voice a step; the arpeggiator's RATE and OCTAVES a position a step.
        Kind::Knob { dial, .. } => {
            let v = (start - dy / 240.0 * rate).clamp(0.0, 1.0);
            match dial.steps() {
                Some(n) => (v * n as f64).round() / n as f64,
                None => v,
            }
        }
        Kind::Selector(set) => {
            let steps = ((dx - dy) / if fine { 60.0 } else { 30.0 }).round();
            set.at(art::position(start) as f64 + steps)
        }
        Kind::Wheel { .. } => (start - dy / 160.0 * rate).clamp(0.0, 1.0),
        Kind::Jack => (start - dy / 240.0 * rate).clamp(0.0, 1.0),
        Kind::Rocker { .. } | Kind::Toggle { .. } | Kind::Key => return None,
    })
}

/// A WAVEFORM switch CONTINUOUS (decisions.md R-CONTINUOUS-WAVE), as the editor operates it: one
/// position along its whole travel, 0 to 1 over [`CONTINUOUS_PLACES`] of the switch's steps:
/// TRIANGLE (0), the second place (1), SAWTOOTH (2), SQUARE (3), WIDE (4), NARROW (5), then on
/// to the floor, without steps.
pub const CONTINUOUS_PLACES: f64 = art::SHAPE_PLACES;

/// The switch's places a CONTINUOUS switch stops at when a legend or the knob is clicked: its
/// six legends', and the floor's.
const CONTINUOUS_STOPS: [f64; 7] = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, CONTINUOUS_PLACES];

/// A CONTINUOUS switch dragged from `start` (0..1 of its travel) by (`dx`, `dy`): a step each 30
/// pixels as the switch's (60 with `fine`), right or up turning it on, in one smooth motion: no
/// place of the switch's holds it (the owner, 2026-10-20: "It should just be one continuous smooth
/// motion").
pub fn continuous_drag(start: f64, dx: f64, dy: f64, fine: bool) -> f64 {
    let at = start * CONTINUOUS_PLACES + (dx - dy) / if fine { 60.0 } else { 30.0 };
    at.clamp(0.0, CONTINUOUS_PLACES) / CONTINUOUS_PLACES
}

/// A CONTINUOUS switch turned `notches` (whole) by the mouse wheel or an arrow key: a tenth of a
/// step a notch (a fiftieth with `fine`), as smoothly through its places as between them.
pub fn continuous_scroll(v: f64, notches: f64, fine: bool) -> f64 {
    let at = v * CONTINUOUS_PLACES + notches * if fine { 0.02 } else { 0.1 };
    at.clamp(0.0, CONTINUOUS_PLACES) / CONTINUOUS_PLACES
}

/// A click on a CONTINUOUS switch's knob, `dx` from its centre: to its next stop that side (a
/// legend's place, or the floor).
pub fn continuous_click(v: f64, dx: f64) -> f64 {
    let at = v * CONTINUOUS_PLACES;
    let next = if dx < 0.0 {
        CONTINUOUS_STOPS
            .iter()
            .rev()
            .find(|&&p| p < at - 1e-9)
            .copied()
    } else {
        CONTINUOUS_STOPS.iter().find(|&&p| p > at + 1e-9).copied()
    };
    next.unwrap_or(at) / CONTINUOUS_PLACES
}

/// A CONTINUOUS switch's legend `k` (0..5) picked: exactly its place.
pub fn continuous_legend(k: usize) -> f64 {
    CONTINUOUS_STOPS[k.min(5)] / CONTINUOUS_PLACES
}

/// One notch of the mouse wheel or an arrow key, `up` or down.
pub fn step(kind: &Kind, v: f64, up: bool, fine: bool) -> f64 {
    scroll(kind, v, if up { 1.0 } else { -1.0 }, fine)
}

/// A wheel's signed travel in notches: fractional for continuous controls, whole for
/// discrete ones (the editor keeps their remainder until the next event).
pub fn scroll(kind: &Kind, v: f64, notches: f64, fine: bool) -> f64 {
    let notches = if kind.is_continuous() {
        notches
    } else {
        notches.trunc()
    };
    if notches == 0.0 {
        return v;
    }
    match kind {
        // VOICES, 2 to 10, a voice a notch; RATE and OCTAVES a position.
        Kind::Knob { dial, .. } => match dial.steps() {
            Some(n) => {
                let n = n as f64;
                ((v * n).round() + notches).clamp(0.0, n) / n
            }
            None => (v + notches * if fine { 0.002 } else { 0.02 }).clamp(0.0, 1.0),
        },
        Kind::Selector(set) => set.at(art::position(v) as f64 + notches),
        Kind::Wheel { .. } => (v + notches * if fine { 0.005 } else { 0.05 }).clamp(0.0, 1.0),
        Kind::Jack => (v + notches * if fine { 0.002 } else { 0.02 }).clamp(0.0, 1.0),
        Kind::Rocker { .. } | Kind::Key => {
            if notches > 0.0 {
                1.0
            } else {
                0.0
            }
        }
        Kind::Toggle { .. } => (toggle_position(v) as f64 + notches).clamp(0.0, 2.0) / 2.0,
    }
}

/// Where an oscillator's TIMED key answers a pointer, about its centre: its cap and a little
/// round it, and the note value printed beside it (decisions.md R-OSC-TIMED).
pub const KEY_HIT: [f64; 4] = [
    -art::KEY.0 / 2.0 - 6.0,
    -art::KEY.1 / 2.0 - 6.0,
    art::KEY_SIGN + 30.0,
    art::KEY.1 / 2.0 + 6.0,
];

/// Half the height of the toggle's nut: a click within it is HI's.
const TOGGLE_NUT: f64 = 16.0;

/// Where QUALITY's toggle and its legends answer a pointer, about its centre: the lever's
/// travel, ULTRA's legend above, LO's below and HI's to the left.
pub const TOGGLE_HIT: [f64; 4] = [-58.0, -64.0, 28.0, 64.0];

/// Where NOTE PRIORITY's toggle (on its side) and its legends answer a pointer, about its
/// centre: the lever's travel, LAST's legend above, LOW's and HIGH's under its ends.
pub const TOGGLE_HIT_ACROSS: [f64; 4] = [-60.0, -44.0, 64.0, 48.0];

/// A toggle's position, 0 (down), 1 (out) or 2 (up), from its normalized value.
pub fn toggle_position(v: f64) -> usize {
    (v.clamp(0.0, 1.0) * 2.0).round() as usize
}

/// A click that did not drag, (`dx`, `dy`) from the control's centre: a switch flips, a
/// selector steps toward the side clicked, QUALITY's toggle goes where it is clicked (above its
/// nut up, below it down, on it or on HI's legend to its left out), NOTE PRIORITY's likewise
/// on its side (left of its nut left, right of it right, on it or on LAST's legend above out).
pub fn click(kind: &Kind, v: f64, dx: f64, dy: f64) -> Option<f64> {
    match kind {
        Kind::Rocker { .. } | Kind::Key => Some(if v >= 0.5 { 0.0 } else { 1.0 }),
        Kind::Toggle { across: true, .. } => Some(if dx < -TOGGLE_NUT {
            0.0
        } else if dx > TOGGLE_NUT {
            1.0
        } else {
            0.5
        }),
        Kind::Toggle { .. } => Some(if dy < -TOGGLE_NUT {
            1.0
        } else if dy > TOGGLE_NUT {
            0.0
        } else {
            0.5
        }),
        Kind::Selector(set) => {
            Some(set.at(art::position(v) as f64 + if dx < 0.0 { -1.0 } else { 1.0 }))
        }
        _ => None,
    }
}

/// How near its centre (of its travel) an LFO's FADE knob let go settles there, off: under
/// one notch of the mouse wheel (0.02), so that a notch from the centre steps out of it; the
/// fades it takes, 0.27 ms or less either way, do nothing a fade could be heard to.
pub const FADE_DETENT: f64 = 0.015;

/// Where a control settles when let go: the PITCH wheel's centre detent catches it near it, and
/// an LFO's FADE knob's catches it at off (decisions.md R-LFO-FADE).
pub fn release(kind: &Kind, v: f64) -> f64 {
    match kind {
        Kind::Wheel { detent: true, .. } if (v - 0.5).abs() <= 0.04 => 0.5,
        Kind::Knob {
            dial: Dial::LfoFade,
            ..
        } if (v - 0.5).abs() <= FADE_DETENT => 0.5,
        _ => v,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A CONTINUOUS switch (decisions.md R-CONTINUOUS-WAVE): dragged or turned, one smooth motion
    /// from TRIANGLE to the floor, held at none of its places; its legends and clicks its stops
    /// exactly.
    #[test]
    fn a_continuous_switch_turns_smoothly_and_lands_on_a_legend_clicked() {
        let p = CONTINUOUS_PLACES;
        let at = |v: f64| v * p;
        assert!((at(continuous_drag(2.0 / p, 15.0, 0.0, false)) - 2.5).abs() < 1e-12);
        assert!(
            (at(continuous_drag(2.0 / p, 29.0, 0.0, false)) - (2.0 + 29.0 / 30.0)).abs() < 1e-12
        );
        assert!((at(continuous_drag(3.0 / p, 45.0, 0.0, true)) - 3.75).abs() < 1e-12);
        assert_eq!(continuous_drag(3.0 / p, 1e4, 0.0, false), 1.0);
        assert_eq!(continuous_drag(3.0 / p, -1e4, 0.0, false), 0.0);
        // Up is on, as the switch's.
        assert!((at(continuous_drag(3.0 / p, 0.0, -30.0, false)) - 4.0).abs() < 1e-12);
        // The legends: their places exactly; clicks, the next stop that side.
        for k in 0..6 {
            assert_eq!(at(continuous_legend(k)), k as f64);
        }
        assert_eq!(at(continuous_click(4.5 / p, 1.0)), 5.0);
        assert_eq!(at(continuous_click(4.5 / p, -1.0)), 4.0);
        assert_eq!(continuous_click(5.0 / p, 1.0), 1.0);
        assert_eq!(at(continuous_click(0.5 / p, -1.0)), 0.0);
        // The wheel: a tenth of a step a notch, through a place as anywhere.
        assert!((at(continuous_scroll(2.95 / p, 1.0, false)) - 3.05).abs() < 1e-12);
        assert!((at(continuous_scroll(3.05 / p, -1.0, false)) - 2.95).abs() < 1e-12);
        assert!((at(continuous_scroll(2.0 / p, 1.0, true)) - 2.02).abs() < 1e-12);
        assert_eq!(continuous_scroll(1.0, 5.0, false), 1.0);
        assert_eq!(continuous_scroll(0.0, -5.0, false), 0.0);
    }

    /// The owner, 2026-10-20: "I don't like the snapping effect moving from across the different
    /// waveforms ... It should just be one continuous smooth motion." A slow drag across every
    /// place, half a pixel at a time, moves the switch the same small way each time: no jump
    /// onto a place, no dwell on one.
    #[test]
    fn a_slow_drag_across_the_places_neither_jumps_nor_dwells() {
        let p = CONTINUOUS_PLACES;
        let start = 0.2 / p;
        let mut last = continuous_drag(start, 0.0, 0.0, false) * p;
        let each = 0.5 / 30.0;
        for k in 1..=340 {
            let now = continuous_drag(start, 0.5 * f64::from(k), 0.0, false) * p;
            let moved = now - last;
            assert!(
                (moved - each).abs() < 1e-9,
                "at {last:.4}: moved {moved:.6}, not {each:.6}"
            );
            last = now;
        }
        assert!(last > 5.8, "crossed every place: {last}");
    }
    use crate::art::POWER;
    use crate::controls::index;
    use crate::fonts::Fonts;

    fn hit(l: &Layout, x: f64, y: f64) -> Option<Target> {
        super::hit(l, &Arrangement::official(), x, y)
    }

    fn layout() -> Layout {
        Layout::measure(&Fonts::new())
    }

    #[test]
    fn every_control_is_found_at_its_centre() {
        let l = layout();
        for (i, c) in CONTROLS.iter().enumerate() {
            let (x, y) = c.centre();
            let found = if c.place.is_drawer() {
                hit_drawer(c.place, x, y)
            } else if c.place == Place::Lfo {
                hit_lfo(&l, x, y)
            } else if c.place == Place::Macro {
                hit_macro(x, y)
            } else {
                hit(&l, x, y)
            };
            assert_eq!(found, Some(Target::Control(i)), "{}", c.param);
        }
        assert_eq!(
            hit(&l, art::COL + POWER.0, art::TOP + POWER.1),
            Some(Target::Power)
        );
        // The name plate does nothing.
        assert_eq!(hit(&l, art::PLATE_X + 10.0, art::PLATE_Y + 10.0), None);
        assert_eq!(hit(&l, art::W - 5.0, art::H - 5.0), Some(Target::Grip));
        assert_eq!(hit(&l, 5.0, 5.0), None);
    }

    #[test]
    fn a_legend_is_found_where_it_is_printed() {
        let l = layout();
        let i = index("osc1_range").expect("a control");
        let (cx, cy) = CONTROLS[i].centre();
        let Kind::Selector(set) = CONTROLS[i].kind else {
            panic!("a selector")
        };
        for k in 0..6 {
            let (x, y) = l.legend_at(set, k);
            assert_eq!(hit(&l, cx + x, cy + y), Some(Target::Legend(i, k)));
        }
    }

    #[test]
    fn gestures_move_values_as_the_panel_does() {
        let knob = &CONTROLS[index("cutoff").expect("a control")].kind;
        assert_eq!(drag(knob, 0.5, 0.0, -120.0, false), Some(1.0));
        assert!((drag(knob, 0.5, 0.0, -120.0, true).unwrap_or(0.0) - 0.55).abs() < 1e-12);
        let sel = &CONTROLS[index("osc1_range").expect("a control")].kind;
        assert_eq!(drag(sel, 0.4, 30.0, 0.0, false), Some(0.6));
        assert_eq!(click(sel, 0.4, -5.0, 0.0), Some(0.2));
        assert_eq!(step(sel, 1.0, true, false), 1.0);
        let voices = &CONTROLS[index("voices").expect("a control")].kind;
        assert_eq!(step(voices, 0.25, true, false), 0.375);
        assert_eq!(step(voices, 0.25, false, true), 0.125);
        assert_eq!(step(voices, 1.0, true, false), 1.0);
        // A drag goes from voice to voice: a little way, nowhere; further, the next.
        assert_eq!(drag(voices, 0.25, 0.0, -10.0, false), Some(0.25));
        assert_eq!(drag(voices, 0.25, 0.0, -20.0, false), Some(0.375));
        for dy in 0..200 {
            let v = drag(voices, 0.25, 0.0, -f64::from(dy), false).expect("a knob");
            assert_eq!(v * 8.0, (v * 8.0).round(), "between voices at {dy}");
        }
        // The arpeggiator's RATE (13 note values) and OCTAVES (5) the same: never between two.
        for (param, n) in [("arp_rate", 12.0), ("arp_octaves", 4.0)] {
            let k = &CONTROLS[index(param).expect("a control")].kind;
            for dy in 0..300 {
                let v = drag(k, 0.0, 0.0, -f64::from(dy), false).expect("a knob");
                assert_eq!(v * n, (v * n).round(), "{param} between at {dy}");
            }
            assert_eq!(step(k, 0.0, true, false), 1.0 / n);
            assert_eq!(step(k, 1.0, true, true), 1.0);
        }
        let gate = &CONTROLS[index("arp_gate").expect("a control")].kind;
        assert!(
            (drag(gate, 0.5, 0.0, -1.0, false).unwrap_or(0.0) - (0.5 + 1.0 / 240.0)).abs() < 1e-12
        );
        let rocker = &CONTROLS[index("osc1_on").expect("a control")].kind;
        assert_eq!(drag(rocker, 1.0, 0.0, 50.0, false), None);
        assert_eq!(click(rocker, 1.0, 0.0, 0.0), Some(0.0));
        // QUALITY's toggle: where it is clicked, a notch of the wheel a position.
        let toggle = &CONTROLS[index("quality").expect("a control")].kind;
        assert_eq!(drag(toggle, 0.5, 0.0, -50.0, false), None);
        assert_eq!(click(toggle, 0.5, 0.0, -40.0), Some(1.0));
        assert_eq!(click(toggle, 1.0, -40.0, 0.0), Some(0.5));
        assert_eq!(click(toggle, 0.5, 0.0, 45.0), Some(0.0));
        assert_eq!(step(toggle, 0.5, true, false), 1.0);
        assert_eq!(step(toggle, 1.0, true, false), 1.0);
        assert_eq!(step(toggle, 0.5, false, false), 0.0);
        // NOTE PRIORITY's, on its side: left LOW, out LAST, right HIGH.
        let across = &CONTROLS[index("note_priority").expect("a control")].kind;
        assert_eq!(click(across, 0.5, -40.0, 38.0), Some(0.0));
        assert_eq!(click(across, 0.0, 0.0, -33.0), Some(0.5));
        assert_eq!(click(across, 0.5, 40.0, 38.0), Some(1.0));
        assert_eq!(step(across, 0.0, true, false), 0.5);
        let pitch = &CONTROLS[index("pitch_wheel").expect("a control")].kind;
        assert_eq!(release(pitch, 0.53), 0.5);
        assert_eq!(release(pitch, 0.55), 0.55);
    }

    #[test]
    fn fractional_scroll_preserves_each_continuous_controls_sensitivity() {
        for c in CONTROLS.iter().filter(|c| c.kind.is_continuous()) {
            for fine in [false, true] {
                let mut v = 0.5;
                let mut values = Vec::new();
                for _ in 0..160 {
                    v = scroll(&c.kind, v, 1.0 / 160.0, fine);
                    values.push(v);
                }
                assert!(values.windows(2).all(|v| v[1] > v[0]), "{}", c.param);
                assert!(
                    (v - step(&c.kind, 0.5, true, fine)).abs() < 1e-12,
                    "{}",
                    c.param
                );
                assert_eq!(scroll(&c.kind, v, 0.0, fine), v);
                assert_eq!(scroll(&c.kind, 1.0, 0.25, fine), 1.0);
                assert_eq!(scroll(&c.kind, 0.0, -0.25, fine), 0.0);
            }
        }
        for id in [
            "voices",
            "arp_rate",
            "arp_octaves",
            "osc1_range",
            "osc1_waveform",
            "quality",
            "osc1_on",
        ] {
            let kind = &CONTROLS[index(id).expect("control")].kind;
            assert!(!kind.is_continuous());
            assert_eq!(scroll(kind, 0.0, 0.75, false), 0.0);
        }
    }

    /// A trackpad turns a continuous knob or a wheel as far as it travels, a fraction of a notch
    /// a fraction of a notch's move (issue #107, R-KNOB-GLIDE's cases on R-KNOB-RESOLUTION's
    /// `scroll`); a whole notch is the wheel's step; a selector, a switch, a toggle and the
    /// knobs of steps go only a whole step at a time.
    #[test]
    fn a_trackpad_turns_continuous_controls_in_proportion() {
        let knob = &CONTROLS[index("cutoff").expect("a control")].kind;
        assert!(knob.is_continuous());
        for (notches, fine, want) in [
            (0.25, false, 0.505),
            (-0.1, false, 0.498),
            (1.0, false, 0.52),
            (3.5, false, 0.57),
            (0.5, true, 0.501),
        ] {
            let v = scroll(knob, 0.5, notches, fine);
            assert!((v - want).abs() < 1e-12, "{notches} {fine}: {v}");
        }
        assert_eq!(scroll(knob, 0.5, 1.0, false), step(knob, 0.5, true, false));
        assert_eq!(scroll(knob, 0.5, -1.0, true), step(knob, 0.5, false, true));
        assert_eq!(scroll(knob, 0.99, 2.0, false), 1.0);
        assert_eq!(scroll(knob, 0.001, -0.5, false), 0.0);
        // Forty small moves of a quarter notch go as far as ten notches.
        let mut v = 0.2;
        for _ in 0..40 {
            v = scroll(knob, v, 0.25, false);
        }
        assert!((v - 0.4).abs() < 1e-12, "{v}");
        let wheel = &CONTROLS[index("mod_wheel").expect("a control")].kind;
        assert!(wheel.is_continuous());
        assert!((scroll(wheel, 0.5, 0.5, false) - 0.525).abs() < 1e-12);
        assert_eq!(
            scroll(wheel, 0.5, 1.0, false),
            step(wheel, 0.5, true, false)
        );
        for id in [
            "voices",
            "arp_rate",
            "arp_octaves",
            "osc1_range",
            "osc1_on",
            "quality",
            "note_priority",
        ] {
            let k = &CONTROLS[index(id).expect("a control")].kind;
            assert!(!k.is_continuous(), "{id}");
        }
    }
}
