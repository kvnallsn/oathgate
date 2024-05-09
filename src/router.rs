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

use flume::Sender;
use parking_lot::Mutex;
use pcap_file::pcap::{PcapPacket, PcapWriter};

use crate::{device::TapDeviceRxQueue, error::ProtocolError};

use self::protocols::{ArpPacket, EtherType, EthernetFrame, Ipv4Header, MacAddress};

mod protocols;

const ETHERNET_HDR_SZ: usize = 14;

pub enum RouterMsg {
    Packet(usize, Vec<u8>),
}

pub struct Router {
    ip4: Ipv4Addr,
    mac: MacAddress,
    pcap: Option<PcapWriter<File>>,
    devices: Arc<Mutex<Vec<TapDeviceRxQueue>>>,
    ports: HashMap<MacAddress, usize>,
}

#[derive(Clone, Debug)]
pub struct RouterHandle {
    tx: Sender<RouterMsg>,
    devices: Arc<Mutex<Vec<TapDeviceRxQueue>>>,
}

impl Router {
    pub fn new(ip4: Ipv4Addr, pcap: Option<PathBuf>) -> Self {
        let pcap_writer = match pcap.as_ref() {
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

        Router {
            ip4,
            mac: MacAddress::generate(),
            pcap: pcap_writer,
            devices: Arc::new(Mutex::new(Vec::new())),
            ports: HashMap::new(),
        }
    }

    pub fn start(mut self) -> Result<RouterHandle, std::io::Error> {
        let (tx, rx) = flume::unbounded();
        let devices = Arc::clone(&self.devices);
        let handle = RouterHandle { tx, devices };

        std::thread::Builder::new()
            .name(String::from("router"))
            .spawn(move || {
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
            })?;
        Ok(handle)
    }

    pub fn log_packet(&mut self, pkt: &[u8]) {
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
    pub fn route(&mut self, port: usize, mut pkt: Vec<u8>) -> Result<(), ProtocolError> {
        if pkt.len() < ETHERNET_HDR_SZ {
            return Err(ProtocolError::NotEnoughData(pkt.len(), ETHERNET_HDR_SZ));
        }

        self.log_packet(&pkt);

        let ef = EthernetFrame::extract(&mut pkt)?;

        // associate MAC address of source with port
        tracing::debug!(mac = ?ef.src, port, "associating mac with router port");
        self.ports.insert(ef.src, port);

        match ef.ethertype {
            EtherType::ARP => self.route_arp(pkt),
            EtherType::IPv4 => self.route_ip4(ef, pkt),
            EtherType::IPv6 => self.route_ip6(pkt),
        }?;

        Ok(())
    }

    fn route_arp(&mut self, pkt: Vec<u8>) -> Result<(), ProtocolError> {
        tracing::trace!("handling arp packet");
        let mut arp = ArpPacket::parse(&pkt)?;
        if arp.tpa == IpAddr::V4(self.ip4) {
            arp.to_reply(self.mac);

            let src = arp.sha;
            let dst = arp.tha;
            self.to_device(src, dst, EtherType::ARP, arp.to_bytes());
        }

        Ok(())
    }

    fn route_ip4(&mut self, ef_hdr: EthernetFrame, mut pkt: Vec<u8>) -> Result<(), ProtocolError> {
        let ip_hdr = Ipv4Header::extract(&mut pkt)?;
        let resp = if ip_hdr.dst == self.ip4 {
            match ip_hdr.protocol {
                1 /* ICMP */ => match pkt[0] {
                    8 /* ECHO REQUEST */ => {
                        Some(self.handle_icmp_echo_req(&pkt))
                    }
                    _ => {
                        tracing::warn!("[local-device] unhandled icmp packet (type = {}, code = {}", pkt[21], pkt[22]);
                        None
                    }
                },
                _ => {
                    tracing::warn!(protocol = pkt[9], "[local-device] unhandled protocol");
                    None
                }
            }
        } else {
            None
        };

        if let Some(data) = resp {
            let ip_hdr = ip_hdr.as_reply();
            let ip_pkt = ip_hdr.to_vec(&data);

            self.to_device(ef_hdr.dst, ef_hdr.src, ef_hdr.ethertype, ip_pkt);
        }

        Ok(())
    }

    fn route_ip6(&self, _pkt: Vec<u8>) -> Result<(), ProtocolError> {
        tracing::warn!("ipv6 not supported");
        Ok(())
    }

    /// Handles a ICMP echo request to the local device ip
    ///
    /// Responds with an echo reply and queues it in the virtqueues rx ring
    fn handle_icmp_echo_req(&self, pkt: &[u8]) -> Vec<u8> {
        tracing::trace!("handling icmp echo request");
        let mut data = Vec::with_capacity(pkt.len());
        data.push(0 /* ECHO REPLY */);
        data.push(0 /* CODE */);
        data.extend_from_slice(&[0x00, 0x00]);
        data.extend_from_slice(&pkt[4..]);

        let csum = checksum(&data);
        data[2..4].copy_from_slice(&csum.to_be_bytes());
        data
    }

    /// Builds the Ethernet (layer 2) header and queues the packet to
    /// be processed by the virtqueues
    ///
    /// ### Arguments
    /// * `src` - Source MAC address
    /// * `dst` - Destination MAC Address
    /// * `ty` - Type of payload
    /// * `data` - Layer3+ data
    fn to_device(&mut self, src: MacAddress, dst: MacAddress, ty: EtherType, mut data: Vec<u8>) {
        let mut pkt = Vec::with_capacity(12 + data.len());
        pkt.extend_from_slice(dst.as_bytes());
        pkt.extend_from_slice(src.as_bytes());
        pkt.extend_from_slice(&ty.as_u16().to_be_bytes());
        pkt.append(&mut data);

        self.log_packet(&pkt);

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
        self.tx.send(RouterMsg::Packet(port, pkt)).ok();
    }

    /// Connects a new device to the router, returning the port it is connected to
    pub fn connect(&self, queue: TapDeviceRxQueue) -> usize {
        let mut devices = self.devices.lock();
        let idx = devices.len();
        devices.push(queue);

        idx
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
