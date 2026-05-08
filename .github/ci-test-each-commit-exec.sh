#!/usr/bin/env bash

set -euo pipefail

echo "--- Testing commit: $(git log -1 --oneline) ---"

# Build the entire workspace. This catches compile errors introduced by this commit.
cargo build --workspace

# Run the test suite. This catches test regressions introduced by this commit.
cargo test --workspace
