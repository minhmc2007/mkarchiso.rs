use anyhow::{Context, Result};
use std::process::{Command, Output, Stdio};

pub fn run_command(cmd: &str, args: &[&str], _label: &str) -> Result<Output> {
    let output = Command::new(cmd)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("Failed to execute '{}' for {}", cmd, _label))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        tracing::error!(
            cmd = cmd,
            label = _label,
            exit_code = ?output.status.code(),
            stdout = %stdout.trim_end(),
            stderr = %stderr.trim_end(),
            "Command failed"
        );
        anyhow::bail!(
            "Command '{}' failed (exit code {:?}) for {}",
            cmd,
            output.status.code(),
            _label
        );
    }

    Ok(output)
}

pub fn binary_exists(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
