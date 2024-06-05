//! Collection of Protocol Handlers

mod icmp;
mod udp;

use oathgate_net::{Ipv4Packet, ProtocolError};

pub use self::{icmp::IcmpHandler, udp::UdpHandler};

pub trait ProtocolHandler: Send + Sync {
    fn protocol(&self) -> u8;

    fn handle_protocol(&self, pkt: &Ipv4Packet, buf: &mut [u8]) -> Result<usize, ProtocolError>;
}
