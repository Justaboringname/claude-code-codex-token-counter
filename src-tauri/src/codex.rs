// Codex (OpenAI) usage parsing — reads `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`.
//
// Rollout files are event streams. The rows we care about:
//   * `turn_context`  — carries `payload.model` (e.g. "gpt-5.5"); token_count
//                       events do NOT name a model, so we attribute each one to
//                       the most recent turn_context seen in file order.
//   * `event_msg` with `payload.type == "token_count"` — one per API call, with
//     `info.last_token_usage` (that call's tokens) and `info.total_token_usage`
//     (cumulative for the process). Also carries the OFFICIAL `rate_limits`
//     snapshot (5h + weekly used_percent) — ground truth we surface directly,
//     unlike the Claude side where quota is estimated.
//   * `response_item` with `payload.type` in {function_call, custom_tool_call,
//     local_shell_call, web_search_call} — tool calls for the tools grid.
//
// Accounting rule (validated against this user's full 105-file history —
// exact match with the final cumulative total on all 89 monotone files):
//   * skip a token_count whose (total, last) pair is identical to the
//     immediately preceding one — older Codex builds emit every event twice,
//     and a naive sum nearly doubles the real figure (258M vs 136.8M);
//   * otherwise sum `last_token_usage`. This also survives mid-file counter
//     resets (process restart appending to the same rollout) where the final
//     `total_token_usage` alone would undercount.
// New rollout files never carry a prior session's total into their first
// event (0 carryover observed), so summing per-file cannot double count
// across resumes.
//
// OpenAI convention: `input_tokens` INCLUDES `cached_input_tokens`, and
// `output_tokens` includes reasoning tokens. We split input into
// uncached (billed full) + cached (billed ~10%) and map them onto the
// existing ModelUsage shape as `in` + `cacheRead`; Codex has no
// cache-write concept, so `cacheWrite` stays 0.

use chrono::{DateTime, Utc};
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

pub struct CodexEntry {
    pub ts: Option<DateTime<Utc>>,
    pub model: String,
    /// Uncached input, k-tokens (input_tokens − cached_input_tokens).
    pub in_k: f64,
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

/// Official rolling-window quota from the newest token_count event.
/// `used_percent` is only current as of `as_of`; if `resets_at` has passed,
/// the window has rolled over with no Codex activity since — the frontend
/// should render 0% rather than the stale figure.
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

fn sessions_root() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".codex").join("sessions"))
}

fn list_rollout_files(root: &Path) -> Vec<PathBuf> {
    let pattern = format!("{}/**/*.jsonl", root.display());
    glob::glob(&pattern)
        .map(|it| it.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

fn parse_ts(v: &Value) -> Option<DateTime<Utc>> {
    v.get("timestamp")
        .and_then(|t| t.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
}

fn u64_of(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(|x| x.as_u64()).unwrap_or(0)
}

// Collapse Codex's several shell-tool spellings (they changed across CLI
// versions: shell → shell_command → exec_command) into one grid tile.
fn canonical_tool(name: &str) -> String {
    match name {
        "shell" | "shell_command" | "exec_command" | "local_shell" | "run" => "shell".into(),
        other => other.to_string(),
    }
}

pub fn collect() -> CodexData {
    match sessions_root() {
        Some(root) if root.exists() => collect_from(&root),
        _ => CodexData::default(),
    }
}

pub fn collect_from(root: &Path) -> CodexData {
    let mut data = CodexData::default();
    // Newest rate_limits snapshot wins, judged by event timestamp.
    let mut quota_ts: Option<DateTime<Utc>> = None;
    // (end ts, session id, start ts) of the most recent session.
    let mut latest: Option<(DateTime<Utc>, String, DateTime<Utc>)> = None;

    for path in list_rollout_files(root) {
        let Ok(file) = File::open(&path) else {
            continue;
        };
        let reader = BufReader::new(file);

        let mut session_id: Option<String> = None;
        // Model attribution: token_count events don't carry the model; the
        // preceding turn_context does. "codex" = never saw one (1 legacy file).
        let mut cur_model = String::from("codex");
        // Previous (total, last) pair for the duplicate-emission skip.
        let mut prev_usage: Option<(Value, Value)> = None;
        let mut span: Option<(DateTime<Utc>, DateTime<Utc>)> = None;

        for line in reader.lines() {
            let Ok(line) = line else { continue };
            if line.trim().is_empty() {
                continue;
            }
            let Ok(row) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let ty = row.get("type").and_then(|t| t.as_str()).unwrap_or("");
            let Some(payload) = row.get("payload") else {
                continue;
            };

            match ty {
                "session_meta" => {
                    session_id = payload
                        .get("id")
                        .or_else(|| payload.get("session_id"))
                        .and_then(|v| v.as_str())
                        .map(String::from);
                }
                "turn_context" => {
                    if let Some(m) = payload.get("model").and_then(|v| v.as_str()) {
                        cur_model = m.to_string();
                    }
                }
                "response_item" => {
                    let pty = payload.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    let name = match pty {
                        "function_call" | "custom_tool_call" => payload
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("shell")
                            .to_string(),
                        "local_shell_call" => "shell".to_string(),
                        "web_search_call" => "web_search".to_string(),
                        _ => continue,
                    };
                    data.tools.push(CodexToolEvent {
                        ts: parse_ts(&row),
                        name: canonical_tool(&name),
                        session_id: session_id.clone(),
                    });
                }
                "event_msg" => {
                    if payload.get("type").and_then(|v| v.as_str()) != Some("token_count") {
                        continue;
                    }
                    // Official quota snapshot — newest event across all files wins.
                    let ts = parse_ts(&row);
                    if let Some(rl) = row
                        .get("payload")
                        .and_then(|p| p.get("rate_limits"))
                        .filter(|rl| !rl.is_null())
                    {
                        if let Some(t) = ts {
                            if quota_ts.is_none_or(|old| t > old) {
                                if let Some(q) = parse_quota(rl, &t) {
                                    data.quota = Some(q);
                                    quota_ts = Some(t);
                                }
                            }
                        }
                    }

                    let Some(info) = payload.get("info").filter(|i| !i.is_null()) else {
                        continue; // old builds emit token_count with info=null
                    };
                    let (Some(total), Some(last)) =
                        (info.get("total_token_usage"), info.get("last_token_usage"))
                    else {
                        continue;
                    };
                    // Duplicate emission: identical (total, last) back to back.
                    if prev_usage
                        .as_ref()
                        .is_some_and(|(pt, pl)| pt == total && pl == last)
                    {
                        continue;
                    }
                    prev_usage = Some((total.clone(), last.clone()));

                    let input = u64_of(last, "input_tokens");
                    let cached = u64_of(last, "cached_input_tokens").min(input);
                    let output = u64_of(last, "output_tokens");
                    if input == 0 && output == 0 {
                        continue;
                    }
                    if let Some(t) = ts {
                        span = Some(match span {
                            Some((s, e)) => (s.min(t), e.max(t)),
                            None => (t, t),
                        });
                    }
                    data.entries.push(CodexEntry {
                        ts,
                        model: cur_model.clone(),
                        in_k: (input - cached) as f64 / 1000.0,
                        cr_k: cached as f64 / 1000.0,
                        out_k: output as f64 / 1000.0,
                        session_id: session_id.clone(),
                    });
                }
                _ => {}
            }
        }

        if let (Some(sid), Some((s, e))) = (session_id, span) {
            match &latest {
                Some((end, _, _)) if *end >= e => {}
                _ => latest = Some((e, sid, s)),
            }
        }
    }

    if let Some(quota) = data.quota.as_mut() {
        expire_quota(quota, Utc::now().timestamp());
    }

    data.latest_session = latest.map(|(e, sid, s)| (sid, s, e));
    data
}

fn parse_quota(rl: &Value, ts: &DateTime<Utc>) -> Option<CodexQuota> {
    let primary = rl.get("primary")?;
    let secondary = rl.get("secondary");
    let window = |v: &Value| {
        v.get("window_minutes")
            .and_then(|x| x.as_u64())
            .unwrap_or(0)
    };
    let pct = |v: &Value| {
        v.get("used_percent")
            .and_then(|x| x.as_f64())
            .unwrap_or(0.0)
    };
    let resets = |v: &Value| {
        v.get("resets_at").and_then(|x| x.as_i64()).or_else(|| {
            // Older Codex rollouts stored a duration relative to the
            // token_count timestamp instead of an absolute epoch.
            v.get("resets_in_seconds")
                .and_then(|x| x.as_i64())
                .map(|seconds| ts.timestamp().saturating_add(seconds.max(0)))
        })
    };
    Some(CodexQuota {
        as_of: ts.to_rfc3339(),
        primary_used_pct: pct(primary),
        primary_window_minutes: window(primary),
        primary_resets_at: resets(primary),
        secondary_used_pct: secondary.map(pct).unwrap_or(0.0),
        secondary_window_minutes: secondary.map(window).unwrap_or(0),
        secondary_resets_at: secondary.and_then(resets),
        plan_type: rl
            .get("plan_type")
            .and_then(|v| v.as_str())
            .map(String::from),
    })
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
    use std::io::Write as _;

    // Print-style smoke over the real ~/.codex tree (like usage's
    // smoke_compute_all) — run with --nocapture and diff against an
    // independent oracle when touching the accounting rules.
    #[test]
    fn smoke_collect_real_dir() {
        let data = collect();
        let mut per_model: std::collections::BTreeMap<String, (f64, f64, f64)> =
            std::collections::BTreeMap::new();
        for e in &data.entries {
            let m = per_model.entry(e.model.clone()).or_default();
            m.0 += e.in_k;
            m.1 += e.cr_k;
            m.2 += e.out_k;
        }
        for (model, (i, c, o)) in &per_model {
            eprintln!("{model:22} in={:9.2}K cached={:9.2}K out={:8.2}K", i, c, o);
        }
        eprintln!(
            "entries={} tools={} quota={:?}",
            data.entries.len(),
            data.tools.len(),
            data.quota
                .as_ref()
                .map(|q| (q.primary_used_pct, q.secondary_used_pct))
        );
    }

    fn write_rollout(dir: &Path, name: &str, lines: &[&str]) {
        std::fs::create_dir_all(dir).unwrap();
        let mut f = File::create(dir.join(name)).unwrap();
        for l in lines {
            writeln!(f, "{l}").unwrap();
        }
    }

    fn token_count(
        ts: &str,
        total: u64,
        last_in: u64,
        last_cached: u64,
        last_out: u64,
        tot_run: u64,
    ) -> String {
        format!(
            r#"{{"timestamp":"{ts}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":{t_in},"cached_input_tokens":0,"output_tokens":0,"total_tokens":{tot_run}}},"last_token_usage":{{"input_tokens":{last_in},"cached_input_tokens":{last_cached},"output_tokens":{last_out},"total_tokens":{total}}}}},"rate_limits":{{"primary":{{"used_percent":65.0,"window_minutes":300,"resets_at":4102444800}},"secondary":{{"used_percent":19.0,"window_minutes":10080,"resets_at":4103049600}},"plan_type":"plus"}}}}}}"#,
            t_in = tot_run,
        )
    }

    #[test]
    fn dedups_double_emitted_events_and_survives_resets() {
        let dir = std::env::temp_dir().join("cctc_codex_dedup_test");
        let _ = std::fs::remove_dir_all(&dir);

        let meta = r#"{"timestamp":"2026-07-01T00:00:00Z","type":"session_meta","payload":{"id":"sess-1"}}"#;
        let turn = r#"{"timestamp":"2026-07-01T00:00:01Z","type":"turn_context","payload":{"model":"gpt-5.5"}}"#;
        let e1 = token_count("2026-07-01T00:00:02Z", 1000, 900, 400, 100, 1000);
        let e1_dup = token_count("2026-07-01T00:00:03Z", 1000, 900, 400, 100, 1000);
        let e2 = token_count("2026-07-01T00:00:04Z", 500, 400, 0, 100, 1500);
        // process restart: cumulative total resets, last is a real new call
        let e3 = token_count("2026-07-01T00:00:05Z", 300, 200, 0, 100, 300);
        write_rollout(
            &dir,
            "rollout-a.jsonl",
            &[meta, turn, &e1, &e1_dup, &e2, &e3],
        );

        let data = collect_from(&dir);
        assert_eq!(data.entries.len(), 3, "dup emission must be skipped");
        let sum_in: f64 = data.entries.iter().map(|e| e.in_k + e.cr_k).sum();
        let sum_out: f64 = data.entries.iter().map(|e| e.out_k).sum();
        assert!((sum_in - (900.0 + 400.0 + 200.0) / 1000.0).abs() < 1e-9);
        assert!((sum_out - 0.3).abs() < 1e-9);
        // cached is split out of input, not added on top
        let e = &data.entries[0];
        assert!((e.in_k - 0.5).abs() < 1e-9, "in = input - cached");
        assert!((e.cr_k - 0.4).abs() < 1e-9);
        assert_eq!(e.model, "gpt-5.5");
        assert_eq!(e.session_id.as_deref(), Some("sess-1"));

        // quota snapshot picked up
        let q = data.quota.expect("quota");
        assert_eq!(q.primary_used_pct, 65.0);
        assert_eq!(q.primary_window_minutes, 300);
        assert_eq!(q.plan_type.as_deref(), Some("plus"));

        // latest session span
        let (sid, s, e) = data.latest_session.expect("latest session");
        assert_eq!(sid, "sess-1");
        assert!(s < e);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tool_calls_are_canonicalized() {
        let dir = std::env::temp_dir().join("cctc_codex_tools_test");
        let _ = std::fs::remove_dir_all(&dir);
        let meta = r#"{"timestamp":"2026-07-01T00:00:00Z","type":"session_meta","payload":{"id":"sess-2"}}"#;
        let fc = r#"{"timestamp":"2026-07-01T00:00:01Z","type":"response_item","payload":{"type":"function_call","name":"exec_command"}}"#;
        let ct = r#"{"timestamp":"2026-07-01T00:00:02Z","type":"response_item","payload":{"type":"custom_tool_call","name":"apply_patch"}}"#;
        let ws = r#"{"timestamp":"2026-07-01T00:00:03Z","type":"response_item","payload":{"type":"web_search_call"}}"#;
        let out = r#"{"timestamp":"2026-07-01T00:00:04Z","type":"response_item","payload":{"type":"function_call_output","name":"exec_command"}}"#;
        write_rollout(&dir, "rollout-b.jsonl", &[meta, fc, ct, ws, out]);

        let data = collect_from(&dir);
        let names: Vec<&str> = data.tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["shell", "apply_patch", "web_search"],
            "outputs must not count as calls"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_quota_schema_gets_absolute_reset_times() {
        let ts = DateTime::parse_from_rfc3339("2026-07-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let rl: Value = serde_json::from_str(
            r#"{
                "primary":{"used_percent":3,"window_minutes":300,"resets_in_seconds":60},
                "secondary":{"used_percent":15,"window_minutes":10080,"resets_in_seconds":120}
            }"#,
        )
        .unwrap();

        let q = parse_quota(&rl, &ts).expect("quota");
        assert_eq!(q.primary_resets_at, Some(ts.timestamp() + 60));
        assert_eq!(q.secondary_resets_at, Some(ts.timestamp() + 120));
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
}
