//! Simple DHCP server

use std::{
    collections::{HashMap, VecDeque},
    net::Ipv4Addr,
};

use dhcproto::{v4, Decodable, Decoder, Encodable, Encoder};
use oathgate_net::{
    types::{Ipv4Network, MacAddress},
    ProtocolError,
};

use crate::config::dhcp::DhcpConfig;

use super::router::handler::PortHandler;

#[derive(Debug)]
pub struct DhcpServer {
    network: Ipv4Network,
    lease_time: u32,

    available: VecDeque<Ipv4Addr>,
    leased: HashMap<Ipv4Addr, MacAddress>,
}

impl DhcpServer {
    pub fn new(network: Ipv4Network, cfg: DhcpConfig) -> Self {
        if !network.contains(cfg.start) || !network.contains(cfg.end) {
            // TODO: return error
        }

        let mut next_avail = Ipv4Network::new(cfg.start, network.subnet_mask_bits());
        tracing::debug!("[dhcp] created server: {network:?}");

        // generate list of available IPs
        let mut available = VecDeque::new();
        loop {
            available.push_back(next_avail.ip());
            match next_avail.next() {
                None => break,
                Some(nxt) if nxt.ip() > cfg.end => break,
                Some(nxt) => next_avail = nxt,
            }
        }

        Self {
            network,
            lease_time: 86400, // 1 day
            available,
            leased: HashMap::new(),
        }
    }

    pub fn lease_ip(&mut self, msg: &v4::Message) -> Option<Ipv4Network> {
        let client_mac = MacAddress::parse(msg.chaddr()).unwrap();

        let ip = match self.get_requested_ip(msg) {
            Some(ria) => match self.leased.get(&ria) {
                None => {
                    self.available.retain(|ip| *ip != ria);
                    Some(ria)
                }
                Some(mac) if *mac == client_mac => {
                    self.available.retain(|ip| *ip != ria);
                    Some(ria)
                }
                _ => self.available.pop_front(),
            },
            None => self.available.pop_front(),
        };

        match ip {
            Some(ip) => {
                self.leased.insert(ip, client_mac);
                Some(Ipv4Network::new(ip, self.network.subnet_mask_bits()))
            }
            None => {
                // no ips available
                None
            }
        }
    }

    fn get_requested_ip(&self, msg: &v4::Message) -> Option<Ipv4Addr> {
        match msg.opts().get(v4::OptionCode::RequestedIpAddress) {
            Some(v4::DhcpOption::RequestedIpAddress(ip)) => Some(*ip),
            _ => None,
        }
    }

    pub fn handle_discover(&mut self, msg: v4::Message) -> Result<v4::Message, ProtocolError> {
        tracing::trace!("[dhcp] handling discover message");
        let ip = match self.lease_ip(&msg) {
            Some(ip) => ip,
            None => {
                tracing::warn!("dhcp ip address space exhausted");
                return Err(ProtocolError::Other("address space exhausted".into()));
            }
        };

        let msg = self.build_message(&msg, ip, v4::MessageType::Offer);
        Ok(msg)
    }

    pub fn handle_request(&mut self, msg: v4::Message) -> Result<v4::Message, ProtocolError> {
        tracing::trace!("[dhcp] handling request message");

        let ip = match self.lease_ip(&msg) {
            Some(ip) => ip,
            None => {
                tracing::warn!("dhcp ip address space exhausted");
                return Err(ProtocolError::Other("address space exhausted".into()));
            }
        };

        let msg = self.build_message(&msg, ip, v4::MessageType::Ack);
        Ok(msg)
    }

    fn build_message(
        &self,
        msg: &v4::Message,
        ip: Ipv4Network,
        ty: v4::MessageType,
    ) -> v4::Message {
        use v4::DhcpOption;

        let mut rmsg = v4::Message::default();
        rmsg.set_flags(msg.flags());
        rmsg.set_opcode(dhcproto::v4::Opcode::BootReply);
        rmsg.set_htype(msg.htype());
        rmsg.set_xid(msg.xid());
        rmsg.set_yiaddr(ip.ip());
        if v4::MessageType::Ack == ty {
            rmsg.set_ciaddr(msg.ciaddr());
        }
        rmsg.set_siaddr(self.network.ip());
        rmsg.set_giaddr(msg.giaddr());
        rmsg.set_chaddr(msg.chaddr());
        rmsg.opts_mut().insert(DhcpOption::MessageType(ty));
        rmsg.opts_mut()
            .insert(DhcpOption::AddressLeaseTime(self.lease_time));
        rmsg.opts_mut()
            .insert(DhcpOption::ServerIdentifier(self.network.ip()));
        rmsg.opts_mut()
            .insert(DhcpOption::SubnetMask(self.network.subnet_mask()));
        rmsg.opts_mut()
            .insert(DhcpOption::BroadcastAddr(self.network.broadcast()));
        rmsg.opts_mut()
            .insert(DhcpOption::Router(vec![self.network.ip()]));
        rmsg.opts_mut()
            .insert(DhcpOption::DomainNameServer(vec![[1, 1, 1, 1].into()]));

        rmsg
    }
}

impl PortHandler for DhcpServer {
    fn port(&self) -> u16 {
        67
    }

    fn handle_port(&mut self, data: &[u8], buf: &mut [u8]) -> Result<usize, ProtocolError> {
        tracing::trace!("[dhcp] got packet");
        let msg = v4::Message::decode(&mut Decoder::new(data))
            .map_err(|e| ProtocolError::Other(e.to_string()))?;

        let mut vbuf = Vec::with_capacity(256);
        let mut encoder = Encoder::new(&mut vbuf);

        let ops = msg.opts();
        match ops
            .msg_type()
            .ok_or_else(|| ProtocolError::Other("dhcp missing msg type".into()))?
        {
            v4::MessageType::Discover => {
                let rmsg = self.handle_discover(msg)?;
                rmsg.encode(&mut encoder)
                    .map_err(|e| ProtocolError::Other(e.to_string()))?;
            }
            v4::MessageType::Request => {
                let rmsg = self.handle_request(msg)?;
                rmsg.encode(&mut encoder)
                    .map_err(|e| ProtocolError::Other(e.to_string()))?;
            }
            v4::MessageType::Offer => {
                tracing::debug!("DHCP-OFFER: should not occur, sent by server")
            }
            v4::MessageType::Ack => {
                tracing::debug!("DHCP-ACKNOWLEDGE: should not occur, sent by server")
            }
            v4::MessageType::Release => (),
            v4::MessageType::Decline => (),
            _ => (),
        }

        let len = vbuf.len();
        buf[0..len].copy_from_slice(&vbuf);
        Ok(len)
    }
}
