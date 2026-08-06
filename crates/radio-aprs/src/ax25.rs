use core::fmt;

/// Maximum APRS digipeater addresses accepted after destination and source.
pub const MAX_DIGIPEATER_ADDRESSES: usize = 8;
/// Maximum AX.25 UI information length accepted by APRS.
pub const MAX_INFORMATION_LEN: usize = 256;
/// Maximum complete de-stuffed frame length, including its two FCS octets.
pub const MAX_FRAME_LEN: usize =
    (2 + MAX_DIGIPEATER_ADDRESSES) * ADDRESS_LEN + 2 + MAX_INFORMATION_LEN + FCS_LEN;

const ADDRESS_LEN: usize = 7;
const MIN_FRAME_LEN: usize = 2 * ADDRESS_LEN + 2 + FCS_LEN;
const FCS_LEN: usize = 2;
const UI_CONTROL: u8 = 0x03;
const NO_LAYER_THREE_PID: u8 = 0xf0;
const GOOD_FCS_RESIDUE: u16 = 0xf0b8;
const REFLECTED_CCITT_POLYNOMIAL: u16 = 0x8408;

/// One decoded AX.25 callsign and SSID.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ax25Callsign {
    characters: [u8; 6],
    length: u8,
    ssid: u8,
}

impl Ax25Callsign {
    /// Returns the unpadded upper-case alphanumeric callsign bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.characters[..usize::from(self.length)]
    }

    /// Returns the four-bit AX.25 secondary station identifier.
    pub const fn ssid(self) -> u8 {
        self.ssid
    }
}

/// One decoded AX.25 address subfield.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ax25Address {
    /// Callsign and SSID carried by the subfield.
    pub callsign: Ax25Callsign,
    /// Role-dependent command/response bit, or repeated bit for a digipeater.
    pub flag: bool,
}

const EMPTY_CALLSIGN: Ax25Callsign = Ax25Callsign {
    characters: [0; 6],
    length: 0,
    ssid: 0,
};
const EMPTY_ADDRESS: Ax25Address = Ax25Address {
    callsign: EMPTY_CALLSIGN,
    flag: false,
};

/// A validated receive-only AX.25 UI frame borrowed from the supplied bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ax25UiFrame<'a> {
    destination: Ax25Address,
    source: Ax25Address,
    digipeaters: [Ax25Address; MAX_DIGIPEATER_ADDRESSES],
    digipeater_count: u8,
    information: &'a [u8],
}

impl<'a> Ax25UiFrame<'a> {
    /// Returns the destination address.
    pub const fn destination(&self) -> Ax25Address {
        self.destination
    }

    /// Returns the source address retained for report attribution.
    pub const fn source(&self) -> Ax25Address {
        self.source
    }

    /// Returns the validated path in on-frame order.
    pub fn digipeaters(&self) -> &[Ax25Address] {
        &self.digipeaters[..usize::from(self.digipeater_count)]
    }

    /// Returns the APRS information field, including its first data-type octet.
    pub const fn information(&self) -> &'a [u8] {
        self.information
    }
}

/// A complete AX.25 UI frame failed bounded validation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ax25Error {
    /// The complete frame cannot contain the mandatory fields.
    FrameTooShort,
    /// The complete frame exceeds the APRS address and information bounds.
    FrameTooLong,
    /// The received FCS did not have the required AX.25 residue.
    InvalidFcs,
    /// A shifted callsign, padding, SSID, or reserved bit was invalid.
    InvalidAddress,
    /// The destination incorrectly ended the address list.
    MissingSource,
    /// More than eight APRS digipeaters preceded the final extension bit.
    TooManyDigipeaters,
    /// The frame did not use the unnumbered-information control value.
    UnsupportedControl,
    /// The UI frame did not use the no-Layer-3 protocol identifier.
    UnsupportedPid,
    /// The information field was empty or exceeded 256 octets.
    InvalidInformationLength,
}

impl fmt::Display for Ax25Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FrameTooShort => formatter.write_str("AX.25 frame is too short"),
            Self::FrameTooLong => formatter.write_str("AX.25 frame exceeds APRS bounds"),
            Self::InvalidFcs => formatter.write_str("AX.25 frame FCS is invalid"),
            Self::InvalidAddress => formatter.write_str("AX.25 address is malformed"),
            Self::MissingSource => formatter.write_str("AX.25 destination ended before source"),
            Self::TooManyDigipeaters => {
                formatter.write_str("AX.25 frame has more than eight APRS digipeaters")
            }
            Self::UnsupportedControl => formatter.write_str("AX.25 frame is not UI control"),
            Self::UnsupportedPid => formatter.write_str("AX.25 UI frame PID is not 0xF0"),
            Self::InvalidInformationLength => {
                formatter.write_str("AX.25 information length is outside APRS bounds")
            }
        }
    }
}

/// Validates one complete de-stuffed AX.25 UI frame including its FCS.
pub fn parse_ui_frame(frame: &[u8]) -> Result<Ax25UiFrame<'_>, Ax25Error> {
    if frame.len() < MIN_FRAME_LEN {
        return Err(Ax25Error::FrameTooShort);
    }
    if frame.len() > MAX_FRAME_LEN {
        return Err(Ax25Error::FrameTooLong);
    }
    if fcs_accumulator(frame) != GOOD_FCS_RESIDUE {
        return Err(Ax25Error::InvalidFcs);
    }

    let (destination, destination_final) = parse_address(&frame[..ADDRESS_LEN])?;
    if destination_final {
        return Err(Ax25Error::MissingSource);
    }
    let (source, mut final_address) = parse_address(&frame[ADDRESS_LEN..2 * ADDRESS_LEN])?;
    let mut index = 2 * ADDRESS_LEN;
    let mut digipeaters = [EMPTY_ADDRESS; MAX_DIGIPEATER_ADDRESSES];
    let mut digipeater_count = 0_usize;

    while !final_address {
        if digipeater_count == MAX_DIGIPEATER_ADDRESSES {
            return Err(Ax25Error::TooManyDigipeaters);
        }
        let end = index
            .checked_add(ADDRESS_LEN)
            .ok_or(Ax25Error::FrameTooLong)?;
        if end + 2 + FCS_LEN > frame.len() {
            return Err(Ax25Error::FrameTooShort);
        }
        let (address, is_final) = parse_address(&frame[index..end])?;
        digipeaters[digipeater_count] = address;
        digipeater_count += 1;
        index = end;
        final_address = is_final;
    }

    if frame[index] != UI_CONTROL {
        return Err(Ax25Error::UnsupportedControl);
    }
    if frame[index + 1] != NO_LAYER_THREE_PID {
        return Err(Ax25Error::UnsupportedPid);
    }
    let information_start = index + 2;
    let information_end = frame.len() - FCS_LEN;
    let information_len = information_end.saturating_sub(information_start);
    if !(1..=MAX_INFORMATION_LEN).contains(&information_len) {
        return Err(Ax25Error::InvalidInformationLength);
    }

    Ok(Ax25UiFrame {
        destination,
        source,
        digipeaters,
        digipeater_count: u8::try_from(digipeater_count)
            .map_err(|_| Ax25Error::TooManyDigipeaters)?,
        information: &frame[information_start..information_end],
    })
}

fn parse_address(encoded: &[u8]) -> Result<(Ax25Address, bool), Ax25Error> {
    if encoded.len() != ADDRESS_LEN {
        return Err(Ax25Error::InvalidAddress);
    }
    let mut characters = [0_u8; 6];
    let mut length = 0_usize;
    let mut padding = false;
    for (index, byte) in encoded[..6].iter().copied().enumerate() {
        if byte & 1 != 0 {
            return Err(Ax25Error::InvalidAddress);
        }
        let character = byte >> 1;
        match character {
            b' ' => padding = true,
            b'A'..=b'Z' | b'0'..=b'9' if !padding => {
                characters[index] = character;
                length += 1;
            }
            _ => return Err(Ax25Error::InvalidAddress),
        }
    }
    if length == 0 {
        return Err(Ax25Error::InvalidAddress);
    }

    let ssid_byte = encoded[6];
    if ssid_byte & 0x60 != 0x60 {
        return Err(Ax25Error::InvalidAddress);
    }
    Ok((
        Ax25Address {
            callsign: Ax25Callsign {
                characters,
                length: u8::try_from(length).map_err(|_| Ax25Error::InvalidAddress)?,
                ssid: (ssid_byte >> 1) & 0x0f,
            },
            flag: ssid_byte & 0x80 != 0,
        },
        ssid_byte & 1 != 0,
    ))
}

fn fcs_accumulator(bytes: &[u8]) -> u16 {
    let mut fcs = 0xffff_u16;
    for byte in bytes {
        fcs ^= u16::from(*byte);
        for _ in 0..8 {
            fcs = if fcs & 1 == 0 {
                fcs >> 1
            } else {
                (fcs >> 1) ^ REFLECTED_CCITT_POLYNOMIAL
            };
        }
    }
    fcs
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::{
        fcs_accumulator, parse_ui_frame, Ax25Error, GOOD_FCS_RESIDUE, MAX_FRAME_LEN,
        MAX_INFORMATION_LEN,
    };

    fn encoded_address(callsign: &[u8], ssid: u8, flag: bool, final_address: bool) -> [u8; 7] {
        let mut encoded = [b' ' << 1; 7];
        for (destination, source) in encoded[..6].iter_mut().zip(callsign.iter().copied()) {
            *destination = source << 1;
        }
        encoded[6] = 0x60 | (ssid << 1) | u8::from(flag) << 7 | u8::from(final_address);
        encoded
    }

    fn append_fcs(bytes: &mut std::vec::Vec<u8>) {
        let fcs = !fcs_accumulator(bytes);
        bytes.extend_from_slice(&fcs.to_le_bytes());
    }

    fn frame(path: &[(&[u8], u8, bool)], information: &[u8]) -> std::vec::Vec<u8> {
        let mut bytes = std::vec::Vec::new();
        bytes.extend_from_slice(&encoded_address(b"APRS", 0, false, false));
        bytes.extend_from_slice(&encoded_address(b"N0CALL", 1, true, path.is_empty()));
        for (index, (callsign, ssid, repeated)) in path.iter().copied().enumerate() {
            bytes.extend_from_slice(&encoded_address(
                callsign,
                ssid,
                repeated,
                index + 1 == path.len(),
            ));
        }
        bytes.extend_from_slice(&[0x03, 0xf0]);
        bytes.extend_from_slice(information);
        append_fcs(&mut bytes);
        bytes
    }

    #[test]
    fn crc_x25_check_value_and_ax25_residue_match_sourced_rule() {
        let check = fcs_accumulator(b"123456789");
        assert_eq!(!check, 0x906e);

        let mut complete = b"123456789".to_vec();
        complete.extend_from_slice(&0x906e_u16.to_le_bytes());
        assert_eq!(fcs_accumulator(&complete), GOOD_FCS_RESIDUE);
    }

    #[test]
    fn complete_ui_frame_preserves_addresses_path_and_information() {
        let bytes = frame(&[(b"WIDE1", 1, true), (b"WIDE2", 2, false)], b">AFIK");
        let parsed = parse_ui_frame(&bytes).unwrap();

        assert_eq!(parsed.destination().callsign.as_bytes(), b"APRS");
        assert_eq!(parsed.source().callsign.as_bytes(), b"N0CALL");
        assert_eq!(parsed.source().callsign.ssid(), 1);
        assert!(parsed.source().flag);
        assert_eq!(parsed.digipeaters().len(), 2);
        assert_eq!(parsed.digipeaters()[0].callsign.as_bytes(), b"WIDE1");
        assert!(parsed.digipeaters()[0].flag);
        assert_eq!(parsed.digipeaters()[1].callsign.ssid(), 2);
        assert_eq!(parsed.information(), b">AFIK");
    }

    #[test]
    fn frame_bounds_and_eight_digipeater_limit_are_exact() {
        let path = [(b"WIDE1".as_slice(), 1, false); 8];
        let bytes = frame(&path, &[b'A'; MAX_INFORMATION_LEN]);
        assert_eq!(bytes.len(), MAX_FRAME_LEN);
        assert_eq!(parse_ui_frame(&bytes).unwrap().digipeaters().len(), 8);

        let overlong = frame(&path, &[b'A'; MAX_INFORMATION_LEN + 1]);
        assert_eq!(parse_ui_frame(&overlong), Err(Ax25Error::FrameTooLong));

        let too_many = frame(&[(b"WIDE1".as_slice(), 1, false); 9], b"A");
        assert_eq!(
            parse_ui_frame(&too_many),
            Err(Ax25Error::TooManyDigipeaters)
        );
    }

    #[test]
    fn invalid_fcs_control_pid_and_information_are_rejected() {
        let mut bad_fcs = frame(&[], b"A");
        bad_fcs[14] ^= 1;
        assert_eq!(parse_ui_frame(&bad_fcs), Err(Ax25Error::InvalidFcs));

        let mut bad_control = frame(&[], b"A");
        bad_control[14] = 0x13;
        bad_control.truncate(bad_control.len() - 2);
        append_fcs(&mut bad_control);
        assert_eq!(
            parse_ui_frame(&bad_control),
            Err(Ax25Error::UnsupportedControl)
        );

        let mut bad_pid = frame(&[], b"A");
        bad_pid[15] = 0xcf;
        bad_pid.truncate(bad_pid.len() - 2);
        append_fcs(&mut bad_pid);
        assert_eq!(parse_ui_frame(&bad_pid), Err(Ax25Error::UnsupportedPid));

        let empty = frame(&[], b"");
        assert_eq!(
            parse_ui_frame(&empty),
            Err(Ax25Error::InvalidInformationLength)
        );
    }

    #[test]
    fn address_encoding_and_termination_are_strict() {
        let mut odd_shift = frame(&[], b"A");
        odd_shift[0] |= 1;
        odd_shift.truncate(odd_shift.len() - 2);
        append_fcs(&mut odd_shift);
        assert_eq!(parse_ui_frame(&odd_shift), Err(Ax25Error::InvalidAddress));

        let mut embedded_space = frame(&[], b"A");
        embedded_space[2] = b' ' << 1;
        embedded_space.truncate(embedded_space.len() - 2);
        append_fcs(&mut embedded_space);
        assert_eq!(
            parse_ui_frame(&embedded_space),
            Err(Ax25Error::InvalidAddress)
        );

        let mut missing_source = frame(&[], b"A");
        missing_source[6] |= 1;
        missing_source.truncate(missing_source.len() - 2);
        append_fcs(&mut missing_source);
        assert_eq!(
            parse_ui_frame(&missing_source),
            Err(Ax25Error::MissingSource)
        );

        let mut reserved = frame(&[], b"A");
        reserved[13] &= !0x20;
        reserved.truncate(reserved.len() - 2);
        append_fcs(&mut reserved);
        assert_eq!(parse_ui_frame(&reserved), Err(Ax25Error::InvalidAddress));
    }
}
