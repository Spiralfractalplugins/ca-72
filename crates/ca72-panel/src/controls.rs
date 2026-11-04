//! The panel's controls: where each is, what it looks like, and the parameter it operates.

/// A knob's printed dial.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dial {
    /// 0 to 10.
    Ten,
    /// TUNE: –2 to 2 (semitones, as printed).
    Tune,
    /// CUTOFF FREQUENCY: –4 to 4.
    Cutoff,
    /// An oscillator's FREQUENCY: –7 to 7 semitones.
    Freq,
    /// The contours' ATTACK and DECAY TIME, in time.
    Time,
    /// MODULATION MIX: 0 to 10 between OSC. 3 and NOISE.
    ModMix,
    /// The strip's (A6): VOICES, 2 to 10, a step a voice.
    Voices,
    /// DETUNE, DOUBLE's: 0 to 100 cents.
    Detune,
    /// DRIVE: 0 to 24 dB.
    Drive,
    /// LEVEL: -30 to +12 dB.
    Level,
    /// The arpeggiator's (decisions.md R-ARP): RATE, its note values 2 (a half note) to 32, the
    /// straight ones numbered, the dotted and triplet ones between them unnumbered ticks.
    ArpRate,
    /// OCTAVES, 1 to 5.
    ArpOctaves,
    /// GATE and SWING: 0 to 100 %.
    Percent,
    /// The expression drawer's MODULATION (decisions.md R-MPE): –100 to 100 %.
    Reach,
    /// An oscillator's FREQUENCY while it is timed (decisions.md R-OSC-TIMED): its DIVISION's
    /// eighteen note values, 4 and 2 bars (4×, 2×) and 1 bar to a thirty-second (1 to 32), the
    /// dotted and triplet ones between them unnumbered ticks.
    Timed,
    /// An LFO's RATE (decisions.md R-LFO): 0.01 to 50 Hz on its parameter's skewed travel
    /// (about 1.6 Hz at its middle), Hz below.
    LfoRate,
    /// An LFO's PHASE: 0 to 360 degrees.
    LfoPhase,
    /// An LFO's FADE (decisions.md R-LFO-FADE): off at its top, out to the left and in to the
    /// right, 10 s at each end, on a cube-root travel each way, so that short fades have room;
    /// its centre a detent (`interact::release`).
    LfoFade,
    /// A macro's (decisions.md R-MACROS): 0 to 10, eleven ticks, only its ends numbered, its
    /// name's screen over its top.
    Macro,
}

impl Dial {
    /// How many steps its travel is in, for a knob of discrete positions (it goes from one to
    /// the next, never between them: the owner, 2026-10-10); none for one that turns smoothly.
    pub fn steps(self) -> Option<usize> {
        match self {
            Dial::Voices => Some(8),
            Dial::ArpRate => Some(12),
            Dial::ArpOctaves => Some(4),
            Dial::Timed => Some(TIMED_STEPS),
            _ => None,
        }
    }
}

/// The timed dial's steps: its eighteen note values' seventeen intervals (R-OSC-TIMED).
pub const TIMED_STEPS: usize = 17;

/// The time dial's labelled marks: angle in degrees and milliseconds (R-KNOB-ENTRY).
pub const TIME_MARKS: [(f64, f64); 6] = [
    (-120.0, 10.0),
    (-90.0, 200.0),
    (-30.0, 600.0),
    (30.0, 1000.0),
    (67.0, 5000.0),
    (108.0, 10000.0),
];

/// A selector's legends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Legends {
    Ranges,
    /// Oscillators 1 and 2's waveforms.
    Waves12,
    /// Oscillator 3's (a reverse sawtooth second).
    Waves3,
    /// FILTER POLES's (decisions.md R-POLES): 1 to 4, on the middle four of the six places.
    Poles,
    /// FILTER MODE's (decisions.md R-POLES): LO, BAND, STEEP HIGH and NOTCH on the middle four,
    /// pictures of their responses.
    Modes,
}

impl Legends {
    /// The places of the six it stops at: all six, or FILTER POLES's middle four.
    pub fn places(self) -> core::ops::RangeInclusive<usize> {
        match self {
            Legends::Poles | Legends::Modes => 1..=4,
            _ => 0..=5,
        }
    }

    /// Its size against RANGE's and WAVEFORM's: the filter's two (decisions.md R-POLES) three
    /// quarters, so that both fit the switch column over the original's switches.
    pub fn scale(self) -> f64 {
        match self {
            Legends::Poles | Legends::Modes => 0.75,
            _ => 1.0,
        }
    }

    /// Place `k`, kept to the places it stops at, as its normalized value.
    pub fn at(self, k: f64) -> f64 {
        let p = self.places();
        k.clamp(*p.start() as f64, *p.end() as f64) / 5.0
    }
}

/// A rocker's colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Colour {
    Blue,
    Red,
    Black,
    Ivory,
}

/// Where a rocker's "on" end is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orient {
    Right,
    Top,
    Bottom,
}

/// A wheel's mark: the PITCH wheel's line, the MODULATION wheel's dot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Line,
    Dot,
}

/// What a control is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Knob {
        dial: Dial,
        big: bool,
    },
    /// Six positions (RANGE, WAVEFORM).
    Selector(Legends),
    /// A switch, `w` by `h` as seen.
    Rocker {
        w: f64,
        h: f64,
        colour: Colour,
        orient: Orient,
    },
    /// A three-position toggle (QUALITY, NOTE PRIORITY), `w` by `h` as seen: its lever down
    /// (0), out (0.5) or up (1); `across`, on its side, left (0), out or right (1).
    Toggle {
        w: f64,
        h: f64,
        across: bool,
    },
    /// `detent`: centred (PITCH); `span`: its turn over its travel, in degrees.
    Wheel {
        mark: Mark,
        detent: bool,
        span: f64,
    },
    /// A small amber key lit from inside while on (`art::KEY` in size), the note value its
    /// oscillator is timed at printed beside it: an oscillator's TIMED (decisions.md
    /// R-OSC-TIMED). It flips at a click, as a switch.
    Key,
    /// An LFO's OUT jack, as its LEVEL's control (decisions.md R-LFO's addendum): no knob of
    /// its own (the page draws the jack), its rings round the jack; a press there drags an
    /// assignment from it, the wheel turns its LEVEL, a typed value sets it.
    Jack,
}

impl Kind {
    /// Whether every fraction of a drag or scroll can change the value, rather than only
    /// whole positions. VOICES and the arpeggiator's RATE and OCTAVES have a knob's shape but
    /// discrete options.
    pub fn is_continuous(&self) -> bool {
        matches!(self, Self::Knob { dial, .. } if dial.steps().is_none())
            || matches!(self, Self::Wheel { .. } | Self::Jack)
    }
}

/// The part of the drawing a control's position is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// The panel, its origin at its top left corner.
    Panel,
    /// The left hand controller's, its origin where its column's was, now on the strip
    /// (`strip::LEFT_HAND`).
    Column,
    /// The strip's, its origin the drawing's.
    Strip,
    /// The arpeggiator's drawer's (`strip::arp`), its origin the drawing's with the drawer
    /// open: drawn by the drawer, found only while it is open.
    Arp,
    /// The expression drawer's (`strip::mpe`; decisions.md R-MPE), as the arpeggiator's.
    Mpe,
    /// The LFO page's (`strip::lfo`; decisions.md R-LFO), its origin the drawing's: drawn by
    /// the page, found only while it shows.
    Lfo,
    /// The macros' (`strip::macros`; decisions.md R-MACROS), in the left hand's area in its
    /// place, its origin the drawing's: drawn by their renderer, found only while they show.
    Macro,
}

impl Place {
    /// Whether it is a drawer's dropped from the rail: drawn by the drawer, found only while it
    /// is open.
    pub fn is_drawer(self) -> bool {
        matches!(self, Place::Arp | Place::Mpe)
    }

    /// Whether its controls are drawn apart from the panel's renderer and found only while
    /// their part shows: a drawer's, or the LFO page's.
    pub fn drawn_apart(self) -> bool {
        self.is_drawer() || self == Place::Lfo || self == Place::Macro
    }
}

/// A control of the panel, operating the parameter of the same function.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Control {
    /// Its parameter's id.
    pub param: &'static str,
    /// Its name, as its tip gives it.
    pub label: &'static str,
    pub place: Place,
    /// Its centre in its place.
    pub x: f64,
    pub y: f64,
    pub kind: Kind,
}

impl Control {
    /// Its centre in the drawing.
    pub fn centre(&self) -> (f64, f64) {
        let (ox, oy) = match self.place {
            Place::Panel => (crate::art::COL, crate::art::TOP),
            Place::Column => crate::strip::LEFT_HAND,
            Place::Strip | Place::Arp | Place::Mpe | Place::Lfo | Place::Macro => (0.0, 0.0),
        };
        (ox + self.x, oy + self.y)
    }
}

const fn knob(param: &'static str, label: &'static str, x: f64, y: f64, dial: Dial) -> Control {
    Control {
        param,
        label,
        place: Place::Panel,
        x,
        y,
        kind: Kind::Knob { dial, big: false },
    }
}

/// A knob of the arpeggiator's drawer, the `at`th of its units (`strip::arp::UNITS`).
const fn arp_knob(param: &'static str, label: &'static str, at: usize, dial: Dial) -> Control {
    Control {
        param,
        label,
        place: Place::Arp,
        x: crate::strip::arp::UNITS[at] + crate::strip::UNIT_KNOB,
        y: crate::strip::arp::ROW,
        kind: Kind::Knob { dial, big: false },
    }
}

/// A knob of the expression drawer, the `at`th of its units (`strip::mpe::UNITS`).
const fn mpe_knob(param: &'static str, label: &'static str, at: usize, dial: Dial) -> Control {
    Control {
        param,
        label,
        place: Place::Mpe,
        x: crate::strip::mpe::UNITS[at] + crate::strip::UNIT_KNOB,
        y: crate::strip::mpe::unit_row(at),
        kind: Kind::Knob { dial, big: false },
    }
}

/// A knob on the strip, at its place there (`strip::KNOBS`).
const fn strip_knob(param: &'static str, label: &'static str, at: usize, dial: Dial) -> Control {
    let (_, x, y) = crate::strip::KNOBS[at];
    Control {
        param,
        label,
        place: Place::Strip,
        x,
        y,
        kind: Kind::Knob { dial, big: false },
    }
}

const fn big(param: &'static str, label: &'static str, x: f64, y: f64) -> Control {
    Control {
        param,
        label,
        place: Place::Panel,
        x,
        y,
        kind: Kind::Knob {
            dial: Dial::Freq,
            big: true,
        },
    }
}

const fn selector(
    param: &'static str,
    label: &'static str,
    x: f64,
    y: f64,
    legends: Legends,
) -> Control {
    Control {
        param,
        label,
        place: Place::Panel,
        x,
        y,
        kind: Kind::Selector(legends),
    }
}

#[allow(clippy::too_many_arguments)]
const fn rocker(
    param: &'static str,
    label: &'static str,
    place: Place,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    colour: Colour,
    orient: Orient,
) -> Control {
    Control {
        param,
        label,
        place,
        x,
        y,
        kind: Kind::Rocker {
            w,
            h,
            colour,
            orient,
        },
    }
}

const fn switch(
    param: &'static str,
    label: &'static str,
    x: f64,
    y: f64,
    colour: Colour,
) -> Control {
    rocker(
        param,
        label,
        Place::Panel,
        x,
        y,
        108.0,
        50.0,
        colour,
        Orient::Right,
    )
}

/// An oscillator's TIMED key (R-OSC-TIMED), centred under its RANGE, or its MOD key
/// (R-CONTINUOUS-WAVE), centred under its WAVEFORM.
const fn key(param: &'static str, label: &'static str, x: f64, y: f64) -> Control {
    Control {
        param,
        label,
        place: Place::Panel,
        x,
        y,
        kind: Kind::Key,
    }
}

/// One of the LFO page's controls (decisions.md R-LFO), at its place in its column
/// (`strip::lfo::places`).
const fn lfo(param: &'static str, label: &'static str, at: (f64, f64), kind: Kind) -> Control {
    Control {
        param,
        label,
        place: Place::Lfo,
        x: at.0,
        y: at.1,
        kind,
    }
}

const fn lfo_knob(dial: Dial) -> Kind {
    Kind::Knob { dial, big: false }
}

/// LFO `n`'s (0 to 3) ten controls, in the order [`LFO_PARTS`] names them.
const fn lfo_column(
    n: usize,
    params: [&'static str; 10],
    labels: [&'static str; 10],
) -> [Control; 10] {
    let p = crate::strip::lfo::places(n);
    [
        lfo(
            params[0],
            labels[0],
            p.wave,
            Kind::Selector(Legends::Waves12),
        ),
        lfo(params[1], labels[1], p.rate, lfo_knob(Dial::LfoRate)),
        lfo(params[2], labels[2], p.timed, Kind::Key),
        lfo(params[3], labels[3], p.phase, lfo_knob(Dial::LfoPhase)),
        lfo(params[4], labels[4], p.fade, lfo_knob(Dial::LfoFade)),
        lfo(
            params[5],
            labels[5],
            p.mode,
            Kind::Toggle {
                w: crate::art::TOGGLE.1,
                h: crate::art::TOGGLE.0,
                across: true,
            },
        ),
        lfo(params[6], labels[6], p.mono, Kind::Key),
        lfo(params[7], labels[7], p.polarity, Kind::Key),
        lfo(params[8], labels[8], p.on, Kind::Key),
        lfo(params[9], labels[9], p.jack, Kind::Jack),
    ]
}

/// The parts of an LFO's column that are controls, their parameters' IDs' last parts.
pub const LFO_PARTS: [&str; 10] = [
    "waveform", "rate", "timed", "phase", "fade", "mode", "mono", "polarity", "on", "level",
];

/// LEVEL's place among [`LFO_PARTS`]: its OUT jack's (R-LFO's addendum).
pub const LFO_LEVEL_PART: usize = 9;

const LFO_COLUMNS: [[Control; 10]; 4] = [
    lfo_column(
        0,
        [
            "lfo1_waveform",
            "lfo1_rate",
            "lfo1_timed",
            "lfo1_phase",
            "lfo1_fade",
            "lfo1_mode",
            "lfo1_mono",
            "lfo1_polarity",
            "lfo1_on",
            "lfo1_level",
        ],
        [
            "LFO 1 WAVEFORM",
            "LFO 1 RATE",
            "LFO 1 TIMED",
            "LFO 1 PHASE",
            "LFO 1 FADE",
            "LFO 1 MODE (FREE, RETRIG or ENV)",
            "LFO 1 MONO",
            "LFO 1 POLARITY (BI or UNI)",
            "LFO 1 ON",
            "LFO 1 LEVEL",
        ],
    ),
    lfo_column(
        1,
        [
            "lfo2_waveform",
            "lfo2_rate",
            "lfo2_timed",
            "lfo2_phase",
            "lfo2_fade",
            "lfo2_mode",
            "lfo2_mono",
            "lfo2_polarity",
            "lfo2_on",
            "lfo2_level",
        ],
        [
            "LFO 2 WAVEFORM",
            "LFO 2 RATE",
            "LFO 2 TIMED",
            "LFO 2 PHASE",
            "LFO 2 FADE",
            "LFO 2 MODE (FREE, RETRIG or ENV)",
            "LFO 2 MONO",
            "LFO 2 POLARITY (BI or UNI)",
            "LFO 2 ON",
            "LFO 2 LEVEL",
        ],
    ),
    lfo_column(
        2,
        [
            "lfo3_waveform",
            "lfo3_rate",
            "lfo3_timed",
            "lfo3_phase",
            "lfo3_fade",
            "lfo3_mode",
            "lfo3_mono",
            "lfo3_polarity",
            "lfo3_on",
            "lfo3_level",
        ],
        [
            "LFO 3 WAVEFORM",
            "LFO 3 RATE",
            "LFO 3 TIMED",
            "LFO 3 PHASE",
            "LFO 3 FADE",
            "LFO 3 MODE (FREE, RETRIG or ENV)",
            "LFO 3 MONO",
            "LFO 3 POLARITY (BI or UNI)",
            "LFO 3 ON",
            "LFO 3 LEVEL",
        ],
    ),
    lfo_column(
        3,
        [
            "lfo4_waveform",
            "lfo4_rate",
            "lfo4_timed",
            "lfo4_phase",
            "lfo4_fade",
            "lfo4_mode",
            "lfo4_mono",
            "lfo4_polarity",
            "lfo4_on",
            "lfo4_level",
        ],
        [
            "LFO 4 WAVEFORM",
            "LFO 4 RATE",
            "LFO 4 TIMED",
            "LFO 4 PHASE",
            "LFO 4 FADE",
            "LFO 4 MODE (FREE, RETRIG or ENV)",
            "LFO 4 MONO",
            "LFO 4 POLARITY (BI or UNI)",
            "LFO 4 ON",
            "LFO 4 LEVEL",
        ],
    ),
];

/// Macro `n`'s (0 to 5) knob.
const fn macro_knob(param: &'static str, label: &'static str, n: usize) -> Control {
    let (x, y) = crate::strip::macros::knob_at(n);
    Control {
        param,
        label,
        place: Place::Macro,
        x,
        y,
        kind: Kind::Knob {
            dial: Dial::Macro,
            big: false,
        },
    }
}

/// Where the macros' knobs begin in [`CONTROLS`] (macro `n`'s is `MACRO_FIRST + n`).
pub const MACRO_FIRST: usize = 96;

/// The index in [`CONTROLS`] of LFO `n`'s (0 to 3) part `k` ([`LFO_PARTS`]' order): its first
/// seven together, its POLARITY and ON keys after the macros' (R-LFO's corrections: so that no
/// control moves), its LEVEL after those (R-LFO's addendum).
pub const fn lfo_index(n: usize, k: usize) -> usize {
    if k < 7 {
        LFO_FIRST + 7 * n + k
    } else if k < LFO_LEVEL_PART {
        LFO_KEYS_FIRST + 2 * n + (k - 7)
    } else {
        LFO_LEVEL_FIRST + n
    }
}

/// Where the LFOs' POLARITY and ON keys begin in [`CONTROLS`]: after the macros' knobs.
pub const LFO_KEYS_FIRST: usize = MACRO_FIRST + 6;

/// Where the LFOs' LEVELs (their OUT jacks) begin in [`CONTROLS`]: after their keys.
pub const LFO_LEVEL_FIRST: usize = LFO_KEYS_FIRST + 8;

/// Where the LFO page's controls begin in [`CONTROLS`].
pub const LFO_FIRST: usize = 68;

/// How far under its RANGE's centre an oscillator's TIMED key is (the mock-ups', the owner's
/// "under the oscillator range selector", 2026-10-20), and its MOD key under its WAVEFORM's.
pub const KEY_BELOW_RANGE: f64 = 84.0;

/// How far down FILTER POLES and FILTER MODE at the switch column's head moved the original's
/// switches under them (decisions.md R-POLES).
pub const COLUMN_DOWN: f64 = 172.0;

/// FILTER MODE's selector, under FILTER POLES in ATTACK TIME's row (decisions.md R-POLES).
pub const FILTER_MODE_Y: f64 = 322.0;

/// FILTER POLES, at the switch column's head in CUTOFF FREQUENCY's row (decisions.md R-POLES).
pub const FILTER_POLES_AT: (f64, f64) = (1907.0, 160.0);

/// The MIXER's rows.
const ROWS: [f64; 5] = [170.0, 274.0, 377.0, 481.0, 583.0];

/// Every control, in the order they are drawn.
pub const CONTROLS: [Control; 117] = [
    // CONTROLLERS.
    knob("tune", "TUNE", 229.0, 273.0, Dial::Tune),
    switch(
        "osc_mod",
        "OSCILLATOR MODULATION",
        444.0,
        269.0,
        Colour::Red,
    ),
    knob("glide", "GLIDE", 126.0, 480.0, Dial::Ten),
    knob("mod_mix", "MODULATION MIX", 332.0, 480.0, Dial::ModMix),
    // OSCILLATOR BANK.
    selector(
        "osc1_range",
        "OSCILLATOR-1 RANGE",
        617.0,
        163.0,
        Legends::Ranges,
    ),
    selector(
        "osc2_range",
        "OSCILLATOR-2 RANGE",
        617.0,
        370.0,
        Legends::Ranges,
    ),
    selector(
        "osc3_range",
        "OSCILLATOR-3 RANGE",
        617.0,
        576.0,
        Legends::Ranges,
    ),
    big("osc2_frequency", "OSCILLATOR-2 FREQUENCY", 820.0, 373.0),
    big("osc3_frequency", "OSCILLATOR-3 FREQUENCY", 820.0, 581.0),
    selector(
        "osc1_waveform",
        "OSCILLATOR-1 WAVEFORM",
        1021.0,
        163.0,
        Legends::Waves12,
    ),
    selector(
        "osc2_waveform",
        "OSCILLATOR-2 WAVEFORM",
        1021.0,
        370.0,
        Legends::Waves12,
    ),
    selector(
        "osc3_waveform",
        "OSCILLATOR-3 WAVEFORM",
        1021.0,
        576.0,
        Legends::Waves3,
    ),
    rocker(
        "osc3_control",
        "OSC. 3 CONTROL",
        Place::Panel,
        480.0,
        585.0,
        55.0,
        111.0,
        Colour::Red,
        Orient::Top,
    ),
    // MIXER: the oscillators' VOLUME at the left, EXTERNAL INPUT and NOISE at the right.
    knob(
        "osc1_volume",
        "OSCILLATOR-1 VOLUME",
        1220.0,
        ROWS[0],
        Dial::Ten,
    ),
    switch("osc1_on", "OSCILLATOR-1 ON", 1419.0, ROWS[0], Colour::Blue),
    knob(
        "ext_volume",
        "EXTERNAL INPUT VOLUME",
        1621.0,
        ROWS[1],
        Dial::Ten,
    ),
    switch("ext_on", "EXTERNAL INPUT ON", 1419.0, ROWS[1], Colour::Blue),
    knob(
        "osc2_volume",
        "OSCILLATOR-2 VOLUME",
        1220.0,
        ROWS[2],
        Dial::Ten,
    ),
    switch("osc2_on", "OSCILLATOR-2 ON", 1419.0, ROWS[2], Colour::Blue),
    knob("noise_volume", "NOISE VOLUME", 1621.0, ROWS[3], Dial::Ten),
    switch("noise_on", "NOISE ON", 1419.0, ROWS[3], Colour::Blue),
    knob(
        "osc3_volume",
        "OSCILLATOR-3 VOLUME",
        1220.0,
        ROWS[4],
        Dial::Ten,
    ),
    switch("osc3_on", "OSCILLATOR-3 ON", 1419.0, ROWS[4], Colour::Blue),
    rocker(
        "noise_type",
        "NOISE (WHITE or PINK)",
        Place::Panel,
        1766.0,
        481.0,
        44.0,
        114.0,
        Colour::Blue,
        Orient::Bottom,
    ),
    // Across MIXER and MODIFIERS: the filter's mode, modulation and keyboard control, under
    // FILTER POLES at the column's head (decisions.md R-POLES; FILTER POLES itself is last in
    // this table). FILTER MODE, which the original does not have (decisions.md R46, R-POLES):
    // a selector of four places, LO, BAND, STEEP HIGH and NOTCH, its legends pictures of them.
    selector(
        "filter_mode",
        "FILTER MODE (LO, BAND, STEEP HIGH or NOTCH)",
        1907.0,
        FILTER_MODE_Y,
        Legends::Modes,
    ),
    switch(
        "filter_mod",
        "FILTER MODULATION",
        1907.0,
        291.0 + COLUMN_DOWN,
        Colour::Red,
    ),
    rocker(
        "keyboard_control_1",
        "KEYBOARD CONTROL 1",
        Place::Panel,
        1907.0,
        386.0 + COLUMN_DOWN,
        108.0,
        48.0,
        Colour::Red,
        Orient::Right,
    ),
    rocker(
        "keyboard_control_2",
        "KEYBOARD CONTROL 2",
        Place::Panel,
        1907.0,
        486.0 + COLUMN_DOWN,
        108.0,
        48.0,
        Colour::Red,
        Orient::Right,
    ),
    // MODIFIERS.
    knob("cutoff", "CUTOFF FREQUENCY", 2081.0, 170.0, Dial::Cutoff),
    knob("emphasis", "EMPHASIS", 2281.0, 170.0, Dial::Ten),
    knob(
        "contour_amount",
        "AMOUNT OF CONTOUR",
        2483.0,
        170.0,
        Dial::Ten,
    ),
    knob(
        "filter_attack",
        "FILTER CONTOUR ATTACK TIME",
        2080.0,
        358.0,
        Dial::Time,
    ),
    knob(
        "filter_decay",
        "FILTER CONTOUR DECAY TIME",
        2281.0,
        358.0,
        Dial::Time,
    ),
    knob(
        "filter_sustain",
        "FILTER CONTOUR SUSTAIN LEVEL",
        2483.0,
        358.0,
        Dial::Ten,
    ),
    knob(
        "loudness_attack",
        "LOUDNESS CONTOUR ATTACK TIME",
        2080.0,
        579.0,
        Dial::Time,
    ),
    knob(
        "loudness_decay",
        "LOUDNESS CONTOUR DECAY TIME",
        2281.0,
        579.0,
        Dial::Time,
    ),
    knob(
        "loudness_sustain",
        "LOUDNESS CONTOUR SUSTAIN LEVEL",
        2483.0,
        579.0,
        Dial::Ten,
    ),
    // OUTPUT.
    knob("volume", "MAIN OUTPUT VOLUME", 2680.0, 167.0, Dial::Ten),
    rocker(
        "main_output",
        "MAIN OUTPUT",
        Place::Panel,
        2853.0,
        165.0,
        110.0,
        48.0,
        Colour::Blue,
        Orient::Right,
    ),
    rocker(
        "a440",
        "A-440",
        Place::Panel,
        2682.0,
        375.0,
        112.0,
        50.0,
        Colour::Blue,
        Orient::Right,
    ),
    // The phones' VOLUME is the plug-in's FEEDBACK (decisions.md R8): the PHONES output
    // cabled into EXTERNAL INPUT, as players overdrive the instrument.
    knob(
        "feedback",
        "FEEDBACK (the phones' VOLUME: the output into EXTERNAL INPUT)",
        2680.0,
        580.0,
        Dial::Ten,
    ),
    // QUALITY (decisions.md R47, R48): a chrome toggle over POWER's lamp, under
    // ULTRA's: its lever up for ULTRA, out for HI, down for LO.
    Control {
        param: "quality",
        label: "QUALITY (ULTRA, HI or LO)",
        place: Place::Panel,
        x: crate::art::QUALITY_AT.0,
        y: crate::art::QUALITY_AT.1,
        kind: Kind::Toggle {
            w: crate::art::TOGGLE.0,
            h: crate::art::TOGGLE.1,
            across: false,
        },
    },
    // The left hand controller.
    rocker(
        "glide_on",
        "GLIDE",
        Place::Column,
        crate::art::LH_ROCKER,
        crate::art::LH_ROWS[0],
        82.0,
        38.0,
        Colour::Ivory,
        Orient::Right,
    ),
    rocker(
        "decay_on",
        "DECAY",
        Place::Column,
        crate::art::LH_ROCKER,
        crate::art::LH_ROWS[1],
        82.0,
        38.0,
        Colour::Ivory,
        Orient::Right,
    ),
    Control {
        param: "pitch_wheel",
        label: "PITCH",
        place: Place::Column,
        x: crate::art::LH_WHEELS[0],
        y: crate::art::WHEEL_Y,
        kind: Kind::Wheel {
            mark: Mark::Line,
            detent: true,
            span: 70.0,
        },
    },
    Control {
        param: "mod_wheel",
        label: "MODULATION",
        place: Place::Column,
        x: crate::art::LH_WHEELS[1],
        y: crate::art::WHEEL_Y,
        kind: Kind::Wheel {
            mark: Mark::Dot,
            detent: false,
            span: 76.0,
        },
    },
    // The strip's knobs (A6: decisions.md R44, R45, R49), the plug-in's own; INNER
    // last, so that every other control keeps its index.
    strip_knob("voices", "VOICES", 0, Dial::Voices),
    strip_knob("entropy", "ENTROPY", 1, Dial::Ten),
    strip_knob("spread", "WIDTH", 2, Dial::Ten),
    strip_knob("double", "DETUNE (DOUBLE)", 3, Dial::Detune),
    strip_knob("drive", "DRIVE", 4, Dial::Drive),
    strip_knob("level", "LEVEL", 5, Dial::Level),
    strip_knob("inner", "INNER", 6, Dial::Ten),
    // NOTE PRIORITY (decisions.md R50), the plug-in's own: QUALITY's toggle on its side,
    // the left hand's first row, over RETRIGGER, GLIDE and DECAY where their switches are, its
    // lever left for LOW, out for LAST, right for HIGH. Last, so that every other control keeps
    // its index.
    Control {
        param: "note_priority",
        label: "NOTE PRIORITY (LOW, LAST or HIGH)",
        place: Place::Column,
        x: crate::art::LH_ROCKER,
        y: crate::art::LH_PRIORITY,
        kind: Kind::Toggle {
            w: crate::art::TOGGLE.1,
            h: crate::art::TOGGLE.0,
            across: true,
        },
    },
    // RETRIGGER (R50), the plug-in's own: an ivory switch as GLIDE's and DECAY's, the
    // left hand's second row, over theirs. After NOTE PRIORITY, for the same reason.
    rocker(
        "retrigger",
        "RETRIGGER",
        Place::Column,
        crate::art::LH_ROCKER,
        crate::art::LH_RETRIGGER,
        82.0,
        38.0,
        Colour::Ivory,
        Orient::Right,
    ),
    // The arpeggiator's drawer's knobs (decisions.md R-ARP), the plug-in's own, after every
    // other, so that no control moves.
    arp_knob("arp_rate", "RATE", 0, Dial::ArpRate),
    arp_knob("arp_octaves", "OCTAVES", 1, Dial::ArpOctaves),
    arp_knob("arp_gate", "GATE", 2, Dial::Percent),
    arp_knob("arp_swing", "SWING", 3, Dial::Percent),
    // The expression drawer's (decisions.md R-MPE), the plug-in's own, after every other.
    mpe_knob("pressure_mod", "PRESSURE MODULATION", 0, Dial::Reach),
    mpe_knob("pressure_filter", "PRESSURE CUTOFF", 1, Dial::Cutoff),
    mpe_knob("timbre_mod", "TIMBRE MODULATION", 2, Dial::Reach),
    mpe_knob("timbre_filter", "TIMBRE CUTOFF", 3, Dial::Cutoff),
    // Oscillators 2's and 3's TIMED keys (decisions.md R-OSC-TIMED), each centred under its
    // own RANGE, after every other, so that no control moves. Oscillator 1 has none: it has no
    // FREQUENCY to turn into note values.
    key(
        "osc2_timed",
        "OSCILLATOR-2 TIMED",
        617.0,
        370.0 + KEY_BELOW_RANGE,
    ),
    key(
        "osc3_timed",
        "OSCILLATOR-3 TIMED",
        617.0,
        576.0 + KEY_BELOW_RANGE,
    ),
    // Each oscillator's MOD key (decisions.md R-CONTINUOUS-WAVE), centred under its own
    // WAVEFORM, after every other, so that no control moves: on the panel only while its
    // oscillator's waveform is CONTINUOUS.
    key(
        "osc1_shape_mod",
        "OSCILLATOR-1 MOD",
        1021.0,
        163.0 + KEY_BELOW_RANGE,
    ),
    key(
        "osc2_shape_mod",
        "OSCILLATOR-2 MOD",
        1021.0,
        370.0 + KEY_BELOW_RANGE,
    ),
    key(
        "osc3_shape_mod",
        "OSCILLATOR-3 MOD",
        1021.0,
        576.0 + KEY_BELOW_RANGE,
    ),
    // The LFO page's (decisions.md R-LFO), the plug-in's own, after every other, so that no
    // control moves: each LFO's WAVEFORM, RATE, its TIMED key, PHASE, FADE, MODE and MONO.
    LFO_COLUMNS[0][0],
    LFO_COLUMNS[0][1],
    LFO_COLUMNS[0][2],
    LFO_COLUMNS[0][3],
    LFO_COLUMNS[0][4],
    LFO_COLUMNS[0][5],
    LFO_COLUMNS[0][6],
    LFO_COLUMNS[1][0],
    LFO_COLUMNS[1][1],
    LFO_COLUMNS[1][2],
    LFO_COLUMNS[1][3],
    LFO_COLUMNS[1][4],
    LFO_COLUMNS[1][5],
    LFO_COLUMNS[1][6],
    LFO_COLUMNS[2][0],
    LFO_COLUMNS[2][1],
    LFO_COLUMNS[2][2],
    LFO_COLUMNS[2][3],
    LFO_COLUMNS[2][4],
    LFO_COLUMNS[2][5],
    LFO_COLUMNS[2][6],
    LFO_COLUMNS[3][0],
    LFO_COLUMNS[3][1],
    LFO_COLUMNS[3][2],
    LFO_COLUMNS[3][3],
    LFO_COLUMNS[3][4],
    LFO_COLUMNS[3][5],
    LFO_COLUMNS[3][6],
    // The macros' (decisions.md R-MACROS), the plug-in's own, after every other, so that no
    // control moves: two across, three down, in the left hand's area while they show.
    macro_knob("macro1", "MACRO 1", 0),
    macro_knob("macro2", "MACRO 2", 1),
    macro_knob("macro3", "MACRO 3", 2),
    macro_knob("macro4", "MACRO 4", 3),
    macro_knob("macro5", "MACRO 5", 4),
    macro_knob("macro6", "MACRO 6", 5),
    // Each LFO's POLARITY and ON keys (R-LFO's corrections), after the macros', so that no
    // control moves.
    LFO_COLUMNS[0][7],
    LFO_COLUMNS[0][8],
    LFO_COLUMNS[1][7],
    LFO_COLUMNS[1][8],
    LFO_COLUMNS[2][7],
    LFO_COLUMNS[2][8],
    LFO_COLUMNS[3][7],
    LFO_COLUMNS[3][8],
    // Each LFO's LEVEL, its OUT jack's (R-LFO's addendum), after those, so that no control
    // moves.
    LFO_COLUMNS[0][9],
    LFO_COLUMNS[1][9],
    LFO_COLUMNS[2][9],
    LFO_COLUMNS[3][9],
    // The velocity's CURVE and FLOOR, the expression drawer's second row (decisions.md
    // R-VELOCITY), after every other, so that no control moves.
    mpe_knob("velocity_curve", "VELOCITY CURVE", 4, Dial::Reach),
    mpe_knob("velocity_floor", "VELOCITY FLOOR", 5, Dial::Ten),
    // FILTER POLES (decisions.md R-POLES), at the switch column's head: the ladder's stage the
    // filter's output is taken from, or the slopes of its band-pass and high-pass. Last, so that
    // no control moves.
    selector(
        "filter_poles",
        "FILTER POLES (1 to 4)",
        FILTER_POLES_AT.0,
        FILTER_POLES_AT.1,
        Legends::Poles,
    ),
];

/// The panel's controls' history (decisions.md R-SKIN-PACKS), so that a skin pack's layout made
/// for one CA-72 works on later ones: the revision of the panel's controls this build has, and
/// each change since the first. A layout says which revision it was made for (`controls`):
/// a control added since is placed by the plug-in, a renamed one found by its old name, a
/// retired one passed over, and a control whose kind changed drawn in its own kind's look.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlSet {
    pub revision: u32,
    /// Controls added after the first revision: each with the revision it came in.
    pub added: &'static [(&'static str, u32)],
    /// Controls renamed: the old name, the new, and the revision of the change.
    pub renamed: &'static [(&'static str, &'static str, u32)],
    /// Controls removed: the name, and the revision it went in.
    pub retired: &'static [(&'static str, u32)],
    /// Controls whose kind changed (a switch become a toggle): the name, and the revision.
    pub kind_changed: &'static [(&'static str, u32)],
}

/// This build's panel controls: the eighth revision; oscillators 2's and 3's TIMED keys added in
/// the second (decisions.md R-OSC-TIMED), each oscillator's MOD key in the third
/// (R-CONTINUOUS-WAVE), the LFO page's in the fourth (R-LFO: the plug-in's own, on its lower
/// page, which a layout does not place, as the strip's), the six macros in the fifth (R-MACROS:
/// in the left hand's area, which a layout does not place either), each LFO's POLARITY (UNI) and
/// ON keys in the sixth (R-LFO's corrections), each LFO's LEVEL (its OUT jack) in the seventh
/// (R-LFO's addendum), FILTER POLES in the eighth, when FILTER MODE became a toggle (R-POLES).
/// Never edit an entry once released: add to it (and raise `revision`) as controls come, go, or
/// change.
pub const CONTROL_SET: ControlSet = ControlSet {
    revision: 8,
    added: &[
        ("osc2_timed", 2),
        ("osc3_timed", 2),
        ("osc1_shape_mod", 3),
        ("osc2_shape_mod", 3),
        ("osc3_shape_mod", 3),
        ("lfo1_waveform", 4),
        ("lfo1_rate", 4),
        ("lfo1_timed", 4),
        ("lfo1_phase", 4),
        ("lfo1_fade", 4),
        ("lfo1_mode", 4),
        ("lfo1_mono", 4),
        ("lfo2_waveform", 4),
        ("lfo2_rate", 4),
        ("lfo2_timed", 4),
        ("lfo2_phase", 4),
        ("lfo2_fade", 4),
        ("lfo2_mode", 4),
        ("lfo2_mono", 4),
        ("lfo3_waveform", 4),
        ("lfo3_rate", 4),
        ("lfo3_timed", 4),
        ("lfo3_phase", 4),
        ("lfo3_fade", 4),
        ("lfo3_mode", 4),
        ("lfo3_mono", 4),
        ("lfo4_waveform", 4),
        ("lfo4_rate", 4),
        ("lfo4_timed", 4),
        ("lfo4_phase", 4),
        ("lfo4_fade", 4),
        ("lfo4_mode", 4),
        ("lfo4_mono", 4),
        ("macro1", 5),
        ("macro2", 5),
        ("macro3", 5),
        ("macro4", 5),
        ("macro5", 5),
        ("macro6", 5),
        ("lfo1_polarity", 6),
        ("lfo1_on", 6),
        ("lfo2_polarity", 6),
        ("lfo2_on", 6),
        ("lfo3_polarity", 6),
        ("lfo3_on", 6),
        ("lfo4_polarity", 6),
        ("lfo4_on", 6),
        ("lfo1_level", 7),
        ("lfo2_level", 7),
        ("lfo3_level", 7),
        ("lfo4_level", 7),
        ("filter_poles", 8),
    ],
    renamed: &[],
    retired: &[],
    kind_changed: &[("filter_mode", 8)],
};

impl ControlSet {
    /// The revision a control came in (1 for the first's).
    pub fn since(&self, param: &str) -> u32 {
        self.added
            .iter()
            .find(|(p, _)| *p == param)
            .map_or(1, |(_, r)| *r)
    }

    /// A name a layout of revision `made_for` uses, as this build knows it: renamed since
    /// (`Some(new)`), or the same.
    pub fn current<'a>(&self, name: &'a str, made_for: u32) -> &'a str
    where
        'static: 'a,
    {
        let mut name = name;
        // (A chain of renames followed in order.)
        for (old, new, at) in self.renamed {
            if *old == name && *at > made_for {
                name = new;
            }
        }
        name
    }

    /// Whether `param` was retired after revision `made_for`.
    pub fn retired_since(&self, param: &str, made_for: u32) -> bool {
        self.retired
            .iter()
            .any(|(p, at)| *p == param && *at > made_for)
    }

    /// Whether `param`'s kind changed after revision `made_for`.
    pub fn kind_changed_since(&self, param: &str, made_for: u32) -> bool {
        self.kind_changed
            .iter()
            .any(|(p, at)| *p == param && *at > made_for)
    }
}

/// The index of the control operating `param`.
pub fn index(param: &str) -> Option<usize> {
    CONTROLS.iter().position(|c| c.param == param)
}

/// FEEDBACK's tip while it is silent ([`feedback_silent`]).
pub const FEEDBACK_SILENT: &str =
    "FEEDBACK (silent: switch EXTERNAL INPUT on in the MIXER and turn its VOLUME up)";

/// Whether FEEDBACK is silent at these values (normalized, by index in [`CONTROLS`]): it is
/// heard through the MIXER's EXTERNAL INPUT, so nothing goes back while that is switched off
/// or its VOLUME is at 0. Its knob is then dimmed, still turnable, and its tip says why (the
/// owner, 2026-10-02).
pub fn feedback_silent(values: &[f64; CONTROLS.len()]) -> bool {
    let at = |param| index(param).map_or(0.0, |i| values[i]);
    !(at("ext_on") >= 0.5 && at("ext_volume") > 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feedback_is_silent_while_external_input_is_closed() {
        let mut v = [0.5; CONTROLS.len()];
        let (on, volume) = (
            index("ext_on").expect("a control"),
            index("ext_volume").expect("a control"),
        );
        assert!(!feedback_silent(&v));
        v[on] = 0.0;
        assert!(feedback_silent(&v));
        v[on] = 1.0;
        v[volume] = 0.0;
        assert!(feedback_silent(&v));
        v[volume] = 0.01;
        assert!(!feedback_silent(&v));
    }
}
