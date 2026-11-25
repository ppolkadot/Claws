//! Wallpaper Engine projects are downloaded by Valve's SteamCMD and exported as ZIPs.
use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use tauri::State;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use tokio::{io::BufReader, process::Command, sync::Mutex, time::timeout};

use crate::{
    bounded_io::read_line_capped,
    download_files::{is_link, move_no_replace},
    error::{AppError, Result},
    state::AppState,
    AppHandle,
};

const APP_ID: u64 = 431960;
static DOWNLOAD: Mutex<()> = Mutex::const_new(());
const STEAM_DETAILS: &str =
    "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/";
const MAX_DETAILS: usize = 256 * 1024;

fn item_id(url: &str) -> Result<u64> {
    let invalid = || AppError::msg("Paste a Steam Workshop item link with a valid item ID.");
    let url = url::Url::parse(url.trim()).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "https" | "http")
        || !matches!(
            url.host_str(),
            Some("steamcommunity.com" | "www.steamcommunity.com")
        )
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || !matches!(
            url.path(),
            "/sharedfiles/filedetails/"
                | "/sharedfiles/filedetails"
                | "/workshop/filedetails/"
                | "/workshop/filedetails"
        )
    {
        return Err(invalid());
    }
    let mut ids = url.query_pairs().filter(|(key, _)| key == "id");
    let (_, id) = ids.next().ok_or_else(invalid)?;
    if ids.next().is_some() || !id.bytes().all(|c| c.is_ascii_digit()) {
        return Err(invalid());
    }
    id.parse().ok().filter(|id| *id != 0).ok_or_else(invalid)
}

fn matches_wallpaper(body: &[u8], id: u64) -> Result<bool> {
    let value: serde_json::Value = serde_json::from_slice(body)?;
    Ok(value["response"]["publishedfiledetails"]
        .as_array()
        .is_some_and(|items| {
            items.iter().any(|item| {
                item["publishedfileid"]
                    .as_str()
                    .and_then(|value| value.parse().ok())
                    == Some(id)
                    && item["result"].as_u64() == Some(1)
                    && item["consumer_app_id"].as_u64() == Some(APP_ID)
            })
        }))
}

async fn read_workshop_kind(id: u64) -> Result<bool> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(8))
        .build()?;
    let mut response = client
        .post(STEAM_DETAILS)
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(format!("itemcount=1&publishedfileids%5B0%5D={id}"))
        .send()
        .await?
        .error_for_status()?;
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if chunk.len() > MAX_DETAILS - body.len() {
            return Err(AppError::msg("Steam returned too much item information."));
        }
        body.extend_from_slice(&chunk);
    }
    matches_wallpaper(&body, id)
}

#[tauri::command]
pub async fn is_wallpaper_workshop_item(url: String) -> Result<bool> {
    read_workshop_kind(item_id(&url)?).await
}

#[tauri::command]
pub fn open_workshop_item(app: AppHandle, url: String) -> Result<()> {
    let id = item_id(&url)?;
    app.opener()
        .open_url(format!("steam://url/CommunityFilePage/{id}"), None::<&str>)
        .map_err(|_| AppError::msg("Couldn't open Steam. Check that Steam is installed."))
}

#[tauri::command]
pub async fn choose_steamcmd(app: AppHandle, state: State<'_, AppState>) -> Result<bool> {
    let (send, receive) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Choose Valve's steamcmd.exe")
        .add_filter("Executable", &["exe"])
        .pick_file(move |file| {
            let _ = send.send(file);
        });
    let file = receive
        .await
        .map_err(|_| AppError::msg("Couldn't open the file picker."))?;
    let Some(file) = file else { return Ok(false) };
    let path = file
        .into_path()
        .map_err(|_| AppError::msg("Choose a local steamcmd.exe."))?;
    validate_steamcmd(&path)?;
    *state.steamcmd.lock().unwrap() = Some(path);
    Ok(true)
}

fn validate_steamcmd(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || !path
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("steamcmd.exe"))
        || !fs::symlink_metadata(path).is_ok_and(|meta| meta.is_file() && !is_link(&meta))
    {
        return Err(AppError::msg("Choose Valve's installed steamcmd.exe."));
    }
    Ok(())
}

#[tauri::command]
pub async fn download_wallpaper(
    state: State<'_, AppState>,
    url: String,
    account: String,
) -> Result<String> {
    let id = item_id(&url)?;
    let account = account.trim();
    if account.is_empty()
        || account.eq_ignore_ascii_case("anonymous")
        || account.len() > 64
        || !account
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
    {
        return Err(AppError::msg(
            "Enter the Steam account name that owns Wallpaper Engine.",
        ));
    }
    let steamcmd = state
        .steamcmd
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| AppError::msg("Choose Valve's installed steamcmd.exe."))?;
    validate_steamcmd(&steamcmd)?;
    let _download = DOWNLOAD
        .try_lock()
        .map_err(|_| AppError::msg("A wallpaper download is already running."))?;
    if !read_workshop_kind(id)
        .await
        .map_err(|_| AppError::msg("Couldn't verify the Workshop item with Steam. Try again."))?
    {
        return Err(AppError::msg(
            "Steam couldn't verify this as a Wallpaper Engine item.",
        ));
    }
    let output = PathBuf::from(state.settings.lock().unwrap().output_dir.clone());
    if !output.is_absolute() {
        return Err(AppError::msg("Choose a download folder in Settings first."));
    }
    fs::create_dir_all(&output)?;
    let output = output.canonicalize()?;
    let steam_dir = steamcmd
        .parent()
        .expect("absolute executable path has a parent");
    let downloaded = run_steamcmd(&steamcmd, steam_dir, id, account).await?;
    let project = checked_project(steam_dir, &downloaded, id)?;
    tauri::async_runtime::spawn_blocking(move || save_project(&project, &output, id)).await?
}

fn reported_project<'a>(line: &'a [u8], marker: &[u8]) -> Option<&'a [u8]> {
    let start = line.windows(marker.len()).position(|part| part == marker)? + marker.len();
    let after = line.get(start..)?;
    let path = &after[..after.iter().position(|byte| *byte == b'"')?];
    (!path.is_empty()).then_some(path)
}

async fn run_steamcmd(exe: &Path, directory: &Path, id: u64, account: &str) -> Result<Vec<u8>> {
    let mut command = Command::new(exe);
    command
        .current_dir(directory)
        .args([
            "+@ShutdownOnFailedCommand",
            "1",
            "+@NoPromptForPassword",
            "1",
        ])
        .args(["+login", account])
        .args([
            "+workshop_download_item",
            &APP_ID.to_string(),
            &id.to_string(),
            "+quit",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let mut child = command
        .spawn()
        .map_err(|_| AppError::msg("Couldn't start SteamCMD."))?;
    let mut stdout = BufReader::new(child.stdout.take().expect("piped stdout"));
    // SteamCMD may be quiet throughout a large download.
    let marker = format!("Success. Downloaded item {id} to \"");
    let result = timeout(Duration::from_secs(90 * 60), async {
        let mut line = Vec::new();
        let mut path = None;
        while let Some(truncated) = read_line_capped(&mut stdout, &mut line, 8192).await {
            if truncated {
                continue;
            }
            if let Some(reported) = reported_project(&line, marker.as_bytes()) {
                path = Some(reported.to_vec());
            }
        }
        let status = child.wait().await?;
        Ok::<_, io::Error>((status, path))
    })
    .await;
    let (status, path) = match result {
        Ok(result) => result?,
        Err(_) => {
            let _ = child.kill().await;
            return Err(AppError::msg("The Steam download timed out. Try again."));
        }
    };
    if status.success() {
        if let Some(path) = path {
            return Ok(path);
        }
    }
    Err(AppError::msg(
        "SteamCMD couldn't download this wallpaper. Sign in through SteamCMD with the account that owns Wallpaper Engine, then try again.",
    ))
}

fn checked_project(steam_dir: &Path, reported: &[u8], id: u64) -> Result<PathBuf> {
    let item = id.to_string();
    let app_id = APP_ID.to_string();
    let mut expected = steam_dir.canonicalize()?;
    for part in ["steamapps", "workshop", "content", &app_id, &item] {
        expected.push(part);
        let meta = fs::symlink_metadata(&expected)?;
        if !meta.is_dir() || is_link(&meta) {
            return Err(AppError::msg(
                "SteamCMD reported an unsafe Workshop folder.",
            ));
        }
    }
    if !matches_reported_path(reported, &expected.canonicalize()?) {
        return Err(AppError::msg(
            "SteamCMD saved the item outside its expected Workshop folder.",
        ));
    }
    Ok(expected)
}

fn matches_reported_path(reported: &[u8], expected: &Path) -> bool {
    let same =
        |path: &Path| path.is_absolute() && path.canonicalize().is_ok_and(|path| path == expected);
    if std::str::from_utf8(reported).is_ok_and(|path| same(Path::new(path))) {
        return true;
    }
    #[cfg(windows)]
    {
        use std::{ffi::OsString, os::windows::ffi::OsStringExt};
        use windows::Win32::Globalization::{
            MultiByteToWideChar, CP_ACP, CP_OEMCP, MB_ERR_INVALID_CHARS,
        };

        // SteamCMD may print a legacy code page instead of UTF-8. Try the
        // Windows encodings, comparing the entire canonical path in either case.
        let mut wide = vec![0u16; reported.len()];
        for code_page in [CP_ACP, CP_OEMCP] {
            let count = unsafe {
                MultiByteToWideChar(code_page, MB_ERR_INVALID_CHARS, reported, Some(&mut wide))
            };
            if count > 0 && same(Path::new(&OsString::from_wide(&wide[..count as usize]))) {
                return true;
            }
        }
    }
    false
}

fn save_project(project: &Path, output: &Path, id: u64) -> Result<String> {
    let meta = fs::symlink_metadata(project.join("project.json"))
        .map_err(|_| AppError::msg("The download doesn't contain a Wallpaper Engine project."))?;
    if !meta.is_file() || meta.len() == 0 || is_link(&meta) {
        return Err(AppError::msg(
            "The download doesn't contain a Wallpaper Engine project.",
        ));
    }
    let partial = output.join(format!(".claws-wallpaper-{}.partial", uuid::Uuid::new_v4()));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)?;
    let result = (|| {
        let mut zip = zip::ZipWriter::new(file);
        let mut entries = 0;
        add_files(&mut zip, project, "", 0, &mut entries)?;
        zip.finish()?.sync_all()?;
        for suffix in 0..1000 {
            let name = if suffix == 0 {
                format!("Wallpaper Engine {id}.zip")
            } else {
                format!("Wallpaper Engine {id} ({suffix}).zip")
            };
            match move_no_replace(&partial, &output.join(&name)) {
                Ok(()) => return Ok(name),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Err(AppError::msg(
            "Too many copies of this wallpaper in the download folder.",
        ))
    })();
    if result.is_err() {
        let _ = fs::remove_file(partial);
    }
    result
}

fn add_files(
    zip: &mut zip::ZipWriter<File>,
    folder: &Path,
    prefix: &str,
    depth: u8,
    entries: &mut usize,
) -> Result<()> {
    if depth > 48 {
        return Err(AppError::msg("This wallpaper has too many nested folders."));
    }
    let mut children = fs::read_dir(folder)?.collect::<io::Result<Vec<_>>>()?;
    children.sort_by_key(|entry| entry.file_name());
    for entry in children {
        *entries += 1;
        if *entries > 50_000 {
            return Err(AppError::msg("This wallpaper has too many files."));
        }
        let path = entry.path();
        let meta = fs::symlink_metadata(&path)?;
        let base = entry.file_name();
        let name = base
            .to_str()
            .filter(|s| !s.contains(['\\', '/', ':']))
            .ok_or_else(|| AppError::msg("A wallpaper file has an unsupported name."))?;
        let name = format!("{prefix}{name}");
        if is_link(&meta) {
            return Err(AppError::msg(
                "The wallpaper contains a linked file or folder.",
            ));
        }
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        if meta.is_dir() {
            let prefix = format!("{name}/");
            zip.add_directory(&prefix, options)?;
            add_files(zip, &path, &prefix, depth + 1, entries)?;
        } else if meta.is_file() {
            zip.start_file(name, options.large_file(meta.len() >= u32::MAX as u64))?;
            io::copy(&mut File::open(path)?, zip)?;
        } else {
            return Err(AppError::msg("The wallpaper contains an unsupported file."));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn classifies_only_the_requested_wallpaper_engine_item() {
        let body = br#"{"response":{"publishedfiledetails":[
            {"publishedfileid":"123","consumer_app_id":431960,"result":1},
            {"publishedfileid":"456","consumer_app_id":480,"result":1},
            {"publishedfileid":"789","consumer_app_id":431960,"result":9}
        ]}}"#;
        assert!(matches_wallpaper(body, 123).unwrap());
        for id in [456, 789, 999] {
            assert!(!matches_wallpaper(body, id).unwrap());
        }
    }

    #[test]
    fn parses_a_single_workshop_id_and_requires_the_same_id_in_steamcmd_output() {
        let url = "https://steamcommunity.com/sharedfiles/filedetails/?id=123&searchtext=blue";
        assert_eq!(item_id(url).unwrap(), 123);
        for invalid in [
            "https://steamcommunity.com.evil.test/sharedfiles/filedetails/?id=123",
            "https://steamcommunity.com/sharedfiles/filedetails/?id=123&id=456",
            "https://steamcommunity.com/workshop/browse/?id=123",
            "https://steamcommunity.com/sharedfiles/filedetails/?id=%2Bquit",
            "https://steamcommunity.com/sharedfiles/filedetails/?id=0",
        ] {
            assert!(item_id(invalid).is_err(), "{invalid}");
        }
        let line = br#"Success. Downloaded item 123 to "C:\SteamCMD\steamapps\workshop\content\431960\123" (42 bytes)"#;
        assert_eq!(
            reported_project(line, b"Success. Downloaded item 123 to \"").unwrap(),
            br"C:\SteamCMD\steamapps\workshop\content\431960\123"
        );
        assert!(reported_project(line, b"Success. Downloaded item 456 to \"").is_none());
    }

    #[test]
    fn exports_the_project_tree_without_replacing_an_existing_backup() {
        let dir = crate::download_tests::temp_dir("workshop-export");
        let project = dir.join("steamapps/workshop/content/431960/123");
        let output = dir.join("output");
        fs::create_dir_all(project.join("assets")).unwrap();
        fs::create_dir(&output).unwrap();
        fs::write(
            project.join("project.json"),
            r#"{"type":"web","file":"assets/index.html"}"#,
        )
        .unwrap();
        fs::write(project.join("assets/index.html"), b"<title>Fixture</title>").unwrap();
        fs::write(output.join("Wallpaper Engine 123.zip"), b"existing").unwrap();

        let project = checked_project(&dir, project.to_str().unwrap().as_bytes(), 123).unwrap();
        assert!(checked_project(&dir, output.to_str().unwrap().as_bytes(), 123).is_err());
        let name = save_project(&project, &output, 123).unwrap();
        assert_eq!(name, "Wallpaper Engine 123 (1).zip");
        assert_eq!(
            fs::read(output.join("Wallpaper Engine 123.zip")).unwrap(),
            b"existing"
        );
        {
            let mut zip = zip::ZipArchive::new(File::open(output.join(name)).unwrap()).unwrap();
            let names: Vec<_> = zip.file_names().map(str::to_owned).collect();
            assert_eq!(names, ["assets/", "assets/index.html", "project.json"]);
            assert!(zip.by_name("project.json").is_ok());
            let mut html = String::new();
            zip.by_name("assets/index.html")
                .unwrap()
                .read_to_string(&mut html)
                .unwrap();
            assert_eq!(html, "<title>Fixture</title>");
        }
        assert_eq!(fs::read_dir(output).unwrap().count(), 2);
        fs::remove_dir_all(dir).unwrap();
    }
}
