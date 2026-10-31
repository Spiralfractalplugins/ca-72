use ca72::voice::{Panel, Quality, Voice};

#[test]
fn feedback_and_ext_handoffs_follow_the_running_waveform() {
    for rate in [44_100.0, 48_000.0, 96_000.0] {
        for (ext, interval) in [(false, 17), (true, 17), (false, 257), (true, 257)] {
            let mut worst = 0.0f64;
            let p = Panel {
                quality: Quality::Potato,
                cutoff: 1.0,
                emphasis: 0.3,
                ext_on: true,
                ..Panel::default()
            };
            let mut v = Voice::new(rate, p);
            v.set_seed(1);
            v.note(60, true);
            if ext {
                v.feedback = 0.3;
            }
            for i in 0..(rate as usize) {
                if i > 1000 && i % interval == 0 {
                    let mut continued = v.clone();
                    if ext {
                        v.panel.ext_on = !v.panel.ext_on;
                    } else {
                        v.feedback = if v.feedback == 0.0 { 0.3 } else { 0.0 };
                    }
                    let before = continued.tick();
                    let after = v.tick();
                    worst = worst.max((after - before).abs());
                    assert!(
                        (after - before).abs() < 1e-6,
                        "{rate} ext={ext} sample={i}: switch error {}",
                        after - before
                    );
                } else {
                    assert!(v.tick().is_finite());
                }
            }
            println!("rate={rate} ext={ext} interval={interval} worst_event_error={worst:.12e}");
        }
    }
}
