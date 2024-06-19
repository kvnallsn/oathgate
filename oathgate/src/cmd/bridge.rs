//! Bridge commands and structures

use std::path::PathBuf;

use anyhow::{anyhow, Context};
use clap::Subcommand;
use dialoguer::Confirm;
use oathgate_bridge::BridgeBuilder;

use crate::{database::{Device, DeviceType}, fork::Forker, State, process};

#[derive(Debug, Subcommand)]
pub enum BridgeCommand {
    /// Creates a new oathgate bridge
    Create {
        /// Path to bridge configuration file
        config: PathBuf,

        /// Path to pcap file, or omit to disable pcap
        #[clap(short, long)]
        pcap: Option<PathBuf>,

        /// Name to identify this bridge, or omitted to generate one
        #[clap(short, long)]
        name: Option<String>,
    },

    /// Returns a list of all bridges, including inactive ones
    List,

    /// Prints logs for a specific bridge
    Logs {
        /// Name of bridge to print logs
        name: String,
    },

    /// Stops an existing oathgate bridge
    Stop {
        /// Name of bridge to stop
        name: String,
    },
}

impl BridgeCommand {
    /// Executes the command contained in this instance of the enum
    pub fn execute(self, state: &State) -> anyhow::Result<()> {
        match self {
            Self::Create { config, pcap, name } => create_bridge(state, config, name, pcap)?,
            Self::List => list_bridges(state)?,
            Self::Logs { name } => print_logs(state, name)?,
            Self::Stop { name } => stop_bridge(state, name)?,
        }

        Ok(())
    }
}

/// Creates a new bridge
///
/// ### Arguments
/// * `state` - Application state
/// * `config` - Path to bridge configuration file
/// * `name` - Name of bridge (or None to generate one)
/// * `pcap` - Path to file to save pcap (or None to disable pcap)
fn create_bridge(
    state: &State,
    config: PathBuf,
    name: Option<String>,
    pcap: Option<PathBuf>,
) -> anyhow::Result<()> {
    let name = name
        .or_else(|| {
            let mut names = names::Generator::default();
            names.next()
        })
        .ok_or_else(|| anyhow!("unable to generate name for device, please provide one"))?;

    tracing::info!(
        name,
        "creating new bridge from configuration file '{}'",
        config.display()
    );

    let stdout = state.base.join(&name).with_extension("log");

    let bridge = BridgeBuilder::default().pcap(pcap).build(config, &name)?;
    let pid = Forker::default().stdout(stdout).fork(move || {
        bridge.run()?;
        Ok(())
    })?;

    let device = Device::new(state.ctx(), pid.as_raw(), &name, DeviceType::Bridge);
    device.save(state.db()).context("unable to save device in database")?;


    tracing::debug!(
        name,
        "inserted device into bridge with id '{}'",
        device.id.as_hyphenated()
    );

    Ok(())
}

/// Creates a new bridge
///
/// ### Arguments
/// * `state` - Application state
/// * `name` - Name of bridge to stop 
fn stop_bridge(state: &State, name: String) -> anyhow::Result<()> {
    // get device
    let device = Device::get(state.db(), &name)?.ok_or_else(|| anyhow!("device '{name}' not found"))?;
    tracing::trace!(?device, "found device");

    if device.is_running() {
        match process::stop(state, device.pid, "Delete device?")? {
            true => {
                device.delete(state.db())?;
                println!("deleted device");
            }
            false => println!("operation cancelled"),
        }
    } else {
        println!(
            "found device but associated process ({}) is missing",
            device.pid
        );

        let confirmation =
            state.skip_confirm() || Confirm::new().with_prompt("Delete device?").interact()?;

        if confirmation {
            device.delete(state.db())?;
            println!("device stopped");
        }
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

fn print_logs(state: &State, name: String) -> anyhow::Result<()> {
    match Device::get(state.db(), &name)? {
        None => println!("bridge not found"),
        Some(dev) => (),
    }
    Ok(())
}
