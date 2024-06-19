//! Shard / virtual machines

use anyhow::Context;
use rusqlite::{params, OptionalExtension, Row};
use uuid::{ClockSequence, Timestamp, Uuid};

use crate::cmd::AsTable;

use super::Database;

/// A shared is a representation of a VM stored in the database
#[derive(Debug)]
pub struct Shard {
    id: Uuid,
    pid: i32,
    name: String,
}

impl Shard {
    /// Returns a string used to create the table in the database
    pub fn table() -> &'static str {
        r#"
            CREATE TABLE IF NOT EXISTS shards (
                id      TEXT PRIMARY KEY,
                name    TEXT NOT NULL,
                pid     INTEGER NOT NULL,
                state   INTEGER NOT NULL
            )
        "#
    }

    /// Creates a new device from the provided parameters
    ///
    /// ### Arguments
    /// * `ctx` - Timestamp context used to generate a UUIDv7
    /// * `pid` - Process Id of running process
    /// * `name` - Name of this device
    /// * `ty` - Type of device
    pub fn new<S: Into<String>, C: ClockSequence<Output = u16>>(ctx: C, pid: i32, name: S) -> Self {
        let id = Uuid::new_v7(Timestamp::now(&ctx));

        Self {
            id,
            pid,
            name: name.into(),
        }
    }

    /// Inserts this record into the database
    pub fn save(&self, db: &Database) -> anyhow::Result<()> {
        db.transaction(|conn| {
            conn.execute(
                "INSERT INTO
                    shards (id, name, pid, state)
                 VALUES
                    (?1, ?2, ?3, ?4)
                 ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name,
                    pid = excluded.pid,
                    state = excluded.state
                ",
                (&self.id, &self.name, &self.pid, 0),
            )?;

            Ok(())
        })
        .context("unable to save state in database")?;

        Ok(())
    }

    /// Returns the shard with the specificed name, or None if one is not found
    ///
    /// ### Arguments
    /// * `db` - Reference to the database
    /// * `name` - Name of the shard
    pub fn get(db: &Database, name: &str) -> anyhow::Result<Option<Shard>> {
        let shard = db.transaction(|conn| {
            let mut stmt = 
                conn
                .prepare("SELECT id, name, pid FROM shards WHERE name = ?1")?;

            let shard = stmt
                .query_row(params![name], Self::from_row).optional()?;

            Ok(shard)
        })?;

        Ok(shard)
    }

    /// Returns all shards in the database
    ///
    /// ### Arguments
    /// * `db` - Reference to the database
    pub fn get_all(db: &Database) -> anyhow::Result<Vec<Shard>> {
        let shards = db.transaction(|conn| {
            let mut stmt = 
                conn
                .prepare("SELECT id, name, pid FROM shards")?;

            let shards = stmt
                .query_map(params![], Self::from_row)?
                .inspect(|f| tracing::trace!(row = ?f, "row result"))
                .filter_map(|dev| dev.ok())
                .collect::<Vec<_>>();

            Ok(shards)
        })?;

        Ok(shards)
    }

    /// Deletes this shard from the database
    ///
    /// ### Arguments
    /// * `db` - Reference to the database
    pub fn delete(&self, db: &Database) -> anyhow::Result<()> {
        db.transaction(|conn| {
            conn.execute("DELETE FROM shards WHERE id = ?1", (&self.id,))?;
            Ok(())
        })?;
        Ok(())
    }

    /// Returns the name of this shard
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    /// Returns the pid of the process running this shard
    pub fn pid(&self) -> i32 {
        self.pid
    }

    /// Parses a Shard from a sqlite row
    ///
    /// ### Arguments
    /// * `row` - Row returned from database
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self{
            id: row.get(0)?,
            name: row.get(1)?,
            pid: row.get(2)?,
        })
    }
}

impl AsTable for Shard {
    fn header() -> &'static [&'static str] {
        &["Name", "PID"]
    }

    fn update_col_width(&self, widths: &mut [usize]) {
        widths[0] = std::cmp::max(widths[0], self.name.len());
        widths[1] = std::cmp::max(widths[1], self.pid.to_string().len());
    }

    fn as_table_row(&self, widths: &[usize]) {
        print!(" {:width$} |", self.name, width = widths[0]);
        print!(" {:width$} |", self.pid, width = widths[1]);
    }
}
