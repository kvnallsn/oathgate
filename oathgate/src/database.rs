//! Database related functions

pub(crate) mod device;
pub(crate) mod log;
pub(crate) mod shard;

use std::{path::{Path, PathBuf}, sync::Arc};

use anyhow::Context;
use parking_lot::Mutex;
use rusqlite::Connection;

use crate::database::{log::LogEntry, shard::Shard};

pub use self::device::{Device, DeviceType};

/// Provides access to the database used to track bridges, etc.
///
/// The database provides a stateful way of monitoring pids associated with various components of
/// the oathgate ecosystem.  When devices (such as bridges) are created, an entry is made into the
/// database containing the necessary metadata (pids, etc.) to control and interact with each
/// device.
pub struct Database {
    conn: Arc<Mutex<Connection>>,
    path: PathBuf,
}

impl Database {
    /// Opens a database for reading/writing
    ///
    /// ### Arguments
    /// * `path` - Location on file system for database
    pub fn open<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let path = path.as_ref();

        let conn = Connection::open(path)?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
            path: path.to_path_buf(),
        };

        db.transaction(|conn| {
            conn.execute(LogEntry::table(), ())
                .context("unable to create log table")?;

            conn.execute(Device::table(), ())
                .context("unable to create device table")?;

            conn.execute(Shard::table(), ())
                .context("unable to create shard table")?;

            Ok(())
        })?;

        Ok(db)
    }

    /// Starts a new transaction within the database
    pub fn transaction<F, T>(&self, f: F) -> anyhow::Result<T>
    where
        F: FnOnce(&Connection) -> anyhow::Result<T>,
    {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        match f(&tx) {
            Ok(val) => {
                tx.commit()?;
                Ok(val)
            }
            Err(err) => {
                tx.rollback()?;
                Err(err)
            }
        }
    }

    /// Returns the path to this database on disk
    pub fn path(&self) -> &Path {
        self.path.as_path()
    }
}
