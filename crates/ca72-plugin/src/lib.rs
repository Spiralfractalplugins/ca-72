//! The CA-72 as a VST3 and CLAP plug-in: the circuit model's voice in its real-time quality,
//! played by MIDI, its EXTERNAL INPUT the side chain; one voice, or with POLY one per note
//! (up to ten), placed across the stereo output by SPREAD.

pub mod agent;
pub mod arp;
mod audio_thread;
pub mod character;
pub mod clipboard;
pub mod compat;
pub mod drive;
pub mod editor;
pub mod engine;
pub mod expression;
#[cfg(feature = "fx-host")]
pub mod fx;
pub mod fx_view;
pub mod helper;
pub mod learn;
pub mod learning;
pub mod lfo;
pub mod library;
pub mod local_notes;
pub mod mod_wheel;
pub mod offline;
pub mod opening;
pub mod params;
pub mod playing;
pub mod pool;
mod preset_actions;
pub mod presets;
mod resizing;
pub mod seats;
pub mod settings;
pub mod smooth;
#[cfg(feature = "standalone-production")]
pub mod standalone_error;
#[cfg(feature = "standalone-production")]
pub mod standalone_settings;
pub mod timed;
pub mod update;
pub mod velocity;
pub mod whats_new;

use std::sync::Arc;

use nih_plug::prelude::*;
use nih_plug::wrapper::state::ParamValue;

use crate::arp::Arp;
use crate::editor::{Ca72Editor, Meters};
use crate::engine::{
    ALL_NOTES_OFF, ALL_SOUND_OFF, Engine, Event, MODULATION_WHEEL, RESET_CONTROLLERS,
};
use crate::helper::{Helper, MEND, SERVE};
use crate::learn::Incoming;
use crate::params::Ca72Params;
use crate::smooth::Smooth;

/// Names the standalone application before its first window opens: on Linux, the X11 class of
/// every top-level window, the editor's and the error window's alike, whatever each one's
/// title. The installed launcher's `StartupWMClass` matches it (decisions.md R-LINUX-ICON).
#[cfg(feature = "standalone-production")]
pub fn identify_standalone() {
    baseview::set_application_class(Ca72::NAME);
}

pub struct Ca72 {
    params: Arc<Ca72Params>,
    // Keep the engine out of the wrapper's repeated by-value construction frames. The
    // Windows debug build must also fit a host's 256 KB thread (R-HOST-STACK).
    engine: Box<Engine>,
    meters: Arc<Meters>,
    local_notes: Arc<local_notes::LocalNotes>,
    note_sources: local_notes::Sources,
    local_generation: u64,
    panic_generation: u64,
    deferred_local: Option<local_notes::LocalEvent>,
    /// The host's sample rate (for each block's deadline).
    rate: f64,
    /// The plug-in's own thread for what the audio thread asks of it (decisions.md R23),
    /// started when the plug-in is first initialised (none if the system would not start it).
    helper: Option<Helper>,
    /// Where the voices have each continuous knob, gliding to its parameter's new value
    /// (decisions.md R34, R-KNOB-GLIDE).
    smooth: Smooth,
    /// AUTO GAIN's curve's writes as the audio thread last read them (`drive.rs`).
    curve_seen: u64,
    preset_actions: Arc<preset_actions::PresetActions>,
    preset_held: [bool; 4096],
    preset_notes: [bool; 2048],
    wheel: mod_wheel::Wheel,
    /// Each note's own expression, and the pressure the wheel reads (decisions.md R-MPE).
    expressions: Box<expression::Expressions>,
    /// The standalone application's effects after the synth (`fx.rs`, docs/fx-host.md):
    /// none in the plug-ins.
    #[cfg(feature = "fx-host")]
    fx: Option<ca72_fx::Runner>,
    /// The arpeggiator (decisions.md R-ARP): every key the note sources pass goes through it, to
    /// the engine as played while it is off; and the sample of the block the events are at.
    arp: Arp,
    /// Oscillator 3 timed's bars between blocks (decisions.md R-OSC-TIMED).
    timed_clock: crate::timed::Clock,
    now: usize,
    /// Each key's velocity as struck, which the notes it gives play at (decisions.md
    /// R-VELOCITY).
    played: velocity::Played,
    /// The standalone's audio thread, made an audio thread as its first block begins
    /// (decisions.md R-DROPOUTS).
    audio_thread: audio_thread::AudioThread,
}

impl std::fmt::Debug for Ca72 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ca72")
            .field("engine", &self.engine)
            .finish()
    }
}

impl Default for Ca72 {
    fn default() -> Self {
        Ca72::with_params(Ca72Params::default())
    }
}

impl Ca72 {
    /// Its parameters, typed (the host's view is [`Plugin::params`]).
    pub fn parameters(&self) -> &Arc<Ca72Params> {
        &self.params
    }

    fn with_params(params: Ca72Params) -> Ca72 {
        // A host may restore MIDI assignments before activation. Load the computer's source
        // first, so a freed CC 1 assignment is not discarded against the temporary default.
        if !cfg!(test) {
            params
                .midi_map
                .set_wheel_source(settings::Settings::load().mod_wheel_source);
        }
        let params = Arc::new(params);
        let expressions = expression::Expressions::counting(params.heard.clone());
        let preset_actions = Arc::new(preset_actions::PresetActions::new(
            params.clone(),
            library::Library::shared(),
        ));
        // The standalone's effects, given what moves their pins (decisions.md R-FX-PARAMS).
        #[cfg(feature = "fx-host")]
        let fx = crate::fx::take_runner();
        #[cfg(feature = "fx-host")]
        if fx.is_some() {
            crate::fx::bind_patch(params.lfo_patch.clone());
        }
        Ca72 {
            params,
            engine: Box::new(Engine::new()),
            meters: Arc::new(Meters::default()),
            local_notes: Arc::new(local_notes::LocalNotes::default()),
            note_sources: local_notes::Sources::default(),
            local_generation: 0,
            panic_generation: 0,
            deferred_local: None,
            rate: 48_000.0,
            helper: None,
            smooth: Smooth::default(),
            curve_seen: 0,
            preset_actions,
            preset_held: [false; 4096],
            preset_notes: [false; 2048],
            wheel: mod_wheel::Wheel::default(),
            expressions,
            #[cfg(feature = "fx-host")]
            fx,
            arp: Arp::default(),
            timed_clock: crate::timed::Clock::default(),
            now: 0,
            played: velocity::Played::default(),
            audio_thread: audio_thread::AudioThread::default(),
        }
    }
}

impl Drop for Ca72 {
    /// The helper stopped and joined first (a mend in progress stops before its next voice),
    /// then the voices' workers, as the engine drops: when the host's destroy returns, no
    /// thread of the plug-in's runs, and the host may unload it (decisions.md R23).
    fn drop(&mut self) {
        drop(self.helper.take());
    }
}

/// The side chain at sample `i`, its channels mixed to mono: a channel shorter than the block
/// (one the host did not supply may be) or a sample that is not finite counts as 0
/// (decisions.md R18).
fn mono(channels: &[&mut [f32]], i: usize) -> f32 {
    if channels.is_empty() {
        return 0.0;
    }
    channels
        .iter()
        .map(|c| c.get(i).copied().filter(|x| x.is_finite()).unwrap_or(0.0))
        .sum::<f32>()
        / channels.len() as f32
}

/// The host's transport as the arpeggiator reads it at a block's start, `tempo` (the standalone's
/// TEMPO) where the host gives none.
fn arp_clock(t: &Transport, rate: f64, tempo: f64) -> arp::Clock {
    let bar = match (t.time_sig_numerator, t.time_sig_denominator) {
        (Some(n), Some(d)) if n > 0 && d > 0 => Some(f64::from(n) * 4.0 / f64::from(d)),
        _ => None,
    };
    arp::Clock {
        tempo: t
            .tempo
            .filter(|t| t.is_finite() && *t > 0.0)
            .unwrap_or(tempo),
        playing: t.playing,
        beats: t.pos_beats(),
        bar_start: t.bar_start_pos_beats(),
        bar,
        rate,
    }
}

/// The host's MIDI as the engine takes it (any channel), if it takes it.
fn event_of(e: NoteEvent<()>) -> Option<Event> {
    Some(match e {
        // Native CLAP (and VST3) permits a zero-velocity attack. MIDI's zero-velocity
        // note-on has already become NoteOff in the wrapper's MIDI decoder.
        NoteEvent::NoteOn { note, .. } => Event::Note {
            key: note,
            on: true,
        },
        NoteEvent::NoteOff { note, .. } | NoteEvent::Choke { note, .. } => Event::Note {
            key: note,
            on: false,
        },
        NoteEvent::MidiPitchBend { value, .. } => Event::PitchBend(value * 2.0 - 1.0),
        NoteEvent::MidiCC { cc, value, .. } => match cc {
            MODULATION_WHEEL => Event::Modulation(value),
            RESET_CONTROLLERS => Event::ResetControllers,
            ALL_SOUND_OFF => Event::AllSoundOff,
            ALL_NOTES_OFF => Event::AllNotesOff,
            _ => return None,
        },
        _ => return None,
    })
}

impl Ca72 {
    /// A MIDI event: the chosen wheel source (R-MODWHEEL-SOURCE), a learnable control change
    /// to MIDI Learn (R34), else the existing notes, bend and channel mode actions. Whether it
    /// set a sound parameter.
    fn midi(&mut self, e: NoteEvent<()>, context: &mut impl ProcessContext<Self>) -> bool {
        self.follow_wheel_source();
        if let NoteEvent::NoteOn { channel, note, .. }
        | NoteEvent::NoteOff { channel, note, .. }
        | NoteEvent::Choke { channel, note, .. } = e
            && channel < 16
            && note < 128
        {
            let on = matches!(e, NoteEvent::NoteOn { .. });
            let slot = usize::from(channel) * 128 + usize::from(note);
            let was = std::mem::replace(&mut self.preset_held[2048 + slot], on);
            let incoming = self
                .params
                .midi_map
                .incoming_note(channel, note, on && !was);
            let mut set = false;
            let used = match incoming {
                Incoming::Caught => true,
                Incoming::Assigned(i) => {
                    if on && !was {
                        self.step_preset(i);
                        // A switch's note turns it over (R-MIDI-MAP).
                        if learn::action(i).is_none() {
                            let timing = match e {
                                NoteEvent::NoteOn { timing, .. } => timing,
                                _ => 0,
                            };
                            set = self.turn_over(i, timing, context);
                        }
                    }
                    true
                }
                Incoming::Unassigned => false,
            };
            let suppressed = self.preset_notes[slot];
            self.preset_notes[slot] = on && (used || suppressed);
            if used || suppressed {
                if !on {
                    // A note that was already sounding before it was assigned still gets its
                    // release, and lets go of the wheel's polyphonic aftertouch.
                    if self.note_sources.physical(channel, note, false) {
                        self.play(note, false, 0.0);
                    }
                    self.expressions.event(e, &mut |x| self.engine.event(x));
                    if let Some(value) = self.wheel.event(e, &self.expressions) {
                        self.engine.event(Event::Modulation(value));
                    }
                }
                // A preset pad is not a held key: its pressure never moves the wheel.
                return set;
            }
        }
        // A note's own expression (decisions.md R-MPE), to the engine before the note; a member
        // channel's bend, pressure or timbre is its note's alone and goes no further.
        let engine = &mut self.engine;
        let member = self.expressions.event(e, &mut |x| engine.event(x));
        if let Some(value) = self.wheel.event(e, &self.expressions) {
            self.engine.event(Event::Modulation(value));
        }
        if member {
            return false;
        }
        if let NoteEvent::MidiCC {
            timing,
            channel,
            cc,
            value,
        } = e
        {
            if learn::reserved(cc).is_none() {
                let on = value.is_finite() && value >= 64.0 / 127.0;
                let slot = usize::from(channel & 15) * 128 + usize::from(cc);
                let was = std::mem::replace(&mut self.preset_held[slot], on);
                match self.params.midi_map.incoming_cc(channel, cc, on && !was) {
                    Incoming::Assigned(i) if learn::action(i).is_some() => {
                        if on && !was {
                            self.step_preset(i);
                        }
                        return false;
                    }
                    Incoming::Assigned(i) => return self.learned(i, value, timing, context),
                    // Caught for the parameter being learned: the sound is not changed by it.
                    Incoming::Caught => return false,
                    Incoming::Unassigned => {}
                }
                if self.wheel.source == mod_wheel::Source::Cc(cc) {
                    self.params.midi_map.refuse(cc);
                    self.engine.event(Event::Modulation(value));
                } else if cc == arp::SUSTAIN {
                    // The sustain pedal, unless MIDI Learn or the MOD wheel has it: the
                    // arpeggiator's, the keys let go while it is down held (decisions.md R-ARP;
                    // the instrument, as before, takes no notice of it).
                    self.pedal(value >= 0.5);
                }
                return false;
            }
            // (Said in the editor while it learns: this one is not learned.)
            self.params.midi_map.refuse(cc);
        }
        if let Some(event) = event_of(e) {
            if let Event::Note { key, on } = event {
                let (channel, velocity) = match e {
                    NoteEvent::NoteOn {
                        channel, velocity, ..
                    } => (channel, velocity),
                    NoteEvent::NoteOff { channel, .. } | NoteEvent::Choke { channel, .. } => {
                        (channel, 0.0)
                    }
                    _ => (0, velocity::FULL),
                };
                if self.note_sources.physical(channel, key, on) {
                    self.play(key, on, velocity);
                }
            } else {
                if matches!(event, Event::ResetControllers) {
                    self.wheel.reset_pressure();
                }
                if matches!(event, Event::AllSoundOff | Event::AllNotesOff) {
                    self.clear_note_sources();
                    self.arp.clear();
                }
                if event == Event::ResetControllers {
                    self.pedal(false);
                }
                self.engine.event(event);
            }
        }
        false
    }

    fn step_preset(&self, i: usize) {
        if let Some(by) = learn::action(i) {
            self.preset_actions.request(by);
            if let Some(helper) = &self.helper {
                helper.ask(helper::PRESETS);
            }
        }
    }

    fn follow_wheel_source(&mut self) {
        let word = self.params.midi_map.wheel_word();
        if word != self.wheel.seen {
            self.wheel.seen = word;
            self.wheel.source = mod_wheel::Source::of_code(word as u8);
            self.wheel.reset_pressure();
            self.engine.event(Event::Modulation(0.0));
        }
    }

    /// A key pressed or let go, as the note sources pass it: through the arpeggiator, which
    /// hands it on as played while it is off, to the engine.
    /// Struck, `velocity` (0..1) is its velocity (decisions.md R-VELOCITY).
    fn play(&mut self, key: u8, on: bool, velocity: f32) {
        if on {
            self.played.struck(key, velocity);
        }
        let (engine, played) = (&mut self.engine, &self.played);
        self.arp.key(key, on, self.now, &mut |key, on| {
            played.to_engine(engine, key, on)
        });
    }

    /// The sustain pedal (control change 64), down or up.
    fn pedal(&mut self, down: bool) {
        let (engine, played) = (&mut self.engine, &self.played);
        self.arp.pedal(down, self.now, &mut |key, on| {
            played.to_engine(engine, key, on)
        });
    }

    /// The tempo `bpm` from sample `at` of a block of `len` (TEMPO moved by a modulation
    /// source, decisions.md R-LFO): the arpeggiator's beats, oscillators 2's and 3's and the
    /// timed LFOs' clock go on from where they were at it.
    fn retime(&mut self, at: usize, bpm: f64, len: usize) {
        if self.arp.retime(at, bpm, self.rate, len)
            && let Some(p) = self.timed_clock.retime(at, self.arp.position().1, len)
        {
            self.engine.retime(p);
        }
    }

    /// What the arpeggiator has due by the block's sample `at` played.
    fn arp_until(&mut self, at: usize) {
        let (engine, played) = (&mut self.engine, &self.played);
        self.arp
            .advance(at, &mut |key, on| played.to_engine(engine, key, on));
    }

    fn clear_note_sources(&mut self) {
        self.wheel.release();
        self.expressions.release();
        if self.wheel.source == mod_wheel::Source::PolyPressure {
            self.engine.event(Event::Modulation(0.0));
        }
        self.local_notes.release();
        self.deferred_local = None;
        self.local_generation = self.local_notes.generation();
        self.note_sources.clear();
        self.preset_held[2048..].fill(false);
        self.preset_notes.fill(false);
        self.local_notes.publish([0, 0]);
    }

    /// Apply one boundary reset before local input. A paired standalone request may arrive
    /// from UI just after the host captured its block: leave fresh notes queued until its
    /// hardware panic arrives, so a later reset cannot kill a note already played.
    fn local_reset(&mut self, standalone_panic: bool, token: Option<u64>) -> bool {
        let awaiting_standalone = self.local_notes.standalone_panic_pending();
        if awaiting_standalone && !standalone_panic {
            self.release_local_generation();
            return false;
        }
        let panic = self.local_notes.panic_generation();
        let ui_panic = panic != self.panic_generation;
        if standalone_panic || ui_panic {
            if standalone_panic {
                if let Some(token) = token {
                    self.local_notes.finish_standalone_panic(token);
                }
                // An external host panic invalidates pending local input. The explicit UI
                // button already advanced its epoch, so coalesce that paired request.
                if token.is_none() && !ui_panic && !awaiting_standalone {
                    self.local_notes.release();
                }
            }
            self.note_sources.clear();
            self.wheel.release();
            self.expressions.release();
            self.local_generation = self.local_notes.generation();
            self.local_notes.publish([0, 0]);
            self.arp.clear();
            self.expressions.reset_controllers();
            self.engine.event(Event::AllSoundOff);
            self.engine.event(Event::ResetControllers);
            if !self.local_notes.standalone_panic_pending() {
                self.panic_generation = panic;
            }
        }
        self.release_local_generation();
        !self.local_notes.standalone_panic_pending()
    }

    fn release_local_generation(&mut self) {
        let generation = self.local_notes.generation();
        if generation != self.local_generation {
            self.local_generation = generation;
            let (engine, arp, now, played) =
                (&mut self.engine, &mut self.arp, self.now, &self.played);
            self.note_sources.release_local(|key| {
                arp.key(key, false, now, &mut |key, on| {
                    played.to_engine(engine, key, on)
                })
            });
        }
    }

    /// A bounded batch at the next callback's boundary; host events retain their sample offsets.
    fn local_events(&mut self, standalone_panic: bool, token: Option<u64>) {
        if !self.local_reset(standalone_panic, token) {
            return;
        }
        for _ in 0..local_notes::CAPACITY {
            let Some(e) = self
                .deferred_local
                .take()
                .or_else(|| self.local_notes.pop())
            else {
                break;
            };
            // Observe a UI reset that arrived during the drain before accepting its fresh
            // epoch. If it awaits hardware panic, retain this one event for the next callback.
            if !self.local_reset(false, None) {
                self.deferred_local = Some(e);
                break;
            }
            if e.generation == self.local_generation && self.note_sources.local(e.note, e.on) {
                self.play(e.note, e.on, e.velocity);
            }
        }
    }

    /// Learnable parameter `i`'s controller at `value` (0..1, as the host gives a control change;
    /// 7 bits), `timing` samples into the block: the parameter set through the host, as the
    /// host's automation sets it (the editor and the host following, the state holding it), and
    /// a knob gliding there in the voices over `learn::DEZIP` ([`Smooth::learned`]). Whether it
    /// changed.
    fn learned(
        &mut self,
        i: usize,
        value: f32,
        timing: u32,
        context: &mut impl ProcessContext<Self>,
    ) -> bool {
        let seven = (value.clamp(0.0, 1.0) * 127.0).round() as u8;
        // (An effects' pin, learned by its pin: the chain sets its effect's parameter, the
        // standalone's alone. R-FX-PARAMS.)
        if let Some(k) = learn::LEARNABLE.get(i).and_then(|l| learn::fx_pin(l.id)) {
            #[cfg(feature = "fx-host")]
            if let Some(fx) = &mut self.fx
                && fx.is_pinned(k)
            {
                fx.set_pin_value(k, f64::from(seven) / 127.0);
                return true;
            }
            let _ = k;
            return false;
        }
        let Some(target) = learn::target(&self.params, i) else {
            return false;
        };
        let normalized = learn::normalized_for(&self.params, i, target, seven);
        if target.at(normalized) {
            return false;
        }
        context.set_parameter_normalized(target.ptr(), normalized, timing);
        if let Some(k) = learn::LEARNABLE.get(i).and_then(|l| smooth::index(l.id)) {
            self.smooth.learned(k, &self.params);
        }
        true
    }

    /// Switch `i` turned over by its learned note (R-MIDI-MAP): off if it is on, else on, as a
    /// controller's 0 or 127 sets it. Whether it set it.
    fn turn_over(
        &mut self,
        i: usize,
        timing: u32,
        context: &mut impl ProcessContext<Self>,
    ) -> bool {
        let Some(target) = learn::target(&self.params, i) else {
            return false;
        };
        let on = target.at(learn::normalized_for(&self.params, i, target, 127));
        self.learned(i, if on { 0.0 } else { 1.0 }, timing, context)
    }

    /// The engine's controls: the parameters, each continuous knob where the voices have it.
    fn controls(&self) -> crate::engine::Controls {
        self.params.controls_with(&|k| self.smooth.value(k))
    }

    /// What the engine asks done off the audio thread since the last block, asked of the
    /// helper (decisions.md R23): allocates nothing and does not wait.
    fn ask_helper(&mut self) {
        let mend = if self.engine.take_mending() { MEND } else { 0 };
        let serve = if self.engine.take_serving() { SERVE } else { 0 };
        if let Some(h) = &self.helper {
            h.ask(mend | serve);
        }
    }
}

/// EXTERNAL INPUT: a side chain of as many channels as the output (mixed to mono at the jack).
const SIDE_CHAIN_STEREO: &[std::num::NonZeroU32] = &[new_nonzero_u32(2)];
const SIDE_CHAIN_MONO: &[std::num::NonZeroU32] = &[new_nonzero_u32(1)];

const fn layout(channels: u32, name: &'static str) -> AudioIOLayout {
    AudioIOLayout {
        main_input_channels: None,
        main_output_channels: Some(new_nonzero_u32(channels)),
        aux_input_ports: if channels == 1 {
            SIDE_CHAIN_MONO
        } else {
            SIDE_CHAIN_STEREO
        },
        names: PortNames {
            layout: Some(name),
            main_output: Some("Output"),
            aux_inputs: &["External Input"],
            ..PortNames::const_default()
        },
        ..AudioIOLayout::const_default()
    }
}

impl Plugin for Ca72 {
    const NAME: &'static str = "CA-72";
    const VENDOR: &'static str = "Idle Foundry";
    const URL: &'static str = "https://github.com/idlefoundry/ca-72";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[layout(2, "Stereo"), layout(1, "Mono")];

    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
    /// None: nih-plug's background thread, shared by every instance, holds a plug-in alive
    /// while it runs one of its tasks, past the host's destroy. The plug-in's own helper does
    /// that work instead (decisions.md R23).
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    /// A session saved before ENTROPY (decisions.md R14) holds ANALOG's `analog`, which no
    /// parameter takes: it opens with ENTROPY at its default, not at what the instance had
    /// (R18).
    ///
    /// A state without MIDI assignments (saved before MIDI Learn) opens with none, whatever the
    /// instance had; a table that is not understood is none either (decisions.md R34).
    ///
    /// A state 0.1 saved has the knobs whose laws 0.2.0's calibration measured moved to where
    /// they set what they set then (`compat`, decisions.md R-TRANSLATE).
    ///
    /// A session saved with SCATTER's EDGES, no longer offered (INNER does its work: decisions.md
    /// R49), or with a placement not understood, opens with EVEN, the default, not at what
    /// the instance had.
    fn filter_state(state: &mut PluginState) {
        learn::filter_state(&mut state.fields);
        // A project saved before preset effects has none to carry, whatever this instance
        // had selected before it was recalled (R-FX-PRESETS).
        let effects = state
            .fields
            .get("preset_effects")
            .and_then(|s| serde_json::from_str::<Option<String>>(s).ok())
            .unwrap_or_default();
        if let Ok(s) = serde_json::to_string(&effects) {
            state.fields.insert("preset_effects".into(), s);
        }
        // A state saved before DOUBLE was a switch of its own: DOUBLE where its DETUNE is
        // above 0 (decisions.md R45).
        if !state.params.contains_key("doubled") {
            let detuned =
                matches!(state.params.get("double"), Some(ParamValue::F32(d)) if *d > 0.0);
            state
                .params
                .insert("doubled".to_owned(), ParamValue::Bool(detuned));
        }
        // A state without AUTO GAIN's curve (saved before DRIVE), or with one that is not
        // understood, opens with the average's, whatever the instance had (decisions.md
        // R45, the CA-74's R29).
        let curve = state
            .fields
            .get(drive::STATE_KEY)
            .and_then(|t| serde_json::from_str::<drive::Saved>(t).ok())
            .unwrap_or_else(drive::unmeasured);
        if let Ok(t) = serde_json::to_string(&curve) {
            state.fields.insert(drive::STATE_KEY.to_owned(), t);
        }
        // A state without the LFOs' assignments (saved before them), or with ones not
        // understood, opens with none, whatever the instance had (decisions.md R-LFO).
        let lfo = state
            .fields
            .get(lfo::STATE_KEY)
            .and_then(|t| serde_json::from_str::<lfo::Saved>(t).ok())
            .unwrap_or_default();
        if let Ok(t) = serde_json::to_string(&lfo) {
            state.fields.insert(lfo::STATE_KEY.to_owned(), t);
        }
        // The pre-0.2.7 ENTROPY range becomes 5..100 %, including an absent amount's
        // old default (R-ENTROPY-FLOOR). Automation belongs to the host and is left to it.
        if compat::before_entropy_floor(&state.version)
            && let Some(ParamValue::F32(v)) = state.params.get_mut("entropy")
        {
            *v = compat::entropy_percent(f64::from(*v)) as f32;
        }
        state.params.remove("analog");
        if !state.params.contains_key("entropy") {
            let entropy = Ca72Params::default().entropy.default_plain_value();
            state
                .params
                .insert("entropy".to_owned(), ParamValue::F32(entropy));
        }
        // A state saved before NOTE PRIORITY: the instrument's own, LOW, whatever the instance
        // had (decisions.md R50).
        if !state.params.contains_key("note_priority") {
            let low = Ca72Params::default().note_priority.default_plain_value();
            state.params.insert(
                "note_priority".to_owned(),
                ParamValue::I32(low.to_index() as i32),
            );
        }
        // And before FILTER POLES: 4, the filter as drawn, whatever the instance had (decisions.md
        // R-POLES). (FILTER MODE keeps its own, HI among them.)
        if !state.params.contains_key("filter_poles") {
            state.params.insert(
                "filter_poles".to_owned(),
                ParamValue::I32(i32::from(ca72::vcf::POLES)),
            );
        }
        // And before QUALITY (0.1's): HI, as such a session played until LO became the default
        // for new instances (R-LO-DEFAULT).
        if !state.params.contains_key("quality") {
            state.params.insert(
                "quality".to_owned(),
                ParamValue::I32(params::QualityMode::Hi.to_index() as i32),
            );
        }
        // And before the arpeggiator: off, each of its settings at its default, whatever the
        // instance had (decisions.md R-ARP).
        let d = Ca72Params::default();
        for (id, v) in [
            ("arp_on", ParamValue::Bool(false)),
            ("arp_latch", ParamValue::Bool(false)),
            (
                "arp_direction",
                ParamValue::I32(d.arp_direction.default_plain_value().to_index() as i32),
            ),
            (
                "arp_rate",
                ParamValue::I32(d.arp_rate.default_plain_value().to_index() as i32),
            ),
            (
                "arp_octaves",
                ParamValue::I32(d.arp_octaves.default_plain_value()),
            ),
            (
                "arp_gate",
                ParamValue::F32(d.arp_gate.default_plain_value()),
            ),
            (
                "arp_swing",
                ParamValue::F32(d.arp_swing.default_plain_value()),
            ),
        ] {
            state.params.entry(id.to_owned()).or_insert(v);
        }
        // And before the oscillators timed: each off, its DIVISION at its default, whatever the
        // instance had (decisions.md R-OSC-TIMED).
        let quarter = d.osc3_division.default_plain_value().to_index() as i32;
        for n in 2..=3 {
            state
                .params
                .entry(format!("osc{n}_timed"))
                .or_insert(ParamValue::Bool(false));
            state
                .params
                .entry(format!("osc{n}_division"))
                .or_insert(ParamValue::I32(quarter));
        }
        // And before CONTINUOUS's waveforms: each oscillator STEPPED, the instrument's switch,
        // its shape at SAWTOOTH, its MOD off and clockwise (decisions.md R-CONTINUOUS-WAVE).
        for n in 1..=3 {
            let (mode, shape, _, direction) = d.shape_params(n - 1);
            for (id, v) in [
                (
                    format!("osc{n}_shape_mode"),
                    ParamValue::I32(mode.default_plain_value().to_index() as i32),
                ),
                (
                    format!("osc{n}_shape"),
                    ParamValue::F32(shape.default_plain_value()),
                ),
                (format!("osc{n}_shape_mod"), ParamValue::Bool(false)),
                (
                    format!("osc{n}_shape_mod_dir"),
                    ParamValue::I32(direction.default_plain_value().to_index() as i32),
                ),
            ] {
                state.params.entry(id).or_insert(v);
            }
        }
        // And before GLIDE's LONG range: NORMAL, the instrument's capacitor (R-GLIDE-LONG).
        state
            .params
            .entry("glide_range".to_owned())
            .or_insert(ParamValue::I32(0));
        // A state 0.3.0's beta saved: each LFO's RISE its FADE, a fade in (R-LFO-FADE).
        for rise in crate::lfo::RISE_IDS {
            if let (Some(v), Some(fade)) = (state.params.remove(rise), crate::lfo::fade_id(rise)) {
                state.params.entry(fade.to_owned()).or_insert(v);
            }
        }
        // And before the macros: each at 0, whatever the instance had (decisions.md R-MACROS).
        for id in crate::lfo::MACRO_IDS {
            state
                .params
                .entry(id.to_owned())
                .or_insert(ParamValue::F32(0.0));
        }
        // And before RETRIGGER: off, single triggering, the instrument's own.
        if !state.params.contains_key("retrigger") {
            state
                .params
                .insert("retrigger".to_owned(), ParamValue::Bool(false));
        }
        // And before each note's own expression: none of it reaches the voices, whatever the
        // instance had (decisions.md R-MPE).
        for id in [
            "pressure_mod",
            "pressure_filter",
            "timbre_mod",
            "timbre_filter",
            // And before the velocity's response: a straight line from 0 (R-VELOCITY).
            "velocity_curve",
            "velocity_floor",
        ] {
            state
                .params
                .entry(id.to_owned())
                .or_insert(ParamValue::F32(0.0));
        }
        let offered = |id: &str| params::Scatter::ids().is_some_and(|ids| ids.contains(&id));
        if matches!(state.params.get("placement"), Some(ParamValue::String(id)) if !offered(id)) {
            let even = Ca72Params::default().placement.default_plain_value();
            state.params.insert(
                "placement".to_owned(),
                ParamValue::I32(even.to_index() as i32),
            );
        }
        // A state 0.1 saved: each knob whose law 0.2.0's calibration changed moved to where it
        // sets what it set (decisions.md R-TRANSLATE).
        if compat::before_0_2(&state.version) {
            compat::translate(&mut state.params);
        }
    }

    fn editor(&mut self, _executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        Some(Box::new(Ca72Editor::with_notes(
            self.params.clone(),
            self.meters.clone(),
            self.local_notes.clone(),
        )))
    }

    fn initialize(
        &mut self,
        _layout: &AudioIOLayout,
        config: &BufferConfig,
        context: &mut impl InitContext<Self>,
    ) -> bool {
        // This computer's FREQUENCY dials and wheel source, with the editor open or not
        // (decisions.md R54, R-MODWHEEL-SOURCE), and the standalone's TEMPO, the arpeggiator's
        // where the host gives none (R-ARP).
        if !cfg!(test) {
            let settings = crate::settings::Settings::load();
            self.params.set_freq_law(settings.frequency);
            self.params
                .midi_map
                .set_wheel_source(settings.mod_wheel_source);
            self.params.set_mpe_mode(settings.mpe);
            self.params.set_timbre_rest(settings.timbre_rest);
            self.params.set_tempo(f64::from(settings.tempo));
        }
        self.engine.set(&self.params.controls());
        self.engine
            .prepare(f64::from(config.sample_rate), self.params.seed());
        // (RANDOM's from the session's seed, so that a render repeats.)
        self.arp.seed(self.params.seed());
        // One latency in every QUALITY mode, ULTRA's (HI and LO held back to match), so a
        // host never compensates afresh when it switches (decisions.md R-LATENCY).
        context.set_latency_samples(self.engine.latency());
        self.params
            .set_standalone(context.plugin_api() == PluginApi::Standalone);
        let block = config.max_buffer_size.max(1) as usize;
        let rate = f64::from(config.sample_rate);
        self.rate = rate;
        // (Every knob at its value at once: a session or a project's state lands, R-KNOB-GLIDE.)
        self.smooth.prepare(rate, block, &self.params);
        // (AUTO GAIN's measurements on the helper thread: decisions.md R45.)
        self.engine.calibrate_with(self.params.drive_curve.clone());
        if self.helper.is_none() {
            self.helper = Helper::start_with_presets(
                self.engine.spares(),
                self.engine.crew(),
                self.params.drive_curve.clone(),
                Some(self.preset_actions.clone()),
            )
            .ok();
        }
        // POLY's workers and ULTRA's split crews, audio threads for the host's largest block
        // (decisions.md R11), as many as this machine gives the instance (R-THREADS), held only
        // while it needs them: started here if it does, else on the helper thread when it comes
        // to (R21, R23). A host loading a state into the running instance comes here again with
        // the audio thread waiting: running threads are kept then (R18).
        let period = std::time::Duration::from_secs_f64(block as f64 / rate);
        // The standalone's own audio thread, an audio thread as the workers are, from its next
        // block (decisions.md R-DROPOUTS): a plug-in's host's thread is the host's. RTKit, where
        // the system refuses it and the workers, is asked only in the standalone application,
        // whose `main` turned it on before any stream (R-RTKIT); never by this library.
        self.audio_thread
            .prepare(context.plugin_api() == PluginApi::Standalone, period);
        self.engine.start_workers(crate::engine::AUTO, Some(period));
        // The effects at the device's rate and block (the audio thread is stopped here).
        #[cfg(feature = "fx-host")]
        if let Some(fx) = &mut self.fx {
            fx.prepare(rate, block);
        }
        true
    }

    /// POLY's workers stopped and given back to the process's budget (decisions.md R21): the
    /// host initialises the plug-in again before it next plays.
    fn deactivate(&mut self) {
        self.engine.stop_workers();
    }

    fn reset(&mut self) {
        self.preset_actions.cancel();
        self.clear_note_sources();
        self.arp.clear();
        self.timed_clock.clear();
        self.smooth.land(&self.params);
        self.preset_held.fill(false);
        self.preset_notes.fill(false);
        self.engine.set(&self.params.controls());
        self.engine.release_all();
        self.expressions.reset_controllers();
        self.engine.event(Event::ResetControllers);
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        #[cfg(feature = "fx-host")]
        let started = std::time::Instant::now();
        // (The standalone's audio thread made an audio thread at its first block: decisions.md
        // R-DROPOUTS. Where the system refuses, it stays as it was.)
        if let Some(Err(_)) = self.audio_thread.begin()
            && let Some(helper) = &self.helper
        {
            helper.ask(helper::REPORT);
        }
        // (The standalone's TEMPO a modulation target there alone: decisions.md R-LFO.)
        self.params
            .set_standalone(context.plugin_api() == PluginApi::Standalone);
        // (A preset a MIDI pad or key chose lands at once in the voices, as the drawer's does:
        // decisions.md R-PRESET-KEYS, R-KNOB-GLIDE.)
        if self.preset_actions.apply(context) {
            self.smooth.land(&self.params);
            if let Some(helper) = &self.helper {
                helper.ask(helper::PRESETS);
            }
        }
        // (A continuous knob with a new value glides there from where the voices have it, or
        // lands there if a preset set it: decisions.md R-KNOB-GLIDE.)
        self.smooth.follow(&self.params);
        // (A curve measured after DRIVE was turned glides in; a preset's or a session's is the
        // sound's at once: decisions.md R-AUTO-GAIN-DRIVE.)
        if let Some(c) = self.params.drive_curve.read(&mut self.curve_seen) {
            if self.params.drive_curve.measured(&c) {
                self.engine.refine_curve(c);
            } else {
                self.engine.set_curve(c);
            }
        }
        self.engine.set(&self.controls());
        self.follow_wheel_source();
        let engine = &mut self.engine;
        self.expressions
            .set_mode(self.params.mpe_mode(), &mut |x| engine.event(x));
        self.expressions
            .set_rest(self.params.timbre_rest(), &mut |x| engine.event(x));
        let len = buffer.samples();
        // The arpeggiator's settings and the host's transport, as the block begins (ON or LATCH
        // changed are taken at its first sample).
        self.now = 0;
        // (Moved by a source, the block begins at the tempo in force at the LFOs' latest tick.)
        let tempo = self.engine.tempo().unwrap_or_else(|| self.params.tempo());
        let clock = arp_clock(context.transport(), self.rate, tempo);
        let (engine, played) = (&mut self.engine, &self.played);
        self.arp
            .begin(self.params.arp(), clock, len, &mut |key, on| {
                played.to_engine(engine, key, on)
            });
        // Oscillator 3 timed keeps the arpeggiator's clock, so that the two agree: the host's
        // place every block while it plays, else run on at the tempo (decisions.md
        // R-OSC-TIMED).
        let (beats, per) = self.arp.position();
        let at = self.timed_clock.begin(context.transport(), beats, per, len);
        // (Where the block began on the clock, for the standalone's effects: decisions.md
        // R-LFO.)
        #[cfg(feature = "fx-host")]
        let timing = ca72_fx::Timing {
            tempo: clock.tempo,
            beats,
            rate: self.rate,
        };
        self.engine.set_position(at);
        let panic_token = context.standalone_panic_token();
        let standalone_panic = context.take_standalone_panic();
        self.local_events(standalone_panic, panic_token);
        // The voices' deadline: a share of the block's period from now (decisions.md R11);
        // none rendering offline, where every voice is waited for (R18).
        let period = len as f64 / self.rate;
        let deadline = (context.process_mode() != ProcessMode::Offline).then(|| {
            std::time::Instant::now()
                + std::time::Duration::from_secs_f64(period * crate::engine::DEADLINE)
        });
        self.engine.set_deadline(deadline);
        let side = aux.inputs.first().map(|b| b.as_slice_immutable());
        let out = buffer.as_slice();
        let mut next = context.next_event();
        // The samples between one event and the next, a run at a time (the engine plays
        // POLY's voices one after another over a run: decisions.md R11), the side chain mixed
        // to mono. A learned MIDI controller sets its parameter at its event, and the voices take
        // it from there; while a knob glides, a run is at most `smooth::STEP` samples
        // (decisions.md R34, R-KNOB-GLIDE). The arpeggiator's notes are events too, at their
        // samples, after the host's at the same one (R-ARP).
        const RUN: usize = 128;
        let (mut ext, mut l, mut r) = ([0.0f32; RUN], [0.0f32; RUN], [0.0f32; RUN]);
        let mut i = 0;
        let mut glided = false;
        while i < len {
            self.now = i;
            let mut learned = false;
            while let Some(e) = next {
                if e.timing() as usize > i {
                    break;
                }
                learned |= self.midi(e, context);
                next = context.next_event();
            }
            self.arp_until(i);
            if learned || glided || self.smooth.moving() {
                self.smooth.follow(&self.params);
                self.engine.set(&self.controls());
            }
            // TEMPO moved by a source (the standalone's: decisions.md R-LFO): the tempo of the
            // LFOs' latest tick from here, everything it times going on from where it was, and
            // the run ending at their next tick, where it changes again.
            if let Some(bpm) = self.engine.tempo() {
                self.retime(i, bpm, len);
            }
            let mut end = next
                .map_or(len, |e| e.timing() as usize)
                .min(len)
                .min(i + RUN);
            if let Some(tick) = self.engine.next_tempo_tick() {
                end = end.min(tick.max(i + 1));
            }
            if self.smooth.moving() {
                end = end.min(i + smooth::STEP);
            }
            if let Some(due) = self.arp.due() {
                end = end.min(due.max(i + 1));
            }
            let n = end - i;
            let ext = match side {
                Some(s) if !s.is_empty() => {
                    for (k, x) in ext[..n].iter_mut().enumerate() {
                        *x = mono(s, i + k);
                    }
                    &ext[..n]
                }
                _ => &[][..],
            };
            // Left and right (SPREAD's voices placed); on a mono output, the two together.
            match &mut out[..] {
                [left, right, ..] => self
                    .engine
                    .render(ext, &mut left[i..end], &mut right[i..end]),
                [only] => {
                    self.engine.render(ext, &mut l[..n], &mut r[..n]);
                    for k in 0..n {
                        only[i + k] = 0.5 * (l[k] + r[k]);
                    }
                }
                [] => self.engine.render(ext, &mut l[..n], &mut r[..n]),
            }
            glided = self.smooth.moving();
            self.smooth.advance(n);
            i = end;
        }
        // (Past the block's end: a parameter set here reaches the voices at the next block's
        // start, a key the arpeggiator there.)
        self.now = len;
        while let Some(e) = next {
            self.midi(e, context);
            next = context.next_event();
        }
        self.local_notes.publish(self.note_sources.held());
        let lamp = self.engine.end_block(len);
        self.meters
            .publish(lamp, self.engine.midi_wheels(), self.engine.sounding_mask());
        // The voices' levels and the output's, for the editor's drops and QUALITY's lamps:
        // only while an editor is open (else a flag read a block, and nothing more).
        if self.meters.watched() {
            let peak = out.iter().fold(0.0f32, |m, ch| {
                ch.iter().take(len).fold(m, |m, v| m.max(v.abs()))
            });
            self.meters.publish_levels(self.engine.levels(), peak);
            // MOD's orbs (decisions.md R-CONTINUOUS-WAVE): where the newest voice's shapes are.
            if let Some(shapes) = self.engine.shapes() {
                self.meters.publish_shapes(shapes);
            }
            // The LFOs' scopes, orbs and rings (decisions.md R-LFO).
            self.params.lfo_scope.publish(&self.engine.lfo_readings());
            // The latest note's velocity, the left hand's meter (decisions.md R-VELOCITY).
            self.meters.publish_velocity(self.engine.last_velocity());
            // The tempo it played at, for the LFO page's timed scopes (decisions.md R-LFO-PAGE),
            // and the TEMPO screen: moved by a source, where the sources have it now (R-LFO).
            self.meters
                .publish_tempo(self.engine.tempo().unwrap_or(clock.tempo));
            // The strip's screens' controls as they play, modulated (R-LFO).
            self.meters.publish_live(&self.engine.controls_now());
        }
        // The standalone's effects, after the synth and its meters: each effect's block is
        // waited for until a share of the period from the block's start, then the effect
        // passes the sound on dry (docs/fx-host.md).
        #[cfg(feature = "fx-host")]
        if let Some(fx) = &mut self.fx
            && !fx.is_empty()
            && let [left, right, ..] = &mut out[..]
        {
            let deadline =
                started + std::time::Duration::from_secs_f64(period * crate::fx::DEADLINE);
            fx.set_timing(timing);
            // (The pins' points where the LFOs and macros moved them: R-FX-PARAMS.)
            let p = self.engine.fx_points();
            fx.process_modulated(
                &mut left[..len],
                &mut right[..len],
                Some(deadline),
                p.moved,
                &p.at[..p.n],
                &p.offsets[..p.n],
            );
        }
        self.ask_helper();
        ProcessStatus::KeepAlive
    }
}

impl ClapPlugin for Ca72 {
    const CLAP_ID: &'static str = "com.idlefoundry.ca-72";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("A synthesizer modelled from its circuit, one voice or up to ten");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Mono,
        ClapFeature::Stereo,
    ];
}

impl Vst3Plugin for Ca72 {
    const VST3_CLASS_ID: [u8; 16] = *b"IdleFoundryCA72!";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Instrument, Vst3SubCategory::Synth];
}

#[cfg(test)]
mod tempo_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use ca72::modulation::PITCH_WHEEL_SEMITONES;
    use nih_plug::context::process::TestProcessContext;
    use std::collections::BTreeMap;

    /// A context with no host, for the plug-in's parameters (nih-plug's `TestProcessContext`).
    pub(crate) fn hostless(p: &Ca72) -> TestProcessContext<Ca72> {
        let params: Arc<dyn Params> = p.params.clone();
        TestProcessContext::new(params, 48_000.0, ProcessMode::Realtime)
    }

    /// The same, rendering offline: every voice waited for (decisions.md R18). For two renders
    /// compared to the bit with POLY or UNISON on: in real time a voice whose worker misses the
    /// block's deadline is silent for the run, and under load that differs from render to render
    /// (R-ARP, "Fixed, 2026-10-27").
    pub(crate) fn offline(p: &Ca72) -> TestProcessContext<Ca72> {
        let params: Arc<dyn Params> = p.params.clone();
        TestProcessContext::new(params, 48_000.0, ProcessMode::Offline)
    }

    fn note(timing: u32, channel: u8, pitch: u8, on: bool) -> NoteEvent<()> {
        if on {
            NoteEvent::NoteOn {
                timing,
                voice_id: None,
                channel,
                note: pitch,
                velocity: 1.0,
            }
        } else {
            NoteEvent::NoteOff {
                timing,
                voice_id: None,
                channel,
                note: pitch,
                velocity: 0.0,
            }
        }
    }

    #[test]
    fn local_keyboard_plays_the_same_samples_as_host_midi_at_the_block_boundary() {
        let (mut local, mut physical) = (initialised(false), initialised(false));
        // A new instance intentionally gets a fresh noise/mismatch seed. Match the persisted
        // seed as loading the same saved patch does before comparing sample identity.
        for p in [&mut local, &mut physical] {
            p.params
                .noise_seed
                .store(0xca72, std::sync::atomic::Ordering::Relaxed);
            p.engine.prepare(48_000.0, p.params.seed());
        }
        let (mut lc, mut pc) = (hostless(&local), hostless(&physical));
        for i in 0..32 {
            if i == 0 || i == 16 {
                let on = i == 0;
                local.local_notes.note(60, on);
                pc.push_event(note(0, 0, 60, on));
            }
            let a = block(&mut local, &mut lc, 256);
            let b = block(&mut physical, &mut pc, 256);
            assert!(a == b, "local and physical MIDI differ in block {i}");
        }
    }

    #[test]
    #[cfg(feature = "standalone-production")]
    fn standalone_invalid_callbacks_and_midi_resets_do_not_acknowledge_ui_tokens() {
        use nih_plug::wrapper::production::Processor;
        use plugin_kit_standalone::{AudioBlock, AudioProcessor, MidiEvent, ProcessConfig};

        for kind in [0, 120, 123] {
            let p = Ca72::default();
            let notes = p.local_notes.clone();
            let mut playing = crate::playing::Playing::default();
            playing.notes = notes.clone();
            let (mut adapter, _) = Processor::new(p, &[]).expect("standalone adapter");
            adapter
                .prepare(ProcessConfig {
                    sample_rate: 48_000.0,
                    max_buffer_frames: 128,
                })
                .expect("prepared");
            let token = playing.panic_standalone();
            let (mut left, mut right) = ([0.0; 128], [0.0; 128]);
            let mut run = |adapter: &mut Processor<Ca72>, panic, midi: &[_]| {
                adapter.process(AudioBlock {
                    left: &mut left,
                    right: &mut right,
                    input_left: &[],
                    input_right: &[],
                    midi,
                    panic,
                });
            };
            if kind == 0 {
                adapter.panic_token(Some(token));
                adapter.process(AudioBlock {
                    left: &mut [0.0],
                    right: &mut [],
                    input_left: &[],
                    input_right: &[],
                    midi: &[],
                    panic: true,
                });
            } else {
                run(
                    &mut adapter,
                    false,
                    &[MidiEvent {
                        source: 1,
                        data: [0xb0, kind, 0],
                        len: 3,
                    }],
                );
            }
            playing.sync();
            playing.press(69);
            // No new metadata is supplied: the invalid call must have consumed its tag.
            run(&mut adapter, kind == 0, &[]);
            assert_eq!(
                notes.held(),
                [0, 0],
                "invalid/generic/CC reset cannot finish the UI request"
            );
            adapter.panic_token(Some(token));
            run(&mut adapter, true, &[]);
            assert_eq!(
                notes.held(),
                [0, 1 << 5],
                "matching boundary admits fresh input"
            );
        }
    }

    #[test]
    #[cfg(feature = "standalone-production")]
    fn repeated_standalone_panics_wait_for_their_own_captured_boundary() {
        use nih_plug::wrapper::production::Processor;
        use plugin_kit_standalone::{AudioBlock, AudioProcessor, ProcessConfig};

        // Old captured UI request, generic reset, and coalesced requests respectively.
        for preceding in [Some(1), Some(0), None] {
            let p = Ca72::default();
            let notes = p.local_notes.clone();
            let mut playing = crate::playing::Playing::default();
            playing.notes = notes.clone();
            let (mut adapter, _) = Processor::new(p, &[]).expect("standalone adapter");
            adapter
                .prepare(ProcessConfig {
                    sample_rate: 48_000.0,
                    max_buffer_frames: 128,
                })
                .expect("prepared");
            let (mut left, mut right) = ([0.0; 128], [0.0; 128]);
            let mut run = |token| {
                adapter.panic_token(token);
                adapter.process(AudioBlock {
                    left: &mut left,
                    right: &mut right,
                    input_left: &[],
                    input_right: &[],
                    midi: &[],
                    panic: true,
                });
            };
            let first = playing.panic_standalone();
            let second = playing.panic_standalone();
            playing.press(69);
            let fresh_epoch = notes.generation();
            if let Some(which) = preceding {
                run((which == 1).then_some(first));
                assert_eq!(
                    notes.held(),
                    [0, 0],
                    "old/generic capture cannot acknowledge newer UI input"
                );
            }
            run(Some(second));
            assert_eq!(
                notes.generation(),
                fresh_epoch,
                "matching capture cannot age fresh input"
            );
            assert_eq!(
                notes.held(),
                [0, 1 << 5],
                "fresh note survives its own boundary"
            );
        }
    }

    #[test]
    #[cfg(feature = "standalone-production")]
    fn standalone_ui_and_core_panics_coalesce_at_the_same_or_next_boundary() {
        use nih_plug::wrapper::production::Processor;
        use plugin_kit_standalone::{AudioBlock, AudioProcessor, MidiEvent, ProcessConfig};

        for split_boundary in [false, true] {
            let p = Ca72::default();
            let notes = p.local_notes.clone();
            let mut playing = crate::playing::Playing::default();
            playing.notes = notes.clone();
            let (mut adapter, _) = Processor::new(p, &[]).expect("standalone adapter");
            adapter
                .prepare(ProcessConfig {
                    sample_rate: 48_000.0,
                    max_buffer_frames: 128,
                })
                .expect("prepared");
            let (mut left, mut right) = ([0.0; 128], [0.0; 128]);
            let mut run = |panic, token, midi: &[_]| {
                adapter.panic_token(token);
                adapter.process(AudioBlock {
                    left: &mut left,
                    right: &mut right,
                    input_left: &[],
                    input_right: &[],
                    midi,
                    panic,
                });
            };
            let physical = |on| MidiEvent {
                source: 1,
                data: [if on { 0x90 } else { 0x80 }, 60, 100],
                len: 3,
            };
            playing.press(59);
            run(false, None, &[physical(true)]);
            assert_eq!(notes.held(), [(1 << 59) | (1 << 60), 0]);
            // The editor marks the paired request and changes the UI epoch. The core's
            // AudioBlock panic can belong to this callback or the following one.
            let token = playing.panic_standalone();
            playing.press(69);
            let fresh_epoch = notes.generation();
            if split_boundary {
                run(false, None, &[]);
                assert_eq!(
                    notes.held(),
                    [1 << 60, 0],
                    "fresh local input waits for hardware reset"
                );
            }
            run(true, Some(token), &[]);
            assert_eq!(
                notes.generation(),
                fresh_epoch,
                "the two panic paths share one UI epoch"
            );
            assert_eq!(
                notes.held(),
                [0, 1 << 5],
                "fresh note survives paired panic"
            );
            playing.release_pointer();
            run(false, None, &[physical(true)]);
            assert_eq!(
                notes.held(),
                [1 << 60, 0],
                "same physical pitch works after ownership reset"
            );
            run(false, None, &[physical(false)]);
            assert_eq!(
                notes.held(),
                [0, 0],
                "physical release does not strand a note"
            );
            playing.press(67);
            run(false, None, &[]);
            playing.press(70);
            run(true, None, &[]);
            assert_eq!(
                notes.held(),
                [0, 0],
                "external standalone panic clears local input too"
            );
            adapter.deactivate();
        }
    }

    #[test]
    fn keyboard_source_overlap_overflow_panic_and_reset_clear_correct_notes() {
        let mut p = initialised(false);
        let mut c = hostless(&p);
        c.push_event(note(31, 0, 60, true));
        c.push_event(note(99, 1, 60, true));
        p.local_notes.note(60, true);
        block(&mut p, &mut c, 128);
        assert_eq!(p.local_notes.held(), [1 << 60, 0]);
        p.local_notes.release();
        c.push_event(note(31, 0, 60, false));
        block(&mut p, &mut c, 128);
        assert_eq!(p.local_notes.held(), [1 << 60, 0], "channel 1 remains held");
        c.push_event(note(63, 1, 60, false));
        block(&mut p, &mut c, 128);
        assert_eq!(p.local_notes.held(), [0, 0]);
        p.local_notes.note(67, true);
        block(&mut p, &mut c, 128);
        for _ in 0..local_notes::CAPACITY {
            p.local_notes.note(67, true);
        }
        assert!(!p.local_notes.note(67, false));
        block(&mut p, &mut c, 128);
        assert_eq!(
            p.local_notes.held(),
            [0, 0],
            "overflow released the sounding local note"
        );
        p.local_notes.note(64, true);
        p.local_notes.panic();
        block(&mut p, &mut c, 128);
        assert_eq!(
            p.local_notes.held(),
            [0, 0],
            "panic invalidates old queued attacks"
        );
        p.local_notes.panic();
        p.local_notes.note(69, true);
        block(&mut p, &mut c, 128);
        assert_eq!(
            p.local_notes.held(),
            [0, 1 << 5],
            "a fresh note after panic remains playable"
        );
        p.local_notes.note(60, true);
        block(&mut p, &mut c, 128);
        p.reset();
        assert_eq!(p.local_notes.held(), [0, 0]);
        c.push_event(NoteEvent::from_midi(3, &[0x90, 60, 0]).expect("MIDI zero-velocity release"));
        block(&mut p, &mut c, 128);
        assert_eq!(
            p.local_notes.held(),
            [0, 0],
            "MIDI velocity zero is a release"
        );
        // CA-72's reserved CC120/123 have always been global, irrespective of channel.
        // Keep that compatibility behavior and invalidate local attacks alongside the engine.
        for controller in [ALL_NOTES_OFF, ALL_SOUND_OFF] {
            c.push_event(note(0, 2, 60, true));
            c.push_event(note(0, 3, 67, true));
            p.local_notes.note(64, true);
            block(&mut p, &mut c, 128);
            assert_ne!(p.local_notes.held(), [0, 0]);
            c.push_event(cc(31, 9, controller, 0));
            block(&mut p, &mut c, 128);
            assert_eq!(
                p.local_notes.held(),
                [0, 0],
                "CC{controller} stays a global reset"
            );
        }
    }

    #[test]
    fn the_hosts_midi_reaches_the_wheels_on_any_channel() {
        let mut p = Ca72::default();
        let mut c = hostless(&p);
        p.midi(
            NoteEvent::MidiPitchBend {
                timing: 0,
                channel: 3,
                value: 0.0,
            },
            &mut c,
        );
        p.midi(
            NoteEvent::MidiCC {
                timing: 0,
                channel: 9,
                cc: MODULATION_WHEEL,
                value: 1.0,
            },
            &mut c,
        );
        let (bend, modulation) = p.engine.wheels();
        assert!((bend + 2.0 / PITCH_WHEEL_SEMITONES).abs() < 1e-9, "{bend}");
        assert_eq!(modulation, 1.0);
        p.midi(
            NoteEvent::MidiCC {
                timing: 0,
                channel: 0,
                cc: RESET_CONTROLLERS,
                value: 0.0,
            },
            &mut c,
        );
        assert_eq!(p.engine.wheels(), (0.0, 0.0));
        assert!(c.reported.is_empty());
    }

    /// All Sound Off silences, All Notes Off releases (decisions.md R18).
    #[test]
    fn all_sound_off_and_all_notes_off_are_told_apart() {
        let cc = |cc| {
            event_of(NoteEvent::MidiCC {
                timing: 0,
                channel: 0,
                cc,
                value: 0.0,
            })
        };
        assert_eq!(cc(ALL_SOUND_OFF), Some(Event::AllSoundOff));
        assert_eq!(cc(ALL_NOTES_OFF), Some(Event::AllNotesOff));
        assert_eq!(cc(7), None);
    }

    /// The side chain's channels mixed, a channel shorter than the block (one the host did
    /// not supply may be) and a sample that is not finite taken as 0 (R18).
    #[test]
    fn the_side_chain_is_read_within_its_channels_and_finite() {
        let (mut a, mut b) = ([1.0, f32::NAN, 2.0, f32::INFINITY], [3.0f32]);
        let side: [&mut [f32]; 2] = [&mut a, &mut b];
        let mixed: Vec<f32> = (0..4).map(|i| mono(&side, i)).collect();
        assert_eq!(mixed, [2.0, 0.0, 1.0, 0.0]);
    }

    /// A session 0.1 saved opens translated into 0.2.0's calibration: its EMPHASIS where 0.2.0's
    /// law sets what 0.1's did, LEVEL trimmed and DRIVE at none (decisions.md R-TRANSLATE); one
    /// saved by 0.2.0 opens as it was saved.
    #[test]
    fn a_session_of_0_1_opens_translated_and_one_of_0_2_as_saved() {
        let state = |version: &str| PluginState {
            version: version.to_owned(),
            params: [("emphasis", 5.0f32), ("tune", 0.5)]
                .iter()
                .map(|&(k, v)| (k.to_owned(), ParamValue::F32(v)))
                .collect(),
            fields: Default::default(),
        };
        let f32_of = |s: &PluginState, id: &str| match s.params.get(id) {
            Some(ParamValue::F32(v)) => Some(*v),
            _ => None,
        };
        let mut old = state("0.1.5");
        Ca72::filter_state(&mut old);
        let want = compat::translated("emphasis", 5.0).unwrap() as f32;
        assert_eq!(f32_of(&old, "emphasis"), Some(want));
        assert!((want - 5.0).abs() > 0.1, "EMPHASIS's law changed: {want}");
        assert_eq!(f32_of(&old, "tune"), Some(0.5));
        assert_eq!(f32_of(&old, "level"), Some(compat::LEVEL_FROM_0_1 as f32));
        assert_eq!(f32_of(&old, "drive"), Some(0.0));
        let mut new = state("0.2.0");
        Ca72::filter_state(&mut new);
        assert_eq!(f32_of(&new, "emphasis"), Some(5.0));
        assert_eq!(f32_of(&new, "level"), None);
    }

    /// A session saved before ENTROPY, with ANALOG's amount, opens with ENTROPY at its
    /// default, not at what the instance had (R14, R18); one saved since keeps its own.
    #[test]
    fn an_old_session_opens_with_entropy_at_its_default() {
        let state = |params: &[(&str, f32)]| PluginState {
            version: String::new(),
            params: params
                .iter()
                .map(|&(k, v)| (k.to_owned(), ParamValue::F32(v)))
                .collect(),
            fields: Default::default(),
        };
        let entropy = |s: &PluginState| match s.params.get("entropy") {
            Some(ParamValue::F32(v)) => Some(*v),
            _ => None,
        };
        let mut old = state(&[("analog", 70.0), ("cutoff", 3.0)]);
        Ca72::filter_state(&mut old);
        assert_eq!(entropy(&old), Some(5.0));
        assert!(!old.params.contains_key("analog"));
        let mut new = state(&[("entropy", 40.0)]);
        Ca72::filter_state(&mut new);
        assert_eq!(entropy(&new), Some(43.0));
    }

    /// Both old and current project states pass through the real migration hook. A state
    /// resaved by this version must not be shifted a second time.
    #[test]
    fn saved_states_migrate_entropy_by_their_writer_version() {
        for (version, plain, want) in [
            ("0.1.5", 0.0, 5.0),
            ("0.2.6", 40.0, 43.0),
            ("0.2.6", 100.0, 100.0),
            (env!("CARGO_PKG_VERSION"), 0.0, 0.0),
            (env!("CARGO_PKG_VERSION"), 43.0, 43.0),
        ] {
            let mut s = PluginState {
                version: version.to_owned(),
                params: [("entropy".to_owned(), ParamValue::F32(plain))]
                    .into_iter()
                    .collect(),
                fields: Default::default(),
            };
            Ca72::filter_state(&mut s);
            assert!(
                matches!(s.params["entropy"], ParamValue::F32(v) if v == want),
                "{version}: {plain}"
            );
            s.version = env!("CARGO_PKG_VERSION").to_owned();
            Ca72::filter_state(&mut s);
            assert!(matches!(s.params["entropy"], ParamValue::F32(v) if v == want));
        }
    }

    /// Moving into a smaller f32 interval cannot retain every old f32. Bound the forward
    /// rounding explicitly; this example's loss is reported in R-ENTROPY-FLOOR.
    #[test]
    fn fractional_entropy_state_migration_has_a_bounded_rounding_error() {
        for old in [0.000001f32, 24.8161, 51.0091, 30.3158, 99.99999] {
            let mut s = PluginState {
                version: "0.2.6".to_owned(),
                params: [("entropy".to_owned(), ParamValue::F32(old))]
                    .into_iter()
                    .collect(),
                fields: Default::default(),
            };
            Ca72::filter_state(&mut s);
            let ParamValue::F32(new) = s.params["entropy"] else {
                panic!("a float")
            };
            let recovered = crate::engine::entropy_depth(f64::from(new) / 100.0) / 2.0 * 100.0;
            assert!((recovered - f64::from(old)).abs() < 4.1e-6);
            if old == 51.0091 {
                assert_ne!(recovered, f64::from(old));
                eprintln!("legacy {old:.17} %, new {new:.17} %, recovered {recovered:.17} %");
            }
        }
    }

    /// A session 0.3.0's beta saved, its LFOs' RISE (`lfoN_rise`), opens with FADE in so many
    /// seconds, a fade in; one saved since keeps its FADE, whatever a RISE beside it says; the
    /// RISE goes (decisions.md R-LFO-FADE). And what an LFO modulated on RISE, and a MIDI
    /// mapping of it, are FADE's.
    #[test]
    fn a_beta_session_s_rise_opens_as_a_fade_in() {
        let filtered = |params: Vec<(&str, f32)>| {
            let mut s = PluginState {
                version: "0.3.0-beta.1".to_owned(),
                params: params
                    .into_iter()
                    .map(|(k, v)| (k.to_owned(), ParamValue::F32(v)))
                    .collect(),
                fields: Default::default(),
            };
            Ca72::filter_state(&mut s);
            s
        };
        let fade = |s: &PluginState, id: &str| match s.params.get(id) {
            Some(ParamValue::F32(v)) => Some(*v),
            _ => None,
        };
        let s = filtered(vec![("lfo2_rise", 0.9), ("lfo4_rise", 5.0714)]);
        assert_eq!(fade(&s, "lfo2_fade"), Some(0.9));
        assert_eq!(fade(&s, "lfo4_fade"), Some(5.0714));
        assert_eq!(
            fade(&s, "lfo1_fade"),
            None,
            "none named: the parameter's default"
        );
        assert!(lfo::RISE_IDS.iter().all(|r| !s.params.contains_key(*r)));
        let s = filtered(vec![("lfo1_fade", -1.5), ("lfo1_rise", 0.9)]);
        assert_eq!(fade(&s, "lfo1_fade"), Some(-1.5));
        // The parameter at it (as the host sets it, through its travel): a fade in, heard as
        // so many seconds.
        let p = Ca72::default();
        let q = &p.params.lfo(1).fade;
        let v = fade(&filtered(vec![("lfo2_rise", 0.9)]), "lfo2_fade").unwrap();
        assert!(hostless(&p).automate(q.as_ptr(), q.preview_normalized(v)));
        assert!((q.value() - 0.9).abs() < 1e-5);
        let set = lfo::settings(&p.params, 1, lfo::Edges::Soft);
        assert_eq!(set.fade, f64::from(0.9f32));
        // An assignment on RISE, as the patch's state saved it: FADE's, at half the depth.
        p.params.lfo_patch.load(&lfo::Saved {
            sources: vec![lfo::SavedSource {
                source: "macro1".into(),
                to: vec![("lfo3_rise".into(), 0.4)],
                sharp: false,
            }],
            names: vec![],
        });
        assert_eq!(
            p.params
                .lfo_patch
                .depth(lfo::Source::Macro(0), lfo::Target::LfoFade(2)),
            Some(0.2)
        );
        // A MIDI mapping of RISE: FADE's.
        let saved = learn::Saved::from_text(
            r#"{"version":1,"assignments":[{"param":"lfo4_rise","channel":1,"cc":21}]}"#,
        );
        let fade4 = learn::index("lfo4_fade").expect("learnable");
        assert_eq!(saved.0.first().map(|(i, _)| *i), Some(fade4));
    }

    /// A session saved before each note's own expression opens with none of it.
    #[test]
    fn a_session_saved_before_expression_opens_with_none_of_it() {
        // (Decisions.md R-MPE: whatever the instance had; one saved since keeps its own.)
        let mut s = PluginState {
            version: "0.2.9".to_owned(),
            params: [("timbre_filter".to_owned(), ParamValue::F32(-1.5))]
                .into_iter()
                .collect(),
            fields: Default::default(),
        };
        Ca72::filter_state(&mut s);
        for id in ["pressure_mod", "pressure_filter", "timbre_mod"] {
            assert!(
                matches!(s.params.get(id), Some(ParamValue::F32(v)) if *v == 0.0),
                "{id}"
            );
        }
        assert!(matches!(s.params.get("timbre_filter"), Some(ParamValue::F32(v)) if *v == -1.5));
    }

    /// A session saved before NOTE PRIORITY and RETRIGGER opens with LOW (its index, as nih-plug
    /// sets an enum by one) and RETRIGGER off, the instrument's own, not at what the instance
    /// had (R50); one saved since keeps its own.
    /// A new instance plays at LO (R-LO-DEFAULT); a session saved before QUALITY (0.1's) opens
    /// at HI, as it did until 0.2.4, not at the new default; one saved since keeps its own.
    #[test]
    fn a_new_instance_is_at_lo_and_an_old_session_at_hi() {
        assert_eq!(
            Ca72Params::default().quality.value(),
            params::QualityMode::Lo
        );
        let filtered = |params: Vec<(String, ParamValue)>| {
            let mut s = PluginState {
                version: "0.1.5".to_owned(),
                params: params.into_iter().collect(),
                fields: Default::default(),
            };
            Ca72::filter_state(&mut s);
            s.params.remove("quality")
        };
        let hi = params::QualityMode::Hi.to_index() as i32;
        assert!(matches!(
            filtered(vec![("cutoff".to_owned(), ParamValue::F32(3.0))]),
            Some(ParamValue::I32(i)) if i == hi
        ));
        let lo = ("quality".to_owned(), ParamValue::String("lo".to_owned()));
        assert!(matches!(
            filtered(vec![lo]),
            Some(ParamValue::String(id)) if id == "lo"
        ));
    }

    /// A session saved before FILTER POLES opens at 4, the filter as drawn, whatever the instance
    /// had; one saved since keeps its own, and FILTER MODE's 0.3 HI stays HI (R-POLES).
    #[test]
    fn an_old_session_opens_with_filter_poles_at_four() {
        let filtered = |params: Vec<(String, ParamValue)>, id: &str| {
            let mut s = PluginState {
                version: "0.3.1".to_owned(),
                params: params.into_iter().collect(),
                fields: Default::default(),
            };
            Ca72::filter_state(&mut s);
            s.params.remove(id)
        };
        assert!(matches!(
            filtered(vec![], "filter_poles"),
            Some(ParamValue::I32(4))
        ));
        assert!(matches!(
            filtered(
                vec![("filter_poles".to_owned(), ParamValue::I32(2))],
                "filter_poles"
            ),
            Some(ParamValue::I32(2))
        ));
        let hi = (
            "filter_mode".to_owned(),
            ParamValue::String("hi".to_owned()),
        );
        assert!(matches!(
            filtered(vec![hi], "filter_mode"),
            Some(ParamValue::String(id)) if id == "hi"
        ));
    }

    #[test]
    fn an_old_session_opens_with_note_priority_low_and_no_retrigger() {
        let filtered_as = |params: Vec<(String, ParamValue)>, id: &str| {
            let mut s = PluginState {
                version: String::new(),
                params: params.into_iter().collect(),
                fields: Default::default(),
            };
            Ca72::filter_state(&mut s);
            s.params.remove(id)
        };
        let filtered = |params| filtered_as(params, "note_priority");
        assert!(matches!(
            filtered_as(vec![], "retrigger"),
            Some(ParamValue::Bool(false))
        ));
        assert!(matches!(
            filtered_as(
                vec![("retrigger".to_owned(), ParamValue::Bool(true))],
                "retrigger"
            ),
            Some(ParamValue::Bool(true))
        ));
        let low = params::NotePriority::Low.to_index() as i32;
        assert!(matches!(
            filtered(vec![("cutoff".to_owned(), ParamValue::F32(3.0))]),
            Some(ParamValue::I32(i)) if i == low
        ));
        let high = (
            "note_priority".to_owned(),
            ParamValue::String("high".to_owned()),
        );
        assert!(matches!(
            filtered(vec![high]),
            Some(ParamValue::String(s)) if s == "high"
        ));
    }

    /// A session saved before the arpeggiator opens with it off and its settings at their
    /// defaults, not at what the instance had (R-ARP); one saved since keeps its own.
    #[test]
    fn an_old_session_opens_with_the_arpeggiator_off() {
        let filtered = |params: Vec<(String, ParamValue)>| {
            let mut s = PluginState {
                version: "0.2.6".to_owned(),
                params: params.into_iter().collect(),
                fields: Default::default(),
            };
            Ca72::filter_state(&mut s);
            s.params
        };
        let old = filtered(vec![("cutoff".to_owned(), ParamValue::F32(3.0))]);
        assert!(matches!(old.get("arp_on"), Some(ParamValue::Bool(false))));
        assert!(matches!(
            old.get("arp_latch"),
            Some(ParamValue::Bool(false))
        ));
        assert!(matches!(old.get("arp_octaves"), Some(ParamValue::I32(1))));
        assert!(matches!(old.get("arp_gate"), Some(ParamValue::F32(g)) if *g == 50.0));
        let sixteenth = params::ArpRate::Sixteenth.to_index() as i32;
        assert!(matches!(old.get("arp_rate"), Some(ParamValue::I32(i)) if *i == sixteenth));
        let new = filtered(vec![
            ("arp_on".to_owned(), ParamValue::Bool(true)),
            ("arp_rate".to_owned(), ParamValue::String("1/8t".to_owned())),
        ]);
        assert!(matches!(new.get("arp_on"), Some(ParamValue::Bool(true))));
        assert!(matches!(new.get("arp_rate"), Some(ParamValue::String(s)) if s == "1/8t"));
    }

    /// A session saving the expression's reaches (R-MPE) and the oscillators timed (R-OSC-TIMED)
    /// keeps both as saved: every parameter's id is the plug-in's once.
    #[test]
    fn a_session_keeps_the_reaches_and_the_timed_oscillators_together() {
        // (Every parameter's id once: the expression's reaches, R-MPE, and the oscillators
        // timed, R-OSC-TIMED, both appended after the arpeggiator's.)
        let p = Ca72Params::default();
        let ids: Vec<String> = p.param_map().into_iter().map(|(id, _, _)| id).collect();
        let unique: std::collections::BTreeSet<&String> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len());
        for id in [
            "pressure_mod",
            "pressure_filter",
            "timbre_mod",
            "timbre_filter",
            "osc2_timed",
            "osc2_division",
            "osc3_timed",
            "osc3_division",
        ] {
            assert!(ids.iter().any(|i| i == id), "{id}");
        }
        // A session saving both keeps both as saved.
        let mut s = PluginState {
            version: "0.3.0".to_owned(),
            params: [
                ("pressure_mod".to_owned(), ParamValue::F32(-40.0)),
                ("timbre_filter".to_owned(), ParamValue::F32(1.5)),
                ("osc2_timed".to_owned(), ParamValue::Bool(true)),
                (
                    "osc3_division".to_owned(),
                    ParamValue::String("1/8d".to_owned()),
                ),
            ]
            .into_iter()
            .collect(),
            fields: Default::default(),
        };
        Ca72::filter_state(&mut s);
        assert!(matches!(s.params.get("pressure_mod"), Some(ParamValue::F32(v)) if *v == -40.0));
        assert!(matches!(s.params.get("timbre_filter"), Some(ParamValue::F32(v)) if *v == 1.5));
        assert!(matches!(
            s.params.get("osc2_timed"),
            Some(ParamValue::Bool(true))
        ));
        assert!(
            matches!(s.params.get("osc3_division"), Some(ParamValue::String(v)) if v == "1/8d")
        );
    }

    /// What the LFOs modulate is kept with the session (its `lfo` field) and read back the
    /// same; a session saved before the LFOs opens with none, whatever the instance had
    /// (decisions.md R-LFO).
    #[test]
    fn a_session_keeps_what_the_lfos_modulate_and_an_old_one_none() {
        use crate::lfo::{Edges, Source, Target};
        let p = Ca72Params::default();
        p.lfo_patch
            .assign(Source::Lfo(0), Target::Cutoff, 0.4)
            .unwrap();
        p.lfo_patch
            .assign(Source::Lfo(2), Target::Osc1Shape, -0.75)
            .unwrap();
        p.lfo_patch.set_edges(3, Edges::Sharp);
        let fields = p.serialize_fields();
        assert!(fields.contains_key(lfo::STATE_KEY));
        let q = Ca72Params::default();
        q.lfo_patch
            .assign(Source::Lfo(1), Target::Drive, 0.1)
            .unwrap();
        q.deserialize_fields(&fields);
        assert_eq!(q.lfo_patch.saved(), p.lfo_patch.saved());
        let mut s = PluginState {
            version: "0.2.8".to_owned(),
            params: Default::default(),
            fields: Default::default(),
        };
        Ca72::filter_state(&mut s);
        q.deserialize_fields(&s.fields);
        assert!(Source::all().all(|x| q.lfo_patch.count(x) == 0));
        assert!((0..lfo::LFOS).all(|l| q.lfo_patch.edges(l) == Edges::Soft));
        // (One saved since keeps its own through the filter.)
        let mut s = PluginState {
            version: "0.3.0".to_owned(),
            params: Default::default(),
            fields: fields.into_iter().collect(),
        };
        Ca72::filter_state(&mut s);
        q.deserialize_fields(&s.fields);
        assert_eq!(q.lfo_patch.saved(), p.lfo_patch.saved());
    }

    /// GLIDE RANGE survives a session's serialisation, while a session from before it opens
    /// NORMAL, whatever the instance's range was (R-GLIDE-LONG).
    #[test]
    fn a_session_keeps_glide_range_and_an_old_one_opens_normal() {
        for range in [None, Some("normal"), Some("long")] {
            let s = PluginState {
                version: "0.3.0".to_owned(),
                params: range
                    .into_iter()
                    .map(|v| ("glide_range".to_owned(), ParamValue::String(v.to_owned())))
                    .collect(),
                fields: Default::default(),
            };
            let mut s: PluginState =
                serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();
            Ca72::filter_state(&mut s);
            match (range, s.params.get("glide_range")) {
                (None, Some(ParamValue::I32(0))) => {}
                (Some(want), Some(ParamValue::String(got))) => assert_eq!(want, got),
                other => panic!("range was not restored: {other:?}"),
            }
        }
        let p = Ca72Params::default();
        assert_eq!(p.glide_range.value(), params::GlideRange::Normal);
        assert!(!p.controls().panel.glide_long);
        assert!(p.param_map().iter().any(|(id, _, _)| id == "glide_range"));
        let i = learn::index("glide_range").expect("learnable");
        assert_eq!(learn::LEARNABLE[i].kind, learn::Kind::Switch);
    }

    /// A session saved before the oscillators timed opens with each off and its DIVISION at a
    /// quarter note, not as the instance had them (R-OSC-TIMED); one saved since keeps its own.
    #[test]
    fn an_old_session_opens_with_the_oscillators_free() {
        let filtered = |params: Vec<(String, ParamValue)>| {
            let mut s = PluginState {
                version: "0.2.8".to_owned(),
                params: params.into_iter().collect(),
                fields: Default::default(),
            };
            Ca72::filter_state(&mut s);
            s.params
        };
        let old = filtered(vec![(
            "osc3_range".to_owned(),
            ParamValue::String("lo".into()),
        )]);
        let quarter = params::Division::Quarter.to_index() as i32;
        for n in 2..=3 {
            let timed = old.get(&format!("osc{n}_timed"));
            assert!(matches!(timed, Some(ParamValue::Bool(false))), "{n}");
            let division = old.get(&format!("osc{n}_division"));
            assert!(
                matches!(division, Some(ParamValue::I32(i)) if *i == quarter),
                "{n}"
            );
        }
        let new = filtered(vec![
            ("osc2_timed".to_owned(), ParamValue::Bool(true)),
            (
                "osc2_division".to_owned(),
                ParamValue::String("1/16t".to_owned()),
            ),
        ]);
        assert!(matches!(
            new.get("osc2_timed"),
            Some(ParamValue::Bool(true))
        ));
        assert!(matches!(new.get("osc2_division"), Some(ParamValue::String(s)) if s == "1/16t"));
        assert!(matches!(
            new.get("osc3_timed"),
            Some(ParamValue::Bool(false))
        ));
        assert!(!new.contains_key("osc1_timed"));
    }

    /// `n` blocks of `len` with the host's transport playing from beat `from` at `tempo` (4/4),
    /// the events pushed for each block first (`events(block)`): every left sample.
    fn transport_render(
        p: &mut Ca72,
        c: &mut TestProcessContext<Ca72>,
        (n, len): (usize, usize),
        (tempo, from): (f64, f64),
        mut events: impl FnMut(usize, &mut TestProcessContext<Ca72>),
    ) -> Vec<f32> {
        let mut out = Vec::new();
        for b in 0..n {
            let beats = from + (b * len) as f64 * tempo / 60.0 / 48_000.0;
            c.set_transport(true, Some(tempo), Some(beats), Some((4, 4)));
            events(b, c);
            out.extend(block(p, c, len));
        }
        out
    }

    /// The arpeggiator's notes go through the engine exactly as played ones: a chord held with it
    /// on at 1/16, UP, GATE 50 %, the transport playing at 120, renders to the bit what the same
    /// keys pressed and let go at its steps' samples render with it off; in blocks of 256 and
    /// 1000, offline (every voice waited for: `offline`). MONO with RETRIGGER, and POLY.
    #[test]
    fn arpeggiated_notes_play_through_the_engine_as_played_ones() {
        // (POLY's workers come from the process's budget: one at a time with the tests that
        // count it.)
        let _one = BUDGET_TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (poly, retrigger) in [(false, true), (true, false)] {
            for len in [256, 1000] {
                let (mut a, mut b) = (initialised(poly), initialised(poly));
                for p in [&mut a, &mut b] {
                    p.params
                        .noise_seed
                        .store(0xca72, std::sync::atomic::Ordering::Relaxed);
                    p.engine.prepare(48_000.0, p.params.seed());
                }
                let (mut ca, mut cb) = (offline(&a), offline(&b));
                for (p, c) in [(&a, &ca), (&b, &cb)] {
                    assert!(
                        c.automate(p.params.retrigger.as_ptr(), f32::from(u8::from(retrigger)))
                    );
                }
                assert!(ca.automate(a.params.arp_on.as_ptr(), 1.0));
                let blocks = 48_000 / len;
                // The chord, pressed at sample 0 of the first block (beat 0, a bar's start).
                let chord = [60u8, 64, 67];
                let arp = transport_render(&mut a, &mut ca, (blocks, len), (120.0, 0.0), |k, c| {
                    if k == 0 {
                        for key in chord {
                            c.push_event(note(0, 0, key, true));
                        }
                    }
                });
                // The same as played: a key every 6000 samples, let go 3168 after (GATE 50 % of
                // what lies between the shortest note, 960, and the shortest gap, 624).
                let played =
                    transport_render(&mut b, &mut cb, (blocks, len), (120.0, 0.0), |k, c| {
                        let (from, to) = (k * len, (k + 1) * len);
                        let mut evs = Vec::new();
                        for s in 0..8usize {
                            let key = chord[s % 3];
                            evs.push((s * 6000, key, true));
                            evs.push((s * 6000 + 3168, key, false));
                        }
                        evs.sort_by_key(|e| (e.0, e.2));
                        for (at, key, on) in evs {
                            if (from..to).contains(&at) {
                                c.push_event(note((at - from) as u32, 0, key, on));
                            }
                        }
                    });
                assert!(arp.iter().any(|v| v.abs() > 1e-3), "it sounds");
                assert!(
                    arp == played,
                    "poly {poly} retrigger {retrigger}, blocks of {len}: the first difference at {:?}",
                    arp.iter().zip(&played).position(|(x, y)| x != y)
                );
            }
        }
    }

    /// With the arpeggiator on, All Notes Off, a reset and a preset turning it off while it plays
    /// leave no key down in the engine; LATCH off lets the latched chord go.
    #[test]
    fn nothing_is_left_stuck_by_resets_latch_or_presets() {
        let mut p = initialised(false);
        let mut c = hostless(&p);
        assert!(c.automate(p.params.arp_on.as_ptr(), 1.0));
        assert!(c.automate(p.params.arp_latch.as_ptr(), 1.0));
        c.push_event(note(0, 0, 60, true));
        c.push_event(note(10, 0, 64, true));
        c.push_event(note(50, 0, 60, false));
        c.push_event(note(50, 0, 64, false));
        for _ in 0..40 {
            block(&mut p, &mut c, 256);
        }
        assert!(
            p.arp.sounding().is_some() || p.arp.due().is_some(),
            "latched, it plays on"
        );
        // LATCH off: silent at the next block, nothing held.
        assert!(c.automate(p.params.arp_latch.as_ptr(), 0.0));
        block(&mut p, &mut c, 256);
        assert_eq!(p.arp.sounding(), None);
        assert_eq!(p.arp.due(), None);
        assert_eq!(p.engine.sounding_mask(), 0, "no key down in the engine");
        // A chord again, then All Notes Off.
        c.push_event(note(0, 0, 62, true));
        for _ in 0..10 {
            block(&mut p, &mut c, 256);
        }
        c.push_event(cc(3, 0, ALL_NOTES_OFF, 0));
        block(&mut p, &mut c, 256);
        assert_eq!((p.arp.sounding(), p.arp.due()), (None, None));
        assert_eq!(p.engine.sounding_mask(), 0);
        // Held keys and a preset that turns it off: the keys sound as played, and once let go
        // nothing is down.
        c.push_event(note(0, 0, 65, true));
        for _ in 0..10 {
            block(&mut p, &mut c, 256);
        }
        assert!(c.automate(p.params.arp_on.as_ptr(), 0.0));
        block(&mut p, &mut c, 256);
        assert_eq!(p.arp.sounding(), None);
        assert_ne!(p.engine.sounding_mask(), 0, "the key held sounds as played");
        c.push_event(note(0, 0, 65, false));
        block(&mut p, &mut c, 256);
        assert_eq!(p.engine.sounding_mask(), 0);
        // A reset with it on and a key held.
        assert!(c.automate(p.params.arp_on.as_ptr(), 1.0));
        c.push_event(note(0, 0, 70, true));
        block(&mut p, &mut c, 256);
        p.reset();
        assert_eq!((p.arp.sounding(), p.arp.due()), (None, None));
        block(&mut p, &mut c, 256);
        assert_eq!(p.engine.sounding_mask(), 0);
    }

    /// HOLD with the arpeggiator off holds the keys as played (the owner's drone, 2026-10-19):
    /// a chord pressed and let go, then a new chord, renders to the bit what the same keys held
    /// by hand render, the first let go as the second is pressed; in MONO with RETRIGGER, MONO
    /// with GLIDE and HIGH priority, POLY and UNISON, in blocks of 256 and 1000, offline (every
    /// voice waited for: `offline`).
    #[test]
    fn a_held_drone_plays_through_the_engine_as_held_keys() {
        // (POLY's workers come from the process's budget: one at a time with the tests that
        // count it.)
        let _one = BUDGET_TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let cases: [(&str, &[(&str, f32)]); 4] = [
            ("mono retrigger", &[("retrigger", 1.0)]),
            (
                "mono glide high",
                &[("glide_on", 1.0), ("note_priority", 1.0)],
            ),
            ("poly", &[("poly", 1.0)]),
            ("unison", &[("unison", 1.0)]),
        ];
        for (name, set) in cases {
            for len in [256, 1000] {
                let (mut a, mut b) = (initialised(false), initialised(false));
                for p in [&mut a, &mut b] {
                    p.params
                        .noise_seed
                        .store(0xca72, std::sync::atomic::Ordering::Relaxed);
                    p.engine.prepare(48_000.0, p.params.seed());
                }
                let (mut ca, mut cb) = (offline(&a), offline(&b));
                for (p, c) in [(&a, &ca), (&b, &cb)] {
                    let map = p.params.param_map();
                    for (id, v) in set {
                        let ptr = map.iter().find(|(i, ..)| i == id).expect("a parameter").1;
                        assert!(c.automate(ptr, *v));
                    }
                }
                assert!(ca.automate(a.params.arp_latch.as_ptr(), 1.0));
                // Held by HOLD: 60, 64, 67 pressed at 0 and let go at 3000; 62 and 65 at 24 000,
                // let go at 25 000. By hand: held until 24 000, then 62 and 65 held on.
                let events = |held: bool| {
                    let mut e: Vec<(usize, u8, bool)> = vec![
                        (0, 60, true),
                        (0, 64, true),
                        (0, 67, true),
                        (24_000, 62, true),
                        (24_000, 65, true),
                    ];
                    if held {
                        e.extend([(3000, 60, false), (3000, 64, false), (3000, 67, false)]);
                        e.extend([(25_000, 62, false), (25_000, 65, false)]);
                    } else {
                        e.extend([
                            (24_000, 60, false),
                            (24_000, 64, false),
                            (24_000, 67, false),
                        ]);
                    }
                    e.sort_by_key(|e| (e.0, e.2));
                    e
                };
                let render = |p: &mut Ca72, c: &mut TestProcessContext<Ca72>, e: Vec<_>| {
                    let mut out = Vec::new();
                    for k in 0..48_000 / len {
                        let (from, to) = (k * len, (k + 1) * len);
                        for &(at, key, on) in &e {
                            if (from..to).contains(&at) {
                                c.push_event(note((at - from) as u32, 0, key, on));
                            }
                        }
                        out.extend(block(p, c, len));
                    }
                    out
                };
                let drone = render(&mut a, &mut ca, events(true));
                let held = render(&mut b, &mut cb, events(false));
                assert!(
                    drone[30_000..].iter().any(|v| v.abs() > 1e-3),
                    "{name}: it sounds"
                );
                assert!(
                    drone == held,
                    "{name}, blocks of {len}: the first difference at {:?}",
                    drone.iter().zip(&held).position(|(x, y)| x != y)
                );
            }
        }
    }

    /// A drone (HOLD, the arpeggiator off) leaves no key down in the engine after HOLD off, All
    /// Notes Off, PANIC, a preset turning HOLD off, a reset, or ON switched on and off while it
    /// holds; the sustain pedal under it changes none of that.
    #[test]
    fn a_drone_leaves_nothing_stuck() {
        // (POLY's workers come from the process's budget: one at a time with the tests that
        // count it.)
        let _one = BUDGET_TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut p = initialised(false);
        let mut c = hostless(&p);
        let chord = |c: &mut TestProcessContext<Ca72>, keys: &[u8]| {
            for &k in keys {
                c.push_event(note(0, 0, k, true));
            }
            for &k in keys {
                c.push_event(note(20, 0, k, false));
            }
        };
        let run = |p: &mut Ca72, c: &mut TestProcessContext<Ca72>, n: usize| {
            for _ in 0..n {
                block(p, c, 256);
            }
        };
        assert!(c.automate(p.params.arp_latch.as_ptr(), 1.0));
        // HOLD off.
        chord(&mut c, &[60, 64]);
        run(&mut p, &mut c, 20);
        assert_ne!(p.engine.sounding_mask(), 0, "held");
        assert!(c.automate(p.params.arp_latch.as_ptr(), 0.0));
        run(&mut p, &mut c, 1);
        assert_eq!(p.engine.sounding_mask(), 0, "HOLD off");
        // All Notes Off, then a drone again.
        assert!(c.automate(p.params.arp_latch.as_ptr(), 1.0));
        chord(&mut c, &[62]);
        run(&mut p, &mut c, 10);
        c.push_event(cc(3, 0, ALL_NOTES_OFF, 0));
        run(&mut p, &mut c, 1);
        assert_eq!(p.engine.sounding_mask(), 0, "All Notes Off");
        chord(&mut c, &[63]);
        run(&mut p, &mut c, 10);
        assert_ne!(p.engine.sounding_mask(), 0, "playable after it");
        // PANIC.
        p.local_notes.panic();
        run(&mut p, &mut c, 1);
        assert_eq!(p.engine.sounding_mask(), 0, "PANIC");
        assert!(c.automate(p.params.arp_latch.as_ptr(), 0.0));
        run(&mut p, &mut c, 1);
        assert_eq!(p.engine.sounding_mask(), 0, "and HOLD off after it");
        // A preset (every arpeggiator setting at its default: off, HOLD off).
        assert!(c.automate(p.params.arp_latch.as_ptr(), 1.0));
        chord(&mut c, &[65, 69]);
        run(&mut p, &mut c, 10);
        assert!(c.automate(p.params.arp_latch.as_ptr(), 0.0));
        assert!(c.automate(p.params.arp_on.as_ptr(), 0.0));
        run(&mut p, &mut c, 1);
        assert_eq!(p.engine.sounding_mask(), 0, "a preset");
        // A reset.
        assert!(c.automate(p.params.arp_latch.as_ptr(), 1.0));
        chord(&mut c, &[67]);
        run(&mut p, &mut c, 10);
        p.reset();
        run(&mut p, &mut c, 1);
        assert_eq!(p.engine.sounding_mask(), 0, "a reset");
        assert!(c.automate(p.params.arp_latch.as_ptr(), 0.0));
        run(&mut p, &mut c, 1);
        assert_eq!(p.engine.sounding_mask(), 0);
        // ON on and off while it holds: arpeggiated, then held again, then let go by HOLD off.
        assert!(c.automate(p.params.arp_latch.as_ptr(), 1.0));
        chord(&mut c, &[60, 67]);
        run(&mut p, &mut c, 10);
        assert!(c.automate(p.params.arp_on.as_ptr(), 1.0));
        run(&mut p, &mut c, 40);
        assert!(p.arp.due().is_some(), "arpeggiated");
        assert!(c.automate(p.params.arp_on.as_ptr(), 0.0));
        run(&mut p, &mut c, 10);
        assert_eq!((p.arp.sounding(), p.arp.due()), (None, None));
        assert_ne!(p.engine.sounding_mask(), 0, "held again");
        assert!(c.automate(p.params.arp_latch.as_ptr(), 0.0));
        run(&mut p, &mut c, 1);
        assert_eq!(p.engine.sounding_mask(), 0, "ON on and off");
        // The pedal down under a drone, a chord let go under it, the pedal up, HOLD off.
        assert!(c.automate(p.params.arp_latch.as_ptr(), 1.0));
        c.push_event(cc(0, 0, arp::SUSTAIN, 127));
        chord(&mut c, &[60, 64]);
        run(&mut p, &mut c, 4);
        chord(&mut c, &[67]);
        run(&mut p, &mut c, 4);
        c.push_event(cc(0, 0, arp::SUSTAIN, 0));
        run(&mut p, &mut c, 4);
        assert_ne!(p.engine.sounding_mask(), 0, "held after the pedal");
        assert!(c.automate(p.params.arp_latch.as_ptr(), 0.0));
        run(&mut p, &mut c, 1);
        assert_eq!(p.engine.sounding_mask(), 0, "the pedal");
        // In POLY: every voice let go (their release tails ended).
        let mut q = initialised(true);
        let mut c = hostless(&q);
        assert!(c.automate(q.params.arp_latch.as_ptr(), 1.0));
        chord(&mut c, &[60, 64, 67]);
        run(&mut q, &mut c, 20);
        assert!(c.automate(q.params.arp_latch.as_ptr(), 0.0));
        run(&mut q, &mut c, 400);
        assert_eq!(q.engine.sounding_mask(), 0, "POLY");
    }

    /// The sustain pedal (CC 64) holds the keys let go while the arpeggiator plays; learned by
    /// MIDI Learn, it is the learned parameter's alone.
    #[test]
    fn the_sustain_pedal_reaches_the_arpeggiator_unless_learned() {
        let mut p = initialised(false);
        let mut c = hostless(&p);
        assert!(c.automate(p.params.arp_on.as_ptr(), 1.0));
        c.push_event(cc(0, 0, arp::SUSTAIN, 127));
        c.push_event(note(1, 0, 60, true));
        c.push_event(note(5, 0, 60, false));
        for _ in 0..40 {
            block(&mut p, &mut c, 256);
        }
        assert!(p.arp.due().is_some(), "held by the pedal");
        c.push_event(cc(0, 0, arp::SUSTAIN, 0));
        block(&mut p, &mut c, 256);
        assert_eq!((p.arp.sounding(), p.arp.due()), (None, None));
        p.params
            .midi_map
            .assign(learnable("cutoff"), on(0, arp::SUSTAIN));
        c.push_event(cc(0, 0, arp::SUSTAIN, 127));
        c.push_event(note(1, 0, 60, true));
        c.push_event(note(5, 0, 60, false));
        block(&mut p, &mut c, 256);
        assert_eq!(p.params.cutoff.value(), 5.0);
        // (The note let go after its shortest, 20 ms: not held on by a pedal.)
        for _ in 0..4 {
            block(&mut p, &mut c, 256);
        }
        assert_eq!((p.arp.sounding(), p.arp.due()), (None, None), "not a pedal");
    }

    /// The sustain pedal is the arpeggiator's only while nothing else has CC 64: the MOD wheel's
    /// source (R-MODWHEEL-SOURCE) and a preset action learned on it (R-PRESET-KEYS) come first.
    #[test]
    fn the_sustain_pedal_yields_to_the_wheel_source_and_a_preset_action() {
        let mut p = initialised(false);
        let mut c = hostless(&p);
        assert!(c.automate(p.params.arp_on.as_ptr(), 1.0));
        p.params
            .midi_map
            .set_wheel_source(mod_wheel::Source::Cc(arp::SUSTAIN));
        block(&mut p, &mut c, 64);
        c.push_event(cc(0, 0, arp::SUSTAIN, 127));
        c.push_event(note(1, 0, 60, true));
        c.push_event(note(5, 0, 60, false));
        block(&mut p, &mut c, 256);
        assert_eq!(
            p.engine.midi_wheels().1,
            ca72::modulation::midi_wheel(1.0) as f32,
            "the wheel's"
        );
        for _ in 0..4 {
            block(&mut p, &mut c, 256);
        }
        assert_eq!(
            (p.arp.sounding(), p.arp.due()),
            (None, None),
            "the wheel's, not a pedal"
        );
        p.params
            .midi_map
            .set_wheel_source(mod_wheel::Source::DEFAULT);
        c.push_event(cc(0, 0, arp::SUSTAIN, 0));
        block(&mut p, &mut c, 64);
        p.params
            .midi_map
            .assign(learnable("next_preset"), on(0, arp::SUSTAIN));
        // (The plug-in's helper, woken by the press, would take it off the queue to prepare the
        // preset, racing the look at the queue below; it failed in the full suite. The press's
        // way, not the preset, is what is tested here.)
        p.helper = None;
        p.midi(cc(0, 0, arp::SUSTAIN, 127), &mut c);
        assert_eq!(p.preset_actions.requested(), Some(2), "NEXT PRESET's");
        c.push_event(note(1, 0, 60, true));
        c.push_event(note(5, 0, 60, false));
        for _ in 0..5 {
            block(&mut p, &mut c, 256);
        }
        assert_eq!(
            (p.arp.sounding(), p.arp.due()),
            (None, None),
            "the action's, not a pedal"
        );
    }

    /// A key the arpeggiator plays, its note then taken by a preset action (R-PRESET-KEYS), is
    /// let go through the arpeggiator when released: nothing is left playing.
    #[test]
    fn a_note_taken_by_a_preset_action_is_let_go_through_the_arpeggiator() {
        let mut p = initialised(false);
        let mut c = hostless(&p);
        assert!(c.automate(p.params.arp_on.as_ptr(), 1.0));
        c.push_event(note(0, 0, 60, true));
        for _ in 0..8 {
            block(&mut p, &mut c, 256);
        }
        assert!(p.arp.due().is_some(), "arpeggiated");
        p.params.midi_map.assign(
            learnable("next_preset"),
            learn::Binding::Note {
                channel: 0,
                note: 60,
            },
        );
        c.push_event(note(0, 0, 60, false));
        for _ in 0..8 {
            block(&mut p, &mut c, 256);
        }
        assert_eq!((p.arp.sounding(), p.arp.due()), (None, None));
    }

    /// A session's arpeggiator, saved and loaded, is as it was (its parameters are the host's).
    #[test]
    fn the_arpeggiators_settings_are_host_parameters() {
        let p = Ca72::default();
        let ids: Vec<String> = p
            .params
            .param_map()
            .into_iter()
            .map(|(id, ..)| id)
            .collect();
        for id in [
            "arp_on",
            "arp_latch",
            "arp_direction",
            "arp_rate",
            "arp_octaves",
            "arp_gate",
            "arp_swing",
        ] {
            assert!(ids.iter().any(|i| i == id), "{id}");
        }
    }

    /// A session saved with SCATTER's EDGES (no longer offered: R49), or with a placement
    /// not understood, opens with EVEN (its index, as nih-plug sets an enum by one), not at what
    /// the instance had (R18); one saved with EVEN or CENTER keeps its own.
    #[test]
    fn a_session_saved_with_edges_opens_with_even() {
        let filtered = |id: &str| {
            let mut s = PluginState {
                version: String::new(),
                params: [("placement".to_owned(), ParamValue::String(id.to_owned()))].into(),
                fields: Default::default(),
            };
            Ca72::filter_state(&mut s);
            s.params.remove("placement")
        };
        let even = params::Scatter::Even.to_index() as i32;
        for id in ["edges", "sideways"] {
            assert!(
                matches!(filtered(id), Some(ParamValue::I32(i)) if i == even),
                "{id}"
            );
        }
        for id in ["even", "centre"] {
            assert!(
                matches!(filtered(id), Some(ParamValue::String(s)) if s == id),
                "{id}"
            );
        }
        assert_eq!(
            params::Scatter::from_index(even as usize),
            params::Scatter::Even
        );
    }

    /// One block of `len` samples processed as a host calls `process`, with the events pushed to
    /// `c` (stereo, no side chain): its left channel.
    #[allow(unsafe_code)]
    pub(crate) fn block(p: &mut Ca72, c: &mut TestProcessContext<Ca72>, len: usize) -> Vec<f32> {
        stereo_block(p, c, len).0
    }

    #[allow(unsafe_code)]
    fn stereo_block(
        p: &mut Ca72,
        c: &mut TestProcessContext<Ca72>,
        len: usize,
    ) -> (Vec<f32>, Vec<f32>) {
        let (mut l, mut r) = (vec![0.0f32; len], vec![0.0f32; len]);
        let mut buffer = Buffer::default();
        // SAFETY: both slices are `len` long and live until `process` has returned.
        unsafe {
            buffer.set_slices(len, |s| {
                s.clear();
                s.push(&mut l);
                s.push(&mut r);
            });
        }
        let mut aux = AuxiliaryBuffers {
            inputs: &mut [],
            outputs: &mut [],
        };
        p.process(&mut buffer, &mut aux, c);
        drop(buffer);
        (l, r)
    }

    /// A control change as a host gives it (channel 0 to 15, the 7-bit value as 0..1).
    fn cc(timing: u32, channel: u8, cc: u8, value: u8) -> NoteEvent<()> {
        NoteEvent::MidiCC {
            timing,
            channel,
            cc,
            value: f32::from(value) / 127.0,
        }
    }

    fn learnable(id: &str) -> usize {
        learn::index(id).expect("learnable")
    }

    fn on(channel: u8, cc: u8) -> learn::Cc {
        learn::Cc { channel, cc }
    }

    #[test]
    fn preset_pads_trigger_only_on_fresh_presses_and_learning_is_silent() {
        let mut p = Ca72::default();
        let mut c = hostless(&p);
        let map = p.params.midi_map.clone();
        let next = learnable("next_preset");
        map.arm(next);
        p.midi(note(0, 2, 36, false), &mut c);
        assert_eq!(map.armed(), Some(next));
        p.midi(note(0, 2, 36, true), &mut c);
        assert!(
            p.preset_actions.requested().is_none(),
            "learning press never triggers"
        );
        p.midi(note(0, 2, 36, true), &mut c);
        assert_eq!(
            p.note_sources.held(),
            [0, 0],
            "a repeated learning press stays silent before polling"
        );
        map.poll().unwrap();
        for _ in 0..3 {
            p.midi(note(0, 2, 36, true), &mut c);
        }
        assert!(p.preset_actions.requested().is_none(), "held learning note");
        p.midi(note(0, 2, 36, false), &mut c);
        assert!(p.preset_actions.requested().is_none(), "note off");
        p.midi(note(0, 1, 36, true), &mut c);
        assert!(p.preset_actions.requested().is_none(), "another channel");
        p.midi(note(0, 2, 36, true), &mut c);
        assert_eq!(p.preset_actions.requested(), Some(2));
        p.midi(note(0, 2, 36, true), &mut c);
        assert!(p.preset_actions.requested().is_none(), "repeated note on");
        map.arm(learnable("previous_preset"));
        p.midi(cc(0, 3, 20, 63), &mut c);
        assert_eq!(map.armed(), Some(learnable("previous_preset")));
        p.midi(cc(0, 3, 20, 64), &mut c);
        map.poll().unwrap();
        for value in [127, 100, 64] {
            p.midi(cc(0, 3, 20, value), &mut c);
        }
        assert!(p.preset_actions.requested().is_none(), "held learning CC");
        p.midi(cc(0, 3, 20, 0), &mut c);
        assert!(p.preset_actions.requested().is_none(), "CC off");
        p.midi(cc(0, 3, 20, 127), &mut c);
        assert_eq!(p.preset_actions.requested(), Some(1));
        p.midi(cc(0, 3, 20, 127), &mut c);
        assert!(p.preset_actions.requested().is_none());
        p.midi(cc(0, 3, 20, 63), &mut c);
        p.midi(cc(0, 3, 20, 64), &mut c);
        assert_eq!(p.preset_actions.requested(), Some(1), "a new crossing");
        assert!(c.reported.is_empty(), "no parameter CCs were emitted");
    }

    #[test]
    fn preset_actions_prepare_off_audio_and_apply_through_the_host_without_an_editor() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = Ca72::default();
        let library = library::Library::at(dir.path());
        let entries = library.entries().0;
        p.preset_actions = Arc::new(preset_actions::PresetActions::new(
            p.params.clone(),
            library,
        ));
        let mut c = hostless(&p);
        p.params
            .midi_map
            .assign(learnable("next_preset"), on(0, 20));
        for expected in &entries[..2] {
            p.midi(cc(0, 0, 20, 127), &mut c);
            assert!(c.reported.is_empty());
            p.preset_actions.prepare();
            assert!(p.preset_actions.apply(&mut c));
            p.preset_actions.prepare();
            assert_eq!(*p.params.preset.read().unwrap(), expected.sound.name);
            for (id, normalized) in presets::targets(&p.params, expected) {
                let (_, ptr, _) = p
                    .params
                    .param_map()
                    .into_iter()
                    .find(|(key, _, _)| key == id)
                    .unwrap();
                assert!(
                    c.reported
                        .iter()
                        .any(|(q, v, t)| *q == ptr && (*v - normalized).abs() < 1e-5 && *t == 0),
                    "{id}"
                );
            }
            assert!(!p.preset_actions.apply(&mut c), "consumed once");
            c.reported.clear();
            p.midi(cc(0, 0, 20, 0), &mut c);
        }
        p.params
            .midi_map
            .assign(learnable("previous_preset"), on(0, 21));
        p.midi(cc(0, 0, 21, 127), &mut c);
        p.preset_actions.prepare();
        p.preset_actions.apply(&mut c);
        p.preset_actions.prepare();
        assert_eq!(*p.params.preset.read().unwrap(), entries[0].sound.name);
        p.midi(cc(0, 0, 21, 0), &mut c);
        p.midi(cc(0, 0, 21, 127), &mut c);
        p.preset_actions.prepare();
        p.preset_actions.apply(&mut c);
        p.preset_actions.prepare();
        assert_eq!(
            *p.params.preset.read().unwrap(),
            entries.last().unwrap().sound.name,
            "wraps"
        );
    }

    /// A preset a MIDI pad chooses lands in the voices at once, as the drawer's does
    /// (R-PRESET-KEYS, R-KNOB-GLIDE); a knob turned after it glides again.
    #[test]
    fn a_preset_a_pad_chooses_lands_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = initialised(false);
        // (The test does the helper's part, `prepare`, itself: the helper would race it.)
        p.helper = None;
        let library = library::Library::at(dir.path());
        let first = library.entries().0[0].clone();
        p.preset_actions = Arc::new(preset_actions::PresetActions::new(
            p.params.clone(),
            library,
        ));
        let mut c = hostless(&p);
        p.params
            .midi_map
            .assign(learnable("next_preset"), on(0, 20));
        block(&mut p, &mut c, 64);
        let before = p.controls();
        c.push_event(cc(0, 0, 20, 127));
        block(&mut p, &mut c, 64);
        p.preset_actions.prepare();
        block(&mut p, &mut c, 64);
        p.preset_actions.prepare();
        assert_eq!(*p.params.preset.read().unwrap(), first.sound.name);
        assert_ne!(p.params.controls(), before, "a preset of another sound");
        assert!(!p.smooth.moving());
        assert_eq!(p.controls(), p.params.controls());
        assert!(c.automate(p.params.cutoff.as_ptr(), 0.9));
        block(&mut p, &mut c, 64);
        assert!(p.smooth.moving());
    }

    #[test]
    fn queued_preset_presses_keep_the_rails_filtered_order() {
        let dir = tempfile::tempdir().unwrap();
        let p = Ca72::default();
        let library = library::Library::at(dir.path());
        let entries = library.entries().0;
        let chosen = [entries[2].sound.name.clone(), entries[5].sound.name.clone()];
        *p.params.midi_map.preset_list.lock().unwrap() = Some(chosen.to_vec());
        let actions = preset_actions::PresetActions::new(p.params.clone(), library);
        let mut c = hostless(&p);
        actions.request(1);
        actions.request(1);
        actions.prepare();
        actions.prepare();
        assert!(
            p.params.preset.read().unwrap().is_empty(),
            "unapplied values keep their old name"
        );
        assert!(actions.apply(&mut c));
        actions.prepare();
        assert_eq!(*p.params.preset.read().unwrap(), chosen[0]);
        assert!(actions.apply(&mut c));
        actions.prepare();
        assert_eq!(*p.params.preset.read().unwrap(), chosen[1]);
        actions.request(1);
        actions.prepare();
        actions.apply(&mut c);
        actions.prepare();
        assert_eq!(
            *p.params.preset.read().unwrap(),
            chosen[0],
            "wraps within the same filtered set"
        );
    }

    #[test]
    fn a_host_reset_discards_queued_and_prepared_preset_changes() {
        let mut p = Ca72::default();
        let mut c = hostless(&p);
        p.preset_actions.request(1);
        p.preset_actions.prepare();
        assert!(
            p.params.preset.read().unwrap().is_empty(),
            "preparing alone cannot rename the sound"
        );
        p.reset();
        p.preset_actions.apply(&mut c);
        p.preset_actions.prepare();
        assert!(
            c.reported.is_empty(),
            "a prepared patch from before reset is discarded"
        );
        assert!(p.params.preset.read().unwrap().is_empty());
        p.preset_actions.request(1);
        p.reset();
        p.preset_actions.prepare();
        assert!(
            !p.preset_actions.apply(&mut c),
            "an old queued press is discarded"
        );
    }

    fn pressure(channel: u8, note: Option<u8>, value: f32) -> NoteEvent<()> {
        match note {
            Some(note) => NoteEvent::PolyPressure {
                timing: 37,
                voice_id: None,
                channel,
                note,
                pressure: value,
            },
            None => NoteEvent::MidiChannelPressure {
                timing: 37,
                channel,
                pressure: value,
            },
        }
    }

    fn wheel_note(channel: u8, note: u8, on: bool) -> NoteEvent<()> {
        if on {
            NoteEvent::NoteOn {
                timing: 0,
                voice_id: None,
                channel,
                note,
                velocity: 1.0,
            }
        } else {
            NoteEvent::NoteOff {
                timing: 0,
                voice_id: None,
                channel,
                note,
                velocity: 0.0,
            }
        }
    }

    /// A preset pad's note is not a held key: its polyphonic aftertouch never moves the wheel,
    /// while a played key's still does (R-PRESET-KEYS with R-MODWHEEL-SOURCE).
    #[test]
    fn a_preset_pads_aftertouch_does_not_move_the_wheel() {
        use mod_wheel::Source;
        let mut p = initialised(false);
        let mut c = hostless(&p);
        p.params.midi_map.set_wheel_source(Source::PolyPressure);
        p.params.midi_map.assign(
            learnable("next_preset"),
            learn::Binding::Note {
                channel: 9,
                note: 36,
            },
        );
        let send = |p: &mut Ca72, c: &mut TestProcessContext<Ca72>, event| {
            c.push_event(event);
            block(p, c, 64);
            p.engine.midi_wheels().1
        };
        send(&mut p, &mut c, wheel_note(9, 36, true));
        assert_eq!(send(&mut p, &mut c, pressure(9, Some(36), 1.0)), 0.0);
        assert_eq!(p.note_sources.held(), [0, 0], "the pad is not played");
        send(&mut p, &mut c, wheel_note(9, 36, false));
        send(&mut p, &mut c, wheel_note(0, 60, true));
        assert!(send(&mut p, &mut c, pressure(0, Some(60), 0.5)) > 0.0);
        assert_eq!(send(&mut p, &mut c, wheel_note(0, 60, false)), 0.0);
    }

    #[test]
    fn every_allowed_cc_moves_the_wheel_on_every_channel_and_source_changes_clear_it() {
        use mod_wheel::Source;
        let mut p = initialised(false);
        let mut c = hostless(&p);
        for cc_number in (0..128).filter(|&cc| Source::cc(cc).is_some()) {
            p.params.midi_map.set_wheel_source(Source::Cc(cc_number));
            block(&mut p, &mut c, 64);
            assert_eq!(
                p.engine.midi_wheels().1,
                0.0,
                "source changed to CC {cc_number}"
            );
            for channel in 0..16 {
                c.push_event(cc(13, channel, cc_number, 93));
                block(&mut p, &mut c, 64);
                assert_eq!(
                    p.engine.midi_wheels().1,
                    ca72::modulation::midi_wheel(f64::from(93.0f32 / 127.0)) as f32
                );
                c.push_event(cc(0, channel, 121, 0));
                block(&mut p, &mut c, 64);
                assert_eq!(p.engine.midi_wheels().1, 0.0);
            }
        }
        c.push_event(cc(0, 0, 119, 127));
        block(&mut p, &mut c, 64);
        p.params.midi_map.set_wheel_source(Source::ChannelPressure);
        p.params.midi_map.set_wheel_source(Source::Cc(119));
        block(&mut p, &mut c, 64);
        assert_eq!(
            p.engine.midi_wheels().1,
            0.0,
            "away and back between callbacks"
        );
        assert!(c.reported.is_empty(), "no host parameter changed");
        p.params.midi_map.set_wheel_source(Source::ChannelPressure);
        for channel in 0..16 {
            c.push_event(pressure(channel, None, 1.0));
            block(&mut p, &mut c, 64);
            assert_eq!(p.engine.midi_wheels().1, 1.0);
            c.push_event(cc(0, channel, 121, 0));
            block(&mut p, &mut c, 64);
            assert_eq!(p.engine.midi_wheels().1, 0.0);
        }
    }

    #[test]
    fn alternative_wheel_sources_render_identically_to_cc1_at_the_same_samples() {
        use mod_wheel::Source;
        let _one = BUDGET_TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for poly in [false, true] {
            for source in [
                Source::Cc(2),
                Source::Cc(4),
                Source::Cc(11),
                Source::Cc(74),
                Source::ChannelPressure,
                Source::PolyPressure,
            ] {
                let mut reference = initialised_with_seed(poly, Some(42));
                let mut p = initialised_with_seed(poly, Some(42));
                p.params.midi_map.set_wheel_source(source);
                let (mut original, mut c) = (offline(&reference), offline(&p));
                for (params, context) in [(&reference.params, &mut original), (&p.params, &mut c)] {
                    context.automate(params.osc_mod.as_ptr(), 1.0);
                    context.automate(params.filter_mod.as_ptr(), 1.0);
                }
                let mut heard = 0.0;
                for b in 0..12 {
                    if b == 0 {
                        original.push_event(wheel_note(5, 60, true));
                        c.push_event(wheel_note(5, 60, true));
                    }
                    let value = (b * 11 % 128) as u8;
                    original.push_event(cc(37, 5, 1, value));
                    c.push_event(match source {
                        Source::Cc(number) => cc(37, 5, number, value),
                        Source::ChannelPressure => pressure(5, None, f32::from(value) / 127.0),
                        Source::PolyPressure => pressure(5, Some(60), f32::from(value) / 127.0),
                    });
                    // Unselected sources do nothing, including the old CC 1.
                    if source != Source::ChannelPressure {
                        c.push_event(pressure(1, None, 1.0));
                    }
                    c.push_event(cc(40, 1, 1, 127));
                    let was = stereo_block(&mut reference, &mut original, 256);
                    let now = stereo_block(&mut p, &mut c, 256);
                    assert!(
                        was.0
                            .iter()
                            .chain(&was.1)
                            .zip(now.0.iter().chain(&now.1))
                            .all(|(a, b)| a.to_bits() == b.to_bits()),
                        "{source:?}, POLY {poly}, block {b}"
                    );
                    heard += was.0.iter().map(|x| f64::from(x.abs())).sum::<f64>();
                    assert_eq!(p.engine.midi_wheels(), reference.engine.midi_wheels());
                }
                assert!(heard > 0.1, "non-silent comparison");
                assert!(c.reported.is_empty());
            }
        }
    }

    #[test]
    fn poly_aftertouch_takes_the_maximum_of_held_keys_and_never_revives_released_pressure() {
        use mod_wheel::Source;
        let mut p = initialised(false);
        let mut c = hostless(&p);
        p.params.midi_map.set_wheel_source(Source::PolyPressure);
        let send = |p: &mut Ca72, c: &mut TestProcessContext<Ca72>, event| {
            c.push_event(event);
            block(p, c, 64);
            p.engine.midi_wheels().1
        };
        assert_eq!(send(&mut p, &mut c, pressure(0, Some(60), 1.0)), 0.0);
        send(&mut p, &mut c, wheel_note(0, 60, true));
        send(&mut p, &mut c, wheel_note(1, 60, true));
        let lower = send(&mut p, &mut c, pressure(0, Some(60), 0.4));
        assert!(lower > 0.0 && lower < 1.0);
        assert_eq!(send(&mut p, &mut c, pressure(1, Some(60), 1.0)), 1.0);
        assert_eq!(send(&mut p, &mut c, wheel_note(1, 60, false)), lower);
        assert_eq!(send(&mut p, &mut c, pressure(1, Some(60), 1.0)), lower);
        assert_eq!(send(&mut p, &mut c, cc(0, 7, 121, 0)), 0.0);
        let smaller = send(&mut p, &mut c, pressure(0, Some(60), 0.2));
        assert!(
            smaller > 0.0 && smaller < lower,
            "reset discarded old pressures"
        );
        assert_eq!(send(&mut p, &mut c, wheel_note(0, 60, false)), 0.0);
        for reset in [120, 123] {
            send(&mut p, &mut c, wheel_note(2, 64, true));
            send(&mut p, &mut c, pressure(2, Some(64), 1.0));
            assert_eq!(send(&mut p, &mut c, cc(0, 4, reset, 0)), 0.0);
            assert_eq!(send(&mut p, &mut c, pressure(2, Some(64), 1.0)), 0.0);
        }
        send(&mut p, &mut c, wheel_note(3, 67, true));
        send(&mut p, &mut c, pressure(3, Some(67), 1.0));
        p.params.midi_map.set_wheel_source(Source::ChannelPressure);
        block(&mut p, &mut c, 64);
        assert_eq!(p.engine.midi_wheels().1, 0.0);
        p.params.midi_map.set_wheel_source(Source::PolyPressure);
        block(&mut p, &mut c, 64);
        assert_eq!(p.engine.midi_wheels().1, 0.0);
        assert!(
            send(&mut p, &mut c, pressure(3, Some(67), 0.2)) > 0.0,
            "held notes remain eligible after changing source"
        );
        p.reset();
        assert_eq!(send(&mut p, &mut c, pressure(3, Some(67), 1.0)), 0.0);
    }

    /// A learned controller sets its parameter at its event's sample, through the host (the
    /// host told once, at that time; a value the parameter already has, not again), with no
    /// editor open; the parameter is at the new value at once.
    #[test]
    fn a_learned_controller_sets_its_parameter_at_its_sample_through_the_host() {
        let mut p = initialised(false);
        let mut c = hostless(&p);
        p.params.midi_map.assign(learnable("cutoff"), on(0, 74));
        c.push_event(cc(100, 0, 74, 127));
        c.push_event(cc(200, 0, 74, 127));
        block(&mut p, &mut c, 256);
        assert_eq!(p.params.cutoff.value(), 5.0);
        assert_eq!(c.reported, [(p.params.cutoff.as_ptr(), 1.0, 100)]);
        c.push_event(cc(10, 0, 74, 0));
        block(&mut p, &mut c, 256);
        assert_eq!(p.params.cutoff.value(), -5.0);
        assert_eq!(
            c.reported.last(),
            Some(&(p.params.cutoff.as_ptr(), 0.0, 10))
        );
    }

    /// While learning, the next learnable controller is caught and changes nothing; once the
    /// editor has assigned it, it sets the parameter.
    #[test]
    fn the_controller_caught_while_learning_changes_nothing() {
        let mut p = initialised(false);
        let mut c = hostless(&p);
        let map = Arc::clone(&p.params.midi_map);
        map.arm(learnable("emphasis"));
        c.push_event(cc(0, 3, 20, 127));
        c.push_event(cc(64, 3, 20, 100));
        block(&mut p, &mut c, 128);
        assert_eq!(p.params.emphasis.value(), 0.0, "the sound unchanged");
        assert!(c.reported.is_empty());
        assert_eq!(
            map.poll().map(|a| (a.param, a.cc)),
            Some((learnable("emphasis"), on(3, 20).into()))
        );
        c.push_event(cc(5, 3, 20, 127));
        block(&mut p, &mut c, 128);
        assert_eq!(p.params.emphasis.value(), 10.0);
    }

    /// The reserved controllers keep what they did (the modulation wheel moves the wheel, reset
    /// all controllers resets it) and are not learned; a controller on another channel than the
    /// one learned does nothing.
    #[test]
    fn the_reserved_controllers_keep_their_paths_and_channels_are_told_apart() {
        let mut p = initialised(false);
        let mut c = hostless(&p);
        let map = Arc::clone(&p.params.midi_map);
        map.assign(learnable("cutoff"), on(0, 74));
        map.arm(learnable("glide"));
        for reserved in [
            0, 1, 6, 32, 38, 96, 97, 98, 99, 100, 101, 122, 124, 125, 126, 127,
        ] {
            c.push_event(cc(0, 0, reserved, 127));
        }
        block(&mut p, &mut c, 64);
        assert_eq!(p.engine.wheels().1, 1.0, "CC 1 is the modulation wheel");
        assert_eq!(
            map.armed(),
            Some(learnable("glide")),
            "nothing reserved was learned"
        );
        assert_eq!(map.refused(), Some(127));
        c.push_event(cc(0, 0, 121, 0));
        block(&mut p, &mut c, 64);
        assert_eq!(p.engine.wheels(), (0.0, 0.0), "CC 121 reset the wheels");
        map.cancel();
        c.push_event(cc(0, 1, 74, 127));
        block(&mut p, &mut c, 64);
        assert_eq!(
            p.params.cutoff.value(),
            0.0,
            "CC 74 on channel 2 is not channel 1's"
        );
        assert!(c.reported.is_empty());
    }

    /// A switch, a selector and VOICES from their controllers' values.
    #[test]
    fn switches_selectors_and_voices_take_their_controllers_values() {
        let mut p = initialised(false);
        let mut c = hostless(&p);
        let map = Arc::clone(&p.params.midi_map);
        map.assign(learnable("osc2_on"), on(0, 20));
        map.assign(learnable("osc1_range"), on(0, 21));
        map.assign(learnable("voices"), on(0, 22));
        c.push_event(cc(0, 0, 20, 64));
        c.push_event(cc(0, 0, 21, 0));
        c.push_event(cc(0, 0, 22, 127));
        block(&mut p, &mut c, 64);
        assert!(p.params.osc2_on.value());
        assert_eq!(p.params.osc1_range.value(), params::Footage::Lo);
        assert_eq!(p.params.voices.value(), 10);
        c.push_event(cc(0, 0, 20, 63));
        c.push_event(cc(0, 0, 21, 127));
        c.push_event(cc(0, 0, 22, 0));
        block(&mut p, &mut c, 64);
        assert!(!p.params.osc2_on.value());
        assert_eq!(p.params.osc1_range.value(), params::Footage::R2);
        assert_eq!(p.params.voices.value(), 2);
    }

    /// A switch learns a note from the MIDI map mode (decisions.md R-MIDI-MAP): the note that
    /// teaches it changes nothing and is not played; each note-on after it turns the switch over
    /// (its note-off nothing), and none of them plays. Armed as MIDI Learn's menu arms it, a
    /// switch lets a note through to the keys as before; a knob never takes one.
    #[test]
    fn a_switchs_learned_note_turns_it_over_and_is_not_played() {
        let mut p = initialised(false);
        let mut c = hostless(&p);
        let map = Arc::clone(&p.params.midi_map);
        let arp = learnable("arp_on");
        assert!(!p.params.arp_on.value());
        map.arm(arp);
        p.midi(note(0, 0, 60, true), &mut c);
        assert_eq!(map.armed(), Some(arp), "the menu's learning takes no note");
        assert_eq!(p.note_sources.held(), [1 << 60, 0], "played as ever");
        p.midi(note(0, 0, 60, false), &mut c);
        map.arm(learnable("cutoff"));
        map.cancel();
        map.arm_with_notes(learnable("cutoff"));
        p.midi(note(0, 0, 61, true), &mut c);
        assert_eq!(
            map.armed(),
            Some(learnable("cutoff")),
            "a knob takes no note"
        );
        p.midi(note(0, 0, 61, false), &mut c);
        map.arm_with_notes(arp);
        p.midi(note(0, 3, 36, true), &mut c);
        assert_eq!(map.armed(), None, "caught");
        assert_eq!(
            p.note_sources.held(),
            [0, 0],
            "the teaching note is not played"
        );
        assert!(!p.params.arp_on.value(), "nor does it change anything");
        let a = map.poll().expect("assigned");
        assert_eq!(
            a.cc,
            learn::Binding::Note {
                channel: 3,
                note: 36
            }
        );
        p.midi(note(0, 3, 36, false), &mut c);
        for want in [true, false, true] {
            assert!(p.midi(note(0, 3, 36, true), &mut c), "a parameter set");
            assert_eq!(p.params.arp_on.value(), want);
            assert_eq!(p.note_sources.held(), [0, 0]);
            assert!(!p.midi(note(0, 3, 36, false), &mut c));
            assert_eq!(p.params.arp_on.value(), want, "its note-off nothing");
        }
        // (Another channel's is a key again.)
        p.midi(note(0, 2, 36, true), &mut c);
        assert_ne!(p.note_sources.held(), [0, 0]);
    }

    /// A knob a learned controller moves is at its new value at once (the host's, the editor's,
    /// the state's), and the voices glide there over `DEZIP` (10 ms), set every `smooth::STEP`
    /// samples; the host's automation meanwhile glides on from where the voices are, over 30 ms
    /// (decisions.md R-KNOB-GLIDE).
    #[test]
    fn a_knob_a_controller_moves_glides_in_the_voices_alone() {
        let mut p = initialised(false);
        let mut c = hostless(&p);
        p.params.midi_map.assign(learnable("cutoff"), on(0, 74));
        let at_rest = p.controls().panel.cutoff;
        assert_eq!(at_rest, p.params.controls().panel.cutoff);
        c.push_event(cc(0, 0, 74, 127));
        block(&mut p, &mut c, 128);
        let voices = p.controls().panel.cutoff;
        let target = p.params.controls().panel.cutoff;
        assert_eq!(target, 1.0, "the parameter at once");
        assert!(
            voices > at_rest && voices < target,
            "{at_rest} {voices} {target}"
        );
        // 480 samples at 48 kHz: there, exactly.
        block(&mut p, &mut c, 352);
        assert_eq!(p.controls(), p.params.controls());
        assert!(!p.smooth.moving());
        // Automation meanwhile: on from where the voices are, over 30 ms.
        c.push_event(cc(0, 0, 74, 0));
        block(&mut p, &mut c, 64);
        assert!(p.smooth.moving());
        let there = p.controls().panel.cutoff;
        assert!(c.automate(p.params.cutoff.as_ptr(), 0.75));
        block(&mut p, &mut c, 32);
        let gliding = p.controls().panel.cutoff;
        // (From where the controller's glide had it, on toward 0.75 and not yet there.)
        assert!(
            there > 0.75 && gliding < there && gliding > 0.75,
            "{there} {gliding}"
        );
        for _ in 0..45 {
            block(&mut p, &mut c, 32);
        }
        assert!(!p.smooth.moving());
        assert_eq!(p.controls(), p.params.controls());
        assert_eq!(p.params.cutoff.value(), 2.5);
        // Switches and selectors do not glide.
        p.params.midi_map.assign(learnable("osc1_range"), on(0, 21));
        c.push_event(cc(0, 0, 21, 127));
        block(&mut p, &mut c, 64);
        assert!(!p.smooth.moving());
        assert_eq!(p.controls(), p.params.controls());
    }

    /// Why a knob a learned controller moves glides (decisions.md R34), measured, by hand
    /// (`cargo test --release -p ca72-plugin --lib a_controllers_steps -- --ignored
    /// --nocapture`): a held A2 on a sawtooth, a knob turned in a quarter of a second three ways:
    /// by the host's automation every 32 samples (the reference), by a 7-bit controller with the
    /// voices following each step at once, and by the same controller as the plug-in plays it
    /// (gliding). The energy above 8 kHz, where the note has little, and the whole, against the
    /// reference's; each knob from and to on its dial, with EMPHASIS where given.
    #[test]
    #[ignore]
    fn a_controllers_steps() {
        use ca72_analysis::fft::{hann, power};
        const RATE: usize = 48_000;
        const STEP: usize = 32;
        let render = |id: &str, from: f64, to: f64, emphasis: f32, how: usize| -> Vec<f32> {
            let mut p = initialised(false);
            let mut c = hostless(&p);
            if how < 2 {
                // (The voices following each step at once, the automation's and the
                // controller's: glides one sample long, R34's measurement as it was made.)
                p.smooth.prepare(1.0 / learn::DEZIP, 1, &p.params);
            }
            let i = learnable(id);
            p.params.midi_map.assign(i, on(0, 74));
            let params = Arc::clone(&p.params);
            let knob = learn::knob_param(&params, i).expect("a knob");
            let ptr = knob.as_ptr();
            let travel = |v: f64| knob.preview_normalized(v as f32);
            c.automate(params.emphasis.as_ptr(), emphasis / 10.0);
            c.automate(params.contour_amount.as_ptr(), 0.0);
            c.automate(ptr, travel(from));
            c.push_event(NoteEvent::NoteOn {
                timing: 0,
                voice_id: None,
                channel: 0,
                note: 45,
                velocity: 1.0,
            });
            for _ in 0..(RATE / 4 / STEP) {
                block(&mut p, &mut c, STEP);
            }
            let mut out = Vec::new();
            let mut sent = None;
            for k in 0..(RATE / 2 / STEP) {
                let t = ((k * STEP) as f64 / RATE as f64 / 0.25).min(1.0);
                let n = travel(from + (to - from) * t);
                if how == 0 {
                    c.automate(ptr, n);
                } else {
                    let v = (f64::from(n) * 127.0).round() as u8;
                    if sent != Some(v) {
                        c.push_event(cc(0, 0, 74, v));
                        sent = Some(v);
                    }
                }
                out.extend(block(&mut p, &mut c, STEP));
            }
            out
        };
        let band = |x: &[f32]| -> (f64, f64) {
            let n = 2048;
            let w = hann(n);
            let (mut high, mut all) = (0.0, 0.0);
            for frame in x.windows(n).step_by(n / 2) {
                let f: Vec<f64> = frame.iter().map(|&s| f64::from(s)).collect();
                for (k, e) in power(&f, &w).iter().enumerate() {
                    all += e;
                    if k as f64 * RATE as f64 / n as f64 >= 8_000.0 {
                        high += e;
                    }
                }
            }
            (high, all)
        };
        let db = |a: f64, b: f64| 10.0 * (a / b).log10();
        for (id, from, to, emphasis) in [
            ("cutoff", -1.0, 2.0, 7.0),
            ("cutoff", -1.0, 2.0, 9.5),
            ("volume", 10.0, 3.0, 0.0),
            ("osc1_volume", 8.0, 2.0, 0.0),
            ("emphasis", 2.0, 9.0, 0.0),
            ("tune", -1.0, 1.0, 0.0),
        ] {
            let emphasis = if id == "emphasis" {
                from as f32
            } else {
                emphasis
            };
            let smooth = render(id, from, to, emphasis, 0);
            let (hs, a0) = band(&smooth);
            let mut line = format!(
                "{id} {from} to {to} (EMPHASIS {emphasis}): reference above 8 kHz {:.1} dB of its whole",
                db(hs, a0)
            );
            for (name, how) in [("stepped", 1), ("glided", 2)] {
                let (h, a) = band(&render(id, from, to, emphasis, how));
                line += &format!(
                    "; {name} {:+.2} dB above 8 kHz, {:+.2} dB all",
                    db(h, hs),
                    db(a, a0)
                );
            }
            println!("{line}");
        }
    }

    /// With no controller learned, `process` plays as it did before MIDI Learn (decisions.md
    /// R34), to the bit: the code before replayed here on the engine itself (its controls set at
    /// the block's start, runs of at most 128 samples between events, each event at its
    /// sample), with notes, the wheels, controllers not learned and reserved ones, POLY off and
    /// on (rendered offline: every voice waited for).
    #[test]
    fn with_nothing_learned_the_sound_is_as_before() {
        use crate::engine::Engine;
        let _one = BUDGET_TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        const LEN: usize = 256;
        let before =
            |e: &mut Engine, controls: &crate::engine::Controls, events: &[NoteEvent<()>]| {
                e.set(controls);
                e.set_deadline(None);
                let (mut l, mut r) = (vec![0.0f32; LEN], vec![0.0f32; LEN]);
                let mut it = events.iter().peekable();
                let mut i = 0;
                while i < LEN {
                    while let Some(ev) = it.peek() {
                        if ev.timing() as usize > i {
                            break;
                        }
                        if let Some(x) = event_of(**ev) {
                            e.event(x);
                        }
                        it.next();
                    }
                    let end = it
                        .peek()
                        .map_or(LEN, |ev| ev.timing() as usize)
                        .min(LEN)
                        .min(i + 128);
                    e.render(&[], &mut l[i..end], &mut r[i..end]);
                    i = end;
                }
                for ev in it {
                    if let Some(x) = event_of(*ev) {
                        e.event(x);
                    }
                }
                e.end_block(LEN);
                (l, r)
            };
        for (poly, quality) in [false, true]
            .into_iter()
            .flat_map(|poly| [0.0, 0.5, 1.0].map(|q| (poly, q)))
        {
            let mut p = initialised(poly);
            let params: Arc<dyn Params> = p.params.clone();
            let mut c = TestProcessContext::new(params, 48_000.0, ProcessMode::Offline);
            c.automate(p.params.osc_mod.as_ptr(), 1.0);
            c.automate(p.params.filter_mod.as_ptr(), 1.0);
            c.automate(p.params.quality.as_ptr(), quality);
            let controls = p.params.controls();
            let mut e = Engine::new();
            e.set(&controls);
            e.prepare(48_000.0, p.params.seed());
            let mut heard = 0.0;
            for b in 0..120u32 {
                let note = |timing, note: u8, on: bool| {
                    if on {
                        NoteEvent::NoteOn {
                            timing,
                            voice_id: None,
                            channel: 0,
                            note,
                            velocity: 1.0,
                        }
                    } else {
                        NoteEvent::NoteOff {
                            timing,
                            voice_id: None,
                            channel: 0,
                            note,
                            velocity: 0.0,
                        }
                    }
                };
                let mut events = vec![];
                if b % 12 == 0 {
                    events.push(note(3, 45 + (b / 12 % 7) as u8, true));
                    events.push(note(3, 52 + (b / 12 % 5) as u8, true));
                }
                if b % 12 == 9 {
                    events.push(note(17, 45 + (b / 12 % 7) as u8, false));
                }
                events.push(cc(40, (b % 16) as u8, 74, (b * 7 % 128) as u8));
                events.push(cc(41, 0, 1, (b * 5 % 128) as u8));
                events.push(NoteEvent::MidiPitchBend {
                    timing: 90,
                    channel: 0,
                    value: 0.5 + 0.4 * ((b as f32) * 0.3).sin(),
                });
                events.push(cc(200, 0, 7, 100));
                if b % 50 == 49 {
                    events.push(cc(250, 0, 121, 0));
                    events.push(cc(255, 0, 123, 0));
                }
                for ev in &events {
                    c.push_event(*ev);
                }
                let now = stereo_block(&mut p, &mut c, LEN);
                let was = before(&mut e, &controls, &events);
                heard += was.0.iter().map(|x| f64::from(x.abs())).sum::<f64>();
                assert!(
                    now.0
                        .iter()
                        .chain(&now.1)
                        .zip(was.0.iter().chain(&was.1))
                        .all(|(a, b)| a.to_bits() == b.to_bits()),
                    "POLY {poly}, QUALITY {quality}, block {b}: not the same samples"
                );
            }
            assert!(c.reported.is_empty());
            assert!(heard > 1.0, "POLY {poly}: silence compared");
        }
    }

    /// The host's automation and a learned controller on one parameter: whichever sets it last,
    /// in time, holds it. At one sample the automation is set first (the wrappers split a block
    /// at an automation point and set it before the run's events: PATCHES.md, change 10), so the
    /// controller's value holds from there until the next automation point.
    #[test]
    fn automation_and_a_controller_on_one_parameter_take_turns_in_time() {
        let mut p = initialised(false);
        let mut c = hostless(&p);
        p.params.midi_map.assign(learnable("emphasis"), on(0, 71));
        // A run from an automation point (set first), with the controller at its first sample.
        c.automate(p.params.emphasis.as_ptr(), 0.2);
        c.push_event(cc(0, 0, 71, 127));
        block(&mut p, &mut c, 32);
        assert_eq!(p.params.emphasis.value(), 10.0);
        // The next automation point, later: it holds.
        c.automate(p.params.emphasis.as_ptr(), 0.3);
        block(&mut p, &mut c, 32);
        assert_eq!(p.params.emphasis.value(), 3.0);
        // (The voices glide there over 30 ms: decisions.md R-KNOB-GLIDE.)
        for _ in 0..45 {
            block(&mut p, &mut c, 32);
        }
        assert_eq!(p.controls(), p.params.controls());
        // The controller again, later in a run: from its sample.
        c.push_event(cc(16, 0, 71, 0));
        block(&mut p, &mut c, 32);
        assert_eq!(p.params.emphasis.value(), 0.0);
        assert_eq!(c.reported.last().map(|r| r.2), Some(16));
    }

    /// Two instances, each its own assignments: one's controller does nothing to the other.
    #[test]
    fn each_instance_has_its_own_assignments() {
        let (mut a, mut b) = (initialised(false), initialised(false));
        let (mut ca, mut cb) = (hostless(&a), hostless(&b));
        a.params.midi_map.assign(learnable("cutoff"), on(0, 74));
        b.params.midi_map.assign(learnable("emphasis"), on(0, 74));
        for (p, c) in [(&mut a, &mut ca), (&mut b, &mut cb)] {
            c.push_event(cc(0, 0, 74, 127));
            block(p, c, 64);
        }
        assert_eq!(
            (a.params.cutoff.value(), a.params.emphasis.value()),
            (5.0, 0.0)
        );
        assert_eq!(
            (b.params.cutoff.value(), b.params.emphasis.value()),
            (0.0, 10.0)
        );
        b.params.midi_map.arm(learnable("glide"));
        assert_eq!(a.params.midi_map.armed(), None);
    }

    /// The assignments are saved with the session and come back with it; a session saved before
    /// MIDI Learn, loaded into an instance with assignments, leaves it none; one whose table is
    /// not understood, none either. An armed learning is not saved.
    #[test]
    fn a_session_holds_its_assignments_and_an_old_one_none() {
        let state = |fields: BTreeMap<String, String>| PluginState {
            version: String::new(),
            params: Default::default(),
            fields,
        };
        let a = Ca72::default();
        a.params.midi_map.assign(learnable("cutoff"), on(0, 74));
        a.params.midi_map.arm(learnable("glide"));
        let saved = a.params.serialize_fields();
        assert!(
            saved
                .get(learn::STATE_KEY)
                .is_some_and(|t| !t.contains("glide"))
        );
        let b = Ca72::default();
        let mut s = state(saved);
        Ca72::filter_state(&mut s);
        b.params.deserialize_fields(&s.fields);
        assert_eq!(
            b.params.midi_map.assignment(learnable("cutoff")),
            Some(on(0, 74))
        );
        assert_eq!(b.params.midi_map.armed(), None);
        // Older, without one: none.
        let mut old = state(BTreeMap::from([(
            "preset".to_owned(),
            "\"Bass\"".to_owned(),
        )]));
        Ca72::filter_state(&mut old);
        b.params.deserialize_fields(&old.fields);
        assert!(b.params.midi_map.assignments().iter().all(Option::is_none));
        // Not understood: none.
        b.params.midi_map.assign(learnable("cutoff"), on(0, 74));
        let mut bad = state(BTreeMap::from([(
            learn::STATE_KEY.to_owned(),
            "[1, 2".to_owned(),
        )]));
        Ca72::filter_state(&mut bad);
        b.params.deserialize_fields(&bad.fields);
        assert!(b.params.midi_map.assignments().iter().all(Option::is_none));
    }

    /// The plug-in tests that use the process's budget of workers, one at a time (POLY's, and
    /// ULTRA's split crews': the timed oscillators' at ULTRA too, `crate::timed`).
    pub(crate) static BUDGET_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct Init;
    impl InitContext<Ca72> for Init {
        fn plugin_api(&self) -> PluginApi {
            PluginApi::Clap
        }
        fn execute(&self, _: ()) {}
        fn set_latency_samples(&self, _: u32) {}
        fn set_current_voice_capacity(&self, _: u32) {}
    }

    /// A plug-in with `params`, initialised as a host does at `rate` in blocks of 256 (`mode`
    /// as it says).
    pub(crate) fn initialised_with(params: Ca72Params, rate: f32, mode: ProcessMode) -> Ca72 {
        let config = BufferConfig {
            sample_rate: rate,
            min_buffer_size: None,
            max_buffer_size: 256,
            process_mode: mode,
        };
        let mut p = Ca72::with_params(params);
        assert!(p.initialize(&Ca72::AUDIO_IO_LAYOUTS[0], &config, &mut Init));
        p
    }

    /// A plug-in, POLY as `poly`, initialised as a host does at 48 kHz in blocks of 256.
    fn bend(timing: u32, channel: u8, value: f32) -> NoteEvent<()> {
        NoteEvent::MidiPitchBend {
            timing,
            channel,
            value,
        }
    }

    /// RPN `rpn` (its coarse and fine numbers) set to `value` on `channel`, as a controller
    /// sends it: the number, then data entry's coarse value.
    fn rpn(p: &mut Ca72, c: &mut TestProcessContext<Ca72>, channel: u8, rpn: u8, value: u8) {
        c.push_event(cc(0, channel, 101, 0));
        c.push_event(cc(0, channel, 100, rpn));
        c.push_event(cc(0, channel, 6, value));
        block(p, c, 64);
    }

    fn send(p: &mut Ca72, c: &mut TestProcessContext<Ca72>, events: &[NoteEvent<()>]) {
        for &e in events {
            c.push_event(e);
        }
        block(p, c, 64);
    }

    fn mpe_x(pressure: f32, timbre: f32, bend: f32) -> crate::engine::Expression {
        crate::engine::Expression {
            pressure,
            timbre,
            bend,
        }
    }

    /// The MPE Configuration Message makes a lower zone (decisions.md R-MPE): each member
    /// channel's bend (48 semitones by default, RPN 0 changing it), pressure and timbre are its
    /// note's alone, neither the PITCH wheel's nor learned; the master channel's are every
    /// voice's; the zone gone, every channel is ordinary again.
    #[test]
    fn a_configured_zone_makes_each_member_channels_messages_its_notes_own() {
        // (POLY's workers: one such test at a time, R21.)
        let _one = BUDGET_TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut p = initialised(true);
        let mut c = hostless(&p);
        // (Timbres 0..1 from 0, as R-MPE first had them: R-MPE's ZERO.)
        p.params.set_timbre_rest(expression::TimbreRest::Zero);
        p.params.midi_map.assign(learnable("cutoff"), on(1, 74));
        let cutoff = p.params.cutoff.value();
        rpn(&mut p, &mut c, 0, 6, 15);
        assert_eq!(p.expressions.zones(), [15, 0]);
        // MPE's order: the member channel's bend and timbre, then the note.
        send(
            &mut p,
            &mut c,
            &[bend(0, 1, 1.0), cc(0, 1, 74, 127), note(0, 1, 60, true)],
        );
        send(&mut p, &mut c, &[note(0, 2, 64, true), cc(0, 2, 74, 0)]);
        assert_eq!(p.engine.key_expression(60), mpe_x(0.0, 1.0, 48.0));
        assert_eq!(p.engine.key_expression(64), crate::engine::Expression::NONE);
        assert_eq!(p.engine.midi_wheels().0, 0.5, "the PITCH wheel untouched");
        assert_eq!(
            p.params.cutoff.value(),
            cutoff,
            "CC 74 on a member channel is not learned"
        );
        send(&mut p, &mut c, &[pressure(2, None, 0.5)]);
        assert_eq!(p.engine.key_expression(64), mpe_x(0.5, 0.0, 0.0));
        assert_eq!(p.engine.shared_expression(), (0.0, 0.0));
        // A member's bend range, every member's.
        rpn(&mut p, &mut c, 2, 0, 12);
        assert_eq!(p.engine.key_expression(60), mpe_x(0.0, 1.0, 12.0));
        send(&mut p, &mut c, &[bend(0, 1, 0.25)]);
        assert_eq!(p.engine.key_expression(60), mpe_x(0.0, 1.0, -6.0));
        // The master channel is every voice's.
        send(
            &mut p,
            &mut c,
            &[bend(0, 0, 1.0), pressure(0, None, 0.25), cc(0, 0, 74, 127)],
        );
        assert!(
            p.engine.midi_wheels().0 > 0.5,
            "the master's bend is the PITCH wheel's"
        );
        assert_eq!(p.engine.shared_expression(), (0.25, 1.0));
        // A note let go keeps its voice's expression; the next on its channel takes the
        // channel's as it stands.
        send(&mut p, &mut c, &[note(0, 1, 60, false), bend(0, 1, 0.5)]);
        assert_eq!(p.engine.key_expression(60), mpe_x(0.0, 1.0, -6.0));
        send(&mut p, &mut c, &[note(0, 1, 62, true)]);
        assert_eq!(p.engine.key_expression(62), mpe_x(0.0, 1.0, 0.0));
        // The zone gone: channel 2's bend the PITCH wheel's again, and its CC 74 learned.
        rpn(&mut p, &mut c, 0, 6, 0);
        assert_eq!(p.expressions.zones(), [0, 0]);
        send(&mut p, &mut c, &[bend(0, 1, 0.5), bend(0, 2, 0.75)]);
        assert!(p.engine.midi_wheels().0 > 0.5);
        send(&mut p, &mut c, &[cc(0, 1, 74, 0)]);
        assert_ne!(p.params.cutoff.value(), cutoff, "learned again");
    }

    /// An upper zone takes the channels from 15 down; a lower zone configured over it pushes it
    /// back to what is left (MPE's rule); the computer's setting makes a lower zone of fifteen
    /// without the message, or none whatever the controller says.
    #[test]
    fn zones_share_the_channels_and_the_setting_overrides_the_message() {
        // (POLY's workers: one such test at a time, R21.)
        let _one = BUDGET_TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut p = initialised(true);
        let mut c = hostless(&p);
        // (Timbres 0..1 from 0, as R-MPE first had them: R-MPE's ZERO.)
        p.params.set_timbre_rest(expression::TimbreRest::Zero);
        rpn(&mut p, &mut c, 15, 6, 5);
        assert_eq!(p.expressions.zones(), [0, 5]);
        assert!((10..=14).all(|ch| p.expressions.is_member(ch)));
        assert!(!p.expressions.is_member(9) && !p.expressions.is_member(15));
        rpn(&mut p, &mut c, 0, 6, 12);
        assert_eq!(p.expressions.zones(), [12, 2]);
        assert!(p.expressions.is_member(12) && p.expressions.is_member(13));
        p.params.set_mpe_mode(expression::Mode::Off);
        block(&mut p, &mut c, 64);
        assert_eq!(p.expressions.zones(), [0, 0]);
        send(&mut p, &mut c, &[note(0, 3, 60, true), bend(0, 3, 1.0)]);
        assert_eq!(p.engine.key_expression(60), crate::engine::Expression::NONE);
        p.params.set_mpe_mode(expression::Mode::Lower);
        block(&mut p, &mut c, 64);
        assert_eq!(p.expressions.zones(), [15, 0]);
        // (Its bend, sent while the channel was ordinary, was the PITCH wheel's; from now the
        // channel's is its note's.)
        assert_eq!(p.engine.key_expression(60), crate::engine::Expression::NONE);
        send(&mut p, &mut c, &[bend(0, 3, 1.0)]);
        assert_eq!(p.engine.key_expression(60), mpe_x(0.0, 0.0, 48.0));
    }

    /// Without MPE, channel pressure and control change 74 move every voice alike, and
    /// polyphonic aftertouch and a host's note expressions (CLAP's and VST3's pressure, tuning
    /// and brightness) their own note's only, from its attack; a new note starts from none.
    #[test]
    fn ordinary_midi_and_host_note_expressions() {
        // (POLY's workers: one such test at a time, R21.)
        let _one = BUDGET_TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut p = initialised(true);
        let mut c = hostless(&p);
        // (Timbres 0..1 from 0, as R-MPE first had them: R-MPE's ZERO.)
        p.params.set_timbre_rest(expression::TimbreRest::Zero);
        send(
            &mut p,
            &mut c,
            &[note(0, 0, 60, true), note(0, 0, 64, true)],
        );
        send(&mut p, &mut c, &[pressure(3, None, 0.75), cc(0, 9, 74, 64)]);
        assert_eq!(p.engine.shared_expression(), (0.75, 64.0 / 127.0));
        assert_eq!(p.engine.key_expression(60), crate::engine::Expression::NONE);
        let tuning = NoteEvent::PolyTuning {
            timing: 0,
            voice_id: None,
            channel: 0,
            note: 60,
            tuning: -2.5,
        };
        let brightness = NoteEvent::PolyBrightness {
            timing: 0,
            voice_id: None,
            channel: 0,
            note: 60,
            brightness: 0.25,
        };
        send(
            &mut p,
            &mut c,
            &[pressure(0, Some(60), 0.5), tuning, brightness],
        );
        assert_eq!(p.engine.key_expression(60), mpe_x(0.5, 0.25, -2.5));
        assert_eq!(p.engine.key_expression(64), crate::engine::Expression::NONE);
        // Not held: nothing.
        send(&mut p, &mut c, &[pressure(0, Some(67), 1.0)]);
        assert_eq!(p.engine.key_expression(67), crate::engine::Expression::NONE);
        send(
            &mut p,
            &mut c,
            &[note(0, 0, 60, false), note(0, 0, 60, true)],
        );
        assert_eq!(p.engine.key_expression(60), crate::engine::Expression::NONE);
        // Reset All Controllers: pressure and bend at rest, the timbre (RP-015) as it was.
        send(&mut p, &mut c, &[tuning, brightness, cc(0, 4, 121, 0)]);
        assert_eq!(p.engine.key_expression(60), mpe_x(0.0, 0.25, 0.0));
        assert_eq!(p.engine.shared_expression(), (0.0, 64.0 / 127.0));
    }

    /// The MOD wheel's aftertouch (R-MODWHEEL-SOURCE) and each voice's pressure read one record
    /// (R-MPE). Channel aftertouch is outside a zone's members: the master's moves the wheel
    /// and every voice alike, a member's only its note. Polyphonic aftertouch is every note's
    /// own pressure, a member channel's included. One reset rests both.
    #[test]
    fn the_wheels_aftertouch_and_each_voices_pressure_share_one_record() {
        use mod_wheel::Source;
        // (POLY's workers: one such test at a time, R21.)
        let _one = BUDGET_TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut p = initialised(true);
        let mut c = hostless(&p);
        p.params.midi_map.set_wheel_source(Source::ChannelPressure);
        rpn(&mut p, &mut c, 0, 6, 15);
        send(
            &mut p,
            &mut c,
            &[note(0, 1, 60, true), note(0, 2, 64, true)],
        );
        send(&mut p, &mut c, &[pressure(1, None, 0.75)]);
        assert_eq!(
            p.engine.midi_wheels().1,
            0.0,
            "a member's pressure is its note's"
        );
        assert_eq!(p.engine.key_expression(60).pressure, 0.75);
        send(&mut p, &mut c, &[pressure(0, None, 0.5)]);
        let half = ca72::modulation::midi_wheel(0.5) as f32;
        assert_eq!(p.engine.midi_wheels().1, half);
        assert_eq!(p.engine.shared_expression().0, 0.5);
        p.params.midi_map.set_wheel_source(Source::PolyPressure);
        block(&mut p, &mut c, 64);
        assert_eq!(p.engine.midi_wheels().1, 0.0, "source changed");
        // (Channel 2's note's pressure since the source changed; channel 1's from before not.)
        send(&mut p, &mut c, &[pressure(2, None, 0.25)]);
        let wheel = |v: f32| ca72::modulation::midi_wheel(f64::from(v)) as f32;
        assert_eq!(p.engine.midi_wheels().1, wheel(0.25));
        send(&mut p, &mut c, &[pressure(1, None, 0.75)]);
        assert_eq!(
            p.engine.midi_wheels().1,
            wheel(0.75),
            "the highest held note's"
        );
        assert_eq!(p.engine.key_expression(64).pressure, 0.25);
        send(&mut p, &mut c, &[cc(0, 0, 121, 0)]);
        assert_eq!(p.engine.midi_wheels().1, 0.0);
        assert_eq!(p.engine.key_expression(60).pressure, 0.0);
        assert_eq!(p.engine.key_expression(64).pressure, 0.0);
        assert_eq!(p.engine.shared_expression().0, 0.0);
    }

    /// With the wheel on channel aftertouch and PRESSURE's reach on MODULATION, ordinary
    /// MIDI's pressure moves the wheel and adds as much again to each voice through its own
    /// reach (the two add, R-MPE): a voice's wheel the larger, up to its end.
    #[test]
    fn channel_aftertouch_on_the_wheel_and_pressures_reach_add() {
        use mod_wheel::Source;
        // (POLY's workers: one such test at a time, R21.)
        let _one = BUDGET_TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut p = initialised(true);
        let mut c = hostless(&p);
        p.params.midi_map.set_wheel_source(Source::ChannelPressure);
        c.automate(p.params.pressure_mod.as_ptr(), 0.75);
        send(&mut p, &mut c, &[note(0, 0, 60, true)]);
        send(&mut p, &mut c, &[pressure(0, None, 0.25)]);
        for _ in 0..20 {
            block(&mut p, &mut c, 256);
        }
        assert_eq!(
            p.engine.midi_wheels().1,
            ca72::modulation::midi_wheel(0.25) as f32
        );
        // (Reach 50 %: 0.25 more of the wheel's MIDI share at a quarter's pressure.)
        assert_eq!(p.params.pressure_mod.value(), 50.0);
        let k = (0..10)
            .find(|&k| p.engine.voice_wheel(k).is_some())
            .expect("a voice");
        let want = ca72::modulation::midi_wheel(0.25 + 0.5 * f64::from(0.25f32));
        assert!((p.engine.voice_wheel(k).unwrap() - want).abs() < 1e-12);
    }

    /// Ordinary MIDI's pressure, timbre and polyphonic aftertouch, with every reach at 0, leave
    /// the plug-in's output as it was to the bit, the one instrument and POLY.
    #[test]
    fn expression_without_reach_leaves_the_output_as_it_was() {
        let _one = BUDGET_TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for poly in [false, true] {
            let mut was = initialised_with_seed(poly, Some(81));
            let mut now = initialised_with_seed(poly, Some(81));
            let (mut cw, mut cn) = (offline(&was), offline(&now));
            let mut heard = 0.0;
            for b in 0..10u8 {
                if b == 0 {
                    for c in [&mut cw, &mut cn] {
                        c.push_event(note(0, 0, 57, true));
                        c.push_event(note(0, 0, 64, true));
                    }
                }
                cn.push_event(pressure(0, None, f32::from(b) / 10.0));
                cn.push_event(pressure(0, Some(57), f32::from(9 - b) / 10.0));
                cn.push_event(cc(50, 0, 74, b * 12));
                let a = stereo_block(&mut was, &mut cw, 256);
                let n = stereo_block(&mut now, &mut cn, 256);
                assert!(
                    a.0.iter()
                        .chain(&a.1)
                        .zip(n.0.iter().chain(&n.1))
                        .all(|(x, y)| x.to_bits() == y.to_bits()),
                    "POLY {poly}, block {b}"
                );
                heard += a.0.iter().map(|x| f64::from(x.abs())).sum::<f64>();
            }
            assert!(heard > 0.1);
        }
    }

    fn brightness(channel: u8, note: u8, brightness: f32) -> NoteEvent<()> {
        NoteEvent::PolyBrightness {
            timing: 0,
            voice_id: None,
            channel,
            note,
            brightness,
        }
    }

    /// TIMBRE REST at its default, the centre (decisions.md R-MPE): ordinary CC 74 at 64
    /// and a host's brightness at 0.5 are at rest, the output with TIMBRE TO FILTER turned up
    /// the same to the bit as with no timbre at all, the one instrument and POLY; 0 and 127
    /// (brightness 0 and 1) reach -1 and 1, a member channel's CC 74 alike.
    #[test]
    fn a_centred_timbre_rests_at_64_and_half_brightness() {
        let _one = BUDGET_TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for poly in [false, true] {
            let mut was = initialised_with_seed(poly, Some(74));
            let mut now = initialised_with_seed(poly, Some(74));
            assert_eq!(now.params.timbre_rest(), expression::TimbreRest::Centred);
            let (mut cw, mut cn) = (offline(&was), offline(&now));
            for (c, p) in [(&cw, &was), (&cn, &now)] {
                assert!(c.automate(p.params.timbre_filter.as_ptr(), 0.9));
                assert!(c.automate(p.params.timbre_mod.as_ptr(), 1.0));
            }
            let mut heard = 0.0;
            for b in 0..10u32 {
                if b == 0 {
                    for c in [&mut cw, &mut cn] {
                        c.push_event(note(0, 0, 57, true));
                        c.push_event(note(0, 0, 64, true));
                    }
                }
                cn.push_event(cc(b * 20, 0, 74, 64));
                cn.push_event(brightness(0, 57, 0.5));
                let a = stereo_block(&mut was, &mut cw, 256);
                let n = stereo_block(&mut now, &mut cn, 256);
                assert!(
                    a.0.iter()
                        .chain(&a.1)
                        .zip(n.0.iter().chain(&n.1))
                        .all(|(x, y)| x.to_bits() == y.to_bits()),
                    "POLY {poly}, block {b}"
                );
                heard += a.0.iter().map(|x| f64::from(x.abs())).sum::<f64>();
            }
            assert!(heard > 0.1);
            assert_eq!(now.engine.shared_expression().1, 0.0);
            assert_eq!(now.engine.key_expression(57).timbre, 0.0);
            let mut c = cn;
            send(&mut now, &mut c, &[cc(0, 3, 74, 0), brightness(0, 57, 0.0)]);
            assert_eq!(now.engine.shared_expression().1, -1.0);
            assert_eq!(now.engine.key_expression(57).timbre, -1.0);
            send(
                &mut now,
                &mut c,
                &[cc(0, 3, 74, 127), brightness(0, 57, 1.0)],
            );
            assert_eq!(now.engine.shared_expression().1, 1.0);
            assert_eq!(now.engine.key_expression(57).timbre, 1.0);
            // (Either side of 64 by a step: a step of 64 below, of 63 above.)
            send(&mut now, &mut c, &[cc(0, 3, 74, 63)]);
            assert_eq!(now.engine.shared_expression().1, 63.0 / 127.0 * 2.0 - 1.0);
            send(&mut now, &mut c, &[cc(0, 3, 74, 65)]);
            assert!((now.engine.shared_expression().1 - 1.0 / 63.0).abs() < 1e-6);
            // A member channel's CC 74, its note's alike; a member channel that sent none, at rest.
            rpn(&mut now, &mut c, 0, 6, 15);
            send(
                &mut now,
                &mut c,
                &[cc(0, 1, 74, 64), note(0, 1, 60, true), note(0, 2, 62, true)],
            );
            assert_eq!(now.engine.key_expression(60).timbre, 0.0);
            assert_eq!(now.engine.key_expression(62).timbre, 0.0);
            send(&mut now, &mut c, &[cc(0, 1, 74, 0)]);
            assert_eq!(now.engine.key_expression(60).timbre, -1.0);
        }
    }

    /// TIMBRE REST at START (decisions.md R-MPE): every note's timbre is measured from
    /// where it began, a fresh touch at rest whatever its CC 74. An MPE member channel's begins
    /// at the channel's last value before the note, else its first after; a host's brightness
    /// at its first value; ordinary MIDI's shared CC 74 at its value as the phrase began (the
    /// first key with none held), else its first after, until every key is let go.
    #[test]
    fn a_timbre_from_its_notes_start_begins_at_rest() {
        let _one = BUDGET_TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut p = initialised(true);
        let mut c = hostless(&p);
        p.params.set_timbre_rest(expression::TimbreRest::NoteStart);
        let step = |n: i32| n as f32 / 127.0;
        let near = |a: f32, b: f32| (a - b).abs() < 1e-6;
        // MPE: the channel's CC 74 before its note, where the note begins.
        rpn(&mut p, &mut c, 0, 6, 15);
        send(&mut p, &mut c, &[cc(0, 1, 74, 100), note(0, 1, 60, true)]);
        assert_eq!(p.engine.key_expression(60).timbre, 0.0, "a fresh touch");
        send(&mut p, &mut c, &[cc(0, 1, 74, 110)]);
        assert!(near(p.engine.key_expression(60).timbre, step(10)));
        send(&mut p, &mut c, &[cc(0, 1, 74, 20)]);
        assert!(near(p.engine.key_expression(60).timbre, step(-80)));
        // A channel that sent none before its note: its first after is where it began.
        send(&mut p, &mut c, &[note(0, 2, 62, true)]);
        assert_eq!(p.engine.key_expression(62).timbre, 0.0);
        send(&mut p, &mut c, &[cc(0, 2, 74, 30)]);
        assert_eq!(p.engine.key_expression(62).timbre, 0.0);
        send(&mut p, &mut c, &[cc(0, 2, 74, 40)]);
        assert!(near(p.engine.key_expression(62).timbre, step(10)));
        // The next note on channel 1 begins at the channel's value now.
        send(
            &mut p,
            &mut c,
            &[note(0, 1, 60, false), note(0, 1, 64, true)],
        );
        assert_eq!(p.engine.key_expression(64).timbre, 0.0);
        send(
            &mut p,
            &mut c,
            &[note(0, 1, 64, false), note(0, 2, 62, false)],
        );
        // A host's brightness: its first value, at the note's own time, where it began.
        send(
            &mut p,
            &mut c,
            &[note(0, 5, 67, true), brightness(5, 67, 0.8)],
        );
        assert_eq!(p.engine.key_expression(67).timbre, 0.0);
        send(&mut p, &mut c, &[brightness(5, 67, 0.5)]);
        assert!(near(p.engine.key_expression(67).timbre, 0.5 - 0.8));
        send(&mut p, &mut c, &[note(0, 5, 67, false)]);
        // Ordinary CC 74: the phrase's start.
        rpn(&mut p, &mut c, 0, 6, 0);
        send(&mut p, &mut c, &[cc(0, 0, 74, 90)]);
        assert_eq!(p.engine.shared_expression().1, 0.0);
        send(&mut p, &mut c, &[note(0, 0, 48, true), cc(0, 0, 74, 100)]);
        assert!(near(p.engine.shared_expression().1, step(10)));
        send(&mut p, &mut c, &[note(0, 0, 52, true)]);
        assert!(
            near(p.engine.shared_expression().1, step(10)),
            "a key under the phrase begins nothing"
        );
        send(
            &mut p,
            &mut c,
            &[note(0, 0, 48, false), note(0, 0, 52, false)],
        );
        send(&mut p, &mut c, &[note(0, 0, 55, true)]);
        assert_eq!(p.engine.shared_expression().1, 0.0, "a new phrase at rest");
        send(&mut p, &mut c, &[cc(0, 0, 74, 80)]);
        assert!(near(p.engine.shared_expression().1, step(-20)));
        // Turned to ZERO while held, the same notes measured from 0 at once.
        p.params.set_timbre_rest(expression::TimbreRest::Zero);
        block(&mut p, &mut c, 64);
        assert_eq!(p.engine.shared_expression().1, step(80));
    }

    /// TIMBRE REST changed while a note is held (decisions.md R-MPE): the note's and the
    /// shared timbre measured afresh at once; at ZERO, CC 74 is 0..1 from 0 as R-MPE had it.
    #[test]
    fn the_timbre_rest_changed_while_playing_measures_afresh() {
        let _one = BUDGET_TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut p = initialised(true);
        let mut c = hostless(&p);
        p.params.set_timbre_rest(expression::TimbreRest::Zero);
        rpn(&mut p, &mut c, 0, 6, 15);
        send(
            &mut p,
            &mut c,
            &[cc(0, 1, 74, 100), note(0, 1, 60, true), cc(0, 0, 74, 32)],
        );
        assert_eq!(p.engine.key_expression(60).timbre, 100.0 / 127.0);
        assert_eq!(p.engine.shared_expression().1, 32.0 / 127.0);
        p.params.set_timbre_rest(expression::TimbreRest::Centred);
        block(&mut p, &mut c, 64);
        assert!((p.engine.key_expression(60).timbre - 36.0 / 63.0).abs() < 1e-6);
        assert_eq!(p.engine.shared_expression().1, 32.0 / 127.0 * 2.0 - 1.0);
        p.params.set_timbre_rest(expression::TimbreRest::NoteStart);
        block(&mut p, &mut c, 64);
        assert_eq!(p.engine.key_expression(60).timbre, 0.0);
    }

    /// The INPUT screen's counts (decisions.md R-MPE): each note's own pressure, timbre
    /// and bend (a member channel's, polyphonic aftertouch, a host's note expressions) counted
    /// per note; ordinary channel pressure and CC 74 (the master channel's included) as the
    /// channel's; anything else neither.
    #[test]
    fn what_arrives_is_counted_for_the_input_screen() {
        let _one = BUDGET_TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut p = initialised(true);
        let mut c = hostless(&p);
        let heard = p.params.heard.clone();
        send(&mut p, &mut c, &[note(0, 0, 60, true), cc(0, 0, 7, 100)]);
        assert_eq!(heard.counts(), (0, 0));
        send(&mut p, &mut c, &[pressure(0, None, 0.5), cc(0, 4, 74, 9)]);
        assert_eq!(heard.counts(), (0, 2));
        let tuning = NoteEvent::PolyTuning {
            timing: 0,
            voice_id: None,
            channel: 0,
            note: 60,
            tuning: 1.0,
        };
        send(
            &mut p,
            &mut c,
            &[pressure(0, Some(60), 0.5), brightness(0, 60, 0.7), tuning],
        );
        assert_eq!(heard.counts(), (3, 2));
        // MPE: a member's bend, pressure and CC 74 its note's; the master's, the channel's.
        rpn(&mut p, &mut c, 0, 6, 15);
        let (n, ch) = heard.counts();
        send(
            &mut p,
            &mut c,
            &[
                bend(0, 1, 0.75),
                pressure(1, None, 0.5),
                cc(0, 1, 74, 3),
                note(0, 1, 62, true),
                pressure(0, None, 0.25),
                cc(0, 0, 74, 3),
                bend(0, 0, 0.75),
            ],
        );
        assert_eq!(heard.counts(), (n + 3, ch + 2));
    }

    pub(crate) fn initialised(poly: bool) -> Ca72 {
        initialised_with_seed(poly, None)
    }

    fn initialised_with_seed(poly: bool, seed: Option<u64>) -> Ca72 {
        let config = BufferConfig {
            sample_rate: 48_000.0,
            min_buffer_size: None,
            max_buffer_size: 256,
            process_mode: ProcessMode::Realtime,
        };
        let mut p = Ca72::with_params(Ca72Params {
            poly: BoolParam::new("Poly", poly),
            ..Ca72Params::default()
        });
        if let Some(seed) = seed {
            p.params
                .noise_seed
                .store(seed, std::sync::atomic::Ordering::Relaxed);
        }
        assert!(p.initialize(&Ca72::AUDIO_IO_LAYOUTS[0], &config, &mut Init));
        p
    }

    /// The plug-in tells its host one latency, ULTRA's (86 samples at 48 kHz), whatever QUALITY
    /// is when it is initialised, and only then (decisions.md R-LATENCY).
    #[test]
    fn one_latency_is_reported_in_every_quality_mode() {
        struct Told(std::cell::Cell<Vec<u32>>);
        impl InitContext<Ca72> for Told {
            fn plugin_api(&self) -> PluginApi {
                PluginApi::Clap
            }
            fn execute(&self, _: ()) {}
            fn set_latency_samples(&self, samples: u32) {
                let mut told = self.0.take();
                told.push(samples);
                self.0.set(told);
            }
            fn set_current_voice_capacity(&self, _: u32) {}
        }
        let config = BufferConfig {
            sample_rate: 48_000.0,
            min_buffer_size: None,
            max_buffer_size: 256,
            process_mode: ProcessMode::Realtime,
        };
        for mode in [
            crate::params::QualityMode::Lo,
            crate::params::QualityMode::Hi,
            crate::params::QualityMode::Ultra,
        ] {
            let mut p = Ca72::with_params(Ca72Params {
                quality: EnumParam::new("Quality", mode),
                ..Ca72Params::default()
            });
            let mut told = Told(std::cell::Cell::new(Vec::new()));
            assert!(p.initialize(&Ca72::AUDIO_IO_LAYOUTS[0], &config, &mut told));
            assert_eq!(told.0.take(), vec![86]);
        }
    }

    /// Waits up to 10 s for `f` (bounded by what happens, not by time: it fails only if the
    /// helper never gets there).
    fn until(what: &str, mut f: impl FnMut() -> bool) {
        let t = std::time::Instant::now();
        while !f() {
            assert!(t.elapsed() < std::time::Duration::from_secs(10), "{what}");
            std::thread::sleep(std::time::Duration::from_micros(500));
        }
    }

    /// The plug-in's workers through its life, out of the process's budget (decisions.md
    /// R21): initialised with POLY off, it holds none; POLY switched on, its helper (R23),
    /// asked as at the end of a block, starts this machine's share, taken at a block's start;
    /// deactivated, they are back in the budget; initialised with POLY on, it holds them at
    /// once, and gives them back when dropped.
    #[test]
    fn the_workers_follow_poly_through_the_plug_ins_life() {
        let _one = BUDGET_TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let held = || crate::pool::BUDGET.held();
        // (A lone instance's share, at most its cap: R-THREADS. A machine of 1 or 2
        // processors has none to give, one of 3 to 7 one: R11, R39. CI's have 3 or 4.)
        let most = crate::engine::workers_alone();
        let mut p = initialised(false);
        assert_eq!((p.engine.workers(), held()), (0, 0));
        // (POLY as `process` reads it at a block's start.)
        let c = crate::engine::Controls {
            poly: true,
            ..p.params.controls()
        };
        p.engine.set(&c);
        p.engine.set_deadline(None);
        p.ask_helper();
        until("no workers from the helper", || {
            p.engine.set_deadline(None);
            p.engine.workers() == most
        });
        assert_eq!(held(), most);
        p.deactivate();
        assert_eq!((p.engine.workers(), held()), (0, 0));
        let q = initialised(true);
        assert_eq!((q.engine.workers(), held()), (most, most));
        drop(q);
        assert_eq!(held(), 0);
    }

    /// The crash R23 fixes: a plug-in dropped (the host's destroy) while its helper is in a
    /// round of mending and serving waits for it, and once the drop returns no thread of the
    /// plug-in's runs: the helper has ended and the voices' workers have stopped, their share
    /// back in the budget, so the host may unload the plug-in.
    #[test]
    fn a_plug_in_dropped_while_its_helper_works_leaves_no_thread_running() {
        let _one = BUDGET_TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let held = || crate::pool::BUDGET.held();
        let p = initialised(true);
        assert_eq!(held(), p.engine.workers());
        let watch = p.helper.as_ref().map(Helper::watch).unwrap();
        watch.hold(true);
        if let Some(h) = &p.helper {
            h.ask(MEND | SERVE);
        }
        until("the helper's round never began", || watch.held());
        let (tx, rx) = std::sync::mpsc::channel();
        let w = watch.clone();
        let dropper = std::thread::spawn(move || {
            drop(p);
            tx.send((w.ended(), held())).unwrap();
        });
        // (The drop must not return while the round runs: given a moment to, it has not.)
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(200))
                .is_err(),
            "the drop returned while the helper's round ran"
        );
        watch.hold(false);
        let (ended, workers) = rx.recv().unwrap();
        assert!(ended, "the drop returned before the helper ended");
        assert_eq!(workers, 0, "the drop returned before the workers stopped");
        dropper.join().unwrap();
    }

    /// The VST3 as a JUCE host plays it (Sandyne, decisions.md R31): set up for blocks of up
    /// to 1920 samples, the side chain deactivated (the host's instrument has no inputs), then
    /// blocks of 480 with that bus's two channels given as null pointers. The blocks play,
    /// every output sample written and finite; 0.1.0 and 0.1.1 read the side chain from
    /// address 0, an access violation.
    #[test]
    #[allow(unsafe_code)]
    fn a_vst3_host_that_deactivates_the_side_chain_has_its_blocks_played() {
        use nih_plug::wrapper::vst3::{Wrapper, vst3_sys};
        use vst3_sys::base::{IPluginBase, IUnknown, kResultOk};
        use vst3_sys::vst::{
            AudioBusBuffers, BusDirections, IAudioProcessor, IComponent, MediaTypes, ProcessData,
            ProcessModes, ProcessSetup, SymbolicSampleSizes,
        };
        let _one = BUDGET_TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        const LEN: usize = 480;
        let realtime = ProcessModes::kRealtime as i32;
        let float32 = SymbolicSampleSizes::kSample32 as i32;

        let w = Box::leak(Wrapper::<Ca72>::new());
        let mut side = [std::ptr::null_mut::<f32>(); 2];
        let (mut left, mut right) = (vec![f32::NAN; LEN], vec![f32::NAN; LEN]);
        let mut out = [left.as_mut_ptr(), right.as_mut_ptr()];
        let mut inputs = AudioBusBuffers {
            num_channels: 2,
            silence_flags: u64::MAX,
            buffers: side.as_mut_ptr().cast(),
        };
        let mut outputs = AudioBusBuffers {
            num_channels: 2,
            silence_flags: 0,
            buffers: out.as_mut_ptr().cast(),
        };
        // SAFETY: the calls a host makes, in its order, on an instance it then releases; the
        // buffers outlive them, and the rest of the block's data is null (no events, no
        // parameter changes, no transport), which the wrapper takes as absent.
        unsafe {
            assert_eq!(w.initialize(std::ptr::null_mut()), kResultOk);
            let setup = ProcessSetup {
                process_mode: realtime,
                symbolic_sample_size: float32,
                max_samples_per_block: 1920,
                sample_rate: 48_000.0,
            };
            assert_eq!(w.setup_processing(&setup), kResultOk);
            // Input bus 0 is the side chain: the plug-in has no main input.
            let (audio, input) = (MediaTypes::kAudio as i32, BusDirections::kInput as i32);
            assert_eq!(w.activate_bus(audio, input, 0, 0), kResultOk);
            assert_eq!(w.set_active(1), kResultOk);
            assert_eq!(w.set_processing(1), kResultOk);

            let mut data: ProcessData = std::mem::zeroed();
            data.process_mode = realtime;
            data.symbolic_sample_size = float32;
            data.num_samples = LEN as i32;
            data.num_inputs = 1;
            data.inputs = &mut inputs;
            data.num_outputs = 1;
            data.outputs = &mut outputs;
            for _ in 0..3 {
                assert_eq!(w.process(&mut data), kResultOk);
            }

            assert_eq!(w.set_processing(0), kResultOk);
            assert_eq!(w.set_active(0), kResultOk);
            assert_eq!(w.terminate(), kResultOk);
            w.release();
        }
        assert!(left.iter().chain(&right).all(|x| x.is_finite()));
    }

    /// R-AUTO-GAIN-DRIVE: a preset's curve, loaded as the presets load it, is the engine's at
    /// the next block and nothing is measured for it; a curve the helper measured (the sound's
    /// after DRIVE was turned) is the engine's too, gliding in.
    #[test]
    fn a_presets_curve_is_in_force_at_the_next_block_and_a_measured_one_glides_in() {
        let _one = BUDGET_TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut p = initialised(false);
        let mut c = hostless(&p);
        let smoke = crate::library::factory()
            .iter()
            .find(|s| s.name == "Smoke Flute")
            .expect("Smoke Flute");
        let db = smoke.drive_curve.expect("its curve");
        p.params.drive_curve.load_preset(db);
        assert!(!p.params.drive_curve.waiting());
        assert!(!p.params.drive_curve.measured(&drive::Curve(db)));
        block(&mut p, &mut c, 256);
        assert_eq!(p.engine.curve(), drive::Curve(db));
        // A measurement (as DRIVE turned in the editor asks for one).
        let mut controls = p.params.controls();
        controls.panel.cutoff = 0.2;
        p.params.drive_curve.ask(&controls);
        until("measured", || !p.params.drive_curve.waiting());
        until("stored and marked", || {
            let saved = p.params.drive_curve.saved();
            saved.sound == drive::sound(&controls)
                && p.params.drive_curve.measured(&drive::Curve(saved.db))
        });
        let measured = drive::Curve(p.params.drive_curve.saved().db);
        block(&mut p, &mut c, 256);
        assert_eq!(p.engine.curve(), measured);
    }

    /// AUTO GAIN's curve is the session's (decisions.md R45, the CA-74's R29): saved,
    /// loaded into another instance, and the engine plays with it from the next block; a
    /// session saved before DRIVE, loaded into an instance with a measured curve, leaves it the
    /// average's; one whose curve is not understood, the average's too.
    #[test]
    fn a_session_holds_its_curve_and_an_old_one_the_average() {
        use crate::drive::{AVERAGE, Curve, STATE_KEY, Saved, unmeasured};
        use nih_plug::params::persist::PersistentField;
        let state = |fields: BTreeMap<String, String>| PluginState {
            version: String::new(),
            params: Default::default(),
            fields,
        };
        let measured = Saved {
            sound: 5,
            db: [-1.0, -2.0, -3.0, -4.0],
        };
        let a = Ca72::default();
        a.params.drive_curve.set(measured);
        let saved = a.params.serialize_fields();
        assert_eq!(
            saved.get(STATE_KEY).map(String::as_str),
            Some(r#"{"sound":5,"db":[-1.0,-2.0,-3.0,-4.0]}"#)
        );
        let mut b = Ca72::default();
        let mut s = state(saved);
        Ca72::filter_state(&mut s);
        b.params.deserialize_fields(&s.fields);
        assert_eq!(b.params.drive_curve.saved(), measured);
        let mut c = hostless(&b);
        block(&mut b, &mut c, 64);
        assert_eq!(b.engine.curve(), Curve(measured.db));
        // Older, without one: the average's.
        let mut old = state(BTreeMap::from([(
            "preset".to_owned(),
            "\"Bass\"".to_owned(),
        )]));
        Ca72::filter_state(&mut old);
        b.params.deserialize_fields(&old.fields);
        assert_eq!(b.params.drive_curve.saved(), unmeasured());
        block(&mut b, &mut c, 64);
        assert_eq!(b.engine.curve(), AVERAGE);
        // Not understood: the average's.
        b.params.drive_curve.set(measured);
        let mut bad = state(BTreeMap::from([(STATE_KEY.to_owned(), "[1, 2".to_owned())]));
        Ca72::filter_state(&mut bad);
        b.params.deserialize_fields(&bad.fields);
        assert_eq!(b.params.drive_curve.saved(), unmeasured());
    }
}

#[allow(unsafe_code)]
mod export {
    use super::Ca72;
    use nih_plug::prelude::*;

    nih_export_clap!(Ca72);
    nih_export_vst3!(Ca72);
}
