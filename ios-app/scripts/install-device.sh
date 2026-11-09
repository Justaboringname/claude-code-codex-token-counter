#!/usr/bin/env bash
# Build Token Counter for the connected iPhone and install it.
#   ios-app/scripts/install-device.sh [device udid]
# With a free Apple ID the install stops opening after 7 days: run this again
# (the pairing code and the widgets stay). Needs Config/Local.xcconfig with
# DEVELOPMENT_TEAM and TC_BUNDLE_PREFIX, the Apple ID added in Xcode's
# Accounts settings, and the iPhone connected, trusted and in Developer Mode.
set -euo pipefail
cd "$(dirname "$0")/.."

command -v xcodegen >/dev/null || { echo "XcodeGen is missing: brew install xcodegen" >&2; exit 1; }
[ -f Config/Local.xcconfig ] || { echo "Copy Config/Local.example.xcconfig to Config/Local.xcconfig and fill it in." >&2; exit 1; }

device="${1:-}"
if [ -z "$device" ]; then
  list="$(mktemp)"
  trap 'rm -f "$list"' EXIT
  xcrun devicectl list devices --json-output "$list" >/dev/null
  device="$(python3 -I - "$list" <<'PY'
import json, sys
for d in json.load(open(sys.argv[1]))["result"]["devices"]:
    hw, cp = d.get("hardwareProperties", {}), d.get("connectionProperties", {})
    if hw.get("reality") == "physical" and hw.get("platform") == "iOS" and cp.get("tunnelState") == "connected":
        print(hw["udid"])
        break
PY
)"
fi
[ -n "$device" ] || { echo "No connected iPhone found (unlock it and check the cable)." >&2; exit 1; }

xcodegen -q
log="$(mktemp)"
if ! xcodebuild build -project TokenCounter.xcodeproj -scheme TokenCounter \
  -destination "platform=iOS,id=$device" -derivedDataPath build/dd-device \
  -allowProvisioningUpdates -allowProvisioningDeviceRegistration >"$log" 2>&1; then
  grep -E "error:|BUILD FAILED" "$log" | sort -u >&2
  echo "Full log: $log" >&2
  exit 1
fi
rm -f "$log"

app="build/dd-device/Build/Products/Debug-iphoneos/Tokens.app"
xcrun devicectl device install app --device "$device" "$app" >/dev/null
bundle_id="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$app/Info.plist")"
if xcrun devicectl device process launch --device "$device" "$bundle_id" >/dev/null 2>&1; then
  echo "Installed and opened Tokens on the iPhone."
else
  echo "Installed. iOS would not open it yet: unlock the iPhone, and the first time trust the developer under"
  echo "Settings → General → VPN & Device Management, then open Tokens."
fi
