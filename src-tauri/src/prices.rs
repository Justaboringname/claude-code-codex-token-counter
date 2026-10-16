// Remote price table — LiteLLM's model_prices_and_context_window.json (the
// same table ccusage uses), refreshed at most once a day and cached on disk.
//
// Why: the hand-maintained table in usage.rs silently priced brand-new models
// (GPT-6 Astra, Opus 5.5) with a family fallback. LiteLLM matched the official
// OpenAI/Anthropic pages for every model checked on 2026-10-16 and listed
// GPT-6 Astra the day it launched. It is the first pricing layer; the built-in
// table stays as the offline fallback.
//
// Only the standard-tier per-token rates are kept. Batch/flex/priority and the
// long-context (>272K, >100K) tiers are ignored: the per-day rollups can't tell
// which requests crossed a threshold.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

pub const SOURCE_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";
const MAX_AGE_SECS: i64 = 24 * 3600;
// A parse that keeps fewer models than this is treated as a broken download.
const MIN_ENTRIES: usize = 50;
// Above $1,000/MTok is a data error, not a price (the priciest kept row, o1-pro
// output, is $600).
const MAX_PER_MTOK: f64 = 1000.0;

/// Standard-tier rates in $ per million tokens. Cache rates are `None` when
/// LiteLLM doesn't publish them; usage.rs fills those gaps.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rates {
    pub input: f64,
    pub output: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_1h: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PriceTable {
    /// RFC 3339 time of the last successful check (200 or 304).
    pub fetched_at: String,
    #[serde(default)]
    pub etag: Option<String>,
    pub models: BTreeMap<String, Rates>,
}

impl Rates {
    fn is_valid(&self) -> bool {
        let ok = |x: f64| (0.0..=MAX_PER_MTOK).contains(&x);
        ok(self.input)
            && ok(self.output)
            && [self.cache_read, self.cache_write, self.cache_write_1h]
                .iter()
                .all(|c| c.is_none_or(ok))
    }
}

impl PriceTable {
    pub fn get(&self, model: &str) -> Option<&Rates> {
        self.models.get(model)
    }

    fn is_fresh(&self, now: chrono::DateTime<chrono::Utc>) -> bool {
        // A fetched_at in the future (clock was wrong) counts as stale.
        chrono::DateTime::parse_from_rfc3339(&self.fetched_at)
            .map(|t| {
                (0..MAX_AGE_SECS).contains(&(now - t.with_timezone(&chrono::Utc)).num_seconds())
            })
            .unwrap_or(false)
    }
}

static TABLE: RwLock<Option<Arc<PriceTable>>> = RwLock::new(None);
// When a downloaded table was rejected. A broken upstream file has a new ETag,
// so without this every hourly check would re-download all ~3 MB of it.
static REJECTED_AT: Mutex<Option<Instant>> = Mutex::new(None);

/// The table in effect right now. Callers snapshot it once per scan so a
/// refresh landing mid-scan can't price half the entries on each table.
pub fn current() -> Option<Arc<PriceTable>> {
    TABLE.read().ok().and_then(|g| g.clone())
}

fn install(table: Arc<PriceTable>) {
    if let Ok(mut g) = TABLE.write() {
        *g = Some(table);
    }
}

fn cache_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| {
        d.join("claude-code-token-counter")
            .join("litellm_prices.json")
    })
}

fn load_at(p: &Path) -> Option<PriceTable> {
    let text = std::fs::read_to_string(p).ok()?;
    let mut t: PriceTable = serde_json::from_str(&text).ok()?;
    // The cache is a local file; apply the same bounds as a fresh download.
    t.models.retain(|_, r| r.is_valid());
    (t.models.len() >= MIN_ENTRIES).then_some(t)
}

fn save_at(p: &Path, t: &PriceTable) {
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(json) = serde_json::to_vec(t) else {
        return;
    };
    let tmp = p.with_extension("json.tmp");
    if std::fs::write(&tmp, json).is_ok() {
        let _ = std::fs::rename(&tmp, p);
    }
}

/// Load the on-disk cache into memory. Cheap; called synchronously at startup
/// so the first scan already uses remote prices.
pub fn load_cached() {
    if let Some(t) = cache_path().and_then(|p| load_at(&p)) {
        install(Arc::new(t));
    }
}

// $/token → $/MTok, rounded to 1e-9 so 2e-07 reads back as exactly 0.2.
fn per_mtok(v: &serde_json::Value, field: &str) -> Option<f64> {
    let x = (v.get(field)?.as_f64()? * 1e15).round() / 1e9;
    // Also rejects NaN.
    (0.0..=MAX_PER_MTOK).contains(&x).then_some(x)
}

/// Keep first-party Anthropic/OpenAI entries keyed by bare model id (the ids
/// Claude Code and Codex log). Provider-prefixed keys ("azure/…") and
/// Bedrock/Vertex ids are other platforms' prices.
pub fn parse_litellm(json: &serde_json::Value) -> BTreeMap<String, Rates> {
    let mut out = BTreeMap::new();
    let Some(obj) = json.as_object() else {
        return out;
    };
    for (key, v) in obj {
        if key.contains('/') {
            continue;
        }
        let provider = v.get("litellm_provider").and_then(|p| p.as_str());
        if !matches!(provider, Some("anthropic") | Some("openai")) {
            continue;
        }
        let (Some(input), Some(output)) = (
            per_mtok(v, "input_cost_per_token"),
            per_mtok(v, "output_cost_per_token"),
        ) else {
            continue;
        };
        if input == 0.0 && output == 0.0 {
            continue;
        }
        let positive = |f: &str| per_mtok(v, f).filter(|x| *x > 0.0);
        out.insert(
            key.to_lowercase(),
            Rates {
                input,
                output,
                cache_read: positive("cache_read_input_token_cost"),
                cache_write: positive("cache_creation_input_token_cost"),
                cache_write_1h: positive("cache_creation_input_token_cost_above_1hr"),
            },
        );
    }
    out
}

/// Fetch a new table if the current one is older than a day. Returns
/// `Ok(Some((previous, new)))` when the rates were replaced, `Ok(None)` when
/// nothing changed or no check was due. On any failure the previous table
/// stays in effect and the next hourly call retries.
#[allow(clippy::type_complexity)]
pub fn refresh_if_stale() -> Result<Option<(Option<Arc<PriceTable>>, Arc<PriceTable>)>, String> {
    let now = chrono::Utc::now();
    let previous = current();
    if previous.as_ref().is_some_and(|t| t.is_fresh(now)) {
        return Ok(None);
    }
    let backing_off = REJECTED_AT
        .lock()
        .ok()
        .and_then(|g| *g)
        .is_some_and(|at| at.elapsed() < Duration::from_secs(MAX_AGE_SECS as u64));
    if backing_off {
        return Ok(None);
    }
    let path = cache_path().ok_or("no app data dir")?;

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .user_agent("TokenCounter/0.2 (+price-table refresh)")
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let mut req = client.get(SOURCE_URL);
    if let Some(etag) = previous.as_ref().and_then(|t| t.etag.as_deref()) {
        req = req.header(reqwest::header::IF_NONE_MATCH, etag);
    }
    let resp = req.send().map_err(|e| format!("fetch: {e}"))?;

    if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
        if let Some(prev) = previous {
            let mut t = (*prev).clone();
            t.fetched_at = now.to_rfc3339();
            save_at(&path, &t);
            install(Arc::new(t));
        }
        return Ok(None);
    }
    if !resp.status().is_success() {
        return Err(format!("fetch: HTTP {}", resp.status()));
    }
    let etag = resp
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let body: serde_json::Value = resp.json().map_err(|e| format!("parse: {e}"))?;
    let models = parse_litellm(&body);
    if models.len() < MIN_ENTRIES {
        if let Ok(mut g) = REJECTED_AT.lock() {
            *g = Some(Instant::now());
        }
        return Err(format!(
            "parse: only {} usable models, keeping the previous table",
            models.len()
        ));
    }

    let changed = previous.as_ref().is_none_or(|p| p.models != models);
    let table = Arc::new(PriceTable {
        fetched_at: now.to_rfc3339(),
        etag,
        models,
    });
    save_at(&path, &table);
    install(table.clone());
    Ok(changed.then_some((previous, table)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_first_party_entries_only() {
        let json = serde_json::json!({
            "sample_spec": { "litellm_provider": "one of …", "input_cost_per_token": 0.0 },
            "claude-opus-5-5": {
                "litellm_provider": "anthropic",
                "input_cost_per_token": 4e-06,
                "output_cost_per_token": 2e-05,
                "cache_read_input_token_cost": 2e-07,
                "cache_creation_input_token_cost": 5e-06,
                "cache_creation_input_token_cost_above_1hr": 8e-06
            },
            "gpt-6-astra": {
                "litellm_provider": "openai",
                "input_cost_per_token": 1e-05,
                "output_cost_per_token": 5e-05,
                "cache_read_input_token_cost": 1e-06
            },
            "azure/gpt-6-astra": {
                "litellm_provider": "azure",
                "input_cost_per_token": 1.1e-05,
                "output_cost_per_token": 5.5e-05
            },
            "anthropic.claude-opus-5-5": {
                "litellm_provider": "bedrock_converse",
                "input_cost_per_token": 4e-06,
                "output_cost_per_token": 2e-05
            },
            "gpt-broken": {
                "litellm_provider": "openai",
                "input_cost_per_token": -1.0,
                "output_cost_per_token": 5e-05
            },
            "gpt-no-output": { "litellm_provider": "openai", "input_cost_per_token": 1e-06 },
            "text-embedding-free": {
                "litellm_provider": "openai",
                "input_cost_per_token": 0.0,
                "output_cost_per_token": 0.0
            }
        });
        let m = parse_litellm(&json);
        assert_eq!(
            m.keys().collect::<Vec<_>>(),
            vec!["claude-opus-5-5", "gpt-6-astra"]
        );
        let o = m["claude-opus-5-5"];
        assert_eq!((o.input, o.output), (4.0, 20.0));
        assert_eq!(o.cache_read, Some(0.2));
        assert_eq!((o.cache_write, o.cache_write_1h), (Some(5.0), Some(8.0)));
        let a = m["gpt-6-astra"];
        assert_eq!((a.input, a.cache_read, a.output), (10.0, Some(1.0), 50.0));
        assert_eq!((a.cache_write, a.cache_write_1h), (None, None));
    }

    #[test]
    fn cache_file_roundtrips_and_rejects_tiny_tables() {
        let dir = std::env::temp_dir().join("cctc_test_prices");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("litellm_prices.json");

        let mut t = PriceTable {
            fetched_at: "2026-10-16T00:00:00Z".into(),
            etag: Some("\"abc\"".into()),
            models: BTreeMap::new(),
        };
        t.models.insert(
            "gpt-6-astra".into(),
            Rates {
                input: 10.0,
                output: 50.0,
                cache_read: Some(1.0),
                cache_write: None,
                cache_write_1h: None,
            },
        );
        save_at(&path, &t);
        assert!(load_at(&path).is_none(), "1 model is below MIN_ENTRIES");

        for i in 0..MIN_ENTRIES {
            t.models
                .insert(format!("gpt-test-{i}"), t.models["gpt-6-astra"]);
        }
        save_at(&path, &t);
        let back = load_at(&path).expect("full table loads");
        assert_eq!(back.models, t.models);

        // A hand-edited or corrupted row is dropped on load, not installed.
        let mut bad = t.clone();
        bad.models.get_mut("gpt-6-astra").unwrap().input = -4.0;
        save_at(&path, &bad);
        let back = load_at(&path).expect("other rows still load");
        assert!(!back.models.contains_key("gpt-6-astra"));
        assert_eq!(back.etag, t.etag);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn freshness_window_is_one_day() {
        let t = PriceTable {
            fetched_at: "2026-10-16T00:00:00Z".into(),
            ..Default::default()
        };
        let at = |s: &str| {
            chrono::DateTime::parse_from_rfc3339(s)
                .unwrap()
                .with_timezone(&chrono::Utc)
        };
        assert!(t.is_fresh(at("2026-10-16T23:59:00Z")));
        assert!(!t.is_fresh(at("2026-10-17T00:00:01Z")));
        let bad = PriceTable::default();
        assert!(!bad.is_fresh(at("2026-10-16T00:00:00Z")));
        // Written while the clock was ahead: stale, not fresh for a year.
        let future = PriceTable {
            fetched_at: "2027-01-01T00:00:00Z".into(),
            ..Default::default()
        };
        assert!(!future.is_fresh(at("2026-10-16T00:00:00Z")));
    }

    // Real-network smoke: `cargo test smoke_fetch_litellm -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn smoke_fetch_litellm() {
        let body: serde_json::Value = reqwest::blocking::get(SOURCE_URL).unwrap().json().unwrap();
        let m = parse_litellm(&body);
        eprintln!("{} usable models", m.len());
        assert!(m.len() >= MIN_ENTRIES);
        assert_eq!(m["claude-opus-5-5"].input, 4.0);
        assert_eq!(m["gpt-6-astra"].output, 50.0);
    }
}
