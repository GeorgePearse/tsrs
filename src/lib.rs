use pyo3::prelude::*;

/// Tree-shaking module for Python
///
/// Provides functionality to identify and remove unused code exports
/// from Python modules and packages.
#[pymodule]
fn tsrs(py: Python, m: &PyModule) -> PyResult<()> {
    m.add("__doc__", "Tree-shaking in Rust for Python")?;
    Ok(())
}
