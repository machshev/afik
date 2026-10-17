//! The standard CTCSS tones and DCS codes, as the reference lists them.
//!
//! `CTCSS_Options` and `DCS_Options` in both pinned trees. Only their packed
//! forms, [`CTCSS_STEPS`] and [`DCS_STANDARD`], are reached at run time, so an
//! image carries 113 bytes for both lists. The receiver finds the nearest tone
//! in them, and the handset's tone lists and its place record number tones by
//! their position here ([`standard_tone`]).

use crate::Tone;

/// `CTCSS_Options`, in tenths of a hertz.
const CTCSS_TONES: [u16; 50] = [
    670, 693, 719, 744, 770, 797, 825, 854, 885, 915, 948, 974, 1000, 1035, 1072, 1109, 1148, 1188,
    1230, 1273, 1318, 1365, 1413, 1462, 1514, 1567, 1598, 1622, 1655, 1679, 1713, 1738, 1773, 1799,
    1835, 1862, 1899, 1928, 1966, 1995, 2035, 2065, 2107, 2181, 2257, 2291, 2336, 2418, 2503, 2541,
];

/// `DCS_Options`: each code's nine bits, three per octal digit, ascending.
const DCS_CODES: [u16; 104] = [
    0x013, 0x015, 0x016, 0x019, 0x01A, 0x01E, 0x023, 0x027, 0x029, 0x02B, 0x02C, 0x035, 0x039,
    0x03A, 0x03B, 0x03C, 0x04C, 0x04D, 0x04E, 0x052, 0x055, 0x059, 0x05A, 0x05C, 0x063, 0x065,
    0x06A, 0x06D, 0x06E, 0x072, 0x075, 0x07A, 0x07C, 0x085, 0x08A, 0x093, 0x095, 0x096, 0x0A3,
    0x0A4, 0x0A5, 0x0A6, 0x0A9, 0x0AA, 0x0AD, 0x0B1, 0x0B3, 0x0B5, 0x0B6, 0x0B9, 0x0BC, 0x0C6,
    0x0C9, 0x0CD, 0x0D5, 0x0D9, 0x0DA, 0x0E3, 0x0E6, 0x0E9, 0x0EE, 0x0F4, 0x0F5, 0x0F9, 0x109,
    0x10A, 0x10B, 0x113, 0x119, 0x11A, 0x125, 0x126, 0x12A, 0x12C, 0x12D, 0x132, 0x134, 0x135,
    0x136, 0x143, 0x146, 0x14E, 0x153, 0x156, 0x15A, 0x166, 0x175, 0x186, 0x18A, 0x194, 0x197,
    0x199, 0x19A, 0x1AC, 0x1B2, 0x1B4, 0x1C3, 0x1CA, 0x1D3, 0x1D9, 0x1DA, 0x1DC, 0x1E3, 0x1EC,
];

/// How many standard CTCSS tones there are.
pub const CTCSS_COUNT: u8 = 50;
/// How many standard DCS codes there are, in each polarity.
pub const DCS_COUNT: u8 = 104;
/// Positions [`standard_tone`] numbers in one polarity: none, every CTCSS
/// tone, then every DCS code.
pub const TONE_POSITIONS: u8 = 1 + CTCSS_COUNT + DCS_COUNT;

/// The lowest CTCSS tone and each next tone's rise over the one before; no
/// rise in the table exceeds 8.5 Hz, so each fits a byte.
const FIRST_CTCSS: u16 = CTCSS_TONES[0];
const CTCSS_STEPS: [u8; CTCSS_TONES.len() - 1] = {
    let mut steps = [0; CTCSS_TONES.len() - 1];
    let mut index = 0;
    while index < steps.len() {
        let rise = CTCSS_TONES[index + 1] - CTCSS_TONES[index];
        assert!(rise > 0 && rise <= u8::MAX as u16);
        // Checked just above; `try_from` is not available in a constant.
        #[allow(clippy::cast_possible_truncation)]
        {
            steps[index] = rise as u8;
        }
        index += 1;
    }
    steps
};

/// One bit for each of the 512 nine-bit codes, set for the standard ones.
const DCS_STANDARD: [u8; 64] = {
    let mut bits = [0; 64];
    let mut index = 0;
    while index < DCS_CODES.len() {
        let code = DCS_CODES[index] as usize;
        bits[code / 8] |= 1 << (code % 8);
        index += 1;
    }
    bits
};

/// The standard CTCSS tone at `index` in tenths of a hertz, ascending.
#[must_use]
pub fn ctcss_tone(index: u8) -> Option<u16> {
    let rises = CTCSS_STEPS.get(..usize::from(index))?;
    let mut tone = FIRST_CTCSS;
    for rise in rises {
        tone += u16::from(*rise);
    }
    Some(tone)
}

/// Every standard CTCSS tone in tenths of a hertz, ascending.
pub fn ctcss_tones() -> impl Iterator<Item = u16> {
    (0..CTCSS_COUNT).filter_map(ctcss_tone)
}

/// Whether `bits`, a code's nine bits, is one of the standard DCS codes.
#[must_use]
pub fn is_standard_dcs(bits: u16) -> bool {
    DCS_STANDARD
        .get(usize::from(bits / 8))
        .is_some_and(|byte| byte & (1 << (bits % 8)) != 0)
}

/// The nine bits of the standard DCS code at `index`, ascending.
fn dcs_bits(index: u8) -> Option<u16> {
    let mut left = index;
    for bits in 0..512 {
        if is_standard_dcs(bits) {
            if left == 0 {
                return Some(bits);
            }
            left -= 1;
        }
    }
    None
}

/// A DCS code's octal digits held as a decimal integer, from its nine bits.
const fn dcs_code(bits: u16) -> u16 {
    (bits >> 6) * 100 + ((bits >> 3) & 7) * 10 + (bits & 7)
}

/// The standard tone at `position` in `inverted` DCS polarity: 0 is none,
/// then each CTCSS tone, then each DCS code. `None` past the last.
#[must_use]
pub fn standard_tone(position: u8, inverted: bool) -> Option<Tone> {
    let Some(index) = position.checked_sub(1) else {
        return Some(Tone::None);
    };
    match index.checked_sub(CTCSS_COUNT) {
        None => ctcss_tone(index).map(Tone::Ctcss),
        Some(code) => dcs_bits(code).map(|bits| Tone::Dcs {
            code: dcs_code(bits),
            inverted,
        }),
    }
}

/// The position [`standard_tone`] gives `tone`, and its DCS polarity;
/// `None` for a tone the reference does not list. Quick enough to run on
/// every pass of the run loop: at most 50 tones or 512 codes are looked at.
#[must_use]
pub fn standard_position(tone: Tone) -> Option<(u8, bool)> {
    let (index, inverted) = match tone {
        Tone::None => return Some((0, false)),
        Tone::Ctcss(tenths) => (
            (0..CTCSS_COUNT).find(|index| ctcss_tone(*index) == Some(tenths))?,
            false,
        ),
        Tone::Dcs { code, inverted } => {
            let bits = (code / 100) << 6 | (code / 10 % 10) << 3 | (code % 10);
            if code > 777 || dcs_code(bits) != code || !is_standard_dcs(bits) {
                return None;
            }
            let below = (0..bits).filter(|each| is_standard_dcs(*each)).count();
            (CTCSS_COUNT + u8::try_from(below).ok()?, inverted)
        }
    };
    Some((index + 1, inverted))
}

#[cfg(test)]
mod tests {
    use super::{
        ctcss_tone, ctcss_tones, dcs_bits, is_standard_dcs, standard_position, standard_tone,
        CTCSS_TONES, DCS_CODES, TONE_POSITIONS,
    };
    use crate::Tone;

    #[test]
    fn the_packed_lists_hold_exactly_the_reference_lists() {
        assert!(ctcss_tones().eq(CTCSS_TONES));
        assert!((0..104).map(|index| dcs_bits(index).unwrap()).eq(DCS_CODES));
        assert_eq!(dcs_bits(104), None);
        assert_eq!(ctcss_tone(50), None);
        assert!(!is_standard_dcs(512));
    }

    #[test]
    fn every_position_names_one_tone_and_back() {
        assert_eq!(standard_tone(0, false), Some(Tone::None));
        assert_eq!(standard_tone(1, false), Some(Tone::Ctcss(670)));
        assert_eq!(standard_tone(50, false), Some(Tone::Ctcss(2541)));
        assert_eq!(
            standard_tone(51, true),
            Some(Tone::Dcs {
                code: 23,
                inverted: true
            })
        );
        assert_eq!(
            standard_tone(TONE_POSITIONS - 1, false),
            Some(Tone::Dcs {
                code: 754,
                inverted: false
            })
        );
        assert_eq!(standard_tone(TONE_POSITIONS, false), None);
        for inverted in [false, true] {
            for position in 0..TONE_POSITIONS {
                let tone = standard_tone(position, inverted).unwrap();
                let polarity = matches!(tone, Tone::Dcs { .. }) && inverted;
                assert_eq!(standard_position(tone), Some((position, polarity)));
            }
        }
        assert_eq!(standard_position(Tone::Ctcss(1000)), Some((13, false)));
        assert_eq!(standard_position(Tone::Ctcss(1001)), None);
        assert_eq!(
            standard_position(Tone::Dcs {
                code: 24,
                inverted: false
            }),
            None
        );
    }
}
