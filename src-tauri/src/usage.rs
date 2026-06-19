use chrono::{DateTime, Datelike, Duration, Local, TimeZone, Timelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Deserialize)]
struct LogLine {
    #[serde(rename = "type")]
    ty: Option<String>,
    timestamp: Option<String>,
    message: Option<MessageField>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    // Top-level request id — half of ccusage's dedup key. The same message.id
    // can recur under a new request id when /btw replays a parent message.
    #[serde(rename = "requestId")]
    request_id: Option<String>,
    // Marks subagent / sidechain rows. ccusage prefers the non-sidechain copy
    // when a message is replayed across a sidechain.
    #[serde(rename = "isSidechain")]
    is_sidechain: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum MessageField {
    Obj(MessageObj),
    #[allow(dead_code)]
    Other(serde_json::Value),
}

#[derive(Debug, Deserialize)]
struct MessageObj {
    id: Option<String>,
    model: Option<String>,
    usage: Option<UsageObj>,
    content: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct UsageObj {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
    #[serde(default)]
    speed: Option<String>,
    #[serde(default)]
    service_tier: Option<String>,
    // Breakdown of cache_creation_input_tokens by TTL. 5m-TTL cache writes
    // are billed at 1.25× base input, 1h-TTL at 2× — so we must split them
    // to price correctly. Older JSONL lines omit this object; we then treat
    // all cache creation as 5m (the historical behaviour).
    #[serde(default)]
    cache_creation: Option<CacheCreation>,
}

#[derive(Debug, Deserialize)]
struct CacheCreation {
    #[serde(default)]
    ephemeral_1h_input_tokens: u64,
}

#[derive(Debug, Serialize, Clone, Default)]
pub struct ModelUsage {
    #[serde(rename = "in")]
    pub input: f64,
    pub out: f64,
    #[serde(rename = "cacheWrite")]
    pub cache_write: f64,
    // The 1h-TTL subset of `cache_write` (which is the 5m+1h total). Carried
    // separately only so cost can reprice it at 2× — token displays and
    // total_tokens keep using `cache_write` and must NOT add this, or the 1h
    // tokens would be double-counted.
    #[serde(rename = "cacheWrite1h")]
    pub cache_write_1h: f64,
    #[serde(rename = "cacheRead")]
    pub cache_read: f64,
    // Dollar cost, computed by the backend so the frontend doesn't need
    // to keep a parallel pricing table. Kept as Option<> so older cached
    // payloads deserialize cleanly in the browser preview path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
}

#[derive(Debug, Serialize, Clone, Default)]
pub struct ToolUsage {
    pub name: String,
    pub calls: u64,
    pub tokens: u64,
}

#[derive(Debug, Serialize, Clone, Default)]
pub struct Usage {
    #[serde(rename = "byModel")]
    pub by_model: HashMap<String, ModelUsage>,
    #[serde(rename = "byTool")]
    pub by_tool: Vec<ToolUsage>,
    #[serde(rename = "sparkline")]
    pub sparkline: Vec<f64>,
    #[serde(rename = "sparkLeft")]
    pub spark_left: String,
    #[serde(rename = "sparkRight")]
    pub spark_right: String,
    #[serde(rename = "rangeLabel")]
    pub range_label: String,
    #[serde(rename = "totalCost")]
    pub total_cost: f64,
    #[serde(rename = "totalTokens")]
    pub total_tokens: f64,
    #[serde(rename = "deltaPct")]
    pub delta_pct: Option<f64>,
    // Rolling-window credit meters. Always computed regardless of selected
    // range, so the UI can show subscription-quota progress.
    #[serde(rename = "creditUsed5h")]
    pub credit_used_5h: f64,
    #[serde(rename = "creditUsedWeek")]
    pub credit_used_week: f64,
    #[serde(rename = "creditLimit5h")]
    pub credit_limit_5h: f64,
    #[serde(rename = "creditLimitWeek")]
    pub credit_limit_week: f64,
    #[serde(rename = "planLabel")]
    pub plan_label: String,
    #[serde(rename = "planId")]
    pub plan_id: String,
    #[serde(rename = "sessionStart")]
    pub session_start: Option<String>,
    // Raw-token rollups — ccusage-style. Useful as a second reference
    // against the derived "credit" number, since Anthropic has never
    // publicly defined the token↔credit ratio.
    #[serde(rename = "tokens5h")]
    pub tokens_5h: f64,
    #[serde(rename = "tokensWeek")]
    pub tokens_week: f64,
    // Number of events in the current 5h window whose service_tier was
    // "priority" (or legacy speed=="fast"). Zero on this user's data
    // today but we surface it for observability.
    #[serde(rename = "priorityEvents5h")]
    pub priority_events_5h: u64,
    // ISO timestamp of when the current 5h block started. Defined as
    // the earliest event within the last 5 hours; becomes None if there
    // has been no activity in the last 5 hours.
    #[serde(rename = "activeBlockStart")]
    pub active_block_start: Option<String>,
    // ccusage-compatible raw-token limit for the 5h block, empirically
    // observed as ~58.6M tokens for Max 5x (the historical 100% ceiling).
    // Surfaced alongside the credit-based limit so the UI can show both.
    #[serde(rename = "tokenLimit5h")]
    pub token_limit_5h: f64,
}

fn normalize_model(raw: &str) -> Option<String> {
    let r = raw.to_lowercase();
    if r.contains("synthetic") {
        return None;
    }
    if !(r.contains("opus") || r.contains("haiku") || r.contains("sonnet") || r.contains("fable")) {
        return None;
    }
    // Strip trailing -YYYYMMDD date suffix
    let canonical = match r.rsplit_once('-') {
        Some((head, tail))
            if tail.len() == 8 && tail.chars().all(|c| c.is_ascii_digit()) =>
        {
            head.to_string()
        }
        _ => r.clone(),
    };
    Some(canonical)
}

struct Pricing {
    input: f64,
    output: f64,
    // 5m-TTL cache write = 1.25× base input.
    cache_write: f64,
    // 1h-TTL cache write = 2× base input. ccusage splits these; lumping 1h
    // into the 5m rate undercounts heavy cache users (deep research) ~10%.
    cache_write_1h: f64,
    cache_read: f64,
}

fn pricing(model: &str) -> Pricing {
    let m = model.to_lowercase();
    let is_fast = m.contains("-fast");
    let base = if m.contains("fable") {
        // Fable 5 pricing as of 2026-06 ($10/$50 per MTok)
        Pricing {
            input: 10.0,
            output: 50.0,
            cache_write: 12.5,
            cache_write_1h: 20.0,
            cache_read: 1.0,
        }
    } else if m.contains("opus") {
        // Opus 4.6/4.7 pricing as of 2026 (per ccusage)
        Pricing {
            input: 5.0,
            output: 25.0,
            cache_write: 6.25,
            cache_write_1h: 10.0,
            cache_read: 0.5,
        }
    } else if m.contains("haiku") {
        Pricing {
            input: 1.0,
            output: 5.0,
            cache_write: 1.25,
            cache_write_1h: 2.0,
            cache_read: 0.1,
        }
    } else {
        // sonnet (any version) and fallback
        Pricing {
            input: 3.0,
            output: 15.0,
            cache_write: 3.75,
            cache_write_1h: 6.0,
            cache_read: 0.3,
        }
    };
    if is_fast {
        // Claude Code priority/fast tier — 6x standard
        Pricing {
            input: base.input * 6.0,
            output: base.output * 6.0,
            cache_write: base.cache_write * 6.0,
            cache_write_1h: base.cache_write_1h * 6.0,
            cache_read: base.cache_read * 6.0,
        }
    } else {
        base
    }
}

fn calc_cost_k(u: &ModelUsage, model: &str) -> f64 {
    let p = pricing(model);
    // cache_write is the 5m+1h total; the 5m portion is the remainder after
    // peeling off the 1h subset (clamped so a stray 1h>total can't go negative).
    let cw_5m = (u.cache_write - u.cache_write_1h).max(0.0);
    (u.input * p.input
        + u.out * p.output
        + cw_5m * p.cache_write
        + u.cache_write_1h * p.cache_write_1h
        + u.cache_read * p.cache_read)
        / 1000.0
}

// Subscription plan limits. Each Claude Code subscription tier has two
// rolling quota windows: a 5-hour rolling window and a 7-day rolling window.
// Numbers verified 2026-04 from the Claude Max help-center docs.
//
//   Pro       $20/mo   5h:    550_000  week:   5_000_000
//   Max 5x   $100/mo   5h:  3_300_000  week:  41_670_000
//   Max 20x  $200/mo   5h: 11_000_000  week:  83_330_000
#[derive(Debug, Clone, Copy)]
pub struct Plan {
    pub id: &'static str,
    pub label: &'static str,
    // Credit-based limits (est., from third-party community sources).
    pub limit_5h: f64,
    pub limit_week: f64,
    // Raw-token 5h limit — ccusage-compatible. Max 5x's ~58.6M figure is
    // derived empirically from historical 100% blocks; Pro and Max 20x
    // scale proportionally to the credit ratio until we can verify
    // them directly.
    pub token_limit_5h: f64,
}

// Plan limits are in *weighted-credits* (priority-tier burndown units).
//
// CALIBRATION (2026-04-23): Cross-referenced against the Anthropic
// Settings → Usage page, which reports "Current session: 17% used"
// for a Max 5x user whose weighted_credits 5h sum was ~10M. That
// pins the actual 5h cap at ≈ 60M weighted credits (10M / 0.17).
// Earlier we derived 10M from ccusage's 58.6M raw-token empirical
// observation — but ccusage's own label says "assuming 58,642,988
// token limit", which is just the historical max of this user's
// blocks, not a documented cap. Anthropic's 17% is the ground truth.
//
// Pro and Max 20x scale from Max 5x using the third-party credit
// ratios (Pro ≈ 1/6 of 5x, Max 20x ≈ 10/3 of 5x). Weekly caps use
// the same ~12.6× week-to-5h ratio from the third-party numbers.
pub const PLAN_PRO: Plan = Plan {
    id: "pro",
    label: "Pro",
    limit_5h: 10_000_000.0,   // 60M × (550K / 3.3M)
    limit_week: 126_000_000.0, // 10M × 12.6
    token_limit_5h: 58_000_000.0, // raw-token ceiling, proportional
};
pub const PLAN_MAX_5X: Plan = Plan {
    id: "max5x",
    label: "Max 5x",
    limit_5h: 60_000_000.0,    // calibrated from Anthropic Settings 17%
    limit_week: 756_000_000.0, // 60M × 12.6
    token_limit_5h: 350_000_000.0, // ≈ 58.6M / 0.17
};
pub const PLAN_MAX_20X: Plan = Plan {
    id: "max20",
    label: "Max 20x",
    limit_5h: 200_000_000.0,     // 60M × (11M / 3.3M)
    limit_week: 2_520_000_000.0, // 200M × 12.6
    token_limit_5h: 1_166_000_000.0, // proportional
};

pub fn plan_from_id(id: &str) -> Plan {
    match id {
        "pro" => PLAN_PRO,
        "max20" | "max-20x" | "max_20x" => PLAN_MAX_20X,
        _ => PLAN_MAX_5X,
    }
}

// Credit conversion.
//
// Anthropic does NOT publicly document the Max-subscription credit
// formula. The nearest public analogue is the Priority-tier burndown
// formula (platform.claude.com/docs/en/api/service-tiers), which we
// adopt as a defensible public heuristic:
//
//   credit = input + output + 0.1 × cache_read + 1.25 × cache_write_5m
//
// No model multiplier, no 5× on output. Those are our adjustments
// from an earlier cost-based formula; the priority-tier doc treats
// input and output equally and doesn't weight by family.
//
// Caveats: (1) cache_creation_input_tokens lumps both 5m and 1h TTLs;
// we treat it as 5m (1.25×) since 5m is the common case. (2) Max
// subscription quota probably follows a related but non-identical
// formula — treat this as an informed estimate.
fn weighted_credits(u: &UsageObj) -> f64 {
    let input = u.input_tokens as f64;
    let output = u.output_tokens as f64;
    let cache_read = u.cache_read_input_tokens as f64;
    let cache_write = u.cache_creation_input_tokens as f64;
    input + output + 0.1 * cache_read + 1.25 * cache_write
}

fn projects_root() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude").join("projects"))
}

fn list_jsonl_files() -> Vec<PathBuf> {
    let Some(root) = projects_root() else { return Vec::new(); };
    let pattern = format!("{}/**/*.jsonl", root.display());
    glob::glob(&pattern)
        .map(|it| it.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

// ---- All-time persistence -------------------------------------------------
//
// Claude Code prunes transcripts older than `cleanupPeriodDays` (default 30),
// so summing the on-disk JSONL only yields a rolling ~30-day window — the
// "All-time" total silently shrinks as old days age out. To make All-time
// genuinely cumulative we persist a per-day, per-model token tally to the OS
// app-data dir and overlay the live (still-on-disk) days on top each poll.
// Pruned days survive in the store; present days keep growing.

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
struct TokenTally {
    #[serde(default)]
    input: f64,
    #[serde(default)]
    out: f64,
    #[serde(default, rename = "cacheWrite")]
    cache_write: f64,
    // 1h-TTL subset of cache_write. Defaults to 0 for store files written
    // before this field existed; max-merge backfills it for any day still
    // on disk on the next poll (pruned-pre-fix days stay at 0 — they predate
    // the data and are a negligible tail).
    #[serde(default, rename = "cacheWrite1h")]
    cache_write_1h: f64,
    #[serde(default, rename = "cacheRead")]
    cache_read: f64,
}

// date (YYYY-MM-DD, local) -> model -> token tally. BTreeMap so serialization
// is deterministic (stable key order) — that's what makes the "save only if
// changed" diff below reliable, and keeps the on-disk file human-diffable.
type DailyStore = BTreeMap<String, BTreeMap<String, TokenTally>>;

// Serialize access to the on-disk store so the 15s poll, the file-watcher
// refresh, and the background range-warmer can't interleave a read-modify-write.
static STORE_LOCK: Mutex<()> = Mutex::new(());

fn store_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("claude-code-token-counter").join("daily_usage.json"))
}

fn load_store_at(p: &std::path::Path) -> DailyStore {
    let Ok(text) = std::fs::read_to_string(p) else { return DailyStore::new(); };
    serde_json::from_str(&text).unwrap_or_default()
}

fn save_store_at(p: &std::path::Path, store: &DailyStore) {
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(json) = serde_json::to_string(store) else { return; };
    // tmp + rename so a crash mid-write can't corrupt the live file.
    let tmp = p.with_extension("json.tmp");
    if std::fs::write(&tmp, json.as_bytes()).is_ok() {
        let _ = std::fs::rename(&tmp, p);
    }
}

fn load_store() -> DailyStore {
    match store_path() {
        Some(p) => load_store_at(&p),
        None => DailyStore::new(),
    }
}

fn save_store(store: &DailyStore) {
    if let Some(p) = store_path() {
        save_store_at(&p, store);
    }
}

// Overlay freshly-computed live days onto the persisted store using an
// elementwise MAX per (date, model, field) — NOT overwrite.
//
// Why max: `cleanupPeriodDays` prunes per *file* (by mtime), and one calendar
// day's events can span several session files (a session resumed across
// midnight, subagent/workflow logs). So a day passes through a window where
// it's *partially* pruned — one file gone, another still present — and during
// that window `live[date]` is smaller than the day's true total. Overwrite
// would replace the good stored value with the diminished one and silently
// lose spend (the very bug we're fixing, in miniature). Max dominates
// overwrite: equal for a fully-present day, and only differs when live < stored
// (i.e. data loss), which is exactly when we want to keep the stored value.
// Consequence by design: if a transcript is ever rewritten *downward*, max
// keeps the higher figure — correct for a spend counter (the spend happened).
fn merge_live(store: &mut DailyStore, live: &DailyStore) {
    for (date, models) in live {
        let day = store.entry(date.clone()).or_default();
        for (model, t) in models {
            let e = day.entry(model.clone()).or_default();
            e.input = e.input.max(t.input);
            e.out = e.out.max(t.out);
            e.cache_write = e.cache_write.max(t.cache_write);
            e.cache_write_1h = e.cache_write_1h.max(t.cache_write_1h);
            e.cache_read = e.cache_read.max(t.cache_read);
        }
    }
}

// Collapse the per-day store into a single cumulative per-model tally.
fn sum_store(store: &DailyStore) -> HashMap<String, TokenTally> {
    let mut out: HashMap<String, TokenTally> = HashMap::new();
    for models in store.values() {
        for (model, t) in models {
            let e = out.entry(model.clone()).or_default();
            e.input += t.input;
            e.out += t.out;
            e.cache_write += t.cache_write;
            e.cache_write_1h += t.cache_write_1h;
            e.cache_read += t.cache_read;
        }
    }
    out
}

// Merge the live days into the persisted store (saving only if something grew)
// and return the cumulative all-time per-model tally. Returns None when the
// store path/IO is unavailable so the caller falls back to window-only data.
fn persist_and_cumulative(live: &DailyStore) -> Option<HashMap<String, TokenTally>> {
    let _guard = STORE_LOCK.lock().ok()?;
    let mut store = load_store();
    let before = serde_json::to_string(&store).unwrap_or_default();
    merge_live(&mut store, live);
    let after = serde_json::to_string(&store).unwrap_or_default();
    if before != after {
        save_store(&store);
    }
    Some(sum_store(&store))
}

pub enum Range {
    Session,
    Today,
    Week,
    Month,
    All,
}

impl Range {
    fn parse(s: &str) -> Self {
        match s {
            "session" => Range::Session,
            "today" => Range::Today,
            "week" => Range::Week,
            "month" => Range::Month,
            _ => Range::All,
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Range::Session => "Current session",
            Range::Today => "Today",
            Range::Week => "This week",
            Range::Month => "This month",
            Range::All => "All-time",
        }
    }

    fn start(&self) -> Option<DateTime<Utc>> {
        let now = Local::now();
        match self {
            // Session is handled via session_id filter, not a time window
            Range::Session => None,
            Range::Today => {
                let d = now.date_naive().and_hms_opt(0, 0, 0).unwrap();
                Local.from_local_datetime(&d).single().map(|x| x.with_timezone(&Utc))
            }
            Range::Week => {
                let weekday = now.weekday().num_days_from_monday() as i64;
                let d = (now - Duration::days(weekday))
                    .date_naive()
                    .and_hms_opt(0, 0, 0)
                    .unwrap();
                Local.from_local_datetime(&d).single().map(|x| x.with_timezone(&Utc))
            }
            Range::Month => {
                let d = now
                    .date_naive()
                    .with_day(1)
                    .unwrap()
                    .and_hms_opt(0, 0, 0)
                    .unwrap();
                Local.from_local_datetime(&d).single().map(|x| x.with_timezone(&Utc))
            }
            Range::All => None,
        }
    }
}

fn extract_tool_uses(content: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(arr) = content.as_array() {
        for item in arr {
            if item.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                if let Some(name) = item.get("name").and_then(|v| v.as_str()) {
                    out.push(name.to_string());
                }
            }
        }
    }
    out
}

fn find_latest_session() -> Option<(String, DateTime<Utc>, DateTime<Utc>)> {
    let mut latest: Option<(DateTime<Utc>, String)> = None;
    let mut spans: HashMap<String, (DateTime<Utc>, DateTime<Utc>)> = HashMap::new();
    for path in list_jsonl_files() {
        let Ok(file) = File::open(&path) else { continue };
        let reader = BufReader::new(file);
        for line in reader.lines() {
            let Ok(line) = line else { continue };
            if line.trim().is_empty() {
                continue;
            }
            let Ok(ev) = serde_json::from_str::<LogLine>(&line) else { continue };
            if ev.ty.as_deref() != Some("assistant") {
                continue;
            }
            let Some(sid) = ev.session_id.as_deref() else { continue };
            let Some(ts) = ev
                .timestamp
                .as_deref()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|d| d.with_timezone(&Utc))
            else {
                continue;
            };
            let entry = spans.entry(sid.to_string()).or_insert((ts, ts));
            if ts < entry.0 {
                entry.0 = ts;
            }
            if ts > entry.1 {
                entry.1 = ts;
            }
            match &latest {
                Some((t, _)) if *t >= ts => {}
                _ => latest = Some((ts, sid.to_string())),
            }
        }
    }
    latest.and_then(|(_, sid)| {
        spans.get(&sid).map(|(s, e)| (sid.clone(), *s, *e))
    })
}

struct SparkCfg {
    start: DateTime<Utc>,
    bucket_seconds: i64,
    count: usize,
    left: String,
    right: String,
}

fn spark_config(range: &Range, session_span: Option<(DateTime<Utc>, DateTime<Utc>)>) -> SparkCfg {
    let now = Utc::now();
    let local_now = Local::now();
    match range {
        Range::Today => {
            let midnight = local_now
                .date_naive()
                .and_hms_opt(0, 0, 0)
                .and_then(|d| Local.from_local_datetime(&d).single())
                .map(|d| d.with_timezone(&Utc))
                .unwrap_or(now);
            SparkCfg {
                start: midnight,
                bucket_seconds: 3600,
                count: 24,
                left: "12am".into(),
                right: "now".into(),
            }
        }
        Range::Week => {
            let weekday = local_now.weekday().num_days_from_monday() as i64;
            let start = (local_now - Duration::days(weekday))
                .date_naive()
                .and_hms_opt(0, 0, 0)
                .and_then(|d| Local.from_local_datetime(&d).single())
                .map(|d| d.with_timezone(&Utc))
                .unwrap_or(now - Duration::days(7));
            SparkCfg {
                start,
                bucket_seconds: 86400,
                count: 7,
                left: "Mon".into(),
                right: "Today".into(),
            }
        }
        Range::Month => {
            let start = local_now
                .date_naive()
                .with_day(1)
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .and_then(|d| Local.from_local_datetime(&d).single())
                .map(|d| d.with_timezone(&Utc))
                .unwrap_or(now - Duration::days(30));
            let days = local_now.day() as usize;
            SparkCfg {
                start,
                bucket_seconds: 86400,
                count: days.max(1),
                left: "1st".into(),
                right: "Today".into(),
            }
        }
        Range::All => SparkCfg {
            start: (local_now - Duration::days(29))
                .date_naive()
                .and_hms_opt(0, 0, 0)
                .and_then(|d| Local.from_local_datetime(&d).single())
                .map(|d| d.with_timezone(&Utc))
                .unwrap_or(now - Duration::days(30)),
            bucket_seconds: 86400,
            count: 30,
            left: "30d ago".into(),
            right: "Today".into(),
        },
        Range::Session => {
            if let Some((start, end)) = session_span {
                let span = (end - start).num_seconds().max(60);
                let bucket = (span / 20).max(30);
                SparkCfg {
                    start,
                    bucket_seconds: bucket,
                    count: 20,
                    left: "start".into(),
                    right: "now".into(),
                }
            } else {
                SparkCfg {
                    start: now - Duration::hours(1),
                    bucket_seconds: 180,
                    count: 20,
                    left: "start".into(),
                    right: "now".into(),
                }
            }
        }
    }
}

// One parsed assistant usage event, before deduplication. We materialize the
// whole set and then run ccusage's dedup pass over it, because the correct
// occurrence to keep (the one with the largest token total) can appear *after*
// earlier partial-streaming occurrences of the same (message_id, request_id).
struct Entry {
    ts: Option<DateTime<Utc>>,
    model: String,
    in_k: f64,
    out_k: f64,
    cw_k: f64,
    cw1h_k: f64,
    cr_k: f64,
    out_raw: u64,
    total_tokens: u64,
    credits: f64,
    cost_k: f64,
    is_priority: bool,
    is_sidechain: bool,
    message_id: Option<String>,
    request_id: Option<String>,
    session_id: Option<String>,
    tools: Vec<String>,
}

// Should `candidate` replace the already-kept `existing` row for the same
// message? Mirrors ccusage's `should_replace_deduped_entry`: prefer the
// non-sidechain row, then the larger token total (the completed streaming
// line, not a partial chunk), then the larger cost, then the priority row.
fn should_replace_entry(candidate: &Entry, existing: &Entry) -> bool {
    if candidate.is_sidechain != existing.is_sidechain {
        return existing.is_sidechain;
    }
    if candidate.total_tokens != existing.total_tokens {
        return candidate.total_tokens > existing.total_tokens;
    }
    if candidate.cost_k != existing.cost_k {
        return candidate.cost_k > existing.cost_k;
    }
    candidate.is_priority && !existing.is_priority
}

// Deduplicate parsed events the way ccusage (v20) does:
//   * key on (message_id, request_id);
//   * fall back to message_id alone when one side is a /btw sidechain replay
//     (those re-emit the parent message under a fresh request_id);
//   * among colliding rows keep the one `should_replace_entry` selects — the
//     max-token-total row, i.e. the completed line, not a partial streaming
//     chunk. (The old code kept the FIRST occurrence, which captured a partial
//     output_tokens count and under-reported output and cost.)
// Rows without a message_id can't be keyed and are kept as-is.
fn dedupe_entries(entries: Vec<Entry>) -> Vec<Entry> {
    let mut deduped: Vec<Entry> = Vec::with_capacity(entries.len());
    let mut exact: HashMap<(String, Option<String>), usize> = HashMap::new();
    let mut by_message: HashMap<String, Vec<usize>> = HashMap::new();

    for entry in entries {
        let Some(mid) = entry.message_id.clone() else {
            deduped.push(entry);
            continue;
        };
        let req = entry.request_id.clone();

        // Exact (message_id, request_id) hit, re-verified against the slot in
        // case it was overwritten by a sidechain merge to a different req.
        let mut hit = exact
            .get(&(mid.clone(), req.clone()))
            .copied()
            .filter(|&i| {
                deduped[i].message_id.as_deref() == Some(mid.as_str())
                    && deduped[i].request_id == req
            });

        // Sidechain fallback: same message_id, different request_id, where the
        // candidate or the kept row is a sidechain replay.
        if hit.is_none() {
            if let Some(idxs) = by_message.get(&mid) {
                for &i in idxs {
                    if deduped[i].message_id.as_deref() == Some(mid.as_str())
                        && (entry.is_sidechain || deduped[i].is_sidechain)
                    {
                        hit = Some(i);
                        break;
                    }
                }
            }
        }

        if let Some(i) = hit {
            if should_replace_entry(&entry, &deduped[i]) {
                let new_key = (mid.clone(), entry.request_id.clone());
                deduped[i] = entry;
                exact.insert(new_key, i);
                let bucket = by_message.entry(mid).or_default();
                if !bucket.contains(&i) {
                    bucket.push(i);
                }
            }
            continue;
        }

        let i = deduped.len();
        exact.insert((mid.clone(), req), i);
        by_message.entry(mid).or_default().push(i);
        deduped.push(entry);
    }

    deduped
}

#[cfg(test)]
pub fn compute_usage(range_str: &str) -> Usage {
    compute_usage_with_plan(range_str, "max5x")
}

pub fn compute_usage_with_plan(range_str: &str, plan_id: &str) -> Usage {
    let range = Range::parse(range_str);
    let range_start = range.start();
    let latest_session = if matches!(range, Range::Session) {
        find_latest_session()
    } else {
        None
    };
    let session_filter: Option<String> = latest_session.as_ref().map(|(sid, _, _)| sid.clone());
    let session_span = latest_session.as_ref().map(|(_, s, e)| (*s, *e));

    let plan = plan_from_id(plan_id);

    let mut by_model: HashMap<String, ModelUsage> = HashMap::new();
    let mut tool_counts: HashMap<String, (u64, u64)> = HashMap::new();
    // Per-day, per-model token tallies for the All-time persistence store —
    // accumulated for every event regardless of the selected UI range.
    let mut live_day_model: DailyStore = DailyStore::new();

    let today_local = Local::now().date_naive();
    let mut daily: Vec<f64> = vec![0.0; 30];

    // Rolling-window credit meters — always computed, independent of the
    // selected UI range. 5h window = Claude Code's in-session cap;
    // 7d window = the weekly cap. Both roll continuously; the clock
    // starts on the first billable event that's still inside the window.
    let now = Utc::now();
    let five_h_ago = now - Duration::hours(5);
    let seven_d_ago = now - Duration::days(7);
    let mut credit_5h: f64 = 0.0;
    let mut credit_week: f64 = 0.0;
    let mut tokens_5h: f64 = 0.0;
    let mut tokens_week: f64 = 0.0;
    let mut priority_events_5h: u64 = 0;
    let mut earliest_in_5h: Option<DateTime<Utc>> = None;

    let spark_cfg = spark_config(&range, session_span);
    let mut spark: Vec<f64> = vec![0.0; spark_cfg.count];

    // Pass 1 — parse every usable assistant event into an `Entry`. No dedup or
    // aggregation yet: we need the whole set first so the dedup can keep the
    // completed (max-token) occurrence of each message rather than the first.
    let mut entries: Vec<Entry> = Vec::new();
    let files = list_jsonl_files();
    for path in files {
        let Ok(file) = File::open(&path) else { continue; };
        let reader = BufReader::new(file);
        for line in reader.lines() {
            let Ok(line) = line else { continue; };
            if line.trim().is_empty() {
                continue;
            }
            let Ok(ev) = serde_json::from_str::<LogLine>(&line) else { continue; };
            if ev.ty.as_deref() != Some("assistant") {
                continue;
            }
            let Some(MessageField::Obj(msg)) = ev.message else { continue; };
            let Some(usage) = msg.usage else { continue; };
            let model_raw = msg.model.as_deref().unwrap_or("");
            let Some(mut model) = normalize_model(model_raw) else { continue; };
            // Priority-tier detection. Prefer the official Anthropic
            // `service_tier == "priority"` marker; fall back to the
            // legacy `speed == "fast"` tag some older JSONLs carry.
            let is_priority = usage.service_tier.as_deref() == Some("priority")
                || usage.speed.as_deref() == Some("fast");
            if is_priority && !model.contains("-fast") {
                model.push_str("-fast");
            }

            let ts = ev
                .timestamp
                .as_deref()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|d| d.with_timezone(&Utc));

            let in_k = usage.input_tokens as f64 / 1000.0;
            let out_k = usage.output_tokens as f64 / 1000.0;
            let cw_k = usage.cache_creation_input_tokens as f64 / 1000.0;
            // 1h-TTL portion of cache writes (subset of cw_k), priced at 2×.
            // Absent breakdown ⇒ 0 ⇒ everything stays 5m (historical behaviour).
            let cw1h_k = usage
                .cache_creation
                .as_ref()
                .map(|c| c.ephemeral_1h_input_tokens as f64 / 1000.0)
                .unwrap_or(0.0);
            let cr_k = usage.cache_read_input_tokens as f64 / 1000.0;

            // Total raw tokens — both the dedup tie-breaker (ccusage keeps the
            // largest) and the value summed into the rolling 5h/7d windows.
            let total_tokens = usage.input_tokens
                + usage.output_tokens
                + usage.cache_creation_input_tokens
                + usage.cache_read_input_tokens;

            let tmp = ModelUsage {
                input: in_k,
                out: out_k,
                cache_write: cw_k,
                cache_write_1h: cw1h_k,
                cache_read: cr_k,
                cost: None,
            };
            let cost_k = calc_cost_k(&tmp, &model);
            let credits = weighted_credits(&usage);
            let tools = msg
                .content
                .as_ref()
                .map(extract_tool_uses)
                .unwrap_or_default();

            entries.push(Entry {
                ts,
                model,
                in_k,
                out_k,
                cw_k,
                cw1h_k,
                cr_k,
                out_raw: usage.output_tokens,
                total_tokens,
                credits,
                cost_k,
                is_priority,
                is_sidechain: ev.is_sidechain.unwrap_or(false),
                message_id: msg.id,
                request_id: ev.request_id,
                session_id: ev.session_id,
                tools,
            });
        }
    }

    // Deduplicate exactly like ccusage before aggregating anything.
    let deduped = dedupe_entries(entries);

    // Pass 2 — aggregate the deduplicated set. Every accumulator (by_model,
    // tool calls, the All-time store, the 5h/7d windows, the sparkline and the
    // 30-day daily series) is driven from `deduped` so they all agree.
    for e in &deduped {
        let in_range = if let Some(ref sid) = session_filter {
            e.session_id.as_deref() == Some(sid.as_str())
        } else {
            match (e.ts, range_start) {
                (_, None) => true,
                (Some(t), Some(s)) => t >= s,
                (None, Some(_)) => false,
            }
        };

        if in_range {
            let entry = by_model.entry(e.model.clone()).or_default();
            entry.input += e.in_k;
            entry.out += e.out_k;
            entry.cache_write += e.cw_k;
            entry.cache_write_1h += e.cw1h_k;
            entry.cache_read += e.cr_k;

            for tool_name in &e.tools {
                let c = tool_counts.entry(tool_name.clone()).or_insert((0, 0));
                c.0 += 1;
                c.1 += e.out_raw;
            }
        }

        if let Some(t) = e.ts {
            let local_d = t.with_timezone(&Local).date_naive();
            let diff = today_local.signed_duration_since(local_d).num_days();

            // Tally this event into its local calendar day for the All-time
            // store. Independent of `in_range` so it captures every day no
            // matter which range the UI is showing.
            {
                let tally = live_day_model
                    .entry(local_d.to_string())
                    .or_default()
                    .entry(e.model.clone())
                    .or_default();
                tally.input += e.in_k;
                tally.out += e.out_k;
                tally.cache_write += e.cw_k;
                tally.cache_write_1h += e.cw1h_k;
                tally.cache_read += e.cr_k;
            }

            // Rolling windows — aggregated regardless of UI range.
            let raw_tokens = e.total_tokens as f64;
            if t >= five_h_ago {
                credit_5h += e.credits;
                tokens_5h += raw_tokens;
                if e.is_priority {
                    priority_events_5h += 1;
                }
                match earliest_in_5h {
                    Some(x) if x <= t => {}
                    _ => earliest_in_5h = Some(t),
                }
            }
            if t >= seven_d_ago {
                credit_week += e.credits;
                tokens_week += raw_tokens;
            }

            if (0..30).contains(&diff) {
                let idx = (29 - diff) as usize;
                daily[idx] += e.cost_k;
            }
            if in_range {
                let delta_secs = (t - spark_cfg.start).num_seconds();
                if delta_secs >= 0 {
                    let bucket = (delta_secs / spark_cfg.bucket_seconds) as usize;
                    if bucket < spark_cfg.count {
                        spark[bucket] += e.cost_k;
                    }
                }
            }
        }
    }

    // All-time persistence. Always update the store (so pruned days are
    // captured no matter which range is being viewed); for the All range,
    // rebuild by_model from the full cumulative history. total_cost and
    // total_tokens below then follow automatically and stay mutually
    // consistent. Deliberate inconsistency: the sparkline + tool-calls stay
    // window-only (~30d) in All view — only the headline spend/tokens and the
    // by-model split go cumulative. Skipped under cfg!(test) so `cargo test`
    // doesn't write to the user's real app-support store. Falls back to the
    // window-only aggregation if the store IO is unavailable.
    if !cfg!(test) {
        if let Some(cumulative) = persist_and_cumulative(&live_day_model) {
            if matches!(range, Range::All) {
                by_model = cumulative
                    .into_iter()
                    .map(|(m, tw)| {
                        (
                            m,
                            ModelUsage {
                                input: tw.input,
                                out: tw.out,
                                cache_write: tw.cache_write,
                                cache_write_1h: tw.cache_write_1h,
                                cache_read: tw.cache_read,
                                cost: None,
                            },
                        )
                    })
                    .collect();
            }
        }
    }

    // Fill per-model cost once here so the frontend doesn't need its own
    // pricing table (was a double-source-of-truth bug in v0.1).
    let total_cost: f64 = by_model
        .iter_mut()
        .map(|(m, u)| {
            let c = calc_cost_k(u, m);
            u.cost = Some(c);
            c
        })
        .sum();
    let total_tokens: f64 = by_model
        .values()
        .map(|u| u.input + u.out + u.cache_write + u.cache_read)
        .sum();

    let mut tools: Vec<ToolUsage> = tool_counts
        .into_iter()
        .map(|(name, (calls, tokens))| ToolUsage { name, calls, tokens })
        .collect();
    tools.sort_by(|a, b| b.calls.cmp(&a.calls));
    tools.truncate(6);

    let delta_pct = if daily.len() >= 2 {
        let today_v = *daily.last().unwrap();
        let yest_v = daily[daily.len() - 2];
        if yest_v > 0.0 {
            Some(((today_v - yest_v) / yest_v) * 100.0)
        } else {
            None
        }
    } else {
        None
    };

    Usage {
        by_model,
        by_tool: tools,
        sparkline: spark,
        spark_left: spark_cfg.left,
        spark_right: spark_cfg.right,
        range_label: range.label().to_string(),
        total_cost,
        total_tokens,
        delta_pct,
        credit_used_5h: credit_5h,
        credit_used_week: credit_week,
        credit_limit_5h: plan.limit_5h,
        credit_limit_week: plan.limit_week,
        plan_label: plan.label.to_string(),
        plan_id: plan.id.to_string(),
        session_start: earliest_in_5h.map(|t| t.to_rfc3339()),
        tokens_5h,
        tokens_week,
        priority_events_5h,
        // Round the earliest-in-5h timestamp DOWN to the hour to
        // mimic ccusage's block semantics (02:00 / 07:00 / ...).
        // A full session-anchored block detector is still TODO —
        // this approximation agrees with ccusage when the user is
        // continuously active, diverges across multi-hour gaps.
        active_block_start: earliest_in_5h.and_then(|t| {
            t.date_naive()
                .and_hms_opt(t.hour(), 0, 0)
                .and_then(|d| Utc.from_local_datetime(&d).single())
                .map(|d| d.to_rfc3339())
        }),
        token_limit_5h: plan.token_limit_5h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke_compute_all() {
        for r in ["session", "today", "week", "month", "all"] {
            let u = compute_usage(r);
            let pct5 = if u.credit_limit_5h > 0.0 {
                u.credit_used_5h / u.credit_limit_5h * 100.0
            } else {
                0.0
            };
            let pctw = if u.credit_limit_week > 0.0 {
                u.credit_used_week / u.credit_limit_week * 100.0
            } else {
                0.0
            };
            let pct_tok = if u.token_limit_5h > 0.0 {
                u.tokens_5h / u.token_limit_5h * 100.0
            } else {
                0.0
            };
            eprintln!(
                "{:10} ${:>6.2} tok={:>8.1}K | {} credits 5h={:>7.0}/{:<7.0} ({:>5.1}%) wk={:>5.1}% | raw 5h={:>7.0}K ({:>5.1}%) pri={}",
                r,
                u.total_cost,
                u.total_tokens,
                u.plan_label,
                u.credit_used_5h,
                u.credit_limit_5h,
                pct5,
                pctw,
                u.tokens_5h / 1000.0,
                pct_tok,
                u.priority_events_5h,
            );
        }
    }

    #[test]
    fn plan_ids_map_correctly() {
        assert_eq!(plan_from_id("pro").label, "Pro");
        assert_eq!(plan_from_id("max5x").label, "Max 5x");
        assert_eq!(plan_from_id("max20").label, "Max 20x");
        assert_eq!(plan_from_id("unknown").label, "Max 5x");
    }

    #[test]
    fn fable_model_recognized_and_priced() {
        assert_eq!(
            normalize_model("claude-fable-5").as_deref(),
            Some("claude-fable-5")
        );
        let p = pricing("claude-fable-5");
        assert_eq!(p.input, 10.0);
        assert_eq!(p.output, 50.0);
        assert_eq!(p.cache_write, 12.5);
        assert_eq!(p.cache_read, 1.0);
    }

    #[test]
    fn merge_live_uses_elementwise_max() {
        // A day that is partially pruned: the live recompute reports a SMALLER
        // tally than what we already stored (one of the day's session files
        // aged out). Max must keep the stored value, not lower it.
        let mut store: DailyStore = BTreeMap::new();
        let mut day = BTreeMap::new();
        day.insert(
            "claude-opus-4".to_string(),
            TokenTally { input: 10.0, out: 5.0, cache_write: 2.0, cache_write_1h: 1.0, cache_read: 100.0 },
        );
        store.insert("2026-05-20".to_string(), day);

        let mut live: DailyStore = BTreeMap::new();
        // same day, diminished (a session file pruned)
        let mut d_old = BTreeMap::new();
        d_old.insert(
            "claude-opus-4".to_string(),
            TokenTally { input: 4.0, out: 5.0, cache_write: 2.0, cache_write_1h: 0.5, cache_read: 40.0 },
        );
        live.insert("2026-05-20".to_string(), d_old);
        // a fresh new day
        let mut d_new = BTreeMap::new();
        d_new.insert(
            "claude-opus-4".to_string(),
            TokenTally { input: 7.0, out: 1.0, cache_write: 0.0, cache_write_1h: 3.0, cache_read: 9.0 },
        );
        live.insert("2026-06-16".to_string(), d_new);

        merge_live(&mut store, &live);

        let kept = &store["2026-05-20"]["claude-opus-4"];
        assert_eq!(kept.input, 10.0, "diminished live must not lower stored");
        assert_eq!(kept.cache_read, 100.0);
        assert_eq!(kept.cache_write_1h, 1.0, "1h subset must also max, not lower");
        assert_eq!(store["2026-06-16"]["claude-opus-4"].input, 7.0, "new day added");

        let total = sum_store(&store);
        let o = &total["claude-opus-4"];
        assert_eq!(o.input, 17.0, "10 kept + 7 new");
        assert_eq!(o.cache_read, 109.0, "100 kept + 9 new");
        assert_eq!(o.cache_write_1h, 4.0, "1.0 kept + 3.0 new");
    }

    #[test]
    fn store_roundtrips_through_disk() {
        // Proves the on-disk serialization survives a save -> load cycle,
        // including the camelCase rename on cache fields and f64 values — the
        // IO path the unit tests above don't exercise.
        let mut store: DailyStore = BTreeMap::new();
        let mut day = BTreeMap::new();
        day.insert(
            "claude-opus-4-fast".to_string(),
            TokenTally { input: 1.5, out: 2.5, cache_write: 3.5, cache_write_1h: 1.5, cache_read: 4.5 },
        );
        store.insert("2026-06-01".to_string(), day);

        let dir = std::env::temp_dir().join("cctc_test_store_roundtrip");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("daily_usage.json");
        save_store_at(&path, &store);

        // Raw JSON uses the camelCase keys.
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("cacheWrite"), "expected camelCase key, got: {raw}");
        assert!(raw.contains("claude-opus-4-fast"));

        let loaded = load_store_at(&path);
        assert_eq!(loaded, store, "store must survive disk roundtrip");

        // Missing file -> empty store, never a panic.
        let _ = std::fs::remove_dir_all(&dir);
        assert!(load_store_at(&path).is_empty());
    }

    #[test]
    fn cost_splits_1h_cache_at_2x() {
        // Opus, 1M cache-creation tokens of which 400k are 1h-TTL. Fields are
        // in k-tokens. 5m portion = 600k @ $6.25/M = $3.75; 1h = 400k @ $10/M
        // = $4.00; total $7.75. (Old all-5m behaviour would have been $6.25.)
        let u = ModelUsage {
            input: 0.0,
            out: 0.0,
            cache_write: 1000.0,
            cache_write_1h: 400.0,
            cache_read: 0.0,
            cost: None,
        };
        let c = calc_cost_k(&u, "claude-opus-4");
        assert!((c - 7.75).abs() < 1e-9, "got {c}, expected 7.75");

        // Clamp: a stray 1h > total must not produce negative 5m cost.
        let weird = ModelUsage {
            input: 0.0,
            out: 0.0,
            cache_write: 0.0,
            cache_write_1h: 100.0,
            cache_read: 0.0,
            cost: None,
        };
        let cw = calc_cost_k(&weird, "claude-opus-4");
        assert!((cw - 1.0).abs() < 1e-9, "got {cw}, expected 1.00 (100k @ $10/M)");
    }

    #[test]
    fn credit_math_consistent() {
        // Credit rollups should match cost × 1e5 within rounding.
        let u = compute_usage_with_plan("all", "max5x");
        // credit_week should be >= credit_5h
        assert!(u.credit_used_week >= u.credit_used_5h);
    }

    fn mk_entry(mid: &str, req: &str, out_raw: u64, is_sidechain: bool) -> Entry {
        Entry {
            ts: None,
            model: "claude-opus-4".to_string(),
            in_k: 1.0,
            out_k: out_raw as f64 / 1000.0,
            cw_k: 0.0,
            cw1h_k: 0.0,
            cr_k: 0.0,
            out_raw,
            // input (1000) + output — grows with output, like a streaming line.
            total_tokens: 1000 + out_raw,
            credits: 0.0,
            cost_k: out_raw as f64 / 1000.0,
            is_priority: false,
            is_sidechain,
            message_id: Some(mid.to_string()),
            request_id: Some(req.to_string()),
            session_id: None,
            tools: Vec::new(),
        }
    }

    #[test]
    fn dedupe_keeps_completed_streaming_line_not_partial() {
        // The same (message_id, request_id) is logged several times as the
        // assistant message streams; output_tokens (and the total) grow until
        // the completed line. ccusage keeps the max-total row — the old
        // keep-first logic kept the tiny partial and under-reported output.
        let entries = vec![
            mk_entry("msg-a", "req-a", 2, false),
            mk_entry("msg-a", "req-a", 2, false),
            mk_entry("msg-a", "req-a", 226, false),
        ];
        let deduped = dedupe_entries(entries);
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].out_raw, 226, "must keep the completed line, not a partial");
    }

    #[test]
    fn dedupe_merges_sidechain_replay_preferring_parent() {
        // /btw replays the parent message under a fresh request_id on a
        // sidechain. ccusage collapses these to one row and keeps the
        // non-sidechain parent even though the replay has more tokens.
        let entries = vec![
            mk_entry("msg-p", "req-parent", 10, false),
            mk_entry("msg-p", "req-replay", 50_000, true),
            mk_entry("msg-s", "req-s", 30, true), // distinct message, kept
        ];
        let deduped = dedupe_entries(entries);
        assert_eq!(deduped.len(), 2);
        let parent = deduped
            .iter()
            .find(|e| e.message_id.as_deref() == Some("msg-p"))
            .unwrap();
        assert_eq!(parent.request_id.as_deref(), Some("req-parent"));
        assert!(!parent.is_sidechain, "non-sidechain parent must win over a sidechain replay");
        assert_eq!(parent.out_raw, 10);
    }
}
