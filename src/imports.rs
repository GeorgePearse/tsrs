//! Import tracking and collection

use crate::error::{Result, TsrsError};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

/// Set of unique imports
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ImportSet {
    /// Imported module names
    pub imports: HashSet<String>,
}

impl ImportSet {
    /// Create a new import set
    pub fn new() -> Self {
        ImportSet {
            imports: HashSet::new(),
        }
    }

    /// Add an import
    pub fn add(&mut self, import: String) {
        self.imports.insert(import);
    }

    /// Get all imports
    pub fn get_imports(&self) -> Vec<String> {
        let mut imports: Vec<_> = self.imports.iter().cloned().collect();
        imports.sort();
        imports
    }
}

/// Collects imports from Python code using regex patterns
pub struct ImportCollector {
    imports: ImportSet,
    import_pattern: Regex,
    from_import_pattern: Regex,
}

impl ImportCollector {
    /// Create a new import collector
    pub fn new() -> Self {
        let import_pattern = Regex::new(r"^\s*import\s+([a-zA-Z0-9_,.\s]+)")
            .expect("Failed to compile import pattern");
        let from_import_pattern = Regex::new(r"^\s*from\s+([a-zA-Z0-9_\.]+)\s+import")
            .expect("Failed to compile from import pattern");

        ImportCollector {
            imports: ImportSet::new(),
            import_pattern,
            from_import_pattern,
        }
    }

    /// Parse a Python file and extract imports
    pub fn collect_from_file<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let source = std::fs::read_to_string(path)
            .map_err(|e| TsrsError::Io(e))?;
        self.collect_from_source(&source)?;
        Ok(())
    }

    /// Parse Python source code and extract imports
    pub fn collect_from_source(&mut self, source: &str) -> Result<()> {
        for line in source.lines() {
            self.extract_imports_from_line(line);
        }
        Ok(())
    }

    /// Extract imports from a single line
    fn extract_imports_from_line(&mut self, line: &str) {
        // Skip comments
        let line = line.split('#').next().unwrap_or("");

        // Handle "import X" statements
        if let Some(caps) = self.import_pattern.captures(line) {
            if let Some(imports) = caps.get(1) {
                for import in imports.as_str().split(',') {
                    let import = import.trim();
                    let top_level = import.split('.').next().unwrap_or(import).trim();
                    if !top_level.is_empty() && !top_level.contains('(') {
                        self.imports.add(top_level.to_string());
                    }
                }
            }
        }

        // Handle "from X import Y" statements
        if let Some(caps) = self.from_import_pattern.captures(line) {
            if let Some(module) = caps.get(1) {
                let module_name = module.as_str();
                let top_level = module_name.split('.').next().unwrap_or(module_name);
                self.imports.add(top_level.to_string());
            }
        }
    }

    /// Get collected imports
    pub fn get_imports(&self) -> ImportSet {
        self.imports.clone()
    }
}

impl Default for ImportCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collect_simple_import() {
        let mut collector = ImportCollector::new();
        collector.collect_from_source("import os").unwrap();
        let imports = collector.get_imports();
        assert!(imports.imports.contains("os"));
    }

    #[test]
    fn test_collect_from_import() {
        let mut collector = ImportCollector::new();
        collector.collect_from_source("from os import path").unwrap();
        let imports = collector.get_imports();
        assert!(imports.imports.contains("os"));
    }

    #[test]
    fn test_collect_multiple_imports() {
        let mut collector = ImportCollector::new();
        collector.collect_from_source(
            r#"
import os
import sys
from collections import defaultdict
            "#
        ).unwrap();
        let imports = collector.get_imports();
        assert!(imports.imports.contains("os"));
        assert!(imports.imports.contains("sys"));
        assert!(imports.imports.contains("collections"));
    }
}
