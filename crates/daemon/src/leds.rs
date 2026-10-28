//! Knob LED colours (SPEC.md section 5). Brightness and colour mixes are worked out the way eyes see them and turned
//! into LED power only at the very end: an LED's light grows in step with its power, but eyes see light on a curve
//! (half the power looks about three quarters as bright), so straight-line fades seem to jump and the LEDs show as
//! steps. Colours mix in `OKLab`, where the halfway point looks halfway, with no grey or dark dip and no hue nobody
//! picked.

use cranked_protocol::convert::{round_u8, saturate_u8};
use cranked_protocol::{Rgb, KNOBS, LEDS_PER_KNOB};

/// How bright an idle knob (nothing on it playing) looks, as a share of full.
const IDLE_LOOK: f64 = 0.5;
/// The calibration patterns' three steps, in how bright they look: full, dimmer, dimmest.
const CALIBRATION_STEPS: [f64; LEDS_PER_KNOB] = [1.0, 0.6, 0.3];

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
    /// Extra dimming (Auto-lower), in how bright it looks; 1.0 = none.
    pub scale: f64,
}

/// The LED power (0..1) that looks `lightness` (0..1) as bright as full power: the CIE 1931 lightness curve, the one
/// the Linux kernel's PWM backlight driver uses so brightness steps look even.
pub fn power_for_lightness(lightness: f64) -> f64 {
    let lightness = lightness.clamp(0.0, 1.0) * 100.0;
    if lightness <= 8.0 {
        lightness / 903.3
    } else {
        ((lightness + 16.0) / 116.0).powi(3)
    }
}

/// Dims a colour so it looks `lightness` as bright, correcting each channel by `gamma` (the LED calibration) so it
/// keeps its tint.
pub fn dim(color: Rgb, lightness: f64, gamma: [f64; 3]) -> Rgb {
    let power = power_for_lightness(lightness);
    let channel =
        |value: u8, correction: f64| round_u8(f64::from(value) * power.powf(1.0 / correction.clamp(0.1, 10.0)));
    (channel(color.0, gamma[0]), channel(color.1, gamma[1]), channel(color.2, gamma[2]))
}

/// `amount` (0..1) of the way from `from` to `to`, mixed in `OKLab`.
pub fn mix(from: Rgb, to: Rgb, amount: f64) -> Rgb {
    let amount = amount.clamp(0.0, 1.0);
    let (start, end) = (to_oklab(from), to_oklab(to));
    from_oklab(std::array::from_fn(|i| start[i] + (end[i] - start[i]) * amount))
}

/// An sRGB channel as linear light (0..1).
fn to_linear(value: u8) -> f64 {
    let encoded = f64::from(value) / 255.0;
    if encoded <= 0.04045 {
        encoded / 12.92
    } else {
        ((encoded + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear light (0..1) back to an sRGB channel.
fn to_encoded(linear: f64) -> u8 {
    let linear = linear.clamp(0.0, 1.0);
    let encoded = if linear <= 0.003_130_8 { linear * 12.92 } else { 1.055 * linear.powf(1.0 / 2.4) - 0.055 };
    round_u8(encoded * 255.0)
}

/// `OKLab` (lightness, green-red, blue-yellow), after Björn Ottosson's published matrices.
fn to_oklab(color: Rgb) -> [f64; 3] {
    let (red, green, blue) = (to_linear(color.0), to_linear(color.1), to_linear(color.2));
    let long = (0.412_221_470_8 * red + 0.536_332_536_3 * green + 0.051_445_992_9 * blue).cbrt();
    let medium = (0.211_903_498_2 * red + 0.680_699_545_1 * green + 0.107_396_956_6 * blue).cbrt();
    let short = (0.088_302_461_9 * red + 0.281_718_837_6 * green + 0.629_978_700_5 * blue).cbrt();
    [
        0.210_454_255_3 * long + 0.793_617_785_0 * medium - 0.004_072_046_8 * short,
        1.977_998_495_1 * long - 2.428_592_205_0 * medium + 0.450_593_709_9 * short,
        0.025_904_037_1 * long + 0.782_771_766_2 * medium - 0.808_675_766_0 * short,
    ]
}

fn from_oklab(lab: [f64; 3]) -> Rgb {
    let [lightness, green_red, blue_yellow] = lab;
    let long = (lightness + 0.396_337_777_4 * green_red + 0.215_803_757_3 * blue_yellow).powi(3);
    let medium = (lightness - 0.105_561_345_8 * green_red - 0.063_854_172_8 * blue_yellow).powi(3);
    let short = (lightness - 0.089_484_177_5 * green_red - 1.291_485_548_0 * blue_yellow).powi(3);
    (
        to_encoded(4.076_741_662_1 * long - 3.307_711_591_3 * medium + 0.230_969_929_2 * short),
        to_encoded(-1.268_438_004_6 * long + 2.609_757_401_1 * medium - 0.341_319_396_5 * short),
        to_encoded(-0.004_196_086_3 * long - 0.703_418_614_7 * medium + 1.707_614_701_0 * short),
    )
}

/// A knob's LEDs: they fill from the left as the level rises, each LED turning from the background colour to the
/// level colour across its share of the dial.
pub fn knob_colors(look: &KnobLook, brightness: u8, dim_idle: bool, gamma: [f64; 3]) -> [Rgb; LEDS_PER_KNOB] {
    let Some(level) = look.level else {
        return [(0, 0, 0); LEDS_PER_KNOB];
    };
    let mut lightness = f64::from(brightness.min(100)) / 100.0;
    if dim_idle && look.idle {
        lightness *= IDLE_LOOK;
    }
    lightness *= look.scale.clamp(0.0, 1.0);
    if look.muted {
        return [dim(look.muted_color, lightness, gamma); LEDS_PER_KNOB];
    }
    let per_led = 100.0 / f64::from(saturate_u8(LEDS_PER_KNOB));
    std::array::from_fn(|index| {
        let fill = ((level - f64::from(saturate_u8(index)) * per_led) / per_led).clamp(0.0, 1.0);
        dim(mix(look.background, look.color, fill), lightness, gamma)
    })
}

pub fn colors(
    looks: &[KnobLook; KNOBS],
    brightness: u8,
    dim_idle: bool,
    gamma: [f64; 3],
) -> [[Rgb; LEDS_PER_KNOB]; KNOBS] {
    std::array::from_fn(|k| knob_colors(&looks[k], brightness, dim_idle, gamma))
}

/// What the mixer shows while calibrating: each knob's level colour at full brightness, then dimmer, then dimmest,
/// so tint drift is easy to see and correct.
pub fn calibration(colours: &[Rgb; KNOBS], brightness: u8, gamma: [f64; 3]) -> [[Rgb; LEDS_PER_KNOB]; KNOBS] {
    let full = f64::from(brightness.min(100)) / 100.0;
    std::array::from_fn(|k| CALIBRATION_STEPS.map(|step| dim(colours[k], full * step, gamma)))
}

/// The guided calibration's side by side: white at full, dimmer and dimmest, with correction `first` on the two left
/// knobs and `second` on the two right ones. The middle knob stays dark to keep the sides apart.
pub fn comparison(first: [f64; 3], second: [f64; 3], brightness: u8) -> [[Rgb; LEDS_PER_KNOB]; KNOBS] {
    const WHITE: Rgb = (255, 255, 255);
    const DARK: [Rgb; LEDS_PER_KNOB] = [(0, 0, 0); LEDS_PER_KNOB];
    let full = f64::from(brightness.min(100)) / 100.0;
    let steps = |gamma| CALIBRATION_STEPS.map(|step| dim(WHITE, full * step, gamma));
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

    #[test]
    fn the_brightness_curve_looks_even() {
        assert!(power_for_lightness(0.0).abs() < 1e-9 && (power_for_lightness(1.0) - 1.0).abs() < 1e-9);
        let half = power_for_lightness(0.5);
        assert!((0.17..0.2).contains(&half), "looking half as bright takes about 18% of the power, got {half}");
        let mut previous = 0.0;
        for step in 1..=100 {
            let power = power_for_lightness(f64::from(step) / 100.0);
            assert!(power > previous, "always brighter as the lightness rises");
            previous = power;
        }
    }

    #[test]
    fn colours_mix_through_an_even_middle() {
        assert_eq!(mix((255, 0, 0), (0, 0, 255), 0.0), (255, 0, 0));
        assert_eq!(mix((255, 0, 0), (0, 0, 255), 1.0), (0, 0, 255));
        let middle = mix((255, 0, 0), (0, 0, 255), 0.5);
        let lightness = |color| to_oklab(color)[0];
        let (red, blue, purple) = (lightness((255, 0, 0)), lightness((0, 0, 255)), lightness(middle));
        assert!(
            (purple - f64::midpoint(red, blue)).abs() < 0.01,
            "looks halfway between the two, not darker: {middle:?}"
        );
        assert!(middle.0 > middle.1 + 40 && middle.2 > middle.1 + 40, "a purple, not a grey: {middle:?}");
        for color in [(255, 128, 0), (0, 200, 255), (17, 34, 51)] {
            assert_eq!(from_oklab(to_oklab(color)), color, "OKLab there and back keeps the colour");
        }
    }

    #[test]
    fn a_knob_fills_from_the_left() {
        let leds = |level| knob_colors(&look(level), 100, true, PLAIN);
        assert_eq!(leds(0), [(0, 0, 255); 3], "empty: all background");
        assert_eq!(leds(100), [(255, 0, 0); 3], "full: all level colour");
        assert_eq!(leds(50)[0], (255, 0, 0), "past the first LED's share it's fully the level colour");
        assert_eq!(leds(50)[2], (0, 0, 255), "the last one hasn't started");
        let edge = leds(50)[1];
        assert!(edge.0 > 0 && edge.2 > 0, "the LED the level is in is part way: {edge:?}");
    }

    #[test]
    fn a_black_background_fades_the_colour_in_evenly() {
        let mut lamp = look(0);
        lamp.background = (0, 0, 0);
        let mut brightness = |level| {
            lamp.level = Some(level);
            knob_colors(&lamp, 100, true, PLAIN)[0].0
        };
        let (quarter, half, three_quarters) = (brightness(100.0 / 12.0), brightness(100.0 / 6.0), brightness(25.0));
        assert!(quarter < half && half < three_quarters && three_quarters < 255);
        assert!((60..120).contains(&half), "half way looks half as bright: far below half power ({half})");
    }

    #[test]
    fn dimming_looks_even_and_the_calibration_keeps_the_tint() {
        let orange = (240, 120, 0);
        assert_eq!(dim(orange, 1.0, [2.0, 0.5, 1.0]), orange, "full brightness is never changed");
        let half = dim(orange, 0.5, PLAIN);
        assert!((40..=46).contains(&half.0), "looking half as bright is about 18% power: {half:?}");
        let corrected = dim(orange, 0.5, [1.0, 2.0, 1.0]);
        assert!(corrected.1 > half.1, "green raised: stays brighter when dimmed ({corrected:?})");
        let pattern = calibration(&[orange; KNOBS], 100, PLAIN);
        assert_eq!(pattern[0][0], orange);
        assert!(pattern[0][1].0 < 240 && pattern[0][2].0 < pattern[0][1].0, "full, then dimmer, then dimmer still");
    }

    #[test]
    fn idle_and_muted_knobs() {
        let mut quiet = look(100);
        quiet.idle = true;
        assert!(knob_colors(&quiet, 100, true, PLAIN)[0].0 < 255, "idle dims");
        assert_eq!(knob_colors(&quiet, 100, false, PLAIN)[0], (255, 0, 0), "unless that's turned off");
        let mut muted = look(60);
        muted.muted = true;
        assert_eq!(knob_colors(&muted, 100, true, PLAIN), [(40, 0, 0); 3]);
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
}
