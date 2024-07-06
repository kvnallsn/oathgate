//! Kernel Database Model

use rusqlite::Row;
use sha3::digest::Output;
use uuid::{ClockSequence, Timestamp, Uuid};

use crate::cmd::AsTable;

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
}

impl Kernel {
    /// Creates a new kernel to store in the database
    ///
    /// ### Arguments
    /// * `ctx` - Timestamp context used to generate a UUIDv7
    /// * `name` - Name of this kernel
    /// * `version` - Version of this kernel
    pub fn new<S1, S2, S3, C>(ctx: C, hash: S1, name: S2, version: S3) -> Self 
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

        Self { id, hash, name, version }
    }

    /// Retrieves all kernel records from the database
    ///
    /// ### Arguments
    /// * `db` - Database reference
    pub fn get_all(db: &Database) -> anyhow::Result<Vec<Self>> {
        let kernels = db.transaction(|conn| {
            let mut stmt = conn.prepare("SELECT id, hash, name, version FROM kernels")?;
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
                INSERT INTO kernels (id, hash, name, version) VALUES (?1, ?2, ?3, ?4)
                ON CONFLICT(id) DO UPDATE SET
                    hash = excluded.hash,
                    name = excluded.name,
                    version = excluded.version
            ", (&self.id, &self.hash, &self.name, &self.version))?;
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
        let version: String = row.get(3)?;

        Ok(Self { id, hash, name, version })
    }
}

impl AsTable for Kernel {
    fn header() -> &'static [&'static str] {
        &["Hash", "Name", "Version"]
    }

    fn update_col_width(&self, widths: &mut [usize]) {
        widths[0] = std::cmp::max(widths[0], self.hash.len());
        widths[1] = std::cmp::max(widths[1], self.name.len());
        widths[2] = std::cmp::max(widths[2], self.version.len());
    }

    fn as_table_row(&self, widths: &[usize]) {
        self.print_field(&self.hash, widths[0]);
        self.print_field(&self.name, widths[1]);
        self.print_field(&self.version, widths[2]);
    }
}
