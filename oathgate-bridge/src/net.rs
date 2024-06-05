//! Networking-related code used by the bridge

pub(crate) const ETHERNET_HDR_SZ: usize = 14;

mod error;
pub mod router;
pub mod switch;
pub mod wan;

pub use self::error::NetworkError;
