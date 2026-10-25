//! Number conversions that can't silently wrap. Each narrowing cast lives here once, reviewed, instead of
//! `as` scattered through the code (where 300 as u8 quietly becomes 44).

/// Narrows an integer to `u8`, saturating: negatives become 0 and anything above 255 becomes 255.
#[must_use]
pub fn saturate_u8<T>(value: T) -> u8
where
    T: TryInto<u8> + PartialOrd + Default,
{
    if value < T::default() {
        0
    } else {
        value.try_into().unwrap_or(u8::MAX)
    }
}

/// Narrows an integer to `u16`, saturating like [`saturate_u8`].
#[must_use]
pub fn saturate_u16<T>(value: T) -> u16
where
    T: TryInto<u16> + PartialOrd + Default,
{
    if value < T::default() {
        0
    } else {
        value.try_into().unwrap_or(u16::MAX)
    }
}

/// Narrows an unsigned integer to `i32`, saturating at `i32::MAX` (UI toolkits count in i32).
#[must_use]
pub fn saturate_i32<T>(value: T) -> i32
where
    T: TryInto<i32>,
{
    value.try_into().unwrap_or(i32::MAX)
}

/// Rounds to the nearest `u8`, clamped to 0..=255; NaN becomes 0.
#[must_use]
pub fn round_u8(value: f64) -> u8 {
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "clamped to 0..=255 first")]
    let byte = value.round().clamp(0.0, 255.0) as u8;
    byte
}

/// Converts a float to `f32` for UI toolkits, which only take f32 (loses precision past ~7 digits).
#[must_use]
pub fn to_f32(value: f64) -> f32 {
    #[expect(clippy::cast_possible_truncation, reason = "f32 is what the UI takes; settings never need more")]
    let narrow = value as f32;
    narrow
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_saturate_instead_of_wrapping() {
        assert_eq!(saturate_u8(300_u32), 255);
        assert_eq!(saturate_u8(-5_i32), 0);
        assert_eq!(saturate_u8(42_usize), 42);
        assert_eq!(saturate_u16(70_000_u32), u16::MAX);
        assert_eq!(saturate_i32(usize::MAX), i32::MAX);
        assert_eq!(saturate_i32(7_usize), 7);
    }

    #[test]
    fn floats_round_and_clamp() {
        assert_eq!(round_u8(41.5), 42);
        assert_eq!(round_u8(-3.0), 0);
        assert_eq!(round_u8(1e9), 255);
        assert_eq!(round_u8(f64::NAN), 0);
        assert!((f64::from(to_f32(2.2)) - 2.2).abs() < 1e-6);
    }
}
