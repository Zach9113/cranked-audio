//! Knob LED colours (SPEC.md section 5).

use cranked_protocol::{Rgb, KNOBS, LEDS_PER_KNOB};

const IDLE_DIM: f64 = 0.25;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KnobLook {
    /// 0..100 (fractional while easing toward a new value), None = not known yet (LEDs stay dark).
    pub level: Option<f64>,
    pub color: Rgb,
    pub background: Rgb,
    pub muted_color: Rgb,
    pub muted: bool,
    /// Nothing on this knob is playing.
    pub idle: bool,
    /// Extra dimming (Auto-lower), 1.0 = none.
    pub scale: f64,
}

/// The edge LED's mix of background -> colour. An S-curve (smoothstep): it eases in and out at the LED's ends, so
/// the LED doesn't pop the moment the level enters it, and it's exactly 50/50 at the middle, so 50% on the knob
/// shows a clear half-and-half: one LED in the colour, one evenly mixed, one in the background.
fn blend(from: Rgb, to: Rgb, amount: f64) -> Rgb {
    let x = amount.clamp(0.0, 1.0);
    let weight = x * x * (3.0 - 2.0 * x);
    let mix = |a: u8, b: u8| (f64::from(a) * (1.0 - weight) + f64::from(b) * weight).round().clamp(0.0, 255.0) as u8;
    (mix(from.0, to.0), mix(from.1, to.1), mix(from.2, to.2))
}

fn scale(color: Rgb, factor: f64) -> Rgb {
    let s = |c: u8| (f64::from(c) * factor).round().clamp(0.0, 255.0) as u8;
    (s(color.0), s(color.1), s(color.2))
}

pub fn knob_colors(look: &KnobLook, brightness: u8, dim_idle: bool) -> [Rgb; LEDS_PER_KNOB] {
    let Some(level) = look.level else {
        return [(0, 0, 0); LEDS_PER_KNOB];
    };
    let mut factor = f64::from(brightness.min(100)) / 100.0;
    if dim_idle && look.idle {
        factor *= IDLE_DIM;
    }
    factor *= look.scale.clamp(0.0, 1.0);
    if look.muted {
        return [scale(look.muted_color, factor); LEDS_PER_KNOB];
    }
    let per_led = 100.0 / LEDS_PER_KNOB as f64;
    std::array::from_fn(|i| {
        let fill = ((level - i as f64 * per_led) / per_led).clamp(0.0, 1.0);
        scale(blend(look.background, look.color, fill), factor)
    })
}

pub fn colors(looks: &[KnobLook; KNOBS], brightness: u8, dim_idle: bool) -> [[Rgb; LEDS_PER_KNOB]; KNOBS] {
    std::array::from_fn(|k| knob_colors(&looks[k], brightness, dim_idle))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look(level: u8) -> KnobLook {
        KnobLook {
            level: Some(f64::from(level)),
            color: (255, 0, 0),
            background: (0, 0, 255),
            muted_color: (40, 0, 0),
            muted: false,
            idle: false,
            scale: 1.0,
        }
    }

    #[test]
    fn fifty_percent_is_half_and_half() {
        let leds = knob_colors(&look(50), 100, true);
        assert_eq!((leds[0], leds[2]), ((255, 0, 0), (0, 0, 255)));
        let (r, g, b) = leds[1];
        assert!(r.abs_diff(128) <= 1 && g == 0 && b.abs_diff(128) <= 1, "middle LED evenly mixed: {:?}", leds[1]);
        assert_eq!(knob_colors(&look(100), 100, true), [(255, 0, 0); 3]);
        assert_eq!(knob_colors(&look(0), 100, true), [(0, 0, 255); 3]);
        // eases in: just past an LED's start it has barely changed
        let entering = knob_colors(&look(36), 100, true)[1];
        assert!(entering.0 < 10, "{entering:?}");
    }

    #[test]
    fn muted_idle_brightness_and_unknown() {
        let mut muted = look(80);
        muted.muted = true;
        assert_eq!(knob_colors(&muted, 100, true), [(40, 0, 0); 3]);
        assert_eq!(knob_colors(&look(100), 50, true)[0], (128, 0, 0));
        let mut idle = look(100);
        idle.idle = true;
        assert_eq!(knob_colors(&idle, 100, true)[0], (64, 0, 0));
        assert_eq!(knob_colors(&idle, 100, false)[0], (255, 0, 0));
        idle.level = None;
        assert_eq!(knob_colors(&idle, 100, true), [(0, 0, 0); 3]);
    }
}
