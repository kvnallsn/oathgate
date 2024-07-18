//! Disk Image  Database Model

use std::{fmt::Display, path::PathBuf, str::FromStr};

use anyhow::anyhow;
use rusqlite::{types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, Value, ValueRef}, Row, ToSql};
use uuid::{ClockSequence, Timestamp, Uuid};

use crate::{cmd::AsTable, State};

use super::Database;

/// Macro to build a SQL select query
macro_rules! select {
    () => {
        "SELECT
           id AS image_id,
           hash AS image_hash,
           name AS image_name,
           format AS image_format,
           root AS image_root
        FROM
            images"
    };

    ($query:literal) => {
        concat!(select!(), " ", $query)
    };
}

#[derive(Debug)]
pub struct DiskImage {
    /// Unique id representing a kernel
    pub id: Uuid,

    /// Hex hashed string
    pub hash: String,

    /// Human-friendly name for a kernel
    pub name: String,

    /// Format of the disk image
    pub format: DiskFormat,

    /// Root partition, or None if a raw image
    pub root: Option<u8>,
}

#[derive(Debug)]
pub enum DiskFormat {
    /// A raw (i.e., ext4) disk image with no partitions
    Raw,

    /// A Qemu qcow2 formatted disk image
    Qcow2
}

impl DiskImage {
    /// Creates a new disk image to store in the database
    ///
    /// ### Arguments
    /// * `ctx` - Timestamp context used to generate a UUIDv7
    /// * `name` - Name of this image
    /// * `version` - Version of this image
    pub fn new<S1, S2, C>(ctx: C, hash: S1, name: S2, format: DiskFormat, root: Option<u8>) -> Self
    where
        S1: Into<String>,
        S2: Into<String>,
        C: ClockSequence<Output = u16>,
    {
        let id = Uuid::new_v7(Timestamp::now(ctx));
        let hash = hash.into();
        let name = name.into();

        Self { id, hash, name, format, root }
    }

    /// Retrieves a specific disk image from the database
    ///
    /// ### Arguments
    /// * `db` - Database connection
    /// * `name` - Name of disk image to retrieve
    pub fn get<S: AsRef<str>>(db: &Database, name: S) -> anyhow::Result<Self> {
        let name = name.as_ref();
        let image = db.transaction(|conn| {
            let mut stmt = conn.prepare(select!("WHERE name = ?1"))?;
            let image = stmt
                .query_row([name], Self::from_row)?;

            Ok(image)
        })?;

        Ok(image)
    }

    /// Retrieves all disk image records from the database
    ///
    /// ### Arguments
    /// * `db` - Database reference
    pub fn get_all(db: &Database) -> anyhow::Result<Vec<Self>> {
        let images = db.transaction(|conn| {
            let mut stmt = conn.prepare(select!())?;
            let images = stmt
                .query_map([], Self::from_row)?
                .into_iter()
                .filter_map(|v| v.ok())
                .collect::<Vec<_>>();

            Ok(images)
        })?;

        Ok(images)
    }

    /// Saves this disk image entry in the database, inserting if it does not exist and updating if a
    /// record with the same id already exists
    ///
    /// ### Arguments
    /// * `db` - Database reference
    pub fn save(&self, db: &Database) -> anyhow::Result<()> {
        db.transaction(|conn| {
            conn.execute(
                "
                INSERT INTO images (id, hash, name, format, root) VALUES (?1, ?2, ?3, ?4, ?5)
                ON CONFLICT(id) DO UPDATE SET
                    hash = excluded.hash,
                    name = excluded.name,
                    format = excluded.format,
                    root = excluded.root
            ",
                (&self.id, &self.hash, &self.name, self.format.to_string(), self.root),
            )?;
            Ok(())
        })?;

        Ok(())
    }

    /// Returns the id of this kernel as a hyphenated string
    pub fn id_str(&self) -> String {
        self.id.as_hyphenated().to_string()
    }

    /// Returns the path to this kernel on the host file system
    ///
    /// ### Arguments
    /// * `state` - Application state
    pub fn path(&self, state: &State) -> PathBuf {
        state.image_dir().join(&self.hash).with_extension("img")
    }

    /// Parses a sqlite row to return an instance of this datatype
    ///
    /// ### Arguments
    /// * `row` - SQLite database row
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        let id: Uuid = row.get("image_id")?;
        let hash: String = row.get("image_hash")?;
        let name: String = row.get("image_name")?;
        let format: DiskFormat = row.get("image_format")?;
        let root: Option<u8> = row.get("image_root")?;

        Ok(Self { id, hash, name, format, root })
    }
}

impl AsTable for DiskImage {
    fn header() -> &'static [&'static str] {
        &["Hash", "Name", "Format", "Root Partition"]
    }

    fn update_col_width(&self, widths: &mut [usize]) {
        let f = self.format.to_string();
        let p = self.root.map(|id| id.to_string()).unwrap_or_else(|| String::from("None"));

        widths[0] = std::cmp::max(widths[0], self.hash.len());
        widths[1] = std::cmp::max(widths[1], self.name.len());
        widths[2] = std::cmp::max(widths[2], f.len());
        widths[3] = std::cmp::max(widths[3], p.len());
    }

    fn as_table_row(&self, widths: &[usize]) {
        let p = self.root.map(|id| id.to_string()).unwrap_or_else(|| String::from("None"));

        self.print_field(&self.hash, widths[0]);
        self.print_field(&self.name, widths[1]);
        self.print_field(&self.format, widths[2]);
        self.print_field(&p, widths[3]);
    }
}

impl Display for DiskFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Raw => write!(f, "raw"),
            Self::Qcow2 => write!(f, "qcow2"),
        }
    }
}

impl FromStr for DiskFormat {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_ref() {
            "raw" => Ok(Self::Raw),
            "qcow2" => Ok(Self::Qcow2),
            fmt => Err(anyhow!("unknown disk format: {fmt}")),
        }
    }
}

impl ToSql for DiskFormat {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::Owned(Value::Text(self.to_string())))
    }
}

impl FromSql for DiskFormat {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value {
            ValueRef::Text(txt) => match DiskFormat::from_str(&String::from_utf8_lossy(txt)) {
                Ok(format) => Ok(format),
                Err(err) => Err(FromSqlError::Other(err.into())),
            }
            _ => Err(FromSqlError::InvalidType)
        }
    }
}

impl Display for DiskImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", &self.name, &self.format)
    }
}
