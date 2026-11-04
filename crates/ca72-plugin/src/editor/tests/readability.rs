//! The editor at 4K and on small screens, for measuring its lettering (issue #70,
//! `docs/readability-4k.md`): `CA72_READABILITY_PNG=<folder> cargo test -p ca72-plugin
//! the_readability_pngs -- --ignored`. Each window size, look, lettering and state is written
//! as `<size>-<look>-<lettering>-<state>.png`, and `sizes.csv` beside them has the em (physical
//! pixels) of the text that follows the window's logical size rather than the panel's: the tip,
//! the menus' items (after any fitting to the window) and a typed value. `quiet` is the GLOBAL
//! page with TICKS & TITLES QUIET.
//! `CA72_READABILITY_ONLY=4k100-fit,laptop` limits the window sizes.

use super::*;

/// The window sizes: a name, its backing scale (physical pixels a logical one) and its logical
/// width. A 3840 by 2160 screen at 100 % opens the editor at 3026 (FIT TO SCREEN, 80 % of its
/// height); at 150 % its 2560 by 1440 logical pixels give 2017, again 3026 physical. SIZE's
/// 100 % is 1380 logical at either scale. A 1920 by 1080 screen's FIT TO SCREEN is 1513; a 1366
/// by 768 laptop fits 1076; SIZE's 75 % is 1035; the grip's smallest 860.
const SIZES: [(&str, f64, u32); 8] = [
    ("4k100-fit", 1.0, 3026),
    ("4k100-size100", 1.0, 1380),
    ("4k150-fit", 1.5, 2017),
    ("4k150-size100", 1.5, 1380),
    ("1080p-fit", 1.0, 1513),
    ("laptop", 1.0, 1076),
    ("size75", 1.0, 1035),
    ("min", 1.0, MIN_WIDTH),
];

#[test]
#[ignore = "writes images for measuring"]
fn the_readability_pngs() {
    let Some(out) = std::env::var_os("CA72_READABILITY_PNG").map(std::path::PathBuf::from) else {
        return;
    };
    std::fs::create_dir_all(&out).unwrap();
    let only = std::env::var("CA72_READABILITY_ONLY").ok();
    let mut csv = String::from(
        "size,look,lettering,dpr,physical_width,scale,tip,menu_item,settings_item,entry\n",
    );
    for (size, dpr, width) in SIZES {
        if only
            .as_ref()
            .is_some_and(|o| !o.split(',').any(|s| s == size))
        {
            continue;
        }
        let physical = (f64::from(width) * dpr).round() as u32;
        for (skin, look) in [(Skin::Worn, "realistic"), (Skin::Drawn, "classic")] {
            for (lettering, lt) in [(Lettering::Standard, "std"), (Lettering::Large, "large")] {
                let (mut e, _, _) = editing();
                e.fit(dpr, width, physical);
                e.set_look(skin);
                e.set_lettering(lettering);
                e.draw();
                let name = |state: &str| out.join(format!("{size}-{look}-{lt}-{state}.png"));
                let away = |e: &mut Editing| go(e, Point { x: 2.0, y: 2.0 });
                let save = |e: &mut Editing, state: &str| {
                    e.composed = None;
                    window_frame(e).save_png(name(state)).unwrap();
                };
                let bar = |e: &mut Editing, t: BarTarget| {
                    let p = at(e, strip::centre(e.rail, StripTarget::Bar(t)));
                    click(e, p);
                    e.draw();
                };
                let s = e.renderer.scale();
                away(&mut e);
                save(&mut e, "global");
                // CUTOFF's tip.
                let cutoff = at(&e, centre("cutoff"));
                go(&mut e, cutoff);
                save(&mut e, "tip");
                let tip = e.renderer.text_size() * s;
                // CUTOFF's menu.
                right_click(&mut e, cutoff);
                let m = e.learning.menu().expect("CUTOFF's menu").clone();
                save(&mut e, "control-menu");
                e.learning.close_menu();
                // The settings' menu, and SIZE's.
                let key = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Settings)));
                click(&mut e, key);
                let settings = e.learning.menu().expect("the settings' menu").clone();
                save(&mut e, "settings");
                menu_item(&mut e, "SIZE…");
                save(&mut e, "size-menu");
                e.learning.close_menu();
                away(&mut e);
                // A typed value (CUTOFF's, in Hz).
                entry_click(&mut e, "cutoff");
                e.draw();
                let entry = e.scene.entry.as_ref().map_or(0.0, |f| f.size * s);
                save(&mut e, "entry");
                keyed(&mut e, Key::Escape);
                away(&mut e);
                // The lower pages, the drawers and the left area's macros.
                bar(&mut e, BarTarget::Keyboard);
                away(&mut e);
                save(&mut e, "keyboard");
                bar(&mut e, BarTarget::Lfo);
                away(&mut e);
                save(&mut e, "lfo");
                bar(&mut e, BarTarget::Macro);
                away(&mut e);
                save(&mut e, "macro");
                bar(&mut e, BarTarget::Macro);
                bar(&mut e, BarTarget::Global);
                bar(&mut e, BarTarget::Arp);
                e.lower.slide = Some((Instant::now() - crate::presets::SLIDE * 2, true));
                away(&mut e);
                save(&mut e, "arp");
                bar(&mut e, BarTarget::Arp);
                e.lower.slide = Some((Instant::now() - crate::presets::SLIDE * 2, false));
                let name_key = on_strip(&e, strip_at(StripTarget::Bar(BarTarget::Name)));
                click(&mut e, name_key);
                std::thread::sleep(crate::presets::SLIDE + Duration::from_millis(20));
                away(&mut e);
                save(&mut e, "presets");
                // The panel with TICKS & TITLES QUIET (decisions.md R-QUIET-PRINT).
                click(&mut e, name_key);
                std::thread::sleep(crate::presets::SLIDE + Duration::from_millis(20));
                e.set_ticks_titles(TicksTitles::Quiet);
                away(&mut e);
                save(&mut e, "quiet");
                e.set_ticks_titles(TicksTitles::Standard);
                let item = |m: &ca72_panel::learn::Menu| m.size * s;
                csv.push_str(&format!(
                    "{size},{look},{lt},{dpr},{physical},{s:.5},{tip:.3},{:.3},{:.3},{entry:.3}\n",
                    item(&m),
                    item(&settings),
                ));
            }
        }
    }
    std::fs::write(out.join("sizes.csv"), csv).unwrap();
}

/// The standalone's settings menu (issue #70, round 4), as it opens (`settings`) and at its
/// fullest (`settings-full`: the audio not real-time, AGENTS allowed and saying how they stand,
/// a setup copied), with the window without it (`global`), at each size, look and lettering:
/// `CA72_STANDALONE_SETTINGS_PNG=<folder> cargo test -p ca72-plugin --features
/// standalone-production the_standalone_settings_pngs -- --ignored`. `sizes.csv` beside them
/// has the menu's columns' rows and its items' em (physical pixels) against its lettering's.
#[cfg(feature = "standalone-production")]
#[test]
#[ignore = "writes images for looking at"]
fn the_standalone_settings_pngs() {
    let Some(out) = std::env::var_os("CA72_STANDALONE_SETTINGS_PNG").map(std::path::PathBuf::from)
    else {
        return;
    };
    std::fs::create_dir_all(&out).unwrap();
    let only = std::env::var("CA72_READABILITY_ONLY").ok();
    let mut csv = String::from("size,look,lettering,state,rows,em,full_em\n");
    for (size, dpr, width) in SIZES {
        if only
            .as_ref()
            .is_some_and(|o| !o.split(',').any(|s| s == size))
        {
            continue;
        }
        let physical = (f64::from(width) * dpr).round() as u32;
        for (skin, look) in [(Skin::Worn, "realistic"), (Skin::Drawn, "classic")] {
            for (lettering, lt) in [(Lettering::Standard, "std"), (Lettering::Large, "large")] {
                for (full, state) in [(false, "settings"), (true, "settings-full")] {
                    let mut e = standalone_settings_editor(full);
                    e.fit(dpr, width, physical);
                    e.set_look(skin);
                    e.set_lettering(lettering);
                    e.draw();
                    let name = |state: &str| out.join(format!("{size}-{look}-{lt}-{state}.png"));
                    let save = |e: &mut Editing, state: &str| {
                        e.composed = None;
                        window_frame(e).save_png(name(state)).unwrap();
                    };
                    go(&mut e, Point { x: 2.0, y: 2.0 });
                    if !full {
                        save(&mut e, "global");
                    }
                    press_settings_key(&mut e);
                    let m = e.learning.menu().expect("the settings' menu").clone();
                    // (The pointer off the menu, nothing in it lit.)
                    go(&mut e, Point { x: 2.0, y: 2.0 });
                    save(&mut e, state);
                    let rows: Vec<String> = std::iter::once(0)
                        .chain(m.columns.iter().map(|c| c.0))
                        .chain(std::iter::once(m.items.len()))
                        .collect::<Vec<_>>()
                        .windows(2)
                        .map(|s| (s[1] - s[0]).to_string())
                        .collect();
                    let s = e.renderer.scale();
                    csv.push_str(&format!(
                        "{size},{look},{lt},{state},{},{:.3},{:.3}\n",
                        rows.join(" "),
                        m.size * s,
                        e.renderer.text_size() * MENU_TEXT * s,
                    ));
                }
            }
        }
    }
    std::fs::write(out.join("sizes.csv"), csv).unwrap();
}
