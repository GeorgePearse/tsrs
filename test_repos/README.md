# Test Repositories

This directory contains test repositories as git subrepos to validate that tree-shaking doesn't break functionality.

Each test repo has:
- Real Python code with actual dependencies
- A `before` script to test the original venv
- An `after` script to test the slimmed venv

## Running Tests

```bash
# Build the CLI first
cargo build --release --bin tsrs-cli

# Run all tests
cd test_repos && bash run_tests.sh
```

## Test Repos

Add test repos as git subrepos:

```bash
git subtree add --prefix test_repos/simple-data simple-data-repo.git main
git subtree add --prefix test_repos/web-app web-app-repo.git main
```
