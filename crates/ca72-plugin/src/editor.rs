//! The editor: the front panel (`ca72-panel`) in a window of its own (baseview), its frames
//! shown with softbuffer, and under it the plug-in's own strip (`ca72_panel::strip`, A6:
//! decisions.md R44): the presets' rail, the left hand's GLIDE, DECAY and wheels, MODE,
//! VOICES and ENTROPY, how the voices are played across the field, WIDTH, INNER, DETUNE and
//! where the voices sound, DRIVE, AUTO GAIN and LEVEL. Every control operates its parameter
//! through the host, a drag one gesture (one undo step in hosts that keep them); the POWER
//! switch is the bypass; the OVERLOAD lamp, the wheels and the voices' display show what the
//! audio thread reports. The grip at the window's bottom right corner resizes it. The presets'
//! drawer drops down from under the rail over the strip (`crate::presets`, decisions.md R10),
//! the window keeping its size, and takes the keyboard while open; in it the update check
//! (`crate::update`, R27). A right click opens a control's menu, for MIDI Learn; the drawer's
//! MIDI button shows its list of assignments (`crate::learning`, R34).
//! PANEL on the rail shows or hides the whole lower panel. The window snaps to its height;
//! opening presets temporarily reveals a hidden panel, closing them hides it again.

use std::any::Any;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::local_notes::LocalNotes;
use crate::playing::Playing;
use baseview::{
    Event, EventStatus, MouseButton, MouseCursor, MouseEvent, ScrollDelta, Size, Window,
    WindowEvent, WindowHandler, WindowInfo, WindowOpenOptions, WindowScalePolicy,
};
use ca72::freq_dial::{Law, Pot};
use ca72_panel::art;
use ca72_panel::controls::{FEEDBACK_SILENT, Place as DrawerPlace, feedback_silent};
use ca72_panel::fonts::Weight;
use ca72_panel::keyboard;
use ca72_panel::learn::{Note, Tone};
use ca72_panel::modulation::{ModRenderer, ModScene};
use ca72_panel::presets::{
    BarTarget, DRAWER_H, DRAWER_TOP, DrawerRenderer, DrawerTarget, PresetKey, ROW_H, drawer_hit,
    in_tags,
};
use ca72_panel::render::TempoKeys;
use ca72_panel::strip::arp::{self as arp_ui, ArpBank, ArpRenderer, ArpScene, ArpTarget};
use ca72_panel::strip::fx::{self as fx_ui, FxRenderer, FxScene};
use ca72_panel::strip::lfo::{self as lfo_ui, LfoRenderer, LfoScene};
use ca72_panel::strip::lower as lower_ui;
use ca72_panel::strip::macros::{self as macro_ui, MacroRenderer, MacroScene};
use ca72_panel::strip::mpe::{self as mpe_ui, MpeRenderer, MpeScene, MpeTarget};
use ca72_panel::strip::{self, Bank, Field, Rail, Readout, StripRenderer, StripScene, StripTarget};
use ca72_panel::{
    CONTROLS, Kind, Lettering, Look, Renderer, Scene, Skin, Target, TicksTitles, interact,
};
use keyboard_types::{Key, KeyState, KeyboardEvent, Modifiers};
use nih_plug::editor::HostKey;
use nih_plug::prelude::*;
use plugin_kit_stereo::place;

use crate::engine::LEVELS;
use crate::fx_view::FxView;
use crate::learning::{Learning, Place, Pressed};
use crate::library::Library;
use crate::opening::{ANIMATE_S, Motion, level_of};
use crate::params::{ArpDirection, ArpRate, Ca72Params, QualityMode, Scatter};
use crate::presets::Browser;
use crate::settings::{Change, PackId, Settings};
use crate::update::Update;

mod agent;
mod entry;
mod fx_pins;
mod lfo_page;
mod macros;
mod mapping;
mod velocity;
use entry::Entry;

/// The share of the usable screen the editor opens at until it is resized.
const SCREEN_SHARE: f64 = 0.8;
/// The editor's width at first where the screen's size is unknown, in logical pixels.
pub const DEFAULT_WIDTH: u32 = 1380;
/// How small and large the grip makes it.
const MIN_WIDTH: u32 = 860;
const MAX_WIDTH: u32 = 3108;

/// The compact view's grip's square in the rail's bottom right corner (drawing units): where
/// it is drawn (`render::compact_grip`) and found.
const COMPACT_GRIP: f64 = 44.0;
/// The SIZE menu's sizes (R-SIZE): shares of the size the editor opens at on a 1920 by 1080
/// screen ([`DEFAULT_WIDTH`]); after them, the size it opens at on this screen.
const SIZES: [(&str, f64); 5] = [
    ("75%", 0.75),
    ("100%", 1.0),
    ("125%", 1.25),
    ("150%", 1.5),
    ("200%", 2.0),
];
/// The grip's tip.
const GRIP_TIP: &str = "DRAG TO RESIZE";

/// Where the settings' menu and the SIZE menu open: their top right corner under the rail's
/// settings key, at its right end (in the same place on either rail).
fn settings_menu_corner() -> (f64, f64) {
    let (x, y) = strip::centre(Rail::Plugin, StripTarget::Bar(BarTarget::Settings));
    (x + strip::KEY_W / 2.0, y + strip::KEY_H / 2.0 + 6.0)
}

/// Two presses within this long are a double click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// A mouse wheel's turns are one gesture until it rests this long.
const SCROLL_REST: Duration = Duration::from_millis(400);
/// A trackpad's logical travel equivalent to one wheel line (also its old notch threshold): a
/// continuous knob turns a notch's move (2 % of its travel) for each, a fraction for a fraction
/// (issue #107).
const SCROLL_PIXELS_PER_LINE: f64 = 40.0;
/// Whether lines scrolled count as far as they go (Windows: a notch is a line, a precision
/// touchpad's or a free-spinning wheel's `WM_MOUSEWHEEL` gives fractions of one, and a fast
/// wheel several in one event). Elsewhere a line event is a notch whatever its size: on macOS a
/// precise device gives points, and a wheel's lines are accelerated, a slow turn giving a
/// fraction of a line for a notch; X11 gives only whole lines, one a notch (R-KNOB-GLIDE).
const PRECISE_LINES: bool = cfg!(target_os = "windows");

/// A scroll's vertical travel in logical pixels, [`SCROLL_PIXELS_PER_LINE`] to a notch: points
/// (macOS's trackpad and Magic Mouse) as they come; lines with `precise_lines` (Windows) as far
/// as they go, else a notch each, as before.
fn scroll_travel(delta: ScrollDelta, precise_lines: bool) -> f64 {
    match delta {
        ScrollDelta::Pixels { y, .. } => f64::from(y),
        ScrollDelta::Lines { y, .. } => {
            let y = f64::from(y);
            let lines = if precise_lines || y == 0.0 {
                y
            } else {
                y.signum()
            };
            lines * SCROLL_PIXELS_PER_LINE
        }
    }
}
/// A control's menu's lettering, against the tip's.
const MENU_TEXT: f64 = 1.15;

/// The editor's height at `width`: the panel and the strip under it, in the drawing's
/// proportions (the official panel's).
pub fn height_for(width: u32) -> u32 {
    height_with(width, art::Panel::OFFICIAL)
}

/// The same with a panel of `panel`'s size (a skin pack's: decisions.md R-SKIN-PACKS).
pub fn height_with(width: u32, panel: art::Panel) -> u32 {
    (f64::from(width) * panel.drawing_h() / art::W).round() as u32
}

/// The official panel's (the tests' measure).
#[cfg(test)]
fn height_for_panel(width: u32, expanded: bool) -> u32 {
    height_in(width, expanded, art::Panel::OFFICIAL)
}

/// The window's height at `width`, the lower panel shown (`expanded`) or not, the panel `panel`.
fn height_in(width: u32, expanded: bool, panel: art::Panel) -> u32 {
    if expanded {
        height_with(width, panel)
    } else {
        (f64::from(width) * (DRAWER_TOP - panel.top()) / art::W).round() as u32
    }
}

/// The panel's size of the look chosen on this computer, as an editor last opened or changed
/// to it: the window's height before an editor opens (its look is read as it opens, and the
/// window asked to follow it if it differs). The official panel's (0) until then.
static CHOSEN_PANEL: AtomicU64 = AtomicU64::new(0);

fn chosen_panel() -> art::Panel {
    match CHOSEN_PANEL.load(Ordering::Relaxed) {
        0 => art::Panel::OFFICIAL,
        bits => art::Panel {
            wood: f64::from(f32::from_bits((bits >> 32) as u32)),
            face: f64::from(f32::from_bits(bits as u32)),
        },
    }
}

fn choose_panel(p: art::Panel) {
    let bits = if p == art::Panel::OFFICIAL {
        0
    } else {
        (u64::from((p.wood as f32).to_bits()) << 32) | u64::from((p.face as f32).to_bits())
    };
    CHOSEN_PANEL.store(bits, Ordering::Relaxed);
}

fn clamp_width(width: u32) -> u32 {
    width.clamp(MIN_WIDTH, MAX_WIDTH)
}

/// The width the editor opens at before it is resized: its share of the usable screen
/// (logical pixels), its height kept within the same share.
fn fitted_width(screen: Option<(f64, f64)>) -> u32 {
    fitted_width_in(screen, chosen_panel())
}

/// [`fitted_width`] for the panel `panel` (the chosen look's is the process's, which an editor
/// opening another look changes).
fn fitted_width_in(screen: Option<(f64, f64)>, panel: art::Panel) -> u32 {
    screen.map_or(DEFAULT_WIDTH, |(w, h)| {
        let w = (w * SCREEN_SHARE).min(h * SCREEN_SHARE * art::W / panel.drawing_h());
        clamp_width(w.round() as u32)
    })
}

/// What the audio thread reports to the editor after each block.
#[derive(Debug)]
pub struct Meters {
    overload: AtomicU32,
    bend: AtomicU32,
    modulation: AtomicU32,
    /// The voices sounding, a bit a voice (`Engine::sounding_mask`).
    voices: AtomicU32,
    /// Each voice's output's peak since the editor last took them (`Engine::levels`), and the
    /// plug-in's output's, the highest kept: a peak is never negative, and such a float's bits
    /// order as its value. Only while an editor watches them (`watchers`, how many).
    levels: [AtomicU32; LEVELS],
    output: AtomicU32,
    watchers: AtomicU32,
    /// Each oscillator's CONTINUOUS shape where the newest sounding voice has it (MOD's orbs:
    /// decisions.md R-CONTINUOUS-WAVE), its switch's steps; NaN for none.
    shapes: [AtomicU32; 3],
    /// The tempo the audio thread plays at (the host's, else the standalone's TEMPO), beats a
    /// minute; NaN before a block: the LFO page's timed scopes (decisions.md R-LFO-PAGE).
    tempo: AtomicU32,
    /// The strip's controls with a screen as the engine plays them, modulated (decisions.md
    /// R-LFO): ENTROPY, WIDTH, DETUNE, DRIVE, LEVEL and INNER ([`LIVE`]'s order), the controls'
    /// own units; NaN before a block.
    live: [AtomicU32; LIVE.len()],
    /// The latest note's velocity as it came, 0..1 (the left hand's meter, the drawer's LAST
    /// NOTE: decisions.md R-VELOCITY); NaN before a note.
    velocity: AtomicU32,
}

/// The modulation targets with a screen on the strip (decisions.md R-LFO: the owner, 2026-10-20,
/// "if we attach a modulator to a parameter with a screen, that screen should represent where
/// the modulator currently is"), in [`Meters`]' order. (TEMPO's screen is the tempo's.)
const LIVE: [crate::lfo::Target; 6] = [
    crate::lfo::Target::Entropy,
    crate::lfo::Target::Spread,
    crate::lfo::Target::Double,
    crate::lfo::Target::Drive,
    crate::lfo::Target::Level,
    crate::lfo::Target::Inner,
];

impl Default for Meters {
    fn default() -> Self {
        Meters {
            overload: AtomicU32::new(0.0f32.to_bits()),
            bend: AtomicU32::new(0.5f32.to_bits()),
            modulation: AtomicU32::new(0.0f32.to_bits()),
            voices: AtomicU32::new(0),
            levels: std::array::from_fn(|_| AtomicU32::new(0)),
            output: AtomicU32::new(0),
            watchers: AtomicU32::new(0),
            shapes: std::array::from_fn(|_| AtomicU32::new(f32::NAN.to_bits())),
            tempo: AtomicU32::new(f32::NAN.to_bits()),
            live: std::array::from_fn(|_| AtomicU32::new(f32::NAN.to_bits())),
            velocity: AtomicU32::new(f32::NAN.to_bits()),
        }
    }
}

impl Meters {
    /// The OVERLOAD lamp (0..1), a MIDI keyboard's wheels as the panel draws them, and the
    /// voices sounding (a bit a voice).
    pub fn publish(&self, overload: f32, (bend, modulation): (f32, f32), voices: u32) {
        self.overload.store(overload.to_bits(), Ordering::Relaxed);
        self.bend.store(bend.to_bits(), Ordering::Relaxed);
        self.modulation
            .store(modulation.to_bits(), Ordering::Relaxed);
        self.voices.store(voices, Ordering::Relaxed);
    }

    /// Each oscillator's CONTINUOUS shape where the newest sounding voice has it (the audio
    /// thread's, a block's).
    /// The tempo a block played at (decisions.md R-LFO-PAGE).
    pub fn publish_tempo(&self, bpm: f64) {
        self.tempo.store((bpm as f32).to_bits(), Ordering::Relaxed);
    }

    /// The strip's screens' controls as the engine plays them now (modulated: decisions.md
    /// R-LFO), from its controls in force.
    pub fn publish_live(&self, c: &crate::engine::Controls) {
        for (a, v) in self
            .live
            .iter()
            .zip([c.entropy, c.spread, c.double, c.drive, c.level, c.inner])
        {
            a.store((v as f32).to_bits(), Ordering::Relaxed);
        }
    }

    /// The latest note's velocity (decisions.md R-VELOCITY), none before a note.
    pub fn publish_velocity(&self, velocity: Option<f32>) {
        let v = velocity.unwrap_or(f32::NAN);
        self.velocity.store(v.to_bits(), Ordering::Relaxed);
    }

    /// [`Meters::publish_velocity`]'s.
    pub(crate) fn velocity(&self) -> Option<f64> {
        let v = f64::from(f32::from_bits(self.velocity.load(Ordering::Relaxed)));
        v.is_finite().then_some(v)
    }

    /// [`Meters::publish_live`]'s, by [`LIVE`]'s order; none before a block.
    fn live(&self) -> [Option<f64>; LIVE.len()] {
        self.live.each_ref().map(|a| {
            let v = f64::from(f32::from_bits(a.load(Ordering::Relaxed)));
            v.is_finite().then_some(v)
        })
    }

    /// The tempo the audio thread last played at, if it has played.
    fn tempo(&self) -> Option<f64> {
        let t = f64::from(f32::from_bits(self.tempo.load(Ordering::Relaxed)));
        (t.is_finite() && t > 0.0).then_some(t)
    }

    pub fn publish_shapes(&self, shapes: [Option<f64>; 3]) {
        for (a, at) in self.shapes.iter().zip(shapes) {
            a.store(
                at.map_or(f32::NAN, |at| at as f32).to_bits(),
                Ordering::Relaxed,
            );
        }
    }

    /// Each oscillator's CONTINUOUS shape where the voices have it, if any.
    fn shapes(&self) -> [Option<f64>; 3] {
        self.shapes.each_ref().map(|a| {
            let at = f64::from(f32::from_bits(a.load(Ordering::Relaxed)));
            at.is_finite().then_some(at)
        })
    }

    /// Whether an editor watches the levels (the audio thread measures the output's then).
    pub fn watched(&self) -> bool {
        self.watchers.load(Ordering::Relaxed) > 0
    }

    /// Each voice's output's peak over the block, and the output's (the audio thread's: a
    /// compare each).
    pub fn publish_levels(&self, levels: &[f32; LEVELS], output: f32) {
        let held = |a: &AtomicU32, v: f32| {
            let v = if v.is_finite() && v > 0.0 { v } else { 0.0 };
            a.fetch_max(v.to_bits(), Ordering::Relaxed);
        };
        for (a, &v) in self.levels.iter().zip(levels) {
            held(a, v);
        }
        held(&self.output, output);
    }

    /// The voices' peaks and the output's since they were last taken, taken.
    fn take_levels(&self) -> ([f64; LEVELS], f64) {
        let take = |a: &AtomicU32| {
            let v = f64::from(f32::from_bits(a.swap(0, Ordering::Relaxed)));
            if v.is_finite() { v } else { 0.0 }
        };
        (
            std::array::from_fn(|k| take(&self.levels[k])),
            take(&self.output),
        )
    }

    fn load(a: &AtomicU32) -> f64 {
        let v = f64::from(f32::from_bits(a.load(Ordering::Relaxed)));
        if v.is_finite() { v } else { 0.0 }
    }
}

/// A parameter as the editor operates it, whatever its type.
trait Operated {
    fn value(&self) -> f32;
    fn default(&self) -> f32;
    fn text(&self, normalized: f32) -> String;
    fn begin(&self, s: &ParamSetter<'_>);
    fn set(&self, s: &ParamSetter<'_>, normalized: f32);
    fn end(&self, s: &ParamSetter<'_>);
}

impl<P: Param> Operated for P {
    fn value(&self) -> f32 {
        self.unmodulated_normalized_value()
    }
    fn default(&self) -> f32 {
        self.default_normalized_value()
    }
    fn text(&self, normalized: f32) -> String {
        self.normalized_value_to_string(normalized, true)
    }
    fn begin(&self, s: &ParamSetter<'_>) {
        s.begin_set_parameter(self);
    }
    fn set(&self, s: &ParamSetter<'_>, normalized: f32) {
        s.set_parameter_normalized(self, normalized);
    }
    fn end(&self, s: &ParamSetter<'_>) {
        s.end_set_parameter(self);
    }
}

/// An oscillator's FREQUENCY knob (decisions.md R54): where it points, read through
/// the computer's law (`ca72::freq_dial`), its pot's position the parameter's (normalized,
/// the wiper). A pot beyond the law's reach shows the knob at its stop.
struct FreqKnob {
    params: Arc<Ca72Params>,
    pot: Pot,
}

impl FreqKnob {
    fn param(&self) -> &FloatParam {
        match self.pot {
            Pot::Two => &self.params.osc2_frequency,
            Pot::Three => &self.params.osc3_frequency,
        }
    }

    /// Its oscillator's index (1, 2).
    fn oscillator(&self) -> usize {
        match self.pot {
            Pot::Two => 1,
            Pot::Three => 2,
        }
    }

    /// Its oscillator's DIVISION while it is timed (its TIMED on, its RANGE at LO): what the
    /// knob turns then, a value of its own, so that FREQUENCY's place is kept for when it is off
    /// (decisions.md R-OSC-TIMED).
    fn division(&self) -> Option<&EnumParam<crate::params::Division>> {
        let n = self.oscillator();
        if !self.params.timed_now(n) {
            return None;
        }
        self.params.timed_params(n).map(|(_, d, _)| d)
    }

    /// The knob at `knob` of its travel: its pot's position.
    fn wiper(&self, knob: f32) -> f32 {
        self.pot.wiper_at(self.params.freq_law(), f64::from(knob)) as f32
    }

    fn knob(&self, wiper: f32) -> f32 {
        self.pot.knob_at(self.params.freq_law(), f64::from(wiper)) as f32
    }
}

impl Operated for FreqKnob {
    fn value(&self) -> f32 {
        if let Some(d) = self.division() {
            return d.unmodulated_normalized_value();
        }
        self.knob(self.param().unmodulated_normalized_value())
    }
    fn default(&self) -> f32 {
        if let Some(d) = self.division() {
            return d.default_normalized_value();
        }
        self.knob(self.param().default_normalized_value())
    }
    fn text(&self, knob: f32) -> String {
        if let Some(d) = self.division() {
            return d.normalized_value_to_string(knob, true);
        }
        self.param()
            .normalized_value_to_string(self.wiper(knob), true)
    }
    fn begin(&self, s: &ParamSetter<'_>) {
        match self.division() {
            Some(d) => s.begin_set_parameter(d),
            None => s.begin_set_parameter(self.param()),
        }
    }
    fn set(&self, s: &ParamSetter<'_>, knob: f32) {
        match self.division() {
            Some(d) => s.set_parameter_normalized(d, knob),
            None => s.set_parameter_normalized(self.param(), self.wiper(knob)),
        }
    }
    fn end(&self, s: &ParamSetter<'_>) {
        match self.division() {
            Some(d) => s.end_set_parameter(d),
            None => s.end_set_parameter(self.param()),
        }
    }
}

/// An oscillator's WAVEFORM switch (decisions.md R-CONTINUOUS-WAVE): its WAVEFORM parameter
/// while it is STEPPED; CONTINUOUS, its SHAPE (one position along the switch's whole travel,
/// [`interact::CONTINUOUS_PLACES`] of its steps), WAVEFORM following it to the nearest of the
/// switch's own positions (what STEPPED plays if it is switched back; oscillator 3's reverse
/// shark tooth and reverse sawtooth its REVERSE SAWTOOTH).
struct WaveformSwitch {
    params: Arc<Ca72Params>,
    /// Its oscillator, 0 to 2.
    n: usize,
}

impl WaveformSwitch {
    fn waveform(&self) -> &dyn Operated {
        match self.n {
            0 => &self.params.osc1_waveform,
            1 => &self.params.osc2_waveform,
            _ => &self.params.osc3_waveform,
        }
    }

    fn shape(&self) -> &FloatParam {
        self.params.shape_params(self.n).1
    }

    fn continuous(&self) -> bool {
        self.params.continuous(self.n)
    }

    /// WAVEFORM's place (0..5) at its normalized value.
    fn place(normalized: f32) -> usize {
        ca72_panel::art::position(f64::from(normalized))
    }

    /// WAVEFORM's position nearest a CONTINUOUS shape `at` (the switch's steps).
    fn nearest(&self, at: f64) -> usize {
        let k = (at.round().max(0.0) as usize).min(5);
        if self.n == 2 && (k == 1 || k == 2) {
            // (Its reverse shark tooth and reverse sawtooth: REVERSE SAWTOOTH.)
            1
        } else {
            k
        }
    }
}

impl Operated for WaveformSwitch {
    fn value(&self) -> f32 {
        if !self.continuous() {
            return self.waveform().value();
        }
        self.shape().unmodulated_normalized_value()
    }
    fn default(&self) -> f32 {
        if !self.continuous() {
            return self.waveform().default();
        }
        self.shape().default_normalized_value()
    }
    fn text(&self, normalized: f32) -> String {
        if !self.continuous() {
            return self.waveform().text(normalized);
        }
        crate::params::shape_text(self.n, f64::from(normalized))
    }
    fn begin(&self, s: &ParamSetter<'_>) {
        self.waveform().begin(s);
        if self.continuous() {
            s.begin_set_parameter(self.shape());
        }
    }
    fn set(&self, s: &ParamSetter<'_>, normalized: f32) {
        if !self.continuous() {
            self.waveform().set(s, normalized);
            return;
        }
        let at = f64::from(normalized) * interact::CONTINUOUS_PLACES;
        self.waveform().set(s, self.nearest(at) as f32 / 5.0);
        s.set_parameter_normalized(self.shape(), normalized);
    }
    fn end(&self, s: &ParamSetter<'_>) {
        self.waveform().end(s);
        if self.continuous() {
            s.end_set_parameter(self.shape());
        }
    }
}

/// The parameter a control of the panel operates, by its id.
fn param<'a>(p: &'a Ca72Params, id: &str) -> Option<&'a dyn Operated> {
    Some(match id {
        "tune" => &p.tune,
        "glide" => &p.glide,
        "mod_mix" => &p.mod_mix,
        "osc_mod" => &p.osc_mod,
        "osc3_control" => &p.osc3_control,
        "osc1_range" => &p.osc1_range,
        "osc1_waveform" => &p.osc1_waveform,
        "osc2_range" => &p.osc2_range,
        "osc2_frequency" => &p.osc2_frequency,
        "osc2_waveform" => &p.osc2_waveform,
        "osc3_range" => &p.osc3_range,
        "osc3_frequency" => &p.osc3_frequency,
        "osc3_waveform" => &p.osc3_waveform,
        "osc1_on" => &p.osc1_on,
        "osc1_volume" => &p.osc1_volume,
        "ext_on" => &p.ext_on,
        "ext_volume" => &p.ext_volume,
        "osc2_on" => &p.osc2_on,
        "osc2_volume" => &p.osc2_volume,
        "noise_on" => &p.noise_on,
        "noise_volume" => &p.noise_volume,
        "noise_type" => &p.noise_type,
        "osc3_on" => &p.osc3_on,
        "osc3_volume" => &p.osc3_volume,
        "filter_mode" => &p.filter_mode,
        "quality" => &p.quality,
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
        "feedback" => &p.feedback,
        "main_output" => &p.main_output,
        "a440" => &p.a440,
        "glide_on" => &p.glide_on,
        "decay_on" => &p.decay_on,
        "note_priority" => &p.note_priority,
        "retrigger" => &p.retrigger,
        // Oscillators 2's and 3's TIMED keys (decisions.md R-OSC-TIMED).
        "osc2_timed" => &p.osc2_timed,
        "osc3_timed" => &p.osc3_timed,
        // Each oscillator's MOD key (decisions.md R-CONTINUOUS-WAVE).
        "osc1_shape_mod" => &p.osc1_shape_mod,
        "osc2_shape_mod" => &p.osc2_shape_mod,
        "osc3_shape_mod" => &p.osc3_shape_mod,
        "pitch_wheel" => &p.pitch_wheel,
        "mod_wheel" => &p.mod_wheel,
        // The strip's knobs (A6).
        "voices" => &p.voices,
        "entropy" => &p.entropy,
        "spread" => &p.spread,
        "inner" => &p.inner,
        "double" => &p.double,
        "drive" => &p.drive,
        "level" => &p.level,
        // The arpeggiator's drawer's (decisions.md R-ARP).
        "arp_rate" => &p.arp_rate,
        "arp_octaves" => &p.arp_octaves,
        "arp_gate" => &p.arp_gate,
        "arp_swing" => &p.arp_swing,
        // Each note's own expression's drawer's (decisions.md R-MPE).
        "pressure_mod" => &p.pressure_mod,
        "pressure_filter" => &p.pressure_filter,
        "timbre_mod" => &p.timbre_mod,
        "timbre_filter" => &p.timbre_filter,
        // Its second row, the velocity's (decisions.md R-VELOCITY).
        "velocity_curve" => &p.velocity_curve,
        "velocity_floor" => &p.velocity_floor,
        // The macros' (decisions.md R-MACROS).
        id if crate::lfo::macro_of_id(id).is_some() => {
            p.macro_param(crate::lfo::macro_of_id(id).unwrap_or(0))
        }
        // The LFO page's (decisions.md R-LFO; its RATE and WAVEFORM operated through
        // `lfo_page::LfoRate` and `LfoWave`).
        id => {
            let (n, part) = lfo_page::lfo_part(id)?;
            return lfo_page::lfo_param(p, n, part);
        }
    })
}

/// A readout a value is typed into (decisions.md R52): the strip's, the arpeggiator's drawer's
/// (RATE, OCTAVES, GATE, SWING; R-ARP), or the expression drawer's (PRESSURE's MODULATION and
/// CUTOFF, TIMBRE's; R-MPE).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Typed {
    Strip(Readout),
    Arp(usize),
    Mpe(usize),
    /// The standalone's TEMPO screen.
    Tempo,
}

/// The arpeggiator's drawer's readout (RATE, OCTAVES, GATE, SWING) of control `i`, if it is
/// one of the drawer's knobs.
fn arp_readout(i: usize) -> Option<usize> {
    let c = &CONTROLS[i];
    (c.place == ca72_panel::controls::Place::Arp)
        .then(|| arp_ui::KNOB_PARAMS.iter().position(|id| *id == c.param))?
}

/// The expression drawer's readout of control `i`, if it is one of the drawer's knobs
/// (decisions.md R-MPE).
fn mpe_readout(i: usize) -> Option<usize> {
    let c = &CONTROLS[i];
    (c.place == ca72_panel::controls::Place::Mpe)
        .then(|| mpe_ui::KNOB_PARAMS.iter().position(|id| *id == c.param))?
}

/// How long INPUT reads what came in last after it stops (decisions.md R-MPE): long enough for a
/// held note's pressure at rest between a controller's messages.
const HEARD_FOR: Duration = Duration::from_secs(3);

/// Logical pixels a beat a minute while the TEMPO screen is dragged (with SHIFT, four times
/// as many), and a trackpad's travel a beat a minute.
const TEMPO_DRAG: f64 = 2.0;
const TEMPO_SCROLL: f64 = 8.0;

/// The drawers dropped from the rail: the arpeggiator's (decisions.md R-ARP), the standalone's
/// effects' (R-FX-DRAWER), and each note's own expression's (R-MPE).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Lower {
    #[default]
    Arp,
    Fx,
    Mpe,
}

impl Lower {
    /// How deep it is.
    fn depth(self) -> f64 {
        match self {
            Lower::Arp => arp_ui::DEPTH,
            Lower::Fx => fx_ui::DEPTH,
            Lower::Mpe => mpe_ui::DEPTH,
        }
    }
}

/// The drawer dropped from the rail, one at a time: which, open or not, and its slide, as the
/// presets' drawer's (`presets::SLIDE`).
#[derive(Debug, Default)]
struct Dropped {
    which: Lower,
    open: bool,
    slide: Option<(Instant, bool)>,
}

impl Dropped {
    /// `which` opened or put away, sliding there; another open put away for it at once (it
    /// drops from the rail anew).
    fn show(&mut self, which: Lower, on: bool) {
        if !on {
            if self.which == which && self.open {
                self.open = false;
                self.slide = Some((Instant::now(), false));
            }
            return;
        }
        if self.open && self.which == which {
            return;
        }
        self.which = which;
        self.open = true;
        self.slide = Some((Instant::now(), true));
    }

    /// Whether `which` is open (or opening).
    fn is_open(&self, which: Lower) -> bool {
        self.open && self.which == which
    }

    /// Put away at once (the lower panel hidden).
    fn shut(&mut self) {
        self.open = false;
        self.slide = None;
    }

    /// How far down it is, 0 to 1.
    fn reveal(&self) -> f64 {
        match self.slide {
            Some((t0, opening)) => {
                let k = (t0.elapsed().as_secs_f64() / crate::presets::SLIDE.as_secs_f64()).min(1.0);
                let k = 1.0 - (1.0 - k) * (1.0 - k);
                if opening { k } else { 1.0 - k }
            }
            None => f64::from(u8::from(self.open)),
        }
    }

    fn sliding(&self) -> bool {
        self.slide
            .is_some_and(|(t0, _)| t0.elapsed() < crate::presets::SLIDE)
    }

    fn tick(&mut self) {
        if self.slide.is_some() && !self.sliding() {
            self.slide = None;
        }
    }

    /// Open and still: its parts answer the pointer.
    fn settled(&self) -> bool {
        self.open && self.slide.is_none()
    }
}

/// The editor, as the plug-in hands it to the host.
pub struct Ca72Editor {
    preferred_width: u32,
    params: Arc<Ca72Params>,
    meters: Arc<Meters>,
    local_notes: Arc<LocalNotes>,
    /// The host's scale (f32 bits; 0 none given: the system's, as on macOS).
    scale: AtomicU32,
    open: Arc<AtomicBool>,
    /// The width fitted to the screen for this opening (0 not yet measured).
    fitted: Arc<AtomicU32>,
    /// What measures the screen ([`screen::usable`]; a test's own screen).
    screen: fn(Option<f64>) -> Option<(f64, f64)>,
    /// The keys the host gives the editor's view rather than its window (decisions.md R53).
    host_keys: Arc<HostKeys>,
}

/// Keys the host gives the editor's view rather than its window (decisions.md R53: Cubase
/// keeps the keyboard from the window and gives a VST3 plug-in its keys through
/// `IPlugView::onKeyDown()`): whether the editor holds the keyboard now (a value typed, the
/// drawer, MIDI Learn), and those taken for it, for its window at its next frame.
#[derive(Debug, Default)]
pub struct HostKeys {
    holding: AtomicBool,
    navigation: AtomicBool,
    queue: Mutex<Vec<KeyboardEvent>>,
    overflow: AtomicBool,
}

impl HostKeys {
    /// A key the host gave: taken while the editor holds the keyboard (every key then, so that
    /// none is the host's key command meanwhile), else the host's.
    pub fn offer(&self, k: HostKey) -> bool {
        let event = host_key_event(k);
        let navigation = self.navigation.load(Ordering::Acquire)
            && event.as_ref().is_some_and(|e| preset_arrow(e).is_some());
        if !self.holding.load(Ordering::Acquire) && !navigation {
            return false;
        }
        if let Some(e) = event
            && let Ok(mut q) = self.queue.lock()
        {
            if q.len() < 64 {
                q.push(e);
            } else {
                self.overflow.store(true, Ordering::Release);
            }
        }
        true
    }

    /// The keys taken since the last time.
    fn take(&self) -> Vec<KeyboardEvent> {
        self.queue
            .lock()
            .map(|mut q| std::mem::take(&mut *q))
            .unwrap_or_default()
    }
}

/// A host's key as the window's own come: a key VST3 names by its code (Backspace, Enter,
/// Escape, the arrows and so on), else its character (a numeric keypad's digit, point or minus
/// sign too when the host gives none); its modifiers. None for what the editor has no use for.
pub fn host_key_event(k: HostKey) -> Option<KeyboardEvent> {
    // VST3's `VirtualKeyCodes` (pluginterfaces/base/keycodes.h).
    let named = match k.key_code {
        1 => Some(Key::Backspace),
        2 => Some(Key::Tab),
        4 | 19 => Some(Key::Enter),
        6 => Some(Key::Escape),
        9 => Some(Key::End),
        10 => Some(Key::Home),
        11 => Some(Key::ArrowLeft),
        12 => Some(Key::ArrowUp),
        13 => Some(Key::ArrowRight),
        14 => Some(Key::ArrowDown),
        15 => Some(Key::PageUp),
        16 => Some(Key::PageDown),
        22 => Some(Key::Delete),
        54 => Some(Key::Shift),
        55 => Some(Key::Control),
        56 => Some(Key::Alt),
        _ => None,
    };
    let key = match (named, k.character) {
        (Some(n), _) => n,
        (None, Some(c)) if !c.is_control() => Key::Character(c.to_string()),
        (None, _) => match k.key_code {
            7 => Key::Character(" ".into()),
            24..=33 => Key::Character(char::from(b'0' + (k.key_code - 24) as u8).to_string()),
            37 => Key::Character("-".into()),
            38 => Key::Character(".".into()),
            _ => return None,
        },
    };
    // VST3's `KeyModifier`: shift, alt, command (Ctrl on Windows), control (the Windows key).
    let mut modifiers = Modifiers::empty();
    if k.modifiers & 1 != 0 {
        modifiers |= Modifiers::SHIFT;
    }
    if k.modifiers & 2 != 0 {
        modifiers |= Modifiers::ALT;
    }
    let (command, control) = if cfg!(target_os = "macos") {
        (Modifiers::META, Modifiers::CONTROL)
    } else {
        (Modifiers::CONTROL, Modifiers::META)
    };
    if k.modifiers & 4 != 0 {
        modifiers |= command;
    }
    if k.modifiers & 8 != 0 {
        modifiers |= control;
    }
    Some(KeyboardEvent {
        state: if k.down { KeyState::Down } else { KeyState::Up },
        key,
        modifiers,
        ..KeyboardEvent::default()
    })
}

/// How long a key coming both ways (the host's and the window's own) waits for its twin.
const TWIN: Duration = Duration::from_millis(150);

/// Keys taken once though a host gives them both ways: each kept for [`TWIN`] to be matched
/// with its twin from the other way, which is then not taken again.
#[derive(Debug, Default)]
struct KeyTwins {
    host: VecDeque<(Instant, Key, KeyState)>,
    own: VecDeque<(Instant, Key, KeyState)>,
}

impl KeyTwins {
    /// Whether a key the host gave is new (not one the window had just now).
    fn host_new(&mut self, k: &KeyboardEvent, now: Instant) -> bool {
        Self::new_one(&mut self.own, &mut self.host, k, now)
    }

    /// Whether a key the window had is new (not one the host gave just now).
    fn own_new(&mut self, k: &KeyboardEvent, now: Instant) -> bool {
        Self::new_one(&mut self.host, &mut self.own, k, now)
    }

    fn new_one(
        other: &mut VecDeque<(Instant, Key, KeyState)>,
        mine: &mut VecDeque<(Instant, Key, KeyState)>,
        k: &KeyboardEvent,
        now: Instant,
    ) -> bool {
        let fresh = |t: &Instant| now.saturating_duration_since(*t) < TWIN;
        other.retain(|(t, ..)| fresh(t));
        mine.retain(|(t, ..)| fresh(t));
        if let Some(i) = other
            .iter()
            .position(|(_, key, state)| *key == k.key && *state == k.state)
        {
            other.remove(i);
            return false;
        }
        mine.push_back((now, k.key.clone(), k.state));
        true
    }
}

impl std::fmt::Debug for Ca72Editor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ca72Editor")
            .field("size", &self.size())
            .finish()
    }
}

impl Ca72Editor {
    pub fn new(params: Arc<Ca72Params>, meters: Arc<Meters>) -> Self {
        Self::with_notes(params, meters, Arc::new(LocalNotes::default()))
    }

    pub fn with_notes(
        params: Arc<Ca72Params>,
        meters: Arc<Meters>,
        local_notes: Arc<LocalNotes>,
    ) -> Self {
        let settings = if cfg!(test) {
            Settings::default()
        } else {
            Settings::load()
        };
        Self::with_settings(params, meters, local_notes, settings)
    }

    fn with_settings(
        params: Arc<Ca72Params>,
        meters: Arc<Meters>,
        local_notes: Arc<LocalNotes>,
        settings: Settings,
    ) -> Self {
        Ca72Editor {
            preferred_width: settings.editor_width,
            params,
            meters,
            local_notes,
            scale: AtomicU32::new(0),
            open: Arc::new(AtomicBool::new(false)),
            fitted: Arc::new(AtomicU32::new(0)),
            screen: screen::usable,
            host_keys: Arc::new(HostKeys::default()),
        }
    }

    fn host_scale(&self) -> Option<f32> {
        let s = f32::from_bits(self.scale.load(Ordering::Relaxed));
        (s > 0.0 && s.is_finite()).then_some(s)
    }

    /// The editor opening: the drawer shut (the window does not open grown), at the size the
    /// host was last told, the size of its window. The screen is not measured again: the one
    /// in use may have changed since the host asked (macOS's main screen is the one with the
    /// key window; the standalone's own window, sized on the Retina screen, had become it on a
    /// 1x monitor, and the editor was drawn wider than it; decisions.md R-TWO-SCREENS).
    fn opening(&self) -> (u32, u32) {
        self.params.editor_presets.store(false, Ordering::Relaxed);
        self.params.editor_panel_applied.store(
            self.params.editor_panel.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        self.params.editor_expanded.store(
            self.params.editor_panel.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        self.size()
    }

    /// The width fitted to the screen, measured once an opening (at the host's first question,
    /// or as the editor opens if it asked none) and kept until the editor closes. Measured
    /// afresh at each of the host's questions, it changed with the drawer (on Linux it is the
    /// monitor under the pointer), and the panel was drawn again at another scale as the
    /// drawer opened and shut (the owner, 2026-10-03; decisions.md R19).
    fn fitted(&self) -> u32 {
        match self.fitted.load(Ordering::Relaxed) {
            0 => {
                let w = fitted_width((self.screen)(self.host_scale().map(f64::from)));
                self.fitted.store(w, Ordering::Relaxed);
                w
            }
            w => w,
        }
    }

    /// What resets the editor as its window closes (the host may ask its size before it
    /// opens it again: the drawer is shut then).
    fn closing(&self) -> Closing {
        Closing {
            open: Arc::clone(&self.open),
            fitted: Arc::clone(&self.fitted),
            resizing: Arc::clone(&self.params.editor_resizing),
        }
    }
}

/// The editor's window closed: not open, the drawer shut, the screen to be measured again.
struct Closing {
    open: Arc<AtomicBool>,
    fitted: Arc<AtomicU32>,
    resizing: Arc<crate::resizing::Resizing>,
}

impl Drop for Closing {
    fn drop(&mut self) {
        self.open.store(false, Ordering::Release);
        self.fitted.store(0, Ordering::Relaxed);
        self.resizing.close();
    }
}

/// How baseview scales the window: on Windows and Linux by the host's scale, else by none.
/// nih-plug gives the host the editor's size times the host's scale (1 when it set none:
/// Ableton Live sets none), and the window must fill that frame; baseview's own scale there
/// (the monitor's, Xft.dpi) made the window draw at one scale, size itself at another and
/// place the pointer at a third (decisions.md R18). macOS scales by itself.
fn scale_policy(host: Option<f32>) -> WindowScalePolicy {
    if cfg!(target_os = "macos") {
        WindowScalePolicy::SystemScaleFactor
    } else {
        WindowScalePolicy::ScaleFactor(host.map_or(1.0, f64::from))
    }
}

impl Ca72Editor {
    /// The editor's window opened in `parent`, its presets from `library` (the tests' own).
    fn spawn_with(
        &self,
        parent: ParentWindowHandle,
        context: Arc<dyn GuiContext>,
        library: Library,
    ) -> Box<dyn Any + Send> {
        let (width, height) = self.opening();
        let scale = self.host_scale();
        let params = Arc::clone(&self.params);
        let meters = Arc::clone(&self.meters);
        let host_keys = Arc::clone(&self.host_keys);
        let local_notes = Arc::clone(&self.local_notes);
        let parent = window::Parent(parent);
        // No window to open in (no parent, no X11 display): nothing shown, rather than
        // baseview's panic in the host's thread (decisions.md R18).
        if !parent.can_open() {
            return Box::new(window::Handle::new(None, self.closing()));
        }
        let window = Window::open_parented(
            &parent,
            WindowOpenOptions {
                title: String::from("CA-72"),
                size: Size::new(f64::from(width), f64::from(height)),
                scale: scale_policy(scale),
            },
            move |window: &mut Window<'_>| {
                let mut panel = PanelWindow::new(
                    window,
                    context,
                    params,
                    meters,
                    width,
                    f64::from(scale.unwrap_or(1.0)),
                    library,
                    host_keys,
                );
                panel.editing.playing.notes = local_notes;
                panel.editing.playing.sync();
                panel
            },
        );
        self.open.store(true, Ordering::Release);
        Box::new(window::Handle::new(Some(window), self.closing()))
    }
}

impl Editor for Ca72Editor {
    fn spawn(
        &self,
        parent: ParentWindowHandle,
        context: Arc<dyn GuiContext>,
    ) -> Box<dyn Any + Send> {
        self.spawn_with(parent, context, Library::shared())
    }

    fn size(&self) -> (u32, u32) {
        let width = match self.params.editor_width.load(Ordering::Relaxed) {
            0 if self.preferred_width != 0 => clamp_width(self.preferred_width),
            0 => self.fitted(),
            w => clamp_width(w),
        };
        let preference = self.params.editor_panel.load(Ordering::Relaxed);
        let expanded = if !self.open.load(Ordering::Acquire) {
            preference
        } else if preference != self.params.editor_panel_applied.load(Ordering::Relaxed) {
            preference || self.params.editor_presets.load(Ordering::Relaxed)
        } else {
            self.params.editor_expanded.load(Ordering::Relaxed)
        };
        (width, height_in(width, expanded, chosen_panel()))
    }

    fn resize_request(&self) -> (u64, (u32, u32)) {
        self.params
            .editor_resizing
            .requested()
            .unwrap_or_else(|| (0, self.size()))
    }

    fn resize_result(&self, request: u64, accepted: bool) {
        if self.open.load(Ordering::Acquire) {
            self.params.editor_resizing.answer(request, accepted);
        }
    }

    fn set_scale_factor(&self, factor: f32) -> bool {
        // The window cannot follow a new scale once open (Ableton Live asks).
        if self.open.load(Ordering::Acquire) {
            return false;
        }
        // A width measured in the old scale's pixels (Windows, Linux) is measured again.
        if self.scale.swap(factor.to_bits(), Ordering::Relaxed) != factor.to_bits() {
            self.fitted.store(0, Ordering::Relaxed);
        }
        true
    }

    // The window reads the parameters every frame.
    fn param_value_changed(&self, _id: &str, _normalized_value: f32) {}
    fn param_modulation_changed(&self, _id: &str, _modulation_offset: f32) {}
    fn param_values_changed(&self) {}

    // Keys the host gives the view (Cubase): the editor's while it holds the keyboard.
    fn on_host_key(&self, key: HostKey) -> bool {
        self.host_keys.offer(key)
    }
}

/// A control being dragged.
#[derive(Debug)]
struct Drag {
    control: usize,
    /// Where the pointer pressed (logical pixels), and what it pressed.
    from: (f64, f64),
    pressed: Target,
    /// Its value at the press; the drag counts from `base` at `at` (Shift changes the rate
    /// mid-drag: the travel then counts from where it changed).
    value: f64,
    base: f64,
    at: (f64, f64),
    fine: bool,
    moved: bool,
}

/// A mouse wheel turning a control.
#[derive(Debug)]
struct Scroll {
    control: usize,
    value: f64,
    last: Instant,
    /// A discrete control's travel not yet a whole notch, in logical scroll pixels.
    travel: f64,
}

/// The editor's window: its surface, and the panel being operated in it.
struct PanelWindow {
    surface: window::Surface,
    physical: (u32, u32),
    /// Its size in logical pixels, as last set or reported.
    logical: (u32, u32),
    /// When the frame was last shown on the window (none since it opened or resized).
    shown: Option<Instant>,
    /// Until when the frame is shown again at every frame, after it was first shown at a size
    /// ([`SETTLE`]).
    settle: Option<Instant>,
    /// When the frame was last drawn while the window could not be seen ([`HIDDEN_DRAW`]).
    hidden_drawn: Option<Instant>,
    editing: Editing,
    /// The keyboard the drawer took from another window (Windows), given back as it shuts.
    keys: Option<window::Keys>,
    /// The keys the host gives the editor's view (decisions.md R53), each taken once if they
    /// come both ways.
    host_keys: Arc<HostKeys>,
    twins: KeyTwins,
    /// The pointer's look as last set (R-SIZE: the resize arrows over the grip).
    cursor: MouseCursor,
}

impl Drop for PanelWindow {
    // Closed: the host's keys are the host's again.
    fn drop(&mut self) {
        self.host_keys.holding.store(false, Ordering::Release);
    }
}

/// The host's context as the panel's controls set the parameters through it, noting a change
/// of DRIVE or AUTO GAIN (decisions.md R-AUTO-GAIN-DRIVE): the one change that asks for AUTO
/// GAIN's curve. It keeps what each was last set to here (normalized, f32's bits; NaN: not
/// yet), since a host may change a parameter only later (CLAP's, at its next block).
struct Watched {
    context: Arc<dyn GuiContext>,
    drive: ParamPtr,
    auto_gain: ParamPtr,
    moved: AtomicBool,
    drive_set: AtomicU32,
    auto_gain_set: AtomicU32,
}

impl Watched {
    fn new(context: Arc<dyn GuiContext>, params: &Ca72Params) -> Self {
        Watched {
            context,
            drive: params.drive.as_ptr(),
            auto_gain: params.auto_gain.as_ptr(),
            moved: AtomicBool::new(false),
            drive_set: AtomicU32::new(f32::NAN.to_bits()),
            auto_gain_set: AtomicU32::new(f32::NAN.to_bits()),
        }
    }

    /// Whether DRIVE or AUTO GAIN has been set here since this was last asked.
    fn take_moved(&self) -> bool {
        self.moved.swap(false, Ordering::AcqRel)
    }

    /// DRIVE (dB) and AUTO GAIN as last set here, else as the parameters have them.
    fn drive_and_auto_gain(&self, p: &Ca72Params) -> (f32, bool) {
        let set =
            |a: &AtomicU32| Some(f32::from_bits(a.load(Ordering::Acquire))).filter(|v| !v.is_nan());
        let drive = set(&self.drive_set).map_or(p.drive.value(), |v| p.drive.preview_plain(v));
        let auto = set(&self.auto_gain_set).map_or(p.auto_gain.value(), |v| v >= 0.5);
        (drive, auto)
    }
}

#[allow(unsafe_code)]
impl GuiContext for Watched {
    fn plugin_api(&self) -> PluginApi {
        self.context.plugin_api()
    }
    fn native_window_occlusion_reliable(&self) -> bool {
        self.context.native_window_occlusion_reliable()
    }
    fn request_resize(&self) -> bool {
        self.context.request_resize()
    }
    fn resize_response_will_follow(&self) -> bool {
        self.context.resize_response_will_follow()
    }
    unsafe fn raw_begin_set_parameter(&self, param: ParamPtr) {
        // SAFETY: the caller's, passed on.
        unsafe { self.context.raw_begin_set_parameter(param) }
    }
    unsafe fn raw_set_parameter_normalized(&self, param: ParamPtr, normalized: f32) {
        if param == self.drive {
            self.drive_set
                .store(normalized.to_bits(), Ordering::Release);
            self.moved.store(true, Ordering::Release);
        } else if param == self.auto_gain {
            self.auto_gain_set
                .store(normalized.to_bits(), Ordering::Release);
            self.moved.store(true, Ordering::Release);
        }
        // SAFETY: the caller's, passed on.
        unsafe { self.context.raw_set_parameter_normalized(param, normalized) }
    }
    unsafe fn raw_end_set_parameter(&self, param: ParamPtr) {
        // SAFETY: the caller's, passed on.
        unsafe { self.context.raw_end_set_parameter(param) }
    }
    fn get_state(&self) -> PluginState {
        self.context.get_state()
    }
    fn set_state(&self, state: PluginState) {
        self.context.set_state(state);
    }
}

/// The panel being operated: what the pointer does to the parameters, and the scene drawn.
/// The folder skin packs are installed in (decisions.md R-SKIN-PACKS): the shared folder's
/// `Skins`, a folder each, for every Idle Foundry plug-in (each pack names the one it is for).
#[cfg(not(test))]
fn packs_dir() -> std::path::PathBuf {
    crate::library::shared_dir().join("Skins")
}

/// An installed skin pack, as the settings' menu lists it: its name, and its look if it can be
/// used (else its folder's name, and why not in the log).
#[derive(Clone, Debug)]
struct Installed {
    name: String,
    look: Option<Look>,
}

#[cfg(test)]
thread_local! {
    /// The packs a test has installed (none of the user's are read in the tests).
    static TEST_PACKS: std::cell::RefCell<Vec<Installed>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The skin packs installed on this computer, each read and checked (in the tests, the test's).
fn installed() -> Vec<Installed> {
    #[cfg(test)]
    {
        TEST_PACKS.with(|p| p.borrow().clone())
    }
    #[cfg(not(test))]
    {
        ca72_panel::look::discover(&packs_dir())
            .into_iter()
            .map(|found| match found {
                Ok(look) => Installed {
                    name: look.menu_name(),
                    look: Some(look),
                },
                Err((folder, report)) => {
                    nih_log!("The skin pack in {folder} cannot be used:\n{report}");
                    Installed {
                        name: folder,
                        look: None,
                    }
                }
            })
            .collect()
    }
}

/// The look the settings choose: the pack chosen, if it is installed and can be used, else the
/// built-in look (the pack's base, kept with it). A pack missing or refused changes nothing
/// but the look: the sound, presets, automation and the session are not the look's.
fn chosen_look(settings: &Settings, installed: &[Installed]) -> Look {
    settings
        .pack
        .and_then(|id| {
            installed
                .iter()
                .filter_map(|p| p.look.as_ref())
                .find(|l| l.id == id.as_str())
                .cloned()
        })
        .unwrap_or_else(|| Look::of(settings.skin))
}

struct Editing {
    context: Arc<dyn GuiContext>,
    params: Arc<Ca72Params>,
    meters: Arc<Meters>,
    /// Physical pixels a logical pixel, and the window's width in logical pixels.
    dpr: f64,
    width: u32,
    physical_width: u32,
    renderer: Renderer,
    /// The skin packs installed, as last looked for (as the editor opened, if one is chosen, and
    /// as the settings' menu opens).
    installed: Vec<Installed>,
    scene: Scene,
    /// The strip's own parts (over the drawing's strip): their renderer and scene, and
    /// ENTROPY's, WIDTH's, DETUNE's and INNER's last amount but 0 (their switches, and SCATTER |
    /// DOUBLE, turn them back on there; `amount` gives each its place).
    strip: StripRenderer,
    strip_scene: StripScene,
    last: [f32; 4],
    /// The presets: the drawer's renderer, what the rail and the drawer show and do, and the
    /// window's frame: the panel's, the strip's parts under its foot, the drawer over them.
    drawer: DrawerRenderer,
    pub(crate) browser: Browser,
    /// The arpeggiator's drawer (decisions.md R-ARP): its renderer, what it shows. The drawer
    /// dropped from the rail: which, whether it is open, and how far down it was in the last frame
    /// composed (and which it was).
    arp: ArpRenderer,
    arp_scene: ArpScene,
    lower: Dropped,
    lower_shown: (f64, Lower),
    last_arp_press: Option<(Instant, ArpTarget)>,
    /// Each note's own expression's drawer (decisions.md R-MPE): its renderer, what it shows, its
    /// readouts' last press (for a double click), and what INPUT heard: the audio thread's counts
    /// as last seen, and when each last changed.
    mpe: MpeRenderer,
    mpe_scene: MpeScene,
    last_mpe_press: Option<(Instant, MpeTarget)>,
    heard: ((u32, u32), Option<Instant>, Option<Instant>),
    /// The standalone's effects' drawer (decisions.md R-FX-DRAWER): the chain as it shows it and
    /// what a pointer does there, its renderer (none without a chain), what it shows; the rail's
    /// keys (its FX key with a chain).
    fx_view: FxView,
    fx: Option<FxRenderer>,
    fx_scene: FxScene,
    fx_pin_drag: Option<fx_pins::PinDrag>,
    rail: Rail,
    /// The standalone's TEMPO screen (decisions.md R-ARP): dragged (from where, logical pixels
    /// down, at what tempo, SHIFT held; the tempo the drag began at), its last press (for a
    /// double click), and a trackpad's travel over it not yet a beat a minute.
    tempo_drag: Option<(f64, f64, bool, u16)>,
    last_tempo_press: Option<Instant>,
    /// The standalone's TYPING, PANIC or MIDI key under the pointer, and PANIC held down (lit
    /// while it is: decisions.md R-TEMPO-CORNER).
    tempo_hover: Option<art::TempoKey>,
    panic_held: bool,
    /// The standalone's MIDI map mode (decisions.md R-MIDI-MAP).
    mapping: mapping::Mapping,
    /// The screens' controls as the engine plays them, modulated ([`Meters::publish_live`]),
    /// and the moving tempo, read at most 30 times a second while a source moves one of them
    /// (decisions.md R-LFO), and when they are next due.
    live: [Option<f64>; LIVE.len()],
    live_tempo: Option<f64>,
    live_due: Option<Instant>,
    tempo_travel: f64,
    composed: Option<ca72_panel::Pixmap>,
    /// Where the window's frame changed in the last `draw` that changed it (pixels: left,
    /// top, right, bottom), or none when it may have changed anywhere (decisions.md R58).
    damage: Option<Vec<[i32; 4]>>,
    /// A size asked of the window.
    resizing_window: bool,
    /// The window to be asked to follow the look's panel's size (it opened at the size last
    /// chosen, and the look read as it opened has another).
    panel_due: bool,
    /// The drawer placed for its opening (below the strip or over the panel), asked once.
    placed: bool,
    /// The update check in the drawer; dropped with the editor, it ends a check under way.
    update: Update,
    /// MIDI Learn: a control's menu, the note over the control being learned, the drawer's
    /// list (decisions.md R34).
    pub(crate) learning: Learning,
    /// The pointer, in logical pixels.
    pointer: (f64, f64),
    hover: Option<Target>,
    drag: Option<Drag>,
    scroll: Option<Scroll>,
    /// Whether a control takes lines scrolled as far as they go ([`PRECISE_LINES`]: this
    /// system's reading; a test's to choose).
    precise_lines: bool,
    /// The presets' list's scrolling not yet a whole row, in rows (+ up).
    list_travel: f64,
    last_press: Option<(Instant, Target)>,
    /// The grip dragged: the pointer's x and the width at the press; the width it last took
    /// the window to, asked of the host at the next frame (once a frame, however often the
    /// pointer moves).
    resizing: Option<(f64, u32)>,
    /// The controls' way to the host, watching for DRIVE and AUTO GAIN changed here: AUTO
    /// GAIN's curve is asked for once nothing is held after one ([`Editing::measure_when_done`];
    /// decisions.md R-AUTO-GAIN-DRIVE). The presets set the parameters through `context`.
    watched: Watched,
    gripped: Option<u32>,
    /// The usable screen (logical pixels) at the window's scale (the editor's opening size, the
    /// SIZE menu's FIT TO SCREEN and the sizes that fit).
    screen: fn(f64) -> Option<(f64, f64)>,
    /// How far the drawer was down (`Browser::reveal`) in the last frame composed.
    reveal_shown: f64,
    /// QUALITY's opening (decisions.md R48; `opening.rs`), moved on by the time since it
    /// last was, at most [`ANIMATE_S`] apart (the strip's drops move on with it: the two are
    /// drawn in one frame); the editor opens with QUALITY's setting up. The voices' levels as last taken (0 to 1), for the strip's drops, and the output's
    /// (the one instrument's drop's, and the lamps').
    motion: Motion,
    motion_at: Option<Instant>,
    /// When the opening is next due to move on: a thirtieth of a second after the last time it
    /// was due, not after it last moved, so that baseview's 15 ms frames (which reach a
    /// thirtieth only every third) still move it thirty times a second on average, not 22; and
    /// never due before the present (after a stalled frame it moves once, and once more at the
    /// next frame at most, not in a burst).
    motion_due: Option<Instant>,
    levels: [f64; LEVELS],
    output: f64,
    /// What this computer keeps (`settings.rs`): ULTRA's shutter or always open, ULTRA's
    /// note read. The note shows once ULTRA's lamp is lit, until it is read.
    settings: Settings,
    /// Where they are kept (none in the tests' editors, which keep nothing on the computer).
    settings_file: Option<std::path::PathBuf>,
    ultra_note: bool,
    whats_new: Option<ca72_panel::whats_new::View>,
    whats_new_checked: bool,
    /// A value being typed into a readout (decisions.md R52): which, and what so far; the
    /// keyboard wanted for it, asked once; the strip's last press (where, when, and the amount
    /// a switch had before it), for a double click.
    typing: Option<(Typed, String)>,
    knob_entry: Option<Entry>,
    typing_keys: bool,
    last_strip_press: Option<(Instant, StripTarget, f32)>,
    /// Oscillators 2 and 3's FREQUENCY knobs, through the computer's law.
    freq_knobs: [FreqKnob; 2],
    waveforms: [WaveformSwitch; 3],
    /// The LFO page (decisions.md R-LFO): its renderer and what it shows; the knobs' rings, a
    /// dragged assignment and the hover pop-up over the panel, and what they show; the editor's
    /// modulation state; each LFO's RATE (Hz or timed) and WAVEFORM as operated.
    lfo: LfoRenderer,
    lfo_scene: LfoScene,
    modulation: ModRenderer,
    mod_scene: ModScene,
    modulating: lfo_page::Modulating,
    lfo_rates: [lfo_page::LfoRate; 4],
    lfo_waves: [lfo_page::LfoWave; 4],
    /// The macros (decisions.md R-MACROS): the left area's renderer, what it shows, and the
    /// editor's state of them (a name typed, the part under the pointer).
    macros: MacroRenderer,
    macro_scene: MacroScene,
    macro_view: macros::MacroView,
    playing: Playing,
    #[cfg(feature = "standalone-production")]
    standalone_dialog: Option<crate::standalone_settings::Dialog>,
    /// AI agents operating the editor through MCP, while allowed (`editor/agent.rs`;
    /// decisions.md R-MCP).
    agents: agent::Agents,
    /// The standalone's audio threads not real-time, said (decisions.md R-RTKIT).
    realtime: Realtime,
}

/// What the standalone's editor says of its audio threads' priority (decisions.md R-RTKIT): read
/// from the audio side's atomics at each frame (`read`: the tests give their own), the warning
/// while one is due, and when the walnut began saying it ([`REALTIME_NOTICE`], once).
#[derive(Debug)]
struct Realtime {
    read: fn() -> (ca72_rt::Standing, ca72_rt::Standing),
    warning: Option<crate::audio_thread::Warning>,
    since: Option<Instant>,
}

impl Default for Realtime {
    fn default() -> Self {
        Self {
            read: crate::audio_thread::standing,
            warning: None,
            since: None,
        }
    }
}

/// How long the walnut says, once, that the standalone's audio is not real-time.
const REALTIME_NOTICE: Duration = Duration::from_secs(15);

impl PanelWindow {
    /// The editor's window: `width` logical pixels wide, `dpr` physical pixels a logical one,
    /// the presets from `library` (the user's, but in the tests).
    #[allow(clippy::too_many_arguments)]
    fn new(
        window: &mut Window<'_>,
        context: Arc<dyn GuiContext>,
        params: Arc<Ca72Params>,
        meters: Arc<Meters>,
        width: u32,
        dpr: f64,
        library: Library,
        host_keys: Arc<HostKeys>,
    ) -> Self {
        let height = height_in(
            width,
            params.editor_panel.load(Ordering::Relaxed),
            chosen_panel(),
        );
        let physical = (
            (f64::from(width) * dpr).round() as u32,
            (f64::from(height) * dpr).round() as u32,
        );
        let surface = window::Surface::new(window, physical);
        PanelWindow {
            surface,
            physical,
            logical: (width, height),
            shown: None,
            settle: None,
            hidden_drawn: None,
            editing: Editing::new(
                context,
                params,
                meters,
                (width, physical.0, dpr),
                library,
                |k| screen::usable(Some(k)),
            ),
            keys: None,
            host_keys,
            twins: KeyTwins::default(),
            cursor: MouseCursor::Default,
        }
    }

    /// A key, the window's own or the host's: the editor's to act on (whether it took it), the
    /// window's size and keyboard following.
    fn key(&mut self, window: &mut Window<'_>, k: &KeyboardEvent) -> bool {
        let taken = self.editing.keyed(k);
        let height_changed = self.follow(window);
        self.keyboard(window);
        self.repaint_height_change(window, height_changed);
        taken
    }

    /// The window now `info`'s size: the surface and every frame drawn to it.
    fn resized(&mut self, info: &WindowInfo) {
        let size = info.physical_size();
        let physical = (size.width.max(1), size.height.max(1));
        let logical = info.logical_size();
        self.logical = (logical.width.round() as u32, logical.height.round() as u32);
        if physical == self.physical && info.scale() == self.editing.dpr && self.shown.is_some() {
            return;
        }
        self.physical = physical;
        self.editing.fit(
            info.scale(),
            info.logical_size().width.round() as u32,
            physical.0,
        );
        self.surface.resize(self.physical);
        self.shown = None;
    }

    /// The window resized to what the editor asks (the grip, the drawer below the bar),
    /// when it asked: on macOS baseview reports no resize of its own making, so the frames
    /// follow here; elsewhere the report that follows finds the size already taken. Whether
    /// the window took a new height at the width it had.
    fn follow(&mut self, window: &mut Window<'_>) -> bool {
        if !std::mem::take(&mut self.editing.resizing_window) {
            return false;
        }
        let want = (self.editing.width, self.editing.window_height());
        if want == self.logical {
            return false;
        }
        let height_only = want.0 == self.logical.0;
        let size = Size::new(f64::from(want.0), f64::from(want.1));
        window.resize(size);
        self.resized(&WindowInfo::from_logical_size(size, self.editing.dpr));
        height_only
    }

    /// On X11 the next frame can be 15 ms away: a height the window has just taken (PANEL,
    /// the drawer) is drawn and shown whole at the end of the event that took it, not left
    /// to the compositor as the old image (decisions.md R-LINUX-PANEL-REPAINT). Outside
    /// `resized` and `draw`: a draw can reconcile a host's delayed refusal, which the
    /// window follows before anything is shown. The grip's widths keep the frames' pace.
    fn repaint_height_change(&mut self, window: &mut Window<'_>, height_changed: bool) {
        if !cfg!(target_os = "linux") || !height_changed {
            return;
        }
        self.editing.draw();
        self.follow(window);
        self.keyboard(window);
        self.present(false);
        let now = Instant::now();
        self.shown = Some(now);
        self.settle = SETTLE.map(|for_| now + for_);
    }

    /// The pointer's look: the resize arrows over the grip and while it is dragged (R-SIZE),
    /// else the system's (on macOS through baseview's change 7, `R-MAC-FIXES`).
    fn cursor(&mut self, window: &mut Window<'_>) {
        let want = if self.editing.on_grip() {
            MouseCursor::NwseResize
        } else {
            MouseCursor::Default
        };
        if want == self.cursor {
            return;
        }
        self.cursor = want;
        window.set_mouse_cursor(want);
    }

    /// The keyboard as the drawer wants it: taken when a field takes the caret (baseview's
    /// focus; not on X11, where baseview has none and the keys go to the window under the
    /// pointer), and on Windows given back to the window that had it as the drawer shuts
    /// (baseview there keeps every key its window is given, the host's shortcuts too:
    /// decisions.md R18).
    fn keyboard(&mut self, window: &mut Window<'_>) {
        // (MIDI Learn too, while a control's menu is open or a controller is awaited: Escape
        // closes or cancels it; decisions.md R34.)
        let asked = std::mem::take(&mut self.editing.browser.wants_keys)
            | std::mem::take(&mut self.editing.learning.take_keys)
            | std::mem::take(&mut self.editing.typing_keys)
            | std::mem::take(&mut self.editing.playing.wants_keys);
        if asked {
            let had = window::take_keys(window);
            if self.keys.is_none() {
                self.keys = had;
            }
        }
        let holding = self.editing.holds_keys();
        // (And the keys the host gives the view: the editor's while it holds them.)
        self.host_keys.holding.store(holding, Ordering::Release);
        self.host_keys
            .navigation
            .store(self.editing.preset_keys_allowed(), Ordering::Release);
        // Every key while it holds them, those a host's dialog keeps for itself too (Windows:
        // in REAPER's FX window an arrow moved REAPER's focus to one of its own buttons); the
        // host's again once it lets them go (decisions.md R34).
        window.set_wants_keys(holding);
        if !holding && let Some(keys) = self.keys.take() {
            window::give_keys_back(window, keys);
        }
    }

    /// The frame shown: where it changed in the last `draw` when `partial`, or all of it.
    fn present(&mut self, partial: bool) {
        let e = &self.editing;
        let frame = e.composed.as_ref().unwrap_or_else(|| e.renderer.frame());
        let damage = e.damage.as_deref().filter(|_| partial);
        self.surface.present(self.physical, &[frame], damage);
    }
}

impl Editing {
    /// An editor `width` logical pixels wide, `physical_width` physical ones, `dpr`
    /// physical pixels a logical one; `screen` the usable screen at a scale.
    fn new(
        context: Arc<dyn GuiContext>,
        params: Arc<Ca72Params>,
        meters: Arc<Meters>,
        (width, physical_width, dpr): (u32, u32, f64),
        library: Library,
        screen: fn(f64) -> Option<(f64, f64)>,
    ) -> Self {
        // (The tests' editors keep nothing on the computer.)
        params.editor_presets.store(false, Ordering::Relaxed);
        params.editor_panel_applied.store(
            params.editor_panel.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        params.editor_expanded.store(
            params.editor_panel.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        params.editor_resizing.opening(crate::resizing::View {
            width,
            saved_width: params.editor_width.load(Ordering::Relaxed),
            expanded: params.editor_expanded.load(Ordering::Relaxed),
            panel: params.editor_panel.load(Ordering::Relaxed),
            applied: params.editor_panel_applied.load(Ordering::Relaxed),
            presets: false,
        });
        let settings = if cfg!(test) {
            Settings::default()
        } else {
            Settings::load()
        };
        let installed = if settings.pack.is_some() {
            installed()
        } else {
            Vec::new()
        };
        params.set_freq_law(settings.frequency);
        params.midi_map.set_wheel_source(settings.mod_wheel_source);
        params.set_tempo(f64::from(settings.tempo));
        // (MPE's, as another editor may have chosen them since: decisions.md R-MPE.)
        params.set_mpe_mode(settings.mpe);
        params.set_timbre_rest(settings.timbre_rest);
        let standalone = context.plugin_api() == PluginApi::Standalone;
        let mut playing = Playing::default();
        playing.standalone = standalone;
        playing.visible = settings.keyboard.unwrap_or(standalone);
        playing.root = settings.keyboard_root;
        playing.typing_root = settings.typing_root;
        let freq_knobs = [Pot::Two, Pot::Three].map(|pot| FreqKnob {
            params: Arc::clone(&params),
            pot,
        });
        let waveforms = [0, 1, 2].map(|n| WaveformSwitch {
            params: Arc::clone(&params),
            n,
        });
        let lfo_rates = [0, 1, 2, 3].map(|n| lfo_page::LfoRate {
            params: Arc::clone(&params),
            n,
        });
        let lfo_waves = [0, 1, 2, 3].map(|n| lfo_page::LfoWave {
            params: Arc::clone(&params),
            n,
        });
        let watched = Watched::new(Arc::clone(&context), &params);
        // The standalone's effects (decisions.md R-FX-DRAWER): the plug-ins have none.
        let effects = if standalone {
            crate::fx_view::installed()
        } else {
            None
        };
        let mut editing = Editing {
            context,
            params,
            meters,
            dpr,
            width,
            physical_width,
            renderer: Renderer::printing(
                chosen_look(&settings, &installed),
                settings.lettering,
                settings.ticks_titles,
                f64::from(physical_width) / art::W,
                dpr,
            ),
            installed,
            scene: Scene::default(),
            strip: StripRenderer::new(f64::from(physical_width) / art::W),
            strip_scene: StripScene::default(),
            last: [0.5; 4],
            drawer: DrawerRenderer::new(f64::from(physical_width) / art::W),
            browser: Browser::new(library),
            arp: ArpRenderer::new(
                f64::from(physical_width) / art::W,
                settings.skin,
                settings.lettering,
            ),
            arp_scene: ArpScene::default(),
            lower: Dropped::default(),
            lower_shown: (0.0, Lower::Arp),
            last_arp_press: None,
            mpe: MpeRenderer::new(
                f64::from(physical_width) / art::W,
                settings.skin,
                settings.lettering,
            ),
            mpe_scene: MpeScene::default(),
            last_mpe_press: None,
            heard: ((0, 0), None, None),
            fx_view: FxView::new(None),
            fx: None,
            fx_scene: FxScene::default(),
            fx_pin_drag: None,
            rail: Rail::Plugin,
            tempo_drag: None,
            last_tempo_press: None,
            tempo_hover: None,
            panic_held: false,
            mapping: mapping::Mapping::default(),
            live: [None; LIVE.len()],
            live_tempo: None,
            live_due: None,
            tempo_travel: 0.0,
            composed: None,
            damage: None,
            resizing_window: false,
            panel_due: false,
            placed: false,
            update: Update::default(),
            learning: Learning::default(),
            pointer: (0.0, 0.0),
            hover: None,
            drag: None,
            scroll: None,
            precise_lines: PRECISE_LINES,
            list_travel: 0.0,
            last_press: None,
            resizing: None,
            gripped: None,
            screen,
            watched,
            reveal_shown: 0.0,
            motion: Motion::default(),
            motion_at: None,
            motion_due: None,
            levels: [0.0; LEVELS],
            output: 0.0,
            settings,
            settings_file: (!cfg!(test)).then(crate::settings::file),
            ultra_note: false,
            whats_new: None,
            whats_new_checked: false,
            typing: None,
            knob_entry: None,
            typing_keys: false,
            last_strip_press: None,
            freq_knobs,
            waveforms,
            lfo: LfoRenderer::new(
                f64::from(physical_width) / art::W,
                settings.skin,
                settings.lettering,
            ),
            lfo_scene: LfoScene::default(),
            modulation: ModRenderer::new(f64::from(physical_width) / art::W),
            mod_scene: ModScene::default(),
            modulating: lfo_page::Modulating::default(),
            lfo_rates,
            lfo_waves,
            macros: MacroRenderer::new(),
            macro_scene: MacroScene::default(),
            macro_view: macros::MacroView::default(),
            playing,
            #[cfg(feature = "standalone-production")]
            standalone_dialog: None,
            agents: agent::Agents::default(),
            realtime: Realtime::default(),
        };
        editing.use_effects(FxView::new(effects));
        editing.opened().listening()
    }

    /// The effects' drawer for `view`'s chain: with one, its renderer (its still parts begun on
    /// their thread) and the rail's FX key; without, the plug-ins' rail.
    fn use_effects(&mut self, view: FxView) {
        self.rail = if view.available() {
            Rail::Effects
        } else {
            Rail::Plugin
        };
        self.fx = view.available().then(|| {
            FxRenderer::new(
                self.renderer.scale(),
                self.renderer.skin(),
                self.renderer.lettering(),
            )
        });
        self.browser.effects = view.effects();
        if let Some(effects) = &self.browser.effects
            && !self
                .params
                .preset
                .read()
                .map(|s| s.is_empty())
                .unwrap_or(true)
        {
            effects.observe_preset(
                self.params
                    .preset_effects
                    .read()
                    .map(|s| s.clone())
                    .unwrap_or_default(),
            );
        }
        self.fx_view = view;
        self.strip.set_rail(self.rail);
        self.learning.rail = self.rail;
        self.composed = None;
    }

    /// The settings' menu, dropping down from under the rail's settings key to its left.
    fn open_settings(&mut self) {
        let size = self.renderer.text_size() * MENU_TEXT;
        // The packs installed now (one may have been added since the editor opened).
        self.installed = installed();
        let packs: Vec<(String, bool)> = self
            .installed
            .iter()
            .map(|p| (p.name.clone(), p.look.is_some()))
            .collect();
        let shown = &self.renderer.look().id;
        let pack = self
            .installed
            .iter()
            .position(|p| p.look.as_ref().is_some_and(|l| &l.id == shown));
        let agents = self.agents_menu();
        self.learning.open_settings_menu(
            self.renderer.fonts(),
            settings_menu_corner(),
            size,
            (
                self.settings.skin,
                (self.settings.lettering, self.settings.ticks_titles),
                self.settings.frequency,
                self.params.midi_map.wheel_source(),
                agents,
                self.settings.hover_cables,
            ),
            (&packs, pack),
            self.realtime.warning.as_ref().map(|w| w.row),
        );
    }

    /// Listening for agents if they are allowed.
    fn listening(mut self) -> Self {
        self.sync_agents();
        self
    }

    /// Agents allowed or not from now on (SETTINGS: AGENTS), on this computer; off, for this
    /// session too (`--agents`).
    fn set_agents(&mut self, on: bool) {
        self.keep_settings(Change::Agents(on));
        if !on {
            crate::agent::force(false);
        }
        self.agents.stop = false;
        self.agents.error = None;
        self.sync_agents();
    }

    /// As it opens: what QUALITY's setting shows up in its opening, not moving (the hamster
    /// asleep); it watches the levels, and the voices' peaks held while it was shut are let go.
    fn opened(mut self) -> Self {
        // The window opened at the panel's size last chosen; the look's may differ.
        self.strip.set_look(self.renderer.look());
        self.drawer.set_look(self.renderer.look());
        self.dress_drawers();
        self.strip.set_under_top(self.renderer.top_px());
        if self.panel() != chosen_panel() {
            choose_panel(self.panel());
            self.panel_due = true;
        }
        self.meters.watchers.fetch_add(1, Ordering::Relaxed);
        let _ = self.meters.take_levels();
        self.motion = Motion::at(self.params.quality.value(), 0.0);
        self.scene.opening = self.motion.opening(!self.settings.shutter);
        self
    }

    /// Called only by an editor window's frame, once it is actually visible. Neither
    /// constructing the plug-in/editor nor drawing a hidden editor consumes the claim.
    fn release_notes_when_visible(&mut self, visible: bool) {
        if !visible || self.whats_new_checked {
            return;
        }
        self.whats_new_checked = true;
        if let Some(path) = &self.settings_file {
            match crate::settings::claim_release_notes(path, crate::update::VERSION) {
                Ok(Some(notes)) => self.show_release_notes(notes),
                Ok(None) => {}
                Err(error) => nih_log!("Could not keep What's New history: {error}"),
            }
        }
    }

    fn show_release_notes(&mut self, notes: crate::whats_new::Notes) {
        self.whats_new = Some(notes.view());
        let height = f64::from(self.physical_height()) / self.renderer.scale();
        if let Some(view) = &mut self.whats_new {
            view.fit(self.renderer.fonts(), height, self.settings.lettering);
        }
        self.list_travel = 0.0;
        self.typing_keys = true;
        self.composed = None;
    }

    /// The editor drawn in `skin` from now on, on this computer (decisions.md R51): its
    /// panel's renderer made again in it (its pictures decoded as it first draws).
    fn set_look(&mut self, skin: Skin) {
        self.keep_settings(Change::Skin(skin));
        let look = Look::of(skin);
        if self.renderer.look().id != look.id {
            self.draw_in(look);
        }
    }

    /// The editor drawn in installed pack `i`'s look from now on, on this computer
    /// (decisions.md R-SKIN-PACKS).
    fn set_pack(&mut self, i: usize) {
        let Some(look) = self.installed.get(i).and_then(|p| p.look.clone()) else {
            return;
        };
        if let Some(id) = PackId::new(&look.id) {
            self.keep_settings(Change::Pack(id, look.base));
        }
        if self.renderer.look().id != look.id {
            self.draw_in(look);
        }
    }

    /// The panel's renderer made again in `look` (its pictures decoded as it first draws).
    ///
    /// A look whose panel is another size than the one shown asks the window to follow it, its
    /// width kept (decisions.md R-SKIN-PACKS); the host refusing, the look shown stays.
    fn draw_in(&mut self, look: Look) {
        let lettering = self.renderer.lettering();
        let before = self.panel();
        let renderer = Renderer::printing(
            look,
            lettering,
            self.settings.ticks_titles,
            self.renderer.scale(),
            self.dpr,
        );
        let was = std::mem::replace(&mut self.renderer, renderer);
        if self.panel() != before {
            if !self.ask_resize(self.width) {
                self.renderer = was;
                return;
            }
            choose_panel(self.panel());
            self.resizing_window = true;
        }
        self.strip.set_look(self.renderer.look());
        self.drawer.set_look(self.renderer.look());
        self.dress_drawers();
        self.strip.set_under_top(self.renderer.top_px());
        self.composed = None;
    }

    /// The drawers dropped from the rail (the arpeggiator's, the effects' and the expression's)
    /// drawn in the look's base skin and lettering: a pack's look dresses none (decisions.md
    /// R-SKIN-PACKS), so they draw its base's built-in artwork.
    fn dress_drawers(&mut self) {
        let (skin, lettering) = (self.renderer.skin(), self.renderer.lettering());
        self.arp.set_look(skin, lettering);
        self.mpe.set_look(skin, lettering);
        self.lfo.set_look(skin, lettering);
        if let Some(fx) = &mut self.fx {
            fx.set_look(skin, lettering);
        }
    }

    /// The window asked to follow the look's panel as it opened; the host refusing, the look's
    /// base drawn instead (its panel the official one, as the window is).
    fn follow_panel(&mut self) {
        if !std::mem::take(&mut self.panel_due) {
            return;
        }
        if self.ask_resize(self.width) {
            self.resizing_window = true;
        } else {
            let base = Look::of(self.renderer.look().base);
            let lettering = self.renderer.lettering();
            self.renderer = Renderer::printing(
                base,
                lettering,
                self.settings.ticks_titles,
                self.renderer.scale(),
                self.dpr,
            );
            choose_panel(self.panel());
            self.strip.set_look(self.renderer.look());
            self.drawer.set_look(self.renderer.look());
            self.dress_drawers();
        }
        self.strip.set_under_top(self.renderer.top_px());
        self.composed = None;
    }

    /// The SIZE menu's sizes (R-SIZE): [`SIZES`], then FIT TO SCREEN, the size the editor
    /// opens at on this screen; each its label, its width (logical pixels) and whether the
    /// window at it, as it is now (the panel shown or not), fits the screen (all, where the
    /// screen is not known).
    fn sizes(&self) -> Vec<(&'static str, u32, bool)> {
        let screen = (self.screen)(self.dpr);
        let expanded = self.expanded();
        let fits = |w: u32| {
            screen.is_none_or(|(sw, sh)| {
                f64::from(w) <= sw && f64::from(height_in(w, expanded, self.panel())) <= sh
            })
        };
        let mut sizes: Vec<_> = SIZES
            .iter()
            .map(|&(t, k)| {
                let w = clamp_width((f64::from(DEFAULT_WIDTH) * k).round() as u32);
                (t, w, fits(w))
            })
            .collect();
        sizes.push(("FIT TO SCREEN", fitted_width(screen), true));
        sizes
    }

    /// The editor's lettering printed at `lettering`'s sizes from now on, on this computer
    /// (R-LETTERING): its panel's renderer made again with them, in the look it is in (the
    /// tip's, notes' and menus' lettering follow it).
    fn set_lettering(&mut self, lettering: Lettering) {
        self.keep_settings(Change::Lettering(lettering));
        if self.renderer.lettering() != lettering {
            let look = self.renderer.look().clone();
            self.renderer = Renderer::printing(
                look,
                lettering,
                self.settings.ticks_titles,
                self.renderer.scale(),
                self.dpr,
            );
            self.dress_drawers();
            self.composed = None;
        }
    }

    /// The panel's ticks and sections' titles printed as `print` has them from now on, on this
    /// computer (decisions.md R-QUIET-PRINT): its renderer made again, in the look and lettering
    /// it is in. A look that prints them quiet itself stays quiet.
    fn set_ticks_titles(&mut self, print: TicksTitles) {
        self.keep_settings(Change::TicksTitles(print));
        let wanted = if self.renderer.look().ticks_titles == TicksTitles::Quiet {
            TicksTitles::Quiet
        } else {
            print
        };
        if self.renderer.print() != wanted {
            let look = self.renderer.look().clone();
            let lettering = self.renderer.lettering();
            self.renderer =
                Renderer::printing(look, lettering, print, self.renderer.scale(), self.dpr);
            self.composed = None;
        }
    }

    fn standalone_dialog_open(&self) -> bool {
        #[cfg(feature = "standalone-production")]
        {
            self.standalone_dialog.is_some()
        }
        #[cfg(not(feature = "standalone-production"))]
        {
            false
        }
    }

    /// Whether `k` quits the standalone (R-QUIT): the system's shortcut, in the standalone only
    /// (a host's shortcuts are its own).
    fn quits(&self, k: &KeyboardEvent) -> bool {
        #[cfg(feature = "standalone-production")]
        return self.playing.standalone && nih_plug::wrapper::production::app_window::quits(k);
        #[cfg(not(feature = "standalone-production"))]
        {
            let _ = k;
            false
        }
    }

    fn preset_keys_allowed(&self) -> bool {
        self.playing.focused
            && !self.browser.open
            && self.typing.is_none()
            && self.macro_view.typing.is_none()
            && self.knob_entry.is_none()
            && !self.learning.holds_keys(&self.params.midi_map)
            && !self.fx_view.list_open()
            && !self.standalone_dialog_open()
            && self.whats_new.is_none()
    }

    fn holds_keys(&self) -> bool {
        self.browser.open
            || self.learning.holds_keys(&self.params.midi_map)
            || self.mapping_on()
            || self.typing.is_some()
            || self.macro_view.typing.is_some()
            || self.knob_entry.is_some()
            || self.fx_view.list_open()
            || self.note_typing_allowed()
            || self.standalone_dialog_open()
            || self.whats_new.is_some()
    }

    fn note_typing_allowed(&self) -> bool {
        self.playing.holds_keys()
            && !self.browser.open
            && !self.browser.sliding()
            && self.typing.is_none()
            && self.macro_view.typing.is_none()
            && self.knob_entry.is_none()
            && !self.learning.holds_keys(&self.params.midi_map)
            && !self.fx_view.list_open()
            && !self.standalone_dialog_open()
            && self.whats_new.is_none()
    }

    /// What moves the MOD wheel from now on, on this computer (R-MODWHEEL-SOURCE): learning
    /// ends, the wheel's CC is reserved, and the setting kept.
    fn set_wheel_source(&mut self, source: crate::mod_wheel::Source) {
        self.params.midi_map.cancel();
        self.params.midi_map.set_wheel_source(source);
        self.keep_settings(Change::ModWheelSource(source));
    }

    /// MPE's zones from `mode` from now on, on this computer (decisions.md R-MPE): the
    /// controller's MPE Configuration Message (AUTO), a lower zone of fifteen channels, or none.
    fn set_mpe_input(&mut self, mode: crate::expression::Mode) {
        self.keep_settings(Change::Mpe(mode));
        self.params.set_mpe_mode(mode);
    }

    /// Timbre (CC 74, a host's brightness) resting at `rest` from now on, on this computer
    /// (decisions.md R-MPE).
    fn set_timbre_rest(&mut self, rest: crate::expression::TimbreRest) {
        self.keep_settings(Change::TimbreRest(rest));
        self.params.set_timbre_rest(rest);
    }

    /// The FREQUENCY knobs read through `law` from now on, on this computer (decisions.md
    /// R54): each points where its pot is through it, the sound as it was.
    fn set_frequency_dials(&mut self, law: Law) {
        self.keep_settings(Change::Frequency(law));
        self.params.set_freq_law(law);
    }

    /// `change` made to this computer's settings: this editor's copy, and that key alone in the
    /// file (not by the tests' editors), so that another editor's changes since this one opened
    /// stay as they are.
    fn keep_settings(&mut self, change: Change) {
        self.settings.apply(change);
        if let Some(path) = &self.settings_file {
            let _ = Settings::keep(path, change);
        }
    }

    fn poll_midi_learn(&mut self) {
        if self
            .learning
            .poll(&self.params.midi_map)
            .is_some_and(|a| a.param == crate::learn::MOD_WHEEL)
        {
            self.keep_settings(Change::ModWheelSource(self.params.midi_map.wheel_source()));
        }
    }

    /// QUALITY's opening moved on towards what its setting `q` shows, by the time since it
    /// last was (at most 50 ms of it: a stalled frame does not jump it), and the voices' levels
    /// taken: at most [`ANIMATE_S`] apart (thirty times a second, whatever the screen's rate).
    /// ULTRA's note shown once its lamp is up, until it is read.
    fn move_opening(&mut self, now: Instant, q: QualityMode) {
        if self.motion_due.is_some_and(|due| now < due) {
            return;
        }
        let tick = Duration::from_secs_f64(ANIMATE_S);
        self.motion_due = Some(match self.motion_due {
            Some(due) => (due + tick).max(now),
            None => now + tick,
        });
        let since = self
            .motion_at
            .map(|t| now.saturating_duration_since(t).as_secs_f64());
        let (peaks, output) = self.meters.take_levels();
        self.levels = peaks.map(level_of);
        self.output = level_of(output);
        if self.settings.shutter {
            self.motion
                .step(since.unwrap_or(0.0).min(0.05), q, self.output);
        } else {
            // (None of it shown: what the setting shows kept up, so that shown again it is
            // there, still.)
            self.motion = Motion::at(q, self.output);
        }
        self.motion_at = Some(now);
        let on = q == QualityMode::Ultra;
        if on && self.motion.coil == 1.0 && !self.settings.ultra_note_read && !self.ultra_note {
            // (Read in another editor since this one opened: not shown again, once a computer.)
            let read = self
                .settings_file
                .as_ref()
                .is_some_and(|p| Settings::read(p).ultra_note_read);
            if read {
                self.settings.apply(Change::NoteRead);
            } else {
                self.ultra_note = true;
            }
        }
        if !on {
            self.ultra_note = false;
        }
        self.scene.opening = self.motion.opening(!self.settings.shutter);
    }

    /// ULTRA's note: what it is for (the owner: "only intended for offline renders, or small
    /// projects on powerful PCs") and what it costs, as this computer plays it (whether it
    /// splits an ULTRA voice over six threads: `engine::split_voices`, decisions.md R-THREADS),
    /// left of its lamp; a press anywhere closes it, read.
    fn ultra_note(&self) -> Note {
        self.ultra_note_for(crate::engine::split_voices() > 0)
    }

    /// [`Editing::ultra_note`] for a computer that splits an ULTRA voice (`splits`) or not.
    fn ultra_note_for(&self, splits: bool) -> Note {
        let size = self.renderer.text_size();
        let lines = ultra_note_lines(splits)
            .iter()
            .map(|&(t, tone)| (t.to_owned(), tone))
            .collect();
        let mut note = Note {
            x: 0.0,
            y: 0.0,
            size,
            lines,
        };
        let (w, h) = note.extent(self.renderer.fonts());
        // (Its box's right edge a little left of the lamp's column, its top at the lamp's: where
        // the layout has QUALITY's opening.)
        let (ux, uy) = self.renderer.arrangement().ultra;
        note.x = art::COL + ux - 83.0 - w / 2.0;
        note.y = art::TOP + uy - 52.0 + h + size * 0.4;
        note
    }

    /// A change of DRIVE or AUTO GAIN made here done (nothing held, no wheel's gesture open):
    /// AUTO GAIN's curve asked for the sound as it now is, measured unless it is already its
    /// (decisions.md R45, the CA-74's R29). Nothing else asks (R-AUTO-GAIN-DRIVE, the owner:
    /// "This should only be remeasured when drive changes"): a curve that arrives while a note
    /// plays moves its level, so another control, a preset, a key or the window opening never
    /// starts one; nor do the host's automation and learned controllers, so that a render does
    /// not depend on when a measurement finished. Without DRIVE or with AUTO GAIN off nothing
    /// is measured (the curve does nothing there).
    fn measure_when_done(&mut self) {
        if self.dragging() || self.scroll.is_some() || !self.watched.take_moved() {
            return;
        }
        // (DRIVE and AUTO GAIN as set here: the host may not have taken them yet. Neither is
        // part of the sound measured.)
        let (drive, auto) = self.watched.drive_and_auto_gain(&self.params);
        if auto && drive > 0.0 {
            self.params.drive_curve.ask(&self.params.controls());
        }
    }

    /// The renderers at the window's scale: `dpr` physical pixels a logical one, `width`
    /// logical pixels wide, `physical_width` physical ones.
    fn fit(&mut self, dpr: f64, width: u32, physical_width: u32) {
        // A height-only snap keeps the scale and the frame just composed. In particular,
        // state recall can ask for that height during draw(), before the window follows it.
        if self.dpr == dpr && self.width == width && self.physical_width == physical_width {
            return;
        }
        self.dpr = dpr;
        self.width = width;
        self.physical_width = physical_width;
        let scale = f64::from(physical_width) / art::W;
        self.renderer.rescale(scale, dpr);
        self.strip.rescale(scale);
        self.strip.set_under_top(self.renderer.top_px());
        self.drawer.rescale(scale);
        self.arp.rescale(scale);
        self.mpe.rescale(scale);
        self.lfo.rescale(scale);
        if let Some(fx) = &mut self.fx {
            fx.rescale(scale);
        }
        self.composed = None;
    }

    fn setter(&self) -> ParamSetter<'_> {
        ParamSetter::new(&self.watched)
    }

    /// The pointer's place in the drawing.
    fn in_drawing(&self, (x, y): (f64, f64)) -> (f64, f64) {
        let k = self.dpr / self.renderer.scale();
        (x * k, y * k + self.renderer.top())
    }

    /// The compact view's grip is on the rail; controls outside the view cannot be hit.
    fn hit(&self, x: f64, y: f64) -> Option<Target> {
        if !self.expanded() {
            let bottom =
                f64::from(self.physical_height()) / self.renderer.scale() + self.renderer.top();
            if !(0.0..bottom).contains(&y) {
                return None;
            }
            if x >= art::W - COMPACT_GRIP && y >= bottom - COMPACT_GRIP {
                return Some(Target::Grip);
            }
            if y >= DRAWER_TOP {
                return None;
            }
        }
        // (The macros' knobs while they show, in the wheels' place: decisions.md R-MACROS.)
        if self.macros_show()
            && let Some(t) = interact::hit_macro(x, y)
        {
            return Some(t);
        }
        // (A control off the panel is not there to find.)
        interact::hit(self.renderer.layout(), self.renderer.arrangement(), x, y)
            .filter(|t| !matches!(t, Target::Control(i) if self.off_panel(*i)))
    }

    /// Whether control `i` is off the panel now: a TIMED key away from its RANGE's LO
    /// (decisions.md R-OSC-TIMED; the owner: "they should disappear ... if it's not on low
    /// mode"), or a MOD key while its oscillator's waveform is STEPPED (R-CONTINUOUS-WAVE). It is
    /// neither drawn nor found by the pointer (no press, wheel, tip or menu), nor ringed while
    /// MIDI Learn learns it from the list.
    fn off_panel(&self, i: usize) -> bool {
        CONTROLS.get(i).is_some_and(|c| {
            let at = |keys: &[&str]| keys.iter().position(|k| *k == c.param);
            // (The LFO page's while it does not show: R-LFO.)
            (c.place == DrawerPlace::Lfo && !self.page_shows())
                // (The wheels and their switches while the macros show in their place, and the
                // macros while they do not: R-MACROS.)
                || (c.place == DrawerPlace::Column && self.macros_show())
                || (c.place == DrawerPlace::Macro && !self.macros_show())
                || at(&ca72_panel::render::TIMED_KEYS)
                    .is_some_and(|k| timed_inert(&self.params, k + 1))
                || at(&ca72_panel::render::MOD_KEYS).is_some_and(|n| !self.params.continuous(n))
        })
    }

    /// The drawer's top now (drawing units) and where it is cut off, while it shows: dropping
    /// down from under the rail over the strip's sections (A6), the window as it is.
    fn drawer_span(&self) -> Option<(f64, f64, f64)> {
        let k = self.browser.reveal();
        if k <= 0.0 {
            return None;
        }
        Some((DRAWER_TOP - (1.0 - k) * DRAWER_H, DRAWER_TOP, art::H))
    }

    /// The pointer's place in the drawer (drawer units), if it is over it.
    fn in_drawer(&self, (x, y): (f64, f64)) -> Option<(f64, f64)> {
        let (top, from, to) = self.drawer_span()?;
        (y >= from && y < to).then_some((x, y - top))
    }

    /// The drawer dropped from the rail: its top now (drawing units), while it shows, dropping
    /// down from under the rail, over the lower page.
    fn lower_depth(&self) -> f64 {
        if self.lower.which == Lower::Fx {
            self.fx_scene.depth()
        } else {
            self.lower.which.depth()
        }
    }

    fn lower_top(&self) -> Option<f64> {
        let k = self.lower.reveal();
        (k > 0.0 && self.expanded()).then_some(lower_ui::TOP - (1.0 - k) * self.lower_depth())
    }

    /// Whether the pointer (the drawing's units) is over the drawer dropped from the rail, as much
    /// of it as shows, or the effects' list hanging from it (the presets' drawer, over it, apart).
    fn in_lower(&self, (x, y): (f64, f64)) -> bool {
        self.lower_top().is_some_and(|top| {
            let plate = (lower_ui::LEFT..lower_ui::RIGHT).contains(&x)
                && y >= DRAWER_TOP
                && y < top + self.lower_depth();
            let list = self.lower.which == Lower::Fx
                && self.lower.settled()
                && self
                    .fx_scene
                    .list
                    .as_ref()
                    .is_some_and(|l| fx_ui::list_hit(l, x, y).is_some());
            plate || list
        }) && self.in_drawer((x, y)).is_none()
    }

    /// Whether the pointer is over the arpeggiator's drawer.
    fn in_arp(&self, at: (f64, f64)) -> bool {
        self.lower.which == Lower::Arp && self.in_lower(at)
    }

    /// Whether the pointer is over the expression drawer.
    fn in_mpe(&self, at: (f64, f64)) -> bool {
        self.lower.which == Lower::Mpe && self.in_lower(at)
    }

    /// Whether the pointer is over the effects' drawer or its list.
    fn in_fx(&self, at: (f64, f64)) -> bool {
        self.lower.which == Lower::Fx && self.in_lower(at)
    }

    fn expanded(&self) -> bool {
        self.params.editor_expanded.load(Ordering::Relaxed)
    }

    /// The window's height (logical pixels): the synth and rail, with the lower panel shown.
    fn window_height(&self) -> u32 {
        height_in(self.width, self.expanded(), self.panel())
    }

    /// The panel's size, its look's.
    fn panel(&self) -> art::Panel {
        self.renderer.arrangement().panel
    }

    fn resize_view(&self) -> crate::resizing::View {
        crate::resizing::View {
            width: self.width,
            saved_width: self.params.editor_width.load(Ordering::Relaxed),
            expanded: self.expanded(),
            panel: self.params.editor_panel.load(Ordering::Relaxed),
            applied: self.params.editor_panel_applied.load(Ordering::Relaxed),
            presets: self.browser.open,
        }
    }

    fn reconcile_resize(&mut self) {
        self.params.editor_resizing.finish(self.resize_view());
        // Keep only the final accepted width after a grip drag, without disk writes each frame.
        if self.resizing.is_none() {
            self.remember_width();
        }
        if let Some(view) = self.params.editor_resizing.take_restore() {
            if view == self.resize_view() {
                return;
            }
            // draw() can consume a delayed refusal before the native window follows it.
            // Refit now so that following the restored width preserves the composed frame.
            let physical = WindowInfo::from_logical_size(
                Size::new(
                    f64::from(view.width),
                    f64::from(height_in(view.width, view.expanded, self.panel())),
                ),
                self.dpr,
            )
            .physical_size();
            self.fit(self.dpr, view.width, physical.width);
            self.params
                .editor_width
                .store(view.saved_width, Ordering::Relaxed);
            self.params
                .editor_expanded
                .store(view.expanded, Ordering::Relaxed);
            self.params
                .editor_panel
                .store(view.panel, Ordering::Relaxed);
            self.params
                .editor_panel_applied
                .store(view.applied, Ordering::Relaxed);
            self.browser.show(view.presets, &self.params);
            self.placed = view.presets;
            self.params
                .editor_presets
                .store(view.presets, Ordering::Relaxed);
            if !view.expanded {
                self.playing.release_pointer();
                self.playing.hover = None;
            }
            self.playing.wants_keys = self.playing.holds_keys();
            self.typing = None;
            self.knob_entry = None;
            self.hover = None;
            self.strip_scene.hover = None;
            self.learning.fit_menu(
                self.renderer.fonts(),
                [
                    0.0,
                    self.renderer.top(),
                    art::W,
                    if view.expanded { art::H } else { DRAWER_TOP },
                ],
            );
            self.composed = None;
            self.resizing_window = true;
        }
    }

    fn remember_width(&mut self) {
        if let Some(width) = self.params.editor_resizing.take_remembered_width() {
            self.keep_settings(Change::EditorWidth(width));
        }
    }

    fn ask_resize(&self, width: u32) -> bool {
        let request = self
            .params
            .editor_resizing
            .issue((width, height_in(width, self.expanded(), self.panel())));
        let accepted = self.context.request_resize();
        if !accepted || !self.context.resize_response_will_follow() {
            self.params.editor_resizing.answer(request, accepted);
        }
        accepted
    }

    fn physical_height(&self) -> u32 {
        (f64::from(self.window_height()) * self.dpr)
            .round()
            .max(1.0) as u32
    }

    /// Publish the height before asking the host: its size query sees the requested view.
    /// Refusing a resize leaves the old view intact.
    fn set_panel_height(&mut self, expanded: bool) -> bool {
        let was = self
            .params
            .editor_expanded
            .swap(expanded, Ordering::Relaxed);
        if was == expanded {
            return true;
        }
        if !self.ask_resize(self.width) {
            self.params.editor_expanded.store(was, Ordering::Relaxed);
            return false;
        }
        if !expanded {
            self.playing.release_pointer();
            self.playing.hover = None;
        }
        self.playing.wants_keys = self.playing.holds_keys();
        self.resizing_window = true;
        self.composed = None;
        true
    }

    /// Show or hide all of the lower panel at once, outside the sound's saved parameters.
    fn toggle_panel(&mut self) {
        self.reconcile_resize();
        let expanded = !self.expanded();
        if !self.set_panel_height(expanded) {
            return;
        }
        self.params.editor_panel.store(expanded, Ordering::Relaxed);
        self.params
            .editor_panel_applied
            .store(expanded, Ordering::Relaxed);
        if !expanded {
            self.lower.shut();
            self.fx_view.close_list();
        }
        self.typing = None;
        self.knob_entry = None;
        self.hover = None;
        self.strip_scene.hover = None;
        self.browser.show(false, &self.params);
        self.learning.close_menu();
        self.follow_drawer();
        self.params.editor_resizing.finish(self.resize_view());
    }

    /// The drawer placed as it opens: always over the strip, the window as it is (A6; it had
    /// grown the window below the strip: decisions.md R18, R19).
    fn follow_drawer(&mut self) {
        let open = self.browser.open;
        if open == self.placed {
            return;
        }
        self.placed = open;
        self.params.editor_presets.store(open, Ordering::Relaxed);
        if open {
            self.playing.release();
        }
        self.browser.below = false;
        let expanded = self.params.editor_panel.load(Ordering::Relaxed) || open;
        if !self.set_panel_height(expanded) {
            // If a host stops accepting sizes mid-session, keep the view it still owns.
            if open {
                self.browser.show(false, &self.params);
                self.placed = false;
                self.params.editor_presets.store(false, Ordering::Relaxed);
            } else {
                self.params.editor_panel.store(true, Ordering::Relaxed);
                self.params
                    .editor_panel_applied
                    .store(true, Ordering::Relaxed);
            }
        }
        self.composed = None;
        self.params.editor_resizing.finish(self.resize_view());
    }

    /// The rail's part under the pointer (the drawing's units), if it is over one.
    fn on_rail(&self, (x, y): (f64, f64)) -> Option<BarTarget> {
        match strip::hit(self.rail, x, y) {
            Some(StripTarget::Bar(t)) => Some(t),
            _ => None,
        }
    }

    /// Control `i`'s kind as it is operated now: as the panel's layout places it, an
    /// oscillator's FREQUENCY stepped through its DIVISION's note values while it is timed
    /// (decisions.md R-OSC-TIMED).
    fn kind_of(&self, i: usize) -> Kind {
        let kind = self.renderer.arrangement().control(i).kind;
        match (kind, self.timed_dial(i)) {
            (Kind::Knob { big, .. }, true) => Kind::Knob {
                dial: ca72_panel::controls::Dial::Timed,
                big,
            },
            _ => kind,
        }
    }

    /// Whether control `i` is oscillator 2's or 3's FREQUENCY knob, timed now.
    fn timed_dial(&self, i: usize) -> bool {
        CONTROLS.get(i).is_some_and(|c| match c.param {
            "osc2_frequency" => self.params.timed_now(1),
            "osc3_frequency" => self.params.timed_now(2),
            _ => false,
        }) || self.lfo_timed(i)
    }

    fn operated(&self, control: usize) -> Option<&dyn Operated> {
        match CONTROLS.get(control)?.param {
            "osc2_frequency" => Some(&self.freq_knobs[0]),
            "osc3_frequency" => Some(&self.freq_knobs[1]),
            "osc1_waveform" => Some(&self.waveforms[0]),
            "osc2_waveform" => Some(&self.waveforms[1]),
            "osc3_waveform" => Some(&self.waveforms[2]),
            // An LFO's (decisions.md R-LFO): its RATE in Hz or timed, its WAVEFORM STEPPED or
            // CONTINUOUS, its others its parameters.
            id => match lfo_page::lfo_part(id) {
                Some((n, "rate")) => Some(&self.lfo_rates[n]),
                Some((n, "waveform")) => Some(&self.lfo_waves[n]),
                Some((n, part)) => lfo_page::lfo_param(&self.params, n, part),
                None => param(&self.params, id),
            },
        }
    }

    fn value_of(&self, control: usize) -> f64 {
        self.operated(control).map_or(0.0, |p| f64::from(p.value()))
    }

    /// One change of a control, a gesture of its own.
    fn set_once(&self, control: usize, v: f64) {
        if let Some(p) = self.operated(control) {
            let s = self.setter();
            p.begin(&s);
            p.set(&s, v as f32);
            p.end(&s);
        }
    }

    /// A value to be typed into readout `r` (decisions.md R52): not AUTO GAIN's, which reads
    /// what is measured, nor DETUNE's in SCATTER or VOICES's where it does nothing
    /// ([`voices_inert`]), their knobs doing nothing either.
    fn start_typing(&mut self, r: Readout) {
        self.playing.release();
        self.knob_entry = None;
        if !r.typed()
            || (r == Readout::Detune && !self.params.doubled.value())
            || (r == Readout::Voices && voices_inert(&self.params))
        {
            return;
        }
        self.typing = Some((Typed::Strip(r), String::new()));
        self.typing_keys = true;
    }

    /// A value to be typed into the arpeggiator's drawer's readout `i` (RATE, OCTAVES, GATE,
    /// SWING; decisions.md R-ARP).
    fn start_arp_typing(&mut self, i: usize) {
        self.playing.release();
        self.knob_entry = None;
        self.typing = Some((Typed::Arp(i.min(3)), String::new()));
        self.typing_keys = true;
    }

    /// A value to be typed into the expression drawer's readout `i` (decisions.md R-MPE).
    fn start_mpe_typing(&mut self, i: usize) {
        self.playing.release();
        self.knob_entry = None;
        self.typing = Some((Typed::Mpe(i.min(mpe_ui::UNIT_COUNT - 1)), String::new()));
        self.typing_keys = true;
    }

    /// The standalone's TEMPO set to `bpm` (within 30 to 300, whole beats), the arpeggiator's at
    /// once, and this computer's: written now if `kept`, else when the gesture ends.
    fn set_tempo(&mut self, bpm: f64, kept: bool) {
        if !bpm.is_finite() {
            return;
        }
        let bpm = crate::settings::tempo_within(bpm.round().clamp(0.0, 1000.0) as u16);
        self.params.set_tempo(f64::from(bpm));
        if kept {
            self.keep_settings(Change::Tempo(bpm));
        } else {
            self.settings.apply(Change::Tempo(bpm));
        }
    }

    /// The tempo as the screen reads it.
    fn tempo_now(&self) -> f64 {
        self.params.tempo().round()
    }

    /// What the TEMPO screen reads: the tempo the sources have it at, while one moves it
    /// (decisions.md R-LFO), else as it is set, and as set while it is dragged or typed into.
    fn tempo_shown(&self) -> f64 {
        match self.live_tempo {
            Some(t)
                if self.tempo_drag.is_none() && !matches!(self.typing, Some((Typed::Tempo, _))) =>
            {
                t.round()
            }
            _ => self.tempo_now(),
        }
    }

    /// Whether a source moves target `t` (an assignment of a depth other than 0).
    fn source_moves(&self, t: crate::lfo::Target) -> bool {
        self.sources_on(t).iter().any(|(_, d)| *d != 0.0)
    }

    /// Whether control `i` is being operated or typed into now (its screen then reads its own
    /// value).
    fn operated_now(&self, i: usize) -> bool {
        self.drag.as_ref().is_some_and(|d| d.control == i)
            || self.scroll.as_ref().is_some_and(|s| s.control == i)
            || self.knob_entry.as_ref().is_some_and(|e| e.is_value_of(i))
    }

    /// The live values read again (at most 30 times a second, and only while a source moves a
    /// screen's control: decisions.md R-LFO); none for the rest.
    fn read_live(&mut self, now: Instant) {
        let tempo = self.playing.standalone && self.source_moves(crate::lfo::Target::Tempo);
        let any = tempo || LIVE.iter().any(|&t| self.source_moves(t));
        if !any {
            (self.live, self.live_tempo, self.live_due) = ([None; LIVE.len()], None, None);
            return;
        }
        if self.live_due.is_some_and(|d| now < d) {
            return;
        }
        let live = self.meters.live();
        self.live = std::array::from_fn(|k| live[k].filter(|_| self.source_moves(LIVE[k])));
        self.live_tempo = self.meters.tempo().filter(|_| tempo);
        // (A thirtieth after it was due, never before now: the first a thirtieth from now.)
        let next = self.live_due.map_or(now, |d| d.max(now - LIVE_FRAME)) + LIVE_FRAME;
        self.live_due = Some(next);
    }

    /// The live values the strip's screens read now: a control's while a source moves it and it
    /// is not operated or typed into.
    fn live_shown(&self) -> [Option<f64>; LIVE.len()] {
        std::array::from_fn(|k| {
            let i = lfo_page::control_of(LIVE[k])?;
            let typed =
                matches!(self.typing, Some((Typed::Strip(r), _)) if readout_of(LIVE[k]) == Some(r));
            self.live[k].filter(|_| !self.operated_now(i) && !typed)
        })
    }

    /// A press on the TEMPO screen: a drag begun, or, the second of a double click, a tempo
    /// typed into it.
    fn tempo_press(&mut self, modifiers: Modifiers) {
        let now = Instant::now();
        if self
            .last_tempo_press
            .is_some_and(|at| now - at < DOUBLE_CLICK)
        {
            self.last_tempo_press = None;
            self.tempo_drag = None;
            self.playing.release();
            self.knob_entry = None;
            self.typing = Some((Typed::Tempo, String::new()));
            self.typing_keys = true;
            return;
        }
        self.last_tempo_press = Some(now);
        self.tempo_drag = Some((
            self.pointer.1,
            self.tempo_now(),
            modifiers.contains(Modifiers::SHIFT),
            self.settings.tempo,
        ));
    }

    /// The TEMPO screen's drag ended: its tempo, if it moved, kept for this computer.
    fn tempo_let_go(&mut self) {
        if let Some((.., began)) = self.tempo_drag.take()
            && self.settings.tempo != began
        {
            self.keep_settings(Change::Tempo(self.settings.tempo));
        }
    }

    /// A key while a value is typed: a digit, the point or the sign added where they fit, the
    /// last taken back, Enter setting the value, Escape leaving it; every key is the editor's
    /// meanwhile.
    fn type_key(&mut self, k: &KeyboardEvent) -> bool {
        if k.state != KeyState::Down {
            return true;
        }
        let Some((r, text)) = &mut self.typing else {
            return false;
        };
        match &k.key {
            Key::Character(c) => {
                for ch in c.chars() {
                    let cells = text.chars().filter(|c| *c != '.').count();
                    let digits = text.chars().filter(char::is_ascii_digit).count();
                    // (RATE's note value: dotted with a point after it, a triplet with a T;
                    // OCTAVES one digit.)
                    let ended = text.ends_with(['.', 't']);
                    let fits = match (*r, ch) {
                        (Typed::Arp(0), '0'..='9') => cells < 2 && !ended,
                        (Typed::Arp(0), '.') => cells > 0 && !ended,
                        (Typed::Arp(0), 't' | 'T') => cells > 0 && !ended,
                        (Typed::Arp(1), '0'..='9') => cells < 1,
                        (Typed::Arp(1), _) => false,
                        // (MODULATION's -100 to 100, CUTOFF's -4 to 4 with a point.)
                        (Typed::Mpe(_), '0'..='9') => digits < 3,
                        (Typed::Mpe(i), '.') => i % 2 == 1 && i < 4 && !text.contains('.'),
                        (Typed::Mpe(_), '-') => text.is_empty(),
                        (Typed::Mpe(_), _) => false,
                        (Typed::Tempo, '0'..='9') => cells < 3,
                        (Typed::Tempo, _) => false,
                        (_, '0'..='9') => cells < 3,
                        (_, '.') => !text.contains('.') && cells < 3,
                        (Typed::Strip(r), '-') => text.is_empty() && matches!(r, Readout::Level),
                        _ => false,
                    };
                    let ch = ch.to_ascii_lowercase();
                    if fits {
                        text.push(ch);
                    }
                }
            }
            Key::Backspace => {
                text.pop();
            }
            Key::Enter => {
                let (r, text) = (*r, text.clone());
                self.typing = None;
                match r {
                    Typed::Strip(r) => self.set_typed(r, &text),
                    Typed::Arp(i) => self.set_arp_typed(i, &text),
                    Typed::Mpe(i) => self.set_mpe_typed(i, &text),
                    Typed::Tempo => {
                        if let Ok(bpm) = text.parse::<f64>() {
                            self.set_tempo(bpm, true);
                        }
                    }
                }
            }
            Key::Escape => self.typing = None,
            _ => {}
        }
        true
    }

    /// The value typed into readout `r` set, as its readout reads it (VOICES a count, ENTROPY,
    /// WIDTH and INNER percent, DETUNE cents, DRIVE and LEVEL dB), within its knob's travel, a
    /// gesture of its own; nothing for what is not a number.
    fn set_typed(&mut self, r: Readout, text: &str) {
        let Ok(v) = text.trim().parse::<f64>() else {
            return;
        };
        if !v.is_finite() {
            return;
        }
        let p = &self.params;
        let (id, normalized) = match r {
            Readout::Voices => (
                "voices",
                p.voices
                    .preview_normalized(v.round().clamp(2.0, 10.0) as i32),
            ),
            Readout::Entropy => ("entropy", p.entropy.preview_normalized(v as f32)),
            Readout::Width => ("spread", p.spread.preview_normalized(v as f32)),
            Readout::Inner => ("inner", p.inner.preview_normalized(v as f32)),
            Readout::Detune => (
                "double",
                p.double
                    .preview_normalized((v / crate::engine::DOUBLE_CENTS * 100.0) as f32),
            ),
            Readout::Drive => ("drive", p.drive.preview_normalized(v as f32)),
            Readout::Level => ("level", p.level.preview_normalized(v as f32)),
            Readout::Auto => return,
        };
        if let Some(i) = ca72_panel::controls::index(id) {
            self.set_once(i, f64::from(normalized.clamp(0.0, 1.0)));
        }
    }

    /// The value typed into the arpeggiator's drawer's readout `i` set, as it reads it: RATE a
    /// note value (16, 16. dotted, 16t a triplet: one of its thirteen), OCTAVES 1 to 5, GATE and
    /// SWING percent; nothing for what is not one.
    fn set_arp_typed(&mut self, i: usize, text: &str) {
        let p = &self.params;
        let normalized = match i {
            0 => {
                let Some(k) = crate::arp::RATES.iter().position(|(t, _)| *t == text) else {
                    return;
                };
                p.arp_rate.preview_normalized(ArpRate::from_index(k))
            }
            1 => {
                let Ok(v) = text.parse::<i32>() else { return };
                p.arp_octaves.preview_normalized(
                    v.clamp(crate::params::ARP_OCTAVES.0, crate::params::ARP_OCTAVES.1),
                )
            }
            _ => {
                let Ok(v) = text.parse::<f32>() else { return };
                if !v.is_finite() {
                    return;
                }
                let q = if i == 2 { &p.arp_gate } else { &p.arp_swing };
                q.preview_normalized(v.clamp(0.0, 100.0))
            }
        };
        if let Some(c) = ca72_panel::controls::index(arp_ui::KNOB_PARAMS[i.min(3)]) {
            self.set_once(c, f64::from(normalized.clamp(0.0, 1.0)));
        }
    }

    /// The value typed into the expression drawer's readout `i` set, as it reads it (decisions.md
    /// R-MPE): MODULATION's percent, CUTOFF's octaves, within the knob's travel; nothing for what
    /// is not a number.
    fn set_mpe_typed(&mut self, i: usize, text: &str) {
        let Ok(v) = text.trim().parse::<f32>() else {
            return;
        };
        if !v.is_finite() {
            return;
        }
        let i = i.min(mpe_ui::UNIT_COUNT - 1);
        let p = &self.params;
        let q = [
            &p.pressure_mod,
            &p.pressure_filter,
            &p.timbre_mod,
            &p.timbre_filter,
            &p.velocity_curve,
            &p.velocity_floor,
        ][i];
        let normalized = q.preview_normalized(v);
        if let Some(c) = ca72_panel::controls::index(mpe_ui::KNOB_PARAMS[i]) {
            self.set_once(c, f64::from(normalized.clamp(0.0, 1.0)));
        }
    }

    /// A press on the expression drawer's readouts (decisions.md R-MPE): one double-clicked typed
    /// into.
    fn mpe_press(&mut self, t: MpeTarget) {
        let now = Instant::now();
        let double = self
            .last_mpe_press
            .is_some_and(|(at, last)| last == t && now - at < DOUBLE_CLICK);
        self.last_mpe_press = Some((now, t));
        let MpeTarget::Readout(i) = t;
        if double {
            self.last_mpe_press = None;
            self.start_mpe_typing(i);
        }
    }

    /// A press on the arpeggiator's drawer's tabs and readouts (decisions.md R-ARP): ON and
    /// HOLD each switched, DIRECTION's choice set, a readout double-clicked typed into.
    fn arp_press(&mut self, t: ArpTarget) {
        let now = Instant::now();
        let double = self
            .last_arp_press
            .is_some_and(|(at, last)| last == t && now - at < DOUBLE_CLICK);
        self.last_arp_press = Some((now, t));
        let p = &self.params;
        match t {
            ArpTarget::Tab(ArpBank::Arp, 0) => self.set_switch(&p.arp_on, !p.arp_on.value()),
            ArpTarget::Tab(ArpBank::Arp, _) => {
                self.set_switch(&p.arp_latch, !p.arp_latch.value());
            }
            ArpTarget::Tab(ArpBank::Direction, i) => {
                let q = &p.arp_direction;
                let last = ArpDirection::variants().len() - 1;
                let to = q.preview_normalized(ArpDirection::from_index(i.min(last)));
                if q.unmodulated_normalized_value() != to {
                    let s = self.setter();
                    s.begin_set_parameter(q);
                    s.set_parameter_normalized(q, to);
                    s.end_set_parameter(q);
                }
            }
            ArpTarget::Readout(i) => {
                if double {
                    self.last_arp_press = None;
                    self.start_arp_typing(i);
                }
            }
        }
    }

    /// The rail's ARP or MPE key, or the standalone's FX key: its drawer dropped down over the lower
    /// page (the other's put away for it), or put away (the presets' drawer, if open, shut for it;
    /// the lower panel shown if hidden).
    fn toggle_lower(&mut self, which: Lower) {
        if !self.expanded() {
            self.toggle_panel();
            if !self.expanded() {
                return;
            }
            self.lower.show(which, true);
        } else if self.browser.open {
            self.browser.show(false, &self.params);
            self.follow_drawer();
            self.lower.show(which, true);
        } else {
            self.lower.show(which, !self.lower.is_open(which));
        }
        self.typing = None;
        self.hover = None;
        self.strip_scene.hover = None;
        self.arp_scene.hover = None;
        self.fx_view.hover = None;
        self.fx_view.cancel_press();
        self.fx_view.close_list();
        self.composed = None;
    }

    fn set_power(&self, on: bool) {
        let s = self.setter();
        s.begin_set_parameter(&self.params.bypass);
        s.set_parameter(&self.params.bypass, !on);
        s.end_set_parameter(&self.params.bypass);
    }

    /// An amount the strip turns off and back on (ENTROPY's, WIDTH's and INNER's readouts,
    /// SCATTER | DOUBLE): its parameter and its place in `last`.
    fn amount(&self, a: Readout) -> (&FloatParam, usize) {
        match a {
            Readout::Width => (&self.params.spread, 1),
            Readout::Detune => (&self.params.double, 2),
            Readout::Inner => (&self.params.inner, 3),
            _ => (&self.params.entropy, 0),
        }
    }

    /// An amount turned off (0, kept) or back on at it (half way if it never was), a gesture of
    /// its own; nothing if it is so already.
    fn switch_amount(&mut self, a: Readout, on: bool) {
        let (p, k) = self.amount(a);
        let now = p.unmodulated_normalized_value();
        if (now > 0.0) == on {
            return;
        }
        let to = if on {
            self.last[k]
        } else {
            self.last[k] = now;
            0.0
        };
        let (p, _) = self.amount(a);
        let s = self.setter();
        s.begin_set_parameter(p);
        s.set_parameter_normalized(p, to);
        s.end_set_parameter(p);
    }

    /// Oscillator `n`'s waveform made STEPPED or CONTINUOUS (its WAVEFORM switch's menu,
    /// decisions.md R-CONTINUOUS-WAVE), one gesture: made CONTINUOUS, its shape put at the switch's
    /// own position exactly first, so that the sound does not move (oscillator 3's sawtooth at
    /// its reverse sawtooth's place, CONTINUOUS's third); made STEPPED, WAVEFORM as it followed
    /// the shape, the switch's position nearest.
    fn set_wave_mode(&self, n: usize, continuous: bool) {
        let (mode, shape, ..) = self.params.shape_params(n);
        let to = if continuous {
            crate::params::WaveMode::Continuous
        } else {
            crate::params::WaveMode::Stepped
        };
        if mode.value() == to {
            return;
        }
        let s = self.setter();
        if continuous {
            let k = WaveformSwitch::place(self.waveforms[n].waveform().value());
            let at = match (n, k) {
                // (Oscillator 3's reverse sawtooth and sawtooth: its reverse sawtooth's place.)
                (2, 1 | 2) => 2,
                _ => k,
            };
            let t = at as f64 / interact::CONTINUOUS_PLACES;
            s.begin_set_parameter(shape);
            s.set_parameter_normalized(shape, t as f32);
            s.end_set_parameter(shape);
        }
        s.begin_set_parameter(mode);
        s.set_parameter(mode, to);
        s.end_set_parameter(mode);
    }

    /// Oscillator `n`'s MOD turned to its next way (its key Command-clicked: clockwise,
    /// anticlockwise, both ways, round again), a gesture of its own.
    fn next_mod_way(&self, n: usize) {
        use crate::params::ShapeModDirection as D;
        let p = self.params.shape_params(n).3;
        let next = match p.value() {
            D::Clockwise => D::Anticlockwise,
            D::Anticlockwise => D::Both,
            D::Both => D::Clockwise,
        };
        let s = self.setter();
        s.begin_set_parameter(p);
        s.set_parameter(p, next);
        s.end_set_parameter(p);
    }

    /// Whether control `i` is a WAVEFORM switch whose waveform is CONTINUOUS (decisions.md
    /// R-CONTINUOUS-WAVE): operated along its whole travel ([`WaveformSwitch`]).
    fn continuous_switch(&self, i: usize) -> bool {
        CONTROLS.get(i).is_some_and(|c| {
            ca72_panel::render::WAVEFORMS
                .iter()
                .position(|w| *w == c.param)
                .is_some_and(|n| self.params.continuous(n))
        }) || self.lfo_continuous(i)
    }

    /// Whether control `i` answers the pointer: all but DETUNE in SCATTER (the owner: "make that
    /// knob unresponsive unless the user is in double mode"), VOICES where it does nothing
    /// ([`voices_inert`]), a TIMED key away from LO and OSC. 3 CONTROL while oscillator 3 is
    /// timed ([`osc3_control_inert`]), each drawn dimmed then; and a MOD key while its
    /// oscillator's waveform is STEPPED, not on the panel then.
    fn operable(&self, i: usize) -> bool {
        CONTROLS.get(i).is_none_or(|c| match c.param {
            "double" => self.params.doubled.value(),
            "voices" => !voices_inert(&self.params),
            // A TIMED key away from its RANGE's LO (R-OSC-TIMED).
            "osc2_timed" => !timed_inert(&self.params, 1),
            "osc3_timed" => !timed_inert(&self.params, 2),
            // A MOD key while its oscillator's waveform is STEPPED: not on the panel
            // (R-CONTINUOUS-WAVE).
            "osc1_shape_mod" => self.params.continuous(0),
            "osc2_shape_mod" => self.params.continuous(1),
            "osc3_shape_mod" => self.params.continuous(2),
            // OSC. 3 CONTROL while oscillator 3 is timed.
            "osc3_control" => !osc3_control_inert(&self.params),
            _ => true,
        })
    }

    /// A switch set, a gesture of its own; nothing if it is so already.
    fn set_switch(&self, p: &BoolParam, on: bool) {
        if p.value() != on {
            let s = self.setter();
            s.begin_set_parameter(p);
            s.set_parameter(p, on);
            s.end_set_parameter(p);
        }
    }

    /// A press on the strip's own parts (the drawing's units): a tab, a readout that is a
    /// switch, the rail's keys and the name.
    fn strip_press(&mut self, t: StripTarget) {
        let now = Instant::now();
        let double = self
            .last_strip_press
            .filter(|(at, last, _)| *last == t && now - *at < DOUBLE_CLICK);
        // A readout double-clicked: a value typed into it (decisions.md R52); a switch's
        // first press, which switched its amount, undone.
        if let (Some((_, _, was)), StripTarget::Switch(r) | StripTarget::Readout(r)) = (double, t) {
            self.last_strip_press = None;
            if let StripTarget::Switch(a) = t {
                let (p, _) = self.amount(a);
                if p.unmodulated_normalized_value() != was {
                    let s = self.setter();
                    s.begin_set_parameter(p);
                    s.set_parameter_normalized(p, was);
                    s.end_set_parameter(p);
                }
            }
            self.start_typing(r);
            return;
        }
        let before = match t {
            StripTarget::Switch(a) => self.amount(a).0.unmodulated_normalized_value(),
            _ => 0.0,
        };
        self.last_strip_press = Some((now, t, before));
        match t {
            StripTarget::Bar(BarTarget::Panel) => self.toggle_panel(),
            StripTarget::Bar(BarTarget::Arp) => self.toggle_lower(Lower::Arp),
            StripTarget::Bar(BarTarget::Mpe) => self.toggle_lower(Lower::Mpe),
            StripTarget::Bar(BarTarget::Fx) if self.fx_view.available() => {
                self.toggle_lower(Lower::Fx);
            }
            StripTarget::Readout(_) => {}
            StripTarget::Switch(a) => {
                let on = self.amount(a).0.unmodulated_normalized_value() <= 0.0;
                self.switch_amount(a, on);
            }
            // MONO: POLY and UNISON off; POLY: POLY on, UNISON off; UNISON: on (POLY as it was,
            // which UNISON outranks: the CA-74's R31).
            StripTarget::Tab(Bank::Mode, i) => {
                let p = &self.params;
                match i {
                    0 => {
                        self.set_switch(&p.unison, false);
                        self.set_switch(&p.poly, false);
                    }
                    1 => {
                        self.set_switch(&p.unison, false);
                        self.set_switch(&p.poly, true);
                    }
                    _ => self.set_switch(&p.unison, true),
                }
            }
            StripTarget::Tab(Bank::Stereo, i) => self.set_switch(&self.params.doubled, i == 1),
            // The tabs are the placement's choices in their order: EVEN, CENTER.
            StripTarget::Tab(Bank::Placement, i) => {
                let p = &self.params.placement;
                let last = Scatter::variants().len() - 1;
                let to = p.preview_normalized(Scatter::from_index(i.min(last)));
                if p.unmodulated_normalized_value() != to {
                    let s = self.setter();
                    s.begin_set_parameter(p);
                    s.set_parameter_normalized(p, to);
                    s.end_set_parameter(p);
                }
            }
            StripTarget::Tab(Bank::Auto, _) => {
                let on = !self.params.auto_gain.value();
                self.set_switch(&self.params.auto_gain, on);
            }
            StripTarget::Bar(t @ (BarTarget::Global | BarTarget::Keyboard | BarTarget::Lfo)) => {
                self.show_lower_page(t);
            }
            StripTarget::Bar(BarTarget::Macro) => self.toggle_macros(),
            // The settings' key: its menu, dropping down from under it to its left.
            StripTarget::Bar(BarTarget::Settings) => {
                self.playing.release();
                self.agents.copied = None;
                self.open_settings();
            }
            StripTarget::Bar(b) => {
                if matches!(
                    b,
                    BarTarget::Prev | BarTarget::Next | BarTarget::Name | BarTarget::Save
                ) {
                    self.playing.release();
                }
                let setter = ParamSetter::new(self.context.as_ref());
                self.browser.bar_press(b, &self.params, &setter);
                self.follow_drawer();
            }
            StripTarget::Display => {}
        }
    }

    fn press(&mut self, modifiers: Modifiers) {
        let (x, y) = self.in_drawing(self.pointer);
        if let Some(notes) = &mut self.whats_new {
            if notes.press(x, y) {
                self.whats_new = None;
            }
            self.composed = None;
            return;
        }
        #[cfg(feature = "standalone-production")]
        if let Some(dialog) = &mut self.standalone_dialog {
            if dialog.press(x, y) {
                self.standalone_dialog = None;
            }
            self.composed = None;
            return;
        }
        if !self.expanded() && y >= DRAWER_TOP {
            return;
        }
        // A lit page key is inert before the usual press cancels typing or ends a gesture.
        // Modal notes and menus still own their presses.
        if self.expanded()
            && !self.ultra_note
            && !self.learning.menu_open()
            && let Some(key @ (BarTarget::Global | BarTarget::Keyboard | BarTarget::Lfo)) =
                self.on_rail((x, y))
            && key == self.lower_page()
        {
            return;
        }
        // The field keeps a press inside it. Elsewhere cancels it, consuming that press.
        if let Some(entry) = &self.knob_entry {
            if !self.entry_field(entry).contains(x, y) {
                self.knob_entry = None;
                self.last_press = None;
            }
            return;
        }
        // A press on the readout a value is being typed into leaves it typed into; anywhere
        // else ends the typing, nothing set (decisions.md R52).
        if let Some((r, _)) = self.typing {
            let on_it = match r {
                Typed::Strip(r) => {
                    !self.in_arp((x, y))
                        && !self.in_mpe((x, y))
                        && matches!(strip::hit(self.rail, x, y), Some(StripTarget::Switch(t) | StripTarget::Readout(t)) if t == r)
                }
                Typed::Arp(i) => {
                    self.in_arp((x, y)) && arp_ui::hit(x, y) == Some(ArpTarget::Readout(i))
                }
                Typed::Mpe(i) => {
                    self.in_mpe((x, y)) && mpe_ui::hit(x, y) == Some(MpeTarget::Readout(i))
                }
                Typed::Tempo => art::tempo_hit(self.renderer.arrangement(), x, y),
            };
            if on_it {
                return;
            }
        }
        // A wheel's gesture still waiting for its rest, or a drag whose release never came
        // (its window lost the pointer): ended before anything else begins.
        self.end_gestures();
        // ULTRA's note showing: the press closes it, read.
        if self.ultra_note {
            self.ultra_note = false;
            self.keep_settings(Change::NoteRead);
            return;
        }
        // A control's menu open: the press is its own (an item, or closing it).
        match self
            .learning
            .press(self.renderer.fonts(), &self.params.midi_map, (x, y))
        {
            Pressed::Nothing => {}
            Pressed::Done => return,
            Pressed::Indicator(shown) => {
                self.keep_settings(Change::Shutter(shown));
                return;
            }
            Pressed::WhatsNew => {
                self.show_release_notes(crate::whats_new::current());
                return;
            }
            Pressed::StandaloneSettings => {
                self.playing.release();
                #[cfg(feature = "standalone-production")]
                crate::standalone_settings::request_open();
                return;
            }
            Pressed::Dials(law) => {
                self.set_frequency_dials(law);
                return;
            }
            Pressed::WheelMenu(range) => {
                self.learning.open_wheel_menu(
                    self.renderer.fonts(),
                    settings_menu_corner(),
                    self.renderer.text_size() * MENU_TEXT,
                    self.params.midi_map.wheel_source(),
                    range,
                );
                return;
            }
            Pressed::WheelSource(source) => {
                self.set_wheel_source(source);
                return;
            }
            Pressed::MpeMenu => {
                self.learning.open_mpe_menu(
                    self.renderer.fonts(),
                    settings_menu_corner(),
                    self.renderer.text_size() * MENU_TEXT,
                    (self.params.mpe_mode(), self.params.timbre_rest()),
                );
                return;
            }
            Pressed::MpeInput(mode) => {
                self.set_mpe_input(mode);
                return;
            }
            Pressed::TimbreRest(rest) => {
                self.set_timbre_rest(rest);
                return;
            }
            Pressed::GlideRange(long) => {
                let p = &self.params.glide_range;
                let s = self.setter();
                s.begin_set_parameter(p);
                s.set_parameter(
                    p,
                    if long {
                        crate::params::GlideRange::Long
                    } else {
                        crate::params::GlideRange::Normal
                    },
                );
                s.end_set_parameter(p);
                return;
            }
            Pressed::Rect(n, variable) => {
                self.set_wave_mode(n, variable);
                return;
            }
            Pressed::Look(skin) => {
                self.set_look(skin);
                return;
            }
            Pressed::Pack(i) => {
                self.set_pack(i);
                return;
            }
            Pressed::Lettering(lettering) => {
                self.set_lettering(lettering);
                return;
            }
            Pressed::TicksTitles(print) => {
                self.set_ticks_titles(print);
                return;
            }
            Pressed::HoverCables(on) => {
                self.keep_settings(Change::HoverCables(on));
                return;
            }
            Pressed::Agents(on) => {
                self.set_agents(on);
                return;
            }
            Pressed::Setup(setup) => {
                self.copy_setup(setup);
                // (Open again, to say it was copied.)
                self.open_settings();
                return;
            }
            Pressed::SizeMenu => {
                let size = self.renderer.text_size() * MENU_TEXT;
                let sizes = self.sizes();
                self.learning.open_size_menu(
                    self.renderer.fonts(),
                    settings_menu_corner(),
                    size,
                    &sizes,
                    self.width,
                );
                return;
            }
            Pressed::Size(width) => {
                // Asked of the host at the next frame, as the grip's are.
                self.gripped = Some(width);
                return;
            }
            Pressed::List(chosen) => {
                self.show_midi_list(chosen);
                return;
            }
            Pressed::Fx(a) => {
                self.fx_view.act(a);
                return;
            }
            Pressed::Mod(m) => {
                self.mod_item(m);
                return;
            }
        }
        // The effects' list open: a press elsewhere puts it away, and is its own.
        if self.fx_view.list_open() && !self.in_fx((x, y)) {
            self.fx_view.close_list();
            return;
        }
        // The MIDI map mode: a click chooses a control and operates nothing (decisions.md
        // R-MIDI-MAP); its MIDI key, TYPING, PANIC, the rail's keys and the drawer as ever.
        if self.mapping_on() && self.map_press((x, y)) {
            return;
        }
        // The standalone's TEMPO screen, on the wood above the panel (R-ARP): Alt/Option, the
        // last touched source's depth on it, as a knob's (R-LFO).
        if self.playing.standalone && art::tempo_hit(self.renderer.arrangement(), x, y) {
            if !(modifiers.contains(Modifiers::ALT) && self.mod_press((x, y), modifiers)) {
                self.tempo_press(modifiers);
            }
            return;
        }
        // Its TYPING and PANIC keys after it (decisions.md R-TEMPO-CORNER).
        if self.playing.standalone
            && let Some(k) = art::tempo_key_hit(self.renderer.arrangement(), x, y)
        {
            match k {
                art::TempoKey::Typing => {
                    self.playing.release();
                    self.playing.typing = !self.playing.typing;
                    self.playing.wants_keys = self.playing.typing;
                }
                art::TempoKey::Panic => {
                    self.panic();
                    self.panic_held = true;
                }
                art::TempoKey::Midi => self.toggle_mapping(),
            }
            return;
        }
        if let Some((dx, dy)) = self.in_drawer((x, y)) {
            match drawer_hit(&self.browser.drawer, dx, dy) {
                Some(DrawerTarget::Update) => self.update.press(),
                Some(DrawerTarget::Key(PresetKey::MidiLearn)) => {
                    self.learning.list = !self.learning.list;
                }
                Some(DrawerTarget::MidiRow(i)) => {
                    self.learning.list_press(&self.params.midi_map, i, None);
                }
                Some(DrawerTarget::MidiAction(i, a)) => {
                    self.learning.list_press(&self.params.midi_map, i, Some(a));
                }
                Some(DrawerTarget::Back) | None => {}
                // Anything of the presets' shows them again in the MIDI list's place.
                Some(t) => {
                    self.learning.list = false;
                    let setter = ParamSetter::new(self.context.as_ref());
                    // (A field's caret where its look's typeface sets the characters.)
                    let dx = self.drawer.press_x(&self.browser.drawer, t, dx);
                    self.browser.drawer_press(t, dx, &self.params, &setter);
                }
            }
            self.follow_drawer();
            return;
        }
        // The arpeggiator's drawer, over the lower page: its own parts only, and while it slides
        // nothing.
        if self.in_arp((x, y)) {
            if !self.lower.settled() {
                return;
            }
            if let Some(t) = arp_ui::hit(x, y) {
                self.arp_press(t);
            } else if let Some(target) = interact::hit_arp(x, y) {
                self.press_target(target, modifiers);
            }
            return;
        }
        // The expression drawer, as the arpeggiator's (decisions.md R-MPE).
        if self.in_mpe((x, y)) {
            if !self.lower.settled() {
                return;
            }
            if let Some(t) = mpe_ui::hit(x, y) {
                self.mpe_press(t);
            } else if let Some(target) = interact::hit_drawer(DrawerPlace::Mpe, x, y) {
                self.press_target(target, modifiers);
            }
            return;
        }
        // The effects' drawer and its list (decisions.md R-FX-DRAWER).
        if self.in_fx((x, y)) {
            if self.lower.settled() && !self.fx_pin_press((x, y), modifiers) {
                self.fx_view.press(x, y);
            }
            return;
        }
        // A name typed is kept as a press goes elsewhere (decisions.md R-MACROS).
        let on_macros = self.macros_show() && macro_ui::inside(x, y);
        if !on_macros {
            self.end_rename(true);
        }
        // Modulation over the panel: a pop-up's dial, a ring's end, Alt/Option on a knob
        // (decisions.md R-LFO).
        if self.mod_press((x, y), modifiers) {
            return;
        }
        // The macros, in the left area (R-MACROS).
        if on_macros && self.macro_press((x, y), modifiers) {
            return;
        }
        // The left hand's VELOCITY jack, with the wheels: an assignment dragged (R-VELOCITY).
        if self.velocity_press((x, y)) {
            return;
        }
        // The LFO page, in the lower page's place.
        if self.page_shows() && lfo_ui::inside(x, y) {
            self.page_press((x, y), modifiers);
            return;
        }
        if self.keyboard_shown()
            && let Some(target) = keyboard::hit(self.playing.root, x, y)
        {
            match target {
                keyboard::Target::Note(note) => {
                    let v = velocity::key_velocity(self.playing.root, note, y);
                    self.playing.press_at(note, v);
                }
                keyboard::Target::Down | keyboard::Target::Up => {
                    self.playing.shift(target == keyboard::Target::Up);
                    self.keep_settings(Change::KeyboardRoot(self.playing.root));
                }
                keyboard::Target::Back => {}
            }
            self.hover = None;
            self.strip_scene.hover = None;
            return;
        }
        if let Some(t) = strip::hit(self.rail, x, y) {
            self.strip_press(t);
            return;
        }
        let Some(target) = self.hit(x, y) else {
            return;
        };
        self.press_target(target, modifiers);
    }

    /// A press on a control (the panel's, the strip's knobs, the arpeggiator's drawer's), on
    /// POWER or the grip.
    fn press_target(&mut self, target: Target, modifiers: Modifiers) {
        if entry::shortcut(modifiers)
            && !self.learning.holds_keys(&self.params.midi_map)
            && let Target::Control(i) | Target::Legend(i, _) = target
        {
            // The arpeggiator's knobs take their values in their drawer's readouts, in its
            // units, as a double click there (R-KNOB-ENTRY with R-ARP).
            if let Some(r) = arp_readout(i) {
                if self.operable(i) {
                    self.start_arp_typing(r);
                    return;
                }
            } else if let Some(r) = mpe_readout(i) {
                // (The expression drawer's the same, R-MPE.)
                self.start_mpe_typing(r);
                return;
            } else if let Some(n) = CONTROLS.get(i).and_then(|c| {
                ca72_panel::render::MOD_KEYS
                    .iter()
                    .position(|k| *k == c.param)
            }) {
                // A MOD key: its next way (R-CONTINUOUS-WAVE; the owner: "the ability to command
                // click on that, so that they could reverse the direction").
                if self.operable(i) {
                    self.next_mod_way(n);
                }
                return;
            } else if self.start_knob_entry(i) {
                return;
            }
        }
        let now = Instant::now();
        let double = self
            .last_press
            .is_some_and(|(t, last)| last == target && now - t < DOUBLE_CLICK);
        self.last_press = Some((now, target));
        match target {
            Target::Control(i) | Target::Legend(i, _) => {
                if !self.operable(i) {
                    return;
                }
                let kind = self.kind_of(i);
                if double && !matches!(kind, Kind::Rocker { .. } | Kind::Toggle { .. } | Kind::Key)
                {
                    if let Some(p) = self.operated(i) {
                        self.set_once(i, f64::from(p.default()));
                    }
                    self.last_press = None;
                    return;
                }
                let value = self.value_of(i);
                if !matches!(kind, Kind::Rocker { .. } | Kind::Toggle { .. } | Kind::Key)
                    && let Some(p) = self.operated(i)
                {
                    p.begin(&self.setter());
                }
                self.drag = Some(Drag {
                    control: i,
                    from: self.pointer,
                    pressed: target,
                    value,
                    base: value,
                    at: self.pointer,
                    fine: modifiers.contains(Modifiers::SHIFT),
                    moved: false,
                });
            }
            Target::Power => self.set_power(self.params.bypass.value()),
            Target::Grip => {
                self.resizing = Some((self.pointer.0, self.width));
            }
        }
    }

    /// PANIC: every note off, from every source, and the controllers reset (the keyboard's
    /// button, and an agent's).
    fn panic(&mut self) {
        #[cfg(feature = "standalone-production")]
        if let Some(app) = plugin_kit_standalone::AppHandle::current() {
            let token = self.playing.panic_standalone();
            let accepted = app.panic_tagged(token);
            debug_assert!(
                accepted,
                "local panic tokens are nonzero and fit in 63 bits"
            );
        } else {
            self.playing.panic();
        }
        #[cfg(not(feature = "standalone-production"))]
        self.playing.panic();
    }

    /// A right click: the menu of the control under the pointer (MIDI Learn: decisions.md R34),
    /// on the panel or the strip; elsewhere none.
    fn context_menu(&mut self) {
        if self.standalone_dialog_open() || self.whats_new.is_some() {
            return;
        }
        self.playing.release();
        self.end_gestures();
        // (A knob's pop-up gives way to its menu.)
        self.modulating.popup = None;
        let (x, y) = self.in_drawing(self.pointer);
        if !self.expanded() && y >= DRAWER_TOP {
            return;
        }
        let size = self.renderer.text_size() * MENU_TEXT;
        let params = Arc::clone(&self.params);
        let map = &params.midi_map;
        // (A drawer dropped from the rail is over the keyboard's top: its own menus.)
        if self.keyboard_shown() && keyboard::inside(x, y) && !self.in_lower((x, y)) {
            self.learning.close_menu();
            return;
        }
        if let Some(t @ (BarTarget::Prev | BarTarget::Next)) = self.on_rail((x, y)) {
            self.learning.open_menu(
                self.renderer.fonts(),
                map,
                Some(Place::Preset(t)),
                None,
                (x, y),
                size,
            );
            return;
        }
        if self.in_drawer((x, y)).is_some() || self.on_rail((x, y)).is_some() {
            self.learning.close_menu();
            return;
        }
        // The arpeggiator's drawer: a knob's menu, as the panel's (its tabs are learned from the
        // presets' drawer's MIDI list).
        if self.in_arp((x, y)) || self.in_mpe((x, y)) {
            let place = if self.in_mpe((x, y)) {
                DrawerPlace::Mpe
            } else {
                DrawerPlace::Arp
            };
            match interact::hit_drawer(place, x, y).filter(|_| self.lower.settled()) {
                Some(target @ Target::Control(i)) => self.learning.open_menu(
                    self.renderer.fonts(),
                    map,
                    Some(Place::Panel(i)),
                    Some(target),
                    (x, y),
                    size,
                ),
                _ => self.learning.close_menu(),
            }
            return;
        }
        // An effects' slot: what its state offers (R-FX-SAFETY), REPLACE… and REMOVE.
        if self.in_fx((x, y)) {
            if self.fx_pin_menu((x, y)) {
                return;
            }
            match self
                .fx_view
                .menu_for(x, y)
                .filter(|_| self.lower.settled() && !self.fx_view.list_open())
            {
                Some((name, items)) => {
                    self.learning
                        .open_fx_menu(self.renderer.fonts(), (x, y), size, &name, &items);
                }
                None => self.learning.close_menu(),
            }
            return;
        }
        // The left hand's VELOCITY jack's: what it modulates (decisions.md R-VELOCITY).
        if self.velocity_menu((x, y)) {
            return;
        }
        // The macros': a knob's, else its macro's colour and RENAME… (decisions.md R-MACROS).
        if self.macros_show() && macro_ui::inside(x, y) {
            if !self.macro_menu((x, y)) {
                self.learning.close_menu();
            }
            return;
        }
        // The LFO page's: a control's, else its LFO's colour and EDGES (decisions.md R-LFO).
        if self.page_shows() && lfo_ui::inside(x, y) {
            self.page_menu((x, y));
            return;
        }
        // The standalone's TEMPO screen: what moves it, and the sources that can (R-LFO).
        if self.tempo_hovered((x, y)) {
            self.open_tempo_menu((x, y));
            return;
        }
        if let Some(t) = strip::hit(self.rail, x, y) {
            // A tab's menu (the strip's knobs' are the panel's), above it.
            match t.control() {
                Some(c) => {
                    let (x0, y0, x1, _) = strip::span(c);
                    self.learning.open_menu(
                        self.renderer.fonts(),
                        map,
                        Some(Place::Strip(c)),
                        None,
                        ((x0 + x1) / 2.0, y0),
                        size,
                    );
                }
                None => self.learning.close_menu(),
            }
            return;
        }
        let target = self.hit(x, y);
        // QUALITY's toggle: its indicator shown or hidden (decisions.md R48).
        if let Some(Target::Control(i) | Target::Legend(i, _)) = target
            && CONTROLS.get(i).is_some_and(|c| c.param == "quality")
        {
            self.learning.open_quality_menu(
                self.renderer.fonts(),
                (x, y),
                size,
                self.settings.shutter,
            );
            return;
        }
        let place = match target {
            Some(Target::Control(i) | Target::Legend(i, _)) => Some(Place::Panel(i)),
            _ => None,
        };
        // (Each source on a knob a head with DEPTH… and REMOVE; a strip knob's LFO head:
        // decisions.md R-LFO.)
        let rows = match target {
            Some(Target::Control(i) | Target::Legend(i, _)) => self.knob_rows(i),
            _ => Vec::new(),
        };
        self.learning.open_menu_with(
            self.renderer.fonts(),
            &self.params.midi_map,
            place,
            target,
            (x, y),
            size,
            rows,
        );
    }

    /// The drawer opened on its MIDI list, `chosen` in view.
    fn show_midi_list(&mut self, chosen: Option<usize>) {
        self.learning.show_list(chosen);
        if !self.browser.open {
            self.browser.show(true, &self.params);
        }
        self.follow_drawer();
    }

    /// The pointer moved.
    fn moved(&mut self, modifiers: Modifiers) {
        if self.standalone_dialog_open() || self.whats_new.is_some() {
            return;
        }
        if let Some((from, bpm, fine, _)) = &mut self.tempo_drag {
            // (SHIFT pressed or let go mid-drag: on from here, at the other pace.)
            let now = modifiers.contains(Modifiers::SHIFT);
            if now != *fine {
                (*from, *bpm, *fine) = (self.pointer.1, self.params.tempo().round(), now);
            }
            let per = if *fine { 4.0 * TEMPO_DRAG } else { TEMPO_DRAG };
            let to = *bpm + (*from - self.pointer.1) / per;
            self.set_tempo(to, false);
            return;
        }
        let (x, y) = self.in_drawing(self.pointer);
        self.tempo_hover = if self.playing.standalone {
            art::tempo_key_hit(self.renderer.arrangement(), x, y)
        } else {
            None
        };
        if self.playing.dragging() {
            let target = keyboard::hit(self.playing.root, x, y);
            match target {
                Some(keyboard::Target::Note(note)) => {
                    let v = velocity::key_velocity(self.playing.root, note, y);
                    self.playing.drag_at(Some(note), v);
                }
                _ => self.playing.drag(None),
            }
            return;
        }
        if self.fx_pin_moved(modifiers) {
            return;
        }
        // An assignment or a depth dragged (decisions.md R-LFO).
        if self.mod_moved((x, y)) {
            return;
        }
        if !self.dragging()
            && !self.learning.menu_open()
            && self
                .mod_scene
                .popup
                .as_ref()
                .is_some_and(|p| ca72_panel::modulation::on_popup(p, x, y))
        {
            self.follow_popup((x, y));
            self.playing.hover = None;
            return;
        }
        self.playing.hover = if self.expanded()
            && self.keyboard_shown()
            && self.in_drawer((x, y)).is_none()
            && !self.in_lower((x, y))
            && !self.fx_view.pressing()
            && !self.learning.over_menu(self.renderer.fonts(), (x, y))
        {
            keyboard::hit(self.playing.root, x, y)
        } else {
            None
        };
        if self.playing.hover.is_some() {
            self.hover = None;
            self.strip_scene.hover = None;
            return;
        }
        if let Some((x0, w0)) = self.resizing {
            // Asked of the host at the next frame: a resize renders the whole panel again.
            let width = clamp_width((f64::from(w0) + self.pointer.0 - x0).round().max(0.0) as u32);
            self.gripped = Some(width);
            return;
        }
        let fine = modifiers.contains(Modifiers::SHIFT);
        let pointer = self.pointer;
        // (The control dragged's kind as it is operated now: a timed FREQUENCY steps.)
        let dragged_kind = self.drag.as_ref().map(|d| self.kind_of(d.control));
        let dragged_variable = self
            .drag
            .as_ref()
            .is_some_and(|d| self.continuous_switch(d.control));
        let Some(d) = &mut self.drag else {
            let at = self.in_drawing(pointer);
            let (x, y) = at;
            if !self.expanded() && y >= DRAWER_TOP {
                self.hover = None;
                self.strip_scene.hover = None;
                self.browser.bar.hover = None;
                return;
            }
            if self.learning.menu_open() {
                self.learning.hover(self.renderer.fonts(), at);
            }
            let over_drawer = self.in_drawer(at);
            self.browser.drawer.hover =
                over_drawer.and_then(|(dx, dy)| drawer_hit(&self.browser.drawer, dx, dy));
            if self.in_arp(at) {
                let settled = self.lower.settled();
                self.arp_scene.hover = arp_ui::hit(x, y).filter(|_| settled);
                self.hover = interact::hit_arp(x, y).filter(|_| settled);
                self.strip_scene.hover = None;
                self.browser.bar.hover = None;
                return;
            }
            self.arp_scene.hover = None;
            if self.in_mpe(at) {
                let settled = self.lower.settled();
                self.hover = interact::hit_drawer(DrawerPlace::Mpe, x, y).filter(|_| settled);
                self.strip_scene.hover = None;
                self.browser.bar.hover = None;
                return;
            }
            // The effects' drawer: a slot carried, the part under the pointer.
            if self.in_fx(at) || self.fx_view.pressing() {
                if self.lower.settled() {
                    self.fx_view.moved(x, y);
                }
                self.hover = self
                    .fx_pin_hit(at)
                    .map(|t| Target::Control(fx_pins::FIRST + t));
                self.strip_scene.hover = None;
                self.browser.bar.hover = None;
                self.follow_popup(at);
                return;
            }
            self.fx_view.hover = None;
            // The macros: their knobs, else their names and jacks (decisions.md R-MACROS).
            if self.macros_show() && macro_ui::inside(x, y) && over_drawer.is_none() {
                self.macro_hover((x, y));
                return;
            }
            self.macro_view.hover = None;
            // The LFO page: its controls, else its parts (decisions.md R-LFO).
            if self.page_shows() && lfo_ui::inside(x, y) && over_drawer.is_none() {
                self.hover = interact::hit_lfo(self.renderer.layout(), x, y);
                self.modulating.hover = if self.hover.is_none() {
                    lfo_ui::hit(x, y)
                } else {
                    None
                };
                self.strip_scene.hover = None;
                self.browser.bar.hover = None;
                // (Its knobs and jacks take sources too, with their pop-ups: R-LFO's addendum.)
                self.follow_popup(at);
                return;
            }
            self.modulating.hover = None;
            let on_strip = if over_drawer.is_some() {
                None
            } else {
                strip::hit(self.rail, x, y)
            };
            self.browser.bar.hover = match on_strip {
                Some(StripTarget::Bar(t)) => Some(t),
                _ => None,
            };
            (self.hover, self.strip_scene.hover) = if over_drawer.is_some() {
                (None, None)
            } else if on_strip.is_some() {
                (None, on_strip)
            } else {
                (self.hit(x, y), None)
            };
            // A modulated knob's pop-up, kept while the pointer is over it.
            self.follow_popup(at);
            return;
        };
        if (pointer.0 - d.from.0).abs() + (pointer.1 - d.from.1).abs() > 3.0 {
            d.moved = true;
        }
        let Some(kind) = dragged_kind else { return };
        // The click threshold belongs to switches and selectors. A continuous control
        // follows even a sub-pixel movement from the press, with no initial jump.
        if !d.moved && !kind.is_continuous() {
            return;
        }
        if fine != d.fine {
            d.base = d.value;
            d.at = pointer;
            d.fine = fine;
        }
        let (dx, dy) = (pointer.0 - d.at.0, pointer.1 - d.at.1);
        let moved_to = if dragged_variable {
            Some(interact::continuous_drag(d.base, dx, dy, d.fine))
        } else {
            interact::drag(&kind, d.base, dx, dy, d.fine)
        };
        if let Some(v) = moved_to
            && v != d.value
        {
            d.value = v;
            let (i, v) = (d.control, v);
            if let Some(p) = self.operated(i) {
                p.set(&self.setter(), v as f32);
            }
        }
    }

    /// Whether a press is being held: a control, an effects' slot or the grip dragged.
    fn dragging(&self) -> bool {
        self.drag.is_some()
            || self.resizing.is_some()
            || self.playing.dragging()
            || self.fx_view.pressing()
            || self.fx_pin_drag.is_some()
            || self.tempo_drag.is_some()
    }

    fn released(&mut self) {
        self.panic_held = false;
        self.playing.release_pointer();
        self.fx_pin_drag = None;
        self.fx_view.released();
        self.tempo_let_go();
        if self.mod_released() {
            return;
        }
        if self.resizing.take().is_some() {
            return;
        }
        if let Some(d) = self.drag.take() {
            self.let_go(d, true);
        }
    }

    /// Every gesture still open ended (a wheel's, a drag's, a slider's; the grip let go):
    /// before another begins, as the editor closes and as its window loses the focus or the
    /// pointer's capture. A gesture never begins inside another (decisions.md R18).
    fn end_gestures(&mut self) {
        self.panic_held = false;
        self.playing.release_pointer();
        self.fx_view.cancel_press();
        self.fx_pin_drag = None;
        self.tempo_let_go();
        self.typing = None;
        self.knob_entry = None;
        self.end_scroll();
        self.resizing = None;
        // (An assignment dragged is dropped nowhere; a depth's drag ends where it is.)
        self.modulating.patching = None;
        self.mod_released();
        if let Some(d) = self.drag.take() {
            self.let_go(d, false);
        }
    }

    /// A drag over, its gesture ended: `released` by the pointer (a press that did not move
    /// then picks what it pressed), else cut short.
    fn let_go(&mut self, d: Drag, released: bool) {
        let placed = *self.renderer.arrangement().control(d.control);
        let kind = self.kind_of(d.control);
        let mut v = d.value;
        if released && !d.moved {
            let variable = self.continuous_switch(d.control);
            let picked = match d.pressed {
                Target::Legend(_, k) if variable => Some(interact::continuous_legend(k)),
                Target::Legend(_, k) => Some(k as f64 / 5.0),
                _ => {
                    let (cx, cy) = placed.centre();
                    let (x, y) = self.in_drawing(d.from);
                    if variable {
                        Some(interact::continuous_click(v, x - cx))
                    } else {
                        interact::click(&kind, v, x - cx, y - cy)
                    }
                }
            };
            if let Some(p) = picked {
                v = p;
            }
        }
        v = interact::release(&kind, v);
        let Some(p) = self.operated(d.control) else {
            return;
        };
        let s = self.setter();
        if matches!(kind, Kind::Rocker { .. } | Kind::Toggle { .. } | Kind::Key) {
            if v != d.value {
                p.begin(&s);
                p.set(&s, v as f32);
                p.end(&s);
            }
        } else {
            if v != d.value {
                p.set(&s, v as f32);
            }
            p.end(&s);
        }
    }

    fn scrolled(&mut self, delta: ScrollDelta, modifiers: Modifiers) {
        if let Some(notes) = &mut self.whats_new {
            // A line for each line's travel, the rest kept for the next event, as the
            // drawer's list; a turn the other way starts again (PR #106's review: a
            // trackpad's fraction left from one way had eaten the first of the other).
            let lines = match delta {
                ScrollDelta::Lines { y, .. } => f64::from(y),
                ScrollDelta::Pixels { y, .. } => {
                    f64::from(y) * self.dpr / self.renderer.scale() / notes.line_height()
                }
            };
            if lines * self.list_travel < 0.0 {
                self.list_travel = 0.0;
            }
            self.list_travel += lines;
            let whole = self.list_travel.trunc();
            if whole != 0.0 {
                notes.scroll(-(whole as i32));
                self.list_travel -= whole;
                self.composed = None;
            }
            return;
        }
        if self.standalone_dialog_open() || self.knob_entry.is_some() {
            return;
        }
        if self.drag.is_some() {
            return;
        }
        let (x, y) = self.in_drawing(self.pointer);
        // The MIDI map mode turns nothing (decisions.md R-MIDI-MAP): the presets' drawer and the
        // effects' list scroll as ever.
        if self.mapping_on() && self.in_drawer((x, y)).is_none() && !self.fx_view.list_open() {
            return;
        }
        // The TEMPO screen: a beat a minute a notch (up faster), or a trackpad's travel.
        if self.playing.standalone && art::tempo_hit(self.renderer.arrangement(), x, y) {
            let beats = match delta {
                ScrollDelta::Lines { y, .. } => f64::from(y.signum()),
                ScrollDelta::Pixels { y, .. } => {
                    if f64::from(y) * self.tempo_travel < 0.0 {
                        self.tempo_travel = 0.0;
                    }
                    self.tempo_travel += f64::from(y) / TEMPO_SCROLL;
                    let whole = self.tempo_travel.trunc();
                    self.tempo_travel -= whole;
                    whole
                }
            };
            if beats != 0.0 {
                self.set_tempo(self.tempo_now() + beats, true);
            }
            return;
        }
        if self.keyboard_shown() && keyboard::inside(x, y) && !self.in_lower((x, y)) {
            return;
        }
        if let Some((dx, dy)) = self.in_drawer((x, y)) {
            // The list (or the tags, under the pointer) scrolls a row for each row's travel, the rest kept for the next: a
            // wheel's lines (a slow turn on macOS gives fractions of one), a trackpad's or a
            // Magic Mouse's points as the list's rows at this size (each event a few points,
            // which a whole-notch threshold had dropped). A turn the other way starts again.
            let rows = match delta {
                ScrollDelta::Lines { y, .. } => f64::from(y),
                ScrollDelta::Pixels { y, .. } => {
                    f64::from(y) * self.dpr / self.renderer.scale() / ROW_H
                }
            };
            if rows * self.list_travel < 0.0 {
                self.list_travel = 0.0;
            }
            self.list_travel += rows;
            let whole = self.list_travel.trunc();
            if whole != 0.0 {
                if self.learning.list {
                    self.learning.scroll(-(whole as i32));
                } else if in_tags(dx, dy) {
                    self.browser.scroll_tags(-(whole as i32));
                } else {
                    self.browser.scroll(-(whole as i32));
                }
                self.list_travel -= whole;
            }
            return;
        }
        if !self.expanded() && y >= DRAWER_TOP {
            return;
        }
        // The effects' list scrolls a row for each row's travel, as the presets' list does.
        if self.in_fx((x, y)) {
            if let Some(t) = self.fx_pin_hit((x, y)) {
                let value = self.fx_view.pin_info(t).map_or(0.0, |p| p.value);
                let step = self
                    .fx_view
                    .pin_info(t)
                    .filter(|p| p.steps > 0)
                    .map_or(0.01, |p| 1.0 / f64::from(p.steps));
                let amount = match delta {
                    ScrollDelta::Lines { y, .. } => f64::from(y),
                    ScrollDelta::Pixels { y, .. } => f64::from(y) / 40.0,
                };
                self.fx_view.set_pin_value(t, value + amount * step);
                return;
            }
            if self.fx_view.list_open() {
                let rows = match delta {
                    ScrollDelta::Lines { y, .. } => f64::from(y),
                    ScrollDelta::Pixels { y, .. } => {
                        f64::from(y) * self.dpr / self.renderer.scale() / fx_ui::LINE
                    }
                };
                if rows * self.list_travel < 0.0 {
                    self.list_travel = 0.0;
                }
                self.list_travel += rows;
                let whole = self.list_travel.trunc();
                if whole != 0.0 {
                    self.fx_view.scroll(-(whole as i32));
                    self.list_travel -= whole;
                }
            }
            return;
        }
        let over = if self.in_arp((x, y)) {
            interact::hit_arp(x, y).filter(|_| self.lower.settled())
        } else if self.in_mpe((x, y)) {
            interact::hit_drawer(DrawerPlace::Mpe, x, y).filter(|_| self.lower.settled())
        } else if self.page_shows() && lfo_ui::inside(x, y) {
            // The LFO page's (decisions.md R-LFO), its column's LFO the last touched.
            let t = interact::hit_lfo(self.renderer.layout(), x, y);
            if let Some(Target::Control(i) | Target::Legend(i, _)) = t
                && let Some((n, _)) = lfo_page::lfo_part(CONTROLS[i].param)
            {
                self.modulating.last = Some(crate::lfo::Source::Lfo(n));
            }
            t
        } else if strip::hit(self.rail, x, y).is_some() {
            return;
        } else {
            self.hit(x, y)
        };
        let Some(Target::Control(i) | Target::Legend(i, _)) = over else {
            return;
        };
        if !self.operable(i) {
            return;
        }
        let travel = scroll_travel(delta, self.precise_lines);
        if !travel.is_finite() || travel == 0.0 {
            return;
        }
        if self.scroll.as_ref().is_some_and(|s| s.control != i) {
            self.end_scroll();
        }
        if self.scroll.is_none() {
            let value = self.value_of(i);
            if let Some(p) = self.operated(i) {
                p.begin(&self.setter());
            }
            self.scroll = Some(Scroll {
                control: i,
                value,
                last: Instant::now(),
                travel: 0.0,
            });
        }
        let kind = self.kind_of(i);
        let Some(s) = &mut self.scroll else { return };
        s.last = Instant::now();
        let notches = if kind.is_continuous() {
            travel / SCROLL_PIXELS_PER_LINE
        } else {
            // Only discrete options wait for a whole notch. Keep excess travel, and start
            // afresh when the direction changes so the old fraction cannot eat a step.
            if travel * s.travel < 0.0 {
                s.travel = 0.0;
            }
            s.travel += travel;
            let whole = (s.travel / SCROLL_PIXELS_PER_LINE).trunc();
            s.travel -= whole * SCROLL_PIXELS_PER_LINE;
            whole
        };
        let variable = self.continuous_switch(i);
        let Some(s) = &mut self.scroll else { return };
        if notches != 0.0 {
            let fine = modifiers.contains(Modifiers::SHIFT);
            let v = if variable {
                interact::continuous_scroll(s.value, notches, fine)
            } else {
                interact::scroll(&kind, s.value, notches, fine)
            };
            if v != s.value {
                s.value = v;
                if let Some(p) = self.operated(i) {
                    p.set(&self.setter(), v as f32);
                }
            }
        }
    }

    fn end_scroll(&mut self) {
        let Some(s) = self.scroll.take() else { return };
        let kind = self.kind_of(s.control);
        let v = interact::release(&kind, s.value);
        if let Some(p) = self.operated(s.control) {
            let setter = self.setter();
            if v != s.value {
                p.set(&setter, v as f32);
            }
            p.end(&setter);
        }
    }

    /// The width the grip took the window to since the last frame, asked of the host.
    fn grip(&mut self) {
        if let Some(width) = self.gripped.take() {
            self.resize(width);
        }
    }

    /// Asks the host for a window `width` wide: whether it agreed.
    fn resize(&mut self, width: u32) -> bool {
        self.reconcile_resize();
        if width == self.width {
            return false;
        }
        let old = self.params.editor_width.swap(width, Ordering::Relaxed);
        let agreed = self.ask_resize(width);
        if agreed {
            self.width = width;
            self.resizing_window = true;
        } else {
            self.params.editor_width.store(old, Ordering::Relaxed);
        }
        self.params.editor_resizing.finish(self.resize_view());
        agreed
    }

    /// The scene from the parameters, the controls operated and what the audio reports.
    fn update_scene(&mut self) {
        self.playing.sync();
        if self.playing.active()
            && !self.note_typing_allowed()
            && (self.browser.open
                || self.typing.is_some()
                || self.macro_view.typing.is_some()
                || self.knob_entry.is_some()
                || self.learning.holds_keys(&self.params.midi_map)
                || self.fx_view.list_open()
                || self.standalone_dialog_open())
        {
            self.playing.release();
        }
        for (i, c) in CONTROLS.iter().enumerate() {
            // A WAVEFORM switch CONTINUOUS: where its WAVEFORM is (its pointer where its shape is,
            // `scene.shape`), the parameters following a drag as it goes.
            if self.continuous_switch(i)
                && let Some(n) = ca72_panel::render::WAVEFORMS
                    .iter()
                    .position(|w| *w == c.param)
            {
                self.scene.values[i] = f64::from(self.waveforms[n].waveform().value());
                continue;
            }
            self.scene.values[i] = match (&self.drag, &self.scroll) {
                (Some(d), _)
                    if d.control == i
                        && !matches!(
                            c.kind,
                            Kind::Rocker { .. } | Kind::Toggle { .. } | Kind::Key
                        ) =>
                {
                    d.value
                }
                (_, Some(s)) if s.control == i => s.value,
                _ => self.value_of(i),
            };
        }
        self.scene.power = !self.params.bypass.value();
        self.scene.detune_off = !self.params.doubled.value();
        self.scene.voices_off = voices_inert(&self.params);
        self.scene.osc3_control_off = osc3_control_inert(&self.params);
        // Oscillators 2's and 3's timed keys and dials (decisions.md R-OSC-TIMED).
        self.scene.timed = timed_scene(&self.params);
        self.scene.glide_long = self.params.glide_range.value() == crate::params::GlideRange::Long;
        self.learning.glide_long = self.scene.glide_long;
        // Each oscillator's waveform: CONTINUOUS's arc, its MOD key in its way's colour, its
        // switch where its shape is, MOD's orb where the voices have the shape (decisions.md
        // R-CONTINUOUS-WAVE).
        let orbs = self.meters.shapes();
        self.scene.shape = std::array::from_fn(|n| {
            use crate::params::ShapeModDirection as D;
            let (_, shape, modulated, direction) = self.params.shape_params(n);
            let continuous = self.params.continuous(n);
            ca72_panel::render::Shaped {
                continuous,
                at: f64::from(shape.unmodulated_normalized_value()) * interact::CONTINUOUS_PLACES,
                way: match direction.value() {
                    D::Clockwise => ca72_panel::art::ModWay::Clockwise,
                    D::Anticlockwise => ca72_panel::art::ModWay::Anticlockwise,
                    D::Both => ca72_panel::art::ModWay::Both,
                },
                orb: orbs[n].filter(|_| continuous && modulated.value()),
            }
        });
        self.learning.variable = std::array::from_fn(|n| self.params.continuous(n));
        self.learning.timed = [self.params.timed_now(1), self.params.timed_now(2)];
        self.learning.lfo_timed = std::array::from_fn(|n| self.params.lfo(n).timed.value());
        self.scene.overload = Meters::load(&self.meters.overload);
        // (Dark until the velocity moves something: decisions.md R-VELOCITY.)
        self.scene.velocity_lit = if self.params.lfo_patch.count(crate::lfo::Source::Velocity) > 0 {
            ca72_panel::strip::velocity::lit(self.meters.velocity())
        } else {
            0
        };
        self.move_opening(Instant::now(), self.params.quality.value());
        self.scene.midi = (
            Meters::load(&self.meters.bend),
            Meters::load(&self.meters.modulation),
        );
        self.read_realtime(Instant::now());
        self.scene.tip = self.tip();
        self.scene.entry = self
            .knob_entry
            .as_ref()
            .map(|entry| self.entry_field(entry));
        if self.scene.entry.is_some() {
            self.scene.tip = None;
        }
        // The screens of the controls a source moves read where it has them (decisions.md
        // R-LFO), the TEMPO screen the moving tempo; each its control's own while that is
        // operated or typed into.
        self.read_live(Instant::now());
        self.scene.tempo = self
            .playing
            .standalone
            .then(|| format!("{} BPM", self.tempo_shown()));
        self.scene.tempo_keys = self.playing.standalone.then_some(TempoKeys {
            typing: self.playing.typing,
            panic: self.panic_held,
            midi: self.mapping_on(),
            hover: self.tempo_hover,
        });
        let live = self.live_shown();
        self.strip_scene = strip_scene(
            &self.params,
            self.meters.voices.load(Ordering::Relaxed),
            (&self.levels, self.output),
            &live,
            std::mem::take(&mut self.strip_scene),
        );
        // (The LFO page in the keyboard's place while it shows: decisions.md R-LFO.)
        self.strip_scene.keyboard = if self.modulating.open {
            None
        } else {
            self.playing.scene(self.renderer.skin())
        };
        self.strip_scene.lfo = self.modulating.open;
        self.browser.bar.keyboard = self.keyboard_shown();
        self.browser.bar.panel = self.expanded();
        self.browser.bar.arp = self.params.arp_on.value();
        // The effects' (decisions.md R-FX-DRAWER): the chain read when it is free; the FX key lit
        // while one plays.
        self.fx_view.poll(Instant::now());
        self.browser.bar.fx = self.fx_view.active();
        self.fx_scene = self.fx_scene_now();
        self.learning.fx_pins = self.fx_view.pins().to_vec();
        self.arp_scene = arp_scene(
            &self.params,
            &self.scene.values,
            std::mem::take(&mut self.arp_scene),
        );
        // Each note's own expression's (decisions.md R-MPE): the MPE key lit while an amount is
        // not 0; INPUT reading what the audio thread heard in the last few seconds.
        self.browser.bar.mpe = reaching(&self.params);
        // The rail's page keys (decisions.md R-LFO): a page's or a drawer's lit while it shows,
        // the others glowing faintly while what they open affects the sound (an LFO assigned and
        // moving its knob).
        self.browser.bar.lfo = self.params.lfo_scope.any_moving();
        self.browser.bar.lfo_page = self.page_shows();
        // The MACRO key's (decisions.md R-MACROS): lit while the macros show, glowing while the
        // wheels do and an assigned macro is above 0.
        self.browser.bar.macros = self.macros_show();
        self.browser.bar.macro_glow = self.macro_glow();
        self.browser.bar.page =
            (self.lower.open && self.expanded()).then_some(match self.lower.which {
                Lower::Arp => BarTarget::Arp,
                Lower::Mpe => BarTarget::Mpe,
                Lower::Fx => BarTarget::Fx,
            });
        // The LFO page's (its orbs at most 30 times a second) and the knobs' rings, cord and
        // pop-up.
        self.read_lfos(Instant::now());
        if self.page_shows() {
            self.lfo_scene = self.lfo_scene_now();
        }
        self.mod_scene = self.mod_scene_now();
        if self.macros_show() {
            self.macro_scene = self.macro_scene_now();
        }
        let now = Instant::now();
        let counts = self.params.heard.counts();
        let (was, per_note, channel) = self.heard;
        self.heard = (
            counts,
            if counts.0 != was.0 {
                Some(now)
            } else {
                per_note
            },
            if counts.1 != was.1 {
                Some(now)
            } else {
                channel
            },
        );
        let recent = |t: Option<Instant>| t.is_some_and(|t| now - t < HEARD_FOR);
        let input = if recent(self.heard.1) {
            mpe_ui::Input::Mpe
        } else if recent(self.heard.2) {
            mpe_ui::Input::Channel
        } else {
            mpe_ui::Input::None
        };
        self.mpe_scene = mpe_scene(
            &self.params,
            &self.scene.values,
            input,
            self.meters.velocity(),
            self.playing.standalone,
        );
        // A value being typed: what is typed so far, lit, the next digit's place marked while
        // there is room for one.
        if let Some((r, text)) = &self.typing {
            let cells = text.chars().filter(|c| *c != '.').count();
            let room = match r {
                Typed::Arp(0) => cells < 2 && !text.ends_with(['.', 't']),
                Typed::Arp(1) => cells < 1,
                Typed::Mpe(_) => text.chars().filter(char::is_ascii_digit).count() < 3,
                _ => cells < 3,
            };
            let shown = if room {
                format!("{text}_")
            } else {
                text.clone()
            };
            match r {
                Typed::Strip(r) => {
                    if let Some(i) = Readout::ALL.iter().position(|x| x == r) {
                        self.strip_scene.readouts[i] = (shown, true);
                    }
                }
                Typed::Arp(i) => self.arp_scene.readouts[(*i).min(3)] = (shown, true),
                Typed::Mpe(i) => {
                    self.mpe_scene.readouts[(*i).min(mpe_ui::UNIT_COUNT - 1)] = (shown, true);
                }
                Typed::Tempo => self.scene.tempo = Some(format!("{shown} BPM")),
            }
        }
    }

    /// The tip: over the control operated, else over the one under the pointer (an effects'
    /// slot's: its effect's whole name and maker); else, for a while once, the standalone's
    /// audio not real-time, on the walnut (decisions.md R-RTKIT).
    fn tip(&self) -> Option<(String, f64, f64)> {
        self.pointed_tip().or_else(|| self.realtime_tip())
    }

    /// The standalone's audio threads' priority read (at each frame, from the audio side's
    /// atomics): the warning while one is due, and the walnut's notice begun the first time.
    fn read_realtime(&mut self, now: Instant) {
        if !self.playing.standalone {
            return;
        }
        let (audio, workers) = (self.realtime.read)();
        self.realtime.warning =
            crate::audio_thread::warning(audio, workers, cfg!(target_os = "linux"));
        if self.realtime.warning.is_some() && self.realtime.since.is_none() {
            self.realtime.since = Some(now);
        }
    }

    /// The walnut's notice that the standalone's audio is not real-time, after its TYPING, MIDI
    /// and PANIC keys as their tips are: for [`REALTIME_NOTICE`] once, while nothing else's
    /// tip is shown.
    fn realtime_tip(&self) -> Option<(String, f64, f64)> {
        let since = self.realtime.since?;
        if since.elapsed() >= REALTIME_NOTICE {
            return None;
        }
        let text = self.realtime.warning.as_ref()?.tip.clone();
        let b = art::tip_bounds(self.renderer.fonts(), &text, self.renderer.text_size());
        let (px, py) = art::tempo_key_at(self.renderer.arrangement(), art::TempoKey::LAST);
        let left = px + art::TEMPO_KEY.0 / 2.0 + 12.0;
        Some((text, left - b[0], py - (b[1] + b[3]) / 2.0))
    }

    /// The tip of what the pointer is on or operates.
    fn pointed_tip(&self) -> Option<(String, f64, f64)> {
        if let Some(t) = self.fx_pin_tip() {
            return Some(t);
        }
        if self.drag.is_none()
            && let Some(t) = self.macro_tip()
        {
            return Some(t);
        }
        if self.drag.is_none()
            && let Some(t) = self.velocity_tip()
        {
            return Some(t);
        }
        if self.drag.is_none()
            && let Some(t) = self.page_tip()
        {
            return Some(self.clear_of_scopes(t));
        }
        if self.lower.which == Lower::Fx
            && self.lower.settled()
            && self.drag.is_none()
            && let Some(t) = self.fx_view.tip()
        {
            return Some(t);
        }
        // The standalone's TEMPO screen with a source on it: its tempo as set, and the depths
        // (R-LFO; it reads the moving tempo).
        if self.drag.is_none()
            && self.tempo_drag.is_none()
            && self.tempo_hovered(self.in_drawing(self.pointer))
            && !self.sources_on(crate::lfo::Target::Tempo).is_empty()
        {
            // (Beside the screen, to its right: above it there is no room in the window.)
            let (_, y, _, h) = art::tempo_window(self.renderer.arrangement().plate_at());
            let text = format!(
                "TEMPO: {} BPM{}",
                self.tempo_now(),
                self.depths_tip(lfo_page::TEMPO_CONTROL)
            );
            let b = art::tip_bounds(self.renderer.fonts(), &text, self.renderer.text_size());
            // (After its TYPING, PANIC and MIDI keys: decisions.md R-TEMPO-CORNER.)
            let (px, _) = art::tempo_key_at(self.renderer.arrangement(), art::TempoKey::LAST);
            let left = px + art::TEMPO_KEY.0 / 2.0 + 12.0;
            return Some((text, left - b[0], y + h / 2.0 - (b[1] + b[3]) / 2.0));
        }
        // TYPING's key is a picture: its tip says what it does, after the keys, as TEMPO's; and
        // MIDI's what its mode is (decisions.md R-MIDI-MAP).
        let tempo_tip = match self.tempo_hover {
            Some(art::TempoKey::Typing) => Some("TYPING: PLAY FROM THE COMPUTER KEYBOARD"),
            Some(art::TempoKey::Midi) => Some(mapping::KEY_TIP),
            _ => None,
        };
        if self.drag.is_none()
            && let Some(text) = tempo_tip
        {
            let b = art::tip_bounds(self.renderer.fonts(), text, self.renderer.text_size());
            let (px, py) = art::tempo_key_at(self.renderer.arrangement(), art::TempoKey::LAST);
            let left = px + art::TEMPO_KEY.0 / 2.0 + 12.0;
            return Some((text.into(), left - b[0], py - (b[1] + b[3]) / 2.0));
        }
        let (px, py) = self.in_drawing(self.pointer);
        let (nx, ny, nw, nh) = self.rail.name();
        if self.browser.bar.preset_fx
            && (nx + nw - 140.0..nx + nw - 52.0).contains(&px)
            && (ny..ny + nh).contains(&py)
        {
            return Some((
                "This preset has its own effects. Unlock to hear them.".into(),
                nx + nw - 95.0,
                ny - 12.0,
            ));
        }
        if self.drag.is_none()
            && let Some(key) = self.on_rail((px, py))
        {
            let text = match key {
                BarTarget::Global => Some("GLOBAL: VOICES, STEREO AND OUTPUT"),
                BarTarget::Keyboard => Some("KEYS: PLAYABLE PIANO"),
                BarTarget::Lfo => Some("LFO: MODULATION SOURCES"),
                _ => None,
            };
            if let Some(text) = text {
                let (x, y) = strip::centre(self.rail, StripTarget::Bar(key));
                return Some((text.into(), x, y - strip::KEY_H / 2.0 - 12.0));
            }
        }
        let operated = self
            .drag
            .as_ref()
            .map(|d| d.control)
            .or(self.scroll.as_ref().map(|s| s.control));
        let target = match operated {
            Some(i) => Target::Control(i),
            None => self.hover?,
        };
        match target {
            // (None for a control gone from the panel under a pointer still: R-OSC-TIMED.)
            Target::Control(i) | Target::Legend(i, _)
                if self.off_panel(i) || fx_pins::target(i).is_some() =>
            {
                None
            }
            Target::Control(i) | Target::Legend(i, _) => {
                let c = *self.renderer.arrangement().control(i);
                let p = self.operated(i)?;
                let (x, y) = c.centre();
                let top = art::bounds(&c.kind)[1];
                let named = macros::macro_of_control(i).map(|n| self.macro_title(n));
                let label = if c.param == "feedback" && feedback_silent(&self.scene.values) {
                    FEEDBACK_SILENT
                } else if let Some(name) = &named {
                    name
                } else {
                    timed_label(&self.params, c.param)
                        .or_else(|| mod_label(&self.params, c.param))
                        .unwrap_or(c.label)
                };
                // (A CONTINUOUS switch's shape, which its scene's value, WAVEFORM's, is not.)
                let v = if self.continuous_switch(i) {
                    p.value()
                } else {
                    self.scene.values[i] as f32
                };
                let range = if c.param == "glide" && self.scene.glide_long {
                    " · LONG"
                } else {
                    ""
                };
                // (An LFO's OUT jack is its LEVEL's: what the jack does after: R-LFO's
                // addendum.)
                let jack = match lfo_page::level_of_control(i) {
                    Some(n) => format!(" \u{b7} LFO {} OUT: DRAG TO A KNOB TO MODULATE IT", n + 1),
                    None => String::new(),
                };
                // (The sources on it and their depths: decisions.md R-LFO.)
                let tip = (
                    format!("{label}: {}{range}{}{jack}", p.text(v), self.depths_tip(i)),
                    x,
                    y + top,
                );
                Some(if c.param == "retrigger" {
                    let text = format!(
                        "RETRIGGER: {} · On: each key strikes.\nNOTE PRIORITY LAST: new pitch with attack.\nLOW/HIGH: a held key may keep the old pitch until lifted.",
                        p.text(v)
                    );
                    let b =
                        art::tip_bounds(self.renderer.fonts(), &text, self.renderer.text_size());
                    (text, x.clamp(-b[0], art::W - b[2]), tip.2)
                } else if c.place == DrawerPlace::Lfo {
                    self.clear_of_scopes(tip)
                } else {
                    tip
                })
            }
            Target::Power => {
                let state = if self.params.bypass.value() {
                    "Off (bypassed)"
                } else {
                    "On"
                };
                Some((
                    format!("POWER: {state}"),
                    self.renderer.arrangement().power_at().0,
                    self.renderer.arrangement().power_at().1 - 64.0,
                ))
            }
            Target::Grip => {
                // Above the grip, its right end at the window's (R-SIZE).
                let size = self.renderer.text_size();
                let w = self
                    .renderer
                    .fonts()
                    .advance(GRIP_TIP, size, Weight::Regular, 0.0)
                    + 0.8 * size;
                let top = if self.expanded() {
                    art::GRIP[1]
                } else {
                    f64::from(self.physical_height()) / self.renderer.scale() + self.renderer.top()
                        - COMPACT_GRIP
                };
                Some((GRIP_TIP.to_owned(), art::W - 8.0 - w / 2.0, top))
            }
        }
    }

    /// Whether the pointer is over the grip, or dragging it.
    fn on_grip(&self) -> bool {
        self.resizing.is_some() || self.hover == Some(Target::Grip)
    }

    /// Brings the frame up to the parameters and the gestures: whether it changed.
    fn draw(&mut self) -> bool {
        self.reconcile_resize();
        self.follow_panel();
        let preference = self.params.editor_panel.load(Ordering::Relaxed);
        let applied = self.params.editor_panel_applied.load(Ordering::Relaxed);
        if preference != applied {
            if !self
                .set_panel_height(preference || self.browser.open || self.standalone_dialog_open())
            {
                self.params.editor_panel.store(applied, Ordering::Relaxed);
            }
            self.params.editor_panel_applied.store(
                self.params.editor_panel.load(Ordering::Relaxed),
                Ordering::Relaxed,
            );
            self.params.editor_resizing.finish(self.resize_view());
        }
        #[cfg(feature = "standalone-production")]
        {
            if let Some(mut dialog) = crate::standalone_settings::take_requested()
                && self.standalone_dialog.is_none()
            {
                self.playing.release();
                self.end_gestures();
                dialog.set_tempo(self.settings.tempo);
                self.standalone_dialog = Some(dialog);
                self.playing.wants_keys = true;
                self.composed = None;
            }
            if let Some(dialog) = &mut self.standalone_dialog
                && dialog.frame()
            {
                self.composed = None;
            }
            // TEMPO changed there: the arpeggiator's at once, and this computer's (R-ARP).
            if let Some(bpm) = self
                .standalone_dialog
                .as_mut()
                .and_then(crate::standalone_settings::Dialog::take_tempo)
            {
                self.keep_settings(Change::Tempo(bpm));
                self.params.set_tempo(f64::from(bpm));
            }
        }
        let requested_height = self.params.editor_panel.load(Ordering::Relaxed)
            || self.browser.open
            || self.standalone_dialog_open();
        if requested_height != self.expanded() {
            self.set_panel_height(requested_height);
        }
        if self
            .scroll
            .as_ref()
            .is_some_and(|s| s.last.elapsed() > SCROLL_REST)
        {
            self.end_scroll();
        }
        self.update_scene();
        self.browser.tick(&self.params);
        self.update.tick();
        self.browser.drawer.update = self.update.scene();
        // MIDI Learn: what the audio thread caught assigned; the ring, the note and the menu;
        // the drawer's list (the presets again once the drawer has shut). A tip gives way to a
        // note or a menu.
        self.poll_midi_learn();
        let map = &self.params.midi_map;
        let bottom = if self.expanded() { art::H } else { DRAWER_TOP };
        self.learning.fit_menu(
            self.renderer.fonts(),
            [0.0, self.renderer.top(), art::W, bottom],
        );
        let (note, ring, strip_ring, menu) = self.learning.scene(map, self.renderer.text_size());
        let note = note.or_else(|| self.ultra_note.then(|| self.ultra_note()));
        // (The MIDI map mode's marks ring the control waiting themselves: R-MIDI-MAP.)
        let mapping = self.mapping_on();
        let ring = ring.filter(|&i| !self.off_panel(i) && !mapping);
        (self.scene.note, self.scene.learning, self.scene.menu) = (note, ring, menu);
        // The MIDI map mode's marks, over everything the controls are on (R-MIDI-MAP).
        self.scene.marks = self.map_marks();
        self.strip_scene.learning = strip_ring.filter(|_| self.strip_scene.extension() && !mapping);
        if self.scene.note.is_some() || self.scene.menu.is_some() {
            self.scene.tip = None;
        }
        if !self.browser.open && !self.browser.sliding() {
            self.learning.list = false;
        }
        self.browser.drawer.midi = self
            .learning
            .list
            .then(|| self.learning.list_scene(&self.params.midi_map));
        let notes_height = f64::from(self.physical_height()) / self.renderer.scale();
        if let Some(notes) = &mut self.whats_new {
            notes.fit(self.renderer.fonts(), notes_height, self.settings.lettering);
        }
        let panel = self.renderer.render(&self.scene);
        self.strip_scene.bar.clone_from(&self.browser.bar);
        // (The strip's parts drawn again over the panel's frame where it changed there: the
        // panel's places that reach the strip, not QUALITY's opening above it as well.)
        // (The drops move on with QUALITY's opening, on its clock: the two in one frame.)
        let foot = self.renderer.strip_row();
        let under = self
            .renderer
            .damages()
            .iter()
            .copied()
            .filter(|d| d[3] > foot)
            .reduce(|a, b| {
                [
                    a[0].min(b[0]),
                    a[1].min(b[1]),
                    a[2].max(b[2]),
                    a[3].max(b[3]),
                ]
            });
        let strip = self.strip.render_at(
            &self.strip_scene,
            self.renderer.frame(),
            under,
            self.motion_at.unwrap_or_else(Instant::now),
        );
        // The LFO page over the lower page while it shows (decisions.md R-LFO; its controls drawn
        // as the panel's renderer draws controls), and modulation's rings, cord and pop-up over
        // the panel.
        let page_shown = self.page_shows();
        let page = page_shown && self.lfo.render(&self.lfo_scene, &mut self.renderer);
        // The macros in the left area while they show (decisions.md R-MACROS), on the strip's
        // bare face there, their knobs and jacks drawn as the panel's renderer draws them.
        // (Their still made on a thread of their own as the editor opens or its look changes.)
        self.macros.prepare(&self.renderer);
        let macros_shown = self.macros_show();
        let macros = macros_shown && self.macros.render(&self.macro_scene, &mut self.renderer);
        self.modulation
            .rescale(self.renderer.scale(), self.renderer.top_px());
        let mods = self.modulation.render(&self.mod_scene);
        // The drawer dropped from the rail over the lower page while it shows: the arpeggiator's
        // (decisions.md R-ARP; its knobs drawn as the panel's renderer draws controls) or the
        // effects' (R-FX-DRAWER).
        self.lower.tick();
        let which = self.lower.which;
        let lower_k = self.lower_top().map_or(0.0, |_| self.lower.reveal());
        let lower_moved = (lower_k, which) != self.lower_shown;
        let lower = lower_k > 0.0
            && (match (which, &mut self.fx) {
                (Lower::Fx, Some(fx)) => fx.render(&self.fx_scene),
                (Lower::Mpe, _) => self.mpe.render(&self.mpe_scene, &mut self.renderer),
                _ => self.arp.render(&self.arp_scene, &mut self.renderer),
            } || self.lower.sliding()
                || lower_moved);
        let lower_shut = lower_k <= 0.0 && lower_moved;
        // The drawer over the strip, dropping down from under the rail, while it shows.
        let k = self.browser.reveal();
        // (Its place changed since the frame last shown is a change too: the slide's last frame
        // stops short of its end, which the frame shown must still reach.)
        let moved = k != self.reveal_shown;
        let drawer = k > 0.0
            && (self.drawer.render(&self.browser.drawer) || self.browser.sliding() || moved);
        let shut = k <= 0.0 && moved;
        if !(panel
            || strip
            || drawer
            || shut
            || lower
            || lower_shut
            || page
            || macros
            || mods
            || self.composed.is_none())
        {
            return false;
        }
        self.reveal_shown = k;
        self.lower_shown = (lower_k, which);
        // The places that changed, the panel's and the strip's, put together again alone and
        // so shown (decisions.md R58); all of the frame while the drawer moves, something
        // floats over everything, or there is no frame of this size to change.
        let height = self.physical_height();
        let kept = self
            .composed
            .as_ref()
            .is_some_and(|c| (c.width(), c.height()) == (self.physical_width, height));
        let lower_moving = lower_moved || lower_shut || self.lower.sliding();
        self.damage = (kept
            && !(drawer
                || shut
                || lower_moving
                || self.renderer.floating()
                || self.standalone_dialog_open()
                || self.whats_new.is_some()))
        .then(|| {
            let mut places = self.renderer.damages().to_vec();
            if let Some([x0, y0, x1, y1]) = self.strip.damage() {
                places.push([x0, foot + y0, x1, foot + y1]);
            }
            let lower_damage = match (which, &self.fx) {
                (Lower::Fx, Some(fx)) => fx.damage(),
                (Lower::Mpe, _) => self.mpe.damage(),
                _ => self.arp.damage(),
            };
            if page {
                // (Its rows the drawing's from its top; the frame's begin at the panel's top.)
                let top = lfo_ui::top_px(self.renderer.scale()) - self.renderer.top_px();
                for &[x0, y0, x1, y1] in self.lfo.damages() {
                    places.push([x0, top + y0, x1, top + y1]);
                }
            }
            if macros {
                // (Its pixels the frame's from the area's top left corner.)
                let r = self.renderer.left_rect();
                for &[x0, y0, x1, y1] in self.macros.damages() {
                    places.push([x0 + r[0], y0 + r[1], x1 + r[0], y1 + r[1]]);
                }
            }
            if mods {
                places.extend_from_slice(self.modulation.damages());
            }
            if lower && let Some([x0, y0, x1, y1]) = lower_damage {
                // (Its rows the drawing's; the frame's begin at the panel's top.)
                let top = lower_ui::top_px(self.renderer.scale()) - self.renderer.top_px();
                places.push([x0, top + y0, x1, top + y1]);
            }
            if !self.expanded() {
                let side = (30.0 * self.renderer.scale()).ceil() as i32;
                places.push([
                    self.physical_width as i32 - side,
                    height as i32 - side,
                    self.physical_width as i32,
                    height as i32,
                ]);
            }
            places
        });
        let expanded = self.expanded();
        let lower_top = self.lower_top();
        let scale = self.renderer.scale();
        compose(
            &mut self.composed,
            &self.renderer,
            self.strip.frame(),
            lower_top.map(|top| {
                lower_frame(which, top, (&self.arp, &self.mpe), self.fx.as_ref(), scale)
            }),
            (k > 0.0).then(|| (self.drawer.frame(), DRAWER_TOP - (1.0 - k) * DRAWER_H)),
            (
                page_shown.then(|| self.lfo.frame()),
                macros_shown.then(|| (self.macros.frame(), self.renderer.left_rect())),
                &self.modulation,
            ),
            self.damage.as_deref(),
            expanded,
            (self.physical_width, height),
        );
        #[cfg(feature = "standalone-production")]
        if let Some(dialog) = &self.standalone_dialog
            && let Some(frame) = &mut self.composed
        {
            dialog.draw(frame, self.renderer.scale());
        }
        if let Some(notes) = &mut self.whats_new
            && let Some(frame) = &mut self.composed
        {
            notes.draw(frame, self.renderer.scale(), self.renderer.fonts());
        }
        true
    }

    /// The pointer's events: whether the event was the panel's.
    fn mouse(&mut self, e: MouseEvent) -> bool {
        self.reconcile_resize();
        match e {
            MouseEvent::CursorMoved {
                position,
                modifiers,
            } => {
                self.pointer = (position.x, position.y);
                self.moved(modifiers);
            }
            MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                modifiers,
            } => self.press(modifiers),
            MouseEvent::ButtonPressed {
                button: MouseButton::Right,
                ..
            } => self.context_menu(),
            MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                ..
            } => self.released(),
            MouseEvent::WheelScrolled { delta, modifiers } => self.scrolled(delta, modifiers),
            MouseEvent::CursorLeft => {
                self.playing.drag(None);
                self.playing.hover = None;
                self.tempo_hover = None;
                if self.drag.is_none() {
                    self.hover = None;
                }
            }
            _ => return false,
        }
        true
    }
}

/// The window's frame: the panel's renderer's (the whole drawing), the strip's parts over its
/// strip (`strip`, from the panel's foot down), the arpeggiator's drawer over that from its top
/// (drawing units, its plate alone, cut off at the rail's foot) with its shadow laid over what is
/// under its foot, the presets' drawer over everything from `drawer`'s top
/// (drawing units, cut off at the rail's foot), and over everything what floats (MIDI Learn's
/// ring, a tip, a note, a menu); in the places `only` alone (pixels: left, top, right,
/// bottom) when given. (The places, not the rows they span: QUALITY's opening at the panel's
/// top and the strip's drops at its foot put together the whole frame between them.)
#[allow(clippy::too_many_arguments)]
fn compose(
    composed: &mut Option<ca72_panel::Pixmap>,
    renderer: &Renderer,
    strip: &ca72_panel::Pixmap,
    lower: Option<LowerFrame<'_>>,
    drawer: Option<(&ca72_panel::Pixmap, f64)>,
    (page, macros, mods): (
        Option<&ca72_panel::Pixmap>,
        Option<Placed<'_>>,
        &ModRenderer,
    ),
    only: Option<&[[i32; 4]]>,
    expanded: bool,
    (width, height): (u32, u32),
) {
    let frame = renderer.frame();
    let all = [0, 0, width as i32, height as i32];
    let kept = composed
        .as_ref()
        .is_some_and(|c| (c.width(), c.height()) == (width, height));
    let places = match only {
        Some(places) if kept => places,
        _ => std::slice::from_ref(&all),
    };
    let c = match composed {
        Some(c) if kept => c,
        _ => composed.insert(ca72_panel::Pixmap::new(width, height).expect("a frame")),
    };
    let s = renderer.scale();
    // (The frame begins at the panel's top, the drawing's row `origin`.)
    let origin = renderer.top_px();
    let top = renderer.strip_row();
    let rail = (DRAWER_TOP * s).round() as i32 - origin;
    let (left, right) = (
        (lower_ui::LEFT * s).round() as i32,
        (lower_ui::RIGHT * s).round() as i32,
    );
    for &[x0, y0, x1, y1] in places {
        paste(c, frame, 0, [x0, y0, x1, y1]);
        paste(c, strip, top, [x0, y0, x1, y1]);
        // The macros in the left area, over the wheels (decisions.md R-MACROS).
        if let Some((m, r)) = macros {
            paste_at(
                c,
                m,
                (r[0], r[1]),
                [x0.max(r[0]), y0.max(r[1]), x1.min(r[2]), y1.min(r[3])],
            );
        }
        // The LFO page in the lower page's place (decisions.md R-LFO), and the knobs' rings.
        if let Some(p) = page {
            let at = lfo_ui::top_px(s) - origin;
            paste(c, p, at, [x0.max(left), y0.max(rail), x1.min(right), y1]);
        }
        mods.lay_rings_where(c, [x0, y0, x1, y1], |i| fx_pins::target(i).is_none());
        if let Some(l) = &lower {
            // (Its frame's rows the drawing's from `top_px` down, open; moved up as it slides.)
            let at = lower_ui::top_px(s) - ((lower_ui::TOP - l.top) * s).round() as i32 - origin;
            let place = [x0.max(left), y0.max(rail), x1.min(right), y1];
            paste(
                c,
                l.frame,
                at,
                [
                    place[0],
                    place[1],
                    place[2],
                    place[3].min(at + (l.depth * s).round() as i32),
                ],
            );
            if lower.as_ref().is_some_and(|l| l.depth == fx_ui::PIN_DEPTH) {
                mods.lay_rings_where(c, [x0, y0, x1, y1], |i| fx_pins::target(i).is_some());
            }
            for &(p, px, py) in &l.under {
                shade(c, p, (px, at + py), place);
            }
        }
        if let Some((d, at)) = drawer {
            paste(
                c,
                d,
                (at * s).round() as i32 - origin,
                [x0, y0.max(rail), x1, y1],
            );
        }
        // A dragged assignment's cord and the hover pop-up over everything but what floats.
        mods.lay_over(c, [x0, y0, x1, y1]);
    }
    // Logical height is rounded before the host applies DPI scaling. If that adds a row
    // beyond the artwork, extend its last row rather than leave an unpainted viewport edge.
    if expanded && height > frame.height() {
        let row = frame.height().saturating_sub(1) as usize;
        let stride = width as usize * 4;
        let last = c.data()[row * stride..(row + 1) * stride].to_vec();
        for y in frame.height() as usize..height as usize {
            c.data_mut()[y * stride..(y + 1) * stride].copy_from_slice(&last);
        }
    }
    if !expanded {
        let side = (COMPACT_GRIP * s).ceil() as i32;
        let grip = [all[2] - side, all[3] - side, all[2], all[3]];
        paste(c, frame, 0, grip);
        paste(c, strip, top, grip);
        ca72_panel::render::compact_grip(c, s);
    }
    if places == [all] && renderer.floating() {
        renderer.draw_floating(c);
    }
}

/// A frame laid at a place of the window's (its top left corner, its right and bottom, in the
/// window's pixels): the macros' area (decisions.md R-MACROS).
type Placed<'a> = (&'a ca72_panel::Pixmap, [i32; 4]);

/// A drawer dropped from the rail as [`compose`] lays it: its frame (its plate alone shown,
/// `lower::LEFT` to `RIGHT`), how deep it is, its top now (drawing units), and what hangs under
/// it, each premultiplied with its top left corner in the frame's pixels (its shadow; the effects'
/// list).
struct LowerFrame<'a> {
    frame: &'a ca72_panel::Pixmap,
    depth: f64,
    top: f64,
    under: Vec<(&'a ca72_panel::Pixmap, i32, i32)>,
}

/// The drawer `which` as [`compose`] lays it, its top at `top` (drawing units).
fn lower_frame<'a>(
    which: Lower,
    top: f64,
    (arp, mpe): (&'a ArpRenderer, &'a MpeRenderer),
    fx: Option<&'a FxRenderer>,
    scale: f64,
) -> LowerFrame<'a> {
    match (which, fx) {
        (Lower::Mpe, _) => LowerFrame {
            frame: mpe.frame(),
            depth: mpe_ui::DEPTH,
            top,
            under: mpe
                .shadow()
                .map(|p| (p, 0, (mpe_ui::DEPTH * scale).round() as i32))
                .into_iter()
                .collect(),
        },
        (Lower::Fx, Some(fx)) => LowerFrame {
            frame: fx.frame(),
            depth: fx.depth(),
            top,
            under: fx.under(),
        },
        _ => LowerFrame {
            frame: arp.frame(),
            depth: arp_ui::DEPTH,
            top,
            under: arp
                .shadow()
                .map(|p| (p, 0, (arp_ui::DEPTH * scale).round() as i32))
                .into_iter()
                .collect(),
        },
    }
}

/// `p` (premultiplied, partly clear) laid over the frame `c` with its top left corner at
/// (`left`, `top`), inside `place` (left, top, right, bottom) only: what hangs under a drawer
/// dropped from the rail.
fn shade(
    c: &mut ca72_panel::Pixmap,
    p: &ca72_panel::Pixmap,
    (left, top): (i32, i32),
    place: [i32; 4],
) {
    let (w, h) = (c.width() as i32, c.height() as i32);
    let (pw, ph) = (p.width() as i32, p.height() as i32);
    let (x0, x1) = (place[0].max(0).max(left), place[2].min(w).min(left + pw));
    let (y0, y1) = (place[1].max(top).max(0), place[3].min(top + ph).min(h));
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    for y in y0..y1 {
        let src = (((y - top) * pw + x0 - left) * 4) as usize;
        let dst = ((y * w + x0) * 4) as usize;
        let n = (x1 - x0) as usize * 4;
        let (s, d) = (&p.data()[src..src + n], &mut c.data_mut()[dst..dst + n]);
        for (d, s) in d.chunks_exact_mut(4).zip(s.chunks_exact(4)) {
            let keep = 255 - u16::from(s[3]);
            for j in 0..4 {
                d[j] = (u16::from(s[j]) + (u16::from(d[j]) * keep + 127) / 255).min(255) as u8;
            }
        }
    }
}

/// `p` put in the frame `c` with its top left corner at (`left`, `top`), inside `place` (left,
/// top, right, bottom) only.
fn paste_at(
    c: &mut ca72_panel::Pixmap,
    p: &ca72_panel::Pixmap,
    (left, top): (i32, i32),
    place: [i32; 4],
) {
    let (w, h) = (c.width() as i32, c.height() as i32);
    let (pw, ph) = (p.width() as i32, p.height() as i32);
    let (x0, x1) = (place[0].max(left).max(0), place[2].min(left + pw).min(w));
    let (y0, y1) = (place[1].max(top).max(0), place[3].min(top + ph).min(h));
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let n = (x1 - x0) as usize * 4;
    for y in y0..y1 {
        let src = (((y - top) * pw + x0 - left) * 4) as usize;
        let dst = ((y * w + x0) * 4) as usize;
        c.data_mut()[dst..dst + n].copy_from_slice(&p.data()[src..src + n]);
    }
}

/// `p` put in the frame `c` with its top at row `top`, inside `place` (left, top, right,
/// bottom) only.
fn paste(c: &mut ca72_panel::Pixmap, p: &ca72_panel::Pixmap, top: i32, place: [i32; 4]) {
    let (w, h) = (c.width() as i32, c.height() as i32);
    let (pw, ph) = (p.width() as i32, p.height() as i32);
    let (x0, x1) = (place[0].max(0), place[2].min(w).min(pw));
    let (y0, y1) = (place[1].max(top).max(0), place[3].min(top + ph).min(h));
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let n = (x1 - x0) as usize * 4;
    for y in y0..y1 {
        let src = (((y - top) * pw + x0) * 4) as usize;
        let dst = ((y * w + x0) * 4) as usize;
        c.data_mut()[dst..dst + n].copy_from_slice(&p.data()[src..src + n]);
    }
}

/// What the arpeggiator's drawer shows (decisions.md R-ARP), from the parameters and the
/// controls' values as the panel shows them (a knob dragged where the drag has it), and the scene
/// before (its hover is set elsewhere).
fn arp_scene(p: &Ca72Params, values: &[f64; CONTROLS.len()], was: ArpScene) -> ArpScene {
    let knob = |id: &str| ca72_panel::controls::index(id).map_or(0.0, |i| values[i]);
    let rate = crate::arp::RATES[p.arp_rate.value().to_index()].0;
    ArpScene {
        on: p.arp_on.value(),
        latch: p.arp_latch.value(),
        direction: p.arp_direction.value().to_index(),
        readouts: [
            (rate.to_owned(), true),
            (p.arp_octaves.value().to_string(), true),
            (format!("{:.0}", p.arp_gate.value()), true),
            (format!("{:.0}", p.arp_swing.value()), true),
        ],
        knobs: arp_ui::KNOB_PARAMS.map(knob),
        hover: was.hover,
    }
}

/// Whether an amount of each note's own expression is not 0 (decisions.md R-MPE): the rail's MPE
/// key lit, as ARP's while the arpeggiator runs.
fn reaching(p: &Ca72Params) -> bool {
    [
        &p.pressure_mod,
        &p.pressure_filter,
        &p.timbre_mod,
        &p.timbre_filter,
    ]
    .iter()
    .any(|q| q.value() != 0.0)
}

/// What the expression drawer shows (decisions.md R-MPE): the readouts in the knobs' units
/// (MODULATION's whole percent, CUTOFF's octaves to a tenth), the knobs as the panel shows them
/// (a knob dragged where the drag has it), and what INPUT heard.
/// Its second row the velocity's (decisions.md R-VELOCITY): the latest note's (`velocity`, 0..1)
/// on LAST NOTE and the RESPONSE screen, what it moves on MODULATES, CURVE's and FLOOR's percent.
fn mpe_scene(
    p: &Ca72Params,
    values: &[f64; CONTROLS.len()],
    input: mpe_ui::Input,
    velocity: Option<f64>,
    standalone: bool,
) -> MpeScene {
    let knob = |id: &str| ca72_panel::controls::index(id).map_or(0.5, |i| values[i]);
    // (No "-0".)
    let shown = |v: f32, places: usize| {
        let t = format!("{v:.places$}");
        if t.trim_start_matches('-')
            .chars()
            .all(|c| c == '0' || c == '.')
        {
            t.trim_start_matches('-').to_owned()
        } else {
            t
        }
    };
    MpeScene {
        input,
        readouts: [
            (shown(p.pressure_mod.value(), 0), true),
            (shown(p.pressure_filter.value(), 1), true),
            (shown(p.timbre_mod.value(), 0), true),
            (shown(p.timbre_filter.value(), 1), true),
            (shown(p.velocity_curve.value(), 0), true),
            (shown(p.velocity_floor.value(), 0), true),
        ],
        knobs: mpe_ui::KNOB_PARAMS.map(knob),
        last: velocity.map_or(String::new(), |v| {
            format!("{}", (v.clamp(0.0, 1.0) * 127.0).round())
        }),
        modulates: lfo_page::source_screen_text(
            &p.lfo_patch,
            crate::lfo::Source::Velocity,
            standalone,
        ),
        response: {
            let r = crate::lfo::response(p);
            mpe_ui::Response {
                curve: f64::from(r.curve),
                floor: f64::from(r.floor),
                point: velocity,
            }
        },
    }
}

/// Whether oscillator `n`'s (1, 2) TIMED key does nothing: its RANGE away from LO (decisions.md
/// R-OSC-TIMED). It is off the panel then, whatever it holds ([`Editing::off_panel`]), and does
/// not answer the pointer.
fn timed_inert(p: &Ca72Params, n: usize) -> bool {
    p.timed_params(n)
        .is_some_and(|(_, _, range)| range.value() != crate::params::Footage::Lo)
}

/// Oscillators 2's and 3's timed keys as the panel shows them: each one's division (its sign) and
/// whether it is inert (off the panel).
fn timed_scene(p: &Ca72Params) -> [ca72_panel::render::Timed; 2] {
    std::array::from_fn(|k| {
        let n = k + 1;
        ca72_panel::render::Timed {
            division: p
                .timed_params(n)
                .map_or(crate::timed::DEFAULT_DIVISION, |(_, d, _)| {
                    d.value().to_index()
                }),
            inert: timed_inert(p, n),
        }
    })
}

/// A tip's name for a control in timed mode (R-OSC-TIMED): a FREQUENCY knob turning its
/// DIVISION.
fn timed_label(p: &Ca72Params, param: &str) -> Option<&'static str> {
    Some(match param {
        "osc2_frequency" if p.timed_now(1) => "OSCILLATOR-2 DIVISION",
        "osc3_frequency" if p.timed_now(2) => "OSCILLATOR-3 DIVISION",
        _ => return None,
    })
}

/// A tip's name for a MOD key (R-CONTINUOUS-WAVE): its way with it.
fn mod_label(p: &Ca72Params, param: &str) -> Option<&'static str> {
    use crate::params::ShapeModDirection as D;
    let n = ca72_panel::render::MOD_KEYS
        .iter()
        .position(|k| *k == param)?;
    let names = [
        [
            "OSCILLATOR-1 MOD, CLOCKWISE",
            "OSCILLATOR-1 MOD, ANTICLOCKWISE",
            "OSCILLATOR-1 MOD, BOTH WAYS",
        ],
        [
            "OSCILLATOR-2 MOD, CLOCKWISE",
            "OSCILLATOR-2 MOD, ANTICLOCKWISE",
            "OSCILLATOR-2 MOD, BOTH WAYS",
        ],
        [
            "OSCILLATOR-3 MOD, CLOCKWISE",
            "OSCILLATOR-3 MOD, ANTICLOCKWISE",
            "OSCILLATOR-3 MOD, BOTH WAYS",
        ],
    ];
    Some(
        names[n][match p.shape_params(n).3.value() {
            D::Clockwise => 0,
            D::Anticlockwise => 1,
            D::Both => 2,
        }],
    )
}

/// Whether OSC. 3 CONTROL does nothing: oscillator 3 timed (its key on, its RANGE at LO;
/// decisions.md R-OSC-TIMED), its cycle kept to the clock whatever the keys play. Then it is
/// dimmed and does not answer the pointer, as VOICES where it does nothing, and keeps its value,
/// never switched off (the owner, 2026-10-20: "Okay, that's fine. Do it that way."); the host's
/// automation and an agent still set it, to count when oscillator 3 follows the keys again.
fn osc3_control_inert(p: &Ca72Params) -> bool {
    p.timed_now(2)
}

/// Whether VOICES does nothing (the owner, 2026-10-11, of a MONO preset's VOICES 4: "I don't
/// think those 4 voices do anything"; decisions.md R64): in MONO (neither POLY nor UNISON)
/// with DOUBLE, whose one pair has no place among others, or scattering with WIDTH at 0, where
/// every one of VOICES's places is the centre (R62). Then its knob is dimmed and not operable,
/// and its readout not lit nor typed into, as DETUNE's in SCATTER; its value stays for when it
/// counts again.
fn voices_inert(p: &Ca72Params) -> bool {
    let c = p.controls();
    !c.poly && !c.unison && (p.doubled.value() || c.spread <= 0.0)
}

/// How often the screens of modulated controls are read again: at most 30 times a second, as
/// the LFO page's orbs (decisions.md R-LFO).
const LIVE_FRAME: Duration = Duration::from_millis(33);

/// The strip's readout showing target `t`, if one does.
fn readout_of(t: crate::lfo::Target) -> Option<Readout> {
    use crate::lfo::Target as T;
    Some(match t {
        T::Entropy => Readout::Entropy,
        T::Spread => Readout::Width,
        T::Double => Readout::Detune,
        T::Drive => Readout::Drive,
        T::Level => Readout::Level,
        T::Inner => Readout::Inner,
        _ => return None,
    })
}

/// What the strip shows, from the parameters, the voices sounding (a bit a voice), their levels
/// (0 to 1, by voice) and the output's, and the scene before (its rail, hover and MIDI Learn's
/// ring are set elsewhere).
fn strip_scene(
    p: &Ca72Params,
    sounding: u32,
    (levels, output): (&[f64], f64),
    live: &[Option<f64>; LIVE.len()],
    was: StripScene,
) -> StripScene {
    let c = p.controls();
    // (The screens of the controls a source moves: where the engine has them, modulated, in
    // their own units; lit as those values are. The rest of the strip as set: R-LFO.)
    let mut r = c;
    for (t, v) in LIVE.iter().zip(live) {
        if let Some(v) = *v {
            match t {
                crate::lfo::Target::Entropy => r.entropy = v,
                crate::lfo::Target::Spread => r.spread = v,
                crate::lfo::Target::Double => r.double = v,
                crate::lfo::Target::Drive => r.drive = v,
                crate::lfo::Target::Level => r.level = v,
                _ => r.inner = v,
            }
        }
    }
    let unison = c.unison;
    let poly = c.poly && !unison;
    let mono = !poly && !unison;
    let doubled = p.doubled.value();
    // (MONO with DOUBLE: the one pair, all of WIDTH's way out, whatever VOICES and the
    // placement; without, it scatters over VOICES's places: decisions.md R62.)
    let alone = mono && doubled;
    let voices = c.voices.clamp(2, 10);
    let placement = c.placement;
    let mode = Some(if unison { 2 } else { usize::from(poly) });
    // Three places, the readouts' (the mock-up's): a decimal under ten, a sign taking a place.
    let num3 = |v: f64| {
        if v.abs() < 0.05 {
            "0.0".to_owned()
        } else if v.abs() < 9.95 {
            format!("{v:.1}")
        } else {
            format!("{}", v.round())
        }
    };
    let percent = |v: f64| {
        if v > 0.0 {
            format!("{}", (v * 100.0).round())
        } else {
            "OFF".to_owned()
        }
    };
    // (AUTO GAIN's at DRIVE as it plays: the engine reads its curve there.)
    let auto = if c.auto_gain && r.drive > 0.0 {
        let curve = crate::drive::Curve(p.drive_curve.saved().db);
        (num3(curve.at(r.drive, crate::engine::DRIVE_TOP)), true)
    } else {
        (String::new(), false)
    };
    let readouts = [
        (format!("{voices}"), !voices_inert(p)),
        (percent(r.entropy), r.entropy > 0.0),
        (percent(r.spread), r.spread > 0.0),
        (
            if doubled {
                crate::params::cents(r.double * crate::engine::DOUBLE_CENTS)
            } else {
                "OFF".to_owned()
            },
            doubled,
        ),
        auto,
        (num3(r.drive), r.drive > 0.0),
        (num3(r.level), true),
        (percent(r.inner), r.inner > 0.0 && r.spread > 0.0),
    ];
    let on = |k: usize| sounding & (1 << k) != 0;
    // Where they sound: in MONO with DOUBLE the one pair, all of WIDTH's way out; else each of
    // VOICES's places by the placement (in MONO the one voice at the place of its last attack,
    // R62), its pair as far out with DOUBLE (decisions.md R45), each in its side's band
    // by INNER: where the engine's gains put it, the kit's (R49).
    let (width, inner) = (c.spread, c.inner);
    let field = if alone {
        Field::Double(vec![(place::pair_at(width, inner, place::ALONE), on(0))])
    } else if doubled {
        Field::Double(
            (0..voices)
                .map(|k| {
                    (
                        place::pair_at(width, inner, placement.pair(k, voices)),
                        on(k),
                    )
                })
                .collect(),
        )
    } else {
        Field::Scatter(
            (0..voices)
                .map(|k| (place::voice_at(width, inner, placement, k, voices), on(k)))
                .collect(),
        )
    };
    // The placement's tab by the plug-in's choice (its tabs are its choices, EVEN and CENTER:
    // not the kit's index, which counts its EDGES); none where it moves nothing.
    let placement_lit = !alone;
    StripScene {
        mode,
        stereo: Some(usize::from(doubled)),
        placement: placement_lit.then_some(p.placement.value().to_index()),
        auto: c.auto_gain,
        readouts,
        field,
        // Each voice's level by its place in the field (the one instrument's, the output's, at
        // whichever place it sounds), in steps of a fortieth (a drop does not move for less).
        levels: if mono {
            vec![output; voices]
        } else {
            levels.iter().take(voices).copied().collect()
        }
        .into_iter()
        .map(|v| (v * 40.0).round() / 40.0)
        .collect(),
        detune: if doubled {
            c.double * crate::engine::DOUBLE_CENTS
        } else {
            0.0
        },
        ..was
    }
}

/// Modified arrows remain host shortcuts. TYPE uses letters and Z/X, so leaves these free.
fn preset_arrow(k: &KeyboardEvent) -> Option<i32> {
    if k.modifiers
        .intersects(Modifiers::SHIFT | Modifiers::CONTROL | Modifiers::ALT | Modifiers::META)
        || k.is_composing
    {
        return None;
    }
    match k.key {
        // Left and right only, as the rail's ‹ › keys (the owner, 2026-10-19); up and down
        // stay free.
        Key::ArrowLeft => Some(-1),
        Key::ArrowRight => Some(1),
        _ => None,
    }
}

impl Editing {
    /// A key: whether the drawer took it (else it is the host's).
    fn keyed(&mut self, k: &KeyboardEvent) -> bool {
        // The system's shortcut for quitting the standalone (Ctrl+Q on Linux; decisions.md
        // R-QUIT) before anything else here has it: TYPING, a value or a name being typed, MIDI
        // Learn, the notes. The app's window closes, as its close button closes it.
        if self.quits(k) {
            #[cfg(feature = "standalone-production")]
            nih_plug::wrapper::production::app_window::request_quit();
            return true;
        }
        self.reconcile_resize();
        if let Some(notes) = &mut self.whats_new {
            if k.state == KeyState::Up {
                self.playing.key(k, false);
            }
            if k.state == KeyState::Down {
                match k.key {
                    Key::Escape | Key::Enter => self.whats_new = None,
                    Key::ArrowDown => notes.scroll(1),
                    Key::ArrowUp => notes.scroll(-1),
                    Key::PageDown => notes.page(1),
                    Key::PageUp => notes.page(-1),
                    Key::Home => notes.home(),
                    Key::End => notes.end(),
                    _ => {}
                }
                self.composed = None;
            }
            return true;
        }
        let allowed = self.note_typing_allowed();
        let typing_root = self.playing.typing_root;
        if self.playing.key(k, allowed) {
            if self.playing.typing_root != typing_root {
                self.keep_settings(Change::TypingRoot(self.playing.typing_root));
                self.composed = None;
            }
            return true;
        }
        #[cfg(feature = "standalone-production")]
        if let Some(dialog) = &mut self.standalone_dialog {
            if dialog.key(k) {
                self.standalone_dialog = None;
            }
            self.composed = None;
            return true;
        }
        // MIDI Learn first (decisions.md R34): Escape closes a control's menu, the drawer's MIDI
        // list takes the keys it shows, and Escape cancels learning.
        if self.macro_view.typing.is_some() {
            return self.rename_key(k);
        }
        if self.typing.is_some() {
            return self.type_key(k);
        }
        if self.knob_entry.is_some() {
            return self.knob_key(k);
        }
        let map = std::sync::Arc::clone(&self.params.midi_map);
        let down = k.state == KeyState::Down && !modifier(&k.key);
        if down && k.key == Key::Escape && self.learning.menu_open() {
            self.learning.close_menu();
            return true;
        }
        if self.fx_view.key(k) || self.fx_view.list_key(k) {
            return true;
        }
        if self.browser.open && self.learning.list {
            let mut close = false;
            let taken = self.learning.list_key(k, &map, &mut close);
            if close {
                self.browser.show(false, &self.params);
                self.follow_drawer();
            }
            return taken;
        }
        if self.map_key(k) {
            return true;
        }
        if down && k.key == Key::Escape && map.armed().is_some() {
            self.learning.cancel(&map);
            return true;
        }
        if self.learning.holds_keys(&map) {
            return false;
        }
        if self.preset_keys_allowed()
            && let Some(by) = preset_arrow(k)
        {
            if down {
                self.end_gestures();
                let setter = ParamSetter::new(self.context.as_ref());
                self.browser.step(by, &self.params, &setter);
            }
            return true;
        }
        // A key the drawer acts on may set the parameters (the arrows choose presets): no
        // gesture is left open under it. Shift and the other modifiers alone leave a drag.
        if self.browser.open && down {
            self.end_gestures();
        }
        let setter = ParamSetter::new(self.context.as_ref());
        let taken = self.browser.key(k, &self.params, &setter);
        self.follow_drawer();
        taken
    }
}

impl Drop for Editing {
    // The editor closed mid-drag, or within a wheel's rest: no parameter is left touched. And
    // MIDI Learn ends: a controller the audio thread has already caught for it is assigned,
    // else nothing is (decisions.md R34). It no longer watches the levels (the audio thread
    // stops measuring them when no editor does).
    fn drop(&mut self) {
        self.remember_width();
        if self
            .learning
            .close(&self.params.midi_map)
            .is_some_and(|a| a.param == crate::learn::MOD_WHEEL)
        {
            self.keep_settings(Change::ModWheelSource(self.params.midi_map.wheel_source()));
        }
        self.end_gestures();
        self.meters.watchers.fetch_sub(1, Ordering::Relaxed);
    }
}

/// How long a frame shown on an X11 window is trusted to stay shown. X11 drops what is drawn
/// to a window not yet on screen (a host maps the editor's window after its first frame:
/// Bitwig on Linux showed it black until the pointer changed something) and, without a
/// compositor, what a covering window hid; baseview reports neither. So on Linux the last
/// frame is shown again at this pace, unchanged (a copy, no rendering; decisions.md R19), and
/// at every frame for a while after a size is first shown ([`SETTLE`]). macOS keeps a window's
/// pixels; Windows asks for them again (`WindowEvent::Damaged`, R28), as X11 does as a window is
/// mapped or resized (baseview's change 12).
const REPAINT: Option<Duration> = if cfg!(target_os = "linux") {
    Some(Duration::from_millis(250))
} else {
    None
};

/// How often the frame is drawn while the window cannot be seen (covered by other windows,
/// minimised, on another desktop, the host hidden): the scene kept up (the drawer, a resize, MIDI
/// Learn) without drawing the shutter and the levels thirty times a second for nobody. Nothing is
/// shown meanwhile; the window seen again is shown whole at once.
const HIDDEN_DRAW: Duration = Duration::from_millis(250);

/// The tests' counts of the frames drawn and shown by the editors' windows (R-EDITOR-CPU).
#[cfg(test)]
static DRAWN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
#[cfg(test)]
static SHOWN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How long after a frame is first shown at a size (the window opened, PANEL, SIZE, the grip)
/// it is shown again at every frame, on X11: what is shown before the host's window has taken
/// that size, or before the host has put it on screen, is lost. Bitwig 5.2.7 on Linux grows its
/// window after the editor's and puts it on screen late: the editor's first frame after it
/// reopened was lost, and the window stayed black until the next showing (decisions.md
/// R-LINUX-RESIZE). A copy a frame, no drawing; 250 ms is the [`REPAINT`] it would wait for.
const SETTLE: Option<Duration> = if cfg!(target_os = "linux") {
    Some(Duration::from_millis(250))
} else {
    None
};

/// Whether the frame must be shown again though nothing in it changed: not yet shown since
/// the window opened or resized, (Linux) settling until `settle`, or (Linux) shown longer ago
/// than [`REPAINT`].
fn repaint_due(shown: Option<Instant>, settle: Option<Instant>, now: Instant) -> bool {
    if settle.is_some_and(|until| now < until) {
        return true;
    }
    match (shown, REPAINT) {
        (None, _) => true,
        (Some(at), Some(every)) => now.saturating_duration_since(at) >= every,
        (Some(_), None) => false,
    }
}

impl WindowHandler for PanelWindow {
    fn on_frame(&mut self, window: &mut Window<'_>) {
        // The keys the host gave the view since the last frame (Cubase), as the window's own,
        // but those the window had too (decisions.md R53).
        if self.host_keys.overflow.swap(false, Ordering::AcqRel) {
            self.editing.playing.release();
            self.host_keys.take();
        }
        for k in self.host_keys.take() {
            if self.twins.host_new(&k, Instant::now()) {
                self.key(window, &k);
            }
        }
        // A drag whose window lost the pointer's capture (Windows: Alt-Tab mid-drag) never
        // sees its release: it ends here (decisions.md R18).
        if self.editing.dragging() && window::lost_capture(window) {
            self.editing.end_gestures();
            self.editing.playing.release_pointer();
        }
        // What agents ask, carried out as the player's edits are, before the frame is drawn.
        if self.editing.serve_agents() {
            self.follow(window);
            self.keyboard(window);
        }
        // The grip's width, asked of the host once a frame.
        self.editing.grip();
        self.follow(window);
        let now = Instant::now();
        // Unseen, drawn only now and then and not shown; seen again, shown whole.
        // A remote AU view may be visible through its host's proxy while its native backing
        // window is occluded. Keep presenting when that window cannot describe visibility.
        let seen = (cfg!(target_os = "macos")
            && !self.editing.context.native_window_occlusion_reliable())
            || window::seen(window);
        if !seen {
            self.shown = None;
            if self
                .hidden_drawn
                .is_some_and(|at| now.saturating_duration_since(at) < HIDDEN_DRAW)
            {
                self.keyboard(window);
                self.editing.measure_when_done();
                return;
            }
            self.hidden_drawn = Some(now);
        }
        if !self.editing.whats_new_checked {
            self.editing
                .release_notes_when_visible(window::release_notes_visible(
                    window,
                    self.editing.context.native_window_occlusion_reliable(),
                ));
        }
        #[cfg(test)]
        DRAWN.fetch_add(1, Ordering::Relaxed);
        let changed = self.editing.draw();
        // A drawer that has just slid shut has asked the host for the shorter window: the
        // editor's view follows in the same frame, before anything is shown. A frame between
        // them showed, on macOS, the tall view in the short window, its top cut off (the
        // owner, 2026-10-03: a flicker as the drawer finished shutting; decisions.md R19).
        self.follow(window);
        // (MIDI Learn done meanwhile, by a controller rather than here: the keyboard back to the
        // host now, not at the editor's next event; decisions.md R34.)
        self.keyboard(window);
        // (Shown again whole when due: X11 may have dropped any of it.)
        let due = repaint_due(self.shown, self.settle, now);
        if seen && (changed || due) {
            #[cfg(test)]
            SHOWN.fetch_add(1, Ordering::Relaxed);
            if self.shown.is_none() {
                self.settle = SETTLE.map(|for_| now + for_);
            }
            self.present(changed && !due);
            self.shown = Some(now);
        }
        self.editing.measure_when_done();
    }

    fn on_event(&mut self, window: &mut Window<'_>, event: Event) -> EventStatus {
        match event {
            Event::Window(WindowEvent::Resized(info)) => {
                self.resized(&info);
                EventStatus::Captured
            }
            // Windows repainting the window (a host's window painted its background over it),
            // or the X server asking for it (the window mapped or at a new size: baseview's
            // change 12, decisions.md R-LINUX-RESIZE): the last frame shown, again at once,
            // unless none has been since it opened or resized (the next frame shows one). Not
            // drawn afresh (decisions.md R28).
            Event::Window(WindowEvent::Damaged) => {
                if self.shown.is_some() {
                    self.present(false);
                }
                EventStatus::Captured
            }
            // No gesture is left open (decisions.md R18), nor a control's menu (R34; learning
            // goes on: the controller to be learned is elsewhere).
            Event::Window(WindowEvent::Focused) => {
                self.editing.playing.focused = true;
                self.keyboard(window);
                EventStatus::Ignored
            }
            Event::Window(WindowEvent::Unfocused | WindowEvent::WillClose) => {
                self.editing.playing.focused = false;
                self.editing.playing.release();
                self.editing.end_gestures();
                self.editing.learning.close_menu();
                self.keyboard(window);
                EventStatus::Ignored
            }
            Event::Mouse(e) => {
                let released = matches!(e, MouseEvent::ButtonReleased { .. });
                let status = if self.editing.mouse(e) {
                    EventStatus::Captured
                } else {
                    EventStatus::Ignored
                };
                if released {
                    window::release_capture(window);
                }
                // The drawer opening or shutting below the bar: the window follows (the
                // owner, 2026-10-02: on macOS it had stayed drawn at the old size until
                // reopened).
                let height_changed = self.follow(window);
                self.keyboard(window);
                self.cursor(window);
                self.repaint_height_change(window, height_changed);
                status
            }
            // The drawer, while open, takes the keys (the host keeps its shortcuts).
            Event::Keyboard(k) => {
                // (One the host gave the view just now too: taken once.)
                if !self.twins.own_new(&k, Instant::now()) {
                    return EventStatus::Captured;
                }
                let taken = self.key(window, &k);
                if taken {
                    EventStatus::Captured
                } else {
                    EventStatus::Ignored
                }
            }
        }
    }
}

/// A modifier key alone.
fn modifier(key: &Key) -> bool {
    matches!(
        key,
        Key::Shift
            | Key::Control
            | Key::Alt
            | Key::AltGraph
            | Key::Meta
            | Key::Super
            | Key::CapsLock
            | Key::Fn
            | Key::FnLock
            | Key::Hyper
            | Key::NumLock
            | Key::ScrollLock
            | Key::Symbol
            | Key::SymbolLock
    )
}

/// The usable part of the screen (less the menu bar, dock or taskbar), in logical pixels
/// (`scale` the window's, where the platform counts physical ones): macOS's main screen,
/// Windows' primary monitor, on X11 the monitor the pointer is on (else the primary).
#[allow(unsafe_code)]
mod screen {
    #[cfg(target_os = "macos")]
    pub fn usable(_scale: Option<f64>) -> Option<(f64, f64)> {
        use cocoa::appkit::NSScreen;
        use cocoa::base::nil;
        // SAFETY: a class method of AppKit's, nil where there is no screen.
        let screen = unsafe { NSScreen::mainScreen(nil) };
        if screen == nil {
            return None;
        }
        // SAFETY: `screen` is an NSScreen.
        let frame = unsafe { NSScreen::visibleFrame(screen) };
        Some((frame.size.width, frame.size.height))
    }

    #[cfg(target_os = "windows")]
    pub fn usable(scale: Option<f64>) -> Option<(f64, f64)> {
        use winapi::shared::windef::RECT;
        use winapi::um::winuser::{SPI_GETWORKAREA, SystemParametersInfoW};
        let mut r = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        // SAFETY: SPI_GETWORKAREA writes one RECT through the pointer.
        let ok =
            unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, (&mut r as *mut RECT).cast(), 0) };
        if ok == 0 {
            return None;
        }
        // The work area is in the pixels of the host's thread, which the window counts in
        // unless the host gave a scale (decisions.md R18).
        let k = scale.unwrap_or(1.0);
        Some((
            f64::from(r.right - r.left) / k,
            f64::from(r.bottom - r.top) / k,
        ))
    }

    /// The X11 screen spans every monitor: two side by side made the editor open wider
    /// than the one it is on, its grip off it (decisions.md R18). RandR's monitors (1.5),
    /// the window manager's work area (`_NET_WORKAREA`), through a connection of our own.
    #[cfg(target_os = "linux")]
    pub fn usable(scale: Option<f64>) -> Option<(f64, f64)> {
        use x11rb::connection::Connection;
        use x11rb::protocol::randr::ConnectionExt as _;
        use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _};
        let (conn, n) = x11rb::connect(None).ok()?;
        let screen = conn.setup().roots.get(n)?;
        let root = screen.root;
        let whole = Rect {
            x: 0,
            y: 0,
            w: i32::from(screen.width_in_pixels),
            h: i32::from(screen.height_in_pixels),
        };
        let monitors: Vec<(Rect, bool)> = conn
            .randr_query_version(1, 5)
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|_| conn.randr_get_monitors(root, true).ok()?.reply().ok())
            .map(|r| {
                r.monitors
                    .iter()
                    .map(|m| {
                        let r = Rect {
                            x: i32::from(m.x),
                            y: i32::from(m.y),
                            w: i32::from(m.width),
                            h: i32::from(m.height),
                        };
                        (r, m.primary)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let pointer = conn
            .query_pointer(root)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|p| (i32::from(p.root_x), i32::from(p.root_y)));
        let work = conn
            .intern_atom(true, b"_NET_WORKAREA")
            .ok()
            .and_then(|c| c.reply().ok())
            .filter(|a| a.atom != 0)
            .and_then(|a| {
                let p = conn
                    .get_property(false, root, a.atom, AtomEnum::CARDINAL, 0, 4)
                    .ok()?
                    .reply()
                    .ok()?;
                let v: Vec<u32> = p.value32()?.collect();
                let [x, y, w, h] = v[..] else { return None };
                Some(Rect {
                    x: i32::try_from(x).ok()?,
                    y: i32::try_from(y).ok()?,
                    w: i32::try_from(w).ok()?,
                    h: i32::try_from(h).ok()?,
                })
            });
        let area = monitor_area(whole, &monitors, pointer, work);
        let k = scale.unwrap_or(1.0);
        Some((f64::from(area.w) / k, f64::from(area.h) / k))
    }

    /// A rectangle of the X11 screen, in pixels.
    #[cfg(any(target_os = "linux", test))]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Rect {
        pub x: i32,
        pub y: i32,
        pub w: i32,
        pub h: i32,
    }

    #[cfg(any(target_os = "linux", test))]
    impl Rect {
        fn contains(&self, (x, y): (i32, i32)) -> bool {
            (self.x..self.x + self.w).contains(&x) && (self.y..self.y + self.h).contains(&y)
        }

        fn and(&self, o: &Rect) -> Option<Rect> {
            let (x, y) = (self.x.max(o.x), self.y.max(o.y));
            let (w, h) = (
                (self.x + self.w).min(o.x + o.w) - x,
                (self.y + self.h).min(o.y + o.h) - y,
            );
            (w > 0 && h > 0).then_some(Rect { x, y, w, h })
        }
    }

    /// Where on the X11 screen the editor opens: the monitor the pointer is on (where the
    /// host was just clicked), else the primary, else the first, else the whole screen; less
    /// the window manager's panels where its work area says (one rectangle for all the
    /// monitors, so only its part on this monitor counts).
    #[cfg(any(target_os = "linux", test))]
    pub fn monitor_area(
        whole: Rect,
        monitors: &[(Rect, bool)],
        pointer: Option<(i32, i32)>,
        work: Option<Rect>,
    ) -> Rect {
        let monitor = pointer
            .and_then(|p| monitors.iter().find(|(r, _)| r.contains(p)))
            .or_else(|| monitors.iter().find(|(_, primary)| *primary))
            .or(monitors.first())
            .map_or(whole, |(r, _)| *r);
        work.and_then(|w| monitor.and(&w)).unwrap_or(monitor)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    pub fn usable(_scale: Option<f64>) -> Option<(f64, f64)> {
        None
    }
}

/// The platform's windows: the host's parent window as baseview takes it, softbuffer's
/// surface on baseview's window (the two use different versions of `raw-window-handle`), and
/// what baseview leaves out: the keyboard given back, and the pointer's capture (Windows).
#[allow(unsafe_code)]
pub(crate) mod window {
    use std::num::{NonZeroIsize, NonZeroU32};
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::ptr::NonNull;

    use baseview::WindowHandle;
    use nih_plug::editor::ParentWindowHandle;
    use raw_window_handle::{
        HasRawDisplayHandle, HasRawWindowHandle, RawDisplayHandle, RawWindowHandle,
    };
    use raw_window_handle_06 as rwh6;

    /// The host's window the editor opens in.
    pub struct Parent(pub ParentWindowHandle);

    impl Parent {
        /// Whether a window can open in it: the host gave one, and on Linux there is an X11
        /// display (baseview panics in the host's thread without one).
        pub fn can_open(&self) -> bool {
            let given = match self.0 {
                ParentWindowHandle::X11Window(w) => w != 0,
                ParentWindowHandle::AppKitNsView(v) => !v.is_null(),
                ParentWindowHandle::Win32Hwnd(h) => !h.is_null(),
            };
            given && display()
        }
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn display() -> bool {
        use x11::xlib;
        // SAFETY: a connection of our own, closed at once.
        unsafe {
            let d = xlib::XOpenDisplay(std::ptr::null());
            if d.is_null() {
                return false;
            }
            xlib::XCloseDisplay(d);
        }
        true
    }

    #[cfg(not(target_os = "linux"))]
    pub(crate) fn display() -> bool {
        true
    }

    // SAFETY: the handle is the host's window, alive while the editor is open, and given as
    // the platform's own kind.
    unsafe impl HasRawWindowHandle for Parent {
        fn raw_window_handle(&self) -> RawWindowHandle {
            match self.0 {
                ParentWindowHandle::X11Window(window) => {
                    let mut h = raw_window_handle::XcbWindowHandle::empty();
                    h.window = window;
                    RawWindowHandle::Xcb(h)
                }
                ParentWindowHandle::AppKitNsView(ns_view) => {
                    let mut h = raw_window_handle::AppKitWindowHandle::empty();
                    h.ns_view = ns_view;
                    RawWindowHandle::AppKit(h)
                }
                ParentWindowHandle::Win32Hwnd(hwnd) => {
                    let mut h = raw_window_handle::Win32WindowHandle::empty();
                    h.hwnd = hwnd;
                    RawWindowHandle::Win32(h)
                }
            }
        }
    }

    /// The open window (if one could open), closed when dropped, the editor then reset.
    pub struct Handle {
        window: Option<WindowHandle>,
        _closing: super::Closing,
    }

    impl Handle {
        pub(super) fn new(window: Option<WindowHandle>, closing: super::Closing) -> Self {
            Handle {
                window,
                _closing: closing,
            }
        }
    }

    // SAFETY: the host drops the editor's handle on its GUI thread, where it opened it; the
    // raw pointers inside are not used from any other.
    unsafe impl Send for Handle {}

    impl Drop for Handle {
        fn drop(&mut self) {
            if let Some(w) = &mut self.window {
                w.close();
            }
        }
    }

    /// The keyboard as the drawer took it from another window (Windows: that window).
    #[derive(Debug)]
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub struct Keys {
        #[cfg(target_os = "windows")]
        had: winapi::shared::windef::HWND,
    }

    #[cfg(target_os = "windows")]
    fn hwnd(window: &baseview::Window<'_>) -> Option<winapi::shared::windef::HWND> {
        match window.raw_window_handle() {
            RawWindowHandle::Win32(h) if !h.hwnd.is_null() => Some(h.hwnd.cast()),
            _ => None,
        }
    }

    /// The keyboard to the editor's window (baseview's focus): on Windows, the window that
    /// had it if another did. Not on X11, where baseview's focus is `unimplemented!()` (it
    /// panicked the editor's thread, the window gone): there the keys go to the window under
    /// the pointer within the host's focused window, so to the drawer while the pointer is
    /// over the editor.
    pub fn take_keys(window: &mut baseview::Window<'_>) -> Option<Keys> {
        #[cfg(target_os = "windows")]
        {
            use winapi::um::winuser::GetFocus;
            let hwnd = hwnd(window)?;
            // SAFETY: the calling thread's focus, the editor's window being on it.
            let had = unsafe { GetFocus() };
            window.focus();
            (had != hwnd).then_some(Keys { had })
        }
        #[cfg(target_os = "macos")]
        {
            window.focus();
            None
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            let _ = window;
            None
        }
    }

    /// The keyboard given back as the drawer shuts, to the window it was taken from (else
    /// the host's window holding the editor's), if the editor's window still has it.
    /// baseview on Windows keeps every key its window is given, the host's shortcuts too.
    pub fn give_keys_back(window: &baseview::Window<'_>, keys: Keys) {
        #[cfg(target_os = "windows")]
        {
            use winapi::um::winuser::{GetFocus, GetParent, IsWindow, SetFocus};
            let Some(hwnd) = hwnd(window) else { return };
            // SAFETY: the calling thread's focus and windows; `keys.had` is checked to be a
            // window still before it is given the focus.
            unsafe {
                if GetFocus() != hwnd {
                    return;
                }
                let to = if !keys.had.is_null() && IsWindow(keys.had) != 0 {
                    keys.had
                } else {
                    GetParent(hwnd)
                };
                SetFocus(to);
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = (window, keys);
        }
    }

    /// Whether any of the editor's window can be seen: macOS's occlusion state of the window
    /// holding it (covered whole, minimised, on another Space or its application hidden: not
    /// seen); on Windows, its top window not minimised (not `IsWindowVisible`: in a session
    /// without a desktop on the screen, as an agent's over SSH, every window says it is not
    /// visible though it is drawn and shown, and the editor would show nothing). X11 says
    /// nothing reliable without a compositor: always seen there.
    // (objc 0.2's `msg_send!` asks after a `cargo-clippy` feature this crate does not have.)
    #[cfg_attr(target_os = "macos", allow(unexpected_cfgs))]
    pub fn seen(window: &baseview::Window<'_>) -> bool {
        #[cfg(target_os = "macos")]
        {
            use cocoa::appkit::{NSWindow, NSWindowOcclusionState};
            use cocoa::base::{id, nil};
            use objc::{msg_send, sel, sel_impl};
            let RawWindowHandle::AppKit(h) = window.raw_window_handle() else {
                return true;
            };
            let view = h.ns_view as id;
            if view == nil {
                return true;
            }
            // SAFETY: baseview's view, on the main thread that runs its frames; its window
            // is nil until the host puts it in one.
            let holder: id = unsafe { msg_send![view, window] };
            if holder == nil {
                return true;
            }
            // SAFETY: `holder` is an NSWindow.
            let state = unsafe { holder.occlusionState() };
            state.contains(NSWindowOcclusionState::NSWindowOcclusionStateVisible)
        }
        #[cfg(target_os = "windows")]
        {
            use winapi::um::winuser::{GA_ROOT, GetAncestor, IsIconic};
            let Some(hwnd) = hwnd(window) else {
                return true;
            };
            // SAFETY: the editor's window and its top window, on the thread that owns them.
            unsafe {
                let top = GetAncestor(hwnd, GA_ROOT);
                top.is_null() || IsIconic(top) == 0
            }
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            let _ = window;
            true
        }
    }

    /// Stricter than repaint visibility: an unmapped scan/test editor must not consume
    /// What's New. A remote AU's backing window can be occluded while its attached view
    /// is presented by the host, so only that case ignores native occlusion (R-AU-REMOTE-VISIBILITY).
    #[cfg_attr(target_os = "macos", allow(unexpected_cfgs))]
    pub fn release_notes_visible(window: &baseview::Window<'_>, reliable: bool) -> bool {
        #[cfg(target_os = "macos")]
        {
            use cocoa::base::{id, nil};
            use objc::{msg_send, sel, sel_impl};
            let RawWindowHandle::AppKit(h) = window.raw_window_handle() else {
                return false;
            };
            let view = h.ns_view as id;
            if view == nil {
                return false;
            }
            // SAFETY: baseview's live NSView, read on its GUI thread.
            let (holder, hidden): (id, objc::runtime::BOOL) = unsafe {
                (
                    msg_send![view, window],
                    msg_send![view, isHiddenOrHasHiddenAncestor],
                )
            };
            if holder == nil || hidden != objc::runtime::NO {
                return false;
            }
            // SAFETY: the live view's NSWindow, on its GUI thread. BOOL differs between
            // Intel and Apple silicon; Rust's bool is not AppKit's return type on Intel.
            let visible: objc::runtime::BOOL = unsafe { msg_send![holder, isVisible] };
            visible != objc::runtime::NO && (!reliable || seen(window))
        }
        #[cfg(target_os = "windows")]
        {
            use winapi::um::winuser::IsWindowVisible;
            let _ = reliable;
            // SAFETY: baseview's live HWND, read on its GUI thread.
            hwnd(window).is_some_and(|hwnd| unsafe { IsWindowVisible(hwnd) } != 0) && seen(window)
        }
        #[cfg(target_os = "linux")]
        {
            use x11rb::protocol::xproto::{ConnectionExt, MapState};
            let _ = reliable;
            let xid = match window.raw_window_handle() {
                RawWindowHandle::Xcb(h) => h.window,
                RawWindowHandle::Xlib(h) => h.window as u32,
                _ => return false,
            };
            let Ok((connection, _)) = x11rb::connect(None) else {
                return false;
            };
            connection
                .get_window_attributes(xid)
                .ok()
                .and_then(|cookie| cookie.reply().ok())
                .is_some_and(|attributes| attributes.map_state == MapState::VIEWABLE)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            let _ = (window, reliable);
            false
        }
    }

    /// Whether the editor's window has lost the pointer's capture (Windows: baseview
    /// captures it at a press and lets it go at the release, and ignores its loss, at Alt-Tab
    /// mid-drag, when no release comes). Elsewhere a release always comes.
    pub fn lost_capture(window: &baseview::Window<'_>) -> bool {
        #[cfg(target_os = "windows")]
        {
            use winapi::um::winuser::GetCapture;
            // SAFETY: the calling thread's capture.
            hwnd(window).is_some_and(|hwnd| unsafe { GetCapture() } != hwnd)
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = window;
            false
        }
    }

    /// The pointer's capture let go once no button is down (Windows). baseview counts the
    /// presses and releases to let it go; a release lost with the capture leaves the count
    /// one up, and its window then kept the capture after every release, the host's own
    /// clicks coming to the editor.
    pub fn release_capture(window: &baseview::Window<'_>) {
        #[cfg(target_os = "windows")]
        {
            use winapi::um::winuser::{
                GetCapture, GetKeyState, ReleaseCapture, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON,
                VK_XBUTTON1, VK_XBUTTON2,
            };
            let Some(hwnd) = hwnd(window) else { return };
            // SAFETY: the calling thread's capture and the buttons as its messages have them.
            unsafe {
                let down = [VK_LBUTTON, VK_RBUTTON, VK_MBUTTON, VK_XBUTTON1, VK_XBUTTON2]
                    .iter()
                    .any(|&b| GetKeyState(b) < 0);
                if !down && GetCapture() == hwnd {
                    ReleaseCapture();
                }
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = window;
        }
    }

    /// baseview's window as softbuffer takes it.
    #[derive(Clone)]
    pub struct Target {
        display: rwh6::RawDisplayHandle,
        window: rwh6::RawWindowHandle,
    }

    impl rwh6::HasDisplayHandle for Target {
        fn display_handle(&self) -> Result<rwh6::DisplayHandle<'_>, rwh6::HandleError> {
            // SAFETY: the display outlives the window drawn in, which outlives the surface.
            Ok(unsafe { rwh6::DisplayHandle::borrow_raw(self.display) })
        }
    }

    impl rwh6::HasWindowHandle for Target {
        fn window_handle(&self) -> Result<rwh6::WindowHandle<'_>, rwh6::HandleError> {
            // SAFETY: the window outlives its surface (both are the editor window's).
            Ok(unsafe { rwh6::WindowHandle::borrow_raw(self.window) })
        }
    }

    fn target(window: &baseview::Window<'_>) -> Option<Target> {
        let display = match window.raw_display_handle() {
            RawDisplayHandle::AppKit(_) => {
                rwh6::RawDisplayHandle::AppKit(rwh6::AppKitDisplayHandle::new())
            }
            RawDisplayHandle::Xlib(h) => rwh6::RawDisplayHandle::Xlib(
                rwh6::XlibDisplayHandle::new(NonNull::new(h.display), h.screen),
            ),
            RawDisplayHandle::Xcb(h) => rwh6::RawDisplayHandle::Xcb(rwh6::XcbDisplayHandle::new(
                NonNull::new(h.connection),
                h.screen,
            )),
            RawDisplayHandle::Windows(_) => {
                rwh6::RawDisplayHandle::Windows(rwh6::WindowsDisplayHandle::new())
            }
            _ => return None,
        };
        let window = match window.raw_window_handle() {
            RawWindowHandle::AppKit(h) => rwh6::RawWindowHandle::AppKit(
                rwh6::AppKitWindowHandle::new(NonNull::new(h.ns_view)?),
            ),
            RawWindowHandle::Xlib(h) => {
                rwh6::RawWindowHandle::Xlib(rwh6::XlibWindowHandle::new(h.window))
            }
            RawWindowHandle::Xcb(h) => {
                rwh6::RawWindowHandle::Xcb(rwh6::XcbWindowHandle::new(NonZeroU32::new(h.window)?))
            }
            RawWindowHandle::Win32(h) => {
                let mut w = rwh6::Win32WindowHandle::new(NonZeroIsize::new(h.hwnd as isize)?);
                w.hinstance = NonZeroIsize::new(h.hinstance as isize);
                rwh6::RawWindowHandle::Win32(w)
            }
            _ => return None,
        };
        Some(Target { display, window })
    }

    /// softbuffer's call, or nothing if it panicked: its Win32 backend asserts that GDI
    /// gave it a device context and a bitmap, which without a desktop it may not, and a
    /// panic there crossed into the host (pluginval crashed, decisions.md R18).
    fn guarded<T>(f: impl FnOnce() -> T) -> Option<T> {
        catch_unwind(AssertUnwindSafe(f)).ok()
    }

    /// A pixel of a frame (premultiplied RGBA, opaque) as the window shows it, opaque: its
    /// highest byte 0xff. softbuffer asks for 0 there, which every backend it is built with
    /// here ignores (Core Graphics skips it, GDI's copy and KMS's XRGB too) except X11 on a
    /// window with an alpha channel, which baseview makes wherever the screen has a 32-bit
    /// visual: there 0 is transparent, and on a compositing desktop the desktop shows through
    /// the panel, added to its colours (decisions.md R38).
    pub fn shown(p: &[u8]) -> u32 {
        0xff00_0000 | (u32::from(p[0]) << 16) | (u32::from(p[1]) << 8) | u32::from(p[2])
    }

    /// The window beyond the frames, opaque as they are.
    pub const EMPTY: u32 = 0xff3b_2213;

    /// The window's pixels (`w` by `h`, a row after another) from frames shown one under the
    /// other from its top left corner, [`EMPTY`] beyond them: a row at a time, each frame's
    /// row turned as a run of pixels (the compiler makes a short loop of it; a pixel at a time,
    /// with a frame found for each, took several times as long).
    pub fn fill(buffer: &mut [u32], (w, h): (usize, usize), frames: &[&ca72_panel::Pixmap]) {
        let mut frames = frames.iter();
        let mut frame = frames.next();
        let mut top = 0usize;
        for (y, row) in buffer.chunks_exact_mut(w).take(h).enumerate() {
            while let Some(f) = frame
                && y >= top + f.height() as usize
            {
                top += f.height() as usize;
                frame = frames.next();
            }
            let n = match frame {
                Some(f) => {
                    let fw = f.width() as usize;
                    let n = fw.min(w);
                    let from = (y - top) * fw * 4;
                    let src = &f.data()[from..from + n * 4];
                    for (out, p) in row[..n].iter_mut().zip(src.chunks_exact(4)) {
                        *out = shown(p);
                    }
                    n
                }
                None => 0,
            };
            row[n..].fill(EMPTY);
        }
    }

    /// The window's pixels in the places `damage` (left, top, right, bottom) alone, as
    /// [`fill`] makes them there, the rest left as it was: the places, cut to the window, that
    /// were turned.
    pub fn fill_places(
        buffer: &mut [u32],
        (w, h): (usize, usize),
        frames: &[&ca72_panel::Pixmap],
        damage: &[[i32; 4]],
    ) -> Vec<[i32; 4]> {
        let mut turned = Vec::with_capacity(damage.len());
        for &[x0, y0, x1, y1] in damage {
            let cut = |v: i32, most: usize| v.clamp(0, most as i32) as usize;
            let (x0, y0, x1, y1) = (cut(x0, w), cut(y0, h), cut(x1, w), cut(y1, h));
            if x1 <= x0 || y1 <= y0 {
                continue;
            }
            turned.push([x0 as i32, y0 as i32, x1 as i32, y1 as i32]);
            let mut frames = frames.iter();
            let mut frame = frames.next();
            let mut top = 0usize;
            for y in y0..y1 {
                while let Some(f) = frame
                    && y >= top + f.height() as usize
                {
                    top += f.height() as usize;
                    frame = frames.next();
                }
                let row = &mut buffer[y * w + x0..y * w + x1];
                let n = match frame {
                    Some(f) if (f.width() as usize) > x0 => {
                        let fw = f.width() as usize;
                        let n = fw.min(x1) - x0;
                        let from = ((y - top) * fw + x0) * 4;
                        let src = &f.data()[from..from + n * 4];
                        for (out, p) in row[..n].iter_mut().zip(src.chunks_exact(4)) {
                            *out = shown(p);
                        }
                        n
                    }
                    _ => 0,
                };
                row[n..].fill(EMPTY);
            }
        }
        turned
    }

    /// Where the frames are shown; nothing if the platform's window could not be drawn in.
    pub struct Surface {
        inner: Option<(
            softbuffer::Context<Target>,
            softbuffer::Surface<Target, Target>,
        )>,
        /// Whether what the window shows is the frame last given, every pixel of it: not
        /// since the surface was made or resized, nor after a frame that could not be shown.
        whole: bool,
    }

    impl Surface {
        pub fn new(window: &baseview::Window<'_>, size: (u32, u32)) -> Self {
            let inner = target(window).and_then(|t| {
                guarded(|| {
                    let context = softbuffer::Context::new(t.clone()).ok()?;
                    let surface = softbuffer::Surface::new(&context, t).ok()?;
                    Some((context, surface))
                })
                .flatten()
            });
            let mut s = Surface {
                inner,
                whole: false,
            };
            s.resize(size);
            s
        }

        #[cfg(all(test, target_os = "linux"))]
        pub fn presented(&self) -> bool {
            self.whole
        }

        pub fn resize(&mut self, (w, h): (u32, u32)) {
            self.whole = false;
            if let (Some((_, surface)), Some(w), Some(h)) =
                (&mut self.inner, NonZeroU32::new(w), NonZeroU32::new(h))
                && guarded(|| surface.resize(w, h)).is_none()
            {
                self.inner = None;
            }
        }

        /// Shows frames (premultiplied RGBA, opaque), one under the other from the top left
        /// corner, in a window of `size` pixels: where they changed since those last shown, in
        /// the places `damage` (left, top, right, bottom), when it is given, or anywhere.
        pub fn present(
            &mut self,
            size: (u32, u32),
            frames: &[&ca72_panel::Pixmap],
            damage: Option<&[[i32; 4]]>,
        ) {
            let Some((_, surface)) = &mut self.inner else {
                return;
            };
            let damage = damage.filter(|_| self.whole);
            match guarded(|| Self::show(surface, size, frames, damage)) {
                Some(shown) => self.whole = shown,
                None => {
                    self.whole = false;
                    self.inner = None;
                }
            }
        }

        /// Whether the frames were shown, all of them.
        fn show(
            surface: &mut softbuffer::Surface<Target, Target>,
            (w, h): (u32, u32),
            frames: &[&ca72_panel::Pixmap],
            damage: Option<&[[i32; 4]]>,
        ) -> bool {
            let Ok(mut buffer) = surface.buffer_mut() else {
                return false;
            };
            let (w, h) = (w as usize, h as usize);
            if buffer.len() < w * h {
                return false;
            }
            // Where the buffer still holds the frame shown last (age 1: a buffer kept, as
            // Windows' and X11's are, and macOS's in this copy of softbuffer, shown in tiles),
            // only the places changed are turned and sent; else the whole frame (decisions.md
            // R58, R59).
            match damage.filter(|_| buffer.age() == 1) {
                Some(damage) => {
                    let rects: Vec<softbuffer::Rect> =
                        fill_places(&mut buffer[..], (w, h), frames, damage)
                            .into_iter()
                            .filter_map(|[x0, y0, x1, y1]| {
                                Some(softbuffer::Rect {
                                    x: x0 as u32,
                                    y: y0 as u32,
                                    width: NonZeroU32::new((x1 - x0) as u32)?,
                                    height: NonZeroU32::new((y1 - y0) as u32)?,
                                })
                            })
                            .collect();
                    buffer.present_with_damage(&rects).is_ok()
                }
                None => {
                    fill(&mut buffer[..], (w, h), frames);
                    buffer.present().is_ok()
                }
            }
        }
    }
}

/// ULTRA's note's lines (decisions.md R-ULTRA-HIFI): its numerical limit and, on a computer
/// that splits a voice over six threads (`splits`), what can be played live.
fn ultra_note_lines(splits: bool) -> Vec<(&'static str, Tone)> {
    let mut lines = vec![
        ("ULTRA · HIGH FIDELITY", Tone::Accent),
        (
            "THE CIRCUIT'S MODEL, WITHIN -120 dBFS OF ITS NUMERICAL REFERENCE.",
            Tone::Plain,
        ),
        (
            "ON MEASURED PATHS; CHAOTIC FEEDBACK CAN DIVERGE.",
            Tone::Plain,
        ),
    ];
    if splits {
        lines.extend([
            // (Wrapped so that no line is wider than the note's widest before.)
            (
                "ON THIS COMPUTER ONE VOICE IS SPREAD OVER SIX THREADS AND",
                Tone::Plain,
            ),
            (
                "CAN BE PLAYED LIVE. WITH MULTIPLE VOICES OR TOO SMALL OF A BUFFER,",
                Tone::Plain,
            ),
            ("IT MAY DROP OUT.", Tone::Plain),
            (
                "IT IS MEANT FOR OFFLINE RENDERS (EXPORT, BOUNCE, FREEZE) AND SMALL",
                Tone::Plain,
            ),
            ("PROJECTS ON POWERFUL COMPUTERS.", Tone::Plain),
        ]);
    } else {
        lines.extend([
            (
                "ONE VOICE TAKES UP TO A WHOLE CORE OF A FAST COMPUTER.",
                Tone::Plain,
            ),
            (
                "IT IS MEANT FOR OFFLINE RENDERS (EXPORT, BOUNCE, FREEZE) AND SMALL",
                Tone::Plain,
            ),
            (
                "PROJECTS ON POWERFUL COMPUTERS. PLAYED LIVE, IT MAY DROP OUT.",
                Tone::Plain,
            ),
        ]);
    }
    lines.extend([
        (
            "RIGHT-CLICK QUALITY'S SWITCH TO HIDE ITS INDICATOR, OR SHOW IT.",
            Tone::Dim,
        ),
        ("CLICK TO CLOSE", Tone::Dim),
    ]);
    lines
}

#[cfg(test)]
mod tests {
    mod fx_pins;
    mod readability;
    use std::sync::Mutex;

    use baseview::Point;
    use nih_plug::context::PluginApi;
    use nih_plug::wrapper::state::PluginState;

    use super::*;
    use ca72_panel::art::POWER;
    use ca72_panel::presets::{self as presets_ui, BarTarget, DrawerTarget, FieldId, PresetKey};
    use ca72_panel::ultra;

    /// What the editor asked of the host.
    #[derive(Debug, Clone, Copy, PartialEq)]
    enum Call {
        Begin(ParamPtr),
        Set(ParamPtr, f32),
        End(ParamPtr),
    }

    /// A host that records the editor's gestures (and does not apply them).
    #[derive(Default)]
    struct Host {
        api: Option<PluginApi>,
        calls: Mutex<Vec<Call>>,
        /// It will not resize the window.
        fixed: AtomicBool,
        /// How often it was asked to.
        resizes: AtomicU32,
        deferred: AtomicBool,
    }

    impl Host {
        fn take(&self) -> Vec<Call> {
            std::mem::take(&mut *self.calls.lock().expect("not poisoned"))
        }
        fn push(&self, c: Call) {
            self.calls.lock().expect("not poisoned").push(c);
        }
    }

    #[allow(unsafe_code)]
    impl GuiContext for Host {
        fn plugin_api(&self) -> PluginApi {
            self.api.unwrap_or(PluginApi::Clap)
        }
        fn request_resize(&self) -> bool {
            self.resizes.fetch_add(1, Ordering::Relaxed);
            !self.fixed.load(Ordering::Relaxed)
        }
        fn resize_response_will_follow(&self) -> bool {
            self.deferred.load(Ordering::Relaxed)
        }
        unsafe fn raw_begin_set_parameter(&self, param: ParamPtr) {
            self.push(Call::Begin(param));
        }
        unsafe fn raw_set_parameter_normalized(&self, param: ParamPtr, normalized: f32) {
            self.push(Call::Set(param, normalized));
        }
        unsafe fn raw_end_set_parameter(&self, param: ParamPtr) {
            self.push(Call::End(param));
        }
        fn get_state(&self) -> PluginState {
            panic!("the editor does not read the state")
        }
        fn set_state(&self, _state: PluginState) {}
    }

    #[test]
    fn release_notes_wait_for_a_visible_editor_and_reopen_without_changing_the_sound() {
        use crate::engine::{Engine, Event};
        let dir = tempfile::tempdir().expect("settings folder");
        let path = dir.path().join("CA-72/settings.toml");
        let (mut e, host, params) = editing();
        e.settings_file = Some(path.clone());
        e.update = Update::with(
            crate::update::fake::no_curl,
            crate::update::fake::no_browser,
        );
        e.draw();
        let controls = params.controls();
        let preset = crate::presets::values_now(&params);
        let fields = params.serialize_fields();
        let update = e.update.scene();
        let render = || {
            let mut engine = Engine::new();
            engine.set(&params.controls());
            engine.prepare(48_000.0, 12345);
            engine.event(Event::Note { key: 60, on: true });
            (0..4096)
                .map(|_| {
                    let (l, r) = engine.tick_stereo(0.0);
                    (l.to_bits(), r.to_bits())
                })
                .collect::<Vec<_>>()
        };
        let audio = render();
        assert!(audio.iter().any(|&(l, _)| f32::from_bits(l).abs() > 0.001));
        for _ in 0..3 {
            e.release_notes_when_visible(false);
            e.draw();
        }
        assert!(!e.whats_new_checked && e.whats_new.is_none());
        assert!(!path.exists(), "a hidden editor never consumes the history");
        e.release_notes_when_visible(true);
        assert!(e.whats_new.is_some());
        let saved = std::fs::read(&path).expect("claim kept");
        e.draw();
        assert!(keyed(&mut e, Key::Escape));
        assert!(e.whats_new.is_none());
        e.release_notes_when_visible(true);
        assert!(e.whats_new.is_none(), "dismissal is final for this editor");
        e.strip_press(StripTarget::Bar(BarTarget::Settings));
        assert!(menu_items(&e).contains(&("WHAT'S NEW…".to_owned(), true)));
        menu_item(&mut e, "WHAT'S NEW…");
        assert!(e.whats_new.is_some(), "manual reopening remains available");
        e.scrolled(ScrollDelta::Lines { x: 0.0, y: -2.0 }, Modifiers::empty());
        e.draw();
        assert!(keyed(&mut e, Key::End));
        assert!(keyed(&mut e, Key::Home));
        assert!(keyed(&mut e, Key::Enter));
        e.draw();
        assert!(e.whats_new.is_none());
        assert_eq!(params.controls(), controls);
        assert_eq!(crate::presets::values_now(&params), preset);
        assert_eq!(
            params.serialize_fields(),
            fields,
            "saved project fields unchanged"
        );
        assert_eq!(render(), audio, "audio remains bit-identical");
        assert!(host.take().is_empty(), "no sound parameter gestures");
        assert_eq!(e.update.scene(), update, "no network request");
        assert_eq!(
            std::fs::read(&path).expect("settings"),
            saved,
            "reopening/dismissal write nothing"
        );
        drop(e);
        for api in [PluginApi::Clap, PluginApi::Vst3, PluginApi::Standalone] {
            let (mut restarted, _, _) = editing_for_api(
                Ca72Params::default(),
                Library::at(dir.path().join("library")),
                api,
            );
            restarted.settings_file = Some(path.clone());
            restarted.release_notes_when_visible(true);
            assert!(
                restarted.whats_new.is_none(),
                "shared history after restart: {api}"
            );
        }
    }

    #[test]
    fn release_notes_do_not_interrupt_a_held_typing_note_or_lose_its_release() {
        let (mut e, _, _) = editing_standalone();
        e.playing.typing = true;
        let down = KeyboardEvent {
            key: Key::Character("a".into()),
            ..Default::default()
        };
        assert!(e.keyed(&down));
        assert!(e.playing.notes.pop().expect("attack").on);
        let generation = e.playing.notes.generation();
        e.show_release_notes(crate::whats_new::current());
        e.draw();
        assert_eq!(e.playing.notes.generation(), generation);
        assert!(
            e.playing.notes.pop().is_none(),
            "opening leaves the note sounding"
        );
        let up = KeyboardEvent {
            state: KeyState::Up,
            ..down
        };
        assert!(e.keyed(&up));
        assert!(!e.playing.notes.pop().expect("release through the notes").on);
        assert!(keyed(&mut e, Key::Escape));
    }

    /// What the notes cost a frame in a Retina window (3440 pixels across), full view: the
    /// frame that opens them, a frame in which the panel behind them changes, a frame that
    /// scrolls them, against a frame of the panel alone (`-- --ignored --nocapture`, in
    /// release).
    #[test]
    #[ignore = "prints timings"]
    fn release_notes_frame_timings() {
        let (mut e, _, _) = editing();
        e.params.editor_panel.store(true, Ordering::Relaxed);
        e.set_panel_height(true);
        e.fit(2.0, 1720, 3440);
        e.draw();
        let ms = |t: Instant, n: u32| t.elapsed().as_secs_f64() * 1000.0 / f64::from(n);
        let n = 20;
        let t = Instant::now();
        for _ in 0..n {
            e.composed = None;
            e.draw();
        }
        println!("the panel alone, all of it: {:.2} ms a frame", ms(t, n));
        let (mut opening, mut laid_out) = (Vec::new(), Vec::new());
        for _ in 0..5 {
            let t = Instant::now();
            e.show_release_notes(
                crate::whats_new::assemble(Some("0.2.3"), crate::update::VERSION)
                    .expect("combined notes"),
            );
            laid_out.push(ms(t, 1));
            e.draw();
            opening.push(ms(t, 1));
            assert!(keyed(&mut e, Key::Escape));
            e.draw();
        }
        println!("opening the notes: {opening:.2?} ms (laid out in {laid_out:.2?} ms)");
        e.show_release_notes(
            crate::whats_new::assemble(Some("0.2.3"), crate::update::VERSION)
                .expect("combined notes"),
        );
        e.draw();
        let t = Instant::now();
        for _ in 0..n {
            e.composed = None;
            e.draw();
        }
        println!("the panel changing behind them: {:.2} ms a frame", ms(t, n));
        let t = Instant::now();
        for i in 0..n {
            keyed(
                &mut e,
                if i % 2 == 0 {
                    Key::ArrowDown
                } else {
                    Key::ArrowUp
                },
            );
            e.draw();
        }
        println!("a line scrolled: {:.2} ms a frame", ms(t, n));
    }

    /// The notes are a modal dialog in the standalone too (the owner, 2026-10-19, of the
    /// stand-in: "on standalone, i don't like that this What's New screen wasn't a modal"):
    /// at its default size and its smallest, compact and full, a box inset from the window's
    /// edges over the editor dimmed, which takes no click, wheel or key meanwhile; GOT IT
    /// closes it. With CA72_WHATS_NEW_PNG, a folder, the frames written there.
    #[test]
    fn release_notes_are_a_modal_dialog_in_the_standalone_at_every_size() {
        for width in [DEFAULT_WIDTH, MIN_WIDTH] {
            for expanded in [false, true] {
                let (mut e, host, params) = editing_standalone();
                e.params.editor_panel.store(expanded, Ordering::Relaxed);
                e.set_panel_height(expanded);
                e.fit(1.0, width, width);
                e.show_release_notes(
                    crate::whats_new::assemble(Some("0.2.3"), crate::update::VERSION)
                        .expect("combined notes"),
                );
                e.draw();
                let frame = e.composed.as_ref().expect("notes frame");
                assert_eq!(frame.width(), width);
                if let Some(out) = std::env::var_os("CA72_WHATS_NEW_PNG") {
                    let out = std::path::Path::new(&out);
                    std::fs::create_dir_all(out).expect("evidence folder");
                    let view = if expanded { "full" } else { "compact" };
                    frame
                        .save_png(out.join(format!("standalone-{view}-{width}.png")))
                        .expect("PNG");
                }
                // The window's edges show the editor, dimmed, round the dialog's box.
                let (w, h) = (frame.width() - 1, frame.height() - 1);
                for (x, y) in [(0, 0), (w, 0), (0, h), (w, h), (w / 2, 2), (w / 2, h - 2)] {
                    let p = frame.pixel(x, y).expect("pixel");
                    assert_ne!(
                        [p.red(), p.green(), p.blue()],
                        [0x1f, 0x22, 0x28],
                        "({x}, {y}) is outside the box"
                    );
                }
                let notes = e.whats_new.as_ref().expect("notes");
                let [x, y, bw, bh] = notes.button();
                let bottom = f64::from(e.physical_height()) / e.renderer.scale();
                assert!(x + bw < art::W - 300.0 && y + bh < bottom - 40.0);
                // Nothing behind it takes a press, a wheel or a key.
                let controls = params.controls();
                for place in [(230.0, 400.0), (1500.0, 600.0), (3000.0, bottom - 20.0)] {
                    let p = at(&e, place);
                    click(&mut e, p);
                    e.scrolled(ScrollDelta::Lines { x: 0.0, y: 1.0 }, Modifiers::empty());
                    e.context_menu();
                }
                e.playing.typing = true;
                let key = KeyboardEvent {
                    key: Key::Character("a".into()),
                    ..Default::default()
                };
                assert!(e.keyed(&key), "the notes keep the keys");
                assert!(e.playing.notes.pop().is_none(), "no note played");
                assert!(e.whats_new.is_some() && e.learning.menu().is_none());
                assert!(!e.browser.open);
                assert_eq!(params.controls(), controls);
                assert!(host.take().is_empty());
                let close = at(&e, (x + bw / 2.0, y + bh / 2.0));
                click(&mut e, close);
                assert!(e.whats_new.is_none());
            }
        }
    }

    /// A trackpad's travel the other way starts afresh (PR #106's review): what was left of
    /// a line from one way does not eat the first line of the other.
    #[test]
    fn release_notes_scroll_back_at_once_when_a_trackpad_turns() {
        let (mut e, _, _) = editing();
        e.show_release_notes(
            crate::whats_new::assemble(Some("0.2.3"), crate::update::VERSION)
                .expect("combined notes"),
        );
        e.draw();
        let notes = e.whats_new.as_mut().expect("notes");
        assert!(notes.scrolls());
        notes.scroll(3);
        let line = notes.line_height() * e.renderer.scale() / e.dpr;
        let pixels = |lines: f64| ScrollDelta::Pixels {
            x: 0.0,
            y: (lines * line) as f32,
        };
        // Most of a line down, then a little up and most of one up: a line up.
        e.scrolled(pixels(-0.9), Modifiers::empty());
        assert_eq!(e.whats_new.as_ref().expect("notes").position(), 3);
        e.scrolled(pixels(0.3), Modifiers::empty());
        e.scrolled(pixels(0.8), Modifiers::empty());
        assert_eq!(e.whats_new.as_ref().expect("notes").position(), 2);
        // And the same way, fractions add up to whole lines.
        for _ in 0..4 {
            e.scrolled(pixels(-0.6), Modifiers::empty());
        }
        assert_eq!(e.whats_new.as_ref().expect("notes").position(), 4);
        e.scrolled(ScrollDelta::Lines { x: 0.0, y: 2.0 }, Modifiers::empty());
        assert_eq!(e.whats_new.as_ref().expect("notes").position(), 2);
    }

    #[test]
    fn release_notes_fit_both_looks_lettering_sizes_and_compact_views() {
        for skin in [Skin::Drawn, Skin::Worn] {
            for lettering in [Lettering::Standard, Lettering::Large] {
                for expanded in [false, true] {
                    let (mut e, host, _) = editing();
                    e.set_look(skin);
                    e.set_lettering(lettering);
                    e.params.editor_panel.store(expanded, Ordering::Relaxed);
                    e.set_panel_height(expanded);
                    e.fit(1.0, MIN_WIDTH, MIN_WIDTH);
                    e.show_release_notes(
                        crate::whats_new::assemble(Some("0.2.3"), crate::update::VERSION)
                            .expect("combined notes"),
                    );
                    e.draw();
                    assert_eq!(e.expanded(), expanded);
                    assert_eq!(
                        e.composed.as_ref().expect("notes frame").height(),
                        e.physical_height()
                    );
                    // (With CA72_WHATS_NEW_PNG, a folder: the frames written there, at the
                    // smallest width and at the mock-up's, 1554 pixels.)
                    if let Some(out) = std::env::var_os("CA72_WHATS_NEW_PNG") {
                        let out = std::path::Path::new(&out);
                        std::fs::create_dir_all(out).expect("evidence folder");
                        let view = if expanded { "full" } else { "compact" };
                        for width in [MIN_WIDTH, 1554] {
                            e.fit(1.0, width, width);
                            e.draw();
                            e.composed
                                .as_ref()
                                .expect("notes frame")
                                .save_png(out.join(format!(
                                    "notes-{skin:?}-{lettering:?}-{view}-{width}.png"
                                )))
                                .expect("PNG");
                        }
                        // Scrolled to the end; and at the full view, an update from 0.2.5,
                        // the mock-up's.
                        keyed(&mut e, Key::End);
                        e.draw();
                        let frame = e.composed.as_ref().expect("notes frame");
                        frame
                            .save_png(
                                out.join(format!(
                                    "notes-{skin:?}-{lettering:?}-{view}-1554-end.png"
                                )),
                            )
                            .expect("PNG");
                        if expanded {
                            e.show_release_notes(
                                crate::whats_new::assemble(Some("0.2.5"), crate::update::VERSION)
                                    .expect("an update"),
                            );
                            e.draw();
                            e.composed
                                .as_ref()
                                .expect("notes frame")
                                .save_png(out.join(format!(
                                    "notes-{skin:?}-{lettering:?}-since-0.2.5-1554.png"
                                )))
                                .expect("PNG");
                        }
                        e.fit(1.0, MIN_WIDTH, MIN_WIDTH);
                        e.draw();
                    }
                    let [x, y, w, h] = e.whats_new.as_ref().expect("notes").button();
                    assert!(y + h < f64::from(e.physical_height()) / e.renderer.scale());
                    let close = at(&e, (x + w / 2.0, y + h / 2.0));
                    click(&mut e, close);
                    assert!(e.whats_new.is_none(), "close button works in each view");
                    assert!(host.take().is_empty());
                }
            }
        }
    }

    fn editing() -> (Editing, Arc<Host>, Arc<Ca72Params>) {
        // A library that is not there (nothing of the user's read; these tests write none).
        editing_with(Library::at(
            std::env::temp_dir().join("ca72-editor-tests-no-library"),
        ))
    }

    /// R-AUTO-GAIN-DRIVE (the owner: "This should only be remeasured when drive changes"):
    /// with DRIVE up, the window opening, CUTOFF turned, a key, a preset chosen and QUALITY
    /// switched ask for no measurement of AUTO GAIN's curve, so nothing arrives while a note
    /// plays; the preset's own curve is in force at once. DRIVE changed here asks; so does AUTO
    /// GAIN switched on, and nothing does with AUTO GAIN off.
    #[test]
    fn only_drive_or_auto_gain_changed_here_asks_for_auto_gains_curve() {
        // (The test's host applies nothing: the parameters stay at their defaults, DRIVE 0,
        // AUTO GAIN on; the editor goes by what it set.)
        let (mut e, host, params) = editing();
        let cal = &params.drive_curve;
        let asked = |e: &mut Editing| {
            e.measure_when_done();
            cal.waiting()
        };
        assert!(!asked(&mut e), "the window opening");
        let cutoff = CONTROLS
            .iter()
            .position(|c| c.param == "cutoff")
            .expect("CUTOFF");
        e.set_once(cutoff, 0.7);
        assert!(!host.take().is_empty());
        assert!(!asked(&mut e), "CUTOFF turned");
        keyed(&mut e, Key::Character("a".into()));
        assert!(!asked(&mut e), "a key");
        let quality = CONTROLS
            .iter()
            .position(|c| c.param == "quality")
            .expect("QUALITY");
        e.set_once(quality, 1.0);
        assert!(!asked(&mut e), "QUALITY switched");
        // A factory preset: its curve (QUALITY LO, as the instance is) at once, nothing asked.
        let smoke = crate::library::factory()
            .iter()
            .find(|s| s.name == "Smoke Flute")
            .expect("Smoke Flute")
            .clone();
        let entry = e.browser.library.find(&smoke.name).expect("listed");
        // (Through the host, as the presets set the parameters.)
        let host_context: &dyn GuiContext = host.as_ref();
        let setter = ParamSetter::new(host_context);
        e.browser.choose(&entry, &params, &setter);
        assert_eq!(Some(cal.saved().db), smoke.drive_curve_lo);
        assert!(!asked(&mut e), "a preset chosen");
        // DRIVE typed into its readout: asked, once nothing is held.
        let readout = on_strip(&e, strip_at(StripTarget::Readout(Readout::Drive)));
        click(&mut e, readout);
        click(&mut e, readout);
        for c in "13".chars() {
            keyed(&mut e, Key::Character(c.to_string()));
        }
        keyed(&mut e, Key::Enter);
        assert!(asked(&mut e), "DRIVE changed here");
        // Measured: the sound's own curve; AUTO GAIN switched off and on here asks again only
        // when it is not the sound's.
        cal.set_rate(crate::engine::MIN_VOICE_RATE);
        cal.measure_now();
        assert!(!cal.waiting());
        // (As the strip's switch sets it.)
        let switch = |e: &Editing, on: bool| {
            let s = e.setter();
            s.begin_set_parameter(&params.auto_gain);
            s.set_parameter(&params.auto_gain, on);
            s.end_set_parameter(&params.auto_gain);
        };
        switch(&e, true);
        assert!(
            !asked(&mut e),
            "AUTO GAIN on, the curve already the sound's"
        );
        switch(&e, false);
        assert!(!asked(&mut e), "AUTO GAIN off: nothing to measure");
        // (Another sound meanwhile: the preset's curve again, its key none.)
        e.browser.choose(&entry, &params, &setter);
        switch(&e, true);
        assert!(
            asked(&mut e),
            "AUTO GAIN switched on, the curve not the sound's"
        );
    }

    #[test]
    fn typing_octave_is_independent_of_panel_octaves_and_sound_parameters() {
        let (mut e, host, _) = editing_standalone();
        e.playing.visible = true;
        let event = |state, character: &str| KeyboardEvent {
            state,
            key: Key::Character(character.into()),
            ..Default::default()
        };
        assert!(
            !e.keyed(&event(KeyState::Down, "x")),
            "TYPE starts disabled"
        );
        e.playing.typing = true;
        assert!(e.keyed(&event(KeyState::Down, "a")));
        assert_eq!(e.playing.notes.pop().expect("C3 attack").note, 48);
        let p = at(&e, (keyboard::UP_X, keyboard::HEADER));
        click(&mut e, p);
        assert_eq!((e.playing.root, e.playing.typing_root), (53, 48));
        assert!(
            e.playing.notes.pop().is_none(),
            "range button leaves TYPE held"
        );
        assert!(e.keyed(&event(KeyState::Down, "x")));
        assert_eq!((e.playing.root, e.settings.keyboard_root), (53, 53));
        assert_eq!((e.playing.typing_root, e.settings.typing_root), (60, 60));
        assert!(!e.playing.notes.pop().expect("original pitch released").on);
        assert!(
            host.take().is_empty(),
            "keyboard octaves are outside the sound"
        );
        assert!(e.keyed(&event(KeyState::Up, "x")));
        assert!(e.keyed(&event(KeyState::Up, "a")));
        assert!(e.keyed(&event(KeyState::Down, "a")));
        assert_eq!(e.playing.notes.pop().expect("C4 attack").note, 60);
        e.strip_press(StripTarget::Bar(BarTarget::Name));
        assert!(!e.note_typing_allowed(), "preset search owns letters");
        e.keyed(&event(KeyState::Down, "z"));
        assert_eq!(e.playing.typing_root, 60);
    }

    #[test]
    fn panel_hide_and_page_selection_preserve_type_notes() {
        let (mut e, _, _) = editing_standalone();
        e.playing.visible = true;
        e.playing.typing = true;
        let key = KeyboardEvent {
            key: Key::Character("a".into()),
            ..Default::default()
        };
        assert!(e.expanded());
        assert!(e.keyed(&key));
        assert!(e.playing.notes.pop().expect("attack").on);
        let generation = e.playing.notes.generation();
        e.toggle_panel();
        assert!(!e.expanded());
        assert_eq!(e.playing.notes.generation(), generation);
        assert!(e.note_typing_allowed());
        assert!(e.keyed(&key), "TYPE still owns a held key in compact view");
        assert!(
            e.playing.notes.pop().is_none(),
            "held key does not retrigger"
        );
        let compact_height = e.window_height();
        e.strip_press(StripTarget::Bar(BarTarget::Global));
        assert!(!e.playing.visible);
        assert!(e.note_typing_allowed(), "GLOBAL preserves TYPE");
        assert_eq!(e.playing.notes.generation(), generation);
        assert!(e.window_height() > compact_height);
        e.strip_press(StripTarget::Bar(BarTarget::Keyboard));
        assert!(e.playing.visible);
        assert!(e.expanded());
        assert!(e.note_typing_allowed());
        let mut up = key.clone();
        up.state = KeyState::Up;
        assert!(e.keyed(&up));
        assert!(!e.playing.notes.pop().expect("original attack released").on);
        assert!(e.keyed(&key));
        assert!(e.playing.notes.pop().expect("new attack").on);
    }

    #[test]
    fn keyboard_view_blocks_hidden_controls_and_releases_for_takeovers() {
        let (mut e, host, params) = editing_standalone();
        let width = e.width;
        let height = e.window_height();
        e.playing.visible = true;
        assert!(e.playing.visible);
        assert_eq!((e.width, e.window_height()), (width, height));
        for id in [
            "voices", "entropy", "spread", "inner", "double", "drive", "level",
        ] {
            let p = at(&e, centre(id));
            go(&mut e, p);
            press(&mut e);
            wheel(&mut e, 1.0);
            release(&mut e);
        }
        assert!(
            host.calls.lock().unwrap().is_empty(),
            "hidden controls cannot gesture"
        );
        let p = at(
            &e,
            art::tempo_key_at(e.renderer.arrangement(), art::TempoKey::Typing),
        );
        go(&mut e, p);
        press(&mut e);
        release(&mut e);
        assert!(e.note_typing_allowed());
        let key = KeyboardEvent {
            key: Key::Character("a".into()),
            ..Default::default()
        };
        assert!(e.keyed(&key));
        let generation = e.playing.notes.generation();
        e.strip_press(StripTarget::Bar(BarTarget::Name));
        assert!(e.playing.notes.generation() > generation);
        assert!(!e.note_typing_allowed());
        e.browser.show(false, &params);
        e.follow_drawer();
        // A readout's typing and a settings menu take over the keyboard as well.
        let generation = e.playing.notes.generation();
        e.start_typing(Readout::Level);
        assert!(e.playing.notes.generation() > generation);
        assert!(!e.note_typing_allowed());
        e.end_gestures();
        e.strip_press(StripTarget::Bar(BarTarget::Settings));
        assert!(!e.note_typing_allowed());
        e.learning.close_menu();
        e.strip_press(StripTarget::Bar(BarTarget::Global));
        assert!(!e.playing.visible);
        assert_eq!((e.width, e.window_height()), (width, height));
    }

    #[test]
    fn keyboard_pointer_releases_outside_and_editor_drop_invalidates_pending_notes() {
        let (mut e, _, _) = editing();
        e.playing.visible = true;
        let r = keyboard::rect(41, 48).unwrap();
        let p = at(&e, ((r[0] + r[2]) / 2.0, r[3] - 20.0));
        go(&mut e, p);
        press(&mut e);
        assert!(e.playing.dragging());
        assert!(e.playing.notes.pop().expect("mouse attack").on);
        go(&mut e, Point { x: -20.0, y: -20.0 });
        assert!(!e.playing.notes.pop().expect("outside release").on);
        assert!(e.playing.dragging(), "can glissando back inside");
        go(&mut e, p);
        assert!(e.playing.notes.pop().expect("reenter attack").on);
        release(&mut e);
        assert!(!e.playing.notes.pop().expect("release").on);
        go(&mut e, p);
        press(&mut e);
        let notes = Arc::clone(&e.playing.notes);
        let generation = notes.generation();
        drop(e);
        assert!(
            notes.generation() > generation,
            "close releases without relying on a queue slot"
        );
    }

    fn editing_with(library: Library) -> (Editing, Arc<Host>, Arc<Ca72Params>) {
        editing_of(Ca72Params::default(), library)
    }

    /// The editor with POLY on, where VOICES counts (decisions.md R64).
    fn editing_poly() -> (Editing, Arc<Host>, Arc<Ca72Params>) {
        let params = Ca72Params {
            poly: BoolParam::new("Poly", true),
            ..Ca72Params::default()
        };
        editing_of(
            params,
            Library::at(std::env::temp_dir().join("ca72-editor-tests-no-library")),
        )
    }

    /// An editor whose instance is at QUALITY HI (a new one is at LO: R-LO-DEFAULT).
    fn editing_at_hi() -> (Editing, Arc<Host>, Arc<Ca72Params>) {
        let params = Ca72Params {
            quality: EnumParam::new("Quality", QualityMode::Hi),
            ..Ca72Params::default()
        };
        editing_of(
            params,
            Library::at(std::env::temp_dir().join("ca72-editor-tests-no-library")),
        )
    }

    fn editing_of(params: Ca72Params, library: Library) -> (Editing, Arc<Host>, Arc<Ca72Params>) {
        editing_for_api(params, library, PluginApi::Clap)
    }

    fn editing_standalone() -> (Editing, Arc<Host>, Arc<Ca72Params>) {
        editing_for_api(
            Ca72Params::default(),
            Library::at(std::env::temp_dir().join("ca72-editor-tests-no-library")),
            PluginApi::Standalone,
        )
    }

    /// The systems' shortcuts for quitting an application, where the standalone's windows act on
    /// them (R-QUIT): Ctrl+Q on Linux; macOS's ⌘Q is its main menu's and Windows' Alt+F4 the
    /// system's, so neither is the windows'.
    #[cfg(feature = "standalone-production")]
    #[test]
    fn ctrl_q_quits_on_linux_and_no_other_key_does() {
        use nih_plug::wrapper::production::app_window::quits_on;
        let key = |key: Key, modifiers: Modifiers, state: KeyState| KeyboardEvent {
            key,
            modifiers,
            state,
            ..Default::default()
        };
        let q = |m| key(Key::Character("q".into()), m, KeyState::Down);
        assert!(quits_on("linux", &q(Modifiers::CONTROL)));
        assert!(quits_on(
            "linux",
            &key(
                Key::Character("Q".into()),
                Modifiers::CONTROL | Modifiers::SHIFT,
                KeyState::Down
            )
        ));
        for (what, k) in [
            ("q alone", q(Modifiers::empty())),
            ("Ctrl+Alt+Q", q(Modifiers::CONTROL | Modifiers::ALT)),
            ("Ctrl+Super+Q", q(Modifiers::CONTROL | Modifiers::SUPER)),
            (
                "Ctrl+W",
                key(
                    Key::Character("w".into()),
                    Modifiers::CONTROL,
                    KeyState::Down,
                ),
            ),
            (
                "Ctrl+Q's release",
                key(Key::Character("q".into()), Modifiers::CONTROL, KeyState::Up),
            ),
        ] {
            assert!(!quits_on("linux", &k), "{what}");
        }
        for os in ["macos", "windows"] {
            assert!(!quits_on(os, &q(Modifiers::CONTROL)), "{os}: Ctrl+Q");
            assert!(!quits_on(os, &q(Modifiers::META)), "{os}: the menu's ⌘Q");
            let alt_f4 = key(Key::F4, Modifiers::ALT, KeyState::Down);
            assert!(!quits_on(os, &alt_f4), "{os}: the system's Alt+F4");
        }
    }

    /// Ctrl+Q in the standalone on Linux asks its window to close before TYPING, a value being
    /// typed or anything else in the editor has it: no note, nothing typed (R-QUIT). A plug-in's
    /// editor leaves the key to its host.
    #[cfg(all(target_os = "linux", feature = "standalone-production"))]
    #[test]
    fn ctrl_q_quits_the_standalone_before_typing_or_a_value_has_it() {
        use nih_plug::wrapper::production::app_window::quit_requested;
        let ctrl_q = KeyboardEvent {
            key: Key::Character("q".into()),
            code: keyboard_types::Code::KeyQ,
            modifiers: Modifiers::CONTROL,
            ..Default::default()
        };
        let release = KeyboardEvent {
            state: KeyState::Up,
            ..ctrl_q.clone()
        };
        let _ = quit_requested();

        let (mut e, _, _) = editing_standalone();
        e.playing.visible = true;
        e.playing.typing = true;
        assert!(e.note_typing_allowed());
        assert!(e.keyed(&ctrl_q), "TYPING: the key the editor's");
        assert!(quit_requested(), "TYPING: quitting asked for");
        e.keyed(&release);
        assert!(e.playing.notes.pop().is_none(), "TYPING: no note");

        e.start_typing(Readout::Drive);
        assert!(e.typing.is_some());
        assert!(e.keyed(&ctrl_q), "a value: the key the editor's");
        assert!(quit_requested(), "a value: quitting asked for");
        assert_eq!(
            e.typing.as_ref().map(|(_, text)| text.as_str()),
            Some(""),
            "a value: nothing typed"
        );

        for api in [PluginApi::Clap, PluginApi::Vst3] {
            let (mut e, _, _) = editing_for_api(
                Ca72Params::default(),
                Library::at(std::env::temp_dir().join("ca72-editor-tests-no-library")),
                api,
            );
            e.keyed(&ctrl_q);
            assert!(!quit_requested(), "{api}: the host's key");
        }
    }

    /// The standalone's TEMPO screen (decisions.md R-ARP; the owner's option D): it reads the
    /// tempo, and is set as the readouts are: dragged up and down, a beat a minute a notch of
    /// the wheel, a tempo typed after a double click, 30 to 300; Escape leaves it as it was. In
    /// a plug-in there is none (the host gives the tempo) and the wood does nothing.
    #[test]
    fn the_standalone_s_tempo_screen_sets_the_tempo() {
        let (mut e, _, params) = editing_standalone();
        e.update_scene();
        assert_eq!(e.scene.tempo.as_deref(), Some("120 BPM"));
        let (x, y, w, h) = art::TEMPO_SCREEN;
        let p = at(&e, (x + w / 2.0, y + h / 2.0));
        // Dragged up 20 logical pixels: ten beats a minute faster; let go, kept.
        go(&mut e, p);
        press(&mut e);
        for dy in 1..=20 {
            go(&mut e, below(p, -f64::from(dy)));
        }
        release(&mut e);
        assert_eq!(params.tempo(), 130.0);
        assert_eq!(e.settings.tempo, 130);
        // Down past the slowest: 30 (a press of its own, not a double click's second).
        e.last_tempo_press = None;
        go(&mut e, p);
        press(&mut e);
        go(&mut e, below(p, 1000.0));
        release(&mut e);
        assert_eq!(params.tempo(), 30.0);
        // The wheel: a beat a minute a notch.
        go(&mut e, p);
        wheel(&mut e, 1.0);
        wheel(&mut e, 1.0);
        assert_eq!(params.tempo(), 32.0);
        wheel(&mut e, -1.0);
        assert_eq!(params.tempo(), 31.0);
        e.update_scene();
        assert_eq!(e.scene.tempo.as_deref(), Some("31 BPM"));
        // Typed: double-clicked, 96, Enter; 999 is the fastest, 300; Escape leaves it.
        let typed = |e: &mut Editing, text: &str, end: Key| {
            e.last_tempo_press = None;
            click(e, p);
            click(e, p);
            assert_eq!(e.typing.as_ref().map(|t| t.0), Some(Typed::Tempo), "{text}");
            e.update_scene();
            assert_eq!(e.scene.tempo.as_deref(), Some("_ BPM"));
            for c in text.chars() {
                keyed(e, Key::Character(c.to_string()));
            }
            keyed(e, end);
        };
        typed(&mut e, "96", Key::Enter);
        assert_eq!((params.tempo(), e.settings.tempo), (96.0, 96));
        typed(&mut e, "999", Key::Enter);
        assert_eq!(params.tempo(), 300.0);
        typed(&mut e, "12", Key::Enter);
        assert_eq!(params.tempo(), 30.0);
        typed(&mut e, "150", Key::Escape);
        assert_eq!(params.tempo(), 30.0);
        assert!(e.typing.is_none());
        // A plug-in: no screen; the wood there does nothing.
        let (mut e, host, params) = editing();
        e.update_scene();
        assert_eq!(e.scene.tempo, None);
        let p = at(&e, (x + w / 2.0, y + h / 2.0));
        go(&mut e, p);
        press(&mut e);
        go(&mut e, below(p, -40.0));
        release(&mut e);
        wheel(&mut e, 1.0);
        assert_eq!(params.tempo(), 120.0);
        assert!(host.take().is_empty());
    }

    #[test]
    fn keyboard_type_and_panic_are_only_available_in_standalone() {
        for api in [PluginApi::Clap, PluginApi::Vst3, PluginApi::Standalone] {
            let standalone = api == PluginApi::Standalone;
            let (mut e, _, _) = editing_for_api(
                Ca72Params::default(),
                Library::at(std::env::temp_dir().join("ca72-editor-tests-no-library")),
                api,
            );
            e.playing.visible = true;
            assert_eq!(e.playing.standalone, standalone);
            e.playing.typing = true;
            assert_eq!(e.note_typing_allowed(), standalone);
            let scene = e.playing.scene(Skin::Worn).expect("keyboard");
            assert_eq!(scene.standalone, standalone);
            assert_eq!(scene.typing, standalone);
            e.playing.typing = false;
            let type_button = at(
                &e,
                art::tempo_key_at(e.renderer.arrangement(), art::TempoKey::Typing),
            );
            go(&mut e, type_button);
            press(&mut e);
            release(&mut e);
            assert_eq!(e.playing.typing, standalone, "{api}: TYPE hit target");
            let key = KeyboardEvent {
                key: Key::Character("a".into()),
                ..Default::default()
            };
            assert_eq!(e.keyed(&key), standalone, "{api}: note keys");
            if standalone {
                assert!(e.playing.notes.pop().expect("typed attack").on);
                let up = KeyboardEvent {
                    state: KeyState::Up,
                    ..key.clone()
                };
                assert!(e.keyed(&up));
                assert!(!e.playing.notes.pop().expect("typed release").on);
            } else {
                assert!(e.playing.notes.pop().is_none());
            }
            // A hidden PANIC target must not reset an existing local note.
            assert!(e.playing.key(&key, true));
            assert!(e.playing.notes.pop().expect("local attack").on);
            let generation = e.playing.notes.generation();
            let panic_button = at(
                &e,
                art::tempo_key_at(e.renderer.arrangement(), art::TempoKey::Panic),
            );
            go(&mut e, panic_button);
            press(&mut e);
            release(&mut e);
            assert_eq!(
                e.playing.notes.generation() > generation,
                standalone,
                "{api}: PANIC hit target"
            );
            e.playing.release();
        }
    }

    /// TYPING and PANIC on the walnut after the TEMPO screen (decisions.md R-TEMPO-CORNER):
    /// pressed from any lower page, the KEYS page's header no longer theirs; TYPING lit while
    /// on, PANIC while held, each lightened under the pointer; none in a plug-in.
    #[test]
    fn typing_and_panic_sit_beside_tempo_and_work_from_every_page() {
        let (mut e, _, _) = editing_for_api(
            Ca72Params::default(),
            Library::at(std::env::temp_dir().join("ca72-editor-tests-no-library")),
            PluginApi::Standalone,
        );
        e.playing.visible = true;
        // Each change drawn in places as it is drawn whole, and only there (R58).
        let shown_whole = |e: &mut Editing, what: &str| {
            e.draw();
            assert!(e.damage.is_some(), "{what}: drawn in places");
            let mut whole = None;
            compose(
                &mut whole,
                &e.renderer,
                e.strip.frame(),
                lower_of(e),
                None,
                (
                    e.page_shows().then(|| e.lfo.frame()),
                    e.macros_show()
                        .then(|| (e.macros.frame(), e.renderer.left_rect())),
                    &e.modulation,
                ),
                None,
                true,
                (e.physical_width, e.physical_height()),
            );
            assert!(e.composed == whole, "{what}: not as put together whole");
        };
        // GLOBAL's page shows, not the piano's.
        e.strip_press(StripTarget::Bar(BarTarget::Global));
        e.draw();
        e.update_scene();
        assert_eq!(
            e.scene.tempo_keys,
            Some(TempoKeys::default()),
            "the keys with the TEMPO screen"
        );
        let typing = at(
            &e,
            art::tempo_key_at(e.renderer.arrangement(), art::TempoKey::Typing),
        );
        go(&mut e, typing);
        e.update_scene();
        assert_eq!(
            e.scene.tempo_keys.map(|k| k.hover),
            Some(Some(art::TempoKey::Typing))
        );
        assert!(
            e.scene
                .tip
                .as_ref()
                .is_some_and(|t| t.0.starts_with("TYPING"))
        );
        press(&mut e);
        release(&mut e);
        // (Off TYPING, whose tip floats over everything and so is shown whole.)
        let panic = at(
            &e,
            art::tempo_key_at(e.renderer.arrangement(), art::TempoKey::Panic),
        );
        go(&mut e, panic);
        shown_whole(&mut e, "TYPING on");
        e.update_scene();
        assert!(e.playing.typing && e.note_typing_allowed());
        assert!(e.scene.tempo_keys.is_some_and(|k| k.typing && !k.panic));
        // PANIC lit while it is held, every note let go.
        let key = KeyboardEvent {
            key: Key::Character("a".into()),
            ..Default::default()
        };
        assert!(e.keyed(&key));
        assert!(e.playing.notes.pop().expect("typed attack").on);
        let generation = e.playing.notes.generation();
        press(&mut e);
        e.update_scene();
        assert!(e.scene.tempo_keys.is_some_and(|k| k.panic));
        assert!(e.playing.notes.generation() > generation, "PANIC");
        shown_whole(&mut e, "PANIC held");
        release(&mut e);
        shown_whole(&mut e, "PANIC let go");
        e.update_scene();
        assert!(e.scene.tempo_keys.is_some_and(|k| !k.panic && k.typing));
        // TYPING pressed again: off.
        go(&mut e, typing);
        press(&mut e);
        release(&mut e);
        assert!(!e.playing.typing);
        // A plug-in's walnut is bare.
        let (mut p, _, _) = editing_for_api(
            Ca72Params::default(),
            Library::at(std::env::temp_dir().join("ca72-editor-tests-no-library")),
            PluginApi::Clap,
        );
        p.update_scene();
        assert_eq!(p.scene.tempo_keys, None);
    }

    /// Where control `param` is in this editor's arrangement, in the window.
    fn control_at(e: &Editing, param: &str) -> Point {
        let i = ca72_panel::controls::index(param).expect("a control");
        at(e, e.renderer.arrangement().control(i).centre())
    }

    /// Control `param` clicked where this editor has it.
    fn click_on(e: &mut Editing, param: &str) {
        let p = control_at(e, param);
        click(e, p);
    }

    /// The learnable controls the MIDI map mode marks now.
    fn marked(e: &Editing) -> Vec<usize> {
        e.map_places().into_iter().map(|(k, _)| k).collect()
    }

    /// The standalone's MIDI map mode (decisions.md R-MIDI-MAP): MIDI between TYPING and PANIC turns it on
    /// (lit, its tip) and off; every learnable control in view is marked, its controller in small
    /// print; a click chooses a control without operating it (no value, no gesture, no drag, no
    /// wheel), MIDI Learn waits on it, the next controller is its own and the mode stays on;
    /// another click moves the waiting; Delete takes the chosen control's controller away;
    /// Escape stops the waiting, then ends the mode. The rail's page keys work in it, and their
    /// pages' controls are marked. A plug-in has no MIDI key.
    #[test]
    fn midi_map_mode_chooses_a_control_learns_it_and_stays_on() {
        let (mut e, host, params) = editing_standalone();
        let map = Arc::clone(&params.midi_map);
        let page = |e: &mut Editing, t: BarTarget| {
            let p = at(e, strip::centre(e.rail, StripTarget::Bar(t)));
            click(e, p);
            e.draw();
        };
        page(&mut e, BarTarget::Global);
        assert!(e.scene.marks.is_empty(), "off: no marks");
        assert_eq!(e.scene.tempo_keys.map(|k| k.midi), Some(false));
        // MIDI between TYPING and PANIC, the rail's gap either side (the owner: "keyboard-midi-
        // panic"); its tip after PANIC.
        let (tx, _) = art::tempo_key_at(e.renderer.arrangement(), art::TempoKey::Typing);
        let (mx, _) = art::tempo_key_at(e.renderer.arrangement(), art::TempoKey::Midi);
        let (px, _) = art::tempo_key_at(e.renderer.arrangement(), art::TempoKey::Panic);
        assert_eq!(mx - tx, art::TEMPO_KEY.0 + 14.0);
        assert_eq!(px - mx, art::TEMPO_KEY.0 + 14.0);
        assert_eq!(art::TempoKey::LAST, art::TempoKey::Panic);
        let midi = at(
            &e,
            art::tempo_key_at(e.renderer.arrangement(), art::TempoKey::Midi),
        );
        go(&mut e, midi);
        e.update_scene();
        assert_eq!(
            e.scene.tip.as_ref().map(|t| t.0.as_str()),
            Some(mapping::KEY_TIP)
        );
        click(&mut e, midi);
        assert!(e.mapping_on() && e.holds_keys());
        e.draw();
        assert_eq!(e.scene.tempo_keys.map(|k| k.midi), Some(true));
        // GLOBAL's knobs and tabs, the panel's, the preset arrows; not the wheels.
        let m = marked(&e);
        for id in [
            "cutoff",
            "osc1_on",
            "glide_on",
            "entropy",
            "poly",
            "unison",
            "doubled",
            "auto_gain",
            "previous_preset",
            "next_preset",
        ] {
            assert!(m.contains(&learnable(id)), "{id} marked");
        }
        assert!(!m.contains(&crate::learn::MOD_WHEEL));
        assert!(
            !m.contains(&learnable("lfo1_rate")),
            "the LFO page is not shown"
        );
        assert_eq!(e.scene.marks.len(), m.len());
        assert!(e.scene.marks.iter().all(|m| m.label.is_none()));
        assert!(e.renderer.floating(), "the marks float over everything");
        // Every mark is found where it is drawn.
        for (k, r) in e.map_places() {
            assert_eq!(
                e.map_at(((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0))
                    .map(|f| f.0),
                Some(k),
                "{}",
                crate::learn::LEARNABLE[k].id
            );
        }
        // A click on CUTOFF chooses it: it waits; nothing set, no gesture.
        let cutoff = learnable("cutoff");
        let before = params.cutoff.unmodulated_normalized_value();
        click_on(&mut e, "cutoff");
        assert_eq!(map.armed(), Some(cutoff));
        e.draw();
        assert_eq!(note_lines(&e)[0], "MIDI LEARN: CUTOFF FREQUENCY");
        assert_eq!(e.scene.marks.iter().filter(|m| m.waiting).count(), 1);
        // The next controller is its own; the mode stays on, CUTOFF chosen.
        assert_eq!(map.incoming(0, 74), crate::learn::Incoming::Caught);
        e.draw();
        assert_eq!(
            map.assignment(cutoff),
            Some(crate::learn::Cc { channel: 0, cc: 74 })
        );
        assert!(e.mapping_on() && map.armed().is_none());
        assert!(
            e.scene
                .marks
                .iter()
                .any(|m| m.chosen && m.label.as_deref() == Some("CC 74"))
        );
        // Another control, then another: the waiting moves. A drag and the wheel turn nothing.
        click_on(&mut e, "emphasis");
        assert_eq!(map.armed(), Some(learnable("emphasis")));
        let glide = control_at(&e, "glide");
        let glide_was = params.glide.unmodulated_normalized_value();
        go(&mut e, glide);
        press(&mut e);
        go(&mut e, below(glide, -120.0));
        release(&mut e);
        wheel(&mut e, 4.0);
        assert_eq!(map.armed(), Some(learnable("glide")));
        assert_eq!(params.glide.unmodulated_normalized_value(), glide_was);
        // A switch and a tab are chosen, not flipped.
        let poly = params.poly.value();
        let tab = at(&e, strip::centre(e.rail, StripTarget::Tab(Bank::Mode, 1)));
        click(&mut e, tab);
        assert_eq!(map.armed(), Some(learnable("poly")));
        assert_eq!(params.poly.value(), poly);
        let osc1 = params.osc1_on.value();
        click_on(&mut e, "osc1_on");
        assert_eq!(params.osc1_on.value(), osc1);
        // A switch waiting takes a note (not played: lib.rs), shown by its number.
        assert_eq!(
            map.incoming_note(0, 36, true),
            crate::learn::Incoming::Caught
        );
        e.draw();
        assert_eq!(
            map.binding(learnable("osc1_on")),
            Some(crate::learn::Binding::Note {
                channel: 0,
                note: 36
            })
        );
        assert!(
            e.scene
                .marks
                .iter()
                .any(|m| m.label.as_deref() == Some("NOTE 36"))
        );
        assert_eq!(params.cutoff.unmodulated_normalized_value(), before);
        assert!(host.take().is_empty(), "no gesture");
        // Delete: CUTOFF chosen again (waiting), its controller taken away.
        click_on(&mut e, "cutoff");
        assert!(keyed(&mut e, Key::Delete));
        assert_eq!(map.assignment(cutoff), None);
        // The LFO page by its key, in the mode: its knobs marked and chosen.
        page(&mut e, BarTarget::Lfo);
        assert!(e.page_shows() && e.mapping_on());
        assert!(marked(&e).contains(&learnable("lfo1_rate")));
        assert!(!marked(&e).contains(&learnable("entropy")));
        click_on(&mut e, "lfo2_rate");
        assert_eq!(map.armed(), Some(learnable("lfo2_rate")));
        // The macros by MACRO.
        page(&mut e, BarTarget::Macro);
        assert!(e.macros_show());
        assert!(marked(&e).contains(&learnable("macro1")));
        click_on(&mut e, "macro3");
        assert_eq!(map.armed(), Some(learnable("macro3")));
        // Escape: the waiting stops, the mode stays; Escape again: off.
        assert!(keyed(&mut e, Key::Escape));
        assert_eq!(map.armed(), None);
        assert!(e.mapping_on());
        assert!(keyed(&mut e, Key::Escape));
        assert!(!e.mapping_on());
        e.draw();
        assert!(e.scene.marks.is_empty());
        assert_eq!(e.scene.tempo_keys.map(|k| k.midi), Some(false));
        // MIDI on, a control waiting, MIDI off: the waiting stops too. Off, a click operates.
        click(&mut e, midi);
        click_on(&mut e, "lfo1_rate");
        assert!(map.armed().is_some());
        click(&mut e, midi);
        assert!(!e.mapping_on() && map.armed().is_none());
        assert_eq!(
            map.assignment(learnable("osc1_on")),
            None,
            "a note, not a CC"
        );
        assert!(map.binding(learnable("osc1_on")).is_some(), "kept");
        // A plug-in's walnut is bare: no MIDI key, no mode.
        let (mut p, _, _) = editing();
        let at_key = at(
            &p,
            art::tempo_key_at(p.renderer.arrangement(), art::TempoKey::Midi),
        );
        click(&mut p, at_key);
        p.draw();
        assert!(!p.mapping_on());
        assert!(p.scene.tempo_keys.is_none() && p.scene.marks.is_empty());
    }

    /// The arpeggiator's drawer in the mode: its knobs and its ON, HOLD and DIRECTION (until
    /// now learned only from the list) are marked and chosen, ON's note over its tab; the
    /// controls under the drawer are not marked, those below it are.
    #[test]
    fn midi_map_mode_reaches_the_arpeggiators_drawer() {
        let (mut e, _host, params) = editing_standalone();
        let map = Arc::clone(&params.midi_map);
        let page = at(
            &e,
            strip::centre(e.rail, StripTarget::Bar(BarTarget::Global)),
        );
        click(&mut e, page);
        e.toggle_mapping();
        e.draw();
        assert!(marked(&e).contains(&learnable("poly")));
        let arp = at(&e, strip::centre(e.rail, StripTarget::Bar(BarTarget::Arp)));
        click(&mut e, arp);
        assert!(e.lower.open, "the ARP key works in the mode");
        e.lower.slide = Some((Instant::now() - crate::presets::SLIDE * 2, true));
        e.draw();
        e.draw();
        let m = marked(&e);
        for id in [
            "arp_on",
            "arp_latch",
            "arp_direction",
            "arp_rate",
            "arp_gate",
        ] {
            assert!(m.contains(&learnable(id)), "{id}");
        }
        assert!(
            !m.contains(&learnable("poly")),
            "MODE's tabs under the drawer"
        );
        assert!(m.contains(&learnable("entropy")), "ENTROPY below it");
        let on = at(&e, ArpBank::Arp.tab(0));
        click(&mut e, on);
        assert_eq!(map.armed(), Some(learnable("arp_on")));
        assert!(!params.arp_on.value());
        e.draw();
        let n = e.scene.note.as_ref().expect("a note over ON");
        assert_eq!(n.lines[0].0, "MIDI LEARN: ARP ON");
        assert!(n.lines[1].0.contains("PRESS A NOTE"));
        assert!((n.x - ArpBank::Arp.tab(0).0).abs() < 1.0);
    }

    /// The MIDI map mode for looking at (decisions.md R-MIDI-MAP), written to
    /// `$CA72_MIDI_MAP_PNG`, the folder, at `$CA72_MIDI_MAP_DPR` (2 by default) the window's
    /// 1720 logical pixels: the standalone with the mode off (`1-off.png`), on over GLOBAL's page
    /// with some controls learned (`2-on-global.png`), a control waiting (`3-waiting.png`), the
    /// LFO page and the macros (`4-lfo-macros.png`), the arpeggiator's drawer
    /// (`4b-arp.png`), and a plug-in's bare walnut (`5-plugin.png`); and the walnut's keys close
    /// up, off and on (`0-keys-off.png`, `0-keys-on.png`).
    #[test]
    #[ignore = "writes images for a look"]
    fn the_midi_map_pngs() {
        let Some(out) = std::env::var_os("CA72_MIDI_MAP_PNG").map(std::path::PathBuf::from) else {
            return;
        };
        std::fs::create_dir_all(&out).unwrap();
        let dpr: f64 = std::env::var("CA72_MIDI_MAP_DPR")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2.0);
        let width = 1720;
        let sized = |e: &mut Editing| {
            e.fit(dpr, width, (f64::from(width) * dpr).round() as u32);
            e.draw();
        };
        let away = |e: &mut Editing| {
            go(e, Point { x: 2.0, y: 2.0 });
        };
        let save = |e: &mut Editing, name: &str| {
            e.composed = None;
            window_frame(e).save_png(out.join(name)).unwrap();
        };
        // The walnut's keys close up: the TEMPO screen to past MIDI's tip.
        let keys = |e: &mut Editing, name: &str| {
            e.composed = None;
            let s = e.renderer.scale();
            let top = e.renderer.top();
            let (x, y, _, h) = art::tempo_window(e.renderer.arrangement().plate_at());
            let (mx, _) = art::tempo_key_at(e.renderer.arrangement(), art::TempoKey::Midi);
            let f = window_frame(e);
            let x0 = ((x - 40.0) * s) as u32;
            let x1 = (((mx + 900.0) * s) as u32).min(f.width());
            let y0 = ((y - 60.0 - top) * s).max(0.0) as u32;
            let y1 = (((y + h + 60.0 - top) * s) as u32).min(f.height());
            let (w, hh) = (x1 - x0, y1 - y0);
            let mut c = ca72_panel::Pixmap::new(w, hh).unwrap();
            for row in 0..hh {
                let from = ((y0 + row) * f.width() + x0) as usize * 4;
                let to = (row * w) as usize * 4;
                c.data_mut()[to..to + w as usize * 4]
                    .copy_from_slice(&f.data()[from..from + w as usize * 4]);
            }
            c.save_png(out.join(name)).unwrap();
        };
        let (mut e, _, params) = editing_standalone();
        let map = Arc::clone(&params.midi_map);
        sized(&mut e);
        let global = at(
            &e,
            strip::centre(e.rail, StripTarget::Bar(BarTarget::Global)),
        );
        click(&mut e, global);
        away(&mut e);
        save(&mut e, "1-off.png");
        keys(&mut e, "0-keys-off.png");
        // On, some controls learned: knobs' controllers, a switch's note, a tab's.
        let cc = |channel, cc| crate::learn::Cc { channel, cc };
        map.assign(learnable("cutoff"), cc(0, 74));
        map.assign(learnable("emphasis"), cc(0, 71));
        map.assign(learnable("loudness_decay"), cc(0, 75));
        map.assign(learnable("entropy"), cc(1, 20));
        map.assign(
            learnable("osc3_on"),
            crate::learn::Binding::Note {
                channel: 0,
                note: 36,
            },
        );
        map.assign(learnable("poly"), cc(0, 80));
        map.assign(learnable("next_preset"), cc(0, 91));
        let midi = at(
            &e,
            art::tempo_key_at(e.renderer.arrangement(), art::TempoKey::Midi),
        );
        go(&mut e, midi);
        e.update_scene();
        keys(&mut e, "0-keys-on-hover.png");
        click(&mut e, midi);
        away(&mut e);
        save(&mut e, "2-on-global.png");
        keys(&mut e, "0-keys-on.png");
        // A control waiting.
        click_on(&mut e, "contour_amount");
        away(&mut e);
        save(&mut e, "3-waiting.png");
        // The LFO page and the macros, some learned; LFO 2's RATE waiting.
        map.assign(learnable("lfo1_rate"), cc(0, 21));
        map.assign(learnable("lfo1_phase"), cc(0, 22));
        map.assign(learnable("macro1"), cc(0, 16));
        map.assign(learnable("macro2"), cc(0, 17));
        for t in [BarTarget::Lfo, BarTarget::Macro] {
            let p = at(&e, strip::centre(e.rail, StripTarget::Bar(t)));
            click(&mut e, p);
        }
        click_on(&mut e, "lfo2_rate");
        away(&mut e);
        save(&mut e, "4-lfo-macros.png");
        // The arpeggiator's drawer over GLOBAL's page, its ON waiting for a note.
        for t in [BarTarget::Macro, BarTarget::Global, BarTarget::Arp] {
            let p = at(&e, strip::centre(e.rail, StripTarget::Bar(t)));
            click(&mut e, p);
        }
        e.lower.slide = Some((Instant::now() - crate::presets::SLIDE * 2, true));
        e.draw();
        map.assign(
            learnable("arp_latch"),
            crate::learn::Binding::Note {
                channel: 0,
                note: 37,
            },
        );
        let on = at(&e, ArpBank::Arp.tab(0));
        click(&mut e, on);
        away(&mut e);
        save(&mut e, "4b-arp.png");
        // A plug-in's walnut: bare.
        let (mut p, _, _) = editing();
        sized(&mut p);
        away(&mut p);
        save(&mut p, "5-plugin.png");
        keys(&mut p, "0-keys-plugin.png");
    }

    /// The audio threads' standings the tests give the editor (decisions.md R-RTKIT): none
    /// asked, one waiting for RTKit, both refused with RTKit absent, the audio thread refused by
    /// RTKit with the workers real-time, two of six workers refused by RTKit, every one through
    /// RTKit.
    #[cfg(target_os = "linux")]
    mod stood {
        use ca72_rt::{Standing, Why};
        const fn s(
            direct: u32,
            rtkit: u32,
            waiting: u32,
            ordinary: u32,
            why: Option<Why>,
        ) -> Standing {
            Standing {
                direct,
                rtkit,
                waiting,
                ordinary,
                why,
            }
        }
        pub fn asking() -> (Standing, Standing) {
            (s(0, 0, 1, 0, None), s(0, 0, 6, 0, None))
        }
        pub fn no_rtkit() -> (Standing, Standing) {
            (
                s(0, 0, 0, 1, Some(Why::NoRtkit)),
                s(0, 0, 0, 6, Some(Why::NoRtkit)),
            )
        }
        pub fn audio_refused() -> (Standing, Standing) {
            (s(0, 0, 0, 1, Some(Why::RtkitRefused)), s(6, 0, 0, 0, None))
        }
        pub fn workers_partly() -> (Standing, Standing) {
            (s(0, 1, 0, 0, None), s(0, 4, 0, 2, Some(Why::RtkitRefused)))
        }
        pub fn through_rtkit() -> (Standing, Standing) {
            (s(0, 1, 0, 0, None), s(0, 6, 0, 0, None))
        }
    }

    /// The standalone's audio not real-time (decisions.md R-RTKIT): said once on the walnut
    /// after its keys, for a while, while nothing else's tip is; nothing while RTKit has yet to
    /// answer or every thread is real-time; never in a plug-in.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_standalone_says_once_that_its_audio_is_not_real_time() {
        let (mut e, _, _) = editing_standalone();
        e.fit(1.0, 1720, 1720);
        go(&mut e, Point { x: 2.0, y: 2.0 });
        e.realtime.read = stood::asking;
        e.update_scene();
        assert!(e.tip().is_none(), "said before RTKit answered");
        e.realtime.read = stood::through_rtkit;
        e.update_scene();
        assert!(e.tip().is_none() && e.realtime.warning.is_none());
        e.realtime.read = stood::no_rtkit;
        e.update_scene();
        let (t, x, _) = e.tip().expect("the walnut's notice");
        assert_eq!(
            t,
            "AUDIO NOT REAL-TIME: MAY DROP OUT · INSTALL RTKIT OR JOIN THE REALTIME OR AUDIO GROUP, THEN LOG IN AGAIN"
        );
        let (px, _) = art::tempo_key_at(e.renderer.arrangement(), art::TempoKey::LAST);
        assert!(x > px + art::TEMPO_KEY.0 / 2.0, "after the walnut's keys");
        assert_eq!(e.scene.tip.as_ref().map(|t| &t.0), Some(&t));
        // A control under the pointer has its own tip meanwhile.
        let cutoff = at(&e, centre("cutoff"));
        go(&mut e, cutoff);
        e.update_scene();
        assert!(e.tip().expect("CUTOFF's tip").0.starts_with("CUTOFF"));
        go(&mut e, Point { x: 2.0, y: 2.0 });
        e.update_scene();
        assert!(e.tip().is_some(), "the notice back while its while lasts");
        // Once: past its while, not again; the warning itself stays (SETTINGS, AUDIO & MIDI).
        e.realtime.since = Some(Instant::now() - REALTIME_NOTICE);
        e.update_scene();
        assert!(e.tip().is_none());
        e.realtime.read = stood::workers_partly;
        e.update_scene();
        assert!(e.tip().is_none(), "said again");
        let w = e.realtime.warning.as_ref().expect("the warning");
        assert!(w.tip.starts_with("POLY'S THREADS"), "{}", w.tip);
        assert_eq!(w.row, "NOT REAL-TIME: SEE AUDIO & MIDI…");
        // A plug-in's: never.
        let (mut p, _, _) = editing();
        p.realtime.read = stood::no_rtkit;
        p.update_scene();
        assert!(p.realtime.warning.is_none() && p.tip().is_none());
    }

    /// The standalone's audio not real-time (decisions.md R-RTKIT) for looking at, written to
    /// `$CA72_RTKIT_PNG`, the folder, at `$CA72_RTKIT_DPR` (2 by default) the window's 1720
    /// logical pixels: the window with the walnut's notice (`1-window.png`); the walnut close up
    /// for each case (`2-walnut-<case>.png`: both refused with RTKit absent, the audio thread
    /// refused by RTKit, two workers of six refused) and in the classic look and LARGE lettering
    /// (`3-walnut-<look>.png`); SETTINGS opened, its STANDALONE at the foot (`4-settings.png`);
    /// AUDIO & MIDI with the warning (`5-audio-midi.png`) and real-time through RTKit
    /// (`5-audio-midi-rtkit.png`).
    #[cfg(all(target_os = "linux", feature = "standalone-production"))]
    #[test]
    #[ignore = "writes images for a look"]
    fn the_realtime_warning_pngs() {
        let Some(out) = std::env::var_os("CA72_RTKIT_PNG").map(std::path::PathBuf::from) else {
            return;
        };
        std::fs::create_dir_all(&out).unwrap();
        let dpr: f64 = std::env::var("CA72_RTKIT_DPR")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2.0);
        let width = 1720;
        let sized = |e: &mut Editing| {
            e.fit(dpr, width, (f64::from(width) * dpr).round() as u32);
            e.draw();
        };
        let away = |e: &mut Editing| go(e, Point { x: 2.0, y: 2.0 });
        let save = |e: &mut Editing, name: &str| {
            e.composed = None;
            window_frame(e).save_png(out.join(name)).unwrap();
        };
        // The top walnut, the window's width, from its top to the panel.
        let walnut = |e: &mut Editing, name: &str| {
            e.composed = None;
            let s = e.renderer.scale();
            let top = e.renderer.top();
            let (_, y, _, h) = art::tempo_window(e.renderer.arrangement().plate_at());
            let f = window_frame(e);
            let y1 = (((y + h + 60.0 - top) * s) as u32).min(f.height());
            let w = f.width();
            let mut c = ca72_panel::Pixmap::new(w, y1).unwrap();
            let n = (w * y1) as usize * 4;
            c.data_mut().copy_from_slice(&f.data()[..n]);
            c.save_png(out.join(name)).unwrap();
        };
        let fresh = |read: fn() -> (ca72_rt::Standing, ca72_rt::Standing)| {
            let (mut e, _, _) = editing_standalone();
            e.realtime.read = read;
            sized(&mut e);
            away(&mut e);
            e.update_scene();
            e
        };
        let mut e = fresh(stood::no_rtkit);
        save(&mut e, "1-window.png");
        walnut(&mut e, "2-walnut-no-rtkit.png");
        let mut e = fresh(stood::audio_refused);
        walnut(&mut e, "2-walnut-audio-rtkit-refused.png");
        let mut e = fresh(stood::workers_partly);
        walnut(&mut e, "2-walnut-workers-partly.png");
        for (name, skin, lettering) in [
            ("classic", Skin::Drawn, Lettering::Standard),
            ("large", Skin::Worn, Lettering::Large),
        ] {
            let mut e = fresh(stood::no_rtkit);
            e.set_look(skin);
            e.set_lettering(lettering);
            sized(&mut e);
            away(&mut e);
            e.update_scene();
            walnut(&mut e, &format!("3-walnut-{name}.png"));
        }
        // SETTINGS and AUDIO & MIDI need the standalone's host: one without devices.
        let directory = tempfile::tempdir().expect("a state folder");
        let mut runtime = plugin_kit_standalone::spawn(
            Box::new(NoAudio),
            None,
            plugin_kit_standalone::AppOptions {
                app_id: "ca72-rtkit-review".into(),
                state_dir: directory.path().to_owned(),
                device_access: false,
            },
        )
        .expect("a host without devices");
        runtime
            .wait_ready(Duration::from_secs(2))
            .expect("the host ready");
        plugin_kit_standalone::AppHandle::install_current(&runtime.handle());
        let mut e = fresh(stood::no_rtkit);
        e.realtime.since = Some(Instant::now() - REALTIME_NOTICE);
        let key = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Settings)));
        click(&mut e, key);
        assert!(
            menu_items(&e)
                .iter()
                .any(|(t, on)| t == "NOT REAL-TIME: SEE AUDIO & MIDI…" && !on),
            "{:?}",
            menu_items(&e)
        );
        // (Its foot in view: the menu is taller than the window.)
        let m = e.learning.menu().expect("the menu").clone();
        let foot = at(&e, m.item_at(m.items.len() - 1));
        go(&mut e, foot);
        for _ in 0..40 {
            wheel(&mut e, -3.0);
        }
        save(&mut e, "4-settings.png");
        for (name, read) in [
            ("5-audio-midi.png", stood::no_rtkit as fn() -> _),
            ("5-audio-midi-rtkit.png", stood::through_rtkit),
        ] {
            let mut e = fresh(read);
            crate::standalone_settings::request_open();
            e.draw();
            e.standalone_dialog
                .as_mut()
                .expect("AUDIO & MIDI open")
                .review(read());
            save(&mut e, name);
        }
        let _ = runtime.shutdown();
    }

    /// A processor that plays nothing, for a standalone host without devices.
    #[cfg(all(target_os = "linux", feature = "standalone-production"))]
    struct NoAudio;

    #[cfg(all(target_os = "linux", feature = "standalone-production"))]
    impl plugin_kit_standalone::AudioProcessor for NoAudio {
        fn prepare(&mut self, _: plugin_kit_standalone::ProcessConfig) -> Result<(), String> {
            Ok(())
        }
        fn process(&mut self, _: plugin_kit_standalone::AudioBlock<'_>) {}
        fn deactivate(&mut self) {}
    }

    fn editing_for_api(
        params: Ca72Params,
        library: Library,
        api: PluginApi,
    ) -> (Editing, Arc<Host>, Arc<Ca72Params>) {
        let host = Arc::new(Host {
            api: Some(api),
            ..Default::default()
        });
        let params = Arc::new(params);
        let context: Arc<dyn GuiContext> = host.clone();
        let e = Editing::new(
            context,
            Arc::clone(&params),
            Arc::new(Meters::default()),
            (1720, 1720, 1.0),
            library,
            |_| None,
        );
        (e, host, params)
    }

    /// The window's pixels as the frames were turned a pixel at a time, before (the
    /// reference for [`window::fill`]).
    fn filled_a_pixel_at_a_time(
        buffer: &mut [u32],
        (w, h): (usize, usize),
        frames: &[&ca72_panel::Pixmap],
    ) {
        let mut frames = frames.iter();
        let mut frame = frames.next();
        let mut top = 0usize;
        for y in 0..h {
            while let Some(f) = frame
                && y >= top + f.height() as usize
            {
                top += f.height() as usize;
                frame = frames.next();
            }
            let row = &mut buffer[y * w..(y + 1) * w];
            for (x, out) in row.iter_mut().enumerate() {
                *out = match frame {
                    Some(f) if x < f.width() as usize => {
                        let (fw, fy) = (f.width() as usize, y - top);
                        window::shown(&f.data()[(fy * fw + x) * 4..(fy * fw + x) * 4 + 4])
                    }
                    _ => window::EMPTY,
                };
            }
        }
    }

    /// The window's pixels, a row at a time, are the pixels turned one at a time, to the bit:
    /// frames narrower than the window and shorter, one under the other, the window beyond.
    #[test]
    fn the_window_s_pixels_a_row_at_a_time_are_its_pixels_one_at_a_time() {
        let frame = |w: u32, h: u32, k: u8| {
            let mut p = ca72_panel::Pixmap::new(w, h).unwrap();
            for (i, px) in p.data_mut().chunks_exact_mut(4).enumerate() {
                px.copy_from_slice(&[(i as u8).wrapping_mul(k), k, (i / 7) as u8, 255]);
            }
            p
        };
        let (a, b) = (frame(37, 11, 3), frame(29, 6, 5));
        let (w, h) = (41, 20);
        let (mut new, mut old) = (vec![0u32; w * h], vec![0u32; w * h]);
        window::fill(&mut new, (w, h), &[&a, &b]);
        filled_a_pixel_at_a_time(&mut old, (w, h), &[&a, &b]);
        assert_eq!(new, old);
    }

    /// What turning a Retina window's frame into the screen's pixels costs, a row at a time and
    /// a pixel at a time: printed (`-- --ignored --nocapture`).
    #[test]
    #[ignore = "prints timings"]
    fn the_window_s_pixels_timings() {
        let (w, h) = (3440usize, 1965usize);
        let f = ca72_panel::Pixmap::new(w as u32, h as u32).unwrap();
        let mut buffer = vec![0u32; w * h];
        for (name, way) in [
            (
                "a row at a time",
                window::fill as fn(&mut [u32], (usize, usize), &[&ca72_panel::Pixmap]),
            ),
            ("a pixel at a time", filled_a_pixel_at_a_time),
        ] {
            let t = Instant::now();
            for _ in 0..50 {
                way(&mut buffer, (w, h), &[&f]);
            }
            println!(
                "{name}: {:.2} ms a frame",
                t.elapsed().as_secs_f64() * 1000.0 / 50.0
            );
        }
    }

    /// The window played for a look and a measure (decisions.md R48): 24 seconds in real
    /// time at sixty frames a second, a Retina window (3440 pixels across): four POLY voices
    /// with DOUBLE playing chords at HI, then ULTRA, then LO, falling quiet, then HI again with
    /// DETUNE swept from none to all of it. Each frame drawn and turned into the screen's
    /// pixels as `present` does, timed: the whole frame, as on macOS, and the places changed
    /// alone, as on Windows and X11 (decisions.md R58), the screen so made the same as
    /// the whole frame's at every frame. Every other frame's QUALITY column and STEREO display
    /// kept, written to `$CA72_RUN`, the folder, as `frames.rgba` (`size.txt`: its width and
    /// height), with the times in `times.txt` (`-- --ignored --nocapture`). With
    /// `CA72_NO_FRAMES` set, the times alone (the frames are 2 GB, in memory and on disk).
    #[test]
    #[ignore = "plays the window for a while"]
    fn the_window_played() {
        let Some(out) = std::env::var_os("CA72_RUN").map(std::path::PathBuf::from) else {
            return;
        };
        std::fs::create_dir_all(&out).unwrap();
        let keep_frames = std::env::var_os("CA72_NO_FRAMES").is_none();
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(Host::default());
        let params = Arc::new(Ca72Params::default());
        let context: Arc<dyn GuiContext> = host.clone();
        let meters = Arc::new(Meters::default());
        let mut e = Editing::new(
            context,
            Arc::clone(&params),
            Arc::clone(&meters),
            (1720, 3440, 2.0),
            Library::at(dir.path()),
            |_| None,
        );
        // (Set as a host's automation sets them: nih-plug's context for tests.)
        let hostless = nih_plug::context::process::TestProcessContext::<crate::Ca72>::new(
            Arc::clone(&params) as Arc<dyn Params>,
            48_000.0,
            ProcessMode::Realtime,
        );
        let set = |p: ParamPtr, v: f32| {
            hostless.automate(p, v);
        };
        set(params.poly.as_ptr(), 1.0);
        set(params.voices.as_ptr(), 0.25);
        set(params.spread.as_ptr(), 0.8);
        set(params.double.as_ptr(), 0.6);
        set(params.quality.as_ptr(), 0.5);
        let s = 3440.0 / art::W;
        // The two parts kept: QUALITY's column, and STEREO's knobs over its display.
        let column = (
            (art::COL + 2925.0) * s,
            (art::TOP - 20.0) * s,
            183.0 * s,
            400.0 * s,
        );
        let (dx, dy, dw, dh) = strip::DISPLAY;
        let stereo = (
            (dx - 10.0) * s,
            (strip::ROWS[1] - 75.0) * s,
            (dw + 20.0) * s,
            (dy + dh + 10.0 - strip::ROWS[1] + 75.0) * s,
        );
        let px = |r: (f64, f64, f64, f64)| (r.0 as usize, r.1 as usize, r.2 as usize, r.3 as usize);
        let (column, stereo) = (px(column), px(stereo));
        let (fw, fh) = (column.2 + 12 + stereo.2, column.3.max(stereo.3));
        let mut frames: Vec<u8> = Vec::new();
        let (mut screen, mut kept): (Vec<u32>, Vec<u32>) = (Vec::new(), Vec::new());
        // A voice's level, `t` seconds into a chord it plays: a quick attack, a decay to its
        // sustain, held 1.6 beats, released.
        let beat = 60.0 / 112.0;
        let env = |t: f64| {
            let held = 1.6 * beat;
            let a = |t: f64| (t / 0.01).min(1.0) * (0.6 + 0.4 * (-t / 0.3).exp());
            if t < 0.0 {
                0.0
            } else if t < held {
                a(t)
            } else {
                a(held) * (-(t - held) / 0.25).exp()
            }
        };
        let t0 = Instant::now();
        // The time this thread has worked, ms: on Linux its own, the computer's other work left
        // out (with builds running alongside, the wall's clock varied a third from run to run);
        // elsewhere the wall's.
        let cpu = || -> f64 {
            #[cfg(target_os = "linux")]
            if let Some(ns) = std::fs::read_to_string("/proc/thread-self/schedstat")
                .ok()
                .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
            {
                return ns / 1e6;
            }
            t0.elapsed().as_secs_f64() * 1000.0
        };
        let ticks = 24 * 60;
        // Each frame's work: drawn and turned whole, drawn and turned in places.
        let (mut whole, mut places) = (Vec::new(), Vec::new());
        let mut presented = 0;
        for k in 0..ticks {
            let at = t0 + Duration::from_micros(k * 16_667);
            if let Some(wait) = at.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            }
            let t = k as f64 / 60.0;
            let quality = if t < 6.0 {
                0.5
            } else if t < 12.0 {
                1.0
            } else if t < 18.0 {
                0.0
            } else {
                0.5
            };
            set(params.quality.as_ptr(), quality);
            if t >= 18.0 {
                set(
                    params.double.as_ptr(),
                    ((t - 18.5) / 5.0).clamp(0.0, 1.0) as f32,
                );
            }
            // Chords every two beats (quiet from 15 to 18 seconds), each voice its own
            // velocity.
            let playing = !(15.0..18.0).contains(&t);
            let chord = (t / (2.0 * beat)).floor();
            let since = t - chord * 2.0 * beat;
            let mut levels = [0.0f32; LEVELS];
            let mut mask = 0u32;
            for (v, level) in levels.iter_mut().enumerate().take(4) {
                let vel = 0.55 + 0.45 * (((chord as usize + v) * 7) % 5) as f64 / 4.0;
                let l = if playing {
                    0.35 * vel * env(since - 0.02 * v as f64)
                } else {
                    0.0
                };
                *level = l as f32;
                if l > 1e-3 {
                    mask |= 1 << v;
                }
            }
            let output = (levels.iter().map(|&l| f64::from(l)).sum::<f64>() * 0.7).min(1.0);
            meters.publish(0.0, (0.5, 0.0), mask);
            meters.publish_levels(&levels, output as f32);
            let started = cpu();
            let changed = e.draw();
            let drawn = cpu() - started;
            let (mut turned, mut in_places) = (0.0, 0.0);
            if changed {
                let f = e.composed.as_ref().unwrap_or_else(|| e.renderer.frame());
                let size = (f.width() as usize, f.height() as usize);
                // (As `present` turns it: the window's own code.)
                let fresh = screen.len() != size.0 * size.1;
                screen.resize(size.0 * size.1, 0);
                kept.resize(size.0 * size.1, 0);
                let started = cpu();
                window::fill(&mut screen, size, &[f]);
                turned = cpu() - started;
                let started = cpu();
                match e.damage.as_deref().filter(|_| !fresh) {
                    Some(d) => {
                        window::fill_places(&mut kept, size, &[f], d);
                    }
                    None => window::fill(&mut kept, size, &[f]),
                }
                in_places = cpu() - started;
                assert!(kept == screen, "the screen turned in places at frame {k}");
                presented += 1;
            }
            whole.push(drawn + turned);
            places.push(drawn + in_places);
            if keep_frames && k % 2 == 0 {
                let f = e.composed.as_ref().unwrap_or_else(|| e.renderer.frame());
                let mut img = vec![0u8; fw * fh * 4];
                for (r, x0) in [(column, 0usize), (stereo, column.2 + 12)] {
                    for y in 0..r.3 {
                        let from = ((r.1 + y) * f.width() as usize + r.0) * 4;
                        let to = (y * fw + x0) * 4;
                        img[to..to + r.2 * 4].copy_from_slice(&f.data()[from..from + r.2 * 4]);
                    }
                }
                frames.extend_from_slice(&img);
            }
        }
        if keep_frames {
            std::fs::write(out.join("frames.rgba"), &frames).unwrap();
            std::fs::write(out.join("size.txt"), format!("{fw} {fh}")).unwrap();
        }
        let said = |times: &[f64]| {
            let total: f64 = times.iter().sum();
            let mean = total / times.len() as f64;
            let mut sorted = times.to_vec();
            sorted.sort_by(f64::total_cmp);
            let p95 = sorted[sorted.len() * 95 / 100];
            format!(
                "{total:.0} ms ({:.1} % of one core), {mean:.2} ms a frame on average, {p95:.2} at the 95th percentile",
                total / 24_000.0 * 100.0
            )
        };
        let report = format!(
            "{ticks} frames in 24 s: {presented} presented; the UI thread's work, each frame turned whole (macOS): {}; turned in the places changed (Windows, X11): {}\n",
            said(&whole),
            said(&places)
        );
        print!("{report}");
        std::fs::write(out.join("times.txt"), report).unwrap();
    }

    /// QUALITY's toggle (decisions.md R47, R48) goes where it is clicked, a gesture of
    /// its own as a rocker's: above its nut ULTRA, below it LO, on it (HI already) nothing.
    #[test]
    fn quality_goes_where_its_toggle_is_clicked() {
        let (mut e, host, params) = editing_at_hi();
        let q = params.quality.as_ptr();
        let (x, y) = centre("quality");
        let up = at(&e, (x, y - 40.0));
        click(&mut e, up);
        assert_eq!(
            host.take(),
            vec![Call::Begin(q), Call::Set(q, 1.0), Call::End(q)]
        );
        let down = at(&e, (x, y + 45.0));
        click(&mut e, down);
        assert_eq!(
            host.take(),
            vec![Call::Begin(q), Call::Set(q, 0.0), Call::End(q)]
        );
        let middle = at(&e, (x, y));
        click(&mut e, middle);
        assert_eq!(host.take(), vec![]);
    }

    /// A host's key (Cubase gives a VST3 plug-in its keys through `onKeyDown()`: decisions.md
    /// R53) as the window's own: VST3's codes named, else the character, the keypad's digits
    /// without one too, the modifiers; none for a key the editor has no use for.
    #[test]
    fn a_hosts_key_is_read_as_the_windows_own() {
        let key = |character, key_code, modifiers| HostKey {
            character,
            key_code,
            modifiers,
            down: true,
        };
        let read = |k| host_key_event(k).map(|e| (e.key, e.modifiers));
        assert_eq!(
            read(key(Some('8'), 0, 0)),
            Some((Key::Character("8".into()), Modifiers::empty()))
        );
        assert_eq!(
            read(key(Some('A'), 0, 1)),
            Some((Key::Character("A".into()), Modifiers::SHIFT))
        );
        assert_eq!(read(key(Some('\r'), 4, 0)).map(|r| r.0), Some(Key::Enter));
        assert_eq!(read(key(None, 19, 0)).map(|r| r.0), Some(Key::Enter));
        assert_eq!(
            read(key(Some('\u{1b}'), 6, 0)).map(|r| r.0),
            Some(Key::Escape)
        );
        assert_eq!(
            read(key(Some('\u{8}'), 1, 0)).map(|r| r.0),
            Some(Key::Backspace)
        );
        assert_eq!(read(key(None, 14, 0)).map(|r| r.0), Some(Key::ArrowDown));
        assert_eq!(
            read(key(None, 29, 0)).map(|r| r.0),
            Some(Key::Character("5".into()))
        );
        assert_eq!(
            read(key(None, 38, 0)).map(|r| r.0),
            Some(Key::Character(".".into()))
        );
        assert_eq!(read(key(None, 44, 0)), None, "F5");
        let up = host_key_event(HostKey {
            down: false,
            ..key(Some('8'), 0, 0)
        })
        .expect("a key");
        assert_eq!(up.state, KeyState::Up);
    }

    /// The host's keys are the editor's only while it holds the keyboard (a value typed, the
    /// drawer, MIDI Learn): then every key is taken, so that none is the host's key command
    /// meanwhile, and those it has a use for are kept for the window; else none is taken.
    #[test]
    fn the_hosts_keys_are_taken_only_while_the_editor_holds_the_keyboard() {
        let keys = HostKeys::default();
        let k = |character, key_code| HostKey {
            character,
            key_code,
            modifiers: 0,
            down: true,
        };
        assert!(!keys.offer(k(Some('8'), 0)));
        assert!(keys.take().is_empty());
        keys.holding.store(true, Ordering::Release);
        assert!(keys.offer(k(Some('8'), 0)));
        assert!(keys.offer(k(None, 44)), "F5: taken, kept for nothing");
        assert!(keys.offer(k(Some('\r'), 4)));
        let taken: Vec<Key> = keys.take().into_iter().map(|e| e.key).collect();
        assert_eq!(taken, vec![Key::Character("8".into()), Key::Enter]);
        assert!(keys.take().is_empty());
    }

    /// A key a host gives both ways (the view's and the window's own) is taken once, whichever
    /// comes first; two presses of one key from one way are two; a twin later than [`TWIN`] is a
    /// key of its own.
    #[test]
    fn a_key_given_both_ways_is_taken_once() {
        let mut twins = KeyTwins::default();
        let ev = |c: &str| KeyboardEvent {
            state: KeyState::Down,
            key: Key::Character(c.into()),
            ..KeyboardEvent::default()
        };
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        assert!(twins.own_new(&ev("8"), at(0)));
        assert!(!twins.host_new(&ev("8"), at(10)));
        assert!(twins.host_new(&ev("4"), at(20)));
        assert!(!twins.own_new(&ev("4"), at(25)));
        assert!(twins.own_new(&ev("1"), at(30)));
        assert!(twins.own_new(&ev("1"), at(40)));
        assert!(!twins.host_new(&ev("1"), at(45)));
        assert!(!twins.host_new(&ev("1"), at(50)));
        assert!(twins.own_new(&ev("2"), at(60)));
        assert!(twins.host_new(&ev("2"), at(60 + 200)));
    }

    /// A value typed with the host's keys (Cubase) as with the window's own.
    #[test]
    fn a_value_typed_with_the_hosts_keys() {
        let (mut e, host, params) = editing();
        let p = on_strip(&e, strip_at(StripTarget::Readout(Readout::Drive)));
        click(&mut e, p);
        click(&mut e, p);
        for (character, key_code) in [(Some('1'), 0), (None, 26), (Some('\r'), 4)] {
            let k = host_key_event(HostKey {
                character,
                key_code,
                modifiers: 0,
                down: true,
            })
            .expect("a key");
            assert!(e.keyed(&k));
        }
        let d = params.drive.as_ptr();
        assert_eq!(
            host.take(),
            vec![
                Call::Begin(d),
                Call::Set(d, params.drive.preview_normalized(12.0)),
                Call::End(d)
            ]
        );
    }

    /// A value typed into a readout (decisions.md R52): a double click on it, its digits,
    /// Enter: its knob set at once, as the readout reads it, within the knob's travel, a gesture
    /// of its own. Escape, or a press elsewhere, leaves it as it was; a press on the readout
    /// itself goes on typing. A switch's readout double-clicked keeps its amount (its first
    /// press undone) and takes a value too. VOICES within 2 to 10; DETUNE in SCATTER takes none;
    /// AUTO GAIN's reads what is measured and takes none.
    #[test]
    fn a_value_typed_into_a_readout_sets_its_knob() {
        let (mut e, host, params) = editing_poly();
        let double = |e: &mut Editing, t: StripTarget| {
            let p = on_strip(e, strip_at(t));
            click(e, p);
            click(e, p);
        };
        let typed = |e: &mut Editing, text: &str| {
            for c in text.chars() {
                keyed(e, Key::Character(c.to_string()));
            }
        };
        let shown = |e: &mut Editing, r: Readout| {
            e.draw();
            let i = Readout::ALL
                .iter()
                .position(|x| *x == r)
                .expect("a readout");
            e.strip_scene.readouts[i].clone()
        };
        // DRIVE: 8.4 dB.
        double(&mut e, StripTarget::Readout(Readout::Drive));
        assert_eq!(host.take(), vec![]);
        assert_eq!(shown(&mut e, Readout::Drive), ("_".to_owned(), true));
        typed(&mut e, "8.4x");
        assert_eq!(shown(&mut e, Readout::Drive), ("8.4_".to_owned(), true));
        // (A press on it goes on typing.)
        let at = on_strip(&e, strip_at(StripTarget::Readout(Readout::Drive)));
        click(&mut e, at);
        keyed(&mut e, Key::Enter);
        let d = params.drive.as_ptr();
        assert_eq!(
            host.take(),
            vec![
                Call::Begin(d),
                Call::Set(d, params.drive.preview_normalized(8.4)),
                Call::End(d)
            ]
        );
        assert!(e.typing.is_none());
        // LEVEL: Escape, and a press elsewhere, leave it; beyond its travel, its end.
        double(&mut e, StripTarget::Readout(Readout::Level));
        typed(&mut e, "-6");
        keyed(&mut e, Key::Escape);
        double(&mut e, StripTarget::Readout(Readout::Level));
        typed(&mut e, "3");
        let elsewhere = on_strip(&e, (1500.0, 1700.0));
        click(&mut e, elsewhere);
        assert_eq!(host.take(), vec![]);
        assert!(e.typing.is_none());
        let l = params.level.as_ptr();
        for (keys, to) in [
            ("-99", 0.0),
            ("159\u{8}", 1.0),
            ("-9", params.level.preview_normalized(-9.0)),
        ] {
            double(&mut e, StripTarget::Readout(Readout::Level));
            for c in keys.chars() {
                if c == '\u{8}' {
                    keyed(&mut e, Key::Backspace);
                } else {
                    keyed(&mut e, Key::Character(c.to_string()));
                }
            }
            keyed(&mut e, Key::Enter);
            assert_eq!(
                host.take(),
                vec![Call::Begin(l), Call::Set(l, to), Call::End(l)],
                "{keys:?}"
            );
        }
        // ENTROPY's readout, a switch: its first press switched it (the test host applies
        // nothing, so there is nothing to undo here), then a value.
        double(&mut e, StripTarget::Switch(Readout::Entropy));
        let n = params.entropy.as_ptr();
        let first = host.take();
        assert_eq!(first.len(), 3, "the first press's switch: {first:?}");
        typed(&mut e, "25");
        keyed(&mut e, Key::Enter);
        assert_eq!(
            host.take(),
            vec![
                Call::Begin(n),
                Call::Set(n, params.entropy.preview_normalized(25.0)),
                Call::End(n)
            ]
        );
        // VOICES: 12 is 10.
        double(&mut e, StripTarget::Readout(Readout::Voices));
        typed(&mut e, "12");
        keyed(&mut e, Key::Enter);
        let v = params.voices.as_ptr();
        assert_eq!(
            host.take(),
            vec![Call::Begin(v), Call::Set(v, 1.0), Call::End(v)]
        );
        // DETUNE in SCATTER: none; AUTO GAIN's: none.
        double(&mut e, StripTarget::Readout(Readout::Detune));
        assert!(e.typing.is_none());
        assert!(!Readout::Auto.typed());
    }

    fn entry_click(e: &mut Editing, id: &str) {
        let p = at(e, centre(id));
        go(e, p);
        e.mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            modifiers: if cfg!(target_os = "macos") {
                Modifiers::META
            } else {
                Modifiers::CONTROL
            },
        });
        release(e);
    }

    #[test]
    fn knob_entry_opens_every_operable_knob_and_selector_without_a_gesture() {
        let (mut e, host, _) = editing_poly();
        // (The drawers' knobs take theirs in their readouts:
        // `the_arp_drawer_takes_knob_entry_in_its_readouts`, `the_mpe_drawer_operates_its_parameters`.)
        for c in CONTROLS.iter().filter(|c| {
            matches!(c.kind, Kind::Knob { .. } | Kind::Selector(_)) && !c.place.drawn_apart()
        }) {
            let i = ca72_panel::controls::index(c.param).expect("a control");
            if !e.operable(i) {
                continue;
            }
            entry_click(&mut e, c.param);
            assert!(e.knob_entry.is_some(), "{} opens", c.param);
            e.update_scene();
            let field = e.scene.entry.as_ref().expect("shown");
            assert!(!field.text.is_empty(), "{} shows its value", c.param);
            assert!(field.selected);
            assert!(e.typing_keys, "asks for the host's keyboard");
            keyed(&mut e, Key::Enter);
            assert!(e.knob_entry.is_none());
        }
        assert!(
            host.take().is_empty(),
            "opening and unchanged Enter set nothing"
        );
    }

    #[test]
    fn knob_entry_sets_one_host_gesture_and_reuses_the_strip_units() {
        let (mut e, host, params) = editing_poly();
        for (id, text, ptr, value) in [
            ("cutoff", "-2.5", params.cutoff.as_ptr(), 0.25),
            ("filter_attack", "1 s", params.filter_attack.as_ptr(), 0.6),
            ("osc1_range", "4", params.osc1_range.as_ptr(), 0.8),
            ("osc3_waveform", "6", params.osc3_waveform.as_ptr(), 1.0),
            ("voices", "99", params.voices.as_ptr(), 1.0),
            (
                "entropy",
                "25",
                params.entropy.as_ptr(),
                params.entropy.preview_normalized(25.0),
            ),
            ("spread", "-10", params.spread.as_ptr(), 0.0),
            ("inner", "999", params.inner.as_ptr(), 1.0),
            (
                "drive",
                "8.4",
                params.drive.as_ptr(),
                params.drive.preview_normalized(8.4),
            ),
            (
                "level",
                "-6",
                params.level.as_ptr(),
                params.level.preview_normalized(-6.0),
            ),
        ] {
            entry_click(&mut e, id);
            keyed(&mut e, Key::Character(text.into()));
            assert!(host.take().is_empty(), "{id} waits for Enter");
            keyed(&mut e, Key::Enter);
            assert_eq!(
                host.take(),
                vec![Call::Begin(ptr), Call::Set(ptr, value), Call::End(ptr)],
                "{id}"
            );
        }
        // DETUNE is only operable in DOUBLE, still typed in cents rather than its percentage.
        let p = nih_plug::context::process::TestProcessContext::<crate::Ca72>::new(
            Arc::clone(&params) as Arc<dyn Params>,
            48_000.0,
            ProcessMode::Realtime,
        );
        assert!(p.automate(params.doubled.as_ptr(), 1.0));
        entry_click(&mut e, "double");
        keyed(&mut e, Key::Character("50".into()));
        keyed(&mut e, Key::Enter);
        let ptr = params.double.as_ptr();
        assert_eq!(
            host.take(),
            vec![
                Call::Begin(ptr),
                Call::Set(
                    ptr,
                    params
                        .double
                        .preview_normalized((50.0 / crate::engine::DOUBLE_CENTS * 100.0) as f32)
                ),
                Call::End(ptr)
            ]
        );
    }

    #[test]
    fn knob_entry_frequency_uses_the_chosen_law_through_the_host() {
        let (mut e, host, params) = editing();
        for law in [Law::Semitones, Law::Hardware] {
            params.set_freq_law(law);
            for (id, pot, ptr) in [
                ("osc2_frequency", Pot::Two, params.osc2_frequency.as_ptr()),
                ("osc3_frequency", Pot::Three, params.osc3_frequency.as_ptr()),
            ] {
                entry_click(&mut e, id);
                keyed(&mut e, Key::Character("5".into()));
                keyed(&mut e, Key::Enter);
                let wiper = pot.wiper_at(law, (5.0 + 7.5) / 15.0) as f32;
                assert_eq!(
                    host.take(),
                    vec![Call::Begin(ptr), Call::Set(ptr, wiper), Call::End(ptr)]
                );
            }
        }
    }

    #[test]
    fn knob_entry_cancels_without_changes_and_keeps_keys_from_the_host() {
        let (mut e, host, _) = editing_standalone();
        e.playing.typing = true;
        for cancel in [Key::Escape, Key::Enter] {
            entry_click(&mut e, "cutoff");
            assert!(!e.note_typing_allowed());
            for key in [
                Key::ArrowUp,
                Key::Tab,
                Key::Character("a".into()),
                Key::Character("2x".into()),
            ] {
                assert!(keyed(&mut e, key));
            }
            assert!(!e.playing.active(), "typing values plays no notes");
            assert!(e.keyed(&KeyboardEvent {
                key: Key::Character("2".into()),
                state: KeyState::Up,
                ..Default::default()
            }));
            keyed(&mut e, cancel);
            assert!(e.knob_entry.is_none());
            assert!(host.take().is_empty());
        }
        entry_click(&mut e, "cutoff");
        keyed(&mut e, Key::Character("3".into()));
        let p = at(&e, centre("volume"));
        click(&mut e, p);
        assert!(e.knob_entry.is_none());
        assert!(
            host.take().is_empty(),
            "outside press cancels and does not operate another knob"
        );
        entry_click(&mut e, "cutoff");
        let p = at(&e, centre("cutoff"));
        click(&mut e, p);
        assert!(e.knob_entry.is_some(), "inside press keeps typing");
        e.end_gestures();
        assert!(e.knob_entry.is_none(), "focus/capture loss cancels");
        assert!(host.take().is_empty());
    }

    #[test]
    fn knob_entry_preserves_reset_alt_click_and_midi_learning() {
        let (mut e, host, params) = editing();
        let p = at(&e, centre("cutoff"));
        go(&mut e, p);
        e.press(Modifiers::ALT);
        release(&mut e);
        assert!(e.knob_entry.is_none());
        click(&mut e, p);
        let calls = host.take();
        assert_eq!(calls.last(), Some(&Call::End(params.cutoff.as_ptr())));
        assert!(calls.contains(&Call::Set(
            params.cutoff.as_ptr(),
            params.cutoff.default_normalized_value()
        )));
        well_formed(&calls);
        e.learning.learn(&params.midi_map, learnable("cutoff"));
        entry_click(&mut e, "cutoff");
        assert!(e.knob_entry.is_none(), "MIDI Learn keeps its click path");
        assert_eq!(params.midi_map.armed(), Some(learnable("cutoff")));
        e.end_gestures();
        well_formed(&host.take());
        e.learning.cancel(&params.midi_map);
        entry_click(&mut e, "double");
        assert!(e.knob_entry.is_none(), "inert DETUNE takes no value");
    }

    #[test]
    fn knob_entry_editing_and_hosts_keypad_keys_set_only_valid_complete_numbers() {
        let (mut e, host, params) = editing();
        for text in [
            "NaN",
            "inf",
            "2x",
            "--2",
            "1..2",
            "2½",
            "000000000000000000000000000000001x",
        ] {
            entry_click(&mut e, "cutoff");
            keyed(&mut e, Key::Character(text.into()));
            keyed(&mut e, Key::Enter);
            assert!(host.take().is_empty(), "invalid {text}");
        }
        entry_click(&mut e, "cutoff");
        keyed(&mut e, Key::Character("123".into()));
        assert!(e.keyed(&KeyboardEvent {
            key: Key::Character("a".into()),
            modifiers: if cfg!(target_os = "macos") {
                Modifiers::META
            } else {
                Modifiers::CONTROL
            },
            ..Default::default()
        }));
        keyed(&mut e, Key::Delete);
        let keys = HostKeys::default();
        keys.holding.store(e.holds_keys(), Ordering::Release);
        for code in [37, 26, 38, 29, 19] {
            // Keypad minus, 2, point, 5, Enter (VST3).
            assert!(keys.offer(HostKey {
                character: None,
                key_code: code,
                modifiers: 0,
                down: true
            }));
        }
        for k in keys.take() {
            assert!(e.keyed(&k));
        }
        let p = params.cutoff.as_ptr();
        assert_eq!(
            host.take(),
            vec![Call::Begin(p), Call::Set(p, 0.25), Call::End(p)]
        );
        assert!(e.knob_entry.is_none());
        keys.holding.store(e.holds_keys(), Ordering::Release);
        assert!(
            !keys.offer(HostKey {
                character: Some('7'),
                key_code: 0,
                modifiers: 0,
                down: true
            }),
            "the host gets keys back after Enter"
        );
    }

    #[test]
    fn knob_entry_initial_text_follows_host_automation() {
        let (mut e, host, params) = editing();
        let ptr = params.drive.as_ptr();
        let context = nih_plug::context::process::TestProcessContext::<crate::Ca72>::new(
            Arc::clone(&params) as Arc<dyn Params>,
            48_000.0,
            ProcessMode::Realtime,
        );
        for value in [0.0, 0.25, 0.7532, 1.0] {
            assert!(context.automate(ptr, value));
            entry_click(&mut e, "drive");
            e.update_scene();
            let shown: f32 = e.scene.entry.as_ref().unwrap().text.parse().unwrap();
            assert!((shown - params.drive.unmodulated_plain_value()).abs() < 0.0001);
            keyed(&mut e, Key::Enter);
        }
        assert!(host.take().is_empty());
    }

    #[test]
    fn knob_entry_fits_and_repaints_in_both_looks_sizes_and_lettering() {
        let output = std::env::var_os("CA72_KNOB_ENTRY_PNG").map(std::path::PathBuf::from);
        if let Some(out) = &output {
            std::fs::create_dir_all(out).unwrap();
        }
        let (mut e, host, _) = editing_poly();
        for skin in [Skin::Drawn, Skin::Worn] {
            e.set_look(skin);
            for lettering in [Lettering::Standard, Lettering::Large] {
                e.set_lettering(lettering);
                for width in [MIN_WIDTH, DEFAULT_WIDTH] {
                    for expanded in [true, false] {
                        e.params.editor_panel.store(expanded, Ordering::Relaxed);
                        e.params
                            .editor_panel_applied
                            .store(expanded, Ordering::Relaxed);
                        e.set_panel_height(expanded);
                        e.fit(1.0, width, width);
                        e.hover = None;
                        e.draw();
                        for id in ["tune", "filter_attack", "osc1_range", "volume", "level"] {
                            if id == "level" && !expanded {
                                continue;
                            }
                            entry_click(&mut e, id);
                            assert!(e.draw());
                            let field = e.scene.entry.as_ref().expect("drawn");
                            let [x0, y0, x1, y1] = field.bounds;
                            let height = f64::from(e.physical_height()) / e.renderer.scale();
                            assert!(
                                x0 >= 2.0 && y0 >= 2.0 && x1 <= art::W - 2.0 && y1 <= height - 2.0,
                                "{id}: {:?}",
                                field.bounds
                            );
                            let mut whole = None;
                            compose(
                                &mut whole,
                                &e.renderer,
                                e.strip.frame(),
                                None,
                                None,
                                (
                                    e.page_shows().then(|| e.lfo.frame()),
                                    e.macros_show()
                                        .then(|| (e.macros.frame(), e.renderer.left_rect())),
                                    &e.modulation,
                                ),
                                None,
                                expanded,
                                (e.physical_width, e.physical_height()),
                            );
                            assert!(
                                e.composed == whole,
                                "{skin:?} {lettering:?} {width} {expanded} {id}: composed whole with its field, actual expanded {}",
                                e.expanded()
                            );
                            if let Some(out) = &output
                                && width == DEFAULT_WIDTH
                                && expanded
                            {
                                e.composed
                                    .as_ref()
                                    .unwrap()
                                    .save_png(out.join(format!("{skin:?}-{lettering:?}-{id}.png")))
                                    .unwrap();
                            }
                            keyed(
                                &mut e,
                                Key::Character("99999999999999999999999999999999".into()),
                            );
                            assert!(e.draw(), "edited text redraws");
                            keyed(&mut e, Key::Escape);
                            assert!(e.draw(), "cancel redraws");
                            assert!(e.scene.entry.is_none());
                            let mut whole = None;
                            compose(
                                &mut whole,
                                &e.renderer,
                                e.strip.frame(),
                                None,
                                None,
                                (
                                    e.page_shows().then(|| e.lfo.frame()),
                                    e.macros_show()
                                        .then(|| (e.macros.frame(), e.renderer.left_rect())),
                                    &e.modulation,
                                ),
                                None,
                                expanded,
                                (e.physical_width, e.physical_height()),
                            );
                            assert!(
                                e.composed == whole,
                                "{id}: no field left in a partial repaint"
                            );
                        }
                    }
                }
            }
        }
        assert!(host.take().is_empty());
    }

    /// ENTROPY's new reading and OFF/recall use the entire travel, including below 5 %.
    #[test]
    fn entropy_displays_the_new_amount_and_recalls_it_after_off() {
        let (mut e, host, params) = editing();
        let context = nih_plug::context::process::TestProcessContext::<crate::Ca72>::new(
            Arc::clone(&params) as Arc<dyn Params>,
            48_000.0,
            ProcessMode::Realtime,
        );
        let p = params.entropy.as_ptr();
        let shown = |e: &mut Editing| {
            e.draw();
            e.strip_scene.readouts[Readout::ALL
                .iter()
                .position(|r| *r == Readout::Entropy)
                .unwrap()]
            .clone()
        };
        assert_eq!(shown(&mut e), ("5".to_owned(), true));
        for (amount, text) in [(0.05, "5"), (0.02, "2"), (0.43, "43")] {
            assert!(context.automate(p, amount));
            assert_eq!(shown(&mut e), (text.to_owned(), true));
            e.switch_amount(Readout::Entropy, false);
            assert_eq!(
                host.take(),
                vec![Call::Begin(p), Call::Set(p, 0.0), Call::End(p)]
            );
            assert!(context.automate(p, 0.0));
            assert_eq!(shown(&mut e), ("OFF".to_owned(), false));
            e.switch_amount(Readout::Entropy, true);
            assert_eq!(
                host.take(),
                vec![Call::Begin(p), Call::Set(p, amount), Call::End(p)]
            );
        }
    }

    /// The rail's settings key (decisions.md R51) opens its menu: under LOOK the editor's
    /// looks, the realistic by default, marked and not to be chosen; CLASSIC draws the editor in
    /// the drawn skin and is kept, REALISTIC brings the realistic back.
    #[test]
    fn the_settings_key_chooses_the_look() {
        let (mut e, _, _) = editing();
        e.update = Update::with(
            crate::update::fake::no_curl,
            crate::update::fake::no_browser,
        );
        let update = e.update.scene();
        assert_eq!(
            (e.settings.skin, e.renderer.skin()),
            (Skin::Worn, Skin::Worn)
        );
        let key = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Settings)));
        click(&mut e, key);
        assert_eq!(
            e.learning.menu().map(|m| m.title.as_str()),
            Some(format!("SETTINGS · CA-72 {}", crate::Ca72::VERSION).as_str())
        );
        assert_eq!(e.update.scene(), update, "opening Settings does not check");
        assert_eq!(
            menu_items(&e)[..3],
            [
                ("LOOK".to_owned(), false),
                ("REALISTIC".to_owned(), false),
                ("CLASSIC".to_owned(), true),
            ]
        );
        assert_eq!(e.learning.menu().map(|m| m.marked[0]), Some(1));
        // The menu within the window, under the key.
        let m = e.learning.menu().expect("a menu");
        let (w, h) = m.extent(e.renderer.fonts());
        assert!(m.x >= 0.0 && m.x + w <= art::W && m.y + h <= art::H);
        // (Under the key where there is room; else, as with MPE's group at this size, its foot
        // at the window's, over the rail: decisions.md R-MPE.)
        if settings_menu_corner().1 + h <= art::H {
            assert!(m.y > strip::centre(Rail::Plugin, StripTarget::Bar(BarTarget::Settings)).1);
        } else {
            assert!((m.y + h - art::H).abs() < 1e-6);
        }
        menu_item(&mut e, "CLASSIC");
        assert_eq!(
            (e.settings.skin, e.renderer.skin()),
            (Skin::Drawn, Skin::Drawn)
        );
        assert!(e.learning.menu().is_none());
        e.draw();
        click(&mut e, key);
        assert_eq!(menu_items(&e)[1], ("REALISTIC".to_owned(), true));
        assert_eq!(e.learning.menu().map(|m| m.marked[0]), Some(2));
        // (The head does nothing: the menu stays open.)
        menu_item(&mut e, "LOOK");
        assert!(e.learning.menu().is_some());
        menu_item(&mut e, "REALISTIC");
        assert_eq!(
            (e.settings.skin, e.renderer.skin()),
            (Skin::Worn, Skin::Worn)
        );
        e.draw();
        e.update.press();
        assert_eq!(e.update.scene().text, "COULD NOT CHECK");
        click(&mut e, key);
        assert_eq!(
            e.learning.menu().map(|m| m.title.as_str()),
            Some(format!("SETTINGS · CA-72 {}", crate::Ca72::VERSION).as_str())
        );
        assert_eq!(e.update.scene().text, "COULD NOT CHECK");
    }

    /// Installed skin packs (decisions.md R-SKIN-PACKS) under LOOK, after the built-in two, one
    /// that cannot be used shown so and not to be chosen. Choosing one draws the editor in it,
    /// a click finding each control where its layout has it; nothing but the look changes (not
    /// one parameter is set); and the pack gone, the editor falls back to the pack's base look.
    #[test]
    fn the_settings_key_chooses_an_installed_pack() {
        let starter = Look::open(std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/skin-packs/starter"
        )))
        .unwrap_or_else(|r| panic!("{r}"));
        TEST_PACKS.with(|p| {
            *p.borrow_mut() = vec![
                Installed {
                    name: starter.menu_name(),
                    look: Some(starter.clone()),
                },
                Installed {
                    name: "broken-pack".into(),
                    look: None,
                },
            ]
        });
        let (mut e, host, _) = editing();
        let key = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Settings)));
        click(&mut e, key);
        assert_eq!(
            menu_items(&e)[..5],
            [
                ("LOOK".to_owned(), false),
                ("REALISTIC".to_owned(), false),
                ("CLASSIC".to_owned(), true),
                ("STARTER (UNSIGNED)".to_owned(), true),
                ("broken-pack (CANNOT BE USED)".to_owned(), false),
            ]
        );
        host.take();
        menu_item(&mut e, "STARTER (UNSIGNED)");
        assert_eq!(e.renderer.look().id, starter.id);
        assert_eq!(
            (
                e.settings.skin,
                e.settings.pack.map(|p| p.as_str().to_owned())
            ),
            (Skin::Drawn, Some(starter.id.clone()))
        );
        assert!(host.take().is_empty(), "a look set a parameter");
        e.draw();
        // A click finds TUNE where the pack has it, not where the official panel does.
        let tune = ca72_panel::controls::index("tune").expect("a control");
        let moved = e.renderer.arrangement().control(tune).centre();
        assert_ne!(moved, CONTROLS[tune].centre());
        assert_eq!(e.hit(moved.0, moved.1), Some(Target::Control(tune)));
        // Marked in the menu, as the look shown.
        click(&mut e, key);
        assert_eq!(e.learning.menu().map(|m| m.marked[0]), Some(3));
        menu_item(&mut e, "REALISTIC");
        assert_eq!(e.settings.pack, None);
        // The pack chosen, then gone: its base look, the setting kept for when it is back.
        let mut settings = Settings::default();
        settings.apply(Change::Pack(
            PackId::new(&starter.id).expect("an id"),
            Skin::Drawn,
        ));
        assert_eq!(chosen_look(&settings, &[]).id, ca72_panel::look::CLASSIC);
        let refused = Installed {
            name: "STARTER".into(),
            look: None,
        };
        assert_eq!(
            chosen_look(&settings, &[refused]).id,
            ca72_panel::look::CLASSIC
        );
        TEST_PACKS.with(|p| p.borrow_mut().clear());
    }

    /// The settings key's LETTERING (R-LETTERING): STANDARD by default, marked and not to be
    /// chosen. LARGE prints the panel's and the strip's lettering a size larger, in the look the
    /// editor is in, the menus' and the tip's with it, and is kept; a look chosen after keeps it;
    /// STANDARD brings the standard back. In the large lettering the settings' menu still fits
    /// the window, the panel shown or hidden.
    #[test]
    fn the_settings_key_chooses_the_lettering() {
        let (mut e, _, _) = editing();
        assert_eq!(
            (e.settings.lettering, e.renderer.lettering()),
            (Lettering::Standard, Lettering::Standard)
        );
        let tip = e.renderer.text_size();
        let key = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Settings)));
        click(&mut e, key);
        assert_eq!(
            menu_items(&e)[3..6],
            [
                ("LETTERING".to_owned(), false),
                ("STANDARD".to_owned(), false),
                ("LARGE".to_owned(), true),
            ]
        );
        assert_eq!(
            e.learning.menu().map(|m| m.marked.clone()),
            Some(vec![1, 4, 7, 10])
        );
        let standard = e.learning.menu().expect("a menu").size;
        menu_item(&mut e, "LARGE");
        assert_eq!(
            (
                e.settings.lettering,
                e.renderer.lettering(),
                e.renderer.skin()
            ),
            (Lettering::Large, Lettering::Large, Skin::Worn)
        );
        assert!((e.renderer.text_size() / tip - 1.25).abs() < 1e-9);
        assert!(e.learning.menu().is_none());
        e.draw();
        click(&mut e, key);
        let m = e.learning.menu().expect("a menu");
        assert!((m.size / standard - 1.25).abs() < 1e-9);
        let (w, h) = m.extent(e.renderer.fonts());
        assert!(m.x >= 0.0 && m.x + w <= art::W && m.y + h <= art::H);
        assert_eq!(menu_items(&e)[5], ("LARGE".to_owned(), false));
        menu_item(&mut e, "CLASSIC");
        assert_eq!(
            (e.renderer.skin(), e.renderer.lettering()),
            (Skin::Drawn, Lettering::Large)
        );
        e.draw();
        // The panel hidden: the menu within the view, above the drawer.
        let panel = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Panel)));
        click(&mut e, panel);
        let key = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Settings)));
        click(&mut e, key);
        e.draw();
        let m = e.learning.menu().expect("a menu");
        let (_, h) = m.extent(e.renderer.fonts());
        // (Shrunk to fit, its foot the drawer's top to a rounding.)
        assert!(m.y >= 0.0 && m.y + h <= DRAWER_TOP + 1e-9, "{} + {h}", m.y);
        menu_item(&mut e, "STANDARD");
        assert_eq!(
            (
                e.settings.lettering,
                e.renderer.lettering(),
                e.renderer.skin()
            ),
            (Lettering::Standard, Lettering::Standard, Skin::Drawn)
        );
        e.draw();
    }

    /// The settings key's TICKS & TITLES (decisions.md R-QUIET-PRINT): STANDARD by default and
    /// marked, under LETTERING's choices; QUIET prints the panel's ticks and titles quiet, in the
    /// look and lettering it is in, and is kept for the computer as the lettering is; STANDARD
    /// brings the approved panel back, to the pixel.
    #[test]
    fn the_settings_key_chooses_quiet_ticks_and_titles() {
        let (mut e, _, _) = editing();
        let dir = tempfile::tempdir().expect("a folder");
        let path = dir.path().join("CA-72").join("settings.toml");
        e.settings_file = Some(path.clone());
        e.draw();
        let approved = e.renderer.frame().data().to_vec();
        let key = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Settings)));
        click(&mut e, key);
        assert_eq!(
            menu_items(&e)[6..9],
            [
                ("TICKS & TITLES".to_owned(), false),
                ("STANDARD".to_owned(), false),
                ("QUIET".to_owned(), true),
            ]
        );
        assert!(e.learning.menu().expect("a menu").marked.contains(&7));
        menu_item(&mut e, "QUIET");
        assert_eq!(
            (
                e.settings.ticks_titles,
                e.renderer.print(),
                e.renderer.lettering(),
                e.renderer.skin()
            ),
            (
                TicksTitles::Quiet,
                TicksTitles::Quiet,
                Lettering::Standard,
                Skin::Worn
            )
        );
        assert_eq!(Settings::read(&path).ticks_titles, TicksTitles::Quiet);
        e.draw();
        assert!(e.renderer.frame().data() != approved.as_slice());
        // LARGE keeps it quiet.
        click(&mut e, key);
        menu_item(&mut e, "LARGE");
        assert_eq!(e.renderer.print(), TicksTitles::Quiet);
        click(&mut e, key);
        menu_item(&mut e, "STANDARD");
        assert_eq!(e.renderer.print(), TicksTitles::Quiet);
        click(&mut e, key);
        assert_eq!(menu_items(&e)[7], ("STANDARD".to_owned(), true));
        let m = e.learning.menu().expect("a menu").clone();
        let p = at(&e, m.item_at(7));
        click(&mut e, p);
        assert_eq!(
            (e.settings.ticks_titles, e.renderer.print()),
            (TicksTitles::Standard, TicksTitles::Standard)
        );
        assert_eq!(Settings::read(&path).ticks_titles, TicksTitles::Standard);
        e.draw();
        e.composed = None;
        e.draw();
        assert!(
            e.renderer.frame().data() == approved.as_slice(),
            "as approved"
        );
    }

    /// The settings' menu opens below its key, under the rail, without covering the key or
    /// the rail, at 4K's FIT TO SCREEN (100 % and 150 %), SIZE's 100 % (at either), a 1080p
    /// screen's FIT TO SCREEN, a 1366 by 768 laptop's 1076, SIZE's 75 % and the grip's 860, in
    /// both looks and letterings (decisions.md R-READABILITY-MINIMUM): in as few columns as fit,
    /// up to five, its groups as evenly spread as they go, its lettering at its size (never
    /// under the 4K minimum) and its rows' pitch as they were; LARGE at 860, which no columns
    /// fit, as little smaller as fits (in four: five of its groups are as tall, six rows, and
    /// wider), and no smaller than round 1 had it there (which shrank it to the window's height
    /// in one column, rising over the rail); each item found where it is drawn. Five columns,
    /// allowed for the standalone's longer menu (round 5), change it nowhere.
    #[test]
    fn the_settings_menu_opens_below_its_key_at_every_size() {
        // The window, its scale, and each lettering's columns' rows (the test's menu: LOOK,
        // LETTERING, TICKS & TITLES, FREQUENCY DIALS three rows each, WINDOW four, MOD WHEEL
        // SOURCE and MPE two).
        let ten_eleven: &[usize] = &[9, 11];
        let three: &[usize] = &[6, 6, 8];
        let four: &[usize] = &[6, 6, 4, 4];
        let fhd = fitted_width_in(Some((1920.0, 1080.0)), art::Panel::OFFICIAL);
        assert_eq!(fhd, 1513, "a 1080p screen's FIT TO SCREEN");
        for (dpr, width, std, large) in [
            (1.0, 3026, ten_eleven, ten_eleven),
            (1.5, 2017, ten_eleven, ten_eleven),
            (1.0, 1380, three, three),
            (1.5, 1380, three, three),
            (1.0, fhd, ten_eleven, three),
            (1.0, 1076, three, four),
            (1.0, 1035, four, four),
            (1.0, MIN_WIDTH, four, four),
        ] {
            for skin in [Skin::Worn, Skin::Drawn] {
                for (lettering, rows) in [(Lettering::Standard, std), (Lettering::Large, large)] {
                    let (mut e, _, _) = editing();
                    let (m, got, at) = settings_menu_at(&mut e, (dpr, width), (skin, lettering));
                    assert_eq!(got, rows, "{at}: its columns' rows");
                    // Its lettering: as it is (so never under the 4K minimum), but for LARGE at
                    // 860, made smaller only as far as fits, as round 3 had it, and no smaller
                    // than round 1's.
                    if width == MIN_WIDTH && lettering == Lettering::Large {
                        settings_menu_as_large_as_fits(&e, &m, &at);
                        // (Round 1's em there and round 3's, physical pixels: `sizes.csv`.)
                        let em = m.size * e.renderer.scale();
                        assert!(em >= 13.493 && (em - 14.57).abs() < 0.005, "{at}: {em}");
                    } else {
                        let full = e.renderer.text_size() * MENU_TEXT;
                        assert_eq!(m.size, full, "{at}: not shrunk");
                    }
                }
            }
        }
    }

    /// The standalone's settings menu (decisions.md R-READABILITY-MINIMUM, rounds 4 and 5): with
    /// STANDALONE and AGENTS after MPE, at the sizes the plug-in's is tried at, below its key and
    /// not over it, in balanced columns that never split a group, up to five. As it opens
    /// (AGENTS off, its setups to copy: 27 rows) and at its fullest (the audio not real-time,
    /// AGENTS allowed and saying how they stand, a setup copied: 29). Its lettering as it is at
    /// 4K FIT TO SCREEN, wherever its columns fit and, as it opens in STANDARD, at every size;
    /// in the smaller windows, where five columns of its groups are wider or taller than the
    /// room left of and under the key, made smaller only as far as fits, to the em recorded.
    #[cfg(feature = "standalone-production")]
    #[test]
    fn the_standalones_settings_menu_opens_below_its_key_at_every_size() {
        // Each window's columns' rows, STANDARD then LARGE, as it opens then at its fullest; the
        // em (physical pixels, `sizes.csv`) where it is made smaller.
        type Case<'a> = (&'a [usize], Option<f64>);
        let (two, three, four, five): (Case, Case, Case, Case) = (
            ([12, 15].as_slice(), None),
            (&[9, 9, 9], None),
            (&[6, 6, 8, 7], None),
            (&[6, 6, 6, 4, 5], None),
        );
        let five_at = |em| (five.0, Some(em));
        let (two_full, three_full, four_full, five_full): (Case, Case, Case, Case) = (
            (&[16, 13], None),
            (&[9, 11, 9], None),
            (&[9, 7, 7, 6], None),
            (&[6, 6, 6, 5, 6], None),
        );
        // At its fullest, smaller: LOOK alone in its first column, the narrowest way and so the
        // one that needs it least (the more even 6, 6, 6, 5, 6 is wider).
        let small_full = |em| ([3, 6, 7, 7, 6].as_slice(), Some(em));
        let fhd = fitted_width_in(Some((1920.0, 1080.0)), art::Panel::OFFICIAL);
        #[rustfmt::skip]
        let cases: [(f64, u32, [Case; 2], [Case; 2]); 8] = [
            (1.0, 3026, [two, three], [two_full, three_full]),
            (1.5, 2017, [two, three], [two_full, three_full]),
            (1.0, 1380, [three, four], [four_full, five_full]),
            (1.5, 1380, [three, four], [four_full, five_full]),
            (1.0, fhd, [three, three], [three_full, four_full]),
            (1.0, 1076, [four, five], [five_full, small_full(14.996)]),
            (1.0, 1035, [five, five_at(16.719)], [five_full, small_full(14.425)]),
            (1.0, MIN_WIDTH, [five, five_at(13.892)], [small_full(11.986), small_full(11.986)]),
        ];
        for (dpr, width, opens, fullest) in cases {
            assert_eq!(
                opens[0].1, None,
                "{width}: STANDARD as it opens, at its size"
            );
            for (full, by_lettering) in [(false, opens), (true, fullest)] {
                for skin in [Skin::Worn, Skin::Drawn] {
                    for (lettering, (rows, shrunk)) in [Lettering::Standard, Lettering::Large]
                        .into_iter()
                        .zip(by_lettering)
                    {
                        let mut e = standalone_settings_editor(full);
                        let (m, got, at) =
                            settings_menu_at(&mut e, (dpr, width), (skin, lettering));
                        let at = format!("{at}, standalone{}", if full { " (full)" } else { "" });
                        let heads: Vec<&str> =
                            m.heads.iter().map(|&i| m.items[i].0.as_str()).collect();
                        assert_eq!(heads[7..], ["STANDALONE", "AGENTS"], "{at}");
                        assert_eq!(m.items.len(), if full { 29 } else { 27 }, "{at}");
                        let said = |text: &str| m.items.iter().any(|(t, _)| t.starts_with(text));
                        assert_eq!(
                            (
                                said("NOT REAL-TIME"),
                                said("COULD NOT LISTEN"),
                                said("COPIED: ")
                            ),
                            (full, full, full),
                            "{at}: {:?}",
                            m.items
                        );
                        assert_eq!(got, rows, "{at}: its columns' rows");
                        if let Some(em) = shrunk {
                            settings_menu_as_large_as_fits(&e, &m, &at);
                            let got = m.size * e.renderer.scale();
                            assert!((got - em).abs() < 0.005, "{at}: {got}");
                        } else {
                            let size = e.renderer.text_size() * MENU_TEXT;
                            assert_eq!(m.size, size, "{at}: not shrunk");
                        }
                    }
                }
            }
        }
    }

    /// An editor made as the standalone's, its host there (the AUDIO & MIDI key) and its bridge
    /// beside it (AGENTS' setups); `full`, its audio not real-time, agents allowed and saying
    /// how they stand, and a setup copied.
    #[cfg(feature = "standalone-production")]
    fn standalone_settings_editor(full: bool) -> Editing {
        crate::standalone_settings::AS_STANDALONE.set(true);
        crate::agent::setup::TEST_BRIDGE.with(|b| {
            *b.borrow_mut() = Some(std::path::PathBuf::from("/opt/ca-72/ca72-mcp"));
        });
        let (mut e, _, _) = editing_standalone();
        if full {
            #[cfg(target_os = "linux")]
            {
                e.realtime.read = stood::no_rtkit;
            }
            e.read_realtime(Instant::now());
            e.settings.agents = true;
            e.agents.error = Some("refused".into());
            e.agents.copied = Some((crate::learning::Setup::Json, true));
        }
        e
    }

    /// The settings key pressed in `e`; a setup copied (`e.agents.copied`, set before), its
    /// menu opened again saying so, as choosing COPY SETUP does (the key forgets it).
    fn press_settings_key(e: &mut Editing) {
        let copied = e.agents.copied;
        let key = on_strip(e, strip_at(StripTarget::Bar(BarTarget::Settings)));
        click(e, key);
        if copied.is_some() {
            e.agents.copied = copied;
            e.open_settings();
        }
    }

    /// The settings key's menu opened in `e` at (`dpr`, `width`) in (`skin`, `lettering`):
    /// below its key and not over it, each column begun at a group's head, each item where it
    /// is drawn, and its items chosen. The menu as it opened, its columns' rows, and the case.
    fn settings_menu_at(
        e: &mut Editing,
        (dpr, width): (f64, u32),
        (skin, lettering): (Skin, Lettering),
    ) -> (ca72_panel::learn::Menu, Vec<usize>, String) {
        let physical = (f64::from(width) * dpr).round() as u32;
        e.fit(dpr, width, physical);
        e.set_look(skin);
        e.set_lettering(lettering);
        e.draw();
        press_settings_key(e);
        let m = e.learning.menu().expect("the settings' menu").clone();
        let (w, h) = m.extent(e.renderer.fonts());
        let (kx, ky) = strip::centre(Rail::Plugin, StripTarget::Bar(BarTarget::Settings));
        let at = format!("{width} at {dpr}, {skin:?} {lettering:?}");
        assert_eq!(
            (m.x + w, m.y),
            settings_menu_corner(),
            "{at}: below its key"
        );
        assert!(
            m.y > ky + strip::KEY_H / 2.0 && m.y + h <= art::H + 1e-9,
            "{at}: under the key, not over it"
        );
        assert!(
            m.x + w <= kx + strip::KEY_W / 2.0 + 1e-9 && m.x >= 0.0,
            "{at}"
        );
        // Balanced: each column begun at a group's head, the rows as even as the groups go.
        assert!(m.columns.iter().all(|&(i, _)| m.heads.contains(&i)), "{at}");
        let starts: Vec<usize> = std::iter::once(0)
            .chain(m.columns.iter().map(|c| c.0))
            .chain(std::iter::once(m.items.len()))
            .collect();
        let rows: Vec<usize> = starts.windows(2).map(|s| s[1] - s[0]).collect();
        for i in 0..m.items.len() {
            let (x, y) = m.item_at(i);
            assert_eq!(
                m.hit(e.renderer.fonts(), x + 1.0, y),
                Some(Some(i)),
                "{at}: {i}"
            );
        }
        menu_item(e, "CLASSIC");
        assert_eq!(e.renderer.skin(), Skin::Drawn, "{at}: its items chosen");
        (m, rows, at)
    }

    /// The settings' menu `m` made smaller than its lettering, no columns fitting: only as far
    /// as fits.
    fn settings_menu_as_large_as_fits(e: &Editing, m: &ca72_panel::learn::Menu, at: &str) {
        let full = e.renderer.text_size() * MENU_TEXT;
        let (w, h) = m.extent(e.renderer.fonts());
        assert!(m.size < full, "{at}: no columns fit");
        let room = (m.x + w, art::H - m.y);
        assert!(
            w > room.0 * (1.0 - 1e-6) || h > room.1 * (1.0 - 1e-6),
            "{at}: as large as fits, {w} {h} {room:?}"
        );
    }

    /// The settings key's FREQUENCY DIALS (decisions.md R54): SEMITONES by default, each
    /// group's choice marked. HARDWARE turns oscillators 2 and 3's knobs to where the hardware's
    /// dial has their pots, the parameters (the sound) as they were, and the tip still reads
    /// semitones; a pot past a law's stop shows its knob at the stop, and a press that does not
    /// move it leaves it there. A drag sets the pot through the law chosen.
    #[test]
    fn the_settings_key_chooses_how_the_frequency_dials_read() {
        use nih_plug::context::process::TestProcessContext;
        let (mut e, host, params) = editing();
        let key = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Settings)));
        click(&mut e, key);
        assert_eq!(
            menu_items(&e)[9..12],
            [
                ("FREQUENCY DIALS".to_owned(), false),
                ("SEMITONES".to_owned(), false),
                ("HARDWARE".to_owned(), true),
            ]
        );
        assert_eq!(
            e.learning
                .menu()
                .map(|m| (m.heads.clone(), m.marked.clone())),
            // (WINDOW's HOVER CABLES row before MOD WHEEL SOURCE: R-VELOCITY; TICKS & TITLES
            // after LETTERING: R-QUIET-PRINT.)
            Some((vec![0, 3, 6, 9, 12, 16, 18], vec![1, 4, 7, 10]))
        );
        // Oscillator 2 at +5 semitones, oscillator 3's pot at its clockwise end (+8.22).
        let ps: Arc<dyn Params> = params.clone();
        let c = TestProcessContext::<crate::Ca72>::new(ps, 48_000.0, ProcessMode::Realtime);
        let five = Pot::Two.wiper(5.0) as f32;
        c.automate(params.osc2_frequency.as_ptr(), five);
        c.automate(params.osc3_frequency.as_ptr(), 1.0);
        menu_item(&mut e, "SEMITONES");
        assert!(
            e.learning.menu().is_some(),
            "the current choice does nothing"
        );
        let away = at(&e, (0.0, 0.0));
        click(&mut e, away);
        let (two, three) = (
            ca72_panel::controls::index("osc2_frequency").expect("a control"),
            ca72_panel::controls::index("osc3_frequency").expect("a control"),
        );
        e.update_scene();
        assert!((e.scene.values[two] - (5.0 + 7.5) / 15.0).abs() < 1e-6);
        assert_eq!(e.scene.values[three], 1.0, "past the stop: at it");
        host.take();
        click(&mut e, key);
        menu_item(&mut e, "HARDWARE");
        assert_eq!(
            (e.settings.frequency, params.freq_law()),
            (Law::Hardware, Law::Hardware)
        );
        assert!(host.take().is_empty(), "the sound as it was");
        e.update_scene();
        let knob = Pot::Two.knob_at(Law::Hardware, f64::from(five));
        assert!((e.scene.values[two] - knob).abs() < 1e-6);
        assert!(
            e.scene.values[two] < (5.0 + 7.5) / 15.0,
            "the hardware's +5 is wider"
        );
        assert_eq!(e.scene.values[three], 1.0);
        click(&mut e, key);
        assert_eq!(
            e.learning.menu().map(|m| m.marked.clone()),
            Some(vec![1, 4, 7, 11])
        );
        let away = at(&e, (0.0, 0.0));
        click(&mut e, away);
        // The tip in semitones either way.
        let p = e.operated(two).expect("a knob");
        assert_eq!(p.text(p.value()), "+5.00 semitones");
        // A drag sets the pot through the law chosen: the knob turned as far through either.
        let drag_up = |e: &mut Editing| {
            let p2 = at(e, centre("osc2_frequency"));
            go(e, p2);
            e.mouse(MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                modifiers: Modifiers::empty(),
            });
            go(
                e,
                Point {
                    x: p2.x,
                    y: p2.y - 20.0,
                },
            );
            e.mouse(MouseEvent::ButtonReleased {
                button: MouseButton::Left,
                modifiers: Modifiers::empty(),
            });
            let f = params.osc2_frequency.as_ptr();
            host.take()
                .into_iter()
                .filter_map(|c| match c {
                    Call::Set(q, v) if q == f => Some(f64::from(v)),
                    _ => None,
                })
                .last()
                .expect("set")
        };
        let turned = |law: Law, wiper: f64| {
            Pot::Two.knob_at(law, wiper) - Pot::Two.knob_at(law, f64::from(five))
        };
        let hardware = turned(Law::Hardware, drag_up(&mut e));
        e.set_frequency_dials(Law::Semitones);
        // (Not a double click, which would restore the default.)
        e.last_press = None;
        let semitones = turned(Law::Semitones, drag_up(&mut e));
        assert!(
            hardware > 0.01 && (hardware - semitones).abs() < 1e-5,
            "{hardware} {semitones}"
        );
        e.set_frequency_dials(Law::Hardware);
        e.last_press = None;
        // Oscillator 3's knob, pinned at its stop, pressed and let go without moving: as it was.
        let p3 = at(&e, centre("osc3_frequency"));
        go(&mut e, p3);
        e.mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            modifiers: Modifiers::empty(),
        });
        e.mouse(MouseEvent::ButtonReleased {
            button: MouseButton::Left,
            modifiers: Modifiers::empty(),
        });
        let g = params.osc3_frequency.as_ptr();
        assert!(
            !host
                .take()
                .iter()
                .any(|c| matches!(c, Call::Set(q, _) if *q == g)),
            "a press that did not move sets nothing"
        );
    }

    /// A right click on QUALITY's opening (decisions.md R48) opens its menu: its shutter or
    /// always open, the one it is in in its title and not to be chosen; choosing the other
    /// switches it, and the menu says so the next time.
    #[test]
    fn quality_s_menu_shows_or_hides_its_indicator() {
        let (mut e, _, _) = editing();
        assert!(e.settings.shutter, "shown by default");
        let toggle = at(&e, centre("quality"));
        right_click(&mut e, toggle);
        assert_eq!(
            e.learning.menu().map(|m| m.title.as_str()),
            Some("QUALITY: SET PER PLUGIN INSTANCE")
        );
        assert_eq!(
            menu_items(&e),
            vec![
                ("SHOW ITS INDICATOR".to_owned(), false),
                ("HIDE ITS INDICATOR".to_owned(), true),
                ("NOT LEARNED BY MIDI LEARN".to_owned(), false),
                ("MIDI ASSIGNMENTS\u{2026}".to_owned(), true),
            ]
        );
        menu_item(&mut e, "HIDE ITS INDICATOR");
        assert!(!e.settings.shutter && e.learning.menu().is_none());
        e.draw();
        assert!(e.scene.opening.hidden);
        right_click(&mut e, toggle);
        assert_eq!(menu_items(&e)[0], ("SHOW ITS INDICATOR".to_owned(), true));
        menu_item(&mut e, "SHOW ITS INDICATOR");
        assert!(e.settings.shutter);
        // (Where the indicator is, a right click is the panel's, as anywhere off a control.)
        let above = at(&e, (art::COL + 3023.0, art::TOP + 92.0));
        right_click(&mut e, above);
        assert!(e.learning.menu().is_none());
    }

    /// QUALITY's opening (decisions.md R48) follows QUALITY by the time between frames
    /// (a stalled frame does not jump it): from HI, its lamp goes down over the closing time,
    /// then ULTRA's Tesla lamp comes up over the opening time; up, ULTRA's note shows until a
    /// press reads it, and not again; with the shutter off, its own times.
    #[test]
    fn quality_s_opening_follows_quality_and_ultra_s_note_is_read_once() {
        let (mut e, _, _) = editing_at_hi();
        assert_eq!(
            (e.motion.lamp, e.motion.coil),
            (1.0, 0.0),
            "it opens at HI, up"
        );
        let t0 = Instant::now();
        let frames = |e: &mut Editing, from: Instant, n: u32, q: QualityMode| {
            for k in 0..=n {
                e.move_opening(from + Duration::from_millis(u64::from(k) * 40), q);
            }
        };
        let ultra = QualityMode::Ultra;
        e.move_opening(t0, ultra);
        frames(&mut e, t0, 30, ultra);
        assert!(
            (e.motion.lamp - (1.0 - 1.2 / ultra::CLOSE_S)).abs() < 1e-9,
            "{}",
            e.motion.lamp
        );
        assert_eq!(e.motion.coil, 0.0);
        // (A stall of a second moves it on 50 ms.)
        e.move_opening(t0 + Duration::from_millis(2200), ultra);
        assert!(
            (e.motion.lamp - (1.0 - 1.25 / ultra::CLOSE_S)).abs() < 1e-9,
            "{}",
            e.motion.lamp
        );
        frames(&mut e, t0 + Duration::from_millis(2200), 100, ultra);
        assert_eq!((e.motion.lamp, e.motion.coil), (0.0, 1.0));
        assert_eq!(e.scene.opening.coil, 1.0);
        assert!(e.ultra_note);
        assert_eq!(e.ultra_note().lines[0].0, "ULTRA · HIGH FIDELITY");
        press(&mut e);
        assert!(!e.ultra_note && e.settings.ultra_note_read);
        e.move_opening(t0 + Duration::from_secs(8), ultra);
        assert!(!e.ultra_note);
        // Hidden: none of it drawn, LO's hamster there at once (shown again, nothing comes up).
        e.settings.shutter = false;
        let t1 = t0 + Duration::from_secs(10);
        frames(&mut e, t1, 1, QualityMode::Lo);
        assert_eq!((e.motion.coil, e.motion.hamster), (0.0, 1.0));
        assert!(e.scene.opening.hidden);
        e.settings.shutter = true;
        frames(&mut e, t1 + Duration::from_secs(1), 1, QualityMode::Lo);
        assert!(!e.scene.opening.hidden && e.scene.opening.hamster == 1.0);
    }

    /// ULTRA's note shows once a computer: read in another editor since this one opened (a
    /// second instance, another host), it is not shown here as the lamp comes up, and this
    /// editor's own change to another setting writes that key alone (the CA-74's review).
    #[test]
    fn ultra_s_note_read_elsewhere_is_not_shown_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CA-72").join("settings.toml");
        let (mut e, _, _) = editing();
        e.settings_file = Some(path.clone());
        // The other editor reads the note and hides nothing; this one then turns its
        // indicator... on (as it was): the note stays read in the file.
        Settings::keep(&path, Change::NoteRead).unwrap();
        e.keep_settings(Change::Shutter(true));
        assert!(Settings::read(&path).ultra_note_read);
        let t0 = Instant::now();
        for k in 0..150u64 {
            e.move_opening(t0 + Duration::from_millis(40 * k), QualityMode::Ultra);
        }
        assert_eq!(e.motion.coil, 1.0);
        assert!(!e.ultra_note && e.settings.ultra_note_read);
    }

    /// What follows the synth moves thirty times a second at baseview's own frame rate, a frame
    /// every 15 ms (macOS, Windows and X11 alike), which reaches a thirtieth of a second only
    /// every third frame: about 90 moves in three seconds (it had been 66, 22 a second: the CA-74's
    /// review); a stalled frame moves it once and the next on time, not in a burst.
    #[test]
    fn the_opening_moves_thirty_times_a_second_on_baseviews_frames() {
        let (mut e, _, _) = editing();
        let t0 = Instant::now();
        let mut moved = 0;
        for k in 0..200u64 {
            let was = e.motion_at;
            e.move_opening(t0 + Duration::from_millis(15 * k), QualityMode::Hi);
            moved += u32::from(e.motion_at != was);
        }
        assert!((89..=91).contains(&moved), "{moved} moves in 3 s");
        // A half-second stall: one move then, the next a thirtieth later or so.
        let late = t0 + Duration::from_millis(3500);
        let was = e.motion_at;
        e.move_opening(late, QualityMode::Hi);
        assert_ne!(e.motion_at, was);
        let mut burst = 0;
        for k in 1..=2u64 {
            let was = e.motion_at;
            e.move_opening(late + Duration::from_millis(15 * k), QualityMode::Hi);
            burst += u32::from(e.motion_at != was);
        }
        assert!(burst <= 1, "{burst} moves within 30 ms of a stall");
    }

    /// What follows the synth is drawn at most thirty times a second: frames a sixtieth of a
    /// second apart move QUALITY's opening on every other one, the levels taken then.
    #[test]
    fn what_follows_the_synth_moves_thirty_times_a_second() {
        let (mut e, _, _) = editing();
        let t0 = Instant::now();
        e.move_opening(t0, QualityMode::Hi);
        let mut moved = 0;
        for k in 1..=60u64 {
            e.meters.publish_levels(&[0.25; LEVELS], 0.25);
            let was = e.motion_at;
            e.move_opening(t0 + Duration::from_micros(k * 16_667), QualityMode::Hi);
            moved += u32::from(e.motion_at != was);
        }
        assert!((28..=31).contains(&moved), "{moved}");
        assert!(e.levels[0] > 0.5 && e.motion.level > 0.0, "{:?}", e.levels);
    }

    /// An editor watches the levels while it is open: the audio thread measures them only
    /// then.
    #[test]
    fn the_levels_are_watched_only_while_an_editor_is_open() {
        let meters = Arc::new(Meters::default());
        assert!(!meters.watched());
        let host = Arc::new(Host::default());
        let context: Arc<dyn GuiContext> = host.clone();
        let dir = tempfile::tempdir().unwrap();
        let e = Editing::new(
            context,
            Arc::new(Ca72Params::default()),
            Arc::clone(&meters),
            (1720, 1720, 1.0),
            Library::at(dir.path()),
            |_| None,
        );
        assert!(meters.watched());
        drop(e);
        assert!(!meters.watched());
    }

    /// The window's frame put together again in the places that changed is the frame put
    /// together whole: through the voices sounding (the strip's drops), the wheels taken from a
    /// keyboard (the strip's left, the panel's layers), the OVERLOAD lamp (the panel above) and
    /// QUALITY's lamp lit by the synth's level at each tick of its clock (the panel's top
    /// right); and the window's pixels turned in those places alone are its pixels turned whole.
    #[test]
    fn the_frame_put_together_in_places_is_the_frame_put_together_whole() {
        let (mut e, _, _) = editing();
        assert!(e.draw());
        let size = |f: &ca72_panel::Pixmap| (f.width() as usize, f.height() as usize);
        let f = e.composed.as_ref().expect("a frame");
        let mut screen = vec![0u32; size(f).0 * size(f).1];
        window::fill(&mut screen, size(f), &[f]);
        let mut places = 0;
        for i in 0..30u8 {
            let lamp = if i % 8 < 4 { 0.0 } else { 1.0 };
            let wheels = (0.5 + 0.05 * f32::from(i % 5), 0.1 * f32::from(i % 3));
            e.meters.publish(lamp, wheels, u32::from(i % 6 < 3));
            let level = 0.2 + 0.1 * f32::from(i % 7);
            e.meters.publish_levels(&[level; LEVELS], level);
            e.motion_due = None;
            if !e.draw() {
                continue;
            }
            let mut whole = None;
            compose(
                &mut whole,
                &e.renderer,
                e.strip.frame(),
                lower_of(&e),
                None,
                (
                    e.page_shows().then(|| e.lfo.frame()),
                    e.macros_show()
                        .then(|| (e.macros.frame(), e.renderer.left_rect())),
                    &e.modulation,
                ),
                None,
                true,
                (e.physical_width, e.physical_height()),
            );
            assert!(e.composed == whole, "frame {i}: not as put together whole");
            let f = e.composed.as_ref().expect("a frame");
            let mut all = vec![0u32; screen.len()];
            window::fill(&mut all, size(f), &[f]);
            let d = e.damage.as_deref().expect("where it changed");
            places += d.len();
            window::fill_places(&mut screen, size(f), &[f], d);
            assert!(screen == all, "frame {i}: not as turned whole");
        }
        assert!(places > 30, "{places} places changed");
    }

    /// The drawer's slide ends where it was going in the frame shown, though its last moving
    /// frame stops short of there: the strip's rows put together again under it afterwards
    /// (the voices' drops) leave it whole. (In REAPER on Windows, 2026-10-10, the list's rows
    /// over the voices' display had come out a few pixels lower than the rest.)
    #[test]
    fn the_drawer_ends_its_slide_in_the_frame_shown() {
        let dir = tempfile::tempdir().unwrap();
        let (mut e, _, _) = editing_with(Library::at(dir.path()));
        e.draw();
        let p = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Name)));
        click(&mut e, p);
        // A moment of the slide short of its end, then its end.
        e.browser
            .slide_began(Instant::now() - crate::presets::SLIDE.mul_f64(0.5));
        e.draw();
        assert!(e.reveal_shown > 0.0 && e.reveal_shown < 0.99);
        e.browser
            .slide_began(Instant::now() - crate::presets::SLIDE * 2);
        e.draw();
        e.meters.publish(0.0, (0.5, 0.0), 1);
        e.draw();
        let mut whole = None;
        compose(
            &mut whole,
            &e.renderer,
            e.strip.frame(),
            None,
            Some((e.drawer.frame(), DRAWER_TOP)),
            (
                e.page_shows().then(|| e.lfo.frame()),
                e.macros_show()
                    .then(|| (e.macros.frame(), e.renderer.left_rect())),
                &e.modulation,
            ),
            None,
            true,
            (e.physical_width, e.physical_height()),
        );
        assert!(e.composed == whole, "the drawer where its slide ended");
    }

    /// The drawer dropped from the rail as the editor lays it.
    fn lower_of(e: &Editing) -> Option<LowerFrame<'_>> {
        e.lower_top().map(|top| {
            lower_frame(
                e.lower.which,
                top,
                (&e.arp, &e.mpe),
                e.fx.as_ref(),
                e.renderer.scale(),
            )
        })
    }

    /// The arpeggiator's drawer opened by the rail's ARP key and slid all the way down.
    fn arp_open(e: &mut Editing) {
        e.draw();
        let p = on_strip(e, strip_at(StripTarget::Bar(BarTarget::Arp)));
        click(e, p);
        assert!(e.lower.open);
        e.lower.slide = Some((Instant::now() - crate::presets::SLIDE * 2, true));
        e.draw();
        e.draw();
        assert!(e.lower.settled());
    }

    /// A skin pack's taller panel (decisions.md R-SKIN-PACKS) begins the frame above the
    /// drawing's top: the arpeggiator's drawer (R-ARP) is still laid under the rail, the frame
    /// shown the frame put together whole, its plate where it is drawn, its knobs found and
    /// turned there; the standalone's TEMPO screen mirrors the pack's name plate, on the taller
    /// panel's wood, and is found there.
    #[test]
    fn a_taller_packs_panel_keeps_the_arp_drawer_and_the_tempo_screen_in_place() {
        let starter = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/skin-packs/starter");
        let dir = tempfile::tempdir().expect("a pack's folder");
        std::fs::create_dir(dir.path().join("art")).expect("its art's folder");
        for f in ["skin.toml", "LICENSE.txt", "README.md", "art/panel.svg"] {
            std::fs::copy(format!("{starter}/{f}"), dir.path().join(f)).expect("copied");
        }
        let layout = std::fs::read_to_string(format!("{starter}/layout.toml")).expect("a layout");
        std::fs::write(
            dir.path().join("layout.toml"),
            layout.replacen("[fixtures]", "[panel]\nface = 1000\n\n[fixtures]", 1),
        )
        .expect("written");
        let tall = Look::open(dir.path()).unwrap_or_else(|r| panic!("{r}"));
        let (mut e, host, params) = editing_standalone();
        e.playing.visible = false;
        e.draw_in(tall);
        let (s, origin, top) = (e.renderer.scale(), e.renderer.top_px(), e.renderer.top());
        assert!(origin < 0, "a taller panel");
        // A point of the drawing in the window, whose frame begins at the panel's top.
        let at_tall = |e: &Editing, (x, y): (f64, f64)| at(e, (x, y - top));
        e.draw();
        let p = at_tall(&e, strip_at(StripTarget::Bar(BarTarget::Arp)));
        click(&mut e, p);
        assert!(e.lower.open);
        e.lower.slide = Some((Instant::now() - crate::presets::SLIDE * 2, true));
        e.draw();
        e.draw();
        assert!(e.lower.settled());
        let mut whole = None;
        compose(
            &mut whole,
            &e.renderer,
            e.strip.frame(),
            lower_of(&e),
            None,
            (
                e.page_shows().then(|| e.lfo.frame()),
                e.macros_show()
                    .then(|| (e.macros.frame(), e.renderer.left_rect())),
                &e.modulation,
            ),
            None,
            true,
            (e.physical_width, e.physical_height()),
        );
        assert!(e.composed == whole, "the drawer as put together whole");
        let c = e.composed.as_ref().expect("a frame");
        let px = |p: &ca72_panel::Pixmap, x: usize, y: i32| {
            let i = (y as usize * p.width() as usize + x) * 4;
            p.data()[i..i + 4].to_vec()
        };
        let (x, y) = (
            (2000.0 * s) as usize,
            ((arp_ui::TOP + 200.0) * s).round() as i32,
        );
        assert_eq!(
            px(c, x, y - origin),
            px(e.arp.frame(), x, y - arp_ui::top_px(s))
        );
        // GATE turned where it is drawn.
        host.take();
        let gate = params.arp_gate.as_ptr();
        let p = at_tall(&e, centre("arp_gate"));
        go(&mut e, p);
        press(&mut e);
        go(&mut e, below(p, -30.0));
        release(&mut e);
        let calls = host.take();
        assert!(calls.contains(&Call::Begin(gate)), "{calls:?}");
        // The TEMPO screen as far down the taller panel's wood as the official one is down its.
        let plate = e.renderer.arrangement().plate_at();
        let (x, y, w, h) = art::tempo_window(plate);
        assert_eq!(y - art::TEMPO_SCREEN.1, plate.1 - art::PLATE_Y);
        assert!(top < y && y + h < top + art::TOP, "on the wood: {y}");
        e.update_scene();
        assert_eq!(e.scene.tempo.as_deref(), Some("120 BPM"));
        let p = at_tall(&e, (x + w / 2.0, y + h / 2.0));
        go(&mut e, p);
        press(&mut e);
        go(&mut e, below(p, -10.0));
        release(&mut e);
        assert_eq!(params.tempo(), 125.0);
    }

    /// The rail's ARP key drops the arpeggiator's drawer over the lower page, inside the window
    /// (its size kept), and puts it away; it is lit while the arpeggiator is on. Open, the
    /// drawer is what the pointer finds there: the strip's tabs and knobs and the keyboard's
    /// buttons under it do nothing; below it, the page answers as before. The frame shown is its
    /// frame put together whole, the drawer's shadow over the page; and the presets' drawer, opened,
    /// is over it.
    #[test]
    fn the_arp_key_drops_its_drawer_over_the_lower_page() {
        let (mut e, host, _) = editing_standalone();
        e.playing.visible = false;
        let size = (e.width, e.window_height());
        arp_open(&mut e);
        assert_eq!((e.width, e.window_height()), size);
        let mut whole = None;
        compose(
            &mut whole,
            &e.renderer,
            e.strip.frame(),
            lower_of(&e),
            None,
            (
                e.page_shows().then(|| e.lfo.frame()),
                e.macros_show()
                    .then(|| (e.macros.frame(), e.renderer.left_rect())),
                &e.modulation,
            ),
            None,
            true,
            (e.physical_width, e.physical_height()),
        );
        assert!(e.composed == whole, "the drawer as put together whole");
        // Its plate in the frame, row for row.
        let c = e.composed.as_ref().expect("a frame");
        let s = e.renderer.scale();
        let (x, y) = ((2000.0 * s) as usize, (arp_ui::TOP + 200.0) * s);
        let y = y.round() as usize;
        let row = arp_ui::top_px(s) as usize;
        let px = |p: &ca72_panel::Pixmap, x: usize, y: usize| {
            let i = (y * p.width() as usize + x) * 4;
            p.data()[i..i + 4].to_vec()
        };
        assert_eq!(px(c, x, y), px(e.arp.frame(), x, y - row));
        // The strip's controls under it do nothing: AUTO GAIN's tab (its top, clear of GATE's
        // readout over the rest of it), VOICES's knob's top.
        let auto = strip::Bank::Auto.tab(0);
        for (x, y) in [
            (auto.0, auto.1 - 26.0),
            (centre("voices").0, arp_ui::TOP + arp_ui::DEPTH - 20.0),
        ] {
            assert_eq!((arp_ui::hit(x, y), interact::hit_arp(x, y)), (None, None));
            let p = at(&e, (x, y));
            go(&mut e, p);
            press(&mut e);
            go(&mut e, below(p, -40.0));
            release(&mut e);
        }
        assert!(host.take().is_empty(), "nothing under the drawer");
        // The keyboard's header under it, the keyboard shown.
        e.strip_press(StripTarget::Bar(BarTarget::Keyboard));
        e.draw();
        let root = e.playing.root;
        let p = at(&e, (keyboard::UP_X, keyboard::HEADER));
        click(&mut e, p);
        assert_eq!(e.playing.root, root, "OCTAVE under the drawer");
        // A key below it plays.
        let p = at(&e, (1000.0, art::H - 60.0));
        go(&mut e, p);
        press(&mut e);
        assert!(e.playing.dragging(), "a key below the drawer");
        release(&mut e);
        // The ARP key lit while the arpeggiator is on.
        e.update_scene();
        assert!(!e.browser.bar.arp);
        // The presets' drawer over it; ARP again puts the presets away and shows it.
        e.strip_press(StripTarget::Bar(BarTarget::Name));
        assert!(e.browser.open && e.lower.open);
        e.strip_press(StripTarget::Bar(BarTarget::Arp));
        assert!(!e.browser.open && e.lower.open);
        e.strip_press(StripTarget::Bar(BarTarget::Arp));
        assert!(!e.lower.open);
        e.lower.slide = None;
        e.draw();
        assert_eq!(e.lower_top(), None);
        // PANEL hiding the lower panel puts it away at once.
        arp_open(&mut e);
        e.toggle_panel();
        assert!(!e.lower.open && e.lower_top().is_none());
    }

    /// ⌘-click (Ctrl-click) on the drawer's knobs types into its readouts, in their units, as a
    /// double click there does (R-KNOB-ENTRY with R-ARP): no panel field opens.
    #[test]
    fn the_arp_drawer_takes_knob_entry_in_its_readouts() {
        let (mut e, host, params) = editing();
        arp_open(&mut e);
        host.take();
        for (i, id) in arp_ui::KNOB_PARAMS.iter().enumerate() {
            entry_click(&mut e, id);
            assert!(e.knob_entry.is_none(), "{id}: no panel field");
            assert_eq!(e.typing.as_ref().map(|t| t.0), Some(Typed::Arp(i)), "{id}");
            assert!(!e.preset_keys_allowed() && !e.note_typing_allowed(), "{id}");
            keyed(&mut e, Key::Escape);
            assert!(e.typing.is_none(), "{id}");
        }
        assert!(
            host.take().is_empty(),
            "nothing set by the clicks or Escape"
        );
        entry_click(&mut e, "arp_gate");
        for c in "25".chars() {
            keyed(&mut e, Key::Character(c.to_string()));
        }
        keyed(&mut e, Key::Enter);
        let gate = params.arp_gate.as_ptr();
        assert!(
            host.take()
                .iter()
                .any(|c| matches!(c, Call::Set(q, v) if *q == gate && (*v - params.arp_gate.preview_normalized(25.0)).abs() < 1e-6)),
            "GATE typed in percent"
        );
    }

    /// The effects' list (the FX drawer's, R-FX-DRAWER) holds the keys while it is open: the
    /// arrows choose no preset and TYPING plays no note under it (R-PRESET-KEYS); Escape
    /// closes it, and they are the arrows' again.
    #[test]
    fn the_effects_list_holds_the_keys_while_open() {
        let (mut e, _, _) = editing_standalone();
        e.playing.focused = true;
        e.playing.visible = true;
        e.playing.typing = true;
        assert!(e.preset_keys_allowed() && e.note_typing_allowed());
        e.fx_view.open_list(0, false);
        assert!(e.fx_view.list_open());
        assert!(e.holds_keys());
        assert!(!e.preset_keys_allowed() && !e.note_typing_allowed());
        keyed(&mut e, Key::Escape);
        assert!(!e.fx_view.list_open());
        assert!(e.preset_keys_allowed() && e.note_typing_allowed());
    }

    /// The drawer's tabs and knobs operate the arpeggiator's parameters, each a gesture: ON and
    /// HOLD switched, DIRECTION's choice set; RATE and OCTAVES dragged from position to
    /// position, never between; GATE turned; a readout double-clicked takes a value typed (RATE
    /// a note value, 8t a triplet, 16. dotted; OCTAVES a digit; GATE percent).
    #[test]
    fn the_arp_drawer_operates_its_parameters() {
        let (mut e, host, params) = editing();
        arp_open(&mut e);
        host.take();
        let tab = |e: &mut Editing, b: ArpBank, i: usize| {
            let p = at(e, arp_ui::centre(ArpTarget::Tab(b, i)));
            click(e, p);
        };
        let (on, latch, dir) = (
            params.arp_on.as_ptr(),
            params.arp_latch.as_ptr(),
            params.arp_direction.as_ptr(),
        );
        tab(&mut e, ArpBank::Arp, 0);
        assert_eq!(
            host.take(),
            vec![Call::Begin(on), Call::Set(on, 1.0), Call::End(on)]
        );
        tab(&mut e, ArpBank::Arp, 1);
        assert_eq!(
            host.take(),
            vec![Call::Begin(latch), Call::Set(latch, 1.0), Call::End(latch)]
        );
        tab(&mut e, ArpBank::Direction, 3);
        assert_eq!(
            host.take(),
            vec![Call::Begin(dir), Call::Set(dir, 0.75), Call::End(dir)]
        );
        tab(&mut e, ArpBank::Direction, 4);
        assert_eq!(
            host.take(),
            vec![Call::Begin(dir), Call::Set(dir, 1.0), Call::End(dir)]
        );
        tab(&mut e, ArpBank::Direction, 0);
        assert!(host.take().is_empty(), "UP already");
        // RATE dragged: positions only.
        let rate = params.arp_rate.as_ptr();
        let p = at(&e, centre("arp_rate"));
        go(&mut e, p);
        press(&mut e);
        for dy in 1..60 {
            go(&mut e, below(p, -f64::from(dy)));
        }
        release(&mut e);
        let calls = host.take();
        well_formed(&calls);
        assert!(calls.len() > 3);
        for c in &calls {
            if let Call::Set(q, v) = *c {
                assert_eq!(q, rate);
                assert!(
                    (v * 12.0 - (v * 12.0).round()).abs() < 1e-5,
                    "between at {v}"
                );
            }
        }
        e.update_scene();
        assert!(
            e.scene
                .tip
                .as_ref()
                .is_some_and(|t| t.0.starts_with("RATE: "))
        );
        // Typed.
        let double = |e: &mut Editing, i: usize| {
            let p = at(e, arp_ui::centre(ArpTarget::Readout(i)));
            click(e, p);
            click(e, p);
        };
        let typed = |e: &mut Editing, text: &str| {
            for c in text.chars() {
                keyed(e, Key::Character(c.to_string()));
            }
            keyed(e, Key::Enter);
        };
        for (i, text, ptr, want) in [
            (
                0,
                "8t",
                rate,
                params.arp_rate.preview_normalized(ArpRate::EighthTriplet),
            ),
            (
                0,
                "16.",
                rate,
                params.arp_rate.preview_normalized(ArpRate::SixteenthDotted),
            ),
            (
                1,
                "3",
                params.arp_octaves.as_ptr(),
                params.arp_octaves.preview_normalized(3),
            ),
            (
                1,
                "5",
                params.arp_octaves.as_ptr(),
                params.arp_octaves.preview_normalized(5),
            ),
            (
                2,
                "25",
                params.arp_gate.as_ptr(),
                params.arp_gate.preview_normalized(25.0),
            ),
        ] {
            double(&mut e, i);
            assert!(e.typing.is_some(), "{text}");
            e.draw();
            assert_eq!(e.arp_scene.readouts[i], ("_".to_owned(), true));
            typed(&mut e, text);
            assert_eq!(
                host.take(),
                vec![Call::Begin(ptr), Call::Set(ptr, want), Call::End(ptr)],
                "{text}"
            );
        }
        // Not a note value: nothing.
        double(&mut e, 0);
        typed(&mut e, "5");
        assert!(host.take().is_empty());
        // A knob turned: the frame shown is the frame put together whole.
        let p = at(&e, centre("arp_gate"));
        go(&mut e, p);
        press(&mut e);
        go(&mut e, below(p, -30.0));
        assert!(e.draw());
        let mut whole = None;
        compose(
            &mut whole,
            &e.renderer,
            e.strip.frame(),
            lower_of(&e),
            None,
            (
                e.page_shows().then(|| e.lfo.frame()),
                e.macros_show()
                    .then(|| (e.macros.frame(), e.renderer.left_rect())),
                &e.modulation,
            ),
            None,
            true,
            (e.physical_width, e.physical_height()),
        );
        assert!(e.composed == whole, "a knob turned in the drawer");
        release(&mut e);
    }

    /// The expression drawer opened by the rail's MPE key and slid all the way down.
    fn mpe_open(e: &mut Editing) {
        e.draw();
        let p = on_strip(e, strip_at(StripTarget::Bar(BarTarget::Mpe)));
        click(e, p);
        assert!(e.lower.is_open(Lower::Mpe));
        e.lower.slide = Some((Instant::now() - crate::presets::SLIDE * 2, true));
        e.draw();
        e.draw();
        assert!(e.lower.settled());
    }

    /// The rail's MPE key (the owner's A, 2026-10-20; decisions.md R-MPE) drops the expression
    /// drawer over the lower page, inside the window (its size kept), one drawer at a time with
    /// ARP's; the frame shown is its frame put together whole. The strip under it does nothing.
    /// The key is lit while any of its four amounts is not 0.
    #[test]
    fn the_mpe_key_drops_its_drawer_one_at_a_time() {
        let (mut e, host, params) = editing();
        e.playing.visible = false;
        let size = (e.width, e.window_height());
        mpe_open(&mut e);
        assert_eq!((e.width, e.window_height()), size);
        assert_eq!(Lower::Mpe.depth(), mpe_ui::DEPTH);
        assert_eq!(e.lower_depth(), mpe_ui::DEPTH);
        assert_eq!(lower_of(&e).expect("MPE's frame").depth, mpe_ui::DEPTH);
        assert!(e.in_mpe((2000.0, mpe_ui::TOP + arp_ui::DEPTH + 20.0)));
        assert!(!e.in_lower((2000.0, mpe_ui::TOP + mpe_ui::DEPTH + 1.0)));
        let mut whole = None;
        compose(
            &mut whole,
            &e.renderer,
            e.strip.frame(),
            lower_of(&e),
            None,
            (
                e.page_shows().then(|| e.lfo.frame()),
                e.macros_show()
                    .then(|| (e.macros.frame(), e.renderer.left_rect())),
                &e.modulation,
            ),
            None,
            true,
            (e.physical_width, e.physical_height()),
        );
        assert!(e.composed == whole, "the drawer as put together whole");
        let c = e.composed.as_ref().expect("a frame");
        let s = e.renderer.scale();
        let (x, y) = (
            (2000.0 * s) as usize,
            ((mpe_ui::TOP + mpe_ui::DEPTH - 20.0) * s).round() as usize,
        );
        let row = mpe_ui::top_px(s) as usize;
        let px = |p: &ca72_panel::Pixmap, x: usize, y: usize| {
            let i = (y * p.width() as usize + x) * 4;
            p.data()[i..i + 4].to_vec()
        };
        assert_eq!(px(c, x, y), px(e.mpe.frame(), x, y - row));
        // VOICES's knob's top, under it, does nothing.
        let (x, y) = (centre("voices").0, mpe_ui::TOP + mpe_ui::DEPTH - 20.0);
        assert_eq!(
            (
                mpe_ui::hit(x, y),
                interact::hit_drawer(DrawerPlace::Mpe, x, y)
            ),
            (None, None)
        );
        let p = at(&e, (x, y));
        go(&mut e, p);
        press(&mut e);
        go(&mut e, below(p, -40.0));
        release(&mut e);
        assert!(host.take().is_empty(), "nothing under the drawer");
        // ARP puts it away for its own, MPE again the arpeggiator's for it, and again puts it
        // away.
        e.strip_press(StripTarget::Bar(BarTarget::Arp));
        assert!(e.lower.is_open(Lower::Arp) && !e.lower.is_open(Lower::Mpe));
        e.strip_press(StripTarget::Bar(BarTarget::Mpe));
        assert!(e.lower.is_open(Lower::Mpe) && !e.lower.is_open(Lower::Arp));
        e.strip_press(StripTarget::Bar(BarTarget::Mpe));
        assert!(!e.lower.open);
        // Lit while an amount is not 0, each of the four.
        let p = nih_plug::context::process::TestProcessContext::<crate::Ca72>::new(
            Arc::clone(&params) as Arc<dyn Params>,
            48_000.0,
            ProcessMode::Realtime,
        );
        e.update_scene();
        assert!(!e.browser.bar.mpe);
        for q in [
            &params.pressure_mod,
            &params.pressure_filter,
            &params.timbre_mod,
            &params.timbre_filter,
        ] {
            assert!(p.automate(q.as_ptr(), 0.6));
            e.update_scene();
            assert!(e.browser.bar.mpe, "{}", q.name());
            assert!(p.automate(q.as_ptr(), 0.5));
            e.update_scene();
            assert!(!e.browser.bar.mpe, "{}", q.name());
        }
    }

    /// INPUT (decisions.md R-MPE) reads MPE while each note's own expression came in the last
    /// 3 s, else CHANNEL while ordinary channel pressure or CC 74 did, else is dark; and the
    /// drawer's frame follows it.
    #[test]
    fn the_mpe_drawers_input_reads_what_came_in_lately() {
        use mpe_ui::Input;
        let (mut e, _, params) = editing();
        mpe_open(&mut e);
        e.update_scene();
        assert_eq!(e.mpe_scene.input, Input::None);
        params.heard.channel();
        e.update_scene();
        assert_eq!(e.mpe_scene.input, Input::Channel);
        params.heard.note();
        e.update_scene();
        assert_eq!(e.mpe_scene.input, Input::Mpe);
        assert!(e.draw());
        // Each note's own, quiet for longer than 3 s: the channel's, heard since, shows.
        params.heard.channel();
        e.update_scene();
        let long_ago = Instant::now() - HEARD_FOR - Duration::from_millis(10);
        e.heard.1 = Some(long_ago);
        e.update_scene();
        assert_eq!(e.mpe_scene.input, Input::Channel);
        e.heard.2 = Some(long_ago);
        e.update_scene();
        assert_eq!(e.mpe_scene.input, Input::None);
        assert!(e.draw());
    }

    /// The drawer's knobs operate the expression's four amounts (decisions.md R-MPE): turned,
    /// each a gesture; a readout double-clicked, or a knob ⌘-clicked (R-KNOB-ENTRY), takes a
    /// value typed in its units (MODULATION percent with its sign, CUTOFF octaves with a
    /// point); the readouts read the amounts (no "-0"); the frame shown with a knob turned is
    /// the frame put together whole.
    #[test]
    fn the_mpe_drawer_operates_its_parameters() {
        let (mut e, host, params) = editing();
        mpe_open(&mut e);
        host.take();
        for (i, id) in mpe_ui::KNOB_PARAMS.iter().enumerate() {
            entry_click(&mut e, id);
            assert!(e.knob_entry.is_none(), "{id}: no panel field");
            assert_eq!(e.typing.as_ref().map(|t| t.0), Some(Typed::Mpe(i)), "{id}");
            keyed(&mut e, Key::Escape);
            assert!(e.typing.is_none(), "{id}");
        }
        assert!(
            host.take().is_empty(),
            "nothing set by the clicks or Escape"
        );
        let double = |e: &mut Editing, i: usize| {
            let p = at(e, mpe_ui::centre(MpeTarget::Readout(i)));
            click(e, p);
            click(e, p);
        };
        let typed = |e: &mut Editing, text: &str| {
            for c in text.chars() {
                keyed(e, Key::Character(c.to_string()));
            }
            keyed(e, Key::Enter);
        };
        for (i, text, q, want) in [
            (0, "-50", &params.pressure_mod, -50.0),
            (1, "2.5", &params.pressure_filter, 2.5),
            (2, "100", &params.timbre_mod, 100.0),
            (3, "-1.25", &params.timbre_filter, -1.25),
            // (Past the knob's travel: its end.)
            (0, "-300", &params.pressure_mod, -100.0),
        ] {
            double(&mut e, i);
            assert!(e.typing.is_some(), "{text}");
            e.draw();
            assert_eq!(e.mpe_scene.readouts[i], ("_".to_owned(), true));
            typed(&mut e, text);
            let ptr = q.as_ptr();
            assert_eq!(
                host.take(),
                vec![
                    Call::Begin(ptr),
                    Call::Set(ptr, q.preview_normalized(want)),
                    Call::End(ptr)
                ],
                "{text}"
            );
        }
        // (A letter is not taken: nothing typed, nothing set.)
        double(&mut e, 1);
        typed(&mut e, "x");
        assert!(host.take().is_empty(), "not a number");
        // The readouts as the amounts read.
        let p = nih_plug::context::process::TestProcessContext::<crate::Ca72>::new(
            Arc::clone(&params) as Arc<dyn Params>,
            48_000.0,
            ProcessMode::Realtime,
        );
        assert!(p.automate(params.pressure_mod.as_ptr(), 0.0));
        assert!(p.automate(params.pressure_filter.as_ptr(), 0.5));
        assert!(p.automate(params.timbre_mod.as_ptr(), 0.65));
        assert!(p.automate(params.timbre_filter.as_ptr(), 0.4375));
        e.update_scene();
        assert_eq!(
            e.mpe_scene.readouts.clone().map(|r| r.0),
            ["-100", "0.0", "30", "-0.5", "0", "0"].map(str::to_owned)
        );
        // A knob turned: a gesture, and the frame shown the frame put together whole.
        let ptr = params.timbre_filter.as_ptr();
        let at_knob = at(&e, centre("timbre_filter"));
        go(&mut e, at_knob);
        press(&mut e);
        go(&mut e, below(at_knob, -30.0));
        assert!(e.draw());
        let mut whole = None;
        compose(
            &mut whole,
            &e.renderer,
            e.strip.frame(),
            lower_of(&e),
            None,
            (
                e.page_shows().then(|| e.lfo.frame()),
                e.macros_show()
                    .then(|| (e.macros.frame(), e.renderer.left_rect())),
                &e.modulation,
            ),
            None,
            true,
            (e.physical_width, e.physical_height()),
        );
        assert!(e.composed == whole, "a knob turned in the drawer");
        e.update_scene();
        assert!(
            e.scene
                .tip
                .as_ref()
                .is_some_and(|t| t.0.starts_with("TIMBRE CUTOFF: ")),
            "{:?}",
            e.scene.tip
        );
        release(&mut e);
        let calls = host.take();
        well_formed(&calls);
        assert!(
            calls
                .iter()
                .any(|c| matches!(c, Call::Set(q, _) if *q == ptr))
        );
    }

    /// The settings' MPE (decisions.md R-MPE): one item, INPUT & TIMBRE…, opening a menu of its
    /// own (the settings' is already taller than the window at its opening size), where MPE
    /// INPUT (AUTO by default) and TIMBRE REST (CENTERED ON 64 by default) are chosen for this
    /// computer, each group's choice marked.
    #[test]
    fn the_settings_mpe_menu_chooses_the_input_and_the_timbre_rest() {
        use crate::expression::{Mode, TimbreRest};
        let (mut e, _, params) = editing();
        let key = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Settings)));
        let open = |e: &mut Editing| {
            click(e, key);
            menu_item(e, "INPUT & TIMBRE…");
        };
        click(&mut e, key);
        let items = menu_items(&e);
        let at_head = items
            .iter()
            .position(|(t, _)| t == "MPE")
            .expect("an MPE head");
        assert_eq!(items[at_head + 1], ("INPUT & TIMBRE…".to_owned(), true));
        e.learning.close_menu();
        open(&mut e);
        assert_eq!(
            menu_items(&e),
            [
                ("MPE INPUT", false),
                ("AUTO", false),
                ("LOWER ZONE", true),
                ("OFF", true),
                ("TIMBRE REST", false),
                ("CENTERED ON 64", false),
                ("FROM 0", true),
                ("FROM THE NOTE'S START", true),
            ]
            .map(|(t, on)| (t.to_owned(), on))
        );
        let m = e.learning.menu().expect("a menu");
        assert_eq!(m.title, "MPE");
        assert_eq!(m.marked, vec![1, 5]);
        assert_eq!(m.heads, vec![0, 4]);
        menu_item(&mut e, "LOWER ZONE");
        assert_eq!(
            (params.mpe_mode(), e.settings.mpe),
            (Mode::Lower, Mode::Lower)
        );
        open(&mut e);
        menu_item(&mut e, "FROM THE NOTE'S START");
        assert_eq!(
            (params.timbre_rest(), e.settings.timbre_rest),
            (TimbreRest::NoteStart, TimbreRest::NoteStart)
        );
        open(&mut e);
        assert_eq!(e.learning.menu().expect("a menu").marked, vec![2, 7]);
        menu_item(&mut e, "OFF");
        assert_eq!(params.mpe_mode(), Mode::Off);
        open(&mut e);
        menu_item(&mut e, "FROM 0");
        assert_eq!(params.timbre_rest(), TimbreRest::Zero);
    }

    /// The standalone's editor with a chain for its effects' drawer (`fx_view::fake`): two
    /// effects in it, three to be found.
    fn editing_effects() -> (Editing, Arc<crate::fx_view::fake::Fake>) {
        let (mut e, _, _) = editing_standalone();
        let fake = crate::fx_view::fake::Fake::with(
            &["Chorus", "Hall Reverb"],
            &["Delay", "Plate", "Flanger"],
        );
        e.use_effects(FxView::new(Some(fake.clone())));
        assert!(
            e.playing.visible,
            "the standalone's keyboard shown, under the drawer"
        );
        (e, fake)
    }

    /// What the chain was asked to do, once the drawer's threads have done it (`n` of them).
    fn effects_done(e: &mut Editing, fake: &crate::fx_view::fake::Fake, n: usize) -> Vec<String> {
        let t0 = Instant::now();
        while fake.done.lock().map_or(0, |d| d.len()) < n && t0.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(2));
        }
        // (The chain read again at once: something done there has finished.)
        std::thread::sleep(Duration::from_millis(5));
        e.update_scene();
        fake.take()
    }

    /// The drawer dropped from the rail opened by `key` and slid all the way down.
    fn lower_open(e: &mut Editing, rail: Rail, key: BarTarget) {
        e.draw();
        let p = at(e, strip::centre(rail, StripTarget::Bar(key)));
        click(e, p);
        e.lower.slide = Some((Instant::now() - crate::presets::SLIDE * 2, true));
        e.draw();
        e.draw();
        assert!(e.lower.settled());
    }

    #[test]
    fn the_chain_lock_and_held_signs_follow_the_preset_with_the_drawer_open_or_shut() {
        let (mut e, fake) = editing_effects();
        e.draw();
        assert!(!e.browser.bar.fx_locked && !e.browser.bar.preset_fx);
        lower_open(&mut e, Rail::Effects, BarTarget::Fx);
        let lock = at(&e, fx_ui::lock_tab());
        click(&mut e, lock);
        e.draw();
        assert!(e.fx_scene.locked && e.browser.bar.fx_locked);
        assert!(!e.browser.bar.preset_fx);
        fake.preset.lock().unwrap().held = Some([
            "VintageVerb".into(),
            "RC-20".into(),
            "OTT".into(),
            "–".into(),
        ]);
        e.draw();
        assert!(e.browser.bar.preset_fx && e.fx_scene.held.is_some());
        let (x, y, w, h) = e.rail.name();
        let tag = at(&e, (x + w - 95.0, y + h / 2.0));
        go(&mut e, tag);
        assert_eq!(
            e.tip().map(|t| t.0),
            Some("This preset has its own effects. Unlock to hear them.".into())
        );
        let key = at(&e, strip::centre(e.rail, StripTarget::Bar(BarTarget::Fx)));
        click(&mut e, key);
        e.lower.slide = None;
        e.draw();
        assert!(!e.lower.is_open(Lower::Fx));
        assert!(e.browser.bar.fx_locked && e.browser.bar.preset_fx);
        fake.preset.lock().unwrap().held = None;
        e.draw();
        assert!(e.browser.bar.fx_locked && !e.browser.bar.preset_fx);
    }

    /// The agreed LOCK and its signs, drawn by the editor for visual review.
    #[test]
    #[ignore = "writes images for a look"]
    fn the_fx_lock_pngs() {
        let out =
            std::path::PathBuf::from(std::env::var_os("CA72_FX_LOCK_PNG").expect("output folder"));
        std::fs::create_dir_all(&out).unwrap();
        for scale in [0.44, 1.0] {
            for skin in [Skin::Worn, Skin::Drawn] {
                for lettering in [Lettering::Standard, Lettering::Large] {
                    let (mut e, fake) = editing_effects();
                    let width = (art::W * scale).round() as u32;
                    e.fit(1.0, width, width);
                    e.set_look(skin);
                    e.set_lettering(lettering);
                    *e.params.preset.write().unwrap() = "Felt Bass".into();
                    e.browser.reread(&e.params);
                    lower_open(&mut e, Rail::Effects, BarTarget::Fx);
                    let save = |e: &mut Editing, state: &str| {
                        window_frame(e)
                            .save_png(
                                out.join(format!("{skin:?}-{lettering:?}-{scale}-{state}.png")),
                            )
                            .unwrap();
                    };
                    save(&mut e, "unlocked");
                    fake.preset.lock().unwrap().locked = true;
                    save(&mut e, "locked");
                    fake.preset.lock().unwrap().held = Some([
                        "VintageVerb".into(),
                        "RC-20".into(),
                        "OTT".into(),
                        "–".into(),
                    ]);
                    save(&mut e, "held");
                    e.lower.shut();
                    save(&mut e, "closed");
                }
            }
        }
    }

    /// The standalone's rail has the FX key after ARP, the name's display a key shorter for it;
    /// the plug-ins' has neither, and no effects' drawer. The FX key drops the effects' drawer
    /// over the lower page inside the window (its size kept), and puts it away; ARP's drawer and
    /// FX's are never open together, each key putting the other's away for its own. The key is lit
    /// while an effect plays; the frame shown is its frame put together whole.
    #[test]
    fn the_fx_key_drops_its_drawer_one_drawer_at_a_time() {
        let (plugin, _, _) = editing();
        assert_eq!((plugin.rail, plugin.fx.is_none()), (Rail::Plugin, true));
        let (standalone, _, _) = editing_standalone();
        assert_eq!(standalone.rail, Rail::Plugin, "no chain: no FX key");
        let (fx_key, arp_key) = (
            strip::centre(Rail::Effects, StripTarget::Bar(BarTarget::Fx)),
            strip::centre(Rail::Effects, StripTarget::Bar(BarTarget::Arp)),
        );
        assert_eq!(
            strip::hit(Rail::Plugin, fx_key.0, fx_key.1),
            Some(StripTarget::Bar(BarTarget::Arp)),
            "the plug-ins' ARP where the standalone's FX is"
        );
        let (mut e, fake) = editing_effects();
        assert_eq!(e.rail, Rail::Effects);
        assert_eq!(
            strip::hit(e.rail, fx_key.0, fx_key.1),
            Some(StripTarget::Bar(BarTarget::Fx))
        );
        assert_eq!(
            strip::hit(e.rail, arp_key.0, arp_key.1),
            Some(StripTarget::Bar(BarTarget::Arp))
        );
        let size = (e.width, e.window_height());
        lower_open(&mut e, Rail::Effects, BarTarget::Fx);
        assert!(e.lower.is_open(Lower::Fx));
        assert_eq!((e.width, e.window_height()), size);
        let mut whole = None;
        compose(
            &mut whole,
            &e.renderer,
            e.strip.frame(),
            lower_of(&e),
            None,
            (
                e.page_shows().then(|| e.lfo.frame()),
                e.macros_show()
                    .then(|| (e.macros.frame(), e.renderer.left_rect())),
                &e.modulation,
            ),
            None,
            true,
            (e.physical_width, e.physical_height()),
        );
        assert!(e.composed == whole, "the drawer as put together whole");
        // ARP puts it away for its own; FX again puts that away for the effects'.
        e.strip_press(StripTarget::Bar(BarTarget::Arp));
        assert!(e.lower.is_open(Lower::Arp) && !e.lower.is_open(Lower::Fx));
        e.strip_press(StripTarget::Bar(BarTarget::Fx));
        assert!(e.lower.is_open(Lower::Fx) && !e.lower.is_open(Lower::Arp));
        e.strip_press(StripTarget::Bar(BarTarget::Fx));
        assert!(!e.lower.open);
        // The key lit while an effect plays.
        e.update_scene();
        assert!(e.browser.bar.fx);
        for s in fake.slots.lock().expect("slots").iter_mut() {
            s.bypass = true;
        }
        e.fx_view.poll(Instant::now() + Duration::from_secs(1));
        e.update_scene();
        assert!(!e.browser.bar.fx, "nothing plays");
        // PANEL hiding the lower panel puts it away at once.
        lower_open(&mut e, Rail::Effects, BarTarget::Fx);
        e.toggle_panel();
        assert!(!e.lower.open && e.lower_top().is_none());
    }

    /// The effects' drawer operates the chain: a name clicked opens its effect's window, a tab
    /// bypasses its effect, a name dragged onto another slot moves it there (shown carried over
    /// it meanwhile); "+ ADD EFFECT" drops the list of the effects found (and SCAN AGAIN), a row
    /// chosen adds its effect; a right click offers REPLACE… (the list again, to put another in
    /// its place) and REMOVE. Under the drawer the page does nothing; the frame shown with the list
    /// open is the frame put together whole.
    #[test]
    fn the_fx_drawer_operates_the_chain() {
        let (mut e, fake) = editing_effects();
        lower_open(&mut e, Rail::Effects, BarTarget::Fx);
        let name = |e: &Editing, i: usize| at(e, fx_ui::centre(fx_ui::FxTarget::Name(i)));
        // A name clicked: its effect's window.
        let p = name(&e, 0);
        click(&mut e, p);
        assert_eq!(effects_done(&mut e, &fake, 1), ["window 0"]);
        // A tab: its effect bypassed.
        let p = at(&e, fx_ui::centre(fx_ui::FxTarget::Tab(1)));
        click(&mut e, p);
        assert_eq!(effects_done(&mut e, &fake, 1), ["bypass 1 true"]);
        assert!(!e.fx_scene.slots[1].lit && e.fx_scene.slots[0].lit);
        // A name dragged onto another's.
        let (from, to) = (name(&e, 0), name(&e, 1));
        go(&mut e, from);
        press(&mut e);
        for k in 1..=10 {
            let t = f64::from(k) / 10.0;
            go(
                &mut e,
                Point {
                    x: from.x + (to.x - from.x) * t,
                    y: from.y,
                },
            );
        }
        e.update_scene();
        assert_eq!(e.fx_scene.drag, Some((0, Some(1))));
        release(&mut e);
        assert_eq!(effects_done(&mut e, &fake, 1), ["move 0 1"]);
        assert_eq!(e.fx_scene.slots[0].text, "Hall Reverb");
        // "+ ADD EFFECT": the list; a row chosen.
        assert_eq!(e.fx_scene.slots[2].text, "+ ADD EFFECT");
        let p = name(&e, 2);
        click(&mut e, p);
        e.update_scene();
        let list = e.fx_scene.list.clone().expect("the list");
        let names: Vec<&str> = list.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["Delay", "Flanger", "Plate", "SCAN AGAIN"]);
        assert_eq!(list.title, "ADD TO SLOT 3");
        e.draw();
        let mut whole = None;
        compose(
            &mut whole,
            &e.renderer,
            e.strip.frame(),
            lower_of(&e),
            None,
            (
                e.page_shows().then(|| e.lfo.frame()),
                e.macros_show()
                    .then(|| (e.macros.frame(), e.renderer.left_rect())),
                &e.modulation,
            ),
            None,
            true,
            (e.physical_width, e.physical_height()),
        );
        assert!(e.composed == whole, "the list as put together whole");
        // (The wheel over it scrolls it; three effects and SCAN AGAIN all show, so it stays.)
        let p = at(&e, fx_ui::row_centre(&list, 2));
        go(&mut e, p);
        wheel(&mut e, -2.0);
        e.update_scene();
        assert_eq!(e.fx_scene.list.as_ref().map(|l| l.first), Some(0));
        click(&mut e, p);
        assert!(!e.fx_view.list_open());
        assert_eq!(effects_done(&mut e, &fake, 1), ["add Plate"]);
        assert_eq!(e.fx_scene.slots[2].text, "Plate");
        assert_eq!(e.fx_scene.slots[3].text, "+ ADD EFFECT");
        // A right click: REPLACE… and REMOVE.
        let p = name(&e, 0);
        right_click(&mut e, p);
        assert_eq!(
            menu_items(&e),
            [
                ("PARAMETERS…".to_owned(), true),
                ("REPLACE\u{2026}".to_owned(), true),
                ("REMOVE".into(), true)
            ]
        );
        menu_item(&mut e, "REPLACE\u{2026}");
        e.update_scene();
        let list = e.fx_scene.list.clone().expect("the list");
        assert_eq!(list.title, "REPLACE SLOT 1");
        let p = at(&e, fx_ui::row_centre(&list, 1));
        click(&mut e, p);
        assert_eq!(effects_done(&mut e, &fake, 1), ["replace 0 Flanger"]);
        let p = name(&e, 1);
        right_click(&mut e, p);
        menu_item(&mut e, "REMOVE");
        assert_eq!(effects_done(&mut e, &fake, 1), ["remove 1"]);
        assert_eq!(e.fx_scene.slots[1].text, "Plate");
        // The list put away by a press elsewhere, which does nothing else.
        let p = name(&e, 2);
        click(&mut e, p);
        e.update_scene();
        assert!(e.fx_view.list_open());
        let p = at(&e, (200.0, 300.0));
        click(&mut e, p);
        assert!(!e.fx_view.list_open());
        // Under the drawer: AUTO GAIN's tab does nothing.
        let (x, y) = strip::Bank::Auto.tab(0);
        assert!(fx_ui::inside(x, y));
        let before = e.params.auto_gain.value();
        let p = at(&e, (x, y));
        click(&mut e, p);
        assert_eq!(
            e.params.auto_gain.value(),
            before,
            "nothing under the drawer"
        );
        assert!(fake.take().is_empty());
    }

    #[test]
    fn the_fx_chooser_takes_typing_and_arrows_and_escape_clears_before_closing() {
        let (mut e, fake) = editing_effects();
        lower_open(&mut e, Rail::Effects, BarTarget::Fx);
        e.playing.typing = true;
        e.fx_view.open_list(2, false);
        let preset = e.browser.bar.name.clone();
        for c in ["l", "a", "t"] {
            assert!(keyed(&mut e, Key::Character(c.into())));
        }
        assert!(keyed(&mut e, Key::ArrowLeft));
        assert!(keyed(&mut e, Key::ArrowRight));
        assert!(!e.playing.active());
        e.update_scene();
        assert_eq!(
            e.fx_scene.list.as_ref().expect("list").rows[0].name,
            "Plate"
        );
        assert_eq!(e.browser.bar.name, preset);
        assert!(keyed(&mut e, Key::Escape));
        e.update_scene();
        assert!(e.fx_view.list_open());
        assert!(
            e.fx_scene
                .list
                .as_ref()
                .expect("list")
                .query
                .text
                .is_empty()
        );
        assert!(keyed(&mut e, Key::Escape));
        assert!(!e.fx_view.list_open());
        e.fx_view.open_list(2, false);
        assert!(keyed(&mut e, Key::Character("lat".into())));
        assert!(keyed(&mut e, Key::Enter));
        assert_eq!(effects_done(&mut e, &fake, 1), ["add Plate"]);
    }

    /// A slot's name clicked again opens its effect's window again (decisions.md R-FX-MAC): after
    /// the window it opened took the focus and was closed, and while it is still open (brought
    /// forward). Each click is a job of its own; the helper shows the window in front.
    #[test]
    fn a_slots_name_clicked_again_opens_its_window_again() {
        let (mut e, fake) = editing_effects();
        lower_open(&mut e, Rail::Effects, BarTarget::Fx);
        let p = at(&e, fx_ui::centre(fx_ui::FxTarget::Name(0)));
        for _ in 0..3 {
            click(&mut e, p);
            assert_eq!(effects_done(&mut e, &fake, 1), ["window 0"]);
            // (The effect's window took the focus: the editor's lost it.)
            e.end_gestures();
        }
    }

    /// Effects turned off, and a safe start, in the drawer (decisions.md R-FX-SAFETY): a slot
    /// turned off after it crashed or stopped responding reads CRASHED or NOT RESPONDING under its
    /// dimmed name, the key unlit; its tip says why; its menu offers LOAD WITHOUT ITS SETTINGS
    /// first when its settings are the cause, else TRY AGAIN first; a click tries it again. One
    /// running without its settings offers TRY ITS OLD SETTINGS, and says FAILED, its name lit,
    /// when they still crash it. After a safe start each reads OFF, its tip says why, its menu
    /// offers TURN EFFECTS BACK ON and TURN THIS ONE ON, and a click turns them all on.
    #[test]
    fn the_fx_drawer_says_why_an_effect_is_off_and_tries_it_again() {
        use crate::fx_view::{Health, SafeStart};
        let (mut e, fake) = editing_effects();
        {
            let mut s = fake.slots.lock().expect("slots");
            s[0].health = Health::Crashed;
            s[0].by_settings = true;
            s[1].health = Health::NotResponding;
        }
        lower_open(&mut e, Rail::Effects, BarTarget::Fx);
        e.fx_view.poll(Instant::now() + Duration::from_secs(1));
        e.update_scene();
        let words = |e: &Editing| {
            e.fx_scene
                .slots
                .iter()
                .map(|s| (s.word, s.dim))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            words(&e)[..2],
            [
                (Some(fx_ui::FxWord::Crashed), true),
                (Some(fx_ui::FxWord::NotResponding), true)
            ]
        );
        assert!(!e.browser.bar.fx, "nothing plays: the key unlit");
        let name = |e: &Editing, i: usize| at(e, fx_ui::centre(fx_ui::FxTarget::Name(i)));
        let p = name(&e, 0);
        go(&mut e, p);
        let tip = e.fx_view.tip().map(|t| t.0).unwrap_or_default();
        assert!(tip.contains("saved settings crash it"), "{tip}");
        let items = |e: &Editing| {
            menu_items(e)
                .into_iter()
                .map(|(t, _)| t)
                .collect::<Vec<_>>()
        };
        right_click(&mut e, p);
        assert_eq!(
            items(&e),
            [
                "LOAD WITHOUT ITS SETTINGS",
                "TRY AGAIN",
                "PARAMETERS…",
                "REPLACE\u{2026}",
                "REMOVE"
            ]
        );
        menu_item(&mut e, "LOAD WITHOUT ITS SETTINGS");
        assert_eq!(effects_done(&mut e, &fake, 1), ["without settings 0"]);
        let p = name(&e, 1);
        right_click(&mut e, p);
        assert_eq!(
            items(&e),
            [
                "TRY AGAIN",
                "LOAD WITHOUT ITS SETTINGS",
                "PARAMETERS…",
                "REPLACE\u{2026}",
                "REMOVE"
            ]
        );
        e.learning.close_menu();
        click(&mut e, p);
        assert_eq!(effects_done(&mut e, &fake, 1), ["retry 1"]);
        // Running without its settings: its old ones offered back; still crashing it, FAILED.
        let p = name(&e, 0);
        right_click(&mut e, p);
        assert_eq!(
            items(&e),
            [
                "TRY ITS OLD SETTINGS",
                "PARAMETERS…",
                "REPLACE\u{2026}",
                "REMOVE"
            ]
        );
        menu_item(&mut e, "TRY ITS OLD SETTINGS");
        assert_eq!(effects_done(&mut e, &fake, 1), ["old settings 0"]);
        assert_eq!(words(&e)[0], (Some(fx_ui::FxWord::Failed), false));
        assert!(e.browser.bar.fx, "it plays on: the key lit");
        // A safe start.
        for s in fake.slots.lock().expect("slots").iter_mut() {
            s.health = Health::Off;
            s.try_failed = false;
        }
        *fake.safe.lock().expect("safe") = Some(SafeStart::DidNotClose);
        e.fx_view.poll(Instant::now() + Duration::from_secs(2));
        e.update_scene();
        assert_eq!(
            words(&e)[..2],
            [
                (Some(fx_ui::FxWord::Off), true),
                (Some(fx_ui::FxWord::Off), true)
            ]
        );
        assert!(!e.browser.bar.fx);
        let p = name(&e, 1);
        go(&mut e, p);
        let tip = e.fx_view.tip().map(|t| t.0).unwrap_or_default();
        assert!(tip.contains("did not close properly"), "{tip}");
        right_click(&mut e, p);
        assert_eq!(
            items(&e),
            [
                "TURN EFFECTS BACK ON",
                "TURN THIS ONE ON",
                "PARAMETERS…",
                "REPLACE\u{2026}",
                "REMOVE"
            ]
        );
        menu_item(&mut e, "TURN THIS ONE ON");
        assert_eq!(effects_done(&mut e, &fake, 1), ["retry 1"]);
        let p = name(&e, 0);
        click(&mut e, p);
        assert_eq!(effects_done(&mut e, &fake, 1), ["turn on"]);
        assert!(e.fx_scene.slots.iter().all(|s| s.word.is_none()));
        assert!(e.browser.bar.fx, "they play: the key lit");
    }

    /// Every gesture well formed: a parameter's begin only while it has none open, its
    /// changes and end only inside one, and none left open.
    fn well_formed(calls: &[Call]) {
        let mut open: Vec<ParamPtr> = Vec::new();
        for (i, c) in calls.iter().enumerate() {
            match *c {
                Call::Begin(p) => {
                    assert!(
                        !open.contains(&p),
                        "a begin inside a gesture at {i}: {calls:?}"
                    );
                    open.push(p);
                }
                Call::Set(p, _) => {
                    assert!(
                        open.contains(&p),
                        "a change outside a gesture at {i}: {calls:?}"
                    )
                }
                Call::End(p) => {
                    let k = open.iter().position(|&q| q == p);
                    let k = k.unwrap_or_else(|| panic!("an end with no begin at {i}: {calls:?}"));
                    open.remove(k);
                }
            }
        }
        assert!(open.is_empty(), "left open: {calls:?}");
    }

    fn press(e: &mut Editing) {
        e.mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            modifiers: Modifiers::empty(),
        });
    }

    fn release(e: &mut Editing) {
        e.mouse(MouseEvent::ButtonReleased {
            button: MouseButton::Left,
            modifiers: Modifiers::empty(),
        });
    }

    fn wheel(e: &mut Editing, lines: f32) {
        e.mouse(MouseEvent::WheelScrolled {
            delta: ScrollDelta::Lines { x: 0.0, y: lines },
            modifiers: Modifiers::empty(),
        });
    }

    fn below(p: Point, dy: f64) -> Point {
        Point {
            x: p.x,
            y: p.y + dy,
        }
    }

    /// Where a point of the drawing is in the window, in logical pixels.
    fn at(e: &Editing, (x, y): (f64, f64)) -> Point {
        let k = e.renderer.scale() / e.dpr;
        Point { x: x * k, y: y * k }
    }

    fn centre(param: &str) -> (f64, f64) {
        CONTROLS[ca72_panel::controls::index(param).expect("a control")].centre()
    }

    fn go(e: &mut Editing, p: Point) {
        e.mouse(MouseEvent::CursorMoved {
            position: p,
            modifiers: Modifiers::empty(),
        });
    }

    fn click(e: &mut Editing, p: Point) {
        go(e, p);
        e.mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            modifiers: Modifiers::empty(),
        });
        e.mouse(MouseEvent::ButtonReleased {
            button: MouseButton::Left,
            modifiers: Modifiers::empty(),
        });
    }

    fn right_click(e: &mut Editing, p: Point) {
        go(e, p);
        e.mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Right,
            modifiers: Modifiers::empty(),
        });
        e.mouse(MouseEvent::ButtonReleased {
            button: MouseButton::Right,
            modifiers: Modifiers::empty(),
        });
    }

    /// The open menu's items, each its label and whether it can be chosen.
    fn menu_items(e: &Editing) -> Vec<(String, bool)> {
        e.learning.menu().expect("a menu").items.clone()
    }

    /// The open menu's item of this label clicked.
    fn menu_item(e: &mut Editing, label: &str) {
        let m = e.learning.menu().expect("a menu").clone();
        let k = m
            .items
            .iter()
            .position(|(t, _)| t == label)
            .unwrap_or_else(|| panic!("no {label} in {:?}", m.items));
        let p = at(e, m.item_at(k));
        click(e, p);
    }

    /// A key as the window gives it to the editor.
    fn keyed(e: &mut Editing, k: Key) -> bool {
        e.keyed(&KeyboardEvent {
            state: KeyState::Down,
            key: k,
            ..KeyboardEvent::default()
        })
    }

    #[test]
    fn preset_arrows_respect_keyboard_takeovers_and_typing_octaves() {
        let (mut e, _, params) = editing_standalone();
        e.draw();
        let current = || params.preset.read().unwrap().clone();
        let before = current();
        assert!(keyed(&mut e, Key::ArrowRight));
        assert_ne!(current(), before);
        let before = current();
        assert!(keyed(&mut e, Key::ArrowLeft));
        assert_ne!(current(), before);
        // Up and down never step presets (the owner: left and right, as the rail's keys).
        for arrow in [Key::ArrowUp, Key::ArrowDown] {
            let before = current();
            let name = format!("{arrow:?}");
            assert!(!keyed(&mut e, arrow), "{name} left free");
            assert_eq!(current(), before);
        }
        e.playing.typing = true;
        let before = current();
        assert!(keyed(&mut e, Key::ArrowRight));
        assert_ne!(current(), before, "TYPE leaves arrows free");
        let root = e.playing.typing_root;
        let before = current();
        assert!(keyed(&mut e, Key::Character("x".into())));
        assert_eq!(e.playing.typing_root, root + 12);
        assert_eq!(current(), before);
        for modifiers in [
            Modifiers::SHIFT,
            Modifiers::ALT,
            Modifiers::CONTROL,
            Modifiers::META,
        ] {
            assert!(!e.keyed(&KeyboardEvent {
                key: Key::ArrowRight,
                modifiers,
                ..KeyboardEvent::default()
            }));
            assert_eq!(current(), before);
        }
        assert!(
            !keyed(&mut e, Key::Character("v".into())),
            "unused key stays the host's"
        );
        assert!(e.keyed(&KeyboardEvent {
            key: Key::ArrowRight,
            state: KeyState::Up,
            ..KeyboardEvent::default()
        }));
        assert_eq!(current(), before, "key release");
        e.playing.focused = false;
        assert!(!keyed(&mut e, Key::ArrowRight));
        assert_eq!(current(), before, "unfocused");
        e.playing.focused = true;
        e.start_typing(Readout::Drive);
        keyed(&mut e, Key::ArrowRight);
        assert_eq!(current(), before, "typed readout");
        keyed(&mut e, Key::Escape);
        params.midi_map.arm(learnable("cutoff"));
        assert!(!keyed(&mut e, Key::ArrowRight));
        assert_eq!(current(), before, "learning");
        params.midi_map.cancel();
        let point = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Next)));
        right_click(&mut e, point);
        assert!(e.learning.menu_open());
        assert!(!keyed(&mut e, Key::ArrowRight));
        assert_eq!(current(), before, "menu");
        keyed(&mut e, Key::Escape);
        e.browser.show(true, &params);
        let before = current();
        assert!(keyed(&mut e, Key::ArrowLeft));
        assert_eq!(current(), before, "search moves its caret");
        assert!(keyed(&mut e, Key::ArrowDown));
        assert_ne!(current(), before, "the open preset list keeps its arrows");
        let before = current();
        e.browser.drawer.editing = Some(ca72_panel::presets::Edit::Rename);
        e.browser.drawer.focus = Some(ca72_panel::presets::FieldId::Edit);
        keyed(&mut e, Key::ArrowDown);
        assert_eq!(current(), before, "rename field");
        e.browser.show(false, &params);
        e.show_release_notes(crate::whats_new::current());
        keyed(&mut e, Key::ArrowDown);
        assert_eq!(current(), before, "What's New scrolls");
    }

    #[test]
    fn both_preset_keys_open_the_existing_learn_and_remove_menu() {
        let (mut e, _, params) = editing();
        for (target, id) in [
            (BarTarget::Prev, "previous_preset"),
            (BarTarget::Next, "next_preset"),
        ] {
            let point = on_strip(&e, strip_at(StripTarget::Bar(target)));
            let before = params.preset.read().unwrap().clone();
            right_click(&mut e, point);
            assert!(
                menu_items(&e)
                    .iter()
                    .any(|(label, _)| label == "MIDI LEARN")
            );
            menu_item(&mut e, "MIDI LEARN");
            assert_eq!(params.midi_map.armed(), Some(learnable(id)));
            assert_eq!(
                params.midi_map.incoming_note(2, 44, true),
                crate::learn::Incoming::Caught
            );
            e.draw();
            assert_eq!(
                params.midi_map.binding(learnable(id)),
                Some(crate::learn::Binding::Note {
                    channel: 2,
                    note: 44
                })
            );
            assert_eq!(
                *params.preset.read().unwrap(),
                before,
                "learning never steps"
            );
            right_click(&mut e, point);
            menu_item(&mut e, "REMOVE MIDI ASSIGNMENT");
            assert_eq!(params.midi_map.binding(learnable(id)), None);
        }
    }

    #[test]
    fn forwarded_preset_arrows_leave_unused_and_modified_keys_with_the_host() {
        let keys = HostKeys::default();
        keys.navigation.store(true, Ordering::Release);
        let key = HostKey {
            character: None,
            key_code: 13,
            modifiers: 0,
            down: true,
        };
        assert!(keys.offer(key));
        assert_eq!(keys.take()[0].key, Key::ArrowRight);
        assert!(!keys.offer(HostKey {
            character: Some('q'),
            key_code: 0,
            ..key
        }));
        assert!(!keys.offer(HostKey {
            modifiers: 2,
            ..key
        }));
        keys.navigation.store(false, Ordering::Release);
        assert!(!keys.offer(key));
    }

    fn learnable(id: &str) -> usize {
        crate::learn::index(id).expect("learnable")
    }

    fn note_lines(e: &Editing) -> Vec<String> {
        e.scene
            .note
            .as_ref()
            .map(|n| n.lines.iter().map(|(t, _)| t.clone()).collect())
            .unwrap_or_default()
    }

    /// Every pixel the window is given is opaque, the frames' and the window's beyond them, so
    /// that a window with an alpha channel (baseview's on X11) shows the panel and nothing
    /// through it; the colour is the frame's.
    #[test]
    fn the_window_is_given_opaque_pixels() {
        assert_eq!(window::shown(&[0x12, 0x34, 0x56, 0xff]), 0xff12_3456);
        assert_eq!(window::shown(&[0, 0, 0, 0xff]) >> 24, 0xff);
        assert_eq!(window::EMPTY >> 24, 0xff);
    }

    /// A control's menu (a right click): MIDI LEARN rings it and says it waits, Escape cancels it,
    /// a controller caught is assigned and said; REMOVE MIDI ASSIGNMENT removes it. None of it
    /// is a gesture of a parameter's (the sound is left as it is), and a press outside the menu
    /// only closes it.
    #[test]
    fn a_controls_menu_learns_cancels_and_removes_its_controller() {
        let (mut e, host, params) = editing();
        let map = Arc::clone(&params.midi_map);
        let cutoff = learnable("cutoff");
        let knob = ca72_panel::controls::index("cutoff").expect("a control");
        let p = at(&e, centre("cutoff"));
        right_click(&mut e, p);
        e.draw();
        let m = e.scene.menu.clone().expect("a menu drawn");
        assert_eq!(m.title, "CUTOFF FREQUENCY · NO MIDI CONTROLLER");
        assert_eq!(
            m.items,
            [
                ("MIDI LEARN".to_owned(), true),
                ("REMOVE MIDI ASSIGNMENT".to_owned(), false),
                ("MIDI ASSIGNMENTS…".to_owned(), true),
            ]
        );
        menu_item(&mut e, "MIDI LEARN");
        assert_eq!(map.armed(), Some(cutoff));
        e.draw();
        assert_eq!((e.scene.menu.clone(), e.scene.learning), (None, Some(knob)));
        assert_eq!(note_lines(&e)[0], "MIDI LEARN: CUTOFF FREQUENCY");
        assert_eq!(e.scene.tip, None);
        assert!(e.learning.holds_keys(&map));
        // Escape: cancelled.
        assert!(keyed(&mut e, Key::Escape));
        assert_eq!(map.armed(), None);
        e.draw();
        assert_eq!(e.scene.learning, None);
        // Learned again, and a controller caught by the audio thread: assigned and said.
        let p = at(&e, centre("cutoff"));
        right_click(&mut e, p);
        menu_item(&mut e, "MIDI LEARN");
        assert_eq!(map.incoming(0, 74), crate::learn::Incoming::Caught);
        e.draw();
        assert_eq!(
            map.assignment(cutoff)
                .map(crate::learn::Cc::text)
                .as_deref(),
            Some("CH 1 · CC 74")
        );
        assert_eq!(note_lines(&e), ["CUTOFF FREQUENCY: CH 1 · CC 74"]);
        assert_eq!(e.scene.learning, None);
        // Its menu now names it, and removes it.
        let p = at(&e, centre("cutoff"));
        right_click(&mut e, p);
        assert_eq!(
            e.learning.menu().map(|m| m.title.as_str()),
            Some("CUTOFF FREQUENCY · CH 1 · CC 74")
        );
        assert!(menu_items(&e)[1].1, "REMOVE can be chosen");
        menu_item(&mut e, "REMOVE MIDI ASSIGNMENT");
        assert_eq!(map.assignment(cutoff), None);
        // A press elsewhere with a menu open only closes it.
        let p = at(&e, centre("cutoff"));
        right_click(&mut e, p);
        let p = at(&e, centre("emphasis"));
        click(&mut e, p);
        assert!(!e.learning.menu_open());
        assert_eq!(params.cutoff.value(), 0.0);
        assert!(host.take().is_empty(), "no gesture");
    }

    /// Arming another control moves the learning there; a controller another control had moves
    /// to the one learned, and the note says from which.
    #[test]
    fn learning_another_control_moves_it_and_a_controller_reused_moves() {
        let (mut e, _host, params) = editing();
        let map = Arc::clone(&params.midi_map);
        map.assign(
            learnable("emphasis"),
            crate::learn::Cc { channel: 0, cc: 74 },
        );
        let p = at(&e, centre("cutoff"));
        right_click(&mut e, p);
        menu_item(&mut e, "MIDI LEARN");
        let p = at(&e, centre("glide"));
        right_click(&mut e, p);
        menu_item(&mut e, "MIDI LEARN");
        assert_eq!(map.armed(), Some(learnable("glide")));
        map.incoming(0, 74);
        e.draw();
        assert_eq!(
            note_lines(&e),
            [
                "GLIDE: CH 1 · CC 74",
                "TAKEN FROM EMPHASIS, WHICH HAS NONE NOW"
            ]
        );
        assert_eq!(map.assignment(learnable("emphasis")), None);
        assert_eq!(map.assignment(learnable("cutoff")), None);
    }

    /// Closing the editor ends learning: nothing assigned, unless the audio thread had already
    /// caught a controller for it (that message came, so it is the assignment).
    #[test]
    fn closing_the_editor_cancels_learning() {
        let (mut e, _host, params) = editing();
        let map = Arc::clone(&params.midi_map);
        let p = at(&e, centre("cutoff"));
        right_click(&mut e, p);
        menu_item(&mut e, "MIDI LEARN");
        drop(e);
        assert_eq!(map.armed(), None);
        assert!(map.assignments().iter().all(Option::is_none));
        let (mut e, _host, _) = editing();
        e.params = Arc::clone(&params);
        let p = at(&e, centre("cutoff"));
        right_click(&mut e, p);
        menu_item(&mut e, "MIDI LEARN");
        map.incoming(2, 20);
        drop(e);
        assert_eq!(map.armed(), None);
        assert_eq!(
            map.assignment(learnable("cutoff")),
            Some(crate::learn::Cc { channel: 2, cc: 20 })
        );
    }

    #[test]
    fn the_mod_wheel_learns_and_keeps_its_computer_source_without_parameter_gestures() {
        use crate::mod_wheel::Source;
        let dir = tempfile::tempdir().expect("computer settings");
        let path = dir.path().join("CA-72/settings.toml");
        let (mut e, host, params) = editing();
        e.settings_file = Some(path.clone());
        let fields = params.serialize_fields();
        let map = Arc::clone(&params.midi_map);
        let p = at(&e, centre("mod_wheel"));
        right_click(&mut e, p);
        assert_eq!(
            e.learning.menu().expect("wheel menu").title,
            "MOD WHEEL SOURCE · CC 1"
        );
        menu_item(&mut e, "MIDI LEARN");
        e.draw();
        assert_eq!(e.scene.learning, ca72_panel::controls::index("mod_wheel"));
        assert!(keyed(&mut e, Key::Escape));
        assert_eq!(map.armed(), None);
        right_click(&mut e, p);
        menu_item(&mut e, "MIDI LEARN");
        assert_eq!(map.incoming(7, 11), crate::learn::Incoming::Caught);
        e.draw();
        assert_eq!(map.wheel_source(), Source::Cc(11));
        assert_eq!(Settings::read(&path).mod_wheel_source, Source::Cc(11));
        assert_eq!(note_lines(&e), ["MOD WHEEL SOURCE: CC 11 · ALL CHANNELS"]);
        assert_eq!(
            params.serialize_fields(),
            fields,
            "source never enters project fields"
        );
        assert!(
            e.learning
                .list_scene(&map)
                .reserved
                .contains("CC 11 (MOD WHEEL SOURCE")
        );
        right_click(&mut e, p);
        menu_item(&mut e, "MIDI LEARN");
        map.incoming(2, 74);
        drop(e);
        assert_eq!(
            Settings::read(&path).mod_wheel_source,
            Source::Cc(74),
            "caught before closing"
        );
        assert_eq!(map.armed(), None);
        assert!(host.take().is_empty(), "source is not a sound parameter");
    }

    #[test]
    fn settings_and_the_wheel_offer_common_sources_and_every_other_allowed_cc() {
        use crate::mod_wheel::Source;
        let (mut e, host, params) = editing();
        let key = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Settings)));
        click(&mut e, key);
        menu_item(&mut e, "CC 1…");
        let m = e.learning.menu().expect("source menu");
        assert_eq!(m.marked, [0]);
        for label in [
            "CC 2 · BREATH",
            "CC 4 · FOOT",
            "CC 11 · EXPRESSION",
            "CC 74 · TIMBRE / MPE Y",
            "CHANNEL AFTERTOUCH",
            "POLYPHONIC AFTERTOUCH · MAX HELD",
        ] {
            assert!(
                m.items.iter().any(|(text, on)| text == label && *on),
                "{label}"
            );
        }
        menu_item(&mut e, "CC 11 · EXPRESSION");
        assert_eq!(params.midi_map.wheel_source(), Source::Cc(11));
        assert_eq!(e.settings.mod_wheel_source, Source::Cc(11));
        for label in ["CHANNEL AFTERTOUCH", "POLYPHONIC AFTERTOUCH · MAX HELD"] {
            let p = at(&e, centre("mod_wheel"));
            right_click(&mut e, p);
            menu_item(&mut e, "MOD WHEEL SOURCE…");
            menu_item(&mut e, label);
        }
        assert_eq!(params.midi_map.wheel_source(), Source::PolyPressure);
        let p = at(&e, centre("mod_wheel"));
        right_click(&mut e, p);
        menu_item(&mut e, "MOD WHEEL SOURCE…");
        menu_item(&mut e, "OTHER…");
        menu_item(&mut e, "CC 16–31…");
        menu_item(&mut e, "CC 23");
        assert_eq!(params.midi_map.wheel_source(), Source::Cc(23));
        for first in (0..128).step_by(16) {
            e.learning.open_wheel_menu(
                e.renderer.fonts(),
                settings_menu_corner(),
                e.renderer.text_size() * MENU_TEXT,
                Source::Cc(23),
                Some(first),
            );
            let m = e.learning.menu().expect("range");
            for (i, (_, enabled)) in m.items.iter().enumerate() {
                let cc = first + i as u8;
                assert_eq!(*enabled, Source::cc(cc).is_some() && cc != 23, "CC {cc}");
            }
        }
        assert!(host.take().is_empty());
    }

    #[test]
    #[ignore = "writes images for a look"]
    fn the_mod_wheel_source_pngs() {
        let out = std::env::var_os("CA72_MODWHEEL_PNG").expect("image folder");
        let out = std::path::PathBuf::from(out);
        std::fs::create_dir_all(&out).expect("image folder");
        for (skin_name, skin) in [("realistic", Skin::Worn), ("classic", Skin::Drawn)] {
            for (text_name, lettering) in [
                ("standard", Lettering::Standard),
                ("large", Lettering::Large),
            ] {
                let (mut e, _, _) = editing();
                e.set_look(skin);
                e.set_lettering(lettering);
                let key = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Settings)));
                click(&mut e, key);
                window_frame(&mut e)
                    .save_png(out.join(format!("settings-{skin_name}-{text_name}.png")))
                    .expect("settings image");
                menu_item(&mut e, "CC 1…");
                window_frame(&mut e)
                    .save_png(out.join(format!("sources-{skin_name}-{text_name}.png")))
                    .expect("source image");
                menu_item(&mut e, "OTHER…");
                menu_item(&mut e, "CC 0–15…");
                window_frame(&mut e)
                    .save_png(out.join(format!("range-{skin_name}-{text_name}.png")))
                    .expect("CC range image");
            }
        }
    }

    /// The strip's controls have the menu too (a tab's above it, the control ringed on the
    /// strip; a knob's as the panel's knobs'); PITCH and POWER say why they are not learned,
    /// MIDI LEARN not offered.
    #[test]
    fn the_strips_controls_learn_and_the_wheels_and_power_say_why_not() {
        let (mut e, host, params) = editing();
        let map = Arc::clone(&params.midi_map);
        let p = on_strip(&e, strip_at(StripTarget::Tab(Bank::Mode, 1)));
        right_click(&mut e, p);
        let m = e.learning.menu().expect("a menu").clone();
        assert_eq!(m.title, "POLY · NO MIDI CONTROLLER");
        let (_, h) = m.extent(e.renderer.fonts());
        assert!(m.y >= 0.0 && m.y + h <= art::H + 1e-6, "inside the window");
        menu_item(&mut e, "MIDI LEARN");
        e.draw();
        assert_eq!(e.strip_scene.learning, Some(strip::StripControl::Poly));
        assert_eq!(e.scene.learning, None);
        let p = at(&e, centre("spread"));
        right_click(&mut e, p);
        assert_eq!(
            e.learning.menu().map(|m| m.title.as_str()),
            Some("WIDTH · NO MIDI CONTROLLER")
        );
        let p = on_strip(&e, strip_at(StripTarget::Tab(Bank::Mode, 1)));
        right_click(&mut e, p);
        menu_item(&mut e, "CANCEL MIDI LEARN");
        assert_eq!(map.armed(), None);
        // INNER's knob (the strip's last): its menu where it was pressed, MIDI LEARN rings it as
        // the panel's knobs are rung, and its note says so over it.
        let inner = ca72_panel::controls::index("inner").expect("a control");
        let p = at(&e, centre("inner"));
        right_click(&mut e, p);
        let m = e.learning.menu().expect("a menu").clone();
        assert_eq!(m.title, "INNER · NO MIDI CONTROLLER");
        let (_, h) = m.extent(e.renderer.fonts());
        assert!((m.x - centre("inner").0).abs() < 1e-6, "at INNER's knob");
        assert!(m.y >= 0.0 && m.y + h <= art::H + 1e-6, "inside the window");
        menu_item(&mut e, "MIDI LEARN");
        assert_eq!(map.armed(), Some(learnable("inner")));
        e.draw();
        assert_eq!(
            (e.scene.learning, e.strip_scene.learning),
            (Some(inner), None)
        );
        assert_eq!(note_lines(&e)[0], "MIDI LEARN: INNER");
        let p = at(&e, centre("inner"));
        right_click(&mut e, p);
        menu_item(&mut e, "CANCEL MIDI LEARN");
        assert_eq!(map.armed(), None);
        for (param, title) in [
            ("pitch_wheel", "PITCH: MIDI PITCH BEND MOVES IT"),
            ("quality", "QUALITY: SET PER PLUGIN INSTANCE"),
        ] {
            let p = at(&e, centre(param));
            right_click(&mut e, p);
            assert_eq!(e.learning.menu().map(|m| m.title.as_str()), Some(title));
            // (QUALITY's has its indicator's two first: `quality_s_menu_shows_or_hides_its_
            // indicator`.)
            let items = menu_items(&e);
            assert_eq!(
                items[items.len() - 2..],
                [
                    ("NOT LEARNED BY MIDI LEARN".to_owned(), false),
                    ("MIDI ASSIGNMENTS…".to_owned(), true),
                ]
            );
            assert_eq!(items.len(), if param == "quality" { 4 } else { 2 });
            // (What cannot be chosen leaves the menu open.)
            menu_item(&mut e, "NOT LEARNED BY MIDI LEARN");
            assert!(e.learning.menu_open());
        }
        let p = at(&e, (art::COL + POWER.0, art::TOP + POWER.1));
        right_click(&mut e, p);
        assert_eq!(
            e.learning.menu().map(|m| m.title.as_str()),
            Some("POWER: THE HOST'S BYPASS")
        );
        assert!(keyed(&mut e, Key::Escape));
        assert!(!e.learning.menu_open());
        assert!(host.take().is_empty(), "no gesture");
    }

    /// The drawer's MIDI list, from its MIDI button or a control's MIDI ASSIGNMENTS…: every
    /// learnable control and its controller, operated from the keyboard (the arrows choose,
    /// Enter learns or cancels, Delete removes, a letter finds a control, Escape cancels then
    /// closes) and by the pointer (a row's LEARN).
    #[test]
    fn the_drawers_midi_list_is_operated_from_the_keyboard() {
        let (mut e, host, params) = editing();
        let map = Arc::clone(&params.midi_map);
        open(&mut e);
        let p = in_drawer(&e, presets_ui::midi_centre());
        click(&mut e, p);
        assert!(e.learning.list);
        e.draw();
        let list = e.browser.drawer.midi.clone().expect("the list shown");
        // (Every learnable control but the effects' pins, the standalone's with effects alone:
        // R-FX-PARAMS.)
        assert_eq!(
            list.rows.len(),
            crate::learn::LEARNABLE.len() - crate::lfo::FX_PINS
        );
        assert_eq!(list.rows.len(), crate::learn::listed());
        assert_eq!(list.rows[0].name, "TUNE");
        assert!(keyed(&mut e, Key::ArrowDown));
        assert!(keyed(&mut e, Key::ArrowDown));
        assert_eq!(e.learning.chosen(), learnable("glide"));
        assert!(keyed(&mut e, Key::Enter));
        assert_eq!(map.armed(), Some(learnable("glide")));
        e.draw();
        assert!(
            e.browser
                .drawer
                .midi
                .as_ref()
                .is_some_and(|l| l.rows[2].waiting)
        );
        assert!(keyed(&mut e, Key::Escape), "Escape cancels first");
        assert_eq!(map.armed(), None);
        assert!(e.browser.open);
        map.assign(learnable("glide"), crate::learn::Cc { channel: 0, cc: 5 });
        e.draw();
        assert_eq!(
            e.browser
                .drawer
                .midi
                .as_ref()
                .map(|l| l.rows[2].assignment.as_str()),
            Some("CH 1 · CC 5")
        );
        assert!(keyed(&mut e, Key::Delete));
        assert_eq!(map.assignment(learnable("glide")), None);
        assert!(keyed(&mut e, Key::Character("c".into())));
        assert_eq!(e.learning.chosen(), learnable("cutoff"));
        e.draw();
        let list = e.browser.drawer.midi.clone().expect("the list");
        assert!(list.first <= list.chosen && list.chosen < list.first + presets_ui::ROWS_SHOWN);
        // LEARN on a row, by the pointer.
        let p = presets_ui::midi_action_centre(&list, list.chosen, presets_ui::MidiAction::Learn)
            .expect("the chosen row's LEARN");
        let p = in_drawer(&e, p);
        click(&mut e, p);
        assert_eq!(map.armed(), Some(learnable("cutoff")));
        assert!(keyed(&mut e, Key::Escape));
        assert!(keyed(&mut e, Key::Escape), "then Escape closes");
        assert!(!e.browser.open);
        slid(&mut e);
        assert_eq!(e.browser.drawer.midi, None, "the presets next time");
        // From a control's menu: the list, that control chosen.
        let p = at(&e, centre("emphasis"));
        right_click(&mut e, p);
        menu_item(&mut e, "MIDI ASSIGNMENTS…");
        assert!(e.browser.open && e.learning.list);
        assert_eq!(e.learning.chosen(), learnable("emphasis"));
        assert!(host.take().is_empty(), "no gesture");
    }

    /// A right click mid-drag ends the drag's gesture before the menu opens.
    #[test]
    fn a_right_click_ends_a_drag_first() {
        let (mut e, host, _params) = editing();
        let p = at(&e, centre("cutoff"));
        go(&mut e, p);
        press(&mut e);
        go(&mut e, below(p, -40.0));
        right_click(&mut e, p);
        assert!(e.learning.menu_open());
        well_formed(&host.take());
    }

    #[test]
    fn a_drag_is_one_gesture_of_its_knob() {
        let (mut e, host, params) = editing();
        let p = at(&e, centre("cutoff"));
        go(&mut e, p);
        e.mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            modifiers: Modifiers::empty(),
        });
        go(
            &mut e,
            Point {
                x: p.x,
                y: p.y - 2.0,
            },
        );
        go(
            &mut e,
            Point {
                x: p.x,
                y: p.y - 60.0,
            },
        );
        go(
            &mut e,
            Point {
                x: p.x,
                y: p.y - 300.0,
            },
        );
        e.mouse(MouseEvent::ButtonReleased {
            button: MouseButton::Left,
            modifiers: Modifiers::empty(),
        });
        let cutoff = params.cutoff.as_ptr();
        assert_eq!(
            host.take(),
            vec![
                Call::Begin(cutoff),
                Call::Set(cutoff, (0.5 + 2.0 / 240.0) as f32),
                Call::Set(cutoff, 0.75),
                Call::Set(cutoff, 1.0),
                Call::End(cutoff)
            ]
        );
        // A knob of the strip's, INNER's, the same (from its 0).
        let p = at(&e, centre("inner"));
        go(&mut e, p);
        press(&mut e);
        go(&mut e, below(p, -2.0));
        go(&mut e, below(p, -60.0));
        go(&mut e, below(p, -300.0));
        release(&mut e);
        let inner = params.inner.as_ptr();
        assert_eq!(
            host.take(),
            vec![
                Call::Begin(inner),
                Call::Set(inner, (2.0 / 240.0) as f32),
                Call::Set(inner, 0.25),
                Call::Set(inner, 1.0),
                Call::End(inner)
            ]
        );
    }

    #[test]
    fn clicks_flip_switches_pick_legends_and_turn_power() {
        let (mut e, host, params) = editing();
        {
            let p = at(&e, centre("osc1_on"));
            click(&mut e, p);
        }
        let on = params.osc1_on.as_ptr();
        assert_eq!(
            host.take(),
            vec![Call::Begin(on), Call::Set(on, 0.0), Call::End(on)]
        );

        let i = ca72_panel::controls::index("osc1_range").expect("a control");
        let Kind::Selector(set) = CONTROLS[i].kind else {
            panic!("a selector")
        };
        let (cx, cy) = CONTROLS[i].centre();
        let (lx, ly) = e.renderer.layout().legend_at(set, 0);
        {
            let p = at(&e, (cx + lx, cy + ly));
            click(&mut e, p);
        }
        let range = params.osc1_range.as_ptr();
        assert_eq!(
            host.take(),
            vec![Call::Begin(range), Call::Set(range, 0.0), Call::End(range)]
        );

        {
            let p = at(&e, (art::COL + POWER.0, art::TOP + POWER.1));
            click(&mut e, p);
        }
        let bypass = params.bypass.as_ptr();
        assert_eq!(
            host.take(),
            vec![
                Call::Begin(bypass),
                Call::Set(bypass, 1.0),
                Call::End(bypass)
            ]
        );
    }

    #[test]
    fn a_double_click_restores_the_default() {
        let (mut e, host, params) = editing();
        let p = at(&e, centre("osc2_frequency"));
        click(&mut e, p);
        host.take();
        click(&mut e, p);
        let f = params.osc2_frequency.as_ptr();
        assert_eq!(
            host.take(),
            vec![Call::Begin(f), Call::Set(f, 0.5), Call::End(f)]
        );
    }

    #[test]
    fn the_pitch_wheel_settles_in_its_detent() {
        let (mut e, host, params) = editing();
        let p = at(&e, centre("pitch_wheel"));
        go(&mut e, p);
        e.mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            modifiers: Modifiers::empty(),
        });
        go(
            &mut e,
            Point {
                x: p.x,
                y: p.y - 5.0,
            },
        );
        e.mouse(MouseEvent::ButtonReleased {
            button: MouseButton::Left,
            modifiers: Modifiers::empty(),
        });
        let w = params.pitch_wheel.as_ptr();
        let calls = host.take();
        assert_eq!(calls.first(), Some(&Call::Begin(w)));
        assert_eq!(
            &calls[calls.len() - 2..],
            &[Call::Set(w, 0.5), Call::End(w)]
        );
    }

    #[test]
    fn the_mouse_wheel_turns_a_knob_until_it_rests() {
        let (mut e, host, params) = editing();
        {
            let p = at(&e, centre("emphasis"));
            go(&mut e, p);
        }
        for _ in 0..2 {
            e.mouse(MouseEvent::WheelScrolled {
                delta: ScrollDelta::Lines { x: 0.0, y: 1.0 },
                modifiers: Modifiers::empty(),
            });
        }
        let emphasis = params.emphasis.as_ptr();
        assert_eq!(
            host.take(),
            vec![
                Call::Begin(emphasis),
                Call::Set(emphasis, 0.02),
                Call::Set(emphasis, 0.04)
            ]
        );
        if let Some(s) = &mut e.scroll {
            s.last -= SCROLL_REST * 2;
        }
        e.draw();
        assert_eq!(host.take(), vec![Call::End(emphasis)]);
    }

    /// Every input event reaches the host before a frame is drawn, including travel below
    /// one logical pixel on a Retina display. The distance and Shift's rate stay the same.
    #[test]
    fn knob_resolution_keeps_every_subpixel_drag_value() {
        let mut counts = Vec::new();
        for (width, dpr) in [(860, 1.0), (1720, 1.0), (860, 2.0), (1720, 2.0)] {
            for fine in [false, true] {
                let (mut e, host, params) = editing();
                e.fit(dpr, width, (f64::from(width) * dpr) as u32);
                let p = at(&e, centre("cutoff"));
                let modifiers = if fine {
                    Modifiers::SHIFT
                } else {
                    Modifiers::empty()
                };
                go(&mut e, p);
                e.mouse(MouseEvent::ButtonPressed {
                    button: MouseButton::Left,
                    modifiers,
                });
                for n in 1..=384 {
                    e.mouse(MouseEvent::CursorMoved {
                        position: below(p, -f64::from(n) / 16.0),
                        modifiers,
                    });
                }
                release(&mut e);
                let calls = host.take();
                well_formed(&calls);
                let values: Vec<_> = calls
                    .iter()
                    .filter_map(|c| match *c {
                        Call::Set(param, v) if param == params.cutoff.as_ptr() => Some(v),
                        _ => None,
                    })
                    .collect();
                assert!(values.windows(2).all(|v| v[1] > v[0]));
                let expected = 0.5 + if fine { 0.01 } else { 0.1 };
                assert!((f64::from(*values.last().expect("drag values")) - expected).abs() < 1e-6);
                println!(
                    "drag: width {width}, scale {dpr}, fine {fine}: {}/384 distinct values",
                    values.len()
                );
                counts.push(values.len());
            }
        }
        assert_eq!(counts, vec![384; 8]);
    }

    #[test]
    fn knob_resolution_keeps_every_pixel_scroll_value() {
        let mut counts = Vec::new();
        for fine in [false, true] {
            for sign in [-1.0, 1.0] {
                let (mut e, host, params) = editing();
                let p = at(&e, centre("cutoff"));
                go(&mut e, p);
                for _ in 0..384 {
                    e.mouse(MouseEvent::WheelScrolled {
                        delta: ScrollDelta::Pixels {
                            x: 0.0,
                            y: sign * 0.25,
                        },
                        modifiers: if fine {
                            Modifiers::SHIFT
                        } else {
                            Modifiers::empty()
                        },
                    });
                }
                e.end_scroll();
                let calls = host.take();
                well_formed(&calls);
                let values: Vec<_> = calls
                    .iter()
                    .filter_map(|c| match *c {
                        Call::Set(param, v) if param == params.cutoff.as_ptr() => Some(v),
                        _ => None,
                    })
                    .collect();
                assert!(values.windows(2).all(|v| (v[1] - v[0]) * sign > 0.0));
                println!(
                    "pixel scroll: fine {fine}, direction {sign}: {}/384 distinct values, final {:?}",
                    values.len(),
                    values.last()
                );
                counts.push(values.len());
                // The same sensitivity as a 40-point notch, with no discarded travel.
                let expected = 0.5 + f64::from(sign) * 2.4 * if fine { 0.002 } else { 0.02 };
                assert!(
                    (f64::from(*values.last().expect("scroll values")) - expected).abs() < 1e-6
                );
            }
        }
        assert_eq!(counts, vec![384; 4]);
    }

    #[test]
    fn knob_resolution_keeps_fractional_lines_excess_travel_and_direction() {
        let (mut e, host, params) = editing();
        // (Lines as Windows reads them, as far as they go: R-KNOB-GLIDE.)
        e.precise_lines = true;
        let p = at(&e, centre("cutoff"));
        go(&mut e, p);
        for _ in 0..384 {
            wheel(&mut e, 1.0 / 64.0);
        }
        let values: Vec<_> = host
            .take()
            .into_iter()
            .filter_map(|c| match c {
                Call::Set(_, v) => Some(v),
                _ => None,
            })
            .collect();
        assert_eq!(values.len(), 384);
        assert!(values.windows(2).all(|v| v[1] > v[0]));
        assert!(
            (values[383] - 0.62).abs() < 1e-6,
            "six lines, not 384 notches"
        );
        // A large event accounts for its whole distance; reversal works even at a limit.
        wheel(&mut e, 30.0);
        wheel(&mut e, -0.125);
        assert_eq!(
            host.take(),
            vec![
                Call::Set(params.cutoff.as_ptr(), 1.0),
                Call::Set(params.cutoff.as_ptr(), 0.9975)
            ]
        );
        e.end_scroll();
        host.take();
        // No vertical motion does not begin a gesture, including horizontal-only events.
        e.scrolled(ScrollDelta::Pixels { x: 5.0, y: 0.0 }, Modifiers::empty());
        assert!(host.take().is_empty());
    }

    #[test]
    fn knob_resolution_leaves_discrete_controls_at_their_positions() {
        let (mut e, host, _) = editing_poly();
        // (Lines as Windows reads them, fractions too: R-KNOB-GLIDE.)
        e.precise_lines = true;
        for (id, positions) in [
            ("voices", 8.0),
            ("osc1_range", 5.0),
            ("osc1_waveform", 5.0),
            ("quality", 2.0),
            ("osc1_on", 1.0),
        ] {
            let p = at(&e, centre(id));
            go(&mut e, p);
            // Tiny drags do not turn a selector into a continuous control.
            press(&mut e);
            for n in 1..=384 {
                go(&mut e, below(p, -f64::from(n) / 4.0));
            }
            release(&mut e);
            let drag_calls = host.take();
            well_formed(&drag_calls);
            for call in drag_calls {
                if let Call::Set(_, v) = call {
                    assert!(
                        (v * positions - (v * positions).round()).abs() < 1e-6,
                        "{id}: {v}"
                    );
                }
            }
            go(&mut e, p);
            let places = f64::from(positions);
            let mut position =
                (e.value_of(ca72_panel::controls::index(id).expect("control")) * places).round();
            for _ in 0..159 {
                e.scrolled(ScrollDelta::Pixels { x: 0.0, y: 0.25 }, Modifiers::SHIFT);
            }
            let mut calls = host.take();
            assert!(
                calls.iter().all(|c| !matches!(c, Call::Set(..))),
                "{id}: no partial step"
            );
            e.scrolled(ScrollDelta::Pixels { x: 0.0, y: 0.25 }, Modifiers::SHIFT);
            position = (position + 1.0).min(places);
            assert_eq!(e.scroll.as_ref().expect("scroll").value, position / places);
            // Multiple notches and their excess, then reverse a fraction left over.
            e.scrolled(ScrollDelta::Pixels { x: 0.0, y: -90.0 }, Modifiers::empty());
            position = (position - 2.0).max(0.0);
            assert_eq!(e.scroll.as_ref().expect("scroll").value, position / places);
            e.scrolled(ScrollDelta::Pixels { x: 0.0, y: -30.0 }, Modifiers::empty());
            position = (position - 1.0).max(0.0);
            assert_eq!(e.scroll.as_ref().expect("scroll").value, position / places);
            wheel(&mut e, 0.75);
            wheel(&mut e, -0.5);
            assert_eq!(e.scroll.as_ref().expect("scroll").value, position / places);
            wheel(&mut e, -0.5);
            position = (position - 1.0).max(0.0);
            assert_eq!(e.scroll.as_ref().expect("scroll").value, position / places);
            e.end_scroll();
            calls.extend(host.take());
            well_formed(&calls);
            assert!(
                calls.iter().any(|c| matches!(c, Call::Set(..))),
                "{id}: whole steps move"
            );
            for call in calls {
                if let Call::Set(_, v) = call {
                    assert!(
                        (v * positions - (v * positions).round()).abs() < 1e-6,
                        "{id}: {v}"
                    );
                }
            }
        }
    }

    #[test]
    fn knob_resolution_rebases_shift_during_a_drag_without_a_jump() {
        let (mut e, host, params) = editing();
        let p = at(&e, centre("cutoff"));
        go(&mut e, p);
        press(&mut e);
        go(&mut e, below(p, -24.0));
        for dy in [-24.0, -48.0] {
            e.mouse(MouseEvent::CursorMoved {
                position: below(p, dy),
                modifiers: Modifiers::SHIFT,
            });
        }
        for dy in [-48.0, -72.0] {
            go(&mut e, below(p, dy));
        }
        release(&mut e);
        let cutoff = params.cutoff.as_ptr();
        assert_eq!(
            host.take(),
            vec![
                Call::Begin(cutoff),
                Call::Set(cutoff, 0.6),
                Call::Set(cutoff, 0.61),
                Call::Set(cutoff, 0.71),
                Call::End(cutoff)
            ]
        );
    }

    /// A trackpad's or a Magic Mouse's points turn a knob as far as they travel (a notch's 2 %
    /// for every 40 points, a fraction for a fraction; issue #107), in one gesture; VOICES, a
    /// stepped knob, steps once a notch's travel has gathered.
    #[test]
    fn a_trackpad_turns_a_knob_in_proportion_and_voices_by_steps() {
        let (mut e, host, params) = editing_poly();
        let pixels = |e: &mut Editing, y: f32| {
            e.mouse(MouseEvent::WheelScrolled {
                delta: ScrollDelta::Pixels { x: 0.0, y },
                modifiers: Modifiers::empty(),
            });
        };
        let p = at(&e, centre("emphasis"));
        go(&mut e, p);
        for y in [4.0, 6.0, 30.0, -10.0] {
            pixels(&mut e, y);
        }
        let emphasis = params.emphasis.as_ptr();
        let calls = host.take();
        assert_eq!(calls.first(), Some(&Call::Begin(emphasis)));
        let set: Vec<f32> = calls
            .iter()
            .filter_map(|c| match c {
                Call::Set(p, v) if *p == emphasis => Some(*v),
                _ => None,
            })
            .collect();
        let want = [0.002, 0.005, 0.02, 0.015];
        assert_eq!(set.len(), want.len(), "{calls:?}");
        for (got, want) in set.iter().zip(want) {
            assert!((got - want).abs() < 1e-6, "{set:?}");
        }
        if let Some(s) = &mut e.scroll {
            s.last -= SCROLL_REST * 2;
        }
        e.draw();
        assert_eq!(host.take(), vec![Call::End(emphasis)]);
        // VOICES: 30 points nothing; 15 more, a voice.
        let p = at(&e, centre("voices"));
        go(&mut e, p);
        let before = params.voices.unmodulated_normalized_value();
        pixels(&mut e, 30.0);
        let voices = params.voices.as_ptr();
        assert_eq!(host.take(), vec![Call::Begin(voices)]);
        pixels(&mut e, 15.0);
        let calls = host.take();
        assert_eq!(calls.len(), 1, "{calls:?}");
        assert!(
            matches!(calls[0], Call::Set(p, v) if p == voices && (v - before - 0.125).abs() < 1e-6),
            "{calls:?} from {before}"
        );
    }

    /// What a scroll is as baseview gives it on each system (issue #107): macOS's points (a
    /// precise device) as they come, 40 to a notch; a line a notch whatever its size (a wheel's,
    /// X11's and macOS's, fractions too); on Windows lines as far as they go, a line a notch,
    /// fractions (a precision touchpad's) and several at once (a fast wheel's) alike.
    #[test]
    fn scrolls_read_as_notches_or_travel_as_each_system_reports_them() {
        let px = |y| ScrollDelta::Pixels { x: 0.0, y };
        let lines = |y| ScrollDelta::Lines { x: 0.0, y };
        for windows in [false, true] {
            assert_eq!(scroll_travel(px(10.0), windows), 10.0);
            assert_eq!(scroll_travel(px(-80.0), windows), -80.0);
            assert_eq!(scroll_travel(lines(1.0), windows), 40.0);
            assert_eq!(scroll_travel(lines(-1.0), windows), -40.0);
            assert_eq!(scroll_travel(lines(0.0), windows), 0.0);
        }
        for (y, elsewhere, windows) in [
            (0.125, 40.0, 5.0),
            (-0.25, -40.0, -10.0),
            (3.0, 40.0, 120.0),
            (-1.5, -40.0, -60.0),
        ] {
            assert_eq!(scroll_travel(lines(y), false), elsewhere, "{y}");
            assert_eq!(scroll_travel(lines(y), true), windows, "{y}");
        }
        assert_eq!(PRECISE_LINES, cfg!(windows));
        // A slow turn of a Mac's wheel, a tenth of a line an event, turns a knob a notch an
        // event, as before.
        let (mut e, host, params) = editing();
        e.precise_lines = false;
        let p = at(&e, centre("cutoff"));
        go(&mut e, p);
        for _ in 0..3 {
            wheel(&mut e, 0.1);
        }
        let cutoff = params.cutoff.as_ptr();
        assert_eq!(
            host.take(),
            vec![
                Call::Begin(cutoff),
                Call::Set(cutoff, 0.52),
                Call::Set(cutoff, 0.54),
                Call::Set(cutoff, 0.56)
            ]
        );
    }

    /// A wheel's gesture waits for the wheel to rest, and a press ends it before beginning
    /// its own, on that control or another: never a begin inside a gesture, which CLAP
    /// forbids (decisions.md R18).
    #[test]
    fn a_wheels_gesture_ends_before_another_begins() {
        let (mut e, host, params) = editing_poly();
        let cutoff = params.cutoff.as_ptr();
        let p = at(&e, centre("cutoff"));
        go(&mut e, p);
        wheel(&mut e, 1.0);
        // CUTOFF grabbed within the wheel's rest.
        press(&mut e);
        go(&mut e, below(p, -60.0));
        release(&mut e);
        let calls = host.take();
        well_formed(&calls);
        assert_eq!(
            (calls[0], calls[2], calls[3], calls[calls.len() - 1]),
            (
                Call::Begin(cutoff),
                Call::End(cutoff),
                Call::Begin(cutoff),
                Call::End(cutoff)
            ),
            "{calls:?}"
        );
        // A wheel on one knob, then a switch clicked; then VOICES stepped by the wheel.
        let p = at(&e, centre("emphasis"));
        go(&mut e, p);
        wheel(&mut e, 1.0);
        let p = at(&e, centre("osc1_on"));
        click(&mut e, p);
        let p = at(&e, centre("emphasis"));
        go(&mut e, p);
        wheel(&mut e, 1.0);
        let p = at(&e, centre("voices"));
        go(&mut e, p);
        wheel(&mut e, 1.0);
        e.end_gestures();
        let calls = host.take();
        well_formed(&calls);
        assert_eq!(
            calls.iter().filter(|c| matches!(c, Call::Begin(_))).count(),
            4
        );
    }

    /// The editor closed within a wheel's rest, or mid-drag: every gesture ends.
    #[test]
    fn closing_the_editor_ends_every_gesture() {
        let (mut e, host, params) = editing();
        let p = at(&e, centre("emphasis"));
        go(&mut e, p);
        wheel(&mut e, 1.0);
        drop(e);
        let calls = host.take();
        well_formed(&calls);
        assert_eq!(calls.last(), Some(&Call::End(params.emphasis.as_ptr())));

        let (mut e, host, _params) = editing();
        let p = at(&e, centre("cutoff"));
        go(&mut e, p);
        press(&mut e);
        go(&mut e, below(p, -60.0));
        drop(e);
        well_formed(&host.take());

        let (mut e, host, _params) = editing();
        let p = at(&e, centre("spread"));
        go(&mut e, p);
        press(&mut e);
        go(&mut e, below(p, -40.0));
        drop(e);
        well_formed(&host.take());
    }

    /// A drag whose release never came (on Windows its window lost the pointer's capture,
    /// at Alt-Tab) ends as the next press comes, and as the window loses the focus or the
    /// capture, picking nothing.
    #[test]
    fn a_drag_whose_release_was_lost_ends_at_the_next_press() {
        let (mut e, host, params) = editing();
        let (cutoff, emphasis) = (params.cutoff.as_ptr(), params.emphasis.as_ptr());
        let p = at(&e, centre("cutoff"));
        go(&mut e, p);
        press(&mut e);
        go(&mut e, below(p, -60.0));
        // No release: EMPHASIS pressed next.
        let p = at(&e, centre("emphasis"));
        go(&mut e, p);
        press(&mut e);
        release(&mut e);
        let calls = host.take();
        well_formed(&calls);
        let at_end = calls.iter().position(|c| *c == Call::End(cutoff));
        let at_begin = calls.iter().position(|c| *c == Call::Begin(emphasis));
        assert!(at_end < at_begin && at_end.is_some(), "{calls:?}");
        // RANGE pressed (a click would step it), cut short by the window: nothing picked.
        let range = params.osc1_range.as_ptr();
        let p = at(&e, centre("osc1_range"));
        go(&mut e, p);
        press(&mut e);
        assert!(e.dragging());
        e.end_gestures();
        assert!(!e.dragging());
        release(&mut e);
        assert_eq!(host.take(), vec![Call::Begin(range), Call::End(range)]);
    }

    /// The name plate does nothing (the owner, 2026-10-03: it no longer turns over): a click
    /// on it sets nothing, redraws nothing and shows no tip.
    #[test]
    fn the_name_plate_does_nothing() {
        let (mut e, host, _params) = editing();
        e.draw();
        let p = at(&e, (art::PLATE_X + 20.0, art::PLATE_Y + 20.0));
        click(&mut e, p);
        assert_eq!(host.take(), vec![]);
        assert!(!e.draw(), "redrawn");
        assert_eq!(e.scene.tip, None);
    }

    #[test]
    fn a_tip_names_the_control_and_its_value() {
        let (mut e, _host, _params) = editing();
        {
            let p = at(&e, centre("osc1_volume"));
            go(&mut e, p);
        }
        e.update_scene();
        assert_eq!(
            e.scene.tip.as_ref().map(|t| t.0.as_str()),
            Some("OSCILLATOR-1 VOLUME: 8.00")
        );
        // A strip knob's, INNER's, in its unit: turned from 0 to 40 %.
        let p = at(&e, centre("inner"));
        go(&mut e, p);
        press(&mut e);
        go(&mut e, below(p, -96.0));
        e.update_scene();
        assert_eq!(
            e.scene.tip.as_ref().map(|t| t.0.as_str()),
            Some("INNER: 40 %")
        );
        release(&mut e);
    }

    /// RETRIGGER's explanation stays readable at the smallest window in either lettering.
    #[test]
    fn retrigger_tip_explains_priority_and_fits() {
        for skin in [Skin::Worn, Skin::Drawn] {
            for lettering in [Lettering::Standard, Lettering::Large] {
                for on in [false, true] {
                    let (mut e, _, _) = editing_of(
                        Ca72Params {
                            retrigger: BoolParam::new("Retrigger", on),
                            ..Ca72Params::default()
                        },
                        Library::at(std::env::temp_dir().join("ca72-editor-tests-no-library")),
                    );
                    e.set_look(skin);
                    e.set_lettering(lettering);
                    e.resize(MIN_WIDTH);
                    e.fit(1.0, MIN_WIDTH, height_for(MIN_WIDTH));
                    let p = at(&e, centre("retrigger"));
                    go(&mut e, p);
                    e.update_scene();
                    let (text, x, y) = e.tip().expect("RETRIGGER's tip");
                    assert!(text.starts_with(if on {
                        "RETRIGGER: On"
                    } else {
                        "RETRIGGER: Off"
                    }));
                    assert!(text.contains("On: each key strikes."));
                    assert!(text.contains("NOTE PRIORITY LAST: new pitch with attack"));
                    assert!(
                        text.contains("LOW/HIGH: a held key may keep the old pitch until lifted.")
                    );
                    let layer = art::tip(e.renderer.fonts(), x, y, &text, e.renderer.text_size());
                    let b = layer.bounds;
                    assert_eq!(layer.body.matches("<text ").count(), 3);
                    assert!(
                        x + b[0] >= 0.0 && x + b[2] <= art::W,
                        "{skin:?}, {lettering:?}: {b:?}"
                    );
                    assert!(y + b[1] >= e.renderer.top(), "tip above the window");
                }
            }
        }
    }

    #[test]
    fn every_control_operates_a_parameter() {
        let p = Ca72Params::default();
        for c in CONTROLS.iter() {
            assert!(param(&p, c.param).is_some(), "{}", c.param);
        }
    }

    #[test]
    fn the_editor_keeps_the_panels_proportions() {
        // The panel's 512 and the strip's 470 under it (A6, the name board gone: the drawing
        // 3108 by 1775).
        assert_eq!(height_for(1720), 982);
        assert_eq!(clamp_width(10), MIN_WIDTH);
        assert_eq!(clamp_width(99_999), MAX_WIDTH);
    }

    #[test]
    fn it_opens_at_four_fifths_of_the_screen_until_resized() {
        // (The official panel's: the process's chosen look is another test's to change.)
        let fitted_width = |screen| fitted_width_in(screen, art::Panel::OFFICIAL);
        // A 14-inch MacBook Pro's screen less the menu bar; a 34-inch ultrawide's.
        assert_eq!(fitted_width(Some((1512.0, 949.0))), 1210);
        // The height's share sets it on a wide screen (the panel and the strip under it).
        assert_eq!(fitted_width(Some((3440.0, 1400.0))), 1961);
        assert_eq!(fitted_width(Some((3840.0, 800.0))), 1121);
        assert_eq!(height_for(1121), 640);
        assert_eq!(fitted_width(None), DEFAULT_WIDTH);
        assert_eq!(
            Ca72Params::default().editor_width.load(Ordering::Relaxed),
            0
        );
    }

    /// X11's screen spans every monitor: the first size comes from the monitor the pointer
    /// is on (two 2560 by 1440 side by side made the editor 3435 wide on one, its grip off
    /// it), less the panels the work area leaves out.
    #[test]
    fn on_x11_it_opens_to_fit_the_monitor_it_is_on() {
        use screen::{Rect, monitor_area};
        let r = |x, y, w, h| Rect { x, y, w, h };
        let fitted_width = |screen| fitted_width_in(screen, art::Panel::OFFICIAL);
        let whole = r(0, 0, 5120, 1440);
        let (left, right) = (r(0, 0, 2560, 1440), r(2560, 0, 2560, 1440));
        let monitors = [(left, true), (right, false)];
        assert_eq!(fitted_width(Some((5120.0, 1440.0))), 2017);
        assert_eq!(
            monitor_area(whole, &monitors, Some((3000, 700)), None),
            right
        );
        assert_eq!(fitted_width(Some((2560.0, 1440.0))), 2017);
        // The pointer elsewhere: the primary; no RandR: the whole screen, as before.
        assert_eq!(monitor_area(whole, &monitors, None, None), left);
        assert_eq!(monitor_area(whole, &[], Some((3000, 700)), None), whole);
        // A panel along the screen's top; a work area that misses the monitor is ignored.
        let work = Some(r(0, 32, 5120, 1408));
        assert_eq!(
            monitor_area(whole, &monitors, Some((3000, 700)), work),
            r(2560, 32, 2560, 1408)
        );
        assert_eq!(
            monitor_area(whole, &monitors, Some((3000, 700)), Some(left)),
            right
        );
    }

    /// On Windows and Linux the window takes the host's scale or none, never the system's:
    /// the host frames it at nih-plug's size times its own scale (1 when it gave none).
    #[test]
    fn the_window_scales_as_the_host_says() {
        if cfg!(target_os = "macos") {
            assert_eq!(scale_policy(None), WindowScalePolicy::SystemScaleFactor);
        } else {
            assert_eq!(scale_policy(None), WindowScalePolicy::ScaleFactor(1.0));
            assert_eq!(scale_policy(Some(1.5)), WindowScalePolicy::ScaleFactor(1.5));
        }
    }

    /// The window is the drawing's proportions, its width as saved, the drawer open or shut
    /// (it drops down over the strip: A6).
    #[test]
    fn the_editor_opens_at_the_drawings_proportions() {
        let params = Arc::new(Ca72Params::default());
        params.editor_width.store(1720, Ordering::Relaxed);
        let editor = Ca72Editor::new(Arc::clone(&params), Arc::new(Meters::default()));
        let size = (1720, height_for(1720));
        assert_eq!(editor.size(), size);
        assert_eq!(editor.opening(), size);
        assert_eq!(size.1, (1720.0 * art::H / art::W).round() as u32);
    }

    /// A point of the strip (the drawing's units) in the window.
    fn on_strip(e: &Editing, p: (f64, f64)) -> Point {
        at(e, p)
    }

    /// Where a strip target is, found where the strip's hit test finds it.
    fn strip_at(target: StripTarget) -> (f64, f64) {
        let p = strip::centre(Rail::Plugin, target);
        assert_eq!(
            strip::hit(Rail::Plugin, p.0, p.1),
            Some(target),
            "{target:?} not found"
        );
        p
    }

    /// The strip's tabs and switches (A6): MODE's MONO, POLY and UNISON set POLY and UNISON;
    /// SCATTER | DOUBLE turns DETUNE off and back on at its amount (half way if it never was);
    /// the placement's EVEN and CENTER are its parameter's two choices; AUTO GAIN's tab toggles
    /// it; ENTROPY's, WIDTH's and INNER's readouts turn their amounts off and on; VOICES is a
    /// knob, a drag one gesture. Each click a gesture of its own for each parameter it changes,
    /// none for one already so.
    #[test]
    fn the_strip_operates_its_parameters() {
        let (mut e, host, params) = editing();
        let tab = |e: &mut Editing, b: Bank, i: usize| {
            let p = on_strip(e, strip_at(StripTarget::Tab(b, i)));
            click(e, p);
        };
        let (poly, unison) = (params.poly.as_ptr(), params.unison.as_ptr());
        tab(&mut e, Bank::Mode, 1);
        assert_eq!(
            host.take(),
            vec![Call::Begin(poly), Call::Set(poly, 1.0), Call::End(poly)]
        );
        tab(&mut e, Bank::Mode, 2);
        assert_eq!(
            host.take(),
            vec![
                Call::Begin(unison),
                Call::Set(unison, 1.0),
                Call::End(unison)
            ]
        );
        // (The test host applies nothing: POLY and UNISON are still off, so MONO is already.)
        tab(&mut e, Bank::Mode, 0);
        assert_eq!(host.take(), vec![], "MONO already");
        let doubled = params.doubled.as_ptr();
        tab(&mut e, Bank::Stereo, 1);
        assert_eq!(
            host.take(),
            vec![
                Call::Begin(doubled),
                Call::Set(doubled, 1.0),
                Call::End(doubled)
            ]
        );
        tab(&mut e, Bank::Stereo, 0);
        assert_eq!(
            host.take(),
            vec![],
            "SCATTER already (DOUBLE still off here)"
        );
        let placement = params.placement.as_ptr();
        tab(&mut e, Bank::Placement, 0);
        assert_eq!(host.take(), vec![], "EVEN already");
        tab(&mut e, Bank::Placement, 1);
        assert_eq!(
            host.take(),
            vec![
                Call::Begin(placement),
                Call::Set(placement, 1.0),
                Call::End(placement)
            ]
        );
        assert_eq!(
            params.placement.preview_plain(1.0),
            Scatter::Centre,
            "the second tab CENTER"
        );
        let auto = params.auto_gain.as_ptr();
        tab(&mut e, Bank::Auto, 0);
        assert_eq!(
            host.take(),
            vec![Call::Begin(auto), Call::Set(auto, 0.0), Call::End(auto)]
        );
        // ENTROPY starts at the old minimum, now 5 %: its first click turns it off.
        // WIDTH and INNER start off: their first click recalls half way (they never were).
        for (r, p, to) in [
            (Readout::Entropy, params.entropy.as_ptr(), 0.0),
            (Readout::Width, params.spread.as_ptr(), 0.5),
            (Readout::Inner, params.inner.as_ptr(), 0.5),
        ] {
            let at = on_strip(&e, strip_at(StripTarget::Switch(r)));
            click(&mut e, at);
            assert_eq!(
                host.take(),
                vec![Call::Begin(p), Call::Set(p, to), Call::End(p)]
            );
        }
        // VOICES's knob dragged up: nothing at these defaults, where it does nothing (MONO,
        // SCATTER, WIDTH 0: R64); with POLY on, one gesture of VOICES.
        let drag_voices = |e: &mut Editing| {
            let p = at(e, centre("voices"));
            go(e, p);
            press(e);
            go(e, below(p, -120.0));
            release(e);
        };
        drag_voices(&mut e);
        assert_eq!(host.take(), vec![], "VOICES dragged where it does nothing");
        let (mut poly_e, poly_host, poly_params) = editing_poly();
        let voices = poly_params.voices.as_ptr();
        drag_voices(&mut poly_e);
        let calls = poly_host.take();
        well_formed(&calls);
        assert_eq!(calls.first(), Some(&Call::Begin(voices)));
        assert_eq!(calls.last(), Some(&Call::End(voices)));
        // The strip draws what the parameters hold.
        e.update_scene();
        assert!(e.draw());
    }

    /// The strip's readouts that are switches turn their amounts off, keeping each, and back on
    /// at it: WIDTH's and INNER's each their own (INNER's switch the same as WIDTH's).
    #[test]
    fn the_strips_switches_turn_their_amounts_off_and_back_on_at_their_last() {
        let amount = |name: &str, percent: f32| {
            FloatParam::new(
                name,
                percent,
                FloatRange::Linear {
                    min: 0.0,
                    max: 100.0,
                },
            )
        };
        let (mut e, host, _) = editing();
        e.params = Arc::new(Ca72Params {
            spread: amount("Width", 70.0),
            inner: amount("Inner", 40.0),
            ..Ca72Params::default()
        });
        let switch = |e: &mut Editing, r: Readout| {
            let p = on_strip(e, strip_at(StripTarget::Switch(r)));
            click(e, p);
        };
        // Off, each kept.
        for (r, p) in [
            (Readout::Width, e.params.spread.as_ptr()),
            (Readout::Inner, e.params.inner.as_ptr()),
        ] {
            switch(&mut e, r);
            assert_eq!(
                host.take(),
                vec![Call::Begin(p), Call::Set(p, 0.0), Call::End(p)]
            );
        }
        // (The test host applies nothing: the parameters as the host would hold them now.)
        e.params = Arc::new(Ca72Params::default());
        // Back on, each at its own.
        for (r, p, was) in [
            (Readout::Inner, e.params.inner.as_ptr(), 0.4),
            (Readout::Width, e.params.spread.as_ptr(), 0.7),
        ] {
            switch(&mut e, r);
            assert_eq!(
                host.take(),
                vec![Call::Begin(p), Call::Set(p, was), Call::End(p)]
            );
        }
    }

    /// The strip lights the placement's tab by the plug-in's choice, EVEN the first and CENTER
    /// the second (not by the kit's index, which counts its EDGES), in MONO too, whose one
    /// voice scatters by it (R62); none in MONO with DOUBLE, whose pair has no place
    /// among others.
    #[test]
    fn the_strip_lights_the_placements_tab() {
        let lit = |poly: bool, doubled: bool, placement: Scatter| {
            let p = Ca72Params {
                poly: BoolParam::new("Poly", poly),
                doubled: BoolParam::new("Double", doubled),
                placement: EnumParam::new("Scatter Placement", placement),
                ..Ca72Params::default()
            };
            strip_scene(
                &p,
                0,
                (&[], 0.0),
                &[None; LIVE.len()],
                StripScene::default(),
            )
            .placement
        };
        assert_eq!(lit(true, false, Scatter::Even), Some(0));
        assert_eq!(lit(true, false, Scatter::Centre), Some(1));
        assert_eq!(lit(false, false, Scatter::Centre), Some(1));
        assert_eq!(lit(false, true, Scatter::Centre), None);
        assert_eq!(Bank::Placement.words(), ["EVEN", "CENTER"]);
    }

    /// In MONO without DOUBLE the strip shows VOICES's places as the engine's gains have them,
    /// the one voice lit at the place its last attack took (the engine's bit for it), at the
    /// output's level, with VOICES, WIDTH and INNER lit (R62); with DOUBLE, the one pair,
    /// and VOICES not lit.
    #[test]
    fn mono_shows_its_places_and_the_one_it_sounds_at() {
        let amount = |name: &str, percent: f32| {
            FloatParam::new(
                name,
                percent,
                FloatRange::Linear {
                    min: 0.0,
                    max: 100.0,
                },
            )
        };
        let scene = |doubled: bool| {
            let p = Ca72Params {
                spread: amount("Width", 100.0),
                inner: amount("Inner", 40.0),
                doubled: BoolParam::new("Double", doubled),
                ..Ca72Params::default()
            };
            strip_scene(
                &p,
                1 << 2,
                (&[], 0.5),
                &[None; LIVE.len()],
                StripScene::default(),
            )
        };
        let s = scene(false);
        let Field::Scatter(places) = &s.field else {
            panic!("{:?}", s.field);
        };
        let lit: Vec<usize> = (0..places.len()).filter(|&k| places[k].1).collect();
        assert_eq!((places.len(), lit), (4, vec![2]));
        for (k, &(x, _)) in places.iter().enumerate() {
            let want = place::voice_at(1.0, 0.4, place::Placement::Even, k, 4);
            assert_eq!(x, want, "place {k}");
        }
        assert_eq!(s.levels[2], 0.5);
        let shown = |s: &StripScene, r: Readout| {
            s.readouts[Readout::ALL.iter().position(|&x| x == r).unwrap()].1
        };
        for r in [Readout::Voices, Readout::Width, Readout::Inner] {
            assert!(shown(&s, r), "{r:?} not lit");
        }
        let d = scene(true);
        assert!(matches!(&d.field, Field::Double(pairs) if pairs.len() == 1));
        assert!(!shown(&d, Readout::Voices), "VOICES lit with DOUBLE");
        assert!(shown(&d, Readout::Width));
    }

    /// VOICES where it does nothing (R64): in MONO with DOUBLE, or scattering at WIDTH 0;
    /// it counts again with POLY, with UNISON, or with WIDTH up in MONO without DOUBLE (R62).
    /// The editor then draws its knob dimmed, takes no press, drag nor typed value for it, and
    /// leaves its readout unlit; as for DETUNE in SCATTER, the other knobs answer.
    #[test]
    fn voices_is_dimmed_where_it_does_nothing() {
        let params = |poly: bool, unison: bool, doubled: bool, width: f32| Ca72Params {
            poly: BoolParam::new("Poly", poly),
            unison: BoolParam::new("Unison", unison),
            doubled: BoolParam::new("Double", doubled),
            spread: FloatParam::new(
                "Width",
                width,
                FloatRange::Linear {
                    min: 0.0,
                    max: 100.0,
                },
            ),
            ..Ca72Params::default()
        };
        for (poly, unison, doubled, width, inert) in [
            (false, false, true, 100.0, true),
            (false, false, true, 0.0, true),
            (false, false, false, 0.0, true),
            (false, false, false, 100.0, false),
            (true, false, false, 0.0, false),
            (true, false, true, 0.0, false),
            (false, true, true, 0.0, false),
            (false, true, false, 100.0, false),
        ] {
            let p = params(poly, unison, doubled, width);
            assert_eq!(
                voices_inert(&p),
                inert,
                "POLY {poly}, UNISON {unison}, DOUBLE {doubled}, WIDTH {width}"
            );
            let s = strip_scene(
                &p,
                0,
                (&[], 0.0),
                &[None; LIVE.len()],
                StripScene::default(),
            );
            assert_eq!(s.readouts[0].1, !inert, "the readout lit");
        }
        // The editor, at the defaults (MONO, SCATTER, WIDTH 0).
        let (mut e, host, params) = editing();
        assert!(voices_inert(&params));
        e.draw();
        assert!(e.scene.voices_off);
        let voices = ca72_panel::controls::index("voices").expect("a control");
        let cutoff = ca72_panel::controls::index("cutoff").expect("a control");
        assert!(!e.operable(voices) && e.operable(cutoff));
        host.take();
        let pt = at(&e, centre("voices"));
        click(&mut e, pt);
        go(&mut e, pt);
        wheel(&mut e, 1.0);
        e.end_gestures();
        assert!(host.take().is_empty(), "VOICES took a press or the wheel");
        e.start_typing(Readout::Voices);
        assert!(e.typing.is_none());
        e.start_typing(Readout::Drive);
        assert!(e.typing.is_some());
    }

    /// GLIDE's menu selects a range in one host gesture, with its current choice marked. LONG
    /// reaches the scene and the tip; selecting NORMAL again removes the print and the suffix.
    #[test]
    fn the_glide_menu_sets_its_range_and_the_panel_and_tip_follow() {
        use crate::params::GlideRange;
        for range in [GlideRange::Normal, GlideRange::Long] {
            let (mut e, host, params) = editing_of(
                Ca72Params {
                    glide: FloatParam::new(
                        "Glide",
                        7.5,
                        FloatRange::Linear {
                            min: 0.0,
                            max: 10.0,
                        },
                    ),
                    glide_range: EnumParam::new("Glide Range", range),
                    ..Ca72Params::default()
                },
                Library::at(std::env::temp_dir().join("ca72-editor-tests-no-library")),
            );
            e.draw();
            let long = range == GlideRange::Long;
            assert_eq!(e.scene.glide_long, long);
            let point = at(&e, centre("glide"));
            e.hover = Some(Target::Control(
                ca72_panel::controls::index("glide").expect("GLIDE"),
            ));
            assert_eq!(
                e.tip().expect("a tip").0,
                if long {
                    "GLIDE: 7.5 · LONG"
                } else {
                    "GLIDE: 7.5"
                }
            );
            right_click(&mut e, point);
            let items = menu_items(&e);
            assert_eq!(
                &items[items.len() - 3..],
                &[
                    ("GLIDE RANGE".to_owned(), false),
                    ("NORMAL".to_owned(), long),
                    ("LONG (×5)".to_owned(), !long),
                ]
            );
            let m = e.learning.menu().expect("a menu");
            assert_eq!(m.heads, vec![items.len() - 3]);
            assert_eq!(m.marked, vec![items.len() - if long { 1 } else { 2 }]);
            host.take();
            menu_item(&mut e, if long { "NORMAL" } else { "LONG (×5)" });
            let p = params.glide_range.as_ptr();
            assert_eq!(
                host.take(),
                vec![
                    Call::Begin(p),
                    Call::Set(p, if long { 0.0 } else { 1.0 }),
                    Call::End(p)
                ]
            );
            // The test host records gestures; apply its automation as the real host does.
            let context = nih_plug::context::process::TestProcessContext::<crate::Ca72>::new(
                params.clone(),
                48_000.0,
                ProcessMode::Realtime,
            );
            assert!(context.automate(p, if long { 0.0 } else { 1.0 }));
            e.draw();
            assert_eq!(e.scene.glide_long, !long);
            assert_eq!(params.controls().panel.glide_long, !long);
        }
    }

    /// A WAVEFORM switch's menu (decisions.md R-CONTINUOUS-WAVE): a WAVEFORM head, STEPPED (the
    /// mode it is in, marked, not chosen again) and CONTINUOUS; CONTINUOUS chosen on WIDE puts the
    /// shape at WIDE's place exactly first, so the sound does not move, then the mode, each a
    /// gesture; oscillator 3's sawtooth goes to its reverse sawtooth's place.
    #[test]
    fn a_waveform_switchs_menu_makes_its_waveform_continuous() {
        use crate::params::{Wave, Wave3};
        let (mut e, host, params) = editing_of(
            Ca72Params {
                osc1_waveform: EnumParam::new("Osc 1 Waveform", Wave::WideRectangle),
                osc3_waveform: EnumParam::new("Osc 3 Waveform", Wave3::Sawtooth),
                ..Ca72Params::default()
            },
            Library::at(std::env::temp_dir().join("ca72-editor-tests-no-library")),
        );
        e.draw();
        let p = at(&e, centre("osc1_waveform"));
        right_click(&mut e, p);
        let items = menu_items(&e);
        let tail: Vec<(String, bool)> = items[items.len() - 3..].to_vec();
        assert_eq!(
            tail,
            [
                ("WAVEFORM".to_owned(), false),
                ("STEPPED".to_owned(), false),
                ("CONTINUOUS".to_owned(), true)
            ]
        );
        let m = e.learning.menu().expect("a menu").clone();
        assert_eq!(m.heads, vec![items.len() - 3]);
        assert_eq!(m.marked, vec![items.len() - 2]);
        host.take();
        menu_item(&mut e, "CONTINUOUS");
        let places = interact::CONTINUOUS_PLACES;
        let (shape, mode) = (params.osc1_shape.as_ptr(), params.osc1_shape_mode.as_ptr());
        assert_eq!(
            host.take(),
            vec![
                Call::Begin(shape),
                Call::Set(shape, (4.0 / places) as f32),
                Call::End(shape),
                Call::Begin(mode),
                Call::Set(mode, 1.0),
                Call::End(mode),
            ]
        );
        // (The engine takes that place as WIDE's exactly.)
        let s = ca72::voice::Shape {
            continuous: true,
            at: f64::from((4.0 / places) as f32) * places,
            modulated: false,
            direction: ca72::voice::ModDirection::Onward,
        };
        assert_eq!(ca72::voice::shape_at(s, Default::default()), 4.0);
        // Oscillator 3 on SAWTOOTH: its reverse sawtooth's place, CONTINUOUS's third.
        let p = at(&e, centre("osc3_waveform"));
        right_click(&mut e, p);
        menu_item(&mut e, "CONTINUOUS");
        let shape3 = params.osc3_shape.as_ptr();
        assert!(
            host.take()
                .contains(&Call::Set(shape3, (2.0 / places) as f32)),
            "oscillator 3's place"
        );
    }

    /// A CONTINUOUS switch (decisions.md R-CONTINUOUS-WAVE): its arc and MOD key on the panel, the
    /// others' not; dragged, it turns without steps, WAVEFORM naming the nearest position; a
    /// legend clicked lands on its place exactly; its tip says the shape; its MOD key flips MOD,
    /// Command-clicked turns MOD's way round (its colour following), a STEPPED oscillator's is
    /// not there at all.
    #[test]
    fn a_continuous_switch_turns_and_its_key_flips_mod_and_its_way() {
        use crate::params::{ShapeModDirection, Wave, WaveMode};
        let places = interact::CONTINUOUS_PLACES;
        let (mut e, host, params) = editing_of(
            Ca72Params {
                osc1_waveform: EnumParam::new("Osc 1 Waveform", Wave::Square),
                osc1_shape_mode: EnumParam::new("Osc 1 Waveform Mode", WaveMode::Continuous),
                ..Ca72Params::default()
            },
            Library::at(std::env::temp_dir().join("ca72-editor-tests-no-library")),
        );
        e.draw();
        let index = |p: &str| ca72_panel::controls::index(p).expect("a control");
        assert_eq!(e.scene.arcs(), [true, false, false]);
        // (A new shape at SAWTOOTH, as WAVEFORM begins.)
        assert!((e.scene.shape[0].at - 2.0).abs() < 1e-6);
        let (key1, key2) = (index("osc1_shape_mod"), index("osc2_shape_mod"));
        assert!(e.operable(key1) && !e.operable(key2));
        let (x, y) = centre("osc2_shape_mod");
        assert_ne!(
            e.hit(x, y),
            Some(Target::Control(key2)),
            "a hidden key found"
        );
        let (x, y) = centre("osc1_shape_mod");
        assert_eq!(e.hit(x, y), Some(Target::Control(key1)));
        let wave1 = index("osc1_waveform");
        e.hover = Some(Target::Control(wave1));
        assert_eq!(e.tip().expect("a tip").0, "OSCILLATOR-1 WAVEFORM: Sawtooth");
        // Dragged 12 pixels (0.4 of a step) from SAWTOOTH: there, WAVEFORM still SAWTOOTH.
        host.take();
        let p = at(&e, centre("osc1_waveform"));
        go(&mut e, p);
        press(&mut e);
        go(
            &mut e,
            Point {
                x: p.x + 12.0,
                y: p.y,
            },
        );
        release(&mut e);
        let calls = host.take();
        let (wave, shape) = (params.osc1_waveform.as_ptr(), params.osc1_shape.as_ptr());
        let last = |calls: &[Call], q| {
            calls
                .iter()
                .rev()
                .find_map(|c| match c {
                    Call::Set(p, v) if *p == q => Some(*v),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("not set: {calls:?}"))
        };
        assert!(
            (f64::from(last(&calls, shape)) * places - 2.4).abs() < 1e-5,
            "{calls:?}"
        );
        assert_eq!(
            last(&calls, wave),
            2.0 / 5.0,
            "WAVEFORM the nearest: {calls:?}"
        );
        // NARROW's legend clicked: NARROW's place exactly.
        let set = ca72_panel::controls::Legends::Waves12;
        let (lx, ly) = e.renderer.layout().legend_at(set, 5);
        let (cx, cy) = centre("osc1_waveform");
        let p = at(&e, (cx + lx, cy + ly));
        click(&mut e, p);
        let calls = host.take();
        assert_eq!(last(&calls, shape), (5.0 / places) as f32);
        // The key: MOD on, a gesture of its own; Command-clicked, its next way.
        let p = at(&e, centre("osc1_shape_mod"));
        click(&mut e, p);
        let m = params.osc1_shape_mod.as_ptr();
        assert_eq!(
            host.take(),
            vec![Call::Begin(m), Call::Set(m, 1.0), Call::End(m)]
        );
        let shortcut = if cfg!(target_os = "macos") {
            Modifiers::META
        } else {
            Modifiers::CONTROL
        };
        go(&mut e, p);
        e.mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            modifiers: shortcut,
        });
        e.mouse(MouseEvent::ButtonReleased {
            button: MouseButton::Left,
            modifiers: shortcut,
        });
        let dir = params.osc1_shape_mod_dir.as_ptr();
        assert_eq!(
            host.take(),
            vec![Call::Begin(dir), Call::Set(dir, 0.5), Call::End(dir)]
        );
        assert_eq!(
            params.osc1_shape_mod_dir.value(),
            ShapeModDirection::Clockwise,
            "(the test's host keeps no value)"
        );
        e.hover = Some(Target::Control(key1));
        assert_eq!(
            e.tip().expect("a tip").0,
            "OSCILLATOR-1 MOD, CLOCKWISE: Off"
        );
        // Oscillator 2's place under its switch takes nothing.
        let p = at(&e, centre("osc2_shape_mod"));
        click(&mut e, p);
        assert!(host.take().is_empty(), "the hidden key took a press");
    }

    /// A CONTINUOUS switch dragged slowly across its places (the owner, 2026-10-20: "It should
    /// just be one continuous smooth motion"): each half pixel moves the shape the host is given
    /// the same small way, through SAWTOOTH and SQUARE as between them.
    #[test]
    fn a_continuous_switch_dragged_slowly_moves_smoothly_through_its_places() {
        use crate::params::WaveMode;
        let places = interact::CONTINUOUS_PLACES;
        let (mut e, host, params) = editing_of(
            Ca72Params {
                osc1_shape_mode: EnumParam::new("Osc 1 Waveform Mode", WaveMode::Continuous),
                ..Ca72Params::default()
            },
            Library::at(std::env::temp_dir().join("ca72-editor-tests-no-library")),
        );
        e.draw();
        host.take();
        let p = at(&e, centre("osc1_waveform"));
        go(&mut e, p);
        press(&mut e);
        // (Past the click's threshold first, then half a pixel at a time across two places.)
        go(
            &mut e,
            Point {
                x: p.x + 4.0,
                y: p.y,
            },
        );
        for k in 1..=80 {
            go(
                &mut e,
                Point {
                    x: p.x + 4.0 + 0.5 * f64::from(k),
                    y: p.y,
                },
            );
        }
        release(&mut e);
        let shape = params.osc1_shape.as_ptr();
        let set: Vec<f64> = host
            .take()
            .iter()
            .filter_map(|c| match c {
                Call::Set(p, v) if *p == shape => Some(f64::from(*v) * places),
                _ => None,
            })
            .collect();
        assert!(set.len() > 60, "{set:?}");
        assert!(set[0] < 2.2 && set[set.len() - 1] > 3.4, "{set:?}");
        let each = 0.5 / 30.0;
        for w in set.windows(2).skip(1) {
            let moved = w[1] - w[0];
            assert!(
                (moved - each).abs() < 1e-5,
                "at {:.4}: moved {moved:.6}",
                w[0]
            );
        }
    }

    /// MOD's orb (decisions.md R-CONTINUOUS-WAVE): where the voices have a CONTINUOUS shape while
    /// its MOD is on, none while it is off or STEPPED.
    #[test]
    fn mods_orb_shows_where_the_voices_have_the_shape() {
        use crate::params::WaveMode;
        let (mut e, _host, _params) = editing_of(
            Ca72Params {
                osc1_shape_mode: EnumParam::new("Osc 1 Waveform Mode", WaveMode::Continuous),
                osc1_shape_mod: BoolParam::new("Osc 1 Shape Mod", true),
                osc2_shape_mode: EnumParam::new("Osc 2 Waveform Mode", WaveMode::Continuous),
                ..Ca72Params::default()
            },
            Library::at(std::env::temp_dir().join("ca72-editor-tests-no-library")),
        );
        e.meters.publish_shapes([Some(3.4), Some(1.0), Some(2.0)]);
        e.draw();
        assert_eq!(e.scene.shape[0].orb, Some(f64::from(3.4f32)));
        assert_eq!(e.scene.shape[1].orb, None, "MOD off");
        assert_eq!(e.scene.shape[2].orb, None, "STEPPED");
        e.meters.publish_shapes([None; 3]);
        e.draw();
        assert_eq!(e.scene.shape[0].orb, None, "no voice");
    }

    /// Oscillators 2's and 3's timed mode on the panel (decisions.md R-OSC-TIMED): at LO with its
    /// key on, oscillator 3's FREQUENCY turns its DIVISION a step at a time (FREQUENCY's own
    /// place untouched), its tip names the value, it takes a note value typed, and a menu on it
    /// learns DIVISION; its key flips TIMED and is learnable; oscillator 2's key, at 8', is
    /// dimmed and takes nothing, and its FREQUENCY is FREQUENCY.
    #[test]
    fn a_timed_oscillators_dial_and_key_operate_its_timed_mode() {
        use crate::params::{Division, Footage};
        let (mut e, host, params) = editing_of(
            Ca72Params {
                osc3_range: EnumParam::new("Osc 3 Range", Footage::Lo),
                osc3_timed: BoolParam::new("Osc 3 Timed", true),
                osc3_division: EnumParam::new("Osc 3 Division", Division::EighthDotted),
                osc2_timed: BoolParam::new("Osc 2 Timed", true),
                ..Ca72Params::default()
            },
            Library::at(std::env::temp_dir().join("ca72-editor-tests-no-library")),
        );
        e.draw();
        let index = |p: &str| ca72_panel::controls::index(p).expect("a control");
        let (freq2, freq3) = (index("osc2_frequency"), index("osc3_frequency"));
        assert_eq!(
            e.scene.timed,
            [
                ca72_panel::render::Timed {
                    division: 8,
                    inert: true
                },
                ca72_panel::render::Timed {
                    division: 9,
                    inert: false
                }
            ]
        );
        assert_eq!(e.scene.timed_dials(), [false, true]);
        assert!((e.scene.values[freq3] - 9.0 / 17.0).abs() < 1e-6);
        assert!(matches!(
            e.kind_of(freq3),
            Kind::Knob {
                dial: ca72_panel::controls::Dial::Timed,
                ..
            }
        ));
        assert!(matches!(
            e.kind_of(freq2),
            Kind::Knob {
                dial: ca72_panel::controls::Dial::Freq,
                ..
            }
        ));
        // A notch of the wheel: one note value, DIVISION's, nothing of FREQUENCY's.
        host.take();
        let pt = at(&e, centre("osc3_frequency"));
        go(&mut e, pt);
        wheel(&mut e, 1.0);
        e.draw();
        let tip = e.scene.tip.clone().expect("a tip").0;
        assert!(tip.starts_with("OSCILLATOR-3 DIVISION: 1/"), "{tip}");
        e.end_gestures();
        let calls = host.take();
        let division = params.osc3_division.as_ptr();
        let frequency = params.osc3_frequency.as_ptr();
        let set: Vec<f32> = calls
            .iter()
            .filter_map(|c| match c {
                Call::Set(p, v) if *p == division => Some(*v),
                _ => None,
            })
            .collect();
        assert_eq!(set.len(), 1, "{calls:?}");
        let step = (set[0] * 17.0 - 9.0).abs();
        assert!((step - 1.0).abs() < 1e-4, "one step: {set:?}");
        assert!(
            !calls
                .iter()
                .any(|c| matches!(c, Call::Begin(p) | Call::Set(p, _) if *p == frequency)),
            "FREQUENCY touched: {calls:?}"
        );
        // A note value typed at the knob: its division's text first, then the one typed.
        assert!(e.start_knob_entry(freq3));
        assert_eq!(e.knob_entry.as_ref().map(|k| k.text()), Some("8."));
        for c in "16t".chars() {
            keyed(&mut e, Key::Character(c.to_string()));
        }
        keyed(&mut e, Key::Enter);
        assert!(
            host.take().contains(&Call::Set(division, 16.0 / 17.0)),
            "1/16 triplet typed"
        );
        // MIDI Learn on the timed knob learns DIVISION; on a key, its TIMED.
        assert_eq!(e.learning.timed, [false, true]);
        for (place, id) in [
            (freq3, "osc3_division"),
            (freq2, "osc2_frequency"),
            (index("osc3_timed"), "osc3_timed"),
            (index("osc2_timed"), "osc2_timed"),
        ] {
            assert_eq!(
                crate::learning::learnable_at(Place::Panel(place), e.learning.timed),
                crate::learn::index(id),
                "{id}"
            );
        }
        assert_eq!(
            crate::learning::place(crate::learn::index("osc3_division").expect("learnable")),
            Some(Place::Panel(freq3))
        );
        // Oscillator 3's key flips TIMED; oscillator 2's, at 8', is inert.
        let (key2, key3) = (index("osc2_timed"), index("osc3_timed"));
        assert!(e.operable(key3) && !e.operable(key2));
        host.take();
        let pt = at(&e, centre("osc3_timed"));
        click(&mut e, pt);
        assert_eq!(
            host.take(),
            vec![
                Call::Begin(params.osc3_timed.as_ptr()),
                Call::Set(params.osc3_timed.as_ptr(), 0.0),
                Call::End(params.osc3_timed.as_ptr())
            ]
        );
        let pt = at(&e, centre("osc2_timed"));
        click(&mut e, pt);
        assert!(host.take().is_empty(), "the hidden key took a press");
    }

    /// A TIMED key away from its RANGE's LO (decisions.md R-OSC-TIMED; the owner: "they should
    /// disappear ... if it's not on low mode"): off the panel, not found by the pointer (no
    /// press, wheel, tip or menu), not ringed while learned from the list; its TIMED and
    /// DIVISION kept. At LO, the same key is there and flips TIMED.
    #[test]
    fn a_timed_key_away_from_lo_is_off_the_panel() {
        use crate::params::{Division, Footage};
        for range in [Footage::R32, Footage::R8, Footage::R2] {
            let (mut e, host, params) = editing_of(
                Ca72Params {
                    osc2_range: EnumParam::new("Osc 2 Range", range),
                    osc2_timed: BoolParam::new("Osc 2 Timed", true),
                    osc2_division: EnumParam::new("Osc 2 Division", Division::Sixteenth),
                    osc3_range: EnumParam::new("Osc 3 Range", Footage::Lo),
                    ..Ca72Params::default()
                },
                Library::at(std::env::temp_dir().join("ca72-editor-tests-no-library")),
            );
            e.draw();
            let index = |p: &str| ca72_panel::controls::index(p).expect("a control");
            let (key2, key3) = (index("osc2_timed"), index("osc3_timed"));
            assert!(e.off_panel(key2) && !e.off_panel(key3), "{range:?}");
            assert!(e.scene.timed[0].inert && !e.scene.timed[1].inert);
            let (x, y) = centre("osc2_timed");
            assert_ne!(e.hit(x, y), Some(Target::Control(key2)), "{range:?}: found");
            let (x, y) = centre("osc3_timed");
            assert_eq!(e.hit(x, y), Some(Target::Control(key3)), "{range:?}");
            // A press, the wheel, a tip and a menu on its place: nothing of the key's.
            host.take();
            let pt = at(&e, centre("osc2_timed"));
            click(&mut e, pt);
            go(&mut e, pt);
            wheel(&mut e, 1.0);
            e.end_gestures();
            assert!(
                host.take().is_empty(),
                "{range:?}: the hidden key took a press"
            );
            e.draw();
            assert!(e.scene.tip.is_none(), "{range:?}: {:?}", e.scene.tip);
            // (A pointer resting there as RANGE leaves LO: no tip.)
            e.hover = Some(Target::Control(key2));
            assert!(e.tip().is_none(), "{range:?}");
            e.hover = None;
            right_click(&mut e, pt);
            assert!(
                !e.learning.menu().is_some_and(|m| m.title.contains("TIMED")),
                "{range:?}: its menu"
            );
            e.learning.close_menu();
            // Learned from the MIDI list: no ring round its empty place.
            e.learning.learn(&params.midi_map, learnable("osc2_timed"));
            e.draw();
            assert_eq!(e.scene.learning, None, "{range:?}: ringed");
            e.learning.learn(&params.midi_map, learnable("osc3_timed"));
            e.draw();
            assert_eq!(e.scene.learning, Some(key3), "{range:?}");
            e.learning.cancel(&params.midi_map);
            // Its values kept.
            assert!(params.osc2_timed.value());
            assert_eq!(params.osc2_division.value(), Division::Sixteenth);
            // At LO, oscillator 3's key is on the panel and flips TIMED.
            host.take();
            let pt = at(&e, centre("osc3_timed"));
            click(&mut e, pt);
            let p = params.osc3_timed.as_ptr();
            assert_eq!(
                host.take(),
                vec![Call::Begin(p), Call::Set(p, 1.0), Call::End(p)],
                "{range:?}"
            );
        }
    }

    /// OSC. 3 CONTROL while oscillator 3 is timed (its key on, its RANGE at LO; decisions.md
    /// R-OSC-TIMED): dimmed, and a click or the wheel on it does nothing, its value kept as it
    /// was, on or off; with the key off or RANGE away from LO, live with its own setting.
    #[test]
    fn osc3_control_is_inert_while_oscillator_3_is_timed() {
        use crate::params::Footage;
        let index = |p: &str| ca72_panel::controls::index(p).expect("a control");
        let control = index("osc3_control");
        for (timed, range, inert) in [
            (true, Footage::Lo, true),
            (true, Footage::R8, false),
            (false, Footage::Lo, false),
            (false, Footage::R16, false),
        ] {
            for held in [true, false] {
                let case = format!("TIMED {timed}, RANGE {range:?}, OSC. 3 CONTROL {held}");
                let (mut e, host, params) = editing_of(
                    Ca72Params {
                        osc3_range: EnumParam::new("Osc 3 Range", range),
                        osc3_timed: BoolParam::new("Osc 3 Timed", timed),
                        osc3_control: BoolParam::new("Osc 3 Control", held),
                        ..Ca72Params::default()
                    },
                    Library::at(std::env::temp_dir().join("ca72-editor-tests-no-library")),
                );
                e.draw();
                assert_eq!(osc3_control_inert(&params), inert, "{case}");
                assert_eq!(e.scene.osc3_control_off, inert, "{case}: dimmed");
                assert_eq!(e.operable(control), !inert, "{case}: operable");
                // (Oscillator 3's RANGE and FREQUENCY live whatever.)
                assert!(e.operable(index("osc3_range")) && e.operable(index("osc3_frequency")));
                assert_eq!(e.scene.values[control], f64::from(u8::from(held)), "{case}");
                host.take();
                let pt = at(&e, centre("osc3_control"));
                click(&mut e, pt);
                let p = params.osc3_control.as_ptr();
                if inert {
                    go(&mut e, pt);
                    wheel(&mut e, 1.0);
                    wheel(&mut e, -1.0);
                    e.end_gestures();
                    assert!(host.take().is_empty(), "{case}: took a press or the wheel");
                    assert_eq!(params.osc3_control.value(), held, "{case}: its value kept");
                } else {
                    assert_eq!(
                        host.take(),
                        vec![
                            Call::Begin(p),
                            Call::Set(p, if held { 0.0 } else { 1.0 }),
                            Call::End(p)
                        ],
                        "{case}: flipped"
                    );
                }
            }
        }
        // Alongside oscillator 3's MOD key (R-CONTINUOUS-WAVE): its waveform CONTINUOUS, the key
        // on the panel and live while OSC. 3 CONTROL is inert.
        let (mut e, host, params) = editing_of(
            Ca72Params {
                osc3_range: EnumParam::new("Osc 3 Range", Footage::Lo),
                osc3_timed: BoolParam::new("Osc 3 Timed", true),
                osc3_shape_mode: EnumParam::new(
                    "Osc 3 Waveform Mode",
                    crate::params::WaveMode::Continuous,
                ),
                ..Ca72Params::default()
            },
            Library::at(std::env::temp_dir().join("ca72-editor-tests-no-library")),
        );
        e.draw();
        let key = index("osc3_shape_mod");
        assert!(e.scene.osc3_control_off && !e.operable(control));
        assert!(e.operable(key) && !e.off_panel(key));
        host.take();
        let pt = at(&e, centre("osc3_control"));
        click(&mut e, pt);
        assert!(host.take().is_empty(), "OSC. 3 CONTROL took a press");
        let pt = at(&e, centre("osc3_shape_mod"));
        click(&mut e, pt);
        let p = params.osc3_shape_mod.as_ptr();
        assert_eq!(
            host.take(),
            vec![Call::Begin(p), Call::Set(p, 1.0), Call::End(p)],
            "the MOD key flipped"
        );
    }

    #[test]
    fn a_chosen_width_survives_a_new_instance_and_project_width_wins() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        Settings::keep(&path, Change::Lettering(Lettering::Large)).unwrap();
        let (mut e, _, _) = editing();
        e.settings_file = Some(path.clone());
        assert!(e.resize(1500));
        e.reconcile_resize();
        let settings = Settings::read(&path);
        assert_eq!(settings.editor_width, 1500);
        assert_eq!(settings.lettering, Lettering::Large);
        let params = Arc::new(Ca72Params::default());
        let editor = Ca72Editor::with_settings(
            Arc::clone(&params),
            Arc::new(Meters::default()),
            Arc::new(LocalNotes::default()),
            settings,
        );
        assert_eq!(
            editor.opening().0,
            1500,
            "fresh instance uses the computer preference"
        );
        params.editor_width.store(1800, Ordering::Relaxed);
        assert_eq!(editor.opening().0, 1800, "saved project takes precedence");
        assert_eq!(
            Settings::read(&path).editor_width,
            1500,
            "recall does not change the preference"
        );
    }

    #[test]
    fn only_an_accepted_width_becomes_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        Settings::keep(&path, Change::EditorWidth(1200)).unwrap();
        let (mut e, host, params) = editing();
        e.settings_file = Some(path.clone());
        host.fixed.store(true, Ordering::Relaxed);
        assert!(!e.resize(1500));
        e.reconcile_resize();
        assert_eq!(Settings::read(&path).editor_width, 1200);
        host.fixed.store(false, Ordering::Relaxed);
        host.deferred.store(true, Ordering::Relaxed);
        assert!(e.resize(1500));
        let (id, _) = params.editor_resizing.requested().unwrap();
        e.reconcile_resize();
        assert_eq!(
            Settings::read(&path).editor_width,
            1200,
            "queued is not accepted"
        );
        params.editor_resizing.answer(id, false);
        e.reconcile_resize();
        assert_eq!(
            Settings::read(&path).editor_width,
            1200,
            "delayed refusal is not saved"
        );
        assert!(e.resize(1600));
        let (id, _) = params.editor_resizing.requested().unwrap();
        params.editor_resizing.answer(id, true);
        e.reconcile_resize();
        assert_eq!(Settings::read(&path).editor_width, 1600);
        e.toggle_panel();
        let (id, _) = params.editor_resizing.requested().unwrap();
        params.editor_resizing.answer(id, true);
        e.reconcile_resize();
        assert_eq!(
            Settings::read(&path).editor_width,
            1600,
            "panel height does not change default width"
        );
    }

    #[test]
    fn grip_drag_remembers_its_final_width_when_released() {
        let (mut e, _, _) = editing();
        e.resizing = Some((0.0, e.width));
        e.resize(1500);
        e.reconcile_resize();
        assert_eq!(e.settings.editor_width, 0);
        e.resize(1600);
        e.reconcile_resize();
        assert_eq!(e.settings.editor_width, 0);
        e.resizing = None;
        e.reconcile_resize();
        assert_eq!(e.settings.editor_width, 1600);
    }

    #[test]
    fn the_grip_saves_the_width_it_was_given() {
        let (mut e, _host, params) = editing();
        assert!(e.resize(1500));
        assert_eq!(
            (e.width, params.editor_width.load(Ordering::Relaxed)),
            (1500, 1500)
        );
        assert!(!e.resize(1500));
        // Every frame then drawn at that size (on macOS the window applies it itself: its
        // baseview reports no resize of its own making).
        e.fit(1.0, 1500, 1500);
        e.draw();
        let widths = [e.renderer.frame().width(), e.strip.frame().width()];
        assert_eq!(widths, [1500; 2]);
        // (The panel's renderer draws the whole drawing, the strip's parts over its foot.)
        let tall = e.renderer.frame().height();
        assert!(
            tall.abs_diff(height_for(1500)) <= 2,
            "{tall} for {}",
            height_for(1500)
        );
    }

    /// The settings key's WINDOW: SIZE… (R-SIZE) opens the SIZE menu where the settings' was:
    /// shares of the size the editor opens at on a 1920 by 1080 screen, then FIT TO SCREEN; the
    /// size the window is marked and not to be chosen. A size chosen is asked of the host as
    /// the grip's are, and saved with the session. Where the screen is known, sizes too large
    /// for it cannot be chosen, and FIT TO SCREEN is the size the editor opens at on it.
    #[test]
    fn the_settings_key_s_size_menu_resizes_the_window() {
        let (mut e, host, params) = editing();
        let key = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Settings)));
        click(&mut e, key);
        let items = menu_items(&e);
        assert_eq!(
            items[12..14],
            [
                ("WINDOW".to_owned(), false),
                ("SIZE\u{2026}".to_owned(), true)
            ]
        );
        menu_item(&mut e, "SIZE\u{2026}");
        let m = e.learning.menu().expect("the SIZE menu");
        assert_eq!(m.title, "SIZE");
        let labels: Vec<_> = m.items.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(
            labels,
            ["75%", "100%", "125%", "150%", "200%", "FIT TO SCREEN"]
        );
        // (At 1720 wide, none of them: none marked; the screen not known, all fit.)
        assert!(m.marked.is_empty() && m.items.iter().all(|(_, can)| *can));
        let (w, h) = m.extent(e.renderer.fonts());
        assert!(m.x >= 0.0 && m.x + w <= art::W && m.y + h <= art::H);
        menu_item(&mut e, "100%");
        assert!(e.learning.menu().is_none());
        // (Asked of the host at the next frame, as the grip's are.)
        e.grip();
        e.draw();
        assert_eq!(
            (e.width, params.editor_width.load(Ordering::Relaxed)),
            (DEFAULT_WIDTH, DEFAULT_WIDTH)
        );
        assert_eq!(host.resizes.load(Ordering::Relaxed), 1);
        e.fit(1.0, DEFAULT_WIDTH, height_for(DEFAULT_WIDTH));
        e.draw();
        // A screen 1512 by 945 (a 14-inch MacBook Pro's, less its menu bar).
        e.screen = |_| Some((1512.0, 945.0));
        let sizes = e.sizes();
        assert_eq!(
            sizes
                .iter()
                .map(|&(_, w, fits)| (w, fits))
                .collect::<Vec<_>>(),
            [
                (1035, true),
                (1380, true),
                (1725, false),
                (2070, false),
                (2760, false),
                (fitted_width(Some((1512.0, 945.0))), true),
            ]
        );
        let key = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Settings)));
        click(&mut e, key);
        menu_item(&mut e, "SIZE\u{2026}");
        assert_eq!(
            e.learning
                .menu()
                .map(|m| (m.marked.clone(), m.items[1].1, m.items[2].1)),
            Some((vec![1], false, false)),
            "100% marked; 125% too large for the screen"
        );
        menu_item(&mut e, "FIT TO SCREEN");
        e.grip();
        e.draw();
        assert_eq!(e.width, fitted_width(Some((1512.0, 945.0))));
    }

    /// The grip (R-SIZE): over it, the tip says what it does, within the window, in the full
    /// view and the compact; and its lines are plain to see.
    #[test]
    fn the_grip_says_what_it_does() {
        let (mut e, _, _) = editing();
        e.draw();
        let p = at(&e, (art::W - 20.0, art::H - 20.0));
        go(&mut e, p);
        assert!(e.on_grip());
        let (t, x, _) = e.tip().expect("a tip");
        assert_eq!(t, GRIP_TIP);
        let size = e.renderer.text_size();
        let w = e.renderer.fonts().advance(&t, size, Weight::Regular, 0.0) + 0.8 * size;
        assert!(x + w / 2.0 <= art::W && x - w / 2.0 >= 0.0);
        e.draw();
        assert!(e.scene.tip.is_some());
        let p = at(&e, (art::W / 2.0, art::H / 2.0));
        go(&mut e, p);
        assert!(!e.on_grip());
        // The compact view's, in the rail's corner.
        let panel = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Panel)));
        click(&mut e, panel);
        e.draw();
        let p = at(&e, (art::W - 20.0, DRAWER_TOP - 20.0));
        go(&mut e, p);
        assert!(e.on_grip());
        assert_eq!(e.tip().map(|(t, ..)| t), Some(GRIP_TIP.to_owned()));
        // The drawing's grip: on the strip's face inside its trim, brighter than the face.
        let mut r = Renderer::new(0.5, 1.0);
        r.render(&Scene::default());
        let f = r.frame();
        let px = |x: f64, y: f64| {
            f.pixel((x * 0.5) as u32, (y * 0.5) as u32)
                .expect("in it")
                .red()
        };
        assert!(px(art::W - 47.0, art::H - 25.0) > px(art::W - 150.0, art::H - 40.0) + 60);
    }

    /// The large lettering's settings' menu in the smallest window, the panel hidden: made
    /// smaller to fit the view rather than running off it (R-LETTERING).
    #[test]
    fn a_menu_too_tall_for_the_view_is_made_smaller() {
        let (mut e, _, _) = editing();
        e.set_lettering(Lettering::Large);
        assert!(e.resize(MIN_WIDTH));
        e.fit(1.0, MIN_WIDTH, height_for(MIN_WIDTH));
        e.draw();
        let panel = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Panel)));
        click(&mut e, panel);
        e.draw();
        let key = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Settings)));
        click(&mut e, key);
        e.draw();
        let m = e.learning.menu().expect("a menu");
        let (w, h) = m.extent(e.renderer.fonts());
        assert!(m.y >= 0.0 && m.y + h <= DRAWER_TOP + 1e-6, "{} + {h}", m.y);
        assert!(m.x >= 0.0 && m.x + w <= art::W);
        assert!(m.size < e.renderer.text_size() * MENU_TEXT, "made smaller");
    }

    #[test]
    fn the_panel_key_snaps_and_presets_temporarily_reveal_it() {
        let (mut e, host, params) = editing();
        e.draw();
        let panel = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Panel)));
        let full = e.window_height();
        let values = e.scene.values;
        click(&mut e, panel);
        assert!(!params.editor_panel.load(Ordering::Relaxed));
        assert_eq!(e.window_height(), height_for_panel(e.width, false));
        assert!(e.window_height() < full && e.resizing_window);
        e.draw();
        assert_eq!(host.resizes.load(Ordering::Relaxed), 1);
        assert_eq!(e.composed.as_ref().unwrap().height(), e.window_height());
        assert_eq!(e.hit(100.0, DRAWER_TOP + 20.0), None);
        assert_eq!(e.hit(art::W - 10.0, DRAWER_TOP - 10.0), Some(Target::Grip));

        open(&mut e);
        assert_eq!(e.window_height(), full);
        assert!(!params.editor_panel.load(Ordering::Relaxed), "temporary");
        assert_eq!(host.resizes.load(Ordering::Relaxed), 2);
        assert!(key(&mut e, Key::Escape));
        e.follow_drawer();
        assert_eq!(e.window_height(), height_for_panel(e.width, false));
        e.draw();
        assert_eq!(host.resizes.load(Ordering::Relaxed), 3);
        click(&mut e, panel);
        assert_eq!(e.window_height(), full);
        assert!(params.editor_panel.load(Ordering::Relaxed));
        assert_eq!(host.resizes.load(Ordering::Relaxed), 4);
        e.draw();
        assert_eq!(e.scene.values, values, "showing the panel changes no sound");
        assert!(host.take().is_empty(), "no parameter gestures");

        click(&mut e, panel);
        params.editor_width.store(e.width, Ordering::Relaxed);
        let editor = Ca72Editor::new(Arc::clone(&params), Arc::clone(&e.meters));
        assert_eq!(
            editor.opening(),
            (e.width, height_for_panel(e.width, false))
        );
    }

    #[test]
    fn a_refused_panel_resize_keeps_the_current_view() {
        let (mut e, host, params) = editing();
        e.draw();
        assert!(e.browser.bar.panel);
        let panel = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Panel)));
        host.fixed.store(true, Ordering::Relaxed);
        click(&mut e, panel);
        assert!(params.editor_panel.load(Ordering::Relaxed));
        assert!(e.expanded());
        assert!(!e.resizing_window);
        e.draw();
        assert!(e.browser.bar.panel, "refused collapse keeps PANEL active");
        host.fixed.store(false, Ordering::Relaxed);
        click(&mut e, panel);
        assert!(!e.expanded());
        e.draw();
        assert!(!e.browser.bar.panel, "collapsed PANEL is neutral");
        host.fixed.store(true, Ordering::Relaxed);
        click(&mut e, panel);
        e.draw();
        assert!(!e.browser.bar.panel, "refused expansion stays neutral");
        let name = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Name)));
        click(&mut e, name);
        assert!(!e.browser.open && !e.expanded());
        assert!(!params.editor_panel.load(Ordering::Relaxed));
    }

    #[test]
    fn the_compact_frame_drawn_in_parts_is_the_frame_drawn_whole() {
        let (mut e, _, params) = editing();
        let hostless = nih_plug::context::process::TestProcessContext::<crate::Ca72>::new(
            Arc::clone(&params) as Arc<dyn Params>,
            48_000.0,
            ProcessMode::Realtime,
        );
        let panel = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Panel)));
        click(&mut e, panel);
        for i in 0..12 {
            hostless.automate(params.cutoff.as_ptr(), i as f32 / 12.0);
            e.draw();
            let mut whole = None;
            compose(
                &mut whole,
                &e.renderer,
                e.strip.frame(),
                None,
                None,
                (
                    e.page_shows().then(|| e.lfo.frame()),
                    e.macros_show()
                        .then(|| (e.macros.frame(), e.renderer.left_rect())),
                    &e.modulation,
                ),
                None,
                false,
                (e.physical_width, e.physical_height()),
            );
            assert!(e.composed == whole, "compact frame {i}");
        }
        let grip = at(&e, (art::W - 10.0, DRAWER_TOP - 10.0));
        go(&mut e, grip);
        e.mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            modifiers: Modifiers::empty(),
        });
        assert!(e.resizing.is_some(), "the compact grip is operable");
        e.end_gestures();
    }

    #[test]
    fn compact_settings_stay_inside_the_view_and_are_operable() {
        let (mut e, _, _) = editing();
        let panel = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Panel)));
        click(&mut e, panel);
        let settings = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Settings)));
        click(&mut e, settings);
        e.draw();
        let menu = e.learning.menu().unwrap();
        let (_, h) = menu.extent(e.renderer.fonts());
        assert!(menu.y >= 0.0 && menu.y + h <= DRAWER_TOP);
        menu_item(&mut e, "CLASSIC");
        assert_eq!(e.renderer.skin(), Skin::Drawn);
        assert!(!e.expanded());
    }

    #[test]
    fn recalled_panel_visibility_reaches_an_open_editor_and_keeps_presets_visible() {
        let (mut e, _, params) = editing();
        params.editor_width.store(e.width, Ordering::Relaxed);
        let editor = Ca72Editor::new(Arc::clone(&params), Arc::clone(&e.meters));
        editor.open.store(true, Ordering::Relaxed);
        params.editor_panel.store(false, Ordering::Relaxed);
        editor.param_values_changed();
        assert_eq!(editor.size().1, height_for_panel(e.width, false));
        e.draw();
        assert!(!e.expanded() && e.resizing_window);
        params.editor_panel.store(true, Ordering::Relaxed);
        assert_eq!(editor.size().1, height_for(e.width));
        e.draw();
        assert!(e.expanded());
        open(&mut e);
        params.editor_panel.store(false, Ordering::Relaxed);
        assert_eq!(editor.size().1, height_for(e.width), "presets still shown");
        e.draw();
        assert!(e.expanded() && e.browser.open);
        assert!(key(&mut e, Key::Escape));
        e.follow_drawer();
        assert_eq!(editor.size().1, height_for_panel(e.width, false));
    }

    #[test]
    fn refused_expansion_preserves_type_notes_when_restoring_compact() {
        let (mut e, host, params) = editing_standalone();
        e.toggle_panel();
        e.draw();
        assert!(!e.expanded());
        host.deferred.store(true, Ordering::Relaxed);
        e.playing.visible = true;
        e.playing.typing = true;
        e.toggle_panel();
        let (request, _) = params.editor_resizing.requested().expect("expansion");
        let key = KeyboardEvent {
            key: Key::Character("a".into()),
            ..Default::default()
        };
        assert!(e.keyed(&key));
        assert!(e.playing.notes.pop().expect("attack").on);
        let generation = e.playing.notes.generation();
        params.editor_resizing.answer(request, false);
        e.draw();
        assert!(!e.expanded());
        assert_eq!(e.playing.notes.generation(), generation);
        assert!(e.playing.wants_keys);
        assert!(e.keyed(&key));
        assert!(
            e.playing.notes.pop().is_none(),
            "held note does not retrigger"
        );
        let mut up = key;
        up.state = KeyState::Up;
        assert!(e.keyed(&up));
        assert!(!e.playing.notes.pop().expect("original note released").on);
    }

    #[test]
    fn delayed_refusal_restores_the_view_and_ignores_superseded_requests() {
        let (mut e, host, params) = editing();
        host.deferred.store(true, Ordering::Relaxed);
        e.draw();
        let original_width = params.editor_width.load(Ordering::Relaxed);
        e.toggle_panel();
        let (first, _) = params.editor_resizing.requested().expect("a request");
        e.draw();
        e.toggle_panel();
        e.draw();
        e.toggle_panel();
        let (last, _) = params.editor_resizing.requested().expect("a request");
        params.editor_resizing.answer(first, false);
        e.draw();
        assert!(
            !e.expanded(),
            "an obsolete refusal cannot undo the last toggle"
        );
        params.editor_resizing.answer(last, false);
        e.draw();
        assert!(e.expanded());
        assert!(params.editor_panel.load(Ordering::Relaxed));
        assert_eq!(params.editor_width.load(Ordering::Relaxed), original_width);
    }

    #[test]
    fn delayed_width_refusal_keeps_the_restored_frame_through_native_follow() {
        for dpr in [1.0, 1.25, 2.0] {
            let (mut e, host, params) = editing();
            let original = e.width;
            let physical = (f64::from(original) * dpr).round() as u32;
            e.fit(dpr, original, physical);
            e.draw();
            host.deferred.store(true, Ordering::Relaxed);
            e.resize(original + 100);
            let wider = e.width;
            e.fit(dpr, wider, (f64::from(wider) * dpr).round() as u32);
            e.draw();
            let (request, _) = params.editor_resizing.requested().expect("width request");
            params.editor_resizing.answer(request, false);
            e.draw();
            assert_eq!(e.width, original);
            let composed = e.composed.clone();
            assert_eq!(composed.as_ref().expect("restored frame").width(), physical);
            // on_frame follows this restoration before presenting the composed controls.
            e.fit(dpr, original, physical);
            assert!(
                e.composed == composed,
                "native follow preserves restored controls"
            );
        }
    }

    #[test]
    fn a_refused_shrink_keeps_presets_and_the_compact_preference() {
        let (mut e, host, params) = editing();
        e.toggle_panel();
        e.draw();
        host.deferred.store(true, Ordering::Relaxed);
        open(&mut e);
        e.draw();
        let (reveal, _) = params.editor_resizing.requested().expect("reveal");
        params.editor_resizing.answer(reveal, true);
        e.draw();
        assert!(e.browser.open);
        e.browser.show(false, &params);
        e.follow_drawer();
        e.draw();
        let (shrink, _) = params.editor_resizing.requested().expect("shrink");
        params.editor_resizing.answer(shrink, false);
        e.draw();
        assert!(e.browser.open, "the host still owns the full presets view");
        assert!(e.expanded());
        assert!(
            !params.editor_panel.load(Ordering::Relaxed),
            "retain the compact preference"
        );
    }

    #[test]
    fn refusal_keeps_presets_that_opened_without_changing_height() {
        let (mut e, host, _) = editing();
        open(&mut e);
        e.draw();
        assert!(e.browser.open);
        host.fixed.store(true, Ordering::Relaxed);
        e.toggle_panel();
        e.draw();
        assert!(
            e.browser.open,
            "a refused toggle must preserve the existing presets view"
        );
        e.resize(e.width + 100);
        e.draw();
        assert!(
            e.browser.open,
            "a refused width must preserve the existing presets view"
        );
    }

    #[test]
    fn recalled_visibility_during_a_queued_width_does_not_retry_refusal() {
        let (mut e, host, params) = editing();
        host.deferred.store(true, Ordering::Relaxed);
        e.resize(e.width + 100);
        e.draw();
        let (width, _) = params.editor_resizing.requested().expect("width request");
        params.editor_panel.store(false, Ordering::Relaxed);
        e.reconcile_resize();
        params.editor_resizing.answer(width, true);
        e.draw();
        let (compact, _) = params.editor_resizing.requested().expect("height request");
        let requests = host.resizes.load(Ordering::Relaxed);
        params.editor_resizing.answer(compact, false);
        e.draw();
        assert_eq!(
            host.resizes.load(Ordering::Relaxed),
            requests,
            "refusal must not immediately retry the collapse"
        );
        assert_eq!(
            params.editor_panel.load(Ordering::Relaxed),
            params.editor_panel_applied.load(Ordering::Relaxed)
        );
        let requests = host.resizes.load(Ordering::Relaxed);
        e.draw();
        assert_eq!(
            host.resizes.load(Ordering::Relaxed),
            requests,
            "a refusal must not become a retry loop"
        );
    }

    #[test]
    fn frames_fit_the_rounded_physical_viewport_at_fractional_scales() {
        for width in [861, 862, 1119, 1121] {
            for dpr in [1.0, 1.25, 1.5, 2.0] {
                let (mut e, _, _) = editing();
                let physical_width = (f64::from(width) * dpr).round() as u32;
                e.fit(dpr, width, physical_width);
                for expanded in [true, false] {
                    if expanded != e.expanded() {
                        e.toggle_panel();
                    }
                    e.draw();
                    let frame = e.composed.as_ref().expect("a frame");
                    assert_eq!(
                        (frame.width(), frame.height()),
                        (physical_width, e.physical_height()),
                        "width {width}, scale {dpr}, expanded {expanded}"
                    );
                    if !expanded {
                        let bottom = f64::from(e.physical_height()) / dpr;
                        let (x, y) =
                            e.in_drawing((f64::from(width) - 1.0 / dpr, bottom - 1.0 / dpr));
                        assert_eq!(e.hit(x, y), Some(Target::Grip));
                    }
                }
            }
        }
    }

    #[test]
    fn a_height_only_resize_keeps_the_frame_just_composed() {
        let (mut e, _, params) = editing();
        for expanded in [false, true] {
            params.editor_panel.store(expanded, Ordering::Relaxed);
            e.draw();
            let composed = e.composed.clone();
            assert!(e.resizing_window && composed.is_some());
            // on_frame follows a resize asked during draw before presenting that frame.
            let physical = WindowInfo::from_logical_size(
                Size::new(f64::from(e.width), f64::from(e.window_height())),
                e.dpr,
            )
            .physical_size();
            e.fit(e.dpr, e.width, physical.width);
            assert!(e.composed == composed, "keep the rail and compact grip");
        }
    }

    /// The grip's drag asks the host for a size once a frame at most, the last the pointer
    /// took it to (each resize renders the whole panel again).
    #[test]
    fn the_grip_resizes_once_a_frame() {
        let (mut e, host, params) = editing();
        let grip = at(&e, (art::W - 27.0, art::H - 27.0));
        go(&mut e, grip);
        press(&mut e);
        assert!(e.resizing.is_some());
        for dx in 1..=20 {
            let p = Point {
                x: grip.x + f64::from(dx) * 5.0,
                y: grip.y,
            };
            go(&mut e, p);
        }
        assert_eq!(
            host.resizes.load(Ordering::Relaxed),
            0,
            "none between frames"
        );
        e.grip();
        assert_eq!(host.resizes.load(Ordering::Relaxed), 1);
        assert_eq!(
            (e.width, params.editor_width.load(Ordering::Relaxed)),
            (1820, 1820)
        );
        e.grip();
        assert_eq!(
            host.resizes.load(Ordering::Relaxed),
            1,
            "nothing moved since"
        );
        release(&mut e);
    }

    // ---- The presets (decisions.md R10).

    /// A point of the drawer (its units) in the window, the drawer open.
    fn in_drawer(e: &Editing, (x, y): (f64, f64)) -> Point {
        let (top, _, _) = e.drawer_span().expect("the drawer open");
        at(e, (x, top + y))
    }

    fn key(e: &mut Editing, k: Key) -> bool {
        let setter = ParamSetter::new(e.context.as_ref());
        e.browser.key(
            &KeyboardEvent {
                state: KeyState::Down,
                key: k,
                ..KeyboardEvent::default()
            },
            &e.params,
            &setter,
        )
    }

    fn typed(e: &mut Editing, s: &str) {
        for c in s.chars() {
            key(e, Key::Character(c.to_string()));
        }
    }

    /// The drawer, opened from the bar's name and slid open.
    fn open(e: &mut Editing) {
        let p = on_strip(e, strip_at(StripTarget::Bar(BarTarget::Name)));
        click(e, p);
        assert!(e.browser.open);
        std::thread::sleep(crate::presets::SLIDE + Duration::from_millis(20));
        e.draw();
        assert_eq!(e.browser.reveal(), 1.0);
    }

    /// The drawer's slide over, and drawn.
    fn slid(e: &mut Editing) {
        std::thread::sleep(crate::presets::SLIDE + Duration::from_millis(20));
        e.draw();
    }

    /// A library of the factory's presets and twelve of the user's (`Pad 01` to `Pad 12`), so
    /// that the list runs on below the rows shown whatever the factory's count.
    fn long_library(dir: &std::path::Path) -> Library {
        let lib = Library::at(dir);
        for i in 1..=12 {
            lib.save(&format!("Pad {i:02}"), vec![], None).unwrap();
        }
        lib
    }

    fn row_of(e: &Editing, name: &str) -> usize {
        e.browser
            .drawer
            .rows
            .iter()
            .position(|r| r.name == name)
            .unwrap_or_else(|| panic!("{name} not listed"))
    }

    fn preset(p: &Ca72Params) -> String {
        p.preset.read().unwrap().clone()
    }

    /// A trackpad's or a Magic Mouse's scrolling (a few points an event) and a wheel turned
    /// slowly (fractions of a line) scroll the list a row for each row's travel; a turn the
    /// other way starts again (the owner, 2026-10-02: neither the wheel nor touch scrolled it).
    #[test]
    fn small_scrolls_add_up_to_rows() {
        let dir = tempfile::tempdir().unwrap();
        let (mut e, _host, _params) = editing_with(long_library(dir.path()));
        e.draw();
        open(&mut e);
        let list = in_drawer(
            &e,
            presets_ui::row_centre(&e.browser.drawer, 0).expect("shown"),
        );
        go(&mut e, list);
        let wheel = |e: &mut Editing, delta: ScrollDelta| {
            e.mouse(MouseEvent::WheelScrolled {
                delta,
                modifiers: Modifiers::empty(),
            });
        };
        // A row's travel in points at this size, in 4-point events: one row down.
        let row_points = ROW_H * e.renderer.scale() / e.dpr;
        let events = (row_points / 4.0).ceil() as usize;
        assert!(events > 2, "{row_points} points a row");
        for _ in 0..events {
            wheel(&mut e, ScrollDelta::Pixels { x: 0.0, y: -4.0 });
        }
        assert_eq!(e.browser.drawer.first, 1, "a row's travel of a trackpad");
        // A wheel turned slowly: three tenths of a line at a time.
        for _ in 0..4 {
            wheel(&mut e, ScrollDelta::Lines { x: 0.0, y: -0.3 });
        }
        assert_eq!(e.browser.drawer.first, 2, "1.2 lines of a slow wheel");
        // Back up: the 0.2 left over is dropped, so one line up is one row.
        wheel(&mut e, ScrollDelta::Lines { x: 0.0, y: 1.0 });
        assert_eq!(e.browser.drawer.first, 1);
    }

    #[test]
    fn the_rail_drops_the_drawer_over_the_strip_and_a_row_sets_the_plugin() {
        let dir = tempfile::tempdir().unwrap();
        let (mut e, host, params) = editing_with(long_library(dir.path()));
        e.draw();
        let shut = e.window_height();
        assert_eq!(shut, height_for(1720));
        open(&mut e);
        // Over the strip from under the rail, the window as it was, nothing asked of the host.
        assert_eq!(e.window_height(), shut);
        assert!(!e.resizing_window);
        assert_eq!(host.resizes.load(Ordering::Relaxed), 0);
        assert_eq!(e.drawer_span().unwrap(), (DRAWER_TOP, DRAWER_TOP, art::H));
        assert!(!e.browser.bar.below && e.browser.bar.open);
        assert_eq!(
            e.browser.drawer.rows.len(),
            crate::library::factory().len() + 12
        );
        // A row sets every parameter it names (or its default), the preset remembered.
        host.take();
        let i = row_of(&e, "Warp Field");
        // Below the rows shown: the wheel scrolls the list to it.
        let list = in_drawer(
            &e,
            presets_ui::row_centre(&e.browser.drawer, 0).expect("shown"),
        );
        go(&mut e, list);
        while presets_ui::row_centre(&e.browser.drawer, i).is_none() {
            let first = e.browser.drawer.first;
            e.mouse(MouseEvent::WheelScrolled {
                delta: ScrollDelta::Lines { x: 0.0, y: -1.0 },
                modifiers: Modifiers::empty(),
            });
            assert_eq!(
                e.browser.drawer.first,
                first + 1,
                "the wheel scrolls the list"
            );
        }
        let (x, y) = presets_ui::row_centre(&e.browser.drawer, i).expect("shown");
        let pt = in_drawer(&e, (x, y));
        click(&mut e, pt);
        assert_eq!(preset(&params), "Warp Field");
        let calls = host.take();
        let set = |p: ParamPtr| {
            calls
                .iter()
                .any(|c| matches!(c, Call::Set(q, _) if *q == p))
        };
        assert!(
            set(params.osc2_on.as_ptr())
                && set(params.cutoff.as_ptr())
                && set(params.mod_wheel.as_ptr())
        );
        assert!(!set(params.pitch_wheel.as_ptr()) && !set(params.bypass.as_ptr()));
        // Each a gesture of its own.
        let begins = calls.iter().filter(|c| matches!(c, Call::Begin(_))).count();
        let ends = calls.iter().filter(|c| matches!(c, Call::End(_))).count();
        assert_eq!(begins, ends);
        e.draw();
        assert_eq!(e.browser.bar.name, "Warp Field");
        assert!(e.browser.drawer.rows[i].current);
        // A double click on a row chooses it and closes the drawer.
        e.browser.scroll(-100);
        let j = row_of(&e, "Brass Choir");
        let pt = in_drawer(
            &e,
            presets_ui::row_centre(&e.browser.drawer, j).expect("shown"),
        );
        click(&mut e, pt);
        click(&mut e, pt);
        assert_eq!(preset(&params), "Brass Choir");
        assert!(!e.browser.open);
        slid(&mut e);
        assert_eq!(e.window_height(), shut);
        // Escape closes it too.
        open(&mut e);
        assert!(key(&mut e, Key::Escape));
        slid(&mut e);
        assert!(!e.browser.open);
        assert!(
            !key(&mut e, Key::Character("a".into())),
            "closed: the host's keys"
        );
    }

    /// The editor's width, fitted to the screen, stays while it is open, and is measured again
    /// for its next opening.
    #[test]
    fn the_width_stays_until_the_editor_opens_again() {
        let ed = Ca72Editor::new(Arc::new(Ca72Params::default()), Arc::new(Meters::default()));
        let (w, h) = ed.opening();
        ed.fitted.store(w, Ordering::Relaxed);
        assert_eq!(ed.size(), (w, h));
        drop(ed.closing());
        assert_eq!(
            ed.fitted.load(Ordering::Relaxed),
            0,
            "measured again next time"
        );
    }

    thread_local! {
        /// The usable screen a test's editor measures, in physical pixels.
        static SCREEN: std::cell::Cell<Option<(f64, f64)>> = const { std::cell::Cell::new(None) };
    }

    /// A test's screen, in the window's units where the host gave a scale (as Windows'
    /// [`screen::usable`]).
    fn test_screen(scale: Option<f64>) -> Option<(f64, f64)> {
        let k = scale.unwrap_or(1.0);
        SCREEN.get().map(|(w, h)| (w / k, h / k))
    }

    /// An editor measuring [`SCREEN`].
    fn on_test_screen() -> Ca72Editor {
        let mut ed = Ca72Editor::new(Arc::new(Ca72Params::default()), Arc::new(Meters::default()));
        ed.screen = test_screen;
        ed
    }

    /// The editor opens at the size the host was told, whichever screen is in use by then: the
    /// standalone was told its size on the Mac's Retina screen (the main screen then), its
    /// window opened on a 1x monitor, and the editor, measuring that monitor as it opened, was
    /// drawn wider than its window, cut off on the right (R-TWO-SCREENS).
    #[test]
    fn the_editor_opens_at_the_size_the_host_was_told() {
        let (retina, monitor) = ((1728.0, 1084.0), (1920.0, 1055.0));
        assert_ne!(
            fitted_width(Some(retina)),
            fitted_width(Some(monitor)),
            "screens that fit different widths"
        );
        let ed = on_test_screen();
        SCREEN.set(Some(retina));
        let told = ed.size();
        assert_eq!(told.0, fitted_width(Some(retina)));
        SCREEN.set(Some(monitor));
        assert_eq!(ed.opening(), told, "the window's size");
        ed.open.store(true, Ordering::Release);
        assert_eq!(ed.size(), told, "asked again while open");
        drop(ed.closing());
        assert_eq!(
            ed.size().0,
            fitted_width(Some(monitor)),
            "the screen in use measured when it next opens"
        );
    }

    /// A host that asks nothing before it opens the editor has it fitted to the screen in use
    /// then; one that gives a scale after asking has the screen measured again in that scale's
    /// pixels (Windows, Linux).
    #[test]
    fn the_screen_is_measured_where_the_host_has_not_asked_in_this_scale() {
        let ed = on_test_screen();
        SCREEN.set(Some((2560.0, 1400.0)));
        assert_eq!(ed.opening().0, fitted_width(Some((2560.0, 1400.0))));
        drop(ed.closing());

        let ed = on_test_screen();
        let told = ed.size();
        assert!(ed.set_scale_factor(2.0));
        let scaled = ed.size();
        assert_eq!(scaled.0, fitted_width(Some((1280.0, 700.0))));
        assert_ne!(scaled, told);
        assert!(ed.set_scale_factor(2.0));
        SCREEN.set(Some((1920.0, 1040.0)));
        assert_eq!(
            ed.opening(),
            scaled,
            "the same scale: the size the host was told"
        );
    }

    /// The drawer drops down from under the rail over the strip's sections and slides back up
    /// under it as it shuts: the panel and the rail stay as they are, the window its size, the
    /// host never asked to resize it (A6; it had grown the window below the strip). A row there
    /// sets the plug-in.
    #[test]
    fn the_drawer_drops_over_the_strip_and_leaves_the_panel() {
        let dir = tempfile::tempdir().unwrap();
        let (mut e, host, _params) = editing_with(Library::at(dir.path()));
        e.draw();
        open(&mut e);
        let c = e.composed.as_ref().expect("the window's frame");
        let frame = e.renderer.frame();
        let viewport_width = e.physical_width as usize;
        let row = |p: &ca72_panel::Pixmap, y: u32| {
            let w = p.width() as usize;
            let left = y as usize * w * 4;
            p.data()[left..left + viewport_width * 4].to_vec()
        };
        let s = e.renderer.scale();
        let panel = ((art::PANEL_H - 40.0) * s) as u32;
        let low = ((art::H - 100.0) * s) as u32;
        assert_eq!(row(c, panel), row(frame, panel), "the panel as it is");
        assert_ne!(row(c, low), row(frame, low), "the drawer over the strip");
        let open_row = row(c, low);
        assert_eq!(host.resizes.load(Ordering::Relaxed), 0);
        // A row there sets the plug-in: one on the list's first page, not the current preset.
        let i = e.browser.drawer.first + 1;
        let name = e.browser.drawer.rows[i].name.clone();
        assert!(!e.browser.drawer.rows[i].current);
        let pt = in_drawer(
            &e,
            presets_ui::row_centre(&e.browser.drawer, i).expect("shown"),
        );
        click(&mut e, pt);
        e.draw();
        assert_eq!(e.browser.bar.name, name);
        // Shut: it slides back up (held at the slide's start), the window as it is.
        assert!(key(&mut e, Key::Escape));
        e.follow_drawer();
        e.browser
            .slide_began(Instant::now() + Duration::from_secs(3600));
        e.draw();
        assert!(!e.browser.open && e.browser.sliding());
        assert_eq!(e.window_height(), height_for(1720));
        e.browser
            .slide_began(Instant::now() - crate::presets::SLIDE * 2);
        e.draw();
        let c = e.composed.as_ref().expect("the window's frame");
        assert_ne!(row(c, low), open_row, "the strip again");
        assert_eq!(host.resizes.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn typing_searches_and_the_arrows_step_through_the_list() {
        let dir = tempfile::tempdir().unwrap();
        let (mut e, _host, params) = editing_with(Library::at(dir.path()));
        open(&mut e);
        typed(&mut e, "GLIDE");
        let names: Vec<&str> = e
            .browser
            .drawer
            .rows
            .iter()
            .map(|r| r.name.as_str())
            .collect();
        assert!(names.contains(&"Singing Drift") && names.contains(&"Horn Bloom"));
        assert!(!names.contains(&"Felt Bass"), "{names:?}");
        key(&mut e, Key::Backspace);
        assert_eq!(e.browser.drawer.search.text, "GLID");
        key(&mut e, Key::ArrowDown);
        let first = e.browser.drawer.rows[0].name.clone();
        assert_eq!(preset(&params), first);
        key(&mut e, Key::ArrowDown);
        assert_eq!(preset(&params), e.browser.drawer.rows[1].name);
        key(&mut e, Key::ArrowUp);
        assert_eq!(preset(&params), first);
        // The host's shortcuts stay the host's.
        let setter = ParamSetter::new(e.context.as_ref());
        assert!(!e.browser.key(
            &KeyboardEvent {
                state: KeyState::Down,
                key: Key::Character("s".into()),
                modifiers: Modifiers::CONTROL,
                ..KeyboardEvent::default()
            },
            &e.params,
            &setter,
        ));
        // A tag chip filters; the bar's arrows step too.
        key(&mut e, Key::Escape);
        open(&mut e);
        let bass = e
            .browser
            .drawer
            .chips
            .iter()
            .position(|(t, _)| t == "prog")
            .expect("a chip");
        let setter = ParamSetter::new(e.context.as_ref());
        e.browser
            .drawer_press(DrawerTarget::Chip(bass), 0.0, &e.params, &setter);
        let names: Vec<&str> = e
            .browser
            .drawer
            .rows
            .iter()
            .map(|r| r.name.as_str())
            .collect();
        assert_eq!(names, ["Singing Drift"]);
        let pt = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Next)));
        click(&mut e, pt);
        assert_eq!(preset(&params), "Singing Drift");
    }

    /// An arrow choosing a preset ends a wheel's gesture still open on a knob it sets.
    #[test]
    fn a_key_choosing_a_preset_ends_a_gesture_first() {
        let dir = tempfile::tempdir().unwrap();
        let (mut e, host, _params) = editing_with(Library::at(dir.path()));
        e.draw();
        open(&mut e);
        let p = at(&e, centre("cutoff"));
        go(&mut e, p);
        wheel(&mut e, 1.0);
        assert!(e.keyed(&KeyboardEvent {
            state: KeyState::Down,
            key: Key::ArrowDown,
            ..KeyboardEvent::default()
        }));
        let calls = host.take();
        well_formed(&calls);
        assert!(calls.iter().filter(|c| matches!(c, Call::Begin(_))).count() > 2);
    }

    #[test]
    fn saved_renamed_tagged_deleted_and_restored_from_the_drawer() {
        let dir = tempfile::tempdir().unwrap();
        let (mut e, _host, params) = editing_with(Library::at(dir.path()));
        // SAVE… opens the drawer at the name; Tab to the tags; Enter saves.
        e.draw();
        let pt = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Save)));
        click(&mut e, pt);
        assert_eq!(e.browser.drawer.focus, Some(FieldId::SaveName));
        std::thread::sleep(crate::presets::SLIDE + Duration::from_millis(20));
        e.draw();
        typed(&mut e, "My Bass");
        key(&mut e, Key::Tab);
        typed(&mut e, "bass, test");
        key(&mut e, Key::Enter);
        assert!(dir.path().join("My Bass.toml").exists());
        assert_eq!(preset(&params), "My Bass");
        let lib = Library::at(dir.path());
        let mine = lib.find("My Bass").unwrap();
        assert_eq!(mine.sound.tags, ["bass", "test"]);
        // Its values: every parameter but the PITCH wheel and the bypass, plain.
        let ids: Vec<&str> = mine.sound.values.iter().map(|(k, _)| k.as_str()).collect();
        assert!(ids.contains(&"cutoff") && ids.contains(&"osc1_range") && ids.contains(&"voices"));
        assert!(!ids.contains(&"pitch_wheel") && !ids.contains(&"bypass"));
        assert!(mine.sound.values.contains(&("osc1_range".to_owned(), 3.0)));
        // SAVE AS: the preset's name to begin with; a factory preset's name, it says it replaces
        // it; Escape takes the save back.
        let key_at = |e: &Editing, k: PresetKey| in_drawer(e, presets_ui::key_centre(k));
        let pt = key_at(&e, PresetKey::SaveAs);
        click(&mut e, pt);
        assert_eq!(e.browser.drawer.save_name.text, "My Bass");
        assert_eq!(e.browser.drawer.focus, Some(FieldId::SaveName));
        for _ in 0.."My Bass".len() {
            key(&mut e, Key::Backspace);
        }
        typed(&mut e, "Felt Bass");
        assert!(e.browser.drawer.replace);
        assert!(e.browser.drawer.hint.contains("factory"));
        key(&mut e, Key::Escape);
        assert_eq!(e.browser.drawer.editing, None);
        assert!(e.browser.open);
        // RENAME, for the preset set: renamed in place (the plug-in keeps it).
        let pt = key_at(&e, PresetKey::Rename);
        click(&mut e, pt);
        assert_eq!(e.browser.drawer.focus, Some(FieldId::Edit));
        for _ in 0.."My Bass".len() {
            key(&mut e, Key::Backspace);
        }
        typed(&mut e, "Your Bass");
        key(&mut e, Key::Enter);
        assert!(lib.find("Your Bass").is_some() && lib.find("My Bass").is_none());
        assert_eq!(preset(&params), "Your Bass");
        // TAGS: its tags edited.
        let pt = key_at(&e, PresetKey::Tags);
        click(&mut e, pt);
        key(&mut e, Key::End);
        typed(&mut e, ", deep");
        key(&mut e, Key::Enter);
        assert_eq!(
            lib.find("Your Bass").unwrap().sound.tags,
            ["bass", "test", "deep"]
        );
        // DELETE: deleted once asked (DELETE again).
        let pt = key_at(&e, PresetKey::Delete);
        click(&mut e, pt);
        assert!(lib.find("Your Bass").is_some(), "deleted before asked");
        e.draw();
        click(&mut e, pt);
        assert!(lib.find("Your Bass").is_none());
        e.draw();
        assert!(!e.browser.bar.found, "the bar says the preset is gone");
        // A factory preset deleted (Enter once asked), then restored; the star makes a
        // favourite.
        let i = row_of(&e, "Singing Drift");
        let setter = ParamSetter::new(e.context.as_ref());
        e.browser
            .drawer_press(DrawerTarget::Row(i), 0.0, &e.params, &setter);
        e.browser.drawer_press(
            DrawerTarget::Key(PresetKey::Delete),
            0.0,
            &e.params,
            &setter,
        );
        key(&mut e, Key::Enter);
        assert!(
            e.browser
                .drawer
                .rows
                .iter()
                .all(|r| r.name != "Singing Drift")
        );
        let setter = ParamSetter::new(e.context.as_ref());
        e.browser.drawer_press(
            DrawerTarget::Key(PresetKey::Restore),
            0.0,
            &e.params,
            &setter,
        );
        let i = row_of(&e, "Singing Drift");
        e.browser
            .drawer_press(DrawerTarget::Star(i), 0.0, &e.params, &setter);
        assert!(lib.find("Singing Drift").unwrap().favorite);
    }

    #[test]
    fn a_frame_is_shown_again_on_linux_only() {
        let now = Instant::now();
        assert!(
            repaint_due(None, None, now),
            "the first frame is always shown"
        );
        assert!(!repaint_due(Some(now), None, now));
        let later = now + Duration::from_secs(1);
        assert_eq!(
            repaint_due(Some(now), None, later),
            cfg!(target_os = "linux")
        );
        // Settling after a size first shown (R-LINUX-RESIZE): again at every frame, until then.
        let until = SETTLE.map(|for_| now + for_);
        let next = now + Duration::from_millis(15);
        assert_eq!(
            repaint_due(Some(now), until, next),
            cfg!(target_os = "linux")
        );
        assert!(
            !repaint_due(Some(next), until, now + Duration::from_millis(260)),
            "settled"
        );
    }

    /// The window as it opens (the panel and the strip under it), written to
    /// `$CA72_EDITOR_PNG` for a look: the README's `docs/panel.png`.
    #[test]
    #[ignore = "writes an image for a look"]
    fn the_window_png() {
        let Some(out) = std::env::var_os("CA72_EDITOR_PNG") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let (mut e, _host, _params) = editing_with(Library::at(dir.path()));
        window_frame(&mut e).save_png(out).unwrap();
    }

    /// The large lettering (R-LETTERING) beside the standard, for a look: in
    /// `$CA72_LETTERING_PNG`, the folder, the window as it opens on a 1920 by 1080 screen (1380
    /// wide) in each look and lettering, `<look>-<lettering>.png`; and in the realistic look with
    /// the settings' menu open, `settings-<lettering>.png`, CUTOFF FREQUENCY's,
    /// `knob-<lettering>.png`, and the SIZE menu, `size-<lettering>.png` (R-SIZE); and the
    /// pointer over the grip, its tip shown, `grip-<lettering>.png`.
    #[test]
    #[ignore = "writes images for a look"]
    fn the_lettering_pngs() {
        let Some(out) = std::env::var_os("CA72_LETTERING_PNG").map(std::path::PathBuf::from) else {
            return;
        };
        std::fs::create_dir_all(&out).unwrap();
        for (name, lettering) in [
            ("standard", Lettering::Standard),
            ("large", Lettering::Large),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let host = Arc::new(Host {
                api: Some(PluginApi::Clap),
                ..Default::default()
            });
            let context: Arc<dyn GuiContext> = host.clone();
            let mut e = Editing::new(
                context,
                Arc::new(Ca72Params::default()),
                Arc::new(Meters::default()),
                (1380, 1380, 1.0),
                Library::at(dir.path()),
                |_| None,
            );
            e.set_lettering(lettering);
            for (look, skin) in [("classic", Skin::Drawn), ("realistic", Skin::Worn)] {
                e.set_look(skin);
                window_frame(&mut e)
                    .save_png(out.join(format!("{look}-{name}.png")))
                    .unwrap();
            }
            let key = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Settings)));
            click(&mut e, key);
            window_frame(&mut e)
                .save_png(out.join(format!("settings-{name}.png")))
                .unwrap();
            let away = at(&e, (0.0, 0.0));
            click(&mut e, away);
            let knob = at(&e, centre("cutoff"));
            right_click(&mut e, knob);
            window_frame(&mut e)
                .save_png(out.join(format!("knob-{name}.png")))
                .unwrap();
            let away = at(&e, (0.0, 0.0));
            click(&mut e, away);
            click(&mut e, key);
            menu_item(&mut e, "SIZE\u{2026}");
            window_frame(&mut e)
                .save_png(out.join(format!("size-{name}.png")))
                .unwrap();
            click(&mut e, away);
            let grip = at(&e, (art::W - 50.0, art::H - 30.0));
            go(&mut e, grip);
            window_frame(&mut e)
                .save_png(out.join(format!("grip-{name}.png")))
                .unwrap();
        }
    }

    /// VOICES dimmed where it does nothing and lit where it counts (decisions.md R64), in
    /// each look, for a look: `voices-<look>-<state>.png` in `$CA72_VOICES_PNG`, the folder.
    #[test]
    #[ignore = "writes images for a look"]
    fn the_window_voices_pngs() {
        let Some(out) = std::env::var_os("CA72_VOICES_PNG").map(std::path::PathBuf::from) else {
            return;
        };
        let amount = |name: &str, v: f32| {
            FloatParam::new(
                name,
                v,
                FloatRange::Linear {
                    min: 0.0,
                    max: 100.0,
                },
            )
        };
        let states = [
            ("mono-width-0", false, false, 0.0),
            ("mono-double", false, true, 60.0),
            ("mono-width-100", false, false, 100.0),
            ("poly", true, false, 60.0),
        ];
        for (look, skin) in [("realistic", Skin::Worn), ("classic", Skin::Drawn)] {
            for (state, poly, doubled, width) in states {
                let params = Ca72Params {
                    poly: BoolParam::new("Poly", poly),
                    doubled: BoolParam::new("Double", doubled),
                    spread: amount("Width", width),
                    ..Ca72Params::default()
                };
                let dir = tempfile::tempdir().unwrap();
                let (mut e, _host, _params) = editing_of(params, Library::at(dir.path()));
                // (Not `set_look`, which keeps the look in this computer's settings.)
                e.renderer = Renderer::with_skin(skin, e.renderer.scale(), e.dpr);
                e.composed = None;
                window_frame(&mut e)
                    .save_png(out.join(format!("voices-{look}-{state}.png")))
                    .unwrap();
            }
        }
    }

    /// The window's frame as shown (the panel's renderer's whole drawing, the strip's parts over
    /// its foot, the drawer and what floats over them: `compose`), for a look.
    fn window_frame(e: &mut Editing) -> &ca72_panel::Pixmap {
        e.draw();
        e.composed.as_ref().expect("the window's frame")
    }

    /// QUALITY's opening (decisions.md R48) for looking at, written to `$CA72_ULTRA_PNG`,
    /// the folder: the panel's last column close up (two pixels a unit) at QUALITY's three
    /// positions, each's thing up at rest and as the synth plays, `quality-<lo|hi|ultra>-<level>
    /// .png`; each coming up, `shutter-<setting>-<p>.png`; none of it, `hidden.png`; and the
    /// window at ULTRA with its note, `note.png`, and the opening's menu, `menu.png`.
    #[test]
    #[ignore = "writes images for a look"]
    fn the_ultra_pngs() {
        let Some(out) = std::env::var_os("CA72_ULTRA_PNG").map(std::path::PathBuf::from) else {
            return;
        };
        std::fs::create_dir_all(&out).unwrap();
        let k = 2.0;
        let mut r = Renderer::with_skin(Skin::Worn, k, 1.0);
        let crop = |r: &Renderer, name: &str| {
            let f = r.frame();
            let (x0, y0) = (
                ((art::COL + 2930.0) * k) as u32,
                ((art::TOP - 20.0) * k) as u32,
            );
            let (w, h) = (((3108.0 - 2930.0) * k) as u32, (640.0 * k) as u32);
            let mut c = ca72_panel::Pixmap::new(w, h).unwrap();
            for y in 0..h {
                let from = ((y0 + y) * f.width() + x0) as usize * 4;
                let to = (y * w) as usize * 4;
                c.data_mut()[to..to + w as usize * 4]
                    .copy_from_slice(&f.data()[from..from + w as usize * 4]);
            }
            c.save_png(out.join(name)).unwrap();
        };
        let q = ca72_panel::controls::index("quality").unwrap();
        let mut scene = Scene {
            values: [0.5; CONTROLS.len()],
            ..Scene::default()
        };
        use ca72_panel::ultra::Opening;
        // QUALITY's three positions, each's thing up: at rest, and at the synth's levels.
        for (name, v) in [("lo", 0.0), ("hi", 0.5), ("ultra", 1.0)] {
            scene.values[q] = v;
            for level in [0.0, 0.5, 1.0] {
                let mut o = Opening {
                    level,
                    spark: 1.3,
                    run: if level > 0.0 { 1.0 } else { 0.0 },
                    asleep: if level > 0.0 { 0.0 } else { 1.0 },
                    stride: 3,
                    turn: 20.0,
                    ..Opening::default()
                };
                match name {
                    "lo" => o.hamster = 1.0,
                    "hi" => o.lamp = 1.0,
                    _ => o.coil = 1.0,
                }
                scene.opening = o;
                r.render(&scene);
                crop(&r, &format!("quality-{name}-{level}.png"));
            }
        }
        // Each coming up through the shutter.
        for (name, v) in [("lo", 0.0), ("hi", 0.5), ("ultra", 1.0)] {
            scene.values[q] = v;
            for p in [0.0, 0.16, 0.32, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0] {
                let mut o = Opening {
                    level: 0.6,
                    spark: 2.1,
                    run: p,
                    stride: (p * 7.0) as u8,
                    ..Opening::default()
                };
                match name {
                    "lo" => o.hamster = p,
                    "hi" => o.lamp = p,
                    _ => o.coil = p,
                }
                scene.opening = o;
                r.render(&scene);
                crop(&r, &format!("shutter-{name}-{p}.png"));
            }
        }
        // None of it: the panel blank above QUALITY.
        scene.opening = Opening {
            hidden: true,
            coil: 1.0,
            ..Opening::default()
        };
        r.render(&scene);
        crop(&r, "hidden.png");
        let dir = tempfile::tempdir().unwrap();
        let (mut e, _host, _params) = editing_with(Library::at(dir.path()));
        e.draw();
        e.scene.values[q] = 1.0;
        e.scene.opening = Opening {
            coil: 1.0,
            ..Opening::default()
        };
        e.scene.note = Some(e.ultra_note());
        e.renderer.render(&e.scene);
        e.renderer.frame().save_png(out.join("note.png")).unwrap();
        // Both of its texts: a computer that splits an ULTRA voice, and one that does not.
        for (splits, name) in [(true, "note-split.png"), (false, "note-whole.png")] {
            e.scene.note = Some(e.ultra_note_for(splits));
            e.renderer.render(&e.scene);
            e.renderer.frame().save_png(out.join(name)).unwrap();
        }
        // QUALITY's menu, its toggle right-clicked.
        e.ultra_note = false;
        e.settings.ultra_note_read = true;
        let size = e.renderer.text_size() * MENU_TEXT;
        let at = (art::COL + 3023.0, art::TOP + 266.0);
        e.learning
            .open_quality_menu(e.renderer.fonts(), at, size, e.settings.shutter);
        e.scene.note = None;
        e.scene.menu = e.learning.menu().cloned();
        e.renderer.render(&e.scene);
        e.renderer.frame().save_png(out.join("menu.png")).unwrap();
    }

    /// ULTRA's note as this computer plays it (decisions.md R-THREADS): where it splits an
    /// ULTRA voice over six threads, that one voice plays live; where it does not, what one
    /// voice takes. Either fits where the old one did, left of the lamp at the window's opening
    /// size: no line wider than the old note's widest, its box within the panel.
    #[test]
    fn ultra_s_note_says_what_this_computer_plays() {
        let (e, _host, _params) = editing();
        let split = e.ultra_note_for(true);
        let whole = e.ultra_note_for(false);
        let text = |n: &Note| -> Vec<String> { n.lines.iter().map(|(t, _)| t.clone()).collect() };
        assert_eq!(
            text(&e.ultra_note()),
            text(&e.ultra_note_for(crate::engine::split_voices() > 0)),
            "not as this computer plays it"
        );
        for (n, says) in [
            (
                &split,
                "ON THIS COMPUTER ONE VOICE IS SPREAD OVER SIX THREADS AND",
            ),
            (
                &whole,
                "ONE VOICE TAKES UP TO A WHOLE CORE OF A FAST COMPUTER.",
            ),
        ] {
            assert_eq!(
                n.lines[0],
                ("ULTRA · HIGH FIDELITY".to_owned(), Tone::Accent)
            );
            assert!(
                text(n)
                    .iter()
                    .any(|t| t.contains("WITHIN -120 dBFS OF ITS NUMERICAL REFERENCE"))
            );
            assert!(text(n).iter().any(|t| t == says), "{:?}", text(n));
            let dim: Vec<_> = n.lines.iter().rev().take(2).collect();
            assert_eq!(dim[0], &("CLICK TO CLOSE".to_owned(), Tone::Dim));
            assert_eq!(
                dim[1].0,
                "RIGHT-CLICK QUALITY'S SWITCH TO HIDE ITS INDICATOR, OR SHOW IT."
            );
        }
        assert!(
            text(&split)
                .join(" ")
                .contains("WITH MULTIPLE VOICES OR TOO SMALL OF A BUFFER")
        );
        assert!(
            text(&whole)
                .iter()
                .any(|t| t.ends_with("PLAYED LIVE, IT MAY DROP OUT."))
        );
        // The old note's widest line, as wide as any may be.
        let fonts = e.renderer.fonts();
        let wide = |t: &str| fonts.advance(t, split.size, Weight::Regular, 0.0);
        let old = [
            "THE CIRCUIT'S MODEL EXACTLY, AT ABOUT SEVEN TIMES THE WORK OF HI:",
            "ONE VOICE TAKES ABOUT A WHOLE CORE OF A FAST COMPUTER.",
            "IT IS MEANT FOR OFFLINE RENDERS (EXPORT, BOUNCE, FREEZE) AND SMALL",
            "PROJECTS ON POWERFUL COMPUTERS. PLAYED LIVE, IT MAY DROP OUT.",
            "RIGHT-CLICK QUALITY'S SWITCH TO HIDE ITS INDICATOR, OR SHOW IT.",
        ]
        .map(wide)
        .into_iter()
        .fold(0.0, f64::max);
        for n in [&split, &whole] {
            for (t, _) in &n.lines {
                assert!(wide(t) <= old + 1e-9, "{t}: {} against {old}", wide(t));
            }
            // Its box: left of the lamp's opening, within the panel (not pushed in by the
            // panel's edge), its foot within the panel's height.
            let (w, h) = n.extent(fonts);
            let (left, right) = (n.x - w / 2.0, n.x + w / 2.0);
            let top = n.y - n.size * 0.4 - h;
            assert!(
                left >= 0.0 && right < art::COL + 3023.0 - 53.6,
                "{left}..{right}"
            );
            assert!(top >= 0.0 && n.y <= art::PANEL_H, "{top}..{}", n.y);
        }
    }

    /// MIDI Learn drawn (decisions.md R34), for looking at: CUTOFF being learned (ringed, its
    /// note) with MAIN OUTPUT VOLUME's menu open, then POLY being learned after a reserved
    /// controller was moved, then the strip's INNER being learned with its menu open, each as
    /// `learn-<n>.png` in `$CA72_LEARN_PNG`, the folder.
    #[test]
    fn the_window_learning_png() {
        let Some(out) = std::env::var_os("CA72_LEARN_PNG") else {
            return;
        };
        let out = std::path::PathBuf::from(out);
        let dir = tempfile::tempdir().unwrap();
        let shot = |e: &mut Editing, name: &str| {
            window_frame(e).save_png(out.join(name)).unwrap();
        };
        let (mut e, _host, params) = editing_with(Library::at(dir.path()));
        params
            .midi_map
            .assign(learnable("volume"), crate::learn::Cc { channel: 0, cc: 7 });
        params.midi_map.arm(learnable("cutoff"));
        let p = at(&e, centre("volume"));
        right_click(&mut e, p);
        shot(&mut e, "learn-1.png");
        e.learning.close_menu();
        params.midi_map.arm(learnable("poly"));
        params.midi_map.refuse(1);
        shot(&mut e, "learn-2.png");
        params.midi_map.arm(learnable("inner"));
        let p = at(&e, centre("inner"));
        right_click(&mut e, p);
        shot(&mut e, "learn-3.png");
        for (target, name) in [(BarTarget::Prev, "previous"), (BarTarget::Next, "next")] {
            e.learning.cancel(&params.midi_map);
            let point = on_strip(&e, strip_at(StripTarget::Bar(target)));
            right_click(&mut e, point);
            shot(&mut e, &format!("preset-{name}-menu.png"));
            menu_item(&mut e, "MIDI LEARN");
            shot(&mut e, &format!("preset-{name}-learning.png"));
        }
    }

    /// The settings key's menu with FREQUENCY DIALS (decisions.md R54), and oscillator 2's
    /// knob at +7 semitones with its tip, through each law: `settings-<n>.png` in
    /// `$CA72_SETTINGS_PNG`, the folder.
    #[test]
    fn the_window_settings_png() {
        use nih_plug::context::process::TestProcessContext;
        let Some(out) = std::env::var_os("CA72_SETTINGS_PNG") else {
            return;
        };
        let out = std::path::PathBuf::from(out);
        let dir = tempfile::tempdir().unwrap();
        let (mut e, _host, params) = editing_with(Library::at(dir.path()));
        let ps: Arc<dyn Params> = params.clone();
        let c = TestProcessContext::<crate::Ca72>::new(ps, 48_000.0, ProcessMode::Realtime);
        c.automate(params.osc2_frequency.as_ptr(), Pot::Two.wiper(7.0) as f32);
        let key = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Settings)));
        click(&mut e, key);
        window_frame(&mut e)
            .save_png(out.join("settings-1.png"))
            .unwrap();
        e.learning.close_menu();
        for (n, law) in [(2, Law::Semitones), (3, Law::Hardware)] {
            e.set_frequency_dials(law);
            let p = at(&e, centre("osc2_frequency"));
            go(&mut e, p);
            window_frame(&mut e)
                .save_png(out.join(format!("settings-{n}.png")))
                .unwrap();
        }
    }

    /// SETTINGS offers AGENTS in the standalone alone (decisions.md R-MCP): ALLOW and OFF, the
    /// one in force marked; off until chosen. `agents-<n>.png` in `$CA72_AGENTS_PNG`.
    #[test]
    fn agents_are_allowed_in_the_standalones_settings_menu() {
        let out = std::env::var_os("CA72_AGENTS_PNG").map(std::path::PathBuf::from);
        let (mut e, _host, _) = editing();
        let key = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Settings)));
        click(&mut e, key);
        assert!(
            !menu_items(&e).iter().any(|(t, _)| t == "AGENTS"),
            "not in a host"
        );
        e.learning.close_menu();
        let (mut e, _host, _) = editing_standalone();
        assert!(!e.settings.agents && !e.agents_allowed());
        click(&mut e, key);
        let items = menu_items(&e);
        let at = items
            .iter()
            .position(|(t, _)| t == "AGENTS")
            .expect("AGENTS");
        assert_eq!(
            items[at + 1..at + 3],
            [("ALLOW".to_owned(), true), ("OFF".to_owned(), false)]
        );
        if let Some(out) = &out {
            window_frame(&mut e)
                .save_png(out.join("agents-1.png"))
                .unwrap();
        }
        menu_item(&mut e, "ALLOW");
        assert!(e.settings.agents && e.agents_allowed());
        click(&mut e, key);
        let items = menu_items(&e);
        let at = items
            .iter()
            .position(|(t, _)| t == "AGENTS")
            .expect("AGENTS");
        assert_eq!(
            items[at + 1..at + 3],
            [("ALLOW".to_owned(), false), ("OFF".to_owned(), true)]
        );
        if let Some(out) = &out {
            window_frame(&mut e)
                .save_png(out.join("agents-2.png"))
                .unwrap();
        }
        menu_item(&mut e, "OFF");
        assert!(!e.agents_allowed());
    }

    /// The drawer's update check (decisions.md R27): its button the editor's, not the
    /// presets'; a check that could not be made offers the releases' page. No parameter is
    /// touched and the drawer stays open.
    #[test]
    fn the_drawer_checks_for_updates() {
        use crate::update::{RELEASES, fake, shown};
        let dir = tempfile::tempdir().unwrap();
        let (mut e, host, _params) = editing_with(Library::at(dir.path()));
        e.update = Update::with(fake::no_curl, fake::browser);
        e.draw();
        open(&mut e);
        let s = &e.browser.drawer.update;
        assert_eq!(
            (s.text.as_str(), s.button.as_str()),
            (format!("CA-72 {}", shown()).as_str(), "CHECK FOR UPDATES")
        );
        let button = in_drawer(&e, presets_ui::update_centre());
        go(&mut e, button);
        assert_eq!(e.browser.drawer.hover, Some(DrawerTarget::Update));
        click(&mut e, button);
        e.draw();
        assert_eq!(e.browser.drawer.update.text, "COULD NOT CHECK");
        click(&mut e, button);
        assert_eq!(fake::opened(), vec![RELEASES.to_owned()]);
        assert!(e.browser.open);
        assert_eq!(host.take(), vec![]);
    }

    /// The compact view, its controls shown, and presets temporarily revealing it, using
    /// the editor's actual frames for the visual preview.
    #[test]
    #[ignore = "writes images for a look"]
    fn the_lower_panel_png() {
        let Some(out) = std::env::var_os("CA72_PANEL_PNG") else {
            return;
        };
        let out = std::path::PathBuf::from(out);
        std::fs::create_dir_all(&out).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let (mut e, _, _) = editing_with(Library::at(dir.path()));
        window_frame(&mut e).save_png(out.join("full.png")).unwrap();
        let panel = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Panel)));
        click(&mut e, panel);
        window_frame(&mut e)
            .save_png(out.join("short.png"))
            .unwrap();
        open(&mut e);
        window_frame(&mut e)
            .save_png(out.join("presets.png"))
            .unwrap();
    }

    /// The window's frames stacked (the panel with the drawer over it, the strip, the bar),
    /// written to `$CA72_EDITOR_PNG` for a look.
    #[test]
    #[ignore = "writes an image for a look"]
    fn the_window_with_the_drawer_open_png() {
        let Some(out) = std::env::var_os("CA72_EDITOR_PNG") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let (mut e, _host, _params) = editing_with(Library::at(dir.path()));
        e.draw();
        open(&mut e);
        let i = row_of(&e, "Brass Choir");
        let pt = in_drawer(
            &e,
            presets_ui::row_centre(&e.browser.drawer, i).expect("shown"),
        );
        click(&mut e, pt);
        let pt = in_drawer(
            &e,
            presets_ui::row_centre(&e.browser.drawer, i + 1).expect("shown"),
        );
        go(&mut e, pt);
        window_frame(&mut e).save_png(out).unwrap();
    }

    /// The LFO page and modulation as the editor shows them, for looking at, written to
    /// `$CA72_LFO_PNG`, the folder: `page.png` (the page, two LFOs on CUTOFF and one on VOLUME,
    /// the orbs moving), `drag.png` (an assignment dragged from LFO 1's jack over EMPHASIS),
    /// `hover.png` (the strip's page, CUTOFF under the pointer: its rings, pop-up and tip),
    /// `menu.png` (CUTOFF's menu), `lfo-menu.png` (LFO 2's own), `glow.png` (the page away, an
    /// LFO moving: its key glowing), `mode-tip.png` and `mode-tip-4.png` (MODE's tips).
    #[test]
    #[ignore = "writes images for a look"]
    fn the_lfo_pngs() {
        use crate::lfo::{Reading, Source, Target as T};
        let Some(out) = std::env::var_os("CA72_LFO_PNG").map(std::path::PathBuf::from) else {
            return;
        };
        std::fs::create_dir_all(&out).unwrap();
        let (mut e, _, params) = editing();
        let patch = Arc::clone(&params.lfo_patch);
        patch.assign(Source::Lfo(0), T::Cutoff, 0.4).unwrap();
        patch.assign(Source::Lfo(1), T::Cutoff, -0.25).unwrap();
        patch
            .assign(Source::Lfo(0), T::Osc2Frequency, -0.1)
            .unwrap();
        patch.assign(Source::Lfo(2), T::Volume, -0.5).unwrap();
        let moving = |k: f32| Reading {
            value: (k * 2.3).sin(),
            phase: (k * 0.37) % 1.0,
            since: 0.4 + k,
            fade: 1.0,
            moving: true,
            ..Reading::default()
        };
        params
            .lfo_scope
            .publish(&[moving(0.1), moving(0.7), moving(1.3), Reading::default()]);
        e.playing.visible = false;
        lfo_open(&mut e);
        e.modulating.due = None;
        let save = |e: &mut Editing, name: &str| {
            window_frame(e).save_png(out.join(name)).unwrap();
        };
        save(&mut e, "page.png");
        // (MODE's tips, LFO 1's and LFO 4's: clear of the scopes, inside the window.)
        for (n, name) in [(0, "mode-tip.png"), (3, "mode-tip-4.png")] {
            let mode = at(&e, lfo_centre(n, 5));
            go(&mut e, mode);
            save(&mut e, name);
        }
        let jack = at(&e, ca72_panel::strip::lfo::places(0).jack);
        go(&mut e, jack);
        e.mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            modifiers: Modifiers::empty(),
        });
        let emphasis = at(&e, centre("emphasis"));
        go(&mut e, emphasis);
        save(&mut e, "drag.png");
        e.end_gestures();
        let (x, y, w, h) = ca72_panel::strip::lfo::places(1).scope;
        let scope = at(&e, (x + w / 2.0, y + h / 2.0));
        right_click(&mut e, scope);
        save(&mut e, "lfo-menu.png");
        e.learning.close_menu();
        let l = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Global)));
        click(&mut e, l);
        let cutoff = at(&e, centre("cutoff"));
        go(&mut e, cutoff);
        save(&mut e, "hover.png");
        right_click(&mut e, cutoff);
        save(&mut e, "menu.png");
        e.learning.close_menu();
        go(&mut e, Point { x: 5.0, y: 5.0 });
        save(&mut e, "glow.png");
        // (The LFOs moving each other, R-LFO's addendum: `cross.png` the page with LFO 1 and 2's
        // RATEs in a loop, LFO 3 on its own PHASE, LFO 2 on LFO 1's LEVEL and macro 1 on LFO
        // 4's; `cross-hover.png` LFO 1's RATE under the pointer, its pop-up; `level-hover.png`
        // LFO 1's jack, its LEVEL's rings and tip.)
        patch.assign(Source::Lfo(1), T::LfoRate(0), 0.4).unwrap();
        patch.assign(Source::Lfo(0), T::LfoRate(1), -0.3).unwrap();
        patch.assign(Source::Lfo(2), T::LfoPhase(2), 0.5).unwrap();
        patch.assign(Source::Lfo(1), T::LfoLevel(0), -0.6).unwrap();
        patch.assign(Source::Macro(0), T::LfoLevel(3), 0.8).unwrap();
        lfo_open(&mut e);
        e.modulating.due = None;
        let (sx, sy, sw, sh) = ca72_panel::strip::lfo::places(3).scope;
        let p = at(&e, (sx + sw / 2.0, sy + sh / 2.0));
        go(&mut e, p);
        save(&mut e, "cross.png");
        let rate = at(&e, lfo_centre(0, 1));
        go(&mut e, rate);
        save(&mut e, "cross-hover.png");
        go(&mut e, Point { x: 5.0, y: 5.0 });
        let jack = at(&e, ca72_panel::strip::lfo::places(0).jack);
        go(&mut e, jack);
        save(&mut e, "level-hover.png");
    }

    // ---- The LFO page and modulation (decisions.md R-LFO).

    /// The LFO page's knobs take exact values typed at them (R-KNOB-ENTRY), in their dials'
    /// units: RATE in Hz, PHASE in degrees, FADE in seconds (signed, or IN and OUT).
    #[test]
    fn the_lfo_knobs_take_values_typed_in_their_units() {
        let (mut e, host, params) = editing();
        lfo_open(&mut e);
        host.take();
        let q = params.lfo(0);
        for (k, text, p, want) in [
            (3, "90", &q.phase, 90.0),
            (4, "1", &q.fade, 1.0),
            // (FADE signed, or IN and OUT: decisions.md R-LFO-FADE.)
            (4, "-1.2", &q.fade, -1.2),
            (4, "Out 0.5 s", &q.fade, -0.5),
            (4, "in 2", &q.fade, 2.0),
            (4, "-20", &q.fade, -10.0),
            (1, "5 Hz", &q.rate, 5.0),
        ] {
            let id = format!("lfo1_{}", ca72_panel::controls::LFO_PARTS[k]);
            entry_click(&mut e, &id);
            assert!(e.knob_entry.is_some(), "{id} opens");
            keyed(&mut e, Key::Character(text.into()));
            keyed(&mut e, Key::Enter);
            let calls = host.take();
            let set = calls.iter().find_map(|c| match c {
                Call::Set(ptr, v) if *ptr == p.as_ptr() => Some(*v),
                _ => None,
            });
            let got = p.preview_plain(set.unwrap_or_else(|| panic!("{id}: {calls:?}")));
            assert!((got - want).abs() < 0.01 * want.max(1.0), "{id}: {got}");
        }
    }

    /// An LFO's FADE knob (decisions.md R-LFO-FADE): its tip reads Off at its centre, In and Out
    /// to either side; let go within its detent of the centre it settles there, off, but let go
    /// further it stays, and a notch of the mouse wheel from the centre steps out of the detent.
    #[test]
    fn the_fade_knob_reads_in_and_out_and_settles_at_off() {
        use ca72_panel::controls::LFO_PARTS;
        use ca72_panel::interact::FADE_DETENT;
        let (mut e, host, params) = editing();
        lfo_open(&mut e);
        host.take();
        let k = LFO_PARTS.iter().position(|p| *p == "fade").expect("FADE");
        let f = params.lfo(0).fade.as_ptr();
        let p = at(&e, lfo_centre(0, k));
        go(&mut e, p);
        e.draw();
        let tip = |e: &Editing| e.tip().expect("FADE's tip").0;
        assert!(tip(&e).starts_with("LFO 1 FADE: Off"), "{}", tip(&e));
        let last_set = |calls: &[Call]| {
            calls.iter().rev().find_map(|c| match c {
                Call::Set(ptr, v) if *ptr == f => Some(*v),
                _ => None,
            })
        };
        // Dragged up, in; down, out; let go within the detent, off.
        press(&mut e);
        go(&mut e, below(p, -60.0));
        e.draw();
        assert!(last_set(&host.take()).is_some_and(|v| v > 0.6));
        assert!(tip(&e).starts_with("LFO 1 FADE: In "), "{}", tip(&e));
        go(&mut e, below(p, 60.0));
        e.draw();
        assert!(last_set(&host.take()).is_some_and(|v| v < 0.4));
        assert!(tip(&e).starts_with("LFO 1 FADE: Out "), "{}", tip(&e));
        go(&mut e, below(p, -1.0));
        let near = last_set(&host.take()).expect("moved");
        assert!(near > 0.5 && f64::from(near) - 0.5 <= FADE_DETENT, "{near}");
        release(&mut e);
        assert_eq!(host.take(), vec![Call::Set(f, 0.5), Call::End(f)]);
        // Let go further: where it was let go. (Not a double click, which puts it back.)
        e.last_press = None;
        go(&mut e, p);
        press(&mut e);
        go(&mut e, below(p, 12.0));
        let out = last_set(&host.take()).expect("moved");
        assert!(0.5 - f64::from(out) > FADE_DETENT, "{out}");
        release(&mut e);
        assert_eq!(host.take(), vec![Call::End(f)]);
        // A notch of the wheel from the centre: out of the detent, and kept.
        e.draw();
        wheel(&mut e, 1.0);
        assert_eq!(host.take(), vec![Call::Begin(f), Call::Set(f, 0.52)]);
        e.end_scroll();
        assert_eq!(host.take(), vec![Call::End(f)]);
    }

    /// The LFO page opened by the rail's LFO key (`lfo_page::Modulating::open`).
    fn lfo_open(e: &mut Editing) {
        e.draw();
        let p = on_strip(e, strip_at(StripTarget::Bar(BarTarget::Lfo)));
        click(e, p);
        assert!(e.page_shows());
        e.draw();
    }

    fn lfo_centre(n: usize, k: usize) -> (f64, f64) {
        CONTROLS[ca72_panel::controls::lfo_index(n, k)].centre()
    }

    /// Each scope shows one cycle of its LFO's waveform across its width, from PHASE (the owner,
    /// 2026-10-20: "why does every waveform show two cycles? it should be 1"), stepped and
    /// CONTINUOUS alike, and timed the same cycle (one note value) whatever the tempo; RATE,
    /// FADE and MODE leave it as it is. The orb is where the LFO is in that cycle, at its
    /// output, its wake at FADE's gain.
    #[test]
    fn each_scope_shows_one_cycle_from_its_phase() {
        use crate::lfo::{LfoMode, LfoParams, Reading, Source, Target as T, wave};
        use crate::params::{Wave, WaveMode};
        use ca72_panel::strip::lfo::TRACE_POINTS;
        let k = TRACE_POINTS - 1;
        let traces = |phase: f32, timed: bool| {
            let lfo = |n: usize| LfoParams {
                waveform: EnumParam::new(format!("LFO {n} Waveform"), Wave::Sawtooth),
                shape_mode: EnumParam::new(
                    format!("LFO {n} Waveform Mode"),
                    if n == 4 {
                        WaveMode::Continuous
                    } else {
                        WaveMode::Stepped
                    },
                ),
                shape: FloatParam::new(
                    format!("LFO {n} Shape"),
                    0.5,
                    FloatRange::Linear { min: 0.0, max: 1.0 },
                ),
                phase: FloatParam::new(
                    format!("LFO {n} Phase"),
                    phase,
                    FloatRange::Linear {
                        min: 0.0,
                        max: 360.0,
                    },
                ),
                timed: BoolParam::new(format!("LFO {n} Timed"), timed && n == 3),
                fade: FloatParam::new(
                    format!("LFO {n} Fade"),
                    if n == 2 { 2.0 } else { 0.0 },
                    crate::lfo::FADE_RANGE,
                ),
                mode: EnumParam::new(
                    format!("LFO {n} Mode"),
                    if n == 2 { LfoMode::Env } else { LfoMode::Free },
                ),
                ..LfoParams::new(n)
            };
            let (mut e, _, params) = editing_of(
                Ca72Params {
                    lfo1: lfo(1),
                    lfo2: lfo(2),
                    lfo3: lfo(3),
                    lfo4: lfo(4),
                    ..Ca72Params::default()
                },
                Library::at(std::env::temp_dir().join("ca72-editor-tests-no-library")),
            );
            lfo_open(&mut e);
            (e.lfo_scene_now(), e, params)
        };
        let (at0, _, _) = traces(0.0, false);
        let (at90, mut e, params) = traces(90.0, true);
        for n in 0..4 {
            let set = crate::lfo::settings(&params, n, crate::lfo::Edges::Soft);
            let (t0, t90) = (&at0.columns[n].trace, &at90.columns[n].trace);
            assert_eq!(t90.len(), TRACE_POINTS);
            for i in 0..=k {
                let x = i as f64 / k as f64 * (1.0 - 1e-9);
                assert_eq!(t0[i], wave(set.at, x) as f32, "LFO {}: {i}", n + 1);
                assert_eq!(t90[i], wave(set.at, 0.25 + x) as f32, "LFO {}: {i}", n + 1);
            }
            // (A sawtooth's one fall: one cycle, its start at PHASE; LFO 4's is CONTINUOUS.)
            if n < 3 {
                let falls = |t: &[f32]| t.windows(2).filter(|w| w[1] < w[0] - 0.5).count();
                assert_eq!(falls(t0), 0, "LFO {}: from its start to its end", n + 1);
                assert_eq!(falls(t90), 1, "LFO {}: a quarter on", n + 1);
            }
        }
        // (ENV with FADE, LFO 2's, and timed, LFO 3's: the same cycle as LFO 1's.)
        assert_eq!(at90.columns[1].trace, at90.columns[0].trace);
        assert_eq!(at90.columns[2].trace, at90.columns[0].trace);
        assert_ne!(at90.columns[3].trace, at90.columns[0].trace);
        // The orb: where the reading has the LFO in its cycle, at its output.
        params
            .lfo_patch
            .assign(Source::Lfo(0), T::Cutoff, 0.4)
            .unwrap();
        params
            .lfo_patch
            .assign(Source::Lfo(1), T::Cutoff, 0.4)
            .unwrap();
        let r = |phase: f32, value: f32, fade: f32, moving: bool| Reading {
            value,
            phase,
            since: 0.3,
            fade,
            moving,
            ..Reading::default()
        };
        params.lfo_scope.publish(&[
            r(0.75, 0.3, 1.0, true),
            r(0.25, -0.5, 0.4, false),
            r(0.5, 0.1, 0.5, true),
            Reading::default(),
        ]);
        e.modulating.due = None;
        e.read_lfos(Instant::now());
        let s = e.lfo_scene_now();
        let set = |n| crate::lfo::settings(&params, n, crate::lfo::Edges::Soft);
        assert_eq!(s.columns[0].orb, Some((0.5, f64::from(0.3f32))));
        assert_eq!(s.columns[0].orb_fade, 1.0);
        // (ENV assigned, its cycle over: held at the cycle's end, on the trace.)
        assert_eq!(s.columns[1].orb, Some((1.0, set(1).cycle_at(1.0))));
        assert_eq!(s.columns[1].orb_fade, 1.0);
        assert_eq!(s.columns[2].orb, Some((0.25, f64::from(0.1f32))));
        assert_eq!(
            s.columns[2].orb_fade, 0.5,
            "FADE fading in: its wake so high"
        );
    }

    /// Where an orb is along its one cycle: from PHASE, a cycle's start at 0 exactly; ENV at its
    /// cycle's end once over, never begun, or in f32 its last instant.
    #[test]
    fn an_orb_is_where_its_lfo_is_in_its_cycle() {
        use crate::lfo::{Mode, Reading, Settings};
        use lfo_page::orb_at;
        let set = |phase: f64, mode: Mode| Settings {
            phase,
            hz: 2.0,
            mode,
            ..Settings::default()
        };
        let r = |phase: f32, since: f32, moving: bool| Reading {
            phase,
            since,
            moving,
            ..Reading::default()
        };
        for mode in [Mode::Free, Mode::Retrig] {
            let s = set(0.25, mode);
            assert_eq!(orb_at(&s, &r(0.25, 0.1, true), 120.0, true), 0.0);
            assert_eq!(orb_at(&s, &r(0.75, 0.1, true), 120.0, true), 0.5);
            assert!((orb_at(&s, &r(0.1, 0.1, true), 120.0, true) - 0.85).abs() < 1e-6);
            // (PHASE in f32 as the reading's: its start not read as just short of the end.)
            let third = set(1.0 / 3.0, mode);
            let start = r((1.0f64 / 3.0) as f32, 0.0, true);
            assert_eq!(orb_at(&third, &start, 120.0, true), 0.0);
        }
        let env = set(0.5, Mode::Env);
        assert!((orb_at(&env, &r(0.8, 0.15, true), 120.0, true) - 0.3).abs() < 1e-6);
        assert_eq!(orb_at(&env, &r(0.5, 2.0, false), 120.0, true), 1.0, "over");
        assert_eq!(
            orb_at(&env, &r(0.5, 0.0, false), 120.0, false),
            1.0,
            "never begun"
        );
        assert_eq!(
            orb_at(&env, &r(0.5, 0.25, false), 120.0, false),
            0.5,
            "not assigned"
        );
        let at0 = set(0.0, Mode::Env);
        assert_eq!(
            orb_at(&at0, &r(0.0, 0.499, true), 120.0, true),
            1.0,
            "its last instant"
        );
        assert_eq!(
            orb_at(&at0, &r(0.0, 0.0, true), 120.0, true),
            0.0,
            "its first"
        );
    }

    /// No tip on the LFO page covers a scope (the owner, 2026-10-20, saw MODE's over its
    /// scope's corner): each control's, the jack's and the screen's, in every column, MODE at
    /// each of its places, with the timed key on and off.
    #[test]
    fn no_tip_on_the_lfo_page_covers_a_scope() {
        use crate::lfo::{LfoMode, LfoParams};
        use ca72_panel::controls::LFO_PARTS;
        use ca72_panel::strip::lfo::{LfoTarget, places, target_centre};
        for timed in [false, true] {
            for mode in [LfoMode::Free, LfoMode::Retrig, LfoMode::Env] {
                let lfo = |n: usize| LfoParams {
                    timed: BoolParam::new(format!("LFO {n} Timed"), timed),
                    mode: EnumParam::new(format!("LFO {n} Mode"), mode),
                    ..LfoParams::new(n)
                };
                let (mut e, _, _) = editing_of(
                    Ca72Params {
                        lfo1: lfo(1),
                        lfo2: lfo(2),
                        lfo3: lfo(3),
                        lfo4: lfo(4),
                        ..Ca72Params::default()
                    },
                    Library::at(std::env::temp_dir().join("ca72-editor-tests-no-library")),
                );
                lfo_open(&mut e);
                let size = e.renderer.text_size();
                let hovered = |e: &mut Editing, p: (f64, f64), what: &str| {
                    go(e, Point { x: 5.0, y: 5.0 });
                    let p = at(e, p);
                    go(e, p);
                    e.draw();
                    let (t, x, y) = e
                        .scene
                        .tip
                        .clone()
                        .unwrap_or_else(|| panic!("{what}: a tip"));
                    let l = art::tip(e.renderer.fonts(), x, y, &t, size);
                    let (x0, y0) = (l.origin.0 + l.bounds[0], l.origin.1 + l.bounds[1]);
                    let (x1, y1) = (l.origin.0 + l.bounds[2], l.origin.1 + l.bounds[3]);
                    assert!(
                        x0 >= 0.0 && x1 <= art::W,
                        "{what}: its tip {t:?} ({x0:.0}–{x1:.0}) leaves the window"
                    );
                    for m in 0..4 {
                        let (sx, sy, sw, sh) = places(m).scope;
                        assert!(
                            x1 <= sx || x0 >= sx + sw || y1 <= sy || y0 >= sy + sh,
                            "{what} ({timed}, {mode:?}): its tip {t:?} \
                             ({x0:.0}, {y0:.0})–({x1:.0}, {y1:.0}) covers scope {m}"
                        );
                    }
                };
                for n in 0..4 {
                    for (k, part) in LFO_PARTS.iter().enumerate() {
                        hovered(&mut e, lfo_centre(n, k), &format!("LFO {} {part}", n + 1));
                    }
                    for t in [LfoTarget::Jack(n), LfoTarget::Screen(n)] {
                        hovered(&mut e, target_centre(t), &format!("{t:?}"));
                    }
                }
            }
        }
    }

    /// The rail's LFO key (the owner, 2026-10-20: "the LFO rail key after the keyboard key")
    /// shows the page in the keyboard's place, its key lit, the strip's own parts not drawn under
    /// it; pressed lit, it stays. The keyboard's key shows the keyboard in its place; the
    /// page's controls answer only while it shows.
    #[test]
    fn the_lfo_key_shows_its_page_in_the_keyboards_place() {
        let (mut e, _, _) = editing();
        e.playing.visible = false;
        e.draw();
        assert!(!e.browser.bar.lfo_page);
        let rate = ca72_panel::controls::lfo_index(0, 1);
        assert!(e.off_panel(rate), "not found while the page is away");
        lfo_open(&mut e);
        assert!(e.browser.bar.lfo_page && e.strip_scene.lfo && !e.strip_scene.extension());
        assert!(!e.off_panel(rate));
        let p = at(&e, lfo_centre(0, 1));
        go(&mut e, p);
        assert_eq!(e.hover, Some(Target::Control(rate)));
        // The keyboard's key: the keyboard there instead.
        let k = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Keyboard)));
        click(&mut e, k);
        e.draw();
        assert!(!e.page_shows() && e.keyboard_shown() && e.strip_scene.keyboard.is_some());
        assert!(e.browser.bar.keyboard && !e.browser.bar.lfo_page);
        lfo_open(&mut e);
        assert!(e.strip_scene.keyboard.is_none() && !e.browser.bar.keyboard);
        let l = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Lfo)));
        click(&mut e, l);
        assert!(e.page_shows(), "the lit key leaves its page showing");
        let g = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Global)));
        click(&mut e, g);
        e.draw();
        assert!(!e.page_shows() && e.strip_scene.extension());
    }

    /// Every route through the three keys, PANEL off included, selects one page. The lit
    /// key is inert, and PANEL restores the selected page without changing its preference.
    #[test]
    fn the_three_lower_page_keys_select_and_remember_one_page() {
        let pages = [BarTarget::Global, BarTarget::Keyboard, BarTarget::Lfo];
        for standalone in [false, true] {
            let (mut e, host, _) = if standalone {
                editing_standalone()
            } else {
                editing()
            };
            if standalone {
                e.use_effects(crate::fx_view::FxView::new(Some(
                    crate::fx_view::fake::Fake::with(&[], &[]),
                )));
            }
            assert_eq!(
                e.rail,
                if standalone {
                    Rail::Effects
                } else {
                    Rail::Plugin
                }
            );
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("settings.toml");
            std::fs::write(&path, "keyboard_view = \"controls\"\nunknown = 42\n").unwrap();
            e.settings.keyboard = Some(false);
            e.playing.visible = false;
            e.settings_file = Some(path.clone());
            for from in pages {
                for hidden in [false, true] {
                    for to in pages {
                        e.show_lower_page(from);
                        let saved = e.settings.keyboard;
                        if hidden {
                            e.toggle_panel();
                        }
                        let p = on_strip(&e, strip::centre(e.rail, StripTarget::Bar(to)));
                        click(&mut e, p);
                        e.update_scene();
                        assert!(e.expanded(), "{from:?} -> {to:?}, hidden {hidden}");
                        assert_eq!(e.lower_page(), to);
                        let lights = pages.map(|t| strip::key_light_of(&e.browser.bar, t));
                        assert_eq!(lights.iter().filter(|&&n| n == 2).count(), 1);
                        assert_eq!(strip::key_light_of(&e.browser.bar, to), 2);
                        assert_eq!(
                            e.settings.keyboard,
                            match to {
                                BarTarget::Global => Some(false),
                                BarTarget::Keyboard => Some(true),
                                _ => saved,
                            }
                        );
                        let remembered = e.settings.keyboard;
                        let file = std::fs::read_to_string(&path).unwrap();
                        e.typing = Some((Typed::Strip(Readout::Level), "0".into()));
                        click(&mut e, p);
                        assert!(e.typing.is_some(), "lit key changes nothing");
                        assert_eq!(std::fs::read_to_string(&path).unwrap(), file);
                        e.typing = None;
                        e.toggle_panel();
                        e.update_scene();
                        assert!(
                            pages
                                .iter()
                                .all(|&t| strip::key_light_of(&e.browser.bar, t) < 2)
                        );
                        e.toggle_panel();
                        assert_eq!(e.lower_page(), to);
                        assert_eq!(e.settings.keyboard, remembered);
                    }
                }
            }
            assert!(
                host.take().is_empty(),
                "view keys never automate sound parameters"
            );
            let file = std::fs::read_to_string(&path).unwrap();
            assert!(file.contains("unknown = 42"));
            assert_eq!(Settings::read(&path).keyboard, e.settings.keyboard);
        }
    }

    #[test]
    fn page_keys_tips_name_their_print_and_the_learning_list_keeps_sound_controls() {
        for standalone in [false, true] {
            let (mut e, _, _) = if standalone {
                editing_standalone()
            } else {
                editing()
            };
            if standalone {
                e.use_effects(crate::fx_view::FxView::new(Some(
                    crate::fx_view::fake::Fake::with(&[], &[]),
                )));
            }
            assert_eq!(
                e.rail,
                if standalone {
                    Rail::Effects
                } else {
                    Rail::Plugin
                }
            );
            for (key, tip) in [
                (BarTarget::Global, "GLOBAL: VOICES, STEREO AND OUTPUT"),
                (BarTarget::Keyboard, "KEYS: PLAYABLE PIANO"),
                (BarTarget::Lfo, "LFO: MODULATION SOURCES"),
            ] {
                let point = on_strip(&e, strip::centre(e.rail, StripTarget::Bar(key)));
                go(&mut e, point);
                assert_eq!(e.tip().map(|t| t.0), Some(tip.into()));
            }
            let list = e.learning.list_scene(&e.params.midi_map);
            for name in ["KEYBOARD CONTROL 1", "KEYBOARD CONTROL 2"] {
                assert!(list.rows.iter().any(|row| row.name == name));
            }
            for name in ["GLOBAL", "KEYS", "LFO"] {
                assert!(!list.rows.iter().any(|row| row.name == name));
            }
        }
    }

    #[test]
    fn page_keys_release_the_pointer_keep_drawers_and_leave_lit_keys_alone() {
        let (mut e, _, _) = editing_standalone();
        e.show_lower_page(BarTarget::Keyboard);
        e.playing.press(60);
        assert!(e.playing.dragging());
        e.strip_press(StripTarget::Bar(BarTarget::Keyboard));
        assert!(e.playing.dragging(), "lit key keeps a pointer note");
        e.show_lower_page(BarTarget::Global);
        assert!(!e.playing.dragging());
        assert!(e.playing.notes.pop().unwrap().on);
        assert!(!e.playing.notes.pop().unwrap().on);
        e.show_lower_page(BarTarget::Keyboard);
        e.playing.press(62);
        e.show_lower_page(BarTarget::Lfo);
        assert!(!e.playing.dragging());
        e.toggle_lower(Lower::Arp);
        for page in [BarTarget::Global, BarTarget::Keyboard, BarTarget::Lfo] {
            e.show_lower_page(page);
            assert!(e.lower.is_open(Lower::Arp));
        }
        e.lower.shut();
        e.show_lower_page(BarTarget::Global);
        e.browser.show(true, &e.params);
        e.follow_drawer();
        e.show_lower_page(BarTarget::Keyboard);
        assert!(
            e.browser.open,
            "keyboard keeps the presets over it as before"
        );
        e.show_lower_page(BarTarget::Lfo);
        assert!(!e.browser.open, "LFO puts the presets away as before");
    }

    #[test]
    fn page_keys_leave_the_page_alone_when_the_host_refuses_to_show_panel() {
        let (mut e, host, _) = editing();
        e.show_lower_page(BarTarget::Keyboard);
        e.toggle_panel();
        host.fixed.store(true, Ordering::Relaxed);
        let saved = e.settings.keyboard;
        for page in [BarTarget::Global, BarTarget::Keyboard, BarTarget::Lfo] {
            e.strip_press(StripTarget::Bar(page));
            assert!(!e.expanded());
            assert_eq!(e.lower_page(), BarTarget::Keyboard);
            assert_eq!(e.settings.keyboard, saved);
        }
    }

    #[test]
    fn selecting_lfo_keeps_panel_shown_after_presets_temporarily_revealed_it() {
        let (mut e, _, _) = editing();
        e.show_lower_page(BarTarget::Global);
        e.toggle_panel();
        e.strip_press(StripTarget::Bar(BarTarget::Name));
        assert!(e.expanded() && e.browser.open);
        assert!(!e.params.editor_panel.load(Ordering::Relaxed));
        let p = on_strip(&e, strip::centre(e.rail, StripTarget::Bar(BarTarget::Lfo)));
        click(&mut e, p);
        assert!(e.expanded() && !e.browser.open);
        assert!(e.params.editor_panel.load(Ordering::Relaxed));
        assert_eq!(e.lower_page(), BarTarget::Lfo);
    }

    /// The real editor's full frames and rail crops, in each lettering, look, rail and scale.
    #[cfg(feature = "standalone-production")]
    #[test]
    #[ignore = "writes the rail keys' review pictures"]
    fn the_rail_keys_pngs() {
        let out = std::path::PathBuf::from(
            std::env::var_os("CA72_RAIL_KEYS_PNG").expect("an output folder"),
        );
        std::fs::create_dir_all(&out).unwrap();
        for standalone in [false, true] {
            let (mut e, _, _) = if standalone {
                editing_standalone()
            } else {
                editing()
            };
            if standalone {
                e.use_effects(crate::fx_view::FxView::new(Some(
                    crate::fx_view::fake::Fake::with(&[], &[]),
                )));
            }
            let rail = if standalone { "standalone" } else { "plugin" };
            for (look, skin) in [("worn", Skin::Worn), ("drawn", Skin::Drawn)] {
                e.set_look(skin);
                for (print, lettering) in
                    [("normal", Lettering::Standard), ("large", Lettering::Large)]
                {
                    e.set_lettering(lettering);
                    for scale in [0.44, 1.0] {
                        let width = (art::W * scale).round() as u32;
                        e.fit(1.0, width, width);
                        for (page, key) in [
                            ("global", BarTarget::Global),
                            ("keyboard", BarTarget::Keyboard),
                            ("lfo", BarTarget::Lfo),
                        ] {
                            e.strip_press(StripTarget::Bar(key));
                            go(&mut e, Point { x: 0.0, y: 0.0 });
                            let name = format!("{rail}-{look}-{print}-{scale}-{page}");
                            let top = e.renderer.strip_row();
                            let height = (strip::RAIL * e.renderer.scale()).round() as u32;
                            let frame = window_frame(&mut e).clone();
                            frame.save_png(out.join(format!("{name}.png"))).unwrap();
                            frame
                                .clone_rect(
                                    resvg::tiny_skia::IntRect::from_xywh(0, top, width, height)
                                        .unwrap(),
                                )
                                .unwrap()
                                .save_png(out.join(format!("{name}-rail.png")))
                                .unwrap();
                            let left = (1900.0 * e.renderer.scale()).round() as u32;
                            frame
                                .clone_rect(
                                    resvg::tiny_skia::IntRect::from_xywh(
                                        left as i32,
                                        top,
                                        width - left,
                                        height,
                                    )
                                    .unwrap(),
                                )
                                .unwrap()
                                .save_png(out.join(format!("{name}-right.png")))
                                .unwrap();
                            // Selecting pages must also clear the former light in a partial redraw.
                            e.composed = None;
                            assert_eq!(frame.data(), window_frame(&mut e).data());
                        }
                    }
                }
            }
        }
    }

    /// Each LFO's POLARITY, REVERSE and ON (R-LFO's corrections): UNI's key and ON's key (and
    /// the title, ON's word) flip their parameters by a click; REVERSE is WAVEFORM's menu's,
    /// marked while on. UNI, the trace runs 0..1 and its rings one way; reversed, the trace is
    /// the cycle backwards; off, MODULATES reads MUTED, the orb goes, its rings dim, and the
    /// tip and the pop-up say UNI and MUTED.
    #[test]
    fn an_lfos_polarity_reverse_and_on_show_and_operate() {
        use crate::learning::{ModItem, Row};
        use crate::lfo::{LfoKeys, Polarity, Source, Target as T};
        use ca72_panel::strip::lfo::{LfoTarget, target_centre};
        let (mut e, host, params) = editing();
        lfo_open(&mut e);
        host.take();
        let k = params.lfo_keys(1);
        for (place, ptr, to) in [
            (lfo_centre(1, 7), k.polarity.as_ptr(), 1.0),
            (lfo_centre(1, 8), k.on.as_ptr(), 0.0),
            (target_centre(LfoTarget::Title(1)), k.on.as_ptr(), 0.0),
        ] {
            let p = at(&e, place);
            click(&mut e, p);
            assert_eq!(
                host.take(),
                vec![Call::Begin(ptr), Call::Set(ptr, to), Call::End(ptr)]
            );
        }
        let wave = ca72_panel::controls::lfo_index(1, 0);
        let reverse = |e: &Editing| {
            e.knob_rows(wave).into_iter().find_map(|r| match r {
                Row::Item(ModItem::Reverse(1), t, true, marked) if t == "REVERSE" => Some(marked),
                _ => None,
            })
        };
        assert_eq!(reverse(&e), Some(false), "WAVEFORM's menu offers it");
        e.mod_item(ModItem::Reverse(1));
        let r = k.reverse.as_ptr();
        assert_eq!(
            host.take(),
            vec![Call::Begin(r), Call::Set(r, 1.0), Call::End(r)]
        );
        // As shown: LFO 2 UNI, reversed and off, on CUTOFF; LFO 1 as it was.
        let (mut e, _, params) = editing_of(
            Ca72Params {
                lfo2_keys: LfoKeys {
                    polarity: EnumParam::new("LFO 2 Polarity", Polarity::Uni),
                    reverse: BoolParam::new("LFO 2 Reverse", true),
                    on: BoolParam::new("LFO 2 On", false),
                },
                ..Ca72Params::default()
            },
            Library::at(std::env::temp_dir().join("ca72-editor-tests-no-library")),
        );
        params
            .lfo_patch
            .assign(Source::Lfo(1), T::Cutoff, 0.25)
            .expect("room");
        params
            .lfo_patch
            .assign(Source::Lfo(0), T::Cutoff, 0.4)
            .expect("room");
        assert_eq!(reverse(&e), Some(true), "marked");
        lfo_open(&mut e);
        let col = &e.lfo_scene.columns[1];
        assert!(col.uni && !col.on);
        assert_eq!(col.screen, "MUTED");
        assert!(!col.in_use && col.orb.is_none());
        assert!(
            col.trace.iter().all(|v| (0.0..=1.0).contains(v)),
            "UNI: 0..1"
        );
        // (TRIANGLE backwards from PHASE 0: from its top, the mirror of forwards.)
        let set = crate::lfo::settings(&params, 1, crate::lfo::Edges::Soft);
        let fwd = crate::lfo::Settings {
            reverse: false,
            ..set
        };
        for (i, v) in col.trace.iter().enumerate().skip(1) {
            let x = i as f64 / (col.trace.len() - 1) as f64;
            assert!((f64::from(*v) - fwd.cycle_at(1.0 - x)).abs() < 1e-3, "{i}");
        }
        let other = &e.lfo_scene.columns[0];
        assert!(!other.uni && other.on && other.screen != "MUTED");
        let cutoff = ca72_panel::controls::index("cutoff").expect("CUTOFF");
        let k = e
            .mod_scene
            .knobs
            .iter()
            .find(|k| k.control == cutoff)
            .expect("its rings")
            .clone();
        assert!(!k.rings[0].one_way && !k.rings[0].dim, "LFO 1's");
        assert!(k.rings[1].one_way && k.rings[1].dim, "LFO 2's");
        let p = at(&e, centre("cutoff"));
        go(&mut e, p);
        e.draw();
        let tip = e.scene.tip.clone().expect("a tip").0;
        assert!(tip.ends_with("\u{b7} LFO 2 +25 % UNI MUTED"), "{tip}");
        let popup = e.mod_scene.popup.clone().expect("its pop-up");
        assert!(popup.dials[1].uni && popup.dials[1].muted);
        assert_eq!(popup.dials[1].note(), "UNI \u{b7} MUTED");
        assert_eq!(popup.dials[0].note(), "");
    }

    /// The page's controls operate their LFO's parameters as the panel's do: a knob dragged in
    /// one gesture, MONO and the timed key flipped by a click, MODE's toggle to the place
    /// clicked; RATE timed turns its DIVISION, stepped.
    #[test]
    fn the_lfo_pages_controls_operate_their_parameters() {
        let (mut e, host, params) = editing();
        lfo_open(&mut e);
        host.take();
        let q = params.lfo(1);
        let p = at(&e, lfo_centre(1, 3));
        go(&mut e, p);
        e.mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            modifiers: Modifiers::empty(),
        });
        go(
            &mut e,
            Point {
                x: p.x,
                y: p.y - 30.0,
            },
        );
        release(&mut e);
        let calls = host.take();
        assert_eq!(calls.first(), Some(&Call::Begin(q.phase.as_ptr())));
        assert_eq!(calls.last(), Some(&Call::End(q.phase.as_ptr())));
        well_formed(&calls);
        let mono = at(&e, lfo_centre(1, 6));
        click(&mut e, mono);
        assert_eq!(
            host.take(),
            vec![
                Call::Begin(q.mono.as_ptr()),
                Call::Set(q.mono.as_ptr(), 0.0),
                Call::End(q.mono.as_ptr())
            ]
        );
        // MODE: its toggle on its side, ENV at its right end.
        let (mx, my) = lfo_centre(1, 5);
        let env = at(&e, (mx + 40.0, my));
        click(&mut e, env);
        assert!(host.take().contains(&Call::Set(q.mode.as_ptr(), 1.0)));
        // RATE timed: its DIVISION turned, a step at a time.
        let p3 = params.lfo(3);
        let timed = Ca72Params::default();
        let _ = timed;
        let i = ca72_panel::controls::lfo_index(3, 1);
        assert!(!e.timed_dial(i));
        assert!(matches!(
            e.kind_of(i),
            Kind::Knob {
                dial: ca72_panel::controls::Dial::LfoRate,
                ..
            }
        ));
        assert_eq!(
            e.operated(i).map(|o| o.text(o.default())),
            Some(
                p3.rate
                    .normalized_value_to_string(p3.rate.default_normalized_value(), true)
            )
        );
        // The last touched LFO is the column's.
        assert_eq!(e.modulating.last, Some(crate::lfo::Source::Lfo(1)));
    }

    /// An assignment dragged from an OUT jack (the owner's "Drag to knob"): while held, the cord
    /// and the knobs that can take it ringed, the one under the plug brightly with its preview
    /// ring at 25 %; let go there, the LFO modulates it at +25 % (Pigments' convention), its
    /// MODULATES screen saying so. Not on a switch; on another LFO's knob, that knob (R-LFO's
    /// addendum).
    #[test]
    fn an_assignment_dragged_from_out_lands_on_a_knob_at_a_quarter() {
        use crate::lfo::{Source, Target as T};
        let (mut e, host, params) = editing();
        lfo_open(&mut e);
        host.take();
        let jack = at(&e, ca72_panel::strip::lfo::places(1).jack);
        go(&mut e, jack);
        e.mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            modifiers: Modifiers::empty(),
        });
        let cutoff = at(&e, centre("cutoff"));
        go(&mut e, cutoff);
        e.draw();
        let cord = e.mod_scene.cord.clone().expect("a cord");
        assert!(cord.over.is_some(), "CUTOFF under the plug");
        assert!(
            cord.targets.len() >= 20,
            "{} knobs ringed",
            cord.targets.len()
        );
        let ringed = e
            .mod_scene
            .knobs
            .iter()
            .find(|k| CONTROLS[k.control].param == "cutoff")
            .expect("its preview");
        assert_eq!(ringed.rings[0].depth, f64::from(crate::lfo::DEFAULT_DEPTH));
        release(&mut e);
        assert_eq!(
            params.lfo_patch.depth(Source::Lfo(1), T::Cutoff),
            Some(0.25)
        );
        assert!(host.take().is_empty(), "patch state: no host gesture");
        assert!(e.modulating.patching.is_none());
        e.draw();
        assert!(e.mod_scene.cord.is_none());
        assert_eq!(e.lfo_scene.columns[1].screen, "CUTOFF +25");
        assert!(e.lfo_scene.columns[1].in_use);
        // Dropped on a switch: nothing; on another LFO's knob, that knob (R-LFO's addendum).
        for p in [at(&e, centre("osc_mod")), at(&e, lfo_centre(2, 3))] {
            go(&mut e, jack);
            e.mouse(MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                modifiers: Modifiers::empty(),
            });
            go(&mut e, p);
            release(&mut e);
        }
        assert_eq!(params.lfo_patch.count(Source::Lfo(1)), 2);
        assert_eq!(
            params.lfo_patch.depth(Source::Lfo(1), T::LfoPhase(2)),
            Some(0.25)
        );
    }

    /// A CONTINUOUS WAVEFORM's rings and pop-up take its own sweep, where its pointer stands at
    /// its travel's ends (R-LFO's corrections: about 199 degrees, not a knob's 300): its ring from
    /// its pointer as drawn, +25 % a quarter of that sweep, the pop-up's dial as far.
    #[test]
    fn a_continuous_waveforms_rings_take_its_own_sweep() {
        use crate::lfo::{Source, Target as T};
        use crate::params::{Wave, WaveMode};
        let (mut e, _host, params) = editing_of(
            Ca72Params {
                osc1_waveform: EnumParam::new("Osc 1 Waveform", Wave::Square),
                osc1_shape_mode: EnumParam::new("Osc 1 Waveform Mode", WaveMode::Continuous),
                ..Ca72Params::default()
            },
            Library::at(std::env::temp_dir().join("ca72-editor-tests-no-library")),
        );
        params
            .lfo_patch
            .assign(Source::Lfo(0), T::Osc1Shape, 0.25)
            .expect("room");
        let wave = at(&e, centre("osc1_waveform"));
        go(&mut e, wave);
        e.draw();
        let k = e
            .mod_scene
            .knobs
            .iter()
            .find(|k| CONTROLS[k.control].param == "osc1_waveform")
            .expect("its rings")
            .clone();
        assert_eq!(k.sweep, art::sweep(&CONTROLS[k.control].kind));
        assert!((k.span() - 5.95 * 33.5).abs() < 1e-9, "{}", k.span());
        let drawn = art::shape_angle(e.scene.shape[0].at);
        assert!((k.angle(k.travel) - drawn).abs() < 1e-9, "from its pointer");
        let (lo, hi, end) = ca72_panel::modulation::swing(&k, &k.rings[0]);
        let quarter = 0.25 * k.span();
        assert!((end - k.angle(k.travel) - quarter).abs() < 1e-9);
        assert!((hi - lo - 2.0 * quarter).abs() < 1e-9, "{lo}..{hi}");
        let popup = e.mod_scene.popup.clone().expect("its pop-up");
        assert_eq!(popup.span, k.span());
    }

    #[test]
    fn a_knobs_depth_popup_hides_on_press_and_cannot_intercept_its_drag() {
        use crate::lfo::{Source, Target as T};
        // A move may arrive before the next frame removes the cached popup.
        for redraw in [false, true] {
            let (mut e, host, params) = editing();
            for source in [Source::Lfo(2), Source::Macro(0)] {
                params
                    .lfo_patch
                    .assign(source, T::Cutoff, 0.15)
                    .expect("room");
            }
            let cutoff = at(&e, centre("cutoff"));
            go(&mut e, cutoff);
            e.draw();
            let popup = e.mod_scene.popup.clone().expect("hover shows the depths");
            let (_, dials) = ca72_panel::modulation::popup_layout(&popup);
            let over_depth = at(&e, dials[0].0);
            host.take();
            press(&mut e);
            if redraw {
                e.draw();
            }
            assert!(e.drag.is_some(), "the main knob is held");
            assert!(
                e.mod_scene_now().popup.is_none(),
                "hidden before any movement"
            );
            let before = e.drag.as_ref().unwrap().value;
            go(&mut e, over_depth);
            e.draw();
            assert!(e.mod_scene.popup.is_none(), "hidden throughout the drag");
            assert_ne!(e.drag.as_ref().unwrap().value, before);
            release(&mut e);
            well_formed(&host.take());
            for source in [Source::Lfo(2), Source::Macro(0)] {
                assert_eq!(params.lfo_patch.depth(source, T::Cutoff), Some(0.15));
            }
            go(&mut e, cutoff);
            e.draw();
            assert!(
                e.mod_scene.popup.is_some(),
                "hover works again after release"
            );
        }
    }

    /// The rings show only while the LFO page shows, an assignment is dragged, or on the knob
    /// under the pointer (the owner's "option 2"); there its pop-up shows a dial per source in its
    /// colour, dragged up for more depth; a ring's end dragged round sets it; Alt/Option-drag on
    /// the knob sets the last touched LFO's; the tip lists the depths; the knob's menu has a head
    /// per source with DEPTH… (typed) and REMOVE.
    #[test]
    fn depths_are_set_by_the_pop_up_the_ring_alt_drag_and_the_menu() {
        use crate::lfo::{Source, Target as T};
        let (mut e, host, params) = editing();
        let patch = Arc::clone(&params.lfo_patch);
        patch.assign(Source::Lfo(0), T::Cutoff, 0.4).expect("room");
        patch
            .assign(Source::Lfo(1), T::Cutoff, -0.25)
            .expect("room");
        e.playing.visible = false;
        e.draw();
        assert!(e.mod_scene.knobs.is_empty(), "the panel as it was");
        let cutoff = at(&e, centre("cutoff"));
        go(&mut e, cutoff);
        e.draw();
        assert_eq!(e.mod_scene.knobs.len(), 1, "its own rings");
        let tip = e.scene.tip.clone().expect("a tip").0;
        assert!(
            tip.ends_with("\u{b7} LFO 1 +40 % \u{b7} LFO 2 \u{2013}25 %"),
            "{tip}"
        );
        let popup = e.mod_scene.popup.clone().expect("its pop-up");
        assert_eq!(popup.dials.len(), 2);
        // The pop-up's second dial dragged up 24 pixels: 10 % more.
        let (_, dials) = ca72_panel::modulation::popup_layout(&popup);
        let d = at(&e, dials[1].0);
        go(&mut e, d);
        e.draw();
        assert!(e.mod_scene.popup.is_some(), "kept over itself");
        e.mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            modifiers: Modifiers::empty(),
        });
        go(
            &mut e,
            Point {
                x: d.x,
                y: d.y - 24.0,
            },
        );
        release(&mut e);
        assert!((patch.depth(Source::Lfo(1), T::Cutoff).unwrap() + 0.15).abs() < 1e-4);
        // The first ring's end dragged round to the knob's pointer: taken away.
        go(&mut e, cutoff);
        e.draw();
        let k = e.mod_scene.knobs[0].clone();
        let end = at(&e, ca72_panel::modulation::ring_end(&k, 0));
        go(&mut e, end);
        e.mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            modifiers: Modifiers::empty(),
        });
        let (px, py) = art::polar(k.skirt - 4.0, k.angle(k.travel));
        let to = at(&e, (k.at.0 + px, k.at.1 + py));
        go(&mut e, to);
        release(&mut e);
        assert_eq!(
            patch.depth(Source::Lfo(0), T::Cutoff),
            None,
            "at 0: removed"
        );
        // Alt-drag on EMPHASIS: the last touched LFO's depth (LFO 3's, its column pressed).
        e.modulating.last = Some(Source::Lfo(2));
        let emphasis = at(&e, centre("emphasis"));
        go(&mut e, emphasis);
        e.mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            modifiers: Modifiers::ALT,
        });
        go(
            &mut e,
            Point {
                x: emphasis.x,
                y: emphasis.y - 48.0,
            },
        );
        release(&mut e);
        assert!((patch.depth(Source::Lfo(2), T::Emphasis).unwrap() - 0.2).abs() < 1e-4);
        assert!(host.take().is_empty(), "EMPHASIS itself untouched");
        // CUTOFF's menu: LFO 2's head, its DEPTH… typed, its REMOVE.
        right_click(&mut e, cutoff);
        let items = menu_items(&e);
        assert!(
            items.iter().any(|(t, _)| t == "LFO 2 \u{b7} \u{2013}15 %"),
            "{items:?}"
        );
        menu_item(&mut e, "DEPTH\u{2026}");
        assert!(e.knob_entry.is_some());
        keyed(&mut e, Key::Character("-60".into()));
        keyed(&mut e, Key::Enter);
        assert!((patch.depth(Source::Lfo(1), T::Cutoff).unwrap() + 0.6).abs() < 1e-6);
        right_click(&mut e, cutoff);
        menu_item(&mut e, "REMOVE");
        assert_eq!(patch.depth(Source::Lfo(1), T::Cutoff), None);
    }

    /// The strip's knobs lie under the LFO page, so they take an LFO from their right-click menu:
    /// a head, LFO 1 to LFO 4; one it already has, not again.
    #[test]
    fn a_strip_knob_takes_an_lfo_from_its_menu() {
        use crate::lfo::{Source, Target as T};
        let (mut e, _, params) = editing();
        e.playing.visible = false;
        e.draw();
        let drive = at(&e, centre("drive"));
        right_click(&mut e, drive);
        menu_item(&mut e, "LFO 3");
        assert_eq!(params.lfo_patch.depth(Source::Lfo(2), T::Drive), Some(0.25));
        right_click(&mut e, drive);
        let items = menu_items(&e);
        assert!(items.contains(&("LFO 3".to_owned(), false)), "{items:?}");
        assert!(items.contains(&("LFO 1".to_owned(), true)));
        assert!(items.iter().any(|(t, _)| t == "LFO 3 \u{b7} +25 %"));
    }

    /// An LFO's own menu (a right click on its scope): its colour from the phosphors, kept on the
    /// computer; its EDGES, the patch's; the choice it is marked.
    #[test]
    fn an_lfos_menu_sets_its_colour_and_edges() {
        use crate::lfo::{Edges, Phosphor};
        let (mut e, _, params) = editing();
        lfo_open(&mut e);
        let (x, y, w, h) = ca72_panel::strip::lfo::places(2).scope;
        let scope = at(&e, (x + w / 2.0, y + h / 2.0));
        right_click(&mut e, scope);
        assert_eq!(
            e.learning.menu().map(|m| m.title.clone()),
            Some("LFO 3".into())
        );
        menu_item(&mut e, "VIOLET");
        assert_eq!(e.settings.lfo_colors[2], Phosphor::Violet);
        e.draw();
        assert_eq!(e.lfo_scene.columns[2].colour, Phosphor::Violet.rgb());
        right_click(&mut e, scope);
        menu_item(&mut e, "SHARP");
        assert_eq!(params.lfo_patch.edges(2), Edges::Sharp);
        right_click(&mut e, scope);
        let m = e.learning.menu().expect("a menu").clone();
        let sharp = m
            .items
            .iter()
            .position(|(t, _)| t == "SHARP")
            .expect("SHARP");
        assert!(m.marked.contains(&sharp) && !m.items[sharp].1);
    }

    /// The rail's page keys: lit while their page or drawer shows, a faint glow while what they
    /// open affects the sound (an LFO assigned and moving), else unlit.
    #[test]
    fn the_lfo_key_glows_while_an_lfo_moves_a_knob() {
        use crate::lfo::Reading;
        let (mut e, _, params) = editing();
        e.draw();
        assert!(!e.browser.bar.lfo);
        let moving = Reading {
            moving: true,
            ..Reading::default()
        };
        params.lfo_scope.publish(&[
            moving,
            Reading::default(),
            Reading::default(),
            Reading::default(),
        ]);
        e.draw();
        assert!(e.browser.bar.lfo && !e.browser.bar.lfo_page, "glowing");
        lfo_open(&mut e);
        assert!(e.browser.bar.lfo_page, "lit");
        let p = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Arp)));
        click(&mut e, p);
        e.draw();
        assert_eq!(
            e.browser.bar.page,
            Some(BarTarget::Arp),
            "a drawer over the page, lit too"
        );
        assert!(e.browser.bar.lfo_page);
    }

    // ---- The macros (decisions.md R-MACROS).

    /// The rail's MACRO key.
    fn macro_key(e: &mut Editing) {
        e.draw();
        let p = on_strip(e, strip_at(StripTarget::Bar(BarTarget::Macro)));
        click(e, p);
        e.draw();
    }

    /// Macro `n`'s value set as the host would have it (the test's host applies nothing).
    fn turn_macro(params: &Arc<Ca72Params>, n: usize, normalized: f32) {
        use nih_plug::context::process::TestProcessContext;
        let c =
            TestProcessContext::<crate::Ca72>::new(params.clone(), 48_000.0, ProcessMode::Realtime);
        assert!(c.automate(params.macro_param(n).as_ptr(), normalized));
    }

    /// The rail's MACRO key (the owner: "the far left edge ... just a button that says macro")
    /// flips the left area: the macros in the wheels' place, their knobs found and the wheels
    /// and switches not; pressed again, the wheels. Its light: lit while the macros show, a
    /// faint glow while the wheels do and an assigned macro is above 0, else unlit. The flip is
    /// the computer's (`Settings::macros_shown`).
    #[test]
    fn the_macro_key_flips_the_left_area() {
        use crate::lfo::{Source, Target as T};
        let (mut e, _, params) = editing();
        let knob = ca72_panel::controls::MACRO_FIRST;
        let wheel = ca72_panel::controls::index("mod_wheel").expect("the wheel");
        e.draw();
        assert!(!e.macros_show() && !e.browser.bar.macros && !e.browser.bar.macro_glow);
        assert!(e.off_panel(knob) && !e.off_panel(wheel));
        let (kx, ky) = CONTROLS[knob].centre();
        assert_ne!(e.hit(kx, ky), Some(Target::Control(knob)));
        macro_key(&mut e);
        assert!(e.macros_show() && e.settings.macros_shown && e.browser.bar.macros);
        assert!(!e.off_panel(knob) && e.off_panel(wheel));
        assert_eq!(e.hit(kx, ky), Some(Target::Control(knob)));
        let (wx, wy) = CONTROLS[wheel].centre();
        assert_ne!(
            e.hit(wx, wy),
            Some(Target::Control(wheel)),
            "the wheel hidden"
        );
        assert_eq!(e.macro_scene.cells[0].name, "MACRO 1");
        assert!(e.macro_scene.cells[0].dim);
        macro_key(&mut e);
        assert!(!e.macros_show() && !e.browser.bar.macros);
        // The glow: assigned and above 0.
        params
            .lfo_patch
            .assign(Source::Macro(2), T::Cutoff, 0.5)
            .expect("room");
        e.draw();
        assert!(!e.browser.bar.macro_glow, "at 0");
        turn_macro(&params, 2, 0.4);
        e.draw();
        assert!(e.browser.bar.macro_glow && !e.browser.bar.macros, "glowing");
        macro_key(&mut e);
        assert!(e.browser.bar.macros, "lit");
        // The key's light as the rail draws it.
        let light = |bar: &ca72_panel::presets::BarScene| {
            ca72_panel::strip::key_light_of(bar, BarTarget::Macro)
        };
        assert_eq!(light(&e.browser.bar), 2);
        macro_key(&mut e);
        assert_eq!(light(&e.browser.bar), 1);
        turn_macro(&params, 2, 0.0);
        e.draw();
        assert_eq!(light(&e.browser.bar), 0);
    }

    /// The stereo and output knobs take LFOs and macros (the owner, 2026-10-20: "make sure the
    /// stereo controls can be controlled by macros and lfos", "and entropy, drive, and
    /// output"): with the strip's page showing and the macros in, a macro's cord lands on WIDTH,
    /// INNER, DETUNE, ENTROPY, DRIVE and LEVEL, and on the panel's MAIN OUTPUT VOLUME and
    /// FEEDBACK; from the LFO page (over the strip) an LFO's cord lands on VOLUME and FEEDBACK;
    /// each strip knob's menu offers every LFO and every macro, and one chosen is assigned. Their
    /// rings take the knob's own sweep. Neither an assignment to DRIVE nor a macro on it turned
    /// here asks for AUTO GAIN's curve (R-AUTO-GAIN-DRIVE: only DRIVE itself changed here does).
    #[test]
    fn the_stereo_and_output_knobs_take_lfos_and_macros() {
        use crate::learning::{ModItem, Row};
        use crate::lfo::{Source, Target as T};
        let (mut e, _host, params) = editing();
        e.playing.visible = false;
        macro_key(&mut e);
        let drag = |e: &mut Editing, from: (f64, f64), to: &str| {
            let p = at(e, from);
            go(e, p);
            e.mouse(MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                modifiers: Modifiers::empty(),
            });
            let q = at(e, centre(to));
            go(e, q);
            e.draw();
            release(e);
            e.draw();
        };
        let knobs = [
            ("spread", T::Spread),
            ("inner", T::Inner),
            ("double", T::Double),
            ("entropy", T::Entropy),
            ("drive", T::Drive),
            ("level", T::Level),
            ("volume", T::Volume),
            ("feedback", T::Feedback),
        ];
        for (n, (id, t)) in knobs.iter().enumerate() {
            let m = n % crate::lfo::MACROS;
            drag(&mut e, ca72_panel::strip::macros::jack_at(m), id);
            assert_eq!(
                params.lfo_patch.depth(Source::Macro(m), *t),
                Some(0.25),
                "a macro's cord on {id}"
            );
        }
        assert!(!e.watched.take_moved(), "nothing for AUTO GAIN to measure");
        // Their rings: a quarter of the knob's own 300 degrees.
        let width = at(&e, centre("spread"));
        go(&mut e, width);
        e.draw();
        let k = e
            .mod_scene
            .knobs
            .iter()
            .find(|k| CONTROLS[k.control].param == "spread")
            .expect("WIDTH's rings")
            .clone();
        assert_eq!(k.sweep, art::KNOB_SWEEP);
        let (_, _, end) = ca72_panel::modulation::swing(&k, &k.rings[0]);
        assert!((end - k.angle(k.travel) - 75.0).abs() < 1e-9);
        // Each strip knob's menu: every LFO and every macro.
        for (id, t) in &knobs[..6] {
            let i = ca72_panel::controls::index(id).expect(id);
            let rows = e.knob_rows(i);
            for s in Source::all() {
                assert!(
                    rows.iter().any(
                        |r| matches!(r, Row::Item(ModItem::Assign(x, y), ..) if *x == s && y == t)
                    ),
                    "{id}'s menu offers {s:?}"
                );
            }
        }
        e.mod_item(ModItem::Assign(Source::Lfo(2), T::Level));
        assert_eq!(params.lfo_patch.depth(Source::Lfo(2), T::Level), Some(0.25));
        e.mod_item(ModItem::Assign(Source::Lfo(1), T::Drive));
        assert_eq!(params.lfo_patch.depth(Source::Lfo(1), T::Drive), Some(0.25));
        // A macro on DRIVE turned here: not DRIVE changed, nothing asked.
        e.set_once(ca72_panel::controls::MACRO_FIRST + 4, 0.8);
        assert!(!e.watched.take_moved(), "a macro on DRIVE turned");
        // From the LFO page, over the strip: VOLUME and FEEDBACK on the panel by cord.
        lfo_open(&mut e);
        for id in ["volume", "feedback"] {
            drag(&mut e, ca72_panel::strip::lfo::places(3).jack, id);
            let t = crate::lfo::Target::of_id(id).expect(id);
            assert_eq!(
                params.lfo_patch.depth(Source::Lfo(3), t),
                Some(0.25),
                "LFO 4 on {id}"
            );
        }
        assert!(!e.watched.take_moved());
    }

    /// The LFOs' own knobs take sources (decisions.md R-LFO's addendum; the owner, 2026-10-20:
    /// "LFOs to control each other's parameters", "Let's try B"): with the page open, LFO 2's cord
    /// lands on LFO 1's RATE, PHASE and FADE, on its own RATE, on LFO 1's WAVEFORM only while it
    /// is CONTINUOUS, on LFO 1's OUT jack as its LEVEL; LFO 1's cord let go on its own jack without
    /// leaving it gives nothing, drawn away and back its own LEVEL; a timed RATE takes none; a
    /// macro's cord lands on an LFO's RATE. The page's knobs then show rings (on their own
    /// sweeps; the jack's round it), the pop-up on hover, the tip's depths and the menu's
    /// DEPTH…/REMOVE; the jack's menu keeps the LFO's COLOR and EDGES; the wheel on the jack
    /// turns LEVEL.
    #[test]
    fn the_lfos_own_knobs_take_sources() {
        use crate::learning::{ModItem, Row};
        use crate::lfo::{Source, Target as T};
        use ca72_panel::controls::LFO_LEVEL_PART;
        use ca72_panel::strip::lfo::places;
        use nih_plug::context::process::TestProcessContext;
        let (mut e, host, params) = editing();
        e.playing.visible = false;
        lfo_open(&mut e);
        let drag = |e: &mut Editing, from: (f64, f64), to: (f64, f64)| {
            let p = at(e, from);
            go(e, p);
            e.mouse(MouseEvent::ButtonPressed {
                button: MouseButton::Left,
                modifiers: Modifiers::empty(),
            });
            let q = at(e, to);
            go(e, q);
            e.draw();
            release(e);
            e.draw();
        };
        let depth = |s, t| params.lfo_patch.depth(s, t);
        for (k, t) in [(1, T::LfoRate(0)), (3, T::LfoPhase(0)), (4, T::LfoFade(0))] {
            drag(&mut e, places(1).jack, lfo_centre(0, k));
            assert_eq!(depth(Source::Lfo(1), t), Some(0.25), "{t:?}");
        }
        drag(&mut e, places(1).jack, lfo_centre(1, 1));
        assert_eq!(depth(Source::Lfo(1), T::LfoRate(1)), Some(0.25), "its own");
        drag(&mut e, places(1).jack, lfo_centre(0, 0));
        assert_eq!(depth(Source::Lfo(1), T::LfoShape(0)), None, "STEPPED");
        drag(&mut e, places(1).jack, places(0).jack);
        assert_eq!(depth(Source::Lfo(1), T::LfoLevel(0)), Some(0.25), "LEVEL");
        // A click on its own jack: nothing; away and back: its own LEVEL.
        let jack = at(&e, places(0).jack);
        click(&mut e, jack);
        assert_eq!(params.lfo_patch.count(Source::Lfo(0)), 0, "a click");
        // (Past a double click's time: a second press then would put LEVEL back at 100 %.)
        e.last_press = None;
        go(&mut e, jack);
        e.mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            modifiers: Modifiers::empty(),
        });
        let away = at(&e, lfo_centre(0, 1));
        go(&mut e, away);
        go(&mut e, jack);
        release(&mut e);
        assert_eq!(
            depth(Source::Lfo(0), T::LfoLevel(0)),
            Some(0.25),
            "away and back"
        );
        // Timed, RATE takes none; CONTINUOUS, WAVEFORM its shape (the parameters as a host sets
        // them).
        let c =
            TestProcessContext::<crate::Ca72>::new(params.clone(), 48_000.0, ProcessMode::Realtime);
        assert!(c.automate(params.lfo(2).timed.as_ptr(), 1.0));
        assert!(c.automate(params.lfo(0).shape_mode.as_ptr(), 1.0));
        e.draw();
        drag(&mut e, places(1).jack, lfo_centre(2, 1));
        assert_eq!(depth(Source::Lfo(1), T::LfoRate(2)), None, "timed");
        drag(&mut e, places(1).jack, lfo_centre(0, 0));
        assert_eq!(
            depth(Source::Lfo(1), T::LfoShape(0)),
            Some(0.25),
            "CONTINUOUS"
        );
        // A macro's.
        macro_key(&mut e);
        drag(
            &mut e,
            ca72_panel::strip::macros::jack_at(0),
            lfo_centre(3, 1),
        );
        assert_eq!(
            depth(Source::Macro(0), T::LfoRate(3)),
            Some(0.25),
            "a macro's"
        );
        // Rings on the page's knobs, the jack's round it.
        e.draw();
        let rate = ca72_panel::controls::lfo_index(0, 1);
        let level = ca72_panel::controls::lfo_index(0, LFO_LEVEL_PART);
        let rings = |e: &Editing, i: usize| {
            e.mod_scene
                .knobs
                .iter()
                .find(|k| k.control == i)
                .map(|k| (k.rings.len(), k.skirt, k.sweep))
        };
        assert_eq!(
            rings(&e, rate).map(|r| (r.0, r.2)),
            Some((1, art::KNOB_SWEEP))
        );
        let (n, skirt, _) = rings(&e, level).expect("LEVEL's rings");
        assert_eq!(n, 2);
        assert!(skirt < art::STD.skirt, "round the jack: {skirt}");
        // The pop-up and the tip on hover.
        let p = at(&e, lfo_centre(0, 1));
        go(&mut e, p);
        e.draw();
        assert_eq!(e.modulating.popup, Some(rate));
        let tip = e.tip().map(|t| t.0).unwrap_or_default();
        assert!(
            tip.starts_with("LFO 1 RATE") && tip.contains("LFO 2 +25 %"),
            "{tip}"
        );
        // (Off RATE's pop-up, which hangs below it, first.)
        let (sx, sy, sw, sh) = places(0).scope;
        let p = at(&e, (sx + sw / 2.0, sy + sh / 2.0));
        go(&mut e, p);
        e.draw();
        let p = at(&e, places(0).jack);
        go(&mut e, p);
        e.draw();
        let tip = e.tip().map(|t| t.0).unwrap_or_default();
        assert!(
            tip.starts_with("LFO 1 LEVEL: 100")
                && tip.contains("LFO 2 +25 %")
                && tip.contains("OUT"),
            "{tip}"
        );
        // The menus.
        let rows = e.knob_rows(rate);
        assert!(rows.iter().any(|r| matches!(
            r,
            Row::Item(ModItem::Depth(Source::Lfo(1), T::LfoRate(0), _), ..)
        )));
        assert!(rows.iter().any(|r| matches!(
            r,
            Row::Item(ModItem::Remove(Source::Lfo(1), T::LfoRate(0)), ..)
        )));
        let rows = e.knob_rows(level);
        assert!(rows.iter().any(|r| matches!(
            r,
            Row::Item(ModItem::Remove(Source::Lfo(0), T::LfoLevel(0)), ..)
        )));
        assert!(
            rows.iter()
                .any(|r| matches!(r, Row::Item(ModItem::Color(0, _), ..)))
        );
        assert!(
            rows.iter()
                .any(|r| matches!(r, Row::Item(ModItem::Edges(0, _), ..)))
        );
        // The wheel on the jack turns LEVEL.
        host.take();
        wheel(&mut e, -1.0);
        let calls = host.take();
        let lvl = params.lfo_level(0).level.as_ptr();
        assert!(
            calls
                .iter()
                .any(|c| matches!(c, Call::Set(p, _) if *p == lvl)),
            "{calls:?}"
        );
    }

    /// The left hand's VELOCITY meter is dark until the velocity moves something, then lit to
    /// the latest note (decisions.md R-VELOCITY).
    #[test]
    fn the_velocity_meter_is_dark_until_assigned() {
        use crate::lfo::{Source, Target as T};
        let (mut e, _, params) = editing();
        e.meters.publish_velocity(Some(0.76));
        e.draw();
        assert_eq!(e.scene.velocity_lit, 0, "nothing assigned");
        params
            .lfo_patch
            .assign(Source::Velocity, T::Cutoff, 0.25)
            .expect("room");
        e.draw();
        assert_eq!(e.scene.velocity_lit, 8);
        params.lfo_patch.remove(Source::Velocity, T::Cutoff);
        e.draw();
        assert_eq!(e.scene.velocity_lit, 0, "its last assignment taken away");
    }

    /// The left hand's VELOCITY jack (decisions.md R-VELOCITY): a cord dragged from it lands at
    /// +25 %, as an LFO's; the pointer on it shows a cord to each knob it moves, in its colour
    /// (as on a macro's jack); with the macros shown it is neither found nor hovered.
    #[test]
    fn the_velocity_jack_is_patched_from_and_shows_its_cords() {
        use crate::lfo::{Source, Target as T};
        let (mut e, _, params) = editing();
        e.playing.visible = false;
        let jack = at(&e, ca72_panel::strip::velocity::jack_at());
        go(&mut e, jack);
        e.mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            modifiers: Modifiers::empty(),
        });
        let cutoff = at(&e, centre("cutoff"));
        go(&mut e, cutoff);
        e.draw();
        let cord = e.mod_scene.cord.clone().expect("a cord");
        assert_eq!(cord.from, ca72_panel::strip::velocity::jack_at());
        assert_eq!(cord.colour, ca72_panel::strip::velocity::RGB);
        release(&mut e);
        assert_eq!(
            params.lfo_patch.depth(Source::Velocity, T::Cutoff),
            Some(0.25)
        );
        params
            .lfo_patch
            .assign(Source::Velocity, T::Volume, 0.5)
            .expect("room");
        // Away from the jack: no cords.
        e.draw();
        assert!(e.mod_scene.patched.is_empty());
        // On it: one to each knob it moves, from it, each ringing its knob.
        go(&mut e, jack);
        e.draw();
        let mut to: Vec<_> = e.mod_scene.patched.iter().map(|c| c.to).collect();
        to.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
        let mut want = vec![centre("cutoff"), centre("volume")];
        want.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
        assert_eq!(to, want);
        for c in &e.mod_scene.patched {
            assert_eq!(c.from, ca72_panel::strip::velocity::jack_at());
            assert_eq!(c.colour, ca72_panel::strip::velocity::RGB);
            assert!(c.over.is_some() && c.targets.is_empty());
        }
        // The macros over the left hand: the jack is not there.
        macro_key(&mut e);
        go(&mut e, cutoff);
        go(&mut e, jack);
        e.draw();
        assert!(e.mod_scene.patched.is_empty());
        // A macro's jack shows its cords alike.
        params
            .lfo_patch
            .assign(Source::Macro(0), T::Emphasis, 0.3)
            .expect("room");
        let macro_jack = at(&e, ca72_panel::strip::macros::jack_at(0));
        go(&mut e, macro_jack);
        e.draw();
        let to: Vec<_> = e.mod_scene.patched.iter().map(|c| c.to).collect();
        assert_eq!(to, [centre("emphasis")]);
        // SETTINGS: HOVER CABLES off, none; on again, back.
        e.keep_settings(crate::settings::Change::HoverCables(false));
        e.draw();
        assert!(e.mod_scene.patched.is_empty(), "turned off");
        e.keep_settings(crate::settings::Change::HoverCables(true));
        e.draw();
        assert_eq!(e.mod_scene.patched.len(), 1);
    }

    /// An assignment dragged from a macro's jack lands on a knob at +25 %, as an LFO's; while
    /// the macros show, every macro's rings show on the panel's knobs, one way from the knob's
    /// setting, an LFO's only on the knob under the pointer; a macro's knob takes none.
    #[test]
    fn a_macro_is_assigned_from_its_jack_and_its_rings_show() {
        use crate::lfo::{Source, Target as T};
        let (mut e, host, params) = editing();
        e.playing.visible = false;
        params
            .lfo_patch
            .assign(Source::Lfo(0), T::Cutoff, 0.4)
            .expect("room");
        macro_key(&mut e);
        host.take();
        let jack = at(&e, ca72_panel::strip::macros::jack_at(1));
        go(&mut e, jack);
        e.mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            modifiers: Modifiers::empty(),
        });
        let cutoff = at(&e, centre("cutoff"));
        go(&mut e, cutoff);
        e.draw();
        let cord = e.mod_scene.cord.clone().expect("a cord");
        assert_eq!(cord.from, ca72_panel::strip::macros::jack_at(1));
        assert_eq!(cord.colour, crate::lfo::MacroColor::Rose.rgb());
        release(&mut e);
        assert_eq!(
            params.lfo_patch.depth(Source::Macro(1), T::Cutoff),
            Some(0.25)
        );
        assert!(host.take().is_empty(), "patch state: no host gesture");
        // Away from CUTOFF: its macro's ring alone, one way, its point where the macro has it.
        turn_macro(&params, 1, 0.5);
        go(&mut e, Point { x: 5.0, y: 5.0 });
        e.draw();
        let k = e
            .mod_scene
            .knobs
            .iter()
            .find(|k| CONTROLS[k.control].param == "cutoff")
            .expect("its rings while the macros show");
        assert_eq!(k.rings.len(), 1);
        assert!(k.rings[0].one_way);
        assert_eq!(k.rings[0].point, Some((k.travel + 0.125).clamp(0.0, 1.0)));
        // Under the pointer: every source's, the LFO's too, and the pop-up names the macro.
        go(&mut e, cutoff);
        e.draw();
        let k = e
            .mod_scene
            .knobs
            .iter()
            .find(|k| CONTROLS[k.control].param == "cutoff");
        assert_eq!(k.map(|k| k.rings.len()), Some(2));
        let popup = e.mod_scene.popup.clone().expect("its pop-up");
        assert_eq!(popup.dials[1].label, "MACRO 2");
        // A macro's knob takes no source.
        let m3 = at(&e, CONTROLS[ca72_panel::controls::MACRO_FIRST + 2].centre());
        go(&mut e, jack);
        e.mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            modifiers: Modifiers::empty(),
        });
        go(&mut e, m3);
        release(&mut e);
        assert_eq!(params.lfo_patch.count(Source::Macro(1)), 1);
        // The wheels again: no rings while nothing else shows them.
        macro_key(&mut e);
        go(&mut e, Point { x: 5.0, y: 5.0 });
        e.draw();
        assert!(e.mod_scene.knobs.is_empty());
    }

    /// The standalone's TEMPO a target (decisions.md R-LFO): a macro's cord dropped on its
    /// screen (a dashed box round it while dragged, a bright one under the plug), its rails under
    /// the screen with the pop-up beneath them while it is hovered, its tip beside it, its menu's
    /// DEPTH… and REMOVE and the sources that can take it, a rail's end dragged; the screen reading
    /// the moving tempo, and its own while it is dragged. In a plug-in none of it.
    #[test]
    fn the_tempo_screen_takes_a_source_and_reads_the_moving_tempo() {
        use crate::lfo::{Source, Target as T};
        use lfo_page::TEMPO_CONTROL;
        let (mut e, host, params) = editing_standalone();
        e.playing.visible = false;
        macro_key(&mut e);
        host.take();
        let (x, y, w, h) = art::tempo_window(e.renderer.arrangement().plate_at());
        let screen = at(&e, (x + w / 2.0, y + h / 2.0));
        let jack = at(&e, ca72_panel::strip::macros::jack_at(0));
        go(&mut e, jack);
        press(&mut e);
        let cutoff = at(&e, centre("cutoff"));
        go(&mut e, cutoff);
        e.draw();
        let cord = e.mod_scene.cord.clone().expect("a cord");
        assert_eq!(cord.boxes, vec![(x, y, w, h)], "TEMPO's screen can take it");
        go(&mut e, screen);
        e.draw();
        let cord = e.mod_scene.cord.clone().expect("a cord");
        assert_eq!((cord.over_box, cord.boxes.len()), (Some((x, y, w, h)), 0));
        release(&mut e);
        assert_eq!(
            params.lfo_patch.depth(Source::Macro(0), T::Tempo),
            Some(0.25)
        );
        assert!(host.take().is_empty(), "patch state: no host gesture");
        // Hovered: its rails, the pop-up under them, its tip beside the screen.
        go(&mut e, Point { x: 5.0, y: 5.0 });
        go(&mut e, screen);
        e.draw();
        let k = e
            .mod_scene
            .knobs
            .iter()
            .find(|k| k.control == TEMPO_CONTROL)
            .cloned()
            .expect("its rails");
        assert_eq!(k.bar, Some(w / 2.0));
        assert!((k.travel - 1.0 / 3.0).abs() < 1e-9, "120 BPM along 30..300");
        let popup = e.mod_scene.popup.clone().expect("its pop-up");
        assert!(popup.knob.0.1 >= y + h, "under the screen");
        assert_eq!(popup.dials[0].label, "MACRO 1");
        let (tip, tx, _) = e.scene.tip.clone().expect("a tip");
        assert_eq!(tip, "TEMPO: 120 BPM \u{b7} MACRO 1 +25 %");
        assert!(tx > x + w, "beside it");
        // A rail's end dragged further: a deeper depth.
        let (ex, ey) = ca72_panel::modulation::ring_end(&k, 0);
        let end = at(&e, (ex, ey));
        go(&mut e, end);
        press(&mut e);
        let further = at(&e, (ex + w / 4.0, ey));
        go(&mut e, further);
        release(&mut e);
        let d = params.lfo_patch.depth(Source::Macro(0), T::Tempo).unwrap();
        assert!((d - 0.5).abs() < 0.02, "{d}");
        // Its menu: what moves it, and the sources that can.
        right_click(&mut e, screen);
        let items: Vec<String> = menu_items(&e).into_iter().map(|(t, _)| t).collect();
        for want in ["DEPTH\u{2026}", "REMOVE", "LFO 1", "MACRO 2"] {
            assert!(items.iter().any(|t| t == want), "{want} in {items:?}");
        }
        menu_item(&mut e, "REMOVE");
        assert_eq!(params.lfo_patch.depth(Source::Macro(0), T::Tempo), None);
        // The screen reads the moving tempo while a source moves it, its own while dragged.
        params
            .lfo_patch
            .assign(Source::Lfo(0), T::Tempo, 0.3)
            .unwrap();
        e.meters.publish_tempo(151.4);
        e.live_due = None;
        e.update_scene();
        assert_eq!(e.scene.tempo.as_deref(), Some("151 BPM"));
        go(&mut e, screen);
        e.last_tempo_press = None;
        press(&mut e);
        e.update_scene();
        assert_eq!(e.scene.tempo.as_deref(), Some("120 BPM"), "dragged: as set");
        release(&mut e);
        e.update_scene();
        assert_eq!(e.scene.tempo.as_deref(), Some("151 BPM"));
        params.lfo_patch.remove(Source::Lfo(0), T::Tempo);
        e.update_scene();
        assert_eq!(
            e.scene.tempo.as_deref(),
            Some("120 BPM"),
            "none on it: as set"
        );
        // In a plug-in: no target, the LFO's screen dim, its menu and tip saying why.
        let (p, _, params) = editing();
        params
            .lfo_patch
            .assign(Source::Lfo(1), T::Tempo, 0.4)
            .unwrap();
        assert_eq!(p.mod_target(TEMPO_CONTROL), None);
        assert_eq!(
            lfo_page::screen_text(&params.lfo_patch, 1, true, false),
            ("TEMPO".to_owned(), false)
        );
        assert_eq!(p.standalone_only(T::Tempo), " (STANDALONE ONLY)");
    }

    /// The strip's screens follow a source moving their controls (decisions.md R-LFO: the owner,
    /// "that screen should represent where the modulator currently is based on modulation
    /// position. Not the position of the static knob"): DRIVE at 0 with an LFO on it reads and
    /// is lit where the engine has it (AUTO GAIN's too), its own value while it is turned, and
    /// its own again once the LFO is taken off; the screens of controls nothing moves as set.
    #[test]
    fn a_modulated_screen_reads_where_its_source_has_the_control() {
        use crate::lfo::{Source, Target as T};
        let (mut e, _, params) = editing();
        let drive = Readout::ALL
            .iter()
            .position(|&r| r == Readout::Drive)
            .unwrap();
        let width = Readout::ALL
            .iter()
            .position(|&r| r == Readout::Width)
            .unwrap();
        e.update_scene();
        let still = e.strip_scene.readouts.clone();
        assert!(!still[drive].1, "DRIVE at 0: dark");
        params
            .lfo_patch
            .assign(Source::Lfo(0), T::Drive, 0.5)
            .unwrap();
        let mut c = params.controls();
        c.drive = 1.3;
        c.spread = 0.9;
        e.meters.publish_live(&c);
        e.update_scene();
        assert_eq!(e.strip_scene.readouts[drive], ("1.3".to_owned(), true));
        assert_eq!(
            e.strip_scene.readouts[width], still[width],
            "nothing on WIDTH"
        );
        // Read again only once a thirtieth has passed.
        c.drive = 2.0;
        e.meters.publish_live(&c);
        e.update_scene();
        assert_eq!(e.strip_scene.readouts[drive].0, "1.3");
        e.live_due = None;
        e.update_scene();
        assert_eq!(e.strip_scene.readouts[drive].0, "2.0");
        // Turned: its own value.
        let i = ca72_panel::controls::index("drive").unwrap();
        e.scroll = None;
        assert!(e.live_shown()[3].is_some());
        e.knob_entry = None;
        let k = at(&e, centre("drive"));
        go(&mut e, k);
        press(&mut e);
        assert!(e.operated_now(i));
        e.update_scene();
        assert_eq!(
            e.strip_scene.readouts[drive], still[drive],
            "its own while turned"
        );
        release(&mut e);
        e.live_due = None;
        e.update_scene();
        assert_eq!(e.strip_scene.readouts[drive].0, "2.0");
        // Taken off: as set again.
        params.lfo_patch.remove(Source::Lfo(0), T::Drive);
        e.update_scene();
        assert_eq!(e.strip_scene.readouts[drive], still[drive]);
    }

    /// A macro's name is typed on its screen, double-clicked (or RENAME… from its menu):
    /// capitals, seven at most, Enter keeping it, Escape leaving it as it was, a press elsewhere
    /// keeping it; it then names the macro in its screen, the pop-up, the tips and the menus.
    /// Its name's menu sets its colour, the computer's.
    #[test]
    fn a_macros_name_is_typed_and_its_colour_chosen() {
        use crate::lfo::{MacroColor, Source, Target as T};
        let (mut e, host, params) = editing();
        macro_key(&mut e);
        host.take();
        let (x, y, w, h) = ca72_panel::strip::macros::name_at(0);
        let name = at(&e, (x + w / 2.0, y + h / 2.0));
        click(&mut e, name);
        click(&mut e, name);
        assert_eq!(e.macro_view.typing, Some((0, String::new())));
        for c in ["b", "r", "i", "g", "h", "t", "e", "r"] {
            keyed(&mut e, Key::Character(c.into()));
        }
        e.draw();
        assert_eq!(e.macro_scene.cells[0].typing.as_deref(), Some("BRIGHTE"));
        keyed(&mut e, Key::Backspace);
        keyed(&mut e, Key::Enter);
        assert_eq!(params.lfo_patch.name(0), "BRIGHT");
        assert!(host.take().is_empty(), "patch state");
        e.draw();
        assert_eq!(e.macro_scene.cells[0].name, "BRIGHT");
        // Escape leaves it.
        e.start_rename(0);
        keyed(&mut e, Key::Character("x".into()));
        keyed(&mut e, Key::Escape);
        assert_eq!(params.lfo_patch.name(0), "BRIGHT");
        // A press elsewhere keeps what is typed.
        e.start_rename(0);
        keyed(&mut e, Key::Backspace);
        let away = at(&e, centre("cutoff"));
        click(&mut e, away);
        assert_eq!(params.lfo_patch.name(0), "BRIGH");
        // Its colour from its name's menu.
        right_click(&mut e, name);
        assert!(e.learning.menu_open());
        e.mod_item(crate::learning::ModItem::MacroColor(0, MacroColor::Purple));
        assert_eq!(e.settings.macro_colors[0], MacroColor::Purple);
        e.learning.close_menu();
        e.draw();
        assert_eq!(e.macro_scene.cells[0].colour, MacroColor::Purple.rgb());
        // Its knob's menu: RENAME…, REMOVE ALL ASSIGNMENTS, a head for each knob it modulates.
        params
            .lfo_patch
            .assign(Source::Macro(0), T::Cutoff, 0.6)
            .expect("room");
        let rows = e.knob_rows(ca72_panel::controls::MACRO_FIRST);
        assert!(
            matches!(&rows[0], crate::learning::Row::Item(_, t, true, _) if t == "RENAME\u{2026}")
        );
        assert!(rows.iter().any(|r| matches!(r, crate::learning::Row::Head(h) if h.starts_with("CUTOFF FREQUENCY \u{b7} +60"))));
        e.mod_item(crate::learning::ModItem::RemoveAll(Source::Macro(0)));
        assert_eq!(params.lfo_patch.count(Source::Macro(0)), 0);
        // The knob's tip names the macro.
        let knob = at(&e, CONTROLS[ca72_panel::controls::MACRO_FIRST].centre());
        go(&mut e, knob);
        e.draw();
        let tip = e.scene.tip.clone().expect("a tip").0;
        assert!(tip.starts_with("MACRO 1 \u{b7} BRIGH: 0"), "{tip}");
        // A strip knob's menu offers the macros as the LFOs.
        let i = ca72_panel::controls::index("drive").expect("DRIVE");
        let rows = e.knob_rows(i);
        assert!(
            rows.iter()
                .any(|r| matches!(r, crate::learning::Row::Head(h) if h == "MACRO"))
        );
        assert!(
            rows.iter()
                .any(|r| matches!(r, crate::learning::Row::Item(_, t, _, _) if t == "BRIGH"))
        );
    }

    /// The window's frame with the macros showing, a macro turned and renamed and its rings on
    /// the panel, is the frame put together whole, each time.
    #[test]
    fn the_macros_put_together_in_parts_are_put_together_whole() {
        use crate::lfo::{Source, Target as T};
        let (mut e, _, params) = editing();
        params
            .lfo_patch
            .assign(Source::Macro(0), T::Cutoff, 0.4)
            .expect("room");
        macro_key(&mut e);
        for i in 0..5 {
            turn_macro(&params, 0, i as f32 / 5.0);
            if i == 2 {
                params.lfo_patch.set_name(3, "SPACE");
            }
            if i == 3 {
                e.start_rename(4);
                keyed(&mut e, Key::Character("s".into()));
            }
            e.draw();
            let mut whole = None;
            compose(
                &mut whole,
                &e.renderer,
                e.strip.frame(),
                lower_of(&e),
                None,
                (
                    e.page_shows().then(|| e.lfo.frame()),
                    e.macros_show()
                        .then(|| (e.macros.frame(), e.renderer.left_rect())),
                    &e.modulation,
                ),
                None,
                true,
                (e.physical_width, e.physical_height()),
            );
            assert!(e.composed == whole, "frame {i}: not as put together whole");
        }
    }

    /// How long the editor takes to draw the macros' frames (decisions.md R-MACROS; the owner:
    /// "Just make sure performance is tight"), at the opening size (0.44) and at twice it: the
    /// first showing, a macro turned (its knob and its rings on the panel), a name typed, the
    /// wheels again, and nothing changed. Printed, not asserted (the machine's load decides).
    #[test]
    #[ignore = "measures frame times"]
    fn the_macros_frame_times() {
        use crate::lfo::{Source, Target as T};
        use std::time::Instant;
        for k in [1.0, 2.0] {
            let (mut e, _, params) = editing();
            let width = (art::W * 0.44 * k) as u32;
            e.fit(k, (art::W * 0.44) as u32, width);
            for (n, to) in [
                (0, T::Cutoff),
                (0, T::Emphasis),
                (1, T::Drive),
                (2, T::Spread),
            ] {
                params.lfo_patch.assign(Source::Macro(n), to, 0.5).unwrap();
            }
            e.playing.visible = false;
            e.draw();
            e.draw();
            let time = |e: &mut Editing, f: &mut dyn FnMut(&mut Editing)| {
                f(e);
                let t = Instant::now();
                e.draw();
                t.elapsed().as_secs_f64() * 1e3
            };
            let first = time(&mut e, &mut |e| {
                let p = on_strip(e, strip_at(StripTarget::Bar(BarTarget::Macro)));
                click(e, p);
            });
            // (Again in another editor, its still made on its thread meanwhile, as when the key
            // is pressed some time after the editor opened.)
            let ready = {
                let (mut f, _, _) = editing();
                f.fit(k, (art::W * 0.44) as u32, width);
                f.playing.visible = false;
                f.draw();
                std::thread::sleep(std::time::Duration::from_millis(600));
                f.draw();
                time(&mut f, &mut |f| {
                    let p = on_strip(f, strip_at(StripTarget::Bar(BarTarget::Macro)));
                    click(f, p);
                })
            };
            let mut turned = Vec::new();
            for i in 1..8 {
                turned.push(time(&mut e, &mut |_| {
                    turn_macro(&params, 0, i as f32 / 8.0);
                }));
            }
            let typed = time(&mut e, &mut |e| {
                e.start_rename(3);
                keyed(e, Key::Character("w".into()));
            });
            let still = time(&mut e, &mut |_| {});
            let back = time(&mut e, &mut |e| {
                e.end_rename(true);
                let p = on_strip(e, strip_at(StripTarget::Bar(BarTarget::Macro)));
                click(e, p);
            });
            let again = time(&mut e, &mut |e| {
                let p = on_strip(e, strip_at(StripTarget::Bar(BarTarget::Macro)));
                click(e, p);
            });
            turned.sort_by(f64::total_cmp);
            println!(
                "scale {:.2}: first showing {first:.1} ms (still waited for), {ready:.1} ms (still ready); a macro turned {:.1}-{:.1} ms (median {:.1}); a name typed {typed:.1} ms; nothing changed {still:.2} ms; the wheels again {back:.1} ms; the macros again {again:.1} ms",
                0.44 * k,
                turned[0],
                turned[turned.len() - 1],
                turned[turned.len() / 2]
            );
        }
    }

    /// The macros as the editor shows them, for looking at, written to `$CA72_MACRO_PNG`, the
    /// folder: `wheels.png` (the wheels, the MACRO key glowing), `macros.png` (the six, round
    /// 2's example, their rings on the panel), `keyboard.png` and `lfo.png` (the six beside the
    /// keyboard and the LFO page), `drag.png` (an assignment dragged from GROWL's jack),
    /// `hover.png` (CUTOFF's rings and pop-up with LFOs and a macro), `menu.png` (BRIGHT's knob's
    /// menu), `name-menu.png` (its name's), `rename.png` (a name typed), `drawn.png` (the drawn
    /// skin).
    #[test]
    #[ignore = "writes images for a look"]
    fn the_macro_pngs() {
        use crate::lfo::{Source, Target as T};
        let Some(out) = std::env::var_os("CA72_MACRO_PNG").map(std::path::PathBuf::from) else {
            return;
        };
        std::fs::create_dir_all(&out).unwrap();
        let (mut e, _, params) = editing();
        let patch = Arc::clone(&params.lfo_patch);
        // Round 2's example.
        for (n, name, value, to) in [
            (
                0,
                "BRIGHT",
                0.55,
                vec![(T::Cutoff, 0.60), (T::Emphasis, -0.20)],
            ),
            (
                1,
                "GROWL",
                0.30,
                vec![(T::ContourAmount, 0.40), (T::Drive, 0.50)],
            ),
            (2, "SPACE", 0.0, vec![(T::Spread, 0.70), (T::Inner, 0.40)]),
            (3, "WOBBLE", 0.20, vec![(T::Osc2Frequency, 0.08)]),
            (4, "SWELL", 0.70, vec![(T::LoudnessAttack, 0.50)]),
        ] {
            patch.set_name(n, name);
            turn_macro(&params, n, value);
            for (t, d) in to {
                patch.assign(Source::Macro(n), t, d).unwrap();
            }
        }
        patch.assign(Source::Lfo(0), T::Cutoff, 0.4).unwrap();
        patch.assign(Source::Lfo(1), T::Cutoff, -0.25).unwrap();
        let save = |e: &mut Editing, name: &str| {
            window_frame(e).save_png(out.join(name)).unwrap();
        };
        e.playing.visible = false;
        e.draw();
        save(&mut e, "wheels.png");
        macro_key(&mut e);
        save(&mut e, "macros.png");
        let k = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Keyboard)));
        click(&mut e, k);
        save(&mut e, "keyboard.png");
        lfo_open(&mut e);
        save(&mut e, "lfo.png");
        let g = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Global)));
        click(&mut e, g);
        e.draw();
        let jack = at(&e, ca72_panel::strip::macros::jack_at(1));
        go(&mut e, jack);
        e.mouse(MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            modifiers: Modifiers::empty(),
        });
        let to = at(&e, centre("mod_mix"));
        go(&mut e, to);
        save(&mut e, "drag.png");
        e.end_gestures();
        let cutoff = at(&e, centre("cutoff"));
        go(&mut e, cutoff);
        save(&mut e, "hover.png");
        let knob = at(&e, CONTROLS[ca72_panel::controls::MACRO_FIRST].centre());
        right_click(&mut e, knob);
        save(&mut e, "menu.png");
        e.learning.close_menu();
        let (x, y, w, h) = ca72_panel::strip::macros::name_at(0);
        let name = at(&e, (x + w / 2.0, y + h / 2.0));
        right_click(&mut e, name);
        save(&mut e, "name-menu.png");
        e.learning.close_menu();
        e.start_rename(5);
        for c in ["s", "u", "b"] {
            keyed(&mut e, Key::Character(c.into()));
        }
        go(&mut e, Point { x: 5.0, y: 5.0 });
        save(&mut e, "rename.png");
        e.end_rename(false);
        e.set_look(Skin::Drawn);
        save(&mut e, "drawn.png");
    }

    /// The window's frame with the LFO page, its orbs moving, a knob on it turned and rings on
    /// the panel is the frame put together whole, each time.
    #[test]
    fn the_lfo_page_put_together_in_parts_is_put_together_whole() {
        use crate::lfo::{Reading, Source, Target as T};
        let (mut e, _, params) = editing();
        params
            .lfo_patch
            .assign(Source::Lfo(0), T::Cutoff, 0.4)
            .expect("room");
        lfo_open(&mut e);
        e.draw();
        for i in 0..6 {
            let r = Reading {
                value: (i as f32 * 0.3).sin(),
                phase: (i as f32 * 0.1) % 1.0,
                since: i as f32 * 0.05,
                fade: 1.0,
                moving: true,
                ..Reading::default()
            };
            params.lfo_scope.publish(&[r; 4]);
            e.modulating.due = None;
            if i == 3 {
                let p = at(&e, lfo_centre(2, 4));
                go(&mut e, p);
            }
            e.draw();
            let mut whole = None;
            compose(
                &mut whole,
                &e.renderer,
                e.strip.frame(),
                lower_of(&e),
                None,
                (
                    e.page_shows().then(|| e.lfo.frame()),
                    e.macros_show()
                        .then(|| (e.macros.frame(), e.renderer.left_rect())),
                    &e.modulation,
                ),
                None,
                true,
                (e.physical_width, e.physical_height()),
            );
            assert!(e.composed == whole, "frame {i}: not as put together whole");
        }
    }
}

/// The editor in a real window on Windows, in a host that resizes windows (decisions.md R26).
/// Opening the presets' drawer, shutting it and dragging the grip ask the host for another size
/// from within the editor's event handler. The host may resize the editor's window right there,
/// from within `IPlugFrame::resizeView()` (here `resizes_editor`), or the editor's own resize may
/// run while it still handles the press (the drawer taking the keyboard sent its window messages
/// from within the handler). Either way baseview gave the busy handler a `WM_SIZE`: a panic inside
/// its window procedure, which aborted the host's process (0.1.0, in Sonar). These tests aborted
/// the test process then. And Windows repainting the editor's window, in a host whose window
/// paints over it (decisions.md R28). The windows are the test's own, off the screen (or, where
/// the editor's pixels are read, on it but invisible), and the pointer is never over them; the
/// presets come from an empty folder of the test's.
#[cfg(all(test, target_os = "windows"))]
#[allow(unsafe_code)]
mod windows_window_tests {
    use std::path::PathBuf;
    use std::ptr::null_mut;
    use std::sync::MutexGuard;

    use ca72_panel::presets::BarTarget;
    use nih_plug::context::PluginApi;
    use nih_plug::editor::ParentWindowHandle;
    use nih_plug::wrapper::state::PluginState;
    use winapi::shared::minwindef::{LPARAM, WPARAM};
    use winapi::shared::windef::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, HBRUSH, HWND, RECT};
    use winapi::um::wingdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC,
        DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDIBits, GetPixel, SRCCOPY, SelectObject,
    };
    use winapi::um::winuser::{
        COLOR_BTNFACE, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GW_CHILD,
        GetClientRect, GetDC, GetWindow, HWND_BOTTOM, LWA_ALPHA, MK_LBUTTON, MSG, PM_REMOVE,
        PW_RENDERFULLCONTENT, PeekMessageW, PostMessageW, PrintWindow, RDW_ALLCHILDREN, RDW_ERASE,
        RDW_INVALIDATE, RDW_NOCHILDREN, RDW_UPDATENOW, RedrawWindow, RegisterClassW, ReleaseDC,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetLayeredWindowAttributes,
        SetThreadDpiAwarenessContext, SetWindowPos, TranslateMessage, WM_LBUTTONDOWN, WM_LBUTTONUP,
        WM_MOUSEMOVE, WNDCLASSW, WS_CLIPCHILDREN, WS_EX_LAYERED, WS_EX_NOACTIVATE,
        WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP, WS_VISIBLE,
    };
    use winapi::um::winuser::{
        DLGC_WANTALLKEYS, MK_RBUTTON, SendMessageW, WM_GETDLGCODE, WM_RBUTTONDOWN, WM_RBUTTONUP,
    };

    use super::*;
    use crate::opening::DOZE_AFTER;

    /// The host's window. Off the screen, where nothing drawn in it shows, its painting kept
    /// off the editor's window (`WS_CLIPCHILDREN`). Or as a host's dialog (REAPER's FX window):
    /// painting its background over the editor's window whenever it is repainted, and on the
    /// screen, so that the editor's pixels can be read; but at the bottom of the windows, an
    /// alpha of 1 in 255, and letting the pointer through to whatever is under it. The host
    /// repaints it as it resizes it, from within the editor's request.
    #[derive(Clone, Copy, PartialEq)]
    enum HostWindow {
        OffScreen,
        Dialog,
    }

    /// A host that grows its own window (the editor's parent) to the size the editor asks, and
    /// with `resizes_editor` the editor's window as well, from within the request; with
    /// `repaints`, it then repaints both windows there and then.
    struct WindowHost {
        editor: Arc<Ca72Editor>,
        parent: usize,
        resizes_editor: bool,
        repaints: bool,
        resizes: AtomicU32,
        /// The gestures the editor began.
        gestures: AtomicU32,
        deferred: AtomicBool,
        refused: AtomicBool,
        pending: Mutex<Vec<(u64, (u32, u32))>>,
    }

    impl GuiContext for WindowHost {
        fn plugin_api(&self) -> PluginApi {
            PluginApi::Vst3
        }
        fn request_resize(&self) -> bool {
            self.resizes.fetch_add(1, Ordering::Relaxed);
            let (request, size) = self.editor.resize_request();
            if self.deferred.load(Ordering::Relaxed) {
                self.pending
                    .lock()
                    .expect("resize requests")
                    .push((request, size));
                return true;
            }
            self.answer(request, size, !self.refused.load(Ordering::Relaxed))
        }
        fn resize_response_will_follow(&self) -> bool {
            true
        }
        unsafe fn raw_begin_set_parameter(&self, _param: ParamPtr) {
            self.gestures.fetch_add(1, Ordering::Relaxed);
        }
        unsafe fn raw_set_parameter_normalized(&self, _param: ParamPtr, _normalized: f32) {}
        unsafe fn raw_end_set_parameter(&self, _param: ParamPtr) {}
        fn get_state(&self) -> PluginState {
            panic!("the editor does not read the state")
        }
        fn set_state(&self, _state: PluginState) {}
    }

    impl WindowHost {
        fn answer(&self, request: u64, (w, h): (u32, u32), accepted: bool) -> bool {
            if !accepted {
                self.editor.resize_result(request, false);
                return false;
            }
            let (w, h) = (w as i32, h as i32);
            let parent = self.parent as HWND;
            let flags = SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE;
            // SAFETY: the test's own windows, on the thread that made them (the editor asks from
            // its event handler).
            unsafe {
                SetWindowPos(parent, null_mut(), 0, 0, w, h, flags);
                if self.resizes_editor {
                    SetWindowPos(GetWindow(parent, GW_CHILD), null_mut(), 0, 0, w, h, flags);
                }
                if self.repaints {
                    let flags = RDW_INVALIDATE | RDW_ERASE | RDW_UPDATENOW | RDW_ALLCHILDREN;
                    RedrawWindow(parent, null_mut(), null_mut(), flags);
                }
            }
            self.editor.resize_result(request, true);
            true
        }
    }

    // Native capture and focus are shared by the desktop, including across test threads.
    static DESKTOP: Mutex<()> = Mutex::new(());

    /// The editor's window open in the host's, at the narrowest width and a pixel a point.
    struct Opened {
        parent: HWND,
        editor: Arc<Ca72Editor>,
        host: Arc<WindowHost>,
        /// What the audio reports, as the plug-in's processing publishes it.
        meters: Arc<Meters>,
        handle: baseview::WindowHandle,
        _presets: tempfile::TempDir,
        // Last to drop: cleanup and all owned window state precede the next fixture.
        _desktop: MutexGuard<'static, ()>,
    }

    impl Opened {
        fn new(resizes_editor: bool, host_kind: HostWindow) -> Self {
            let desktop = DESKTOP
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // The windows in pixels whatever the screen's scale (baseview sets the process's
            // awareness only once its first window is open, and the tests share the process).
            // SAFETY: this thread's own setting.
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
            let presets = tempfile::tempdir().expect("a temporary folder");
            let params = Arc::new(Ca72Params::default());
            params.editor_width.store(MIN_WIDTH, Ordering::Relaxed);
            let meters = Arc::new(Meters::default());
            let editor = Arc::new(Ca72Editor::new(Arc::clone(&params), Arc::clone(&meters)));
            let (w, h) = editor.opening();
            editor.open.store(true, Ordering::Release);
            let parent = host_window(w, h, host_kind);
            let host = Arc::new(WindowHost {
                editor: Arc::clone(&editor),
                parent: parent as usize,
                resizes_editor,
                repaints: host_kind == HostWindow::Dialog,
                resizes: AtomicU32::new(0),
                gestures: AtomicU32::new(0),
                deferred: AtomicBool::new(false),
                refused: AtomicBool::new(false),
                pending: Mutex::new(Vec::new()),
            });
            let context: Arc<dyn GuiContext> = host.clone();
            let window_meters = Arc::clone(&meters);
            let dir: PathBuf = presets.path().to_path_buf();
            let handle = Window::open_parented(
                &window::Parent(ParentWindowHandle::Win32Hwnd(parent.cast())),
                WindowOpenOptions {
                    title: String::from("CA-72"),
                    size: Size::new(f64::from(w), f64::from(h)),
                    scale: WindowScalePolicy::ScaleFactor(1.0),
                },
                move |window: &mut Window<'_>| {
                    PanelWindow::new(
                        window,
                        context,
                        params,
                        window_meters,
                        w,
                        1.0,
                        Library::at(dir),
                        Arc::new(HostKeys::default()),
                    )
                },
            );
            pump(Duration::from_millis(300));
            Opened {
                parent,
                editor,
                host,
                meters,
                handle,
                _presets: presets,
                _desktop: desktop,
            }
        }

        /// The editor's window.
        fn window(&self) -> HWND {
            // SAFETY: the test's own window.
            unsafe { GetWindow(self.parent, GW_CHILD) }
        }

        /// Its size, in pixels (logical ones: its scale is 1).
        fn size(&self) -> (u32, u32) {
            let mut r = RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            // SAFETY: the test's own window and a rectangle of this function's.
            unsafe { GetClientRect(self.window(), &mut r) };
            ((r.right - r.left) as u32, (r.bottom - r.top) as u32)
        }

        /// Drawing units to the window's pixels.
        fn at(&self, (x, y): (f64, f64)) -> (f64, f64) {
            let k = f64::from(self.size().0) / art::W;
            (x * k, y * k)
        }

        /// The pointer moved to `at` (pixels), pressed, moved `dx` across in steps, released.
        fn drag(&self, at: (f64, f64), dx: f64) {
            let lp = |(x, y): (f64, f64)| {
                (((y.round() as u32) << 16) | (x.round() as u32 & 0xFFFF)) as LPARAM
            };
            let w = self.window();
            // SAFETY: messages posted to the test's own window.
            unsafe {
                PostMessageW(w, WM_MOUSEMOVE, 0, lp(at));
                PostMessageW(w, WM_LBUTTONDOWN, MK_LBUTTON as WPARAM, lp(at));
            }
            pump(Duration::from_millis(50));
            for i in 1..=4 {
                let to = (at.0 + dx * f64::from(i) / 4.0, at.1);
                // SAFETY: as above.
                unsafe { PostMessageW(w, WM_MOUSEMOVE, MK_LBUTTON as WPARAM, lp(to)) };
                pump(Duration::from_millis(50));
            }
            // SAFETY: as above.
            unsafe { PostMessageW(w, WM_LBUTTONUP, 0, lp((at.0 + dx, at.1))) };
            pump(Duration::from_millis(400));
        }

        fn click_bar(&self, target: BarTarget) {
            let (x, y) = strip::centre(Rail::Plugin, StripTarget::Bar(target));
            self.drag(self.at((x, y)), 0.0);
        }

        /// A click on the preset's name, in the strip under the panel.
        fn click_name(&self) {
            self.click_bar(BarTarget::Name);
        }

        fn close(self) {
            drop(self);
        }
    }

    impl Drop for Opened {
        fn drop(&mut self) {
            // Also clean up after an assertion fails, on the thread that owns the HWNDs.
            self.handle.close();
            pump(Duration::from_millis(100));
            // SAFETY: the test's own parent; its child has processed its close request.
            unsafe { DestroyWindow(self.parent) };
        }
    }

    /// The host's window, `w` by `h`, of `kind`, with no button on the taskbar and not brought
    /// to the front, so that it does not take the developer's pointer or keys.
    fn host_window(w: u32, h: u32, kind: HostWindow) -> HWND {
        let (name, background, clips, see_through, at) = match kind {
            HostWindow::OffScreen => ("CA72TestHost\0", null_mut(), WS_CLIPCHILDREN, 0, -20000),
            HostWindow::Dialog => (
                "CA72TestDialog\0",
                (COLOR_BTNFACE + 1) as usize as HBRUSH,
                0,
                WS_EX_LAYERED | WS_EX_TRANSPARENT,
                0,
            ),
        };
        let class: Vec<u16> = name.encode_utf16().collect();
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(DefWindowProcW),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: null_mut(),
            hIcon: null_mut(),
            hCursor: null_mut(),
            hbrBackground: background,
            lpszMenuName: null_mut(),
            lpszClassName: class.as_ptr(),
        };
        // SAFETY: a window class of the test's (registered by the first test to get here; the
        // others' calls fail harmlessly) and a window of it, on this thread.
        let hwnd = unsafe {
            RegisterClassW(&wc);
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | see_through,
                class.as_ptr(),
                class.as_ptr(),
                WS_POPUP | WS_VISIBLE | clips,
                at,
                at,
                w as i32,
                h as i32,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
            )
        };
        assert!(!hwnd.is_null(), "a window for the test");
        if kind == HostWindow::Dialog {
            let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
            // SAFETY: the test's own window.
            unsafe {
                SetLayeredWindowAttributes(hwnd, 0, 1, LWA_ALPHA);
                SetWindowPos(hwnd, HWND_BOTTOM, 0, 0, 0, 0, flags);
            }
        }
        hwnd
    }

    /// The editor's window's pixels, 64 by 64 across it, as drawn there (`GetPixel` reads
    /// the window's own, whatever covers it on the screen, but nothing off the screen).
    fn pixels(o: &Opened) -> Vec<u32> {
        let (w, h) = o.size();
        let at = |i: u32, n: u32| (n * (2 * i + 1) / 128) as i32;
        let window = o.window();
        // SAFETY: the test's own window, and its device context, released.
        unsafe {
            let dc = GetDC(window);
            let p = (0..64)
                .flat_map(|j| (0..64).map(move |i| (at(i, w), at(j, h))))
                .map(|(x, y)| GetPixel(dc, x, y))
                .collect();
            ReleaseDC(window, dc);
            p
        }
    }

    /// Every pixel of the editor's window, as drawn there (as `pixels`, but all of them).
    fn every_pixel(o: &Opened) -> Vec<u32> {
        let (w, h) = o.size();
        let mut out = vec![0u32; (w * h) as usize];
        let window = o.window();
        // SAFETY: the test's own window and its device context, released; a device context
        // and a bitmap of this function's, deleted; `out` holds the bitmap's pixels.
        unsafe {
            let src = GetDC(window);
            let dc = CreateCompatibleDC(src);
            let bitmap = CreateCompatibleBitmap(src, w as i32, h as i32);
            let old = SelectObject(dc, bitmap.cast());
            BitBlt(dc, 0, 0, w as i32, h as i32, src, 0, 0, SRCCOPY);
            SelectObject(dc, old);
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = w as i32;
            info.bmiHeader.biHeight = -(h as i32);
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB;
            GetDIBits(
                dc,
                bitmap,
                0,
                h,
                out.as_mut_ptr().cast(),
                &mut info,
                DIB_RGB_COLORS,
            );
            DeleteObject(bitmap.cast());
            DeleteDC(dc);
            ReleaseDC(window, src);
        }
        out
    }

    fn colours(pixels: &[u32]) -> usize {
        let mut c = pixels.to_vec();
        c.sort_unstable();
        c.dedup();
        c.len()
    }

    /// `window` captured as a screenshot tool does (`PrintWindow`), into a bitmap thrown away.
    fn print(window: HWND) {
        let mut r = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        // SAFETY: the test's own window; a device context and a bitmap of this function's,
        // deleted, and the screen's, released.
        unsafe {
            GetClientRect(window, &mut r);
            let screen = GetDC(null_mut());
            let dc = CreateCompatibleDC(screen);
            let bitmap = CreateCompatibleBitmap(screen, r.right, r.bottom);
            let old = SelectObject(dc, bitmap.cast());
            PrintWindow(window, dc, PW_RENDERFULLCONTENT);
            SelectObject(dc, old);
            DeleteObject(bitmap.cast());
            DeleteDC(dc);
            ReleaseDC(null_mut(), screen);
        }
    }

    /// This thread's messages, for `d`.
    fn pump(d: Duration) {
        let until = Instant::now() + d;
        // SAFETY: a message structure of this function's, for this thread's messages.
        let mut msg: MSG = unsafe { std::mem::zeroed() };
        while Instant::now() < until {
            // SAFETY: as above.
            while unsafe { PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) } != 0 {
                // SAFETY: a message just taken from this thread's queue.
                unsafe {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// The editor's window's size once it is `want`, or after 5 s, its messages handled meanwhile.
    fn settled(o: &Opened, want: (u32, u32)) -> (u32, u32) {
        let until = Instant::now() + Duration::from_secs(5);
        while o.size() != want && Instant::now() < until {
            pump(Duration::from_millis(20));
        }
        o.size()
    }

    /// The presets' drawer opens and shuts over the strip, the window as it is: the host never
    /// asked to resize it (A6; the drawer had grown the window below the strip), where the host
    /// grows its own window and where it resizes the editor's as well.
    fn the_drawer_opens_and_shuts(resizes_editor: bool) {
        let o = Opened::new(resizes_editor, HostWindow::OffScreen);
        let short = (MIN_WIDTH, height_for(MIN_WIDTH));
        assert_eq!(settled(&o, short), short);
        o.click_name();
        pump(Duration::from_millis(600));
        assert_eq!(o.size(), short, "the window as it was, the drawer open");
        o.click_name();
        pump(Duration::from_millis(600));
        assert_eq!(o.size(), short, "the window as it was, the drawer shut");
        assert_eq!(o.host.resizes.load(Ordering::Relaxed), 0, "the host asked");
        o.close();
    }

    #[test]
    fn the_drawer_opens_and_shuts_where_the_host_grows_its_own_window() {
        the_drawer_opens_and_shuts(false);
    }

    #[test]
    fn the_drawer_opens_and_shuts_where_the_host_resizes_the_editors_window() {
        the_drawer_opens_and_shuts(true);
    }

    /// Repeated height changes travel through actual native windows, including a host that
    /// resizes the child synchronously from inside the request. Presets temporarily reveal
    /// the lower panel, then restore the compact view, whose grip still changes the width.
    fn panel_sizes(resizes_editor: bool) {
        let o = Opened::new(resizes_editor, HostWindow::OffScreen);
        let full = (MIN_WIDTH, height_for_panel(MIN_WIDTH, true));
        let compact = (MIN_WIDTH, height_for_panel(MIN_WIDTH, false));
        for _ in 0..8 {
            o.click_bar(BarTarget::Panel);
            assert_eq!(settled(&o, compact), compact);
            assert_eq!(o.editor.size(), compact);
            o.click_bar(BarTarget::Panel);
            assert_eq!(settled(&o, full), full);
            assert_eq!(o.editor.size(), full);
        }
        assert_eq!(o.host.resizes.load(Ordering::Relaxed), 16);
        o.click_bar(BarTarget::Panel);
        assert_eq!(settled(&o, compact), compact);
        o.click_name();
        assert_eq!(settled(&o, full), full, "presets reveal the hidden panel");
        o.click_name();
        assert_eq!(
            settled(&o, compact),
            compact,
            "presets restore the compact view"
        );
        let grip = o.at((art::W - 12.0, DRAWER_TOP - 12.0));
        o.drag(grip, 120.0);
        let asked = o.editor.size();
        assert!(asked.0 > MIN_WIDTH, "the compact grip changes the width");
        assert_eq!(asked.1, height_for_panel(asked.0, false));
        assert_eq!(settled(&o, asked), asked);
        o.click_bar(BarTarget::Panel);
        let expanded = (asked.0, height_for_panel(asked.0, true));
        assert_eq!(settled(&o, expanded), expanded, "expanding keeps the width");
        o.close();
    }

    #[test]
    fn panel_sizes_where_the_host_grows_its_own_window() {
        panel_sizes(false);
    }

    #[test]
    fn panel_sizes_where_the_host_resizes_the_editors_window() {
        panel_sizes(true);
    }

    #[test]
    fn refused_and_delayed_panel_sizes_restore_the_native_window() {
        let o = Opened::new(true, HostWindow::OffScreen);
        let full = (MIN_WIDTH, height_for_panel(MIN_WIDTH, true));
        let compact = (MIN_WIDTH, height_for_panel(MIN_WIDTH, false));
        o.host.refused.store(true, Ordering::Relaxed);
        o.click_bar(BarTarget::Panel);
        assert_eq!(o.size(), full);
        o.host.refused.store(false, Ordering::Relaxed);
        o.host.deferred.store(true, Ordering::Relaxed);
        for _ in 0..3 {
            o.click_bar(BarTarget::Panel);
        }
        let requests = std::mem::take(&mut *o.host.pending.lock().expect("requests"));
        assert_eq!(requests.len(), 3);
        o.host.answer(requests[0].0, requests[0].1, false);
        pump(Duration::from_millis(200));
        assert_eq!(
            o.size(),
            compact,
            "an obsolete refusal does not change the latest view"
        );
        o.host.answer(requests[1].0, requests[1].1, true);
        o.host.answer(requests[2].0, requests[2].1, false);
        pump(Duration::from_millis(500));
        assert_eq!(o.size(), full, "the last host-confirmed view is restored");
        assert_eq!(o.editor.size(), full);
        o.host.deferred.store(false, Ordering::Relaxed);
        o.click_bar(BarTarget::Panel);
        assert_eq!(settled(&o, compact), compact);
        o.close();
    }

    #[test]
    fn changing_panel_height_keeps_the_native_frame_after_repaint() {
        let o = Opened::new(true, HostWindow::Dialog);
        for expanded in [false, true, false, true] {
            o.click_bar(BarTarget::Panel);
            let asked = (MIN_WIDTH, height_for_panel(MIN_WIDTH, expanded));
            assert_eq!(settled(&o, asked), asked);
            pump(Duration::from_millis(400));
            let before = every_pixel(&o);
            assert!(colours(&before) >= 100, "the panel is drawn");
            let flags = RDW_INVALIDATE | RDW_ERASE | RDW_UPDATENOW | RDW_ALLCHILDREN;
            // SAFETY: the test's own windows on their creating thread.
            unsafe { RedrawWindow(o.parent, null_mut(), null_mut(), flags) };
            pump(Duration::from_millis(200));
            assert_eq!(
                every_pixel(&o),
                before,
                "repainting preserves the complete frame"
            );
        }
        o.close();
    }

    #[test]
    fn delayed_width_refusal_preserves_the_native_controls_after_repaint() {
        let o = Opened::new(true, HostWindow::Dialog);
        let original = o.size();
        o.host.deferred.store(true, Ordering::Relaxed);
        let grip = o.at((art::W - 12.0, art::H - 12.0));
        o.drag(grip, 120.0);
        assert!(o.size().0 > original.0);
        let requests = std::mem::take(&mut *o.host.pending.lock().expect("requests"));
        let &(request, size) = requests.last().expect("width request");
        o.host.answer(request, size, false);
        assert_eq!(settled(&o, original), original);
        pump(Duration::from_millis(300));
        let before = every_pixel(&o);
        assert!(colours(&before) >= 100, "the restored controls are drawn");
        let flags = RDW_INVALIDATE | RDW_ERASE | RDW_UPDATENOW | RDW_ALLCHILDREN;
        // SAFETY: the test's own windows on their creating thread.
        unsafe { RedrawWindow(o.parent, null_mut(), null_mut(), flags) };
        pump(Duration::from_millis(200));
        assert!(
            every_pixel(&o) == before,
            "repainting preserves the restored controls"
        );
        o.close();
    }

    /// A press with no move of the pointer before it (the pointer set where it is, or the
    /// window come under it) lands where it is: POLY's tab, pressed so, is chosen. baseview gave
    /// the press without its place, and the editor took it where the pointer had last been
    /// seen (in Cubase, a first click that did nothing; baseview's PATCHES.md, change 5).
    #[test]
    fn a_press_with_no_move_before_it_lands_where_it_is() {
        let o = Opened::new(false, HostWindow::OffScreen);
        let (x, y) = o.at(strip::centre(
            Rail::Plugin,
            StripTarget::Tab(strip::Bank::Mode, 1),
        ));
        let at = (((y.round() as u32) << 16) | (x.round() as u32 & 0xFFFF)) as LPARAM;
        let w = o.window();
        // SAFETY: messages posted to the test's own window.
        unsafe { PostMessageW(w, WM_LBUTTONDOWN, MK_LBUTTON as WPARAM, at) };
        pump(Duration::from_millis(60));
        // SAFETY: as above.
        unsafe { PostMessageW(w, WM_LBUTTONUP, 0, at) };
        pump(Duration::from_millis(300));
        assert!(
            o.host.gestures.load(Ordering::Relaxed) >= 1,
            "POLY's tab not chosen"
        );
        o.close();
    }

    /// The window shown where it changed (decisions.md R58, R59: on Windows the
    /// changed places alone are turned and sent to the window) is the window shown whole: the
    /// synth playing (QUALITY's opening following it, the voices' drops), a knob turned, the
    /// presets' drawer sliding open and shut, then all of it still; every pixel of the window
    /// then as when Windows has the editor repaint all of it. Still is the same frame at every
    /// look for longer than anything on the panel holds still before it moves again: at LO, a
    /// new instance's QUALITY (R-LO-DEFAULT), the hamster's wheel stops and he stands
    /// [`DOZE_AFTER`] before he dozes off (two frames alike 400 ms apart, there, were called
    /// still and he curled up after: R-STILL-LONGER).
    #[test]
    fn the_window_shown_in_places_is_the_window_shown_whole() {
        let o = Opened::new(false, HostWindow::Dialog);
        let play = |levels: f32, ms: u64| {
            let until = Instant::now() + Duration::from_millis(ms);
            let mut k = 0u32;
            while Instant::now() < until {
                let mut l = [0.0f32; LEVELS];
                for (v, l) in l.iter_mut().enumerate().take(4) {
                    *l = levels * (0.6 + 0.4 * ((k as usize + v) % 3) as f32 / 2.0);
                }
                let mask = if levels > 0.0 { 0b1111 } else { 0 };
                o.meters.publish(0.0, (0.5, 0.0), mask);
                o.meters.publish_levels(&l, levels);
                pump(Duration::from_millis(30));
                k += 1;
            }
        };
        play(0.6, 1500);
        let cutoff = &CONTROLS[ca72_panel::controls::index("cutoff").expect("a control")];
        o.drag(o.at(cutoff.centre()), 50.0);
        play(0.8, 800);
        o.click_name();
        play(0.5, 700);
        o.click_name();
        play(0.0, 300);
        // Still: the same frame at every look, 400 ms apart, for a second past the longest
        // anything holds still.
        let span = Duration::from_secs_f64(DOZE_AFTER + 1.0);
        let until = Instant::now() + Duration::from_secs(20);
        let mut still = every_pixel(&o);
        let mut since = Instant::now();
        while since.elapsed() < span {
            pump(Duration::from_millis(400));
            let now = every_pixel(&o);
            if now != still {
                assert!(Instant::now() < until, "the window never still");
                (still, since) = (now, Instant::now());
            }
        }
        assert!(colours(&still) >= 100, "the panel shown");
        // Windows asks for all of it again: the editor shows its whole frame.
        let flags = RDW_INVALIDATE | RDW_ERASE | RDW_UPDATENOW | RDW_ALLCHILDREN;
        // SAFETY: the test's own windows.
        unsafe { RedrawWindow(o.parent, null_mut(), null_mut(), flags) };
        pump(Duration::from_millis(200));
        let whole = every_pixel(&o);
        let apart = still.iter().zip(&whole).filter(|(a, b)| a != b).count();
        assert!(
            apart == 0,
            "{apart} pixels shown otherwise than in the whole frame"
        );
        o.close();
    }

    /// An editor whose window cannot be seen (its host's window minimised) draws its frame a
    /// few times a second instead of at the screen's rate, and shows none of it; seen again,
    /// it shows its whole frame (decisions.md R-EDITOR-CPU).
    #[test]
    fn an_unseen_window_draws_seldom_and_is_whole_when_seen_again() {
        use winapi::um::winuser::{SW_MINIMIZE, SW_RESTORE, ShowWindow};
        let o = Opened::new(false, HostWindow::Dialog);
        let play = |levels: f32, ms: u64| {
            let until = Instant::now() + Duration::from_millis(ms);
            let mut k = 0u32;
            while Instant::now() < until {
                let mut l = [0.0f32; LEVELS];
                for (v, l) in l.iter_mut().enumerate().take(4) {
                    *l = levels * (0.6 + 0.4 * ((k as usize + v) % 3) as f32 / 2.0);
                }
                o.meters.publish(0.0, (0.5, 0.0), 0b1111);
                o.meters.publish_levels(&l, levels);
                pump(Duration::from_millis(30));
                k += 1;
            }
        };
        let counts = || (DRAWN.load(Ordering::Relaxed), SHOWN.load(Ordering::Relaxed));
        play(0.6, 800);
        let (d0, s0) = counts();
        play(0.6, 2000);
        let (d1, s1) = counts();
        // SAFETY: the test's own window.
        unsafe { ShowWindow(o.parent, SW_MINIMIZE) };
        play(0.6, 400);
        let (d2, s2) = counts();
        play(0.6, 2000);
        let (d3, s3) = counts();
        let (seen, unseen) = (d1 - d0, d3 - d2);
        println!(
            "2 s seen: {seen} frames drawn, {} shown; 2 s unseen: {unseen} drawn, {} shown",
            s1 - s0,
            s3 - s2
        );
        assert!(seen >= 40, "{seen} frames drawn in 2 s while seen");
        assert!(unseen <= 12, "{unseen} frames drawn in 2 s while unseen");
        assert_eq!(s3 - s2, 0, "frames shown while unseen");
        // SAFETY: the test's own window.
        unsafe { ShowWindow(o.parent, SW_RESTORE) };
        play(0.0, 300);
        // Still (as `the_window_shown_in_places_is_the_window_shown_whole` waits for it), then
        // the window shown whole by Windows' own repaint: the same pixels.
        let span = Duration::from_secs_f64(crate::opening::DOZE_AFTER + 1.0);
        let until = Instant::now() + Duration::from_secs(20);
        let mut still = every_pixel(&o);
        let mut since = Instant::now();
        while since.elapsed() < span {
            pump(Duration::from_millis(400));
            let now = every_pixel(&o);
            if now != still {
                assert!(Instant::now() < until, "the window never still");
                (still, since) = (now, Instant::now());
            }
        }
        assert!(colours(&still) >= 100, "the panel shown again");
        let flags = RDW_INVALIDATE | RDW_ERASE | RDW_UPDATENOW | RDW_ALLCHILDREN;
        // SAFETY: the test's own windows.
        unsafe { RedrawWindow(o.parent, null_mut(), null_mut(), flags) };
        pump(Duration::from_millis(200));
        let whole = every_pixel(&o);
        let apart = still.iter().zip(&whole).filter(|(a, b)| a != b).count();
        assert_eq!(
            apart, 0,
            "{apart} pixels shown otherwise than in the whole frame"
        );
        o.close();
    }

    /// The grip asks for its width at the editor's next frame; the host resizes the editor's
    /// window from within that.
    #[test]
    fn the_grip_resizes_where_the_host_resizes_the_editors_window() {
        let o = Opened::new(true, HostWindow::OffScreen);
        let grip = o.at((
            (art::GRIP[0] + art::GRIP[2]) / 2.0,
            (art::GRIP[1] + art::GRIP[3]) / 2.0,
        ));
        o.drag(grip, 120.0);
        assert!(
            o.host.resizes.load(Ordering::Relaxed) >= 1,
            "the host asked"
        );
        let asked = o.editor.size();
        assert_eq!(
            settled(&o, asked),
            asked,
            "the window as the editor last asked"
        );
        o.close();
    }

    /// Windows repainting the editor's window, in a host whose window paints its background
    /// over it as REAPER's FX window does: the editor shows its frame again, though nothing in it
    /// changed. 0.1.0 left the host's background there until something in the panel changed.
    #[test]
    fn the_panel_is_shown_again_as_windows_repaints_its_window() {
        let o = Opened::new(false, HostWindow::Dialog);
        let panel = pixels(&o);
        assert!(
            colours(&panel) >= 100,
            "the panel shown ({} colours)",
            colours(&panel)
        );
        let repaint = |flags| {
            // SAFETY: the test's own window.
            unsafe { RedrawWindow(o.parent, null_mut(), null_mut(), flags) };
            pump(Duration::from_millis(100));
            pixels(&o)
        };
        // The host's window alone: its background over the editor's, which Windows does not
        // ask to repaint. So this host's painting reaches the editor's pixels here.
        let shown = repaint(RDW_INVALIDATE | RDW_ERASE | RDW_UPDATENOW | RDW_NOCHILDREN);
        assert_eq!(colours(&shown), 1, "the host's background");
        // Both windows, as a host repainting its own does; then a capture of the host's.
        let shown = repaint(RDW_INVALIDATE | RDW_ERASE | RDW_UPDATENOW | RDW_ALLCHILDREN);
        assert!(
            shown == panel,
            "the panel again within 100 ms of the host's window repainted ({} colours)",
            colours(&shown)
        );
        print(o.parent);
        pump(Duration::from_millis(100));
        let shown = pixels(&o);
        assert!(
            shown == panel,
            "the panel again within 100 ms of the host's window captured ({} colours)",
            colours(&shown)
        );
        // The drawer open over the strip, the window as it is: shown again as Windows repaints
        // it.
        o.click_name();
        pump(Duration::from_millis(600));
        let drawer = pixels(&o);
        assert!(drawer != panel, "the drawer shown");
        assert!(
            colours(&drawer) >= 100,
            "the panel and the drawer shown ({} colours)",
            colours(&drawer)
        );
        let shown = repaint(RDW_INVALIDATE | RDW_ERASE | RDW_UPDATENOW | RDW_ALLCHILDREN);
        assert!(
            shown == drawer,
            "the panel and the drawer again ({} colours)",
            colours(&shown)
        );
        o.close();
    }

    /// While the editor holds the keyboard (the presets' drawer open, a control's menu), its
    /// window asks a host's dialog for every key: in REAPER's FX window, a dialog, an arrow moved
    /// REAPER's focus to one of its own buttons rather than reaching the drawer's list
    /// (decisions.md R34). And only then: otherwise the dialog keeps its keys.
    #[test]
    fn the_editor_asks_a_dialog_for_every_key_only_while_it_holds_them() {
        let o = Opened::new(false, HostWindow::OffScreen);
        let asks = || {
            // SAFETY: a message sent to the test's own window.
            let code = unsafe { SendMessageW(o.window(), WM_GETDLGCODE, 0, 0) };
            code & DLGC_WANTALLKEYS != 0
        };
        assert!(!asks(), "nothing held");
        o.click_name();
        assert!(asks(), "the presets' drawer open");
        o.click_name();
        pump(Duration::from_millis(400));
        assert!(!asks(), "the drawer shut");
        // A right click on CUTOFF FREQUENCY: its menu.
        let cutoff = &CONTROLS[ca72_panel::controls::index("cutoff").expect("a control")];
        let (x, y) = o.at(cutoff.centre());
        let at = (((y.round() as u32) << 16) | (x.round() as u32 & 0xFFFF)) as LPARAM;
        let w = o.window();
        // SAFETY: messages posted to the test's own window.
        unsafe {
            PostMessageW(w, WM_MOUSEMOVE, 0, at);
            PostMessageW(w, WM_RBUTTONDOWN, MK_RBUTTON as WPARAM, at);
        }
        pump(Duration::from_millis(50));
        // SAFETY: as above.
        unsafe { PostMessageW(w, WM_RBUTTONUP, 0, at) };
        pump(Duration::from_millis(300));
        assert!(asks(), "a control's menu open");
        // A click on the wood above the panel: the menu shut.
        o.drag(o.at((art::W / 2.0, 20.0)), 0.0);
        assert!(!asks(), "the menu shut");
        o.close();
    }

    /// The standalone's window and its editor at a scale (decisions.md R-WIN-STANDALONE, issue
    /// 64): the window's client the editor's window, pixel for pixel, as it opens, as the editor
    /// resizes it and once Windows has scaled the window for its monitor's new scale; and no
    /// frame to drag and no maximise box. The window as the app opens it (nih-plug's
    /// `production::app_window::options`, baseview's window of its own), the editor in it as it
    /// opens there, and a handler that fits the window as the app's does (`app_window::fit`
    /// and `refits`). Before, the editor
    /// drew at 1 in a window at the monitor's scale (at 125 %, 1143 by 653 pixels in a client of
    /// 1429 by 816, black beyond it), and the frame could be dragged and maximised with the
    /// editor left as it was.
    #[cfg(feature = "standalone-production")]
    #[test]
    fn the_standalone_s_window_is_its_editor_s_at_every_scale() {
        use nih_plug::wrapper::production::app_window;
        use raw_window_handle::{HasRawWindowHandle, RawWindowHandle};
        use winapi::um::winuser::{GWL_STYLE, GetWindowLongPtrW, WS_MAXIMIZEBOX, WS_THICKFRAME};

        /// As the app's context: a resize asked is made at the window's next frame.
        struct App {
            resize: AtomicBool,
        }
        impl GuiContext for App {
            fn plugin_api(&self) -> PluginApi {
                PluginApi::Standalone
            }
            fn request_resize(&self) -> bool {
                self.resize.store(true, Ordering::Release);
                true
            }
            unsafe fn raw_begin_set_parameter(&self, _param: ParamPtr) {}
            unsafe fn raw_set_parameter_normalized(&self, _param: ParamPtr, _normalized: f32) {}
            unsafe fn raw_end_set_parameter(&self, _param: ParamPtr) {}
            fn get_state(&self) -> PluginState {
                panic!("the editor does not read the state")
            }
            fn set_state(&self, _state: PluginState) {}
        }

        /// What was seen at a step: the window's client, the editor's, what the editor asks.
        type Seen = ((u32, u32), (u32, u32), (u32, u32));

        /// The app's window's handler: at its frames the steps (a look, then a click on PANEL
        /// or the window scaled from outside), then it closes.
        struct Frames {
            editor: Arc<Ca72Editor>,
            app: Arc<App>,
            _handle: Box<dyn Any + Send>,
            window: HWND,
            frames: u32,
            seen: Arc<Mutex<Vec<Seen>>>,
            style: Arc<AtomicU32>,
        }
        impl Frames {
            fn client(w: HWND) -> (u32, u32) {
                let mut r = RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                };
                // SAFETY: the test's own window and a rectangle of this function's.
                unsafe { GetClientRect(w, &mut r) };
                ((r.right - r.left) as u32, (r.bottom - r.top) as u32)
            }
            fn look(&self) {
                // SAFETY: the test's own window.
                let child = unsafe { GetWindow(self.window, GW_CHILD) };
                let seen = (
                    Self::client(self.window),
                    Self::client(child),
                    self.editor.size(),
                );
                self.seen.lock().expect("seen").push(seen);
            }
            fn click_panel(&self) {
                // SAFETY: the test's own window.
                let child = unsafe { GetWindow(self.window, GW_CHILD) };
                let k = f64::from(Self::client(child).0) / art::W;
                let (x, y) = strip::centre(Rail::Plugin, StripTarget::Bar(BarTarget::Panel));
                let at = ((((y * k).round() as u32) << 16) | ((x * k).round() as u32 & 0xFFFF))
                    as LPARAM;
                // SAFETY: messages posted to the test's own window.
                unsafe {
                    PostMessageW(child, WM_MOUSEMOVE, 0, at);
                    PostMessageW(child, WM_LBUTTONDOWN, MK_LBUTTON as WPARAM, at);
                    PostMessageW(child, WM_LBUTTONUP, 0, at);
                }
            }
        }
        impl baseview::WindowHandler for Frames {
            fn on_frame(&mut self, window: &mut Window<'_>) {
                if self.app.resize.swap(false, Ordering::AcqRel) {
                    app_window::fit(window, self.editor.size());
                }
                self.frames += 1;
                match self.frames {
                    20 => {
                        // SAFETY: the test's own window.
                        let style = unsafe { GetWindowLongPtrW(self.window, GWL_STYLE) };
                        self.style.store(style as u32, Ordering::Relaxed);
                        self.look();
                        self.click_panel();
                    }
                    50 => {
                        self.look();
                        self.click_panel();
                    }
                    80 => {
                        self.look();
                        // The window scaled by Windows, as when its monitor's scale changes
                        // (from 125 % to 150 %, the window's client 1.2 times larger).
                        let (w, h) = Self::client(self.window);
                        let (w, h) = (w as i32 * 6 / 5, h as i32 * 6 / 5);
                        // SAFETY: the test's own window, on its thread.
                        unsafe {
                            SetWindowPos(
                                self.window,
                                null_mut(),
                                0,
                                0,
                                w,
                                h,
                                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                            )
                        };
                    }
                    110 => {
                        self.look();
                        window.close();
                    }
                    _ => {}
                }
            }
            fn on_event(&mut self, _window: &mut Window<'_>, event: Event) -> EventStatus {
                if app_window::refits(&event) {
                    self.app.resize.store(true, Ordering::Release);
                }
                EventStatus::Ignored
            }
        }

        let _desktop = DESKTOP
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // The windows in pixels whatever the screen's scale, as `app_window::scale` makes the
        // app's process (the scale here is the test's, not the monitor's).
        // SAFETY: this thread's own setting.
        unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        for scale in [1.0f32, 1.25, 1.5, 1.75, 2.0] {
            let presets = tempfile::tempdir().expect("a temporary folder");
            let params = Arc::new(Ca72Params::default());
            params.editor_width.store(MIN_WIDTH, Ordering::Relaxed);
            let editor = Arc::new(Ca72Editor::new(
                Arc::clone(&params),
                Arc::new(Meters::default()),
            ));
            let options = app_window::options("CA-72".into(), editor.as_ref(), Some(scale));
            let app = Arc::new(App {
                resize: AtomicBool::new(false),
            });
            let seen = Arc::new(Mutex::new(Vec::new()));
            let style = Arc::new(AtomicU32::new(0));
            let (handler_editor, handler_app, handler_seen, handler_style) = (
                Arc::clone(&editor),
                Arc::clone(&app),
                Arc::clone(&seen),
                Arc::clone(&style),
            );
            let library = presets.path().to_path_buf();
            Window::open_blocking(options, move |window: &mut Window<'_>| {
                let RawWindowHandle::Win32(h) = window.raw_window_handle() else {
                    panic!("a Win32 window");
                };
                let hwnd: HWND = h.hwnd.cast();
                // Off the screen, not brought to the front.
                // SAFETY: the window being made, on this thread.
                unsafe {
                    SetWindowPos(
                        hwnd,
                        null_mut(),
                        -20000,
                        -20000,
                        0,
                        0,
                        SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                    )
                };
                let context: Arc<dyn GuiContext> = handler_app.clone();
                let handle = handler_editor.spawn_with(
                    ParentWindowHandle::Win32Hwnd(h.hwnd),
                    context,
                    Library::at(library),
                );
                Frames {
                    editor: handler_editor,
                    app: handler_app,
                    _handle: handle,
                    window: hwnd,
                    frames: 0,
                    seen: handler_seen,
                    style: handler_style,
                }
            });
            let at = |(w, h): (u32, u32)| {
                let k = f64::from(scale);
                (
                    (f64::from(w) * k).round() as u32,
                    (f64::from(h) * k).round() as u32,
                )
            };
            let seen = seen.lock().expect("seen").clone();
            assert_eq!(seen.len(), 4, "at {scale}: the window closed early");
            let full = (MIN_WIDTH, height_for_panel(MIN_WIDTH, true));
            let compact = (MIN_WIDTH, height_for_panel(MIN_WIDTH, false));
            for ((window, child, asked), (step, want)) in seen.iter().zip([
                ("opened", full),
                ("PANEL off", compact),
                ("PANEL on", full),
                ("scaled by Windows", full),
            ]) {
                assert_eq!(*asked, want, "at {scale}, {step}: the editor's size");
                assert_eq!(*child, at(want), "at {scale}, {step}: the editor's window");
                assert_eq!(
                    window, child,
                    "at {scale}, {step}: the app's window's client"
                );
            }
            let style = style.load(Ordering::Relaxed);
            assert_eq!(style & WS_THICKFRAME, 0, "at {scale}: a frame to drag");
            assert_eq!(style & WS_MAXIMIZEBOX, 0, "at {scale}: a maximise box");
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod linux_resize_tests;
