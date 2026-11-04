//! The CA-72's front panel, drawn: the instrument's layout and lettering as SVG (in panel
//! units), rendered with resvg into a frame, and what a pointer finds on it. It knows no
//! window and no plugin: the editor shows its frames and turns pointer gestures into
//! parameter changes with it.

pub mod arrangement;
pub mod art;
pub mod controls;
pub mod fonts;
mod glow;
pub mod interact;
pub mod keyboard;
pub mod learn;
pub mod look;
pub mod modulation;
pub mod notes;
pub mod presets;
pub mod render;
pub mod skin;
pub mod strip;
mod svg;
pub mod ultra;
pub mod value_entry;
pub mod whats_new;

pub use arrangement::Arrangement;
pub use art::{H, Layout, Lettering, PLATE_NAME, TicksTitles, W};
pub use controls::{CONTROLS, Control, Kind};
pub use interact::Target;
pub use look::Look;
pub use render::{Renderer, Scene};
/// The frames' type.
pub use resvg::tiny_skia::Pixmap;
pub use skin::Skin;
