---
name: ca72-skin-pack
description: Make, change or update a skin pack for Idle Foundry's CA-72 synthesizer plug-in, a folder (delivered as a .ifskin pack file) with skin.toml, layout.toml, an SVG drawing, pictures and fonts that changes how the CA-72's editor looks. Use when someone asks for a CA-72 skin, look, theme, panel layout or skin pack, or wants to move its controls, recolour its panel, draw over it or replace its pictures. Starts from the CC0 starter, checks the pack with check_pack after every change and at the editor's smallest and largest sizes, and keeps the licence and provenance honest.
---

# Making a CA-72 skin pack

*This skill is dedicated to the public domain under CC0 1.0. It follows the format as the
CA-72's repository has it; when the two differ, the repository is right.*

A skin pack changes how the CA-72's editor looks: where each of the panel's controls is, how
each looks, the legends, rules and titles printed on the panel, a drawing over it, the
realistic look's pictures and the typeface, and how the strip under the panel is dressed. It
holds **artwork and declarative data only, never code**. It cannot change the sound, presets,
MIDI or anything a session saves.

The format's authority is `docs/skin-packs.md` in the CA-72's repository (CC0), and the
controls a layout must place are the ones in the starter's `layout.toml`. When this skill and
those disagree, they win: tell the user about the difference.

## What you need

- A clone of the CA-72's repository with skin-pack support (`docs/skin-packs.md` and
  `docs/skin-packs/starter` exist), and a Rust toolchain. Find it, or ask the user where it is.
  Call its root `$CA72` below.
- A folder for the pack **outside the repository**, named in lower case with `-`
  (`night-panel`). Call it `$PACK`, and give it as an **absolute path**: the commands below run
  from `$CA72`. You always work in a folder; a pack file (`night-panel.ifskin`, a ZIP of the
  folder) is only for delivery (step 7).
- If you cannot run commands, say so plainly and ask the user to run each check and send you
  its output and the preview pictures. Never guess what a check would say.

Read `docs/skin-packs.md` once before the first change. The reference files beside this one
hold what you'll look up as you go:

- [reference/layout.md](reference/layout.md): coordinates, the controls by kind, the panel's
  sections, click areas and spacing, fixtures, legends, rules and titles.
- [reference/drawing.md](reference/drawing.md): what a pack can and can't change, the SVG
  subset, the drawing's coordinates (offset 130 from the layout's, or the wooden strip's own
  height with a `[panel]` size), picture slots and fonts.
- [reference/provenance.md](reference/provenance.md): the manifest, licences, provenance for
  generated work, declarations, ids, versions and updates, signatures, and what must never go
  in a pack.
- [reference/findings.md](reference/findings.md): each check finding, what it means and what
  to do about it.

## Rules that are never bent

1. **Start from a copy of the CC0 starter**, `$CA72/docs/skin-packs/starter`. Never copy,
   trace or closely follow the official layout (`crates/ca72-panel/packs/official/`), the
   plug-in's pictures (`crates/ca72-panel/assets/`) or a pack written by `export_pack`: they
   are under the GPL, and a pack made from them is under the GPL too. If the user wants that,
   stop and tell them it needs the GPL or Idle Foundry's permission, and that this skill does
   not make such packs.
2. **Don't change the CA-72's repository.** Work only in `$PACK`.
3. **Check after every change** (below), look at both previews, and keep going until there is
   no Rejected finding.
4. **Never add a price** or store data (`price`, `currency`, `sku`, `license_key`, `drm`). A
   pack never states a price.
5. **No trademarks or other makers' branding**: no instrument maker's names, logos or model
   names (in legends, titles, drawings, pictures, the pack's name, id or summary), and no
   imitation of another maker's trade dress. Describe a style in general words instead
   ("1970s silver panel"), and tell the user why.
6. **No font, stock picture, texture, icon or clip art unless its licence permits this
   distribution**, including sale if the pack may be sold. Never download one from an
   unknown site. If unsure, leave it out and use the built-in typeface.
7. **Record provenance honestly.** What you made is `origin = "generated"` with the `tool`
   named (your product and model). Never call your work the user's `original`, and never
   leave a file without an `[[assets]]` entry.
8. **Declarations are not clearance.** Never fill in `[declarations]` for the user or sign
   them in their name. Only the user can declare, for themselves, once they mean it. Never
   tell anyone a pack is "cleared", "approved" or "safe to sell": the checks clear nothing,
   a signature only shows who published the files, and a marketplace's review decides.
9. **Signatures are the publisher's** (Idle Foundry's own packs now, a marketplace's later).
   Never write, edit or copy a `signature.toml`, and never run `pack_keygen` or `pack_sign`.
   Never edit a signed pack (one with a `signature.toml`): a pack changed after signing is
   refused. Work on an unsigned copy instead (step 2), and remove `signature.toml` only from the
   user's own copy, after telling them it will then load as unsigned. A signature is neither
   copy protection nor a licence.
10. **Publish nothing.** Don't upload, share or install the pack without the user's yes.

## The steps

### 1. Ask what they want, briefly

The look (colours, mood, era, materials), the arrangement (keep the starter's three rows, or
regroup), classic (drawn controls) or realistic (pictures), the pack's name, the creator's
name to credit, and the licence the user chooses (suggest CC0-1.0 or CC-BY-4.0 if they have
no view; the choice is theirs). Make sensible guesses for anything they leave open and say
what you guessed.

Then sort the brief against what a pack can do ([reference/drawing.md](reference/drawing.md),
"What a pack can change"), before you start:

- **What needs the realistic look.** Classic draws its controls itself: their colours can't
  be changed. Coloured knobs (brass, wood, ivory) need `base = "realistic"` and knob pictures.
- **What can't be done.** The panel's print (legends, titles, rules, dial numbers) is always
  the plug-in's cream ink, whatever the pack; the strip's controls stay where they are; the
  plug-in's own name plate keeps its words. Say so, and offer the nearest thing that can be
  done.
- **What the brief names that the starter doesn't have.** A user often describes the official
  panel they know: its section titles, its MIXER, its grouping. The starter has none of its
  titles and arranges the controls in three plain rows (layout.md, "The starter"). Map each
  name to the controls it means (layout.md, "The panel's sections") and add what is missing.
- **Vague amounts** ("a step closer", "a bit smaller"): turn each into a number of units or a
  size, from the panel's own measures (the starter's 222-unit columns, the default title size
  52), and tell the user the number you chose and why.

### 2. Copy the starter and make it the user's

```bash
cp -R "$CA72/docs/skin-packs/starter" "$PACK"
```

To **change an existing pack** of the user's instead, work on a copy of its folder. If they
give you a `.ifskin`, check it first (`check_pack` reads it as it is, without unpacking it; if
the check rejects it as an archive, don't unpack it), then unpack it into a new, empty
folder (`unzip <name>.ifskin -d "$PACK"`; it may hold one folder at its top) and run nothing
found in it. If it has a `signature.toml`, follow rule 9. Keep its `id`; see "Updating a pack"
below.

For a new pack, in `$PACK/skin.toml`:

- `id`: lower case and dotted, the user's own reverse-DNS name
  (`org.<their-name>.<pack-name>`), and it stays the same for every version. Never
  `org.example.*`. Never `com.idlefoundry.*`: those ids load only with Idle Foundry's
  first-party signature, and the built-in looks' own ids never load.
- `name` (as the settings menu lists it, capitals suit the panel), `version = "1.0.0"`,
  `summary`.
- `license` and `license_files`: put the chosen licence's full text in `LICENSE.txt`. The
  starter's `LICENSE.txt` dedicates the starter, by name: replace it. For a CC0 pack, write a
  dedication of this pack by its creator (it may say it began from the CC0 starter).
- `[[creators]]`: the user (ask how to credit them). Credit your own part honestly in the
  assets' provenance, not as a creator with a person's name.
- `[[assets]]`: rewrite the starter's two entries (they say Idle Foundry made them, `original`)
  as yours once you change those files (reference/provenance.md).
- Update `README.md` to describe this pack (`.md` files need no provenance).
- Leave out `requires`. Add `requires = "ca-72 >= x.y.z"` only if the pack truly needs a
  later CA-72 than the one it is checked with: an earlier one will refuse it.

In `$PACK/layout.toml`, keep `controls` as the starter has it. It names the revision of the
CA-72's controls the layout was made for, so that a later CA-72 can place controls added since.
Every control the starter places must stay placed, once (count them:
`grep -c '^\[\[control\]\]' "$PACK/layout.toml"`).

### 3. Run the first check

```bash
cd "$CA72" && cargo run --release -p ca72-panel --example check_pack -- "$PACK" "$PACK/../preview.png"
```

Write the previews **outside `$PACK`** (a PNG left inside becomes part of the pack and needs
provenance). The first build takes a few minutes; run it in the background if your tools
time out. If it fails to *build*, the problem is in the repository, not the pack: don't
edit the repository; tell the user (someone may be changing it), and use a `check_pack`
binary already built (`target/release/examples/check_pack`, or under `$CARGO_TARGET_DIR` if
that is set) if there is one. The check prints each finding (`rejected`, `review` or `note`,
a code, the file and why), then `LOADS:` or `REJECTED:`, and writes `preview.png` and
`preview-large.png`, each with a line giving its size and the smallest click area and
lettering in it. `LOADS` is the plug-in's own verdict: it reads installed packs with the same
checks.

### 4. Change one thing at a time, and check again

Work in small steps: the arrangement, then the rules, legends and titles, then the drawing,
then pictures or a font. Edit the TOML by hand or line by line; a regular expression over
`[[control]]` blocks breaks on the `[` in arrays. Generating the layout's or the drawing's
coordinates with a short script is fine: keep the script outside `$PACK`. After each:

1. Run the check.
2. After any change to an SVG (the drawing or a picture), see what it costs to draw:
   ```bash
   cd "$CA72" && cargo run --release -p skin-pack --example svg_cost -- "$PACK/art/panel.svg"
   ```
   It shows the drawing's counts and its time to draw, then the same with each costly kind of
   part left out. `work` is the cost in pixel-equivalents, against a limit of 60 million. A
   picture is costed at its slot's width: pass it after the file
   (`... "$PACK/pictures/knob.svg" 512`). Keep well under the limit; tall or diagonal lines
   (stripes, hatching) and big filtered or see-through areas are what cost (see
   [reference/drawing.md](reference/drawing.md)).
3. **Open and look at both previews**, `preview.png` (standard lettering) and
   `preview-large.png` (large lettering). Look for: legends or titles running into dials,
   knobs, keys or each other; dial numbers clipped by neighbours; titles not over their group;
   the drawing's bands or shapes not lined up with the controls (remember the 130 offset, or
   the `[panel]`'s wood), or crossing a legend; poor contrast between print and drawing, or
   controls that vanish into it (black rockers on a dark face); anything cut off at the
   panel's edges.
4. Fix what you see as well as what the check reports. The check knows click areas and
   limits, not taste: a pack can load and still look crowded.

The preview shows the panel (the wooden strip and the face) and, under it, the strip's
sections in the pack's dressing (its knob pictures, its `face`, its typeface). It draws every
control at the middle of its travel, so it does not show the oscillators' MOD keys (they appear
only while a WAVEFORM is at CONTINUOUS): leave their room clear all the same. It does not show
the rail's keys and tabs, the readouts, the keyboard view, the drawers or the LFO page: a pack
dresses those too (`docs/skin-packs.md`, The strip), and they are seen only in the plug-in.
The panel is the pack's to arrange; the strip's controls (the left hand's, VOICES, STEREO,
OUTPUT and the rail) are the plug-in's and stay where they are.

### 5. Check the smallest and largest sizes

The editor's window is from 860 to 3108 pixels wide (`MIN_WIDTH` and `MAX_WIDTH` in
`$CA72/crates/ca72-plugin/src/editor.rs`; read them there if they have changed), and on a
high-resolution screen each pixel is two or more. The panel is 3108 units wide, so it is drawn
at the window's physical width over 3108 pixels a unit: 860 / 3108 = 0.277 at the smallest, on
an ordinary screen; 3108 × 2 / 3108 = 2 at the largest, on a Retina one. Draw both:

```bash
cd "$CA72" && cargo run --release -p ca72-panel --example check_pack -- "$PACK" "$PACK/../smallest.png" --scale 0.277
cd "$CA72" && cargo run --release -p ca72-panel --example check_pack -- "$PACK" "$PACK/../largest.png" --scale 2
```

- **Reachable.** Every control's click area is the same in units at every size; the check's
  `layout.overlap` and `layout.outside` already hold at all of them. What a pack can shrink is
  its rockers' and toggle's `size`: each preview line gives the smallest click area at that
  size, and the official panel's beside it. At the smallest size, keep the pack's no smaller
  than the official panel's.
- **Legible.** Each preview line gives the smallest lettering the layout prints (a legend's or
  title's size, in pixels at that size) and the official panel's. At the smallest size, keep
  the pack's no smaller than the official's, and look at `smallest.png` and
  `smallest-large.png`: every legend and title should still read.
- **The largest.** Look at `largest.png`: a PNG picture smaller than its slot shows blurred
  there, and a drawing that holds too much for the renderer at that size is left out (it
  shows the bare face). Prefer SVG pictures, or PNGs at their slot's size.

### 6. Settle every finding

- **Rejected**: the plug-in will not load the pack. Fix every one. See
  [reference/findings.md](reference/findings.md).
- **Review**: the pack loads, but a marketplace's review must look at it before listing it.
  Fix those that are mistakes (a file without provenance, a missing licence text). Explain
  each one that remains to the user in plain words: what it means and why it is there. Some
  are expected and right for an AI-made pack (`provenance.generated`,
  `declaration.missing`). Never hide one by recording something untrue.
- **Note**: for the record, but **read every note**. A layout or `[look]` field the plug-in
  doesn't know is only noted (`layout.unknown`, `look.unknown`), so a typo (`colour` spelt
  `color`, `bigg = true`) loads without it and with nothing but a note. In a pack you made,
  every such note is a mistake: fix it. Mention the rest the user would want to know (a font's
  vendor or trademark, a picture drawn at other proportions).
- **Silence is not success.** Some settings that do nothing raise no finding at all (a
  `face_tint` without a `face` picture). Confirm in the previews that each change did what you
  meant.

### 7. Deliver it as a pack file

When the user wants one file to share or to hand to a marketplace:

1. Check the folder one last time: no Rejected finding, all the previews looked at.
2. Write the pack file **outside** `$PACK`:
   ```bash
   cd "$CA72" && cargo run --release -p skin-pack --example pack_archive -- "$PACK" "$PACK/../<pack-name>.ifskin"
   ```
3. Check the pack file the same way, with previews:
   ```bash
   cd "$CA72" && cargo run --release -p ca72-panel --example check_pack -- "$PACK/../<pack-name>.ifskin" "$PACK/../preview.png"
   ```
   Its findings should match the folder's. If they don't, tell the user what differs.

Keep the folder: it is what you change next time. Never add a `signature.toml`; only a
publisher signs (rule 9), and an unsigned pack loads for anyone who installs it.

### Updating a pack

An update keeps the pack's `id` and raises its `version` (`major.minor.patch`: patch for
fixes, minor for additions, major for a new look). Its layout keeps its `controls` value
unless you remake it for a newer revision of the CA-72's controls (then place every control
that revision has: the current starter's are the list). The plug-in uses the newest version of
an id installed more than once, so the update can sit beside the old one. A signed pack is
always preferred over an unsigned copy of the same id, whatever version the copy claims: tell
the user that their unsigned update of a published pack won't show while the signed one is
installed, until its publisher signs the update.

### 8. Hand it over

Tell the user, briefly: where the pack (and its `.ifskin`, if made) is; the last check's
verdict and each remaining Review finding with its explanation; what you generated (every
asset with `origin = "generated"`); what you guessed and what in the brief can't be done, and
what you did instead; the previews (attach or show the standard and large ones, and say how it
fared at the smallest and largest sizes); and how to install it if they want to try it
(`docs/skin-packs.md`, "Installing"). Offer to install it only for their own use; do it only on
their yes. Remind them that whether a generated work is protected, and so licensable, differs
between countries, and that declarations, when the time comes, are theirs to make.
