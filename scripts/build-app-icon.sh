#!/usr/bin/env bash
# Rebuild every app-icon asset from the Icon Composer source
# src-tauri/icons/AppIcon.icon (layered SVGs + icon.json, editable in Xcode's Icon Composer).
#
# Outputs (all in src-tauri/icons/):
#   Assets.car  - native Liquid Glass icon (Default/Dark/Clear/Tinted), compiled by actool.
#                 bundle.macOS.files copies it into Contents/Resources and
#                 CFBundleIconName=AppIcon (src-tauri/Info.plist) makes macOS 26+ use it.
#   icon.icns, *.png, icon.ico
#               - flattened fallback for older macOS / other targets: Apple's renderer (ictool)
#                 draws the Default appearance as the 824px tile of the 1024px macOS icon grid,
#                 padded to 1024 and fed to `tauri icon`. (actool's own fallback icns stops at 256px.)
# Requires Xcode 26+ (actool .icon support and Icon Composer's ictool).
set -euo pipefail
icons="$(cd "$(dirname "$0")/../src-tauri/icons" && pwd)"
root="$(cd "$icons/../.." && pwd)"
ictool="$(xcode-select -p)/../Applications/Icon Composer.app/Contents/Executables/ictool"
work="$(mktemp -d)"
trap 'rm -r "$work"' EXIT

xcrun actool --compile "$work" --platform macosx --minimum-deployment-target 10.13 \
  --app-icon AppIcon --output-partial-info-plist "$work/partial.plist" \
  "$icons/AppIcon.icon" >/dev/null
[ -f "$work/Assets.car" ] || { echo "actool did not produce Assets.car" >&2; exit 1; }
cp "$work/Assets.car" "$icons/Assets.car"

"$ictool" "$icons/AppIcon.icon" --export-image --output-file "$work/tile.png" \
  --platform macOS --rendition Default --width 824 --height 824 --scale 1 >/dev/null
sips -p 1024 1024 "$work/tile.png" --out "$work/icon-1024.png" >/dev/null
(cd "$root" && pnpm -s tauri icon "$work/icon-1024.png" -o "$work/gen" >/dev/null 2>&1)
for f in "$work"/gen/*.png "$work"/gen/icon.ico "$work"/gen/icon.icns; do cp "$f" "$icons/"; done
echo "app icon rebuilt from $icons/AppIcon.icon"
