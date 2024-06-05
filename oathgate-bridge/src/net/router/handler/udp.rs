//! ICMP Protocol Handler

use std::net::IpAddr;

use dhcproto::{v4::{DhcpOption, Message, MessageType}, Decodable, Decoder, Encodable, Encoder};
use oathgate_net::{
    protocols::{NET_PROTOCOL_UDP, UDP_HDR_SZ}, types::NetworkAddress, Ipv4Packet, ProtocolError
};

const UDP_PORT_DHCP_SRV: u16 = 67;

use super::ProtocolHandler;

#[derive(Default)]
pub struct UdpHandler;

impl ProtocolHandler for UdpHandler {
    fn protocol(&self) -> u8 {
        NET_PROTOCOL_UDP
    }

    fn handle_protocol(&self, pkt: &Ipv4Packet, buf: &mut [u8]) -> Result<usize, ProtocolError> {
        let payload = pkt.payload();

        if payload.len() < UDP_HDR_SZ {
            return Err(ProtocolError::NotEnoughData(payload.len(), UDP_HDR_SZ))?;
        }

        let src_port = u16::from_be_bytes([payload[0], payload[1]]);
        let dst_port = u16::from_be_bytes([payload[2], payload[3]]);

        match dst_port {
            UDP_PORT_DHCP_SRV => {
                let sz = self.handle_dhcp(&payload[8..], &mut buf[8..])?;
                let len = sz + 8;

                buf[0..2].copy_from_slice(&dst_port.to_be_bytes());
                buf[2..4].copy_from_slice(&src_port.to_be_bytes());
                buf[4..6].copy_from_slice(&len.to_be_bytes()[6..8]);
                buf[6..8].copy_from_slice(&[0x00, 0x00]);
                Ok(len)
            }
            _ => Ok(0)
        }
    }
}

impl UdpHandler {
    pub fn handle_dhcp(&self, payload: &[u8], buf: &mut [u8]) -> Result<usize, ProtocolError> {
        let network = NetworkAddress::new_v4([10, 10, 10, 1], 24);
        let myip = match network.ip() {
            IpAddr::V4(ip) => ip,
            _ => panic!("not an ipv4 address"),
        };

        let mysubnet = match network.subnet_mask() {
            IpAddr::V4(subnet) => subnet,
            _ => panic!("not an ipv4 address"),
        };

        let mybroadcast = match network.broadcast () {
            IpAddr::V4(bcast) => bcast,
            _ => panic!("not an ipv4 address"),
        };


        tracing::debug!("handling dhcp packet");
        let msg = Message::decode(&mut Decoder::new(payload))
            .map_err(|e| ProtocolError::Other(e.to_string()))?;

        let mut vbuf = Vec::with_capacity(256);
        let mut encoder = Encoder::new(&mut vbuf);
      
        tracing::debug!(?msg, "dhcp message");

        let mut rmsg = Message::default();
        let ops = msg.opts();
        match ops.msg_type().ok_or_else(|| ProtocolError::Other("dhcp missing msg type".into()))? {
            MessageType::Discover => {
                rmsg.set_flags(msg.flags());
                rmsg.set_opcode(dhcproto::v4::Opcode::BootReply);
                rmsg.set_htype(msg.htype());
                rmsg.set_xid(msg.xid());
                rmsg.set_yiaddr([10, 10, 10, 213]);
                rmsg.set_siaddr(myip);
                rmsg.set_giaddr(msg.giaddr());
                rmsg.set_chaddr(msg.chaddr());
                rmsg.opts_mut()
                    .insert(DhcpOption::MessageType(MessageType::Offer));
                rmsg.opts_mut()
                    .insert(DhcpOption::AddressLeaseTime(86400 /* 1 day */));
                rmsg.opts_mut()
                    .insert(DhcpOption::ServerIdentifier(myip));
                rmsg.opts_mut()
                    .insert(DhcpOption::SubnetMask(mysubnet));
                rmsg.opts_mut()
                    .insert(DhcpOption::BroadcastAddr(mybroadcast));
                rmsg.opts_mut()
                    .insert(DhcpOption::Router(vec![myip]));
                rmsg.opts_mut()
                    .insert(DhcpOption::DomainNameServer(vec![[1, 1, 1, 1].into()]));

                for (code, opt) in ops.iter() {
                    tracing::debug!("[dhcp-request] code: {code:?}, option: {opt:?}");
                }

                tracing::debug!(?rmsg, "dhcp offer response");
                rmsg.encode(&mut encoder).map_err(|e| ProtocolError::Other(e.to_string()))?;
            },
            MessageType::Request => {
                rmsg.set_flags(msg.flags());
                rmsg.set_opcode(dhcproto::v4::Opcode::BootReply);
                rmsg.set_htype(msg.htype());
                rmsg.set_xid(msg.xid());
                rmsg.set_ciaddr(msg.ciaddr());
                rmsg.set_yiaddr([10, 10, 10, 213]);
                rmsg.set_siaddr(myip);
                rmsg.set_giaddr(msg.giaddr());
                rmsg.set_chaddr(msg.chaddr());
                rmsg.opts_mut()
                    .insert(DhcpOption::MessageType(MessageType::Ack));
                rmsg.opts_mut()
                    .insert(DhcpOption::AddressLeaseTime(86400 /* 1 day */));
                rmsg.opts_mut()
                    .insert(DhcpOption::ServerIdentifier(myip));
                rmsg.opts_mut()
                    .insert(DhcpOption::SubnetMask(mysubnet));
                rmsg.opts_mut()
                    .insert(DhcpOption::BroadcastAddr(mybroadcast));
                rmsg.opts_mut()
                    .insert(DhcpOption::Router(vec![myip]));
                rmsg.opts_mut()
                    .insert(DhcpOption::DomainNameServer(vec![[1, 1, 1, 1].into()]));

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
