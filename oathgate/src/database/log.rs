//! Represents a device in the database

use std::{borrow::Cow, collections::BTreeMap};

use rusqlite::params;
use serde_json::json;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::logger::{DataType, LogLevel, OathgateEvent};

use super::Database;

#[derive(Debug)]
pub struct LogEntry;

impl LogEntry {
    /// Returns a string representing the table's schema
    pub fn table() -> &'static str {
        "CREATE TABLE IF NOT EXISTS logs (
            id      BLOB PRIMARY KEY,
            device  BLOB,
            level   TEXT NOT NULL,
            target  TEXT NOT NULL,
            ts      TEXT NOT NULL,
            module  TEXT,
            line    INTEGER,
            data    JSON

        )"
    }

    pub fn save(db: &Database, device: Option<Uuid>, event: &OathgateEvent) -> anyhow::Result<()> {
        let id = Uuid::new_v4();
        let level = event.level.to_string();
        let data = json!(&event.data);

        db.transaction(move |conn| {
            conn.execute(
                "INSERT INTO logs (id, device, level, target, ts, module, line, data) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                (&id, &device, &level, &event.target, &event.ts, &event.module, event.line, data),
            )?;
            Ok(())
        })?;

        Ok(())
    }

    pub fn get(db: &Database, device: Uuid) -> anyhow::Result<Vec<OathgateEvent>> {
        let logs = db.transaction(|conn| {
            let mut stmt = conn.prepare(
                "SELECT level, target, ts, module, line, data FROM logs WHERE device = ?1",
            )?;

            let res = stmt
                .query_map(params![device], |row| {
                    let level: String = row.get(0)?;
                    let target: String = row.get(1)?;
                    let ts: OffsetDateTime = row.get(2)?;
                    let module: Option<String> = row.get(3)?;
                    let line: Option<u32> = row.get(4)?;
                    let data: serde_json::Value = row.get(5)?;

                    let level: LogLevel = level.parse().unwrap();
                    let data: BTreeMap<Cow<'_, str>, DataType<'_>> =
                        serde_json::from_value(data).unwrap();

                    Ok(OathgateEvent {
                        level,
                        target: Cow::Owned(target),
                        ts,
                        module: module.map(|m| Cow::Owned(m)),
                        line,
                        data,
                    })
                })?
                .filter_map(|r| r.ok())
                .collect::<Vec<_>>();

            Ok(res)
        })?;

        Ok(logs)
    }
}
