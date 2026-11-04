//! Rendering the panel: the background once for a scale, each moving part's layer again only
//! when its art changes, and the frame composed of them; in either skin (`crate::skin`), the
//! worn one's pictures turned and moved in the drawn parts' places.

use std::fmt;
use std::sync::Arc;

use resvg::tiny_skia::{
    BlendMode, Color, FillRule, FilterQuality, IntSize, Paint, PathBuilder, Pattern, Pixmap,
    PixmapPaint, Point, Rect, SpreadMode, Transform,
};
use resvg::usvg;

use crate::arrangement::Arrangement;
use crate::art::{self, Blend, H, Layer, Layout, Lettering, W};
use crate::controls::{CONTROLS, Kind, Mark, feedback_silent};
use crate::fonts::{FAMILY, Fonts};
use crate::learn::{self, Menu, Note};
use crate::look::Look;
use crate::skin::{Part, Pictures, Skin};
use crate::ultra::Glow;

/// A skin pack's drawing over `frame` at `scale`, its view stretched over the panel (the
/// wooden strip and the face, the drawing's whole width, from the frame's top to `foot`, the
/// row the plug-in's strip begins at) and nothing outside it.
fn draw_pack_art(frame: &mut Pixmap, scale: f64, foot: i32, drawing: &usvg::Tree) {
    let (pw, ph) = ((W * scale).round().max(1.0) as u32, foot.max(1) as u32);
    let Some(mut layer) = Pixmap::new(pw, ph) else {
        return;
    };
    let size = drawing.size();
    // (Not drawn if it cannot be within its buffers' limits at this size.)
    if !skin_pack::svg::render(
        drawing,
        Transform::from_scale(pw as f32 / size.width(), ph as f32 / size.height()),
        &mut layer.as_mut(),
    ) {
        return;
    }
    frame.draw_pixmap(
        0,
        0,
        layer.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        None,
    );
}

/// A skin pack's drawing of the strip over `frame` at `scale`, stretched over the plug-in's strip
/// (the rail and the sections, the drawing's whole width, from the frame's row `foot` down).
fn draw_strip_art(frame: &mut Pixmap, scale: f64, foot: i32, drawing: &usvg::Tree) {
    let (pw, ph) = (
        (W * scale).round().max(1.0) as u32,
        (frame.height() as i32 - foot).max(1) as u32,
    );
    let Some(mut layer) = Pixmap::new(pw, ph) else {
        return;
    };
    let size = drawing.size();
    // (Not drawn if it cannot be within its buffers' limits at this size.)
    if !skin_pack::svg::render(
        drawing,
        Transform::from_scale(pw as f32 / size.width(), ph as f32 / size.height()),
        &mut layer.as_mut(),
    ) {
        return;
    }
    frame.draw_pixmap(
        0,
        foot,
        layer.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        None,
    );
}

/// The compact editor's grip, within the rail's bottom right corner: as plain as the
/// drawing's (R-SIZE).
pub fn compact_grip(frame: &mut Pixmap, scale: f64) {
    let (w, h) = (frame.width() as f32, frame.height() as f32);
    let s = scale as f32;
    let mut path = PathBuilder::new();
    for k in 1..=4 {
        let d = 8.0 * k as f32 * s;
        path.move_to(w - 5.0 * s - d, h - 5.0 * s);
        path.line_to(w - 5.0 * s, h - 5.0 * s - d);
    }
    if let Some(path) = path.finish() {
        let mut paint = Paint::default();
        paint.set_color_rgba8(217, 210, 192, 166);
        frame.stroke_path(
            &path,
            &paint,
            &resvg::tiny_skia::Stroke {
                width: 3.0 * s,
                line_cap: resvg::tiny_skia::LineCap::Round,
                ..Default::default()
            },
            Transform::identity(),
            None,
        );
    }
}

/// The standalone's TYPING, PANIC and MIDI keys (decisions.md R-TEMPO-CORNER, R-MIDI-MAP):
/// TYPING on, PANIC held down, the MIDI map mode on, the key under the pointer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TempoKeys {
    pub typing: bool,
    pub panic: bool,
    pub midi: bool,
    pub hover: Option<art::TempoKey>,
}

/// What the panel shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    /// Each control's value, normalized, by its index in [`CONTROLS`].
    pub values: [f64; CONTROLS.len()],
    /// A MIDI keyboard's pitch bend (0 fully down, 0.5 centred, 1 fully up) and its
    /// modulation wheel (0..1): the panel's wheels are drawn where they take them.
    pub midi: (f64, f64),
    /// On (playing), or bypassed.
    pub power: bool,
    /// The OVERLOAD lamp's level, 0 dark to 1 fully lit.
    pub overload: f64,
    /// QUALITY's opening (decisions.md R48): what it shows and how far each has come.
    pub opening: crate::ultra::Opening,
    /// DETUNE unused (SCATTER): its knob drawn dimmed, as FEEDBACK's is with EXTERNAL INPUT
    /// shut.
    pub detune_off: bool,
    /// VOICES where it does nothing (the plug-in's `voices_inert`): dimmed, as DETUNE.
    pub voices_off: bool,
    /// OSC. 3 CONTROL while oscillator 3 is timed (its key on, its RANGE at LO; decisions.md
    /// R-OSC-TIMED), where it does nothing: dimmed, as VOICES.
    pub osc3_control_off: bool,
    /// A tip: its text, centred above (x, y) in the drawing.
    pub tip: Option<(String, f64, f64)>,
    /// MIDI Learn (decisions.md R34): the control being learned, by its index in [`CONTROLS`]
    /// (ringed), a note over the panel saying what it does, and a control's menu.
    pub learning: Option<usize>,
    pub note: Option<Note>,
    pub menu: Option<Menu>,
    /// An exact value being typed over a knob (R-KNOB-ENTRY).
    pub entry: Option<crate::value_entry::Field>,
    /// The standalone's TEMPO screen's text (`120 BPM`; decisions.md R-ARP), none in a plug-in.
    pub tempo: Option<String>,
    /// The standalone's TYPING, PANIC and MIDI keys after the TEMPO screen (decisions.md
    /// R-TEMPO-CORNER, R-MIDI-MAP), none in a plug-in.
    pub tempo_keys: Option<TempoKeys>,
    /// The MIDI map mode's marks (R-MIDI-MAP): every learnable control in view, outlined, its
    /// controller in small print; none while it is off.
    pub marks: Vec<learn::MapMark>,
    /// Oscillators 2's and 3's timed mode (decisions.md R-OSC-TIMED): each key's sign's
    /// division and whether it is inert (its RANGE away from LO: off the panel). A key on and
    /// not inert lights, and its oscillator's FREQUENCY dial is printed in note values (the knob drawn
    /// at its DIVISION, `values` holding that).
    pub timed: [Timed; 2],
    /// Each oscillator's waveform (decisions.md R-CONTINUOUS-WAVE): CONTINUOUS (its WAVEFORM's
    /// arc printed, its MOD key on the panel, its switch where its shape is) or STEPPED, MOD's
    /// way and its orb.
    pub shape: [Shaped; 3],
    /// GLIDE in LONG: ×5 printed beside its dial (decisions.md R-GLIDE-LONG).
    pub glide_long: bool,
    /// The left hand's VELOCITY meter's lamps lit (decisions.md R-VELOCITY:
    /// `strip::velocity::lit`).
    pub velocity_lit: usize,
}

/// An oscillator's waveform as the panel shows it (decisions.md R-CONTINUOUS-WAVE).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Shaped {
    pub continuous: bool,
    /// Where its switch is, in its steps from TRIANGLE (0 to [`art::SHAPE_PLACES`]).
    pub at: f64,
    /// Which way MOD moves it (its key's colour), and where MOD has it now (its orb), if MOD
    /// is moving it.
    pub way: art::ModWay,
    pub orb: Option<f64>,
}

/// Each oscillator's WAVEFORM and MOD key, oscillator 1's to 3's.
pub const WAVEFORMS: [&str; 3] = ["osc1_waveform", "osc2_waveform", "osc3_waveform"];
pub const MOD_KEYS: [&str; 3] = ["osc1_shape_mod", "osc2_shape_mod", "osc3_shape_mod"];

/// An oscillator's timed mode as the panel shows it (decisions.md R-OSC-TIMED).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timed {
    /// Its division, by its place among the eighteen (`params::Division`'s order).
    pub division: usize,
    /// Its RANGE away from LO: its key and sign not on the panel, its dial in semitones.
    pub inert: bool,
}

impl Default for Timed {
    fn default() -> Self {
        Self {
            division: 8,
            inert: false,
        }
    }
}

/// The TIMED keys' and the FREQUENCY knobs' parameters, oscillator 2's then 3's.
pub const TIMED_KEYS: [&str; 2] = ["osc2_timed", "osc3_timed"];
pub const TIMED_DIALS: [&str; 2] = ["osc2_frequency", "osc3_frequency"];

impl Scene {
    /// Which oscillators' WAVEFORM switches are printed CONTINUOUS (their arc; oscillator 3's
    /// legends): each while its waveform is (decisions.md R-CONTINUOUS-WAVE).
    pub fn arcs(&self) -> [bool; 3] {
        self.shape.map(|s| s.continuous)
    }

    /// Which oscillators' FREQUENCY dials are printed in note values: oscillator 2's and 3's,
    /// each while its key is on and its RANGE at LO.
    pub fn timed_dials(&self) -> [bool; 2] {
        std::array::from_fn(|n| {
            crate::controls::index(TIMED_KEYS[n]).is_some_and(|i| self.values[i] >= 0.5)
                && !self.timed[n].inert
        })
    }
}

impl Default for Scene {
    fn default() -> Self {
        Self {
            values: [0.0; CONTROLS.len()],
            midi: (0.5, 0.0),
            power: true,
            overload: 0.0,
            opening: crate::ultra::Opening::default(),
            detune_off: false,
            voices_off: false,
            osc3_control_off: false,
            tip: None,
            learning: None,
            note: None,
            menu: None,
            entry: None,
            tempo: None,
            tempo_keys: None,
            marks: Vec::new(),
            timed: [Timed::default(); 2],
            shape: [Shaped::default(); 3],
            glide_long: false,
            velocity_lit: 0,
        }
    }
}

/// A layer as last rendered: its art, and its pixels at their place in the frame.
#[derive(Default)]
struct Slot {
    layer: Option<Layer>,
    pixels: Option<(i32, i32, Pixmap)>,
}

/// The panel's renderer at a scale.
pub struct Renderer {
    fonts: Fonts,
    /// What the panel is drawn with (a built-in look, or a skin pack's), and where everything
    /// on it is (its layout).
    look: Look,
    arrangement: Arc<Arrangement>,
    layout: Layout,
    options: usvg::Options<'static>,
    /// Pixels a panel unit; pixels a logical pixel (for the tip, sized for reading).
    scale: f64,
    ui: f64,
    background: Pixmap,
    frame: Pixmap,
    slots: Vec<Slot>,
    /// The scene the frame shows.
    shown: Option<Scene>,
    /// The pixels the last render changed: left, top, right, bottom (the last two past the
    /// change).
    damage: Option<[i32; 4]>,
    /// The same, as the separate places it is in (a few, those that overlap or nearly merged):
    /// each put together again on its own (the CA-74's review: one rectangle round all of them
    /// put most of the panel together again for QUALITY's opening and a lamp at once).
    damages: Vec<[i32; 4]>,
    skin: Skin,
    /// The worn skin's pictures, once it has been shown.
    pictures: Option<Pictures>,
    /// The lights' pictures made at this scale, each in its layer's place (left, top, width,
    /// height: pixels), drawn as their layers change (decisions.md R58).
    lights: Vec<(Glow, [i32; 4], Pixmap)>,
    /// The overlays drawn at this scale, each in its layer's place (decisions.md R59).
    overlays: Vec<(String, [i32; 4], Pixmap)>,
    /// Pictures placed in layers (turned, mirrored, scaled, moved by fractions of a pixel), kept
    /// to be put again as they were without being resampled, the least lately used given up
    /// past [`PLACED_BYTES`] (decisions.md R59); and the count of the renders, their clock.
    placed: Vec<Placed>,
    renders: u64,
    /// The TEMPO screen's bezel and glass at this scale, in its layer's place (left, top,
    /// width, height: pixels), once it is drawn.
    screens: Option<([i32; 4], Pixmap)>,
    /// The background again where oscillators 2's and 3's FREQUENCY dials are, printed in note
    /// values (decisions.md R-OSC-TIMED), drawn the first time one is shown at this scale; and
    /// where each dial's print is in the frame (pixels: left, top, right, bottom, clamped to
    /// it), laid over the background there while that dial is timed.
    timed_background: Option<[Pixmap; 2]>,
    timed_rects: [[i32; 4]; 2],
    /// The worn skin's face's mean level under its print, as the whole background measured it:
    /// a part's print is worn by it.
    face_mean: Option<f64>,
    /// The timed dials' places in the background under its print (its surfaces), kept as it is
    /// painted.
    dial_surfaces: Option<[Pixmap; 2]>,
    /// The same for each oscillator's WAVEFORM switch's arc (decisions.md R-CONTINUOUS-WAVE): its
    /// place printed with the arc, drawn the first time one is CONTINUOUS at this scale, laid
    /// over the background while it is; the place in the frame; its surfaces under the print.
    arc_background: Option<[Pixmap; 3]>,
    arc_rects: [[i32; 4]; 3],
    arc_surfaces: Option<[Pixmap; 3]>,
    /// GLIDE's LONG sign, printed and kept as the arcs are, laid only while LONG is on.
    glide_background: Option<Pixmap>,
    glide_rect: [i32; 4],
    glide_surface: Option<Pixmap>,
    /// The strip's bare face under the left area, as the background had it before anything was
    /// drawn on it (decisions.md R-MACROS: the macros' plate is laid on it, the wheels left
    /// out), and its place in the frame.
    left_face: Option<Pixmap>,
    left_rect: [i32; 4],
}

/// How much the pictures placed in layers may keep: some dozens of layers' pictures.
const PLACED_BYTES: usize = 32 << 20;

/// A picture placed in a layer, as [`Placed::key`] says, from its pixel `at` in the layer.
struct Placed {
    key: PlacedKey,
    at: (i32, i32),
    pixels: Pixmap,
    used: u64,
}

/// A picture's part at its size in pixels, where its middle is in its layer's pixels, how far
/// it is turned (degrees), mirrored and scaled.
#[derive(Clone, Copy, PartialEq)]
struct PlacedKey {
    part: Part,
    size: (u32, u32),
    at: (u64, u64),
    deg: u32,
    flip: (bool, bool),
    zoom: u32,
}

impl fmt::Debug for Renderer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Renderer")
            .field("scale", &self.scale)
            .field("size", &self.size())
            .finish()
    }
}

/// The frame's size at `scale` pixels a panel unit, the official panel's.
pub fn size_at(scale: f64) -> (u32, u32) {
    size_for(scale, art::Panel::OFFICIAL)
}

/// The frame's size at `scale` for a panel of `panel`'s size: from its top (the drawing's
/// [`art::Panel::top`], the frame's first row: [`top_row`]) to the drawing's foot.
pub fn size_for(scale: f64, panel: art::Panel) -> (u32, u32) {
    (
        (W * scale).ceil().max(1.0) as u32,
        ((H * scale).ceil() as i32 - top_row(scale, panel)).max(1) as u32,
    )
}

/// The drawing's row (at `scale`) the frame of a panel of `panel`'s size begins at: 0 for the
/// official panel's, less for a taller one.
pub fn top_row(scale: f64, panel: art::Panel) -> i32 {
    (panel.top() * scale).round() as i32
}

/// The tip's lettering, in logical pixels.
const TIP_TEXT: f64 = 12.0;

impl Renderer {
    /// A renderer drawing `scale` pixels a panel unit, `ui` pixels a logical pixel, in the
    /// drawn skin.
    pub fn new(scale: f64, ui: f64) -> Self {
        Self::with_skin(Skin::Drawn, scale, ui)
    }

    /// A renderer drawing the panel in `skin`.
    pub fn with_skin(skin: Skin, scale: f64, ui: f64) -> Self {
        Self::with_look(skin, Lettering::Standard, scale, ui)
    }

    /// A renderer drawing the panel in `skin`, its lettering printed at `lettering`'s sizes
    /// (and the tip's, a note's and the menus' with it).
    pub fn with_look(skin: Skin, lettering: Lettering, scale: f64, ui: f64) -> Self {
        Self::with_pack(Look::of(skin), lettering, scale, ui)
    }

    /// A renderer drawing the panel in `look` (a skin pack's, or a built-in one).
    pub fn with_pack(look: Look, lettering: Lettering, scale: f64, ui: f64) -> Self {
        let mut r = Self::blank(look, lettering, scale, ui, true);
        r.draw_background();
        r
    }

    /// A drawer's layers alone, with the same pictures and light, without painting another
    /// whole panel just to turn its knobs (decisions.md R-FX-PARAMS).
    pub(crate) fn for_parts(skin: Skin, scale: f64) -> Self {
        Self::blank(Look::of(skin), Lettering::Standard, scale, 1.0, false)
    }

    fn blank(look: Look, lettering: Lettering, scale: f64, ui: f64, panel: bool) -> Self {
        let skin = look.base;
        let arrangement = Arc::clone(&look.arrangement);
        let fonts = look
            .fonts
            .and_then(|(regular, bold)| Fonts::with(regular, bold).ok())
            .unwrap_or_default();
        let layout = Layout::measure_in(&fonts, lettering, &arrangement);
        let options = usvg::Options {
            fontdb: fonts.database(),
            font_family: FAMILY.into(),
            ..usvg::Options::default()
        };
        let (w, h) = if panel {
            size_for(scale, arrangement.panel)
        } else {
            (1, 1)
        };
        let empty = || Pixmap::new(w, h).expect("a frame of at least a pixel");
        Self {
            fonts,
            look,
            arrangement,
            layout,
            options,
            scale,
            ui,
            background: empty(),
            frame: empty(),
            slots: Vec::new(),
            shown: None,
            damage: None,
            damages: Vec::new(),
            skin,
            pictures: None,
            lights: Vec::new(),
            overlays: Vec::new(),
            placed: Vec::new(),
            renders: 0,
            screens: None,
            timed_background: None,
            timed_rects: [[0; 4]; 2],
            face_mean: None,
            dial_surfaces: None,
            arc_background: None,
            arc_rects: [[0; 4]; 3],
            arc_surfaces: None,
            glide_background: None,
            left_face: None,
            left_rect: [0; 4],
            glide_rect: [0; 4],
            glide_surface: None,
        }
    }

    /// The skin the panel is drawn in.
    pub fn skin(&self) -> Skin {
        self.skin
    }

    /// The size its lettering is printed at.
    pub fn lettering(&self) -> Lettering {
        self.layout.lettering()
    }

    /// Redraws everything at a new scale.
    pub fn rescale(&mut self, scale: f64, ui: f64) {
        if scale == self.scale && ui == self.ui {
            return;
        }
        let (w, h) = size_for(scale, self.arrangement.panel);
        self.scale = scale;
        self.ui = ui;
        self.background = Pixmap::new(w, h).expect("a frame of at least a pixel");
        self.frame = self.background.clone();
        self.slots.clear();
        self.shown = None;
        if let Some(p) = &mut self.pictures {
            p.forget_scale();
        }
        self.lights.clear();
        self.overlays.clear();
        self.placed.clear();
        self.screens = None;
        self.draw_background();
    }

    /// The arrangement with oscillators 2's and 3's FREQUENCY dials in note values.
    fn timed_arrangement(&self) -> Arrangement {
        let mut a = (*self.arrangement).clone();
        for c in a.controls.iter_mut() {
            if TIMED_DIALS.contains(&c.param)
                && let Kind::Knob { big, .. } = c.kind
            {
                c.kind = Kind::Knob {
                    dial: crate::controls::Dial::Timed,
                    big,
                };
            }
        }
        a
    }

    /// Where each of oscillators 2's and 3's FREQUENCY dials' print is in the frame, either
    /// dial's (pixels: left, top, right, bottom).
    /// Where each WAVEFORM switch's arc is in the frame (pixels: left, top, right, bottom).
    fn waveform_arc_rects(&self) -> [[i32; 4]; 3] {
        let (s, top) = (self.scale, self.top_px());
        let (fw, fh) = (
            self.background.width() as i32,
            self.background.height() as i32,
        );
        std::array::from_fn(|n| {
            let Some(c) = self
                .arrangement
                .controls
                .iter()
                .find(|c| c.param == WAVEFORMS[n] && c.place == crate::controls::Place::Panel)
            else {
                return [0; 4];
            };
            let (x, y) = c.centre();
            let e = art::ARC_EXTENT;
            [
                (((x + e[0]) * s).floor() as i32).clamp(0, fw),
                (((y + e[1]) * s).floor() as i32 - top).clamp(0, fh),
                (((x + e[2]) * s).ceil() as i32).clamp(0, fw),
                (((y + e[3]) * s).ceil() as i32 - top).clamp(0, fh),
            ]
        })
    }

    fn glide_long_rect(&self) -> [i32; 4] {
        let Some(c) = self
            .arrangement
            .controls
            .iter()
            .find(|c| c.param == "glide" && c.place == crate::controls::Place::Panel)
        else {
            return [0; 4];
        };
        let (x, y) = c.centre();
        let (s, top) = (self.scale, self.top_px());
        let (w, h) = (
            self.background.width() as i32,
            self.background.height() as i32,
        );
        let e = art::GLIDE_LONG_EXTENT;
        [
            (((x + e[0]) * s).floor() as i32).clamp(0, w),
            (((y + e[1]) * s).floor() as i32 - top).clamp(0, h),
            (((x + e[2]) * s).ceil() as i32).clamp(0, w),
            (((y + e[3]) * s).ceil() as i32 - top).clamp(0, h),
        ]
    }

    /// The left area's place in the frame (`strip::macros::AREA`), its pixels whole.
    pub fn left_rect(&self) -> [i32; 4] {
        let (s, top) = (self.scale, self.top_px());
        let (x0, y0, x1, y1) = crate::strip::macros::AREA;
        [
            (x0 * s).ceil() as i32,
            (y0 * s).ceil() as i32 - top,
            (x1 * s).floor() as i32,
            (y1 * s).floor() as i32 - top,
        ]
    }

    /// The strip's bare face under the left area (`left_rect`), as the background had it before
    /// anything was drawn on it: the worn skin's textured black (under its lamp: `lamp_over`),
    /// the drawn skin's black.
    pub fn left_face(&self) -> Option<&Pixmap> {
        self.left_face.as_ref()
    }

    /// The lamp's light over `p`, its top left the frame's pixel `at`, as over the background
    /// (the worn skin's; the drawn skin has none).
    pub fn lamp_over(&self, p: &mut Pixmap, at: (i32, i32)) {
        if self.skin == Skin::Worn {
            lamp_in(p, self.scale, at, self.drawing_h());
        }
    }

    /// The drawing's height from the panel's top (the lamp's falloff is over it).
    pub fn drawing_h(&self) -> f64 {
        self.arrangement.panel.drawing_h()
    }

    fn dial_rects(&self) -> [[i32; 4]; 2] {
        let (s, top) = (self.scale, self.top_px());
        std::array::from_fn(|n| {
            let Some(c) = self
                .arrangement
                .controls
                .iter()
                .find(|c| c.param == TIMED_DIALS[n])
            else {
                return [0; 4];
            };
            let (x, y) = c.centre();
            let e = art::print_extent(&c.kind);
            let (fw, fh) = (
                self.background.width() as i32,
                self.background.height() as i32,
            );
            // (A little over, for the print's stroke and its blur into the face.)
            [
                (((x + e[0] - 4.0) * s).floor() as i32).clamp(0, fw),
                (((y + e[1] - 4.0) * s).floor() as i32 - top).clamp(0, fh),
                (((x + e[2] + 4.0) * s).ceil() as i32).clamp(0, fw),
                (((y + e[3] + 4.0) * s).ceil() as i32 - top).clamp(0, fh),
            ]
        })
    }

    pub fn scale(&self) -> f64 {
        self.scale
    }

    pub fn size(&self) -> (u32, u32) {
        (self.frame.width(), self.frame.height())
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// The drawing's row its frame begins at ([`top_row`]): the frame's pixel `y` is the
    /// drawing's `y + top_px()`.
    pub fn top_px(&self) -> i32 {
        top_row(self.scale, self.arrangement.panel)
    }

    /// The drawing's y at the frame's top (its first row's).
    pub fn top(&self) -> f64 {
        f64::from(self.top_px()) / self.scale
    }

    /// The frame's row the plug-in's strip begins at (the panel's foot).
    pub fn strip_row(&self) -> i32 {
        (art::PANEL_H * self.scale).round() as i32 - self.top_px()
    }

    /// Where everything on the panel is.
    pub fn arrangement(&self) -> &Arrangement {
        &self.arrangement
    }

    /// What the panel is drawn with.
    pub fn look(&self) -> &Look {
        &self.look
    }

    pub fn fonts(&self) -> &Fonts {
        &self.fonts
    }

    /// The frame last composed (premultiplied RGBA, opaque).
    pub fn frame(&self) -> &Pixmap {
        &self.frame
    }

    fn draw_background(&mut self) {
        // (The timed dials' places, whose surfaces are kept as they are painted: R-OSC-TIMED.)
        self.timed_rects = self.dial_rects();
        self.timed_background = None;
        self.arc_rects = self.waveform_arc_rects();
        self.arc_background = None;
        self.glide_rect = self.glide_long_rect();
        self.glide_background = None;
        self.left_rect = self.left_rect();
        self.left_face = None;
        if self.skin == Skin::Worn {
            self.draw_worn_background();
            return;
        }
        // (The drawn skin's strip face: its black, as `art::strip_face` has it.)
        let r = self.left_rect;
        if let Some(mut face) =
            Pixmap::new((r[2] - r[0]).max(1) as u32, (r[3] - r[1]).max(1) as u32)
        {
            let doc = art::document(
                [0.0, 0.0, f64::from(face.width()), f64::from(face.height())],
                face.width(),
                face.height(),
                &format!(
                    "<rect width='100%' height='100%' fill='{}'/>",
                    crate::svg::colour::PANEL
                ),
            );
            if let Ok(tree) = usvg::Tree::from_str(&doc, &self.options) {
                resvg::render(&tree, Transform::identity(), &mut face.as_mut());
            }
            self.left_face = Some(face);
        }
        let (w, h) = (self.background.width(), self.background.height());
        let view = [
            0.0,
            self.top(),
            f64::from(w) / self.scale,
            f64::from(h) / self.scale,
        ];
        self.background
            .fill(Color::from_rgba8(0x3b, 0x22, 0x13, 0xff));
        let svg = |frame: &mut Pixmap, body: &str, options: &usvg::Options<'_>| {
            let doc = art::document(view, w, h, body);
            if let Ok(tree) = usvg::Tree::from_str(&doc, options) {
                resvg::render(&tree, Transform::identity(), &mut frame.as_mut());
            }
        };
        match (self.look.art.clone(), self.look.strip_art.clone()) {
            // (The surfaces, then the panel's print over them: the timed dials' places kept between.)
            (None, None) => {
                svg(
                    &mut self.background,
                    &art::background_surfaces(&self.layout, &self.arrangement),
                    &self.options,
                );
                self.keep_dial_surfaces();
                svg(
                    &mut self.background,
                    &art::panel_print(&self.layout, &self.arrangement),
                    &self.options,
                );
            }
            // A pack's drawings between the panel's and the strip's surfaces and their print
            // (the strip's print laid again over its drawing).
            (panel, strip) => {
                svg(
                    &mut self.background,
                    &art::background_surfaces(&self.layout, &self.arrangement),
                    &self.options,
                );
                let foot = self.strip_row();
                if let Some(drawing) = panel {
                    draw_pack_art(&mut self.background, self.scale, foot, &drawing);
                }
                if let Some(drawing) = &strip {
                    draw_strip_art(&mut self.background, self.scale, foot, drawing);
                }
                self.keep_dial_surfaces();
                svg(
                    &mut self.background,
                    &art::panel_print(&self.layout, &self.arrangement),
                    &self.options,
                );
                if strip.is_some() {
                    svg(
                        &mut self.background,
                        &art::strip_printed(&self.layout, &self.arrangement),
                        &self.options,
                    );
                }
            }
        }
        self.frame = self.background.clone();
    }

    /// The worn skin's background: walnut, the faces' textured black, the shadows and the
    /// trim, the print over them, the screws and jacks, and the lamp's falloff over all.
    fn draw_worn_background(&mut self) {
        let s = self.scale;
        let (w, h) = (self.background.width(), self.background.height());
        // (The frame's top in the drawing, and the panel's size.)
        let top = self.top();
        let panel = self.arrangement.panel;
        let foot = self.strip_row();
        let (overrides, tint, turn) = (
            &self.look.pictures,
            self.look.face_tint,
            self.look.caps_turn,
        );
        let pictures = self
            .pictures
            .get_or_insert_with(|| Pictures::load_with(overrides, tint, turn));
        let (arrangement, options) = (&*self.arrangement, &self.options);
        let view = [0.0, top, f64::from(w) / s, f64::from(h) / s];
        let tree =
            |body: &str| usvg::Tree::from_str(&art::document(view, w, h, body), options).ok();
        // The wood and the faces, their rows drawn in bands side by side, and the shadows, the
        // trims' and the moving parts', each drawn side by side with the others: at a new scale
        // they were most of the time the editor's window waited (decisions.md R-LINUX-RESIZE).
        let (wood, face) = (&pictures.walnut, &pictures.face);
        in_bands(&mut self.background, |bg, row| {
            let top = top + f64::from(row) / s;
            bg.fill(Color::from_rgba8(0x1d, 0x1b, 0x1a, 0xff));
            // The wood: its grain along the top strip and the strip's rail; each board its own
            // band of the picture, drawn out along the grain.
            let along = (art::W * s) as f32 / wood.width() as f32;
            let across = along / 2.2;
            for (y, hh, band) in [
                (panel.top(), panel.wood, 40.0),
                (art::PANEL_H, crate::strip::RAIL, 260.0),
            ] {
                if hh <= 0.0 {
                    continue;
                }
                fill(
                    bg,
                    wood,
                    (0.0, (y - top) * s, art::W * s, hh * s),
                    None,
                    Transform::from_row(
                        along,
                        0.0,
                        0.0,
                        across,
                        0.0,
                        ((y - top) * s) as f32 - band * across,
                    ),
                );
            }
            // The faces: the picture 640 units across, mirrored tile to tile.
            let k = (640.0 * s) as f32 / face.width() as f32;
            let at = |x: f64, y: f64| {
                Transform::from_scale(k, k).post_translate((x * s) as f32, ((y - top) * s) as f32)
            };
            fill(
                bg,
                face,
                (
                    art::COL * s,
                    (panel.face_top() - top) * s,
                    art::PW * s,
                    panel.face * s,
                ),
                None,
                at(art::COL, panel.face_top()),
            );
            let strip_top = art::PANEL_H + crate::strip::RAIL;
            fill(
                bg,
                face,
                (
                    0.0,
                    (strip_top - top) * s,
                    art::W * s,
                    (art::H - strip_top) * s,
                ),
                None,
                at(320.0, strip_top),
            );
        });
        let bg = &mut self.background;
        // (The left area's bare face, before the left hand's shadows, print and parts.)
        self.left_face = Some(crop(bg, self.left_rect));
        if let Some(overlay) = tree(&art::worn_overlay(arrangement)) {
            render_parts(&overlay, bg);
        }
        // A skin pack's drawing, over the surfaces and under the print.
        if let Some(drawing) = &self.look.art {
            draw_pack_art(bg, s, foot, drawing);
        }
        if let Some(drawing) = &self.look.strip_art {
            draw_strip_art(bg, s, foot, drawing);
        }
        // The timed dials' places as they are under the print (R-OSC-TIMED), and the face's mean
        // level, which the print is worn by.
        let rects = self.timed_rects;
        self.dial_surfaces = Some(rects.map(|r| crop(bg, r)));
        let rects = self.arc_rects;
        self.arc_surfaces = Some(rects.map(|r| crop(bg, r)));
        self.glide_surface = Some(crop(bg, self.glide_rect));
        let mean = face_mean(bg);
        self.face_mean = Some(mean);
        // The print, worn into the face: its texture showing through it a little.
        if let Some(mut print) = Pixmap::new(w, h) {
            if let Some(tree) = tree(&art::printed(&self.layout, arrangement)) {
                resvg::render(&tree, Transform::identity(), &mut print.as_mut());
            }
            worn_into_by(&mut print, bg, mean);
            bg.draw_pixmap(
                0,
                0,
                print.as_ref(),
                &PixmapPaint {
                    opacity: 0.94,
                    ..PixmapPaint::default()
                },
                Transform::identity(),
                None,
            );
        }
        // The screws, each turned as it was last tightened, and the jacks.
        for (i, (x, y, r)) in art::screws(arrangement).into_iter().enumerate() {
            let d = (2.0 * r * s).round().max(1.0) as u32;
            let deg = (i as f32 * 47.0) % 180.0;
            put_picture(
                bg,
                pictures.part(Part::Screw, d, d),
                (x * s, (y - top) * s),
                deg,
                (false, false),
                (1.0, 1.0),
            );
        }
        for (x, y, r, nut) in art::jacks(arrangement) {
            let d = (2.0 * if nut { r + 6.0 } else { r * 1.12 } * s)
                .round()
                .max(1.0) as u32;
            put_picture(
                bg,
                pictures.part(Part::Jack, d, d),
                (x * s, (y - top) * s),
                20.0,
                (false, false),
                (1.0, 1.0),
            );
        }
        // The lamp's light over all of it.
        let height = panel.drawing_h();
        in_bands(bg, |bg, row| lamp_in(bg, s, (0, row), height));
        self.frame = self.background.clone();
    }
    /// The timed dials' places in the background as it is now (its surfaces, before the print).
    fn keep_dial_surfaces(&mut self) {
        let bg = &self.background;
        self.dial_surfaces = Some(self.timed_rects.map(|r| crop(bg, r)));
        self.arc_surfaces = Some(self.arc_rects.map(|r| crop(bg, r)));
        self.glide_surface = Some(crop(bg, self.glide_rect));
    }

    /// The timed dial `n`'s place (oscillator 2's or 3's) printed as `timed` has the dials
    /// (decisions.md R-OSC-TIMED): its surfaces as they were kept under the print, the dials'
    /// ink printed on them as the whole background's is (worn into the face, the lamp over it),
    /// its pixels that and the background's own wherever the two dials' ink is the same (their
    /// edges, and whatever else is printed there), so it is laid without a seam. A few
    /// milliseconds: the dials' ink alone is drawn again.
    fn timed_part(&self, n: usize, timed: &Arrangement) -> Option<Pixmap> {
        self.print_part(
            self.timed_rects[n],
            &self.dial_surfaces.as_ref()?[n],
            &art::dials_print(&self.layout, &self.arrangement),
            &art::dials_print(&self.layout, timed),
        )
    }

    /// Oscillator `n`'s WAVEFORM switch's place printed with its arc (decisions.md
    /// R-CONTINUOUS-WAVE), as [`Renderer::timed_part`] prints a timed dial's.
    fn arc_part(&self, n: usize) -> Option<Pixmap> {
        self.print_part(
            self.arc_rects[n],
            &self.arc_surfaces.as_ref()?[n],
            &art::waveform_print(&self.layout, &self.arrangement, n, false),
            &art::waveform_print(&self.layout, &self.arrangement, n, true),
        )
    }

    /// The background's place `r` printed with `printed` instead of `plain` (each the place's
    /// print alone, as the panel's SVG): `surface`, the place under the print, the ink printed
    /// on it as the whole background's is (worn into the face, the lamp over it), and the
    /// background's own pixels wherever the two prints' ink is the same, so that it lies
    /// without a seam.
    fn print_part(
        &self,
        r: [i32; 4],
        surface: &Pixmap,
        plain: &str,
        printed: &str,
    ) -> Option<Pixmap> {
        let mut part = surface.clone();
        let (w, h) = (part.width(), part.height());
        let s = self.scale;
        let view = [
            f64::from(r[0]) / s,
            self.top() + f64::from(r[1]) / s,
            f64::from(w) / s,
            f64::from(h) / s,
        ];
        let ink = |body: &str| {
            let mut p = Pixmap::new(w, h)?;
            let doc = art::document(view, w, h, body);
            let tree = usvg::Tree::from_str(&doc, &self.options).ok()?;
            resvg::render(&tree, Transform::identity(), &mut p.as_mut());
            Some(p)
        };
        let (plain, printed) = (ink(plain)?, ink(printed)?);
        let mut worn = printed.clone();
        let opacity = if self.skin == Skin::Worn {
            worn_into_by(&mut worn, &part, self.face_mean.unwrap_or(1.0));
            0.94
        } else {
            1.0
        };
        part.draw_pixmap(
            0,
            0,
            worn.as_ref(),
            &PixmapPaint {
                opacity,
                ..PixmapPaint::default()
            },
            Transform::identity(),
            None,
        );
        if self.skin == Skin::Worn {
            lamp_in(
                &mut part,
                s,
                (r[0], r[1]),
                self.arrangement.panel.drawing_h(),
            );
        }
        // (Where the two dials' ink is the same, the background's own pixels.)
        let fw = self.background.width() as usize;
        let bg = self.background.data();
        let (a, b, out) = (plain.data(), printed.data(), part.data_mut());
        for y in 0..h as usize {
            for x in 0..w as usize {
                let i = (y * w as usize + x) * 4;
                if a[i..i + 4] == b[i..i + 4] {
                    let j = ((y + r[1] as usize) * fw + x + r[0] as usize) * 4;
                    out[i..i + 4].copy_from_slice(&bg[j..j + 4]);
                }
            }
        }
        Some(part)
    }

    /// The layers showing `scene`, in the order they are drawn.
    fn layers(&self, scene: &Scene) -> Vec<Layer> {
        let silent = feedback_silent(&scene.values);
        let worn = self.skin == Skin::Worn;
        let control = if worn {
            art::control_worn
        } else {
            art::control
        };
        let a = &self.arrangement;
        let mut out: Vec<Layer> = a
            .controls
            .iter()
            .zip(scene.values)
            .map(|(c, v)| {
                // (The drawers and the LFO page draw their own: `Renderer::paint_control`.)
                if c.place.drawn_apart() {
                    return Layer::default();
                }
                let midi = match c.kind {
                    Kind::Wheel { detent: true, .. } => scene.midi.0,
                    Kind::Wheel { .. } => scene.midi.1,
                    _ => 0.0,
                };
                // An oscillator's MOD key, on the panel while its waveform is CONTINUOUS, in its
                // way's colour (R-CONTINUOUS-WAVE).
                if let Some(n) = MOD_KEYS.iter().position(|k| *k == c.param) {
                    return art::key(
                        c,
                        art::Keyed {
                            on: v >= 0.5,
                            word: Some("MOD"),
                            hidden: !scene.shape[n].continuous,
                            way: scene.shape[n].way,
                            ..art::Keyed::default()
                        },
                        self.layout.lettering(),
                        self.layout.times(),
                        worn,
                    );
                }
                // An oscillator's TIMED key, its sign its division's (R-OSC-TIMED).
                if c.kind == Kind::Key {
                    let n = usize::from(c.param == TIMED_KEYS[1]);
                    let t = scene.timed[n];
                    return art::key(
                        c,
                        art::Keyed {
                            on: v >= 0.5,
                            inert: t.inert,
                            division: t.division,
                            hidden: t.inert,
                            ..art::Keyed::default()
                        },
                        self.layout.lettering(),
                        self.layout.times(),
                        worn,
                    );
                }
                // A WAVEFORM switch CONTINUOUS: turned where its shape is (R-CONTINUOUS-WAVE).
                if let Some(n) = WAVEFORMS.iter().position(|w| *w == c.param)
                    && scene.shape[n].continuous
                {
                    return art::selector_turned(c, art::shape_angle(scene.shape[n].at), worn);
                }
                let mut layer = control(c, v, midi);
                // A pack's rocker drawn in its two states: its picture on or off, neither
                // mirrored, no paddle lit or shadowed by the plug-in.
                if worn
                    && let Some(sp) = layer.sprites.first_mut()
                    && let Some(slot) = crate::skin::on_slot(sp.part)
                    && self.look.pictures.contains_key(slot)
                {
                    if v >= 0.5 {
                        sp.part = match sp.part {
                            Part::RockerBlue => Part::RockerBlueOn,
                            Part::RockerOrange => Part::RockerOrangeOn,
                            _ => Part::RockerIvoryOn,
                        };
                    }
                    sp.flip = (false, false);
                    layer.paddle = None;
                    layer.body = String::new();
                }
                // FEEDBACK while EXTERNAL INPUT is closed: dimmed as the parts with no
                // parameter are, still operable; DETUNE in SCATTER, VOICES where it does
                // nothing and OSC. 3 CONTROL while oscillator 3 is timed: dimmed so, and not
                // operable (the editor's `operable`).
                if (silent && c.param == "feedback")
                    || (scene.detune_off && c.param == "double")
                    || (scene.voices_off && c.param == "voices")
                    || (scene.osc3_control_off && c.param == "osc3_control")
                {
                    if worn {
                        // (A picture is not dimmed: the knob is shaded, a rocker over the
                        // whole of its picture.)
                        let n = crate::svg::N;
                        layer.over.push_str(&match c.kind {
                            Kind::Rocker { w, h, .. } => format!(
                                "<rect x='{}' y='{}' width='{}' height='{}' rx='4' fill='#000' fill-opacity='0.5'/>",
                                n(-w / 2.0 - 2.0),
                                n(-h / 2.0 - 2.0),
                                n(w + 4.0),
                                n(h + 4.0)
                            ),
                            _ => format!(
                                "<circle r='{}' fill='#000' fill-opacity='0.5'/>",
                                n(art::STD.skirt + 1.0)
                            ),
                        });
                    } else {
                        layer.body = format!("<g opacity='0.5'>{}</g>", layer.body);
                    }
                }
                layer
            })
            .collect();
        // MOD's orbs, where each CONTINUOUS shape is now (decisions.md R-CONTINUOUS-WAVE): over
        // the switches, one a layer, there or not.
        for (n, wave) in WAVEFORMS.iter().enumerate() {
            let shape = scene.shape[n];
            out.push(
                match (
                    shape.orb.filter(|_| shape.continuous),
                    a.controls.iter().find(|c| c.param == *wave),
                ) {
                    (Some(at), Some(c)) => art::shape_orb(c, at, shape.way),
                    _ => Layer::default(),
                },
            );
        }
        // The fixtures, each drawn about the official layout's place and moved to this one's.
        let at = |(x, y): (f64, f64)| (art::COL + x, art::TOP + y);
        let moved = |mut l: Layer, to: (f64, f64)| {
            l.origin = to;
            l
        };
        if worn {
            out.push(moved(art::power_worn(scene.power), at(a.power)));
            out.push(moved(art::lamp_worn(scene.power), at(a.lamp)));
            out.push(moved(art::overload_worn(scene.overload), at(a.overload)));
            out.extend(crate::ultra::ultra_worn(&scene.opening).map(|l| moved(l, a.ultra_at())));
        } else {
            out.push(moved(art::power(scene.power), at(a.power)));
            out.push(moved(art::lamp(scene.power), at(a.lamp)));
            out.push(moved(art::overload(scene.overload), at(a.overload)));
            out.push(moved(crate::ultra::ultra(&scene.opening), a.ultra_at()));
            // (The drawn skin's opening is one layer: an empty second keeps the count.)
            out.push(Layer::default());
        }
        out.push(moved(
            if worn {
                art::plate_worn()
            } else {
                art::plate()
            },
            a.plate_at(),
        ));
        // (Always a layer, none in a plug-in, so that the count stays.)
        out.push(
            scene
                .tempo
                .as_deref()
                .map(|t| art::tempo_screen(t, a))
                .unwrap_or_default(),
        );
        // Its keys after it, in the rail's keys' print (a pack's where it has one); three layers
        // always, as the screen's.
        let print = self.look.colours.key_print.map_or_else(
            || crate::strip::KEY_PRINT.to_owned(),
            |(r, g, b)| format!("#{r:02x}{g:02x}{b:02x}"),
        );
        for k in art::TempoKey::ALL {
            out.push(
                scene
                    .tempo_keys
                    .map(|t| {
                        let lit = match k {
                            art::TempoKey::Typing => t.typing,
                            art::TempoKey::Panic => t.panic,
                            art::TempoKey::Midi => t.midi,
                        };
                        art::tempo_key(a, k, lit, t.hover == Some(k), &print)
                    })
                    .unwrap_or_default(),
            );
        }
        // The left hand's VELOCITY: its meter (decisions.md R-VELOCITY),
        // in a pack's `velocity` colour where it has one.
        out.push(crate::strip::velocity::meter(
            scene.velocity_lit,
            crate::strip::velocity::colour(&self.look.colours),
        ));
        // The MIDI map mode's marks (R-MIDI-MAP), floating, under the ring, a tip, a note, a menu.
        let small = self.text_size() * learn::MARK_TEXT;
        out.extend(
            scene
                .marks
                .iter()
                .map(|m| learn::map_mark(&self.fonts, m, small)),
        );
        if let Some(c) = scene.learning.and_then(|i| a.controls.get(i)) {
            out.push(learn::ring(c));
        }
        if let Some((t, x, y)) = &scene.tip {
            out.push(art::tip(&self.fonts, *x, *y, t, self.text_size()));
        }
        if let Some(n) = &scene.note {
            out.push(learn::note(&self.fonts, n));
        }
        if let Some(m) = &scene.menu {
            out.push(learn::menu(&self.fonts, m));
        }
        if let Some(entry) = &scene.entry {
            out.push(entry.layer());
        }
        out
    }

    /// The lettering's size of the tip, a note and a menu, in drawing units: its logical pixels at
    /// this scale, but never smaller than the panel's legends, which grow with the window where
    /// these would not (a 4K screen at 100 %: decisions.md R-READABILITY-MINIMUM).
    pub fn text_size(&self) -> f64 {
        let lettering = self.layout.lettering();
        (TIP_TEXT * lettering.text() * self.ui / self.scale).max(lettering.legend())
    }

    /// Control `i` at `v` (normalized) as this look draws it, alone, for a frame drawn apart (the
    /// arpeggiator's drawer, `crate::strip::arp`): its pixels and where their top left corner
    /// is in the drawing's pixels (not this frame's, which a taller pack's panel begins above
    /// the drawing's top).
    pub fn paint_control(&mut self, i: usize, v: f64) -> Option<(i32, i32, Pixmap)> {
        let c = *self.arrangement.controls.get(i)?;
        let layer = if self.skin == Skin::Worn {
            art::control_worn(&c, v, 0.0)
        } else {
            art::control(&c, v, 0.0)
        };
        let top = self.top_px();
        self.render_layer(&layer).map(|(x, y, p)| (x, y + top, p))
    }

    /// Any layer as this look draws it, alone, for a frame drawn apart (the LFO page's keys,
    /// switches and jack: decisions.md R-LFO), as [`Renderer::paint_control`] paints a
    /// control.
    pub(crate) fn paint_layer(&mut self, layer: &Layer) -> Option<(i32, i32, Pixmap)> {
        let top = self.top_px();
        self.render_layer(layer).map(|(x, y, p)| (x, y + top, p))
    }

    /// A layer's pixels, aligned to the frame's: their place and their image (its body, its
    /// pictures turned in place, then what goes over them).
    fn render_layer(&mut self, layer: &Layer) -> Option<(i32, i32, Pixmap)> {
        let (ox, oy) = layer.origin;
        let b = layer.bounds;
        let s = self.scale;
        let (x0, y0) = (((ox + b[0]) * s).floor(), ((oy + b[1]) * s).floor());
        let (x1, y1) = (((ox + b[2]) * s).ceil(), ((oy + b[3]) * s).ceil());
        let (w, h) = ((x1 - x0).max(1.0) as u32, (y1 - y0).max(1.0) as u32);
        let view = [x0 / s, y0 / s, f64::from(w) / s, f64::from(h) / s];
        let body = format!(
            "<g transform='translate({} {})'>{}</g>",
            crate::svg::N(ox),
            crate::svg::N(oy),
            layer.body
        );
        let mut pixmap = Pixmap::new(w, h)?;
        if !layer.body.is_empty() {
            let doc = art::document(view, w, h, &body);
            let tree = usvg::Tree::from_str(&doc, &self.options).ok()?;
            resvg::render(&tree, Transform::identity(), &mut pixmap.as_mut());
        }
        // A screen: its bezel and glass made once in the layer's place and kept, its dots drawn.
        if let Some(screen) = &layer.screen {
            let (wx, wy, ww, wh) = screen.window;
            let window = (ox + wx - x0 / s, oy + wy - y0 / s, ww, wh);
            let place = [x0 as i32, y0 as i32, w as i32, h as i32];
            if self.screens.as_ref().is_none_or(|(at, _)| *at != place) {
                let glass = Pixmap::decode_png(crate::skin::GLASS).expect("the glass decodes");
                let mut still = Pixmap::new(w, h)?;
                crate::strip::screen_still(&self.options, &mut still, &glass, s, window);
                self.screens = Some((place, still));
            }
            if let Some((_, still)) = &self.screens {
                put(&mut pixmap, still, (0, 0), 1.0);
            }
            // (The built-in glass and lamp: a skin pack does not dress the TEMPO screen.)
            crate::strip::screen_text(
                &mut pixmap,
                s,
                window,
                &screen.text,
                &crate::strip::Lamp::ORANGE,
            );
        }
        // Each picture placed (resampled once; kept if it is placed so again), then put with
        // its opacity: drawn so whether it was kept or not.
        self.renders += 1;
        for sp in &layer.sprites {
            let key = PlacedKey {
                part: sp.part,
                size: (
                    (sp.size.0 * s).round().max(1.0) as u32,
                    (sp.size.1 * s).round().max(1.0) as u32,
                ),
                at: (
                    ((ox + sp.at.0) * s - x0).to_bits(),
                    ((oy + sp.at.1) * s - y0).to_bits(),
                ),
                deg: (sp.deg as f32).to_bits(),
                flip: sp.flip,
                zoom: (sp.zoom as f32).to_bits(),
            };
            let Some(i) = self.placed_picture(key) else {
                continue;
            };
            let p = &self.placed[i];
            put(&mut pixmap, &p.pixels, p.at, sp.alpha as f32);
        }
        // Each light from its picture, made once in the layer's place, so drawn without moving
        // a pixel; scaled about the layer's origin while its lamp comes up.
        for light in &layer.lights {
            let place = [x0 as i32, y0 as i32, w as i32, h as i32];
            let made = self
                .lights
                .iter()
                .position(|(g, at, _)| (*g, *at) == (light.glow, place));
            let i = match made {
                Some(i) => i,
                None => {
                    let svg = format!(
                        "<g transform='translate({} {})'>{}</g>",
                        crate::svg::N(ox),
                        crate::svg::N(oy),
                        light.glow.svg()
                    );
                    let doc = art::document(view, w, h, &svg);
                    let tree = usvg::Tree::from_str(&doc, &self.options).ok()?;
                    let mut p = Pixmap::new(w, h)?;
                    resvg::render(&tree, Transform::identity(), &mut p.as_mut());
                    self.lights.push((light.glow, place, p));
                    self.lights.len() - 1
                }
            };
            let (cx, cy) = ((ox * s - x0) as f32, (oy * s - y0) as f32);
            let zoom = light.zoom as f32;
            if zoom == 1.0 {
                put(&mut pixmap, &self.lights[i].2, (0, 0), light.alpha as f32);
                continue;
            }
            pixmap.draw_pixmap(
                0,
                0,
                self.lights[i].2.as_ref(),
                &PixmapPaint {
                    quality: FilterQuality::Bicubic,
                    opacity: (light.alpha as f32).clamp(0.0, 1.0),
                    ..PixmapPaint::default()
                },
                Transform::from_translate(cx, cy)
                    .pre_scale(zoom, zoom)
                    .pre_translate(-cx, -cy),
                None,
            );
        }
        // Each overlay from its picture, drawn once in the layer's place, with its blend (a
        // knob's lamp and cap lights not, where the look's caps turn: its picture is lit).
        let skip = layer.round && self.look.caps_turn;
        for o in layer.overlays.iter().filter(|_| !skip) {
            let place = [x0 as i32, y0 as i32, w as i32, h as i32];
            let made = self
                .overlays
                .iter()
                .position(|(svg, at, _)| *at == place && *svg == o.svg);
            let i = match made {
                Some(i) => i,
                None => {
                    let svg = format!(
                        "<g transform='translate({} {})'>{}</g>",
                        crate::svg::N(ox),
                        crate::svg::N(oy),
                        o.svg
                    );
                    let doc = art::document(view, w, h, &svg);
                    let tree = usvg::Tree::from_str(&doc, &self.options).ok()?;
                    let mut p = Pixmap::new(w, h)?;
                    resvg::render(&tree, Transform::identity(), &mut p.as_mut());
                    self.overlays.push((o.svg.clone(), place, p));
                    self.overlays.len() - 1
                }
            };
            let blend_mode = match o.blend {
                Blend::Normal => {
                    put(&mut pixmap, &self.overlays[i].2, (0, 0), 1.0);
                    continue;
                }
                Blend::ColorDodge => BlendMode::ColorDodge,
                Blend::Multiply => BlendMode::Multiply,
            };
            pixmap.draw_pixmap(
                0,
                0,
                self.overlays[i].2.as_ref(),
                &PixmapPaint {
                    blend_mode,
                    quality: FilterQuality::Nearest,
                    ..PixmapPaint::default()
                },
                Transform::identity(),
                None,
            );
        }
        let origin = ((ox * s - x0) as f32, (oy * s - y0) as f32);
        for d in &layer.drawn {
            d.draw(&mut pixmap, origin, s as f32);
        }
        if let Some(wheel) = layer.wheel {
            // (A look's colour as written, sRGB: the wheel is shaded in linear light.)
            let albedo = self
                .look
                .colours
                .wheel
                .map(|(r, g, b)| [r, g, b].map(|v| (f64::from(v) / 255.0).powf(2.2)));
            draw_wheel(&mut pixmap, (ox * s - x0, oy * s - y0), s, wheel, albedo);
        }
        if let Some(paddle) = layer.paddle {
            light_paddle(&mut pixmap, (ox * s - x0, oy * s - y0), s, paddle);
        }
        if !layer.over.is_empty() {
            let over = format!(
                "<g transform='translate({} {})'>{}</g>",
                crate::svg::N(ox),
                crate::svg::N(oy),
                layer.over
            );
            let doc = art::document(view, w, h, &over);
            let tree = usvg::Tree::from_str(&doc, &self.options).ok()?;
            resvg::render(&tree, Transform::identity(), &mut pixmap.as_mut());
        }
        Some((x0 as i32, y0 as i32 - self.top_px(), pixmap))
    }

    /// The picture placed as `key` says, kept or placed now (and kept): its index in
    /// [`Renderer::placed`].
    fn placed_picture(&mut self, key: PlacedKey) -> Option<usize> {
        if let Some(i) = self.placed.iter().position(|p| p.key == key) {
            self.placed[i].used = self.renders;
            return Some(i);
        }
        let (overrides, tint, turn) = (
            &self.look.pictures,
            self.look.face_tint,
            self.look.caps_turn,
        );
        let pictures = self
            .pictures
            .get_or_insert_with(|| Pictures::load_with(overrides, tint, turn));
        let picture = pictures.part(key.part, key.size.0, key.size.1);
        let at = (f64::from_bits(key.at.0), f64::from_bits(key.at.1));
        let (x, y, pixels) = place_picture(
            picture,
            at,
            f32::from_bits(key.deg),
            key.flip,
            f32::from_bits(key.zoom),
        )?;
        let bytes = |p: &Placed| p.pixels.data().len();
        let mut kept: usize = self.placed.iter().map(bytes).sum();
        while kept + pixels.data().len() > PLACED_BYTES && !self.placed.is_empty() {
            let oldest = (0..self.placed.len())
                .min_by_key(|&i| self.placed[i].used)
                .expect("one at least");
            kept -= bytes(&self.placed[oldest]);
            self.placed.swap_remove(oldest);
        }
        self.placed.push(Placed {
            key,
            at: (x, y),
            pixels,
            used: self.renders,
        });
        Some(self.placed.len() - 1)
    }

    /// Brings the frame up to `scene`, rendering only the layers whose art changed: whether
    /// the frame changed.
    pub fn render(&mut self, scene: &Scene) -> bool {
        self.damage = None;
        self.damages.clear();
        if self.shown.as_ref() == Some(scene) {
            return false;
        }
        let first = self.shown.is_none();
        let dials_were = self.shown.as_ref().map(Scene::timed_dials);
        let arcs_were = self.shown.as_ref().map(Scene::arcs);
        let glide_was = self.shown.as_ref().map(|s| s.glide_long);
        self.shown = Some(scene.clone());
        let dials = scene.timed_dials();
        if dials.contains(&true) && self.timed_background.is_none() {
            let a = self.timed_arrangement();
            self.timed_background = match (self.timed_part(0, &a), self.timed_part(1, &a)) {
                (Some(two), Some(three)) => Some([two, three]),
                _ => None,
            };
        }
        let arcs = scene.arcs();
        if arcs.contains(&true) && self.arc_background.is_none() {
            self.arc_background = match (self.arc_part(0), self.arc_part(1), self.arc_part(2)) {
                (Some(a), Some(b), Some(c)) => Some([a, b, c]),
                _ => None,
            };
        }
        if scene.glide_long && self.glide_background.is_none() {
            self.glide_background = self.glide_surface.as_ref().and_then(|surface| {
                self.print_part(
                    self.glide_rect,
                    surface,
                    "",
                    &art::glide_long_print(&self.layout, &self.arrangement),
                )
            });
        }
        let layers = self.layers(scene);
        let mut changed = self.slots.len() != layers.len();
        // Where the layers changed, as they were and as they are; and a dial's print that
        // changed.
        let mut rects: Vec<[i32; 4]> = Vec::new();
        if glide_was.is_some_and(|on| on != scene.glide_long) {
            rects.push(self.glide_rect);
            changed = true;
        }
        for n in 0..2 {
            if dials_were.is_some_and(|d| d[n] != dials[n]) {
                rects.push(self.timed_rects[n]);
                changed = true;
            }
        }
        for n in 0..3 {
            if arcs_were.is_some_and(|a| a[n] != arcs[n]) {
                rects.push(self.arc_rects[n]);
                changed = true;
            }
        }
        let mut add = |pixels: &Option<(i32, i32, Pixmap)>| {
            if let Some((x, y, p)) = pixels {
                rects.push([*x, *y, x + p.width() as i32, y + p.height() as i32]);
            }
        };
        for slot in self.slots.iter().skip(layers.len()) {
            add(&slot.pixels);
        }
        self.slots.resize_with(layers.len(), Slot::default);
        for (i, layer) in layers.into_iter().enumerate() {
            if self.slots[i].layer.as_ref() == Some(&layer) {
                continue;
            }
            add(&self.slots[i].pixels);
            self.slots[i].pixels = self.render_layer(&layer);
            add(&self.slots[i].pixels);
            self.slots[i].layer = Some(layer);
            changed = true;
        }
        self.damages = if first {
            vec![[0, 0, self.frame.width() as i32, self.frame.height() as i32]]
        } else {
            merged(rects)
        };
        self.damage = self.damages.iter().copied().reduce(union);
        if changed {
            for d in self.damages.clone() {
                self.recompose(d);
            }
        }
        changed
    }

    /// The frame put together again inside `rect` (pixels: left, top, right, bottom), from the
    /// background and every layer over it there, in order: each of its pixels as when the whole
    /// frame is put together.
    fn recompose(&mut self, rect: [i32; 4]) {
        let (fw, fh) = (self.frame.width() as i32, self.frame.height() as i32);
        let [x0, y0, x1, y1] = [
            rect[0].clamp(0, fw),
            rect[1].clamp(0, fh),
            rect[2].clamp(0, fw),
            rect[3].clamp(0, fh),
        ];
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let whole = [x0, y0, x1, y1] == [0, 0, fw, fh];
        let mut patch = if whole {
            self.background.clone()
        } else {
            let Some(mut p) = Pixmap::new((x1 - x0) as u32, (y1 - y0) as u32) else {
                return;
            };
            let n = (x1 - x0) as usize * 4;
            for y in y0..y1 {
                let from = (y as usize * fw as usize + x0 as usize) * 4;
                let to = (y - y0) as usize * n;
                p.data_mut()[to..to + n].copy_from_slice(&self.background.data()[from..from + n]);
            }
            p
        };
        // A FREQUENCY dial timed: its print from the background drawn so (R-OSC-TIMED); a
        // WAVEFORM switch CONTINUOUS: its place printed with its arc (R-CONTINUOUS-WAVE).
        if let Some(scene) = &self.shown {
            let dials = scene.timed_dials();
            let arcs = scene.arcs();
            let timed = self.timed_background.iter().flat_map(|t| {
                t.iter()
                    .zip(self.timed_rects)
                    .zip(dials)
                    .filter(|(_, on)| *on)
                    .map(|(part, _)| part)
            });
            let arced = self.arc_background.iter().flat_map(|a| {
                a.iter()
                    .zip(self.arc_rects)
                    .zip(arcs)
                    .filter(|(_, on)| *on)
                    .map(|(part, _)| part)
            });
            let glide = self
                .glide_background
                .iter()
                .filter(|_| scene.glide_long)
                .map(|p| (p, self.glide_rect));
            for (part, r) in timed.chain(arced).chain(glide) {
                let [a0, b0, a1, b1] = [r[0].max(x0), r[1].max(y0), r[2].min(x1), r[3].min(y1)];
                if a1 <= a0 || b1 <= b0 {
                    continue;
                }
                let (pw, tw, n4) = (
                    patch.width() as usize,
                    part.width() as usize,
                    (a1 - a0) as usize * 4,
                );
                for y in b0..b1 {
                    let from = ((y - r[1]) as usize * tw + (a0 - r[0]) as usize) * 4;
                    let to = ((y - y0) as usize * pw + (a0 - x0) as usize) * 4;
                    patch.data_mut()[to..to + n4].copy_from_slice(&part.data()[from..from + n4]);
                }
            }
        }
        for slot in &self.slots {
            if let Some((x, y, p)) = &slot.pixels {
                let (r, b) = (x + p.width() as i32, y + p.height() as i32);
                if *x < x1 && r > x0 && *y < y1 && b > y0 {
                    put(&mut patch, p, (x - x0, y - y0), 1.0);
                }
            }
        }
        if whole {
            self.frame = patch;
            return;
        }
        let n = (x1 - x0) as usize * 4;
        for y in y0..y1 {
            let to = (y as usize * fw as usize + x0 as usize) * 4;
            let from = (y - y0) as usize * n;
            self.frame.data_mut()[to..to + n].copy_from_slice(&patch.data()[from..from + n]);
        }
    }

    /// The pixels the last render changed (left, top, right, bottom; the last two past the
    /// change), if any: where the strip's parts are drawn again over the frame
    /// (`crate::strip`) and the editor's frame is put together again.
    pub fn damage(&self) -> Option<[i32; 4]> {
        self.damage
    }

    /// The same, as the separate places it is in, each put together again on its own.
    pub fn damages(&self) -> &[[i32; 4]] {
        &self.damages
    }

    /// The layers that float over everything (MIDI Learn's ring, the tip, the note, a menu)
    /// drawn again over `onto`, a frame the drawing's size at this scale: the editor's, over the
    /// strip's own parts (`crate::strip`), which are drawn over this frame's strip.
    pub fn draw_floating(&self, onto: &mut Pixmap) {
        // (The controls', then POWER's, its lamp's, OVERLOAD's, QUALITY's opening's two, the
        // name plate's, the TEMPO screen's and its three keys', the VELOCITY meter's: fixed.)
        let fixed = CONTROLS.len() + FIXED;
        for slot in self.slots.iter().skip(fixed) {
            if let Some((x, y, p)) = &slot.pixels {
                onto.draw_pixmap(
                    *x,
                    *y,
                    p.as_ref(),
                    &PixmapPaint::default(),
                    Transform::identity(),
                    None,
                );
            }
        }
    }

    /// Whether any layer floats now ([`Renderer::draw_floating`]).
    pub fn floating(&self) -> bool {
        self.slots.len() > CONTROLS.len() + FIXED
    }
}

/// The smallest rectangle round two (left, top, right, bottom).
fn union(a: [i32; 4], b: [i32; 4]) -> [i32; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

/// How near two places changed may be, pixels, to be put together again as one.
const NEAR: i32 = 8;

/// The most places put together again apart; beyond them, one round all.
const MOST_PLACES: usize = 8;

/// Places changed, those that overlap or come within [`NEAR`] of each other merged (again until
/// none do), at most [`MOST_PLACES`] of them.
fn merged(mut rects: Vec<[i32; 4]>) -> Vec<[i32; 4]> {
    let near = |a: [i32; 4], b: [i32; 4]| {
        a[0] - NEAR < b[2] && b[0] - NEAR < a[2] && a[1] - NEAR < b[3] && b[1] - NEAR < a[3]
    };
    'again: loop {
        for i in 0..rects.len() {
            for j in i + 1..rects.len() {
                if near(rects[i], rects[j]) {
                    rects[i] = union(rects[i], rects[j]);
                    rects.swap_remove(j);
                    continue 'again;
                }
            }
        }
        break;
    }
    if rects.len() > MOST_PLACES {
        rects = rects.into_iter().reduce(union).into_iter().collect();
    }
    rects
}

/// The layers drawn after the controls whatever the scene: MOD's three orbs (each empty while
/// it does not show), POWER, its lamp, OVERLOAD, QUALITY's opening (its back and its front), the
/// name plate, the TEMPO screen and its TYPING, PANIC and MIDI keys (all four empty in a
/// plug-in), the left hand's VELOCITY meter (decisions.md R-VELOCITY).
const FIXED: usize = 14;

/// The most bands [`in_bands`] draws side by side.
const BANDS: usize = 8;

#[cfg(test)]
thread_local! {
    /// The bands a test has [`in_bands`] draw, instead of a band a processor.
    static TEST_BANDS: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// `draw` over `frame` in bands of its rows, side by side on threads of their own (a band a
/// processor, at most [`BANDS`], none under 64 rows): each band a copy of its rows, copied back
/// once drawn. `draw` is given the band and the frame's row at the band's top, and draws only
/// what falls on it, each pixel as over the whole frame.
fn in_bands(frame: &mut Pixmap, draw: impl Fn(&mut Pixmap, i32) + Sync) {
    let (w, h) = (frame.width(), frame.height());
    let n = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    #[cfg(test)]
    let n = TEST_BANDS.with(std::cell::Cell::get).unwrap_or(n);
    let n = n.clamp(1, BANDS).min((h / 64).max(1) as usize) as u32;
    if n == 1 {
        draw(frame, 0);
        return;
    }
    let rows = h.div_ceil(n);
    let stride = w as usize * 4;
    let draw = &draw;
    std::thread::scope(|scope| {
        for (i, band) in frame
            .data_mut()
            .chunks_mut(rows as usize * stride)
            .enumerate()
        {
            scope.spawn(move || {
                let size = IntSize::from_wh(w, (band.len() / stride) as u32)
                    .expect("a band of at least a row");
                let mut pixmap = Pixmap::from_vec(band.to_vec(), size).expect("the band's rows");
                draw(&mut pixmap, (i as u32 * rows) as i32);
                band.copy_from_slice(pixmap.data());
            });
        }
    });
}

/// `tree` (an SVG drawing over the whole of `frame`) drawn over it, its parts drawn side by side
/// on threads of their own (a part a time each, as many threads as [`in_bands`] has bands): each
/// part over a pixmap of its own, its box's, and these laid over the frame in the drawing's order.
/// So resvg draws a part with a filter (a shadow's blur) over the canvas, from the pixmap it
/// filtered it in; a part without, each pixel within a level of light of it drawn directly.
fn render_parts(tree: &usvg::Tree, frame: &mut Pixmap) {
    let mut parts = Vec::new();
    parts_of(tree.root(), Transform::identity(), &mut parts);
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    #[cfg(test)]
    let threads = TEST_BANDS.with(std::cell::Cell::get).unwrap_or(threads);
    let threads = threads.clamp(1, BANDS).min(parts.len().max(1));
    let next = std::sync::atomic::AtomicUsize::new(0);
    let drawn: Vec<std::sync::Mutex<Option<(i32, i32, Pixmap)>>> =
        parts.iter().map(|_| std::sync::Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(&(part, ts)) = parts.get(i) else {
                        break;
                    };
                    let Some(b) = part.abs_layer_bounding_box() else {
                        continue;
                    };
                    // (Whole pixels, two more either side for a smoothed edge.)
                    let (x0, y0) = (b.x().floor() as i32 - 2, b.y().floor() as i32 - 2);
                    let (x1, y1) = (b.right().ceil() as i32 + 2, b.bottom().ceil() as i32 + 2);
                    let Some(mut layer) = Pixmap::new((x1 - x0) as u32, (y1 - y0) as u32) else {
                        continue;
                    };
                    // (resvg draws a part at its box's corner: put back, at the layer's.)
                    let at = Transform::from_translate(-x0 as f32, -y0 as f32)
                        .pre_concat(ts)
                        .pre_translate(b.x(), b.y());
                    resvg::render_node(part, at, &mut layer.as_mut());
                    *drawn[i].lock().expect("a part's layer") = Some((x0, y0, layer));
                }
            });
        }
    });
    for layer in drawn {
        if let Some((x, y, layer)) = layer.into_inner().expect("a part's layer") {
            frame.draw_pixmap(
                x,
                y,
                layer.as_ref(),
                &PixmapPaint::default(),
                Transform::identity(),
                None,
            );
        }
    }
}

/// The parts of `group` drawn each on its own (with what it is drawn through, `ts`): its
/// children, and those of the groups in it that only move theirs.
fn parts_of<'a>(group: &'a usvg::Group, ts: Transform, out: &mut Vec<(&'a usvg::Node, Transform)>) {
    for node in group.children() {
        match node {
            usvg::Node::Group(g) if !g.should_isolate() => {
                parts_of(g, ts.pre_concat(g.transform()), out);
            }
            _ => out.push((node, ts)),
        }
    }
}

/// The lamp's light over `frame`, a part of the official panel's drawing whose top left pixel is
/// the drawing's pixel `at` (the drawers dropped from the rail: `crate::strip::lower`).
pub(crate) fn lamp_at(frame: &mut Pixmap, s: f64, at: (i32, i32)) {
    lamp_in(frame, s, at, art::H);
}

/// The panel's lamp, up and to the left, over the whole drawing at `s` pixels a unit: brighter
/// near it (the face's sheen, catching the specks of its texture), falling off towards the far
/// corner. Each pixel's light multiplied (`LAMP`: at the lamp, far from it, and how far its
/// light reaches across and down, as shares of the drawing).
///
/// Here over `frame`, its top left pixel `at` from the top left of a drawing `height` tall (a
/// skin pack's panel may be taller than the official one).
pub(crate) fn lamp_in(frame: &mut Pixmap, s: f64, at: (i32, i32), height: f64) {
    const LAMP: (f64, f64, f64, f64) = (1.26, 0.84, 0.55, 0.9);
    let (w, h) = (frame.width() as usize, frame.height() as usize);
    let across: Vec<f64> = (0..w)
        .map(|x| {
            let u = (x as f64 + f64::from(at.0) + 0.5) / s / art::W;
            (-((u - 0.2) / LAMP.2).powi(2)).exp()
        })
        .collect();
    let down: Vec<f64> = (0..h)
        .map(|y| {
            let v = (y as f64 + f64::from(at.1) + 0.5) / s / height;
            (-(v / LAMP.3).powi(2)).exp()
        })
        .collect();
    let data = frame.data_mut();
    for (y, d) in down.iter().enumerate() {
        for (x, a) in across.iter().enumerate() {
            let k = LAMP.1 + (LAMP.0 - LAMP.1) * a * d;
            let i = 4 * (y * w + x);
            for c in &mut data[i..i + 3] {
                *c = (f64::from(*c) * k).round().min(255.0) as u8;
            }
        }
    }
}

/// The print as worn into the face under it: each pixel of ink a little darker where the face's
/// texture is darker than its mean, so the texture shows through it, and faded a shade.
pub(crate) fn worn_into(ink: &mut Pixmap, face: &Pixmap) {
    worn_into_by(ink, face, face_mean(face));
}

/// The pixels of `p` inside `r` (left, top, right, bottom), a pixmap of their own.
fn crop(p: &Pixmap, r: [i32; 4]) -> Pixmap {
    let (w, h) = ((r[2] - r[0]).max(1) as usize, (r[3] - r[1]).max(1) as usize);
    let mut out = Pixmap::new(w as u32, h as u32).expect("a part of at least a pixel");
    let pw = p.width() as usize;
    for y in 0..h {
        let from = ((y + r[1] as usize) * pw + r[0] as usize) * 4;
        if from + w * 4 <= p.data().len() {
            out.data_mut()[y * w * 4..(y + 1) * w * 4]
                .copy_from_slice(&p.data()[from..from + w * 4]);
        }
    }
    out
}

/// A pixel's level (luma) as the print is worn by it.
fn level(r: u8, g: u8, b: u8) -> f64 {
    0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b)
}

/// The face's mean level, as [`worn_into`] measures it.
pub(crate) fn face_mean(face: &Pixmap) -> f64 {
    let (sum, n) = face
        .pixels()
        .iter()
        .step_by(97)
        .fold((0.0, 0.0), |(s, n), p| {
            (s + level(p.red(), p.green(), p.blue()), n + 1.0)
        });
    (sum / n).max(1.0)
}

/// The same, the face's mean given: a part of a frame's print worn as the whole frame's is
/// (`Renderer::timed_part`).
pub(crate) fn worn_into_by(ink: &mut Pixmap, face: &Pixmap, mean: f64) {
    for (p, f) in ink.pixels_mut().iter_mut().zip(face.pixels()) {
        if p.alpha() == 0 {
            continue;
        }
        let ratio = level(f.red(), f.green(), f.blue()) / mean;
        let k = (0.9 + 0.22 * (ratio - 1.0)).clamp(0.74, 1.0);
        let at = |v: u8| (f64::from(v) * k).round() as u8;
        if let Some(c) = resvg::tiny_skia::PremultipliedColorU8::from_rgba(
            at(p.red()),
            at(p.green()),
            at(p.blue()),
            p.alpha(),
        ) {
            *p = c;
        }
    }
}

/// The panel's lamp (above, and up and to the left of, the panel) as a unit vector, x right, y
/// down the panel, z out of it; and the half way between it and the eye.
fn lamp_vectors() -> ((f64, f64, f64), (f64, f64, f64)) {
    let unit = |(x, y, z): (f64, f64, f64)| {
        let n = (x * x + y * y + z * z).sqrt();
        (x / n, y / n, z / n)
    };
    let light = unit((-0.45, -0.55, 0.7));
    (light, unit((light.0, light.1, light.2 + 1.0)))
}

/// A rocker's paddle lit by its shape, `at` its middle (pixels) at `s` pixels a unit: the
/// pressed half low and flat; the raised half rising from the pivot in a smooth hump that rounds
/// over at its end; its long sides rounded. Each pixel of its picture (lit evenly) is made as
/// much lighter or darker as its surface faces the lamp more or less than a flat one does, with
/// a little of the lamp's gloss on it.
fn light_paddle(frame: &mut Pixmap, at: (f64, f64), s: f64, p: art::PaddleArt) {
    let (light, half) = lamp_vectors();
    let flat = 0.3 + 0.7 * light.2;
    let (hl, hw) = (p.pw / 2.0, p.ph / 2.0);
    // The hump's height, and where it crests along the raised half (a share of it).
    let (rise, crest) = (0.32 * p.ph, 0.78);
    let (sin, cos) = p.deg.to_radians().sin_cos();
    let flip = if p.on { -1.0 } else { 1.0 };
    let reach = ((hl + 2.0).hypot(hw + 2.0) * s).ceil();
    let (fw, fh) = (frame.width() as i32, frame.height() as i32);
    let x0 = ((at.0 - reach) as i32).max(0);
    let x1 = ((at.0 + reach) as i32 + 1).min(fw);
    let y0 = ((at.1 - reach) as i32).max(0);
    let y1 = ((at.1 + reach) as i32 + 1).min(fh);
    let data = frame.data_mut();
    for py in y0..y1 {
        for px in x0..x1 {
            let (dx, dy) = (
                (f64::from(px) + 0.5 - at.0) / s,
                (f64::from(py) + 0.5 - at.1) / s,
            );
            // Into the paddle's own frame: along it (raised half positive) and across.
            let u = flip * (dx * cos + dy * sin);
            let v = -dx * sin + dy * cos;
            if u.abs() > hl - 1.0 || v.abs() > hw - 1.0 {
                continue;
            }
            // The slope along it: up the hump to its crest, then rounding over at the end.
            let t = u / hl;
            let du = if t <= 0.0 {
                0.04
            } else if t < crest {
                let x = t / crest;
                rise * 6.0 * x * (1.0 - x) / (crest * hl)
            } else {
                let x = (t - crest) / (1.0 - crest);
                -rise * 0.8 * x * x * 3.0 / ((1.0 - crest) * hl)
            };
            // The long sides rounded over their last few units.
            let e = ((v.abs() - (hw - 5.0)) / 5.0).clamp(0.0, 1.0);
            let dv = v.signum() * e * e * 1.4;
            // The surface's normal in the paddle's frame, then on the panel.
            let (nu, nv) = (-du, -dv);
            let m = (nu * nu + nv * nv + 1.0).sqrt();
            let (nu, nv, nz) = (nu / m, nv / m, 1.0 / m);
            let nu = nu * flip;
            let (nx, ny) = (nu * cos - nv * sin, nu * sin + nv * cos);
            let diffuse = (nx * light.0 + ny * light.1 + nz * light.2).max(0.0);
            let k = (0.3 + 0.7 * diffuse) / flat;
            let gloss = (nx * half.0 + ny * half.1 + nz * half.2).max(0.0).powi(28) * 60.0;
            let i = 4 * (py * fw + px) as usize;
            let a = f64::from(data[i + 3]);
            if a == 0.0 {
                continue;
            }
            for c in 0..3 {
                let v = f64::from(data[i + c]) * k + gloss * a / 255.0;
                data[i + c] = v.round().clamp(0.0, a) as u8;
            }
        }
    }
}

/// A wheel seen from above in its slot, `at` its middle (pixels) at `s` pixels a unit: white
/// ridged thermoset, a cylinder turning about its axis across the slot, lit by the panel's lamp
/// (above, and up and to the left of, the panel): its ridges every 8 degrees rolling with it,
/// each lit on its flank towards the lamp; its rim rounded over at its sides; its ends going
/// down into the slot's shade; PITCH's line or MOD.'s dot rolling with it.
fn draw_wheel(
    frame: &mut Pixmap,
    at: (f64, f64),
    s: f64,
    wheel: art::WheelArt,
    albedo: Option<[f64; 3]>,
) {
    let k = art::WHEEL_SCALE;
    let (hw, hh, r, corner) = (24.0 * k, 67.0 * k, 84.0 * k, 10.0 * k);
    let unit = |(x, y, z): (f64, f64, f64)| {
        let n = (x * x + y * y + z * z).sqrt();
        (x / n, y / n, z / n)
    };
    let (light, half) = lamp_vectors();
    // Its colour: a look's, else the built-in cream.
    let albedo = albedo.unwrap_or([0.80, 0.765, 0.67]);
    let (fw, fh) = (frame.width() as i32, frame.height() as i32);
    let x0 = ((at.0 - hw * s).floor() as i32).max(0);
    let x1 = ((at.0 + hw * s).ceil() as i32).min(fw);
    let y0 = ((at.1 - hh * s).floor() as i32).max(0);
    let y1 = ((at.1 + hh * s).ceil() as i32).min(fh);
    let data = frame.data_mut();
    for py in y0..y1 {
        for px in x0..x1 {
            let (dx, dy) = (
                (f64::from(px) + 0.5 - at.0) / s,
                (f64::from(py) + 0.5 - at.1) / s,
            );
            // Inside the wheel's rounded window, smoothed over a pixel.
            let (qx, qy) = (
                (dx.abs() - (hw - corner)).max(0.0),
                (dy.abs() - (hh - corner)).max(0.0),
            );
            let cover = ((corner - qx.hypot(qy)) * s + 0.5).clamp(0.0, 1.0);
            if cover <= 0.0 {
                continue;
            }
            // Where on the cylinder: the angle from its top, towards the panel's foot.
            let th = (dy / r).clamp(-0.999, 0.999).asin();
            let deg = th.to_degrees();
            // A ridge's flanks tilt the surface along the wheel.
            let mut ph = ((deg + wheel.turn) / 8.0).rem_euclid(1.0);
            if ph > 0.5 {
                ph -= 1.0;
            }
            let tilt = if ph.abs() < 0.18 {
                -0.55 * (std::f64::consts::PI * ph / 0.18).sin()
            } else {
                0.0
            };
            let a = th + tilt;
            let side = (dx / hw).signum() * (dx.abs() / hw).powi(4) * 0.75;
            let n = unit((side, a.sin(), a.cos() * (1.0 - side * side).max(0.0).sqrt()));
            let diffuse = (n.0 * light.0 + n.1 * light.1 + n.2 * light.2).max(0.0);
            let spec = (n.0 * half.0 + n.1 * half.1 + n.2 * half.2)
                .max(0.0)
                .powi(36)
                * 0.3;
            // Its ends go down into the slot's shade.
            let t = ((dy.abs() / hh - 0.55) / 0.45).clamp(0.0, 1.0);
            let shade = 1.0 - 0.6 * t * t * (3.0 - 2.0 * t);
            let marked = match wheel.mark {
                Mark::Line => (deg + wheel.turn).abs() < 1.3,
                Mark::Dot => {
                    let at = (7.0 - wheel.turn).to_radians();
                    at.cos() > 0.0 && dx.hypot(dy - r * at.sin()) < 5.0 * k
                }
            };
            let i = 4 * (py * fw + px) as usize;
            for (c, alb) in albedo.iter().enumerate() {
                let lit = if marked {
                    0.012
                } else {
                    alb * (0.2 + 0.85 * diffuse) * shade + spec * shade
                };
                let v = 255.0 * lit.clamp(0.0, 1.0).powf(1.0 / 2.2);
                data[i + c] = (v * cover + f64::from(data[i + c]) * (1.0 - cover)).round() as u8;
            }
            data[i + 3] = (255.0 * cover + f64::from(data[i + 3]) * (1.0 - cover)).round() as u8;
        }
    }
}

/// A picture drawn with its middle at `at` (pixels), mirrored as `flip` (across, down) says,
/// then turned `deg` clockwise.
/// A picture `p` placed as [`put_picture`] puts it, at full opacity, on a pixmap of its own
/// just round it: that pixmap and where its top left corner is (whole pixels), so that put
/// there it is the picture put directly, to the bit.
fn place_picture(
    p: &Pixmap,
    at: (f64, f64),
    deg: f32,
    flip: (bool, bool),
    zoom: f32,
) -> Option<(i32, i32, Pixmap)> {
    let (pw, ph) = (p.width() as f32, p.height() as f32);
    let sign = |f: bool| if f { -zoom } else { zoom };
    let t = Transform::from_translate(-pw / 2.0, -ph / 2.0)
        .post_scale(sign(flip.0), sign(flip.1))
        .post_rotate(deg)
        .post_translate(at.0 as f32, at.1 as f32);
    let mut corners = [
        Point::from_xy(0.0, 0.0),
        Point::from_xy(pw, 0.0),
        Point::from_xy(0.0, ph),
        Point::from_xy(pw, ph),
    ];
    t.map_points(&mut corners);
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for c in corners {
        (x0, y0, x1, y1) = (x0.min(c.x), y0.min(c.y), x1.max(c.x), y1.max(c.y));
    }
    let (x, y) = (x0.floor() as i32 - 1, y0.floor() as i32 - 1);
    let (w, h) = (
        (x1.ceil() as i32 + 1 - x).max(1) as u32,
        (y1.ceil() as i32 + 1 - y).max(1) as u32,
    );
    let mut out = Pixmap::new(w, h)?;
    out.draw_pixmap(
        0,
        0,
        p.as_ref(),
        &PixmapPaint {
            quality: FilterQuality::Bicubic,
            ..PixmapPaint::default()
        },
        t.post_translate(-x as f32, -y as f32),
        None,
    );
    Some((x, y, out))
}

/// `a` times `b` over 255, rounded (as tiny-skia's 8-bit pipeline multiplies).
#[inline(always)]
pub(crate) fn mul255(a: u8, b: u8) -> u8 {
    let t = u16::from(a) * u16::from(b) + 128;
    ((t + (t >> 8)) >> 8) as u8
}

/// `src` (premultiplied) put over `dst` with its top left corner at pixel (`x`, `y`),
/// `opacity` (0 to 1) opaque: as tiny-skia's `draw_pixmap` puts a pixmap neither scaled nor
/// turned at whole pixels, but several times as fast (tiny-skia samples it as a pattern, a
/// pixel at a time, in floats; decisions.md R59).
pub(crate) fn put(dst: &mut Pixmap, src: &Pixmap, (x, y): (i32, i32), opacity: f32) {
    let o = (opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    if o == 0 {
        return;
    }
    let (dw, dh) = (dst.width() as i32, dst.height() as i32);
    let (sw, sh) = (src.width() as i32, src.height() as i32);
    let (x0, y0) = (x.max(0), y.max(0));
    let (x1, y1) = ((x + sw).min(dw), (y + sh).min(dh));
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let n = (x1 - x0) as usize * 4;
    let (sd, dd) = (src.data(), dst.data_mut());
    for row in y0..y1 {
        let from = (((row - y) * sw + (x0 - x)) * 4) as usize;
        let to = ((row * dw + x0) * 4) as usize;
        let (s, d) = (&sd[from..from + n], &mut dd[to..to + n]);
        if o == 255 {
            for (d, s) in d.chunks_exact_mut(4).zip(s.chunks_exact(4)) {
                let keep = 255 - s[3];
                for j in 0..4 {
                    d[j] = s[j] + mul255(d[j], keep);
                }
            }
        } else {
            for (d, s) in d.chunks_exact_mut(4).zip(s.chunks_exact(4)) {
                let keep = 255 - mul255(s[3], o);
                for j in 0..4 {
                    d[j] = mul255(s[j], o) + mul255(d[j], keep);
                }
            }
        }
    }
}

fn put_picture(
    frame: &mut Pixmap,
    p: &Pixmap,
    at: (f64, f64),
    deg: f32,
    flip: (bool, bool),
    (zoom, alpha): (f32, f32),
) {
    let (pw, ph) = (p.width() as f32, p.height() as f32);
    let sign = |f: bool| if f { -zoom } else { zoom };
    frame.draw_pixmap(
        0,
        0,
        p.as_ref(),
        &PixmapPaint {
            quality: FilterQuality::Bicubic,
            opacity: alpha.clamp(0.0, 1.0),
            ..PixmapPaint::default()
        },
        Transform::from_translate(-pw / 2.0, -ph / 2.0)
            .post_scale(sign(flip.0), sign(flip.1))
            .post_rotate(deg)
            .post_translate(at.0 as f32, at.1 as f32),
        None,
    );
}

/// A rectangle (pixels: left, top, width, height), its corners rounded by `r` if given, filled
/// with a picture placed by `place`, mirrored tile to tile beyond it.
pub(crate) fn fill(
    frame: &mut Pixmap,
    p: &Pixmap,
    (x, y, w, h): (f64, f64, f64, f64),
    r: Option<f64>,
    place: Transform,
) {
    let paint = Paint {
        shader: Pattern::new(
            p.as_ref(),
            SpreadMode::Reflect,
            FilterQuality::Bicubic,
            1.0,
            place,
        ),
        anti_alias: true,
        ..Paint::default()
    };
    let path = match r {
        Some(r) => rounded_rect(x, y, w, h, r),
        None => Rect::from_xywh(x as f32, y as f32, w as f32, h as f32).map(PathBuilder::from_rect),
    };
    if let Some(path) = path {
        frame.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

/// A rounded rectangle's outline (pixels).
pub(crate) fn rounded_rect(
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    r: f64,
) -> Option<resvg::tiny_skia::Path> {
    let (x, y, w, h) = (x as f32, y as f32, w as f32, h as f32);
    let r = (r as f32).min(w / 2.0).min(h / 2.0);
    // A quarter circle's control points, as a share of its radius.
    let k = 0.552_284_8 * r;
    let mut p = PathBuilder::new();
    p.move_to(x + r, y);
    p.line_to(x + w - r, y);
    p.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    p.line_to(x + w, y + h - r);
    p.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    p.line_to(x + r, y + h);
    p.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    p.line_to(x, y + r);
    p.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    p.close();
    p.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controls::index;
    use crate::ultra::Opening;
    use std::time::Instant;

    /// A frame put together again where it changed is the frame put together whole, to the
    /// bit: through knobs turned above the strip and on it, a switch, a tip coming and going.
    /// A lamp's light drawn from its picture, made once at its strongest, is its light drawn
    /// afresh at its strength as it was at each tick, within three levels of 255 (each light
    /// rounded once more; the Tesla lamp's flash over its steady spill, twice): the amber lamp
    /// and the Tesla lamp, quiet, playing and sparking, at rest and coming up (the flare then
    /// scaled), at the window's opening scale and a Retina screen's (decisions.md R58).
    #[test]
    fn a_light_from_its_picture_is_the_light_drawn() {
        for scale in [1720.0 / W, 3440.0 / W] {
            let mut r = Renderer::with_skin(Skin::Worn, scale, 1.0);
            for (lamp, coil) in [(1.0, 0.0), (0.0, 1.0)] {
                for (p, level, spark) in [
                    (1.0, 0.0, 0.0),
                    (1.0, 0.45, 0.3),
                    (1.0, 1.0, 1.1),
                    (0.8, 0.7, 0.6),
                ] {
                    let o = Opening {
                        lamp: lamp * p,
                        coil: coil * p,
                        level,
                        spark,
                        ..Opening::default()
                    };
                    let [_, mut front] = crate::ultra::ultra_worn(&o);
                    assert!(!front.lights.is_empty());
                    // (The lightning left out: `the_opening_drawn_by_code_is_the_opening_drawn`.)
                    front.drawn.clear();
                    // The same layer with its lights drawn as they were, under its SVG.
                    let mut drawn = front.clone();
                    drawn.lights.clear();
                    let mut over = String::new();
                    for (i, l) in front.lights.iter().enumerate() {
                        over += &l
                            .glow
                            .drawn(l.zoom, l.alpha)
                            .replace("id='", &format!("id='l{i}-"))
                            .replace("url(#", &format!("url(#l{i}-"));
                    }
                    drawn.over = over + &front.over;
                    let (_, _, a) = r.render_layer(&front).expect("drawn");
                    let (_, _, b) = r.render_layer(&drawn).expect("drawn");
                    let off = a
                        .data()
                        .iter()
                        .zip(b.data())
                        .map(|(x, y)| x.abs_diff(*y))
                        .max()
                        .unwrap_or(0);
                    assert!(
                        off <= 3,
                        "scale {scale:.2}, {o:?}: {off} levels off the light drawn"
                    );
                }
            }
        }
    }

    /// What the layers that cost most cost to draw at a Retina window's scale, as they were
    /// drawn (SVG, every picture resampled) and as they are (decisions.md R59): QUALITY's
    /// shutter half closed, the Tesla lamp sparking, the amber lamp lit and playing, a knob
    /// turned. Printed, the best of five runs of 100 (`-- --ignored --nocapture`).
    #[test]
    #[ignore = "prints timings"]
    fn costs_timings() {
        use crate::art::Blend;
        let scale = 3440.0 / W;
        let mut r = Renderer::with_skin(Skin::Worn, scale, 1.0);
        let scene = Scene::default();
        r.render(&scene);
        // A layer as it was drawn: what is drawn by code as SVG, overlays as SVG blended.
        let as_was = |l: &Layer| {
            let mut was = l.clone();
            let mut over: String = l.drawn.iter().map(|d| d.svg()).collect();
            for o in &l.overlays {
                over += &match o.blend {
                    Blend::Normal => o.svg.clone(),
                    Blend::ColorDodge => {
                        format!("<g style='mix-blend-mode:color-dodge'>{}</g>", o.svg)
                    }
                    Blend::Multiply => format!("<g style='mix-blend-mode:multiply'>{}</g>", o.svg),
                };
            }
            was.over = over + &l.over;
            was.drawn.clear();
            was.overlays.clear();
            was
        };
        let time = |r: &mut Renderer, l: &Layer, kept: bool| {
            let mut best = f64::MAX;
            for _ in 0..5 {
                let t = Instant::now();
                for _ in 0..100 {
                    if !kept {
                        r.placed.clear();
                    }
                    r.render_layer(l);
                }
                best = best.min(t.elapsed().as_secs_f64() * 10.0);
            }
            best
        };
        let [_, shutter] = crate::ultra::ultra_worn(&Opening {
            lamp: 0.2,
            ..Opening::default()
        });
        let [_, tesla] = crate::ultra::ultra_worn(&Opening {
            coil: 1.0,
            level: 1.0,
            spark: 1.1,
            ..Opening::default()
        });
        let [_, amber] = crate::ultra::ultra_worn(&Opening {
            lamp: 1.0,
            level: 0.6,
            ..Opening::default()
        });
        let knob = r.layers(&scene)[index("emphasis").expect("a control")].clone();
        for (name, l) in [
            ("the shutter half closed", shutter),
            ("the Tesla lamp sparking", tesla),
            ("the amber lamp lit", amber),
            ("a knob turned", knob),
        ] {
            let was = time(&mut r, &as_was(&l), false);
            let now = time(&mut r, &l, true);
            println!("{name}: {was:.3} ms as it was, {now:.3} ms now");
        }
    }

    /// QUALITY's opening drawn by code (its shutter, the hamster's shade in the well, the Tesla
    /// lamp's lightning) is the opening drawn as SVG, as it was: closing, opening, nearly open,
    /// over the lamp and the hamster, sparking soft and loud, at the window's opening scale and
    /// a Retina screen's: on the whole within a quarter of a level of 255, under half a percent
    /// of it more than three levels off, none more than 16 (along the blades' edges, whose
    /// coverage of a pixel is worked out a pixel at a time, not by the rasterizer; decisions.md
    /// R59).
    #[test]
    fn the_opening_drawn_by_code_is_the_opening_drawn() {
        for scale in [1720.0 / W, 3440.0 / W] {
            let mut r = Renderer::with_skin(Skin::Worn, scale, 1.0);
            for o in [
                Opening::default(),
                Opening {
                    lamp: 0.1,
                    ..Opening::default()
                },
                Opening {
                    lamp: 0.25,
                    ..Opening::default()
                },
                Opening {
                    lamp: 0.43,
                    ..Opening::default()
                },
                Opening {
                    hamster: 0.3,
                    run: 1.0,
                    ..Opening::default()
                },
                Opening {
                    hamster: 0.6,
                    run: 1.0,
                    ..Opening::default()
                },
                Opening {
                    coil: 1.0,
                    level: 0.3,
                    spark: 0.4,
                    ..Opening::default()
                },
                Opening {
                    coil: 1.0,
                    level: 1.0,
                    spark: 1.1,
                    ..Opening::default()
                },
            ] {
                let [_, mut front] = crate::ultra::ultra_worn(&o);
                assert!(!front.drawn.is_empty(), "{o:?}: nothing drawn by code");
                front.lights.clear();
                let mut svg = front.clone();
                svg.drawn.clear();
                svg.over = front.drawn.iter().map(|d| d.svg()).collect::<String>() + &front.over;
                let (_, _, a) = r.render_layer(&front).expect("drawn");
                let (_, _, b) = r.render_layer(&svg).expect("drawn");
                let off: Vec<u8> = a
                    .data()
                    .iter()
                    .zip(b.data())
                    .map(|(x, y)| x.abs_diff(*y))
                    .collect();
                let most = off.iter().copied().max().unwrap_or(0);
                let mean = off.iter().map(|&d| f64::from(d)).sum::<f64>() / off.len() as f64;
                let far = off.iter().filter(|&&d| d > 3).count() as f64 / off.len() as f64;
                let at = format!("scale {scale:.2}, {o:?}");
                assert!(mean < 0.25, "{at}: {mean:.3} levels off on average");
                assert!(far < 0.005, "{at}: {:.2} % past 3 levels off", far * 100.0);
                assert!(most <= 16, "{at}: {most} levels off");
            }
        }
    }

    /// The standalone's TEMPO screen mirrors the name plate (the owner's option D): its surround
    /// as wide and tall, at the same height, as far from the left edge as the plate is from the
    /// right; found where it is drawn, and only there.
    #[test]
    fn the_tempo_screen_mirrors_the_name_plate() {
        let (x, y, w, h) = art::TEMPO_SCREEN;
        let o = crate::strip::NAME_SURROUND;
        let official = Arrangement::official();
        assert_eq!(art::tempo_window(official.plate_at()), art::TEMPO_SCREEN);
        let tempo_hit = |x, y| art::tempo_hit(&official, x, y);
        assert_eq!(
            (x - o, y - o),
            (art::W - art::PLATE_X - art::PLATE_W, art::PLATE_Y)
        );
        assert_eq!((w + 2.0 * o, h + 2.0 * o), (art::PLATE_W, art::PLATE_H));
        assert!(tempo_hit(x + w / 2.0, y + h / 2.0) && tempo_hit(x - o + 0.5, y - o + 0.5));
        assert!(!tempo_hit(x - o - 1.0, y) && !tempo_hit(x, y + h + o + 1.0));
        assert!(y + h + o < art::TOP, "on the wood");
    }

    #[test]
    fn a_frame_put_together_in_part_is_the_frame_put_together_whole() {
        let s = 0.25;
        let mut r = Renderer::new(s, 1.0);
        let mut scene = Scene {
            values: [0.5; CONTROLS.len()],
            ..Scene::default()
        };
        r.render(&scene);
        let at = |p: &str| index(p).expect(p);
        for i in 0..14 {
            match i % 7 {
                0 => scene.values[at("cutoff")] = 0.1 * f64::from(i),
                1 => scene.values[at("drive")] = 0.08 * f64::from(i),
                2 => scene.values[at("pitch_wheel")] = 0.05 * f64::from(i),
                3 => scene.values[at("osc1_on")] = f64::from(i / 7 % 2),
                4 => scene.tip = Some(("CUTOFF: 1.2 kHz".into(), 2100.0, 300.0)),
                // The standalone's TEMPO screen shown, set, and gone (R-ARP), its TYPING and
                // PANIC keys with it, lit, held and pointed at (R-TEMPO-CORNER).
                5 => {
                    scene.tempo = Some(format!("{} BPM", 100 + i));
                    scene.tempo_keys = Some(TempoKeys {
                        typing: i > 7,
                        panic: i < 7,
                        midi: i > 7,
                        hover: (i < 7).then_some(art::TempoKey::Typing),
                    });
                }
                _ => {
                    scene.tip = None;
                    if i > 7 {
                        scene.tempo = None;
                        scene.tempo_keys = None;
                    }
                }
            }
            assert!(r.render(&scene));
            let mut whole = Renderer::new(s, 1.0);
            whole.render(&scene);
            assert!(
                r.frame() == whole.frame(),
                "step {i}: not as put together whole"
            );
        }
        // The TEMPO keys in the worn skin too, where their caps are pictures.
        let mut r = Renderer::with_skin(Skin::Worn, s, 1.0);
        let mut scene = Scene {
            values: [0.5; CONTROLS.len()],
            tempo: Some("120 BPM".into()),
            ..Scene::default()
        };
        r.render(&scene);
        for (i, keys) in [
            Some(TempoKeys::default()),
            Some(TempoKeys {
                typing: true,
                ..TempoKeys::default()
            }),
            Some(TempoKeys {
                typing: true,
                panic: true,
                midi: false,
                hover: Some(art::TempoKey::Panic),
            }),
            Some(TempoKeys {
                midi: true,
                hover: Some(art::TempoKey::Midi),
                ..TempoKeys::default()
            }),
            Some(TempoKeys {
                hover: Some(art::TempoKey::Typing),
                ..TempoKeys::default()
            }),
            None,
        ]
        .into_iter()
        .enumerate()
        {
            scene.tempo_keys = keys;
            assert!(r.render(&scene));
            let mut whole = Renderer::with_skin(Skin::Worn, s, 1.0);
            whole.render(&scene);
            assert!(
                r.frame() == whole.frame(),
                "worn step {i}: not as put together whole"
            );
        }
    }

    /// The TEMPO keys (R-TEMPO-CORNER): the rail's keys' size, centred on the TEMPO screen's
    /// height, the rail's gap after its surround and between them, each hit where it is drawn
    /// and nowhere else.
    #[test]
    fn the_tempo_keys_follow_the_screen_at_the_rails_size_and_gaps() {
        let official = crate::Arrangement::official();
        let (x, y, w, h) = art::TEMPO_SCREEN;
        let (tx, ty) = art::tempo_key_at(&official, art::TempoKey::Typing);
        let (px, py) = art::tempo_key_at(&official, art::TempoKey::Panic);
        let (mx, my) = art::tempo_key_at(&official, art::TempoKey::Midi);
        assert_eq!(art::TEMPO_KEY, (crate::strip::KEY_W, crate::strip::KEY_H));
        assert_eq!((ty, py, my), (y + h / 2.0, y + h / 2.0, y + h / 2.0));
        let half = art::TEMPO_KEY.0 / 2.0;
        assert_eq!(tx - half - (x + w + art::TEMPO_SURROUND), 14.0);
        // (MIDI between TYPING and PANIC, the same gaps: R-MIDI-MAP.)
        assert_eq!(mx - half - (tx + half), 14.0);
        assert_eq!(px - half - (mx + half), 14.0);
        let hit = |x, y| art::tempo_key_hit(&official, x, y);
        assert_eq!(hit(tx, ty), Some(art::TempoKey::Typing));
        assert_eq!(hit(px, py), Some(art::TempoKey::Panic));
        assert_eq!(hit(mx, my), Some(art::TempoKey::Midi));
        assert_eq!(hit((tx + mx) / 2.0, ty), None, "the gap before MIDI");
        assert_eq!(hit((mx + px) / 2.0, py), None, "the gap after it");
        assert_eq!(hit(x + w / 2.0, y + h / 2.0), None, "the screen");
        assert_eq!(hit(px + half + 5.0, py), None);
        assert!(
            !art::tempo_hit(&official, tx, ty),
            "the screen's own hit stops short"
        );
    }

    /// A timed dial's part of the background (R-OSC-TIMED), in either skin: the background's own
    /// pixels but for the dial's marks, its edges and what both prints share (a legend reaching
    /// into it) to the bit, so no seam where it is laid; the panel's own print painted so is the
    /// background itself; and the dial's marks do change.
    #[test]
    fn a_timed_dials_part_of_the_background_is_laid_without_a_seam() {
        for skin in [Skin::Drawn, Skin::Worn] {
            for s in [0.37, 0.44, 0.87] {
                let r = Renderer::with_skin(skin, s, 1.0);
                let (panel, timed) = (Arc::clone(&r.arrangement), r.timed_arrangement());
                let whole = r.background.clone();
                for (n, rect) in r.timed_rects.into_iter().enumerate() {
                    let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
                    let at = |p: &Pixmap, x: i32, y: i32| {
                        let c = p.pixel(x as u32, y as u32).expect("in it");
                        [c.red(), c.green(), c.blue(), c.alpha()]
                    };
                    let (plain, part) = (
                        r.timed_part(n, &panel).expect("a part"),
                        r.timed_part(n, &timed).expect("a part"),
                    );
                    let mut changed = 0usize;
                    for y in 0..h {
                        for x in 0..w {
                            let bg = at(&whole, x + rect[0], y + rect[1]);
                            assert_eq!(at(&plain, x, y), bg, "{skin:?} at {s}: the panel's own");
                            let edge = x.min(w - 1 - x).min(y).min(h - 1 - y) == 0;
                            if edge {
                                assert_eq!(at(&part, x, y), bg, "{skin:?} at {s}: its edge");
                            }
                            changed += usize::from(at(&part, x, y) != bg);
                        }
                    }
                    assert!(changed > 0, "{skin:?} at {s}: the dial's marks changed");
                }
            }
        }
    }

    /// A timed dial's part (R-OSC-TIMED) is what the whole background printed with the timed
    /// dials has there, in either skin, but for the anti-aliasing of a few of the dial's pixels
    /// (its ink drawn at another origin).
    #[test]
    fn a_timed_dials_part_is_the_whole_timed_backgrounds() {
        for skin in [Skin::Drawn, Skin::Worn] {
            for s in [0.37, 0.87] {
                let r = Renderer::with_skin(skin, s, 1.0);
                let timed = r.timed_arrangement();
                let mut look = Look::of(skin);
                look.arrangement = Arc::new(timed.clone());
                let whole = Renderer::with_pack(look, Lettering::Standard, s, 1.0);
                for (n, rect) in r.timed_rects.into_iter().enumerate() {
                    let part = r.timed_part(n, &timed).expect("a part");
                    let (mut worst, mut off, mut count) = (0u8, 0usize, 0usize);
                    for y in rect[1]..rect[3] {
                        for x in rect[0]..rect[2] {
                            let p = part
                                .pixel((x - rect[0]) as u32, (y - rect[1]) as u32)
                                .expect("in it");
                            let q = whole.background.pixel(x as u32, y as u32).expect("in it");
                            let d = [
                                p.red().abs_diff(q.red()),
                                p.green().abs_diff(q.green()),
                                p.blue().abs_diff(q.blue()),
                            ]
                            .into_iter()
                            .max()
                            .unwrap_or(0);
                            worst = worst.max(d);
                            off += usize::from(d > 2);
                            count += 1;
                        }
                    }
                    println!(
                        "{skin:?} at {s}, dial {n}: {off} of {count} pixels over 2 levels, at most {worst}"
                    );
                    assert!(off * 200 < count, "{skin:?} at {s}: {off} of {count} apart");
                }
            }
        }
    }

    /// Oscillators 2's and 3's timed mode (decisions.md R-OSC-TIMED) in either skin: a key
    /// pressed at LO lights it and prints its FREQUENCY dial in note values; its sign follows its
    /// division; at another RANGE the key is dimmed and the dial is the semitone dial; each frame
    /// put together in part is the frame put together whole, and off again is the frame before,
    /// to the pixel. Only the key and the dial's print change.
    #[test]
    fn a_timed_dial_and_its_key_are_drawn_and_undrawn_in_place() {
        let s = 0.2;
        for skin in [Skin::Drawn, Skin::Worn] {
            let at = |p: &str| index(p).expect(p);
            let mut scene = Scene {
                values: [0.5; CONTROLS.len()],
                ..Scene::default()
            };
            for range in ["osc2_range", "osc3_range"] {
                scene.values[at(range)] = 0.0;
            }
            for key in TIMED_KEYS {
                scene.values[at(key)] = 0.0;
            }
            let mut r = Renderer::with_skin(skin, s, 1.0);
            // (The built-in typeface prints 4× and 2× with the multiplication sign.)
            assert_eq!(r.layout().times(), "×");
            r.render(&scene);
            let before = r.frame().clone();
            let steps: [(usize, f64, usize, bool); 5] = [
                // (Oscillator, its key, its division, its RANGE away from LO.)
                (1, 1.0, 9, false),
                (0, 1.0, 2, false),
                (1, 1.0, 16, false),
                (0, 1.0, 2, true),
                (1, 0.0, 16, false),
            ];
            for (step, &(n, key, division, away)) in steps.iter().enumerate() {
                scene.values[at(TIMED_KEYS[n])] = key;
                scene.timed[n] = Timed {
                    division,
                    inert: away,
                };
                let ranges = ["osc2_range", "osc3_range"];
                scene.values[at(ranges[n])] = if away { 0.6 } else { 0.0 };
                if scene.timed_dials()[n] {
                    scene.values[at(TIMED_DIALS[n])] = division as f64 / 17.0;
                }
                assert!(r.render(&scene), "{skin:?} step {step}");
                let mut whole = Renderer::with_skin(skin, s, 1.0);
                whole.render(&scene);
                assert!(
                    r.frame() == whole.frame(),
                    "{skin:?} step {step}: not as put together whole"
                );
            }
            assert_eq!(scene.timed_dials(), [false, false]);
            // Everything back as it was: the same frame as before, to the pixel.
            scene.values[at(TIMED_KEYS[0])] = 0.0;
            scene.values[at("osc2_range")] = 0.0;
            scene.timed = [Timed::default(); 2];
            for d in TIMED_DIALS {
                scene.values[at(d)] = 0.5;
            }
            r.render(&scene);
            assert!(r.frame() == &before, "{skin:?}: not the frame before");
        }
    }

    /// Each oscillator's waveform (decisions.md R-CONTINUOUS-WAVE) in either skin: CONTINUOUS
    /// prints its WAVEFORM's arc (oscillator 3's legends too), puts its MOD key on the panel in its
    /// way's colour and turns its switch anywhere; MOD's orb comes, moves and goes; each frame put
    /// together in part is the frame put together whole, and STEPPED again is the frame before,
    /// to the pixel. Only the switch, its print, its key and the orb change.
    #[test]
    fn a_continuous_switchs_arc_key_and_orb_are_drawn_and_undrawn_in_place() {
        use crate::art::ModWay;
        let s = 0.2;
        for skin in [Skin::Drawn, Skin::Worn] {
            let at = |p: &str| index(p).expect(p);
            let mut scene = Scene {
                values: [0.5; CONTROLS.len()],
                ..Scene::default()
            };
            for key in MOD_KEYS {
                scene.values[at(key)] = 0.0;
            }
            let mut r = Renderer::with_skin(skin, s, 1.0);
            r.render(&scene);
            let before = r.frame().clone();
            let steps: [(usize, bool, f64, ModWay, Option<f64>, f64); 7] = [
                // (Oscillator, CONTINUOUS, its place, MOD's way, its orb, its MOD key.)
                (0, true, 1.4, ModWay::Clockwise, None, 0.0),
                (2, true, 5.95, ModWay::Anticlockwise, Some(4.2), 1.0),
                (0, true, 0.3, ModWay::Both, Some(0.9), 1.0),
                (0, true, 0.3, ModWay::Both, Some(2.6), 1.0),
                (2, false, 5.95, ModWay::Anticlockwise, None, 1.0),
                (1, true, 2.0, ModWay::Clockwise, Some(2.0), 1.0),
                (1, true, 2.0, ModWay::Clockwise, None, 0.0),
            ];
            for (step, &(n, continuous, place, way, orb, key)) in steps.iter().enumerate() {
                scene.shape[n] = Shaped {
                    continuous,
                    at: place,
                    way,
                    orb,
                };
                scene.values[at(MOD_KEYS[n])] = key;
                assert!(r.render(&scene), "{skin:?} step {step}");
                let mut whole = Renderer::with_skin(skin, s, 1.0);
                whole.render(&scene);
                assert!(
                    r.frame() == whole.frame(),
                    "{skin:?} step {step}: not as put together whole"
                );
            }
            assert_eq!(scene.arcs(), [true, true, false]);
            // Everything back as it was: the same frame as before, to the pixel.
            scene.shape = [Shaped::default(); 3];
            for key in MOD_KEYS {
                scene.values[at(key)] = 0.0;
            }
            r.render(&scene);
            assert!(r.frame() == &before, "{skin:?}: not the frame before");
        }
    }

    /// LONG's print appears and disappears in place in both skins and lettering sizes; a
    /// partial redraw is the same frame as one drawn whole, and returning NORMAL is exact.
    #[test]
    fn long_glides_print_is_drawn_and_undrawn_in_place() {
        for skin in [Skin::Drawn, Skin::Worn] {
            for lettering in [Lettering::Standard, Lettering::Large] {
                let s = 0.44;
                let mut r = Renderer::with_look(skin, lettering, s, 1.0);
                let mut scene = Scene::default();
                r.render(&scene);
                let before = r.frame().clone();
                scene.glide_long = true;
                assert!(r.render(&scene));
                assert!(r.frame() != &before, "{skin:?}: no print");
                for y in 0..before.height() {
                    for x in 0..before.width() {
                        let [x0, y0, x1, y1] = r.glide_rect;
                        if (x as i32) < x0
                            || (x as i32) >= x1
                            || (y as i32) < y0
                            || (y as i32) >= y1
                        {
                            let i = (y as usize * before.width() as usize + x as usize) * 4;
                            assert_eq!(&before.data()[i..i + 4], &r.frame().data()[i..i + 4]);
                        }
                    }
                }
                for v in [0.0, 0.75, 1.0] {
                    scene.values[index("glide").expect("GLIDE")] = v;
                    r.render(&scene);
                    let mut whole = Renderer::with_look(skin, lettering, s, 1.0);
                    whole.render(&scene);
                    assert!(r.frame() == whole.frame());
                }
                scene = Scene::default();
                r.render(&scene);
                assert!(r.frame() == &before, "NORMAL is the frame before");
            }
        }
    }

    /// The LONG sign's overlay meets neither another replaceable print nor any of the
    /// background's print, including GLIDE's own numbers, in either lettering size.
    #[test]
    fn long_glides_print_meets_no_other_print() {
        let starter = Look::open(std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/skin-packs/starter"
        )))
        .expect("the starter pack");
        for look in [Look::of(Skin::Drawn), starter] {
            for lettering in [Lettering::Standard, Lettering::Large] {
                let r = Renderer::with_pack(look.clone(), lettering, 1.0, 1.0);
                let g = r.glide_rect;
                for other in r.arc_rects.into_iter().chain(r.timed_rects) {
                    assert!(
                        g[2] <= other[0]
                            || other[2] <= g[0]
                            || g[3] <= other[1]
                            || other[3] <= g[1]
                    );
                }
                let doc = art::document(
                    [
                        g[0] as f64,
                        g[1] as f64,
                        (g[2] - g[0]) as f64,
                        (g[3] - g[1]) as f64,
                    ],
                    (g[2] - g[0]) as u32,
                    (g[3] - g[1]) as u32,
                    &art::panel_print(&r.layout, &r.arrangement),
                );
                let tree = usvg::Tree::from_str(&doc, &r.options).expect("the panel's print");
                let mut print =
                    Pixmap::new((g[2] - g[0]) as u32, (g[3] - g[1]) as u32).expect("a print");
                resvg::render(&tree, Transform::identity(), &mut print.as_mut());
                assert!(
                    print.pixels().iter().all(|p| p.alpha() == 0),
                    "{lettering:?}: another print in LONG's place"
                );
            }
        }
    }

    /// A WAVEFORM switch's CONTINUOUS print keeps clear of the FREQUENCY dials' (each laid over
    /// the background alone: R-CONTINUOUS-WAVE, R-OSC-TIMED).
    #[test]
    fn the_arcs_print_and_the_timed_dials_print_do_not_meet() {
        for skin in [Skin::Drawn, Skin::Worn] {
            let r = Renderer::with_skin(skin, 0.37, 1.0);
            for a in r.arc_rects {
                for d in r.timed_rects {
                    let apart = a[2] <= d[0] || d[2] <= a[0] || a[3] <= d[1] || d[3] <= a[1];
                    assert!(apart, "{skin:?}: {a:?} meets {d:?}");
                }
            }
        }
    }

    /// A TIMED key away from its RANGE's LO (decisions.md R-OSC-TIMED) in either skin: neither
    /// it, nor its sign, nor its cap's shadow on the panel, on or off, whatever its division: its
    /// place is the plain panel, the frame the same to the pixel as the panel drawn with the keys
    /// placed off it. At LO, on or off, it is drawn.
    #[test]
    fn a_timed_key_away_from_lo_leaves_the_plain_panel() {
        let s = 0.25;
        for skin in [Skin::Drawn, Skin::Worn] {
            let mut scene = Scene {
                values: [0.5; CONTROLS.len()],
                ..Scene::default()
            };
            for key in TIMED_KEYS {
                scene.values[index(key).expect(key)] = 0.0;
            }
            // The panel with the keys placed above the frame, there (off) but out of sight.
            let mut look = Look::of(skin);
            let mut away = (*look.arrangement).clone();
            for key in TIMED_KEYS {
                away.controls[index(key).expect(key)].y = -2000.0;
            }
            look.arrangement = Arc::new(away);
            let mut plain = Renderer::with_pack(look, Lettering::Standard, s, 1.0);
            plain.render(&scene);
            let mut r = Renderer::with_skin(skin, s, 1.0);
            r.render(&scene);
            assert!(r.frame() != plain.frame(), "{skin:?}: no key at LO");
            for (on, division) in [(0.0, 8), (1.0, 2), (1.0, 16), (0.0, 13)] {
                for key in TIMED_KEYS {
                    scene.values[index(key).expect(key)] = on;
                }
                scene.timed = [Timed {
                    division,
                    inert: true,
                }; 2];
                assert!(!scene.timed_dials().contains(&true));
                r.render(&scene);
                assert!(
                    r.frame() == plain.frame(),
                    "{skin:?} {on} {division}: not the plain panel"
                );
                let mut whole = Renderer::with_skin(skin, s, 1.0);
                whole.render(&scene);
                assert!(whole.frame() == plain.frame(), "{skin:?} {on} {division}");
            }
            // Back at LO, on: drawn again, lit.
            scene.timed = [Timed::default(); 2];
            r.render(&scene);
            assert!(r.frame() != plain.frame(), "{skin:?}: not drawn again");
        }
    }

    /// OSC. 3 CONTROL while oscillator 3 is timed (decisions.md R-OSC-TIMED) in either skin, on
    /// or off: dimmed in place (the frame put together in part the frame put together whole),
    /// only about the rocker (shaded darker in the worn skin), and live again the frame before,
    /// to the pixel; the same with oscillator 3's waveform CONTINUOUS, its MOD key on the panel
    /// and on (R-CONTINUOUS-WAVE).
    #[test]
    fn osc3_control_is_dimmed_and_undimmed_in_place() {
        let s = 0.2;
        let at = index("osc3_control").expect("a control");
        for skin in [Skin::Drawn, Skin::Worn] {
            for (on, continuous) in [(0.0, false), (1.0, false), (1.0, true)] {
                let mut scene = Scene {
                    values: [0.5; CONTROLS.len()],
                    ..Scene::default()
                };
                scene.values[at] = on;
                for key in MOD_KEYS {
                    scene.values[index(key).expect(key)] = 0.0;
                }
                if continuous {
                    scene.shape[2] = Shaped {
                        continuous,
                        at: 4.0,
                        ..Shaped::default()
                    };
                    scene.values[index(MOD_KEYS[2]).expect("a key")] = 1.0;
                }
                let mut r = Renderer::with_skin(skin, s, 1.0);
                r.render(&scene);
                let before = r.frame().clone();
                scene.osc3_control_off = true;
                assert!(r.render(&scene), "{skin:?} {on}");
                let mut whole = Renderer::with_skin(skin, s, 1.0);
                whole.render(&scene);
                assert!(
                    r.frame() == whole.frame(),
                    "{skin:?} {on}: not as put together whole"
                );
                let lum = |p: &resvg::tiny_skia::PremultipliedColorU8| {
                    u32::from(p.red()) + u32::from(p.green()) + u32::from(p.blue())
                };
                let (mut changed, mut darker) = (0, 0);
                for (a, b) in before.pixels().iter().zip(r.frame().pixels()) {
                    if a != b {
                        changed += 1;
                        darker += usize::from(lum(b) < lum(a));
                    }
                }
                let all = before.pixels().len();
                assert!(changed > 0, "{skin:?} {on}: not dimmed");
                assert!(
                    changed * 100 < all,
                    "{skin:?} {on}: {changed} of {all} changed"
                );
                // (The worn skin shades it; the drawn skin's half opacity lifts its darks too.)
                if skin == Skin::Worn {
                    assert!(
                        darker == changed,
                        "{skin:?} {on}: {darker} of {changed} darker"
                    );
                }
                scene.osc3_control_off = false;
                r.render(&scene);
                assert!(r.frame() == &before, "{skin:?} {on}: not the frame before");
            }
        }
    }

    /// QUALITY's opening and a knob far from it changing in one frame are put together again as
    /// two places, not one round both (the CA-74's review: most of the panel had been put
    /// together again), and the frame is still the frame put together whole; places that
    /// overlap are one.
    #[test]
    fn places_changed_apart_are_put_together_again_apart() {
        let s = 0.25;
        let mut r = Renderer::new(s, 1.0);
        let mut scene = Scene {
            values: [0.5; CONTROLS.len()],
            ..Scene::default()
        };
        scene.opening = Opening {
            lamp: 1.0,
            level: 0.2,
            ..Opening::default()
        };
        r.render(&scene);
        scene.opening.level = 0.8;
        scene.values[index("tune").expect("TUNE")] = 0.9;
        assert!(r.render(&scene));
        let places = r.damages().to_vec();
        assert_eq!(places.len(), 2, "{places:?}");
        let area = |d: &[i32; 4]| i64::from(d[2] - d[0]) * i64::from(d[3] - d[1]);
        let apart: i64 = places.iter().map(area).sum();
        let round = area(&r.damage().expect("changed"));
        assert!(
            apart * 8 < round,
            "{apart} pixels apart against {round} round both"
        );
        let mut whole = Renderer::new(s, 1.0);
        whole.render(&scene);
        assert!(r.frame() == whole.frame(), "not as put together whole");
        assert_eq!(
            merged(vec![[0, 0, 10, 10], [5, 5, 20, 20], [100, 0, 110, 10]]),
            vec![[0, 0, 20, 20], [100, 0, 110, 10]]
        );
        assert_eq!(
            merged(vec![[0, 0, 10, 10], [14, 0, 20, 10]]),
            vec![[0, 0, 20, 10]]
        );
        let many: Vec<_> = (0..12).map(|k| [k * 100, 0, k * 100 + 10, 10]).collect();
        assert_eq!(merged(many), vec![[0, 0, 1110, 10]]);
    }

    /// The renderer says where its frame changed: a knob turned on the panel above the strip
    /// changes nothing on the strip, one of the strip's does (its parts are drawn again over
    /// it then, `crate::strip`); the first frame changes everything.
    #[test]
    fn it_says_where_its_frame_changed() {
        let s = 0.2;
        let mut r = Renderer::new(s, 1.0);
        let mut scene = Scene::default();
        assert!(r.render(&scene));
        let (w, h) = r.size();
        assert_eq!(r.damage(), Some([0, 0, w as i32, h as i32]));
        let strip = (art::PANEL_H * s).round() as i32;
        scene.values[index("cutoff").expect("CUTOFF")] = 0.7;
        assert!(r.render(&scene));
        assert!(
            r.damage()
                .is_some_and(|d| d[3] <= strip && d[2] - d[0] < w as i32 / 8)
        );
        scene.values[index("drive").expect("DRIVE")] = 0.7;
        assert!(r.render(&scene));
        assert!(r.damage().is_some_and(|d| d[1] >= strip));
        assert!(!r.render(&scene));
        assert_eq!(r.damage(), None);
    }

    /// The pixels two frames differ in, inside and outside `within` (the drawing's units:
    /// left, top, right, bottom).
    fn differ(a: &Pixmap, b: &Pixmap, scale: f64, within: [f64; 4]) -> (usize, usize) {
        let (mut inside, mut outside) = (0, 0);
        for (i, (p, q)) in a.pixels().iter().zip(b.pixels()).enumerate() {
            if p == q {
                continue;
            }
            let (x, y) = (
                f64::from(i as u32 % a.width()) / scale,
                f64::from(i as u32 / a.width()) / scale,
            );
            if (within[0]..within[2]).contains(&x) && (within[1]..within[3]).contains(&y) {
                inside += 1;
            } else {
                outside += 1;
            }
        }
        (inside, outside)
    }

    /// The worn skin draws the panel in its pictures, not as drawn; a knob turned changes its
    /// own place and nothing else, and a rocker switched its own.
    #[test]
    fn the_worn_skin_turns_its_knobs_in_place() {
        let scale = 0.4;
        let scene = Scene {
            values: [0.5; CONTROLS.len()],
            ..Scene::default()
        };
        let mut worn = Renderer::with_skin(Skin::Worn, scale, 1.0);
        assert_eq!(worn.skin(), Skin::Worn);
        assert!(worn.render(&scene));
        let before = worn.frame().clone();
        let mut drawn = Renderer::new(scale, 1.0);
        drawn.render(&scene);
        let (all, _) = differ(&before, drawn.frame(), scale, [0.0, 0.0, W, H]);
        assert!(
            all > before.pixels().len() / 2,
            "{all} pixels from the drawn panel's"
        );
        for (param, v) in [("cutoff", 0.95), ("osc1_range", 0.0), ("filter_mod", 0.0)] {
            let i = index(param).expect("a control");
            let mut turned = scene.clone();
            turned.values[i] = v;
            assert!(worn.render(&turned), "{param} not drawn again");
            let c = &CONTROLS[i];
            let (cx, cy) = c.centre();
            // (The worn layer's extent: a rocker's reaches its own shadow.)
            let k = art::control_worn(c, v, 0.0).bounds;
            // (Its layer is whole pixels: two past its extent.)
            let m = 2.0 / scale;
            let (inside, outside) = differ(
                &before,
                worn.frame(),
                scale,
                [cx + k[0] - m, cy + k[1] - m, cx + k[2] + m, cy + k[3] + m],
            );
            assert!(inside > 50, "{param}: {inside} pixels of its own changed");
            assert_eq!(outside, 0, "{param}: {outside} pixels elsewhere changed");
            worn.render(&scene);
        }
    }

    /// A worn knob's light stays where the lamp is, up and to the left: turned to either end,
    /// its upper left is lighter than its lower right, by about as much (the pictures lit
    /// evenly all round; the lamp's light laid over them, not turned with them).
    #[test]
    fn a_worn_knobs_light_stays_with_the_lamp() {
        let scale = 0.8;
        let i = index("emphasis").expect("a control");
        let (cx, cy) = CONTROLS[i].centre();
        let light = |v: f64| {
            let mut scene = Scene {
                values: [0.5; CONTROLS.len()],
                ..Scene::default()
            };
            scene.values[i] = v;
            let mut r = Renderer::with_skin(Skin::Worn, scale, 1.0);
            r.render(&scene);
            // The mean level of a patch of the skirt, `dx`, `dy` units from the axis.
            let patch = |dx: f64, dy: f64| {
                let (x0, y0) = (((cx + dx) * scale) as u32, ((cy + dy) * scale) as u32);
                let mut sum = 0.0;
                for y in y0 - 3..y0 + 3 {
                    for x in x0 - 3..x0 + 3 {
                        let p = r.frame().pixel(x, y).expect("inside").demultiply();
                        sum += 0.2126 * f64::from(p.red())
                            + 0.7152 * f64::from(p.green())
                            + 0.0722 * f64::from(p.blue());
                    }
                }
                sum / 36.0
            };
            // Cap and grip, each at the upper left and the lower right.
            let (cap, grip) = (
                20.0 / std::f64::consts::SQRT_2,
                42.0 / std::f64::consts::SQRT_2,
            );
            (
                patch(-cap, -cap) - patch(cap, cap),
                patch(-grip, -grip) - patch(grip, grip),
            )
        };
        let (low, high) = (light(0.0), light(1.0));
        // (The cap is drawn unturned; the grip turns, its picture lit evenly all round.)
        for ((a, b), part, apart) in [
            ((low.0, high.0), "cap", 0.5),
            ((low.1, high.1), "grip", 6.0),
        ] {
            assert!(
                a > 1.0 && b > 1.0,
                "{part}: upper left {a:.1} and {b:.1} levels lighter"
            );
            assert!(
                (a - b).abs() < apart,
                "{part}: {a:.1} against {b:.1} levels as it turns"
            );
        }
    }

    /// FEEDBACK's knob is drawn dimmed while EXTERNAL INPUT is switched off, and nothing
    /// else on the panel changes.
    #[test]
    fn feedback_dims_while_external_input_is_closed() {
        let open = Scene {
            values: [0.5; CONTROLS.len()],
            ..Scene::default()
        };
        let mut closed = open.clone();
        let feedback = index("feedback").expect("a control");
        let ext = index("ext_on").expect("a control");
        closed.values[ext] = 0.0;
        let frame = |scene: &Scene| {
            let mut r = Renderer::new(0.5, 1.0);
            r.render(scene);
            (r.frame().clone(), r.scale())
        };
        let ((a, s), (b, _)) = (frame(&open), frame(&closed));
        let c = &CONTROLS[feedback];
        let (cx, cy) = c.centre();
        let k = art::bounds(&c.kind);
        let knob = |x: u32, y: u32| {
            let (x, y) = (f64::from(x) / s, f64::from(y) / s);
            (cx + k[0]..cx + k[2]).contains(&x) && (cy + k[1]..cy + k[3]).contains(&y)
        };
        // EXTERNAL INPUT's own switch moves too.
        let switch = &CONTROLS[ext];
        let (sx, sy) = switch.centre();
        let sk = art::bounds(&switch.kind);
        let rocker = |x: u32, y: u32| {
            let (x, y) = (f64::from(x) / s, f64::from(y) / s);
            (sx + sk[0]..sx + sk[2]).contains(&x) && (sy + sk[1]..sy + sk[3]).contains(&y)
        };
        let (mut inside, mut outside) = (0, 0);
        for (i, (p, q)) in a.pixels().iter().zip(b.pixels()).enumerate() {
            let (x, y) = (i as u32 % a.width(), i as u32 / a.width());
            if p != q {
                if knob(x, y) {
                    inside += 1;
                } else if !rocker(x, y) {
                    outside += 1;
                }
            }
        }
        assert!(inside > 500, "{inside} of the knob's pixels dimmed");
        assert_eq!(outside, 0, "{outside} pixels elsewhere changed");
    }

    /// The worn skin's background drawn side by side on threads of their own (decisions.md
    /// R-LINUX-RESIZE: the wood and faces in bands of rows, the shadows a part a thread) is the
    /// background drawn on one, within two levels of light (smoothing rounded once more as each
    /// thread's part is laid over the frame): at the smallest window, the window's opening size
    /// and a Retina screen's. And the shadows laid over a frame part by part are those drawn over
    /// it at once (resvg's own way), within three. (Against 0.3.0's, which drew all of it at once
    /// on one thread, the background was within three levels at these sizes when this was made.)
    #[test]
    fn the_worn_background_drawn_side_by_side_is_the_background_drawn_at_once() {
        let drawn = |bands: usize, scale: f64| {
            TEST_BANDS.with(|b| b.set(Some(bands)));
            let started = Instant::now();
            let r = Renderer::with_skin(Skin::Worn, scale, 1.0);
            let took = started.elapsed();
            TEST_BANDS.with(|b| b.set(None));
            (r, took)
        };
        let most = |a: &Pixmap, b: &Pixmap| {
            assert_eq!(a.data().len(), b.data().len());
            a.data()
                .iter()
                .zip(b.data())
                .map(|(x, y)| x.abs_diff(*y))
                .max()
                .unwrap_or(0)
        };
        for width in [860.0, 1720.0, 3440.0] {
            let scale = width / W;
            let (whole, alone) = drawn(1, scale);
            let (banded, side_by_side) = drawn(BANDS, scale);
            let off = most(&whole.background, &banded.background);
            eprintln!(
                "{width}: {off} levels at most; on one thread {alone:?}, on {BANDS} {side_by_side:?}"
            );
            assert!(off <= 2, "{width}: {off} levels off");
            // The shadows over the faces, part by part and at once.
            let (w, h) = (whole.background.width(), whole.background.height());
            let view = [0.0, whole.top(), f64::from(w) / scale, f64::from(h) / scale];
            let doc = art::document(view, w, h, &art::worn_overlay(&whole.arrangement));
            let tree = usvg::Tree::from_str(&doc, &whole.options).expect("the shadows");
            let mut parts = Pixmap::new(w, h).expect("a frame");
            parts.fill(Color::from_rgba8(0x50, 0x48, 0x40, 0xff));
            let faces = parts.clone();
            let mut at_once = parts.clone();
            render_parts(&tree, &mut parts);
            resvg::render(&tree, Transform::identity(), &mut at_once.as_mut());
            let off = most(&parts, &at_once);
            assert!(off <= 3, "{width}: the shadows {off} levels off");
            assert!(most(&faces, &at_once) > 100, "shadows drawn");
        }
    }
}
