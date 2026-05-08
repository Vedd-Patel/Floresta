#!/usr/bin/env bash

set -euo pipefail

echo ""
echo "================================================"
echo "Testing commit: $(git log -1 --oneline)"
echo "================================================"
echo ""

cargo build --workspace --release
cargo test --workspace --release
