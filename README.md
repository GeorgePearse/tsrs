# tsrs - Tree-Shaking in Rust for Python

A high-performance tree-shaking implementation in Rust for Python modules and packages.

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

## References

- [Reddit Discussion: Is there any support in Python for something like tree-shaking?](https://www.reddit.com/r/Python/comments/aqqzjl/is_there_any_support_in_python_for_something/)

## License

TBD
