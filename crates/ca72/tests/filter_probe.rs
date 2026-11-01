use ca72::voice::{Jacks, Quality, Voice};

#[test]
fn diagnostic_filter_voltage_does_not_change_audio() {
    for rate in [44_100.0, 48_000.0, 96_000.0] {
        for quality in [
            Quality::Potato,
            Quality::HighFidelity,
            Quality::NoCompromises,
        ] {
            let mut normal = Voice::prototype(rate);
            normal.panel.quality = quality;
            normal.panel.ext_on = true;
            normal.note(60, true);
            let mut diagnostic = normal.clone();
            diagnostic.set_filter_probe(true);
            let mut peak = 0.0f64;
            for i in 0..4096 {
                let feedback = if (1024..2048).contains(&i) { 0.3 } else { 0.0 };
                normal.feedback = feedback;
                diagnostic.feedback = feedback;
                assert_eq!(normal.tick().to_bits(), diagnostic.tick().to_bits());
                peak = peak.max(diagnostic.probe().2.abs());
                if quality == Quality::Potato && feedback == 0.0 {
                    assert_eq!(normal.probe().2, 0.0, "default HI probe did extra work");
                } else if feedback == 0.0 {
                    assert_eq!(normal.probe(), diagnostic.probe());
                }
            }
            assert!(peak > 1e-3, "silent filter probe: {quality:?} {rate}");
        }
    }
}

#[test]
fn paired_diagnostic_filter_probes_match_serial_with_independent_opt_in() {
    for rate in [44_100.0, 48_000.0, 96_000.0] {
        let mut a = Voice::prototype(rate);
        a.panel.quality = Quality::Potato;
        a.set_filter_probe(true);
        a.set_seed(3);
        a.note(60, true);
        let mut b = a.clone();
        b.set_filter_probe(false);
        b.set_seed(11);
        b.note(72, true);
        let (mut ap, mut bp) = (a.clone(), b.clone());
        let mut peak = 0.0f64;
        for _ in 0..4096 {
            let j = Jacks::default();
            let (ya, yb) = (a.tick_jacks(&j), b.tick_jacks(&j));
            let (ha, hb) = (ap.before_filter(&j), bp.before_filter(&j));
            let (fa, fb) = Voice::filter_pair(&mut ap, &ha, &mut bp, &hb);
            assert_eq!(ya.to_bits(), ap.after_filter(&ha, fa).to_bits());
            assert_eq!(yb.to_bits(), bp.after_filter(&hb, fb).to_bits());
            assert_eq!(a.probe(), ap.probe());
            assert_eq!(b.probe(), bp.probe());
            assert_eq!(b.probe().2, 0.0);
            peak = peak.max(ap.probe().2.abs());
        }
        assert!(peak > 1e-3, "silent paired filter probe at {rate}");
    }
}
