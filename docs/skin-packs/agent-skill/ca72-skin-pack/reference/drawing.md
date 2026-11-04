# What a pack can change, the drawing, pictures and fonts

*CC0 1.0. Checked against the CA-72 0.3.1's format, November 2026; `docs/skin-packs.md` is the
authority.*

## What a pack can change

Sort a brief by this table before you start (SKILL.md, step 1), and tell the user what can't be
done.

| The user asks for | How, or why not |
|---|---|
| The panel's colour, bands, frames, a texture | The drawing (`art`), over the face; with `base = "realistic"`, also a `face` picture, which the strip, the keyboard view and the drawer are dressed in too |
| The strip's colour | A `face` picture (realistic), `strip_art`, `[look.colours]`; the strip's controls can't move |
| Controls moved, regrouped, a knob large, a rocker sized, coloured (`blue`, `red`, `black`, `ivory`) or turned | The layout ([layout.md](layout.md)), the panel's controls only |
| Knobs, selectors, rockers, jacks, screws in another colour or material (brass, wood) | `base = "realistic"` and pictures for their slots (below). In the classic look the plug-in draws them: their colours can't be changed, and the drawing is drawn under them |
| The lettering's colour | Not possible: legends, titles, rules and dial numbers are printed in the plug-in's cream ink (`#ebe6d8`) in both looks. Give them a dark enough face. The strip's tabs, keys and displays have colours in `[look.colours]` |
| Lettering smaller or larger, moved, renamed | The layout's `[[text]]` and `[[title]]` (`size`, `large_size`, `at`, `span`); the dial numbers and the selectors' legends are the plug-in's and can't be changed |
| Another typeface | `fonts`, under its licence's conditions (below) |
| A taller or shorter panel | The layout's `[panel]` ([layout.md](layout.md)); the width is always 3108 |
| The name plate's words, another maker's name or logo | Not possible: the plate says "CA-72" and "IDLE FOUNDRY"; a pack may move it. No other maker's branding (SKILL.md, rule 5) |
| Animation, moving parts, sounds, behaviour | Not possible: a pack is artwork and data only |

## The strip's drawing, `strip_art`

The plug-in's strip under the panel (the presets' rail and the VOICES, STEREO and OUTPUT
sections) has its own drawing: `viewBox="0 0 3108 850"`, y from the rail's top (the rail 0 to
116). Run `cargo run -p ca72-panel --example strip_places` for every tab, readout window,
display and knob, and draw bays and windows round them; the strip's controls stay where they
are. Its tabs, keys, glass and lamp colour are the pack's too: see `docs/skin-packs.md`, The
strip. Make `tab` and `tab-lit` square caps framed alike (the lit one an edit of the unlit):
the strip widens them by repeating their middle column, so keep anything particular (a lamp
window) in one half.

## The keyboard view's and the presets' drawer's drawings

`keyboard_art` covers the keyboard view's case, in the strip's units. `drawer_art` covers the
presets' drawer: `viewBox="0 0 3108 734"`, y from the rail's foot (the strip's y less 116). Its
displays and keys stay where they are, and their places are listed in `docs/skin-packs.md`, The
strip. The displays reach within 27 units of the drawer's foot, so keep frames round them slim.
`drawer_lettering = "type"` letters the drawer's displays in the pack's typeface, in `lamp` and
`display`, rather than in dots.

## The drawing, `art/panel.svg`

Give it `viewBox="0 0 3108 925"`. It covers the wooden strip (y 0 to 130) and the face (y 130
to 925), and is stretched over them. **Its y is the layout's y + 130**: a control at layout
`[777, 360]` is at `777, 490` in the drawing. Its x is the layout's x.

With a layout's own `[panel]` size, the viewBox is `0 0 3108 <wood + face>` and its y is the
layout's y + `wood`: the wooden strip from 0 to `wood`, the face from `wood` to `wood + face`.

It is drawn over the panel's surfaces (the wood, the face, the realistic look's trim and
shadows) and under the print, controls, screws and jacks. So:

- A face colour is a `rect` from y 130 to 925. Leave y 0 to 130 alone (or keep it
  translucent) to keep the wood showing; fill it to replace the wood.
- Bands, panels and frames behind groups of controls are the usual way to give a look its
  character. Line them up with the layout's rows (add 130).
- Keep enough contrast for the print. The plug-in prints legends, titles, rules and dial
  numbers in its cream ink (`#ebe6d8`) in both looks, whatever the pack: a light drawing makes
  them hard to read. Check both previews.

### Allowed

`svg`, `g`, `defs`, `title`, `desc`, `metadata`, `path`, `rect`, `circle`, `ellipse`, `line`,
`polyline`, `polygon`, `linearGradient`, `radialGradient`, `stop`, `clipPath`, `mask`,
`pattern`, `symbol`, `use`, `image`, `filter`, `feGaussianBlur`, `feOffset`, `feFlood`,
`feComposite`, `feMerge`, `feMergeNode`, `feColorMatrix`, `feBlend`, `feDropShadow`, in SVG's
namespace. Style with presentation attributes (`fill`, `stroke`, `opacity`, ...).

### Refused (`svg.refused`)

A DTD or entities, processing instructions, `<script>`, `<foreignObject>`, animation,
**`<text>` and `<tspan>`**, `<style>` (by that name in any namespace, anywhere), markers,
interactive elements, `on...` attributes, anything of SVG's inside `<metadata>` (which may
hold only other namespaces' data, such as a licence's RDF), a pattern filled with another
pattern, an `id` given to two elements (or to anything not SVG's, or inside `<metadata>`),
and any reference outside the file: only `href="#name"`, `url(#name)` and `<image>`
with `data:image/png;base64,...`. Drawing programs often add `<style>`, editor namespaces and
fonts: write the SVG yourself, or strip those out.

### Limits

20,000 elements drawn (each reference, a `use`, `href` or `url(#name)`, counts as all it
repeats; one that repeats itself is refused), 200,000 path segments, 32 filter primitives, a
blur's deviation of at most 64, 4096 pattern tiles over all that patterns fill (and 20,000
elements drawn in those tiles), and embedded pictures within the picture limits. Before it is
drawn, the buffers it needs (each group's layer for its opacity, clip, mask or filter, each
pattern tile, each filter's region, each embedded picture) are counted at the size it is used:
at most 16 million pixels each and 256 MB together (`svg.refused`). The plug-in counts them
again when it draws the drawing larger (a large window on a high-resolution screen), against
four times those limits, and past them leaves the drawing out: check the largest size
(SKILL.md, step 5).

### What it costs to draw

Counts don't bound the cost: a few long lines across the panel cost more than thousands of
small shapes. The check estimates the work at the size the drawing is used (the panel's
drawing 3000 pixels across, a picture at its slot's size) in pixel-equivalents, and refuses
past **60 million**:

| Work | Counts |
|---|---|
| a pixel painted (a fill's box; a stroke's length by its width) | 1 |
| a pixel row a path's edges cross | 100 |
| a pixel of a layer (a group's `opacity`, a clip, a mask) | 6 |
| a pixel of a filter's region | 10 for each primitive |

Edge rows are counted for every edge, by how far it climbs: the sum, over all edges, of the
pixel rows each spans (a stroke's twice over). So tall or diagonal lines are what cost:
hundreds of stripes or hatching running down or across the panel add up fast, while
horizontal edges cost almost nothing and short shapes little. Keep tall and diagonal edges
few; put `opacity` on shapes rather than on big groups; keep filters to small regions;
prefer gradients to blurs and to hatching. Run `svg_cost` (SKILL.md, step 4) on every drawing you
make: it prints the cost part by part, with each costly kind left out in turn.

A timed test drawing (2 s, `svg.slow`) is a backstop. If it is skipped because another
drawing's test is running, `svg.untested` notes it; the counted cost still bounds the drawing.

Write the SVG by hand or with a script, and keep it readable: name gradients and groups, and
put a `<title>` saying what it is.

## Pictures (`base = "realistic"` only)

```toml
[look]
base = "realistic"

[look.pictures]
knob = "pictures/knob.svg"
face = "pictures/face.png"
```

Each slot takes a PNG or an SVG; an SVG is drawn at the built-in picture's size, so give it
that `viewBox`. Coloured knobs need three slots: `knob`, `knob-big`
(oscillators 2's and 3's FREQUENCY and any `big` knob) and `pointer` (the RANGE and WAVEFORM
selectors); a slot left out keeps the built-in picture. The strip's knobs are the pack's knobs
too. A picture is drawn at the built-in one's proportions (another shape gets a
`look.picture` note and is stretched). Frame the part as the built-in one is framed, centred
on a transparent ground; round parts that turn should be lit evenly all round, because the
panel lays its own lamp over them. The slots and their sizes are in `docs/skin-packs.md`,
"Pictures"; the framing of each is described in
`crates/ca72-panel/assets/worn/README.md`. Read that description, but **don't open, copy,
trace or edit the built-in pictures themselves**: they are under the GPL, and a byte-for-byte
copy is flagged (`asset.official-copy`).

A coding agent draws slot pictures best as SVG (a knob from above: concentric circles,
radial gradients, a mark at twelve o'clock). **Put a knob's mark on its skirt, outside its
cap**: the cap (the middle 57.8 % of `knob`, 65.3 % of `knob-big`, 43 % of `pointer`) is drawn
still over the turning knob, so a mark inside it never turns. A mark that must cross the middle
needs `knob_caps = "turn"` in `[look]`. Light it evenly all round (radial gradients centred on
the knob): the plug-in lays its own lamp over it.

**The face.** `face` is tiled 640 units across, on the panel and the strip alike: a flat colour,
or a texture that runs on across its edges (lines straight across it), shows no seams.
`face_tint` scales a `face` picture's channels, and only the pack's own: without a `face`
picture it does nothing, and no finding says so. A picture made with an image generator is
`origin = "generated"` with that generator named as the `tool`.

Picture limits: 4096 pixels a side, **16 million pixels in all**, pictures embedded in SVGs
counted too (`pack.pixels`), 8 MiB a file. A PNG or SVG picture at other proportions than its
slot is stretched and noted (`look.picture`).

## Fonts

```toml
[look]
fonts = { regular = "fonts/Face-Regular.ttf", bold = "fonts/Face-Bold.ttf" }
```

Prefer none: the built-in typeface (URW Gothic) suits the panel. Use a font only when the
user supplies it or names one whose licence lets a pack carry and sell it (OFL-1.1,
Apache-2.0, CC0-1.0, MIT, Unlicense, Bitstream-Vera), from its maker's own source. Then:

- give it an `[[assets]]` entry with `origin = "third-party"`, `source` (where it came from)
  and `license` (the font's licence, e.g. `"OFL-1.1"`);
- put its licence text in the pack (`OFL.txt`), and a `[[notices]]` entry pointing at it;
- don't rename or change a font whose licence reserves its name;
- one face per file, `.otf` or `.ttf`, at most 4 fonts, 4 MiB each.

The font must have a glyph for every legend and title, "CA-72" (the plate's) and everything
the plug-in prints itself: printable ASCII, "–" and "…". Otherwise the built-in typeface is
used and a `look.fonts` review finding says why. A legend that runs off the panel in the font
is drawn cut off (`look.fit`, a review): the font is still used, and the layout is what to
fix. See what a font says of itself and what it lacks before using it:

```bash
cd "$CA72" && cargo run -p skin-pack --example font_info -- fonts/Face-Regular.ttf "EVERY LEGEND"
```

It prints its names (family, copyright, licence and link, trademark), its maker, its
embedding permission, the printable ASCII it lacks and whether it has each character of the
text given.
