mod config;
mod error;
mod net;

use std::{os::fd::AsRawFd, path::PathBuf};

use mio::{unix::SourceFd, Events, Interest, Poll, Token};
use nix::sys::{
    signal::Signal,
    signalfd::{SfdFlags, SigSet, SignalFd},
};
use oathgate_vhost::{DeviceOpts, VHostSocket};

pub use self::config::Config as BridgeConfig;

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

pub struct Bridge {
    socket_path: PathBuf,
    pcap: Option<PathBuf>,
    cfg: BridgeConfig,
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

    pub fn build<S: Into<String>>(self, cfg: BridgeConfig, name: S) -> Result<Bridge, Error> {
        let name = name.into();
        let socket = format!("/tmp/oathgate/{name}.sock");

        tracing::debug!(?cfg, "bridge configuration");

        Ok(Bridge {
            socket_path: socket.into(),
            pcap: self.pcap,
            cfg,
        })
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

impl Bridge {
    pub fn run(self) -> Result<(), Error> {
        const TOKEN_VHOST: Token = Token(0);
        const TOKEN_SIGNAL: Token = Token(1);

        tracing::debug!(socket = %self.socket_path.display(), "bridge starting");

        let mut socket = VHostSocket::new(&self.socket_path)?;
        let switch = VirtioSwitch::new(self.pcap)?;

        // spawn the default route / upstream
        let wan = parse_wan(self.cfg.wan)?;

        let mut udp_handler = UdpHandler::default();
        udp_handler
            .register_port_handler(DhcpServer::new(self.cfg.router.ipv4, self.cfg.router.dhcp));

        // spawn thread to receive messages/packets
        let _router = Router::builder()
            .wan(wan)
            .register_proto_handler(IcmpHandler::default())
            .register_proto_handler(udp_handler)
            .spawn(self.cfg.router.ipv4, switch.clone())?;

        let mut sigmask = SigSet::empty();
        sigmask.add(Signal::SIGTERM);
        sigmask.thread_block()?;

        let sfd = SignalFd::with_flags(&sigmask, SfdFlags::SFD_NONBLOCK)?;

        let mut poller = Poll::new()?;
        poller
            .registry()
            .register(&mut socket, TOKEN_VHOST, Interest::READABLE)?;

        poller.registry().register(
            &mut SourceFd(&sfd.as_raw_fd()),
            TOKEN_SIGNAL,
            Interest::READABLE,
        )?;

        tracing::info!(socket = %self.socket_path.display(), "bridge started");
        let mut events = Events::with_capacity(10);
        'poll: loop {
            poller.poll(&mut events, None)?;

            for event in &events {
                match event.token() {
                    TOKEN_VHOST => {
                        if let Err(error) =
                            socket.accept_and_spawn(DeviceOpts::default(), switch.clone())
                        {
                            tracing::error!(%error, "unable to accet connection");
                        }
                    }
                    TOKEN_SIGNAL => match sfd.read_signal() {
                        Ok(None) => { /* no nothing, no signal read */ }
                        Ok(Some(sig)) => match sig.ssi_signo {
                            15 /* SIGTERM */ => break 'poll,
                            signo => tracing::warn!(%signo, "unhandled signal"),
                        },
                        Err(error) => {
                            tracing::error!(%error, "unable to read signal");
                        }
                    },
                    Token(token) => tracing::debug!(%token, "[main] unknown mio token"),
                }
            }
        }

        tracing::info!(socket = %self.socket_path.display(), "bridge stopped");

        Ok(())
    }
}
