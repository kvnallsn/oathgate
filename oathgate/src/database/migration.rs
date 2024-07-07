//! Migration routines and functions

use rusqlite::Connection;

/// Runs the initial migration
pub fn version_000(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS logs (
            id      BLOB PRIMARY KEY,
            device  BLOB,
            level   TEXT NOT NULL,
            target  TEXT NOT NULL,
            ts      TEXT NOT NULL,
            module  TEXT,
            line    INTEGER,
            data    JSON
        );

        CREATE TABLE IF NOT EXISTS devices (
            id      BLOB PRIMARY KEY,
            pid     INTEGER,
            name    TEXT NOT NULL,
            device  INTEGER NOT NULL,
            config  JSON NOT NULL
        );

        CREATE TABLE IF NOT EXISTS kernels (
            id      BLOB PRIMARY KEY,
            hash    TEXT NOT NULL UNIQUE,
            name    TEXT NOT NULL,
            version TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS images (
            id      BLOB PRIMARY KEY,
            hash    TEXT NOT NULL UNIQUE,
            name    TEXT NOT NULL,
            format  TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS shard_template_disks (
            id      BLOB PRIMARY KEY,
            name    TEXT NOT NULL,
            format  TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS shard_templates (
            id      BLOB PRIMARY KEY,
            name    TEXT NOT NULL,
            machine TEXT NOT NULL,
            memory  INTEGER NOT NULL,
            kargs   TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS shards (
            id      BLOB PRIMARY KEY,
            name    TEXT NOT NULL,
            pid     INTEGER,
            cid     INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS shard_devices (
            device_id   BLOB REFERENCES devices(id),
            shard_id    BLOB REFERENCES shards(id),
            interface   TEXT NOT NULL,
            PRIMARY KEY (device_id, shard_id)
        );
    "#,
    )?;

    Ok(())
}
