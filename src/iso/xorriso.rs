use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::info;

use crate::util::cmd::run_command;

pub struct IsoBuilder {
    out_dir: PathBuf,
    iso_dir: PathBuf,
    iso_name: String,
    iso_version: String,
    iso_label: String,
    iso_application: String,
    iso_publisher: String,
}

impl IsoBuilder {
    pub fn new(
        out_dir: PathBuf,
        iso_dir: PathBuf,
        iso_name: String,
        iso_version: String,
        iso_label: String,
        iso_application: String,
        iso_publisher: String,
    ) -> Self {
        Self { out_dir, iso_dir, iso_name, iso_version, iso_label, iso_application, iso_publisher }
    }

    pub fn build(&self) -> Result<PathBuf> {
        let iso_path = self.out_dir.join(format!("{}-{}.iso", self.iso_name, self.iso_version));
        info!(output = %iso_path.display(), "Building hybrid ISO");
        fs::create_dir_all(&self.out_dir)?;
        self.run_xorriso(&iso_path)?;
        self.generate_checksum(&iso_path)?;
        info!(iso = %iso_path.display(), "ISO build complete");
        Ok(iso_path)
    }

    fn run_xorriso(&self, iso_path: &Path) -> Result<()> {
        let isolinux_bin = self.iso_dir.join("boot/syslinux/isolinux.bin");
        let isohdpfx_bin = self.iso_dir.join("boot/syslinux/isohdpfx.bin");
        let boot_catalog = self.iso_dir.join("boot/syslinux/boot.cat");
        let efi_img = self.iso_dir.join("EFI/archiso/efiboot.img");

        // xorriso needs paths relative to the ISO root, not absolute host paths.
        let iso_root = self.iso_dir.to_str().unwrap_or_default();
        let rel = |p: &Path| -> String {
            p.to_str().unwrap_or_default()
                .strip_prefix(iso_root)
                .unwrap_or(p.to_str().unwrap_or_default())
                .to_string()
        };

        let mut args: Vec<String> = Vec::new();

        // Prevent xorriso from reading user config files.
        args.push("-no_rc".into());

        args.push("-as".into()); args.push("mkisofs".into());
        args.push("-iso-level".into()); args.push("3".into());
        args.push("-full-iso9660-filenames".into());
        args.push("-joliet".into());
        args.push("-joliet-long".into());
        args.push("-rational-rock".into());

        args.push("-volid".into()); args.push(self.iso_label.clone());
        args.push("-appid".into()); args.push(self.iso_application.clone());
        args.push("-publisher".into()); args.push(self.iso_publisher.clone());
        args.push("-preparer".into()); args.push("mkarchiso-rs".into());

        // BIOS El Torito boot.
        if isolinux_bin.exists() {
            args.push("-eltorito-boot".into());
            args.push(rel(&isolinux_bin));
            args.push("-eltorito-catalog".into());
            args.push(rel(&boot_catalog));
            args.push("-no-emul-boot".into());
            args.push("-boot-load-size".into()); args.push("4".into());
            args.push("-boot-info-table".into());
            if isohdpfx_bin.exists() {
                args.push("-isohybrid-mbr".into());
                args.push(isohdpfx_bin.to_str().unwrap_or_default().to_string());
            }
            args.push("--mbr-force-bootable".into());
            args.push("-partition_offset".into()); args.push("16".into());
        }

        // EFI El Torito boot.
        if efi_img.exists() {
            args.push("-eltorito-alt-boot".into());
            args.push("-e".into());
            args.push(rel(&efi_img));
            args.push("-no-emul-boot".into());
            args.push("-isohybrid-gpt-basdat".into());
        }

        // Output must come before the source directory.
        args.push("-o".into());
        args.push(iso_path.to_str().unwrap_or_default().to_string());

        // Source directory (must be last).
        args.push(self.iso_dir.to_str().unwrap_or_default().to_string());

        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        run_command("xorriso", &arg_refs, "xorriso-mkisofs").context("xorriso failed")?;
        Ok(())
    }

    fn generate_checksum(&self, iso_path: &Path) -> Result<()> {
        let dir = iso_path.parent().context("ISO path has no parent")?;
        let checksum_path = dir.join(format!("{}.sha256sum",
            iso_path.file_stem().unwrap_or_default().to_string_lossy()));
        let output = run_command("sha256sum", &[iso_path.to_str().unwrap_or_default()], "sha256sum")?;
        std::fs::write(&checksum_path, &output.stdout)?;
        Ok(())
    }
}
