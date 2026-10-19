# CLAUDE.md — Token Counter project handoff

_Last updated: 2026-10-19_

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

## Advisor calls + unlogged subagent output (2026-10-19, uncommitted)

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
  `BUILTIN_VERIFIED` and get a spurious price "est." pill. The advisor entries
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
- Output: `ModelUsage.outEst`/`costEst`, `Usage.totalCostEst`. NOT included in
  `totalCost`, `out`, `cost`, `totalTokens`, the sparkline, the daily delta or
  the 5h/7d windows. UI: "+$X est. unlogged subagent output" under the headline,
  "+$x est." beside a model's cost, "+NK est." after its output tokens, all
  with `OUTPUT_EST_TIP` (distinct from the price "est." pill, which means an
  unconfirmed rate). Browser preview: the sample's Opus 4 row carries an estimate.

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
and the "est." line is the extra.

Verified: `cargo test` 35 passed / 3 ignored (new: advisor once at own model +
fallback ignored, settled/complete/young estimate eligibility, per-family fit +
fallbacks, stored features reprice without ratchet), clippy `-D warnings`,
`cargo fmt --check`, `pnpm build`, browser preview light+dark. Scan cost: the
release 5-range smoke went 3.82s → 4.37s (+14%); one Claude pass ≈0.69s.
End-to-end through `compute_usage_with_plan` (`smoke_compute_all` prints
`totalCostEst`): today +$37.43, October +$132.27, on-disk window +$187.84 —
the oracle gives $132.28 / $187.85. The All path's store read
(`tw.est_inputs()`) is covered only by unit tests: cfg(test) skips the store,
and running the app would max-merge into the real `daily_usage.json`.
Not committed; app not rebuilt/installed.

## Before committing

- Inspect the full uncommitted diff; do not include unrelated changes.
- Keep the existing bundle identifier and all-time data path so user history
  remains intact.
- Commit/push only when the user asks (last push: 2026-07-18).
