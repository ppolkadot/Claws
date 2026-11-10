use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

use crate::AppHandle;

use crate::download_files;
use crate::download_policy::VideoPreset;
use crate::downloader::{DownloadOptions, Job};
use crate::error::notify;
use crate::history_mutation;
use crate::state::AppState;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HistoryItem {
    pub id: String,
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub match_origin: Option<crate::spotify_match::MatchOrigin>,
    pub uploader: Option<String>,
    pub duration: Option<f64>,
    /// Remote thumbnail, used until (or if) a local copy exists.
    pub thumbnail: Option<String>,
    pub thumbnail_local: Option<String>,
    pub file_path: Option<String>,
    pub file_size: Option<u64>,
    /// "video", "audio" or "transcript".
    pub mode: String,
    pub quality: String,
    pub finished_at: i64,
    /// What proves `file_path` is the very file this download saved: its identity, taken
    /// from the verified publication record. Absent for entries saved before this existed,
    /// which are never assumed to own whatever file now sits at their path.
    #[serde(default)]
    pub file_identity: Option<crate::download_files::FileIdentity>,
    /// Side files (captions) the download really published next to the media, as recorded
    /// by the verified publication journal. Only these may be deleted with the entry.
    #[serde(default)]
    pub published_sidecars: Vec<crate::download_files::Move>,
    /// Side files whose names were taken in the output folder, left in the download's work
    /// folder. Saved with the entry, so they stay reachable after the job leaves the Queue
    /// and after a restart. Only a path to them, never permission to delete them.
    #[serde(default)]
    pub kept_files: Option<crate::download_files::KeptFiles>,
}

/// Adds (or replaces, by job id) the job's Library entry and saves the Library, which
/// changes only once the save worked. Callers treat the error as "not recorded": the job
/// keeps its recovery record and is retried. Blocking (it reads the publication record).
pub fn record(app: &AppHandle, job: &Job) -> crate::error::Result<()> {
    let file_size = job
        .file_path
        .as_deref()
        .and_then(|p| std::fs::metadata(p).ok())
        .map(|m| m.len());
    let quality = quality(&job.options);
    // From the verified publication record, which is still there: it is retired only after
    // this entry is saved. No record (an old download) means no evidence, not a guess.
    // A marker that cannot be read right now is not "no evidence": saving the entry without
    // it would cost the download its ownership proof for good, so the job is not recorded
    // yet (it keeps its recovery record and Try Again records it).
    let evidence = match job.workspace.as_ref().zip(job.file_path.as_deref()) {
        Some((workspace, path)) => download_files::evidence(workspace, &job.id, Path::new(path))
            .map_err(|e| crate::error::AppError::msg(format!("its publication record could not be read right now: {e}")))?,
        None => None,
    };
    let item = HistoryItem {
        id: job.id.clone(),
        title: job.title.clone(),
        url: job.url.clone(),
        match_origin: job.match_origin.clone(),
        uploader: job.uploader.clone(),
        duration: job.duration,
        thumbnail: job.thumbnail.clone(),
        thumbnail_local: None,
        file_path: job.file_path.clone(),
        file_size,
        mode: job.options.mode.clone(),
        quality,
        finished_at: chrono::Utc::now().timestamp_millis(),
        file_identity: evidence.as_ref().map(|e| e.identity.clone()),
        published_sidecars: evidence.map(|e| e.sidecars).unwrap_or_default(),
        kept_files: job
            .workspace
            .as_ref()
            .and_then(|workspace| download_files::kept_files(workspace, &job.id, job.file_path.as_deref().map(Path::new))),
    };
    history_mutation::add(app, item)?;

    if let Some(url) = job.thumbnail.clone() {
        let app = app.clone();
        let id = job.id.clone();
        tauri::async_runtime::spawn(async move {
            cache_thumbnail(&app, &id, &url).await;
        });
    }
    Ok(())
}

/// What the last look for kept side files of older downloads found.
#[derive(Serialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RepairReport {
    /// Entries that were given a kept-files record by this look.
    pub attached: usize,
    /// Known output folders that could not be read, so their downloads were not checked.
    /// A retry looks at them again.
    pub inaccessible: Vec<String>,
    /// Work folders left alone: no matching Library entry, or a marker that does not prove it.
    pub ignored: usize,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum KeptStatus {
    /// The kept files were found and shown.
    Shown,
    /// The output folder (or its drive) cannot be reached right now.
    Offline,
    /// The folder is reachable but the kept files are gone.
    Missing,
    /// A folder is there, but Claws cannot prove it is this download's.
    Unverified,
}

/// The answer to "Show kept files", for the Library and Queue to tell apart.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KeptReveal {
    pub status: KeptStatus,
    /// The folder holding the kept files.
    pub folder: String,
    /// How many kept files are there.
    pub files: usize,
}

/// What "Show kept files" does for an entry: the answer, and the file to reveal when the
/// kept files were found. Only the entry's saved record is used, resolved through the
/// verified work folder. Blocking.
pub fn reveal_plan(item: &HistoryItem) -> crate::error::Result<(KeptReveal, Option<PathBuf>)> {
    use download_files::KeptState;
    let kept = item.kept_files.as_ref().ok_or_else(|| crate::error::AppError::msg("No side files were kept for this download."))?;
    let folder = kept.workspace.path.to_string_lossy().into_owned();
    let (status, files) = match download_files::locate_kept(kept, &item.id) {
        KeptState::Available(files) => (KeptStatus::Shown, files),
        KeptState::Offline => (KeptStatus::Offline, Vec::new()),
        KeptState::Missing => (KeptStatus::Missing, Vec::new()),
        KeptState::Unverified => (KeptStatus::Unverified, Vec::new()),
    };
    Ok((KeptReveal { status, folder, files: files.len() }, files.into_iter().next()))
}

/// Gives a Library entry the kept-files record of its job's work folder if it has none.
/// For a download that was recorded before kept records existed and is finished at start.
pub fn attach_kept(app: &AppHandle, job: &Job) -> crate::error::Result<()> {
    let Some(kept) = job
        .workspace
        .as_ref()
        .and_then(|workspace| download_files::kept_files(workspace, &job.id, job.file_path.as_deref().map(Path::new)))
    else {
        return Ok(());
    };
    history_mutation::mutate(app, |history| match history.iter_mut().find(|h| h.id == job.id && h.kept_files.is_none()) {
        Some(item) => {
            item.kept_files = Some(kept);
            ((), true)
        }
        None => ((), false),
    })
}

/// One look at a time (a retry waits for the startup look instead of running beside it).
static LOOKING: Mutex<()> = Mutex::new(());
/// The last report of each app's data folder.
static LAST_REPAIR: Mutex<Vec<(PathBuf, RepairReport)>> = Mutex::new(Vec::new());

/// The most output folders discovered from the Library's saved paths.
const MAX_DISCOVERED_FOLDERS: usize = 256;
/// How far above a saved file a work folder is looked for (a filename template can nest).
const ANCESTOR_LEVELS: usize = 4;

/// The last report, for the Library to show; empty until a look has run.
pub fn last_repair(app: &AppHandle) -> RepairReport {
    let dir = app.state::<AppState>().data_dir.clone();
    let reports = LAST_REPAIR.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    reports.iter().find(|(d, _)| *d == dir).map(|(_, r)| r.clone()).unwrap_or_default()
}

/// Case-insensitive, separator-neutral spelling of a path, for comparing and de-duplicating
/// the folders Windows would treat as one.
fn spelled(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

fn is_under(root: &Path, path: &Path) -> bool {
    let (root, path) = (spelled(root), spelled(path));
    path.strip_prefix(&root).is_some_and(|rest| rest.starts_with('\\'))
}

/// Looks, once and bounded, for kept side files that downloads of older versions left in
/// their work folders. Blocking.
///
/// Only known output folders are looked at: the current one, those of jobs in the Queue, and
/// folders at (or a few levels above) the paths the Library saved. In each, only the work
/// folders directly inside `.claws-work` are read, and only one a Library entry of the same id
/// and media claims, and that verifies as that download's, is given a record. Nothing is
/// moved, adopted or deleted. A known folder that cannot be read is reported so the Library
/// can offer another look.
pub fn repair_kept_files(app: &AppHandle) -> RepairReport {
    let _one_at_a_time = LOOKING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let report = look_for_kept_files(app);
    let dir = app.state::<AppState>().data_dir.clone();
    {
        let mut reports = LAST_REPAIR.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        reports.retain(|(d, _)| *d != dir);
        reports.push((dir, report.clone()));
    }
    if report.attached == 0 && !report.inaccessible.is_empty() {
        // A Library that was opened before this look finished re-reads the report when the
        // Library changes. Attaching a record already announces that.
        let _ = app.emit("history-changed", ());
    }
    report
}

/// What looking for `.claws-work` inside a folder amounted to.
#[derive(Debug, PartialEq, Eq)]
enum Lookup {
    Found,
    /// No such folder: the ordinary case.
    Nothing,
    /// The folder could not be looked at (access denied, a drive that is not ready).
    Unreadable,
}

fn classify_lookup(lookup: Result<(), std::io::ErrorKind>) -> Lookup {
    match lookup {
        Ok(()) => Lookup::Found,
        Err(std::io::ErrorKind::NotFound) => Lookup::Nothing,
        Err(_) => Lookup::Unreadable,
    }
}

fn look_for_kept_files(app: &AppHandle) -> RepairReport {
    let state = app.state::<AppState>();
    // Entries that could still need a record: (id, saved media path). An entry without a saved
    // file cannot be matched to a work folder by more than its id, so it is not a candidate.
    // `settled` holds the ids whose work folders are no mystery: entries that already have
    // their record, and jobs still in the Queue.
    let (candidates, mut settled): (Vec<(String, Option<PathBuf>)>, HashSet<String>) = {
        let history = state.history.lock().unwrap();
        let candidates = history
            .iter()
            .filter(|h| h.kept_files.is_none())
            .filter_map(|h| h.file_path.as_ref().map(|path| (h.id.clone(), Some(PathBuf::from(path)))))
            .collect();
        let settled = history.iter().filter(|h| h.kept_files.is_some()).map(|h| h.id.clone()).collect();
        (candidates, settled)
    };
    settled.extend(state.jobs.lock().unwrap().iter().map(|j| j.id.clone()));
    let mut explicit: Vec<PathBuf> = vec![PathBuf::from(state.settings.lock().unwrap().output_dir.clone())];
    explicit.extend(state.jobs.lock().unwrap().iter().filter_map(|j| j.execution.as_ref().map(|e| e.output_dir.clone())));
    explicit.retain(|p| !p.as_os_str().is_empty());

    let mut report = RepairReport::default();
    let mut seen = HashSet::new();
    let mut roots: Vec<PathBuf> = Vec::new();
    for root in explicit {
        if !seen.insert(spelled(&root)) {
            continue;
        }
        if fs::metadata(&root).is_ok_and(|m| m.is_dir()) {
            roots.push(root);
        } else if candidates.iter().any(|(_, media)| media.as_deref().is_some_and(|m| is_under(&root, m))) {
            // Downloads were saved there and it cannot be read now (an unplugged drive).
            report.inaccessible.push(root.to_string_lossy().into_owned());
        }
    }
    // A drive (or share) that only the Library's saved paths remember: a missing drive answers
    // "not found" like any folder without a work folder, so it is asked about directly.
    let mut drives = HashSet::new();
    for (_, media) in &candidates {
        let drive = media.as_deref().and_then(|m| m.ancestors().last()).filter(|d| matches!(d.components().next(), Some(std::path::Component::Prefix(_))));
        if let Some(drive) = drive.filter(|d| drives.insert(spelled(d))) {
            if fs::metadata(drive).is_err() {
                report.inaccessible.retain(|p| !is_under(drive, Path::new(p)));
                report.inaccessible.push(drive.to_string_lossy().into_owned());
            }
        }
    }
    let mut examined = HashSet::new();
    'discover: for (_, media) in &candidates {
        let mut dir = media.as_deref().and_then(Path::parent);
        for _ in 0..ANCESTOR_LEVELS {
            let Some(current) = dir else { break };
            if examined.insert(spelled(current)) {
                if examined.len() > MAX_DISCOVERED_FOLDERS {
                    break 'discover;
                }
                if seen.insert(spelled(current)) {
                    // Not there is the ordinary case. Any other answer (access denied, a drive
                    // that is not ready) means this folder was not really looked at.
                    let lookup = fs::symlink_metadata(current.join(download_files::WORK_DIR)).map(|_| ()).map_err(|e| e.kind());
                    match classify_lookup(lookup) {
                        Lookup::Found => roots.push(current.to_path_buf()),
                        Lookup::Unreadable => report.inaccessible.push(current.to_string_lossy().into_owned()),
                        Lookup::Nothing => {}
                    }
                }
            }
            dir = current.parent();
        }
    }

    let mut found: Vec<(String, download_files::KeptFiles)> = Vec::new();
    for root in &roots {
        match download_files::work_folders(root, download_files::MAX_WORK_FOLDERS) {
            Ok(folders) => {
                for folder in folders {
                    let kept = candidates
                        .iter()
                        .find(|(id, _)| *id == folder.job_id)
                        .and_then(|(id, media)| download_files::kept_files(&folder.reference, id, media.as_deref()).map(|kept| (id.clone(), kept)));
                    match kept {
                        Some(entry) if !found.iter().any(|(id, _)| *id == entry.0) => found.push(entry),
                        None if settled.contains(&folder.job_id) && !candidates.iter().any(|(id, _)| *id == folder.job_id) => {}
                        _ => report.ignored += 1,
                    }
                }
            }
            Err(_) => report.inaccessible.push(root.to_string_lossy().into_owned()),
        }
    }
    if !found.is_empty() {
        let attached = history_mutation::mutate(app, |history| {
            let mut attached = 0;
            for (id, kept) in &found {
                if let Some(item) = history.iter_mut().find(|h| h.id == *id && h.kept_files.is_none()) {
                    item.kept_files = Some(kept.clone());
                    attached += 1;
                }
            }
            (attached, attached > 0)
        });
        match attached {
            Ok(count) => report.attached = count,
            Err(e) => notify(app, format!("Claws found side files kept from earlier downloads but could not save your library ({e}). It will look again the next time it starts.")),
        }
    }
    report
}

/// The look at startup, off the startup path.
pub fn start_kept_repair(app: &AppHandle) -> std::thread::JoinHandle<()> {
    let app = app.clone();
    std::thread::spawn(move || {
        repair_kept_files(&app);
    })
}

/// The Library's one-word description of what was kept: the audio format, or the video's
/// size cap, followed by the preset when it is not the default (Compatible); for a
/// transcript, its language, kind of captions and format.
pub fn quality(options: &DownloadOptions) -> String {
    if options.mode == "transcript" {
        return crate::transcript::quality(options);
    }
    if options.mode == "audio" {
        return options.audio_format.to_uppercase();
    }
    let size = options.max_height.map(|h| format!("{h}p")).unwrap_or_else(|| "Best".into());
    match options.video_preset {
        VideoPreset::Compatible => size,
        VideoPreset::Original => format!("{size} · Original"),
        VideoPreset::EditingProRes => format!("{size} · Editing"),
    }
}

const THUMB_MAX_BYTES: usize = 5 * 1024 * 1024;

/// At most `limit` bytes, checked against the declared length and while streaming.
async fn read_capped(response: reqwest::Response, limit: usize) -> Option<Vec<u8>> {
    if response.content_length().is_some_and(|len| len > limit as u64) {
        return None;
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.ok()?;
        if body.len() + chunk.len() > limit {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    Some(body)
}

/// Best effort: keeps the library's artwork after the site's link expires.
async fn cache_thumbnail(app: &AppHandle, id: &str, url: &str) {
    let state = app.state::<AppState>();
    if !url.starts_with("https://") {
        return;
    }
    let result = async {
        let response = state
            .http
            .get(url)
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?;
        let ext = match response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
        {
            Some(t) if t.contains("png") => "png",
            Some(t) if t.contains("webp") => "webp",
            _ => "jpg",
        };
        let bytes = read_capped(response, THUMB_MAX_BYTES).await?;
        let dir = state.thumbs_dir();
        tokio::fs::create_dir_all(&dir).await.ok()?;
        // The file name is our own uuid, never anything from the remote site.
        let path: PathBuf = dir.join(format!("{id}.{ext}"));
        tokio::fs::write(&path, &bytes).await.ok()?;
        Some(path)
    }
    .await;

    let Some(path) = result else { return };
    let (handle, entry, local) = (app.clone(), id.to_string(), path.to_string_lossy().into_owned());
    let stored = tauri::async_runtime::spawn_blocking(move || {
        history_mutation::mutate(&handle, |history| match history.iter_mut().find(|h| h.id == entry) {
            Some(item) => {
                item.thumbnail_local = Some(local);
                (true, true)
            }
            None => (false, false),
        })
    })
    .await;
    match stored {
        Ok(Ok(true)) => {}
        Ok(Err(e)) => {
            notify(app, format!("Claws could not save your library: {e}"));
            let _ = tokio::fs::remove_file(&path).await;
        }
        // The entry was removed while the artwork downloaded.
        _ => {
            let _ = tokio::fs::remove_file(&path).await;
        }
    }
}

pub fn remove_thumbnail(item: &HistoryItem) {
    if let Some(path) = &item.thumbnail_local {
        let _ = std::fs::remove_file(Path::new(path));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_library_names_the_size_and_a_non_default_preset() {
        let video = |max_height, video_preset| DownloadOptions { max_height, video_preset, ..DownloadOptions::default() };
        assert_eq!(quality(&video(Some(1080), VideoPreset::Compatible)), "1080p", "unchanged for the default");
        assert_eq!(quality(&video(None, VideoPreset::Compatible)), "Best");
        assert_eq!(quality(&video(Some(4320), VideoPreset::Original)), "4320p · Original");
        assert_eq!(quality(&video(None, VideoPreset::Original)), "Best · Original");
        let audio = DownloadOptions { mode: "audio".into(), audio_format: "flac".into(), video_preset: VideoPreset::Original, ..DownloadOptions::default() };
        assert_eq!(quality(&audio), "FLAC");
    }

    #[test]
    fn the_library_names_a_transcript_by_language_kind_and_format() {
        let transcript = DownloadOptions {
            mode: "transcript".into(),
            subtitle_langs: vec!["en".into()],
            auto_subs: true,
            transcript_format: crate::transcript::TranscriptFormat::Txt,
            ..DownloadOptions::default()
        };
        assert_eq!(quality(&transcript), "Transcript · English (auto-generated) · TXT");
        let human = DownloadOptions { auto_subs: false, transcript_format: crate::transcript::TranscriptFormat::Srt, ..transcript };
        assert_eq!(quality(&human), "Transcript · English · SRT");
        let saved = r#"[{"id":"h2","title":"T","url":"https://example.com/v","uploader":null,"duration":1.0,"thumbnail":null,"thumbnailLocal":null,"filePath":"D:/Out/T [en].txt","fileSize":12,"mode":"transcript","quality":"Transcript · English · TXT","finishedAt":2}]"#;
        let items: Vec<HistoryItem> = serde_json::from_str(saved).unwrap();
        assert_eq!(items[0].mode, "transcript");
    }

    use std::fs;

    use crate::download_files::{self, KeptFiles};
    use crate::download_tests::temp_dir;
    use crate::lifecycle_tests::TestApp;

    fn app(tag: &str) -> TestApp {
        let dir = temp_dir(tag);
        let out = dir.join("out");
        fs::create_dir_all(&out).unwrap();
        TestApp::with_data(dir, out, Vec::new())
    }

    /// A download of an older version: `<stem>.mp4` published, its `<stem>.en.srt` withheld
    /// because the user's own file held the name, and nothing in the Library about the kept copy.
    fn orphan(root: &Path, job: &str, stem: &str) -> (download_files::WorkspaceRef, PathBuf) {
        let (reference, mut manifest) = download_files::open_workspace(root, job, 0).unwrap();
        fs::write(reference.path.join(format!("{stem}.mp4")), b"media").unwrap();
        fs::write(reference.path.join(format!("{stem}.en.srt")), b"kept subs").unwrap();
        fs::write(root.join(format!("{stem}.en.srt")), b"the user's own").unwrap();
        manifest.outputs = vec![format!("{stem}.mp4"), format!("{stem}.en.srt")];
        manifest.state = download_files::Publication::Ready;
        download_files::write_manifest(&manifest).unwrap();
        let published = download_files::publish(&mut manifest, &mut |_| true).unwrap();
        (reference, published.media)
    }

    fn entry(id: &str, media: &Path) -> HistoryItem {
        serde_json::from_value(serde_json::json!({
            "id": id, "title": "Old", "url": "https://fixture.invalid/old", "uploader": null, "duration": null,
            "thumbnail": null, "thumbnailLocal": null, "filePath": media.to_string_lossy(), "fileSize": 5,
            "mode": "video", "quality": "Best", "finishedAt": 1
        }))
        .unwrap()
    }

    fn seed(t: &TestApp, items: Vec<HistoryItem>) {
        crate::store::save_snapshot(&t.dir.join("history.json"), || items.clone()).unwrap();
        *t.state().history.lock().unwrap() = items;
    }

    fn kept_of(t: &TestApp, id: &str) -> Option<KeptFiles> {
        t.state().history.lock().unwrap().iter().find(|h| h.id == id).and_then(|h| h.kept_files.clone())
    }

    fn saved_kept(t: &TestApp, id: &str) -> Option<KeptFiles> {
        let text = fs::read_to_string(t.dir.join("history.json")).unwrap_or_default();
        serde_json::from_str::<Vec<HistoryItem>>(&text).unwrap_or_default().into_iter().find(|h| h.id == id).and_then(|h| h.kept_files)
    }

    #[test]
    fn an_older_download_with_a_withheld_file_gets_its_kept_record_in_memory_and_on_disk() {
        let t = app("kept-repair");
        let job = uuid::Uuid::new_v4().to_string();
        let (reference, media) = orphan(&t.out, &job, "clip");
        seed(&t, vec![entry(&job, &media)]);
        assert!(kept_of(&t, &job).is_none());

        let report = repair_kept_files(t.handle());
        assert_eq!((report.attached, report.ignored), (1, 0), "{report:?}");
        assert!(report.inaccessible.is_empty());
        let kept = kept_of(&t, &job).expect("attached in memory");
        assert_eq!(kept, KeptFiles { workspace: reference.clone(), sources: vec!["clip.en.srt".into()] });
        assert_eq!(saved_kept(&t, &job), Some(kept), "and saved with the Library");
        assert_eq!(last_repair(t.handle()), report);
        // Nothing was moved, replaced or deleted.
        assert_eq!(fs::read(reference.path.join("clip.en.srt")).unwrap(), b"kept subs");
        assert_eq!(fs::read(t.out.join("clip.en.srt")).unwrap(), b"the user's own");
        // A second look finds nothing new, and does not count what already has its record as ignored.
        let second = repair_kept_files(t.handle());
        assert_eq!((second.attached, second.ignored), (0, 0), "{second:?}");
    }

    #[test]
    fn an_entry_is_matched_to_its_folder_whatever_the_letter_case_of_the_saved_path() {
        let t = app("kept-case");
        let job = uuid::Uuid::new_v4().to_string();
        let (_, media) = orphan(&t.out, &job, "clip");
        // The setting is spelled as it is on disk; the Library saved the path in other letters.
        let saved = PathBuf::from(media.to_string_lossy().to_uppercase());
        assert_ne!(saved, media);
        seed(&t, vec![entry(&job, &saved)]);
        let report = repair_kept_files(t.handle());
        assert_eq!((report.attached, report.ignored), (1, 0), "{report:?}");
        assert!(kept_of(&t, &job).is_some());
    }

    #[test]
    fn an_entry_without_a_saved_file_is_not_matched_on_its_id_alone() {
        let t = app("kept-no-media");
        let job = uuid::Uuid::new_v4().to_string();
        let (_, media) = orphan(&t.out, &job, "clip");
        let mut item = entry(&job, &media);
        item.file_path = None;
        seed(&t, vec![item]);
        let report = repair_kept_files(t.handle());
        assert_eq!((report.attached, report.ignored), (0, 1), "{report:?}");
        assert!(kept_of(&t, &job).is_none());
    }

    #[test]
    fn work_folders_without_a_matching_entry_or_with_a_marker_that_proves_nothing_are_left_alone() {
        let t = app("kept-ignored");
        // No Library entry at all (the Library was cleared): not adopted, not deleted.
        let stray = uuid::Uuid::new_v4().to_string();
        let (stray_ref, _) = orphan(&t.out, &stray, "stray");
        // An entry, but another media file than the marker published.
        let other = uuid::Uuid::new_v4().to_string();
        let (other_ref, _) = orphan(&t.out, &other, "other");
        seed(&t, vec![entry(&other, &t.out.join("a-different-file.mp4"))]);
        // A marker copied under another job's folder name.
        let honest = uuid::Uuid::new_v4().to_string();
        let (honest_ref, media) = orphan(&t.out, &honest, "honest");
        let impostor = uuid::Uuid::new_v4().to_string();
        let impostor_ref = download_files::WorkspaceRef { path: download_files::workspace_path(&t.out, &impostor, 0), ..honest_ref.clone() };
        fs::create_dir_all(&impostor_ref.path).unwrap();
        fs::copy(honest_ref.path.join(download_files::MARKER), impostor_ref.path.join(download_files::MARKER)).unwrap();
        fs::write(impostor_ref.path.join("honest.en.srt"), b"not theirs").unwrap();
        let mut items = t.state().history.lock().unwrap().clone();
        items.push(entry(&impostor, &media));
        seed(&t, items);

        let report = repair_kept_files(t.handle());
        assert_eq!(report.attached, 0, "{report:?}");
        assert_eq!(report.ignored, 4, "{report:?}");
        for item in t.state().history.lock().unwrap().iter() {
            assert!(item.kept_files.is_none(), "{} adopted a folder it does not own", item.id);
        }
        for path in [stray_ref.path.join("stray.en.srt"), other_ref.path.join("other.en.srt"), honest_ref.path.join("honest.en.srt"), impostor_ref.path.join("honest.en.srt")] {
            assert!(path.is_file(), "{} was deleted", path.display());
        }
    }

    #[test]
    fn an_output_folder_that_cannot_be_read_is_recorded_and_checked_again_on_retry() {
        let t = app("kept-offline");
        let drive = temp_dir("kept-offline-drive").join("not-mounted-yet");
        t.state().settings.lock().unwrap().output_dir = drive.to_string_lossy().into_owned();
        let job = uuid::Uuid::new_v4().to_string();
        // A download that was saved there while the drive was attached.
        seed(&t, vec![entry(&job, &drive.join("clip.mp4"))]);
        let first = repair_kept_files(t.handle());
        assert_eq!(first.inaccessible, [drive.to_string_lossy().into_owned()], "{first:?}");
        assert_eq!(last_repair(t.handle()).inaccessible.len(), 1, "the Library can show it");

        // The drive comes back, with an older download's kept file on it.
        fs::create_dir_all(&drive).unwrap();
        orphan(&drive, &job, "clip");
        let second = repair_kept_files(t.handle());
        assert!(second.inaccessible.is_empty(), "{second:?}");
        assert_eq!(second.attached, 1);
        assert!(kept_of(&t, &job).is_some());
        let _ = fs::remove_dir_all(drive.parent().unwrap());
    }

    /// A drive letter nothing is mounted on, if any.
    fn unmounted_drive() -> Option<PathBuf> {
        ('D'..='Z').map(|letter| PathBuf::from(format!("{letter}:\\"))).find(|root| fs::metadata(root).is_err())
    }

    #[test]
    fn a_drive_that_is_gone_is_recorded_as_unreadable_even_when_only_the_library_knows_it() {
        let Some(drive) = unmounted_drive() else { return };
        let t = app("kept-gone-drive");
        let job = uuid::Uuid::new_v4().to_string();
        // A download saved on a drive that is not attached now; neither the settings nor the Queue name it.
        seed(&t, vec![entry(&job, &drive.join("Videos").join("clip.mp4"))]);
        let report = repair_kept_files(t.handle());
        assert_eq!(report.inaccessible, [drive.to_string_lossy().into_owned()], "{report:?}");
        assert_eq!(last_repair(t.handle()).inaccessible.len(), 1, "the Library can show it");
    }

    #[test]
    fn a_folder_that_was_moved_away_is_not_a_drive_that_is_gone() {
        let t = app("kept-moved");
        let job = uuid::Uuid::new_v4().to_string();
        let moved = temp_dir("kept-moved-away").join("deleted-since");
        seed(&t, vec![entry(&job, &moved.join("clip.mp4"))]);
        let report = repair_kept_files(t.handle());
        assert!(report.inaccessible.is_empty(), "the drive is there, the folder simply is not: {report:?}");
    }

    #[test]
    fn what_the_lookup_of_a_work_folder_says_decides_whether_the_folder_was_really_looked_at() {
        use std::io::ErrorKind::{NotFound, PermissionDenied};
        assert_eq!(classify_lookup(Ok(())), Lookup::Found);
        assert_eq!(classify_lookup(Err(NotFound)), Lookup::Nothing);
        assert_eq!(classify_lookup(Err(PermissionDenied)), Lookup::Unreadable);
    }

    #[test]
    fn a_look_that_found_unreadable_folders_tells_an_open_library_to_read_the_report_again() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use tauri::Listener;
        let t = app("kept-announce");
        let announced = Arc::new(AtomicUsize::new(0));
        let counter = announced.clone();
        t.handle().listen("history-changed", move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        repair_kept_files(t.handle());
        assert_eq!(announced.load(Ordering::SeqCst), 0, "nothing to say");
        let drive = temp_dir("kept-announce-drive").join("not-mounted-yet");
        t.state().settings.lock().unwrap().output_dir = drive.to_string_lossy().into_owned();
        seed(&t, vec![entry(&uuid::Uuid::new_v4().to_string(), &drive.join("clip.mp4"))]);
        let report = repair_kept_files(t.handle());
        assert_eq!(report.inaccessible.len(), 1);
        assert_eq!(announced.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_library_entry_saved_before_presets_still_loads() {
        let saved = r#"[{"id":"h1","title":"T","url":"https://example.com/v","uploader":null,"duration":1.0,"thumbnail":null,"thumbnailLocal":null,"filePath":"D:/Out/T.mp4","fileSize":10,"mode":"video","quality":"4K","finishedAt":1}]"#;
        let items: Vec<HistoryItem> = serde_json::from_str(saved).unwrap();
        assert_eq!(items[0].quality, "4K");
    }
}
