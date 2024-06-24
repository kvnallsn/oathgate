//! Virtual Machine commands

mod fabrial;

use std::path::PathBuf;

use anyhow::anyhow;
use clap::Subcommand;
use oathgate_runner::{config::Config, hypervisor::Hypervisor};

use crate::{
    database::{shard::Shard, Device},
    fork::Forker,
    process, State,
};

use super::draw_table;

#[derive(Debug, Subcommand)]
pub enum ShardCommand {
    Create {
        /// Path to configuration file
        config: PathBuf,
    },
    /// Runs a new virtual machine attached to an oathgate bridge
    Run {
        /// Name of the bridge to connect to the shard
        #[clap(short, long)]
        bridge: String,

        /// Name of this virtual machine / shard
        #[clap(short, long)]
        name: Option<String>,

        /// Path to the configuration file for a virtual machine
        #[clap(short, long)]
        config: PathBuf,
    },

    /// Returns a list of all shards, including inactive ones
    List,

    /// Name of shard to attach a pty/tty
    Attach {
        name: String,

        /// Port to connect on the vsock socket
        #[clap(short, long, default_value = "3715")]
        port: u32,
    },

    /// Stop a running shard
    Stop {
        /// Name of the shard to stop
        name: String,
    },
}

impl ShardCommand {
    /// Executes this cli command
    pub fn execute(self, state: &State) -> anyhow::Result<()> {
        match self {
            Self::Create { config } => (),
            Self::Run {
                bridge,
                name,
                config,
            } => {
                run_shard(state, bridge, name, config)?;
            }
            Self::List => list_shards(state)?,
            Self::Attach { name, port } => attach_shard(state, name, port)?,
            Self::Stop { name } => stop_shard(state, name)?,
        }
        Ok(())
    }
}

fn run_shard(
    state: &State,
    bridge: String,
    name: Option<String>,
    config: PathBuf,
) -> anyhow::Result<()> {
    let mut names = names::Generator::default();

    let name = name
        .or_else(|| names.next())
        .ok_or_else(|| anyhow!("unable to generate name for shard"))?;

    let bridge =
        Device::get(state.db(), &bridge)?.ok_or_else(|| anyhow!("unknown bridge: {bridge}"))?;

    let cfg = Config::from_yaml(config)?;
    let mut hv = Hypervisor::new(bridge.uds(state), name, cfg.machine)?;

    let logger = state.subscriber(bridge.id())?;
    let name = hv.name().to_owned();
    let cid = hv.cid();
    let pid = Forker::with_subscriber(logger).fork(move || {
        hv.run()?;
        Ok(())
    })?;

    let shard = Shard::new(state.ctx(), pid.as_raw(), cid, name.clone());
    shard.save(state.db())?;

    println!("spawned shard {name} (pid = {pid})");

    Ok(())
}

fn list_shards(state: &State) -> anyhow::Result<()> {
    let shards = Shard::get_all(state.db())?;
    match shards.is_empty() {
        true => println!("no shards found!"),
        false => draw_table(&shards),
    }
    Ok(())
}

fn stop_shard(state: &State, name: String) -> anyhow::Result<()> {
    match Shard::get(state.db(), &name)? {
        None => println!("shard '{name}' not found!"),
        Some(shard) => match process::stop(state, shard.pid(), "Stop shard?")? {
            true => {
                shard.delete(state.db())?;
                println!("shard stopped");
            }
            false => println!("operation cancelled"),
        },
    }
    Ok(())
}

fn attach_shard(state: &State, name: String, port: u32) -> anyhow::Result<()> {
    match Shard::get(state.db(), &name)? {
        None => println!("shard '{name}' not found!"),
        Some(shard) => fabrial::run(shard.cid(), port)?,
    }
    Ok(())
}
