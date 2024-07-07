//! Disk Image  Database Model

use std::{fmt::Display, str::FromStr};

use anyhow::anyhow;
use rusqlite::{types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, Value, ValueRef}, Row, ToSql};
use uuid::{ClockSequence, Timestamp, Uuid};

use crate::cmd::AsTable;

use super::Database;

pub struct DiskImage {
    /// Unique id representing a kernel
    pub id: Uuid,

    /// Hex hashed string
    pub hash: String,

    /// Human-friendly name for a kernel
    pub name: String,

    /// Format of the disk image
    pub format: DiskFormat,
}

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
    pub fn new<S1, S2, C>(ctx: C, hash: S1, name: S2, format: DiskFormat) -> Self
    where
        S1: Into<String>,
        S2: Into<String>,
        C: ClockSequence<Output = u16>,
    {
        let id = Uuid::new_v7(Timestamp::now(ctx));
        let hash = hash.into();
        let name = name.into();

        Self { id, hash, name, format }
    }

    /// Retrieves all disk image records from the database
    ///
    /// ### Arguments
    /// * `db` - Database reference
    pub fn get_all(db: &Database) -> anyhow::Result<Vec<Self>> {
        let images = db.transaction(|conn| {
            let mut stmt = conn.prepare("SELECT id, hash, name, format FROM images")?;
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
                INSERT INTO images (id, hash, name, format) VALUES (?1, ?2, ?3, ?4)
                ON CONFLICT(id) DO UPDATE SET
                    hash = excluded.hash,
                    name = excluded.name,
                    format = excluded.format
            ",
                (&self.id, &self.hash, &self.name, self.format.to_string()),
            )?;
            Ok(())
        })?;

        Ok(())
    }

    /// Returns the id of this kernel as a hyphenated string
    pub fn id_str(&self) -> String {
        self.id.as_hyphenated().to_string()
    }

    /// Parses a sqlite row to return an instance of this datatype
    ///
    /// ### Arguments
    /// * `row` - SQLite database row
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        let id: Uuid = row.get(0)?;
        let hash: String = row.get(1)?;
        let name: String = row.get(2)?;
        let format: DiskFormat = row.get(3)?;

        Ok(Self { id, hash, name, format })
    }
}

impl AsTable for DiskImage {
    fn header() -> &'static [&'static str] {
        &["Hash", "Name", "Format"]
    }

    fn update_col_width(&self, widths: &mut [usize]) {
        let f = self.format.to_string();

        widths[0] = std::cmp::max(widths[0], self.hash.len());
        widths[1] = std::cmp::max(widths[1], self.name.len());
        widths[2] = std::cmp::max(widths[2], f.len());
    }

    fn as_table_row(&self, widths: &[usize]) {
        self.print_field(&self.hash, widths[0]);
        self.print_field(&self.name, widths[1]);
        self.print_field(&self.format, widths[2]);
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
