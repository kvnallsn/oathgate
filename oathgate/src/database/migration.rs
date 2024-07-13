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
            id          BLOB PRIMARY KEY,
            hash        TEXT NOT NULL UNIQUE,
            name        TEXT NOT NULL,
            version     TEXT NOT NULL,
            is_default  INTEGER DEFAULT 0 CHECK(is_default IN (0, 1))
        );

        CREATE TABLE IF NOT EXISTS images (
            id      BLOB PRIMARY KEY,
            hash    TEXT NOT NULL UNIQUE,
            name    TEXT NOT NULL,
            format  TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS shards (
            id      BLOB PRIMARY KEY,
            name    TEXT NOT NULL,
            pid     INTEGER,
            cid     INTEGER NOT NULL,
            cpu     TEXT NOT NULL,
            memory  INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS shard_devices (
            device_id   BLOB REFERENCES devices(id),
            shard_id    BLOB REFERENCES shards(id),
            interface   TEXT NOT NULL,
            PRIMARY KEY (device_id, shard_id)
        );

        CREATE TRIGGER IF NOT EXISTS enforce_kernel_default_insert
        BEFORE INSERT ON kernels
        FOR EACH ROW
        WHEN NEW.is_default = 1
        BEGIN
            UPDATE kernels SET is_default = 0 WHERE is_default = 1;
        END;

        CREATE TRIGGER IF NOT EXISTS enforce_kernel_default_update
        BEFORE UPDATE OF is_default ON kernels
        FOR EACH ROW
        WHEN NEW.is_default = 1
        BEGIN
            UPDATE kernels SET is_default = 0 WHERE is_default = 1;
        END;
    "#,
    )?;

    Ok(())
}
