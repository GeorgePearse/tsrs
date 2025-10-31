//! Virtual environment analysis

use crate::error::{Result, TsrsError};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Information about a Python virtual environment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VenvInfo {
    /// Path to the venv
    pub path: PathBuf,
    /// Python version (if detectable)
    pub python_version: Option<String>,
    /// List of installed packages
    pub packages: Vec<PackageInfo>,
}

/// Information about an installed package
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct PackageInfo {
    /// Package name
    pub name: String,
    /// Package version
    pub version: Option<String>,
    /// Path to the package
    pub path: PathBuf,
}

/// Analyzes Python virtual environments
pub struct VenvAnalyzer {
    venv_path: PathBuf,
}

impl VenvAnalyzer {
    /// Create a new venv analyzer
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let venv_path = path.as_ref().to_path_buf();

        // Validate that this looks like a venv
        if !venv_path.exists() {
            return Err(TsrsError::InvalidVenvPath(format!(
                "Venv path does not exist: {}",
                venv_path.display()
            )));
        }

        Ok(VenvAnalyzer { venv_path })
    }

    /// Analyze the venv and collect package information
    pub fn analyze(&self) -> Result<VenvInfo> {
        let site_packages_path = self.find_site_packages()?;
        let packages = self.discover_packages(&site_packages_path)?;

        Ok(VenvInfo {
            path: self.venv_path.clone(),
            python_version: self.detect_python_version(),
            packages,
        })
    }

    /// Find the site-packages directory
    fn find_site_packages(&self) -> Result<PathBuf> {
        let lib_path = self.venv_path.join("lib");

        if !lib_path.exists() {
            return Err(TsrsError::InvalidVenvPath(
                "No lib directory found in venv".to_string(),
            ));
        }

        // Look for pythonX.Y/site-packages
        for entry in std::fs::read_dir(&lib_path)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                if name.starts_with("python") {
                    let site_packages = path.join("site-packages");
                    if site_packages.exists() {
                        return Ok(site_packages);
                    }
                }
            }
        }

        Err(TsrsError::InvalidVenvPath(
            "Could not find site-packages directory".to_string(),
        ))
    }

    /// Discover all installed packages
    fn discover_packages(&self, site_packages: &Path) -> Result<Vec<PackageInfo>> {
        let mut packages = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for entry in std::fs::read_dir(site_packages)? {
            let entry = entry?;
            let path = entry.path();
            let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();

            // Skip special directories
            if name.starts_with("_") || name.starts_with(".") || name == "dist-info" {
                continue;
            }

            if path.is_dir() && !seen.contains(&name) {
                // Check if it has __init__.py (is a package)
                if path.join("__init__.py").exists() || name.ends_with(".dist-info") {
                    let version = self.extract_version(&name);
                    packages.push(PackageInfo {
                        name: name.clone(),
                        version,
                        path: path.clone(),
                    });
                    seen.insert(name);
                }
            }
        }

        packages.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(packages)
    }

    /// Extract version from dist-info directory name
    fn extract_version(&self, name: &str) -> Option<String> {
        if name.ends_with(".dist-info") {
            let parts: Vec<&str> = name.rsplitn(2, '-').collect();
            if parts.len() == 2 {
                return Some(parts[0].to_string());
            }
        }
        None
    }

    /// Try to detect the Python version from the venv
    fn detect_python_version(&self) -> Option<String> {
        let lib_path = self.venv_path.join("lib");
        if let Ok(entries) = std::fs::read_dir(&lib_path) {
            for entry in entries.flatten() {
                let name = entry
                    .file_name()
                    .to_string_lossy()
                    .to_string();
                if name.starts_with("python") {
                    return Some(name);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invalid_venv_path() {
        let result = VenvAnalyzer::new("/nonexistent/path");
        assert!(result.is_err());
    }
}
