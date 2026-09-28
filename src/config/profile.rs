use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePermission {
    pub path: String,
    pub owner: String,
    pub mode: String,
}

#[derive(Debug, Clone)]
pub struct ProfileConfig {
    pub iso_name: String,
    pub iso_label: String,
    pub iso_publisher: String,
    pub iso_application: String,
    pub iso_version: String,
    pub install_dir: String,
    pub file_permissions: Vec<FilePermission>,
}

impl ProfileConfig {
    pub fn from_file(path: &Path) -> Result<Self> {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("Failed to read profiledef.sh at {}", path.display()))?;
        Self::parse(&contents, &path.display().to_string())
    }

    pub fn parse(contents: &str, source: &str) -> Result<Self> {
        let mut iso_name = None;
        let mut iso_label = None;
        let mut iso_publisher = None;
        let mut iso_application = None;
        let mut iso_version = None;
        let mut install_dir = None;
        let mut file_permissions = Vec::new();
        let mut in_permissions_array = false;

        for line in contents.lines() {
            let trimmed = line.trim();

            if trimmed.is_empty() || trimmed.starts_with('#') {
                if in_permissions_array && trimmed.starts_with(')') {
                    in_permissions_array = false;
                }
                continue;
            }

            if in_permissions_array {
                if trimmed.starts_with(')') {
                    in_permissions_array = false;
                    continue;
                }
                if let Some(entry) = parse_permission_tuple(trimmed) {
                    file_permissions.push(entry);
                }
                continue;
            }

            if let Some(rest) = trimmed.strip_prefix("file_permissions") {
                let rest = rest.trim();
                if let Some(after_eq) = rest.strip_prefix('=') {
                    let after_eq = after_eq.trim();
                    if after_eq == "(" {
                        in_permissions_array = true;
                        continue;
                    }
                }
            }

            if let Some((key, value)) = parse_assignment(trimmed) {
                let val = value.to_string();
                match key {
                    "iso_name" => iso_name = Some(val),
                    "iso_label" => iso_label = Some(val),
                    "iso_publisher" => iso_publisher = Some(val),
                    "iso_application" => iso_application = Some(val),
                    "iso_version" => iso_version = Some(val),
                    "install_dir" => install_dir = Some(val),
                    _ => {}
                }
            }
        }

        Ok(Self {
            iso_name: iso_name
                .ok_or_else(|| anyhow::anyhow!("Missing 'iso_name' in {}", source))?,
            iso_label: iso_label
                .filter(|s| !s.contains("$("))
                .unwrap_or_else(|| format!("ARCH_{}", chrono_fallback_date())),
            iso_publisher: iso_publisher
                .ok_or_else(|| anyhow::anyhow!("Missing 'iso_publisher' in {}", source))?,
            iso_application: iso_application
                .ok_or_else(|| anyhow::anyhow!("Missing 'iso_application' in {}", source))?,
            iso_version: iso_version
                .filter(|s| !s.contains("$("))
                .unwrap_or_else(|| chrono_fallback_date()),
            install_dir: install_dir.unwrap_or_else(|| "arch".to_string()),
            file_permissions,
        })
    }
}

fn parse_assignment(line: &str) -> Option<(&str, &str)> {
    let eq_pos = line.find('=')?;
    let key = line[..eq_pos].trim();
    let value_part = line[eq_pos + 1..].trim();

    if let Some(stripped) = strip_surrounding(value_part, '"', '"') {
        return Some((key, stripped));
    }

    if let Some(stripped) = strip_surrounding_single(value_part) {
        return Some((key, stripped));
    }

    Some((key, value_part))
}

fn strip_surrounding<'a>(text: &'a str, open: char, close: char) -> Option<&'a str> {
    if text.starts_with(open) && text.ends_with(close) && text.len() >= 2 {
        Some(&text[1..text.len() - 1])
    } else {
        None
    }
}

fn strip_surrounding_single<'a>(text: &'a str) -> Option<&'a str> {
    strip_surrounding(text, '\'', '\'')
}

fn parse_permission_tuple(line: &str) -> Option<FilePermission> {
    let line = line.trim();

    if let Some(rest) = line.strip_prefix('[') {
        if let Some(bracket_end) = rest.find(']') {
            let key = &rest[..bracket_end];
            let remainder = &rest[bracket_end + 1..];
            if let Some(eq_pos) = remainder.find('=') {
                let after_eq = remainder[eq_pos + 1..].trim();
                if let Some(val) = strip_surrounding(after_eq, '"', '"') {
                    let path = strip_surrounding(key, '"', '"').unwrap_or(key);
                    let parts: Vec<&str> = val.split(':').collect();
                    let owner = if parts.len() >= 2 {
                        format!("{}:{}", parts[0], parts[1])
                    } else {
                        "0:0".to_string()
                    };
                    let mode = if parts.len() >= 3 {
                        parts[2].to_string()
                    } else {
                        "0644".to_string()
                    };
                    return Some(FilePermission {
                        path: path.to_string(),
                        owner,
                        mode,
                    });
                }
            }
        }
    }

    let inner = line.strip_prefix('(')?.strip_suffix(')')?;
    let inner = inner.trim();

    let segments: Vec<&str> = inner
        .split_whitespace()
        .map(|s| strip_surrounding(s, '"', '"').or_else(|| strip_surrounding_single(s)))
        .collect::<Option<Vec<_>>>()?;

    if segments.len() != 3 {
        return None;
    }

    Some(FilePermission {
        path: segments[0].to_string(),
        owner: segments[1].to_string(),
        mode: segments[2].to_string(),
    })
}

fn chrono_fallback_date() -> String {
    use std::time::SystemTime;
    let Ok(duration) = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) else {
        return "unknown".to_string();
    };
    let secs = duration.as_secs();
    let days = secs / 86400;

    let mut remaining = days;
    let mut year = 1970u64;
    loop {
        let leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
        let year_days = if leap { 366 } else { 365 };
        if remaining < year_days {
            break;
        }
        remaining -= year_days;
        year += 1;
    }

    let mut month = 1u64;
    let days_in_month = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for (i, &dim) in days_in_month.iter().enumerate() {
        let dim = if i == 1 && ((year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)) {
            dim + 1
        } else {
            dim
        };
        if remaining < dim {
            break;
        }
        remaining -= dim;
        month += 1;
    }
    let day = remaining + 1;

    format!("{:04}{:02}{:02}", year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_profiledef() {
        let input = r#"
iso_name="archlinux"
iso_label="ARCH_20230601"
iso_publisher="Arch Linux <https://archlinux.org>"
iso_application="Arch Linux Live/Rescue CD"
iso_version="20230601"
file_permissions=(
    ("etc/shadow" "0:0" "0400")
    ("etc/passwd" "0:0" "0644")
)
"#;
        let config = ProfileConfig::parse(input, "test_profiledef.sh").unwrap();
        assert_eq!(config.iso_name, "archlinux");
        assert_eq!(config.iso_label, "ARCH_20230601");
        assert_eq!(
            config.iso_publisher,
            "Arch Linux <https://archlinux.org>"
        );
        assert_eq!(config.iso_application, "Arch Linux Live/Rescue CD");
        assert_eq!(config.iso_version, "20230601");
        assert_eq!(config.file_permissions.len(), 2);
        assert_eq!(config.file_permissions[0].path, "etc/shadow");
        assert_eq!(config.file_permissions[0].owner, "0:0");
        assert_eq!(config.file_permissions[0].mode, "0400");
        assert_eq!(config.file_permissions[1].path, "etc/passwd");
        assert_eq!(config.file_permissions[1].owner, "0:0");
        assert_eq!(config.file_permissions[1].mode, "0644");
    }

    #[test]
    fn test_missing_field_errors() {
        let input = r#"
iso_name="archlinux"
iso_label="ARCH_20230601"
"#;
        let result = ProfileConfig::parse(input, "test.sh");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Missing 'iso_publisher'"));
    }

    #[test]
    fn test_single_quoted_permissions() {
        let input = r#"
iso_name="test"
iso_label="TEST"
iso_publisher="Test"
iso_application="Test App"
iso_version="1.0"
file_permissions=(
    ('etc/shadow' '0:0' '0400')
)
"#;
        let config = ProfileConfig::parse(input, "test.sh").unwrap();
        assert_eq!(config.file_permissions.len(), 1);
        assert_eq!(config.file_permissions[0].mode, "0400");
    }

    #[test]
    fn test_parse_profiledef_associative_array() {
        let input = r#"
iso_name="archlinux"
iso_label="ARCH_20240101"
iso_publisher="Arch Linux <https://archlinux.org>"
iso_application="Arch Linux Live/Rescue DVD"
iso_version="2024.01.01"
file_permissions=(
  ["/etc/shadow"]="0:0:400"
  ["/root"]="0:0:750"
  ["/usr/local/bin/choose-mirror"]="0:0:755"
)
"#;
        let config = ProfileConfig::parse(input, "releng_profiledef.sh").unwrap();
        assert_eq!(config.iso_name, "archlinux");
        assert_eq!(config.file_permissions.len(), 3);
        assert_eq!(config.file_permissions[0].path, "/etc/shadow");
        assert_eq!(config.file_permissions[0].owner, "0:0");
        assert_eq!(config.file_permissions[0].mode, "400");
        assert_eq!(config.file_permissions[1].path, "/root");
        assert_eq!(config.file_permissions[1].owner, "0:0");
        assert_eq!(config.file_permissions[1].mode, "750");
        assert_eq!(config.file_permissions[2].path, "/usr/local/bin/choose-mirror");
        assert_eq!(config.file_permissions[2].mode, "755");
    }

    #[test]
    fn test_parse_actual_releng_profiledef() {
        let input = r#"#!/usr/bin/env bash
# shellcheck disable=SC2034

iso_name="archlinux"
iso_label="ARCH_$(date --date="@${SOURCE_DATE_EPOCH:-$(date +%s)}" +%Y%m)"
iso_publisher="Arch Linux <https://archlinux.org>"
iso_application="Arch Linux Live/Rescue DVD"
iso_version="$(date --date="@${SOURCE_DATE_EPOCH:-$(date +%s)}" +%Y.%m.%d)"
install_dir="arch"
buildmodes=('iso')
bootmodes=('bios.syslinux'
           'uefi.systemd-boot')
pacman_conf="pacman.conf"
airootfs_image_type="squashfs"
airootfs_image_tool_options=('-comp' 'xz' '-Xbcj' 'x86,arm64' '-b' '1M' '-Xdict-size' '1M')
bootstrap_tarball_compression=('zstd' '-c' '-T0' '--auto-threads=logical' '--long' '-19')
file_permissions=(
  ["/etc/shadow"]="0:0:400"
  ["/root"]="0:0:750"
  ["/root/.automated_script.sh"]="0:0:755"
  ["/root/.gnupg"]="0:0:700"
  ["/usr/local/bin/choose-mirror"]="0:0:755"
  ["/usr/local/bin/Installation_guide"]="0:0:755"
  ["/usr/local/bin/livecd-sound"]="0:0:755"
)
"#;
        let config = ProfileConfig::parse(input, "releng/profiledef.sh").unwrap();
        assert_eq!(config.iso_name, "archlinux");
        assert!(config.iso_label.starts_with("ARCH_"));
        assert_eq!(config.iso_publisher, "Arch Linux <https://archlinux.org>");
        assert_eq!(config.iso_application, "Arch Linux Live/Rescue DVD");
        assert_eq!(config.file_permissions.len(), 7);
        assert_eq!(config.file_permissions[0].path, "/etc/shadow");
        assert_eq!(config.file_permissions[0].owner, "0:0");
        assert_eq!(config.file_permissions[0].mode, "400");
        assert_eq!(config.file_permissions[6].path, "/usr/local/bin/livecd-sound");
    }

    #[test]
    fn test_no_permissions_is_fine() {
        let input = r#"
iso_name="test"
iso_label="TEST"
iso_publisher="Test"
iso_application="Test App"
iso_version="1.0"
"#;
        let config = ProfileConfig::parse(input, "test.sh").unwrap();
        assert!(config.file_permissions.is_empty());
    }
}
