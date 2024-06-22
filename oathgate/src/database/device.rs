//! Represents a device in the database

use std::{fmt::Display, path::PathBuf};

use anyhow::Context;
use console::style;
use rusqlite::{
    params,
    types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef},
    OptionalExtension, Row, ToSql,
};
use serde::{de::DeserializeOwned, Serialize};
use uuid::{ClockSequence, Timestamp, Uuid};

use crate::{cmd::AsTable, process::{self, ProcessState}, State};

use super::Database;

#[derive(Debug)]
pub struct Device {
    id: Uuid,
    pid: Option<i32>,
    name: String,
    ty: DeviceType,
    cfg: serde_json::Value,
    state: ProcessState,
}

#[derive(Debug)]
pub enum DeviceType {
    Bridge,
}

impl Device {
    /// Returns a string representing the table's schema
    pub fn table() -> &'static str {
        "CREATE TABLE IF NOT EXISTS devices (
            id      BLOB PRIMARY KEY,
            pid     INTEGER,
            name    TEXT NOT NULL,
            device  INTEGER NOT NULL,
            config  JSON NOT NULL
        )"
    }
    /// Creates a new device from the provided parameters
    ///
    /// ### Arguments
    /// * `ctx` - Timestamp context used to generate a UUIDv7
    /// * `pid` - Process Id of running process
    /// * `name` - Name of this device
    /// * `ty` - Type of device
    pub fn new<S: Into<String>, C: ClockSequence<Output = u16>, V: Serialize>(
        ctx: C,
        name: S,
        ty: DeviceType,
        config: &V,
    ) -> Self {
        let id = Uuid::new_v7(Timestamp::now(&ctx));
        let cfg = serde_json::to_value(config).unwrap();

        Self {
            id,
            pid: None,
            name: name.into(),
            ty,
            cfg,
            state: ProcessState::Stopped
        }
    }

    /// Inserts this record into the database
    pub fn save(&self, db: &Database) -> anyhow::Result<()> {
        db.transaction(|conn| {
            conn.execute(
                "INSERT INTO
                    devices (id, pid, name, device, config)
                 VALUES
                    (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(id) DO UPDATE SET
                    pid = excluded.pid,
                    name = excluded.name,
                    device = excluded.device,
                    config = excluded.config
                ",
                (&self.id, self.pid, &self.name, &self.ty, &self.cfg),
            )?;

            Ok(())
        })
        .context("unable to save device in database")?;

        Ok(())
    }

    /// Loads the device with the specificed name from the database
    ///
    /// ### Arguments
    /// * `db` - Reference to the database
    /// * `name` - Name of device to load / retrieve
    pub fn get<S: AsRef<str>>(db: &Database, name: S) -> anyhow::Result<Option<Device>> {
        let name = name.as_ref();

        let device = db.transaction(|conn| {
            let mut stmt =
                conn.prepare("SELECT id, pid, name, device, config FROM devices where name = ?1")?;

            let device = stmt.query_row(params![name], Self::from_row).optional()?;

            Ok(device)
        })?;

        Ok(device)
    }

    /// Gets all the devices from the database
    ///
    /// ### Arguments
    /// * `db` - Reference to the database
    pub fn get_all(db: &Database) -> anyhow::Result<Vec<Device>> {
        let devices = db.transaction(|conn| {
            let mut stmt = conn.prepare("SELECT id, pid, name, device, config FROM devices")?;

            let devices = stmt
                .query_map(params![], Self::from_row)?
                .filter_map(|dev| dev.ok())
                .collect::<Vec<_>>();

            Ok(devices)
        })?;

        Ok(devices)
    }

    /// Deletes this device from the database
    ///
    /// ### Arguments
    /// * `db` - Reference to the database
    pub fn delete(&self, db: &Database) -> anyhow::Result<()> {
        db.transaction(|conn| {
            conn.execute("DELETE FROM devices WHERE id = ?1", (&self.id,))?;
            Ok(())
        })?;
        Ok(())
    }

    /// Returns the unique id of this device
    pub fn id(&self) -> Uuid {
        self.id
    }

    /// Sets the pid associated with this device
    pub fn set_pid(&mut self, pid: i32) {
        self.pid = Some(pid);
    }

    /// Clears the pid associated with this device
    pub fn clear_pid(&mut self) {
        self.pid = None;
    }


    /// Returns the pid associated with the process, if it is running
    pub fn pid(&self) -> Option<i32> {
        self.pid
    }

    /// Returns the path to the unix domain socket connected to this process
    pub fn uds(&self, state: &State) -> PathBuf {
        state.base.join(&self.name).with_extension("sock")
    }

    /// Returns the configuration object stored in this device entry
    pub fn config<D: DeserializeOwned>(&self) -> anyhow::Result<D> {
        Ok(serde_json::from_value(self.cfg.clone())?)
    }

    /// Parses a Device from a sqlite row
    ///
    /// ### Arguments
    /// * `row` - Row returned from database
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        let pid: Option<i32> = row.get(1)?;
        let state = match pid {
            Some(pid) => process::check(pid).unwrap_or(ProcessState::Dead),
            None => ProcessState::Stopped,
        };
        let cfg: serde_json::Value = row.get(4)?;

        Ok(Device {
            id: row.get(0)?,
            pid,
            name: row.get(2)?,
            ty: row.get(3)?,
            cfg,
            state
        })
    }
}

impl Display for DeviceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Bridge => "bridge",
        };

        write!(f, "{name}")
    }
}

impl FromSql for DeviceType {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value {
            ValueRef::Integer(i) => match i {
                0 => Ok(DeviceType::Bridge),
                _ => Err(FromSqlError::Other("invalid number for device type".into())),
            },
            _ => Err(FromSqlError::InvalidType),
        }
    }
}

impl ToSql for DeviceType {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        match self {
            Self::Bridge => Ok(ToSqlOutput::Owned(rusqlite::types::Value::Integer(0))),
        }
    }
}

impl AsTable for Device {
    fn header() -> &'static [&'static str] {
        &["Name", "PID", "Type", "State"]
    }

    fn update_col_width(&self, widths: &mut [usize]) {
        widths[0] = std::cmp::max(widths[0], self.name.len());
        widths[1] = match self.pid.as_ref() {
            Some(pid) => std::cmp::max(widths[1], pid.to_string().len()),
            None => "None".len(),
        };
        widths[2] = std::cmp::max(widths[2], self.ty.to_string().len());
        widths[3] = std::cmp::max(widths[3], self.state.to_string().len());
    }

    fn as_table_row(&self, widths: &[usize]) {
        self.print_field(&self.name, widths[0]);

        match self.pid.as_ref() {
            Some(pid) => self.print_field(pid, widths[1]),
            None => self.print_field(style("None").dim(), widths[1]),
        }

        self.print_field(&self.ty, widths[2]);
        self.print_field(self.state.styled(), widths[3]);
    }
}
