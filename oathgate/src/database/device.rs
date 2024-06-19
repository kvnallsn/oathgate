//! Represents a device in the database

use std::fmt::Display;

use anyhow::Context;
use rusqlite::{
    params,
    types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef},
    OptionalExtension, Row, ToSql,
};
use uuid::{ClockSequence, Timestamp, Uuid};

use crate::cmd::AsTable;

use super::Database;

#[derive(Debug)]
pub struct Device {
    pub id: Uuid,
    pub pid: i32,
    pub name: String,
    pub ty: DeviceType,
    pub state: DeviceState,
}

#[derive(Debug)]
pub enum DeviceType {
    Bridge,
}

#[derive(Debug)]
pub enum DeviceState {
    Created,
    Running,
    Stopped,
}

impl Device {
    /// Returns a string representing the table's schema
    pub fn table() -> &'static str {
        "CREATE TABLE IF NOT EXISTS devices (
            id      BLOB PRIMARY KEY,
            pid     INTEGER NOT NULL,
            name    TEXT NOT NULL,
            device  INTEGER NOT NULL,
            state   INTEGER NOT NULL
        )"
    }
    /// Creates a new device from the provided parameters
    ///
    /// ### Arguments
    /// * `ctx` - Timestamp context used to generate a UUIDv7
    /// * `pid` - Process Id of running process
    /// * `name` - Name of this device
    /// * `ty` - Type of device
    pub fn new<S: Into<String>, C: ClockSequence<Output = u16>>(
        ctx: C,
        pid: i32,
        name: S,
        ty: DeviceType,
    ) -> Self {
        let id = Uuid::new_v7(Timestamp::now(&ctx));

        Self {
            id,
            pid,
            name: name.into(),
            ty,
            state: DeviceState::Running,
        }
    }

    /// Inserts this record into the database
    pub fn save(&self, db: &Database) -> anyhow::Result<()> {
        db.transaction(|conn| {
            conn.execute(
                "INSERT INTO
                    devices (id, pid, name, device, state)
                 VALUES
                    (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(id) DO UPDATE SET
                    pid = excluded.pid,
                    name = excluded.name,
                    device = excluded.device,
                    state = excluded.state
                ",
                (&self.id, self.pid, &self.name, &self.ty, &self.state),
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
                conn.prepare("SELECT id, pid, name, device, state FROM devices where name = ?1")?;

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
            let mut stmt = conn.prepare("SELECT id, pid, name, device, state FROM devices")?;

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

    /// Updates the state on this device
    ///
    /// ### Arguments
    /// * `state` - New value of the state field
    pub fn set_state(&mut self, state: DeviceState) {
        self.state = state;
    }

    /// Returns true if this device is running (defined by having a valid pid)
    pub fn is_running(&self) -> bool {
        matches!(self.state, DeviceState::Running)
    }

    /// Parses a Device from a sqlite row
    ///
    /// ### Arguments
    /// * `row` - Row returned from database
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Device {
            id: row.get(0)?,
            pid: row.get(1)?,
            name: row.get(2)?,
            ty: row.get(3)?,
            state: row.get(4)?,
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

impl Display for DeviceState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = match self {
            Self::Created => "created",
            Self::Running => "running",
            Self::Stopped => "stopped",
        };

        write!(f, "{state}")
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

impl FromSql for DeviceState {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value {
            ValueRef::Integer(i) => match i {
                0 => Ok(DeviceState::Created),
                1 => Ok(DeviceState::Running),
                2 => Ok(DeviceState::Stopped),
                _ => Err(FromSqlError::Other(
                    "invalid number for device state".into(),
                )),
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

impl ToSql for DeviceState {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        match self {
            Self::Created => Ok(ToSqlOutput::Owned(rusqlite::types::Value::Integer(0))),
            Self::Running => Ok(ToSqlOutput::Owned(rusqlite::types::Value::Integer(1))),
            Self::Stopped => Ok(ToSqlOutput::Owned(rusqlite::types::Value::Integer(2))),
        }
    }
}

impl AsTable for Device {
    fn header() -> &'static [&'static str] {
        &["Name", "PID", "Type", "State"]
    }

    fn update_col_width(&self, widths: &mut [usize]) {
        widths[0] = std::cmp::max(widths[0], self.name.len());
        widths[1] = std::cmp::max(widths[1], self.pid.to_string().len());
        widths[2] = std::cmp::max(widths[2], self.ty.to_string().len());
        widths[3] = std::cmp::max(widths[3], self.state.to_string().len());
    }

    fn as_table_row(&self, widths: &[usize]) {
        print!(" {:width$} |", self.name, width = widths[0]);
        print!(" {:width$} |", self.pid, width = widths[1]);
        print!(" {:width$} |", self.ty, width = widths[2]);
        print!(" {:width$} |", self.state, width = widths[3]);
    }
}
