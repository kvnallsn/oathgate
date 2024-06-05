//! Local device for handling local IP packets

use std::net::{IpAddr, Ipv4Addr};

use oathgate_net::{
    checksum, protocols::ArpPacket, types::{EtherType, MacAddress}, EthernetFrame, Ipv4Header
};

use super::RouterAction;

const MAX_MTU: usize = 1560;

#[derive(Debug)]
pub struct PacketBuffer([u8; MAX_MTU]);

/// Represents an `in-memory` local device to respond to packets sent to a specific IP addrss
#[derive(Debug)]
pub struct LocalDevice {
    /// IPv4 address to assign to this device
    ip4: Ipv4Addr,

    /// MAC address to assign to this device
    mac: MacAddress,

    /// Buffer used to construct packets
    pktbuf: PacketBuffer,
}

impl PacketBuffer {
    /// Creates a new PacketBuffer
    pub fn new() -> Self {
        Self([0u8; MAX_MTU])
    }

    /// Returns the slice of data representing the ethernet frame / layer 2 header
    pub fn l2(&mut self) -> &mut [u8] {
        &mut self.0[0..14]
    }

    /// Returns a slice representing the layer 3 (ip4/ip4/etc.) header
    pub fn l3(&mut self) -> &mut [u8] {
        &mut self.0[14..]
    }
}

impl LocalDevice {
    /// Creates a new `LocalDevice` with the provided IPv4 address and a random MAC address
    ///
    /// ### Arguments
    /// * `ip4` - IPv4 address to assign to this device
    pub fn new(ip4: Ipv4Addr) -> Self {
        let mac = MacAddress::generate();
        Self {
            ip4,
            mac,
            pktbuf: PacketBuffer::new(),
        }
    }

    /// Returns true if a packet is destined for this local device
    pub fn can_handle<A: Into<IpAddr>>(&self, dst: A) -> bool {
        match dst.into() {
            IpAddr::V4(dst) => dst == self.ip4,
            IpAddr::V6(_) => false,
        }
    }

    pub fn handle_arp_request(&mut self, mut arp: ArpPacket) -> Vec<u8> {
        arp.to_reply(self.mac);
        let ef = EthernetFrame::new(arp.sha, arp.tha, EtherType::ARP);

        let mut rpkt = vec![0u8; EthernetFrame::size() + arp.size()];
        ef.as_bytes(&mut rpkt);
        arp.as_bytes(&mut rpkt[EthernetFrame::size()..]);

        ef.as_bytes(self.pktbuf.l2());
        arp.as_bytes(self.pktbuf.l3());

        rpkt
    }

    /// Handles an ip4 packet sent to this device
    pub fn handle_ip4_packet(
        &self,
        iphdr: Ipv4Header,
        payload: Vec<u8>,
    ) -> RouterAction {
        const L3_HEADER_SZ: usize = 20;

        let mut rpkt = vec![0u8; MAX_MTU];

        let resp = match iphdr.protocol {
            1 /* ICMP */ => match payload[0] {
                8 /* ECHO REQUEST */ => {
                    Some(self.handle_icmp_echo_req(&payload, &mut rpkt[L3_HEADER_SZ..]))
                }
                _ => {
                    tracing::warn!("[local-device] unhandled icmp packet (type = {}, code = {}", payload[0], payload[1]);
                    None
                }
            },
            protocol => {
                tracing::warn!(protocol, "[local-device] unhandled protocol");
                None
            }
        };

        match resp {
            Some(len) => {
                // Drop unneeded bytes
                rpkt.truncate(L3_HEADER_SZ + len);

                // build ipv4 header
                let iphdr = iphdr.gen_reply(&rpkt[20..]);
                iphdr.as_bytes(&mut rpkt[0..20]);

                RouterAction::Respond(iphdr.dst.into(), rpkt)
            }
            None => RouterAction::Drop(payload),
        }
    }

    /// Handles a ICMP echo request to the local device ip
    ///
    /// Responds with an echo reply and queues it in the virtqueues rx ring
    fn handle_icmp_echo_req(&self, pkt: &[u8], rpkt: &mut [u8]) -> usize {
        let len = pkt.len();
        tracing::trace!("handling icmp echo request");
        rpkt[4..len].copy_from_slice(&pkt[4..]);

        let csum = checksum(&rpkt);
        rpkt[2..4].copy_from_slice(&csum.to_be_bytes());
        pkt.len()
    }
}
