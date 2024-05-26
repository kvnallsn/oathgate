//! Configuration file module

use std::{
    fs::File,
    io,
    net::{Ipv4Addr, SocketAddr},
    path::Path,
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    pub wan: WanConfig,
    pub router: RouterConfig,
    pub virtio: VirtioConfig,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum WanConfig {
    Tap(TapConfig),
    Udp(UdpConfig),
    Wireguard(WgConfig),
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WgConfig {
    pub key: String,
    pub ipv4: Ipv4Addr,
    pub peer: String,
    pub endpoint: SocketAddr,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TapConfig {
    pub device: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct UdpConfig {
    pub endpoint: SocketAddr,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RouterConfig {
    pub ipv4: Ipv4Addr,
    pub dhcp: bool,
    pub dns: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct VirtioConfig {
    pub queues: u8,
}

impl Config {
    /// Loads a configuration file from disk
    ///
    /// ### Arguments
    /// * `path` - Path to the configuration file
    pub fn load<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let f = File::open(path)?;
        let cfg: Config =
            serde_yaml::from_reader(f).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        Ok(cfg)
    }
}
