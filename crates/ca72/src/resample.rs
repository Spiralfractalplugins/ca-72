//! Band-limiting helpers: band-limited steps, impulses and corners at sub-sample times
//! (BLEP and BLAMP, from one windowed sinc), and halfband decimation from the oversampled
//! rate to the output rate.

use std::sync::OnceLock;

/// The band-limiting kernel's reach either side of an event, samples: a corrected line's
/// values come out this many samples late (decisions.md R-BLEP; the two-point polyBLEP it
/// replaced held one).
pub const BLEP_DELAY: usize = 16;

/// The kernel: a low-pass at 0.48 of the rate, Kaiser-windowed (beta 8) to 32 taps, linear
/// phase; tabulated at 256 points a sample. Chosen by the lab's sweep (R-BLEP): the least
/// aliasing of the sawtooth from 20 Hz to 20 kHz at 44.1 and 48 kHz with every harmonic below
/// 20 kHz within about a decibel of the oscillator's at 8x.
const BLEP_BETA: f64 = 8.0;
const BLEP_CUTOFF: f64 = 0.48;
pub const BLEP_POINTS: usize = 256;

/// The widest rounded corner ([`BlepLine::fillet`]) corrected, samples; a wider one is left
/// as it is sampled (it then has little above the band).
pub const MAX_FILLET: f64 = 24.0;

/// The kernel's tables over `x`, the samples from an event to the sample corrected, from
/// `-BLEP_DELAY` to `BLEP_DELAY` at [`BLEP_POINTS`] a sample: the band-limited impulse (unit
/// area), step (0 to 1) and ramp (its slope 0 to 1 a sample), and the ramp's correction
/// integrated (for rounded corners); the step and ramp also in single precision, for the
/// light model.
#[derive(Debug)]
pub struct BlepTables {
    imp: Vec<f64>,
    step: Vec<f64>,
    ramp: Vec<f64>,
    ramp_sum: Vec<f64>,
    ramp_total: f64,
    pub step32: Vec<f32>,
    pub ramp32: Vec<f32>,
}

/// The kernel's tables, made once a process (with `libm`, so the same on every platform).
pub fn blep_tables() -> &'static BlepTables {
    static T: OnceLock<BlepTables> = OnceLock::new();
    T.get_or_init(BlepTables::new)
}

impl BlepTables {
    fn new() -> BlepTables {
        use core::f64::consts::PI;
        // The impulse at G points a sample, integrated there, and kept at P.
        const G: usize = 4 * BLEP_POINTS;
        let k = BLEP_DELAY as f64;
        let n = 2 * BLEP_DELAY * G;
        let i0b = bessel_i0(BLEP_BETA);
        let h: Vec<f64> = (0..=n)
            .map(|i| {
                let x = -k + i as f64 / G as f64;
                let r = x / k;
                if r.abs() >= 1.0 {
                    return 0.0;
                }
                let w = bessel_i0(BLEP_BETA * libm::sqrt(1.0 - r * r)) / i0b;
                let t = 2.0 * BLEP_CUTOFF * x;
                let s = if t == 0.0 {
                    1.0
                } else {
                    libm::sin(PI * t) / (PI * t)
                };
                2.0 * BLEP_CUTOFF * s * w
            })
            .collect();
        let g = G as f64;
        let area = (h.iter().sum::<f64>() - 0.5 * (h[0] + h[n])) / g;
        let h: Vec<f64> = h.iter().map(|v| v / area).collect();
        let integrate = |v: &[f64]| {
            let mut out = vec![0.0; v.len()];
            for i in 1..v.len() {
                out[i] = out[i - 1] + 0.5 * (v[i - 1] + v[i]) / g;
            }
            out
        };
        let step = integrate(&h);
        let ramp = integrate(&step);
        // The ramp's correction (the band-limited ramp less the ramp), and its integral.
        let residual: Vec<f64> = ramp
            .iter()
            .enumerate()
            .map(|(i, r)| r - (-k + i as f64 / g).max(0.0))
            .collect();
        let ramp_sum = integrate(&residual);
        let keep =
            |v: &[f64]| -> Vec<f64> { (0..=n / 4).map(|i| v[i * (G / BLEP_POINTS)]).collect() };
        let (step, ramp) = (keep(&step), keep(&ramp));
        BlepTables {
            imp: keep(&h),
            step32: step.iter().map(|&v| v as f32).collect(),
            ramp32: ramp.iter().map(|&v| v as f32).collect(),
            step,
            ramp,
            ramp_total: ramp_sum[n],
            ramp_sum: keep(&ramp_sum),
        }
    }

    /// `v` at `x` samples from an event (`v`'s values beyond its ends held).
    #[inline]
    fn at(v: &[f64], x: f64) -> f64 {
        let u = (x + BLEP_DELAY as f64) * BLEP_POINTS as f64;
        if u <= 0.0 {
            return v[0];
        }
        let last = v.len() - 1;
        let i = u as usize;
        if i >= last {
            return v[last];
        }
        let f = u - i as f64;
        v[i] + (v[i + 1] - v[i]) * f
    }

    /// A rounded corner's correction at `x` for a slope that grows by one a sample evenly
    /// over `w` samples from `x` = 0: the ramp's correction averaged over the width.
    pub(crate) fn fillet(&self, x: f64, w: f64) -> f64 {
        let sum = |x: f64| {
            if x <= -(BLEP_DELAY as f64) {
                0.0
            } else if x >= BLEP_DELAY as f64 {
                self.ramp_total
            } else {
                Self::at(&self.ramp_sum, x)
            }
        };
        (sum(x) - sum(x - w)) / w
    }
}

/// Samples a line holds: more than the kernel's reach either side and the widest fillet.
const RING: usize = 64;

/// A signal with its discontinuities band-limited by the kernel ([`blep_tables`]): steps,
/// impulses, corners and rounded corners at sub-sample times, each spread over the samples
/// [`BLEP_DELAY`] either side of it. Values come out [`BLEP_DELAY`] samples late, so that an
/// event's correction can still reach the samples before it.
#[derive(Debug, Clone)]
pub struct BlepLine {
    tables: &'static BlepTables,
    ring: [f64; RING],
    /// The coming sample's place in the ring.
    c: usize,
}

impl Default for BlepLine {
    fn default() -> Self {
        BlepLine {
            tables: blep_tables(),
            ring: [0.0; RING],
            c: 0,
        }
    }
}

impl BlepLine {
    /// Starts again from silence.
    pub fn clear(&mut self) {
        self.ring = [0.0; RING];
        self.c = 0;
    }

    /// Adds `f(table index, its fraction, j)` to each sample `j` from the coming one that
    /// the kernel reaches from an event at `frac` of the way from the previous sample to the
    /// coming one (0 < frac <= 1): `x` = `j` + `d` samples after it, `d` = 1 - `frac`.
    #[inline]
    fn spread(&mut self, frac: f64, f: impl Fn(usize, f64, isize, f64) -> f64) {
        let d = (1.0 - frac).clamp(0.0, 1.0);
        let u = d * BLEP_POINTS as f64;
        let k = (u as usize).min(BLEP_POINTS - 1);
        let fr = u - k as f64;
        let reach = BLEP_DELAY as isize;
        for j in -reach..reach {
            let i = (j + reach) as usize * BLEP_POINTS + k;
            let slot = (self.c as isize + j).rem_euclid(RING as isize) as usize;
            self.ring[slot] += f(i, fr, j, j as f64 + d);
        }
    }

    /// Records a step of `delta` at `frac` of the way from the previous sample to the one
    /// about to be pushed (0 < frac <= 1).
    pub fn step(&mut self, delta: f64, frac: f64) {
        let t = &self.tables.step;
        self.spread(frac, |i, f, j, _| {
            let v = t[i] + (t[i + 1] - t[i]) * f;
            delta * (v - if j >= 0 { 1.0 } else { 0.0 })
        });
    }

    /// Records an impulse of `area` (value x samples) at `frac` of the way from the previous
    /// sample to the coming one: the limit of two opposite steps closing together (its area
    /// is kept).
    pub fn impulse(&mut self, area: f64, frac: f64) {
        let t = &self.tables.imp;
        self.spread(frac, |i, f, _, _| area * (t[i] + (t[i + 1] - t[i]) * f));
    }

    /// Records a corner at `frac`: the slope grows by `slope` (value a sample) there.
    pub fn ramp(&mut self, slope: f64, frac: f64) {
        let t = &self.tables.ramp;
        self.spread(frac, |i, f, _, x| {
            slope * (t[i] + (t[i + 1] - t[i]) * f - x.max(0.0))
        });
    }

    /// Records a rounded corner: the slope grows by `slope` (value a sample) evenly over
    /// `width` samples from `frac` (none wider than [`MAX_FILLET`]).
    pub fn fillet(&mut self, slope: f64, frac: f64, width: f64) {
        if width > MAX_FILLET {
            return;
        }
        if width < 1e-3 {
            return self.ramp(slope, frac);
        }
        let d = (1.0 - frac).clamp(0.0, 1.0);
        let reach = BLEP_DELAY as isize;
        let hi = (reach as f64 + width - d).floor() as isize;
        for j in -reach..=hi {
            let x = j as f64 + d;
            let slot = (self.c as isize + j).rem_euclid(RING as isize) as usize;
            self.ring[slot] += slope * self.tables.fillet(x, width);
        }
    }

    /// Pushes the naive value of the new sample; returns the sample [`BLEP_DELAY`] before it,
    /// corrected.
    #[inline]
    pub fn push(&mut self, naive: f64) -> f64 {
        self.ring[self.c] += naive;
        let o = (self.c + RING - BLEP_DELAY) % RING;
        let out = self.ring[o];
        self.ring[o] = 0.0;
        self.c = (self.c + 1) % RING;
        out
    }
}

fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let y = x * x / 4.0;
    for k in 1..60 {
        term *= y / (k as f64 * k as f64);
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
    }
    sum
}

/// Kaiser-windowed halfband low-pass taps (odd length), for decimation by two.
pub fn halfband_taps(len: usize, beta: f64) -> Vec<f64> {
    let n = len | 1;
    let m = (n / 2) as f64;
    let i0b = bessel_i0(beta);
    (0..n)
        .map(|i| {
            let k = i as f64 - m;
            let sinc = if k == 0.0 {
                0.5
            } else {
                crate::ulp::sin(core::f64::consts::PI * k / 2.0) / (core::f64::consts::PI * k)
            };
            let r = k / m;
            sinc * bessel_i0(beta * libm::sqrt((1.0 - r * r).max(0.0))) / i0b
        })
        .collect()
}

/// A decimator by two: a halfband FIR evaluated every other input sample.
#[derive(Debug, Clone)]
pub struct Halfband {
    taps: Vec<f64>,
    line: Vec<f64>,
    pos: usize,
    phase: bool,
    /// Sparse ([`Halfband::sparse`]): the taps an even distance from the centre, which a
    /// halfband's are zero but for rounding (about 1e-17 of the rest), exactly zero and
    /// skipped.
    sparse: bool,
}

impl Halfband {
    /// A windowed low-pass at `cutoff` cycles per input sample. HI's split VCA
    /// uses 0.1875 before decimating: the ordinary halfband admits too much energy
    /// above the audio band into the 2x nonlinear solve.
    pub fn lowpass(len: usize, beta: f64, cutoff: f64) -> Halfband {
        if cutoff == 0.25 {
            return Self::sparse(len, beta);
        }
        let mut h = Self::new(len, beta);
        let m = (len / 2) as f64;
        let i0b = bessel_i0(beta);
        for (i, t) in h.taps.iter_mut().enumerate() {
            let k = i as f64 - m;
            let sinc = if k == 0.0 {
                2.0 * cutoff
            } else {
                libm::sin(2.0 * core::f64::consts::PI * cutoff * k) / (core::f64::consts::PI * k)
            };
            let r = k / m;
            *t = sinc * bessel_i0(beta * libm::sqrt((1.0 - r * r).max(0.0))) / i0b;
        }
        h
    }

    pub fn new(len: usize, beta: f64) -> Halfband {
        let taps = halfband_taps(len, beta);
        let n = taps.len();
        Halfband {
            taps,
            line: vec![0.0; 2 * n],
            pos: 0,
            phase: false,
            sparse: false,
        }
    }

    /// [`Halfband::new`] with the taps an even distance from the centre (their sin(pi k /
    /// 2) is zero but for rounding) exactly zero and skipped: a little under half the work
    /// (Potato).
    pub fn sparse(len: usize, beta: f64) -> Halfband {
        let mut h = Halfband::new(len, beta);
        let m = h.taps.len() / 2;
        for (i, t) in h.taps.iter_mut().enumerate() {
            if i != m && (i as isize - m as isize) % 2 == 0 {
                *t = 0.0;
            }
        }
        h.sparse = true;
        h
    }

    /// The window's sum with the taps: in a sparse halfband only the taps an odd distance
    /// from the centre and the centre's.
    fn sum(&self, w: &[f64]) -> f64 {
        if self.sparse {
            let m = self.taps.len() / 2;
            let mut acc = 0.0;
            for i in ((1 - m % 2)..self.taps.len()).step_by(2) {
                acc += self.taps[i] * w[i];
            }
            acc + self.taps[m] * w[m]
        } else {
            let mut acc = 0.0;
            for (t, v) in self.taps.iter().zip(w) {
                acc += t * v;
            }
            acc
        }
    }

    /// Group delay in input samples.
    pub fn delay(&self) -> usize {
        self.taps.len() / 2
    }

    /// Pushes one input sample; returns an output sample every second call.
    pub fn push(&mut self, x: f64) -> Option<f64> {
        let n = self.taps.len();
        // A doubled ring buffer: the window [pos, pos + n) is always contiguous.
        self.line[self.pos] = x;
        self.line[self.pos + n] = x;
        self.pos = if self.pos == 0 { n - 1 } else { self.pos - 1 };
        self.phase = !self.phase;
        if !self.phase {
            return None;
        }
        let w = &self.line[self.pos + 1..self.pos + 1 + n];
        Some(self.sum(w))
    }

    /// Filters one sample at the input rate (every sample produces one): for
    /// interpolation, where the halfband runs at the higher rate.
    pub fn filter(&mut self, x: f64) -> f64 {
        let n = self.taps.len();
        self.line[self.pos] = x;
        self.line[self.pos + n] = x;
        let w = &self.line[self.pos..self.pos + n];
        let acc = self.sum(w);
        self.pos = if self.pos == 0 { n - 1 } else { self.pos - 1 };
        acc
    }

    /// [`Halfband::filter`] of a stuffed zero in a sparse halfband: its window then holds
    /// samples an odd distance from the newest only, which meet no tap but the centre's.
    fn filter_stuffed_zero(&mut self) -> f64 {
        let n = self.taps.len();
        self.line[self.pos] = 0.0;
        self.line[self.pos + n] = 0.0;
        let m = n / 2;
        let acc = if m % 2 == 1 {
            self.taps[m] * self.line[self.pos + m]
        } else {
            self.sum(&self.line[self.pos..self.pos + n])
        };
        self.pos = if self.pos == 0 { n - 1 } else { self.pos - 1 };
        acc
    }

    pub fn reset(&mut self) {
        self.line.iter_mut().for_each(|v| *v = 0.0);
        self.pos = 0;
        self.phase = false;
    }
}

/// Taps in a decimator's halfbands before its last.
const FIRST_HALFBANDS: usize = 23;

/// Taps in a decimator's last halfband ([`Decimator::new`]): it passes to 0.417 of the output
/// rate and stops from 0.583 (20 and 28 kHz at 48 kHz) by about 100 dB.
pub const LAST_HALFBAND: usize = 79;

/// Taps in the oscillators' decimators' last halfband ([`Decimator::for_oscillators`]): it
/// passes to 0.4535 of the output rate and stops from 0.5465 (20 and 24.1 kHz at 44.1 kHz), so
/// that nothing folds into 20 Hz to 20 kHz at 44.1 kHz either (79 taps stop only from 25.7
/// kHz there; decisions.md R-BLEP, the owner's choice of 2026-10-16).
pub const OSCILLATOR_LAST_HALFBAND: usize = 139;

/// Decimation by 2^stages: a short halfband first, a long one last.
#[derive(Debug, Clone)]
pub struct Decimator {
    stages: Vec<Halfband>,
}

impl Decimator {
    /// `factor` is 1, 2, 4 or 8. The last stage passes 0..0.417 of its output Nyquist
    /// (20 kHz at 48 kHz) and stops from 0.583 (28 kHz) down by about 100 dB.
    pub fn new(factor: usize) -> Decimator {
        Self::with(factor, Halfband::new)
    }

    /// [`Decimator::new`] with sparse halfbands ([`Halfband::sparse`]; Potato).
    pub fn sparse(factor: usize) -> Decimator {
        Self::with(factor, Halfband::sparse)
    }

    /// [`Decimator::new`] for the oscillators' outputs: its last stage
    /// [`OSCILLATOR_LAST_HALFBAND`] taps long (decisions.md R-BLEP; at 4x, ULTRA's).
    pub fn for_oscillators(factor: usize) -> Decimator {
        Self::with_last(factor, OSCILLATOR_LAST_HALFBAND, Halfband::new)
    }

    fn with(factor: usize, make: fn(usize, f64) -> Halfband) -> Decimator {
        Self::with_last(factor, LAST_HALFBAND, make)
    }

    fn with_last(factor: usize, last: usize, make: fn(usize, f64) -> Halfband) -> Decimator {
        let stages = stages_len(factor);
        let stages = (0..stages)
            .map(|i| {
                if i + 1 == stages {
                    make(last, 10.06)
                } else {
                    make(FIRST_HALFBANDS, 10.06)
                }
            })
            .collect();
        Decimator { stages }
    }

    /// The group delay in output samples of a decimator by `factor` whose last stage is
    /// `last` taps long, as [`Decimator::delay`] gives it (nothing made).
    pub fn delay_of(factor: usize, last: usize) -> f64 {
        let n = stages_len(factor);
        (0..n)
            .map(|k| {
                let len = if k + 1 == n { last } else { FIRST_HALFBANDS };
                ((len | 1) / 2) as f64 / f64::from(1u32 << (n - k))
            })
            .sum()
    }

    pub fn push(&mut self, x: f64) -> Option<f64> {
        let mut v = x;
        for s in &mut self.stages {
            v = s.push(v)?;
        }
        Some(v)
    }

    /// Group delay in output samples: each stage's delay at its own input rate, which is
    /// 2^(stages left, itself included) times the output rate.
    pub fn delay(&self) -> f64 {
        let n = self.stages.len();
        self.stages
            .iter()
            .enumerate()
            .map(|(k, s)| s.delay() as f64 / f64::from(1u32 << (n - k)))
            .sum()
    }

    pub fn reset(&mut self) {
        self.stages.iter_mut().for_each(Halfband::reset);
    }
}

/// Interpolation by 2^stages: zero-stuffing and the same halfband filters as [`Decimator`]
/// in the reverse order (long first), with the gain of two each stage needs.
#[derive(Debug, Clone)]
pub struct Interpolator {
    stages: Vec<Halfband>,
    factor: usize,
}

impl Interpolator {
    /// `factor` is 1, 2, 4 or 8.
    pub fn new(factor: usize) -> Interpolator {
        Self::with(factor, Halfband::new)
    }

    /// [`Interpolator::new`] with sparse halfbands ([`Halfband::sparse`]; Potato): each
    /// stuffed zero's output takes one tap.
    pub fn sparse(factor: usize) -> Interpolator {
        Self::with(factor, Halfband::sparse)
    }

    fn with(factor: usize, make: fn(usize, f64) -> Halfband) -> Interpolator {
        let stages = stages_len(factor);
        let stages = (0..stages)
            .map(|i| {
                if i == 0 {
                    make(79, 10.06)
                } else {
                    make(23, 10.06)
                }
            })
            .collect();
        Interpolator {
            stages,
            factor: 1 << (stages_len(factor)),
        }
    }

    pub fn factor(&self) -> usize {
        self.factor
    }

    /// One input sample to `factor` output samples.
    pub fn push(&mut self, x: f64, out: &mut [f64]) {
        let mut buf = [0.0f64; 8];
        let mut n = 1usize;
        buf[0] = x;
        for s in &mut self.stages {
            let mut next = [0.0f64; 8];
            for i in 0..n {
                next[2 * i] = s.filter(2.0 * buf[i]);
                next[2 * i + 1] = if s.sparse {
                    s.filter_stuffed_zero()
                } else {
                    s.filter(0.0)
                };
            }
            n *= 2;
            buf = next;
        }
        out[..n].copy_from_slice(&buf[..n]);
    }

    /// Group delay in input samples.
    pub fn delay(&self) -> f64 {
        self.stages
            .iter()
            .enumerate()
            .map(|(k, s)| s.delay() as f64 / f64::from(1u32 << (k + 1)))
            .sum()
    }

    /// The group delay in input samples of an interpolator by `factor`, as
    /// [`Interpolator::delay`] gives it (nothing made).
    pub fn delay_of(factor: usize) -> f64 {
        (0..stages_len(factor))
            .map(|k| {
                let len = if k == 0 {
                    LAST_HALFBAND
                } else {
                    FIRST_HALFBANDS
                };
                ((len | 1) / 2) as f64 / f64::from(1u32 << (k + 1))
            })
            .sum()
    }

    pub fn reset(&mut self) {
        self.stages.iter_mut().for_each(Halfband::reset);
    }
}

/// The halfband stages for an oversampling factor; any other factor than 1, 2, 4 or 8 is a
/// caller's error, not something to round.
fn stages_len(factor: usize) -> u32 {
    match factor {
        1 => 0,
        2 => 1,
        4 => 2,
        8 => 3,
        _ => panic!("oversampling factor {factor}: 1, 2, 4 or 8"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kernels_corrections_end_where_they_reach() {
        let t = blep_tables();
        let k = BLEP_DELAY as f64;
        // The step rises from 0 to 1, the ramp's slope from 0 to 1, the impulse's area is 1.
        assert!(BlepTables::at(&t.step, -k).abs() < 1e-12);
        assert!((BlepTables::at(&t.step, k) - 1.0).abs() < 1e-9);
        assert!(BlepTables::at(&t.ramp, -k).abs() < 1e-12);
        assert!((BlepTables::at(&t.ramp, k) - k).abs() < 1e-6);
        let area: f64 = (0..t.imp.len() - 1)
            .map(|i| 0.5 * (t.imp[i] + t.imp[i + 1]) / BLEP_POINTS as f64)
            .sum();
        assert!((area - 1.0).abs() < 1e-6, "{area}");
        // A rounded corner's correction ends too.
        assert!(t.fillet(k + 3.0, 2.0).abs() < 1e-9);
        assert!(t.fillet(-k, 2.0).abs() < 1e-12);
    }

    #[test]
    fn a_decimators_delay_is_known_before_it_is_made() {
        for factor in [1, 2, 4, 8] {
            let d = Decimator::new(factor).delay();
            assert_eq!(Decimator::delay_of(factor, LAST_HALFBAND), d);
            let d = Decimator::for_oscillators(factor).delay();
            assert_eq!(Decimator::delay_of(factor, OSCILLATOR_LAST_HALFBAND), d);
        }
        for factor in [1, 2, 4, 8] {
            assert_eq!(
                Interpolator::delay_of(factor),
                Interpolator::new(factor).delay()
            );
        }
        // At ULTRA's 4x the longer last stage is 15 samples later.
        assert_eq!(
            Decimator::delay_of(4, OSCILLATOR_LAST_HALFBAND)
                - Decimator::delay_of(4, LAST_HALFBAND),
            15.0
        );
    }

    #[test]
    fn a_corrected_step_settles_on_the_step() {
        // A step of 1 between samples, wherever it falls: the corrected line comes out
        // [`BLEP_DELAY`] late, at 0 before the kernel's reach and 1 after it, and its samples
        // sum to the continuous step's area, centred on the step's time (the naive samples'
        // and 1/2 - `frac` more).
        for frac in [0.01, 0.25, 0.5, 0.99, 1.0] {
            let mut l = BlepLine::default();
            let mut out = Vec::new();
            for n in 0..80 {
                if n == 40 {
                    l.step(1.0, frac);
                }
                out.push(l.push(if n >= 40 { 1.0 } else { 0.0 }));
            }
            assert!(out[..40 - 1].iter().all(|v| v.abs() < 1e-12), "{frac}");
            assert!(
                out[40 + 2 * BLEP_DELAY..]
                    .iter()
                    .all(|v| (v - 1.0).abs() < 1e-9)
            );
            let naive = (80 - BLEP_DELAY - 40) as f64;
            let sum: f64 = out.iter().sum();
            assert!(
                (sum - naive - (0.5 - frac)).abs() < 1e-5,
                "{frac}: {sum} {naive}"
            );
        }
    }
}
