//! Bounded receive-only AX.25 APRS parsing and repeater discovery.

#![no_std]
#![forbid(unsafe_code)]

mod ax25;
mod report;

pub use ax25::{
    parse_ui_frame, Ax25Address, Ax25Callsign, Ax25Error, Ax25UiFrame, MAX_DIGIPEATER_ADDRESSES,
    MAX_FRAME_LEN, MAX_INFORMATION_LEN,
};
pub use report::{
    parse_repeater_event, parse_report, AdvertisedOffset, AdvertisedRange, AdvertisedTone,
    AprsError, AprsReport, CtcssPrefix, ObjectTimestamp, RangeUnit, RawPosition,
    RepeaterAdvertisement, RepeaterEvent, ReportKind, ReportName,
};
