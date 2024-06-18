use std::path::PathBuf;

use clap::Parser;
use oathgate_bridge::BridgeBuilder;
use tracing::Level;


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

    if let Err(error) = BridgeBuilder::default().pcap(opts.pcap).build(opts.config, "oathgate.sock") {
        tracing::error!(?error, "unable to run oathgate-bridge");
    }
}
