//! MIDI Learn as a CLAP host meets it, through the plug-in's own CLAP entry point in this
//! process (decisions.md R34; `third_party/nih-plug/PATCHES.md`, change 10): a learned
//! controller's MIDI message sets its parameter at its time, and the plug-in tells the host as a
//! parameter value flagged not to be recorded (the MIDI is what a host records), asking for no
//! rescan of the parameters; at one sample
//! the host's automation is set before the controller, whatever the host's order; the
//! assignments go out with the plug-in's state and come back with it, and a state from before
//! them leaves an instance none. No editor is ever opened: all of it plays with the editor
//! closed.

#![allow(unsafe_code, clippy::unwrap_used)]

// (Linked for its `clap_entry`, which nothing else here names.)
extern crate ca72_plugin;

use std::ffi::{CStr, c_char, c_void};
use std::ptr::{null, null_mut};

use clap_sys::audio_buffer::clap_audio_buffer;
use clap_sys::entry::clap_plugin_entry;
use clap_sys::events::{
    CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_DONT_RECORD, CLAP_EVENT_IS_LIVE, CLAP_EVENT_MIDI,
    CLAP_EVENT_NOTE_EXPRESSION, CLAP_EVENT_NOTE_OFF, CLAP_EVENT_NOTE_ON, CLAP_EVENT_PARAM_VALUE,
    CLAP_EVENT_TRANSPORT, CLAP_NOTE_EXPRESSION_BRIGHTNESS, CLAP_NOTE_EXPRESSION_PRESSURE,
    CLAP_NOTE_EXPRESSION_TUNING, CLAP_TRANSPORT_IS_PLAYING, clap_event_header, clap_event_midi,
    clap_event_note, clap_event_note_expression, clap_event_param_value, clap_event_transport,
    clap_input_events, clap_output_events,
};
use clap_sys::ext::params::{
    CLAP_EXT_PARAMS, clap_host_params, clap_param_clear_flags, clap_param_info,
    clap_param_rescan_flags, clap_plugin_params,
};
use clap_sys::ext::render::{CLAP_EXT_RENDER, CLAP_RENDER_OFFLINE, clap_plugin_render};
use clap_sys::ext::state::{CLAP_EXT_STATE, clap_plugin_state};
use clap_sys::factory::plugin_factory::{CLAP_PLUGIN_FACTORY_ID, clap_plugin_factory};
use clap_sys::host::clap_host;
use clap_sys::id::clap_id;
use clap_sys::plugin::clap_plugin;
use clap_sys::process::clap_process;
use clap_sys::stream::{clap_istream, clap_ostream};
use clap_sys::version::CLAP_VERSION;
use std::sync::atomic::{AtomicUsize, Ordering};

unsafe extern "C" {
    /// The plug-in's CLAP entry point (`nih_export_clap!`), linked from this crate.
    static clap_entry: clap_plugin_entry;
}

const RATE: f64 = 48_000.0;
const BLOCK: u32 = 256;

/// A parameter's CLAP id: nih-plug's hash of its string id (`wrapper::util::hash_param_id`).
fn id(param: &str) -> clap_id {
    let mut h: u32 = 0;
    for b in param.bytes() {
        h = h.wrapping_mul(31).wrapping_add(u32::from(b));
    }
    h & !(1 << 31)
}

/// The host's count of rescans asked of it, of any kind (`host_data`).
unsafe extern "C" fn rescan(host: *const clap_host, _: clap_param_rescan_flags) {
    // SAFETY: `host_data` is the instance's counter, alive as long as its host.
    unsafe { (*(*host).host_data.cast::<AtomicUsize>()).fetch_add(1, Ordering::SeqCst) };
}
unsafe extern "C" fn clear(_: *const clap_host, _: clap_id, _: clap_param_clear_flags) {}
unsafe extern "C" fn request(_: *const clap_host) {}

static HOST_PARAMS: clap_host_params = clap_host_params {
    rescan: Some(rescan),
    clear: Some(clear),
    request_flush: Some(request),
};

unsafe extern "C" fn host_extension(_: *const clap_host, ext: *const c_char) -> *const c_void {
    // SAFETY: the plug-in passes an extension's name.
    if unsafe { CStr::from_ptr(ext) } == CLAP_EXT_PARAMS {
        (&raw const HOST_PARAMS).cast()
    } else {
        null()
    }
}

/// An event for the plug-in.
#[derive(Clone, Copy, Debug)]
enum In {
    /// A control change: time, channel (0 to 15), controller, value (0 to 127).
    Cc(u32, u8, u8, u8),
    /// The host's automation: time, parameter, value (CLAP's: normalized for a knob, the
    /// position for a selector).
    Param(u32, &'static str, f64),
    Midi(u32, [u8; 3]),
    Note(u32, u16, i32, i16, i16, f64),
    Transport(u32, bool),
    /// A note expression (decisions.md R-MPE): time, expression, note id, channel, key, value.
    Expression(u32, i32, i32, i16, i16, f64),
}

/// A parameter value the plug-in told the host: time, flags, parameter, value.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Out {
    time: u32,
    flags: u32,
    param: clap_id,
    value: f64,
}

/// The events of one process call, in the host's order (each boxed: `headers` points into it).
#[allow(clippy::vec_box)]
struct Events {
    headers: Vec<*const clap_event_header>,
    _midi: Vec<Box<clap_event_midi>>,
    _params: Vec<Box<clap_event_param_value>>,
    _notes: Vec<Box<clap_event_note>>,
    _transport: Vec<Box<clap_event_transport>>,
    _expressions: Vec<Box<clap_event_note_expression>>,
}

unsafe extern "C" fn in_size(list: *const clap_input_events) -> u32 {
    // SAFETY: `ctx` is the call's `Events`.
    unsafe {
        let events = &*(*list).ctx.cast::<Events>();
        events.headers.len() as u32
    }
}

unsafe extern "C" fn in_get(list: *const clap_input_events, i: u32) -> *const clap_event_header {
    // SAFETY: as above.
    unsafe {
        let events = &*(*list).ctx.cast::<Events>();
        events.headers.get(i as usize).copied().unwrap_or(null())
    }
}

unsafe extern "C" fn out_push(
    list: *const clap_output_events,
    e: *const clap_event_header,
) -> bool {
    // SAFETY: `ctx` is the call's `Vec<Out>`; the plug-in's event is valid for the call.
    unsafe {
        let out = &mut *(*list).ctx.cast::<Vec<Out>>();
        if (*e).space_id == CLAP_CORE_EVENT_SPACE_ID && (*e).type_ == CLAP_EVENT_PARAM_VALUE {
            let p = &*e.cast::<clap_event_param_value>();
            out.push(Out {
                time: p.header.time,
                flags: p.header.flags,
                param: p.param_id,
                value: p.value,
            });
        }
    }
    true
}

unsafe extern "C" fn write(stream: *const clap_ostream, buffer: *const c_void, size: u64) -> i64 {
    // SAFETY: `ctx` is a `Vec<u8>`; the plug-in's buffer holds `size` bytes.
    unsafe {
        let v = &mut *(*stream).ctx.cast::<Vec<u8>>();
        v.extend_from_slice(std::slice::from_raw_parts(
            buffer.cast::<u8>(),
            size as usize,
        ));
    }
    size as i64
}

unsafe extern "C" fn read(stream: *const clap_istream, buffer: *mut c_void, size: u64) -> i64 {
    // SAFETY: `ctx` is a `(Vec<u8>, usize)` (the bytes and where reading is); the plug-in's
    // buffer has room for `size`.
    unsafe {
        let (v, at) = &mut *(*stream).ctx.cast::<(Vec<u8>, usize)>();
        let n = (v.len() - *at).min(size as usize);
        std::ptr::copy_nonoverlapping(v[*at..].as_ptr(), buffer.cast::<u8>(), n);
        *at += n;
        n as i64
    }
}

/// An instance of the plug-in, activated and processing, as a host has it.
struct Instance {
    plugin: *const clap_plugin,
    _host: Box<clap_host>,
    /// How often the plug-in asked its host to rescan its parameters.
    rescans: Box<AtomicUsize>,
    block: u32,
}

impl Instance {
    fn new() -> Instance {
        Self::with_block(BLOCK)
    }

    fn with_block(block: u32) -> Instance {
        Instance::with_rate_block(RATE, block)
    }

    fn with_rate_block(rate: f64, block: u32) -> Instance {
        let rescans = Box::new(AtomicUsize::new(0));
        let host = Box::new(clap_host {
            clap_version: CLAP_VERSION,
            host_data: (&raw const *rescans).cast_mut().cast(),
            name: c"ca72 tests".as_ptr(),
            vendor: c"".as_ptr(),
            url: c"".as_ptr(),
            version: c"0".as_ptr(),
            get_extension: Some(host_extension),
            request_restart: Some(request),
            request_process: Some(request),
            request_callback: Some(request),
        });
        // SAFETY: the plug-in's own entry point and factory, used as CLAP has them.
        unsafe {
            let entry = &clap_entry;
            assert!((entry.init.unwrap())(c"".as_ptr()));
            let factory = (entry.get_factory.unwrap())(CLAP_PLUGIN_FACTORY_ID.as_ptr())
                .cast::<clap_plugin_factory>();
            let plugin = ((*factory).create_plugin.unwrap())(
                factory,
                &*host,
                c"com.idlefoundry.ca-72".as_ptr(),
            );
            assert!(!plugin.is_null());
            assert!(((*plugin).init.unwrap())(plugin));
            assert!(((*plugin).activate.unwrap())(plugin, rate, 1, block));
            assert!(((*plugin).start_processing.unwrap())(plugin));
            Instance {
                plugin,
                _host: host,
                rescans,
                block,
            }
        }
    }

    fn extension<T>(&self, name: &CStr) -> &T {
        // SAFETY: the plug-in's extension of that name is a `T`.
        unsafe {
            let e = ((*self.plugin).get_extension.unwrap())(self.plugin, name.as_ptr());
            assert!(!e.is_null(), "{name:?}");
            &*e.cast::<T>()
        }
    }

    /// One block with `events` in this order; what the plug-in told the host of parameters.
    fn process(&self, events: &[In]) -> Vec<Out> {
        self.audio(self.block, events).0
    }

    fn audio(&self, frames: u32, events: &[In]) -> (Vec<Out>, Vec<f32>) {
        let (told, l, _) = self.stereo(frames, events);
        (told, l)
    }

    /// [`Instance::audio`], both channels.
    fn stereo(&self, frames: u32, events: &[In]) -> (Vec<Out>, Vec<f32>, Vec<f32>) {
        assert!(frames <= self.block);
        let mut ev = Events {
            headers: Vec::new(),
            _midi: Vec::new(),
            _params: Vec::new(),
            _notes: Vec::new(),
            _transport: Vec::new(),
            _expressions: Vec::new(),
        };
        for &e in events {
            match e {
                In::Cc(..) | In::Midi(..) => {
                    let (time, data) = match e {
                        In::Cc(time, channel, cc, value) => (time, [0xb0 | channel, cc, value]),
                        In::Midi(time, data) => (time, data),
                        _ => unreachable!(),
                    };
                    let m = Box::new(clap_event_midi {
                        header: clap_event_header {
                            size: size_of::<clap_event_midi>() as u32,
                            time,
                            space_id: CLAP_CORE_EVENT_SPACE_ID,
                            type_: CLAP_EVENT_MIDI,
                            flags: CLAP_EVENT_IS_LIVE,
                        },
                        port_index: 0,
                        data,
                    });
                    ev.headers.push(&raw const m.header);
                    ev._midi.push(m);
                }
                In::Param(time, param, value) => {
                    let p = Box::new(clap_event_param_value {
                        header: clap_event_header {
                            size: size_of::<clap_event_param_value>() as u32,
                            time,
                            space_id: CLAP_CORE_EVENT_SPACE_ID,
                            type_: CLAP_EVENT_PARAM_VALUE,
                            flags: 0,
                        },
                        param_id: id(param),
                        cookie: null_mut(),
                        note_id: -1,
                        port_index: -1,
                        channel: -1,
                        key: -1,
                        value,
                    });
                    ev.headers.push(&raw const p.header);
                    ev._params.push(p);
                }
                In::Note(time, type_, note_id, channel, key, velocity) => {
                    let note = Box::new(clap_event_note {
                        header: clap_event_header {
                            size: size_of::<clap_event_note>() as u32,
                            time,
                            space_id: CLAP_CORE_EVENT_SPACE_ID,
                            type_,
                            flags: 0,
                        },
                        note_id,
                        port_index: 0,
                        channel,
                        key,
                        velocity,
                    });
                    ev.headers.push(&raw const note.header);
                    ev._notes.push(note);
                }
                In::Expression(time, expression_id, note_id, channel, key, value) => {
                    let x = Box::new(clap_event_note_expression {
                        header: clap_event_header {
                            size: size_of::<clap_event_note_expression>() as u32,
                            time,
                            space_id: CLAP_CORE_EVENT_SPACE_ID,
                            type_: CLAP_EVENT_NOTE_EXPRESSION,
                            flags: 0,
                        },
                        expression_id,
                        note_id,
                        port_index: 0,
                        channel,
                        key,
                        value,
                    });
                    ev.headers.push(&raw const x.header);
                    ev._expressions.push(x);
                }
                In::Transport(time, playing) => {
                    // SAFETY: every field of the CLAP transport permits zero.
                    let mut transport: Box<clap_event_transport> =
                        Box::new(unsafe { std::mem::zeroed() });
                    transport.header = clap_event_header {
                        size: size_of::<clap_event_transport>() as u32,
                        time,
                        space_id: CLAP_CORE_EVENT_SPACE_ID,
                        type_: CLAP_EVENT_TRANSPORT,
                        flags: 0,
                    };
                    transport.flags = if playing {
                        CLAP_TRANSPORT_IS_PLAYING
                    } else {
                        0
                    };
                    ev.headers.push(&raw const transport.header);
                    ev._transport.push(transport);
                }
            }
        }
        let n = frames as usize;
        let (mut out_l, mut out_r, mut in_l, mut in_r) = (
            vec![0.0f32; n],
            vec![0.0f32; n],
            vec![0.0f32; n],
            vec![0.0f32; n],
        );
        let mut outs = [out_l.as_mut_ptr(), out_r.as_mut_ptr()];
        let mut ins = [in_l.as_mut_ptr(), in_r.as_mut_ptr()];
        let mut output = clap_audio_buffer {
            data32: outs.as_mut_ptr(),
            data64: null_mut(),
            channel_count: 2,
            latency: 0,
            constant_mask: 0,
        };
        let input = clap_audio_buffer {
            data32: ins.as_mut_ptr(),
            data64: null_mut(),
            channel_count: 2,
            latency: 0,
            constant_mask: 0,
        };
        let in_events = clap_input_events {
            ctx: (&raw mut ev).cast(),
            size: Some(in_size),
            get: Some(in_get),
        };
        let mut told: Vec<Out> = Vec::new();
        let out_events = clap_output_events {
            ctx: (&raw mut told).cast(),
            try_push: Some(out_push),
        };
        let process = clap_process {
            steady_time: -1,
            frames_count: frames,
            transport: null(),
            audio_inputs: &input,
            audio_outputs: &mut output,
            audio_inputs_count: 1,
            audio_outputs_count: 1,
            in_events: &in_events,
            out_events: &out_events,
        };
        // SAFETY: a process call as CLAP has it, everything it points to alive through it.
        unsafe {
            ((*self.plugin).process.unwrap())(self.plugin, &process);
        }
        assert!(out_l.iter().chain(&out_r).all(|x| x.is_finite()));
        (told, out_l, out_r)
    }

    /// A parameter's value as the host reads it (CLAP's: normalized for a knob, the position
    /// for a selector).
    fn value(&self, param: &str) -> f64 {
        let params: &clap_plugin_params = self.extension(CLAP_EXT_PARAMS);
        let mut v = f64::NAN;
        // SAFETY: as CLAP has it.
        assert!(unsafe { (params.get_value.unwrap())(self.plugin, id(param), &mut v) });
        v
    }

    /// The plug-in's state as the host saves it.
    fn save(&self) -> Vec<u8> {
        let state: &clap_plugin_state = self.extension(CLAP_EXT_STATE);
        let mut bytes: Vec<u8> = Vec::new();
        let stream = clap_ostream {
            ctx: (&raw mut bytes).cast(),
            write: Some(write),
        };
        // SAFETY: as CLAP has it.
        assert!(unsafe { (state.save.unwrap())(self.plugin, &stream) });
        bytes
    }

    /// A state loaded as a host loads one: whether the plug-in took it.
    fn load(&self, bytes: &[u8]) -> bool {
        let state: &clap_plugin_state = self.extension(CLAP_EXT_STATE);
        let mut source = (bytes.to_vec(), 0usize);
        let stream = clap_istream {
            ctx: (&raw mut source).cast(),
            read: Some(read),
        };
        // SAFETY: as CLAP has it.
        unsafe { (state.load.unwrap())(self.plugin, &stream) }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        // SAFETY: as CLAP has it, once.
        unsafe {
            ((*self.plugin).stop_processing.unwrap())(self.plugin);
            ((*self.plugin).deactivate.unwrap())(self.plugin);
            ((*self.plugin).destroy.unwrap())(self.plugin);
        }
    }
}

/// nih-plug's CLAP state: its length (8 bytes, little-endian), then its JSON.
fn json_of(bytes: &[u8]) -> serde_json::Value {
    let n = u64::from_le_bytes(bytes[..8].try_into().unwrap()) as usize;
    serde_json::from_slice(&bytes[8..8 + n]).unwrap()
}

fn bytes_of(v: &serde_json::Value) -> Vec<u8> {
    let json = serde_json::to_vec(v).unwrap();
    let mut out = (json.len() as u64).to_le_bytes().to_vec();
    out.extend(json);
    out
}

fn envelope_instance(block: u32, mode: u8, priority: u8, retrigger: bool) -> Instance {
    let p = Instance::with_block(block);
    // SAFETY: the instance is stopped while the host changes its rendering mode.
    unsafe {
        ((*p.plugin).stop_processing.unwrap())(p.plugin);
        let render: &clap_plugin_render = p.extension(CLAP_EXT_RENDER);
        assert!((render.set.unwrap())(p.plugin, CLAP_RENDER_OFFLINE));
        assert!(((*p.plugin).start_processing.unwrap())(p.plugin));
    }
    let mut state = json_of(&p.save());
    state["fields"]["noise_seed"] = "51826".into();
    assert!(p.load(&bytes_of(&state)));
    p.audio(
        1,
        &[
            In::Param(0, "quality", 0.0),
            In::Param(0, "poly", f64::from(mode == 1)),
            In::Param(0, "unison", f64::from(mode == 2)),
            In::Param(0, "voices", 2.0),
            In::Param(0, "note_priority", f64::from(priority)),
            In::Param(0, "retrigger", f64::from(retrigger)),
            In::Param(0, "lock", 1.0),
            In::Param(0, "decay_on", 0.0),
            In::Param(0, "filter_attack", 0.0),
            In::Param(0, "loudness_attack", 0.0),
            In::Param(0, "filter_sustain", 0.5),
            In::Param(0, "loudness_sustain", 0.5),
        ],
    );
    // Every buffer size starts the sequence at the same circuit time.
    // SAFETY: the host resets a stopped instance before rendering.
    unsafe {
        ((*p.plugin).stop_processing.unwrap())(p.plugin);
        ((*p.plugin).reset.unwrap())(p.plugin);
        assert!(((*p.plugin).start_processing.unwrap())(p.plugin));
    }
    p
}

fn at(e: In, timing: u32) -> In {
    match e {
        In::Cc(_, ch, cc, value) => In::Cc(timing, ch, cc, value),
        In::Param(_, param, value) => In::Param(timing, param, value),
        In::Midi(_, data) => In::Midi(timing, data),
        In::Note(_, kind, id, ch, key, velocity) => In::Note(timing, kind, id, ch, key, velocity),
        In::Transport(_, playing) => In::Transport(timing, playing),
        In::Expression(_, x, id, ch, key, value) => In::Expression(timing, x, id, ch, key, value),
    }
}

fn render(p: &Instance, events: &[(u32, In)], length: u32) -> Vec<f32> {
    let mut audio = Vec::new();
    let mut event = 0;
    let mut start = 0;
    while start < length {
        let frames = p.block.min(length - start);
        let mut block = Vec::new();
        while event < events.len() && events[event].0 < start + frames {
            let (time, e) = events[event];
            assert!(time >= start, "events must be ordered");
            block.push(at(e, time - start));
            event += 1;
        }
        audio.extend(p.audio(frames, &block).1);
        start += frames;
    }
    assert_eq!(event, events.len());
    audio
}

fn rms(audio: &[f32]) -> f64 {
    (audio.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>() / audio.len() as f64).sqrt()
}

const SOUNDING: [usize; 14] = [
    0, 9_600, 17_040, 26_400, 28_800, 38_400, 43_200, 48_000, 55_200, 60_000, 67_200, 76_800,
    86_400, 93_600,
];

fn sounding(audio: &[f32], onset: usize) -> f64 {
    // CC123 cuts the penultimate note short: inspect it before that release.
    let (start, end) = if onset == 86_400 {
        (1_100, 1_550)
    } else {
        (2_400, 4_000)
    };
    rms(&audio[onset + start..onset + end])
}

#[test]
fn native_clap_zero_velocity_starts_and_sustains_but_midi_zero_velocity_releases() {
    for mode in 0..3 {
        for priority in 0..3 {
            for retrigger in [false, true] {
                let p = envelope_instance(256, mode, priority, retrigger);
                let audio = render(
                    &p,
                    &[
                        (0, In::Note(0, CLAP_EVENT_NOTE_ON, -1, 0, 60, 0.0)),
                        (12_000, In::Midi(0, [0x90, 60, 0])),
                    ],
                    24_000,
                );
                let held = rms(&audio[4_800..12_000]);
                let released = rms(&audio[19_200..24_000]);
                assert!(
                    held > 0.03,
                    "mode {mode}, priority {priority}, retrigger {retrigger}: no sustain ({held})"
                );
                assert!(
                    released < held * 0.2,
                    "MIDI zero did not release: {released} against {held}"
                );
            }
        }
    }
}

/// Native notes and MIDI bytes go through different CLAP decoding paths. Apart from zero
/// velocity they must play the same notes, including the source's order at equal times.
#[test]
fn envelope_sequences_match_in_both_clap_dialects_at_every_buffer_size() {
    let notes = [
        (0, 60, true),
        (7_200, 60, false),
        (9_600, 64, true),
        (16_800, 64, false),
        // Rapid repeats, then overlapping keys and a release in the following block.
        (17_040, 64, true),
        (24_000, 64, false),
        (26_400, 60, true),
        (28_800, 67, true),
        (31_200, 60, false),
        (36_000, 67, false),
        // Note-off/on at one sample, both orders, different keys and the same key.
        (38_400, 60, true),
        (43_200, 60, false),
        (43_200, 64, true),
        (48_000, 67, true),
        (48_000, 64, false),
        (52_800, 67, false),
        (55_200, 60, true),
        (60_000, 60, false),
        (60_000, 60, true),
        (64_800, 60, true),
        (64_800, 60, false),
        (67_200, 64, true),
        (72_000, 64, false),
        (76_800, 60, true),
        (81_600, 60, false),
        (86_400, 67, true),
        (91_200, 67, false),
    ];
    let mut midi: Vec<_> = notes
        .iter()
        .map(|&(t, k, on)| {
            (
                t,
                In::Midi(
                    0,
                    [if on { 0x90 } else { 0x80 }, k, if on { 100 } else { 0 }],
                ),
            )
        })
        .collect();
    // CC64 is currently unassigned, rather than a sustain-pedal implementation. Its events
    // must not swallow later notes. Transport alone does not release a live keyboard.
    midi.extend([
        (66_000, In::Cc(0, 0, 64, 127)),
        (73_000, In::Cc(0, 0, 64, 0)),
        (74_000, In::Transport(0, true)),
        (79_000, In::Transport(0, false)),
        (80_000, In::Transport(0, true)),
        (88_000, In::Cc(0, 0, 123, 0)),
        (92_000, In::Cc(0, 0, 120, 0)),
        (93_600, In::Midi(0, [0x90, 72, 100])),
        (98_400, In::Midi(0, [0x80, 72, 0])),
    ]);
    midi.sort_by_key(|(t, _)| *t);
    let native: Vec<_> = midi
        .iter()
        .map(|&(t, e)| {
            (
                t,
                match e {
                    In::Midi(_, [status, key, velocity]) => In::Note(
                        0,
                        if status == 0x90 {
                            CLAP_EVENT_NOTE_ON
                        } else {
                            CLAP_EVENT_NOTE_OFF
                        },
                        -1,
                        0,
                        i16::from(key),
                        f64::from(velocity) / 127.0,
                    ),
                    e => e,
                },
            )
        })
        .collect();
    for mode in 0..3 {
        for priority in 0..3 {
            for retrigger in [false, true] {
                let mut reference: Option<Vec<f32>> = None;
                for block in [32, 64, 128, 256, 512, 1024, 2048] {
                    let p = envelope_instance(block, mode, priority, retrigger);
                    let state = p.save();
                    // (Both instances load the state, so that both voices' noise, the mixer's
                    // own in LO too (decisions.md R-LO-HISS), starts from the same seeding.)
                    assert!(p.load(&state));
                    let a = render(&p, &midi, 105_600);
                    if let Some(reference) = &reference {
                        // A released POLY voice is retired at a block boundary, so its next
                        // oscillator phase can differ. Compare the note's level, not phase.
                        for t in SOUNDING {
                            let level = sounding(&a, t);
                            let before = sounding(reference, t);
                            let db = 20.0 * (level / before).log10();
                            assert!(
                                db.abs() < 3.0,
                                "buffer size changes note {t} by {db} dB, mode {mode}, priority {priority}, retrigger {retrigger}, block {block}"
                            );
                        }
                    } else {
                        reference = Some(a.clone());
                    }
                    let q = envelope_instance(block, mode, priority, retrigger);
                    assert!(q.load(&state));
                    let b = render(&q, &native, 105_600);
                    assert_eq!(
                        a, b,
                        "mode {mode}, priority {priority}, retrigger {retrigger}, block {block}"
                    );
                    for t in SOUNDING {
                        assert!(
                            sounding(&a, t) > 0.03,
                            "silent note at {t}, mode {mode}, priority {priority}, retrigger {retrigger}, block {block}"
                        );
                    }
                    // The equal-key on/off pair, CC123 and the final ordinary release must
                    // actually lower the output, not merely agree between the dialects.
                    for (start, end, held) in [
                        (66_800, 67_150, 60_000),
                        (90_400, 91_100, 86_400),
                        (103_200, 105_600, 93_600),
                    ] {
                        let quiet = rms(&a[start..end]);
                        let held_level = sounding(&a, held);
                        assert!(
                            quiet < held_level * 0.3,
                            "note did not release at {start}: {quiet} against {held_level}, mode {mode}, priority {priority}, retrigger {retrigger}, block {block}"
                        );
                    }
                }
            }
        }
    }
}

/// `instance`'s state with its MIDI assignments' field set to `table` (removed: None).
fn with_table(instance: &Instance, table: Option<&str>) -> Vec<u8> {
    let mut state = json_of(&instance.save());
    let fields = state["fields"].as_object_mut().unwrap();
    match table {
        Some(t) => {
            fields.insert("midi_map".into(), serde_json::Value::String(t.into()));
        }
        None => {
            fields.remove("midi_map");
        }
    }
    bytes_of(&state)
}

/// OSCILLATOR-1 RANGE on CC 21 and CUTOFF on CC 74, channel 1; EMPHASIS on CC 71, channel 2 (in
/// the order the plug-in writes them: the controls').
const TABLE: &str = r#"{"version":1,"assignments":[{"param":"osc1_range","channel":1,"cc":21},{"param":"cutoff","channel":1,"cc":74},{"param":"emphasis","channel":2,"cc":71}]}"#;

/// A learned controller sets its parameter at its message's time, and the host is told: a
/// parameter value at that time, flagged not to be recorded (and not as a live gesture of the
/// user's), the value as set; a controller on another channel, the modulation wheel and one not
/// assigned tell nothing.
#[test]
fn a_learned_controller_is_set_at_its_time_and_told_not_to_be_recorded() {
    let p = Instance::new();
    assert!(p.load(&with_table(&p, Some(TABLE))));
    let before = p.rescans.load(Ordering::SeqCst);
    let told = p.process(&[
        In::Cc(10, 1, 74, 127),
        In::Cc(20, 0, 1, 127),
        In::Cc(30, 0, 75, 127),
        In::Cc(100, 0, 74, 127),
        In::Cc(150, 0, 21, 127),
    ]);
    assert_eq!(
        told,
        [
            Out {
                time: 100,
                flags: CLAP_EVENT_DONT_RECORD,
                param: id("cutoff"),
                value: 1.0
            },
            Out {
                time: 150,
                flags: CLAP_EVENT_DONT_RECORD,
                param: id("osc1_range"),
                value: 5.0
            },
        ]
    );
    assert_eq!(p.value("cutoff"), 1.0);
    assert_eq!(p.value("osc1_range"), 5.0);
    // And no rescan of the parameters is asked for (this host's process thread is its main
    // thread, so one would have come at once): the Audio Unit's wrapper rebuilds its whole
    // parameter list for one.
    assert_eq!(p.rescans.load(Ordering::SeqCst), before);
    // The same value again: nothing changes, nothing is told.
    assert!(p.process(&[In::Cc(5, 0, 74, 127)]).is_empty());
    // The host's own automation is not echoed back.
    assert!(p.process(&[In::Param(5, "cutoff", 0.25)]).is_empty());
    assert_eq!(p.value("cutoff"), 0.25);
}

/// At one sample, the host's automation of a parameter is set before a learned controller of
/// it, whichever the host lists first; later in time, whichever comes later holds.
#[test]
fn at_one_sample_automation_is_set_before_a_learned_controller() {
    let p = Instance::new();
    assert!(p.load(&with_table(&p, Some(TABLE))));
    p.process(&[In::Cc(64, 1, 71, 127), In::Param(64, "emphasis", 0.3)]);
    assert_eq!(p.value("emphasis"), 1.0, "the controller, listed first");
    p.process(&[In::Param(64, "emphasis", 0.3), In::Cc(64, 1, 71, 0)]);
    assert_eq!(p.value("emphasis"), 0.0, "the controller, listed second");
    p.process(&[In::Cc(64, 1, 71, 127), In::Param(96, "emphasis", 0.3)]);
    assert!(
        (p.value("emphasis") - 0.3).abs() < 1e-6,
        "the automation, later"
    );
    p.process(&[In::Param(32, "emphasis", 0.3), In::Cc(96, 1, 71, 0)]);
    assert_eq!(p.value("emphasis"), 0.0, "the controller, later");
}

/// The assignments go out with the state the host saves and come back with it (into another
/// instance); a state from before MIDI Learn leaves an instance with none; a table not
/// understood is none, and the state still loads.
#[test]
fn the_assignments_go_out_and_come_back_with_the_state() {
    let a = Instance::new();
    assert!(a.load(&with_table(&a, Some(TABLE))));
    let saved = a.save();
    let table = json_of(&saved)["fields"]["midi_map"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(table, TABLE);
    let b = Instance::new();
    assert!(
        b.process(&[In::Cc(0, 0, 74, 127)]).is_empty(),
        "a new instance: none"
    );
    assert!(b.load(&saved));
    assert_eq!(b.process(&[In::Cc(0, 0, 74, 127)]).len(), 1);
    // Older, with no table: none, though B had them.
    assert!(b.load(&with_table(&a, None)));
    assert!(b.process(&[In::Cc(0, 0, 74, 0)]).is_empty());
    assert_eq!(
        b.value("cutoff"),
        0.5,
        "the sound as the state has it (A's)"
    );
    assert!(b.load(&saved));
    assert!(b.load(&with_table(&a, Some("{\"version\":9,"))));
    assert!(
        b.process(&[In::Cc(0, 0, 74, 0), In::Cc(0, 1, 71, 0)])
            .is_empty()
    );
}

/// Two instances, one with assignments: the other's controllers do nothing.
#[test]
fn instances_keep_their_own_assignments() {
    let a = Instance::new();
    let b = Instance::new();
    assert!(a.load(&with_table(&a, Some(TABLE))));
    assert!(b.process(&[In::Cc(0, 0, 74, 127)]).is_empty());
    assert_eq!(b.value("cutoff"), 0.5);
    assert_eq!(a.process(&[In::Cc(0, 0, 74, 127)]).len(), 1);
}

/// The serialized state takes the same versioned migration as an actual host load, and
/// a second save/load keeps its reading. Host automation still sets the new travel.
#[test]
fn entropy_saved_by_an_old_version_moves_once_and_automation_uses_the_new_range() {
    let a = Instance::new();
    assert!((a.value("entropy") - 0.05).abs() < 1e-7);
    for (old, new) in [(0.0, 5.0), (40.0, 43.0), (100.0, 100.0)] {
        let mut state = json_of(&a.save());
        state["version"] = "0.2.6".into();
        state["params"]["entropy"] = serde_json::json!({"f32": old});
        assert!(a.load(&bytes_of(&state)));
        assert!((a.value("entropy") - new / 100.0).abs() < 1e-7);
        let saved = a.save();
        assert_eq!(json_of(&saved)["version"], env!("CARGO_PKG_VERSION"));
        let b = Instance::new();
        assert!(b.load(&saved));
        assert_eq!(a.value("entropy"), b.value("entropy"));
    }
    a.process(&[In::Param(0, "entropy", 0.0)]);
    assert_eq!(a.value("entropy"), 0.0);
    let b = Instance::new();
    assert!(b.load(&a.save()));
    assert_eq!(b.value("entropy"), 0.0);
}

/// Scanner-style construction, live processing and offline rendering all leave the
/// shared What's New history alone when no editor opens. Child processes isolate the
/// settings override from the other host tests.
#[test]
fn no_editor_means_no_release_notes_during_scanning_or_rendering() {
    if std::env::var_os("CA72_NOTES_HEADLESS_CHILD").is_some() {
        use clap_sys::ext::render::{CLAP_EXT_RENDER, CLAP_RENDER_OFFLINE, clap_plugin_render};
        let path = ca72_plugin::settings::file();
        let before = std::fs::read(&path).ok();
        for _ in 0..3 {
            let instance = Instance::new();
            let state = instance.save();
            instance.process(&[]);
            let render: &clap_plugin_render = instance.extension(CLAP_EXT_RENDER);
            // SAFETY: the live instance's CLAP render extension, on its main test thread.
            assert!(unsafe { (render.set.unwrap())(instance.plugin, CLAP_RENDER_OFFLINE) });
            instance.process(&[]);
            assert_eq!(
                instance.save(),
                state,
                "processing adds no release-note project state"
            );
        }
        assert_eq!(std::fs::read(&path).ok(), before, "no editor, no claim");
        return;
    }
    let dir = tempfile::tempdir().expect("shared data folder");
    for existing in [false, true] {
        if existing {
            let folder = dir.path().join("CA-72");
            std::fs::create_dir_all(&folder).expect("settings folder");
            std::fs::write(
                folder.join("settings.toml"),
                "last_version_shown = '0.2.3'\n",
            )
            .expect("previous version");
        }
        let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "no_editor_means_no_release_notes_during_scanning_or_rendering",
                "--nocapture",
            ])
            .env("CA72_NOTES_HEADLESS_CHILD", "1")
            .env("IDLE_FOUNDRY_SHARED", dir.path())
            .env("CA72_SEATS_DIR", dir.path().join("seats"))
            .output()
            .expect("headless host");
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// Run in a fresh process so each instance really reads its computer assignments at birth,
/// without changing this test runner's environment or any installed settings.
#[test]
fn midi_preset_actions_work_through_clap_with_no_editor() {
    use std::time::{Duration, Instant};
    if std::env::var_os("CA72_PRESET_ACTION_CHILD").is_none() {
        let target = std::env::var_os("CARGO_TARGET_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap().join("target"));
        std::fs::create_dir_all(&target).unwrap();
        let dir = tempfile::tempdir_in(target).unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "midi_preset_actions_work_through_clap_with_no_editor",
                "--nocapture",
            ])
            .env("CA72_PRESET_ACTION_CHILD", "1")
            .env("IDLE_FOUNDRY_SHARED", dir.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let shared = std::path::PathBuf::from(std::env::var_os("IDLE_FOUNDRY_SHARED").unwrap());
    std::fs::create_dir_all(shared.join("CA-72")).unwrap();
    std::fs::write(shared.join("CA-72/settings.toml"), "[preset_actions.next_preset]\nchannel = 16\nnote = 127\n[preset_actions.previous_preset]\nchannel = 15\ncc = 119\n").unwrap();
    let entries = ca72_plugin::library::Library::at(shared.join("CA-72/Presets"))
        .entries()
        .0;
    let p = Instance::new();
    let name = || {
        let state = json_of(&p.save());
        serde_json::from_str::<String>(state["fields"]["preset"].as_str().unwrap()).unwrap()
    };
    let wait = |expected: &str| {
        let start = Instant::now();
        let mut applied = false;
        loop {
            let told = p.process(&[]);
            if !told.is_empty() {
                assert!(
                    told.iter()
                        .all(|event| event.time == 0 && event.flags & CLAP_EVENT_DONT_RECORD != 0)
                );
                applied = true;
            }
            if name() == expected && applied {
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "preset {expected} not applied: {}",
                name()
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    p.process(&[In::Midi(0, [0x9f, 127, 127])]);
    wait(&entries[0].sound.name);
    for _ in 0..3 {
        assert!(p.process(&[In::Midi(0, [0x9f, 127, 127])]).is_empty());
        assert_eq!(name(), entries[0].sound.name);
    }
    assert!(p.process(&[In::Midi(0, [0x8f, 127, 0])]).is_empty());
    p.process(&[In::Note(0, CLAP_EVENT_NOTE_ON, 4, 15, 127, 1.0)]);
    wait(&entries[1].sound.name);
    assert!(
        p.process(&[In::Note(0, CLAP_EVENT_NOTE_OFF, 4, 15, 127, 0.0)])
            .is_empty()
    );
    p.process(&[In::Cc(0, 14, 119, 127)]);
    wait(&entries[0].sound.name);
    for v in [127, 100, 64, 63, 0] {
        assert!(p.process(&[In::Cc(0, 14, 119, v)]).is_empty());
        assert_eq!(name(), entries[0].sound.name);
    }
    p.process(&[In::Cc(0, 14, 119, 64)]);
    wait(&entries.last().unwrap().sound.name);
    let state = json_of(&p.save());
    assert!(
        !state["fields"]["midi_map"]
            .as_str()
            .unwrap()
            .contains("preset")
    );
}

/// A note's expression through CLAP's note expressions (pressure, brightness, tuning: what
/// Bitwig sends an MPE controller's as) and the same through MIDI in an MPE zone (the MPE
/// Configuration Message, then each note on its member channel), both into the real CLAP
/// entry: the same samples to the bit, the one instrument and POLY; and audibly not the
/// notes without it (decisions.md R-MPE).
#[test]
fn clap_note_expressions_and_mpe_midi_play_the_same_samples() {
    // MPE's bend for MIDI's 14-bit value, as the plug-in reads it (over 48 semitones).
    let bend_of = |v: u16| ((f32::from(v) / 16_383.0) * 2.0 - 1.0) * 48.0;
    let seven = |v: u8| f64::from(f32::from(v) / 127.0);
    let bend = 10_240u16;
    let (lsb, msb) = ((bend & 127) as u8, (bend >> 7) as u8);
    for mode in [0u8, 1] {
        let instance = || {
            let p = envelope_instance(256, mode, 0, false);
            p.audio(
                1,
                &[
                    In::Param(0, "pressure_filter", 0.8),
                    In::Param(0, "timbre_filter", 0.3),
                    In::Param(0, "pressure_mod", 0.9),
                ],
            );
            p
        };
        let x = |t: u32, id: i32, note: i32, key: i16, v: f64| {
            (t, In::Expression(0, id, note, 0, key, v))
        };
        let on =
            |t: u32, note: i32, key: i16| (t, In::Note(0, CLAP_EVENT_NOTE_ON, note, 0, key, 1.0));
        let off =
            |t: u32, note: i32, key: i16| (t, In::Note(0, CLAP_EVENT_NOTE_OFF, note, 0, key, 0.0));
        let native = [
            on(0, 1, 60),
            x(0, CLAP_NOTE_EXPRESSION_PRESSURE, 1, 60, seven(96)),
            x(0, CLAP_NOTE_EXPRESSION_BRIGHTNESS, 1, 60, seven(100)),
            x(
                0,
                CLAP_NOTE_EXPRESSION_TUNING,
                1,
                60,
                f64::from(bend_of(bend)),
            ),
            on(4_800, 2, 67),
            x(9_600, CLAP_NOTE_EXPRESSION_PRESSURE, 1, 60, seven(20)),
            x(12_000, CLAP_NOTE_EXPRESSION_PRESSURE, 2, 67, seven(127)),
            off(16_800, 1, 60),
            off(19_200, 2, 67),
        ];
        let midi = [
            (0, In::Cc(0, 0, 101, 0)),
            (0, In::Cc(0, 0, 100, 6)),
            (0, In::Cc(0, 0, 6, 15)),
            (0, In::Midi(0, [0xe1, lsb, msb])),
            (0, In::Cc(0, 1, 74, 100)),
            (0, In::Midi(0, [0xd1, 96, 0])),
            (0, In::Midi(0, [0x91, 60, 127])),
            (4_800, In::Midi(0, [0x92, 67, 127])),
            (9_600, In::Midi(0, [0xd1, 20, 0])),
            (12_000, In::Midi(0, [0xd2, 127, 0])),
            (16_800, In::Midi(0, [0x81, 60, 0])),
            (19_200, In::Midi(0, [0x82, 67, 0])),
        ];
        let plain = [
            on(0, 1, 60),
            on(4_800, 2, 67),
            off(16_800, 1, 60),
            off(19_200, 2, 67),
        ];
        let a = render(&instance(), &native, 24_000);
        let b = render(&instance(), &midi, 24_000);
        let c = render(&instance(), &plain, 24_000);
        assert!(rms(&a) > 0.01, "mode {mode}: sounding");
        assert!(
            a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()),
            "mode {mode}: CLAP's note expressions and MPE's MIDI differ"
        );
        let apart = a
            .iter()
            .zip(&c)
            .map(|(x, y)| f64::from(x - y).powi(2))
            .sum::<f64>();
        assert!(
            apart > 1.0,
            "mode {mode}: the expression is heard ({apart})"
        );
    }
}

/// TIMBRE REST through the real CLAP entry, as the computer's `settings.toml` chooses it
/// (decisions.md R-MPE), each in a process of its own with a shared folder of its own:
/// at "centre" (and with none set) a host's brightness at 0.5 is at rest, the notes the same to
/// the bit as with none, TIMBRE TO FILTER turned up; at "zero" it is half the reach, audibly not;
/// at "start", a note's first brightness is where it begins, so 0.5 then 0.9 is heard and 0.5
/// alone is not.
#[test]
fn clap_brightness_rests_where_the_computers_setting_says() {
    if let Ok(word) = std::env::var("CA72_TIMBRE_REST_CHILD") {
        let instance = || {
            let p = envelope_instance(256, 1, 0, false);
            p.audio(1, &[In::Param(0, "timbre_filter", 2.0)]);
            p
        };
        let x = |t: u32, note: i32, key: i16, v: f64| {
            (
                t,
                In::Expression(0, CLAP_NOTE_EXPRESSION_BRIGHTNESS, note, 0, key, v),
            )
        };
        let on =
            |t: u32, note: i32, key: i16| (t, In::Note(0, CLAP_EVENT_NOTE_ON, note, 0, key, 1.0));
        let off =
            |t: u32, note: i32, key: i16| (t, In::Note(0, CLAP_EVENT_NOTE_OFF, note, 0, key, 0.0));
        let plain = [
            on(0, 1, 60),
            on(4_800, 2, 67),
            off(16_800, 1, 60),
            off(19_200, 2, 67),
        ];
        let half = [
            on(0, 1, 60),
            x(0, 1, 60, 0.5),
            on(4_800, 2, 67),
            x(4_800, 2, 67, 0.5),
            off(16_800, 1, 60),
            off(19_200, 2, 67),
        ];
        let moved = [
            on(0, 1, 60),
            x(0, 1, 60, 0.5),
            x(2_400, 1, 60, 0.9),
            on(4_800, 2, 67),
            off(16_800, 1, 60),
            off(19_200, 2, 67),
        ];
        let a = render(&instance(), &plain, 24_000);
        let same = |b: &[f32]| a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits());
        assert!(rms(&a) > 0.01, "{word}: sounding");
        let b = render(&instance(), &half, 24_000);
        assert_eq!(same(&b), word != "zero", "{word}: brightness 0.5");
        let c = render(&instance(), &moved, 24_000);
        assert!(!same(&c), "{word}: brightness moved from 0.5 to 0.9");
        return;
    }
    for word in ["", "centre", "zero", "start"] {
        let dir = tempfile::tempdir().expect("shared data folder");
        if !word.is_empty() {
            let folder = dir.path().join("CA-72");
            std::fs::create_dir_all(&folder).expect("settings folder");
            std::fs::write(
                folder.join("settings.toml"),
                format!("timbre_rest = \"{word}\"\n"),
            )
            .expect("settings");
        }
        let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "clap_brightness_rests_where_the_computers_setting_says",
                "--nocapture",
            ])
            .env("CA72_TIMBRE_REST_CHILD", word)
            .env("IDLE_FOUNDRY_SHARED", dir.path())
            .env("CA72_SEATS_DIR", dir.path().join("seats"))
            .output()
            .expect("child host");
        let told = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && told.contains("1 passed"),
            "{word}:\n{told}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// Host display text must remain stable after typing it back, including waveform blends,
/// rectangle duties and LFO rate units. Rounded displays need not preserve the original value.
#[test]
fn every_parameter_display_roundtrips_through_the_host_api() {
    let p = Instance::with_rate_block(RATE, BLOCK);
    let params: &clap_plugin_params = p.extension(CLAP_EXT_PARAMS);
    // SAFETY: the instance and CLAP output buffers live through every call; zero is valid
    // initial storage for clap_param_info's scalar fields, pointers and character arrays.
    unsafe {
        for index in 0..(params.count.unwrap())(p.plugin) {
            let mut info: clap_param_info = std::mem::zeroed();
            assert!((params.get_info.unwrap())(p.plugin, index, &mut info));
            let name = CStr::from_ptr(info.name.as_ptr());
            // Dense sampling includes rounded 0%/100% blends and rectangle aliases.
            let steps = if name.to_bytes().ends_with(b" Shape") {
                10_000
            } else {
                100
            };
            if name.to_bytes().starts_with(b"LFO ") && name.to_bytes().ends_with(b" Shape") {
                let mut travel = f64::NAN;
                assert!((params.text_to_value.unwrap())(
                    p.plugin,
                    info.id,
                    c"0.25".as_ptr(),
                    &mut travel
                ));
                assert_eq!(travel, 0.25, "bare LFO shape numbers retain travel units");
                assert!(!(params.text_to_value.unwrap())(
                    p.plugin,
                    info.id,
                    c"NaN".as_ptr(),
                    &mut travel
                ));
            }
            for step in 0..=steps {
                let value = info.min_value
                    + (info.max_value - info.min_value) * f64::from(step) / f64::from(steps);
                let mut text = [0 as c_char; 256];
                let mut again = [0 as c_char; 256];
                let mut back = f64::NAN;
                let mut last = f64::NAN;
                assert!((params.value_to_text.unwrap())(
                    p.plugin,
                    info.id,
                    value,
                    text.as_mut_ptr(),
                    text.len() as u32
                ));
                let shown = CStr::from_ptr(text.as_ptr());
                assert!(
                    (params.text_to_value.unwrap())(p.plugin, info.id, text.as_ptr(), &mut back),
                    "{name:?} {value}: {shown:?}"
                );
                assert!(
                    back.is_finite() && (info.min_value..=info.max_value).contains(&back),
                    "{name:?} {shown:?} -> {back}"
                );
                assert!((params.value_to_text.unwrap())(
                    p.plugin,
                    info.id,
                    back,
                    again.as_mut_ptr(),
                    again.len() as u32
                ));
                assert_eq!(
                    shown,
                    CStr::from_ptr(again.as_ptr()),
                    "{name:?} {value} -> {back}"
                );
                assert!((params.text_to_value.unwrap())(
                    p.plugin,
                    info.id,
                    again.as_ptr(),
                    &mut last
                ));
                assert_eq!(back, last, "{name:?} {shown:?}");
            }
        }
    }
}

/// A host may make, start and play an instance on a thread of its own, with less stack than a
/// main thread's (a macOS secondary thread has 512 KB; a host's own threads may have less): the
/// instance's whole life, made, activated, played at each QUALITY in POLY with GLIDE moving,
/// its state loaded, deactivated and destroyed, fits in 256 KB (decisions.md R-HOST-STACK:
/// making one took 576 to 640 KB in a release build and over 1 MB in a debug build, its ten
/// spare voices built on the stack).
#[test]
fn an_instance_lives_on_a_host_thread_of_256_kb() {
    std::thread::Builder::new()
        .stack_size(256 << 10)
        .spawn(|| {
            let p = Instance::with_rate_block(RATE, BLOCK);
            let state = p.save();
            for quality in [0.0, 1.0, 2.0] {
                p.audio(
                    BLOCK,
                    &[
                        In::Param(0, "quality", quality),
                        In::Param(0, "poly", 1.0),
                        In::Param(0, "glide_on", 1.0),
                        In::Note(0, CLAP_EVENT_NOTE_ON, -1, 0, 48, 1.0),
                        In::Note(0, CLAP_EVENT_NOTE_ON, -1, 0, 55, 1.0),
                    ],
                );
                for k in 0..4u32 {
                    p.audio(
                        BLOCK,
                        &[
                            In::Param(0, "glide", f64::from(k) / 4.0),
                            In::Param(BLOCK / 2, "glide", f64::from(k + 1) / 4.0),
                            In::Note(BLOCK / 2, CLAP_EVENT_NOTE_ON, -1, 0, 60 + k as i16, 1.0),
                        ],
                    );
                }
                assert!(p.load(&state));
                p.audio(BLOCK, &[]);
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

/// A state saved at ULTRA, with UNISON and with POLY, loaded on a host's thread of 256 KB, and an
/// instance activated with it there: ULTRA's split crews, made as the workers start, took some
/// 250 KB of the thread's stack and overflowed it (decisions.md R-ULTRA-STACK; found trying to
/// reproduce issue #58). Each load starts the workers again.
#[test]
fn a_state_saved_at_ultra_loads_on_a_host_thread_of_256_kb() {
    let states: Vec<Vec<u8>> = [("unison", 1.0, 0.0), ("poly", 0.0, 1.0)]
        .iter()
        .map(|&(_, unison, poly)| {
            session(&[
                ("quality", 2.0),
                ("unison", unison),
                ("poly", poly),
                ("voices", 4.0),
            ])
        })
        .collect();
    std::thread::Builder::new()
        .stack_size(256 << 10)
        .spawn(move || {
            for state in &states {
                let p = Instance::with_rate_block(22_050.0, 32);
                for _ in 0..3 {
                    assert!(p.load(state));
                    p.audio(32, &[In::Note(0, CLAP_EVENT_NOTE_ON, -1, 0, 48, 1.0)]);
                }
                drop(p);
                // (Made, its state loaded, and activated again with it, as a host opens a
                // project.)
                let p = Instance::with_rate_block(22_050.0, 32);
                assert!(p.load(state));
                // SAFETY: as CLAP has it: stopped and deactivated before it is activated again.
                unsafe {
                    ((*p.plugin).stop_processing.unwrap())(p.plugin);
                    ((*p.plugin).deactivate.unwrap())(p.plugin);
                    assert!(((*p.plugin).activate.unwrap())(p.plugin, 22_050.0, 1, 32));
                    assert!(((*p.plugin).start_processing.unwrap())(p.plugin));
                }
                p.audio(32, &[In::Note(0, CLAP_EVENT_NOTE_ON, -1, 0, 48, 1.0)]);
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

/// GLIDE moved as hard as a host can move it (issue #58: a crash in FL Studio on Windows 11
/// "when messing with glide"): its knob set at many points of every block, at its ends, by a
/// denormal's breadth and, with `CA72_HOSTILE` set, out of its range and not a number; its switch
/// and its RANGE (NORMAL, LONG) flipped; keys played legato and detached, the bend moving; NOTE
/// PRIORITY, RETRIGGER, POLY, UNISON, VOICES and QUALITY (ULTRA too) changed; a saved state (a
/// host's preset) loaded while it glides; the host's calls of the block's size, of none and of
/// one frame, and every other size between; every other instance rendering offline.
/// `CA72_RATES` (22050,44100,48000,88200,96000,176400,192000), `CA72_BLOCKS` (32,256,1024),
/// `CA72_SECONDS` (2; ULTRA a tenth of it), `CA72_STACK` (the thread's stack, bytes; 8 MB).
/// Every sample must be finite and within ±16.
///
/// By hand, in a debug build (overflow checks and debug assertions):
/// `cargo test -p ca72-plugin --test clap_host -- --ignored --nocapture glide_stress`.
#[test]
#[ignore = "a stress test, run by hand"]
fn glide_stress() {
    // (On a thread of `CA72_STACK` bytes, as a host's own thread may be smaller than a main
    // thread's: the plug-in's every call is made there.)
    let stack: usize = std::env::var("CA72_STACK").map_or(8 << 20, |s| s.parse().unwrap());
    std::thread::Builder::new()
        .stack_size(stack)
        .spawn(|| {
            fn list<T: std::str::FromStr + Clone>(name: &str, default: &[T]) -> Vec<T> {
                std::env::var(name).map_or_else(
                    |_| default.to_vec(),
                    |s| {
                        s.split(',')
                            .map(|v| v.trim().parse().ok().unwrap())
                            .collect()
                    },
                )
            }
            glide_stress_here(
                &list(
                    "CA72_RATES",
                    &[
                        22_050.0, 44_100.0, 48_000.0, 88_200.0, 96_000.0, 176_400.0, 192_000.0,
                    ],
                ),
                &list("CA72_BLOCKS", &[32, 256, 1024]),
                list("CA72_SECONDS", &[2.0])[0],
                std::env::var_os("CA72_HOSTILE").is_some(),
            );
        })
        .unwrap()
        .join()
        .unwrap();
}

/// [`glide_stress`] in brief, as every test run has it: a fifth of a second of each, at the
/// lowest and highest rates the plug-in is played at, in blocks of up to 64 frames, GLIDE's
/// values out of its range and not a number among the rest (issue #58).
#[test]
fn glide_moved_as_hard_as_a_host_can_plays_on_at_every_rate() {
    glide_stress_here(&[22_050.0, 192_000.0], &[64], 0.2, true);
}

fn glide_stress_here(rates: &[f64], blocks: &[u32], seconds: f64, hostile: bool) {
    let mut seed = 0x5EED_u64;
    let mut rand = move || {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) as f64 / f64::from(1u32 << 31)
    };
    let mut made = 0u32;
    for &rate in rates {
        for &block in blocks {
            for quality in [0.0, 1.0, 2.0] {
                for mode in 0..3 {
                    // (Made on a thread of a main thread's stack where `CA72_MADE_BIG` is set,
                    // so that `CA72_STACK` measures what the rest needs.)
                    let p = if std::env::var_os("CA72_MADE_BIG").is_some() {
                        struct Made(Instance);
                        // SAFETY: the instance is made there and only used here after.
                        unsafe impl Send for Made {}
                        std::thread::Builder::new()
                            .stack_size(8 << 20)
                            .spawn(move || Made(Instance::with_rate_block(rate, block)))
                            .unwrap()
                            .join()
                            .unwrap()
                            .0
                    } else {
                        Instance::with_rate_block(rate, block)
                    };
                    // (Every other instance as a host's bounce renders it.)
                    made += 1;
                    if made.is_multiple_of(2) {
                        // SAFETY: the instance is stopped while the host changes its rendering
                        // mode.
                        unsafe {
                            ((*p.plugin).stop_processing.unwrap())(p.plugin);
                            let render: &clap_plugin_render = p.extension(CLAP_EXT_RENDER);
                            assert!((render.set.unwrap())(p.plugin, CLAP_RENDER_OFFLINE));
                            assert!(((*p.plugin).start_processing.unwrap())(p.plugin));
                        }
                    }
                    p.audio(
                        1,
                        &[
                            In::Param(0, "quality", quality),
                            In::Param(0, "poly", f64::from(mode == 1)),
                            In::Param(0, "unison", f64::from(mode == 2)),
                            In::Param(0, "voices", 4.0),
                            In::Param(0, "glide_on", 1.0),
                            In::Param(0, "glide", 0.6),
                            In::Param(0, "emphasis", 0.8),
                        ],
                    );
                    let preset = p.save();
                    let length = if quality == 2.0 {
                        seconds / 10.0
                    } else {
                        seconds
                    };
                    let total = (length * rate) as u32;
                    let (mut start, mut held, mut peak) = (0u32, Vec::<i16>::new(), 0.0f32);
                    let mut quality = quality;
                    while start < total {
                        // (The host's call: its block, or none, one frame or any other size.)
                        let r = rand();
                        let frames = if r < 0.6 {
                            block
                        } else if r < 0.7 {
                            0
                        } else if r < 0.8 {
                            1
                        } else {
                            (rand() * f64::from(block)) as u32
                        }
                        .min(total - start);
                        let mut ev: Vec<In> = Vec::new();
                        let mut times: Vec<u32> = (0..(rand() * 9.0) as u32)
                            .map(|_| (rand() * f64::from(frames)) as u32)
                            .collect();
                        times.sort_unstable();
                        for t in times {
                            let r = rand();
                            let v = if r < 0.1 {
                                0.0
                            } else if r < 0.2 {
                                1.0
                            } else if r < 0.25 {
                                1e-40
                            } else if hostile && r < 0.3 {
                                [f64::NAN, -0.5, 1.5, f64::INFINITY, -1e300]
                                    [(rand() * 5.0) as usize]
                            } else {
                                rand()
                            };
                            ev.push(In::Param(t, "glide", v));
                            let r = rand();
                            if r < 0.04 {
                                ev.push(In::Param(t, "glide_on", f64::from(rand() < 0.7)));
                            } else if r < 0.07 {
                                ev.push(In::Param(t, "glide_range", f64::from(rand() < 0.5)));
                            } else if r < 0.25 {
                                // A key, legato over those held or alone.
                                let key = 24 + (rand() * 84.0) as i16;
                                if !held.contains(&key) {
                                    ev.push(In::Note(t, CLAP_EVENT_NOTE_ON, -1, 0, key, 1.0));
                                    held.push(key);
                                }
                            } else if r < 0.4 && !held.is_empty() {
                                let key = held.remove((rand() * held.len() as f64) as usize);
                                ev.push(In::Note(t, CLAP_EVENT_NOTE_OFF, -1, 0, key, 0.0));
                            } else if r < 0.45 {
                                let bend = (rand() * 16_383.0) as u16;
                                ev.push(In::Midi(
                                    t,
                                    [0xe0, (bend & 0x7f) as u8, (bend >> 7) as u8],
                                ));
                            }
                        }
                        let r = rand();
                        if r < 0.03 {
                            ev.push(In::Param(0, "note_priority", (rand() * 3.0).floor()));
                        } else if r < 0.06 {
                            ev.push(In::Param(0, "retrigger", f64::from(rand() < 0.5)));
                        } else if r < 0.07 {
                            ev.push(In::Param(0, "voices", 2.0 + (rand() * 9.0).floor()));
                        } else if r < 0.075 {
                            ev.push(In::Param(0, "poly", f64::from(rand() < 0.5)));
                        } else if r < 0.08 {
                            ev.push(In::Param(0, "unison", f64::from(rand() < 0.5)));
                        } else if r < 0.083 && quality < 2.0 {
                            // (Into ULTRA too, for the render's last tenth: it costs the most.)
                            let most = if start * 10 > total * 9 { 3.0 } else { 2.0 };
                            quality = (rand() * most).floor();
                            ev.push(In::Param(0, "quality", quality));
                        }
                        // (Sorted by time, as a host gives them.)
                        let time = |e: &In| match *e {
                            In::Param(t, ..) | In::Note(t, ..) | In::Midi(t, ..) => t,
                            In::Cc(t, ..) | In::Transport(t, ..) | In::Expression(t, ..) => t,
                        };
                        ev.sort_by_key(time);
                        if rand() < 0.01 && std::env::var_os("CA72_NO_LOAD").is_none() {
                            // A host's preset loaded while the keys are held and GLIDE glides.
                            assert!(p.load(&preset));
                        }
                        // (A call of no frames brings no events: a host gives them with the next.)
                        if frames == 0 {
                            ev.clear();
                        }
                        let (_, l, r) = p.stereo(frames, &ev);
                        for x in l.iter().chain(&r) {
                            assert!(x.is_finite());
                            peak = peak.max(x.abs());
                        }
                        start += frames;
                    }
                    eprintln!(
                        "{rate} Hz, {block} frames, QUALITY {quality}, mode {mode}: peak {peak:.3}"
                    );
                    assert!(peak < 16.0, "the output ran away: {peak}");
                }
            }
        }
    }
}

/// GLIDE RANGE goes through the host's automation and state, including its stable enum ids;
/// a state from before it replaces LONG with NORMAL. A learned CC steps at the switch's
/// threshold, without turning GLIDE's knob (decisions.md R-GLIDE-LONG).
#[test]
fn glide_range_is_automated_learned_and_saved_with_the_session() {
    let a = Instance::new();
    assert_eq!(a.value("glide_range"), 0.0);
    a.process(&[In::Param(0, "glide_range", 1.0)]);
    assert_eq!(a.value("glide_range"), 1.0);
    let saved = a.save();
    assert_eq!(
        json_of(&saved)["params"]["glide_range"],
        serde_json::json!({"string": "long"})
    );
    let b = Instance::new();
    assert!(b.load(&saved));
    assert_eq!(b.value("glide_range"), 1.0);
    assert_eq!(
        json_of(&b.save())["params"]["glide_range"],
        serde_json::json!({"string": "long"})
    );
    let mut old = json_of(&saved);
    old["params"].as_object_mut().unwrap().remove("glide_range");
    assert!(b.load(&bytes_of(&old)));
    assert_eq!(b.value("glide_range"), 0.0);
    assert_eq!(
        json_of(&b.save())["params"]["glide_range"],
        serde_json::json!({"string": "normal"})
    );
    let table = r#"{"version":1,"assignments":[{"param":"glide_range","channel":1,"cc":20}]}"#;
    assert!(b.load(&with_table(&b, Some(table))));
    let knob = b.value("glide");
    for (cc, range) in [(127, 1.0), (63, 0.0), (64, 1.0), (0, 0.0)] {
        b.process(&[In::Cc(0, 0, 20, cc)]);
        assert_eq!(b.value("glide_range"), range);
        assert_eq!(b.value("glide"), knob);
    }
}

/// An instance rendering offline at `rate` in blocks of at most `block` frames, as a host's
/// bounce has it (CLAP's render extension set offline before processing starts).
fn bouncing(rate: f64, block: u32) -> Instance {
    let p = Instance::with_rate_block(rate, block);
    // SAFETY: the instance is stopped while the host changes its rendering mode.
    unsafe {
        ((*p.plugin).stop_processing.unwrap())(p.plugin);
        let render: &clap_plugin_render = p.extension(CLAP_EXT_RENDER);
        assert!((render.set.unwrap())(p.plugin, CLAP_RENDER_OFFLINE));
        assert!(((*p.plugin).start_processing.unwrap())(p.plugin));
    }
    p
}

/// A session's state with the panel's `params` set and its noise seed fixed.
fn session(params: &[(&'static str, f64)]) -> Vec<u8> {
    let p = Instance::with_block(64);
    let mut state = json_of(&p.save());
    state["fields"]["noise_seed"] = "51826".into();
    assert!(p.load(&bytes_of(&state)));
    let ev: Vec<In> = params.iter().map(|&(k, v)| In::Param(0, k, v)).collect();
    p.audio(1, &ev);
    p.save()
}

/// A phrase at `rate` (sample, event; in order): a four-note chord and a two-note one after it
/// (with POLY off the keys held alone and legato), and the bend moving every 2003 samples; with
/// `knobs`, CUTOFF, EMPHASIS and GLIDE automated every 1013 samples between.
fn bounce_phrase(rate: f64, knobs: bool) -> Vec<(u32, In)> {
    let t = |s: f64| (s * rate) as u32;
    let mut e: Vec<(u32, In)> = Vec::new();
    for (on, off, keys) in [
        (0.02, 0.45, &[48i16, 55, 60, 64][..]),
        (0.5, 0.9, &[50, 57][..]),
    ] {
        for &key in keys {
            e.push((t(on), In::Note(0, CLAP_EVENT_NOTE_ON, -1, 0, key, 0.8)));
            e.push((t(off), In::Note(0, CLAP_EVENT_NOTE_OFF, -1, 0, key, 0.0)));
        }
    }
    let mut k = 0u32;
    while knobs && t(0.1) + 1013 * k < t(0.8) {
        let x = f64::from(k);
        let at = t(0.1) + 1013 * k;
        e.push((
            at,
            In::Param(0, "cutoff", 0.35 + 0.3 * (0.37 * x).sin().abs()),
        ));
        e.push((at, In::Param(0, "emphasis", 0.5 + 0.3 * (0.21 * x).cos())));
        e.push((
            at,
            In::Param(0, "glide", 0.2 + 0.6 * (0.13 * x).sin().abs()),
        ));
        k += 1;
    }
    k = 0;
    while 2003 * k < t(0.95) {
        let bend = (8192.0 + 4000.0 * (0.3 * f64::from(k)).sin()) as u16;
        e.push((
            2003 * k,
            In::Midi(0, [0xe0, (bend & 0x7f) as u8, (bend >> 7) as u8]),
        ));
        k += 1;
    }
    e.sort_by_key(|x| x.0);
    e
}

/// `events` (sample, event; in order) played over `length` frames in blocks of the sizes
/// `sizes` gives (0 among them), both channels interleaved. Between blocks `beside` plays: the
/// project's other instruments on the host's thread.
fn bounce(
    p: &Instance,
    events: &[(u32, In)],
    length: u32,
    sizes: &mut dyn FnMut() -> u32,
    beside: &mut dyn FnMut(),
) -> Vec<f32> {
    let mut audio = Vec::with_capacity(2 * length as usize);
    let (mut event, mut start) = (0, 0);
    while start < length {
        let frames = sizes().min(p.block).min(length - start);
        let mut block = Vec::new();
        // (A block of no frames takes no events: a host gives them with the next.)
        while frames > 0 && event < events.len() && events[event].0 < start + frames {
            let (time, e) = events[event];
            block.push(at(e, time - start));
            event += 1;
        }
        let (_, l, r) = p.stereo(frames, &block);
        audio.extend(l.iter().zip(&r).flat_map(|(a, b)| [*a, *b]));
        start += frames;
        beside();
    }
    audio
}

/// The largest step from one sample to the next of interleaved stereo.
fn largest_step(x: &[f32]) -> f32 {
    x.chunks(2)
        .zip(x.chunks(2).skip(1))
        .fold(0.0f32, |m, (a, b)| {
            m.max((a[0] - b[0]).abs()).max((a[1] - b[1]).abs())
        })
}

/// The bounces of [`an_offline_bounce_is_the_same_in_any_blocks_and_beside_another_instance`]:
/// for each QUALITY and POLY on and off, the bounce in blocks of 512 and the same in blocks of
/// 509, of one frame, of varying sizes (calls of none and one frame among them) and of varying
/// sizes with another instance played between its blocks.
fn bounces(knobs: bool, mut check: impl FnMut(&str, &[f32], &[f32])) {
    const RATE: f64 = 96_000.0;
    const MOST: u32 = 512;
    let phrase = bounce_phrase(RATE, knobs);
    let mut seed = 0x0053_u64;
    let mut rand = move || {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) as f64 / f64::from(1u32 << 31)
    };
    let mut varying = move || {
        let r = rand();
        if r < 0.15 {
            0
        } else if r < 0.3 {
            1
        } else {
            (rand() * f64::from(MOST + 1)) as u32
        }
    };
    let other_state = session(&[
        ("quality", 1.0),
        ("poly", 1.0),
        ("voices", 6.0),
        ("emphasis", 0.9),
        ("feedback", 0.6),
        ("spread", 1.0),
    ]);
    for quality in [0.0, 1.0, 2.0] {
        for poly in [1.0, 0.0] {
            let state = session(&[
                ("quality", quality),
                ("poly", poly),
                ("voices", 4.0),
                ("glide_on", 1.0),
                ("glide", 0.3),
                ("emphasis", 0.6),
            ]);
            // (ULTRA's bounces shorter: they cost the most.)
            let length = (if quality == 2.0 { 0.3 } else { 1.0 } * RATE) as u32;
            let play = |sizes: &mut dyn FnMut() -> u32, beside: &mut dyn FnMut()| {
                let p = bouncing(RATE, MOST);
                assert!(p.load(&state));
                bounce(&p, &phrase, length, sizes, beside)
            };
            let reference = play(&mut || MOST, &mut || {});
            let other = bouncing(RATE, MOST);
            assert!(other.load(&other_state));
            let mut n = 0u32;
            let mut beside = || {
                let mut ev = Vec::new();
                if n.is_multiple_of(40) {
                    for key in [36, 43, 52, 59, 62, 67] {
                        let kind = if n.is_multiple_of(80) {
                            CLAP_EVENT_NOTE_ON
                        } else {
                            CLAP_EVENT_NOTE_OFF
                        };
                        ev.push(In::Note(0, kind, -1, 0, key, 0.7));
                    }
                }
                if n == 300 {
                    assert!(other.load(&other_state));
                }
                other.audio(1 + (n * 37) % MOST, &ev);
                n += 1;
            };
            for (name, x) in [
                ("blocks of 509", play(&mut || 509, &mut || {})),
                ("blocks of 1", play(&mut || 1, &mut || {})),
                ("varying blocks", play(&mut varying, &mut || {})),
                (
                    "varying blocks beside another",
                    play(&mut varying, &mut beside),
                ),
            ] {
                assert_eq!(x.len(), reference.len());
                check(
                    &format!("QUALITY {quality}, POLY {poly}, {name}"),
                    &x,
                    &reference,
                );
            }
        }
    }
}

/// Issue #53 (artifacts bouncing in Bitwig at 96 kHz, clean once the project's other
/// plug-ins were removed): an offline bounce at 96 kHz, POLY and MONO with GLIDE, at each
/// QUALITY, is the same to the bit in blocks of 512, of 509, of one frame, of sizes that vary
/// from call to call with calls of no frames and of one among them, and with another instance
/// (another sound, a state loaded while it plays) played on the same thread between its
/// blocks. Every sample finite. In blocks of one frame POLY's voices let go were cut off at
/// a zero crossing of their release (decisions.md R-SHORT-BLOCKS).
#[test]
fn an_offline_bounce_is_the_same_in_any_blocks_and_beside_another_instance() {
    bounces(false, |name, x, reference| {
        let peak = reference.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak > 0.01 && peak < 4.0, "{name}: peak {peak}");
        let first = x
            .iter()
            .zip(reference)
            .position(|(a, b)| a.to_bits() != b.to_bits());
        let most = x
            .iter()
            .zip(reference)
            .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(
            first.is_none(),
            "{name}: differs from frame {:?}, by at most {most:e}",
            first.map(|i| i / 2)
        );
    });
}

/// The same bounces with CUTOFF, EMPHASIS and GLIDE automated: the knobs' glides are set again
/// every `smooth::STEP` samples from the start of each of the host's blocks (decisions.md
/// R-KNOB-GLIDE), so the samples follow the blocks (an edge a fraction of a sample earlier or
/// later), but no block makes a click: no step from a sample to the next more than 15 % larger
/// than the largest in blocks of 512 (they were 0.92 to 1.07 of it), and the whole as loud
/// within a tenth of a decibel (0.02).
#[test]
fn an_automated_offline_bounce_makes_no_click_in_any_blocks() {
    bounces(true, |name, x, reference| {
        let (step, most) = (largest_step(x), largest_step(reference));
        assert!(
            step <= most * 1.15,
            "{name}: a step of {step} ({most} in 512)"
        );
        let db = 20.0 * (rms(x) / rms(reference)).log10();
        assert!(db.abs() < 0.1, "{name}: the loudness {db:+.3} dB");
    });
}

/// A parameter's change that is the first event of the host's block, at a later sample, is set
/// at its sample (`third_party/nih-plug/PATCHES.md`, change 18): the block is the same to the
/// bit as the host's two blocks either side of the change, and not as the change at the
/// block's start. Before, it was set at the block's first sample.
#[test]
fn a_parameter_change_first_in_a_block_is_set_at_its_sample() {
    let state = session(&[("quality", 1.0), ("emphasis", 0.6)]);
    let play = |blocks: &[(u32, Option<u32>)]| {
        let p = bouncing(RATE, BLOCK);
        assert!(p.load(&state));
        p.audio(64, &[In::Note(0, CLAP_EVENT_NOTE_ON, -1, 0, 48, 0.8)]);
        let mut out = Vec::new();
        for &(frames, change) in blocks {
            let ev: Vec<In> = change
                .map(|t| In::Param(t, "cutoff", 0.9))
                .into_iter()
                .collect();
            out.extend(p.audio(frames, &ev).1);
        }
        out
    };
    let whole = play(&[(BLOCK, Some(150)), (BLOCK, None)]);
    let split = play(&[(150, None), (BLOCK - 150, Some(0)), (BLOCK, None)]);
    let early = play(&[(BLOCK, Some(0)), (BLOCK, None)]);
    assert_eq!(whole, split);
    assert_ne!(whole, early);
}
