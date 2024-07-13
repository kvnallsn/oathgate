//! Virtual Machine commands

mod fabrial;

use std::{
    fs::{self, File},
    path::Path,
};

use anyhow::{anyhow, Context};
use clap::{Args, Subcommand};
use oathgate_runner::{config::Config, hypervisor::Hypervisor};

use crate::{
    database::{image::DiskImage, kernel::Kernel, shard::Shard, Device},
    fork::Forker,
    State,
};

use super::{draw_table, LogFormat};

#[derive(Debug, Subcommand)]
pub enum ShardCommand {
    /// Deploys a new shard using the specified parameters
    Deploy {
        #[command(flatten)]
        opts: DeployOpts,
    },

    /// Runs a new virtual machine attached to an oathgate bridge
    Run {
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
    },
}

#[derive(Args, Debug)]
pub struct DeployOpts {
    /// Name of this shard (or omit to auto-generate)
    #[clap(short, long)]
    pub name: Option<String>,

    /// Backing image to deploy (run `oathgate images ls` to see availabe images)
    #[clap(short, long)]
    pub image: String,

    /// Kernel to use (or omit to use the default)
    #[clap(short, long)]
    pub kernel: Option<String>,

    /// Amount of RAM (memory), in megabytes
    #[clap(short, long, default_value_t = 512)]
    pub memory: u16,

    /// Qemu CPU type
    #[clap(short, long, default_value = "q35")]
    pub cpu: String,

    /// Network/Bridges to connect to this shard
    #[clap(short = 'b', long)]
    pub network: Vec<String>,
}

impl ShardCommand {
    /// Executes this cli command
    pub fn execute(self, state: &State) -> anyhow::Result<()> {
        match self {
            Self::Deploy { opts } => shard_deploy(state, opts)?,
            Self::Run { name } => {
                run_shard(state, name)?;
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

/// Deploys a new shard
///
/// Copies the disk image and kernel to a new location and adds references to the relevant networks
///
/// ### Arguments
/// * `state` - Application state
/// * `opts` - Deploy cli options
fn shard_deploy(state: &State, opts: DeployOpts) -> anyhow::Result<()> {
    let name = opts.name.unwrap_or_else(|| state.generate_name());

    // fetch the image, kernel, and networks
    let image = DiskImage::get(state.db(), &opts.image)?;
    let kernel = match opts.kernel.as_ref() {
        None => Kernel::get_default(state.db())?,
        Some(name) => Kernel::get(state.db(), name)?,
    };
    let devices = Device::get_many(state.db(), &opts.network)?;

    // validate all networks are present

    println!("shard configuration:");
    println!("--> name:    {}", name);
    println!("--> image:   {}", image);
    println!("--> kernel:  {}", kernel);
    println!("--> memory:  {}M", opts.memory);
    println!("--> cpu:     {}", opts.cpu);
    for dev in &devices {
        println!("--> network: {}", dev.name());
    }

    super::confirm(state, "Deploy Shard?")?;

    let bar = super::spinner(format!("deploying shard {name}"));

    let shard = Shard::new(state.ctx(), &name, &opts.cpu, opts.memory);

    let sdir = shard.dir(state).join(&name);
    shard.save(state.db())?;

    copy_file(kernel.path(state), sdir.with_extension("bin"))?;
    copy_file(image.path(state), sdir.with_extension("img"))?;

    bar.finish_with_message(format!("deployed shard {name}"));

    Ok(())
}

fn run_shard(state: &State, name: String) -> anyhow::Result<()> {
    let mut shard = get_shard(state, &name)?;
    let bar = super::spinner(format!("starting shard {name}"));

    let networks = shard.networks(state.db())?;

    let mut stopped = Vec::new();
    let sockets = networks
        .into_iter()
        .filter_map(|net| match net.is_running() {
            true => Some(net.uds(state)),
            false => {
                stopped.push(net.name().to_owned());
                None
            }
        })
        .collect::<Vec<_>>();

    if !stopped.is_empty() {
        return Err(anyhow!(
            "all networks must be running. stopped networks: {}",
            stopped.join(", ")
        ));
    }

    let cfg = Config::from_yaml(shard.config_file_path(state))
        .context("unable to parse configuration file")?;

    let mut hv = Hypervisor::new(&sockets, shard.name(), shard.cid(), cfg.machine)
        .context("unable to create hypervisor")?;

    let logger = state.subscriber(shard.id())?;
    let pid = Forker::with_subscriber(logger)
        .cwd(shard.dir(state))
        .fork(move |_sfd| {
            hv.run()?;
            Ok(())
        })?;

    shard.set_running(pid.as_raw());
    shard.save(state.db())?;

    bar.finish_with_message(format!("started {name} with pid {pid}", pid = 0));

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

/// Copies a file from one path to another
///
/// ### Arguments
/// * `src` - Path to source file on disk
/// * `dst` - Path to destination file on disk
fn copy_file<S: AsRef<Path>, D: AsRef<Path>>(src: S, dst: D) -> anyhow::Result<()> {
    let mut src = File::options().read(true).write(false).open(src)?;

    let mut dst = File::options()
        .read(false)
        .write(true)
        .create(true)
        .truncate(true)
        .open(dst)?;

    std::io::copy(&mut src, &mut dst)?;
    Ok(())
}
