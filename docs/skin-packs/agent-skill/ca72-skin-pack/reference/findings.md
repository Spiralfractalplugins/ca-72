# What the check finds

*CC0 1.0. Checked against the CA-72 0.3.1's checks, November 2026. Codes are stable; messages
may change.*

Each line of the check's output is `<severity> [<code>] <file>: <why>`. Then `LOADS: ...` or
`REJECTED: ...`. The process exits 1 when the pack is rejected.

## Rejected: fix every one

The plug-in will not load the pack. A layout with any rejection is not read at all, so fix
these first and check again: later problems may only show once earlier ones are gone.

| Code | Usual cause | Fix |
|---|---|---|
| `manifest.missing`, `manifest.encoding` | No `skin.toml` at the pack's top, or not UTF-8 | Put the manifest at the top of the folder (or of the pack file's one folder) |
| `manifest.syntax`, `manifest.fields`, `look.fields`, `layout.syntax` | Bad TOML; a manifest field the format doesn't have (the manifest is strict); a known field with a value of the wrong type | Correct the TOML. (An unknown *layout* or `[look]` field is only noted: see the notes below) |
| `manifest.store` | `price`, `currency`, `sku`, `license_key` or `drm` | Remove it. Never add one |
| `manifest.id` | Id not lower-case dotted; `com.idlefoundry.*` without a first-party signature; a built-in look's id | Use the user's own `org.<name>.<pack>` |
| `signature.invalid` | A signed pack changed since signing (a file, its id or version), or a signature that doesn't match | Don't edit signed packs. Work on an unsigned copy; on the user's own copy, remove `signature.toml` with their agreement (it then loads as unsigned) |
| `signature.format` | A `signature.toml` that can't be read | Never write one. Remove it from the user's own copy, as above |
| `pack.file`, `archive.format`, `archive.entry`, `archive.method`, `archive.encrypted` | A pack file that isn't a plain file or a ZIP, an entry badly placed or named, compressed other than stored or deflated, or encrypted | Write it again with `pack_archive` from the folder; don't unpack a rejected one |
| `manifest.name`, `manifest.summary`, `manifest.version`, `manifest.plugin`, `manifest.format` | Name 1 to 64 characters, summary 1 to 280, `major.minor.patch`, `plugin = "ca-72"`, `format = 1` | Correct the value |
| `licence.expression` | Not an SPDX expression the checks know, nor `LicenseRef-<name>` | Use a listed identifier exactly (`CC-BY-4.0`, not `CC BY 4.0`) |
| `licence.text` | A `LicenseRef-` licence without its text, or a `license_files` entry not in the pack | Add the text file and name it |
| `credits.name` | A creator's name empty or over 120 characters | Correct it |
| `notice.file`, `notice.licence` | A `[[notices]]` entry whose file isn't a text file in the pack, or whose licence isn't an expression the checks know | Add the notice's file; correct its licence |
| `asset.missing`, `asset.duplicate`, `asset.licence`, `asset.hash` | An `[[assets]]` path that isn't there, given twice, a bad licence, or a `sha256` that no longer matches | Fix the entry; drop `sha256` while still editing |
| `look.missing`, `look.base`, `look.file`, `look.picture`, `look.art`, `look.face-tint` | No `[look]`; base not `classic`/`realistic`; a named file missing; an unknown picture slot or unreadable picture; art not SVG; a tint outside 0 to 2 | Correct `[look]` |
| `look.strip-art`, `look.keyboard-art`, `look.drawer-art` | The strip's, keyboard view's or drawer's drawing missing, not SVG, or refused as a drawing is | Correct the file, as for `svg.refused` |
| `look.colour`, `look.knob-caps`, `look.keyboard-case`, `look.drawer-lettering`, `look.drawer-legends`, `look.drawer-titles` | A `[look.colours]` value not `#rrggbb`; one of those `[look]` options not one of its words | Use the values `docs/skin-packs.md` lists |
| `look.pixels` | The slots' pictures together, each decoded at its slot's size, past the picture limit | Use fewer or smaller pictures; SVG pictures are decoded at their slot's size too |
| `layout.control` | A control missing, unknown, placed twice, one of the strip's, or a field its kind doesn't take | Every control the starter places, once, with its kind's fields ([layout.md](layout.md)) |
| `layout.controls`, `layout.panel` | `controls` below 1; a `[panel]` face outside 700 to 1200 or wood outside 60 to 200 (or 0) | Keep the starter's `controls`; correct `[panel]` |
| `layout.overlap` | Two click areas overlap, or one overlaps POWER | Move them apart: knobs need 124 between centres (big 146, selectors 120) plus room for their print |
| `layout.outside` | A control, fixture, plate, screw or PHONES reaches past x 0 to 3108, y -130 to 795 | Move it in, allowing for its click area or radius |
| `layout.line`, `layout.text`, `layout.title`, `layout.count`, `layout.number`, `layout.format` | A rule outside or of width beyond 0.5 to 20; a legend empty, over 48 characters or sized beyond 6 to 80; a title's span reversed or its top outside; too many; not a number; `format` not 1 | Correct the entry |
| `svg.refused` | `<text>`, `<style>`, a script, an outside reference, a DTD, animation, an element not on the list, an `id` used twice; or a count, the cost to draw (60 million pixel-equivalents) or the buffers it needs (16 million pixels each, 256 MB together) past its limit | Rewrite without it; for cost, run `svg_cost` and cut what it shows ([drawing.md](drawing.md)) |
| `svg.slow`, `svg.size` | The timed test drawing took over 2 s; no size | Run `svg_cost`; fewer tall lines, smaller filters and layers; give `viewBox` |
| `svg.panicked` | Drawing it as a test crashed the renderer | Simplify the drawing (filters, masks, nesting) until it passes, and tell the user: it is worth reporting to the CA-72 |
| `manifest.requires` | `requires` not `"ca-72 >= major.minor.patch"`, or naming a CA-72 later than this one | Remove it unless the pack truly needs a later CA-72 |
| `file.type`, `file.executable` | A file type not allowed, or code/an archive whatever its name | Remove it (and keep previews outside the pack) |
| `path.*`, `pack.*` (`pack.pixels`: over 16 million picture pixels, embedded ones included), `file.size`, `text.encoding`, `picture.png` | Names, depth, links, sizes, counts (in a folder or a pack file), non-UTF-8 text, a broken PNG | Rename, flatten, shrink or re-save |
| `font.file`, `font.embedding` (restricted), `font.licence` ("personal use", "demo"...) | A font that isn't a single OTF/TTF face, or whose maker forbids carrying it | Remove the font |
| `declaration.signed` | Declarations without `declared_by` and a `declared_on` date | Remove `[declarations]`: they are the user's to make |

## Review: explain each to the user

The pack loads and works for its owner. A marketplace's review must look at these before
listing it. Fix the ones that are mistakes; explain the rest in plain words.

| Code | What to tell the user |
|---|---|
| `declaration.missing` | Expected. No seller's declarations: they are needed only before a marketplace lists the pack, there is no seller agreement yet, and only you can make them. |
| `declaration.incomplete` | You declared one of the two statements but not both. |
| `provenance.generated` | Expected for each file I made. AI-generated work may not be protected by copyright in some countries, so whether it can be licensed (or sold under a licence) is uncertain; a review will ask. The record is honest, which is what matters. |
| `provenance.tool` | A generated file without its tool named. Mistake: name the tool. |
| `provenance.missing` | A file with no `[[assets]]` entry. Mistake: add one (or remove a stray file). |
| `provenance.source` | A derived or third-party file without its source or licence. Mistake: name both. |
| `credits.missing` | No creators credited. Ask the user how to be credited. |
| `licence.text` | An open licence without its text in the pack. Add `LICENSE.txt`. |
| `asset.official-copy` | A byte-for-byte copy of the plug-in's GPL picture in a pack under another licence. Remove it and draw your own. |
| `font.licence`, `font.embedding` (preview and print), `font.notice`, `font.reserved-name` | The font's licence is uncertain, disagrees with what was declared, its licence text is missing, or a changed font keeps a reserved name. Fix it or remove the font. |
| `look.fonts` | The pack's typeface can't print a legend, or something the plug-in prints (printable ASCII, "–", "…"), so the built-in typeface is used. Check it with `font_info`; use a fuller font or none. |
| `look.fit` | A legend runs off the panel in either lettering size, in the typeface used, and is drawn cut off. Mistake: move it in or shorten it. |
| `look.glyphs` | The built-in typeface (Latin, Greek, Cyrillic) can't print some legends: they'd show as empty boxes. Bring a typeface that has them, or change the legends. |
| `layout.print-outside` | A control's own printed dial or legends would be cut off at the panel's edge. Mistake: move it in. |
| `layout.full` | A control added in a later CA-72 found no free place, so the official layout is used for this pack (its drawing, pictures and typeface still its own). Leave room, or remake the layout for the newer revision. |

## Note: for the record, and read every one

`layout.unknown`, `look.unknown` (a field the plug-in doesn't read, passed over: **in a pack you
made, a typo to fix**; in an older plug-in reading a newer pack, expected), `layout.newer` (a
control a later CA-72 has, in a layout made for it), `layout.added` (a control added since the
layout's revision, placed by the plug-in), `layout.renamed`, `layout.retired`, `layout.kind` (a
control renamed, retired or changed in kind since, followed), `svg.untested` (the timed test was
skipped while another ran; the counted cost bounds it), `signature.none` (unsigned: not published by the marketplace, and it loads all the same; every
pack you make is unsigned), `signature.unknown-key` (signed with a key this plug-in doesn't
know, so treated as unsigned), `signature.valid` (who signed it and when: published, not
cleared), `licence.terms` (the pack's licence and that the plug-in's doesn't apply to it),
`declaration.declared` ("declared, not cleared"), `asset.official` (the plug-in's own picture
in a GPL pack), `look.picture` (a picture stretched to the built-in proportions),
`font.licence` (licence as declared and named), `font.vendor`, `font.trademark`. Mention
those the user would care about; never present a note as approval.
