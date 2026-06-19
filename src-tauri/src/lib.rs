mod usage;

use std::time::Duration;
use tauri::{Emitter, Manager};

#[tauri::command]
async fn get_usage(range: String, plan: Option<String>) -> usage::Usage {
    // This parses hundreds of MB of JSONL — heavy, blocking work. A synchronous
    // Tauri command runs on the main thread, so it would freeze the popover
    // ("Application Not Responding") for the whole parse. That's especially bad
    // because the file watcher re-fires this on every change while Claude Code
    // is active. Run it on the blocking pool so the UI thread stays responsive.
    tauri::async_runtime::spawn_blocking(move || {
        let plan_id = plan.as_deref().unwrap_or("max5x");
        usage::compute_usage_with_plan(&range, plan_id)
    })
    .await
    .unwrap_or_default()
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
    let projects_dir = home.join(".claude").join("projects");
    if !projects_dir.exists() {
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
        if let Err(e) = debouncer
            .watcher()
            .watch(&projects_dir, RecursiveMode::Recursive)
        {
            eprintln!("file watcher watch failed: {e}");
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
        .invoke_handler(tauri::generate_handler![get_usage, set_theme])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
