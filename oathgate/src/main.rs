//! Main binary to interact with oathgate system

pub(crate) mod cmd;
pub(crate) mod database;
pub(crate) mod fork;
pub(crate) mod process;

use std::path::PathBuf;

use anyhow::anyhow;
use clap::{Parser, Subcommand};
use cmd::{BridgeCommand, ShardCommand};
use database::Device;
use nix::{errno::Errno, unistd::Pid};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, Layer};
use uuid::NoContext;

use crate::database::DeviceState;

use self::database::Database;

#[derive(Debug, Parser)]
pub struct Opts {
    /// Verbosity of logging (-v, -vv, -vvv)
    #[clap(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Path to base directory to store application files
    #[clap(short, long, default_value = "/tmp/oathgate")]
    pub base: PathBuf,

    /// Oathgate database used to track bridges, vms, etc
    #[clap(short, long, default_value = "oathgate.db")]
    pub database: PathBuf,

    /// Assume yes, don't prompt for confirmation
    #[clap(long, global = true)]
    pub yes_dont_ask_again: bool,

    /// Command to execute
    #[clap(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create, modify, or delete an oathgate bridge
    Bridge {
        #[clap(subcommand)]
        command: BridgeCommand,
    },

    /// Control virtual machines conneted to a bridge
    Shard {
        #[clap(subcommand)]
        command: ShardCommand,
    },
}

pub struct State {
    /// Path to the base / working directory
    base: PathBuf,

    /// Connection to the database
    database: Database,

    /// Set to true to skip confirmation step
    no_confirm: bool,

    /// Context used to generate unique ids
    ctx: NoContext,
}

impl State {
    /// Creates a new state object from the cli options
    ///
    /// ### Arguments
    /// * `opts` - Command line options / flags
    pub fn new(opts: &Opts) -> anyhow::Result<Self> {
        if opts.base.exists() && !opts.base.is_dir() {
            tracing::error!(path = %opts.base.display(), "path specified for base directory is not a directory");
            return Err(anyhow!("invalid base directory"));
        } else if !opts.base.exists() {
            tracing::debug!(path = %opts.base.display(), "base directory does not exist, creating");
            std::fs::create_dir_all(&opts.base)?;
        }

        let db_path = match opts.database.is_relative() {
            true => opts.base.join(&opts.database),
            false => opts.database.clone(),
        };

        let db = Database::open(&db_path)?;

        Ok(Self {
            base: opts.base.clone(),
            database: db,
            no_confirm: opts.yes_dont_ask_again,
            ctx: NoContext::default(),
        })
    }

    /// Returns the full path the log file
    ///
    /// ### Arguments
    /// * `name` - Name of this device
    pub fn log_file(&self, name: &str) -> PathBuf {
        self.base.join(name).with_extension("log")
    }

    /// Returns a reference to a database connection
    pub fn db(&self) -> &Database {
        &self.database
    }

    /// Returns true if we can skip the confirmation step
    pub fn skip_confirm(&self) -> bool {
        self.no_confirm
    }

    /// UUID Timestamp context
    pub fn ctx(&self) -> &NoContext {
        &self.ctx
    }

    pub fn hypervisor_dir(&self) -> PathBuf {
        let hvdir = self.base.join("hypervisor");
        if !hvdir.exists() {
            std::fs::create_dir_all(&hvdir).ok();
        }
        hvdir
    }
}

fn execute(opts: Opts) -> anyhow::Result<()> {
    let state = State::new(&opts)?;

    // get all devices and mark those with dead pids as stale
    let devices = Device::get_all(state.db())?;

    for mut device in devices {
        match nix::sys::signal::kill(Pid::from_raw(device.pid), None) {
            Ok(_) => { /* do nothing, valid pid */ }
            Err(Errno::ESRCH) => {
                tracing::warn!(device = %device.name, pid = %device.pid, "device: process not found");
                device.set_state(DeviceState::Stopped);
                device.save(state.db())?;
            }
            Err(Errno::EPERM) => {
                tracing::warn!(device = %device.name, pid = %device.pid, "device: process permission denied");
            }
            Err(errno) => {
                tracing::warn!(device = %device.name, pid = %device.pid, "unable to check device: {errno}");
            }
        }
    }

    match opts.command {
        Command::Bridge { command } => command.execute(&state)?,
        Command::Shard { command } => command.execute(&state)?,
    }

    Ok(())
}

fn main() -> anyhow::Result<()> {
    let opts = Opts::parse();

    let layer = tracing_subscriber::fmt::Layer::default().with_filter(match opts.verbose {
        0 => tracing_subscriber::filter::LevelFilter::WARN,
        1 => tracing_subscriber::filter::LevelFilter::INFO,
        2 => tracing_subscriber::filter::LevelFilter::DEBUG,
        _ => tracing_subscriber::filter::LevelFilter::TRACE,
    });

    tracing_subscriber::registry().with(layer).init();

    // use with_default here so we can set a global subscriber after forking later
    execute(opts)?;

    Ok(())
}
