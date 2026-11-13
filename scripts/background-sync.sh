#!/usr/bin/env bash
# Keep Token Counter running in the background so sync keeps uploading when
# its window is closed, after Cmd+Q, and after a restart.
#
# Installs a per-user LaunchAgent that starts the installed app with
# --background (no window, no Dock icon) at login and starts it again whenever
# it exits. Opening the app (Dock, Spotlight, Finder) shows the window of that
# same running copy; closing the window hides it again.
#
#   scripts/background-sync.sh install     # also: on   (re-run after moving the app)
#   scripts/background-sync.sh uninstall   # also: off  (stops the background copy)
#   scripts/background-sync.sh restart     # after installing a new build
#   scripts/background-sync.sh status
#
# TC_APP overrides the app location (default: /Applications/Token Counter.app).
set -euo pipefail

APP="${TC_APP:-/Applications/Token Counter.app}"
INFO="$APP/Contents/Info.plist"
DOMAIN="gui/$(id -u)"
LOG_DIR="$HOME/Library/Logs/Token Counter"
LOG="$LOG_DIR/background.log"

die() { echo "background-sync: $*" >&2; exit 1; }

app_info() {
  [[ -f "$INFO" ]] || die "no app at \"$APP\" (set TC_APP to its location)"
  BUNDLE_ID="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$INFO")"
  EXE="$APP/Contents/MacOS/$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$INFO")"
  [[ -x "$EXE" ]] || die "\"$EXE\" is not executable"
  LABEL="$BUNDLE_ID.background"
  PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
}

# The label comes from the app's bundle id; without an app (uninstall after
# deleting it) fall back to the one this script has always used.
label_only() {
  if [[ -f "$INFO" ]]; then
    app_info
  else
    LABEL="com.anthropic.claudetokencounter.background"
    PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
  fi
}

loaded() { launchctl print "$DOMAIN/$LABEL" >/dev/null 2>&1; }

# PIDs of this app's executable, however it was started.
app_pids() { pgrep -f -- "^$EXE( |\$)" || true; }

wait_until() { # wait_until <seconds> <command...>
  local limit=$(( $1 * 10 )); shift
  for _ in $(seq 1 "$limit"); do "$@" && return 0; sleep 0.1; done
  return 1
}
not_loaded() { ! loaded; }
app_running() { [[ -n "$(app_pids)" ]]; }
no_app_running() { ! app_running; }

xml() { sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g' <<<"$1"; }

write_plist() {
  mkdir -p "$(dirname "$PLIST")" "$LOG_DIR"
  local tmp="$PLIST.tmp.$$"
  cat >"$tmp" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>$(xml "$LABEL")</string>
	<key>ProgramArguments</key>
	<array>
		<string>$(xml "$EXE")</string>
		<string>--background</string>
	</array>
	<!-- Shown as "Token Counter" under Login Items > Allow in the Background. -->
	<key>AssociatedBundleIdentifiers</key>
	<array>
		<string>$(xml "$BUNDLE_ID")</string>
	</array>
	<key>RunAtLoad</key>
	<true/>
	<!-- Started again after Cmd+Q or a crash (launchd waits at least 10 s). -->
	<key>KeepAlive</key>
	<true/>
	<key>LimitLoadToSessionType</key>
	<string>Aqua</string>
	<!-- It is the app itself (its window can be opened at any time): no launchd throttling. -->
	<key>ProcessType</key>
	<string>Interactive</string>
	<key>StandardOutPath</key>
	<string>$(xml "$LOG")</string>
	<key>StandardErrorPath</key>
	<string>$(xml "$LOG")</string>
</dict>
</plist>
EOF
  plutil -lint -s "$tmp" || { rm -f "$tmp"; die "generated plist is invalid"; }
  mv -f "$tmp" "$PLIST"
}

stop_job() {
  if loaded; then
    launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || true
    wait_until 15 not_loaded || die "the background job did not stop"
  fi
}

cmd_install() {
  app_info
  stop_job
  # A copy the user opened is not the agent's: quit it so the agent's copy is
  # the only one, then bring its window back below.
  local was_open=""
  if app_running; then
    was_open=1
    osascript -e "quit app id \"$BUNDLE_ID\"" >/dev/null 2>&1 || true
    wait_until 15 no_app_running ||
      die "Token Counter didn't quit; quit it (Cmd+Q) and run this again"
  fi
  write_plist
  launchctl bootstrap "$DOMAIN" "$PLIST"
  wait_until 10 app_running || true
  if [[ -n "$was_open" ]]; then
    sleep 1
    open "$APP"
  fi
  echo "Background sync is on: Token Counter now starts at login and keeps running with its window closed."
  cmd_status
}

cmd_uninstall() {
  label_only
  stop_job
  rm -f "$PLIST"
  echo "Background sync is off. Token Counter runs (and syncs) only while you have it open."
}

cmd_restart() {
  app_info
  loaded || die "background sync is not installed (run: $0 install)"
  launchctl kickstart -k "$DOMAIN/$LABEL"
  wait_until 10 app_running || true
  cmd_status
}

cmd_status() {
  label_only
  if loaded; then
    echo "LaunchAgent: loaded ($LABEL)"
    launchctl print "$DOMAIN/$LABEL" |
      grep -E $'^\t(state|pid|last exit code|runs) = ' | sed 's/^[[:space:]]*/  /' || true
  elif [[ -f "$PLIST" ]]; then
    echo "LaunchAgent: installed but not loaded (it loads at the next login; or run: $0 install)"
  else
    echo "LaunchAgent: not installed"
  fi
  if [[ -n "${EXE:-}" ]]; then
    local pids; pids="$(app_pids | tr '\n' ' ')"
    echo "Running copies: ${pids:-none}"
  fi
  if [[ -f "$LOG" ]]; then
    echo "Log ($LOG), last lines:"
    tail -n 5 "$LOG" | sed 's/^/  /'
  fi
}

case "${1:-status}" in
  install | on) cmd_install ;;
  uninstall | off) cmd_uninstall ;;
  restart) cmd_restart ;;
  status) cmd_status ;;
  *) die "usage: $0 install|uninstall|restart|status" ;;
esac
