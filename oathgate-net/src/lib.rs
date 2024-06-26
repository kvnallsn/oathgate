mod frame;
mod ipv4;
mod macros;
pub mod nat;
pub mod protocols;
pub mod types;

use std::net::Ipv4Addr;

pub use self::{
    frame::{EthernetFrame, EthernetPacket},
    ipv4::{Ipv4Header, Ipv4Packet},
};

#[derive(thiserror::Error, Debug)]
pub enum ProtocolError {
    #[error("not enough data for payload, got = {0}, expected = {1}")]
    NotEnoughData(usize, usize),

    #[error("malformed packet: {0}")]
    MalformedPacket(String),

    #[error("{0}")]
    Other(String),
}

pub trait Switch: Clone + Send + Sync {
    /// Returns the port associated with the new switch device
    fn connect<P: SwitchPort + 'static>(&self, port: P) -> usize;

    /// Process a packet, sending it to the correct device
    fn process(&self, port: usize, pkt: Vec<u8>) -> Result<(), ProtocolError>;
}

/// A `SwitchPort` represents a device that can be connected to a switch
pub trait SwitchPort: Send + Sync {
    /// Places a packet in the device's receive queue
    ///
    /// ### Arguments
    /// * `frame` - Ethernet frame header
    /// * `pkt` - Ethernet frame payload
    fn enqueue(&self, frame: EthernetFrame, pkt: Vec<u8>);
}

/// Computes the checksum used in various networking protocols
///
/// Algorithm is the one's complement of the sum of the data as big-ending u16 values
///
/// ### Arguments
/// * `data` - Data to checksum
pub fn checksum(data: &[u8]) -> u16 {
    let mut sum = 0;
    for b in data.chunks(2) {
        let b0 = b[0];
        let b1 = match b.len() {
            1 => 0x00,
            _ => b[1],
        };

        sum += u32::from_be_bytes([0x00, 0x00, b0, b1]);
    }

    !(((sum & 0xFFFF) + ((sum >> 16) & 0xFFFF)) as u16)
}

/// Computes the pseudo-header checksum as used by TCP and UDP
///
/// ### Arguments
/// * `src` - Source IPv4 Address
/// * `dst` - Destination IPv4 Address
/// * `proto` - Protocol Number (i.e. 6 for TCP)
/// * `data` - TCP/UDP header + payload
pub fn ph_checksum(src: Ipv4Addr, dst: Ipv4Addr, proto: u8, data: &[u8]) -> u16 {
    let mut sum = 0;
    let ip = src.octets();
    sum += u32::from_be_bytes([0x00, 0x00, ip[2], ip[3]]);
    sum += u32::from_be_bytes([0x00, 0x00, ip[0], ip[1]]);
    let ip = dst.octets();
    sum += u32::from_be_bytes([0x00, 0x00, ip[2], ip[3]]);
    sum += u32::from_be_bytes([0x00, 0x00, ip[0], ip[1]]);
    sum += u32::from(proto);

    let len = data.len();
    sum += (len & 0xFFFF) as u32;

    for b in data.chunks(2) {
        let b0 = b[0];
        let b1 = match b.len() {
            1 => 0x00,
            _ => b[1],
        };

        sum += u32::from_be_bytes([0x00, 0x00, b0, b1]);
    }

    !(((sum & 0xFFFF) + ((sum >> 16) & 0xFFFF)) as u16)
}
