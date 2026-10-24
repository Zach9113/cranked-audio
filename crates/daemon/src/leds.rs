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

/// The edge LED's colour as the level moves through it.
///
/// Only ever the two colours the user picked: the LED is the background colour until the level reaches its middle,
/// then the active colour. Mixing them (in any colour space) shows hues that were never chosen, and fading through
/// black leaves a dark gap moving along the knob. With a black background the active colour simply fades in, which
/// is still only that one colour.
fn blend(from: Rgb, to: Rgb, amount: f64) -> Rgb {
    let x = amount.clamp(0.0, 1.0);
    if from == (0, 0, 0) {
        return scale(to, x);
    }
    if x >= 0.5 {
        to
    } else {
        from
    }
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
    fn only_the_two_chosen_colours_ever_show() {
        assert_eq!(knob_colors(&look(100), 100, true), [(255, 0, 0); 3]);
        assert_eq!(knob_colors(&look(0), 100, true), [(0, 0, 255); 3]);
        for level in 0..=100 {
            for led in knob_colors(&look(level), 100, true) {
                assert!(led == (255, 0, 0) || led == (0, 0, 255), "{level}%: {led:?} is neither colour");
            }
        }
        // the edge LED switches as the level passes its middle
        assert_eq!(knob_colors(&look(49), 100, true)[1], (0, 0, 255));
        assert_eq!(knob_colors(&look(51), 100, true)[1], (255, 0, 0));
    }

    #[test]
    fn a_black_background_fades_the_one_colour_in() {
        let mut lamp = look(50);
        lamp.background = (0, 0, 0);
        let edge = knob_colors(&lamp, 100, true)[1];
        assert!(edge.1 == 0 && edge.2 == 0 && (120..=135).contains(&edge.0), "only red, about half: {edge:?}");
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
