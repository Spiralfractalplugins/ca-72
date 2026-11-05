//! POLY's voices on worker threads of the plug-in's own (decisions.md R11, R-THREADS): the
//! host's audio thread and workers (as many as the machine gives the instance) share each run of
//! samples' voices, every voice played whole by one of them (or at ULTRA on a crew of its own), and the host's thread sums them in the voices' order, so the output is the
//! same to the bit as with every voice on the host's thread.
//!
//! Safe Rust only, as `ca72::threaded`: each voice sits behind a mutex that the audio thread
//! only ever tries (a voice is free whenever no one has taken it for the run at hand); a run
//! is offered through atomics. One word holds the run's number, its voices and the next one
//! to take, so whoever takes a voice takes it by a compare-and-swap on the word, and a
//! worker late from an earlier run can take nothing from a later one. The audio thread takes
//! voices too, until none is left: any voice no worker has started, it plays itself. It then
//! waits for the workers' voices until the block's deadline, spinning, then yielding, then
//! sleeping a little at a time ([`Backoff`]; on Windows it goes on yielding); a voice not
//! done by then is silent for that run and sits out until its worker lets it go. Once the
//! deadline has passed, the block's further runs are not offered: the audio thread plays them.
//! Nothing here allocates once the workers are started.
//!
//! A run is over when its player returns, and the word says so: a worker held up between
//! taking a voice and locking it, which finds its run over (the engine may have listed the
//! voice again meanwhile, its keys and panel brought up), leaves the voice as it is. The mark
//! of the run a voice last played for is written only by the worker holding the voice, so a
//! late worker cannot overwrite a later run's (decisions.md R18).
//!
//! The workers are started off the audio thread and run at the audio threads' priority
//! (`ca72_rt::promote`: on macOS the time-constraint policy for the host's block period). On
//! macOS they also join a work interval of their own, which the first of them opens at each
//! run and closes as it parks: without it the system ran them, busy a third of each period,
//! about half as fast as a thread busy for most of it (decisions.md R11). Each run is due
//! within its share of a run of [`CHUNK`] samples' time (all of it in a block no longer than
//! that): given a long block's whole period, the system slowed the workers until they filled
//! it, at twice the processor time and more (decisions.md R-MAC-512). A plug-in cannot join
//! its host's audio workgroup:
//! neither VST3 nor nih-plug's CLAP hands one over. They spin a moment after a run (the next
//! run of the block follows at once) and park after it; the audio thread wakes them for each
//! run. A run carries the audio thread's floating-point flush bits (nih-plug's flush-to-zero),
//! which a worker takes before it plays.
//!
//! Workers made audio threads come out of one budget for the process (decisions.md R18,
//! R-THREADS): all instances' together are at most [`crate::engine::budget_workers`] for the
//! instances holding threads, shared evenly among them in the order they came (each takes a
//! [`Seat`]); an instance's pool holds at most [`crate::engine::instance_cap`] workers. A pool
//! whose share has changed, or which got less than its share while some is free again, is let
//! go and asked for again, so the shares follow the instances. An instance holds a pool only
//! while it needs threads (POLY's or UNISON's voices, or ULTRA's split voices): its audio
//! thread asks for one and lets it go through a [`Crew`], which the plug-in's helper thread
//! serves, starting and stopping pools (decisions.md R21, R23).
//!
//! With ULTRA a pool also holds up to [`crate::engine::split_voices`] crews of five helpers
//! (`ca72::threaded::Helpers`), each lent to a voice for a run, which then plays on six threads
//! (decisions.md R-THREADS); they count against the instance's share, and while its voices are
//! split no more of its workers are woken than the voices less one. On macOS each crew's
//! helpers join a work interval of their own, opened by the keyboard's helper.

use crate::engine::{CHUNK, MIX_LEN, Mix, Playing};
use crate::seats::SeatFiles;
use ca72::threaded::{Backoff, Helpers};
use ca72::voice::Voice;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{JoinHandle, Thread};
use std::time::{Duration, Instant};

/// The most voices a run holds.
pub const MAX: usize = 10;

/// Whether [`Pool::play`] wakes only as many threads as share a run's voices out in as few turns
/// ([`fewest`]): on macOS, where a thread woken only to wait cost a third more processor time at
/// HI with DOUBLE. Elsewhere it saved nothing measurable, and past the deadline the more threads
/// started at once, the more voices are heard (decisions.md R-MAC-512).
const EVEN: bool = cfg!(target_os = "macos");

/// [`Pool::play`]'s `threads` for every thread its voices can take, woken whether or not they
/// shorten the run: ULTRA's, whose heavy voices often run past the deadline, when the more
/// threads started at once, the more voices are heard (decisions.md R-MAC-512).
pub const EVERY: usize = usize::MAX;

/// A voice's mark when it could not be played (its lock poisoned by a panic): done, but
/// silent.
const FAILED: u64 = 1 << 63;

/// The claim word's mark of a run over: its player has returned.
const CLOSED: u64 = 1 << 16;

/// How long an idle worker spins for the next run before it parks: long enough for a block's
/// next run (its second [`crate::engine::CHUNK`]), not the next block, which is milliseconds
/// away; 200 us spent most of it after a block's last voice, a fifth of a core and more for
/// one HI voice (decisions.md R-SPIN).
const SPIN: Duration = Duration::from_micros(50);

/// A voice as the engine and the workers share it.
pub type Shared<V = Playing> = Arc<Mutex<V>>;

/// What a voice does for a run: the engine's [`Playing`] (stand-ins in the tests).
pub trait Play: Send + 'static {
    /// `n` samples, `ext` at EXTERNAL INPUT, into the voice's own buffer.
    fn play(&mut self, n: usize, ext: &[f64; CHUNK], rate: f64, mix: &Mix);

    /// Two voices' runs, each as [`Play::play`] plays it (the engine's solve their filters
    /// together at HI: decisions.md R-HI-PAIRS).
    fn play_pair(a: &mut Self, b: &mut Self, n: usize, ext: &[f64; CHUNK], rate: f64, mix: &Mix)
    where
        Self: Sized,
    {
        a.play(n, ext, rate, mix);
        b.play(n, ext, rate, mix);
    }

    /// The split crews the voice holds ([`Crew`]'s), handed to `into` (whose room is kept).
    fn recall(&mut self, into: &mut Vec<Split>) {
        let _ = into;
    }
}

impl Play for Playing {
    fn play(&mut self, n: usize, ext: &[f64; CHUNK], rate: f64, mix: &Mix) {
        Playing::play(self, n, ext, rate, mix);
    }

    fn play_pair(a: &mut Self, b: &mut Self, n: usize, ext: &[f64; CHUNK], rate: f64, mix: &Mix) {
        Playing::play_pair(a, b, n, ext, rate, mix);
    }

    fn recall(&mut self, into: &mut Vec<Split>) {
        Playing::recall(self, into);
    }
}

/// A crew of five helpers a voice borrows for a run at ULTRA (`ca72::threaded::Helpers`).
pub type Split = Arc<Mutex<Helpers>>;

/// What an instance asks threads for: POLY's or UNISON's voices to share (`workers`), and
/// ULTRA's voices to split (`split`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Need {
    pub workers: bool,
    pub split: bool,
}

/// How many threads made audio threads the pools may hold together (`most`, for as many
/// instances as hold seats, in every CA-72 process of the user: [`crate::seats`]), how many
/// this process's hold, an instance's most workers (`cap`), and this process's instances
/// holding threads, each one's share and seat file.
pub(crate) struct Budget {
    most: fn(usize) -> usize,
    cap: fn() -> usize,
    held: AtomicUsize,
    seats: Mutex<Vec<SeatEntry>>,
    seated: AtomicUsize,
    /// Where the seat files are (None: this process's seats alone), opened once; the other
    /// processes' seats at the last look, and when that was.
    dir: fn() -> Option<std::path::PathBuf>,
    files: std::sync::OnceLock<Option<SeatFiles>>,
    others: AtomicU64,
    looked: Mutex<Option<Instant>>,
}

/// One of this process's seats: its share, and the seat file it holds (None if there was none
/// to have).
struct SeatEntry {
    share: Arc<AtomicUsize>,
    file: Option<usize>,
}

/// How often the helper threads look at the other processes' seats ([`Budget::refresh`]).
pub(crate) const LOOK: Duration = Duration::from_millis(500);

impl Budget {
    /// One of the tests' own, of `most` threads, no instance's capped, its seats the process's
    /// alone.
    #[cfg(test)]
    pub(crate) const fn new(most: fn(usize) -> usize) -> Budget {
        Budget::with_files(most, || None)
    }

    /// One of the tests' own, of `most` threads, no instance's capped, its seat files in the
    /// folder `dir` gives.
    #[cfg(test)]
    pub(crate) const fn with_files(
        most: fn(usize) -> usize,
        dir: fn() -> Option<std::path::PathBuf>,
    ) -> Budget {
        Budget {
            most,
            cap: || usize::MAX,
            held: AtomicUsize::new(0),
            seats: Mutex::new(Vec::new()),
            seated: AtomicUsize::new(0),
            dir,
            files: std::sync::OnceLock::new(),
            others: AtomicU64::new(0),
            looked: Mutex::new(None),
        }
    }

    #[cfg(test)]
    pub(crate) fn held(&self) -> usize {
        self.held.load(Ordering::Acquire)
    }

    /// The instances holding seats in every process, at the last count.
    pub(crate) fn seated(&self) -> usize {
        self.seated.load(Ordering::Acquire)
    }

    /// [`Budget::refresh`] now, whenever it last looked (the tests').
    #[cfg(test)]
    pub(crate) fn refresh_now(&self) {
        *lock(&self.looked) = None;
        self.refresh();
    }

    /// The most for the instances seated now (at least one).
    fn most(&self) -> usize {
        (self.most)(self.seated.load(Ordering::Acquire).max(1))
    }

    /// How many are free now (of this process's count).
    fn free(&self) -> usize {
        self.most()
            .saturating_sub(self.held.load(Ordering::Acquire))
    }

    /// The seat files, opened the first time (never on an audio thread: seats are taken and
    /// given up only where pools are started and stopped).
    fn files(&self) -> Option<&SeatFiles> {
        self.files
            .get_or_init(|| (self.dir)().and_then(|d| SeatFiles::open(&d)))
            .as_ref()
    }

    /// A seat for an instance, its share from now on, a seat file taken with it; every seat's
    /// share worked out again, the other processes' counted now.
    fn sit(&'static self) -> Seat {
        let share = Arc::new(AtomicUsize::new(0));
        let mut seats = lock(&self.seats);
        let file = self.files().and_then(SeatFiles::take);
        seats.push(SeatEntry {
            share: share.clone(),
            file,
        });
        self.look_and_reshare(&seats);
        Seat {
            budget: self,
            share,
        }
    }

    /// The other processes' seats counted now ([`SeatFiles::others`]), and the shares worked
    /// out again for them.
    fn look_and_reshare(&self, seats: &[SeatEntry]) {
        let others = self.files().map_or(0, SeatFiles::others);
        self.others.store(others, Ordering::Release);
        *lock(&self.looked) = Some(Instant::now());
        self.reshare(seats, others);
    }

    /// The other processes' seats looked at again, at most every [`LOOK`], while this process
    /// holds any; the shares worked out again if they have changed. The helper threads' (never
    /// an audio thread's).
    pub(crate) fn refresh(&self) {
        let seats = lock(&self.seats);
        if seats.is_empty() || self.files().is_none() {
            return;
        }
        if lock(&self.looked).is_some_and(|t| t.elapsed() < LOOK) {
            return;
        }
        let before = self.others.load(Ordering::Acquire);
        let others = self.files().map_or(0, SeatFiles::others);
        *lock(&self.looked) = Some(Instant::now());
        if others != before {
            self.others.store(others, Ordering::Release);
            self.reshare(&seats, others);
        }
    }

    /// Every seat's share: the most for every instance seated in every process, evenly, the
    /// remainder to the first seats (the seat files' order, then this process's instances that
    /// hold none).
    fn reshare(&self, seats: &[SeatEntry], others: u64) {
        let mine = seats
            .iter()
            .filter_map(|e| e.file)
            .fold(0u64, |m, k| m | (1 << k));
        let filed = (others | mine).count_ones() as usize;
        let n = filed + seats.iter().filter(|e| e.file.is_none()).count();
        self.seated.store(n, Ordering::Release);
        if n == 0 {
            return;
        }
        let most = (self.most)(n);
        let mut unfiled = 0;
        for e in seats {
            let rank = match e.file {
                Some(k) => ((others | mine) & ((1u64 << k) - 1)).count_ones() as usize,
                None => {
                    unfiled += 1;
                    filed + unfiled - 1
                }
            };
            e.share
                .store(most / n + usize::from(rank < most % n), Ordering::Release);
        }
    }

    /// Up to `want` threads: as many as are left.
    fn claim(&self, want: usize) -> usize {
        let most = self.most();
        let mut held = self.held.load(Ordering::Acquire);
        loop {
            let k = want.min(most.saturating_sub(held));
            if k == 0 {
                return 0;
            }
            match self.held.compare_exchange_weak(
                held,
                held + k,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return k,
                Err(h) => held = h,
            }
        }
    }

    fn give_back(&self, k: usize) {
        self.held.fetch_sub(k, Ordering::AcqRel);
    }
}

/// The plug-in's, for all its instances in the process, its seats shared with every CA-72
/// process of the user ([`crate::seats::user_dir`]).
pub(crate) static BUDGET: Budget = Budget {
    most: crate::engine::budget_workers,
    cap: crate::engine::instance_cap,
    held: AtomicUsize::new(0),
    seats: Mutex::new(Vec::new()),
    seated: AtomicUsize::new(0),
    dir: crate::seats::user_dir,
    files: std::sync::OnceLock::new(),
    others: AtomicU64::new(0),
    looked: Mutex::new(None),
};

/// The instances holding threads in every CA-72 process of the user at the plug-in's last
/// count, and the seat files this process holds: for the measurements.
#[doc(hidden)]
pub fn seats_now() -> (usize, u32) {
    let files = BUDGET.files.get().and_then(Option::as_ref);
    (
        BUDGET.seated(),
        files.map_or(0, |f| f.held_here().count_ones()),
    )
}

/// An instance's place in the budget while it holds threads: its share, which moves as others
/// come and go, in this process or another. Given up when dropped (its seat file let go), the
/// others' shares worked out again.
pub(crate) struct Seat {
    budget: &'static Budget,
    share: Arc<AtomicUsize>,
}

impl Drop for Seat {
    fn drop(&mut self) {
        let b = self.budget;
        let mut seats = lock(&b.seats);
        if let Some(at) = seats
            .iter()
            .position(|e| Arc::ptr_eq(&e.share, &self.share))
        {
            let e = seats.remove(at);
            if let (Some(k), Some(files)) = (e.file, b.files()) {
                files.give_back(k);
            }
        }
        b.look_and_reshare(&seats);
    }
}

/// The threads a pool for `need` needs at once: its workers, or its split voices', each on six
/// threads, its front on the thread that plays it (the host's for the first, a worker's for the
/// others; with no workers, every split voice on the host's), whichever is more.
fn threads(need: Need, workers: usize, splits: usize) -> usize {
    let split = match (splits, need.workers) {
        (0, _) => 0,
        (s, true) => 6 * s - 1,
        (s, false) => 5 * s,
    };
    workers.max(split)
}

/// The workers and split crews for `need` within `share` threads, `ask` workers at most and an
/// instance's `cap`, `most` split voices (one for the one instrument).
fn sizes(need: Need, share: usize, ask: usize, cap: usize, most: usize) -> (usize, usize) {
    let workers = if need.workers {
        share.min(ask).min(cap).min(MAX - 1)
    } else {
        0
    };
    let splits = match (need.split, need.workers) {
        (false, _) => 0,
        (true, true) => most.min(share.saturating_add(1) / 6),
        // (The one instrument: one crew, DOUBLE's twin played on it after the voice.)
        (true, false) => most.min(share / 5).min(1),
    };
    (workers, splits)
}

/// A crew's work interval on macOS (`ca72_rt::WorkInterval`), for its helpers: they work to the
/// host's period (decisions.md R11, R-THREADS).
struct SplitInterval {
    interval: ca72_rt::WorkInterval,
    period: Duration,
}

thread_local! {
    /// A helper's membership of its crew's interval.
    static JOINED: std::cell::RefCell<Option<ca72_rt::JoinToken>> =
        const { std::cell::RefCell::new(None) };
}

impl ca72::threaded::Interval for SplitInterval {
    fn join(&self) -> bool {
        let token = self.interval.join();
        let joined = token.is_some();
        JOINED.with(|j| *j.borrow_mut() = token);
        joined
    }

    fn leave(&self) {
        if let Some(t) = JOINED.with(|j| j.borrow_mut().take()) {
            self.interval.leave(t);
        }
    }

    fn start(&self) -> bool {
        self.interval.start(self.period)
    }

    fn finish(&self) {
        self.interval.finish();
    }
}

/// What a run offers the workers.
struct Board<V> {
    /// The run's number (bits 32 up), whether it is over ([`CLOSED`]), its voices (bits 8
    /// to 15) and the next to take (bits 0 to 7).
    claim: AtomicU64,
    /// The voices' places among the engine's, in the order their outputs are summed.
    list: [AtomicUsize; MAX],
    /// Each voice's run number once a worker has played it for that run (written while the
    /// worker holds the voice).
    done: Box<[AtomicU64]>,
    /// The run's samples, the voices' rate and ENTROPY and SPREAD (f64 bits), EXTERNAL
    /// INPUT's samples, and the audio thread's flush bits ([`ca72_rt::flush_mode`]).
    n: AtomicUsize,
    rate: AtomicU64,
    mix: [AtomicU64; MIX_LEN],
    ext: [AtomicU64; CHUNK],
    flush: AtomicU32,
    /// Whether the run's voices are taken two at a time ([`take_some`]).
    pair: AtomicBool,
    /// The host's block is over ([`Pool::rest`]): a worker done parks at once instead of
    /// spinning for a next run, which comes only with the next block (decisions.md
    /// R-LO-WORKERS). Cleared by each run offered.
    rest: AtomicBool,
    quit: AtomicBool,
    voices: Vec<Shared<V>>,
    /// The share of a run's own time it is due within in the host's block now (f64 bits;
    /// [`Pool::block`]), and the work interval the workers join, the first of them opening it
    /// at each run and closing it as it parks (macOS; [`ca72_rt::WorkInterval`]).
    due: AtomicU64,
    interval: Option<ca72_rt::WorkInterval>,
}

/// The share of a run's own time a run is due within, in a host's block of `len` samples (0:
/// not known): a run of [`CHUNK`] samples' time shared among the block's runs, or each run's
/// whole time in a block no longer than that.
fn due_share(len: usize) -> f64 {
    if len > CHUNK {
        CHUNK as f64 / len as f64
    } else {
        1.0
    }
}

/// When a run of `n` samples at `rate` Hz opened now is due, from now, in a block whose
/// [`due_share`] is `share`: never later than `period`, the host's largest block's.
fn run_due(n: usize, rate: f64, share: f64, period: Duration) -> Duration {
    if rate > 0.0 && n > 0 {
        Duration::from_secs_f64(n as f64 / rate * share).min(period)
    } else {
        period
    }
}

/// The fewest threads, at most `threads` (at least one), that play `units` voices (or pairs)
/// taken one at a time as soon as `threads` would: as many as share them out in the same number
/// of turns. Ten doubled voices on nine threads take two turns, as on five; the four threads
/// more only waited, at a third more processor time (decisions.md R-MAC-512).
fn fewest(units: usize, threads: usize) -> usize {
    let threads = threads.clamp(1, units.max(1));
    units.div_ceil(units.div_ceil(threads).max(1))
}

fn word(run: u32, count: usize, next: usize) -> u64 {
    (u64::from(run) << 32) | ((count as u64) << 8) | next as u64
}

/// Whether run `run` is the one on the board, and not over.
fn current<V>(b: &Board<V>, run: u32) -> bool {
    let w = b.claim.load(Ordering::Acquire);
    (w >> 32) as u32 == run && w & CLOSED == 0
}

/// The next voice of run `run`, taken (its list place and its place among the voices), or
/// None once none is left (or the run is over, or another has begun).
fn take<V>(b: &Board<V>, run: u32) -> Option<(usize, usize)> {
    let mut w = b.claim.load(Ordering::Acquire);
    loop {
        let (r, count, next) = (
            (w >> 32) as u32,
            ((w >> 8) & 0xff) as usize,
            (w & 0xff) as usize,
        );
        if r != run || w & CLOSED != 0 || next >= count {
            return None;
        }
        // (The place is read before the swap: while the swap succeeds, the run has voices
        // left, so its list is not yet being written for the next.)
        let slot = b.list[next].load(Ordering::Relaxed);
        match b
            .claim
            .compare_exchange_weak(w, w + 1, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => return Some((next, slot)),
            Err(x) => w = x,
        }
    }
}

/// As [`take`], two voices at once where `two` and two are left: each one's list place and
/// place among the voices (decisions.md R-HI-PAIRS).
type Taken = ((usize, usize), Option<(usize, usize)>);

fn take_some<V>(b: &Board<V>, run: u32, two: bool) -> Option<Taken> {
    if !two {
        return take(b, run).map(|t| (t, None));
    }
    let mut w = b.claim.load(Ordering::Acquire);
    loop {
        let (r, count, next) = (
            (w >> 32) as u32,
            ((w >> 8) & 0xff) as usize,
            (w & 0xff) as usize,
        );
        if r != run || w & CLOSED != 0 || next >= count {
            return None;
        }
        let k = if next + 1 < count { 2 } else { 1 };
        let first = b.list[next].load(Ordering::Relaxed);
        let second = (k == 2).then(|| b.list[next + 1].load(Ordering::Relaxed));
        match b
            .claim
            .compare_exchange_weak(w, w + k as u64, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => return Some(((next, first), second.map(|s| (next + 1, s)))),
            Err(x) => w = x,
        }
    }
}

/// The workers and their board, and the split crews.
pub struct Pool<V: Play = Playing> {
    board: Arc<Board<V>>,
    threads: Vec<(Thread, Option<JoinHandle<()>>)>,
    run: u32,
    /// Until when, in the present block, a run offered past its deadline is still shared
    /// ([`Pool::block`]; None: none, the run played here, decisions.md R-DROPOUTS).
    grace: Option<Instant>,
    /// The budget its threads came out of, and how many.
    budget: Option<(&'static Budget, usize)>,
    /// ULTRA's split crews, the host's block count their keyboards' helpers open their
    /// intervals by, and the crews recalled from the voices before the pool is stopped (room
    /// kept for every voice's two).
    splits: Vec<Split>,
    split_block: Arc<AtomicU64>,
    recalled: Vec<Split>,
    /// What it was started for: the need, its seat's share and the share it was started at, and
    /// whether it got less.
    need: Need,
    share: Option<Arc<AtomicUsize>>,
    target: usize,
    short: bool,
}

/// What a pool is started with ([`Pool::start_from`]).
struct Spec<'a, V> {
    workers: usize,
    voices: &'a [Shared<V>],
    period: Option<Duration>,
    need: Need,
    /// Split crews, made from this voice's parts, at most.
    splits: usize,
    template: Option<&'a Voice>,
    share: Option<Arc<AtomicUsize>>,
    target: usize,
}

impl<V: Play> std::fmt::Debug for Pool<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pool")
            .field("workers", &self.threads.len())
            .field("splits", &self.splits.len())
            .finish()
    }
}

impl<V: Play> Pool<V> {
    /// Up to `workers` threads for these voices, each made an audio thread for blocks of
    /// `period` (None: left ordinary, for measurements that run as fast as they go). Audio
    /// threads come out of the process's budget, so it may start fewer, or none
    /// ([`Pool::workers`]; decisions.md R18). Not on the audio thread.
    pub fn start(
        workers: usize,
        voices: &[Shared<V>],
        period: Option<Duration>,
    ) -> std::io::Result<Pool<V>> {
        // On macOS the standard library makes a mutex's lock on its first use: each voice's
        // taken once here, off the audio thread (as `ca72::threaded`'s workers; history.md,
        // "The worker locks").
        for v in voices {
            drop(v.lock());
        }
        Pool::start_from(
            &BUDGET,
            Spec {
                workers,
                voices,
                period,
                need: Need {
                    workers: true,
                    split: false,
                },
                splits: 0,
                template: None,
                share: None,
                target: 0,
            },
        )
    }

    /// As [`Pool::start`], from `budget`, the voices' locks not taken: a [`Crew`] starts a
    /// pool while the audio thread may be trying them (they were taken once when the voices
    /// were built).
    fn start_from(budget: &'static Budget, spec: Spec<'_, V>) -> std::io::Result<Pool<V>> {
        let Spec {
            workers,
            voices,
            period,
            need,
            splits,
            template,
            share,
            target,
        } = spec;
        let splits = if template.is_some() { splits } else { 0 };
        let want = threads(need, workers, splits);
        let got = match period {
            Some(_) => budget.claim(want),
            None => want,
        };
        // (Given less, as many workers and split voices as that holds.)
        let (workers, splits) = if got < want {
            let (w, s) = sizes(need, got, workers, usize::MAX, splits);
            (w.min(workers), s)
        } else {
            (workers, splits)
        };
        let board = Arc::new(Board {
            claim: AtomicU64::new(0),
            list: std::array::from_fn(|_| AtomicUsize::new(0)),
            done: voices.iter().map(|_| AtomicU64::new(0)).collect(),
            n: AtomicUsize::new(0),
            rate: AtomicU64::new(0),
            mix: std::array::from_fn(|_| AtomicU64::new(0)),
            ext: std::array::from_fn(|_| AtomicU64::new(0)),
            flush: AtomicU32::new(0),
            pair: AtomicBool::new(false),
            rest: AtomicBool::new(false),
            quit: AtomicBool::new(false),
            voices: voices.to_vec(),
            due: AtomicU64::new(1.0f64.to_bits()),
            interval: period
                .filter(|_| workers > 0)
                .and_then(|_| ca72_rt::WorkInterval::new("CA-72 voices")),
        });
        if period.is_some() && (workers > 0 || splits > 0) {
            ca72_rt::watch();
            // (Its threads' promotions counted afresh: decisions.md R-RTKIT.)
            crate::audio_thread::WORKERS.clear();
        }
        // (The pool first: should a worker not start, those started stop and are joined and
        // the budget is given back as it drops.)
        let mut pool = Pool {
            board,
            threads: Vec::with_capacity(workers),
            run: 0,
            grace: None,
            budget: period.map(|_| (budget, got)),
            splits: Vec::with_capacity(splits),
            split_block: Arc::new(AtomicU64::new(0)),
            recalled: Vec::with_capacity(2 * voices.len()),
            need,
            share,
            target,
            short: got < want,
        };
        for k in 0..workers {
            #[cfg(test)]
            if tests::REFUSE.get() == Some(k) {
                return Err(std::io::Error::other("refused for the test"));
            }
            let b = pool.board.clone();
            let handle = std::thread::Builder::new()
                .name(format!("ca72-voices-{k}"))
                .spawn(move || work(&b, period, k == 0))?;
            pool.threads.push((handle.thread().clone(), Some(handle)));
        }
        if let Some(template) = template {
            for _ in 0..splits {
                let start: ca72::threaded::Start = Arc::new(move || {
                    if let Some(p) = period {
                        let _ = ca72_rt::promote_in(&crate::audio_thread::WORKERS, p, p / 2);
                    }
                });
                let interval = period
                    .and_then(|p| {
                        ca72_rt::WorkInterval::new("CA-72 split voice").map(|interval| {
                            Arc::new(SplitInterval {
                                interval,
                                period: p,
                            }) as Arc<dyn ca72::threaded::Interval>
                        })
                    })
                    .map(|i| (i, pool.split_block.clone()));
                let crew = Arc::new(Mutex::new(Helpers::new(template.clone(), start, interval)?));
                // (Its lock taken once here, off the audio thread: on macOS the standard
                // library makes a mutex's lock on its first use.)
                drop(crew.lock());
                pool.splits.push(crew);
            }
        }
        Ok(pool)
    }

    pub fn workers(&self) -> usize {
        self.threads.len()
    }

    /// ULTRA's split crews, to be lent to the voices for a run.
    pub fn splits(&self) -> &[Split] {
        &self.splits
    }

    /// What it was started for.
    pub(crate) fn need(&self) -> Need {
        self.need
    }

    /// Whether it is to be started again: its seat's share has changed, or it got less than
    /// its share and some is free again (decisions.md R-THREADS).
    pub(crate) fn stale(&self) -> bool {
        let moved = self
            .share
            .as_ref()
            .is_some_and(|s| s.load(Ordering::Acquire) != self.target);
        moved || (self.short && self.budget.is_some_and(|(b, _)| b.free() > 0))
    }

    /// The split crews its voices hold, taken back from every voice not held now (a late
    /// worker's keeps its), to be stopped with the pool, off the audio thread.
    fn recall(&mut self) {
        for v in &self.board.voices {
            if let Ok(mut p) = v.try_lock() {
                p.recall(&mut self.recalled);
            }
        }
    }

    /// A host's block of `len` samples begins (0: not known). Its runs are due within a run of
    /// [`CHUNK`] samples' time between them, each its share (all of a run's own in a block no
    /// longer than that): the workers' interval told the whole of a longer block's period, the
    /// system ran them slowly enough to fill it (decisions.md R-MAC-512).
    pub fn block(&mut self, len: usize, grace: Option<Instant>) {
        self.board
            .due
            .store(due_share(len).to_bits(), Ordering::Relaxed);
        self.split_block.fetch_add(1, Ordering::Release);
        self.grace = grace;
    }

    /// The host's block is over, its runs played: the workers park as they finish, rather
    /// than spin [`SPIN`] for a run that comes only with the next block. That spin was most of
    /// a worker's time beside LO's light voices (decisions.md R-LO-WORKERS).
    pub fn rest(&self) {
        self.board.rest.store(true, Ordering::Release);
    }

    /// Plays the voices at `list` (places among the voices, in the order they are summed)
    /// for `n` samples: offered to the workers and taken by this thread too, or, once
    /// `deadline` has passed, played here; two at a time where `pairs` (their filters solved
    /// together: HI, DOUBLE off) and there are more voices than threads; no more workers woken
    /// than `threads` less this one ([`MAX`]: as many as there are voices to take, less this
    /// thread's one; LO's light voices fewer, decisions.md R-LO-WORKERS), and of those only as
    /// many as share the voices out in as few turns ([`fewest`]) on macOS, unless `threads` is
    /// [`EVERY`].
    /// Returns which list places played by `deadline` (None: every one waited for, as long as it
    /// takes).
    #[allow(clippy::too_many_arguments)]
    pub fn play(
        &mut self,
        list: &[usize],
        n: usize,
        ext: &[f64; CHUNK],
        rate: f64,
        mix: &Mix,
        pairs: bool,
        threads: usize,
        deadline: Option<Instant>,
    ) -> [bool; MAX] {
        let count = list.len().min(MAX);
        let mut played = [false; MAX];
        // Past the deadline the run is still shared, and waited for until the block's grace
        // (its period's end, [`Pool::block`]): played here alone, a block cut into many short
        // runs (an LFO's or a macro's steps) took several periods once its deadline had passed
        // (decisions.md R-DROPOUTS). Past the grace, or with none, the run is not offered: the
        // workers would take voices only for this thread to give them up at once, silent for
        // the run. Late, rather than silent (R18).
        let now = Instant::now();
        let past = deadline.is_some_and(|d| now >= d);
        let until = if past { self.grace } else { deadline };
        if self.threads.is_empty() || (past && until.is_none_or(|g| now >= g)) {
            for (ok, &slot) in played.iter_mut().zip(&list[..count]) {
                if let Ok(mut p) = self.board.voices[slot].try_lock() {
                    p.play(n, ext, rate, mix);
                    *ok = true;
                }
            }
            return played;
        }
        // More voices than threads to play them: two at a time, their filters solved together
        // at HI (decisions.md R-HI-PAIRS); else one at a time, as many threads at work as can be.
        // Not where they cannot pair (DOUBLE's voices, each already played with its twin; LO;
        // ULTRA): a thread would play two one after the other while another had none, ten
        // doubled voices in five lots of four voices' work (R-POLY-CRACKLE).
        let two = pairs && count > self.threads.len() + 1;
        self.board.pair.store(two, Ordering::Relaxed);
        let run = self.offer(&list[..count], n, ext, rate, mix);
        // (No more workers woken than the voices, or pairs, less this thread's one: the others
        // would only spin, beside a split voice's helpers among them.)
        let units = if two { count.div_ceil(2) } else { count };
        let woken = if EVEN && threads != EVERY {
            fewest(units, threads.min(self.threads.len() + 1))
        } else {
            units.min(threads)
        };
        self.wake(woken.saturating_sub(1));
        let b = &*self.board;
        // This thread's share: whatever no worker has taken.
        let mut here = [false; MAX];
        while let Some((x, y)) = take_some(b, run, two) {
            here[x.0] = true;
            let px = b.voices[x.1].try_lock();
            let Some(y) = y else {
                if let Ok(mut p) = px {
                    p.play(n, ext, rate, mix);
                    played[x.0] = true;
                }
                continue;
            };
            here[y.0] = true;
            match (px, b.voices[y.1].try_lock()) {
                (Ok(mut p), Ok(mut q)) => {
                    V::play_pair(&mut p, &mut q, n, ext, rate, mix);
                    (played[x.0], played[y.0]) = (true, true);
                }
                (px, py) => {
                    if let Ok(mut p) = px {
                        p.play(n, ext, rate, mix);
                        played[x.0] = true;
                    }
                    if let Ok(mut q) = py {
                        q.play(n, ext, rate, mix);
                        played[y.0] = true;
                    }
                }
            }
        }
        for (i, &slot) in list.iter().enumerate().take(count) {
            if here[i] {
                continue;
            }
            // In real time (a deadline) the audio thread spins, its clock read every few looks:
            // it never yields or sleeps, which on a busy computer gave its processor away for
            // longer than the block had left (decisions.md R-DROPOUTS). Offline, with every
            // voice waited for, it backs off as before.
            let mut wait = Backoff::new();
            let mut looks = 0u32;
            loop {
                let d = b.done[slot].load(Ordering::Acquire);
                if d & !FAILED == u64::from(run) {
                    played[i] = d & FAILED == 0;
                    break;
                }
                let Some(until) = until else {
                    wait.snooze();
                    continue;
                };
                looks = looks.wrapping_add(1);
                if looks.is_multiple_of(16) && Instant::now() >= until {
                    break;
                }
                std::hint::spin_loop();
            }
        }
        self.close();
        played
    }

    /// Puts a run of the voices at `list` on the board (the workers not yet woken); returns
    /// its number.
    fn offer(&mut self, list: &[usize], n: usize, ext: &[f64; CHUNK], rate: f64, mix: &Mix) -> u32 {
        let b = &*self.board;
        // (Run 0 is no run: the workers start having seen it.)
        self.run = self.run.wrapping_add(1).max(1);
        for (place, &slot) in b.list.iter().zip(list) {
            place.store(slot, Ordering::Relaxed);
        }
        b.n.store(n, Ordering::Relaxed);
        b.rate.store(rate.to_bits(), Ordering::Relaxed);
        for (m, v) in b.mix.iter().zip(mix.to_array()) {
            m.store(v.to_bits(), Ordering::Relaxed);
        }
        for (e, v) in b.ext.iter().zip(&ext[..n]) {
            e.store(v.to_bits(), Ordering::Relaxed);
        }
        b.flush.store(ca72_rt::flush_mode(), Ordering::Relaxed);
        b.rest.store(false, Ordering::Relaxed);
        b.claim
            .store(word(self.run, list.len(), 0), Ordering::Release);
        self.run
    }

    fn wake(&self, n: usize) {
        for (t, _) in self.threads.iter().take(n) {
            t.unpark();
        }
    }

    /// The run is over, before the engine next tries any of its voices' locks: a worker
    /// that locks one after the engine has (and so after this) finds it so.
    fn close(&self) {
        self.board.claim.fetch_or(CLOSED, Ordering::AcqRel);
    }
}

impl<V: Play> Drop for Pool<V> {
    fn drop(&mut self) {
        self.board.quit.store(true, Ordering::Release);
        for (t, h) in &mut self.threads {
            t.unpark();
            if let Some(h) = h.take() {
                let _ = h.join();
            }
        }
        if let Some((budget, k)) = self.budget {
            budget.give_back(k);
        }
    }
}

/// An instance's threads, held only while it needs them (decisions.md R21, R-THREADS): its
/// workers while POLY or UNISON is on, its split crews while ULTRA is chosen; its audio thread asks
/// for a pool and lets it go ([`Crew::follow`]), never waiting and never starting or stopping a
/// thread; the plug-in's helper thread stops the pools let go and starts the one asked for
/// ([`Crew::serve`]), for the voices last planned ([`Crew::plan`]). A pool started for a plan
/// no longer asked for (POLY off again meanwhile, the voices built anew, the plug-in
/// deactivated) is stopped where it was started, so no worker is kept from the budget.
pub struct Crew<V: Play = Playing> {
    budget: &'static Budget,
    /// Held while the helper thread serves, so that a new plan waits for a pool being
    /// started for an earlier one, and stops it before a pool of its own claims workers.
    serving: Mutex<()>,
    plan: Mutex<Plan<V>>,
    hand: Mutex<Hand<V>>,
    /// The instance's seat in the budget while it holds or asks for threads (the helper
    /// thread's).
    seat: Mutex<Option<Seat>>,
}

/// What a pool is started for: the voices, how many workers at most and the period they are
/// made audio threads for, and a voice to make split crews from (ULTRA's); a new generation
/// each time it is set. (The voice boxed: inline, the crew and each plan moved it about the
/// stack of the host's thread that made, started or stopped the instance: decisions.md
/// R-HOST-STACK.)
struct Plan<V> {
    generation: u64,
    voices: Vec<Shared<V>>,
    workers: usize,
    period: Option<Duration>,
    template: Option<Box<Voice>>,
}

/// What passes between the audio thread and the helper thread.
struct Hand<V: Play> {
    /// The plan's generation the audio thread asks a pool for (0: none), and for what.
    want: u64,
    need: Need,
    /// The answer for a generation: its pool, or none (the system would not start one).
    ready: Option<(u64, Option<Pool<V>>)>,
    /// A pool the audio thread let go, to be stopped.
    back: Option<Pool<V>>,
}

/// Where the audio thread's ask for workers stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Ask {
    #[default]
    Not,
    /// Asked; until the answer comes, the voices play on the audio thread.
    Waiting,
    /// Answered: a pool, or none, until POLY is next switched on.
    Answered,
}

impl<V: Play> std::fmt::Debug for Crew<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Crew").finish_non_exhaustive()
    }
}

impl<V: Play> Default for Crew<V> {
    fn default() -> Self {
        Crew::new()
    }
}

/// A lock the audio thread never waits for (it only tries it), taken off it.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl<V: Play> Crew<V> {
    /// With nothing planned, its workers from the process's budget.
    pub fn new() -> Crew<V> {
        Crew::with_budget(&BUDGET)
    }

    pub(crate) fn with_budget(budget: &'static Budget) -> Crew<V> {
        let crew = Crew {
            budget,
            serving: Mutex::new(()),
            plan: Mutex::new(Plan {
                generation: 0,
                voices: Vec::new(),
                workers: 0,
                period: None,
                template: None,
            }),
            hand: Mutex::new(Hand {
                want: 0,
                need: Need::default(),
                ready: None,
                back: None,
            }),
            seat: Mutex::new(None),
        };
        // (Its lock taken once here, off the audio thread: on macOS the standard library may
        // make a mutex's lock on its first use.)
        drop(crew.hand.try_lock());
        crew
    }

    /// Pools from now on for up to `workers` workers sharing `voices`, made audio threads for
    /// blocks of `period` ([`Pool::start`]), with ULTRA's split crews made from `template`'s
    /// parts; returns the plan's generation. Any pool started or let go for an earlier plan is
    /// stopped, here (one being started is waited for), its ask withdrawn and the seat given
    /// up. Not on the audio thread.
    pub fn plan(
        &self,
        voices: &[Shared<V>],
        workers: usize,
        period: Option<Duration>,
        template: Option<Box<Voice>>,
    ) -> u64 {
        let _serving = lock(&self.serving);
        let (generation, old) = {
            let mut p = lock(&self.plan);
            p.generation += 1;
            p.workers = workers;
            p.period = period;
            (
                p.generation,
                (
                    std::mem::replace(&mut p.voices, voices.to_vec()),
                    std::mem::replace(&mut p.template, template),
                ),
            )
        };
        drop(old);
        let stop = {
            let mut h = lock(&self.hand);
            h.want = 0;
            (h.ready.take(), h.back.take())
        };
        drop(stop);
        drop(lock(&self.seat).take());
        generation
    }

    /// Nothing planned: as [`Crew::plan`], for no voices.
    pub fn clear(&self) {
        self.plan(&[], 0, None, None);
    }

    /// A pool for plan `generation` and `need` while it is the plan, or none (another planned
    /// since, or the system would not start one): within the instance's share of the budget,
    /// its seat taken if it has none. Not on the audio thread.
    pub fn start(&self, generation: u64, need: Need) -> Option<Pool<V>> {
        let p = lock(&self.plan);
        if p.generation != generation
            || p.voices.is_empty()
            || (p.workers == 0 && !need.split)
            || need == Need::default()
        {
            return None;
        }
        let share = lock(&self.seat)
            .get_or_insert_with(|| self.budget.sit())
            .share
            .clone();
        let target = share.load(Ordering::Acquire);
        // (Unpaced, for measurements, the share does not bound it.)
        let room = if p.period.is_some() {
            target
        } else {
            usize::MAX
        };
        let (workers, splits) = sizes(
            need,
            room,
            p.workers,
            (self.budget.cap)(),
            crate::engine::split_voices(),
        );
        Pool::start_from(
            self.budget,
            Spec {
                workers,
                voices: &p.voices,
                period: p.period,
                need,
                splits,
                template: p.template.as_deref(),
                share: Some(share),
                target,
            },
        )
        .ok()
    }

    /// The audio thread's asks served: the pools it let go stopped (their workers back in the
    /// budget), then a pool started for the plan it asks for, unless one is ready. Not on the
    /// audio thread: the plug-in's helper thread (decisions.md R21, R23).
    pub fn serve(&self) {
        let _serving = lock(&self.serving);
        if let Some((generation, need)) = self.tidy() {
            self.deliver(generation, self.start(generation, need));
        }
    }

    /// The pools let go, or ready for an ask withdrawn, stopped (and with nothing asked for,
    /// the seat given up); the generation and need asked for, if a pool is to be started.
    fn tidy(&self) -> Option<(u64, Need)> {
        let (stop, start, idle) = {
            let mut h = lock(&self.hand);
            let want = h.want;
            let stale = h.ready.take_if(|(g, _)| *g != want);
            let start = (want != 0 && h.ready.is_none()).then_some((want, h.need));
            let back = h.back.take();
            let idle = want == 0 && h.ready.is_none() && back.is_some();
            ((back, stale), start, idle)
        };
        // (Stopped once the lock is let go: the audio thread may be trying it.)
        drop(stop);
        if idle {
            drop(lock(&self.seat).take());
        }
        start
    }

    /// The answer for plan `generation` to the audio thread, while it still asks for it and
    /// has none; else its pool stopped here.
    fn deliver(&self, generation: u64, pool: Option<Pool<V>>) {
        let late = {
            let mut h = lock(&self.hand);
            if h.want == generation && h.ready.is_none() {
                h.ready = Some((generation, pool));
                None
            } else {
                pool
            }
        };
        drop(late);
    }

    /// At a block's start, on the audio thread, without waiting (decisions.md R21): its `pool`
    /// and `ask` brought towards what POLY asks (`on`) of plan `generation`. On, a pool
    /// started for the plan is taken once it is ready, or asked for, once (until it comes, the
    /// caller plays every voice itself); off, the pool is let go to be stopped off this thread
    /// and the ask withdrawn. A pool moves; none is started or stopped here. Should the
    /// helper thread hold the lock, this is tried again at the next block. Returns whether
    /// [`Crew::serve`] is due.
    pub(crate) fn follow(
        &self,
        on: Option<Need>,
        generation: u64,
        pool: &mut Option<Pool<V>>,
        ask: &mut Ask,
    ) -> bool {
        // (A pool for another need, or whose share has moved: let go and asked for again.)
        let again = match (on, pool.as_ref()) {
            (Some(need), Some(p)) => p.need() != need || p.stale(),
            _ => false,
        };
        let settled = match on {
            Some(_) => !again && (pool.is_some() || *ask == Ask::Answered),
            None => pool.is_none() && *ask == Ask::Not,
        };
        if settled || generation == 0 {
            return false;
        }
        let Ok(mut h) = self.hand.try_lock() else {
            return false;
        };
        if let Some(need) = on {
            if again {
                if h.back.is_some() {
                    // (The last one let go not stopped yet: asked again at a later block.)
                    return true;
                }
                if let Some(mut p) = pool.take() {
                    p.recall();
                    h.back = Some(p);
                }
                h.want = generation;
                h.need = need;
                *ask = Ask::Waiting;
                return true;
            }
            // (With no pool here: taking the answer drops nothing. One may be ready for an ask
            // withdrawn and made again since.)
            if let Some((_, answer)) = h.ready.take_if(|(g, _)| *g == generation) {
                *pool = answer;
                h.want = 0;
                *ask = Ask::Answered;
                return false;
            }
            if *ask == Ask::Not {
                h.want = generation;
                h.need = need;
                *ask = Ask::Waiting;
                return true;
            }
            // (Asked for another need meanwhile: the answer, when it comes, is let go again.)
            h.need = need;
            return false;
        }
        // A pool started for an ask withdrawn is stopped by the helper thread: a pool
        // ready, at once, or one being started, as it comes.
        if h.want == generation {
            h.want = 0;
        }
        *ask = Ask::Not;
        let mut due = h.ready.is_some();
        if pool.is_some() && h.back.is_none() {
            if let Some(mut p) = pool.take() {
                p.recall();
                h.back = Some(p);
            }
            due = true;
        }
        due
    }
}

/// A worker: made an audio thread, it waits for a run (spinning a moment, then parked) and
/// plays the voices it takes.
fn work<V: Play>(b: &Board<V>, period: Option<Duration>, driver: bool) {
    // What a thread sets up the first time it parks or reads the clock is done before it
    // becomes an audio thread.
    std::thread::park_timeout(Duration::from_nanos(1));
    let _ = Instant::now().elapsed();
    // (Where the system refuses, the standalone asks RTKit, through `ca72_rt`'s thread: the
    // answer is said there, and counted for the editor. decisions.md R-RTKIT.)
    if let Some(period) = period
        && let Err(e) = ca72_rt::promote_in(&crate::audio_thread::WORKERS, period, period / 2)
    {
        // (Said once a process, here on a worker: not the audio thread. They play on as
        // ordinary threads, which other work preempts: decisions.md R-DROPOUTS.)
        static SAID: AtomicBool = AtomicBool::new(false);
        if !SAID.swap(true, Ordering::AcqRel) {
            eprintln!("CA-72: POLY's workers run at ordinary priority: {e}");
        }
    }
    let token = b.interval.as_ref().and_then(ca72_rt::WorkInterval::join);
    // (The first worker opens the interval at each run, due within the run's share of the
    // block's time (`Pool::block`), and closes it at the next or when it parks.)
    let interval = if driver && token.is_some() {
        b.interval.as_ref().zip(period)
    } else {
        None
    };
    let mut open = false;
    let mut seen = 0u32;
    let mut ext = [0.0f64; CHUNK];
    let mut flush = ca72_rt::flush_mode();
    // A thread must leave its work interval before it ends (macOS stops the process
    // otherwise), the interval it opened closed first.
    struct Member<'a>(Option<(&'a ca72_rt::WorkInterval, ca72_rt::JoinToken)>);
    impl Drop for Member<'_> {
        fn drop(&mut self) {
            if let Some((i, t)) = self.0.take() {
                i.leave(t);
            }
        }
    }
    let _member = Member(b.interval.as_ref().zip(token));
    loop {
        let since = Instant::now();
        let mut spins = 0u32;
        let run = loop {
            if b.quit.load(Ordering::Acquire) {
                if let Some((i, _)) = interval
                    && open
                {
                    i.finish();
                }
                return;
            }
            let run = (b.claim.load(Ordering::Acquire) >> 32) as u32;
            if run != seen {
                break run;
            }
            spins = spins.wrapping_add(1);
            // (Once the block is over, no spin: its next run is the next block's.)
            let rest = b.rest.load(Ordering::Acquire);
            if !rest && (!spins.is_multiple_of(64) || since.elapsed() < SPIN) {
                std::hint::spin_loop();
            } else {
                if let Some((i, _)) = interval
                    && open
                {
                    i.finish();
                    open = false;
                }
                std::thread::park();
            }
        };
        seen = run;
        // A run's own interval, due within its share of the block's time: one for the block,
        // due by its period, slowed the workers to fill it, and once the first worker had
        // parked between runs, the block's later runs were played outside it (decisions.md
        // R-MAC-512).
        if let Some((i, period)) = interval {
            if open {
                i.finish();
            }
            let share = f64::from_bits(b.due.load(Ordering::Relaxed));
            let rate = f64::from_bits(b.rate.load(Ordering::Relaxed));
            open = i.start(run_due(b.n.load(Ordering::Relaxed), rate, share, period));
        }
        // The run's settings, read before any voice is taken (while one can be taken, they
        // are this run's), and the audio thread's flush bits taken: a voice's samples are
        // the same whichever thread plays it.
        let n = b.n.load(Ordering::Relaxed).min(CHUNK);
        let rate = f64::from_bits(b.rate.load(Ordering::Relaxed));
        let mix = Mix::from_array(std::array::from_fn(|k| {
            f64::from_bits(b.mix[k].load(Ordering::Relaxed))
        }));
        for (x, e) in ext.iter_mut().zip(&b.ext[..n]) {
            *x = f64::from_bits(e.load(Ordering::Relaxed));
        }
        let want = b.flush.load(Ordering::Relaxed);
        if want != flush {
            ca72_rt::set_flush_mode(want);
            flush = want;
        }
        let two = b.pair.load(Ordering::Relaxed);
        let lock = |slot: usize| match b.voices[slot].lock() {
            Ok(p) => (p, false),
            Err(e) => (e.into_inner(), true),
        };
        let failed = u64::from(run) | FAILED;
        while let Some(((_, slot), other)) = take_some(b, run, two) {
            let Some((_, other)) = other else {
                let (mut p, poisoned) = lock(slot);
                // Held up between taking the voice and locking it, past the run's end: the
                // voice is left as it is, neither played nor marked (decisions.md R18).
                if !current(b, run) {
                    break;
                }
                let mark = if poisoned {
                    failed
                } else {
                    p.play(n, &ext, rate, &mix);
                    u64::from(run)
                };
                // The voice let go before it is marked: once marked, the caller's thread may
                // close the run and take the voice in the next, and a lock still held here would
                // fail its `try_lock` there, the voice silent for that run (seen in CI as samples
                // that differed from one thread's).
                drop(p);
                b.done[slot].store(mark, Ordering::Release);
                continue;
            };
            // Two voices taken: their filters solved together where both are free now
            // (decisions.md R-HI-PAIRS); else each alone, a free one first, so that a voice
            // another thread still holds keeps its partner waiting no longer than alone.
            let free = |slot: usize| match b.voices[slot].try_lock() {
                Ok(p) => Some((p, false)),
                Err(std::sync::TryLockError::Poisoned(e)) => Some((e.into_inner(), true)),
                Err(std::sync::TryLockError::WouldBlock) => None,
            };
            match (free(slot), free(other)) {
                (Some((mut p, false)), Some((mut q, false))) => {
                    if !current(b, run) {
                        break;
                    }
                    V::play_pair(&mut p, &mut q, n, &ext, rate, &mix);
                    drop(p);
                    drop(q);
                    b.done[slot].store(u64::from(run), Ordering::Release);
                    b.done[other].store(u64::from(run), Ordering::Release);
                }
                (gp, gq) => {
                    let mut each = [(slot, gp), (other, gq)];
                    each.sort_by_key(|(_, g)| g.is_none());
                    let mut over = false;
                    for (s, g) in each {
                        let (mut p, poisoned) = g.unwrap_or_else(|| lock(s));
                        if !current(b, run) {
                            over = true;
                            break;
                        }
                        let mark = if poisoned {
                            failed
                        } else {
                            p.play(n, &ext, rate, &mix);
                            u64::from(run)
                        };
                        drop(p);
                        b.done[s].store(mark, Ordering::Release);
                    }
                    if over {
                        break;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::ThreadId;

    thread_local! {
        /// The worker [`Pool::start`] fails to start, as if the system had refused it.
        pub(super) static REFUSE: std::cell::Cell<Option<usize>> =
            const { std::cell::Cell::new(None) };
    }

    /// A voice that counts its runs (and those played as one of a pair) and keeps where and
    /// how it played the last.
    #[derive(Default)]
    struct Fake {
        runs: usize,
        paired: usize,
        n: usize,
        thread: Option<ThreadId>,
        flush: u32,
        /// Set as its next run starts, the run then held until `hold` is set.
        started: Option<Arc<AtomicBool>>,
        hold: Option<Arc<AtomicBool>>,
        /// Set as it plays, and a pause after.
        release: Option<(Arc<AtomicBool>, Duration)>,
        /// A pause when played on this thread (so that the workers take the others).
        slow_on: Option<ThreadId>,
        /// Set when played on any other thread; with it, the pause on `slow_on` lasts until
        /// it is set (5 s at most) rather than 2 ms, so a loaded system's late worker still
        /// takes a voice.
        elsewhere: Option<Arc<AtomicBool>>,
    }

    impl Play for Fake {
        fn play(&mut self, n: usize, _: &[f64; CHUNK], _: f64, _: &Mix) {
            self.runs += 1;
            self.n = n;
            self.thread = Some(std::thread::current().id());
            self.flush = ca72_rt::flush_mode();
            if let Some(s) = self.started.take() {
                s.store(true, Ordering::Release);
                if let Some(h) = self.hold.take() {
                    while !h.load(Ordering::Acquire) {
                        std::thread::sleep(Duration::from_micros(200));
                    }
                }
            }
            if let Some((r, pause)) = &self.release {
                r.store(true, Ordering::Release);
                std::thread::sleep(*pause);
            }
            if self.slow_on.is_some() && self.slow_on == self.thread {
                match &self.elsewhere {
                    Some(e) => {
                        let t = Instant::now();
                        while !e.load(Ordering::Acquire) && t.elapsed() < Duration::from_secs(5) {
                            std::thread::sleep(Duration::from_micros(100));
                        }
                    }
                    None => std::thread::sleep(Duration::from_millis(2)),
                }
            } else if let Some(e) = &self.elsewhere {
                e.store(true, Ordering::Release);
            }
        }

        fn play_pair(a: &mut Self, b: &mut Self, n: usize, ext: &[f64; CHUNK], rate: f64, m: &Mix) {
            a.paired += 1;
            b.paired += 1;
            a.play(n, ext, rate, m);
            b.play(n, ext, rate, m);
        }
    }

    fn fakes(k: usize) -> Vec<Shared<Fake>> {
        (0..k)
            .map(|_| Arc::new(Mutex::new(Fake::default())))
            .collect()
    }

    fn mix() -> Mix {
        Mix::from_array([0.0; MIX_LEN])
    }

    const EXT: [f64; CHUNK] = [0.0; CHUNK];

    /// What POLY asks for at HI.
    const POLY_NEED: Need = Need {
        workers: true,
        split: false,
    };
    const POLY: Option<Need> = Some(POLY_NEED);

    /// A pool of `workers` for `voices`, as [`Pool::start`] starts one.
    fn spec(workers: usize, voices: &[Shared<Fake>], period: Option<Duration>) -> Spec<'_, Fake> {
        Spec {
            workers,
            voices,
            period,
            need: POLY_NEED,
            splits: 0,
            template: None,
            share: None,
            target: 0,
        }
    }

    /// Waits up to 5 s for `f`.
    fn until(what: &str, f: impl Fn() -> bool) {
        let t = Instant::now();
        while !f() {
            assert!(t.elapsed() < Duration::from_secs(5), "{what}");
            std::thread::sleep(Duration::from_micros(100));
        }
    }

    /// No more threads woken than share the voices out in as few turns as all of them would
    /// (decisions.md R-MAC-512): ten doubled voices on the Mac's nine threads take two turns,
    /// as on five; on the Ryzen's ten, or a computer of four workers, every thread is woken as
    /// before.
    #[test]
    fn as_few_threads_are_woken_as_take_as_many_turns() {
        assert_eq!(fewest(10, 9), 5);
        assert_eq!(fewest(10, 10), 10);
        assert_eq!(fewest(10, 5), 5);
        assert_eq!(fewest(10, 4), 4);
        assert_eq!(fewest(8, 9), 8);
        assert_eq!(fewest(8, 5), 4);
        assert_eq!(fewest(5, 9), 5);
        assert_eq!(fewest(1, 9), 1);
        assert_eq!(fewest(0, 9), 0);
        for units in 1..=MAX {
            for threads in 1..=MAX {
                let k = fewest(units, threads);
                assert!(
                    k >= 1 && k <= threads.min(units),
                    "{units} on {threads}: {k}"
                );
                assert_eq!(
                    units.div_ceil(k),
                    units.div_ceil(threads.min(units)),
                    "{units} on {threads}: {k}"
                );
            }
        }
    }

    /// A block's runs are due within a run of `CHUNK` samples' time between them, each its
    /// share; in a block no longer than that, each within its own time, as one block was before
    /// (decisions.md R-MAC-512).
    #[test]
    fn a_long_blocks_runs_are_due_within_one_runs_time() {
        let rate = 48_000.0;
        let period = Duration::from_secs_f64(512.0 / rate);
        let near = |d: Duration, us: f64| (d.as_secs_f64() * 1e6 - us).abs() < 0.01;
        // 128 frames or fewer: a run's own time, a block's period.
        assert!(near(run_due(128, rate, due_share(128), period), 2666.67));
        assert!(near(run_due(64, rate, due_share(64), period), 1333.33));
        // 512 frames: four runs of 128 within 128 samples' time; a glide's runs of 32 their
        // share of it.
        assert!(near(run_due(128, rate, due_share(512), period), 666.67));
        assert!(near(run_due(32, rate, due_share(512), period), 166.67));
        // Not known (offline): a run's own time.
        assert!(near(run_due(128, rate, due_share(0), period), 2666.67));
        // Never past the largest block's period; with no rate, that period.
        assert_eq!(
            run_due(128, rate, 1.0, Duration::from_millis(1)),
            Duration::from_millis(1)
        );
        assert_eq!(run_due(128, 0.0, 1.0, period), period);
    }

    /// More voices than threads: taken two at a time where they pair (HI without DOUBLE,
    /// decisions.md R-HI-PAIRS), else one at a time, so that no thread plays two voices that
    /// cannot pair, one after the other, while another has none (R-POLY-CRACKLE: ten doubled
    /// voices on four threads had five lots of four voices' work).
    #[test]
    fn voices_that_cannot_pair_are_taken_one_at_a_time() {
        for pairs in [false, true] {
            let voices = fakes(6);
            let mut pool = Pool::start(1, &voices, None).unwrap();
            for _ in 0..20 {
                let played = pool.play(
                    &[0, 1, 2, 3, 4, 5],
                    32,
                    &EXT,
                    48_000.0,
                    &mix(),
                    pairs,
                    MAX,
                    None,
                );
                assert!(played[..6].iter().all(|&p| p), "{played:?}");
            }
            let paired: usize = voices.iter().map(|v| v.lock().unwrap().paired).sum();
            let runs: usize = voices.iter().map(|v| v.lock().unwrap().runs).sum();
            assert_eq!(runs, 120);
            if pairs {
                assert_eq!(paired, runs, "voices that pair, played alone");
            } else {
                assert_eq!(paired, 0, "voices that cannot pair, taken two at a time");
            }
        }
    }

    /// The workers park at a block's end ([`Pool::rest`]) and are woken for the next block's
    /// first run: every run after a rest is shared still, none of its voices waited for in
    /// vain (decisions.md R-LO-WORKERS).
    #[test]
    fn a_pool_at_rest_shares_the_next_blocks_run() {
        let voices = fakes(3);
        let me = std::thread::current().id();
        let elsewhere = Arc::new(AtomicBool::new(false));
        for v in &voices {
            let mut v = v.lock().unwrap();
            v.slow_on = Some(me);
            v.elsewhere = Some(elsewhere.clone());
        }
        let mut pool = Pool::start(2, &voices, None).unwrap();
        for _ in 0..20 {
            elsewhere.store(false, Ordering::Release);
            pool.block(CHUNK, None);
            let played = pool.play(&[0, 1, 2], 64, &EXT, 48_000.0, &mix(), false, MAX, None);
            assert_eq!(&played[..3], &[true; 3]);
            let elsewhere = voices
                .iter()
                .filter(|v| v.lock().unwrap().thread != Some(me))
                .count();
            assert!(
                elsewhere > 0,
                "a run after a rest played on this thread alone"
            );
            pool.rest();
            assert!(pool.board.rest.load(Ordering::Acquire));
            // (Long enough for the workers to park.)
            std::thread::sleep(Duration::from_millis(3));
        }
        // A run offered is no longer at rest.
        pool.play(&[0], 64, &EXT, 48_000.0, &mix(), false, MAX, None);
        assert!(!pool.board.rest.load(Ordering::Acquire));
    }

    /// A worker held up between taking a voice and locking it, which locks it only after
    /// the run is over (the voice listed again meanwhile), leaves it alone: it neither plays
    /// it with the old run's samples and settings nor marks it.
    #[test]
    fn a_worker_late_for_a_run_over_leaves_its_voice_alone() {
        let voices = fakes(2);
        let mut pool = Pool::start(1, &voices, None).unwrap();
        let held = voices[0].lock().unwrap();
        let run = pool.offer(&[0], 64, &EXT, 48_000.0, &mix());
        pool.wake(usize::MAX);
        until("the worker took nothing", || {
            pool.board.claim.load(Ordering::Acquire) & 0xff == 1
        });
        // (The worker waits for the voice's lock; the run's player gives up on it.)
        std::thread::sleep(Duration::from_millis(10));
        pool.close();
        drop(held);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(voices[0].lock().unwrap().runs, 0, "played for a run over");
        assert_ne!(pool.board.done[0].load(Ordering::Acquire), u64::from(run));
        // The next run plays it as usual.
        let played = pool.play(&[0, 1], 32, &EXT, 48_000.0, &mix(), true, MAX, None);
        assert_eq!(&played[..2], &[true, true]);
        let v = voices[0].lock().unwrap();
        assert_eq!((v.runs, v.n), (1, 32));
    }

    /// A worker late from a run over that finishes its voice while a later run is played
    /// marks that voice, never the later run's voice at the same list place: the later run's
    /// voices are all heard, and nothing is waited for until the deadline.
    #[test]
    fn a_late_worker_does_not_unmark_a_later_runs_voice() {
        let voices = fakes(3);
        let (started, go) = (
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
        );
        {
            let mut v = voices[0].lock().unwrap();
            v.started = Some(started.clone());
            v.hold = Some(go.clone());
        }
        voices[2].lock().unwrap().release = Some((go.clone(), Duration::from_millis(20)));
        let mut pool = Pool::start(1, &voices, None).unwrap();
        // Voice 0 alone, taken by the worker, which is held up playing it past the run's end.
        pool.offer(&[0], 64, &EXT, 48_000.0, &mix());
        pool.wake(usize::MAX);
        until("the worker took nothing", || {
            started.load(Ordering::Acquire)
        });
        pool.close();
        // Voices 1 and 2 here (the worker is busy): voice 2 lets voice 0 go, and its worker
        // finishes it while this thread plays voice 2.
        let t = Instant::now();
        let deadline = Some(t + Duration::from_millis(500));
        let played = pool.play(&[1, 2], 32, &EXT, 48_000.0, &mix(), true, MAX, deadline);
        assert_eq!(&played[..2], &[true, true]);
        assert!(
            t.elapsed() < Duration::from_millis(400),
            "{:?}",
            t.elapsed()
        );
        // (Its own mark, the run it was taken for.)
        until("voice 0 not marked", || {
            pool.board.done[0].load(Ordering::Acquire) == 1
        });
    }

    /// Once the block's deadline has passed, a run is not offered to the workers: the
    /// caller's thread plays every voice, and every voice is heard.
    #[test]
    fn past_the_deadline_the_callers_thread_plays_every_voice() {
        let voices = fakes(4);
        let mut pool = Pool::start(2, &voices, None).unwrap();
        let past = Instant::now();
        std::thread::sleep(Duration::from_millis(1));
        let played = pool.play(
            &[0, 1, 2, 3],
            64,
            &EXT,
            48_000.0,
            &mix(),
            true,
            MAX,
            Some(past),
        );
        assert_eq!(&played[..4], &[true; 4]);
        let me = std::thread::current().id();
        for v in &voices {
            assert_eq!(v.lock().unwrap().thread, Some(me));
        }
        assert_eq!(pool.board.claim.load(Ordering::Acquire), 0, "offered");
    }

    /// The workers play with the caller's flush bits, whatever they are.
    #[test]
    fn the_workers_take_the_callers_flush_bits() {
        let voices = fakes(3);
        let me = std::thread::current().id();
        for v in &voices {
            v.lock().unwrap().slow_on = Some(me);
        }
        let mut pool = Pool::start(2, &voices, None).unwrap();
        let was = ca72_rt::flush_mode();
        for mode in [ca72_rt::FLUSH, 0, ca72_rt::FLUSH] {
            ca72_rt::set_flush_mode(mode);
            let mut elsewhere = 0;
            for _ in 0..50 {
                let played = pool.play(&[0, 1, 2], 16, &EXT, 48_000.0, &mix(), true, MAX, None);
                assert_eq!(&played[..3], &[true; 3]);
                for v in &voices {
                    let v = v.lock().unwrap();
                    assert_eq!(v.flush, mode);
                    elsewhere += usize::from(v.thread != Some(me));
                }
                if elsewhere > 0 {
                    break;
                }
            }
            assert!(elsewhere > 0, "no voice played on a worker");
        }
        ca72_rt::set_flush_mode(was);
    }

    /// Audio-thread workers come out of one budget: pools together hold at most its size,
    /// one given none plays every voice on the caller's thread, and a pool's go back when it
    /// stops. Ordinary workers (for measurements) are not counted.
    #[test]
    fn workers_come_out_of_one_budget() {
        static THREE: Budget = Budget::new(|_| 3);
        let voices = fakes(2);
        let period = Some(Duration::from_millis(5));
        let a = Pool::start_from(&THREE, spec(2, &voices, period)).unwrap();
        let b = Pool::start_from(&THREE, spec(2, &voices, period)).unwrap();
        let mut c = Pool::start_from(&THREE, spec(2, &voices, period)).unwrap();
        assert_eq!((a.workers(), b.workers(), c.workers()), (2, 1, 0));
        assert_eq!(THREE.held(), 3);
        let played = c.play(&[0, 1], 16, &EXT, 48_000.0, &mix(), true, MAX, None);
        assert_eq!(&played[..2], &[true, true]);
        let ordinary = Pool::start_from(&THREE, spec(4, &voices, None)).unwrap();
        assert_eq!(ordinary.workers(), 4);
        drop(a);
        assert_eq!(THREE.held(), 1);
        let d = Pool::start_from(&THREE, spec(4, &voices, period)).unwrap();
        assert_eq!(d.workers(), 2);
        drop((b, c, d, ordinary));
        assert_eq!(THREE.held(), 0);
    }

    /// A worker the system will not start: those started stop and are joined (nothing holds
    /// the voices any more), and the budget is given back.
    #[test]
    fn a_worker_not_started_stops_the_others() {
        static FOUR: Budget = Budget::new(|_| 4);
        let voices = fakes(2);
        REFUSE.set(Some(2));
        let r = Pool::start_from(&FOUR, spec(3, &voices, Some(Duration::from_millis(5))));
        REFUSE.set(None);
        assert!(r.is_err());
        assert_eq!(FOUR.held.load(Ordering::Acquire), 0);
        assert_eq!(Arc::strong_count(&voices[0]), 1, "a worker runs on");
    }

    /// A crew holds workers only while POLY is on (decisions.md R21): none asked for while it
    /// is off; switched on, asked for once and taken once the helper thread has started
    /// them (the caller plays every voice itself until then); switched off, let go and stopped
    /// only when the helper thread serves, their workers then back in the budget. Each
    /// step here is the audio thread's or the helper thread's, run in turn.
    #[test]
    fn a_crew_holds_workers_only_while_poly_is_on() {
        static THREE: Budget = Budget::new(|_| 3);
        let voices = fakes(3);
        let crew = Crew::with_budget(&THREE);
        let plan = crew.plan(&voices, 2, Some(Duration::from_millis(5)), None);
        let (mut pool, mut ask) = (None, Ask::Not);
        assert!(!crew.follow(None, plan, &mut pool, &mut ask));
        crew.serve();
        assert_eq!(THREE.held(), 0);
        assert!(
            crew.follow(POLY, plan, &mut pool, &mut ask),
            "not asked for"
        );
        assert!(
            !crew.follow(POLY, plan, &mut pool, &mut ask),
            "asked for twice"
        );
        assert!(pool.is_none());
        crew.serve();
        assert_eq!(THREE.held(), 2);
        assert!(!crew.follow(POLY, plan, &mut pool, &mut ask));
        assert_eq!(pool.as_ref().map(Pool::workers), Some(2));
        let played =
            pool.as_mut()
                .unwrap()
                .play(&[0, 1, 2], 16, &EXT, 48_000.0, &mix(), true, MAX, None);
        assert_eq!(&played[..3], &[true; 3]);
        assert!(crew.follow(None, plan, &mut pool, &mut ask), "not let go");
        assert!(pool.is_none());
        assert_eq!(THREE.held(), 2, "stopped on the audio thread");
        crew.serve();
        assert_eq!(THREE.held(), 0);
        // (The voices held now by the test and the plan alone: no worker runs on.)
        assert_eq!(Arc::strong_count(&voices[0]), 2);
    }

    /// POLY switched on and off again quickly: an ask withdrawn before it is served starts
    /// nothing; a pool ready for an ask withdrawn is stopped when next served, or taken if POLY
    /// is switched on again first; a pool still being started when the ask is withdrawn is
    /// stopped as it comes. No worker is kept from the budget.
    #[test]
    fn poly_switched_quickly_keeps_no_worker() {
        static FOUR: Budget = Budget::new(|_| 4);
        let voices = fakes(3);
        let crew = Crew::with_budget(&FOUR);
        let plan = crew.plan(&voices, 3, Some(Duration::from_millis(5)), None);
        let (mut pool, mut ask) = (None, Ask::Not);
        let mut follow = |on: bool, pool: &mut Option<Pool<Fake>>| {
            crew.follow(on.then_some(POLY_NEED), plan, pool, &mut ask)
        };
        // On and off before the helper thread serves.
        assert!(follow(true, &mut pool));
        follow(false, &mut pool);
        crew.serve();
        assert_eq!(FOUR.held(), 0);
        // Served, then off before the pool is taken: stopped at the next serve.
        assert!(follow(true, &mut pool));
        crew.serve();
        assert_eq!(FOUR.held(), 3);
        assert!(follow(false, &mut pool), "the pool ready left running");
        assert!(pool.is_none());
        crew.serve();
        assert_eq!(FOUR.held(), 0);
        // Served, off, and on again before the next serve: the pool ready taken after all.
        assert!(follow(true, &mut pool));
        crew.serve();
        follow(false, &mut pool);
        follow(true, &mut pool);
        assert_eq!(pool.as_ref().map(Pool::workers), Some(3));
        crew.serve();
        assert_eq!(FOUR.held(), 3);
        assert!(follow(false, &mut pool));
        crew.serve();
        assert_eq!(FOUR.held(), 0);
        // Off while the helper thread is starting the pool: stopped as it comes.
        assert!(follow(true, &mut pool));
        let (asked, need) = crew.tidy().unwrap();
        let started = crew.start(asked, need);
        assert_eq!(FOUR.held(), 3);
        follow(false, &mut pool);
        crew.deliver(asked, started);
        assert_eq!(FOUR.held(), 0);
        follow(true, &mut pool);
        assert!(pool.is_none(), "a pool taken for an ask withdrawn");
    }

    /// Instances share the budget evenly in the order they came, the remainder to the first
    /// (decisions.md R-THREADS); one leaving gives the others its share.
    #[test]
    fn seats_share_the_budget_evenly() {
        static TEN: Budget = Budget::new(|n| 12 - n);
        let a = TEN.sit();
        let share = |s: &Seat| s.share.load(Ordering::Acquire);
        assert_eq!(share(&a), 11);
        let b = TEN.sit();
        let c = TEN.sit();
        // (9 for three: 3 each.)
        assert_eq!([share(&a), share(&b), share(&c)], [3, 3, 3]);
        let d = TEN.sit();
        // (8 for four.)
        assert_eq!([share(&a), share(&b), share(&c), share(&d)], [2, 2, 2, 2]);
        drop(b);
        // (9 for three again, a and c and d.)
        assert_eq!([share(&a), share(&c), share(&d)], [3, 3, 3]);
        drop((a, c));
        assert_eq!(share(&d), 11);
    }

    /// A pool started at its instance's whole share is let go and asked for again when
    /// another instance takes a seat, and comes back at its new share; the other's pool, which
    /// got less while the first held more, is asked for again once some is free, until both
    /// hold their shares (decisions.md R-THREADS). Each step the audio thread's or a helper
    /// thread's, in turn.
    #[test]
    fn the_shares_follow_the_instances() {
        static SIX: Budget = Budget::new(|_| 6);
        let period = Some(Duration::from_millis(5));
        let (va, vb) = (fakes(8), fakes(8));
        let (a, b) = (Crew::with_budget(&SIX), Crew::with_budget(&SIX));
        let (pa, pb) = (a.plan(&va, 8, period, None), b.plan(&vb, 8, period, None));
        let (mut poola, mut aska) = (None, Ask::Not);
        let (mut poolb, mut askb) = (None, Ask::Not);
        assert!(a.follow(POLY, pa, &mut poola, &mut aska));
        a.serve();
        a.follow(POLY, pa, &mut poola, &mut aska);
        assert_eq!(
            poola.as_ref().map(Pool::workers),
            Some(6),
            "alone: the whole budget"
        );
        // The second instance asks: seated, its share 3, but only what is free (none).
        assert!(b.follow(POLY, pb, &mut poolb, &mut askb));
        b.serve();
        b.follow(POLY, pb, &mut poolb, &mut askb);
        assert_eq!(poolb.as_ref().map(Pool::workers), Some(0));
        // The first sees its share moved: let go, asked for again, back at 3.
        assert!(
            a.follow(POLY, pa, &mut poola, &mut aska),
            "not asked for again"
        );
        assert!(poola.is_none());
        a.serve();
        a.follow(POLY, pa, &mut poola, &mut aska);
        assert_eq!(poola.as_ref().map(Pool::workers), Some(3));
        // The second, short while some is free: asked for again, now its 3.
        assert!(
            b.follow(POLY, pb, &mut poolb, &mut askb),
            "the short one not asked again"
        );
        b.serve();
        b.follow(POLY, pb, &mut poolb, &mut askb);
        assert_eq!(poolb.as_ref().map(Pool::workers), Some(3));
        assert_eq!(SIX.held(), 6);
        // Settled: neither asks again.
        assert!(!a.follow(POLY, pa, &mut poola, &mut aska));
        assert!(!b.follow(POLY, pb, &mut poolb, &mut askb));
        // The first switched off: its seat given up; the second's share is all again.
        assert!(a.follow(None, pa, &mut poola, &mut aska));
        a.serve();
        assert!(b.follow(POLY, pb, &mut poolb, &mut askb));
        b.serve();
        b.follow(POLY, pb, &mut poolb, &mut askb);
        assert_eq!(poolb.as_ref().map(Pool::workers), Some(6));
        drop(poolb);
        assert_eq!(SIX.held(), 0);
    }

    /// The budget's shares follow the instances of other processes too (decisions.md
    /// R-THREADS): another process holding two seats, this one's lone instance shares with
    /// them, the first seat's the remainder; that process killed, its share is all again.
    #[test]
    fn the_shares_follow_other_processes_instances() {
        static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
        let dir = DIR.get_or_init(|| tempfile::tempdir().unwrap()).path();
        static TEN: Budget =
            Budget::with_files(|n| 12 - n, || DIR.get().map(|d| d.path().to_path_buf()));
        let share = |s: &Seat| s.share.load(Ordering::Acquire);
        let a = TEN.sit();
        assert_eq!((TEN.seated(), share(&a)), (1, 11));
        let mut child = crate::seats::tests::holder(dir, 2);
        TEN.refresh_now();
        // (9 for three, 3 each.)
        assert_eq!((TEN.seated(), share(&a)), (3, 3));
        let b = TEN.sit();
        // (8 for four: 2 each; this process's two among them.)
        assert_eq!((TEN.seated(), share(&a), share(&b)), (4, 2, 2));
        child.kill().unwrap();
        child.wait().unwrap();
        TEN.refresh_now();
        // (10 for two: 5 each.)
        assert_eq!((TEN.seated(), share(&a), share(&b)), (2, 5, 5));
        drop(b);
        assert_eq!((TEN.seated(), share(&a)), (1, 11));
    }

    /// A new plan (the voices built anew at a new rate, the plug-in deactivated) stops a pool
    /// ready or let go for the old one, and an ask for the old one starts nothing; a pool for
    /// the old plan that comes later is stopped. The audio thread asks again for the new.
    #[test]
    fn a_new_plan_stops_the_old_ones_pools() {
        static TWO: Budget = Budget::new(|_| 2);
        let (voices, others) = (fakes(2), fakes(2));
        let period = Some(Duration::from_millis(5));
        let crew = Crew::with_budget(&TWO);
        let old = crew.plan(&voices, 2, period, None);
        let (mut pool, mut ask) = (None, Ask::Not);
        assert!(crew.follow(POLY, old, &mut pool, &mut ask));
        crew.serve();
        assert_eq!(TWO.held(), 2);
        let new = crew.plan(&others, 2, period, None);
        assert_eq!(TWO.held(), 0, "the old plan's pool kept");
        // (The engine forgets its ask with its plan.)
        (pool, ask) = (None, Ask::Not);
        assert!(crew.follow(POLY, new, &mut pool, &mut ask));
        let (asked, need) = crew.tidy().unwrap();
        crew.clear();
        assert!(crew.start(asked, need).is_none(), "started for a plan gone");
        crew.deliver(asked, None);
        // A pool for the old plan that comes after the new one: stopped.
        let new = crew.plan(&others, 2, period, None);
        (pool, ask) = (None, Ask::Not);
        assert!(crew.follow(POLY, new, &mut pool, &mut ask));
        let (asked, need) = crew.tidy().unwrap();
        let late = crew.start(asked, need);
        let newer = crew.plan(&voices, 2, period, None);
        crew.deliver(asked, late);
        assert_eq!(TWO.held(), 0);
        (pool, ask) = (None, Ask::Not);
        assert!(crew.follow(POLY, newer, &mut pool, &mut ask));
        crew.serve();
        crew.follow(POLY, newer, &mut pool, &mut ask);
        assert_eq!(pool.as_ref().map(Pool::workers), Some(2));
        drop(pool);
        assert_eq!(TWO.held(), 0);
    }
}
