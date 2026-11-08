// Tests drive the real lifecycle code against a mock app, so the runtime is an alias.
#![cfg_attr(test, allow(dead_code))]

mod bounded_io;
mod commands;
mod desktop;
mod disk_budget;
#[cfg(test)]
mod download_tests;
mod download_files;
mod download_policy;
mod downloader;
mod error;
mod estimate;
mod history;
#[cfg(test)]
mod lifecycle_tests;
mod links;
mod probe;
mod queue_writer;
mod settings;
mod state;
mod store;
mod tools;
mod transcript;
mod transfer_progress;
mod window;

#[cfg(not(test))]
pub(crate) type Runtime = tauri::Wry;
#[cfg(test)]
pub(crate) type Runtime = tauri::test::MockRuntime;
pub(crate) type AppHandle = tauri::AppHandle<Runtime>;
pub(crate) type WebviewWindow = tauri::WebviewWindow<Runtime>;

#[cfg(not(test))]
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    use tauri::Manager;

    use crate::downloader::{Job, PendingCleanup};
    use crate::history::HistoryItem;
    use crate::settings::Settings;
    use crate::state::AppState;

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
            let queue: Vec<Job> = queue_writer::load(&data_dir.join("queue.json"), &mut notices);
            let pending_cleanup: Vec<PendingCleanup> = store::load_or_recover(&data_dir.join("cleanup.json"), "cleanup list", &mut notices);

            let queue_writer = downloader::start_queue_writer(app.handle());
            // Only with the notification-area icon, or a hidden window could not be found again.
            let start_hidden = settings.keep_running && std::env::args().any(|a| a == desktop::BACKGROUND_ARG);
            app.manage(AppState::new(data_dir, settings.clone(), history, pending_cleanup, queue_writer, start_hidden));
            let _recovery = downloader::restore_queue(app.handle(), queue);
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
            commands::estimate_download,
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
