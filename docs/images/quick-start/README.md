# Quick-start pictures

These are the editor's own frames of CA-72 0.3.2, not captures from a particular host: the
plug-in's and the standalone's whole windows, drawn headlessly by the ignored test
`the_quick_start_pngs` in `crates/ca72-plugin/src/editor.rs`, 1720 logical pixels wide at
1.25 physical pixels each, in the default realistic look (`Skin::Worn`) and standard
lettering. The test sets each state through the editor's parameters, pointer and keys, as a
player would, so the tips, menus, notes and rings are the editor's own:

```sh
CA72_QUICK_START_PNG=<frames> cargo test -p ca72-plugin --features standalone-production \
  --lib -- --ignored the_quick_start_pngs
```

- `01`: the standalone as it first opens, the keyboard below.
- `02`: a plug-in's window at Init, GLOBAL's page below.
- `03`: oscillator 3 at LO on its triangle, OSC. 3 CONTROL off (its tip shown), OSCILLATOR
  MODULATION on, MODULATION MIX at OSC. 3, the MOD wheel at 0.3; and the wheels.
- `04`: the tips of MODULATION MIX, OSCILLATOR MODULATION, FILTER MODULATION and the
  MODULATION wheel (decisions.md R-MOD-WORDING).
- `05`: the LFO page, LFO 1's OUT on TUNE at 4 %, TUNE pointed at.
- `06`: POLY, four voices, WIDTH 70 %.
- `07`: QUALITY at LO, HI and ULTRA, ULTRA's first note (its threads are the rendering
  computer's).
- `08`: the panel's OUTPUT and GLOBAL's, DRIVE at 6 dB with its tip.
- `09`: the presets' drawer over the factory library.
- `10`: CUTOFF FREQUENCY's menu, then CUTOFF waiting for MIDI Learn.
- `11`: the standalone's MIDI map mode, four controls learned, AMOUNT OF CONTOUR waiting.
- `12`: the ARP drawer (ON), and the standalone's FX drawer with the tests' stand-in effects
  (their names are illustrative).

ImageMagick crops and combines the frames, strips metadata and writes indexed PNGs of 160
colours, 1280 pixels wide at most; all twelve together are about 2.3 MB. The crop script
and the frames are on the lab share, `/mnt/lab/artifacts/ca-72/issue-73/` (`crop.sh`,
`frames/`). For an update, render the frames again rather than painting a different setting
onto a picture.

The 0.3.2 refresh includes the POLES selector and FILTER MODE's four response pictures.
All twelve images were regenerated from the release source, rather than compositing new
controls onto earlier screenshots. Raw frames and the crop script for this refresh are in
`/mnt/lab/artifacts/ca-72/release-032/screenshots/` (`raw/`, `crop.sh` and `cropped/`).
