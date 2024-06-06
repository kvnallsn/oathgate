//! Network Address

use std::{
    fmt::Display,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    str::FromStr,
};

use serde::{de::Visitor, Deserialize, Serialize};

const BROADCAST4: Ipv4Addr = Ipv4Addr::new(255, 255, 255, 255);

#[derive(Debug)]
pub struct NetworkAddress {
    ip: IpAddr,
    mask: IpAddr,
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
    /// or if the address is the univeral broadcast address (i.e. 255.255.255.255)
    ///
    /// If an IPv6 address is passed to an IPv4 network, returns false.
    /// If an IPv4 address is passed to an IPv6 network, returns false.
    ///
    /// ### Arguments
    /// * `ip` - IP address to check if in cidr / network
    pub fn contains<I: Into<IpAddr>>(&self, ip: I) -> bool {
        match (ip.into(), self.mask) {
            (IpAddr::V4(BROADCAST4), _) => true,
            (IpAddr::V4(ip), IpAddr::V4(mask)) => (ip & mask) == self.network(),
            (IpAddr::V6(ip), IpAddr::V6(mask)) => (ip & mask) == self.network(),
            (_, _) => false,
        }
    }
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

impl FromStr for NetworkAddress {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split("/");
        let ip = parts.next().ok_or_else(|| "missing ip component")?;
        let mask = parts.next().unwrap_or_else(|| "32");

        let ip: IpAddr = ip.parse().map_err(|_| "unable to parse ip address")?;
        let mask: u8 = mask.parse().map_err(|_| "unable to parse subnet mask")?;

        Ok(Self::new(ip, mask))
    }
}

impl PartialEq<IpAddr> for NetworkAddress {
    fn eq(&self, other: &IpAddr) -> bool {
        self.ip == *other
    }
}

struct NetworkAddressVisitor;

impl<'de> Visitor<'de> for NetworkAddressVisitor {
    type Value = NetworkAddress;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a network address, like 192.168.2.1/24")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        v.parse::<NetworkAddress>()
            .map_err(|e| E::custom(e.to_string()))
    }
}

impl<'de> Deserialize<'de> for NetworkAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_str(NetworkAddressVisitor)
    }
}

impl Serialize for NetworkAddress {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
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

    #[test]
    fn parse_ipv4_string() {
        let cidr: NetworkAddress = "10.10.10.213/25".parse().unwrap();
        let net = cidr.network();
        let broadcast = cidr.broadcast();
        assert_eq!("10.10.10.128", &net.to_string(), "network mismatch");
        assert_eq!("10.10.10.255", &broadcast.to_string(), "broadcast mismatch");
    }
}
