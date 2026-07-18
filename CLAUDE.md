# CLAUDE.md — Token Counter project handoff

_Last updated: 2026-07-03_

## Current state

This working tree contains an **uncommitted v0.2.0 feature** that expands the
former “Claude Code Token Counter” into **Token Counter**, covering both Claude
Code and OpenAI Codex. Do not discard or rewrite the existing changes.

Main changed files:

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

Official OpenAI prices were rechecked for GPT-5/5.1/5.2/5.4/5.5/5.6 on
2026-07-17. GPT-5.6 Sol/Terra/Luna use their distinct official rates;
unsuffixed `gpt-5.6` follows the Sol alias. GPT-5.3-Codex remains mapped to the
GPT-5.2 rate. The internal `codex-auto-review` approval reviewer keeps its
legacy-rate estimate because no public billable model map is documented.

## Before committing

- Inspect the full uncommitted diff; do not include unrelated changes.
- Keep the existing bundle identifier and all-time data path so user history
  remains intact.
- The user has not yet requested a commit or push.
