//! Simple L3 Router

mod switch;

pub mod handler;
pub mod wan;

use std::{
    borrow::Cow, collections::HashMap, net::{IpAddr, Ipv4Addr}
};

use flume::{Receiver, Sender};

pub use self::switch::{Switch, SwitchPort};

use crate::{
    protocols::ArpPacket,
    types::{EtherType, MacAddress, NetworkAddress},
    EthernetFrame, EthernetPacket, Ipv4Packet, ProtocolError,
};

use self::{
    handler::ProtocolHandler,
    wan::{Wan, WanHandle},
};

const ETHERNET_HDR_SZ: usize = 14;
const IPV4_HDR_SZ: usize = 20;

pub enum RouterMsg {
    Lan(EthernetPacket),
    Wan4(Ipv4Packet),
}

pub enum RouterAction {
    ToLan(EtherType, IpAddr, Vec<u8>),
    ToWan(Ipv4Packet),
    Drop(Vec<u8>),
}

#[derive(Clone)]
pub struct RouterHandle {
    tx: Sender<RouterMsg>,
}

pub struct Router {
    arp: HashMap<IpAddr, MacAddress>,
    switch: Switch,
    port: usize,
    wan: Option<Box<dyn WanHandle>>,
    mac: MacAddress,
    network: NetworkAddress,
    ip4_handlers: HashMap<u8, Box<dyn ProtocolHandler>>,
}

pub struct RouterBuilder {
    /// Mapping of ipv4 protocol numbers to a handler to run when a packet
    /// matching the protocol is received
    ip4_handlers: HashMap<u8, Box<dyn ProtocolHandler>>,

    /// Wide Area Network (WAN) connection
    wan: Option<Box<dyn Wan>>,
}

/// Collection of errors that may occur during routing/switching packets
#[derive(Debug, thiserror::Error)]
pub enum RouterError {
    #[error("router: {0}")]
    Generic(Cow<'static, str>),

    #[error("nix: {0}")]
    Errno(#[from] nix::errno::Errno),

    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),

    #[error("protocol failed: {0}")]
    Protocol(#[from] ProtocolError),

    #[error("pcap: {0}")]
    Pcap(#[from] pcap_file::PcapError),

    #[error("unable to decode base64: {0}")]
    DecodeSlice(#[from] base64::DecodeSliceError),

    #[error("sender channel closed")]
    ChannelClosed,
}

impl<T> From<flume::SendError<T>> for RouterError {
    fn from(_: flume::SendError<T>) -> Self {
        Self::ChannelClosed 
    }
}

#[allow(dead_code)]
impl RouterBuilder {
    pub fn wan(mut self, wan: Option<Box<dyn Wan>>) -> Self {
        self.wan = wan;
        self
    }

    pub fn register_proto_handler<P: ProtocolHandler + 'static>(mut self, handler: P) -> Self {
        let proto = handler.protocol();
        self.ip4_handlers.insert(proto, Box::new(handler));
        self
    }

    /// Create the router, spawning a new thread to run the core logic
    ///
    /// ### Arguments
    /// * `ip4` - IPv4 address to assign to this router
    pub fn spawn<I: Into<Ipv4Addr>>(self, ip4: I, switch: Switch) -> std::io::Result<()> {
        let network = NetworkAddress::new(ip4.into(), 24);

        let (tx, rx) = flume::unbounded();

        let handle = RouterHandle { tx };
        let port = switch.connect(handle.clone());

        let wan = self.wan.and_then(|wan| match wan.spawn(handle) {
            Ok(handle) => Some(handle),
            Err(error) => {
                tracing::warn!(?error, "unable to start wan");
                None
            }
        });

        let router = Router {
            arp: HashMap::new(),
            switch,
            port,
            wan,
            mac: MacAddress::generate(),
            network,
            ip4_handlers: self.ip4_handlers,
        };

        std::thread::Builder::new()
            .name(String::from("router"))
            .spawn(move || router.run(rx))?;

        Ok(())
    }
}

impl Router {
    pub fn builder() -> RouterBuilder {
        RouterBuilder {
            ip4_handlers: HashMap::new(),
            wan: None,
        }
    }

    pub fn run(mut self, rx: Receiver<RouterMsg>) {
        loop {
            match rx.recv() {
                Ok(RouterMsg::Lan(pkt)) => match self.route(pkt.frame.ethertype, pkt.payload) {
                    Ok(_) => (),
                    Err(error) => tracing::warn!(?error, "unable to route lan packet"),
                },
                Ok(RouterMsg::Wan4(pkt)) => {
                    if let Err(error) = self
                        .route_ip4(pkt)
                        .and_then(|action| self.handle_action(action))
                    {
                        tracing::warn!(?error, "unable to route wan packet");
                    }
                }
                Err(error) => {
                    tracing::error!(?error, "unable to receive packet");
                    break;
                }
            }
        }

        tracing::info!("router died");
    }

    /// Routes a packet based on it's packet type
    ///
    /// ### Arguments
    /// * `ethertype` - What type of data is contained in the packet
    /// * `pkt` - Packet data (based on ethertype)
    fn route(&mut self, ethertype: EtherType, pkt: Vec<u8>) -> Result<(), ProtocolError> {
        let action = match ethertype {
            EtherType::ARP => self.handle_arp(pkt),
            EtherType::IPv4 => {
                let pkt = Ipv4Packet::parse(pkt)?;
                self.route_ip4(pkt)
            }
            EtherType::IPv6 => self.route_ip6(pkt),
        }?;

        self.handle_action(action)
    }

    fn handle_action(&mut self, action: RouterAction) -> Result<(), ProtocolError> {
        match action {
            RouterAction::ToLan(ethertype, dst, pkt) => match self.arp.get(&dst) {
                Some(dst) => self.write_to_switch(*dst, ethertype, pkt),
                None => {
                    tracing::warn!(ip = ?dst, "[router] mac not found in arp cache, dropping packet")
                }
            },
            RouterAction::ToWan(pkt) => match self.forward_packet(pkt) {
                Ok(_) => tracing::trace!("[router] forwarded packet"),
                Err(error) => tracing::warn!(?error, "[router] unable to forward packet"),
            },
            RouterAction::Drop(_pkt) => tracing::debug!("[router] dropping packet"),
        }

        Ok(())
    }

    /// Returns true if a packet is destined for this local device
    fn is_local<A: Into<IpAddr>>(&self, dst: A) -> bool {
        let dst = dst.into();
        self.network == dst
    }

    fn handle_arp(&mut self, pkt: Vec<u8>) -> Result<RouterAction, ProtocolError> {
        tracing::trace!("handling arp packet");
        let mut arp = ArpPacket::parse(&pkt)?;

        tracing::trace!(
            "[router] associating mac to ip: {:?} -> {}",
            arp.spa,
            arp.sha
        );
        self.arp.insert(arp.spa, arp.sha);

        if self.is_local(arp.tpa) {
            // responsd with router's mac
            let mut rpkt = vec![0u8; arp.size()];
            arp.to_reply(self.mac);
            arp.as_bytes(&mut rpkt);
            Ok(RouterAction::ToLan(EtherType::ARP, arp.tpa, rpkt))
        } else {
            // Not for us..ignore the packet
            Ok(RouterAction::Drop(pkt))
        }
    }

    fn forward_packet(&mut self, pkt: Ipv4Packet) -> Result<(), RouterError> {
        if let Some(ref wan) = self.wan {
            if let Err(error) = wan.write(pkt) {
                tracing::warn!(?error, "unable to write to wan, dropping packet");
            }
        } else {
            tracing::warn!("[router] no wan device, dropping packet");
        }
        Ok(())
    }

    fn route_ip4(&mut self, pkt: Ipv4Packet) -> Result<RouterAction, ProtocolError> {
        match self.network.contains(pkt.dest()) {
            true => match self.is_local(pkt.dest()) {
                true => Ok(self.handle_local_ipv4(pkt)),
                false => {
                    let dst = pkt.dest();
                    Ok(RouterAction::ToLan(
                        EtherType::IPv4,
                        IpAddr::V4(dst),
                        pkt.into_bytes(),
                    ))
                }
            },
            false => Ok(RouterAction::ToWan(pkt)),
        }
    }

    fn route_ip6(&self, pkt: Vec<u8>) -> Result<RouterAction, ProtocolError> {
        tracing::warn!("ipv6 not supported");
        Ok(RouterAction::Drop(pkt))
    }

    fn handle_local_ipv4(&self, pkt: Ipv4Packet) -> RouterAction {
        let mut rpkt = vec![0u8; 1560];

        match self.ip4_handlers.get(&pkt.protocol()) {
            Some(ref handler) => {
                match handler.handle_protocol(&pkt, &mut rpkt[IPV4_HDR_SZ..]) {
                    Ok(0) => RouterAction::Drop(Vec::new()),
                    Ok(sz) => {
                        rpkt.truncate(IPV4_HDR_SZ + sz);

                        // build response ipv4 header
                        let hdr = pkt.reply(&rpkt[IPV4_HDR_SZ..]);
                        hdr.as_bytes(&mut rpkt[0..IPV4_HDR_SZ]);

                        RouterAction::ToLan(EtherType::IPv4, hdr.dst.into(), rpkt)
                    }
                    Err(error) => {
                        tracing::warn!(
                            ?error,
                            protocol = pkt.protocol(),
                            "unable to handle packet"
                        );
                        RouterAction::Drop(Vec::new())
                    }
                }
            }
            None => RouterAction::Drop(vec![]),
        }
    }

    fn write_to_switch(&self, dst: MacAddress, ethertype: EtherType, mut pkt: Vec<u8>) {
        let mut data = Vec::with_capacity(12 + pkt.len());
        data.extend_from_slice(&dst.as_bytes());
        data.extend_from_slice(&self.mac.as_bytes());
        data.extend_from_slice(&ethertype.as_u16().to_be_bytes());
        data.append(&mut pkt);

        tracing::trace!("[router] write to switch: {:02x?}", &data[14..34]);

        if let Err(error) = self.switch.process(self.port, data) {
            tracing::warn!(?error, "unable to write to switch");
        }
    }
}

impl RouterHandle {
    fn route_ipv4(&self, pkt: Ipv4Packet) {
        self.tx.send(RouterMsg::Wan4(pkt)).ok();
    }

    fn route_ipv6(&self, _pkt: Vec<u8>) {
        tracing::warn!("[router] no ipv6 support");
    }
}

impl SwitchPort for RouterHandle {
    fn enqueue(&self, frame: EthernetFrame, pkt: Vec<u8>) {
        let pkt = EthernetPacket::new(frame, pkt);
        self.tx.send(RouterMsg::Lan(pkt)).ok();
    }
}
