//! Checks a skin pack as the CA-72 will (decisions.md R-SKIN-PACKS) and draws a preview of it:
//! `cargo run --release -p ca72-panel --example check_pack -- <folder> [preview.png]`.
//!
//! Prints what the checks found: what rejects the pack (the plug-in will not load it), what a
//! marketplace's review must look at (the pack loads, but is not cleared for sale: the checks
//! never clear anything), and notes. With a PNG's path, writes the panel drawn in the pack's
//! look, every control at the middle of its travel, in both letterings (`<name>.png` and
//! `<name>-large.png`). Exits 1 if the pack is rejected. The pack may be a folder or a pack file
//! (`.ifskin`). `--trust <key-id>:<public-key>` checks its signature against that key too (the
//! owner's, before the plug-in has it built in; this grants first-party trust for the check).
//! `--scale <pixels a unit>` draws the previews at another size than 0.5 (the editor's is its
//! window's width in physical pixels over 3108): with each, the smallest click area and the
//! smallest lettering of the pack's layout at that size, and the official panel's beside them.

use std::path::Path;

use ca72_panel::arrangement::hit_bounds;
use ca72_panel::controls::Place;
use ca72_panel::{Arrangement, CONTROLS, Lettering, Look, Renderer, Scene};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut keys: Vec<skin_pack::signature::TrustedKey> = ca72_panel::look::TRUSTED_KEYS.to_vec();
    while let Some(i) = args.iter().position(|a| a == "--trust") {
        let given = args.get(i + 1).cloned().unwrap_or_default();
        let (id, public) = given.split_once(':').unwrap_or(("", ""));
        let Some(public) = skin_pack::signature::public_key(public) else {
            eprintln!("--trust <key-id>:<public key, 64 hexadecimal digits>");
            std::process::exit(2);
        };
        keys.push(skin_pack::signature::TrustedKey {
            id: Box::leak(id.to_owned().into_boxed_str()),
            public,
            first_party: true,
        });
        args.drain(i..=(i + 1).min(args.len() - 1));
    }
    let mut scale = 0.5;
    if let Some(i) = args.iter().position(|a| a == "--scale") {
        scale = match args.get(i + 1).and_then(|s| s.parse::<f64>().ok()) {
            Some(s) if (0.05..=4.0).contains(&s) => s,
            _ => {
                eprintln!("--scale <pixels a unit, 0.05 to 4>");
                std::process::exit(2);
            }
        };
        args.drain(i..=(i + 1).min(args.len() - 1));
    }
    let Some(dir) = args.first() else {
        eprintln!(
            "check_pack <folder or .ifskin> [preview.png] [--scale <pixels a unit>] \
             [--trust <key-id>:<public-key>]"
        );
        std::process::exit(2);
    };
    let look = match Look::open_with(Path::new(dir), &skin_pack::Limits::default(), &keys) {
        Ok(look) => look,
        Err(report) => {
            print!("{report}");
            println!("REJECTED: the CA-72 will not load this pack.");
            std::process::exit(1);
        }
    };
    print!("{}", look.report);
    println!(
        "LOADS: {} {} ({}), drawn {}, {}{}{}.",
        look.name,
        look.version,
        look.licence,
        if look.base == ca72_panel::Skin::Worn {
            "realistic"
        } else {
            "classic"
        },
        if look.first_party() {
            "published by Idle Foundry, "
        } else if look.signed.is_some() {
            "published by the marketplace, "
        } else if look.report.has("signature.unknown-key") {
            "signed by an unknown key (treated as unsigned), "
        } else {
            "unsigned, "
        },
        if look.fonts.is_some() {
            "in its own typeface"
        } else {
            "in the built-in typeface"
        },
        if look.report.needs_review() {
            "; a review must look at the findings above before it is listed"
        } else {
            ""
        }
    );
    let Some(out) = args.get(1) else { return };
    let scene = Scene {
        values: [0.5; CONTROLS.len()],
        ..Scene::default()
    };
    if let Some(folder) = Path::new(out)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
    {
        std::fs::create_dir_all(folder).expect("the preview's folder");
    }
    for (lettering, suffix) in [(Lettering::Standard, ""), (Lettering::Large, "-large")] {
        let mut r = Renderer::with_pack(look.clone(), lettering, scale, 1.0);
        r.render(&scene);
        let path = match out.strip_suffix(".png") {
            Some(stem) => format!("{stem}{suffix}.png"),
            None => format!("{out}{suffix}.png"),
        };
        r.frame().save_png(&path).expect("the preview written");
        let ((w, h, param), (size, text)) = smallest(&look.arrangement, lettering, scale);
        let ((ow, oh, _), (osize, _)) = smallest(&Arrangement::official(), lettering, scale);
        println!(
            "preview: {path} ({} by {} pixels, {scale} a unit): the smallest click area {w:.0} by \
             {h:.0} pixels (`{param}`), the smallest lettering {size:.1} (\"{text}\"); the \
             official panel's at this size {ow:.0} by {oh:.0}, {osize:.1}",
            r.frame().width(),
            r.frame().height()
        );
    }
}

/// At `scale` pixels a unit, the smallest click area of the panel's controls in `arrangement`
/// (its width and height, its control) and the smallest lettering it prints in `lettering` (a
/// legend's or a title's size, its text).
fn smallest(
    arrangement: &Arrangement,
    lettering: Lettering,
    scale: f64,
) -> ((f64, f64, &'static str), (f64, String)) {
    let area = arrangement
        .controls
        .iter()
        .filter(|c| c.place == Place::Panel)
        .map(|c| {
            let b = hit_bounds(&c.kind);
            ((b[2] - b[0]) * scale, (b[3] - b[1]) * scale, c.param)
        })
        .min_by(|p, q| p.0.min(p.1).total_cmp(&q.0.min(q.1)))
        .unwrap_or((0.0, 0.0, ""));
    let print = arrangement
        .texts
        .iter()
        .map(|t| (t.size_in(lettering), t.text.as_str()))
        .chain(arrangement.titles.iter().map(|t| (t.size, t.text.as_str())))
        .min_by(|p, q| p.0.total_cmp(&q.0))
        .map_or((0.0, String::new()), |(size, text)| {
            (size * scale, text.to_owned())
        });
    (area, print)
}
