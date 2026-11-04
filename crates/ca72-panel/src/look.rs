//! A look: what the editor's panel is drawn with (decisions.md R-SKIN-PACKS). It is one of the
//! two built in, CLASSIC and REALISTIC, or a skin pack's: a folder of artwork and declarative
//! appearance data in the format every pack uses (`docs/skin-packs.md`; the built-in two are
//! packs too, `packs/official/`). A pack brings no code: the open-source renderer draws the
//! controls, the pack says where they are and what they look like.
//!
//! A pack is untrusted input. It is read and checked by `skin_pack` (its paths, its files'
//! kinds and sizes, its SVG, its fonts, its manifest), then here: its `[look]` table, its
//! layout (`crate::arrangement`), its pictures' slots, and whether its fonts print every legend
//! and fit them on the panel. A pack that fails is not loaded, and the editor draws the
//! built-in look instead; nothing a look does reaches the sound, the presets, automation or
//! what a session saves (the look chosen is the computer's setting, not the session's).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use resvg::usvg;
use serde::Deserialize;
use skin_pack::paths::{self, FileKind};
use skin_pack::signature::{Signed, TrustedKey};
use skin_pack::{Context, Limits, Pack, Report};

use crate::arrangement::Arrangement;
use crate::art::{self, Lettering, TicksTitles};
use crate::fonts::{Fonts, Weight};
use crate::skin::{self, Overrides, Skin};

/// The width, in pixels, a pack's drawing over the panel is costed at: about the panel's at
/// the size the editor opens on a high-density screen (each larger window costs more).
pub const ART_SIDE: u32 = 3000;

/// The plug-in's id in a pack's manifest.
pub const PLUGIN: &str = "ca-72";

/// The plug-in's own artwork's licence.
pub const OFFICIAL_LICENCE: &str = "GPL-3.0-or-later";

/// Idle Foundry's public key, with room for rotation and a future marketplace's keys.
/// The owner fills `skin_pack::keys::IDLE_FOUNDRY_KEY` before the release; unsigned packs
/// remain the main route and need no key. The signer and the editor share this list.
pub use skin_pack::keys::TRUSTED_KEYS;

/// The built-in looks' ids.
pub const CLASSIC: &str = "com.idlefoundry.ca72.classic";
pub const REALISTIC: &str = "com.idlefoundry.ca72.realistic";

/// A look the panel can be drawn in.
#[derive(Clone)]
pub struct Look {
    pub id: String,
    pub name: String,
    pub version: String,
    /// Which of the renderer's two ways of drawing the controls it uses.
    pub base: Skin,
    pub arrangement: Arc<Arrangement>,
    /// Its pictures, where they are not the built-in ones (decoded), and its face picture's
    /// colour (each channel scaled).
    pub pictures: Arc<Overrides>,
    pub face_tint: Option<[f32; 3]>,
    /// Whether its knobs' and pointer knobs' caps turn with them (`knob_caps = "turn"`: a line
    /// across a knob's top, say) or are drawn still, as a spun aluminium cap's sheen should be.
    pub caps_turn: bool,
    /// Its drawing over the panel's surfaces, under its print.
    pub art: Option<Arc<usvg::Tree>>,
    /// Its drawing over the plug-in's strip's surfaces (the rail and the sections under the
    /// panel), under their print and parts.
    pub strip_art: Option<Arc<usvg::Tree>>,
    /// Its drawing over the keyboard view's case, under its key bed, keys and print (in the
    /// strip's units).
    pub keyboard_art: Option<Arc<usvg::Tree>>,
    /// The keyboard view's case in the face's picture rather than the wood's.
    pub keyboard_face: bool,
    /// Its drawing over the presets' drawer's face, under its displays' glass, its keys and
    /// its print (in the drawer's units).
    pub drawer_art: Option<Arc<usvg::Tree>>,
    /// The drawer's displays lettered in its typeface (`drawer_lettering = "type"`) rather
    /// than in dots.
    pub drawer_type: bool,
    /// The drawer's keys' legends printed on their caps (`drawer_legends = "caps"`) rather
    /// than over them.
    pub drawer_caps: bool,
    /// The drawer's sections' titles centred over them, larger (`drawer_titles = "centre"`),
    /// rather than at their left.
    pub drawer_centred: bool,
    /// Its panel's ticks and sections' titles printed quiet (`ticks_and_titles = "quiet"`),
    /// rather than as approved (decisions.md R-QUIET-PRINT).
    pub ticks_titles: TicksTitles,
    /// Its colours for what the strip lights and prints (none: the built-in ones).
    pub colours: Colours,
    /// Its typeface (regular, bold), if it brings one that prints and fits every legend.
    pub fonts: Option<(&'static [u8], &'static [u8])>,
    /// Its licence (an SPDX expression), its creators, and what its checks found.
    pub licence: String,
    pub creators: Vec<String>,
    pub report: Report,
    /// Its signature, checked: who published it, if signed by a trusted key.
    pub signed: Option<Signed>,
}

impl std::fmt::Debug for Look {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Look")
            .field("id", &self.id)
            .field("base", &self.base)
            .field("pictures", &self.pictures.len())
            .field("art", &self.art.is_some())
            .field("fonts", &self.fonts.is_some())
            .finish()
    }
}

impl Look {
    fn built_in(id: &str, name: &str, base: Skin) -> Look {
        Look {
            id: id.into(),
            name: name.into(),
            version: "1.0.0".into(),
            base,
            arrangement: Arrangement::official(),
            pictures: Arc::new(Overrides::new()),
            face_tint: None,
            caps_turn: false,
            art: None,
            strip_art: None,
            keyboard_art: None,
            keyboard_face: false,
            drawer_art: None,
            drawer_type: false,
            drawer_caps: false,
            drawer_centred: false,
            ticks_titles: TicksTitles::Standard,
            colours: Colours::default(),
            fonts: None,
            licence: OFFICIAL_LICENCE.into(),
            creators: vec!["Idle Foundry Ltd.".into()],
            report: Report::default(),
            signed: None,
        }
    }

    /// CLASSIC: the panel as drawn.
    pub fn classic() -> Look {
        Self::built_in(CLASSIC, "CLASSIC", Skin::Drawn)
    }

    /// REALISTIC: the panel in pictures of the original's materials.
    pub fn realistic() -> Look {
        Self::built_in(REALISTIC, "REALISTIC", Skin::Worn)
    }

    /// The built-in look drawn in `skin`.
    pub fn of(skin: Skin) -> Look {
        match skin {
            Skin::Drawn => Self::classic(),
            Skin::Worn => Self::realistic(),
        }
    }

    /// Whether it is one of the plug-in's own.
    pub fn is_built_in(&self) -> bool {
        self.id == CLASSIC || self.id == REALISTIC
    }

    /// The pack at `path` (a folder, or a pack file, `<name>.ifskin`), read and checked: its
    /// look, or what rejected it.
    pub fn open(path: &Path) -> Result<Look, Report> {
        Self::open_with(path, &Limits::default(), TRUSTED_KEYS)
    }

    /// The same, within `limits`, its signature checked against `keys`.
    pub fn open_with(path: &Path, limits: &Limits, keys: &[TrustedKey]) -> Result<Look, Report> {
        let official = skin::official_pictures();
        let context = Context {
            plugin: PLUGIN,
            official: &official,
            official_licence: OFFICIAL_LICENCE,
            built_in: false,
            test_drawings: true,
            keys,
            plugin_version: Some(env!("CARGO_PKG_VERSION")),
        };
        let pack = Pack::open(path, limits, &context)?;
        // The built-in looks' own ids are theirs alone: an installed pack, signed or not, never
        // takes one.
        if pack.manifest.id == CLASSIC || pack.manifest.id == REALISTIC {
            let mut report = pack.report;
            report.reject(
                "manifest.id",
                Some(skin_pack::MANIFEST),
                "the id of a look built into the plug-in",
            );
            return Err(report);
        }
        Self::from_pack(pack, limits)
    }

    /// A checked pack's look.
    pub fn from_pack(pack: Pack, limits: &Limits) -> Result<Look, Report> {
        let mut report = pack.report.clone();
        let m = &pack.manifest;
        let at = Some(skin_pack::MANIFEST);
        let Some(table) = m.look.clone() else {
            report.reject("look.missing", at, "no `[look]` table: what the pack draws");
            return Err(report);
        };
        let spec: LookTable = match table.try_into() {
            Ok(t) => t,
            Err(e) => {
                report.reject("look.fields", at, e.to_string());
                return Err(report);
            }
        };
        let unknown: Vec<String> = spec
            .rest
            .keys()
            .map(|k| format!("look.{k}"))
            .chain(
                spec.fonts
                    .iter()
                    .flat_map(|f| f.rest.keys().map(|k| format!("look.fonts.{k}"))),
            )
            .collect();
        if !unknown.is_empty() {
            report.note(
                "look.unknown",
                at,
                format!(
                    "passed over, not read by this CA-72: {}",
                    unknown
                        .iter()
                        .map(|k| format!("`{k}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            );
        }
        let base = match spec.base.as_str() {
            "classic" => Skin::Drawn,
            "realistic" => Skin::Worn,
            b => {
                report.reject(
                    "look.base",
                    at,
                    format!("`{b}`: a look is drawn as `classic` or `realistic`"),
                );
                return Err(report);
            }
        };
        // A file `[look]` names, of a kind it may be: every file the renderer uses has passed
        // the check for its kind (a font named `.txt` would have skipped the font's).
        let file = |path: &str, kinds: &[FileKind], report: &mut Report| -> Option<Vec<u8>> {
            if !paths::kind(path).is_some_and(|k| kinds.contains(&k)) {
                report.reject(
                    "look.file",
                    Some(path),
                    format!("named in `[look]` as one of {kinds:?}, which it is not"),
                );
                return None;
            }
            let bytes = pack.files.get(path).map(<[u8]>::to_vec);
            if bytes.is_none() {
                report.reject(
                    "look.file",
                    Some(path),
                    "named in `[look]`, not in the pack",
                );
            }
            bytes
        };

        // The layout: the pack's, or the official one.
        let arrangement = match &spec.layout {
            None => Arrangement::official(),
            Some(path) => {
                let text = path
                    .ends_with(".toml")
                    .then(|| pack.text(path).map(str::to_owned))
                    .flatten();
                match text {
                    None => {
                        report.reject(
                            "look.file",
                            Some(path),
                            "the layout is not a text file in the pack",
                        );
                        return Err(report);
                    }
                    Some(t) => match Arrangement::read_at(&t, path, &mut report) {
                        Some(a) => Arc::new(a),
                        None => return Err(report),
                    },
                }
            }
        };

        // The pictures, by slot: PNG as it is, SVG drawn at the built-in picture's size. Each
        // slot decodes its own, so the pixels are counted by slot as well as by file: one large
        // picture named by every slot is that many pictures' pixels.
        let mut pictures = Overrides::new();
        let mut slot_pixels = 0u64;
        for (name, path) in &spec.pictures {
            let Some(&(slot, official)) = skin::SLOTS
                .iter()
                .chain(skin::STRIP_SLOTS.iter())
                .find(|(n, _)| n == name)
            else {
                report.reject(
                    "look.picture",
                    at,
                    format!("`{name}`: no such picture (the slots are in docs/skin-packs.md)"),
                );
                continue;
            };
            let Some(bytes) = file(path, &[FileKind::Png, FileKind::Svg], &mut report) else {
                continue;
            };
            // Kept as the pack holds it (decoded by a renderer as it first needs it); its size
            // read now, to note proportions not the built-in picture's (it is drawn stretched
            // to them).
            let size = skin::slot_size(name).unwrap_or_else(|| skin::png_size(official));
            let (source, own) = if path.ends_with(".svg") {
                // Its cost counted at the size it is drawn: the built-in picture's.
                match skin_pack::svg::parse_at(&bytes, limits, Some(size.0.max(size.1))) {
                    Ok((tree, _)) => {
                        let own = (tree.size().width(), tree.size().height());
                        (Some(skin::Source::Svg(Arc::new(tree))), own)
                    }
                    Err(why) => {
                        report.reject("look.picture", Some(path), why);
                        continue;
                    }
                }
            } else {
                let (w, h) = skin::png_size(&bytes);
                (
                    Some(skin::Source::Png(Arc::new(bytes))),
                    (w as f32, h as f32),
                )
            };
            let Some(source) = source else { continue };
            slot_pixels += match &source {
                skin::Source::Svg(_) => u64::from(size.0) * u64::from(size.1),
                skin::Source::Png(_) => own.0 as u64 * own.1 as u64,
            };
            let (k0, k1) = (
                f64::from(own.0) / f64::from(own.1.max(1.0)),
                f64::from(size.0) / f64::from(size.1),
            );
            if (k0 / k1 - 1.0).abs() > 0.02 {
                report.note(
                    "look.picture",
                    Some(path),
                    format!(
                        "drawn at the built-in {name}'s proportions ({} by {}): it is {} by {}",
                        size.0, size.1, own.0, own.1
                    ),
                );
            }
            pictures.insert(slot, source);
        }
        if slot_pixels > limits.picture_pixels {
            report.reject(
                "look.pixels",
                at,
                format!(
                    "the slots' pictures are {slot_pixels} pixels together, decoded each for its \
                     slot: more than {}",
                    limits.picture_pixels
                ),
            );
        }

        if let Some(t) = spec.face_tint
            && !t.iter().all(|v| (0.0..=2.0).contains(v))
        {
            report.reject("look.face-tint", at, "a face tint of 0 to 2 a channel");
        }

        let caps_turn = match spec.knob_caps.as_deref() {
            None | Some("still") => false,
            Some("turn") => true,
            Some(_) => {
                report.reject("look.knob-caps", at, "knob_caps: \"still\" or \"turn\"");
                false
            }
        };

        // The drawing over the strip, as the panel's.
        let strip_art = match &spec.strip_art {
            None => None,
            Some(path) if path.ends_with(".svg") => {
                let bytes = file(path, &[FileKind::Svg], &mut report);
                match bytes.map(|b| skin_pack::svg::parse_at(&b, limits, Some(ART_SIDE))) {
                    Some(Ok((tree, _))) => Some(Arc::new(tree)),
                    Some(Err(why)) => {
                        report.reject("look.strip-art", Some(path), why);
                        None
                    }
                    None => None,
                }
            }
            Some(path) => {
                report.reject("look.strip-art", Some(path), "the strip's drawing is SVG");
                None
            }
        };

        // The drawing over the keyboard view's case.
        let keyboard_art = match &spec.keyboard_art {
            None => None,
            Some(path) if path.ends_with(".svg") => {
                let bytes = file(path, &[FileKind::Svg], &mut report);
                match bytes.map(|b| skin_pack::svg::parse_at(&b, limits, Some(ART_SIDE))) {
                    Some(Ok((tree, _))) => Some(Arc::new(tree)),
                    Some(Err(why)) => {
                        report.reject("look.keyboard-art", Some(path), why);
                        None
                    }
                    None => None,
                }
            }
            Some(path) => {
                report.reject(
                    "look.keyboard-art",
                    Some(path),
                    "the keyboard's drawing is SVG",
                );
                None
            }
        };

        let keyboard_face = match spec.keyboard_case.as_deref() {
            None | Some("wood") => false,
            Some("face") => true,
            Some(_) => {
                report.reject(
                    "look.keyboard-case",
                    at,
                    "keyboard_case: \"wood\" or \"face\"",
                );
                false
            }
        };

        // The drawing over the presets' drawer, and how its displays and keys are lettered.
        let drawer_art = match &spec.drawer_art {
            None => None,
            Some(path) if path.ends_with(".svg") => {
                let bytes = file(path, &[FileKind::Svg], &mut report);
                match bytes.map(|b| skin_pack::svg::parse_at(&b, limits, Some(ART_SIDE))) {
                    Some(Ok((tree, _))) => Some(Arc::new(tree)),
                    Some(Err(why)) => {
                        report.reject("look.drawer-art", Some(path), why);
                        None
                    }
                    None => None,
                }
            }
            Some(path) => {
                report.reject("look.drawer-art", Some(path), "the drawer's drawing is SVG");
                None
            }
        };
        let drawer_type = match spec.drawer_lettering.as_deref() {
            None | Some("dots") => false,
            Some("type") => true,
            Some(_) => {
                report.reject(
                    "look.drawer-lettering",
                    at,
                    "drawer_lettering: \"dots\" or \"type\"",
                );
                false
            }
        };
        let drawer_caps = match spec.drawer_legends.as_deref() {
            None | Some("above") => false,
            Some("caps") => true,
            Some(_) => {
                report.reject(
                    "look.drawer-legends",
                    at,
                    "drawer_legends: \"above\" or \"caps\"",
                );
                false
            }
        };
        let drawer_centred = match spec.drawer_titles.as_deref() {
            None | Some("left") => false,
            Some("centre") => true,
            Some(_) => {
                report.reject(
                    "look.drawer-titles",
                    at,
                    "drawer_titles: \"left\" or \"centre\"",
                );
                false
            }
        };

        let ticks_titles = match spec.ticks_and_titles.as_deref() {
            None | Some("standard") => TicksTitles::Standard,
            Some("quiet") => TicksTitles::Quiet,
            Some(_) => {
                report.reject(
                    "look.ticks-and-titles",
                    at,
                    "ticks_and_titles: \"standard\" or \"quiet\"",
                );
                TicksTitles::Standard
            }
        };

        // The strip's colours.
        let mut colours = Colours::default();
        if let Some(c) = &spec.colours {
            for (name, value, slot) in [
                ("lamp", &c.lamp, &mut colours.lamp),
                ("drops", &c.drops, &mut colours.drops),
                ("tab_print", &c.tab_print, &mut colours.tab_print),
                ("key_print", &c.key_print, &mut colours.key_print),
                ("wheel", &c.wheel, &mut colours.wheel),
                ("display", &c.display, &mut colours.display),
                ("selection", &c.selection, &mut colours.selection),
                ("velocity", &c.velocity, &mut colours.velocity),
            ] {
                if let Some(v) = value {
                    match hex(v) {
                        Some(rgb) => *slot = Some(rgb),
                        None => report.reject(
                            "look.colour",
                            at,
                            format!("colours.{name}: a colour as #rrggbb"),
                        ),
                    }
                }
            }
            rest_of("look.colours.", &c.rest, &mut report);
        }

        // The drawing over the panel.
        let art = match &spec.art {
            None => None,
            Some(path) if path.ends_with(".svg") => {
                let bytes = file(path, &[FileKind::Svg], &mut report);
                // Its cost counted drawn as the panel is in a large window.
                match bytes.map(|b| skin_pack::svg::parse_at(&b, limits, Some(ART_SIDE))) {
                    Some(Ok((tree, _))) => Some(Arc::new(tree)),
                    Some(Err(why)) => {
                        report.reject("look.art", Some(path), why);
                        None
                    }
                    None => None,
                }
            }
            Some(path) => {
                report.reject("look.art", Some(path), "the panel's drawing is SVG");
                None
            }
        };

        // The typeface: it must print every legend and keep them on the panel, or the
        // built-in one is used (and the pack goes to review).
        let fonts = match &spec.fonts {
            None => None,
            Some(f) => {
                let regular = file(&f.regular, &[FileKind::Font], &mut report);
                let bold = match &f.bold {
                    Some(b) => file(b, &[FileKind::Font], &mut report),
                    None => regular.clone(),
                };
                match (regular, bold) {
                    (Some(r), Some(b)) => {
                        let pair = (keep(r), keep(b));
                        fitting(pair, &arrangement, &mut report).then_some(pair)
                    }
                    _ => None,
                }
            }
        };
        // The legends that run off the panel in the typeface used (the pack's, or the built-in
        // one): drawn cut off, and put to review (the layout's to move, not the typeface's).
        let used = fonts
            .and_then(|(r, b)| Fonts::with(r, b).ok())
            .unwrap_or_default();
        let off = run_off(&used, &arrangement);
        if !off.is_empty() {
            report.review(
                "look.fit",
                None,
                format!(
                    "in its typeface, {} would run off the panel",
                    off.iter()
                        .take(8)
                        .map(|t| format!("\"{t}\""))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            );
        }
        // Without a typeface of its own (or with one not used), the built-in one prints the
        // legends: what it cannot print would show as empty boxes.
        if fonts.is_none() {
            let missing = unprintable(&Fonts::new(), &arrangement);
            if !missing.is_empty() {
                report.review(
                    "look.glyphs",
                    None,
                    format!(
                        "the built-in typeface has no glyph for some of {}: they would print as \
                         empty boxes (bring a typeface that has them)",
                        missing
                            .iter()
                            .take(8)
                            .map(|t| format!("\"{t}\""))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                );
            }
        }
        if report.rejected() {
            return Err(report);
        }
        Ok(Look {
            id: m.id.clone(),
            name: m.name.clone(),
            version: m.version.clone(),
            base,
            arrangement,
            pictures: Arc::new(pictures),
            face_tint: spec.face_tint,
            caps_turn,
            art,
            strip_art,
            keyboard_art,
            keyboard_face,
            drawer_art,
            drawer_type,
            drawer_caps,
            drawer_centred,
            ticks_titles,
            colours,
            fonts,
            licence: m.license.clone(),
            creators: m.creators.iter().map(|c| c.name.clone()).collect(),
            report,
            signed: pack.signed.clone(),
        })
    }

    /// Published by Idle Foundry (signed as its own).
    pub fn first_party(&self) -> bool {
        self.signed
            .as_ref()
            .is_some_and(|s| s.publisher == skin_pack::signature::Publisher::FirstParty)
    }

    /// The settings menu's name and trust. Every name gets a status, so a pack's own name
    /// cannot impersonate a checked publisher by ending in the same words.
    pub fn menu_name(&self) -> String {
        let label = if self.first_party() {
            "IDLE FOUNDRY"
        } else if self.signed.is_some() {
            "MARKETPLACE"
        } else if self.report.has("signature.unknown-key") {
            "UNKNOWN SIGNING KEY"
        } else {
            "UNSIGNED"
        };
        format!("{} ({label})", self.name)
    }
}

/// What a `[look]` holds that this build does not read: noted, not refused (a field a later
/// CA-72 reads is passed over by an earlier one).
type Rest = BTreeMap<String, toml::Value>;

#[derive(Deserialize)]
struct LookTable {
    base: String,
    #[serde(default)]
    layout: Option<String>,
    #[serde(default)]
    art: Option<String>,
    #[serde(default)]
    fonts: Option<FontsTable>,
    #[serde(default)]
    pictures: BTreeMap<String, String>,
    #[serde(default)]
    face_tint: Option<[f32; 3]>,
    #[serde(default)]
    knob_caps: Option<String>,
    #[serde(default)]
    strip_art: Option<String>,
    #[serde(default)]
    keyboard_art: Option<String>,
    #[serde(default)]
    keyboard_case: Option<String>,
    #[serde(default)]
    drawer_art: Option<String>,
    #[serde(default)]
    drawer_lettering: Option<String>,
    #[serde(default)]
    drawer_legends: Option<String>,
    #[serde(default)]
    drawer_titles: Option<String>,
    #[serde(default)]
    ticks_and_titles: Option<String>,
    #[serde(default)]
    colours: Option<ColoursTable>,
    #[serde(flatten)]
    rest: Rest,
}

#[derive(Deserialize)]
struct ColoursTable {
    #[serde(default)]
    lamp: Option<String>,
    #[serde(default)]
    drops: Option<String>,
    #[serde(default)]
    tab_print: Option<String>,
    #[serde(default)]
    key_print: Option<String>,
    #[serde(default)]
    wheel: Option<String>,
    #[serde(default)]
    display: Option<String>,
    #[serde(default)]
    selection: Option<String>,
    #[serde(default)]
    velocity: Option<String>,
    #[serde(flatten)]
    rest: Rest,
}

/// A look's colours for the strip (decisions.md R-SKIN-PACKS): its lamps' (the readouts' lit
/// segments, the displays' dots, the voices' drops), the tabs' print and the rail's keys'.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Colours {
    pub lamp: Option<(u8, u8, u8)>,
    /// The voices' drops' light (else made from `lamp`).
    pub drops: Option<(u8, u8, u8)>,
    pub tab_print: Option<(u8, u8, u8)>,
    pub key_print: Option<(u8, u8, u8)>,
    /// The PITCH and MOD wheels' (else the built-in cream).
    pub wheel: Option<(u8, u8, u8)>,
    /// The drawer's displays' lettering that is not lit, in its typeface (else the tabs'
    /// print's, else a cream).
    pub display: Option<(u8, u8, u8)>,
    /// The drawer's current row's band, lettered in its typeface (else faintly the lamps').
    pub selection: Option<(u8, u8, u8)>,
    /// The left hand's VELOCITY meter's lamps and its cords (else the look's `lamp`, else the built-in
    /// `strip::velocity::RGB`; decisions.md R-VELOCITY).
    pub velocity: Option<(u8, u8, u8)>,
}

/// A colour written `#rrggbb`.
fn hex(v: &str) -> Option<(u8, u8, u8)> {
    let h = v.strip_prefix('#')?;
    if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let b = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some((b(0)?, b(2)?, b(4)?))
}

/// A table's fields this build does not read, noted.
fn rest_of(table: &str, rest: &Rest, report: &mut Report) {
    if !rest.is_empty() {
        report.note(
            "look.unknown",
            Some(skin_pack::MANIFEST),
            format!(
                "passed over, not read by this CA-72: {}",
                rest.keys()
                    .map(|k| format!("`{table}{k}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        );
    }
}

#[derive(Deserialize)]
struct FontsTable {
    regular: String,
    #[serde(default)]
    bold: Option<String>,
    #[serde(flatten)]
    rest: Rest,
}

/// A pack's font's bytes kept for as long as the plug-in runs (its faces borrow them), each
/// distinct font once however often a pack is chosen.
fn keep(bytes: Vec<u8>) -> &'static [u8] {
    static KEPT: OnceLock<Mutex<Vec<&'static [u8]>>> = OnceLock::new();
    let kept = KEPT.get_or_init(|| Mutex::new(Vec::new()));
    let mut kept = match kept.lock() {
        Ok(k) => k,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(k) = kept.iter().find(|k| **k == bytes.as_slice()) {
        return k;
    }
    let leaked: &'static [u8] = Box::leak(bytes.into_boxed_slice());
    kept.push(leaked);
    leaked
}

/// What the plug-in itself prints in a look's typeface, besides a layout's legends (its dials'
/// numbers, its selectors' legends, its plate, tips, notes and menus): printable ASCII, the en
/// dash and the ellipsis. A pack's typeface must have them all.
pub const PLUGIN_PRINT: &str = concat!(
    " !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`",
    "abcdefghijklmnopqrstuvwxyz{|}~\u{2013}\u{2026}"
);

/// The legends and titles of `arrangement` that `fonts` cannot print in full.
fn unprintable<'a>(fonts: &Fonts, arrangement: &'a Arrangement) -> Vec<&'a str> {
    arrangement
        .texts
        .iter()
        .map(|t| t.text.as_str())
        .chain(arrangement.titles.iter().map(|t| t.text.as_str()))
        .filter(|t| !fonts.covers(t))
        .collect()
}

/// The legends of `arrangement` that run off the panel in `fonts`, in either lettering.
fn run_off(fonts: &Fonts, arrangement: &Arrangement) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for lettering in [Lettering::Standard, Lettering::Large] {
        for t in &arrangement.texts {
            let size = t.size_in(lettering);
            let (x, y) = t.at_in(lettering);
            let half = fonts.advance(&t.text, size, Weight::Bold, 0.0) * art::STRETCH / 2.0;
            let c = arrangement.panel.canvas();
            let inside =
                x - half >= c[0] && x + half <= c[2] && y - size >= c[1] && y + size <= c[3];
            if !inside && !out.contains(&t.text) {
                out.push(t.text.clone());
            }
        }
    }
    out
}

/// Whether a pack's typeface prints every legend of `arrangement` and what the plug-in prints
/// (reported if not: the built-in typeface is then used).
fn fitting(
    (regular, bold): (&'static [u8], &'static [u8]),
    arrangement: &Arrangement,
    report: &mut Report,
) -> bool {
    let fonts = match Fonts::with(regular, bold) {
        Ok(f) => f,
        Err(why) => {
            report.review(
                "look.fonts",
                None,
                format!("{why}: the built-in typeface is used"),
            );
            return false;
        }
    };
    let texts = arrangement
        .texts
        .iter()
        .map(|t| t.text.as_str())
        .chain(arrangement.titles.iter().map(|t| t.text.as_str()))
        .chain([art::PLATE_NAME, PLUGIN_PRINT]);
    for t in texts {
        if !fonts.covers(t) {
            report.review(
                "look.fonts",
                None,
                format!(
                    "its typeface has no glyph for some of \"{t}\": the built-in typeface is used"
                ),
            );
            return false;
        }
    }
    true
}

/// Every pack in the folder at `dir` (a folder or a pack file each), read and checked against
/// [`TRUSTED_KEYS`]: its look, or its name there and what rejected it. Something that is not a
/// pack is passed over (no link is followed). Of the same pack (its id) found more than once,
/// one is kept ([`supersedes`]): a pack's update, added beside it, is the one used.
///
/// A pack read before and unchanged since (its files' paths, sizes and times the same) is not
/// checked again: the settings' menu asks each time it opens, and a pack's checks (every
/// picture decoded, every drawing timed) take long enough to stall the editor.
pub fn discover(dir: &Path) -> Vec<Result<Look, (String, Report)>> {
    discover_in(dir, TRUSTED_KEYS, true)
}

pub fn discover_with(dir: &Path, keys: &[TrustedKey]) -> Vec<Result<Look, (String, Report)>> {
    discover_in(dir, keys, false)
}

/// What each pack read was found to be, by its path and its files' stamp ([`stamp`]).
type Checked = Vec<(std::path::PathBuf, u64, Result<Look, Report>)>;
static CHECKED: Mutex<Checked> = Mutex::new(Vec::new());

/// A pack's (folder or file) files as they are now: each one's path, size and modification
/// time, hashed; none if it cannot be read or holds more than a pack may (it is then checked,
/// and refused, every time).
fn stamp(path: &Path) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let mut todo = vec![path.to_path_buf()];
    let mut seen = 0usize;
    while let Some(p) = todo.pop() {
        let m = std::fs::symlink_metadata(&p).ok()?;
        seen += 1;
        if seen > Limits::default().files * 2 + 1 {
            return None;
        }
        (p.as_os_str(), m.len(), m.modified().ok(), m.is_dir()).hash(&mut h);
        if m.is_dir() {
            let mut names: Vec<_> = std::fs::read_dir(&p)
                .ok()?
                .filter_map(Result::ok)
                .map(|e| e.path())
                .collect();
            names.sort();
            todo.extend(names);
        }
    }
    Some(h.finish())
}

/// `path`'s pack read and checked, or (`cached`) as found before if its files are unchanged.
fn open_cached(path: &Path, keys: &[TrustedKey], cached: bool) -> Result<Look, Report> {
    let stamp = cached.then(|| stamp(path)).flatten();
    let lock = || CHECKED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(st) = stamp
        && let Some((_, _, found)) = lock().iter().find(|(p, s, _)| p == path && *s == st)
    {
        return found.clone();
    }
    let found = Look::open_with(path, &Limits::default(), keys);
    if let Some(st) = stamp {
        let mut checked = lock();
        checked.retain(|(p, _, _)| p != path && p.exists());
        checked.push((path.to_path_buf(), st, found.clone()));
    }
    found
}

fn discover_in(
    dir: &Path,
    keys: &[TrustedKey],
    cached: bool,
) -> Vec<Result<Look, (String, Report)>> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<_> = entries
        .filter_map(Result::ok)
        .filter(|e| match e.file_type() {
            Ok(t) if t.is_dir() => e.path().join(skin_pack::MANIFEST).is_file(),
            Ok(t) if t.is_file() => skin_pack::archive::is_pack_file(&e.path()),
            _ => false,
        })
        .collect();
    found.sort_by_key(std::fs::DirEntry::file_name);
    let mut out: Vec<Result<Look, (String, Report)>> = Vec::new();
    for e in found.into_iter().take(64) {
        match open_cached(&e.path(), keys, cached) {
            Ok(look) => {
                let same = out
                    .iter()
                    .position(|o| o.as_ref().is_ok_and(|l| l.id == look.id));
                match same {
                    Some(i) => {
                        let replaces = out[i].as_ref().is_ok_and(|old| supersedes(&look, old));
                        if replaces {
                            out[i] = Ok(look);
                        }
                    }
                    None => out.push(Ok(look)),
                }
            }
            Err(r) => out.push(Err((e.file_name().to_string_lossy().into_owned(), r))),
        }
    }
    out
}

/// Whether `a` is used rather than `b`, the same pack: a signed one over an unsigned one (an
/// unsigned copy, whatever its version, never takes a published pack's place), then the newer.
fn supersedes(a: &Look, b: &Look) -> bool {
    match (a.signed.is_some(), b.signed.is_some()) {
        (true, false) => true,
        (false, true) => false,
        _ => newer(&a.version, &b.version),
    }
}

/// Whether version `a` (`major.minor.patch`, as a manifest's) is newer than `b`.
fn newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> { v.split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    parts(a) > parts(b)
}
