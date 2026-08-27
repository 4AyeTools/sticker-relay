#!/usr/bin/env bash
set -euo pipefail

target="$1"
label="$2"
project_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$project_root"

version="$(node -p "require('./package.json').version")"
release_root="src-tauri/target/$target/release"
output_root="release"
mkdir -p "$output_root"

app_path="$(find "$release_root/bundle/macos" -maxdepth 1 -type d -name '*.app' -print -quit)"
dmg_path="$(find "$release_root/bundle/dmg" -maxdepth 1 -type f -name '*.dmg' -print -quit)"
test -n "$app_path"
test -n "$dmg_path"

cp "$dmg_path" "$output_root/sticker-relay-$version-$label.dmg"
ditto -c -k --sequesterRsrc --keepParent "$app_path" "$output_root/sticker-relay-$version-$label.app.zip"

find "$release_root/bundle" -type f \( -name '*.sig' -o -name '*.tar.gz' -o -name '*.zip' \) -exec cp {} "$output_root/" \;
