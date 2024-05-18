//! Simple Router

use std::{
    borrow::Cow,
    collections::HashMap,
    fs::File,
    net::{IpAddr, Ipv4Addr},
    path::PathBuf,
    sync::Arc,
    time::UNIX_EPOCH,
};

use flume::{Receiver, Sender};
use oathgate_net::{
    protocols::ArpPacket,
    types::{EtherType, MacAddress},
    EthernetFrame, Ipv4Header, ProtocolError,
};
use parking_lot::Mutex;
use pcap_file::pcap::{PcapPacket, PcapWriter};

use crate::{device::TapDeviceRxQueue, upstream::BoxUpstreamHandle};

use self::local::LocalDevice;

mod local;

const ETHERNET_HDR_SZ: usize = 14;

pub trait RouterPort: Send + Sync {
    fn enqueue(&self, frame: EthernetFrame, pkt: Vec<u8>);
}

pub enum RouterMsg {
    /// An Ethernet-Framed packet
    L2Packet(usize, Vec<u8>),

    /// An IPv4/IPv6 packet
    L3Packet(EtherType, Vec<u8>),

    /// Sets the default route to the associated sender
    SetUpstream(BoxUpstreamHandle),
}

pub enum RouterAction {
    Respond(IpAddr, Vec<u8>),
    Forward(Ipv4Header, Vec<u8>),
    Drop(Vec<u8>),
}

pub struct Router {
    pcap: Option<PcapWriter<File>>,
    devices: Arc<Mutex<Vec<Box<dyn RouterPort>>>>,
    ports: HashMap<MacAddress, usize>,
    arp: HashMap<IpAddr, MacAddress>,
    local: LocalDevice,
    upstream: Option<BoxUpstreamHandle>,
    mac: MacAddress,
    ip4: Ipv4Addr,
}

#[derive(Debug, Default)]
pub struct RouterBuilder {
    pcap: Option<PathBuf>,
    upstream: Option<Sender<()>>,
}

#[derive(Clone)]
pub struct RouterHandle {
    tx: Sender<RouterMsg>,
    devices: Arc<Mutex<Vec<Box<dyn RouterPort>>>>,
}

#[allow(dead_code)]
impl RouterBuilder {
    /// Set the file to save pcap output to, or None to disable pcap completely
    ///
    /// If the file does not exist, it will be created, if the file already exists,
    /// it will be truncated.
    ///
    /// ### Arguments
    /// * `pcap` - Path on disk to location of pcap file
    pub fn pcap<P: Into<Option<PathBuf>>>(mut self, pcap: P) -> Self {
        self.pcap = pcap.into();
        self
    }

    pub fn upstream(mut self, tx: Sender<()>) -> Self {
        self.upstream = Some(tx);
        self
    }

    /// Create the router, spawning a new thread to run the core logic
    ///
    /// ### Arguments
    /// * `ip4` - IPv4 address to assign to this router
    pub fn build<I: Into<Ipv4Addr>>(self, ip4: I) -> std::io::Result<RouterHandle> {
        let ip4 = ip4.into();

        let pcap_writer = match self.pcap.as_ref() {
            None => None,
            Some(f) => {
                let file = File::options()
                    .create(true)
                    .write(true)
                    .open(f)
                    .expect("unable to open pcap file");
                let wr = PcapWriter::new(file).expect("unable to create pcap writer");
                tracing::info!(path = ?f, "logging pcap to file");
                Some(wr)
            }
        };

        let (tx, rx) = flume::unbounded();
        let devices = Arc::new(Mutex::new(Vec::new()));
        let handle = RouterHandle {
            tx,
            devices: Arc::clone(&devices),
        };

        let local = LocalDevice::new(ip4);

        let router = Router {
            pcap: pcap_writer,
            devices,
            ports: HashMap::new(),
            arp: HashMap::new(),
            local,
            upstream: None,
            mac: MacAddress::generate(),
            ip4,
        };

        std::thread::Builder::new()
            .name(String::from("router"))
            .spawn(move || router.run(rx))?;

        Ok(handle)
    }
}

impl Router {
    pub fn builder() -> RouterBuilder {
        RouterBuilder::default()
    }

    fn run(mut self, rx: Receiver<RouterMsg>) {
        loop {
            match rx.recv() {
                Ok(RouterMsg::L2Packet(port, pkt)) => match self.switch(port, pkt) {
                    Ok(_) => tracing::trace!("routed l2 packet"),
                    Err(error) => tracing::warn!(?error, "unable to route l2 packet"),
                },
                Ok(RouterMsg::L3Packet(ty, pkt)) => match self.route(ty, pkt) {
                    Ok(_) => tracing::trace!("routed l3 packet"),
                    Err(error) => tracing::warn!(?error, "unable to route l2 packet"),
                },
                Ok(RouterMsg::SetUpstream(handle)) => {
                    self.upstream = Some(handle);
                }
                Err(error) => {
                    tracing::error!(?error, "router channel closed");
                    break;
                }
            }
        }

        tracing::debug!("router thread died");
    }

    fn log_packet(&mut self, pkt: &[u8]) {
        if let Some(pwr) = self.pcap.as_mut() {
            // write packet to pcap file
            match pwr.write_packet(&PcapPacket {
                timestamp: UNIX_EPOCH.elapsed().unwrap(),
                orig_len: pkt.len() as u32,
                data: Cow::Borrowed(&pkt),
            }) {
                Ok(_) => (),
                Err(error) => tracing::warn!(?error, "unable to log pcap"),
            }
        }
    }

    fn switch(&mut self, port: usize, mut pkt: Vec<u8>) -> Result<(), ProtocolError> {
        if pkt.len() < ETHERNET_HDR_SZ {
            return Err(ProtocolError::NotEnoughData(pkt.len(), ETHERNET_HDR_SZ));
        }

        self.log_packet(&pkt);

        let ef = EthernetFrame::extract(&mut pkt)?;

        // associate MAC address of source with port
        match self.ports.insert(ef.src, port) {
            Some(old_port) if port == old_port => { /* do nothing, no port change */ }
            Some(old_port) => {
                tracing::debug!(port, old_port, ?ef.src, "associating mac with new port")
            }
            None => tracing::debug!(mac = ?ef.src, port, "associating mac with router port"),
        }

        self.route(ef.ethertype, pkt)
    }

    /// Routes a packet based on it's packet type
    ///
    /// ### Arguments
    /// * `ethertype` - What type of data is contained in the packet
    /// * `pkt` - Packet data (based on ethertype)
    fn route(&mut self, ethertype: EtherType, pkt: Vec<u8>) -> Result<(), ProtocolError> {
        if pkt.len() < ETHERNET_HDR_SZ {
            return Err(ProtocolError::NotEnoughData(pkt.len(), ETHERNET_HDR_SZ));
        }

        let action = match ethertype {
            EtherType::ARP => self.handle_arp(pkt),
            EtherType::IPv4 => self.route_ip4(pkt),
            EtherType::IPv6 => self.route_ip6(pkt),
        }?;

        match action {
            RouterAction::Respond(dst, pkt) => match self.arp.get(&dst) {
                Some(dst) => self.to_device(*dst, ethertype, pkt),
                None => {
                    tracing::warn!(ip = ?dst, "[router] mac not found in arp cache, dropping packet")
                }
            },
            RouterAction::Forward(dst, pkt) => match self.upstream.as_ref() {
                Some(upstream) => {
                    upstream.write(dst, pkt).ok();
                }
                None => tracing::warn!("[router] no upstream device found"),
            },
            RouterAction::Drop(_pkt) => tracing::debug!("[router] dropping packet"),
        }

        Ok(())
    }

    /// Returns true if a packet is destined for this local device
    fn is_local<A: Into<IpAddr>>(&self, dst: A) -> bool {
        match dst.into() {
            IpAddr::V4(dst) => dst == self.ip4,
            IpAddr::V6(_) => false,
        }
    }

    fn handle_arp(&mut self, pkt: Vec<u8>) -> Result<RouterAction, ProtocolError> {
        tracing::trace!("handling arp packet");
        let mut arp = ArpPacket::parse(&pkt)?;

        tracing::debug!("associating mac to ip: {:?} -> {}", arp.spa, arp.sha);
        self.arp.insert(arp.spa, arp.sha);

        if self.is_local(arp.tpa) {
            // responsd with router's mac
            let mut rpkt = vec![0u8; arp.size()];
            arp.to_reply(self.mac);
            arp.as_bytes(&mut rpkt);
            Ok(RouterAction::Respond(arp.tpa, rpkt))
        } else {
            // TODO: broadcast to all other connected devices
            Ok(RouterAction::Drop(pkt))
        }
    }

    fn route_ip4(&mut self, mut pkt: Vec<u8>) -> Result<RouterAction, ProtocolError> {
        let ip_hdr = Ipv4Header::extract(&mut pkt)?;
        match self.local.can_handle(ip_hdr.dst) {
            true => Ok(self.local.handle_ip4_packet(ip_hdr, pkt)),
            false => Ok(RouterAction::Forward(ip_hdr, pkt)),
        }
    }

    fn route_ip6(&self, pkt: Vec<u8>) -> Result<RouterAction, ProtocolError> {
        tracing::warn!("ipv6 not supported");
        Ok(RouterAction::Drop(pkt))
    }

    /// Builds the Ethernet (layer 2) header and queues the packet to
    /// be processed by the virtqueues
    ///
    /// ### Arguments
    /// * `src` - Source MAC address
    /// * `dst` - Destination MAC Address
    /// * `ty` - Type of payload
    /// * `data` - Layer3+ data
    fn to_device(&mut self, dst: MacAddress, ty: EtherType, pkt: Vec<u8>) {
        self.log_packet(&pkt);

        // build the EF header
        let frame = EthernetFrame::new(self.mac, dst, ty);

        // get the port to send out on
        match self.ports.get(&dst) {
            None => tracing::warn!(?dst, "mac address not associated with port on router"),
            Some(port) => {
                let devs = self.devices.lock();
                match devs.get(*port) {
                    None => tracing::warn!(?dst, ?port, "no device connected to port"),
                    Some(dev) => dev.enqueue(frame, pkt),
                }
            }
        }
    }
}

impl RouterHandle {
    /// Sends a packet to the router
    ///
    /// ### Arguments
    /// * `port` - Port device is connected to
    /// * `pkt` - Ethernet-Framed (i.e. Layer 2) packet
    pub fn switch(&self, port: usize, pkt: Vec<u8>) {
        self.tx.send(RouterMsg::L2Packet(port - 1, pkt)).ok();
    }

    pub fn route(&self, ty: EtherType, pkt: Vec<u8>) {
        self.tx.send(RouterMsg::L3Packet(ty, pkt)).ok();
    }

    /// Registers the provided upstream handle as the default route
    ///
    /// ### Arguments
    /// * `handle` - Default route for packets not destined for conencted devices
    pub fn set_upstream(&self, handle: BoxUpstreamHandle) {
        self.tx.send(RouterMsg::SetUpstream(handle)).ok();
    }

    /// Connects a new device to the router, returning the port it is connected to
    pub fn connect<P: RouterPort + 'static>(&self, port: P) -> usize {
        let mut devices = self.devices.lock();
        let idx = devices.len();
        devices.push(Box::new(port));

        idx + 1
    }
}
