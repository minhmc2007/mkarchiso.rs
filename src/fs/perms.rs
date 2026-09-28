use anyhow::{Context, Result};
use nix::sys::stat::{fchmodat, FchmodatFlags, Mode};
use nix::unistd::{chown, Gid, Uid};
use std::path::Path;
use tracing::{debug, info};

#[derive(Debug, Clone)]
pub struct FilePermission {
    pub path: String,
    pub uid: u32,
    pub gid: u32,
    pub mode: u32,
}

pub fn apply_permissions(root_dir: &Path, permissions: &[FilePermission]) -> Result<()> {
    info!(count = permissions.len(), "Applying file permissions");

    for perm in permissions {
        let target = root_dir.join(&perm.path);

        if !target.exists() {
            debug!(path = %perm.path, "File not found, skipping permission apply");
            continue;
        }

        let uid = Uid::from_raw(perm.uid);
        let gid = Gid::from_raw(perm.gid);

        chown(&target, Some(uid), Some(gid)).with_context(|| {
            format!(
                "Failed to chown {} to {}:{}",
                target.display(),
                perm.uid,
                perm.gid
            )
        })?;

        let mode = Mode::from_bits(perm.mode).with_context(|| {
            format!(
                "Invalid mode bits {:o} for {}",
                perm.mode,
                target.display()
            )
        })?;

        fchmodat(None, &target, mode, FchmodatFlags::FollowSymlink).with_context(|| {
            format!(
                "Failed to chmod {} to {:o}",
                target.display(),
                perm.mode
            )
        })?;

        debug!(
            path = %perm.path,
            uid = perm.uid,
            gid = perm.gid,
            mode = format!("{:o}", perm.mode),
            "Applied permissions"
        );
    }

    info!("File permissions applied");
    Ok(())
}
