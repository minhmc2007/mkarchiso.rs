use anyhow::{Context, Result};
use rusqlite::{params, Connection};

#[derive(Debug, Clone)]
pub struct AirootfsEntry {
    pub size: i64,
    pub mtime_sec: i64,
    pub mtime_nsec: i64,
    pub sha256: String,
}

#[derive(Debug, Clone)]
pub struct BuildArtifact {
    pub content_hash: String,
}

pub struct StateStore {
    conn: Connection,
}

impl StateStore {
    pub fn new(conn: Connection) -> Self {
        Self { conn }
    }

    pub fn upsert_airootfs_entry(
        &self,
        path: &str,
        size: i64,
        mtime_sec: i64,
        mtime_nsec: i64,
        sha256: &str,
    ) -> Result<()> {
        let now = epoch_now();
        self.conn
            .execute(
                "INSERT INTO airootfs_manifest
                     (path, size, mtime_sec, mtime_nsec, sha256, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(path) DO UPDATE SET
                     size       = excluded.size,
                     mtime_sec  = excluded.mtime_sec,
                     mtime_nsec = excluded.mtime_nsec,
                     sha256     = excluded.sha256,
                     updated_at = excluded.updated_at",
                params![path, size, mtime_sec, mtime_nsec, sha256, now],
            )
            .context("upsert airootfs_manifest")?;
        Ok(())
    }

    pub fn get_airootfs_entry(&self, path: &str) -> Option<AirootfsEntry> {
        self.conn
            .query_row(
                "SELECT size, mtime_sec, mtime_nsec, sha256
                 FROM airootfs_manifest
                 WHERE path = ?1",
                params![path],
                |row| {
                    Ok(AirootfsEntry {
                        size: row.get(0)?,
                        mtime_sec: row.get(1)?,
                        mtime_nsec: row.get(2)?,
                        sha256: row.get(3)?,
                    })
                },
            )
            .ok()
    }

    pub fn set_build_artifact(
        &self,
        artifact_key: &str,
        content_hash: &str,
        file_path: &str,
    ) -> Result<()> {
        let now = epoch_now();
        self.conn
            .execute(
                "INSERT INTO build_artifacts (artifact_key, content_hash, file_path, updated_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(artifact_key) DO UPDATE SET
                     content_hash = excluded.content_hash,
                     file_path    = excluded.file_path,
                     updated_at   = excluded.updated_at",
                params![artifact_key, content_hash, file_path, now],
            )
            .context("upsert build_artifacts")?;
        Ok(())
    }

    pub fn get_build_artifact(&self, artifact_key: &str) -> Option<BuildArtifact> {
        self.conn
            .query_row(
                "SELECT content_hash
                 FROM build_artifacts
                 WHERE artifact_key = ?1",
                params![artifact_key],
                |row| {
                    Ok(BuildArtifact {
                        content_hash: row.get(0)?,
                    })
                },
            )
            .ok()
    }
}

fn epoch_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is before Unix epoch")
        .as_secs() as i64
}
