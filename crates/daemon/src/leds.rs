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

/// How dim the edge LED gets when the level sits exactly at its middle (half one colour, half the other).
const EDGE_FLOOR: f64 = 0.6;

/// The edge LED's colour as the level moves through it.
///
/// Only ever the two colours the user picked (mixing them, in any colour space, shows hues that were never chosen):
/// the LED shows whichever colour covers more of its share of the dial, and its brightness says how much: full when
/// that colour fills it, down to `EDGE_FLOOR` when the boundary sits right in its middle. So the boundary still
/// glides along the knob, never through black. With a black background the active colour simply fades in.
fn blend(from: Rgb, to: Rgb, amount: f64) -> Rgb {
    let x = amount.clamp(0.0, 1.0);
    if from == (0, 0, 0) {
        return scale(to, x);
    }
    let (colour, share) = if x >= 0.5 { (to, x) } else { (from, 1.0 - x) };
    scale(colour, EDGE_FLOOR + (1.0 - EDGE_FLOOR) * (share - 0.5) * 2.0)
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

    /// `led` is `colour` at some brightness (never another hue).
    fn is_shade_of(led: Rgb, colour: Rgb) -> bool {
        let f = |c: u8| f64::from(c);
        let k = f(led.0.max(led.1).max(led.2)) / f(colour.0.max(colour.1).max(colour.2));
        (f(led.0) - f(colour.0) * k).abs() <= 1.0 && (f(led.1) - f(colour.1) * k).abs() <= 1.0 && (f(led.2) - f(colour.2) * k).abs() <= 1.0
    }

    #[test]
    fn only_the_two_chosen_colours_ever_show() {
        assert_eq!(knob_colors(&look(100), 100, true), [(255, 0, 0); 3]);
        assert_eq!(knob_colors(&look(0), 100, true), [(0, 0, 255); 3]);
        for level in 0..=100 {
            for led in knob_colors(&look(level), 100, true) {
                assert!(is_shade_of(led, (255, 0, 0)) || is_shade_of(led, (0, 0, 255)), "{level}%: {led:?} is a new hue");
                assert!(led.0.max(led.2) >= 150, "{level}%: {led:?} too dark");
            }
        }
    }

    #[test]
    fn the_edge_led_dims_toward_the_boundary_and_switches_there() {
        let edge = |level| knob_colors(&look(level), 100, true)[1];
        assert_eq!(edge(33), (0, 0, 255), "not reached yet: all background");
        assert!(edge(45).2 < 255 && edge(45).0 == 0, "background, dimming as the level arrives");
        assert!(edge(55).0 < 255 && edge(55).2 == 0, "past the middle: level colour, still dimmed");
        assert_eq!(edge(67), (255, 0, 0), "filled");
        assert!(edge(50).0 >= 150 || edge(50).2 >= 150, "never dark at the boundary");
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
