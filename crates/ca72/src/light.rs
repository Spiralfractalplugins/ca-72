//! Potato mode's voice (decisions.md R31): the instrument's panel played by a model far
//! cheaper than the circuit's, for computers the circuit is too heavy for. Not derived from
//! the circuit but fitted to the circuit model's measured behaviour ([`laws`]; the plug-in's
//! `tests/potato_reference.rs` measures both): band-limited oscillators (BLEP and BLAMP) at the
//! circuit's levels and shapes, the mixer's channels through their loaded pots, the input
//! pair's compression, one ladder of four one-pole stages solved without delay around the
//! loop (its gain the emphasis, its own limiter setting how loud it rings), FILTER MODE HI
//! (the bus less the filter), the plug-in's DRIVE, contours of RC segments with the circuit's
//! time laws and trigger, and the VCA's measured law. The audio
//! path runs in single precision, the filter at eight times the voice's rate; what the panel and the contours set
//! (the oscillators' increments, the ladder's coefficient) is worked out every
//! [`CONTROL_EVERY`] samples and followed between, FILTER MODULATION's line through the cutoff's
//! law at twice the voice's rate (decisions.md R-LO-FILTER-FM).
//!
//! The keys are the instrument's: lowest-note priority (or NOTE PRIORITY's HIGH or LAST, as the
//! circuit's voice has them: [`crate::voice::Priority`]), single triggering (a key pressed while
//! another is held does not start the contours again), the contours re-armed once the trigger
//! contact has been open 11.5 ms; GLIDE moves the pitch as the circuit's hold does. Nothing here
//! allocates once made.

use crate::resample::{BlepTables, Decimator, Interpolator};
use crate::tuning::FREQ_CENTRE;
use crate::vco::Timed;
#[cfg(test)]
use crate::voice::OscPanel;
use crate::voice::{
    ContourKnobs, FILTER_ATTACK, FILTER_DECAY, LOUDNESS_ATTACK, LOUDNESS_DECAY, Panel, Priority,
    Waveform, contour_amount_track, cutoff_track, emphasis_r14, ext_taper, glide_r, sustain_track,
    time_pot, volume_track,
};

/// Samples between the control laws' updates.
pub const CONTROL_EVERY: usize = 16;

/// The contours' past outputs kept, samples (more than [`Light`]'s filter's delay).
const LATE_PAST: usize = 64;

/// Keep the nonlinear ladder's ultrasonic harmonics out of the output band.
const FILTER_OVERSAMPLE: usize = 8;

/// FILTER MODULATION's line against the voice's rate, where it meets the cutoff's law.
const LINE_OVERSAMPLE: usize = 2;

/// The laws, fitted to the circuit model's measurements at 48 kHz (`tests/potato_reference.rs`;
/// decisions.md R31), as calibrated against the hardware reference (R42 on): where the circuit
/// reads a knob through the reference's law (voice.rs's `volume_track`, `time_pot` and the
/// rest), the light voice reads it through the same law.
pub mod laws {
    /// Key 69 on 8' at TUNE's centre, Hz (the circuit's: 439.78, within 1.4 cents of equal
    /// temperament from key 24 to 108).
    pub const A4: f64 = 440.0;
    /// FREQUENCY (oscillators 2 and 3) from its centre, cents: a cubic in the pot's track
    /// (the panel's FREQUENCY is the pot's wiper: `freq_dial`), within 1.2 cents of the
    /// measured.
    pub const OSC2_CENTS: [f64; 4] = [-855.506, 1753.088, -94.505, 19.578];
    pub const OSC3_CENTS: [f64; 4] = [-862.370, 1806.991, -206.463, 83.185];
    /// TUNE at either end, cents.
    pub const TUNE_CENTS: f64 = 267.44;
    /// Oscillator 3 with OSC. 3 CONTROL off: as this key held on 8' (201.5 Hz) at FREQUENCY's
    /// centre; FREQUENCY then, from its centre, cents, a quintic in the pot's track (its
    /// summing node without the keys' input: 69 semitones end to end); TUNE moves it not at
    /// all.
    pub const OSC3_FREE_KEY: f64 = 55.479;
    pub const OSC3_FREE_CENTS: [f64; 6] = [-3797.431, 9228.8, -4699.331, 3541.6, -1559.71, 402.07];

    /// Each waveform's level against the sawtooth's (the bus's unit: a sawtooth at VOLUME 10
    /// is 1 at its peak), its channel's source resistance (ohm, for the pot's loading) and the
    /// rectangles' duty cycles.
    pub const TRIANGLE: f32 = 0.89530206;
    pub const SAWTOOTH: f32 = 1.0;
    pub const REVERSE_SAWTOOTH: f32 = 1.0;
    // Reduced from the full voice MIX at220Hz, VOLUME8, with the same physical
    // comparator component set. Frequency-dependent pulse-edge residuals remain (LO).
    pub const SQUARE: f32 = 1.056762;
    pub const WIDE: f32 = 1.056852;
    pub const NARROW: f32 = 1.056932;
    pub const DUTY: [f32; 3] = match crate::pulse::ACTIVE_PROFILE {
        crate::pulse::PulseProfile::Factory => [0.48447033, 0.30409349, 0.18917401],
        crate::pulse::PulseProfile::ReferenceStandard => [0.48181775, 0.29378574, 0.17421979],
    };
    /// Source mixing uses the same physical R030/R031 network as the circuit voice.
    pub const SHARK_TRIANGLE: f32 = TRIANGLE * SHARK_TRIANGLE_WEIGHT;
    pub const SHARK_SAWTOOTH: f32 = (1.0 - SHARK_TRIANGLE_WEIGHT) * SAWTOOTH;
    pub const SHARK_TRIANGLE_WEIGHT: f32 = ((4300.0 * 4700.0 / 9000.0
        + crate::voice::SHARK_SAW_OHMS)
        / (4300.0 * 4700.0 / 9000.0
            + crate::voice::SHARK_SAW_OHMS
            + 750.0 * 8200.0 / 8950.0
            + crate::voice::SHARK_TRIANGLE_OHMS)) as f32;
    pub const SOURCE_OHMS: [f64; 4] = [
        687.0,
        2246.0,
        3500.0,
        1.0 / (1.0 / (4300.0 * 4700.0 / 9000.0 + crate::voice::SHARK_SAW_OHMS)
            + 1.0 / (750.0 * 8200.0 / 8950.0 + crate::voice::SHARK_TRIANGLE_OHMS)),
    ];
    /// Oscillator 3's channel against the others' (its sawtooth through the reverse
    /// sawtooth's stage).
    pub const OSC3_LEVEL: f32 = 0.914;
    /// The high-passes the output shows (the waveforms' droop), Hz: the filter's output
    /// coupling (C5/C1, which FILTER MODE HI's direct branch does not pass through) and the
    /// rest after the VCA; together within 0.1 dB of the circuit's at 7 to 28 Hz.
    pub const COUPLING_HZ: f64 = 13.0;
    pub const DROOP_HZ: f64 = 4.4;
    /// FILTER MODE HI (decisions.md R46): the bus less the filter's output, the bus at the
    /// filter's own passband at EMPHASIS 0 (the open ladder's, times this) and through its
    /// own coupling, Hz (vcf.rs's `MODE_HZ`).
    pub const MODE_GAIN: f32 = 1.0;
    pub const MODE_HZ: f64 = 3.0;
    /// The VCA's control feeding through to the output, its thump as the loudness contour
    /// moves: the VCA's gain through a high-pass and a low-pass (time constants, s), times
    /// this (fitted to the circuit's output with nothing sounding, four contours at once,
    /// within 14 dB of the thump; most of it under 5 Hz).
    pub const THUMP: f32 = 0.0385;
    pub const THUMP_TIMES: (f64, f64) = (0.11, 0.035);

    /// The noise: white's and pink's level (on their generators' units), their channels'
    /// source resistances (pink's through R50), and each one's band (a high-pass and a
    /// low-pass, Hz).
    pub const WHITE: f32 = 0.600;
    pub const PINK: f32 = 0.440;
    pub const NOISE_OHMS: (f64, f64) = (700.0, 26.2e3);
    pub const WHITE_BAND: (f64, f64) = (60.0, 10_000.0);
    pub const PINK_BAND: (f64, f64) = (40.0, 6_500.0);
    /// The white's low-pass at each of `NOISE_TOP_AT`'s rates, Hz: the fit at 48 kHz takes in
    /// the circuit's own top near Nyquist; at 88.2 kHz and over its noise reaches higher.
    pub const NOISE_TOP_AT: [f64; 2] = [48_000.0, 88_200.0];
    pub const WHITE_TOP: [f64; 2] = [10_000.0, 19_000.0];
    pub const PINK_TOP: [f64; 2] = [6_500.0, 9_000.0];

    /// The ladder's input pair, a tanh on the input less the loop's feedback: the bus in its
    /// units (two and three sawtooths at full VOLUME compress 0.6 and 1.1 dB, the circuit's
    /// 0.7 and 1.2), off its centre by `BIAS` (with the triangle's own, its second harmonic
    /// from -37 dB at VOLUME 1 to -32 dB at 10, as the circuit's).
    pub const DRIVE: f32 = 0.69;
    pub const BIAS: f32 = 0.10;
    /// The loop's gain in `LOOP` (fitted with the pair off its centre by 0.28) times this:
    /// the same gain through the pair's slope at `BIAS`.
    pub const LOOP_SCALE: f64 = 0.941_52;
    /// The plug-in's DRIVE at gain G: the pair's limit (its tanh's) lowered to G to this
    /// power, its slope kept: the circuit's ladder, its every stage a pair, holds a driven
    /// input lower than one pair would (0.88 of it at 24 dB; the level so within 0.75 dB of
    /// the circuit's from 0 to 24 dB on six patches).
    pub const DRIVE_CEILING: f64 = -0.045;
    /// The output from the ladder's (at the VCA's gain 1: the loudness contour at 4.455 V).
    pub const OUT: f32 = 0.613;

    /// CUTOFF: log2 of the self-oscillating ladder's pitch, Hz, at the knob's 0, 0.05, ...,
    /// 1 (measured from 0.15 to 0.9; below, continued at the slope there in the pot's track,
    /// voice.rs's `cutoff_track`; above, past Nyquist, continued).
    pub const CUTOFF_LOG2: [f32; 21] = [
        5.2, 5.5, 5.81, 6.345, 6.787, 7.257, 7.745, 8.306, 8.878, 9.459, 10.045, 10.626, 11.208,
        11.789, 12.366, 12.874, 13.375, 13.866, 14.229, 14.42, 14.55,
    ];
    /// Above the fitted dial's top, Q28 saturates instead of continuing exponentially.
    /// Relative pitch from the circuit's converter and ladder: CUTOFF at 10, CUT CV
    /// advanced by 0, 0.5, 1, 1.5, 2, 3, 4, 6 and 10 octaves (R-FILTER-CONTOUR).
    pub const CUTOFF_SAT_AT: [f64; 9] = [
        14.55, 15.05, 15.55, 16.05, 16.55, 17.55, 18.55, 20.55, 24.55,
    ];
    pub const CUTOFF_SAT: [f64; 9] = [
        14.55,
        14.76425192,
        14.79149723,
        14.80223027,
        14.80798777,
        14.81368897,
        14.81613421,
        14.81781641,
        14.81830415,
    ];
    /// KEYBOARD CONTROL 1, 2 and both: octaves of the cutoff an octave of the keys, from C
    /// (36, where the keyboard's voltage is 0).
    pub const KEYBOARD_CONTROL: [f64; 3] = [0.3276, 0.6486, 0.9555];
    pub const TRACKING_FROM: f64 = 36.0;
    /// EMPHASIS: the loop's gain (4 rings; times `LOOP_SCALE`) at these positions of the knob
    /// drawn with the generic taper (below), for cutoffs of 60, 87, 228, 912 and 3731 Hz (log2
    /// below; between them interpolated, beyond them held): fitted to the drawn circuit's
    /// responses to noise at EMPHASIS up to 7 (decisions.md R31), within 0.4 dB of the
    /// calibrated circuit's passbands at 5 and 7 and 1 dB of its peaks at 5, 6 and 7 below its
    /// threshold. Past the threshold the gain follows R14's law, 11.84 / (R14 + 1.58K) (R14
    /// in K), to 7.5 at 10, scaled by `RING_GAIN`; at 10 it rings down to about 81 Hz and not
    /// below, as the circuit's.
    pub const EMPHASIS_AT: [f64; 11] = [
        0.0, 0.25, 0.5, 0.7, 0.72, 0.735, 0.744, 0.79, 0.85, 0.9, 1.0,
    ];
    pub const LOOP_AT_LOG2: [f64; 10] = [
        5.9, 6.44, 6.864, 7.783, 8.75, 9.745, 10.749, 11.752, 12.747, 13.711,
    ];
    pub const LOOP: [[f64; 11]; 10] = [
        [
            0.146, 0.502, 1.08, 1.95, 2.06, 2.14, 2.2, 2.45, 2.8, 3.2, 3.8,
        ],
        [
            0.154, 0.528, 1.2, 2.3, 2.45, 2.56, 2.65, 3.05, 3.55, 3.9, 4.212,
        ],
        [
            0.158, 0.54, 1.502, 2.79, 2.955, 3.079, 3.445, 3.805, 4.1, 4.229, 4.429,
        ],
        [
            0.166, 0.567, 1.575, 3.306, 3.509, 3.678, 3.693, 4.13, 4.407, 4.652, 5.229,
        ],
        [
            0.201, 0.616, 1.675, 3.595, 3.815, 3.962, 3.964, 4.313, 5.043, 5.669, 6.713,
        ],
        [
            0.238, 0.668, 1.779, 3.749, 3.986, 4.079, 4.127, 4.651, 5.436, 6.112, 7.695,
        ],
        [
            0.244, 0.676, 1.798, 3.82, 4.033, 4.121, 4.127, 4.707, 5.502, 6.186, 8.055,
        ],
        [
            0.248, 0.68, 1.81, 3.849, 4.06, 4.123, 4.127, 4.737, 5.536, 6.225, 8.29,
        ],
        [
            0.248, 0.68, 1.811, 3.845, 4.061, 4.125, 4.127, 4.74, 5.54, 6.23, 8.267,
        ],
        [
            0.248, 0.68, 1.811, 4.05, 4.123, 4.125, 4.127, 4.74, 5.54, 6.23, 8.267,
        ],
    ];
    pub const RING_GAIN: f64 = 2.5;
    /// EMPHASIS read as the knob drawn with its generic taper would set R14 (voice.rs's
    /// `emphasis_r14` against `emphasis_r14_drawn`); and over the last `EMPHASIS_NEAR` of
    /// the drawn knob below the calibrated circuit's ringing threshold (`THRESHOLD`, the drawn
    /// knob, at these cutoffs, log2 Hz; between them interpolated, beyond them held: it rings
    /// from 8.3 near 210 Hz, 7.25 near 1 kHz and 7.0 near 5 kHz), moved on by `SHIFT` to the
    /// table's own threshold there.
    pub const THRESHOLD_AT_LOG2: [f64; 3] = [7.74, 10.04, 12.37];
    pub const THRESHOLD: [f64; 3] = [0.7923, 0.7274, 0.7164];
    pub const SHIFT: [f64; 3] = [-0.0032, 0.0195, 0.0176];
    pub const EMPHASIS_NEAR: f64 = 0.012;
    /// The ladder's corner above the ring's pitch (the table above), octaves, below the ringing
    /// threshold: fitted to the drawn circuit's responses at its CUTOFF 0.3, 0.5 and 0.7 (220,
    /// 858 and 3454 Hz rings), and below them to the calibrated circuit's resonant peaks at
    /// CUTOFF 0.05 to 0.2 (within 4 Hz); it closes to nothing at the threshold, as the ring's
    /// own saturation pulls the circuit's corner down to its pitch.
    pub const CORNER: [f64; 4] = [-0.1, 0.048, 0.088, 0.111];
    pub const CORNER_AT_LOG2: [f64; 4] = [6.2, 7.78, 9.74, 11.75];
    /// The filter's passband against the open filter's, as the circuit's (whose passband
    /// falls as its cutoff goes below 100 Hz, 4.2 dB at CUTOFF 0): the ladder's output times
    /// this at these cutoffs (log2 Hz), dB.
    pub const PASS_AT_LOG2: [f64; 13] = [
        5.2, 5.5, 5.81, 6.345, 6.787, 7.257, 7.745, 8.306, 8.878, 10.045, 10.626, 14.229, 14.55,
    ];
    pub const PASS_DB: [f64; 13] = [
        -4.1, -3.4, -2.8, -2.0, -1.5, -1.1, -0.8, -0.45, -0.2, 0.1, 0.3, 0.1, 0.0,
    ];
    /// Past the threshold, where `CORNER` has closed, the ring's own pitch raised this much,
    /// octaves (the input pair's knee in the loop pulls the ring flat; the circuit's rings
    /// measured, within 3 cents from 81 Hz to 3.5 kHz).
    pub const RING_RAISE: [f64; 5] = [0.001, -0.0015, 0.0085, 0.0065, -0.0025];
    pub const RING_RAISE_AT_LOG2: [f64; 5] = [6.44, 7.78, 8.75, 11.75, 12.75];
    /// Past the threshold (as `CORNER` closes), the pair's limit raised to this at these
    /// cutoffs (log2 Hz): the circuit's rings at EMPHASIS 10 louder against the input's limit
    /// as they rise (its every stage a pair), within 0.3 dB of them to 15 kHz.
    pub const RING_LEVEL_AT_LOG2: [f64; 15] = [
        6.34, 6.79, 7.26, 7.74, 8.31, 8.88, 9.46, 10.04, 10.63, 11.21, 11.79, 12.37, 12.87, 13.38,
        13.87,
    ];
    pub const RING_LEVEL: [f64; 15] = [
        1.738, 1.352, 1.239, 1.236, 1.205, 1.212, 1.228, 1.235, 1.226, 1.244, 1.264, 1.300, 1.350,
        1.376, 1.451,
    ];

    /// The contours: their rest from power-on and the level an attack stops at (its
    /// plateau), V (filter, loudness); where its RC curve aims, as a share of the plateau's
    /// rise from rest; and how much earlier a fast attack stops (the timing capacitor's
    /// series resistance against the charging current): the plateau less the aim's distance
    /// above it times this over the attack's time constant, s.
    pub const REST: [f64; 2] = [0.0743, -0.516];
    pub const PEAK: [f64; 2] = [4.7917, 5.6014];
    pub const ATTACK_AIM: [f64; 2] = [1.82, 1.40];
    pub const ATTACK_ESR: [f64; 2] = [1.93e-5, 1.66e-5];
    /// The time constants, s, against the pots' resistance R (voice.rs's `time_pot` through
    /// the reference's laws, ohm): A (R / 100K)^B + T0 as (A, B, T0) for ATTACK, DECAY from the
    /// peak, and the release with the DECAY switch on (DECAY's pot), each (filter, loudness):
    /// within 0.6 % of the circuit's at ATTACK 2 to 10 (DECAY with `DECAY_FALL`).
    pub const ATTACK_TIME: [(f64, f64, f64); 2] =
        [(1.114_91, 1.007_46, 0.0007), (1.115_32, 1.003_32, 0.0008)];
    pub const DECAY_TIME: [(f64, f64, f64); 2] =
        [(1.168, 1.000_67, 0.0003), (1.167, 1.000_55, 0.0003)];
    /// DECAY from the peak faster the further it has to fall: its rate times 1 + this times
    /// the volts to go (filter, loudness). It stood in for the absorbed charge's sag after a
    /// fast attack (0.05 and 0.035); with that charge modelled (`Contour::absorbed`) the
    /// circuit's decay is one exponential again, within 0.03 V over 30 s at DECAY 7 to 10.
    pub const DECAY_FALL: [f64; 2] = [0.0, 0.0];
    /// The contour's capacitor's own share of [`ATTACK_TIME`], [`DECAY_TIME`] and
    /// [`RELEASE_TIME`] (the rest its absorbed charge's: `Contour::absorbed`), fitted to the
    /// circuit's slow attacks and decays (decisions.md R-LO-CONTOUR).
    pub const ABSORB_TIME_SCALE: [f64; 3] = [0.885, 0.85, 0.8933];
    pub const RELEASE_TIME: [(f64, f64, f64); 2] =
        [(1.119_76, 1.002_92, 0.0003), (1.123_86, 1.004_29, 0.0003)];
    /// SUSTAIN's level, V: a cubic in the pot's track (voice.rs's `sustain_track`; filter,
    /// loudness), within 6 mV of the circuit's.
    pub const SUSTAIN: [[f64; 4]; 2] = [
        [0.0431, 3.7727, -2.2079, 2.2245],
        [-0.4729, 4.9704, -2.4266, 2.3829],
    ];
    /// Where a release falls to, V (filter, loudness), with the DECAY switch on (at DECAY's
    /// time).
    pub const RELEASED: [f64; 2] = [0.1426, -0.4964];
    /// With the switch off, by DECAY at each of `RELEASE_OFF_AT`: where it falls to, V, and
    /// its time constant, s (the capacitor through DECAY's pot and the switch's resistor at
    /// once: at once and to `RELEASED`'s level at 0, lower and slower as the pot turns up;
    /// the circuit's falls so far and then creeps on, its last tenth over some hundred
    /// milliseconds).
    pub const RELEASE_OFF_AT: [f64; 11] = [0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0];
    pub const RELEASED_OFF: [[f64; 11]; 2] = [
        [
            0.1392, -0.0179, -0.0688, -0.0872, -0.1049, -0.1178, -0.1263, -0.1936, -0.2195,
            -0.2363, -0.2443,
        ],
        [
            -0.4998, -0.6582, -0.6988, -0.7182, -0.7368, -0.7513, -0.7601, -0.8294, -0.8585,
            -0.8773, -0.886,
        ],
    ];
    pub const RELEASE_OFF: [[f64; 11]; 2] = [
        [
            0.0005, 0.0178, 0.0209, 0.0217, 0.0224, 0.0228, 0.0231, 0.0249, 0.0256, 0.026, 0.0262,
        ],
        [
            0.0005, 0.0174, 0.0200, 0.0208, 0.0215, 0.0220, 0.0222, 0.0238, 0.0243, 0.0246, 0.0247,
        ],
    ];
    /// The trigger: the contours start this long after a key goes down and fall this long
    /// after the last is let go, s; a key pressed `REARM` or more after the last was let go
    /// starts them again (an attack), even before they have fallen.
    pub const TRIGGER_ON: f64 = 0.0078;
    pub const TRIGGER_OFF: f64 = 0.0141;
    pub const REARM: f64 = 0.0115;

    /// The VCA: its gain (1 at the loudness contour's 4.455 V) every 0.1 V from 0 V; past
    /// the last, as there.
    pub const VCA_STEP: f64 = 0.1;
    pub const VCA_GAIN: [f32; 54] = [
        0.0, 0.001, 0.0096, 0.0281, 0.0503, 0.0737, 0.0976, 0.1215, 0.1454, 0.1691, 0.1927, 0.219,
        0.245, 0.2706, 0.296, 0.3211, 0.3458, 0.3702, 0.3969, 0.4232, 0.4491, 0.4746, 0.4997,
        0.5245, 0.5512, 0.5775, 0.601, 0.6264, 0.6536, 0.6803, 0.7065, 0.7321, 0.7573, 0.782,
        0.8082, 0.8338, 0.8589, 0.8854, 0.9112, 0.9364, 0.961, 0.9839, 0.9959, 0.9992, 1.0001, 1.0,
        0.9994, 0.9985, 0.9974, 0.9961, 0.9947, 0.9933, 0.9917, 0.99,
    ];

    /// GLIDE: the pitch slides at a constant rate, semitones a second times its pot's ohms
    /// (voice.rs's `glide_r`, the hold's 330 ohm added), rising; falling, this share of it.
    /// It holds while no key is down (the keyboard's track-and-hold).
    pub const GLIDE_RATE: f64 = 9.76e7;
    pub const GLIDE_FALLING: f64 = 0.535;
    /// The keys the keyboard circuit plays (F to C, MIDI); beyond them the plug-in adds the
    /// rest of the scale after it, moving through GLIDE's resistance and the hold's 1 uF alone
    /// (an RC, not the slide), while a key is down (voice.rs's `beyond`).
    pub const KEYBOARD_KEYS: (f64, f64) = (41.0, 84.0);
    pub const GLIDE_FARADS: f64 = 1e-6;
    /// Where a fresh voice's pitch rests, as a key: the keyboard's output before any key.
    pub const PITCH_AT_REST: f64 = 53.75;
    /// How long a released key's pitch contact stays closed after its trigger contact opens,
    /// s (voice.rs's `RELEASE_LEAD`): a key let go while a higher one is held moves the pitch
    /// this much later.
    pub const RELEASE_LEAD: f64 = 0.002;

    /// The modulation at the MODULATION wheel fully forward, from oscillator 3: semitones of
    /// the oscillators at its waveform's swing (each from -1 to 1) and its centre against the
    /// triangle's, DC-coupled as the circuit's line is: the triangle, reverse sawtooth and
    /// sawtooth about their centres, the rectangles one-sided and inverted against their
    /// sound (up while the rectangle is low: 0.28 to 18.9 semitones with the line's offset);
    /// the cutoff's octaves `FILTER_PER_SEMITONE` of the oscillators' semitones; the line's
    /// offset; from the noise (MODULATION MIX at 10): their RMS and offset.
    pub const MOD_SWING: [f64; 4] = [-8.93, -8.85, -8.81, -9.32];
    pub const MOD_CENTRE: [f64; 4] = [0.0, -1.47, -0.69, 8.54];
    pub const FILTER_PER_SEMITONE: f64 = 0.2632;
    pub const MOD_OFFSET: (f64, f64) = (1.06, 0.29);
    pub const NOISE_SEMITONES_RMS: f64 = 2.80;
    pub const NOISE_OCTAVES_RMS: f64 = 0.679;
    pub const NOISE_OFFSET: (f64, f64) = (0.25, 0.062);
    /// The swing's share against the wheel's position as the drawing's wheel would set its
    /// resistance (a cubic), and the offsets' against the wheel's resistance R (voice.rs's
    /// `mod_wheel_r`): `OFFSET.0` R / (R + `OFFSET.1`) (the line divided by its rheostat;
    /// within 12 % of the circuit's from a quarter forward).
    pub const WHEEL: [f64; 3] = [0.828, 0.584, -0.412];
    pub const OFFSET: (f64, f64) = (0.9915, 218.3);

    /// EXTERNAL INPUT: its gain to the output, `EXT_GAIN t / (1 + EXT_LOAD t (1 - t))` for
    /// VOLUME's law t (voice.rs's `ext_taper`: the 1M pot loaded by the preamplifier; within
    /// 2 %), the level its preamplifier clips at there, and the OVERLOAD lamp's onset and
    /// full brightness there.
    pub const EXT_GAIN: f64 = 173.5;
    pub const EXT_LOAD: f64 = 10.16;
    pub const EXT_CLIP: f64 = 0.765;
    pub const OVERLOAD: (f64, f64) = (0.24, 0.36);
    /// EXTERNAL INPUT's couplings, high-passes, Hz: at the jack, and after its preamplifier
    /// (C20) on the way to the mixer (the circuit's input from the jack to the output within
    /// 0.6 dB of it from 3 to 30 Hz).
    pub const EXT_COUPLING_HZ: f64 = 9.0;
    pub const PREAMP_COUPLING_HZ: f64 = 8.0;

    /// A-440: its level at the output (its table's units).
    pub const A440_LEVEL: f32 = 0.2385;

    /// FEEDBACK's cable: how late the output reaches EXTERNAL INPUT, samples, at each of
    /// `FEEDBACK_AT`'s rates (the circuit's loop from EXTERNAL INPUT back is a count of
    /// samples, not a time: 88 at 44.1 and 48 kHz, 95 at 88.2 and 96, less Potato's own path's;
    /// the loop's time sets how it oscillates).
    pub const FEEDBACK_AT: [f64; 4] = [44_100.0, 48_000.0, 88_200.0, 96_000.0];
    pub const FEEDBACK_DELAY: [f64; 4] = [87.9, 87.0, 94.0, 94.2];
}

/// The noise generators' coefficients at a rate: the white's scale (its density a hertz the
/// same at every rate), the pink filter's poles and gains (Paul Kellet's, at 48 kHz, their
/// corners kept in hertz), the red's pole and gain, and the bands' (white's high-pass and
/// low-pass, pink's).
#[derive(Debug, Clone, Copy)]
struct NoiseLaw {
    scale: f32,
    poles: [f32; 3],
    gains: [f32; 3],
    red: (f32, f32),
    band: [f32; 4],
}

impl NoiseLaw {
    fn new(rate: f64) -> NoiseLaw {
        // (At 48 kHz each factor is 1 exactly: the coefficients as they were.)
        let r = (48_000.0 / rate) as f32;
        let pole = |p: f32| p.powf(r);
        let gain = |g: f32, p: f32| g * ((1.0 - pole(p)) / (1.0 - p));
        let one_pole = |hz: f64| 1.0 - (-std::f64::consts::TAU * hz / rate).exp();
        NoiseLaw {
            scale: (rate / 48_000.0).sqrt() as f32,
            poles: [pole(0.99765), pole(0.963), pole(0.57)],
            gains: [
                gain(0.099_046, 0.99765),
                gain(0.296_516_4, 0.963),
                gain(1.052_691_3, 0.57),
            ],
            red: (pole(0.995), gain(0.05, 0.995)),
            band: [
                (-std::f64::consts::TAU * laws::WHITE_BAND.0 / rate).exp() as f32,
                one_pole(interpolate(&laws::NOISE_TOP_AT, &laws::WHITE_TOP, rate).min(0.45 * rate))
                    as f32,
                (-std::f64::consts::TAU * laws::PINK_BAND.0 / rate).exp() as f32,
                one_pole(interpolate(&laws::NOISE_TOP_AT, &laws::PINK_TOP, rate).min(0.45 * rate))
                    as f32,
            ],
        }
    }
}

/// A-440's waveform, one period (the circuit model's, measured).
const A440_SHAPE: [f32; 64] = [
    -0.014, 0.045, 0.106, 0.170, 0.238, 0.304, 0.362, 0.415, 0.459, 0.491, 0.504, 0.496, 0.480,
    0.464, 0.448, 0.433, 0.418, 0.403, 0.389, 0.375, 0.361, 0.346, 0.329, 0.311, 0.289, 0.265,
    0.239, 0.210, 0.179, 0.146, 0.112, 0.076, 0.038, -0.001, -0.042, -0.085, -0.130, -0.176,
    -0.221, -0.264, -0.304, -0.340, -0.374, -0.404, -0.430, -0.452, -0.470, -0.483, -0.492, -0.496,
    -0.495, -0.490, -0.479, -0.464, -0.443, -0.418, -0.388, -0.354, -0.316, -0.273, -0.227, -0.178,
    -0.126, -0.071,
];

/// tanh, rational: within 2.4 % of it, exactly odd, ±1 beyond ±3.
#[inline(always)]
fn tanh(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}

/// 2^x for |x| < 64, within 2e-7 of it relatively: the nearest integer in the exponent's
/// bits (found by adding and taking away 1.5 * 2^23, not by a library call), and 2^f for the
/// rest (|f| <= 1/2) by its series to the sixth power.
#[inline(always)]
fn exp2(x: f32) -> f32 {
    const SHIFT: f32 = 12_582_912.0;
    let x = x.clamp(-60.0, 60.0);
    let i = (x + SHIFT) - SHIFT;
    let f = x - i;
    let p = 1.0
        + f * (std::f32::consts::LN_2
            + f * (0.240_226_5
                + f * (0.055_504_11
                    + f * (0.009_618_129 + f * (0.001_333_355 + f * 0.000_154_035)))));
    f32::from_bits((p.to_bits() as i32 + ((i as i32) << 23)) as u32)
}

/// The band-limiting corrections around phase `p` (0..1) for an increment `dt` a sample's
/// reciprocal `inv`: from every edge at phase `at` (each cycle) within the kernel's reach
/// ([`crate::resample::blep_tables`], [`crate::resample::BLEP_DELAY`] samples either side),
/// a step of one (`corner` false) or a corner where the slope grows by one a sample (`corner`
/// true). The phase tells where each edge is, before it comes as after, so nothing is held
/// back (decisions.md R-BLEP).
#[inline(always)]
fn around(t: &BlepTables, p: f32, at: f32, inv: f32, corner: bool) -> f32 {
    const REACH: f32 = crate::resample::BLEP_DELAY as f32;
    let table = if corner { &t.ramp32 } else { &t.step32 };
    let value = |x: f32| {
        let u = (x + REACH) * crate::resample::BLEP_POINTS as f32;
        let i = (u as usize).min(table.len() - 2);
        let f = u - i as f32;
        let v = table[i] + (table[i + 1] - table[i]) * f;
        if corner {
            v - x.max(0.0)
        } else if x >= 0.0 {
            v - 1.0
        } else {
            v
        }
    };
    let mut q = p - at;
    if q < 0.0 {
        q += 1.0;
    }
    let mut sum = 0.0;
    // The edges passed (x >= 0), then those to come.
    let mut k = 0.0;
    loop {
        let x = (q + k) * inv;
        if x >= REACH {
            break;
        }
        sum += value(x);
        k += 1.0;
    }
    let mut k = -1.0;
    loop {
        let x = (q + k) * inv;
        if x <= -REACH {
            break;
        }
        sum += value(x);
        k -= 1.0;
    }
    sum
}

/// A table's value at `x`, its points at `xs` (rising); continued flat beyond its ends.
fn interpolate(xs: &[f64], ys: &[f64], x: f64) -> f64 {
    if x <= xs[0] {
        return ys[0];
    }
    for i in 1..xs.len() {
        if x <= xs[i] {
            let a = (x - xs[i - 1]) / (xs[i] - xs[i - 1]);
            return ys[i - 1] + (ys[i] - ys[i - 1]) * a;
        }
    }
    ys[ys.len() - 1]
}

/// A mixer channel's gain for its VOLUME (25K linear, its wiper through `series` ohm to the
/// bus, the source `source` ohm behind it), against the same at 10 for the sawtooth's.
fn channel(volume: f64, source: f64, series: f64) -> f32 {
    let law = |p: f64, source: f64| {
        let p = p.clamp(0.0, 1.0);
        let (up, down) = (source + 25e3 * (1.0 - p), (25e3 * p).max(1e-3));
        (down / (up + down)) / (up * down / (up + down) + series)
    };
    (law(volume, source) / law(1.0, laws::SOURCE_OHMS[1])) as f32
}

/// The filter's control node takes currents into its SCALE network (about 2.1K, against
/// which the panel's 33K to 300K hardly load it): CUTOFF's, from its wiper (the hardware
/// reference's track, `cutoff_track`) across +-10 V through R55 200K, A.
fn knob_current(cutoff: f64) -> f64 {
    let w = cutoff_track(cutoff);
    (-10.0 + 20.0 * w) / (200e3 + 5e3 * w * (1.0 - w))
}

/// The filter contour's current into the control node, A a volt of contour: AMOUNT OF
/// CONTOUR's wiper (the hardware reference's track, [`contour_amount_track`]) through R74
/// with the pot's own resistance in series ([`crate::voice::contour_input`]).
fn contour_current(amount: f64) -> f64 {
    let t = contour_amount_track(amount);
    t / (crate::vcf::R74 + 5e3 * t * (1.0 - t))
}

/// The ladder's log2 pitch for a current into the control node: [`laws::CUTOFF_LOG2`] at
/// CUTOFF's own currents (`at`, [`knob_current`] at each of its points), straight between
/// them, beyond its ends at their slopes. So the contour moves the filter as CUTOFF would by
/// the same current, as in the circuit, where the converter's law is not exponential at the
/// bottom.
fn node_log2(at: &[f64; 21], current: f64) -> f64 {
    let c = &laws::CUTOFF_LOG2;
    let k = match at.iter().position(|&a| a > current) {
        Some(0) => 0,
        Some(k) => k - 1,
        None => 19,
    }
    .min(19);
    let (x0, x1) = (at[k], at[k + 1]);
    let (y0, y1) = (f64::from(c[k]), f64::from(c[k + 1]));
    y0 + (y1 - y0) * (current - x0) / (x1 - x0)
}

/// The keys as the engine plays them: which are held (a bit a MIDI note), lowest first.
#[derive(Debug, Clone, Copy, Default)]
struct Held(u128);

impl Held {
    fn set(&mut self, key: i32, on: bool) {
        if (0..128).contains(&key) {
            let b = 1u128 << key;
            if on {
                self.0 |= b;
            } else {
                self.0 &= !b;
            }
        }
    }

    fn lowest(self) -> Option<i32> {
        (self.0 != 0).then(|| self.0.trailing_zeros() as i32)
    }

    fn highest(self) -> Option<i32> {
        (self.0 != 0).then(|| 127 - self.0.leading_zeros() as i32)
    }

    /// The key held pressed last, by `pressed` (when each was pressed, counted in presses).
    fn newest(self, pressed: &[u32; 128]) -> Option<i32> {
        let mut bits = self.0;
        let mut newest: Option<usize> = None;
        while bits != 0 {
            let k = bits.trailing_zeros() as usize;
            bits &= bits - 1;
            if newest.is_none_or(|n| pressed[k] > pressed[n]) {
                newest = Some(k);
            }
        }
        newest.map(|k| k as i32)
    }
}

/// LO's triangle source transfer, approximated from the ngspice-derived DC table.
/// Two finite knees and the broad transistor curvature replace the ideal triangle.
/// Coefficients are fitted to source voltage, never recorded audio or spectral targets.
/// `docs/calibration/waveform-fold-reference.md` records the source and approximation.
mod triangle_source {
    pub const V0: f32 = 1.5566981;
    pub const SLOPE: f32 = -7.3142695;
    pub const CURVATURE: f32 = 0.020533603;
    pub const MEAN: f32 = -0.19010404;
    pub const SCALE: f32 = 2.116305;
    pub const RESET_STEP: f32 = -0.14804143;
    pub const RESET_SLOPE: f32 = -14.60845;
    pub const KNEES: [(f32, f32, f32); 2] = [
        (0.34542942, 0.13680515, 0.6440923),
        (0.48223457, 0.020794975, 13.92329),
    ];

    const _: () = {
        assert!(CURVATURE >= 0.0);
        let mut i = 0;
        while i < KNEES.len() {
            let (_, width, gain) = KNEES[i];
            assert!(width > 0.0 && gain > 0.0);
            i += 1;
        }
    };

    pub const fn value(p: f32) -> f32 {
        let mut v = V0 + SLOPE * p + CURVATURE * p * p;
        let mut i = 0;
        while i < KNEES.len() {
            let (at, width, gain) = KNEES[i];
            let x = p - at;
            let hinge = if x <= 0.0 {
                0.0
            } else if x < width {
                x * x / (2.0 * width)
            } else {
                x - 0.5 * width
            };
            v += gain * hinge;
            i += 1;
        }
        (v - MEAN) / SCALE
    }

    const fn slope(p: f32) -> f32 {
        let mut v = SLOPE + 2.0 * CURVATURE * p;
        let mut i = 0;
        while i < KNEES.len() {
            let (at, width, gain) = KNEES[i];
            let x = p - at;
            v += gain
                * if x <= 0.0 {
                    0.0
                } else if x < width {
                    x / width
                } else {
                    1.0
                };
            i += 1;
        }
        v
    }

    /// This convex source transfer's minimum is its zero slope, and its maximum an
    /// endpoint. Evaluate at compile time: no first-use source scan on the audio thread.
    const fn extrema() -> (f32, f32) {
        let (mut lo, mut hi) = (0.0, 1.0);
        let mut i = 0;
        while i < 32 {
            let p = 0.5 * (lo + hi);
            if slope(p) < 0.0 {
                lo = p;
            } else {
                hi = p;
            }
            i += 1;
        }
        let a = value(0.0);
        let b = value(1.0);
        (value(0.5 * (lo + hi)), if a > b { a } else { b })
    }

    pub const EXTREMA: (f32, f32) = extrema();
}

/// The light model's triangle source before band-limiting, for the circuit lab.
#[doc(hidden)]
pub fn triangle_naive(p: f32) -> f32 {
    triangle_source::value(p)
}

/// CONTINUOUS MOD's reference is the current triangle's midpoint and half-swing, as in
/// the circuit voice's `mod_reference`. The calibrated fold is not symmetric about its
/// period mean. Keep the physical pitch/filter modulation laws separate from this share
/// of the plug-in's shape travel.
const SHAPE_MOD_REFERENCE: (f32, f32) = {
    let (lo, hi) = triangle_source::EXTREMA;
    let gain = laws::MOD_SWING[0] as f32 / laws::TRIANGLE;
    (
        0.5 * (lo + hi) * gain + laws::MOD_CENTRE[0] as f32,
        -0.5 * (hi - lo) * gain,
    )
};

#[inline(always)]
fn shape_mod_swing(swing: f32) -> f64 {
    f64::from((swing - SHAPE_MOD_REFERENCE.0) / SHAPE_MOD_REFERENCE.1)
}

/// A finite knee's periodic BLAMP correction, using the circuit voice's same kernel.
#[inline(always)]
fn around_fillet(t: &BlepTables, p: f32, at: f32, inv: f32, width: f32) -> f32 {
    let w = width * inv;
    if f64::from(w) > crate::resample::MAX_FILLET {
        return 0.0;
    }
    let reach = crate::resample::BLEP_DELAY as f32;
    let mut q = p - at;
    if q < 0.0 {
        q += 1.0;
    }
    let mut sum = 0.0;
    let mut k = 0.0;
    loop {
        let x = (q + k) * inv;
        if x >= reach + w {
            break;
        }
        sum += t.fillet(f64::from(x), f64::from(w)) as f32;
        k += 1.0;
    }
    let mut k = -1.0;
    loop {
        let x = (q + k) * inv;
        if x <= -reach {
            break;
        }
        sum += t.fillet(f64::from(x), f64::from(w)) as f32;
        k -= 1.0;
    }
    sum
}

/// An oscillator: its phase (0..1) and waveform.
#[derive(Debug, Clone, Copy)]
struct Osc {
    phase: f32,
    wave: Waveform,
    /// The rectangle's duty cycle and level ([`rect_law`]): its switch's, or CONTINUOUS's
    /// width (decisions.md R-CONTINUOUS-WAVE).
    duty: f32,
    level: f32,
    /// CONTINUOUS between two of the switch's places, or moved by MOD: where its shape is
    /// (the switch's steps), its places' levels against the sawtooth's through the mixer
    /// (each place's source resistance), and whether it is oscillator 3 (its second and third
    /// places a reverse shark tooth and its reverse sawtooth). `None`: a place of the switch's,
    /// played as the switch's own position (`wave`).
    shape: Option<f32>,
    ratios: [f32; 4],
    third: bool,
    /// The band-limiting kernel's tables.
    tables: &'static BlepTables,
}

impl Osc {
    /// One sample at increment `dt` (cycles a sample, under a half) and its reciprocal.
    #[inline(always)]
    fn tick(&mut self, dt: f32, inv: f32) -> f32 {
        let mut p = self.phase + dt;
        if p >= 1.0 {
            p -= 1.0;
        }
        self.phase = p;
        // The sawtooth falls and jumps up at its reset; the reverse sawtooth rises.
        let t = self.tables;
        // A step at phase `at`, and a corner there where the slope (a unit of phase) grows
        // by `g`.
        let step = |at: f32| around(t, p, at, inv, false);
        let corner = |at: f32, g: f32| g * dt * around(t, p, at, inv, true);
        let fall = || 1.0 - 2.0 * p + 2.0 * step(0.0);
        let rect = |d: f32| {
            let naive = if p < d { 1.0 } else { -1.0 };
            // (Centred: the rectangle's mean taken out, as the coupling does.)
            naive + 2.0 * (step(0.0) - step(d)) - (2.0 * d - 1.0)
        };
        // The Q2/Q1 source curve, its finite fold knees and its physical reset step.
        let triangle = || {
            use triangle_source::*;
            let mut v = triangle_naive(p)
                + RESET_STEP / SCALE * step(0.0)
                + RESET_SLOPE / SCALE * corner(0.0, 1.0);
            for (at, width, gain) in KNEES {
                v += gain / SCALE * dt * around_fillet(t, p, at, inv, width);
            }
            v
        };
        let shark = || laws::SHARK_TRIANGLE_WEIGHT * triangle() + laws::SHARK_SAWTOOTH * fall();
        let sawtooth = || laws::SAWTOOTH * fall();
        let reverse = || -laws::REVERSE_SAWTOOTH * fall();
        let rectangle = || self.level * rect(self.duty);
        let Some(at) = self.shape else {
            return match self.wave {
                Waveform::Triangle => triangle(),
                Waveform::SharkTooth => shark(),
                Waveform::Sawtooth => sawtooth(),
                Waveform::ReverseSawtooth => reverse(),
                Waveform::Square | Waveform::WideRectangle | Waveform::NarrowRectangle => {
                    rectangle()
                }
            };
        };
        // CONTINUOUS (decisions.md R-CONTINUOUS-WAVE): oscillator 3's reverse shark tooth its
        // shark tooth's triangle with the reverse sawtooth where the sawtooth was, at the
        // reverse sawtooth's level.
        let place = |k: usize| match (k, self.third) {
            (0, _) => triangle(),
            (1, false) => shark(),
            (1, true) => {
                laws::SHARK_TRIANGLE_WEIGHT * triangle()
                    + laws::SHARK_SAWTOOTH * (laws::REVERSE_SAWTOOTH / laws::SAWTOOTH) * -fall()
            }
            (2, false) => sawtooth(),
            (2, true) => reverse(),
            _ => rectangle(),
        };
        if at >= 3.0 {
            return self.ratios[3] * place(3);
        }
        let k = at as usize;
        let w = at - k as f32;
        if w == 0.0 {
            return self.ratios[k] * place(k);
        }
        (1.0 - w) * self.ratios[k] * place(k) + w * self.ratios[k + 1] * place(k + 1)
    }

    /// Its waveform's level at its peak (oscillator 3's swing as a modulation source).
    fn peak(&self) -> f32 {
        match self.wave {
            Waveform::Triangle => laws::TRIANGLE,
            Waveform::SharkTooth => laws::SHARK_TRIANGLE + laws::SHARK_SAWTOOTH,
            Waveform::Sawtooth => laws::SAWTOOTH,
            Waveform::ReverseSawtooth => laws::REVERSE_SAWTOOTH,
            Waveform::Square | Waveform::WideRectangle | Waveform::NarrowRectangle => self.level,
        }
    }

    /// An oscillator on `wave` at phase 0, its rectangle the switch's.
    fn new(wave: Waveform) -> Self {
        let (duty, level) = rect_law(wave.width());
        Osc {
            phase: 0.0,
            wave,
            duty,
            level,
            shape: None,
            ratios: [1.0; 4],
            third: false,
            tables: crate::resample::blep_tables(),
        }
    }
}

/// The modulation's depth at the MODULATION wheel's `wheel` (the wheel as the drawing's would
/// set its resistance, through `laws::WHEEL`).
fn light_depth(wheel: f64) -> f64 {
    let r = crate::modulation::mod_wheel_r(wheel);
    let w = 3.0 * (1.0 + r / 1.2e3 * (81f64.powf(1.0 / 3.0) - 1.0)).ln() / 81f64.ln();
    let w = w.clamp(0.0, 1.0);
    let [a, b, c] = laws::WHEEL;
    w * (a + w * (b + w * c))
}

/// Oscillator `o`'s waveform as the light voice plays it at the panel (decisions.md
/// R-CONTINUOUS-WAVE): the switch's own position (STEPPED, or CONTINUOUS at one of the
/// switch's places or past SQUARE, the rectangle at its width), with no blend; else, between
/// two places or moved by MOD with the wheel forward (`depth` above 0), where its shape is,
/// the waveform the nearest place is named for its swing as a modulation source.
fn light_shape(o: usize, p: &Panel, depth: f64) -> (Waveform, Option<f32>) {
    use crate::voice::{ModSwing, shape_at, shape_place};
    let s = p.shape[o];
    if !s.continuous {
        return (p.osc[o].waveform, None);
    }
    let at = shape_at(s, ModSwing::default());
    let nearest =
        |at: f64| shape_place(o, (at.round() as usize).min(3)).unwrap_or(Waveform::ReverseSawtooth);
    if s.modulated && depth > 0.0 {
        return (nearest(at), Some(at as f32));
    }
    if at >= 3.0 {
        return (Waveform::Square, None);
    }
    match shape_place(o, at as usize) {
        Some(w) if at.fract() == 0.0 => (w, None),
        _ => (nearest(at), Some(at as f32)),
    }
}

/// The rectangle's duty cycle and level at a width bias (V): the switch's three as fitted
/// ([`laws::DUTY`], [`laws::SQUARE`], [`laws::WIDE`], [`laws::NARROW`]) at its 0, -1.5 and
/// -2.5 V, exactly; straight between them, and on past the narrow one as from the wide one
/// to it, to CONTINUOUS's floor (decisions.md R-CONTINUOUS-WAVE; the circuit's duty is straight
/// in the bias: the comparator's threshold moves 0.998 V a volt along a straight ramp).
pub(crate) fn rect_law(bias: f64) -> (f32, f32) {
    let knots = [
        (0.0, laws::DUTY[0], laws::SQUARE),
        (-1.5, laws::DUTY[1], laws::WIDE),
        (-2.5, laws::DUTY[2], laws::NARROW),
    ];
    let k = if bias >= -1.5 { 0 } else { 1 };
    let ((b0, d0, l0), (b1, d1, l1)) = (knots[k], knots[k + 1]);
    let t = (bias - b0) / (b1 - b0);
    let at = |a: f32, b: f32| ((1.0 - t) * f64::from(a) + t * f64::from(b)) as f32;
    (at(d0, d1), at(l0, l1))
}

/// The mixer's own noise on the bus in the bus's units, a uniform generator's scale at 48 kHz:
/// the circuit's [`crate::voice::MIXER_HISS`] against its bus's 0.0319 mA RMS for a sawtooth at
/// VOLUME 10 (both uniform and sawtooth 1/sqrt(3) of their peak RMS).
const HISS: f64 = crate::voice::MIXER_HISS / 0.0319e-3;
/// Its generator's seed, apart from the noise's.
const HISS_SEED: u32 = 0x6869_7373;

/// White noise (xorshift), and pink and red from it (filtered), with the white's band.
#[derive(Debug, Clone, Copy)]
struct Noise {
    state: u32,
    pink: [f32; 3],
    red: f32,
    /// The white's high-pass (its last input and output) and low-pass, the pink's.
    hp: (f32, f32),
    lp: f32,
    pink_hp: (f32, f32),
    pink_lp: f32,
}

impl Noise {
    #[inline(always)]
    fn white(&mut self) -> f32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        (x as i32 as f32) * (1.0 / 2_147_483_648.0)
    }

    /// White (its band), pink (its) and red, each in its generator's units; `c` the
    /// generators' coefficients at the rate.
    #[inline(always)]
    fn tick(&mut self, c: &NoiseLaw) -> (f32, f32, f32) {
        let w = self.white() * c.scale;
        let b = &mut self.pink;
        b[0] = c.poles[0] * b[0] + w * c.gains[0];
        b[1] = c.poles[1] * b[1] + w * c.gains[1];
        b[2] = c.poles[2] * b[2] + w * c.gains[2];
        let pink = 0.25 * (b[0] + b[1] + b[2] + w * 0.1848);
        self.red = c.red.0 * self.red + c.red.1 * pink;
        let (x0, y0) = self.hp;
        let hp = c.band[0] * (y0 + w - x0);
        self.hp = (w, hp);
        self.lp += (hp - self.lp) * c.band[1];
        let (x0, y0) = self.pink_hp;
        let hp = c.band[2] * (y0 + pink - x0);
        self.pink_hp = (pink, hp);
        self.pink_lp += (hp - self.pink_lp) * c.band[3];
        (self.lp, self.pink_lp, self.red)
    }
}

/// The ladder: four one-pole stages, trapezoidal, the loop solved without delay for its
/// nonlinear input pair; the loop's feedback through its own limit (how loud it rings).
#[derive(Debug, Clone, Copy, Default)]
struct Ladder {
    s: [f32; 4],
}

impl Ladder {
    /// One sample of input `x` (the pair's units) at stage gain `g` (G / (1 + G) of the
    /// prewarped coefficient G) and loop gain `k`: the fourth stage's output. The input pair
    /// takes the input less the loop's feedback through its knee ([`pair`], off its centre by
    /// `bias`; `limit` its limit, the limit's reciprocal and its output at the bias alone), as
    /// the circuit's does: driven hard, it squashes the resonance and loses less bass.
    #[inline(always)]
    fn tick(&mut self, x: f32, g: f32, k: f32, bias: f32, limit: (f32, f32, f32)) -> f32 {
        let s = &mut self.s;
        let h = 1.0 - g;
        let g2 = g * g;
        let g4 = g2 * g2;
        let rest = h * (g * (g * (g * s[0] + s[1]) + s[2]) + s[3]);
        let y4 = (g4 * x + rest) / (1.0 + k * g4);
        let (top, inv, rest_bias) = limit;
        let u = x - k * y4 + bias;
        let mut y = if top == 1.0 {
            tanh(u)
        } else {
            top * tanh(u * inv)
        } - rest_bias;
        // Solve y = top*tanh((x - k*(g^4*y + rest) + bias)/top) - rest_bias.
        // Its residual is monotone (slope >= 1). Newton normally takes a few steps;
        // the pair's limits bracket the root if a step crosses its knee.
        let q = k * g4;
        let v = x - k * rest + bias;
        let (mut lo, mut hi) = (-top - rest_bias, top - rest_bias);
        let mut converged = false;
        for _ in 0..24 {
            let a = (v - q * y) * inv;
            let t = tanh(a);
            let r = y - (top * t - rest_bias);
            if r.abs() < 1e-6 {
                converged = true;
                break;
            }
            if r < 0.0 {
                lo = y;
            } else {
                hi = y;
            }
            let slope = if a.abs() >= 3.0 {
                0.0
            } else {
                let a2 = a * a;
                let d = (a2 - 9.0) / (a2 + 3.0);
                d * d / 9.0
            };
            let next = y - r / (1.0 + q * top * inv * slope);
            y = if next >= lo && next <= hi {
                next
            } else {
                (lo + hi) * 0.5
            };
        }
        if !converged {
            crate::unconverged::note(crate::unconverged::Solver::FilterLoop);
        }
        for st in s.iter_mut() {
            let v = (y - *st) * g;
            let out = v + *st;
            *st = out + v;
            y = out;
        }
        y
    }
}

/// A contour generator: its level (V) and whether it is in its attack.
#[derive(Debug, Clone, Copy)]
struct Contour {
    volts: f64,
    /// The timing capacitor's absorbed charge: each of its three branches' voltage.
    absorbed: [f64; 3],
    attacking: bool,
    /// Never yet started (it rests where power-on left it).
    fresh: bool,
}

/// A contour's coefficients a sample (the attack's, the decay's, the release's), and the
/// levels it aims at, stops at, holds and rests at (V).
#[derive(Debug, Clone, Copy, Default)]
struct ContourLaw {
    attack: f64,
    decay: f64,
    release: f64,
    aim: f64,
    stop: f64,
    sustain: f64,
    released: f64,
    fall: f64,
    /// The dielectric absorption's branches ([`crate::contour::C_ABSORPTION`] across the
    /// timing capacitor's 10 uF): each one's share of a sample's settling toward the
    /// capacitor, and the shares of what moves that the capacitor and the branch take.
    absorb: [(f64, f64, f64); 3],
}

impl ContourLaw {
    fn new(k: ContourKnobs, decay_switch: bool, which: usize, rate: f64) -> ContourLaw {
        type Law = [(f64, f64)];
        let (attack_law, decay_law): (&Law, &Law) = if which == 0 {
            (&FILTER_ATTACK, &FILTER_DECAY)
        } else {
            (&LOUDNESS_ATTACK, &LOUDNESS_DECAY)
        };
        let time = |(a, b, t0): (f64, f64, f64), ohms: f64| a * (ohms / 1e5).powf(b) + t0;
        let r_decay = time_pot(k.decay, decay_law);
        let tau_a = time(laws::ATTACK_TIME[which], time_pot(k.attack, attack_law));
        let (rest, peak) = (laws::REST[which], laws::PEAK[which]);
        let aim = rest + laws::ATTACK_AIM[which] * (peak - rest);
        let [c0, c1, c2, c3] = laws::SUSTAIN[which];
        let s = sustain_track(k.sustain);
        let (tau_r, released) = if decay_switch {
            (
                time(laws::RELEASE_TIME[which], r_decay),
                laws::RELEASED[which],
            )
        } else {
            let d = k.decay.clamp(0.0, 1.0);
            (
                interpolate(&laws::RELEASE_OFF_AT, &laws::RELEASE_OFF[which], d),
                interpolate(&laws::RELEASE_OFF_AT, &laws::RELEASED_OFF[which], d),
            )
        };
        let per = |tau: f64| 1.0 - (-1.0 / (tau * rate)).exp();
        // (The laws' times are the capacitor's with its absorbed charge; the branches below
        // carry that, so the capacitor's own are this much shorter.)
        let [ka, kd, kr] = laws::ABSORB_TIME_SCALE;
        let c = crate::contour::FILTER_SECTION.c;
        let absorb = crate::contour::C_ABSORPTION.map(|(ck, rk)| {
            let tau = rk * c * ck / (c + ck);
            (per(tau), ck / (c + ck), c / (c + ck))
        });
        ContourLaw {
            attack: per(tau_a * ka),
            decay: per(time(laws::DECAY_TIME[which], r_decay) * kd),
            release: per(tau_r * kr),
            aim,
            stop: peak - (aim - peak) * laws::ATTACK_ESR[which] / tau_a,
            sustain: c0 + s * (c1 + s * (c2 + s * c3)),
            released,
            fall: laws::DECAY_FALL[which],
            absorb,
        }
    }
}

impl Contour {
    /// At rest: where the circuit's rests with these settings (its DECAY and the switch).
    fn at_rest(law: &ContourLaw) -> Contour {
        Contour {
            volts: law.released,
            absorbed: [law.released; 3],
            attacking: false,
            fresh: true,
        }
    }

    /// A sample on, its gate high or low.
    #[inline(always)]
    fn tick(&mut self, gate: bool, law: &ContourLaw) -> f64 {
        if gate {
            self.fresh = false;
            if self.attacking {
                let next = self.volts + (law.aim - self.volts) * law.attack;
                if next >= law.stop {
                    // (The comparator flips at its threshold: the attack ends there.)
                    self.volts = law.stop.max(self.volts);
                    self.attacking = false;
                } else {
                    self.volts = next;
                }
            } else {
                let to_go = self.volts - law.sustain;
                self.volts -= to_go * law.decay * (1.0 + law.fall * to_go);
            }
        } else if self.fresh {
            // (Never started: where the circuit rests with these settings.)
            self.volts = law.released;
            self.absorbed = [law.released; 3];
            return self.volts;
        } else {
            self.attacking = false;
            self.volts += (law.released - self.volts) * law.release;
        }
        // The absorbed charge catching up with the capacitor, or giving itself back.
        for (u, &(a, to_c, to_u)) in self.absorbed.iter_mut().zip(&law.absorb) {
            let e = (self.volts - *u) * a;
            self.volts -= e * to_c;
            *u += e * to_u;
        }
        self.volts
    }
}

/// What the panel sets, worked out when it changes.
#[derive(Debug, Clone, Copy, Default)]
struct Setting {
    /// Each oscillator's pitch offset from its key, semitones, and whether it follows the
    /// keys (oscillator 3: OSC. 3 CONTROL).
    offset: [f64; 3],
    follows: [bool; 3],
    /// Each channel's gain into the bus: the oscillators, the noise, the external input.
    gain: [f32; 3],
    noise: f32,
    ext: f32,
    /// CUTOFF's current into the control node (A), the keys' share (octaves an octave),
    /// AMOUNT OF CONTOUR's current a volt of the filter contour (A), EMPHASIS's loop gain at
    /// each of [`laws::LOOP_AT_LOG2`]'s cutoffs.
    cutoff: f64,
    tracking: f64,
    contour: f64,
    emphasis: [f64; 10],
    /// How much of the corner's rise above the ring's pitch EMPHASIS leaves (1 below 7, 0 from
    /// the threshold).
    corner: f64,
    /// The modulation: its depth (the wheel), its mix, where it goes; oscillator 3 as its
    /// source: its sample to its waveform's -1..1 (times `swing`, plus `swing_bias`: the
    /// rectangles' levels before their coupling), and that to semitones.
    depth: f64,
    /// The depth with the wheel fully forward (MOD's measure).
    depth_full: f32,
    offset_depth: f64,
    mix: f32,
    osc_mod: bool,
    filter_mod: bool,
    /// Which oscillators' shapes follow the modulation (CONTINUOUS with MOD: decisions.md
    /// R-CONTINUOUS-WAVE).
    shape_mod: [bool; 3],
    swing: f32,
    swing_bias: f32,
    mod_swing: f32,
    mod_centre: f32,
    /// GLIDE's slide a control step, semitones (rising, falling), and its RC's share left a
    /// control step (beyond the keyboard's keys); the external input's gain to the output.
    glide: (f64, f64),
    beyond: f64,
    ext_gain: f64,
    contours: [ContourLaw; 2],
}

/// Potato mode's voice.
#[derive(Debug, Clone)]
pub struct Light {
    rate: f64,
    /// MOD's share of CONTINUOUS's travel (onward), the last sample's ([`Light::probe_shape_mod`]).
    mod_u: f32,
    /// Each oscillator's CONTINUOUS shape where it was, the last sample's ([`Light::probe_shapes`]).
    shape_now: [Option<f64>; 3],
    /// The panel, and whether `set` is worked out from it.
    panel: Panel,
    set_for_panel: bool,
    set: Setting,
    held: Held,
    /// When each key was last pressed, counted in presses (for LAST), and the count.
    pressed: [u32; 128],
    presses: u32,
    /// RETRIGGER's keys pressed, waiting for the keys' gate to have been low `gap` samples
    /// (voice.rs's `RETRIGGER_GAP`).
    waiting: Held,
    gap: u32,
    /// A released key whose pitch contact is still closed, the samples it stays so, and that
    /// time in samples.
    lead: (i32, u32),
    lead_samples: u32,
    /// The pitch the keys ask for and where GLIDE has it, semitones (MIDI): the keyboard's
    /// own (F to C), and the keys beyond it (asked for, and where they are).
    target: f64,
    pitch: f64,
    foot: f64,
    beyond: f64,
    /// The keys' gate; the contours' (the trigger's), the samples to its next change, the
    /// samples the keys' gate has been low, and the trigger's times in samples.
    gate: bool,
    trigger: bool,
    pending: u32,
    /// Whether the trigger, when it follows the keys up, starts an attack (re-armed).
    attack: bool,
    open: u32,
    rearm: u32,
    trigger_on: u32,
    trigger_off: u32,
    osc: [Osc; 3],
    /// Each oscillator's increment, its step a sample, and the reciprocal of where it goes.
    dt: [f32; 3],
    dt_step: [f32; 3],
    inv: [f32; 3],
    noise: Noise,
    noise_c: NoiseLaw,
    /// The mixer's own noise on the bus (the circuit's [`crate::voice::MIXER_HISS`]): its
    /// generator's state and its scale at the rate. The filter's self-oscillation starts
    /// from it; without it, nothing on the bus, the ladder never sings.
    hiss: (u32, f32),
    /// FEEDBACK's cable at the rate, samples.
    feedback_delay: f32,
    /// The output's high-passes (its droop): the filter's coupling and the one after the VCA,
    /// each its coefficient, last input and output; FILTER MODE HI's direct branch's
    /// coupling likewise, and that branch's gain from the bus (control rate); the input
    /// pair's offset at rest.
    coupling: (f32, f32, f32),
    droop: (f32, f32, f32),
    mode: (f32, f32, f32),
    direct: f32,
    /// The filter's passband at its cutoff (control rate).
    pass: f32,
    /// The thump's band-pass: its coefficients (high-pass, low-pass) and its state (the
    /// VCA's last gain, the high-pass's output, the low-pass's).
    thump_c: (f32, f32),
    thump: (f32, f32, f32),
    bias: f32,
    ladder: Ladder,
    filter_up: Interpolator,
    filter_down: Decimator,
    /// FILTER MODULATION's line ahead of the cutoff's law, whether anything is in it, and the
    /// ladder's stage gain and loop gain where the last sample left them.
    line_up: Interpolator,
    line_live: bool,
    ladder_at: (f32, f32),
    /// The ladder's stage gain (and its step a sample) and loop gain.
    g: f32,
    g_step: f32,
    k: f32,
    contours: [Contour; 2],
    /// The contours' volts of the last samples (filter, loudness), the newest at `late_at`:
    /// each reaches what it shapes as late as the sound does ([`Light::late_contours`];
    /// decisions.md R-LATENCY), these samples late (filter, loudness).
    late_past: [(f64, f64); LATE_PAST],
    late_at: usize,
    late: (f64, f64),
    /// The filter contour as it reaches the ladder, V.
    filter_volts: f64,
    /// Oscillator 1's resets (its phase's wraps), a probe: the latest's time in samples and
    /// how many, and the samples played.
    resets: (f64, u64),
    samples: u64,
    count: usize,
    /// ENTROPY's offsets: each oscillator's cents and the cutoff's octaves.
    detune: [f64; 3],
    cutoff_offset: f64,
    /// The plug-in's LFOs' glide of CUTOFF's current since their step, A (decisions.md
    /// R-LFO-STRESS; 0: none).
    cutoff_swept: f64,
    /// CUTOFF's currents at its table's points ([`node_log2`]), and the control node's log2
    /// pitch at the last control step.
    cutoff_at: [f64; 21],
    node_log2: f64,
    /// Each oscillator timed: where its cycle stands on the host's clock (decisions.md
    /// R-OSC-TIMED), for the samples that follow; its latest reset while timed, in samples as
    /// [`Light::probe_resets`] counts them, and how many there have been.
    timed: [Option<Timed>; 3],
    timed_resets: [(f64, u64); 3],
    /// Oscillator 3's last sample (the modulation's source).
    osc3: f32,
    /// A-440's phase and its step a sample.
    a440: (f32, f32),
    /// FEEDBACK's share; the output's last samples (FEEDBACK's delay) and where the next
    /// goes; the OVERLOAD lamp's brightest.
    pub feedback: f64,
    fed_back: [f32; 128],
    fed_at: usize,
    lamp: f64,
    /// EXTERNAL INPUT's couplings (the jack's, the preamplifier's): each its coefficient, last
    /// input and output.
    ext_hp: (f64, f64, f64),
    pre_hp: (f32, f32, f32),
    overload_peak: f64,
    /// The plug-in's DRIVE: the gain on the bus into the ladder's input pair (1 the circuit),
    /// and the pair's limit with it (as [`Ladder::tick`] takes it), raised by `ring_lift` past
    /// EMPHASIS's threshold.
    drive: f32,
    drive_top: f32,
    ring_lift: f32,
    limit: (f32, f32, f32),
}

/// Oscillator `o`'s pitch offset from its key on panel `p`, semitones: its RANGE, its
/// FREQUENCY's law and TUNE's (none for oscillator 3 free of the keys).
fn pitch_offset(p: &Panel, o: usize) -> f64 {
    let cubic = |c: [f64; 4], x: f64| c[0] + x * (c[1] + x * (c[2] + x * c[3]));
    let range = (2.0 - p.osc[o].range.octaves_below_2()) * 12.0;
    let x = p.osc[o].freq.clamp(0.0, 1.0);
    let free = o == 2 && !p.osc3_control;
    let freq = match o {
        0 => 0.0,
        1 => {
            let at = |x: f64| cubic(laws::OSC2_CENTS, x);
            (at(x) - at(FREQ_CENTRE)) / 100.0
        }
        _ if free => {
            let quintic = |x: f64| {
                laws::OSC3_FREE_CENTS
                    .iter()
                    .rev()
                    .fold(0.0, |a, c| a * x + c)
            };
            (quintic(x) - quintic(FREQ_CENTRE)) / 100.0
        }
        _ => {
            let at = |x: f64| cubic(laws::OSC3_CENTS, x);
            (at(x) - at(FREQ_CENTRE)) / 100.0
        }
    };
    let tune = if free {
        0.0
    } else {
        p.tune * laws::TUNE_CENTS / 100.0
    };
    range + freq + tune
}

/// What the light voice reads of panel `p`'s pitches and CUTOFF, after their knobs' laws:
/// each oscillator's offset from its key, semitones, and CUTOFF's current into the filter's
/// control node, A ([`knob_current`], before the ladder's law with the contour's,
/// [`node_log2`]). The plug-in's LFOs glide these across their steps (decisions.md
/// R-LFO-STRESS): the pitches as ENTROPY's offsets ([`Light::set_offsets`]), the current by
/// [`Light::set_cutoff_swept`].
pub fn swept_laws(p: &Panel) -> ([f64; 3], f64) {
    (
        std::array::from_fn(|o| pitch_offset(p, o)),
        knob_current(p.cutoff.clamp(0.0, 1.0)),
    )
}

impl Light {
    /// A voice at `rate` Hz, at rest, with the default panel.
    pub fn new(rate: f64) -> Light {
        let filter_rate = rate * FILTER_OVERSAMPLE as f64;
        let filter_up = Interpolator::sparse(FILTER_OVERSAMPLE);
        let filter_down = Decimator::sparse(FILTER_OVERSAMPLE);
        let to_ladder = filter_up.delay();
        let filter_delay = to_ladder + filter_down.delay();
        let at = |s: f64| (s * rate).round() as u32;
        let law = ContourLaw::new(Panel::default().filter_contour, true, 0, rate);
        let mut l = Light {
            rate,
            panel: Panel::default(),
            set_for_panel: false,
            mod_u: 0.0,
            shape_now: [None; 3],
            set: Setting::default(),
            held: Held::default(),
            pressed: [0; 128],
            presses: 0,
            waiting: Held::default(),
            gap: at(crate::voice::RETRIGGER_GAP),
            lead: (0, 0),
            lead_samples: at(laws::RELEASE_LEAD),
            target: laws::PITCH_AT_REST,
            pitch: laws::PITCH_AT_REST,
            foot: 0.0,
            beyond: 0.0,
            gate: false,
            trigger: false,
            pending: 0,
            attack: false,
            open: u32::MAX,
            rearm: at(laws::REARM),
            trigger_on: at(laws::TRIGGER_ON),
            trigger_off: at(laws::TRIGGER_OFF),
            osc: [Osc::new(Waveform::Sawtooth); 3],
            dt: [0.0; 3],
            dt_step: [0.0; 3],
            inv: [0.0; 3],
            noise: Noise {
                state: 0x4D6F_6F67,
                pink: [0.0; 3],
                red: 0.0,
                hp: (0.0, 0.0),
                lp: 0.0,
                pink_hp: (0.0, 0.0),
                pink_lp: 0.0,
            },
            noise_c: NoiseLaw::new(rate),
            hiss: (HISS_SEED, (HISS * (rate / 48e3).sqrt()) as f32),
            feedback_delay: (interpolate(&laws::FEEDBACK_AT, &laws::FEEDBACK_DELAY, rate)
                - filter_delay)
                .max(1.0) as f32,
            coupling: (
                (-std::f64::consts::TAU * laws::COUPLING_HZ / filter_rate).exp() as f32,
                0.0,
                0.0,
            ),
            droop: (
                (-std::f64::consts::TAU * laws::DROOP_HZ / rate).exp() as f32,
                0.0,
                0.0,
            ),
            mode: (
                (-std::f64::consts::TAU * laws::MODE_HZ / filter_rate).exp() as f32,
                0.0,
                0.0,
            ),
            direct: 0.0,
            pass: 1.0,
            thump_c: (
                (-1.0 / (laws::THUMP_TIMES.0 * rate)).exp() as f32,
                (1.0 - (-1.0 / (laws::THUMP_TIMES.1 * rate)).exp()) as f32,
            ),
            thump: (0.0, 0.0, 0.0),
            bias: tanh(laws::BIAS),
            ladder: Ladder::default(),
            filter_up,
            filter_down,
            line_up: Interpolator::sparse(LINE_OVERSAMPLE),
            line_live: false,
            ladder_at: (0.0, 0.0),
            g: 0.0,
            g_step: 0.0,
            k: 0.0,
            contours: [Contour::at_rest(&law); 2],
            late_past: [(0.0, 0.0); LATE_PAST],
            late_at: 0,
            // The filter contour as late as its sound at the ladder's input (the input
            // resampler's delay, less the control step's: the ladder's coefficients follow it
            // a step late); the loudness contour reaching the VCA when ULTRA's does once the
            // output is lined up (`voice::LINE_UP`): 46.75 samples.
            late: (
                (to_ladder - (CONTROL_EVERY - 1) as f64).max(0.0),
                (crate::voice::latency(crate::voice::Quality::NoCompromises)
                    - crate::voice::LINE_UP.1 as f64)
                    .max(0.0),
            ),
            filter_volts: 0.0,
            resets: (0.0, 0),
            samples: 0,
            count: 0,
            detune: [0.0; 3],
            cutoff_offset: 0.0,
            cutoff_swept: 0.0,
            cutoff_at: core::array::from_fn(|i| knob_current(i as f64 / 20.0)),
            node_log2: f64::from(laws::CUTOFF_LOG2[20]),
            timed: [None; 3],
            timed_resets: [(0.0, 0); 3],
            osc3: 0.0,
            a440: (0.0, (440.0 / rate) as f32),
            feedback: 0.0,
            fed_back: [0.0; 128],
            fed_at: 0,
            lamp: 0.0,
            ext_hp: (
                (-std::f64::consts::TAU * laws::EXT_COUPLING_HZ / rate).exp(),
                0.0,
                0.0,
            ),
            pre_hp: (
                (-std::f64::consts::TAU * laws::PREAMP_COUPLING_HZ / rate).exp() as f32,
                0.0,
                0.0,
            ),
            overload_peak: 0.0,
            drive: 1.0,
            drive_top: 1.0,
            ring_lift: 1.0,
            limit: (0.0, 0.0, 0.0),
        };
        l.set_drive(1.0);
        // (The oscillators start in phase, as the circuit's do when its voice is made; ENTROPY's
        // floor of mismatch then draws them apart: decisions.md R9, R31.)
        l.set_panel(&Panel::default());
        l.contours = [
            Contour::at_rest(&l.set.contours[0]),
            Contour::at_rest(&l.set.contours[1]),
        ];
        l.settle_late();
        l
    }

    /// The contours' past as they are now (at rest).
    fn settle_late(&mut self) {
        let now = (self.contours[0].volts, self.contours[1].volts);
        self.late_past = [now; LATE_PAST];
        self.filter_volts = now.0;
    }

    /// The contours' outputs this sample (filter, loudness) kept; the filter contour as it
    /// reaches the ladder ([`Light::filter_volts`]) and the loudness contour as it reaches the
    /// VCA, each [`Light::late`] late (interpolated between samples): as late as the sound
    /// each shapes, as the circuit has them (decisions.md R-LATENCY).
    #[inline(always)]
    fn late_contours(&mut self, filter: f64, loud: f64) -> f64 {
        self.late_at = (self.late_at + 1) % LATE_PAST;
        self.late_past[self.late_at] = (filter, loud);
        let at = |late: f64| {
            let (whole, part) = (late as usize, late.fract());
            let k = |k: usize| self.late_past[(self.late_at + LATE_PAST - k) % LATE_PAST];
            (k(whole), k(whole + 1), part)
        };
        let (f0, f1, part) = at(self.late.0);
        self.filter_volts = f0.0 + (f1.0 - f0.0) * part;
        let (l0, l1, part) = at(self.late.1);
        l0.1 + (l1.1 - l0.1) * part
    }

    /// Oscillator `n`'s latest reset while timed (decisions.md R-OSC-TIMED), how long before
    /// the latest sample, exact, and how many there have been (a probe).
    pub fn probe_timed_reset(&self, n: usize) -> (f64, u64) {
        (
            self.samples as f64 - 1.0 - self.timed_resets[n].0,
            self.timed_resets[n].1,
        )
    }

    /// Oscillator 1's latest reset (its phase's wrap) in samples since the voice was made,
    /// exact, and how many there have been (a probe: decisions.md R-LATENCY).
    pub fn probe_resets(&self) -> (f64, u64) {
        self.resets
    }

    pub fn rate(&self) -> f64 {
        self.rate
    }

    /// The plug-in's DRIVE, as [`crate::voice::Voice::set_drive`]: the mixer's signal into the
    /// filter's input pair raised by `gain` (the ladder's feedback not), driving the pair
    /// harder than the panel's mixer can; 1 is the circuit.
    pub fn set_drive(&mut self, gain: f64) {
        let gain = gain.max(1e-3);
        self.drive = gain as f32;
        self.drive_top = gain.powf(laws::DRIVE_CEILING).min(1.0) as f32;
        // (The engine calls this every block: the ring's lift stays in the limit.)
        self.set_limit();
    }

    /// The pair's limit from DRIVE's and the ring's, worked out again if it moved.
    fn set_limit(&mut self) {
        let top = self.drive_top * self.ring_lift;
        if top != self.limit.0 {
            self.limit = (top, 1.0 / top, top * tanh(laws::BIAS / top));
        }
    }

    /// The three oscillators started together `at` one share of the way through their cycles
    /// (from the sawtooth's top), as [`crate::voice::Voice::start_oscillators_at`]: for a voice
    /// made or put back to rest that is not the first, so that voices playing one note
    /// together do not start in step. Never at a note.
    pub fn start_oscillators_at(&mut self, at: f64) {
        let at = at.clamp(0.0, 0.98) as f32;
        for o in &mut self.osc {
            o.phase = at;
        }
    }

    /// Seeds the noise (each voice its own stream).
    pub fn set_seed(&mut self, seed: u64) {
        let s = (seed ^ (seed >> 32)) as u32;
        self.noise.state = if s == 0 { 0x4D6F_6F67 } else { s };
        let h = s ^ HISS_SEED;
        self.hiss.0 = if h == 0 { HISS_SEED } else { h };
    }

    /// The last sample's MOD share of CONTINUOUS's travel (onward), 0..1 (decisions.md R-CONTINUOUS-WAVE).
    pub fn probe_shape_mod(&self) -> f64 {
        f64::from(self.mod_u)
    }

    /// Each oscillator's CONTINUOUS shape where the last sample had it (its switch's steps), MOD
    /// and all; `None` for a STEPPED one.
    pub fn probe_shapes(&self) -> [Option<f64>; 3] {
        self.shape_now
    }

    /// The panel: what it sets is worked out again if it changed (some microseconds: not for
    /// every sample).
    pub fn set_panel(&mut self, p: &Panel) {
        if self.set_for_panel && *p == self.panel {
            return;
        }
        self.set_for_panel = true;
        self.panel = *p;
        // (MOD moves a CONTINUOUS shape only with the wheel forward: R-CONTINUOUS-WAVE.)
        let depth = light_depth(p.mod_wheel);
        for o in 0..3 {
            let (wave, shape) = light_shape(o, p, depth);
            self.shape_now[o] = p.shape[o]
                .continuous
                .then(|| crate::voice::shape_at(p.shape[o], crate::voice::ModSwing::default()));
            self.osc[o].wave = wave;
            self.osc[o].shape = shape;
            self.osc[o].third = o == 2;
            let bias = match (p.shape[o].continuous, shape) {
                (false, _) => wave.width(),
                (true, _) => crate::voice::shape_bias(crate::voice::shape_at(
                    p.shape[o],
                    crate::voice::ModSwing::default(),
                )),
            };
            (self.osc[o].duty, self.osc[o].level) = rect_law(bias);
            self.set.offset[o] = pitch_offset(p, o);
            let source = match wave {
                Waveform::Triangle => laws::SOURCE_OHMS[0],
                Waveform::Sawtooth | Waveform::ReverseSawtooth => laws::SOURCE_OHMS[1],
                Waveform::SharkTooth => laws::SOURCE_OHMS[3],
                _ => laws::SOURCE_OHMS[2],
            };
            let level = if o == 2 { laws::OSC3_LEVEL } else { 1.0 };
            let volume = volume_track(p.osc[o].volume);
            // (Blending, each place's level through the mixer against the sawtooth's, the
            // gain the sawtooth's: CONTINUOUS's places' sources, R-CONTINUOUS-WAVE.)
            let (source, ratios) = match shape {
                None => (source, [1.0; 4]),
                Some(_) => {
                    let at = |ohms: f64| {
                        channel(volume, ohms, 33e3) / channel(volume, laws::SOURCE_OHMS[1], 33e3)
                    };
                    (
                        laws::SOURCE_OHMS[1],
                        [
                            at(laws::SOURCE_OHMS[0]),
                            at(laws::SOURCE_OHMS[3]),
                            1.0,
                            at(laws::SOURCE_OHMS[2]),
                        ],
                    )
                }
            };
            self.osc[o].ratios = ratios;
            self.set.gain[o] = if p.osc[o].on {
                channel(volume, source, 33e3) * level
            } else {
                0.0
            };
        }
        self.set.follows = [true, true, p.osc3_control];
        self.set.noise = if p.noise_on {
            let (ohms, level) = if p.noise_pink {
                (laws::NOISE_OHMS.1, laws::PINK)
            } else {
                (laws::NOISE_OHMS.0, laws::WHITE)
            };
            channel(volume_track(p.noise_volume), ohms, 11e3) / channel(1.0, ohms, 11e3) * level
        } else {
            0.0
        };
        // (The external input works at the output's level; the bus's is that over OUT and
        // DRIVE through the ladder's passband at EMPHASIS 0.)
        // (The pair's slope at its offset.)
        let slope = 1.0 - self.bias * self.bias;
        let passband = f64::from(laws::OUT * laws::DRIVE * slope)
            / (1.0 + laws::LOOP[3][0] * laws::LOOP_SCALE);
        // FILTER MODE HI's direct branch: the open ladder's passband at EMPHASIS 0, as the
        // circuit's is the same at every cutoff (vcf.rs's `MODE_RT`).
        self.direct = laws::MODE_GAIN * laws::DRIVE * slope
            / (1.0 + (laws::LOOP[9][0] * laws::LOOP_SCALE) as f32);
        self.set.ext = if p.ext_on {
            (1.0 / passband) as f32
        } else {
            0.0
        };
        let t = ext_taper(p.ext_volume);
        self.set.ext_gain = laws::EXT_GAIN * t / (1.0 + laws::EXT_LOAD * t * (1.0 - t));
        self.set.cutoff = knob_current(p.cutoff.clamp(0.0, 1.0));
        self.set.tracking = match (p.keyboard_control_1, p.keyboard_control_2) {
            (false, false) => 0.0,
            (true, false) => laws::KEYBOARD_CONTROL[0],
            (false, true) => laws::KEYBOARD_CONTROL[1],
            (true, true) => laws::KEYBOARD_CONTROL[2],
        };
        self.set.contour = contour_current(p.contour_amount);
        // (EMPHASIS as the knob drawn with its generic taper would set R14.)
        let emphasis = 1.0 - (1.0 + 80.0 * emphasis_r14(p.emphasis) / 50e3).ln() / 81f64.ln();
        self.set.corner = ((0.744 - emphasis) / 0.044).clamp(0.0, 1.0);
        for ((k, row), &log2) in self
            .set
            .emphasis
            .iter_mut()
            .zip(&laws::LOOP)
            .zip(&laws::LOOP_AT_LOG2)
        {
            let threshold = interpolate(&laws::THRESHOLD_AT_LOG2, &laws::THRESHOLD, log2);
            let shift = interpolate(&laws::THRESHOLD_AT_LOG2, &laws::SHIFT, log2);
            let near = 1.0 - ((threshold - emphasis) / laws::EMPHASIS_NEAR).clamp(0.0, 1.0);
            let at = interpolate(&laws::EMPHASIS_AT, row, emphasis + near * shift);
            // (Past the threshold, `RING_GAIN` of the rise beyond it.)
            *k = laws::LOOP_SCALE
                * if at > 4.0 {
                    4.0 + (at - 4.0) * laws::RING_GAIN
                } else {
                    at
                };
        }
        // (The wheel as the drawing's would set its resistance.)
        let r = crate::modulation::mod_wheel_r(p.mod_wheel);
        self.set.depth = depth;
        self.set.depth_full = light_depth(1.0) as f32;
        self.set.offset_depth = laws::OFFSET.0 * r / (r + laws::OFFSET.1);
        self.set.mix = p.mod_mix.clamp(0.0, 1.0) as f32;
        self.set.osc_mod = p.osc_mod;
        self.set.filter_mod = p.filter_mod;
        let w3 = self.osc[2].wave;
        self.set.swing = 1.0 / self.osc[2].peak();
        let (kind, duty) = match w3 {
            Waveform::Triangle => (0, None),
            Waveform::ReverseSawtooth | Waveform::SharkTooth => (1, None),
            Waveform::Sawtooth => (2, None),
            Waveform::Square | Waveform::WideRectangle | Waveform::NarrowRectangle => {
                (3, Some(self.osc[2].duty))
            }
        };
        self.set.shape_mod = std::array::from_fn(|o| p.shape_mod_on(o));
        self.set.swing_bias = duty.map_or(0.0, |d| 2.0 * d - 1.0);
        self.set.mod_swing = laws::MOD_SWING[kind] as f32;
        self.set.mod_centre = laws::MOD_CENTRE[kind] as f32;
        // (GLIDE off, the hold's own 330 ohm: at once.)
        let ohms = glide_r(p.glide, p.glide_on) + 330.0;
        let rate = if p.glide_long {
            laws::GLIDE_RATE / 5.0
        } else {
            laws::GLIDE_RATE
        };
        let farads = if p.glide_long {
            laws::GLIDE_FARADS * 5.0
        } else {
            laws::GLIDE_FARADS
        };
        let step = rate / ohms * CONTROL_EVERY as f64 / self.rate;
        self.set.glide = (step, step * laws::GLIDE_FALLING);
        self.set.beyond = (-(CONTROL_EVERY as f64 / self.rate) / (ohms * farads)).exp();
        self.set.contours = [
            ContourLaw::new(p.filter_contour, p.decay, 0, self.rate),
            ContourLaw::new(p.loudness_contour, p.decay, 1, self.rate),
        ];
    }

    /// ENTROPY's offsets for the samples that follow: each oscillator's cents, the cutoff's
    /// octaves.
    pub fn set_offsets(&mut self, detune: [f64; 3], cutoff: f64) {
        self.detune = detune;
        self.cutoff_offset = cutoff;
    }

    /// The plug-in's LFOs' glide of CUTOFF for the samples that follow (decisions.md
    /// R-LFO-STRESS): its current into the control node since their step, A, added to the
    /// panel's before the ladder's law, as the contour's is. 0: the panel's, to the bit.
    pub fn set_cutoff_swept(&mut self, current: f64) {
        self.cutoff_swept = current;
    }

    /// The oscillators timed, for the samples that follow (decisions.md R-OSC-TIMED): where
    /// each one's cycle stands on the host's clock, its RANGE at LO; none, its own.
    pub fn set_timed(&mut self, timed: [Option<Timed>; 3]) {
        self.timed = timed;
    }

    /// Oscillator `n` timed for one sample (decisions.md R-OSC-TIMED): the step, worked out in
    /// double precision, that puts it where the host's clock has it (its own phase single),
    /// none of the pitch's inputs in it; further off than the tolerance (the clock jumped), put
    /// there at once.
    fn timed_tick(&mut self, n: usize, t: Timed) -> f32 {
        let per = if t.per.is_finite() {
            t.per.clamp(1e-12, 0.25)
        } else {
            1e-12
        };
        let now = if t.phase.is_finite() {
            t.phase.rem_euclid(1.0)
        } else {
            0.0
        };
        let osc = &mut self.osc[n];
        let mut step = now - f64::from(osc.phase);
        step -= step.round();
        if (step - per).abs() > crate::vco::TIMED_TOLERANCE {
            // (Where the clock was a sample before; a whole cycle there is its start, just past
            // its wrap.)
            let before = (now - per).rem_euclid(1.0);
            osc.phase = if before < 1.0 {
                (before as f32).min(1.0 - f32::EPSILON / 2.0)
            } else {
                0.0
            };
            step = per;
        }
        let step = step.clamp(per * 1e-3, 0.45) as f32;
        let before = osc.phase;
        let w = osc.tick(step, 1.0 / step);
        if osc.phase < before {
            let at = (self.samples - 1) as f64 - f64::from(osc.phase / step);
            self.timed_resets[n] = (at, self.timed_resets[n].1 + 1);
        }
        w
    }

    /// A MIDI note pressed or released.
    pub fn note(&mut self, midi: i32, on: bool) {
        if !(0..128).contains(&midi) {
            return;
        }
        let bit = 1u128 << midi;
        if on && (self.held.0 | self.waiting.0) & bit == 0 {
            self.presses = self.presses.wrapping_add(1);
            self.pressed[midi as usize] = self.presses;
            if self.panel.retrigger {
                // (Down once the keys' gate has been low long enough: `tick`.)
                self.waiting.set(midi, true);
                return;
            }
        }
        if !on && self.waiting.0 & bit != 0 {
            // (Let go before it came down: it never sounded.)
            self.waiting.set(midi, false);
            return;
        }
        if !on && self.sounding() == Some(midi) {
            self.lead = (midi, self.lead_samples);
        }
        self.held.set(midi, on);
    }

    /// The key held that sounds, as the panel's priority chooses it.
    fn sounding(&self) -> Option<i32> {
        match self.panel.priority {
            Priority::Low => self.held.lowest(),
            Priority::High => self.held.highest(),
            Priority::Last => self.held.newest(&self.pressed),
        }
    }

    /// Whether a key is held (or pressed and waiting to come down).
    pub fn held(&self) -> bool {
        self.held.0 != 0 || self.waiting.0 != 0
    }

    /// As [`crate::voice::Voice::trigger_closed`]: the keys' gate at the last sample.
    pub fn trigger_closed(&self) -> bool {
        self.gate
    }

    /// As [`crate::voice::Voice::slow_state`]: the pitch (as volts, an octave a volt) and
    /// the contours, V, once no key is down and the trigger has followed.
    pub fn slow_state(&self) -> Option<[f64; 3]> {
        if self.held() || self.trigger || self.pending > 0 {
            return None;
        }
        Some([
            (self.pitch + self.beyond) / 12.0,
            self.contours[0].volts,
            self.contours[1].volts,
        ])
    }

    /// The contours (filter, loudness), V.
    pub fn contours(&self) -> (f64, f64) {
        (self.contours[0].volts, self.contours[1].volts)
    }

    /// The OVERLOAD lamp now (0 off, 1 fully lit).
    pub fn overload(&self) -> f64 {
        self.lamp
    }

    /// The OVERLOAD lamp's brightest since the last call.
    pub fn take_overload_peak(&mut self) -> f64 {
        std::mem::take(&mut self.overload_peak)
    }

    /// Back to rest: every key up, the contours and the ladder at rest (All Sound Off).
    pub fn rest(&mut self) {
        self.held = Held::default();
        self.waiting = Held::default();
        self.lead = (0, 0);
        (self.gate, self.trigger, self.pending, self.open) = (false, false, 0, u32::MAX);
        self.contours = [
            Contour::at_rest(&self.set.contours[0]),
            Contour::at_rest(&self.set.contours[1]),
        ];
        self.settle_late();
        self.ladder = Ladder::default();
        self.filter_up.reset();
        self.filter_down.reset();
        self.line_up.reset();
        self.line_live = false;
        self.coupling = (self.coupling.0, 0.0, 0.0);
        self.droop = (self.droop.0, 0.0, 0.0);
        self.mode = (self.mode.0, 0.0, 0.0);
        self.thump = (0.0, 0.0, 0.0);
        self.ext_hp = (self.ext_hp.0, 0.0, 0.0);
        self.pre_hp = (self.pre_hp.0, 0.0, 0.0);
        self.fed_back = [0.0; 128];
        // (In phase again, as the circuit's clean voice is.)
        for o in &mut self.osc {
            o.phase = 0.0;
        }
        self.pitch = self.target;
        self.beyond = self.foot;
    }

    /// The control laws, every [`CONTROL_EVERY`] samples: the pitch (GLIDE), the
    /// oscillators' increments and the ladder's coefficients for the next samples.
    fn control(&mut self) {
        let sounding = self.sounding();
        let sounding = if self.lead.1 > 0 {
            self.lead.1 = self.lead.1.saturating_sub(CONTROL_EVERY as u32);
            let lead = self.lead.0;
            sounding.map(|k| match self.panel.priority {
                Priority::Low => k.min(lead),
                Priority::High => k.max(lead),
                Priority::Last if self.pressed[lead as usize] > self.pressed[k as usize] => lead,
                Priority::Last => k,
            })
        } else {
            sounding
        };
        if let Some(key) = sounding {
            let (lo, hi) = laws::KEYBOARD_KEYS;
            let key = f64::from(key);
            self.target = key.clamp(lo, hi);
            self.foot = key - self.target;
            // (The track-and-hold follows while a key is down.)
            let (up, down) = self.set.glide;
            self.pitch += (self.target - self.pitch).clamp(-down, up);
            self.beyond = self.foot + (self.beyond - self.foot) * self.set.beyond;
        }
        let pitch = self.pitch + self.beyond;
        let wheel = self.panel.pitch_wheel * crate::modulation::PITCH_WHEEL_SEMITONES;
        for o in 0..3 {
            let key = if self.set.follows[o] {
                pitch + wheel
            } else {
                laws::OSC3_FREE_KEY
            };
            let semis = key - 69.0 + self.set.offset[o] + self.detune[o] / 100.0;
            let hz = laws::A4 * f64::from(exp2((semis / 12.0) as f32));
            let to = ((hz / self.rate) as f32).min(0.45);
            self.dt_step[o] = (to - self.dt[o]) / CONTROL_EVERY as f32;
            self.inv[o] = 1.0 / to.max(1e-9);
        }
        let knob = if self.cutoff_swept != 0.0 {
            self.set.cutoff + self.cutoff_swept
        } else {
            self.set.cutoff
        };
        self.node_log2 = node_log2(&self.cutoff_at, knob + self.set.contour * self.filter_volts);
        let (g, k) = self.coefficients(0.0);
        self.g_step = (g - self.g) / CONTROL_EVERY as f32;
        self.k = k;
        let log2 = self.node_log2
            + self.set.tracking * (self.pitch + self.beyond - laws::TRACKING_FROM) / 12.0
            + self.cutoff_offset;
        // The passband at this cutoff.
        let pass = interpolate(&laws::PASS_AT_LOG2, &laws::PASS_DB, log2);
        self.pass = f64::from(exp2((pass * (std::f64::consts::LOG2_10 / 20.0)) as f32)) as f32;
        // Past the threshold, the pair's limit for the ring's level at this cutoff.
        let ringing = 1.0 - self.set.corner;
        self.ring_lift = if ringing > 0.0 {
            let lift = interpolate(&laws::RING_LEVEL_AT_LOG2, &laws::RING_LEVEL, log2);
            (1.0 + ringing * (lift - 1.0)) as f32
        } else {
            1.0
        };
        self.set_limit();
    }

    /// The ladder's stage gain and loop gain for the cutoff the panel, the keys and the
    /// filter contour set, `extra` octaves added.
    #[inline(always)]
    fn coefficients(&self, extra: f64) -> (f32, f32) {
        let s = &self.set;
        let log2 = self.node_log2
            + s.tracking * (self.pitch + self.beyond - laws::TRACKING_FROM) / 12.0
            + self.cutoff_offset
            + extra;
        let corner = interpolate(&laws::CORNER_AT_LOG2, &laws::CORNER, log2) * s.corner
            + interpolate(&laws::RING_RAISE_AT_LOG2, &laws::RING_RAISE, log2) * (1.0 - s.corner);
        let frequency_log = log2 + corner;
        let frequency_log = if frequency_log > laws::CUTOFF_SAT_AT[0] {
            interpolate(&laws::CUTOFF_SAT_AT, &laws::CUTOFF_SAT, frequency_log)
        } else {
            frequency_log
        };
        let hz = f64::from(exp2(frequency_log as f32));
        let w = (std::f64::consts::PI * hz / (self.rate * FILTER_OVERSAMPLE as f64)).min(1.4);
        // tan by its Padé form, close enough below 1.4 for the ladder's tuning.
        let w2 = w * w;
        let t = w * (15.0 - w2) / (15.0 - 6.0 * w2);
        let t = if t.is_finite() && t > 0.0 { t } else { 1e3 };
        // The loop's gain at this cutoff, between the fitted ones.
        let k = interpolate(&laws::LOOP_AT_LOG2, &s.emphasis, log2);
        ((t / (1.0 + t)) as f32, k as f32)
    }

    /// One step of the ladder at stage gain `g` and loop gain `k`, its input `bus` at the
    /// filter's rate, through to its decimator: an output sample at the voice's rate, when one
    /// falls due.
    #[inline(always)]
    fn ladder_step(&mut self, bus: f32, g: f32, k: f32) -> Option<f64> {
        let y = self.ladder.tick(
            laws::DRIVE * self.drive * bus,
            g.clamp(0.0, 0.999),
            k,
            laws::BIAS,
            self.limit,
        );
        // The filter's output coupling; FILTER MODE HI, the bus (through its own coupling)
        // less it.
        let y = y * self.pass;
        let (a, x0, y0) = self.coupling;
        let lo = a * (y0 + y - x0);
        self.coupling = (a, y, lo);
        let y = if self.panel.filter_hi {
            let (a, x0, y0) = self.mode;
            let d = self.direct * bus;
            let direct = a * (y0 + d - x0);
            self.mode = (a, d, direct);
            direct - lo
        } else {
            // (The direct branch's coupling at rest: in HI it starts from the bus's sample.)
            self.mode.1 = 0.0;
            self.mode.2 = 0.0;
            lo
        };
        self.filter_down.push(f64::from(y))
    }

    /// One output sample (1.0 = 5 V), `ext` at EXTERNAL INPUT.
    #[inline]
    pub fn tick(&mut self, ext: f64) -> f64 {
        if self.count == 0 {
            self.count = CONTROL_EVERY;
            self.control();
        }
        self.count -= 1;
        // RETRIGGER's keys come down once the keys' gate has been low long enough for the
        // contours to start again; until then it is low, the keys held as they were.
        if self.waiting.0 != 0 && !self.gate && self.open >= self.gap {
            self.held.0 |= self.waiting.0;
            self.waiting = Held::default();
        }
        // The trigger: the contours follow the keys' gate late, and a key pressed again
        // within the re-arming time does not start them again.
        let gate = self.held.0 != 0 && self.waiting.0 == 0;
        if gate != self.gate {
            self.gate = gate;
            if gate {
                self.attack = self.open >= self.rearm;
                self.pending = if self.attack {
                    // (Re-armed: an attack, even if the contours have not yet fallen.)
                    self.trigger_on.max(1)
                } else if self.trigger {
                    // (Let go and pressed again before the contours fell: they hold.)
                    0
                } else {
                    1
                };
            } else {
                self.pending = self.trigger_off.max(1);
            }
            self.open = 0;
        }
        if !gate {
            self.open = self.open.saturating_add(1);
        }
        if self.pending > 0 {
            self.pending -= 1;
            if self.pending == 0 {
                self.trigger = gate;
                if gate {
                    for c in &mut self.contours {
                        c.attacking = self.attack;
                    }
                }
            }
        }
        let trigger = self.trigger;
        let filter = self.contours[0].tick(trigger, &self.set.contours[0]);
        let loud = self.contours[1].tick(trigger, &self.set.contours[1]);
        let loud = self.late_contours(filter, loud);

        // (What the panel sets, read as needed: not the whole of it copied each sample.)
        let s = &self.set;
        let (gain, noise_gain, ext_bus, ext_gain) = (s.gain, s.noise, s.ext, s.ext_gain);
        let (depth, offset_depth, mix_k) = (s.depth, s.offset_depth, s.mix);
        let (swing, swing_bias, mod_swing, mod_centre) =
            (s.swing, s.swing_bias, s.mod_swing, s.mod_centre);
        let (osc_mod, filter_mod, follows3) = (s.osc_mod, s.filter_mod, s.follows[2]);
        let moved = s.shape_mod;
        let any_mod = moved[0] || moved[1] || moved[2];
        for o in 0..3 {
            self.dt[o] += self.dt_step[o];
        }
        let mut dt = self.dt;
        let mut inv = self.inv;
        let mut g = self.g + self.g_step;
        self.g = g;
        let mut k = self.k;
        // (What the line adds to them each ladder's step, through each half of this sample.)
        let mut ramp = [(0.0f32, 0.0f32); LINE_OVERSAMPLE];
        let modulating = depth > 0.0 && (osc_mod || filter_mod || any_mod);
        let needs_noise = noise_gain != 0.0 || (modulating && mix_k > 0.0);
        let (white, pink, red) = if needs_noise {
            self.noise.tick(&self.noise_c)
        } else {
            (0.0, 0.0, 0.0)
        };
        if !(modulating && any_mod) {
            self.mod_u = 0.0;
        }
        if self.line_live && !(modulating && filter_mod) {
            self.line_live = false;
            self.line_up.reset();
        }
        if modulating {
            // Oscillator 3 (its swing to ±1) and the noise (its RMS to 1) as MODULATION MIX
            // blends them, and the line's offset, through the wheel.
            let (n, rms) = if self.panel.noise_pink {
                (red, 2.163)
            } else {
                (pink, 0.425)
            };
            // (Oscillator 3's semitones at the wheel fully forward.)
            let o = (self.osc3 * swing + swing_bias) * mod_swing + mod_centre;
            let mix = f64::from(mix_k);
            let depth = depth as f32;
            let offset = offset_depth as f32;
            if osc_mod {
                let semis = depth
                    * ((1.0 - mix_k) * o + mix_k * n * (laws::NOISE_SEMITONES_RMS / rms) as f32)
                    + offset
                        * ((1.0 - mix) * laws::MOD_OFFSET.0 + mix * laws::NOISE_OFFSET.0) as f32;
                let r = exp2(semis * (1.0 / 12.0));
                let ri = 1.0 / r;
                for o in 0..if follows3 { 3 } else { 2 } {
                    dt[o] = (dt[o] * r).min(0.45);
                    inv[o] *= ri;
                }
            }
            if any_mod {
                // MOD (decisions.md R-CONTINUOUS-WAVE): the line's swing at the wheel fully
                // forward against oscillator 3's triangle's, as deep as the wheel is forward
                // (the full voice's `ModSwing`).
                let swing =
                    (1.0 - mix_k) * o + mix_k * n * (laws::NOISE_SEMITONES_RMS / rms) as f32;
                let m = crate::voice::ModSwing {
                    depth: f64::from(depth / self.set.depth_full),
                    swing: shape_mod_swing(swing),
                };
                self.mod_u = m.share(crate::voice::ModDirection::Onward) as f32;
                let p = &self.panel;
                for (((osc, on), shape), now) in self
                    .osc
                    .iter_mut()
                    .zip(moved)
                    .zip(p.shape)
                    .zip(self.shape_now.iter_mut())
                {
                    if on {
                        let at = crate::voice::shape_at(shape, m);
                        osc.shape = Some(at as f32);
                        *now = Some(at);
                        (osc.duty, osc.level) = rect_law(crate::voice::shape_bias(at));
                    }
                }
            }
            if filter_mod {
                let octaves = depth
                    * ((1.0 - mix_k) * o * laws::FILTER_PER_SEMITONE as f32
                        + mix_k * n * (laws::NOISE_OCTAVES_RMS / rms) as f32)
                    + offset
                        * ((1.0 - mix) * laws::MOD_OFFSET.1 + mix * laws::NOISE_OFFSET.1) as f32;
                // Use the same bounded cutoff law for audio-rate modulation and contours: at
                // twice the voice's rate, or what its exponential adds to a fast line folds
                // (decisions.md R-LO-FILTER-FM). The bus takes longer to reach the ladder than
                // the line takes through this, so no latency comes of it.
                if !self.line_live {
                    self.line_live = true;
                    self.ladder_at = (g, k);
                }
                let mut line = [0.0; LINE_OVERSAMPLE];
                self.line_up.push(f64::from(octaves), &mut line);
                (g, k) = self.ladder_at;
                let steps = (FILTER_OVERSAMPLE / LINE_OVERSAMPLE) as f32;
                for (half, at) in ramp.iter_mut().zip(line) {
                    let (from_g, from_k) = self.ladder_at;
                    self.ladder_at = self.coefficients(at);
                    *half = (
                        (self.ladder_at.0 - from_g) / steps,
                        (self.ladder_at.1 - from_k) / steps,
                    );
                }
            }
        }
        // The oscillators.
        // (Each oscillator's increment's reciprocal the control step's: its edges' places in
        // samples exact at a steady pitch, off by the step's change while it moves.)
        self.samples += 1;
        let (timed0, timed1, timed2) = (self.timed[0], self.timed[1], self.timed[2]);
        let w0 = if gain[0] != 0.0
            && let Some(t) = timed0
        {
            self.timed_tick(0, t)
        } else if gain[0] != 0.0 {
            let before = self.osc[0].phase;
            let w = self.osc[0].tick(dt[0], inv[0]);
            if self.osc[0].phase < before {
                // (It wrapped this sample, its phase now so many increments past.)
                let at = (self.samples - 1) as f64 - f64::from(self.osc[0].phase / dt[0]);
                self.resets = (at, self.resets.1 + 1);
            }
            w
        } else {
            0.0
        };
        let w1 = if gain[1] != 0.0 {
            match timed1 {
                Some(t) => self.timed_tick(1, t),
                None => self.osc[1].tick(dt[1], inv[1]),
            }
        } else {
            0.0
        };
        // (Oscillator 3 also as the modulation's source; neither heard nor modulating, its
        // phase rests, as an unheard free-running oscillator's may.)
        let w2 = if gain[2] != 0.0 || modulating {
            match timed2 {
                Some(t) => self.timed_tick(2, t),
                None => self.osc[2].tick(dt[2], inv[2]),
            }
        } else {
            0.0
        };
        self.osc3 = w2;
        let noise = if self.panel.noise_pink { pink } else { white };
        // EXTERNAL INPUT through its preamplifier (FEEDBACK's cable added), at the output's
        // level.
        let ext_in = if self.feedback > 0.0 {
            let d = self.feedback_delay;
            let back = |n: usize| self.fed_back[(self.fed_at + 128 - n) & 127];
            let (n, f) = (d as usize, d.fract());
            ext + self.feedback * f64::from(back(n) + (back(n + 1) - back(n)) * f)
        } else {
            ext
        };
        let mut pre = 0.0f32;
        // (Through its coupling, while anything is in it.)
        let ext_in = if ext_in != 0.0 || self.ext_hp.2 != 0.0 {
            let (a, x0, y0) = self.ext_hp;
            let y = a * (y0 + ext_in - x0);
            let y = if y.abs() < 1e-15 && ext_in == 0.0 {
                0.0
            } else {
                y
            };
            self.ext_hp = (a, ext_in, y);
            y
        } else {
            0.0
        };
        if ext_in != 0.0 {
            let x = ext_in * ext_gain;
            let (on, full) = laws::OVERLOAD;
            let lamp = ((x.abs() - on) / (full - on)).clamp(0.0, 1.0);
            self.lamp = lamp;
            self.overload_peak = self.overload_peak.max(lamp);
            pre = tanh((x / laws::EXT_CLIP) as f32) * laws::EXT_CLIP as f32;
        }
        // (The preamplifier's output coupling, while anything is in it.)
        if pre != 0.0 || self.pre_hp.2 != 0.0 {
            let (a, x0, y0) = self.pre_hp;
            let y = a * (y0 + pre - x0);
            let y = if y.abs() < 1e-12 && pre == 0.0 {
                0.0
            } else {
                y
            };
            self.pre_hp = (a, pre, y);
            pre = y;
        }
        let hiss = {
            let mut x = self.hiss.0;
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            self.hiss.0 = x;
            (x as i32 as f32) * (self.hiss.1 / 2_147_483_648.0)
        };
        let bus =
            gain[0] * w0 + gain[1] * w1 + gain[2] * w2 + noise_gain * noise + ext_bus * pre + hiss;
        let mut inputs = [0.0; FILTER_OVERSAMPLE];
        self.filter_up.push(f64::from(bus), &mut inputs);
        let mut filtered = 0.0;
        if self.line_live {
            for (step, input) in inputs.iter().enumerate() {
                let (more_g, more_k) = ramp[step * LINE_OVERSAMPLE / FILTER_OVERSAMPLE];
                g += more_g;
                k += more_k;
                filtered = self.ladder_step(*input as f32, g, k).unwrap_or(filtered);
            }
        } else {
            // (Their ramp is nothing: g and k stand, as the plain ladder's sums can use.)
            for input in &inputs {
                filtered = self.ladder_step(*input as f32, g, k).unwrap_or(filtered);
            }
        }
        let y = filtered as f32;
        // The VCA on the loudness contour.
        let g = &laws::VCA_GAIN;
        let v = (loud.max(0.0) * (1.0 / laws::VCA_STEP)) as f32;
        let i = (v as usize).min(g.len() - 2);
        let f = (v - i as f32).min(1.0);
        let gain = g[i] + (g[i + 1] - g[i]) * f;
        // The output's coupling (the rest of the waveforms' droop).
        let y = y * laws::OUT * gain;
        let (a, x0, y0) = self.droop;
        let coupled = a * (y0 + y - x0);
        self.droop = (a, y, coupled);
        // The VCA's thump: its gain through the band-pass.
        let ((ha, lb), (g0, h0, l0)) = (self.thump_c, self.thump);
        let h = ha * (h0 + gain - g0);
        let l = l0 + lb * (h - l0);
        self.thump = (gain, h, l);
        let mut out = f64::from(coupled + laws::THUMP * l);
        if self.panel.a440 {
            let (phase, step) = self.a440;
            let p = phase + step;
            let p = if p >= 1.0 { p - 1.0 } else { p };
            self.a440.0 = p;
            let at = p * 64.0;
            let j = (at as usize).min(63);
            let f = at - j as f32;
            let (a, b) = (A440_SHAPE[j], A440_SHAPE[(j + 1) % 64]);
            out += f64::from((a + (b - a) * f) * laws::A440_LEVEL);
        }
        self.fed_back[self.fed_at] = out as f32;
        self.fed_at = (self.fed_at + 1) & 127;
        out
    }
}

/// `n` samples of one oscillator alone at `rate`, `hz`, its waveform at its level (the lab's
/// aliasing measurement, `ca72-lab alias`; decisions.md R-BLEP).
pub fn oscillator_alone(rate: f64, hz: f64, wave: Waveform, n: usize) -> Vec<f32> {
    let mut o = Osc::new(wave);
    let dt = (hz / rate) as f32;
    (0..n).map(|_| o.tick(dt, 1.0 / dt)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverse_hybrid_keeps_the_same_calibrated_network_in_lo() {
        for dt in [0.003_f32, 0.071_f32] {
            let mut tri = Osc::new(Waveform::Triangle);
            let mut saw = Osc::new(Waveform::Sawtooth);
            let mut hybrid = Osc::new(Waveform::Triangle);
            hybrid.third = true;
            hybrid.shape = Some(1.0);
            for _ in 0..4096 {
                let triangle = f64::from(tri.tick(dt, 1.0 / dt));
                let reverse = -f64::from(saw.tick(dt, 1.0 / dt))
                    * f64::from(laws::REVERSE_SAWTOOTH / laws::SAWTOOTH);
                let mix = f64::from(hybrid.tick(dt, 1.0 / dt));
                let current = (triangle - mix)
                    / (750.0 * 8200.0 / 8950.0 + crate::voice::SHARK_TRIANGLE_OHMS)
                    + (reverse - mix) / (4300.0 * 4700.0 / 9000.0 + crate::voice::SHARK_SAW_OHMS);
                assert!(current.abs() < 1e-10, "LO reverse mix KCL {current}");
            }
        }
    }

    /// The fast functions follow the true ones closely enough for the model.
    #[test]
    fn the_fast_functions_follow() {
        for i in -2000..2000 {
            let x = i as f32 * 0.01;
            let t = tanh(x);
            assert!((t - x.tanh()).abs() < 0.025, "tanh({x}) = {t}");
            assert_eq!(tanh(-x), -t);
            let e = exp2(x);
            assert!((e / x.exp2() - 1.0).abs() < 1e-6, "exp2({x}) = {e}");
        }
    }

    /// The pair must solve its actual feedback, including its knee and stored charge.
    #[test]
    fn the_ladder_solves_its_nonlinear_loop() {
        let _ = crate::unconverged::take();
        for g in [0.05, 0.2, 0.5, 0.9, 0.999] {
            for k in [0.0, 2.0, 7.5] {
                for top in [0.78, 1.0] {
                    for x in [-10.0, -1.0, 0.0, 1.0, 10.0] {
                        let before = [0.2, -0.4, 0.6, -0.8];
                        let mut ladder = Ladder { s: before };
                        let rest = (1.0 - g)
                            * (g * (g * (g * before[0] + before[1]) + before[2]) + before[3]);
                        let inv = 1.0 / top;
                        let bias = top * tanh(laws::BIAS * inv);
                        let out = ladder.tick(x, g, k, laws::BIAS, (top, inv, bias));
                        // Recover the input pair's solution from the first stage's charge.
                        let y = (ladder.s[0] - before[0]) / (2.0 * g) + before[0];
                        let residual = y
                            - (top * tanh((x - k * (g.powi(4) * y + rest) + laws::BIAS) * inv)
                                - bias);
                        assert!(
                            out.is_finite() && residual.abs() < 5e-6,
                            "g={g}, k={k}, top={top}, x={x}: residual {residual}"
                        );
                    }
                }
            }
        }
        assert!(crate::unconverged::take().is_empty(), "LO solver cap");
    }

    /// A key sounds at its pitch, and silence follows its release.
    #[test]
    fn a_note_sounds_and_ends() {
        let mut l = Light::new(48_000.0);
        l.set_panel(&Panel {
            cutoff: 1.0,
            decay: false,
            ..Panel::default()
        });
        for _ in 0..4800 {
            assert!(l.tick(0.0).abs() < 1e-9);
        }
        l.note(57, true);
        // (The contours start 7.8 ms after the key, as the circuit's do.)
        for _ in 0..4800 {
            l.tick(0.0);
        }
        let mut peak = 0.0f64;
        let mut crossings = 0;
        let mut last = 0.0;
        for _ in 0..48_000 {
            let y = l.tick(0.0);
            assert!(y.is_finite());
            peak = peak.max(y.abs());
            if last <= 0.0 && y > 0.0 {
                crossings += 1;
            }
            last = y;
        }
        assert!(peak > 0.05, "silent: {peak}");
        assert!(
            (219..=221).contains(&crossings),
            "{crossings} cycles in a second at 220 Hz"
        );
        l.note(57, false);
        // (Two seconds: the VCA's thump has gone too.)
        for _ in 0..96_000 {
            l.tick(0.0);
        }
        assert!(l.tick(0.0).abs() < 1e-6);
        assert!(l.slow_state().is_some());
    }

    /// The panel the laws are checked on: oscillator 1's sawtooth alone at VOLUME 3, the
    /// filter open, the loudness contour held at its top (decisions.md R31's measurement
    /// panel).
    fn measuring() -> Panel {
        let mut p = Panel {
            cutoff: 1.0,
            contour_amount: 0.0,
            keyboard_control_1: false,
            ..Panel::default()
        };
        p.osc[0].volume = 0.3;
        p.loudness_contour = ContourKnobs {
            attack: 0.0,
            decay: 0.5,
            sustain: 1.0,
        };
        p
    }

    /// `n` samples' output.
    fn run(l: &mut Light, n: usize) -> Vec<f64> {
        (0..n).map(|_| l.tick(0.0)).collect()
    }

    /// The frequency of `x` from its rising zero crossings, Hz at 48 kHz.
    fn hz(x: &[f64]) -> f64 {
        let ups: Vec<f64> = (1..x.len())
            .filter(|&i| x[i - 1] <= 0.0 && x[i] > 0.0)
            .map(|i| i as f64 - x[i] / (x[i] - x[i - 1]))
            .collect();
        (ups.len() - 1) as f64 * 48_000.0 / (ups[ups.len() - 1] - ups[0])
    }

    /// An audio-rate line through the cutoff's law folds nothing near the sawtooth's level
    /// (decisions.md R-LO-FILTER-FM): oscillator 3 two octaves above oscillator 1 on the same
    /// key, so that all its true products are the key's harmonics, and foldover is what lies
    /// at the rate less a whole number of them.
    #[test]
    fn a_fast_line_through_the_cutoff_folds_nothing_loud() {
        let mut p = Panel {
            cutoff: 0.7,
            emphasis: 0.5,
            contour_amount: 0.0,
            keyboard_control_1: false,
            filter_mod: true,
            mod_wheel: 1.0,
            mod_mix: 0.0,
            ..Panel::default()
        };
        p.osc[0].range = crate::tuning::Range::R8;
        p.osc[0].waveform = Waveform::Sawtooth;
        p.osc[2].range = crate::tuning::Range::R2;
        p.osc[2].waveform = Waveform::Sawtooth;
        p.loudness_contour = ContourKnobs {
            attack: 0.0,
            decay: 0.5,
            sustain: 1.0,
        };
        p.filter_contour = p.loudness_contour;
        let mut l = Light::new(48_000.0);
        l.set_panel(&p);
        l.note(72, true);
        run(&mut l, 48_000);
        let x = run(&mut l, 1 << 15);
        // A Hann-windowed partial's level at `f` Hz.
        let at = |f: f64| {
            let (mut re, mut im) = (0.0, 0.0);
            for (n, v) in x.iter().enumerate() {
                let t = n as f64 / x.len() as f64;
                let w = 1.0 - (std::f64::consts::TAU * t).cos();
                let a = std::f64::consts::TAU * f * n as f64 / 48_000.0;
                re += v * w * a.cos();
                im += v * w * a.sin();
            }
            re.hypot(im) / x.len() as f64
        };
        let f0 = 440.0 * 2f64.powf(3.0 / 12.0) * 2.0;
        let loudest = (20..92)
            .map(|h| 48_000.0 - f64::from(h) * f0)
            .filter(|f| *f > 500.0 && *f < 12_000.0)
            .map(at)
            .fold(0.0, f64::max);
        let partials = (1..24)
            .map(|h| at(f64::from(h) * f0).powi(2))
            .sum::<f64>()
            .sqrt();
        let db = 20.0 * (loudest / partials).log10();
        assert!(db < -48.0, "the loudest foldover {db:.1} dBc");
    }

    /// Key 69 on 8' is 440 Hz, a RANGE an octave, oscillator 2's FREQUENCY pot at its
    /// counter-clockwise end the circuit's -856.4 cents (`freq_dial`).
    #[test]
    fn the_pitch_laws() {
        let mut l = Light::new(48_000.0);
        l.set_panel(&measuring());
        l.note(69, true);
        run(&mut l, 4800);
        assert!((hz(&run(&mut l, 48_000)) / 440.0 - 1.0).abs() < 2e-4);
        let mut p = measuring();
        p.osc[0].range = crate::tuning::Range::R4;
        l.set_panel(&p);
        run(&mut l, 4800);
        assert!((hz(&run(&mut l, 48_000)) / 880.0 - 1.0).abs() < 2e-4);
        p.osc[0] = OscPanel {
            on: false,
            ..p.osc[0]
        };
        p.osc[1] = OscPanel {
            on: true,
            volume: 0.3,
            freq: 0.0,
            ..p.osc[1]
        };
        l.set_panel(&p);
        run(&mut l, 4800);
        let cents = 1200.0 * (hz(&run(&mut l, 48_000)) / 440.0).log2();
        assert!((cents + 856.4).abs() < 1.5, "{cents} cents");
    }

    /// The filter at EMPHASIS 10 rings at the circuit's pitch and level once something
    /// reaches it (CUTOFF 0.5: 1056 Hz, 0.170 RMS), not at all at CUTOFF 0.1; at CUTOFF 0.5
    /// it rings from EMPHASIS
    /// 7.35, not at 7.15 (the circuit's threshold 7.25); in FILTER MODE HI as in LO.
    #[test]
    fn the_filter_rings_where_the_circuits_does() {
        let ring = |cutoff: f64, emphasis: f64| {
            let mut l = Light::new(48_000.0);
            let mut p = measuring();
            p.cutoff = cutoff;
            p.emphasis = emphasis;
            p.filter_hi = emphasis == 0.99;
            l.set_panel(&p);
            l.note(60, true);
            run(&mut l, 960);
            // (A kick from oscillator 1, then nothing reaching the filter.)
            p.osc[0].on = false;
            l.set_panel(&p);
            run(&mut l, 48_000);
            run(&mut l, 24_000)
        };
        let x = ring(0.5, 1.0);
        let rms = (x.iter().map(|y| y * y).sum::<f64>() / x.len() as f64).sqrt();
        // (The circuit's 0.170: its ring louder than one pair's limit would have it.)
        assert!((rms / 0.170 - 1.0).abs() < 0.04, "rms {rms}");
        assert!((hz(&x) / 1056.0 - 1.0).abs() < 0.01, "{} Hz", hz(&x));
        let hi = ring(0.5, 0.99);
        assert!((hz(&hi) / 1056.0 - 1.0).abs() < 0.01, "HI: {} Hz", hz(&hi));
        let quiet = |x: &[f64]| x.iter().all(|y| y.abs() < 1e-4);
        assert!(quiet(&ring(0.1, 1.0)), "rang at CUTOFF 0.1");
        assert!(!quiet(&ring(0.5, 0.735)), "silent at EMPHASIS 7.35");
        assert!(quiet(&ring(0.5, 0.715)), "rang at EMPHASIS 7.15");
    }

    /// EXTERNAL INPUT coupled at its jack and after its preamplifier as the circuit's: 5 Hz
    /// through the open filter 21 to 24.5 dB under 100 Hz (the circuit's 22.8; the output's
    /// droop alone 11.4, with the jack's coupling alone 18).
    #[test]
    fn the_external_input_is_coupled() {
        let level = |hz: f64| {
            let mut l = Light::new(48_000.0);
            let mut p = measuring();
            p.osc[0].on = false;
            p.ext_on = true;
            p.ext_volume = 0.5;
            l.set_panel(&p);
            l.note(60, true);
            let w = std::f64::consts::TAU * hz / 48_000.0;
            let mut peak = 0.0f64;
            for i in 0..96_000 {
                let y = l.tick(0.001 * (w * f64::from(i)).sin());
                if i >= 48_000 {
                    peak = peak.max(y.abs());
                }
            }
            20.0 * peak.log10()
        };
        let d = level(5.0) - level(100.0);
        assert!(
            (-24.5..-21.0).contains(&d),
            "5 Hz {d:+.1} dB against 100 Hz"
        );
    }

    /// The noise as loud at 44.1 and 96 kHz as at 48 (within 1 dB, white and pink at
    /// VOLUME 5 through the open filter; the circuit's white 0.7 dB louder at 96 kHz, its
    /// open filter reaching higher): its density a hertz, not a sample, as the circuit's.
    #[test]
    fn the_noise_is_as_loud_at_every_rate() {
        let rms = |rate: f64, pink: bool| {
            let mut l = Light::new(rate);
            let mut p = measuring();
            p.osc[0].on = false;
            p.noise_on = true;
            p.noise_pink = pink;
            p.noise_volume = 0.5;
            l.set_panel(&p);
            l.note(60, true);
            let n = (rate * 0.5) as usize;
            run(&mut l, n);
            let x = run(&mut l, 4 * n);
            10.0 * (x.iter().map(|y| y * y).sum::<f64>() / x.len() as f64).log10()
        };
        for pink in [false, true] {
            let at48 = rms(48_000.0, pink);
            for rate in [44_100.0, 96_000.0] {
                let d = rms(rate, pink) - at48;
                assert!(
                    d.abs() < 1.0,
                    "{} at {rate} Hz: {d:+.2} dB",
                    if pink { "pink" } else { "white" }
                );
            }
        }
    }

    /// Beyond the keyboard's keys (F to C) the rest of a glide is GLIDE's RC alone, as the
    /// circuit's voice has it: at GLIDE 6, from key 36 held a second to 43, still 2.14
    /// semitones short after 0.4 s (the circuit's 2.14); and the slide holds once the key is
    /// let go (the keyboard's track-and-hold).
    #[test]
    fn glide_beyond_the_keys_and_held_once_let_go() {
        let mut l = Light::new(48_000.0);
        let mut p = measuring();
        p.glide = 0.6;
        p.glide_on = true;
        l.set_panel(&p);
        l.note(36, true);
        run(&mut l, 48_000);
        l.note(43, true);
        l.note(36, false);
        run(&mut l, 19_200);
        let short = 43.0 - (l.pitch + l.beyond);
        assert!((short - 2.14).abs() < 0.1, "{short} semitones short");
        // Let go mid-slide in the keyboard's own range: the pitch holds there.
        l.note(72, true);
        l.note(43, false);
        run(&mut l, 1440);
        l.note(72, false);
        run(&mut l, 32);
        let at = l.pitch + l.beyond;
        assert!(at < 70.0, "the slide over by then: {at}");
        run(&mut l, 24_000);
        assert!((l.pitch + l.beyond - at).abs() < 1e-9, "slid on from {at}");
    }

    /// LONG divides the measured slide's rate by five, rising and falling, and multiplies
    /// the beyond-the-keys RC by five. The middle of the travel excludes the contacts' lead.
    #[test]
    fn long_glide_takes_five_times_as_long_in_the_light_voice() {
        let middle = |setting: f64, long: bool, falling: bool, beyond: bool| {
            let mut l = Light::new(48_000.0);
            let p = Panel {
                glide_on: true,
                glide: setting,
                glide_long: long,
                ..measuring()
            };
            l.set_panel(&p);
            let (from, to): (f64, f64) = if beyond { (96.0, 108.0) } else { (48.0, 60.0) };
            let (from, to) = if falling { (to, from) } else { (from, to) };
            l.note(to as i32, true);
            l.pitch = from.clamp(41.0, 84.0);
            l.beyond = from - l.pitch;
            let mut at = [None; 2];
            let mut before = 0.0;
            for i in 0..400_000 {
                l.control();
                let progress = (l.pitch + l.beyond - from) / (to - from);
                for (k, threshold) in [0.2, 0.8].into_iter().enumerate() {
                    if at[k].is_none() && progress >= threshold {
                        at[k] = Some(i as f64 + (threshold - before) / (progress - before));
                    }
                }
                if let [Some(a), Some(b)] = at {
                    return b - a;
                }
                before = progress;
            }
            panic!("no glide: {at:?}");
        };
        for setting in [0.0, 0.3, 0.5, 0.8, 1.0] {
            let mut normal = Light::new(48_000.0);
            let mut long = Light::new(48_000.0);
            let p = Panel {
                glide_on: true,
                glide: setting,
                ..measuring()
            };
            normal.set_panel(&p);
            long.set_panel(&Panel {
                glide_long: true,
                ..p
            });
            assert!((normal.set.glide.0 / long.set.glide.0 - 5.0).abs() < 1e-12);
            assert!((normal.set.glide.1 / long.set.glide.1 - 5.0).abs() < 1e-12);
            assert!((normal.set.beyond.ln() / long.set.beyond.ln() - 5.0).abs() < 1e-9);
            // The in-range slide at 0 finishes within one control tick.
            if setting == 0.0 {
                continue;
            }
            for falling in [false, true] {
                for beyond in [false, true] {
                    let ratio = middle(setting, true, falling, beyond)
                        / middle(setting, false, falling, beyond);
                    assert!(
                        (ratio - 5.0).abs() < 0.003,
                        "GLIDE {setting}, falling {falling}, beyond {beyond}: {ratio}"
                    );
                }
            }
        }
    }

    /// The contours start 7.8 ms after the key; SUSTAIN at 10 holds the loudness contour at
    /// 4.45 V; with the DECAY switch off the release falls with a time constant near 20 ms
    /// after its 14 ms, towards -0.70 V (DECAY 2); GLIDE at 4 slides the 11 semitones from
    /// 48 to 59 in about 27 ms from the lower key's release (the circuit's 27.3 ms: its pitch
    /// contact 2 ms behind, then 456 semitones a second).
    #[test]
    fn the_contour_and_glide_laws() {
        let mut l = Light::new(48_000.0);
        let mut p = measuring();
        p.decay = false;
        p.loudness_contour.decay = 0.2;
        l.set_panel(&p);
        let rest = l.contours().1;
        l.note(60, true);
        let mut started = None;
        for i in 0..4800 {
            l.tick(0.0);
            if started.is_none() && l.contours().1 > rest + 0.01 {
                started = Some(i as f64 / 48.0);
            }
        }
        let ms = started.expect("no attack");
        assert!((ms - 7.8).abs() < 0.2, "the attack at {ms} ms");
        run(&mut l, 48_000);
        assert!((l.contours().1 - 4.4549).abs() < 0.01, "{}", l.contours().1);
        l.note(60, false);
        // 14 ms of the trigger, then a time constant (20 ms at DECAY 2): from 4.45 V towards
        // -0.70 V.
        run(&mut l, 677 + 1440);
        let fallen = (l.contours().1 + 0.6988) / (4.4549 + 0.6988);
        let tau = 0.030 / -fallen.ln();
        assert!((tau - 0.0200).abs() < 0.002, "tau {tau}");
        // GLIDE.
        p.glide = 0.4;
        p.glide_on = true;
        l.set_panel(&p);
        l.note(48, true);
        run(&mut l, 48_000);
        l.note(60, true);
        l.note(48, false);
        let mut at = None;
        for i in 0..9600 {
            l.tick(0.0);
            if at.is_none() && l.pitch > 59.0 {
                at = Some(i as f64 / 48.0);
            }
        }
        let ms = at.expect("no glide");
        assert!((ms - 27.3).abs() < 2.0, "11 semitones in {ms} ms");
    }

    /// The contours' times through the reference's laws (voice.rs's `time_pot`), as the
    /// circuit's: ATTACK 6 takes the loudness contour to its plateau in 0.661 s (the circuit's
    /// 0.666), DECAY 6 the filter contour half way down from it in 0.345 to 0.365 s (0.355)
    /// and nine tenths in 1.26 to 1.33 s (1.297; with the absorbed charge, R-LO-CONTOUR),
    /// SUSTAIN 5 holds the loudness contour at 1.675 V; and a fast attack stops short of the
    /// plateau, as the circuit's does (ATTACK 0: 4.685 V, against 4.792).
    #[test]
    fn the_contours_follow_the_references_laws() {
        let mut p = measuring();
        p.loudness_contour = ContourKnobs {
            attack: 0.6,
            decay: 0.5,
            sustain: 1.0,
        };
        p.filter_contour = ContourKnobs {
            attack: 0.0,
            decay: 0.6,
            sustain: 0.0,
        };
        let mut l = Light::new(48_000.0);
        l.set_panel(&p);
        l.note(60, true);
        let (mut peak, mut top, mut half, mut at) = (0.0f64, 0.0f64, None, 0.0);
        let mut ninety = None;
        for i in 0..96_000 {
            l.tick(0.0);
            let (f, v) = l.contours();
            if f > peak {
                peak = f;
            }
            if half.is_none() && peak > 4.0 && f < 0.5 * (peak + 0.0431) {
                half = Some(i as f64 / 48_000.0);
            }
            if ninety.is_none() && peak > 4.0 && f < 0.1 * peak + 0.9 * 0.0431 {
                ninety = Some(i as f64 / 48_000.0);
            }
            if v > top + 1e-9 {
                top = v;
                at = i as f64 / 48_000.0;
            }
        }
        assert!((peak - 4.685).abs() < 0.01, "ATTACK 0's peak {peak} V");
        assert!(
            (at - 0.666).abs() < 0.01,
            "ATTACK 6 to its plateau in {at} s"
        );
        let half = half.expect("no decay") - 0.0086;
        assert!(
            (0.345..0.365).contains(&half),
            "DECAY 6 half way in {half} s"
        );
        let ninety = ninety.expect("no decay") - 0.0086;
        assert!(
            (1.26..1.33).contains(&ninety),
            "DECAY 6 nine tenths in {ninety} s"
        );
        p.loudness_contour.sustain = 0.5;
        l.set_panel(&p);
        run(&mut l, 240_000);
        let held = l.contours().1;
        assert!((held - 1.675).abs() < 0.01, "SUSTAIN 5 at {held} V");
    }

    /// Shape MOD uses both source extrema, rather than assuming that the peak about
    /// the period mean is also its negative trough. This is source voltage, not an
    /// adjustment fitted to the shape-travel test or the hardware recording.
    #[test]
    fn shape_mod_uses_the_current_triangles_midpoint_and_half_swing() {
        let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
        for i in 0..=65_536 {
            let v = triangle_naive(i as f32 / 65_536.0);
            lo = lo.min(v);
            hi = hi.max(v);
        }
        assert!((lo - triangle_source::EXTREMA.0).abs() < 2e-6);
        assert!((hi - triangle_source::EXTREMA.1).abs() < 2e-6);
        // The normalization must retain this asymmetry; its midpoint is not its mean.
        assert!((lo + hi).abs() > 0.05);
        let line =
            |v: f32| v / laws::TRIANGLE * laws::MOD_SWING[0] as f32 + laws::MOD_CENTRE[0] as f32;
        assert!((shape_mod_swing(line(lo)) - 1.0).abs() < 5e-6);
        assert!((shape_mod_swing(line(hi)) + 1.0).abs() < 5e-6);
        assert!(shape_mod_swing(line(0.5 * (lo + hi))).abs() < 5e-6);
    }

    /// The hybrid's fifth harmonic follows the current circuit reference at the original
    /// measurement panel, within the same 1 dB budget. Measure it here rather than retaining
    /// the old -27.6 dBc fixture: the Q2 fold and R031 reference calibration changed the
    /// physical source (docs/calibration/waveform-fold-reference.md).
    #[test]
    fn the_shark_tooth_as_the_circuits() {
        let mut p = measuring();
        p.osc[0].waveform = Waveform::SharkTooth;
        p.osc[0].volume = 0.2;
        let mut l = Light::new(48_000.0);
        l.set_panel(&p);
        l.note(33, true);
        run(&mut l, 24_000);
        let light = run(&mut l, 48_000);
        // The original potato_reference.rs harness measures Voice at this quality too.
        p.quality = crate::voice::Quality::Potato;
        let mut reference = crate::voice::Voice::new(48_000.0, p);
        reference.note(33, true);
        for _ in 0..24_000 {
            reference.tick();
        }
        let circuit: Vec<f64> = (0..48_000).map(|_| reference.tick()).collect();
        let fifth = |x: &[f64]| {
            let f0 = hz(x);
            let amp = |h: f64| {
                let w = std::f64::consts::TAU * h * f0 / 48_000.0;
                let (mut re, mut im) = (0.0, 0.0);
                for (i, y) in x.iter().enumerate() {
                    let window =
                        0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / x.len() as f64).cos();
                    re += window * y * (w * i as f64).cos();
                    im += window * y * (w * i as f64).sin();
                }
                re.hypot(im)
            };
            20.0 * (amp(5.0) / amp(1.0)).log10()
        };
        let light_h5 = fifth(&light);
        let circuit_h5 = fifth(&circuit);
        assert!(
            (light_h5 - circuit_h5).abs() < 1.0,
            "the fifth harmonic: light {light_h5:.3}, circuit {circuit_h5:.3} dBc"
        );
    }

    /// The passband falls as the circuit's at the lowest cutoffs: a sawtooth at 55 Hz
    /// (VOLUME 3) through CUTOFF 0.1 15.9 dB under CUTOFF 1's (the circuit's 17.3; with the
    /// ladder's own passband alone, 13).
    #[test]
    fn the_passband_falls_at_the_lowest_cutoffs() {
        let level = |cutoff: f64| {
            let mut l = Light::new(48_000.0);
            let mut p = measuring();
            p.cutoff = cutoff;
            l.set_panel(&p);
            l.note(33, true);
            run(&mut l, 24_000);
            let x = run(&mut l, 48_000);
            10.0 * (x.iter().map(|y| y * y).sum::<f64>() / x.len() as f64).log10()
        };
        let d = level(0.1) - level(1.0);
        assert!((d + 15.9).abs() < 1.0, "CUTOFF 0.1 {d:+.1} dB against 1");
    }

    /// DRIVE as the circuit's: oscillator 1's sawtooth at VOLUME 5, the filter open, 6, 12
    /// and 24 dB of it raise the level 5.7, 10.75 and 15.0 dB (the circuit's 5.7, 10.5 and
    /// 15.0: the pair saturates, and its limit falls with DRIVE as the circuit's ladder holds
    /// a driven input).
    #[test]
    fn drive_raises_the_level_less_and_less() {
        let level = |db: f64| {
            let mut l = Light::new(48_000.0);
            let mut p = measuring();
            p.osc[0].volume = 0.5;
            l.set_panel(&p);
            l.set_drive(10f64.powf(db / 20.0));
            l.note(45, true);
            run(&mut l, 24_000);
            let x = run(&mut l, 48_000);
            10.0 * (x.iter().map(|y| y * y).sum::<f64>() / x.len() as f64).log10()
        };
        let off = level(0.0);
        for (db, want) in [(6.0, 5.75), (12.0, 10.75), (24.0, 15.0)] {
            let up = level(db) - off;
            assert!((up - want).abs() < 0.3, "DRIVE {db} dB: {up:+.2} dB");
        }
    }

    /// DRIVE leaves the pair's limit its share for the ring: told again each block, as the
    /// engine tells it, it changes no sample wherever the blocks fall against the control
    /// laws' 16 samples, and moved, it moves the limit under the ring's lift
    /// (decisions.md R-LO-SET-DRIVE).
    #[test]
    fn set_drive_keeps_the_rings_limit() {
        let ringing = |l: &mut Light| {
            let mut p = measuring();
            p.cutoff = 0.3;
            p.emphasis = 0.7;
            l.set_panel(&p);
            l.note(72, true);
        };
        for offset in [0, 1, 4, 8, 10, 15] {
            let play = |told: bool| {
                let mut l = Light::new(48_000.0);
                ringing(&mut l);
                let mut x = run(&mut l, offset);
                for _ in 0..200 {
                    if told {
                        l.set_drive(1.0);
                    }
                    x.extend(run(&mut l, 256));
                }
                x
            };
            assert_eq!(play(true), play(false), "offset {offset}");
        }
        let mut l = Light::new(48_000.0);
        ringing(&mut l);
        run(&mut l, 64);
        let lift = l.limit.0 / l.drive_top;
        assert!(lift > 1.01, "the ring's lift {lift}");
        l.set_drive(4.0);
        assert!(l.drive_top < 1.0);
        let moved = l.limit.0 / l.drive_top;
        assert!((moved - lift).abs() < 1e-5, "{lift} to {moved}");
    }

    /// FILTER MODE HI as the circuit's: the bus less the filter's output, so that a sawtooth
    /// at 110 Hz (VOLUME 3) under CUTOFF 0.7 (5.3 kHz) is mostly gone, 10.6 dB under LO's
    /// (the circuit's 10.8), and over CUTOFF 0.3 (214 Hz) 6.3 dB louder than LO's (6.5); at
    /// EMPHASIS 5 the bass stands 3 dB under the bus (a shelf: 5.7 dB over LO's, the
    /// circuit's 5.7).
    #[test]
    fn filter_mode_hi_is_the_bus_less_the_filter() {
        let level = |cutoff: f64, emphasis: f64, hi: bool| {
            let mut l = Light::new(48_000.0);
            let mut p = measuring();
            p.cutoff = cutoff;
            p.emphasis = emphasis;
            p.filter_hi = hi;
            l.set_panel(&p);
            l.note(45, true);
            run(&mut l, 24_000);
            let x = run(&mut l, 48_000);
            10.0 * (x.iter().map(|y| y * y).sum::<f64>() / x.len() as f64).log10()
        };
        for (cutoff, emphasis, want) in [(0.7, 0.0, -10.8), (0.3, 0.0, 6.5), (0.5, 0.5, 5.7)] {
            let d = level(cutoff, emphasis, true) - level(cutoff, emphasis, false);
            assert!(
                (d - want).abs() < 0.8,
                "CUTOFF {cutoff}, EMPHASIS {emphasis}: HI {d:+.2} dB against LO"
            );
        }
    }

    /// With every source off, the filter at EMPHASIS 10 sings from the mixer's own noise as
    /// the circuit's does (decisions.md R-LO-HISS): at CUTOFF's centre, near the circuit's
    /// 1056 Hz and its level, where without the noise it stayed silent.
    #[test]
    fn the_filter_sings_alone_from_the_mixers_noise() {
        let mut l = Light::new(48_000.0);
        let mut p = measuring();
        for o in &mut p.osc {
            o.on = false;
        }
        p.cutoff = 0.5;
        p.emphasis = 1.0;
        l.set_panel(&p);
        l.note(45, true);
        run(&mut l, 48_000);
        let x = run(&mut l, 48_000);
        let rms = (x.iter().map(|y| y * y).sum::<f64>() / x.len() as f64).sqrt();
        assert!(rms > 0.05, "the filter alone: RMS {rms:.2e}");
        let f = hz(&x);
        assert!(
            (f / 1056.0).log2().abs() * 1200.0 < 50.0,
            "the filter alone at {f:.0} Hz"
        );
    }

    /// NOTE PRIORITY (decisions.md R50) as the circuit's voice has it: LOW the lowest
    /// key held, HIGH the highest, LAST the key pressed last, then the last still held; a key
    /// let go keeps sounding for the release lead (2 ms) unless a newer key takes over; beyond
    /// the keyboard's keys, the chosen key continues the scale.
    #[test]
    fn the_priority_chooses_the_key_held_that_sounds() {
        let plays = |priority: Priority, steps: &[(i32, i32)]| {
            let mut l = Light::new(48_000.0);
            let mut p = measuring();
            p.priority = priority;
            l.set_panel(&p);
            for &(key, sounding) in steps {
                l.note(key.abs(), key > 0);
                run(&mut l, 480);
                let at = l.target + l.foot;
                assert!(
                    (at - f64::from(sounding)).abs() < 1e-9,
                    "{priority:?}: after {key:+}, key {at} for {sounding}"
                );
            }
        };
        plays(
            Priority::Low,
            &[(57, 57), (64, 57), (50, 50), (-50, 57), (-57, 64)],
        );
        plays(
            Priority::High,
            &[
                (57, 57),
                (64, 64),
                (50, 64),
                (-64, 57),
                (-57, 50),
                (100, 100),
            ],
        );
        plays(
            Priority::Last,
            &[
                (57, 57),
                (64, 64),
                (50, 50),
                (-50, 64),
                (60, 60),
                (-64, 60),
                (-60, 57),
                (30, 30),
            ],
        );
        // The release lead: LAST's key let go sounds 2 ms on, then the last still held.
        let mut l = Light::new(48_000.0);
        let mut p = measuring();
        p.priority = Priority::Last;
        l.set_panel(&p);
        l.note(57, true);
        l.note(64, true);
        run(&mut l, 480);
        l.note(64, false);
        run(&mut l, 48);
        assert_eq!(l.target, 64.0, "within the lead");
        run(&mut l, 96);
        assert_eq!(l.target, 57.0, "after it");
    }

    /// RETRIGGER as the circuit's voice has it: a key pressed while another is held lowers the
    /// keys' gate for the gap (13 ms) and then comes down, the contours starting again (an
    /// attack) and the pitch moving to it only then; off, single triggering.
    #[test]
    fn retrigger_starts_the_contours_again_with_keys_held() {
        let run = |retrigger: bool| {
            let mut l = Light::new(48_000.0);
            let mut p = measuring();
            p.priority = Priority::Last;
            p.retrigger = retrigger;
            p.loudness_contour.decay = 0.3;
            p.loudness_contour.sustain = 0.2;
            l.set_panel(&p);
            l.note(57, true);
            run(&mut l, 24_000);
            let before = l.contours().1;
            l.note(64, true);
            run(&mut l, 384);
            let early = l.target;
            let most = (0..3840)
                .map(|_| {
                    l.tick(0.0);
                    l.contours().1
                })
                .fold(0.0, f64::max);
            (before, most, early, l.target)
        };
        let (before, most, early, late) = run(false);
        assert!((most - before).abs() < 0.05, "{before} V, then {most} V");
        assert_eq!((early, late), (64.0, 64.0));
        let (before, most, early, late) = run(true);
        assert!(most > before + 2.0, "{before} V, then {most} V");
        assert_eq!((early, late), (57.0, 64.0));
    }
}
