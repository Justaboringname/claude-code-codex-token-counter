mod claude_limits;
mod claude_refresh;
mod codex;
mod codex_exec;
mod codex_limits;
mod prices;
mod sync;
mod tray;
mod usage;

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;
use tauri::{Emitter, Manager};

// Serialize the heavy JSONL rescan. A burst of refresh triggers (the 15s poll,
// file-watcher events that fire repeatedly while Claude Code is writing, and
// manual refresh) would otherwise launch several hundred-MB scans on the
// blocking pool at once. Overlapping scans read the live-appended files at
// different points and finish out of order — that out-of-order resolution is
// what made the headline number flicker ($99 → $94 → $99).
// sync.rs's publisher takes it per range and releases it with `unlock_fair`,
// which hands it straight to a parked `get_usage`, so the UI's own scans
// interleave with the publisher's ranges. (parking_lot, not std: std's Mutex
// on macOS lets the releasing thread re-lock first.)
pub(crate) static COMPUTE_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

// An official-limits snapshot (Claude or Codex) as the quota cache sees it.
trait QuotaSnapshot: Clone + serde::Serialize + serde::de::DeserializeOwned {
    fn status(&mut self) -> &mut String;
    fn message(&mut self) -> &mut Option<String>;
    fn fetched_at(&self) -> &str;
    /// A failed fetch that was the endpoint's rate limit (HTTP 429).
    fn rate_limited(&self) -> bool;
}

impl QuotaSnapshot for claude_limits::ClaudeQuota {
    fn status(&mut self) -> &mut String {
        &mut self.status
    }
    fn message(&mut self) -> &mut Option<String> {
        &mut self.message
    }
    fn fetched_at(&self) -> &str {
        &self.fetched_at
    }
    fn rate_limited(&self) -> bool {
        self.message.as_deref() == Some(claude_limits::RATE_LIMITED)
    }
}

impl QuotaSnapshot for codex_limits::CodexLiveQuota {
    fn status(&mut self) -> &mut String {
        &mut self.status
    }
    fn message(&mut self) -> &mut Option<String> {
        &mut self.message
    }
    fn fetched_at(&self) -> &str {
        &self.fetched_at
    }
    fn rate_limited(&self) -> bool {
        self.message.as_deref() == Some(codex_limits::RATE_LIMITED)
    }
}

// How often a non-forced call may hit the network. Both the popover (every
// 60s) and the sync publisher (every 60s, around the clock) ask for the quota;
// at one fetch a minute Anthropic's usage endpoint answered 200 and 429 in
// turn (measured 2026-11-09), so the row flipped ok/stale every minute and the
// publisher uploaded each flip. The endpoint is shared with Claude Code's
// /usage and Claude Usage Monitor, so the background rate also matters to them.
const QUOTA_REFRESH: Duration = Duration::from_secs(180);
// After consecutive 429s the next try waits QUOTA_REFRESH × 2^n, up to this.
const QUOTA_BACKOFF_CAP: Duration = Duration::from_secs(600);
// A rate-limited refresh keeps showing the last good reading as current while
// it is at most this old (longer than the backoff cap, so a lone 429 never
// dims the row); past that it is shown as stale like any other failure.
const QUOTA_RATE_LIMIT_GRACE_SECS: i64 = 15 * 60;

struct QuotaCache<T> {
    /// The last good ("ok") snapshot, also persisted to disk.
    value: Option<T>,
    /// What the last call returned; served unchanged until `wait` has passed.
    shown: Option<T>,
    checked_at: Option<Instant>,
    wait: Duration,
    rate_limited_in_a_row: u32,
}

impl<T> QuotaCache<T> {
    const fn new() -> Self {
        Self {
            value: None,
            shown: None,
            checked_at: None,
            wait: QUOTA_REFRESH,
            rate_limited_in_a_row: 0,
        }
    }
}

fn quota_backoff(rate_limited_in_a_row: u32) -> Duration {
    let factor = 1u32 << rate_limited_in_a_row.min(8);
    QUOTA_REFRESH.saturating_mul(factor).min(QUOTA_BACKOFF_CAP)
}

/// Whether a snapshot fetched at `fetched_at` is recent enough to stand in for
/// a refresh that was rate limited. Unparseable counts as too old.
fn within_rate_limit_grace(fetched_at: &str, now: chrono::DateTime<chrono::Utc>) -> bool {
    chrono::DateTime::parse_from_rfc3339(fetched_at)
        .map(|at| {
            (now - at.with_timezone(&chrono::Utc)).num_seconds() <= QUOTA_RATE_LIMIT_GRACE_SECS
        })
        .unwrap_or(false)
}

static CLAUDE_QUOTA_CACHE: Mutex<QuotaCache<claude_limits::ClaudeQuota>> =
    Mutex::new(QuotaCache::new());
static CODEX_QUOTA_CACHE: Mutex<QuotaCache<codex_limits::CodexLiveQuota>> =
    Mutex::new(QuotaCache::new());

fn quota_cache_path(file: &str) -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("claude-code-token-counter").join(file))
}

fn load_quota_cache<T: QuotaSnapshot>(file: &str) -> Option<T> {
    let text = std::fs::read_to_string(quota_cache_path(file)?).ok()?;
    serde_json::from_str(&text).ok()
}

fn save_quota_cache<T: QuotaSnapshot>(file: &str, quota: &T) {
    let Some(path) = quota_cache_path(file) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // tmp + rename: usage.rs reads claude_quota.json during a scan and must
    // never see a truncated file.
    if let Ok(json) = serde_json::to_vec(quota) {
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

// A fresh snapshot at most every QUOTA_REFRESH unless forced (longer after
// 429s); the last good one is persisted, kept as current through a brief rate
// limit, and returned as "stale" when a fetch fails otherwise.
fn cached_quota<T: QuotaSnapshot>(
    cache: &Mutex<QuotaCache<T>>,
    file: &str,
    force: bool,
    fetch: fn() -> T,
) -> T {
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    if cache.value.is_none() {
        cache.value = load_quota_cache(file);
    }
    quota_step(
        &mut cache,
        force,
        Instant::now(),
        chrono::Utc::now(),
        fetch,
        |fresh| save_quota_cache(file, fresh),
    )
}

/// cached_quota's policy with the clock, the fetch and the disk write passed
/// in, so tests can drive it.
fn quota_step<T: QuotaSnapshot>(
    cache: &mut QuotaCache<T>,
    force: bool,
    now: Instant,
    wall_now: chrono::DateTime<chrono::Utc>,
    fetch: impl FnOnce() -> T,
    save: impl FnOnce(&T),
) -> T {
    if !force
        && cache
            .checked_at
            .is_some_and(|at| now.saturating_duration_since(at) < cache.wait)
    {
        if let Some(shown) = cache.shown.clone().or_else(|| cache.value.clone()) {
            return shown;
        }
    }

    let mut fresh = fetch();
    cache.checked_at = Some(now);
    if fresh.status() == "ok" {
        save(&fresh);
        cache.value = Some(fresh.clone());
        cache.shown = Some(fresh.clone());
        cache.rate_limited_in_a_row = 0;
        cache.wait = QUOTA_REFRESH;
        return fresh;
    }

    let rate_limited = fresh.rate_limited();
    if rate_limited {
        cache.rate_limited_in_a_row = cache.rate_limited_in_a_row.saturating_add(1);
        cache.wait = quota_backoff(cache.rate_limited_in_a_row);
    } else {
        cache.rate_limited_in_a_row = 0;
        cache.wait = QUOTA_REFRESH;
    }
    let shown = match cache.value.clone() {
        Some(last) if rate_limited && within_rate_limit_grace(last.fetched_at(), wall_now) => last,
        Some(mut last) => {
            *last.status() = "stale".into();
            *last.message() = fresh.message().take();
            last
        }
        None => fresh,
    };
    cache.shown = Some(shown.clone());
    shown
}

#[tauri::command]
async fn get_usage(range: String, plan: Option<String>) -> usage::Usage {
    // This parses hundreds of MB of JSONL — heavy, blocking work. A synchronous
    // Tauri command runs on the main thread, so it would freeze the popover
    // ("Application Not Responding") for the whole parse. That's especially bad
    // because the file watcher re-fires this on every change while Claude Code
    // is active. Run it on the blocking pool so the UI thread stays responsive.
    tauri::async_runtime::spawn_blocking(move || {
        // Hold the rescan lock for the whole parse so concurrent invocations
        // queue instead of racing. parking_lot has no poisoning: a panicked
        // scan releases the lock as it unwinds, so it can't wedge every later
        // refresh.
        let _guard = COMPUTE_LOCK.lock();
        let plan_id = plan.as_deref().unwrap_or("max5x");
        usage::compute_usage_with_plan(&range, plan_id)
    })
    .await
    .unwrap_or_default()
}

// What get_claude_quota returns. Blocking (Keychain + network on a cache
// miss); the sync publisher calls it non-forced, so it shares the cache
// and never adds a fetch of its own.
pub(crate) fn claude_quota(force: bool) -> claude_limits::ClaudeQuota {
    let quota = cached_quota(
        &CLAUDE_QUOTA_CACHE,
        "claude_quota.json",
        force,
        claude_limits::fetch,
    );
    if let (true, Some(b)) = (quota.status == "ok", &quota.breakdown) {
        usage::record_week_breakdown(b);
    }
    claude_limits::with_desktop_sample(quota)
}

// What get_codex_quota returns; same cache rules as claude_quota.
pub(crate) fn codex_quota(force: bool) -> codex_limits::CodexLiveQuota {
    cached_quota(
        &CODEX_QUOTA_CACHE,
        "codex_quota.json",
        force,
        codex_limits::fetch,
    )
}

#[tauri::command]
async fn get_claude_quota(force: Option<bool>) -> claude_limits::ClaudeQuota {
    tauri::async_runtime::spawn_blocking(move || claude_quota(force.unwrap_or(false)))
        .await
        .unwrap_or_else(|e| claude_limits::ClaudeQuota {
            five_hour: None,
            weekly: None,
            status: "error".into(),
            message: Some(format!("quota task failed: {e}")),
            fetched_at: chrono::Utc::now().to_rfc3339(),
            subscription: None,
            breakdown: None,
        })
}

#[tauri::command]
async fn get_codex_quota(force: Option<bool>) -> codex_limits::CodexLiveQuota {
    tauri::async_runtime::spawn_blocking(move || codex_quota(force.unwrap_or(false)))
        .await
        .unwrap_or_else(|e| codex_limits::CodexLiveQuota {
            five_hour: None,
            weekly: None,
            status: "error".into(),
            message: Some(format!("quota task failed: {e}")),
            fetched_at: chrono::Utc::now().to_rfc3339(),
            plan_type: None,
            reset_credits: None,
        })
}

#[tauri::command]
fn set_theme(window: tauri::Window, theme: String) {
    let theme = match theme.as_str() {
        "dark" => Some(tauri::Theme::Dark),
        "light" => Some(tauri::Theme::Light),
        _ => None,
    };
    let _ = window.set_theme(theme);
}

fn spawn_file_watcher(app_handle: tauri::AppHandle) {
    // Watch both providers' transcript roots (Codex: live and archived
    // rollouts); any may be absent (user without Codex installed, or vice
    // versa) — watch whichever exists. The 15s poll covers roots created later.
    let roots: Vec<_> = usage::projects_root()
        .into_iter()
        .chain(codex::rollout_roots())
        .filter(|p| p.exists())
        .collect();
    if roots.is_empty() {
        return;
    }

    std::thread::spawn(move || {
        use notify::{RecursiveMode, Watcher};
        use notify_debouncer_full::new_debouncer;

        let (tx, rx) = std::sync::mpsc::channel();
        let mut debouncer = match new_debouncer(Duration::from_millis(500), None, tx) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("file watcher init failed: {e}");
                return;
            }
        };
        let mut watching = 0;
        for root in &roots {
            match debouncer.watcher().watch(root, RecursiveMode::Recursive) {
                Ok(()) => watching += 1,
                Err(e) => eprintln!("file watcher watch failed for {}: {e}", root.display()),
            }
        }
        if watching == 0 {
            return;
        }

        for res in rx {
            if res.is_ok() {
                let _ = app_handle.emit("usage-changed", ());
            }
        }
    });
}

// Keep the LiteLLM price table at most a day old. Checks hourly (cheap: a
// fresh table returns immediately, a stale one costs one conditional GET), so
// a failed fetch retries within the hour while the cached table stays in use.
fn spawn_price_refresher(app_handle: tauri::AppHandle) {
    std::thread::spawn(move || loop {
        match prices::refresh_if_stale() {
            Ok(Some((old, new))) => {
                let n = usage::log_price_changes(old.as_deref(), &new);
                if n > 0 {
                    eprintln!("price table refreshed: {n} stored models repriced");
                }
                let _ = app_handle.emit("usage-changed", ());
            }
            Ok(None) => {}
            Err(e) => eprintln!("price table refresh failed: {e}"),
        }
        std::thread::sleep(Duration::from_secs(3600));
    });
}

/// The panel's 展开详情 button: hide the panel, show the full window.
#[tauri::command]
fn tray_open_full(app: tauri::AppHandle) {
    tray::show_full_window(&app);
}

/// The panel's 退出 button: quit the whole app.
#[tauri::command]
fn tray_quit(app: tauri::AppHandle) {
    app.exit(0);
}

/// Menu bar title, e.g. today's spend. Empty clears it.
#[tauri::command]
fn tray_set_title(app: tauri::AppHandle, text: String) {
    let text = text.trim();
    tray::set_title(&app, (!text.is_empty()).then(|| text.to_string()));
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_liquid_glass::init())
        .on_window_event(|window, event| tray::on_window_event(window, event))
        .setup(|app| {
            // Menu bar app: no Dock icon; the full window opens from the panel.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            #[cfg(target_os = "macos")]
            {
                use tauri_plugin_liquid_glass::{
                    GlassMaterialVariant, LiquidGlassConfig, LiquidGlassExt,
                };
                // macOS 26+ Liquid Glass via the private NSGlassEffectView
                // (inserted as the window's backdrop, below the transparent
                // webview). Falls back to NSVisualEffectView pre-26. This is
                // real refractive glass — the CSS surface only styles content.
                for label in [tray::MAIN_LABEL, tray::TRAY_LABEL] {
                    let Some(window) = app.get_webview_window(label) else {
                        continue;
                    };
                    if let Err(e) = app.liquid_glass().set_effect(
                        &window,
                        LiquidGlassConfig {
                            corner_radius: 20.0,
                            variant: GlassMaterialVariant::Regular,
                            ..Default::default()
                        },
                    ) {
                        eprintln!("liquid glass apply failed: {e}");
                    }
                }
            }
            tray::install(app.handle())?;
            spawn_file_watcher(app.handle().clone());
            prices::load_cached();
            spawn_price_refresher(app.handle().clone());
            sync::spawn_publisher(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_usage,
            get_claude_quota,
            get_codex_quota,
            set_theme,
            tray_open_full,
            tray_quit,
            tray_set_title,
            sync::sync_get_config,
            sync::sync_set_config,
            sync::sync_status,
            sync::sync_publish_now
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod quota_cache_tests {
    use super::*;
    use claude_limits::{ClaudeQuota, LimitWindow};

    fn t0() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339("2026-11-09T15:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    fn good(at: chrono::DateTime<chrono::Utc>, five_hour: f64) -> ClaudeQuota {
        ClaudeQuota {
            five_hour: Some(LimitWindow {
                used_pct: five_hour,
                resets_at: Some("2026-11-09T18:00:00Z".into()),
            }),
            weekly: Some(LimitWindow {
                used_pct: 50.0,
                resets_at: Some("2026-11-11T05:00:00Z".into()),
            }),
            status: "ok".into(),
            message: None,
            fetched_at: at.to_rfc3339(),
            subscription: Some("max".into()),
            breakdown: None,
        }
    }

    fn failed(at: chrono::DateTime<chrono::Utc>, message: &str) -> ClaudeQuota {
        ClaudeQuota {
            five_hour: None,
            weekly: None,
            status: "stale".into(),
            message: Some(message.into()),
            fetched_at: at.to_rfc3339(),
            subscription: Some("max".into()),
            breakdown: None,
        }
    }

    /// One call `secs` after the start, answering with `reply` if it fetches.
    /// Returns what the cache handed back and whether it fetched.
    fn call(
        cache: &mut QuotaCache<ClaudeQuota>,
        start: Instant,
        secs: u64,
        force: bool,
        reply: ClaudeQuota,
    ) -> (ClaudeQuota, bool) {
        let mut fetched = false;
        let wall = t0() + chrono::Duration::seconds(secs as i64);
        let out = quota_step(
            cache,
            force,
            start + Duration::from_secs(secs),
            wall,
            || {
                fetched = true;
                reply
            },
            |_| {},
        );
        (out, fetched)
    }

    #[test]
    fn polling_every_minute_fetches_at_most_every_refresh_interval() {
        let mut cache = QuotaCache::new();
        let start = Instant::now();
        let mut fetches = 0;
        for minute in 0..30u64 {
            let (_, fetched) = call(&mut cache, start, minute * 60, false, good(t0(), 3.0));
            fetches += fetched as u32;
        }
        // 30 minutes of once-a-minute polling: one fetch per 3 minutes.
        assert_eq!(fetches, 10);
    }

    #[test]
    fn alternating_rate_limits_keep_the_reading_current_and_back_off() {
        let mut cache = QuotaCache::new();
        let start = Instant::now();
        let (first, fetched) = call(&mut cache, start, 0, false, good(t0(), 3.0));
        assert!(fetched);
        let key = |q: &ClaudeQuota| {
            let mut v = serde_json::to_value(q).unwrap();
            v.as_object_mut().unwrap().remove("fetchedAt");
            v
        };
        let mut fetch_times = vec![0u64];
        let mut ok_next = false;
        for minute in 1..=60u64 {
            let secs = minute * 60;
            let wall = t0() + chrono::Duration::seconds(secs as i64);
            let reply = if ok_next {
                good(wall, 3.0)
            } else {
                failed(wall, claude_limits::RATE_LIMITED)
            };
            let (out, fetched) = call(&mut cache, start, secs, false, reply);
            if fetched {
                fetch_times.push(minute);
                ok_next = !ok_next;
            }
            // Every answer is the same reading, "ok", with no message: the sync
            // content hash (which ignores fetchedAt) never changes.
            assert_eq!(out.status, "ok", "minute {minute}");
            assert_eq!(out.message, None);
            assert_eq!(key(&out), key(&first), "minute {minute}");
        }
        // ok at 0, 429 at 3 → wait 6 → ok at 9 → 3 → 429 at 12 → 6 → ok at 18 …
        assert_eq!(&fetch_times[..5], &[0, 3, 9, 12, 18]);
    }

    #[test]
    fn sustained_rate_limit_turns_stale_once_past_the_grace_period() {
        let mut cache = QuotaCache::new();
        let start = Instant::now();
        call(&mut cache, start, 0, false, good(t0(), 3.0));
        let mut fetches = Vec::new();
        let mut first_stale = None;
        for minute in 1..=40u64 {
            let secs = minute * 60;
            let wall = t0() + chrono::Duration::seconds(secs as i64);
            let (out, fetched) = call(
                &mut cache,
                start,
                secs,
                false,
                failed(wall, claude_limits::RATE_LIMITED),
            );
            if fetched {
                fetches.push(minute);
            }
            if out.status == "stale" && first_stale.is_none() {
                first_stale = Some(minute);
            }
            if let Some(since) = first_stale {
                assert_eq!(
                    out.status, "stale",
                    "minute {minute} after stale at {since}"
                );
                assert_eq!(out.message.as_deref(), Some(claude_limits::RATE_LIMITED));
                // Still the last real numbers, just marked stale.
                assert_eq!(out.five_hour.as_ref().unwrap().used_pct, 3.0);
            }
        }
        // 429s at 3 (wait 6), 9 (wait 10), 19, 29, 39: the reading from minute 0
        // is 19 minutes old at the third one, past the 15-minute grace.
        assert_eq!(fetches, vec![3, 9, 19, 29, 39]);
        assert_eq!(first_stale, Some(19));
    }

    #[test]
    fn other_failures_are_stale_at_once_and_force_bypasses_the_wait() {
        let mut cache = QuotaCache::new();
        let start = Instant::now();
        call(&mut cache, start, 0, false, good(t0(), 3.0));
        let wall = t0() + chrono::Duration::seconds(180);
        let (out, _) = call(&mut cache, start, 180, false, failed(wall, "HTTP 503"));
        assert_eq!(out.status, "stale");
        assert_eq!(out.message.as_deref(), Some("HTTP 503"));
        // Within the wait a forced refresh still fetches.
        let (out, fetched) = call(&mut cache, start, 200, true, good(wall, 7.0));
        assert!(fetched);
        assert_eq!(out.status, "ok");
        assert_eq!(out.five_hour.unwrap().used_pct, 7.0);
    }

    #[test]
    fn a_rate_limit_with_an_old_or_missing_reading_is_not_dressed_up() {
        // Old reading loaded from disk at launch, first refresh rate limited.
        let mut cache = QuotaCache::new();
        cache.value = Some(good(t0() - chrono::Duration::hours(3), 3.0));
        let start = Instant::now();
        let (out, _) = call(
            &mut cache,
            start,
            0,
            false,
            failed(t0(), claude_limits::RATE_LIMITED),
        );
        assert_eq!(out.status, "stale");
        // No reading at all: the failure itself.
        let mut empty = QuotaCache::new();
        let (out, _) = call(
            &mut empty,
            start,
            0,
            false,
            failed(t0(), claude_limits::RATE_LIMITED),
        );
        assert_eq!(out.status, "stale");
        assert!(out.five_hour.is_none());
    }
}
