//! Knob LED colours (SPEC.md section 5).

use cranked_ipc::settings::LedBlend;
use cranked_protocol::convert::{round_u8, saturate_u8};
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
/// With the Soft fill, how far each LED's fade reaches into its neighbours' share of the dial, as a part of its own.
const SOFT_REACH: f64 = 0.5;

/// The edge LED as the level moves through it: whichever of the two picked colours covers more of its share of
/// the dial, at a brightness that says how much (full when it fills the LED, `EDGE_FLOOR` when the boundary sits
/// in its middle). Never a mix of the two: that shows hues nobody picked. With a black background the level colour
/// simply fades in. Returns the colour and its brightness; the dimming itself happens in `dim`.
fn edge(background: Rgb, color: Rgb, amount: f64) -> (Rgb, f64) {
    let x = amount.clamp(0.0, 1.0);
    if background == (0, 0, 0) {
        return (color, x);
    }
    let (lit, share) = if x >= 0.5 { (color, x) } else { (background, 1.0 - x) };
    (lit, EDGE_FLOOR + (1.0 - EDGE_FLOOR) * (share - 0.5) * 2.0)
}

/// The edge LED's colour and brightness for `blend`, `amount` of the way from background to level colour.
fn edge_for(blend: LedBlend, background: Rgb, color: Rgb, amount: f64) -> (Rgb, f64) {
    let x = amount.clamp(0.0, 1.0);
    let mix = |w_bg: f64, w_fill: f64| {
        let m = |b: u8, f: u8| round_u8(f64::from(b) * w_bg + f64::from(f) * w_fill);
        (m(background.0, color.0), m(background.1, color.1), m(background.2, color.2))
    };
    match blend {
        LedBlend::Clean => edge(background, color, x),
        // Turn Up's curves: the level colour comes in slowly (x^2.2) as the background goes out quickly ((1-x)^3)
        LedBlend::Classic => (mix((1.0 - x).powi(3), x.powf(2.2)), 1.0),
        LedBlend::Smooth | LedBlend::Soft => {
            let w = x * x * (3.0 - 2.0 * x);
            (mix(1.0 - w, w), 1.0)
        }
    }
}

/// How far LED `index` has turned from background to level colour at `level` (0..100). Each LED owns an equal share
/// of the dial; with the Soft fill its fade reaches into its neighbours' shares, so two LEDs change together.
fn led_fill(blend: LedBlend, level: f64, index: usize) -> f64 {
    let per_led = 100.0 / f64::from(saturate_u8(LEDS_PER_KNOB));
    let start = f64::from(saturate_u8(index)) * per_led;
    let (start, end) = if blend == LedBlend::Soft {
        let reach = per_led * SOFT_REACH;
        ((start - reach).max(0.0), (start + per_led + reach).min(100.0))
    } else {
        (start, start + per_led)
    };
    ((level - start) / (end - start)).clamp(0.0, 1.0)
}

/// Dims a colour to `factor` of its brightness, correcting each channel by `gamma` so the LED keeps its tint.
pub fn dim(color: Rgb, factor: f64, gamma: [f64; 3]) -> Rgb {
    let factor = factor.clamp(0.0, 1.0);
    let channel = |c: u8, g: f64| round_u8(f64::from(c) * factor.powf(1.0 / g.clamp(0.1, 10.0)));
    (channel(color.0, gamma[0]), channel(color.1, gamma[1]), channel(color.2, gamma[2]))
}

pub fn knob_colors(
    look: &KnobLook,
    brightness: u8,
    dim_idle: bool,
    gamma: [f64; 3],
    blend: LedBlend,
) -> [Rgb; LEDS_PER_KNOB] {
    let Some(level) = look.level else {
        return [(0, 0, 0); LEDS_PER_KNOB];
    };
    let mut factor = f64::from(brightness.min(100)) / 100.0;
    if dim_idle && look.idle {
        factor *= IDLE_DIM;
    }
    factor *= look.scale.clamp(0.0, 1.0);
    if look.muted {
        return [dim(look.muted_color, factor, gamma); LEDS_PER_KNOB];
    }
    std::array::from_fn(|i| {
        let fill = led_fill(blend, level, i);
        let (colour, share) = edge_for(blend, look.background, look.color, fill);
        dim(colour, factor * share, gamma)
    })
}

pub fn colors(
    looks: &[KnobLook; KNOBS],
    brightness: u8,
    dim_idle: bool,
    gamma: [f64; 3],
    blend: LedBlend,
) -> [[Rgb; LEDS_PER_KNOB]; KNOBS] {
    std::array::from_fn(|k| knob_colors(&looks[k], brightness, dim_idle, gamma, blend))
}

/// What the mixer shows while calibrating: each knob's level colour at full brightness, then dimmed to the edge
/// LED's lowest brightness, then dimmer still, so tint drift is easy to see and correct.
pub fn calibration(colours: &[Rgb; KNOBS], brightness: u8, gamma: [f64; 3]) -> [[Rgb; LEDS_PER_KNOB]; KNOBS] {
    let full = f64::from(brightness.min(100)) / 100.0;
    std::array::from_fn(|k| {
        [dim(colours[k], full, gamma), dim(colours[k], full * EDGE_FLOOR, gamma), dim(colours[k], full * 0.3, gamma)]
    })
}

/// The guided calibration's side by side: white at full, dimmer and dimmest, with correction `first` on the two left
/// knobs and `second` on the two right ones. The middle knob stays dark to keep the sides apart.
pub fn comparison(first: [f64; 3], second: [f64; 3], brightness: u8) -> [[Rgb; LEDS_PER_KNOB]; KNOBS] {
    const WHITE: Rgb = (255, 255, 255);
    const DARK: [Rgb; LEDS_PER_KNOB] = [(0, 0, 0); LEDS_PER_KNOB];
    let full = f64::from(brightness.min(100)) / 100.0;
    let steps = |gamma| [dim(WHITE, full, gamma), dim(WHITE, full * EDGE_FLOOR, gamma), dim(WHITE, full * 0.3, gamma)];
    [steps(first), steps(first), DARK, steps(second), steps(second)]
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

    const PLAIN: [f64; 3] = [1.0; 3];

    fn knob_colors_plain(look: &KnobLook, brightness: u8, dim_idle: bool) -> [Rgb; LEDS_PER_KNOB] {
        knob_colors(look, brightness, dim_idle, PLAIN, LedBlend::Clean)
    }

    #[test]
    fn classic_and_smooth_fade_between_the_two_colours() {
        let middle = |blend, level| knob_colors(&look(level), 100, true, PLAIN, blend)[1];
        for blend in [LedBlend::Classic, LedBlend::Smooth] {
            assert_eq!(middle(blend, 33), (0, 0, 255), "{blend:?}");
            assert_eq!(middle(blend, 67), (255, 0, 0), "{blend:?}");
            let half = middle(blend, 50);
            assert!(half.0 > 0 && half.2 > 0, "{blend:?}: both colours in the mix at 50% ({half:?})");
        }
        // Turn Up's dips a little darker through the middle; the S-curve is an even half-and-half
        assert!(middle(LedBlend::Classic, 50).0 < middle(LedBlend::Smooth, 50).0);
        let even = middle(LedBlend::Smooth, 50);
        assert!(even.0.abs_diff(128) <= 1 && even.2.abs_diff(128) <= 1 && even.1 == 0, "{even:?}");
    }

    /// `led` is `colour` at some brightness (never another hue).
    fn is_shade_of(led: Rgb, colour: Rgb) -> bool {
        let f = |c: u8| f64::from(c);
        let k = f(led.0.max(led.1).max(led.2)) / f(colour.0.max(colour.1).max(colour.2));
        (f(led.0) - f(colour.0) * k).abs() <= 1.0
            && (f(led.1) - f(colour.1) * k).abs() <= 1.0
            && (f(led.2) - f(colour.2) * k).abs() <= 1.0
    }

    #[test]
    fn only_the_two_chosen_colours_never_dark() {
        assert_eq!(knob_colors_plain(&look(100), 100, true), [(255, 0, 0); 3]);
        assert_eq!(knob_colors_plain(&look(0), 100, true), [(0, 0, 255); 3]);
        for level in 0..=100 {
            for led in knob_colors_plain(&look(level), 100, true) {
                assert!(
                    is_shade_of(led, (255, 0, 0)) || is_shade_of(led, (0, 0, 255)),
                    "{level}%: {led:?} is a new hue"
                );
                assert!(led.0.max(led.2) >= 150, "{level}%: {led:?} too dark");
            }
        }
    }

    #[test]
    fn the_edge_led_dims_toward_the_boundary_and_switches_there() {
        let edge = |level| knob_colors_plain(&look(level), 100, true)[1];
        assert_eq!(edge(33), (0, 0, 255));
        assert!(edge(45).2 < 255 && edge(45).0 == 0, "background, dimming as the level arrives");
        assert!(edge(55).0 < 255 && edge(55).2 == 0, "past the middle: level colour, still dimmed");
        assert_eq!(edge(67), (255, 0, 0));
    }

    #[test]
    fn a_black_background_fades_the_one_colour_in() {
        let mut lamp = look(50);
        lamp.background = (0, 0, 0);
        let led = knob_colors_plain(&lamp, 100, true)[1];
        assert!(led.1 == 0 && led.2 == 0 && (120..=135).contains(&led.0), "{led:?}");
    }

    #[test]
    fn gamma_keeps_a_channel_brighter_when_dimmed() {
        let orange = (240, 120, 0);
        assert_eq!(dim(orange, 1.0, [2.0, 0.5, 1.0]), orange, "full brightness is never changed");
        assert_eq!(dim(orange, 0.5, PLAIN), (120, 60, 0));
        let corrected = dim(orange, 0.5, [1.0, 2.0, 1.0]);
        assert_eq!(corrected.0, 120);
        assert!(corrected.1 > 60, "green raised: stays brighter when dimmed ({corrected:?})");
        let cal = calibration(&[orange; KNOBS], 100, PLAIN);
        assert_eq!(cal[0][0], orange);
        assert!(cal[0][1].0 < 240 && cal[0][2].0 < cal[0][1].0, "full, then dimmer, then dimmer still");
    }

    #[test]
    fn the_soft_fill_fades_neighbouring_leds_together() {
        let partly = |fill: f64| fill > 0.0 && fill < 1.0;
        let changing = |blend, level| (0..LEDS_PER_KNOB).filter(|&i| partly(led_fill(blend, level, i))).count();
        assert_eq!(changing(LedBlend::Classic, 30.0), 1, "one LED at a time steps around the knob");
        assert_eq!(changing(LedBlend::Soft, 30.0), 2, "two LEDs share the move, so it glides");
        for blend in [LedBlend::Classic, LedBlend::Soft] {
            assert!((0..LEDS_PER_KNOB).all(|i| led_fill(blend, 0.0, i) == 0.0), "{blend:?}: empty at 0");
            assert!(
                (0..LEDS_PER_KNOB).all(|i| (led_fill(blend, 100.0, i) - 1.0).abs() < f64::EPSILON),
                "{blend:?}: full at 100"
            );
            let mut previous = [0.0; LEDS_PER_KNOB];
            for level in 0..=100 {
                let fills: [f64; LEDS_PER_KNOB] = std::array::from_fn(|i| led_fill(blend, f64::from(level), i));
                assert!(
                    fills.iter().zip(previous).all(|(now, before)| *now >= before),
                    "{blend:?}: only ever fills up"
                );
                previous = fills;
            }
        }
    }

    #[test]
    fn comparison_shows_two_corrections_apart() {
        let pattern = comparison(PLAIN, [2.0, 1.0, 1.0], 100);
        assert_eq!(pattern[0], pattern[1], "the two left knobs show the first correction");
        assert_eq!(pattern[3], pattern[4], "the two right knobs show the second");
        assert_eq!(pattern[2], [(0, 0, 0); LEDS_PER_KNOB], "the middle knob is dark");
        assert_eq!(pattern[0][0], (255, 255, 255), "both start from full white");
        assert!(pattern[3][2].0 > pattern[0][2].0, "more red correction keeps red brighter when dimmed");
    }

    #[test]
    fn muted_idle_brightness_and_unknown() {
        let mut muted = look(80);
        muted.muted = true;
        assert_eq!(knob_colors_plain(&muted, 100, true), [(40, 0, 0); 3]);
        assert_eq!(knob_colors_plain(&look(100), 50, true)[0], (128, 0, 0));
        let mut idle = look(100);
        idle.idle = true;
        assert_eq!(knob_colors_plain(&idle, 100, true)[0], (64, 0, 0));
        assert_eq!(knob_colors_plain(&idle, 100, false)[0], (255, 0, 0));
        idle.level = None;
        assert_eq!(knob_colors_plain(&idle, 100, true), [(0, 0, 0); 3]);
    }
}
