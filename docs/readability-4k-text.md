# Small text: complete instance table

Companion to [the 4K survey](readability-4k.md), measured on base `17f1b5f`.
All sizes are physical pixels at OS/host scale 1. Each cell is **x-height / cap height**.
The x-height is a same-font reference for capitals and numerals; their cap height is the
more direct glyph measure. Rows with x-reference at most 9 are included conservatively.
Fixed labels are listed separately by drawing coordinates; dynamic text sharing a size is
explicitly grouped. The LARGE column is the base build, before the three QUALITY captions
grow from 15 to 17 on this branch. Each becomes 9.17 / 12.23 at the fresh size and
4.18 / 5.58 at 1380 px. All other table values are unchanged by the fix.

**2026-11-04 (R-READABILITY-MINIMUM):** these tables are the 0.3.0 survey's. Since then, in
LARGE only, the dial captions (M-SEC., SEC., OSC. 3, NOISE, the LFOs' Hz and SEC.) and OSC. 3
CONTROL are 17 units: 9.17 / 12.23 at the fresh size, 4.18 / 5.58 at 1380 px. The tips', notes'
and menus' rows change above about 1960 logical pixels wide, and the menu heads and typed unit
at every size; [the survey's follow-up](readability-4k.md#follow-up-2026-11-04-a-minimum-and-the-classes-raised-to-it)
has the new values.

## Fresh 4K opening: 3026 × 1728

| Element | Where (drawing x, y where supplied) | STANDARD x / cap px | LARGE x / cap px |
|---|---|---:|---:|
| –2 | panel tune (151, 454) | 8.09 / 10.79 | 9.17 / 12.23 |
| –1 | panel tune (151, 363) | 8.09 / 10.79 | 9.17 / 12.23 |
| 0 | panel tune (229, 322) | 8.09 / 10.79 | 9.17 / 12.23 |
| 1 | panel tune (303, 365) | 8.09 / 10.79 | 9.17 / 12.23 |
| 2 | panel tune (303, 451) | 8.09 / 10.79 | 9.17 / 12.23 |
| 0 | panel glide (83, 690) | 8.09 / 10.79 | 9.17 / 12.23 |
| 2 | panel glide (40, 615) | 8.09 / 10.79 | 9.17 / 12.23 |
| 4 | panel glide (83, 541) | 8.09 / 10.79 | 9.17 / 12.23 |
| 6 | panel glide (169, 541) | 8.09 / 10.79 | 9.17 / 12.23 |
| 8 | panel glide (212, 615) | 8.09 / 10.79 | 9.17 / 12.23 |
| 10 | panel glide (170, 692) | 8.09 / 10.79 | 9.17 / 12.23 |
| 0 | panel mod_mix (289, 690) | 8.09 / 10.79 | 9.17 / 12.23 |
| 2 | panel mod_mix (246, 615) | 8.09 / 10.79 | 9.17 / 12.23 |
| 4 | panel mod_mix (289, 541) | 8.09 / 10.79 | 9.17 / 12.23 |
| 6 | panel mod_mix (375, 541) | 8.09 / 10.79 | 9.17 / 12.23 |
| 8 | panel mod_mix (418, 615) | 8.09 / 10.79 | 9.17 / 12.23 |
| 10 | panel mod_mix (376, 692) | 8.09 / 10.79 | 9.17 / 12.23 |
| OSC. 3 | panel mod_mix (282, 715) | 7.55 / 10.07 | 8.63 / 11.51 |
| NOISE | panel osc3_control (381, 715) | 7.55 / 10.07 | 8.63 / 11.51 |
| LO | panel osc1_range (528, 289) | 8.09 / 10.79 | 9.17 / 12.23 |
| 32' | panel osc1_range (545, 239) | 8.09 / 10.79 | 9.17 / 12.23 |
| 16' | panel osc1_range (591, 213) | 8.09 / 10.79 | 9.17 / 12.23 |
| 8' | panel osc1_range (643, 213) | 8.09 / 10.79 | 9.17 / 12.23 |
| 4' | panel osc1_range (686, 241) | 8.09 / 10.79 | 9.17 / 12.23 |
| 2' | panel osc1_range (705, 289) | 8.09 / 10.79 | 9.17 / 12.23 |
| LO | panel osc2_range (528, 496) | 8.09 / 10.79 | 9.17 / 12.23 |
| 32' | panel osc2_range (545, 446) | 8.09 / 10.79 | 9.17 / 12.23 |
| 16' | panel osc2_range (591, 420) | 8.09 / 10.79 | 9.17 / 12.23 |
| 8' | panel osc2_range (643, 420) | 8.09 / 10.79 | 9.17 / 12.23 |
| 4' | panel osc2_range (686, 448) | 8.09 / 10.79 | 9.17 / 12.23 |
| 2' | panel osc2_range (705, 496) | 8.09 / 10.79 | 9.17 / 12.23 |
| LO | panel osc3_control (528, 702) | 8.09 / 10.79 | 9.17 / 12.23 |
| 32' | panel osc3_range (545, 652) | 8.09 / 10.79 | 9.17 / 12.23 |
| 16' | panel osc3_range (591, 626) | 8.09 / 10.79 | 9.17 / 12.23 |
| 8' | panel osc3_range (643, 626) | 8.09 / 10.79 | 9.17 / 12.23 |
| 4' | panel osc3_range (686, 654) | 8.09 / 10.79 | 9.17 / 12.23 |
| 2' | panel osc3_range (705, 702) | 8.09 / 10.79 | 9.17 / 12.23 |
| –7 | panel osc2_frequency (755, 586) | 8.09 / 10.79 | 9.17 / 12.23 |
| –5 | panel osc2_frequency (719, 526) | 8.09 / 10.79 | 9.17 / 12.23 |
| –3 | panel osc2_frequency (731, 457) | 8.09 / 10.79 | 9.17 / 12.23 |
| –1 | panel osc2_frequency (786, 415) | 8.09 / 10.79 | 9.17 / 12.23 |
| 1 | panel osc2_frequency (854, 416) | 8.09 / 10.79 | 9.17 / 12.23 |
| 3 | panel osc2_frequency (905, 459) | 8.09 / 10.79 | 9.17 / 12.23 |
| 5 | panel osc2_frequency (917, 525) | 8.09 / 10.79 | 9.17 / 12.23 |
| 7 | panel osc2_frequency (883, 583) | 8.09 / 10.79 | 9.17 / 12.23 |
| –7 | panel osc3_frequency (755, 794) | 8.09 / 10.79 | 9.17 / 12.23 |
| –5 | panel osc3_frequency (719, 734) | 8.09 / 10.79 | 9.17 / 12.23 |
| –3 | panel osc3_frequency (731, 665) | 8.09 / 10.79 | 9.17 / 12.23 |
| –1 | panel osc3_frequency (786, 623) | 8.09 / 10.79 | 9.17 / 12.23 |
| 1 | panel osc3_frequency (854, 624) | 8.09 / 10.79 | 9.17 / 12.23 |
| 3 | panel osc3_frequency (905, 667) | 8.09 / 10.79 | 9.17 / 12.23 |
| 5 | panel osc3_frequency (917, 733) | 8.09 / 10.79 | 9.17 / 12.23 |
| 7 | panel osc3_frequency (883, 791) | 8.09 / 10.79 | 9.17 / 12.23 |
| 0 | panel osc1_volume (1177, 380) | 8.09 / 10.79 | 9.17 / 12.23 |
| 2 | panel osc1_volume (1134, 305) | 8.09 / 10.79 | 9.17 / 12.23 |
| 4 | panel osc1_volume (1177, 231) | 8.09 / 10.79 | 9.17 / 12.23 |
| 6 | panel osc1_volume (1263, 231) | 8.09 / 10.79 | 9.17 / 12.23 |
| 8 | panel osc1_volume (1306, 305) | 8.09 / 10.79 | 9.17 / 12.23 |
| 10 | panel osc1_volume (1264, 382) | 8.09 / 10.79 | 9.17 / 12.23 |
| 0 | panel ext_volume (1578, 484) | 8.09 / 10.79 | 9.17 / 12.23 |
| 2 | panel ext_volume (1535, 409) | 8.09 / 10.79 | 9.17 / 12.23 |
| 4 | panel ext_volume (1578, 335) | 8.09 / 10.79 | 9.17 / 12.23 |
| 6 | panel ext_volume (1664, 335) | 8.09 / 10.79 | 9.17 / 12.23 |
| 8 | panel ext_volume (1707, 409) | 8.09 / 10.79 | 9.17 / 12.23 |
| 10 | panel ext_volume (1665, 486) | 8.09 / 10.79 | 9.17 / 12.23 |
| 0 | panel osc2_volume (1177, 587) | 8.09 / 10.79 | 9.17 / 12.23 |
| 2 | panel osc2_volume (1134, 512) | 8.09 / 10.79 | 9.17 / 12.23 |
| 4 | panel osc2_volume (1177, 438) | 8.09 / 10.79 | 9.17 / 12.23 |
| 6 | panel osc2_volume (1263, 438) | 8.09 / 10.79 | 9.17 / 12.23 |
| 8 | panel osc2_volume (1306, 512) | 8.09 / 10.79 | 9.17 / 12.23 |
| 10 | panel osc2_volume (1264, 589) | 8.09 / 10.79 | 9.17 / 12.23 |
| 0 | panel noise_volume (1578, 691) | 8.09 / 10.79 | 9.17 / 12.23 |
| 2 | panel noise_volume (1535, 616) | 8.09 / 10.79 | 9.17 / 12.23 |
| 4 | panel noise_volume (1578, 542) | 8.09 / 10.79 | 9.17 / 12.23 |
| 6 | panel noise_volume (1664, 542) | 8.09 / 10.79 | 9.17 / 12.23 |
| 8 | panel noise_type (1707, 616) | 8.09 / 10.79 | 9.17 / 12.23 |
| 10 | panel noise_volume (1665, 693) | 8.09 / 10.79 | 9.17 / 12.23 |
| 0 | panel osc3_volume (1177, 793) | 8.09 / 10.79 | 9.17 / 12.23 |
| 2 | panel osc3_volume (1134, 718) | 8.09 / 10.79 | 9.17 / 12.23 |
| 4 | panel osc3_volume (1177, 644) | 8.09 / 10.79 | 9.17 / 12.23 |
| 6 | panel osc3_volume (1263, 644) | 8.09 / 10.79 | 9.17 / 12.23 |
| 8 | panel osc3_volume (1306, 718) | 8.09 / 10.79 | 9.17 / 12.23 |
| 10 | panel osc3_volume (1264, 795) | 8.09 / 10.79 | 9.17 / 12.23 |
| –4 | panel cutoff (2003, 351) | 8.09 / 10.79 | 9.17 / 12.23 |
| –2 | panel cutoff (2003, 260) | 8.09 / 10.79 | 9.17 / 12.23 |
| 0 | panel cutoff (2081, 219) | 8.09 / 10.79 | 9.17 / 12.23 |
| 2 | panel cutoff (2155, 262) | 8.09 / 10.79 | 9.17 / 12.23 |
| 4 | panel cutoff (2155, 348) | 8.09 / 10.79 | 9.17 / 12.23 |
| 0 | panel emphasis (2238, 380) | 8.09 / 10.79 | 9.17 / 12.23 |
| 2 | panel emphasis (2195, 305) | 8.09 / 10.79 | 9.17 / 12.23 |
| 4 | panel emphasis (2238, 231) | 8.09 / 10.79 | 9.17 / 12.23 |
| 6 | panel emphasis (2324, 231) | 8.09 / 10.79 | 9.17 / 12.23 |
| 8 | panel emphasis (2367, 305) | 8.09 / 10.79 | 9.17 / 12.23 |
| 10 | panel emphasis (2325, 382) | 8.09 / 10.79 | 9.17 / 12.23 |
| 0 | panel contour_amount (2440, 380) | 8.09 / 10.79 | 9.17 / 12.23 |
| 2 | panel contour_amount (2397, 305) | 8.09 / 10.79 | 9.17 / 12.23 |
| 4 | panel contour_amount (2440, 231) | 8.09 / 10.79 | 9.17 / 12.23 |
| 6 | panel contour_amount (2526, 231) | 8.09 / 10.79 | 9.17 / 12.23 |
| 8 | panel contour_amount (2569, 305) | 8.09 / 10.79 | 9.17 / 12.23 |
| 10 | panel contour_amount (2527, 382) | 8.09 / 10.79 | 9.17 / 12.23 |
| 10 | panel filter_attack (2002, 539) | 8.09 / 10.79 | 9.17 / 12.23 |
| 200 | panel keyboard_control_1 (1984, 493) | 8.09 / 10.79 | 9.17 / 12.23 |
| 600 | panel filter_attack (2040, 424) | 8.09 / 10.79 | 9.17 / 12.23 |
| 1 | panel filter_attack (2120, 424) | 8.09 / 10.79 | 9.17 / 12.23 |
| 5 | panel filter_attack (2159, 460) | 8.09 / 10.79 | 9.17 / 12.23 |
| 10 | panel filter_attack (2166, 521) | 8.09 / 10.79 | 9.17 / 12.23 |
| M-SEC. | panel filter_attack (2019, 568) | 7.55 / 10.07 | 8.63 / 11.51 |
| SEC. | panel filter_attack (2130, 568) | 7.55 / 10.07 | 8.63 / 11.51 |
| 10 | panel filter_decay (2203, 539) | 8.09 / 10.79 | 9.17 / 12.23 |
| 200 | panel filter_decay (2185, 493) | 8.09 / 10.79 | 9.17 / 12.23 |
| 600 | panel filter_decay (2241, 424) | 8.09 / 10.79 | 9.17 / 12.23 |
| 1 | panel filter_decay (2321, 424) | 8.09 / 10.79 | 9.17 / 12.23 |
| 5 | panel filter_decay (2360, 460) | 8.09 / 10.79 | 9.17 / 12.23 |
| 10 | panel filter_decay (2367, 521) | 8.09 / 10.79 | 9.17 / 12.23 |
| M-SEC. | panel filter_decay (2220, 568) | 7.55 / 10.07 | 8.63 / 11.51 |
| SEC. | panel filter_decay (2331, 568) | 7.55 / 10.07 | 8.63 / 11.51 |
| 0 | panel filter_sustain (2440, 568) | 8.09 / 10.79 | 9.17 / 12.23 |
| 2 | panel filter_sustain (2397, 493) | 8.09 / 10.79 | 9.17 / 12.23 |
| 4 | panel filter_sustain (2440, 419) | 8.09 / 10.79 | 9.17 / 12.23 |
| 6 | panel filter_sustain (2526, 419) | 8.09 / 10.79 | 9.17 / 12.23 |
| 8 | panel filter_sustain (2569, 493) | 8.09 / 10.79 | 9.17 / 12.23 |
| 10 | panel filter_sustain (2527, 570) | 8.09 / 10.79 | 9.17 / 12.23 |
| 10 | panel loudness_attack (2002, 760) | 8.09 / 10.79 | 9.17 / 12.23 |
| 200 | panel loudness_attack (1984, 714) | 8.09 / 10.79 | 9.17 / 12.23 |
| 600 | panel loudness_attack (2040, 645) | 8.09 / 10.79 | 9.17 / 12.23 |
| 1 | panel loudness_attack (2120, 645) | 8.09 / 10.79 | 9.17 / 12.23 |
| 5 | panel loudness_attack (2159, 681) | 8.09 / 10.79 | 9.17 / 12.23 |
| 10 | panel loudness_attack (2166, 742) | 8.09 / 10.79 | 9.17 / 12.23 |
| M-SEC. | panel loudness_attack (2019, 789) | 7.55 / 10.07 | 8.63 / 11.51 |
| SEC. | panel loudness_attack (2130, 789) | 7.55 / 10.07 | 8.63 / 11.51 |
| 10 | panel loudness_decay (2203, 760) | 8.09 / 10.79 | 9.17 / 12.23 |
| 200 | panel loudness_decay (2185, 714) | 8.09 / 10.79 | 9.17 / 12.23 |
| 600 | panel loudness_decay (2241, 645) | 8.09 / 10.79 | 9.17 / 12.23 |
| 1 | panel loudness_decay (2321, 645) | 8.09 / 10.79 | 9.17 / 12.23 |
| 5 | panel loudness_decay (2360, 681) | 8.09 / 10.79 | 9.17 / 12.23 |
| 10 | panel loudness_decay (2367, 742) | 8.09 / 10.79 | 9.17 / 12.23 |
| M-SEC. | panel loudness_decay (2220, 789) | 7.55 / 10.07 | 8.63 / 11.51 |
| SEC. | panel loudness_decay (2331, 789) | 7.55 / 10.07 | 8.63 / 11.51 |
| 0 | panel loudness_sustain (2440, 789) | 8.09 / 10.79 | 9.17 / 12.23 |
| 2 | panel loudness_sustain (2397, 714) | 8.09 / 10.79 | 9.17 / 12.23 |
| 4 | panel loudness_sustain (2440, 640) | 8.09 / 10.79 | 9.17 / 12.23 |
| 6 | panel loudness_sustain (2526, 640) | 8.09 / 10.79 | 9.17 / 12.23 |
| 8 | panel loudness_sustain (2569, 714) | 8.09 / 10.79 | 9.17 / 12.23 |
| 10 | panel loudness_sustain (2527, 791) | 8.09 / 10.79 | 9.17 / 12.23 |
| 0 | panel volume (2637, 377) | 8.09 / 10.79 | 9.17 / 12.23 |
| 2 | panel volume (2594, 302) | 8.09 / 10.79 | 9.17 / 12.23 |
| 4 | panel volume (2637, 228) | 8.09 / 10.79 | 9.17 / 12.23 |
| 6 | panel volume (2723, 228) | 8.09 / 10.79 | 9.17 / 12.23 |
| 8 | panel volume (2766, 302) | 8.09 / 10.79 | 9.17 / 12.23 |
| 10 | panel volume (2724, 379) | 8.09 / 10.79 | 9.17 / 12.23 |
| 0 | panel feedback (2637, 790) | 8.09 / 10.79 | 9.17 / 12.23 |
| 2 | panel feedback (2594, 715) | 8.09 / 10.79 | 9.17 / 12.23 |
| 4 | panel feedback (2637, 641) | 8.09 / 10.79 | 9.17 / 12.23 |
| 6 | panel feedback (2723, 641) | 8.09 / 10.79 | 9.17 / 12.23 |
| 8 | panel feedback (2766, 715) | 8.09 / 10.79 | 9.17 / 12.23 |
| 10 | panel feedback (2724, 792) | 8.09 / 10.79 | 9.17 / 12.23 |
| OSC. 3 | panel osc3_control (488, 626) | 8.63 / 11.51 | 8.63 / 11.51 |
| CONTROL | panel osc3_control (488, 645) | 8.63 / 11.51 | 8.63 / 11.51 |
| ULTRA | panel quality (3023, 351) | 8.09 / 10.79 | 8.09 / 10.79 |
| HI | panel quality (2983, 401) | 8.09 / 10.79 | 8.09 / 10.79 |
| LO | panel quality (3023, 453) | 8.09 / 10.79 | 8.09 / 10.79 |
| ON | strip left hand (292, 1307) | 8.09 / 10.79 | 9.17 / 12.23 |
| ON | strip left hand (292, 1387) | 8.09 / 10.79 | 9.17 / 12.23 |
| ON | strip left hand (292, 1227) | 8.09 / 10.79 | 9.17 / 12.23 |
| LAST | strip left hand (256, 1070) | 8.09 / 10.79 | 9.17 / 12.23 |
| LOW | strip left hand (216, 1141) | 8.09 / 10.79 | 9.17 / 12.23 |
| HIGH | strip left hand (296, 1141) | 8.09 / 10.79 | 9.17 / 12.23 |
| dB | strip OUTPUT (2935, 1153) | 8.63 / 11.51 | 9.71 / 12.95 |
| dB | strip OUTPUT (2935, 1377) | 8.63 / 11.51 | 9.71 / 12.95 |
| dB | strip OUTPUT (2935, 1601) | 8.63 / 11.51 | 9.71 / 12.95 |
| L | strip STEREO (1037, 1601) | 8.63 / 11.51 | 9.71 / 12.95 |
| R | strip STEREO (2309, 1601) | 8.63 / 11.51 | 9.71 / 12.95 |
| 2 | strip VOICES (546, 1457) | 8.09 / 10.79 | 9.17 / 12.23 |
| 3 | strip VOICES (510, 1415) | 8.09 / 10.79 | 9.17 / 12.23 |
| 4 | strip VOICES (506, 1360) | 8.09 / 10.79 | 9.17 / 12.23 |
| 5 | strip VOICES (537, 1314) | 8.09 / 10.79 | 9.17 / 12.23 |
| 6 | strip VOICES (589, 1296) | 8.09 / 10.79 | 9.17 / 12.23 |
| 7 | strip VOICES (641, 1314) | 8.09 / 10.79 | 9.17 / 12.23 |
| 8 | strip VOICES (672, 1360) | 8.09 / 10.79 | 9.17 / 12.23 |
| 9 | strip VOICES (668, 1415) | 8.09 / 10.79 | 9.17 / 12.23 |
| 10 | strip VOICES (633, 1459) | 8.09 / 10.79 | 9.17 / 12.23 |
| 0 | strip VOICES (546, 1681) | 8.09 / 10.79 | 9.17 / 12.23 |
| 2 | strip VOICES (503, 1606) | 8.09 / 10.79 | 9.17 / 12.23 |
| 4 | strip VOICES (546, 1532) | 8.09 / 10.79 | 9.17 / 12.23 |
| 6 | strip VOICES (632, 1532) | 8.09 / 10.79 | 9.17 / 12.23 |
| 8 | strip VOICES (675, 1606) | 8.09 / 10.79 | 9.17 / 12.23 |
| 10 | strip VOICES (633, 1683) | 8.09 / 10.79 | 9.17 / 12.23 |
| 0 | strip STEREO (1116, 1457) | 8.09 / 10.79 | 9.17 / 12.23 |
| 2 | strip STEREO (1073, 1382) | 8.09 / 10.79 | 9.17 / 12.23 |
| 4 | strip STEREO (1116, 1308) | 8.09 / 10.79 | 9.17 / 12.23 |
| 6 | strip STEREO (1202, 1308) | 8.09 / 10.79 | 9.17 / 12.23 |
| 8 | strip STEREO (1245, 1382) | 8.09 / 10.79 | 9.17 / 12.23 |
| 10 | strip STEREO (1203, 1459) | 8.09 / 10.79 | 9.17 / 12.23 |
| 0 | strip STEREO (1976, 1457) | 8.09 / 10.79 | 9.17 / 12.23 |
| 25 | strip STEREO (1931, 1359) | 8.09 / 10.79 | 9.17 / 12.23 |
| 50 | strip STEREO (2019, 1296) | 8.09 / 10.79 | 9.17 / 12.23 |
| 75 | strip STEREO (2107, 1359) | 8.09 / 10.79 | 9.17 / 12.23 |
| 100 | strip STEREO (2064, 1461) | 8.09 / 10.79 | 9.17 / 12.23 |
| 0 | strip OUTPUT (2598, 1457) | 8.09 / 10.79 | 9.17 / 12.23 |
| 6 | strip OUTPUT (2558, 1360) | 8.09 / 10.79 | 9.17 / 12.23 |
| 12 | strip OUTPUT (2641, 1296) | 8.09 / 10.79 | 9.17 / 12.23 |
| 18 | strip OUTPUT (2729, 1359) | 8.09 / 10.79 | 9.17 / 12.23 |
| 24 | strip OUTPUT (2685, 1459) | 8.09 / 10.79 | 9.17 / 12.23 |
| –30 | strip OUTPUT (2596, 1685) | 8.09 / 10.79 | 9.17 / 12.23 |
| –20 | strip OUTPUT (2547, 1587) | 8.09 / 10.79 | 9.17 / 12.23 |
| –10 | strip OUTPUT (2630, 1520) | 8.09 / 10.79 | 9.17 / 12.23 |
| 0 | strip OUTPUT (2718, 1569) | 8.09 / 10.79 | 9.17 / 12.23 |
| +10 | strip OUTPUT (2706, 1673) | 8.09 / 10.79 | 9.17 / 12.23 |
| 0 | strip STEREO (1546, 1457) | 8.09 / 10.79 | 9.17 / 12.23 |
| 2 | strip STEREO (1503, 1382) | 8.09 / 10.79 | 9.17 / 12.23 |
| 4 | strip STEREO (1546, 1308) | 8.09 / 10.79 | 9.17 / 12.23 |
| 6 | strip STEREO (1632, 1308) | 8.09 / 10.79 | 9.17 / 12.23 |
| 8 | strip STEREO (1675, 1382) | 8.09 / 10.79 | 9.17 / 12.23 |
| 10 | strip STEREO (1633, 1459) | 8.09 / 10.79 | 9.17 / 12.23 |
| IDLE FOUNDRY | panel (2604, 92) | 6.47 / 8.63 | 6.47 / 8.63 |
| % (GATE and SWING) | arp drawer units | 8.63 / 11.51 | 9.71 / 12.95 |
| RATE: 2, 4, 8, 16, 32; OCTAVES: 1…5; GATE / SWING: 0, 25, 50, 75, 100 | arp drawer dials | 8.09 / 10.79 | 9.17 / 12.23 |
| LOADING / RESTARTING / CRASHED / NOT RESPONDING / MISSING / FAILED / OFF | FX slot small print | 8.63 / 11.51 | 9.71 / 12.95 |
| CUTOFF FREQUENCY: 1.2 kHz; DRAG TO RESIZE; FX description | tooltips | 6.56 / 8.87 | 8.21 / 11.08 |
| MIDI Learn notes and notifications | overlay notes | 6.56 / 8.87 | 8.21 / 11.08 |
| SETTINGS · CA-72 0.3.0 | settings menu | 6.27 / 8.36 | 7.84 / 10.45 |
| LOOK / LETTERING / FREQUENCY DIALS / WINDOW / MOD WHEEL SOURCE / STANDALONE / AGENTS | settings menu | 5.50 / 7.34 | 6.88 / 9.18 |
| REALISTIC / CLASSIC / STANDARD / LARGE / SEMITONES / HARDWARE / SIZE… / WHAT'S NEW… / CC 1… / AUDIO & MIDI… / ALLOW / OFF | settings menu | 7.55 / 10.20 | 9.44 / 12.75 |
| 75% / 100% / 125% / 150% / 200% / FIT TO SCREEN | size / control / MOD-source / FX context menus | 7.55 / 10.20 | 9.44 / 12.75 |
| MIDI LEARN / REMOVE MIDI ASSIGNMENT / MIDI ASSIGNMENTS…; all context-menu options | size / control / MOD-source / FX context menus | 7.55 / 10.20 | 9.44 / 12.75 |
| SIZE / control / MOD-source / FX menu titles | other menu titles | 6.27 / 8.36 | 7.84 / 10.45 |
| Typed knob unit | value-entry overlay | 5.28 / 7.14 | 6.61 / 8.92 |

## SIZE menu 100 % / unknown-screen fallback: 1380 × 788

| Element | Where (drawing x, y where supplied) | STANDARD x / cap px | LARGE x / cap px |
|---|---|---:|---:|
| –2 | panel tune (151, 454) | 3.69 / 4.92 | 4.18 / 5.58 |
| –1 | panel tune (151, 363) | 3.69 / 4.92 | 4.18 / 5.58 |
| 0 | panel tune (229, 322) | 3.69 / 4.92 | 4.18 / 5.58 |
| 1 | panel tune (303, 365) | 3.69 / 4.92 | 4.18 / 5.58 |
| 2 | panel tune (303, 451) | 3.69 / 4.92 | 4.18 / 5.58 |
| 0 | panel glide (83, 690) | 3.69 / 4.92 | 4.18 / 5.58 |
| 2 | panel glide (40, 615) | 3.69 / 4.92 | 4.18 / 5.58 |
| 4 | panel glide (83, 541) | 3.69 / 4.92 | 4.18 / 5.58 |
| 6 | panel glide (169, 541) | 3.69 / 4.92 | 4.18 / 5.58 |
| 8 | panel glide (212, 615) | 3.69 / 4.92 | 4.18 / 5.58 |
| 10 | panel glide (170, 692) | 3.69 / 4.92 | 4.18 / 5.58 |
| 0 | panel mod_mix (289, 690) | 3.69 / 4.92 | 4.18 / 5.58 |
| 2 | panel mod_mix (246, 615) | 3.69 / 4.92 | 4.18 / 5.58 |
| 4 | panel mod_mix (289, 541) | 3.69 / 4.92 | 4.18 / 5.58 |
| 6 | panel mod_mix (375, 541) | 3.69 / 4.92 | 4.18 / 5.58 |
| 8 | panel mod_mix (418, 615) | 3.69 / 4.92 | 4.18 / 5.58 |
| 10 | panel mod_mix (376, 692) | 3.69 / 4.92 | 4.18 / 5.58 |
| OSC. 3 | panel mod_mix (282, 715) | 3.44 / 4.59 | 3.94 / 5.25 |
| NOISE | panel osc3_control (381, 715) | 3.44 / 4.59 | 3.94 / 5.25 |
| LO | panel osc1_range (528, 289) | 3.69 / 4.92 | 4.18 / 5.58 |
| 32' | panel osc1_range (545, 239) | 3.69 / 4.92 | 4.18 / 5.58 |
| 16' | panel osc1_range (591, 213) | 3.69 / 4.92 | 4.18 / 5.58 |
| 8' | panel osc1_range (643, 213) | 3.69 / 4.92 | 4.18 / 5.58 |
| 4' | panel osc1_range (686, 241) | 3.69 / 4.92 | 4.18 / 5.58 |
| 2' | panel osc1_range (705, 289) | 3.69 / 4.92 | 4.18 / 5.58 |
| LO | panel osc2_range (528, 496) | 3.69 / 4.92 | 4.18 / 5.58 |
| 32' | panel osc2_range (545, 446) | 3.69 / 4.92 | 4.18 / 5.58 |
| 16' | panel osc2_range (591, 420) | 3.69 / 4.92 | 4.18 / 5.58 |
| 8' | panel osc2_range (643, 420) | 3.69 / 4.92 | 4.18 / 5.58 |
| 4' | panel osc2_range (686, 448) | 3.69 / 4.92 | 4.18 / 5.58 |
| 2' | panel osc2_range (705, 496) | 3.69 / 4.92 | 4.18 / 5.58 |
| LO | panel osc3_control (528, 702) | 3.69 / 4.92 | 4.18 / 5.58 |
| 32' | panel osc3_range (545, 652) | 3.69 / 4.92 | 4.18 / 5.58 |
| 16' | panel osc3_range (591, 626) | 3.69 / 4.92 | 4.18 / 5.58 |
| 8' | panel osc3_range (643, 626) | 3.69 / 4.92 | 4.18 / 5.58 |
| 4' | panel osc3_range (686, 654) | 3.69 / 4.92 | 4.18 / 5.58 |
| 2' | panel osc3_range (705, 702) | 3.69 / 4.92 | 4.18 / 5.58 |
| –7 | panel osc2_frequency (755, 586) | 3.69 / 4.92 | 4.18 / 5.58 |
| –5 | panel osc2_frequency (719, 526) | 3.69 / 4.92 | 4.18 / 5.58 |
| –3 | panel osc2_frequency (731, 457) | 3.69 / 4.92 | 4.18 / 5.58 |
| –1 | panel osc2_frequency (786, 415) | 3.69 / 4.92 | 4.18 / 5.58 |
| 1 | panel osc2_frequency (854, 416) | 3.69 / 4.92 | 4.18 / 5.58 |
| 3 | panel osc2_frequency (905, 459) | 3.69 / 4.92 | 4.18 / 5.58 |
| 5 | panel osc2_frequency (917, 525) | 3.69 / 4.92 | 4.18 / 5.58 |
| 7 | panel osc2_frequency (883, 583) | 3.69 / 4.92 | 4.18 / 5.58 |
| –7 | panel osc3_frequency (755, 794) | 3.69 / 4.92 | 4.18 / 5.58 |
| –5 | panel osc3_frequency (719, 734) | 3.69 / 4.92 | 4.18 / 5.58 |
| –3 | panel osc3_frequency (731, 665) | 3.69 / 4.92 | 4.18 / 5.58 |
| –1 | panel osc3_frequency (786, 623) | 3.69 / 4.92 | 4.18 / 5.58 |
| 1 | panel osc3_frequency (854, 624) | 3.69 / 4.92 | 4.18 / 5.58 |
| 3 | panel osc3_frequency (905, 667) | 3.69 / 4.92 | 4.18 / 5.58 |
| 5 | panel osc3_frequency (917, 733) | 3.69 / 4.92 | 4.18 / 5.58 |
| 7 | panel osc3_frequency (883, 791) | 3.69 / 4.92 | 4.18 / 5.58 |
| 0 | panel osc1_volume (1177, 380) | 3.69 / 4.92 | 4.18 / 5.58 |
| 2 | panel osc1_volume (1134, 305) | 3.69 / 4.92 | 4.18 / 5.58 |
| 4 | panel osc1_volume (1177, 231) | 3.69 / 4.92 | 4.18 / 5.58 |
| 6 | panel osc1_volume (1263, 231) | 3.69 / 4.92 | 4.18 / 5.58 |
| 8 | panel osc1_volume (1306, 305) | 3.69 / 4.92 | 4.18 / 5.58 |
| 10 | panel osc1_volume (1264, 382) | 3.69 / 4.92 | 4.18 / 5.58 |
| 0 | panel ext_volume (1578, 484) | 3.69 / 4.92 | 4.18 / 5.58 |
| 2 | panel ext_volume (1535, 409) | 3.69 / 4.92 | 4.18 / 5.58 |
| 4 | panel ext_volume (1578, 335) | 3.69 / 4.92 | 4.18 / 5.58 |
| 6 | panel ext_volume (1664, 335) | 3.69 / 4.92 | 4.18 / 5.58 |
| 8 | panel ext_volume (1707, 409) | 3.69 / 4.92 | 4.18 / 5.58 |
| 10 | panel ext_volume (1665, 486) | 3.69 / 4.92 | 4.18 / 5.58 |
| 0 | panel osc2_volume (1177, 587) | 3.69 / 4.92 | 4.18 / 5.58 |
| 2 | panel osc2_volume (1134, 512) | 3.69 / 4.92 | 4.18 / 5.58 |
| 4 | panel osc2_volume (1177, 438) | 3.69 / 4.92 | 4.18 / 5.58 |
| 6 | panel osc2_volume (1263, 438) | 3.69 / 4.92 | 4.18 / 5.58 |
| 8 | panel osc2_volume (1306, 512) | 3.69 / 4.92 | 4.18 / 5.58 |
| 10 | panel osc2_volume (1264, 589) | 3.69 / 4.92 | 4.18 / 5.58 |
| 0 | panel noise_volume (1578, 691) | 3.69 / 4.92 | 4.18 / 5.58 |
| 2 | panel noise_volume (1535, 616) | 3.69 / 4.92 | 4.18 / 5.58 |
| 4 | panel noise_volume (1578, 542) | 3.69 / 4.92 | 4.18 / 5.58 |
| 6 | panel noise_volume (1664, 542) | 3.69 / 4.92 | 4.18 / 5.58 |
| 8 | panel noise_type (1707, 616) | 3.69 / 4.92 | 4.18 / 5.58 |
| 10 | panel noise_volume (1665, 693) | 3.69 / 4.92 | 4.18 / 5.58 |
| 0 | panel osc3_volume (1177, 793) | 3.69 / 4.92 | 4.18 / 5.58 |
| 2 | panel osc3_volume (1134, 718) | 3.69 / 4.92 | 4.18 / 5.58 |
| 4 | panel osc3_volume (1177, 644) | 3.69 / 4.92 | 4.18 / 5.58 |
| 6 | panel osc3_volume (1263, 644) | 3.69 / 4.92 | 4.18 / 5.58 |
| 8 | panel osc3_volume (1306, 718) | 3.69 / 4.92 | 4.18 / 5.58 |
| 10 | panel osc3_volume (1264, 795) | 3.69 / 4.92 | 4.18 / 5.58 |
| –4 | panel cutoff (2003, 351) | 3.69 / 4.92 | 4.18 / 5.58 |
| –2 | panel cutoff (2003, 260) | 3.69 / 4.92 | 4.18 / 5.58 |
| 0 | panel cutoff (2081, 219) | 3.69 / 4.92 | 4.18 / 5.58 |
| 2 | panel cutoff (2155, 262) | 3.69 / 4.92 | 4.18 / 5.58 |
| 4 | panel cutoff (2155, 348) | 3.69 / 4.92 | 4.18 / 5.58 |
| 0 | panel emphasis (2238, 380) | 3.69 / 4.92 | 4.18 / 5.58 |
| 2 | panel emphasis (2195, 305) | 3.69 / 4.92 | 4.18 / 5.58 |
| 4 | panel emphasis (2238, 231) | 3.69 / 4.92 | 4.18 / 5.58 |
| 6 | panel emphasis (2324, 231) | 3.69 / 4.92 | 4.18 / 5.58 |
| 8 | panel emphasis (2367, 305) | 3.69 / 4.92 | 4.18 / 5.58 |
| 10 | panel emphasis (2325, 382) | 3.69 / 4.92 | 4.18 / 5.58 |
| 0 | panel contour_amount (2440, 380) | 3.69 / 4.92 | 4.18 / 5.58 |
| 2 | panel contour_amount (2397, 305) | 3.69 / 4.92 | 4.18 / 5.58 |
| 4 | panel contour_amount (2440, 231) | 3.69 / 4.92 | 4.18 / 5.58 |
| 6 | panel contour_amount (2526, 231) | 3.69 / 4.92 | 4.18 / 5.58 |
| 8 | panel contour_amount (2569, 305) | 3.69 / 4.92 | 4.18 / 5.58 |
| 10 | panel contour_amount (2527, 382) | 3.69 / 4.92 | 4.18 / 5.58 |
| 10 | panel filter_attack (2002, 539) | 3.69 / 4.92 | 4.18 / 5.58 |
| 200 | panel keyboard_control_1 (1984, 493) | 3.69 / 4.92 | 4.18 / 5.58 |
| 600 | panel filter_attack (2040, 424) | 3.69 / 4.92 | 4.18 / 5.58 |
| 1 | panel filter_attack (2120, 424) | 3.69 / 4.92 | 4.18 / 5.58 |
| 5 | panel filter_attack (2159, 460) | 3.69 / 4.92 | 4.18 / 5.58 |
| 10 | panel filter_attack (2166, 521) | 3.69 / 4.92 | 4.18 / 5.58 |
| M-SEC. | panel filter_attack (2019, 568) | 3.44 / 4.59 | 3.94 / 5.25 |
| SEC. | panel filter_attack (2130, 568) | 3.44 / 4.59 | 3.94 / 5.25 |
| 10 | panel filter_decay (2203, 539) | 3.69 / 4.92 | 4.18 / 5.58 |
| 200 | panel filter_decay (2185, 493) | 3.69 / 4.92 | 4.18 / 5.58 |
| 600 | panel filter_decay (2241, 424) | 3.69 / 4.92 | 4.18 / 5.58 |
| 1 | panel filter_decay (2321, 424) | 3.69 / 4.92 | 4.18 / 5.58 |
| 5 | panel filter_decay (2360, 460) | 3.69 / 4.92 | 4.18 / 5.58 |
| 10 | panel filter_decay (2367, 521) | 3.69 / 4.92 | 4.18 / 5.58 |
| M-SEC. | panel filter_decay (2220, 568) | 3.44 / 4.59 | 3.94 / 5.25 |
| SEC. | panel filter_decay (2331, 568) | 3.44 / 4.59 | 3.94 / 5.25 |
| 0 | panel filter_sustain (2440, 568) | 3.69 / 4.92 | 4.18 / 5.58 |
| 2 | panel filter_sustain (2397, 493) | 3.69 / 4.92 | 4.18 / 5.58 |
| 4 | panel filter_sustain (2440, 419) | 3.69 / 4.92 | 4.18 / 5.58 |
| 6 | panel filter_sustain (2526, 419) | 3.69 / 4.92 | 4.18 / 5.58 |
| 8 | panel filter_sustain (2569, 493) | 3.69 / 4.92 | 4.18 / 5.58 |
| 10 | panel filter_sustain (2527, 570) | 3.69 / 4.92 | 4.18 / 5.58 |
| 10 | panel loudness_attack (2002, 760) | 3.69 / 4.92 | 4.18 / 5.58 |
| 200 | panel loudness_attack (1984, 714) | 3.69 / 4.92 | 4.18 / 5.58 |
| 600 | panel loudness_attack (2040, 645) | 3.69 / 4.92 | 4.18 / 5.58 |
| 1 | panel loudness_attack (2120, 645) | 3.69 / 4.92 | 4.18 / 5.58 |
| 5 | panel loudness_attack (2159, 681) | 3.69 / 4.92 | 4.18 / 5.58 |
| 10 | panel loudness_attack (2166, 742) | 3.69 / 4.92 | 4.18 / 5.58 |
| M-SEC. | panel loudness_attack (2019, 789) | 3.44 / 4.59 | 3.94 / 5.25 |
| SEC. | panel loudness_attack (2130, 789) | 3.44 / 4.59 | 3.94 / 5.25 |
| 10 | panel loudness_decay (2203, 760) | 3.69 / 4.92 | 4.18 / 5.58 |
| 200 | panel loudness_decay (2185, 714) | 3.69 / 4.92 | 4.18 / 5.58 |
| 600 | panel loudness_decay (2241, 645) | 3.69 / 4.92 | 4.18 / 5.58 |
| 1 | panel loudness_decay (2321, 645) | 3.69 / 4.92 | 4.18 / 5.58 |
| 5 | panel loudness_decay (2360, 681) | 3.69 / 4.92 | 4.18 / 5.58 |
| 10 | panel loudness_decay (2367, 742) | 3.69 / 4.92 | 4.18 / 5.58 |
| M-SEC. | panel loudness_decay (2220, 789) | 3.44 / 4.59 | 3.94 / 5.25 |
| SEC. | panel loudness_decay (2331, 789) | 3.44 / 4.59 | 3.94 / 5.25 |
| 0 | panel loudness_sustain (2440, 789) | 3.69 / 4.92 | 4.18 / 5.58 |
| 2 | panel loudness_sustain (2397, 714) | 3.69 / 4.92 | 4.18 / 5.58 |
| 4 | panel loudness_sustain (2440, 640) | 3.69 / 4.92 | 4.18 / 5.58 |
| 6 | panel loudness_sustain (2526, 640) | 3.69 / 4.92 | 4.18 / 5.58 |
| 8 | panel loudness_sustain (2569, 714) | 3.69 / 4.92 | 4.18 / 5.58 |
| 10 | panel loudness_sustain (2527, 791) | 3.69 / 4.92 | 4.18 / 5.58 |
| 0 | panel volume (2637, 377) | 3.69 / 4.92 | 4.18 / 5.58 |
| 2 | panel volume (2594, 302) | 3.69 / 4.92 | 4.18 / 5.58 |
| 4 | panel volume (2637, 228) | 3.69 / 4.92 | 4.18 / 5.58 |
| 6 | panel volume (2723, 228) | 3.69 / 4.92 | 4.18 / 5.58 |
| 8 | panel volume (2766, 302) | 3.69 / 4.92 | 4.18 / 5.58 |
| 10 | panel volume (2724, 379) | 3.69 / 4.92 | 4.18 / 5.58 |
| 0 | panel feedback (2637, 790) | 3.69 / 4.92 | 4.18 / 5.58 |
| 2 | panel feedback (2594, 715) | 3.69 / 4.92 | 4.18 / 5.58 |
| 4 | panel feedback (2637, 641) | 3.69 / 4.92 | 4.18 / 5.58 |
| 6 | panel feedback (2723, 641) | 3.69 / 4.92 | 4.18 / 5.58 |
| 8 | panel feedback (2766, 715) | 3.69 / 4.92 | 4.18 / 5.58 |
| 10 | panel feedback (2724, 792) | 3.69 / 4.92 | 4.18 / 5.58 |
| TUNE | panel tune (229, 299) | 4.67 / 6.23 | 5.41 / 7.22 |
| OSCILLATOR | panel osc_mod (446, 349) | 4.67 / 6.23 | 5.41 / 7.22 |
| MODULATION | panel osc_mod (446, 369) | 4.67 / 6.23 | 5.41 / 7.22 |
| ON | panel osc_mod (492, 450) | 4.67 / 6.23 | 5.41 / 7.22 |
| GLIDE | panel glide (128, 518) | 4.67 / 6.23 | 5.41 / 7.22 |
| MODULATION MIX | panel mod_mix (332, 518) | 4.67 / 6.23 | 5.41 / 7.22 |
| RANGE | panel osc1_range (618, 194) | 4.67 / 6.23 | 5.41 / 7.22 |
| OSCILLATOR –1 | panel osc1_waveform (821, 194) | 4.67 / 6.23 | 5.41 / 7.22 |
| FREQUENCY | panel osc1_waveform (821, 218) | 4.67 / 6.23 | 5.41 / 7.22 |
| WAVEFORM | panel osc1_waveform (1022, 194) | 4.67 / 6.23 | 5.41 / 7.22 |
| OSCILLATOR –2 | panel osc2_frequency (822, 397) | 4.67 / 6.23 | 5.41 / 7.22 |
| OSCILLATOR –3 | panel osc3_frequency (822, 608) | 4.67 / 6.23 | 5.41 / 7.22 |
| OSC. 3 | panel osc3_control (488, 626) | 3.94 / 5.25 | 3.94 / 5.25 |
| CONTROL | panel osc3_control (488, 645) | 3.94 / 5.25 | 3.94 / 5.25 |
| VOLUME | panel osc1_volume (1221, 196) | 4.67 / 6.23 | 5.41 / 7.22 |
| ON | panel osc1_on (1467, 347) | 4.67 / 6.23 | 5.41 / 7.22 |
| ON | panel ext_on (1467, 451) | 4.67 / 6.23 | 5.41 / 7.22 |
| ON | panel osc2_on (1467, 554) | 4.67 / 6.23 | 5.41 / 7.22 |
| ON | panel noise_on (1467, 658) | 4.67 / 6.23 | 5.41 / 7.22 |
| ON | panel osc3_on (1467, 760) | 4.67 / 6.23 | 5.41 / 7.22 |
| EXTERNAL | panel ext_volume (1620, 281) | 4.67 / 6.23 | 5.41 / 7.22 |
| INPUT VOLUME | panel ext_volume (1622, 301) | 4.67 / 6.23 | 5.41 / 7.22 |
| NOISE VOLUME | panel noise_volume (1622, 522) | 4.67 / 6.23 | 5.41 / 7.22 |
| OVERLOAD | panel ext_volume (1763, 363) | 4.67 / 6.23 | 5.41 / 7.22 |
| WHITE | panel noise_type (1764, 544) | 4.67 / 6.23 | 5.41 / 7.22 |
| PINK | panel noise_type (1764, 695) | 4.67 / 6.23 | 5.41 / 7.22 |
| FILTER | panel filter_mode (1910, 251) | 4.67 / 6.23 | 5.41 / 7.22 |
| MODE | panel filter_mode (1910, 271) | 4.67 / 6.23 | 5.41 / 7.22 |
| LO | panel filter_mode (1860, 344) | 4.67 / 6.23 | 5.41 / 7.22 |
| HI | panel filter_mode (1954, 344) | 4.67 / 6.23 | 5.41 / 7.22 |
| FILTER | panel filter_mod (1910, 373) | 4.67 / 6.23 | 5.41 / 7.22 |
| MODULATION | panel filter_mod (1910, 393) | 4.67 / 6.23 | 5.41 / 7.22 |
| ON | panel filter_mod (1954, 466) | 4.67 / 6.23 | 5.41 / 7.22 |
| ON | panel keyboard_control_1 (1954, 487) | 4.67 / 6.23 | 5.41 / 7.22 |
| 1 | panel keyboard_control_1 (1837, 522) | 4.67 / 6.23 | 5.41 / 7.22 |
| KEYBOARD | panel keyboard_control_2 (1910, 568) | 4.67 / 6.23 | 5.41 / 7.22 |
| CONTROL | panel keyboard_control_2 (1910, 585) | 4.67 / 6.23 | 5.41 / 7.22 |
| 2 | panel keyboard_control_2 (1837, 622) | 4.67 / 6.23 | 5.41 / 7.22 |
| ON | panel keyboard_control_2 (1954, 660) | 4.67 / 6.23 | 5.41 / 7.22 |
| FILTER | panel emphasis (2282, 172) | 7.63 / 10.17 | 7.63 / 10.17 |
| CUTOFF FREQUENCY | panel cutoff (2081, 192) | 4.67 / 6.23 | 5.41 / 7.22 |
| EMPHASIS | panel emphasis (2280, 192) | 4.67 / 6.23 | 5.41 / 7.22 |
| AMOUNT | panel contour_amount (2482, 169) | 4.67 / 6.23 | 5.41 / 7.22 |
| OF CONTOUR | panel contour_amount (2482, 191) | 4.67 / 6.23 | 5.41 / 7.22 |
| ATTACK TIME | panel filter_attack (2081, 403) | 4.67 / 6.23 | 5.41 / 7.22 |
| DECAY TIME | panel filter_decay (2281, 403) | 4.67 / 6.23 | 5.41 / 7.22 |
| SUSTAIN LEVEL | panel filter_sustain (2483, 403) | 4.67 / 6.23 | 5.41 / 7.22 |
| ATTACK TIME | panel loudness_attack (2081, 623) | 4.67 / 6.23 | 5.41 / 7.22 |
| DECAY TIME | panel loudness_decay (2281, 623) | 4.67 / 6.23 | 5.41 / 7.22 |
| SUSTAIN LEVEL | panel loudness_sustain (2483, 623) | 4.67 / 6.23 | 5.41 / 7.22 |
| LOUDNESS CONTOUR | panel loudness_decay (2284, 602) | 6.64 / 8.86 | 6.64 / 8.86 |
| VOLUME | panel volume (2681, 188) | 4.67 / 6.23 | 5.41 / 7.22 |
| MAIN OUTPUT | panel main_output (2854, 188) | 4.67 / 6.23 | 5.41 / 7.22 |
| ON | panel main_output (2902, 337) | 4.67 / 6.23 | 5.41 / 7.22 |
| A–440 | panel a440 (2682, 473) | 4.67 / 6.23 | 5.41 / 7.22 |
| ON | panel a440 (2728, 547) | 4.67 / 6.23 | 5.41 / 7.22 |
| FEEDBACK | panel feedback (2682, 622) | 4.67 / 6.23 | 5.41 / 7.22 |
| PHONES | panel feedback (2856, 622) | 4.67 / 6.23 | 5.41 / 7.22 |
| QUALITY | panel quality (3023, 305) | 4.67 / 6.23 | 5.41 / 7.22 |
| ULTRA | panel quality (3023, 351) | 3.69 / 4.92 | 3.69 / 4.92 |
| HI | panel quality (2983, 401) | 3.69 / 4.92 | 3.69 / 4.92 |
| LO | panel quality (3023, 453) | 3.69 / 4.92 | 3.69 / 4.92 |
| POWER | panel quality (3023, 602) | 4.67 / 6.23 | 5.41 / 7.22 |
| ON | panel quality (3023, 637) | 4.67 / 6.23 | 5.41 / 7.22 |
| GLIDE | strip left hand (160, 1275) | 4.67 / 6.23 | 5.41 / 7.22 |
| DECAY | strip left hand (160, 1355) | 4.67 / 6.23 | 5.41 / 7.22 |
| ON | strip left hand (292, 1307) | 3.69 / 4.92 | 4.18 / 5.58 |
| ON | strip left hand (292, 1387) | 3.69 / 4.92 | 4.18 / 5.58 |
| PRIORITY | strip left hand (150, 1105) | 4.67 / 6.23 | 5.41 / 7.22 |
| RETRIGGER | strip left hand (146, 1195) | 4.67 / 6.23 | 5.41 / 7.22 |
| ON | strip left hand (292, 1227) | 3.69 / 4.92 | 4.18 / 5.58 |
| LAST | strip left hand (256, 1070) | 3.69 / 4.92 | 4.18 / 5.58 |
| LOW | strip left hand (216, 1141) | 3.69 / 4.92 | 4.18 / 5.58 |
| HIGH | strip left hand (296, 1141) | 3.69 / 4.92 | 4.18 / 5.58 |
| PITCH | strip left hand (124, 1694) | 4.67 / 6.23 | 5.41 / 7.22 |
| MOD. | strip left hand (246, 1694) | 4.67 / 6.23 | 5.41 / 7.22 |
| MODE | strip VOICES (673, 1086) | 5.17 / 6.89 | 5.66 / 7.55 |
| VOICES PLAYED AS | strip STEREO (1418, 1086) | 5.17 / 6.89 | 5.66 / 7.55 |
| PLACEMENT | strip STEREO (1928, 1086) | 5.17 / 6.89 | 5.66 / 7.55 |
| AUTO GAIN | strip OUTPUT (2641, 1086) | 5.17 / 6.89 | 5.66 / 7.55 |
| VOICES | strip VOICES (589, 1268) | 5.17 / 6.89 | 5.66 / 7.55 |
| ENTROPY | strip VOICES (589, 1492) | 5.17 / 6.89 | 5.66 / 7.55 |
| WIDTH | strip STEREO (1159, 1268) | 5.17 / 6.89 | 5.66 / 7.55 |
| DETUNE | strip STEREO (2019, 1268) | 5.17 / 6.89 | 5.66 / 7.55 |
| DRIVE | strip OUTPUT (2641, 1268) | 5.17 / 6.89 | 5.66 / 7.55 |
| LEVEL | strip OUTPUT (2641, 1492) | 5.17 / 6.89 | 5.66 / 7.55 |
| INNER | strip STEREO (1589, 1268) | 5.17 / 6.89 | 5.66 / 7.55 |
| dB | strip OUTPUT (2935, 1153) | 3.94 / 5.25 | 4.43 / 5.91 |
| dB | strip OUTPUT (2935, 1377) | 3.94 / 5.25 | 4.43 / 5.91 |
| dB | strip OUTPUT (2935, 1601) | 3.94 / 5.25 | 4.43 / 5.91 |
| L | strip STEREO (1037, 1601) | 3.94 / 5.25 | 4.43 / 5.91 |
| R | strip STEREO (2309, 1601) | 3.94 / 5.25 | 4.43 / 5.91 |
| WHERE THE VOICES SOUND | strip STEREO (1673, 1507) | 5.17 / 6.89 | 5.66 / 7.55 |
| 2 | strip VOICES (546, 1457) | 3.69 / 4.92 | 4.18 / 5.58 |
| 3 | strip VOICES (510, 1415) | 3.69 / 4.92 | 4.18 / 5.58 |
| 4 | strip VOICES (506, 1360) | 3.69 / 4.92 | 4.18 / 5.58 |
| 5 | strip VOICES (537, 1314) | 3.69 / 4.92 | 4.18 / 5.58 |
| 6 | strip VOICES (589, 1296) | 3.69 / 4.92 | 4.18 / 5.58 |
| 7 | strip VOICES (641, 1314) | 3.69 / 4.92 | 4.18 / 5.58 |
| 8 | strip VOICES (672, 1360) | 3.69 / 4.92 | 4.18 / 5.58 |
| 9 | strip VOICES (668, 1415) | 3.69 / 4.92 | 4.18 / 5.58 |
| 10 | strip VOICES (633, 1459) | 3.69 / 4.92 | 4.18 / 5.58 |
| 0 | strip VOICES (546, 1681) | 3.69 / 4.92 | 4.18 / 5.58 |
| 2 | strip VOICES (503, 1606) | 3.69 / 4.92 | 4.18 / 5.58 |
| 4 | strip VOICES (546, 1532) | 3.69 / 4.92 | 4.18 / 5.58 |
| 6 | strip VOICES (632, 1532) | 3.69 / 4.92 | 4.18 / 5.58 |
| 8 | strip VOICES (675, 1606) | 3.69 / 4.92 | 4.18 / 5.58 |
| 10 | strip VOICES (633, 1683) | 3.69 / 4.92 | 4.18 / 5.58 |
| 0 | strip STEREO (1116, 1457) | 3.69 / 4.92 | 4.18 / 5.58 |
| 2 | strip STEREO (1073, 1382) | 3.69 / 4.92 | 4.18 / 5.58 |
| 4 | strip STEREO (1116, 1308) | 3.69 / 4.92 | 4.18 / 5.58 |
| 6 | strip STEREO (1202, 1308) | 3.69 / 4.92 | 4.18 / 5.58 |
| 8 | strip STEREO (1245, 1382) | 3.69 / 4.92 | 4.18 / 5.58 |
| 10 | strip STEREO (1203, 1459) | 3.69 / 4.92 | 4.18 / 5.58 |
| 0 | strip STEREO (1976, 1457) | 3.69 / 4.92 | 4.18 / 5.58 |
| 25 | strip STEREO (1931, 1359) | 3.69 / 4.92 | 4.18 / 5.58 |
| 50 | strip STEREO (2019, 1296) | 3.69 / 4.92 | 4.18 / 5.58 |
| 75 | strip STEREO (2107, 1359) | 3.69 / 4.92 | 4.18 / 5.58 |
| 100 | strip STEREO (2064, 1461) | 3.69 / 4.92 | 4.18 / 5.58 |
| 0 | strip OUTPUT (2598, 1457) | 3.69 / 4.92 | 4.18 / 5.58 |
| 6 | strip OUTPUT (2558, 1360) | 3.69 / 4.92 | 4.18 / 5.58 |
| 12 | strip OUTPUT (2641, 1296) | 3.69 / 4.92 | 4.18 / 5.58 |
| 18 | strip OUTPUT (2729, 1359) | 3.69 / 4.92 | 4.18 / 5.58 |
| 24 | strip OUTPUT (2685, 1459) | 3.69 / 4.92 | 4.18 / 5.58 |
| –30 | strip OUTPUT (2596, 1685) | 3.69 / 4.92 | 4.18 / 5.58 |
| –20 | strip OUTPUT (2547, 1587) | 3.69 / 4.92 | 4.18 / 5.58 |
| –10 | strip OUTPUT (2630, 1520) | 3.69 / 4.92 | 4.18 / 5.58 |
| 0 | strip OUTPUT (2718, 1569) | 3.69 / 4.92 | 4.18 / 5.58 |
| +10 | strip OUTPUT (2706, 1673) | 3.69 / 4.92 | 4.18 / 5.58 |
| 0 | strip STEREO (1546, 1457) | 3.69 / 4.92 | 4.18 / 5.58 |
| 2 | strip STEREO (1503, 1382) | 3.69 / 4.92 | 4.18 / 5.58 |
| 4 | strip STEREO (1546, 1308) | 3.69 / 4.92 | 4.18 / 5.58 |
| 6 | strip STEREO (1632, 1308) | 3.69 / 4.92 | 4.18 / 5.58 |
| 8 | strip STEREO (1675, 1382) | 3.69 / 4.92 | 4.18 / 5.58 |
| 10 | strip STEREO (1633, 1459) | 3.69 / 4.92 | 4.18 / 5.58 |
| IDLE FOUNDRY | panel (2604, 92) | 2.95 / 3.94 | 2.95 / 3.94 |
| SAVE / PANEL / ARP / FX | preset rail | 4.92 / 6.56 | 4.92 / 6.56 |
| MONO / POLY / UNISON; SCATTER / DOUBLE; EVEN / CENTER; ON | strip tab caps | 4.18 / 5.58 | 4.18 / 5.58 |
| OCTAVE | keyboard header | 6.80 / 9.19 | 6.80 / 9.19 |
| − / + / TYPING / PANIC | keyboard header | 4.92 / 6.56 | 4.92 / 6.56 |
| A = C3 · Z / X OCTAVE | keyboard header | 4.37 / 5.91 | 4.37 / 5.91 |
| C2 / C3 / C4 / C5 | CLASSIC keyboard keys | 5.34 / 7.22 | 5.34 / 7.22 |
| FIND / PRESETS / PRESET | preset drawer headings | 6.40 / 8.53 | 6.40 / 8.53 |
| RENAME | preset drawer action key | 6.89 / 9.19 | 6.89 / 9.19 |
| TAGS | preset drawer action key | 6.89 / 9.19 | 6.89 / 9.19 |
| DELETE | preset drawer action key | 6.89 / 9.19 | 6.89 / 9.19 |
| REVERT | preset drawer action key | 6.89 / 9.19 | 6.89 / 9.19 |
| SAVE AS | preset drawer action key | 6.89 / 9.19 | 6.89 / 9.19 |
| RESTORE | preset drawer action key | 6.89 / 9.19 | 6.89 / 9.19 |
| MIDI LEARN | preset drawer action key | 6.65 / 8.87 | 6.65 / 8.87 |
| CLOSE | preset drawer action key | 6.89 / 9.19 | 6.89 / 9.19 |
| INIT | preset drawer action key | 6.89 / 9.19 | 6.89 / 9.19 |
| ALL / FAVORITES / MINE | preset drawer PRESETS tabs | 8.70 / 12.25 | 8.70 / 12.25 |
| Preset names | preset drawer PRESETS rows | 8.70 / 12.25 | 8.70 / 12.25 |
| YOURS / EDITED; preset tags | preset drawer PRESETS rows | 8.70 / 12.25 | 8.70 / 12.25 |
| Available tags | preset drawer FIND tags | 8.70 / 12.25 | 8.70 / 12.25 |
| Current name, origin, tags | preset drawer PRESET status | 8.70 / 12.25 | 8.70 / 12.25 |
| Rename / Save / Tags fields and prompts | preset drawer PRESET editing | 8.70 / 12.25 | 8.70 / 12.25 |
| Update / version / download / status messages | preset drawer PRESET footer | 8.70 / 12.25 | 8.70 / 12.25 |
| MIDI control names and assignments | preset drawer PRESETS MIDI list | 8.70 / 12.25 | 8.70 / 12.25 |
| MIDI keys, reserved CCs, status and actions | preset drawer FIND / PRESET MIDI list | 8.70 / 12.25 | 8.70 / 12.25 |
| ARP / DIRECTION; RATE / OCTAVES / GATE / SWING | arp drawer legends | 5.17 / 6.89 | 5.66 / 7.55 |
| ON / HOLD; UP / DOWN / UP/DOWN / RANDOM / ORDER | arp drawer tab caps | 4.18 / 5.58 | 4.18 / 5.58 |
| % (GATE and SWING) | arp drawer units | 3.94 / 5.25 | 4.43 / 5.91 |
| RATE: 2, 4, 8, 16, 32; OCTAVES: 1…5; GATE / SWING: 0, 25, 50, 75, 100 | arp drawer dials | 3.69 / 4.92 | 4.18 / 5.58 |
| SLOT 1 / SLOT 2 / SLOT 3 / SLOT 4 | FX drawer legends | 5.17 / 6.89 | 5.66 / 7.55 |
| ON (four tabs) | FX drawer tab caps | 4.18 / 5.58 | 4.18 / 5.58 |
| LOADING / RESTARTING / CRASHED / NOT RESPONDING / MISSING / FAILED / OFF | FX slot small print | 3.94 / 5.25 | 4.43 / 5.91 |
| ADD TO SLOT n / REPLACE SLOT n; SCANNING… / effect count; SCAN AGAIN | FX list title and action | 8.38 / 11.94 | 8.38 / 11.94 |
| Effect names and makers | FX list rows | 8.38 / 11.94 | 8.38 / 11.94 |
| VST3 / CLAP; arrows | FX list format | 8.38 / 11.94 | 8.38 / 11.94 |
| CUTOFF FREQUENCY: 1.2 kHz; DRAG TO RESIZE; FX description | tooltips | 6.56 / 8.87 | 8.21 / 11.08 |
| MIDI Learn notes and notifications | overlay notes | 6.56 / 8.87 | 8.21 / 11.08 |
| SETTINGS · CA-72 0.3.0 | settings menu | 6.27 / 8.36 | 7.84 / 10.45 |
| LOOK / LETTERING / FREQUENCY DIALS / WINDOW / MOD WHEEL SOURCE / STANDALONE / AGENTS | settings menu | 5.50 / 7.34 | 6.88 / 9.18 |
| REALISTIC / CLASSIC / STANDARD / LARGE / SEMITONES / HARDWARE / SIZE… / WHAT'S NEW… / CC 1… / AUDIO & MIDI… / ALLOW / OFF | settings menu | 7.55 / 10.20 | 9.44 / 12.75 |
| 75% / 100% / 125% / 150% / 200% / FIT TO SCREEN | size / control / MOD-source / FX context menus | 7.55 / 10.20 | 9.44 / 12.75 |
| MIDI LEARN / REMOVE MIDI ASSIGNMENT / MIDI ASSIGNMENTS…; all context-menu options | size / control / MOD-source / FX context menus | 7.55 / 10.20 | 9.44 / 12.75 |
| SIZE / control / MOD-source / FX menu titles | other menu titles | 6.27 / 8.36 | 7.84 / 10.45 |
| Typed knob unit | value-entry overlay | 5.28 / 7.14 | 6.61 / 8.92 |
| Installed version and coverage | What’s New dialog | 7.53 / 10.17 | 9.41 / 12.71 |
| Release-note section headings | What’s New dialog | 7.29 / 9.84 | 9.11 / 12.30 |
| Release-note text | What’s New dialog | 8.01 / 10.83 | 10.02 / 13.54 |
| Shown once… footer | What’s New dialog | 6.56 / 8.86 | 8.20 / 11.07 |
| GOT IT | What’s New dialog | 8.50 / 11.48 | 10.63 / 14.36 |
