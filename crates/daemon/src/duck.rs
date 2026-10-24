//! Auto-lower timing (SPEC.md section 6): how far the lowered channels are turned down right now.

use std::time::{Duration, Instant};

const FADE_DOWN: f64 = 0.25;
const FADE_UP: f64 = 0.8;
/// Stay down this long after the last loud moment, so normal pauses in speech don't pump the volume.
const HOLD: Duration = Duration::from_millis(1200);
/// Longest step one update may take, so a late first update doesn't jump straight down.
const MAX_STEP: f64 = 0.1;

#[derive(Debug)]
pub struct Ducker {
    /// 1.0 = normal, `low` = fully lowered.
    factor: f64,
    last_loud: Option<Instant>,
    last_update: Option<Instant>,
}

impl Default for Ducker {
    fn default() -> Self {
        Self { factor: 1.0, last_loud: None, last_update: None }
    }
}

impl Ducker {
    pub fn factor(&self) -> f64 {
        self.factor
    }

    pub fn loud(&mut self, now: Instant) {
        self.last_loud = Some(now);
    }

    fn holding(&self, now: Instant) -> bool {
        self.last_loud.is_some_and(|t| now.duration_since(t) < HOLD)
    }

    /// Something is still moving or about to (the caller should keep ticking).
    pub fn busy(&self, now: Instant) -> bool {
        self.factor < 1.0 || self.holding(now)
    }

    /// Advance the fade. `low` is the fully lowered factor (0.7 for "30%"). Returns whether the factor changed.
    pub fn update(&mut self, now: Instant, low: f64) -> bool {
        let step = self.last_update.map_or(0.0, |t| now.duration_since(t).as_secs_f64().min(MAX_STEP));
        self.last_update = Some(now);
        let before = self.factor;
        let low = low.clamp(0.0, 1.0);
        let range = 1.0 - low;
        self.factor = if range <= 0.0 {
            1.0
        } else if self.holding(now) {
            (self.factor - range * step / FADE_DOWN).max(low)
        } else {
            (self.factor + range * step / FADE_UP).min(1.0)
        };
        (self.factor - before).abs() > f64::EPSILON
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(ducker: &mut Ducker, from: Instant, millis: u64, low: f64) -> Instant {
        let mut now = from;
        for _ in 0..millis / 50 {
            now += Duration::from_millis(50);
            ducker.update(now, low);
        }
        now
    }

    #[test]
    fn fades_down_holds_and_fades_back() {
        let start = Instant::now();
        let mut d = Ducker::default();
        d.update(start, 0.7);
        d.loud(start);
        let t = run(&mut d, start, 100, 0.7);
        assert!(d.factor() < 1.0 && d.factor() > 0.7, "partway down after 0.1 s: {}", d.factor());
        let t = run(&mut d, t, 200, 0.7);
        assert!((d.factor() - 0.7).abs() < 1e-9, "fully down after 0.3 s");
        let t = run(&mut d, t, 850, 0.7);
        assert!((d.factor() - 0.7).abs() < 1e-9, "still down 1.15 s after the last loud moment");
        let t = run(&mut d, t, 400, 0.7);
        assert!(d.factor() > 0.7 && d.factor() < 1.0, "coming back up");
        run(&mut d, t, 800, 0.7);
        assert_eq!(d.factor(), 1.0);
        assert!(!d.busy(t + Duration::from_secs(1)));
    }

    #[test]
    fn a_late_first_update_does_not_jump() {
        let start = Instant::now();
        let mut d = Ducker::default();
        d.update(start, 0.5);
        d.loud(start + Duration::from_secs(5));
        d.update(start + Duration::from_secs(5), 0.5);
        assert!(d.factor() > 0.75, "{}", d.factor());
    }

    #[test]
    fn amount_off_never_lowers() {
        let start = Instant::now();
        let mut d = Ducker::default();
        d.loud(start);
        run(&mut d, start, 500, 1.0);
        assert_eq!(d.factor(), 1.0);
    }
}
