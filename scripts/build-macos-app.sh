#!/bin/bash
set -euo pipefail
repo_dir="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_dir"
if [[ "$(uname -s)" != Darwin ]]; then
  echo "macOS is required" >&2
  exit 1
fi
if [[ "${1:-}" != "" && "${1:-}" != "--install" ]]; then
  echo "Usage: bash scripts/build-macos-app.sh [--install]" >&2
  exit 1
fi
export TMPDIR=/private/tmp
cargo build --profile macos-app -p jianxin-cli --bin jianxin
cargo build --profile macos-app -p jianxin-desktop --bin jianxin-desktop
build_dir="$(mktemp -d /private/tmp/jianxin-app.XXXXXX)"
app_dir="$build_dir/Jianxin.app"
mkdir -p "$app_dir/Contents/MacOS" "$app_dir/Contents/Resources" "$build_dir/Jianxin.iconset"
cp platform/macos/Info.plist "$app_dir/Contents/Info.plist"
cp target/macos-app/jianxin "$app_dir/Contents/Resources/jianxin"
cp target/macos-app/jianxin-desktop "$app_dir/Contents/Resources/jianxin-desktop"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" assets/jianxin-icon.png --out "$build_dir/Jianxin.iconset/icon_${size}x${size}.png" >/dev/null
  retina_size=$((size * 2))
  sips -z "$retina_size" "$retina_size" assets/jianxin-icon.png --out "$build_dir/Jianxin.iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$build_dir/Jianxin.iconset" -o "$app_dir/Contents/Resources/Jianxin.icns"
swiftc -Osize -module-cache-path "$build_dir/module-cache" -framework AppKit -framework UserNotifications \
  -target "$(uname -m)-apple-macos11.0" platform/macos/Jianxin.swift -o "$app_dir/Contents/MacOS/Jianxin"
codesign --force --sign - --identifier com.jianxin.mail.cli "$app_dir/Contents/Resources/jianxin"
codesign --force --sign - --identifier com.jianxin.mail.desktop "$app_dir/Contents/Resources/jianxin-desktop"
codesign --force --sign - --identifier com.jianxin.mail "$app_dir"
codesign --verify --deep --strict "$app_dir"
mkdir -p target/macos
destination="$repo_dir/target/macos/Jianxin.app"
if [[ -e "$destination" ]]; then
  mv "$destination" "$build_dir/previous-build.app"
fi
ditto "$app_dir" "$destination"
if [[ "${1:-}" == "--install" ]]; then
  destination="$HOME/Applications/Jianxin.app"
  mkdir -p "$HOME/Applications"
  if [[ -e "$destination" ]]; then
    mv "$destination" "$build_dir/previous-installed.app"
  fi
  ditto "$app_dir" "$destination"
  /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$destination"
fi
echo "Jianxin.app: $destination"
echo "Build/backup directory retained: $build_dir"
echo "Next: jianxin notifications --register"
