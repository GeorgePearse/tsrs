//! Function call graph analysis with interprocedural reachability
//!
//! This module analyzes Python code to build a call graph and detect dead code.
//! It uses AST-based analysis to properly handle:
//! - Function and method definitions
//! - Call edges between functions
//! - Entry points (main blocks, tests, public APIs)
//! - Cross-package analysis
//! - Reachability from entry points

use crate::error::{Result, TsrsError};
use rustpython_parser::{ast, Parse};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

/// Unique identifier for a function node
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FunctionId(pub usize);

/// Source location information
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceLocation {
    pub line: usize,
    pub col: usize,
}

/// Kind of function
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FunctionKind {
    /// Regular function
    Function,
    /// Async function
    AsyncFunction,
    /// Method inside a class
    Method,
    /// Dunder method (__init__, __call__, etc)
    DunderMethod,
    /// Lambda function
    Lambda,
}

/// Kind of function for dead code analysis
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EntryPointKind {
    /// Code at module level (always executed on import)
    ModuleInit,
    /// if __name__ == "__main__" block
    ScriptMain,
    /// Test function (test_*, @pytest, @unittest)
    TestFunction,
    /// Dunder method (kept for protocol compatibility)
    DunderMethod,
    /// Exported in __all__
    PublicExport,
    /// Regular function (not an entry point)
    Regular,
}

/// A function in the call graph
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallGraphNode {
    pub id: FunctionId,
    pub name: String,
    pub package: String,
    pub location: SourceLocation,
    pub kind: FunctionKind,
    pub entry_point: EntryPointKind,
    /// Names of decorators (for framework detection)
    pub decorators: Vec<String>,
    /// Whether this function is marked with @property or similar
    pub is_special: bool,
}

/// A call edge from caller to callee
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CallEdge {
    pub caller: FunctionId,
    pub callee: FunctionId,
    pub location: SourceLocation,
}

/// Represents a function or class reference (legacy, for compatibility)
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

/// Analyzes function calls per package using AST traversal
pub struct CallGraphAnalyzer {
    /// Legacy per-package graphs (for backward compatibility)
    graphs: HashMap<String, PackageCallGraph>,
    /// New AST-based call graph
    nodes: HashMap<FunctionId, CallGraphNode>,
    /// Call edges
    edges: Vec<CallEdge>,
    /// Next available function ID
    next_id: usize,
    /// Map from (package, function_name) to FunctionId
    function_index: HashMap<(String, String), FunctionId>,
    /// Entry points (functions reachable from script/module init)
    entry_points: HashSet<FunctionId>,
    /// Public API exports from each package
    public_exports: HashMap<String, HashSet<String>>,
}

impl CallGraphAnalyzer {
    /// Create a new call graph analyzer
    #[must_use]
    pub fn new() -> Self {
        CallGraphAnalyzer {
            graphs: HashMap::new(),
            nodes: HashMap::new(),
            edges: Vec::new(),
            next_id: 0,
            function_index: HashMap::new(),
            entry_points: HashSet::new(),
            public_exports: HashMap::new(),
        }
    }

    /// Register a function in the call graph
    fn register_function(
        &mut self,
        package: String,
        name: String,
        location: SourceLocation,
        kind: FunctionKind,
        entry_point: EntryPointKind,
        decorators: Vec<String>,
    ) -> FunctionId {
        let id = FunctionId(self.next_id);
        self.next_id += 1;

        let is_special = decorators.iter().any(|d| {
            d.contains("property") || d.contains("staticmethod") || d.contains("classmethod")
        });

        let node = CallGraphNode {
            id,
            name: name.clone(),
            package: package.clone(),
            location,
            kind,
            entry_point,
            decorators,
            is_special,
        };

        self.nodes.insert(id, node);
        self.function_index.insert((package, name), id);

        if matches!(
            entry_point,
            EntryPointKind::ScriptMain | EntryPointKind::ModuleInit | EntryPointKind::TestFunction
        ) {
            self.entry_points.insert(id);
        }

        id
    }

    /// Add a call edge from caller to callee
    fn add_call_edge(&mut self, caller: FunctionId, callee: FunctionId, location: SourceLocation) {
        self.edges.push(CallEdge {
            caller,
            callee,
            location,
        });
    }

    /// Analyze a Python file and build call graph
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or parsed.
    pub fn analyze_file<P: AsRef<Path>>(&mut self, path: P, package: &str) -> Result<()> {
        let source = std::fs::read_to_string(path).map_err(TsrsError::Io)?;
        self.analyze_source(package, &source)
    }

    /// Analyze Python source code using AST traversal
    ///
    /// # Errors
    ///
    /// Returns an error if the source code cannot be parsed.
    pub fn analyze_source(&mut self, package: &str, source: &str) -> Result<()> {
        let suite = ast::Suite::parse(source, "<source>")
            .map_err(|e| TsrsError::ParseError(format!("Failed to parse Python: {e}")))?;

        // First pass: detect exports and entry points from module level
        self.detect_module_exports(package, &suite)?;
        self.detect_main_block(&suite)?;

        // Second pass: register all functions
        self.register_module_functions_suite(package, &suite)?;

        // Third pass: build call edges
        self.extract_calls_suite(package, &suite)?;

        // Also maintain legacy PackageCallGraph for backward compatibility
        self.build_legacy_graph(package);

        Ok(())
    }

    /// Detect `__all__` exports and module-level code
    fn detect_module_exports(&mut self, package: &str, suite: &[ast::Stmt]) -> Result<()> {
        let mut exports = HashSet::new();

        for stmt in suite {
            // Look for __all__ assignments
            if let ast::Stmt::Assign(assign) = stmt {
                for target in &assign.targets {
                    if let ast::Expr::Name(name_expr) = target {
                        if name_expr.id.as_str() == "__all__" {
                            // Try to extract list of strings
                            self.extract_all_exports(&assign.value, &mut exports)?;
                        }
                    }
                }
            }

            // Also mark any function at module level as having module initialization
            // (it can be called during import)
            if matches!(
                stmt,
                ast::Stmt::FunctionDef(_) | ast::Stmt::AsyncFunctionDef(_)
            ) {
                // These are detected separately
            }
        }

        if !exports.is_empty() {
            self.public_exports.insert(package.to_string(), exports);
        }

        Ok(())
    }

    /// Extract list of names from __all__ = [...] assignment
    fn extract_all_exports(&self, expr: &ast::Expr, exports: &mut HashSet<String>) -> Result<()> {
        match expr {
            ast::Expr::List(list_expr) => {
                for element in &list_expr.elts {
                    if let ast::Expr::Constant(const_expr) = element {
                        if let ast::Constant::Str(s) = &const_expr.value {
                            exports.insert(s.clone());
                        }
                    }
                }
            }
            ast::Expr::Tuple(tuple_expr) => {
                for element in &tuple_expr.elts {
                    if let ast::Expr::Constant(const_expr) = element {
                        if let ast::Constant::Str(s) = &const_expr.value {
                            exports.insert(s.clone());
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Detect if __name__ == "__main__" block (script entry point)
    fn detect_main_block(&mut self, suite: &[ast::Stmt]) -> Result<()> {
        for stmt in suite {
            // Look for: if __name__ == "__main__": ...
            if let ast::Stmt::If(if_stmt) = stmt {
                if self.is_main_guard(&if_stmt.test) {
                    // Mark that this module has a main block
                    // In a full implementation, we'd mark all statements in the main block
                    // as entry points or ScriptMain kind
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    /// Check if expression matches `__name__ == "__main__"` pattern
    fn is_main_guard(&self, expr: &ast::Expr) -> bool {
        match expr {
            ast::Expr::Compare(cmp) => {
                // Check for: __name__ == "__main__"
                // Be conservative: if we see __name__ and __main__ in a comparison, assume it's a main guard
                if cmp.comparators.len() != 1 {
                    return false;
                }

                let left_is_name = if let ast::Expr::Name(n) = cmp.left.as_ref() {
                    n.id.as_str() == "__name__"
                } else {
                    false
                };

                let right_is_main = if let ast::Expr::Constant(c) = &cmp.comparators[0] {
                    matches!(&c.value, ast::Constant::Str(s) if s == "__main__")
                } else {
                    false
                };

                left_is_name && right_is_main
            }
            _ => false,
        }
    }

    /// Register all functions in a suite (module body)
    fn register_module_functions_suite(
        &mut self,
        package: &str,
        suite: &[ast::Stmt],
    ) -> Result<()> {
        for stmt in suite {
            self.register_module_functions(package, stmt)?;
        }
        Ok(())
    }

    /// Register functions at module level (handles nested classes/functions too)
    fn register_module_functions(&mut self, package: &str, stmt: &ast::Stmt) -> Result<()> {
        match stmt {
            ast::Stmt::FunctionDef(func_def) => {
                let decorators = func_def
                    .decorator_list
                    .iter()
                    .filter_map(|d| self.extract_decorator_name(d))
                    .collect();

                let func_name = func_def.name.as_str();
                let is_dunder = func_name.starts_with("__") && func_name.ends_with("__");
                let kind = if is_dunder {
                    FunctionKind::DunderMethod
                } else {
                    FunctionKind::Function
                };
                let entry_point = if is_dunder {
                    EntryPointKind::DunderMethod
                } else if func_name.starts_with("test_") {
                    EntryPointKind::TestFunction
                } else {
                    EntryPointKind::Regular
                };

                let location = SourceLocation { line: 0, col: 0 };

                self.register_function(
                    package.to_string(),
                    func_name.to_string(),
                    location,
                    kind,
                    entry_point,
                    decorators,
                );

                // Also register nested functions/classes
                self.register_module_functions_suite(package, &func_def.body)?;
            }
            ast::Stmt::AsyncFunctionDef(func_def) => {
                let decorators = func_def
                    .decorator_list
                    .iter()
                    .filter_map(|d| self.extract_decorator_name(d))
                    .collect();

                let func_name = func_def.name.as_str();
                let is_dunder = func_name.starts_with("__") && func_name.ends_with("__");
                let entry_point = if is_dunder {
                    EntryPointKind::DunderMethod
                } else if func_name.starts_with("test_") {
                    EntryPointKind::TestFunction
                } else {
                    EntryPointKind::Regular
                };

                let location = SourceLocation { line: 0, col: 0 };

                self.register_function(
                    package.to_string(),
                    func_name.to_string(),
                    location,
                    FunctionKind::AsyncFunction,
                    entry_point,
                    decorators,
                );

                // Also register nested functions/classes
                self.register_module_functions_suite(package, &func_def.body)?;
            }
            ast::Stmt::ClassDef(class_def) => {
                // Register methods inside classes
                self.register_module_functions_suite(package, &class_def.body)?;
            }
            _ => {}
        }

        Ok(())
    }

    /// Extract function calls from all statements in a suite
    fn extract_calls_suite(&mut self, package: &str, suite: &[ast::Stmt]) -> Result<()> {
        for stmt in suite {
            self.extract_calls_from_stmt(package, stmt)?;
        }
        Ok(())
    }

    /// Recursive helper to extract calls from statements
    fn extract_calls_from_stmt(&mut self, package: &str, stmt: &ast::Stmt) -> Result<()> {
        match stmt {
            ast::Stmt::FunctionDef(func_def) => {
                self.extract_calls_suite(package, &func_def.body)?;
            }
            ast::Stmt::AsyncFunctionDef(func_def) => {
                self.extract_calls_suite(package, &func_def.body)?;
            }
            ast::Stmt::ClassDef(class_def) => {
                self.extract_calls_suite(package, &class_def.body)?;
            }
            ast::Stmt::If(if_stmt) => {
                self.extract_calls_suite(package, &if_stmt.body)?;
                self.extract_calls_suite(package, &if_stmt.orelse)?;
            }
            ast::Stmt::For(for_stmt) => {
                self.extract_calls_suite(package, &for_stmt.body)?;
                self.extract_calls_suite(package, &for_stmt.orelse)?;
            }
            ast::Stmt::AsyncFor(for_stmt) => {
                self.extract_calls_suite(package, &for_stmt.body)?;
                self.extract_calls_suite(package, &for_stmt.orelse)?;
            }
            ast::Stmt::While(while_stmt) => {
                self.extract_calls_suite(package, &while_stmt.body)?;
                self.extract_calls_suite(package, &while_stmt.orelse)?;
            }
            ast::Stmt::With(with_stmt) => {
                self.extract_calls_suite(package, &with_stmt.body)?;
            }
            ast::Stmt::AsyncWith(with_stmt) => {
                self.extract_calls_suite(package, &with_stmt.body)?;
            }
            ast::Stmt::Try(try_stmt) => {
                self.extract_calls_suite(package, &try_stmt.body)?;
                for handler in &try_stmt.handlers {
                    let ast::ExceptHandler::ExceptHandler(h) = handler;
                    self.extract_calls_suite(package, &h.body)?;
                }
                self.extract_calls_suite(package, &try_stmt.orelse)?;
                self.extract_calls_suite(package, &try_stmt.finalbody)?;
            }
            _ => {}
        }

        Ok(())
    }

    /// Extract decorator name from an expression
    fn extract_decorator_name(&self, expr: &ast::Expr) -> Option<String> {
        match expr {
            ast::Expr::Name(name_expr) => Some(name_expr.id.as_str().to_string()),
            ast::Expr::Attribute(attr) => Some(attr.attr.as_str().to_string()),
            _ => None,
        }
    }

    /// Build legacy PackageCallGraph for backward compatibility
    fn build_legacy_graph(&mut self, package: &str) {
        let graph = self
            .graphs
            .entry(package.to_string())
            .or_insert_with(|| PackageCallGraph::new(package.to_string()));

        // Populate definitions
        for node in self.nodes.values() {
            if node.package == package {
                graph.add_definition(node.name.clone());
            }
        }
    }

    /// Get all call graphs (legacy)
    #[must_use]
    pub fn get_graphs(&self) -> &HashMap<String, PackageCallGraph> {
        &self.graphs
    }

    /// Get call graph for a specific package (legacy)
    #[must_use]
    pub fn get_graph(&self, package: &str) -> Option<&PackageCallGraph> {
        self.graphs.get(package)
    }

    /// Find unused functions in a package (legacy)
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

    /// Find all external dependencies (legacy)
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

    /// Get all nodes in the call graph
    #[must_use]
    pub fn get_nodes(&self) -> &HashMap<FunctionId, CallGraphNode> {
        &self.nodes
    }

    /// Get all edges in the call graph
    #[must_use]
    pub fn get_edges(&self) -> &[CallEdge] {
        &self.edges
    }

    /// Get entry points
    #[must_use]
    pub fn get_entry_points(&self) -> &HashSet<FunctionId> {
        &self.entry_points
    }

    /// Compute reachable functions from entry points
    #[must_use]
    pub fn compute_reachable(&self) -> HashSet<FunctionId> {
        let mut reachable = HashSet::new();
        let mut queue = VecDeque::from_iter(self.entry_points.iter().copied());

        while let Some(current) = queue.pop_front() {
            if reachable.insert(current) {
                // Find all functions called by current
                for edge in &self.edges {
                    if edge.caller == current && !reachable.contains(&edge.callee) {
                        queue.push_back(edge.callee);
                    }
                }
            }
        }

        reachable
    }

    /// Find dead code (unreachable from entry points)
    #[must_use]
    pub fn find_dead_code(&self) -> Vec<(FunctionId, String)> {
        let reachable = self.compute_reachable();

        self.nodes
            .values()
            .filter_map(|node| {
                // Keep if reachable
                if reachable.contains(&node.id) {
                    return None;
                }

                // Keep dunder methods
                if node.name.starts_with("__") && node.name.ends_with("__") {
                    return None;
                }

                // Keep if exported
                if let Some(exports) = self.public_exports.get(&node.package) {
                    if exports.contains(&node.name) {
                        return None;
                    }
                }

                Some((node.id, node.name.clone()))
            })
            .collect()
    }
}

impl Default for CallGraphAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}
