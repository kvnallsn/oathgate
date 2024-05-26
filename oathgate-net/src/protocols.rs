//! Collection of higher-level protocols

mod arp;
pub mod icmp;

pub const NET_PROTOCOL_ICMP: u8 = 1;
pub const NET_PROTOCOL_TCP: u8 = 6;
pub const NET_PROTOCOL_UDP: u8 = 17;

pub use self::{arp::ArpPacket, icmp::IcmpPacket};
