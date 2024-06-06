//! ICMP Protocol Handler

use std::{collections::HashMap, net::IpAddr};

use dhcproto::{v4::{DhcpOption, Message, MessageType}, Decodable, Decoder, Encodable, Encoder};
use oathgate_net::{
    protocols::{NET_PROTOCOL_UDP, UDP_HDR_SZ}, types::{MacAddress, Ipv4Network}, Ipv4Packet, ProtocolError
};

const UDP_PORT_DHCP_SRV: u16 = 67;

use crate::config::dhcp::DhcpConfig;

use super::{PortHandler, ProtocolHandler};

#[derive(Default)]
pub struct UdpHandler {
    handlers: HashMap<u16, Box<dyn PortHandler>>,
}


impl UdpHandler {
    /// Registers a port handler for this udp handler
    ///
    /// ### Arguments
    /// * `handler` - Implementation of a `PortHandler`
    pub fn register_port_handler<P: PortHandler + 'static>(&mut self, handler: P) {
        self.handlers.insert(handler.port(), Box::new(handler));
    }
}

impl ProtocolHandler for UdpHandler {
    fn protocol(&self) -> u8 {
        NET_PROTOCOL_UDP
    }

    fn handle_protocol(&mut self, pkt: &Ipv4Packet, buf: &mut [u8]) -> Result<usize, ProtocolError> {
        let payload = pkt.payload();

        if payload.len() < UDP_HDR_SZ {
            return Err(ProtocolError::NotEnoughData(payload.len(), UDP_HDR_SZ))?;
        }

        let src_port = u16::from_be_bytes([payload[0], payload[1]]);
        let dst_port = u16::from_be_bytes([payload[2], payload[3]]);

        if let Some(handler) = self.handlers.get_mut(&dst_port) {
            let len = handler.handle_port(&payload[8..], &mut buf[8..])?;
            let len = len + 8;

            buf[0..2].copy_from_slice(&dst_port.to_be_bytes());
            buf[2..4].copy_from_slice(&src_port.to_be_bytes());
            buf[4..6].copy_from_slice(&len.to_be_bytes()[6..8]);
            buf[6..8].copy_from_slice(&[0x00, 0x00]);
            Ok(len)
        } else {
            Ok(0)
        }
    }
}

pub struct DhcpServer {
    network: Ipv4Network,
    start: IpAddr,
    end: IpAddr,
    lease_time: u32,

    next_avail: Option<Ipv4Network>,
    leased: HashMap<IpAddr, MacAddress>,
}

impl DhcpServer {
    pub fn new(network: Ipv4Network, cfg: DhcpConfig) -> Self {
        if !network.contains(cfg.start) || !network.contains(cfg.end) {
            // TODO: return error
        }

        let next_avail = Ipv4Network::new(cfg.start, network.subnet_mask_bits());

        Self {
            network,
            start: cfg.start.into(),
            end: cfg.end.into(),
            lease_time: 86400, // 1 day
            next_avail: Some(next_avail),
            leased: HashMap::new(),
        }
    }

    pub fn lease_ip(&mut self) -> Option<Ipv4Network> {
        if let Some(ip) = self.next_avail.take() {
            self.next_avail = ip.next();
            Some(ip)
        } else {
            None
        }
    }

    pub fn handle_discover(&mut self, msg: Message) -> Result<Message, ProtocolError> {
        let ip = match self.lease_ip() {
            Some(ip) => ip,
            None => {
                tracing::warn!("dhcp ip address space exhausted");
                return Err(ProtocolError::Other("address space exhausted".into()));
            }
        };

        let msg = self.build_message(&msg, ip, MessageType::Offer);
        Ok(msg)
    }

    pub fn handle_request(&mut self, msg: Message) -> Result<Message, ProtocolError> {
        let ip = match self.lease_ip() {
            Some(ip) => ip,
            None => {
                tracing::warn!("dhcp ip address space exhausted");
                return Err(ProtocolError::Other("address space exhausted".into()));
            }
        };

        let msg = self.build_message(&msg, ip, MessageType::Ack);
        Ok(msg)
    }

    fn build_message(&self, msg: &Message, ip: Ipv4Network, ty: MessageType) -> Message {
        let mut rmsg = Message::default();
        rmsg.set_flags(msg.flags());
        rmsg.set_opcode(dhcproto::v4::Opcode::BootReply);
        rmsg.set_htype(msg.htype());
        rmsg.set_xid(msg.xid());
        rmsg.set_yiaddr(ip.ip());
        if MessageType::Ack == ty {
            rmsg.set_ciaddr(msg.ciaddr());
        }
        rmsg.set_siaddr(self.network.ip());
        rmsg.set_giaddr(msg.giaddr());
        rmsg.set_chaddr(msg.chaddr());
        rmsg.opts_mut()
            .insert(DhcpOption::MessageType(ty));
        rmsg.opts_mut()
            .insert(DhcpOption::AddressLeaseTime(86400 /* 1 day */));
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
        UDP_PORT_DHCP_SRV
    }

    fn handle_port(&mut self, data: &[u8], buf: &mut [u8]) -> Result<usize, ProtocolError> {
        tracing::debug!("handling dhcp packet");
        let msg = Message::decode(&mut Decoder::new(data))
            .map_err(|e| ProtocolError::Other(e.to_string()))?;

        let mut vbuf = Vec::with_capacity(256);
        let mut encoder = Encoder::new(&mut vbuf);
      
        tracing::debug!(?msg, "dhcp message");

        let ops = msg.opts();
        match ops.msg_type().ok_or_else(|| ProtocolError::Other("dhcp missing msg type".into()))? {
            MessageType::Discover => {
                let rmsg = self.handle_discover(msg)?;
                rmsg.encode(&mut encoder).map_err(|e| ProtocolError::Other(e.to_string()))?;
            },
            MessageType::Request => {
                let rmsg = self.handle_request(msg)?;
                rmsg.encode(&mut encoder).map_err(|e| ProtocolError::Other(e.to_string()))?;
            }
            MessageType::Offer => tracing::debug!("DHCP-OFFER: should not occur, sent by server"),
            MessageType::Ack=> tracing::debug!("DHCP-ACKNOWLEDGE: should not occur, sent by server"),
            MessageType::Release => (),
            MessageType::Decline => (),
            _ => (),
        }

        let len = vbuf.len();
        buf[0..len].copy_from_slice(&vbuf);
        Ok(len)
    }
}
