//! Press vs hold (SPEC.md section 4). No I/O: down/up/tick in, gestures out.

use std::time::{Duration, Instant};

use cranked_ipc::settings::MAX_CONTROLS as KNOBS;

pub const HOLD_AFTER: Duration = Duration::from_millis(500);
const REPEAT_GUARD: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gesture {
    Press(usize),
    Hold(usize),
}

#[derive(Debug, Default)]
pub struct Buttons {
    down_at: [Option<Instant>; KNOBS],
    hold_fired: [bool; KNOBS],
    last_gesture: [Option<Instant>; KNOBS],
    /// Buttons with a hold action; the others count a long press as a press.
    pub has_hold: [bool; KNOBS],
}

impl Buttons {
    pub fn any_down(&self) -> bool {
        self.down_at.iter().any(Option::is_some)
    }

    pub fn is_down(&self, channel: usize) -> bool {
        self.down_at.get(channel).is_some_and(Option::is_some)
    }

    pub fn down(&mut self, channel: usize, now: Instant) {
        if channel >= KNOBS || self.down_at[channel].is_some() {
            return;
        }
        if self.last_gesture[channel].is_some_and(|t| now.duration_since(t) < REPEAT_GUARD) {
            return;
        }
        self.down_at[channel] = Some(now);
        self.hold_fired[channel] = false;
    }

    pub fn up(&mut self, channel: usize, now: Instant) -> Option<Gesture> {
        let down_at = self.down_at.get_mut(channel)?.take()?;
        if self.hold_fired[channel] {
            return None;
        }
        if self.has_hold[channel] && now.duration_since(down_at) >= HOLD_AFTER {
            // the release came before a tick noticed the hold
            self.last_gesture[channel] = Some(now);
            return Some(Gesture::Hold(channel));
        }
        self.last_gesture[channel] = Some(now);
        Some(Gesture::Press(channel))
    }

    /// Holds fire as soon as the button has been down long enough, without waiting for the release.
    pub fn tick(&mut self, now: Instant) -> Vec<Gesture> {
        let mut out = Vec::new();
        for channel in 0..KNOBS {
            if let Some(down_at) = self.down_at[channel] {
                if self.has_hold[channel] && !self.hold_fired[channel] && now.duration_since(down_at) >= HOLD_AFTER {
                    self.hold_fired[channel] = true;
                    self.last_gesture[channel] = Some(now);
                    out.push(Gesture::Hold(channel));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buttons() -> Buttons {
        // button 1 has no hold action, the rest do
        Buttons { has_hold: std::array::from_fn(|i| i != 1), ..Buttons::default() }
    }

    #[test]
    fn short_press() {
        let t = Instant::now();
        let mut b = buttons();
        b.down(0, t);
        assert!(b.tick(t + Duration::from_millis(100)).is_empty());
        assert_eq!(b.up(0, t + Duration::from_millis(150)), Some(Gesture::Press(0)));
    }

    #[test]
    fn hold_fires_while_held_and_the_release_does_nothing() {
        let t = Instant::now();
        let mut b = buttons();
        b.down(0, t);
        assert_eq!(b.tick(t + HOLD_AFTER), [Gesture::Hold(0)]);
        assert!(b.tick(t + HOLD_AFTER * 2).is_empty());
        assert_eq!(b.up(0, t + HOLD_AFTER * 3), None);
    }

    #[test]
    fn long_press_without_a_hold_action_is_a_press() {
        let t = Instant::now();
        let mut b = buttons();
        b.down(1, t);
        assert!(b.tick(t + HOLD_AFTER).is_empty());
        assert_eq!(b.up(1, t + HOLD_AFTER * 2), Some(Gesture::Press(1)));
    }

    #[test]
    fn bounce_right_after_a_press_is_ignored() {
        let t = Instant::now();
        let mut b = buttons();
        b.down(2, t);
        b.up(2, t + Duration::from_millis(50));
        b.down(2, t + Duration::from_millis(120));
        assert_eq!(b.up(2, t + Duration::from_millis(150)), None);
        b.down(2, t + Duration::from_millis(400));
        assert_eq!(b.up(2, t + Duration::from_millis(450)), Some(Gesture::Press(2)));
    }

    #[test]
    fn release_without_press_and_bad_channels_are_ignored() {
        let t = Instant::now();
        let mut b = buttons();
        assert_eq!(b.up(3, t), None);
        b.down(20, t);
        assert_eq!(b.up(20, t), None);
    }
}
