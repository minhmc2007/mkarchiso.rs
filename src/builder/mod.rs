pub mod erofs;
pub mod foundation;
pub mod initramfs;

use anyhow::Result;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use tracing::info;

use crate::util::cmd::run_command;

pub use foundation::FoundationBuilder;

pub fn run_worker_pool(
    workspace: &Path,
    work_dir: &Path,
    package_groups: &[HashSet<String>],
    jobs: usize,
) -> Result<()> {
    let pacman_conf = workspace.join("pacman.conf");
    let base_rootfs = work_dir.join("layers").join("base").join("rootfs");
    let total = package_groups.len();

    let actual_jobs = jobs.min(total);

    let mp = MultiProgress::new();
    let slot_bars: Vec<ProgressBar> = (0..actual_jobs)
        .map(|_| {
            let bar = mp.add(ProgressBar::new(0));
            bar.set_style(
                ProgressStyle::default_bar()
                    .template("  {spinner:.dim} {msg}")
                    .unwrap()
                    .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"),
            );
            bar.enable_steady_tick(std::time::Duration::from_millis(80));
            bar
        })
        .collect();

    let summary_bar = mp.add(ProgressBar::new(total as u64));
    summary_bar.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{pos}/{len}] {msg}")
            .unwrap()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"),
    );

    let pacman_conf = Arc::new(pacman_conf);
    let base_rootfs = Arc::new(base_rootfs);
    let work_dir = Arc::new(work_dir.to_path_buf());

    let (tx, rx) = std::sync::mpsc::sync_channel::<(usize, HashSet<String>)>(total);
    for (i, group) in package_groups.iter().enumerate() {
        tx.send((i, group.clone())).ok();
    }
    drop(tx);

    let rx = Arc::new(std::sync::Mutex::new(rx));

    let mut handles = Vec::with_capacity(actual_jobs);
    for slot in 0..actual_jobs {
        let rx = Arc::clone(&rx);
        let pacman_conf = Arc::clone(&pacman_conf);
        let base_rootfs = Arc::clone(&base_rootfs);
        let work_dir = Arc::clone(&work_dir);
        let bar = slot_bars[slot].clone();
        let summary = summary_bar.clone();

        handles.push(std::thread::spawn(move || {
            while let Ok((idx, group)) = rx.lock().unwrap().recv() {
                let group_id = format!("group_{}", idx + 1);
                bar.set_message(format!("{} ({} pkgs)", group_id, group.len()));

                let group_dir = work_dir.join("layers").join(&group_id);
                let upper_dir = group_dir.join("upper");
                let work_subdir = group_dir.join("work");
                let merged_dir = group_dir.join("merged");

                std::fs::create_dir_all(&upper_dir).ok();
                std::fs::create_dir_all(&work_subdir).ok();
                std::fs::create_dir_all(&merged_dir).ok();

                let overlay = crate::fs::overlay::OverlayFs::new(
                    base_rootfs.to_str().unwrap_or_default(),
                    upper_dir.to_str().unwrap_or_default(),
                    work_subdir.to_str().unwrap_or_default(),
                    merged_dir.to_str().unwrap_or_default(),
                );
                if let Err(e) = overlay.mount() {
                    bar.finish_with_message(format!("{} FAILED mount: {}", group_id, e));
                    summary.inc(1);
                    continue;
                }

                let masked = crate::fs::overlay::mask_hooks(&merged_dir).unwrap_or_default();

                let pacman_str = pacman_conf.to_str().unwrap_or_default();
                let merged_str = merged_dir.to_str().unwrap_or_default();
                let mut args: Vec<&str> = vec!["-C", pacman_str, "-c", "-G", "-M", merged_str];
                let mut pkgs: Vec<&str> = group.iter().map(|s| s.as_str()).collect();
                pkgs.sort();
                args.extend_from_slice(&pkgs);

                let result = run_command("pacstrap", &args, &group_id);

                let _ = crate::fs::overlay::unmask_hooks(&masked);
                let _ = overlay.umount();

                match result {
                    Ok(_) => {
                        bar.finish_with_message(format!("{} done", group_id));
                    }
                    Err(e) => {
                        bar.finish_with_message(format!("{} FAILED: {}", group_id, e));
                    }
                }
                summary.inc(1);
            }
        }));
    }

    summary_bar.set_message(format!("installing packages (j={})", actual_jobs));
    for handle in handles {
        handle.join().ok();
    }

    summary_bar.finish_with_message("packages done");
    info!(groups = total, jobs, "Worker pool complete");
    Ok(())
}
