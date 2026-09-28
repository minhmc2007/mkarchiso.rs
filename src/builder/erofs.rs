use anyhow::{Context, Result};
use std::path::Path;

use crate::util::cmd::run_command;

pub fn build_erofs(merged_dir: &Path, erofs_output: &Path) -> Result<()> {
    tracing::info!(
        output = %erofs_output.display(),
        source = %merged_dir.display(),
        "Building EROFS image"
    );

    let args: Vec<&str> = vec![
        "-zlz4hc,9",
        "-E", "all-fragments,dedupe",
        "-T", "0",
        "-U", "00000000-0000-0000-0000-000000000000",
        erofs_output.to_str().context("erofs_output not valid UTF-8")?,
        merged_dir.to_str().context("merged_dir not valid UTF-8")?,
    ];

    run_command("mkfs.erofs", &args, "erofs-build")
        .with_context(|| format!("mkfs.erofs failed for {}", erofs_output.display()))?;

    tracing::info!(output = %erofs_output.display(), "EROFS image built");

    Ok(())
}
