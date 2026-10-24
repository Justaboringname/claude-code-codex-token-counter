# Token Counter

_Last updated: 2026-10-20

macOS popover app that shows how many tokens **Claude Code** and **OpenAI Codex** have used, what they would cost at standard API-equivalent rates, and each provider's official 5-hour / weekly usage limits. Token and cost history comes from local JSONL; Claude's official limit row makes one read-only request to Anthropic. ChatGPT subscription usage is covered by plan limits and credits rather than billed directly as the displayed dollar estimate.

Claude models render in the warm **orange** ramp, Codex/GPT models in the cool **blue** ramp.

## Features

- **Two providers, one counter**: Claude Code + Codex side by side — per-model bars, dual sparkline (orange = Claude, blue = Codex), merged tool grid
- **Cost breakdown by range**: Session / Today / Week / Month / All-time (Session = most recently active session across either provider)
- **Per-model totals** with explicit 2026 pricing for current and historical model IDs (including distinct GPT-5.6 Sol/Terra/Luna and GPT-5.4 Pro/mini/nano tiers); cached input is priced separately from uncached input
- **Official Claude quota meters**: true 5h + weekly utilization from Anthropic's `/api/oauth/usage`, cached for 60s with last-good fallback
- **Official Codex quota meters**: 5h + weekly `used_percent` straight from the CLI's own rate-limits telemetry (not estimated)
- **Top tool calls** grid (Read, Edit, Bash, Grep… plus Codex's Shell, Apply Patch, Plan) with call counts
- **14-day sparkline** of daily spend with delta vs yesterday
- **Auto-refresh**: file watchers on `~/.claude/projects/` and `~/.codex/{sessions,archived_sessions}/` emit updates instantly when new events land; 15s polling fallback
- **Native liquid glass** via macOS `NSVisualEffectView` — no CSS backdrop-filter flicker
- **Dark mode** that syncs native window appearance (follows system or manual override)
- **Dynamic window height** via `ResizeObserver` + `setSize`
- **Draggable** from the top bar, **resizable** from any edge

## Build

```bash
pnpm install
pnpm tauri build        # .app + .dmg land in src-tauri/target/release/bundle/
```

For development with hot reload:

```bash
pnpm tauri dev
```

## Tech stack

- **Shell**: Tauri 2 (Rust + WKWebView)
- **Frontend**: React 18 + TypeScript + Vite (inline styles, no CSS framework)
- **Glass**: `window-vibrancy` crate with `NSVisualEffectMaterial::HudWindow` + `NSVisualEffectState::Active` (stays active when window loses focus)
- **File watcher**: `notify` + `notify-debouncer-full` (500ms debounce on `~/.claude/projects/` and `~/.codex/{sessions,archived_sessions}/`)
- **Claude limits**: read-only Keychain access + `reqwest`/rustls request to Anthropic; never writes or refreshes OAuth credentials
- **Drag region**: Tauri's `data-tauri-drag-region` attribute — WKWebView doesn't honor Electron's `-webkit-app-region` CSS

## Data source quirks

The JSONL events have a few traps worth knowing if you want to build anything similar:

- **Dedup by `message.id`, not event uuid** — Claude Code writes multiple JSONL lines per API response when sessions resume/fork. UUIDs are unique per line, but the same `message.id` shows up in 2–9 places. Roughly half of all assistant events in a mature `.claude` dir are duplicates.
- **`usage.speed == "fast"` marks fast mode** — the raw `message.model` never contains "-fast"; ccusage synthesizes the suffix. Fast mode is 2× standard pricing on Opus 4.8 / 5 / 5.5 (6× on the retired Opus 4.6 fast mode). `usage.service_tier == "priority"` is Priority Tier capacity, not fast mode.
- **Opus 4.6/4.7 is NOT priced like legacy Opus 4** — it's $5/$25/$6.25/$0.5 per 1M (input/output/cache-write/cache-read), not the $15/$75 Oct-2025 numbers. Cross-check with `claude usage --json` (an alias for `npx ccusage`).
- Strip trailing `-YYYYMMDD` date suffix when normalizing model IDs.
- `isSidechain: true` marks subagent calls (~12% of events in typical usage). Current code includes them in totals, matching ccusage.
- Skip `message.model == "<synthetic>"` events entirely.

Claude's official quota row is separate from token counting. It reads the
`Claude Code-credentials` Keychain item and calls Anthropic's private
`/api/oauth/usage` endpoint with the same headers used by Claude Usage Monitor.
The access token is sent only to Anthropic. Token Counter never writes the
Keychain or rotates the single-use refresh token; a 401 falls back to the last
successful snapshot until Claude Code refreshes its own login.

Codex rollouts live under `$CODEX_HOME` (default `~/.codex`) in `sessions/YYYY/MM/DD/` **and** the flat `archived_sessions/` (where the archive action moves threads), as `rollout-*.jsonl` or, once Codex has compressed a file idle for 7 days, `rollout-*.jsonl.zst`. They have their own traps:

- **Missing files, not missing prices** — reading only `sessions/**/*.jsonl` drops every archived or compressed thread. Read both directories and both forms; when `X.jsonl` and `X.jsonl.zst` coexist mid-(de)compression, read the plain one.
- **`token_usage_record` (Codex ≥ 0.153) is the per-response truth** — one line per Responses API response with usage, keyed by `response_id`, including remote compactions that no `token_count` ever reports. `token_count` is a running snapshot (`info.{total,last}_token_usage`) and is only summed for stretches without records (older rollouts): skip consecutive duplicate pairs, then sum `last_token_usage`, which survives mid-file counter resets.
- **Neither usage line names a model or tier** — take the model from the record's `turn_context` (`turn_id`) or the latest `turn_context` / `thread_settings_applied`, and the requested service tier from `thread_settings_applied.thread_settings.service_tier` (`priority` = Fast, billed 2×; GPT-6 Sol/Luna default to it). Prompts over 272K tokens bill 2× input / 1.5× output.
- **Forks copy history** — legacy forks re-write the parent's `token_count` lines (and records, under the parent's `thread_id`) into the child with fresh timestamps; paginated subagents mark an inherited prefix with `subagent_history_start_ordinal`. Count each response id / fork-copied snapshot once, at its earliest occurrence.
- **`input_tokens` INCLUDES `cached_input_tokens` and `cache_write_input_tokens`** (OpenAI convention) — split them before pricing, or cached reads get billed at the full input rate.
- **`codex-auto-review` is an internal approval reviewer, not the PR Code Review product** — no public billable model mapping is documented, so the displayed dollar value remains an explicit legacy-rate estimate.
- **`token_count.rate_limits` is not the official quota any more** — Codex keeps one "latest" slot that every quota bucket (`codex_other`, per-model limits, metadata-only updates with null windows) overwrites, and it only changes while Codex makes requests. The Codex limits row reads what Codex's own `/status` reads, `GET https://chatgpt.com/backend-api/wham/usage`, with the ChatGPT access token from `$CODEX_HOME/auth.json` (or the `Codex Auth` Keychain item). Like the Claude row it is read-only: the single-use refresh token is never touched, and an expired token shows the last snapshot as stale. If the live read fails, the row falls back to the newest main-bucket (`limit_id` `codex`) windows in the rollouts.
- Usage that never reaches a local rollout (Codex cloud tasks, ephemeral threads, memories) still counts against the plan limits but can't be priced.

## Why native vibrancy, not CSS glass

Early versions used stacked `backdrop-filter` + `mix-blend-mode` per the design handoff. On macOS that had a reliable ~1s color shift when the window gained key focus (WKWebView re-samples backdrop and re-composites blend modes on key/non-key transitions). No amount of CSS tuning — opacity, `will-change`, `contain: paint`, GPU promotion, reduced filter complexity — fixed it. Moving the glass to `NSVisualEffectView` via `window-vibrancy` eliminated the flicker entirely because the glass is composited at the AppKit layer, not in WKWebView. The decoration layers (tint, highlights, sheen) still live in CSS on top.

## License

MIT

---

Built on a design handoff from Anthropic's Claude Code Token Counter prototype. The target stack (Tauri + React) and liquid-glass aesthetic follow the handoff spec; deviations (native vibrancy, pricing updates) are documented above.
