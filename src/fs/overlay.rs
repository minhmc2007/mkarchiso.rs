use anyhow::{bail, Context, Result};
use nix::mount::{mount, MsFlags};
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use tracing::{debug, info};

const MASKED_HOOKS: &[&str] = &[
    "90-mkinitcpio-install.hook",
    "90-mkinitcpio-generate-shim-early-hook",
];

pub struct OverlayFs {
    lowerdir: String,
    upperdir: String,
    workdir: String,
    merged: String,
}

impl OverlayFs {
    pub fn new(lowerdir: &str, upperdir: &str, workdir: &str, merged: &str) -> Self {
        Self {
            lowerdir: lowerdir.to_string(),
            upperdir: upperdir.to_string(),
            workdir: workdir.to_string(),
            merged: merged.to_string(),
        }
    }

    pub fn mount(&self) -> Result<()> {
        let options = format!(
            "lowerdir={},upperdir={},workdir={}",
            self.lowerdir, self.upperdir, self.workdir
        );

        debug!(
            lowerdir = %self.lowerdir,
            upperdir = %self.upperdir,
            workdir = %self.workdir,
            merged = %self.merged,
            "Mounting overlay"
        );

        mount(
            Some("overlay"),
            self.merged.as_str(),
            Some("overlay"),
            MsFlags::empty(),
            Some(options.as_str()),
        )
        .context(format!(
            "Failed to mount overlay at {} (lowerdir={}, upperdir={}, workdir={})",
            self.merged, self.lowerdir, self.upperdir, self.workdir
        ))?;

        debug!("Overlay mounted at {}", self.merged);
        Ok(())
    }

    pub fn umount(&self) -> Result<()> {
        debug!(merged = %self.merged, "Unmounting overlay");

        nix::mount::umount2(
            self.merged.as_str(),
            nix::mount::MntFlags::MNT_DETACH,
        )
        .context(format!("Failed to unmount overlay at {}", self.merged))?;

        debug!("Overlay unmounted at {}", self.merged);
        Ok(())
    }
}

pub fn mask_hooks(merged_dir: &Path) -> Result<Vec<PathBuf>> {
    let hooks_dir = merged_dir.join("usr/share/libalpm/hooks");

    if !hooks_dir.is_dir() {
        bail!(
            "ALPM hooks directory does not exist: {}",
            hooks_dir.display()
        );
    }

    let mut created = Vec::new();

    for hook_name in MASKED_HOOKS {
        let hook_path = hooks_dir.join(hook_name);

        if !hook_path.exists() {
            debug!("Hook {} not found, skipping mask", hook_name);
            continue;
        }

        fs::remove_file(&hook_path).context(format!(
            "Failed to remove hook {} for masking",
            hook_path.display()
        ))?;

        symlink("/dev/null", &hook_path).context(format!(
            "Failed to create mask symlink for {}",
            hook_path.display()
        ))?;

        info!(hook = %hook_path.display(), "Masked ALPM hook");
        created.push(hook_path);
    }

    Ok(created)
}

pub fn unmask_hooks(symlinks: &[PathBuf]) -> Result<()> {
    for symlink_path in symlinks {
        if symlink_path.is_symlink() {
            fs::remove_file(symlink_path).context(format!(
                "Failed to remove mask symlink {}",
                symlink_path.display()
            ))?;
            debug!(path = %symlink_path.display(), "Removed mask symlink");
        }
    }
    Ok(())
}
