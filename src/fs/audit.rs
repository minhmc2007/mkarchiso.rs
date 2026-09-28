use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use tracing::{debug, info};

use crate::db::state::StateStore;

#[derive(Debug)]
pub struct AuditResult {
    pub dirty: bool,
    pub new_files: Vec<PathBuf>,
    pub changed_files: Vec<PathBuf>,
    pub deleted_files: Vec<PathBuf>,
}

pub struct AirootfsAudit {
    airootfs_dir: PathBuf,
    state: StateStore,
}

impl AirootfsAudit {
    pub fn new(airootfs_dir: PathBuf, state_store: StateStore) -> Self {
        Self {
            airootfs_dir,
            state: state_store,
        }
    }

    pub fn audit(&self) -> Result<AuditResult> {
        info!(dir = %self.airootfs_dir.display(), "Auditing airootfs");

        let mut new_files = Vec::new();
        let mut changed_files = Vec::new();
        let mut seen_paths: Vec<String> = Vec::new();

        for entry in walkdir::WalkDir::new(&self.airootfs_dir)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();

            if !path.is_file() {
                continue;
            }

            let relative = path
                .strip_prefix(&self.airootfs_dir)
                .context("Failed to compute relative path")?;
            let relative_str = relative.to_string_lossy().to_string();

            seen_paths.push(relative_str.clone());

            let metadata = fs::metadata(path)
                .with_context(|| format!("Failed to stat {}", path.display()))?;

            let mtime_sec = metadata.mtime();
            let mtime_nsec = metadata.mtime_nsec() as i64;
            let size = metadata.len() as i64;

            if let Some(entry) = self.state.get_airootfs_entry(&relative_str) {
                if entry.mtime_sec == mtime_sec
                    && entry.mtime_nsec == mtime_nsec
                    && entry.size == size
                {
                    debug!(path = %relative_str, "File unchanged (metadata match)");
                    continue;
                }

                let hash = sha256_file(path)
                    .with_context(|| format!("Failed to hash {}", path.display()))?;

                if hash == entry.sha256 {
                    debug!(path = %relative_str, "File content unchanged, updating metadata");
                    self.state.upsert_airootfs_entry(
                        &relative_str,
                        size,
                        mtime_sec,
                        mtime_nsec,
                        &hash,
                    )?;
                    continue;
                }

                info!(path = %relative_str, "File changed");
                changed_files.push(path.to_path_buf());

                self.state.upsert_airootfs_entry(
                    &relative_str,
                    size,
                    mtime_sec,
                    mtime_nsec,
                    &hash,
                )?;
            } else {
                let hash = sha256_file(path)
                    .with_context(|| format!("Failed to hash {}", path.display()))?;

                info!(path = %relative_str, "New file discovered");
                new_files.push(path.to_path_buf());

                self.state.upsert_airootfs_entry(
                    &relative_str,
                    size,
                    mtime_sec,
                    mtime_nsec,
                    &hash,
                )?;
            }
        }

        let dirty = !new_files.is_empty() || !changed_files.is_empty();

        if dirty {
            info!(
                new = new_files.len(),
                changed = changed_files.len(),
                "Audit complete"
            );
        } else {
            info!("Audit complete");
        }

        Ok(AuditResult {
            dirty,
            new_files,
            changed_files,
            deleted_files: Vec::new(),
        })
    }
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file =
        fs::File::open(path).with_context(|| format!("Failed to open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];

    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    Ok(format!("{:x}", hasher.finalize()))
}
