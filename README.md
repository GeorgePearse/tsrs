# tsrs - Tree-Shaking in Rust for Python

A high-performance tree-shaking implementation in Rust for Python modules and packages.

## Manifesto

> "Ever had someone say, 'just copy the function, we don't need the whole package'? What if that didn't have to be true?"

Tree-shaking enables developers to depend on large, well-designed libraries while only deploying the code they actually use. No more choosing between monolithic packages or duplicating code. Get the best of both worlds: leverage battle-tested libraries while keeping your deployments lean and efficient.

## Overview

Tree-shaking is the process of analyzing code to identify and remove unused exports from Python modules. This project provides a Rust-based implementation that can be used from Python to detect dead code and optimize module sizes.

## Building

This project uses Rust with PyO3 to create Python bindings.

### Requirements
- Rust 1.56+
- Python 3.7+
- maturin (for building Python wheels)

```bash
# Build the Rust extension
cargo build --release

# Or build a Python wheel
pip install maturin
maturin develop
```

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

### Discussion
- [Reddit Discussion: Is there any support in Python for something like tree-shaking?](https://www.reddit.com/r/Python/comments/aqqzjl/is_there_any_support_in_python_for_something/)

## License

TBD
