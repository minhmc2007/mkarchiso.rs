use anyhow::{Context, Result};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use crate::db::state::StateStore;
use crate::util::cmd::run_command;
use crate::util::hash::blake3_bytes;

pub struct FoundationBuilder {
    work_dir: PathBuf,
    pacman_conf: PathBuf,
    bootstrap_packages: HashSet<String>,
    state: StateStore,
}

impl FoundationBuilder {
    pub fn new(
        work_dir: PathBuf,
        pacman_conf: PathBuf,
        bootstrap_packages: HashSet<String>,
        state: StateStore,
    ) -> Self {
        Self {
            work_dir,
            pacman_conf,
            bootstrap_packages,
            state,
        }
    }

    pub fn build(&self) -> Result<PathBuf> {
        let rootfs_dir = self.work_dir.join("layers").join("base").join("rootfs");

        let mut packages_sorted: Vec<&str> =
            self.bootstrap_packages.iter().map(|s| s.as_str()).collect();
        packages_sorted.sort();

        let pacman_bytes = fs::read(&self.pacman_conf)
            .with_context(|| format!("Failed to read pacman.conf: {}", self.pacman_conf.display()))?;

        let mut input = packages_sorted.join(",");
        input.push('\n');
        input.push_str(&String::from_utf8_lossy(&pacman_bytes));

        let input_hash = blake3_bytes(input.as_bytes());
        let layer_id = "foundation";

        if let Some(cached) = self.state.get_build_artifact(layer_id) {
            if cached.content_hash == input_hash && rootfs_dir.exists() {
                tracing::info!(
                    layer_id,
                    hash = %input_hash,
                    "Foundation cache hit"
                );
                return Ok(rootfs_dir);
            }
        }

        tracing::info!(layer_id, hash = %input_hash, "Foundation cache miss");

        fs::create_dir_all(&rootfs_dir).with_context(|| {
            format!(
                "Failed to create rootfs directory: {}",
                rootfs_dir.display()
            )
        })?;

        let pkg_args: Vec<&str> = packages_sorted.iter().copied().collect();

        let pacman_conf_str = self.pacman_conf.to_str().unwrap_or_default();
        let rootfs_str = rootfs_dir.to_str().unwrap_or_default();

        let mut args: Vec<&str> = vec![
            "-C", pacman_conf_str,
            "-c", "-G", "-M",
            rootfs_str,
        ];
        args.extend_from_slice(&pkg_args);

        run_command("pacstrap", &args, "foundation-pacstrap")
            .context("pacstrap failed for foundation layer")?;

        self.state
            .set_build_artifact(layer_id, &input_hash, rootfs_dir.to_str().unwrap_or_default())
            .context("Failed to record foundation artifact in state DB")?;

        tracing::info!(layer_id, path = %rootfs_dir.display(), "Foundation built");
        Ok(rootfs_dir)
    }
}
