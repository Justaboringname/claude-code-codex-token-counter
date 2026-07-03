mod claude_limits;
mod codex;
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
static COMPUTE_LOCK: Mutex<()> = Mutex::new(());

struct ClaudeQuotaCache {
    value: Option<claude_limits::ClaudeQuota>,
    checked_at: Option<Instant>,
}

static CLAUDE_QUOTA_CACHE: Mutex<ClaudeQuotaCache> = Mutex::new(ClaudeQuotaCache {
    value: None,
    checked_at: None,
});

fn claude_quota_cache_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| {
        d.join("claude-code-token-counter")
            .join("claude_quota.json")
    })
}

fn load_claude_quota_cache() -> Option<claude_limits::ClaudeQuota> {
    let text = std::fs::read_to_string(claude_quota_cache_path()?).ok()?;
    serde_json::from_str(&text).ok()
}

fn save_claude_quota_cache(quota: &claude_limits::ClaudeQuota) {
    let Some(path) = claude_quota_cache_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_vec(quota) {
        let _ = std::fs::write(path, json);
    }
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
        // queue instead of racing. `into_inner` recovers from a poisoned lock so
        // one panicked scan can't wedge every later refresh.
        let _guard = COMPUTE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let plan_id = plan.as_deref().unwrap_or("max5x");
        usage::compute_usage_with_plan(&range, plan_id)
    })
    .await
    .unwrap_or_default()
}

#[tauri::command]
async fn get_claude_quota(force: Option<bool>) -> claude_limits::ClaudeQuota {
    tauri::async_runtime::spawn_blocking(move || {
        let mut cache = CLAUDE_QUOTA_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if cache.value.is_none() {
            cache.value = load_claude_quota_cache();
        }
        if !force.unwrap_or(false)
            && cache
                .checked_at
                .is_some_and(|at| at.elapsed() < Duration::from_secs(60))
        {
            if let Some(value) = cache.value.clone() {
                return value;
            }
        }

        let fresh = claude_limits::fetch();
        cache.checked_at = Some(Instant::now());
        if fresh.status == "ok" {
            save_claude_quota_cache(&fresh);
            cache.value = Some(fresh.clone());
            return fresh;
        }

        if let Some(mut last) = cache.value.clone() {
            last.status = "stale".into();
            last.message = fresh.message;
            return last;
        }
        fresh
    })
    .await
    .unwrap_or_else(|e| claude_limits::ClaudeQuota {
        five_hour: None,
        weekly: None,
        status: "error".into(),
        message: Some(format!("quota task failed: {e}")),
        fetched_at: chrono::Utc::now().to_rfc3339(),
        subscription: None,
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
    let Some(home) = dirs::home_dir() else { return };
    // Watch both providers' transcript roots; either may be absent (user
    // without Codex installed, or vice versa) — watch whichever exists.
    let roots: Vec<_> = [
        home.join(".claude").join("projects"),
        home.join(".codex").join("sessions"),
    ]
    .into_iter()
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_liquid_glass::init())
        .setup(|app| {
            #[cfg(target_os = "macos")]
            {
                use tauri_plugin_liquid_glass::{
                    GlassMaterialVariant, LiquidGlassConfig, LiquidGlassExt,
                };
                let window = app.get_webview_window("main").expect("main window");
                // macOS 26+ Liquid Glass via the private NSGlassEffectView
                // (inserted as the window's backdrop, below the transparent
                // webview). Falls back to NSVisualEffectView pre-26. This is
                // real refractive glass — the CSS surface only styles content.
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
            spawn_file_watcher(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_usage,
            get_claude_quota,
            set_theme
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
