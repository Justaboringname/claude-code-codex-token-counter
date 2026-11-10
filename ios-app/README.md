# Token Counter for iPhone

Last updated: 2026-11-10

A native SwiftUI app that shows what the main Mac publishes through [cloud sync](../docs/SYNC.md): spend per
range, the dual Claude/Codex sparkline, per-model bars, the official limit meters and tool calls. It is a
read-only viewer, like the phone web app, plus what a web page can't do:

- **Widgets**: Home-screen small and medium, Lock-Screen rectangular, circular and inline. They refresh by
  themselves (iOS picks the moments, usually 15–60 minutes apart) and whenever the app gets new numbers; when
  the Mac has stopped syncing they say "as of" the time the numbers are from. Long-press a widget → Edit to
  choose its range.
- **Fresh on demand**: opening the app, the refresh button and pull-to-refresh ask the Mac to upload its latest
  numbers right away (about 3 seconds) instead of showing what it uploaded a minute ago.
- **QR pairing inside the app**: scan the QR code from the Mac's Sync sheet, or paste the pairing code.
- The pairing code is kept in the Keychain, the last snapshot in the app's shared container for offline use.

Nothing here talks to Anthropic or OpenAI. The phone only downloads the encrypted snapshot and decrypts it
with the pairing code (same protocol as the Mac and the web app: SHA-256 derived keys, AES-256-GCM over gzip).

## Requirements

- Xcode 26 or later (the app uses Liquid Glass and the Icon Composer `.icon` of the Mac app), iOS 26+.
- [XcodeGen](https://github.com/yonaskolb/XcodeGen): `brew install xcodegen`. The `.xcodeproj` is generated
  and not committed.

## Build and run

```bash
cd ios-app
cp Config/Local.example.xcconfig Config/Local.xcconfig   # then edit it, see below
xcodegen
open TokenCounter.xcodeproj
```

`Config/Local.xcconfig` (gitignored) holds the two personal values:

- `TC_BUNDLE_PREFIX`: a reverse-DNS prefix nobody else uses. The app is `<prefix>`, the widgets
  `<prefix>.widgets`, the shared App Group `group.<prefix>`.
- `DEVELOPMENT_TEAM`: your team id. Not needed for the Simulator.

In Xcode pick the `TokenCounter` scheme and an iPhone simulator or your iPhone, then Run.

### On your own iPhone with a free Apple ID

A free Apple ID ("Personal Team") is enough; App Groups and Keychain sharing, which the widgets need, are
available to it.

1. Xcode → Settings → Accounts → add your Apple ID.
2. Connect the iPhone, trust the Mac, and turn on Developer Mode (Settings → Privacy & Security).
3. Put your team id in `Config/Local.xcconfig`. Once Xcode has opened any project with the account added:
   ```bash
   plutil -p ~/Library/Preferences/com.apple.dt.Xcode.plist | grep teamID
   ```
4. Build and install from the command line (or press Run in Xcode):
   ```bash
   scripts/install-device.sh
   ```
5. The first time, trust the developer on the phone (Settings → General → VPN & Device Management), then
   open Tokens and pair: tap *Scan QR code* and point it at the QR in the Mac's Sync sheet, or paste the code.

Limits of a free team: the app stops opening after 7 days until `scripts/install-device.sh` is run again (the
pairing and the widgets stay), and at most 10 new App IDs can be registered per week, so choose
`TC_BUNDLE_PREFIX` once.

## Tests

```bash
xcodebuild test -project TokenCounter.xcodeproj -scheme TokenCounter \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro'
```

The expected values are not hand-written: `node scripts/make-test-vectors.mjs` runs the web app's own
TypeScript (`src/sync/protocol.ts`, `snapshot.ts`, `format.ts`, …) and writes `Tests/Fixtures/vectors.json`,
and the Rust side's envelope fixture is opened too. So the tests fail when the phone would decrypt, round or
label something differently from the Mac or the web app. Re-run the script after changing that TypeScript.

## Layout

| Path | What |
|---|---|
| `project.yml`, `Config/` | XcodeGen spec, build settings, Info.plist additions, the App Group entitlement |
| `Sources/Shared/` | Compiled into the app and the widgets: protocol, gunzip, snapshot validation, Keychain, cache, formatting, the numbers on screen (`Dashboard.swift`), colours, sparkline |
| `Sources/App/` | Polling store, pairing, the dashboard, the Sync sheet, the QR scanner, the wallpaper (it makes waves under a finger and when the phone is shaken) |
| `Sources/Widgets/` | The widget extension |
| `Tests/` | Swift Testing suites and fixtures |

Debug builds running in the Simulator accept a few environment variables for looking at states without a
server (a snapshot file, a sync error, the pairing screen, every widget family at its real size); see
`DebugPreview` in `Sources/App/TokenCounterApp.swift`. The Simulator has no motion sensors, so
`TC_PREVIEW_WAVES` (`DebugWaves` in `Sources/App/Wallpaper.swift`) stirs the wallpaper by itself.
