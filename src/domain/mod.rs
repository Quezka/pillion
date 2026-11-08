//! The rules: what the moped says and means, geometry on the map, speed limits, places, range. Pure Rust, no I/O,
//! no screen, no network; nothing here knows about the layers around it.

pub mod geo;
pub mod places;
pub mod protocol;
pub mod speed_limit;
