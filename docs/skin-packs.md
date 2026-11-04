# Skin packs

*This document, and the starter pack in [skin-packs/starter](skin-packs/starter), are dedicated to
the public domain under [CC0 1.0](https://creativecommons.org/publicdomain/zero/1.0/): follow
them, copy them and build on them under any terms you like. The CA-72 itself, its code and its
official artwork are under the GNU General Public License, version 3 or later.*

**Status: a prototype (decisions.md R-SKIN-PACKS).** The format, the checks and the CA-72's
support for packs exist on the `skin-system` branch, with tests. Nothing has been released:
there is no store, no seller agreement and no review process yet. Manual, unsigned packs remain
the main route. The first-party public key is to ship in 0.3.0 so Idle Foundry can publish signed
packs later without a plug-in update. The format may still change before its first release.

A skin pack changes how the CA-72's editor looks: where each control is on the panel, how each
looks, every legend, rule and title printed on it, the pictures its realistic look is made of,
a drawing over the panel, and its typeface. A pack holds **artwork and declarative data only,
never code**. The plug-in's own open-source renderer draws the controls and decides what each
one does. A pack cannot change the sound, the presets, MIDI, automation or what a session saves.

The CA-72's own looks, CLASSIC and REALISTIC, are packs in this format. They are built into the
plug-in under the GPL; `cargo run -p ca72-panel --example export_pack -- realistic <folder>`
writes one out as a folder to study. Its id (`com.idlefoundry.*`) is reserved for the built-in
packs, so a copy must be given another id before it will load.

## Licences: the pack's are its own

- **A pack's licence is its creator's choice**, named in its manifest: an open licence (CC0,
  CC BY, CC BY-SA, the GPL, MIT, ...), one with restrictions (CC's NonCommercial or
  NoDerivatives), or the creator's own terms (`LicenseRef-...`, the text in the pack).
- **Price and licence are independent.** An open pack may be sold, and a proprietary one given
  away. A pack never states a price: that belongs to the store. A manifest with `price`,
  `currency`, `sku`, `license_key` or `drm` is rejected.
- **The plug-in's licence does not apply to a pack, and a pack's does not apply to the
  plug-in.** Every pack keeps its licence, creators, each asset's provenance and its
  third-party notices in its own manifest and folder.
- **Don't copy the CA-72's artwork into a pack under other terms.** Its pictures, its drawn
  panel and its official layout (`crates/ca72-panel/packs/official/layout.toml`) are under the
  GPL. A pack made from them is under the GPL too, unless Idle Foundry gives additional
  permission. If a pack under another licence contains a byte-for-byte copy of one of the
  plug-in's pictures, the check flags it for review. Start from the [CC0
  starter](skin-packs/starter) instead.
- **Fonts, stock pictures and other makers' branding** go in a pack only when their licences
  permit that distribution, including sale if the pack is sold. A pack never uses a trademark
  it has no right to: no instrument maker's names or logos.
- **Declarations are not clearance.** A seller declares the right to distribute every asset
  under the pack's licence, and authorizes the marketplace to license the pack to its
  customers. The checks record these as declared, never as cleared: a review decides.

## Installing

A pack is a folder while it is being made, and a single **pack file** (`<name>.ifskin`) as
a creator shares it. Put either in the `Skins` folder of Idle Foundry's shared folder:

| System | Folder |
|---|---|
| macOS | `~/Library/Application Support/Idle Foundry/Skins/` |
| Windows | `%APPDATA%\Idle Foundry\Skins\` |
| Linux | `~/.local/share/Idle Foundry/Skins/` (or `$XDG_DATA_HOME/Idle Foundry/Skins/`) |

The settings key on the editor's rail opens its menu, which lists the installed packs under
LOOK after REALISTIC and CLASSIC. A pack that cannot be used is shown with "(CANNOT BE USED)";
the reasons are in the plug-in's log. A verified first-party pack has "(IDLE FOUNDRY)" after its
name; a future marketplace pack has "(MARKETPLACE)". An unknown signing key is labelled
"(UNKNOWN SIGNING KEY)" and treated as unsigned. An unsigned pack is labelled "(UNSIGNED)" and loads as before. Every pack has a status after
its name, so a creator cannot impersonate the verified label by naming an unsigned pack that
way.
No store login or network check is needed to use an installed pack, and none is made.

**Updates.** If the same pack (the same `id`) is installed more than once, the newest version is
used. A pack's update can simply be added beside it, and the setting, which names the pack by
its id, follows it. A signed pack is always preferred over an unsigned copy, whatever version
the copy claims, so a published pack can't be displaced by an unsigned file.

The look chosen is a setting for the computer (`settings.toml`), not something the session
saves. If the chosen pack is missing or refused, the editor draws the pack's base look
(REALISTIC or CLASSIC) and keeps the setting, so the pack is used again once it is back. An
older CA-72 reads only the base look.

## Pack files

A pack file is a ZIP archive of the pack's folder, its files at the top or in one folder at the
top. It is read as the folder is, within the same limits, without being unpacked to disk. On top
of the folder's rules:

- only stored or deflated entries are read;
- encrypted entries and links are refused;
- an archive with more entries than a pack may hold is refused before anything is read;
- each entry is read no further than its size limit, whatever its header claims (a "ZIP bomb"
  stops at the limit).

```bash
cargo run -p skin-pack --example pack_archive -- path/to/pack night.ifskin
```

This writes a folder as a pack file: its files in order, the same bytes every time.

## Signatures

Manual, unsigned packs remain the main route. Idle Foundry can also sign its own packs:
`signature.toml` in the pack, made with its secret key. The plug-in checks it **offline**, against
public keys built into it. There is no login and no network. A signature proves who published
these files. A marketplace could have its own key later, once it has a review process.

| The pack | What the plug-in does |
|---|---|
| Unsigned | Loads it (anyone's pack, made and shared manually). |
| Signed by a key the plug-in does not know (a newer key than an older plug-in has) | Loads it as unsigned; it cannot verify that signature. |
| Signature still names a known key, but any file, id, version, key id or publisher changed since | Refuses it, with `signature.invalid` and advice to get an unchanged copy from its publisher. |
| Signed by a future marketplace key | Loads it as published by the marketplace. That key cannot claim to be Idle Foundry. |
| Signed as first-party by Idle Foundry's key | Loads it as published by Idle Foundry. Only these may use the `com.idlefoundry.*` id prefix. |

The built-in looks' own ids cannot be taken by any installed pack, signed or not.

What is signed is every field of the signature: its key's id, the pack's id and version, the
publisher (`marketplace` or `first-party`), the date, and a digest of every file of the pack
(each path with its SHA-256).

**A signature is neither copy protection nor a licence.** A pack works however it was obtained.
Its licence says what its user may do; a signature grants no additional rights.

## Signing first-party packs

The owner makes the real key pair on a machine of their choice before the 0.3.0 release
candidate. No secret key belongs in any repository, including an ignored folder or a worktree's
`target`. The tools work offline; build them first if the signing machine will be disconnected.
Do not use a throwaway test key in a release.

1. Choose an existing private folder **outside every Git work tree**, then generate the key
   there. This example assumes `$HOME/offline-signing` already exists outside a repository:

   ```sh
   cargo run -p skin-pack --example pack_keygen -- "$HOME/offline-signing/idlefoundry-1.key"
   ```

   The tool refuses paths in work trees (including linked worktrees and paths through links),
   bare repositories, existing files and existing links (including Windows alternate data streams). Git must be available to check the
   destination. It uses the operating system's randomness, creates the secret with owner-only
   access (0600 on macOS/Linux; a protected user-only DACL on Windows, refusing volumes without
   persistent ACLs), and prints only the
   public half and its id, `idlefoundry-1`. It never prints the secret. Keep that file private.

2. Make an **offline backup** of the secret, outside any repository, and keep it securely.
   Keep the signing copy on the machine that signs. Never commit either copy. Losing the secret
   means this key cannot sign future packs; the public half cannot recover it.

3. Paste the printed `pub const IDLE_FOUNDRY_KEY: Option<TrustedKey> = Some(...)` line into
   `crates/skin-pack/src/keys.rs`, replacing its `None` constant. Commit **only the public
   constant**. `ca72_panel::look::TRUSTED_KEYS`, the signer and the release gate share that list.
   Check it before building the release:

   ```sh
   cargo run -p skin-pack --example check_release_key
   ```

   This fails if the public key is missing, weak, not marked first-party, has the wrong id, or
   is absent from the trust list. A public release's builds also fail at compile time while
   the slot is empty: the release sets `CA72_PUBLIC_RELEASE=1` (the Mac's
   `CA72_REQUIRE_NOTARIZATION=1` counts too), and `scripts/package.sh` runs this diagnostic
   under either. CI sets `CA72_PUBLIC_RELEASE=1` when it runs for a version tag (`v*`), for
   the bundles, the installers and the tests alike. Ordinary release and bundle builds, CI's
   for branches and pull requests included, are not blocked, so work continues before the key
   exists.

4. Finish and check the pack, then sign its folder or `.ifskin` (the date is the publication
   date). The secret is read from a file, never as a command-line argument:

   ```sh
   cargo run -p ca72-panel --example check_pack -- path/to/pack
   cargo run -p skin-pack --example pack_sign -- path/to/pack "$HOME/offline-signing/idlefoundry-1.key" idlefoundry-1 first-party 2026-10-18
   ```

   For a first-party `com.idlefoundry.*` id, the unsigned check initially refuses that reserved
   prefix; the signer checks the signed pack with the built-in key, which permits it. All
   other findings still apply. `pack_sign` accepts `.ifskin` in the same position and replaces
   it atomically; for a folder it replaces only `signature.toml`. The signer checks the format
   and verifies the signature against the built-in public keys **before writing**. A missing
   key id or a secret that does not match it refuses the operation.

   To read from stdin, use `-` for the file argument. Redirect from the secret file or a secure
   secret provider; never type the secret into a shell command:

   ```sh
   cargo run -p skin-pack --example pack_sign -- night.ifskin - idlefoundry-1 first-party 2026-10-18 < "$HOME/offline-signing/idlefoundry-1.key"
   ```

5. Verify with the CA-72's full look checks, using the built-in key by default:

   ```sh
   cargo run -p ca72-panel --example check_pack -- night.ifskin
   ```

   It must report `signature.valid`, "published by Idle Foundry", and `LOADS`. No `--trust`
   override belongs in the release verification. After signing, any change to a pack requires
   signing it again. To turn a signed folder into a pack file, use `pack_archive` as above.

**Rotation and a future marketplace.** Add a second public key, such as `idlefoundry-2`, to
`TRUSTED_KEYS` beside the first, with `first_party: true`, and ship it before using it. Keep the
first key for existing packs. A marketplace key would be another entry with its own id and
`first_party: false`; it cannot sign as Idle Foundry. A plug-in learns of a new key or a retired
one only when updated: there is no online revocation. A maker can use `check_pack --trust
<key-id>:<public-key>` for a local check before a public key is built in; this explicitly grants
first-party trust for that check alone and does not change the shipped keys.

## Checking a pack

```bash
cargo run -p ca72-panel --example check_pack -- path/to/pack preview.png
```

This prints what the checks found and draws the panel in the pack's look, in both lettering
sizes. Each finding has a stable code (`svg.refused`, `layout.overlap`, ...) and one of three
severities:

| Severity | What it means |
|---|---|
| **Rejected** | The plug-in will not load the pack. |
| **Review** | The pack loads for its owner, but a marketplace's review must look at this before listing it. |
| **Note** | For the record: its licence's terms, its declarations as declared, a font's vendor. |

## The folder

```
night-panel/
  skin.toml            the manifest (required)
  LICENSE.txt          the pack's licence text
  layout.toml          where everything is (optional: the official layout otherwise)
  art/panel.svg        a drawing over the panel (optional)
  pictures/knob.png    pictures replacing the realistic look's (optional)
  fonts/Face-Bold.otf  a typeface (optional)
  NOTICES.txt          third-party notices (as their licences ask)
  signature.toml       the marketplace's signature (it writes this; see Signatures)
  README.md            anything you like
```

The folder is read as untrusted input:

- **Allowed types:** `.toml`, `.txt` and `.md` (UTF-8 text), `.png`, `.svg`, `.otf` and `.ttf`,
  with lower-case extensions. Anything else is rejected, as is code or an archive whatever
  its name (programs, libraries, scripts, WebAssembly, ZIP files).
- **Names:** ASCII letters, digits, `-`, `_` and `.`. A name must not start with a dot, end with
  one, or be a Windows device name (`con`, `nul`, ...). No two names may differ only in case.
  At most three folders deep, and paths of at most 128 bytes. `.DS_Store`, `Thumbs.db`,
  `desktop.ini` and `__MACOSX` are ignored.
- **No links** of any kind, and no special files.
- **Limits:** 256 files; 8 MiB per file, 256 KiB per text file, 4 MiB per font; 64 MiB in all.
  Pictures at most 4096 pixels a side and 16 million pixels in all (about three times the
  official REALISTIC look's), pictures embedded in drawings counted with them. At most 4 fonts.

## The manifest, `skin.toml`

```toml
format = 1
id = "org.example.night-panel"   # lower case, dotted, yours; com.idlefoundry.* is reserved
name = "NIGHT PANEL"             # as the settings menu lists it
version = "1.0.0"                # major.minor.patch
plugin = "ca-72"
# requires = "ca-72 >= 0.4.0"    # optional: the CA-72 it needs at least (see Compatibility)
summary = "A dark panel."        # optional
license = "CC-BY-4.0"            # an SPDX expression, or LicenseRef-<name>
license_files = ["LICENSE.txt"]  # required for a LicenseRef-, expected for the rest

[[creators]]
name = "A. Maker"
role = "artwork"                 # optional
url = "https://example.org"      # optional

[[assets]]                       # one for each file but the manifest, licences, notices and .md
path = "pictures/knob.png"
origin = "original"              # original | generated | derived | third-party
author = "A. Maker"              # optional
tool = "..."                     # generated: the tool and its version
source = "..."                   # derived or third-party: the work it came from
license = "CC0-1.0"              # if not the pack's own (derived or third-party: required)
sha256 = "..."                   # optional; checked if given
note = "..."                     # optional

[[notices]]                      # a third-party work's notice, as its licence asks
name = "Some Sans"
license = "OFL-1.1"
file = "NOTICES.txt"

[declarations]                   # a seller's; needed before a marketplace lists the pack
rights_to_distribute = true
authorizes_customer_licensing = true
agreement = "2027-01"            # the seller agreement's version, once there is one
declared_by = "A. Maker"
declared_on = "2026-10-16"

[look]                           # the CA-72's table: see below
base = "classic"
```

Unknown fields are rejected. **Licences known to the checks:** CC0-1.0; CC-BY-3.0, CC-BY-4.0,
CC-BY-SA-3.0, CC-BY-SA-4.0; CC-BY-NC-4.0, CC-BY-NC-SA-4.0, CC-BY-ND-4.0, CC-BY-NC-ND-4.0;
GPL-2.0-only, GPL-2.0-or-later, GPL-3.0-only, GPL-3.0-or-later; AGPL-3.0-only,
AGPL-3.0-or-later; LGPL-3.0-or-later; MPL-2.0; MIT; Apache-2.0; BSD-2-Clause, BSD-3-Clause;
ISC; Unlicense; OFL-1.1, OFL-1.1-RFN, OFL-1.1-no-RFN; Bitstream-Vera; with `AND`, `OR`,
parentheses, and `WITH Font-exception-2.0`. A creator's own terms are `LicenseRef-` followed by
a name, with their text in the pack.

**For review:** every file without provenance; every generated asset (whether a picture made
with a generator is protected, and so can be licensed, differs between countries); a derived
or third-party asset without its source or licence; a pack with no creators or no
declarations.

## The look, `[look]`

```toml
[look]
base = "realistic"           # how the controls are drawn: realistic (pictures) or classic (drawn)
layout = "layout.toml"       # optional: the official layout if absent
art = "art/panel.svg"        # optional: drawn over the panel's surfaces, under its print
fonts = { regular = "fonts/Face-Regular.otf", bold = "fonts/Face-Bold.otf" }  # optional
face_tint = [1.03, 1.0, 0.94] # optional: the face picture's channels scaled (0 to 2)
knob_caps = "turn"           # optional: "still" (the default) or "turn" (see Pictures)
strip_art = "art/strip.svg"  # optional: drawn over the plug-in's strip (see The strip)
keyboard_art = "art/keyboard.svg" # optional: drawn over the keyboard view's case (see The strip)
keyboard_case = "face"       # optional: the keyboard view's case in "wood" (the default) or "face"
drawer_art = "art/drawer.svg" # optional: drawn over the presets' drawer's face (see The strip)
drawer_lettering = "type"    # optional: its displays in "dots" (the default) or the typeface
drawer_legends = "caps"      # optional, no longer used: its keys' legends are always on them
drawer_titles = "centre"     # optional: its sections' titles at their "left" (the default) or centred
ticks_and_titles = "quiet"   # optional: the panel's ticks and titles "standard" (the default) or "quiet"

[look.colours]               # optional: the strip's (see The strip), each "#rrggbb"
lamp = "#ffa040"             # its readouts' lit segments, its displays' dots, the voices' drops
drops = "#ffa62a"            # optional: the voices' drops' light (else made from lamp)
tab_print = "#e6dcc4"        # the tabs' lettering
key_print = "#e6dcc4"        # the rail's keys' marks and lettering
wheel = "#c4b08e"            # the PITCH and MOD wheels' colour (they are shaded by the plug-in)
display = "#e6dcc4"          # the drawer's displays' lettering in the typeface, where not lit
selection = "#12303a"        # the drawer's current row's band, in the typeface lettering
velocity = "#ff7028"         # the left hand's VELOCITY: its meter's lamps and its cords (else `lamp`)

[look.pictures]              # optional: realistic only; pictures replacing the built-in ones
knob = "pictures/knob.png"
walnut = "pictures/walnut.svg"
```

- **The drawing** (`art`) is stretched over the panel: the wooden strip and the face, 3108 by
  925 units for the official panel's size (give it `viewBox="0 0 3108 925"`), or 3108 by the
  wooden strip and the face together for a layout's own (see The panel's size). It is drawn over the panel's surfaces (the wood,
  the face, the realistic look's trim and shadows) and under its print, controls, screws and
  jacks. Nothing outside the panel is drawn. The strip below the panel, which holds the
  plug-in's own controls, has a drawing of its own (`strip_art`: see The strip).
- **Quiet ticks and titles** (`ticks_and_titles = "quiet"`): the panel's and the strip's dials'
  and selectors' ticks printed thinner (about the dials' numbers' strokes: 0.725 of their
  width, 0.8 in the large lettering), shorter (0.72 of their length, from their inner ends) and
  a shade dimmer than the lettering (80 % of its ink), and the panel's sections' titles (its
  layout's `[[title]]`s) at 0.75 of their size, spaced in the same proportion on the same
  baselines, as dim. Nothing else moves. It is what a player gets with the settings' TICKS &
  TITLES: QUIET, in any look; a look that sets it prints so whatever that says. Without it, the
  panel is printed as approved. A CA-72 older than this setting passes over it (`look.unknown`).
- **The typeface** prints the panel's legends, titles, dial numbers, tips and menus. It is used
  only if it has a glyph for every legend and for everything the plug-in prints itself
  (printable ASCII, "–" and "…"); otherwise the built-in typeface (URW Gothic) is used and the
  pack goes to review. Without a `bold`, the regular is used for both.
- **Legends that won't print or fit go to review.** A legend the typeface in use can't print
  (the built-in one has Latin, Greek and Cyrillic, but not Arabic or Chinese) would show as
  nothing: `look.glyphs`. A legend that runs off the panel, in either lettering size, is drawn
  cut off: `look.fit`. A control placed so that its own printed dial or legends would be cut off
  at the panel's edge: `layout.print-outside`. Each is the layout's to fix.

### Pictures

`base = "realistic"` draws its parts from pictures. A pack may replace any of them by slot,
with a PNG, or with an SVG drawn at the built-in picture's size. A picture is drawn at the
built-in one's proportions. Lay out your pictures as the built-in ones are laid out (each
described in `crates/ca72-panel/assets/worn/README.md`): the same framing, the part centred,
on a transparent ground. Round parts that turn (knobs, pointers, the bezels) should be lit
evenly all round, because the panel lays its own fixed lamp over them.

**Knob caps.** The built-in knobs have spun aluminium caps, whose sheen lies where the lamp is
however the knob is turned, so the middle of a knob's picture (its cap, the share the table
gives) is drawn still over the turned knob. A knob whose mark crosses its middle (a line from
its centre to its edge) needs the whole picture to turn: `knob_caps = "turn"` in `[look]`.
It applies to the knobs, the big knobs and the pointer knobs together. A CA-72 older than
this setting passes over it (`look.unknown`) and draws the caps still.

| Slot | What | Built-in size |
|---|---|---|
| `walnut` | The wood of the top strip and the rail, grain along it | 1400 × 788 |
| `face` | The face's surface, tiled 640 units across | 1024 × 1024 |
| `knob`, `knob-big` | A knob from above, its dot at twelve o'clock; its skirt 97.8 % (big: 98.9 %) of the picture, its cap 57.8 % (big: 65.3 %) | 512 × 512 |
| `pointer` | A selector's pointer knob, the pointer at twelve o'clock reaching the picture's edge; its body 78 %, its cap 43 % | 512 × 512 |
| `rocker-blue`, `rocker-orange` | A rocker in its opening, its right half raised (the ivory ones are made from the blue) | 512 × 194, 512 × 181 |
| `power` | POWER's rocker, its upper half raised | 112 × 320 |
| `jewel` | POWER's lamp in its bezel (OVERLOAD's darker one is made from it) | 256 × 252 |
| `jack`, `screw` | A jack's ring nut; a screw head | 256 × 251, 96 × 96 |
| `toggle-up`, `toggle-mid`, `toggle-down` | QUALITY's toggle, its nut in the same place in all three | 180 × 291 |
| `ultra-lens-off`, `ultra-lens-on`, `ultra-bezel-off`, `ultra-bezel-on`, `ultra-floor` | ULTRA's lamp: its lens, its bezel (the lens cut out), dark and lit; the well's floor | 300 × 300 |
| `tesla-lens-off`, `tesla-lens-on`, `tesla-bezel-off`, `tesla-bezel-on` | ULTRA's Tesla lamp, as the amber one | 300 × 300 |
| `hamster-wheel`, `hamster-asleep` | LO's hamster's wheel; the hamster asleep | 300 × 300, 300 × 289 |
| `hamster-run` | The hamster's run: eight frames side by side | 1840 × 120 |
| `plate` | The name plate, blank (its lettering is printed on it) | 1044 × 328 |

### The strip

The plug-in's strip under the panel (the presets' rail, the left hand, VOICES, STEREO and OUTPUT)
keeps its controls where they are, but a pack may dress it: its wood and face are the pack's
`walnut` and `face`, its knobs the pack's knobs, its legends the pack's typeface, and:

- **A drawing** (`strip_art`), stretched over the strip as the panel's drawing is over the
  panel: 3108 by 850 units (`viewBox="0 0 3108 850"`), y from the rail's top (the rail 0 to
  116, the sections below it). It is drawn over the strip's wood and face and under its print,
  knobs, tabs, keys and glass. `cargo run -p ca72-panel --example strip_places` prints where
  each of those is, to draw bays, recesses and windows round them. The rail has two sets of
  keys: the plug-ins' and the standalone's, which adds FX after ARP and shortens the preset's
  name a key; both are printed.
- **Pictures** in `[look.pictures]`:

| Slot | What | Built-in size |
|---|---|---|
| `tab` | A tab's cap (MONO, POLY, SCATTER...), square: the strip keeps its left and right halves and repeats its middle column to the tab's width. Without `tab-lit`, it is made translucent amber and lit from inside, as the built-in one is | 256 × 255 |
| `tab-lit` | The same cap lit, framed exactly as `tab` (an edit of it): a tab's two states, no light spilt round it | 256 × 255 |
| `key` | The rail's keys' cap, square, widened as a tab's | 256 × 255 |
| `glass` | The readouts' and the displays' glass, empty | 900 × 507 |
| `rocker-ivory` | The ivory rocker (the left hand's switches, and a layout's ivory ones), as `rocker-blue`; otherwise made from `rocker-blue` | 512 × 194 |
| `rocker-blue-on`, `rocker-orange-on`, `rocker-ivory-on` | A rocker ON, as the pack draws it (a lamp lit, say), framed exactly as the rocker's own picture. Given, that rocker shows its plain picture when off and this one when on, neither mirrored, and the plug-in neither lights nor shadows a rocking paddle on it: for switches that are flat keys with a lamp rather than paddles. Without it, the one picture is mirrored, as before | 512 × 194, 512 × 181 |

- **Colours** in `[look.colours]`: the lamps' (`lamp`), from which the plug-in makes their glow
  and the drops' light as it does its own orange's, and the tabs' and keys' print.
- **The left hand's VELOCITY** (the jack under GLIDE and DECAY, and the meter of ten lamps
  beside it): the jack is a jack as the left hand's others are (the pack's `jack`); its lamps,
  the cords dragged from it and the rings they make on the knobs are drawn in `velocity`, else
  in the pack's `lamp` (so that it lights as the screens do), else in the built-in orange
  (`#ff7028`); the lit lamps glowing in it, the unlit ones faintly in it over their dark wells.

**The keyboard view** (the keyboard shown in place of VOICES, STEREO and OUTPUT) is dressed too:
its header keys are the pack's `key`, its range display the pack's `glass` and `lamp`, its
lettering the pack's typeface, its case the pack's `walnut` (or, with `keyboard_case = "face"`,
its `face`, laid as the strip's face), and:

- `keyboard_art`: a drawing over its case, in the strip's units (3108 by 850 from the rail's
  top; the case from x 370 to 3080, y 116 to 844; the key bed 402 to 3050, 185 to 817; the
  range display 1069 to 1417, 124 to 174), drawn under its key bed, keys and print.
- Pictures `white-key` and `black-key`: one key each, seen from above, its back at the top
  (171 × 1385 and 181 × 1232). The white key whole: the plug-in cuts the black keys' notches.

**The presets' drawer** (FIND, PRESETS and PRESET, shown in place of the strip's sections) keeps
its displays and keys where they are, and is dressed too: its face is the pack's `face` (as the
panel's, tinted by `face_tint`), its displays' glass the pack's `glass`, its keys the pack's `key`,
its dots the pack's `lamp`, its print the pack's typeface in `tab_print`'s colour, and:

- `drawer_art`: a drawing over its face, in the drawer's own units (3108 by 734, y from the
  rail's foot: the strip's y less 116), drawn under its print, glass and keys. Its places: the
  search display from x 63 to 750, y 51 to 98; the tags' 63 to 750, 110 to 707; the list's 814
  to 2290, 51 to 707; PRESET's display 2354 to 3032, 244 to 707 (each with a 6-unit bezel the
  plug-in draws round it); PRESET's eight keys 136 by 51, centred at x 2422, 2603, 2784 and 2965,
  y 99 and 197; the sections' titles across y 20 to 42.
- `drawer_lettering = "type"`: its displays lettered in the pack's typeface rather than in dots
  (no unlit dots shown): what is lit (the current preset, a chosen tag or tab, a title) in the
  `lamp` colour, the rest in `display` (else `tab_print`, else a cream), fainter as the plug-in
  dims it; text typed into a field in `display`, its caret lit. The stars, arrows and the current
  row's mark are drawn as shapes, not letters. A press in a field puts the caret where the
  typeface sets the characters.
- `drawer_legends`: still read ("above" or "caps"), but the keys' legends are on their caps in
  every look now, in `key_print` and the look's typeface (the keys grew to hold them).
- `drawer_titles = "centre"`: FIND, PRESETS and PRESET centred over their sections and larger.

### Drawings: SVG's allowed subset

**Allowed:** `svg`, `g`, `defs`, `title`, `desc`, `metadata` (its contents are not read), `path`,
`rect`, `circle`, `ellipse`, `line`, `polyline`, `polygon`, `linearGradient`, `radialGradient`,
`stop`, `clipPath`, `mask`, `pattern`, `symbol`, `use`, `image`, `filter`, `feGaussianBlur`,
`feOffset`, `feFlood`, `feComposite`, `feMerge`, `feMergeNode`, `feColorMatrix`, `feBlend` and
`feDropShadow`, in SVG's namespace.

**Refused:**

- a DTD, and with it every entity, and processing instructions;
- scripts and foreign content;
- animation;
- text (the legends are the layout's, printed by the plug-in in the pack's typeface);
- style sheets (`<style>`, by that name in any namespace, anywhere), markers and interactive
  elements;
- anything of SVG's inside `<metadata>`, which may hold only other namespaces' data (a
  licence's RDF);
- event attributes (`on...`);
- any reference outside the drawing: an `href` other than `#name`, a `url(...)` other than
  `url(#name)`, or an `<image>` other than embedded PNG data (`data:image/png;base64,...`);
- a pattern filled with another pattern, or whose own elements are filled or stroked with it
  (by an element around the pattern, which they inherit from, also through `inherit`, whichever
  of their own declarations wins);
- an `id` given to two elements, or to anything that is not SVG's or is inside `<metadata>`.

**Limits:** 20,000 elements drawn, each reference (a `use`, a `href`, a `url(#name)`) counted
as everything it repeats (a reference that repeats itself is refused); 200,000 path segments;
32 filter primitives; a blur's deviation of at most 64; 4096 pattern tiles over what patterns
fill, all their uses together, and 20,000 elements drawn in those tiles; embedded pictures
within the picture limits.

**What drawing it costs.** Counts of parts don't bound the cost by themselves: a few long lines
crossing the whole panel cost far more than thousands of small shapes. So the check also
estimates the work, at the size the drawing is used (the panel's drawing 3000 pixels across;
a picture at its slot's size), in "pixel-equivalents":

- each pixel painted counts 1 (a fill's box; a stroke's length by its width);
- each pixel row a path's edges cross counts 100;
- each pixel of a layer (a group's opacity, a clip, a mask) counts 6;
- each pixel of a filter's region counts 10, for each of its primitives.

At most **60 million**: about 100 ms on an M4 Pro, half a second on a machine five times
slower. The counting stops as soon as a limit is passed.

**What drawing it holds.** The renderer allocates a buffer for each layer, pattern tile,
filter result, clip and mask, and a buffer it cannot have ends the plug-in's host at once (it
is not an error the plug-in can catch). So before anything is drawn, the check also counts
those buffers, at the size the drawing is used, each as large as the renderer makes it:

- a group's layer (its opacity, clip, mask or filter): its box, at most five times the
  drawing's size each way;
- a pattern's tile, at its transformed size, however little of it shows;
- a filter's whole region (it is not cut to the drawing) for each primitive, with its inputs,
  copies and scratch;
- a clip's or a mask's copies of the layer it cuts, and again for each clip or mask that cuts
  it;
- an embedded picture, decoded.

Each buffer is at most **16 million pixels**, and all of them together, counted as if none were
freed, at most **256 MB**. Coordinates, filter offsets and blurs too large for the renderer's
integer arithmetic are refused, and elements may be nested (references followed) at most 64
deep. When the plug-in draws a pack's drawing, larger than it was checked at (a large window on
a high-resolution screen) or stretched to a part's proportions, its buffers are counted again
there, against four times these limits: past them, it is not drawn.

A drawing is also drawn once as a test, on a thread of its own, against a 2 s limit, as a
backstop. While another drawing's test is still running past its deadline, the test is
skipped, with a note: the counted cost already bounds it. A test drawing that panics refuses
its pack (`svg.panicked`), and the packs after it are tested as before.

### Fonts

OpenType or TrueType, one face per file (no collections, no WOFF). The check reads what the
font's own tables say about it and sorts it into one of three outcomes. **It does not clear
it:** anyone who edits a font can change what its tables say.

- **Passes** when its declared licence (its `[[assets]]` entry's `license`) is one that lets a
  pack carry and sell it (OFL-1.1, Apache-2.0, CC0-1.0, MIT, Unlicense, Bitstream-Vera) and its
  own tables name the same licence. The OFL's or Apache's text must be in the pack.
- **Rejected** when its embedding permission is "restricted", or its licence text says
  "personal use", "non-commercial", "demo", "trial version", "evaluation", "may not be
  redistributed" or "not be embedded".
- **Review** for everything else: no declared licence; a licence not on that list; a
  difference between the declared licence and the font's own; a "preview and print"
  embedding permission; a changed font whose licence reserves its name.

## The layout, `layout.toml`

The official layout is the reference for every field: it is
`crates/ca72-panel/packs/official/layout.toml` (under the GPL), and the [starter's](skin-packs/starter/layout.toml)
is a CC0 one to begin from.

Units are the drawing's. The face's top left corner is at 0, 0, x runs right (0 to 3108) and y
runs down: the face from 0 to 795, the wooden strip above it from -130 to 0 (for the official
panel's size; a layout may choose another, below).

**The panel's size.** A layout may give its panel another height:

```toml
[panel]
face = 1000               # the face's height: 700 to 1200 (the official panel's is 795)
wood = 130                # optional: the wooden strip's, 60 to 200, or 0 for none (130)
```

Its width is always 3108. Its places are then in its own size's units: the face from 0 to
`face`, the wooden strip from `-wood` to 0, and its drawing 3108 by `wood + face`. The editor's
window keeps its width and changes its height to fit (the strip below the panel, with the
plug-in's own controls, is the same at every size); if a host will not resize the window, the
look shown before stays. A CA-72 older than `[panel]` passes it over and reads the layout at
the official size, where a taller layout's controls fall outside the panel and it is refused:
give such a pack `requires` (see Compatibility).

```toml
format = 1
controls = 2              # the revision of the CA-72's controls it was made for (see Compatibility)

[fixtures]
power = [3023, 580]       # POWER's switch (centre)
lamp = [3023, 426]        # its lamp
overload = [1764, 274]    # OVERLOAD's lamp
ultra = [3023, 92]        # QUALITY's opening (ULTRA's lamp, LO's hamster)
plate = [2578, -106]      # the name plate's top left corner (261 by 82)
phones = [2855, 581, 34]  # optional: the PHONES jack, centre and radius
screws = [[155.4, -22, 7]] # optional: the wooden strip's screws

[[control]]               # one for each of the panel's 44 controls, no more
param = "osc_mod"
at = [444, 269]           # its centre
size = [108, 50]          # a switch's or toggle's width and height, 16 to 240
colour = "red"            # a rocker's: blue, red, black, ivory
on = "right"              # the end a rocker is on at: right, top, bottom
# big = true              # a knob's larger size
# across = true           # a toggle on its side

[[line]]                  # a rule
from = [445, 9]
to = [445, 198]
width = 4                 # 0.5 to 20

[[title]]                 # a section title, spaced out to its extent
text = "MIXER"
span = [1442, 1591]
baseline = 724
# size = 52

[[text]]                  # a legend, centred on `at` (the middle of its capitals)
text = "VOLUME"
at = [1221, 59]
# size = 16               # fixed; without it, the panel's legend size (19, large 22)
# large_size = 18         # optional size in LARGE; without it, `size` still applies
# large = [1221, 55]      # its place in the large lettering, if different
```

**What a layout can't change.** Each control's parameter, kind (knob, selector, switch,
toggle), dial and legend values, and how it responds to the mouse are the plug-in's. A layout
may move a control, make a knob large, size a switch or toggle, colour a rocker and choose
its "on" end. Fields that don't apply to a control's kind are rejected.
The dials' numbers and the selectors' legends are printed by the plug-in, because they state
real values. The left hand's controls (PITCH, MODULATION, GLIDE, DECAY, NOTE PRIORITY,
RETRIGGER) and the strip's (VOICES, STEREO, OUTPUT, the presets' rail) are the plug-in's own
and stay where they are.

**Rejected:**

- a control missing, unknown, placed twice, or one of the strip's;
- anything reaching outside the panel;
- two controls (or a control and POWER) whose click areas overlap, because a click would
  find both;
- more than 300 rules, 300 legends, 16 titles or 32 screws;
- a legend of more than 48 characters;
- a layout `format` this plug-in does not read.

The controls' parameters:

> `tune`, `osc_mod`, `glide`, `mod_mix`, `osc1_range`, `osc2_range`, `osc3_range`,
> `osc2_frequency`, `osc3_frequency`, `osc1_waveform`, `osc2_waveform`, `osc3_waveform`,
> `osc3_control`, `osc1_volume`, `osc1_on`, `ext_volume`, `ext_on`, `osc2_volume`, `osc2_on`,
> `noise_volume`, `noise_on`, `osc3_volume`, `osc3_on`, `noise_type`, `filter_mode`,
> `filter_mod`, `keyboard_control_1`, `keyboard_control_2`, `cutoff`, `emphasis`,
> `contour_amount`, `filter_attack`, `filter_decay`, `filter_sustain`, `loudness_attack`,
> `loudness_decay`, `loudness_sustain`, `volume`, `main_output`, `a440`, `feedback`, `quality`,
> `osc2_timed`, `osc3_timed`.

`osc2_timed` and `osc3_timed` are oscillators 2's and 3's TIMED keys (revision 2 of the
controls): small amber keys, taking no field but `at`, dressed by the `tab` and `tab-lit`
pictures as the strip's tabs are. Each prints its oscillator's note value to its right (the
plug-in's print, centred on the key, up to 26 units above and below its centre), and while one is on at
its RANGE's LO its oscillator's FREQUENCY dial prints note values. A layout made for revision 1
gets them from the plug-in, at their official places under oscillators 2's and 3's RANGE when
nothing of the layout's is there.

Revision 4 of the controls (0.3.0) adds the LFO page's 28 (`lfo1_waveform`, `lfo1_rate`,
`lfo1_timed`, `lfo1_phase`, `lfo1_fade`, `lfo1_mode`, `lfo1_mono`, and the same for LFOs 2 to
4). They are the plug-in's own, on a lower page of its own, and a layout does not place them,
as it does not place the strip's: a layout naming one is refused (`layout.control`), and a
layout made for revisions 1 to 3 loads as it did. The page is drawn in the pack's base look
(WORN or CLASSIC), as the drawers are; the rings an LFO puts on a knob follow the knob to the
layout's place and size.

Revision 5 of the controls (0.3.0) adds the six macros' knobs (`macro1` to `macro6`). They are
the plug-in's own, in the strip's left area in the wheels' place while the rail's MACRO key shows
them, and a layout does not place them either: a layout naming one is refused, and a layout made
for revisions 1 to 4 loads as it did. Their area is drawn on the strip's bare face in the pack's
base look; a macro's rings on a knob follow the knob to the layout's place and size, as an LFO's.

Revision 6 of the controls (0.3.0) adds each LFO's UNI and ON keys (`lfo1_polarity`, `lfo1_on`,
and the same for LFOs 2 to 4), on the LFO page beside each column's title. A layout does not place
them, as it does not place the page's others: a layout naming one is refused, and a layout made
for revisions 1 to 5 loads as it did.

Revision 7 of the controls (0.3.0) adds each LFO's LEVEL (`lfo1_level` to `lfo4_level`), a
control of a kind of its own, `Kind::Jack`: its LFO's OUT jack on the LFO page, which has no knob
(the page draws the jack; LEVEL's rings go round it). A layout does not place it either: a layout
naming one is refused, and a layout made for revisions 1 to 6 loads as it did.

## Compatibility: packs keep working as the CA-72 changes

A pack made today must keep working on later CA-72s, and a pack made for a later one should do
its best on an earlier one.

- **Each layout says which revision of the CA-72's controls it was made for** (`controls = 7`
  today; a layout without it is made for the first). The plug-in keeps a record of its controls over
  time: when each arrived, and any rename, retirement or change of kind since.
- **A control added after the pack was made** is placed by the plug-in: at its official place if
  nothing of the pack's is there, else at the nearest free spot on the panel (`layout.added`, a
  note). If the panel has no room, the official layout is used for that pack, its pictures,
  drawing and typeface still its own (`layout.full`, a review).
- **A renamed control** is found by its old name (`layout.renamed`); **a retired one** is passed
  over (`layout.retired`); **one whose kind changed** is drawn in its new kind's own look, the
  pack's look settings for the old kind passed over (`layout.kind`). All are notes.
- **A control the pack should have placed but didn't** (one that existed when it was made) is
  still refused: that is a mistake, not age.
- **A layout or `[look]` field this CA-72 doesn't read** (a later one's) is passed over with a
  note (`layout.unknown`, `look.unknown`), not refused; `check_pack` shows it, so typos are seen.
  So is a control's name a later CA-72 has (`layout.newer`) in a layout made for a later
  revision.
- **The manifest stays strict**: its licence, provenance and declarations are what a review
  relies on, so an unknown field there is refused.
- **`requires`**: a pack that truly needs a later CA-72 says so (`requires = "ca-72 >= 0.4.0"`),
  and an earlier one refuses it with that reason, instead of drawing it wrong.
- **Packs as they were are kept as tests.** Samples of every released format and revision live in
  `crates/ca72-panel/tests/compat/`, never edited; every build must load each as it was made.

## Making a pack with an AI agent

Coding agents (Claude Code, Codex and others) can write a layout and SVG drawings well. Give
the agent this document and the starter, have it run `check_pack` after every change and
look at the preview, and keep what it reports. Record what the agent made in each asset's
provenance: `origin = "generated"` and the `tool`. Whether such work is protected, and so
can be licensed, differs between countries, so a marketplace's review will ask about it.

## Not in the first version

These are each flagged for separate review, and none is supported:

- scripts or custom widget code;
- animation;
- the strip's layout, the keyboard's and the presets' drawer's pictures and layout;
- colours of the drawn (classic) parts other than through a drawing;
- packs shared between plug-ins (bundles are the marketplace's: several packs sold together);
- telling the user that a pack has an update (the marketplace does, when the user opens it;
  the plug-in never asks the network).
