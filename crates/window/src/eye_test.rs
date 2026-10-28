//! The guided LED calibration: an eye test that finds each colour's dimming correction from "which side looks better"
//! answers, one colour at a time. Each round shows two corrections a third apart across what's left of the range
//! (counted on a log scale, since a correction is a ratio) and keeps the part the answer points to.

use cranked_ipc::settings::GAMMA_RANGE;

/// Rounds per colour: each one keeps two thirds of the range, so five leave about an eighth of it.
pub const ROUNDS_PER_COLOUR: u8 = 5;
/// The colours in the order they are tuned.
pub const COLOUR_NAMES: [&str; 3] = ["red", "green", "blue"];

/// What the person said about the two sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Left,
    Right,
    Same,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EyeTest {
    /// The correction found so far; the colour being tuned is replaced by each candidate.
    best: [f64; 3],
    /// The colour being tuned (0 red, 1 green, 2 blue); 3 once all are done.
    colour: usize,
    /// What's left of the range for this colour, as natural logs of the correction.
    low: f64,
    high: f64,
    round: u8,
}

impl EyeTest {
    /// Starts from `current`, the correction in use now; each colour is searched across the whole allowed range.
    pub fn new(current: [f64; 3]) -> Self {
        let (low, high) = GAMMA_RANGE;
        Self { best: current, colour: 0, low: low.ln(), high: high.ln(), round: 0 }
    }

    pub fn is_done(&self) -> bool {
        self.colour >= COLOUR_NAMES.len()
    }

    /// The corrections to show this round: the left side and the right side.
    pub fn candidates(&self) -> ([f64; 3], [f64; 3]) {
        let third = (self.high - self.low) / 3.0;
        (self.with_colour(self.low + third), self.with_colour(self.high - third))
    }

    /// Narrows the range toward the answer; after the last round of a colour, settles it and moves to the next.
    pub fn answer(&mut self, answer: Answer) {
        if self.is_done() {
            return;
        }
        let third = (self.high - self.low) / 3.0;
        match answer {
            Answer::Left => self.high -= third,
            Answer::Right => self.low += third,
            Answer::Same => {
                self.low += third / 2.0;
                self.high -= third / 2.0;
            }
        }
        self.round += 1;
        if self.round >= ROUNDS_PER_COLOUR {
            if let Some(channel) = self.best.get_mut(self.colour) {
                *channel = round_correction(f64::midpoint(self.low, self.high).exp());
            }
            let (low, high) = GAMMA_RANGE;
            (self.colour, self.low, self.high, self.round) = (self.colour + 1, low.ln(), high.ln(), 0);
        }
    }

    /// The correction found so far (all of it once `is_done`).
    pub fn result(&self) -> [f64; 3] {
        self.best
    }

    /// Where the test is, for the window: "Red, round 2 of 5".
    pub fn progress(&self) -> String {
        match COLOUR_NAMES.get(self.colour) {
            Some(name) => {
                let mut title = (*name).to_string();
                if let Some(first) = title.get_mut(..1) {
                    first.make_ascii_uppercase();
                }
                format!("{title}, round {} of {ROUNDS_PER_COLOUR}", self.round + 1)
            }
            None => "Done".into(),
        }
    }

    fn with_colour(&self, log_correction: f64) -> [f64; 3] {
        let mut gamma = self.best;
        if let Some(channel) = gamma.get_mut(self.colour) {
            *channel = round_correction(log_correction.exp());
        }
        gamma
    }
}

/// Two decimals, like the settings file keeps them.
fn round_correction(correction: f64) -> f64 {
    let (low, high) = GAMMA_RANGE;
    ((correction * 100.0).round() / 100.0).clamp(low, high)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plays the test against someone whose ideal correction is `ideal`: they pick the side closer to it.
    fn run(ideal: [f64; 3]) -> [f64; 3] {
        let mut test = EyeTest::new([1.0; 3]);
        while !test.is_done() {
            let colour = test.colour;
            let (left, right) = test.candidates();
            let distance = |gamma: [f64; 3]| (gamma[colour].ln() - ideal[colour].ln()).abs();
            let (left, right) = (distance(left), distance(right));
            test.answer(if (left - right).abs() < 1e-9 {
                Answer::Same
            } else if left < right {
                Answer::Left
            } else {
                Answer::Right
            });
        }
        test.result()
    }

    #[test]
    fn finds_each_colour_close_to_what_the_eye_prefers() {
        for ideal in [[1.0, 1.0, 1.0], [1.6, 0.8, 1.2], [0.5, 2.2, 0.9]] {
            let found = run(ideal);
            for colour in 0..3 {
                let off = (found[colour].ln() - ideal[colour].ln()).abs();
                assert!(off < 0.12, "{} off by {off:.3}: found {found:?}, ideal {ideal:?}", COLOUR_NAMES[colour]);
            }
        }
    }

    #[test]
    fn walks_through_the_colours_in_rounds() {
        let mut test = EyeTest::new([1.0; 3]);
        assert_eq!(test.progress(), "Red, round 1 of 5");
        let (left, right) = test.candidates();
        assert!(left[0] < right[0], "left tries less correction than right");
        assert_eq!((left[1], left[2]), (1.0, 1.0), "only the colour being tuned changes");
        for _ in 0..ROUNDS_PER_COLOUR {
            test.answer(Answer::Same);
        }
        assert_eq!(test.progress(), "Green, round 1 of 5");
        for _ in 0..2 * ROUNDS_PER_COLOUR {
            test.answer(Answer::Left);
        }
        assert!(test.is_done());
        assert_eq!(test.progress(), "Done");
        let result = test.result();
        assert!(result.iter().all(|c| (GAMMA_RANGE.0..=GAMMA_RANGE.1).contains(c)), "{result:?}");
        assert!(result[1] < 1.0 && result[2] < 1.0, "always left means less correction");
    }
}
