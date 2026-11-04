# The layout, `layout.toml`

*CC0 1.0. Checked against the CA-72's `skin-packs` prototype, October 2026; `docs/skin-packs.md`
is the authority.*

## The header

```toml
format = 1
controls = 3    # the revision of the CA-72's controls this layout was made for
```

Keep `controls` as the starter has it. A later CA-72 uses it to place controls added since
(`layout.added`, a note; `layout.full`, a review, if the panel has no room, when the official
layout is used for the pack) and to follow renamed, retired or changed ones (`layout.renamed`,
`layout.retired`, `layout.kind`, notes). A control that existed in the layout's revision and
isn't placed is still refused.

A field the plug-in doesn't know is passed over with a note (`layout.unknown`), not refused.
That keeps old CA-72s reading newer layouts, but it means a misspelt field is silently
ignored: read every note.

## Coordinates

Units, not pixels. The face's top left corner is `0, 0`; x runs right to **3108**, y runs
down to **795**. The wooden strip above the face runs from **y = -130** to 0. Everything in
the layout (controls with their click areas, fixtures, rules, legends, titles) must stay
inside x 0 to 3108, y -130 to 795.

**A taller or shorter panel.** A `[panel]` table gives the face another height (`face`, 700 to
1200) and the wooden strip another (`wood`, 60 to 200, or 0 for none; 130 if absent); the width
stays 3108. Then everything stays inside x 0 to 3108, y `-wood` to `face`. Use it when the
design needs room, not by default: a taller panel makes the editor's window taller at the same
width. Four rows of knobs with legends need a face of about 1,000. Give the pack
`requires = "ca-72 >= <the first version with [panel]>"` so that an older CA-72 refuses it
with a reason instead of rejecting its layout.

The drawing (`art/panel.svg`) uses the same x but **y + wood** (130): see
[drawing.md](drawing.md).

## The 45 controls, by kind

Every one must appear exactly once as a `[[control]]` with its `param` and `at` (its centre).
Others are rejected (the left-hand column and the lower strip are the plug-in's own).

| Kind | Params | Fields allowed besides `param`, `at` |
|---|---|---|
| Knob (21) | `tune`, `glide`, `mod_mix`, `osc2_frequency`, `osc3_frequency`, `osc1_volume`, `osc2_volume`, `osc3_volume`, `ext_volume`, `noise_volume`, `cutoff`, `emphasis`, `contour_amount`, `filter_attack`, `filter_decay`, `filter_sustain`, `loudness_attack`, `loudness_decay`, `loudness_sustain`, `volume`, `feedback` | `big = true/false` (`osc2_frequency`, `osc3_frequency` are big unless set false) |
| Selector (7) | `osc1_range`, `osc2_range`, `osc3_range`, `osc1_waveform`, `osc2_waveform`, `osc3_waveform`; `filter_poles` (FILTER POLES, added in revision 8: four places, 1 to 4, on the middle four of the six) | none |
| Rocker (13) | `osc_mod`, `osc3_control`, `osc1_on`, `osc2_on`, `osc3_on`, `ext_on`, `noise_on`, `noise_type`, `filter_mod`, `keyboard_control_1`, `keyboard_control_2`, `main_output`, `a440` | `size = [w, h]` (16 to 240 each), `colour` (`blue`, `red`, `black`, `ivory`), `on` (`right`, `top`, `bottom`) |
| Toggle (2) | `quality`; `filter_mode` (FILTER MODE: LO, BAND, STEEP HIGH; a toggle since revision 8, a rocker before) | `size = [w, h]` (16 to 240; default 50 by 81), `across = true` |
| Key (5) | `osc2_timed`, `osc3_timed` (oscillators 2's and 3's TIMED, added in revision 2); `osc1_shape_mod`, `osc2_shape_mod`, `osc3_shape_mod` (each oscillator's MOD, added in revision 3: on the panel only while its waveform is CONTINUOUS; place each under its WAVEFORM) | none |

A field that doesn't apply to a control's kind is rejected (`layout.control`). A rocker that
is on at `top` or `bottom` reads best taller than wide (`size = [55, 111]`); one on at `right`
wider than tall (`size = [108, 50]`).

The plug-in prints each knob's dial numbers and each selector's legends itself, round the
control (oscillators 2's and 3's FREQUENCY dials print note values while they are timed, and a
TIMED key prints its note value to its right, centred on the key, reaching up to 26 units above and below its
centre): they are not in the layout and can't be changed. Leave room for them, including at
the panel's edges: a control whose own dial or legends would be cut off by the edge goes to
review (`layout.print-outside`), though its click area fits.

## Click areas and spacing

No two click areas may overlap, nor any with POWER's (`layout.overlap`), and none may reach
outside the panel (`layout.outside`). Click areas, from the centre:

| Kind | Click area | Room it needs to look right (with dial print) |
|---|---|---|
| Knob | a square of ±62 | about 200 across, 170 down |
| Big knob | ±73 | about 220 across, 190 down |
| Selector | ±60 | about 200 across, 170 down |
| Rocker, toggle | ±(w/2 + 4), ±(h/2 + 4) | its size plus a legend |
| Key | 36 left, 69 right (its note value), ±18 down | 120 across, 26 above and below its centre |
| POWER | ±26 across, ±64 down | |

The starter spaces its columns 222 apart and its rows 240 apart, with legends 100 below each
centre: copy that rhythm when regrouping, and check the large-lettering preview, where
legends are wider.

## Fixtures

```toml
[fixtures]
power = [x, y]          # POWER's switch, centre; needs 64 clear all round
lamp = [x, y]           # its lamp, radius 24
overload = [x, y]       # OVERLOAD's lamp, radius 34
ultra = [x, y]          # QUALITY's opening (ULTRA's lamp, LO's hamster), radius 54
plate = [x, y]          # the name plate's top left corner; it is 261 by 82
phones = [x, y, r]      # optional; r up to 60
screws = [[x, y, r]]    # optional; at most 32, r up to 60
```

The plate prints "CA-72" and "IDLE FOUNDRY" itself: that is the plug-in's own name, and a
pack may move the plate but cannot change or remove what it says. Don't add other makers'
names anywhere.

## Print

```toml
[[line]]                  # a rule; at most 300
from = [x, y]
to = [x, y]
width = 2                 # 0.5 to 20

[[text]]                  # a legend, centred on `at` (the middle of its capitals); at most 300
text = "CUTOFF"           # 1 to 48 characters
at = [x, y]
# size = 16               # 6 to 80; without it the panel's legend size (19, large 22)
# large = [x, y]          # its place in the large lettering, if it must move there

[[title]]                 # a section title, spaced out across its span; at most 16
text = "FILTER"
span = [x0, x1]           # x0 < x1
baseline = y              # its top (baseline - 60) must be inside the panel too
# size = 52               # 6 to 80
```

Legends are printed in capitals by convention; keep them short. One that runs off the panel
in either lettering size, in the typeface used, is drawn cut off (`look.fit`, a review): move
it in or shorten it. The built-in typeface prints Latin, Greek and Cyrillic, not Arabic or
Chinese (`look.glyphs`, a review): for other scripts bring a typeface that has them. Text goes only in the layout:
an SVG drawing may not contain text.
