mod config;

use std::path::PathBuf;

use clap::Parser;
use config::Config;
use mio::{Events, Interest, Poll, Token};
use oathgate_net::router::{
    handler::IcmpHandler,
    wan::{TunTap, UdpDevice, Wan, WgDevice},
    Router, Switch,
};
use oathgate_vhost::{DeviceOpts, VHostSocket};
use tracing::Level;

use crate::config::WanConfig;

#[derive(Parser)]
pub(crate) struct Opts {
    /// Path to configuration file
    pub config: PathBuf,

    /// Path to the unix socket to communicate with qemu's vhost-user driver
    #[arg(short, long, default_value = "/tmp/oathgate.sock")]
    pub socket: PathBuf,

    /// Path to pcap file, or blank to not capture pcap
    #[arg(short, long)]
    pub pcap: Option<PathBuf>,

    /// Control the level of output to stdout (-v, -vv, -vvv)
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,
}

fn parse_wan(cfg: WanConfig) -> Result<Option<Box<dyn Wan>>, oathgate_vhost::Error> {
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

fn run(opts: Opts, cfg: Config) -> Result<(), oathgate_vhost::Error> {
    const TOKEN_VHOST: Token = Token(0);

    let mut socket = VHostSocket::new(&opts.socket)?;
    let switch = Switch::new(opts.pcap)?;

    // spawn the default route / upstream
    let wan = parse_wan(cfg.wan)?;

    // spawn thread to receive messages/packets
    let _router = Router::builder()
        .wan(wan)
        .register_proto_handler(IcmpHandler::default())
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

fn main() {
    let opts = Opts::parse();

    let level = match opts.verbose {
        0 => Level::WARN,
        1 => Level::INFO,
        2 => Level::DEBUG,
        _ => Level::TRACE,
    };

    tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(level)
        .init();

    let cfg = Config::load(&opts.config).unwrap();
    tracing::debug!(?cfg, "configuration");

    if let Err(error) = run(opts, cfg) {
        tracing::error!(?error, "unable to run oathgate");
    }
}
