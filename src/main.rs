mod config;
mod device;
mod error;
mod queue;
mod router;
mod types;
mod upstream;

use std::path::PathBuf;

use clap::{Args, Parser};
use config::Config;
use device::EventPoller;
use error::AppResult;
use router::Router;
use tracing::Level;

use crate::{
    config::UpstreamConfig,
    upstream::{Tun, UdpDevice},
};

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

fn run(opts: Opts) -> AppResult<()> {
    let cfg = Config::load(opts.config)?;
    tracing::debug!(?cfg, "configuration");

    let mut poller = EventPoller::new(opts.socket)?;

    // spawn thread to receive messages/packets
    let router = Router::builder().pcap(opts.pcap).build(cfg.router.ipv4)?;

    // spawn the default route / upstream
    match cfg.upstream {
        UpstreamConfig::Tap(opts) => {
            let upstream = Tun::create(opts.device)?;
            upstream.spawn(router.clone())?;
        }
        UpstreamConfig::Udp(opts) => {
            let upstream = UdpDevice::connect(opts.endpoint)?;
            upstream.spawn(router.clone())?;
        }
        UpstreamConfig::Wireguard(_opts) => (),
    }

    poller.run(opts.device, router)?;

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
