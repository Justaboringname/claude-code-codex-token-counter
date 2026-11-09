#!/usr/bin/env bash
# Phone web app icons (web/public/) from the Icon Composer source
# src-tauri/icons/AppIcon.icon: Apple's renderer (ictool) draws the iOS Default
# appearance, scripts/flatten-icon.swift makes it an opaque square (iOS masks
# apple-touch-icons itself and turns transparency black).
# Requires Xcode 26+ (Icon Composer's ictool).
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
ictool="$(xcode-select -p)/../Applications/Icon Composer.app/Contents/Executables/ictool"
work="$(mktemp -d)"
trap 'rm -r "$work"' EXIT

"$ictool" "$root/src-tauri/icons/AppIcon.icon" --export-image --output-file "$work/ios.png" \
  --platform iOS --rendition Default --width 1024 --height 1024 --scale 1 >/dev/null
mkdir -p "$root/web/public"
xcrun swift "$root/scripts/flatten-icon.swift" "$work/ios.png" \
  "$root/web/public/apple-touch-icon.png" 180 \
  "$root/web/public/icon-192.png" 192
