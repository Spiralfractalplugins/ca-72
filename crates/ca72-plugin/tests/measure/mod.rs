//! A measurement harness for the voice's laws: how each panel control sets the pitch, the
//! levels, the filter's cutoff and resonance and the contours' times, measured from a
//! model's output samples and its contours alone (no probe only the circuit has), so that
//! the circuit's voice and a cheaper model of it are measured alike and their results
//! compare (`potato_reference.rs` runs it on the circuit).
//!
//! Every measurement starts from [`panel`], the measurement panel, plays fresh models from a
//! [`Bench`], holds a key at least half a second before measuring anything steady and
//! measures over at least half a second. Frequencies come from rising crossings of the mean
//! ([`rising`]), levels from RMS and from Hann-windowed DFTs at the measured frequency
//! ([`tone`]), spectra from Welch's method ([`psd`]), and the laws from least squares
//! ([`line`], [`poly`], [`approach`]).

#![allow(dead_code, clippy::unwrap_used)]

use ca72::tuning::{FREQ_CENTRE, Range};
use ca72::voice::{Jacks, OscPanel, Panel, Quality, Voice, Waveform};
use ca72_analysis::fft::{hann, power};
use serde::Serialize;
use std::f64::consts::{FRAC_PI_2, PI};
use std::fmt::Write;

/// A voice as the measurements play it.
pub trait Model {
    /// The sample rate, Hz.
    fn rate(&self) -> f64;
    /// Sets every control.
    fn set_panel(&mut self, p: &Panel);
    /// A MIDI note (0 to 127) pressed or released.
    fn note(&mut self, midi: i32, on: bool);
    /// One output sample (1.0 = 5 V at the main output), `ext` at EXTERNAL INPUT (1.0 = 5 V).
    fn tick(&mut self, ext: f64) -> f64;
    /// The contours' latest values (filter, loudness), V.
    fn contours(&self) -> (f64, f64);
    /// The plug-in's DRIVE: the gain on the bus into the filter's input pair (1 the circuit).
    fn set_drive(&mut self, gain: f64);
    /// FEEDBACK: the share of the output sent back to EXTERNAL INPUT.
    fn set_feedback(&mut self, share: f64);
}

/// The circuit's voice, in Potato whatever the panel asks.
impl Model for Voice {
    fn rate(&self) -> f64 {
        Voice::rate(self)
    }

    fn set_panel(&mut self, p: &Panel) {
        self.panel = Panel {
            quality: Quality::Potato,
            ..*p
        };
    }

    fn note(&mut self, midi: i32, on: bool) {
        Voice::note(self, midi, on);
    }

    fn tick(&mut self, ext: f64) -> f64 {
        self.tick_jacks(&Jacks {
            ext,
            ..Jacks::default()
        })
    }

    fn contours(&self) -> (f64, f64) {
        self.probe_contours()
    }

    fn set_drive(&mut self, gain: f64) {
        Voice::set_drive(self, gain);
    }

    fn set_feedback(&mut self, share: f64) {
        self.feedback = share;
    }
}

/// How long a key is held before anything steady is measured, s.
pub const SETTLE: f64 = 0.5;
/// How long a steady measurement takes, s.
pub const SPAN: f64 = 1.0;
/// How long a fresh model rests on its panel before its first key, s.
pub const REST: f64 = 0.1;

/// The model under measurement: a factory of fresh models, each in the same state, and
/// their sample rate.
pub struct Bench<M> {
    new: Box<dyn Fn() -> M>,
    pub rate: f64,
}

impl<M> std::fmt::Debug for Bench<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Bench at {} Hz", self.rate)
    }
}

impl<M: Model> Bench<M> {
    pub fn new(new: impl Fn() -> M + 'static) -> Bench<M> {
        let rate = new().rate();
        Bench {
            new: Box::new(new),
            rate,
        }
    }

    /// A fresh model on `p`, rested [`REST`] with no key.
    pub fn fresh(&self, p: &Panel) -> M {
        let mut m = (self.new)();
        m.set_panel(p);
        run(&mut m, REST);
        m
    }

    /// A fresh model on `p` holding `key`: `settle` s after the press, `seconds` s of its
    /// output.
    pub fn held(&self, p: &Panel, key: i32, settle: f64, seconds: f64) -> Vec<f64> {
        let mut m = self.fresh(p);
        m.note(key, true);
        run(&mut m, settle);
        run(&mut m, seconds)
    }

    /// The frequency of [`Bench::held`]'s output after [`SETTLE`], over `seconds`, Hz.
    pub fn hz(&self, p: &Panel, key: i32, seconds: f64) -> f64 {
        frequency(&self.held(p, key, SETTLE, seconds), self.rate).unwrap_or(f64::NAN)
    }

    /// A fresh model on `p` holding `key`, its filter set ringing by a burst ([`kick`]) as
    /// the key goes down: `settle` s after the burst, `seconds` s of its output.
    pub fn kicked(&self, p: &Panel, key: i32, burst: f64, settle: f64, seconds: f64) -> Vec<f64> {
        let mut m = self.fresh(p);
        m.note(key, true);
        kick(&mut m, p, burst);
        run(&mut m, settle);
        run(&mut m, seconds)
    }
}

/// `seconds` of a model's output, nothing at the external input.
pub fn run<M: Model>(m: &mut M, seconds: f64) -> Vec<f64> {
    let n = (seconds * m.rate()).round() as usize;
    (0..n).map(|_| m.tick(0.0)).collect()
}

/// `seconds` of a model's contours (filter, loudness), a pair a sample.
pub fn run_contours<M: Model>(m: &mut M, seconds: f64) -> Vec<(f64, f64)> {
    let n = (seconds * m.rate()).round() as usize;
    (0..n)
        .map(|_| {
            m.tick(0.0);
            m.contours()
        })
        .collect()
}

/// Plays a model, appending its contours to `rec`, at least `min` s and until both contours
/// move less than `tol` V over 0.2 s, or `cap` s.
pub fn contours_until_settled<M: Model>(
    m: &mut M,
    rec: &mut Vec<(f64, f64)>,
    min: f64,
    cap: f64,
    tol: f64,
) {
    let chunk = (0.2 * m.rate()).round() as usize;
    let start = rec.len();
    loop {
        rec.extend(run_contours(m, 0.2));
        let done = rec.len() - start;
        let t = done as f64 / m.rate();
        if t >= cap {
            return;
        }
        if t >= min && done > chunk {
            let (a, b) = (rec[rec.len() - 1], rec[rec.len() - 1 - chunk]);
            if (a.0 - b.0).abs() < tol && (a.1 - b.1).abs() < tol {
                return;
            }
        }
    }
}

/// Sets the filter ringing: oscillator 1's sawtooth at 8' at VOLUME `volume` for 10 ms, then
/// the panel `p` again. A model without noise of its own does not leave its resting state
/// unprompted: the circuit's voice at EMPHASIS 10 with every source off stays silent until
/// something reaches the filter.
pub fn kick<M: Model>(m: &mut M, p: &Panel, volume: f64) {
    let mut k = *p;
    k.osc[0] = OscPanel {
        range: Range::R8,
        waveform: Waveform::Sawtooth,
        on: true,
        volume,
        freq: FREQ_CENTRE,
    };
    m.set_panel(&k);
    run(m, 0.01);
    m.set_panel(p);
}

/// The measurement panel: oscillator 1 alone on its sawtooth at 8', VOLUME 3; oscillators
/// 2 and 3 off; the filter open (CUTOFF +5, EMPHASIS 0, AMOUNT OF CONTOUR 0, both keyboard
/// controls off); the loudness contour ATTACK 0, DECAY 5, SUSTAIN 10; the filter contour
/// ATTACK 0, DECAY 5, SUSTAIN 0; DECAY on; GLIDE, the noise, the external input and the
/// A-440 off; TUNE and the wheels at 0; OSC. 3 CONTROL on.
pub fn panel() -> Panel {
    let mut p = Panel::default();
    p.osc[0] = osc(Waveform::Sawtooth, 0.3);
    p.osc[1].on = false;
    p.osc[2].on = false;
    p.cutoff = 1.0;
    p.emphasis = 0.0;
    p.contour_amount = 0.0;
    p.keyboard_control_1 = false;
    p.keyboard_control_2 = false;
    p.loudness_contour = ca72::voice::ContourKnobs {
        attack: 0.0,
        decay: 0.5,
        sustain: 1.0,
    };
    p.filter_contour = ca72::voice::ContourKnobs {
        attack: 0.0,
        decay: 0.5,
        sustain: 0.0,
    };
    p.decay = true;
    p.glide = 0.0;
    p.glide_on = false;
    p.noise_on = false;
    p.ext_on = false;
    p.a440 = false;
    p.tune = 0.0;
    p.pitch_wheel = 0.0;
    p.mod_wheel = 0.0;
    p.osc3_control = true;
    p
}

/// An oscillator on at 8' with `waveform` at `volume`, FREQUENCY centred.
pub fn osc(waveform: Waveform, volume: f64) -> OscPanel {
    OscPanel {
        range: Range::R8,
        waveform,
        on: true,
        volume,
        freq: FREQ_CENTRE,
    }
}

/// The measurement panel with every source off (oscillator 1 too).
pub fn silent() -> Panel {
    let mut p = panel();
    p.osc[0].on = false;
    p
}

/// A range's name as the panel prints it.
pub fn range_name(r: Range) -> &'static str {
    match r {
        Range::Lo => "LO",
        Range::R32 => "32'",
        Range::R16 => "16'",
        Range::R8 => "8'",
        Range::R4 => "4'",
        Range::R2 => "2'",
    }
}

// ---------------------------------------------------------------------------------------
// Signals.

pub fn mean(x: &[f64]) -> f64 {
    x.iter().sum::<f64>() / x.len().max(1) as f64
}

/// The RMS about the mean.
pub fn rms(x: &[f64]) -> f64 {
    let m = mean(x);
    (x.iter().map(|v| (v - m) * (v - m)).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

pub fn peak_to_peak(x: &[f64]) -> f64 {
    let (lo, hi) = x
        .iter()
        .fold((f64::MAX, f64::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    hi - lo
}

/// The rising crossings of the mean of `ys` sampled at times `ts`, interpolated, each counted
/// only once `ys` has been below the mean by a fifth of its RMS since the last (so that
/// ripple near the mean does not count twice).
pub fn rising_at(ts: &[f64], ys: &[f64]) -> Vec<f64> {
    let m = mean(ys);
    let h = 0.2 * rms(ys);
    let mut armed = false;
    let mut out = Vec::new();
    for i in 1..ys.len() {
        if ys[i - 1] < m - h {
            armed = true;
        }
        if armed && ys[i - 1] < m && ys[i] >= m {
            let a = (m - ys[i - 1]) / (ys[i] - ys[i - 1]);
            out.push(ts[i - 1] + a * (ts[i] - ts[i - 1]));
            armed = false;
        }
    }
    out
}

/// [`rising_at`] on samples: the crossings' times in samples.
pub fn rising(x: &[f64]) -> Vec<f64> {
    let ts: Vec<f64> = (0..x.len()).map(|i| i as f64).collect();
    rising_at(&ts, x)
}

/// The mean frequency of `x`, Hz: its rising crossings' count over their span (none with
/// fewer than three).
pub fn frequency(x: &[f64], rate: f64) -> Option<f64> {
    let r = rising(x);
    (r.len() >= 3).then(|| rate * (r.len() - 1) as f64 / (r[r.len() - 1] - r[0]))
}

/// Each period's frequency: its middle (s from the start of `x`) and Hz.
pub fn periods(x: &[f64], rate: f64) -> Vec<(f64, f64)> {
    rising(x)
        .windows(2)
        .map(|w| (0.5 * (w[0] + w[1]) / rate, rate / (w[1] - w[0])))
        .collect()
}

/// The component of `x` at `hz` through a Hann window: its amplitude, and its phase as a
/// cosine from `x`'s first sample.
pub fn tone(x: &[f64], rate: f64, hz: f64) -> (f64, f64) {
    let n = x.len();
    let m = mean(x);
    let w = 2.0 * PI * hz / rate;
    let (mut re, mut im, mut sum) = (0.0, 0.0, 0.0);
    for (i, v) in x.iter().enumerate() {
        let h = 0.5 - 0.5 * (2.0 * PI * i as f64 / n as f64).cos();
        let (s, c) = (w * i as f64).sin_cos();
        re += h * (v - m) * c;
        im -= h * (v - m) * s;
        sum += h;
    }
    (2.0 * re.hypot(im) / sum, im.atan2(re))
}

/// The amplitudes of harmonics 1 to `n` of `x`, whose fundamental is `hz`.
pub fn harmonics(x: &[f64], rate: f64, hz: f64, n: usize) -> Vec<f64> {
    (1..=n)
        .map(|k| {
            if k as f64 * hz < 0.5 * rate {
                tone(x, rate, k as f64 * hz).0
            } else {
                0.0
            }
        })
        .collect()
}

/// `x` at fractional sample `t`, linearly interpolated.
fn at(x: &[f64], t: f64) -> f64 {
    let i = (t.floor() as usize).min(x.len() - 2);
    let a = t - i as f64;
    x[i] + (x[i + 1] - x[i]) * a
}

/// One period of `x` (frequency `hz`) averaged over every whole period in it, at `points`
/// phases, the first at the fundamental's rising zero crossing (so that two models' waves
/// line up whatever their phases).
pub fn fold(x: &[f64], rate: f64, hz: f64, points: usize) -> Vec<f64> {
    let period = rate / hz;
    let (_, ph) = tone(x, rate, hz);
    // The fundamental is a cos(2 pi f t + ph): it rises through zero where the angle is
    // -pi/2.
    let t0 = ((-FRAC_PI_2 - ph) / (2.0 * PI) * period).rem_euclid(period);
    let whole = ((x.len() as f64 - 2.0 - t0) / period).floor().max(1.0) as usize;
    (0..points)
        .map(|k| {
            let s: f64 = (0..whole)
                .map(|m| at(x, t0 + (m as f64 + k as f64 / points as f64) * period))
                .sum();
            s / whole as f64
        })
        .collect()
}

/// Welch's power spectral density of `x`: Hann frames of `n` points overlapping by half,
/// one-sided, (output units)^2 per Hz, bins `0..=n/2` at `rate / n` Hz apart.
pub fn psd(x: &[f64], rate: f64, n: usize) -> Vec<f64> {
    let w = hann(n);
    let u: f64 = w.iter().map(|v| v * v).sum();
    let mut acc = vec![0.0; n / 2 + 1];
    let mut frames = 0usize;
    let mut s = 0;
    while s + n <= x.len() {
        let frame = &x[s..s + n];
        let m = mean(frame);
        let centred: Vec<f64> = frame.iter().map(|v| v - m).collect();
        for (a, p) in acc.iter_mut().zip(power(&centred, &w)) {
            *a += p;
        }
        frames += 1;
        s += n / 2;
    }
    acc.iter()
        .enumerate()
        .map(|(k, a)| {
            let one_sided = if k == 0 || k == n / 2 { 1.0 } else { 2.0 };
            a / frames.max(1) as f64 / (u * rate) * one_sided
        })
        .collect()
}

/// The mean of a [`psd`]'s bins whose frequencies lie in `[lo, hi)`, or the nearest bin to
/// the band's geometric centre when none does.
pub fn band(p: &[f64], rate: f64, lo: f64, hi: f64) -> f64 {
    let n = 2 * (p.len() - 1);
    let bin = rate / n as f64;
    let (a, b) = ((lo / bin).ceil() as usize, (hi / bin).ceil() as usize);
    let b = b.min(p.len());
    if b > a {
        mean(&p[a..b])
    } else {
        let k = ((lo * hi).sqrt() / bin).round() as usize;
        p[k.min(p.len() - 1)]
    }
}

/// The third-octave centres 1000 * 2^(k/3) Hz from 20 Hz to 16 kHz.
pub fn third_octaves() -> Vec<f64> {
    (-17..=12)
        .map(|k| 1000.0 * 2f64.powf(f64::from(k) / 3.0))
        .collect()
}

pub fn db(x: f64) -> f64 {
    20.0 * x.log10()
}

pub fn cents(f: f64, to: f64) -> f64 {
    1200.0 * (f / to).log2()
}

// ---------------------------------------------------------------------------------------
// Fits.

/// A straight line y = a + b x fitted by least squares, and its worst residual.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Line {
    pub a: f64,
    pub b: f64,
    pub worst: f64,
}

pub fn line(xs: &[f64], ys: &[f64]) -> Line {
    let p = poly(xs, ys, 1);
    Line {
        a: p.c[0],
        b: p.c[1],
        worst: p.worst,
    }
}

/// A polynomial fitted by least squares: its coefficients from the constant up, and its
/// worst residual.
#[derive(Debug, Clone, Serialize)]
pub struct Poly {
    pub c: Vec<f64>,
    pub worst: f64,
}

impl Poly {
    pub fn at(&self, x: f64) -> f64 {
        self.c.iter().rev().fold(0.0, |acc, c| acc * x + c)
    }
}

pub fn poly(xs: &[f64], ys: &[f64], degree: usize) -> Poly {
    let n = degree + 1;
    // The normal equations, solved by Gaussian elimination with partial pivoting.
    let mut a = vec![vec![0.0; n + 1]; n];
    for (&x, &y) in xs.iter().zip(ys) {
        for (i, row) in a.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().take(n).enumerate() {
                *cell += x.powi((i + j) as i32);
            }
            row[n] += y * x.powi(i as i32);
        }
    }
    for col in 0..n {
        let pivot = (col..n)
            .max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))
            .unwrap_or(col);
        a.swap(col, pivot);
        let top = a[col].clone();
        if top[col] == 0.0 {
            continue;
        }
        for (r, row) in a.iter_mut().enumerate() {
            if r != col {
                let f = row[col] / top[col];
                for (cell, t) in row.iter_mut().zip(&top).skip(col) {
                    *cell -= f * t;
                }
            }
        }
    }
    let c: Vec<f64> = (0..n).map(|i| a[i][n] / a[i][i]).collect();
    let mut p = Poly { c, worst: 0.0 };
    p.worst = xs
        .iter()
        .zip(ys)
        .map(|(&x, &y)| (p.at(x) - y).abs())
        .fold(0.0, f64::max);
    p
}

/// An exponential approach to a level: y = end - e^(c - t / tau) rising, end + e^(c - t /
/// tau) falling. The level it heads for, its time constant (s), `c`, and the worst residual
/// (y's units).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Approach {
    pub end: f64,
    pub tau: f64,
    pub c: f64,
    pub worst: f64,
    pub rising: bool,
}

impl Approach {
    pub fn at(&self, t: f64) -> f64 {
        let e = (self.c - t / self.tau).exp();
        if self.rising {
            self.end - e
        } else {
            self.end + e
        }
    }

    /// When the fitted curve passes `y`, s.
    pub fn when(&self, y: f64) -> f64 {
        (self.c - (self.end - y).abs().ln()) * self.tau
    }
}

/// [`Approach`] with the level it heads for given: a line through ln |end - y|.
pub fn approach_to(ts: &[f64], ys: &[f64], end: f64) -> Option<Approach> {
    let rising = ys.last()? > ys.first()?;
    let (t, z): (Vec<f64>, Vec<f64>) = ts
        .iter()
        .zip(ys)
        .filter(|&(_, &y)| (end - y).abs() > 0.0)
        .map(|(&t, &y)| (t, (end - y).abs().ln()))
        .unzip();
    if t.len() < 4 {
        return None;
    }
    let l = line(&t, &z);
    let mut a = Approach {
        end,
        tau: -1.0 / l.b,
        c: l.a,
        worst: 0.0,
        rising,
    };
    a.worst = ts
        .iter()
        .zip(ys)
        .map(|(&t, &y)| (a.at(t) - y).abs())
        .fold(0.0, f64::max);
    Some(a)
}

/// [`Approach`] fitted with the level it heads for found too: of the levels beyond the data
/// in the direction it moves, the one that leaves the least RMS residual.
pub fn approach(ts: &[f64], ys: &[f64]) -> Option<Approach> {
    if ys.len() < 6 {
        return None;
    }
    let rising = ys.last()? > ys.first()?;
    let (lo, hi) = ys
        .iter()
        .fold((f64::MAX, f64::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    let span = (hi - lo).max(1e-12);
    let end_at = |u: f64| {
        if rising {
            hi + span * 10f64.powf(u)
        } else {
            lo - span * 10f64.powf(u)
        }
    };
    let cost = |u: f64| -> f64 {
        approach_to(ts, ys, end_at(u)).map_or(f64::MAX, |a| {
            let s: f64 = ts
                .iter()
                .zip(ys)
                .map(|(&t, &y)| (a.at(t) - y).powi(2))
                .sum();
            s / ys.len() as f64
        })
    };
    // A coarse scan, then golden sections about its best.
    let grid: Vec<f64> = (0..=40).map(|k| -8.0 + 0.25 * f64::from(k)).collect();
    let best = grid
        .iter()
        .copied()
        .min_by(|&a, &b| cost(a).total_cmp(&cost(b)))
        .unwrap_or(0.0);
    let (mut a, mut b) = (best - 0.25, best + 0.25);
    let g = 0.5 * (5f64.sqrt() - 1.0);
    for _ in 0..40 {
        let (c, d) = (b - g * (b - a), a + g * (b - a));
        if cost(c) < cost(d) {
            b = d;
        } else {
            a = c;
        }
    }
    approach_to(ts, ys, end_at(0.5 * (a + b)))
}

/// At most `n` evenly spaced points of a record (for fits).
fn thin<T: Copy>(x: &[T], n: usize) -> Vec<T> {
    let step = x.len().div_ceil(n.max(1)).max(1);
    x.iter().copied().step_by(step).collect()
}

/// The knob at which a table of (knob, value) reaches `value`, linearly interpolated
/// (the table monotonic).
pub fn knob_for(table: &[(f64, f64)], value: f64) -> f64 {
    for w in table.windows(2) {
        let ((k0, v0), (k1, v1)) = (w[0], w[1]);
        if (v0 - value) * (v1 - value) <= 0.0 && v1 != v0 {
            return k0 + (k1 - k0) * (value - v0) / (v1 - v0);
        }
    }
    f64::NAN
}

// ---------------------------------------------------------------------------------------
// A. Pitch.

/// A frequency at a key or a knob, and its offset in cents (from the fitted law, or from
/// oscillator 1 alone at 8', as [`Pitch`] says).
#[derive(Debug, Clone, Serialize)]
pub struct At {
    pub x: f64,
    pub hz: f64,
    pub cents: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RangeHz {
    pub range: &'static str,
    pub hz: f64,
    /// Against 8'.
    pub ratio: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Free {
    pub range: &'static str,
    pub key: i32,
    pub hz: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Pitch {
    /// Oscillator 1 on its sawtooth at 8': (MIDI key, Hz, cents off the fitted law).
    pub keys: Vec<At>,
    /// The law log2 Hz = a + b key fitted over them: Hz at key 69, semitones a key, the key
    /// at 440 Hz, and the worst residual, cents.
    pub hz_at_69: f64,
    pub semitones_per_key: f64,
    pub key_at_440: f64,
    pub law_worst_cents: f64,
    /// RANGE at key 60, against 8'.
    pub ranges: Vec<RangeHz>,
    /// Oscillators 2 and 3 alone at 8', key 60: FREQUENCY 0 to 1, cents from oscillator 1
    /// alone; each fitted by a cubic in the knob (cents).
    pub osc2: Vec<At>,
    pub osc2_law: Poly,
    pub osc3: Vec<At>,
    pub osc3_law: Poly,
    /// TUNE and the pitch wheel -1 to 1 (oscillator 1, key 60): cents from 0; fitted by
    /// cubics.
    pub tune: Vec<At>,
    pub tune_law: Poly,
    pub wheel: Vec<At>,
    pub wheel_law: Poly,
    /// OSC. 3 CONTROL off: oscillator 3 alone, FREQUENCY at its centre.
    pub osc3_free: Vec<Free>,
    /// OSC. 3 CONTROL off, key 60: oscillator 3 alone at 8', FREQUENCY 0 to 1, cents from
    /// oscillator 1 alone at key 60.
    pub osc3_free_freq: Vec<At>,
}

pub fn pitch<M: Model>(b: &Bench<M>) -> Pitch {
    let base = panel();
    let keys: Vec<i32> = (0..8).map(|k| 24 + 12 * k).collect();
    let hz: Vec<f64> = keys.iter().map(|&k| b.hz(&base, k, SPAN)).collect();
    let kx: Vec<f64> = keys.iter().map(|&k| f64::from(k)).collect();
    let l2: Vec<f64> = hz.iter().map(|f| f.log2()).collect();
    let law = line(&kx, &l2);
    let keys = keys
        .iter()
        .zip(&hz)
        .map(|(&k, &f)| At {
            x: f64::from(k),
            hz: f,
            cents: 1200.0 * (f.log2() - (law.a + law.b * f64::from(k))),
        })
        .collect();
    let ranges_hz: Vec<(Range, f64)> = [
        Range::Lo,
        Range::R32,
        Range::R16,
        Range::R8,
        Range::R4,
        Range::R2,
    ]
    .iter()
    .map(|&r| {
        let mut p = base;
        p.osc[0].range = r;
        (r, b.hz(&p, 60, if r == Range::Lo { 8.0 } else { SPAN }))
    })
    .collect();
    let f1 = ranges_hz[3].1;
    let ranges = ranges_hz
        .iter()
        .map(|&(r, f)| RangeHz {
            range: range_name(r),
            hz: f,
            ratio: f / f1,
        })
        .collect();
    let knobs: Vec<f64> = (0..=10).map(|k| f64::from(k) / 10.0).collect();
    let freq = |n: usize| -> Vec<At> {
        knobs
            .iter()
            .map(|&k| {
                let mut p = base;
                p.osc[0].on = false;
                p.osc[n] = OscPanel {
                    freq: k,
                    ..osc(Waveform::Sawtooth, 0.3)
                };
                let f = b.hz(&p, 60, SPAN);
                At {
                    x: k,
                    hz: f,
                    cents: cents(f, f1),
                }
            })
            .collect()
    };
    let swept = |set: &dyn Fn(&mut Panel, f64)| -> Vec<At> {
        [-1.0, -0.5, 0.0, 0.5, 1.0]
            .iter()
            .map(|&k| {
                let mut p = base;
                set(&mut p, k);
                let f = if k == 0.0 { f1 } else { b.hz(&p, 60, SPAN) };
                At {
                    x: k,
                    hz: f,
                    cents: cents(f, f1),
                }
            })
            .collect()
    };
    let fit = |t: &[At]| {
        let (x, y): (Vec<f64>, Vec<f64>) = t.iter().map(|a| (a.x, a.cents)).unzip();
        poly(&x, &y, 3)
    };
    let (osc2, osc3) = (freq(1), freq(2));
    let tune = swept(&|p, k| p.tune = k);
    let wheel = swept(&|p, k| p.pitch_wheel = k);
    let mut osc3_free = Vec::new();
    for r in [Range::R8, Range::Lo] {
        for key in [36, 60, 84] {
            let mut p = base;
            p.osc[0].on = false;
            p.osc[2] = OscPanel {
                range: r,
                ..osc(Waveform::Sawtooth, 0.3)
            };
            p.osc3_control = false;
            osc3_free.push(Free {
                range: range_name(r),
                key,
                hz: b.hz(&p, key, if r == Range::Lo { 8.0 } else { SPAN }),
            });
        }
    }
    let osc3_free_freq = knobs
        .iter()
        .map(|&k| {
            let mut p = base;
            p.osc[0].on = false;
            p.osc[2] = OscPanel {
                freq: k,
                ..osc(Waveform::Sawtooth, 0.3)
            };
            p.osc3_control = false;
            let f = b.hz(&p, 60, SPAN);
            At {
                x: k,
                hz: f,
                cents: cents(f, f1),
            }
        })
        .collect();
    Pitch {
        keys,
        osc3_free_freq,
        hz_at_69: 2f64.powf(law.a + 69.0 * law.b),
        semitones_per_key: 12.0 * law.b,
        key_at_440: (440f64.log2() - law.a) / law.b,
        law_worst_cents: 1200.0 * law.worst,
        ranges,
        osc2_law: fit(&osc2),
        osc3_law: fit(&osc3),
        tune_law: fit(&tune),
        wheel_law: fit(&wheel),
        osc2,
        osc3,
        tune,
        wheel,
        osc3_free,
    }
}

// ---------------------------------------------------------------------------------------
// B. Waveforms.

#[derive(Debug, Clone, Serialize)]
pub struct Wave {
    pub name: String,
    pub hz: f64,
    /// One period at 64 phases from the fundamental's rising zero crossing, less its mean,
    /// over its peak-to-peak.
    pub shape: Vec<f64>,
    /// Peak-to-peak, RMS about the mean, and the mean, output units.
    pub pp: f64,
    pub rms: f64,
    pub mean: f64,
    /// Rectangles: the share of the period above the midpoint between its extremes.
    pub duty: Option<f64>,
    /// Sawtooths: "rising" or "falling", the way the slow ramp goes.
    pub ramp: Option<&'static str>,
    /// The fundamental's amplitude, output units, and harmonics 2 to 16 against it, dB.
    pub fundamental: f64,
    pub harmonics_db: Vec<f64>,
    /// Harmonics 2 to 16's phases against the fundamental's, phi_k - k phi_1 (each a
    /// cosine's phase), wrapped to -180..180 degrees: with the levels, the wave through the
    /// output's coupling.
    pub phases_deg: Vec<f64>,
}

/// The open filter's and the output's flatness: oscillator 1's sawtooth at VOLUME 2, at 32'
/// (keys 21, 33, 45: 6.9 to 27.5 Hz) and 8' (keys 21 to 81: 27.5 to 880 Hz): the
/// fundamental's level and the RMS against 8' key 69's, dB.
#[derive(Debug, Clone, Serialize)]
pub struct Flat {
    pub range: &'static str,
    pub key: i32,
    pub hz: f64,
    pub fundamental_db: f64,
    pub rms_db: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Waveforms {
    pub waves: Vec<Wave>,
    pub flat: Vec<Flat>,
}

/// One waveform held at `key` on panel `p`.
fn wave<M: Model>(b: &Bench<M>, p: &Panel, key: i32, name: String, w: Waveform) -> Wave {
    let x = b.held(p, key, SETTLE, 2.0);
    let hz = frequency(&x, b.rate).unwrap_or(f64::NAN);
    let fine = fold(&x, b.rate, hz, 1024);
    let m = mean(&fine);
    let pp = peak_to_peak(&fine);
    let shape = fold(&x, b.rate, hz, 64)
        .iter()
        .map(|v| (v - m) / pp)
        .collect();
    let (lo, hi) = fine
        .iter()
        .fold((f64::MAX, f64::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    let mid = 0.5 * (lo + hi);
    let duty = matches!(
        w,
        Waveform::Square | Waveform::WideRectangle | Waveform::NarrowRectangle
    )
    .then(|| fine.iter().filter(|&&v| v > mid).count() as f64 / fine.len() as f64);
    let ramp = matches!(w, Waveform::Sawtooth | Waveform::ReverseSawtooth).then(|| {
        let ups = fine.windows(2).filter(|d| d[1] > d[0]).count() as f64;
        if ups > 0.5 * fine.len() as f64 {
            "rising"
        } else {
            "falling"
        }
    });
    let h = harmonics(&x, b.rate, hz, 16);
    let phi1 = tone(&x, b.rate, hz).1;
    let phases_deg = (2..=16)
        .map(|k| {
            let d = tone(&x, b.rate, f64::from(k) * hz).1 - f64::from(k) * phi1;
            (d + PI).rem_euclid(2.0 * PI).to_degrees() - 180.0
        })
        .collect();
    Wave {
        name,
        hz,
        shape,
        pp,
        rms: rms(&x),
        mean: mean(&x),
        duty,
        ramp,
        fundamental: h[0],
        harmonics_db: h[1..].iter().map(|a| db(a / h[0])).collect(),
        phases_deg,
    }
}

pub fn waveforms<M: Model>(b: &Bench<M>) -> Waveforms {
    let mut waves = Vec::new();
    for w in [
        Waveform::Triangle,
        Waveform::SharkTooth,
        Waveform::Sawtooth,
        Waveform::Square,
        Waveform::WideRectangle,
        Waveform::NarrowRectangle,
    ] {
        let mut p = panel();
        p.osc[0] = osc(w, 0.2);
        waves.push(wave(b, &p, 33, format!("oscillator 1 {w:?}"), w));
    }
    let mut p = silent();
    p.osc[2] = osc(Waveform::ReverseSawtooth, 0.2);
    waves.push(wave(
        b,
        &p,
        33,
        "oscillator 3 ReverseSawtooth".into(),
        Waveform::ReverseSawtooth,
    ));
    let levels: Vec<(Range, i32, f64, f64, f64)> = [
        (Range::R32, 21),
        (Range::R32, 33),
        (Range::R32, 45),
        (Range::R8, 21),
        (Range::R8, 33),
        (Range::R8, 45),
        (Range::R8, 57),
        (Range::R8, 69),
        (Range::R8, 81),
    ]
    .iter()
    .map(|&(r, k)| {
        let mut p = panel();
        p.osc[0] = OscPanel {
            range: r,
            ..osc(Waveform::Sawtooth, 0.2)
        };
        let x = b.held(&p, k, SETTLE, 3.0);
        let hz = frequency(&x, b.rate).unwrap_or(f64::NAN);
        (r, k, hz, tone(&x, b.rate, hz).0, rms(&x))
    })
    .collect();
    let (_, _, _, f69, r69) = levels[7];
    let flat = levels
        .iter()
        .map(|&(r, key, hz, f, v)| Flat {
            range: range_name(r),
            key,
            hz,
            fundamental_db: db(f / f69),
            rms_db: db(v / r69),
        })
        .collect();
    Waveforms { waves, flat }
}

// ---------------------------------------------------------------------------------------
// C. Mixer and drive.

/// A level at a knob: output RMS, and the fitted law's error there, %.
#[derive(Debug, Clone, Serialize)]
pub struct Level {
    pub knob: f64,
    pub rms: f64,
    pub error_pct: f64,
}

/// A mixer channel's law: rms = v / (c0 + c1 v + c2 v^2) at VOLUME v, the form of a linear
/// pot between a source and ground whose wiper feeds a series resistor (v / rms is a
/// quadratic in v), fitted over VOLUME 0.1 to `fitted_to`; the worst error there, %.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ChannelLaw {
    pub c0: f64,
    pub c1: f64,
    pub c2: f64,
    pub fitted_to: f64,
    pub worst_pct: f64,
}

impl ChannelLaw {
    pub fn at(&self, v: f64) -> f64 {
        v / (self.c0 + self.c1 * v + self.c2 * v * v)
    }
}

/// Fits [`ChannelLaw`] to (VOLUME, RMS) over VOLUME 0.1 to `to`.
pub fn channel_law(points: &[(f64, f64)], to: f64) -> ChannelLaw {
    let used: Vec<(f64, f64)> = points
        .iter()
        .copied()
        .filter(|&(v, r)| v >= 0.1 - 1e-9 && v <= to + 1e-9 && r > 0.0)
        .collect();
    let (x, y): (Vec<f64>, Vec<f64>) = used.iter().map(|&(v, r)| (v, v / r)).unzip();
    let q = poly(&x, &y, 2);
    let mut law = ChannelLaw {
        c0: q.c[0],
        c1: q.c[1],
        c2: q.c[2],
        fitted_to: to,
        worst_pct: 0.0,
    };
    law.worst_pct = used
        .iter()
        .map(|&(v, r)| 100.0 * (law.at(v) / r - 1.0).abs())
        .fold(0.0, f64::max);
    law
}

/// Oscillators summed at equal VOLUME: the output's RMS, what each alone gives summed as
/// uncorrelated signals (the linear sum), and the difference, dB.
#[derive(Debug, Clone, Serialize)]
pub struct Stack {
    pub oscillators: usize,
    pub volume: f64,
    pub rms: f64,
    pub linear: f64,
    pub compression_db: f64,
}

/// Oscillator 1's triangle at 110 Hz: the fundamental's amplitude and harmonics 2 and 3
/// against it, dB.
#[derive(Debug, Clone, Serialize)]
pub struct Distortion {
    pub volume: f64,
    pub fundamental: f64,
    pub h2_db: f64,
    pub h3_db: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Mixer {
    /// Oscillator 1's sawtooth at key 45 (110 Hz), VOLUME 0 to 1.
    pub volume: Vec<Level>,
    pub law: ChannelLaw,
    /// The same law fitted over the whole travel.
    pub law_whole: ChannelLaw,
    /// Oscillators 2 and 3 alone at the stacks' volumes (their FREQUENCY as given): RMS.
    pub osc2_alone: Vec<(f64, f64)>,
    pub osc3_alone: Vec<(f64, f64)>,
    /// The FREQUENCY knobs the stacks used, and the intervals they give, cents.
    pub osc2_knob: f64,
    pub osc3_knob: f64,
    pub stacks: Vec<Stack>,
    pub triangle: Vec<Distortion>,
}

/// The mixer: oscillator 2 at FREQUENCY `osc2_knob` and oscillator 3 at `osc3_knob` (an
/// interval each, chosen so that no low harmonics of the three coincide: a fifth's would beat
/// slowly and move the RMS).
pub fn mixer<M: Model>(b: &Bench<M>, osc2_knob: f64, osc3_knob: f64) -> Mixer {
    let key = 45;
    let alone = |n: usize, v: f64| -> f64 {
        let mut p = silent();
        p.osc[n] = OscPanel {
            freq: [FREQ_CENTRE, osc2_knob, osc3_knob][n],
            ..osc(Waveform::Sawtooth, v)
        };
        rms(&b.held(&p, key, SETTLE, 2.0))
    };
    let points: Vec<(f64, f64)> = (0..=10)
        .map(|k| {
            let v = f64::from(k) / 10.0;
            (v, alone(0, v))
        })
        .collect();
    let law = channel_law(&points, 0.5);
    let law_whole = channel_law(&points, 1.0);
    let volume = points
        .iter()
        .map(|&(v, r)| Level {
            knob: v,
            rms: r,
            error_pct: if r > 0.0 {
                100.0 * (law.at(v) / r - 1.0)
            } else {
                0.0
            },
        })
        .collect();
    let vols: Vec<f64> = (1..=5).map(|k| 0.2 * f64::from(k)).collect();
    let osc2_alone: Vec<(f64, f64)> = vols.iter().map(|&v| (v, alone(1, v))).collect();
    let osc3_alone: Vec<(f64, f64)> = vols.iter().map(|&v| (v, alone(2, v))).collect();
    let mut stacks = Vec::new();
    for n in [2usize, 3] {
        for (i, &v) in vols.iter().enumerate() {
            let mut p = silent();
            for (j, f) in [FREQ_CENTRE, osc2_knob, osc3_knob]
                .iter()
                .enumerate()
                .take(n)
            {
                p.osc[j] = OscPanel {
                    freq: *f,
                    ..osc(Waveform::Sawtooth, v)
                };
            }
            let r = rms(&b.held(&p, key, SETTLE, 2.0));
            let one = points[(v * 10.0).round() as usize].1;
            let mut lin = one * one + osc2_alone[i].1 * osc2_alone[i].1;
            if n == 3 {
                lin += osc3_alone[i].1 * osc3_alone[i].1;
            }
            let lin = lin.sqrt();
            stacks.push(Stack {
                oscillators: n,
                volume: v,
                rms: r,
                linear: lin,
                compression_db: db(r / lin),
            });
        }
    }
    let triangle = (1..=10)
        .map(|k| {
            let v = f64::from(k) / 10.0;
            let mut p = panel();
            p.osc[0] = osc(Waveform::Triangle, v);
            let x = b.held(&p, key, SETTLE, SPAN);
            let hz = frequency(&x, b.rate).unwrap_or(f64::NAN);
            let h = harmonics(&x, b.rate, hz, 3);
            Distortion {
                volume: v,
                fundamental: h[0],
                h2_db: db(h[1] / h[0]),
                h3_db: db(h[2] / h[0]),
            }
        })
        .collect();
    Mixer {
        volume,
        law,
        law_whole,
        osc2_alone,
        osc3_alone,
        osc2_knob,
        osc3_knob,
        stacks,
        triangle,
    }
}

// ---------------------------------------------------------------------------------------
// D. Filter.

/// The self-oscillation at a CUTOFF: its frequency (none if it does not oscillate), its RMS
/// and its peak (half the peak-to-peak), output units.
#[derive(Debug, Clone, Serialize)]
pub struct Ring {
    pub cutoff: f64,
    pub hz: Option<f64>,
    pub rms: f64,
    pub peak: f64,
    /// Its third harmonic against its fundamental, dB.
    pub h3_db: Option<f64>,
}

/// KEYBOARD CONTROL 1 and 2 at CUTOFF 0.5: the self-oscillation at keys 36, 60 and 84, the
/// octaves it moves an octave of keys (a line through the three), and where that line puts
/// the untracked frequency (CUTOFF 0.5's ring), as a MIDI key.
#[derive(Debug, Clone, Serialize)]
pub struct Track {
    pub kbd1: bool,
    pub kbd2: bool,
    pub hz: Vec<(i32, f64)>,
    pub octaves_per_octave: f64,
    pub untracked_at_key: f64,
}

/// One EMPHASIS of a threshold scan: the RMS 2.0 to 2.5 s and 2.5 to 3.0 s after a small
/// burst, and whether the ringing grows or holds at a level (it oscillates on its own).
#[derive(Debug, Clone, Serialize)]
pub struct Growth {
    pub emphasis: f64,
    pub rms_early: f64,
    pub rms_late: f64,
    pub oscillates: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Threshold {
    pub cutoff: f64,
    /// The lowest EMPHASIS on the grid (steps of 0.025) from which every step up
    /// oscillates, and the threshold found by bisection below it (to 0.001).
    pub emphasis: Option<f64>,
    pub fine: Option<f64>,
    pub steps: Vec<Growth>,
}

/// AMOUNT OF CONTOUR with the filter contour held at a SUSTAIN: the contour's voltage, the
/// self-oscillation, the shift from AMOUNT 0 in octaves, the CUTOFF that would ring at that
/// frequency on its own (read from the rings, so the self-oscillation's own curvature
/// drops out), and that CUTOFF's move per contour volt per unit of AMOUNT.
#[derive(Debug, Clone, Serialize)]
pub struct Amount {
    pub sustain: f64,
    pub amount: f64,
    pub contour: f64,
    pub hz: Option<f64>,
    pub octaves: Option<f64>,
    pub cutoff_equivalent: Option<f64>,
    pub cutoff_per_volt: Option<f64>,
}

/// The response to white noise at a CUTOFF and EMPHASIS, against the open filter's (CUTOFF
/// 1.0, EMPHASIS 0). Its passband is the mean from a sixteenth to a quarter of EMPHASIS 0's
/// -3 dB corner (from 10 Hz), and its bass the mean from 10 to 25 Hz, each against EMPHASIS
/// 0's over the same band; the corner is where the curve first falls 3 dB below its own
/// passband; the peak its highest point and its height above its own passband; the slope a
/// line from 4 to 16 times EMPHASIS 0's corner (to 12 kHz); the curve at third-octave
/// centres, dB against EMPHASIS 0's passband. `oscillates`: at or above the threshold
/// found at this CUTOFF, where the noise only colours a self-oscillation.
#[derive(Debug, Clone, Serialize)]
pub struct Response {
    pub cutoff: f64,
    pub emphasis: f64,
    pub oscillates: bool,
    pub passband_db: f64,
    pub bass_db: f64,
    pub corner_hz: Option<f64>,
    pub peak_hz: Option<f64>,
    pub peak_db: Option<f64>,
    pub slope_db_per_octave: Option<f64>,
    pub curve: Vec<(f64, f64)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Filter {
    /// EMPHASIS 10, every source off, keyboard controls off, CUTOFF 0 to 1.
    pub rings: Vec<Ring>,
    /// The lowest CUTOFF that rings.
    pub lowest: Option<f64>,
    /// log2 Hz = a + b * CUTOFF over the rings from 200 Hz to 8 kHz: Hz at CUTOFF 0.5,
    /// octaves over the knob's travel and the worst residual, cents; and a cubic in CUTOFF
    /// for log2 Hz over every ring.
    pub hz_at_half: f64,
    pub octaves_per_knob: f64,
    pub law_worst_cents: f64,
    pub law_cubic: Poly,
    pub tracking: Vec<Track>,
    pub thresholds: Vec<Threshold>,
    /// The CUTOFF the AMOUNT OF CONTOUR series start from.
    pub amount_cutoff: f64,
    pub amount: Vec<Amount>,
    /// The open filter's response to the noise (CUTOFF 1.0, EMPHASIS 0) at third-octave
    /// centres, dB against its mean from 100 Hz to 1 kHz (the noise's own spectrum and the
    /// open filter together).
    pub open: Vec<(f64, f64)>,
    pub response: Vec<Response>,
}

/// The self-oscillation on panel `p` (`key` held), after a burst: its frequency, RMS, peak
/// and third harmonic, 1 s after the burst, over 1 s.
fn ring<M: Model>(b: &Bench<M>, p: &Panel, key: i32) -> Ring {
    let x = b.kicked(p, key, 0.3, 1.0, SPAN);
    let r = rms(&x);
    let hz = if r > 1e-4 {
        frequency(&x, b.rate)
    } else {
        None
    };
    Ring {
        cutoff: p.cutoff,
        hz,
        rms: r,
        peak: 0.5 * peak_to_peak(&x),
        h3_db: hz.filter(|f| 3.0 * f < 0.5 * b.rate).map(|f| {
            let h = harmonics(&x, b.rate, f, 3);
            db(h[2] / h[0])
        }),
    }
}

/// The CUTOFF at which the rings would sound `hz` (log-linear between them).
fn cutoff_for(rings: &[Ring], hz: f64) -> Option<f64> {
    let table: Vec<(f64, f64)> = rings
        .iter()
        .filter_map(|r| r.hz.map(|f| (r.cutoff, f.log2())))
        .collect();
    let k = knob_for(&table, hz.log2());
    k.is_finite().then_some(k)
}

/// Whether the filter on `p` oscillates on its own: a small burst (oscillator 1 at VOLUME
/// 0.001 for 10 ms), then `settle` s; the ringing over the next second grows, or stands at
/// a level no burst that small leaves after seconds.
fn oscillates<M: Model>(b: &Bench<M>, p: &Panel, settle: f64) -> Growth {
    let x = b.kicked(p, 60, 0.001, settle, 1.0);
    let half = x.len() / 2;
    let (e, l) = (rms(&x[..half]), rms(&x[half..]));
    Growth {
        emphasis: p.emphasis,
        rms_early: e,
        rms_late: l,
        oscillates: l > 1e-3 || (l > e && l > 1e-6),
    }
}

/// The response curve on a fine grid (1/24 octave from 10 Hz to 20 kHz) from two spectra:
/// each point the ratio of the bins within a 48th of an octave (or the nearest), dB.
fn ratio_curve(p: &[f64], reference: &[f64], rate: f64) -> Vec<(f64, f64)> {
    (0..=264)
        .map(|k| {
            let f = 10.0 * 2f64.powf(f64::from(k) / 24.0);
            let (lo, hi) = (f / 2f64.powf(1.0 / 48.0), f * 2f64.powf(1.0 / 48.0));
            (
                f,
                10.0 * (band(p, rate, lo, hi) / band(reference, rate, lo, hi)).log10(),
            )
        })
        .collect()
}

/// The ratio of two spectra over `[lo, hi)`, dB.
fn ratio_db(p: &[f64], reference: &[f64], rate: f64, lo: f64, hi: f64) -> f64 {
    10.0 * (band(p, rate, lo, hi) / band(reference, rate, lo, hi)).log10()
}

pub fn filter<M: Model>(b: &Bench<M>) -> Filter {
    let mut res = silent();
    res.emphasis = 1.0;
    let rings: Vec<Ring> = (0..=20)
        .map(|k| {
            let mut p = res;
            p.cutoff = f64::from(k) / 20.0;
            ring(b, &p, 60)
        })
        .collect();
    let lowest = rings.iter().find(|r| r.hz.is_some()).map(|r| r.cutoff);
    let ringing: Vec<(f64, f64)> = rings
        .iter()
        .filter_map(|r| r.hz.map(|f| (r.cutoff, f.log2())))
        .collect();
    let (x, y): (Vec<f64>, Vec<f64>) = ringing
        .iter()
        .copied()
        .filter(|&(_, l)| (200f64.log2()..=8e3f64.log2()).contains(&l))
        .unzip();
    let law = line(&x, &y);
    let (cx, cy): (Vec<f64>, Vec<f64>) = ringing.iter().copied().unzip();
    let law_cubic = poly(&cx, &cy, 3);
    let half = rings
        .iter()
        .find(|r| (r.cutoff - 0.5).abs() < 1e-9)
        .and_then(|r| r.hz);
    let mut tracking = Vec::new();
    for (kbd1, kbd2) in [(true, false), (false, true), (true, true)] {
        let mut p = res;
        p.cutoff = 0.5;
        p.keyboard_control_1 = kbd1;
        p.keyboard_control_2 = kbd2;
        let hz: Vec<(i32, f64)> = [36, 60, 84]
            .iter()
            .map(|&k| (k, ring(b, &p, k).hz.unwrap_or(f64::NAN)))
            .collect();
        let (x, y): (Vec<f64>, Vec<f64>) = hz
            .iter()
            .map(|&(k, f)| (f64::from(k) / 12.0, f.log2()))
            .unzip();
        let l = line(&x, &y);
        tracking.push(Track {
            kbd1,
            kbd2,
            hz,
            octaves_per_octave: l.b,
            untracked_at_key: half.map_or(f64::NAN, |f| 12.0 * (f.log2() - l.a) / l.b),
        });
    }
    let thresholds: Vec<Threshold> = [0.3, 0.5, 0.7]
        .iter()
        .map(|&c| {
            let at = |e: f64, settle: f64| {
                let mut p = silent();
                p.cutoff = c;
                p.emphasis = e;
                oscillates(b, &p, settle)
            };
            let steps: Vec<Growth> = (0..=24)
                .map(|k| at(0.4 + 0.025 * f64::from(k), 2.0))
                .collect();
            let first = match steps.iter().rposition(|g| !g.oscillates) {
                None => Some(0),
                Some(i) => (i + 1 < steps.len()).then_some(i + 1),
            };
            let fine = first.filter(|&i| i > 0).map(|i| {
                let (mut lo, mut hi) = (steps[i - 1].emphasis, steps[i].emphasis);
                while hi - lo > 0.001 {
                    let mid = 0.5 * (lo + hi);
                    if at(mid, 3.0).oscillates {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                0.5 * (lo + hi)
            });
            Threshold {
                cutoff: c,
                emphasis: first.map(|i| steps[i].emphasis),
                fine,
                steps,
            }
        })
        .collect();
    // AMOUNT OF CONTOUR: from a little above the lowest CUTOFF that rings, the filter
    // contour held at SUSTAIN 10 and 5.
    let amount_cutoff = lowest.map_or(0.2, |c| c + 0.05);
    let mut amount = Vec::new();
    for sustain in [1.0, 0.5] {
        let mut base_hz = None;
        let mut base_cut = None;
        for k in 0..=4 {
            let a = 0.25 * f64::from(k);
            let mut p = res;
            p.cutoff = amount_cutoff;
            p.contour_amount = a;
            p.filter_contour = ca72::voice::ContourKnobs {
                attack: 0.0,
                decay: 0.2,
                sustain,
            };
            let mut m = b.fresh(&p);
            m.note(60, true);
            kick(&mut m, &p, 0.3);
            run(&mut m, 2.0);
            let n = (SPAN * b.rate) as usize;
            let mut x = Vec::with_capacity(n);
            let mut v = 0.0;
            for _ in 0..n {
                x.push(m.tick(0.0));
                v += m.contours().0;
            }
            let v = v / n as f64;
            let hz = if rms(&x) > 1e-4 {
                frequency(&x, b.rate)
            } else {
                None
            };
            let cut = hz.and_then(|f| cutoff_for(&rings, f));
            if k == 0 {
                base_hz = hz;
                base_cut = cut;
            }
            amount.push(Amount {
                sustain,
                amount: a,
                contour: v,
                hz,
                octaves: hz.zip(base_hz).map(|(f, f0)| (f / f0).log2()),
                cutoff_equivalent: cut,
                cutoff_per_volt: cut
                    .zip(base_cut)
                    .filter(|_| a > 0.0)
                    .map(|(c, c0)| (c - c0) / (v * a)),
            });
        }
    }
    // The responses: white noise at VOLUME 5, 16 s each, against the open filter's.
    let n = 16384;
    let noise = |cutoff: f64, emphasis: f64| -> Vec<f64> {
        let mut p = silent();
        p.noise_on = true;
        p.noise_volume = 0.5;
        p.cutoff = cutoff;
        p.emphasis = emphasis;
        psd(&b.held(&p, 60, SETTLE, 16.0), b.rate, n)
    };
    let reference = noise(1.0, 0.0);
    let thirds = third_octaves();
    let third = |f: f64| (f / 2f64.powf(1.0 / 6.0), f * 2f64.powf(1.0 / 6.0));
    let ref_mid = 10.0 * band(&reference, b.rate, 100.0, 1000.0).log10();
    let open = thirds
        .iter()
        .map(|&f| {
            let (lo, hi) = third(f);
            (f, 10.0 * band(&reference, b.rate, lo, hi).log10() - ref_mid)
        })
        .collect();
    let mut response = Vec::new();
    for t in &thresholds {
        let c = t.cutoff;
        let flat = noise(c, 0.0);
        let curve0 = ratio_curve(&flat, &reference, b.rate);
        let below = |curve: &[(f64, f64)], level: f64| {
            curve.windows(2).find_map(|w| {
                let ((f0, d0), (f1, d1)) = (w[0], w[1]);
                (f0 >= 25.0 && d0 >= level && d1 < level).then(|| {
                    let a = (level - d0) / (d1 - d0);
                    f0 * (f1 / f0).powf(a)
                })
            })
        };
        let pass0 = ratio_db(&flat, &reference, b.rate, 10.0, 25.0);
        let corner0 = below(&curve0, pass0 - 3.0);
        let (plo, phi) = corner0.map_or((10.0, 25.0), |fc| {
            ((fc / 16.0).max(10.0), (fc / 4.0).max(25.0))
        });
        let flat_db = ratio_db(&flat, &reference, b.rate, plo, phi);
        let bass0 = pass0;
        for &e in &[0.0, 0.25, 0.5, 0.6, 0.7, 0.85] {
            let p = if e == 0.0 { flat.clone() } else { noise(c, e) };
            let curve = ratio_curve(&p, &reference, b.rate);
            let pass = ratio_db(&p, &reference, b.rate, plo, phi);
            let corner = below(&curve, pass - 3.0);
            let top = curve
                .iter()
                .filter(|&&(f, _)| f >= 25.0)
                .copied()
                .max_by(|a, b| a.1.total_cmp(&b.1));
            let (peak_hz, peak_db) = match top {
                Some((f, d)) if d > pass + 0.5 => (Some(f), Some(d - pass)),
                _ => (None, None),
            };
            let slope = corner0.and_then(|fc| {
                let (x, y): (Vec<f64>, Vec<f64>) = curve
                    .iter()
                    .filter(|&&(f, _)| f >= 4.0 * fc && f <= (16.0 * fc).min(12e3))
                    .map(|&(f, d)| (f.log2(), d))
                    .unzip();
                (x.len() >= 4).then(|| line(&x, &y).b)
            });
            response.push(Response {
                cutoff: c,
                emphasis: e,
                oscillates: t.fine.is_some_and(|th| e >= th),
                passband_db: pass - flat_db,
                bass_db: ratio_db(&p, &reference, b.rate, 10.0, 25.0) - bass0,
                corner_hz: corner,
                peak_hz,
                peak_db,
                slope_db_per_octave: slope,
                curve: thirds
                    .iter()
                    .map(|&f| {
                        let (lo, hi) = third(f);
                        (f, ratio_db(&p, &reference, b.rate, lo, hi) - flat_db)
                    })
                    .collect(),
            });
        }
    }
    Filter {
        rings,
        lowest,
        hz_at_half: 2f64.powf(law.a + 0.5 * law.b),
        octaves_per_knob: law.b,
        law_worst_cents: 1200.0 * law.worst,
        law_cubic,
        tracking,
        thresholds,
        amount_cutoff,
        amount,
        open,
        response,
    }
}

// ---------------------------------------------------------------------------------------
// E. Contours.

/// An attack (DECAY 5, SUSTAIN 10): from the key's press, the times to 1 % (the trigger's
/// delay), 10, 50 and 90 % of the rise from rest to peak and to the peak itself, s; and the
/// rise from 5 to 95 % fitted as an RC charge toward a level above the peak:
/// x = target * (1 - e^(-(t - start) / tau)), x the fraction of the rise.
#[derive(Debug, Clone, Serialize)]
pub struct Attack {
    pub knob: f64,
    /// The peak, V.
    pub peak: f64,
    pub t1: f64,
    pub t10: f64,
    pub t50: f64,
    pub t90: f64,
    pub t100: f64,
    pub start: Option<f64>,
    pub tau: Option<f64>,
    pub target: Option<f64>,
    /// The fit's worst residual, as a fraction of the rise.
    pub worst: Option<f64>,
}

/// A fall from the peak or the held level: from the moment it is due (the peak, or the
/// key's release), the delay to 1 % of the way to where it settles and the times to 50, 90
/// and 99 % of the way, s; and the fall from 1 % to 90 % of the way fitted as an
/// exponential approach: its time constant, s, the level it heads for, V, and the worst
/// residual, V (a fall of more than one time constant shows as a poor fit there and in the
/// 99 % time).
#[derive(Debug, Clone, Serialize)]
pub struct Fall {
    pub knob: f64,
    pub from: f64,
    pub settled: f64,
    pub delay: f64,
    pub t50: f64,
    pub t90: f64,
    pub t99: f64,
    pub tau: Option<f64>,
    pub end: Option<f64>,
    pub worst: Option<f64>,
}

/// A release with the DECAY switch on or off.
#[derive(Debug, Clone, Serialize)]
pub struct Release {
    pub decay_switch: bool,
    pub fall: Fall,
}

/// A key released partway through a slow attack (ATTACK 6): the level at the release and
/// the highest after it (fractions of the full rise), when that is (s after the release),
/// and the fall after it.
#[derive(Debug, Clone, Serialize)]
pub struct Cut {
    pub decay_switch: bool,
    pub released_at: f64,
    pub level: f64,
    pub highest: f64,
    pub highest_after: f64,
    pub fall: Fall,
}

#[derive(Debug, Clone, Serialize)]
pub struct Contour {
    /// No key, V.
    pub rest: f64,
    /// The attack's peak at ATTACK 0, DECAY 5, SUSTAIN 0 (it overshoots), and at ATTACK 2
    /// to 10 (the mean), V.
    pub peak: f64,
    pub plateau: f64,
    pub attack: Vec<Attack>,
    /// ATTACK 0, SUSTAIN 0: the fall from the peak.
    pub decay: Vec<Fall>,
    /// The held level (ATTACK 0, DECAY 2): (SUSTAIN, V, fraction of the way from rest to
    /// the plateau), and the volts fitted by a cubic in the knob.
    pub sustain: Vec<(f64, f64, f64)>,
    pub sustain_law: Poly,
    pub release: Vec<Release>,
    /// The DECAY switch off, DECAY 0 to 1 (SUSTAIN 10): each release.
    pub release_off: Vec<Fall>,
    pub cut: Vec<Cut>,
}

/// A key pressed again `gap` s after its release (ATTACK 0, DECAY 3, SUSTAIN 3): whether the
/// loudness contour starts a new attack.
#[derive(Debug, Clone, Serialize)]
pub struct Retrigger {
    pub gap: f64,
    pub retriggered: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Contours {
    pub filter: Contour,
    pub loudness: Contour,
    pub retrigger: Vec<Retrigger>,
    /// A second key pressed while the first is held: whether the contours start again.
    pub legato_retriggers: bool,
}

/// The mean of an attack table's peaks at ATTACK 2 and up (ATTACK 0 overshoots).
fn plateau(t: &[Attack]) -> f64 {
    mean(
        &t.iter()
            .filter(|a| a.knob > 0.1)
            .map(|a| a.peak)
            .collect::<Vec<_>>(),
    )
}

/// Both contours' knobs set alike.
fn knobs(attack: f64, decay: f64, sustain: f64) -> Panel {
    let mut p = silent();
    let k = ca72::voice::ContourKnobs {
        attack,
        decay,
        sustain,
    };
    p.filter_contour = k;
    p.loudness_contour = k;
    p
}

/// The first time (s, interpolated) a record of one contour reaches `level` rising.
fn first_up(t: &[f64], v: &[f64], level: f64) -> f64 {
    for i in 1..v.len() {
        if v[i - 1] < level && v[i] >= level {
            let a = (level - v[i - 1]) / (v[i] - v[i - 1]);
            return t[i - 1] + a * (t[i] - t[i - 1]);
        }
    }
    f64::NAN
}

/// One contour's record from sample `from` on, as a [`Fall`].
fn fall(rate: f64, v: &[f64], from: usize, knob: f64) -> Fall {
    let tail = &v[from..];
    let settled = *tail.last().unwrap_or(&f64::NAN);
    let span = tail[0] - settled;
    let way = |x: f64| (tail[0] - x) / span;
    let when = |q: f64| {
        tail.iter()
            .position(|&x| way(x) >= q)
            .map_or(f64::NAN, |i| i as f64 / rate)
    };
    let (i0, i1) = (
        tail.iter().position(|&x| way(x) >= 0.01).unwrap_or(0),
        tail.iter()
            .position(|&x| way(x) >= 0.9)
            .unwrap_or(tail.len()),
    );
    let points: Vec<(f64, f64)> = (i0..i1.max(i0 + 1).min(tail.len()))
        .map(|i| (i as f64 / rate, tail[i]))
        .collect();
    let (ts, ys): (Vec<f64>, Vec<f64>) = thin(&points, 3000).into_iter().unzip();
    let a = approach(&ts, &ys);
    Fall {
        knob,
        from: tail[0],
        settled,
        delay: when(0.01),
        t50: when(0.5),
        t90: when(0.9),
        t99: when(0.99),
        tau: a.map(|a| a.tau),
        end: a.map(|a| a.end),
        worst: a.map(|a| a.worst),
    }
}

pub fn contours<M: Model>(b: &Bench<M>) -> Contours {
    let rate = b.rate;
    let tol = 1e-5;
    // Rest and peak.
    let mut m = b.fresh(&knobs(0.0, 0.5, 0.0));
    run(&mut m, 0.4);
    let rest = m.contours();
    m.note(60, true);
    let rec = run_contours(&mut m, 0.5);
    let peak = rec
        .iter()
        .fold((f64::MIN, f64::MIN), |(a, c), &(f, l)| (a.max(f), c.max(l)));
    let rests = [rest.0, rest.1];
    let peaks = [peak.0, peak.1];
    let pick = |r: &[(f64, f64)], s: usize| -> Vec<f64> {
        r.iter().map(|&(f, l)| if s == 0 { f } else { l }).collect()
    };
    let mut attack = [Vec::new(), Vec::new()];
    for k in 0..=5 {
        let knob = 0.2 * f64::from(k);
        let mut m = b.fresh(&knobs(knob, 0.5, 1.0));
        m.note(60, true);
        let mut rec = Vec::new();
        contours_until_settled(&mut m, &mut rec, 0.5, 40.0, tol);
        for s in 0..2 {
            let v = pick(&rec, s);
            let top = v.iter().copied().fold(f64::MIN, f64::max);
            let i_top = v.iter().position(|&x| x == top).unwrap_or(0);
            let x: Vec<f64> = v
                .iter()
                .map(|y| (y - rests[s]) / (top - rests[s]))
                .collect();
            let t: Vec<f64> = (0..x.len()).map(|i| i as f64 / rate).collect();
            let rise: Vec<(f64, f64)> = t[..i_top]
                .iter()
                .zip(&x[..i_top])
                .filter(|&(_, &x)| (0.05..=0.95).contains(&x))
                .map(|(&t, &x)| (t, x))
                .collect();
            let (ts, xs): (Vec<f64>, Vec<f64>) = thin(&rise, 3000).into_iter().unzip();
            let fit = approach(&ts, &xs);
            attack[s].push(Attack {
                knob,
                peak: top,
                t1: first_up(&t, &x, 0.01),
                t10: first_up(&t, &x, 0.1),
                t50: first_up(&t, &x, 0.5),
                t90: first_up(&t, &x, 0.9),
                t100: i_top as f64 / rate,
                start: fit.map(|a| a.when(0.0)),
                tau: fit.map(|a| a.tau),
                target: fit.map(|a| a.end),
                worst: fit.map(|a| a.worst),
            });
        }
    }
    let mut decay = [Vec::new(), Vec::new()];
    for k in 0..=5 {
        let knob = 0.2 * f64::from(k);
        let mut m = b.fresh(&knobs(0.0, knob, 0.0));
        m.note(60, true);
        let mut rec = Vec::new();
        contours_until_settled(&mut m, &mut rec, 0.5, 60.0, tol);
        for (s, d) in decay.iter_mut().enumerate() {
            let v = pick(&rec, s);
            let top = v.iter().copied().fold(f64::MIN, f64::max);
            let i_top = v.iter().position(|&x| x == top).unwrap_or(0);
            d.push(fall(rate, &v, i_top, knob));
        }
    }
    let mut sustain = [Vec::new(), Vec::new()];
    for k in 0..=4 {
        let knob = 0.25 * f64::from(k);
        let mut m = b.fresh(&knobs(0.0, 0.2, knob));
        m.note(60, true);
        let mut rec = Vec::new();
        contours_until_settled(&mut m, &mut rec, 1.0, 20.0, tol);
        let held = rec[rec.len() - 1];
        for (s, v) in [held.0, held.1].iter().enumerate() {
            sustain[s].push((knob, *v, (v - rests[s]) / (plateau(&attack[s]) - rests[s])));
        }
    }
    let mut release = [Vec::new(), Vec::new()];
    for switch in [true, false] {
        for knob in [0.2, 0.6] {
            let mut p = knobs(0.0, knob, 1.0);
            p.decay = switch;
            let mut m = b.fresh(&p);
            m.note(60, true);
            let mut rec = Vec::new();
            contours_until_settled(&mut m, &mut rec, 1.0, 40.0, tol);
            m.note(60, false);
            let from = rec.len();
            contours_until_settled(&mut m, &mut rec, 0.5, 40.0, tol);
            for (s, r) in release.iter_mut().enumerate() {
                r.push(Release {
                    decay_switch: switch,
                    fall: fall(rate, &pick(&rec, s), from, knob),
                });
            }
        }
    }
    let mut release_off = [Vec::new(), Vec::new()];
    for k in 0..=10 {
        let knob = 0.1 * f64::from(k);
        let mut p = knobs(0.0, knob, 1.0);
        p.decay = false;
        let mut m = b.fresh(&p);
        m.note(60, true);
        let mut rec = Vec::new();
        contours_until_settled(&mut m, &mut rec, 1.0, 40.0, tol);
        m.note(60, false);
        let from = rec.len();
        contours_until_settled(&mut m, &mut rec, 0.5, 40.0, tol);
        for (s, r) in release_off.iter_mut().enumerate() {
            r.push(fall(rate, &pick(&rec, s), from, knob));
        }
    }
    // A key released at 30 % of ATTACK 6's time to its peak.
    let slow = attack[1][3].t100;
    let mut cut = [Vec::new(), Vec::new()];
    for switch in [true, false] {
        let mut p = knobs(0.6, 0.5, 1.0);
        p.decay = switch;
        let mut m = b.fresh(&p);
        m.note(60, true);
        let mut rec = run_contours(&mut m, 0.3 * slow);
        m.note(60, false);
        let from = rec.len();
        contours_until_settled(&mut m, &mut rec, 0.5, 40.0, tol);
        for s in 0..2 {
            let v = pick(&rec, s);
            let top_v = v[from..].iter().copied().fold(f64::MIN, f64::max);
            let i_top = from + v[from..].iter().position(|&x| x == top_v).unwrap_or(0);
            let rise = plateau(&attack[s]) - rests[s];
            cut[s].push(Cut {
                decay_switch: switch,
                released_at: from as f64 / rate,
                level: (v[from - 1] - rests[s]) / rise,
                highest: (top_v - rests[s]) / rise,
                highest_after: (i_top - from) as f64 / rate,
                fall: fall(rate, &v, i_top, 0.5),
            });
        }
    }
    let mut retrigger = Vec::new();
    for gap in [
        0.002, 0.005, 0.008, 0.010, 0.012, 0.014, 0.016, 0.020, 0.050,
    ] {
        let mut m = b.fresh(&knobs(0.0, 0.3, 0.3));
        m.note(60, true);
        let held = run_contours(&mut m, 2.0)[(2.0 * rate) as usize - 1].1;
        m.note(60, false);
        run_contours(&mut m, gap);
        m.note(60, true);
        let after = run_contours(&mut m, 0.3);
        let top = after.iter().map(|c| c.1).fold(f64::MIN, f64::max);
        retrigger.push(Retrigger {
            gap,
            retriggered: top > held + 0.5 * (peaks[1] - held),
        });
    }
    let legato_retriggers = {
        let mut m = b.fresh(&knobs(0.0, 0.3, 0.3));
        m.note(60, true);
        let held = run_contours(&mut m, 2.0)[(2.0 * rate) as usize - 1].1;
        m.note(64, true);
        let after = run_contours(&mut m, 0.3);
        let top = after.iter().map(|c| c.1).fold(f64::MIN, f64::max);
        top > held + 0.5 * (peaks[1] - held)
    };
    let [fa, la] = attack;
    let [fd, ld] = decay;
    let [fs, ls] = sustain;
    let [fr, lr] = release;
    let [fo, lo] = release_off;
    let [fc, lc] = cut;
    let section = |s: usize,
                   attack: Vec<Attack>,
                   decay,
                   sustain: Vec<(f64, f64, f64)>,
                   release,
                   release_off,
                   cut| {
        let (x, y): (Vec<f64>, Vec<f64>) = sustain.iter().map(|&(k, v, _)| (k, v)).unzip();
        Contour {
            rest: rests[s],
            peak: peaks[s],
            plateau: plateau(&attack),
            attack,
            decay,
            sustain_law: poly(&x, &y, 3),
            sustain,
            release,
            release_off,
            cut,
        }
    };
    Contours {
        filter: section(0, fa, fd, fs, fr, fo, fc),
        loudness: section(1, la, ld, ls, lr, lo, lc),
        retrigger,
        legato_retriggers,
    }
}

// ---------------------------------------------------------------------------------------
// F. VCA.

/// The loudness contour held at a SUSTAIN: its voltage, the output's RMS (oscillator 1's
/// sawtooth at 110 Hz, VOLUME 3, the filter open) and that against SUSTAIN 10's.
#[derive(Debug, Clone, Serialize)]
pub struct Gain {
    pub sustain: f64,
    pub volts: f64,
    pub rms: f64,
    pub gain: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Vca {
    pub points: Vec<Gain>,
    /// The gain against SUSTAIN 10's through ATTACK 10's slow rise (11 s from rest to its
    /// peak): the mean over each 0.1 V of the contour, (V, gain), from four periods' RMS at
    /// a time.
    pub sweep: Vec<(f64, f64)>,
    /// No key held: the output's RMS, its fundamental's amplitude and both against SUSTAIN
    /// 10's, dB; and the loudness contour then, V.
    pub bleed_rms: f64,
    pub bleed_fundamental: f64,
    pub bleed_db: f64,
    pub rest_volts: f64,
}

pub fn vca<M: Model>(b: &Bench<M>) -> Vca {
    let mut points = Vec::new();
    for s in [0.0, 0.05, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0] {
        let mut p = panel();
        p.loudness_contour.decay = 0.2;
        p.loudness_contour.sustain = s;
        let mut m = b.fresh(&p);
        m.note(45, true);
        run(&mut m, 2.0);
        let n = (SPAN * b.rate) as usize;
        let mut x = Vec::with_capacity(n);
        let mut v = 0.0;
        for _ in 0..n {
            x.push(m.tick(0.0));
            v += m.contours().1;
        }
        points.push(Gain {
            sustain: s,
            volts: v / n as f64,
            rms: rms(&x),
            gain: 0.0,
        });
    }
    let full = points.last().map_or(1.0, |g| g.rms);
    for g in &mut points {
        g.gain = g.rms / full;
    }
    let mut p = panel();
    p.loudness_contour.attack = 1.0;
    let mut m = b.fresh(&p);
    m.note(45, true);
    let window = (4.0 * b.rate / 110.0).round() as usize;
    let mut bins: Vec<(f64, f64, usize)> = (0..60)
        .map(|k| (0.1 * f64::from(k) - 0.6, 0.0, 0))
        .collect();
    let mut top = f64::MIN;
    for _ in 0..(14.0 * b.rate) as usize / window {
        let mut x = Vec::with_capacity(window);
        let mut v = 0.0;
        for _ in 0..window {
            x.push(m.tick(0.0));
            v += m.contours().1;
        }
        let v = v / window as f64;
        if v < top - 0.01 {
            break;
        }
        top = top.max(v);
        let k = ((v + 0.65) / 0.1).floor();
        if (0.0..60.0).contains(&k) {
            let bin = &mut bins[k as usize];
            bin.1 += rms(&x) / full;
            bin.2 += 1;
        }
    }
    let sweep = bins
        .iter()
        .filter(|b| b.2 > 0)
        .map(|b| (b.0, b.1 / b.2 as f64))
        .collect();
    let mut m = b.fresh(&panel());
    run(&mut m, SETTLE);
    let x = run(&mut m, 2.0);
    let rest_volts = m.contours().1;
    let f = b.hz(&panel(), 45, SPAN);
    let bleed_fundamental = tone(&x, b.rate, f).0;
    let bleed_rms = rms(&x);
    Vca {
        points,
        sweep,
        bleed_rms,
        bleed_fundamental,
        bleed_db: db(bleed_rms / full),
        rest_volts,
    }
}

// ---------------------------------------------------------------------------------------
// G. Noise.

/// The noise at a VOLUME, oscillators off, the filter open, a key held: the output's RMS,
/// the spectrum's slope from 100 Hz to 10 kHz (a line through the third-octave bands'
/// densities against octaves), oscillator 1's sawtooth at 110 Hz at the same VOLUME, and
/// the noise against it, dB; and the third-octave densities, dB against 1 kHz's.
#[derive(Debug, Clone, Serialize)]
pub struct NoiseLevel {
    pub pink: bool,
    pub volume: f64,
    pub rms: f64,
    pub slope_db_per_octave: f64,
    pub saw_rms: f64,
    pub against_saw_db: f64,
    pub bands: Vec<(f64, f64)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Noise {
    pub levels: Vec<NoiseLevel>,
    /// The white and the pink noise's RMS at VOLUME 0 to 1, and their channel's laws (11K
    /// in series, not 33K, and the pink from behind R50 24K: [`ChannelLaw`]).
    pub white_volume: Vec<(f64, f64)>,
    pub white_law: ChannelLaw,
    pub pink_volume: Vec<(f64, f64)>,
    pub pink_law: ChannelLaw,
}

pub fn noise<M: Model>(b: &Bench<M>) -> Noise {
    let noisy = |pink: bool, v: f64, seconds: f64| -> Vec<f64> {
        let mut p = silent();
        p.noise_on = true;
        p.noise_pink = pink;
        p.noise_volume = v;
        b.held(&p, 60, SETTLE, seconds)
    };
    let mut levels = Vec::new();
    let thirds: Vec<f64> = third_octaves()
        .into_iter()
        .filter(|&f| (99.0..=10.1e3).contains(&f))
        .collect();
    for pink in [false, true] {
        for v in [1.0, 0.5] {
            let x = noisy(pink, v, 4.0);
            let sp = psd(&x, b.rate, 8192);
            let bands: Vec<(f64, f64)> = thirds
                .iter()
                .map(|&f| {
                    let (lo, hi) = (f / 2f64.powf(1.0 / 6.0), f * 2f64.powf(1.0 / 6.0));
                    (f, 10.0 * band(&sp, b.rate, lo, hi).log10())
                })
                .collect();
            let (lx, ly): (Vec<f64>, Vec<f64>) = bands.iter().map(|&(f, d)| (f.log2(), d)).unzip();
            let at_1k = bands
                .iter()
                .min_by(|a, c| (a.0 - 1e3).abs().total_cmp(&(c.0 - 1e3).abs()))
                .map_or(0.0, |&(_, d)| d);
            let mut p = panel();
            p.osc[0].volume = v;
            let saw = rms(&b.held(&p, 45, SETTLE, SPAN));
            let r = rms(&x);
            levels.push(NoiseLevel {
                pink,
                volume: v,
                rms: r,
                slope_db_per_octave: line(&lx, &ly).b,
                saw_rms: saw,
                against_saw_db: db(r / saw),
                bands: bands.iter().map(|&(f, d)| (f, d - at_1k)).collect(),
            });
        }
    }
    let sweep = |pink: bool| -> Vec<(f64, f64)> {
        (0..=10)
            .map(|k| {
                let v = f64::from(k) / 10.0;
                (v, rms(&noisy(pink, v, 2.0)))
            })
            .collect()
    };
    let (white_volume, pink_volume) = (sweep(false), sweep(true));
    Noise {
        levels,
        white_law: channel_law(&white_volume, 1.0),
        white_volume,
        pink_law: channel_law(&pink_volume, 1.0),
        pink_volume,
    }
}

// ---------------------------------------------------------------------------------------
// H. Modulation.

/// Oscillator 3 on LO (its triangle, FREQUENCY at its centre, out of the mixer) through
/// MODULATION MIX 0 and the MODULATION wheel. On oscillator 1 (OSCILLATOR MODULATION; its
/// sawtooth at 8', key 60): the pitch's highest and lowest, cents from the wheel at 0, half
/// the swing, the middle of the swing and its mean over time. On the filter (FILTER
/// MODULATION, EMPHASIS 10, CUTOFF 0.5, sources off): the same in octaves. And the LFO's
/// frequency, read from the swing, and one cycle of the swing at 32 phases from its rising
/// crossing of its mean (as [`fold`]).
#[derive(Debug, Clone, Serialize)]
pub struct Swing {
    pub wheel: f64,
    pub lfo_hz: Option<f64>,
    pub up: f64,
    pub down: f64,
    pub depth: f64,
    pub centre: f64,
    pub mean: f64,
    pub shape: Vec<f64>,
}

/// MODULATION MIX 1 (the noise): the pitch's RMS deviation about its mean and that mean
/// (cents from the wheel at 0, one estimate a period, weighted by its length), and the
/// self-oscillating filter's (octaves).
#[derive(Debug, Clone, Serialize)]
pub struct Jitter {
    pub wheel: f64,
    pub pitch_rms_cents: f64,
    pub pitch_mean_cents: f64,
    pub filter_rms_octaves: f64,
    pub filter_mean_octaves: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Modulation {
    /// With the wheel at 0: oscillator 1's frequency, the filter's ringing frequency.
    pub pitch_hz: f64,
    pub filter_hz: f64,
    pub pitch: Vec<Swing>,
    pub filter: Vec<Swing>,
    pub noise: Vec<Jitter>,
}

/// The modulation panel: oscillator 3 on LO, its triangle, off the mixer, MODULATION MIX
/// `mix`, the wheel at `wheel`; oscillator 1 on (OSCILLATOR MODULATION) or every source off
/// and the filter ringing (FILTER MODULATION).
fn modulated(mix: f64, wheel: f64, on_filter: bool) -> Panel {
    let mut p = if on_filter { silent() } else { panel() };
    p.osc[2] = OscPanel {
        range: Range::Lo,
        on: false,
        ..osc(Waveform::Triangle, 0.3)
    };
    p.mod_mix = mix;
    p.mod_wheel = wheel;
    if on_filter {
        p.filter_mod = true;
        p.emphasis = 1.0;
        p.cutoff = 0.5;
    } else {
        p.osc_mod = true;
    }
    p
}

/// The track of one estimate a period of a modulated output: (s, Hz).
fn track<M: Model>(b: &Bench<M>, p: &Panel, on_filter: bool, seconds: f64) -> Vec<(f64, f64)> {
    let x = if on_filter {
        b.kicked(p, 60, 0.3, 1.0, seconds)
    } else {
        b.held(p, 60, 1.0, seconds)
    };
    periods(&x, b.rate)
}

/// The mean and the RMS about it of a track of one estimate a period, each weighted by its
/// period (an estimate a period counts high frequencies more often than low).
fn timed(t: &[(f64, f64)], to: f64, scale: f64) -> (f64, f64) {
    let (mut w, mut s, mut s2) = (0.0, 0.0, 0.0);
    for &(_, f) in t {
        let y = scale * (f / to).log2();
        s += y / f;
        s2 += y * y / f;
        w += 1.0 / f;
    }
    let m = s / w;
    (m, (s2 / w - m * m).max(0.0).sqrt())
}

/// A track resampled every millisecond (linearly between its estimates).
fn uniform(ts: &[f64], ys: &[f64]) -> Vec<f64> {
    let mut out = Vec::new();
    let mut i = 0;
    let mut t = ts[0];
    while t <= ts[ts.len() - 1] {
        while ts[i + 1] < t {
            i += 1;
        }
        let a = (t - ts[i]) / (ts[i + 1] - ts[i]);
        out.push(ys[i] + a * (ys[i + 1] - ys[i]));
        t += 1e-3;
    }
    out
}

fn swing(t: &[(f64, f64)], wheel: f64, to: f64, scale: f64) -> Swing {
    let (ts, ys): (Vec<f64>, Vec<f64>) =
        t.iter().map(|&(s, f)| (s, scale * (f / to).log2())).unzip();
    let ups = rising_at(&ts, &ys);
    let lfo_hz = (ups.len() >= 3).then(|| (ups.len() - 1) as f64 / (ups[ups.len() - 1] - ups[0]));
    let (lo, hi) = ys
        .iter()
        .fold((f64::MAX, f64::MIN), |(a, c), &v| (a.min(v), c.max(v)));
    let shape = lfo_hz.map_or_else(Vec::new, |f| fold(&uniform(&ts, &ys), 1e3, f, 32));
    Swing {
        wheel,
        lfo_hz,
        up: hi,
        down: lo,
        depth: 0.5 * (hi - lo),
        centre: 0.5 * (hi + lo),
        mean: timed(t, to, scale).0,
        shape,
    }
}

pub fn modulation<M: Model>(b: &Bench<M>) -> Modulation {
    let pitch_hz = b.hz(&modulated(0.0, 0.0, false), 60, SPAN);
    let filter_hz = frequency(
        &b.kicked(&modulated(0.0, 0.0, true), 60, 0.3, 1.0, SPAN),
        b.rate,
    )
    .unwrap_or(f64::NAN);
    let mut pitch = Vec::new();
    let mut filter = Vec::new();
    for wheel in [1.0, 0.75, 0.5, 0.25] {
        let t = track(b, &modulated(0.0, wheel, false), false, 6.0);
        pitch.push(swing(&t, wheel, pitch_hz, 1200.0));
        let t = track(b, &modulated(0.0, wheel, true), true, 6.0);
        filter.push(swing(&t, wheel, filter_hz, 1.0));
    }
    let noise = [1.0, 0.5]
        .iter()
        .map(|&wheel| {
            let (pm, pr) = timed(
                &track(b, &modulated(1.0, wheel, false), false, 4.0),
                pitch_hz,
                1200.0,
            );
            let (fm, fr) = timed(
                &track(b, &modulated(1.0, wheel, true), true, 4.0),
                filter_hz,
                1.0,
            );
            Jitter {
                wheel,
                pitch_rms_cents: pr,
                pitch_mean_cents: pm,
                filter_rms_octaves: fr,
                filter_mean_octaves: fm,
            }
        })
        .collect();
    Modulation {
        pitch_hz,
        filter_hz,
        pitch,
        filter,
        noise,
    }
}

// ---------------------------------------------------------------------------------------
// I. External input.

/// A 220 Hz sine at EXTERNAL INPUT (amplitude in units of 5 V), oscillators off, the filter
/// open, a key held: the output's fundamental (amplitude), the gain from the jack, the
/// harmonics 2 to 10 against the fundamental (THD, %), the output's peak from its mean, and
/// the OVERLOAD lamp's brightest and mean (from the model's own reading, where it has one).
#[derive(Debug, Clone, Serialize)]
pub struct Input {
    pub volume: f64,
    pub amplitude: f64,
    pub fundamental: f64,
    pub gain: f64,
    pub thd_pct: f64,
    pub peak: f64,
    pub lamp_max: f64,
    pub lamp_mean: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct External {
    pub inputs: Vec<Input>,
}

/// The external input; `lamp` reads the model's OVERLOAD lamp (0 off, 1 fully lit) after
/// each sample, if it has one.
pub fn external<M: Model>(b: &Bench<M>, lamp: &dyn Fn(&M) -> f64) -> External {
    let mut inputs = Vec::new();
    for volume in [0.25, 0.5, 0.75, 1.0] {
        for a in [0.0, 0.0003, 0.001, 0.003, 0.01, 0.03, 0.1, 0.3, 1.0] {
            let mut p = silent();
            p.ext_on = true;
            p.ext_volume = volume;
            let mut m = b.fresh(&p);
            m.note(60, true);
            let w = 2.0 * PI * 220.0 / b.rate;
            let settle = (SETTLE * b.rate) as usize;
            let n = (SPAN * b.rate) as usize;
            let mut x = Vec::with_capacity(n);
            let (mut lmax, mut lsum) = (0.0f64, 0.0);
            for i in 0..settle + n {
                let y = m.tick(a * (w * i as f64).sin());
                if i >= settle {
                    x.push(y);
                    let l = lamp(&m);
                    lmax = lmax.max(l);
                    lsum += l;
                }
            }
            let h = harmonics(&x, b.rate, 220.0, 10);
            let rest: f64 = h[1..].iter().map(|v| v * v).sum::<f64>().sqrt();
            let mu = mean(&x);
            inputs.push(Input {
                volume,
                amplitude: a,
                fundamental: h[0],
                gain: if a > 0.0 { h[0] / a } else { 0.0 },
                thd_pct: if a > 0.0 { 100.0 * rest / h[0] } else { 0.0 },
                peak: x.iter().map(|v| (v - mu).abs()).fold(0.0, f64::max),
                lamp_max: lmax,
                lamp_mean: lsum / n as f64,
            });
        }
    }
    External { inputs }
}

// ---------------------------------------------------------------------------------------
// J. A-440.

/// A-440 on, no key, the rest at the measurement panel: the output's RMS, frequency and
/// harmonics (1 to 10: the fundamental's amplitude, the rest against it, dB).
#[derive(Debug, Clone, Serialize)]
pub struct A440 {
    pub rms: f64,
    pub hz: f64,
    pub fundamental: f64,
    pub harmonics_db: Vec<f64>,
    /// One period at 64 phases (as [`Wave::shape`]).
    pub shape: Vec<f64>,
}

pub fn a440<M: Model>(b: &Bench<M>) -> A440 {
    let mut p = panel();
    p.a440 = true;
    let mut m = b.fresh(&p);
    run(&mut m, SETTLE);
    let x = run(&mut m, SPAN);
    let hz = frequency(&x, b.rate).unwrap_or(f64::NAN);
    let h = harmonics(&x, b.rate, hz, 10);
    let fine = fold(&x, b.rate, hz, 64);
    let (m0, pp) = (mean(&fine), peak_to_peak(&fine));
    A440 {
        rms: rms(&x),
        hz,
        fundamental: h[0],
        harmonics_db: h[1..].iter().map(|a| db(a / h[0])).collect(),
        shape: fine.iter().map(|v| (v - m0) / pp).collect(),
    }
}

// ---------------------------------------------------------------------------------------
// K. Glide.

/// A jump between two keys with GLIDE on, oscillator 1's sawtooth at 2' (an estimate of its
/// pitch every millisecond or two): legato (the new key pressed while the old is held, the
/// old then released, unless the new key is lower, when lowest-note priority takes it at
/// once) or after a gap (the old key released, the new pressed 50 ms later). From the event
/// that starts it (the old key's release or the new key's press), in semitones of the jump:
/// the delay to 2 % of the way, the times to 50 % of the way, to within a semitone and to
/// within 10 cents; the middle (25 to 75 % of the way) fitted as a line, its rate
/// (semitones a second) and worst residual (semitones); the arrival, the semitones left from
/// half a semitone until the pitch first comes within 3 cents, fitted as an exponential
/// decay, its time constant and worst residual; and the largest distance from the new key's
/// pitch after that, cents.
#[derive(Debug, Clone, Serialize)]
pub struct Slide {
    pub knob: f64,
    pub from: i32,
    pub to: i32,
    pub legato: bool,
    pub delay: f64,
    pub to_half: f64,
    pub to_semitone: f64,
    pub to_10_cents: f64,
    pub rate: Option<f64>,
    pub rate_worst: Option<f64>,
    pub tau: Option<f64>,
    pub tau_worst: Option<f64>,
    pub settle_cents: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Glide {
    pub slides: Vec<Slide>,
}

/// One jump from key `from` to key `to` at GLIDE `knob` on `base`, `target` the new key's
/// frequency without GLIDE.
fn slide<M: Model>(
    b: &Bench<M>,
    base: &Panel,
    knob: f64,
    from: i32,
    to: i32,
    legato: bool,
    target: f64,
) -> Slide {
    let mut p = *base;
    p.glide_on = true;
    p.glide = knob;
    let mut m = b.fresh(&p);
    m.note(from, true);
    run(&mut m, 0.6);
    let mut x = Vec::new();
    if !legato {
        m.note(from, false);
        x = run(&mut m, 0.05);
        m.note(to, true);
    } else if to > from {
        m.note(to, true);
        run(&mut m, 0.2);
        m.note(from, false);
    } else {
        m.note(to, true);
    }
    let lead = x.len() as f64 / b.rate;
    // Long enough for GLIDE's own law at its slowest guess (5M on 1 uF, 8 time
    // constants), at most 20 s.
    let seconds = (1.0 + 40.0 * (81f64.powf(knob) - 1.0) / 80.0).min(20.0);
    x.extend(run(&mut m, seconds));
    let jump = f64::from(to - from);
    // The fraction of the jump left, against time from the event.
    let left: Vec<(f64, f64)> = periods(&x, b.rate)
        .into_iter()
        .map(|(s, f)| (s - lead, 12.0 * (target / f).log2() / jump))
        .filter(|&(s, _)| s > 0.0)
        .collect();
    let when = |q: f64| {
        left.iter()
            .find(|&&(_, u)| u <= 1.0 - q)
            .map_or(f64::NAN, |&(s, _)| s)
    };
    let within = |d: f64| {
        left.iter()
            .rposition(|&(_, u)| (u * jump).abs() > d)
            .and_then(|i| left.get(i + 1))
            .map_or(f64::NAN, |&(s, _)| s)
    };
    let middle: Vec<(f64, f64)> = left
        .iter()
        .copied()
        .filter(|&(_, u)| (0.25..=0.75).contains(&u))
        .map(|(s, u)| (s, (1.0 - u) * jump.abs()))
        .collect();
    let (mt, ms): (Vec<f64>, Vec<f64>) = middle.iter().copied().unzip();
    let ramp = (mt.len() >= 3).then(|| line(&mt, &ms));
    let arrived = left
        .iter()
        .position(|&(_, u)| (u * jump).abs() <= 0.03)
        .unwrap_or(left.len());
    let tail: Vec<(f64, f64)> = left[..arrived]
        .iter()
        .copied()
        .map(|(s, u)| (s, u * jump.abs()))
        .filter(|&(_, d)| (0.03..=0.5).contains(&d))
        .collect();
    let settle_cents = left[arrived..]
        .iter()
        .map(|&(_, u)| 100.0 * (u * jump).abs())
        .fold(0.0, f64::max);
    let (tt, ts): (Vec<f64>, Vec<f64>) = thin(&tail, 3000).into_iter().unzip();
    let decay = approach_to(&tt, &ts, 0.0);
    Slide {
        knob,
        from,
        to,
        legato,
        delay: when(0.02),
        to_half: when(0.5),
        to_semitone: within(1.0),
        to_10_cents: within(0.1),
        rate: ramp.map(|l| l.b),
        rate_worst: ramp.map(|l| l.worst),
        tau: decay.map(|a| a.tau),
        tau_worst: decay.map(|a| a.worst),
        settle_cents,
    }
}

pub fn glide<M: Model>(b: &Bench<M>) -> Glide {
    let mut base = panel();
    base.osc[0].range = Range::R2;
    let target = |k: i32| b.hz(&base, k, SPAN);
    let (t48, t60, t62) = (target(48), target(60), target(62));
    let mut slides = Vec::new();
    for knob in [0.2, 0.4, 0.6, 0.8, 1.0] {
        slides.push(slide(b, &base, knob, 48, 60, true, t60));
        slides.push(slide(b, &base, knob, 48, 60, false, t60));
        slides.push(slide(b, &base, knob, 60, 48, true, t48));
        slides.push(slide(b, &base, knob, 60, 62, true, t62));
    }
    Glide { slides }
}

// ---------------------------------------------------------------------------------------
// L. Output.

/// The default panel (`Panel::default()`) with key 57 held: the output's RMS and peak from
/// its mean; and at rest (no key): the output's mean and RMS, on the default panel and the
/// measurement panel.
#[derive(Debug, Clone, Serialize)]
pub struct Output {
    pub default_rms: f64,
    pub default_peak: f64,
    pub default_rest_mean: f64,
    pub default_rest_rms: f64,
    pub measurement_rest_mean: f64,
    pub measurement_rest_rms: f64,
}

pub fn output<M: Model>(b: &Bench<M>) -> Output {
    let d = Panel::default();
    let x = b.held(&d, 57, SETTLE, SPAN);
    let mu = mean(&x);
    let rest = |p: &Panel| {
        let mut m = b.fresh(p);
        run(&mut m, SETTLE);
        run(&mut m, SPAN)
    };
    let (r0, r1) = (rest(&d), rest(&panel()));
    Output {
        default_rms: rms(&x),
        default_peak: x.iter().map(|v| (v - mu).abs()).fold(0.0, f64::max),
        default_rest_mean: mean(&r0),
        default_rest_rms: rms(&r0),
        measurement_rest_mean: mean(&r1),
        measurement_rest_rms: rms(&r1),
    }
}

// ---------------------------------------------------------------------------------------
// M. The plug-in's own: DRIVE, FILTER MODE and FEEDBACK.

/// The energy of a [`psd`] over `[lo, hi)`, (output units)^2.
pub fn energy(p: &[f64], rate: f64, lo: f64, hi: f64) -> f64 {
    let n = 2 * (p.len() - 1);
    let bin = rate / n as f64;
    let (a, b) = (
        (lo / bin).ceil() as usize,
        ((hi / bin).ceil() as usize).min(p.len()),
    );
    p[a.min(b)..b].iter().sum::<f64>() * bin
}

/// A [`psd`]'s centroid from 20 Hz to 16 kHz, Hz.
pub fn centroid(p: &[f64], rate: f64) -> f64 {
    let n = 2 * (p.len() - 1);
    let bin = rate / n as f64;
    let (mut s, mut w) = (0.0, 0.0);
    for (k, v) in p.iter().enumerate() {
        let f = k as f64 * bin;
        if (20.0..16e3).contains(&f) {
            s += f * v;
            w += v;
        }
    }
    s / w
}

/// A [`psd`]'s third-octave densities, dB (of (output units)^2 per Hz).
pub fn thirds_db(p: &[f64], rate: f64) -> Vec<(f64, f64)> {
    third_octaves()
        .iter()
        .map(|&f| {
            let (lo, hi) = (f / 2f64.powf(1.0 / 6.0), f * 2f64.powf(1.0 / 6.0));
            (f, 10.0 * band(p, rate, lo, hi).log10())
        })
        .collect()
}

/// A sound's level and colour: its RMS, the spectrum's centroid, the energy over 1 kHz
/// against that under it (dB), its fundamental's frequency when it has one, and its
/// third-octave densities.
#[derive(Debug, Clone, Serialize)]
pub struct Colour {
    pub rms: f64,
    pub centroid_hz: f64,
    pub highs_db: f64,
    pub hz: Option<f64>,
    pub bands: Vec<(f64, f64)>,
}

pub fn colour(x: &[f64], rate: f64) -> Colour {
    let p = psd(x, rate, 8192);
    Colour {
        rms: rms(x),
        centroid_hz: centroid(&p, rate),
        highs_db: 10.0 * (energy(&p, rate, 1e3, 16e3) / energy(&p, rate, 20.0, 1e3)).log10(),
        hz: frequency(x, rate),
        bands: thirds_db(&p, rate),
    }
}

/// A patch at a DRIVE (dB): its colour, and its level against DRIVE off, dB.
#[derive(Debug, Clone, Serialize)]
pub struct Driven {
    pub patch: &'static str,
    pub drive_db: f64,
    pub level_db: f64,
    pub colour: Colour,
}

/// FILTER MODE HI against LO at a CUTOFF and EMPHASIS, white noise at VOLUME 5: each mode's
/// curve at third-octave centres, dB against LO's open filter (CUTOFF 1, EMPHASIS 0).
#[derive(Debug, Clone, Serialize)]
pub struct ModeCurve {
    pub cutoff: f64,
    pub emphasis: f64,
    pub lo: Vec<(f64, f64)>,
    pub hi: Vec<(f64, f64)>,
}

/// A held sawtooth through either mode: CUTOFF, EMPHASIS, and each mode's colour.
#[derive(Debug, Clone, Serialize)]
pub struct ModeSaw {
    pub cutoff: f64,
    pub emphasis: f64,
    pub lo: Colour,
    pub hi: Colour,
}

/// FEEDBACK's knob (0..1) and share on a patch: the colour 1 s after a burst.
#[derive(Debug, Clone, Serialize)]
pub struct Fed {
    pub patch: &'static str,
    pub knob: f64,
    pub share: f64,
    pub colour: Colour,
}

#[derive(Debug, Clone, Serialize)]
pub struct More {
    pub drive: Vec<Driven>,
    pub mode_curves: Vec<ModeCurve>,
    pub mode_saws: Vec<ModeSaw>,
    /// EMPHASIS 10 at CUTOFF 0.5 in HI: its ring (as [`Filter::rings`]).
    pub mode_ring: Ring,
    pub feedback: Vec<Fed>,
}

/// A fresh model on `p` at DRIVE's gain and FEEDBACK's share (`(gain, share)`), `key`
/// held, a burst (as [`kick`]) when `burst` is above 0: `settle` s, then `seconds` s of its
/// output.
fn played<M: Model>(
    b: &Bench<M>,
    p: &Panel,
    (gain, share): (f64, f64),
    (key, burst): (i32, f64),
    settle: f64,
    seconds: f64,
) -> Vec<f64> {
    let mut m = (b.new)();
    m.set_drive(gain);
    m.set_feedback(share);
    m.set_panel(p);
    run(&mut m, REST);
    m.note(key, true);
    if burst > 0.0 {
        kick(&mut m, p, burst);
    }
    run(&mut m, settle);
    run(&mut m, seconds)
}

/// The DRIVE patches: (name, panel, key).
pub fn drive_patches() -> Vec<(&'static str, Panel, i32)> {
    let saws = |volume: f64, cutoff: f64, emphasis: f64| {
        let mut p = silent();
        // (Oscillator 2 a tritone up, 3 a minor third down: FREQUENCY's laws measured.)
        for (j, f) in [FREQ_CENTRE, 0.8, 0.29].iter().enumerate() {
            p.osc[j] = OscPanel {
                freq: *f,
                ..osc(Waveform::Sawtooth, volume)
            };
        }
        p.cutoff = cutoff;
        p.emphasis = emphasis;
        p
    };
    let mut one = panel();
    one.osc[0].volume = 0.5;
    let mut tri = panel();
    tri.osc[0] = osc(Waveform::Triangle, 0.5);
    let mut sq = silent();
    sq.osc[0] = osc(Waveform::Square, 0.5);
    sq.cutoff = 0.35;
    sq.emphasis = 0.7;
    vec![
        ("saw 5 open", one, 45),
        ("triangle 5 open", tri, 45),
        ("three saws 5, CUTOFF .6", saws(0.5, 0.6, 0.0), 45),
        (
            "three saws 5, CUTOFF .6 EMPHASIS 7",
            saws(0.5, 0.6, 0.7),
            45,
        ),
        (
            "three saws 10, CUTOFF .4 EMPHASIS 7",
            saws(1.0, 0.4, 0.7),
            45,
        ),
        ("square 5, CUTOFF .35 EMPHASIS 7", sq, 33),
    ]
}

/// The FEEDBACK patches: (name, panel, key, burst).
pub fn feedback_patches() -> Vec<(&'static str, Panel, i32, f64)> {
    let fed = |p: &mut Panel, volume: f64| {
        p.ext_on = true;
        p.ext_volume = volume;
    };
    let mut alone = silent();
    fed(&mut alone, 0.8);
    alone.cutoff = 0.5;
    let mut alone7 = alone;
    alone7.cutoff = 0.3;
    alone7.emphasis = 0.7;
    let mut saw = panel();
    saw.osc[0].volume = 0.5;
    fed(&mut saw, 0.8);
    saw.cutoff = 0.6;
    let mut growl = silent();
    for (j, f) in [FREQ_CENTRE, 0.8, 0.29].iter().enumerate() {
        growl.osc[j] = OscPanel {
            freq: *f,
            ..osc(Waveform::Sawtooth, 0.7)
        };
    }
    fed(&mut growl, 0.9);
    growl.cutoff = 0.1;
    growl.emphasis = 0.7;
    vec![
        ("alone, CUTOFF .5", alone, 60, 0.3),
        ("alone, CUTOFF .3 EMPHASIS 7", alone7, 60, 0.3),
        ("saw 5, CUTOFF .6", saw, 45, 0.0),
        ("three saws 7, CUTOFF .1 EMPHASIS 7", growl, 33, 0.0),
    ]
}

pub fn more<M: Model>(b: &Bench<M>) -> More {
    let mut drive = Vec::new();
    for (name, p, key) in drive_patches() {
        let mut off = f64::NAN;
        for d in [0.0, 6.0, 12.0, 18.0, 24.0] {
            let x = played(b, &p, (10f64.powf(d / 20.0), 0.0), (key, 0.0), SETTLE, SPAN);
            let c = colour(&x, b.rate);
            if d == 0.0 {
                off = c.rms;
            }
            drive.push(Driven {
                patch: name,
                drive_db: d,
                level_db: db(c.rms / off),
                colour: c,
            });
        }
    }
    let noisy = |cutoff: f64, emphasis: f64, hi: bool| {
        let mut p = silent();
        p.noise_on = true;
        p.noise_volume = 0.5;
        p.cutoff = cutoff;
        p.emphasis = emphasis;
        p.filter_mode = if hi {
            ca72::vcf::FilterMode::Hi
        } else {
            ca72::vcf::FilterMode::Lo
        };
        psd(&b.held(&p, 60, SETTLE, 8.0), b.rate, 16384)
    };
    let reference = noisy(1.0, 0.0, false);
    let thirds = third_octaves();
    let curve = |p: &[f64]| -> Vec<(f64, f64)> {
        thirds
            .iter()
            .map(|&f| {
                let (lo, hi) = (f / 2f64.powf(1.0 / 6.0), f * 2f64.powf(1.0 / 6.0));
                (f, ratio_db(p, &reference, b.rate, lo, hi))
            })
            .collect()
    };
    let mut mode_curves = Vec::new();
    for c in [0.3, 0.5, 0.7] {
        for e in [0.0, 0.25, 0.5, 0.75] {
            mode_curves.push(ModeCurve {
                cutoff: c,
                emphasis: e,
                lo: curve(&noisy(c, e, false)),
                hi: curve(&noisy(c, e, true)),
            });
        }
    }
    let mut mode_saws = Vec::new();
    for (c, e) in [(0.3, 0.0), (0.5, 0.0), (0.7, 0.0), (0.5, 0.5), (0.5, 0.7)] {
        let mut p = panel();
        p.cutoff = c;
        p.emphasis = e;
        let lo = colour(&b.held(&p, 45, SETTLE, SPAN), b.rate);
        p.filter_mode = ca72::vcf::FilterMode::Hi;
        let hi = colour(&b.held(&p, 45, SETTLE, SPAN), b.rate);
        mode_saws.push(ModeSaw {
            cutoff: c,
            emphasis: e,
            lo,
            hi,
        });
    }
    let mut res = silent();
    res.emphasis = 1.0;
    res.cutoff = 0.5;
    res.filter_mode = ca72::vcf::FilterMode::Hi;
    let mode_ring = ring(b, &res, 60);
    let mut feedback = Vec::new();
    for (name, p, key, burst) in feedback_patches() {
        for knob in [0.2, 0.38, 0.5, 0.7] {
            let share = ca72::voice::feedback_law(knob);
            let x = played(b, &p, (1.0, share), (key, burst), 1.0, SPAN);
            feedback.push(Fed {
                patch: name,
                knob,
                share,
                colour: colour(&x, b.rate),
            });
        }
    }
    More {
        drive,
        mode_curves,
        mode_saws,
        mode_ring,
        feedback,
    }
}

// ---------------------------------------------------------------------------------------
// Everything, and its report.

/// Every measurement of one model.
#[derive(Debug, Clone, Serialize)]
pub struct Reference {
    pub rate: f64,
    pub pitch: Pitch,
    pub waveforms: Waveforms,
    pub mixer: Mixer,
    pub filter: Filter,
    pub contours: Contours,
    pub vca: Vca,
    pub noise: Noise,
    pub modulation: Modulation,
    pub external: External,
    pub a440: A440,
    pub glide: Glide,
    pub output: Output,
}

/// The FREQUENCY knobs for the mixer's stacks: oscillator 2 a tritone above oscillator 1,
/// oscillator 3 a minor third below (no low harmonics of the three coincide), from the
/// pitch measurement's tables.
pub fn stack_knobs(p: &Pitch) -> (f64, f64) {
    let table = |t: &[At]| t.iter().map(|a| (a.x, a.cents)).collect::<Vec<_>>();
    (
        knob_for(&table(&p.osc2), 600.0),
        knob_for(&table(&p.osc3), -300.0),
    )
}

/// Runs every measurement, each section's time (s) printed as it finishes; `lamp` as for
/// [`external`].
pub fn measure_all<M: Model>(b: &Bench<M>, lamp: &dyn Fn(&M) -> f64) -> Reference {
    let clock = std::time::Instant::now();
    let lap = |what: &str| eprintln!("{what}: {:.1} s", clock.elapsed().as_secs_f64());
    let pitch = pitch(b);
    lap("pitch");
    let waveforms = waveforms(b);
    lap("waveforms");
    let (k2, k3) = stack_knobs(&pitch);
    let mixer = mixer(b, k2, k3);
    lap("mixer");
    let filter = filter(b);
    lap("filter");
    let contours = contours(b);
    lap("contours");
    let vca = vca(b);
    lap("vca");
    let noise = noise(b);
    lap("noise");
    let modulation = modulation(b);
    lap("modulation");
    let external = external(b, lamp);
    lap("external input");
    let a440 = a440(b);
    lap("a-440");
    let glide = glide(b);
    lap("glide");
    let output = output(b);
    lap("output");
    Reference {
        rate: b.rate,
        pitch,
        waveforms,
        mixer,
        filter,
        contours,
        vca,
        noise,
        modulation,
        external,
        a440,
        glide,
        output,
    }
}

fn fall_text(d: &Fall) -> String {
    format!(
        "{:.4} V to {:.4} V: delay {:.4} s, 50 % {:.4} s, 90 % {:.4} s, 99 % {:.4} s; tau {} s toward {} V (worst {} V)",
        d.from,
        d.settled,
        d.delay,
        d.t50,
        d.t90,
        d.t99,
        opt(d.tau, 4),
        opt(d.end, 4),
        opt(d.worst, 4)
    )
}

fn law_text(l: &ChannelLaw) -> String {
    format!(
        "rms = v / ({:.5} {:+.5} v {:+.5} v^2), worst {:.3} %",
        l.c0, l.c1, l.c2, l.worst_pct
    )
}

/// Numbers to `digits` places, separated by spaces.
fn row(x: &[f64], digits: usize) -> String {
    x.iter()
        .map(|v| format!("{v:.digits$}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn opt(x: Option<f64>, digits: usize) -> String {
    x.map_or_else(|| "-".to_owned(), |v| format!("{v:.digits$}"))
}

fn poly_text(p: &Poly, var: &str, digits: usize) -> String {
    let mut s = String::new();
    for (i, c) in p.c.iter().enumerate() {
        let term = match i {
            0 => String::new(),
            1 => format!(" {var}"),
            _ => format!(" {var}^{i}"),
        };
        if i == 0 {
            let _ = write!(s, "{c:.digits$}");
        } else {
            let _ = write!(
                s,
                " {} {:.digits$}{term}",
                if *c < 0.0 { '-' } else { '+' },
                c.abs()
            );
        }
    }
    s
}

/// The results as text: each law with its constants and residual, then the tables.
pub fn report(r: &Reference) -> String {
    let mut o = String::new();
    let w = &mut o;
    let p = &r.pitch;
    let _ = writeln!(w, "# Reference at {} Hz\n", r.rate);
    let _ = writeln!(w, "## A. Pitch");
    let _ = writeln!(
        w,
        "oscillator 1, 8': Hz = {:.4} * 2^((key - 69) * {:.6} / 12), worst {:.2} cents (keys 24 to 108); 440 Hz at key {:.3}",
        p.hz_at_69, p.semitones_per_key, p.law_worst_cents, p.key_at_440
    );
    for k in &p.keys {
        let _ = writeln!(
            w,
            "  key {:>3}: {:>10.4} Hz  {:+.2} cents off the law",
            k.x, k.hz, k.cents
        );
    }
    let _ = writeln!(w, "RANGE at key 60:");
    for g in &p.ranges {
        let _ = writeln!(
            w,
            "  {:>4}: {:>10.4} Hz, {:.5} x 8' ({:+.2} cents from a power of two)",
            g.range,
            g.hz,
            g.ratio,
            cents(g.ratio, 2f64.powf(g.ratio.log2().round()))
        );
    }
    for (name, t, law) in [
        ("oscillator 2 FREQUENCY", &p.osc2, &p.osc2_law),
        ("oscillator 3 FREQUENCY", &p.osc3, &p.osc3_law),
        ("TUNE", &p.tune, &p.tune_law),
        ("PITCH wheel", &p.wheel, &p.wheel_law),
    ] {
        let _ = writeln!(
            w,
            "{name}: cents = {}, worst {:.2} cents",
            poly_text(law, "k", 3),
            law.worst
        );
        for a in t.iter() {
            let _ = writeln!(w, "  {:+.2}: {:>9.4} Hz  {:+9.2} cents", a.x, a.hz, a.cents);
        }
    }
    let _ = writeln!(w, "OSC. 3 CONTROL off, FREQUENCY 0.5:");
    for f in &p.osc3_free {
        let _ = writeln!(w, "  {:>3} key {}: {:.4} Hz", f.range, f.key, f.hz);
    }
    let _ = writeln!(w, "\n## B. Waveforms (key 33, VOLUME 2)");
    for v in &r.waveforms.waves {
        let _ = writeln!(
            w,
            "{}: {:.3} Hz, pp {:.5}, RMS {:.5}, mean {:+.6}, fundamental {:.5}{}{}",
            v.name,
            v.hz,
            v.pp,
            v.rms,
            v.mean,
            v.fundamental,
            v.duty.map_or(String::new(), |d| format!(", duty {:.4}", d)),
            v.ramp.map_or(String::new(), |d| format!(", ramp {d}"))
        );
        let _ = writeln!(w, "  harmonics 2..16, dB: {}", row(&v.harmonics_db, 1));
        let _ = writeln!(w, "  phases 2..16, degrees: {}", row(&v.phases_deg, 1));
        let _ = writeln!(w, "  shape: {}", row(&v.shape, 3));
    }
    let _ = writeln!(
        w,
        "the open filter and the output, sawtooth VOLUME 2 (dB against key 69):"
    );
    for f in &r.waveforms.flat {
        let _ = writeln!(
            w,
            "  {} key {} ({:.2} Hz): fundamental {:+.3} dB, RMS {:+.3} dB",
            f.range, f.key, f.hz, f.fundamental_db, f.rms_db
        );
    }
    let m = &r.mixer;
    let _ = writeln!(w, "\n## C. Mixer");
    for (name, l) in [
        ("VOLUME 0.1 to 0.5", &m.law),
        ("whole travel", &m.law_whole),
    ] {
        let _ = writeln!(
            w,
            "oscillator 1 sawtooth 110 Hz, fitted over {name}: {}",
            law_text(l)
        );
    }
    for l in &m.volume {
        let _ = writeln!(
            w,
            "  VOLUME {:.1}: RMS {:.5} ({:+.2} % from the first law)",
            l.knob, l.rms, l.error_pct
        );
    }
    let _ = writeln!(
        w,
        "stacks (oscillator 2 FREQUENCY {:.4}, oscillator 3 {:.4}):",
        m.osc2_knob, m.osc3_knob
    );
    for (v, (a, b2)) in m
        .osc2_alone
        .iter()
        .zip(&m.osc3_alone)
        .map(|(a, b2)| (a.0, (a.1, b2.1)))
    {
        let _ = writeln!(
            w,
            "  alone at {v:.1}: oscillator 2 {a:.5}, oscillator 3 {b2:.5}"
        );
    }
    for s in &m.stacks {
        let _ = writeln!(
            w,
            "  {} at {:.1}: RMS {:.5}, linear sum {:.5}, {:+.3} dB",
            s.oscillators, s.volume, s.rms, s.linear, s.compression_db
        );
    }
    let _ = writeln!(w, "triangle 110 Hz:");
    for d in &m.triangle {
        let _ = writeln!(
            w,
            "  VOLUME {:.1}: fundamental {:.5}, H2 {:.1} dB, H3 {:.2} dB",
            d.volume, d.fundamental, d.h2_db, d.h3_db
        );
    }
    let f = &r.filter;
    let _ = writeln!(w, "\n## D. Filter");
    let _ = writeln!(
        w,
        "self-oscillation (EMPHASIS 10): Hz = {:.2} * 2^({:.4} (CUTOFF - 0.5)), worst {:.1} cents (200 Hz to 8 kHz); log2 Hz = {}, worst {:.1} cents (every ring); none below CUTOFF {}",
        f.hz_at_half,
        f.octaves_per_knob,
        f.law_worst_cents,
        poly_text(&f.law_cubic, "c", 4),
        1200.0 * f.law_cubic.worst,
        opt(f.lowest, 2)
    );
    for g in &f.rings {
        let _ = writeln!(
            w,
            "  CUTOFF {:.2}: {} Hz, RMS {:.5}, peak {:.5}, H3 {} dB",
            g.cutoff,
            opt(g.hz, 2),
            g.rms,
            g.peak,
            opt(g.h3_db, 1)
        );
    }
    for t in &f.tracking {
        let _ = writeln!(
            w,
            "  KEYBOARD CONTROL {}{}: {} -> {:.4} octaves an octave, untracked at key {:.2}",
            if t.kbd1 { "1" } else { "" },
            if t.kbd2 { "2" } else { "" },
            t.hz.iter()
                .map(|(k, h)| format!("key {k} {h:.2} Hz"))
                .collect::<Vec<_>>()
                .join(", "),
            t.octaves_per_octave,
            t.untracked_at_key
        );
    }
    for t in &f.thresholds {
        let _ = writeln!(
            w,
            "threshold at CUTOFF {:.1}: EMPHASIS {} on the grid, {} bisected",
            t.cutoff,
            opt(t.emphasis, 3),
            opt(t.fine, 4)
        );
        for g in &t.steps {
            let _ = writeln!(
                w,
                "    {:.3}: {:.3e} -> {:.3e}{}",
                g.emphasis,
                g.rms_early,
                g.rms_late,
                if g.oscillates { "  oscillates" } else { "" }
            );
        }
    }
    let _ = writeln!(w, "AMOUNT OF CONTOUR from CUTOFF {:.2}:", f.amount_cutoff);
    for a in &f.amount {
        let _ = writeln!(
            w,
            "  SUSTAIN {:.1}, AMOUNT {:.2}: contour {:.4} V, {} Hz, {} octaves, as CUTOFF {}, {} of CUTOFF per volt per unit",
            a.sustain,
            a.amount,
            a.contour,
            opt(a.hz, 2),
            opt(a.octaves, 3),
            opt(a.cutoff_equivalent, 4),
            opt(a.cutoff_per_volt, 5)
        );
    }
    let _ = writeln!(
        w,
        "the open filter's noise response, dB against 100 Hz to 1 kHz: {}",
        f.open
            .iter()
            .map(|(h, d)| format!("{h:.0}:{d:.2}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    for s in &f.response {
        let _ = writeln!(
            w,
            "  CUTOFF {:.1} EMPHASIS {:.2}{}: passband {:+.2} dB, bass {:+.2} dB, corner {} Hz, peak {} Hz {} dB above, slope {} dB/octave",
            s.cutoff,
            s.emphasis,
            if s.oscillates { " (oscillates)" } else { "" },
            s.passband_db,
            s.bass_db,
            opt(s.corner_hz, 1),
            opt(s.peak_hz, 1),
            opt(s.peak_db, 2),
            opt(s.slope_db_per_octave, 1)
        );
        let _ = writeln!(
            w,
            "    {}",
            s.curve
                .iter()
                .map(|(h, d)| format!("{h:.0}:{d:.1}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
    let c = &r.contours;
    let _ = writeln!(w, "\n## E. Contours");
    for (name, s) in [("filter", &c.filter), ("loudness", &c.loudness)] {
        let _ = writeln!(
            w,
            "{name}: rest {:.4} V, peak {:.4} V at ATTACK 0, {:.4} V at ATTACK 2 to 10",
            s.rest, s.peak, s.plateau
        );
        for a in &s.attack {
            let _ = writeln!(
                w,
                "  ATTACK {:.1}: peak {:.4} V; 1 % {:.4} s, 10 % {:.4}, 50 % {:.4}, 90 % {:.4}, peak {:.4}; RC from {} s, tau {} s, toward {} of the rise, worst {}",
                a.knob,
                a.peak,
                a.t1,
                a.t10,
                a.t50,
                a.t90,
                a.t100,
                opt(a.start, 4),
                opt(a.tau, 4),
                opt(a.target, 4),
                opt(a.worst, 4)
            );
        }
        for d in &s.decay {
            let _ = writeln!(w, "  DECAY {:.1}: {}", d.knob, fall_text(d));
        }
        let _ = writeln!(
            w,
            "  SUSTAIN: volts = {}, worst {:.4} V",
            poly_text(&s.sustain_law, "k", 4),
            s.sustain_law.worst
        );
        for (k, v, x) in &s.sustain {
            let _ = writeln!(w, "    {k:.2}: {v:.4} V, {x:.4} of the rise");
        }
        for rl in &s.release {
            let _ = writeln!(
                w,
                "  release, DECAY switch {}, DECAY {:.1}: {}",
                if rl.decay_switch { "on" } else { "off" },
                rl.fall.knob,
                fall_text(&rl.fall)
            );
        }
        for k in &s.cut {
            let _ = writeln!(
                w,
                "  released in ATTACK 6's rise, DECAY switch {}: at {:.3} s, level {:.4}, highest after {:.4} ({:.4} s later); then {}",
                if k.decay_switch { "on" } else { "off" },
                k.released_at,
                k.level,
                k.highest,
                k.highest_after,
                fall_text(&k.fall)
            );
        }
    }
    let _ = writeln!(
        w,
        "retrigger after a gap: {}; a legato key retriggers: {}",
        c.retrigger
            .iter()
            .map(|g| format!(
                "{:.0} ms {}",
                g.gap * 1e3,
                if g.retriggered { "yes" } else { "no" }
            ))
            .collect::<Vec<_>>()
            .join(", "),
        c.legato_retriggers
    );
    let v = &r.vca;
    let _ = writeln!(w, "\n## F. VCA");
    for g in &v.points {
        let _ = writeln!(
            w,
            "  SUSTAIN {:.2}: contour {:.4} V, RMS {:.6}, gain {:.5} ({:.2} dB)",
            g.sustain,
            g.volts,
            g.rms,
            g.gain,
            db(g.gain)
        );
    }
    let _ = writeln!(
        w,
        "  no key: contour {:.4} V, RMS {:.3e} ({:.1} dB), fundamental {:.3e}",
        v.rest_volts, v.bleed_rms, v.bleed_db, v.bleed_fundamental
    );
    let _ = writeln!(
        w,
        "  through ATTACK 10's rise (V:gain): {}",
        v.sweep
            .iter()
            .map(|(a, g)| format!("{a:.1}:{g:.4}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let n = &r.noise;
    let _ = writeln!(w, "\n## G. Noise");
    for l in &n.levels {
        let _ = writeln!(
            w,
            "  {} at {:.1}: RMS {:.5}, slope {:+.2} dB/octave, sawtooth {:.5}, {:+.2} dB against it",
            if l.pink { "pink" } else { "white" },
            l.volume,
            l.rms,
            l.slope_db_per_octave,
            l.saw_rms,
            l.against_saw_db
        );
        let _ = writeln!(
            w,
            "    {}",
            l.bands
                .iter()
                .map(|(h, d)| format!("{h:.0}:{d:.1}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
    for (name, l, t) in [
        ("white", &n.white_law, &n.white_volume),
        ("pink", &n.pink_law, &n.pink_volume),
    ] {
        let _ = writeln!(
            w,
            "  {name} VOLUME: {}: {}",
            law_text(l),
            t.iter()
                .map(|(k, x)| format!("{k:.1}:{x:.5}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
    let md = &r.modulation;
    let _ = writeln!(
        w,
        "\n## H. Modulation (unmodulated: oscillator 1 {:.3} Hz, filter {:.2} Hz)",
        md.pitch_hz, md.filter_hz
    );
    for s in &md.pitch {
        let _ = writeln!(
            w,
            "  pitch, wheel {:.2}: LFO {} Hz, {:+.1} / {:+.1} cents, depth {:.1}, centre {:+.1}, mean {:+.1}",
            s.wheel,
            opt(s.lfo_hz, 4),
            s.up,
            s.down,
            s.depth,
            s.centre,
            s.mean
        );
        let _ = writeln!(w, "    {}", row(&s.shape, 0));
    }
    for s in &md.filter {
        let _ = writeln!(
            w,
            "  filter, wheel {:.2}: LFO {} Hz, {:+.3} / {:+.3} octaves, depth {:.3}, centre {:+.3}, mean {:+.3}",
            s.wheel,
            opt(s.lfo_hz, 4),
            s.up,
            s.down,
            s.depth,
            s.centre,
            s.mean
        );
        let _ = writeln!(w, "    {}", row(&s.shape, 3));
    }
    for j in &md.noise {
        let _ = writeln!(
            w,
            "  noise, wheel {:.1}: pitch RMS {:.2} cents (mean {:+.2}), filter RMS {:.4} octaves (mean {:+.4})",
            j.wheel,
            j.pitch_rms_cents,
            j.pitch_mean_cents,
            j.filter_rms_octaves,
            j.filter_mean_octaves
        );
    }
    let _ = writeln!(w, "\n## I. External input (220 Hz)");
    for i in &r.external.inputs {
        let _ = writeln!(
            w,
            "  VOLUME {:.2}, {:.4}: fundamental {:.5}, gain {:.4}, THD {:.2} %, peak {:.5}, lamp max {:.3} mean {:.3}",
            i.volume,
            i.amplitude,
            i.fundamental,
            i.gain,
            i.thd_pct,
            i.peak,
            i.lamp_max,
            i.lamp_mean
        );
    }
    let a = &r.a440;
    let _ = writeln!(
        w,
        "\n## J. A-440: {:.3} Hz, RMS {:.5}, fundamental {:.5}; harmonics 2..10 dB: {}",
        a.hz,
        a.rms,
        a.fundamental,
        row(&a.harmonics_db, 1)
    );
    let _ = writeln!(w, "  shape: {}", row(&a.shape, 3));
    let _ = writeln!(w, "\n## K. Glide");
    for s in &r.glide.slides {
        let _ = writeln!(
            w,
            "  GLIDE {:.1}, {} to {} {}: delay {:.4} s, half {:.4} s, within a semitone {:.4} s, 10 cents {:.4} s; middle {} semitones/s (worst {}); arrival tau {} s (worst {}); then within {:.2} cents",
            s.knob,
            s.from,
            s.to,
            if s.legato { "legato" } else { "after a gap" },
            s.delay,
            s.to_half,
            s.to_semitone,
            s.to_10_cents,
            opt(s.rate, 2),
            opt(s.rate_worst, 3),
            opt(s.tau, 5),
            opt(s.tau_worst, 4),
            s.settle_cents
        );
    }
    let u = &r.output;
    let _ = writeln!(
        w,
        "\n## L. Output\n  default panel, key 57: RMS {:.5}, peak {:.5}; at rest: mean {:+.3e}, RMS {:.3e}; the measurement panel at rest: mean {:+.3e}, RMS {:.3e}",
        u.default_rms,
        u.default_peak,
        u.default_rest_mean,
        u.default_rest_rms,
        u.measurement_rest_mean,
        u.measurement_rest_rms
    );
    o
}
