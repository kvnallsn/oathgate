mod config;
mod device;
mod error;
mod queue;
mod types;

use std::path::PathBuf;

use clap::{Args, Parser};
use config::Config;
use device::EventPoller;
use error::AppResult;
use oathgate_net::router::{
    handler::IcmpHandler,
    wan::{TunTap, UdpDevice, Wan, WgDevice},
    Router, Switch,
};
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

    #[command(flatten)]
    pub device: DeviceOpts,
}

#[derive(Args, Clone)]
pub(crate) struct DeviceOpts {
    /// Number of transmit/receive queue pairs to create
    #[arg(long, default_value_t = 1)]
    pub device_queues: u8,
}

fn parse_wan(cfg: WanConfig) -> AppResult<Option<Box<dyn Wan>>> {
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

fn run(opts: Opts) -> AppResult<()> {
    let cfg = Config::load(opts.config)?;
    tracing::debug!(?cfg, "configuration");

    let mut poller = EventPoller::new(opts.socket)?;

    let switch = Switch::new(opts.pcap)?;

    // spawn the default route / upstream
    let wan = parse_wan(cfg.wan)?;

    // spawn thread to receive messages/packets
    let _router = Router::builder()
        .wan(wan)
        .register_proto_handler(IcmpHandler::default())
        .build(cfg.router.ipv4, switch.clone())?;

    poller.run(opts.device, switch)?;

    Ok(())
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

    if let Err(error) = run(opts) {
        tracing::error!(?error, "unable to run oathgate-vost");
    }
}
