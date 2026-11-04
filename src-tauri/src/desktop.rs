//! Quick access from anywhere in Windows: the Quick Open shortcut, the notification-area
//! icon that keeps Claws running when its window is closed, and opening at login.

use std::sync::atomic::Ordering;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::settings::Settings;
use crate::state::AppState;

const TRAY_ID: &str = "claws";
/// Passed by the login entry, so Claws starts in the notification area.
pub const BACKGROUND_ARG: &str = "--background";

/// Shows, restores and focuses the main window.
pub fn bring_forward(app: &AppHandle) {
    app.state::<AppState>().start_hidden.store(false, Ordering::SeqCst);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// The shortcut toggles, like Spotlight: it brings Claws forward (and the UI looks up a
/// link on the clipboard), or puts it away again when it is already in front.
fn quick_open(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else { return };
    let in_front = window.is_visible().unwrap_or(false)
        && window.is_focused().unwrap_or(false)
        && !window.is_minimized().unwrap_or(false);
    if in_front {
        let keep_running = app.state::<AppState>().settings.lock().unwrap().keep_running;
        let _ = if keep_running { window.hide() } else { window.minimize() };
        return;
    }
    bring_forward(app);
    crate::window::take_foreground(&window);
    let _ = window.set_focus();
    let _ = app.emit("quick-open", ());
}

pub fn shortcut_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, _shortcut, event| {
            if event.state() != ShortcutState::Pressed {
                return;
            }
            // Windows will not hand focus to a window while the shortcut's Ctrl/Alt/Shift
            // are still held: the window would appear behind the active app, and the next
            // press would bring it forward again instead of putting it away. So wait
            // (briefly) for the keys to be let go, then act on the main thread.
            let app = app.clone();
            std::thread::spawn(move || {
                wait_for_modifiers_released(std::time::Duration::from_millis(600));
                let handle = app.clone();
                let _ = app.run_on_main_thread(move || quick_open(&handle));
            });
        })
        .build()
}

#[cfg(windows)]
fn wait_for_modifiers_released(limit: std::time::Duration) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT};
    let held = || {
        [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN]
            .iter()
            // The high bit is set while the key is down.
            .any(|key| unsafe { GetAsyncKeyState(i32::from(key.0)) } as u16 & 0x8000 != 0)
    };
    let deadline = std::time::Instant::now() + limit;
    while held() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[cfg(not(windows))]
fn wait_for_modifiers_released(_limit: std::time::Duration) {}

pub fn create_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Claws", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Claws", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open, &separator, &quit])?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Claws")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => bring_forward(app),
            // Runs RunEvent::Exit, which stops downloads and saves the queue.
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                bring_forward(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// Human-readable name of a shortcut choice, for messages.
fn shortcut_label(choice: &str) -> String {
    choice
        .split('+')
        .map(|part| match part {
            "ctrl" => "Ctrl".to_string(),
            "alt" => "Alt".to_string(),
            "shift" => "Shift".to_string(),
            "space" => "Space".to_string(),
            other => other.to_uppercase(),
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// Applies the quick-access settings: registers the shortcut, shows or hides the
/// notification-area icon, and adds or removes the login entry. Returns a message for
/// whatever could not be applied; the settings themselves are already saved.
pub fn apply(app: &AppHandle, settings: &Settings) -> Option<String> {
    let mut problems = Vec::new();

    let shortcuts = app.global_shortcut();
    let _ = shortcuts.unregister_all();
    if settings.quick_open_shortcut != "off" {
        if let Err(e) = shortcuts.register(settings.quick_open_shortcut.as_str()) {
            problems.push(format!(
                "{} is already used by another app, so Quick Open could not use it ({e}). Choose another shortcut.",
                shortcut_label(&settings.quick_open_shortcut)
            ));
        }
    }

    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_visible(settings.keep_running);
    }

    let autostart = app.autolaunch();
    let result = if settings.open_at_login {
        // Re-enabled on every start too, so the entry follows the app if it moves.
        autostart.enable()
    } else if autostart.is_enabled().unwrap_or(false) {
        autostart.disable()
    } else {
        Ok(())
    };
    if let Err(e) = result {
        problems.push(format!("Claws could not change whether it opens at login ({e})."));
    }

    (!problems.is_empty()).then(|| problems.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_read_naturally() {
        assert_eq!(shortcut_label("ctrl+alt+d"), "Ctrl+Alt+D");
        assert_eq!(shortcut_label("ctrl+shift+space"), "Ctrl+Shift+Space");
    }
}
