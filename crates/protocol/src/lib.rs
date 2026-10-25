//! Mixer wire format and knob filtering (SPEC.md sections 1 and 2).
//!
//! No I/O here: feed bytes in, get events out, so everything is unit-testable without hardware.

pub mod convert;

use convert::{saturate_u16, saturate_u8};

pub const KNOBS: usize = 5;
pub const LEDS_PER_KNOB: usize = 3;
pub const RAW_MAX: u16 = 1023;

/// One input event from the mixer (or from the on-screen controls, which use the same format).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Knob { channel: u8, raw: u16 },
    ButtonDown { channel: u8 },
    ButtonUp { channel: u8 },
}

/// Incremental packet decoder. Keeps a partial packet between calls.
#[derive(Debug, Default)]
pub struct Decoder {
    pending: Vec<u8>,
}

impl Decoder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append bytes and return every complete event, in order.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Event> {
        self.pending.extend_from_slice(bytes);
        let mut events = Vec::new();
        let mut at = 0;
        while self.pending.len() >= at + 3 {
            if self.pending[at] != 0xFF || self.pending[at + 1] != 0xFE {
                at += 1;
                continue;
            }
            let length = match self.pending[at + 2] {
                0x02 => 3,
                0x03 => 6,
                0x06 | 0x07 => 4,
                _ => {
                    at += 2;
                    continue;
                }
            };
            if self.pending.len() < at + length {
                break;
            }
            let packet = &self.pending[at..at + length];
            match *packet {
                [_, _, 0x03, channel, high, low] => {
                    events.push(Event::Knob { channel, raw: u16::from_be_bytes([high, low]) });
                }
                [_, _, 0x06, channel, ..] => events.push(Event::ButtonDown { channel }),
                [_, _, 0x07, channel, ..] => events.push(Event::ButtonUp { channel }),
                _ => {}
            }
            at += length;
        }
        self.pending.drain(..at);
        // never let garbage grow without bound
        if self.pending.len() > 128 {
            let keep = self.pending.len() - 5;
            self.pending.drain(..keep);
        }
        events
    }
}

/// Encode a knob position the way the mixer reports it (used by the on-screen controls).
#[must_use]
pub fn knob_packet(channel: u8, percent: u8) -> [u8; 6] {
    let [high, low] = percent_to_raw(percent).to_be_bytes();
    [0xFF, 0xFE, 0x03, channel, high, low]
}

#[must_use]
pub fn button_packet(channel: u8, down: bool) -> [u8; 4] {
    [0xFF, 0xFE, if down { 0x06 } else { 0x07 }, channel]
}

pub type Rgb = (u8, u8, u8);

/// LED frame: `FE 05`, then knobs 0..4 x LEDs left/middle/right x RGB, then `FF` (48 bytes).
#[must_use]
pub fn led_frame(colors: &[[Rgb; LEDS_PER_KNOB]; KNOBS]) -> [u8; 48] {
    let mut frame = [0_u8; 48];
    frame[0] = 0xFE;
    frame[1] = 0x05;
    let mut at = 2;
    for knob in colors {
        for &(r, g, b) in knob {
            frame[at] = r;
            frame[at + 1] = g;
            frame[at + 2] = b;
            at += 3;
        }
    }
    frame[47] = 0xFF;
    frame
}

/// The pots never quite reach their ends, so 0..100% is spread over this raw range (about 2%..98% of travel):
/// both ends are always reachable and every step in between exists (no jump from 3% straight to 0%).
const RAW_LOW: u16 = 20;
const RAW_HIGH: u16 = 1000;

/// Raw 0..1023 to percent.
#[must_use]
pub fn raw_to_percent(raw: u16) -> u8 {
    let span = u32::from(RAW_HIGH - RAW_LOW);
    let above = u32::from(raw.clamp(RAW_LOW, RAW_HIGH) - RAW_LOW);
    saturate_u8((above * 100 + span / 2) / span)
}

/// The raw reading that means `percent` (the inverse of [`raw_to_percent`]).
#[must_use]
pub fn percent_to_raw(percent: u8) -> u16 {
    let span = u32::from(RAW_HIGH - RAW_LOW);
    RAW_LOW + saturate_u16((u32::from(percent.min(100)) * span + 50) / 100)
}

const JUMP_CONFIRM: i16 = 25;
const JUMP_TOLERANCE: i16 = 3;
const RAW_HYSTERESIS: i32 = 7;

/// Turns raw knob readings into accepted percentages: drops corrupt values, holds lone spikes until confirmed,
/// and only moves in 1% steps once the raw value has really moved (no boundary flicker).
#[derive(Debug, Default)]
pub struct KnobFilter {
    accepted: [Option<(u8, u16)>; KNOBS],
    pending_jump: [Option<u8>; KNOBS],
}

impl KnobFilter {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the position without filtering (startup sync).
    pub fn seed(&mut self, channel: usize, raw: u16) -> Option<u8> {
        if channel >= KNOBS || raw > RAW_MAX {
            return None;
        }
        let percent = raw_to_percent(raw);
        self.accepted[channel] = Some((percent, raw));
        self.pending_jump[channel] = None;
        Some(percent)
    }

    /// Returns the new percentage if this reading should be acted on.
    pub fn feed(&mut self, channel: usize, raw: u16) -> Option<u8> {
        if channel >= KNOBS || raw > RAW_MAX {
            return None;
        }
        let percent = raw_to_percent(raw);
        let Some((previous, previous_raw)) = self.accepted[channel] else {
            return self.seed(channel, raw);
        };
        let pending = self.pending_jump[channel].take();
        if (i16::from(percent) - i16::from(previous)).abs() >= JUMP_CONFIRM {
            let confirmed = pending.is_some_and(|p| (i16::from(p) - i16::from(percent)).abs() <= JUMP_TOLERANCE);
            if !confirmed {
                self.pending_jump[channel] = Some(percent);
                return None;
            }
        }
        if percent == previous {
            return None;
        }
        let moved_enough = (i32::from(raw) - i32::from(previous_raw)).abs() >= RAW_HYSTERESIS;
        if !moved_enough && percent != 0 && percent != 100 {
            return None;
        }
        self.accepted[channel] = Some((percent, raw));
        Some(percent)
    }

    #[must_use]
    pub fn position(&self, channel: usize) -> Option<u8> {
        self.accepted.get(channel).copied().flatten().map(|(p, _)| p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_for(percent: u8) -> u16 {
        percent_to_raw(percent)
    }

    #[test]
    fn decodes_knob_button_and_heartbeat_packets_across_chunks() {
        let mut decoder = Decoder::new();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0xFF, 0xFE, 0x02]);
        bytes.extend_from_slice(&knob_packet(2, 50));
        bytes.extend_from_slice(&button_packet(1, true));
        bytes.extend_from_slice(&button_packet(1, false));
        let (first, second) = bytes.split_at(7);
        let mut events = decoder.push(first);
        events.extend(decoder.push(second));
        assert_eq!(
            events,
            vec![
                Event::Knob { channel: 2, raw: percent_to_raw(50) },
                Event::ButtonDown { channel: 1 },
                Event::ButtonUp { channel: 1 },
            ]
        );
    }

    #[test]
    fn resyncs_after_garbage_and_unknown_types() {
        let mut decoder = Decoder::new();
        let mut bytes = vec![0x00, 0x13, 0xFF, 0xFE, 0x42];
        bytes.extend_from_slice(&knob_packet(0, 100));
        assert_eq!(decoder.push(&bytes), vec![Event::Knob { channel: 0, raw: RAW_HIGH }]);
    }

    #[test]
    fn led_frame_layout() {
        let mut colors = [[(0, 0, 0); 3]; 5];
        colors[0][0] = (1, 2, 3);
        colors[4][2] = (7, 8, 9);
        let frame = led_frame(&colors);
        assert_eq!(&frame[..5], &[0xFE, 0x05, 1, 2, 3]);
        assert_eq!(&frame[44..], &[7, 8, 9, 0xFF]);
    }

    #[test]
    fn ends_are_reachable_and_every_step_exists() {
        assert_eq!(raw_to_percent(0), 0);
        assert_eq!(raw_to_percent(20), 0);
        assert_eq!(raw_to_percent(1000), 100);
        assert_eq!(raw_to_percent(1023), 100);
        assert_eq!(raw_to_percent(512), 50);
        for p in 0..=100 {
            assert_eq!(raw_to_percent(percent_to_raw(p)), p);
        }
    }

    #[test]
    fn turning_down_to_zero_passes_three_two_one() {
        let mut filter = KnobFilter::new();
        filter.seed(0, raw_for(6));
        let steps: Vec<_> = (0..=raw_for(6)).rev().filter_map(|raw| filter.feed(0, raw)).collect();
        assert_eq!(steps, vec![5, 4, 3, 2, 1, 0]);
    }

    #[test]
    fn corrupt_reading_is_dropped() {
        let mut filter = KnobFilter::new();
        filter.seed(0, raw_for(50));
        assert_eq!(filter.feed(0, 0xFE03), None);
        assert_eq!(filter.position(0), Some(50));
    }

    #[test]
    fn lone_spike_is_ignored_and_confirmed_jump_is_applied() {
        let mut filter = KnobFilter::new();
        filter.seed(0, raw_for(50));
        assert_eq!(filter.feed(0, raw_for(100)), None);
        assert_eq!(filter.feed(0, raw_for(50)), None);
        assert_eq!(filter.feed(0, raw_for(90)), None);
        assert_eq!(filter.feed(0, raw_for(90)), Some(90));
    }

    #[test]
    fn slow_turn_moves_in_one_percent_steps() {
        let mut filter = KnobFilter::new();
        filter.seed(1, raw_for(50));
        let steps: Vec<_> = (51..=56).filter_map(|p| filter.feed(1, raw_for(p) + 2)).collect();
        assert_eq!(steps, vec![51, 52, 53, 54, 55, 56]);
    }

    #[test]
    fn knob_on_a_boundary_does_not_flicker() {
        let mut filter = KnobFilter::new();
        filter.seed(1, 512);
        let changes: Vec<_> = [516_u16, 517].repeat(10).into_iter().filter_map(|raw| filter.feed(1, raw)).collect();
        assert!(changes.is_empty(), "{changes:?}");
    }
}
