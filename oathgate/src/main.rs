//! Main binary to interact with oathgate system

pub(crate) mod cmd;
pub(crate) mod database;
pub(crate) mod fork;
pub(crate) mod logger;
pub(crate) mod process;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use cmd::{BridgeCommand, ShardCommand};
use logger::SqliteSubscriber;
use uuid::{NoContext, Uuid};

use self::database::Database;

#[derive(Debug, Parser)]
pub struct Opts {
    /// Log level verbosity (-v, -vv, -vvv)
    #[clap(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Don't log to stdout
    #[clap(short, long, global = true)]
    pub silent: bool,

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

    /// Maximum level to log at
    max_log_level: tracing::Level,
}

impl Opts {
    /// Creates the base directory if it does not exist
    ///
    /// ### Panics
    /// - If we cannot create the base directory
    /// - If the base directory exists but it is not a directory
    pub fn validate(&mut self) {
        if !self.base.exists() {
            std::fs::create_dir_all(&self.base).expect("unable to create base directory");
        } else if !self.base.is_dir() {
            panic!("base directory is not a directory");
        }

        if self.database.is_relative() {
            self.database = self.base.join(&self.database);
        }
    }
}

impl State {
    /// Creates a new state object from the cli options
    ///
    /// ### Arguments
    /// * `opts` - Command line options / flags
    pub fn new(opts: &Opts) -> anyhow::Result<Self> {
        let db = Database::open(&opts.database)?;

        let max_log_level = match opts.verbose {
            0 => tracing::Level::WARN,
            1 => tracing::Level::INFO,
            2 => tracing::Level::DEBUG,
            _ => tracing::Level::TRACE,
        };

        Ok(Self {
            base: opts.base.clone(),
            database: db,
            no_confirm: opts.yes_dont_ask_again,
            ctx: NoContext::default(),
            max_log_level,
        })
    }

    /// Returns the tracing subscriber that will be installed in child process when forked
    pub fn subscriber(&self, device_id: Uuid) -> anyhow::Result<SqliteSubscriber> {
        SqliteSubscriber::builder()
            .with_max_level(self.max_log_level)
            .with_device_id(device_id)
            .finish(self.database.path())
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

    /// Returns the path the hypervisor's directory based on the base path
    pub fn hypervisor_dir(&self) -> PathBuf {
        let hvdir = self.base.join("hypervisor");
        if !hvdir.exists() {
            std::fs::create_dir_all(&hvdir).ok();
        }
        hvdir
    }

    /// Returns the path the network directory based on the base path
    ///
    /// The network directory stores the various files (such as unix domain sockets) needed
    /// to provided access to a given network or bridge
    pub fn network_dir(&self) -> PathBuf {
        let dir = self.base.join("network");
        if !dir.exists() {
            std::fs::create_dir_all(&dir).ok();
        }
        dir
    }
}

fn main() -> anyhow::Result<()> {
    let mut opts = Opts::parse();
    opts.validate();

    let execute = || {
        let state = State::new(&opts)?;

        match opts.command {
            Command::Bridge { command } => command.execute(&state)?,
            Command::Shard { command } => command.execute(&state)?,
        }

        Ok::<(), anyhow::Error>(())
    };

    execute()?;

    Ok(())
}
