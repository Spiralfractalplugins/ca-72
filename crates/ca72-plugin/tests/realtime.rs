//! The engine neither allocates nor frees on the audio thread once its voice is built: notes,
//! pitch bend, the modulation wheel, the MIDI resets, All Sound Off putting the voices to rest
//! (decisions.md R18), every kind of control moving, the side chain driven hard (samples that
//! are not finite among them) and POLY's workers asked for and let go (R21), a block at a
//! time, and asking the plug-in's helper thread for its work (R23), counted by a global
//! allocator on this thread.
//! A failure names where the first came from. (Built with `assert_process_allocs`, nih-plug's
//! own global allocator watches instead.)

#![cfg(not(feature = "assert_process_allocs"))]
#![allow(unsafe_code, clippy::unwrap_used)]

use ca72::tuning::Range;
use ca72::voice::Waveform;
use ca72_plugin::engine::{Controls, Engine, Event};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct Counting;
static ARMED: AtomicBool = AtomicBool::new(false);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static FREES: AtomicUsize = AtomicUsize::new(0);
static FIRST: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
thread_local! {
    static HERE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Counts one call on the armed thread, recording the first's backtrace (disarmed meanwhile,
/// so the recording itself is not counted).
fn count(counter: &AtomicUsize) {
    if HERE.with(std::cell::Cell::get) && ARMED.swap(false, Ordering::SeqCst) {
        counter.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut f) = FIRST.lock()
            && f.is_none()
        {
            *f = Some(std::backtrace::Backtrace::force_capture().to_string());
        }
        ARMED.store(true, Ordering::SeqCst);
    }
}

// SAFETY: forwards to the system allocator; only counts (and records a backtrace) while armed.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(&ALLOCS);
        // SAFETY: the caller's contract is passed through.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        count(&FREES);
        // SAFETY: as above.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static A: Counting = Counting;

/// The tests share the counters: one at a time, each from zero.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Leave this thread disarmed before releasing SERIAL, including if an assertion unwinds.
/// Otherwise libtest's cleanup can be counted after the next test arms the shared counters.
struct Session {
    _serial: std::sync::MutexGuard<'static, ()>,
}

impl Drop for Session {
    fn drop(&mut self) {
        ARMED.store(false, Ordering::SeqCst);
        HERE.with(|h| h.set(false));
    }
}

fn start() -> Session {
    let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    ARMED.store(false, Ordering::SeqCst);
    HERE.with(|h| h.set(false));
    ALLOCS.store(0, Ordering::SeqCst);
    FREES.store(0, Ordering::SeqCst);
    if let Ok(mut f) = FIRST.lock() {
        *f = None;
    }
    Session { _serial: guard }
}

const RATE: f64 = 48_000.0;
const BLOCK: usize = 128;

/// All Sound Off now and then, and the spares it used made again (off the count, as the
/// plug-in's helper thread does) at other times, so that it both finds them made and
/// waits for them.
fn hush(e: &mut Engine, b: usize) {
    if b % 89 == 7 {
        e.event(Event::AllSoundOff);
    }
    if b % 178 == 60 && e.take_mending() {
        ARMED.store(false, Ordering::SeqCst);
        e.spares().mend();
        ARMED.store(true, Ordering::SeqCst);
    }
}

/// POLY's workers started and stopped as the engine asks (decisions.md R21), off the count as
/// the plug-in's helper thread does, a few blocks late, so that the voices also play on
/// this thread while the workers are asked for.
fn serve(e: &mut Engine, b: usize, due: &mut bool) {
    *due |= e.take_serving();
    if *due && b.is_multiple_of(5) {
        ARMED.store(false, Ordering::SeqCst);
        e.crew().serve();
        ARMED.store(true, Ordering::SeqCst);
        *due = false;
    }
}

#[test]
fn the_engine_neither_allocates_nor_frees_while_it_plays() {
    let _serial = start();
    let mut e = Engine::new();
    let mut c = Controls::default();
    e.set(&c);
    e.event(Event::Note { key: 57, on: true });
    e.prepare(RATE, 7);
    // Counted from the voice's first sample (its first step records the solvers'
    // eliminations), on this thread only.
    HERE.with(|h| h.set(true));
    ARMED.store(true, Ordering::SeqCst);
    let tau = 2.0 * std::f64::consts::PI;
    let mut sink = 0.0f32;
    let mut at = 0usize;
    while at < 2 * RATE as usize {
        let b = at / BLOCK;
        // Notes over the 44 keys and beyond them, the wheels from MIDI, the resets.
        if b.is_multiple_of(20) {
            e.event(Event::Note {
                key: (30 + b % 70) as u8,
                on: true,
            });
        }
        if b % 20 == 15 {
            e.event(Event::Note {
                key: (30 + b % 70) as u8,
                on: false,
            });
        }
        e.event(Event::PitchBend((at as f32 * 1e-4).sin()));
        e.event(Event::Modulation(0.5 + 0.5 * (at as f32 * 3e-4).sin()));
        if b.is_multiple_of(97) {
            e.event(Event::ResetControllers);
        }
        if b.is_multiple_of(131) {
            e.event(Event::AllNotesOff);
        }
        hush(&mut e, b);
        // Every kind of control: knobs, switches, selectors, the output and bend range, and
        // QUALITY's ULTRA (the circuit's model with High Fidelity numerics) on and off.
        c.panel.cutoff = 0.5 + 0.3 * (at as f64 * 1e-4).sin();
        c.panel.noise_volume = 0.5 + 0.3 * (at as f64 * 3e-4).sin();
        c.panel.noise_on = b % 50 < 25;
        c.panel.ext_on = true;
        c.panel.ext_volume = 1.0;
        c.panel.glide_on = b % 40 < 20;
        c.panel.glide = 0.4;
        c.panel.glide_long = b % 24 < 12;
        c.panel.keyboard_control_1 = b % 30 < 15;
        c.panel.osc[1].on = true;
        c.panel.osc[1].range = [Range::R16, Range::R8, Range::R4][b % 3];
        c.panel.osc[2].waveform = [Waveform::ReverseSawtooth, Waveform::Square][b % 2];
        c.panel.a440 = b % 200 > 190;
        c.volume = 0.7 + 0.3 * (at as f64 * 2e-4).sin();
        c.bend_range = [2.0, 12.0, 0.0][b % 3];
        c.ultra = b % 200 >= 180;
        e.set(&c);
        for i in 0..BLOCK {
            let t = (at + i) as f64 / RATE;
            let x = if b % 70 == 3 && i == 5 {
                f32::NAN
            } else {
                (0.9 * (tau * 220.0 * t).sin()) as f32
            };
            sink += e.tick(x);
        }
        sink += e.end_block(BLOCK);
        at += BLOCK;
    }
    e.release_all();
    ARMED.store(false, Ordering::SeqCst);
    std::hint::black_box(sink);
    let (allocs, frees) = (ALLOCS.load(Ordering::SeqCst), FREES.load(Ordering::SeqCst));
    let first = FIRST
        .lock()
        .ok()
        .and_then(|f| f.clone())
        .unwrap_or_default();
    assert_eq!(
        (allocs, frees),
        (0, 0),
        "the engine allocated {allocs} and freed {frees} times while playing; the first:\n{first}"
    );
}

/// With POLY on (eight of the ten voices, chords of ten taking held voices, keys beyond the
/// 44), ENTROPY and SPREAD on, POLY switched off and on and VOICES changed while it plays: no
/// allocation, no free.
#[test]
fn poly_neither_allocates_nor_frees_while_it_plays() {
    let _serial = start();
    let mut e = Engine::new();
    let mut c = Controls {
        poly: true,
        voices: 8,
        entropy: 0.7,
        spread: 0.9,
        ..Controls::default()
    };
    e.set(&c);
    e.prepare(RATE, 7);
    HERE.with(|h| h.set(true));
    ARMED.store(true, Ordering::SeqCst);
    let mut sink = 0.0f32;
    let mut at = 0usize;
    while at < 2 * RATE as usize {
        let b = at / BLOCK;
        if b.is_multiple_of(40) {
            for k in [24u8, 45, 52, 57, 61, 64, 69, 73, 76, 110] {
                e.event(Event::Note {
                    key: k + (b / 40 % 3) as u8,
                    on: true,
                });
            }
        }
        if b % 40 == 30 {
            for k in [24u8, 45, 52, 57, 61, 64] {
                e.event(Event::Note {
                    key: k + (b / 40 % 3) as u8,
                    on: false,
                });
            }
        }
        e.event(Event::PitchBend((at as f32 * 1e-4).sin()));
        hush(&mut e, b);
        if b % 150 == 149 {
            c.poly = !c.poly;
        }
        c.voices = [8, 10, 3][b / 100 % 3];
        c.panel.cutoff = 0.5 + 0.3 * (at as f64 * 1e-4).sin();
        e.set(&c);
        for _ in 0..BLOCK {
            let (l, r) = e.tick_stereo(0.0);
            sink += l + r;
        }
        sink += e.end_block(BLOCK);
        at += BLOCK;
    }
    e.release_all();
    ARMED.store(false, Ordering::SeqCst);
    std::hint::black_box(sink);
    let (allocs, frees) = (ALLOCS.load(Ordering::SeqCst), FREES.load(Ordering::SeqCst));
    let first = FIRST
        .lock()
        .ok()
        .and_then(|f| f.clone())
        .unwrap_or_default();
    assert_eq!(
        (allocs, frees),
        (0, 0),
        "POLY allocated {allocs} and freed {frees} times while playing; the first:\n{first}"
    );
}

/// QUALITY at LO (decisions.md R47): the light voices played as the plug-in plays its
/// host's blocks, the one instrument a run at a time and POLY's chords, with the side chain
/// and FEEDBACK, ENTROPY and SPREAD, DOUBLE and UNISON switched on and off, the panel moving,
/// QUALITY and POLY switched while it plays, All Sound Off among it: no allocation, no free on
/// the audio thread.
#[test]
fn quality_lo_neither_allocates_nor_frees_while_it_plays() {
    let _serial = start();
    let mut e = Engine::new();
    let mut c = Controls {
        potato: true,
        voices: 8,
        entropy: 0.7,
        spread: 0.9,
        feedback: 0.4,
        drive: 9.0,
        ..Controls::default()
    };
    e.set(&c);
    e.prepare(RATE, 7);
    HERE.with(|h| h.set(true));
    ARMED.store(true, Ordering::SeqCst);
    let mut sink = 0.0f32;
    let (mut l, mut r, mut ext) = ([0.0f32; BLOCK], [0.0f32; BLOCK], [0.0f32; BLOCK]);
    for b in 0..(2 * RATE as usize / BLOCK) {
        if b.is_multiple_of(40) {
            for k in [24u8, 45, 52, 57, 61, 64, 69, 73, 76, 110] {
                e.event(Event::Note {
                    key: k + (b / 40 % 3) as u8,
                    on: true,
                });
            }
        }
        if b % 40 == 30 {
            for k in [24u8, 45, 52, 57, 61, 64] {
                e.event(Event::Note {
                    key: k + (b / 40 % 3) as u8,
                    on: false,
                });
            }
        }
        hush(&mut e, b);
        c.poly = b / 75 % 2 == 1;
        c.unison = b / 50 % 3 == 2;
        c.double = if b / 60 % 2 == 1 { 0.6 } else { 0.0 };
        if b % 170 == 169 {
            c.potato = !c.potato;
        }
        c.panel.cutoff = 0.5 + 0.3 * (b as f64 * 0.05).sin();
        e.set(&c);
        for (i, x) in ext.iter_mut().enumerate() {
            *x = 0.1 * ((b * BLOCK + i) as f32 * 0.01).sin();
        }
        e.render(&ext, &mut l, &mut r);
        sink += l.iter().chain(&r).sum::<f32>();
        sink += e.end_block(BLOCK);
    }
    e.release_all();
    ARMED.store(false, Ordering::SeqCst);
    std::hint::black_box(sink);
    let (allocs, frees) = (ALLOCS.load(Ordering::SeqCst), FREES.load(Ordering::SeqCst));
    let first = FIRST
        .lock()
        .ok()
        .and_then(|f| f.clone())
        .unwrap_or_default();
    assert_eq!(
        (allocs, frees),
        (0, 0),
        "LO allocated {allocs} and freed {frees} times while playing; the first:\n{first}"
    );
}

/// POLY's voices shared with workers (decisions.md R11), played between the events as the
/// plug-in plays its host's blocks, each block given its deadline, the side chain driven,
/// chords of ten taking held voices, VOICES changed and POLY and QUALITY's LO switched while it plays, its
/// workers let go and asked for again as it is (R21): no allocation, no free on the audio
/// thread.
#[test]
fn poly_with_workers_neither_allocates_nor_frees_while_it_plays() {
    use ca72_plugin::engine::DEADLINE;
    use std::time::{Duration, Instant};
    let _serial = start();
    let mut e = Engine::new();
    let mut c = Controls {
        poly: true,
        voices: 10,
        entropy: 0.7,
        spread: 0.9,
        ..Controls::default()
    };
    c.panel.ext_on = true;
    e.set(&c);
    e.prepare(RATE, 7);
    // (Ordinary threads: this plays as fast as it goes.)
    assert!(e.start_workers(3, None) > 0, "no workers");
    let period = Duration::from_secs_f64(BLOCK as f64 / RATE);
    let (mut l, mut r, mut ext) = ([0.0f32; BLOCK], [0.0f32; BLOCK], [0.0f32; BLOCK]);
    // (Blocks with POLY switched on again that its workers shared, once they came.)
    let (mut due, mut shared) = (false, 0);
    HERE.with(|h| h.set(true));
    ARMED.store(true, Ordering::SeqCst);
    let mut sink = 0.0f32;
    let mut at = 0usize;
    while at < 2 * RATE as usize {
        let b = at / BLOCK;
        e.set_deadline(Some(Instant::now() + period.mul_f64(DEADLINE)));
        if b > 150 && c.poly && e.workers() > 0 {
            shared += 1;
        }
        for (i, x) in ext.iter_mut().enumerate() {
            *x = if b % 70 == 3 && i == 5 {
                f32::INFINITY
            } else {
                0.3 * ((at + i) as f32 * 0.03).sin()
            };
        }
        if b.is_multiple_of(30) {
            for k in [24u8, 45, 52, 57, 61, 64, 69, 73, 76, 110] {
                e.event(Event::Note {
                    key: k + (b / 30 % 3) as u8,
                    on: true,
                });
            }
        }
        if b % 30 == 20 {
            for k in [24u8, 45, 52, 57, 61, 64] {
                e.event(Event::Note {
                    key: k + (b / 30 % 3) as u8,
                    on: false,
                });
            }
        }
        hush(&mut e, b);
        if b % 150 == 149 {
            c.poly = !c.poly;
        }
        c.voices = [10, 6, 3][b / 100 % 3];
        c.feedback = if b % 60 < 30 { 0.0 } else { 0.4 };
        c.panel.cutoff = 0.5 + 0.3 * (at as f64 * 1e-4).sin();
        // (QUALITY's LO, whose voices the workers share too when there is work enough:
        // decisions.md R-LO-WORKERS.)
        c.potato = b / 120 % 2 == 1;
        e.set(&c);
        // (The block in two runs, a pitch bend between them.)
        e.render(&ext[..BLOCK / 2], &mut l[..BLOCK / 2], &mut r[..BLOCK / 2]);
        e.event(Event::PitchBend((at as f32 * 1e-4).sin()));
        e.render(&ext[BLOCK / 2..], &mut l[BLOCK / 2..], &mut r[BLOCK / 2..]);
        sink += l.iter().chain(&r).sum::<f32>();
        sink += e.end_block(BLOCK);
        serve(&mut e, b, &mut due);
        at += BLOCK;
    }
    e.release_all();
    ARMED.store(false, Ordering::SeqCst);
    std::hint::black_box(sink);
    let (allocs, frees) = (ALLOCS.load(Ordering::SeqCst), FREES.load(Ordering::SeqCst));
    let first = FIRST
        .lock()
        .ok()
        .and_then(|f| f.clone())
        .unwrap_or_default();
    assert_eq!(
        (allocs, frees),
        (0, 0),
        "POLY with workers allocated {allocs} and freed {frees} times while playing; the first:\n{first}"
    );
    assert!(shared > 0, "no workers once POLY was switched on again");
}

/// ULTRA's split voices (decisions.md R-THREADS): POLY's voices and the one instrument each
/// lent a crew for a run, POLY switched as it plays (the crews let go with their pool and
/// asked for again, the one instrument then holding its own), DOUBLE and FEEDBACK switched (a
/// voice with FEEDBACK plays whole), each block given its deadline: no allocation, no free on
/// the audio thread, and voices played split.
#[test]
fn ultra_s_split_voices_neither_allocate_nor_free_while_they_play() {
    use ca72_plugin::engine::{DEADLINE, set_split_for_measurement};
    use std::time::{Duration, Instant};
    let _serial = start();
    set_split_for_measurement(Some(2));
    let mut e = Engine::new();
    let mut c = Controls {
        poly: true,
        voices: 2,
        ultra: true,
        drive: 4.0,
        ..Controls::default()
    };
    e.set(&c);
    e.prepare(RATE, 7);
    // (Ordinary threads: this plays as fast as it goes.)
    e.start_workers(2, None);
    let period = Duration::from_secs_f64(BLOCK as f64 / RATE);
    let (mut l, mut r) = ([0.0f32; BLOCK], [0.0f32; BLOCK]);
    let mut due = false;
    HERE.with(|h| h.set(true));
    ARMED.store(true, Ordering::SeqCst);
    let mut sink = 0.0f32;
    let mut at = 0usize;
    // (Short: ULTRA is slow, the more so in a debug build.)
    let seconds = if cfg!(debug_assertions) { 0.4 } else { 1.5 };
    while at < (seconds * RATE) as usize {
        let b = at / BLOCK;
        e.set_deadline(Some(Instant::now() + period.mul_f64(DEADLINE)));
        if b.is_multiple_of(25) {
            for k in [45u8, 52] {
                e.event(Event::Note {
                    key: k + (b / 25 % 3) as u8,
                    on: true,
                });
            }
        }
        if b % 25 == 18 {
            for k in [45u8, 52] {
                e.event(Event::Note {
                    key: k + (b / 25 % 3) as u8,
                    on: false,
                });
            }
        }
        c.poly = (b / 40).is_multiple_of(2);
        c.double = if b / 30 % 2 == 1 { 0.3 } else { 0.0 };
        c.feedback = if b % 70 < 60 { 0.0 } else { 0.3 };
        e.set(&c);
        e.render(&[], &mut l[..BLOCK / 2], &mut r[..BLOCK / 2]);
        e.event(Event::PitchBend((at as f32 * 1e-4).sin()));
        e.render(&[], &mut l[BLOCK / 2..], &mut r[BLOCK / 2..]);
        sink += l.iter().chain(&r).sum::<f32>();
        sink += e.end_block(BLOCK);
        serve(&mut e, b, &mut due);
        at += BLOCK;
    }
    ARMED.store(false, Ordering::SeqCst);
    std::hint::black_box(sink);
    let (runs, _) = e.split_runs();
    e.release_all();
    drop(e);
    set_split_for_measurement(None);
    let (allocs, frees) = (ALLOCS.load(Ordering::SeqCst), FREES.load(Ordering::SeqCst));
    let first = FIRST
        .lock()
        .ok()
        .and_then(|f| f.clone())
        .unwrap_or_default();
    assert_eq!(
        (allocs, frees),
        (0, 0),
        "ULTRA's split voices allocated {allocs} and freed {frees} times while playing; the first:\n{first}"
    );
    assert!(runs > 0, "no voice played split");
}

/// The budget shared with other processes by seat files (decisions.md R-THREADS): POLY's
/// workers made audio threads (so their instance takes a seat and its file), the plug-in's
/// helper serving them and looking at the seats as it does, POLY switched as it plays (the
/// seat given up and taken again): no file operation, no allocation and no free on the audio
/// thread, and a seat file held while POLY was on.
#[test]
fn the_seats_neither_touch_files_nor_allocate_on_the_audio_thread() {
    use ca72_plugin::engine::DEADLINE;
    use ca72_plugin::helper::{Helper, SERVE};
    use std::time::{Duration, Instant};
    let _serial = start();
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: set before this test starts any thread of its own; the seats' folder is read
    // once a process, the first time a seat is taken.
    unsafe { std::env::set_var("CA72_SEATS_DIR", dir.path()) };
    let mut e = Engine::new();
    let mut c = Controls {
        poly: true,
        voices: 6,
        ..Controls::default()
    };
    e.set(&c);
    e.prepare(RATE, 7);
    let period = Duration::from_secs_f64(BLOCK as f64 / RATE);
    e.start_workers(2, Some(period));
    let helper = Helper::start(e.spares(), e.crew(), Default::default()).unwrap();
    let (mut l, mut r) = ([0.0f32; BLOCK], [0.0f32; BLOCK]);
    let mut seated = 0u32;
    let io = ca72_plugin::seats::io_here();
    HERE.with(|h| h.set(true));
    ARMED.store(true, Ordering::SeqCst);
    let mut sink = 0.0f32;
    for b in 0..1500usize {
        e.set_deadline(Some(Instant::now() + period.mul_f64(DEADLINE)));
        if b.is_multiple_of(40) {
            for k in [45u8, 52, 57] {
                e.event(Event::Note { key: k, on: true });
            }
        }
        if b % 40 == 30 {
            for k in [45u8, 52, 57] {
                e.event(Event::Note { key: k, on: false });
            }
        }
        c.poly = b / 300 % 2 == 0;
        e.set(&c);
        e.render(&[], &mut l, &mut r);
        sink += l.iter().chain(&r).sum::<f32>();
        sink += e.end_block(BLOCK);
        if e.take_serving() {
            helper.ask(SERVE);
        }
        if b % 50 == 49 {
            ARMED.store(false, Ordering::SeqCst);
            seated = seated.max(ca72_plugin::pool::seats_now().1);
            ARMED.store(true, Ordering::SeqCst);
        }
    }
    ARMED.store(false, Ordering::SeqCst);
    std::hint::black_box(sink);
    let io_after = ca72_plugin::seats::io_here();
    drop(helper);
    drop(e);
    let (allocs, frees) = (ALLOCS.load(Ordering::SeqCst), FREES.load(Ordering::SeqCst));
    let first = FIRST
        .lock()
        .ok()
        .and_then(|f| f.clone())
        .unwrap_or_default();
    assert_eq!(io_after, io, "the seats' files touched on the audio thread");
    assert_eq!(
        (allocs, frees),
        (0, 0),
        "the seats allocated {allocs} and freed {frees} times while playing; the first:\n{first}"
    );
    assert!(seated >= 1, "no seat file held while POLY was on");
}

/// What the plug-in asks of its helper thread at a block's end (decisions.md R23), the
/// spares mended and POLY's workers served, asked again and again while the helper does it:
/// no allocation, no free on the asking thread.
#[test]
fn asking_the_helper_neither_allocates_nor_frees() {
    use ca72_plugin::helper::{Helper, MEND, SERVE};
    let _serial = start();
    let mut e = Engine::new();
    e.set(&Controls {
        poly: true,
        ..Controls::default()
    });
    e.prepare(RATE, 7);
    let helper = Helper::start(e.spares(), e.crew(), Default::default()).unwrap();
    HERE.with(|h| h.set(true));
    ARMED.store(true, Ordering::SeqCst);
    for k in 0..2_000u32 {
        helper.ask([MEND, SERVE, MEND | SERVE, 0][k as usize % 4]);
        std::hint::spin_loop();
    }
    ARMED.store(false, Ordering::SeqCst);
    drop(helper);
    let (allocs, frees) = (ALLOCS.load(Ordering::SeqCst), FREES.load(Ordering::SeqCst));
    let first = FIRST
        .lock()
        .ok()
        .and_then(|f| f.clone())
        .unwrap_or_default();
    assert_eq!(
        (allocs, frees),
        (0, 0),
        "asking the helper allocated {allocs} and freed {frees} times; the first:\n{first}"
    );
}

/// MIDI Learn on the audio thread (decisions.md R34), through the plug-in's own `process`: learned
/// controllers setting knobs (gliding in the voices), switches, a selector and VOICES through the
/// host, a controller caught while learning, the reserved ones refused, the host's automation
/// gliding on from a learned glide (R-KNOB-GLIDE): no allocation, no free. (The stand-in host's record of the changes is reserved
/// first, as the wrappers reserve theirs; its events are pushed off the count.)
#[test]
fn midi_learn_neither_allocates_nor_frees() {
    use ca72_plugin::Ca72;
    use ca72_plugin::learn::{Cc, index};
    use nih_plug::context::process::TestProcessContext;
    use nih_plug::prelude::*;
    use std::sync::Arc;

    struct Init;
    impl InitContext<Ca72> for Init {
        fn plugin_api(&self) -> PluginApi {
            PluginApi::Clap
        }
        fn execute(&self, _: ()) {}
        fn set_latency_samples(&self, _: u32) {}
        fn set_current_voice_capacity(&self, _: u32) {}
    }

    let _serial = start();
    let mut p = Ca72::default();
    let config = BufferConfig {
        sample_rate: RATE as f32,
        min_buffer_size: None,
        max_buffer_size: BLOCK as u32,
        process_mode: ProcessMode::Realtime,
    };
    assert!(p.initialize(&Ca72::AUDIO_IO_LAYOUTS[0], &config, &mut Init));
    let params = p.params();
    let map = Arc::clone(&p.parameters().midi_map);
    let learned = |id: &str, cc: u8| {
        map.assign(index(id).unwrap(), Cc { channel: 0, cc })
            .unwrap();
    };
    learned("cutoff", 74);
    learned("emphasis", 71);
    learned("osc2_on", 20);
    learned("osc1_range", 21);
    learned("voices", 22);
    let mut c = TestProcessContext::<Ca72>::new(params, RATE as f32, ProcessMode::Realtime);
    c.reported.reserve(1 << 16);
    let cutoff = p.parameters().cutoff.as_ptr();
    let (mut l, mut r) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
    let mut buffer = Buffer::default();
    // SAFETY: both slices are `BLOCK` long and live as long as the buffer.
    unsafe {
        buffer.set_slices(BLOCK, |s| {
            s.clear();
            s.push(&mut l);
            s.push(&mut r);
        });
    }
    let mut aux = AuxiliaryBuffers {
        inputs: &mut [],
        outputs: &mut [],
    };
    let cc = |timing: u32, cc: u8, value: f32| NoteEvent::MidiCC {
        timing,
        channel: 0,
        cc,
        value,
    };
    HERE.with(|h| h.set(true));
    for b in 0..400usize {
        // (The host's queue filled off the count.)
        ARMED.store(false, Ordering::SeqCst);
        if b.is_multiple_of(50) {
            c.push_event(NoteEvent::NoteOn {
                timing: 0,
                voice_id: None,
                channel: 0,
                note: 45 + (b / 50 % 12) as u8,
                velocity: 1.0,
            });
        }
        for k in 0..4u32 {
            let v = ((b as f32 * 4.0 + k as f32) * 0.07).sin() * 0.5 + 0.5;
            c.push_event(cc(k * 30, 74, v));
            c.push_event(cc(k * 30 + 3, 71, 1.0 - v));
        }
        c.push_event(cc(5, 20, if b % 6 < 3 { 0.0 } else { 1.0 }));
        c.push_event(cc(6, 21, (b % 7) as f32 / 6.0));
        c.push_event(cc(7, 22, (b % 9) as f32 / 8.0));
        c.push_event(cc(9, 1, 0.5));
        c.push_event(cc(9, 100, 0.5));
        if b % 40 == 10 {
            map.arm(index("glide").unwrap());
            c.push_event(cc(11, 30 + (b / 40) as u8, 0.5));
        }
        if b % 40 == 11 {
            map.poll();
        }
        if b % 30 == 29 {
            c.automate(cutoff, 0.25);
        }
        ARMED.store(true, Ordering::SeqCst);
        p.process(&mut buffer, &mut aux, &mut c);
    }
    ARMED.store(false, Ordering::SeqCst);
    let (allocs, frees) = (ALLOCS.load(Ordering::SeqCst), FREES.load(Ordering::SeqCst));
    let first = FIRST
        .lock()
        .ok()
        .and_then(|f| f.clone())
        .unwrap_or_default();
    assert_eq!(
        (allocs, frees),
        (0, 0),
        "MIDI Learn allocated {allocs} and freed {frees} times while playing; the first:\n{first}"
    );
    assert!(c.reported.len() > 1000, "{} changes told", c.reported.len());
}

/// The arpeggiator (decisions.md R-ARP) neither allocates nor frees while it plays: the keys
/// held and let go, HOLD (with ON off, its drone too), the sustain pedal, its every setting
/// automated (ORDER and OCTAVES 5 among them), the transport playing, jumping back (a loop) and
/// stopping, ON switched off and on with keys held, All Notes Off, a block at a time, in the
/// plug-in's own process.
#[test]
fn the_arpeggiator_neither_allocates_nor_frees() {
    use ca72_plugin::Ca72;
    use nih_plug::context::process::TestProcessContext;
    use nih_plug::prelude::*;

    struct Init;
    impl InitContext<Ca72> for Init {
        fn plugin_api(&self) -> PluginApi {
            PluginApi::Clap
        }
        fn execute(&self, _: ()) {}
        fn set_latency_samples(&self, _: u32) {}
        fn set_current_voice_capacity(&self, _: u32) {}
    }

    let _serial = start();
    let mut p = Ca72::default();
    let config = BufferConfig {
        sample_rate: RATE as f32,
        min_buffer_size: None,
        max_buffer_size: BLOCK as u32,
        process_mode: ProcessMode::Realtime,
    };
    assert!(p.initialize(&Ca72::AUDIO_IO_LAYOUTS[0], &config, &mut Init));
    let params = p.params();
    let a = p.parameters().clone();
    let mut c = TestProcessContext::<Ca72>::new(params, RATE as f32, ProcessMode::Realtime);
    assert!(c.automate(a.arp_on.as_ptr(), 1.0));
    let (mut l, mut r) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
    let mut buffer = Buffer::default();
    // SAFETY: both slices are `BLOCK` long and live as long as the buffer.
    unsafe {
        buffer.set_slices(BLOCK, |s| {
            s.clear();
            s.push(&mut l);
            s.push(&mut r);
        });
    }
    let mut aux = AuxiliaryBuffers {
        inputs: &mut [],
        outputs: &mut [],
    };
    let key = |timing: u32, note: u8, on: bool| {
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
    let cc = |timing: u32, cc: u8, value: f32| NoteEvent::MidiCC {
        timing,
        channel: 0,
        cc,
        value,
    };
    HERE.with(|h| h.set(true));
    let mut beats = 0.0;
    for b in 0..1200usize {
        // (The host's queue filled and its automation set off the count.)
        ARMED.store(false, Ordering::SeqCst);
        if b % 300 == 250 {
            beats = 4.0;
        }
        let playing = b % 400 < 330;
        c.set_transport(
            playing,
            Some(100.0 + (b % 3) as f64 * 20.0),
            Some(beats),
            Some((4, 4)),
        );
        beats += BLOCK as f64 * 2.0 / RATE;
        if b % 40 == 0 {
            for (i, k) in [48u8, 55, 60, 64, 120].iter().enumerate() {
                c.push_event(key(i as u32 * 7, *k + (b / 40 % 5) as u8, true));
            }
        }
        if b % 40 == 30 {
            for k in [48u8, 55, 60, 64, 120] {
                c.push_event(key(64, k + (b / 40 % 5) as u8, false));
            }
        }
        if b % 50 == 5 {
            c.push_event(cc(3, 64, if b % 100 < 50 { 1.0 } else { 0.0 }));
        }
        if b % 97 == 40 {
            c.push_event(cc(9, 123, 0.0));
        }
        let k = (b / 23) as f32;
        c.automate(a.arp_latch.as_ptr(), if b % 160 < 80 { 0.0 } else { 1.0 });
        c.automate(a.arp_direction.as_ptr(), (k % 5.0) / 4.0);
        c.automate(a.arp_rate.as_ptr(), (k % 13.0) / 12.0);
        c.automate(a.arp_octaves.as_ptr(), (k % 5.0) / 4.0);
        c.automate(a.arp_gate.as_ptr(), (k * 0.37) % 1.0);
        c.automate(a.arp_swing.as_ptr(), (k * 0.61) % 1.0);
        c.automate(a.arp_on.as_ptr(), if b % 210 < 150 { 1.0 } else { 0.0 });
        ARMED.store(true, Ordering::SeqCst);
        p.process(&mut buffer, &mut aux, &mut c);
    }
    ARMED.store(false, Ordering::SeqCst);
    let (allocs, frees) = (ALLOCS.load(Ordering::SeqCst), FREES.load(Ordering::SeqCst));
    let first = FIRST
        .lock()
        .ok()
        .and_then(|f| f.clone())
        .unwrap_or_default();
    assert_eq!(
        (allocs, frees),
        (0, 0),
        "the arpeggiator allocated {allocs} and freed {frees} times while playing; the first:\n{first}"
    );
}

/// Each note's own expression (decisions.md R-MPE) neither allocates nor frees, in the plug-in's
/// own process: the MPE Configuration Message and its zone gone and back, notes on member
/// channels with their bends, pressures and timbres, a member's bend range (RPN 0), polyphonic
/// aftertouch and a host's note expressions (pressure, tuning, brightness), ordinary channel
/// pressure and timbre, the MOD wheel on polyphonic aftertouch, the reaches automated, POLY
/// switched and voices stolen, QUALITY at LO and HI, Reset All Controllers and All Notes Off,
/// TIMBRE REST switched through its three (R-MPE) and the INPUT screen's counts read,
/// a block at a time.
#[test]
fn mpe_expression_neither_allocates_nor_frees() {
    use ca72_plugin::Ca72;
    use ca72_plugin::expression::TimbreRest;
    use ca72_plugin::mod_wheel::Source;
    use nih_plug::context::process::TestProcessContext;
    use nih_plug::prelude::*;

    struct Init;
    impl InitContext<Ca72> for Init {
        fn plugin_api(&self) -> PluginApi {
            PluginApi::Clap
        }
        fn execute(&self, _: ()) {}
        fn set_latency_samples(&self, _: u32) {}
        fn set_current_voice_capacity(&self, _: u32) {}
    }

    let _serial = start();
    let mut p = Ca72::default();
    let config = BufferConfig {
        sample_rate: RATE as f32,
        min_buffer_size: None,
        max_buffer_size: BLOCK as u32,
        process_mode: ProcessMode::Realtime,
    };
    assert!(p.initialize(&Ca72::AUDIO_IO_LAYOUTS[0], &config, &mut Init));
    p.parameters()
        .midi_map
        .set_wheel_source(Source::PolyPressure);
    let params = p.params();
    let q = p.parameters().clone();
    let mut c = TestProcessContext::<Ca72>::new(params, RATE as f32, ProcessMode::Realtime);
    c.reported.reserve(1 << 16);
    let (mut l, mut r) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
    let mut buffer = Buffer::default();
    // SAFETY: both slices are `BLOCK` long and live as long as the buffer.
    unsafe {
        buffer.set_slices(BLOCK, |s| {
            s.clear();
            s.push(&mut l);
            s.push(&mut r);
        });
    }
    let mut aux = AuxiliaryBuffers {
        inputs: &mut [],
        outputs: &mut [],
    };
    let cc = |timing: u32, channel: u8, cc: u8, value: f32| NoteEvent::MidiCC {
        timing,
        channel,
        cc,
        value,
    };
    let key = |timing: u32, channel: u8, note: u8, on: bool| {
        if on {
            NoteEvent::NoteOn {
                timing,
                voice_id: None,
                channel,
                note,
                velocity: 1.0,
            }
        } else {
            NoteEvent::NoteOff {
                timing,
                voice_id: None,
                channel,
                note,
                velocity: 0.0,
            }
        }
    };
    HERE.with(|h| h.set(true));
    let mut sink = 0.0f32;
    let mut seen = 0u32;
    for b in 0..600usize {
        // (The host's queue filled off the count.)
        ARMED.store(false, Ordering::SeqCst);
        let v = |k: usize| ((b * 3 + k) as f32 * 0.11).sin() * 0.5 + 0.5;
        if b % 150 == 0 {
            // The zone: fifteen members, then (every other time) gone for a while.
            let n = if b % 300 == 0 { 15.0 } else { 0.0 };
            c.push_event(cc(0, 0, 101, 0.0));
            c.push_event(cc(0, 0, 100, 6.0 / 127.0));
            c.push_event(cc(0, 0, 6, n / 127.0));
        }
        if b % 50 == 7 {
            c.push_event(cc(1, 3, 101, 0.0));
            c.push_event(cc(1, 3, 100, 0.0));
            c.push_event(cc(1, 3, 6, [48.0, 12.0, 2.0][b / 50 % 3] / 127.0));
        }
        // A note on each of six member channels in turn, its controllers first.
        let ch = 1 + (b / 6 % 6) as u8;
        let note = 40 + (b * 7 % 48) as u8;
        if b % 6 == 0 {
            c.push_event(NoteEvent::MidiPitchBend {
                timing: 2,
                channel: ch,
                value: v(0),
            });
            c.push_event(cc(2, ch, 74, v(1)));
            c.push_event(NoteEvent::MidiChannelPressure {
                timing: 2,
                channel: ch,
                pressure: v(2),
            });
            c.push_event(key(3, ch, note, true));
        }
        for k in 0..3u32 {
            let ch = 1 + ((b + k as usize) % 6) as u8;
            c.push_event(NoteEvent::MidiPitchBend {
                timing: 20 + k * 40,
                channel: ch,
                value: v(k as usize),
            });
            c.push_event(NoteEvent::MidiChannelPressure {
                timing: 21 + k * 40,
                channel: ch,
                pressure: v(k as usize + 1),
            });
            c.push_event(cc(22 + k * 40, ch, 74, v(k as usize + 2)));
        }
        // A host's note expressions and polyphonic aftertouch on the note just played.
        c.push_event(NoteEvent::PolyPressure {
            timing: 150,
            voice_id: None,
            channel: ch,
            note,
            pressure: v(4),
        });
        c.push_event(NoteEvent::PolyTuning {
            timing: 151,
            voice_id: None,
            channel: ch,
            note,
            tuning: v(5) * 4.0 - 2.0,
        });
        c.push_event(NoteEvent::PolyBrightness {
            timing: 152,
            voice_id: None,
            channel: ch,
            note,
            brightness: v(6),
        });
        // Ordinary channel pressure and timbre on the master channel.
        c.push_event(NoteEvent::MidiChannelPressure {
            timing: 160,
            channel: 0,
            pressure: v(7),
        });
        c.push_event(cc(161, 0, 74, v(8)));
        if b % 6 == 5 {
            c.push_event(key(200, ch, note, false));
        }
        if b % 97 == 0 {
            c.push_event(cc(210, 9, 121, 0.0));
        }
        if b % 131 == 0 {
            c.push_event(cc(211, 0, 123, 0.0));
        }
        if b % 20 == 0 {
            c.automate(q.pressure_mod.as_ptr(), v(9));
            c.automate(q.pressure_filter.as_ptr(), v(10));
            c.automate(q.timbre_mod.as_ptr(), v(11));
            c.automate(q.timbre_filter.as_ptr(), v(12));
        }
        if b % 100 == 50 {
            c.automate(q.poly.as_ptr(), if b % 200 == 50 { 1.0 } else { 0.0 });
            c.automate(q.voices.as_ptr(), 0.0);
        }
        if b % 120 == 60 {
            c.automate(q.quality.as_ptr(), if b % 240 == 60 { 0.5 } else { 0.0 });
        }
        if b % 40 == 13 {
            q.set_timbre_rest(TimbreRest::ALL[b / 40 % 3]);
        }
        ARMED.store(true, Ordering::SeqCst);
        p.process(&mut buffer, &mut aux, &mut c);
        let (per_note, channel) = q.heard.counts();
        seen = seen.max(per_note.min(channel));
        sink += buffer.as_slice()[0].iter().map(|x| x.abs()).sum::<f32>();
    }
    assert!(seen > 0, "both counted");
    ARMED.store(false, Ordering::SeqCst);
    assert!(sink > 1.0, "it played ({sink})");
    let (allocs, frees) = (ALLOCS.load(Ordering::SeqCst), FREES.load(Ordering::SeqCst));
    let first = FIRST
        .lock()
        .ok()
        .and_then(|f| f.clone())
        .unwrap_or_default();
    assert_eq!(
        (allocs, frees),
        (0, 0),
        "MPE allocated {allocs} and freed {frees} times while playing; the first:\n{first}"
    );
}

/// The oscillators timed (decisions.md R-OSC-TIMED) neither allocate nor free while they play:
/// each TIMED switched, each RANGE moved to and from LO, each DIVISION and QUALITY automated
/// (LO, HI, ULTRA), all three heard and oscillator 3 on the modulation, the transport playing, jumping back (a loop),
/// changing tempo and stopping, keys pressed and let go, in the plug-in's own process.
#[test]
fn the_oscillators_timed_neither_allocate_nor_free() {
    use ca72_plugin::Ca72;
    use nih_plug::context::process::TestProcessContext;
    use nih_plug::prelude::*;

    struct Init;
    impl InitContext<Ca72> for Init {
        fn plugin_api(&self) -> PluginApi {
            PluginApi::Clap
        }
        fn execute(&self, _: ()) {}
        fn set_latency_samples(&self, _: u32) {}
        fn set_current_voice_capacity(&self, _: u32) {}
    }

    let _serial = start();
    let mut p = Ca72::default();
    let config = BufferConfig {
        sample_rate: RATE as f32,
        min_buffer_size: None,
        max_buffer_size: BLOCK as u32,
        process_mode: ProcessMode::Realtime,
    };
    assert!(p.initialize(&Ca72::AUDIO_IO_LAYOUTS[0], &config, &mut Init));
    let params = p.params();
    let a = p.parameters().clone();
    let mut c = TestProcessContext::<Ca72>::new(params, RATE as f32, ProcessMode::Realtime);
    for (q, v) in [
        (a.osc2_timed.as_ptr(), 1.0),
        (a.osc2_on.as_ptr(), 1.0),
        (a.osc3_timed.as_ptr(), 1.0),
        (a.osc3_range.as_ptr(), 0.0),
        (a.osc3_on.as_ptr(), 1.0),
        (a.osc_mod.as_ptr(), 1.0),
        (a.filter_mod.as_ptr(), 1.0),
        (a.mod_wheel.as_ptr(), 0.5),
    ] {
        assert!(c.automate(q, v));
    }
    let (mut l, mut r) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
    let mut buffer = Buffer::default();
    // SAFETY: both slices are `BLOCK` long and live as long as the buffer.
    unsafe {
        buffer.set_slices(BLOCK, |s| {
            s.clear();
            s.push(&mut l);
            s.push(&mut r);
        });
    }
    let mut aux = AuxiliaryBuffers {
        inputs: &mut [],
        outputs: &mut [],
    };
    let key = |timing: u32, note: u8, on: bool| {
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
    HERE.with(|h| h.set(true));
    let mut beats = 0.0;
    for b in 0..900usize {
        // (The host's queue filled and its automation set off the count.)
        ARMED.store(false, Ordering::SeqCst);
        if b % 300 == 250 {
            beats = 4.0;
        }
        let playing = b % 400 < 330;
        let tempo = 90.0 + (b % 5) as f64 * 17.3;
        c.set_transport(playing, Some(tempo), Some(beats), Some((4, 4)));
        beats += BLOCK as f64 * tempo / 60.0 / RATE;
        if b % 60 == 0 {
            c.push_event(key(3, 48 + (b / 60 % 7) as u8, true));
        }
        if b % 60 == 45 {
            c.push_event(key(9, 48 + (b / 60 % 7) as u8, false));
        }
        let k = (b / 19) as f32;
        c.automate(a.osc3_division.as_ptr(), (k % 18.0) / 17.0);
        c.automate(a.osc2_division.as_ptr(), ((k + 11.0) % 18.0) / 17.0);
        c.automate(a.osc3_timed.as_ptr(), if b % 170 < 150 { 1.0 } else { 0.0 });
        c.automate(a.osc3_range.as_ptr(), if b % 130 < 100 { 0.0 } else { 0.6 });
        c.automate(a.osc1_range.as_ptr(), if b % 110 < 70 { 0.0 } else { 0.6 });
        c.automate(a.osc2_range.as_ptr(), if b % 150 < 90 { 0.0 } else { 0.2 });
        c.automate(a.quality.as_ptr(), (b / 300) as f32 / 2.0);
        c.automate(a.glide_range.as_ptr(), if b % 80 < 40 { 1.0 } else { 0.0 });
        ARMED.store(true, Ordering::SeqCst);
        p.process(&mut buffer, &mut aux, &mut c);
    }
    ARMED.store(false, Ordering::SeqCst);
    let (allocs, frees) = (ALLOCS.load(Ordering::SeqCst), FREES.load(Ordering::SeqCst));
    let first = FIRST
        .lock()
        .ok()
        .and_then(|f| f.clone())
        .unwrap_or_default();
    assert_eq!(
        (allocs, frees),
        (0, 0),
        "the oscillators timed allocated {allocs} and freed {frees} times while playing; the first:\n{first}"
    );
}

/// The LFOs (decisions.md R-LFO) modulating eight knobs each, MONO off on POLY's ten voices,
/// their settings automated (MODE, MONO, RATE, timed, POLARITY, REVERSE, ON), QUALITY LO, HI and
/// ULTRA, and the patch changed between blocks as the editor changes it: no allocation, no free.
#[test]
fn the_lfos_neither_allocate_nor_free() {
    lfos_play_without_allocating(false);
}

/// The LFOs moving each other's knobs and their own (R-LFO's addendum), in loops, per voice
/// (MONO off on ten voices), on their ticks, with the macros on their knobs too, their LEVELs
/// and WAVEFORMs' CONTINUOUS automated besides: no allocation, no free.
#[test]
fn the_lfos_moving_each_other_neither_allocate_nor_free() {
    lfos_play_without_allocating(true);
}

/// The LFOs' run for [`the_lfos_neither_allocate_nor_free`]; `cross`, half of each source's
/// knobs the LFOs' own (an LFO's own RATE, the next one's RATE and PHASE, another's LEVEL; the
/// macros' FADE, shape and LEVEL).
fn lfos_play_without_allocating(cross: bool) {
    use ca72_plugin::Ca72;
    use ca72_plugin::lfo::{Source, Target};
    use nih_plug::context::process::TestProcessContext;
    use nih_plug::prelude::*;

    struct Init;
    impl InitContext<Ca72> for Init {
        fn plugin_api(&self) -> PluginApi {
            PluginApi::Clap
        }
        fn execute(&self, _: ()) {}
        fn set_latency_samples(&self, _: u32) {}
        fn set_current_voice_capacity(&self, _: u32) {}
    }

    let _serial = start();
    let mut p = Ca72::default();
    let config = BufferConfig {
        sample_rate: RATE as f32,
        min_buffer_size: None,
        max_buffer_size: BLOCK as u32,
        process_mode: ProcessMode::Realtime,
    };
    assert!(p.initialize(&Ca72::AUDIO_IO_LAYOUTS[0], &config, &mut Init));
    let params = p.params();
    let a = p.parameters().clone();
    let mut c = TestProcessContext::<Ca72>::new(params, RATE as f32, ProcessMode::Realtime);
    for (q, v) in [
        (a.poly.as_ptr(), 1.0),
        (a.voices.as_ptr(), 1.0),
        (a.osc2_on.as_ptr(), 1.0),
    ] {
        assert!(c.automate(q, v));
    }
    let targets = |n: usize| {
        let on = [
            Target::Cutoff,
            Target::Emphasis,
            Target::Osc2Frequency,
            Target::Osc1Volume,
            Target::Volume,
            Target::Feedback,
            Target::Tune,
            Target::Spread,
        ];
        if !cross {
            return on;
        }
        // (R-LFO's addendum: an LFO's own RATE, the next one's RATE and PHASE, another's LEVEL.)
        let m = n % 4;
        [
            on[0],
            on[1],
            on[4],
            on[5],
            Target::LfoRate(m),
            Target::LfoRate((m + 1) % 4),
            Target::LfoPhase((m + 1) % 4),
            Target::LfoLevel((m + 2) % 4),
        ]
    };
    for n in 0..4 {
        for (k, t) in targets(n).iter().enumerate() {
            a.lfo_patch
                .assign(Source::Lfo(n), *t, 0.05 * (k as f32 - 3.5))
                .expect("room");
        }
        assert!(c.automate(a.lfo(n).mono.as_ptr(), 0.0));
    }
    // (The six macros on the same knobs, one way each: decisions.md R-MACROS; moving the
    // LFOs, their FADE, shape and LEVEL too.)
    for n in 0..ca72_plugin::lfo::MACROS {
        let mut on = targets(n);
        if cross {
            (on[4], on[5], on[6]) = (
                Target::LfoFade(n % 4),
                Target::LfoShape((n + 1) % 4),
                Target::LfoLevel((n + 3) % 4),
            );
        }
        for (k, t) in on.iter().enumerate() {
            a.lfo_patch
                .assign(Source::Macro(n), *t, 0.04 * (k as f32 - 3.5))
                .expect("room");
        }
    }
    let (mut l, mut r) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
    let mut buffer = Buffer::default();
    // SAFETY: both slices are `BLOCK` long and live as long as the buffer.
    unsafe {
        buffer.set_slices(BLOCK, |s| {
            s.clear();
            s.push(&mut l);
            s.push(&mut r);
        });
    }
    let mut aux = AuxiliaryBuffers {
        inputs: &mut [],
        outputs: &mut [],
    };
    let key = |timing: u32, note: u8, on: bool| {
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
    HERE.with(|h| h.set(true));
    let mut beats = 0.0;
    for b in 0..900usize {
        ARMED.store(false, Ordering::SeqCst);
        c.set_transport(b % 400 < 330, Some(120.0), Some(beats), Some((4, 4)));
        beats += BLOCK as f64 * 2.0 / RATE;
        if b % 20 == 0 {
            c.push_event(key(3, 48 + (b / 20 % 12) as u8, true));
        }
        if b % 20 == 15 && b > 200 {
            c.push_event(key(9, 48 + (b / 20 % 12) as u8, false));
        }
        let n = b % 4;
        let q = a.lfo(n);
        c.automate(q.mode.as_ptr(), (b / 37 % 3) as f32 / 2.0);
        c.automate(q.rate.as_ptr(), (b % 50) as f32 / 49.0);
        c.automate(q.timed.as_ptr(), if b % 90 < 45 { 1.0 } else { 0.0 });
        c.automate(q.fade.as_ptr(), (b % 7) as f32 / 6.0);
        c.automate(q.mono.as_ptr(), if b % 130 < 100 { 0.0 } else { 1.0 });
        // (POLARITY, REVERSE and ON: R-LFO's corrections; ON's fade through.)
        let keys = a.lfo_keys(n);
        c.automate(keys.polarity.as_ptr(), if b % 70 < 35 { 1.0 } else { 0.0 });
        c.automate(keys.reverse.as_ptr(), if b % 110 < 55 { 1.0 } else { 0.0 });
        c.automate(keys.on.as_ptr(), if b % 60 < 40 { 1.0 } else { 0.0 });
        if cross {
            // (LEVEL and CONTINUOUS's shape, the macros' and the LFOs' targets: R-LFO's
            // addendum.)
            c.automate(a.lfo_level(n).level.as_ptr(), (b % 13) as f32 / 12.0);
            c.automate(q.shape_mode.as_ptr(), if b % 80 < 50 { 1.0 } else { 0.0 });
            c.automate(q.shape.as_ptr(), (b % 9) as f32 / 8.0);
        }
        c.automate(a.quality.as_ptr(), (b / 300) as f32 / 2.0);
        // (A macro turned, gliding there; one renamed, between blocks, by the editor.)
        c.automate(a.macro_param(b % 6).as_ptr(), (b % 33) as f32 / 32.0);
        if b % 50 == 10 {
            a.lfo_patch.set_name(b % 6, "GROWL");
        }
        ARMED.store(true, Ordering::SeqCst);
        // (The patch as the editor changes it, between blocks; its atomics alone.)
        if b % 50 == 25 {
            a.lfo_patch.set_depth(Source::Lfo(n), Target::Cutoff, -0.3);
            if cross {
                a.lfo_patch
                    .set_depth(Source::Lfo(n), Target::LfoRate(n), 0.9);
            }
        }
        p.process(&mut buffer, &mut aux, &mut c);
    }
    ARMED.store(false, Ordering::SeqCst);
    let (allocs, frees) = (ALLOCS.load(Ordering::SeqCst), FREES.load(Ordering::SeqCst));
    let first = FIRST
        .lock()
        .ok()
        .and_then(|f| f.clone())
        .unwrap_or_default();
    assert_eq!(
        (allocs, frees),
        (0, 0),
        "the LFOs allocated {allocs} and freed {frees} times while playing; the first:\n{first}"
    );
}

/// The standalone's TEMPO moved by the sources (decisions.md R-LFO): an LFO free, a timed one
/// (a loop through its own clock) and a macro on it, the arpeggiator playing at the moving tempo,
/// oscillator 3 timed and the timed LFO following its beats, the depths changed between blocks
/// as the editor changes them: no allocation, no free.
#[test]
fn a_moving_tempo_neither_allocates_nor_frees() {
    use ca72_plugin::Ca72;
    use ca72_plugin::lfo::{Source, Target};
    use nih_plug::context::process::TestProcessContext;
    use nih_plug::prelude::*;

    struct Init;
    impl InitContext<Ca72> for Init {
        fn plugin_api(&self) -> PluginApi {
            PluginApi::Standalone
        }
        fn execute(&self, _: ()) {}
        fn set_latency_samples(&self, _: u32) {}
        fn set_current_voice_capacity(&self, _: u32) {}
    }

    let _serial = start();
    let mut p = Ca72::default();
    let config = BufferConfig {
        sample_rate: RATE as f32,
        min_buffer_size: None,
        max_buffer_size: BLOCK as u32,
        process_mode: ProcessMode::Realtime,
    };
    assert!(p.initialize(&Ca72::AUDIO_IO_LAYOUTS[0], &config, &mut Init));
    let params = p.params();
    let a = p.parameters().clone();
    // (The standalone's own: its transport stopped, its clock running on at TEMPO.)
    let mut c = TestProcessContext::<Ca72>::new(params, RATE as f32, ProcessMode::Realtime);
    for (q, v) in [
        (a.poly.as_ptr(), 1.0),
        (a.voices.as_ptr(), 1.0),
        (a.arp_on.as_ptr(), 1.0),
        (a.osc3_timed.as_ptr(), 1.0),
        (a.lfo(1).timed.as_ptr(), 1.0),
    ] {
        assert!(c.automate(q, v));
    }
    for (s, d) in [
        (Source::Lfo(0), 0.3),
        (Source::Lfo(1), -0.4),
        (Source::Macro(0), 0.2),
    ] {
        a.lfo_patch.assign(s, Target::Tempo, d).expect("room");
    }
    a.lfo_patch
        .assign(Source::Lfo(1), Target::Cutoff, 0.2)
        .expect("room");
    let (mut l, mut r) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
    let mut buffer = Buffer::default();
    // SAFETY: both slices are `BLOCK` long and live as long as the buffer.
    unsafe {
        buffer.set_slices(BLOCK, |s| {
            s.clear();
            s.push(&mut l);
            s.push(&mut r);
        });
    }
    let mut aux = AuxiliaryBuffers {
        inputs: &mut [],
        outputs: &mut [],
    };
    let key = |timing: u32, note: u8, on: bool| {
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
    HERE.with(|h| h.set(true));
    for b in 0..900usize {
        ARMED.store(false, Ordering::SeqCst);
        if b % 40 == 0 {
            c.push_event(key(3, 48 + (b / 40 % 12) as u8, true));
            c.push_event(key(5, 55 + (b / 40 % 12) as u8, true));
        }
        if b % 40 == 30 {
            c.push_event(key(9, 48 + (b / 40 % 12) as u8, false));
            c.push_event(key(9, 55 + (b / 40 % 12) as u8, false));
        }
        c.automate(a.lfo(0).rate.as_ptr(), (b % 50) as f32 / 49.0);
        c.automate(a.macro_param(0).as_ptr(), (b % 33) as f32 / 32.0);
        c.automate(a.quality.as_ptr(), (b / 300) as f32 / 2.0);
        ARMED.store(true, Ordering::SeqCst);
        if b % 50 == 25 {
            a.lfo_patch.set_depth(
                Source::Lfo(0),
                Target::Tempo,
                if b % 100 == 25 { 0.8 } else { -0.5 },
            );
        }
        p.process(&mut buffer, &mut aux, &mut c);
    }
    ARMED.store(false, Ordering::SeqCst);
    let (allocs, frees) = (ALLOCS.load(Ordering::SeqCst), FREES.load(Ordering::SeqCst));
    let first = FIRST
        .lock()
        .ok()
        .and_then(|f| f.clone())
        .unwrap_or_default();
    assert_eq!(
        (allocs, frees),
        (0, 0),
        "the moving tempo allocated {allocs} and freed {frees} times; the first:\n{first}"
    );
}
