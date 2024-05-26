//! IPv4 related structures

use std::net::Ipv4Addr;

use rand::Rng;

use crate::{
    cast, ph_checksum,
    protocols::{NET_PROTOCOL_TCP, NET_PROTOCOL_UDP},
    ProtocolError,
};

/// Represents the Ipv4 header
#[derive(Debug)]
pub struct Ipv4Header {
    pub version: u8,
    pub ihl: u8,
    pub length: u16,
    pub id: u16,
    pub flags: u8,
    pub frag_offset: u16,
    pub ttl: u8,
    pub protocol: u8,
    pub checksum: u16,
    pub src: Ipv4Addr,
    pub dst: Ipv4Addr,
}

#[derive(Debug)]
pub struct Ipv4Packet {
    header: Ipv4Header,
    data: Vec<u8>,
}

impl Ipv4Header {
    /// Creates a new IPv4 header from the supplied values
    ///
    /// ### Arguments
    /// * `src` - Source address
    /// * `dst` - Destination address
    /// * `protocol` - Next header protocol (e.g., TCP, UDP, etc)
    /// * `length` - Length of the expected payload data
    pub fn new(src: Ipv4Addr, dst: Ipv4Addr, protocol: u8, length: u16) -> Self {
        let mut rng = rand::thread_rng();

        Self {
            version: 4,
            ihl: 5,
            length: length + 20,
            id: rng.gen(),
            flags: 2, // don't fragment
            frag_offset: 0,
            ttl: 64,
            protocol,
            checksum: 0,
            src,
            dst,
        }
    }

    /// Extracts the IPv4 header from a vector of bytes, or returns an error
    /// if the supplied buffer is too small
    ///
    /// The first 20 bytes will be drained from the vector
    ///
    /// ### Arguments
    /// * `pkt` - Vector containing ipv4 header
    pub fn extract(pkt: &mut Vec<u8>) -> Result<Self, ProtocolError> {
        if pkt.len() < 20 {
            return Err(ProtocolError::NotEnoughData(pkt.len(), 20));
        }

        let hdr = pkt.drain(0..20).collect::<Vec<_>>();
        Self::extract_from_slice(&hdr)
    }

    /// Extracts the IPv4 header from a vector of bytes, or returns an error
    /// if the supplied buffer is too small
    ///
    /// ### Arguments
    /// * `hdr` - Buffer containing ipv4 header
    pub fn extract_from_slice(hdr: &[u8]) -> Result<Self, ProtocolError> {
        if hdr.is_empty() {
            return Err(ProtocolError::NotEnoughData(0, 20));
        }

        let ihl = hdr[0] & 0x0F;
        let header_sz = usize::from(ihl) * 4;

        if hdr.len() < header_sz {
            return Err(ProtocolError::NotEnoughData(hdr.len(), header_sz));
        }

        let version = hdr[0] >> 4;
        let length = cast!(be16, hdr[2..4]);
        let id = cast!(be16, hdr[4..6]);
        let flags = hdr[6] >> 5;
        let frag_offset = cast!(be16, hdr[6..8]) & 0x1FFF;
        let ttl = hdr[8];
        let protocol = hdr[9];
        let checksum = cast!(be16, hdr[10..12]);
        let src = Ipv4Addr::from(cast!(be32, hdr[12..16]));
        let dst = Ipv4Addr::from(cast!(be32, hdr[16..20]));

        Ok(Self {
            version,
            ihl,
            length,
            id,
            flags,
            frag_offset,
            ttl,
            protocol,
            checksum,
            src,
            dst,
        })
    }

    /// Returns the length, in bytes, of the header
    pub fn header_length(&self) -> usize {
        usize::from(self.ihl) * 4
    }

    /// Reverses the IPv4 source and destination addresses, generates a new id, and computes
    /// the internet checksum over the provided payload length
    ///
    /// ### Arguments
    /// * `payload` - Payload used to fill length field and generate checksum
    pub fn gen_reply(&self, payload: &[u8]) -> Self {
        Ipv4Header::new(self.dst, self.src, self.protocol, payload.len() as u16)
    }

    /// Replaces the source address with the supplied value and returns
    /// the original ipv4 address
    ///
    /// ### Arguments
    /// * `src` - New IPv4 src address
    pub fn masquerade(&mut self, src: Ipv4Addr) -> Ipv4Addr {
        let old = self.src;
        self.src = src;
        old
    }

    /// Replaces the destinaton address with the supplied value and returns
    /// the original ipv4 address
    ///
    /// ### Arguments
    /// * `src` - New IPv4 destination address
    pub fn unmasquerade(&mut self, dst: Ipv4Addr) -> Ipv4Addr {
        let old = self.dst;
        self.dst = dst;
        old
    }

    /// Returns this header as a byte slice / array.
    ///
    /// This does not append the payload but the length field and checksum
    /// are calcuated from the payload length
    pub fn as_bytes(&self, rpkt: &mut [u8]) {
        let flags_frag = ((self.flags as u16) << 13) | self.frag_offset;

        rpkt[0] = (self.version << 4) | 5; // Generally 0x45
        rpkt[2..4].copy_from_slice(&self.length.to_be_bytes());
        rpkt[4..6].copy_from_slice(&self.id.to_be_bytes());
        rpkt[6..8].copy_from_slice(&flags_frag.to_be_bytes());
        rpkt[8] = self.ttl;
        rpkt[9] = self.protocol;
        rpkt[10..12].copy_from_slice(&[0x00, 0x00]); // clear checksum
        rpkt[12..16].copy_from_slice(&self.src.octets());
        rpkt[16..20].copy_from_slice(&self.dst.octets());

        let csum = crate::checksum(&rpkt[0..20]);
        rpkt[10..12].copy_from_slice(&csum.to_be_bytes());
    }

    /// Returns this header an array of bytes
    pub fn into_bytes(self) -> [u8; 20] {
        let mut buf = [0u8; 20];
        self.as_bytes(&mut buf);
        buf
    }
}

impl Ipv4Packet {
    /// Parses an IPv4 packet, extracting the header from the start of the data vector
    ///
    /// Note: This does not drain the header from the vector. Use the `payload` function
    /// to access the transport layer header
    ///
    /// ### Arguments
    /// * `data` - An Ipv4 packet, including the header
    pub fn parse(data: Vec<u8>) -> Result<Self, ProtocolError> {
        let header = Ipv4Header::extract_from_slice(&data)?;
        Ok(Self { header, data })
    }

    /// Returns the next layer (i.e., transport) layer protocol
    pub fn protocol(&self) -> u8 {
        self.header.protocol
    }

    /// Returns the source ip address
    pub fn src(&self) -> Ipv4Addr {
        self.header.src
    }

    /// Returns the destination ip address
    pub fn dest(&self) -> Ipv4Addr {
        self.header.dst
    }

    /// Returns the slice of data containing the Ipv4 packet's payload (aka the transport layer
    /// data)
    pub fn payload(&self) -> &[u8] {
        let offset = self.header.header_length();
        &self.data[offset..]
    }

    /// Returns the slice of data containing the Ipv4 packet's payload (aka the transport layer
    /// data)
    pub fn payload_mut(&mut self) -> &mut [u8] {
        let offset = self.header.header_length();
        &mut self.data[offset..]
    }

    /// Returns this packet as a slice of bytes, including the header
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Returns this packet as a vector of bytes
    pub fn into_bytes(self) -> Vec<u8> {
        self.data
    }

    /// Sets the source ip address to the provided value and recomputes the header checksum
    ///
    /// ### Arguments
    /// * `ip` - New src ip address
    pub fn masquerade(&mut self, ip: Ipv4Addr) {
        self.header.masquerade(ip);
        self.header.as_bytes(&mut self.data);
        self.fix_transport_checksum();
    }

    /// Sets the destination ip address to the provided value and recomputes the header checksum
    ///
    /// ### Arguments
    /// * `ip` - New destinaton ip address
    pub fn unmasquerade(&mut self, ip: Ipv4Addr) {
        self.header.unmasquerade(ip);
        self.header.as_bytes(&mut self.data);
        self.fix_transport_checksum();
    }

    /// TCP and UDP both use a pseudo-ip header in their checksum fields
    /// so we'll need to update the TCP/UDP checksum (if necessary)
    fn fix_transport_checksum(&mut self) {
        let src = self.src();
        let dst = self.dest();
        let proto = self.protocol();
        let payload = self.payload_mut();

        match proto {
            NET_PROTOCOL_TCP => {
                payload[16..18].copy_from_slice(&[0, 0]);
                let sum = ph_checksum(src, dst, proto, payload);
                payload[16..18].copy_from_slice(&sum.to_be_bytes());
            }
            NET_PROTOCOL_UDP => {
                payload[6..8].copy_from_slice(&[0, 0]);
                let sum = ph_checksum(src, dst, proto, payload);
                payload[6..8].copy_from_slice(&sum.to_be_bytes());
            }
            _ => (),
        }
    }

    /// Generates a new Ipv4 header to use as a reply message
    ///
    /// ### Arguments
    /// * `payload` - Payload that will be set in the reply
    pub fn reply(&self, payload: &[u8]) -> Ipv4Header {
        self.header.gen_reply(payload)
    }
}