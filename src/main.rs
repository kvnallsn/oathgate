mod device;
mod error;
mod queue;
mod router;
mod types;

use std::{net::Ipv4Addr, path::PathBuf};

use anyhow::Result;
use clap::{Args, Parser};
use device::EventPoller;
use router::Router;
use tracing::Level;

#[derive(Parser)]
pub(crate) struct Opts {
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

fn run(opts: Opts) -> Result<()> {
    let mut poller = EventPoller::new(opts.socket)?;

    let ip4 = Ipv4Addr::from([10, 10, 10, 1]);
    let router = Router::new(ip4, opts.pcap);

    // spawn thread to receive messages/packets
    let router = router.start()?;

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
