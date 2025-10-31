//! Import tracking and collection

use crate::error::{Result, TsrsError};
use rustpython_parser::{ast, Parse};
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
    #[must_use]
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
    #[must_use]
    pub fn get_imports(&self) -> Vec<String> {
        let mut imports: Vec<_> = self.imports.iter().cloned().collect();
        imports.sort();
        imports
    }
}

/// Collects imports from Python code via AST traversal
pub struct ImportCollector {
    imports: ImportSet,
}

impl ImportCollector {
    /// Create a new import collector
    #[must_use]
    pub fn new() -> Self {
        ImportCollector {
            imports: ImportSet::new(),
        }
    }

    /// Parse a Python file and extract imports
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or parsed.
    pub fn collect_from_file<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let path_ref = path.as_ref();
        let source = std::fs::read_to_string(path_ref).map_err(TsrsError::Io)?;
        let filename = path_ref.display().to_string();
        self.collect_from_source_with_name(&source, &filename)?;
        Ok(())
    }

    /// Parse Python source code and extract imports
    ///
    /// # Errors
    ///
    /// Returns an error if the source cannot be parsed.
    pub fn collect_from_source(&mut self, source: &str) -> Result<()> {
        self.collect_from_source_with_name(source, "<memory>")?;
        Ok(())
    }

    fn collect_from_source_with_name(&mut self, source: &str, filename: &str) -> Result<()> {
        let suite = ast::Suite::parse(source, filename)
            .map_err(|err| TsrsError::ParseError(err.to_string()))?;
        self.visit_suite(&suite);
        Ok(())
    }

    fn visit_suite(&mut self, suite: &[ast::Stmt]) {
        for stmt in suite {
            self.visit_stmt(stmt);
        }
    }

    fn visit_stmt(&mut self, stmt: &ast::Stmt) {
        match stmt {
            ast::Stmt::Import(import) => self.handle_import(import),
            ast::Stmt::ImportFrom(import_from) => self.handle_import_from(import_from),
            ast::Stmt::FunctionDef(function_def) => self.visit_suite(&function_def.body),
            ast::Stmt::AsyncFunctionDef(function_def) => self.visit_suite(&function_def.body),
            ast::Stmt::ClassDef(class_def) => self.visit_suite(&class_def.body),
            ast::Stmt::For(for_stmt) => {
                self.visit_suite(&for_stmt.body);
                self.visit_suite(&for_stmt.orelse);
            }
            ast::Stmt::AsyncFor(for_stmt) => {
                self.visit_suite(&for_stmt.body);
                self.visit_suite(&for_stmt.orelse);
            }
            ast::Stmt::While(while_stmt) => {
                self.visit_suite(&while_stmt.body);
                self.visit_suite(&while_stmt.orelse);
            }
            ast::Stmt::If(if_stmt) => {
                self.visit_suite(&if_stmt.body);
                self.visit_suite(&if_stmt.orelse);
            }
            ast::Stmt::With(with_stmt) => self.visit_suite(&with_stmt.body),
            ast::Stmt::AsyncWith(with_stmt) => self.visit_suite(&with_stmt.body),
            ast::Stmt::Try(try_stmt) => {
                self.visit_suite(&try_stmt.body);
                self.visit_suite(&try_stmt.orelse);
                self.visit_suite(&try_stmt.finalbody);
                for handler in &try_stmt.handlers {
                    let ast::ExceptHandler::ExceptHandler(handler) = handler;
                    self.visit_suite(&handler.body);
                }
            }
            ast::Stmt::TryStar(try_stmt) => {
                self.visit_suite(&try_stmt.body);
                self.visit_suite(&try_stmt.orelse);
                self.visit_suite(&try_stmt.finalbody);
                for handler in &try_stmt.handlers {
                    let ast::ExceptHandler::ExceptHandler(handler) = handler;
                    self.visit_suite(&handler.body);
                }
            }
            ast::Stmt::Match(match_stmt) => {
                for case in &match_stmt.cases {
                    self.visit_suite(&case.body);
                }
            }
            _ => {}
        }
    }

    fn handle_import(&mut self, import: &ast::StmtImport) {
        for alias in &import.names {
            self.add_identifier_name(&alias.name);
        }
    }

    fn handle_import_from(&mut self, import_from: &ast::StmtImportFrom) {
        let level = import_from.level.as_ref().map_or(0, ast::Int::to_u32);

        if level > 0 {
            // Relative imports refer to the current package; skip to avoid
            // incorrectly attributing them to external dependencies.
            return;
        }

        if let Some(module) = &import_from.module {
            self.add_identifier_name(module);
        } else {
            // Absolute import without explicit module (rare). Fall back to alias names.
            for alias in &import_from.names {
                self.add_identifier_name(&alias.name);
            }
        }
    }

    fn add_identifier_name(&mut self, identifier: &ast::Identifier) {
        self.add_module_name(identifier.as_str());
    }

    fn add_module_name(&mut self, name: &str) {
        let top_level = name.split('.').next().unwrap_or(name);
        if top_level.is_empty() || top_level == "*" {
            return;
        }
        self.imports.add(top_level.to_string());
    }

    /// Get collected imports
    #[must_use]
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

    fn imports_from(source: &str) -> Vec<String> {
        let mut collector = ImportCollector::new();
        collector
            .collect_from_source(source)
            .expect("import collection should succeed");
        collector.get_imports().get_imports()
    }

    #[test]
    fn collects_top_level_modules() {
        let imports = imports_from(
            r#"
import os
import sys as system
import numpy.linalg
from collections import defaultdict
from pandas import (DataFrame, Series)
"#,
        );

        assert_eq!(
            imports,
            vec![
                "collections".to_string(),
                "numpy".to_string(),
                "os".to_string(),
                "pandas".to_string(),
                "sys".to_string()
            ]
        );
    }

    #[test]
    fn skips_relative_imports() {
        let imports = imports_from(
            r#"
from . import local_module
from ..package import feature
"#,
        );

        assert!(imports.is_empty());
    }

    #[test]
    fn ignores_duplicates_and_aliases() {
        let imports = imports_from(
            r#"
import os
import os.path as osp
from os import path
"#,
        );

        assert_eq!(imports, vec!["os".to_string()]);
    }
}
