//! The MCP tools: each one an API method a CA-72 carries out (`editor/agent.rs`), or one the
//! bridge answers itself (finding and choosing an instance), with its description and the
//! JSON Schema of its arguments. The parameters' schema is made from the parameters
//! themselves (`values::table`), so that it is never out of step with them (DAW-v3's D165,
//! item 1: the tools' schemas generated from the API's methods).

use serde_json::{Value, json};

use super::values;
use crate::params::Ca72Params;

/// Where a tool is carried out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Runs {
    /// By the bridge, which finds and chooses the instance.
    Bridge,
    /// By the CA-72 chosen, in its editor.
    Instance,
}

#[derive(Debug)]
pub struct Tool {
    pub name: &'static str,
    pub method: &'static str,
    pub description: &'static str,
    pub schema: fn(&Ca72Params) -> Value,
    pub read_only: bool,
    pub destructive: bool,
    pub runs: Runs,
}

fn obj(props: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": props, "required": required, "additionalProperties": false})
}

fn none(_: &Ca72Params) -> Value {
    obj(json!({}), &[])
}

fn name_only(_: &Ca72Params) -> Value {
    obj(
        json!({"name": {"type": "string", "description": "The preset's name (any case)."}}),
        &["name"],
    )
}

/// Notes as MIDI note numbers (60 is C4, middle C) or names ("C4", "F#2", "Bb3").
fn notes_schema() -> Value {
    json!({
        "type": "array",
        "items": {"type": ["integer", "string"], "minimum": 0, "maximum": 127},
        "description": "MIDI note numbers 0 to 127 (60 = C4, middle C) or note names such as \"C4\", \"F#2\", \"Bb3\"."
    })
}

/// A note's velocity (decisions.md R-VELOCITY).
fn velocity_schema() -> Value {
    json!({"type": "integer", "minimum": 1, "maximum": 127, "default": 100, "description": "How hard the keys are struck, MIDI's 1 to 127."})
}

/// A parameter's id, label or host name.
fn param_key(p: &Ca72Params) -> Value {
    json!({"type": "string", "description": "A parameter's id (see ca72_describe), its panel label or its host name.", "examples": values::ids(p)})
}

/// Every tool, the bridge's first.
pub fn tools() -> Vec<Tool> {
    use Runs::*;
    let t = |name, method, description, schema, read_only, destructive, runs| Tool {
        name,
        method,
        description,
        schema,
        read_only,
        destructive,
        runs,
    };
    vec![
        t(
            "ca72_instances",
            "instances",
            "The CA-72s open on this computer that allow agents (SETTINGS > AGENTS in the CA-72), each with its id, kind (standalone), version and process, and which one the other tools act on. Calls go to the only one open unless you choose with ca72_select.",
            none,
            true,
            false,
            Bridge,
        ),
        t(
            "ca72_select",
            "select",
            "Choose the CA-72 the other tools act on, by its id from ca72_instances.",
            |_| {
                obj(
                    json!({"instance": {"type": "string", "description": "Its id from ca72_instances."}}),
                    &["instance"],
                )
            },
            true,
            false,
            Bridge,
        ),
        t(
            "ca72_describe",
            "describe",
            "Start here. The whole state of the CA-72: its version; every parameter with its panel label, section, value (a number in the unit the panel prints, a position's name, or on/off), the value as printed, its range or choices and default; the preset it is set to and whether its sound has been changed since; the settings; what the window shows (the presets' drawer, the lower panel, the on-screen keyboard and its range); MIDI Learn's assignments; the notes you hold; the audio's state.",
            none,
            true,
            false,
            Instance,
        ),
        t(
            "ca72_get",
            "parameters.get",
            "Read parameters: each one's value (in the panel's units), as printed, with its range or choices. All of them without `ids`.",
            |p| {
                obj(
                    json!({"ids": {"type": "array", "items": param_key(p)}}),
                    &[],
                )
            },
            true,
            false,
            Instance,
        ),
        t(
            "ca72_set",
            "parameters.set",
            "Set parameters as the panel does, together, each change a gesture of its own. Values are in the units the panel prints: the knobs in their dials' numbers (0 to 10, CUTOFF -5 to +5, TUNE -2.5 to +2.5), oscillators 2 and 3's FREQUENCY in semitones from the centre (or \"+12 cents\"), ENTROPY, WIDTH and INNER in %, DETUNE in cents, DRIVE and LEVEL in dB, VOICES a count; a selector's position by its name (\"16'\", \"Sawtooth\") or index from 0; a switch true or false. A value outside the control's range is refused, not clamped, and nothing is set if any value is wrong. DETUNE is refused while the voices play as SCATTER (set `doubled` true, in the same call or before) and VOICES while it does nothing, as on the panel. `mode` sets the strip's MODE (MONO, POLY or UNISON) as its tabs do. Returns the values as set.",
            set_schema,
            false,
            false,
            Instance,
        ),
        t(
            "ca72_reset",
            "parameters.reset",
            "Return parameters to their defaults, as a double click on a control does.",
            |p| {
                obj(
                    json!({"ids": {"type": "array", "items": param_key(p), "minItems": 1}}),
                    &["ids"],
                )
            },
            false,
            false,
            Instance,
        ),
        t(
            "ca72_presets",
            "presets.list",
            "The presets as the drawer lists them (factory, your edits of them, and yours), with their tags and favorites, filtered as the drawer filters: `search` text, `tags` (all of them), `favorites` only, or `mine` only. Also the preset the CA-72 is set to.",
            |_| {
                obj(
                    json!({
                        "search": {"type": "string"},
                        "tags": {"type": "array", "items": {"type": "string"}},
                        "favorites": {"type": "boolean"},
                        "mine": {"type": "boolean"},
                    }),
                    &[],
                )
            },
            true,
            false,
            Instance,
        ),
        t(
            "ca72_preset_load",
            "presets.load",
            "Set the CA-72 to a preset, as choosing it in the drawer does: every sound parameter to the preset's value (QUALITY, POWER and the PITCH wheel stay as they are).",
            name_only,
            false,
            false,
            Instance,
        ),
        t(
            "ca72_preset_step",
            "presets.step",
            "The next (1) or previous (-1) preset of the drawer's list, as the rail's arrows choose it.",
            |_| obj(json!({"by": {"type": "integer", "enum": [-1, 1]}}), &["by"]),
            false,
            false,
            Instance,
        ),
        t(
            "ca72_preset_save",
            "presets.save",
            "Save the current sound as a preset of yours (SAVE AS), with tags, and set the CA-72 to it. A name already taken is refused unless `replace` is true (yours is then replaced; a factory preset's name puts yours in its place, which REVERT undoes).",
            |_| {
                obj(
                    json!({
                        "name": {"type": "string", "minLength": 1, "maxLength": 64},
                        "tags": {"type": "array", "items": {"type": "string", "maxLength": 32}, "maxItems": 16},
                        "replace": {"type": "boolean", "default": false},
                    }),
                    &["name"],
                )
            },
            false,
            true,
            Instance,
        ),
        t(
            "ca72_preset_rename",
            "presets.rename",
            "Rename a preset (RENAME).",
            |_| {
                obj(
                    json!({"name": {"type": "string"}, "to": {"type": "string", "minLength": 1, "maxLength": 64}}),
                    &["name", "to"],
                )
            },
            false,
            false,
            Instance,
        ),
        t(
            "ca72_preset_tags",
            "presets.tags",
            "Set a preset's tags (TAGS), replacing those it has.",
            |_| {
                obj(
                    json!({"name": {"type": "string"}, "tags": {"type": "array", "items": {"type": "string", "maxLength": 32}, "maxItems": 16}}),
                    &["name", "tags"],
                )
            },
            false,
            false,
            Instance,
        ),
        t(
            "ca72_preset_favorite",
            "presets.favorite",
            "Mark a preset a favorite or not (its star).",
            |_| {
                obj(
                    json!({"name": {"type": "string"}, "favorite": {"type": "boolean"}}),
                    &["name", "favorite"],
                )
            },
            false,
            false,
            Instance,
        ),
        t(
            "ca72_preset_delete",
            "presets.delete",
            "Delete a preset (DELETE). A preset of yours is deleted; a factory preset is hidden, and ca72_presets_restore brings it back.",
            name_only,
            false,
            true,
            Instance,
        ),
        t(
            "ca72_preset_revert",
            "presets.revert",
            "Take a factory preset you have edited back to the factory's (REVERT).",
            name_only,
            false,
            true,
            Instance,
        ),
        t(
            "ca72_presets_restore",
            "presets.restore",
            "Bring back every factory preset you have deleted (RESTORE).",
            none,
            false,
            false,
            Instance,
        ),
        t(
            "ca72_settings",
            "settings.get",
            "The settings this computer keeps for the CA-72 (the SETTINGS menu's, the MOD wheel's source among them, QUALITY's indicator, agents allowed), and the window's size.",
            none,
            true,
            false,
            Instance,
        ),
        t(
            "ca72_settings_set",
            "settings.set",
            "Change settings, as the SETTINGS menu does: `look` (REALISTIC or CLASSIC), `lettering` (STANDARD or LARGE), `ticks_and_titles` (STANDARD or QUIET: the panel's ticks and sections' titles as approved, or thinner, shorter, smaller and dimmer), `frequency_dials` (SEMITONES or HARDWARE: how oscillators 2 and 3's FREQUENCY dials read; the sound stays as it is), `quality_indicator` (QUALITY's indicator shown or hidden), `mod_wheel_source` (what moves the MOD wheel, on any channel: `CC 1` to `CC 127` but the reserved ones, `CHANNEL AFTERTOUCH` or `POLYPHONIC AFTERTOUCH`), `mpe_input` (where MPE's zone comes from on this computer: AUTO, the controller's MPE Configuration Message; LOWER ZONE, fifteen member channels, for hosts that do not pass it on; or OFF), `timbre_rest` (where timbre, CC 74 or a host's brightness, moves nothing: CENTERED ON 64, FROM 0, or FROM THE NOTE'S START), `size` (the window's width in pixels, one of the SIZE menu's or FIT), `agents` (false turns agents off: this connection then ends, and only the player can turn them on again).",
            |_| {
                obj(
                    json!({
                        "look": {"type": "string", "enum": ["REALISTIC", "CLASSIC"]},
                        "lettering": {"type": "string", "enum": ["STANDARD", "LARGE"]},
                        "ticks_and_titles": {"type": "string", "enum": ["STANDARD", "QUIET"]},
                        "frequency_dials": {"type": "string", "enum": ["SEMITONES", "HARDWARE"]},
                        "tempo": {"type": "number", "minimum": 30, "maximum": 300, "description": "Standalone TEMPO in BPM; follows the screen's own path."},
                        "quality_indicator": {"type": "boolean"},
                        "mod_wheel_source": {"type": "string", "description": "CC 1 (the default) or another CC by number (\"CC 11\"), CHANNEL AFTERTOUCH or POLYPHONIC AFTERTOUCH."},
                        "mpe_input": {"type": "string", "enum": ["AUTO", "LOWER ZONE", "OFF"]},
                        "timbre_rest": {"type": "string", "enum": ["CENTERED ON 64", "FROM 0", "FROM THE NOTE'S START"]},
                        "size": {"type": ["integer", "string"], "description": "A width in logical pixels from the SIZE menu (ca72_settings lists them), or \"FIT\"."},
                        "agents": {"type": "boolean", "enum": [false]},
                    }),
                    &[],
                )
            },
            false,
            false,
            Instance,
        ),
        t(
            "ca72_view",
            "view.set",
            "What the window shows, as the rail's keys and the keyboard's buttons change it: `presets` (the presets' drawer open), `panel` (the lower panel shown), `lower_page` (select `global` for VOICES/STEREO/OUTPUT, `keyboard` for KEYS or `lfo`, showing PANEL if hidden; pressing the selected key does nothing), `keyboard` (legacy: true selects keyboard, false selects global), `keyboard_octave` (move its range an octave `up` or `down`), `midi_list` (the drawer's MIDI Learn list in place of the presets).",
            |_| {
                obj(
                    json!({
                        "presets": {"type": "boolean"},
                        "panel": {"type": "boolean"},
                        "lower_page": {"type": "string", "enum": ["global", "keyboard", "lfo"]},
                        "keyboard": {"type": "boolean"},
                        "keyboard_octave": {"type": "string", "enum": ["up", "down"]},
                        "midi_list": {"type": "boolean"},
                    }),
                    &[],
                )
            },
            false,
            false,
            Instance,
        ),
        t(
            "ca72_note_on",
            "notes.on",
            "Press keys, as the on-screen keyboard does, until ca72_note_off, at `velocity` (1 to 127, 100 if omitted; it moves only what the patch's VELOCITY is assigned to). The CA-72 plays them as any key held: one note at a time by NOTE PRIORITY unless POLY or UNISON. A note already held by you is held still.",
            |_| {
                obj(
                    json!({"notes": notes_schema(), "velocity": velocity_schema()}),
                    &["notes"],
                )
            },
            false,
            false,
            Instance,
        ),
        t(
            "ca72_note_off",
            "notes.off",
            "Let go of keys you pressed; all of yours without `notes`.",
            |_| obj(json!({"notes": notes_schema()}), &[]),
            false,
            false,
            Instance,
        ),
        t(
            "ca72_play",
            "notes.play",
            "Play notes together for `seconds` (0.01 to 30), then let them go; returns at once. To hear a sound's release, wait out its decay before the next.",
            |_| {
                obj(
                    json!({"notes": notes_schema(), "seconds": {"type": "number", "minimum": 0.01, "maximum": 30}, "velocity": velocity_schema()}),
                    &["notes", "seconds"],
                )
            },
            false,
            false,
            Instance,
        ),
        t(
            "ca72_panic",
            "notes.panic",
            "PANIC: every note off, from every source (MIDI devices too), and the controllers reset.",
            none,
            false,
            false,
            Instance,
        ),
        t(
            "ca72_modulations",
            "modulation.list",
            "What each modulation source (LFO 1 to LFO 4, `lfo1` to `lfo4`; MACRO 1 to MACRO 6, `macro1` to `macro6`; VELOCITY, `velocity`) modulates: each knob by its id and label with its depth in % of the knob's travel (-100 to 100; negative inverts), `shared` where an LFO with MONO off still moves it with one copy (a knob every voice shares: MAIN OUTPUT VOLUME, the strip's), `inert` where it does nothing now; each LFO's EDGES and MONO; each macro's typed `name` (\"\" for none) and its knob's `value` in %; the most a source takes (8); and the knobs that can take one now. A macro moves its knobs one way, by depth × its value; VELOCITY one way too, each voice by its own note's velocity through the expression drawer's CURVE and FLOOR (`velocity_curve`, `velocity_floor`). The LFOs' own controls and the macros' knobs are parameters (`lfo1_rate`, `macro1`: ca72_describe).",
            none,
            true,
            false,
            Instance,
        ),
        t(
            "ca72_modulation_set",
            "modulation.set",
            "Make a source modulate a knob, as dragging from an LFO's OUT jack, a macro's jack or the left hand's VELOCITY jack does, or change its depth: `source` (lfo1 to lfo4, \"LFO 1\" or 1 to 4; macro1 to macro6, \"MACRO 1\" or a macro's name; velocity), `target` (a continuous knob of the panel or the strip by id or label; an oscillator's WAVEFORM only while CONTINUOUS; including an LFO's own RATE (while untimed), PHASE, FADE, continuous shape and LEVEL; not a switch, selector, wheel, VOICES, a timed dial or a macro; VELOCITY not an LFO's own knob nor TEMPO) and `depth` in % of the knob's travel, -100 to 100 (25 if a new one is given none). `edges` (SOFT or SHARP) sets an LFO's EDGES; `name` (at most 7 characters, capitals; \"\" for none) a macro's name. A source takes at most 8 knobs. Patch state, saved with presets and sessions. Returns what every source modulates.",
            modulation_set_schema,
            false,
            false,
            Instance,
        ),
        t(
            "ca72_modulation_remove",
            "modulation.remove",
            "Stop a source modulating a knob (the knob's menu's REMOVE), or every knob it modulates (no `target`). Returns what every source modulates.",
            |p| {
                obj(
                    json!({
                        "source": {"type": ["string", "integer"], "description": "lfo1 to lfo4, \"LFO 1\" or 1 to 4; macro1 to macro6, \"MACRO 1\" or a macro's name; velocity."},
                        "target": param_key(p),
                    }),
                    &["source"],
                )
            },
            false,
            true,
            Instance,
        ),
        t(
            "ca72_effect_parameters",
            "effects.parameters",
            "The standalone application's effects (not the plug-ins'): the parameters of the effect in `slot` (1 to 4) a player may pin, in name order (`find`: only those whose name holds every word), each with its `parameter` ID, names, units, steps, value in % of its travel, the plug-in's own `text` for it (\"2.40 s\") and its pin target if pinned (`fx1` to `fx16`). Pinned parameters are modulation targets (ca72_modulation_set with `target` fx1 to fx16).",
            |_| {
                obj(
                    json!({
                        "slot": {"type": "integer", "minimum": 1, "maximum": 4},
                        "find": {"type": "string", "description": "Words the parameter's name holds."},
                    }),
                    &["slot"],
                )
            },
            true,
            false,
            Instance,
        ),
        t(
            "ca72_effect_pin",
            "effects.pin",
            "Pin a parameter of the effect in `slot` (1 to 4) by its `parameter` ID or name: it shows as a knob under its effect, and becomes a modulation target (its pin's id, `fx1` to `fx16`, returned) that LFOs and macros move. At most 4 an effect. Saved with the effects' chain.",
            |_| {
                obj(
                    json!({
                        "slot": {"type": "integer", "minimum": 1, "maximum": 4},
                        "parameter": {"type": ["integer", "string"], "description": "Its ID (ca72_effect_parameters) or its name."},
                    }),
                    &["slot", "parameter"],
                )
            },
            false,
            false,
            Instance,
        ),
        t(
            "ca72_effect_repin",
            "effects.repin",
            "Replace a pin's parameter, keeping its target, modulation assignments and MIDI Learn. Its base is the new parameter's current value. Saved with the effects' chain.",
            |_| {
                obj(
                    json!({
                        "target": {"type": "string", "description": "fx1 to fx16."},
                        "parameter": {"type": ["integer", "string"], "description": "Its ID or name in the same effect."},
                    }),
                    &["target", "parameter"],
                )
            },
            false,
            false,
            Instance,
        ),
        t(
            "ca72_effect_unpin",
            "effects.unpin",
            "Unpin an effect's parameter by its pin's `target` (`fx1` to `fx16`): what modulated it is let go of, and the parameter stays at its value.",
            |_| {
                obj(
                    json!({"target": {"type": "string", "description": "fx1 to fx16."}}),
                    &["target"],
                )
            },
            false,
            true,
            Instance,
        ),
        t(
            "ca72_midi_learn",
            "midi.learn",
            "MIDI Learn for a control, or for PREVIOUS PRESET or NEXT PRESET (`previous_preset`, `next_preset`: kept on this computer): the next controller (CC) moved on a MIDI device is assigned to it (a preset action also takes a note: a pad), check with ca72_midi_assignments. `cancel` true stops waiting. `channel` and `cc` given, the controller is assigned at once, without waiting (as if moved); a preset action takes `channel` and `note` too.",
            |p| {
                obj(
                    json!({
                        "control": param_key(p),
                        "cancel": {"type": "boolean"},
                        "channel": {"type": "integer", "minimum": 1, "maximum": 16},
                        "cc": {"type": "integer", "minimum": 0, "maximum": 127},
                        "note": {"type": "integer", "minimum": 0, "maximum": 127},
                    }),
                    &[],
                )
            },
            false,
            false,
            Instance,
        ),
        t(
            "ca72_midi_assignments",
            "midi.list",
            "MIDI Learn's assignments: each learnable control (and PREVIOUS PRESET and NEXT PRESET) with its controller (channel 1 to 16 and CC, or a preset action's note) or none, the one being learned, and the controllers that are never learned (the MOD wheel's source among them).",
            none,
            true,
            false,
            Instance,
        ),
        t(
            "ca72_midi_forget",
            "midi.remove",
            "Remove a control's MIDI assignment (REMOVE MIDI ASSIGNMENT).",
            |p| obj(json!({"control": param_key(p)}), &["control"]),
            false,
            true,
            Instance,
        ),
        t(
            "ca72_snapshot",
            "render.snapshot",
            "Capture every current parameter, unsaved edits, LFO and macro assignments, AUTO GAIN curve, tempo and random seed for a detached offline render. Returns a snapshot for ca72_render; does not copy notes held or oscillator phase. Does not change the live sound.",
            none,
            true,
            false,
            Instance,
        ),
        t(
            "ca72_preset_export",
            "audio.preset_export",
            "Export a fitted candidate snapshot as a normal user-preset TOML into a new absolute output_dir. Does not need the app or install into the player's library. Preserves assignments, macro names and AUTO GAIN with the normal four-decimal preset serialization. Returns the exact preset_file, hash and render_conditions (quality, player controls, tempo, frequency-dial law, seed). Reload using ca72_render with preset_file and those conditions to verify the delivered file.",
            super::audio::export_schema,
            false,
            false,
            Bridge,
        ),
        t(
            "ca72_render",
            "audio.render",
            "Render the current sound, a named library preset, or a previous snapshot through the same synth process as playback, without an audio device or changing the live instrument. Supply MIDI notes with start and duration in seconds, total seconds including the release tail, and a new absolute output_dir. A preset or snapshot also works with no application open. effects must explicitly be dry: third-party effects are excluded and external input is silent. Returns a float WAV, exact snapshot, hashes and manifest. Fixed seed and settings repeat the render; latency is compensated. Optional values use ca72_set's parameter IDs and panel units, including all four LFOs' Env modes. Optional modulations edits their routing after values, including secondary envelopes, with no app. Only this candidate changes; routing is preserved by snapshot/export/reload.",
            super::audio::render_schema,
            false,
            false,
            Bridge,
        ),
        t(
            "ca72_analyze",
            "audio.analyze",
            "Optionally supply named regions for separate notes/chords/attacks/releases, several known fundamentals and approximate modulation depth. Decode once per call. Measure a local WAV reference or candidate: pitch track, loudness, 10 ms RMS envelope, onsets, harmonics and optional modulation sidebands, plus a log-frequency spectrogram PNG. Works without the app. Supply a new output_dir, optional start and seconds (up to 60), fundamental_hz for a selected held note, modulation_hz for its sidebands. No upload or source separation. A full mix's measurements include its other parts; isolate notes to interpret harmonics. Full metrics are written to JSON; PNG returned inline unless include_image=false.",
            super::audio::analyze_schema,
            false,
            false,
            Bridge,
        ),
        t(
            "ca72_compare",
            "audio.compare",
            "Optional named regions return separate harmonic/full-spectrum/peak diagnostics. Optional approved audio and explicit preservation_limits check actual expression separately; unresolved requested measurements cannot pass. Compare a reference WAV and candidate WAV locally at the same rate and duration, with explicit crop starts and optional seconds. Level matches by LUFS (RMS for short clips), compares time-varying spectra at three resolutions, envelopes, pitch and selected-note harmonics/sidebands. Writes full metrics, two spectrograms on one colour scale and level-matched A/B WAVs to a new output_dir. No hidden time/pitch alignment, truncation or similarity percentage. Choose matching notes/timing first, inspect individual notes, and listen. Works without the app.",
            super::audio::compare_schema,
            false,
            false,
            Bridge,
        ),
        t(
            "ca72_experiment",
            "audio.experiment",
            "Run 1 to 8 independent offline candidate variations against named regions of an original local WAV, with optional approved-expression baseline and explicit preservation limits. Decode/analyze reference regions once per batch, validate all candidates before output, retain exact render manifests/snapshots and separate timbre/peak/expression diagnostics. Bounded to 120 rendered seconds/12 million frames. Write a common-level audition report and complete experiment.json; no automatic winner, preset installation or live-state changes. Use the chosen manifest's snapshot with ca72_preset_export, then reload the exact file for delivery.",
            super::audio::experiment::schema,
            false,
            false,
            Bridge,
        ),
        t(
            "ca72_audio",
            "audio.get",
            "The standalone's audio and MIDI (AUDIO & MIDI…): the output device, sample rate and buffer, the external input, the MIDI inputs and channel, the devices there are, its state and any error. Read only in this version.",
            none,
            true,
            false,
            Instance,
        ),
        t(
            "ca72_version",
            "version",
            "The CA-72's version, and the update check's state. `check` true checks for a newer release on GitHub, as the drawer's CHECK FOR UPDATES does (the answer shows in a later call).",
            |_| obj(json!({"check": {"type": "boolean"}}), &[]),
            true,
            false,
            Instance,
        ),
    ]
}

/// Shared routing schema for live edits and detached render candidates.
pub(crate) fn modulation_set_schema(p: &Ca72Params) -> Value {
    obj(
        json!({
            "source": {"type": ["string", "integer"], "description": "lfo1 to lfo4, \"LFO 1\" or 1 to 4; macro1 to macro6, \"MACRO 1\" or a macro's name; velocity."},
            "target": param_key(p),
            "depth": {"type": "number", "minimum": -100, "maximum": 100},
            "edges": {"type": "string", "enum": ["SOFT", "SHARP"]},
            "name": {"type": "string", "maxLength": 7, "description": "A macro's name (\"\" for none)."},
        }),
        &["source"],
    )
}

/// `parameters.set`'s schema: a value for any parameter, each typed by its shape.
fn set_schema(p: &Ca72Params) -> Value {
    let mut props = serde_json::Map::new();
    for e in values::table(p) {
        props.insert(e.id.into(), e.schema());
    }
    props.insert(
        "mode".into(),
        json!({"type": "string", "enum": ["MONO", "POLY", "UNISON"], "description": "The strip's MODE, as its tabs set POLY and UNISON."}),
    );
    obj(
        json!({"values": {
            "type": "object",
            "description": "Parameter ids (or labels) and their values.",
            "properties": props,
            "additionalProperties": {"type": ["number", "string", "boolean", "integer"]},
            "minProperties": 1,
        }}),
        &["values"],
    )
}

/// The tool of this name.
pub fn find(name: &str) -> Option<Tool> {
    tools().into_iter().find(|t| t.name == name)
}

/// `tools/list`'s list.
pub fn list(p: &Ca72Params) -> Vec<Value> {
    tools()
        .into_iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": (t.schema)(p),
                "annotations": {
                    "readOnlyHint": t.read_only,
                    "destructiveHint": t.destructive,
                    "openWorldHint": false,
                },
            })
        })
        .collect()
}

/// The methods an instance answers (all but the bridge's).
pub fn instance_methods() -> Vec<&'static str> {
    tools()
        .into_iter()
        .filter(|t| t.runs == Runs::Instance)
        .map(|t| t.method)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each tool has a name of its own, MCP's form of name, a method of its own, a description
    /// and an object schema that names only properties it lists as required.
    #[test]
    fn the_tools_are_well_formed() {
        let p = Ca72Params::default();
        let all = tools();
        let mut names: Vec<_> = all.iter().map(|t| t.name).collect();
        let mut methods: Vec<_> = all.iter().map(|t| t.method).collect();
        names.sort_unstable();
        names.dedup();
        methods.sort_unstable();
        methods.dedup();
        assert_eq!((names.len(), methods.len()), (all.len(), all.len()));
        for t in &all {
            assert!(
                t.name.len() <= 64
                    && t.name
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit()),
                "{}",
                t.name
            );
            assert!(t.description.len() > 20, "{}", t.name);
            let s = (t.schema)(&p);
            assert_eq!(s["type"], json!("object"), "{}", t.name);
            let props = s["properties"].as_object().expect("properties");
            for r in s["required"].as_array().expect("required") {
                assert!(
                    props.contains_key(r.as_str().unwrap_or("")),
                    "{}: {r}",
                    t.name
                );
            }
            assert!(!(t.read_only && t.destructive), "{}", t.name);
        }
        assert_eq!(list(&p).len(), all.len());
    }

    /// `ca72_set`'s schema names every parameter, typed by its shape: ranges in the panel's
    /// units, choices by name.
    #[test]
    fn the_set_schema_is_made_from_the_parameters() {
        let p = Ca72Params::default();
        let s = set_schema(&p);
        let props = &s["properties"]["values"]["properties"];
        for id in values::ids(&p) {
            assert!(props.get(id).is_some(), "{id}");
        }
        assert!(
            props["emphasis"]["description"]
                .as_str()
                .unwrap_or("")
                .contains("0 to 10")
        );
        assert!(
            props["drive"]["description"]
                .as_str()
                .unwrap_or("")
                .contains("dB")
        );
        let ranges = props["osc1_range"]["enum"].as_array().expect("enum");
        assert!(ranges.contains(&json!("16'")));
        assert_eq!(props["mode"]["enum"], json!(["MONO", "POLY", "UNISON"]));
    }
}
