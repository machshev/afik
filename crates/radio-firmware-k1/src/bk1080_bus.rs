//! Hardware-independent K1 BK1080 two-wire sequencing.
//!
//! The pins and edge order are the pinned K1 reference firmware's
//! (`EVID-BCFM-066-001`): clock `PF5`, data `PF6`, both push-pull with
//! pull-ups, the data line turned to input only to read. One microsecond
//! separates the edges, and an acknowledge is polled for at most 255 of them.
//! A board adapter supplies the pins, so the sequence is host testable.

use radio_bk1080::TwoWire;

/// Port F pin driving the clock.
pub const SCL_PIN: u8 = 5;
/// Port F pin carrying the data line.
pub const SDA_PIN: u8 = 6;
/// Microseconds an acknowledge is polled for.
pub const ACK_POLLS: u32 = 255;

/// Board primitives for one bit-banged transfer.
pub trait TwoWirePins {
    /// Drives the clock line.
    fn set_clock(&mut self, high: bool);
    /// Drives the data line, making it an output.
    fn drive_data(&mut self, high: bool);
    /// Turns the data line to an input.
    fn release_data(&mut self);
    /// Samples the data line.
    fn read_data(&mut self) -> bool;
    /// Waits one microsecond.
    fn tick(&mut self);
}

/// [`TwoWire`] over K1 pins.
pub struct BitBang<P>(pub P);

impl<P: TwoWirePins> BitBang<P> {
    fn clock(&mut self, high: bool) {
        self.0.set_clock(high);
        self.0.tick();
    }

    fn data(&mut self, high: bool) {
        self.0.drive_data(high);
        self.0.tick();
    }
}

impl<P: TwoWirePins> TwoWire for BitBang<P> {
    fn start(&mut self) {
        self.data(true);
        self.clock(true);
        self.data(false);
        self.clock(false);
    }

    fn stop(&mut self) {
        self.data(false);
        self.clock(false);
        self.clock(true);
        self.data(true);
    }

    fn write(&mut self, byte: u8) -> bool {
        self.clock(false);
        for bit in (0..8).rev() {
            self.data(byte & (1 << bit) != 0);
            self.clock(true);
            self.clock(false);
        }
        self.0.release_data();
        self.0.tick();
        self.clock(true);
        let mut acknowledged = false;
        for _ in 0..ACK_POLLS {
            if !self.0.read_data() {
                acknowledged = true;
                break;
            }
            self.0.tick();
        }
        self.clock(false);
        self.0.drive_data(true);
        acknowledged
    }

    fn read(&mut self, final_byte: bool) -> u8 {
        self.0.tick();
        self.0.release_data();
        let mut value = 0;
        for _ in 0..8 {
            self.clock(false);
            self.clock(true);
            self.0.tick();
            value = (value << 1) | u8::from(self.0.read_data());
            self.clock(false);
        }
        self.clock(false);
        // An acknowledge holds the line low, a final byte leaves it high.
        self.data(final_byte);
        self.clock(true);
        self.clock(false);
        value
    }
}

#[cfg(test)]
mod tests {
    use super::{BitBang, TwoWirePins, ACK_POLLS};
    use radio_bk1080::TwoWire;
    use std::vec::Vec;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Edge {
        Clock(bool),
        Data(bool),
        Release,
    }

    #[derive(Default)]
    struct Pins {
        edges: Vec<Edge>,
        input: Vec<bool>,
        ticks: u32,
    }

    impl TwoWirePins for Pins {
        fn set_clock(&mut self, high: bool) {
            self.edges.push(Edge::Clock(high));
        }
        fn drive_data(&mut self, high: bool) {
            self.edges.push(Edge::Data(high));
        }
        fn release_data(&mut self) {
            self.edges.push(Edge::Release);
        }
        fn read_data(&mut self) -> bool {
            if self.input.is_empty() {
                true
            } else {
                self.input.remove(0)
            }
        }
        fn tick(&mut self) {
            self.ticks += 1;
        }
    }

    #[test]
    fn start_and_stop_move_data_while_the_clock_is_high() {
        use Edge::{Clock, Data};
        let mut bus = BitBang(Pins::default());
        bus.start();
        bus.stop();
        assert_eq!(
            bus.0.edges,
            [
                Data(true),
                Clock(true),
                Data(false),
                Clock(false),
                Data(false),
                Clock(false),
                Clock(true),
                Data(true),
            ]
        );
    }

    #[test]
    fn a_byte_goes_out_most_significant_first_and_reads_the_acknowledge() {
        let mut bus = BitBang(Pins {
            input: std::vec![false],
            ..Pins::default()
        });
        assert!(bus.write(0x80));
        let data: Vec<bool> = bus
            .0
            .edges
            .iter()
            .filter_map(|edge| match edge {
                Edge::Data(high) => Some(*high),
                _ => None,
            })
            .collect();
        // Eight bits, then the line driven high again after the acknowledge.
        assert_eq!(
            data,
            [true, false, false, false, false, false, false, false, true]
        );
        assert!(bus.0.edges.contains(&Edge::Release));
    }

    #[test]
    fn a_silent_device_is_not_acknowledged_after_the_bounded_poll() {
        let mut bus = BitBang(Pins::default());
        let before = bus.0.ticks;
        assert!(!bus.write(0x80));
        assert!(bus.0.ticks - before >= ACK_POLLS);
    }

    #[test]
    fn a_read_samples_eight_bits_and_answers_ack_or_nack() {
        let mut bus = BitBang(Pins {
            input: std::vec![true, false, true, false, false, false, true, true],
            ..Pins::default()
        });
        assert_eq!(bus.read(false), 0xA3);
        assert_eq!(bus.0.edges.iter().rev().nth(2), Some(&Edge::Data(false)));
        let mut bus = BitBang(Pins::default());
        assert_eq!(bus.read(true), 0xFF);
        assert_eq!(bus.0.edges.iter().rev().nth(2), Some(&Edge::Data(true)));
    }
}
