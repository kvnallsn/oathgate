//! Virtual Machine commands

use std::path::PathBuf;

use anyhow::anyhow;
use clap::Subcommand;
use oathgate_runner::{config::Config, hypervisor::Hypervisor};

use crate::{database::shard::Shard, fork::Forker, State};

use super::draw_table;

#[derive(Debug, Subcommand)]
pub enum ShardCommand {
    /// Runs a new virtual machine attached to an oathgate bridge
    Run {
        /// Path to the  network socket (for a vhost-user network)
        #[clap(short, long)]
        bridge: PathBuf,

        /// Name of this virtual machine / shard
        #[clap(short, long)]
        name: Option<String>,

        /// Path to the configuration file for a virtual machine
        #[clap(short, long)]
        config: PathBuf,
    },

    /// Returns a list of all shards, including inactive ones
    List,

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
            Self::Run {
                bridge,
                name,
                config,
            } => {
                run_shard(state, bridge, name, config)?;
            }
            Self::List => list_shards(state)?,
            Self::Stop { name } => stop_shard(state, name)?,
        }
        Ok(())
    }
}

fn run_shard(
    state: &State,
    bridge: PathBuf,
    name: Option<String>,
    config: PathBuf,
) -> anyhow::Result<()> {
    let mut names = names::Generator::default();

    let name = name
        .or_else(|| names.next())
        .ok_or_else(|| anyhow!("unable to generate name for shard"))?;

    let cfg = Config::from_yaml(config)?;
    let mut hv = Hypervisor::new(bridge, name, cfg.machine)?;

    let log = state.hypervisor_dir().join("vm.log");

    let name = hv.name().to_owned();
    let pid = Forker::default().stdout(log).fork(move || {
        hv.run()?;
        Ok(())
    })?;

    let shard = Shard::new(state.ctx(), pid.as_raw(), name.clone());
    shard.save(state.db())?;

    println!("spawned vm {name} (pid = {pid})");

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
        Some(shard) => {
            tracing::debug!(?shard, "stopping shard");
            match super::stop_process(state, shard.pid(), "Stop shard?")? {
                true => {
                    shard.delete(state.db())?;
                    println!("shard stopped");
                }
                false => println!("operation cancelled"),
            }
        }
    }
    Ok(())
}
