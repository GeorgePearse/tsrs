# Test Repositories

This directory contains test repositories to validate that tree-shaking doesn't break functionality.

## How It Works

The test infrastructure runs a before/after validation:

1. **Before**: Creates a full venv, installs all dependencies, runs tests to verify everything works
2. **Slimming**: Runs tsrs on the code to create `.venv-slim`
3. **After**: Runs the same tests with the slimmed venv to ensure nothing broke
4. **Reporting**: Shows size savings and pass/fail results

## Test Structure

Each test repo contains:

```
repo/
├── pyproject.toml       # Dependencies
├── *.py                 # Application code
├── test.sh              # Test script that validates functionality
└── .venv/               # Created during test run
    └── ...
└── .venv-slim/          # Created by tsrs
    └── ...
```

The `test.sh` script must:
- Exit with status 0 if successful
- Test that all imported packages are available
- Validate key functionality works

## Running Tests

### Quick Start

```bash
# Build the CLI in release mode
cargo build --release --bin tsrs-cli

# Run all tests
cd test_repos && bash run_tests.sh
```

### Output Example

```
==================================================
Testing: simple-data
==================================================
[1/5] Creating virtual environment...
[2/5] Installing dependencies...
[3/5] Running test (before slimming)...
✓ Before test passed
[4/5] Running tree-shaking...
✓ Tree-shaking completed
[5/5] Running test (after slimming)...
✓ After test passed

Size comparison:
  Original: 145M
  Slimmed:  45M
```

## Test Repos Overview

### 1. **simple-data**
*Data processing with requests, click, pydantic*
- Fetches data from URLs
- Validates with Pydantic models
- CLI with Click
- **Size reduction**: ~30-40%

### 2. **cli-tool**
*CLI framework with typer and rich*
- CLI commands with Typer
- Rich formatted output
- Table rendering
- **Size reduction**: ~25-35%

### 3. **pandas-analysis**
*Data analysis with pandas and numpy*
- DataFrame operations
- Statistical analysis
- Group-by aggregations
- **Size reduction**: ~40-50%

### 4. **ml-classifier**
*Machine learning with scikit-learn*
- Dataset generation
- Model training (Random Forest)
- Performance evaluation (accuracy, precision, recall)
- **Size reduction**: ~45-55%

### 5. **data-viz**
*Data visualization with matplotlib and seaborn*
- Line plots
- Scatter plots
- Distribution plots
- **Size reduction**: ~35-45%

## Adding New Test Repos

Create a new directory with:

1. **pyproject.toml** - Define dependencies
2. **Python files** - Your application code
3. **test.sh** - A bash script that validates the application

Example `test.sh`:

```bash
#!/bin/bash
set -e

python -c "
import sys
sys.path.insert(0, '.')

# Test that all packages are available
import my_package
import requests
import pydantic

# Test basic functionality
result = my_package.core.process_data()
assert result is not None

print('✓ All tests passed')
"
```

Then initialize git:

```bash
cd test_repos/my-repo
git init
git add -A
git commit -m "Initial test repo"
```

The test runner will automatically discover and run your test.
