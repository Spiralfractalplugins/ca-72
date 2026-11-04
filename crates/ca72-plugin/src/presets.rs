//! The presets in the editor (decisions.md R10): the bar under the strip and the drawer over
//! the panel (`ca72_panel::presets` draws them), working on the library shared with the
//! DAW the model was developed in (`crate::library`).
//!
//! Choosing a preset sets every parameter but the PITCH wheel and the bypass to its value or
//! its default (POLY, VOICES, ENTROPY and SPREAD among them: R12), each a gesture of its own;
//! MIDI BEND RANGE, the player's keyboard's, only when the preset names it (R18). The plug-in
//! remembers it (`preset`, saved with the session); the bar marks it • once a value leaves
//! it. The drawer (the mock-up's, A6: decisions.md R44): typing goes to the field with the
//! caret (the search when none); the arrow keys step through the list, setting each preset;
//! Enter commits a field (the search: closes) or, once DELETE has asked, deletes; Tab moves to
//! the next field shown; Escape takes back what a key began, else closes; a double click on a
//! row chooses it and closes the drawer. The PRESET keys RENAME, TAGS, DELETE and REVERT act
//! on the preset the plug-in is set to; what they begin stays with that preset as the list
//! changes (a filter, another program's change) and ends when it is gone or another is set,
//! so it never acts on another (R18). SAVE AS begins a save (the bar's SAVE as well), and saves
//! when pressed again. The library is read again when the drawer opens, after each change and
//! every two seconds while it is open, so that DAW's changes show too (its files parsed only
//! when the folder changed); typing in the search and the filters work on the presets as read
//! (R18).

use std::time::{Duration, Instant};

use ca72_panel::presets::{
    BarScene, BarTarget, Current, DrawerScene, DrawerTarget, Edit, Field, FieldId, PresetKey,
    ROWS_SHOWN, Row, TAG_LINES, Tab, caret_at,
};
use keyboard_types::{Key, KeyState, KeyboardEvent, Modifiers};
use nih_plug::prelude::*;

use crate::library::{self, Entry, Library, Origin};
use crate::params::{Ca72Params, QualityMode};

/// How long the drawer takes to slide open or shut.
pub const SLIDE: Duration = Duration::from_millis(140);
/// How often the library is read again while the drawer is open.
const REREAD: Duration = Duration::from_secs(2);
/// Two clicks on a row within this long: chosen, and the drawer closed.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// A parameter as a preset sets it: its plain value as the files give it (a choice by its
/// index, a switch 0 or 1), the normalized value of a plain one, and one change (a gesture).
pub trait SoundParam {
    fn plain(&self) -> f64;
    fn normalized(&self) -> f32;
    fn normalized_of(&self, plain: f64) -> f32;
    fn default_normalized(&self) -> f32;
    fn set_once(&self, s: &ParamSetter<'_>, normalized: f32);
}

impl SoundParam for FloatParam {
    fn plain(&self) -> f64 {
        f64::from(self.unmodulated_plain_value())
    }
    fn normalized(&self) -> f32 {
        self.unmodulated_normalized_value()
    }
    fn normalized_of(&self, plain: f64) -> f32 {
        self.preview_normalized(plain as f32)
    }
    fn default_normalized(&self) -> f32 {
        Param::default_normalized_value(self)
    }
    fn set_once(&self, s: &ParamSetter<'_>, normalized: f32) {
        s.begin_set_parameter(self);
        s.set_parameter_normalized(self, normalized);
        s.end_set_parameter(self);
    }
}

impl SoundParam for IntParam {
    fn plain(&self) -> f64 {
        f64::from(self.unmodulated_plain_value())
    }
    fn normalized(&self) -> f32 {
        self.unmodulated_normalized_value()
    }
    fn normalized_of(&self, plain: f64) -> f32 {
        self.preview_normalized(plain.round() as i32)
    }
    fn default_normalized(&self) -> f32 {
        Param::default_normalized_value(self)
    }
    fn set_once(&self, s: &ParamSetter<'_>, normalized: f32) {
        s.begin_set_parameter(self);
        s.set_parameter_normalized(self, normalized);
        s.end_set_parameter(self);
    }
}

impl SoundParam for BoolParam {
    fn plain(&self) -> f64 {
        f64::from(u8::from(self.unmodulated_plain_value()))
    }
    fn normalized(&self) -> f32 {
        self.unmodulated_normalized_value()
    }
    fn normalized_of(&self, plain: f64) -> f32 {
        self.preview_normalized(plain >= 0.5)
    }
    fn default_normalized(&self) -> f32 {
        Param::default_normalized_value(self)
    }
    fn set_once(&self, s: &ParamSetter<'_>, normalized: f32) {
        s.begin_set_parameter(self);
        s.set_parameter_normalized(self, normalized);
        s.end_set_parameter(self);
    }
}

impl<T: Enum + PartialEq + 'static> SoundParam for EnumParam<T> {
    fn plain(&self) -> f64 {
        self.unmodulated_plain_value().to_index() as f64
    }
    fn normalized(&self) -> f32 {
        self.unmodulated_normalized_value()
    }
    fn normalized_of(&self, plain: f64) -> f32 {
        let n = T::variants().len().max(1);
        self.preview_normalized(T::from_index((plain.round().max(0.0) as usize).min(n - 1)))
    }
    fn default_normalized(&self) -> f32 {
        Param::default_normalized_value(self)
    }
    fn set_once(&self, s: &ParamSetter<'_>, normalized: f32) {
        s.begin_set_parameter(self);
        s.set_parameter_normalized(self, normalized);
        s.end_set_parameter(self);
    }
}

/// Every parameter a preset sets, by its ID (all but the PITCH wheel and the bypass).
pub fn sound_params(p: &Ca72Params) -> Vec<(&'static str, &dyn SoundParam)> {
    let mut all: Vec<(&'static str, &dyn SoundParam)> = vec![
        ("tune", &p.tune),
        ("glide", &p.glide),
        ("mod_mix", &p.mod_mix),
        ("osc_mod", &p.osc_mod),
        ("osc3_control", &p.osc3_control),
        ("osc1_range", &p.osc1_range),
        ("osc1_waveform", &p.osc1_waveform),
        ("osc2_range", &p.osc2_range),
        ("osc2_frequency", &p.osc2_frequency),
        ("osc2_waveform", &p.osc2_waveform),
        ("osc3_range", &p.osc3_range),
        ("osc3_frequency", &p.osc3_frequency),
        ("osc3_waveform", &p.osc3_waveform),
        ("osc1_on", &p.osc1_on),
        ("osc1_volume", &p.osc1_volume),
        ("ext_on", &p.ext_on),
        ("ext_volume", &p.ext_volume),
        ("osc2_on", &p.osc2_on),
        ("osc2_volume", &p.osc2_volume),
        ("noise_on", &p.noise_on),
        ("noise_volume", &p.noise_volume),
        ("noise_type", &p.noise_type),
        ("osc3_on", &p.osc3_on),
        ("osc3_volume", &p.osc3_volume),
        ("filter_mode", &p.filter_mode),
        // FILTER POLES (decisions.md R-POLES): a preset that does not name it is at 4, as drawn.
        ("filter_poles", &p.filter_poles),
        ("filter_mod", &p.filter_mod),
        ("keyboard_control_1", &p.keyboard_control_1),
        ("keyboard_control_2", &p.keyboard_control_2),
        ("cutoff", &p.cutoff),
        ("emphasis", &p.emphasis),
        ("contour_amount", &p.contour_amount),
        ("filter_attack", &p.filter_attack),
        ("filter_decay", &p.filter_decay),
        ("filter_sustain", &p.filter_sustain),
        ("loudness_attack", &p.loudness_attack),
        ("loudness_decay", &p.loudness_decay),
        ("loudness_sustain", &p.loudness_sustain),
        ("volume", &p.volume),
        ("main_output", &p.main_output),
        ("a440", &p.a440),
        ("glide_on", &p.glide_on),
        ("decay_on", &p.decay_on),
        ("note_priority", &p.note_priority),
        ("retrigger", &p.retrigger),
        ("mod_wheel", &p.mod_wheel),
        ("midi_bend_range", &p.midi_bend_range),
        ("poly", &p.poly),
        ("voices", &p.voices),
        ("entropy", &p.entropy),
        ("spread", &p.spread),
        ("inner", &p.inner),
        ("placement", &p.placement),
        ("unison", &p.unison),
        ("double", &p.double),
        ("drive", &p.drive),
        ("level", &p.level),
        ("auto_gain", &p.auto_gain),
        ("doubled", &p.doubled),
        ("feedback", &p.feedback),
        ("lock", &p.lock),
        // The arpeggiator (decisions.md R-ARP): a preset that does not name it plays without it.
        ("arp_on", &p.arp_on),
        ("arp_latch", &p.arp_latch),
        ("arp_direction", &p.arp_direction),
        ("arp_rate", &p.arp_rate),
        ("arp_octaves", &p.arp_octaves),
        ("arp_gate", &p.arp_gate),
        ("arp_swing", &p.arp_swing),
        // Each note's own pressure and timbre's reach (decisions.md R-MPE): a preset that does
        // not name them plays as before, untouched by them.
        ("pressure_mod", &p.pressure_mod),
        ("pressure_filter", &p.pressure_filter),
        ("timbre_mod", &p.timbre_mod),
        ("timbre_filter", &p.timbre_filter),
        // The velocity's CURVE and FLOOR (decisions.md R-VELOCITY): a preset that does not name
        // them has a straight line from 0.
        ("velocity_curve", &p.velocity_curve),
        ("velocity_floor", &p.velocity_floor),
        // The oscillators timed (decisions.md R-OSC-TIMED): a preset that does not name them
        // plays them free.
        ("osc2_timed", &p.osc2_timed),
        ("osc2_division", &p.osc2_division),
        ("osc3_timed", &p.osc3_timed),
        ("osc3_division", &p.osc3_division),
        // Each oscillator's waveform (decisions.md R-CONTINUOUS-WAVE): a preset that does not
        // name it plays the instrument's switch.
        ("osc1_shape_mode", &p.osc1_shape_mode),
        ("osc1_shape", &p.osc1_shape),
        ("osc1_shape_mod", &p.osc1_shape_mod),
        ("osc1_shape_mod_dir", &p.osc1_shape_mod_dir),
        ("osc2_shape_mode", &p.osc2_shape_mode),
        ("osc2_shape", &p.osc2_shape),
        ("osc2_shape_mod", &p.osc2_shape_mod),
        ("osc2_shape_mod_dir", &p.osc2_shape_mod_dir),
        ("osc3_shape_mode", &p.osc3_shape_mode),
        ("osc3_shape", &p.osc3_shape),
        ("osc3_shape_mod", &p.osc3_shape_mod),
        ("osc3_shape_mod_dir", &p.osc3_shape_mod_dir),
        ("glide_range", &p.glide_range),
    ];
    // The LFOs (decisions.md R-LFO): a preset that does not name them has each at its default;
    // what they modulate is the preset's too (`lfo::Patch::values`), none where it names none.
    for (n, ids) in crate::lfo::PARAM_IDS.iter().enumerate() {
        for (id, q) in ids.iter().zip(crate::lfo::sound_params(p, n)) {
            all.push((id, q));
        }
    }
    // The macros (decisions.md R-MACROS): a preset that does not name them has each at 0; their
    // names are the preset's too (`macro_names`), none where it names none.
    for (n, id) in crate::lfo::MACRO_IDS.iter().enumerate() {
        all.push((id, p.macro_param(n)));
    }
    all
}

/// The player's parameters, not the sound's: MIDI BEND RANGE suits the keyboard played. A
/// preset saved does not name it, and choosing one that does not leaves it as it is (one that
/// does, another program's, sets it) (decisions.md R18).
pub const PLAYERS: &[&str] = &["midi_bend_range"];

/// What a preset sets each parameter to (normalized): its value, else the parameter's
/// default (but [`PLAYERS`]'s). POLY, VOICES, ENTROPY and SPREAD are part of a preset like
/// any other (decisions.md R12; R10 had left POLY and VOICES as they were when a preset did not
/// name them).
pub(crate) fn targets(p: &Ca72Params, e: &Entry) -> Vec<(&'static str, f32)> {
    targets_for_sound(p, &e.sound)
}

/// `targets` for a sound (the factory measurements load presets by it: `tests/common`).
pub fn targets_for_sound(
    p: &Ca72Params,
    sound: &crate::library::Sound,
) -> Vec<(&'static str, f32)> {
    let named = |id: &str| sound.values.iter().find(|(k, _)| k == id).map(|(_, v)| *v);
    // (A preset 0.3.0's beta saved names its LFOs' RISE: their FADE, a fade in, R-LFO-FADE.)
    let value = |id: &str| named(id).or_else(|| crate::lfo::rise_id(id).and_then(named));
    sound_params(p)
        .into_iter()
        .filter_map(|(id, q)| match value(id) {
            Some(v) => Some((id, q.normalized_of(v))),
            None if PLAYERS.contains(&id) => None,
            // (A preset saved before DOUBLE was a switch of its own: DOUBLE where its DETUNE
            // is above 0.)
            None if id == "doubled" => Some((
                id,
                q.normalized_of(f64::from(u8::from(
                    value("double").is_some_and(|d| d > 0.0),
                ))),
            )),
            None => Some((id, q.default_normalized())),
        })
        .collect()
}

/// The current sound as a preset's values (plain, four decimals): not [`PLAYERS`]'s.
pub fn values_now(p: &Ca72Params) -> Vec<(String, f64)> {
    let mut values: Vec<(String, f64)> = sound_params(p)
        .into_iter()
        .filter(|(id, _)| !PLAYERS.contains(id))
        .map(|(id, q)| (id.to_owned(), (q.plain() * 1e4).round() / 1e4))
        .collect();
    // (What the LFOs modulate, and their EDGES: decisions.md R-LFO.)
    values.extend(p.lfo_patch.values());
    values
}

/// The macros' names as a preset keeps them: none where none is typed (decisions.md R-MACROS).
pub fn names_now(p: &Ca72Params) -> Vec<String> {
    let names = p.lfo_patch.names();
    if names.iter().all(String::is_empty) {
        Vec::new()
    } else {
        names.to_vec()
    }
}

/// Where a preset is from, as a row says it: `EDITED` (the user's version of a factory
/// preset) or `YOURS`; nothing for the factory's.
fn origin_word(o: Origin) -> Option<&'static str> {
    match o {
        Origin::Factory => None,
        Origin::Edited => Some("EDITED"),
        Origin::User => Some("YOURS"),
    }
}

fn same(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// A save taking fresh effect snapshots off the editor thread (R-FX-PRESETS).
#[derive(Debug)]
pub(crate) struct Saving {
    before: String,
    pub(crate) name: String,
    result: std::sync::mpsc::Receiver<Result<Entry, String>>,
}

impl Saving {
    pub(crate) fn poll(&self) -> Option<Result<Entry, String>> {
        match self.result.try_recv() {
            Ok(r) => Some(r),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Some(Err("The preset could not be saved.".into()))
            }
        }
    }
}

struct SaveRequest {
    library: Library,
    name: String,
    values: Vec<(String, f64)>,
    tags: Vec<String>,
    curve: (bool, [f32; crate::drive::STEPS]),
    names: Vec<String>,
    effects: Option<std::sync::mpsc::Receiver<Result<Option<String>, String>>>,
    carried: Option<String>,
    reply: std::sync::mpsc::Sender<Result<Entry, String>>,
}

impl SaveRequest {
    fn run(self) {
        let result = (|| {
            let section = match self.effects {
                Some(rx) => rx.recv().map_err(|e| e.to_string())??,
                None => self.carried,
            };
            self.library.save_with(
                &self.name,
                self.values,
                Some(self.tags),
                Some(self.curve),
                self.names,
                section,
            )
        })();
        let _ = self.reply.send(result);
    }
}

/// The presets' bar and drawer: what they show and what operating them does.
pub struct Browser {
    pub library: Library,
    pub(crate) effects: Option<std::sync::Arc<dyn crate::fx_view::Effects>>,
    pub open: bool,
    /// The drawer sliding: when it began, and whether opening.
    slide: Option<(Instant, bool)>,
    pub bar: BarScene,
    pub drawer: DrawerScene,
    /// The list as filtered, and every preset.
    list: Vec<Entry>,
    all: Vec<Entry>,
    /// The current preset's values as set (normalized), to mark it changed.
    wanted: Option<(String, Vec<(&'static str, f32)>)>,
    /// Whether what the LFOs modulate is the current preset's: its name, the patch's change
    /// count it was worked out at, and the answer (decisions.md R-LFO).
    lfo_seen: Option<(String, u32, bool)>,
    read: Option<Instant>,
    /// The window should take the keyboard (a field took the caret).
    pub wants_keys: bool,
    /// The drawer opens below the bar, the window growing for it; false where the host
    /// would not resize the window (it then opens over the panel).
    pub below: bool,
    /// The last row clicked, and when: a second click soon after closes the drawer.
    last_row: Option<(Instant, usize)>,
    /// The preset being renamed, tagged or asked about deleting, by its name and origin: the
    /// edit stays with it as the list changes, and ends when it is gone or another preset is
    /// set, so it never acts on another (decisions.md R18).
    editing: Option<(String, Origin)>,
    saving: Option<Saving>,
    save_queue: std::sync::Mutex<Option<std::sync::mpsc::Sender<SaveRequest>>>,
}

impl std::fmt::Debug for Browser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Browser")
            .field("open", &self.open)
            .field("rows", &self.list.len())
            .finish()
    }
}

impl Browser {
    pub fn new(library: Library) -> Self {
        Browser {
            library,
            effects: crate::fx_view::installed(),
            open: false,
            slide: None,
            bar: BarScene::default(),
            drawer: DrawerScene::default(),
            list: Vec::new(),
            all: Vec::new(),
            wanted: None,
            lfo_seen: None,
            read: None,
            wants_keys: false,
            below: true,
            last_row: None,
            editing: None,
            saving: None,
            save_queue: std::sync::Mutex::new(None),
        }
    }

    /// The preset the plug-in was last set to.
    fn current(p: &Ca72Params) -> String {
        p.preset.read().map(|s| s.clone()).unwrap_or_default()
    }

    fn set_current(p: &Ca72Params, name: &str) {
        if let Ok(mut s) = p.preset.write() {
            *s = name.to_owned();
        }
    }

    /// How far the drawer is open, 0 to 1.
    pub fn reveal(&self) -> f64 {
        match self.slide {
            Some((t0, opening)) => {
                let k = (t0.elapsed().as_secs_f64() / SLIDE.as_secs_f64()).min(1.0);
                // Eased out.
                let k = 1.0 - (1.0 - k) * (1.0 - k);
                if opening { k } else { 1.0 - k }
            }
            None => f64::from(u8::from(self.open)),
        }
    }

    /// Whether the drawer is moving.
    pub fn sliding(&self) -> bool {
        self.slide.is_some_and(|(t0, _)| t0.elapsed() < SLIDE)
    }

    /// The drawer's slide as if begun at `t0` (tests: a moment of it, whatever the load).
    #[cfg(test)]
    pub(crate) fn slide_began(&mut self, t0: Instant) {
        if let Some((_, opening)) = self.slide {
            self.slide = Some((t0, opening));
        }
    }

    /// The library read again (once; unparsed when its folder has not changed), then the
    /// list filtered.
    pub fn reread(&mut self, p: &Ca72Params) {
        self.read = Some(Instant::now());
        self.all = self.library.entries().0;
        self.refilter(p);
    }

    /// The list as the filters have it and the tags, from the presets as read.
    fn refilter(&mut self, p: &Ca72Params) {
        let picked: Vec<String> = self
            .drawer
            .chips
            .iter()
            .filter(|(_, on)| *on)
            .map(|(t, _)| t.clone())
            .collect();
        self.list = library::matching(
            &self.all,
            &self.drawer.search.text,
            &picked,
            self.drawer.favourites,
            self.drawer.mine,
        );
        if self.drawer.factory {
            self.list.retain(|e| e.origin == Origin::Factory);
        }
        if let Ok(mut names) = p.midi_map.preset_list.lock() {
            *names = Some(self.list.iter().map(|e| e.sound.name.clone()).collect());
        }
        let mut chips: Vec<(String, bool)> = library::tags_of(&self.all)
            .into_iter()
            .map(|t| {
                let on = picked.iter().any(|x| same(x, &t));
                (t, on)
            })
            .collect();
        for t in picked {
            if !chips.iter().any(|(c, _)| same(c, &t)) {
                chips.push((t, true));
            }
        }
        self.drawer.chips = chips;
        let most = self
            .drawer
            .chips
            .len()
            .div_ceil(2)
            .saturating_sub(TAG_LINES);
        self.drawer.tags_first = self.drawer.tags_first.min(most);
        // What a key began on a preset ends when the preset has gone.
        if self.editing.is_some() && self.edited_entry().is_none() {
            self.end_edit();
        }
        self.rows(p);
    }

    /// The preset the plug-in is set to edited (`how`): renamed, tagged or asked about
    /// deleting.
    fn begin_edit(&mut self, e: &Entry, how: Edit) {
        self.drawer.editing = Some(how);
        self.drawer.hint.clear();
        self.editing = Some((e.sound.name.clone(), e.origin));
    }

    /// What a key began ended (done, taken back, or its preset gone); a field it had, the
    /// search's.
    fn end_edit(&mut self) {
        if matches!(
            self.drawer.focus,
            Some(FieldId::Edit | FieldId::SaveName | FieldId::SaveTags)
        ) {
            self.drawer.focus = Some(FieldId::Search);
        }
        self.drawer.editing = None;
        self.editing = None;
    }

    /// The preset being edited, among the presets as read.
    fn edited_entry(&self) -> Option<&Entry> {
        let (name, origin) = self.editing.as_ref()?;
        self.all
            .iter()
            .find(|e| same(&e.sound.name, name) && e.origin == *origin)
    }

    /// The preset being edited, the library read again first: none when it has gone (said).
    fn edited_now(&mut self, p: &Ca72Params) -> Option<Entry> {
        let who = self.editing.clone();
        self.reread(p);
        let e = self.edited_entry().cloned();
        if e.is_none()
            && let Some((name, _)) = who
        {
            self.drawer.hint = format!("{name} is no longer there.");
        }
        e
    }

    fn rows(&mut self, p: &Ca72Params) {
        let current = Self::current(p);
        self.drawer.rows = self
            .list
            .iter()
            .map(|e| Row {
                name: e.sound.name.clone(),
                tags: e.sound.tags.join(" "),
                origin: origin_word(e.origin),
                favorite: e.favorite,
                current: same(&e.sound.name, &current),
            })
            .collect();
        self.drawer.total = self.all.len();
        self.drawer.current = self
            .all
            .iter()
            .find(|e| same(&e.sound.name, &current))
            .map(|e| Current {
                name: e.sound.name.clone(),
                origin: origin_word(e.origin),
                tags: e.sound.tags.join(" "),
                favorite: e.favorite,
            });
        let most = self.drawer.rows.len().saturating_sub(ROWS_SHOWN);
        self.drawer.first = self.drawer.first.min(most);
    }

    /// Brings the bar and the drawer up to the parameters (each frame).
    pub fn tick(&mut self, p: &Ca72Params) {
        if let Some(saving) = self.saving.take() {
            if let Some(result) = saving.poll() {
                if let Ok(e) = &result {
                    self.finish_save(&saving, e, p);
                }
                self.saved_says(result, p);
            } else {
                self.saving = Some(saving);
            }
        }
        if self.open && self.read.is_none_or(|t| t.elapsed() > REREAD) {
            self.reread(p);
        }
        if self.slide.is_some_and(|(t0, _)| t0.elapsed() >= SLIDE) {
            self.slide = None;
        }
        let current = Self::current(p);
        if self.read.is_none() {
            self.all = self.library.entries().0;
            self.list = self.all.clone();
            self.read = Some(Instant::now());
        }
        // Another preset set (the host, the bar): what a key began on the last one ends, and
        // the drawer shows the one set.
        if self
            .editing
            .as_ref()
            .is_some_and(|(name, _)| !same(name, &current))
        {
            self.end_edit();
        }
        if self.open
            && self.drawer.current.as_ref().map(|c| c.name.as_str()) != Some(current.as_str())
            && self.all.iter().any(|e| same(&e.sound.name, &current))
        {
            self.rows(p);
        }
        let entry = self.all.iter().find(|e| same(&e.sound.name, &current));
        if self.wanted.as_ref().is_none_or(|(n, _)| !same(n, &current)) {
            self.wanted = entry.map(|e| (current.clone(), targets(p, e)));
        }
        let changed = self.wanted.as_ref().is_some_and(|(_, t)| {
            let now = sound_params(p);
            t.iter().any(|(id, v)| {
                now.iter()
                    .find(|(i, _)| i == id)
                    .is_some_and(|(_, q)| (q.normalized() - v).abs() > 1e-4)
            })
        });
        // (What the LFOs modulate, as the preset has it: worked out again only when the patch
        // or the preset changes.)
        let changed = changed
            || entry.is_some_and(|e| {
                let at = p.lfo_patch.changes();
                match &self.lfo_seen {
                    Some((n, g, d)) if same(n, &current) && *g == at => *d,
                    _ => {
                        let d = !p.lfo_patch.same_as_values(&e.sound.values)
                            || !p.lfo_patch.same_names(&e.sound.macro_names);
                        self.lfo_seen = Some((current.clone(), at, d));
                        d
                    }
                }
            });
        let fx = self
            .effects
            .as_ref()
            .map(|e| e.preset_status())
            .unwrap_or_default();
        let changed = changed || (!fx.locked && fx.differs);
        self.bar = BarScene {
            fx_locked: fx.locked,
            preset_fx: fx.held.is_some(),
            name: entry.map_or(current.clone(), |e| e.sound.name.clone()),
            found: entry.is_some(),
            changed: entry.is_some() && changed,
            favorite: entry.is_some_and(|e| e.favorite),
            open: self.open,
            below: self.below,
            hover: self.bar.hover,
            keyboard: self.bar.keyboard,
            panel: self.bar.panel,
            arp: self.bar.arp,
            fx: self.bar.fx,
            mpe: self.bar.mpe,
            lfo: self.bar.lfo,
            page: self.bar.page,
            lfo_page: self.bar.lfo_page,
            macros: self.bar.macros,
            macro_glow: self.bar.macro_glow,
        };
    }

    fn load_effects(&self, p: &Ca72Params, section: Option<String>, load: bool) {
        if let Ok(mut raw) = p.preset_effects.write() {
            *raw = section.clone();
        }
        if let Some(effects) = &self.effects {
            if load {
                effects.load_preset(section);
            } else {
                effects.observe_preset(section);
            }
        }
    }

    /// Opens or shuts the drawer.
    pub fn show(&mut self, on: bool, p: &Ca72Params) {
        if on == self.open {
            return;
        }
        self.open = on;
        self.slide = Some((Instant::now(), on));
        if on {
            self.reread(p);
            self.drawer.focus = Some(FieldId::Search);
            self.wants_keys = true;
        } else {
            self.end_edit();
            self.drawer.focus = None;
            self.drawer.hover = None;
        }
    }

    /// The plug-in set to a preset.
    pub fn choose(&mut self, e: &Entry, p: &Ca72Params, s: &ParamSetter<'_>) {
        // (What a key began on the last preset, or a save of its sound, ends.)
        self.end_edit();
        self.drawer.hint.clear();
        let params = sound_params(p);
        // (Its knobs land at once in the voices, not gliding: decisions.md R-KNOB-GLIDE.)
        let landing = p.lands.begin();
        for (id, v) in targets(p, e) {
            if let Some((_, q)) = params.iter().find(|(i, _)| *i == id)
                && (q.normalized() - v).abs() > 1e-6
            {
                p.lands.mark(landing, p, id, v);
                q.set_once(s, v);
            }
        }
        // What its LFOs and macros modulate, and its macros' names: none where it names none
        // (decisions.md R-LFO, R-MACROS).
        p.lfo_patch.load_values(&e.sound.values);
        p.lfo_patch.load_names(&e.sound.macro_names);
        // Its AUTO GAIN curve in force at once, nothing measured (decisions.md
        // R-AUTO-GAIN-DRIVE); a preset saved before curves keeps the one in force.
        if let Some(db) = e.sound.curve(p.quality.value() == QualityMode::Lo) {
            p.drive_curve.load_preset(db);
        }
        self.load_effects(p, e.sound.effects.clone(), true);
        Self::set_current(p, &e.sound.name);
        self.wanted = Some((e.sound.name.clone(), targets(p, e)));
        self.rows(p);
    }

    /// Start again from this build's fresh-instance sound. Use the parameters' own
    /// defaults, with the same player/computer exclusions as a saved sound. This is an
    /// action, not a library entry: it cannot replace a user's preset named Init.
    fn init(&mut self, p: &Ca72Params, s: &ParamSetter<'_>) {
        self.end_edit();
        // (Its knobs land at once, as a preset's: decisions.md R-KNOB-GLIDE.)
        let landing = p.lands.begin();
        for (id, q) in sound_params(p) {
            if !PLAYERS.contains(&id) {
                let v = q.default_normalized();
                if q.normalized() != v {
                    p.lands.mark(landing, p, id, v);
                    q.set_once(s, v);
                }
            }
        }
        // (No LFO assigned, as a fresh instance: decisions.md R-LFO.)
        p.lfo_patch.clear();
        // (AUTO GAIN's curve a fresh instance's too: the presets' average.)
        p.drive_curve.load(&crate::drive::unmeasured());
        self.load_effects(p, None, true);
        Self::set_current(p, "");
        self.wanted = None;
        self.last_row = None;
        self.drawer.focus = Some(FieldId::Search);
        self.drawer.hint = "Initial sound restored. SAVE AS keeps your new patch.".into();
        self.rows(p);
        self.tick(p);
    }

    /// The next (`by` 1) or previous (-1) preset of the list, set.
    pub fn step(&mut self, by: i32, p: &Ca72Params, s: &ParamSetter<'_>) {
        if self.list.is_empty() {
            return;
        }
        let current = Self::current(p);
        let n = self.list.len() as i32;
        let at = self.list.iter().position(|e| same(&e.sound.name, &current));
        let i = match at {
            Some(i) => (i as i32 + by).rem_euclid(n),
            None if by > 0 => 0,
            None => n - 1,
        } as usize;
        let e = self.list[i].clone();
        self.choose(&e, p, s);
        self.bring_into_view(&e.sound.name);
    }

    /// The list scrolled so the preset of this name shows.
    fn bring_into_view(&mut self, name: &str) {
        let Some(i) = self.list.iter().position(|e| same(&e.sound.name, name)) else {
            return;
        };
        if i < self.drawer.first {
            self.drawer.first = i;
        } else if i >= self.drawer.first + ROWS_SHOWN {
            self.drawer.first = i + 1 - ROWS_SHOWN;
        }
    }

    /// A press on the bar.
    pub fn bar_press(&mut self, t: BarTarget, p: &Ca72Params, s: &ParamSetter<'_>) {
        match t {
            BarTarget::Name => self.show(!self.open, p),
            BarTarget::Prev => self.step(-1, p, s),
            // (The editor's: its menu.)
            BarTarget::Settings
            | BarTarget::Panel
            | BarTarget::Global
            | BarTarget::Keyboard
            | BarTarget::Arp
            | BarTarget::Fx
            | BarTarget::Mpe
            | BarTarget::Lfo
            | BarTarget::Macro => {}
            BarTarget::Next => self.step(1, p, s),
            BarTarget::Star => {
                let current = Self::current(p);
                if let Some(e) = self.all.iter().find(|e| same(&e.sound.name, &current)) {
                    let on = !e.favorite;
                    let name = e.sound.name.clone();
                    self.report(self.library.favorite(&name, on));
                    self.reread(p);
                }
            }
            BarTarget::Save => {
                self.show(true, p);
                self.begin_save(p);
            }
        }
    }

    /// A save begun: the PRESET display asks for a name and tags, the preset's to begin with.
    fn begin_save(&mut self, p: &Ca72Params) {
        self.end_edit();
        let current = Self::current(p);
        let e = self.all.iter().find(|e| same(&e.sound.name, &current));
        self.drawer.save_name = Field::new(e.map_or("", |e| e.sound.name.as_str()));
        self.drawer.save_tags = Field::new(&e.map_or(String::new(), |e| e.sound.tags.join(", ")));
        self.drawer.editing = Some(Edit::Save);
        self.drawer.focus = Some(FieldId::SaveName);
        self.wants_keys = true;
        self.saving_says();
    }

    fn report(&mut self, r: Result<impl Sized, String>) {
        if let Err(e) = r {
            self.drawer.hint = e;
        }
    }

    /// What saving under the name typed does, said before.
    fn saving_says(&mut self) {
        let name = self.drawer.save_name.text.trim().to_owned();
        let there = (!name.is_empty())
            .then(|| self.all.iter().find(|e| same(&e.sound.name, &name)))
            .flatten();
        self.drawer.replace = there.is_some();
        self.drawer.hint = match there {
            None => String::new(),
            Some(e) if e.origin == Origin::Factory => {
                "Takes the factory's place (REVERT brings it back).".into()
            }
            Some(e) => format!("Replaces your {}.", e.sound.name),
        };
    }

    /// The current sound saved under the name and tags typed; the plug-in then that preset.
    fn save(&mut self, p: &Ca72Params, s: &ParamSetter<'_>) {
        let name = self.drawer.save_name.text.trim().to_owned();
        if name.is_empty() {
            self.drawer.focus = Some(FieldId::SaveName);
            return;
        }
        let tags = match library::parse_tags(&self.drawer.save_tags.text) {
            Ok(t) => t,
            Err(e) => {
                self.drawer.hint = e;
                return;
            }
        };
        if self.saving.is_some() {
            return;
        }
        if self.effects.is_some() {
            match self.start_save_sound(&name, tags, p) {
                Ok(saving) => {
                    self.saving = Some(saving);
                    self.drawer.hint = "Saving effects…".into();
                }
                Err(e) => self.drawer.hint = e,
            }
        } else {
            let result = self.save_sound(&name, tags, p);
            self.saved_says(result, p);
        }
        let _ = s;
    }

    fn saved_says(&mut self, result: Result<Entry, String>, p: &Ca72Params) {
        match result {
            Ok(e) => {
                self.drawer.hint = format!("Saved {}.", e.sound.name);
                self.drawer.save_name = Field::default();
                self.drawer.save_tags = Field::default();
                self.drawer.replace = false;
                self.end_edit();
                self.reread(p);
                self.bring_into_view(&e.sound.name);
            }
            Err(e) => self.drawer.hint = e,
        }
        self.reread(p);
    }

    pub(crate) fn start_save_sound(
        &self,
        name: &str,
        tags: Vec<String>,
        p: &Ca72Params,
    ) -> Result<Saving, String> {
        library::check_name(name)?;
        let mut queue = self.save_queue.lock().map_err(|e| e.to_string())?;
        if queue.is_none() {
            let (tx, rx) = std::sync::mpsc::channel::<SaveRequest>();
            std::thread::Builder::new()
                .name("ca72-preset-save".into())
                .spawn(move || {
                    for request in rx {
                        request.run();
                    }
                })
                .map_err(|e| e.to_string())?;
            *queue = Some(tx);
        }
        let before = Self::current(p);
        let (reply, result) = std::sync::mpsc::channel();
        let request = SaveRequest {
            library: self.library.clone(),
            name: name.to_owned(),
            values: values_now(p),
            tags,
            curve: (
                p.quality.value() == QualityMode::Lo,
                p.drive_curve.saved().db,
            ),
            names: names_now(p),
            effects: self.effects.as_ref().map(|e| e.prepare_save_preset()),
            carried: p
                .preset_effects
                .read()
                .map(|s| s.clone())
                .unwrap_or_default(),
            reply,
        };
        queue
            .as_ref()
            .ok_or("The preset’s save thread did not start")?
            .send(request)
            .map_err(|e| e.to_string())?;
        Ok(Saving {
            before,
            name: name.to_owned(),
            result,
        })
    }

    pub(crate) fn saving_as(&self, name: &str) -> bool {
        self.saving.as_ref().is_some_and(|s| same(&s.name, name))
    }

    pub(crate) fn finish_save(&mut self, saving: &Saving, e: &Entry, p: &Ca72Params) {
        // A save finishing after another preset was chosen never chooses the saved sound.
        if Self::current(p) == saving.before {
            self.saved_sound(e, p);
        } else {
            self.reread(p);
        }
    }

    fn saved_sound(&mut self, e: &Entry, p: &Ca72Params) {
        self.load_effects(p, e.sound.effects.clone(), false);
        Self::set_current(p, &e.sound.name);
        self.wanted = Some((e.sound.name.clone(), targets(p, e)));
        self.reread(p);
    }

    /// The current sound saved under `name` with `tags` (SAVE AS's, and an agent's:
    /// `crate::agent`), the plug-in then that preset; the library read again.
    pub fn save_sound(
        &mut self,
        name: &str,
        tags: Vec<String>,
        p: &Ca72Params,
    ) -> Result<Entry, String> {
        // (With AUTO GAIN's curve as it is heard: the preset plays at this level again.)
        let curve = (
            p.quality.value() == QualityMode::Lo,
            p.drive_curve.saved().db,
        );
        let effects = match &self.effects {
            Some(effects) => effects.save_preset()?,
            None => p
                .preset_effects
                .read()
                .map(|s| s.clone())
                .unwrap_or_default(),
        };
        let r = self.library.save_with(
            name,
            values_now(p),
            Some(tags),
            Some(curve),
            names_now(p),
            effects,
        );

        if let Ok(e) = &r {
            self.saved_sound(e, p);
        }
        self.reread(p);
        r
    }

    /// A preset renamed (RENAME's, and an agent's): the plug-in's preset is still it if it
    /// was; the library read again.
    pub fn rename_preset(&mut self, name: &str, to: &str, p: &Ca72Params) -> Result<Entry, String> {
        let r = self.library.rename(name, to);
        if let Ok(n) = &r
            && same(&Self::current(p), name)
        {
            Self::set_current(p, &n.sound.name);
        }
        self.reread(p);
        r
    }

    /// The preset the plug-in is set to, by name (empty when none).
    pub fn current_name(p: &Ca72Params) -> String {
        Self::current(p)
    }

    /// The presets as last read: every one, and the list as the drawer's filters have it.
    pub fn presets(&self) -> (&[Entry], &[Entry]) {
        (&self.all, &self.list)
    }

    /// Commits the preset being renamed or tagged, wherever its row is now; none if it has
    /// gone.
    fn commit_edit(&mut self, p: &Ca72Params) {
        let Some(how @ (Edit::Rename | Edit::Tags)) = self.drawer.editing else {
            return;
        };
        let Some(e) = self.edited_now(p) else {
            return;
        };
        let text = self.drawer.edit.text.clone();
        let mut shown = e.sound.name.clone();
        let r = match how {
            Edit::Rename => self.rename_preset(&e.sound.name, &text, p).map(|n| {
                shown = n.sound.name;
            }),
            _ => library::parse_tags(&text).and_then(|t| self.library.tag(&e.sound.name, t)),
        };
        match r {
            Ok(()) => {
                self.end_edit();
                self.drawer.focus = Some(FieldId::Search);
            }
            Err(err) => self.drawer.hint = err,
        }
        self.reread(p);
        self.bring_into_view(&shown);
    }

    /// A press in the drawer, at `x` across it (panel units: a field's caret goes there).
    pub fn drawer_press(&mut self, t: DrawerTarget, x: f64, p: &Ca72Params, s: &ParamSetter<'_>) {
        match t {
            DrawerTarget::Field(id) => {
                let caret = caret_at(&self.drawer, id, x);
                self.field_mut(id).caret = caret;
                self.drawer.focus = Some(id);
                self.wants_keys = true;
            }
            DrawerTarget::Tab(t) => {
                self.drawer.factory = t == Tab::Factory;
                (self.drawer.favourites, self.drawer.mine) = match t {
                    Tab::All | Tab::Factory => (false, false),
                    Tab::Favourites => (true, false),
                    Tab::Mine => (false, true),
                };
                self.drawer.first = 0;
                self.refilter(p);
            }
            DrawerTarget::Chip(i) => {
                if let Some(c) = self.drawer.chips.get_mut(i) {
                    c.1 = !c.1;
                }
                self.drawer.first = 0;
                self.refilter(p);
            }
            DrawerTarget::Row(i) => {
                if let Some(e) = self.list.get(i).cloned() {
                    self.end_edit();
                    self.choose(&e, p, s);
                }
                // A double click chooses it and closes the drawer.
                let now = Instant::now();
                let double = self
                    .last_row
                    .is_some_and(|(t, r)| r == i && now - t < DOUBLE_CLICK);
                self.last_row = Some((now, i));
                if double {
                    self.last_row = None;
                    self.show(false, p);
                }
            }
            DrawerTarget::Star(i) => {
                if let Some(e) = self.list.get(i).cloned() {
                    self.report(self.library.favorite(&e.sound.name, !e.favorite));
                    self.reread(p);
                }
            }
            DrawerTarget::Key(k) => self.key_press(k, p, s),
            // The update check is the editor's (`crate::update`), and so is the MIDI list
            // (`crate::learning`).
            DrawerTarget::Update
            | DrawerTarget::MidiRow(_)
            | DrawerTarget::MidiAction(..)
            | DrawerTarget::Back => {}
        }
    }

    /// A PRESET key: RENAME, TAGS, DELETE and REVERT for the preset the plug-in is set to
    /// (DELETE again, once it has asked, deletes it); SAVE AS (again, once begun, saves);
    /// INIT restores the fresh-instance sound; RESTORE; CLOSE. (MIDI LEARN is the editor's.)
    fn key_press(&mut self, k: PresetKey, p: &Ca72Params, s: &ParamSetter<'_>) {
        let current = Self::current(p);
        let entry = self
            .all
            .iter()
            .find(|e| same(&e.sound.name, &current))
            .cloned();
        match (k, entry) {
            (PresetKey::Rename | PresetKey::Tags | PresetKey::Delete | PresetKey::Revert, None) => {
                self.end_edit();
                self.drawer.hint = "Choose a preset first.".into();
            }
            (PresetKey::Rename, Some(e)) => {
                self.begin_edit(&e, Edit::Rename);
                self.drawer.edit = Field::new(&e.sound.name);
                self.drawer.focus = Some(FieldId::Edit);
                self.wants_keys = true;
            }
            (PresetKey::Tags, Some(e)) => {
                self.begin_edit(&e, Edit::Tags);
                self.drawer.edit = Field::new(&e.sound.tags.join(", "));
                self.drawer.focus = Some(FieldId::Edit);
                self.wants_keys = true;
            }
            (PresetKey::Delete, Some(e)) => {
                if matches!(self.drawer.editing, Some(Edit::Delete { .. })) {
                    self.confirm_delete(p);
                } else {
                    let factory = e.origin != Origin::User;
                    self.begin_edit(&e, Edit::Delete { factory });
                    self.drawer.focus = Some(FieldId::Search);
                }
            }
            (PresetKey::Revert, Some(e)) => {
                self.end_edit();
                if e.origin == Origin::Edited {
                    self.drawer.hint = match self.library.revert(&e.sound.name) {
                        Ok(_) => format!("{} is the factory's again.", e.sound.name),
                        Err(err) => err,
                    };
                } else {
                    self.drawer.hint = "Only a factory preset you changed can be reverted.".into();
                }
                self.reread(p);
            }
            (PresetKey::SaveAs, _) => {
                if self.drawer.editing == Some(Edit::Save) {
                    self.save(p, s);
                } else {
                    self.begin_save(p);
                }
            }
            (PresetKey::Restore, _) => {
                self.end_edit();
                match self.library.restore() {
                    Ok(r) if r.is_empty() => {
                        self.drawer.hint = "No factory preset was deleted.".into()
                    }
                    Ok(r) => self.drawer.hint = format!("Restored {}.", r.join(", ")),
                    Err(e) => self.drawer.hint = e,
                }
                self.reread(p);
            }
            (PresetKey::Init, _) => self.init(p, s),
            (PresetKey::Close, _) => self.show(false, p),
            (PresetKey::MidiLearn, _) => {}
        }
    }

    /// The preset DELETE asked about deleted, wherever it is now; none if it has gone.
    fn confirm_delete(&mut self, p: &Ca72Params) {
        if matches!(self.drawer.editing, Some(Edit::Delete { .. }))
            && let Some(e) = self.edited_now(p)
        {
            self.drawer.hint = match self.library.remove(&e.sound.name) {
                Ok(_) => format!("Deleted {}.", e.sound.name),
                Err(err) => err,
            };
        }
        self.end_edit();
        self.reread(p);
    }

    fn field_mut(&mut self, id: FieldId) -> &mut Field {
        match id {
            FieldId::Search => &mut self.drawer.search,
            FieldId::Edit => &mut self.drawer.edit,
            FieldId::SaveName => &mut self.drawer.save_name,
            FieldId::SaveTags => &mut self.drawer.save_tags,
        }
    }

    /// The list scrolled by `rows`.
    pub fn scroll(&mut self, rows: i32) {
        let most = self.drawer.rows.len().saturating_sub(ROWS_SHOWN) as i32;
        self.drawer.first = (self.drawer.first as i32 + rows).clamp(0, most) as usize;
    }

    /// The tags scrolled by `lines` (two tags a line).
    pub fn scroll_tags(&mut self, lines: i32) {
        let most = self
            .drawer
            .chips
            .len()
            .div_ceil(2)
            .saturating_sub(TAG_LINES) as i32;
        self.drawer.tags_first = (self.drawer.tags_first as i32 + lines).clamp(0, most) as usize;
    }

    /// A key: whether the drawer took it (else it is the host's).
    pub fn key(&mut self, e: &KeyboardEvent, p: &Ca72Params, s: &ParamSetter<'_>) -> bool {
        if !self.open {
            return false;
        }
        if e.state != KeyState::Down {
            return true;
        }
        // The host's shortcuts (Command, Control) stay the host's.
        if e.modifiers.intersects(Modifiers::META | Modifiers::CONTROL) {
            return false;
        }
        let focus = self.drawer.focus;
        match (&e.key, focus) {
            (Key::Escape, _) => {
                if self.drawer.editing.is_some() {
                    self.end_edit();
                    self.drawer.focus = Some(FieldId::Search);
                } else {
                    self.show(false, p);
                }
            }
            (Key::Enter, _) if matches!(self.drawer.editing, Some(Edit::Delete { .. })) => {
                self.confirm_delete(p);
            }
            (Key::ArrowDown | Key::ArrowUp, None | Some(FieldId::Search)) => {
                self.step(if e.key == Key::ArrowDown { 1 } else { -1 }, p, s);
            }
            (Key::Enter, None | Some(FieldId::Search)) => self.show(false, p),
            (Key::Enter, Some(FieldId::Edit)) => self.commit_edit(p),
            (Key::Enter, Some(FieldId::SaveName | FieldId::SaveTags)) => self.save(p, s),
            // The next field shown: the search, then what a key began's.
            (Key::Tab, _) => {
                self.drawer.focus = Some(match (self.drawer.editing, focus) {
                    (Some(Edit::Save), None | Some(FieldId::Search)) => FieldId::SaveName,
                    (Some(Edit::Save), Some(FieldId::SaveName)) => FieldId::SaveTags,
                    (Some(Edit::Rename | Edit::Tags), None | Some(FieldId::Search)) => {
                        FieldId::Edit
                    }
                    _ => FieldId::Search,
                });
            }
            (k, f) => {
                let id = f.unwrap_or(FieldId::Search);
                let field = self.field_mut(id);
                match k {
                    Key::Character(c) => field.insert(c),
                    Key::Backspace => field.backspace(),
                    Key::Delete => field.delete(),
                    Key::ArrowLeft => field.left(),
                    Key::ArrowRight => field.right(),
                    Key::Home => field.home(),
                    Key::End => field.end(),
                    _ => return false,
                }
                self.drawer.focus = Some(id);
                match id {
                    FieldId::Search => {
                        self.drawer.first = 0;
                        self.refilter(p);
                    }
                    FieldId::SaveName => self.saving_says(),
                    _ => {}
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nih_plug::context::process::TestProcessContext;
    use std::sync::{Arc, Mutex, atomic::Ordering};

    /// Applies editor gestures as the wrapper would, to test the resulting sound rather
    /// than just the requested values. The context owns the parameters behind each pointer.
    struct ApplyingHost {
        process: Mutex<TestProcessContext<crate::Ca72>>,
        gestures: Mutex<Vec<ParamPtr>>,
    }

    impl ApplyingHost {
        fn new(p: Arc<Ca72Params>) -> Self {
            Self {
                process: Mutex::new(TestProcessContext::new(p, 48_000.0, ProcessMode::Realtime)),
                gestures: Mutex::default(),
            }
        }

        fn automate(&self, ptr: ParamPtr, value: f32) {
            assert!(self.process.lock().unwrap().automate(ptr, value));
        }
    }

    #[allow(unsafe_code)]
    impl GuiContext for ApplyingHost {
        fn plugin_api(&self) -> PluginApi {
            PluginApi::Clap
        }
        fn request_resize(&self) -> bool {
            true
        }
        unsafe fn raw_begin_set_parameter(&self, ptr: ParamPtr) {
            self.gestures.lock().unwrap().push(ptr);
        }
        unsafe fn raw_set_parameter_normalized(&self, ptr: ParamPtr, value: f32) {
            self.automate(ptr, value);
        }
        unsafe fn raw_end_set_parameter(&self, ptr: ParamPtr) {
            assert_eq!(self.gestures.lock().unwrap().last(), Some(&ptr));
        }
        fn get_state(&self) -> PluginState {
            panic!("Init uses parameter gestures, not project state")
        }
        fn set_state(&self, _: PluginState) {
            panic!("Init must not replace project state")
        }
    }

    #[test]
    fn a_plugin_save_as_keeps_the_selected_presets_effects_verbatim() {
        let p = Arc::new(Ca72Params::default());
        let host = ApplyingHost::new(p.clone());
        let setter = ParamSetter::new(&host);
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::at(dir.path());
        let raw = "[effects] # opaque\nchain = 'a future version'\nextra = [42]\n";
        let e = lib
            .save_with(
                "Carried",
                Vec::new(),
                None,
                None,
                Vec::new(),
                Some(raw.into()),
            )
            .unwrap();
        let mut b = Browser::new(lib);
        b.effects = None;
        b.choose(&e, &p, &setter);
        let fields = p.serialize_fields();
        let reopened = Ca72Params::default();
        reopened.deserialize_fields(&fields);
        let saved = b.save_sound("Copy", Vec::new(), &reopened).unwrap();
        assert_eq!(saved.sound.effects.as_deref(), Some(raw));
        assert!(
            std::fs::read_to_string(dir.path().join("Copy.toml"))
                .unwrap()
                .ends_with(raw)
        );
        assert_eq!(crate::library::FORMAT, 1);
        for old in [None, Some("not JSON")] {
            *reopened.preset_effects.write().unwrap() = Some(raw.into());
            let mut state = PluginState {
                version: "0.2.8".into(),
                params: Default::default(),
                fields: Default::default(),
            };
            if let Some(old) = old {
                state.fields.insert("preset_effects".into(), old.into());
            }
            crate::Ca72::filter_state(&mut state);
            reopened.deserialize_fields(&state.fields);
            assert!(reopened.preset_effects.read().unwrap().is_none());
        }
    }

    #[test]
    fn effect_saves_wait_off_the_editor_keep_order_and_never_choose_over_a_new_preset() {
        let p = Ca72Params::default();
        let dir = tempfile::tempdir().unwrap();
        let mut b = Browser::new(Library::at(dir.path()));
        let fake = crate::fx_view::fake::Fake::with(&[], &[]);
        fake.delay_save
            .store(true, std::sync::atomic::Ordering::Relaxed);
        b.effects = Some(fake.clone());
        let first = b.start_save_sound("Copy", Vec::new(), &p).unwrap();
        let second = b.start_save_sound("Copy", Vec::new(), &p).unwrap();
        assert!(first.poll().is_none() && second.poll().is_none());
        b.tick(&p);
        Browser::set_current(&p, "Another preset");
        let mut replies = fake.save_replies.lock().unwrap();
        let raw = "[effects]\nchain = 'the later save'\n";
        replies.pop().unwrap().send(Ok(Some(raw.into()))).unwrap();
        assert!(
            second.poll().is_none(),
            "files wait in the order SAVE was pressed"
        );
        replies.pop().unwrap().send(Ok(None)).unwrap();
        drop(replies);
        for saving in [&first, &second] {
            let start = Instant::now();
            let e = loop {
                if let Some(result) = saving.poll() {
                    break result.unwrap();
                }
                assert!(start.elapsed() < Duration::from_secs(5));
                std::thread::sleep(Duration::from_millis(2));
            };
            b.finish_save(saving, &e, &p);
            assert_eq!(Browser::current(&p), "Another preset");
        }
        assert_eq!(
            b.library.find("Copy").unwrap().sound.effects.as_deref(),
            Some(raw)
        );
    }

    #[test]
    fn the_chain_marks_a_preset_changed_except_under_its_lock() {
        let p = Ca72Params::default();
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::at(dir.path());
        let mut b = Browser::new(lib);
        let e = b.library.find("Felt Bass").unwrap();
        Browser::set_current(&p, &e.sound.name);
        let fake = crate::fx_view::fake::Fake::with(&[], &[]);
        b.effects = Some(fake.clone());
        b.reread(&p);
        b.wanted = Some((
            e.sound.name.clone(),
            sound_params(&p)
                .into_iter()
                .map(|(id, q)| (id, q.normalized()))
                .collect(),
        ));
        b.tick(&p);
        assert!(!b.bar.changed);
        fake.preset.lock().unwrap().differs = true;
        b.tick(&p);
        assert!(b.bar.changed);
        fake.preset.lock().unwrap().locked = true;
        b.tick(&p);
        assert!(!b.bar.changed);
        assert!(b.bar.fx_locked);
    }

    /// A preset chosen, and Init, set the voices to their knobs at the plug-in's next block, not
    /// gliding; a knob turned in the editor glides there over 30 ms and lands on its value
    /// (decisions.md R-KNOB-GLIDE).
    #[test]
    fn a_chosen_preset_lands_in_the_voices_at_once_and_a_turned_knob_glides() {
        let mut plugin = crate::tests::initialised(false);
        let mut c = crate::tests::hostless(&plugin);
        let p = plugin.parameters().clone();
        let host = ApplyingHost::new(p.clone());
        let setter = ParamSetter::new(&host);
        let dir = tempfile::tempdir().unwrap();
        let mut b = Browser::new(Library::at(dir.path()));
        b.show(true, &p);
        crate::tests::block(&mut plugin, &mut c, 256);
        let landed = |plugin: &crate::Ca72, what: &str| {
            assert!(!plugin.smooth.moving(), "{what}");
            assert_eq!(plugin.controls(), p.controls(), "{what}");
        };
        for name in ["Brass Choir", "Felt Bass", "Brass Choir"] {
            let e = b.library.find(name).unwrap();
            b.choose(&e, &p, &setter);
            crate::tests::block(&mut plugin, &mut c, 64);
            landed(&plugin, name);
        }
        b.drawer_press(DrawerTarget::Key(PresetKey::Init), 0.0, &p, &setter);
        crate::tests::block(&mut plugin, &mut c, 64);
        landed(&plugin, "Init");
        // The editor's turn glides.
        p.cutoff.set_once(&setter, 0.9);
        crate::tests::block(&mut plugin, &mut c, 64);
        assert!(plugin.smooth.moving());
        let voices = plugin.controls().panel.cutoff;
        assert!(voices > 0.5 && voices < 0.9, "{voices}");
        for _ in 0..22 {
            crate::tests::block(&mut plugin, &mut c, 64);
        }
        landed(&plugin, "the turned knob, 30 ms on");
        assert_eq!(plugin.controls().panel.cutoff.to_bits(), 0.9f64.to_bits());
    }

    #[test]
    fn init_restores_every_sound_default_and_preserves_preferences() {
        use crate::learn::{Cc, index};

        let dir = tempfile::tempdir().unwrap();
        let p = Arc::new(Ca72Params::default());
        let host = ApplyingHost::new(p.clone());
        let setter = ParamSetter::new(&host);
        // Every hosted parameter is either a sound parameter or explicitly excluded.
        // A newly added parameter cannot silently fall outside Init's coverage.
        let sound = sound_params(&p);
        for (id, ptr, _) in p.param_map() {
            assert!(
                sound.iter().any(|(i, _)| *i == id) || library::KEPT.contains(&id.as_str()),
                "{id}"
            );
            // SAFETY: p owns every pointer in its parameter map throughout this test.
            #[allow(unsafe_code)]
            let default = unsafe { ptr.default_normalized_value() };
            host.automate(ptr, if default < 0.5 { 1.0 } else { 0.0 });
        }
        // Even a value extremely close to the default must return exactly to it.
        host.automate(p.tune.as_ptr(), p.tune.default_normalized_value() + 2e-7);
        p.editor_width.store(860, Ordering::Relaxed);
        p.editor_panel.store(false, Ordering::Relaxed);
        p.editor_expanded.store(false, Ordering::Relaxed);
        p.editor_panel_applied.store(false, Ordering::Relaxed);
        p.editor_presets.store(true, Ordering::Relaxed);
        p.freq_law.store(1, Ordering::Relaxed);
        p.noise_seed.store(12345, Ordering::Relaxed);
        p.midi_map
            .assign(index("cutoff").unwrap(), Cc { channel: 2, cc: 74 });
        let assignments = p.midi_map.saved();
        let kept: Vec<_> = p
            .param_map()
            .into_iter()
            .filter(|(id, _, _)| {
                library::KEPT.contains(&id.as_str()) || PLAYERS.contains(&id.as_str())
            })
            .map(|(id, ptr, _)| {
                // SAFETY: p keeps ptr alive throughout this test.
                #[allow(unsafe_code)]
                (id, unsafe { ptr.unmodulated_normalized_value() })
            })
            .collect();
        let mut b = Browser::new(Library::at(dir.path()));
        b.show(true, &p);
        Browser::set_current(&p, "Felt Bass");
        b.begin_save(&p);
        b.drawer_press(DrawerTarget::Key(PresetKey::Init), 0.0, &p, &setter);
        for (id, q) in sound_params(&p) {
            if !PLAYERS.contains(&id) {
                assert_eq!(q.normalized(), q.default_normalized(), "{id}");
            }
        }
        for (id, value) in kept {
            let (_, ptr, _) = p
                .param_map()
                .into_iter()
                .find(|(i, _, _)| *i == id)
                .unwrap();
            // SAFETY: p owns ptr and is still alive.
            #[allow(unsafe_code)]
            let actual = unsafe { ptr.unmodulated_normalized_value() };
            assert_eq!(actual, value, "{id}");
        }
        assert_eq!(p.midi_map.saved(), assignments);
        assert_eq!(p.editor_width.load(Ordering::Relaxed), 860);
        assert!(!p.editor_panel.load(Ordering::Relaxed));
        assert!(p.editor_presets.load(Ordering::Relaxed));
        assert!(!p.editor_expanded.load(Ordering::Relaxed));
        assert!(!p.editor_panel_applied.load(Ordering::Relaxed));
        assert_eq!(p.freq_law.load(Ordering::Relaxed), 1);
        assert_eq!(p.seed(), 12345);
        assert_eq!(Browser::current(&p), "");
        assert_eq!(b.drawer.editing, None);
        assert!(b.drawer.current.is_none());
        assert!(!b.bar.found && !b.bar.changed);
        let gestures = host.gestures.lock().unwrap().len();
        b.drawer_press(DrawerTarget::Key(PresetKey::Init), 0.0, &p, &setter);
        assert_eq!(
            host.gestures.lock().unwrap().len(),
            gestures,
            "repeated Init has no extra gestures"
        );
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            0,
            "Init writes no presets or preferences"
        );
    }

    #[test]
    fn init_saved_and_reloaded_matches_a_fresh_instances_state_and_audio() {
        use crate::engine::{Engine, Event};
        use crate::params::QualityMode;

        fn render(p: &Ca72Params, seed: u64) -> Vec<(u32, u32)> {
            let mut engine = Engine::new();
            engine.set(&p.controls());
            engine.prepare(48_000.0, seed);
            let mut out = Vec::new();
            for key in [36, 60, 72] {
                engine.event(Event::Note { key, on: true });
                for _ in 0..4096 {
                    let (l, r) = engine.tick_stereo(0.0);
                    assert!(l.is_finite() && r.is_finite());
                    out.push((l.to_bits(), r.to_bits()));
                }
                engine.event(Event::Note { key, on: false });
                for _ in 0..1024 {
                    let (l, r) = engine.tick_stereo(0.0);
                    assert!(l.is_finite() && r.is_finite());
                    out.push((l.to_bits(), r.to_bits()));
                }
            }
            assert!(out.iter().any(|&(l, _)| f32::from_bits(l).abs() > 0.001));
            out
        }

        for quality in [QualityMode::Lo, QualityMode::Hi, QualityMode::Ultra] {
            let dir = tempfile::tempdir().unwrap();
            let p = Arc::new(Ca72Params::default());
            let host = ApplyingHost::new(p.clone());
            let setter = ParamSetter::new(&host);
            host.automate(p.quality.as_ptr(), p.quality.preview_normalized(quality));
            let fresh_params = Arc::new(Ca72Params::default());
            let fresh_host = ApplyingHost::new(fresh_params.clone());
            fresh_host.automate(
                fresh_params.quality.as_ptr(),
                fresh_params.quality.preview_normalized(quality),
            );
            let fresh = fresh_params.controls();
            let before = values_now(&fresh_params);
            let expected = render(&fresh_params, 12345);
            let mut b = Browser::new(Library::at(dir.path()));
            b.show(true, &p);
            let factory = b.library.entries().0;
            let changed = b.library.find("Brass Choir").unwrap();
            b.choose(&changed, &p, &setter);
            assert_ne!(p.controls(), fresh);
            b.drawer_press(DrawerTarget::Key(PresetKey::Init), 0.0, &p, &setter);
            assert_eq!(values_now(&p), before);
            assert_eq!(p.controls(), fresh);
            assert_eq!(render(&p, 12345), expected, "Init audio at {quality:?}");
            b.key_press(PresetKey::SaveAs, &p, &setter);
            b.drawer.save_name = Field::new("From Init");
            b.key_press(PresetKey::SaveAs, &p, &setter);
            // Reopen the library so this checks the file, rather than the first browser's
            // in-memory entry, and alter the saved patch before keeping it.
            host.automate(p.cutoff.as_ptr(), 0.75);
            let patch = p.controls();
            let patch_values = values_now(&p);
            let patch_audio = render(&p, 12345);
            b.key_press(PresetKey::SaveAs, &p, &setter);
            b.drawer.save_name = Field::new("Edited from Init");
            b.key_press(PresetKey::SaveAs, &p, &setter);
            drop(b);
            let mut b = Browser::new(Library::at(dir.path()));
            b.show(true, &p);
            let saved = b.library.find("From Init").unwrap();
            let edited = b.library.find("Edited from Init").unwrap();
            b.choose(&changed, &p, &setter);
            b.choose(&saved, &p, &setter);
            assert_eq!(values_now(&p), before, "saved values at {quality:?}");
            assert_eq!(p.controls(), fresh, "saved and reloaded at {quality:?}");
            assert_eq!(
                render(&p, 12345),
                expected,
                "audio at equivalent seed, phase and {quality:?}"
            );
            b.choose(&edited, &p, &setter);
            assert_eq!(values_now(&p), patch_values);
            assert_eq!(p.controls(), patch);
            assert_eq!(render(&p, 12345), patch_audio);
            // Init is an action even when a user's patch is named Init.
            let named_init = b
                .library
                .save("Init", vec![("cutoff".into(), 4.0)], None)
                .unwrap();
            b.choose(&named_init, &p, &setter);
            b.drawer_press(DrawerTarget::Key(PresetKey::Init), 0.0, &p, &setter);
            assert_eq!(p.controls(), fresh);
            assert_eq!(b.library.find("Init").unwrap(), named_init);
            for entry in factory {
                assert_eq!(b.library.find(&entry.sound.name).unwrap(), entry);
            }
        }
    }

    /// A host that takes every change and keeps none.
    struct Host;

    #[allow(unsafe_code)]
    impl GuiContext for Host {
        fn plugin_api(&self) -> PluginApi {
            PluginApi::Clap
        }
        fn request_resize(&self) -> bool {
            true
        }
        unsafe fn raw_begin_set_parameter(&self, _: ParamPtr) {}
        unsafe fn raw_set_parameter_normalized(&self, _: ParamPtr, _: f32) {}
        unsafe fn raw_end_set_parameter(&self, _: ParamPtr) {}
        fn get_state(&self) -> PluginState {
            panic!("the browser does not read the state")
        }
        fn set_state(&self, _: PluginState) {}
    }

    fn down(key: Key) -> KeyboardEvent {
        KeyboardEvent {
            state: KeyState::Down,
            key,
            ..KeyboardEvent::default()
        }
    }

    fn typed(b: &mut Browser, text: &str, p: &Ca72Params) {
        let s = ParamSetter::new(&Host);
        for c in text.chars() {
            b.key(&down(Key::Character(c.to_string())), p, &s);
        }
    }

    fn rows(b: &Browser) -> Vec<&str> {
        b.drawer.rows.iter().map(|r| r.name.as_str()).collect()
    }

    #[test]
    fn factory_category_uses_provenance_and_preserves_filters_and_navigation() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::at(dir.path());
        lib.save("My Bass", vec![], None).unwrap();
        lib.save("Singing Drift", vec![], None).unwrap();
        lib.favorite("My Bass", true).unwrap();
        lib.favorite("Singing Drift", true).unwrap();
        lib.favorite("Felt Bass", true).unwrap();
        lib.tag("Felt Bass", vec!["study".into()]).unwrap();
        let p = Ca72Params::default();
        let setter = ParamSetter::new(&Host);
        let mut b = Browser::new(lib);
        b.show(true, &p);
        let all = b.list.clone();
        let select = |b: &mut Browser, tab| {
            b.drawer.first = 100;
            b.drawer_press(DrawerTarget::Tab(tab), 0.0, &p, &setter);
            assert_eq!(b.drawer.first, 0);
            assert_eq!(ca72_panel::presets::tab_of(&b.drawer), tab);
            let names: Vec<_> = b.list.iter().map(|e| e.sound.name.clone()).collect();
            assert_eq!(*p.midi_map.preset_list.lock().unwrap(), Some(names));
        };
        select(&mut b, Tab::Factory);
        assert!(!b.list.is_empty());
        assert!(b.list.iter().all(|e| e.origin == Origin::Factory));
        assert_eq!(b.list.len(), all.len() - 2);
        assert!(!rows(&b).contains(&"Singing Drift"));
        assert!(rows(&b).contains(&"Felt Bass"));
        Browser::set_current(&p, "My Bass");
        for _ in 0..=b.list.len() {
            b.step(1, &p, &setter);
            assert!(b.list.iter().any(|e| e.sound.name == Browser::current(&p)));
        }
        b.step(-1, &p, &setter);
        assert_eq!(Browser::current(&p), b.list.last().unwrap().sound.name);
        b.drawer.search = Field::new("felt");
        b.refilter(&p);
        assert_eq!(rows(&b), ["Felt Bass"]);
        b.drawer.chips = vec![("study".into(), true)];
        select(&mut b, Tab::Mine);
        assert!(b.list.is_empty());
        select(&mut b, Tab::Factory);
        assert_eq!(rows(&b), ["Felt Bass"]);
        b.drawer.search = Field::new("no such preset xyz");
        b.refilter(&p);
        assert!(b.list.is_empty() && b.drawer.rows.is_empty());
        assert_eq!(*p.midi_map.preset_list.lock().unwrap(), Some(vec![]));
        let current = Browser::current(&p);
        b.step(1, &p, &setter);
        assert_eq!(Browser::current(&p), current);
        b.drawer.search = Field::default();
        b.drawer.chips.clear();
        select(&mut b, Tab::Mine);
        assert_eq!(rows(&b), ["My Bass", "Singing Drift"]);
        select(&mut b, Tab::Favourites);
        assert_eq!(rows(&b), ["Felt Bass", "My Bass", "Singing Drift"]);
        b.drawer_press(DrawerTarget::Star(0), 0.0, &p, &setter);
        assert_eq!(rows(&b), ["My Bass", "Singing Drift"]);
        select(&mut b, Tab::Factory);
        assert!(rows(&b).contains(&"Felt Bass"));
        b.library.revert("Singing Drift").unwrap();
        b.reread(&p);
        assert!(rows(&b).contains(&"Singing Drift"));
        select(&mut b, Tab::All);
        assert_eq!(b.list.len(), all.len());
    }

    /// MIDI BEND RANGE is the player's: no factory preset names it, a preset saved does not,
    /// and choosing one that does not leaves it as it is; one that names it sets it
    /// (decisions.md R18).
    #[test]
    fn the_bend_range_stays_the_players() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::at(dir.path());
        let p = Ca72Params::default();
        let bend = |e: &Entry| {
            targets(&p, e)
                .into_iter()
                .find(|(k, _)| *k == "midi_bend_range")
                .map(|(_, v)| v)
        };
        for e in lib.entries().0 {
            assert_eq!(bend(&e), None, "{}", e.sound.name);
            assert_eq!(targets(&p, &e).len(), sound_params(&p).len() - 1);
        }
        assert!(values_now(&p).iter().all(|(k, _)| k != "midi_bend_range"));
        let named = lib
            .save("Wide Bend", vec![("midi_bend_range".into(), 12.0)], None)
            .unwrap();
        assert_eq!(
            bend(&named),
            Some(p.midi_bend_range.preview_normalized(12.0))
        );
    }

    /// MIDI BEND RANGE set as the standalone's PITCH wheel menu sets it (a gesture through the
    /// host) stays the player's through a factory preset chosen, the previous and next, and
    /// Init; the sound's Save names it not (decisions.md R18, R-BEND-RANGE).
    #[test]
    fn presets_and_init_leave_the_bend_range_set() {
        let p = Arc::new(Ca72Params::default());
        let host = ApplyingHost::new(p.clone());
        let setter = ParamSetter::new(&host);
        let dir = tempfile::tempdir().unwrap();
        let mut b = Browser::new(Library::at(dir.path()));
        b.show(true, &p);
        for range in [7.0, 0.0, 4.25, 2.0] {
            let q = &p.midi_bend_range;
            setter.begin_set_parameter(q);
            setter.set_parameter(q, range);
            setter.end_set_parameter(q);
            assert_eq!(q.value(), q.preview_plain(q.preview_normalized(range)));
            let set = q.value();
            for name in ["Brass Choir", "Felt Bass"] {
                let e = b.library.find(name).unwrap();
                b.choose(&e, &p, &setter);
                assert_eq!(q.value(), set, "{range}: {name}");
            }
            b.drawer_press(DrawerTarget::Key(PresetKey::Init), 0.0, &p, &setter);
            assert_eq!(q.value(), set, "{range}: Init");
            assert!(values_now(&p).iter().all(|(k, _)| k != "midi_bend_range"));
        }
    }

    /// A preset 0.3.0's beta saved names its LFOs' RISE (`lfoN_rise`): each is read as FADE at
    /// as many seconds, a fade in; a preset naming FADE has its own, whatever a RISE beside it
    /// says; one naming neither, FADE off; an assignment to RISE is FADE's at half the depth
    /// (decisions.md R-LFO-FADE). A preset saved now names FADE, not RISE.
    #[test]
    fn a_beta_preset_s_rise_is_read_as_a_fade_in() {
        let p = Ca72Params::default();
        let sound = |values: &[(&str, f64)]| crate::library::Sound {
            name: "Beta".into(),
            description: String::new(),
            tags: Vec::new(),
            values: values.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect(),
            drive_curve: None,
            drive_curve_lo: None,
            macro_names: Vec::new(),
            effects: None,
        };
        let fade = |s: &crate::library::Sound, n: usize| {
            targets_for_sound(&p, s)
                .into_iter()
                .find(|(k, _)| *k == crate::lfo::PARAM_IDS[n][crate::lfo::FADE_PART])
                .map(|(_, v)| p.lfo(n).fade.preview_plain(v))
        };
        let beta = sound(&[
            ("lfo1_rise", 0.9),
            ("lfo4_rise", 5.0714),
            ("lfo1_to_tune", 3.0),
            ("macro1_to_lfo2_rise", 40.0),
        ]);
        assert!((fade(&beta, 0).unwrap() - 0.9).abs() < 1e-5);
        assert!((fade(&beta, 3).unwrap() - 5.0714).abs() < 1e-5);
        assert_eq!(fade(&beta, 1), Some(0.0), "none named: off");
        assert_eq!(
            crate::lfo::fade_heard(fade(&beta, 0).unwrap()),
            f64::from(0.9f32),
            "heard as the file names it"
        );
        let both = sound(&[("lfo1_fade", -2.0), ("lfo1_rise", 0.9)]);
        assert!((fade(&both, 0).unwrap() + 2.0).abs() < 1e-5);
        p.lfo_patch.load_values(&beta.values);
        assert_eq!(
            p.lfo_patch
                .depth(crate::lfo::Source::Macro(0), crate::lfo::Target::LfoFade(1)),
            Some(0.2)
        );
        let saved = values_now(&p);
        assert!(saved.iter().any(|(k, _)| k == "lfo1_fade"));
        assert!(saved.iter().any(|(k, _)| k == "macro1_to_lfo2_fade"));
        assert!(saved.iter().all(|(k, _)| !k.ends_with("_rise")));
    }

    /// What the LFOs modulate, and their EDGES, are a preset's (decisions.md R-LFO): saved with
    /// it, chosen with it, none from a preset that names none, none after Init; the LFOs'
    /// parameters are a preset's as any sound parameter's.
    #[test]
    fn a_preset_keeps_what_the_lfos_modulate() {
        use crate::lfo::{Edges, Source, Target};
        let sorted = |p: &Ca72Params| {
            Source::all()
                .map(|x| {
                    let mut a: Vec<_> = p
                        .lfo_patch
                        .assignments(x)
                        .into_iter()
                        .flatten()
                        .map(|a| (a.target, a.depth))
                        .collect();
                    a.sort_by_key(|(t, _)| *t);
                    a
                })
                .collect::<Vec<_>>()
        };
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::at(dir.path());
        let p = Ca72Params::default();
        let patch = &p.lfo_patch;
        patch.assign(Source::Lfo(0), Target::Cutoff, 0.4).unwrap();
        patch
            .assign(Source::Lfo(0), Target::Osc2Frequency, -0.125)
            .unwrap();
        patch.assign(Source::Lfo(3), Target::Volume, -0.5).unwrap();
        // (The standalone's TEMPO a preset's too, an LFO's and a macro's: decisions.md R-LFO.)
        patch.assign(Source::Lfo(2), Target::Tempo, 0.3).unwrap();
        patch.assign(Source::Macro(1), Target::Tempo, -0.6).unwrap();
        patch.set_edges(1, Edges::Sharp);
        let before = sorted(&p);
        let values = values_now(&p);
        assert!(
            values
                .iter()
                .any(|(k, v)| k == "lfo3_to_tempo" && (*v - 30.0).abs() < 1e-3)
        );
        assert!(
            values
                .iter()
                .any(|(k, v)| k == "macro2_to_tempo" && (*v + 60.0).abs() < 1e-3)
        );
        assert!(
            values
                .iter()
                .any(|(k, v)| k == "lfo1_to_cutoff" && *v == 40.0)
        );
        assert!(values.iter().any(|(k, v)| k == "lfo2_edges" && *v == 1.0));
        assert!(values.iter().any(|(k, _)| k == "lfo4_rate"));
        lib.save("Wobble", values, None).unwrap();
        let s = ParamSetter::new(&Host);
        let mut b = Browser::new(Library::at(dir.path()));
        b.show(true, &p);
        b.choose(&lib.find("Felt Bass").unwrap(), &p, &s);
        assert!(
            Source::all().all(|x| patch.count(x) == 0),
            "a preset without them"
        );
        assert_eq!(patch.edges(1), Edges::Soft);
        b.choose(&lib.find("Wobble").unwrap(), &p, &s);
        assert_eq!(sorted(&p), before, "read back from its file");
        assert_eq!(patch.edges(1), Edges::Sharp);
        b.tick(&p);
        assert!(!b.bar.changed, "as the preset has it");
        patch.set_depth(Source::Lfo(3), Target::Volume, -0.4);
        b.tick(&p);
        assert!(b.bar.changed, "a depth changed marks the preset");
        patch.set_depth(Source::Lfo(3), Target::Volume, -0.5);
        b.tick(&p);
        assert!(!b.bar.changed, "and back");
        b.init(&p, &s);
        assert!(Source::all().all(|x| patch.count(x) == 0), "Init");
        let felt = lib.find("Felt Bass").unwrap();
        let rate = targets(&p, &felt)
            .into_iter()
            .find(|(k, _)| *k == "lfo3_rate")
            .map(|(_, v)| v);
        assert_eq!(rate, Some(p.lfo3.rate.default_normalized_value()));
        // Each LFO's POLARITY, REVERSE and ON (R-LFO's corrections) are a preset's, and one
        // saved before them reads BI, forwards and ON.
        for n in 1..=crate::lfo::LFOS {
            // (And its LEVEL, R-LFO's addendum: 100 % in one saved before it.)
            for part in ["polarity", "reverse", "on", "level"] {
                let id = format!("lfo{n}_{part}");
                assert!(values_now(&p).iter().any(|(k, _)| *k == id), "{id} saved");
                let old = targets(&p, &felt)
                    .into_iter()
                    .find(|(k, _)| **k == id)
                    .map(|(_, v)| v);
                let at = if matches!(part, "on" | "level") {
                    1.0
                } else {
                    0.0
                };
                assert_eq!(old, Some(at), "{id}");
            }
        }
    }

    /// The MIDI assignments (decisions.md R34) are the instance's, not a sound's: choosing,
    /// saving, replacing and reverting a preset leave them as they are, and no preset file holds
    /// them.
    #[test]
    fn presets_leave_the_midi_assignments_alone() {
        use crate::learn::{Cc, index};
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::at(dir.path());
        let p = Ca72Params::default();
        let map = &p.midi_map;
        map.assign(index("cutoff").unwrap(), Cc { channel: 0, cc: 74 });
        map.assign(index("voices").unwrap(), Cc { channel: 9, cc: 20 });
        let before = map.saved();
        let s = ParamSetter::new(&Host);
        let mut b = Browser::new(Library::at(dir.path()));
        b.show(true, &p);
        let bass = lib.find("Felt Bass").unwrap();
        b.choose(&bass, &p, &s);
        b.step(1, &p, &s);
        assert_eq!(map.saved(), before, "chosen");
        // SAVE AS begins a save; pressed again, it saves.
        let save_as = |b: &mut Browser, name: &str| {
            b.drawer_press(DrawerTarget::Key(PresetKey::SaveAs), 0.0, &p, &s);
            b.drawer.save_name = Field::new(name);
            b.drawer_press(DrawerTarget::Key(PresetKey::SaveAs), 0.0, &p, &s);
        };
        save_as(&mut b, "Mine");
        save_as(&mut b, "Singing Drift");
        assert_eq!(lib.find("Singing Drift").unwrap().origin, Origin::Edited);
        assert_eq!(map.saved(), before, "saved");
        for f in std::fs::read_dir(dir.path()).unwrap() {
            let text = std::fs::read_to_string(f.unwrap().path()).unwrap();
            assert!(
                !text.contains("midi_map") && !text.contains("assignments"),
                "{text}"
            );
        }
        // REVERT, of the preset set (the one just saved).
        b.drawer_press(DrawerTarget::Key(PresetKey::Revert), 0.0, &p, &s);
        assert_eq!(lib.find("Singing Drift").unwrap().origin, Origin::Factory);
        assert_eq!(map.saved(), before, "reverted");
        assert!(values_now(&p).iter().all(|(k, _)| !k.contains("midi_map")));
    }

    /// A session whose preset has since left the factory's (decisions.md R55) keeps its
    /// sound: it holds every value itself, and its preset's name is the bar's alone, shown as
    /// not found and never as changed. Nothing is set from the name (`tick` has no setter).
    #[test]
    fn a_session_whose_preset_was_retired_keeps_its_sound() {
        let dir = tempfile::tempdir().unwrap();
        let p = Ca72Params::default();
        Browser::set_current(&p, "Retired Pluck");
        let before = values_now(&p);
        let mut b = Browser::new(Library::at(dir.path()));
        b.tick(&p);
        assert_eq!(
            (b.bar.name.as_str(), b.bar.found, b.bar.changed),
            ("Retired Pluck", false, false)
        );
        b.show(true, &p);
        b.tick(&p);
        assert!(b.drawer.current.is_none());
        assert_eq!(values_now(&p), before);
        assert_eq!(Browser::current(&p), "Retired Pluck");
    }

    /// RENAME, TAGS and DELETE act on the preset the plug-in is set to; what they begin stays
    /// with it as the list changes (a filter, another program's change), and ends when it has
    /// gone or another preset is set: another preset is never renamed, tagged or deleted in its
    /// place (decisions.md R18).
    #[test]
    fn an_edit_stays_with_its_preset() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::at(dir.path());
        for n in ["Pad A", "Pad B"] {
            lib.save(n, vec![], None).unwrap();
        }
        lib.favorite("Pad A", true).unwrap();
        lib.favorite("Warp Field", true).unwrap();
        let p = Ca72Params::default();
        let s = ParamSetter::new(&Host);
        let mut b = Browser::new(Library::at(dir.path()));
        let key = |b: &mut Browser, k: PresetKey| {
            b.drawer_press(DrawerTarget::Key(k), 0.0, &p, &s);
        };
        // (A row set, as a single click: not taken with the last for a double click.)
        let set = |b: &mut Browser, name: &str| {
            let i = rows(b).iter().position(|r| *r == name).expect("listed");
            b.last_row = None;
            b.drawer_press(DrawerTarget::Row(i), 0.0, &p, &s);
        };
        b.show(true, &p);
        // With no preset set, the keys that act on one say so.
        key(&mut b, PresetKey::Rename);
        assert_eq!(b.drawer.editing, None);
        assert_eq!(b.drawer.hint, "Choose a preset first.");
        typed(&mut b, "pad", &p);
        // (With the factory pads, R-PRESETS-0.3.0 and R-OWNER-PRESETS.)
        let pads = [
            "Finger in a Light Socket",
            "Gatekeeper",
            "Level Pump",
            "Morph Bloom",
        ];
        let with_pads = |mine: &[&str]| {
            let mut all: Vec<String> = pads.iter().chain(mine).map(|n| n.to_string()).collect();
            all.sort_by_key(|n| n.to_lowercase());
            all
        };
        assert_eq!(rows(&b), with_pads(&["Pad A", "Pad B", "Warp Field"]));

        // Renaming Pad B, FAVORITES leaves it out of the list: the rename stays with it.
        set(&mut b, "Pad B");
        key(&mut b, PresetKey::Rename);
        assert_eq!(b.drawer.editing, Some(Edit::Rename));
        assert_eq!(b.drawer.focus, Some(FieldId::Edit));
        b.drawer.edit = Field::new("Pad Renamed");
        b.drawer_press(DrawerTarget::Tab(Tab::Favourites), 0.0, &p, &s);
        assert_eq!(rows(&b), ["Pad A", "Warp Field"]);
        assert_eq!(b.drawer.editing, Some(Edit::Rename));
        b.key(&down(Key::Enter), &p, &s);
        assert!(lib.find("Pad B").is_none() && lib.find("Pad Renamed").is_some());
        assert_eq!(Browser::current(&p), "Pad Renamed");
        assert_eq!(b.drawer.editing, None);
        b.drawer_press(DrawerTarget::Tab(Tab::All), 0.0, &p, &s);

        // Tagging it, a preset arriving from elsewhere: the tags go to it.
        key(&mut b, PresetKey::Tags);
        b.drawer.edit = Field::new("soft");
        lib.save("Pad 0", vec![], None).unwrap();
        b.reread(&p);
        assert_eq!(
            rows(&b),
            with_pads(&["Pad 0", "Pad A", "Pad Renamed", "Warp Field"])
        );
        b.key(&down(Key::Enter), &p, &s);
        assert_eq!(lib.find("Pad Renamed").unwrap().sound.tags, ["soft"]);
        assert!(lib.find("Pad A").unwrap().sound.tags.is_empty());

        // DELETE asked about it, another preset set: the question ends, and Enter deletes
        // nothing; asked of Pad A, DELETE again deletes Pad A.
        key(&mut b, PresetKey::Delete);
        assert_eq!(b.drawer.editing, Some(Edit::Delete { factory: false }));
        set(&mut b, "Pad A");
        assert_eq!(b.drawer.editing, None);
        b.key(&down(Key::Enter), &p, &s);
        assert!(lib.find("Pad Renamed").is_some());
        b.show(true, &p);
        key(&mut b, PresetKey::Delete);
        key(&mut b, PresetKey::Delete);
        assert!(lib.find("Pad A").is_none());
        assert_eq!(b.drawer.hint, "Deleted Pad A.");

        // RENAME of a preset deleted elsewhere since the list was read: said, nothing done.
        set(&mut b, "Pad Renamed");
        key(&mut b, PresetKey::Rename);
        b.drawer.edit = Field::new("Pad Again");
        lib.remove("Pad Renamed").unwrap();
        b.key(&down(Key::Enter), &p, &s);
        assert_eq!(b.drawer.hint, "Pad Renamed is no longer there.");
        assert_eq!(b.drawer.editing, None);
        assert!(lib.find("Pad Again").is_none() && lib.find("Warp Field").is_some());

        // REVERT of a preset that is not a changed factory one: said, nothing done.
        set(&mut b, "Warp Field");
        key(&mut b, PresetKey::Revert);
        assert_eq!(
            b.drawer.hint,
            "Only a factory preset you changed can be reverted."
        );
        // Escape takes back what a key began; again, it closes the drawer.
        key(&mut b, PresetKey::Tags);
        b.key(&down(Key::Escape), &p, &s);
        assert_eq!((b.drawer.editing, b.open), (None, true));
        b.key(&down(Key::Escape), &p, &s);
        assert!(!b.open);
    }

    /// Typing in the search and the filters work on the presets as read: no file is read for
    /// them (decisions.md R18).
    #[test]
    fn the_search_works_on_the_presets_as_read() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::at(dir.path());
        lib.save("Zed Pad", vec![], Some(vec!["zed".into()]))
            .unwrap();
        let p = Ca72Params::default();
        let mut b = Browser::new(Library::at(dir.path()));
        b.show(true, &p);
        std::fs::remove_file(dir.path().join("Zed Pad.toml")).unwrap();
        typed(&mut b, "zed", &p);
        assert_eq!(rows(&b), ["Zed Pad"]);
        assert!(b.drawer.chips.iter().any(|(t, _)| t == "zed"));
        b.reread(&p);
        assert!(rows(&b).is_empty());
    }

    /// GLIDE RANGE is saved with the sound; every older preset chooses NORMAL, even if the
    /// instance was in LONG. The preset format stays the same.
    #[test]
    fn glide_range_is_a_presets_and_normal_where_it_does_not_name_it() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::at(dir.path());
        let p = std::sync::Arc::new(Ca72Params::default());
        let context = nih_plug::context::process::TestProcessContext::<crate::Ca72>::new(
            p.clone(),
            48_000.0,
            ProcessMode::Realtime,
        );
        assert!(context.automate(p.glide_range.as_ptr(), 1.0));
        let value = |e: &Entry| {
            targets(&p, e)
                .into_iter()
                .find(|(id, _)| *id == "glide_range")
                .map(|(_, v)| v)
        };
        assert!(values_now(&p).contains(&("glide_range".to_owned(), 1.0)));
        for e in lib.entries().0 {
            assert_eq!(value(&e), Some(0.0), "{}", e.sound.name);
        }
        for (name, range) in [("Long Slide", 1.0), ("Normal Slide", 0.0)] {
            lib.save(name, vec![("glide_range".to_owned(), range)], None)
                .unwrap();
            let reopened = Library::at(dir.path());
            let saved = reopened
                .entries()
                .0
                .into_iter()
                .find(|e| e.sound.name == name)
                .expect("the saved preset");
            assert_eq!(value(&saved), Some(range as f32));
        }
    }

    /// The oscillators timed (R-OSC-TIMED) are part of a preset: saved with the sound, set by a
    /// preset naming them, and off (DIVISION a quarter note) in one that does not, every
    /// factory preset that does not among them; FREQUENCY keeps its own place whichever.
    #[test]
    fn oscillator_3_timed_is_part_of_a_preset_and_off_in_one_that_does_not_name_it() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::at(dir.path());
        let p = Ca72Params::default();
        let target = |e: &Entry, id: &str| {
            targets(&p, e)
                .into_iter()
                .find(|(k, _)| *k == id)
                .map(|(_, v)| v)
        };
        for n in 2..=3 {
            for id in [format!("osc{n}_timed"), format!("osc{n}_division")] {
                assert!(values_now(&p).iter().any(|(k, _)| *k == id), "{id}");
            }
        }
        let names = |e: &Entry, id: &str| e.sound.values.iter().any(|(k, _)| k == id);
        for e in lib.entries().0 {
            for n in 2..=3 {
                let (on, division) = (format!("osc{n}_timed"), format!("osc{n}_division"));
                if names(&e, &on) || names(&e, &division) {
                    continue;
                }
                assert_eq!(target(&e, &on), Some(0.0), "{}", e.sound.name);
                assert_eq!(
                    target(&e, &division),
                    Some(p.osc3_division.default_normalized_value()),
                    "{}",
                    e.sound.name
                );
            }
        }
        let on = lib
            .save(
                "Timed Wobble",
                vec![
                    ("osc3_range".to_owned(), 0.0),
                    ("osc3_frequency".to_owned(), 2.5),
                    ("osc3_timed".to_owned(), 1.0),
                    ("osc3_division".to_owned(), 13.0),
                ],
                None,
            )
            .unwrap();
        assert_eq!(target(&on, "osc3_timed"), Some(1.0));
        assert_eq!(
            target(&on, "osc3_division"),
            Some(
                p.osc3_division
                    .preview_normalized(crate::params::Division::EighthTriplet)
            )
        );
        assert_eq!(
            target(&on, "osc3_frequency"),
            Some(p.osc3_frequency.preview_normalized(2.5))
        );
    }

    /// NOTE PRIORITY and RETRIGGER are a preset's (the owner, 2026-10-10: "This item should be
    /// saved per patch"; R50): saved with the sound, set by a preset naming them (the factory's
    /// the owner made since), and LOW and off, the instrument's own, by one saved before them.
    /// Each note's own expression's reaches (decisions.md R-MPE) are a preset's: saved with the
    /// sound and set by a preset naming them, and none (0, the sound as before) in one that
    /// does not, every factory preset among them.
    #[test]
    fn the_expression_reaches_are_a_presets_and_none_where_it_does_not_name_them() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::at(dir.path());
        let p = Ca72Params::default();
        let target = |e: &Entry, id: &str| {
            targets(&p, e)
                .into_iter()
                .find(|(k, _)| *k == id)
                .map(|(_, v)| v)
        };
        let ids = [
            "pressure_mod",
            "pressure_filter",
            "timbre_mod",
            "timbre_filter",
        ];
        for id in ids {
            assert!(values_now(&p).iter().any(|(k, _)| k == id), "{id}");
        }
        for e in lib.entries().0 {
            // (Those that name them, as Pressure Solo does, set them: below.)
            for id in ids
                .iter()
                .filter(|id| !e.sound.values.iter().any(|(k, _)| k == *id))
            {
                assert_eq!(target(&e, id), Some(0.5), "{} {id}", e.sound.name);
            }
        }
        let saved = lib
            .save(
                "Pressed Open",
                vec![
                    ("pressure_filter".to_owned(), 2.0),
                    ("timbre_mod".to_owned(), -50.0),
                ],
                None,
            )
            .unwrap();
        assert_eq!(target(&saved, "pressure_filter"), Some(0.75));
        assert_eq!(target(&saved, "timbre_mod"), Some(0.25));
        assert_eq!(target(&saved, "pressure_mod"), Some(0.5));
    }

    /// The expression's reaches (R-MPE) and the oscillators timed (R-OSC-TIMED), both after
    /// every other parameter, are a preset's side by side: one naming both sets both.
    #[test]
    fn a_preset_names_the_reaches_and_the_timed_oscillators_together() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::at(dir.path());
        let p = Ca72Params::default();
        let target = |e: &Entry, id: &str| {
            targets(&p, e)
                .into_iter()
                .find(|(k, _)| *k == id)
                .map(|(_, v)| v)
        };
        let saved = lib
            .save(
                "Pressed in Time",
                vec![
                    ("pressure_filter".to_owned(), 2.0),
                    ("timbre_mod".to_owned(), -50.0),
                    ("osc2_timed".to_owned(), 1.0),
                    ("osc2_division".to_owned(), 9.0),
                    ("osc3_timed".to_owned(), 1.0),
                    ("osc3_division".to_owned(), 13.0),
                ],
                None,
            )
            .unwrap();
        assert_eq!(target(&saved, "pressure_filter"), Some(0.75));
        assert_eq!(target(&saved, "timbre_mod"), Some(0.25));
        assert_eq!(target(&saved, "pressure_mod"), Some(0.5));
        assert_eq!(target(&saved, "osc2_timed"), Some(1.0));
        assert_eq!(target(&saved, "osc3_timed"), Some(1.0));
        assert_eq!(
            target(&saved, "osc2_division"),
            Some(
                p.osc2_division
                    .preview_normalized(crate::params::Division::EighthDotted)
            )
        );
        assert_eq!(
            target(&saved, "osc3_division"),
            Some(
                p.osc3_division
                    .preview_normalized(crate::params::Division::EighthTriplet)
            )
        );
    }

    #[test]
    fn the_arpeggiator_is_part_of_a_preset_and_off_in_one_that_does_not_name_it() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::at(dir.path());
        let p = Ca72Params::default();
        let target = |e: &Entry, id: &str| {
            targets(&p, e)
                .into_iter()
                .find(|(k, _)| *k == id)
                .map(|(_, v)| v)
        };
        for id in [
            "arp_on",
            "arp_latch",
            "arp_direction",
            "arp_rate",
            "arp_octaves",
            "arp_gate",
            "arp_swing",
        ] {
            assert!(values_now(&p).iter().any(|(k, _)| k == id), "{id}");
        }
        // Every factory preset that does not name it: off, its settings their defaults.
        for e in lib
            .entries()
            .0
            .into_iter()
            .filter(|e| !e.sound.values.iter().any(|(k, _)| k == "arp_on"))
        {
            assert_eq!(target(&e, "arp_on"), Some(0.0), "{}", e.sound.name);
            assert_eq!(
                target(&e, "arp_rate"),
                Some(p.arp_rate.default_normalized_value()),
                "{}",
                e.sound.name
            );
        }
        let on = lib
            .save(
                "Arpeggio",
                vec![
                    ("arp_on".to_owned(), 1.0),
                    ("arp_rate".to_owned(), 6.0),
                    ("arp_gate".to_owned(), 25.0),
                ],
                None,
            )
            .unwrap();
        assert_eq!(target(&on, "arp_on"), Some(1.0));
        assert_eq!(
            target(&on, "arp_rate"),
            Some(
                p.arp_rate
                    .preview_normalized(crate::params::ArpRate::Eighth)
            )
        );
        assert_eq!(target(&on, "arp_gate"), Some(0.25));
        // Saved plain, as a session is: OCTAVES 4 and RANDOM, saved before OCTAVES went to 5 and
        // ORDER came, are OCTAVES 4 and RANDOM still (R-ARP).
        let before = lib
            .save(
                "Four Up Random",
                vec![
                    ("arp_octaves".to_owned(), 4.0),
                    ("arp_direction".to_owned(), 3.0),
                ],
                None,
            )
            .unwrap();
        let set = |id: &str| target(&before, id).unwrap();
        assert_eq!(p.arp_octaves.preview_plain(set("arp_octaves")), 4);
        assert_eq!(
            p.arp_direction.preview_plain(set("arp_direction")),
            crate::params::ArpDirection::Random
        );
    }

    /// SAVE AS (and an agent's save: `save_sound`) keeps the arpeggiator's settings (R-ARP) and
    /// AUTO GAIN's curve in force (R-AUTO-GAIN-DRIVE) together: read back, each as saved.
    #[test]
    fn a_saved_sound_keeps_its_arpeggiator_and_auto_gains_curve() {
        let dir = tempfile::tempdir().unwrap();
        let p = Ca72Params {
            arp_on: nih_plug::prelude::BoolParam::new("Arpeggiator", true),
            ..Ca72Params::default()
        };
        let db = [-5.0_f32, -9.0, -12.0, -14.0];
        p.drive_curve.load_preset(db);
        let lo = p.quality.value() == QualityMode::Lo;
        let mut b = Browser::new(Library::at(dir.path()));
        let e = b.save_sound("Arp And Curve", Vec::new(), &p).unwrap();
        assert_eq!(e.sound.curve(lo), Some(db));
        let read = Library::at(dir.path()).find("Arp And Curve").unwrap();
        assert_eq!(read.sound.curve(lo), Some(db));
        assert_eq!(
            targets(&p, &read)
                .into_iter()
                .find(|(k, _)| *k == "arp_on")
                .map(|(_, v)| v),
            Some(1.0)
        );
    }

    #[test]
    fn note_priority_and_retrigger_are_part_of_a_preset() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::at(dir.path());
        let p = Ca72Params::default();
        let target = |e: &Entry| {
            targets(&p, e)
                .into_iter()
                .find(|(k, _)| *k == "note_priority")
                .map(|(_, v)| v)
        };
        assert!(values_now(&p).iter().any(|(k, _)| k == "note_priority"));
        let named = |e: &Entry, id: &str| {
            e.sound
                .values
                .iter()
                .find(|(k, _)| k == id)
                .map(|(_, v)| *v)
        };
        for e in lib.entries().0 {
            let v = named(&e, "note_priority").map_or(0.0, |v| {
                let i = crate::params::NotePriority::from_index(v as usize);
                p.note_priority.preview_normalized(i)
            });
            assert_eq!(target(&e), Some(v), "{}", e.sound.name);
        }
        let last = lib
            .save("Lead", vec![("note_priority".to_owned(), 1.0)], None)
            .unwrap();
        assert_eq!(target(&last), Some(0.5));
        let high = lib
            .save("Lead High", vec![("note_priority".to_owned(), 2.0)], None)
            .unwrap();
        assert_eq!(target(&high), Some(1.0));
        let retrigger = |e: &Entry| {
            targets(&p, e)
                .into_iter()
                .find(|(k, _)| *k == "retrigger")
                .map(|(_, v)| v)
        };
        assert!(values_now(&p).iter().any(|(k, _)| k == "retrigger"));
        for e in lib.entries().0 {
            let v = named(&e, "retrigger").map_or(0.0, |v| if v >= 0.5 { 1.0 } else { 0.0 });
            assert_eq!(retrigger(&e), Some(v), "{}", e.sound.name);
        }
        let on = lib
            .save("Lead Retrig", vec![("retrigger".to_owned(), 1.0)], None)
            .unwrap();
        assert_eq!(retrigger(&on), Some(1.0));
    }

    /// A preset sets POLY, VOICES, ENTROPY and SPREAD like any other parameter (decisions.md
    /// R12): the factory's ensemble presets turn POLY on with their voices, ENTROPY and
    /// SPREAD, the others turn it off; a preset that does not name them sets their
    /// defaults.
    #[test]
    fn poly_voices_entropy_and_spread_are_part_of_a_preset() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::at(dir.path());
        let p = Ca72Params::default();
        let target = |e: &Entry, id: &str| {
            targets(&p, e)
                .into_iter()
                .find(|(k, _)| *k == id)
                .map(|(_, v)| v)
        };
        let brass = lib.find("Brass Choir").unwrap();
        assert_eq!(target(&brass, "poly"), Some(1.0));
        assert_eq!(
            target(&brass, "voices"),
            Some(p.voices.preview_normalized(8))
        );
        assert_eq!(
            target(&brass, "entropy"),
            Some(p.entropy.preview_normalized(43.0))
        );
        assert_eq!(
            target(&brass, "spread"),
            Some(p.spread.preview_normalized(50.0))
        );
        let bass = lib.find("Felt Bass").unwrap();
        assert_eq!(target(&bass, "poly"), Some(0.0));
        assert_eq!(
            target(&bass, "entropy"),
            Some(p.entropy.preview_normalized(5.0))
        );
        // Every factory preset names all four.
        for e in lib.entries().0 {
            for id in ["poly", "voices", "entropy", "spread"] {
                assert!(
                    e.sound.values.iter().any(|(k, _)| k == id),
                    "{}: no {id}",
                    e.sound.name
                );
            }
        }
        // A preset naming none of them: their defaults (POLY off).
        let saved = lib
            .save("Plain", vec![("cutoff".to_owned(), 1.0)], None)
            .unwrap();
        assert_eq!(
            target(&saved, "poly"),
            Some(p.poly.default_normalized_value())
        );
        assert_eq!(
            target(&saved, "voices"),
            Some(p.voices.default_normalized_value())
        );
    }
}
