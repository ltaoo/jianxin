#!/usr/bin/env bash
set -euo pipefail
repo_dir="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_dir"
cargo build --locked --release -p jianxin-cli -p jianxin-desktop
echo "Built target/release/jianxin and target/release/jianxin-desktop"
echo "Start the tray: target/release/jianxin tray"
