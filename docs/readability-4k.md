# CA-72 0.3.0 at 4K and 100 % scaling

Survey for [issue #70](https://github.com/idlefoundry/ca-72/issues/70), 2026-10-19. Base:
`17f1b5f`, release-0.3.0. Both built-in looks, STANDARD and LARGE, one physical pixel a
logical pixel. These are offscreen renders of the editor's components, not photographs of a
27 or 32 inch display or a test in a host.

The fresh editor on a 3840 by 2160 usable screen is already almost the largest window:
**3026 by 1728**, not the SIZE menu's **100 %**, which is 1380 by 788. At the fresh size most
legends, the keyboard's controls and the presets' drawer clear the threshold below. At the
100 % menu size many do not. Tooltips and menu headings stay small at every window size.

## Method and threshold

Flag an x-height of **9 physical pixels or less**. This is a conservative screen for small
print, not a guarantee of comfort above 9. [Legge and Bigelow (2011)](https://pmc.ncbi.nlm.nih.gov/articles/PMC3428264/)
report a fluent-reading lower limit near 0.2 degrees of x-height. Calculated for a 16:9 4K
display, 9 pixels are 1.40 mm at 27 inches or 1.66 mm at 32 inches. At a 60 cm viewing
distance these subtend only 0.134 to 0.159 degrees; 0.2 degrees would take about 13.5 or
11.4 pixels respectively. The 9 px cutoff is therefore a useful lower warning line.

Font size is the em, not the visible height. The bundled URW Gothic outlines have:

| Typeface | Capital H / em | Lowercase x / em |
|---|---:|---:|
| Book | 0.739 | 0.547 |
| Demi | 0.739 | 0.554 |

For type, multiply the drawing's font size by the outline ratio and `window_width / 3108`.
For tooltips and ordinary menus the size is in logical pixels and does **not** grow with the
window: `Renderer::text_size` divides out the drawing scale. LARGE multiplies it by 1.25.
The settings' menu is reduced further if it cannot fit the viewport.

The hand-drawn dot font has five rows a lowercase x and seven a capital. Measure the centres'
span plus the lit dot's diameter, excluding glow: presets' drawer x/cap = 4.9/6.9 times
pitch; rail and FX x/cap = 4.72/6.72 times pitch. Seven-segment readouts have a 53.76-unit
capital envelope (`84 * 0.64`); they are above the threshold at every offered size.

The appendix gives **both x-height and cap height**. For capitals and numerals, x-height is a
reference for the same size of type, not an assertion that a lowercase x appears there.
A numeral with an 8 px x-reference can have an 11 px capital height. Such rows are cautionary,
less severe than mixed-case text whose actual lowercase letters are below 9 px. Stroke, glow,
contrast, grain, sight and viewing distance also matter; these measurements do not establish
that every flagged item is unreadable.

## Window sizes

`editor.rs`: `SIZES`, `fitted_width`, `height_for`, `MIN_WIDTH`, `MAX_WIDTH`. FIT TO SCREEN
uses 80 % of the usable screen, limited by height as well as width; saved sessions can retain
another width. Window decorations, docks and host chrome reduce the usable area. All five
menu percentages fit a full 3840 by 2160 screen. Minimum and maximum grip widths are included.

| Choice | Physical viewport | Drawing scale |
|---|---:|---:|
| Minimum grip | 860 × 491 | 0.27671 |
| 75 % | 1035 × 591 | 0.33301 |
| 100 % / screen size unknown fallback | 1380 × 788 | 0.44402 |
| 125 % | 1725 × 985 | 0.55502 |
| 150 % | 2070 × 1182 | 0.66602 |
| 200 % | 2760 × 1576 | 0.88803 |
| Fresh / FIT TO SCREEN, full 4K usable area | 3026 × 1728 | 0.97362 |
| Maximum grip | 3108 × 1775 | 1.00000 |

The renderer allocates the ceiling of the drawing's extent; each evidence image is clipped
by at most one row or column to the editor's rounded viewport. No evidence image is rescaled.
The 4K canvases centre the window at one source pixel a screen pixel. The annotated crop
sheets additionally show explicitly labelled nearest-neighbour enlargements.

## What remains small

Values here are x-height, STANDARD / LARGE, in physical pixels, before the narrow fix below.
The complete per-instance table includes capitals, positions and the 1380 px case.

| Element | At 1380 px wide | At fresh 3026 px wide | Interpretation |
|---|---:|---:|---|
| Panel legends, 19 / 22 | 4.67 / 5.41 | 10.25 / 11.87 | Fresh 4K clears the line; smaller saved windows do not |
| Dial numbers, 15 / 17 | 3.69 / 4.18 | 8.09 / 9.17 | LARGE helps at FIT TO SCREEN; actual capitals are taller |
| M-SEC., SEC., OSC. 3, NOISE dial captions, 14 / 16 | 3.44 / 3.94 | 7.55 / 8.63 | Still small in LARGE |
| OSC. 3 CONTROL, fixed 16 | 3.94 / 3.94 | 8.63 / 8.63 | Explicitly left fixed by R-LETTERING |
| QUALITY's ULTRA / HI / LO, fixed 15 | 3.69 / 3.69 | 8.09 / 8.09 | Omission corrected on this branch |
| IDLE FOUNDRY, fixed 12 | 2.95 / 2.95 | 6.47 / 6.47 | Decorative maker name; cap height only 8.63 px even fresh |
| Strip legends, 21 / 23 | 5.17 / 5.66 | 11.33 / 12.41 | LARGE works |
| Strip small print, 16 / 18 | 3.94 / 4.43 | 8.63 / 9.71 | LARGE works |
| Strip / arp / FX tab caps, fixed 17 | 4.18 / 4.18 | 9.17 / 9.17 | Explicit fixed-size family, still small in narrow windows |
| SAVE / PANEL / ARP / FX rail caps, fixed 20 | 4.92 / 4.92 | 10.79 / 10.79 | Unchanged by LARGE |
| Keyboard mapping, fixed 18 Book | 4.37 / 4.37 | 9.59 / 9.59 | Borderline at fresh size; SMALL at 100 % menu size |
| Keyboard TYPING / PANIC / − / +, fixed 20 | 4.92 / 4.92 | 10.79 / 10.79 | LARGE leaves the keyboard unchanged |
| Preset drawer list, tags, status and MIDI text, pitch 4 | 8.70 / 8.70 | 19.08 / 19.08 | Below the line at 100 %; easily clears it fresh |
| Preset drawer search, pitch 4.5 | 9.79 / 9.79 | 21.47 / 21.47 | Search already larger than the list |
| Preset drawer headings, 26 | 6.40 / 6.40 | 14.02 / 14.02 | Cap heights 8.53 / 18.71 px respectively |
| Preset action keys, 28 (MIDI LEARN fitted) | about 6.7–6.9 / same | about 14.6–15.1 / same | Actions already enlarged on this base; no further change here |
| FX list names and makers, pitch 4 | 8.38 / 8.38 | 18.38 / 18.38 | Effect chooser ignores LARGE; needs a coordinated layout change |
| Tooltip / MIDI Learn note, 12 / 15 Book | 6.56 / 8.21 | 6.56 / 8.21 | LARGE already works; both sizes below the line |
| Menu items, 13.8 / 17.25 Book | 7.55 / 9.44 | 7.55 / 9.44 | LARGE works at these sizes |
| Menu title, 82 % of items, Demi | 6.27 / 7.84 | 6.27 / 7.84 | Both below the line |
| Menu group headings, 72 % of items, Demi | 5.50 / 6.88 | 5.50 / 6.88 | Smallest functional text in the fresh 4K window |
| Typed knob value's unit, 70 % of the entry's size, Book | 5.28 / 6.61 | 5.28 / 6.61 | Entry follows LARGE; unit still small |

At 860 px the standalone settings fixture's 19 items plus title force both lettering sizes
down to about 12.16 px em: x-height 6.65 px for items and 4.85 px for group headings. At
1035 px LARGE is reduced to about 14.63 px em, only a little above STANDARD's 13.8. A
plug-in's shorter menu fits better; installed packs can make either menu taller. These
conditional shrinkages are in the measurements and overlay manifest.

The typical 15 / 17-unit numbers' x-reference, across the widths above, is respectively:
2.30 / 2.61, 2.77 / 3.14, 3.69 / 4.18, 4.61 / 5.23, 5.53 / 6.27,
7.38 / 8.36, 8.09 / 9.17, and 8.31 / 9.42 px. Increasing the window is much more effective
than LARGE alone for the panel. The already approved panel has tight legends; increasing
all its type again would require moving controls, outside this survey's scope.

## Changes and proposals

1. **Implemented: QUALITY's small captions follow LARGE, 15 to 17 units.** R-LETTERING
   names the panel's LO / HI small print as part of the 15 to 17 step, but the three QUALITY
   positions remained hard-coded at 15 even before skin packs. `large_size` is an optional
   layout property; the official layout uses it for these three captions only. STANDARD and
   older packs' fixed sizes are unchanged. Font-fit checks use the selected size too. At
   1380 px this remains tiny (cap 4.92 to 5.58 px); at 3026 px the x-reference goes from 8.09
   to 9.17 px, cap 10.79 to 12.23. The owner should inspect the crops before accepting this
   interpretation of R-LETTERING.
2. **Highest value, owner's eye: a larger minimum for tooltips and menu headings.** A Book
   font size of 17 logical pixels gives a 9.30 px x-height; 21.25 in LARGE gives 11.62.
   Headings also need a minimum of about 17 rather than a 0.72 multiplier. Keep the title's
   band, colour and spacing. A scrolling menu or shorter submenus would preserve that minimum
   in small windows; shrinking the entire menu defeats LARGE. This changes density and
   behaviour, so it is proposed, not implemented. The current tooltip and ordinary-menu paths
   already honour LARGE; increasing their baseline is a design choice, not an omitted setting.
3. **High value, coordinate with preset-browser work: a drawer minimum or responsive LARGE.**
   At 1380 px changing the list's pitch from 4 to 4.5 would raise its lowercase height from
   8.70 to 9.79 px. Rows, columns, scroll counts and hit targets must change together. A
   physical minimum near 9.5 px x-height needs still larger pitch in the minimum window.
   The search is already pitch 4.5. R-LETTERING explicitly left the drawer unchanged; do not
   treat a new drawer layout as a bug fix. The current action captions are already on caps
   at 28 units, rather than the old tiny legends above them.
4. **Medium value, owner's eye: FX chooser text.** It is a menu made from a fixed dot display,
   introduced after R-LETTERING. A 1.25 step makes its 4-unit pitch 5 (10.48 px x-height at
   1380 px), but nine rows of dots including descenders no longer fit 40-unit lines. Its
   existing plate is 540 units tall with only about 564 available below the drawer. A larger
   font requires fewer visible rows, altered columns and shared drawing/hit-test geometry.
   The FX context menu already follows LARGE. The chooser is left for a coordinated fix.
5. **Medium value, owner's eye: keyboard mapping and tab/rail caps in LARGE.** Extend LARGE
   deliberately with fitted labels, rather than scale caps blindly: PANEL's narrow cap and
   ARP's UP/DOWN need width checks. The keyboard, caps and displays were explicitly outside
   the original enlargement. Do not alter the approved realistic panel to accommodate them.
6. **Low cost, owner's eye: make the existing sizing choices clearer.** The fresh 4K opening
   is already 3026 px wide, beyond the 200 % menu entry. A larger default would gain at most
   2.7 % before the 3108 px limit. Prefer explaining FIT TO SCREEN and considering an explicit
   4K-friendly choice or access to LETTERING beside it. Do not silently override saved sizes.
   Leave decorative maker lettering and the realistic face's layout for the owner's judgement.

## Evidence and limits

The off-repository handoff contains 224 whole-window baseline PNGs: eight widths, two looks,
two lettering settings, seven states (strip, keyboard with typing, presets, rename, MIDI list,
arp, FX with chooser). A further 32 show the settings fixture and tooltip in every combination.
The fixture uses the panel's real menu renderer and the built-in standalone's 0.3.0 item list;
it is not a live host capture. Eight before/after comparison renders cover the narrow change
in both looks at 1380 and 3026 px. Crops retain native pixels; the manifest names their source
and rectangle. The image inventory includes dimensions and SHA-256 hashes.

`examples/png.rs` now accepts `CA72_TIP=1`, `CA72_MENU=settings` and
`CA72_PRINT_SVG=<file>` for reproduction. `examples/golden.rs` accepts
`CA72_GOLDEN_LAYOUT=<file>` to compare the frozen v1 layout with the official one. SVG text
was measured before rasterisation, font outlines with fontTools; action-caption advances
were shaped with HarfBuzz. Dynamic strings sharing one size are explicitly grouped in the
appendix; each fixed panel/strip text instance has its own row and drawing coordinates.
The CSV contains every width and both lettering modes, plus the fixed LARGE result.

`cargo test -p ca72-panel` passes, including both approved-panel tests and frozen-pack
compatibility. `cargo clippy -p ca72-panel --all-targets -- -D warnings` passes. The generated
32-frame golden comparison has 16 identical STANDARD frames; the 16 LARGE frames change
only QUALITY's three captions, at scales 0.44 and 1.0 in both looks and all four scenes.
Neither `tests/approved.png` nor `tests/approved-large.png` was changed. They exclude this
QUALITY region by design; the separate before/after comparison records its change.

Not tested: a real 4K monitor, a human comfort trial, hosts, OS/host scales other than 1x,
third-party font packs, a live installed-effects scan or menus with arbitrary installed-pack
counts. Geometry and existing hit targets are unchanged by the fix; panel tests cover both
looks and the minimum viewport. Proposed reflows need their own small-screen and pointer
checks before implementation. The original review does not establish which build it tested;
this survey makes no claim to reproduce that review's exact environment.

## Follow-up, 2026-11-04: a minimum, and the classes raised to it

Base `88575ba` (0.3.1). decisions.md R-READABILITY-MINIMUM records the decision. This pass
renders the real editor (`the_readability_pngs` in the plug-in's editor tests, not the panel
example) and measures each class of text, now including the LFO and MACRO pages, the typed
value, the SIZE menu and 150 % host scaling.

**The minimum:** x-height above 9 physical pixels (cap height at least 12.2) at FIT TO
SCREEN on a 4K screen at 100 %, the warning line above. It applies to LARGE's panel print and
to the tip, notes, menus and typed value in both letterings; STANDARD's panel print is the
approved panel (R-LETTERING) and stays as it was.

**Window sizes.** At 150 % a 4K screen is 2560 by 1440 logical pixels; FIT TO SCREEN gives
2017 logical, 3026 physical, so the panel's print is the same size in physical pixels as at
100 %, and only what is set in logical pixels differs. SIZE 100 % is 1380 logical (1380 or
2070 physical). Since R-WINDOW-DEFAULT a new editor opens at the width last chosen on this
computer, which may be smaller than FIT TO SCREEN.

**Below the line before:** the tip, menu items, titles and group heads, and the typed value's
unit in both letterings at 100 % (heads and unit still at 150 %); in LARGE, the dial captions
(M-SEC., SEC., OSC. 3, NOISE, the LFOs' Hz and SEC.) and OSC. 3 CONTROL. Also below it and
left: IDLE FOUNDRY (decorative) and STANDARD's panel numbers, captions, QUALITY, OSC. 3
CONTROL and strip small print (the owner's STANDARD).

**Changed:** tips, notes and menus are never smaller than the panel's legends (19 units,
LARGE 22), which takes effect only above about 1960 logical pixels wide; menu group heads
0.72 to 0.8 of the items; the typed value's unit 0.7 to 0.8 of its field; LARGE's dial captions
16 to 17 and OSC. 3 CONTROL 16 to 17 (its LARGE place 488 to 491, keeping its clearance from
the section's rule). Every class in the table now clears the line at 4K FIT TO SCREEN in LARGE,
and every tip, note, menu and typed value does in STANDARD.

x-height / cap height in physical pixels, from the em and the outlines (the panel's print is
also stroked by 0.7 units, about 0.7 px more at 4K). Pixel ink heights in the renders agree
within about half a pixel: the tip's cap 9.48 to 14.20 px, menu items 10.45 to 16.11, titles
8.56 to 13.18, heads 7.50 to 13.00; LARGE's NOISE caption grew 0.78 px, an LFO's SEC. 0.89.

#### 4K 100 %, FIT TO SCREEN (3026 px)

| Class | STANDARD before | STANDARD after | LARGE before | LARGE after |
|---|---:|---:|---:|---:|
| Panel legends | 10.25 / 13.67 | 10.25 / 13.67 | 11.87 / 15.83 | 11.87 / 15.83 |
| Section titles (CONTROLLERS … OUTPUT) | 28.05 / 37.41 | 28.05 / 37.41 | 28.05 / 37.41 | 28.05 / 37.41 |
| Dial numbers (panel, strip, LFO, MACRO, ARP) | 8.09 / 10.79 | 8.09 / 10.79 | 9.17 / 12.23 | 9.17 / 12.23 |
| Dial captions: M-SEC., SEC., OSC. 3, NOISE, LFO Hz and SEC. | 7.55 / 10.07 | 7.55 / 10.07 | 8.63 / 11.51 | 9.17 / 12.23 |
| OSC. 3 CONTROL | 8.63 / 11.51 | 8.63 / 11.51 | 8.63 / 11.51 | 9.17 / 12.23 |
| QUALITY ULTRA / HI / LO | 8.09 / 10.79 | 8.09 / 10.79 | 9.17 / 12.23 | 9.17 / 12.23 |
| IDLE FOUNDRY (maker name, decorative) | 6.47 / 8.63 | 6.47 / 8.63 | 6.47 / 8.63 | 6.47 / 8.63 |
| Strip legends, GLOBAL/ARP bank legends, LFO legends | 11.33 / 15.11 | 11.33 / 15.11 | 12.41 / 16.55 | 12.41 / 16.55 |
| Strip small print (dB, L, R, %) | 8.63 / 11.51 | 8.63 / 11.51 | 9.71 / 12.95 | 9.71 / 12.95 |
| Strip / arp / FX tab caps | 9.17 / 12.23 | 9.17 / 12.23 | 9.17 / 12.23 | 9.17 / 12.23 |
| Rail caps (SAVE, PANEL, GLOBAL …) | 10.79 / 14.39 | 10.79 / 14.39 | 10.79 / 14.39 | 10.79 / 14.39 |
| Keyboard mapping letters (Book) | 9.59 / 12.95 | 9.59 / 12.95 | 9.59 / 12.95 | 9.59 / 12.95 |
| Keyboard TYPING / PANIC / − / + | 10.79 / 14.39 | 10.79 / 14.39 | 10.79 / 14.39 | 10.79 / 14.39 |
| Presets drawer list, tags, MIDI list (dots, pitch 4) | 19.08 / 26.87 | 19.08 / 26.87 | 19.08 / 26.87 | 19.08 / 26.87 |
| Tip, MIDI Learn note (Book) | 6.56 / 8.87 | 10.12 / 13.67 | 8.21 / 11.08 | 11.72 / 15.83 |
| Menu items (Book, 1.15 × tip) | 7.55 / 10.20 | 11.64 / 15.72 | 9.44 / 12.75 | 13.47 / 18.20 |
| Menu titles (Demi, 0.82 × items) | 6.27 / 8.36 | 9.66 / 12.89 | 7.84 / 10.45 | 11.19 / 14.93 |
| Menu group heads (Demi, 0.72 → 0.8 × items) | 5.50 / 7.34 | 9.43 / 12.58 | 6.88 / 9.18 | 10.92 / 14.56 |
| Typed value's unit (Book, 0.7 → 0.8 × field) | 5.28 / 7.14 | 9.31 / 12.58 | 6.61 / 8.92 | 10.78 / 14.56 |

#### 4K 150 %, FIT TO SCREEN (2017 logical, 3026 px)

| Class | STANDARD before | STANDARD after | LARGE before | LARGE after |
|---|---:|---:|---:|---:|
| Panel legends | 10.25 / 13.67 | 10.25 / 13.67 | 11.87 / 15.83 | 11.87 / 15.83 |
| Section titles (CONTROLLERS … OUTPUT) | 28.05 / 37.41 | 28.05 / 37.41 | 28.05 / 37.41 | 28.05 / 37.41 |
| Dial numbers (panel, strip, LFO, MACRO, ARP) | 8.09 / 10.79 | 8.09 / 10.79 | 9.17 / 12.23 | 9.17 / 12.23 |
| Dial captions: M-SEC., SEC., OSC. 3, NOISE, LFO Hz and SEC. | 7.55 / 10.07 | 7.55 / 10.07 | 8.63 / 11.51 | 9.17 / 12.23 |
| OSC. 3 CONTROL | 8.63 / 11.51 | 8.63 / 11.51 | 8.63 / 11.51 | 9.17 / 12.23 |
| QUALITY ULTRA / HI / LO | 8.09 / 10.79 | 8.09 / 10.79 | 9.17 / 12.23 | 9.17 / 12.23 |
| IDLE FOUNDRY (maker name, decorative) | 6.47 / 8.63 | 6.47 / 8.63 | 6.47 / 8.63 | 6.47 / 8.63 |
| Strip legends, GLOBAL/ARP bank legends, LFO legends | 11.33 / 15.11 | 11.33 / 15.11 | 12.41 / 16.55 | 12.41 / 16.55 |
| Strip small print (dB, L, R, %) | 8.63 / 11.51 | 8.63 / 11.51 | 9.71 / 12.95 | 9.71 / 12.95 |
| Strip / arp / FX tab caps | 9.17 / 12.23 | 9.17 / 12.23 | 9.17 / 12.23 | 9.17 / 12.23 |
| Rail caps (SAVE, PANEL, GLOBAL …) | 10.79 / 14.39 | 10.79 / 14.39 | 10.79 / 14.39 | 10.79 / 14.39 |
| Keyboard mapping letters (Book) | 9.59 / 12.95 | 9.59 / 12.95 | 9.59 / 12.95 | 9.59 / 12.95 |
| Keyboard TYPING / PANIC / − / + | 10.79 / 14.39 | 10.79 / 14.39 | 10.79 / 14.39 | 10.79 / 14.39 |
| Presets drawer list, tags, MIDI list (dots, pitch 4) | 19.08 / 26.87 | 19.08 / 26.87 | 19.08 / 26.87 | 19.08 / 26.87 |
| Tip, MIDI Learn note (Book) | 9.85 / 13.30 | 10.12 / 13.67 | 12.31 / 16.63 | 12.31 / 16.63 |
| Menu items (Book, 1.15 × tip) | 11.32 / 15.30 | 11.64 / 15.72 | 14.15 / 19.12 | 14.15 / 19.12 |
| Menu titles (Demi, 0.82 × items) | 9.40 / 12.54 | 9.66 / 12.89 | 11.75 / 15.68 | 11.75 / 15.68 |
| Menu group heads (Demi, 0.72 → 0.8 × items) | 8.26 / 11.01 | 9.43 / 12.58 | 10.32 / 13.77 | 11.47 / 15.30 |
| Typed value's unit (Book, 0.7 → 0.8 × field) | 7.93 / 10.71 | 9.31 / 12.58 | 9.91 / 13.39 | 11.32 / 15.30 |

#### 4K 100 %, SIZE 100 % (1380 px)

| Class | STANDARD before | STANDARD after | LARGE before | LARGE after |
|---|---:|---:|---:|---:|
| Panel legends | 4.67 / 6.23 | 4.67 / 6.23 | 5.41 / 7.22 | 5.41 / 7.22 |
| Section titles (CONTROLLERS … OUTPUT) | 12.79 / 17.06 | 12.79 / 17.06 | 12.79 / 17.06 | 12.79 / 17.06 |
| Dial numbers (panel, strip, LFO, MACRO, ARP) | 3.69 / 4.92 | 3.69 / 4.92 | 4.18 / 5.58 | 4.18 / 5.58 |
| Dial captions: M-SEC., SEC., OSC. 3, NOISE, LFO Hz and SEC. | 3.44 / 4.59 | 3.44 / 4.59 | 3.94 / 5.25 | 4.18 / 5.58 |
| OSC. 3 CONTROL | 3.94 / 5.25 | 3.94 / 5.25 | 3.94 / 5.25 | 4.18 / 5.58 |
| QUALITY ULTRA / HI / LO | 3.69 / 4.92 | 3.69 / 4.92 | 4.18 / 5.58 | 4.18 / 5.58 |
| IDLE FOUNDRY (maker name, decorative) | 2.95 / 3.94 | 2.95 / 3.94 | 2.95 / 3.94 | 2.95 / 3.94 |
| Strip legends, GLOBAL/ARP bank legends, LFO legends | 5.17 / 6.89 | 5.17 / 6.89 | 5.66 / 7.55 | 5.66 / 7.55 |
| Strip small print (dB, L, R, %) | 3.94 / 5.25 | 3.94 / 5.25 | 4.43 / 5.91 | 4.43 / 5.91 |
| Strip / arp / FX tab caps | 4.18 / 5.58 | 4.18 / 5.58 | 4.18 / 5.58 | 4.18 / 5.58 |
| Rail caps (SAVE, PANEL, GLOBAL …) | 4.92 / 6.56 | 4.92 / 6.56 | 4.92 / 6.56 | 4.92 / 6.56 |
| Keyboard mapping letters (Book) | 4.37 / 5.91 | 4.37 / 5.91 | 4.37 / 5.91 | 4.37 / 5.91 |
| Keyboard TYPING / PANIC / − / + | 4.92 / 6.56 | 4.92 / 6.56 | 4.92 / 6.56 | 4.92 / 6.56 |
| Presets drawer list, tags, MIDI list (dots, pitch 4) | 8.70 / 12.25 | 8.70 / 12.25 | 8.70 / 12.25 | 8.70 / 12.25 |
| Tip, MIDI Learn note (Book) | 6.56 / 8.87 | 6.56 / 8.87 | 8.21 / 11.08 | 8.21 / 11.08 |
| Menu items (Book, 1.15 × tip) | 7.55 / 10.20 | 7.55 / 10.20 | 9.44 / 12.75 | 9.44 / 12.75 |
| Menu titles (Demi, 0.82 × items) | 6.27 / 8.36 | 6.27 / 8.36 | 7.84 / 10.45 | 7.84 / 10.45 |
| Menu group heads (Demi, 0.72 → 0.8 × items) | 5.50 / 7.34 | 6.12 / 8.16 | 6.88 / 9.18 | 7.65 / 10.20 |
| Typed value's unit (Book, 0.7 → 0.8 × field) | 5.28 / 7.14 | 6.04 / 8.16 | 6.61 / 8.92 | 7.55 / 10.20 |

**Small screens and hit targets.** At the 1366 by 768 laptop's 1076, SIZE 75 % (1035) and the
grip's 860, the tips, control menus and the typed value have the extents and places they had
before; only the heads and the unit are larger, within the same rows and fields. Every tip,
menu and typed value stays inside the window at every size rendered. Menus are found where they
are drawn (`Menu::hit` uses the same layout).

### Round 2, 2026-11-04

**The settings' menu** is set in columns where one would not fit below its key (decisions.md
R-READABILITY-MINIMUM, round 2). Its lettering and row pitch are unchanged. Where it opens:

| Window | STANDARD | LARGE |
|---|---|---|
| 4K FIT TO SCREEN, 100 % and 150 % | two columns, below its key | two columns, below its key |
| SIZE 100 % | three columns, below its key | three columns, below its key |
| Laptop (1076), SIZE 75 % | four columns, below its key | four columns, below its key |
| 860 | four columns, below its key | four columns, over the rail and its key |

In round 1 (and before it) it rose over the rail and its key at every one of these sizes. It is
no longer shrunk to fit the window's height, so in small windows its items
grow:

| Window | Lettering | Settings' items, em in px |
|---|---|---|
| Laptop (1076) | LARGE | 16.88 → **17.25** |
| SIZE 75 % | LARGE | 16.24 → **17.25** |
| 860 | STANDARD | 13.49 → **13.80** |
| 860 | LARGE | 13.49 → **17.25** |

Every other size is unchanged (`round2/after/sizes.csv`).

**OSC. 3 CONTROL in LARGE** is 5 units higher. Its CONTROL is now 8.1 px from RANGE's 32' at 4K
FIT (7.1 before round 1, 3.2 after it); its size and x-height are as in round 1 (9.17 px).

**TICKS & TITLES: QUIET** (issue #145, decisions.md R-QUIET-PRINT; off by default).
- **Ticks:** 0.725 of their width (LARGE 0.8), 0.72 of their length, at 80 % ink. At 4K FIT
  a dial's tick goes from 3.9 px wide and 12.7 long to 2.8 by 9.1.
- **Section titles:** 0.75 of their size, at 80 % ink: 28.05 / 37.41 px x-height / cap height
  become 21.04 / 28.06.

Nothing else changes size.

### Round 3, 2026-11-04

**The settings' menu** now opens below its key at every size rendered, LARGE at 860 included
(decisions.md R-READABILITY-MINIMUM, round 3). Its groups are spread over its columns as evenly
as they go, and where four columns do not fit it is made only as much smaller as fits.

| Window | STANDARD | LARGE |
|---|---|---|
| 4K FIT TO SCREEN, 100 % and 150 % | two columns (9, 11 rows) | two (9, 11) |
| SIZE 100 % | three (6, 6, 8) | three (6, 6, 8) |
| 1080p FIT TO SCREEN (1513) | two (9, 11) | three (6, 6, 8) |
| Laptop (1076) | three (6, 6, 8) | four (6, 6, 4, 4) |
| SIZE 75 % | four (6, 6, 4, 4) | four (6, 6, 4, 4) |
| 860 | four (6, 6, 4, 4) | four (6, 6, 4, 4), smaller |

The four-column layouts were six, six, six and two rows, with MPE alone in the last column.
Round 2's table above gave the laptop's STANDARD as four columns; its frames show three. In
`round3/after/sizes.csv` only one window and lettering changes size (the four 1080p rows are
new, and match SIZE 100 %'s):

| Window | Lettering | Settings' items, em in px (round 1 → round 2 → round 3) |
|---|---|---|
| 860 | LARGE | 13.49 → 17.25 (over the rail) → **14.57** (below its key) |

### Round 4, 2026-11-04

**The standalone's settings' menu** (STANDALONE and AGENTS added: 27 rows as it opens, 29 at
its fullest) opens below its key at every size, in balanced columns, its lettering full size at
4K FIT TO SCREEN (decisions.md R-READABILITY-MINIMUM, round 4). Where four columns are taller
than the room under the key it is made smaller, as round 3 allows:

| Window | Lettering | Settings' items, em in px (as it opens / at its fullest) |
|---|---|---|
| SIZE 100 % | LARGE | 17.25 / **16.50** |
| Laptop (1076) | STANDARD | 13.80 / **12.86** |
| Laptop (1076) | LARGE | **14.26** / **12.86** |
| SIZE 75 % | both | **13.72** / **12.37** |
| 860 | both | **11.40** / **10.28** |

Every other case keeps its lettering's size. The layout code is unchanged; a fifth column
(measured, not adopted) would give 13.89 px at 860 in LARGE.

### Round 5, 2026-11-04

**The settings' menu may have five columns** (the owner's choice; decisions.md
R-READABILITY-MINIMUM, round 5). The rule is as before: fewest columns that fit, then the most
even, then as little smaller as needed. Five are used only where four would make the text
smaller. The plug-in's menu does not change at any size: five columns of its groups are no
shorter than four (six rows), so four stay. That includes LARGE at 860, at 14.57 px. The
standalone's changes:

| Window | Lettering | Settings' items, em in px, as it opens / at its fullest (round 4 → round 5) |
|---|---|---|
| SIZE 100 % | LARGE | 17.25 / 16.50 → 17.25 / 17.25 |
| Laptop (1076) | STANDARD | 13.80 / 12.86 → 13.80 / 13.80 |
| Laptop (1076) | LARGE | 14.26 / 12.86 → 17.25 / **15.00** |
| SIZE 75 % | STANDARD | 13.72 / 12.37 → 13.80 / 13.80 |
| SIZE 75 % | LARGE | 13.72 / 12.37 → **16.72** / **14.42** |
| 860 | STANDARD | 11.40 / 10.28 → 13.80 / **11.99** |
| 860 | LARGE | 11.40 / 10.28 → **13.89** / **11.99** |

STANDARD now keeps its full size as it opens at every size. Where it is still made smaller at
its fullest, the least shrink puts LOOK alone in a first column (3, 6, 7, 7, 6 rows). The more
even 6, 6, 6, 5, 6 is wider, so it would be about 3.5 % smaller (11.57 px at 860). Round 4's
12.60 px for 860 LARGE at its fullest was measured before the COPIED row widened the menu.

**Evidence** is under `/mnt/lab/artifacts/ca-72/issue-70/`: `before/` and `after/` (308 frames
each, with `sizes.csv`), `crops/` (1:1 before | after of each changed class, both looks),
`golden-before/`, `golden-after/`, `golden-compare.txt` and `golden-regions.txt`; round 2's in
`round2/` (`index.html`, the matrix again in `round2/after/` with a `quiet` state, its crops and
golden frames against `dd23251`); round 3's in `round3/` (`index.html`, the matrix with 1080p
FIT in `round3/after/`, crops of the settings' menu, golden frames against `ef41abf`: none
differ); round 4's in `round4/` (`index.html`, crops of the standalone's menu, frames and
`sizes.csv` in `round4/frames/`); round 5's in `round5/` (`index.html`, crops of each changed
menu, round 4 left and round 5 right, the frames of both, golden frames against `1033538`:
none differ). Not tested: a
real 4K monitor, a host, Windows or macOS scaling, a human comfort trial.

See [the complete small-text inventory](readability-4k-text.md).
