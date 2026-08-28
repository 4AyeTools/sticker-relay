#!/usr/bin/env bash
set -euo pipefail

target="$1"
label="$2"
updater_enabled="${3:-false}"
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

updater_archive="$(find "$release_root/bundle/macos" -maxdepth 1 -type f -name '*.app.tar.gz' -print -quit)"
if [[ "$updater_enabled" == "true" ]]; then
  test -n "$updater_archive"
  if [[ ! -f "$updater_archive.sig" ]]; then
    npx tauri signer sign "$updater_archive"
  fi
  test -f "$updater_archive.sig"
  updater_name="sticker-relay-$version-$label.app.tar.gz"
  cp "$updater_archive" "$output_root/$updater_name"
  cp "$updater_archive.sig" "$output_root/$updater_name.sig"
fi
