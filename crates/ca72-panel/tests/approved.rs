//! The panel as drawn matches the design the owner approved (`approved.png`: the mock-up at one
//! panel unit a pixel, every control at half its travel). The switch column between MIXER and
//! MODIFIERS (the mock-up's x 2100 to 2330, y 140 to 840) is the renderer's own, as the owner
//! chose it on 2026-10-09 with FILTER MODE at its head (decisions.md R46), and on 2026-11-03
//! with FILTER POLES at its head and FILTER MODE's toggle under it (R-POLES). The mock-up had the
//! left hand's controls in a column left of the panel, 330 units wide; they are on the strip
//! below now (A6, R44), so the panel is compared with the mock-up right of its column. The
//! name board under the face is gone (the plate moved up into the top strip, R44): the top
//! strip and the face are compared, the plate's place there left out. The last column's head,
//! above POWER's lamp, is left out: QUALITY, its toggle and ULTRA's lamp came after the mock-up
//! and were approved in their own (decisions.md R47, R48).
//!
//! The large lettering (R-LETTERING, a setting) is compared with `approved-large.png`: the
//! same mock-up, but where the large lettering changes the drawing, the renderer's pixels, as
//! the owner approved them on 2026-10-15 (as the switch column has the renderer's).
//!
//! Oscillators 2's and 3's TIMED keys (decisions.md R-OSC-TIMED) came after the mock-up and were
//! approved in their own; away from LO, as at 8' here, they are off the panel, so both pictures
//! are the mock-up's own there again (as before 2026-10-20's keys).

use ca72_panel::art::{COL, PANEL_H, PH, PLATE_H, PLATE_X, PLATE_Y, TOP, W};
use ca72_panel::render::Timed;
use ca72_panel::{CONTROLS, Lettering, Renderer, Scene, Skin};
use resvg::tiny_skia::Pixmap;

/// The width of the mock-up's column, left of the panel.
const COLUMN: u32 = 330;

/// The mock-up drew its name plate on a name board below the face, in a fallback face; the
/// plate is in the top strip now, so its place there is left out.
fn outside_the_plate(x: u32, y: u32) -> bool {
    let (x, y) = (f64::from(x), f64::from(y));
    !(PLATE_X - 30.0..PLATE_X + 300.0).contains(&x)
        || !(PLATE_Y - 12.0..PLATE_Y + PLATE_H + 16.0).contains(&y)
}

/// QUALITY's and ULTRA's place: the last column (between its divider and the trim), from the panel's top
/// to above POWER's lamp.
fn outside_quality(x: u32, y: u32) -> bool {
    let (x, y) = (f64::from(x), f64::from(y));
    !(COL + 2956.0..COL + 3080.0).contains(&x) || !(TOP..TOP + 395.0).contains(&y)
}

#[test]
fn the_panel_is_drawn_as_approved() {
    drawn_as_approved(Lettering::Standard, "approved.png");
}

#[test]
fn the_panel_in_the_large_lettering_is_drawn_as_approved() {
    drawn_as_approved(Lettering::Large, "approved-large.png");
}

/// The panel drawn in `lettering` against the approved drawing in `file`.
fn drawn_as_approved(lettering: Lettering, file: &str) {
    let approved = Pixmap::load_png(format!("{}/tests/{file}", env!("CARGO_MANIFEST_DIR")))
        .expect("the approved panel");
    let mut r = Renderer::with_look(Skin::Drawn, lettering, 1.0, 1.0);
    // (Every RANGE at half its travel is at 8': the TIMED keys are off the panel there.)
    r.render(&Scene {
        values: [0.5; CONTROLS.len()],
        timed: [Timed {
            division: 8,
            inert: true,
        }; 2],
        ..Scene::default()
    });
    let drawn = r.frame();
    assert_eq!(PANEL_H, TOP + PH);
    assert_eq!(drawn.width(), approved.width() - COLUMN);
    assert_eq!(drawn.width(), W as u32);

    let mut differences: Vec<u8> = Vec::new();
    for y in 0..PANEL_H as u32 {
        for x in 0..drawn.width() {
            let a = drawn.pixel(x, y).expect("in the frame");
            let b = approved.pixel(x + COLUMN, y).expect("in the mock-up");
            if !outside_the_plate(x, y) || !outside_quality(x, y) {
                continue;
            }
            let d = [
                a.red().abs_diff(b.red()),
                a.green().abs_diff(b.green()),
                a.blue().abs_diff(b.blue()),
            ];
            differences.push(d.into_iter().max().unwrap_or(0));
        }
    }
    let mean = differences.iter().map(|&d| f64::from(d)).sum::<f64>() / differences.len() as f64;
    // Strongly different pixels (32 levels or more): at most as many as a hundredth of the
    // approved panel's whole area, as when it had its name board (3108 by 1057; held to a
    // hundredth of what is compared now, the board's wood, which matched closely, gone from it,
    // the same face would fail).
    let strong = differences.iter().filter(|&&d| d >= 32).count();
    let allowed = (W * 1057.0 / 100.0) as usize;
    // The margin, for the record on each machine (`-- --nocapture`).
    println!(
        "the panel as drawn against {file}: mean difference {mean:.3}, {strong} pixels 32 levels or more apart (passes under 1.5 and {allowed})"
    );
    // Text rasterises a little lighter here than in the browser that drew the mock-up.
    assert!(
        mean < 1.5 && strong < allowed,
        "mean difference {mean:.2}, {strong} pixels 32 levels or more apart"
    );
}
