//! The presets (decisions.md R10), in a library shared with the DAW the model was developed
//! in: a sound is named panel settings, the dials' plain values by parameter ID (a choice by
//! its index, a switch 0 or 1, POLY and VOICES too; never the PITCH wheel or the bypass).
//!
//! - **The factory's** are built in (`sounds/presets.toml`; the DAW carries its own copy) and
//!   never change. The file ships whole inside the plug-in, so it names no trademark (R1).
//! - **The user's** are files in the library's folder, `<name>.toml` (`format`, `name`,
//!   `description`, `tags`, `favorite`, `replaces`, `values`), written whole and moved into
//!   place so the other program never reads half of one. A user preset named as a factory
//!   one (or saying it `replaces` one) stands in for it: an edit or a rename of a factory
//!   preset, taken back by [`Library::revert`].
//! - **The factory's tags, favourites and deletions** are the folder's `factory.toml`
//!   (`[presets."<name>"]`: `tags`, `favorite`, `hidden`); [`Library::restore`] brings the
//!   deleted back.
//! - The folder is `<shared>/CA-72/Presets`, `<shared>` being `$IDLE_FOUNDRY_SHARED`, else the
//!   user's data folder's `Idle Foundry` (`~/.local/share`, `~/Library/Application Support`,
//!   `%APPDATA%`).
//!
//! Names are 1 to 64 characters, none of `/ \ : * ? " < > |`, not starting or ending with a
//! space or a dot, unique ignoring case; never `factory` (the folder's own file) nor a name
//! Windows keeps for a device (`CON`, `NUL`, `COM1` and the like, with any extension), on every
//! system, since the folder may be shared between them (decisions.md R18). A file whose `name`
//! breaks these is reported, not listed.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use toml::Value;

use crate::drive::STEPS;

/// The format of the files this reads and writes.
pub const FORMAT: i64 = 1;

/// The parameters a preset never sets: the PITCH wheel (where the player leaves it), the
/// bypass (the host's), POWER (the player's: decisions.md R-POWER) and QUALITY (the
/// computer's: R47).
pub const KEPT: &[&str] = &["pitch_wheel", "bypass", "power", "quality"];

const MOST_TAGS: usize = 16;
const LONGEST_TAG: usize = 32;

/// The folder's own file's name (the factory presets' tags, favourites and deletions).
const OVERLAY: &str = "factory";

/// The names Windows keeps for its devices, an extension or not (`NUL.txt` is `NUL`), and
/// those it adds a digit to (`COM0` to `COM9`, `COM¹` to `COM³`, `LPT` the same).
const DEVICES: &[&str] = &["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"];
const NUMBERED_DEVICES: &[&str] = &["COM", "LPT"];

/// A preset's sound: its values (plain), AUTO GAIN's curve for it, and what a browser shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Sound {
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub values: Vec<(String, f64)>,
    /// AUTO GAIN's curve for the sound (`drive.rs`), measured at QUALITY HI (ULTRA too) and at
    /// LO: in force as soon as the preset is chosen, so that its level is its own at once
    /// (decisions.md R-AUTO-GAIN-DRIVE). None in a preset saved before.
    pub drive_curve: Option<[f32; STEPS]>,
    pub drive_curve_lo: Option<[f32; STEPS]>,
    /// Its macros' typed names, in their order (`macro_names`, a list; empty for one without a
    /// name; decisions.md R-MACROS). None in a preset saved before them, or naming none.
    pub macro_names: Vec<String>,
    /// The standalone’s effects, kept verbatim by plug-in instances (R-FX-PRESETS).
    pub effects: Option<String>,
}

impl Sound {
    /// AUTO GAIN's curve for the sound at QUALITY LO (`lo`) or above: that quality's, else
    /// the other's.
    pub fn curve(&self, lo: bool) -> Option<[f32; STEPS]> {
        if lo {
            self.drive_curve_lo.or(self.drive_curve)
        } else {
            self.drive_curve.or(self.drive_curve_lo)
        }
    }
}

/// A curve as a preset names it: [`STEPS`] numbers, else none (one not understood is left
/// out: the preset plays with the curve in force, as one saved before curves did).
fn curve_of(v: Option<&Value>) -> Option<[f32; STEPS]> {
    let a = v?.as_array()?;
    let db: Vec<f32> = a
        .iter()
        .map(|x| match x {
            Value::Integer(i) => Some(*i as f32),
            Value::Float(f) => Some(*f as f32),
            _ => None,
        })
        .collect::<Option<_>>()?;
    let db = <[f32; STEPS]>::try_from(db.as_slice()).ok()?;
    db.iter().all(|d| d.is_finite()).then_some(db)
}

/// A curve as a preset's file holds it: each correction written as short as it reads back.
fn curve_value(db: &[f32; STEPS]) -> Value {
    Value::Array(
        db.iter()
            .map(|d| Value::Float(d.to_string().parse().unwrap_or(0.0)))
            .collect(),
    )
}

/// Where a preset comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Factory,
    /// The user's version of a factory preset (edited or renamed), standing in for it.
    Edited,
    User,
}

/// A preset as the library offers it.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub sound: Sound,
    pub favorite: bool,
    pub origin: Origin,
    /// The factory preset it stands in for.
    pub replaces: Option<String>,
    file: Option<PathBuf>,
}

impl Entry {
    pub(crate) fn file(&self) -> Option<&Path> {
        self.file.as_deref()
    }
}

fn same(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// A sound from its table (`name`, `description?`, `tags?`, `values`).
fn sound_of(t: &toml::Table) -> Result<Sound, String> {
    let name = t
        .get("name")
        .and_then(Value::as_str)
        .ok_or("a preset has no `name`")?
        .to_owned();
    let description = t
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let tags = match t.get("tags") {
        None => Vec::new(),
        Some(Value::Array(a)) => a
            .iter()
            .map(|v| v.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| format!("{name}: `tags` holds text only"))?,
        Some(_) => return Err(format!("{name}: `tags` is a list")),
    };
    let mut values = Vec::new();
    if let Some(v) = t.get("values") {
        let v = v
            .as_table()
            .ok_or_else(|| format!("{name}: `values` is a table"))?;
        for (k, x) in v {
            let x = match x {
                Value::Integer(i) => *i as f64,
                Value::Float(f) => *f,
                Value::Boolean(b) => f64::from(u8::from(*b)),
                _ => return Err(format!("{name}: `{k}` is not a number")),
            };
            values.push((k.clone(), x));
        }
    }
    if crate::compat::before_entropy_floor(
        t.get("version").and_then(Value::as_str).unwrap_or_default(),
    ) {
        crate::compat::translate_entropy_preset(&mut values);
    }
    Ok(Sound {
        name,
        description,
        tags,
        values,
        drive_curve: curve_of(t.get("drive_curve")),
        drive_curve_lo: curve_of(t.get("drive_curve_lo")),
        effects: None,
        // (A list of text, else none: one not understood is left out, as a curve is.)
        macro_names: t
            .get("macro_names")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|v| v.as_str().unwrap_or_default().to_owned())
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// An opaque section, including unknown keys, comments and the state’s exact spelling.
/// Keep its formatting when a plug-in saves a sound it cannot host (R-FX-PRESETS).
pub(crate) fn effects_section(text: &str) -> Option<String> {
    // Slice the original text by the parser's header spans. Formatting it again would
    // normalise CRLF, including mixed line endings and newlines inside saved state.
    fn table_headers(table: &toml_edit::Table, keep: bool, out: &mut Vec<(usize, bool)>) {
        if !table.is_implicit()
            && !table.is_dotted()
            && let Some(span) = table.span()
        {
            let start = table
                .decor()
                .prefix()
                .and_then(|p| p.span())
                .map_or(span.start, |s| s.start);
            out.push((start, keep));
        }
        for (_, child) in table.iter() {
            headers(child, keep, out);
        }
    }
    fn headers(item: &toml_edit::Item, keep: bool, out: &mut Vec<(usize, bool)>) {
        if let Some(table) = item.as_table() {
            table_headers(table, keep, out);
        } else if let Some(array) = item.as_array_of_tables() {
            for table in array.iter() {
                table_headers(table, keep, out);
            }
        }
    }
    let doc = toml_edit::Document::parse(text).ok()?;
    doc.get("effects")?;
    let mut starts = Vec::new();
    for (key, item) in doc.iter() {
        headers(item, key == "effects", &mut starts);
    }
    starts.sort_by_key(|(start, _)| *start);
    starts.push((text.len(), false));
    let mut raw = String::new();
    for pair in starts.windows(2) {
        if pair[0].1 {
            raw.push_str(text.get(pair[0].0..pair[1].0)?);
        }
    }
    (!raw.is_empty()).then_some(raw)
}

/// The factory presets' file, built in whole.
const FACTORY_FILE: &str = include_str!("../sounds/presets.toml");

/// The factory's presets.
pub fn factory() -> &'static [Sound] {
    static FACTORY: OnceLock<Vec<Sound>> = OnceLock::new();
    FACTORY.get_or_init(|| parse_factory(FACTORY_FILE).unwrap_or_default())
}

/// A factory file's presets.
pub fn parse_factory(text: &str) -> Result<Vec<Sound>, String> {
    let t: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
    match t.get("format").and_then(Value::as_integer) {
        Some(FORMAT) => {}
        other => return Err(format!("presets of format {other:?}, not {FORMAT}")),
    }
    match t.get("preset") {
        None => Ok(Vec::new()),
        Some(Value::Array(a)) => a
            .iter()
            .map(|p| {
                p.as_table()
                    .ok_or_else(|| "a preset is a table".to_owned())
                    .and_then(|p| {
                        let mut p = p.clone();
                        if let Some(version) = t.get("version") {
                            p.entry("version").or_insert_with(|| version.clone());
                        }
                        sound_of(&p)
                    })
            })
            .collect(),
        Some(_) => Err("`preset` is a list".into()),
    }
}

/// The tag of every preset in the retired presets' pack, and its files' suffix.
pub const PACK_TAG: &str = "0.1";

/// The retired presets' pack (decisions.md R55): each of `sounds` written into `dir` as
/// the user's preset, tagged [`PACK_TAG`], in `<name>_0.1.toml`. The user's own preset is
/// `<name>.toml`, so copying the pack into a library never writes over one; where both name
/// one preset (their edit of a factory preset since retired), theirs, first by file name, is
/// the one listed. Saved again, a pack's preset moves to `<name>.toml` when that is free.
pub fn write_pack(dir: &Path, sounds: &[Sound]) -> Result<Vec<PathBuf>, String> {
    let lib = Library::at(dir);
    sounds
        .iter()
        .map(|s| {
            check_name(&s.name)?;
            let mut s = s.clone();
            if !s.tags.iter().any(|t| same(t, PACK_TAG)) {
                s.tags.push(PACK_TAG.to_owned());
            }
            let file = dir.join(format!("{}_{PACK_TAG}.toml", s.name));
            lib.write_user(&file, &s, false, None)?;
            Ok(file)
        })
        .collect()
}

/// Whether a file named `stem` (before its first dot) is one of Windows's devices.
fn device(stem: &str) -> bool {
    let stem = stem.trim_end().to_uppercase();
    DEVICES.contains(&stem.as_str())
        || NUMBERED_DEVICES.iter().any(|d| {
            stem.strip_prefix(d).is_some_and(|n| {
                let mut c = n.chars();
                c.next()
                    .is_some_and(|c| c.is_ascii_digit() || "¹²³".contains(c))
                    && c.next().is_none()
            })
        })
}

/// Whether `name` can be a preset's (and its file's) name, on every system (decisions.md R18).
pub fn check_name(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && name.chars().count() <= 64
        && !name.starts_with(['.', ' '])
        && !name.ends_with([' ', '.'])
        && !name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|'])
        && !name.chars().any(char::is_control);
    if !ok {
        return Err("A name is 1 to 64 characters, without / \\ : * ? \" < > |".into());
    }
    if same(name, OVERLAY) {
        return Err(format!("{name} is the library's own file"));
    }
    if device(name.split('.').next().unwrap_or(name)) {
        return Err(format!("{name} is a name Windows keeps for a device"));
    }
    Ok(())
}

/// Tags as typed (`bass, dark`): trimmed, without repeats (ignoring case) or empty ones.
pub fn parse_tags(text: &str) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for t in text.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        if t.chars().count() > LONGEST_TAG || t.chars().any(char::is_control) {
            return Err(format!("A tag is 1 to {LONGEST_TAG} characters: {t}"));
        }
        if !out.iter().any(|o| same(o, t)) {
            out.push(t.to_owned());
        }
    }
    if out.len() > MOST_TAGS {
        return Err(format!("At most {MOST_TAGS} tags"));
    }
    Ok(out)
}

/// The folder of the libraries the plug-in shares with the DAW.
pub fn shared_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("IDLE_FOUNDRY_SHARED") {
        return PathBuf::from(d);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(windows) {
        if let Some(a) = std::env::var_os("APPDATA") {
            return PathBuf::from(a).join("Idle Foundry");
        }
    } else if cfg!(target_os = "macos") {
        if let Some(h) = &home {
            return h.join("Library/Application Support/Idle Foundry");
        }
    } else if let Some(x) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(x).join("Idle Foundry");
    } else if let Some(h) = &home {
        return h.join(".local/share/Idle Foundry");
    }
    PathBuf::from("idle-foundry-shared")
}

#[derive(Clone, Debug, Default, PartialEq)]
struct FactoryState {
    tags: Option<Vec<String>>,
    favorite: bool,
    hidden: bool,
}

fn strings(list: &[String]) -> Value {
    Value::Array(list.iter().map(|s| Value::String(s.clone())).collect())
}

/// A factory preset's marks in the overlay changed: its table, found by name ignoring case
/// (the key kept as written), a new one if there is none.
fn change_marks(presets: &mut toml::Table, name: &str, change: impl FnOnce(&mut toml::Table)) {
    let key = presets
        .keys()
        .find(|k| *k == name)
        .or_else(|| presets.keys().find(|k| same(k, name)))
        .cloned()
        .unwrap_or_else(|| name.to_owned());
    let mut marks = match presets.remove(&key) {
        Some(Value::Table(m)) => m,
        _ => toml::Table::new(),
    };
    change(&mut marks);
    presets.insert(key, Value::Table(marks));
}

/// A switch among a preset's marks: written when on, left out when off.
fn set_mark(marks: &mut toml::Table, key: &str, on: bool) {
    if on {
        marks.insert(key.into(), Value::Boolean(true));
    } else {
        marks.remove(key);
    }
}

/// The folder's files as found: each one's name, size and modification time.
type Seen = Vec<(OsString, u64, Option<SystemTime>)>;

/// The presets as read, and the files that could not be.
type Read = (Vec<Entry>, Vec<String>);

/// The library: its folder.
#[derive(Clone, Debug)]
pub struct Library {
    dir: PathBuf,
    /// The presets as last read and the folder as it was then: read again only when a file's
    /// name, size or modification time changes, or this library writes (decisions.md R18).
    last: Arc<Mutex<Option<(Seen, Read)>>>,
}

impl Library {
    /// The library shared with the DAW: `<shared>/CA-72/Presets`.
    pub fn shared() -> Self {
        Library::at(shared_dir().join("CA-72").join("Presets"))
    }

    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Library {
            dir: dir.into(),
            last: Arc::default(),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn overlay_file(&self) -> PathBuf {
        self.dir.join(format!("{OVERLAY}.toml"))
    }

    /// The overlay as written, every key kept (another program's, a newer version's), so
    /// writing it again loses none: empty when there is none; an error when it cannot be
    /// read, and it is then never written over (decisions.md R18).
    fn overlay_table(&self) -> Result<toml::Table, String> {
        let file = self.overlay_file();
        let unreadable = |why: &str| {
            let why = why.lines().next().unwrap_or_default();
            format!("{} cannot be read ({why}): left as it is", file.display())
        };
        let text = match std::fs::read_to_string(&file) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(toml::Table::new()),
            Err(e) => return Err(unreadable(&e.to_string())),
        };
        let t: toml::Table = text
            .parse()
            .map_err(|e: toml::de::Error| unreadable(&e.to_string()))?;
        if !matches!(t.get("format"), None | Some(Value::Integer(FORMAT))) {
            return Err(unreadable("a format this does not know"));
        }
        if !matches!(t.get("presets"), None | Some(Value::Table(_))) {
            return Err(unreadable("`presets` is not a table"));
        }
        Ok(t)
    }

    /// The factory presets' marks, by name as written.
    fn read_overlay(&self) -> Result<BTreeMap<String, FactoryState>, String> {
        let t = self.overlay_table()?;
        let Some(Value::Table(p)) = t.get("presets") else {
            return Ok(BTreeMap::new());
        };
        Ok(p.iter()
            .filter_map(|(name, s)| {
                let s = s.as_table()?;
                Some((
                    name.clone(),
                    FactoryState {
                        tags: s.get("tags").and_then(Value::as_array).map(|a| {
                            a.iter()
                                .filter_map(|t| t.as_str().map(str::to_owned))
                                .collect()
                        }),
                        favorite: s.get("favorite").and_then(Value::as_bool).unwrap_or(false),
                        hidden: s.get("hidden").and_then(Value::as_bool).unwrap_or(false),
                    },
                ))
            })
            .collect())
    }

    /// The overlay's presets changed by `change` and the file written again, every other key
    /// as it was; not written when nothing changed, nor when it cannot be read.
    fn change_overlay(&self, change: impl FnOnce(&mut toml::Table)) -> Result<(), String> {
        let mut t = self.overlay_table()?;
        let mut presets = match t.remove("presets") {
            Some(Value::Table(p)) => p,
            _ => toml::Table::new(),
        };
        let before = presets.clone();
        change(&mut presets);
        // A preset left with no mark.
        presets.retain(|_, v| v.as_table().is_none_or(|m| !m.is_empty()));
        if presets == before {
            return Ok(());
        }
        t.entry("format").or_insert(Value::Integer(FORMAT));
        t.insert("presets".into(), Value::Table(presets));
        self.put(&self.overlay_file(), &t)
    }

    fn user_file(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.toml"))
    }

    /// The user's presets (with their files, favourites and what they stand in for), and
    /// the files that could not be read.
    #[allow(clippy::type_complexity)]
    fn users(&self) -> (Vec<(PathBuf, Sound, bool, Option<String>)>, Vec<String>) {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&self.dir)
            .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect())
            .unwrap_or_default();
        files.sort();
        let (mut out, mut bad) = (Vec::new(), Vec::new());
        for f in files {
            if f.extension().is_none_or(|e| e != "toml")
                || f.file_stem()
                    .is_some_and(|n| same(&n.to_string_lossy(), OVERLAY))
            {
                continue;
            }
            let read = read_user(&f);
            match read {
                Ok((s, fav, rep)) => out.push((f, s, fav, rep)),
                Err(e) => bad.push(format!("{}: {e}", f.display())),
            }
        }
        (out, bad)
    }

    /// The folder's files as they are now.
    fn seen(&self) -> Seen {
        let mut seen: Seen = std::fs::read_dir(&self.dir)
            .map(|rd| {
                rd.filter_map(Result::ok)
                    .map(|e| {
                        let m = std::fs::metadata(e.path()).ok();
                        let size = m.as_ref().map_or(0, std::fs::Metadata::len);
                        (e.file_name(), size, m.and_then(|m| m.modified().ok()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        seen.sort();
        seen
    }

    /// The presets as read forgotten: this library changed the folder (a file rewritten
    /// within the file system's time resolution, its size the same, would not show).
    fn forget(&self) {
        if let Ok(mut last) = self.last.lock() {
            *last = None;
        }
    }

    /// A table written to the folder.
    fn put(&self, file: &Path, t: &toml::Table) -> Result<(), String> {
        let r = write(file, t);
        self.forget();
        r
    }

    /// A file of the folder deleted.
    fn delete(&self, file: &Path) -> Result<(), String> {
        let r = std::fs::remove_file(file).map_err(|e| format!("{}: {e}", file.display()));
        self.forget();
        r
    }

    /// Every preset, by name (ignoring case), and the files that could not be read: as last
    /// read while the folder is as it was then (decisions.md R18).
    pub fn entries(&self) -> (Vec<Entry>, Vec<String>) {
        let seen = self.seen();
        if let Ok(last) = self.last.lock()
            && let Some((then, read)) = last.as_ref()
            && *then == seen
        {
            return read.clone();
        }
        let read = self.read();
        if let Ok(mut last) = self.last.lock() {
            *last = Some((seen, read.clone()));
        }
        read
    }

    /// Every preset, read from the folder.
    fn read(&self) -> Read {
        let factory = factory();
        let (overlay, mut bad) = match self.read_overlay() {
            Ok(o) => (o, Vec::new()),
            Err(e) => (BTreeMap::new(), vec![e]),
        };
        let marks = |name: &str| {
            overlay
                .get(name)
                .or_else(|| overlay.iter().find(|(k, _)| same(k, name)).map(|(_, s)| s))
                .cloned()
                .unwrap_or_default()
        };
        let (mut users, unread) = self.users();
        bad.extend(unread);
        // A name is one preset's, ignoring case, on every file system (decisions.md R18): a
        // second file of it is reported, the first by file name listed.
        let mut names: Vec<String> = Vec::new();
        users.retain(|(f, s, _, _)| {
            let again = names.iter().any(|n| same(n, &s.name));
            if again {
                bad.push(format!("{}: a second preset {}", f.display(), s.name));
            } else {
                names.push(s.name.clone());
            }
            !again
        });
        // The factory preset a user's stands in for, spelt as the factory spells it: the one
        // it `replaces`, else the one of its name.
        let stands_for = |s: &Sound, r: &Option<String>| -> Option<String> {
            [r.as_deref(), Some(s.name.as_str())]
                .into_iter()
                .flatten()
                .find_map(|n| factory.iter().find(|f| same(&f.name, n)))
                .map(|f| f.name.clone())
        };
        let replaced: Vec<String> = users
            .iter()
            .filter_map(|(_, s, _, r)| stands_for(s, r))
            .collect();
        let mut list: Vec<Entry> = factory
            .iter()
            .filter(|f| !marks(&f.name).hidden && !replaced.iter().any(|r| same(r, &f.name)))
            .map(|f| {
                let state = marks(&f.name);
                let mut sound = f.clone();
                if let Some(t) = state.tags {
                    sound.tags = t;
                }
                Entry {
                    sound,
                    favorite: state.favorite,
                    origin: Origin::Factory,
                    replaces: None,
                    file: None,
                }
            })
            .collect();
        for (file, sound, favorite, r) in users {
            let replaces = stands_for(&sound, &r);
            list.push(Entry {
                origin: if replaces.is_some() {
                    Origin::Edited
                } else {
                    Origin::User
                },
                sound,
                favorite,
                replaces,
                file: Some(file),
            });
        }
        list.sort_by(|a, b| {
            a.sound
                .name
                .to_lowercase()
                .cmp(&b.sound.name.to_lowercase())
                .then(a.sound.name.cmp(&b.sound.name))
        });
        (list, bad)
    }

    /// The preset of this name (ignoring case).
    pub fn find(&self, name: &str) -> Option<Entry> {
        self.entries()
            .0
            .into_iter()
            .find(|e| same(&e.sound.name, name))
    }

    /// The presets that match ([`matching`]).
    pub fn search(&self, query: &str, tags: &[String], favorite: bool, mine: bool) -> Vec<Entry> {
        matching(&self.entries().0, query, tags, favorite, mine)
    }

    /// Every tag in use, by name ([`tags_of`]).
    pub fn tags(&self) -> Vec<String> {
        tags_of(&self.entries().0)
    }

    /// Whether `file` is free for a preset: no file of the folder has its name, ignoring case
    /// (where the file system ignores it, that is the same file), but `own`.
    fn free(&self, file: &Path, own: Option<&Path>) -> bool {
        let Some(want) = file.file_name().map(|n| n.to_string_lossy()) else {
            return false;
        };
        let own = own.and_then(Path::file_name);
        std::fs::read_dir(&self.dir).map_or(true, |rd| {
            rd.filter_map(Result::ok).all(|e| {
                let n = e.file_name();
                Some(n.as_os_str()) == own || !same(&n.to_string_lossy(), &want)
            })
        })
    }

    /// The file a preset named `name` is written to (decisions.md R18): `<name>.toml`, its own
    /// file (`own`) moved there first when that is free, so a new name (if only in case) leaves
    /// no second file on any file system; else its own file where it is. Never another's.
    fn place(&self, name: &str, own: Option<&Path>) -> Result<PathBuf, String> {
        let file = self.user_file(name);
        match own {
            Some(own) if own == file => Ok(file),
            Some(own) if self.free(&file, Some(own)) => {
                let moved = std::fs::rename(own, &file);
                self.forget();
                match moved {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                        Err(format!("{}: {e}", file.display()))
                    }
                    _ => Ok(file),
                }
            }
            Some(own) => Ok(own.to_path_buf()),
            None if self.free(&file, None) => Ok(file),
            None => Err(format!("There is already a file {}", file.display())),
        }
    }

    /// A preset written whole to `file`.
    fn write_user(
        &self,
        file: &Path,
        sound: &Sound,
        favorite: bool,
        replaces: Option<&str>,
    ) -> Result<(), String> {
        let mut t = toml::Table::new();
        t.insert("format".into(), Value::Integer(FORMAT));
        t.insert(
            "version".into(),
            Value::String(env!("CARGO_PKG_VERSION").into()),
        );
        t.insert("name".into(), Value::String(sound.name.clone()));
        if !sound.description.is_empty() {
            t.insert(
                "description".into(),
                Value::String(sound.description.clone()),
            );
        }
        t.insert("tags".into(), strings(&sound.tags));
        t.insert("favorite".into(), Value::Boolean(favorite));
        if let Some(r) = replaces {
            t.insert("replaces".into(), Value::String(r.to_owned()));
        }
        let mut values = toml::Table::new();
        for (k, x) in &sound.values {
            let v = if x.fract() == 0.0 && x.abs() < 1e6 {
                Value::Integer(*x as i64)
            } else {
                Value::Float(*x)
            };
            values.insert(k.clone(), v);
        }
        if let Some(db) = &sound.drive_curve {
            t.insert("drive_curve".into(), curve_value(db));
        }
        if let Some(db) = &sound.drive_curve_lo {
            t.insert("drive_curve_lo".into(), curve_value(db));
        }
        if sound.macro_names.iter().any(|n| !n.is_empty()) {
            t.insert("macro_names".into(), strings(&sound.macro_names));
        }
        t.insert("values".into(), Value::Table(values));
        let mut text = toml::to_string(&t).map_err(|e| e.to_string())?;
        if let Some(effects) = &sound.effects {
            text.push_str(effects);
        }
        let result = write_text(file, &text);
        self.forget();
        result
    }

    /// The current sound saved as `name` (`values` plain, by parameter ID): replacing the
    /// user's preset of that name (its description, tags and favourite kept unless `tags`
    /// is given), or, named as a factory preset, the user's version standing in for it.
    pub fn save(
        &self,
        name: &str,
        values: Vec<(String, f64)>,
        tags: Option<Vec<String>>,
    ) -> Result<Entry, String> {
        self.save_with_curve(name, values, tags, None)
    }

    /// [`Library::save`], with AUTO GAIN's curve heard as it was saved, at QUALITY LO or above
    /// (`(lo, db)`; decisions.md R-AUTO-GAIN-DRIVE).
    pub fn save_with_curve(
        &self,
        name: &str,
        values: Vec<(String, f64)>,
        tags: Option<Vec<String>>,
        curve: Option<(bool, [f32; STEPS])>,
    ) -> Result<Entry, String> {
        self.save_with(
            name,
            values,
            tags,
            curve,
            Vec::new(),
            self.find(name).and_then(|e| e.sound.effects),
        )
    }

    /// [`Library::save_with_curve`], with the macros' names (decisions.md R-MACROS).
    pub fn save_with(
        &self,
        name: &str,
        values: Vec<(String, f64)>,
        tags: Option<Vec<String>>,
        curve: Option<(bool, [f32; STEPS])>,
        macro_names: Vec<String>,
        effects: Option<String>,
    ) -> Result<Entry, String> {
        let name = name.trim();
        check_name(name)?;
        let existing = self.find(name);
        let factory = factory()
            .iter()
            .find(|f| same(&f.name, name))
            .map(|f| f.name.clone());
        let (description, old_tags, favorite, replaces, own) = match existing {
            Some(e) => (
                e.sound.description,
                e.sound.tags,
                e.favorite,
                e.replaces.or(factory),
                e.file,
            ),
            None => (String::new(), Vec::new(), false, factory, None),
        };
        let sound = Sound {
            name: name.to_owned(),
            description,
            tags: tags.unwrap_or(old_tags),
            values,
            drive_curve: curve.and_then(|(lo, db)| (!lo).then_some(db)),
            drive_curve_lo: curve.and_then(|(lo, db)| lo.then_some(db)),
            macro_names,
            effects,
        };
        let file = self.place(name, own.as_deref())?;
        self.write_user(&file, &sound, favorite, replaces.as_deref())?;
        self.find(name)
            .ok_or_else(|| format!("{name} was not saved"))
    }

    /// A preset renamed; a factory preset renamed is the user's version standing in for it.
    pub fn rename(&self, name: &str, to: &str) -> Result<Entry, String> {
        let to = to.trim();
        check_name(to)?;
        let e = self.find(name).ok_or_else(|| format!("No preset {name}"))?;
        if self
            .entries()
            .0
            .iter()
            .any(|o| same(&o.sound.name, to) && !same(&o.sound.name, &e.sound.name))
        {
            return Err(format!("There is already a preset {to}"));
        }
        let replaces = e
            .replaces
            .clone()
            .or_else(|| (e.origin == Origin::Factory).then(|| e.sound.name.clone()));
        let mut sound = e.sound.clone();
        sound.name = to.to_owned();
        let file = self.place(to, e.file.as_deref())?;
        self.write_user(&file, &sound, e.favorite, replaces.as_deref())?;
        self.find(to).ok_or_else(|| format!("{to} was not saved"))
    }

    fn mark(
        &self,
        e: &Entry,
        tags: Option<Vec<String>>,
        favorite: Option<bool>,
    ) -> Result<(), String> {
        match &e.file {
            // Its own file, whatever its name says (decisions.md R18).
            Some(file) => {
                let mut sound = e.sound.clone();
                if let Some(t) = tags {
                    sound.tags = t;
                }
                self.write_user(
                    file,
                    &sound,
                    favorite.unwrap_or(e.favorite),
                    e.replaces.as_deref(),
                )
            }
            None => self.change_overlay(|p| {
                change_marks(p, &e.sound.name, |m| {
                    if let Some(t) = tags {
                        m.insert("tags".into(), strings(&t));
                    }
                    if let Some(f) = favorite {
                        set_mark(m, "favorite", f);
                    }
                })
            }),
        }
    }

    /// A preset's tags, replaced.
    pub fn tag(&self, name: &str, tags: Vec<String>) -> Result<(), String> {
        let e = self.find(name).ok_or_else(|| format!("No preset {name}"))?;
        self.mark(&e, Some(tags), None)
    }

    /// A preset made a favourite, or not.
    pub fn favorite(&self, name: &str, on: bool) -> Result<(), String> {
        let e = self.find(name).ok_or_else(|| format!("No preset {name}"))?;
        self.mark(&e, None, Some(on))
    }

    /// A preset deleted: the user's file; a factory preset (or the one the user's version
    /// stood in for) hidden until [`Library::restore`].
    pub fn remove(&self, name: &str) -> Result<(), String> {
        let e = self.find(name).ok_or_else(|| format!("No preset {name}"))?;
        let hide = match e.origin {
            Origin::Factory => Some(e.sound.name.clone()),
            Origin::Edited => e.replaces.clone(),
            Origin::User => None,
        };
        // Hidden first: an overlay that cannot be written leaves the user's file.
        if let Some(h) = hide {
            self.change_overlay(|p| change_marks(p, &h, |m| set_mark(m, "hidden", true)))?;
        }
        if let Some(f) = &e.file {
            self.delete(f)?;
        }
        Ok(())
    }

    /// The user's version of a factory preset taken back: the factory's shown again (its
    /// name).
    pub fn revert(&self, name: &str) -> Result<String, String> {
        let e = self.find(name).ok_or_else(|| format!("No preset {name}"))?;
        let (Origin::Edited, Some(file), Some(factory)) = (e.origin, &e.file, &e.replaces) else {
            return Err(format!("{name} is not a factory preset edited"));
        };
        self.change_overlay(|p| change_marks(p, factory, |m| set_mark(m, "hidden", false)))?;
        self.delete(file)?;
        Ok(factory.clone())
    }

    /// The factory presets deleted shown again: their names.
    pub fn restore(&self) -> Result<Vec<String>, String> {
        let mut restored = Vec::new();
        self.change_overlay(|p| {
            for (name, m) in p.iter_mut() {
                if let Value::Table(m) = m
                    && m.get("hidden").and_then(Value::as_bool) == Some(true)
                {
                    m.remove("hidden");
                    restored.push(name.clone());
                }
            }
        })?;
        Ok(restored)
    }
}

/// Read this exact user-preset file with the browser's parser, without searching its
/// directory or resolving factory overlays or another preset with the same name.
pub(crate) fn parse_user_sound(text: &str) -> Result<Sound, String> {
    parse_user(text).map(|(sound, ..)| sound)
}

fn read_user(path: &Path) -> Result<(Sound, bool, Option<String>), String> {
    let s = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    parse_user(&s)
}

fn parse_user(s: &str) -> Result<(Sound, bool, Option<String>), String> {
    let t = s.parse::<toml::Table>().map_err(|e| e.to_string())?;
    match t.get("format").and_then(Value::as_integer) {
        Some(FORMAT) | None => {}
        Some(other) => return Err(format!("format {other}")),
    }
    let mut sound = sound_of(&t)?;
    sound.effects = effects_section(s);
    check_name(&sound.name).map_err(|e| format!("{:?}: {e}", sound.name))?;
    let fav = t.get("favorite").and_then(Value::as_bool).unwrap_or(false);
    let rep = t.get("replaces").and_then(Value::as_str).map(str::to_owned);
    Ok((sound, fav, rep))
}

/// The presets of `list` that match: each of `query`'s words in the name, the description or
/// a tag (ignoring case); every one of `tags`; favourites only; the user's only (and the
/// factory's edited).
pub fn matching(
    list: &[Entry],
    query: &str,
    tags: &[String],
    favorite: bool,
    mine: bool,
) -> Vec<Entry> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    list.iter()
        .filter(|e| {
            let text = format!(
                "{} {} {}",
                e.sound.name,
                e.sound.description,
                e.sound.tags.join(" ")
            )
            .to_lowercase();
            words.iter().all(|w| text.contains(w.as_str()))
                && tags.iter().all(|t| e.sound.tags.iter().any(|x| same(x, t)))
                && (!favorite || e.favorite)
                && (!mine || e.origin != Origin::Factory)
        })
        .cloned()
        .collect()
}

/// Every tag `list` uses, by name.
pub fn tags_of(list: &[Entry]) -> Vec<String> {
    let mut all: Vec<String> = Vec::new();
    for e in list {
        for t in &e.sound.tags {
            if !all.iter().any(|a| same(a, t)) {
                all.push(t.clone());
            }
        }
    }
    all.sort_by_key(|t| t.to_lowercase());
    all
}

/// A table written whole, then moved into place.
pub(crate) fn write(file: &Path, t: &toml::Table) -> Result<(), String> {
    let text = toml::to_string(t).map_err(|e| e.to_string())?;
    write_text(file, &text)
}

fn write_text(file: &Path, text: &str) -> Result<(), String> {
    if let Some(d) = file.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    // A temporary name of its own for each write, this process's and its count: with one name
    // for all, two processes (two hosts, or a host's own plug-in processes) could rename a
    // file the other was still writing into place (the CA-74's review).
    static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = file.with_extension(format!("toml.{}-{n}.tmp", std::process::id()));
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, file).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", file.display())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[Entry]) -> Vec<&str> {
        list.iter().map(|e| e.sound.name.as_str()).collect()
    }

    /// Factory sounds must work without any external plug-in installed (R-FINAL-PRESETS).
    /// Inspect the source too: a section silently ignored by a reader must not ship either.
    #[test]
    fn factory_presets_never_include_hosted_effects() {
        let raw: toml::Table = FACTORY_FILE.parse().unwrap();
        assert!(
            !raw.contains_key("effects"),
            "factory-level effects section"
        );
        for preset in raw["preset"].as_array().unwrap() {
            let table = preset.as_table().unwrap();
            assert!(
                !table.contains_key("effects"),
                "{} includes a hosted effects section",
                table["name"]
            );
        }
        for sound in factory() {
            assert!(sound.effects.is_none(), "{} includes effects", sound.name);
        }
    }

    #[test]
    fn effects_are_opaque_and_keep_every_byte_through_library_edits() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::at(dir.path());
        let raw = "\n# Kept by readers that cannot host it.\n[effects] # future fields too\nchain = '{\"format\":1,\"slots\":[]}'\nunknown = 123\n\n[effects.future]\nvalue = [1, 2, 3] # exactly as written\n# The end.\n";
        let file = dir.path().join("Opaque.toml");
        std::fs::write(
            &file,
            format!("format = 1\nname = 'Opaque'\n[values]\ncutoff = 5\n{raw}"),
        )
        .unwrap();
        let entry = lib.find("Opaque").unwrap();
        assert_eq!(entry.sound.effects.as_deref(), Some(raw));
        lib.save("Opaque", vec![("cutoff".into(), 7.0)], None)
            .unwrap();
        assert!(std::fs::read_to_string(&file).unwrap().ends_with(raw));
        lib.rename("Opaque", "Renamed").unwrap();
        let renamed = lib.find("Renamed").unwrap();
        assert_eq!(renamed.sound.effects.as_deref(), Some(raw));
        assert!(factory().iter().all(|s| s.effects.is_none()));
    }

    #[test]
    fn effects_keep_mixed_line_endings_and_header_text_inside_strings() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::at(dir.path());
        let template = "\n# untouched\n[effects] # here\nchain = '''{\"format\":1,\"slots\":[]}'''\ntext = '''\n[values]\nnot_a_header = true\n'''\n[[effects.future]]\nvalue = 0xDEAD # as typed\n";
        for (n, raw) in [
            template.to_owned(),
            template.replace('\n', "\r\n"),
            template.replace("# here\n", "# here\r\n"),
        ]
        .into_iter()
        .enumerate()
        {
            let file = dir.path().join(format!("Lines{n}.toml"));
            let name = format!("Lines{n}");
            std::fs::write(
                &file,
                format!("format = 1\nname = '{name}'\n{raw}\n[values]\ncutoff = 5\n"),
            )
            .unwrap();
            let e = lib.find(&name).unwrap();
            assert_eq!(e.sound.effects.as_deref(), Some(raw.as_str()));
            lib.save(&name, vec![("cutoff".into(), 7.0)], None).unwrap();
            assert!(std::fs::read_to_string(file).unwrap().ends_with(&raw));
        }
    }

    #[test]
    fn the_factory_file_is_read_whole() {
        let f = factory();
        assert_eq!(f.len(), 42);
        assert!(f.iter().any(|s| s.name == "Gatekeeper"));
        assert!(!f.iter().any(|s| s.name == "Gated Chords"));
        assert_eq!(f[0].name, "Felt Bass");
        assert!(f.iter().all(|s| !s.values.is_empty()));
        assert!(
            f.iter()
                .all(|s| s.values.iter().all(|(k, _)| !KEPT.contains(&k.as_str())))
        );
    }

    /// The factory file ships inside the plug-in whole, its comments too: no trademark, no
    /// other product's or artist's name, no private record (decisions.md R1).
    /// Stacked Fifths (0.1's, retired since: R63) a fourth and a fifth over its root to
    /// the cent (the owner, 2026-10-10: "Become a true 4th and 5th"), the presets' FREQUENCY
    /// values their pots' positions (decisions.md R54); Cycle of Fifths' a fifth 16 cents wide,
    /// as its owner set it.
    #[test]
    fn stacked_fifths_is_a_fourth_and_a_fifth() {
        use ca72::freq_dial::Pot;
        let at = |name: &str, id: &str, pot: Pot| {
            let retired = parse_factory(RETIRED_FILE).unwrap();
            let s = factory()
                .iter()
                .chain(&retired)
                .find(|s| s.name == name)
                .expect("a preset");
            let (_, v) = s.values.iter().find(|(k, _)| k == id).expect("a value");
            pot.semitones(crate::params::wiper_of(*v as f32))
        };
        let fourth = at("Stacked Fifths", "osc2_frequency", Pot::Two);
        let fifth = at("Stacked Fifths", "osc3_frequency", Pot::Three);
        assert!(
            (fourth - 5.0).abs() < 1e-3 && (fifth - 7.0).abs() < 1e-3,
            "{fourth} {fifth}"
        );
        let cycle = at("Cycle of Fifths", "osc2_frequency", Pot::Two);
        assert!((cycle - 7.16).abs() < 0.01, "{cycle}");
    }

    #[test]
    fn the_factory_file_names_no_trademark() {
        // Its words: the comments, the names, descriptions and tags (not the values); the
        // retired presets' too, which every release's pack ships.
        for (file, text) in [("presets", FACTORY_FILE), ("retired", RETIRED_FILE)] {
            let text = text
                .lines()
                .filter(|l| l.starts_with('#') || l.contains('"'))
                .collect::<Vec<_>>()
                .join("\n")
                .to_lowercase();
            for word in ["moog", "model d", "808", "909", "303", "roland", "minimoog"] {
                assert!(!text.contains(word), "{file}.toml says {word:?}");
            }
        }
    }

    /// The presets retired from the plug-in (decisions.md R55), which no build reads but
    /// the pack's: read whole, each name one a library takes and none twice, none a factory
    /// preset's (so a session's preset name means one sound), each setting only the sound's
    /// parameters.
    const RETIRED_FILE: &str = include_str!("../sounds/retired.toml");

    #[test]
    fn the_retired_presets_are_well_formed() {
        let retired = parse_factory(RETIRED_FILE).unwrap();
        let p = crate::params::Ca72Params::default();
        let ids: Vec<&str> = crate::presets::sound_params(&p)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        for (i, s) in retired.iter().enumerate() {
            check_name(&s.name).unwrap();
            assert!(
                !retired[..i].iter().any(|r| same(&r.name, &s.name)),
                "{}",
                s.name
            );
            assert!(
                !factory().iter().any(|f| same(&f.name, &s.name)),
                "{}",
                s.name
            );
            assert!(!s.values.is_empty(), "{}", s.name);
            for (k, _) in &s.values {
                assert!(ids.contains(&k.as_str()), "{}: {k}", s.name);
            }
        }
    }

    /// The pack (decisions.md R55) copied into a library: its presets the user's, tagged
    /// `0.1`; the player's own file of a name it has too (their edit of a preset since retired)
    /// neither written over nor hidden, the pack's reported as a second; and a pack's preset
    /// saved again moves to the file of its name.
    #[test]
    fn the_pack_reads_back_as_the_users_presets_and_writes_over_none() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::at(dir.path());
        lib.save("Old Pluck", vec![("cutoff".into(), 1.5)], None)
            .unwrap();
        let own = std::fs::read(dir.path().join("Old Pluck.toml")).unwrap();
        let sound = |name: &str, cutoff: f64| Sound {
            name: name.into(),
            description: "Retired.".into(),
            tags: vec!["bass".into()],
            values: vec![("cutoff".into(), cutoff), ("osc1_range".into(), 3.0)],
            drive_curve: None,
            drive_curve_lo: None,
            macro_names: Vec::new(),
            effects: None,
        };
        let pack = [sound("Old Pluck", -3.0), sound("Old Pad", 0.25)];
        assert_eq!(write_pack(dir.path(), &pack).unwrap().len(), 2);
        assert_eq!(
            std::fs::read(dir.path().join("Old Pluck.toml")).unwrap(),
            own
        );
        let (_, bad) = lib.entries();
        assert_eq!(bad.len(), 1, "{bad:?}");
        assert!(bad[0].contains("Old Pluck_0.1.toml"), "{bad:?}");
        assert_eq!(
            lib.find("Old Pluck").unwrap().sound.values,
            [("cutoff".to_owned(), 1.5)]
        );
        let pad = lib.find("old pad").unwrap();
        assert_eq!(pad.origin, Origin::User);
        assert_eq!(pad.sound.description, "Retired.");
        assert_eq!(pad.sound.tags, ["bass", "0.1"]);
        assert_eq!(pad.sound.values, sound("Old Pad", 0.25).values);
        lib.save("Old Pad", pad.sound.values, None).unwrap();
        assert!(dir.path().join("Old Pad.toml").exists());
        assert!(!dir.path().join("Old Pad_0.1.toml").exists());
        assert_eq!(lib.find("Old Pad").unwrap().sound.tags, ["bass", "0.1"]);
    }

    #[test]
    fn the_users_presets_are_saved_found_tagged_renamed_and_removed() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::at(dir.path());
        let values = vec![("cutoff".to_owned(), -2.5), ("osc1_range".to_owned(), 3.0)];
        let e = lib
            .save(
                "Deep Bass",
                values,
                Some(parse_tags("bass, midnight, Bass").unwrap()),
            )
            .unwrap();
        assert_eq!(e.origin, Origin::User);
        assert_eq!(e.sound.tags, ["bass", "midnight"]);
        // The file as the DAW reads it.
        let t: toml::Table = std::fs::read_to_string(dir.path().join("Deep Bass.toml"))
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(t["format"].as_integer(), Some(1));
        assert_eq!(t["values"]["cutoff"].as_float(), Some(-2.5));
        assert_eq!(t["values"]["osc1_range"].as_integer(), Some(3));
        assert_eq!(
            names(&lib.search("ROUND deep", &[], false, false)),
            Vec::<&str>::new()
        );
        // (Deep Boom, a factory preset since 0.3.0.)
        assert_eq!(
            names(&lib.search("deep", &[], false, false)),
            ["Deep Bass", "Deep Boom"]
        );
        assert_eq!(
            names(&lib.search("", &["midnight".into()], false, false)),
            ["Deep Bass"]
        );
        assert_eq!(names(&lib.search("", &[], false, true)), ["Deep Bass"]);
        lib.favorite("deep bass", true).unwrap();
        lib.favorite("Singing Drift", true).unwrap();
        assert_eq!(
            names(&lib.search("", &[], true, false)),
            ["Deep Bass", "Singing Drift"]
        );
        lib.tag("Felt Bass", vec!["bass".into(), "mine".into()])
            .unwrap();
        assert_eq!(
            names(&lib.search("", &["mine".into()], false, false)),
            ["Felt Bass"]
        );
        assert!(lib.tags().contains(&"midnight".to_owned()));
        lib.rename("Deep Bass", "Deeper Bass").unwrap();
        assert!(!dir.path().join("Deep Bass.toml").exists());
        assert!(lib.find("Deeper Bass").unwrap().favorite);
        assert!(lib.rename("Deeper Bass", "singing drift").is_err());
        lib.remove("Deeper Bass").unwrap();
        assert!(lib.search("", &[], false, true).is_empty());
        assert!(check_name("a/b").is_err() && check_name(" x").is_err() && check_name("").is_err());
    }

    #[test]
    fn a_factory_preset_edited_renamed_or_removed_comes_back() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::at(dir.path());
        let edited = lib
            .save("singing drift", vec![("emphasis".into(), 9.0)], None)
            .unwrap();
        assert_eq!(edited.origin, Origin::Edited);
        assert_eq!(edited.replaces.as_deref(), Some("Singing Drift"));
        assert_eq!(edited.sound.tags, ["lead", "glide", "prog"]);
        assert_eq!(
            lib.entries()
                .0
                .iter()
                .filter(|e| same(&e.sound.name, "singing drift"))
                .count(),
            1
        );
        assert_eq!(lib.revert("singing drift").unwrap(), "Singing Drift");
        assert_eq!(lib.find("Singing Drift").unwrap().origin, Origin::Factory);
        lib.rename("Felt Bass", "Round Bass").unwrap();
        assert!(lib.find("Felt Bass").is_none());
        assert_eq!(lib.find("Round Bass").unwrap().origin, Origin::Edited);
        lib.revert("Round Bass").unwrap();
        assert!(lib.find("Felt Bass").is_some());
        lib.remove("Felt Bass").unwrap();
        lib.save("Singing Drift", vec![], None).unwrap();
        lib.remove("Singing Drift").unwrap();
        assert!(lib.find("Felt Bass").is_none() && lib.find("Singing Drift").is_none());
        assert!(!dir.path().join("Singing Drift.toml").exists());
        assert_eq!(lib.restore().unwrap(), ["Felt Bass", "Singing Drift"]);
        assert_eq!(lib.entries().0.len(), factory().len());
        assert!(lib.revert("Felt Bass").is_err());
    }

    /// `factory` is the folder's own file (the factory presets' marks): no preset takes the
    /// name, in any case, and the marks survive the try (decisions.md R18).
    #[test]
    fn factory_is_no_presets_name() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::at(dir.path());
        lib.favorite("Singing Drift", true).unwrap();
        lib.tag("Felt Bass", vec!["mine".into()]).unwrap();
        for n in ["factory", "Factory", "FACTORY"] {
            assert!(
                lib.save(n, vec![("cutoff".into(), 1.0)], None).is_err(),
                "{n}"
            );
            assert!(lib.rename("Singing Drift", n).is_err(), "{n}");
        }
        assert!(lib.find("Singing Drift").unwrap().favorite);
        assert_eq!(lib.find("Felt Bass").unwrap().sound.tags, ["mine"]);
        assert!(lib.save("Factory Floor", vec![], None).is_ok());
    }

    /// Windows's device names (an extension or not, any case) are no preset's on any system,
    /// nor are names starting or ending with a space or a dot (decisions.md R18).
    #[test]
    fn windows_device_names_are_refused_everywhere() {
        for n in [
            "CON",
            "con",
            "Prn",
            "AUX",
            "NUL",
            "nul.txt",
            "NUL.tar.gz",
            "COM1",
            "com9",
            "COM0",
            "LPT1",
            "lpt9.x",
            "COM¹",
            "LPT³",
            "CONIN$",
            "conout$",
            "Aux .x",
            "Dot.",
            "Space ",
            " Space",
            ".Dot",
        ] {
            assert!(check_name(n).is_err(), "{n:?}");
        }
        for n in [
            "CONSOLE",
            "Nullify",
            "COM10",
            "LPT",
            "Com Bass",
            "Auxiliary Lead",
            "a.con",
            "Bass 2",
        ] {
            assert!(check_name(n).is_ok(), "{n:?}");
        }
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::at(dir.path());
        assert!(lib.save("aux", vec![], None).is_err());
        assert!(lib.rename("Singing Drift", "NUL.lead").is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    /// A file's `name` is checked as a typed one: a name reaching out of the folder, the
    /// folder's own file's or a device's is reported, not listed (decisions.md R18).
    #[test]
    fn a_files_unsafe_name_is_reported_not_listed() {
        let dir = tempfile::tempdir().unwrap();
        for (file, name) in [
            ("up.toml", "../../outside"),
            ("root.toml", "/tmp/outside"),
            ("device.toml", "NUL"),
            ("own.toml", "factory"),
            ("fine.toml", "Fine"),
        ] {
            std::fs::write(
                dir.path().join(file),
                format!("format = 1\nname = {name:?}\n\n[values]\ncutoff = 1\n"),
            )
            .unwrap();
        }
        let lib = Library::at(dir.path());
        let (list, bad) = lib.entries();
        let mine: Vec<&str> = list
            .iter()
            .filter(|e| e.origin == Origin::User)
            .map(|e| e.sound.name.as_str())
            .collect();
        assert_eq!(mine, ["Fine"]);
        assert_eq!(bad.len(), 4, "{bad:?}");
    }

    /// An overlay that cannot be read (a hand edit gone wrong, a newer format) is never
    /// written over: marking, deleting or restoring a factory preset says why and changes
    /// nothing, the user's version deleted included; the presets still list (decisions.md
    /// R18).
    #[test]
    fn an_unreadable_overlay_is_left_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::at(dir.path());
        lib.save("Singing Drift", vec![("cutoff".into(), 1.0)], None)
            .unwrap();
        let overlay = dir.path().join("factory.toml");
        for text in [
            "format = 1\n[presets.\"Felt Bass\"\nfavorite = true\n",
            "format = 2\n[presets.\"Felt Bass\"]\nfavorite = true\nrating = 5\n",
            "format = 1\npresets = 3\n",
        ] {
            std::fs::write(&overlay, text).unwrap();
            let e = lib.favorite("Felt Bass", true).unwrap_err();
            assert!(e.contains("factory.toml cannot be read"), "{e}");
            assert!(lib.tag("Felt Bass", vec!["x".into()]).is_err());
            assert!(lib.remove("Felt Bass").is_err());
            assert!(lib.remove("Singing Drift").is_err());
            assert!(lib.revert("Singing Drift").is_err());
            assert!(lib.restore().is_err());
            assert_eq!(std::fs::read_to_string(&overlay).unwrap(), text);
            assert!(dir.path().join("Singing Drift.toml").exists());
            let (list, bad) = lib.entries();
            assert_eq!(list.len(), factory().len());
            assert_eq!(bad.len(), 1, "{bad:?}");
        }
    }

    /// What the overlay holds beyond these marks (another program's keys, a key in another
    /// case) is kept when it is written again (decisions.md R18).
    #[test]
    fn the_overlay_keeps_what_it_does_not_know() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::at(dir.path());
        let overlay = dir.path().join("factory.toml");
        std::fs::write(
            &overlay,
            "format = 1\nnote = \"kept\"\n\n[presets.\"Singing Drift\"]\nfavorite = true\nrating = 5\n\n[presets.\"felt bass\"]\nhidden = true\ntags = [\"low\"]\n",
        )
        .unwrap();
        assert!(lib.find("Singing Drift").unwrap().favorite);
        assert!(
            lib.find("Felt Bass").is_none(),
            "hidden whatever the key's case"
        );
        lib.favorite("Singing Drift", false).unwrap();
        assert_eq!(lib.restore().unwrap(), ["felt bass"]);
        assert_eq!(lib.find("Felt Bass").unwrap().sound.tags, ["low"]);
        lib.tag("Felt Bass", vec!["round".into()]).unwrap();
        let t: toml::Table = std::fs::read_to_string(&overlay).unwrap().parse().unwrap();
        assert_eq!(t["note"].as_str(), Some("kept"));
        assert_eq!(
            t["presets"]["Singing Drift"]["rating"].as_integer(),
            Some(5)
        );
        assert!(t["presets"]["Singing Drift"].get("favorite").is_none());
        assert_eq!(t["presets"]["felt bass"]["tags"][0].as_str(), Some("round"));
        assert!(t["presets"].get("Felt Bass").is_none());
        assert_eq!(lib.find("Felt Bass").unwrap().sound.tags, ["round"]);
    }

    /// Legacy amounts are translated once, including an absent ENTROPY. Rewriting metadata
    /// writes the migrated sound with its writer version, so reopening cannot shift it again.
    #[test]
    fn old_presets_move_to_the_new_entropy_scale_once() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Old.toml");
        std::fs::write(
            &file,
            "format = 1\nname = \"Old\"\n[values]\nentropy = 40\ncutoff = 2\n",
        )
        .unwrap();
        let lib = Library::at(dir.path());
        let entropy = |s: &Sound| s.values.iter().find(|(k, _)| k == "entropy").unwrap().1;
        let old = lib.find("Old").unwrap();
        assert_eq!(entropy(&old.sound), 43.0);
        assert_eq!(
            old.sound
                .values
                .iter()
                .find(|(k, _)| k == "cutoff")
                .unwrap()
                .1,
            2.0
        );
        lib.favorite("Old", true).unwrap();
        let written = std::fs::read_to_string(&file).unwrap();
        let table: toml::Table = written.parse().unwrap();
        assert_eq!(table["version"].as_str(), Some(env!("CARGO_PKG_VERSION")));
        assert_eq!(
            entropy(&Library::at(dir.path()).find("Old").unwrap().sound),
            43.0
        );
        for (version, value, want) in [
            ("", "", 5.0),
            ("0.2.6", "entropy = 0", 5.0),
            ("0.2.7", "entropy = 0", 0.0),
        ] {
            let version = if version.is_empty() {
                String::new()
            } else {
                format!("version = {version:?}\n")
            };
            let text = format!(
                "format = 1\n{version}[[preset]]\nname = \"Test\"\n[preset.values]\n{value}\n"
            );
            let sounds = parse_factory(&text).unwrap();
            assert_eq!(entropy(&sounds[0]), want, "{text}");
        }
    }

    fn toml_files(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".toml"))
            .collect();
        v.sort();
        v
    }

    /// A star or tags go to the preset's own file, whatever its name says: one file, one row,
    /// and nothing written where the name alone would put it (decisions.md R18).
    #[test]
    fn marks_go_to_the_presets_own_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("my_patch.toml"),
            "format = 1\nname = \"My Patch\"\n\n[values]\ncutoff = 1\n",
        )
        .unwrap();
        let lib = Library::at(dir.path());
        lib.favorite("My Patch", true).unwrap();
        lib.tag("my patch", vec!["round".into()]).unwrap();
        assert_eq!(toml_files(dir.path()), ["my_patch.toml"]);
        let e = lib.find("My Patch").unwrap();
        assert!(e.favorite && e.sound.tags == ["round"]);
        assert_eq!(lib.search("", &[], false, true).len(), 1);
        // Saved again, it takes its name's file (nothing else there), still one.
        lib.save("My Patch", vec![("cutoff".into(), 2.0)], None)
            .unwrap();
        assert_eq!(toml_files(dir.path()), ["My Patch.toml"]);
        assert!(lib.find("My Patch").unwrap().favorite);
    }

    /// A name changed only in case leaves one file, on a file system that tells cases apart
    /// too (decisions.md R18).
    #[test]
    fn a_name_changed_in_case_leaves_one_file() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::at(dir.path());
        lib.save(
            "Deep Bass",
            vec![("cutoff".into(), 1.0)],
            Some(vec!["low".into()]),
        )
        .unwrap();
        lib.save("deep bass", vec![("cutoff".into(), 2.0)], None)
            .unwrap();
        assert_eq!(toml_files(dir.path()), ["deep bass.toml"]);
        let e = lib.find("Deep Bass").unwrap();
        assert_eq!(e.sound.name, "deep bass");
        assert_eq!(e.sound.tags, ["low"]);
        assert!(e.sound.values.contains(&("cutoff".to_owned(), 2.0)));
        lib.rename("deep bass", "DEEP BASS").unwrap();
        assert_eq!(toml_files(dir.path()), ["DEEP BASS.toml"]);
        assert_eq!(lib.search("", &[], false, true).len(), 1);
    }

    /// Two files of one name (in any case) list once, the other reported; a file in the way
    /// of a name (not the preset's) is never written over (decisions.md R18).
    #[test]
    fn a_name_is_one_presets_and_another_file_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let preset = |file: &str, name: &str| {
            std::fs::write(
                dir.path().join(file),
                format!("format = 1\nname = {name:?}\n\n[values]\ncutoff = 1\n"),
            )
            .unwrap();
        };
        preset("a.toml", "Twin");
        preset("b.toml", "twin");
        preset("c.toml", "Third");
        std::fs::write(dir.path().join("Third.toml"), "name = ").unwrap();
        std::fs::write(dir.path().join("Fourth.toml"), "format = 9\n").unwrap();
        let lib = Library::at(dir.path());
        let (list, bad) = lib.entries();
        let twins = list.iter().filter(|e| same(&e.sound.name, "twin")).count();
        assert_eq!(twins, 1);
        assert_eq!(bad.len(), 3, "{bad:?}");
        // Third's own file takes it; the file of its name is not its own and stays.
        lib.save("Third", vec![("cutoff".into(), 3.0)], None)
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("Third.toml")).unwrap(),
            "name = "
        );
        assert!(
            lib.find("Third")
                .unwrap()
                .sound
                .values
                .contains(&("cutoff".to_owned(), 3.0))
        );
        assert!(lib.save("Fourth", vec![], None).is_err());
        assert!(lib.rename("Twin", "fourth").is_ok(), "into its own file");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("Fourth.toml")).unwrap(),
            "format = 9\n"
        );
    }

    /// A user's preset that `replaces` a factory one, in another case, stands in for it as
    /// the factory spells it: deleting it hides that one (decisions.md R18).
    #[test]
    fn replaces_is_the_factorys_name_in_any_case() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Mine.toml"),
            "format = 1\nname = \"Mine\"\nreplaces = \"singing drift\"\n\n[values]\ncutoff = 1\n",
        )
        .unwrap();
        let lib = Library::at(dir.path());
        let e = lib.find("Mine").unwrap();
        assert_eq!(
            (e.origin, e.replaces.as_deref()),
            (Origin::Edited, Some("Singing Drift"))
        );
        assert!(lib.find("Singing Drift").is_none());
        lib.remove("Mine").unwrap();
        assert!(lib.find("Singing Drift").is_none() && lib.find("Mine").is_none());
        assert_eq!(lib.restore().unwrap(), ["Singing Drift"]);
    }

    /// The folder is parsed again only when it changed: a file of the same name, size and
    /// time is taken as read; another time, a file more, or a write of its own is read
    /// (decisions.md R18).
    #[test]
    fn the_folder_is_read_again_only_when_it_changes() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::at(dir.path());
        let file = dir.path().join("Pad.toml");
        let text = "format = 1\nname = \"Pad\"\n\n[values]\ncutoff = 1\n";
        std::fs::write(&file, text).unwrap();
        assert!(lib.find("Pad").is_some());
        let then = std::fs::metadata(&file).unwrap().modified().unwrap();
        let touch = |t: SystemTime| {
            std::fs::File::options()
                .write(true)
                .open(&file)
                .unwrap()
                .set_modified(t)
                .unwrap();
        };
        // Garbage of the same size at the same time: not parsed, Pad as read.
        std::fs::write(&file, "#".repeat(text.len())).unwrap();
        touch(then);
        assert!(
            lib.find("Pad").is_some(),
            "parsed again, the folder unchanged"
        );
        assert_eq!(lib.search("pad", &[], false, true).len(), 1);
        // Another time: parsed, and it has no name.
        touch(then + std::time::Duration::from_secs(2));
        assert!(lib.find("Pad").is_none());
        assert_eq!(lib.entries().1.len(), 1);
        // A file more, and the library's own writes.
        lib.save("Lid", vec![], Some(vec!["new".into()])).unwrap();
        assert!(lib.tags().contains(&"new".to_owned()));
        lib.favorite("Lid", true).unwrap();
        assert!(lib.find("Lid").unwrap().favorite);
    }

    /// The DAW's files (written by its own TOML writer) read here, and an unreadable one
    /// reported, not fatal.
    #[test]
    fn a_preset_written_by_the_daw_is_read() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Wide Pad.toml"),
            "format = 1\nname = \"Wide Pad\"\ntags = [\"pad\"]\nfavorite = false\n\n[values]\ncutoff = 1.5\nloudness_attack = 6\npoly = 1\nvoices = 6\nentropy = 40\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("Broken.toml"), "name = ").unwrap();
        let lib = Library::at(dir.path());
        let (list, bad) = lib.entries();
        let pad = list.iter().find(|e| e.sound.name == "Wide Pad").unwrap();
        assert_eq!(pad.origin, Origin::User);
        assert!(pad.sound.values.contains(&("voices".to_owned(), 6.0)));
        assert_eq!(bad.len(), 1);
    }

    /// R-AUTO-GAIN-DRIVE: a preset saved with AUTO GAIN's curve reads back with it, at the
    /// quality it was heard at, each correction to the bit; one saved before has none, and one
    /// not understood is none too (it plays with the curve in force).
    #[test]
    fn a_presets_curve_is_saved_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::at(dir.path());
        let db = [-5.962_001_f32, -11.82312, -17.329_067, -21.718_765];
        let e = lib
            .save_with_curve(
                "Curved",
                vec![("drive".into(), 9.9)],
                None,
                Some((true, db)),
            )
            .unwrap();
        assert_eq!(
            (e.sound.drive_curve, e.sound.drive_curve_lo),
            (None, Some(db))
        );
        assert_eq!(
            e.sound.curve(false),
            Some(db),
            "the other quality's stands in"
        );
        let e = lib
            .save_with_curve(
                "Curved",
                vec![("drive".into(), 9.9)],
                None,
                Some((false, db)),
            )
            .unwrap();
        assert_eq!(
            (e.sound.drive_curve, e.sound.drive_curve_lo),
            (Some(db), None)
        );
        let reread = Library::at(dir.path()).find("Curved").unwrap();
        assert_eq!(reread.sound.drive_curve, Some(db));
        for (name, curve) in [
            ("Before", ""),
            ("Short", "drive_curve = [-1.0, -2.0]\n"),
            ("Words", "drive_curve = [\"a\", -2.0, -3.0, -4.0]\n"),
        ] {
            std::fs::write(
                dir.path().join(format!("{name}.toml")),
                format!("format = 1\nversion = \"0.2.7\"\nname = {name:?}\n{curve}\n[values]\ndrive = 6\n"),
            )
            .unwrap();
            let e = Library::at(dir.path()).find(name).unwrap();
            assert_eq!(
                (e.sound.drive_curve, e.sound.drive_curve_lo),
                (None, None),
                "{name}"
            );
        }
    }
}

/// Each program reads the other's files: with `CA72_CROSS_DIR` set (a library folder), writes
/// a preset there as the plug-in does and lists what is there (the DAW's included).
#[cfg(test)]
mod cross {
    use super::*;

    #[test]
    #[ignore = "a check against the DAW's files, run by hand"]
    fn cross_check() {
        let Some(dir) = std::env::var_os("CA72_CROSS_DIR") else {
            return;
        };
        let lib = Library::at(dir);
        lib.save(
            "From The Plug-in",
            vec![
                ("cutoff".into(), -1.25),
                ("osc1_range".into(), 1.0),
                ("voices".into(), 6.0),
            ],
            Some(vec!["cross".into()]),
        )
        .unwrap();
        lib.favorite("Feedback Funk", true).unwrap();
        for e in lib
            .entries()
            .0
            .iter()
            .filter(|e| e.origin != Origin::Factory || e.favorite)
        {
            eprintln!(
                "CROSS {} {:?} fav={} tags={:?} values={}",
                e.sound.name,
                e.origin,
                e.favorite,
                e.sound.tags,
                e.sound.values.len()
            );
        }
    }
}
