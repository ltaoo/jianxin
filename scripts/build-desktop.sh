#!/usr/bin/env bash
set -euo pipefail
repo_dir="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_dir"
cargo build --locked --release -p jianxin-desktop
echo "Built target/release/jianxin-desktop (native tray + embedded Web Server)"
echo "Start the tray: target/release/jianxin-desktop"
