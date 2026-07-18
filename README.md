# Token Counter

_Last updated: 2026-07-17_

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
- **Auto-refresh**: file watchers on `~/.claude/projects/` and `~/.codex/sessions/` emit updates instantly when new events land; 15s polling fallback
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
- **File watcher**: `notify` + `notify-debouncer-full` (500ms debounce on `~/.claude/projects/` and `~/.codex/sessions/`)
- **Claude limits**: read-only Keychain access + `reqwest`/rustls request to Anthropic; never writes or refreshes OAuth credentials
- **Drag region**: Tauri's `data-tauri-drag-region` attribute — WKWebView doesn't honor Electron's `-webkit-app-region` CSS

## Data source quirks

The JSONL events have a few traps worth knowing if you want to build anything similar:

- **Dedup by `message.id`, not event uuid** — Claude Code writes multiple JSONL lines per API response when sessions resume/fork. UUIDs are unique per line, but the same `message.id` shows up in 2–9 places. Roughly half of all assistant events in a mature `.claude` dir are duplicates.
- **`usage.speed == "fast"` marks Claude Code's priority tier** — the raw `message.model` never contains "-fast"; ccusage synthesizes the suffix. Priority is 6× standard pricing.
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

Codex rollouts (`~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`) have their own traps:

- **`token_count` events don't name a model** — attribute each one to the most recent `turn_context.payload.model` in file order.
- **Older Codex builds emit every `token_count` twice** (identical `total`/`last` pairs back to back). A naive sum nearly doubles spend; skip consecutive duplicates, then sum `last_token_usage`.
- **Cumulative totals reset mid-file** when a process restarts and appends to the same rollout — another reason to sum per-call `last` values instead of trusting the final `total_token_usage`.
- **`input_tokens` INCLUDES `cached_input_tokens`** (OpenAI convention) — split them before pricing, or cached reads get billed at the full input rate.
- **`codex-auto-review` is an internal approval reviewer, not the PR Code Review product** — no public billable model mapping is documented, so the displayed dollar value remains an explicit legacy-rate estimate.
- `rate_limits` on each `token_count` carries the **official** 5h/weekly `used_percent` — no estimation needed, unlike the Claude side.

## Why native vibrancy, not CSS glass

Early versions used stacked `backdrop-filter` + `mix-blend-mode` per the design handoff. On macOS that had a reliable ~1s color shift when the window gained key focus (WKWebView re-samples backdrop and re-composites blend modes on key/non-key transitions). No amount of CSS tuning — opacity, `will-change`, `contain: paint`, GPU promotion, reduced filter complexity — fixed it. Moving the glass to `NSVisualEffectView` via `window-vibrancy` eliminated the flicker entirely because the glass is composited at the AppKit layer, not in WKWebView. The decoration layers (tint, highlights, sheen) still live in CSS on top.

## License

MIT

---

Built on a design handoff from Anthropic's Claude Code Token Counter prototype. The target stack (Tauri + React) and liquid-glass aesthetic follow the handoff spec; deviations (native vibrancy, pricing updates) are documented above.
