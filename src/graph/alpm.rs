use std::path::PathBuf;

use anyhow::Result;

pub trait DependencyProvider {
    fn get_deps(&self, package: &str) -> Result<Vec<String>>;
}

pub struct AlpmProvider {
    _pacman_conf: PathBuf,
}

impl AlpmProvider {
    pub fn new(pacman_conf: PathBuf) -> Self {
        Self { _pacman_conf: pacman_conf }
    }
}

impl DependencyProvider for AlpmProvider {
    fn get_deps(&self, _package: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
}

#[cfg(test)]
pub struct MockDependencyProvider {
    deps: std::collections::HashMap<String, Vec<String>>,
}

#[cfg(test)]
impl MockDependencyProvider {
    pub fn new() -> Self {
        Self {
            deps: std::collections::HashMap::new(),
        }
    }

    pub fn insert(&mut self, package: &str, dependencies: &[&str]) {
        self.deps.insert(
            package.to_string(),
            dependencies.iter().map(|s| s.to_string()).collect(),
        );
    }
}

#[cfg(test)]
impl Default for MockDependencyProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
impl DependencyProvider for MockDependencyProvider {
    fn get_deps(&self, package: &str) -> Result<Vec<String>> {
        Ok(self.deps.get(package).cloned().unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_returns_inserted_deps() {
        let mut mock = MockDependencyProvider::new();
        mock.insert("base", &["glibc", "gcc-libs"]);

        let deps = mock.get_deps("base").unwrap();
        assert_eq!(deps, vec!["glibc", "gcc-libs"]);
    }

    #[test]
    fn mock_returns_empty_for_unknown() {
        let mock = MockDependencyProvider::new();
        assert!(mock.get_deps("nonexistent").unwrap().is_empty());
    }
}
