//! MIDI Learn (decisions.md R34): a hardware controller's absolute 7-bit control change, on its
//! own MIDI channel, assigned to one of the plug-in's sound parameters; each instance its own
//! table, saved with the host's project, not with the sound presets. PREVIOUS PRESET and NEXT
//! PRESET also learn notes and are kept on this computer instead (R-PRESET-KEYS). A switch learns
//! a note too while the standalone's MIDI map mode has it waiting (R-MIDI-MAP): each note-on
//! turns it over, and the note is not played.
//!
//! - **What may be learned** ([`LEARNABLE`]): the panel's knobs, selectors and switches, the
//!   strip's POLY, UNISON, VOICES, ENTROPY, WIDTH (SPREAD), INNER, SCATTER's placement, DETUNE
//!   (DOUBLE), DRIVE, LEVEL and AUTO GAIN, LOCK (a host parameter without a control: the
//!   drawer's list learns it), the arpeggiator's seven (decisions.md R-ARP), and PREVIOUS
//!   PRESET and NEXT PRESET. The MOD wheel learns its computer setting separately
//!   (R-MODWHEEL-SOURCE). Not PITCH (MIDI pitch bend), POWER (the host's bypass), MIDI BEND
//!   RANGE (the player's), nor anything else.
//! - **What is not learned** ([`reserved`], [`MidiMap::reserved`]): bank select (CC 0, 32),
//!   the wheel's chosen CC,
//!   data entry and RPN/NRPN (CC 6, 38, 96–101) and the channel mode messages (CC 120–127). The
//!   channel mode actions (CC 120, 121, 123) keep doing what they did.
//! - **One controller a parameter, one parameter a controller**: learning a parameter replaces its
//!   controller; learning a controller another parameter had moves it ([`MidiMap::assign`] says
//!   which). Removing one leaves the sound as it is.
//! - **Learning**: the editor arms a parameter ([`MidiMap::arm`]); the audio thread takes the next
//!   learnable control change, on its channel, as the one ([`MidiMap::incoming`]), changing
//!   nothing with it; the editor makes the assignment at its next frame ([`MidiMap::poll`]).
//!   Escape, CANCEL and the editor closing disarm it ([`MidiMap::cancel`]); arming another
//!   parameter moves the learning there. Nothing of learning is saved.
//! - **Playing**: a learned control change sets its parameter at its sample, through the host
//!   (`ProcessContext::set_parameter_normalized`, `third_party/nih-plug/PATCHES.md` change 10):
//!   a knob to value / 127 of its travel, a selector (or VOICES) to the position its share of
//!   0–127 falls in, a switch off at 0–63 and on at 64–127. The value jumps there ("jump"
//!   takeover); a knob's voices follow it over [`DEZIP`] (`crate::smooth::Smooth::learned`).
//!
//! The audio thread reads the table and arms nothing: it only ever loads [`MidiMap`]'s atomics,
//! takes an arming with a compare-and-swap and posts what it caught in an atomic word. Every
//! change of the table is made off it, under a lock it never takes.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering};

use nih_plug::prelude::*;
use serde::de::{Deserialize, Deserializer};
use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};

use crate::mod_wheel::Source;
use ca72::freq_dial::Pot;

use crate::params::Ca72Params;

/// How a learned control change's 7-bit value sets a parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A knob or slider: value / 127 of its travel.
    Continuous,
    /// A selector (or VOICES): its positions each an equal share of 0–127.
    Stepped,
    /// A switch: off at 0–63, on at 64–127.
    Switch,
    /// A preset action: one press, rather than a parameter value.
    Action,
}

/// A parameter MIDI Learn may assign a controller to: its id (as saved), its name as the editor
/// gives it, and its kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Learnable {
    pub id: &'static str,
    pub label: &'static str,
    pub kind: Kind,
}

const fn knob(id: &'static str, label: &'static str) -> Learnable {
    Learnable {
        id,
        label,
        kind: Kind::Continuous,
    }
}

const fn stepped(id: &'static str, label: &'static str) -> Learnable {
    Learnable {
        id,
        label,
        kind: Kind::Stepped,
    }
}

const fn switch(id: &'static str, label: &'static str) -> Learnable {
    Learnable {
        id,
        label,
        kind: Kind::Switch,
    }
}

/// Every parameter MIDI Learn may assign a controller to, in the panel's order, then the strip's,
/// then LOCK, then the arpeggiator's, then the preset actions. Its order is the drawer's list;
/// the table is saved by id, so it may change.
pub const LEARNABLE: [Learnable; 169] = [
    // CONTROLLERS.
    knob("tune", "TUNE"),
    switch("osc_mod", "OSCILLATOR MODULATION"),
    knob("glide", "GLIDE"),
    switch("glide_range", "GLIDE RANGE"),
    knob("mod_mix", "MODULATION MIX"),
    // OSCILLATOR BANK.
    stepped("osc1_range", "OSCILLATOR-1 RANGE"),
    stepped("osc2_range", "OSCILLATOR-2 RANGE"),
    stepped("osc3_range", "OSCILLATOR-3 RANGE"),
    knob("osc2_frequency", "OSCILLATOR-2 FREQUENCY"),
    knob("osc3_frequency", "OSCILLATOR-3 FREQUENCY"),
    stepped("osc1_waveform", "OSCILLATOR-1 WAVEFORM"),
    stepped("osc2_waveform", "OSCILLATOR-2 WAVEFORM"),
    stepped("osc3_waveform", "OSCILLATOR-3 WAVEFORM"),
    switch("osc3_control", "OSC. 3 CONTROL"),
    // The oscillators timed (R-OSC-TIMED): DIVISION's steps as RATE's.
    switch("osc2_timed", "OSC. 2 TIMED"),
    stepped("osc2_division", "OSC. 2 DIVISION"),
    switch("osc3_timed", "OSC. 3 TIMED"),
    stepped("osc3_division", "OSC. 3 DIVISION"),
    // MIXER.
    knob("osc1_volume", "OSCILLATOR-1 VOLUME"),
    switch("osc1_on", "OSCILLATOR-1 ON"),
    knob("ext_volume", "EXTERNAL INPUT VOLUME"),
    switch("ext_on", "EXTERNAL INPUT ON"),
    knob("osc2_volume", "OSCILLATOR-2 VOLUME"),
    switch("osc2_on", "OSCILLATOR-2 ON"),
    knob("noise_volume", "NOISE VOLUME"),
    switch("noise_on", "NOISE ON"),
    knob("osc3_volume", "OSCILLATOR-3 VOLUME"),
    switch("osc3_on", "OSCILLATOR-3 ON"),
    switch("noise_type", "NOISE (WHITE or PINK)"),
    // FILTER POLES and FILTER MODE (R-POLES): MODE's sweep stops at LO, BAND, STEEP HIGH and
    // NOTCH only, as its selector does (`normalized_for`).
    stepped("filter_poles", "FILTER POLES"),
    stepped("filter_mode", "FILTER MODE (LO, BAND, STEEP HIGH or NOTCH)"),
    switch("filter_mod", "FILTER MODULATION"),
    switch("keyboard_control_1", "KEYBOARD CONTROL 1"),
    switch("keyboard_control_2", "KEYBOARD CONTROL 2"),
    // MODIFIERS.
    knob("cutoff", "CUTOFF FREQUENCY"),
    knob("emphasis", "EMPHASIS"),
    knob("contour_amount", "AMOUNT OF CONTOUR"),
    knob("filter_attack", "FILTER CONTOUR ATTACK TIME"),
    knob("filter_decay", "FILTER CONTOUR DECAY TIME"),
    knob("filter_sustain", "FILTER CONTOUR SUSTAIN LEVEL"),
    knob("loudness_attack", "LOUDNESS CONTOUR ATTACK TIME"),
    knob("loudness_decay", "LOUDNESS CONTOUR DECAY TIME"),
    knob("loudness_sustain", "LOUDNESS CONTOUR SUSTAIN LEVEL"),
    // OUTPUT.
    knob("volume", "MAIN OUTPUT VOLUME"),
    switch("main_output", "MAIN OUTPUT"),
    switch("a440", "A-440"),
    knob("feedback", "FEEDBACK"),
    // The left hand controller's switches (not its wheels).
    switch("glide_on", "GLIDE (switch)"),
    switch("decay_on", "DECAY (switch)"),
    stepped("note_priority", "NOTE PRIORITY"),
    switch("retrigger", "RETRIGGER"),
    // The GLOBAL page (VOICES, STEREO and OUTPUT), and LOCK.
    switch("poly", "POLY"),
    switch("unison", "UNISON"),
    stepped("voices", "VOICES"),
    knob("entropy", "ENTROPY"),
    knob("spread", "WIDTH"),
    knob("inner", "INNER"),
    // (Two choices, EVEN and CENTER: a switch, as NOISE's WHITE or PINK.)
    switch("placement", "SCATTER PLACEMENT"),
    switch("doubled", "DOUBLE"),
    knob("double", "DETUNE (DOUBLE)"),
    knob("drive", "DRIVE"),
    knob("level", "LEVEL"),
    switch("auto_gain", "AUTO GAIN"),
    switch("lock", "LOCK (oscillators identical)"),
    // The arpeggiator (R-ARP).
    switch("arp_on", "ARP ON"),
    switch("arp_latch", "ARP HOLD"),
    stepped("arp_direction", "ARP DIRECTION"),
    stepped("arp_rate", "ARP RATE"),
    stepped("arp_octaves", "ARP OCTAVES"),
    knob("arp_gate", "ARP GATE"),
    knob("arp_swing", "ARP SWING"),
    // Each note's own expression's reach (R-MPE).
    knob("pressure_mod", "PRESSURE TO MODULATION"),
    knob("pressure_filter", "PRESSURE TO FILTER"),
    knob("timbre_mod", "TIMBRE TO MODULATION"),
    knob("timbre_filter", "TIMBRE TO FILTER"),
    // The velocity's response (R-VELOCITY).
    knob("velocity_curve", "VELOCITY CURVE"),
    knob("velocity_floor", "VELOCITY FLOOR"),
    // Each oscillator's waveform (R-CONTINUOUS-WAVE).
    switch(
        "osc1_shape_mode",
        "OSCILLATOR-1 WAVEFORM (STEPPED or CONTINUOUS)",
    ),
    knob("osc1_shape", "OSCILLATOR-1 SHAPE"),
    switch("osc1_shape_mod", "OSCILLATOR-1 MOD"),
    stepped("osc1_shape_mod_dir", "OSCILLATOR-1 MOD DIRECTION"),
    switch(
        "osc2_shape_mode",
        "OSCILLATOR-2 WAVEFORM (STEPPED or CONTINUOUS)",
    ),
    knob("osc2_shape", "OSCILLATOR-2 SHAPE"),
    switch("osc2_shape_mod", "OSCILLATOR-2 MOD"),
    stepped("osc2_shape_mod_dir", "OSCILLATOR-2 MOD DIRECTION"),
    switch(
        "osc3_shape_mode",
        "OSCILLATOR-3 WAVEFORM (STEPPED or CONTINUOUS)",
    ),
    knob("osc3_shape", "OSCILLATOR-3 SHAPE"),
    switch("osc3_shape_mod", "OSCILLATOR-3 MOD"),
    stepped("osc3_shape_mod_dir", "OSCILLATOR-3 MOD DIRECTION"),
    // The LFOs (decisions.md R-LFO).
    stepped("lfo1_waveform", "LFO 1 WAVEFORM"),
    switch("lfo1_shape_mode", "LFO 1 WAVEFORM (STEPPED or CONTINUOUS)"),
    knob("lfo1_shape", "LFO 1 SHAPE"),
    knob("lfo1_rate", "LFO 1 RATE"),
    switch("lfo1_timed", "LFO 1 TIMED"),
    stepped("lfo1_division", "LFO 1 DIVISION"),
    knob("lfo1_phase", "LFO 1 PHASE"),
    knob("lfo1_fade", "LFO 1 FADE"),
    stepped("lfo1_mode", "LFO 1 MODE"),
    switch("lfo1_mono", "LFO 1 MONO"),
    stepped("lfo2_waveform", "LFO 2 WAVEFORM"),
    switch("lfo2_shape_mode", "LFO 2 WAVEFORM (STEPPED or CONTINUOUS)"),
    knob("lfo2_shape", "LFO 2 SHAPE"),
    knob("lfo2_rate", "LFO 2 RATE"),
    switch("lfo2_timed", "LFO 2 TIMED"),
    stepped("lfo2_division", "LFO 2 DIVISION"),
    knob("lfo2_phase", "LFO 2 PHASE"),
    knob("lfo2_fade", "LFO 2 FADE"),
    stepped("lfo2_mode", "LFO 2 MODE"),
    switch("lfo2_mono", "LFO 2 MONO"),
    stepped("lfo3_waveform", "LFO 3 WAVEFORM"),
    switch("lfo3_shape_mode", "LFO 3 WAVEFORM (STEPPED or CONTINUOUS)"),
    knob("lfo3_shape", "LFO 3 SHAPE"),
    knob("lfo3_rate", "LFO 3 RATE"),
    switch("lfo3_timed", "LFO 3 TIMED"),
    stepped("lfo3_division", "LFO 3 DIVISION"),
    knob("lfo3_phase", "LFO 3 PHASE"),
    knob("lfo3_fade", "LFO 3 FADE"),
    stepped("lfo3_mode", "LFO 3 MODE"),
    switch("lfo3_mono", "LFO 3 MONO"),
    stepped("lfo4_waveform", "LFO 4 WAVEFORM"),
    switch("lfo4_shape_mode", "LFO 4 WAVEFORM (STEPPED or CONTINUOUS)"),
    knob("lfo4_shape", "LFO 4 SHAPE"),
    knob("lfo4_rate", "LFO 4 RATE"),
    switch("lfo4_timed", "LFO 4 TIMED"),
    stepped("lfo4_division", "LFO 4 DIVISION"),
    knob("lfo4_phase", "LFO 4 PHASE"),
    knob("lfo4_fade", "LFO 4 FADE"),
    stepped("lfo4_mode", "LFO 4 MODE"),
    switch("lfo4_mono", "LFO 4 MONO"),
    // The macros (decisions.md R-MACROS).
    knob("macro1", "MACRO 1"),
    knob("macro2", "MACRO 2"),
    knob("macro3", "MACRO 3"),
    knob("macro4", "MACRO 4"),
    knob("macro5", "MACRO 5"),
    knob("macro6", "MACRO 6"),
    // Each LFO's POLARITY, REVERSE and ON (decisions.md R-LFO's corrections).
    switch("lfo1_polarity", "LFO 1 POLARITY (BI or UNI)"),
    switch("lfo1_reverse", "LFO 1 REVERSE"),
    switch("lfo1_on", "LFO 1 ON"),
    switch("lfo2_polarity", "LFO 2 POLARITY (BI or UNI)"),
    switch("lfo2_reverse", "LFO 2 REVERSE"),
    switch("lfo2_on", "LFO 2 ON"),
    switch("lfo3_polarity", "LFO 3 POLARITY (BI or UNI)"),
    switch("lfo3_reverse", "LFO 3 REVERSE"),
    switch("lfo3_on", "LFO 3 ON"),
    switch("lfo4_polarity", "LFO 4 POLARITY (BI or UNI)"),
    switch("lfo4_reverse", "LFO 4 REVERSE"),
    switch("lfo4_on", "LFO 4 ON"),
    // Each LFO's LEVEL, its OUT jack's (decisions.md R-LFO's addendum).
    knob("lfo1_level", "LFO 1 LEVEL"),
    knob("lfo2_level", "LFO 2 LEVEL"),
    knob("lfo3_level", "LFO 3 LEVEL"),
    knob("lfo4_level", "LFO 4 LEVEL"),
    Learnable {
        id: "previous_preset",
        label: "PREVIOUS PRESET",
        kind: Kind::Action,
    },
    Learnable {
        id: "next_preset",
        label: "NEXT PRESET",
        kind: Kind::Action,
    },
    // The standalone's effects' pins, learned by the pin (decisions.md R-FX-PARAMS): the
    // controller follows the pin, not its slot. Last, and listed only where there are effects
    // ([`listed`]), so that no other's place in the list moves.
    knob("fx1", "EFFECT PIN 1"),
    knob("fx2", "EFFECT PIN 2"),
    knob("fx3", "EFFECT PIN 3"),
    knob("fx4", "EFFECT PIN 4"),
    knob("fx5", "EFFECT PIN 5"),
    knob("fx6", "EFFECT PIN 6"),
    knob("fx7", "EFFECT PIN 7"),
    knob("fx8", "EFFECT PIN 8"),
    knob("fx9", "EFFECT PIN 9"),
    knob("fx10", "EFFECT PIN 10"),
    knob("fx11", "EFFECT PIN 11"),
    knob("fx12", "EFFECT PIN 12"),
    knob("fx13", "EFFECT PIN 13"),
    knob("fx14", "EFFECT PIN 14"),
    knob("fx15", "EFFECT PIN 15"),
    knob("fx16", "EFFECT PIN 16"),
];

/// The effects' pin (0 to 15) a learnable id is, if it is one (`fx1` to `fx16`: R-FX-PARAMS).
pub fn fx_pin(id: &str) -> Option<usize> {
    let k: usize = id.strip_prefix("fx")?.parse().ok()?;
    (1..=crate::lfo::FX_PINS).contains(&k).then(|| k - 1)
}

/// How many of [`LEARNABLE`] the learning list shows: every one with the standalone's effects,
/// else all but the effects' pins, which are last.
pub fn listed() -> usize {
    if crate::fx_view::installed().is_some() {
        LEARNABLE.len()
    } else {
        LEARNABLE.len() - crate::lfo::FX_PINS
    }
}

/// Whether learnable `i` is an effects' pin: the standalone's alone (its learning list leaves
/// them out where there are no effects).
pub fn is_fx_pin(i: usize) -> bool {
    LEARNABLE.get(i).is_some_and(|l| fx_pin(l.id).is_some())
}

/// Whether learnable `i` may learn a note: a preset action (R-PRESET-KEYS), or a switch, which
/// a learned note turns over at each note-on (R-MIDI-MAP).
pub fn takes_note(i: usize) -> bool {
    action(i).is_some() || LEARNABLE.get(i).is_some_and(|l| l.kind == Kind::Switch)
}

pub fn action(i: usize) -> Option<i32> {
    match LEARNABLE.get(i)?.id {
        "previous_preset" => Some(-1),
        "next_preset" => Some(1),
        _ => None,
    }
}

/// The learnable parameter of this id.
pub fn index(id: &str) -> Option<usize> {
    LEARNABLE.iter().position(|l| l.id == id)
}

/// A learning target outside the sound assignments: it changes the computer's wheel source.
pub const MOD_WHEEL: usize = LEARNABLE.len();

/// Why a control change is not learned, if it is not: what it is.
pub fn reserved(cc: u8) -> Option<&'static str> {
    Some(match cc {
        0 | 32 => "bank select",
        6 | 38 => "data entry",
        96 | 97 => "data increment and decrement",
        98..=101 => "an RPN or NRPN number",
        120 => "all sound off",
        121 => "reset all controllers",
        122 => "local control",
        123 => "all notes off",
        124..=127 => "a channel mode message",
        128..=u8::MAX => "not a control change",
        _ => return None,
    })
}

/// The reserved controllers, as the editor lists them.
pub const RESERVED_TEXT: &str = "NOT LEARNED: CC 0 AND 32 (BANK), 6, 38 AND \
                                 96-101 (DATA ENTRY, RPN, NRPN), 120-127 (CHANNEL MODE)";

/// A controller: a MIDI channel (0 to 15; shown 1 to 16) and a control change (0 to 127).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Cc {
    pub channel: u8,
    pub cc: u8,
}

impl Cc {
    /// As the editor shows it: `CH 1 · CC 74`.
    pub fn text(self) -> String {
        format!("CH {} · CC {}", u32::from(self.channel) + 1, self.cc)
    }

    fn slot(self) -> usize {
        usize::from(self.channel & 15) * 128 + usize::from(self.cc & 127)
    }

    fn of_slot(slot: usize) -> Cc {
        Cc {
            channel: (slot / 128) as u8,
            cc: (slot % 128) as u8,
        }
    }

    fn learnable(self) -> bool {
        self.channel < 16 && self.cc < 128 && reserved(self.cc).is_none()
    }
}

/// A learned input. Notes are available for actions and switches alone ([`takes_note`]); a
/// switch's note is saved with the project beside the controllers (`"note"` in place of
/// `"cc"`), which an older CA-72 leaves out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Binding {
    Cc(Cc),
    Note { channel: u8, note: u8 },
}

impl From<Cc> for Binding {
    fn from(cc: Cc) -> Self {
        Self::Cc(cc)
    }
}

impl Binding {
    pub fn text(self) -> String {
        match self {
            Self::Cc(cc) => cc.text(),
            Self::Note { channel, note } => format!("CH {} · NOTE {note}", u32::from(channel) + 1),
        }
    }

    fn slot(self) -> usize {
        match self {
            Self::Cc(cc) => cc.slot(),
            Self::Note { channel, note } => {
                2048 + usize::from(channel & 15) * 128 + usize::from(note & 127)
            }
        }
    }

    fn of_slot(slot: usize) -> Self {
        if slot < 2048 {
            Self::Cc(Cc::of_slot(slot))
        } else {
            Self::Note {
                channel: ((slot - 2048) / 128) as u8,
                note: (slot % 128) as u8,
            }
        }
    }

    fn learnable(self) -> bool {
        match self {
            Self::Cc(cc) => cc.learnable(),
            Self::Note { channel, note } => channel < 16 && note < 128,
        }
    }
}

/// What the audio thread does with a learnable control change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Incoming {
    /// Assigned to no parameter: nothing.
    Unassigned,
    /// Caught for the parameter being learned: nothing else (the sound is not changed by it).
    Caught,
    /// Assigned: set the learnable parameter of this index.
    Assigned(usize),
}

/// What an assignment changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Assigned {
    /// The parameter and its controller now.
    pub param: usize,
    pub cc: Binding,
    /// The controller it had before (replaced), if another.
    pub replaced: Option<Binding>,
    /// The parameter the controller was taken from, if another had it.
    pub displaced: Option<usize>,
}

/// A slot of [`MidiMap::routes`] holds the learnable's index plus one; 0 is none.
const NONE: u8 = 0;

/// The learning word ([`MidiMap::armed`]): the parameter armed (index + 1; 0 none) in its low
/// byte, the arming's number above it.
const TARGET_BITS: u32 = 8;
/// A caught controller ([`MidiMap::caught`]): valid, the arming's number, the parameter, the
/// channel and the control change.
const CAUGHT: u64 = 1 << 63;

/// One instance's MIDI assignments and its learning (decisions.md R34). Saved with the plug-in's
/// state under `midi_map` ([`Saved`]).
pub struct MidiMap {
    /// By channel and controller (channel × 128 + CC): the learnable parameter it sets, its
    /// index plus one ([`NONE`] none). The audio thread only loads these; every change is made
    /// under `writer`, which keeps at most one controller a target. The second half is notes,
    /// by the same channel × 128 + note indexing, for preset actions alone.
    routes: [AtomicU8; 2 * 16 * 128],
    /// Computer assignments are written only by the editor, never by processing or state recall.
    pub(crate) settings_file: Option<std::path::PathBuf>,
    writer: Mutex<()>,
    pub(crate) preset_list: Mutex<Option<Vec<String>>>,
    /// The parameter being learned and the arming's number ([`TARGET_BITS`]).
    armed: AtomicU32,
    /// Whether the arming takes a note for a switch (the MIDI map mode's: R-MIDI-MAP); set
    /// before `armed`, so the audio thread reads it with the arming it belongs to.
    notes: std::sync::atomic::AtomicBool,
    /// What the audio thread caught for the arming ([`CAUGHT`]), for [`MidiMap::poll`].
    caught: AtomicU64,
    /// The last reserved control change seen while learning, for the editor to explain: the
    /// arming's number above, the control change + 1 in the low byte (0 none).
    refused: AtomicU32,
    /// Counts every change of the assignments (the editor's lists follow it).
    version: AtomicU32,
    /// The computer's wheel source, with a generation above its low byte. No file access or
    /// lock on the audio thread; switching away and back between callbacks still clears it.
    wheel_source: AtomicU64,
}

impl Default for MidiMap {
    fn default() -> Self {
        MidiMap {
            routes: std::array::from_fn(|_| AtomicU8::new(NONE)),
            settings_file: None,
            writer: Mutex::new(()),
            preset_list: Mutex::new(None),
            armed: AtomicU32::new(0),
            notes: std::sync::atomic::AtomicBool::new(false),
            caught: AtomicU64::new(0),
            refused: AtomicU32::new(0),
            version: AtomicU32::new(0),
            wheel_source: AtomicU64::new(u64::from(Source::DEFAULT.code())),
        }
    }
}

impl std::fmt::Debug for MidiMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MidiMap")
            .field("assignments", &self.saved().0)
            .field("armed", &self.armed())
            .finish()
    }
}

impl MidiMap {
    /// A new instance takes the computer's preset actions. Project recall replaces only
    /// parameter assignments, so it cannot undo these.
    pub fn for_computer() -> Self {
        Self::for_settings(crate::settings::file())
    }

    /// [`MidiMap::for_computer`], from the settings file at `path`.
    pub(crate) fn for_settings(path: std::path::PathBuf) -> Self {
        let mut map = Self::default();
        // The computer's wheel source first: an action learned on a CC the wheel has freed
        // (CC 1 among them) is kept, and one on the wheel's CC is not taken.
        map.set_wheel_source(crate::settings::Settings::read(&path).mod_wheel_source);
        for (i, binding) in crate::settings::preset_actions(&path) {
            map.assign(i, binding);
        }
        map.settings_file = Some(path);
        map
    }

    pub fn keep_actions(&self, changed: &[usize]) -> Result<(), String> {
        if !changed.iter().any(|&i| action(i).is_some()) {
            return Ok(());
        }
        if let Some(path) = &self.settings_file {
            crate::settings::keep_preset_actions(path, &self.bindings(), changed, None)?;
        }
        Ok(())
    }

    /// Persist the explicit target and release a displaced input only if the file still has
    /// that input. Another instance may have moved the displaced action since we read it.
    pub fn keep_assignment(&self, assigned: &Assigned) -> Result<(), String> {
        let released = assigned
            .displaced
            .filter(|&i| action(i).is_some())
            .map(|_| assigned.cc);
        if action(assigned.param).is_none() && released.is_none() {
            return Ok(());
        }
        if let Some(path) = &self.settings_file {
            crate::settings::keep_preset_actions(
                path,
                &self.bindings(),
                &[assigned.param],
                released,
            )?;
        }
        Ok(())
    }

    pub fn wheel_source(&self) -> Source {
        Source::of_code(self.wheel_word() as u8)
    }

    pub fn wheel_word(&self) -> u64 {
        self.wheel_source.load(Ordering::Acquire)
    }

    /// Chosen in Settings or learned from the wheel. Every assignment of that CC, on every
    /// channel, is removed: the wheel follows it on any channel. Never saved in `midi_map`.
    pub fn set_wheel_source(&self, source: Source) {
        let _w = self.lock();
        let source = Source::of_code(source.code());
        if source == self.wheel_source() {
            return;
        }
        if let Source::Cc(cc) = source {
            for channel in 0..16 {
                self.routes[channel * 128 + usize::from(cc)].store(NONE, Ordering::Release);
            }
        }
        let word = self.wheel_word();
        self.wheel_source.store(
            (word.wrapping_add(256) & !255) | u64::from(source.code()),
            Ordering::Release,
        );
        self.version.fetch_add(1, Ordering::AcqRel);
    }

    pub fn reserved(&self, cc: u8) -> Option<&'static str> {
        if self.wheel_source() == Source::Cc(cc) {
            Some("the modulation wheel source")
        } else {
            reserved(cc)
        }
    }

    pub fn reserved_text(&self) -> String {
        match self.wheel_source() {
            Source::Cc(cc) => format!("{RESERVED_TEXT} · CC {cc} (MOD WHEEL SOURCE, ALL CHANNELS)"),
            source => format!("{RESERVED_TEXT} · MOD WHEEL SOURCE: {}", source.text()),
        }
    }

    // ---- The audio thread's: loads, a compare-and-swap and stores, nothing else.

    /// A learnable control change `cc` on `channel` (0 to 15), on the audio thread: caught for
    /// the parameter being learned (once), else its assignment.
    pub fn incoming(&self, channel: u8, cc: u8) -> Incoming {
        self.input(Binding::Cc(Cc { channel, cc }), true)
    }

    pub fn incoming_note(&self, channel: u8, note: u8, on: bool) -> Incoming {
        self.input(Binding::Note { channel, note }, on)
    }

    fn input(&self, c: Binding, capture: bool) -> Incoming {
        if !c.learnable() {
            return Incoming::Unassigned;
        }
        // (Twice at most: the editor may arm again or cancel between the load and the swap.)
        for _ in 0..2 {
            let a = self.armed.load(Ordering::Acquire);
            let target = a & ((1 << TARGET_BITS) - 1);
            if target == 0 || !capture || (matches!(c, Binding::Note { .. }) && !self.notes_for(a))
            {
                break;
            }
            // Use this arming's target: the editor may have moved learning from the wheel
            // to a sound control since the event arrived. A reserved CC leaves it waiting.
            if target as usize - 1 != MOD_WHEEL && self.reserved_input(c) {
                return Incoming::Unassigned;
            }
            let disarmed = a & !((1 << TARGET_BITS) - 1);
            if self
                .armed
                .compare_exchange(a, disarmed, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                let word = CAUGHT
                    | (u64::from(a >> TARGET_BITS) << 24)
                    | (u64::from(target - 1) << 16)
                    | c.slot() as u64;
                self.caught.store(word, Ordering::Release);
                return Incoming::Caught;
            }
        }
        if self.reserved_input(c) {
            return Incoming::Unassigned;
        }
        match self.routes[c.slot()].load(Ordering::Acquire) {
            NONE => Incoming::Unassigned,
            k => Incoming::Assigned(usize::from(k) - 1),
        }
    }

    /// Whether the arming `a` takes a note: a preset action's always, a switch's while the map
    /// mode arms it ([`MidiMap::arm_with_notes`]).
    fn notes_for(&self, a: u32) -> bool {
        let Some(i) = ((a & ((1 << TARGET_BITS) - 1)) as usize).checked_sub(1) else {
            return false;
        };
        action(i).is_some() || (self.notes.load(Ordering::Acquire) && takes_note(i))
    }

    /// A reserved control change (the wheel's chosen CC among them); a note never is.
    fn reserved_input(&self, input: Binding) -> bool {
        matches!(input, Binding::Cc(c) if self.reserved(c.cc).is_some())
    }

    /// Actions learn a CC's on value; a parameter may learn any value as before.
    pub fn incoming_cc(&self, channel: u8, cc: u8, on: bool) -> Incoming {
        let capture = self.armed().is_none_or(|i| action(i).is_none()) || on;
        self.input(Binding::Cc(Cc { channel, cc }), capture)
    }

    /// A reserved control change seen, on the audio thread: while learning, kept for the editor
    /// to say why it was not learned.
    pub fn refuse(&self, cc: u8) {
        let a = self.armed.load(Ordering::Acquire);
        if a & ((1 << TARGET_BITS) - 1) != 0 {
            let word = (a & !((1 << TARGET_BITS) - 1)) | (u32::from(cc.min(127)) + 1);
            self.refused.store(word, Ordering::Release);
        }
    }

    // ---- The editor's and the host's (state): never on the audio thread.

    /// The learnable parameter set by this controller, if any.
    pub fn target(&self, cc: Cc) -> Option<usize> {
        self.binding_target(cc.into())
    }

    pub fn binding_target(&self, input: Binding) -> Option<usize> {
        match self.routes[input.slot()].load(Ordering::Acquire) {
            NONE => None,
            k => Some(usize::from(k) - 1),
        }
    }

    /// Every learnable parameter's controller, by its index.
    pub fn bindings(&self) -> [Option<Binding>; LEARNABLE.len()] {
        let mut out = [None; LEARNABLE.len()];
        for (slot, r) in self.routes.iter().enumerate() {
            let k = r.load(Ordering::Acquire);
            if let Some(o) = usize::from(k).checked_sub(1).and_then(|i| out.get_mut(i)) {
                *o = Some(Binding::of_slot(slot));
            }
        }
        out
    }

    /// The controller assigned to learnable parameter `param`, if any.
    pub fn assignment(&self, param: usize) -> Option<Cc> {
        self.binding(param).and_then(|b| match b {
            Binding::Cc(cc) => Some(cc),
            _ => None,
        })
    }

    pub fn assignments(&self) -> [Option<Cc>; LEARNABLE.len()] {
        self.bindings().map(|b| {
            b.and_then(|b| match b {
                Binding::Cc(cc) => Some(cc),
                _ => None,
            })
        })
    }

    pub fn binding(&self, param: usize) -> Option<Binding> {
        self.bindings().get(param).copied().flatten()
    }

    /// How often the assignments have changed (the editor follows it).
    pub fn version(&self) -> u32 {
        self.version.load(Ordering::Acquire)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ()> {
        self.writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The slot of `param`'s controller, under the writer's lock.
    fn slot_of(&self, param: usize) -> Option<usize> {
        let want = u8::try_from(param + 1).ok()?;
        self.routes
            .iter()
            .position(|r| r.load(Ordering::Acquire) == want)
    }

    /// `cc` assigned to learnable parameter `param`: its controller before is let go (relearning
    /// replaces it), and a parameter that had `cc` loses it (it moves). None if either is not
    /// learnable.
    pub fn assign(&self, param: usize, cc: impl Into<Binding>) -> Option<Assigned> {
        let cc = cc.into();
        if !cc.learnable() {
            return None;
        }
        if param == MOD_WHEEL {
            let Binding::Cc(c) = cc else {
                return None;
            };
            self.set_wheel_source(Source::Cc(c.cc));
            return Some(Assigned {
                param,
                cc,
                replaced: None,
                displaced: None,
            });
        }
        if param >= LEARNABLE.len() || (matches!(cc, Binding::Note { .. }) && !takes_note(param)) {
            return None;
        }
        let _w = self.lock();
        if matches!(cc, Binding::Cc(c) if self.reserved(c.cc).is_some()) {
            return None;
        }
        let before = self.slot_of(param);
        if let Some(s) = before {
            self.routes[s].store(NONE, Ordering::Release);
        }
        let had = self.routes[cc.slot()].swap(param as u8 + 1, Ordering::AcqRel);
        self.version.fetch_add(1, Ordering::AcqRel);
        Some(Assigned {
            param,
            cc,
            replaced: before.map(Binding::of_slot).filter(|b| *b != cc),
            displaced: usize::from(had).checked_sub(1).filter(|&d| d != param),
        })
    }

    /// Learnable parameter `param`'s controller removed: which it was.
    pub fn remove(&self, param: usize) -> Option<Binding> {
        let _w = self.lock();
        let s = self.slot_of(param)?;
        self.routes[s].store(NONE, Ordering::Release);
        self.version.fetch_add(1, Ordering::AcqRel);
        Some(Binding::of_slot(s))
    }

    /// The assignments as saved: by learnable index, in its order.
    pub fn saved(&self) -> Saved {
        let _w = self.lock();
        Saved(
            self.bindings()
                .iter()
                .enumerate()
                .filter(|(i, _)| action(*i).is_none())
                .filter_map(|(i, c)| c.map(|c| (i, c)))
                .collect(),
        )
    }

    /// Every assignment replaced by `saved`'s (an empty table: none).
    pub fn load(&self, saved: &Saved) {
        let _w = self.lock();
        for r in &self.routes {
            let target = r.load(Ordering::Acquire);
            if target != NONE && action(usize::from(target) - 1).is_none() {
                r.store(NONE, Ordering::Release);
            }
        }
        for &(i, c) in &saved.0 {
            if i < LEARNABLE.len()
                && action(i).is_none()
                && c.learnable()
                && (takes_note(i) || matches!(c, Binding::Cc(_)))
                && !self.reserved_input(c)
                && self.binding_target(c).is_none()
            {
                self.routes[c.slot()].store(i as u8 + 1, Ordering::Release);
            }
        }
        self.version.fetch_add(1, Ordering::AcqRel);
    }

    /// Learnable parameter `param` armed: the next learnable control change is its controller.
    /// Another armed before is no longer (arming another moves the learning).
    pub fn arm(&self, param: usize) {
        if param > MOD_WHEEL {
            return;
        }
        self.notes.store(false, Ordering::Release);
        self.rearm(param as u32 + 1);
    }

    /// The same, a note-on taken as well if `param` is a switch (the standalone's MIDI map mode:
    /// R-MIDI-MAP). A knob's arming is [`MidiMap::arm`]'s.
    pub fn arm_with_notes(&self, param: usize) {
        if param > MOD_WHEEL {
            return;
        }
        self.notes.store(takes_note(param), Ordering::Release);
        self.rearm(param as u32 + 1);
    }

    /// Learning cancelled: the assignments as they were.
    pub fn cancel(&self) {
        self.rearm(0);
    }

    fn rearm(&self, target: u32) {
        let n = (self.armed.load(Ordering::Acquire) >> TARGET_BITS).wrapping_add(1)
            & (u32::MAX >> TARGET_BITS);
        self.armed
            .store((n << TARGET_BITS) | target, Ordering::Release);
    }

    /// The parameter being learned, if any.
    pub fn armed(&self) -> Option<usize> {
        let target = self.armed.load(Ordering::Acquire) & ((1 << TARGET_BITS) - 1);
        (target as usize).checked_sub(1)
    }

    /// Whether the parameter being learned takes a note: a preset action, or a switch armed by
    /// the MIDI map mode ([`MidiMap::arm_with_notes`]).
    pub fn notes_armed(&self) -> bool {
        self.notes_for(self.armed.load(Ordering::Acquire))
    }

    /// The arming's number.
    fn arming(&self) -> u32 {
        self.armed.load(Ordering::Acquire) >> TARGET_BITS
    }

    /// What the audio thread caught for the present arming, assigned (none: nothing caught, or
    /// caught for an arming since replaced or cancelled).
    pub fn poll(&self) -> Option<Assigned> {
        let word = self.caught.swap(0, Ordering::AcqRel);
        if word & CAUGHT == 0
            || ((word >> 24) & u64::from(u32::MAX >> TARGET_BITS)) as u32 != self.arming()
        {
            return None;
        }
        let param = ((word >> 16) & 0xff) as usize;
        let cc = Binding::of_slot((word & 0xfff) as usize);
        self.assign(param, cc)
    }

    /// The last reserved control change seen while this arming waits, if any.
    pub fn refused(&self) -> Option<u8> {
        let word = self.refused.load(Ordering::Acquire);
        let cc = word & 0xff;
        (cc != 0 && word >> TARGET_BITS == self.arming()).then(|| (cc - 1) as u8)
    }
}

/// The assignments as the plug-in's state holds them (`midi_map`): learnable index and controller,
/// in [`LEARNABLE`]'s order, at most one a parameter and one a controller. As JSON:
/// `{"version":1,"assignments":[{"param":"cutoff","channel":1,"cc":74}]}`, the channel 1 to 16 as
/// shown; a switch's note `{"param":"arp_on","channel":1,"note":36}` (R-MIDI-MAP), which a CA-72
/// before it leaves out. Read leniently ([`Saved::from_json`]); an armed learning is never part
/// of it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Saved(pub Vec<(usize, Binding)>);

/// The table's format.
pub const VERSION: u64 = 1;

impl Saved {
    /// A table as saved, checked: anything but an object of version 1 is an empty table; an
    /// assignment of a parameter not learnable here, a channel outside 1 to 16 or a controller
    /// outside 0 to 127 or reserved, or of a parameter or a controller already assigned above it,
    /// is left out.
    pub fn from_json(v: &serde_json::Value) -> Saved {
        let mut out: Vec<(usize, Binding)> = Vec::new();
        if v.get("version").and_then(serde_json::Value::as_u64) != Some(VERSION) {
            return Saved(out);
        }
        let Some(list) = v.get("assignments").and_then(serde_json::Value::as_array) else {
            return Saved(out);
        };
        for a in list {
            // (An LFO's RISE, as 0.3.0's beta saved it, is its FADE: R-LFO-FADE.)
            let param = a
                .get("param")
                .and_then(serde_json::Value::as_str)
                .and_then(|id| index(id).or_else(|| index(crate::lfo::fade_id(id)?)));
            let channel = a.get("channel").and_then(serde_json::Value::as_u64);
            let cc = a.get("cc").and_then(serde_json::Value::as_u64);
            // (A switch's note, R-MIDI-MAP: an entry with a controller as well is a controller's.)
            let note = a.get("note").and_then(serde_json::Value::as_u64);
            let (Some(param), Some(channel @ 1..=16)) = (param, channel) else {
                continue;
            };
            let channel = (channel - 1) as u8;
            let c: Binding = match (cc, note) {
                (Some(cc @ 0..=127), _) => Cc {
                    channel,
                    cc: cc as u8,
                }
                .into(),
                (None, Some(note @ 0..=127)) if takes_note(param) => Binding::Note {
                    channel,
                    note: note as u8,
                },
                _ => continue,
            };
            if action(param).is_none()
                && c.learnable()
                && !out.iter().any(|&(p, o)| p == param || o == c)
            {
                out.push((param, c));
            }
        }
        out.sort_by_key(|&(p, _)| p);
        Saved(out)
    }

    /// The table from the state's text: none (an empty table) if it is not JSON.
    pub fn from_text(text: &str) -> Saved {
        serde_json::from_str::<serde_json::Value>(text)
            .map(|v| Saved::from_json(&v))
            .unwrap_or_default()
    }
}

impl Serialize for Saved {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        struct Entry(usize, Binding);
        impl Serialize for Entry {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                let mut m = s.serialize_map(Some(3))?;
                m.serialize_entry("param", LEARNABLE[self.0].id)?;
                match self.1 {
                    Binding::Cc(c) => {
                        m.serialize_entry("channel", &(u32::from(c.channel) + 1))?;
                        m.serialize_entry("cc", &c.cc)?;
                    }
                    Binding::Note { channel, note } => {
                        m.serialize_entry("channel", &(u32::from(channel) + 1))?;
                        m.serialize_entry("note", &note)?;
                    }
                }
                m.end()
            }
        }
        struct List<'a>(&'a [(usize, Binding)]);
        impl Serialize for List<'_> {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                let mut seq = s.serialize_seq(Some(self.0.len()))?;
                for &(i, c) in self.0 {
                    seq.serialize_element(&Entry(i, c))?;
                }
                seq.end()
            }
        }
        let mut m = s.serialize_map(Some(2))?;
        m.serialize_entry("version", &VERSION)?;
        m.serialize_entry("assignments", &List(&self.0))?;
        m.end()
    }
}

/// Any JSON reads ([`Saved::from_json`]): a table that is not understood is an empty one, never
/// an error, so a state is never half loaded over the assignments an instance had.
impl<'de> Deserialize<'de> for Saved {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Saved::from_json(&serde_json::Value::deserialize(d)?))
    }
}

/// The table in the plug-in's state (nih-plug's persistent fields): saved as [`Saved`], loaded
/// over every assignment the instance had.
impl<'a> nih_plug::params::persist::PersistentField<'a, Saved> for std::sync::Arc<MidiMap> {
    fn set(&self, new_value: Saved) {
        self.load(&new_value);
    }

    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&Saved) -> R,
    {
        f(&self.saved())
    }
}

/// The key of the table in the plug-in's state.
pub const STATE_KEY: &str = "midi_map";

/// A plug-in state's table made good before it is loaded (`Plugin::filter_state`): a state
/// without one (saved before MIDI Learn, or by a host's preset of another version) gets an empty
/// table, so that loading it into an instance with assignments leaves none; one that is not
/// JSON, or not understood, is an empty table too; one understood is written back checked.
pub fn filter_state(fields: &mut std::collections::BTreeMap<String, String>) {
    let saved = fields
        .get(STATE_KEY)
        .map(|t| Saved::from_text(t))
        .unwrap_or_default();
    let text = serde_json::to_string(&saved)
        .unwrap_or_else(|_| format!("{{\"version\":{VERSION},\"assignments\":[]}}"));
    fields.insert(STATE_KEY.to_owned(), text);
}

/// A parameter as MIDI Learn sets it, whatever its type.
pub trait Target {
    /// The pointer the host knows it by.
    fn ptr(&self) -> ParamPtr;
    /// The normalized value a 7-bit `value` sets it to: through the parameter's own
    /// normalization, a knob value / 127 of its travel, a stepped parameter the position its
    /// share of 0–127 holds (a switch: off below 64).
    fn normalized_for(&self, value: u8) -> f32;
    /// Whether it is at `normalized` already.
    fn at(&self, normalized: f32) -> bool;
    /// How many positions it has (none: continuous).
    fn positions(&self) -> Option<usize>;
}

impl<P: Param> Target for P {
    fn ptr(&self) -> ParamPtr {
        self.as_ptr()
    }

    fn normalized_for(&self, value: u8) -> f32 {
        let value = value.min(127);
        match self.step_count() {
            None => f32::from(value) / 127.0,
            Some(steps) => {
                let n = steps + 1;
                let position = (usize::from(value) * n / 128).min(steps);
                self.preview_normalized(self.preview_plain(position as f32 / steps.max(1) as f32))
            }
        }
    }

    fn at(&self, normalized: f32) -> bool {
        self.preview_plain(normalized) == self.preview_plain(self.unmodulated_normalized_value())
    }

    fn positions(&self) -> Option<usize> {
        self.step_count().map(|s| s + 1)
    }
}

/// The learnable parameter of index `i` (see [`LEARNABLE`]).
pub fn target(p: &Ca72Params, i: usize) -> Option<&dyn Target> {
    Some(match LEARNABLE.get(i)?.id {
        "tune" => &p.tune,
        "osc_mod" => &p.osc_mod,
        "glide" => &p.glide,
        "glide_range" => &p.glide_range,
        "mod_mix" => &p.mod_mix,
        "osc1_range" => &p.osc1_range,
        "osc2_range" => &p.osc2_range,
        "osc3_range" => &p.osc3_range,
        "osc2_frequency" => &p.osc2_frequency,
        "osc3_frequency" => &p.osc3_frequency,
        "osc1_waveform" => &p.osc1_waveform,
        "osc2_waveform" => &p.osc2_waveform,
        "osc3_waveform" => &p.osc3_waveform,
        "osc3_control" => &p.osc3_control,
        "osc2_timed" => &p.osc2_timed,
        "osc2_division" => &p.osc2_division,
        "osc3_timed" => &p.osc3_timed,
        "osc3_division" => &p.osc3_division,
        "osc1_volume" => &p.osc1_volume,
        "osc1_on" => &p.osc1_on,
        "ext_volume" => &p.ext_volume,
        "ext_on" => &p.ext_on,
        "osc2_volume" => &p.osc2_volume,
        "osc2_on" => &p.osc2_on,
        "noise_volume" => &p.noise_volume,
        "noise_on" => &p.noise_on,
        "osc3_volume" => &p.osc3_volume,
        "osc3_on" => &p.osc3_on,
        "noise_type" => &p.noise_type,
        "filter_mode" => &p.filter_shape,
        "filter_poles" => &p.filter_poles,
        "filter_mod" => &p.filter_mod,
        "keyboard_control_1" => &p.keyboard_control_1,
        "keyboard_control_2" => &p.keyboard_control_2,
        "cutoff" => &p.cutoff,
        "emphasis" => &p.emphasis,
        "contour_amount" => &p.contour_amount,
        "filter_attack" => &p.filter_attack,
        "filter_decay" => &p.filter_decay,
        "filter_sustain" => &p.filter_sustain,
        "loudness_attack" => &p.loudness_attack,
        "loudness_decay" => &p.loudness_decay,
        "loudness_sustain" => &p.loudness_sustain,
        "volume" => &p.volume,
        "main_output" => &p.main_output,
        "a440" => &p.a440,
        "feedback" => &p.feedback,
        "glide_on" => &p.glide_on,
        "decay_on" => &p.decay_on,
        "note_priority" => &p.note_priority,
        "retrigger" => &p.retrigger,
        "poly" => &p.poly,
        "voices" => &p.voices,
        "entropy" => &p.entropy,
        "spread" => &p.spread,
        "inner" => &p.inner,
        "lock" => &p.lock,
        "placement" => &p.placement,
        "unison" => &p.unison,
        "doubled" => &p.doubled,
        "double" => &p.double,
        "drive" => &p.drive,
        "level" => &p.level,
        "auto_gain" => &p.auto_gain,
        "arp_on" => &p.arp_on,
        "arp_latch" => &p.arp_latch,
        "arp_direction" => &p.arp_direction,
        "arp_rate" => &p.arp_rate,
        "arp_octaves" => &p.arp_octaves,
        "arp_gate" => &p.arp_gate,
        "arp_swing" => &p.arp_swing,
        "pressure_mod" => &p.pressure_mod,
        "pressure_filter" => &p.pressure_filter,
        "timbre_mod" => &p.timbre_mod,
        "timbre_filter" => &p.timbre_filter,
        "velocity_curve" => &p.velocity_curve,
        "velocity_floor" => &p.velocity_floor,
        "osc1_shape_mode" => &p.osc1_shape_mode,
        "osc1_shape" => &p.osc1_shape,
        "osc1_shape_mod" => &p.osc1_shape_mod,
        "osc1_shape_mod_dir" => &p.osc1_shape_mod_dir,
        "osc2_shape_mode" => &p.osc2_shape_mode,
        "osc2_shape" => &p.osc2_shape,
        "osc2_shape_mod" => &p.osc2_shape_mod,
        "osc2_shape_mod_dir" => &p.osc2_shape_mod_dir,
        "osc3_shape_mode" => &p.osc3_shape_mode,
        "osc3_shape" => &p.osc3_shape,
        "osc3_shape_mod" => &p.osc3_shape_mod,
        "osc3_shape_mod_dir" => &p.osc3_shape_mod_dir,
        id if crate::lfo::macro_of_id(id).is_some() => {
            p.macro_param(crate::lfo::macro_of_id(id).unwrap_or(0))
        }
        id => return crate::lfo::learn_target(p, id),
    })
}

/// Where learnable `i`'s controller at `value` (0–127) sets `target`, its parameter
/// (normalized): a FREQUENCY knob turned to value / 127 of its travel, its pot where the
/// computer's law puts it then, as the editor's knob (decisions.md R54); anything else
/// as [`Target::normalized_for`].
pub fn normalized_for(p: &Ca72Params, i: usize, target: &dyn Target, value: u8) -> f32 {
    let knob = f64::from(value.min(127)) / 127.0;
    match LEARNABLE.get(i).map(|l| l.id) {
        Some("osc2_frequency") => Pot::Two.wiper_at(p.freq_law(), knob) as f32,
        Some("osc3_frequency") => Pot::Three.wiper_at(p.freq_law(), knob) as f32,
        // FILTER MODE: LO, BAND, STEEP HIGH and NOTCH each a quarter of 0–127, never 0.3's HI,
        // which is not on the panel (decisions.md R-POLES).
        Some("filter_mode") => {
            use crate::params::FilterShape as M;
            let mode = [M::Lo, M::Band, M::High, M::Notch][usize::from(value.min(127)) / 32];
            p.filter_shape.preview_normalized(mode)
        }
        _ => target.normalized_for(value),
    }
}

/// The knob of learnable index `i`, if it is one (each glides: `crate::smooth::KNOBS`).
pub fn knob_param(p: &Ca72Params, i: usize) -> Option<&FloatParam> {
    Some(match LEARNABLE.get(i)?.id {
        "tune" => &p.tune,
        "glide" => &p.glide,
        "mod_mix" => &p.mod_mix,
        "osc2_frequency" => &p.osc2_frequency,
        "osc3_frequency" => &p.osc3_frequency,
        "osc1_volume" => &p.osc1_volume,
        "ext_volume" => &p.ext_volume,
        "osc2_volume" => &p.osc2_volume,
        "noise_volume" => &p.noise_volume,
        "osc3_volume" => &p.osc3_volume,
        "cutoff" => &p.cutoff,
        "emphasis" => &p.emphasis,
        "contour_amount" => &p.contour_amount,
        "filter_attack" => &p.filter_attack,
        "filter_decay" => &p.filter_decay,
        "filter_sustain" => &p.filter_sustain,
        "loudness_attack" => &p.loudness_attack,
        "loudness_decay" => &p.loudness_decay,
        "loudness_sustain" => &p.loudness_sustain,
        "volume" => &p.volume,
        "feedback" => &p.feedback,
        "entropy" => &p.entropy,
        "spread" => &p.spread,
        "inner" => &p.inner,
        "double" => &p.double,
        "drive" => &p.drive,
        "level" => &p.level,
        "arp_gate" => &p.arp_gate,
        "arp_swing" => &p.arp_swing,
        "pressure_mod" => &p.pressure_mod,
        "pressure_filter" => &p.pressure_filter,
        "timbre_mod" => &p.timbre_mod,
        "timbre_filter" => &p.timbre_filter,
        "velocity_curve" => &p.velocity_curve,
        "velocity_floor" => &p.velocity_floor,
        "osc1_shape" => &p.osc1_shape,
        "osc2_shape" => &p.osc2_shape,
        "osc3_shape" => &p.osc3_shape,
        id => match crate::lfo::macro_of_id(id) {
            Some(n) => p.macro_param(n),
            None => return crate::lfo::knob(p, id),
        },
    })
}

/// How long the voices take to follow a knob a learned controller moves, s: a 7-bit controller
/// moves a knob in steps of 1/127 of its travel, which the voices would otherwise take as steps
/// (MAIN OUTPUT VOLUME's click: decisions.md R34 measures them). The parameter itself (the
/// host's, the editor's, the state's) is at the new value at once. Every other change of a
/// knob glides over `smooth::GLIDE` (decisions.md R-KNOB-GLIDE).
pub const DEZIP: f64 = 0.010;

#[cfg(test)]
mod tests {
    use super::*;

    fn cc(channel: u8, cc: u8) -> Cc {
        Cc { channel, cc }
    }

    fn at(id: &str) -> usize {
        index(id).expect("learnable")
    }

    #[test]
    fn actions_learn_notes_and_cc_presses_but_parameters_keep_their_cc_contract() {
        let m = MidiMap::default();
        let next = at("next_preset");
        m.arm(next);
        assert_eq!(m.incoming_note(1, 36, false), Incoming::Unassigned);
        assert_eq!(m.armed(), Some(next));
        assert_eq!(m.incoming_note(1, 36, true), Incoming::Caught);
        let assigned = m.poll().expect("note assigned");
        assert_eq!(
            assigned.cc,
            Binding::Note {
                channel: 1,
                note: 36
            }
        );
        assert_eq!(m.incoming_note(0, 36, true), Incoming::Unassigned);
        assert_eq!(m.incoming_note(1, 36, false), Incoming::Assigned(next));
        assert!(m.saved().0.is_empty(), "actions never enter a project");
        m.load(&Saved::default());
        assert_eq!(
            m.binding(next),
            Some(assigned.cc),
            "state recall keeps computer actions"
        );
        m.arm(at("cutoff"));
        assert_eq!(m.incoming_note(1, 37, true), Incoming::Unassigned);
        assert_eq!(m.armed(), Some(at("cutoff")));
        assert!(m.assign(at("cutoff"), assigned.cc).is_none());
        m.arm(at("previous_preset"));
        assert_eq!(m.incoming_cc(3, 20, false), Incoming::Unassigned);
        assert_eq!(m.incoming_cc(3, 20, true), Incoming::Caught);
        assert_eq!(m.poll().unwrap().cc, cc(3, 20).into());
        let a = m.assign(at("cutoff"), cc(3, 20)).unwrap();
        assert_eq!(
            a.displaced,
            Some(at("previous_preset")),
            "one input, one target"
        );
    }

    /// The list is explicit and whole: every sound parameter a preset sets is learnable but the
    /// MODULATION wheel and MIDI BEND RANGE (the PITCH wheel and POWER are no preset's either),
    /// each id once, each resolving to its parameter, its kind its parameter's steps, the knobs
    /// (and only they) gliding.
    #[test]
    fn every_sound_parameter_but_the_wheels_bypass_and_bend_range_is_learnable() {
        let p = Ca72Params::default();
        // (The effects' pins are the standalone's effects', not the sound's: R-FX-PARAMS.)
        let mut ids: Vec<&str> = LEARNABLE
            .iter()
            .filter(|l| l.kind != Kind::Action && fx_pin(l.id).is_none())
            .map(|l| l.id)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(
            ids.len() + 2 + crate::lfo::FX_PINS,
            LEARNABLE.len(),
            "an id twice"
        );
        assert!(
            (0..LEARNABLE.len())
                .filter(|&i| is_fx_pin(i))
                .eq(listed()..LEARNABLE.len())
        );
        let mut sound: Vec<&str> = crate::presets::sound_params(&p)
            .into_iter()
            .map(|(id, _)| id)
            .filter(|id| !["mod_wheel", "midi_bend_range", "filter_mode"].contains(id))
            // The stable panel/MIDI key operates the new shape parameter.
            .map(|id| {
                if id == "filter_shape" {
                    "filter_mode"
                } else {
                    id
                }
            })
            .collect();
        sound.sort_unstable();
        assert_eq!(ids, sound);
        for id in ["pitch_wheel", "mod_wheel", "bypass", "midi_bend_range"] {
            assert_eq!(index(id), None, "{id} is not learnable");
        }
        for (i, l) in LEARNABLE.iter().enumerate() {
            if l.kind == Kind::Action || is_fx_pin(i) {
                assert!(target(&p, i).is_none());
                continue;
            }
            let t = target(&p, i).expect(l.id);
            let kind = match t.positions() {
                None => Kind::Continuous,
                Some(2) => Kind::Switch,
                Some(_) => Kind::Stepped,
            };
            assert_eq!(kind, l.kind, "{}", l.id);
            assert_eq!(
                knob_param(&p, i).is_some(),
                l.kind == Kind::Continuous,
                "{}",
                l.id
            );
            if let Some(k) = knob_param(&p, i) {
                assert_eq!(t.ptr(), k.as_ptr(), "{}", l.id);
            }
        }
        let counts = |k: Kind| {
            LEARNABLE
                .iter()
                .filter(|l| l.kind == k && fx_pin(l.id).is_none())
                .count()
        };
        assert_eq!(
            (
                counts(Kind::Continuous),
                counts(Kind::Stepped),
                counts(Kind::Switch)
            ),
            // (The arpeggiator's: GATE and SWING knobs, DIRECTION, RATE and OCTAVES selectors,
            // ON and HOLD switches; the expression's four reaches, knobs: R-MPE; oscillators
            // 2's and 3's timed DIVISION and TIMED; each oscillator's waveform: its SHAPE knob,
            // its STEPPED or CONTINUOUS and its MOD switches, its MOD DIRECTION stepped;
            // GLIDE RANGE, a switch; each LFO's SHAPE, RATE, PHASE and FADE knobs, WAVEFORM,
            // DIVISION and MODE selectors, STEPPED or CONTINUOUS, TIMED and MONO switches: R-LFO;
            // the six macros, knobs: R-MACROS; each LFO's POLARITY, REVERSE and ON switches:
            // R-LFO's corrections; each LFO's LEVEL, a knob: R-LFO's addendum; the velocity's
            // CURVE and FLOOR, knobs: R-VELOCITY; FILTER POLES and FILTER MODE, selectors:
            // R-POLES.)
            (64, 30, 57)
        );
    }

    /// Bank select, the modulation wheel, data entry and RPN/NRPN, and the channel mode messages
    /// are not learned; every other control change is.
    #[test]
    fn the_reserved_controllers_are_not_learned() {
        let m = MidiMap::default();
        let reserved_ccs: Vec<u8> = (0..=127).filter(|&c| m.reserved(c).is_some()).collect();
        let mut want = vec![0, 1, 6, 32, 38, 96, 97, 98, 99, 100, 101];
        want.extend(120..=127);
        assert_eq!(reserved_ccs, want);
        for &c in &want {
            assert_eq!(m.assign(0, cc(0, c)), None, "CC {c}");
        }
        m.arm(at("cutoff"));
        assert_eq!(m.incoming(0, 1), Incoming::Unassigned);
        assert_eq!(m.armed(), Some(at("cutoff")), "a reserved CC is not caught");
        m.refuse(1);
        assert_eq!(m.refused(), Some(1));
        m.arm(at("emphasis"));
        assert_eq!(m.refused(), None, "said for the arming it came in");
        m.cancel();
        m.refuse(7);
        assert_eq!(m.refused(), None, "nothing said while not learning");
    }

    /// The wheel's CC is reserved from the preset actions too; their notes are not CCs.
    #[test]
    fn the_wheel_source_takes_its_cc_from_preset_actions_but_not_their_notes() {
        let m = MidiMap::default();
        let (previous, next) = (at("previous_preset"), at("next_preset"));
        assert!(
            m.assign(previous, cc(0, 1)).is_none(),
            "CC 1 is the wheel's"
        );
        m.assign(previous, cc(4, 20)).unwrap();
        let pad = Binding::Note {
            channel: 4,
            note: 20,
        };
        m.assign(next, pad).unwrap();
        m.arm(MOD_WHEEL);
        assert_eq!(m.incoming_cc(0, 20, true), Incoming::Caught);
        assert_eq!(m.poll().map(|a| a.param), Some(MOD_WHEEL));
        assert_eq!(m.binding(previous), None, "the wheel's CC on every channel");
        assert_eq!(m.binding(next), Some(pad));
        assert_eq!(m.incoming_cc(4, 20, true), Incoming::Unassigned);
        m.arm(previous);
        assert_eq!(m.incoming_cc(4, 20, true), Incoming::Unassigned);
        assert_eq!(m.armed(), Some(previous), "still waiting");
        assert_eq!(m.incoming_note(4, 20, true), Incoming::Caught);
        assert_eq!(m.poll().map(|a| a.cc), Some(pad));
        assert_eq!(m.binding(next), None, "the pad moved");
        m.set_wheel_source(Source::PolyPressure);
        assert!(m.assign(next, cc(4, 20)).is_some(), "freed again");
    }

    #[test]
    fn the_wheel_learns_a_computer_source_and_reserves_it_on_all_channels() {
        let m = MidiMap::default();
        m.assign(at("cutoff"), cc(0, 11));
        m.assign(at("emphasis"), cc(15, 11));
        m.arm(MOD_WHEEL);
        assert_eq!(m.incoming(3, 0), Incoming::Unassigned);
        assert_eq!(m.armed(), Some(MOD_WHEEL), "bank select refused");
        assert_eq!(m.incoming(3, 11), Incoming::Caught);
        assert_eq!(m.poll().map(|a| a.param), Some(MOD_WHEEL));
        assert_eq!(m.wheel_source(), Source::Cc(11));
        assert!(
            m.saved().0.is_empty(),
            "wheel source is never project state"
        );
        assert!(
            m.reserved_text()
                .contains("CC 11 (MOD WHEEL SOURCE, ALL CHANNELS)")
        );
        for channel in 0..16 {
            assert_eq!(m.assign(at("cutoff"), cc(channel, 11)), None);
            m.arm(at("cutoff"));
            assert_eq!(m.incoming(channel, 11), Incoming::Unassigned);
            assert_eq!(m.armed(), Some(at("cutoff")));
        }
        m.cancel();
        assert!(m.assign(at("glide"), cc(0, 1)).is_some(), "CC 1 freed");
        let saved = m.saved();
        assert_eq!(
            Saved::from_text(&serde_json::to_string(&saved).expect("JSON")),
            saved
        );
        m.set_wheel_source(Source::ChannelPressure);
        assert!(m.assign(at("cutoff"), cc(0, 11)).is_some());
        m.set_wheel_source(Source::DEFAULT);
        assert!(m.assignment(at("glide")).is_none(), "new wheel CC removed");
        m.load(&saved);
        assert!(
            m.saved().0.is_empty(),
            "state cannot restore a conflicting assignment"
        );
        m.arm(MOD_WHEEL);
        assert_eq!(
            m.incoming(15, 1),
            Incoming::Caught,
            "wheel can learn its current CC"
        );
        assert!(m.poll().is_some());
        assert_eq!(m.wheel_source(), Source::Cc(1));
    }

    /// A FREQUENCY knob turned to value / 127 of its travel through the computer's law
    /// (decisions.md R54): its stops -7.5 and +7.5 semitones and the centre between 63
    /// and 64 in semitones; the pot's ends at the stops in the hardware's.
    #[test]
    fn a_controller_turns_a_frequency_knob_through_the_dials_law() {
        use ca72::freq_dial::Law;
        let p = Ca72Params::default();
        let i = at("osc2_frequency");
        let t = target(&p, i).expect("a knob");
        let st = |v: u8| Pot::Two.semitones(f64::from(normalized_for(&p, i, t, v)));
        assert!((st(0) + 7.5).abs() < 1e-4 && (st(127) - 7.5).abs() < 1e-4);
        assert!(st(63) < 0.0 && st(64) > 0.0);
        p.set_freq_law(Law::Hardware);
        assert_eq!(normalized_for(&p, i, t, 127), 1.0);
        let three = at("osc3_frequency");
        let t3 = target(&p, three).expect("a knob");
        assert_eq!(normalized_for(&p, three, t3, 0), 0.0008);
        // Any other knob as its normalization has it.
        let cutoff = at("cutoff");
        let c = target(&p, cutoff).expect("a knob");
        assert_eq!(normalized_for(&p, cutoff, c, 64), 64.0 / 127.0);
    }

    /// FILTER MODE learned: LO, BAND, STEEP HIGH and NOTCH each a quarter of 0–127, never 0.3's
    /// HI; FILTER POLES its four, each a quarter (decisions.md R-POLES).
    #[test]
    fn filter_mode_learned_steps_through_the_selectors_four() {
        use crate::params::FilterShape as M;
        let p = Ca72Params::default();
        let i = at("filter_mode");
        let t = target(&p, i).expect("a selector");
        let mode = |v: u8| p.filter_shape.preview_plain(normalized_for(&p, i, t, v));
        assert_eq!(
            [
                mode(0),
                mode(31),
                mode(32),
                mode(63),
                mode(64),
                mode(95),
                mode(96),
                mode(127)
            ],
            [
                M::Lo,
                M::Lo,
                M::Band,
                M::Band,
                M::High,
                M::High,
                M::Notch,
                M::Notch
            ]
        );
        let i = at("filter_poles");
        let t = target(&p, i).expect("a selector");
        let poles = |v: u8| p.filter_poles.preview_plain(normalized_for(&p, i, t, v));
        assert_eq!(
            [
                poles(0),
                poles(31),
                poles(32),
                poles(64),
                poles(96),
                poles(127)
            ],
            [1, 1, 2, 3, 4, 4]
        );
    }

    /// A knob: value / 127 of its travel. A switch (SCATTER's placement too): off at 0–63, on at
    /// 64–127. A selector: its six positions each an equal share of 0–127; VOICES its nine.
    #[test]
    fn a_value_sets_each_kind_through_its_parameters_normalization() {
        let p = Ca72Params::default();
        let cutoff = target(&p, at("cutoff")).expect("a knob");
        assert_eq!(cutoff.normalized_for(0), 0.0);
        assert_eq!(cutoff.normalized_for(127), 1.0);
        assert_eq!(cutoff.normalized_for(64), 64.0 / 127.0);
        let on = target(&p, at("osc1_on")).expect("a switch");
        assert_eq!(on.normalized_for(63), 0.0);
        assert_eq!(on.normalized_for(64), 1.0);
        let pink = target(&p, at("noise_type")).expect("a two-way switch");
        assert_eq!(
            (pink.normalized_for(63), pink.normalized_for(64)),
            (0.0, 1.0)
        );
        // SCATTER's placement, two choices: EVEN at 0–63, CENTER at 64–127.
        let placement = target(&p, at("placement")).expect("a two-way switch");
        let chosen = |v: u8| p.placement.preview_plain(placement.normalized_for(v));
        assert_eq!(
            [chosen(0), chosen(63), chosen(64), chosen(127)],
            [
                crate::params::Scatter::Even,
                crate::params::Scatter::Even,
                crate::params::Scatter::Centre,
                crate::params::Scatter::Centre
            ]
        );
        let range = target(&p, at("osc1_range")).expect("a selector");
        let position = |v: u8| (range.normalized_for(v) * 5.0).round() as u8;
        let firsts: Vec<u8> = (1..=127)
            .filter(|&v| position(v) != position(v - 1))
            .collect();
        assert_eq!(firsts, [22, 43, 64, 86, 107]);
        assert_eq!((position(0), position(127)), (0, 5));
        let voices = target(&p, at("voices")).expect("VOICES");
        let plain = |v: u8| p.voices.preview_plain(voices.normalized_for(v));
        assert_eq!((plain(0), plain(127)), (2, 10));
        let mut seen: Vec<i32> = (0..=127).map(plain).collect();
        seen.dedup();
        assert_eq!(seen, (2..=10).collect::<Vec<_>>());
    }

    /// Learning: the next learnable controller, on its channel, is caught (changing nothing),
    /// and assigned at the editor's next look; what follows sets the parameter.
    #[test]
    fn learning_catches_the_next_controller_and_assigns_it() {
        let m = MidiMap::default();
        let cutoff = at("cutoff");
        m.arm(cutoff);
        assert_eq!(m.armed(), Some(cutoff));
        assert_eq!(m.incoming(1, 74), Incoming::Caught);
        assert_eq!(m.armed(), None);
        // (Before the editor assigns it, the knob's next moves do nothing.)
        assert_eq!(m.incoming(1, 74), Incoming::Unassigned);
        let a = m.poll().expect("assigned");
        assert_eq!(
            a,
            Assigned {
                param: cutoff,
                cc: cc(1, 74).into(),
                replaced: None,
                displaced: None
            }
        );
        assert_eq!(m.poll(), None, "once");
        assert_eq!(m.incoming(1, 74), Incoming::Assigned(cutoff));
        assert_eq!(m.assignment(cutoff), Some(cc(1, 74)));
        assert_eq!(cc(1, 74).text(), "CH 2 · CC 74");
    }

    /// Cancelling keeps every assignment; a controller caught for an arming since replaced or
    /// cancelled is not assigned.
    #[test]
    fn cancelling_keeps_the_assignments() {
        let m = MidiMap::default();
        let (cutoff, emphasis) = (at("cutoff"), at("emphasis"));
        m.assign(emphasis, cc(0, 71));
        m.arm(cutoff);
        m.cancel();
        assert_eq!(m.armed(), None);
        assert_eq!(m.incoming(0, 72), Incoming::Unassigned, "not caught");
        assert_eq!(m.incoming(0, 71), Incoming::Assigned(emphasis));
        m.arm(cutoff);
        assert_eq!(m.incoming(0, 72), Incoming::Caught);
        m.cancel();
        assert_eq!(m.poll(), None, "caught, then cancelled: not assigned");
        assert_eq!(m.saved(), Saved(vec![(emphasis, cc(0, 71).into())]));
    }

    /// Arming another control moves the learning to it.
    #[test]
    fn arming_another_control_moves_the_learning() {
        let m = MidiMap::default();
        m.arm(at("cutoff"));
        m.arm(at("emphasis"));
        assert_eq!(m.incoming(3, 20), Incoming::Caught);
        assert_eq!(m.poll().map(|a| a.param), Some(at("emphasis")));
        assert_eq!(m.assignment(at("cutoff")), None);
        // Caught for one, then another armed before the editor looked: the other waits.
        m.arm(at("glide"));
        assert_eq!(m.incoming(3, 21), Incoming::Caught);
        m.arm(at("tune"));
        assert_eq!(m.poll(), None);
        assert_eq!(m.armed(), Some(at("tune")));
    }

    /// One controller a parameter (relearning replaces it), one parameter a controller (reusing
    /// one moves it, and says from where); removing one leaves the others.
    #[test]
    fn relearning_replaces_and_reusing_moves() {
        let m = MidiMap::default();
        let (cutoff, emphasis) = (at("cutoff"), at("emphasis"));
        m.assign(cutoff, cc(0, 74));
        let a = m.assign(cutoff, cc(0, 75)).expect("assigned");
        assert_eq!((a.replaced, a.displaced), (Some(cc(0, 74).into()), None));
        assert_eq!(m.target(cc(0, 74)), None);
        let b = m.assign(emphasis, cc(0, 75)).expect("assigned");
        assert_eq!((b.replaced, b.displaced), (None, Some(cutoff)));
        assert_eq!(m.assignment(cutoff), None);
        assert_eq!(m.assignment(emphasis), Some(cc(0, 75)));
        assert_eq!(
            m.assign(emphasis, cc(0, 75))
                .map(|a| (a.replaced, a.displaced)),
            Some((None, None))
        );
        assert_eq!(m.remove(emphasis), Some(cc(0, 75).into()));
        assert_eq!(m.remove(emphasis), None);
        assert_eq!(m.incoming(0, 75), Incoming::Unassigned);
    }

    /// The same controller number on two channels is two controllers.
    #[test]
    fn channels_are_told_apart() {
        let m = MidiMap::default();
        let (cutoff, emphasis) = (at("cutoff"), at("emphasis"));
        m.assign(cutoff, cc(0, 74));
        assert_eq!(m.incoming(1, 74), Incoming::Unassigned);
        m.assign(emphasis, cc(1, 74));
        assert_eq!(m.incoming(0, 74), Incoming::Assigned(cutoff));
        assert_eq!(m.incoming(1, 74), Incoming::Assigned(emphasis));
        assert_eq!(m.incoming(15, 74), Incoming::Unassigned);
    }

    /// Saved as JSON by id, the channel 1 to 16, and read back the same.
    #[test]
    fn the_table_is_saved_by_id_and_read_back() {
        let m = MidiMap::default();
        m.assign(at("emphasis"), cc(15, 71));
        m.assign(at("cutoff"), cc(0, 74));
        m.assign(at("voices"), cc(2, 20));
        let text = serde_json::to_string(&m.saved()).expect("JSON");
        assert_eq!(
            text,
            r#"{"version":1,"assignments":[{"param":"cutoff","channel":1,"cc":74},{"param":"emphasis","channel":16,"cc":71},{"param":"voices","channel":3,"cc":20}]}"#
        );
        let back = MidiMap::default();
        back.load(&Saved::from_text(&text));
        assert_eq!(back.assignments(), m.assignments());
        let empty = MidiMap::default();
        assert_eq!(
            serde_json::to_string(&empty.saved()).expect("JSON"),
            r#"{"version":1,"assignments":[]}"#
        );
    }

    /// A switch's note (the MIDI map mode's: decisions.md R-MIDI-MAP): taken only while the mode
    /// arms the switch, never by a knob nor by MIDI Learn's menu's arming; saved with the
    /// controllers as `"note"` and read back; a note for a knob in a table is left out.
    #[test]
    fn a_switch_learns_a_note_from_the_map_mode_alone_and_keeps_it() {
        let m = MidiMap::default();
        let (arp, cutoff) = (at("arp_on"), at("cutoff"));
        assert!(takes_note(arp) && takes_note(at("next_preset")));
        assert!(!takes_note(cutoff) && !takes_note(at("arp_direction")));
        m.arm(arp);
        assert!(!m.notes_armed());
        assert_eq!(m.incoming_note(0, 36, true), Incoming::Unassigned);
        m.arm_with_notes(cutoff);
        assert!(!m.notes_armed());
        assert_eq!(m.incoming_note(0, 36, true), Incoming::Unassigned);
        m.arm_with_notes(arp);
        assert!(m.notes_armed());
        assert_eq!(m.incoming_note(4, 36, true), Incoming::Caught);
        let note = Binding::Note {
            channel: 4,
            note: 36,
        };
        assert_eq!(m.poll().map(|a| a.cc), Some(note));
        assert_eq!(m.incoming_note(4, 36, true), Incoming::Assigned(arp));
        assert!(m.assign(cutoff, note).is_none(), "a knob takes no note");
        m.assign(cutoff, cc(0, 74));
        let text = serde_json::to_string(&m.saved()).expect("JSON");
        assert_eq!(
            text,
            r#"{"version":1,"assignments":[{"param":"cutoff","channel":1,"cc":74},{"param":"arp_on","channel":5,"note":36}]}"#
        );
        let back = MidiMap::default();
        back.load(&Saved::from_text(&text));
        assert_eq!(back.bindings(), m.bindings());
        assert_eq!(
            Saved::from_text(
                r#"{"version":1,"assignments":[{"param":"cutoff","channel":1,"note":36},{"param":"arp_on","channel":1,"note":128}]}"#
            ),
            Saved::default()
        );
    }

    /// A table read leniently: another version or no object is an empty table; an entry of an
    /// unknown or unlearnable parameter, a channel or controller out of range or reserved, a
    /// value of the wrong type, or a parameter or controller already assigned above it, is left
    /// out (the first one kept).
    #[test]
    fn malformed_duplicate_and_unknown_entries_are_left_out() {
        for text in [
            "",
            "garbage",
            "[]",
            "42",
            r#"{"assignments":[{"param":"cutoff","channel":1,"cc":74}]}"#,
            r#"{"version":2,"assignments":[{"param":"cutoff","channel":1,"cc":74}]}"#,
            r#"{"version":"1","assignments":[{"param":"cutoff","channel":1,"cc":74}]}"#,
            r#"{"version":1,"assignments":{"param":"cutoff","channel":1,"cc":74}}"#,
        ] {
            assert_eq!(Saved::from_text(text), Saved::default(), "{text}");
        }
        let text = r#"{"version":1,"extra":true,"assignments":[
            {"param":"cutoff","channel":1,"cc":74},
            {"param":"cutoff","channel":1,"cc":75},
            {"param":"emphasis","channel":1,"cc":74},
            {"param":"emphasis","channel":2,"cc":74,"later":"ignored"},
            {"param":"no_such","channel":1,"cc":10},
            {"param":"pitch_wheel","channel":1,"cc":11},
            {"param":"bypass","channel":1,"cc":12},
            {"param":"midi_bend_range","channel":1,"cc":13},
            {"param":"glide","channel":0,"cc":14},
            {"param":"glide","channel":17,"cc":14},
            {"param":"glide","channel":1,"cc":128},
            {"param":"glide","channel":1,"cc":-1},
            {"param":"glide","channel":"1","cc":14},
            {"param":"glide","channel":1,"cc":0},
            {"param":"glide","channel":1,"cc":120},
            {"param":"glide","channel":1},
            "nonsense",
            {"param":"tune","channel":16,"cc":127},
            {"param":"tune","channel":16,"cc":119}
        ]}"#;
        assert_eq!(
            Saved::from_text(text),
            Saved(vec![
                (at("tune"), cc(15, 119).into()),
                (at("cutoff"), cc(0, 74).into()),
                (at("emphasis"), cc(1, 74).into()),
            ])
        );
    }

    /// Loaded over an instance's assignments, a table replaces every one of them; an armed
    /// learning is never part of what is saved.
    #[test]
    fn a_table_loaded_replaces_every_assignment_and_learning_is_not_saved() {
        let m = MidiMap::default();
        m.assign(at("cutoff"), cc(0, 74));
        m.arm(at("glide"));
        let saved = m.saved();
        assert_eq!(saved, Saved(vec![(at("cutoff"), cc(0, 74).into())]));
        let text = serde_json::to_string(&saved).expect("JSON");
        assert!(!text.contains("glide"), "{text}");
        m.load(&Saved(vec![(at("emphasis"), cc(2, 3).into())]));
        assert_eq!(m.assignment(at("cutoff")), None);
        assert_eq!(m.assignment(at("emphasis")), Some(cc(2, 3)));
        m.load(&Saved::default());
        assert!(m.assignments().iter().all(Option::is_none));
    }

    /// A state's table made good before it loads: none (saved before MIDI Learn) or not
    /// understood is an empty one; one understood is written back checked.
    #[test]
    fn a_states_table_is_made_good_before_it_loads() {
        use std::collections::BTreeMap;
        let empty = r#"{"version":1,"assignments":[]}"#;
        let mut fields = BTreeMap::new();
        filter_state(&mut fields);
        assert_eq!(fields.get(STATE_KEY).map(String::as_str), Some(empty));
        fields.insert(STATE_KEY.into(), "{not json".into());
        filter_state(&mut fields);
        assert_eq!(fields.get(STATE_KEY).map(String::as_str), Some(empty));
        fields.insert(
            STATE_KEY.into(),
            r#"{"version":1,"assignments":[{"param":"cutoff","channel":1,"cc":74},{"param":"x","channel":1,"cc":2}]}"#.into(),
        );
        filter_state(&mut fields);
        assert_eq!(
            fields.get(STATE_KEY).map(String::as_str),
            Some(r#"{"version":1,"assignments":[{"param":"cutoff","channel":1,"cc":74}]}"#)
        );
    }
}
