//! The preamplifier's circuit with its linear nodes eliminated ahead of Newton's method
//! (decisions.md R-PREAMP-SOLVE): the equations of the [`Circuit`] it is made from, with its
//! trapezoidal rule, its predictor, its junction limiting and its stopping rule, and the
//! same iterates to rounding, without its generality's cost.
//!
//! Only the transistors' nine internal nodes touch anything nonlinear; the other ten fall
//! into groups that reach them only through the transistors' series resistances: N78 and B27
//! behind Q27's base, C27 and N64 between Q27's collector and Q33's base, TAIL between the
//! pair's emitters, and B32, N61, N62, AMP and OUT between Q33's collector and Q32's base.
//! Newton's step solves each group's linear equations exactly, so its voltages after a step
//! are a fixed function of the transistor nodes' ([`Group`]: a move for the substep, from the
//! accepted point and the source, and slopes that change only with the step's length, R9 or
//! SW10), whatever they were before. The iterates are therefore those of a system over the
//! nine alone, where the groups leave a constant matrix (`Reduced::joined`) that the
//! transistors' stamps add to. The groups' voltages are worked out at each iterate only
//! because the stopping rule looks at every node.
//!
//! That system is a ring (each transistor's three nodes joined, one group from each to the
//! next), so it eliminates in about a hundred operations ([`solve`]), in an order fixed in
//! advance, without pivoting: first the collectors and emitters, whose 1 and 0.5 ohm
//! resistances dominate their rows, then the bases. The tests hold it to the general
//! solver's pivoted solve, there as far past the panel's reach as the circuit converges at
//! all. Everything changes in place, without allocating: a move of R9 or SW10 on the audio
//! thread works out one group again, a shorter step all of them.

use super::{AMP, B27, B32, C27, GND, HELD, N10, N61, N62, N64, N78, OUT, P10, SRC, TAIL};
use crate::devices::BjtAt;
use crate::mna::{
    Circuit, Depletion, DepletionConsts, GMIN, Node, Part, diffusion_charge, iteration_bin, pnjlim,
};

const LINEAR: usize = 10;
const INNER: usize = 9;

/// The transistor nodes' places: collector, base and emitter of Q27, Q32 and Q33, as the
/// circuit adds them, after the linear nodes.
const C1: usize = 0;
const B1: usize = 1;
const E1: usize = 2;
const C2: usize = 3;
const B2: usize = 4;
const E2: usize = 5;
const C3: usize = 6;
const B3: usize = 7;
const E3: usize = 8;

type Matrix = [[f64; INNER]; INNER];

/// The most iterations of a step, as the circuit's.
const ITERATIONS: usize = 100;

/// Inverts a group's matrix (a symmetric conductance matrix, so its pivots are safe).
fn invert<const M: usize>(mut a: [[f64; M]; M]) -> [[f64; M]; M] {
    let mut x = [[0.0; M]; M];
    for (k, row) in x.iter_mut().enumerate() {
        row[k] = 1.0;
    }
    for p in 0..M {
        let d = 1.0 / a[p][p];
        for c in 0..M {
            a[p][c] *= d;
            x[p][c] *= d;
        }
        for r in (0..M).filter(|&r| r != p) {
            let m = a[r][p];
            for c in 0..M {
                a[r][c] -= m * a[p][c];
                x[r][c] -= m * x[p][c];
            }
        }
    }
    x
}

/// `M` linear nodes that reach `T` transistor nodes, each through a series resistance: their
/// voltages as a function of those nodes', and what they make of each of those resistances.
///
/// Everything here is a difference, never a large voltage or charge. The circuit's rounding
/// is about 1e-13 V at AMP, and the plainer forms of the same algebra lose four digits of it.
/// A group's voltage worked out and taken from its transistor node's puts an ulp of 12 V through
/// an ohm into that node's row alone, with no row at the far end to balance it as the circuit
/// has: into the 27K at C27, a nanovolt at AMP. And C26's history as a charge over the step,
/// an ampere and a half, rounds the same way at every step: its charge drifts, and the
/// feedback multiplies that by 232. So a substep starts from the accepted point's own
/// residual ([`Group::start`]), and the series currents are the network they are, the
/// neighbours' differences and each node's slack ([`Group::currents`]).
#[derive(Debug, Clone, Copy)]
struct Group<const M: usize, const T: usize> {
    /// Each transistor node's series conductance and the group's node it reaches.
    series: [f64; T],
    reach: [usize; T],
    /// The group's conductance matrix's inverse.
    inverse: [[f64; M]; M],
    /// The voltages' slopes against each transistor node.
    slope: [[f64; T]; M],
    /// What the slopes leave of 1, each node's share of the supplies and capacitors, worked
    /// out without the subtraction.
    slack: [f64; M],
    /// The accepted point: the group's voltages, the transistor nodes', and each series
    /// resistance's voltage.
    from: [f64; M],
    from_t: [f64; T],
    across: [f64; T],
    /// The voltages' change with the transistor nodes where they were.
    moved: [f64; M],
}

impl<const M: usize, const T: usize> Group<M, T> {
    fn new(series: [f64; T], reach: [usize; T]) -> Self {
        Group {
            series,
            reach,
            inverse: [[0.0; M]; M],
            slope: [[0.0; T]; M],
            slack: [0.0; M],
            from: [0.0; M],
            from_t: [0.0; T],
            across: [0.0; T],
            moved: [0.0; M],
        }
    }

    /// From the conductances joining its nodes and from each node to the supplies, ground
    /// and the capacitors' other sides.
    fn set<const L: usize>(&mut self, links: [(usize, usize, f64); L], shunt: [f64; M]) {
        let mut a = [[0.0; M]; M];
        for k in 0..M {
            a[k][k] = shunt[k];
        }
        for (x, y, g) in links {
            a[x][x] += g;
            a[y][y] += g;
            a[x][y] -= g;
            a[y][x] -= g;
        }
        for (&at, g) in self.reach.iter().zip(&self.series) {
            a[at][at] += g;
        }
        self.inverse = invert(a);
        for ((row, s), slack) in self
            .inverse
            .iter()
            .zip(&mut self.slope)
            .zip(&mut self.slack)
        {
            for j in 0..T {
                s[j] = row[self.reach[j]] * self.series[j];
            }
            *slack = row.iter().zip(&shunt).map(|(x, y)| x * y).sum();
        }
    }

    /// A substep from the accepted point (the group's voltages `v`, the transistor nodes'
    /// `t`), where the source and the capacitors' currents leave each node the current
    /// `f` over.
    #[inline(always)]
    fn start(&mut self, f: [f64; M], v: [f64; M], t: [f64; T]) {
        for (w, row) in self.moved.iter_mut().zip(&self.inverse) {
            *w = 0.0;
            for (x, y) in row.iter().zip(&f) {
                *w -= x * y;
            }
        }
        for i in 0..T {
            self.across[i] = t[i] - v[self.reach[i]];
        }
        (self.from, self.from_t) = (v, t);
    }

    /// The voltages with the transistor nodes at `t`.
    #[inline(always)]
    fn at(&self, t: [f64; T]) -> [f64; M] {
        let mut v = self.from;
        for ((x, s), m) in v.iter_mut().zip(&self.slope).zip(&self.moved) {
            let mut d = *m;
            for ((k, a), b) in s.iter().zip(&t).zip(&self.from_t) {
                d += k * (a - b);
            }
            *x += d;
        }
        v
    }

    /// The currents leaving the transistor nodes, at `t`, into their series resistances.
    #[inline(always)]
    fn currents(&self, t: [f64; T]) -> [f64; T] {
        let mut out = [0.0; T];
        for i in 0..T {
            let at = self.reach[i];
            let s = t[i] - self.from_t[i];
            let mut d = self.across[i] + self.slack[at] * s - self.moved[at];
            for j in (0..T).filter(|&j| j != i) {
                d += self.slope[at][j] * (s - (t[j] - self.from_t[j]));
            }
            out[i] = self.series[i] * d;
        }
        out
    }

    /// Its part of the reduced system's matrix, on the transistor nodes `nodes`.
    fn join(&self, s: &mut Matrix, nodes: [usize; T]) {
        for i in 0..T {
            let (g, row) = (self.series[i], &self.slope[self.reach[i]]);
            let mut diagonal = self.slack[self.reach[i]];
            for j in (0..T).filter(|&j| j != i) {
                s[nodes[i]][nodes[j]] = -g * row[j];
                diagonal += row[j];
            }
            s[nodes[i]][nodes[i]] = g * diagonal + GMIN;
        }
    }
}

/// A charge: its value and current at the last accepted point, and its value, capacitance
/// and voltage at the last load (for taking the accepted point's without evaluating it).
#[derive(Debug, Clone, Copy, Default)]
struct Charge {
    q: f64,
    i: f64,
    load: [f64; 3],
}

impl Charge {
    /// Its value `q` and capacitance `c` at `v` for a step of `ith` (2 / the step): its
    /// current, and the slope.
    #[inline(always)]
    fn current(&mut self, (q, c): (f64, f64), v: f64, ith: f64) -> (f64, f64) {
        self.load = [q, c, v];
        ((q - self.q) * ith - self.i, c * ith)
    }

    /// The accepted point's, at `v`.
    #[inline(always)]
    fn accept(&mut self, v: f64, ith: f64) {
        let [q, c, at] = self.load;
        let q = q + c * (v - at);
        self.i = (q - self.q) * ith - self.i;
        self.q = q;
    }
}

/// A transistor as the circuit has it, between its internal nodes: its Gummel-Poon currents,
/// its junctions' charges and its forward transit time's.
#[derive(Debug, Clone, Copy)]
struct Transistor {
    m: BjtAt,
    vaf: f64,
    pol: f64,
    vcrit: f64,
    je: Depletion,
    jc: Depletion,
    ke: DepletionConsts,
    kc: DepletionConsts,
    /// The diffusion charge's transit time, and that times IS and over VT.
    tt: f64,
    q_d: f64,
    c_d: f64,
    /// The junction voltages at the last iterate (SPICE's limiting).
    lim: [f64; 2],
    /// The reference for each junction's power ([`Depletion::charge_near`]).
    near: [[f64; 2]; 2],
    charge: [Charge; 3],
}

/// A transistor's currents leaving its collector, base and emitter, and their slopes against
/// those three nodes.
struct Stamp {
    i: [f64; 3],
    g: [[f64; 3]; 3],
}

impl Transistor {
    /// The junction voltages (base-emitter, base-collector) at its nodes' voltages.
    #[inline(always)]
    fn junctions(&self, [vc, vb, ve]: [f64; 3]) -> (f64, f64) {
        (self.pol * (vb - ve), self.pol * (vb - vc))
    }

    /// The limiting's start: the present junction voltages, no further forward than their
    /// critical voltage.
    #[inline(always)]
    fn start(&mut self, v: [f64; 3]) {
        let (x1, x2) = self.junctions(v);
        self.lim = [x1.min(self.vcrit), x2.min(self.vcrit)];
    }

    /// Its stamp at the nodes' voltages `v`, and whether a junction was limited (as
    /// [`Circuit`]'s load: the transistor about its limited junction voltages, its charges at
    /// the present ones).
    #[inline(always)]
    fn load(&mut self, v: [f64; 3], ith: f64) -> (Stamp, bool) {
        let (x1_0, x2_0) = self.junctions(v);
        let (m, pol) = (&self.m, self.pol);
        let x1 = pnjlim(x1_0, self.lim[0], m.vt, self.vcrit);
        let x2 = pnjlim(x2_0, self.lim[1], m.vt, self.vcrit);
        let limited = x1 != x1_0 || x2 != x2_0;
        self.lim = [x1, x2];
        let d = m.currents_d(x1, x2, self.vaf);
        let (dx1, dx2) = (x1_0 - x1, x2_0 - x2);
        let ic = d.ic + d.dic[0] * dx1 + d.dic[1] * dx2 - GMIN * x2_0;
        let ib = d.ib + d.dib[0] * dx1 + d.dib[1] * dx2 + GMIN * (x1_0 + x2_0);
        let (c1, c2) = (d.dic[0], d.dic[1] - GMIN);
        let (b1, b2) = (d.dib[0] + GMIN, d.dib[1] + GMIN);
        let [je, jc, df] = &mut self.charge;
        let (i_e, g_e) = je.current(
            self.je.charge_near(x1_0, &mut self.near[0], &self.ke),
            x1_0,
            ith,
        );
        let (i_c, g_c) = jc.current(
            self.jc.charge_near(x2_0, &mut self.near[1], &self.kc),
            x2_0,
            ith,
        );
        // The diffusion charge's exponential is the transistor's own unless the junction was
        // limited (performance).
        let q_c = if x1 == x1_0 && x1_0 <= 1.0 {
            (self.q_d * (d.ef - 1.0), self.c_d * d.ef)
        } else {
            diffusion_charge(x1_0, m.is, m.vt, self.tt)
        };
        let (i_d, g_d) = df.current(q_c, x1_0, ith);
        let (i_e, g_e) = (pol * (i_e + i_d), g_e + g_d);
        let i_c = pol * i_c;
        let (ic, ib) = (pol * ic, pol * ib);
        let i = [ic - i_c, ib + i_e + i_c, -ic - ib - i_e];
        let g = [
            [g_c - c2, c1 + c2 - g_c, -c1],
            [-b2 - g_c, b1 + b2 + g_e + g_c, -b1 - g_e],
            [c2 + b2, -(c1 + c2 + b1 + b2) - g_e, c1 + b1 + g_e],
        ];
        (Stamp { i, g }, limited)
    }

    #[inline(always)]
    fn accept(&mut self, v: [f64; 3], ith: f64) {
        let (x1, x2) = self.junctions(v);
        let [je, jc, df] = &mut self.charge;
        je.accept(x1, ith);
        jc.accept(x2, ith);
        df.accept(x1, ith);
    }
}

/// Column `p` of the reduced system's elimination, `rest` being the rows and columns below it
/// that it reaches. Its pivot's inverse stays on the diagonal.
#[inline(always)]
fn eliminate<const R: usize>(s: &mut Matrix, f: &mut [f64; INNER], p: usize, rest: [usize; R]) {
    let d = 1.0 / s[p][p];
    s[p][p] = d;
    let (row, fp) = (s[p], f[p]);
    for r in rest {
        let m = s[r][p] * d;
        for c in rest {
            s[r][c] -= m * row[c];
        }
        f[r] -= m * fp;
    }
}

/// Column `p`'s back substitution.
#[inline(always)]
fn back<const R: usize>(s: &Matrix, x: &mut [f64; INNER], p: usize, rest: [usize; R]) {
    let mut y = x[p];
    for c in rest {
        y -= s[p][c] * x[c];
    }
    x[p] = y * s[p][p];
}

/// Solves `s x = f`, leaving `x` in `f`: the ring (see the module's notes) in its fixed order.
#[inline(always)]
fn solve(s: &mut Matrix, f: &mut [f64; INNER]) {
    eliminate(s, f, C2, [B2, E2]);
    eliminate(s, f, E3, [C3, B3]);
    eliminate(s, f, C1, [B1, E1, B3]);
    eliminate(s, f, C3, [B2, B3]);
    eliminate(s, f, E2, [E1, B2]);
    eliminate(s, f, E1, [B1, B2, B3]);
    eliminate(s, f, B1, [B2, B3]);
    eliminate(s, f, B2, [B3]);
    eliminate(s, f, B3, []);
    back(s, f, B3, []);
    back(s, f, B2, [B3]);
    back(s, f, B1, [B2, B3]);
    back(s, f, E1, [B1, B2, B3]);
    back(s, f, E2, [E1, B2]);
    back(s, f, C3, [B2, B3]);
    back(s, f, C1, [B1, E1, B3]);
    back(s, f, E3, [C3, B3]);
    back(s, f, C2, [B2, E2]);
}

/// A linear capacitor: its value, and its charge and current at the last accepted point.
#[derive(Debug, Clone, Copy, Default)]
struct Capacitor {
    c: f64,
    q: f64,
    i: f64,
}

/// The preamplifier's circuit with its linear nodes eliminated; see the module's notes.
#[derive(Debug, Clone)]
pub(super) struct Reduced {
    /// Newton's tolerance, as [`Circuit`]'s.
    pub tolerance: (f64, f64),
    /// The solved nodes' voltages, the linear ones first, in the circuit's order.
    v: [f64; LINEAR + INNER],
    saved: [f64; LINEAR + INNER],
    prev: [f64; LINEAR + INNER],
    prev2: [f64; LINEAR + INNER],
    prev_h: f64,
    prev2_h: f64,
    /// The step the coefficients are for, and 2 / it (the trapezoidal rule's).
    h: f64,
    ith: f64,
    q: [Transistor; 3],
    /// The groups, in the circuit's order: N78 and B27; C27 and N64; TAIL; B32, N61, N62, AMP
    /// and OUT.
    input: Group<2, 1>,
    load: Group<2, 2>,
    tail: Group<1, 2>,
    feedback: Group<5, 2>,
    /// The series conductances of the two transistor nodes that reach a supply directly, and
    /// the reduced system's matrix without the transistors.
    g_c2: f64,
    g_e3: f64,
    joined: Matrix,
    /// The linear parts' conductances, and those of the source (R9 and R78) and the output's
    /// load (R46 or SW10 off).
    g: Parts,
    g_src: f64,
    g_out: f64,
    /// The linear capacitors: N78 to B27 (C23), N64 to P10, N62 to ground (C26), B32 to AMP
    /// and AMP to OUT (C20).
    caps: [Capacitor; 5],
    /// The supplies, V.
    p10: f64,
    n10: f64,
}

/// The linear parts' conductances, by the nodes they join.
#[derive(Debug, Clone, Copy)]
struct Parts {
    b27_gnd: f64,
    p10_c27: f64,
    c27_n64: f64,
    tail_n10: f64,
    amp_n10: f64,
    b32_n61: f64,
    n61_amp: f64,
    n61_n62: f64,
}

impl Reduced {
    /// From the circuit, at its operating point, for steps of `h`: its parts' values are
    /// read from it, so that the netlist stays in one place.
    pub(super) fn new(net: &Circuit, h: f64) -> Reduced {
        assert_eq!(
            net.extent(),
            (LINEAR + INNER, 36),
            "the preamplifier's parts"
        );
        let resistor = |a: Node, b: Node| {
            net.parts.iter().find_map(|p| match *p {
                Part::Resistor { a: x, b: y, r } if (x, y) == (a, b) => Some(1.0 / r),
                _ => None,
            })
        };
        let g = |a: Node, b: Node| resistor(a, b).expect("the preamplifier's resistor");
        let capacitor = |a: Node, b: Node| {
            let c = net.parts.iter().find_map(|p| match *p {
                Part::Capacitor { a: x, b: y, c } if (x, y) == (a, b) => Some(c),
                _ => None,
            });
            c.expect("the preamplifier's capacitor")
        };
        let mut v = [0.0; LINEAR + INNER];
        for (k, x) in v.iter_mut().enumerate() {
            *x = net.v(k + HELD);
        }
        let mut q: Vec<Transistor> = Vec::new();
        let mut series = [0.0; INNER];
        let outer = [[C27, B27, TAIL], [P10, B32, TAIL], [AMP, C27, P10]];
        for (idx, p) in net.parts.iter().enumerate() {
            let Part::Bjt {
                c,
                b,
                e,
                m,
                vaf,
                pnp,
            } = *p
            else {
                continue;
            };
            let k = q.len();
            assert_eq!((c, b, e), (HELD + LINEAR + 3 * k, c + 1, c + 2));
            for (j, n) in [c, b, e].into_iter().enumerate() {
                series[3 * k + j] = g(outer[k][j], n);
            }
            let (
                Part::Junction { cap: je, .. },
                Part::Junction { cap: jc, .. },
                Part::Diffusion { is, vt, tt, .. },
            ) = (net.parts[idx + 1], net.parts[idx + 2], net.parts[idx + 3])
            else {
                panic!("the preamplifier's transistor's charges");
            };
            let mut t = Transistor {
                m,
                vaf,
                pol: if pnp { -1.0 } else { 1.0 },
                vcrit: crate::mna::vcrit(m.vt, m.is),
                je,
                jc,
                ke: je.consts(),
                kc: jc.consts(),
                tt,
                q_d: tt * is,
                c_d: tt * is / vt,
                lim: [0.0; 2],
                near: [[f64::NAN; 2]; 2],
                charge: [Charge::default(); 3],
            };
            let (x1, x2) = t.junctions([net.v(c), net.v(b), net.v(e)]);
            t.charge[0].q = je.charge(x1).0;
            t.charge[1].q = jc.charge(x2).0;
            t.charge[2].q = diffusion_charge(x1, is, vt, tt).0;
            q.push(t);
        }
        let q: [Transistor; 3] = q.try_into().expect("the preamplifier's transistors");
        let parts = Parts {
            b27_gnd: g(B27, GND),
            p10_c27: g(P10, C27),
            c27_n64: g(C27, N64),
            tail_n10: g(TAIL, N10),
            amp_n10: g(AMP, N10),
            b32_n61: g(B32, N61),
            n61_amp: g(N61, AMP),
            n61_n62: g(N61, N62),
        };
        let caps = [(N78, B27), (N64, P10), (N62, GND), (B32, AMP), (AMP, OUT)].map(|(a, b)| {
            let c = capacitor(a, b);
            Capacitor {
                c,
                q: c * (net.v(a) - net.v(b)),
                i: 0.0,
            }
        });
        let mut r = Reduced {
            tolerance: net.tolerance,
            v,
            saved: v,
            prev: v,
            prev2: v,
            prev_h: 0.0,
            prev2_h: 0.0,
            h,
            ith: 2.0 / h,
            q,
            input: Group::new([series[B1]], [1]),
            load: Group::new([series[C1], series[B3]], [0, 0]),
            tail: Group::new([series[E1], series[E2]], [0, 0]),
            feedback: Group::new([series[B2], series[C3]], [0, 3]),
            g_c2: series[C2],
            g_e3: series[E3],
            joined: [[0.0; INNER]; INNER],
            g: parts,
            g_src: g(SRC, N78),
            g_out: g(OUT, GND),
            caps,
            p10: net.v(P10),
            n10: net.v(N10),
        };
        r.joined[C2][C2] = r.g_c2 + GMIN;
        r.joined[E3][E3] = r.g_e3 + GMIN;
        r.tail.set([], [r.g.tail_n10 + GMIN]);
        r.tail.join(&mut r.joined, [E1, E2]);
        r.prepare();
        r
    }

    /// The coefficients of the groups that depend on the step.
    fn prepare(&mut self) {
        self.set_input();
        self.set_load();
        self.set_feedback();
    }

    fn set_input(&mut self) {
        let gc = self.caps[0].c * self.ith;
        let shunt = [self.g_src + GMIN, self.g.b27_gnd + GMIN];
        self.input.set([(0, 1, gc)], shunt);
        self.input.join(&mut self.joined, [B1]);
    }

    fn set_load(&mut self) {
        let shunt = [self.g.p10_c27 + GMIN, self.caps[1].c * self.ith + GMIN];
        self.load.set([(0, 1, self.g.c27_n64)], shunt);
        self.load.join(&mut self.joined, [C1, B3]);
    }

    fn set_feedback(&mut self) {
        let ith = self.ith;
        let Parts {
            amp_n10,
            b32_n61,
            n61_amp,
            n61_n62,
            ..
        } = self.g;
        let links = [
            (0, 1, b32_n61),
            (1, 3, n61_amp),
            (1, 2, n61_n62),
            (0, 3, self.caps[3].c * ith),
            (3, 4, self.caps[4].c * ith),
        ];
        let shunt = [
            GMIN,
            GMIN,
            self.caps[2].c * ith + GMIN,
            amp_n10 + GMIN,
            self.g_out + GMIN,
        ];
        self.feedback.set(links, shunt);
        self.feedback.join(&mut self.joined, [B2, C3]);
    }

    /// R9's source resistance and R78 together, ohm.
    pub(super) fn set_source(&mut self, r: f64) {
        self.g_src = 1.0 / r;
        self.set_input();
    }

    /// The output's load, ohm.
    pub(super) fn set_output(&mut self, r: f64) {
        self.g_out = 1.0 / r;
        self.set_feedback();
    }

    /// A linear node's voltage.
    pub(super) fn v(&self, n: Node) -> f64 {
        self.v[n - HELD]
    }

    /// The groups from the accepted point `saved`, with the source now at `src`: each node's
    /// current left over there, with its capacitors' currents as they were.
    #[inline(always)]
    fn start(&mut self, src: f64) {
        let [
            n78,
            b27,
            c27,
            n64,
            tail,
            b32,
            n61,
            n62,
            amp,
            out,
            c1,
            b1,
            e1,
            _,
            b2,
            e2,
            c3,
            b3,
            _,
        ] = self.saved;
        let i = self.caps.map(|c| c.i);
        let g = self.g;
        let gb1 = self.input.series[0];
        self.input.start(
            [
                self.g_src * (n78 - src) + GMIN * n78 - i[0],
                g.b27_gnd * b27 + GMIN * b27 + gb1 * (b27 - b1) + i[0],
            ],
            [n78, b27],
            [b1],
        );
        let [g1, g3] = self.load.series;
        self.load.start(
            [
                g.p10_c27 * (c27 - self.p10)
                    + g.c27_n64 * (c27 - n64)
                    + GMIN * c27
                    + g1 * (c27 - c1)
                    + g3 * (c27 - b3),
                g.c27_n64 * (n64 - c27) + GMIN * n64 - i[1],
            ],
            [c27, n64],
            [c1, b3],
        );
        let [g1, g2] = self.tail.series;
        self.tail.start(
            [g.tail_n10 * (tail - self.n10) + GMIN * tail + g1 * (tail - e1) + g2 * (tail - e2)],
            [tail],
            [e1, e2],
        );
        let [g2, g3] = self.feedback.series;
        self.feedback.start(
            [
                g2 * (b32 - b2) + g.b32_n61 * (b32 - n61) + GMIN * b32 - i[3],
                g.b32_n61 * (n61 - b32)
                    + g.n61_amp * (n61 - amp)
                    + g.n61_n62 * (n61 - n62)
                    + GMIN * n61,
                g.n61_n62 * (n62 - n61) + GMIN * n62 - i[2],
                g.n61_amp * (amp - n61)
                    + g.amp_n10 * (amp - self.n10)
                    + GMIN * amp
                    + g3 * (amp - c3)
                    + i[3]
                    - i[4],
                self.g_out * out + GMIN * out + i[4],
            ],
            [b32, n61, n62, amp, out],
            [b2, c3],
        );
    }

    /// The linear nodes with the transistor nodes at `t`.
    #[inline(always)]
    fn linear(&self, t: &[f64; INNER]) -> [f64; LINEAR] {
        let [a0, a1] = self.input.at([t[B1]]);
        let [b0, b1] = self.load.at([t[C1], t[B3]]);
        let [c0] = self.tail.at([t[E1], t[E2]]);
        let [d0, d1, d2, d3, d4] = self.feedback.at([t[B2], t[C3]]);
        [a0, a1, b0, b1, c0, d0, d1, d2, d3, d4]
    }

    /// Newton's method for one step from the present voltages, as [`Circuit`]'s, its counts
    /// going to `net`.
    #[inline(always)]
    fn newton(&mut self, net: &mut Circuit) -> Option<usize> {
        let (mut l, mut t) = ([0.0; LINEAR], [0.0; INNER]);
        l.copy_from_slice(&self.v[..LINEAR]);
        t.copy_from_slice(&self.v[LINEAR..]);
        let nodes = |t: &[f64; INNER], k: usize| [t[3 * k], t[3 * k + 1], t[3 * k + 2]];
        for (k, q) in self.q.iter_mut().enumerate() {
            q.start(nodes(&t, k));
        }
        net.solves += 1;
        let tol = self.tolerance;
        let ith = self.ith;
        // (`l` is only for the stopping rule: the load takes the groups from the transistor
        // nodes, so a predicted voltage of theirs has no say in the iterates.)
        for it in 1..=ITERATIONS {
            net.total_iterations += 1;
            let mut laps = crate::prof::Laps::start();
            #[cfg(feature = "profile")]
            let t0 = std::time::Instant::now();
            let mut f = t.map(|v| GMIN * v);
            let [b1] = self.input.currents([t[B1]]);
            let [c1, b3] = self.load.currents([t[C1], t[B3]]);
            let [e1, e2] = self.tail.currents([t[E1], t[E2]]);
            let [b2, c3] = self.feedback.currents([t[B2], t[C3]]);
            f[B1] += b1;
            f[C1] += c1;
            f[B3] += b3;
            f[E1] += e1;
            f[E2] += e2;
            f[B2] += b2;
            f[C3] += c3;
            f[C2] += self.g_c2 * (t[C2] - self.p10);
            f[E3] += self.g_e3 * (t[E3] - self.p10);
            let mut s = self.joined;
            let mut limited = false;
            for (k, q) in self.q.iter_mut().enumerate() {
                let (d, lim) = q.load(nodes(&t, k), ith);
                limited |= lim;
                for r in 0..3 {
                    f[3 * k + r] += d.i[r];
                    for c in 0..3 {
                        s[3 * k + r][3 * k + c] += d.g[r][c];
                    }
                }
            }
            laps.lap(crate::prof::Part::SolverLoad);
            #[cfg(feature = "profile")]
            let t1 = std::time::Instant::now();
            solve(&mut s, &mut f);
            laps.lap(crate::prof::Part::SolverSolve);
            #[cfg(feature = "profile")]
            {
                net.nanos_load += (t1 - t0).as_nanos() as u64;
                net.nanos_lu += t1.elapsed().as_nanos() as u64;
            }
            if f.iter().any(|x| !x.is_finite()) {
                break;
            }
            for k in 0..INNER {
                t[k] -= f[k];
            }
            let at = self.linear(&t);
            let within = |d: f64, v: f64| d.abs() <= tol.0 + tol.1 * v.abs();
            let small = (0..INNER).all(|k| within(f[k], t[k]))
                && (0..LINEAR).all(|k| within(at[k] - l[k], at[k]));
            l = at;
            if small && !limited {
                self.v[..LINEAR].copy_from_slice(&l);
                self.v[LINEAR..].copy_from_slice(&t);
                net.iterations = it;
                net.iteration_counts[iteration_bin(it)] += 1;
                return Some(it);
            }
        }
        net.iteration_counts[7] += 1;
        net.iterations = ITERATIONS;
        None
    }

    /// One trapezoidal step of `h` s with the source at `src`, from the last accepted point:
    /// its Newton iterations, or `None` with the voltages restored, so that the step can be
    /// retried shorter.
    pub(super) fn step(&mut self, h: f64, src: f64, net: &mut Circuit) -> Option<usize> {
        #[cfg(feature = "profile")]
        let t0 = std::time::Instant::now();
        if h != self.h {
            self.h = h;
            self.ith = 2.0 / h;
            self.prepare();
        }
        self.saved = self.v;
        self.start(src);
        if self.prev_h == h && self.prev2_h == h {
            for k in 0..LINEAR + INNER {
                self.v[k] = 3.0 * self.v[k] - 3.0 * self.prev[k] + self.prev2[k];
            }
        }
        let r = self.newton(net);
        if r.is_some() {
            self.prev2 = self.prev;
            self.prev = self.saved;
            self.prev2_h = self.prev_h;
            self.prev_h = h;
            self.accept();
        } else {
            self.v = self.saved;
        }
        #[cfg(feature = "profile")]
        {
            net.nanos += t0.elapsed().as_nanos() as u64;
        }
        r
    }

    /// The accepted point's charges and currents.
    #[inline(always)]
    fn accept(&mut self) {
        let ith = self.ith;
        let n = |x: Node| self.v[x - HELD];
        let across = [
            n(N78) - n(B27),
            n(N64) - self.p10,
            n(N62),
            n(B32) - n(AMP),
            n(AMP) - n(OUT),
        ];
        for (c, v) in self.caps.iter_mut().zip(across) {
            let q = c.c * v;
            c.i = (q - c.q) * ith - c.i;
            c.q = q;
        }
        for k in 0..3 {
            let o = LINEAR + 3 * k;
            self.q[k].accept([self.v[o], self.v[o + 1], self.v[o + 2]], ith);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{NODES, Preamp, PreampCircuit, SUBSTEPS, V_POS};
    use super::*;

    /// A source that moves as a load demands.
    type Drive = fn(usize, f64) -> f64;

    /// The solvers side by side.
    struct Lockstep {
        /// The largest difference from the general solver at any solved node, V.
        worst: f64,
        /// The same for the general solver against itself on supplies a few ulps away: its
        /// own rounding, which a difference of its size is no more than. (More than one: on
        /// the hardest loads most such supplies part from it by volts, and a few do not.)
        own: f64,
        /// Newton's iterations and the failed steps: the general solver's, then the reduced
        /// one's.
        iterations: [u64; 2],
        failed: [usize; 2],
    }

    /// The general solver beside the reduced one, substep by substep over `n` substeps from
    /// `r_src`; `each` changes R9 or SW10 on all of them before a substep.
    fn lockstep(
        rate: f64,
        r_src: f64,
        tol: (f64, f64),
        n: usize,
        drive: Drive,
        each: &mut dyn FnMut(usize, &mut Preamp, &mut Circuit),
    ) -> Lockstep {
        let k = PreampCircuit::default();
        let mut p = Preamp::new(k, rate, r_src).unwrap();
        let mut c = Preamp::amplifier(&k, r_src).unwrap();
        p.amp.tolerance = tol;
        c.tolerance = tol;
        let mut twins = [3.0, 23.0, 101.0].map(|ulps| {
            let mut t = c.clone();
            t.set(P10, V_POS + ulps * 1.8e-15);
            t
        });
        let h = 1.0 / rate / SUBSTEPS as f64;
        let (mut worst, mut own) = (0.0f64, 0.0f64);
        let mut failed = 0;
        for i in 0..n {
            each(i, &mut p, &mut c);
            let x = drive(i, i as f64 * h);
            p.fine(x);
            c.set(SRC, x);
            failed += usize::from(!Preamp::step(&mut c, h));
            for t in &mut twins {
                each(i, &mut p, t);
                t.set(SRC, x);
                Preamp::step(t, h);
            }
            for node in HELD..NODES + INNER {
                worst = worst.max((p.voltage(node) - c.v(node)).abs());
                for t in &twins {
                    own = own.max((t.v(node) - c.v(node)).abs());
                }
            }
        }
        Lockstep {
            worst,
            own,
            iterations: [c.total_iterations, p.net.total_iterations],
            failed: [failed, p.failed],
        }
    }

    fn sine(amp: f64, hz: f64, t: f64) -> f64 {
        amp * (std::f64::consts::TAU * hz * t).sin()
    }

    /// A rough, wide source: octaves of a sine under a slow square.
    fn rough(t: f64) -> f64 {
        let square = if sine(1.0, 90.0, t) > 0.0 { 1.0 } else { -1.0 };
        square + (0..7).fold(0.0, |a, k| a + sine(0.7, 311.0 * (1 << k) as f64, t))
    }

    const DRIVES: [(&str, Drive); 7] = [
        ("silence", |_, _| 0.0),
        ("small tone", |_, t| sine(0.005, 1000.0, t)),
        ("clipping tone", |_, t| sine(0.3, 220.0, t)),
        ("high tone", |_, t| sine(2.0, 15000.0, t)),
        ("steps", |i, _| if i / 4000 % 2 == 0 { 2.0 } else { -2.0 }),
        ("rough", |_, t| rough(t)),
        ("past the supplies", |_, t| 12.0 * rough(t)),
    ];

    /// A difference of a solve's rounding: within a few times what the general solver makes
    /// of a change of a few ulps, which is itself the larger part of it. Newton's tolerance
    /// `tol` lets more through: the second-order remainder of a last correction of that size
    /// (at a thermal voltage's curvature; nothing at No Compromises' tolerance), and after a
    /// failed step the shorter steps' iterates part by as much as `tol`.
    fn rounding(r: &Lockstep, tol: f64) -> bool {
        let retried = if r.failed[0] > 0 { tol } else { 0.0 };
        r.worst <= (8.0 * r.own.max(2e-13)).max(tol * tol / 0.01).max(retried)
    }

    /// The reduced solve is the circuit's: every node within the general solver's own
    /// rounding at every substep, in the same Newton iterations, at each tolerance, sample
    /// rate and setting of R9, on loads from silence to far past anything the panel can reach.
    /// Where steps fail the same number fail, and the iterations stay within 0.1 %, unless the
    /// general solver's own path depends on its rounding (2 V at 15 kHz from no source
    /// resistance, 400 times what clips it: its failed steps then part it from itself by
    /// volts), when there is nothing to hold it to.
    #[test]
    fn it_follows_the_general_solver() {
        for (name, drive) in DRIVES {
            for rate in [44_100.0, 96_000.0] {
                for r_src in [0.0, 1e3, 100e3, 1e6] {
                    for tol in [(1e-9, 1e-9), (1e-5, 1e-5)] {
                        let n = (0.05 * rate) as usize * SUBSTEPS;
                        let r = lockstep(rate, r_src, tol, n, drive, &mut |_, _, _| {});
                        let [a, b] = r.iterations;
                        println!(
                            "{name}, {rate} Hz, {r_src} ohm, {tol:?}: {:.1e} V (its own {:.1e}), {a} {b} iterations, {:?} failed",
                            r.worst, r.own, r.failed
                        );
                        assert!(rounding(&r, tol.0), "{name}: {r_src}: {:.1e} V", r.worst);
                        if r.own < 1e-6 {
                            let near = a.abs_diff(b) as f64 <= 1e-3 * a as f64 + 2.0;
                            assert!(near, "{name}: {r_src}: {a} against {b} iterations");
                            assert_eq!(r.failed[0], r.failed[1], "{name}: {r_src}");
                        }
                    }
                }
            }
        }
    }

    /// R9 moving and SW10 thrown under a load keep their coefficients right.
    #[test]
    fn its_coefficients_follow_the_controls() {
        let part = |c: &Circuit, x: Node, y: Node| {
            let at = |p: &Part| matches!(*p, Part::Resistor { a, b, .. } if (a, b) == (x, y));
            c.parts.iter().position(at).unwrap()
        };
        for (name, drive) in DRIVES {
            let r = lockstep(
                48_000.0,
                100e3,
                (1e-9, 1e-9),
                48_000,
                drive,
                &mut |i, p, c| {
                    if i % 4 == 0 {
                        let r = 250e3 * (1.0 + (i as f64 * 1e-4).sin());
                        p.set_source_resistance(r);
                        c.set_resistance(part(c, SRC, N78), r + 1e3);
                    }
                    if i % 9973 == 0 {
                        let on = i / 9973 % 2 == 1;
                        p.set_switch(on);
                        c.set_resistance(part(c, OUT, GND), if on { 33e3 } else { 1e12 });
                    }
                },
            );
            println!("{name}: {:.1e} V (its own {:.1e})", r.worst, r.own);
            assert!(rounding(&r, 1e-9), "{name}: {:.1e} V", r.worst);
        }
    }

    /// A failed step's shorter steps (the coefficients for a sixteenth of the step, and back)
    /// and the predictor starting again after them, as the circuit's.
    #[test]
    fn a_shorter_step_is_the_circuits() {
        let k = PreampCircuit::default();
        let mut c = Preamp::amplifier(&k, 100e3).unwrap();
        let mut count = c.clone();
        let mut r = Reduced::new(&c, 1.0 / 192e3);
        for i in 0..4000 {
            let x = rough(i as f64 / 192e3);
            let hs: &[f64] = if i % 7 == 3 {
                &[1.0 / 16.0; 16]
            } else {
                &[1.0]
            };
            for s in hs {
                let h = s / 192e3;
                c.set(SRC, x);
                c.step(h).unwrap();
                r.step(h, x, &mut count).expect("the step");
                for n in HELD..NODES + INNER {
                    assert!((c.v(n) - r.v(n)).abs() < 1e-11, "{i}: node {n}");
                }
            }
        }
        let (a, b) = (c.total_iterations, count.total_iterations);
        assert!(a.abs_diff(b) <= a / 1000, "{a} against {b} iterations");
    }

    /// A Newton solve from anywhere gives the same answer in the same iterations: the
    /// transistor nodes moved at random from an accepted point, near and far (the junctions
    /// limited, or not).
    #[test]
    fn a_solve_from_anywhere_is_the_circuits() {
        let k = PreampCircuit::default();
        let h = 1.0 / 192e3;
        let mut c = Preamp::amplifier(&k, 100e3).unwrap();
        let mut count = c.clone();
        let mut r = Reduced::new(&c, h);
        let mut seed = 12345u64;
        let mut random = move || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 11) as f64 / (1u64 << 53) as f64 - 0.5
        };
        for i in 0..3000 {
            let x = sine(1.0, 700.0, i as f64 * h);
            c.set(SRC, x);
            c.step(h).unwrap();
            r.step(h, x, &mut count).expect("the step");
            let (mut a, mut b) = (c.clone(), r.clone());
            let scale = [1e-3, 0.05, 0.5, 3.0][i % 4];
            for n in HELD + LINEAR..NODES + INNER {
                let v = a.v(n) + scale * random();
                a.set(n, v);
                b.v[n - HELD] = v;
            }
            b.prev_h = 0.0;
            let x = sine(1.0, 700.0, (i + 1) as f64 * h);
            a.set(SRC, x);
            let (ra, rb) = (a.step(h), b.step(h, x, &mut count));
            assert_eq!(ra.ok(), rb, "{i}");
            for n in HELD..NODES + INNER {
                assert!((a.v(n) - b.v(n)).abs() < 1e-10, "{i}: node {n}");
            }
        }
    }
}
