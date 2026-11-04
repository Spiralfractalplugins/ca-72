//! The panel's art, as SVG, in panel units: the panel measured from a photograph of the
//! instrument's taken face on (Commons, "Minimoog panel.jpg"; the panel is 3108 by 795 of
//! them), under a wooden top strip with the name plate on it; and under that the plug-in's
//! strip (`crate::strip`), the left hand controller's GLIDE, DECAY and wheels at its left
//! (A6, decisions.md R44: they had a column of their own beside the panel).
//!
//! What never moves is drawn once ([`background`]); each control, lamp and the name plate
//! is a layer of its own, drawn over it ([`Layer`]).

use crate::arrangement::Arrangement;
use crate::controls::{Colour, Control, Dial, Kind, Legends, Mark, Orient, Place};
use crate::fonts::{FAMILY, Fonts, Weight};
use crate::skin::Part;
use crate::svg::{N, Svg, colour::*, defs, escape, put};

pub const PW: f64 = 3108.0;
pub const PH: f64 = 795.0;
pub const TOP: f64 = 130.0;
/// The panel's left edge: the drawing's (the left hand controller's column, 330 units wide,
/// was there until A6: its controls are on the strip, `strip::LEFT_HAND`).
pub const COL: f64 = 0.0;
/// The drawing's width.
pub const W: f64 = COL + PW;
/// The panel's height: the top strip and the face. (The name board under the face, 132 units,
/// went when the name plate moved up into the top strip: the owner, 2026-10-10, "If we moved
/// the CA-72 badge to the top, we could save a lot of space in that center wooden area.")
pub const PANEL_H: f64 = TOP + PH;
/// The strip's under it (A6): the presets' rail, then three rows.
pub const STRIP_H: f64 = 850.0;
/// The drawing's height: the panel and the strip.
pub const H: f64 = PANEL_H + STRIP_H;

/// A look's panel's size (decisions.md R-SKIN-PACKS): its wooden strip's height and its face's
/// (drawing units), the width always [`PW`]. Its face's foot is always at [`PANEL_H`], where
/// the plug-in's strip begins, so that everything under it keeps its place in the drawing;
/// a panel taller than the official one reaches above the drawing's 0, a shorter one starts
/// below it. Places on the face are kept as the official panel's ([`TOP`] its face's top):
/// a layout's are moved by [`Panel::shift`] as it is read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Panel {
    pub wood: f64,
    pub face: f64,
}

impl Panel {
    /// The official panel's.
    pub const OFFICIAL: Panel = Panel {
        wood: TOP,
        face: PH,
    };
    /// What a pack may choose: the face's height, and the wooden strip's (or none).
    pub const FACE: (f64, f64) = (700.0, 1200.0);
    pub const WOOD: (f64, f64) = (60.0, 200.0);

    /// The drawing's y of the face's top.
    pub fn face_top(&self) -> f64 {
        PANEL_H - self.face
    }

    /// The drawing's y of the panel's top, the wooden strip's: where the frame begins.
    pub fn top(&self) -> f64 {
        PANEL_H - self.face - self.wood
    }

    /// The panel's height: the wooden strip and the face.
    pub fn height(&self) -> f64 {
        self.wood + self.face
    }

    /// The drawing's height from the panel's top: the panel and the strip.
    pub fn drawing_h(&self) -> f64 {
        H - self.top()
    }

    /// What a layout's face units are moved by to be the official panel's (its face's foot
    /// at [`PH`]).
    pub fn shift(&self) -> f64 {
        PH - self.face
    }

    /// The canvas in the official panel's face units: the wooden strip and the face.
    pub fn canvas(&self) -> [f64; 4] {
        [0.0, self.top() - TOP, PW, PH]
    }
}

/// How large the lettering is printed (R-LETTERING): as the photograph has it, or a size
/// larger for those who find that hard to read at the size the editor opens, chosen in the
/// settings' menu. LARGE moves four legends clear of what grew beside them; the controls stay
/// where they are.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Lettering {
    #[default]
    Standard,
    Large,
}

impl Lettering {
    /// The panel's legends.
    pub const fn legend(self) -> f64 {
        match self {
            Lettering::Standard => 19.0,
            Lettering::Large => 22.0,
        }
    }

    /// The dials' numbers and the panel's small print (ON, LO and HI, the left hand's).
    pub const fn number(self) -> f64 {
        match self {
            Lettering::Standard => 15.0,
            Lettering::Large => 17.0,
        }
    }

    /// The strip's legends.
    pub const fn strip_legend(self) -> f64 {
        match self {
            Lettering::Standard => 21.0,
            Lettering::Large => 23.0,
        }
    }

    /// The strip's small print (the readouts' units, the display's L and R).
    pub const fn strip_small(self) -> f64 {
        match self {
            Lettering::Standard => 16.0,
            Lettering::Large => 18.0,
        }
    }

    /// The tip's, a note's and the menus' lettering, against the standard's.
    pub const fn text(self) -> f64 {
        match self {
            Lettering::Standard => 1.0,
            Lettering::Large => 1.25,
        }
    }
}

/// The panel's lettering is about 9 % wider than the typeface at the same capital height:
/// its legends are drawn stretched so (the section titles, set to their printed widths, and
/// the name plate are not).
pub(crate) const STRETCH: f64 = 1.09;

/// The name plate's name, the instrument's. Its maker's name is under it.
pub const PLATE_NAME: &str = "CA-72";
const PLATE_MAKER: &str = "IDLE FOUNDRY";
/// The name's margin from the plate's left end.
const PLATE_PAD: f64 = 24.0;
/// The plate's width: as when a click turned it over to a second, wider name, gone since
/// (the owner, 2026-10-03), so the panel stays as approved.
pub const PLATE_W: f64 = 261.0;
/// The plate's top left corner: its left end on the MODIFIERS|OUTPUT line (the divider's,
/// 2578), in the top strip between its screws, in the strip's middle (the owner, 2026-10-10,
/// of three places rendered whole: "I think we can go with option C").
pub const PLATE_X: f64 = COL + 2578.0;
pub const PLATE_H: f64 = 82.0;
pub const PLATE_Y: f64 = (TOP - PLATE_H) / 2.0;
const NAME_Y: f64 = 31.0;
const MAKER_Y: f64 = 64.0;

/// The left hand controller's place as its column had it: its left edge then, its width
/// (its controls are placed relative to it, now at `strip::LEFT_HAND`).
pub const LH_X: f64 = 22.0;
pub const LPW: f64 = 330.0 - 44.0;
pub(crate) const LH_JACK: f64 = 46.0;
const LH_LABEL: f64 = 118.0;
pub(crate) const LH_ROCKER: f64 = 214.0;
const LH_SCREW_DX: f64 = 50.0;
pub(crate) const LH_ROWS: [f64; 2] = [265.0, 345.0];
/// VELOCITY's row, under DECAY's (decisions.md R-VELOCITY): its jack, legend and meter
/// (`strip::velocity`).
pub(crate) const LH_VELOCITY: f64 = 425.0;
/// NOTE PRIORITY's toggle on its side and RETRIGGER's switch (decisions.md R50), each a
/// row over GLIDE and DECAY in one column with theirs, where their switches are; their legends'
/// centres, left of them as GLIDE's and DECAY's are (moved in, clear of the switch and of the
/// lever at LOW).
pub(crate) const LH_PRIORITY: f64 = 95.0;
const LH_PRIORITY_LABEL: f64 = 108.0;
pub(crate) const LH_RETRIGGER: f64 = 185.0;
const LH_RETRIGGER_LABEL: f64 = 104.0;
/// RETRIGGER's centre in the large lettering: its right end as clear of its switch.
const LH_RETRIGGER_LABEL_LARGE: f64 = 96.0;
pub(crate) const LH_WHEELS: [f64; 2] = [82.0, 204.0];
/// The wheels' centre (80 lower than A6 had it, for NOTE PRIORITY's and RETRIGGER's rows and
/// VELOCITY's: decisions.md R-VELOCITY).
pub(crate) const WHEEL_Y: f64 = 595.0;
/// The wheels are drawn larger in the column than the instrument's.
pub const WHEEL_SCALE: f64 = 1.75;
/// A wheel's slot.
const SLOT_W: f64 = 60.0;
const SLOT_H: f64 = 146.0;

/// The POWER switch (the plugin's bypass) and its lamp, on the panel. These places, QUALITY's,
/// ULTRA's and the plate's are the official layout's (`packs/official/layout.toml`, held equal to
/// them by its tests): the parts are drawn here about them, and the renderer moves each to where
/// the layout shown has it (`crate::arrangement`).
pub const POWER: (f64, f64) = (3023.0, 580.0);
pub(crate) const LAMP_AT: (f64, f64) = (3023.0, 426.0);
/// The OVERLOAD lamp, on the panel.
pub(crate) const OVERLOAD_AT: (f64, f64) = (1764.0, 274.0);
/// QUALITY's toggle (decisions.md R48), on the panel, and its size as seen: its pictures'
/// (180 by 291 pixels, the nut at their middle) 50 units wide.
pub(crate) const QUALITY_AT: (f64, f64) = (3023.0, 266.0);
pub(crate) const TOGGLE: (f64, f64) = (50.0, 50.0 * 291.0 / 180.0);
/// ULTRA's lamp (decisions.md R48), on the panel, over QUALITY.
pub(crate) const ULTRA_AT: (f64, f64) = (3023.0, 92.0);

/// The editor's resize grip, in the drawing's bottom right corner (left, top, right, bottom).
pub const GRIP: [f64; 4] = [W - 90.0, H - 60.0, W, H];
/// The grip's lines' corner: inside the strip's trim (28 at its right end, 6 along its foot).
const GRIP_CORNER: (f64, f64) = (W - 36.0, H - 14.0);

/// The waveform pictograms, drawn 24 by 14 and printed about 18 by 10 (`GLYPH`).
const GLYPH: f64 = 0.75;
const TRIANGLE: &str = "M -11 7 L 0 -7 L 11 7";
const SHARK: &str = "M -11 7 L 3 -7 L 3 0 L 11 7";
const REVERSE: &str = "M -10 7 V -7 L 10 7";
const SAWTOOTH: &str = "M -10 7 L 10 -7 V 7";
/// Oscillator 3's second place in CONTINUOUS (decisions.md R-CONTINUOUS-WAVE): the shark tooth
/// turned back, as its reverse sawtooth is the sawtooth turned back.
const REVERSE_SHARK: &str = "M -11 7 L -3 0 L -3 -7 L 11 7";
/// Oscillator 3's legends in CONTINUOUS: its reverse shark tooth, then its reverse sawtooth.
const WAVES3_CONTINUOUS: [&str; 6] = [TRIANGLE, REVERSE_SHARK, REVERSE, SQUARE, WIDE, NARROW];
const SQUARE: &str = "M -12 7 V -7 H 0 V 7 H 12";
const WIDE: &str = "M -12 7 V -7 H -3 V 7 H 12";
const NARROW: &str = "M -12 7 V -7 H -7 V 7 H 12";
const RANGES: [&str; 6] = ["LO", "32'", "16'", "8'", "4'", "2'"];
const WAVES12: [&str; 6] = [TRIANGLE, SHARK, SAWTOOTH, SQUARE, WIDE, NARROW];
const WAVES3: [&str; 6] = [TRIANGLE, REVERSE, SAWTOOTH, SQUARE, WIDE, NARROW];
/// FILTER POLES's (decisions.md R-POLES): on the middle four places, the outer two unused.
const POLES: [&str; 6] = ["", "1", "2", "3", "4", ""];

/// A six-position selector's positions (RANGE, WAVEFORM): their angles from twelve
/// o'clock, 33.5 degrees apart, as the photograph's ticks and legends (on a circle of 89
/// about the axis) place them.
pub const SIX: [f64; 6] = [-83.75, -50.25, -16.75, 16.75, 50.25, 83.75];
/// The selectors' printed ticks (inner and outer radius, 4 wide) and their legends'
/// radius; a legend clears its tick by `SIX_CLEAR` at least.
const SIX_TICK: (f64, f64) = (61.0, 70.0);
const SIX_LEGEND: f64 = 89.0;
const SIX_CLEAR: f64 = 6.5;

/// A knob's size: its black skirt, its aluminium cap, its pointer dot's radius from the
/// centre and its own.
#[derive(Clone, Copy, Debug)]
pub struct Size {
    pub skirt: f64,
    cap: f64,
    dot: f64,
    dot_r: f64,
}

pub const STD: Size = Size {
    skirt: 54.0,
    cap: 36.0,
    dot: 40.0,
    dot_r: 4.0,
};
pub const BIG: Size = Size {
    skirt: 65.0,
    cap: 49.0,
    dot: 54.0,
    dot_r: 5.0,
};

fn rad(deg: f64) -> f64 {
    deg.to_radians()
}

/// The point at `r` from the centre at `deg` clockwise from twelve o'clock.
pub fn polar(r: f64, deg: f64) -> (f64, f64) {
    (r * rad(deg).sin(), -r * rad(deg).cos())
}

/// The lettering's kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Style {
    Legend,
    Title,
    Plate,
}

/// Text centred on (x, y) (`y` the middle of its capitals); `start`: begun at `x`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn text(
    s: &mut Svg,
    x: f64,
    y: f64,
    t: &str,
    size: f64,
    style: Style,
    start: bool,
    extra: &str,
) {
    let k = if style == Style::Legend { STRETCH } else { 1.0 };
    let look = match style {
        Style::Legend => format!(
            "fill='{LEGEND}' font-weight='700' stroke='{LEGEND}' stroke-width='0.7' paint-order='stroke'"
        ),
        Style::Title => format!("fill='{LEGEND}' font-weight='700'"),
        Style::Plate => format!("fill='{PLATE_TEXT}' font-weight='700'"),
    };
    let anchor = if start { "start" } else { "middle" };
    put!(
        s,
        "<text x='0' y='{}' transform='translate({} {}) scale({} 1)' text-anchor='{anchor}' font-size='{}' {look} {extra}>{}</text>",
        N(size * 0.36),
        N(x),
        N(y),
        N(k),
        N(size),
        escape(t)
    );
}

fn legend(s: &mut Svg, x: f64, y: f64, t: &str) {
    let size = s.1.legend();
    text(s, x, y, t, size, Style::Legend, false, "");
}

fn lettered(s: &mut Svg, x: f64, y: f64, t: &str, size: f64) {
    text(s, x, y, t, size, Style::Legend, false, "");
}

fn line(s: &mut Svg, x1: f64, y1: f64, x2: f64, y2: f64, w: f64) {
    put!(
        s,
        "<line x1='{}' y1='{}' x2='{}' y2='{}' stroke='{LEGEND}' stroke-width='{}' stroke-linecap='round'/>",
        N(x1),
        N(y1),
        N(x2),
        N(y2),
        N(w)
    );
}

fn shadow(s: &mut Svg, cx: f64, cy: f64, r: f64) {
    put!(
        s,
        "<circle cx='{}' cy='{}' r='{}' fill='{SHADOW}' fill-opacity='{SHADOW_OPACITY}'/>",
        N(cx),
        N(cy),
        N(r)
    );
}

/// A small dark screw head with its slot.
fn screw(s: &mut Svg, x: f64, y: f64, r: f64) {
    put!(
        s,
        "<circle cx='{}' cy='{}' r='{}' fill='{SCREW}'/>",
        N(x),
        N(y),
        N(r)
    );
    put!(
        s,
        "<line x1='{}' y1='{}' x2='{}' y2='{}' stroke='{HOLE}' stroke-width='2'/>",
        N(x - r * 0.7),
        N(y - r * 0.2),
        N(x + r * 0.7),
        N(y + r * 0.2)
    );
}

/// A dial's printed marks: ticks (angle, inner and outer radius, width) and numbers (angle,
/// text, and a radius where the number's printed place is not the dial's), and captions.
struct DialMarks {
    ticks: Vec<(f64, f64, f64, f64)>,
    labels: Vec<(f64, &'static str, Option<f64>)>,
    r: f64,
    captions: Vec<(f64, f64, &'static str)>,
}

fn eleven(r0: f64, r1: f64) -> Vec<(f64, f64, f64, f64)> {
    (0..11)
        .map(|k| (-150.0 + 30.0 * f64::from(k), r0, r1, 4.0))
        .collect()
}

/// A strip knob's dial: a tick at each of `ticks` (degrees from twelve o'clock), numbered
/// by `labels[i]` where it says `Some(i)`.
fn numbered(ticks: &[(f64, Option<i32>)], labels: &[&'static str]) -> DialMarks {
    DialMarks {
        ticks: ticks.iter().map(|&(a, _)| (a, 58.0, 71.0, 4.0)).collect(),
        labels: ticks
            .iter()
            .filter_map(|&(a, i)| Some((a, *labels.get(usize::try_from(i?).ok()?)?, None)))
            .collect(),
        r: 86.0,
        captions: vec![],
    }
}

/// The timed dial's numbered values: by their place among its eighteen (R-OSC-TIMED).
const TIMED_NUMBERS: [(usize, &str); 8] = [
    (0, "4×"),
    (1, "2×"),
    (2, "1"),
    (5, "2"),
    (8, "4"),
    (11, "8"),
    (14, "16"),
    (17, "32"),
];
/// The timed dial's numbers' radius; how far above and below its centre their middles go at
/// most (the semitone dial's ±1 and ±7: 98 out at 20 and 140 degrees, the 7s a little further
/// for their width); and how far out across an end's number is hung, there.
const TIMED_R: f64 = 92.0;
const TIMED_TOP: f64 = 92.0;
const TIMED_FOOT: f64 = 77.0;
const TIMED_HUNG: f64 = 7.0;

/// Whether the timed dial prints BARS beside its 4× and 2× (`CA72_TIMED_BARS=1`, the mock-ups'
/// comparison; without it, none: the × says they are bars).
fn timed_bars_caption() -> bool {
    std::env::var("CA72_TIMED_BARS").is_ok_and(|v| v == "1")
}

fn marks(dial: Dial) -> DialMarks {
    let ten = || DialMarks {
        ticks: eleven(58.0, 71.0),
        labels: ["0", "2", "4", "6", "8", "10"]
            .iter()
            .enumerate()
            .map(|(i, s)| (-150.0 + 60.0 * i as f64, *s, None))
            .collect(),
        r: 86.0,
        captions: vec![],
    };
    match dial {
        Dial::Ten => ten(),
        Dial::ModMix => DialMarks {
            captions: vec![(-50.0, 100.0, "OSC. 3"), (49.0, 100.0, "NOISE")],
            ..ten()
        },
        Dial::Tune => DialMarks {
            ticks: eleven(58.0, 71.0),
            labels: vec![
                (-120.0, "–2", None),
                (-60.0, "–1", None),
                (0.0, "0", None),
                (60.0, "1", None),
                (120.0, "2", None),
            ],
            r: 86.0,
            captions: vec![],
        },
        Dial::Cutoff => DialMarks {
            ticks: eleven(58.0, 71.0),
            labels: vec![
                (-120.0, "–4", None),
                (-60.0, "–2", None),
                (0.0, "0", None),
                (60.0, "2", None),
                (120.0, "4", None),
            ],
            r: 86.0,
            captions: vec![],
        },
        // A tick a semitone, 20 degrees apart, from –7 to +7 (the odd ones numbered, 0
        // long), and an end mark past each end.
        Dial::Freq => DialMarks {
            ticks: (0..15)
                .map(|k| {
                    if k == 7 {
                        (0.0, 71.0, 93.0, 7.0)
                    } else {
                        (f64::from(k - 7) * 20.0, 72.0, 85.0, 4.0)
                    }
                })
                .chain([(-156.0, 72.0, 85.0, 4.0), (156.0, 72.0, 85.0, 4.0)])
                .collect(),
            labels: [
                (-7, "–7"),
                (-5, "–5"),
                (-3, "–3"),
                (-1, "–1"),
                (1, "1"),
                (3, "3"),
                (5, "5"),
                (7, "7"),
            ]
            .iter()
            .map(|(n, s)| (f64::from(*n) * 20.0, *s, None))
            .collect(),
            r: 98.0,
            captions: vec![],
        },
        // The strip's: a tick at every step (VOICES's every voice, numbered; DETUNE's every
        // 2.5 cents and DRIVE's every 3 dB, every other numbered), LEVEL's every 5 dB from
        // -30 to +10 and its end, at their places on its travel.
        Dial::Voices => numbered(
            &(0..9)
                .map(|k| (-150.0 + 37.5 * f64::from(k), Some(k)))
                .collect::<Vec<_>>(),
            &["2", "3", "4", "5", "6", "7", "8", "9", "10"],
        ),
        // The arpeggiator's (decisions.md R-ARP): RATE's thirteen note values 25 degrees apart,
        // the straight ones (every third) numbered by their note value; OCTAVES's five a quarter
        // of its travel apart (the owner, 2026-10-19: five "will make the spacing around the
        // knob look better"); GATE's and SWING's every 12.5 %, every other numbered.
        Dial::ArpRate => numbered(
            &(0..13)
                .map(|k| (-150.0 + 25.0 * f64::from(k), (k % 3 == 0).then_some(k / 3)))
                .collect::<Vec<_>>(),
            &["2", "4", "8", "16", "32"],
        ),
        Dial::ArpOctaves => numbered(
            &(0..5)
                .map(|k| (-150.0 + 75.0 * f64::from(k), Some(k)))
                .collect::<Vec<_>>(),
            &["1", "2", "3", "4", "5"],
        ),
        // The expression drawer's MODULATION (decisions.md R-MPE): every 25 %, every other
        // numbered, 0 at the top.
        Dial::Reach => numbered(
            &(0..9)
                .map(|k| (-150.0 + 37.5 * f64::from(k), (k % 2 == 0).then_some(k / 2)))
                .collect::<Vec<_>>(),
            &["–100", "–50", "0", "50", "100"],
        ),
        Dial::Detune | Dial::Percent => numbered(
            &(0..9)
                .map(|k| (-150.0 + 37.5 * f64::from(k), (k % 2 == 0).then_some(k / 2)))
                .collect::<Vec<_>>(),
            &["0", "25", "50", "75", "100"],
        ),
        Dial::Drive => numbered(
            &(0..9)
                .map(|k| (-150.0 + 37.5 * f64::from(k), (k % 2 == 0).then_some(k / 2)))
                .collect::<Vec<_>>(),
            &["0", "6", "12", "18", "24"],
        ),
        Dial::Level => {
            let at = |db: f64| -150.0 + (db + 30.0) * 300.0 / 42.0;
            let mut ticks: Vec<(f64, Option<i32>)> = (0..9)
                .map(|k| {
                    let db = -30.0 + 5.0 * f64::from(k);
                    (at(db), (k % 2 == 0).then_some(k / 2))
                })
                .collect();
            ticks.push((150.0, None));
            numbered(&ticks, &["–30", "–20", "–10", "0", "+10"])
        }
        // An oscillator's FREQUENCY timed (decisions.md R-OSC-TIMED): its eighteen note values
        // over the knob's travel, the straight ones numbered as one count (4× and 2× the bars,
        // then 1 a bar, 2 a half note, to 32), the dotted and triplet ones between them shorter
        // unnumbered ticks, as the arpeggiator's RATE has them. The numbers sit no higher at the
        // top, and no lower at the foot, than the semitone dial's (the owner, 2026-10-20: "the
        // number four runs into the word oscillator at the top"), clear of the legend above it
        // and, under oscillator 2's, of oscillator 3's.
        Dial::Timed => {
            let at = |k: usize| -150.0 + 300.0 * k as f64 / crate::controls::TIMED_STEPS as f64;
            DialMarks {
                ticks: (0..=crate::controls::TIMED_STEPS)
                    .map(|k| {
                        if TIMED_NUMBERS.iter().any(|(n, _)| *n == k) {
                            (at(k), 72.0, 80.0, 4.0)
                        } else {
                            (at(k), 72.0, 77.0, 3.0)
                        }
                    })
                    .collect(),
                labels: TIMED_NUMBERS
                    .iter()
                    .map(|&(k, t)| {
                        let a = at(k);
                        let (x, y) = polar(TIMED_R, a);
                        // (No higher than the semitone dial's ±1; at the ends, no lower than its
                        // ±7, hung out beside their ticks.)
                        let (x, y) = if y < -TIMED_TOP {
                            let r = TIMED_TOP / -rad(a).cos();
                            polar(r, a)
                        } else if y > TIMED_FOOT {
                            (x + x.signum() * TIMED_HUNG, TIMED_FOOT)
                        } else {
                            (x, y)
                        };
                        (x.atan2(-y).to_degrees(), t, Some(x.hypot(y)))
                    })
                    .collect(),
                r: TIMED_R,
                captions: if timed_bars_caption() {
                    vec![(-128.0, 30.0, "BARS")]
                } else {
                    vec![]
                },
            }
        }
        // An LFO's RATE (decisions.md R-LFO): its parameter's skewed travel from 0.01 to 50 Hz
        // (a share `t` of the travel is 0.01 + 49.99 t^5 Hz), ticked at 1, 2 and 5 of each
        // decade, the numbered ones longer (not .5, which crowds 1 and 2), Hz at its foot.
        Dial::LfoRate => {
            let ticks = [
                0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0,
            ];
            let labels = [
                (0.01, ".01"),
                (0.1, ".1"),
                (1.0, "1"),
                (2.0, "2"),
                (5.0, "5"),
                (10.0, "10"),
                (50.0, "50"),
            ];
            DialMarks {
                ticks: ticks
                    .iter()
                    .map(|&f| {
                        let long = labels.iter().any(|l| l.0 == f);
                        (lfo_rate_angle(f), 58.0, if long { 71.0 } else { 66.0 }, 4.0)
                    })
                    .collect(),
                labels: labels
                    .iter()
                    .map(|&(f, t)| (lfo_rate_angle(f), t, None))
                    .collect(),
                r: 86.0,
                captions: vec![(0.0, 82.0, "Hz")],
            }
        }
        // A macro's: the ten's ticks, its ends alone numbered (its top is its name's).
        Dial::Macro => DialMarks {
            ticks: eleven(58.0, 71.0),
            labels: vec![(-150.0, "0", None), (150.0, "10", None)],
            r: 86.0,
            captions: vec![],
        },
        // PHASE: every 45 degrees, every other numbered.
        Dial::LfoPhase => numbered(
            &(0..9)
                .map(|k| (-150.0 + 37.5 * f64::from(k), (k % 2 == 0).then_some(k / 2)))
                .collect::<Vec<_>>(),
            &["0", "90", "180", "270", "360"],
        ),
        // FADE (decisions.md R-LFO-FADE): RISE's dial mirrored about its top, OFF there, a
        // share `t` of the way to either end 10 t^3 s; ticks at .1, .5, 1, 2, 5 and 10 s each
        // way, 1 and the ends' 10 numbered; OUT and IN either side of OFF, over the halves they
        // name (as MODULATION MIX's OSC. 3 and NOISE name its ends); SEC. at its foot as RISE's.
        Dial::LfoFade => {
            let at = |s: f64| 150.0 * s.signum() * (s.abs() / 10.0).cbrt();
            let ticks = [0.1, 0.5, 1.0, 2.0, 5.0, 10.0];
            let numbered = [1.0, 10.0];
            DialMarks {
                ticks: std::iter::once((0.0, 58.0, 76.0, 4.0))
                    .chain(ticks.iter().flat_map(|&s| {
                        let r1 = if numbered.contains(&s) { 71.0 } else { 66.0 };
                        [(at(-s), 58.0, r1, 4.0), (at(s), 58.0, r1, 4.0)]
                    }))
                    .collect(),
                labels: [
                    (0.0, "OFF", None),
                    (-FADE_WAY, "OUT", None),
                    (FADE_WAY, "IN", None),
                ]
                .into_iter()
                .chain(
                    [(1.0, "1"), (10.0, "10")]
                        .into_iter()
                        .flat_map(|(s, t)| [(at(-s), t, None), (at(s), t, None)]),
                )
                .collect(),
                r: 86.0,
                captions: vec![(0.0, 82.0, "SEC.")],
            }
        }
        // Marked in time, not evenly (as printed); the 200 ms mark is printed long ("200—").
        Dial::Time => DialMarks {
            ticks: [
                -150.0, -120.0, -105.0, -90.0, -60.0, -30.0, 0.0, 30.0, 49.0, 67.0, 86.0, 108.0,
                148.0,
            ]
            .iter()
            .map(|&a| (a, 58.0, if a == -90.0 { 78.0 } else { 71.0 }, 4.0))
            .collect(),
            labels: crate::controls::TIME_MARKS
                .iter()
                .zip(["10", "200", "600", "1", "5", "10"])
                .map(|(&(angle, _), label)| {
                    (angle, label, matches!(label, "600" | "1").then_some(80.0))
                })
                .collect(),
            r: 86.0,
            captions: vec![(-61.0, 75.0, "M-SEC."), (50.0, 75.0, "SEC.")],
        },
    }
}

/// Where FADE's OUT and IN stand either side of its OFF (degrees from twelve o'clock).
const FADE_WAY: f64 = 40.0;

/// Where an LFO's RATE dial points at `hz` (degrees from twelve o'clock): its parameter's
/// skewed travel, 0.01 to 50 Hz, the travel's share the fifth root of the range's.
pub fn lfo_rate_angle(hz: f64) -> f64 {
    -150.0 + 300.0 * ((hz - 0.01) / 49.99).clamp(0.0, 1.0).powf(0.2)
}

/// Where a dial's number is printed about its centre: at its own radius if it has one, else at
/// the dial's, wider numbers a little further out at the sides.
fn label_at(d: &DialMarks, a: f64, t: &str, at: Option<f64>, num: f64) -> (f64, f64) {
    let r = at.unwrap_or(
        d.r + 0.3 * num * STRETCH * (t.chars().count() as f64 - 1.0) * rad(a).sin().abs(),
    );
    polar(r, a)
}

pub(crate) fn draw_dial(s: &mut Svg, cx: f64, cy: f64, dial: Dial) {
    draw_dial_in(s, cx, cy, dial, "×");
}

/// The same, `times` the multiplication sign as the typeface prints it ([`Layout::times`]).
fn draw_dial_in(s: &mut Svg, cx: f64, cy: f64, dial: Dial, times: &str) {
    let d = marks(dial);
    let num = s.1.number();
    put!(s, "<g transform='translate({} {})'>", N(cx), N(cy));
    for &(a, r0, r1, w) in &d.ticks {
        let (x0, y0) = polar(r0, a);
        let (x1, y1) = polar(r1, a);
        line(s, x0, y0, x1, y1, w);
    }
    for &(a, t, at) in &d.labels {
        let (x, y) = label_at(&d, a, t, at, num);
        lettered(s, x, y, &t.replace('×', times), num);
    }
    for &(x, y, t) in &d.captions {
        lettered(s, x, y, t, num - 1.0);
    }
    s.0.push_str("</g>");
}

fn legends(set: Legends) -> (&'static [&'static str; 6], bool) {
    match set {
        Legends::Ranges => (&RANGES, false),
        Legends::Waves12 => (&WAVES12, true),
        Legends::Waves3 => (&WAVES3, true),
        Legends::Poles => (&POLES, false),
    }
}

fn set_index(set: Legends) -> usize {
    match set {
        Legends::Ranges => 0,
        Legends::Waves12 => 1,
        Legends::Waves3 => 2,
        Legends::Poles => 3,
    }
}

/// Every set of selector legends.
const LEGEND_SETS: [Legends; 4] = [
    Legends::Ranges,
    Legends::Waves12,
    Legends::Waves3,
    Legends::Poles,
];

/// What the lettering's measures decide: where each selector legend sits, and which titles
/// are spaced out to their printed widths.
#[derive(Clone, Debug)]
pub struct Layout {
    /// Each legend's radius about its selector's axis, by legend set and position.
    legend_r: [[f64; 6]; LEGEND_SETS.len()],
    /// Each title's width if spaced out to its printed one, by its index in the layout's.
    titles: Vec<Option<f64>>,
    /// The lettering's size.
    lettering: Lettering,
    /// Whether the typeface prints the multiplication sign (the timed dials' 4× and 2×,
    /// GLIDE's ×5): a skin pack's may not, which prints an x instead.
    times: bool,
}

/// The extent of a pictogram's path (its commands are absolute M, L, H and V).
fn path_box(d: &str) -> [f64; 4] {
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    let (mut x, mut y) = (0.0, 0.0);
    let mut words = d.split_whitespace();
    while let Some(cmd) = words.next() {
        let mut num = || {
            words
                .next()
                .and_then(|w| w.parse::<f64>().ok())
                .unwrap_or(0.0)
        };
        match cmd {
            "M" | "L" => {
                x = num();
                y = num();
            }
            "H" => x = num(),
            "V" => y = num(),
            _ => continue,
        }
        b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
    }
    b
}

impl Layout {
    /// Measures the lettering of the official layout, as the mock-up did once the page's fonts
    /// had loaded.
    pub fn measure(fonts: &Fonts) -> Self {
        Self::measure_in(fonts, Lettering::Standard, &Arrangement::official())
    }

    /// The same, the lettering printed at `lettering`'s sizes, its titles `arrangement`'s.
    pub fn measure_in(fonts: &Fonts, lettering: Lettering, arrangement: &Arrangement) -> Self {
        let num = lettering.number();
        let (ascent, descent) = fonts.extent(Weight::Bold);
        let mut legend_r = [[SIX_LEGEND; 6]; LEGEND_SETS.len()];
        for set in LEGEND_SETS {
            let (texts, glyphs) = legends(set);
            for i in set.places() {
                let a = SIX[i];
                // The ink's box about the legend's centre (through its own scale), with its
                // stroke: a text's from its start to its advance, ascent to descent.
                let b = if glyphs {
                    let p = path_box(texts[i]);
                    [
                        p[0] * GLYPH - 1.5,
                        p[1] * GLYPH - 1.5,
                        p[2] * GLYPH + 1.5,
                        p[3] * GLYPH + 1.5,
                    ]
                } else {
                    let adv = fonts.advance(texts[i], num, Weight::Bold, 0.0);
                    let y0 = num * 0.36 - ascent * num;
                    [
                        -adv / 2.0 * STRETCH - 0.35,
                        y0 - 0.35,
                        adv / 2.0 * STRETCH + 0.35,
                        y0 + (ascent + descent) * num + 0.35,
                    ]
                };
                let (x1, y1) = polar(SIX_TICK.0, a);
                let (x2, y2) = polar(SIX_TICK.1, a);
                let clear = |(cx, cy): (f64, f64)| {
                    let mut least = f64::INFINITY;
                    for k in 0..=50 {
                        let t = f64::from(k) * 0.02;
                        let x = x1 + (x2 - x1) * t - cx;
                        let y = y1 + (y2 - y1) * t - cy;
                        let dx = (b[0] - x).max(0.0).max(x - b[2]);
                        let dy = (b[1] - y).max(0.0).max(y - b[3]);
                        least = least.min(dx.hypot(dy) - 2.0);
                    }
                    least
                };
                let mut r = SIX_LEGEND;
                while clear(polar(r, a)) < SIX_CLEAR && r < SIX_LEGEND + 20.0 {
                    r += 0.5;
                }
                legend_r[set_index(set)][i] = r;
            }
        }
        // A title spans its printed width, spaced out to it, never squeezed: one wider by
        // nature is set at its own width, centred.
        let titles = arrangement
            .titles
            .iter()
            .map(|t| {
                let (x0, x1) = t.span;
                (fonts.advance(&t.text, t.size, Weight::Bold, 0.0) <= x1 - x0).then_some(x1 - x0)
            })
            .collect();
        Self {
            legend_r,
            titles,
            lettering,
            times: fonts.covers("×"),
        }
    }

    /// The multiplication sign, or an x where the typeface has none.
    pub fn times(&self) -> &'static str {
        if self.times { "×" } else { "x" }
    }

    /// The lettering's size.
    pub fn lettering(&self) -> Lettering {
        self.lettering
    }

    /// Where a selector's legend `i` is, about its axis.
    pub fn legend_at(&self, set: Legends, i: usize) -> (f64, f64) {
        let (x, y) = polar(self.legend_r[set_index(set)][i], SIX[i]);
        ((x * 100.0).round() / 100.0, (y * 100.0).round() / 100.0)
    }
}

/// An SVG document showing `view` (x, y, width, height, in panel units) at `px` by `py`
/// pixels.
pub fn document(view: [f64; 4], px: u32, py: u32, body: &str) -> String {
    format!(
        "<svg xmlns='http://www.w3.org/2000/svg' viewBox='{} {} {} {}' width='{px}' height='{py}' font-family='{FAMILY}'>{}{body}</svg>",
        N(view[0]),
        N(view[1]),
        N(view[2]),
        N(view[3]),
        defs()
    )
}

/// Everything that does not move: the wood, the panel's face and lettering, the dials'
/// marks, and the parts with no parameter (PHONES, the controller's jacks), dimmed.
pub fn background(layout: &Layout, arrangement: &Arrangement) -> String {
    let mut s = Svg(String::new(), layout.lettering);
    wood(&mut s, arrangement);
    panel(&mut s, layout, arrangement, Ink::All);
    strip_face(&mut s);
    crate::strip::surfaces(&mut s);
    column(&mut s, Ink::All);
    strip_print(&mut s, arrangement);
    grip(&mut s);
    s.0
}

/// What a part of the drawing draws: all of it, its print alone (the worn skin lays the
/// print over pictures of the surfaces: decisions.md R44), or its surfaces alone (a skin pack's
/// drawing goes between them and the print: decisions.md R-SKIN-PACKS).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ink {
    All,
    Print,
    Surface,
}

/// The background with the panel's surfaces but not its print ([`panel_print`]): a skin pack's
/// drawing goes between them.
pub fn background_surfaces(layout: &Layout, arrangement: &Arrangement) -> String {
    let mut s = Svg(String::new(), layout.lettering);
    wood(&mut s, arrangement);
    panel(&mut s, layout, arrangement, Ink::Surface);
    strip_face(&mut s);
    crate::strip::surfaces(&mut s);
    column(&mut s, Ink::All);
    strip_print(&mut s, arrangement);
    grip(&mut s);
    s.0
}

/// The panel's print alone: its rules, titles, dials' marks and legends.
/// The panel's surfaces alone (the wood and the face, without the strip's): what a part of the
/// panel's background is painted on (`Renderer::timed_part`, R-OSC-TIMED).
pub fn panel_surfaces(layout: &Layout, arrangement: &Arrangement) -> String {
    let mut s = Svg(String::new(), layout.lettering);
    wood(&mut s, arrangement);
    panel(&mut s, layout, arrangement, Ink::Surface);
    s.0
}

/// Oscillators 2's and 3's FREQUENCY dials' print alone, as `arrangement` has their dials (the
/// semitone dial, or the timed one: R-OSC-TIMED), where the panel's print puts them.
pub fn dials_print(layout: &Layout, arrangement: &Arrangement) -> String {
    let mut s = Svg(String::new(), layout.lettering);
    put!(s, "<g transform='translate({} {})'>", N(COL), N(TOP));
    for c in arrangement
        .controls
        .iter()
        .filter(|c| c.place == Place::Panel)
    {
        if let Kind::Knob {
            dial: dial @ (Dial::Freq | Dial::Timed),
            ..
        } = c.kind
        {
            draw_dial_in(&mut s, c.x, c.y, dial, layout.times());
        }
    }
    s.0.push_str("</g>");
    s.0
}

/// LONG's ×5 beside GLIDE, below and to the right of its numbers (R-GLIDE-LONG): its print's
/// box about the knob's centre, with room for LARGE and the ink's blur into the face.
pub const GLIDE_LONG_EXTENT: [f64; 4] = [48.0, 88.0, 108.0, 117.0];

/// GLIDE's LONG sign alone, in the legends' ink, size and typeface. A pack without × prints x.
pub fn glide_long_print(layout: &Layout, arrangement: &Arrangement) -> String {
    let mut s = Svg(String::new(), layout.lettering);
    if let Some(c) = arrangement
        .controls
        .iter()
        .find(|c| c.param == "glide" && c.place == Place::Panel)
    {
        put!(s, "<g transform='translate({} {})'>", N(COL), N(TOP));
        legend(
            &mut s,
            c.x + 78.0,
            c.y + 100.0,
            &format!("{}5", layout.times()),
        );
        s.0.push_str("</g>");
    }
    s.0
}

/// Where a WAVEFORM switch's place printed CONTINUOUS differs about its centre (decisions.md
/// R-CONTINUOUS-WAVE): its arc from TRIANGLE's tick round to the floor's, over the ticks' ring,
/// and oscillator 3's second and third legends, a little over for the print's stroke and its
/// blur into the face. (Clear of the FREQUENCY dials' print on its left: `Renderer`'s test.)
pub const ARC_EXTENT: [f64; 4] = [-82.0, -106.0, 80.0, 40.0];

/// Oscillator `n`'s WAVEFORM switch's print alone (its ticks and legends), as `arrangement`
/// places it, CONTINUOUS's with `continuous`: its arc, and oscillator 3's legends its
/// reverse shark tooth and reverse sawtooth (`Renderer::arc_part`).
pub fn waveform_print(
    layout: &Layout,
    arrangement: &Arrangement,
    n: usize,
    continuous: bool,
) -> String {
    let mut s = Svg(String::new(), layout.lettering);
    put!(s, "<g transform='translate({} {})'>", N(COL), N(TOP));
    let param = ["osc1_waveform", "osc2_waveform", "osc3_waveform"][n];
    if let Some(c) = arrangement
        .controls
        .iter()
        .find(|c| c.param == param && c.place == Place::Panel)
        && let Kind::Selector(set) = c.kind
    {
        let texts = (continuous && set == Legends::Waves3).then_some(&WAVES3_CONTINUOUS);
        selector_marks_with(&mut s, c, set, layout, texts);
        if continuous {
            shape_arc(&mut s, c.x, c.y);
        }
    }
    s.0.push_str("</g>");
    s.0
}

/// CONTINUOUS's arc about a WAVEFORM switch at (x, y) (the owner, 2026-10-20: "I love the arc.
/// I think it's very elegant"): a hairline in the legends' ink through the ticks' ring from
/// TRIANGLE's tick round through every place to a short tick at the floor.
fn shape_arc(s: &mut Svg, x: f64, y: f64) {
    let r = (SIX_TICK.0 + SIX_TICK.1) / 2.0;
    let end = shape_angle(SHAPE_PLACES);
    let (x0, y0) = polar(r, SIX[0]);
    let (x1, y1) = polar(r, end);
    let (a0, b0) = polar(SIX_TICK.0 + 2.5, end);
    let (a1, b1) = polar(SIX_TICK.1 - 2.5, end);
    let large = u8::from(end - SIX[0] > 180.0);
    put!(
        s,
        "<g transform='translate({} {})'><path d='M {} {} A {} {} 0 {large} 1 {} {}' fill='none' stroke='{LEGEND}' stroke-width='{ARC_W}'/><line x1='{}' y1='{}' x2='{}' y2='{}' stroke='{LEGEND}' stroke-width='{ARC_W}'/></g>",
        N(x),
        N(y),
        N(x0),
        N(y0),
        N(r),
        N(r),
        N(x1),
        N(y1),
        N(a0),
        N(b0),
        N(a1),
        N(b1)
    );
}

/// The arc's stroke, units (the approved mock-up's: about a pixel at the opening size).
const ARC_W: f64 = 2.4;

pub fn panel_print(layout: &Layout, arrangement: &Arrangement) -> String {
    let mut s = Svg(String::new(), layout.lettering);
    panel(&mut s, layout, arrangement, Ink::Print);
    s.0
}

/// The strip's print alone (the left hand's legends, the strip's, the grip's lines): laid again
/// over a skin pack's drawing of the strip in the drawn skin.
pub fn strip_printed(layout: &Layout, arrangement: &Arrangement) -> String {
    let mut s = Svg(String::new(), layout.lettering);
    column(&mut s, Ink::Print);
    strip_print(&mut s, arrangement);
    grip(&mut s);
    s.0
}

/// The print alone, as the background has it: the face's and the column's lettering, rules,
/// dials' marks and selectors' legends, and the grip's lines in the corner.
pub fn printed(layout: &Layout, arrangement: &Arrangement) -> String {
    let mut s = Svg(String::new(), layout.lettering);
    panel(&mut s, layout, arrangement, Ink::Print);
    column(&mut s, Ink::Print);
    strip_print(&mut s, arrangement);
    grip(&mut s);
    s.0
}

/// The strip's face, in the drawn skin (the worn skin's is a picture): the panel's black,
/// under the rail, with the panel's trim at its ends and foot.
fn strip_face(s: &mut Svg) {
    let top = PANEL_H + crate::strip::RAIL;
    put!(
        s,
        "<rect x='0' y='{}' width='{}' height='{}' fill='{PANEL}'/>",
        N(top),
        N(W),
        N(H - top)
    );
    put!(s, "<g transform='translate(0 {})'>", N(top));
    strip_trims(s);
    s.0.push_str("</g>");
}

/// The aluminium trim at the strip face's ends and along its foot, in its own place (its top
/// left corner under the rail).
pub(crate) fn strip_trims(s: &mut Svg) {
    let h = H - PANEL_H - crate::strip::RAIL;
    put!(
        s,
        "<rect x='0' y='0' width='15' height='{}' fill='url(#trim)'/>",
        N(h)
    );
    put!(
        s,
        "<rect x='{}' y='0' width='28' height='{}' fill='url(#trim)'/>",
        N(W - 28.0),
        N(h)
    );
    put!(
        s,
        "<rect x='0' y='{}' width='{}' height='6' fill='url(#trim)'/>",
        N(h - 6.0),
        N(W)
    );
}

/// The strip's print (`strip::print`) and its knobs' dials.
fn strip_print(s: &mut Svg, arrangement: &Arrangement) {
    crate::strip::print(s);
    for c in arrangement
        .controls
        .iter()
        .filter(|c| c.place == Place::Strip)
    {
        if let Kind::Knob { dial, .. } = c.kind {
            let (x, y) = c.centre();
            draw_dial(s, x, y, dial);
        }
    }
}

/// The top strip's screws: centre and radius, in the drawing (the layout's, in the face's
/// units).
fn top_screws(arrangement: &Arrangement) -> impl Iterator<Item = (f64, f64, f64)> + '_ {
    arrangement
        .screws
        .iter()
        .map(|&(x, y, r)| (COL + x, TOP + y, r))
}

/// The left hand's screws, in its own place: its switches' and its wheels' mountings (no
/// plate of its own on the strip's face, nor its corners' screws: the owner, 2026-10-09, "no
/// need to put a back plate on top of another backplate").
fn column_screws() -> Vec<(f64, f64, f64)> {
    let mut mountings = Vec::new();
    for y in LH_ROWS.into_iter().chain([LH_RETRIGGER]) {
        for x in [LH_ROCKER - LH_SCREW_DX, LH_ROCKER + LH_SCREW_DX] {
            mountings.push((x, y, 5.0));
        }
    }
    for x in LH_WHEELS {
        for dy in [-64.0, 64.0] {
            mountings.push((
                x - (30.0 * WHEEL_SCALE + 10.0),
                WHEEL_Y + dy * WHEEL_SCALE,
                5.0,
            ));
        }
    }
    mountings
}

/// Every screw's centre and radius, in the drawing.
pub fn screws(arrangement: &Arrangement) -> Vec<(f64, f64, f64)> {
    let (lx, ly) = crate::strip::LEFT_HAND;
    top_screws(arrangement)
        .chain(
            column_screws()
                .into_iter()
                .map(|(x, y, r)| (lx + x, ly + y, r)),
        )
        .collect()
}

/// The jacks with no parameter (the controller's GLIDE and DECAY, the panel's PHONES): centre,
/// radius and whether a nut is round it, in the drawing.
pub fn jacks(arrangement: &Arrangement) -> Vec<(f64, f64, f64, bool)> {
    let (lx, ly) = crate::strip::LEFT_HAND;
    LH_ROWS
        .iter()
        .chain([&LH_VELOCITY])
        .map(|y| (lx + LH_JACK, ly + y, 22.0, true))
        .chain(
            arrangement
                .phones
                .map(|(x, y, r)| (COL + x, TOP + y, r, false)),
        )
        .collect()
}

fn wood(s: &mut Svg, arrangement: &Arrangement) {
    // The top strip (the panel's size's, if any) and the strip's rail. No cheeks: the ends
    // are square.
    let p = arrangement.panel;
    if p.wood > 0.0 {
        put!(
            s,
            "<rect x='0' y='{}' width='{}' height='{}' fill='url(#wood-h)'/>",
            N(p.top()),
            N(W),
            N(p.wood)
        );
    }
    put!(
        s,
        "<rect x='0' y='{}' width='{}' height='{}' fill='url(#wood-h)'/>",
        N(PANEL_H),
        N(W),
        N(crate::strip::RAIL)
    );
    // Grain: long, gently wavering lines (the same every time).
    let mut seed: u64 = 7;
    let mut rnd = || {
        seed = seed * 16807 % 2_147_483_647;
        seed as f64 / 2_147_483_647.0
    };
    let mut grain = |s: &mut Svg, x0: f64, y0: f64, w: f64, h: f64, across: bool, n: u32| {
        for i in 0..n {
            let t = (f64::from(i) + 0.2 + rnd() * 0.6) / f64::from(n);
            let mut pts = Vec::with_capacity(9);
            for k in 0..=8 {
                let u = f64::from(k) / 8.0;
                let wob = (rnd() - 0.5) * if across { h } else { w } * 0.05;
                pts.push(if across {
                    format!("{:.1},{:.1}", x0 + u * w, y0 + t * h + wob)
                } else {
                    format!("{:.1},{:.1}", x0 + t * w + wob, y0 + u * h)
                });
            }
            put!(
                s,
                "<polyline points='{}' fill='none' stroke='{WOOD_DARK}' stroke-width='{}' stroke-opacity='0.3'/>",
                pts.join(" "),
                N(1.5 + rnd() * 2.5)
            );
        }
    };
    // The top strip's grain laid out as when the left hand's column stood left of the panel,
    // 330 units wide: the panel's wood stays as approved.
    if p.wood > 12.0 {
        grain(s, -330.0, p.top() + 6.0, W + 330.0, p.wood - 12.0, true, 7);
    }
    grain(s, 0.0, PANEL_H + 6.0, W, crate::strip::RAIL - 12.0, true, 6);
    for (x, y, r) in top_screws(arrangement) {
        screw(s, x, y, r);
    }
}

/// The resize grip's four lines in the strip's bottom right corner, on its face inside the
/// trim, drawn last, plain to see (R-SIZE: people had not found it; drawn at the drawing's
/// corner, under the trim, it had not shown in CLASSIC and only in part in REALISTIC).
fn grip(s: &mut Svg) {
    let (x, y) = GRIP_CORNER;
    for k in 1..=4 {
        let d = 11.0 * f64::from(k);
        put!(
            s,
            "<line x1='{}' y1='{}' x2='{}' y2='{}' stroke='{LEGEND}' stroke-opacity='0.65' stroke-width='4' stroke-linecap='round'/>",
            N(x - d),
            N(y),
            N(x),
            N(y - d)
        );
    }
}

/// The face and its print, as `arrangement` lays them out: the rules, the section titles,
/// the dials' marks and the selectors' legends, the legends, and the PHONES jack.
fn panel(s: &mut Svg, layout: &Layout, arrangement: &Arrangement, ink: Ink) {
    put!(s, "<g transform='translate({} {})'>", N(COL), N(TOP));
    if ink != Ink::Print {
        // (Its top in the face's units: 0 on the official panel.)
        let face = arrangement.panel.face;
        let y = PH - face;
        put!(
            s,
            "<rect x='0' y='{}' width='{}' height='{}' fill='{PANEL}'/>",
            N(y),
            N(PW),
            N(face)
        );
        put!(
            s,
            "<rect x='0' y='{}' width='{}' height='7' fill='{SHADOW}' fill-opacity='{SHADOW_OPACITY}'/>",
            N(y),
            N(PW)
        );
        put!(s, "<g transform='translate(0 {})'>", N(y));
        trims(s, face);
        s.0.push_str("</g>");
    }
    if ink != Ink::Surface {
        print(s, layout, arrangement);
    }
    // The PHONES jack has no parameter: drawn for the panel's sake, dimmed.
    if ink != Ink::Print
        && let Some((x, y, r)) = arrangement.phones
    {
        put!(
            s,
            "<g transform='translate({} {})' opacity='0.5'>",
            N(x),
            N(y)
        );
        jack(s, r, false);
        s.0.push_str("</g>");
    }
    s.0.push_str("</g>");
}

/// The face's print, in its own place.
fn print(s: &mut Svg, layout: &Layout, arrangement: &Arrangement) {
    for l in &arrangement.lines {
        line(s, l.from.0, l.from.1, l.to.0, l.to.1, l.width);
    }
    // Section titles along the foot, as wide as printed.
    for (i, t) in arrangement.titles.iter().enumerate() {
        let spaced = layout
            .titles
            .get(i)
            .copied()
            .flatten()
            .map_or(String::new(), |w| {
                format!("textLength='{}' lengthAdjust='spacing'", N(w))
            });
        text(
            s,
            (t.span.0 + t.span.1) / 2.0,
            t.baseline - t.size * 0.36,
            &t.text,
            t.size,
            Style::Title,
            false,
            &spaced,
        );
    }
    // The dials' marks and the selectors' ticks and legends.
    for c in arrangement
        .controls
        .iter()
        .filter(|c| c.place == Place::Panel)
    {
        match c.kind {
            Kind::Knob { dial, .. } => draw_dial_in(s, c.x, c.y, dial, layout.times()),
            Kind::Selector(set) => selector_marks(s, c, set, layout),
            _ => {}
        }
    }
    for t in &arrangement.texts {
        let (x, y) = t.at_in(s.1);
        lettered(s, x, y, &t.text, t.size_in(s.1));
    }
}

/// The aluminium trim at the face's ends and along its foot, in the face's own place (its top
/// left corner), the face `face` tall.
pub(crate) fn trims(s: &mut Svg, face: f64) {
    put!(
        s,
        "<rect x='0' y='0' width='15' height='{}' fill='url(#trim)'/>",
        N(face)
    );
    put!(
        s,
        "<rect x='{}' y='0' width='28' height='{}' fill='url(#trim)'/>",
        N(PW - 28.0),
        N(face)
    );
    put!(
        s,
        "<rect x='0' y='{}' width='{}' height='6' fill='url(#trim)'/>",
        N(face - 6.0),
        N(PW)
    );
}

fn selector_marks(s: &mut Svg, c: &Control, set: Legends, layout: &Layout) {
    selector_marks_with(s, c, set, layout, None);
}

/// A selector's ticks and legends, its legends `texts` where given (its set's places).
fn selector_marks_with(
    s: &mut Svg,
    c: &Control,
    set: Legends,
    layout: &Layout,
    texts: Option<&[&str; 6]>,
) {
    put!(s, "<g transform='translate({} {})'>", N(c.x), N(c.y));
    for a in set.places().map(|k| SIX[k]) {
        let (x0, y0) = polar(SIX_TICK.0, a);
        let (x1, y1) = polar(SIX_TICK.1, a);
        put!(
            s,
            "<line x1='{}' y1='{}' x2='{}' y2='{}' stroke='{LEGEND}' stroke-width='4'/>",
            N(x0),
            N(y0),
            N(x1),
            N(y1)
        );
    }
    let (own, glyphs) = legends(set);
    for (i, t) in texts.unwrap_or(own).iter().enumerate() {
        if !set.places().contains(&i) {
            continue;
        }
        let (x, y) = layout.legend_at(set, i);
        put!(s, "<g transform='translate({} {})'>", N(x), N(y));
        if glyphs {
            put!(
                s,
                "<path d='{t}' transform='scale({GLYPH})' fill='none' stroke='{LEGEND}' stroke-width='{}' stroke-linecap='round' stroke-linejoin='round'/>",
                N(3.0 / GLYPH)
            );
        } else {
            let num = s.1.number();
            lettered(s, 0.0, 0.0, t, num);
        }
        s.0.push_str("</g>");
    }
    s.0.push_str("</g>");
}

/// A jack's chrome ring; `nut`: with the hex nut that holds it.
fn jack(s: &mut Svg, r: f64, nut: bool) {
    if nut {
        let pts: Vec<String> = (0..6)
            .map(|k| {
                let (x, y) = polar(r + 6.0, 30.0 + 60.0 * f64::from(k));
                format!("{x:.2},{y:.2}")
            })
            .collect();
        put!(
            s,
            "<polygon points='{}' fill='url(#cap)' stroke='{CAP_DARK}' stroke-width='1.5'/>",
            pts.join(" ")
        );
    }
    shadow(s, 3.0, 4.0, r + 1.0);
    put!(
        s,
        "<circle r='{}' fill='url(#cap)' stroke='{CAP_DARK}' stroke-width='1.5'/>",
        N(r)
    );
    put!(
        s,
        "<circle r='{}' fill='none' stroke='{CAP_DARK}' stroke-width='3'/>",
        N(r * 0.66)
    );
    put!(s, "<circle r='{}' fill='{HOLE}'/>", N(r * 0.36));
}

/// The left hand controller: GLIDE and DECAY, then the PITCH and MODULATION wheels, at the
/// strip's left, straight on its face (A6).
fn column(s: &mut Svg, ink: Ink) {
    let (lx, ly) = crate::strip::LEFT_HAND;
    put!(s, "<g transform='translate({} {})'>", N(lx), N(ly));
    if ink == Ink::All {
        let mountings = column_screws();
        // The GLIDE and DECAY jacks have no parameter: drawn, dimmed.
        for y in LH_ROWS {
            put!(
                s,
                "<g transform='translate({} {})' opacity='0.5'>",
                N(LH_JACK),
                N(y)
            );
            jack(s, 22.0, true);
            s.0.push_str("</g>");
        }
        // VELOCITY's is the velocity source's (decisions.md R-VELOCITY): drawn whole.
        put!(
            s,
            "<g transform='translate({} {})'>",
            N(LH_JACK),
            N(LH_VELOCITY)
        );
        jack(s, crate::strip::velocity::JACK_R, true);
        s.0.push_str("</g>");
        // The switches' and the wheels' mounting screws.
        for (x, y, r) in mountings {
            screw(s, x, y, r);
        }
    }
    legend(s, LH_LABEL, LH_ROWS[0], "GLIDE");
    legend(s, LH_LABEL, LH_ROWS[1], "DECAY");
    let (vx, vy) = crate::strip::velocity::legend_at();
    legend(s, vx, vy, "VELOCITY");
    let num = s.1.number();
    lettered(s, LH_ROCKER + 36.0, LH_ROWS[0] + 34.0, "ON", num);
    lettered(s, LH_ROCKER + 36.0, LH_ROWS[1] + 34.0, "ON", num);
    // NOTE PRIORITY, its lever's places round it as QUALITY's are (LAST, the lever out, above
    // it; LOW and HIGH under its ends), and RETRIGGER, its ON as GLIDE's.
    legend(s, LH_PRIORITY_LABEL, LH_PRIORITY, "PRIORITY");
    let retrigger = match s.1 {
        Lettering::Standard => LH_RETRIGGER_LABEL,
        Lettering::Large => LH_RETRIGGER_LABEL_LARGE,
    };
    legend(s, retrigger, LH_RETRIGGER, "RETRIGGER");
    lettered(s, LH_ROCKER + 36.0, LH_RETRIGGER + 34.0, "ON", num);
    let (px, py) = (LH_ROCKER, LH_PRIORITY);
    lettered(s, px, py - 33.0, "LAST", num);
    lettered(s, px - 40.0, py + 38.0, "LOW", num);
    lettered(s, px + 40.0, py + 38.0, "HIGH", num);
    let label_y = WHEEL_Y + 73.0 * WHEEL_SCALE + 16.0;
    legend(s, LH_WHEELS[0], label_y, "PITCH");
    legend(s, LH_WHEELS[1], label_y, "MOD.");
    s.0.push_str("</g>");
}

// ---- The parts that move.

/// The fluted black body seen from above: nine broad, shallow, smooth flutes.
fn fluted(s: &mut Svg, r: f64) {
    let lobes = 9.0;
    let n = 9 * 16;
    s.0.push_str("<path d='");
    for k in 0..n {
        let a = f64::from(k) / f64::from(n) * 360.0;
        let (x, y) = polar(r - 3.5 * (1.0 - rad(a * lobes).cos()), a);
        put!(s, "{} {x:.2} {y:.2} ", if k == 0 { "M" } else { "L" });
    }
    put!(
        s,
        "Z' fill='url(#skirt)' stroke='{KNOB_RIM}' stroke-opacity='0.6' stroke-width='1.2'/>"
    );
}

/// The light on a black knob's shoulder (between its cap at `r0` and its edge at `r1`): one
/// soft highlight at the upper left, where the light is (it does not turn).
fn shine(s: &mut Svg, r0: f64, r1: f64) {
    let (x, y) = polar((r0 + r1) / 2.0, -45.0);
    put!(
        s,
        "<ellipse cx='{}' cy='{}' rx='{}' ry='{}' transform='rotate(-45 {} {})' fill='url(#gloss)'/>",
        N(x),
        N(y),
        N((r1 - r0) * 0.75),
        N((r1 - r0) * 0.38),
        N(x),
        N(y)
    );
}

/// A spun aluminium cap: its two faint wedges.
fn aluminium(s: &mut Svg, r: f64) {
    put!(
        s,
        "<circle r='{}' fill='url(#cap-flat)' stroke='{CAP_DARK}' stroke-width='1.5'/>",
        N(r)
    );
    put!(s, "<circle r='{}' fill='url(#cap-shade)'/>", N(r));
    let (ax, ay) = polar(r, -40.0);
    let (bx, by) = polar(r, -20.0);
    let (cx, cy) = polar(r, 140.0);
    let (dx, dy) = polar(r, 160.0);
    let r = N(r);
    put!(
        s,
        "<path d='M 0 0 L {} {} A {r} {r} 0 0 1 {} {} Z M 0 0 L {} {} A {r} {r} 0 0 1 {} {} Z' fill='{CAP_LIGHT}' fill-opacity='0.16'/>",
        N(ax),
        N(ay),
        N(bx),
        N(by),
        N(cx),
        N(cy),
        N(dx),
        N(dy)
    );
}

/// A knob at the origin turned `deg` from twelve o'clock: glossy black, its base's
/// silhouette fluted, its pointer a white dot on the shoulder outside the cap.
fn knob_body(s: &mut Svg, size: Size, deg: f64) {
    shadow(s, 3.0, 5.0, size.skirt + 1.0);
    put!(s, "<g transform='rotate({deg:.2})'>");
    fluted(s, size.skirt);
    put!(
        s,
        "<circle cx='0' cy='{}' r='{}' fill='{LEGEND}'/>",
        N(-size.dot),
        N(size.dot_r)
    );
    s.0.push_str("</g>");
    shine(s, size.cap, size.skirt);
    aluminium(s, size.cap);
}

/// A pointer knob at position `i` of six: the fluted black body, a fin along its pointer
/// carrying a white line from the cap to its tip, a spun aluminium cap.
/// QUALITY's toggle as drawn: a chrome nut and its bushing, the lever's ball tip up, out (at
/// the nut's middle) or down, `i` 2, 1 or 0.
fn toggle_body(s: &mut Svg, i: usize) {
    put!(
        s,
        "<polygon points='{}' fill='#b8b8b2' stroke='#5e5e5a' stroke-width='1.2'/><circle r='11' fill='#d6d6d0' stroke='#7a7a75'/><circle r='7.5' fill='#8c8c87'/>",
        (0..6)
            .map(|k| {
                let a = (30.0 + 60.0 * f64::from(k)).to_radians();
                format!("{},{}", N(17.0 * a.cos()), N(17.0 * a.sin()))
            })
            .collect::<Vec<_>>()
            .join(" ")
    );
    match i {
        1 => put!(
            s,
            "<circle cy='1.5' r='5.5' fill='#f0f0ea' stroke='#8a8a85'/>"
        ),
        _ => {
            let d = if i == 2 { -1.0 } else { 1.0 };
            put!(
                s,
                "<line x1='0' y1='0' x2='0' y2='{}' stroke='#e2e2dc' stroke-width='6' stroke-linecap='round'/><circle cy='{}' r='5' fill='#f2f2ec' stroke='#8a8a85'/>",
                N(28.0 * d),
                N(31.0 * d)
            );
        }
    }
}

fn selector_body(s: &mut Svg, angle: f64) {
    shadow(s, 3.0, 5.0, 57.0);
    put!(s, "<g transform='rotate({})'>", N(angle));
    fluted(s, 56.0);
    put!(
        s,
        "<path d='M -17 12 L -17 -20 L -8 -46 Q 0 -52 8 -46 L 17 -20 L 17 12 Q 0 22 -17 12 Z' transform='scale(1.15)' fill='url(#skirt)' stroke='{KNOB_RIM}' stroke-opacity='0.6' stroke-width='1'/>"
    );
    put!(
        s,
        "<line x1='0' y1='-28' x2='0' y2='-56' stroke='{LEGEND}' stroke-width='5' stroke-linecap='round'/>"
    );
    s.0.push_str("</g>");
    shine(s, 26.0, 56.0);
    aluminium(s, 26.0);
}

fn colour_name(c: Colour) -> &'static str {
    match c {
        Colour::Blue => "blue",
        Colour::Red => "red",
        Colour::Black => "black",
        Colour::Ivory => "ivory",
    }
}

/// A rocker's paddle, `w` by `h`, its "on" end at +x and pressed when `on`, else mirrored.
fn paddle(s: &mut Svg, w: f64, h: f64, colour: Colour, on: bool) {
    let c = colour_name(colour);
    put!(
        s,
        "<rect x='{}' y='{}' width='{}' height='{}' rx='3' fill='{HOLE}'/>",
        N(-w / 2.0 - 1.5),
        N(-h / 2.0 - 1.5),
        N(w + 3.0),
        N(h + 3.0)
    );
    s.0.push_str(if on {
        "<g>"
    } else {
        "<g transform='scale(-1 1)'>"
    });
    put!(
        s,
        "<rect x='0' y='{}' width='{}' height='{}' rx='3' fill='url(#rk-{c}-p)'/>",
        N(-h / 2.0 + 3.0),
        N(w / 2.0),
        N(h - 3.0)
    );
    // The raised half, its outer end rounded over.
    let (top, bottom, end, r) = (-h / 2.0 - 3.0, h / 2.0, -w / 2.0, (h * 0.2).min(8.0));
    put!(
        s,
        "<path d='M 2 {} H {} Q {} {} {} {} V {} Q {} {} {} {} H 2 Z' fill='url(#rk-{c}-r)'/>",
        N(top),
        N(end + r),
        N(end),
        N(top),
        N(end),
        N(top + r),
        N(bottom - r),
        N(end),
        N(bottom),
        N(end + r),
        N(bottom)
    );
    s.0.push_str("</g>");
}

/// A wheel seen from above in its slot, at `shown` (0..1) of its travel: its ribs and mark
/// roll with it.
fn wheel(s: &mut Svg, mark: Mark, span: f64, shown: f64) {
    let sh = SLOT_H;
    put!(s, "<g transform='scale({WHEEL_SCALE})'>");
    put!(
        s,
        "<rect x='{}' y='{}' width='{}' height='{}' rx='7' fill='{HOLE}'/>",
        N(-SLOT_W / 2.0),
        N(-sh / 2.0),
        N(SLOT_W),
        N(sh)
    );
    put!(
        s,
        "<rect x='-24' y='{}' width='48' height='{}' rx='10' fill='url(#ivory)'/>",
        N(-sh / 2.0 + 6.0),
        N(sh - 12.0)
    );
    // The wheel's turn: `span` degrees over its travel, centred.
    let turn = (shown - 0.5) * span;
    let rib = 8.0;
    let r = 84.0;
    let mut a = -64.0 + turn.rem_euclid(rib);
    while a <= 64.0 {
        let y = -r * rad(a).sin();
        if y.abs() <= sh / 2.0 - 9.0 {
            put!(
                s,
                "<line x1='-22' y1='{}' x2='22' y2='{}' stroke='{IVORY_DARK}' stroke-width='2.4' stroke-opacity='{:.2}'/>",
                N(y),
                N(y),
                rad(a).cos()
            );
        }
        a += rib;
    }
    let at = if mark == Mark::Line { turn } else { turn - 7.0 };
    let y = -r * rad(at).sin();
    if at.abs() < 60.0 && y.abs() < sh / 2.0 - 9.0 {
        match mark {
            Mark::Line => put!(
                s,
                "<line x1='-22' y1='{}' x2='22' y2='{}' stroke='{KNOB}' stroke-width='4'/>",
                N(y),
                N(y)
            ),
            Mark::Dot => put!(s, "<circle cx='0' cy='{}' r='5' fill='{KNOB}'/>", N(y)),
        }
    }
    put!(
        s,
        "<rect x='-24' y='{}' width='48' height='{}' rx='10' fill='url(#ivory-sides)'/>",
        N(-sh / 2.0 + 6.0),
        N(sh - 12.0)
    );
    s.0.push_str("</g>");
}

/// A part of the drawing that moves: its art about `origin` (in the drawing), within `bounds`
/// of it (left, top, right, bottom).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layer {
    pub origin: (f64, f64),
    pub bounds: [f64; 4],
    pub body: String,
    /// The worn skin's pictures drawn over the body, each about the origin (decisions.md
    /// R44), lights over them, then what does not change drawn once and kept (the lamp's
    /// light on a knob, which does not turn with it), what is drawn by code (QUALITY's
    /// opening's shutter and lightning), and SVG over all of those.
    pub sprites: Vec<Sprite>,
    pub lights: Vec<Light>,
    pub overlays: Vec<Overlay>,
    /// Its overlays are the lamp's on a turning part's plastic and still cap (a knob's, a
    /// selector's): left off for a look whose caps turn, whose pictures carry their own light.
    pub round: bool,
    pub drawn: Vec<crate::ultra::Drawn>,
    pub over: String,
    /// A wheel the worn skin's renderer draws, and a rocker's paddle it lights by its shape
    /// (none in the drawn skin).
    pub wheel: Option<WheelArt>,
    pub paddle: Option<PaddleArt>,
    /// A dot-matrix screen the renderer draws over the body (the standalone's TEMPO).
    pub screen: Option<Screen>,
}

/// A dot-matrix screen in a layer: its window (left, top, width, height, from the layer's
/// origin) in the rail's screen's bezel and glass, `text` lit in it, centred
/// (`strip::pixel_screen`).
#[derive(Clone, Debug, PartialEq)]
pub struct Screen {
    pub window: (f64, f64, f64, f64),
    pub text: String,
}

/// A picture of a part drawn in a layer: `size` (drawing units) about `at` (from the layer's
/// origin), turned `deg` clockwise after it is mirrored as `flip` (across, down) says, drawn
/// `zoom` times its size (its picture made at `size`, so that a part growing does not make a
/// picture a frame) and `alpha` opaque.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sprite {
    pub part: Part,
    pub size: (f64, f64),
    pub at: (f64, f64),
    pub deg: f64,
    pub flip: (bool, bool),
    pub zoom: f64,
    pub alpha: f64,
}

/// A light drawn in a layer over its pictures: a picture of it made once a scale and kept
/// (`crate::ultra::Glow`, a lamp's), drawn `zoom` times its size about the layer's origin and
/// `alpha` opaque, its strength, rather than drawn afresh as it changes (decisions.md
/// R58).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    pub glow: crate::ultra::Glow,
    pub zoom: f64,
    pub alpha: f64,
}

/// SVG over a layer's pictures that does not change as the layer does: drawn once in the
/// layer's place and kept, then put over the pictures with `blend` (as SVG's
/// `mix-blend-mode`), rather than drawn afresh each time the layer is (decisions.md R59).
#[derive(Clone, Debug, PartialEq)]
pub struct Overlay {
    pub svg: String,
    pub blend: Blend,
}

/// How an [`Overlay`] goes over what is under it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Blend {
    Normal,
    ColorDodge,
    Multiply,
}

/// An oscillator's TIMED key's cap (decisions.md R-OSC-TIMED): the strip's amber tab at 0.4 of
/// its size (the mock-ups' composite, approved), and where its note value's sign begins, right
/// of its centre (the mock-ups' eighth note's place).
pub const KEY: (f64, f64) = (60.0, 24.0);
pub const KEY_SIGN: f64 = 39.0;
/// How far above and below the key's middle its sign may reach: half a thirty-second note's
/// height in the large lettering, a triplet's 3 above it, and a little over.
pub const KEY_SIGN_TOP: f64 = 26.0;

/// A control's extent about its centre (left, top, right, bottom), its shadow included.
pub fn bounds(kind: &Kind) -> [f64; 4] {
    match *kind {
        // (The cap and its light round it, and the sign printed to its right: a note centred on
        // it, its stem and flags at their natural length above and below; or a MOD key's word.)
        Kind::Key => [
            -KEY.0 / 2.0 - 8.0,
            -KEY_SIGN_TOP,
            KEY_SIGN + 56.0,
            KEY_SIGN_TOP,
        ],
        Kind::Knob { big, .. } => {
            let r = if big { BIG.skirt } else { STD.skirt };
            [-(r + 3.0), -(r + 3.0), r + 6.0, r + 8.0]
        }
        Kind::Selector(_) => [-62.0, -62.0, 63.0, 64.0],
        Kind::Rocker { w, h, .. } | Kind::Toggle { w, h, .. } => {
            [-w / 2.0 - 4.0, -h / 2.0 - 4.0, w / 2.0 + 4.0, h / 2.0 + 4.0]
        }
        Kind::Wheel { .. } => {
            let (hx, hy) = (
                SLOT_W / 2.0 * WHEEL_SCALE + 2.0,
                SLOT_H / 2.0 * WHEEL_SCALE + 2.0,
            );
            [-hx, -hy, hx, hy]
        }
        Kind::Jack => {
            let r = crate::strip::lfo::JACK_R + 4.0;
            [-r, -r, r, r]
        }
    }
}

/// The box a control's printed dial (its ticks, numbers and captions) or legends take about its
/// centre (left, top, right, bottom), in the large lettering (the larger): what a layout must
/// leave room for at the panel's edges. A switch's and a toggle's legends are the layout's own.
pub fn print_extent(kind: &Kind) -> [f64; 4] {
    let num = Lettering::Large.number();
    match *kind {
        // (An oscillator's FREQUENCY prints note values while it is timed: either dial's.)
        Kind::Knob {
            dial: Dial::Freq, ..
        } => {
            let (one, two) = (dial_extent(Dial::Freq, num), dial_extent(Dial::Timed, num));
            [
                one[0].min(two[0]),
                one[1].min(two[1]),
                one[2].max(two[2]),
                one[3].max(two[3]),
            ]
        }
        Kind::Knob { dial, .. } => dial_extent(dial, num),
        Kind::Selector(_) => {
            let r = SIX_LEGEND + 20.0;
            let x = r * rad(SIX[5]).sin().abs() + 20.0;
            [-x, -(r + num), x, 64.0]
        }
        // A TIMED key's sign, beside it (R-OSC-TIMED).
        Kind::Key => bounds(kind),
        _ => [0.0; 4],
    }
}

/// A control's print, including GLIDE's optional LONG sign, which a layout leaves room for.
pub fn control_print_extent(c: &Control) -> [f64; 4] {
    let e = print_extent(&c.kind);
    if c.param != "glide" {
        return e;
    }
    let g = GLIDE_LONG_EXTENT;
    [
        e[0].min(g[0]),
        e[1].min(g[1]),
        e[2].max(g[2]),
        e[3].max(g[3]),
    ]
}

/// The box a dial's print takes about its centre, its numbers `num` in size.
fn dial_extent(dial: Dial, num: f64) -> [f64; 4] {
    // A text's half width at `size` (a capital about 0.62 of it, stretched as legends are).
    let half = |t: &str, size: f64| t.chars().count() as f64 * size * 0.62 * STRETCH / 2.0;
    let d = marks(dial);
    // The numbers round the dial: each where it is printed, its box about it.
    let mut b = [0.0f64; 4];
    let mut grow = |x: f64, y: f64, hw: f64, hh: f64| {
        b = [
            b[0].min(x - hw),
            b[1].min(y - hh),
            b[2].max(x + hw),
            b[3].max(y + hh),
        ];
    };
    for (_, r0, r1, _) in &d.ticks {
        grow(0.0, 0.0, r0.max(*r1), r0.max(*r1));
    }
    for &(a, t, at) in &d.labels {
        let (x, y) = label_at(&d, a, t, at, num);
        grow(x, y, half(t, num) + 2.0, num * 0.6);
    }
    for (x, y, t) in &d.captions {
        grow(*x, *y, half(t, num - 1.0) + 2.0, num * 0.6);
    }
    b
}

/// Where a wheel is drawn: its parameter `v`, moved on by a MIDI keyboard's wheel `midi`
/// (the bend adds, 0.5 centred; the modulation wheel goes as far as the greater).
pub fn wheel_shown(detent: bool, v: f64, midi: f64) -> f64 {
    if detent {
        (v + midi - 0.5).clamp(0.0, 1.0)
    } else {
        v.max(midi)
    }
}

/// The selector `c` with its pointer at `angle` (degrees from twelve o'clock), in the worn skin
/// or the drawn: at one of its six places ([`SIX`]), or anywhere CONTINUOUS turns it along its
/// travel (decisions.md R-CONTINUOUS-WAVE).
pub fn selector_turned(c: &Control, angle: f64, worn: bool) -> Layer {
    use crate::skin::{POINTER_BODY, POINTER_CAP};
    let (origin, bounds) = (c.centre(), bounds(&c.kind));
    if worn {
        let d = 2.0 * SELECTOR_R / POINTER_BODY;
        return Layer {
            origin,
            bounds,
            sprites: vec![
                sprite(Part::Pointer, (d, d), angle, (false, false)),
                sprite(Part::PointerCap, (d, d), CAP_TURN, (false, false)),
            ],
            overlays: round_lights(SELECTOR_R, d / 2.0 * POINTER_CAP),
            round: true,
            ..Layer::default()
        };
    }
    let mut s = Svg::default();
    selector_body(&mut s, angle);
    Layer {
        origin,
        bounds,
        body: s.0,
        ..Layer::default()
    }
}

/// CONTINUOUS's travel past SQUARE in the switch's steps (decisions.md R-CONTINUOUS-WAVE): to
/// WIDE and NARROW one each, then on to the floor at NARROW's volts a step (the engine's
/// `voice::RECT_STEPS`, which the plug-in's tests hold this to); and its whole travel, from
/// TRIANGLE.
pub const RECT_STEPS: f64 = 2.95;
pub const SHAPE_PLACES: f64 = 3.0 + RECT_STEPS;

/// The angle of a WAVEFORM switch `at` its steps from TRIANGLE (CONTINUOUS's travel).
pub fn shape_angle(at: f64) -> f64 {
    SIX[0] + at.clamp(0.0, SHAPE_PLACES) * (SIX[1] - SIX[0])
}

/// Which way MOD moves a CONTINUOUS shape, and its colour (the owner, 2026-10-20: "the same red
/// that we have on some of the buttons and the same blue ... amber, orange, or red, I guess, and
/// blue"): clockwise amber, anticlockwise blue, both ways red.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ModWay {
    #[default]
    Clockwise,
    Anticlockwise,
    Both,
}

impl ModWay {
    /// Its light's colour round its key and its orb's.
    pub fn light(self) -> &'static str {
        match self {
            ModWay::Clockwise => KEY_LIGHT,
            ModWay::Anticlockwise => "#3f8fe8",
            ModWay::Both => "#ff4422",
        }
    }

    /// Its orb's hot middle.
    fn core(self) -> &'static str {
        match self {
            ModWay::Clockwise => "#ffd9a0",
            ModWay::Anticlockwise => "#cfe6ff",
            ModWay::Both => "#ffc0a8",
        }
    }
}

/// MOD's orb (the owner, 2026-10-20: "a little glowing amber orb that showed the position of
/// where the effective waveform sits due to the modulation ... the knob stays put"): a small
/// glowing light on the switch's ring at `at` (its steps), in its way's colour.
pub fn shape_orb(c: &Control, at: f64, way: ModWay) -> Layer {
    let r = (SIX_TICK.0 + SIX_TICK.1) / 2.0;
    let (x, y) = polar(r, shape_angle(at));
    let mut s = Svg::default();
    put!(
        s,
        "<defs><radialGradient id='orb' fx='0.42' fy='0.4'><stop offset='0' stop-color='{}'/><stop offset='0.55' stop-color='{}'/><stop offset='1' stop-color='{}' stop-opacity='0.6'/></radialGradient><filter id='orb-glow' x='-2' y='-2' width='5' height='5'><feGaussianBlur stdDeviation='{}'/></filter></defs><circle cx='{}' cy='{}' r='{}' fill='{}' fill-opacity='0.5' filter='url(#orb-glow)'/><circle cx='{}' cy='{}' r='{}' fill='url(#orb)'/>",
        way.core(),
        way.light(),
        way.light(),
        N(ORB_R * 0.9),
        N(x),
        N(y),
        N(ORB_R * 1.5),
        way.light(),
        N(x),
        N(y),
        N(ORB_R)
    );
    Layer {
        origin: c.centre(),
        bounds: [
            x - 4.0 * ORB_R,
            y - 4.0 * ORB_R,
            x + 4.0 * ORB_R,
            y + 4.0 * ORB_R,
        ],
        body: s.0,
        ..Layer::default()
    }
}

/// The orb's radius, units.
const ORB_R: f64 = 5.0;

/// A control at its normalized value `v` (a wheel moved on by `midi`).
pub fn control(c: &Control, v: f64, midi: f64) -> Layer {
    let mut s = Svg::default();
    let v = v.clamp(0.0, 1.0);
    match c.kind {
        Kind::Knob { big, .. } => knob_body(&mut s, if big { BIG } else { STD }, knob_angle(v)),
        Kind::Selector(_) => selector_body(&mut s, SIX[position(v)]),
        Kind::Toggle { across, .. } => {
            let at = crate::interact::toggle_position(v);
            if across {
                // On its side: the drawing mirrored about its diagonal, up to the left.
                s.0.push_str("<g transform='matrix(0 1 1 0 0 0)'>");
                toggle_body(&mut s, 2 - at);
                s.0.push_str("</g>");
            } else {
                toggle_body(&mut s, at);
            }
        }
        Kind::Rocker {
            w,
            h,
            colour,
            orient,
        } => {
            let (turn, pw, ph) = match orient {
                Orient::Right => ("", w, h),
                Orient::Top => ("rotate(-90)", h, w),
                Orient::Bottom => ("rotate(90)", h, w),
            };
            put!(s, "<g transform='{turn}'>");
            paddle(&mut s, pw, ph, colour, v >= 0.5);
            s.0.push_str("</g>");
        }
        Kind::Wheel { mark, detent, span } => {
            wheel(&mut s, mark, span, wheel_shown(detent, v, midi))
        }
        // (The LFO page draws its jack: R-LFO's addendum.)
        Kind::Jack => {}
        // (Its sign is its division's, which the renderer knows: `key`.)
        Kind::Key => {
            return key(
                c,
                Keyed {
                    on: v >= 0.5,
                    ..Keyed::default()
                },
                Lettering::Standard,
                "×",
                false,
            );
        }
    }
    Layer {
        origin: c.centre(),
        bounds: bounds(&c.kind),
        body: s.0,
        ..Layer::default()
    }
}

/// A selector's position (0..5) at its normalized value.
pub fn position(v: f64) -> usize {
    (v.clamp(0.0, 1.0) * 5.0).round() as usize
}

/// The POWER switch's extent about its centre.
pub const POWER_BOUNDS: [f64; 4] = [-26.0, -64.0, 26.0, 64.0];

/// The POWER switch, on (playing) or off (bypassed).
pub fn power(on: bool) -> Layer {
    let mut s = Svg::default();
    s.0.push_str("<g transform='rotate(-90)'>");
    paddle(&mut s, 119.0, 43.0, Colour::Black, on);
    s.0.push_str("</g>");
    Layer {
        origin: (COL + POWER.0, TOP + POWER.1),
        bounds: POWER_BOUNDS,
        body: s.0,
        ..Layer::default()
    }
}

/// The POWER lamp, lit while the plugin plays.
pub fn lamp(on: bool) -> Layer {
    let mut s = Svg::default();
    shadow(&mut s, 3.0, 4.0, 19.0);
    put!(
        s,
        "<circle r='18' fill='url(#cap)' stroke='{CAP_DARK}' stroke-width='1.5'/>"
    );
    put!(
        s,
        "<circle r='13' fill='{}'/>",
        if on { LAMP } else { LAMP_OFF }
    );
    put!(s, "<circle cx='-4' cy='-4' r='3.2' fill='{CAP_LIGHT}'/>");
    Layer {
        origin: (COL + LAMP_AT.0, TOP + LAMP_AT.1),
        bounds: [-20.0, -20.0, 23.0, 24.0],
        body: s.0,
        ..Layer::default()
    }
}

/// The OVERLOAD lamp at `level` (0 dark, 1 fully lit): a plain dark jewel, smooth, domed,
/// with a small highlight; lit, it glows red through.
pub fn overload(level: f64) -> Layer {
    let level = (level.clamp(0.0, 1.0) * 100.0).round() / 100.0;
    let mut s = Svg::default();
    shadow(&mut s, 3.0, 4.0, 23.0);
    put!(
        s,
        "<circle r='22' fill='url(#cap)' stroke='{CAP_DARK}' stroke-width='1.5'/>"
    );
    put!(s, "<circle r='15' fill='url(#jewel)'/>");
    if level >= 0.05 {
        put!(
            s,
            "<filter id='lit' x='-1' y='-1' width='3' height='3'><feDropShadow dx='0' dy='0' stdDeviation='3' flood-color='{LAMP}'/></filter>"
        );
        put!(
            s,
            "<circle r='15' fill='{LAMP}' fill-opacity='{}' filter='url(#lit)'/>",
            N(level)
        );
    }
    put!(
        s,
        "<circle cx='-5' cy='-5.5' r='2.8' fill='{CAP_LIGHT}' fill-opacity='0.75'/>"
    );
    Layer {
        origin: (COL + OVERLOAD_AT.0, TOP + OVERLOAD_AT.1),
        bounds: [-34.0, -34.0, 34.0, 34.0],
        body: s.0,
        ..Layer::default()
    }
}

/// The name plate: the instrument's name, its maker's under it.
pub fn plate() -> Layer {
    let (w, h, r) = (PLATE_W, PLATE_H, 3.0);
    let mut s = Svg::default();
    put!(
        s,
        "<path d='M {r} 0 H {} A {r} {r} 0 0 1 {} {r} V {} A {r} {r} 0 0 1 {} {} H {r} A {r} {r} 0 0 1 0 {} V {r} A {r} {r} 0 0 1 {r} 0 Z' fill='{KNOB}'/>",
        N(w - r),
        N(w),
        N(h - r),
        N(w - r),
        N(h),
        N(h - r)
    );
    text(
        &mut s,
        PLATE_PAD,
        NAME_Y,
        PLATE_NAME,
        38.0,
        Style::Plate,
        true,
        "",
    );
    text(
        &mut s,
        PLATE_PAD + 2.0,
        MAKER_Y,
        PLATE_MAKER,
        12.0,
        Style::Plate,
        true,
        "letter-spacing='4.2'",
    );
    Layer {
        origin: (PLATE_X, PLATE_Y),
        bounds: [-24.0, -10.0, w + 24.0, h + 10.0],
        body: s.0,
        ..Layer::default()
    }
}

// ---- The worn skin (decisions.md R44): the same parts in the same places, their surfaces
// pictures (`crate::skin`), the lamp's light laid over them where it does not turn.

/// The name plate in the worn skin (the owner, 2026-10-10: "it's not up to the same realistic
/// quality as everything else"): a picture of a blank plate, standing a little proud of the
/// wood (its shadow soft, down and to the right), its lettering printed on it as the panel's is:
/// paint filled into engraving, the cut's upper left wall in shade over it, its lower right
/// catching the lamp a little.
pub fn plate_worn() -> Layer {
    let (w, h) = (PLATE_W, PLATE_H);
    let mut body = Svg::default();
    put!(
        body,
        "<defs><filter id='plate-shadow' x='-0.2' y='-0.5' width='1.4' height='2'><feGaussianBlur stdDeviation='4'/></filter></defs><rect x='3' y='5' width='{}' height='{}' rx='4' fill='#000' fill-opacity='0.6' filter='url(#plate-shadow)'/><rect x='1' y='1.5' width='{}' height='{}' rx='3' fill='#000' fill-opacity='0.5'/>",
        N(w),
        N(h),
        N(w),
        N(h)
    );
    let mut over = Svg::default();
    let line = |s: &mut Svg, x: f64, y: f64, t: &str, size: f64, extra: &str| {
        for (dx, dy, fill, opacity) in [
            (0.6, 0.7, "#ffffff", 0.1),
            (-0.5, -0.6, "#000000", 0.75),
            (0.0, 0.0, PLATE_PAINT, 0.95),
        ] {
            put!(
                s,
                "<text x='{}' y='{}' text-anchor='start' font-size='{}' font-weight='700' fill='{fill}' fill-opacity='{opacity}' {extra}>{}</text>",
                N(x + dx),
                N(y + size * 0.36 + dy),
                N(size),
                escape(t)
            );
        }
    };
    line(&mut over, PLATE_PAD, NAME_Y, PLATE_NAME, 38.0, "");
    line(
        &mut over,
        PLATE_PAD + 2.0,
        MAKER_Y,
        PLATE_MAKER,
        12.0,
        "letter-spacing='4.2'",
    );
    Layer {
        origin: (PLATE_X, PLATE_Y),
        bounds: [-24.0, -10.0, w + 24.0, h + 14.0],
        body: body.0,
        sprites: vec![Sprite {
            at: (w / 2.0, h / 2.0),
            ..sprite(Part::Plate, (w, h), 0.0, (false, false))
        }],
        over: over.0,
        ..Layer::default()
    }
}

/// The plate's lettering's paint: an off white, as the panel's print.
const PLATE_PAINT: &str = "#e4ded0";

/// The standalone's TEMPO screen (decisions.md R-ARP; the owner's option D, 2026-10-19): at the
/// top strip's left end, mirroring the name plate at its right, its surround as wide and as tall
/// as the plate and as far from the drawing's left edge as the plate is from its right; the
/// window inside the surround, as the rail's screen's. Only the standalone's: in a plug-in the
/// host gives the tempo, and the wood is bare. The official panel's; a skin pack's layout that
/// moves the plate has it mirrored with it ([`tempo_window`], decisions.md R-SKIN-PACKS).
pub const TEMPO_SCREEN: (f64, f64, f64, f64) = tempo_window((PLATE_X, PLATE_Y));

/// The TEMPO screen's window (left, top, width, height, the drawing's units) on a panel whose
/// name plate's top left corner is at `plate` (`Arrangement::plate_at`): the plate mirrored
/// across the drawing, less its surround.
pub const fn tempo_window(plate: (f64, f64)) -> (f64, f64, f64, f64) {
    (
        W - plate.0 - PLATE_W + crate::strip::NAME_SURROUND,
        plate.1 + crate::strip::NAME_SURROUND,
        PLATE_W - 2.0 * crate::strip::NAME_SURROUND,
        PLATE_H - 2.0 * crate::strip::NAME_SURROUND,
    )
}

/// How wide the TEMPO screen's surround is round its window (the name plate's).
pub const TEMPO_SURROUND: f64 = crate::strip::NAME_SURROUND;

/// Whether (`x`, `y`) is on the TEMPO screen, its surround included, the panel laid out as
/// `arrangement` has it.
pub fn tempo_hit(arrangement: &Arrangement, x: f64, y: f64) -> bool {
    let (sx, sy, w, h) = tempo_window(arrangement.plate_at());
    let o = crate::strip::NAME_SURROUND;
    (sx - o..sx + w + o).contains(&x) && (sy - o..sy + h + o).contains(&y)
}

/// The TEMPO screen reading `text` (`120 BPM`), the panel laid out as `arrangement` has it.
pub fn tempo_screen(text: &str, arrangement: &Arrangement) -> Layer {
    let (x, y, w, h) = tempo_window(arrangement.plate_at());
    Layer {
        origin: (x, y),
        bounds: [-16.0, -14.0, w + 16.0, h + 18.0],
        screen: Some(Screen {
            window: (0.0, 0.0, w, h),
            text: text.to_owned(),
        }),
        ..Layer::default()
    }
}

/// The standalone's TYPING and PANIC keys on the walnut after the TEMPO screen (decisions.md
/// R-TEMPO-CORNER; the owner, 2026-10-26: "typing should be a computer keyboard icon", then "Go
/// with 62 units (as tall as the rail keys)"): moved up from the KEYS page so that they work from
/// every page. MIDI, the MIDI map mode's, between them (R-MIDI-MAP; the owner, 2026-10-27:
/// "keyboard-midi-panic"). The standalone's alone, as the screen is; a plug-in's wood stays bare.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TempoKey {
    Typing,
    Midi,
    Panic,
}

impl TempoKey {
    /// In their order after the screen.
    pub const ALL: [TempoKey; 3] = [TempoKey::Typing, TempoKey::Midi, TempoKey::Panic];
    /// The last, after which a tip beside the keys goes.
    pub const LAST: TempoKey = TempoKey::Panic;
}

/// A TEMPO key's size: the rail's keys'.
pub const TEMPO_KEY: (f64, f64) = (crate::strip::KEY_W, crate::strip::KEY_H);
/// The gap after the screen's surround and between the keys: the rail's between its keys.
const TEMPO_KEY_GAP: f64 = 14.0;

/// Key `k`'s middle, the panel laid out as `arrangement` has it: centred on the screen's
/// height, a gap after its surround, the next a gap after it.
pub fn tempo_key_at(arrangement: &Arrangement, k: TempoKey) -> (f64, f64) {
    let (x, y, w, h) = tempo_window(arrangement.plate_at());
    let first = x + w + TEMPO_SURROUND + TEMPO_KEY_GAP + TEMPO_KEY.0 / 2.0;
    let i = TempoKey::ALL.iter().position(|&t| t == k).unwrap_or(0) as f64;
    (first + i * (TEMPO_KEY.0 + TEMPO_KEY_GAP), y + h / 2.0)
}

/// The TEMPO key at (`x`, `y`), as far round it as the rail's keys are hit round theirs.
pub fn tempo_key_hit(arrangement: &Arrangement, x: f64, y: f64) -> Option<TempoKey> {
    TempoKey::ALL.into_iter().find(|&k| {
        let (cx, cy) = tempo_key_at(arrangement, k);
        (x - cx).abs() <= TEMPO_KEY.0 / 2.0 + 4.0 && (y - cy).abs() <= TEMPO_KEY.1 / 2.0 + 6.0
    })
}

/// A TEMPO key as the rail's are drawn: their cap, its mark in `print` (a computer keyboard for
/// TYPING, the word for PANIC and MIDI), the rail's light over it while `lit` (TYPING on, PANIC
/// held, the MIDI map mode on) and lightened a little under the pointer.
pub fn tempo_key(
    arrangement: &Arrangement,
    k: TempoKey,
    lit: bool,
    hover: bool,
    print: &str,
) -> Layer {
    let (w, h) = TEMPO_KEY;
    let mut over = Svg::default();
    match k {
        TempoKey::Typing => {
            // A computer keyboard: its case, two rows of six keys and the space bar.
            let (bw, bh) = (46.0, 28.0);
            put!(
                over,
                "<rect x='{}' y='{}' width='{}' height='{}' rx='4' fill='none' stroke='{print}' stroke-width='2.6'/>",
                N(-bw / 2.0),
                N(-bh / 2.0),
                N(bw),
                N(bh)
            );
            for row in 0..2 {
                for col in 0..6 {
                    put!(
                        over,
                        "<rect x='{}' y='{}' width='3.4' height='3.4' fill='{print}'/>",
                        N(-bw / 2.0 + 6.5 + f64::from(col) * 6.0),
                        N(-bh / 2.0 + 6.5 + f64::from(row) * 6.0)
                    );
                }
            }
            put!(
                over,
                "<rect x='{}' y='{}' width='22' height='3.4' fill='{print}'/>",
                N(-11.0),
                N(-bh / 2.0 + 19.5)
            );
        }
        TempoKey::Panic => put!(
            over,
            "<text x='0' y='0' font-size='20' font-weight='700' text-anchor='middle' dominant-baseline='central' fill='{print}'>PANIC</text>"
        ),
        TempoKey::Midi => put!(
            over,
            "<text x='0' y='0' font-size='20' font-weight='700' text-anchor='middle' dominant-baseline='central' fill='{print}'>MIDI</text>"
        ),
    }
    if lit {
        put!(
            over,
            "<rect x='{}' y='{}' width='{}' height='{}' rx='7' fill='#b0702b' opacity='0.28'/>",
            N(-w / 2.0),
            N(-h / 2.0),
            N(w),
            N(h)
        );
    }
    if hover {
        put!(
            over,
            "<rect x='{}' y='{}' width='{}' height='{}' rx='8' fill='#fff' fill-opacity='0.07'/>",
            N(-w / 2.0 + 2.0),
            N(-h / 2.0 + 2.0),
            N(w - 4.0),
            N(h - 4.0)
        );
    }
    Layer {
        origin: tempo_key_at(arrangement, k),
        bounds: [
            -w / 2.0 - 6.0,
            -h / 2.0 - 6.0,
            w / 2.0 + 8.0,
            h / 2.0 + 10.0,
        ],
        sprites: vec![sprite(Part::RailKey, (w, h), 0.0, (false, false))],
        over: over.0,
        ..Layer::default()
    }
}

/// A selector's knob's body: as wide as the drawn one's fluted outline, less the pointer's
/// reach (the picture's wedge reaches its edge, 64 units out, as the drawn fin about does).
pub(crate) const SELECTOR_R: f64 = 50.0;

/// How the panel's lamp lights a round part's near side, up and to the left (a colour dodge to
/// 1 / (1 - `LAMP_NEAR`) of its light at the edge), and darkens its far side (multiplied by
/// `LAMP_FAR` at the edge), each back to nothing by the middle: the CA-74's.
const LAMP_NEAR: f64 = 0.45;
const LAMP_FAR: f64 = 0.35;

/// How far a cap's picture is turned, clockwise, so that its spun sheen (generated lying across
/// it, along three and nine o'clock) lies along the line to the lamp, up and to the left, as a
/// spun disc's does.
const CAP_TURN: f64 = 45.0;

/// The lamp's light on a spun cap `r` across, over its sheen: its near side a touch lighter.
fn cap_light(r: f64) -> Overlay {
    Overlay {
        svg: format!(
            "<defs><linearGradient id='cap' x1='0' y1='0' x2='1' y2='1'><stop offset='0.15' stop-color='#fff' stop-opacity='0.12'/><stop offset='0.5' stop-color='#fff' stop-opacity='0'/><stop offset='0.55' stop-color='#000' stop-opacity='0'/><stop offset='0.85' stop-color='#000' stop-opacity='0.1'/></linearGradient></defs><circle r='{}' fill='url(#cap)'/>",
            N(r)
        ),
        blend: Blend::Normal,
    }
}

/// The panel's lamp over a round part's black plastic, from `inner` (its spun cap's rim, whose
/// sheen is its own) out to `r`: its near side dodged lighter, its far side multiplied darker.
/// It does not turn with the part.
fn lamp_over(r: f64, inner: f64) -> [Overlay; 2] {
    let grey = |v: f64| {
        let g = (v * 255.0).round() as u8;
        format!("#{g:02x}{g:02x}{g:02x}")
    };
    let ring = format!(
        "M {o} 0 A {o} {o} 0 1 0 -{o} 0 A {o} {o} 0 1 0 {o} 0 Z M {i} 0 A {i} {i} 0 1 0 -{i} 0 A {i} {i} 0 1 0 {i} 0 Z",
        o = N(r - 1.0),
        i = N(inner)
    );
    [
        Overlay {
            svg: format!(
                "<defs><linearGradient id='near' x1='0' y1='0' x2='1' y2='1'><stop offset='0.15' stop-color='{n}'/><stop offset='0.5' stop-color='{n}' stop-opacity='0'/></linearGradient></defs><path d='{ring}' fill-rule='evenodd' fill='url(#near)'/>",
                n = grey(LAMP_NEAR)
            ),
            blend: Blend::ColorDodge,
        },
        Overlay {
            svg: format!(
                "<defs><linearGradient id='far' x1='0' y1='0' x2='1' y2='1'><stop offset='0.5' stop-color='{f}' stop-opacity='0'/><stop offset='0.85' stop-color='{f}'/></linearGradient></defs><path d='{ring}' fill-rule='evenodd' fill='url(#far)'/>",
                f = grey(LAMP_FAR)
            ),
            blend: Blend::Multiply,
        },
    ]
}

/// A round part's lights that do not turn with it: the lamp's on its plastic, then its cap's.
fn round_lights(r: f64, cap: f64) -> Vec<Overlay> {
    let mut lights = lamp_over(r, cap).to_vec();
    lights.push(cap_light(cap));
    lights
}

/// A sprite at a layer's origin.
fn sprite(part: Part, size: (f64, f64), deg: f64, flip: (bool, bool)) -> Sprite {
    Sprite {
        part,
        size,
        at: (0.0, 0.0),
        deg,
        flip,
        zoom: 1.0,
        alpha: 1.0,
    }
}

/// A control in the worn skin at its normalized value: a knob's or a selector's picture turned
/// as the drawn one is (the light on its spun cap and the lamp's over it not turned), a
/// rocker's picture pressed at its end; the wheels as drawn.
pub fn control_worn(c: &Control, v: f64, midi: f64) -> Layer {
    use crate::skin::{BIG_CAP, BIG_SKIRT, KNOB_CAP, KNOB_SKIRT};
    let v = v.clamp(0.0, 1.0);
    let (origin, bounds) = (c.centre(), bounds(&c.kind));
    match c.kind {
        Kind::Knob { big, .. } => {
            let (size, skirt, part, cap, cap_share) = if big {
                (BIG, BIG_SKIRT, Part::KnobBig, Part::KnobBigCap, BIG_CAP)
            } else {
                (STD, KNOB_SKIRT, Part::Knob, Part::KnobCap, KNOB_CAP)
            };
            let d = 2.0 * size.skirt / skirt;
            Layer {
                origin,
                bounds,
                sprites: vec![
                    sprite(part, (d, d), knob_angle(v), (false, false)),
                    sprite(cap, (d, d), CAP_TURN, (false, false)),
                ],
                overlays: round_lights(size.skirt, d / 2.0 * cap_share),
                round: true,
                ..Layer::default()
            }
        }
        Kind::Selector(_) => selector_turned(c, SIX[position(v)], true),
        // QUALITY's chrome toggle: its picture for the lever's place. NOTE PRIORITY's on its
        // side: the picture mirrored about its diagonal (flipped, then turned a quarter), so
        // that its light still comes from the upper left; its lever up to the left.
        Kind::Toggle { w, h, across } => {
            let at = crate::interact::toggle_position(v);
            let part = match if across { 2 - at } else { at } {
                2 => Part::ToggleUp,
                1 => Part::ToggleMid,
                _ => Part::ToggleDown,
            };
            let sprite = if across {
                sprite(part, (h, w), 90.0, (false, true))
            } else {
                sprite(part, (w, h), 0.0, (false, false))
            };
            Layer {
                origin,
                bounds,
                sprites: vec![sprite],
                ..Layer::default()
            }
        }
        Kind::Rocker {
            w,
            h,
            colour,
            orient,
        } => {
            let (deg, pw, ph) = match orient {
                Orient::Right => (0.0, w, h),
                Orient::Top => (-90.0, h, w),
                Orient::Bottom => (90.0, h, w),
            };
            let on = v >= 0.5;
            if colour == Colour::Black {
                // POWER's black ribbed rocker (QUALITY), upright as POWER's: its picture's
                // upper half raised, its lower when its upper end is pressed.
                let top = match orient {
                    Orient::Bottom => !on,
                    _ => on,
                };
                return Layer {
                    origin,
                    bounds,
                    // (Its picture as much larger than its paddle as POWER's, 46 by 124 for 43 by 119.)
                    sprites: vec![sprite(
                        Part::Power,
                        (w * 46.0 / 43.0, h * 124.0 / 119.0),
                        0.0,
                        (false, top),
                    )],
                    ..Layer::default()
                };
            }
            let part = match colour {
                Colour::Blue => Part::RockerBlue,
                Colour::Red => Part::RockerOrange,
                Colour::Ivory | Colour::Black => Part::RockerIvory,
            };
            // The picture's right half is raised; on, its left (the drawing's "on" end at its
            // right pressed).
            let m = 16.0;
            Layer {
                origin,
                bounds: [bounds[0] - m, bounds[1] - m, bounds[2] + m, bounds[3] + m],
                body: paddle_shadow(pw, ph, deg, on),
                sprites: vec![sprite(part, (pw + 4.0, ph + 4.0), deg, (on, false))],
                paddle: Some(PaddleArt { pw, ph, deg, on }),
                ..Layer::default()
            }
        }
        Kind::Wheel { mark, detent, span } => {
            let turn = (wheel_shown(detent, v, midi) - 0.5) * span;
            Layer {
                origin,
                bounds,
                body: wheel_slot(),
                wheel: Some(WheelArt { mark, turn }),
                ..Layer::default()
            }
        }
        Kind::Key => key(
            c,
            Keyed {
                on: v >= 0.5,
                ..Keyed::default()
            },
            Lettering::Standard,
            "×",
            true,
        ),
        // (The LFO page draws its jack: R-LFO's addendum.)
        Kind::Jack => Layer {
            origin,
            bounds,
            ..Layer::default()
        },
    }
}

/// How an oscillator's TIMED key is shown (decisions.md R-OSC-TIMED): on or off, inert (dimmed,
/// unlit whatever it holds, as VOICES is where it does nothing), and its oscillator's division,
/// by its place among the eighteen (`params::Division`'s order); `hidden` away from LO.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keyed {
    pub on: bool,
    pub inert: bool,
    pub division: usize,
    /// A word printed beside it instead of a note value: a MOD key's (decisions.md
    /// R-CONTINUOUS-WAVE).
    pub word: Option<&'static str>,
    /// Not on the panel at all: a MOD key while its oscillator's waveform is STEPPED.
    pub hidden: bool,
    /// Its colour: a MOD key's its way's (decisions.md R-CONTINUOUS-WAVE); a TIMED key amber.
    pub way: ModWay,
}

impl Default for Keyed {
    fn default() -> Self {
        // (A quarter note: the division a new patch has.)
        Self {
            on: false,
            inert: false,
            division: 8,
            word: None,
            hidden: false,
            way: ModWay::Clockwise,
        }
    }
}

/// Whether a key comes and goes from the panel, rather than always standing on it: its cap's
/// shadow is then its own, not the background's. Every key does: a TIMED key is on the panel
/// only at its RANGE's LO (decisions.md R-OSC-TIMED), as a MOD key only while its oscillator's
/// waveform is CONTINUOUS (R-CONTINUOUS-WAVE).
pub fn comes_and_goes(c: &Control) -> bool {
    c.kind == Kind::Key
}

/// An oscillator's TIMED key (decisions.md R-OSC-TIMED), in either skin the strip's amber tab
/// (its pictures, a skin pack's where it has them), lit from inside while on, and the note
/// value its oscillator is timed at printed beside it in the panel's ink (the owner,
/// 2026-10-20: "it dynamically shows whatever is selected on the oscillator"); dimmed where it
/// does nothing, its sign with it; nothing at all where it is hidden (away from LO).
/// (`times`: the multiplication sign as the typeface prints it, [`Layout::times`].)
pub fn key(c: &Control, k: Keyed, lettering: Lettering, times: &str, worn: bool) -> Layer {
    if k.hidden {
        return Layer {
            origin: c.centre(),
            bounds: bounds(&Kind::Key),
            ..Layer::default()
        };
    }
    let lit = k.on && !k.inert;
    let (w, h) = KEY;
    let mut s = Svg::default();
    if comes_and_goes(c) {
        // Its cap's shadow, which the background has for a key always on the panel.
        put!(
            s,
            "<defs><filter id='key-shadow' x='-0.5' y='-1' width='2' height='3'><feGaussianBlur stdDeviation='1.6'/></filter></defs><rect x='{}' y='{}' width='{}' height='{}' rx='4' fill='#000' fill-opacity='0.35' filter='url(#key-shadow)'/>",
            N(-w / 2.0 + 1.0),
            N(-h / 2.0 + 2.0),
            N(w + 2.0),
            N(h + 2.0)
        );
    }
    if lit {
        // Its light on the face round it: a soft amber, faint (the owner is sensitive to
        // brightness; the strip's tabs' light, at this size).
        put!(
            s,
            "<defs><filter id='key-light' x='-0.5' y='-1' width='2' height='3'><feGaussianBlur stdDeviation='4.5'/></filter></defs><rect x='{}' y='{}' width='{}' height='{}' rx='5' fill='{}' fill-opacity='0.34' filter='url(#key-light)'/>",
            N(-w / 2.0 - 1.0),
            N(-h / 2.0 - 1.0),
            N(w + 2.0),
            N(h + 2.0),
            k.way.light()
        );
    }
    // The sign, printed (worn into the face in the worn skin, as the print is).
    let opacity = match (k.inert, worn) {
        (true, _) => 0.5,
        (false, true) => 0.94,
        (false, false) => 1.0,
    };
    put!(
        s,
        "<g transform='translate({} 0)' opacity='{}'>",
        N(KEY_SIGN),
        N(opacity)
    );
    match k.word {
        // (Its capitals' middle on the key's, as the panel's legends beside a control.)
        Some(word) => text(
            &mut s,
            0.0,
            0.0,
            word,
            lettering.legend(),
            Style::Legend,
            true,
            "",
        ),
        None => note_sign(
            &mut s,
            k.division,
            sign_head(lettering),
            lettering.number(),
            times,
        ),
    }
    s.0.push_str("</g>");
    let over = if k.inert {
        format!(
            "<rect x='{}' y='{}' width='{}' height='{}' rx='4' fill='#000' fill-opacity='0.5'/>",
            N(-w / 2.0),
            N(-h / 2.0),
            N(w),
            N(h)
        )
    } else {
        String::new()
    };
    Layer {
        origin: c.centre(),
        bounds: bounds(&Kind::Key),
        body: s.0,
        sprites: vec![sprite(
            match (lit, k.way) {
                (false, ModWay::Clockwise) => Part::Key,
                (true, ModWay::Clockwise) => Part::KeyLit,
                (false, ModWay::Anticlockwise) => Part::KeyBlue,
                (true, ModWay::Anticlockwise) => Part::KeyBlueLit,
                (false, ModWay::Both) => Part::KeyRed,
                (true, ModWay::Both) => Part::KeyRedLit,
            },
            KEY,
            0.0,
            (false, false),
        )],
        over,
        ..Layer::default()
    }
}

/// The lit key's light round it.
const KEY_LIGHT: &str = "#ff8a2e";

/// A note sign's notehead's height (decisions.md R-OSC-TIMED): about the panel's lettering's
/// lower case, the note standing at its natural height above it (taller than the key: the owner,
/// 2026-10-20, "I would rather that than have these stumpy-looking notes").
pub fn sign_head(lettering: Lettering) -> f64 {
    lettering.legend() * 7.0 / 19.0
}

/// Whether a two-bar sign is a double whole note (`CA72_TIMED_BREVE=1`, the mock-ups'
/// comparison) rather than a whole note marked 2×.
fn breve_for_two_bars() -> bool {
    std::env::var("CA72_TIMED_BREVE").is_ok_and(|v| v == "1")
}

/// A note value's sign (decisions.md R-OSC-TIMED), its left edge at x 0 and the note centred on
/// y 0 from its notehead to its stem's or flag's tip (the owner, 2026-10-20: "can you center
/// everything?"; its dot and its triplet's 3 hang off it, outside the centring), its notehead
/// `head` tall, in the panel's ink: engraved notes (`crate::notes`, GNU
/// FreeSerif's), 4 and 2 bars a whole note marked 4× and 2× (the dial's numbers), 1 bar a whole
/// note, then a half note to a thirty-second, a dotted value with its dot and a triplet with a
/// small 3. `division` is its place among the eighteen; `num` the dials' numbers' size.
pub fn note_sign(s: &mut Svg, division: usize, head: f64, num: f64, times: &str) {
    use crate::notes::{Note, glyph};
    // (Its value: 0 whole, 1 half, 2 quarter, 3 eighth, 4 sixteenth, 5 thirty-second; and its
    // mark.)
    enum Mark {
        None,
        Dot,
        Triplet,
        Times(&'static str),
    }
    let (value, mark) = match division {
        0 => (0, Mark::Times("4×")),
        1 => (0, Mark::Times("2×")),
        2 => (0, Mark::None),
        d => {
            let (j, r) = ((d - 3) / 3, (d - 3) % 3);
            match r {
                0 => (j + 1, Mark::Dot),
                1 => (j, Mark::Triplet),
                _ => (j + 1, Mark::None),
            }
        }
    };
    let note = if division == 1 && breve_for_two_bars() {
        Note::DoubleWhole
    } else {
        Note::Value(value.min(5) as u8)
    };
    let g = glyph(note);
    let k = head / crate::notes::HEAD;
    let dy = -(g.bounds[1] + g.bounds[3]) / 2.0 * k;
    // (Thickened a little, as the legends are, to stand with the lettering.)
    put!(
        s,
        "<g transform='translate(0 {}) scale({})' fill='{LEGEND}' stroke='{LEGEND}' stroke-width='{}' stroke-linejoin='round'>{}",
        N(dy),
        N(k),
        N(0.45 / k),
        g.path
    );
    let width = g.bounds[2];
    if matches!(mark, Mark::Dot) {
        let d = glyph(Note::Dot);
        put!(
            s,
            "<g transform='translate({} 0)'>{}</g>",
            N(width + 50.0),
            d.path
        );
    }
    s.0.push_str("</g>");
    let (width, top) = (width * k, g.bounds[1] * k + dy);
    match mark {
        Mark::None | Mark::Dot => {}
        Mark::Triplet => text(
            s,
            width + 1.4,
            if value == 0 { top - 1.0 } else { top + 3.6 },
            "3",
            num * 0.7,
            Style::Legend,
            true,
            "",
        ),
        Mark::Times(t) => text(
            s,
            width + 2.6,
            dy,
            &t.replace('×', times),
            num * 0.85,
            Style::Legend,
            true,
            "",
        ),
    }
}

/// A wheel's slot in the worn skin: its opening, black, under the wheel the renderer draws.
fn wheel_slot() -> String {
    format!(
        "<g transform='scale({WHEEL_SCALE})'><rect x='{}' y='{}' width='{}' height='{}' rx='7' fill='{HOLE}'/></g>",
        N(-SLOT_W / 2.0),
        N(-SLOT_H / 2.0),
        N(SLOT_W),
        N(SLOT_H)
    )
}

/// A wheel in the worn skin, drawn by the renderer: its mark, and how far it is turned
/// (degrees, 0 in its middle).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WheelArt {
    pub mark: Mark,
    pub turn: f64,
}

/// Which way a rocker's raised half lies on the panel (a unit vector, the drawing's x and y),
/// turned `deg` and mirrored when `on`; and how squarely the lamp, up and to the left, falls on
/// a face of the paddle tilted towards `towards` (-1 to 1).
fn raised_towards(deg: f64, on: bool) -> (f64, f64) {
    let k = if on { -1.0 } else { 1.0 };
    (k * deg.to_radians().cos(), k * deg.to_radians().sin())
}

fn lamp_on(towards: (f64, f64)) -> f64 {
    -(towards.0 + towards.1) / std::f64::consts::SQRT_2
}

/// A rocker's paddle the worn skin's renderer lights by its shape: `pw` by `ph` along its own
/// length, turned `deg`, its raised half to its right, or (when `on`) its left, before turning.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaddleArt {
    pub pw: f64,
    pub ph: f64,
    pub deg: f64,
    pub on: bool,
}

/// The shadow a rocker's paddle casts on the panel, down and to the right of it, the raised end's
/// the longer.
fn paddle_shadow(pw: f64, ph: f64, deg: f64, on: bool) -> String {
    let r = raised_towards(deg, on);
    // How far the raised end's shadow falls beyond the paddle: most when it lies away from the
    // lamp.
    let reach = 4.0 + 6.0 * (-lamp_on(r)).max(0.0);
    let (ex, ey) = (r.0 * pw / 2.0, r.1 * pw / 2.0);
    format!(
        "<defs><filter id='cast' x='-0.5' y='-0.5' width='2' height='2'><feGaussianBlur stdDeviation='3'/></filter></defs><g filter='url(#cast)' fill='#000'><rect x='{}' y='{}' width='{}' height='{}' rx='4' transform='translate(4 6) rotate({})' fill-opacity='0.45'/><circle cx='{}' cy='{}' r='{}' fill-opacity='0.35'/></g>",
        N(-pw / 2.0),
        N(-ph / 2.0),
        N(pw),
        N(ph),
        N(deg),
        N(ex + 3.0 + r.0.abs() * reach * 0.6),
        N(ey + 5.0 + r.1.abs() * reach * 0.6),
        N(ph * 0.42)
    )
}

/// The POWER switch in the worn skin: its picture's upper half raised, its lower when on.
pub fn power_worn(on: bool) -> Layer {
    Layer {
        origin: (COL + POWER.0, TOP + POWER.1),
        bounds: POWER_BOUNDS,
        sprites: vec![sprite(Part::Power, (46.0, 124.0), 0.0, (false, on))],
        ..Layer::default()
    }
}

/// A jewel's light from its lamp: hot in the middle, red to its rim, and a little round it.
fn jewel_light(r: f64, level: f64) -> String {
    format!(
        "<defs><radialGradient id='lit' fx='0.4' fy='0.38'><stop offset='0' stop-color='#ffd0a0'/><stop offset='0.45' stop-color='#ff4a28'/><stop offset='1' stop-color='#c81408' stop-opacity='0.3'/></radialGradient><filter id='halo' x='-1' y='-1' width='3' height='3'><feGaussianBlur stdDeviation='{}'/></filter></defs><g opacity='{}'><circle r='{}' fill='#ff3018' fill-opacity='0.55' filter='url(#halo)'/><circle r='{}' fill='url(#lit)'/></g>",
        N(r * 0.45),
        N(level),
        N(r * 1.05),
        N(r)
    )
}

/// The POWER lamp in the worn skin: a red jewel in its chrome bezel, its filament lit while
/// the plug-in plays.
pub fn lamp_worn(on: bool) -> Layer {
    Layer {
        origin: (COL + LAMP_AT.0, TOP + LAMP_AT.1),
        bounds: [-30.0, -30.0, 30.0, 30.0],
        sprites: vec![sprite(Part::Jewel, (40.0, 40.0), 0.0, (false, false))],
        over: if on {
            jewel_light(12.5, 1.0)
        } else {
            String::new()
        },
        ..Layer::default()
    }
}

/// The OVERLOAD lamp in the worn skin at `level` (0 dark, 1 fully lit): a dark jewel, its
/// filament glowing red through it.
pub fn overload_worn(level: f64) -> Layer {
    let level = (level.clamp(0.0, 1.0) * 100.0).round() / 100.0;
    Layer {
        origin: (COL + OVERLOAD_AT.0, TOP + OVERLOAD_AT.1),
        bounds: [-34.0, -34.0, 34.0, 34.0],
        sprites: vec![sprite(Part::JewelDark, (48.0, 48.0), 0.0, (false, false))],
        over: if level >= 0.05 {
            jewel_light(15.0, level)
        } else {
            String::new()
        },
        ..Layer::default()
    }
}

/// What the worn skin lays over its pictures of the wood and the face, once a scale: the
/// wood's edges rounded over where the lamp falls and darker where they turn away, the shadow
/// the top strip casts on the faces, the face's aluminium trim, and each moving part's shadow,
/// soft, down and to the right (a part's shadow is the same whichever way it is turned).
pub fn worn_overlay(arrangement: &Arrangement) -> String {
    let mut s = Svg::default();
    put!(
        s,
        "<defs><linearGradient id='edge-lit' x1='0' y1='0' x2='0' y2='1'><stop offset='0' stop-color='#ffe6c8' stop-opacity='0.16'/><stop offset='1' stop-color='#ffe6c8' stop-opacity='0'/></linearGradient><linearGradient id='edge-dark' x1='0' y1='0' x2='0' y2='1'><stop offset='0' stop-color='#000' stop-opacity='0'/><stop offset='1' stop-color='#000' stop-opacity='0.45'/></linearGradient><linearGradient id='under-wood' x1='0' y1='0' x2='0' y2='1'><stop offset='0' stop-color='#000' stop-opacity='0.6'/><stop offset='1' stop-color='#000' stop-opacity='0'/></linearGradient><filter id='long' x='-0.6' y='-0.6' width='2.2' height='2.2'><feGaussianBlur stdDeviation='7'/></filter><filter id='soft' x='-0.5' y='-0.5' width='2' height='2'><feGaussianBlur stdDeviation='3.5'/></filter><filter id='softer' x='-0.5' y='-0.5' width='2' height='2'><feGaussianBlur stdDeviation='2'/></filter></defs>"
    );
    let rail = crate::strip::RAIL;
    let p = arrangement.panel;
    let wood = if p.wood > 0.0 {
        vec![
            (p.top(), 10.0, "edge-lit"),
            (p.face_top() - 14.0, 14.0, "edge-dark"),
        ]
    } else {
        vec![]
    };
    for (y, h, fill) in wood.into_iter().chain([
        (PANEL_H, 10.0, "edge-lit"),
        (PANEL_H + rail - 14.0, 14.0, "edge-dark"),
    ]) {
        put!(
            s,
            "<rect x='0' y='{}' width='{}' height='{}' fill='url(#{fill})'/>",
            N(y),
            N(W),
            N(h)
        );
    }
    for y in [p.face_top(), PANEL_H + rail] {
        if y == p.face_top() && p.wood == 0.0 {
            continue;
        }
        put!(
            s,
            "<rect x='{}' y='{}' width='{}' height='14' fill='url(#under-wood)'/>",
            N(COL),
            N(y),
            N(PW)
        );
    }
    put!(
        s,
        "<g transform='translate({} {})'>",
        N(COL),
        N(p.face_top())
    );
    trims(&mut s, p.face);
    s.0.push_str("</g>");
    put!(s, "<g transform='translate(0 {})'>", N(PANEL_H + rail));
    strip_trims(&mut s);
    s.0.push_str("</g>");
    // The moving parts' shadows (the drawers' knobs cast theirs on their drawers, the LFO
    // page's on its page).
    for c in arrangement
        .controls
        .iter()
        .filter(|c| !c.place.drawn_apart())
    {
        let (x, y) = c.centre();
        match c.kind {
            // A tall knob casts a long soft shadow away from the lamp, and a dark one where it
            // stands.
            Kind::Knob { .. } | Kind::Selector(_) => {
                let r = match c.kind {
                    Kind::Knob { big: true, .. } => BIG.skirt,
                    Kind::Knob { .. } => STD.skirt,
                    _ => SELECTOR_R + 2.0,
                };
                put!(
                    s,
                    "<circle cx='{}' cy='{}' r='{}' fill='#000' fill-opacity='0.55' filter='url(#long)'/><circle cx='{}' cy='{}' r='{}' fill='#000' fill-opacity='0.7' filter='url(#softer)'/>",
                    N(x + 10.0),
                    N(y + 14.0),
                    N(r),
                    N(x + 2.0),
                    N(y + 3.0),
                    N(r)
                );
            }
            // The toggle's nut's (its lever's is too small to tell).
            Kind::Toggle { w, h, .. } => put!(
                s,
                "<circle cx='{}' cy='{}' r='{}' fill='#000' fill-opacity='0.5' filter='url(#softer)'/>",
                N(x + 2.0),
                N(y + 3.0),
                N(w.min(h) * 0.46)
            ),
            // A rocker's own shadow goes with its layer; its opening's is the panel's.
            Kind::Rocker { w, h, .. } => put!(
                s,
                "<rect x='{}' y='{}' width='{}' height='{}' rx='3' fill='#000' fill-opacity='0.4' filter='url(#softer)'/>",
                N(x - w / 2.0 + 1.0),
                N(y - h / 2.0 + 2.0),
                N(w + 3.0),
                N(h + 3.0)
            ),
            // A key's cap's is its own: it comes and goes ([`comes_and_goes`]).
            Kind::Key => {}
            Kind::Wheel { .. } | Kind::Jack => {}
        }
    }
    put!(
        s,
        "<rect x='{}' y='{}' width='48' height='124' rx='3' fill='#000' fill-opacity='0.55' filter='url(#softer)'/>",
        N(COL + arrangement.power.0 - 21.0),
        N(TOP + arrangement.power.1 - 59.0)
    );
    // (QUALITY's opening's ring casts its own, with it: it can be hidden, `ultra::ring_shade`.)
    for ((x, y), r) in [(arrangement.lamp, 20.0), (arrangement.overload, 24.0)] {
        put!(
            s,
            "<circle cx='{}' cy='{}' r='{}' fill='#000' fill-opacity='0.55' filter='url(#softer)'/>",
            N(COL + x + 2.0),
            N(TOP + y + 3.0),
            N(r)
        );
    }
    for (x, y, r, nut) in jacks(arrangement) {
        put!(
            s,
            "<circle cx='{}' cy='{}' r='{}' fill='#000' fill-opacity='0.5' filter='url(#softer)'/>",
            N(x + 2.0),
            N(y + 3.0),
            N(if nut { r + 6.0 } else { r * 1.12 })
        );
    }
    s.0
}

/// A hover tip's box (its border's stroke included) about the point it is centred above: left,
/// top, right, foot (panel units).
pub fn tip_bounds(fonts: &Fonts, t: &str, size: f64) -> [f64; 4] {
    let (w, h, gap) = tip_box(fonts, t, size);
    let m = size / 6.0;
    [-w / 2.0 - m, -h - gap - m, w / 2.0 + m, m]
}

/// A tip's box's width and height, and its gap above the point it names.
fn tip_box(fonts: &Fonts, t: &str, size: f64) -> (f64, f64, f64) {
    let pad = size * 0.4;
    let w = t
        .split('\n')
        .map(|line| fonts.advance(line, size, Weight::Regular, 0.0))
        .fold(0.0, f64::max)
        + 2.0 * pad;
    (w, size * 1.5 * t.split('\n').count() as f64, size * 0.3)
}

/// A hover tip: `text` in a box of the tip's colours (`TIP`, `TIP_BORDER`), `size` its
/// lettering's height in panel units, centred above (x, y). Newlines separate lines.
pub fn tip(fonts: &Fonts, x: f64, y: f64, t: &str, size: f64) -> Layer {
    let (w, h, gap) = tip_box(fonts, t, size);
    let (x0, y0) = (-w / 2.0, -h - gap);
    let mut s = Svg::default();
    put!(
        s,
        "<rect x='{}' y='{}' width='{}' height='{}' rx='{}' fill='{TIP}' stroke='{TIP_BORDER}' stroke-width='{}'/>",
        N(x0),
        N(y0),
        N(w),
        N(h),
        N(size * 0.25),
        N(size / 12.0)
    );
    for (i, line) in t.split('\n').enumerate() {
        put!(
            s,
            "<text x='0' y='{}' text-anchor='middle' font-size='{}' fill='{TIP_TEXT}'>{}</text>",
            N(y0 + size * (0.75 + 1.5 * i as f64) + size * 0.36),
            N(size),
            escape(line)
        );
    }
    Layer {
        origin: (x, y),
        bounds: tip_bounds(fonts, t, size),
        body: s.0,
        ..Layer::default()
    }
}

// ---- The LFO page's (decisions.md R-LFO; `strip::lfo`): its print and its jacks, drawn with the
// panel's own parts.

/// A dial's print at (`x`, `y`), the multiplication sign `times`.
pub(crate) fn dial_print(s: &mut Svg, x: f64, y: f64, dial: Dial, times: &str) {
    draw_dial_in(s, x, y, dial, times);
}

/// A WAVEFORM switch's ticks and legends (oscillators 1 and 2's) at (`x`, `y`), and CONTINUOUS's
/// arc where it is (`continuous`).
pub(crate) fn waveform_marks(s: &mut Svg, x: f64, y: f64, layout: &Layout, continuous: bool) {
    let c = Control {
        param: "",
        label: "",
        place: Place::Lfo,
        x,
        y,
        kind: Kind::Selector(Legends::Waves12),
    };
    selector_marks_with(s, &c, Legends::Waves12, layout, None);
    if continuous {
        shape_arc(s, x, y);
    }
}

/// One of the panel's screws, `r` in radius.
pub(crate) fn screw_at(s: &mut Svg, x: f64, y: f64, r: f64) {
    screw(s, x, y, r);
}

/// A jack at (`x`, `y`), `r` its ring's radius, with its nut, as the left hand's are: the worn
/// skin's picture, or the drawn one.
pub(crate) fn jack_layer(x: f64, y: f64, r: f64, worn: bool) -> Layer {
    let d = 2.0 * (r + 6.0);
    if worn {
        return Layer {
            origin: (x, y),
            bounds: [-d, -d, d, d],
            sprites: vec![sprite(Part::Jack, (d, d), 20.0, (false, false))],
            ..Layer::default()
        };
    }
    let mut s = Svg::default();
    jack(&mut s, r, true);
    Layer {
        origin: (x, y),
        bounds: [-d, -d, d, d],
        body: s.0,
        ..Layer::default()
    }
}

/// Where every knob's pointer stands at the ends of its travel (degrees from twelve o'clock):
/// stop to stop, as its dial's printed ticks run (TUNE's, CUTOFF's, the FREQUENCYs' and the
/// TIMEs' numbers stop short of them; the TIMEs' last tick is at 148).
pub const KNOB_SWEEP: (f64, f64) = (-150.0, 150.0);

/// Where a knob's pointer stands at `v` (0..1) of its travel.
pub fn knob_angle(v: f64) -> f64 {
    KNOB_SWEEP.0 + (KNOB_SWEEP.1 - KNOB_SWEEP.0) * v
}

/// Where a control that takes a modulation's rings (decisions.md R-LFO) points at the ends of
/// its travel, its own sweep (R-LFO's corrections: the owner, 2026-10-20, "the % seems to be
/// calculated as the same radial distance for each knob regardless the total active radius that
/// knob has"): a knob's [`KNOB_SWEEP`], a WAVEFORM switch's CONTINUOUS travel from TRIANGLE's
/// place to the floor ([`shape_angle`], about 199 degrees).
pub fn sweep(kind: &Kind) -> (f64, f64) {
    match *kind {
        Kind::Selector(_) => (shape_angle(0.0), shape_angle(SHAPE_PLACES)),
        _ => KNOB_SWEEP,
    }
}

/// The radius a control's rings go round (decisions.md R-LFO): a knob's skirt, a selector's
/// pointer and a little more; none for a control that takes none.
pub fn skirt(kind: &Kind) -> f64 {
    match *kind {
        Kind::Knob { big: true, .. } => BIG.skirt,
        Kind::Knob { .. } => STD.skirt,
        Kind::Selector(_) => SELECTOR_R + 2.0,
        // (An LFO's OUT jack's, its LEVEL's: R-LFO's addendum.)
        Kind::Jack => crate::strip::lfo::JACK_R + 8.0,
        _ => 0.0,
    }
}

#[cfg(test)]
mod glide_tests {
    use super::*;

    #[test]
    fn long_glides_sign_uses_the_typefaces_multiplication_sign_or_x() {
        let fonts = Fonts::new();
        let a = Arrangement::official();
        let mut layout = Layout::measure(&fonts);
        assert!(glide_long_print(&layout, &a).contains(">×5</text>"));
        layout.times = false;
        assert!(glide_long_print(&layout, &a).contains(">x5</text>"));
    }
}
