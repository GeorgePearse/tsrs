//! Function call graph analysis per package

use crate::error::{Result, TsrsError};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Represents a function or class reference
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FunctionRef {
    /// Package name (top-level module)
    pub package: String,
    /// Function or class name
    pub name: String,
}

impl FunctionRef {
    /// Create a new function reference
    #[must_use]
    pub fn new(package: String, name: String) -> Self {
        FunctionRef { package, name }
    }

    /// Check if this is a standard library or builtin
    #[must_use]
    pub fn is_builtin(&self) -> bool {
        matches!(
            self.package.as_str(),
            "builtins" | "sys" | "os" | "__builtin__" | "__builtins__"
        )
    }
}

/// Call graph for a single package
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageCallGraph {
    /// Package name
    pub package: String,
    /// Functions/classes defined in this package
    pub definitions: HashSet<String>,
    /// Functions/classes used from other packages
    pub external_calls: HashSet<FunctionRef>,
    /// Functions/classes used locally (from same package)
    pub internal_calls: HashSet<String>,
}

impl PackageCallGraph {
    /// Create a new package call graph
    #[must_use]
    pub fn new(package: String) -> Self {
        PackageCallGraph {
            package,
            definitions: HashSet::new(),
            external_calls: HashSet::new(),
            internal_calls: HashSet::new(),
        }
    }

    /// Add a function definition
    pub fn add_definition(&mut self, name: String) {
        self.definitions.insert(name);
    }

    /// Add an external call
    pub fn add_external_call(&mut self, call: FunctionRef) {
        self.external_calls.insert(call);
    }

    /// Add an internal call
    pub fn add_internal_call(&mut self, name: String) {
        self.internal_calls.insert(name);
    }
}

/// Analyzes function calls per package
pub struct CallGraphAnalyzer {
    graphs: HashMap<String, PackageCallGraph>,
    function_pattern: Regex,
    call_pattern: Regex,
    _import_pattern: Regex,
}

impl CallGraphAnalyzer {
    /// Create a new call graph analyzer
    ///
    /// # Errors
    ///
    /// Returns an error if regex compilation fails.
    pub fn new() -> Result<Self> {
        let function_pattern = Regex::new(r"^\s*(?:async\s+)?def\s+([a-zA-Z_][a-zA-Z0-9_]*)")
            .map_err(|e| TsrsError::ParseError(format!("Failed to compile regex: {e}")))?;

        let call_pattern = Regex::new(r"\b([a-zA-Z_][a-zA-Z0-9_\.]*)\s*\(")
            .map_err(|e| TsrsError::ParseError(format!("Failed to compile regex: {e}")))?;

        let import_pattern =
            Regex::new(r"^(?:from\s+([a-zA-Z0-9_\.]+)\s+)?import\s+([a-zA-Z0-9_,\s]+)")
                .map_err(|e| TsrsError::ParseError(format!("Failed to compile regex: {e}")))?;

        Ok(CallGraphAnalyzer {
            graphs: HashMap::new(),
            function_pattern,
            call_pattern,
            _import_pattern: import_pattern,
        })
    }

    /// Analyze a Python file and build call graph
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read.
    pub fn analyze_file<P: AsRef<Path>>(&mut self, path: P, package: &str) -> Result<()> {
        let source = std::fs::read_to_string(path).map_err(TsrsError::Io)?;
        self.analyze_source(package, &source);
        Ok(())
    }

    /// Analyze Python source and extract function definitions and calls
    pub fn analyze_source(&mut self, package: &str, source: &str) {
        let graph = self
            .graphs
            .entry(package.to_string())
            .or_insert_with(|| PackageCallGraph::new(package.to_string()));

        for line in source.lines() {
            // Skip comments and empty lines
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }

            // Extract function definitions
            let is_def_line = line.starts_with("def ");
            if is_def_line {
                if let Some(caps) = self.function_pattern.captures(line) {
                    if let Some(func_name) = caps.get(1) {
                        graph.add_definition(func_name.as_str().to_string());
                    }
                }
                // Skip extracting calls from def lines
                continue;
            }

            // Extract function calls (but not from def lines)
            for caps in self.call_pattern.captures_iter(line) {
                if let Some(call_expr) = caps.get(1) {
                    let call_str = call_expr.as_str();
                    let parts: Vec<&str> = call_str.split('.').collect();

                    if parts.len() == 1 {
                        // Local call
                        graph.add_internal_call(parts[0].to_string());
                    } else if parts.len() >= 2 {
                        // External call
                        let module = parts[0];
                        let func = parts[1..].join(".");
                        graph.add_external_call(FunctionRef::new(module.to_string(), func));
                    }
                }
            }
        }
    }

    /// Get all call graphs
    #[must_use]
    pub fn get_graphs(&self) -> &HashMap<String, PackageCallGraph> {
        &self.graphs
    }

    /// Get call graph for a specific package
    #[must_use]
    pub fn get_graph(&self, package: &str) -> Option<&PackageCallGraph> {
        self.graphs.get(package)
    }

    /// Find unused functions in a package
    #[must_use]
    pub fn find_unused_functions(&self, package: &str) -> HashSet<String> {
        if let Some(graph) = self.get_graph(package) {
            let called: HashSet<String> = graph.internal_calls.iter().cloned().collect();
            graph
                .definitions
                .iter()
                .filter(|def| !called.contains(*def) && !def.starts_with('_'))
                .cloned()
                .collect()
        } else {
            HashSet::new()
        }
    }

    /// Find all external dependencies
    #[must_use]
    pub fn find_external_dependencies(&self) -> HashSet<String> {
        let mut deps = HashSet::new();
        for graph in self.graphs.values() {
            for call in &graph.external_calls {
                deps.insert(call.package.clone());
            }
        }
        deps
    }
}

impl Default for CallGraphAnalyzer {
    fn default() -> Self {
        Self::new().expect("Failed to create CallGraphAnalyzer")
    }
}
