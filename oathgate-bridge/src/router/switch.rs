//! Simple network switch

use std::{
    borrow::Cow, collections::HashMap, fs::File, path::PathBuf, sync::Arc, time::UNIX_EPOCH,
};

use flume::Sender;
use parking_lot::RwLock;
use pcap_file::pcap::{PcapPacket, PcapWriter};

use oathgate_net::{types::MacAddress, EthernetFrame, ProtocolError, Switch, SwitchPort};

use super::{RouterError, ETHERNET_HDR_SZ};

#[derive(Clone, Default)]
pub struct VirtioSwitch {
    /// Handles to devices connected to switch ports
    ports: Arc<RwLock<Vec<Box<dyn SwitchPort>>>>,

    /// Map of MacAddress to switch ports
    cache: Arc<RwLock<HashMap<MacAddress, usize>>>,

    /// Pcap logger, if configured
    logger: PcapLogger,
}

#[derive(Clone, Debug, Default)]
struct PcapLogger {
    tx: Option<Sender<Vec<u8>>>,
}

impl VirtioSwitch {
    /// Creates a new, empty switch with no ports connected
    pub fn new(pcap: Option<PathBuf>) -> Result<Self, RouterError> {
        let logger = PcapLogger::new(pcap)?;
        Ok(Self {
            logger,
            ..Default::default()
        })
    }

    fn associate_port(&self, port: usize, mac: MacAddress) {
        let mut cache = self.cache.write();

        // associate MAC address of source with port
        match cache.insert(mac, port) {
            Some(old_port) if port == old_port => { /* do nothing, no port change */ }
            Some(old_port) => {
                tracing::trace!(
                    port,
                    old_port,
                    "[switch] associating mac ({}) with new port",
                    mac
                )
            }
            None => tracing::trace!("[switch] associating mac ({}) with port {}", mac, port),
        }
    }

    fn get_port(&self, mac: MacAddress) -> Option<usize> {
        let cache = self.cache.read();
        cache.get(&mac).map(|port| *port)
    }
}

impl Switch for VirtioSwitch {
    /// Connects a new device to the router, returning the port it is connected to
    ///
    /// ### Arguments
    /// * `port` - Device to connect to this switch
    fn connect<P: SwitchPort + 'static>(&self, port: P) -> usize {
        let mut ports = self.ports.write();
        let idx = ports.len();
        ports.push(Box::new(port));

        idx
    }

    /// Processes a packet through the switch, sending it to the desired port
    /// or flooding it to all ports if the mac is not known
    ///
    /// ### Arguments
    /// * `port` - Port id this packet was sent from
    /// * `pkt` - Ethernet Framed packet (Layer 2)
    fn process(&self, port: usize, mut pkt: Vec<u8>) -> Result<(), ProtocolError> {
        if pkt.len() < ETHERNET_HDR_SZ {
            return Err(ProtocolError::NotEnoughData(pkt.len(), ETHERNET_HDR_SZ));
        }

        self.logger.log_packet(&pkt);

        let frame = EthernetFrame::extract(&mut pkt)?;

        // update our cached mac address / port cache mapping if needed for the source port
        match self.get_port(frame.src) {
            Some(p) if p == port => { /* do nothing, no need to update cache */ }
            Some(_) | None => self.associate_port(port, frame.src),
        }

        // write packet to destination port
        let ports = self.ports.read();
        if frame.dst.is_broadcast() {
            // write to all ports (but originator)
            tracing::trace!(?frame, "[switch] got broadcast message");
            for (_, dev) in ports.iter().enumerate().filter(|(idx, _)| *idx != port) {
                dev.enqueue(frame, pkt.clone());
            }
        } else {
            match self.get_port(frame.dst) {
                Some(port) => match ports.get(port) {
                    Some(dev) => dev.enqueue(frame, pkt),
                    None => tracing::warn!(port, "[switch] device not connected to port!"),
                },
                None => tracing::warn!("[switch] mac ({}) not associated with port", frame.dst),
            }
        }

        Ok(())
    }
}

impl PcapLogger {
    pub fn new(path: Option<PathBuf>) -> Result<Self, RouterError> {
        match path {
            Some(path) => {
                let tx = Self::spawn(path)?;
                Ok(Self { tx: Some(tx) })
            }
            None => Ok(Self { tx: None }),
        }
    }

    fn spawn(path: PathBuf) -> Result<Sender<Vec<u8>>, RouterError> {
        let file = File::options().create(true).write(true).open(path)?;

        let mut writer = PcapWriter::new(file)?;
        let (tx, rx) = flume::unbounded::<Vec<u8>>();

        std::thread::Builder::new()
            .name(String::from("pcap-logger"))
            .spawn(move || {
                while let Ok(pkt) = rx.recv() {
                    writer
                        .write_packet(&PcapPacket {
                            timestamp: UNIX_EPOCH.elapsed().unwrap(),
                            orig_len: pkt.len() as u32,
                            data: Cow::Borrowed(&pkt),
                        })
                        .ok();
                }
            })?;

        Ok(tx)
    }

    fn log_packet(&self, pkt: &[u8]) {
        if let Some(ref tx) = self.tx {
            tx.send(pkt.to_vec()).ok();
        }
    }
}
