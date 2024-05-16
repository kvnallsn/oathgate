//! Various "upstream" providers

mod tap;
mod udp;
mod wireguard;

use crate::{error::Error, router::protocols::Ipv4Header};

pub use self::{udp::UdpDevice, tap::Tun};

pub trait UpstreamHandle: Send + Sync {
    /// Writes a packet to the upstream device
    fn write(&self, hdr: Ipv4Header, buf: Vec<u8>) -> Result<(), Error>;
}

pub type BoxUpstreamHandle = Box<dyn UpstreamHandle>;
