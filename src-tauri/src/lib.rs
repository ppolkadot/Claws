mod commands;
mod desktop;
#[cfg(test)]
mod download_tests;
mod download_files;
mod download_policy;
mod downloader;
mod error;
mod history;
mod links;
mod probe;
mod settings;
mod state;
mod store;
mod tools;
mod window;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tauri::Manager;

use crate::downloader::{Job, PendingCleanup};
use crate::history::HistoryItem;
use crate::settings::Settings;
use crate::state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        // Must be registered first. A second launch would run its own queue against the
        // same files, so it brings the existing window forward instead.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            desktop::bring_forward(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(desktop::shortcut_plugin())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![desktop::BACKGROUND_ARG]),
        ))
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;

            store::sweep_temp_files(&data_dir);
            let mut notices = Vec::new();
            let mut settings: Settings = store::load_or_recover(&data_dir.join("settings.json"), "settings", &mut notices);
            if let Some(notice) = settings.repair() {
                notices.push(notice);
            }
            if settings.output_dir.trim().is_empty() {
                settings.output_dir = app
                    .path()
                    .download_dir()?
                    .join("Claws")
                    .to_string_lossy()
                    .into_owned();
            }
            // An exit in the middle of a tool install can leave a half-swapped tool.
            tools::recover_bin(&data_dir.join("bin"));
            let history: Vec<HistoryItem> = store::load_or_recover(&data_dir.join("history.json"), "library", &mut notices);
            let queue: Vec<Job> = store::load_or_recover(&data_dir.join("queue.json"), "download queue", &mut notices);
            let pending_cleanup: Vec<PendingCleanup> = store::load_or_recover(&data_dir.join("cleanup.json"), "cleanup list", &mut notices);

            let http = reqwest::Client::builder()
                .user_agent(concat!("Claws/", env!("CARGO_PKG_VERSION")))
                .connect_timeout(Duration::from_secs(15))
                .build()?;

            app.manage(AppState {
                data_dir,
                settings: Mutex::new(settings.clone()),
                history: Mutex::new(history),
                jobs: Mutex::new(Vec::new()),
                running: Mutex::new(HashMap::new()),
                http,
                engine: tokio::sync::RwLock::new(()),
                notices: Mutex::new(Vec::new()),
                shutting_down: AtomicBool::new(false),
                workers: AtomicUsize::new(0),
                live_jobs: Mutex::new(std::collections::HashSet::new()),
                probes: Mutex::new(HashMap::new()),
                recent_probes: Mutex::new(Vec::new()),
                // Only with the notification-area icon, or a hidden window could not be found again.
                start_hidden: AtomicBool::new(
                    settings.keep_running && std::env::args().any(|a| a == desktop::BACKGROUND_ARG),
                ),
                pending_cleanup: Mutex::new(pending_cleanup),
                destinations: Mutex::new(std::collections::HashSet::new()),
            });
            downloader::restore_queue(app.handle(), queue);
            let state = app.state::<AppState>();
            probe::prune_cache(&state.probe_cache_dir());
            links::prune(&links::cache_dir(&state.data_dir));

            desktop::create_tray(app.handle())?;
            if let Some(problem) = desktop::apply(app.handle(), &settings) {
                notices.push(problem);
            }

            if let Some(main) = app.get_webview_window("main") {
                window::apply_theme(&main, &settings.theme);
                if let Err(e) = window::apply_backdrop(&main, &settings.backdrop) {
                    notices.push(e);
                }
                // The UI shows the window after its first paint; this is the fallback
                // so a frontend failure can never leave the app invisible.
                let fallback = main.clone();
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(4)).await;
                    if !handle.state::<AppState>().start_hidden.load(Ordering::SeqCst) {
                        let _ = fallback.show();
                    }
                });
            }
            *app.state::<AppState>().notices.lock().unwrap() = notices;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap,
            commands::app_ready,
            commands::quit_app,
            commands::probe_url,
            commands::cached_link,
            commands::enqueue,
            commands::pause_job,
            commands::resume_job,
            commands::cancel_job,
            commands::remove_job,
            commands::clear_finished,
            commands::save_settings,
            commands::list_history,
            commands::open_download,
            commands::reveal_download,
            commands::open_output_dir,
            commands::delete_history,
            commands::clear_history,
            commands::tools_status,
            commands::install_ytdlp,
            commands::update_ytdlp,
            commands::install_ffmpeg,
            commands::default_output_dir,
            commands::open_font_page,
        ])
        .build(tauri::generate_context!())
        .expect("error while starting Claws");

    app.run(|app, event| {
        // Stop yt-dlp and ffmpeg and save the queue before exiting.
        if let tauri::RunEvent::Exit = event {
            downloader::shutdown(app);
        }
    });
}
