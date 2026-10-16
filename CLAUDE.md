# CLAUDE.md — Token Counter project handoff

_Last updated: 2026-10-16_

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
  `--headless=new --screenshot` instead).

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

- The implementation is intentionally read-only: it never writes Keychain and
  never rotates the single-use refresh token. This avoids a race when the
  separate Claude Usage Monitor app is running at the same time.
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

Source: `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`.

- Attribute each `token_count` to the latest preceding
  `turn_context.payload.model`.
- Skip consecutive duplicate `(total_token_usage, last_token_usage)` pairs;
  older rollouts emit identical token events twice.
- Sum `last_token_usage`, not the final cumulative total, because totals can
  reset when a process appends to an existing rollout.
- `input_tokens` includes `cached_input_tokens`; subtract cached input before
  applying the full input price.
- Use the newest `rate_limits` snapshot for official 5h/weekly Codex quota.
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
- Remaining latent gap, not present in the data: the Codex Fast tier
  (`thread_settings_applied.service_tier`, API-equivalent 2×) is not detected
  and would be priced at Standard.
- Codex `cache_write_input_tokens` (subset of `input_tokens`) is 0 in every
  event so far and is not parsed. `codex-auto-review` keeps its legacy estimate.

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
`ModelUsage.priceSource` drives the UI's "est." pill on estimate rows.
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
  kept and the row is flagged "est.". So a new "est." pill on a model that used
  to look fine means LiteLLM and the official built-in row conflict — check the
  raw official page and update the built-in row. Price changes within 2× (e.g.
  the GPT-5.6 Sol promo ending) apply automatically; larger ones (like the
  Aug 2026 Luna $1→$0.2 cut) wait for a built-in update.
- Codex write rates stay 0 regardless of LiteLLM (no cache-write tokens).
- A rejected download (<50 usable models) backs off for 24h instead of
  re-downloading hourly; a `fetched_at` in the future counts as stale; rows
  loaded from the cache file get the same bounds check as a download
  ($0–$1,000/MTok).
- Estimate rows are never cut by the top-10 "By model" limit.
- Known gap: fetch failures only reach stderr; the UI doesn't show the table's
  age, so a long-stale table would go unnoticed (new models are still flagged).

When updating `BUILTIN_VERIFIED`/the built-in rows, verify against the raw
official pages, not LiteLLM, so the fallback stays independent.

## Before committing

- Inspect the full uncommitted diff; do not include unrelated changes.
- Keep the existing bundle identifier and all-time data path so user history
  remains intact.
- Commit/push only when the user asks (last push: 2026-07-18).
