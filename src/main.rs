mod builder;
mod config;
mod db;
mod fs;
mod graph;
mod iso;
mod util;

use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use nix::unistd::getuid;
use tracing::{info, warn};

use crate::builder::FoundationBuilder;
use crate::builder::erofs::build_erofs;
use crate::builder::initramfs::generate_initramfs;
use crate::builder::run_worker_pool;
use crate::config::packages::{check_deduplication, parse_package_list};
use crate::config::profile::ProfileConfig;
use crate::db::init_db;
use crate::db::StateStore;
use crate::fs::audit::AirootfsAudit;
use crate::fs::collision::{check_collisions, index_package_files};
use crate::fs::perms::{apply_permissions, FilePermission};
use crate::graph::alpm::AlpmProvider;
use crate::graph::dag::partition_packages;
use crate::iso::bootloaders::stage_bootloaders;
use crate::iso::xorriso::IsoBuilder;
use crate::util::cmd::binary_exists;

#[derive(Parser, Debug)]
#[command(name = "mkarchiso-rs", version, about = "Build Arch Linux live ISO images")]
struct Cli {
    #[arg(short, long, default_value = ".")]
    workspace: PathBuf,

    #[arg(short, long, default_value = "./out")]
    out: PathBuf,

    #[arg(long, default_value = "./work")]
    work_dir: PathBuf,

    #[arg(short = 'j', long, default_value_t = num_cpus())]
    jobs: usize,

    #[arg(long)]
    clean: bool,
}

fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    info!(version = env!("CARGO_PKG_VERSION"), "mkarchiso-rs starting");
    preflight(&cli)?;

    if cli.clean {
        info!(dir = %cli.work_dir.display(), "Cleaning working directory");
        if cli.work_dir.exists() {
            // Lazy-unmount any stale mounts left by a previous crashed run.
            let work_x86 = cli.work_dir.join("x86_64");
            let merged = work_x86.join("merged");
            for target in [
                merged.join("run"),
                merged.join("sys"),
                merged.join("proc"),
                merged.join("dev"),
                merged.clone(),
            ] {
                let _ = nix::mount::umount2(
                    target.to_str().unwrap_or_default(),
                    nix::mount::MntFlags::MNT_DETACH,
                );
            }
            // Also unmount per-group merged dirs if any leaked.
            if let Ok(entries) = std::fs::read_dir(work_x86.join("layers")) {
                for entry in entries.flatten() {
                    let _ = nix::mount::umount2(
                        entry.path().join("merged").to_str().unwrap_or_default(),
                        nix::mount::MntFlags::MNT_DETACH,
                    );
                }
            }
            let _ = std::fs::remove_file(std::env::temp_dir().join("ol"));
            std::fs::remove_dir_all(&cli.work_dir).context("Failed to clean")?;
        }
    }

    let work_x86_64 = cli.work_dir.join("x86_64");
    let iso_dir = cli.out.join("iso");
    let state_db = work_x86_64.join("state.db");
    let airootfs_dir = work_x86_64.join("airootfs");

    std::fs::create_dir_all(&work_x86_64)
        .with_context(|| format!("Failed to create {}", work_x86_64.display()))?;
    std::fs::create_dir_all(&airootfs_dir)
        .with_context(|| format!("Failed to create {}", airootfs_dir.display()))?;

    let pb = ProgressBar::new(9);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{pos}/{len}] {msg}")
            .unwrap()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"),
    );

    pb.set_message("opening database");
    let conn = init_db(&state_db)
        .with_context(|| format!("Failed to open {}", state_db.display()))?;
    let state = StateStore::new(conn);
    pb.inc(1);

    pb.set_message("parsing config");
    let profile_path = cli.workspace.join("profiledef.sh");
    let profile = ProfileConfig::from_file(&profile_path)
        .with_context(|| format!("Failed to parse {}", profile_path.display()))?;
    info!(name = %profile.iso_name, "Profile loaded");

    let bootstrap_path = cli.workspace.join("bootstrap_packages");
    let packages_path = cli.workspace.join("packages.x86_64");

    let bootstrap_pkgs = if bootstrap_path.exists() {
        parse_package_list(&bootstrap_path).context("Failed to parse bootstrap_packages")?
    } else {
        warn!("No bootstrap_packages found");
        HashSet::new()
    };

    let packages = if packages_path.exists() {
        parse_package_list(&packages_path).context("Failed to parse packages.x86_64")?
    } else {
        bail!("packages.x86_64 not found in {}", cli.workspace.display());
    };

    if let Err(e) = check_deduplication(&bootstrap_pkgs, &packages) {
        warn!("{}", e);
    }
    info!(bootstrap = bootstrap_pkgs.len(), packages = packages.len(), "Config parsed");
    pb.inc(1);

    pb.set_message("building dependency graph");
    let pacman_conf = cli.workspace.join("pacman.conf");
    let provider = AlpmProvider::new(pacman_conf.clone());
    let package_groups = partition_packages(&packages, &provider)
        .context("Failed to partition dependency graph")?;
    info!(groups = package_groups.len(), "Dependency graph built");
    pb.inc(1);

    pb.set_message("installing foundation");
    let foundation = FoundationBuilder::new(
        work_x86_64.clone(),
        pacman_conf.clone(),
        bootstrap_pkgs.clone(),
        state,
    );
    let _base_rootfs = foundation.build().context("Foundation build failed")?;
    pb.inc(1);

    pb.set_message("installing packages");
    run_worker_pool(
        &cli.workspace,
        &work_x86_64,
        &package_groups,
        cli.jobs,
    )
    .context("Worker pool failed")?;
    pb.inc(1);

    pb.set_message("auditing airootfs");
    let state2 = crate::db::init_db(&state_db)
        .map(StateStore::new)
        .ok();
    if let Some(st) = state2 {
        let audit = AirootfsAudit::new(airootfs_dir.clone(), st);
        match audit.audit() {
            Ok(result) => {
                if result.dirty {
                    info!(
                        new = result.new_files.len(),
                        changed = result.changed_files.len(),
                        deleted = result.deleted_files.len(),
                        "airootfs is dirty"
                    );
                } else {
                    info!("airootfs unchanged (cache hit)");
                }
            }
            Err(e) => warn!(error = %e, "airootfs audit failed, continuing"),
        }
    }

    let collision_ok = (|| -> Result<bool> {
        let merged_rootfs = std::env::temp_dir().join("mkarchiso-collision-check");
        std::fs::create_dir_all(&merged_rootfs)?;

        let short_work = crate::builder::initramfs::short_link(
            &work_x86_64,
        )?;
        let opts = crate::builder::initramfs::lowerdir_opts(
            &short_work,
            package_groups.len(),
        );
        crate::builder::initramfs::remove_short_link(&short_work);

        if opts == "lowerdir=" {
            let _ = std::fs::remove_dir(&merged_rootfs);
            return Ok(true);
        }

        if let Err(e) = nix::mount::mount(
            Some("overlay"),
            merged_rootfs.to_str().unwrap_or_default(),
            Some("overlay"),
            nix::mount::MsFlags::empty(),
            Some(opts.as_str()),
        ) {
            warn!(error = %e, "Could not mount merged overlay for collision check, skipping");
            let _ = std::fs::remove_dir(&merged_rootfs);
            return Ok(true);
        }

        let pkg_index = index_package_files(&merged_rootfs)
            .unwrap_or_else(|e| {
                warn!(error = %e, "Could not index package files, skipping collision check");
                std::collections::HashMap::new()
            });

        let _ = nix::mount::umount2(
            merged_rootfs.to_str().unwrap_or_default(),
            nix::mount::MntFlags::MNT_DETACH,
        );
        let _ = std::fs::remove_dir(&merged_rootfs);

        let collisions = check_collisions(&airootfs_dir, &pkg_index)?;
        Ok(collisions.is_empty())
    })();

    match collision_ok {
        Ok(true) => info!("No file collisions detected"),
        Ok(false) => bail!("File collision detected! Aborting to prevent silent clobbering."),
        Err(e) => warn!(error = %e, "Collision check failed, continuing"),
    }

    if !profile.file_permissions.is_empty() {
        let perms: Vec<FilePermission> = profile.file_permissions.iter().map(|fp| {
            let (uid, gid) = parse_owner(&fp.owner);
            let mode = u32::from_str_radix(&fp.mode, 8).unwrap_or(0o644);
            FilePermission { path: fp.path.clone(), uid, gid, mode }
        }).collect();
        apply_permissions(&airootfs_dir, &perms).context("Failed to apply permissions")?;
    }
    pb.inc(1);

    pb.set_message("generating initramfs");
    generate_initramfs(&work_x86_64, &iso_dir, package_groups.len())
        .context("Initramfs failed")?;
    pb.inc(1);

    pb.set_message("building EROFS image");
    let erofs_path = work_x86_64.join(format!("{}.img", profile.iso_name));
    let merged_overlay = work_x86_64.join("merged");
    build_erofs(&merged_overlay, &erofs_path)
        .context("EROFS build failed")?;
    crate::builder::initramfs::cleanup_merged_overlay(&work_x86_64);

    let erofs_dest = iso_dir.join(format!("{}/x86_64/airootfs.erofs", profile.install_dir));
    std::fs::create_dir_all(erofs_dest.parent().unwrap())?;
    std::fs::copy(&erofs_path, &erofs_dest)
        .with_context(|| format!("Failed to copy EROFS image to {}", erofs_dest.display()))?;
    pb.inc(1);

    pb.set_message("building ISO");
    // The ISO9660 volume date (which blkid reports as the filesystem UUID) must
    // match %ARCHISO_UUID% in the bootloader configs. xorriso derives the volume
    // date from SOURCE_DATE_EPOCH, so pin it and format the same string here.
    let epoch: i64 = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            now
        });
    std::env::set_var("SOURCE_DATE_EPOCH", epoch.to_string());
    let iso_uuid = crate::util::iso_uuid(epoch);
    // GRUB searches for this empty file to locate the ISO volume.
    let search_file = iso_dir.join(format!("boot/{}.uuid", iso_uuid));
    std::fs::create_dir_all(search_file.parent().unwrap())?;
    std::fs::write(&search_file, "")?;

    stage_bootloaders(&cli.workspace, &work_x86_64, &iso_dir, &profile, &iso_uuid)
        .context("Bootloader staging failed")?;

    let iso_builder = IsoBuilder::new(
        cli.out.clone(),
        iso_dir.clone(),
        profile.iso_name.clone(),
        profile.iso_version.clone(),
        profile.iso_label.clone(),
        profile.iso_application.clone(),
        profile.iso_publisher.clone(),
    );
    let iso_path = iso_builder.build().context("ISO build failed")?;
    pb.inc(1);

    pb.finish_with_message("done");
    info!(iso = %iso_path.display(), "Build complete");

    Ok(())
}

fn preflight(_cli: &Cli) -> Result<()> {
    if !getuid().is_root() {
        bail!("mkarchiso-rs must be run as root (use sudo)");
    }
    let required_binaries = ["pacstrap", "mkfs.erofs", "xorriso", "mkfs.fat", "dd"];
    for bin in &required_binaries {
        if !binary_exists(bin) {
            bail!("Required binary '{}' not found in PATH", bin);
        }
    }
    info!("Pre-flight checks passed");
    Ok(())
}

fn parse_owner(owner: &str) -> (u32, u32) {
    let parts: Vec<&str> = owner.split(':').collect();
    let uid = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
    let gid = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    (uid, gid)
}
