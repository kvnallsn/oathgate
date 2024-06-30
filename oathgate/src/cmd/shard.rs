//! Virtual Machine commands

mod fabrial;

use anyhow::{anyhow, Context};
use clap::Subcommand;
use oathgate_runner::{config::Config, hypervisor::Hypervisor};

use crate::{
    database::{shard::Shard, Device},
    fork::Forker,
    State,
};

use super::{draw_table, LogFormat};

#[derive(Debug, Subcommand)]
pub enum ShardCommand {
    /// Runs a new virtual machine attached to an oathgate bridge
    Run {
        /// Name of the bridge to connect to the shard
        #[clap(short, long)]
        bridge: String,

        /// Name of the (deployed) virtual machine / shard
        name: String,
    },

    /// Returns a list of all shards, including inactive ones
    #[clap(alias = "status")]
    List,

    /// Print logs for a shard
    Logs {
        /// Shard name
        name: String,

        /// Format to save logs
        #[clap(short, long, default_value_t = LogFormat::Pretty)]
        format: LogFormat,
    },

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

    /// Deletes a stopped shard (unrecoverable!)
    Delete {
        /// Name of the shard to delete
        name: String,
    }
}

impl ShardCommand {
    /// Executes this cli command
    pub fn execute(self, state: &State) -> anyhow::Result<()> {
        match self {
            Self::Run { bridge, name } => {
                run_shard(state, bridge, name)?;
            }
            Self::List => list_shards(state)?,
            Self::Logs { name, format } => print_logs(state, name, format)?,
            Self::Attach { name, port } => attach_shard(state, name, port)?,
            Self::Stop { name } => stop_shard(state, name)?,
            Self::Delete { name } => delete_shard(state, name)?,
        }
        Ok(())
    }
}

fn get_shard(state: &State, name: &str) -> anyhow::Result<Shard> {
    Ok(Shard::get(state.db(), &name)?.ok_or_else(|| anyhow!("shard not found"))?)
}

fn run_shard(state: &State, bridge: String, name: String) -> anyhow::Result<()> {
    let mut shard = get_shard(state, &name)?;
    let bar = super::spinner(format!("starting shard {name}"));

    let bridge =
        Device::get(state.db(), &bridge)?.ok_or_else(|| anyhow!("unknown bridge: {bridge}"))?;

    if !bridge.is_running() {
        return Err(anyhow!(
            "bridge is not running. Start it with `oathgate bridge start {}`",
            bridge.name(),
        ))?;
    }

    let cfg = Config::from_yaml(shard.config_file_path(state))
        .context("unable to parse configuration file")?;
    let mut hv = Hypervisor::new(bridge.uds(state), shard.name(), shard.cid(), cfg.machine)?;

    let logger = state.subscriber(shard.id())?;
    let pid = Forker::with_subscriber(logger)
        .cwd(shard.dir(state))
        .fork(move |_sfd| {
            hv.run()?;
            Ok(())
        })?;

    shard.add_device_ref(state.db(), &bridge)?;
    shard.set_running(pid.as_raw());
    shard.save(state.db())?;

    bar.finish_with_message(format!("started {name} with pid {pid}"));

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
    let mut shard = get_shard(state, &name)?;

    super::confirm(state, "Stop shard?")?;

    let bar = super::spinner(format!("stopping shard {name}"));
    shard.stop()?;
    shard.save(state.db())?;
    bar.finish_with_message("shard stopped");

    Ok(())
}

fn delete_shard(state: &State, name: String) -> anyhow::Result<()> {
    let shard = get_shard(state, &name)?;

    if shard.is_running() {
        return Err(anyhow!("shard is running. stop shard before deleting"));
    }

    super::warning("WARNING: this action will delete ALL shard files and is unrecoverable!");
    super::confirm(state, "Delete shard?")?;

    let bar = super::spinner(format!("deleting shard {name}"));
    shard.purge(state)?;
    bar.finish_with_message("shard deleted");

    Ok(())
}

fn attach_shard(state: &State, name: String, port: u32) -> anyhow::Result<()> {
    let shard = get_shard(state, &name)?;
    fabrial::run(shard.cid(), port)?;
    Ok(())
}

/// Prints logs to the terminal
///
/// ### Arguments
/// * `state` - Application state
/// * `name` - Name of device
/// * `format` - Format to print logs (json, pretty, etc.)
fn print_logs(state: &State, name: String, format: LogFormat) -> anyhow::Result<()> {
    let shard = get_shard(state, &name)?;
    super::print_logs(state, shard.id(), format)?;
    Ok(())
}
