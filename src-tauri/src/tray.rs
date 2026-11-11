// Menu bar item. A left click on the item drops the compact "tray" window
// below it (the same UI as the full window, without the title bar); a click
// elsewhere hides it again. The full window is still there for "展开详情"
// and the right-click menu. The item shows today's spend as its title.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, PhysicalPosition, Rect, Runtime, WebviewWindow, Window};

pub const TRAY_ID: &str = "token-counter";
pub const TRAY_LABEL: &str = "tray";
pub const MAIN_LABEL: &str = "main";
// Gap between the menu bar and the top of the panel, in logical points.
const PANEL_GAP_PT: f64 = 6.0;
// When the panel loses focus because the item was clicked, the same click
// must not reopen it. A hide this recent counts as that click.
const REOPEN_GUARD: Duration = Duration::from_millis(250);

static LAST_HIDE: Mutex<Option<Instant>> = Mutex::new(None);

/// No menu is attached: on macOS an attached NSMenu takes over the left
/// click, so the panel would never open. Left click toggles the panel,
/// right click opens the full window, and the panel has 展开详情 and 退出.
pub fn install<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Token Counter")
        .title("…")
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button,
                button_state: MouseButtonState::Up,
                rect,
                ..
            } = event
            {
                match button {
                    MouseButton::Left => toggle_panel(tray.app_handle(), &rect),
                    MouseButton::Right => show_full_window(tray.app_handle()),
                    _ => {}
                }
            }
        })
        .build(app)?;
    Ok(())
}

/// Called for every window event; hides the panel when it loses focus and
/// keeps both windows alive when their close button is pressed.
pub fn on_window_event<R: Runtime>(window: &Window<R>, event: &tauri::WindowEvent) {
    match event {
        tauri::WindowEvent::Focused(false) if window.label() == TRAY_LABEL => {
            hide_panel(window.app_handle());
        }
        tauri::WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            let _ = window.hide();
            if window.label() == TRAY_LABEL {
                mark_hidden();
            }
        }
        _ => {}
    }
}

fn toggle_panel<R: Runtime>(app: &AppHandle<R>, rect: &Rect) {
    let Some(panel) = app.get_webview_window(TRAY_LABEL) else {
        return;
    };
    if panel.is_visible().unwrap_or(false) {
        hide_panel(app);
        return;
    }
    // The click that hid the panel by losing focus also reaches us; skip it.
    if let Some(at) = *LAST_HIDE.lock().unwrap_or_else(|e| e.into_inner()) {
        if at.elapsed() < REOPEN_GUARD {
            return;
        }
    }
    place_under(&panel, rect);
    let _ = panel.show();
    let _ = panel.set_focus();
}

/// Centres the panel under the menu bar item, clamped to the screen.
fn place_under<R: Runtime>(panel: &WebviewWindow<R>, rect: &Rect) {
    let scale = panel.scale_factor().unwrap_or(1.0);
    let at = rect.position.to_physical::<f64>(scale);
    let size = rect.size.to_physical::<f64>(scale);
    let width = panel.outer_size().map(|s| s.width as f64).unwrap_or(380.0 * scale);

    let mut x = at.x + size.width / 2.0 - width / 2.0;
    let y = at.y + size.height + PANEL_GAP_PT * scale;

    if let Ok(Some(monitor)) = panel.current_monitor() {
        let left = monitor.position().x as f64;
        let right = left + monitor.size().width as f64;
        x = x.clamp(left + 8.0 * scale, (right - width - 8.0 * scale).max(left));
    }
    let _ = panel.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
}

pub fn hide_panel<R: Runtime>(app: &AppHandle<R>) {
    if let Some(panel) = app.get_webview_window(TRAY_LABEL) {
        let _ = panel.hide();
    }
    mark_hidden();
}

fn mark_hidden() {
    *LAST_HIDE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
}

/// Opens the full window (also used by the panel's 展开详情 button).
pub fn show_full_window<R: Runtime>(app: &AppHandle<R>) {
    hide_panel(app);
    if let Some(main) = app.get_webview_window(MAIN_LABEL) {
        let _ = main.show();
        let _ = main.unminimize();
        let _ = main.set_focus();
    }
}

/// Sets the menu bar title, e.g. "$12.4". Empty or None clears it.
pub fn set_title<R: Runtime>(app: &AppHandle<R>, text: Option<String>) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_title(text);
    }
}
