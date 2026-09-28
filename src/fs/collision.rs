use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tracing::{debug, info, warn};

#[derive(Debug)]
pub struct Collision {
    pub path: String,
    pub conflicting_package: String,
}

pub fn index_package_files(root_dir: &Path) -> Result<HashMap<String, String>> {
    let local_dir = root_dir.join("var/lib/pacman/local");

    if !local_dir.is_dir() {
        anyhow::bail!(
            "Pacman local database not found at {}",
            local_dir.display()
        );
    }

    let mut index: HashMap<String, String> = HashMap::new();

    for entry in fs::read_dir(&local_dir)
        .with_context(|| format!("Failed to read {}", local_dir.display()))?
    {
        let entry = entry?;
        let pkg_dir = entry.path();

        if !pkg_dir.is_dir() {
            continue;
        }

        let files_path = pkg_dir.join("files");
        if !files_path.exists() {
            debug!(
                pkg = %pkg_dir.display(),
                "No files manifest, skipping"
            );
            continue;
        }

        let pkg_name = pkg_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        let content = fs::read_to_string(&files_path)
            .with_context(|| format!("Failed to read {}", files_path.display()))?;

        let mut in_files_section = false;

        for line in content.lines() {
            let trimmed = line.trim();

            if trimmed == "%FILES%" {
                in_files_section = true;
                continue;
            }
            if trimmed.starts_with('%') && trimmed.ends_with('%') {
                in_files_section = false;
                continue;
            }

            if !in_files_section || trimmed.is_empty() {
                continue;
            }

            if trimmed.ends_with('/') {
                continue;
            }

            index.insert(trimmed.to_string(), pkg_name.clone());
        }
    }

    info!(
        packages_indexed = index.len(),
        "Built package file index"
    );

    Ok(index)
}

pub fn check_collisions(
    airootfs_dir: &Path,
    package_file_index: &HashMap<String, String>,
) -> Result<Vec<Collision>> {
    let mut collisions = Vec::new();

    for entry in walkdir::WalkDir::new(airootfs_dir)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();

        if !path.is_file() {
            continue;
        }

        let relative = path
            .strip_prefix(airootfs_dir)
            .context("Failed to compute relative path")?;
        let relative_str = relative.to_string_lossy().to_string();

        if let Some(owner) = package_file_index.get(&relative_str) {
            collisions.push(Collision {
                path: relative_str,
                conflicting_package: owner.clone(),
            });
        }
    }

    if collisions.is_empty() {
        info!("No file collisions detected");
    } else {
        let show = collisions.len().min(10);
        for c in collisions.iter().take(show) {
            warn!(
                path = %c.path,
                package = %c.conflicting_package,
                "File collision"
            );
        }
        if collisions.len() > show {
            warn!(
                remaining = collisions.len() - show,
                "... (use --clean to see full list)"
            );
        }
        warn!(count = collisions.len(), "File collisions detected");
    }

    Ok(collisions)
}
