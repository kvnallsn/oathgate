use std::{fs::File, io, path::PathBuf};

use clap::Parser;
use oathgate_runner::{config::Config, hypervisor::Hypervisor, tui, Error};
use tracing::Level;
use tracing_subscriber::fmt::writer::{BoxMakeWriter, Tee};

#[derive(Parser)]
pub struct Opts {
    /// Path to configuration file
    config: PathBuf,

    /// Path to the network's unix socket (for a vhost-user network)
    #[clap(short, long)]
    network: PathBuf,

    /// Run in background / as daemon
    #[clap(short, long)]
    daemon: bool,

    /// Verbosity (-v, -vv, -vvv)
    #[clap(short, long, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Location on disk to save log output
    #[clap(short, long, default_value = "oathgate.log")]
    logfile: PathBuf,
}

fn get_log_writer(opts: &Opts) -> Result<BoxMakeWriter, Error> {
    let file = File::options()
        .write(true)
        .append(true)
        .create(true)
        .open(&opts.logfile)?;

    match opts.daemon {
        false => Ok(BoxMakeWriter::new(file)),
        true => Ok(BoxMakeWriter::new(Tee::new(file, io::stderr))),
    }
}
fn main() -> Result<(), Error> {
    let opts = Opts::parse();

    tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(Level::DEBUG)
        .with_writer(get_log_writer(&opts)?)
        .init();

    let fd = File::open(&opts.config)?;
    let cfg: Config = serde_yaml::from_reader(fd)?;

    let mut hypervisor = Hypervisor::new(&opts.network, cfg.machine)?;

    if !opts.daemon {
        tui::run(hypervisor)?;
    } else {
        hypervisor.run()?;
    }

    Ok(())
}
