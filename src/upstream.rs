//! Various "upstream" providers

mod tap;
mod udp;
mod wireguard;

use oathgate_net::Ipv4Header;

use crate::error::Error;

pub use self::{udp::UdpDevice, tap::Tun};

pub trait UpstreamHandle: Send + Sync {
    /// Writes a packet to the upstream device
    fn write(&self, hdr: Ipv4Header, buf: Vec<u8>) -> Result<(), Error>;
}

pub type BoxUpstreamHandle = Box<dyn UpstreamHandle>;
