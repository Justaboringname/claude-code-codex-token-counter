#!/usr/bin/env bash
# Keep a free-Apple-ID install alive: reinstall Tokens when its 7-day profile is about to
# run out (or already has). Meant to be run every few minutes by a LaunchAgent; it does
# nothing until the last TC_RENEW_WINDOW_H hours (default 12), and nothing when the paired
# iPhone is out of reach. It never opens the app on the phone.
#   ios-app/scripts/renew-device.sh [--force]
set -uo pipefail
cd "$(dirname "$0")/.."
export PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"

window_h="${TC_RENEW_WINDOW_H:-12}"
app="build/dd-device/Build/Products/Debug-iphoneos/Tokens.app"
state="build/renew-state"   # "<expiry epoch>" of a profile a rebuild did not extend
say() { echo "$(date '+%Y-%m-%d %H:%M:%S') $*"; }
notify() { osascript -e "display notification \"$1\" with title \"Tokens (iPhone)\"" >/dev/null 2>&1 || true; }

# Earliest expiry (epoch seconds) among the app's and the widget extension's profiles; 0 if unknown.
expiry() {
  local min=0 p d e
  for p in "$app/embedded.mobileprovision" "$app"/PlugIns/*.appex/embedded.mobileprovision; do
    [ -f "$p" ] || continue
    d="$(security cms -D -i "$p" 2>/dev/null | plutil -extract ExpirationDate raw -o - - 2>/dev/null)" || continue
    e="$(date -j -u -f '%Y-%m-%dT%H:%M:%SZ' "$d" +%s 2>/dev/null)" || continue
    if [ "$min" -eq 0 ] || [ "$e" -lt "$min" ]; then min="$e"; fi
  done
  echo "$min"
}

now="$(date +%s)"
before="$(expiry)"
if [ "${1:-}" != "--force" ] && [ "$before" -gt 0 ]; then
  [ $((before - now)) -gt $((window_h * 3600)) ] && exit 0
  # A rebuild inside the window did not get a newer profile: wait for this one to run out.
  if [ -f "$state" ] && [ "$(cat "$state")" = "$before" ] && [ "$now" -lt "$before" ]; then exit 0; fi
fi

list="$(mktemp)"
trap 'rm -f "$list"' EXIT
xcrun devicectl list devices --json-output "$list" >/dev/null 2>&1
device="$(python3 -I - "$list" <<'PY'
import json, sys
try:
    devices = json.load(open(sys.argv[1]))["result"]["devices"]
except Exception:
    devices = []
for d in devices:
    hw, cp = d.get("hardwareProperties", {}), d.get("connectionProperties", {})
    if hw.get("reality") == "physical" and hw.get("platform") == "iOS" and cp.get("pairingState") == "paired":
        print(hw["udid"])
        break
PY
)"
if [ -z "$device" ]; then
  say "no paired iPhone in reach; will retry"
  exit 0
fi

# The build or the install can hang (phone walks out of Wi-Fi); a hung run would block every
# later one, so give it TC_RENEW_TIMEOUT seconds and then kill its whole process group.
outf="$(mktemp)"
trap 'rm -f "$list" "$outf"' EXIT
set -m
TC_NO_LAUNCH=1 scripts/install-device.sh "$device" >"$outf" 2>&1 &
job=$!
( sleep "${TC_RENEW_TIMEOUT:-900}"; echo "timed out" >>"$outf"; kill -- -"$job" 2>/dev/null ) &
dog=$!
wait "$job"; rc=$?
kill -- -"$dog" 2>/dev/null; wait "$dog" 2>/dev/null
set +m
out="$(cat "$outf")"

if [ "$rc" -eq 0 ]; then
  after="$(expiry)"
  until="$(date -r "$after" '+%b %-d %H:%M')"
  if [ "$after" -gt "$before" ]; then
    rm -f "$state"
    say "renewed: good until $until"
    notify "Renewed, good until $until."
  else
    echo "$after" >"$state"
    say "reinstalled, but the profile was not extended (still $until); will renew once it runs out"
  fi
else
  say "failed: $out"
  # Only nag once the app has actually stopped opening, and at most twice a day.
  stamp="build/renew-failed"
  if [ "$now" -ge "$before" ] && { [ ! -f "$stamp" ] || [ $((now - $(stat -f %m "$stamp"))) -gt 43200 ]; }; then
    touch "$stamp"
    notify "Could not renew. See ~/Library/Logs/tokens-ios-renew.log"
  fi
  exit 1
fi
