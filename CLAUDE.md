# CLAUDE.md — Token Counter project handoff

_Last updated: 2026-11-10_

## Cloud sync + phone web app (2026-11-09, v0.3.0)

User-facing description, privacy model, setup and protocol: `docs/SYNC.md`. Internals:

- Rust `src-tauri/src/sync.rs`: pairing-code parse/derive, AES-256-GCM envelope over gzip, `sync.json`
  (app-data dir, 0600, tmp+rename), the `sync-publisher` thread (first tick 20 s, then 60 s; Condvar wake on
  config change / Sync now), backoff 60 s → 10 min, `sync-status` event, commands `sync_get_config`,
  `sync_set_config`, `sync_status`, `sync_publish_now`. `lib.rs`: `claude_quota()` / `codex_quota()` are the
  shared cached paths the commands and the publisher both use (never a second network call).
- `COMPUTE_LOCK` is a `parking_lot::Mutex`; the publisher computes the five ranges one lock at a time and
  `unlock_fair`s between them so a waiting popover scan gets in (a std Mutex let the releasing thread re-lock).
- Content hash (what decides an upload) is computed on a normalized copy: volatile timestamps, rolling-window
  figures no screen shows, key order and float noise removed. Heartbeat upload every 10 min regardless. If
  versions climb ~60/h on an idle Mac, a volatile field slipped into the hash.
- TS: `src/sync/protocol.ts` (pure; also imported by `scripts/sync-selftest.mjs` via Node type stripping),
  `remote.ts` (30 s head poll while visible, get only on a new version, validated cache), `snapshot.ts`
  (strict sanitize; refuses a newer schema major), `pairing.ts` (web pairing, `#pair=` rules: auto-pair only
  when unpaired, otherwise a confirm; standalone launch links are consumed once), `desktop.ts` (commands +
  localStorage role hint), `SyncSheet.tsx`, `PairingScreen.tsx`. `useLocalData.ts` holds the old local
  scan/cache/coalescing logic unchanged.
- Web build: `npm run build:web` → `dist-web/` (`--mode web`; publicDir `web/public` incl. `vercel.json` CSP
  headers and the apple-touch-icon; a small service worker for offline launch; no web manifest on purpose).
  Deploy: `TC_WEB_DEPLOY_DIR=… VERCEL_SCOPE=… scripts/deploy-web.sh`.
- Dev harness (never bundled): `pnpm dev` + `?mock=off|primary|viewer` (+`&mockerr`, `&stale`, `&nosync`,
  `&slowcfg`), `?source=remote`, `?crash=` for the error boundaries.
- Tests: `cargo test` (vectors, envelope, hash, backoff, config, lock fairness), ignored live tests need
  `TC_SYNC_TEST_CODE`; `node scripts/sync-selftest.mjs` (vectors, Rust fixture interop, gzip-bomb cap, live
  round trip with `TC_SYNC_TEST_CODE`).
- Gotchas found on install: (1) each ad-hoc-signed build is a new code identity, so macOS asks again for
  Documents-folder access (the `codex_exec` log dir can live there). Until someone answers, `open()` blocks
  inside `get_usage` while it holds `COMPUTE_LOCK`, which also stalls the publisher. Launching the binary from
  a terminal inherits the terminal's grant. (2) Two instances with the same bundle id share one WKWebView
  storage, so a second (viewer) instance writes the shared `sync-role-hint-v1`; it is rewritten from the real
  config on every launch. (3) Found the morning after: at one fetch a minute Anthropic's usage endpoint
  answered 200 and 429 in turn, so `claudeQuota.status` flipped ok/stale every minute and the publisher
  uploaded each flip (~60 uploads/h with nothing else changing). `cached_quota` (lib.rs, `quota_step`) now
  fetches at most every 3 min, backs off ×2 per consecutive 429 (cap 10 min; forced refresh bypasses), and
  keeps a good reading as "ok" through a 429 while it is ≤15 min old — stale only after that.

## Sync on demand + standby uploader (2026-11-10, v0.3.1)

- Server (`supabase/migrations/20261110000000_tc_sync_requests.sql`): `tc_request` stamps
  `refresh_requested_at`; `tc_poll(p_role)` is what an uploading Mac calls every 5 s (returns the pending request
  and ages computed on the server's clock; role `primary` also stamps `primary_seen_at`, at most every 20 s).
  Older clients are unaffected (`tc_head`/`tc_get`/`tc_put` unchanged).
- Rust (`sync.rs`, "Polling"): the publisher loop now has two clocks, `next_publish` (60 s, as before) and
  `next_poll` (5 s for an uploader, 30 s for a standby that is only watching; no polling for off/plain viewer).
  `decide()` is the pure step (unit-tested): answer each refresh request once while it is < 2 min old; a standby
  becomes active when `primary_alive()` is false (silent > 150 s; for a pre-polling main Mac: no upload for
  12 min) and uploads at once; any active uploader re-uploads when the server holds a version it didn't upload
  (the other Mac did), at most once a minute. `STANDBY_ACTIVE` gates `publish_cycle` for a standby; its uploads
  carry `p_device = "standby"` (a role tag, not a name).
- Config: `sync.json` `standby: true` (only stored when on), `sync_set_config(standby)`, status fields
  `standby` / `standbyActive`; the switch is in the desktop Sync sheet under *Show another Mac*.
- Viewers: `askForFresh` (TS `remote.ts`, Swift `RemoteStore`) = `tc_request`, then poll `tc_head` each second
  until `updatedAt >= requestedAt` (10 s cap), only if an uploader checked in within 60 s. Measured on the real
  channel: 2.7–2.8 s from request to new version.
- A standby uploads ITS OWN local usage (a Mac only has its own transcripts). Merging several Macs into one
  total would need per-bucket data with timestamps (range aggregates can't be added across day/week
  boundaries); not done.
- Tests: `cargo test` (decide/primary_alive/parse_poll/config), live: `TC_SYNC_TEST_CODE="$(cat <test code file>)"
  cargo test --lib live_ -- --ignored --test-threads=1` and the same variable for `node scripts/sync-selftest.mjs`.

## Native iPhone app + widgets (2026-11-09, `ios-app/`)

A read-only SwiftUI viewer of the synced snapshot, with WidgetKit widgets. User docs: `ios-app/README.md`.

- XcodeGen project (`project.yml`; the `.xcodeproj` is generated and gitignored). Personal values (team id,
  bundle prefix) live in the gitignored `Config/Local.xcconfig`. iOS 26+, Swift 6 mode. App icon is the Mac's
  `src-tauri/icons/AppIcon.icon`, referenced in place.
- `Sources/Shared/` is compiled into both the app and the widget extension: `SyncProtocol.swift` (CryptoKit),
  `Gzip.swift`, `Snapshot.swift` (the sanitizer), `SyncClient.swift`, `CodeStore.swift` (Keychain),
  `SnapshotCache.swift`, `Format.swift` (JS-exact rounding), `Dashboard.swift` (every number/label on screen,
  no SwiftUI), `WidgetViews.swift`. `Sources/App/`: `RemoteStore` (remote.ts port: 30 s head poll while
  active, get on a new version, failed-version memo, `dispose()` on unpair), views. `Sources/Widgets/`: the
  timeline provider (asks for a refresh every 15 min; entries at limit-window resets).
- Parity is enforced by tests, not by eye: `node ios-app/scripts/make-test-vectors.mjs` runs the web app's
  TypeScript and writes `Tests/Fixtures/vectors.json` (pairing codes, envelopes, gzip framings, coercions,
  toFixed ties, and what each range shows for four snapshots); the Rust envelope fixture is opened too.
  Re-run it after changing `src/sync/*.ts`, `src/format.ts` or the PopoverContent helpers it copies.
  `xcodebuild test -project TokenCounter.xcodeproj -scheme TokenCounter -destination 'platform=iOS Simulator,…'`.
- Gotchas:
  - Apple's Compression framework swallows the 8-byte gzip trailer without reporting where the deflate stream
    ended, so `Gzip.swift` supports one member only and reads CRC/size from the last 8 bytes.
  - `String(format:)` rounds exact ties to even, JavaScript's `toFixed` away from zero (12.5% → "13%"):
    always format through `JS.toFixed`.
  - `JSONSerialization` reads `-1e999` as −∞ and then raises (uncatchable) when asked to write it, so the cache
    stores the validated snapshot bytes as base64 and never re-serializes.
  - A widget is refreshed only every 15–60 min, so the app's "older than 15 min = stale" rule would flag it all
    the time. `Dashboard.widgetStaleSince` judges the snapshot's age at the widget's last server check, plus a
    2 h "never refreshed" fallback.
  - The pairing code is saved in the App Group's Keychain access group so the extension can read it
    (falls back to the app's own group when the App Group is missing; `CodeStore.shareWithWidgets` migrates).
  - A free Personal Team does get App Groups (verified on a device build). The team id shows up in
    `~/Library/Preferences/com.apple.dt.Xcode.plist` (`teamID`) once Xcode has opened a project with the
    account added. Profiles last 7 days: `ios-app/scripts/install-device.sh` rebuilds and reinstalls.
  - XcodeGen's iOS preset sets `TARGETED_DEVICE_FAMILY = "1,2"` per target; a project-level override is ignored.
  - Layout (2026-11-10, the user's call, in two steps): ONE card, a whole rounded rectangle (never cut
    straight by the screen edge). The numbers scroll in its upper part and fade out above the range switch,
    which is fixed at the card's bottom, in the thumb's reach. Tried and rejected by the user: the switch at
    the card's top (too far to reach) and as a separate floating bar below the card ("ugly"; merge it in).
  - Wallpaper (2026-11-10, the user asked for something dreamier and moving): a 4×4 palette whose control
    points drift on 4–6 s loops (at 10–15 s the user took it for a still image), 30 fps, paused with Reduce
    Motion or when the app is not active. Code: `Sources/App/Wallpaper.swift`.
  - Waves (2026-11-10, asked for: "like stirring an ocean of blue and orange", then "also when I shake the
    phone"): the wallpaper is a water surface (`WaveField`, a 10×22 height field with the wave equation,
    reflecting edges, damping). Fingers push it through a passive window-level gesture recognizer
    (`WaveTouchRecognizer`: never recognizes, prevents nothing, so scrolling and buttons are untouched);
    the phone's movement pushes it through CoreMotion (`MotionWaves` + `ShakeFilter`: the hand's acceleration,
    plus gravity only while it changes, so a tilt sloshes and a held tilt does not lean; no permission needed).
    While there are waves the gradient redraws every frame, then falls back to 30 fps.
    Drawn as one `MeshGradient` on a fixed regular grid; each frame only computes node colours (the drift
    and the waves shift where the palette is sampled), so the mesh can never fold.
    What made it visible (the first version was correct and invisible): (1) a crest takes its colour from
    further towards the orange corner and a trough from towards the blue (`tide`), because lighting alone
    does not show on a soft gradient behind glass; (2) the shown height is `tanh(gain · h)`, because a ring
    that has spread is a fraction of its splash. Strengths are in `WaveField` / `WallpaperFrame`; the user
    had not yet judged them on the phone when this was written, and shaking can't be tried in the Simulator.
    Simulator: `SIMCTL_CHILD_TC_PREVIEW_WAVES=swipe|tap|shake|tilt|watch` stirs it without a finger,
    `…_WAVES_LOG=1` prints the wave height (launch with `--console-pty`), and every strength has a
    `TC_PREVIEW_*` override (see the file). Stills barely show it: grab 7 frames 0.35 s apart into a sheet.
    Day: a saturated Claude orange top left (the user wants more chroma, not a lighter orange), Codex blue on
    the right, violet bottom left, under a thin veil; the white header text has its own soft shadow. The day
    card's glass is only 12 % white (`Tuning.dayCardTint`) so the wallpaper shows through; 50 % read as "too
    white, too solid".
    Night is NOT a dimmed copy of day: warm colours go brown when dimmed (the user rejected that twice), cool
    ones stay deep. So one small vivid orange glow in the top-left corner and every dark tone in wine, violet
    or blue (fall-off orange → wine → violet → blue). The user picked this ("G") from three rounds of
    candidates; "H" (same idea, less orange, more blue) was the runner-up. Rejected: a pink/violet palette
    ("too pink"), large bright fields ("too bright"), a uniformly dimmed orange ("ugly").
    To try a look without rebuilding (Debug, Simulator): `SIMCTL_CHILD_TC_PREVIEW_NIGHT` / `…_DAY` = 16
    comma-separated `#RRGGBB`, row by row; `…_CARD` (day glass whiteness) and `…_VEIL` are numbers. Rendering
    a labelled sheet of candidates for the user to pick from worked well. The web app still has its static
    wallpaper.
  - The card's gap to the bottom edge of the screen equals its side gap (`Card.margin`), so all four corners
    are concentric with the screen's; the home indicator then lies over the card's bottom margin, under the
    range switch.
  - Card corners are concentric with the screen's (`Sources/App/CardCorners.swift`). SwiftUI's
    `ConcentricRectangle` resolves per position (radius = container radius − the larger edge inset, so a card
    away from the corners drops to its minimum) and cannot be a `containerShape`; a custom
    `RoundedRectangularShape` returning `.concentric` corners makes nested shapes square. So a UIKit probe
    (`cornerConfiguration` + `effectiveRadius(corner:)`) reads the number once, cards use a fixed
    `RoundedRectangle` of that radius and declare it as `containerShape`, and the tool tiles are
    `ConcentricRectangle`s inside it.
  - Simulator-only debug hooks (`DebugPreview`, `SIMCTL_CHILD_TC_PREVIEW_*`) show a snapshot file, error
    states, the pairing screen and all widget families without a server. Synthetic simulator taps of zero
    duration are swallowed by SwiftUI buttons inside a ScrollView; tap with a duration of ~0.1 s.
- Not done: Lock-Screen widgets were only checked for layout in the debug gallery, not on a real Lock Screen;
  camera QR pairing needs a device.

## Liquid Glass app icon (2026-11-09)

The app icon is now a native Icon Composer document, `src-tauri/icons/AppIcon.icon`
(four flat SVG layers + `icon.json`), so macOS 26+ renders it with real Liquid Glass
and switches Default / Dark / Clear / Tinted itself. Design: "quota rings" — Claude
outer arc (orange on a white tile; orange on charcoal in Dark), Codex inner arc
(cobalt; light blue in Dark) filling the opposite way, hub dot, faint tracks
(neutral gray at 7% on white). The Claude glass group has translucency 0 in both
appearances so the orange stays saturated on white. The tile was orange with a white
arc until the user asked for white (2026-11-09); explorations live outside the repo.
It replaced the old dual-sparkline SVG (`app-icon.svg`, deleted).

- Edit the `.icon` (Xcode → Icon Composer, or the SVGs/`icon.json` directly), then run
  `scripts/build-app-icon.sh`. It regenerates everything: `Assets.car` via
  `actool`, and the flattened fallbacks (`icon.icns`, PNGs, `icon.ico`) from
  Icon Composer's `ictool` drawing the Default appearance as the 824 px tile of
  the 1024 px macOS grid, then `tauri icon`. Needs Xcode 26+.
- Wiring: `bundle.macOS.files` copies `icons/Assets.car` into
  `Contents/Resources/`, and `src-tauri/Info.plist` (merged by Tauri) sets
  `CFBundleIconName = AppIcon`. Older macOS falls back to `CFBundleIconFile` (`icon.icns`).
- Gotchas: actool's own fallback `.icns` stops at 256 px (don't ship it).
  `/usr/bin/ictool` is a different tool; the renderer is
  `Icon Composer.app/Contents/Executables/ictool --export-image ...`.
  `opacity-specializations` (per-appearance layer opacity) works but is not in
  Apple's public docs.
- Verified: installed bundle contains `Assets.car` + `CFBundleIconName`, and
  `NSWorkspace.icon(forFile:)` returns the Liquid Glass rendition (Dark while the
  system is dark). Screen capture is blocked on this machine, so the Dock itself
  was not screenshotted.

## Per-file Claude parse cache (2026-11-07)

Why: every `get_usage` re-parsed all ~2,100 transcripts (6 GB, ≈5.7 s), and
every write by a running session triggers one. Baseline of the installed
build, 60 s with a session writing: 32% of a core on average, 100% during
each scan, 21 s of CPU.

What (`usage.rs`, "Claude pass with a per-file parse cache"):
- `CLAUDE_CACHE` holds a `ClaudeFileParse` (rows, request groups, session
  spans) per path, keyed by `(len, mtime)` read *before* the file (as
  `codex.rs` does). A changed file is re-parsed whole; no tail parsing.
- `collect_claude` concatenates rows in `list_jsonl_files` order (dedup ties
  depend on it), merges `ReqGroups` as if rows were folded in that order
  (first file fixes `family`; `stopped |=`, `final_out`/`last_ts` max,
  first-seen blocks), and merges session spans (strict-greater `latest`).
- `find_latest_session`'s separate full read is gone: spans come from the same
  pass (any assistant row with sessionId + timestamp, usage or not).
- A different price-table `Arc` (refresh, ~daily) drops the whole cache:
  `cost_k` is baked in at parse and is a dedup tie-breaker. Cold parses run on
  up to 4 threads.
- Tests: `claude_cache_matches_uncached_scan` checks every step (cold, warm,
  append incl. an unterminated last line, truncate, same-length rewrite,
  price swaps, delete, new file) against `collect_claude_uncached`, the old
  loop kept verbatim under `cfg(test)`. `claude_cache_real_data` (ignored)
  does the same on an APFS snapshot: `cp -cR ~/.claude/projects <snap>/projects`
  then `CLAUDE_CONFIG_DIR=<snap> cargo test --release claude_cache_real_data
  -- --ignored --nocapture --test-threads=1` (it rewrites snapshot files).

Real data (2,124 files, 151k rows, 62k request groups, 293 sessions): cache
equals the old path cold, warm, with the 5 largest files cut mid-line and
regrown. Old sequential pass 6.6 s; cache cold 0.95 s (4 threads), warm
24–31 ms, 5 largest regrown 80 ms, finalize 50 ms; `compute_usage("week")`
cold 1.9–2.0 s, warm 0.10–0.11 s.

Memory trap found after the first install: merging with `extend` into an
unsized Vec (and an unsized groups map) made macOS malloc keep ~38 MB of freed
large blocks per scan; footprint went 0.5 → 0.96 GB within minutes, and a
40-scan probe grew linearly to 1.6 GB (it is retention, not a leak: it fell
back once, but far too late). Presizing both (`with_capacity` from the
per-file counts) keeps 30 consecutive `compute_usage` scans flat at 432 MB
in the probe. The cache itself stays resident (≈170 MB in the probe), where
the old path freed everything after each ~5.7 s scan; that is the trade.

Frontend: the "Updating… · cached" dim now waits 400 ms (`Hero`), since
most scans land in ~0.1 s and an instant dim flickered on every range switch.
Checked in the browser preview: no "Updating" on fast switches; with
`?usage=slow` it appears between 300 and 550 ms and clears when the scan lands.

## Cached numbers marked while a rescan runs (2026-11-07)

Symptom: the Week view read $1,986 and was taken for a miscount; seconds
later it read $2,439. Audit: the $1,986 payload matched a recount of the logs
at 11-06 22:43 ET to the token (Opus 5.5 in 72.1M / out 19.0M / cache read
3807M), i.e. a cached payload from the night before; the refreshed $2,439
matched an independent recount (Opus 5.5 $2,289 vs $2,290.21 a minute later;
Cowork/Duet = 2/96 of Claude Code each). No accounting bug.

Cause: `pickRange` (and the first render) shows the range's localStorage
payload at once, and only a finished `get_usage` of that range replaces it.
That payload can come from an earlier app run. A full rescan now reads ~6 GB
of transcripts: `smoke_compute_all` (5 ranges + the session pass) takes 34 s
in release, ≈5.7 s per pass, and a switch waits behind the in-flight scan
(coalescing), so an old number can sit there at full strength for ~6–12 s, or
longer right after launch while the warm-up queue scans the other ranges.
The limits row is polled separately and was current, which made the stale
headline look current too.

Fix (frontend only): `writeCached` also stores `usage-cache-v1:<range>:at`.
`cachedAt` state is set when a cached (or the previous range's) payload is on
screen and cleared when a fresh scan of the current range lands; while set,
the hero number is dimmed (0.45) and the delta row reads "Updating… · cached
<time>". Browser preview: `?usage=slow` delays the sample by 5 s.

Follow-up (same day): per-file Claude parse cache, see the section above.

## Keychain token auto-refresh + Claude product colors (2026-11-06)

Symptom: the week's Chat/Cowork/Duet split stayed frozen from 11-04 midday.
Cause: the terminal Claude Code was signed out at that time (a "Not logged
in" transcript from 11-04, right after two desktop-app update restarts;
whether the update caused it is unproven), so `Claude Code-credentials` held
only `mcpOAuth` and no `claudeAiOauth`. Claude Code in the desktop app
authenticates through the app and never writes or refreshes that item. Fixed
for the moment by `claude auth login` in Terminal; the split did not actually
change (95/0/4/1), so `week_breakdowns.json` was correctly left alone.

So the read stops going stale, `claude_refresh.rs` (new, at the user's
request) refreshes the item itself, reversing the read-only rule below:

- Only after `/api/oauth/usage` answers 401. Re-reads the item first and uses
  a newer access token if one appeared.
- Skipped while any transcript under `~/.claude/projects` with an
  `"entrypoint"` other than `claude-desktop` was written in the last 2 min
  (terminal `claude`, `claude -p`, SDK): that process refreshes on its own.
  Desktop transcripts don't count, otherwise a desktop-only user would never
  refresh. During such a window the limits stay stale for a few minutes.
- The request mirrors Claude Code 2.1.317 (read from the binary):
  `POST https://platform.claude.com/v1/oauth/token`, JSON `grant_type`,
  `refresh_token`, `client_id` (Claude Code's), `scope` = stored scopes.
- Write: `security add-generic-password -U -a <item acct> -s … -X <hex>`, via
  `security -i` stdin when the line fits 4000 chars, else argv (the blob is
  ~10 KB with MCP logins, so argv) — the same as Claude Code. Done under
  Claude Code's own write lock, proper-lockfile at
  `~/.claude/.storage-write.lock` (mkdir; stale after 15 s); a busy lock does
  not block the write.
- Compare-and-swap: writes only if the item still holds the refresh token we
  spent, or an empty one (Claude Code blanks a refresh token it saw rejected).
  All other blob fields are kept. The account comes from the item's `acct`
  attribute (fallback `$USER`) and is resolved before the request.
- Once the token request succeeded, nothing drops the new tokens: no lock →
  write anyway; re-read fails → use the pre-request blob; both writes fail →
  kept in memory (`Pending`) and written on the next 401.
- invalid_grant on an unchanged item → "signed out, run `claude auth login`",
  and that refresh token is never retried. Other failures back off 10 min.
- Refresh tokens have their own lifetime (`refreshTokenExpiresAt`, ~30 days
  from login; a refresh did not extend it), after which a new
  `claude auth login` is needed.
- Claude Usage Monitor refreshes the same item without the lock or the
  blank-token case, so running both leaves a small race.

Verified: `cargo test` (10 new unit tests: response merge, request body, CAS,
invalid_grant, entrypoint guard, lock takeover, hex, acct parsing), clippy
`-D warnings`, fmt; ignored `smoke_keychain_round_trip` (throwaway item,
stdin and argv paths); ignored `smoke_refresh_real` forced one real rotation:
both tokens changed, expiresAt now+8h, blob keys and MCP entries unchanged,
`fetch()` ok, and `claude auth status` still logged in afterwards.

Colors: `claude-chat`/`claude-cowork`/`claude-duet` and unknown Claude
families use `FAMILY_COLORS.other`, now Claude orange `#D97757` (was brown
`#8B6F47`, which the user disliked).

## Chat, Cowork and Claude Duet estimates (2026-11-02)

None of the three leaves a local transcript, and no endpoint gives their
tokens. `/api/oauth/usage` does return `seven_day_breakdown.rows`: each
product's percentage of the week. `usage.rs` keeps the latest reading per week
in app-data `week_breakdowns.json` and prices each Claude entry in that week:

    product $ = Claude Code $ × product % / Claude Code %

- Ranges: Week, Month and All only. A weekly percentage says nothing about a
  day or a session. By-model rows `claude-chat`, `claude-duet`, `claude-cowork`
  have a cost and no tokens; the sparkline, delta and store stay logs-only.
- `totalCost` and `totalCostEst` include them. No "est." marks, per the user's
  standing request.
- Whole percentages make this rough: a 2% row is ±25%, and a change of one
  point moves Duet by about $14 at $1,100 of Claude Code spend, so all-time
  deltas wobble by that much.
- Tied to the OAuth read, so a week with no good read adds nothing, and earlier
  weeks can't be filled in. The first recorded week starts 2026-10-28.
- The Cowork backfill below ends 07-10, so the two never cover the same days.

## Accuracy audit + stale Claude limits fallback (2026-11-02)

Audit (independent Python recount of all 1836 transcripts, 43.9k requests):
Claude tokens in `daily_usage.json` match the recount on every October+ day
except 10-24 (+10.8M tokens, $4.23: a transcript gone from disk, kept by the
store) and the current day (scan timing). Rates match the official pricing page
for every model on disk. `claude -p --output-format json` is the ground truth
for logged spend: its `modelUsage.costUSD` for Opus equals our formula to the
digit ($0.4937802), and the transcript holds exactly its tokens.

- ccusage 20.0.47 is below us by $38.57 since 10-01 ($44.77 on disk). On
  multi-iteration (advisor) requests the top-level `usage.cache_creation`
  holds the first `message` iteration only, while
  `cache_creation_input_tokens` is the sum. ccusage prices the breakdown and
  drops the rest. We keep the total, and pricing the remainder as 5m is right:
  summing the iterations' own 5m/1h split changes our cost by $0.02 all-time.
- The workflow-output estimate is history: since 10-22 only stray unlogged
  requests remain (2.1.309 wrote a final row for all 4728 workflow requests
  since 10-28). $206.80 on disk.
- Not logged anywhere, so ccusage never sees it (the last three are now
  estimated, see the section above): WebSearch's Haiku side call
  (≈$0.012 each, $0.01 of it the search fee; 2115 in October ≈ $25); WebFetch's
  (≈$0.0002 on a small page); Claude Duet, Cowork and Chat (14% / 4% / 2% of the
  week of 10-28, per `seven_day_breakdown.rows`).
- Cowork through 07-10 sits in
  `~/Library/Application Support/Claude/local-agent-mode-sessions/**/.claude/projects`
  (311 files, 5792 requests, $763.03, 03-27..07-10). The app doesn't scan it.
  Backfilled once, at the user's request, by adding those tokens to
  `daily_usage.json` per day and model (app quit first; 94 days, 56 of them
  new). Not a scan root: its rows would max-merge with Claude Code rows under
  the same model ids, and rescanning 1.2 GB gains nothing, since no local log is
  newer than 07-10 although Cowork still uses quota. All-time stepped up once
  by $763 at 02:05 on 11-02, so an all-time delta across that time is off. The
  tokens added are in `cowork_backfill_20261102.json` (its presence means don't
  apply again), and the pre-merge store is
  `daily_usage.json.bak-20261102-pre-cowork`. 0 of the 5792 requests overlap
  the transcripts on disk, including the $56 of days both still cover. Python's
  recursive glob skips the hidden `.claude` directory; use `os.walk`.
- The store exceeds disk by $379 on days ccusage also has. Mostly whole
  requests gone from disk: output and cache rise together, no negative rows,
  and `07-02` has none left. On the largest rows input equals live exactly,
  because the advisor backfill raised live input past the stored value, so the
  store under-records a deleted session's input. Three model-days (07-27,
  08-18, 08-20) match a recount in another timezone: a day re-keyed after a
  timezone change was max-merged, a one-time double count of about $25–50.
  All-time starts at 05-31 in every backup.
- Other `CLAUDE_CONFIG_DIR`s on the machine are not scanned.

Fix: the Claude limits row showed 5h 0% / weekly 3% against live 40% / 74%.
The Keychain token had expired on 10-29 and the desktop app never refreshes it;
"stale" appeared only in the tooltip. The last good snapshot was 10-28; why
nothing landed before the token expired is unknown. The token was refreshed
during the audit, at the time of two `claude -p` runs (cause not proven), and the
live read came back on its own.

- `claude_limits.rs`: `with_desktop_sample`, on any non-ok status, uses the
  newest sample of the desktop app's `plan-usage-history.json` if it beats the
  snapshot. Samples are sparse, so 5h can be well behind (0% vs 40% in the
  audit); `resets_at` is sample + 5h, and weekly steps the old reset forward.
  Applied after `cached_quota`, so it never reaches `claude_quota.json` and
  `week_window` stays on real resets. No OAuth refresh, on purpose. No org
  filter: all 57 samples are one org, and the newest is the current account's.
- `LimitsRow`: `staleAsOf` dims the meters and prints the reading's time.
  Browser preview: `?quota=stale`.

Verified: `cargo test` 64 passed / 5 ignored, clippy `-D warnings`, `cargo fmt
--check`, WebKit preview light+dark (row stays 35.5px, no overflow), the real
sample file via `smoke_desktop_sample_real`. Installed 11-02 01:47. The stale
branch has not been seen in the real popover: the live read works again.

## Week = official weekly-limit window; output estimate folded into totals (2026-10-21)

User: "I burned the whole week's Fable, it can't be only $200." Audit (no code
change): the app matched the logs to the cent — the quota week 10-14 05:00Z →
10-21 05:00Z had Fable 5.1 $229.49 (independent oracle = `daily_usage.json` =
ccusage@latest; October Claude $1,653.65 = ccusage too, all October Fable is
10-17+, i.e. after the Max upgrade). The ~$200 on screen came from "This week"
starting calendar Monday plus the hidden workflow-output estimate (+$45 for
that week). Fable hit its limit 10-20 20:34Z (`<synthetic>` "You've reached
your Fable limit", no apiErrorParams). Fable eats ~2.6× the quota of Opus per
API$ (pure-Opus segment $16.4/weekly-point, clean Fable segment $6.4), so a
full Fable allowance (50% of weekly) is only ~$300 API$; Chat/phone/web use
counts against the quota with no local log.

Changes, at the user's request:
- `usage.rs`: `Range::Week` and its sparkline start at the official weekly
  reset (`week_window()`): `claude_quota.json` (last good snapshot written by
  get_claude_quota) `weekly.resetsAt`, snapped to the minute (the endpoint adds
  sub-second jitter), stepped by 7 days to the window containing now (a stale
  snapshot still works). No snapshot ever → Wednesday 01:00 local (the
  user's reset as they know it; `last_wednesday_1am`, test
  `fallback_week_starts_wednesday_1am`). Spark labels "Wed 1am" … "now". cfg(test) never reads the snapshot.
  Test: `week_start_follows_official_reset`. Codex has its own weekly reset;
  the mixed Week view follows Claude's. Review follow-ups (same day): the
  window is read once per scan and passed to `spark_config`; the Week
  sparkline uses 6h buckets trimmed to now (≤28) instead of 7 fixed days; the
  last reset seen is kept in app-data `week_anchor` (minute-snapped) for when
  an ok snapshot lacks `weekly.resetsAt`; `save_quota_cache` writes tmp+rename;
  the fallback uses date arithmetic (24h steps drift across DST changes).
- Month now reads the store too (`sum_store_since(store, 1st of month)`, plus
  a per-day sparkline from the store), like All. Live-only Month had missed
  $56.71 of 10-08/10-10 Codex (chats deleted in the Codex app; the store kept
  them). Claude October store = live to the cent. The 10-21 Codex
  re-derivation (`reset_rederivable_codex_days`) had replaced 10-12's stored
  Codex with the surviving rollouts only, dropping ≈$13.6 of a deleted chat;
  restored at the user's request by hand (app quit, field-wise max with
  `daily_usage.json.bak-20261019-pre-advisor`, pre-edit copy
  `daily_usage.json.bak-20261021-pre-1012-restore`). That function still
  drops a deleted chat's share whenever some Codex of the same day survives.
- Workflow-output estimate: the user first asked to show it again, then (same
  day) to drop the separate "+$X est." line and simply count it. Now
  `fold_output_estimate` adds `outEst` into `ModelUsage.out` before `cost_k`,
  so per-model `cost`, `totalCost` and `totalTokens` include it; `outEst` /
  `costEst` / `totalCostEst` stay in the payload as "the included part". The
  sparkline and daily delta add the same estimate per request
  (`series_cost`, output rate per model). NOT folded: the local 5h/7d
  credit/token windows (`credits`, `tokens_5h/week`) and tool-token counts.
  The store still keeps raw features (no ratchet). Frontend unchanged (no
  est. marks anywhere). All-time/Month/Week therefore step up once by the
  estimate (≈ +$142 for October, +$197 all-time on 10-21).

## `codex exec --json` logs (2026-10-21)

After 81b0402 the Month view still showed GPT-6 Astra $56 while the live
/wham/usage read said 5h 100% / weekly 79% and no Codex request on this Mac
was in either window. A launchd job of the user's runs
`codex --search exec … --ephemeral --json` every ~12 min whenever quota
allows; `--ephemeral` writes no rollout and no thread row, so 119 runs
(10-13..10-20: 113.8M input, 105.3M cached, 1.39M output ≈ $260 at Astra
rates) were invisible. The --json stream of each run is kept in its
`control/logs/<YYYYMMDDTHHMMSS>-<pid>.jsonl`.

`codex_exec.rs` reads such logs from the dirs listed in
`codex_exec_logs.json` (app-data dir; `{"sources":[{"dir":…,"model":…}]}`,
`model` required, unknown keys rejected). One run = one file: thread.started
thread_id, the summed `turn.completed.usage`, tools from item.completed
(command_execution→shell, file_change→apply_patch, web_search). Time = file
name stamp (UTC), else mtime. No `-long` (turn totals span requests);
`turn.failed` runs report no usage (27 of 119, undercount). A thread that also
has a rollout is skipped. Never the "current session".
`CodexData.exec_sources` fingerprints the sources; the `codex_store_version`
marker is now `"2"` or `"2\n<dir>=<model>…"`, so adding or editing a source
re-derives the on-disk Codex days (no stale-id double count); an unreadable
config/dir yields None and skips that re-derivation.

The watchdog keeps `--ephemeral` (its test asserts it; dropping it would make
worker rollouts take over "Current session", may start Codex's memories
pipeline, and lets the currently failing `collab spawn … invalid thread-store`
subagents run).

## Codex coverage, tiers and official Codex limits (2026-10-21, branch `claude/gpt6-billing-issue-dgyxwc`)

Reported symptoms: Month GPT-6 $28 (in 1.3M / out 49K / cached 12.1M) while
the user had exhausted Codex limits 4-5 times, and Codex limits stuck at
5h 0% / wk 0%. Prices were right (that row is GPT-6 Astra at $10/$1/$50 —
the UI dropped the -astra/-sol/-luna variant); token coverage and the quota
source were wrong. Researched against openai/codex main (2026-10-20, latest
stable 0.163.0). No access to the user's ~/.codex, so which coverage gap
dominates on their Mac is unconfirmed; ask for
`find ~/.codex -name 'rollout-*.jsonl.zst' | wc -l`,
`ls ~/.codex/archived_sessions | wc -l` and `echo $CODEX_HOME`, or run
`cargo test --release smoke_collect_real_dir -- --nocapture` (prints rollout,
compressed and archived counts and per-model totals).

`codex.rs` (rewritten):
- Files: `$CODEX_HOME` (default ~/.codex) `sessions/**` **and** the flat
  `archived_sessions/` (Codex's archive action renames threads there),
  `rollout-*.jsonl` and `rollout-*.jsonl.zst` (`zstd` crate). Compression
  (≥7 days idle) is off by default in OSS Codex but runs on the app-server
  `rollout/compress` RPC (0.156+), so the desktop app/IDE may trigger it. A
  plain sibling wins over its `.zst` (both exist mid-transition). Temp files
  are excluded by the strict `rollout-*.jsonl[.zst]` match.
- Usage: `token_usage_record` lines (0.153+, one per response incl. local and
  remote compaction and, 0.163+, incomplete responses) are counted from
  `payload.usage`, own thread only (`payload.thread_id` == first
  session_meta id), globally deduped by `response_id`. `token_count`
  `last_token_usage` is counted only when no record appeared since the
  previous token_count (older stretches); remote compactions never appear in
  token_count, so the old rule missed every one. Stale-`last` repeat (0.163+
  incomplete response without a record) uses Δtotal.
- Copies: legacy forks re-write the parent's token_count lines with fresh
  timestamps; for fork/subagent files (session_meta `forked_from_id`,
  `parent_thread_id` or `source.subagent`) a token_count pair already seen
  earlier in another file is dropped. Paginated subagents' inherited prefix
  (`ordinal` < `subagent_history_start_ordinal`) is skipped. Only the FIRST
  session_meta names the thread.
- Model: `turn_context` by `turn_id`, else latest `turn_context` / own
  `thread_settings_applied.thread_settings.model`. Tier: own
  `thread_settings_applied.thread_settings.service_tier` →
  `-fast` (priority), `-flex`, `-ultrafast`; prompts > 272K → `-long`
  (before the tier). `cache_write_input_tokens` → ModelUsage.cacheWrite.
- Per-file scan cache keyed by (len, mtime); response_item lines that are not
  tool calls are skipped before JSON parsing.
- Rollout quota fallback: only `rate_limits.limit_id` == "codex" (or absent,
  old files) with object windows; 5h/weekly by `window_minutes` (±5%), each
  window its newest value. Codex keeps ONE latest-snapshot slot that every
  bucket overwrites (`codex_other`, per-model, metadata-only nulls), so the
  newest line is usually not the main bucket — that, plus `"primary": null`
  parsed as 0%, was the 0%/0% bug.

`codex_limits.rs` (new): `GET https://chatgpt.com/backend-api/wham/usage`
(what Codex /status polls), `Authorization: Bearer tokens.access_token`,
`ChatGPT-Account-Id: tokens.account_id` (or the id_token claim). Credentials
from `$CODEX_HOME/auth.json`, else Keychain service `Codex Auth`, account
`cli|` + 16 hex of sha256(canonical CODEX_HOME). Read-only: never refreshes
(Codex refresh tokens are single-use). Windows by `limit_window_seconds`
(18000 / 604800), `rate_limit_reset_credits.available_count` in the tooltip.
`get_codex_quota` shares lib.rs's generic 60s cache + persisted
`codex_quota.json` with the Claude quota. UI prefers it; falls back to the
rollout quota ("from session logs"). Untested against the live endpoint from
here (no ChatGPT login in the dev container); the User-Agent is
`token-counter/0.2` — if OpenAI rejects non-Codex agents the row falls back.

Pricing (`usage.rs pricing_with`): OpenAI tier multipliers from LiteLLM's
`*_priority/_flex/_ultrafast` rows: priority 2× (2.5× gpt-5.5), flex 0.5×,
ultrafast 6× (published for gpt-6-astra / gpt-6.1-sol only; else estimate);
`-long` 2× input kinds / 1.5× output, stacking. Codex cache writes 1.25×
input for gpt-6*/gpt-5.6*/daybreak (LiteLLM row wins), else input rate. The
bundled Codex catalog defaults gpt-6-sol and gpt-6-luna to Fast ("priority").
Claude: `-fast` only for `usage.speed == "fast"`; `service_tier: "priority"`
(Priority Tier) is only counted. Usage counters accept JSON null.
`CLAUDE_CONFIG_DIR` is honored. Built-in rows added: Mythos 5.1/5 (official,
verified), daybreak blue/red (LiteLLM, estimate). Sonnet 5.5 cache read stays
$0.10 (official page 2026-10-20).

Frontend: Codex labels keep the variant and tags (GPT-6 Sol Fast, GPT-6 Astra
272K+), Claude fast rows say "Fast".

All-time store: `CODEX_STORE_VERSION` ("2", marker file `codex_store_version`
next to daily_usage.json). On the first scan after a bump, stored Codex
tallies of days whose Codex usage is still on disk are replaced instead of
max-merged, so old ids (e.g. `gpt-6-sol` now billed as `gpt-6-sol-fast`)
don't linger beside the new ones. Days no longer on disk keep their figures.

Verified (Linux dev container): `cargo test` (43 passed, 4 ignored), `cargo
fmt --check`, `cargo clippy --all-targets` (only the Linux-only unused
`Manager` import warning, pre-existing), `pnpm build`, browser preview
light+dark. Synthetic CODEX_HOME (paginated Fast GPT-6 Sol with records,
remote compactions, a >272K call and bucket overwrites; archived .zst legacy
Astra with double emission; a legacy fork copying it) matches an independent
Python oracle exactly ($17.84, quota 78%/70%). The previous code on the same
data: $11.52 and quota 5%/0% (a `codex_other` snapshot); it missed every
remote compaction and the Fast multiplier, and saw the archived thread only
through the fork's copy, dated at the fork.

Residual gaps: Codex cloud tasks / ephemeral threads / memories never reach a
rollout but use the plan quota; `codex-auto-review` price unknown (estimate);
Claude workflow-subagent unlogged output (estimated and, since 2026-10-21,
included in the totals) and Claude Code cloud sessions (transcripts never reach
the Mac).

## Liquid Glass content restyle (2026-07-18, committed + pushed)

The native glass was already real (tauri-plugin-liquid-glass 0.1.6 →
NSGlassEffectView on macOS 26; lib.rs unchanged). This pass restyled the
CONTENT layer to Apple's Liquid Glass design language:

- `theme.ts` — neutral colorless palettes (system label-style ink levels);
  dead tokens (glassTint/sheenGrad/lensGrad/specularGrad/…) removed; new
  seg*/btn*/tile* tokens. `?theme=dark|light` URL override for previews.
- `PopoverContent.tsx` — SF Pro throughout (serif hero dropped), capsule
  segmented range control (recessed trough + floating glass thumb), circular
  glass title-bar buttons, capsule meters, 14px concentric tool chips,
  uniform glass bottom (bottomBg removed).
- `format.ts` — `liftForDark()` accent lift so deep ramp colors (GPT-5.5
  navy) stay visible on dark glass; applied to model rows when dark.
- `App.tsx`/`GlassSurface.tsx` — browser preview now uses a blue-purple
  wallpaper + backdrop-filter frosted stand-in (real app: native glass only).
- Verified via headless-Chrome screenshots light+dark at 520px (screencapture
  is blocked — no Screen Recording permission for the CLI host; use
  `--headless=new --screenshot` instead). Pass `'--screen-info={1920x1200}'`
  too: headless screen = window size trips GlassSurface's fullscreen check
  (2× zoom). Chrome doesn't exit by itself (app timers); kill it once the PNG
  exists.

## Current state

Everything below is **committed and pushed** to origin/main as of
2026-07-18 (v0.2.1: GPT-5.6 pricing fix + Liquid Glass restyle, plus the
v0.2.0 dual-provider feature described here). The v0.2.0 feature expanded
the former “Claude Code Token Counter” into **Token Counter**, covering
both Claude Code and OpenAI Codex.

Main v0.2.0 files:

- `src-tauri/src/codex.rs` — new Codex rollout parser and tests.
- `src-tauri/src/claude_limits.rs` — read-only official Claude 5h/weekly quota
  fetcher, adapted from `~/Projects/claude-usage-monitor`.
- `src-tauri/src/usage.rs` — merges Claude + Codex usage, pricing, ranges,
  sparklines, tools, and all-time persistence.
- `src-tauri/src/lib.rs` — watches both transcript roots and serializes scans.
- `src/PopoverContent.tsx`, `src/Sparkline.tsx`, `src/format.ts` — dual-provider
  UI, blue Codex styling, quota meters, and merged tool/model displays.
- `src-tauri/tauri.conf.json`, `index.html`, `README.md` — v0.2.0 rename/docs.

## Claude official limits

Source: `GET https://api.anthropic.com/api/oauth/usage`, using the OAuth access
token from the macOS Keychain item `Claude Code-credentials`.

- Read-only until 2026-11-06. Since then an expired token (401) is refreshed
  and written back under the guards in the 2026-11-06 section at the top.
- `get_claude_quota` runs independently of the expensive JSONL scan, caches
  successful responses for 60 seconds, persists the last good snapshot under
  the existing Token Counter app-data directory, and returns it as `stale` on
  transient failure.
- The UI polls every 60 seconds and the title-bar refresh button forces a fresh
  request. Expired reset timestamps render as 0%.
- “By model” displays only the 10 highest-cost models after sorting. Headline
  cost/tokens, persistence, and sparklines still aggregate every model.
- Real Keychain/endpoint smoke on 2026-07-03 returned status `ok`, 5h 100%,
  weekly 44%. The test prints percentages only, never credentials.

## Codex data rules

Source: `$CODEX_HOME/{sessions,archived_sessions}/**/rollout-*.jsonl[.zst]`
(see the 2026-10-21 section for records, copies, tiers and the quota).

- Attribute each `token_count` to the latest preceding
  `turn_context.payload.model`.
- Skip consecutive duplicate `(total_token_usage, last_token_usage)` pairs;
  older rollouts emit identical token events twice.
- Sum `last_token_usage`, not the final cumulative total, because totals can
  reset when a process appends to an existing rollout.
- `input_tokens` includes `cached_input_tokens`; subtract cached input before
  applying the full input price.
- Rollout quota: newest main-bucket (`limit_id` "codex") window only; the
  live /wham/usage read supersedes it (2026-10-21).
- Both absolute `resets_at` and legacy relative `resets_in_seconds` are
  supported. Expired windows are normalized to 0% during every scan.

## Codex follow-up fixes

Codex resumed the interrupted Fable 5 session and reviewed the five findings
from workflow `w4s8r2e99`. Two related quota issues were real and fixed:

1. A quota snapshot could remain visibly stale after its reset time because an
   unchanged backend payload did not trigger a React render.
2. Legacy rollouts used `resets_in_seconds`, so their quota could never acquire
   an absolute expiry time.

Regression tests:

- `old_quota_schema_gets_absolute_reset_times`
- `expired_quota_windows_are_zeroed_on_each_scan`

The full-DOM parser concern is a performance improvement, not a confirmed
correctness defect at the current real dataset size (106 rollouts, about 44 MB).
The GPT future-major color-ramp concern is not reachable with current models.
Renaming `productName` intentionally creates `Token Counter.app`; an old
`Claude Code Token Counter.app` must be removed during installation to avoid
parallel copies sharing the same bundle identifier/data store.

## Verified 2026-07-03

- `cargo test` — 16 passed, 0 failed, 1 real-endpoint smoke ignored by default.
- `cargo test smoke_fetch_real -- --ignored --nocapture` — passed.
- `cargo clippy --all-targets -- -D warnings` — passed.
- `cargo fmt --check` — passed.
- `pnpm build` — passed.
- 520px browser preview — Claude/Codex limit rows are 35.5px each with no
  horizontal overflow.
- Real-history five-range smoke — passed.
- `pnpm tauri build` — produced
  `src-tauri/target/release/bundle/dmg/Token Counter_0.2.0_aarch64.dmg`.
- Installed `/Applications/Token Counter.app` 0.2.0 contains the
  `get_claude_quota` command and is currently running. Both old
  `Claude Code Token Counter.app` 0.1.0 copies and the pre-limits 0.2.0 copy
  were moved to Trash.
- The installed app persisted an `ok` Claude snapshot with 5h 100% / weekly
  44%, proving its production binary can read Keychain and reach Anthropic.
- `git diff --check` — clean.

Official prices were rechecked on 2026-10-16 against the raw pages
(developers.openai.com/api/docs/pricing, platform.claude.com/docs/en/about-claude/pricing.md;
the OpenAI tables are Astro-serialized JSON inside the HTML; decode them raw
rather than trusting a WebFetch summary):

- GPT-6 Astra/Sol/6.1 Sol/Luna were missing and fell through to the unknown
  fallback ($5/$0.5/$30; Astra is $10/$1/$50). The fallback now uses Astra's rate.
- GPT-5.6 changed after 2026-07-17: Terra $2/$12 and Luna $0.2/$1.2 by
  2026-08-02; Sol $4/$20 promotional "at least through 2026-11-21" (list $5/$30).
  History is repriced at the current table, so July Sol usage now shows lower.
- Claude rows are versioned: Opus 5.5 is $4/$20 with $0.20 cache reads (it had
  been priced as generic Opus $5/$25/$0.50, overstating it ~46%); Sonnet 5/5.5
  $2/$10; Haiku 5.5 $0.10/$0.50; Fable 5.1 cache reads $0.25. Fast mode is 2× on
  Opus 4.8/5/5.5; legacy Opus 4.6 keeps its published 6×. Any other `-fast`
  id (e.g. a `service_tier=="priority"` event on Sonnet) gets a 6× guess and
  is flagged as an estimate — zero such events in the data so far.
- The Codex Fast tier (`thread_settings_applied.service_tier`) and
  `cache_write_input_tokens` were unhandled here; both are priced since
  2026-10-21. `codex-auto-review` keeps its legacy estimate.

## Remote price layer (2026-10-16)

`src-tauri/src/prices.rs` keeps LiteLLM's
`model_prices_and_context_window.json` (the table ccusage uses) at most a day
old: `lib.rs` loads the disk cache synchronously at startup
(`litellm_prices.json` in the app-data dir), then a thread checks hourly and
does a conditional GET (ETag) once the table is >24h old. Only bare-id
`anthropic`/`openai` entries with standard-tier rates are kept; a parse with
<50 models is rejected and the previous table stays in use.

`usage.rs pricing_with(id, table)` order: exact id in the LiteLLM table →
built-in row if the id is in `BUILTIN_VERIFIED` → family guess (`estimate`).
Missing remote cache rates fall back to the verified built-in row, then to
1.25×/2×/0.1× of the remote input. Claude fast mode multiplies whichever base
wins. Each scan snapshots one table (`prices::current()`); tests never load it.
`ModelUsage.priceSource` marks estimate rows in the payload (the UI's "est."
pill was removed at the user's request on 2026-10-19).
`normalize_model` keeps any `claude-*` id so new families are flagged rather
than dropped.

Because history is repriced at the current table, every effective-rate change
for a stored model is appended to `price_changes.jsonl` (app-data dir) after a
refresh — check it when all-time jumps. On 2026-10-16 the live LiteLLM table
matched the built-in rows for all 18 stored ids except `codex-auto-review`
(not in LiteLLM → estimate).

Safeguards (from the 2026-10-16 adversarial review):

- A remote row that disagrees >2× (input, output or cache read) with a
  `BUILTIN_VERIFIED` row is treated as an upstream typo: the verified rate is
  kept and the row's `priceSource` becomes `estimate` (not visible in the UI
  since 2026-10-19 — check the payload / `price_changes.jsonl`). Such a flip on
  a model that used to look fine means LiteLLM and the official built-in row
  conflict — check the raw official page and update the built-in row. Price changes within 2× (e.g.
  the GPT-5.6 Sol promo ending) apply automatically; larger ones (like the
  Aug 2026 Luna $1→$0.2 cut) wait for a built-in update.
- Codex write rates stay 0 regardless of LiteLLM (no cache-write tokens).
- A rejected download (<50 usable models) backs off for 24h instead of
  re-downloading hourly; a `fetched_at` in the future counts as stale; rows
  loaded from the cache file get the same bounds check as a download
  ($0–$1,000/MTok).
- "By model" is a plain top 10 (estimate rows used to bypass the cut to keep
  their pill visible; both removed 2026-10-19).
- Table health is on `Usage.priceHealth` (`prices::health_at`, computed for
  the scan's snapshot) and shown at the right of the "By model" header:
  nothing <48h, muted "Prices 2d old" at 48–72h, amber warning past 72h (or an
  unusable/future `fetched_at`) and "Price update failed" after a failed
  attempt; "Built-in prices" when no table is loaded (amber if a fetch
  failed). The last error is in-memory only and is cleared only by a
  completed 200/304 check, so it stays visible through the 24h rejection
  backoff. Tooltip has the check time and error text. Browser preview:
  `?prices=aging|stale|error|builtin|builtin-error`.

When updating `BUILTIN_VERIFIED`/the built-in rows, verify against the raw
official pages, not LiteLLM, so the fallback stays independent.

## Advisor calls + unlogged subagent output (2026-10-19, committed + pushed)

Two systematic Claude undercounts, both in `usage.rs` (`parse_claude_line` →
`finalize_claude`, which wraps `dedupe_entries`).

**Advisor (exact).** A row using the advisor tool has
`message.usage.iterations[]` items with `type:"advisor_message"` and their own
`model`, typically ~240k uncached input + ~17k output, ≈$1.3 each. Top-level
usage is the sum of the `type:"message"` iterations only (checked on every
kept row with iterations), so advisor calls are billed on top:

- Parsed per row into `Entry.advisor` (lenient `serde_json::Value`, so an odd
  item can't drop the line), kept out of `total_tokens`/`cost_k` (dedup
  tie-breakers), and turned into entries only after dedup, from the kept row.
  The final usage is repeated on several block rows of one request (1926 rows
  with advisor → 393 requests), so per-row counting would bill it ~5×; the
  kept row carried the advisor in 393/393 groups.
- Attribution: merged into the advisor's own normalized model row (e.g.
  `claude-opus-5-5`, even under a Fable parent), at standard speed (no `-fast`
  inherited). A suffixed id like `…-advisor` would miss both LiteLLM and
  `BUILTIN_VERIFIED` and be flagged as a price estimate (`priceSource`). The advisor entries
  count everywhere exact usage does: by-model, store, sparkline, daily delta,
  5h/7d credit and token windows.
- Only `advisor_message` is added. One `fallback_message` row (2026-07-01) has
  top-level = the fallback iteration (opus-4-8) while an abandoned Fable 5
  `message` iteration is uncounted — a known 1-row gap. Never re-derive
  top-level usage from `iterations`.
- Effect: all-time steps up once by the advisor history still on disk ($431.65
  from 2026-07-01 on, as of 2026-10-19); already-pruned days can't be backfilled.

**Workflow subagent output (estimate).** `subagents/workflows/wf_*/agent-*.jsonl`
often keeps only a request's partial streaming rows (`stop_reason` null,
`output_tokens` 3–9); the real output (thinking included) is logged nowhere.
For a `(message.id, requestId)` group with no `stop_reason` on any row whose
last row is ≥15 min old (`EST_SETTLE_MINUTES`; 29 such groups were still
streaming during the 10-19 check — without the guard the store would keep their
estimate after the real usage lands):

    out ≈ visible × V + signature × S − already-logged output, floored at 0

- V = UTF-8 bytes of text + thinking text + compact tool_use input JSON, S =
  thinking signature (+ redacted_thinking data) bytes. Blocks are deduped per
  group by a u64 key (tool_use id / signature / text hash); server blocks
  (`server_tool_use`, `advisor_tool_result`) are skipped. Thinking text is
  usually empty now — the signature carries the length.
- Coefficients are refit every scan per family (`OutCoefs::fit`): OLS through
  the origin on all complete groups (main + subagent + workflow pooled); needs
  ≥200 samples and both coefficients >0, else the 10-19 fallbacks (Opus
  0.44/0.27, Fable 0.42/0.31; Sonnet/Haiku/unknown use Opus). 10-19 fit: Opus
  n=15.8k a=0.44 c=0.27 (time halves 0.42/0.28 vs 0.46/0.25), Fable n=357
  0.42/0.31; mean |err| ≈210 tokens/request; global Opus coefficients predict
  complete *workflow* requests at 1.008× actual.
- Persistence: `TokenTally` stores raw features `estVisible`/`estSignature`
  (k bytes) and `estLogged` (k tokens), max-merged like the other fields and
  omitted when 0. Tokens/cost are computed at read time with the current
  coefficients and output rate (same path for every range), so a refit reprices
  all days smoothly instead of ratcheting through the max-merge — like the price
  table. Applying the formula to per-day sums instead of per request changed
  results ≤0.02% on 10-16..19. Residual risk: a request quiet ≥15 min that later
  gets its final row is counted twice for that day (rare: the final row follows
  the last content block immediately).
- Output: `ModelUsage.outEst`/`costEst`, `Usage.totalCostEst`. Until
  2026-10-21 NOT included in `totalCost`, `out`, `cost`, `totalTokens`, the
  sparkline, the daily delta or the 5h/7d windows; now folded into all but the
  5h/7d windows (see the top section). UI: the user had every output-estimate mark removed
  (2026-10-19: the headline "+$X est. unlogged subagent output" line, then the
  per-model "+$x est." / "+NK est." marks); on 2026-10-21 they asked for the
  estimate to be counted in the totals instead (see the top section). The unrelated price "est."
  pill (`priceSource == "estimate"`, an unconfirmed rate) was removed in the
  same pass; `priceSource` stays in the payload.

**Real data (2026-10-19, built-in prices, local days)** — `cargo test --release
real_data_daily_corrections -- --ignored --nocapture`; matches an independent
Python oracle to the cent for 10-10..18:

| day   | exact before | + advisor | est. (requests)   | ccusage 20.0.36 |
|-------|-------------:|----------:|------------------:|----------------:|
| 10-16 |      $213.77 |    $34.12 | $22.56 (1165)     |         $247.67 |
| 10-17 |      $268.41 |    $43.85 | $10.26 (439)      |         $311.97 |
| 10-18 |      $237.28 |    $23.85 | $28.04 (1055)     |         $260.96 |
| 10-19 | (in progress) |     $3.09 | $37.44 (181, mostly Fable 5.1) | — |

10-19's advisor is three calls made after the diagnosis (12:21–12:37).

**ccusage** (`claude usage claude --since 20261016 --json`; the `~/.zshrc`
wrapper now runs `npx ccusage@latest`, 20.0.36, not a pinned 20.0.5): it already
counts advisor calls — its per-day totals exceed our old exact base by $33.90 /
$43.56 / $23.68, within $0.3 of our advisor sums, folded into
`claude-opus-5-5`, and its native binary has `UsageIterations`/`advisor_message`
parsing. It has the workflow gap (e.g. 10-17 `claude-fable-5-1` 8 output tokens
for $1.47) and no estimate. So after this change our exact total ≈ ccusage,
and the (no longer displayed) estimate is the extra.

Verified: `cargo test` 35 passed / 3 ignored (new: advisor once at own model +
fallback ignored, settled/complete/young estimate eligibility, per-family fit +
fallbacks, stored features reprice without ratchet), clippy `-D warnings`,
`cargo fmt --check`, `pnpm build`, browser preview light+dark. Scan cost: the
release 5-range smoke went 3.82s → 4.37s (+14%); one Claude pass ≈0.69s.
End-to-end through `compute_usage_with_plan` (`smoke_compute_all` prints
`totalCostEst`; then kept apart, since 2026-10-21 printed as "incl. $X est."
because it is inside the total): today +$37.43, October +$132.27, on-disk window +$187.84 —
the oracle gives $132.28 / $187.85. The All path's store read
(`tw.est_inputs()`) is covered only by unit tests: cfg(test) skips the store,
and running the app would max-merge into the real `daily_usage.json`.
Committed and pushed to main 2026-10-19 (ff12299, b7a7eda, 85abb60).

## Before committing

- Inspect the full uncommitted diff; do not include unrelated changes.
- Keep the existing bundle identifier and all-time data path so user history
  remains intact.
- This repo is public. Keep personal paths, other private projects and travel
  details out of this file, commits and code.
- Commit/push only when the user asks (last push: 2026-11-07).
