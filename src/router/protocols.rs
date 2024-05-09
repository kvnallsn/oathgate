//! Protocols

use std::{
    fmt::Debug,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
};

use rand::{Rng, RngCore};

use crate::{cast, error::ProtocolError};

use super::checksum;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EtherType {
    IPv4 = 0x0800,
    IPv6 = 0x86DD,
    ARP = 0x0806,
}

impl TryFrom<u16> for EtherType {
    type Error = ProtocolError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            x if x == EtherType::IPv4 as u16 => Ok(EtherType::IPv4),
            x if x == EtherType::IPv6 as u16 => Ok(EtherType::IPv6),
            x if x == EtherType::ARP as u16 => Ok(EtherType::ARP),
            _ => Err(ProtocolError::MalformedPacket(format!(
                "unknown ethertype: 0x{value:04x}"
            ))),
        }
    }
}

impl TryFrom<&[u8]> for EtherType {
    type Error = ProtocolError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        match value.len() {
            0 | 1 => Err(ProtocolError::NotEnoughData(value.len(), 2)),
            _ => EtherType::try_from(cast!(be16, value[0..2])),
        }
    }
}

impl Debug for EtherType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "EtherType(0x{:04x})", self.as_u16())
    }
}

impl EtherType {
    pub fn as_u16(self) -> u16 {
        self as u16
    }
}

/// Representation of  MAC address
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct MacAddress([u8; 6]);

impl Debug for MacAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MacAddress({:02x?})", self.0)
    }
}

impl MacAddress {
    /// Parses a MAC address from a byte buffer
    ///
    /// ### Arguments
    /// * `bytes` - Bytes to extract MAC address from
    pub fn parse(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let mut mac = [0u8; 6];
        if bytes.len() < mac.len() {
            return Err(ProtocolError::NotEnoughData(bytes.len(), mac.len()));
        }

        mac.copy_from_slice(&bytes[0..6]);
        Ok(Self(mac))
    }

    /// Generates a new MAC address with the prefix 52:54:00
    pub fn generate() -> Self {
        let mut rng = rand::thread_rng();
        let mut mac = [0x52, 0x54, 0x00, 0x00, 0x00, 0x00];
        rng.fill_bytes(&mut mac[3..6]);
        Self(mac)
    }

    /// Returns a reference to the underlying bytes
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Debug)]
pub struct EthernetFrame {
    pub dst: MacAddress,
    pub src: MacAddress,
    pub ethertype: EtherType
}

impl EthernetFrame {
    pub fn extract(pkt: &mut Vec<u8>) -> Result<Self, ProtocolError> {
        if pkt.len() < 14 {
            return Err(ProtocolError::NotEnoughData(pkt.len(), 14));
        }

        let hdr = pkt.drain(0..14).collect::<Vec<_>>();
        let dst = MacAddress::parse(&hdr[0..6])?;
        let src = MacAddress::parse(&hdr[6..12])?;
        let ethertype = EtherType::try_from(&hdr[12..14])?;

        Ok(Self { dst, src, ethertype })
    }

    pub fn as_reply(&self) -> Self {
        Self { dst: self.src, src: self.dst, ethertype: self.ethertype }
    }

    pub fn to_vec(self, data: &[u8]) -> Vec<u8> {
        let mut pkt = Vec::with_capacity(data.len() + 12);
        pkt.extend_from_slice(&self.dst.as_bytes());
        pkt.extend_from_slice(&self.src.as_bytes());
        pkt.extend_from_slice(&self.ethertype.as_u16().to_be_bytes());
        pkt.extend_from_slice(data);
        pkt
    }
}

pub struct Ipv4Header {
    pub version: u8,
    pub id: u16,
    pub flags: u8,
    pub frag_offset: u16,
    pub ttl: u8,
    pub protocol: u8,
    pub checksum: u16,
    pub src: Ipv4Addr,
    pub dst: Ipv4Addr
}

impl Ipv4Header {
    pub fn new(src: Ipv4Addr, dst: Ipv4Addr, protocol: u8) -> Self {
        let mut rng = rand::thread_rng();

        Self {
            version: 4,
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

    pub fn extract(pkt: &mut Vec<u8>) -> Result<Self, ProtocolError> {
        if pkt.len() < 20 {
            return Err(ProtocolError::NotEnoughData(pkt.len(), 20));
        }

        let hdr = pkt.drain(0..20).collect::<Vec<_>>();
        let version = hdr[0] >> 4;
        let id = cast!(be16, hdr[4..6]);
        let flags = hdr[6] >> 5;
        let frag_offset = cast!(be16, hdr[6..8]) & 0x1FFF;
        let ttl = hdr[8];
        let protocol = hdr[9];
        let checksum = cast!(be16, hdr[10..12]);
        let src = Ipv4Addr::from(cast!(be32, hdr[12..16]));
        let dst = Ipv4Addr::from(cast!(be32, hdr[16..20]));

        Ok(Self { version, id, flags, frag_offset, ttl, protocol, checksum, src, dst })
    }

    pub fn as_reply(&self) -> Self {
        Ipv4Header::new(self.dst, self.src, self.protocol)
    }

    pub fn to_vec(self, data: &[u8]) -> Vec<u8> {
        let length = (20 + data.len()) as u16;
        let flags_frag = ((self.flags as u16) << 13) | self.frag_offset;

        let mut pkt = Vec::new();
        pkt.push((self.version << 4) | 5);
        pkt.push(0x00 /* DSCP * ECN */);
        pkt.extend_from_slice(&length.to_be_bytes());
        pkt.extend_from_slice(&self.id.to_be_bytes());
        pkt.extend_from_slice(&flags_frag.to_be_bytes());
        pkt.push(self.ttl);
        pkt.push(self.protocol);
        pkt.extend_from_slice(&[0x00, 0x00] /* checksum */);
        pkt.extend_from_slice(&self.src.octets());
        pkt.extend_from_slice(&self.dst.octets());
        pkt.extend_from_slice(&data);

        let csum = checksum(&pkt[0..20]);
        pkt[10..12].copy_from_slice(&csum.to_be_bytes());
        pkt
    }
}

#[derive(Debug)]
pub struct ArpPacket {
    /// Netlink link protocol type (e.g. Ethernet => 1)
    pub hardware_type: u16,

    /// Internetwork protocol for which the ARP packet is intended
    pub protocol_type: EtherType,

    /// Length (in octects) of a hardware address
    pub hardware_len: u8,

    /// Length (in octets) of internetwork address (e.g. ipv4 => 4)
    pub protocol_len: u8,

    /// Specifices the operation the sender is performing
    /// 1: Request
    /// 2: Reply
    pub operation: u16,

    /// MAC address of the sender
    /// - Request => address of host sending request
    /// - Reply => address of host the request was looking for
    pub sha: MacAddress,

    /// Internetwork address of the sender
    pub spa: IpAddr,

    /// MAC address of the intended receiver
    /// - Request => ignored / zeros
    /// - Reply => address of host that sent the request
    pub tha: MacAddress,

    /// Internetwork address of the intended receiver
    pub tpa: IpAddr,
}

impl ArpPacket {
    /// Parses an ARP packet from a byte buffer
    ///
    /// The byte buffer is expected to be in network (big) endian format
    ///
    /// ### Arguments
    /// * `bytes` - Series of bytes to parse ARP packet from
    pub fn parse(bytes: &[u8]) -> Result<Self, ProtocolError> {
        /// A 28-byte packet is an ARP IPv4 packet
        const MIN_SZ: usize = 28;
        if bytes.len() < MIN_SZ {
            return Err(ProtocolError::NotEnoughData(bytes.len(), MIN_SZ));
        }

        let hardware_type = cast!(be16, bytes[0..2]);
        let protocol_type = EtherType::try_from(&bytes[2..4])?;
        let hardware_len = bytes[4];
        let protocol_len = bytes[5];
        let operation = cast!(be16, bytes[6..8]);

        match (hardware_type, hardware_len) {
            (1, 6) => { /* do nothing, good match */ }
            (1, _) => {
                return Err(ProtocolError::MalformedPacket(format!(
                    "hardware type (ethernet) does have expected length (6), has length {hardware_len}"
                )));
            }
            _ => {
                return Err(ProtocolError::MalformedPacket(format!(
                    "unknown hardware type: 0x{hardware_type:04x}"
                )))
            }
        }

        // compute dynamic offsets for addresses
        let hlu: usize = hardware_len.into();
        let plu: usize = protocol_len.into();

        let sha_start: usize = 8;
        let sha_end = sha_start + hlu;
        let spa_start = sha_end;
        let spa_end = spa_start + plu;

        let tha_start = spa_end;
        let tha_end = tha_start + hlu;
        let tpa_start = tha_end;
        let tpa_end = tpa_start + plu;

        let sha = MacAddress::parse(&bytes[sha_start..sha_end])?;
        let tha = MacAddress::parse(&bytes[tha_start..tha_end])?;

        let (spa, tpa) = match (protocol_type, protocol_len) {
            (EtherType::IPv4, 4) => {
                let spa = Ipv4Addr::from(cast!(be32, &bytes[spa_start..spa_end]));
                let tpa = Ipv4Addr::from(cast!(be32, &bytes[tpa_start..tpa_end]));
                (IpAddr::V4(spa), IpAddr::V4(tpa))
            }
            (EtherType::IPv6, 16) => {
                let spa = Ipv6Addr::from(cast!(be128, &bytes[spa_start..spa_end]));
                let tpa = Ipv6Addr::from(cast!(be128, &bytes[tpa_start..tpa_end]));
                (IpAddr::V6(spa), IpAddr::V6(tpa))
            }
            (EtherType::IPv4, _) => {
                return Err(ProtocolError::MalformedPacket(format!("protocol type (ipv4) does not have expected length (4), has length {protocol_len}")));
            }
            (EtherType::IPv6, _) => {
                return Err(ProtocolError::MalformedPacket(format!("protocol type (ipv6) does not have expected length (16), has length {protocol_len}")));
            }
            _ => {
                return Err(ProtocolError::MalformedPacket(format!(
                    "invalid ethertype for ARP packet: {protocol_type:?}"
                )));
            }
        };

        Ok(Self {
            hardware_type,
            protocol_type,
            hardware_len,
            protocol_len,
            operation,
            sha,
            spa,
            tha,
            tpa,
        })
    }

    /// Builds an ARP reply packet based on this packet
    pub fn to_reply(&mut self, mac: MacAddress) {
        let tpa = self.tpa;
        self.tpa = self.spa;
        self.tha = self.sha;
        self.spa = tpa;
        self.sha = mac;
        self.operation = 2;
    }

    pub fn len(&self) -> usize {
        std::mem::size_of_val(self)
    }

    pub fn to_bytes(self) -> Vec<u8> {
        let mut data = Vec::with_capacity(std::mem::size_of_val(&self));
        data.extend_from_slice(&self.hardware_type.to_be_bytes());
        data.extend_from_slice(&self.protocol_type.as_u16().to_be_bytes());
        data.push(self.hardware_len);
        data.push(self.protocol_len);
        data.extend_from_slice(&self.operation.to_be_bytes());
        data.extend_from_slice(&self.sha.as_bytes());
        match self.spa {
            IpAddr::V4(ip) => data.extend_from_slice(ip.octets().as_slice()),
            IpAddr::V6(ip) => data.extend_from_slice(ip.octets().as_slice()),
        }
        data.extend_from_slice(&self.tha.as_bytes());
        match self.tpa {
            IpAddr::V4(ip) => data.extend_from_slice(ip.octets().as_slice()),
            IpAddr::V6(ip) => data.extend_from_slice(ip.octets().as_slice()),
        }
        data
    }
}
