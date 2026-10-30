//! A voice lent to helpers for a run (`ca72::threaded::Helpers`, the plug-in's ULTRA:
//! decisions.md R-THREADS) gives the same samples as the voice run in turn, to the bit: one
//! crew lent in turn to two voices, each played whole between, notes pressed and released
//! inside runs, the rear jacks moving; the trigger contact, the OVERLOAD lamp and the probes
//! after each run as the voice in turn leaves them. Its helpers join a work interval, the
//! keyboard's opening it once a block, and leave it when they end.

use ca72::threaded::{Helpers, Interval, Start};
use ca72::voice::{Jacks, Quality, Voice};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

fn noop() -> Start {
    Arc::new(|| {})
}

/// The jacks at sample `i`: the external input on, the oscillators' input and the filter's
/// moving.
fn jacks(i: usize, rate: f64) -> Jacks {
    let t = i as f64 / rate;
    let tau = 2.0 * core::f64::consts::PI;
    Jacks {
        ext: 0.3 * libm::sin(tau * 220.0 * t),
        osc: (t > 0.2).then(|| 0.1 * libm::sin(tau * 5.0 * t)),
        filter: Some(libm::sin(tau * 2.0 * t)),
        detune: [1.0, -2.0, 0.5],
        ..Jacks::default()
    }
}

#[test]
fn a_lent_voice_gives_its_own_samples() {
    for quality in [Quality::NoCompromises, Quality::HighFidelity] {
        lent_samples(quality);
    }
}

fn lent_samples(quality: Quality) {
    let rate = 48_000.0;
    let proto = Voice::prototype(rate);
    let make = |seed: u64| {
        let mut v = proto.clone();
        v.set_seed(seed);
        v.panel.quality = quality;
        v.panel.ext_on = true;
        v.panel.ext_volume = 0.6;
        v.panel.osc[1].on = true;
        v.panel.emphasis = 0.6;
        v
    };
    // The voices lent and their twins played in turn.
    let mut lent = [make(3), make(4)];
    let mut whole = [make(3), make(4)];
    let mut crew = Helpers::new(proto.clone(), noop(), None).expect("the helpers start");
    // Runs of these lengths; in each, which voice borrows the crew (the other plays whole on
    // the caller's thread), and the notes within it.
    let lengths = [128usize, 37, 128, 1, 90, 128, 64, 128, 5, 128, 128, 77];
    let (mut at, mut run) = (0usize, 0usize);
    while at < (0.8 * rate) as usize {
        let n = lengths[run % lengths.len()];
        let who = run % 2;
        // A note on at 1/3 of the run and off at 2/3, on alternate runs, each voice its own.
        let key = [45, 57, 52, 64, 49][run % 5] + who as i32;
        let notes: Vec<(usize, i32, bool)> = if n > 3 {
            let mut v = vec![(n / 3, key, true)];
            if run % 3 == 2 {
                v.push((2 * n / 3, key, false));
            }
            v
        } else {
            Vec::new()
        };
        let js: Vec<Jacks> = (0..n).map(|i| jacks(at + i, rate)).collect();
        for k in 0..2 {
            let mut want = vec![0.0; n];
            let mut want_closed = vec![false; n];
            let mut next = 0;
            for i in 0..n {
                while next < notes.len() && notes[next].0 <= i {
                    whole[k].note(notes[next].1, notes[next].2);
                    next += 1;
                }
                want[i] = whole[k].tick_jacks(&js[i]);
                want_closed[i] = whole[k].trigger_closed();
            }
            let mut got = vec![0.0; n];
            let mut closed = vec![false; n];
            if k == who {
                assert!(Helpers::take(&lent[k]));
                let silent = crew.play(&mut lent[k], &notes, &js, &mut got, &mut closed);
                assert_eq!(silent, 0, "run {run}");
            } else {
                let mut next = 0;
                for i in 0..n {
                    while next < notes.len() && notes[next].0 <= i {
                        lent[k].note(notes[next].1, notes[next].2);
                        next += 1;
                    }
                    got[i] = lent[k].tick_jacks(&js[i]);
                    closed[i] = lent[k].trigger_closed();
                }
            }
            for i in 0..n {
                assert_eq!(
                    got[i].to_bits(),
                    want[i].to_bits(),
                    "voice {k}, sample {} (run {run})",
                    at + i
                );
            }
            assert_eq!(closed, want_closed, "voice {k}, run {run}");
            assert_eq!(lent[k].trigger_closed(), whole[k].trigger_closed());
            assert_eq!(lent[k].overload().to_bits(), whole[k].overload().to_bits());
            assert_eq!(lent[k].probe(), whole[k].probe());
            assert_eq!(lent[k].probe_contours(), whole[k].probe_contours());
            if run % 4 == 0 {
                assert_eq!(
                    lent[k].take_overload_peak().to_bits(),
                    whole[k].take_overload_peak().to_bits()
                );
            }
        }
        at += n;
        run += 1;
    }
    assert_eq!(crew.failed(), 0);
}

/// An interval that counts what is done with it.
#[derive(Default)]
struct Counted {
    joined: AtomicUsize,
    left: AtomicUsize,
    started: AtomicUsize,
    finished: AtomicUsize,
}

impl Interval for Counted {
    fn join(&self) -> bool {
        self.joined.fetch_add(1, Ordering::AcqRel);
        true
    }
    fn leave(&self) {
        self.left.fetch_add(1, Ordering::AcqRel);
    }
    fn start(&self) -> bool {
        self.started.fetch_add(1, Ordering::AcqRel);
        true
    }
    fn finish(&self) {
        self.finished.fetch_add(1, Ordering::AcqRel);
    }
}

#[test]
fn the_helpers_open_their_interval_once_a_block_and_leave_it_when_they_end() {
    let rate = 48_000.0;
    let proto = Voice::prototype(rate);
    let counted = Arc::new(Counted::default());
    let block = Arc::new(AtomicU64::new(0));
    let interval: Arc<dyn Interval> = counted.clone();
    let mut crew = Helpers::new(proto.clone(), noop(), Some((interval, block.clone())))
        .expect("the helpers start");
    let mut v = proto;
    v.panel.quality = Quality::NoCompromises;
    v.note(48, true);
    let js = vec![Jacks::default(); 64];
    let (mut out, mut closed) = (vec![0.0; 64], vec![false; 64]);
    let blocks = 20;
    for _ in 0..blocks {
        block.fetch_add(1, Ordering::AcqRel);
        // Two passes a block.
        for _ in 0..2 {
            assert_eq!(crew.play(&mut v, &[], &js, &mut out, &mut closed), 0);
        }
    }
    let started = counted.started.load(Ordering::Acquire);
    assert_eq!(started, blocks, "opened once a block");
    assert_eq!(counted.joined.load(Ordering::Acquire), 5);
    drop(crew);
    // (The helpers end as they see they are told to: waited for, bounded by what they do.)
    let t = std::time::Instant::now();
    while counted.left.load(Ordering::Acquire) < 5 {
        assert!(t.elapsed().as_secs() < 10, "the helpers never left");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(
        counted.finished.load(Ordering::Acquire),
        counted.started.load(Ordering::Acquire),
        "every block's interval closed"
    );
}
