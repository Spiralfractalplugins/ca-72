//! MIDI Learn's parts of the panel (the CA-72's `docs/decisions.md` R34): a control's menu (a
//! right click), the note over the control being learned, and the ring that marks it. Each is a
//! layer the renderer draws over the controls; a menu's items are found where they are drawn
//! ([`Menu::hit`]), both from one layout.

use crate::art::{H, Layer, W};
use crate::controls::Control;
use crate::fonts::{Fonts, Weight};
use crate::svg::{N, Svg, colour, escape, put};

/// The strip's and the drawer's accent: what MIDI Learn waits on, and what it did.
pub const ACCENT: &str = "#f0a030";
const DIM: &str = "#8a909c";
const WARN: &str = "#e5b94d";
const BACK: &str = "#1f2228";
const BORDER: &str = "#4a4640";
const HOVER: &str = "#3a3022";
/// A menu's title's band, darker than its items' (the owner, 2026-10-10: "we need to better
/// distinguish menu headers from menu options").
const HEAD: &str = "#14161a";

/// A line's colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Accent,
    Dim,
    Warn,
}

impl Tone {
    fn fill(self) -> &'static str {
        match self {
            Tone::Plain => colour::LEGEND,
            Tone::Accent => ACCENT,
            Tone::Dim => DIM,
            Tone::Warn => WARN,
        }
    }
}

/// A control's menu: its top left corner (drawing units), its lettering's size (drawing units),
/// its title (not an item), its items (each its label and whether it can be chosen), those of
/// them that head a group of the items after them (not to be chosen), those that are their
/// group's current choice (marked), the one under the pointer, and those that begin a column
/// beside the one before (each a group's head, with where its labels begin from the menu's left
/// edge in the lettering's sizes; none, the menu one column).
#[derive(Clone, Debug, PartialEq)]
pub struct Menu {
    pub x: f64,
    pub y: f64,
    pub size: f64,
    pub title: String,
    pub items: Vec<(String, bool)>,
    pub heads: Vec<usize>,
    pub marked: Vec<usize>,
    pub hover: Option<usize>,
    pub columns: Vec<(usize, f64)>,
}

impl Menu {
    fn pad(&self) -> f64 {
        self.size * 0.8
    }

    fn row(&self) -> f64 {
        self.size * 2.0
    }

    /// The title's lettering: smaller than the items', bold, spaced (its size, its spacing).
    fn title_type(&self) -> (f64, f64) {
        (self.size * 0.82, self.size * 0.14)
    }

    /// A group's head's lettering: a little smaller still, bold, spaced (0.72 of the items' left
    /// it the smallest print in the window: decisions.md R-READABILITY-MINIMUM).
    fn head_type(&self) -> (f64, f64) {
        (self.size * 0.8, self.size * 0.12)
    }

    /// Item `i`'s indent: a group's items are set in under their head.
    fn indent(&self, i: usize) -> f64 {
        if !self.heads.contains(&i) && self.heads.iter().any(|&h| h < i) {
            self.size
        } else {
            0.0
        }
    }

    /// Item `i`'s width: its label's, set in under its head where it is.
    fn width(&self, fonts: &Fonts, i: usize) -> f64 {
        let t = &self.items[i].0;
        if self.heads.contains(&i) {
            let (hs, hsp) = self.head_type();
            fonts.advance(t, hs, Weight::Bold, hsp)
        } else {
            fonts.advance(t, self.size, Weight::Regular, 0.0) + self.indent(i)
        }
    }

    /// Its columns' items, first to last.
    fn column_items(&self) -> Vec<std::ops::Range<usize>> {
        let mut starts = vec![0];
        starts.extend(self.columns.iter().map(|&(i, _)| i));
        starts.push(self.items.len());
        starts.windows(2).map(|w| w[0]..w[1]).collect()
    }

    /// Item `i`'s column and its row in it (0 its first item's).
    fn place(&self, i: usize) -> (usize, usize) {
        let c = self.columns.iter().filter(|&&(s, _)| s <= i).count();
        let start = if c == 0 { 0 } else { self.columns[c - 1].0 };
        (c, i - start)
    }

    /// Where each column's labels begin, from the menu's left edge.
    fn lefts(&self) -> Vec<f64> {
        std::iter::once(self.pad())
            .chain(self.columns.iter().map(|&(_, k)| k * self.size))
            .collect()
    }

    /// Column `c`'s extent across the menu (its left and right edges, from the menu's), `w`
    /// the menu's width: from the rule before it to the rule after it, the outer ones from and
    /// to the menu's edges.
    fn column_span(&self, c: usize, w: f64) -> (f64, f64) {
        let lefts = self.lefts();
        let left = if c == 0 { 0.0 } else { lefts[c] - self.pad() };
        let right = lefts.get(c + 1).map_or(w, |&next| next - self.pad());
        (left, right)
    }

    /// Its width and height.
    pub fn extent(&self, fonts: &Fonts) -> (f64, f64) {
        let (ts, tsp) = self.title_type();
        let last = self.column_items().pop().unwrap_or(0..0);
        let left = self.lefts().last().copied().unwrap_or(0.0);
        let widest = last
            .map(|i| left - self.pad() + self.width(fonts, i))
            .fold(fonts.advance(&self.title, ts, Weight::Bold, tsp), f64::max);
        let rows = self
            .column_items()
            .iter()
            .map(|r| r.len())
            .max()
            .unwrap_or(0);
        (
            widest + 2.0 * self.pad(),
            self.row() * (rows + 1) as f64 + self.size * 0.4,
        )
    }

    /// Its items in as few columns as make it fit (`width`, `height`), at most `most`, each
    /// column begun at a group's head (a group never split), the groups kept in order and
    /// spread as evenly as they allow: of the ways that fit, the one whose columns' heights
    /// differ least (then the shortest tallest, then the longer first), a margin's width either
    /// side of the rule between two. Its lettering and rows stay as they are where it fits; none
    /// fitting, they are made smaller, as little as the way that needs it least allows (the
    /// fewer columns where two need it as little), so that it does.
    pub fn in_columns(mut self, fonts: &Fonts, (width, height): (f64, f64), most: usize) -> Menu {
        let heads: Vec<usize> = self.heads.iter().copied().filter(|&h| h > 0).collect();
        let n = self.items.len();
        let widths: Vec<f64> = (0..n).map(|i| self.width(fonts, i)).collect();
        let heights = |starts: &[usize]| -> Vec<usize> {
            std::iter::once(0)
                .chain(starts.iter().copied())
                .zip(starts.iter().copied().chain(std::iter::once(n)))
                .map(|(a, b)| b - a)
                .collect()
        };
        // How evenly: the sum of the columns' heights' squares (their total is the items'),
        // then the tallest, then the later starts (the longer first).
        let evenness = |starts: &Vec<usize>| {
            let h = heights(starts);
            (
                h.iter().map(|&r| r * r).sum::<usize>(),
                h.iter().copied().max().unwrap_or(0),
                std::cmp::Reverse(starts.clone()),
            )
        };
        self.columns.clear();
        // (How much smaller it must be to fit, at most 1; its columns.)
        let need = |m: &Menu| {
            let (w, h) = m.extent(fonts);
            (width / w).min(height / h)
        };
        let mut best = (need(&self), Vec::new());
        if best.0 >= 1.0 {
            return self;
        }
        for more in 1..most.min(heads.len() + 1) {
            let mut ways = choose(&heads, more);
            ways.sort_by_cached_key(evenness);
            for starts in ways {
                // Each column's labels begin a margin past the rule after the one before.
                self.columns.clear();
                let mut x = self.pad();
                let mut from = 0;
                for &start in &starts {
                    x += widths[from..start].iter().copied().fold(0.0, f64::max) + 2.0 * self.pad();
                    self.columns.push((start, x / self.size));
                    from = start;
                }
                let k = need(&self);
                if k >= 1.0 {
                    return self;
                }
                if k > best.0 * (1.0 + 1e-9) {
                    best = (k, self.columns.clone());
                }
            }
        }
        // (The columns' places are in the lettering's sizes, so they shrink with it; a hair
        // more, so that rounding leaves it inside the room.)
        self.columns = best.1;
        self.size *= best.0 * (1.0 - 1e-9);
        self
    }

    /// The menu moved to fit within `area` (left, top, right, bottom), its top left corner at
    /// (`x`, `y`) where it can be; its lettering made smaller where the menu would be taller or
    /// wider than `area` (the large lettering in a small window: R-LETTERING), its extent
    /// growing with its lettering's size.
    pub fn placed(mut self, fonts: &Fonts, x: f64, y: f64, area: [f64; 4]) -> Menu {
        let (w, h) = self.extent(fonts);
        let k = ((area[2] - area[0]) / w).min((area[3] - area[1]) / h);
        if k < 1.0 {
            self.size *= k;
        }
        let (w, h) = self.extent(fonts);
        self.x = x.min(area[2] - w).max(area[0]);
        self.y = y.min(area[3] - h).max(area[1]);
        self
    }

    /// Where item `i`'s label begins, mid-height (drawing units).
    pub fn item_at(&self, i: usize) -> (f64, f64) {
        let (c, r) = self.place(i);
        (
            self.x + self.lefts()[c] + self.size,
            self.y + self.row() * (r as f64 + 1.5),
        )
    }

    /// What is at (`x`, `y`): `Some(Some(i))` item `i`, `Some(None)` the menu but no item
    /// (its title, its margin), `None` outside it.
    pub fn hit(&self, fonts: &Fonts, x: f64, y: f64) -> Option<Option<usize>> {
        let (w, h) = self.extent(fonts);
        if !(self.x..self.x + w).contains(&x) || !(self.y..self.y + h).contains(&y) {
            return None;
        }
        let c = (1..=self.columns.len())
            .filter(|&c| self.column_span(c, w).0 <= x - self.x)
            .count();
        let r = self.column_items()[c].clone();
        let k = ((y - self.y) / self.row()).floor() as usize;
        Some(
            k.checked_sub(1)
                .map(|k| r.start + k)
                .filter(|i| r.contains(i)),
        )
    }
}

/// Every way of choosing `k` of `from`, in order.
fn choose(from: &[usize], k: usize) -> Vec<Vec<usize>> {
    if k == 0 {
        return vec![vec![]];
    }
    (0..from.len())
        .flat_map(|j| {
            choose(&from[j + 1..], k - 1)
                .into_iter()
                .map(move |mut rest| {
                    rest.insert(0, from[j]);
                    rest
                })
        })
        .collect()
}

/// A menu's layer: a box in the tips' colours, its title on a darker band ruled off from the
/// items, in smaller, bold, spaced amber capitals, then its items, the one under the pointer
/// lit, one that cannot be chosen dim; a group's head dim, small, bold and spaced, ruled off
/// from what is above it, its items set in under it; the current choice marked with a dot.
pub fn menu(fonts: &Fonts, m: &Menu) -> Layer {
    let (w, h) = m.extent(fonts);
    let mut s = Svg::default();
    put!(
        s,
        "<rect x='0' y='0' width='{}' height='{}' rx='{}' fill='{BACK}' stroke='{BORDER}' stroke-width='{}'/>",
        N(w),
        N(h),
        N(m.size * 0.3),
        N(m.size / 10.0)
    );
    let row = m.row();
    let base = |k: usize| row * k as f64 + row * 0.5 + m.size * 0.36;
    let r = m.size * 0.3;
    put!(
        s,
        "<path d='M 0 {r} A {r} {r} 0 0 1 {r} 0 H {} A {r} {r} 0 0 1 {} {r} V {row} H 0 Z' fill='{HEAD}'/><line x1='0' y1='{row}' x2='{}' y2='{row}' stroke='{BORDER}' stroke-width='{}'/>",
        N(w - r),
        N(w),
        N(w),
        N(m.size / 10.0),
        r = N(r),
        row = N(row)
    );
    // (The outline again over the band.)
    put!(
        s,
        "<rect x='0' y='0' width='{}' height='{}' rx='{}' fill='none' stroke='{BORDER}' stroke-width='{}'/>",
        N(w),
        N(h),
        N(r),
        N(m.size / 10.0)
    );
    let (ts, tsp) = m.title_type();
    put!(
        s,
        "<text x='{}' y='{}' font-size='{}' font-weight='700' letter-spacing='{}' fill='{ACCENT}'>{}</text>",
        N(m.pad()),
        N(row * 0.5 + ts * 0.36),
        N(ts),
        N(tsp),
        escape(&m.title)
    );
    // The rules between columns, from under the title's band.
    for c in 1..=m.columns.len() {
        let x = m.column_span(c, w).0;
        put!(
            s,
            "<line x1='{}' y1='{}' x2='{}' y2='{}' stroke='{BORDER}' stroke-width='{}'/>",
            N(x),
            N(row + m.size * 0.4),
            N(x),
            N(h - m.size * 0.4),
            N(m.size / 12.0)
        );
    }
    let lefts = m.lefts();
    for (i, (label, enabled)) in m.items.iter().enumerate() {
        let (c, k) = m.place(i);
        let (left, right) = m.column_span(c, w);
        let top = row * (k + 1) as f64;
        if m.heads.contains(&i) {
            if k > 0 {
                put!(
                    s,
                    "<line x1='{}' y1='{}' x2='{}' y2='{}' stroke='{BORDER}' stroke-width='{}'/>",
                    N(left + m.size * 0.4),
                    N(top),
                    N(right - m.size * 0.4),
                    N(top),
                    N(m.size / 12.0)
                );
            }
            let (hs, hsp) = m.head_type();
            put!(
                s,
                "<text x='{}' y='{}' font-size='{}' font-weight='700' letter-spacing='{}' fill='{DIM}'>{}</text>",
                N(lefts[c]),
                N(top + row * 0.5 + hs * 0.36),
                N(hs),
                N(hsp),
                escape(label)
            );
            continue;
        }
        let x = lefts[c] + m.indent(i);
        let marked = m.marked.contains(&i);
        if marked {
            put!(
                s,
                "<circle cx='{}' cy='{}' r='{}' fill='{ACCENT}'/>",
                N(x - m.size * 0.55),
                N(top + row * 0.5),
                N(m.size * 0.16)
            );
        }
        if m.hover == Some(i) && *enabled {
            put!(
                s,
                "<rect x='{}' y='{}' width='{}' height='{}' rx='{}' fill='{HOVER}'/>",
                N(left + m.size * 0.2),
                N(top),
                N(right - left - m.size * 0.4),
                N(row),
                N(m.size * 0.2)
            );
        }
        put!(
            s,
            "<text x='{}' y='{}' font-size='{}' fill='{}'>{}</text>",
            N(x),
            N(base(k + 1)),
            N(m.size),
            if *enabled || marked {
                colour::LEGEND
            } else {
                DIM
            },
            escape(label)
        );
    }
    let e = m.size / 5.0;
    Layer {
        origin: (m.x, m.y),
        bounds: [-e, -e, w + e, h + e],
        body: s.0,
        ..Layer::default()
    }
}

/// A note: its lines (each its text and colour), its lettering's size (drawing units), centred
/// above (`x`, `y`) in the drawing, kept within the panel's width.
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub x: f64,
    pub y: f64,
    pub size: f64,
    pub lines: Vec<(String, Tone)>,
}

impl Note {
    /// Its width and height.
    pub fn extent(&self, fonts: &Fonts) -> (f64, f64) {
        let widest = self
            .lines
            .iter()
            .map(|(t, _)| fonts.advance(t, self.size, Weight::Regular, 0.0))
            .fold(0.0, f64::max);
        (
            widest + self.size * 1.2,
            self.size * (1.5 * self.lines.len() as f64 + 0.3),
        )
    }
}

/// A note's layer: a box with the accent's border above its point, its lines centred.
pub fn note(fonts: &Fonts, n: &Note) -> Layer {
    let (w, h) = n.extent(fonts);
    let gap = n.size * 0.4;
    // Centred on x, but within the panel.
    let left = (n.x - w / 2.0).clamp(0.0, (W - w).max(0.0));
    let top = (n.y - gap - h).max(0.0);
    let (ox, oy) = (left + w / 2.0, top);
    let mut s = Svg::default();
    put!(
        s,
        "<rect x='{}' y='0' width='{}' height='{}' rx='{}' fill='{BACK}' stroke='{ACCENT}' stroke-width='{}'/>",
        N(-w / 2.0),
        N(w),
        N(h),
        N(n.size * 0.3),
        N(n.size / 8.0)
    );
    for (k, (t, tone)) in n.lines.iter().enumerate() {
        put!(
            s,
            "<text x='0' y='{}' text-anchor='middle' font-size='{}' fill='{}'>{}</text>",
            N(n.size * (1.5 * k as f64 + 1.25)),
            N(n.size),
            tone.fill(),
            escape(t)
        );
    }
    let e = n.size / 4.0;
    Layer {
        origin: (ox, oy),
        bounds: [-w / 2.0 - e, -e, w / 2.0 + e, h + e],
        body: s.0,
        ..Layer::default()
    }
}

/// The note's place for a control: centred above its top.
pub fn above(c: &Control) -> (f64, f64) {
    let (x, y) = c.centre();
    (x, y + crate::art::bounds(&c.kind)[1])
}

/// The ring around control `c` being learned: its extent, a little outside it, in the accent.
pub fn ring(c: &Control) -> Layer {
    let b = crate::art::bounds(&c.kind);
    ring_around(c.centre(), [b[0] - 8.0, b[1] - 8.0, b[2] + 8.0, b[3] + 8.0])
}

/// A ring at `origin` around `b` (left, top, right, bottom about it).
pub fn ring_around(origin: (f64, f64), b: [f64; 4]) -> Layer {
    let mut s = Svg::default();
    put!(
        s,
        "<rect x='{}' y='{}' width='{}' height='{}' rx='14' fill='none' stroke='{ACCENT}' stroke-width='6' stroke-dasharray='18 10'/>",
        N(b[0]),
        N(b[1]),
        N(b[2] - b[0]),
        N(b[3] - b[1])
    );
    Layer {
        origin,
        bounds: [b[0] - 5.0, b[1] - 5.0, b[2] + 5.0, b[3] + 5.0],
        body: s.0,
        ..Layer::default()
    }
}

/// The MIDI map mode's mark round a learnable control in view (the CA-72's `docs/decisions.md`
/// R-MIDI-MAP): where it is (left, top, right, bottom, the drawing's units), its controller in
/// small print (`CC 74`) if it has one, whether it waits for one (the learn ring's dashes), and
/// whether it is the one chosen.
#[derive(Clone, Debug, PartialEq)]
pub struct MapMark {
    pub rect: [f64; 4],
    pub label: Option<String>,
    pub waiting: bool,
    /// The control chosen last (what Delete takes the controller of), outlined more strongly.
    pub chosen: bool,
}

/// The small print's size, a tip's lettering's share.
pub const MARK_TEXT: f64 = 0.8;

/// A mark's layer: the drawer's accent laid faintly over the control (a little more over one
/// that has a controller), outlined as the learn ring is but thin and solid, or the ring's
/// dashes while it waits; its controller on a tab in the notes' colours across its outline's foot,
/// `size` its lettering's.
pub fn map_mark(fonts: &Fonts, m: &MapMark, size: f64) -> Layer {
    let [x0, y0, x1, y1] = m.rect;
    let (w, h) = (x1 - x0, y1 - y0);
    let mut s = Svg::default();
    let tint = if m.label.is_some() { 0.16 } else { 0.07 };
    if m.waiting {
        put!(
            s,
            "<rect x='{}' y='{}' width='{}' height='{}' rx='14' fill='{ACCENT}' fill-opacity='{tint}' stroke='{ACCENT}' stroke-width='6' stroke-dasharray='18 10'/>",
            N(-w / 2.0),
            N(-h / 2.0),
            N(w),
            N(h)
        );
    } else {
        let (opacity, width) = if m.chosen { (1.0, 6.0) } else { (0.75, 3.0) };
        put!(
            s,
            "<rect x='{}' y='{}' width='{}' height='{}' rx='14' fill='{ACCENT}' fill-opacity='{tint}' stroke='{ACCENT}' stroke-opacity='{opacity}' stroke-width='{width}'/>",
            N(-w / 2.0),
            N(-h / 2.0),
            N(w),
            N(h)
        );
    }
    let mut bounds = [-w / 2.0 - 6.0, -h / 2.0 - 6.0, w / 2.0 + 6.0, h / 2.0 + 6.0];
    if let Some(t) = &m.label {
        let spacing = size * 0.06;
        let tw = fonts.advance(t, size, Weight::Bold, spacing) + size * 0.9;
        let th = size * 1.4;
        let top = h / 2.0 - th / 2.0;
        put!(
            s,
            "<rect x='{}' y='{}' width='{}' height='{}' rx='{}' fill='{BACK}' stroke='{ACCENT}' stroke-width='{}'/>",
            N(-tw / 2.0),
            N(top),
            N(tw),
            N(th),
            N(th * 0.3),
            N(size / 10.0)
        );
        put!(
            s,
            "<text x='0' y='{}' text-anchor='middle' font-size='{}' font-weight='700' letter-spacing='{}' fill='{ACCENT}'>{}</text>",
            N(top + th / 2.0 + size * 0.36),
            N(size),
            N(spacing),
            escape(t)
        );
        let e = size / 8.0;
        bounds = [
            bounds[0].min(-tw / 2.0 - e),
            bounds[1],
            bounds[2].max(tw / 2.0 + e),
            bounds[3].max(top + th + e),
        ];
    }
    Layer {
        origin: ((x0 + x1) / 2.0, (y0 + y1) / 2.0),
        bounds,
        body: s.0,
        ..Layer::default()
    }
}

/// The panel's area, for placing a menu within it.
pub const PANEL: [f64; 4] = [0.0, 0.0, W, H];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controls::{CONTROLS, index};

    fn menu_at(x: f64, y: f64) -> Menu {
        Menu {
            x,
            y,
            size: 26.0,
            title: "CUTOFF FREQUENCY · CH 1 · CC 74".into(),
            items: vec![
                ("MIDI LEARN".into(), true),
                ("REMOVE MIDI ASSIGNMENT".into(), true),
                ("MIDI ASSIGNMENTS…".into(), true),
            ],
            heads: Vec::new(),
            marked: Vec::new(),
            hover: None,
            columns: Vec::new(),
        }
    }

    /// Each item is found where it is drawn, the title and the margin are the menu's but no
    /// item, and outside it nothing.
    #[test]
    fn a_menus_items_are_found_where_drawn() {
        let fonts = Fonts::new();
        let m = menu_at(100.0, 200.0);
        let (w, h) = m.extent(&fonts);
        let row = m.size * 2.0;
        assert_eq!(m.hit(&fonts, 110.0, 210.0), Some(None), "the title");
        for i in 0..3 {
            let y = 200.0 + row * (i as f64 + 1.5);
            assert_eq!(m.hit(&fonts, 100.0 + w / 2.0, y), Some(Some(i)), "item {i}");
        }
        assert_eq!(m.hit(&fonts, 100.0 + w / 2.0, 200.0 + h - 1.0), Some(None));
        assert_eq!(m.hit(&fonts, 99.0, 210.0), None);
        assert_eq!(m.hit(&fonts, 100.0 + w + 1.0, 210.0), None);
        assert_eq!(m.hit(&fonts, 110.0, 200.0 + h + 1.0), None);
    }

    /// A menu of groups (each a head and `n` items), at (100, 200).
    fn grouped(groups: &[(&str, usize)]) -> Menu {
        let mut items = Vec::new();
        let mut heads = Vec::new();
        for &(head, n) in groups {
            heads.push(items.len());
            items.push((head.to_owned(), false));
            items.extend((0..n).map(|k| (format!("{head} {k}"), true)));
        }
        Menu {
            items,
            heads,
            ..menu_at(100.0, 200.0)
        }
    }

    /// Where each column begins and how many rows each has.
    fn split(m: &Menu) -> (Vec<usize>, Vec<usize>) {
        (
            m.columns.iter().map(|c| c.0).collect(),
            m.column_items().iter().map(|r| r.len()).collect(),
        )
    }

    /// Each item is found where it is drawn, and each column begins right of the widest label
    /// in the one before.
    fn found_where_drawn(fonts: &Fonts, m: &Menu) {
        for i in 0..m.items.len() {
            let (x, y) = m.item_at(i);
            assert_eq!(m.hit(fonts, x + 1.0, y), Some(Some(i)), "{i}");
        }
        let cols = m.column_items();
        for (c, pair) in cols.windows(2).enumerate() {
            let widest = pair[0]
                .clone()
                .map(|i| m.width(fonts, i))
                .fold(0.0, f64::max);
            let left = m.lefts();
            assert!(left[c + 1] - left[c] >= widest, "{c}: {left:?} {widest}");
        }
    }

    /// A menu of groups set in columns, as few as make it fit (the settings', decisions.md
    /// R-READABILITY-MINIMUM): each column begun at a head, the groups as evenly spread as they
    /// allow, its lettering and rows as they were, each item found where it is drawn, its
    /// columns side by side; one that fits stays one column.
    #[test]
    fn a_menu_in_columns_begins_each_at_a_head_and_finds_its_items() {
        let fonts = Fonts::new();
        // (LOOK, LETTERING, TICKS, WINDOW, MPE: three, three, three, four and two rows.)
        let m = grouped(&[
            ("LOOK", 2),
            ("LETTERING", 2),
            ("TICKS", 2),
            ("WINDOW", 3),
            ("MPE", 1),
        ]);
        let (w1, h1) = m.extent(&fonts);
        assert_eq!(
            m.clone().in_columns(&fonts, (W, h1), 3),
            m,
            "it fits: one column"
        );
        let two = m.clone().in_columns(&fonts, (W, h1 * 0.7), 3);
        // (LOOK, LETTERING, TICKS: nine rows; WINDOW, MPE: six. As even as six and nine, the
        // longer first.)
        assert_eq!(split(&two), (vec![9], vec![9, 6]));
        let (w2, h2) = two.extent(&fonts);
        assert_eq!(two.size, m.size);
        assert!((h2 - (two.row() * 10.0 + two.size * 0.4)).abs() < 1e-9 && h2 <= h1 * 0.7);
        assert!(w2 >= w1);
        let tall = |rows: f64| two.row() * (rows + 1.0) + two.size * 0.4;
        // Room for six rows: three columns (LOOK, LETTERING; TICKS; WINDOW, MPE: six, three and
        // six, as even as three, six and six, and the longer first).
        let three = m.clone().in_columns(&fonts, (W, tall(6.0)), 3);
        assert_eq!(split(&three), (vec![6, 9], vec![6, 3, 6]));
        assert_eq!(three.size, m.size);
        for menu in [&m, &two, &three] {
            found_where_drawn(&fonts, menu);
        }
    }

    /// The columns' heights as even as the groups allow, in their order, a group never split
    /// (the owner, 2026-11-04: "MPE sits alone in a short last column"): the settings' groups
    /// (three, three, three, three, four, two and two rows) in four columns of six, six, four
    /// and four, not six, six, six and two; and in as few columns as fit, up to five (the
    /// standalone's, round 5: with STANDALONE and AGENTS, two and five rows more, in five of
    /// six, six, six, four and five where four would be eight tall).
    #[test]
    fn a_menus_columns_are_balanced() {
        let fonts = Fonts::new();
        let m = grouped(&[
            ("LOOK", 2),
            ("LETTERING", 2),
            ("TICKS & TITLES", 2),
            ("FREQUENCY DIALS", 2),
            ("WINDOW", 3),
            ("MOD WHEEL SOURCE", 1),
            ("MPE", 1),
        ]);
        let tall = |rows: f64| m.row() * (rows + 1.0) + m.size * 0.4;
        // Four, not five: five of these groups are no shorter.
        let four = m.clone().in_columns(&fonts, (W, tall(6.0)), 5);
        assert_eq!(split(&four), (vec![6, 12, 16], vec![6, 6, 4, 4]));
        // Room for eight: three columns (six, six and eight), not four.
        let three = m.clone().in_columns(&fonts, (W, tall(8.0)), 5);
        assert_eq!(split(&three), (vec![6, 12], vec![6, 6, 8]));
        // Room for eleven: two (nine and eleven).
        let two = m.clone().in_columns(&fonts, (W, tall(11.0)), 5);
        assert_eq!(split(&two), (vec![9], vec![9, 11]));
        // The standalone's: room for six, five columns (as even as six, six, four, six and
        // five, the longer first); four would be made smaller; room for eight, four.
        let long = grouped(&[
            ("LOOK", 2),
            ("LETTERING", 2),
            ("TICKS & TITLES", 2),
            ("FREQUENCY DIALS", 2),
            ("WINDOW", 3),
            ("MOD WHEEL SOURCE", 1),
            ("MPE", 1),
            ("STANDALONE", 1),
            ("AGENTS", 4),
        ]);
        let five = long.clone().in_columns(&fonts, (W, tall(6.0)), 5);
        assert_eq!(split(&five), (vec![6, 12, 18, 22], vec![6, 6, 6, 4, 5]));
        assert!(long.clone().in_columns(&fonts, (W, tall(6.0)), 4).size < long.size);
        let four_long = long.clone().in_columns(&fonts, (W, tall(8.0)), 5);
        assert_eq!(split(&four_long), (vec![6, 12, 20], vec![6, 6, 8, 7]));
        for menu in [&four, &three, &two, &five, &four_long] {
            assert_eq!(menu.size, m.size);
            assert!(menu.columns.iter().all(|c| menu.heads.contains(&c.0)));
            assert!(menu.extent(&fonts).1 <= tall(11.0) + 1e-9);
            found_where_drawn(&fonts, menu);
        }
    }

    /// A menu that no way of setting in columns fits (LARGE in the smallest window: decisions.md
    /// R-READABILITY-MINIMUM) is made smaller, as little as fits it, filling the room's height or
    /// its width; the way it is set in the one that needs it least, the most even of those.
    #[test]
    fn a_menu_no_columns_fit_is_made_smaller_as_little_as_fits() {
        let fonts = Fonts::new();
        let m = grouped(&[
            ("LOOK", 2),
            ("LETTERING", 2),
            ("TICKS & TITLES", 2),
            ("FREQUENCY DIALS", 2),
            ("WINDOW", 3),
            ("MOD WHEEL SOURCE", 1),
            ("MPE", 1),
        ]);
        let tall = |rows: f64| m.row() * (rows + 1.0) + m.size * 0.4;
        // Room for four rows' height, the width ample: four columns, the tallest six rows, made
        // 10.4 / 14.4 of its size (no way of five columns or fewer has a column under six, and
        // of those that need it as little, four are the fewest).
        let room = (W, tall(4.0));
        let small = m.clone().in_columns(&fonts, room, 5);
        assert_eq!(split(&small), (vec![6, 12, 16], vec![6, 6, 4, 4]));
        let (w, h) = small.extent(&fonts);
        assert!(h <= room.1 && h > room.1 * (1.0 - 1e-6), "{h}");
        assert!(w <= room.0);
        assert!((small.size / m.size - tall(4.0) / tall(6.0)).abs() < 1e-6);
        found_where_drawn(&fonts, &small);
        // Every way, at most five columns: none fits larger.
        for more in 0..5 {
            for starts in choose(&m.heads[1..], more) {
                let mut one = m.clone();
                one.columns = starts.iter().map(|&s| (s, 0.0)).collect();
                let rows = one.column_items().iter().map(|r| r.len()).max().unwrap();
                assert!(rows >= 6, "{starts:?}");
            }
        }
        // Narrow as well: the width it fills, or the height.
        let (w4, _) = m
            .clone()
            .in_columns(&fonts, (W, tall(6.0)), 5)
            .extent(&fonts);
        let room = (w4 * 0.6, tall(4.0));
        let narrow = m.clone().in_columns(&fonts, room, 5);
        let (w, h) = narrow.extent(&fonts);
        assert!(w <= room.0 && h <= room.1, "{w} {h}");
        assert!(
            w > room.0 * (1.0 - 1e-6) || h > room.1 * (1.0 - 1e-6),
            "as little smaller as fits: {w} {h}"
        );
        assert!(narrow.size < small.size);
        found_where_drawn(&fonts, &narrow);
        // A menu that fits is not made smaller.
        assert_eq!(m.clone().in_columns(&fonts, (W, H), 5).size, m.size);
    }

    /// A menu opened at the panel's bottom right corner is moved to fit inside it.
    #[test]
    fn a_menu_fits_within_the_panel() {
        let fonts = Fonts::new();
        let m = menu_at(0.0, 0.0).placed(&fonts, W - 5.0, H - 5.0, PANEL);
        let (w, h) = m.extent(&fonts);
        assert!(m.x >= 0.0 && m.x + w <= W + 1e-9, "{} {w}", m.x);
        assert!(m.y >= 0.0 && m.y + h <= H + 1e-9, "{} {h}", m.y);
    }

    /// A menu taller than the area it is placed in is made smaller to fit it, its lettering
    /// with it; one that fits keeps its size.
    #[test]
    fn a_menu_too_tall_is_made_smaller() {
        let fonts = Fonts::new();
        let m = menu_at(0.0, 0.0);
        let (_, h) = m.extent(&fonts);
        let area = [0.0, 0.0, W, h / 2.0];
        let small = m.clone().placed(&fonts, 10.0, 10.0, area);
        let (w2, h2) = small.extent(&fonts);
        assert!((small.size - m.size / 2.0).abs() < 1e-9, "{}", small.size);
        assert!(small.y >= 0.0 && small.y + h2 <= area[3] + 1e-9);
        assert!(small.x + w2 <= W);
        assert_eq!(m.clone().placed(&fonts, 10.0, 10.0, PANEL).size, m.size);
    }

    /// A note over a control at the panel's left edge stays on the panel.
    #[test]
    fn a_note_stays_on_the_panel() {
        let fonts = Fonts::new();
        let tune = &CONTROLS[index("tune").expect("a control")];
        let (x, y) = above(tune);
        let n = Note {
            x,
            y,
            size: 26.0,
            lines: vec![(
                "A LINE MUCH WIDER THAN THE KNOB IT IS ABOUT, AND THEN SOME MORE".into(),
                Tone::Plain,
            )],
        };
        let l = note(&fonts, &n);
        assert!(l.origin.0 + l.bounds[0] >= -10.0, "{:?}", l);
        assert!(l.origin.1 + l.bounds[1] >= -10.0, "{:?}", l);
    }
}
