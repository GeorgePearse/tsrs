# tsrs - Tree-Shaking in Rust for Python

A high-performance tree-shaking implementation in Rust for Python modules and packages.

## Manifesto

> "Ever had someone say, 'just copy the function, we don't need the whole package'? What if that didn't have to be true?"

Tree-shaking enables developers to depend on large, well-designed libraries while only deploying the code they actually use. No more choosing between monolithic packages or duplicating code. Get the best of both worlds: leverage battle-tested libraries while keeping your deployments lean and efficient.

## Overview

Tree-shaking is the process of analyzing code to identify and remove unused exports from Python modules. This project provides a Rust-based implementation that can be used from Python to detect dead code and optimize module sizes.

## Usage

### CLI
```bash
# Analyze a virtual environment
./target/debug/tsrs-cli analyze /path/to/venv

# Create a slim venv from Python code and venv
./target/debug/tsrs-cli slim <python-directory> <venv-location>

# Create slim venv with custom output path
./target/debug/tsrs-cli slim <python-directory> <venv-location> -o /path/to/output/.venv-slim
```

### How it Works

1. **Scans the Python code directory** for all import statements
2. **Analyzes the source venv** to discover all installed packages
3. **Maps imports to packages** and copies only the used packages to a new slim venv
4. **Creates `.venv-slim`** with only the minimal dependencies needed

### Example
```bash
# Slim your venv based on actual code usage
tsrs-cli slim ./src ./.venv
# Creates: ./.venv-slim with only the packages your code imports
```

## Building

### CLI Only
```bash
cargo build --release --bin tsrs-cli
./target/release/tsrs-cli --help
```

### With Python Extension
This project can also build as a Python extension module using PyO3.

```bash
# Setup (optional Python feature)
pip install maturin

# Build and develop
maturin develop

# Or build a wheel
maturin build --release
```

## Architecture

### Core Modules

- **`venv`** - Virtual environment discovery and package analysis
- **`imports`** - Import statement extraction and tracking
- **`callgraph`** - Function call graph analysis per package
  - Tracks which functions are defined in each package
  - Maps external dependencies between packages
  - Identifies unused/dead code that is never called
- **`slim`** - Creates minimal venvs based on code analysis

### How Tree-Shaking Works

The tool builds a complete picture of your code's dependencies:

1. **Import Analysis**: Extracts all `import` and `from...import` statements from your code
2. **Call Graph Building**: Analyzes function definitions and calls in your Python code
3. **Package Mapping**: Maps imports to actual packages in your venv
4. **Dead Code Detection**: Identifies functions/classes that are defined but never used
5. **Dependency Reduction**: Creates a slim venv with only the necessary packages

This multi-layered approach ensures you don't accidentally remove code that's used through indirect calls or dynamic imports.

## Development

```bash
cargo test
cargo fmt
cargo clippy
```

## References & Inspiration

### Related Projects
- **[Ruff](https://github.com/astral-sh/ruff)** - An extremely fast Python linter written in Rust. Uses `ruff_python_parser` for parsing Python code.
- **[Pylyzer](https://github.com/mtshiba/pylyzer)** - A fast, feature-rich static code analyzer & language server for Python. Uses Rust internally with type checking capabilities.
- **[Skylos](https://github.com/duriantaco/skylos)** - A static analysis tool for Python codebases that detects dead code, unused functions, classes, imports, and variables. Also includes security flaw detection.

### Discussion
- [Reddit Discussion: Is there any support in Python for something like tree-shaking?](https://www.reddit.com/r/Python/comments/aqqzjl/is_there_any_support_in_python_for_something/)

## License

TBD
