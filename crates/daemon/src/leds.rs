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

/// The edge LED's transition between background and active colour.
///
/// Mixing two colours channel by channel goes through grey or brown (purple + gold = mud), and fading one out to
/// black before the other comes in leaves a dark gap travelling along the knob. Instead the mix happens in OKLCH,
/// a perceptual colour space: lightness, saturation and hue each move evenly and the hue takes the short way round
/// the colour wheel, so the edge LED stays bright and vivid the whole way (purple to gold passes through pink and
/// orange, not brown).
fn blend(from: Rgb, to: Rgb, amount: f64) -> Rgb {
    let x = amount.clamp(0.0, 1.0);
    // a black side has no hue to travel from: just fade the other colour
    if from == (0, 0, 0) {
        return scale(to, x);
    }
    if to == (0, 0, 0) {
        return scale(from, 1.0 - x);
    }
    let (l1, c1, h1) = oklch(from);
    let (l2, c2, h2) = oklch(to);
    let turn = (h2 - h1 + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI;
    let (l, c, h) = (l1 + (l2 - l1) * x, c1 + (c2 - c1) * x, h1 + turn * x);
    from_oklab(l, c * h.cos(), c * h.sin())
}

fn to_linear(c: u8) -> f64 {
    let c = f64::from(c) / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn to_byte(c: f64) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let c = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (c * 255.0).round().clamp(0.0, 255.0) as u8
}

/// sRGB -> OKLCH (lightness, chroma, hue in radians).
fn oklch((r, g, b): Rgb) -> (f64, f64, f64) {
    let (r, g, b) = (to_linear(r), to_linear(g), to_linear(b));
    let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
    let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
    let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
    let lightness = 0.210_454_255_3 * l + 0.793_617_785 * m - 0.004_072_046_8 * s;
    let a = 1.977_998_495_1 * l - 2.428_592_205 * m + 0.450_593_709_9 * s;
    let b = 0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766 * s;
    (lightness, a.hypot(b), b.atan2(a))
}

fn from_oklab(lightness: f64, a: f64, b: f64) -> Rgb {
    let l = (lightness + 0.396_337_777_4 * a + 0.215_803_757_3 * b).powi(3);
    let m = (lightness - 0.105_561_345_8 * a - 0.063_854_172_8 * b).powi(3);
    let s = (lightness - 0.089_484_177_5 * a - 1.291_485_548 * b).powi(3);
    (
        to_byte(4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s),
        to_byte(-1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s),
        to_byte(-0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701 * s),
    )
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

    fn brightness_of(c: Rgb) -> u32 {
        u32::from(c.0) + u32::from(c.1) + u32::from(c.2)
    }

    #[test]
    fn fills_from_the_left_and_the_edge_never_goes_dark_or_muddy() {
        assert_eq!(knob_colors(&look(100), 100, true), [(255, 0, 0); 3]);
        assert_eq!(knob_colors(&look(0), 100, true), [(0, 0, 255); 3]);
        let half = knob_colors(&look(50), 100, true);
        assert_eq!((half[0], half[2]), ((255, 0, 0), (0, 0, 255)));
        // every step of the edge LED stays bright and saturated (no dark dip, no grey)
        for level in 34..67 {
            let edge = knob_colors(&look(level), 100, true)[1];
            let (hi, lo) = (edge.0.max(edge.1).max(edge.2), edge.0.min(edge.1).min(edge.2));
            assert!(brightness_of(edge) > 150, "{level}%: {edge:?} too dark");
            assert!(hi - lo > 120, "{level}%: {edge:?} washed out");
        }
    }

    #[test]
    fn exact_colours_survive_the_round_trip() {
        for c in [(255, 0, 0), (0, 186, 20), (122, 35, 0), (92, 5, 156), (5, 194, 137)] {
            assert_eq!(blend(c, (1, 2, 3), 0.0), c);
            assert_eq!(blend((1, 2, 3), c, 1.0), c);
        }
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
