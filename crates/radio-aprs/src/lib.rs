//! Bounded receive-only AX.25 APRS parsing and repeater discovery.

#![no_std]
#![forbid(unsafe_code)]

mod ax25;

pub use ax25::{
    parse_ui_frame, Ax25Address, Ax25Callsign, Ax25Error, Ax25UiFrame, MAX_DIGIPEATER_ADDRESSES,
    MAX_FRAME_LEN, MAX_INFORMATION_LEN,
};
