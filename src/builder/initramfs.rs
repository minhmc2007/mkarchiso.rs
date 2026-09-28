use anyhow::{Context, Result};
use nix::mount::{mount, MsFlags};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

use crate::util::cmd::run_command;

fn unmount_chroot_leaks(dir: &Path) {
    for &d in &["/run", "/sys", "/proc", "/dev"] {
        let target = dir.join(d.trim_start_matches('/'));
        if target.exists() {
            let _ = nix::mount::umount2(
                target.to_str().unwrap_or_default(),
                nix::mount::MntFlags::MNT_DETACH,
            );
        }
    }
}

pub fn short_link(work_dir: &Path) -> Result<PathBuf> {
    let canonical = fs::canonicalize(work_dir)
        .with_context(|| format!("Failed to canonicalize {}", work_dir.display()))?;
    let short = std::env::temp_dir().join("ol");
    if short.symlink_metadata().map(|m| m.is_symlink()).unwrap_or(false) {
        fs::remove_file(&short).ok();
    }
    std::os::unix::fs::symlink(&canonical, &short)
        .with_context(|| format!("Failed to symlink {} -> {}", short.display(), canonical.display()))?;
    Ok(short)
}

pub fn remove_short_link(short: &Path) {
    if short.symlink_metadata().map(|m| m.is_symlink()).unwrap_or(false) {
        let _ = fs::remove_file(short);
    }
}

pub fn lowerdir_opts(short_work: &Path, group_count: usize) -> String {
    let mut parts: Vec<String> = Vec::new();

    for i in (1..=group_count).rev() {
        let upper = short_work.join("layers").join(format!("group_{}", i)).join("upper");
        if upper.exists() {
            parts.push(upper.to_str().unwrap_or_default().to_string());
        }
    }
    let base = short_work.join("layers").join("base").join("rootfs");
    if base.exists() {
        parts.push(base.to_str().unwrap_or_default().to_string());
    }

    format!("lowerdir={}", parts.join(":"))
}

pub struct MergedOverlay {
    pub merged: PathBuf,
    _eph_upper: PathBuf,
    _eph_work: PathBuf,
    _ol: PathBuf,
}

pub fn mount_merged_overlay(
    work_dir: &Path,
    group_count: usize,
) -> Result<MergedOverlay> {
    // Short symlink so mount options stay under the kernel's 4096-byte limit.
    let ol = short_link(work_dir)?;

    // Mount point and ephemeral dirs live inside work_dir (ext4, not tmpfs).
    let merged = work_dir.join("merged");
    let eph_upper = work_dir.join(".eph-upper");
    let eph_work = work_dir.join(".eph-work");

    // Clean stale state from previous (possibly crashed) runs.
    let _ = nix::mount::umount2(merged.to_str().unwrap_or_default(), nix::mount::MntFlags::MNT_DETACH);
    let _ = fs::remove_dir_all(&eph_upper);
    let _ = fs::remove_dir_all(&eph_work);
    let _ = fs::remove_dir(&merged);

    fs::create_dir_all(&merged)?;
    fs::create_dir_all(&eph_upper)?;
    fs::create_dir_all(&eph_work)?;

    // All paths in mount options go through the short symlink.
    let lower = lowerdir_opts(&ol, group_count);
    let opts = format!(
        "{},upperdir={},workdir={}",
        lower,
        ol.join(".eph-upper").to_str().unwrap(),
        ol.join(".eph-work").to_str().unwrap(),
    );

    mount(
        Some("overlay"),
        merged.to_str().context("merged not UTF-8")?,
        Some("overlay"),
        MsFlags::empty(),
        Some(opts.as_str()),
    )
    .context("Failed to mount merged overlay")?;

    info!("Mounted merged overlay");

    Ok(MergedOverlay { merged, _eph_upper: eph_upper, _eph_work: eph_work, _ol: ol })
}

pub fn generate_initramfs(
    work_dir: &Path,
    iso_dir: &Path,
    group_count: usize,
) -> Result<()> {
    let overlay = mount_merged_overlay(work_dir, group_count)?;
    let merged = &overlay.merged;
    let iso_boot_dir = iso_dir.join("arch/boot/x86_64");

    fs::create_dir_all(&iso_boot_dir)?;

    for &dir in &["/dev", "/proc", "/sys", "/run"] {
        let target = merged.join(dir.trim_start_matches('/'));
        fs::create_dir_all(&target).ok();
        mount(
            Some(dir),
            target.to_str().context("bind target not UTF-8")?,
            None::<&str>,
            MsFlags::MS_BIND,
            None::<&str>,
        )
        .with_context(|| format!("Failed to bind-mount {dir}"))?;
    }

    let merged_str = merged.to_str().context("merged not UTF-8")?;
    let chroot_result = run_command(
        "arch-chroot",
        &[merged_str, "/usr/bin/mkinitcpio", "-P"],
        "mkinitcpio",
    );

    for &dir in ["/run", "/sys", "/proc", "/dev"].iter() {
        let target = merged.join(dir.trim_start_matches('/'));
        let _ = nix::mount::umount2(target.to_str().unwrap_or_default(), nix::mount::MntFlags::MNT_DETACH);
    }
    unmount_chroot_leaks(merged);

    chroot_result?;

    let boot_dir = merged.join("boot");
    let mut found = 0;
    if let Ok(entries) = fs::read_dir(&boot_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let s = name.to_string_lossy();
            if s.starts_with("vmlinuz-") {
                let suffix = &s["vmlinuz-".len()..];
                fs::copy(entry.path(), iso_boot_dir.join(&name)).ok();
                info!(kernel = %suffix, "Extracted kernel");
                let iramfs = format!("initramfs-{}.img", suffix);
                let src = boot_dir.join(&iramfs);
                if src.exists() {
                    fs::copy(&src, iso_boot_dir.join(&iramfs)).ok();
                    info!(initramfs = %iramfs, "Extracted initramfs");
                }
                found += 1;
            }
        }
    }
    if found == 0 {
        warn!("No kernels found in /boot");
    }

    Ok(())
}

pub fn cleanup_merged_overlay(work_dir: &Path) {
    let merged = work_dir.join("merged");
    let ol = std::env::temp_dir().join("ol");

    for &d in &["/run", "/sys", "/proc", "/dev"] {
        let target = merged.join(d.trim_start_matches('/'));
        let _ = nix::mount::umount2(target.to_str().unwrap_or_default(), nix::mount::MntFlags::MNT_DETACH);
    }
    unmount_chroot_leaks(&merged);
    let _ = nix::mount::umount2(merged.to_str().unwrap_or_default(), nix::mount::MntFlags::MNT_DETACH);

    let _ = fs::remove_file(&ol);
    let _ = fs::remove_dir_all(work_dir.join(".eph-upper"));
    let _ = fs::remove_dir_all(work_dir.join(".eph-work"));
    let _ = fs::remove_dir(&merged);
}
