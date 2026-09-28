use anyhow::{Context, Result};
use std::fs;
use std::path::Path;
use tracing::{debug, info, warn};

use crate::config::profile::ProfileConfig;
use crate::util::cmd::run_command;

pub fn stage_bootloaders(
    workspace: &Path,
    work_x86_64: &Path,
    iso_dir: &Path,
    profile: &ProfileConfig,
    iso_uuid: &str,
) -> Result<()> {
    info!(name = %profile.iso_name, "Staging bootloader files");
    fs::create_dir_all(iso_dir)?;

    stage_systemd_boot(workspace, iso_dir)?;
    stage_syslinux(workspace, work_x86_64, iso_dir, profile, iso_uuid)?;
    stage_grub(workspace, iso_dir)?;
    create_efi_boot_image(workspace, work_x86_64, iso_dir)?;

    info!("Bootloader staging complete");
    Ok(())
}

fn stage_systemd_boot(workspace: &Path, iso_dir: &Path) -> Result<()> {
    let efiboot_src = workspace.join("efiboot/loader");
    let loader_dst = iso_dir.join("loader");

    if efiboot_src.is_dir() {
        copy_dir_recursive(&efiboot_src, &loader_dst)?;
    } else {
        warn!(path = %efiboot_src.display(), "efiboot/loader/ not found, skipping");
    }

    // Copy EFI binaries from efiboot/ root (e.g. bootx64.efi, fbx64.efi).
    let efi_boot_dst = iso_dir.join("EFI/BOOT");
    fs::create_dir_all(&efi_boot_dst)?;

    let efi_src = workspace.join("efiboot");
    if efi_src.is_dir() {
        for entry in fs::read_dir(&efi_src)?.flatten() {
            let name = entry.file_name();
            if name.to_string_lossy().ends_with(".efi") {
                fs::copy(entry.path(), efi_boot_dst.join(&name)).ok();
            }
        }
    }
    Ok(())
}

fn stage_syslinux(
    workspace: &Path,
    work_x86_64: &Path,
    iso_dir: &Path,
    profile: &ProfileConfig,
    iso_uuid: &str,
) -> Result<()> {
    let syslinux_dst = iso_dir.join("boot/syslinux");
    fs::create_dir_all(&syslinux_dst)?;

    // Copy syslinux config files from workspace.
    let syslinux_src = workspace.join("syslinux");
    if syslinux_src.is_dir() {
        copy_dir_recursive(&syslinux_src, &syslinux_dst)?;
    }

    // Substitute %INSTALL_DIR% / %ARCH% placeholders in staged configs.
    if let Ok(entries) = fs::read_dir(&syslinux_dst) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().map(|e| e == "cfg").unwrap_or(false) {
                if let Ok(contents) = fs::read_to_string(&p) {
                    let sub = contents
                        .replace("%INSTALL_DIR%", &profile.install_dir)
                        .replace("%ARCH%", "x86_64")
                        .replace("%ARCHISO_UUID%", iso_uuid);
                    if sub != contents {
                        fs::write(&p, sub).ok();
                    }
                }
            }
        }
    }

    // Source syslinux binaries from the pacstrap overlay layers, not the host.
    let layers_dir = work_x86_64.join("layers");
    if let Ok(entries) = fs::read_dir(&layers_dir) {
        for layer in entries.flatten() {
            let bios_dir = layer.path().join("upper/usr/lib/syslinux/bios");
            if bios_dir.is_dir() {
                for entry in fs::read_dir(&bios_dir)?.flatten() {
                    let name = entry.file_name();
                    let s = name.to_string_lossy();
                    if s.ends_with(".c32") || s == "isolinux.bin" || s == "isohdpfx.bin"
                        || s == "lpxelinux.0" || s == "memdisk"
                        || s == "ldlinux.sys"
                    {
                        let dst = syslinux_dst.join(&name);
                        if !dst.exists() {
                            fs::copy(entry.path(), &dst).ok();
                            debug!(file = %s, "Copied syslinux module");
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

fn stage_grub(workspace: &Path, iso_dir: &Path) -> Result<()> {
    let grub_src = workspace.join("grub");
    let grub_dst = iso_dir.join("boot/grub");
    if grub_src.is_dir() {
        fs::create_dir_all(&grub_dst)?;
        copy_dir_recursive(&grub_src, &grub_dst)?;
    }
    Ok(())
}

fn create_efi_boot_image(workspace: &Path, work_x86_64: &Path, iso_dir: &Path) -> Result<()> {
    let efi_arch_dir = iso_dir.join("EFI/archiso");
    fs::create_dir_all(&efi_arch_dir)?;

    let efiboot_img = efi_arch_dir.join("efiboot.img");
    let efi_boot_src = iso_dir.join("EFI/BOOT");

    if !efi_boot_src.is_dir() {
        warn!("EFI/BOOT/ not found, skipping EFI boot image");
        return Ok(());
    }

    let has_efi = fs::read_dir(&efi_boot_src)?
        .flatten()
        .any(|e| e.path().extension().map(|x| x == "efi").unwrap_or(false));
    if !has_efi {
        warn!("No .efi files in EFI/BOOT/, skipping");
        return Ok(());
    }

    // Calculate image size: sum of EFI/BOOT + kernel/initramfs + overhead.
    let mut size_bytes: u64 = 8 * 1024 * 1024; // 8 MiB overhead
    add_dir_size(&efi_boot_src, &mut size_bytes);

    // Also include kernel and initramfs (systemd-boot needs them on the ESP).
    let merged_boot = work_x86_64.join("layers/base/rootfs/boot");
    if merged_boot.is_dir() {
        for entry in fs::read_dir(&merged_boot)?.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("vmlinuz-") || name.starts_with("initramfs-") {
                size_bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }

    let size_mib = ((size_bytes / (1024 * 1024)) + 1) as usize;

    // Create blank FAT image.
    run_command("dd", &[
        "if=/dev/zero",
        &format!("of={}", efiboot_img.display()),
        "bs=1M",
        &format!("count={}", size_mib),
    ], "efiboot-dd")?;

    run_command("mkfs.fat", &[
        "-F", "32", "-n", "ARCHISO_EFI",
        efiboot_img.to_str().unwrap_or_default(),
    ], "efiboot-mkfs")?;

    // Use mtools to copy files (no mount needed).
    // Copy EFI/BOOT contents.
    copy_to_fat_image(&efiboot_img, &efi_boot_src, "EFI/BOOT")?;

    // Copy kernel and initramfs into the FAT image root.
    if merged_boot.is_dir() {
        for entry in fs::read_dir(&merged_boot)?.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("vmlinuz-") || name.starts_with("initramfs-") {
                copy_file_to_fat(&efiboot_img, &entry.path(), &name)?;
                debug!(file = %name, "Copied to EFI boot image");
            }
        }
    }

    info!(size = format!("{} MiB", size_mib), "EFI boot image created");
    Ok(())
}

fn copy_to_fat_image(img: &Path, src_dir: &Path, dest_prefix: &str) -> Result<()> {
    // Ensure subdirectory exists in the FAT image.
    let _ = run_command("mmd", &[
        "-i", img.to_str().unwrap_or_default(),
        &format!("::{}", dest_prefix),
    ], "mmd-efi");

    for entry in fs::read_dir(src_dir)?.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let dest_path = format!("{}/{}", dest_prefix, name);
        if entry.path().is_dir() {
            let _ = run_command("mmd", &[
                "-i", img.to_str().unwrap_or_default(),
                &format!("::{}", dest_path),
            ], "mmd-subdir");
            copy_to_fat_image(img, &entry.path(), &dest_path)?;
        } else {
            run_command("mcopy", &[
                "-i", img.to_str().unwrap_or_default(),
                entry.path().to_str().unwrap_or_default(),
                &format!("::{}", dest_path),
            ], "mcopy-efi").ok();
        }
    }
    Ok(())
}

fn copy_file_to_fat(img: &Path, src: &Path, dest_name: &str) -> Result<()> {
    run_command("mcopy", &[
        "-i", img.to_str().unwrap_or_default(),
        src.to_str().unwrap_or_default(),
        &format!("::{}", dest_name),
    ], "mcopy-file").context(format!("Failed to copy {} to EFI image", dest_name))?;
    Ok(())
}

fn add_dir_size(dir: &Path, total: &mut u64) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                add_dir_size(&entry.path(), total);
            } else if let Ok(meta) = entry.metadata() {
                *total += meta.len();
            }
        }
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}
