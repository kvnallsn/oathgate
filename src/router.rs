//! Simple Router

use std::{
    borrow::Cow, collections::HashMap, fs::File, net::Ipv4Addr, path::PathBuf, sync::Arc,
    time::UNIX_EPOCH,
};

use flume::{Receiver, Sender};
use parking_lot::Mutex;
use pcap_file::pcap::{PcapPacket, PcapWriter};

use crate::{device::TapDeviceRxQueue, error::ProtocolError};

use self::{
    local::LocalDevice,
    protocols::{ArpPacket, EtherType, EthernetFrame, Ipv4Header, MacAddress},
};

mod local;
mod protocols;

const ETHERNET_HDR_SZ: usize = 14;

pub enum RouterMsg {
    Packet(usize, Vec<u8>),
}

pub enum RouterAction {
    Respond(Vec<u8>),
    Forward(Vec<u8>),
    Drop(Vec<u8>),
}

pub struct Router {
    pcap: Option<PcapWriter<File>>,
    devices: Arc<Mutex<Vec<TapDeviceRxQueue>>>,
    ports: HashMap<MacAddress, usize>,
    local: LocalDevice,
}

#[derive(Debug, Default)]
pub struct RouterBuilder {
    pcap: Option<PathBuf>,
    upstream: Option<Sender<()>>,
}

#[derive(Clone, Debug)]
pub struct RouterHandle {
    tx: Sender<RouterMsg>,
    devices: Arc<Mutex<Vec<TapDeviceRxQueue>>>,
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
            local,
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
                Ok(RouterMsg::Packet(port, pkt)) => match self.route(port, pkt) {
                    Ok(_) => tracing::trace!("routed packet"),
                    Err(error) => tracing::warn!(?error, "unable to route packet"),
                },
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

    /// Routes a packet based on it's packet type
    ///
    /// ### Arguments
    /// * `port` - Port packet was received on
    /// * `pkt` - An Ethernet II framed packet
    fn route(&mut self, port: usize, mut pkt: Vec<u8>) -> Result<(), ProtocolError> {
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

        let action = match ef.ethertype {
            EtherType::ARP => self.handle_arp(pkt),
            EtherType::IPv4 => self.route_ip4(ef, pkt),
            EtherType::IPv6 => self.route_ip6(pkt),
        }?;

        match action {
            RouterAction::Respond(pkt) => self.to_device(pkt),
            RouterAction::Forward(_pkt) => tracing::debug!("[router] forwarding packet upstream"),
            RouterAction::Drop(_pkt) => tracing::debug!("[router] dropping packet"),
        }

        Ok(())
    }

    fn handle_arp(&mut self, pkt: Vec<u8>) -> Result<RouterAction, ProtocolError> {
        tracing::trace!("handling arp packet");
        let arp = ArpPacket::parse(&pkt)?;
        if self.local.can_handle(arp.tpa) {
            let pkt = self.local.handle_arp_request(arp);
            Ok(RouterAction::Respond(pkt))
        } else {
            // TODO: broadcast to all other connected devices
            Ok(RouterAction::Drop(pkt))
        }
    }

    fn route_ip4(
        &mut self,
        ef_hdr: EthernetFrame,
        mut pkt: Vec<u8>,
    ) -> Result<RouterAction, ProtocolError> {
        let ip_hdr = Ipv4Header::extract(&mut pkt)?;
        match self.local.can_handle(ip_hdr.dst) {
            true => Ok(self.local.handle_ip4_packet(ef_hdr, ip_hdr, pkt)),
            false => Ok(RouterAction::Forward(pkt)),
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
    fn to_device(&mut self, pkt: Vec<u8>) {
        self.log_packet(&pkt);

        let dst = MacAddress::parse(&pkt[0..6]).expect("dst mac not found");

        // get the port to send out on
        match self.ports.get(&dst) {
            None => tracing::warn!(?dst, "mac address not associated with port on router"),
            Some(port) => {
                let devs = self.devices.lock();
                match devs.get(*port) {
                    None => tracing::warn!(?dst, ?port, "no device connected to port"),
                    Some(dev) => dev.enqueue(pkt),
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
    pub fn route(&self, port: usize, pkt: Vec<u8>) {
        self.tx.send(RouterMsg::Packet(port - 1, pkt)).ok();
    }

    /// Connects a new device to the router, returning the port it is connected to
    pub fn connect(&self, queue: TapDeviceRxQueue) -> usize {
        let mut devices = self.devices.lock();
        let idx = devices.len();
        devices.push(queue);

        idx + 1
    }
}

/// Computes the checksum used in various networking protocols
///
/// Algorithm is the one's complement of the sum of the data as big-ending u16 values
///
/// ### Arguments
/// * `data` - Data to checksum
pub fn checksum(data: &[u8]) -> u16 {
    let mut sum = 0;
    for b in data.chunks(2) {
        let lower = b[0];
        let upper = match b.len() {
            1 => 0x00,
            _ => b[1],
        };

        sum += u32::from_ne_bytes([upper, lower, 0x00, 0x00]);
    }

    !(((sum & 0xFFFF) + ((sum >> 16) & 0xFF)) as u16)
}
