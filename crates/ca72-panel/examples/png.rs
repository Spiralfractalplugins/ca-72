//! Renders the panel to a PNG: `cargo run -p ca72-panel --example png -- out.png [scale]`
//! (pixels a panel unit, 0.5 by default); `CA72_SKIN=worn` in the worn skin; `CA72_MONO=1` the
//! strip in MONO without DOUBLE (the one voice lit at one of VOICES's places); `CA72_LETTERING=
//! large` in the large lettering; `CA72_QUIET=1` its ticks and titles quiet (decisions.md
//! R-QUIET-PRINT).
//! `CA72_STANDALONE=1` shows the standalone's TEMPO screen with its TYPING, PANIC and MIDI keys
//! after it (`CA72_TEMPO=96` another tempo; `CA72_TYPING=1` TYPING on, `CA72_PANIC=1` PANIC held,
//! `CA72_MIDI=1` the MIDI map mode's key lit, `CA72_TEMPO_HOVER=typing`, `panic` or `midi` a key
//! under the pointer: decisions.md R-TEMPO-CORNER, R-MIDI-MAP).
//! `CA72_ARP=1` drops the arpeggiator's drawer over the lower page (decisions.md R-ARP): ON and
//! UP lit, a sixteenth, two octaves, GATE 60 %, SWING 25 %, the ARP key lit; `CA72_ARP_HOLD=1`
//! HOLD lit too, `CA72_ARP_DIRECTION=4` another DIRECTION (ORDER), `CA72_ARP_OCTAVES=5` other
//! OCTAVES.
//! `CA72_FX=1` the standalone's rail (its FX key, lit) and the effects' drawer over the lower
//! page (decisions.md R-FX-DRAWER): three effects, the third bypassed, the fourth slot empty;
//! `CA72_FX=list` with the effects' list dropped for the fourth; `CA72_FX=crashed` the second
//! given up on; `CA72_FX=safety` a slot turned off after a crash, one after a hang, one
//! loading; `CA72_FX=off` a safe start's slots (decisions.md R-FX-SAFETY); `CA72_FX=rail` the
//! rail alone.
//! `CA72_MPE=1` drops the expression drawer over the lower page (decisions.md R-MPE): PRESSURE
//! 50 % and 1.0 OCT, TIMBRE -30 % and 2.5 OCT (the mock-up's), INPUT reading MPE, the MPE key
//! lit; `CA72_MPE_INPUT=channel` or `none` INPUT so.
//! `CA72_LFO=1` the LFO page over the lower page (decisions.md R-LFO), the round-2 mock-ups'
//! four LFOs, the LFO key lit (`CA72_LFO_KEYS=1` LFO 2 UNI and LFO 3 muted; `CA72_LFO_FADE=-1.2`
//! every LFO's FADE at so many seconds, out below 0, in above, off at 0: R-LFO-FADE); `CA72_LFO_COLORS=amber,green,blue,red` other phosphors;
//! `CA72_LFO_RINGS=1` the mock-ups' rings on CUTOFF, OSCILLATOR-2 FREQUENCY and VOLUME (each
//! LFO's point on them), `CA72_LFO_CORD=<param>` an assignment from LFO 1's jack dragged over
//! that knob, `CA72_LFO_POPUP=1` CUTOFF's hover pop-up.
//! `CA72_SET=param=value,...` controls at other values; `CA72_TIMED=2:1:9,3:0:13` oscillator 2's
//! or 3's TIMED on or off and its division (decisions.md R-OSC-TIMED).
//! `CA72_GLIDE_LONG=1` prints LONG's ×5 beside GLIDE (decisions.md R-GLIDE-LONG).
//! `CA72_VELOCITY=0.76` lights the left hand's VELOCITY meter for a note at that velocity
//! (decisions.md R-VELOCITY); `CA72_MPE=1` draws the drawer's VELOCITY row as its mock-up had it.
//! `CA72_PACK=<folder>` in a skin pack's look (decisions.md R-SKIN-PACKS), what its checks
//! found printed.
//! `CA72_TIP=1` adds CUTOFF's hover tip; `CA72_MENU=settings` the built-in standalone
//! settings' menu (a review fixture, without installed packs). `CA72_PRINT_SVG=<file>` writes
//! the drawing's text as SVG, before rasterisation, for measuring its physical heights.

use ca72_panel::{CONTROLS, Renderer, Scene, Skin};

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args.next().unwrap_or_else(|| "panel.png".into());
    let scale: f64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(0.5);
    let t0 = std::time::Instant::now();
    let skin = match std::env::var("CA72_SKIN").as_deref() {
        Ok("worn") => Skin::Worn,
        _ => Skin::Drawn,
    };
    let lettering = match std::env::var("CA72_LETTERING").as_deref() {
        Ok("large") => ca72_panel::Lettering::Large,
        _ => ca72_panel::Lettering::Standard,
    };
    let print = if std::env::var("CA72_QUIET").is_ok_and(|v| v == "1") {
        ca72_panel::TicksTitles::Quiet
    } else {
        ca72_panel::TicksTitles::Standard
    };
    let mut r = match std::env::var("CA72_PACK") {
        Ok(dir) => match ca72_panel::Look::open(std::path::Path::new(&dir)) {
            Ok(look) => {
                print!("{}", look.report);
                Renderer::printing(look, lettering, print, scale, 1.0)
            }
            Err(report) => {
                eprint!("{report}");
                std::process::exit(1);
            }
        },
        Err(_) => Renderer::printing(ca72_panel::Look::of(skin), lettering, print, scale, 1.0),
    };
    let arp = std::env::var("CA72_ARP").is_ok_and(|v| v == "1");
    let fx = std::env::var("CA72_FX").ok();
    let mpe = std::env::var("CA72_MPE").is_ok_and(|v| v == "1");
    let built = t0.elapsed();
    if let Ok(path) = std::env::var("CA72_PRINT_SVG") {
        let fonts = ca72_panel::fonts::Fonts::new();
        let arrangement = r.arrangement();
        let layout = ca72_panel::Layout::measure_in(&fonts, lettering, arrangement);
        let plate = ca72_panel::art::plate();
        let body = format!(
            "{}<g transform='translate({} {})'>{}</g>",
            ca72_panel::art::background(&layout, arrangement),
            plate.origin.0,
            plate.origin.1,
            plate.body
        );
        std::fs::write(
            path,
            ca72_panel::art::document([0.0, 0.0, ca72_panel::W, ca72_panel::H], 3108, 1775, &body),
        )
        .expect("the print's SVG written");
    }
    let mut scene = Scene {
        glide_long: std::env::var("CA72_GLIDE_LONG").is_ok_and(|v| v == "1"),
        // (`CA72_VELOCITY=0.76`: the left hand's VELOCITY meter lit for a note at 0.76:
        // decisions.md R-VELOCITY.)
        velocity_lit: ca72_panel::strip::velocity::lit(
            std::env::var("CA72_VELOCITY")
                .ok()
                .and_then(|v| v.parse().ok()),
        ),
        ..Scene::default()
    };
    // The plugin's defaults: as the mock-up was drawn; the strip's knobs at what its readouts
    // read. `CA72_ALL=0.5`: every control at one value instead.
    let all: Option<f64> = std::env::var("CA72_ALL").ok().and_then(|s| s.parse().ok());
    for (i, c) in CONTROLS.iter().enumerate() {
        scene.values[i] =
            all.unwrap_or_else(|| strip_knob(c.param).unwrap_or_else(|| default_of(c.param)));
    }
    // `CA72_SET=param=value,...`: controls at other normalized values (RANGE's LO is 0).
    if let Ok(set) = std::env::var("CA72_SET") {
        for kv in set.split(',').filter(|kv| !kv.is_empty()) {
            let (k, v) = kv.split_once('=').expect("param=value");
            let i = ca72_panel::controls::index(k).expect("a control");
            scene.values[i] = v.parse().expect("a number");
        }
    }
    // `CA72_TIMED=2:1:9,3:0:13`: oscillator 2's or 3's TIMED on (1) or off (0) and its
    // division (0 to 17, `params::Division`'s order; decisions.md R-OSC-TIMED). Each key is
    // inert where its RANGE is not at LO, and its dial, timed, shows its division.
    if let Ok(timed) = std::env::var("CA72_TIMED") {
        for spec in timed.split(',').filter(|s| !s.is_empty()) {
            let parts: Vec<&str> = spec.split(':').collect();
            let n = usize::from(parts[0] == "3");
            let at = |p: &str| ca72_panel::controls::index(p).expect("a control");
            let key = at(ca72_panel::render::TIMED_KEYS[n]);
            scene.values[key] = if parts[1] == "1" { 1.0 } else { 0.0 };
            let division: usize = parts[2].parse().expect("a division");
            let range = at(if n == 0 { "osc2_range" } else { "osc3_range" });
            scene.timed[n] = ca72_panel::render::Timed {
                division,
                inert: ca72_panel::art::position(scene.values[range]) != 0,
            };
            if scene.timed_dials()[n] {
                scene.values[at(ca72_panel::render::TIMED_DIALS[n])] =
                    division as f64 / ca72_panel::controls::TIMED_STEPS as f64;
            }
        }
    }
    let mono = std::env::var("CA72_MONO").is_ok_and(|v| v == "1");
    if std::env::var("CA72_STANDALONE").is_ok_and(|v| v == "1") {
        let bpm = std::env::var("CA72_TEMPO").unwrap_or_else(|_| "120".into());
        scene.tempo = Some(format!("{bpm} BPM"));
        let on = |name: &str| std::env::var(name).is_ok_and(|v| v == "1");
        scene.tempo_keys = Some(ca72_panel::render::TempoKeys {
            typing: on("CA72_TYPING"),
            panic: on("CA72_PANIC"),
            midi: on("CA72_MIDI"),
            hover: match std::env::var("CA72_TEMPO_HOVER").as_deref() {
                Ok("typing") => Some(ca72_panel::art::TempoKey::Typing),
                Ok("panic") => Some(ca72_panel::art::TempoKey::Panic),
                Ok("midi") => Some(ca72_panel::art::TempoKey::Midi),
                _ => None,
            },
        });
    }
    if std::env::var("CA72_TIP").is_ok_and(|v| v == "1") {
        scene.tip = Some(("CUTOFF FREQUENCY: 1.2 kHz".into(), 2081.0, 300.0));
    }
    // `CA72_CONTINUOUS=1:2.4,3:5.2`: oscillators 1 and 3 CONTINUOUS, their switches 2.4 and 5.2
    // steps from TRIANGLE (R-CONTINUOUS-WAVE); `CA72_MOD=1:a:3.1` oscillator 1's MOD key on, its
    // way a (amber, clockwise), b (blue) or r (red), its orb at 3.1 (or `-`, none).
    for kv in std::env::var("CA72_CONTINUOUS")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
    {
        let (n, at) = kv.split_once(':').expect("oscillator:steps");
        let n: usize = n.parse::<usize>().expect("1 to 3") - 1;
        scene.shape[n].continuous = true;
        scene.shape[n].at = at.parse().expect("steps");
    }
    for kv in std::env::var("CA72_MOD")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
    {
        let mut f = kv.split(':');
        let n: usize = f
            .next()
            .and_then(|n| n.parse::<usize>().ok())
            .expect("1 to 3")
            - 1;
        scene.shape[n].way = match f.next() {
            Some("b") => ca72_panel::art::ModWay::Anticlockwise,
            Some("r") => ca72_panel::art::ModWay::Both,
            _ => ca72_panel::art::ModWay::Clockwise,
        };
        scene.shape[n].orb = f.next().and_then(|o| o.parse().ok());
        let i = ca72_panel::controls::index(ca72_panel::render::MOD_KEYS[n]).expect("there");
        scene.values[i] = 1.0;
    }
    let t1 = std::time::Instant::now();
    r.render(&scene);
    let first = t1.elapsed();
    scene.values[0] = 0.8;
    let t2 = std::time::Instant::now();
    r.render(&scene);
    let one = t2.elapsed();
    scene.values[0] = default_of("tune");
    r.render(&scene);
    eprintln!("background {built:?}, every layer {first:?}, one knob {one:?}");
    // The strip's own parts over the drawing's strip (A6), as the editor shows them.
    let mut strip = ca72_panel::strip::StripRenderer::new(scale);
    let t3 = std::time::Instant::now();
    let scene_values = scene.values.to_vec();
    let mut scene = example_strip(mono);
    if std::env::var("CA72_KEYBOARD").is_ok_and(|v| v == "1") {
        let held = if std::env::var("CA72_KEYS_PLAYED").is_ok_and(|v| v == "1") {
            [(1u64 << 60), (1u64 << 0) | (1u64 << 3)]
        } else {
            [0, 0]
        };
        scene.keyboard = Some(ca72_panel::keyboard::Scene {
            skin,
            held,
            typing: std::env::var("CA72_TYPING").is_ok_and(|v| v == "1"),
            standalone: std::env::var("CA72_STANDALONE").is_ok_and(|v| v == "1"),
            ..Default::default()
        });
        scene.bar.keyboard = true;
    }
    strip.set_look(r.look());
    strip.set_under_top(r.top_px());
    scene.bar.arp = arp;
    scene.bar.mpe = mpe;
    if std::env::var("CA72_LFO").is_ok_and(|v| v == "1") {
        scene.lfo = true;
        scene.bar.lfo = true;
        scene.bar.lfo_page = true;
    }
    if arp {
        scene.bar.page = Some(ca72_panel::presets::BarTarget::Arp);
    } else if mpe {
        scene.bar.page = Some(ca72_panel::presets::BarTarget::Mpe);
    }
    if fx.is_some() {
        strip.set_rail(ca72_panel::strip::Rail::Effects);
        scene.bar.fx = true;
    }
    strip.render(&scene, r.frame(), None);
    eprintln!("the strip {:?}", t3.elapsed());
    let mut all = r.frame().clone();
    let top = r.strip_row() as usize;
    let w = all.width() as usize;
    let sf = strip.frame();
    let rows = (sf.height() as usize).min(all.height() as usize - top);
    for y in 0..rows {
        let n = w.min(sf.width() as usize) * 4;
        let (from, to) = (y * sf.width() as usize * 4, (top + y) * w * 4);
        all.data_mut()[to..to + n].copy_from_slice(&sf.data()[from..from + n]);
    }
    if std::env::var("CA72_LFO").is_ok_and(|v| v == "1") {
        lfo_page(&mut all, &mut r, scale, skin, lettering);
    }
    lfo_overlay(&mut all, &r, scale, &scene_values);
    if arp {
        drop_arp(&mut all, &mut r, scale, skin, lettering);
    }
    if mpe {
        drop_mpe(&mut all, &mut r, scale, skin, lettering);
    }
    if let Some(fx) = fx.filter(|v| v != "rail") {
        drop_fx(&mut all, scale, r.top_px(), skin, lettering, &fx);
    }
    // `CA72_DRAWER=1`: the presets' drawer open over the strip, as the editor shows it; `edit`,
    // a search typed and the preset being renamed; `midi`, MIDI Learn's list.
    let drawer_shown = std::env::var("CA72_DRAWER").unwrap_or_default();
    if ["1", "edit", "midi"].contains(&drawer_shown.as_str()) {
        use ca72_panel::presets::{
            Current, DRAWER_TOP, DrawerRenderer, DrawerScene, DrawerTarget, Edit, Field, FieldId,
            MidiList, MidiRow, Row,
        };
        let mut drawer = DrawerRenderer::new(scale);
        drawer.set_look(r.look());
        let presets = [
            "Brass Tutti",
            "Deep Mod Bass",
            "Glass Lead",
            "Night Pad",
            "Pluck Ladder",
        ];
        let mut scene = DrawerScene {
            chips: ["bass", "brass", "lead", "pad", "pluck"]
                .iter()
                .map(|t| ((*t).to_owned(), *t == "lead"))
                .collect(),
            rows: (0..18)
                .map(|i| Row {
                    name: format!("{} {}", presets[i % presets.len()], i / presets.len() + 1),
                    tags: "lead bright".into(),
                    origin: (i == 3).then_some("EDITED"),
                    favorite: i % 4 == 0,
                    current: i == 0,
                })
                .collect(),
            current: Some(Current {
                name: "Brass Tutti 1".into(),
                origin: None,
                tags: "brass".into(),
                favorite: false,
            }),
            total: 18,
            ..DrawerScene::default()
        };
        if drawer_shown == "edit" {
            scene.search = Field {
                text: "bra".into(),
                caret: 3,
            };
            scene.editing = Some(Edit::Rename);
            scene.edit = Field {
                text: "Brass Tutti 1".into(),
                caret: 5,
            };
            scene.focus = Some(FieldId::Edit);
            scene.hover = Some(DrawerTarget::Row(2));
        }
        if drawer_shown == "midi" {
            let row = |name: &str, assignment: &str| MidiRow {
                name: name.into(),
                assignment: assignment.into(),
                waiting: false,
            };
            scene.midi = Some(MidiList {
                rows: vec![
                    row("TUNE", "CC 21"),
                    row("GLIDE", ""),
                    row("CUTOFF", "CC 74"),
                    row("EMPHASIS", "CC 71"),
                ],
                chosen: 1,
                keys: "ENTER: LEARN \u{b7} DELETE: REMOVE".into(),
                reserved: "CC 0, 32, 120 TO 127 ARE NOT LEARNED".into(),
                status: "CHOOSE A CONTROL, THEN MOVE A CONTROLLER".into(),
                ..MidiList::default()
            });
        }
        drawer.render(&scene);
        let d = drawer.frame();
        let at = (DRAWER_TOP * scale).round() as usize - r.top_px().max(0) as usize
            + (-r.top_px()).max(0) as usize;
        let w = all.width() as usize;
        let rows = (d.height() as usize).min(all.height() as usize - at);
        for y in 0..rows {
            let n = w.min(d.width() as usize) * 4;
            let (from, to) = (y * d.width() as usize * 4, (at + y) * w * 4);
            all.data_mut()[to..to + n].copy_from_slice(&d.data()[from..from + n]);
        }
    }
    if std::env::var("CA72_MENU").as_deref() == Ok("settings") {
        settings_menu(&mut all, &r);
    }
    if std::env::var("CA72_KEYBOARD_CROP").is_ok_and(|v| v == "1") {
        let [x0, y0, x1, y1] = [370.0, 925.0, 3080.0, 1750.0].map(|v| (v * scale).round() as u32);
        let mut cropped = ca72_panel::Pixmap::new(x1 - x0, y1 - y0).expect("keyboard proof crop");
        let n = (x1 - x0) as usize * 4;
        for y in y0..y1 {
            let src = (y as usize * all.width() as usize + x0 as usize) * 4;
            let dst = (y - y0) as usize * n;
            cropped.data_mut()[dst..dst + n].copy_from_slice(&all.data()[src..src + n]);
        }
        all = cropped;
    }
    if let Err(e) = all.save_png(&out) {
        eprintln!("{out}: {e}");
        std::process::exit(1);
    }
}

/// The built-in standalone's menu, as `learning::open_settings_menu` lays it out in 0.3.0.
/// Drawn last, over the strip too, as the editor composes its overlays.
fn settings_menu(frame: &mut ca72_panel::Pixmap, r: &Renderer) {
    use ca72_panel::art::{H, W};
    use ca72_panel::learn::{self, Menu};
    let labels = [
        "LOOK",
        "REALISTIC",
        "CLASSIC",
        "LETTERING",
        "STANDARD",
        "LARGE",
        "FREQUENCY DIALS",
        "SEMITONES",
        "HARDWARE",
        "WINDOW",
        "SIZE…",
        "WHAT'S NEW…",
        "MOD WHEEL SOURCE",
        "CC 1…",
        "STANDALONE",
        "AUDIO & MIDI…",
        "AGENTS",
        "ALLOW",
        "OFF",
    ];
    let mut menu = Menu {
        x: 0.0,
        y: 0.0,
        size: r.text_size() * 1.15,
        title: "SETTINGS · CA-72 0.3.0".into(),
        items: labels.iter().map(|t| ((*t).into(), true)).collect(),
        heads: vec![0, 3, 6, 9, 12, 14, 16],
        marked: vec![
            1,
            if r.lettering() == ca72_panel::Lettering::Large {
                5
            } else {
                4
            },
            7,
            17,
        ],
        hover: None,
        columns: Vec::new(),
    };
    let (w, _) = menu.extent(r.fonts());
    menu = menu.placed(r.fonts(), 3055.0 - w, 1017.0, [0.0, 0.0, W, H]);
    let layer = learn::menu(r.fonts(), &menu);
    let body = format!(
        "<g transform='translate({} {})'>{}</g>",
        menu.x, menu.y, layer.body
    );
    let doc = ca72_panel::art::document(
        [0.0, r.top(), W, H - r.top()],
        frame.width(),
        frame.height(),
        &body,
    );
    let options = resvg::usvg::Options {
        fontdb: r.fonts().database(),
        font_family: ca72_panel::fonts::FAMILY.into(),
        ..Default::default()
    };
    let tree = resvg::usvg::Tree::from_str(&doc, &options).expect("the review menu parses");
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut frame.as_mut(),
    );
    eprintln!("settings items {} logical px", menu.size * r.scale());
}

/// The plugin's default for a parameter, normalized.
fn default_of(param: &str) -> f64 {
    match param {
        "tune" | "osc2_frequency" | "osc3_frequency" | "cutoff" | "pitch_wheel" => 0.5,
        "pressure_mod" | "pressure_filter" | "timbre_mod" | "timbre_filter" => 0.5,
        "osc1_range" | "osc2_range" | "osc3_range" => 0.6,
        "osc1_waveform" | "osc2_waveform" | "osc3_waveform" => 0.4,
        "osc1_volume" | "osc2_volume" | "osc3_volume" => 0.7,
        "osc1_on" | "osc2_on" | "osc3_on" | "main_output" | "keyboard_control_1"
        | "keyboard_control_2" | "osc_mod" => 1.0,
        "emphasis" | "glide" | "mod_mix" | "mod_wheel" | "ext_volume" | "noise_volume" => 0.0,
        "contour_amount" | "filter_sustain" | "loudness_sustain" | "volume" => 0.5,
        "filter_attack" | "loudness_attack" => 0.1,
        "filter_decay" | "loudness_decay" => 0.4,
        _ => 0.0,
    }
}

/// The strip's knobs at what [`example_strip`]'s readouts read (normalized): VOICES 8 (of 2 to
/// 10), ENTROPY 40 %, WIDTH 70 %, INNER 40 %, DRIVE 8.4 dB (of 24), LEVEL 0 dB (of -30 to +12);
/// DETUNE off.
fn strip_knob(param: &str) -> Option<f64> {
    Some(match param {
        "voices" => 0.75,
        "entropy" | "inner" => 0.4,
        "spread" => 0.7,
        "drive" => 8.4 / 24.0,
        "level" => 30.0 / 42.0,
        _ => return None,
    })
}

/// The strip as the mock-up showed it: POLY, SCATTER, EVEN, AUTO GAIN on, eight voices of which
/// four sound, a preset chosen. In MONO (without DOUBLE) the one voice sounds at one of the eight
/// places, the one its last attack took (decisions.md R62), the readouts lit as in POLY.
fn example_strip(mono: bool) -> ca72_panel::strip::StripScene {
    use ca72_panel::strip::{Field, StripScene};
    let r = |t: &str, on: bool| (t.to_owned(), on);
    StripScene {
        mode: Some(if mono { 0 } else { 1 }),
        stereo: Some(0),
        placement: Some(0),
        auto: true,
        readouts: [
            r("8", true),
            r("40", true),
            r("70", true),
            r("OFF", false),
            r("-7.2", true),
            r("8.4", true),
            r("0.0", true),
            r("40", true),
        ],
        // EVEN's places for eight voices, each in its side's band at WIDTH 70 %, INNER 40 %.
        field: Field::Scatter(
            [-7.0, 7.0, -3.0, 3.0, -5.0, 5.0, -1.0, 1.0]
                .iter()
                .map(|p: &f64| p / 7.0)
                .enumerate()
                .map(|(i, p)| {
                    let on = if mono { i == 2 } else { i % 2 == 0 };
                    (0.7 * p.signum() * (0.4 + 0.6 * p.abs()), on)
                })
                .collect(),
        ),
        bar: ca72_panel::presets::BarScene {
            panel: true,
            name: "Brass Tutti".into(),
            found: true,
            favorite: true,
            ..Default::default()
        },
        ..StripScene::default()
    }
}

/// The arpeggiator's drawer over `all` (the editor's `compose`): its plate from its top, its
/// shadow laid over the page under its foot.
fn drop_arp(
    all: &mut ca72_panel::Pixmap,
    r: &mut Renderer,
    scale: f64,
    skin: Skin,
    lettering: ca72_panel::Lettering,
) {
    use ca72_panel::strip::arp::{self, ArpRenderer, ArpScene};
    let t = std::time::Instant::now();
    let mut drawer = ArpRenderer::new(scale, skin, lettering);
    let r2 = |s: &str| (s.to_owned(), true);
    let var = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<usize>().ok());
    let octaves = var("CA72_ARP_OCTAVES").unwrap_or(2).clamp(1, 5);
    drawer.render(
        &ArpScene {
            on: true,
            latch: std::env::var("CA72_ARP_HOLD").is_ok_and(|v| v == "1"),
            direction: var("CA72_ARP_DIRECTION").unwrap_or(0).min(4),
            readouts: [r2("16"), r2(&octaves.to_string()), r2("60"), r2("25")],
            knobs: [9.0 / 12.0, (octaves - 1) as f64 / 4.0, 0.6, 0.25],
            hover: None,
        },
        r,
    );
    eprintln!("the arpeggiator's drawer {:?}", t.elapsed());
    let (w, f) = (all.width() as usize, drawer.frame());
    // (The frame begins at the panel's top: a skin pack's taller panel's above the drawing's.)
    let top = (arp::top_px(scale) - r.top_px()) as usize;
    let (x0, x1) = (
        (arp::LEFT * scale).round() as usize,
        (arp::RIGHT * scale).round() as usize,
    );
    let depth = (arp::DEPTH * scale).round() as usize;
    for y in 0..depth.min(f.height() as usize) {
        let (from, to) = ((y * f.width() as usize + x0) * 4, ((top + y) * w + x0) * 4);
        let n = (x1 - x0) * 4;
        all.data_mut()[to..to + n].copy_from_slice(&f.data()[from..from + n]);
    }
    if let Some(shadow) = drawer.shadow() {
        all.draw_pixmap(
            0,
            (top + depth) as i32,
            shadow.as_ref(),
            &resvg::tiny_skia::PixmapPaint::default(),
            resvg::tiny_skia::Transform::identity(),
            None,
        );
    }
}

/// The expression drawer over `all` (the editor's `compose`), as the arpeggiator's.
fn drop_mpe(
    all: &mut ca72_panel::Pixmap,
    r: &mut Renderer,
    scale: f64,
    skin: Skin,
    lettering: ca72_panel::Lettering,
) {
    use ca72_panel::strip::mpe::{self, Input, MpeRenderer, MpeScene};
    let t = std::time::Instant::now();
    let mut drawer = MpeRenderer::new(scale, skin, lettering);
    let r2 = |s: &str| (s.to_owned(), true);
    let input = match std::env::var("CA72_MPE_INPUT").as_deref() {
        Ok("channel") => Input::Channel,
        Ok("none") => Input::None,
        _ => Input::Mpe,
    };
    drawer.render(
        &MpeScene {
            input,
            readouts: [
                r2(&std::env::var("CA72_MPE_P").unwrap_or_else(|_| "50".into())),
                r2("1.0"),
                r2("-30"),
                r2("2.5"),
                r2("-35"),
                r2("15"),
            ],
            knobs: [0.75, 5.0 / 8.0, 0.35, 6.5 / 8.0, 0.325, 0.15],
            // (The velocity's row, decisions.md R-VELOCITY: the mock-up's.)
            last: "97".into(),
            modulates: ("3 KNOBS".into(), true),
            response: mpe::Response {
                curve: -0.35,
                floor: 0.15,
                point: Some(97.0 / 127.0),
            },
        },
        r,
    );
    eprintln!("the expression drawer {:?}", t.elapsed());
    let (w, f) = (all.width() as usize, drawer.frame());
    let top = (mpe::top_px(scale) - r.top_px()) as usize;
    let (x0, x1) = (
        (mpe::LEFT * scale).round() as usize,
        (mpe::RIGHT * scale).round() as usize,
    );
    let depth = (mpe::DEPTH * scale).round() as usize;
    for y in 0..depth.min(f.height() as usize) {
        let (from, to) = ((y * f.width() as usize + x0) * 4, ((top + y) * w + x0) * 4);
        let n = (x1 - x0) * 4;
        all.data_mut()[to..to + n].copy_from_slice(&f.data()[from..from + n]);
    }
    if let Some(shadow) = drawer.shadow() {
        all.draw_pixmap(
            0,
            (top + depth) as i32,
            shadow.as_ref(),
            &resvg::tiny_skia::PixmapPaint::default(),
            resvg::tiny_skia::Transform::identity(),
            None,
        );
    }
}

/// The effects' drawer over `all` (the editor's `compose`): its plate from its top, what hangs
/// under its foot (its shadow, the list) laid over the page.
fn drop_fx(
    all: &mut ca72_panel::Pixmap,
    scale: f64,
    origin: i32,
    skin: Skin,
    lettering: ca72_panel::Lettering,
    how: &str,
) {
    use ca72_panel::strip::fx::{self, FxList, FxRenderer, FxRow, FxScene, FxSlot, FxWord};
    let t = std::time::Instant::now();
    let mut drawer = FxRenderer::new(scale, skin, lettering);
    let slot = |text: &str, lit: bool| FxSlot {
        text: text.into(),
        dim: false,
        lit,
        word: None,
    };
    let mut scene = FxScene {
        slots: [
            slot("Chorus", true),
            slot("Hall Reverb", true),
            slot("Tape Delay", false),
            slot("+ ADD EFFECT", false),
        ],
        ..FxScene::default()
    };
    if how == "crashed" {
        scene.slots[1] = FxSlot {
            text: "Hall Reverb".into(),
            dim: true,
            lit: false,
            word: Some(FxWord::Crashed),
        };
    }
    // The safety states (decisions.md R-FX-SAFETY): one off for a safe start, one turned off
    // after it crashed, one after it stopped responding, one loading.
    if how == "safety" || how == "off" {
        let word = |text: &str, word: FxWord| FxSlot {
            text: text.into(),
            dim: true,
            lit: false,
            word: Some(word),
        };
        scene.slots = if how == "off" {
            [
                word("Chorus", FxWord::Off),
                word("Hall Reverb", FxWord::Off),
                word("Tape Delay", FxWord::Off),
                slot("+ ADD EFFECT", false),
            ]
        } else {
            [
                slot("Chorus", true),
                word("Hall Reverb", FxWord::Crashed),
                word("Tape Delay", FxWord::NotResponding),
                word("Shimmer", FxWord::Loading),
            ]
        };
    }
    if how == "list" {
        let row = |name: &str, maker: &str, format: &str| FxRow {
            name: name.into(),
            maker: maker.into(),
            format: format.into(),
            kind: if name == "Gain" { "Tools" } else { "Reverb" }.into(),
            ..FxRow::default()
        };
        let mut rows = vec![
            row("Dragonfly Early Reflections", "Michael Willis", "CLAP"),
            row("Dragonfly Hall Reverb", "Michael Willis", "CLAP"),
            row("Dragonfly Plate Reverb", "Michael Willis", "CLAP"),
            row("Dragonfly Room Reverb", "Michael Willis", "CLAP"),
            row("Dragonfly Early Reflections", "Michael Willis", "VST3"),
            row("Dragonfly Hall Reverb", "Michael Willis", "VST3"),
            row("Gain", "DAW-v3 tests", "CLAP"),
            row("Gain", "DAW-v3 tests", "VST3"),
        ];
        rows.push(FxRow {
            name: "SCAN AGAIN".into(),
            action: true,
            ..FxRow::default()
        });
        scene.list = Some(FxList {
            slot: 3,
            title: "ADD TO SLOT 4".into(),
            note: "8 EFFECTS".into(),
            rows,
            first: 0,
            hover: Some(1),
            ..FxList::default()
        });
    }
    drawer.render(&scene);
    eprintln!("the effects' drawer {:?}", t.elapsed());
    let (w, f) = (all.width() as usize, drawer.frame());
    let top = (fx::top_px(scale) - origin) as usize;
    let (x0, x1) = (
        (fx::LEFT * scale).round() as usize,
        (fx::RIGHT * scale).round() as usize,
    );
    let depth = (fx::DEPTH * scale).round() as usize;
    for y in 0..depth.min(f.height() as usize) {
        let (from, to) = ((y * f.width() as usize + x0) * 4, ((top + y) * w + x0) * 4);
        let n = (x1 - x0) * 4;
        all.data_mut()[to..to + n].copy_from_slice(&f.data()[from..from + n]);
    }
    for (p, x, y) in drawer.under() {
        all.draw_pixmap(
            x,
            top as i32 + y,
            p.as_ref(),
            &resvg::tiny_skia::PixmapPaint::default(),
            resvg::tiny_skia::Transform::identity(),
            None,
        );
    }
}

/// The round-2 mock-ups' four LFOs, as the editor shows them: their states and traces.
pub fn lfo_example() -> ca72_panel::strip::lfo::LfoScene {
    use ca72_panel::strip::lfo::{Column, LfoScene, TRACE_POINTS};
    // (wave place, continuous, rate Hz, timed division, phase deg, mode, mono, fade s, screen, orb)
    #[allow(clippy::type_complexity)]
    let lfos: [(
        f64,
        bool,
        f64,
        Option<usize>,
        f64,
        f64,
        bool,
        f64,
        &str,
        f64,
    ); 4] = [
        (0.0, false, 2.4, None, 0.0, 0.0, true, 0.0, "2 KNOBS", 0.16),
        (
            2.0,
            false,
            0.35,
            None,
            180.0,
            0.5,
            false,
            1.2,
            "CUTOFF -25",
            0.62,
        ),
        (
            3.0,
            false,
            1.0,
            Some(9),
            90.0,
            1.0,
            true,
            0.0,
            "VOLUME -50",
            0.28,
        ),
        (
            2.4,
            true,
            6.5,
            None,
            0.0,
            0.0,
            true,
            0.5,
            "NOT IN USE",
            0.80,
        ),
    ];
    let colours =
        std::env::var("CA72_LFO_COLORS").unwrap_or_else(|_| "amber,green,blue,red".into());
    let rgb = |w: &str| match w {
        "green" => [0x26, 0xff, 0xb7],
        "blue" => [0x4c, 0x8c, 0xff],
        "red" => [0xe6, 0x39, 0x48],
        "violet" => [0xc0, 0x80, 0xff],
        "white" => [0xe4, 0xec, 0xf4],
        _ => [0xff, 0xb0, 0x20],
    };
    let words: Vec<&str> = colours.split(',').collect();
    let mut scene = LfoScene::default();
    for (n, (c, l)) in scene.columns.iter_mut().zip(lfos).enumerate() {
        let (at, continuous, hz, timed, phase, mode, mono, fade, screen, orb) = l;
        let fade = std::env::var("CA72_LFO_FADE")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(fade);
        // (A dotted eighth at 120 BPM: 0.75 of a beat.)
        let hz = if timed.is_some() { 2.0 / 0.75 } else { hz };
        let shape = |at: f64, p: f64| -> f64 {
            let p = p.rem_euclid(1.0);
            let place = |k: usize| match k {
                0 => {
                    if p < 0.5 {
                        -1.0 + 4.0 * p
                    } else {
                        3.0 - 4.0 * p
                    }
                }
                1 => {
                    let peak = 14.0 / 22.0;
                    if p < peak {
                        -1.0 + 2.0 * p / peak
                    } else {
                        -(p - peak) / (1.0 - peak)
                    }
                }
                2 => 2.0 * p - 1.0,
                _ => {
                    if p < 0.5 {
                        1.0
                    } else {
                        -1.0
                    }
                }
            };
            if at >= 3.0 {
                return place(3);
            }
            let k = at.floor() as usize;
            let f = at - k as f64;
            (1.0 - f) * place(k) + f * place(k + 1)
        };
        // (One cycle from PHASE, as the editor's: its end just short of the next one's start.)
        let out = |x: f64| shape(at, phase / 360.0 + x * (1.0 - 1e-9));
        c.trace = (0..TRACE_POINTS)
            .map(|i| out(i as f64 / (TRACE_POINTS - 1) as f64) as f32)
            .collect();
        // (ENV's orb held at its cycle's end; a FADE is fading in or out, a little way into it.)
        let orb = if mode == 1.0 { 1.0 } else { orb };
        let gain = if fade != 0.0 { 0.6 } else { 1.0 };
        c.orb = Some((orb, out(orb) * gain));
        c.orb_fade = gain;
        c.colour = rgb(words.get(n).copied().unwrap_or("amber"));
        c.continuous = continuous;
        c.shape_at = at;
        c.wave = at.round() / 5.0;
        c.rate = ((hz - 0.01) / 49.99).powf(0.2);
        if let Some(d) = timed {
            c.timed = true;
            c.division = d;
            c.rate = d as f64 / 17.0;
        }
        c.phase = phase / 360.0;
        c.fade = 0.5 + 0.5 * fade.signum() * (fade.abs() / 10.0).cbrt();
        c.mode = mode;
        c.mono = mono;
        c.screen = screen.into();
        c.in_use = screen != "NOT IN USE";
        let _ = Column::default;
    }
    // (`CA72_LFO_KEYS=1`: LFO 2 UNI, its trace 0..1; LFO 3 off, MUTED: R-LFO's corrections.)
    if std::env::var("CA72_LFO_KEYS").is_ok_and(|v| v == "1") {
        let c = &mut scene.columns[1];
        c.uni = true;
        for v in &mut c.trace {
            *v = 0.5 * (*v + 1.0);
        }
        c.orb = c.orb.map(|(x, v)| (x, 0.5 * (v + 1.0)));
        let c = &mut scene.columns[2];
        c.on = false;
        c.orb = None;
        c.screen = "MUTED".into();
        c.in_use = false;
    }
    scene
}

/// The LFO page over the lower page (the editor's `compose`).
fn lfo_page(
    all: &mut ca72_panel::Pixmap,
    r: &mut Renderer,
    scale: f64,
    skin: Skin,
    lettering: ca72_panel::Lettering,
) {
    use ca72_panel::strip::lfo::{self, LfoRenderer};
    let t = std::time::Instant::now();
    let mut page = LfoRenderer::new(scale, skin, lettering);
    let mut scene = lfo_example();
    // (`CA72_LFO_LIVE=1`: every scope's trace drawn as a moved one's, its cheaper glow:
    // decisions.md R-LFO's corrections.)
    if std::env::var("CA72_LFO_LIVE").is_ok_and(|v| v == "1") {
        for c in &mut scene.columns {
            c.live = true;
        }
    }
    page.render(&scene, r);
    eprintln!("the LFO page {:?}", t.elapsed());
    let (w, f) = (all.width() as usize, page.frame());
    let top = (lfo::top_px(scale) - r.top_px()) as usize;
    let (x0, x1) = (
        (lfo::LEFT * scale).round() as usize,
        (lfo::RIGHT * scale).round() as usize,
    );
    let rows = (f.height() as usize).min(all.height() as usize - top);
    for y in 0..rows {
        let (from, to) = ((y * f.width() as usize + x0) * 4, ((top + y) * w + x0) * 4);
        let n = (x1 - x0) * 4;
        all.data_mut()[to..to + n].copy_from_slice(&f.data()[from..from + n]);
    }
}

/// The rings, a dragged assignment and the hover pop-up over `all`, as the editor lays them.
fn lfo_overlay(all: &mut ca72_panel::Pixmap, r: &Renderer, scale: f64, values: &[f64]) {
    use ca72_panel::modulation::{Cord, KnobRings, ModRenderer, ModScene, PopDial, Popup, Ring};
    let var = |k: &str| std::env::var(k).ok();
    let page = lfo_example();
    let colour = |n: usize| page.columns[n].colour;
    let knob = |param: &str| {
        let i = CONTROLS.iter().position(|c| c.param == param)?;
        let c = r.arrangement().control(i);
        let skirt = match c.kind {
            ca72_panel::Kind::Knob { big: true, .. } => ca72_panel::art::BIG.skirt,
            _ => ca72_panel::art::STD.skirt,
        };
        Some((i, c.centre(), skirt, values[i]))
    };
    let assigned: [(&str, &[(usize, f64)]); 3] = [
        ("cutoff", &[(0, 0.40), (1, -0.25)]),
        ("osc2_frequency", &[(0, -0.10)]),
        ("volume", &[(2, -0.50)]),
    ];
    let mut scene = ModScene::default();
    if var("CA72_LFO_RINGS").as_deref() == Some("1") {
        for (param, mods) in assigned {
            let Some((control, at, skirt, travel)) = knob(param) else {
                continue;
            };
            scene.knobs.push(KnobRings {
                bar: None,
                control,
                at,
                skirt,
                sweep: ca72_panel::art::KNOB_SWEEP,
                travel,
                rings: mods
                    .iter()
                    .map(|&(n, depth)| Ring {
                        colour: colour(n),
                        depth,
                        point: page.columns[n]
                            .orb
                            .map(|(_, v)| (travel + depth * v).clamp(0.0, 1.0)),
                        one_way: false,
                        dim: false,
                    })
                    .collect(),
            });
        }
    }
    if let Some(to) = var("CA72_LFO_CORD") {
        let targets = [
            "tune",
            "glide",
            "mod_mix",
            "osc2_frequency",
            "osc3_frequency",
            "osc1_volume",
            "ext_volume",
            "osc2_volume",
            "noise_volume",
            "osc3_volume",
            "cutoff",
            "emphasis",
            "contour_amount",
            "filter_attack",
            "filter_decay",
            "filter_sustain",
            "loudness_attack",
            "loudness_decay",
            "loudness_sustain",
            "volume",
            "feedback",
        ];
        let over = knob(&to).map(|(_, at, skirt, _)| (at, skirt));
        scene.cord = Some(Cord {
            boxes: Vec::new(),
            over_box: None,
            from: ca72_panel::strip::lfo::places(0).jack,
            to: over.map_or((2281.0, 170.0), |(at, _)| at),
            colour: colour(0),
            targets: targets
                .iter()
                .filter(|p| **p != to)
                .filter_map(|p| knob(p).map(|(_, at, skirt, _)| (at, skirt)))
                .collect(),
            over,
        });
    }
    if var("CA72_LFO_POPUP").as_deref() == Some("1")
        && let Some((_, at, skirt, _)) = knob("cutoff")
    {
        scene.popup = Some(Popup {
            knob: (at, skirt),
            span: 300.0,
            dials: vec![
                PopDial {
                    colour: colour(0),
                    label: "LFO 1".into(),
                    depth: 0.4,
                    shared: false,
                    uni: false,
                    muted: false,
                },
                PopDial {
                    colour: colour(1),
                    label: "LFO 2".into(),
                    depth: -0.25,
                    shared: true,
                    uni: false,
                    muted: false,
                },
            ],
            hover: Some(0),
            size: r.text_size(),
        });
    }
    // (`CA72_TEMPO_RAILS=1`: the standalone's TEMPO screen moved by LFO 1 and a macro, its
    // rails and pop-up under it: decisions.md R-LFO.)
    if var("CA72_TEMPO_RAILS").as_deref() == Some("1") {
        let (x, y, w, h) = ca72_panel::art::tempo_window(r.arrangement().plate_at());
        let foot = (x + w / 2.0, y + h + ca72_panel::art::TEMPO_SURROUND);
        let travel = (120.0 - 30.0) / 270.0;
        let macro_colour = [0xe8, 0x6a, 0x9a];
        scene.knobs.push(KnobRings {
            bar: Some(w / 2.0),
            control: usize::MAX,
            at: foot,
            skirt: 0.0,
            sweep: (-150.0, 150.0),
            travel,
            rings: vec![
                Ring {
                    colour: colour(0),
                    depth: 0.25,
                    point: Some(travel + 0.25 * 0.6),
                    one_way: false,
                    dim: false,
                },
                Ring {
                    colour: macro_colour,
                    depth: -0.2,
                    point: Some(travel - 0.2 * 0.5),
                    one_way: true,
                    dim: false,
                },
            ],
        });
        scene.popup = Some(Popup {
            knob: (foot, ca72_panel::modulation::rails_depth(2)),
            span: 300.0,
            dials: vec![
                PopDial {
                    colour: colour(0),
                    label: "LFO 1".into(),
                    depth: 0.25,
                    shared: false,
                    uni: false,
                    muted: false,
                },
                PopDial {
                    colour: macro_colour,
                    label: "MACRO 1".into(),
                    depth: -0.2,
                    shared: false,
                    uni: true,
                    muted: false,
                },
            ],
            hover: None,
            size: r.text_size(),
        });
    }
    if scene == ModScene::default() {
        return;
    }
    let mut m = ModRenderer::new(scale);
    m.rescale(scale, r.top_px());
    let t = std::time::Instant::now();
    m.render(&scene);
    eprintln!("the modulation overlay {:?}", t.elapsed());
    let place = [0, 0, all.width() as i32, all.height() as i32];
    m.lay_rings(all, place);
    m.lay_over(all, place);
}
