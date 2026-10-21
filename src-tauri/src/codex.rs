// Codex (OpenAI) usage parsing — reads every rollout under CODEX_HOME
// (default ~/.codex): `sessions/YYYY/MM/DD/` plus the flat `archived_sessions/`
// that Codex's archive action moves threads into. Each rollout is
// `rollout-*.jsonl`, or `rollout-*.jsonl.zst` once Codex has compressed a file
// idle for 7 days (rollout/compress). Subagent, /review and guardian threads
// are rollouts of their own in the same tree.
//
// Lines are `{"timestamp", "ordinal"?, "type", "payload"}`. The rows we use:
//   * `session_meta` — the FIRST one names this file's thread (`payload.id`);
//     copied fork histories repeat their ancestors' after it.
//     `subagent_history_start_ordinal` marks an inherited prefix copied from
//     the parent (lines with a lower `ordinal` are not this thread's usage).
//   * `token_usage_record` (Codex ≥ 0.153) — one per Responses API response
//     with usage: sampling, local + remote compaction, incomplete responses.
//     `payload.usage` is that response's tokens; `response_id` keys it.
//   * `event_msg` / `token_count` — a snapshot of the session's running
//     `info.{total,last}_token_usage` and the latest `rate_limits`. Before
//     0.153 it is the only usage source; after, it misses remote compactions
//     and repeats a stale `last` after an incomplete response, so it is only
//     counted for stretches with no `token_usage_record`.
//   * `turn_context` (`payload.turn_id`, `payload.model`) and
//     `event_msg` / `thread_settings_applied` (`payload.thread_settings.model`,
//     `.service_tier`) — neither usage line names a model or tier.
//   * `response_item` with `payload.type` in {function_call, custom_tool_call,
//     local_shell_call, web_search_call} — tool calls for the tools grid.
//
// Accounting (token_count path validated on this user's 105-file July
// history — exact match with the final cumulative total on all 89 monotone
// files):
//   * skip a token_count whose (total, last) pair repeats the previous one —
//     rate-limit updates and older builds re-emit the same snapshot;
//   * otherwise sum `last_token_usage`, which also survives mid-file counter
//     resets (a process restart appending to the same rollout).
// Copies: legacy forks re-write the parent's token_count lines (and records,
// under the parent's thread_id) into the child with fresh timestamps, so a
// token_count pair or response_id seen in several files counts once, at its
// earliest occurrence.
//
// OpenAI convention: `input_tokens` INCLUDES `cached_input_tokens` and
// `cache_write_input_tokens`; `output_tokens` includes reasoning tokens.
// Input is split into uncached (`in`), cache writes (`cacheWrite`) and cached
// reads (`cacheRead`) before pricing.
//
// Billing suffixes on the model id (priced in usage::pricing_with):
// `-long` when the prompt exceeds 272K tokens (2× input, 1.5× output), then
// the requested service tier: `-fast` (priority), `-flex` or `-ultrafast`.

use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

const SESSIONS_DIR: &str = "sessions";
const ARCHIVED_DIR: &str = "archived_sessions";

/// OpenAI's long-context tier: a request whose prompt exceeds this many tokens
/// bills at 2× input (cached and cache-write too) and 1.5× output.
const LONG_CONTEXT_TOKENS: i64 = 272_000;

pub struct CodexEntry {
    pub ts: Option<DateTime<Utc>>,
    /// Model id plus billing suffixes (`-long`, `-fast`/`-flex`/`-ultrafast`).
    pub model: String,
    /// Uncached input, k-tokens (input − cached − cache writes).
    pub in_k: f64,
    /// Cache-write input, k-tokens — maps onto ModelUsage.cache_write.
    pub cw_k: f64,
    /// Cached input, k-tokens — maps onto ModelUsage.cache_read.
    pub cr_k: f64,
    /// Output (incl. reasoning), k-tokens.
    pub out_k: f64,
    pub session_id: Option<String>,
}

pub struct CodexToolEvent {
    pub ts: Option<DateTime<Utc>>,
    /// Canonical name: "shell", "apply_patch", "update_plan", "web_search", …
    pub name: String,
    pub session_id: Option<String>,
}

/// Codex's main ("codex") quota bucket as last seen in the rollouts, by window
/// length: `primary*` is the 5h window, `secondary*` the weekly one.
/// `used_percent` is only current as of `as_of`; if `resets_at` has passed,
/// the window has rolled over with no Codex activity since — the frontend
/// should render 0% rather than the stale figure. The live /wham/usage read
/// (codex_limits.rs) supersedes this whenever it succeeds.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CodexQuota {
    #[serde(rename = "asOf")]
    pub as_of: String,
    #[serde(rename = "primaryUsedPct")]
    pub primary_used_pct: f64,
    #[serde(rename = "primaryWindowMinutes")]
    pub primary_window_minutes: u64,
    #[serde(rename = "primaryResetsAt")]
    pub primary_resets_at: Option<i64>,
    #[serde(rename = "secondaryUsedPct")]
    pub secondary_used_pct: f64,
    #[serde(rename = "secondaryWindowMinutes")]
    pub secondary_window_minutes: u64,
    #[serde(rename = "secondaryResetsAt")]
    pub secondary_resets_at: Option<i64>,
    #[serde(rename = "planType")]
    pub plan_type: Option<String>,
}

#[derive(Default)]
pub struct CodexData {
    pub entries: Vec<CodexEntry>,
    pub tools: Vec<CodexToolEvent>,
    pub quota: Option<CodexQuota>,
    /// (session_id, first token ts, last token ts) of the most recent session.
    pub latest_session: Option<(String, DateTime<Utc>, DateTime<Utc>)>,
}

/// `$CODEX_HOME` when set and non-empty (as Codex resolves it), else ~/.codex.
/// A Finder-launched app only sees CODEX_HOME if it is set for launchd.
pub fn codex_home() -> Option<PathBuf> {
    match std::env::var_os("CODEX_HOME") {
        Some(v) if !v.is_empty() => Some(PathBuf::from(v)),
        _ => dirs::home_dir().map(|h| h.join(".codex")),
    }
}

/// The directories rollouts live in, for the file watcher.
pub fn rollout_roots() -> Vec<PathBuf> {
    codex_home()
        .map(|h| vec![h.join(SESSIONS_DIR), h.join(ARCHIVED_DIR)])
        .unwrap_or_default()
}

fn is_compressed(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "zst")
}

fn list_rollout_files(home: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for sub in [SESSIONS_DIR, ARCHIVED_DIR] {
        let root = home.join(sub);
        if !root.is_dir() {
            continue;
        }
        let base = glob::Pattern::escape(&root.to_string_lossy());
        // The literal `rollout-` prefix and `.jsonl[.zst]` suffix skip Codex's
        // temp files (rollout-compress-*.tmp, .rollout-materialize-*, …).
        for pattern in ["rollout-*.jsonl", "rollout-*.jsonl.zst"] {
            if let Ok(paths) = glob::glob(&format!("{base}/**/{pattern}")) {
                out.extend(paths.filter_map(Result::ok));
            }
        }
    }
    // While Codex compresses or decompresses a rollout, X.jsonl and
    // X.jsonl.zst briefly coexist; like Codex, read the plain one.
    let plain: HashSet<PathBuf> = out.iter().filter(|p| !is_compressed(p)).cloned().collect();
    out.retain(|p| !is_compressed(p) || !plain.contains(&p.with_extension("")));
    out.sort();
    out
}

fn open_rollout(path: &Path) -> io::Result<Box<dyn BufRead>> {
    let file = File::open(path)?;
    if is_compressed(path) {
        Ok(Box::new(BufReader::new(zstd::stream::read::Decoder::new(
            file,
        )?)))
    } else {
        Ok(Box::new(BufReader::new(file)))
    }
}

fn parse_ts(v: &Value) -> Option<DateTime<Utc>> {
    v.get("timestamp")
        .and_then(|t| t.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
}

fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(|x| x.as_str())
}

// Collapse Codex's several shell-tool spellings (they changed across CLI
// versions: shell → shell_command → exec_command) into one grid tile.
fn canonical_tool(name: &str) -> String {
    match name {
        "shell" | "shell_command" | "exec_command" | "local_shell" | "run" => "shell".into(),
        other => other.to_string(),
    }
}

/// One TokenUsage object (all six counters, so equal values mean an identical
/// snapshot).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
struct Tokens {
    input: i64,
    cached: i64,
    cache_write: i64,
    output: i64,
    reasoning: i64,
    total: i64,
}

impl Tokens {
    fn of(v: &Value) -> Self {
        let n = |k: &str| v.get(k).and_then(|x| x.as_i64()).unwrap_or(0);
        Tokens {
            input: n("input_tokens"),
            cached: n("cached_input_tokens"),
            cache_write: n("cache_write_input_tokens"),
            output: n("output_tokens"),
            reasoning: n("reasoning_output_tokens"),
            total: n("total_tokens"),
        }
    }

    fn has_usage(&self) -> bool {
        self.input > 0 || self.output > 0
    }

    /// `self − earlier`, or None if any counter went backwards.
    fn minus(&self, earlier: &Tokens) -> Option<Tokens> {
        let d = Tokens {
            input: self.input - earlier.input,
            cached: self.cached - earlier.cached,
            cache_write: self.cache_write - earlier.cache_write,
            output: self.output - earlier.output,
            reasoning: self.reasoning - earlier.reasoning,
            total: self.total - earlier.total,
        };
        [
            d.input,
            d.cached,
            d.cache_write,
            d.output,
            d.reasoning,
            d.total,
        ]
        .iter()
        .all(|x| *x >= 0)
        .then_some(d)
    }
}

/// A token_count's (total_token_usage, last_token_usage).
type Pair = (Tokens, Tokens);

/// The service tier a thread requested (thread_settings_applied).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tier {
    Standard,
    Fast,
    Flex,
    Ultrafast,
}

impl Tier {
    fn of(s: Option<&str>) -> Self {
        match s {
            Some("priority") | Some("fast") => Tier::Fast,
            Some("flex") => Tier::Flex,
            Some("ultrafast") => Tier::Ultrafast,
            _ => Tier::Standard,
        }
    }

    fn suffix(self) -> &'static str {
        match self {
            Tier::Standard => "",
            Tier::Fast => "-fast",
            Tier::Flex => "-flex",
            Tier::Ultrafast => "-ultrafast",
        }
    }
}

fn billing_id(model: &str, u: &Tokens, tier: Tier) -> String {
    let long = if u.input > LONG_CONTEXT_TOKENS {
        "-long"
    } else {
        ""
    };
    format!("{model}{long}{}", tier.suffix())
}

fn make_entry(
    ts: Option<DateTime<Utc>>,
    model: String,
    u: &Tokens,
    session_id: &Option<String>,
) -> Option<CodexEntry> {
    if !u.has_usage() {
        return None;
    }
    let input = u.input.max(0);
    let cached = u.cached.clamp(0, input);
    let cache_write = u.cache_write.clamp(0, input - cached);
    Some(CodexEntry {
        ts,
        model,
        in_k: (input - cached - cache_write) as f64 / 1000.0,
        cw_k: cache_write as f64 / 1000.0,
        cr_k: cached as f64 / 1000.0,
        out_k: u.output.max(0) as f64 / 1000.0,
        session_id: session_id.clone(),
    })
}

/// Where a line sits for "earliest occurrence wins": (timestamp ms, line).
/// The file index is added when files are compared.
type LineOrder = (i64, usize);

fn order_of(ts: Option<DateTime<Utc>>, line: usize) -> LineOrder {
    (ts.map_or(i64::MAX, |t| t.timestamp_millis()), line)
}

/// A usage line that counts unless an earlier copy exists in another file.
struct Candidate<K> {
    key: K,
    order: LineOrder,
    entry: CodexEntry,
}

/// One window of the main quota bucket as seen on a token_count line.
#[derive(Debug, Clone)]
struct WindowObs {
    ts: DateTime<Utc>,
    order: LineOrder,
    used_pct: f64,
    window_minutes: u64,
    resets_at: Option<i64>,
}

#[derive(Debug, Clone, Default)]
struct QuotaObs {
    five_hour: Option<WindowObs>,
    weekly: Option<WindowObs>,
    plan_type: Option<(LineOrder, String)>,
}

/// Everything one rollout contributes, before cross-file dedup. Cached per
/// file and reused while the file's size and mtime are unchanged.
#[derive(Default)]
struct FileScan {
    thread_id: Option<String>,
    /// A fork or subagent thread: the only files that hold copied lines, so
    /// the only ones whose token_count usage can lose to an earlier copy.
    is_fork: bool,
    records: Vec<Candidate<String>>,
    fallback: Vec<Candidate<Pair>>,
    /// Earliest line of every usage-bearing token_count pair, counted or not:
    /// a copy elsewhere loses to it.
    pairs: HashMap<Pair, LineOrder>,
    tools: Vec<CodexToolEvent>,
    quota: QuotaObs,
    span: Option<(DateTime<Utc>, DateTime<Utc>)>,
}

impl FileScan {
    fn extend_span(&mut self, ts: Option<DateTime<Utc>>) {
        if let Some(t) = ts {
            self.span = Some(match self.span {
                Some((s, e)) => (s.min(t), e.max(t)),
                None => (t, t),
            });
        }
    }
}

// A response_item line that is not a tool call can be skipped unparsed; most
// of a rollout's bytes are messages, reasoning and tool output. Codex writes
// compact JSON, and these exact byte sequences can only be structural (quotes
// inside strings are escaped), so a miss just means a full parse.
fn skippable(line: &str) -> bool {
    line.contains(r#""type":"response_item""#)
        && ![
            r#""type":"function_call""#,
            r#""type":"custom_tool_call""#,
            r#""type":"local_shell_call""#,
            r#""type":"web_search_call""#,
        ]
        .iter()
        .any(|t| line.contains(t))
}

fn scan_file(path: &Path) -> io::Result<FileScan> {
    let mut s = FileScan::default();
    let mut meta_seen = false;
    // Lines with a lower ordinal were copied from the parent thread.
    let mut prefix_end: Option<u64> = None;
    // "codex" = never saw a model (1 legacy file).
    let mut cur_model = String::from("codex");
    let mut turn_models: HashMap<String, String> = HashMap::new();
    let mut tier = Tier::Standard;
    let mut prev_pair: Option<Pair> = None;
    // A token_usage_record since the previous token_count: that snapshot's
    // usage is already counted.
    let mut recorded = false;

    for (line_no, bytes) in open_rollout(path)?.split(b'\n').enumerate() {
        // A read/decode error ends the file; a bad line is skipped.
        let Ok(bytes) = bytes else { break };
        let Ok(line) = std::str::from_utf8(&bytes) else {
            continue;
        };
        if line.trim().is_empty() || skippable(line) {
            continue;
        }
        let Ok(row) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(payload) = row.get("payload") else {
            continue;
        };
        let ts = parse_ts(&row);
        let order = order_of(ts, line_no);
        let inherited = match (prefix_end, row.get("ordinal").and_then(|o| o.as_u64())) {
            (Some(end), Some(ord)) => ord < end,
            _ => false,
        };

        match row.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "session_meta" if !meta_seen => {
                meta_seen = true;
                s.thread_id = str_of(payload, "id")
                    .or_else(|| str_of(payload, "session_id"))
                    .map(String::from);
                s.is_fork = str_of(payload, "forked_from_id").is_some()
                    || str_of(payload, "parent_thread_id").is_some()
                    || payload
                        .get("source")
                        .and_then(|v| v.get("subagent"))
                        .is_some();
                prefix_end = payload
                    .get("subagent_history_start_ordinal")
                    .and_then(|o| o.as_u64());
            }
            "turn_context" => {
                if let Some(m) = str_of(payload, "model") {
                    cur_model = m.to_string();
                    if let Some(turn) = str_of(payload, "turn_id") {
                        turn_models.insert(turn.to_string(), cur_model.clone());
                    }
                }
            }
            "token_usage_record" => {
                recorded = true;
                // Records copied from another thread carry its thread_id.
                let foreign = matches!(
                    (str_of(payload, "thread_id"), s.thread_id.as_deref()),
                    (Some(t), Some(own)) if t != own
                );
                if inherited || foreign {
                    continue;
                }
                let Some(usage) = payload.get("usage") else {
                    continue;
                };
                let u = Tokens::of(usage);
                let model = str_of(payload, "turn_id")
                    .and_then(|t| turn_models.get(t))
                    .unwrap_or(&cur_model);
                if let Some(entry) = make_entry(ts, billing_id(model, &u, tier), &u, &s.thread_id) {
                    s.extend_span(ts);
                    s.records.push(Candidate {
                        key: str_of(payload, "response_id").unwrap_or("").to_string(),
                        order,
                        entry,
                    });
                }
            }
            "response_item" if !inherited => {
                let name = match str_of(payload, "type").unwrap_or("") {
                    "function_call" | "custom_tool_call" => {
                        str_of(payload, "name").unwrap_or("shell").to_string()
                    }
                    "local_shell_call" => "shell".to_string(),
                    "web_search_call" => "web_search".to_string(),
                    _ => continue,
                };
                s.tools.push(CodexToolEvent {
                    ts,
                    name: canonical_tool(&name),
                    session_id: s.thread_id.clone(),
                });
            }
            "event_msg" => match str_of(payload, "type") {
                Some("thread_settings_applied") => {
                    // Copied snapshots keep their original owner's thread_id.
                    let owner = str_of(payload, "thread_id");
                    if owner.is_none() || owner == s.thread_id.as_deref() {
                        if let Some(settings) = payload.get("thread_settings") {
                            if let Some(m) = str_of(settings, "model") {
                                cur_model = m.to_string();
                            }
                            tier = Tier::of(str_of(settings, "service_tier"));
                        }
                    }
                }
                Some("token_count") => {
                    let covered = std::mem::take(&mut recorded);
                    if !inherited {
                        if let (Some(rl), Some(t)) = (payload.get("rate_limits"), ts) {
                            observe_quota(&mut s.quota, rl, t, order);
                        }
                    }
                    // Old builds emit token_count with info=null.
                    let Some(info) = payload.get("info").filter(|i| !i.is_null()) else {
                        continue;
                    };
                    let (Some(total), Some(last)) =
                        (info.get("total_token_usage"), info.get("last_token_usage"))
                    else {
                        continue;
                    };
                    let pair = (Tokens::of(total), Tokens::of(last));
                    let prev = prev_pair.replace(pair);
                    if prev == Some(pair) || !pair.1.has_usage() || inherited {
                        continue;
                    }
                    s.pairs.entry(pair).or_insert(order);
                    if covered {
                        continue;
                    }
                    // An incomplete response (Codex ≥ 0.163) adds to the total
                    // but leaves `last` at the previous response's usage; the
                    // total's growth is then the real figure. For a normal
                    // repeat of identical usage the two agree.
                    let u = match prev {
                        Some((pt, pl)) if pl == pair.1 => pair.0.minus(&pt).unwrap_or(pair.1),
                        _ => pair.1,
                    };
                    if let Some(entry) =
                        make_entry(ts, billing_id(&cur_model, &u, tier), &u, &s.thread_id)
                    {
                        s.extend_span(ts);
                        s.fallback.push(Candidate {
                            key: pair,
                            order,
                            entry,
                        });
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
    Ok(s)
}

/// Fold one token_count's `rate_limits` into the file's quota view. Codex
/// keeps a single "latest" snapshot that every bucket overwrites, so most
/// lines can hold another bucket (`codex_other`, per-model limits, …) or a
/// metadata-only update with null windows. Only the main bucket counts, each
/// window on its own: its newest non-null value.
fn observe_quota(q: &mut QuotaObs, rl: &Value, ts: DateTime<Utc>, order: LineOrder) {
    if !rl.is_object() {
        return;
    }
    // Rollouts from before buckets existed have no limit_id.
    if str_of(rl, "limit_id").is_some_and(|id| id != "codex") {
        return;
    }
    if let Some(plan) = str_of(rl, "plan_type") {
        q.plan_type = Some((order, plan.to_string()));
    }
    for (key, is_primary) in [("primary", true), ("secondary", false)] {
        let Some(obs) = rl.get(key).and_then(|w| window_obs(w, ts, order)) else {
            continue;
        };
        let slot = match window_kind(obs.window_minutes) {
            Some(true) => &mut q.five_hour,
            Some(false) => &mut q.weekly,
            None if is_primary => &mut q.five_hour,
            None => &mut q.weekly,
        };
        if slot.as_ref().is_none_or(|old| obs.order >= old.order) {
            *slot = Some(obs);
        }
    }
}

/// Some(true) for a ~5h window, Some(false) for a ~weekly one (±5%, as
/// Codex's /status labels them), None otherwise.
pub(crate) fn window_kind(minutes: u64) -> Option<bool> {
    let near = |target: f64| (minutes as f64 - target).abs() <= target * 0.05;
    if near(300.0) {
        Some(true)
    } else if near(10080.0) {
        Some(false)
    } else {
        None
    }
}

fn window_obs(w: &Value, ts: DateTime<Utc>, order: LineOrder) -> Option<WindowObs> {
    if !w.is_object() {
        return None;
    }
    let used_pct = w.get("used_percent").and_then(|x| x.as_f64())?;
    let resets_at = match w.get("resets_at") {
        Some(Value::Number(n)) => n.as_i64(),
        // Some legacy rollouts wrote an RFC3339 string.
        Some(Value::String(s)) => DateTime::parse_from_rfc3339(s).ok().map(|d| d.timestamp()),
        _ => None,
    }
    .or_else(|| {
        // Older Codex rollouts stored a duration relative to the
        // token_count timestamp instead of an absolute epoch.
        w.get("resets_in_seconds")
            .and_then(|x| x.as_i64())
            .map(|seconds| ts.timestamp().saturating_add(seconds.max(0)))
    });
    Some(WindowObs {
        ts,
        order,
        used_pct,
        window_minutes: w
            .get("window_minutes")
            .and_then(|x| x.as_u64())
            .unwrap_or(0),
        resets_at,
    })
}

struct CachedScan {
    len: u64,
    modified: Option<SystemTime>,
    scan: Arc<FileScan>,
}

// Rollouts only grow (or turn into a .zst sibling, a new path), so a file
// whose size and mtime are unchanged needn't be re-read or re-decompressed.
static SCAN_CACHE: Mutex<BTreeMap<PathBuf, CachedScan>> = Mutex::new(BTreeMap::new());

fn scan_cached(path: &Path, cache: &mut BTreeMap<PathBuf, CachedScan>) -> Option<Arc<FileScan>> {
    // Metadata before content: an append during the read changes it, so the
    // next scan re-reads the file.
    let meta = std::fs::metadata(path).ok()?;
    let (len, modified) = (meta.len(), meta.modified().ok());
    if let Some(c) = cache.get(path) {
        if c.len == len && c.modified == modified {
            return Some(Arc::clone(&c.scan));
        }
    }
    let scan = Arc::new(scan_file(path).ok()?);
    cache.insert(
        path.to_path_buf(),
        CachedScan {
            len,
            modified,
            scan: Arc::clone(&scan),
        },
    );
    Some(scan)
}

pub fn collect() -> CodexData {
    let Some(home) = codex_home() else {
        return CodexData::default();
    };
    let files = list_rollout_files(&home);
    let mut cache = SCAN_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let scans: Vec<Arc<FileScan>> = files
        .iter()
        .filter_map(|p| scan_cached(p, &mut cache))
        .collect();
    let live: HashSet<&PathBuf> = files.iter().collect();
    cache.retain(|p, _| live.contains(p));
    drop(cache);
    merge(&scans, Utc::now().timestamp())
}

/// Uncached scan of one CODEX_HOME (tests).
#[cfg(test)]
pub fn collect_from(home: &Path) -> CodexData {
    let scans: Vec<Arc<FileScan>> = list_rollout_files(home)
        .iter()
        .filter_map(|p| scan_file(p).ok().map(Arc::new))
        .collect();
    merge(&scans, Utc::now().timestamp())
}

/// Cross-file pass: keep each response_id and token_count pair at its
/// earliest occurrence, then gather tools, quota and the latest session.
fn merge(scans: &[Arc<FileScan>], now_epoch: i64) -> CodexData {
    type Owner = (i64, usize, usize);
    let owner = |o: LineOrder, file: usize| -> Owner { (o.0, file, o.1) };
    let mut pair_owner: HashMap<&Pair, Owner> = HashMap::new();
    let mut resp_owner: HashMap<&str, Owner> = HashMap::new();
    for (fi, s) in scans.iter().enumerate() {
        for (pair, o) in &s.pairs {
            let o = owner(*o, fi);
            pair_owner
                .entry(pair)
                .and_modify(|x| *x = (*x).min(o))
                .or_insert(o);
        }
        for c in s.records.iter().filter(|c| !c.key.is_empty()) {
            let o = owner(c.order, fi);
            resp_owner
                .entry(c.key.as_str())
                .and_modify(|x| *x = (*x).min(o))
                .or_insert(o);
        }
    }

    let mut data = CodexData::default();
    // Newest observation per window across files (ties go to the later file).
    let mut five_hour: Option<(Owner, &WindowObs)> = None;
    let mut weekly: Option<(Owner, &WindowObs)> = None;
    let mut plan_type: Option<(Owner, &str)> = None;
    let mut latest: Option<(DateTime<Utc>, String, DateTime<Utc>)> = None;
    let copy = |e: &CodexEntry| CodexEntry {
        ts: e.ts,
        model: e.model.clone(),
        in_k: e.in_k,
        cw_k: e.cw_k,
        cr_k: e.cr_k,
        out_k: e.out_k,
        session_id: e.session_id.clone(),
    };
    for (fi, s) in scans.iter().enumerate() {
        for c in &s.records {
            if c.key.is_empty() || resp_owner.get(c.key.as_str()) == Some(&owner(c.order, fi)) {
                data.entries.push(copy(&c.entry));
            }
        }
        for c in &s.fallback {
            // Two unrelated threads can log an identical snapshot (e.g.
            // repeated `codex exec` runs of one prompt); only fork files hold
            // copies.
            if !s.is_fork || pair_owner.get(&c.key) == Some(&owner(c.order, fi)) {
                data.entries.push(copy(&c.entry));
            }
        }
        data.tools.extend(s.tools.iter().map(|t| CodexToolEvent {
            ts: t.ts,
            name: t.name.clone(),
            session_id: t.session_id.clone(),
        }));

        for (mine, best) in [
            (&s.quota.five_hour, &mut five_hour),
            (&s.quota.weekly, &mut weekly),
        ] {
            if let Some(w) = mine {
                let o = owner(w.order, fi);
                if best.as_ref().is_none_or(|(b, _)| o >= *b) {
                    *best = Some((o, w));
                }
            }
        }
        if let Some((order, plan)) = &s.quota.plan_type {
            let o = owner(*order, fi);
            if plan_type.as_ref().is_none_or(|(b, _)| o >= *b) {
                plan_type = Some((o, plan.as_str()));
            }
        }

        if let (Some(sid), Some((start, end))) = (&s.thread_id, s.span) {
            match &latest {
                Some((e, _, _)) if *e >= end => {}
                _ => latest = Some((end, sid.clone(), start)),
            }
        }
    }

    data.quota = quota_of(
        five_hour.map(|(_, w)| w),
        weekly.map(|(_, w)| w),
        plan_type.map(|(_, p)| p.to_string()),
        now_epoch,
    );
    data.latest_session = latest.map(|(e, sid, s)| (sid, s, e));
    data
}

fn quota_of(
    five_hour: Option<&WindowObs>,
    weekly: Option<&WindowObs>,
    plan_type: Option<String>,
    now_epoch: i64,
) -> Option<CodexQuota> {
    let as_of = five_hour.into_iter().chain(weekly).map(|w| w.ts).max()?;
    // A missing window reports 0 minutes (the UI hides it); one without a
    // stated length gets its slot's nominal one.
    let pick = |w: Option<&WindowObs>, nominal: u64| match w {
        Some(w) if w.window_minutes == 0 => (w.used_pct, nominal, w.resets_at),
        Some(w) => (w.used_pct, w.window_minutes, w.resets_at),
        None => (0.0, 0, None),
    };
    let (p_pct, p_min, p_reset) = pick(five_hour, 300);
    let (s_pct, s_min, s_reset) = pick(weekly, 10080);
    let mut quota = CodexQuota {
        as_of: as_of.to_rfc3339(),
        primary_used_pct: p_pct,
        primary_window_minutes: p_min,
        primary_resets_at: p_reset,
        secondary_used_pct: s_pct,
        secondary_window_minutes: s_min,
        secondary_resets_at: s_reset,
        plan_type,
    };
    expire_quota(&mut quota, now_epoch);
    Some(quota)
}

fn expire_quota(quota: &mut CodexQuota, now_epoch: i64) {
    if quota
        .primary_resets_at
        .is_some_and(|reset| reset <= now_epoch)
    {
        quota.primary_used_pct = 0.0;
    }
    if quota
        .secondary_resets_at
        .is_some_and(|reset| reset <= now_epoch)
    {
        quota.secondary_used_pct = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write as _;

    // Print-style smoke over the real CODEX_HOME (like usage's
    // smoke_compute_all) — run with --nocapture and diff against an
    // independent oracle when touching the accounting rules.
    #[test]
    fn smoke_collect_real_dir() {
        if let Some(home) = codex_home() {
            let files = list_rollout_files(&home);
            let zst = files.iter().filter(|p| is_compressed(p)).count();
            let archived = files
                .iter()
                .filter(|p| p.starts_with(home.join(ARCHIVED_DIR)))
                .count();
            eprintln!(
                "rollouts={} compressed={zst} archived={archived}",
                files.len()
            );
        }
        let data = collect();
        let mut per_model: BTreeMap<String, (f64, f64, f64, f64)> = BTreeMap::new();
        for e in &data.entries {
            let m = per_model.entry(e.model.clone()).or_default();
            m.0 += e.in_k;
            m.1 += e.cr_k;
            m.2 += e.cw_k;
            m.3 += e.out_k;
        }
        for (model, (i, c, w, o)) in &per_model {
            eprintln!("{model:26} in={i:9.2}K cached={c:9.2}K write={w:7.2}K out={o:8.2}K");
        }
        eprintln!(
            "entries={} tools={} quota={:?}",
            data.entries.len(),
            data.tools.len(),
            data.quota.as_ref().map(|q| (
                q.primary_used_pct,
                q.secondary_used_pct,
                q.as_of.clone()
            ))
        );
    }

    fn temp_home(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cctc_codex_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn day_dir(home: &Path) -> PathBuf {
        home.join(SESSIONS_DIR).join("2026/10/01")
    }

    fn write_rollout(dir: &Path, name: &str, lines: &[String]) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        let mut f = File::create(&path).unwrap();
        for l in lines {
            writeln!(f, "{l}").unwrap();
        }
        path
    }

    fn write_zst(dir: &Path, name: &str, lines: &[String]) {
        std::fs::create_dir_all(dir).unwrap();
        let text = lines.join("\n") + "\n";
        let packed = zstd::stream::encode_all(text.as_bytes(), 3).unwrap();
        std::fs::write(dir.join(name), packed).unwrap();
    }

    fn line(ts: &str, ty: &str, payload: Value) -> String {
        json!({ "timestamp": ts, "type": ty, "payload": payload }).to_string()
    }

    fn meta(ts: &str, id: &str) -> String {
        line(ts, "session_meta", json!({ "id": id, "session_id": id }))
    }

    fn turn(ts: &str, turn_id: &str, model: &str) -> String {
        line(
            ts,
            "turn_context",
            json!({ "turn_id": turn_id, "model": model }),
        )
    }

    fn usage(input: i64, cached: i64, output: i64) -> Value {
        json!({
            "input_tokens": input,
            "cached_input_tokens": cached,
            "cache_write_input_tokens": 0,
            "output_tokens": output,
            "reasoning_output_tokens": 0,
            "total_tokens": input + output,
        })
    }

    fn record(ts: &str, thread: &str, turn_id: &str, resp: &str, u: Value) -> String {
        line(
            ts,
            "token_usage_record",
            json!({
                "thread_id": thread,
                "turn_id": turn_id,
                "session_id": thread,
                "root_turn_id": turn_id,
                "response_id": resp,
                "usage": u,
                "turn_token_usage": {},
                "thread_token_usage": {},
            }),
        )
    }

    fn token_count(ts: &str, total: Value, last: Value, rate_limits: Value) -> String {
        line(
            ts,
            "event_msg",
            json!({
                "type": "token_count",
                "info": { "total_token_usage": total, "last_token_usage": last },
                "rate_limits": rate_limits,
            }),
        )
    }

    fn rate_limits(five: f64, weekly: f64) -> Value {
        json!({
            "primary": { "used_percent": five, "window_minutes": 300, "resets_at": 4102444800i64 },
            "secondary": { "used_percent": weekly, "window_minutes": 10080, "resets_at": 4103049600i64 },
            "plan_type": "plus",
        })
    }

    fn settings(ts: &str, thread: &str, model: &str, tier: Option<&str>) -> String {
        let mut s = json!({ "model": model });
        if let Some(t) = tier {
            s["service_tier"] = json!(t);
        }
        line(
            ts,
            "event_msg",
            json!({ "type": "thread_settings_applied", "thread_id": thread, "thread_settings": s }),
        )
    }

    fn sum(data: &CodexData, model: &str) -> (f64, f64, f64) {
        data.entries
            .iter()
            .filter(|e| e.model == model)
            .fold((0.0, 0.0, 0.0), |(i, c, o), e| {
                (i + e.in_k + e.cw_k, c + e.cr_k, o + e.out_k)
            })
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn dedups_double_emitted_events_and_survives_resets() {
        let home = temp_home("dedup");
        let rl = rate_limits(65.0, 19.0);
        let e1 = token_count(
            "2026-07-01T00:00:02Z",
            usage(900, 400, 100),
            usage(900, 400, 100),
            rl.clone(),
        );
        // Re-emitted snapshot (rate-limit update / old double emission).
        let e1_dup = token_count(
            "2026-07-01T00:00:03Z",
            usage(900, 400, 100),
            usage(900, 400, 100),
            rl.clone(),
        );
        let e2 = token_count(
            "2026-07-01T00:00:04Z",
            usage(1300, 400, 200),
            usage(400, 0, 100),
            rl.clone(),
        );
        // Process restart: the cumulative total resets, last is a real call.
        let e3 = token_count(
            "2026-07-01T00:00:05Z",
            usage(200, 0, 100),
            usage(200, 0, 100),
            rl,
        );
        write_rollout(
            &day_dir(&home),
            "rollout-a.jsonl",
            &[
                meta("2026-07-01T00:00:00Z", "sess-1"),
                turn("2026-07-01T00:00:01Z", "t1", "gpt-5.5"),
                e1,
                e1_dup,
                e2,
                e3,
            ],
        );

        let data = collect_from(&home);
        assert_eq!(data.entries.len(), 3, "dup emission must be skipped");
        let (input, cached, out) = sum(&data, "gpt-5.5");
        assert!(near(input + cached, (900.0 + 400.0 + 200.0) / 1000.0));
        assert!(near(out, 0.3));
        // cached is split out of input, not added on top
        let e = &data.entries[0];
        assert!(near(e.in_k, 0.5), "in = input - cached");
        assert!(near(e.cr_k, 0.4));
        assert_eq!(e.session_id.as_deref(), Some("sess-1"));

        let q = data.quota.expect("quota");
        assert_eq!(q.primary_used_pct, 65.0);
        assert_eq!(q.primary_window_minutes, 300);
        assert_eq!(q.secondary_used_pct, 19.0);
        assert_eq!(q.plan_type.as_deref(), Some("plus"));

        let (sid, s, e) = data.latest_session.expect("latest session");
        assert_eq!(sid, "sess-1");
        assert!(s < e);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn tool_calls_are_canonicalized() {
        let home = temp_home("tools");
        let item = |ts: &str, p: Value| line(ts, "response_item", p);
        write_rollout(
            &day_dir(&home),
            "rollout-b.jsonl",
            &[
                meta("2026-07-01T00:00:00Z", "sess-2"),
                item(
                    "2026-07-01T00:00:01Z",
                    json!({"type":"function_call","name":"exec_command"}),
                ),
                item(
                    "2026-07-01T00:00:02Z",
                    json!({"type":"custom_tool_call","name":"apply_patch"}),
                ),
                item("2026-07-01T00:00:03Z", json!({"type":"web_search_call"})),
                item(
                    "2026-07-01T00:00:04Z",
                    json!({"type":"function_call_output","name":"exec_command"}),
                ),
                item(
                    "2026-07-01T00:00:05Z",
                    json!({"type":"message","role":"assistant","content":[]}),
                ),
            ],
        );
        let data = collect_from(&home);
        let names: Vec<&str> = data.tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["shell", "apply_patch", "web_search"],
            "outputs must not count as calls"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    // A one-call rollout: session_meta, turn_context, one token_count.
    fn one_call(id: &str, ts: &str, model: &str, input: i64) -> Vec<String> {
        vec![
            meta(ts, id),
            turn(ts, "t1", model),
            token_count(ts, usage(input, 0, 10), usage(input, 0, 10), Value::Null),
        ]
    }

    #[test]
    fn compressed_and_archived_rollouts_are_read_once() {
        let home = temp_home("zst");
        let day = day_dir(&home);
        write_rollout(
            &day,
            "rollout-plain.jsonl",
            &one_call("a", "2026-10-01T01:00:00Z", "gpt-6-sol", 1000),
        );
        write_zst(
            &day,
            "rollout-cold.jsonl.zst",
            &one_call("b", "2026-10-01T02:00:00Z", "gpt-6-sol", 2000),
        );
        // Mid-(de)compression: both forms exist; only the plain one is read.
        let both = one_call("c", "2026-10-01T03:00:00Z", "gpt-6-sol", 4000);
        write_rollout(&day, "rollout-both.jsonl", &both);
        write_zst(&day, "rollout-both.jsonl.zst", &both);
        // Archived threads sit flat in archived_sessions/, compressed or not.
        let archived = home.join(ARCHIVED_DIR);
        write_rollout(
            &archived,
            "rollout-old.jsonl",
            &one_call("d", "2026-10-01T04:00:00Z", "gpt-6-sol", 8000),
        );
        write_zst(
            &archived,
            "rollout-older.jsonl.zst",
            &one_call("e", "2026-10-01T05:00:00Z", "gpt-6-sol", 16000),
        );
        // Codex temp files are never rollouts.
        write_rollout(
            &day,
            "rollout-compress-x1y2.tmp",
            &one_call("f", "2026-10-01T06:00:00Z", "gpt-6-sol", 32000),
        );
        write_rollout(
            &day,
            ".rollout-materialize-x1y2",
            &one_call("g", "2026-10-01T06:00:00Z", "gpt-6-sol", 64000),
        );

        let data = collect_from(&home);
        let (input, _, _) = sum(&data, "gpt-6-sol");
        assert!(near(input, 31.0), "1+2+4+8+16 K, got {input}");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn usage_records_are_exact_and_token_count_fills_gaps() {
        let home = temp_home("records");
        let lines = vec![
            meta("2026-10-01T00:00:00Z", "thr"),
            turn("2026-10-01T00:00:01Z", "t1", "gpt-6-sol"),
            settings("2026-10-01T00:00:01Z", "thr", "gpt-6-sol", Some("priority")),
            // Response 1: its record, then the token_count snapshot (not again).
            record(
                "2026-10-01T00:00:02Z",
                "thr",
                "t1",
                "resp_1",
                usage(1000, 600, 50),
            ),
            token_count(
                "2026-10-01T00:00:03Z",
                usage(1000, 600, 50),
                usage(1000, 600, 50),
                Value::Null,
            ),
            // Remote compaction: a record and only a non-usage recompute snapshot.
            record(
                "2026-10-01T00:00:04Z",
                "thr",
                "t1",
                "resp_2",
                usage(5000, 0, 300),
            ),
            token_count(
                "2026-10-01T00:00:05Z",
                usage(1000, 600, 50),
                json!({"input_tokens":0,"cached_input_tokens":0,"output_tokens":0,"total_tokens":4800}),
                Value::Null,
            ),
            // A prompt over 272K, still Fast.
            record(
                "2026-10-01T00:00:06Z",
                "thr",
                "t1",
                "resp_3",
                usage(300_000, 250_000, 70),
            ),
            token_count(
                "2026-10-01T00:00:07Z",
                usage(301_000, 250_600, 120),
                usage(300_000, 250_000, 70),
                Value::Null,
            ),
            // Fast turned off; an older build resumes without records.
            settings("2026-10-01T01:00:00Z", "thr", "gpt-6-sol", Some("default")),
            token_count(
                "2026-10-01T01:00:01Z",
                usage(302_000, 250_600, 220),
                usage(1000, 0, 100),
                Value::Null,
            ),
            // Incomplete response: total grows, `last` repeats the previous one.
            token_count(
                "2026-10-01T01:00:02Z",
                usage(302_500, 250_600, 230),
                usage(1000, 0, 100),
                Value::Null,
            ),
        ];
        write_rollout(&day_dir(&home), "rollout-r.jsonl", &lines);
        let data = collect_from(&home);

        let models: Vec<&str> = data.entries.iter().map(|e| e.model.as_str()).collect();
        assert_eq!(
            models,
            vec![
                "gpt-6-sol-fast",
                "gpt-6-sol-fast",
                "gpt-6-sol-long-fast",
                "gpt-6-sol",
                "gpt-6-sol"
            ]
        );
        let (i, c, o) = sum(&data, "gpt-6-sol-fast");
        assert!(near(i, 0.4 + 5.0) && near(c, 0.6) && near(o, 0.35));
        let (i, c, o) = sum(&data, "gpt-6-sol-long-fast");
        assert!(near(i, 50.0) && near(c, 250.0) && near(o, 0.07));
        let (i, _, o) = sum(&data, "gpt-6-sol");
        assert!(near(i, 1.0 + 0.5), "incomplete counts its own 500, got {i}");
        assert!(near(o, 0.1 + 0.01));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn records_take_their_turns_model_and_split_cache_writes() {
        let home = temp_home("turns");
        let mut u = usage(1000, 200, 10);
        u["cache_write_input_tokens"] = json!(300);
        write_rollout(
            &day_dir(&home),
            "rollout-t.jsonl",
            &[
                meta("2026-10-01T00:00:00Z", "thr"),
                turn("2026-10-01T00:00:01Z", "t1", "gpt-6-astra"),
                turn("2026-10-01T00:00:02Z", "t2", "gpt-6-luna"),
                // A late record of turn t1 still bills t1's model.
                record("2026-10-01T00:00:03Z", "thr", "t1", "resp_a", u),
            ],
        );
        let data = collect_from(&home);
        let e = &data.entries[0];
        assert_eq!(e.model, "gpt-6-astra");
        assert!(near(e.in_k, 0.5) && near(e.cw_k, 0.3) && near(e.cr_k, 0.2));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn fork_copies_and_inherited_prefixes_count_once() {
        let home = temp_home("forks");
        let day = day_dir(&home);
        // Parent: one legacy call and one recorded call.
        write_rollout(
            &day,
            "rollout-parent.jsonl",
            &[
                meta("2026-10-01T00:00:00Z", "parent"),
                turn("2026-10-01T00:00:01Z", "p1", "gpt-6-sol"),
                token_count(
                    "2026-10-01T00:00:02Z",
                    usage(1000, 0, 10),
                    usage(1000, 0, 10),
                    Value::Null,
                ),
                record(
                    "2026-10-01T00:00:03Z",
                    "parent",
                    "p1",
                    "resp_p",
                    usage(2000, 0, 20),
                ),
                token_count(
                    "2026-10-01T00:00:04Z",
                    usage(3000, 0, 30),
                    usage(2000, 0, 20),
                    Value::Null,
                ),
            ],
        );
        // Legacy fork: the parent's lines re-written with fork-time stamps,
        // records still under the parent's thread_id, then its own call.
        write_rollout(
            &day,
            "rollout-child.jsonl",
            &[
                line(
                    "2026-10-02T00:00:00Z",
                    "session_meta",
                    json!({ "id": "child", "session_id": "child", "forked_from_id": "parent" }),
                ),
                meta("2026-10-02T00:00:00Z", "parent"),
                turn("2026-10-02T00:00:00Z", "p1", "gpt-6-sol"),
                token_count(
                    "2026-10-02T00:00:00Z",
                    usage(1000, 0, 10),
                    usage(1000, 0, 10),
                    Value::Null,
                ),
                record(
                    "2026-10-02T00:00:00Z",
                    "parent",
                    "p1",
                    "resp_p",
                    usage(2000, 0, 20),
                ),
                token_count(
                    "2026-10-02T00:00:00Z",
                    usage(3000, 0, 30),
                    usage(2000, 0, 20),
                    Value::Null,
                ),
                turn("2026-10-02T00:00:01Z", "c1", "gpt-6-sol"),
                record(
                    "2026-10-02T00:00:02Z",
                    "child",
                    "c1",
                    "resp_c",
                    usage(4000, 0, 40),
                ),
            ],
        );
        // Paginated subagent: the prefix below its start ordinal is the
        // parent's (a guardian checkpoint repeats the trunk's snapshot).
        let ord = |n: u64, l: String| {
            let mut v: Value = serde_json::from_str(&l).unwrap();
            v["ordinal"] = json!(n);
            v.to_string()
        };
        let mut sub_meta: Value =
            serde_json::from_str(&meta("2026-10-03T00:00:00Z", "sub")).unwrap();
        sub_meta["payload"]["subagent_history_start_ordinal"] = json!(3);
        write_rollout(
            &day,
            "rollout-sub.jsonl",
            &[
                ord(0, sub_meta.to_string()),
                ord(1, turn("2026-10-03T00:00:00Z", "p1", "gpt-6-sol")),
                ord(
                    2,
                    token_count(
                        "2026-10-03T00:00:00Z",
                        usage(9000, 0, 90),
                        usage(9000, 0, 90),
                        rate_limits(99.0, 99.0),
                    ),
                ),
                ord(3, turn("2026-10-03T00:00:01Z", "s1", "gpt-6-luna")),
                ord(
                    4,
                    record(
                        "2026-10-03T00:00:02Z",
                        "sub",
                        "s1",
                        "resp_s",
                        usage(8000, 0, 80),
                    ),
                ),
            ],
        );

        let data = collect_from(&home);
        let (sol, _, _) = sum(&data, "gpt-6-sol");
        assert!(
            near(sol, 1.0 + 2.0 + 4.0),
            "parent once + child's own, got {sol}"
        );
        let (luna, _, _) = sum(&data, "gpt-6-luna");
        assert!(near(luna, 8.0));
        assert!(
            data.quota.is_none(),
            "inherited snapshot is not this thread's"
        );
        let child: Vec<_> = data
            .entries
            .iter()
            .filter(|e| e.session_id.as_deref() == Some("child"))
            .collect();
        assert_eq!(child.len(), 1, "first session_meta names the file's thread");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn identical_snapshots_in_unrelated_threads_both_count() {
        let home = temp_home("twins");
        let day = day_dir(&home);
        write_rollout(
            &day,
            "rollout-x.jsonl",
            &one_call("x", "2026-10-01T01:00:00Z", "gpt-6-sol", 1000),
        );
        write_rollout(
            &day,
            "rollout-y.jsonl",
            &one_call("y", "2026-10-01T02:00:00Z", "gpt-6-sol", 1000),
        );
        let (input, _, _) = sum(&collect_from(&home), "gpt-6-sol");
        assert!(near(input, 2.0), "got {input}");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn quota_tracks_the_main_bucket_by_window_length() {
        let home = temp_home("quota");
        let tc = |ts: &str, rl: Value| token_count(ts, usage(10, 0, 1), usage(10, 0, 1), rl);
        let mut other = rate_limits(90.0, 90.0);
        other["limit_id"] = json!("codex_other");
        let mut main = rate_limits(40.0, 20.0);
        main["limit_id"] = json!("codex");
        write_rollout(
            &day_dir(&home),
            "rollout-q.jsonl",
            &[
                meta("2026-10-01T00:00:00Z", "thr"),
                tc("2026-10-01T00:00:01Z", main),
                // Another bucket overwrote Codex's single snapshot slot…
                tc("2026-10-01T00:00:02Z", other),
                // …and a metadata-only update nulled the windows.
                tc(
                    "2026-10-01T00:00:03Z",
                    json!({"limit_id":"codex","primary":null,"secondary":null}),
                ),
                // A newer main-bucket weekly window reported as primary.
                tc(
                    "2026-10-01T00:00:04Z",
                    json!({"limit_id":"codex","primary":{"used_percent":25.0,"window_minutes":10080,"resets_at":4103049600i64},"secondary":null}),
                ),
            ],
        );
        let q = collect_from(&home).quota.expect("quota");
        assert_eq!(q.primary_used_pct, 40.0, "5h from the last main snapshot");
        assert_eq!(q.secondary_used_pct, 25.0, "weekly classified by length");
        assert_eq!(q.secondary_window_minutes, 10080);
        assert_eq!(
            DateTime::parse_from_rfc3339(&q.as_of).unwrap().timestamp(),
            DateTime::parse_from_rfc3339("2026-10-01T00:00:04Z")
                .unwrap()
                .timestamp()
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn old_quota_schema_gets_absolute_reset_times() {
        let ts = DateTime::parse_from_rfc3339("2026-07-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let relative = json!({"used_percent":3,"window_minutes":300,"resets_in_seconds":60});
        let w = window_obs(&relative, ts, (0, 0)).expect("window");
        assert_eq!(w.resets_at, Some(ts.timestamp() + 60));
        assert_eq!(w.used_pct, 3.0);
        let rfc =
            json!({"used_percent":15,"window_minutes":10080,"resets_at":"2026-07-02T00:00:00Z"});
        let w = window_obs(&rfc, ts, (0, 0)).expect("window");
        assert_eq!(w.resets_at, Some(ts.timestamp() + 86_400));
        assert!(window_obs(&Value::Null, ts, (0, 0)).is_none());
    }

    #[test]
    fn expired_quota_windows_are_zeroed_on_each_scan() {
        let mut q = CodexQuota {
            as_of: "2026-07-01T00:00:00Z".into(),
            primary_used_pct: 65.0,
            primary_window_minutes: 300,
            primary_resets_at: Some(100),
            secondary_used_pct: 19.0,
            secondary_window_minutes: 10080,
            secondary_resets_at: Some(300),
            plan_type: Some("plus".into()),
        };

        expire_quota(&mut q, 200);
        assert_eq!(q.primary_used_pct, 0.0);
        assert_eq!(q.secondary_used_pct, 19.0);
    }

    #[test]
    fn scan_cache_rereads_grown_files() {
        let home = temp_home("cache");
        let path = write_rollout(
            &day_dir(&home),
            "rollout-c.jsonl",
            &one_call("c", "2026-10-01T00:00:00Z", "gpt-6-sol", 1000),
        );
        let mut cache = BTreeMap::new();
        let first = scan_cached(&path, &mut cache).unwrap();
        assert!(Arc::ptr_eq(
            &first,
            &scan_cached(&path, &mut cache).unwrap()
        ));
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(
            f,
            "{}",
            token_count(
                "2026-10-01T00:00:01Z",
                usage(3000, 0, 20),
                usage(2000, 0, 10),
                Value::Null
            )
        )
        .unwrap();
        drop(f);
        let second = scan_cached(&path, &mut cache).unwrap();
        assert_eq!(second.fallback.len(), 2);
        let _ = std::fs::remove_dir_all(&home);
    }
}
