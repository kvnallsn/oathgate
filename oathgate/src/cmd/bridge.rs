//! Bridge commands and structures

use std::path::PathBuf;

use anyhow::{anyhow, Context};
use clap::Subcommand;
use dialoguer::Confirm;
use oathgate_bridge::{BridgeBuilder, BridgeConfig};

use crate::{
    database::{Device, DeviceType},
    fork::Forker,
    logger::LogLevel,
    process::{self, ProcessState}, State,
};

use super::LogFormat;

#[derive(Debug, Subcommand)]
pub enum BridgeCommand {
    /// Creates a new oathgate bridge
    Create {
        /// Path to bridge configuration file
        config: PathBuf,

        /// Name to identify this bridge, or omitted to auto-generate
        #[clap(short, long)]
        name: Option<String>,
    },

    /// Starts a bridge, spawning a new process/daemon
    Start {
        /// Path to pcap file, or omit to disable pcap
        #[clap(short, long)]
        pcap: Option<PathBuf>,

        /// Log spawned process
        #[clap(long, default_value_t = LogLevel::Info)]
        log_level: LogLevel,

        /// Name of bridge to start
        name: String,
    },

    /// Returns a list of all bridges, including inactive ones
    #[clap(alias = "status")]
    List,

    /// Prints logs for a specific bridge
    Logs {
        /// Name of bridge to print logs
        name: String,

        /// Format to save logs
        #[clap(short, long, default_value_t = LogFormat::Pretty)]
        format: LogFormat,
    },

    /// Stops an existing oathgate bridge
    Stop {
        /// Name of bridge to stop
        name: String,
    },

    /// Delete an existing oathgate bridge
    Delete {
        /// Name of bridge to delete
        name: String,
    },
}

impl BridgeCommand {
    /// Executes the command contained in this instance of the enum
    pub fn execute(self, state: &State) -> anyhow::Result<()> {
        let res = match self {
            Self::Create { config, name } => create_bridge(state, config, name),
            Self::Start { pcap, name, .. } => start_bridge(state, name, pcap),
            Self::List => list_bridges(state),
            Self::Logs { name, format } => print_logs(state, name, format),
            Self::Stop { name } => stop_bridge(state, name),
            Self::Delete { name } => delete_bridge(state, name),
        };

        res.context("failed to execute bridge command")?;

        Ok(())
    }
}

/// Returns the bridge with the specified name. Returns an error if a bridge with the specified
/// name is not found.
///
/// ### Arguments
/// * `state` - Application state
/// * `name` - Name of bridge to get / find
fn get_bridge(state: &State, name: &str) -> anyhow::Result<Device> {
    let device =
        Device::get(state.db(), &name)?.ok_or_else(|| anyhow!("device '{name}' not found"))?;
    Ok(device)
}

/// Creates a new bridge
///
/// ### Arguments
/// * `state` - Application state
/// * `config` - Path to bridge configuration file
/// * `name` - Name of bridge (or None to generate one)
/// * `start` - Starts the bridge after it is created
fn create_bridge(state: &State, config: PathBuf, name: Option<String>) -> anyhow::Result<()> {
    let name = name
        .or_else(|| {
            let mut names = names::Generator::default();
            names.next()
        })
        .ok_or_else(|| anyhow!("unable to generate name for device, please provide one"))?;

    let cfg = BridgeConfig::load(&config).context("failed to parse bridge config")?;
    let device = Device::new(state.ctx(), &name, DeviceType::Bridge, &cfg);
    device.save(state.db()).context("failed to insert bridge into database")?;

    Ok(())
}

/// Starts running an oathgate bridge, spawning a new process to handle the traffic
///
/// ### Arguments
/// * `state` - Application state
/// * `config` - Path to bridge configuration file
/// * `name` - Name of bridge (or None to generate one)
/// * `pcap` - Path to file to save pcap (or None to disable pcap)
fn start_bridge(state: &State, name: String, pcap: Option<PathBuf>) -> anyhow::Result<()> {
    let mut device = get_bridge(state, &name)?;

    let config: BridgeConfig = device.config()?;

    let bridge = BridgeBuilder::default()
        .pcap(pcap)
        .base(state.network_dir())
        .build(config, &name)?;

    let logger = state.subscriber(device.id())?;
    let pid = Forker::with_subscriber(logger).fork(move |sfd| {
        bridge.run(sfd)?;
        Ok(())
    })?;

    device.set_started(pid.as_raw());
    device
        .save(state.db())
        .context("unable to save device in database")?;

    Ok(())
}

/// Creates a new bridge
///
/// ### Arguments
/// * `state` - Application state
/// * `name` - Name of bridge to stop
fn stop_bridge(state: &State, name: String) -> anyhow::Result<()> {
    let mut device = get_bridge(state, &name)?;

    match device.status() {
        ProcessState::Running(pid) => match process::stop(state, pid, "Stop device")? {
            true => {
                device.set_stopped();
                device.save(state.db())?;
                println!("stopped device");
            }
            false => println!("operation cancelled"),
        }
        _ => (),
    }

    Ok(())
}

fn list_bridges(state: &State) -> anyhow::Result<()> {
    let devices = Device::get_all(state.db())?;
    match devices.is_empty() {
        true => println!("no bridges found!"),
        false => super::draw_table(&devices),
    }
    Ok(())
}

/// Prints logs to the terminal
///
/// ### Arguments
/// * `state` - Application state
/// * `name` - Name of device
/// * `format` - Format to print logs (json, pretty, etc.)
fn print_logs(state: &State, name: String, format: LogFormat) -> anyhow::Result<()> {
    let device = get_bridge(state, &name)?;
    super::print_logs(state, device.id(), format)?;
    Ok(())
}

/// Deletes a bridge, stopping it if it is running
///
/// ### Arguments
/// * `state` - Application state
/// * `name` - Name of bridge to delete
fn delete_bridge(state: &State, name: String) -> anyhow::Result<()> {
    let mut device = get_bridge(state, &name)?;

    match device.status() {
        ProcessState::Running(pid) => match process::stop(state, pid, "Stop device?")? {
            true => {
                device.set_stopped();
                device.save(state.db())?;
                println!("stopped device");
            }
            false => {
                println!("operation cancelled");
                return Ok(());
            }
        },
        _ => (),
    }

    let confirmation =
        state.skip_confirm() || Confirm::new().with_prompt("Delete device?").interact()?;

    if confirmation {
        device.delete(state.db())?;
        println!("delete device");
    } else {
        println!("operation cancelled");
    }

    Ok(())
}
