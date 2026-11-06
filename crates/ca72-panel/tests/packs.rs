//! Skin packs (decisions.md R-SKIN-PACKS): the official looks are packs in the same format as
//! anyone's and draw as the built-in looks do; a pack's layout moves what is drawn and what a
//! click finds; a pack that is refused, or whose typeface cannot print its legends, falls back.

use std::path::Path;

use ca72_panel::look::Look;
use ca72_panel::{CONTROLS, Lettering, Renderer, Scene, Skin, Target, interact};
use skin_pack::{Context, Files, Limits, Pack, Report, Severity};

#[path = "../packs/official/files.rs"]
mod files;

fn official_context<'a>(pictures: &'a [(&'static str, &'static [u8])]) -> Context<'a> {
    Context {
        plugin: "ca-72",
        official: pictures,
        official_licence: "GPL-3.0-or-later",
        built_in: true,
        test_drawings: true,
        keys: &[],
        plugin_version: None,
    }
}

/// The official pack `key`, checked as any pack is.
fn official(key: &str) -> Pack {
    let files = files::files(key).expect("an official pack");
    let mut report = Report::default();
    let files = Files::from_memory(
        files.iter().map(|(p, b)| (p.as_str(), b.as_slice())),
        &Limits::default(),
        &mut report,
    );
    let pictures = ca72_panel::skin::official_pictures();
    Pack::check(
        files,
        report,
        &Limits::default(),
        &official_context(&pictures),
    )
    .unwrap_or_else(|r| panic!("{key}:\n{r}"))
}

#[test]
fn the_official_looks_are_packs_in_the_one_format() {
    for key in ["classic", "realistic"] {
        let pack = official(key);
        // Its only reviews: that its pictures were generated with a tool (whether such pictures
        // are protected differs between countries).
        for f in &pack.report.findings {
            if f.severity == Severity::Review {
                assert_eq!(f.code, "provenance.generated", "{key}: {f}");
            }
        }
        let notes = pack
            .report
            .findings
            .iter()
            .filter(|f| f.code == "asset.official")
            .count();
        assert_eq!(notes, if key == "realistic" { 27 } else { 0 }, "{key}");
        assert_eq!(pack.manifest.license, "GPL-3.0-or-later");
    }
}

fn frame(look: Look, scale: f64) -> Vec<u8> {
    let mut r = Renderer::with_pack(look, Lettering::Standard, scale, 1.0);
    let mut scene = Scene {
        values: [0.5; CONTROLS.len()],
        ..Scene::default()
    };
    scene.values[3] = 0.8;
    r.render(&scene);
    r.frame().data().to_vec()
}

/// Read back as packs, the official looks draw exactly as the built-in ones.
#[test]
fn the_official_packs_draw_as_the_built_in_looks() {
    for (key, skin) in [("classic", Skin::Drawn), ("realistic", Skin::Worn)] {
        let look = Look::from_pack(official(key), &Limits::default())
            .unwrap_or_else(|r| panic!("{key}:\n{r}"));
        assert_eq!(look.base, skin);
        assert!(
            frame(look, 0.3) == frame(Look::of(skin), 0.3),
            "{key} drawn differently"
        );
    }
}

fn starter() -> &'static Path {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/skin-packs/starter"
    ))
}

/// The CC0 starter: a pack of its own layout and drawing, every control found where it is drawn.
#[test]
fn the_starter_pack_moves_the_controls_and_what_a_click_finds() {
    let look = Look::open(starter()).unwrap_or_else(|r| panic!("{r}"));
    assert_eq!(look.licence, "CC0-1.0");
    assert!(look.art.is_some());
    assert!(look.report.has("declaration.missing"), "{}", look.report);
    let a = look.arrangement.clone();
    let mut r = Renderer::with_pack(look, Lettering::Standard, 0.3, 1.0);
    r.render(&Scene::default());
    let official = ca72_panel::Arrangement::official();
    let mut moved = 0;
    for (i, c) in a.controls.iter().enumerate() {
        let (x, y) = c.centre();
        // (The drawers' knobs, the plug-in's own, are found while their drawer is open.)
        let found = if c.place.is_drawer() {
            interact::hit_drawer(c.place, x, y)
        } else if c.place == ca72_panel::controls::Place::Lfo {
            // (The LFO page's, found while it shows: decisions.md R-LFO.)
            interact::hit_lfo(r.layout(), x, y)
        } else if c.place == ca72_panel::controls::Place::Macro {
            // (The macros', found while they show: decisions.md R-MACROS.)
            interact::hit_macro(x, y)
        } else {
            interact::hit(r.layout(), &a, x, y)
        };
        assert_eq!(found, Some(Target::Control(i)), "{}", c.param);
        moved += usize::from(c.centre() != official.controls[i].centre());
    }
    assert!(moved >= 40, "{moved} moved");
    let (px, py) = a.power_at();
    assert_eq!(interact::hit(r.layout(), &a, px, py), Some(Target::Power));
}

fn copy(from: &Path, to: &Path) {
    for e in std::fs::read_dir(from).expect("a folder") {
        let e = e.expect("an entry");
        let target = to.join(e.file_name());
        if e.file_type().expect("a type").is_dir() {
            std::fs::create_dir_all(&target).expect("a folder");
            copy(&e.path(), &target);
        } else {
            std::fs::copy(e.path(), target).expect("copied");
        }
    }
}

/// The starter copied, its `edit` made to its files.
fn edited(edit: impl FnOnce(&Path)) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a folder");
    copy(starter(), dir.path());
    edit(dir.path());
    dir
}

fn replace(path: &Path, from: &str, to: &str) {
    let text = std::fs::read_to_string(path).expect("read");
    assert!(text.contains(from), "{from}");
    std::fs::write(path, text.replacen(from, to, 1)).expect("written");
}

#[test]
fn a_pack_whose_layout_is_refused_is_not_loaded() {
    let dir = edited(|d| replace(&d.join("layout.toml"), "at = [333, 120]", "at = [130, 130]"));
    let r = Look::open(dir.path()).expect_err("refused");
    assert!(r.has("layout.overlap"), "{r}");
    let dir = edited(|d| {
        replace(
            &d.join("skin.toml"),
            "base = \"classic\"",
            "base = \"neon\"",
        )
    });
    assert!(
        Look::open(dir.path())
            .expect_err("refused")
            .has("look.base")
    );
    let dir = edited(|d| {
        std::fs::write(
            d.join("art/panel.svg"),
            "<svg xmlns='http://www.w3.org/2000/svg'><script>x()</script></svg>",
        )
        .expect("written");
    });
    assert!(
        Look::open(dir.path())
            .expect_err("refused")
            .has("svg.refused")
    );
}

#[test]
fn a_packs_pictures_replace_the_built_in_ones_by_slot() {
    let dir = edited(|d| {
        std::fs::write(
            d.join("art/knob.svg"),
            "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 10 10'><circle cx='5' cy='5' r='5' fill='#c33'/></svg>",
        )
        .expect("written");
        replace(
            &d.join("skin.toml"),
            "base = \"classic\"",
            "base = \"realistic\"",
        );
        let text = std::fs::read_to_string(d.join("skin.toml")).expect("read");
        std::fs::write(
            d.join("skin.toml"),
            text.replace(
                "[look]",
                "[[assets]]\npath = \"art/knob.svg\"\norigin = \"original\"\n\n[look]",
            ) + "\n[look.pictures]\nknob = \"art/knob.svg\"\n",
        )
        .expect("written");
    });
    let look = Look::open(dir.path()).unwrap_or_else(|r| panic!("{r}"));
    assert_eq!(look.pictures.len(), 1);
    // Kept as the drawing it is; drawn at the built-in knob's size as a renderer needs it.
    let knob = look.pictures["knob"].picture((512, 512)).expect("drawn");
    assert_eq!((knob.width(), knob.height()), (512, 512));
    assert!(matches!(
        look.pictures["knob"],
        ca72_panel::skin::Source::Svg(_)
    ));
    let base = Look::realistic();
    assert!(frame(look, 0.3) != frame(base, 0.3));
    // A slot that is not one of the plug-in's is refused.
    let dir = edited(|d| {
        let text = std::fs::read_to_string(d.join("skin.toml")).expect("read");
        std::fs::write(
            d.join("skin.toml"),
            text + "\n[look.pictures]\nlogo = \"art/panel.svg\"\n",
        )
        .expect("written");
    });
    assert!(
        Look::open(dir.path())
            .expect_err("refused")
            .has("look.picture")
    );
}

/// A typeface that cannot print a legend is not used: the built-in one is, and the pack goes
/// to review; one that can is used.
#[test]
fn a_packs_typeface_is_used_only_if_it_prints_every_legend() {
    let font = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../third_party/urw-gothic/URWGothic-Demi.otf"
    ))
    .expect("the built-in font");
    let with_font = |legend: &'static str| {
        let font = font.clone();
        edited(move |d| {
            std::fs::create_dir_all(d.join("fonts")).expect("a folder");
            std::fs::write(d.join("fonts/Gothic-Demi.otf"), &font).expect("written");
            let text = std::fs::read_to_string(d.join("skin.toml")).expect("read");
            std::fs::write(
                d.join("skin.toml"),
                text.replace(
                    "[look]",
                    "[[assets]]\npath = \"fonts/Gothic-Demi.otf\"\norigin = \"third-party\"\nsource = \"URW base 35\"\nlicense = \"AGPL-3.0-only WITH Font-exception-2.0\"\n\n[look]",
                ) + "fonts = { regular = \"fonts/Gothic-Demi.otf\" }\n",
            )
            .expect("written");
            replace(&d.join("layout.toml"), "text = \"TUNE\"", legend);
        })
    };
    let dir = with_font("text = \"TUNE\"");
    let look = Look::open(dir.path()).unwrap_or_else(|r| panic!("{r}"));
    assert!(look.fonts.is_some(), "{}", look.report);
    let dir = with_font("text = \"TUNE 調律\"");
    let look = Look::open(dir.path()).unwrap_or_else(|r| panic!("{r}"));
    assert!(look.fonts.is_none());
    assert!(look.report.has("look.fonts"), "{}", look.report);
    // A font named as another kind of file is refused: it would have skipped the font's checks.
    let dir = edited(|d| {
        std::fs::write(d.join("notes.txt"), "plain text, not a font").expect("written");
        let text = std::fs::read_to_string(d.join("skin.toml")).expect("read");
        std::fs::write(
            d.join("skin.toml"),
            text.replace(
                "[look]",
                "[[assets]]\npath = \"notes.txt\"\norigin = \"original\"\n\n[look]",
            ) + "fonts = { regular = \"notes.txt\" }\n",
        )
        .expect("written");
    });
    assert!(
        Look::open(dir.path())
            .expect_err("refused")
            .has("look.file")
    );
    // Drawn all the same, in the built-in typeface.
    let _ = frame(look, 0.3);
}

// ---- Pack files, signatures and updates (the marketplace's: decisions.md R-SKIN-PACKS).

use skin_pack::signature::{self, Publisher, TrustedKey};

/// A key for the tests (from fixed bytes: never one the plug-in trusts).
fn test_key() -> (String, TrustedKey) {
    let secret = "07".repeat(32);
    let key = signature::signing_key(&secret).expect("a key");
    let public = signature::public_key(&signature::public_hex(&key)).expect("its public half");
    (
        secret,
        TrustedKey {
            id: "test-1",
            public,
            first_party: true,
        },
    )
}

/// The starter's files, its manifest's id and version changed, read from a folder.
fn starter_files(id: &str, version: &str) -> Files {
    let dir = edited(|d| {
        replace(&d.join("skin.toml"), "org.example.ca72-starter", id);
        replace(
            &d.join("skin.toml"),
            "version = \"1.0.0\"",
            &format!("version = \"{version}\""),
        );
    });
    let mut r = Report::default();
    Files::read_dir(dir.path(), &Limits::default(), &mut r)
}

/// `files` signed with the test key, as a pack file in `dir`, named `name`.
fn signed_pack_file(
    dir: &Path,
    name: &str,
    files: &Files,
    (id, version): (&str, &str),
    publisher: Publisher,
) -> std::path::PathBuf {
    let (secret, _) = test_key();
    let key = signature::signing_key(&secret).expect("a key");
    let sig = signature::sign(
        files,
        (id, version),
        ("test-1", &key),
        publisher,
        "2026-10-16",
    );
    let mut r = Report::default();
    let with_sig = Files::from_memory(
        files.iter().chain([(signature::SIGNATURE, sig.as_bytes())]),
        &Limits::default(),
        &mut r,
    );
    let path = dir.join(name);
    std::fs::write(
        &path,
        skin_pack::archive::write(&with_sig).expect("written"),
    )
    .expect("saved");
    path
}

#[test]
fn a_signed_pack_file_loads_as_published_and_unsigned_without_its_key() {
    let out = tempfile::tempdir().expect("a folder");
    let files = starter_files("org.example.night", "1.0.0");
    let path = signed_pack_file(
        out.path(),
        "night.ifskin",
        &files,
        ("org.example.night", "1.0.0"),
        Publisher::Marketplace,
    );
    let (_, key) = test_key();
    let look = Look::open_with(&path, &Limits::default(), &[key]).unwrap_or_else(|r| panic!("{r}"));
    assert_eq!(
        look.signed.as_ref().map(|s| s.publisher),
        Some(Publisher::Marketplace)
    );
    assert!(!look.first_party());
    assert_eq!(look.menu_name(), format!("{} (MARKETPLACE)", look.name));
    // Without the key (an older plug-in, a newer key): as if unsigned, and it loads.
    let look = Look::open(&path).unwrap_or_else(|r| panic!("{r}"));
    assert!(look.signed.is_none());
    assert!(look.report.has("signature.unknown-key"), "{}", look.report);
    assert_eq!(
        look.menu_name(),
        format!("{} (UNKNOWN SIGNING KEY)", look.name)
    );
}

#[test]
fn an_unsigned_packs_menu_name_cannot_claim_a_trusted_publisher() {
    let mut look = Look::open(starter()).unwrap_or_else(|r| panic!("{r}"));
    assert!(look.signed.is_none());
    assert!(look.report.has("signature.none"));
    assert_eq!(look.menu_name(), format!("{} (UNSIGNED)", look.name));
    look.name = "NIGHT (IDLE FOUNDRY)".into();
    assert_eq!(look.menu_name(), "NIGHT (IDLE FOUNDRY) (UNSIGNED)");
}

#[test]
fn a_pack_changed_after_it_was_signed_is_refused() {
    let out = tempfile::tempdir().expect("a folder");
    let files = starter_files("org.example.night", "1.0.0");
    let path = signed_pack_file(
        out.path(),
        "night.ifskin",
        &files,
        ("org.example.night", "1.0.0"),
        Publisher::Marketplace,
    );
    // Unpacked, one legend changed, packed again.
    let mut r = Report::default();
    let read = skin_pack::archive::read_file(&path, &Limits::default(), &mut r);
    let layout = String::from_utf8(read.get("layout.toml").expect("its layout").to_vec())
        .expect("text")
        .replacen("\"TUNE\"", "\"TUNED\"", 1);
    let changed = Files::from_memory(
        read.iter().map(|(p, b)| {
            if p == "layout.toml" {
                (p, layout.as_bytes())
            } else {
                (p, b)
            }
        }),
        &Limits::default(),
        &mut r,
    );
    std::fs::write(&path, skin_pack::archive::write(&changed).expect("written")).expect("saved");
    let (_, key) = test_key();
    let report = Look::open_with(&path, &Limits::default(), &[key]).expect_err("refused");
    assert!(report.has("signature.invalid"), "{report}");
}

/// The makers' id prefix: refused unsigned, or signed as a seller's; allowed signed as theirs.
#[test]
fn only_a_first_party_signature_allows_the_makers_ids() {
    let out = tempfile::tempdir().expect("a folder");
    let id = "com.idlefoundry.ca72.dusk";
    let files = starter_files(id, "1.0.0");
    let (_, key) = test_key();
    let first = signed_pack_file(
        out.path(),
        "a.ifskin",
        &files,
        (id, "1.0.0"),
        Publisher::FirstParty,
    );
    let look =
        Look::open_with(&first, &Limits::default(), &[key]).unwrap_or_else(|r| panic!("{r}"));
    assert!(look.first_party());
    assert_eq!(look.menu_name(), format!("{} (IDLE FOUNDRY)", look.name));
    let seller = signed_pack_file(
        out.path(),
        "b.ifskin",
        &files,
        (id, "1.0.0"),
        Publisher::Marketplace,
    );
    let r = Look::open_with(&seller, &Limits::default(), &[key]).expect_err("refused");
    assert!(r.has("manifest.id"), "{r}");
    let r = Look::open_with(&first, &Limits::default(), &[]).expect_err("refused unsigned");
    assert!(r.has("manifest.id"), "{r}");
    // And never the built-in looks' own ids, signed or not.
    let builtin = ca72_panel::look::REALISTIC;
    let files = starter_files(builtin, "1.0.0");
    let p = signed_pack_file(
        out.path(),
        "c.ifskin",
        &files,
        (builtin, "1.0.0"),
        Publisher::FirstParty,
    );
    assert!(Look::open_with(&p, &Limits::default(), &[key]).is_err());
}

/// An update added beside a pack is the one used; an unsigned copy never replaces a published
/// pack, whatever its version.
#[test]
fn a_packs_update_replaces_it_and_an_unsigned_copy_does_not() {
    let skins = tempfile::tempdir().expect("a folder");
    let (_, key) = test_key();
    let id = "org.example.night";
    signed_pack_file(
        skins.path(),
        "night-1.0.0.ifskin",
        &starter_files(id, "1.0.0"),
        (id, "1.0.0"),
        Publisher::Marketplace,
    );
    signed_pack_file(
        skins.path(),
        "night-1.1.0.ifskin",
        &starter_files(id, "1.1.0"),
        (id, "1.1.0"),
        Publisher::Marketplace,
    );
    let found = ca72_panel::look::discover_with(skins.path(), &[key]);
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].as_ref().map(|l| l.version.as_str()).ok(),
        Some("1.1.0")
    );
    // An unsigned folder of the same id, "version 9": the published 1.1.0 stays.
    let copy = skins.path().join("night-copy");
    std::fs::create_dir_all(&copy).expect("a folder");
    copy_dir_files(&starter_files(id, "9.0.0"), &copy);
    let found = ca72_panel::look::discover_with(skins.path(), &[key]);
    assert_eq!(found.len(), 1);
    let kept = found[0].as_ref().expect("a look");
    assert_eq!(
        (kept.version.as_str(), kept.signed.is_some()),
        ("1.1.0", true)
    );
}

fn copy_dir_files(files: &Files, to: &Path) {
    for (p, b) in files.iter() {
        let path = to.join(p);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("a folder");
        std::fs::write(path, b).expect("written");
    }
}

/// A `[look]` field this CA-72 does not read (a later one's) is noted, not refused.
#[test]
fn a_look_field_from_a_later_ca72_is_passed_over() {
    let dir = edited(|d| {
        let text = std::fs::read_to_string(d.join("skin.toml")).expect("read");
        std::fs::write(d.join("skin.toml"), text + "glow = 0.5\n").expect("written");
    });
    let look = Look::open(dir.path()).unwrap_or_else(|r| panic!("{r}"));
    assert!(look.report.has("look.unknown"), "{}", look.report);
}

/// `knob_caps = "turn"`: a knob's cap turns with it, so that a mark across its middle turns as
/// a whole; without it (or "still") the cap is drawn still, as a spun cap's sheen should be.
#[test]
fn a_packs_knob_caps_turn_with_them_when_it_says_so() {
    let with = |caps: Option<&str>| {
        edited(|d| {
            // A white knob with a black bar from its middle to twelve o'clock.
            std::fs::write(
                d.join("art/knob.svg"),
                "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 10 10'><circle cx='5' cy='5' r='4.9' fill='#eee'/><rect x='4.5' y='0.2' width='1' height='4.8' fill='#000'/></svg>",
            )
            .expect("written");
            replace(
                &d.join("skin.toml"),
                "base = \"classic\"",
                "base = \"realistic\"",
            );
            let text = std::fs::read_to_string(d.join("skin.toml")).expect("read");
            let caps = caps.map_or(String::new(), |c| format!("knob_caps = \"{c}\"\n"));
            std::fs::write(
                d.join("skin.toml"),
                text.replace(
                    "[look]",
                    "[[assets]]\npath = \"art/knob.svg\"\norigin = \"original\"\n\n[look]",
                ) + &caps
                    + "\n[look.pictures]\nknob = \"art/knob.svg\"\n",
            )
            .expect("written");
        })
    };
    let open = |caps| {
        let dir = with(caps);
        Look::open(dir.path()).unwrap_or_else(|r| panic!("{r}"))
    };
    let (still, said_still, turn) = (open(None), open(Some("still")), open(Some("turn")));
    assert!(!still.caps_turn && !said_still.caps_turn && turn.caps_turn);
    assert!(!turn.report.has("look.unknown"), "{}", turn.report);
    // The bar's middle drawn still at twelve o'clock, or turned with the knob.
    assert!(frame(still, 0.3) != frame(turn, 0.3));
    let dir = with(Some("spin"));
    assert!(
        Look::open(dir.path())
            .expect_err("refused")
            .has("look.knob-caps")
    );
}

/// A layout's `[panel]`: its face's height and its wooden strip's. Its places are moved so
/// that the face's foot stays where the plug-in's strip begins; the frame grows upward; a
/// control is drawn and found where the layout put it; the strip under it is drawn as under the
/// official panel; and sizes outside the limits are refused.
#[test]
fn a_pack_chooses_its_panels_size() {
    use ca72_panel::art::{self, Panel};
    use ca72_panel::strip::{StripRenderer, StripScene};
    let sized = |panel: &str| {
        let panel = panel.to_owned();
        edited(move |d| {
            let layout = d.join("layout.toml");
            let text = std::fs::read_to_string(&layout).expect("read");
            std::fs::write(
                &layout,
                text.replacen("[fixtures]", &format!("{panel}\n[fixtures]"), 1),
            )
            .expect("written");
        })
    };
    let dir = sized("[panel]\nface = 1000\n");
    let tall = Look::open(dir.path()).unwrap_or_else(|r| panic!("{r}"));
    let p = tall.arrangement.panel;
    assert_eq!(
        p,
        Panel {
            wood: art::TOP,
            face: 1000.0
        }
    );
    assert_eq!(p.top(), art::PANEL_H - 1130.0);
    // The starter's CUTOFF, 600 down its face: 400 above the face's foot, as on any panel.
    let cutoff = ca72_panel::controls::index("cutoff").expect("CUTOFF");
    let c = tall.arrangement.controls[cutoff];
    assert_eq!((c.x, c.y), (111.0, 600.0 - 205.0));
    let (cx, cy) = c.centre();
    assert_eq!(cy, art::PANEL_H - 400.0);
    let s = 0.25;
    let mut r = Renderer::with_pack(tall.clone(), Lettering::Standard, s, 1.0);
    assert_eq!(r.size(), ca72_panel::render::size_for(s, p));
    assert_eq!(
        r.size().1,
        ((art::H * s).ceil() as i32 - (p.top() * s).round() as i32) as u32
    );
    let mut scene = Scene::default();
    r.render(&scene);
    scene.values[cutoff] = 0.9;
    assert!(r.render(&scene));
    let d = r.damage().expect("CUTOFF drawn again");
    let row = ((cy - p.top()) * s).round() as i32;
    assert!(d[1] < row && row < d[3], "{d:?} {row}");
    assert_eq!(
        ca72_panel::interact::hit(r.layout(), r.arrangement(), cx, cy),
        Some(ca72_panel::Target::Control(cutoff))
    );
    assert_eq!(
        r.strip_row(),
        (art::PANEL_H * s).round() as i32 - r.top_px()
    );
    // The strip under it as under the same pack at the official size.
    let official = Look::open(starter()).unwrap_or_else(|r| panic!("{r}"));
    let mut under = Renderer::with_pack(official, Lettering::Standard, s, 1.0);
    let mut r = Renderer::with_pack(tall, Lettering::Standard, s, 1.0);
    under.render(&Scene::default());
    r.render(&Scene::default());
    let (mut a, mut b) = (StripRenderer::new(s), StripRenderer::new(s));
    b.set_under_top(r.top_px());
    a.render(&StripScene::default(), under.frame(), None);
    b.render(&StripScene::default(), r.frame(), None);
    assert!(a.frame().data() == b.frame().data());
    // No wood; and sizes outside the limits refused.
    let dir = sized("[panel]\nface = 900\nwood = 0\n");
    let bare = Look::open(dir.path()).expect_err("its plate is on wood it no longer has");
    assert!(bare.has("layout.outside"), "{bare}");
    for panel in [
        "[panel]\nface = 600\n",
        "[panel]\nface = 900\nwood = 30\n",
        "[panel]\nface = 1300\n",
    ] {
        let dir = sized(panel);
        assert!(
            Look::open(dir.path())
                .expect_err("refused")
                .has("layout.panel"),
            "{panel}"
        );
    }
}

/// The strip in a look (decisions.md R-SKIN-PACKS): given an official look it is drawn exactly
/// as before; a pack's tabs, keys, glass, lamp colour and print colours change it, and its
/// drawing of the strip changes the panel renderer's strip, not its panel.
#[test]
fn a_packs_strip_is_its_own() {
    use ca72_panel::strip::{StripRenderer, StripScene};
    let s = 0.2;
    let scene = StripScene::default();
    let draw = |look: &Look, set: bool| {
        let mut r = Renderer::with_pack(look.clone(), Lettering::Standard, s, 1.0);
        r.render(&Scene::default());
        let mut strip = StripRenderer::new(s);
        if set {
            strip.set_look(look);
        }
        strip.render(&scene, r.frame(), None);
        (
            r.frame().data().to_vec(),
            strip.frame().data().to_vec(),
            r.strip_row(),
        )
    };
    for look in [Look::realistic(), Look::classic()] {
        assert!(draw(&look, false).1 == draw(&look, true).1, "{}", look.id);
    }
    // A pack's strip: a red tab, a green key, the lamps blue, and a drawing over the strip.
    let dir = edited(|d| {
        let red = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 10 10'><rect width='10' height='10' rx='3' fill='#a22'/></svg>";
        let green = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 10 10'><rect width='10' height='10' rx='3' fill='#2a2'/></svg>";
        let bands = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 3108 850'><rect x='300' y='200' width='2500' height='500' fill='#123'/></svg>";
        std::fs::write(d.join("art/tab.svg"), red).expect("written");
        std::fs::write(d.join("art/key.svg"), green).expect("written");
        std::fs::write(d.join("art/strip.svg"), bands).expect("written");
        replace(
            &d.join("skin.toml"),
            "base = \"classic\"",
            "base = \"realistic\"",
        );
        let text = std::fs::read_to_string(d.join("skin.toml")).expect("read");
        let assets = ["art/tab.svg", "art/key.svg", "art/strip.svg"]
            .map(|p| format!("[[assets]]\npath = \"{p}\"\norigin = \"original\"\n\n"))
            .concat();
        std::fs::write(
            d.join("skin.toml"),
            text.replace("[look]", &format!("{assets}[look]"))
                + "strip_art = \"art/strip.svg\"\n\n[look.colours]\nlamp = \"#3080ff\"\ntab_print = \"#ffffff\"\nkey_print = \"#ffffff\"\n\n[look.pictures]\ntab = \"art/tab.svg\"\nkey = \"art/key.svg\"\n",
        )
        .expect("written");
    });
    let pack = Look::open(dir.path()).unwrap_or_else(|r| panic!("{r}"));
    assert!(pack.strip_art.is_some());
    assert_eq!(pack.colours.lamp, Some((0x30, 0x80, 0xff)));
    let base = Look::realistic();
    let (bp, bs, row) = draw(&base, true);
    let (pp, ps, prow) = draw(&pack, true);
    assert_eq!(row, prow);
    assert!(bs != ps, "the strip's parts drawn in the pack's");
    // The panel renderer: its strip changed by the drawing, its panel above the strip as the
    // starter's drawing leaves it (compared with the same pack without its strip drawing).
    let w = (ca72_panel::W * s).ceil() as usize * 4;
    let row = row as usize;
    assert!(bp[row * w..] != pp[row * w..]);
    let bad = edited(|d| {
        let text = std::fs::read_to_string(d.join("skin.toml")).expect("read");
        std::fs::write(
            d.join("skin.toml"),
            text + "\n[look.colours]\nlamp = \"orange\"\n",
        )
        .expect("written");
    });
    assert!(
        Look::open(bad.path())
            .expect_err("refused")
            .has("look.colour")
    );
}

/// A pack's rockers in two states: its picture on drawn when on and its plain one when off,
/// neither mirrored and no paddle lit by the plug-in; without its picture on, mirrored as
/// before. And a pack's wheel colour shades the PITCH and MOD wheels.
#[test]
fn a_packs_switches_show_their_state_and_its_wheels_their_colour() {
    let with = |on: bool, wheel: bool| {
        edited(move |d| {
            let off = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 20 8'><rect width='20' height='8' fill='#111'/><rect x='15' y='2' width='4' height='4' fill='#420'/></svg>";
            let lit = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 20 8'><rect width='20' height='8' fill='#111'/><rect x='15' y='2' width='4' height='4' fill='#fa4'/></svg>";
            std::fs::write(d.join("art/off.svg"), off).expect("written");
            std::fs::write(d.join("art/on.svg"), lit).expect("written");
            replace(
                &d.join("skin.toml"),
                "base = \"classic\"",
                "base = \"realistic\"",
            );
            let text = std::fs::read_to_string(d.join("skin.toml")).expect("read");
            let assets = ["art/off.svg", "art/on.svg"]
                .map(|p| format!("[[assets]]\npath = \"{p}\"\norigin = \"original\"\n\n"))
                .concat();
            let colours = if wheel {
                "\n[look.colours]\nwheel = \"#806040\"\n"
            } else {
                ""
            };
            let pictures = if on {
                "\n[look.pictures]\nrocker-blue = \"art/off.svg\"\nrocker-blue-on = \"art/on.svg\"\n"
            } else {
                "\n[look.pictures]\nrocker-blue = \"art/off.svg\"\n"
            };
            std::fs::write(
                d.join("skin.toml"),
                text.replace("[look]", &format!("{assets}[look]")) + colours + pictures,
            )
            .expect("written");
        })
    };
    let s = 0.3;
    let draw = |look: Look, value: f64| {
        let mut r = Renderer::with_pack(look, Lettering::Standard, s, 1.0);
        let mut scene = Scene::default();
        let i = ca72_panel::controls::index("osc1_on").expect("OSC 1 ON");
        scene.values[i] = value;
        r.render(&scene);
        r.frame().data().to_vec()
    };
    let open = |on, wheel| {
        let dir = with(on, wheel);
        Look::open(dir.path()).unwrap_or_else(|r| panic!("{r}"))
    };
    let (two, one) = (open(true, false), open(false, false));
    // On, the two-state pack differs from the mirrored one; off, they are the same picture
    // unmirrored against mirrored, so they differ too, but the two-state pack's off and on
    // differ only by the lamp.
    assert!(draw(two.clone(), 1.0) != draw(one.clone(), 1.0));
    assert!(draw(two.clone(), 0.0) != draw(two.clone(), 1.0));
    let wheel = open(false, true);
    assert_eq!(wheel.colours.wheel, Some((0x80, 0x60, 0x40)));
    assert!(draw(wheel, 0.0) != draw(one, 0.0));
}

/// A pack's `velocity` colour dresses the left hand's VELOCITY lamps; without it
/// they are drawn as the official looks draw them.
#[test]
fn a_packs_velocity_colour_dresses_the_meter() {
    let with = |colour: Option<&str>| {
        edited(move |d| {
            if let Some(c) = colour {
                let text = std::fs::read_to_string(d.join("skin.toml")).expect("read");
                std::fs::write(
                    d.join("skin.toml"),
                    text + &format!("\n[look.colours]\nvelocity = \"{c}\"\n"),
                )
                .expect("written");
            }
        })
    };
    let draw = |look: Look, lit: usize| {
        let mut r = Renderer::with_pack(look, Lettering::Standard, 0.3, 1.0);
        r.render(&Scene {
            velocity_lit: lit,
            ..Scene::default()
        });
        r.frame().data().to_vec()
    };
    let open = |colour| {
        let dir = with(colour);
        Look::open(dir.path()).unwrap_or_else(|r| panic!("{r}"))
    };
    let (plain, blue) = (open(None), open(Some("#40c0ff")));
    assert_eq!(plain.colours.velocity, None);
    assert_eq!(blue.colours.velocity, Some((0x40, 0xc0, 0xff)));
    assert!(!blue.report.has("look.unknown"), "{}", blue.report);
    // The built-in colour's the default: given it, the pack draws as without it.
    let orange = open(Some("#ff7028"));
    assert!(draw(orange, 6) == draw(plain.clone(), 6));
    // The pack's colour draws the lamps, lit or not, in it.
    assert!(draw(blue.clone(), 6) != draw(plain.clone(), 6));
    assert!(draw(blue, 0) != draw(plain, 0));
    let bad = with(Some("pink"));
    assert!(
        Look::open(bad.path())
            .expect_err("refused")
            .has("look.colour")
    );
}

/// The keyboard view in a look: given an official one it is drawn exactly as before; a pack's
/// keys and its drawing over the case change it.
#[test]
fn a_packs_keyboard_is_its_own() {
    use ca72_panel::strip::{StripRenderer, StripScene};
    let s = 0.2;
    let scene = StripScene {
        keyboard: Some(ca72_panel::keyboard::Scene {
            skin: Skin::Worn,
            ..Default::default()
        }),
        ..StripScene::default()
    };
    let draw = |look: &Look, set: bool| {
        let mut r = Renderer::with_pack(look.clone(), Lettering::Standard, s, 1.0);
        r.render(&Scene::default());
        let mut strip = StripRenderer::new(s);
        if set {
            strip.set_look(look);
        }
        strip.render(&scene, r.frame(), None);
        strip.frame().data().to_vec()
    };
    let realistic = Look::realistic();
    assert!(draw(&realistic, false) == draw(&realistic, true));
    let dir = edited(|d| {
        let ivory = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 10 80'><rect width='10' height='80' fill='#ec8'/></svg>";
        let case = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 3108 850'><rect x='370' y='116' width='2710' height='728' fill='#123'/></svg>";
        std::fs::write(d.join("art/white.svg"), ivory).expect("written");
        std::fs::write(d.join("art/case.svg"), case).expect("written");
        replace(
            &d.join("skin.toml"),
            "base = \"classic\"",
            "base = \"realistic\"",
        );
        let text = std::fs::read_to_string(d.join("skin.toml")).expect("read");
        let assets = ["art/white.svg", "art/case.svg"]
            .map(|p| format!("[[assets]]\npath = \"{p}\"\norigin = \"original\"\n\n"))
            .concat();
        std::fs::write(
            d.join("skin.toml"),
            text.replace("[look]", &format!("{assets}[look]"))
                + "keyboard_art = \"art/case.svg\"\n\n[look.pictures]\nwhite-key = \"art/white.svg\"\n",
        )
        .expect("written");
    });
    let pack = Look::open(dir.path()).unwrap_or_else(|r| panic!("{r}"));
    assert!(pack.keyboard_art.is_some());
    assert!(draw(&pack, true) != draw(&realistic, true));
}

/// A pack dresses the presets' drawer (decisions.md R-SKIN-PACKS): the official looks draw the
/// built-in drawer; a pack's drawing over its face, its displays lettered in its typeface, its
/// keys' legends on their caps and its titles centred draw another, a press in a field placing
/// the caret where that typeface sets the characters; a value it does not know is refused.
#[test]
fn a_packs_drawer_is_its_own() {
    use ca72_panel::presets::{
        DrawerRenderer, DrawerScene, DrawerTarget, Field, FieldId, caret_at,
    };
    let scene = DrawerScene {
        search: Field {
            text: "Brass Tutti".into(),
            caret: 0,
        },
        focus: Some(FieldId::Search),
        ..DrawerScene::default()
    };
    let draw = |look: Option<&Look>| {
        let mut d = DrawerRenderer::new(0.25);
        if let Some(l) = look {
            d.set_look(l);
        }
        d.render(&scene);
        d.frame().data().to_vec()
    };
    let built_in = draw(None);
    assert!(draw(Some(&Look::realistic())) == built_in);
    assert!(draw(Some(&Look::of(Skin::Drawn))) == built_in);

    let pack_with = |fields: &str| {
        edited(|d| {
            let art = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 3108 734'><rect x='30' y='10' width='750' height='714' fill='#123'/></svg>";
            std::fs::write(d.join("art/drawer.svg"), art).expect("written");
            let text = std::fs::read_to_string(d.join("skin.toml")).expect("read");
            std::fs::write(
                d.join("skin.toml"),
                text.replace(
                    "[look]",
                    "[[assets]]\npath = \"art/drawer.svg\"\norigin = \"original\"\n\n[look]",
                ) + fields,
            )
            .expect("written");
        })
    };
    let dir = pack_with(
        "drawer_art = \"art/drawer.svg\"\ndrawer_lettering = \"type\"\ndrawer_legends = \"caps\"\ndrawer_titles = \"centre\"\n\n[look.colours]\ndisplay = \"#e6dcc4\"\n",
    );
    let pack = Look::open(dir.path()).unwrap_or_else(|r| panic!("{r}"));
    assert!(pack.drawer_art.is_some() && pack.drawer_type);
    assert!(pack.drawer_caps && pack.drawer_centred);
    assert!(draw(Some(&pack)) != built_in);
    // Its panel's ticks and titles quiet (decisions.md R-QUIET-PRINT); as approved unless it
    // says so.
    assert_eq!(pack.ticks_titles, ca72_panel::TicksTitles::Standard);
    let quiet = Look::open(pack_with("ticks_and_titles = \"quiet\"\n").path())
        .unwrap_or_else(|r| panic!("{r}"));
    assert_eq!(quiet.ticks_titles, ca72_panel::TicksTitles::Quiet);

    // Along the search field, the first place a press puts the caret after its fifth
    // character: set in the typeface, nearer than five of the dots' characters.
    let first_at = |d: &DrawerRenderer, k: usize| {
        (630..7500)
            .map(|x| f64::from(x) / 10.0)
            .find(|&x| {
                let t = DrawerTarget::Field(FieldId::Search);
                caret_at(&scene, FieldId::Search, d.press_x(&scene, t, x)) >= k
            })
            .expect("a place")
    };
    let dots = DrawerRenderer::new(0.25);
    let mut typed = DrawerRenderer::new(0.25);
    typed.set_look(&pack);
    assert!(first_at(&typed, 5) < first_at(&dots, 5) - 20.0);
    let ends = |d: &DrawerRenderer| {
        let t = DrawerTarget::Field(FieldId::Search);
        (
            caret_at(&scene, FieldId::Search, d.press_x(&scene, t, 0.0)),
            caret_at(&scene, FieldId::Search, d.press_x(&scene, t, 3000.0)),
        )
    };
    assert_eq!(ends(&typed), (0, 11));

    for (field, code) in [
        ("drawer_lettering = \"neon\"\n", "look.drawer-lettering"),
        ("drawer_legends = \"under\"\n", "look.drawer-legends"),
        ("drawer_titles = \"right\"\n", "look.drawer-titles"),
        ("ticks_and_titles = \"faint\"\n", "look.ticks-and-titles"),
        ("drawer_art = \"art/drawer.png\"\n", "look.drawer-art"),
    ] {
        let dir = pack_with(field);
        let r = Look::open(dir.path()).expect_err("refused");
        assert!(r.has(code), "{code}: {r}");
    }
}

/// A pack's pictures are counted by slot as well as by file (each slot decodes its own): one
/// large picture named by many slots is that many pictures' pixels, and refused past the limit.
#[test]
fn a_picture_named_by_many_slots_is_counted_for_each() {
    let slots = [
        "knob",
        "knob-big",
        "pointer",
        "face",
        "plate",
        "walnut",
        "screw",
        "jack",
        "jewel",
        "power",
        "rocker-blue",
        "rocker-orange",
        "tab",
        "tab-lit",
        "key",
        "glass",
        "rocker-ivory",
        "white-key",
        "black-key",
    ];
    let pack_with = |n: usize| {
        edited(|d| {
            let big = ca72_panel::Pixmap::new(1000, 1000).expect("a picture");
            std::fs::write(d.join("art/big.png"), big.encode_png().expect("encoded"))
                .expect("written");
            replace(
                &d.join("skin.toml"),
                "base = \"classic\"",
                "base = \"realistic\"",
            );
            let text = std::fs::read_to_string(d.join("skin.toml")).expect("read");
            let named: String = slots[..n]
                .iter()
                .map(|s| format!("{s} = \"art/big.png\"\n"))
                .collect();
            std::fs::write(
                d.join("skin.toml"),
                text.replace(
                    "[look]",
                    "[[assets]]\npath = \"art/big.png\"\norigin = \"original\"\n\n[look]",
                ) + "\n[look.pictures]\n"
                    + &named,
            )
            .expect("written");
        })
    };
    // One million pixels: in one slot, within the limit; in all of them, past it.
    let one = pack_with(1);
    Look::open(one.path()).unwrap_or_else(|r| panic!("{r}"));
    let all = pack_with(slots.len());
    let r = Look::open(all.path()).expect_err("refused");
    assert!(r.has("look.pixels"), "{r}");
}

/// The packs installed are read once while unchanged (the settings' menu asks each time it
/// opens): one edited is read again, its change shown.
#[test]
fn an_installed_pack_is_read_again_only_when_it_changes() {
    let dir = tempfile::tempdir().expect("a folder");
    let pack = dir.path().join("starter");
    std::fs::create_dir_all(&pack).expect("a folder");
    copy(starter(), &pack);
    let names = || {
        ca72_panel::look::discover(dir.path())
            .into_iter()
            .map(|f| {
                f.map(|l| l.name)
                    .unwrap_or_else(|(n, r)| panic!("{n}: {r}"))
            })
            .collect::<Vec<_>>()
    };
    let first = names();
    assert_eq!(first.len(), 1);
    assert_eq!(names(), first);
    let manifest = pack.join("skin.toml");
    let text = std::fs::read_to_string(&manifest).expect("read");
    let name_line = text
        .lines()
        .find(|l| l.starts_with("name = "))
        .expect("a name")
        .to_owned();
    std::fs::write(
        &manifest,
        text.replace(&name_line, "name = \"STARTER, EDITED\""),
    )
    .expect("written");
    assert_eq!(names(), vec!["STARTER, EDITED".to_owned()]);
}

/// The pack made by hand whose check once aborted the process (its drawing's pattern's tile
/// asked for some 17 TB; an allocation that fails is not a panic) in a Skins folder beside the
/// starter: in a child process, so that a regression fails this test, not the run, the plug-in
/// reads the folder, refuses that pack and loads the starter.
#[test]
fn a_pack_whose_check_once_aborted_is_refused_and_the_next_loads() {
    let vast = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/vast-pattern"
    ));
    let skins = tempfile::tempdir().expect("a folder");
    for (from, name) in [(vast, "a-vast"), (starter(), "b-starter")] {
        let to = skins.path().join(name);
        std::fs::create_dir_all(&to).expect("a folder");
        copy(from, &to);
    }
    let out = std::process::Command::new(std::env::current_exe().expect("this test"))
        .args([
            "--exact",
            "the_skins_folder_named_by_the_parent_is_read",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_SKINS, skins.path())
        .output()
        .expect("a child process");
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "the Skins folder's reading ended {}:\n{said}\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(said.contains("refused a-vast"), "{said}");
    assert!(said.contains("a pattern's tile"), "{said}");
    assert!(said.contains("loaded STARTER"), "{said}");
}

/// The Skins folder a parent test names, read as the plug-in reads it, in this process.
const CHILD_SKINS: &str = "CA72_PANEL_TEST_CHILD_SKINS";

#[test]
fn the_skins_folder_named_by_the_parent_is_read() {
    let Some(dir) = std::env::var_os(CHILD_SKINS) else {
        return;
    };
    for found in ca72_panel::look::discover_with(Path::new(&dir), &[]) {
        match found {
            Ok(look) => println!("loaded {}", look.name),
            Err((name, report)) => println!("refused {name}:\n{report}"),
        }
    }
}

/// LONG's sign follows a pack's GLIDE rather than the official layout's, in its own typeface
/// and in either base skin. Turning LONG off restores the pack's frame to the pixel.
#[test]
fn long_glides_print_follows_a_skin_packs_dial() {
    for base in ["classic", "realistic"] {
        let dir = edited(|d| {
            if base == "realistic" {
                replace(
                    &d.join("skin.toml"),
                    "base = \"classic\"",
                    "base = \"realistic\"",
                );
            }
        });
        let look = Look::open(dir.path()).expect("the starter pack");
        for lettering in [Lettering::Standard, Lettering::Large] {
            let mut r = Renderer::with_pack(look.clone(), lettering, 0.37, 1.0);
            let mut scene = Scene::default();
            r.render(&scene);
            let before = r.frame().clone();
            scene.glide_long = true;
            r.render(&scene);
            assert!(r.frame() != &before, "{base}: no sign");
            let c = r
                .arrangement()
                .controls
                .iter()
                .find(|c| c.param == "glide")
                .expect("GLIDE");
            let (x, y) = c.centre();
            let e = ca72_panel::art::GLIDE_LONG_EXTENT;
            let mut changed = 0;
            for py in 0..before.height() {
                for px in 0..before.width() {
                    let i = (py as usize * before.width() as usize + px as usize) * 4;
                    if before.data()[i..i + 4] != r.frame().data()[i..i + 4] {
                        let (px, py) = (
                            f64::from(px) / 0.37,
                            f64::from(py) / 0.37 + r.arrangement().panel.top(),
                        );
                        assert!(
                            px >= x + e[0] - 3.0
                                && px <= x + e[2] + 3.0
                                && py >= y + e[1] - 3.0
                                && py <= y + e[3] + 3.0,
                            "{base}: print away from GLIDE"
                        );
                        changed += 1;
                    }
                }
            }
            assert!(changed > 10, "{base}: sign too small");
            scene.glide_long = false;
            r.render(&scene);
            assert!(
                r.frame() == &before,
                "{base}: NORMAL did not restore the frame"
            );
        }
    }
}
