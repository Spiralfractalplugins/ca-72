//! A value typed at a knob, in the readouts' dark glass and amber lettering (R-KNOB-ENTRY).

use crate::art::Layer;
use crate::fonts::{Fonts, Weight};
use crate::svg::{N, Svg, escape, put};
use plugin_kit_materials::font;

/// A field in drawing units, kept inside the visible window, its initial value selected.
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub bounds: [f64; 4],
    pub size: f64,
    pub text: String,
    pub unit: &'static str,
    pub selected: bool,
}

/// The unit's lettering, against the digits' size (0.7 left it the smallest print in a 4K
/// window: decisions.md R-READABILITY-MINIMUM).
const UNIT: f64 = 0.8;

impl Field {
    pub fn placed(
        mut self,
        fonts: &Fonts,
        (x, y): (f64, f64),
        [left, top, right, bottom]: [f64; 4],
    ) -> Self {
        let size = self.size;
        let text = self.text.chars().count() as f64 * 6.0 * size / 7.0;
        let unit = fonts.advance(self.unit, size * UNIT, Weight::Regular, 0.0);
        let w = (text + unit + size * 3.0).max(size * 8.0);
        let h = size * 2.2;
        let fit = ((right - left) / (w + size * 0.6))
            .min((bottom - top) / (h + size * 0.6))
            .min(1.0);
        self.size *= fit;
        let (w, h, margin) = (w * fit, h * fit, size * 0.3 * fit);
        let x = (x - w / 2.0).clamp(left + margin, right - margin - w);
        let y = (y - h / 2.0).clamp(top + margin, bottom - margin - h);
        self.bounds = [x, y, x + w, y + h];
        self
    }

    pub fn contains(&self, x: f64, y: f64) -> bool {
        let [x0, y0, x1, y1] = self.bounds;
        (x0..=x1).contains(&x) && (y0..=y1).contains(&y)
    }

    pub(crate) fn layer(&self) -> Layer {
        let [x0, y0, x1, y1] = self.bounds;
        let (w, h, size) = (x1 - x0, y1 - y0, self.size);
        let pad = size * 0.6;
        let pitch = size / 7.0;
        let advance = self.text.chars().count() as f64 * 6.0 * pitch;
        let mut s = Svg::default();
        put!(
            s,
            "<rect width='{}' height='{}' rx='6' fill='#111619' stroke='#f0a030' stroke-width='2'/>",
            N(w),
            N(h)
        );
        if self.selected && !self.text.is_empty() {
            put!(
                s,
                "<rect x='{}' y='{}' width='{}' height='{}' rx='2' fill='#4b3921'/>",
                N(pad - 3.0),
                N(size * 0.45),
                N(advance + 6.0),
                N(size * 1.3)
            );
        } else {
            put!(
                s,
                "<path d='M {} {} h {}' stroke='#f0a030' stroke-width='2'/>",
                N(pad + advance + 2.0),
                N(size * 1.7),
                N(pitch * 5.0)
            );
        }
        // The preset rename field's dot font, with the same steady caret under the next cell.
        for (i, ch) in self.text.chars().enumerate() {
            for (row, bits) in font::glyph(ch).iter().enumerate() {
                for col in 0..5 {
                    let fill = if bits & (1 << (4 - col)) != 0 {
                        "#f0a030"
                    } else {
                        "#24302e"
                    };
                    put!(
                        s,
                        "<circle cx='{}' cy='{}' r='{}' fill='{fill}'/>",
                        N(pad + (i * 6 + col) as f64 * pitch),
                        N(size * 0.55 + row as f64 * pitch),
                        N(pitch * 0.36)
                    );
                }
            }
        }
        put!(
            s,
            "<text x='{}' y='{}' text-anchor='end' font-size='{}' fill='#b5b0a4'>{}</text>",
            N(w - pad),
            N(size * 1.45),
            N(size * UNIT),
            escape(self.unit)
        );
        Layer {
            origin: (x0, y0),
            bounds: [-2.0, -2.0, w + 2.0, h + 2.0],
            body: s.0,
            ..Layer::default()
        }
    }
}
