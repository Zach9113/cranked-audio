//! Knob LED colours (SPEC.md section 5).

use cranked_protocol::{Rgb, KNOBS, LEDS_PER_KNOB};

const GAMMA: f64 = 2.2;
const IDLE_DIM: f64 = 0.25;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KnobLook {
    /// 0..100, None = not known yet (LEDs stay dark).
    pub level: Option<u8>,
    pub color: Rgb,
    pub background: Rgb,
    pub muted_color: Rgb,
    pub muted: bool,
    /// Nothing on this knob is playing.
    pub idle: bool,
    /// Extra dimming (Auto-lower), 1.0 = none.
    pub scale: f64,
}

fn to_linear(c: u8) -> f64 {
    (f64::from(c) / 255.0).powf(GAMMA)
}

fn to_byte(linear: f64) -> u8 {
    (linear.clamp(0.0, 1.0).powf(1.0 / GAMMA) * 255.0).round() as u8
}

/// Mix in linear light so the edge LED fades evenly instead of looking muddy.
fn blend(from: Rgb, to: Rgb, amount: f64) -> Rgb {
    let mix = |a: u8, b: u8| to_byte(to_linear(a) + (to_linear(b) - to_linear(a)) * amount);
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
        let fill = ((f64::from(level) - i as f64 * per_led) / per_led).clamp(0.0, 1.0);
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
            level: Some(level),
            color: (255, 0, 0),
            background: (0, 0, 255),
            muted_color: (40, 0, 0),
            muted: false,
            idle: false,
            scale: 1.0,
        }
    }

    #[test]
    fn fills_from_the_left_with_a_soft_edge() {
        let leds = knob_colors(&look(50), 100, true);
        assert_eq!(leds[0], (255, 0, 0));
        assert_eq!(leds[2], (0, 0, 255));
        // halfway in linear light, not a dark 50/50 byte mix
        assert!(leds[1].0 > 150 && leds[1].2 > 150, "{:?}", leds[1]);
        assert_eq!(knob_colors(&look(100), 100, true), [(255, 0, 0); 3]);
        assert_eq!(knob_colors(&look(0), 100, true), [(0, 0, 255); 3]);
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
