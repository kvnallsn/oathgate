//! Various Networking  Types

use std::{
    fmt::{Debug, Display},
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    u128,
};

use rand::RngCore;

use crate::{cast, ProtocolError};

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
        write!(
            f,
            "MacAddress({:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x})",
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[5]
        )
    }
}

impl Display for MacAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[5]
        )
    }
}

impl MacAddress {
    /// Returns the broadcast MacAddress (FF:FF:FF:FF:FF:FF)
    pub const fn broadcast() -> Self {
        Self([0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF])
    }

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

    /// Returns true if this MAC is the broadcast address
    pub fn is_broadcast(&self) -> bool {
        *self == Self::broadcast()
    }
}

impl TryFrom<&[i8]> for MacAddress {
    type Error = ProtocolError;

    fn try_from(bytes: &[i8]) -> Result<Self, Self::Error> {
        let mut mac = [0u8; 6];
        if bytes.len() < mac.len() {
            return Err(ProtocolError::NotEnoughData(bytes.len(), mac.len()));
        }

        for i in 0..6 {
            mac[i] = bytes[i] as u8;
        }
        Ok(Self(mac))
    }
}

#[derive(Debug)]
pub struct NetworkAddress {
    ip: IpAddr,
    mask: IpAddr,
}

impl Display for NetworkAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mask = match self.mask {
            IpAddr::V4(ip) => u32::from(ip).count_ones(),
            IpAddr::V6(ip) => u128::from(ip).count_ones(),
        };
        write!(f, "{}/{}", self.ip, mask)
    }
}

impl NetworkAddress {
    /// Creates a new Classless Inter-Romain Routing (CIDR) address
    ///
    /// ### Arguments
    /// * `ip` - IP address component of the CIDR
    /// * `mask` - Subnet mask of the CIDR
    pub fn new<I: Into<IpAddr>>(ip: I, mask: u8) -> Self {
        match ip.into() {
            IpAddr::V4(ip) => Self::new_v4(ip, mask),
            IpAddr::V6(ip) => Self::new_v6(ip, mask),
        }
    }

    pub fn new_v4<I: Into<Ipv4Addr>>(ip: I, mask: u8) -> Self {
        let mut subnet: u32 = u32::MAX;
        for idx in 0..(32 - mask) {
            subnet = subnet ^ (1 << idx);
        }

        Self {
            ip: IpAddr::V4(ip.into()),
            mask: IpAddr::V4(subnet.into()),
        }
    }

    pub fn new_v6<I: Into<Ipv6Addr>>(ip: I, mask: u8) -> Self {
        let mut subnet: u128 = u128::MAX;
        for idx in 0..(128 - mask) {
            subnet = subnet ^ (1 << idx);
        }

        Self {
            ip: IpAddr::V6(ip.into()),
            mask: IpAddr::V6(subnet.into()),
        }
    }

    /// Returns the IP address used to create this network
    pub fn ip(&self) -> IpAddr {
        self.ip
    }

    /// Returns the subnet mask formatted as an IP address
    /// (i.e., 255.255.255.0)
    pub fn subnet_mask(&self) -> IpAddr {
        self.mask
    }

    /// Returns the network address (aka all zeros in the host component)
    pub fn network(&self) -> IpAddr {
        match (self.ip, self.mask) {
            (IpAddr::V4(ip), IpAddr::V4(mask)) => IpAddr::V4(ip & mask),
            (IpAddr::V6(ip), IpAddr::V6(mask)) => IpAddr::V6(ip & mask),
            (_, _) => unreachable!("mismatch ip and subnet ip versions"),
        }
    }

    /// Returns the broadcast address (aka all ones in the host component)
    pub fn broadcast(&self) -> IpAddr {
        match (self.ip, self.mask) {
            (IpAddr::V4(ip), IpAddr::V4(mask)) => IpAddr::V4(ip | !mask),
            (IpAddr::V6(ip), IpAddr::V6(mask)) => IpAddr::V6(ip | !mask),
            (_, _) => unreachable!("mismatch ip and subnet ip versions"),
        }
    }

    /// Returns true of the provided IP address is contained in this network
    ///
    /// If an IPv6 address is passed to an IPv4 network, returns false.
    /// If an IPv4 address is passed to an IPv6 network, returns false.
    ///
    /// ### Arguments
    /// * `ip` - IP address to check if in cidr / network
    pub fn contains<I: Into<IpAddr>>(&self, ip: I) -> bool {
        match (ip.into(), self.mask) {
            (IpAddr::V4(ip), IpAddr::V4(mask)) => (ip & mask) == self.network(),
            (IpAddr::V6(ip), IpAddr::V6(mask)) => (ip & mask) == self.network(),
            (_, _) => false,
        }
    }
}

impl PartialEq<IpAddr> for NetworkAddress {
    fn eq(&self, other: &IpAddr) -> bool {
        self.ip == *other
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::NetworkAddress;

    #[test]
    fn create_cidr_ipv4() {
        let cidr = NetworkAddress::new_v4([10, 10, 10, 1], 24);
        assert_eq!("10.10.10.1/24", &cidr.to_string());
    }

    #[test]
    fn get_network_addresss_ipv4() {
        let cidr = NetworkAddress::new_v4([10, 10, 10, 1], 24);
        let net = cidr.network();
        assert_eq!("10.10.10.0", &net.to_string());
    }

    #[test]
    fn get_broadcast_addresss_ipv4() {
        let cidr = NetworkAddress::new_v4([10, 10, 10, 1], 24);
        let net = cidr.broadcast();
        assert_eq!("10.10.10.255", &net.to_string());
    }

    #[test]
    fn contains_ipv4_good() {
        let cidr = NetworkAddress::new_v4([10, 10, 10, 1], 24);
        let val = cidr.contains(Ipv4Addr::from([10, 10, 10, 45]));
        assert_eq!(val, true, "10.10.10.0/24 cidr should contain 10.10.10.45");
    }

    #[test]
    fn contains_ipv4_bad() {
        let cidr = NetworkAddress::new_v4([10, 10, 10, 1], 24);
        let val = cidr.contains(Ipv4Addr::from([10, 10, 11, 45]));
        assert_eq!(
            val, false,
            "10.10.10.0/24 cidr should not contain 10.10.11.45"
        );
    }
}
