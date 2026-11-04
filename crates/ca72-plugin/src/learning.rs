//! MIDI Learn in the editor (decisions.md R34): a control's menu (a right click: MIDI LEARN,
//! REMOVE MIDI ASSIGNMENT, MIDI ASSIGNMENTS…), the ring and the note over the control being
//! learned, what was done said over it for a while, and the drawer's MIDI list, which the keyboard
//! operates. The assignments themselves are [`crate::learn::MidiMap`]'s; this is what the editor
//! shows of them and asks of it. Nothing here runs on the audio thread.

use std::time::{Duration, Instant};

use ca72::freq_dial::Law;
use ca72_panel::controls::{CONTROLS, index as control_index};
use ca72_panel::fonts::Fonts;
use ca72_panel::learn::{Menu, Note, PANEL, Tone, above};
use ca72_panel::presets::{
    BarTarget, MidiAction, MidiList, MidiRow, ROWS_SHOWN, Tone as DrawerTone,
};
use ca72_panel::strip::{Rail, StripControl, span};
use ca72_panel::{Lettering, Skin, Target, TicksTitles};
use keyboard_types::{Key, KeyState, KeyboardEvent, Modifiers};

use crate::learn::{Assigned, Binding, LEARNABLE, MOD_WHEEL, MidiMap, index};
use crate::mod_wheel::Source;

/// How long what was done stays said over its control.
const NOTICE: Duration = Duration::from_secs(4);

/// The keys the drawer's MIDI list takes, as it says.
pub const KEYS_TEXT: &str = "UP, DOWN: CHOOSE · ENTER: LEARN · DELETE: REMOVE · ESC: CANCEL";

/// Where a learnable parameter's control is: the panel's (its index in [`CONTROLS`]: the
/// GLOBAL page's knobs and the left hand's are the panel's controls too), the strip's tabs, or none
/// (LOCK: the drawer's list alone learns it). GLOBAL, KEYS and LFO select views and do not
/// appear in the learning list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Panel(usize),
    Strip(StripControl),
    Preset(BarTarget),
    Fx(usize),
}

/// Where learnable parameter `i`'s control is.
pub fn place(i: usize) -> Option<Place> {
    if i == MOD_WHEEL {
        return Some(Place::Panel(control_index("mod_wheel")?));
    }
    let id = LEARNABLE.get(i)?.id;
    if let Some(n) = crate::learn::fx_pin(id) {
        return Some(Place::Fx(n));
    }
    Some(match id {
        "previous_preset" => Place::Preset(BarTarget::Prev),
        "next_preset" => Place::Preset(BarTarget::Next),
        "poly" => Place::Strip(StripControl::Poly),
        "unison" => Place::Strip(StripControl::Unison),
        "doubled" => Place::Strip(StripControl::Double),
        "placement" => Place::Strip(StripControl::Placement),
        "auto_gain" => Place::Strip(StripControl::AutoGain),
        // A DIVISION's control is its oscillator's FREQUENCY knob, which turns it while it is
        // timed (decisions.md R-OSC-TIMED).
        "osc2_division" => Place::Panel(control_index("osc2_frequency")?),
        "osc3_division" => Place::Panel(control_index("osc3_frequency")?),
        // An LFO's DIVISION's is its RATE knob (decisions.md R-LFO-PAGE).
        id if id.starts_with("lfo") && id.ends_with("_division") => {
            Place::Panel(control_index(&id.replace("_division", "_rate"))?)
        }
        _ => Place::Panel(control_index(id)?),
    })
}

/// The learnable parameter a place's control operates, `timed` saying which of oscillators 2's
/// and 3's FREQUENCY knobs turn their DIVISION now (R-OSC-TIMED).
pub fn learnable_at(p: Place, timed: [bool; 2]) -> Option<usize> {
    match p {
        Place::Fx(n) => index(crate::lfo::Target::Fx(n).id()),
        Place::Panel(c) if timed[0] && CONTROLS.get(c)?.param == "osc2_frequency" => {
            index("osc2_division")
        }
        Place::Panel(c) if timed[1] && CONTROLS.get(c)?.param == "osc3_frequency" => {
            index("osc3_division")
        }
        Place::Preset(BarTarget::Prev) => index("previous_preset"),
        Place::Preset(BarTarget::Next) => index("next_preset"),
        Place::Preset(_) => None,
        Place::Panel(c) => {
            let id = CONTROLS.get(c)?.param;
            if id == "mod_wheel" {
                Some(MOD_WHEEL)
            } else {
                index(id)
            }
        }
        Place::Strip(s) => index(match s {
            StripControl::Poly => "poly",
            StripControl::Unison => "unison",
            StripControl::Double => "doubled",
            StripControl::Placement => "placement",
            StripControl::AutoGain => "auto_gain",
        }),
    }
}

/// Where a note about a place's control goes: centred above it.
fn note_point(rail: Rail, p: Place) -> (f64, f64) {
    match p {
        Place::Fx(_) => (0.0, 0.0),
        Place::Preset(t) => ca72_panel::strip::centre(rail, ca72_panel::strip::StripTarget::Bar(t)),
        Place::Panel(c) => above(&CONTROLS[c]),
        Place::Strip(s) => {
            let (x0, y0, x1, _) = span(s);
            ((x0 + x1) / 2.0, y0)
        }
    }
}

/// What a menu's items do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Learn,
    Cancel,
    Remove,
    List,
    /// Says why the control cannot be learned; does nothing.
    Not,
    /// QUALITY's (decisions.md R48): its indicator above it (the hamster, the lamp, the
    /// Tesla lamp, through the shutter) shown, or hidden (the panel blank there).
    Show,
    Hide,
    /// The settings' (decisions.md R51): the look the editor is drawn in, its lettering's
    /// size (R-LETTERING), its ticks and titles printed as approved or quiet (R-QUIET-PRINT),
    /// and how the FREQUENCY knobs' dials read (R54), each under its group's head (which does
    /// nothing).
    Look(Skin),
    /// An installed skin pack's look, by its place in the editor's list (decisions.md
    /// R-SKIN-PACKS).
    Pack(usize),
    Lettering(Lettering),
    TicksTitles(TicksTitles),
    Dials(Law),
    Head,
    /// A WAVEFORM switch's (decisions.md R-CONTINUOUS-WAVE): its oscillator's waveform STEPPED
    /// (false) or CONTINUOUS.
    Rect(usize, bool),
    GlideRange(bool),
    StandaloneSettings,
    WhatsNew,
    /// The settings' WINDOW: SIZE…, the SIZE menu wanted (R-SIZE).
    SizeMenu,
    /// The SIZE menu's: the window this wide (logical pixels).
    Size(u32),
    WheelMenu(Option<u8>),
    WheelSource(Source),
    /// An effects' slot's (decisions.md R-FX-DRAWER, R-FX-SAFETY): another effect in its place,
    /// none, the effect tried again...
    Fx(crate::fx_view::FxAction),
    /// The settings' AGENTS (the standalone's; decisions.md R-MCP): AI agents allowed to
    /// operate it through MCP, or not.
    Agents(bool),
    /// The settings' AGENTS: a client's setup copied.
    Setup(Setup),
    /// The settings' HOVER CABLES: the cords from a jack the pointer is on shown (true) or not
    /// from now (decisions.md R-VELOCITY), the row toggling it.
    HoverCables(bool),
    /// The settings' MPE: INPUT & TIMBRE…, the MPE menu wanted (decisions.md R-MPE); its MPE
    /// INPUT and TIMBRE REST.
    MpeMenu,
    MpeInput(crate::expression::Mode),
    TimbreRest(crate::expression::TimbreRest),
    /// Modulation's (decisions.md R-LFO): a source's depth on a knob typed, taken away, a
    /// source given to a knob; an LFO's colour, its EDGES, its waveform STEPPED or CONTINUOUS.
    Mod(ModItem),
}

/// The most columns the settings' menu is set in, to drop below its key (five: the
/// standalone's longer menu, decisions.md R-READABILITY-MINIMUM round 5).
const SETTINGS_COLUMNS: usize = 5;

/// What a modulation item of a menu does (decisions.md R-LFO).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModItem {
    Repin(usize),
    Unpin(usize),
    /// Source's depth on the knob (its control's index) typed, in its knob's entry field.
    Depth(crate::lfo::Source, crate::lfo::Target, usize),
    Remove(crate::lfo::Source, crate::lfo::Target),
    /// The source given to the knob at the default depth (a strip knob's: they lie under the
    /// LFO page, so nothing can be dragged to them).
    Assign(crate::lfo::Source, crate::lfo::Target),
    /// LFO `n`'s phosphor.
    Color(usize, crate::lfo::Phosphor),
    Edges(usize, crate::lfo::Edges),
    /// LFO `n`'s waveform STEPPED (false) or CONTINUOUS.
    Wave(usize, bool),
    /// LFO `n`'s REVERSE turned (R-LFO's corrections).
    Reverse(usize),
    /// Macro `n`'s colour (decisions.md R-MACROS).
    MacroColor(usize, crate::lfo::MacroColor),
    /// Macro `n`'s name typed on its screen.
    Rename(usize),
    /// Every knob the source modulates let go.
    RemoveAll(crate::lfo::Source),
}

/// A row a menu gains: a head, or an item (its text, whether it can be chosen, whether it is
/// marked: the choice it is).
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    Head(String),
    Item(ModItem, String, bool, bool),
}

/// A menu open: for which learnable parameter (none: a control that cannot be learned), what
/// its items do, and how it is drawn.
#[derive(Clone, Debug, PartialEq)]
struct Open {
    learnable: Option<usize>,
    items: Vec<Item>,
    menu: Menu,
}

/// What a press on the panel's menu did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pressed {
    /// Nothing open: the press is the panel's.
    Nothing,
    /// Closed (pressed outside it, or on its title), or an item done: the press is the menu's.
    Done,
    /// MIDI ASSIGNMENTS…: the drawer's MIDI list wanted, this parameter chosen.
    List(Option<usize>),
    /// QUALITY's menu: its indicator shown (true), or hidden.
    Indicator(bool),
    /// A WAVEFORM switch's menu: its oscillator's waveform STEPPED (false) or CONTINUOUS.
    Rect(usize, bool),
    /// GLIDE's menu: NORMAL (false) or LONG (true).
    GlideRange(bool),
    /// The settings' menu: the editor drawn in this look.
    Look(Skin),
    /// The settings' menu: the editor drawn in this installed pack's look.
    Pack(usize),
    /// The settings' menu: the editor's lettering printed at this size.
    Lettering(Lettering),
    /// The settings' menu: the panel's ticks and titles printed so.
    TicksTitles(TicksTitles),
    /// The settings' menu: the FREQUENCY knobs read through this law.
    Dials(Law),
    StandaloneSettings,
    WhatsNew,
    /// The settings' menu: SIZE…, the SIZE menu wanted where the settings' was.
    SizeMenu,
    /// The SIZE menu: the window this wide (logical pixels).
    Size(u32),
    /// None: common sources; 128: CC ranges; otherwise the first CC in a range of sixteen.
    WheelMenu(Option<u8>),
    WheelSource(Source),
    /// An effects' slot's menu: the list to replace its effect, its effect taken out, tried
    /// again...
    Fx(crate::fx_view::FxAction),
    /// The settings' menu: agents allowed (true) or not.
    Agents(bool),
    /// The settings' menu: a client's setup to be copied.
    Setup(Setup),
    /// The settings' menu: the cords from a hovered jack shown (true) or not.
    HoverCables(bool),
    /// The settings' MPE: INPUT & TIMBRE…, the MPE menu wanted where the settings' was; its
    /// choices (decisions.md R-MPE).
    MpeMenu,
    MpeInput(crate::expression::Mode),
    TimbreRest(crate::expression::TimbreRest),
    /// Modulation's (decisions.md R-LFO).
    Mod(ModItem),
}

/// The settings' AGENTS as the menu shows them (the standalone's; decisions.md R-MCP).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentsMenu {
    /// Whether they are allowed: ALLOW marked, else OFF.
    pub allowed: bool,
    /// How they stand, said under the choices (none while they are off).
    pub status: Option<String>,
    /// Whether the bridge is beside the application, so that its setups can be copied.
    pub bridge: bool,
    /// The setup last copied, and whether it reached the clipboard.
    pub copied: Option<(Setup, bool)>,
}

/// A client's setup the settings' AGENTS copies (`crate::agent::setup`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setup {
    /// Claude Code's `claude mcp add` command.
    ClaudeCode,
    /// The `mcpServers` entry of a client's JSON settings (Claude Desktop's and others').
    Json,
}

/// What was last done, said over a control for a while: its lines, the learnable parameter
/// whose control it is, and when.
type Notice = (Vec<(String, Tone)>, usize, Instant);

/// The editor's MIDI Learn.
#[derive(Debug, Default)]
pub struct Learning {
    open: Option<Open>,
    /// What was last done: its lines, the parameter whose control they are said over, and when.
    notice: Option<Notice>,
    /// The drawer shows the MIDI list (not the presets), its chosen row and first shown, and
    /// what was last done (its status line).
    pub list: bool,
    chosen: usize,
    first: usize,
    status: (String, DrawerTone),
    /// The window should take the keyboard (a menu opened, or learning begun: Escape is ours),
    /// asked once.
    pub take_keys: bool,
    /// The rail the editor shows (the standalone's has FX, its arrows elsewhere): where a
    /// note about PREVIOUS or NEXT PRESET goes.
    pub rail: Rail,
    /// Which of oscillators 2's and 3's FREQUENCY knobs turn their DIVISION now (decisions.md
    /// R-OSC-TIMED): what a menu opened on one learns.
    pub timed: [bool; 2],
    /// Which oscillators' waveforms are CONTINUOUS (decisions.md R-CONTINUOUS-WAVE): the one its
    /// WAVEFORM switch's menu marks.
    pub variable: [bool; 3],
    /// Which LFOs' RATE knobs turn their DIVISION now (decisions.md R-LFO-PAGE).
    pub lfo_timed: [bool; 4],
    /// Which of GLIDE's ranges its knob's menu marks (decisions.md R-GLIDE-LONG).
    pub glide_long: bool,
    pub fx_pins: Vec<crate::fx_view::PinInfo>,
    /// Where the MIDI map mode found a learnable whose control has no place of its own here
    /// (the arpeggiator's ON, HOLD and DIRECTION: R-MIDI-MAP): its note goes above that.
    pub found_at: Option<(usize, (f64, f64))>,
}

/// A parameter's name.
fn label(i: usize) -> &'static str {
    if i == MOD_WHEEL {
        return "MOD WHEEL SOURCE";
    }
    LEARNABLE.get(i).map_or("", |l| l.label)
}

/// What learnable `i` waits for, `notes` while its arming takes a note (a switch's in the MIDI map
/// mode: R-MIDI-MAP).
fn instruction(i: usize, notes: bool) -> &'static str {
    if crate::learn::action(i).is_some() {
        "PRESS A NOTE OR CC PAD ON YOUR MIDI DEVICE"
    } else if notes {
        "MOVE A CONTROLLER OR PRESS A NOTE ON YOUR MIDI DEVICE"
    } else {
        "MOVE A CONTROLLER ON YOUR MIDI DEVICE"
    }
}

/// What a control that cannot be learned says in its menu.
fn not_learnable(t: Target) -> Option<String> {
    Some(match t {
        Target::Control(i) | Target::Legend(i, _) => match CONTROLS.get(i)?.param {
            "pitch_wheel" => "PITCH: MIDI PITCH BEND MOVES IT".into(),
            "quality" => "QUALITY: SET PER PLUGIN INSTANCE".into(),
            _ => return None,
        },
        Target::Power => "POWER: THE HOST'S BYPASS".into(),
        Target::Grip => return None,
    })
}

impl Learning {
    fn listed(&self) -> usize {
        if self.rail == Rail::Effects {
            LEARNABLE.len()
        } else {
            LEARNABLE.len() - crate::lfo::FX_PINS
        }
    }
    fn label(&self, i: usize) -> String {
        if let Some(n) = LEARNABLE.get(i).and_then(|l| crate::learn::fx_pin(l.id))
            && let Some(p) = self.fx_pins.iter().find(|p| p.target == n)
        {
            return format!("{}, {}", p.name, p.effect).to_uppercase();
        }
        label(i).into()
    }

    fn note_point(&self, p: Place) -> (f64, f64) {
        if let Place::Fx(n) = p
            && let Some(pin) = self.fx_pins.iter().find(|p| p.target == n)
            && let Some(place) = self
                .fx_pins
                .iter()
                .filter(|p| p.slot == pin.slot)
                .position(|p| p.target == n)
        {
            let (x, y, w, _) = ca72_panel::strip::fx::pin_screen(pin.slot, place);
            return (x + w / 2.0, y - 6.0);
        }
        note_point(self.rail, p)
    }

    /// Where a note about learnable `i` goes: above its control, else where the MIDI map mode
    /// found it (R-MIDI-MAP).
    fn point_of(&self, i: usize) -> Option<(f64, f64)> {
        place(i)
            .map(|p| self.note_point(p))
            .or_else(|| self.found_at.filter(|(j, _)| *j == i).map(|(_, at)| at))
    }

    /// The source heads and UNPIN first, MIDI Learn after them (R-FX-PARAMS).
    pub fn pin_menu_title(&mut self, title: String) {
        let Some(open) = &mut self.open else {
            return;
        };
        open.menu.title = title;
        let first = open
            .items
            .iter()
            .position(|item| matches!(item, Item::Head | Item::Mod(_)))
            .unwrap_or(open.items.len());
        open.items.rotate_left(first);
        open.menu.items.rotate_left(first);
        let len = open.items.len();
        for i in open
            .menu
            .heads
            .iter_mut()
            .chain(open.menu.marked.iter_mut())
        {
            *i = (*i + len - first) % len;
        }
    }

    /// The learnable parameter `p`'s control operates now: a timed FREQUENCY knob's DIVISION
    /// (R-OSC-TIMED), and an LFO's RATE while timed its DIVISION (decisions.md R-LFO-PAGE).
    pub fn learnable_now(&self, p: Place) -> Option<usize> {
        let learnable = learnable_at(p, self.timed);
        match p {
            Place::Panel(c) => CONTROLS
                .get(c)
                .and_then(|c| c.param.strip_prefix("lfo"))
                .and_then(|r| r.strip_suffix("_rate"))
                .and_then(|n| n.parse::<usize>().ok())
                .filter(|n| (1..=4).contains(n) && self.lfo_timed[n - 1])
                .and_then(|n| index(&format!("lfo{n}_division")))
                .or(learnable),
            _ => learnable,
        }
    }

    /// Whether the editor should keep the keyboard: learning, or a menu open.
    pub fn holds_keys(&self, map: &MidiMap) -> bool {
        self.open.is_some() || map.armed().is_some()
    }

    /// Whether a menu is open.
    pub fn menu_open(&self) -> bool {
        self.open.is_some()
    }

    /// The menu open, as drawn.
    pub fn menu(&self) -> Option<&Menu> {
        self.open.as_ref().map(|o| &o.menu)
    }

    /// Keep a floating menu inside the editor's current viewport, including its compact view.
    pub fn fit_menu(&mut self, fonts: &Fonts, area: [f64; 4]) {
        if let Some(open) = &mut self.open {
            let menu = &open.menu;
            let (w, h) = menu.extent(fonts);
            if menu.x < area[0] || menu.y < area[1] || menu.x + w > area[2] || menu.y + h > area[3]
            {
                open.menu = menu.clone().placed(fonts, menu.x, menu.y, area);
            }
        }
    }

    /// A control's menu opened at (`x`, `y`) on the panel (drawing units) for `place`'s control,
    /// or for a control that cannot be learned (`not`: what it says); `size` the lettering's.
    pub fn open_menu(
        &mut self,
        fonts: &Fonts,
        map: &MidiMap,
        place: Option<Place>,
        not: Option<Target>,
        (x, y): (f64, f64),
        size: f64,
    ) {
        self.open_menu_with(fonts, map, place, not, (x, y), size, Vec::new());
    }

    /// The same, `rows` under its MIDI Learn's (decisions.md R-LFO: each source on the knob a
    /// head with DEPTH… and REMOVE; a strip knob's LFO head; an LFO WAVEFORM's modes). A control
    /// that cannot be learned has its rows alone, under its name.
    #[allow(clippy::too_many_arguments)]
    pub fn open_menu_with(
        &mut self,
        fonts: &Fonts,
        map: &MidiMap,
        place: Option<Place>,
        not: Option<Target>,
        (x, y): (f64, f64),
        size: f64,
        rows: Vec<Row>,
    ) {
        let learnable = place.and_then(|p| self.learnable_now(p));
        let (title, items) = match learnable {
            Some(MOD_WHEEL) => (
                format!("MOD WHEEL SOURCE · {}", map.wheel_source().text()),
                vec![
                    if map.armed() == Some(MOD_WHEEL) {
                        (Item::Cancel, "CANCEL MIDI LEARN")
                    } else {
                        (Item::Learn, "MIDI LEARN")
                    },
                    (Item::WheelMenu(None), "MOD WHEEL SOURCE…"),
                    (Item::List, "MIDI ASSIGNMENTS…"),
                ],
            ),
            Some(i) => {
                let cc = map
                    .binding(i)
                    .map_or("NO MIDI CONTROLLER".to_owned(), Binding::text);
                let learn = if map.armed() == Some(i) {
                    (Item::Cancel, "CANCEL MIDI LEARN")
                } else {
                    (Item::Learn, "MIDI LEARN")
                };
                let mut items = vec![
                    learn,
                    (Item::Remove, "REMOVE MIDI ASSIGNMENT"),
                    (Item::List, "MIDI ASSIGNMENTS…"),
                ];
                // A WAVEFORM switch's: its waveform STEPPED or CONTINUOUS, under their head
                // (decisions.md R-CONTINUOUS-WAVE; the owner: "you right click on the knob and then
                // you can switch the mode").
                if let Some(n) = ["osc1_waveform", "osc2_waveform", "osc3_waveform"]
                    .iter()
                    .position(|w| LEARNABLE.get(i).is_some_and(|l| l.id == *w))
                {
                    items.extend([
                        (Item::Head, "WAVEFORM"),
                        (Item::Rect(n, false), "STEPPED"),
                        (Item::Rect(n, true), "CONTINUOUS"),
                    ]);
                }
                if LEARNABLE[i].id == "glide" {
                    items.extend([
                        (Item::Head, "GLIDE RANGE"),
                        (Item::GlideRange(false), "NORMAL"),
                        (
                            Item::GlideRange(true),
                            if fonts.covers("×") {
                                "LONG (×5)"
                            } else {
                                "LONG (x5)"
                            },
                        ),
                    ]);
                }
                (format!("{} · {cc}", self.label(i)), items)
            }
            None => match not.and_then(not_learnable) {
                Some(t) => (
                    t,
                    vec![
                        (Item::Not, "NOT LEARNED BY MIDI LEARN"),
                        (Item::List, "MIDI ASSIGNMENTS…"),
                    ],
                ),
                None => {
                    self.open = None;
                    return;
                }
            },
        };
        let assigned = learnable.is_some_and(|i| map.binding(i).is_some());
        let mut items: Vec<(Item, String)> = items
            .into_iter()
            .map(|(it, t)| (it, t.to_owned()))
            .collect();
        let mut extra_enabled: Vec<bool> = Vec::new();
        let mut extra_marked: Vec<bool> = Vec::new();
        let first_extra = items.len();
        for r in rows {
            match r {
                Row::Head(t) => {
                    items.push((Item::Head, t));
                    extra_enabled.push(false);
                    extra_marked.push(false);
                }
                Row::Item(m, t, on, mark) => {
                    items.push((Item::Mod(m), t));
                    extra_enabled.push(on);
                    extra_marked.push(mark);
                }
            }
        }
        let menu = Menu {
            x,
            y,
            size,
            title,
            items: items
                .iter()
                .enumerate()
                .map(|(k, (it, t))| {
                    let enabled = match it {
                        _ if k >= first_extra => extra_enabled[k - first_extra],
                        Item::Remove => assigned,
                        Item::Not | Item::Head => false,
                        // (The mode it is in marked, not chosen again.)
                        Item::Rect(n, v) => self.variable[*n] != *v,
                        Item::GlideRange(v) => self.glide_long != *v,
                        _ => true,
                    };
                    (t.clone(), enabled)
                })
                .collect(),
            heads: items
                .iter()
                .enumerate()
                .filter(|(_, (it, _))| *it == Item::Head)
                .map(|(k, _)| k)
                .collect(),
            marked: items
                .iter()
                .enumerate()
                .filter(|(k, (it, _))| {
                    matches!(it, Item::Rect(n, v) if self.variable[*n] == *v)
                        || matches!(it, Item::GlideRange(v) if self.glide_long == *v)
                        || (*k >= first_extra && extra_marked[*k - first_extra])
                })
                .map(|(k, _)| k)
                .collect(),
            hover: None,
            columns: Vec::new(),
        }
        .placed(fonts, x, y, PANEL);
        self.open = Some(Open {
            learnable,
            items: items.into_iter().map(|(it, _)| it).collect(),
            menu,
        });
        self.take_keys = true;
    }

    /// A menu of `rows` alone under `title` (decisions.md R-LFO: an LFO's colour and EDGES, what
    /// it modulates), opened at (`x`, `y`).
    pub fn open_rows_menu(
        &mut self,
        fonts: &Fonts,
        (x, y): (f64, f64),
        size: f64,
        title: String,
        rows: Vec<Row>,
    ) {
        let mut items = Vec::new();
        let mut shown = Vec::new();
        let (mut heads, mut marked) = (Vec::new(), Vec::new());
        for (k, r) in rows.into_iter().enumerate() {
            match r {
                Row::Head(t) => {
                    heads.push(k);
                    items.push(Item::Head);
                    shown.push((t, false));
                }
                Row::Item(m, t, on, mark) => {
                    if mark {
                        marked.push(k);
                    }
                    items.push(Item::Mod(m));
                    shown.push((t, on));
                }
            }
        }
        let menu = Menu {
            x,
            y,
            size,
            title,
            items: shown,
            heads,
            marked,
            hover: None,
            columns: Vec::new(),
        }
        .placed(fonts, x, y, PANEL);
        self.open = Some(Open {
            learnable: None,
            items,
            menu,
        });
        self.take_keys = true;
    }

    /// QUALITY's menu (decisions.md R48; a right click on its toggle, the owner: "default
    /// mode is to show the quality indicators (tesla coil, hamster, etc.), but they can be
    /// hidden as an option. This can be selectable by right clicking the toggle switch."),
    /// opened at (`x`, `y`): why MIDI Learn does not learn it, as its title, then its indicator
    /// shown or hidden (`shown`: the one it is, dim), and the MIDI list, as a control's that
    /// cannot be learned has.
    pub fn open_quality_menu(&mut self, fonts: &Fonts, (x, y): (f64, f64), size: f64, shown: bool) {
        let items = [
            (Item::Show, "SHOW ITS INDICATOR", !shown),
            (Item::Hide, "HIDE ITS INDICATOR", shown),
            (Item::Not, "NOT LEARNED BY MIDI LEARN", false),
            (Item::List, "MIDI ASSIGNMENTS\u{2026}", true),
        ];
        let menu = Menu {
            x,
            y,
            size,
            title: "QUALITY: SET PER PLUGIN INSTANCE".to_owned(),
            items: items
                .iter()
                .map(|(_, t, on)| ((*t).to_owned(), *on))
                .collect(),
            heads: Vec::new(),
            marked: Vec::new(),
            hover: None,
            columns: Vec::new(),
        }
        .placed(fonts, x, y, PANEL);
        self.open = Some(Open {
            learnable: None,
            items: items.into_iter().map(|(it, ..)| it).collect(),
            menu,
        });
        self.take_keys = true;
    }

    /// The settings' menu (decisions.md R51; the rail's settings key, the owner: "We can let
    /// people change their skin there"), opened with its top right corner at (`x`, `y`): under
    /// LOOK, the looks the editor can be drawn in on this computer (`skin`: the one it is,
    /// marked); under LETTERING, its lettering's sizes (`lettering`, marked: R-LETTERING); under
    /// TICKS & TITLES, its ticks and titles as approved or quiet (`print`, marked:
    /// R-QUIET-PRINT); under FREQUENCY DIALS, how oscillators 2 and 3's FREQUENCY knobs read (`law`, marked:
    /// decisions.md R54). Under LOOK, after the built-in two, each installed skin pack (`packs`:
    /// its name, and whether it can be used; `pack` the one the editor is drawn in, if any); under
    /// MOD WHEEL SOURCE, what moves the MOD wheel (`source`: R-MODWHEEL-SOURCE); in the
    /// standalone, under AGENTS, whether AI agents may operate it (`agents`, marked; none in a
    /// host: decisions.md R-MCP), how they stand, and the setups for their clients to copy; under
    /// STANDALONE, its audio not real-time (`realtime`'s row, if any: decisions.md R-RTKIT). The
    /// title always names this build's version, independent of the update check; opening this
    /// menu does not start a check.
    pub fn open_settings_menu(
        &mut self,
        fonts: &Fonts,
        (x, y): (f64, f64),
        size: f64,
        (skin, (lettering, print), law, source, agents, cables): (
            Skin,
            (Lettering, TicksTitles),
            Law,
            Source,
            Option<AgentsMenu>,
            bool,
        ),
        (packs, pack): (&[(String, bool)], Option<usize>),
        realtime: Option<&str>,
    ) {
        let mut items: Vec<(Item, String)> = vec![
            (Item::Head, "LOOK".into()),
            (Item::Look(Skin::Worn), "REALISTIC".into()),
            (Item::Look(Skin::Drawn), "CLASSIC".into()),
        ];
        let mut unusable = Vec::new();
        for (i, (name, usable)) in packs.iter().enumerate() {
            if !usable {
                unusable.push(items.len());
            }
            items.push((
                Item::Pack(i),
                if *usable {
                    name.clone()
                } else {
                    format!("{name} (CANNOT BE USED)")
                },
            ));
        }
        let rest: Vec<(Item, &str)> = vec![
            (Item::Head, "LETTERING"),
            (Item::Lettering(Lettering::Standard), "STANDARD"),
            (Item::Lettering(Lettering::Large), "LARGE"),
            (Item::Head, "TICKS & TITLES"),
            (Item::TicksTitles(TicksTitles::Standard), "STANDARD"),
            (Item::TicksTitles(TicksTitles::Quiet), "QUIET"),
            (Item::Head, "FREQUENCY DIALS"),
            (Item::Dials(Law::Semitones), "SEMITONES"),
            (Item::Dials(Law::Hardware), "HARDWARE"),
            (Item::Head, "WINDOW"),
            (Item::SizeMenu, "SIZE\u{2026}"),
            (Item::WhatsNew, "WHAT'S NEW…"),
            // (A row that toggles it, the menu already taller than the window: R-VELOCITY.)
            (
                Item::HoverCables(!cables),
                if cables {
                    "HOVER CABLES: ON"
                } else {
                    "HOVER CABLES: OFF"
                },
            ),
            (Item::Head, "MOD WHEEL SOURCE"),
            (Item::WheelMenu(None), "CHOOSE SOURCE…"),
            // (Its two settings in a menu of their own: this one is already taller than the
            // window at its opening size, decisions.md R-MPE.)
            (Item::Head, "MPE"),
            (Item::MpeMenu, "INPUT & TIMBRE…"),
        ];
        items.extend(rest.into_iter().map(|(it, t)| (it, t.to_owned())));
        #[cfg(feature = "standalone-production")]
        if crate::standalone_settings::available() {
            items.push((Item::Head, "STANDALONE".into()));
            items.push((Item::StandaloneSettings, "AUDIO & MIDI…".into()));
            // (Its audio not real-time, as AGENTS says how they stand: decisions.md R-RTKIT.)
            if let Some(row) = realtime {
                items.push((Item::Not, row.to_owned()));
            }
        }
        #[cfg(not(feature = "standalone-production"))]
        let _ = realtime;
        if let Some(a) = &agents {
            items.push((Item::Head, "AGENTS".into()));
            items.push((Item::Agents(true), "ALLOW".into()));
            items.push((Item::Agents(false), "OFF".into()));
            if let Some(status) = &a.status {
                items.push((Item::Not, status.clone()));
            }
            if a.bridge {
                for (setup, what, paste) in [
                    (Setup::ClaudeCode, "CLAUDE CODE", "PASTE IT IN A TERMINAL"),
                    (
                        Setup::Json,
                        "JSON FOR OTHER APPS",
                        "ADD IT TO THE APP'S MCP SERVERS",
                    ),
                ] {
                    let text = match a.copied {
                        Some((s, true)) if s == setup => format!("COPIED: {paste}"),
                        Some((s, false)) if s == setup => "COULD NOT COPY TO THE CLIPBOARD".into(),
                        _ => format!("COPY SETUP: {what}"),
                    };
                    items.push((Item::Setup(setup), text));
                }
            } else {
                items.push((Item::Not, "SETUP: NO BRIDGE BESIDE THE APP".into()));
            }
        }
        let current = |it: &Item| {
            let look = match pack {
                Some(p) => *it == Item::Pack(p),
                None => *it == Item::Look(skin),
            };
            look || *it == Item::Lettering(lettering)
                || *it == Item::TicksTitles(print)
                || *it == Item::Dials(law)
                || agents
                    .as_ref()
                    .is_some_and(|a| *it == Item::Agents(a.allowed))
        };
        let mut menu = Menu {
            x,
            y,
            size,
            title: format!("SETTINGS · CA-72 {}", crate::update::VERSION),
            items: items
                .iter()
                .enumerate()
                .map(|(i, (it, t))| {
                    let choosable = matches!(
                        it,
                        Item::Look(_)
                            | Item::Pack(_)
                            | Item::Lettering(_)
                            | Item::TicksTitles(_)
                            | Item::Dials(_)
                            | Item::Agents(_)
                            | Item::Setup(_)
                            | Item::HoverCables(_)
                            | Item::StandaloneSettings
                            | Item::SizeMenu
                            | Item::WhatsNew
                            | Item::MpeMenu
                    ) && !current(it)
                        && !unusable.contains(&i);
                    let choosable = choosable || matches!(it, Item::WheelMenu(_));
                    let text = if matches!(it, Item::WheelMenu(_)) {
                        format!("{}…", source.text())
                    } else {
                        t.clone()
                    };
                    (text, choosable)
                })
                .collect(),
            heads: items
                .iter()
                .enumerate()
                .filter(|(_, (it, _))| *it == Item::Head)
                .map(|(i, _)| i)
                .collect(),
            marked: items
                .iter()
                .enumerate()
                .filter(|(_, (it, _))| current(it))
                .map(|(i, _)| i)
                .collect(),
            hover: None,
            columns: Vec::new(),
        };
        // Below its key, in as many columns as that takes (taller than the room under the rail
        // in one column at 4K's FIT TO SCREEN and SIZE's 100 %, it rose over the rail and its
        // key: decisions.md R-READABILITY-MINIMUM), up to five and no wider than the room left of
        // the key, its groups spread over them as evenly as they go, its lettering and rows as
        // they are; where none fit (LARGE in the smallest window, the standalone's longer menu
        // in the smaller ones), as little smaller as fits.
        menu = menu.in_columns(fonts, (x - PANEL[0], PANEL[3] - y), SETTINGS_COLUMNS);
        let (w, _) = menu.extent(fonts);
        menu = menu.placed(fonts, x - w, y, PANEL);
        self.open = Some(Open {
            learnable: None,
            items: items.into_iter().map(|(it, _)| it).collect(),
            menu,
        });
        self.take_keys = true;
    }

    /// The MPE menu (decisions.md R-MPE; the settings' MPE: INPUT & TIMBRE…), opened with its top
    /// right corner at (`x`, `y`) as the settings' is: under MPE INPUT, where the zones come from
    /// (`mode`); under TIMBRE REST, where CC 74 and a host's brightness rest (`rest`); the ones
    /// they are marked, the others to be chosen. This computer's, as the settings' are.
    pub fn open_mpe_menu(
        &mut self,
        fonts: &Fonts,
        (x, y): (f64, f64),
        size: f64,
        (mode, rest): (crate::expression::Mode, crate::expression::TimbreRest),
    ) {
        use crate::expression::{Mode, TimbreRest};
        let items: Vec<(Item, &str)> = vec![
            (Item::Head, "MPE INPUT"),
            (Item::MpeInput(Mode::Auto), "AUTO"),
            (Item::MpeInput(Mode::Lower), "LOWER ZONE"),
            (Item::MpeInput(Mode::Off), "OFF"),
            (Item::Head, "TIMBRE REST"),
            (Item::TimbreRest(TimbreRest::Centred), "CENTERED ON 64"),
            (Item::TimbreRest(TimbreRest::Zero), "FROM 0"),
            (
                Item::TimbreRest(TimbreRest::NoteStart),
                "FROM THE NOTE'S START",
            ),
        ];
        let current = |it: &Item| *it == Item::MpeInput(mode) || *it == Item::TimbreRest(rest);
        let mut menu = Menu {
            x,
            y,
            size,
            title: "MPE".into(),
            items: items
                .iter()
                .map(|(it, t)| ((*t).to_owned(), *it != Item::Head && !current(it)))
                .collect(),
            heads: items
                .iter()
                .enumerate()
                .filter(|(_, (it, _))| *it == Item::Head)
                .map(|(i, _)| i)
                .collect(),
            marked: items
                .iter()
                .enumerate()
                .filter(|(_, (it, _))| current(it))
                .map(|(i, _)| i)
                .collect(),
            hover: None,
            columns: Vec::new(),
        };
        let (w, _) = menu.extent(fonts);
        menu = menu.placed(fonts, x - w, y, PANEL);
        self.open = Some(Open {
            learnable: None,
            items: items.into_iter().map(|(it, _)| it).collect(),
            menu,
        });
        self.take_keys = true;
    }

    /// A short list first; OTHER… opens eight ranges, each at most sixteen controllers.
    pub fn open_wheel_menu(
        &mut self,
        fonts: &Fonts,
        (x, y): (f64, f64),
        size: f64,
        source: Source,
        range: Option<u8>,
    ) {
        let items: Vec<(Item, String, bool)> = match range {
            None => {
                let mut items: Vec<_> = [
                    (Source::Cc(1), "CC 1 · MODULATION WHEEL"),
                    (Source::Cc(2), "CC 2 · BREATH"),
                    (Source::Cc(4), "CC 4 · FOOT"),
                    (Source::Cc(11), "CC 11 · EXPRESSION"),
                    (Source::Cc(74), "CC 74 · TIMBRE / MPE Y"),
                    (Source::ChannelPressure, "CHANNEL AFTERTOUCH"),
                    (Source::PolyPressure, "POLYPHONIC AFTERTOUCH · MAX HELD"),
                ]
                .into_iter()
                .map(|(s, t)| (Item::WheelSource(s), t.into(), s != source))
                .collect();
                if matches!(source, Source::Cc(cc) if ![1, 2, 4, 11, 74].contains(&cc)) {
                    items.push((Item::WheelSource(source), source.text(), false));
                }
                items.push((Item::WheelMenu(Some(128)), "OTHER…".into(), true));
                items
            }
            Some(128) => (0..128)
                .step_by(16)
                .map(|cc| {
                    (
                        Item::WheelMenu(Some(cc)),
                        format!("CC {cc}–{}…", cc + 15),
                        true,
                    )
                })
                .collect(),
            Some(first) => (first..=first.saturating_add(15).min(127))
                .map(|cc| {
                    let allowed = Source::cc(cc).is_some();
                    let text = match crate::learn::reserved(cc) {
                        Some(why) => format!("CC {cc} · {}", why.to_uppercase()),
                        None => format!("CC {cc}"),
                    };
                    (
                        Item::WheelSource(Source::Cc(cc)),
                        text,
                        allowed && source != Source::Cc(cc),
                    )
                })
                .collect(),
        };
        let mut menu = Menu {
            x,
            y,
            size,
            title: "MOD WHEEL SOURCE · ALL CHANNELS".into(),
            items: items
                .iter()
                .map(|(_, text, on)| (text.clone(), *on))
                .collect(),
            heads: Vec::new(),
            marked: items
                .iter()
                .enumerate()
                .filter_map(|(i, (item, _, _))| (*item == Item::WheelSource(source)).then_some(i))
                .collect(),
            hover: None,
            columns: Vec::new(),
        };
        let (w, _) = menu.extent(fonts);
        menu = menu.placed(fonts, x - w, y, PANEL);
        self.open = Some(Open {
            learnable: None,
            items: items.into_iter().map(|(item, _, _)| item).collect(),
            menu,
        });
        self.take_keys = true;
    }

    /// The SIZE menu (R-SIZE; the settings' WINDOW: SIZE…), opened with its top right corner
    /// at (`x`, `y`) as the settings' is: `sizes`, each its label, the window's width (logical
    /// pixels) and whether it fits the screen; the one the window is (`width`) marked, it and
    /// those that do not fit not to be chosen.
    pub fn open_size_menu(
        &mut self,
        fonts: &Fonts,
        (x, y): (f64, f64),
        size: f64,
        sizes: &[(&str, u32, bool)],
        width: u32,
    ) {
        let mut menu = Menu {
            x,
            y,
            size,
            title: "SIZE".to_owned(),
            items: sizes
                .iter()
                .map(|&(t, w, fits)| (t.to_owned(), fits && w != width))
                .collect(),
            heads: Vec::new(),
            marked: sizes
                .iter()
                .enumerate()
                .filter(|(_, (_, w, _))| *w == width)
                .map(|(i, _)| i)
                .collect(),
            hover: None,
            columns: Vec::new(),
        };
        let (w, _) = menu.extent(fonts);
        menu = menu.placed(fonts, x - w, y, PANEL);
        self.open = Some(Open {
            learnable: None,
            items: sizes.iter().map(|&(_, w, _)| Item::Size(w)).collect(),
            menu,
        });
        self.take_keys = true;
    }

    /// An effects' slot's menu (decisions.md R-FX-DRAWER; the owner's FX-A: "right-click to
    /// replace/remove"), opened at (`x`, `y`): its effect's name as its title, then `items`
    /// (what the effect's state offers, R-FX-SAFETY, then REPLACE… and REMOVE).
    pub fn open_fx_menu(
        &mut self,
        fonts: &Fonts,
        (x, y): (f64, f64),
        size: f64,
        name: &str,
        items: &[(crate::fx_view::FxAction, &str)],
    ) {
        let items: Vec<(Item, &str)> = items.iter().map(|(a, t)| (Item::Fx(*a), *t)).collect();
        let menu = Menu {
            x,
            y,
            size,
            title: name.to_uppercase(),
            items: items.iter().map(|(_, t)| ((*t).to_owned(), true)).collect(),
            heads: Vec::new(),
            marked: Vec::new(),
            hover: None,
            columns: Vec::new(),
        }
        .placed(fonts, x, y, PANEL);
        self.open = Some(Open {
            learnable: None,
            items: items.into_iter().map(|(it, _)| it).collect(),
            menu,
        });
        self.take_keys = true;
    }

    /// Whether (`x`, `y`) is over the open menu (the keyboard under it does not answer the pointer
    /// there).
    pub fn over_menu(&self, fonts: &Fonts, (x, y): (f64, f64)) -> bool {
        self.open
            .as_ref()
            .is_some_and(|o| o.menu.hit(fonts, x, y).is_some())
    }

    /// The menu closed.
    pub fn close_menu(&mut self) {
        self.open = None;
    }

    /// The pointer over the panel at (`x`, `y`): the menu's item under it lit.
    pub fn hover(&mut self, fonts: &Fonts, (x, y): (f64, f64)) {
        if let Some(o) = &mut self.open {
            o.menu.hover = o.menu.hit(fonts, x, y).flatten();
        }
    }

    /// A press at (`x`, `y`) on the panel while a menu may be open: its item done, or the menu
    /// closed by a press elsewhere (which does nothing else).
    pub fn press(&mut self, fonts: &Fonts, map: &MidiMap, (x, y): (f64, f64)) -> Pressed {
        let Some(o) = self.open.take() else {
            return Pressed::Nothing;
        };
        let Some(Some(k)) = o.menu.hit(fonts, x, y) else {
            return Pressed::Done;
        };
        if !o.menu.items.get(k).is_some_and(|(_, enabled)| *enabled) {
            // (An item that cannot be chosen leaves the menu open.)
            self.open = Some(o);
            return Pressed::Done;
        }
        match (o.items.get(k), o.learnable) {
            (Some(Item::Learn), Some(i)) => self.learn(map, i),
            (Some(Item::Cancel), _) => self.cancel(map),
            (Some(Item::Remove), Some(i)) => self.remove(map, i),
            (Some(Item::List), l) => return Pressed::List(l.filter(|&i| i != MOD_WHEEL)),
            (Some(Item::Rect(n, variable)), _) => return Pressed::Rect(*n, *variable),
            (Some(Item::GlideRange(long)), _) => return Pressed::GlideRange(*long),
            (Some(Item::Show), _) => return Pressed::Indicator(true),
            (Some(Item::Hide), _) => return Pressed::Indicator(false),
            (Some(Item::StandaloneSettings), _) => return Pressed::StandaloneSettings,
            (Some(Item::WhatsNew), _) => return Pressed::WhatsNew,
            (Some(Item::Look(s)), _) => return Pressed::Look(*s),
            (Some(Item::Pack(i)), _) => return Pressed::Pack(*i),
            (Some(Item::Lettering(l)), _) => return Pressed::Lettering(*l),
            (Some(Item::TicksTitles(p)), _) => return Pressed::TicksTitles(*p),
            (Some(Item::SizeMenu), _) => return Pressed::SizeMenu,
            (Some(Item::Size(w)), _) => return Pressed::Size(*w),
            (Some(Item::Dials(l)), _) => return Pressed::Dials(*l),
            (Some(Item::WheelMenu(range)), _) => return Pressed::WheelMenu(*range),
            (Some(Item::WheelSource(source)), _) => return Pressed::WheelSource(*source),
            (Some(Item::Fx(a)), _) => return Pressed::Fx(*a),
            (Some(Item::Agents(on)), _) => return Pressed::Agents(*on),
            (Some(Item::Setup(s)), _) => return Pressed::Setup(*s),
            (Some(Item::HoverCables(on)), _) => return Pressed::HoverCables(*on),
            (Some(Item::MpeMenu), _) => return Pressed::MpeMenu,
            (Some(Item::MpeInput(m)), _) => return Pressed::MpeInput(*m),
            (Some(Item::TimbreRest(r)), _) => return Pressed::TimbreRest(*r),
            (Some(Item::Mod(m)), _) => return Pressed::Mod(*m),
            _ => {}
        }
        Pressed::Done
    }

    /// Learnable parameter `i` learned: the next controller moved is its own (another being
    /// learned is no longer).
    pub fn learn(&mut self, map: &MidiMap, i: usize) {
        map.arm(i);
        self.begun(map, i);
    }

    /// The same from the MIDI map mode (R-MIDI-MAP): a switch takes a note as well.
    pub fn learn_with_notes(&mut self, map: &MidiMap, i: usize) {
        map.arm_with_notes(i);
        self.begun(map, i);
    }

    fn begun(&mut self, map: &MidiMap, i: usize) {
        self.notice = None;
        if i < LEARNABLE.len() {
            self.chosen = i;
        }
        self.say(
            format!("{}: {}", self.label(i), instruction(i, map.notes_armed())),
            DrawerTone::News,
        );
        self.take_keys = true;
    }

    /// Learning cancelled; the assignments as they were.
    pub fn cancel(&mut self, map: &MidiMap) {
        if let Some(i) = map.armed() {
            self.say(
                format!("{}: MIDI LEARN CANCELLED.", self.label(i)),
                DrawerTone::Dim,
            );
        }
        map.cancel();
    }

    /// Learnable parameter `i`'s controller removed (the sound as it is).
    pub fn remove(&mut self, map: &MidiMap, i: usize) {
        if let Some(cc) = map.remove(i) {
            let line = format!("{}: {} REMOVED", self.label(i), cc.text());
            self.say(format!("{line}."), DrawerTone::Dim);
            self.notice = Some((vec![(line, Tone::Plain)], i, Instant::now()));
            if let Err(error) = map.keep_actions(&[i]) {
                self.say(
                    format!("MIDI ASSIGNMENT NOT SAVED: {error}"),
                    DrawerTone::Trouble,
                );
            }
        }
    }

    fn say(&mut self, text: String, tone: DrawerTone) {
        self.status = (text, tone);
    }

    /// The editor closing: a controller the audio thread has already caught is assigned (the
    /// message that came is the assignment), then learning ends.
    pub fn close(&mut self, map: &MidiMap) -> Option<Assigned> {
        let caught = self.poll(map);
        map.cancel();
        self.open = None;
        caught
    }

    /// What the audio thread caught for the parameter being learned, assigned, and said.
    pub fn poll(&mut self, map: &MidiMap) -> Option<Assigned> {
        let a = map.poll()?;
        let controller = match a.cc {
            Binding::Cc(c) if a.param == MOD_WHEEL => format!("CC {} · ALL CHANNELS", c.cc),
            input => input.text(),
        };
        let head = format!("{}: {}", self.label(a.param), controller);
        let mut lines = vec![(head.clone(), Tone::Accent)];
        let mut status = head;
        if let Some(d) = a.displaced {
            let t = format!("TAKEN FROM {}, WHICH HAS NONE NOW", self.label(d));
            status = format!("{status}, {t}");
            lines.push((t, Tone::Plain));
        }
        if let Some(r) = a.replaced {
            let t = format!("IN PLACE OF {}", r.text());
            status = format!("{status}, {t}");
            lines.push((t, Tone::Dim));
        }
        self.say(format!("{status}."), DrawerTone::News);
        self.notice = Some((lines, a.param, Instant::now()));
        if let Err(error) = map.keep_assignment(&a) {
            self.say(
                format!("MIDI ASSIGNMENT NOT SAVED: {error}"),
                DrawerTone::Trouble,
            );
        }
        Some(a)
    }

    /// The note over the control being learned (or the one last done, for a while), and the
    /// panel's control to ring and the strip's.
    pub fn scene(
        &mut self,
        map: &MidiMap,
        size: f64,
    ) -> (
        Option<Note>,
        Option<usize>,
        Option<StripControl>,
        Option<Menu>,
    ) {
        if self
            .notice
            .as_ref()
            .is_some_and(|(_, _, t)| t.elapsed() > NOTICE)
        {
            self.notice = None;
        }
        let armed = map.armed();
        let ring = armed.and_then(place);
        let (panel, strip) = match ring {
            Some(Place::Panel(c)) => (Some(c), None),
            Some(Place::Strip(s)) => (None, Some(s)),
            Some(Place::Fx(_) | Place::Preset(_)) | None => (None, None),
        };
        let notes = map.notes_armed();
        let note = match armed {
            Some(i) => self.point_of(i).map(|(x, y)| {
                let third = match map.refused() {
                    Some(cc) => (
                        format!(
                            "CC {cc} IS {}: NOT LEARNED. MOVE ANOTHER CONTROLLER.",
                            map.reserved(cc).unwrap_or("reserved").to_uppercase()
                        ),
                        Tone::Warn,
                    ),
                    None => (
                        if i == MOD_WHEEL {
                            "SETS THIS COMPUTER'S SOURCE · ALL CHANNELS".into()
                        } else {
                            map.reserved_text()
                        },
                        Tone::Dim,
                    ),
                };
                Note {
                    x,
                    y,
                    size,
                    lines: vec![
                        (format!("MIDI LEARN: {}", self.label(i)), Tone::Accent),
                        (
                            format!("{} · ESC OR CANCEL STOPS", instruction(i, notes)),
                            Tone::Plain,
                        ),
                        third,
                    ],
                }
            }),
            None => self.notice.as_ref().and_then(|(lines, i, _)| {
                self.point_of(*i).map(|(x, y)| Note {
                    x,
                    y,
                    size,
                    lines: lines.clone(),
                })
            }),
        };
        (
            note,
            panel,
            strip,
            self.open.as_ref().map(|o| o.menu.clone()),
        )
    }

    /// The drawer's MIDI list as it is now.
    pub fn list_scene(&mut self, map: &MidiMap) -> MidiList {
        let assigned = map.bindings();
        let armed = map.armed();
        let rows: Vec<MidiRow> = LEARNABLE
            .iter()
            .take(self.listed())
            .enumerate()
            .map(|(i, _)| MidiRow {
                name: self.label(i),
                assignment: assigned[i].map(Binding::text).unwrap_or_default(),
                waiting: armed == Some(i),
            })
            .collect();
        self.chosen = self.chosen.min(rows.len().saturating_sub(1));
        let refused = map.refused().map(|cc| {
            (
                format!(
                    "CC {cc} IS {}: NOT LEARNED. MOVE ANOTHER CONTROLLER.",
                    map.reserved(cc).unwrap_or("reserved").to_uppercase()
                ),
                DrawerTone::Trouble,
            )
        });
        let (status, tone) = refused.unwrap_or_else(|| self.status.clone());
        MidiList {
            rows,
            first: self.first,
            chosen: self.chosen,
            keys: KEYS_TEXT.into(),
            reserved: map.reserved_text(),
            status,
            tone,
        }
    }

    /// The list shown with learnable parameter `i` chosen (in view).
    pub fn show_list(&mut self, chosen: Option<usize>) {
        self.list = true;
        if let Some(i) = chosen {
            self.choose(i);
        }
    }

    fn choose(&mut self, i: usize) {
        self.chosen = i.min(self.listed() - 1);
        if self.chosen < self.first {
            self.first = self.chosen;
        } else if self.chosen >= self.first + ROWS_SHOWN {
            self.first = self.chosen + 1 - ROWS_SHOWN;
        }
    }

    /// The list scrolled by `rows`.
    pub fn scroll(&mut self, rows: i32) {
        let most = self.listed().saturating_sub(ROWS_SHOWN) as i32;
        self.first = (self.first as i32 + rows).clamp(0, most) as usize;
    }

    /// A press on the list: a row chosen, or its button done.
    pub fn list_press(&mut self, map: &MidiMap, row: usize, action: Option<MidiAction>) {
        self.choose(row);
        match action {
            Some(MidiAction::Learn) => self.learn(map, row),
            Some(MidiAction::Cancel) => self.cancel(map),
            Some(MidiAction::Remove) => self.remove(map, row),
            None => {}
        }
    }

    /// A key while the drawer shows the list: whether it was the list's. `close` shuts the
    /// drawer (Escape with nothing being learned).
    pub fn list_key(&mut self, e: &KeyboardEvent, map: &MidiMap, close: &mut bool) -> bool {
        if e.state != KeyState::Down {
            return true;
        }
        // The host's shortcuts (Command, Control) stay the host's.
        if e.modifiers.intersects(Modifiers::META | Modifiers::CONTROL) {
            return false;
        }
        match &e.key {
            Key::ArrowDown => self.choose(self.chosen + 1),
            Key::ArrowUp => self.choose(self.chosen.saturating_sub(1)),
            Key::PageDown => self.choose(self.chosen + ROWS_SHOWN),
            Key::PageUp => self.choose(self.chosen.saturating_sub(ROWS_SHOWN)),
            Key::Home => self.choose(0),
            Key::End => self.choose(self.listed() - 1),
            Key::Enter => {
                if map.armed() == Some(self.chosen) {
                    self.cancel(map);
                } else {
                    self.learn(map, self.chosen);
                }
            }
            Key::Delete | Key::Backspace => self.remove(map, self.chosen),
            Key::Escape => {
                if map.armed().is_some() {
                    self.cancel(map);
                } else {
                    *close = true;
                }
            }
            // A letter chooses the next control whose name begins with it.
            Key::Character(c) => {
                let c = c.to_uppercase();
                let n = self.listed();
                if let Some(i) = (1..=n)
                    .map(|k| (self.chosen + k) % n)
                    .find(|&i| LEARNABLE[i].label.starts_with(&c))
                {
                    self.choose(i);
                }
            }
            _ => return false,
        }
        true
    }

    /// The parameter chosen in the list.
    pub fn chosen(&self) -> usize {
        self.chosen
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ca72_panel::strip::{StripTarget, centre};

    /// The settings' AGENTS (decisions.md R-MCP): how agents stand under the choices while they
    /// are allowed, the clients' setups to copy while the bridge is beside the application,
    /// and the one copied said so.
    #[test]
    fn the_settings_agents_say_how_they_stand_and_copy_setups() {
        let fonts = Fonts::new();
        let rows = |agents: AgentsMenu| {
            let mut l = Learning::default();
            l.open_settings_menu(
                &fonts,
                (800.0, 40.0),
                12.0,
                (
                    Skin::Worn,
                    (Lettering::Standard, TicksTitles::Standard),
                    Law::Semitones,
                    Source::DEFAULT,
                    Some(agents),
                    true,
                ),
                (&[], None),
                None,
            );
            let items = l.menu().expect("menu").items.clone();
            let at = items
                .iter()
                .position(|(t, _)| t == "AGENTS")
                .expect("AGENTS");
            items[at + 1..].to_vec()
        };
        let row = |t: &str, on: bool| (t.to_owned(), on);
        let listening = AgentsMenu {
            allowed: true,
            status: Some("LISTENING: 1 AGENT CONNECTED".into()),
            bridge: true,
            copied: None,
        };
        assert_eq!(
            rows(listening.clone()),
            [
                row("ALLOW", false),
                row("OFF", true),
                row("LISTENING: 1 AGENT CONNECTED", false),
                row("COPY SETUP: CLAUDE CODE", true),
                row("COPY SETUP: JSON FOR OTHER APPS", true),
            ]
        );
        let copied = AgentsMenu {
            copied: Some((Setup::ClaudeCode, true)),
            ..listening.clone()
        };
        assert_eq!(rows(copied)[3], row("COPIED: PASTE IT IN A TERMINAL", true));
        let failed = AgentsMenu {
            copied: Some((Setup::Json, false)),
            ..listening
        };
        assert_eq!(
            rows(failed)[4],
            row("COULD NOT COPY TO THE CLIPBOARD", true)
        );
        let off_alone = AgentsMenu::default();
        assert_eq!(
            rows(off_alone),
            [
                row("ALLOW", true),
                row("OFF", false),
                row("SETUP: NO BRIDGE BESIDE THE APP", false),
            ]
        );
    }

    /// PREVIOUS and NEXT PRESET's notes go over their arrows on the rail the editor shows: the
    /// standalone's, with FX (R-FX-DRAWER), has NEXT a key further left (R-PRESET-KEYS).
    #[test]
    fn a_preset_actions_note_goes_over_its_arrow_on_either_rail() {
        for rail in [Rail::Plugin, Rail::Effects] {
            for t in [BarTarget::Prev, BarTarget::Next] {
                assert_eq!(
                    note_point(rail, Place::Preset(t)),
                    centre(rail, StripTarget::Bar(t)),
                    "{rail:?} {t:?}"
                );
            }
        }
        assert_ne!(
            note_point(Rail::Plugin, Place::Preset(BarTarget::Next)),
            note_point(Rail::Effects, Place::Preset(BarTarget::Next))
        );
    }
}
