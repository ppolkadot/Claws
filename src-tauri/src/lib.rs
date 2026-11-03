mod commands;
mod downloader;
mod error;
mod history;
mod probe;
mod settings;
mod state;
mod store;
mod tools;
mod window;

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use tauri::Manager;

use crate::history::HistoryItem;
use crate::settings::Settings;
use crate::state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;

            let mut settings: Settings = store::load(&data_dir.join("settings.json"));
            if settings.output_dir.trim().is_empty() {
                settings.output_dir = app
                    .path()
                    .download_dir()?
                    .join("Claws")
                    .to_string_lossy()
                    .into_owned();
            }
            let history: Vec<HistoryItem> = store::load(&data_dir.join("history.json"));

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
            });
            downloader::restore_queue(app.handle());
            probe::prune_cache(&app.state::<AppState>().probe_cache_dir());

            if let Some(main) = app.get_webview_window("main") {
                window::apply_theme(&main, &settings.theme);
                if let Err(e) = window::apply_backdrop(&main, &settings.backdrop) {
                    eprintln!("could not apply the window backdrop: {e}");
                }
                // The UI shows the window after its first paint; this is the fallback
                // so a frontend failure can never leave the app invisible.
                let fallback = main.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(4)).await;
                    let _ = fallback.show();
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap,
            commands::app_ready,
            commands::probe_url,
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
        .run(tauri::generate_context!())
        .expect("error while running Claws");
}
