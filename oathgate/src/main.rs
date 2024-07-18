//! Main binary to interact with oathgate system

pub(crate) mod cmd;
pub(crate) mod database;
pub(crate) mod fork;
pub(crate) mod logger;
pub(crate) mod process;

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use cmd::{BridgeCommand, ImageCommand, KernelCommand, ShardCommand};
use console::style;
use logger::SqliteSubscriber;
use parking_lot::Mutex;
use rand::{rngs::ThreadRng, Rng};
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

    /// Assume yes, don't prompt for confirmationimplementation
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

    /// Manage and install Linux kernels
    Kernel {
        #[clap(subcommand)]
        command: KernelCommand,
    },

    /// Manage and install disk images
    Image {
        #[clap(subcommand)]
        command: ImageCommand,
    },

    /// Control virtual machines conneted to a bridge
    Shard {
        #[clap(subcommand)]
        command: ShardCommand,
    },

    /*
    /// Manage shard templates
    Template {
        #[clap(subcommand)]
        command: TemplateCommand,
    },
    */

    /// Print status of entire system
    Status,
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

    /// Random number generator
    rng: Mutex<ThreadRng>,

    /// Random name generator
    name_gen: Mutex<names::Generator<'static>>,

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
            rng: Mutex::new(rand::thread_rng()),
            name_gen: Mutex::new(names::Generator::default()),
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

    /// Generates a new name to identify a device/shard/etc.
    pub fn generate_name(&self) -> String {
        self.name_gen.lock().next().unwrap()
    }

    /// Generates a new UUIDv7
    pub fn generate_id(&self) -> Uuid {
        Uuid::now_v7()
    }

    /// Generates a new number inbetween 20,000 and 30,000 to use as a context id
    pub fn generate_cid(&self) -> u32 {
        self.rng.lock().gen_range(20_000..30_000)
    }

    pub fn get_mime<P: AsRef<Path>>(&self, path: P) -> anyhow::Result<String> {
        use magic::{Cookie, cookie::Flags};

        let cookie = Cookie::open(Flags::ERROR | Flags::MIME_TYPE)?;
        let database = Default::default();
        let cookie = cookie.load(&database).unwrap();
        let mime = cookie.file(path)?;

        Ok(mime)
    }

    /// Returns the path the network directory based on the base path
    ///
    /// The network directory stores the various files (such as unix domain sockets) needed
    /// to provided access to a given network or bridge
    pub fn network_dir(&self) -> PathBuf {
        self.subdir("networks")
    }

    /// Returns the path to the archive directory
    ///
    /// The archive directory contains compressed images of VMs/shards ready to be deployed
    pub fn archive_dir(&self) -> PathBuf {
        self.subdir("archives")
    }

    /// Returns the path to the shard directory
    ///
    /// The shard directory contains the files for active/running shards (vms)
    pub fn shard_dir(&self) -> PathBuf {
        self.subdir("shards")
    }

    /// Returns the path to the kernels directory
    ///
    /// The kernels directory contains the available kernels that can be used to start a shard
    pub fn kernel_dir(&self) -> PathBuf {
        self.subdir("kernels")
    }

    /// Returns the path to the images directory
    ///
    /// The images directory contains the available disk images that can be used to start a shard
    pub fn image_dir(&self) -> PathBuf {
        self.subdir("images")
    }

    /// Returns the path to a specific subdirectory relative to the base path
    ///
    /// ### Arguments
    /// * `name` - Name of subdirectory to open/create
    fn subdir(&self, name: &str) -> PathBuf {
        let dir = self.base.join(name);
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
            Command::Kernel { command } => command.execute(&state)?,
            Command::Image { command } => command.execute(&state)?,
            Command::Shard { command } => command.execute(&state)?,
            //Command::Template { command } => command.execute(&state)?,
            Command::Status => {
                println!("Networks");
                self::cmd::BridgeCommand::List.execute(&state)?;

                println!("Images");
                self::cmd::ImageCommand::List.execute(&state)?;

                println!("Kernels");
                self::cmd::KernelCommand::List.execute(&state)?;

                println!("\nShards");
                self::cmd::ShardCommand::List.execute(&state)?;
            }
        }

        Ok::<(), anyhow::Error>(())
    };

    if let Err(error) = execute() {
        eprintln!("{} {:?}", style("error:").red(), style(error).red());
    }

    Ok(())
}
