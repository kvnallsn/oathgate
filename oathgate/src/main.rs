//! Main binary to interact with oathgate system

pub(crate) mod cmd;
pub(crate) mod database;
pub(crate) mod fork;
pub(crate) mod logger;
pub(crate) mod process;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use cmd::{BridgeCommand, ShardCommand};
use logger::{LogDestination, SubscriberHandle};
use uuid::{NoContext, Uuid};

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

    /// Handle to modify the tracing subscriber
    handle: SubscriberHandle,
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

        //self.database = self.database.canonicalize().expect("unable to canonicalize database file path");
    }

    /// Returns the maximum log level based on the number of verbose flags
    pub fn log_level(&self) -> tracing::Level {
        match self.verbose {
            0 => tracing::Level::WARN,
            1 => tracing::Level::INFO,
            2 => tracing::Level::DEBUG,
            _ => tracing::Level::TRACE,
        }
    }
}

impl State {
    /// Creates a new state object from the cli options
    ///
    /// ### Arguments
    /// * `opts` - Command line options / flags
    pub fn new(opts: &Opts, handle: SubscriberHandle) -> anyhow::Result<Self> {
        let db = Database::open(&opts.database)?;

        Ok(Self {
            base: opts.base.clone(),
            database: db,
            no_confirm: opts.yes_dont_ask_again,
            ctx: NoContext::default(),
            handle,
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

    /// Returns the tracing subscriber handle
    pub fn tracing_handle(&self) -> SubscriberHandle {
        self.handle.clone()
    }

    pub fn hypervisor_dir(&self) -> PathBuf {
        let hvdir = self.base.join("hypervisor");
        if !hvdir.exists() {
            std::fs::create_dir_all(&hvdir).ok();
        }
        hvdir
    }

    /// Changes the destination of the tracing subscriber to the database
    ///
    /// ### Arguments
    /// * `id` - ID of device we're logging about
    pub fn log_to_database(&self, id: Uuid) {
        self.handle.set_destination(LogDestination::Database(id));
    }

    /// Set the tracing subscriber to write to stdout
    pub fn log_to_stdout(&self) {
        self.handle.set_destination(LogDestination::Stdout);
    }
}

fn main() -> anyhow::Result<()> {
    let mut opts = Opts::parse();
    opts.validate();

    /*
    let filter_layer = tracing_subscriber::fmt::Layer::default().with_filter(match opts.verbose {
        0 => tracing_subscriber::filter::LevelFilter::WARN,
        1 => tracing_subscriber::filter::LevelFilter::INFO,
        2 => tracing_subscriber::filter::LevelFilter::DEBUG,
        _ => tracing_subscriber::filter::LevelFilter::TRACE,
    });

    tracing_subscriber::registry().with(layer).init(); //.with(logger::SqliteLayer).init();
    */


    tracing::info!(?opts, "validated options");

    let execute = || {
        let handle = logger::SqliteSubscriber::builder()
            .with_max_level(opts.log_level())
            .init(&opts.database)?;

        let state = State::new(&opts, handle)?;

        match opts.command {
            Command::Bridge { command } => command.execute(&state)?,
            Command::Shard { command } => command.execute(&state)?,
        }

        Ok::<(), anyhow::Error>(())
    };

    match execute() {
        Ok(_) => (),
        Err(error) => {
            eprintln!("{error}");
        }
    }

    Ok(())
}
