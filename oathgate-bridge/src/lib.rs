mod config;
mod error;
mod net;

use std::path::{Path, PathBuf};

use config::Config;
use mio::{
    Events, Interest, Poll, Token,
};
use oathgate_vhost::{DeviceOpts, VHostSocket};

use crate::{
    config::WanConfig,
    error::Error,
    net::{
        dhcp::DhcpServer,
        router::{
            handler::{IcmpHandler, UdpHandler},
            Router,
        },
        switch::VirtioSwitch,
        wan::{TunTap, UdpDevice, Wan, WgDevice},
    },
};

#[derive(Default)]
pub struct BridgeBuilder {
    /// Path to pcap file, or None to disable pcap
    pcap: Option<PathBuf>,
}

impl BridgeBuilder {
    /// Configures bridge to log all traffic transiting the switch
    ///
    /// ### Arguments
    /// * `pcap` - Path to location to save pcap file, or None to disable
    pub fn pcap(mut self, pcap: Option<PathBuf>) -> Self {
        self.pcap = pcap;
        self
    }

    pub fn build<P: AsRef<Path>, S: Into<String>>(self, cfg: P, name: S) -> Result<(), Error> {
        let name = name.into();
        let socket = format!("/tmp/oathgate/{name}.sock");

        let cfg = Config::load(cfg)?;
        tracing::debug!(?cfg, "bridge configuration");

        if let Err(error) = run(socket, self.pcap, cfg) {
            tracing::error!(?error, "unable to run oathgate");
        }

        Ok(())
    }
}

fn parse_wan(cfg: WanConfig) -> Result<Option<Box<dyn Wan>>, Error> {
    match cfg {
        WanConfig::Tap(opts) => {
            let wan = TunTap::create_tap(opts.device)?;
            Ok(Some(Box::new(wan)))
        }
        WanConfig::Udp(opts) => {
            let wan = UdpDevice::connect(opts.endpoint)?;
            Ok(Some(Box::new(wan)))
        }
        WanConfig::Wireguard(opts) => {
            let wan = WgDevice::create(opts)?;
            Ok(Some(Box::new(wan)))
        }
    }
}

fn run<P1: Into<PathBuf>, P2: Into<PathBuf>>(
    socket: P1,
    pcap: Option<P2>,
    cfg: Config,
) -> Result<(), Error> {
    const TOKEN_VHOST: Token = Token(0);
    let socket = socket.into();
    let pcap = pcap.map(|p| p.into());

    tracing::info!("creating oathgate bridge at {}", socket.display());

    let mut socket = VHostSocket::new(socket)?;
    let switch = VirtioSwitch::new(pcap)?;

    // spawn the default route / upstream
    let wan = parse_wan(cfg.wan)?;

    let mut udp_handler = UdpHandler::default();
    udp_handler.register_port_handler(DhcpServer::new(cfg.router.ipv4, cfg.router.dhcp));

    // spawn thread to receive messages/packets
    let _router = Router::builder()
        .wan(wan)
        .register_proto_handler(IcmpHandler::default())
        .register_proto_handler(udp_handler)
        .spawn(cfg.router.ipv4, switch.clone())?;

    let mut poller = Poll::new()?;
    poller
        .registry()
        .register(&mut socket, TOKEN_VHOST, Interest::READABLE)?;


    let mut events = Events::with_capacity(10);
    loop {
        poller.poll(&mut events, None)?;

        for event in &events {
            match event.token() {
                TOKEN_VHOST => {
                    if let Err(error) =
                        socket.accept_and_spawn(DeviceOpts::default(), switch.clone())
                    {
                        tracing::error!(?error, "unable to accet connection");
                    }
                }
                Token(token) => tracing::debug!(%token, "[main] unknown mio token"),
            }
        }
    }
}

