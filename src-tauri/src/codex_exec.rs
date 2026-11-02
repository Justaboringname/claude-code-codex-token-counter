// Usage from saved `codex exec --json` output streams. A `codex exec
// --ephemeral` run writes no rollout, so its tokens never reach CODEX_HOME;
// when a script keeps the JSON event stream (one file per run), that file is
// the only record. The directories to read are listed in `codex_exec_logs.json`
// next to daily_usage.json (absent = nothing to read):
//
//   {"sources": [{"dir": "/path/to/logs", "model": "gpt-6-astra"}]}
//
// Each `*.jsonl` directly in `dir` is one run. The stream names neither the
// model nor a time, so:
//   * `model` (required) is the billing id to charge — the Codex config
//     default the runs used, plus a tier suffix such as "-fast" if they ran on
//     one. Keys are checked: a missing or misspelled `model` rejects the file
//     rather than billing the runs at some fallback rate.
//   * the timestamp is the file name's leading `YYYYMMDDTHHMMSS` (UTC, as
//     `datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S")` writes it), else
//     the file's mtime.
// `turn.completed.usage` covers the whole turn, summed over its requests, so
// the per-request long-context (>272K) surcharge can't be applied, and a
// `turn.failed` run (e.g. stopped by the usage limit) reports no usage at all —
// both undercount. A run whose thread also has a rollout is skipped: the
// rollout is exact. Runs feed by-model, the store, the sparkline and the
// tools grid, but never the "current session".
//
// `CodexData::exec_sources` fingerprints the sources read ("" when none are
// configured); usage.rs re-derives the stored Codex days when it changes, so
// editing a source's `model` replaces the old id instead of adding a second
// copy. It is None when the file or a listed directory can't be read: such a
// scan may be missing exec usage the store still has, so it must not drive a
// re-derivation.

use crate::codex::{canonical_tool, make_entry, CodexData, CodexToolEvent, Tokens};
use chrono::{DateTime, NaiveDateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

const CONFIG_FILE: &str = "codex_exec_logs.json";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub dir: PathBuf,
    pub model: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    #[serde(default)]
    sources: Vec<Source>,
}

fn config_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("claude-code-token-counter").join(CONFIG_FILE))
}

fn parse_config(text: &str) -> Result<Vec<Source>, String> {
    serde_json::from_str::<Config>(text)
        .map(|c| c.sources)
        .map_err(|e| format!("{CONFIG_FILE} is not valid: {e}"))
}

/// The configured sources; no file means none. Err when it exists but can't
/// be used.
fn load_sources() -> Result<Vec<Source>, String> {
    let Some(path) = config_path() else {
        return Ok(Vec::new());
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => parse_config(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("{CONFIG_FILE} unreadable: {e}")),
    }
}

/// What one run's file contributes.
#[derive(Default)]
struct ExecRun {
    thread_id: Option<String>,
    ts: Option<DateTime<Utc>>,
    usage: Tokens,
    tools: Vec<String>,
}

fn ts_from_name(path: &Path) -> Option<DateTime<Utc>> {
    let name = path.file_name()?.to_str()?;
    let stamp = name.get(..15)?;
    NaiveDateTime::parse_from_str(stamp, "%Y%m%dT%H%M%S")
        .ok()
        .map(|t| t.and_utc())
}

fn tool_of(item: &Value) -> Option<String> {
    let name = match item.get("type").and_then(|t| t.as_str())? {
        "command_execution" => "shell",
        "file_change" => "apply_patch",
        "web_search" => "web_search",
        "mcp_tool_call" => item.get("tool").and_then(|t| t.as_str()).unwrap_or("mcp"),
        _ => return None,
    };
    Some(canonical_tool(name))
}

fn scan_run(path: &Path, modified: Option<SystemTime>) -> std::io::Result<ExecRun> {
    let mut run = ExecRun {
        ts: ts_from_name(path).or_else(|| modified.map(DateTime::<Utc>::from)),
        ..Default::default()
    };
    let reader = BufReader::new(std::fs::File::open(path)?);
    for bytes in reader.split(b'\n') {
        // A read error ends the file; a bad line is skipped.
        let Ok(bytes) = bytes else { break };
        let Ok(line) = std::str::from_utf8(&bytes) else {
            continue;
        };
        // Most bytes are item output; only three event types matter.
        if ![
            r#""type":"thread.started""#,
            r#""type":"turn.completed""#,
            r#""type":"item.completed""#,
        ]
        .iter()
        .any(|t| line.contains(t))
        {
            continue;
        }
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match event.get("type").and_then(|t| t.as_str()) {
            Some("thread.started") => {
                if let Some(id) = event.get("thread_id").and_then(|t| t.as_str()) {
                    run.thread_id.get_or_insert_with(|| id.to_string());
                }
            }
            Some("turn.completed") => {
                if let Some(u) = event.get("usage") {
                    run.usage = run.usage.plus(&Tokens::of(u));
                }
            }
            Some("item.completed") => {
                if let Some(name) = event.get("item").and_then(tool_of) {
                    run.tools.push(name);
                }
            }
            _ => {}
        }
    }
    Ok(run)
}

struct CachedRun {
    len: u64,
    modified: Option<SystemTime>,
    run: Arc<ExecRun>,
}

// A finished run's file never changes, so it is parsed once; one still being
// written is re-read when its size or mtime moves.
static RUN_CACHE: Mutex<BTreeMap<PathBuf, CachedRun>> = Mutex::new(BTreeMap::new());

fn scan_cached(path: &Path, cache: &mut BTreeMap<PathBuf, CachedRun>) -> Option<Arc<ExecRun>> {
    let meta = std::fs::metadata(path).ok()?;
    let (len, modified) = (meta.len(), meta.modified().ok());
    if let Some(c) = cache.get(path) {
        if c.len == len && c.modified == modified {
            return Some(Arc::clone(&c.run));
        }
    }
    let run = Arc::new(scan_run(path, modified).ok()?);
    cache.insert(
        path.to_path_buf(),
        CachedRun {
            len,
            modified,
            run: Arc::clone(&run),
        },
    );
    Some(run)
}

/// The run files in `dir`, or None if it can't be listed.
fn list_runs(dir: &Path) -> Option<Vec<PathBuf>> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "jsonl"))
        .collect();
    out.sort();
    Some(out)
}

/// Add the configured exec runs to `data`, skipping threads that have a rollout.
pub fn append(data: &mut CodexData, rollout_threads: &HashSet<&str>) {
    append_from(data, load_sources(), rollout_threads);
}

fn append_from(
    data: &mut CodexData,
    sources: Result<Vec<Source>, String>,
    rollout_threads: &HashSet<&str>,
) {
    let sources = sources.unwrap_or_else(|e| {
        eprintln!("{e}");
        data.exec_sources = None;
        Vec::new()
    });
    let mut cache = RUN_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let mut live: HashSet<PathBuf> = HashSet::new();
    let mut seen_threads: HashSet<String> = HashSet::new();
    let mut all_read = true;
    for source in &sources {
        let model = source.model.as_str();
        let Some(paths) = list_runs(&source.dir) else {
            all_read = false;
            continue;
        };
        for path in paths {
            // The same directory listed twice must not count twice.
            if !live.insert(path.clone()) {
                continue;
            }
            let Some(run) = scan_cached(&path, &mut cache) else {
                continue;
            };
            if let Some(t) = run.thread_id.as_deref() {
                if rollout_threads.contains(t) || !seen_threads.insert(t.to_string()) {
                    continue;
                }
            }
            // No `-long`: the turn total spans many requests.
            if let Some(entry) = make_entry(run.ts, model.to_string(), &run.usage, &run.thread_id) {
                data.entries.push(entry);
            }
            data.tools
                .extend(run.tools.iter().map(|name| CodexToolEvent {
                    ts: run.ts,
                    name: name.clone(),
                    session_id: run.thread_id.clone(),
                }));
        }
    }
    cache.retain(|p, _| live.contains(p));
    if !all_read {
        data.exec_sources = None;
    } else if data.exec_sources.is_some() {
        let mut keys: Vec<String> = sources
            .iter()
            .map(|s| format!("{}={}", s.dir.display(), s.model))
            .collect();
        keys.sort();
        data.exec_sources = Some(keys.join("\n"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write as _;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cctc_exec_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_run(dir: &Path, name: &str, events: &[Value]) -> PathBuf {
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        for e in events {
            writeln!(f, "{e}").unwrap();
        }
        path
    }

    fn completed(input: i64, cached: i64, output: i64) -> Value {
        json!({"type": "turn.completed", "usage": {
            "input_tokens": input, "cached_input_tokens": cached,
            "cache_write_input_tokens": 0, "output_tokens": output,
            "reasoning_output_tokens": 0,
        }})
    }

    fn item(ty: &str) -> Value {
        json!({"type": "item.completed", "item": {"id": "i", "type": ty, "status": "completed"}})
    }

    fn source(dir: &Path, model: &str) -> Source {
        Source {
            dir: dir.to_path_buf(),
            model: model.to_string(),
        }
    }

    fn run_config(sources: Result<Vec<Source>, String>, rollouts: &[&str]) -> CodexData {
        let mut data = CodexData {
            exec_sources: Some(String::new()),
            ..Default::default()
        };
        append_from(&mut data, sources, &rollouts.iter().copied().collect());
        data
    }

    fn run_sources(sources: &[Source], rollouts: &[&str]) -> CodexData {
        run_config(Ok(sources.to_vec()), rollouts)
    }

    #[test]
    fn completed_run_counts_with_configured_model_and_name_time() {
        let dir = temp_dir("completed");
        write_run(
            &dir,
            "20261021T024838-54951.jsonl",
            &[
                json!({"type": "thread.started", "thread_id": "t1"}),
                json!({"type": "turn.started"}),
                item("command_execution"),
                item("file_change"),
                item("web_search"),
                item("agent_message"),
                // The whole turn: far above 272K but not a single request.
                completed(1_200_000, 1_100_000, 15_000),
            ],
        );
        let data = run_sources(&[source(&dir, "gpt-6-astra")], &[]);
        assert_eq!(data.entries.len(), 1);
        let e = &data.entries[0];
        assert_eq!(e.model, "gpt-6-astra");
        assert_eq!(e.ts.unwrap().to_rfc3339(), "2026-10-21T02:48:38+00:00");
        assert_eq!(
            (e.in_k, e.cr_k, e.cw_k, e.out_k),
            (100.0, 1100.0, 0.0, 15.0)
        );
        assert_eq!(e.session_id.as_deref(), Some("t1"));
        let tools: Vec<_> = data.tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(tools, ["shell", "apply_patch", "web_search"]);
        assert!(data.latest_session.is_none());
    }

    #[test]
    fn failed_run_has_no_usage_and_unnamed_file_uses_mtime() {
        let dir = temp_dir("failed");
        write_run(
            &dir,
            "run.jsonl",
            &[
                json!({"type": "thread.started", "thread_id": "t2"}),
                item("command_execution"),
                json!({"type": "turn.failed", "error": {"message": "You've hit your usage limit."}}),
            ],
        );
        let data = run_sources(&[source(&dir, "gpt-6-astra")], &[]);
        assert!(data.entries.is_empty());
        assert_eq!(data.tools.len(), 1);
        assert!(data.tools[0].ts.is_some());
    }

    #[test]
    fn rollout_threads_duplicate_dirs_and_other_files_are_skipped() {
        let dir = temp_dir("dedup");
        let run = |id: &str| {
            [
                json!({"type": "thread.started", "thread_id": id}),
                completed(2_000, 1_000, 100),
            ]
        };
        write_run(&dir, "20261020T010000-1.jsonl", &run("has-rollout"));
        write_run(&dir, "20261020T020000-2.jsonl", &run("exec-only"));
        write_run(&dir, "20261020T020000-2.stderr.log", &run("not-a-run"));
        let s = source(&dir, "gpt-6-astra");
        let data = run_sources(&[s.clone(), s], &["has-rollout"]);
        let ids: Vec<_> = data
            .entries
            .iter()
            .map(|e| e.session_id.clone().unwrap())
            .collect();
        assert_eq!(ids, ["exec-only"]);
        assert_eq!(data.entries[0].model, "gpt-6-astra");
    }

    #[test]
    fn config_requires_model_and_rejects_unknown_keys() {
        let sources = parse_config(
            r#"{"sources": [{"dir": "/b", "model": "gpt-6-astra"}, {"dir": "/a", "model": "gpt-6-astra-fast"}]}"#,
        )
        .unwrap();
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[1].model, "gpt-6-astra-fast");
        assert!(parse_config("{}").unwrap().is_empty());
        assert!(parse_config("not json").is_err());
        // A missing model would otherwise bill as legacy "codex" at GPT-5 rates.
        assert!(parse_config(r#"{"sources": [{"dir": "/b"}]}"#).is_err());
        assert!(parse_config(r#"{"sources": [{"dir": "/b", "modle": "gpt-6-astra"}]}"#).is_err());
    }

    #[test]
    fn fingerprint_names_sources_and_is_none_when_unreadable() {
        let dir = temp_dir("fingerprint");
        write_run(&dir, "20261020T010000-1.jsonl", &[completed(10, 0, 1)]);
        let a = source(&dir, "gpt-6-astra");
        let b = source(&dir.join("other"), "gpt-6-astra-fast");
        std::fs::create_dir_all(&b.dir).unwrap();
        let fp = run_sources(&[b.clone(), a.clone()], &[])
            .exec_sources
            .unwrap();
        assert_eq!(fp, run_sources(&[a.clone(), b], &[]).exec_sources.unwrap());
        let mut lines: Vec<_> = fp.lines().map(|l| l.rsplit('=').next().unwrap()).collect();
        lines.sort();
        assert_eq!(lines, ["gpt-6-astra", "gpt-6-astra-fast"]);
        assert_eq!(run_sources(&[], &[]).exec_sources.as_deref(), Some(""));

        // An unreadable dir or config: count what can be read, but flag the scan.
        let missing = run_sources(&[a.clone(), source(&dir.join("gone"), "x")], &[]);
        assert_eq!(missing.entries.len(), 1);
        assert!(missing.exec_sources.is_none());
        let bad = run_config(Err("bad".into()), &[]);
        assert!(bad.entries.is_empty() && bad.exec_sources.is_none());
    }

    #[test]
    fn a_run_still_being_written_is_reread() {
        let dir = temp_dir("growing");
        let path = write_run(
            &dir,
            "20261020T010000-1.jsonl",
            &[json!({"type": "thread.started", "thread_id": "g"})],
        );
        let s = [source(&dir, "gpt-6-astra")];
        assert!(run_sources(&s, &[]).entries.is_empty());
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(f, "{}", completed(5_000, 0, 50)).unwrap();
        drop(f);
        let data = run_sources(&s, &[]);
        assert_eq!(data.entries.len(), 1);
        assert_eq!(data.entries[0].in_k, 5.0);
    }
}
