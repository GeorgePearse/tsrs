#!/bin/bash

# Test runner for tsrs
# Runs before/after tests on each test repo

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"
CLI_PATH="${ROOT_DIR}/target/release/tsrs-cli"

echo "=================================================="
echo "tsrs Test Runner"
echo "=================================================="
echo ""

# Check if CLI exists
if [ ! -f "$CLI_PATH" ]; then
    echo "Error: CLI not found at $CLI_PATH"
    echo "Please run: cargo build --release --bin tsrs-cli"
    exit 1
fi

# Find all test repos
TEST_REPOS=$(find "$SCRIPT_DIR" -maxdepth 1 -type d -name "*/." -prune -o -type d -print | sort | tail -n +2)

PASSED=0
FAILED=0

for repo_dir in $TEST_REPOS; do
    repo_name=$(basename "$repo_dir")
    
    # Skip non-test directories
    if [ ! -f "$repo_dir/test.sh" ]; then
        continue
    fi
    
    echo ""
    echo "=================================================="
    echo "Testing: $repo_name"
    echo "=================================================="
    
    cd "$repo_dir"
    
    # Create venv
    echo "[1/5] Creating virtual environment..."
    python -m venv .venv
    
    # Activate and install
    echo "[2/5] Installing dependencies..."
    .venv/bin/pip install -q -e . 2>/dev/null || true
    
    # Run test before
    echo "[3/5] Running test (before slimming)..."
    if .venv/bin/python test.sh 2>&1; then
        echo "✓ Before test passed"
    else
        echo "✗ Before test FAILED"
        FAILED=$((FAILED + 1))
        continue
    fi
    
    # Run tsrs
    echo "[4/5] Running tree-shaking..."
    if "$CLI_PATH" slim . .venv -o .venv-slim 2>&1 | grep -v "^Compiling\|^Downloading\|^Installing"; then
        echo "✓ Tree-shaking completed"
    else
        echo "✗ Tree-shaking FAILED"
        FAILED=$((FAILED + 1))
        continue
    fi
    
    # Run test after with slim venv
    echo "[5/5] Running test (after slimming)..."
    SLIM_TEST_RESULT=0
    if VIRTUAL_ENV=.venv-slim .venv-slim/bin/python test.sh 2>&1; then
        echo "✓ After test passed"
        PASSED=$((PASSED + 1))
    else
        echo "✗ After test FAILED"
        FAILED=$((FAILED + 1))
    fi
    
    # Show sizes
    ORIG_SIZE=$(du -sh .venv | cut -f1)
    SLIM_SIZE=$(du -sh .venv-slim | cut -f1)
    echo ""
    echo "Size comparison:"
    echo "  Original: $ORIG_SIZE"
    echo "  Slimmed:  $SLIM_SIZE"
    
    cd - > /dev/null
done

echo ""
echo "=================================================="
echo "Summary"
echo "=================================================="
echo "Passed: $PASSED"
echo "Failed: $FAILED"
echo "=================================================="

if [ $FAILED -eq 0 ]; then
    echo "✓ All tests passed!"
    exit 0
else
    echo "✗ Some tests failed"
    exit 1
fi
