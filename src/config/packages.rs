use anyhow::{bail, Context, Result};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

pub fn parse_package_list(path: &Path) -> Result<HashSet<String>> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("Failed to read package list at {}", path.display()))?;

    Ok(parse_package_list_str(&contents))
}

pub fn parse_package_list_str(contents: &str) -> HashSet<String> {
    contents
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.to_string())
        .collect()
}

pub fn check_deduplication(
    bootstrap: &HashSet<String>,
    packages: &HashSet<String>,
) -> Result<()> {
    let duplicates: Vec<&String> = bootstrap.intersection(packages).collect();

    if duplicates.is_empty() {
        return Ok(());
    }

    let mut sorted: Vec<&str> = duplicates.iter().map(|s| s.as_str()).collect();
    sorted.sort();

    bail!(
        "The following packages appear in both bootstrap_packages and packages.x86_64: {}",
        sorted.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_package_list_str() {
        let input = "# Core packages\nbase\nlinux\n\n# Extra\ntmux\nvim\n";
        let pkgs = parse_package_list_str(input);
        assert_eq!(pkgs.len(), 4);
        assert!(pkgs.contains("base"));
        assert!(pkgs.contains("linux"));
        assert!(pkgs.contains("tmux"));
        assert!(pkgs.contains("vim"));
    }

    #[test]
    fn test_parse_package_list_str_trims_whitespace() {
        let input = "  base  \n  linux  \n";
        let pkgs = parse_package_list_str(input);
        assert!(pkgs.contains("base"));
        assert!(pkgs.contains("linux"));
    }

    #[test]
    fn test_parse_empty_input() {
        let pkgs = parse_package_list_str("");
        assert!(pkgs.is_empty());
    }

    #[test]
    fn test_parse_only_comments() {
        let pkgs = parse_package_list_str("# just a comment\n# another comment\n");
        assert!(pkgs.is_empty());
    }

    #[test]
    fn test_deduplication_no_overlap() {
        let a: HashSet<String> = ["base", "linux"].into_iter().map(String::from).collect();
        let b: HashSet<String> = ["tmux", "vim"].into_iter().map(String::from).collect();
        assert!(check_deduplication(&a, &b).is_ok());
    }

    #[test]
    fn test_deduplication_with_overlap() {
        let a: HashSet<String> = ["base", "linux"].into_iter().map(String::from).collect();
        let b: HashSet<String> = ["linux", "vim"].into_iter().map(String::from).collect();
        let err = check_deduplication(&a, &b).unwrap_err();
        assert!(err.to_string().contains("linux"));
    }
}
