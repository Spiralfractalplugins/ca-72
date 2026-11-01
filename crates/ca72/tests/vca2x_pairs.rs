use ca72::tuning::Range;
use ca72::voice::{Panel, Quality, Voice};
#[test]
fn upper_register_pairs_and_feedback_switches() {
    for rate in [44100.0, 48000.0, 96000.0] {
        let mut p = Panel {
            quality: Quality::Potato,
            cutoff: 1.0,
            ..Panel::default()
        };
        p.osc[0].range = Range::R2;
        p.ext_on = true;
        let mut a = Voice::new(rate, p);
        let mut b = Voice::new(rate, p);
        a.set_drive(10f64.powf(12.0 / 20.0));
        b.set_drive(10f64.powf(12.0 / 20.0));
        a.set_seed(3);
        b.set_seed(11);
        a.note(84, true);
        b.note(96, true);
        let (mut ap, mut bp) = (a.clone(), b.clone());
        for i in 0..(rate as usize / 4) {
            if i == 1000 || i == 3000 || i == 5000 {
                let fb = if i == 3000 { 0.0 } else { 0.3 };
                for v in [&mut a, &mut b, &mut ap, &mut bp] {
                    v.feedback = fb;
                }
            }
            let j = Default::default();
            let ya = a.tick_jacks(&j);
            let yb = b.tick_jacks(&j);
            let ha = ap.before_filter(&j);
            let hb = bp.before_filter(&j);
            let (fa, fb) = Voice::filter_pair(&mut ap, &ha, &mut bp, &hb);
            assert_eq!(
                ya.to_bits(),
                ap.after_filter(&ha, fa).to_bits(),
                "a {rate} {i}"
            );
            assert_eq!(
                yb.to_bits(),
                bp.after_filter(&hb, fb).to_bits(),
                "b {rate} {i}"
            );
        }
    }
}
