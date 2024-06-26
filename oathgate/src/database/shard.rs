//! Shard / virtual machines

use std::path::PathBuf;

use anyhow::Context;
use rand::RngCore;
use rusqlite::{params, OptionalExtension, Row};
use uuid::{ClockSequence, Timestamp, Uuid};

use crate::{
    cmd::AsTable, process::{self, ProcessState}, State
};

use super::{Database, Device};

/// A `ShardTemplate` references an archive of a shard that can be deployed
#[derive(Debug)]
pub struct ShardTemplate {
    /// Unique id of the shard template
    id: Uuid,

    /// Unique name of the shard template
    name: String,
}

/// A shard is a representation of a VM stored in the database
#[derive(Debug)]
pub struct Shard {
    /// A unique id used internally, should not be exposed
    id: Uuid,

    /// Context id used to communicate using vhost-vsock devices
    cid: u32,

    /// Name of this shard
    name: String,

    /// Current state of the process (not saved in the database)
    state: ProcessState,
}

impl ShardTemplate {
    /// Creates a new shard template from provided parameters
    ///
    /// ### Arguments
    /// * `ctx` - Timestamp context used to generate a UUIDv7
    /// * `name` - Name of this shard template
    pub fn new<S: Into<String>, C: ClockSequence<Output = u16>>(
        ctx: C,
        name: S,
    ) -> Self {
        let id = Uuid::new_v7(Timestamp::now(&ctx));
        let name = name.into();

        Self { id, name }
    }

    /// Returns the shard template with the provided name
    pub fn get<S: AsRef<str>>(db: &Database, name: S) -> anyhow::Result<Self> {
        let name = name.as_ref();

        let template = db.transaction(|conn| {
            let mut stmt = conn.prepare("SELECT id, name FROM shard_templates WHERE name = ?1")?;
            let template = stmt.query_row(params![name], Self::from_row)?;
            Ok(template)
        })?;

        Ok(template)
    }

    /// Retrieves all templates in the system
    ///
    /// ### Arguments
    /// * `db` - Database connection
    pub fn get_all(db: &Database) -> anyhow::Result<Vec<Self>> {
        let templates = db.transaction(|conn| {
            let mut stmt = conn.prepare("SELECT id, name FROM shard_templates")?;
            let templates = stmt
                .query_map(params![], Self::from_row)?
                .filter_map(|r| r.ok())
                .collect::<Vec<_>>();

            Ok(templates)
        })?;

        Ok(templates)
    }

    /// Updates this shard template in the database
    pub fn save(&self, db: &Database) -> anyhow::Result<()> {
        db.transaction(|conn| {
            conn.execute(
                "INSERT INTO
                    shard_templates (id, name)
                 VALUES
                    (?1, ?2)
                 ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name
                ",
                (&self.id, &self.name)
            )?;

            Ok(())
        })?;
        Ok(())
    }

    /// Returns the name of this shard template
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    /// Decodes a SQLite row into a `ShardTemplate`
    ///
    /// ### Arguments
    /// * `row` - Sqlite database row
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        let id: Uuid = row.get(0)?;
        let name: String = row.get(1)?;
        Ok(Self { id, name })
    }
}

impl Shard {
    /// Creates a new shard from the provided parameters
    ///
    /// ### Arguments
    /// * `ctx` - Timestamp context used to generate a UUIDv7
    /// * `cid` - Context Id used with vhost-vsock
    /// * `name` - Name of this shard
    pub fn new<S: Into<String>, C: ClockSequence<Output = u16>>(
        ctx: C,
        name: S,
    ) -> Self {
        let mut rng = rand::thread_rng();
        let id = Uuid::new_v7(Timestamp::now(&ctx));
        let cid = rng.next_u32();

        Self {
            id,
            cid,
            name: name.into(),
            state: ProcessState::Stopped,
        }
    }

    /// Inserts this record into the database
    pub fn save(&self, db: &Database) -> anyhow::Result<()> {
        db.transaction(|conn| {
            conn.execute(
                "INSERT INTO
                    shards (id, name, pid, cid)
                 VALUES
                    (?1, ?2, ?3, ?4)
                 ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name,
                    pid = excluded.pid,
                    cid = excluded.cid
                ",
                (&self.id, &self.name, self.state.optional(), self.cid),
            )?;

            Ok(())
        })
        .context("unable to save state in database")?;

        Ok(())
    }

    /// Associates a device with this shard
    ///
    /// ### Argumnets
    /// * `db` - Database connection
    /// * `dev` - Device to associate with shard
    pub fn add_device_ref(&self, db: &Database, dev: &Device) -> anyhow::Result<()> {
        db.transaction(|conn| {
            conn.execute(
                "INSERT INTO shard_devices (device_id, shard_id) VALUES (?1, ?2) ON CONFLICT(device_id, shard_id) DO NOTHING",
                (dev.id(), &self.id)
            )?;

            Ok(())
        })?;

        Ok(())
    }

    /// Returns the shard with the specificed name, or None if one is not found
    ///
    /// ### Arguments
    /// * `db` - Reference to the database
    /// * `name` - Name of the shard
    pub fn get(db: &Database, name: &str) -> anyhow::Result<Option<Shard>> {
        let shard = db.transaction(|conn| {
            let mut stmt = conn.prepare("SELECT id, name, pid, cid FROM shards WHERE name = ?1")?;

            let shard = stmt.query_row(params![name], Self::from_row).optional()?;

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
            let mut stmt = conn.prepare("SELECT id, name, pid, cid FROM shards")?;

            let shards = stmt
                .query_map(params![], Self::from_row)?
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

    /// Returns this shard's unique identifier
    pub fn id(&self) -> Uuid {
        self.id
    }

    /// Returns the name of this shard
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    /// Returns the context id used with vhost-vsock devices
    pub fn cid(&self) -> u32 {
        self.cid
    }

    /// Returns the current state of this process
    pub fn state(&self) -> ProcessState {
        self.state
    }

    /// Updates the state of this process to running
    ///
    /// ### Arguments
    /// * `pid` - Process identifier of the newly started process
    pub fn set_running(&mut self, pid: i32) {
        self.state = ProcessState::Running(pid)
    }

    /// Updates the state of this process to stopped
    pub fn set_stopped(&mut self) {
        self.state = ProcessState::Stopped;
    }

    /// Returns the path to the shard's directory (based on the base path)
    pub fn dir(&self, state: &State) -> PathBuf {
        state.shard_dir().join(&self.name())
    }

    /// Returns the path to the shard's configuration file
    pub fn config_file_path(&self, state: &State) -> PathBuf {
        self.dir(state).join("config.yml")
    }

    /// Parses a Shard from a sqlite row
    ///
    /// ### Arguments
    /// * `row` - Row returned from database
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        let pid: Option<i32> = row.get(2)?;
        let state = match pid {
            Some(pid) => process::check(pid).unwrap(),
            None => ProcessState::Stopped,
        };

        Ok(Self {
            id: row.get(0)?,
            name: row.get(1)?,
            cid: row.get(3)?,
            state,
        })
    }
}

impl AsTable for ShardTemplate {
    fn header() -> &'static [&'static str] {
        &["Name"]
    }

    fn update_col_width(&self, widths: &mut [usize]) {
        widths[0] = std::cmp::max(widths[0], self.name.len());
    }

    fn as_table_row(&self, widths: &[usize]) {
        self.print_field(&self.name, widths[0]);
    }
}

impl AsTable for Shard {
    fn header() -> &'static [&'static str] {
        &["Name", "State", "Context Id"]
    }

    fn update_col_width(&self, widths: &mut [usize]) {
        let cid = format!("0x{:02x}", self.cid);

        widths[0] = std::cmp::max(widths[0], self.name.len());
        widths[1] = std::cmp::max(widths[1], self.state.to_string().len());
        widths[2] = std::cmp::max(widths[2], cid.to_string().len());
    }

    fn as_table_row(&self, widths: &[usize]) {
        let cid = format!("0x{:02x}", self.cid);

        self.print_field(&self.name, widths[0]);
        self.print_field(&self.state.styled(), widths[1]);
        self.print_field(&cid, widths[2]);
    }
}
