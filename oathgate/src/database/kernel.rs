//! Kernel Database Model

use std::{fmt::Display, path::PathBuf};

use rusqlite::Row;
use uuid::{ClockSequence, Timestamp, Uuid};

use crate::{cmd::AsTable, State};

use super::Database;

pub struct Kernel {
    /// Unique id representing a kernel
    pub id: Uuid,

    /// Hex hashed string
    pub hash: String,

    /// Human-friendly name for a kernel
    pub name: String,

    /// Version (i.e., 6.9.1) for a kernel
    pub version: String,

    /// Set to true if this is the default kernel to use
    pub default: bool,
}

impl Kernel {
    /// Creates a new kernel to store in the database
    ///
    /// ### Arguments
    /// * `ctx` - Timestamp context used to generate a UUIDv7
    /// * `name` - Name of this kernel
    /// * `version` - Version of this kernel
/// * `default` - True if this is the default kernel to use then deploying shards
    pub fn new<S1, S2, S3, C>(ctx: C, hash: S1, name: S2, version: S3, default: bool) -> Self 
    where
        S1: Into<String>,
        S2: Into<String>,
        S3: Into<String>,
        C: ClockSequence<Output = u16>
    {
        let id = Uuid::new_v7(Timestamp::now(ctx));
        let hash = hash.into();
        let name = name.into();
        let version = version.into();

        Self { id, hash, name, version, default }
    }

    /// Retrieves a specific kernel from the database
    ///
    /// ### Arguments
    /// * `db` - Database connection
    /// * `name` - Name of the kernel to fetch from the database
    pub fn get<S: AsRef<str>>(db: &Database, name: S) -> anyhow::Result<Self> {
        let name = name.as_ref();
        let kernel = db.transaction(|conn| {
            let mut stmt = conn.prepare("SELECT id, hash, name, version, is_default FROM kernels WHERE name = ?1")?;
            let kernel = stmt
                .query_row([name], Self::from_row)?;

            Ok(kernel)
        })?;

        Ok(kernel)
    }

    /// Retrieves the default kernel from the database
    ///
    ///
    /// ### Arguments
    /// * `db` - Database connection
    pub fn get_default(db: &Database) -> anyhow::Result<Self> {
        let kernel = db.transaction(|conn| {
            let mut stmt = conn.prepare("SELECT id, hash, name, version, is_default FROM kernels WHERE is_default = 1")?;
            let kernel = stmt
                .query_row([], Self::from_row)?;

            Ok(kernel)
        })?;

        Ok(kernel)
    }

    /// Retrieves all kernel records from the database
    ///
    /// ### Arguments
    /// * `db` - Database reference
    pub fn get_all(db: &Database) -> anyhow::Result<Vec<Self>> {
        let kernels = db.transaction(|conn| {
            let mut stmt = conn.prepare("SELECT id, hash, name, version, is_default FROM kernels")?;
            let kernels = stmt
                .query_map([], Self::from_row)?
                .into_iter()
                .filter_map(|k| k.ok())
                .collect::<Vec<_>>();

            Ok(kernels)
        })?;

        Ok(kernels)
    }

    /// Saves this kernel entry in the database, inserting if it does not exist and updating if a
    /// record with the same id already exists
    ///
    /// ### Arguments
    /// * `db` - Database reference
    pub fn save(&self, db: &Database) -> anyhow::Result<()> {
        db.transaction(|conn| {
            conn.execute("
                INSERT INTO kernels (id, hash, name, version, is_default) VALUES (?1, ?2, ?3, ?4, ?5)
                ON CONFLICT(id) DO UPDATE SET
                    hash = excluded.hash,
                    name = excluded.name,
                    version = excluded.version,
                    is_default = excluded.is_default
            ", (&self.id, &self.hash, &self.name, &self.version, self.default))?;
            Ok(())
        })?;

        Ok(())
    }

    /// Returns the id of this kernel as a hyphenated string
    pub fn id_str(&self) -> String {
        self.id.as_hyphenated().to_string()
    }

    /// Sets this kernel as the default kernel to use when starting shards
    ///
    /// NOTE: Only one kernel can be set as the default. This is enforced by a SQL trigger
    pub fn set_default(&mut self) {
        self.default = true;
    }

    /// Returns the path to this kernel on disk
    ///
    /// ### Arguments
    /// * `state` - Application state
    pub fn path(&self, state: &State) -> PathBuf {
        state.kernel_dir().join(&self.hash).with_extension("bin")
    }

    /// Parses a sqlite row to return an instance of this datatype
    ///
    /// ### Arguments
    /// * `row` - SQLite database row
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        let id: Uuid = row.get(0)?;
        let hash: String = row.get(1)?;
        let name: String = row.get(2)?;
        let version: String = row.get(3)?;
        let default: bool = row.get(4)?;

        Ok(Self { id, hash, name, version, default })
    }
}

impl AsTable for Kernel {
    fn header() -> &'static [&'static str] {
        &["Hash", "Name", "Version", "Default"]
    }

    fn update_col_width(&self, widths: &mut [usize]) {
        let default = self.default.to_string();

        widths[0] = std::cmp::max(widths[0], self.hash.len());
        widths[1] = std::cmp::max(widths[1], self.name.len());
        widths[2] = std::cmp::max(widths[2], self.version.len());
        widths[3] = std::cmp::max(widths[3], default.len());
    }

    fn as_table_row(&self, widths: &[usize]) {
        self.print_field(&self.hash, widths[0]);
        self.print_field(&self.name, widths[1]);
        self.print_field(&self.version, widths[2]);
        self.print_field(self.default, widths[3]);
    }
}

impl Display for Kernel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", &self.name, &self.version)
    }
}
