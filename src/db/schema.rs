use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

pub fn init_db(db_path: &Path) -> Result<Connection> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }

    let conn = Connection::open(db_path)
        .with_context(|| format!("failed to open database at {}", db_path.display()))?;

    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;",
    )
    .context("failed to set pragmas")?;

    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS layers_meta (
            layer_id    TEXT PRIMARY KEY,
            layer_type  TEXT NOT NULL,
            config_hash TEXT NOT NULL,
            status      TEXT NOT NULL,
            updated_at  INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS layer_packages (
            package_name TEXT NOT NULL,
            layer_id     TEXT NOT NULL REFERENCES layers_meta(layer_id) ON DELETE CASCADE,
            version      TEXT NOT NULL,
            PRIMARY KEY (package_name, layer_id)
        );

        CREATE TABLE IF NOT EXISTS airootfs_manifest (
            path       TEXT PRIMARY KEY,
            size       INTEGER NOT NULL,
            mtime_sec  INTEGER NOT NULL,
            mtime_nsec INTEGER NOT NULL,
            sha256     TEXT    NOT NULL,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS build_artifacts (
            artifact_key  TEXT PRIMARY KEY,
            content_hash  TEXT NOT NULL,
            file_path     TEXT NOT NULL,
            updated_at    INTEGER NOT NULL
        );
        ",
    )
    .context("failed to create tables")?;

    Ok(conn)
}
