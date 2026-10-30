//! The prototype of ULTRA's split voices (decisions.md R-THREADS): ULTRA's voices played through the
//! plug-in's own pool (`pool::Pool`, the host's thread and its workers sharing the voices),
//! `CA72_SPLIT` of them each on six threads of its own (`ca72::threaded::Threaded`, with
//! DRIVE), the rest whole. Every thread made an audio thread comes out of one budget,
//! `CA72_BUDGET`: each split voice takes its five helpers from it, the pool what is left.
//!
//! The load is `poly_bench`'s at ULTRA: `CA72_VOICES` voices (2) playing a chord of as many
//! notes changing every half second (each held seven eighths of it), oscillators 2 and 3 on,
//! the filter modulated, GLIDE on, the cutoff swept, DRIVE at `CA72_DRIVE` dB (6); no
//! ENTROPY or SPREAD (the voices' characters are the engine's), no EXTERNAL INPUT. Paced on a
//! promoted host thread at `CA72_BLOCK` frames (256), 2 s then `CA72_SECONDS` (10) measured,
//! each block given the plug-in's deadline. Prints one `split:` line.
//!
//! `CA72_HELPER_WG=1` (macOS) puts the split voices' helpers in a work interval of their own,
//! as the pool's workers are in theirs (decisions.md R11): a driver thread, a member too,
//! opens it at each block's start with the period as its deadline and closes it when the
//! block's runs are done, as the pool's first worker does for its own (the host's thread
//! cannot join one, R11). Off by default: the split numbers in R-THREADS are without it.
//!
//! `the_split_voice_with_drive_is_the_voice` checks the prototype's samples: the same to the
//! bit as the whole voice's with DRIVE, as the engine plays it.
//!
//! By hand: `cargo test --release -p ca72-plugin --test ultra_split -- --ignored --nocapture`.

#![allow(clippy::unwrap_used)]

mod common;

use ca72::threaded::Threaded;
use ca72::voice::{Jacks, Panel, Quality, Voice};
use ca72_plugin::engine::{CHUNK, DEADLINE, MIX_LEN, Mix, set_budget_for_measurement};
use ca72_plugin::pool::{EVERY, Play, Pool, Shared};
use common::env;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const RATE: f64 = 48_000.0;

/// A voice at ULTRA as the pool plays it: whole on the thread that takes it, or split.
enum Ultra {
    Whole(Box<Voice>),
    Split(Box<Threaded>),
}

struct Played {
    v: Ultra,
    out: [f64; CHUNK],
}

impl Played {
    fn panel(&mut self, p: Panel) {
        match &mut self.v {
            Ultra::Whole(v) => v.panel = p,
            Ultra::Split(t) => t.panel = p,
        }
    }

    fn note(&mut self, key: i32, on: bool) {
        match &mut self.v {
            Ultra::Whole(v) => v.note(key, on),
            Ultra::Split(t) => t.note(key, on),
        }
    }
}

impl Play for Played {
    fn play(&mut self, n: usize, _ext: &[f64; CHUNK], _rate: f64, mix: &Mix) {
        let gain = mix.to_array()[8];
        let j = Jacks::default();
        match &mut self.v {
            Ultra::Whole(v) => {
                v.set_drive(gain);
                for y in &mut self.out[..n] {
                    *y = v.tick_jacks(&j);
                }
            }
            Ultra::Split(t) => {
                t.set_drive(gain);
                let out = &mut self.out;
                t.process(n, |_| j, |i, y| out[i] = y);
            }
        }
    }
}

/// The DRIVE the runs carry ([`Mix`]'s gain), `db` dB.
fn mix(db: f64) -> Mix {
    let mut a = [0.0; MIX_LEN];
    a[5] = -1.0;
    a[6] = 1.0;
    a[8] = 10f64.powf(db / 20.0);
    Mix::from_array(a)
}

/// `poly_bench`'s panel at ULTRA, its cutoff at sample `at`.
fn panel(at: usize) -> Panel {
    let mut p = Panel {
        quality: Quality::HighFidelity,
        ..Panel::default()
    };
    p.osc[1].on = true;
    p.osc[1].freq = 0.52;
    p.osc[2].on = true;
    p.osc[2].range = ca72::tuning::Range::Lo;
    p.filter_mod = true;
    p.mod_wheel = 0.4;
    p.glide_on = true;
    p.glide = 0.15;
    p.emphasis = 0.5;
    p.cutoff = 0.5 + 0.2 * (at as f64 * 2.0 * std::f64::consts::PI / RATE).sin();
    p
}

/// The block period the split voices' helpers are made audio threads for (us; 0: left
/// ordinary).
static PERIOD_US: AtomicU64 = AtomicU64::new(0);

/// The split voices' helpers' work interval, when they join one (`CA72_HELPER_WG`).
static HELPER_WG: OnceLock<Option<ca72_rt::WorkInterval>> = OnceLock::new();

fn helper_start() {
    let us = PERIOD_US.load(Ordering::Relaxed);
    if us > 0 {
        let period = Duration::from_micros(us);
        let _ = ca72_rt::promote(period, period / 2);
    }
    // (Joined for good: the measurement never lets its split voices go, so no helper ends
    // while joined, which would stop the process.)
    if let Some(Some(wg)) = HELPER_WG.get()
        && let Some(token) = wg.join()
    {
        // (Off macOS the token is an empty stand-in: forgetting it does nothing there.)
        #[allow(clippy::forget_non_drop)]
        std::mem::forget(token);
    }
}

/// The driver of the helpers' interval: a member that opens it when told a block has begun
/// (`begun` moves on) and closes it when told it is done (`ended` catches up).
struct Driver {
    begun: AtomicU64,
    ended: AtomicU64,
    quit: AtomicBool,
}

fn drive_interval(d: &Driver, wg: &ca72_rt::WorkInterval, period: Duration) -> u64 {
    let _ = ca72_rt::promote(period, period / 2);
    let token = wg.join();
    let (mut seen, mut open, mut opened) = (0u64, false, 0u64);
    while !d.quit.load(Ordering::Acquire) {
        let begun = d.begun.load(Ordering::Acquire);
        if begun != seen {
            seen = begun;
            if open {
                wg.finish();
            }
            open = wg.start(period);
            opened += u64::from(open);
        }
        if open && d.ended.load(Ordering::Acquire) == seen {
            wg.finish();
            open = false;
        }
        std::thread::park_timeout(Duration::from_millis(50));
    }
    if open {
        wg.finish();
    }
    if let Some(t) = token {
        wg.leave(t);
    }
    opened
}

fn voices(n: usize, split: usize) -> Vec<Shared<Played>> {
    let proto = Voice::prototype(RATE);
    (0..n)
        .map(|k| {
            let mut v = proto.clone();
            v.panel = panel(0);
            let v = if k < split {
                Ultra::Split(Box::new(Threaded::new(v, helper_start).unwrap()))
            } else {
                Ultra::Whole(Box::new(v))
            };
            Arc::new(Mutex::new(Played {
                v,
                out: [0.0; CHUNK],
            }))
        })
        .collect()
}

/// The chord's notes at sample `n` for voice `k` of `voices`: (key, on).
fn note_at(n: usize, k: usize, voices: usize) -> Option<(i32, bool)> {
    let roots = [45i32, 41, 43, 48, 40, 45, 50, 43];
    let shape = [0i32, 7, 12, 16, 19, 24, 28, 31, 34, 36];
    let beat = (0.5 * RATE) as usize;
    let b = n / beat;
    let r = roots[b % roots.len()] + (b / roots.len() % 3) as i32;
    if k >= voices {
        return None;
    }
    if n.is_multiple_of(beat) {
        Some((r + shape[k], true))
    } else if n % beat == beat * 7 / 8 {
        Some((r + shape[k], false))
    } else {
        None
    }
}

#[test]
fn the_split_voice_with_drive_is_the_voice() {
    let n = 2;
    let whole = voices(n, 0);
    let split = voices(n, n);
    // (And the whole voice without DRIVE, which must differ: DRIVE is heard.)
    let plain = voices(n, 0);
    let m = mix(9.0);
    let mut differs = false;
    let ext = [0.0; CHUNK];
    let len = (0.6 * RATE) as usize;
    let mut at = 0;
    while at < len {
        let end = (at + CHUNK).min(len);
        // (Each run starts at a key change, as the engine's runs do.)
        let end = (at + 1..end)
            .find(|&s| (0..n).any(|k| note_at(s, k, n).is_some()))
            .unwrap_or(end);
        for vs in [&whole, &split] {
            for (k, v) in vs.iter().enumerate() {
                let mut v = v.lock().unwrap();
                v.panel(panel(at));
                if let Some((key, on)) = note_at(at, k, n) {
                    v.note(key, on);
                }
                v.play(end - at, &ext, RATE, &m);
            }
        }
        for (k, v) in plain.iter().enumerate() {
            let mut v = v.lock().unwrap();
            v.panel(panel(at));
            if let Some((key, on)) = note_at(at, k, n) {
                v.note(key, on);
            }
            v.play(end - at, &ext, RATE, &mix(0.0));
            let w = whole[k].lock().unwrap();
            differs |= (0..end - at).any(|i| (v.out[i] - w.out[i]).abs() > 1e-4);
        }
        for (a, b) in whole.iter().zip(&split) {
            let (a, b) = (a.lock().unwrap(), b.lock().unwrap());
            for i in 0..end - at {
                assert_eq!(a.out[i].to_bits(), b.out[i].to_bits(), "sample {}", at + i);
            }
        }
        at = end;
    }
    let heard = whole[0].lock().unwrap().out.iter().any(|y| y.abs() > 1e-3);
    assert!(heard, "the voice was not heard");
    assert!(differs, "DRIVE made no difference");
}

#[test]
#[ignore = "a measurement, run by hand"]
fn split_ultra_voices_in_real_time() {
    let n: usize = env("CA72_VOICES", 2);
    let split: usize = env::<usize>("CA72_SPLIT", 0).min(n);
    let budget: usize = env("CA72_BUDGET", 4);
    let block: usize = env("CA72_BLOCK", 256);
    let seconds: f64 = env("CA72_SECONDS", 10.0);
    let db: f64 = env("CA72_DRIVE", 6.0);
    let period = Duration::from_secs_f64(block as f64 / RATE);
    PERIOD_US.store(period.as_micros() as u64, Ordering::Relaxed);
    let helper_wg = env("CA72_HELPER_WG", 0) == 1;
    if helper_wg {
        let _ = HELPER_WG.set(ca72_rt::WorkInterval::new("CA-72 split voice"));
    }
    let helpers = 5 * split;
    let ask = budget.saturating_sub(helpers);
    set_budget_for_measurement(ask.max(1));
    let vs = voices(n, split);
    let mut pool = Pool::start(ask, &vs, Some(period)).unwrap();
    let workers = pool.workers();
    let driver = Arc::new(Driver {
        begun: AtomicU64::new(0),
        ended: AtomicU64::new(0),
        quit: AtomicBool::new(false),
    });
    let driving = (helper_wg && split > 0)
        .then(|| {
            let d = driver.clone();
            std::thread::Builder::new()
                .name("ca72-split-interval".into())
                .spawn(move || {
                    let wg = HELPER_WG.get().and_then(Option::as_ref);
                    wg.map_or(0, |wg| drive_interval(&d, wg, period))
                })
                .ok()
        })
        .flatten();
    let rt = ca72_rt::promote(period, period / 2);
    ca72_rt::watch();
    let m = mix(db);
    let ext = [0.0; CHUNK];
    let list: Vec<usize> = (0..n).collect();
    let warm = (2.0 * RATE) as usize / block;
    let blocks = (seconds * RATE / block as f64) as usize;
    let mut took = Vec::with_capacity(blocks);
    let (mut silent, mut peak) = (0u64, 0.0f64);
    let start = Instant::now();
    let mut next = 0u32;
    for b in 0..warm + blocks {
        let due = start + period * next;
        if let Some(w) = due.checked_duration_since(Instant::now()) {
            std::thread::sleep(w);
        }
        let t = Instant::now();
        let deadline = Some(t + period.mul_f64(DEADLINE));
        pool.block(block, None);
        if let Some(h) = &driving {
            driver.begun.store(b as u64 + 1, Ordering::Release);
            h.thread().unpark();
        }
        let at0 = b * block;
        let mut at = at0;
        while at < at0 + block {
            let end = (at + CHUNK).min(at0 + block);
            let end = (at + 1..end)
                .find(|&s| (0..n).any(|k| note_at(s, k, n).is_some()))
                .unwrap_or(end);
            for (k, v) in vs.iter().enumerate() {
                if let Ok(mut v) = v.try_lock() {
                    v.panel(panel(at0));
                    if let Some((key, on)) = note_at(at, k, n) {
                        v.note(key, on);
                    }
                }
            }
            let played = pool.play(&list, end - at, &ext, RATE, &m, false, EVERY, deadline);
            for (k, v) in vs.iter().enumerate() {
                if !played[k] {
                    silent += 1;
                } else if let Ok(v) = v.try_lock() {
                    peak = v.out[..end - at].iter().fold(peak, |p, y| p.max(y.abs()));
                }
            }
            at = end;
        }
        if b >= warm {
            took.push(t.elapsed().as_secs_f64() * 1e3);
        }
        if let Some(h) = &driving {
            driver.ended.store(b as u64 + 1, Ordering::Release);
            h.thread().unpark();
        }
        let elapsed = start.elapsed().as_secs_f64() / period.as_secs_f64();
        next = (next + 1).max(elapsed.ceil() as u32);
    }
    let p_ms = period.as_secs_f64() * 1e3;
    let late = took.iter().filter(|&&t| t > p_ms).count();
    let opened = driving.map_or(0, |h| {
        driver.quit.store(true, Ordering::Release);
        h.thread().unpark();
        h.join().unwrap_or(0)
    });
    let mean = took.iter().sum::<f64>() / took.len() as f64 / p_ms * 100.0;
    took.sort_by(f64::total_cmp);
    let q = |x: f64| took[((took.len() - 1) as f64 * x) as usize] / p_ms * 100.0;
    println!(
        "split: block={block} voices={n} split={split} budget={budget} threads={} workers={workers} helpers={helpers} drive={db} mean={mean:.1} p50={:.1} p99={:.1} p999={:.1} worst={:.1} late={late} blocks={} silent_runs={silent} demotions={} rt={} helper_wg={helper_wg} intervals={opened}",
        1 + workers + helpers,
        q(0.5),
        q(0.99),
        q(0.999),
        took[took.len() - 1] / p_ms * 100.0,
        took.len(),
        ca72_rt::demotions(),
        rt.is_ok(),
    );
    assert!(peak > 1e-3, "the voices were not heard");
    drop(pool);
    // (The split voices' helpers are joined to their interval for good: kept, not ended.)
    if helper_wg {
        std::mem::forget(vs);
    }
    set_budget_for_measurement(0);
}
