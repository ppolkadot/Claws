//! Wallpaper Engine Workshop items. Details come from Valve's public API; the project comes
//! either from the user's own Steam library (an item they subscribed to in Steam) or from
//! Valve's SteamCMD signed in with the account that owns Wallpaper Engine. Either way the
//! project is exported as a ZIP. Claws never writes into Steam's folders, except to remove
//! SteamCMD's own copy when the user asked for that.
use std::{
    collections::VecDeque,
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Read, Write},
    path::{Path, PathBuf},
    process::Stdio,
    sync::atomic::Ordering,
    time::Duration,
};

use serde::Serialize;
use tauri::{Emitter, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use tokio::{
    io::{AsyncRead, BufReader},
    process::Command,
    time::timeout,
};

use crate::{
    bounded_io::{read_line_capped, read_tail},
    disk_budget,
    download_files::{is_link, move_no_replace},
    downloader::kill_tree,
    error::{AppError, Result},
    state::AppState,
    AppHandle,
};

pub const APP_ID: u64 = 431960;
/// Workshop collections are published under Steam's own collection app.
const COLLECTION_APP: u64 = 766;
const STEAM_DETAILS: &str = "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/";
const STEAM_COLLECTION: &str = "https://api.steampowered.com/ISteamRemoteStorage/GetCollectionDetails/v1/";
const MAX_DETAILS: usize = 512 * 1024;
/// SteamCMD may be quiet throughout a large download.
const STEAMCMD_LIMIT: Duration = Duration::from_secs(90 * 60);
/// Output lines kept for the "Show the full message" detail.
const TAIL_LINES: usize = 60;
const MAX_EXPORT_BYTES: u64 = 64 * 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 50_000;
const PARTIAL_PREFIX: &str = ".claws-wallpaper-";
const PARTIAL_SUFFIX: &str = ".partial";

const STEAM_UNREACHABLE: &str = "Couldn't reach Steam to check this Workshop item. Check your connection, then try again.";
const RUNNING: &str = "A wallpaper download is already running.";

/// The HTTP client for Valve's public Workshop API: no redirects, short timeouts.
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("Claws/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("Steam HTTP client configuration is valid")
}

/// The one export that may run at a time. Leaf lock.
#[derive(Default)]
pub struct Run {
    pub active: bool,
    pub cancelled: bool,
    /// SteamCMD's process id while it runs.
    pub pid: Option<u32>,
}

// ---------------------------------------------------------------------------------------
// Links and details

fn item_id(url: &str) -> Result<u64> {
    let invalid = || AppError::msg("Paste a Steam Workshop item link with a valid item ID.");
    let url = url::Url::parse(url.trim()).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "https" | "http")
        || !matches!(url.host_str(), Some("steamcommunity.com" | "www.steamcommunity.com"))
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || !matches!(
            url.path(),
            "/sharedfiles/filedetails/" | "/sharedfiles/filedetails" | "/workshop/filedetails/" | "/workshop/filedetails"
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

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkshopItem {
    pub id: String,
    pub title: String,
    pub preview_url: Option<String>,
    pub file_size: Option<u64>,
    pub time_updated: Option<i64>,
    /// "Scene", "Video", "Web", "Application" or "Preset", from the item's tags.
    pub wallpaper_type: Option<String>,
    /// Other Workshop items this one needs (asset packs and the like).
    pub required_items: Vec<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum WorkshopLookup {
    Wallpaper { item: WorkshopItem },
    Collection { title: String, items: usize },
    OtherApp { app_id: u64 },
    NotFound,
}

/// Steam writes ids and sizes as numbers in some fields and as strings in others.
fn number(value: &serde_json::Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str().and_then(|s| s.parse().ok()))
}

fn preview_url(value: &serde_json::Value) -> Option<String> {
    let url = url::Url::parse(value.as_str()?).ok()?;
    let host = url.host_str()?;
    let valve = ["steamusercontent.com", "steamstatic.com", "akamaihd.net", "steampowered.com"]
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")));
    (url.scheme() == "https" && valve).then(|| url.to_string())
}

fn parse_details(body: &[u8], id: u64) -> Result<WorkshopLookup> {
    let value: serde_json::Value = serde_json::from_slice(body)?;
    let Some(item) = value["response"]["publishedfiledetails"]
        .as_array()
        .and_then(|items| items.iter().find(|item| number(&item["publishedfileid"]) == Some(id)))
    else {
        return Ok(WorkshopLookup::NotFound);
    };
    if item["result"].as_u64() != Some(1) {
        return Ok(WorkshopLookup::NotFound);
    }
    let consumer = number(&item["consumer_app_id"]).unwrap_or(0);
    if consumer != APP_ID {
        return Ok(WorkshopLookup::OtherApp { app_id: consumer });
    }
    let title = item["title"].as_str().unwrap_or("").trim().to_string();
    if number(&item["creator_app_id"]) == Some(COLLECTION_APP) {
        return Ok(WorkshopLookup::Collection { title, items: 0 });
    }
    let wallpaper_type = item["tags"].as_array().and_then(|tags| {
        tags.iter()
            .filter_map(|tag| tag["tag"].as_str())
            .find(|tag| ["Scene", "Video", "Web", "Application", "Preset"].contains(tag))
            .map(str::to_string)
    });
    Ok(WorkshopLookup::Wallpaper {
        item: WorkshopItem {
            id: id.to_string(),
            title,
            preview_url: preview_url(&item["preview_url"]),
            file_size: number(&item["file_size"]).filter(|size| *size > 0),
            time_updated: item["time_updated"].as_i64(),
            wallpaper_type,
            required_items: Vec::new(),
        },
    })
}

/// The children Steam lists for an item: a collection's items, or a wallpaper's required items.
fn parse_children(body: &[u8], id: u64) -> Vec<String> {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else { return Vec::new() };
    value["response"]["collectiondetails"]
        .as_array()
        .and_then(|items| items.iter().find(|item| number(&item["publishedfileid"]) == Some(id)))
        .filter(|item| item["result"].as_u64() == Some(1))
        .and_then(|item| item["children"].as_array())
        .map(|children| {
            children
                .iter()
                .filter_map(|child| number(&child["publishedfileid"]))
                .filter(|child| *child != id)
                .map(|child| child.to_string())
                .collect()
        })
        .unwrap_or_default()
}

async fn post(client: &reqwest::Client, url: &str, body: String) -> Result<Vec<u8>> {
    let mut response = client
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await?
        .error_for_status()?;
    let mut out = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if chunk.len() > MAX_DETAILS - out.len() {
            return Err(AppError::msg("Steam returned too much item information."));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

async fn lookup(client: &reqwest::Client, id: u64) -> Result<WorkshopLookup> {
    let details = post(client, STEAM_DETAILS, format!("itemcount=1&publishedfileids%5B0%5D={id}")).await?;
    let mut found = parse_details(&details, id)?;
    if matches!(found, WorkshopLookup::Wallpaper { .. } | WorkshopLookup::Collection { .. }) {
        // Optional: without it a wallpaper still downloads, only without the warning.
        let children = post(client, STEAM_COLLECTION, format!("collectioncount=1&publishedfileids%5B0%5D={id}"))
            .await
            .map(|body| parse_children(&body, id))
            .unwrap_or_default();
        match &mut found {
            WorkshopLookup::Wallpaper { item } => item.required_items = children,
            WorkshopLookup::Collection { items, .. } => *items = children.len(),
            _ => {}
        }
    }
    Ok(found)
}

fn not_a_wallpaper(found: &WorkshopLookup) -> AppError {
    AppError::msg(match found {
        WorkshopLookup::Collection { .. } => "This link is a Workshop collection. Open each wallpaper in it on its own.",
        WorkshopLookup::OtherApp { .. } => "Claws downloads Workshop items for Wallpaper Engine only.",
        _ => "Steam couldn't find this Workshop item. It may be private or removed.",
    })
}

#[tauri::command]
pub async fn workshop_item_details(state: State<'_, AppState>, url: String) -> Result<WorkshopLookup> {
    let id = item_id(&url)?;
    lookup(&state.steam_http, id).await.map_err(|_| AppError::msg(STEAM_UNREACHABLE))
}

#[tauri::command]
pub fn open_workshop_item(app: AppHandle, url: String) -> Result<()> {
    let id = item_id(&url)?;
    if app.opener().open_url(format!("steam://url/CommunityFilePage/{id}"), None::<&str>).is_ok() {
        return Ok(());
    }
    // No Steam client: the item's page in the browser.
    app.opener()
        .open_url(format!("https://steamcommunity.com/sharedfiles/filedetails/?id={id}"), None::<&str>)
        .map_err(|_| AppError::msg("Couldn't open the Workshop page."))
}

// ---------------------------------------------------------------------------------------
// SteamCMD location and account

/// Steam account names use letters, digits and underscores.
pub fn check_account(account: &str) -> Result<String> {
    let account = account.trim();
    if account.is_empty()
        || account.eq_ignore_ascii_case("anonymous")
        || account.len() > 64
        || !account.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
    {
        return Err(AppError::msg("Enter the Steam account name that owns Wallpaper Engine."));
    }
    Ok(account.to_string())
}

/// Whether `path` sits in a OneDrive folder, whose placeholders SteamCMD cannot use.
fn in_sync_folder(path: &Path) -> bool {
    let lower = |p: &Path| p.to_string_lossy().replace('/', "\\").to_lowercase();
    let path = lower(path);
    ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"]
        .iter()
        .filter_map(std::env::var_os)
        .map(|root| lower(Path::new(&root)))
        .filter(|root| !root.is_empty())
        .any(|root| path == root || path.starts_with(&format!("{}\\", root.trim_end_matches('\\'))))
}

/// The checks every use of a SteamCMD path passes: Valve's signed steamcmd.exe, a plain
/// file, outside OneDrive.
pub fn validate_steamcmd(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || !path
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("steamcmd.exe"))
        || !fs::symlink_metadata(path).is_ok_and(|meta| meta.is_file() && !is_link(&meta))
    {
        return Err(AppError::msg("Choose Valve's installed SteamCMD (steamcmd.exe)."));
    }
    if in_sync_folder(path) {
        return Err(AppError::msg(
            "SteamCMD can't run from OneDrive. Move its folder somewhere else, such as C:\\SteamCMD, then choose it again.",
        ));
    }
    if !signed_by_valve(path) {
        return Err(AppError::msg("This steamcmd.exe isn't signed by Valve. Download SteamCMD from Valve again."));
    }
    Ok(())
}

/// Authenticode: a valid signature whose signer's organisation is Valve.
#[cfg(windows)]
pub(crate) fn signed_by_valve(path: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Security::Cryptography::{CertGetNameStringW, CERT_NAME_ATTR_TYPE};
    use windows::Win32::Security::WinTrust::{
        WTHelperGetProvSignerFromChain, WTHelperProvDataFromStateData, WinVerifyTrust, WINTRUST_ACTION_GENERIC_VERIFY_V2,
        WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO, WTD_CHOICE_FILE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE,
        WTD_STATEACTION_VERIFY, WTD_UI_NONE,
    };

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut file = WINTRUST_FILE_INFO {
        cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: PCWSTR(wide.as_ptr()),
        ..Default::default()
    };
    let mut data = WINTRUST_DATA {
        cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_NONE,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 { pFile: &mut file },
        dwStateAction: WTD_STATEACTION_VERIFY,
        ..Default::default()
    };
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    let verify = |data: &mut WINTRUST_DATA, action: &mut windows::core::GUID| unsafe {
        WinVerifyTrust(HWND(std::ptr::null_mut()), action, data as *mut WINTRUST_DATA as *mut _)
    };
    let valid = verify(&mut data, &mut action) == 0;
    let mut organisation = String::new();
    if valid {
        unsafe {
            let provider = WTHelperProvDataFromStateData(data.hWVTStateData);
            let signer = WTHelperGetProvSignerFromChain(provider, 0, false, 0);
            if !signer.is_null() && !(*signer).pasCertChain.is_null() {
                let cert = (*(*signer).pasCertChain).pCert;
                // szOID_ORGANIZATION_NAME
                let oid = b"2.5.4.10\0";
                let mut buf = [0u16; 256];
                let len = CertGetNameStringW(cert, CERT_NAME_ATTR_TYPE, 0, Some(oid.as_ptr() as *const _), Some(&mut buf));
                if len > 1 {
                    organisation = String::from_utf16_lossy(&buf[..len as usize - 1]);
                }
            }
        }
    }
    data.dwStateAction = WTD_STATEACTION_CLOSE;
    verify(&mut data, &mut action);
    valid && organisation == "Valve"
}

#[cfg(not(windows))]
pub(crate) fn signed_by_valve(_path: &Path) -> bool {
    false
}

#[tauri::command]
pub async fn choose_steamcmd(app: AppHandle) -> Result<Option<String>> {
    let (send, receive) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Choose Valve's steamcmd.exe")
        .add_filter("SteamCMD", &["exe"])
        .pick_file(move |file| {
            let _ = send.send(file);
        });
    let file = receive.await.map_err(|_| AppError::msg("Couldn't open the file picker."))?;
    let Some(file) = file else { return Ok(None) };
    let path = file.into_path().map_err(|_| AppError::msg("Choose a local steamcmd.exe."))?;
    let checked = path.clone();
    tauri::async_runtime::spawn_blocking(move || validate_steamcmd(&checked)).await??;
    Ok(Some(path.to_string_lossy().into_owned()))
}

/// Opens SteamCMD in its own console signed in as `account`, so the password and any Steam
/// Guard code are typed there. SteamCMD keeps the sign-in for later downloads.
#[tauri::command]
pub async fn sign_in_steamcmd(state: State<'_, AppState>, account: String) -> Result<()> {
    let account = check_account(&account)?;
    let steamcmd = steamcmd_path(&state)?;
    let exe = steamcmd.clone();
    tauri::async_runtime::spawn_blocking(move || validate_steamcmd(&exe)).await??;
    let mut command = std::process::Command::new(&steamcmd);
    command
        .current_dir(steamcmd.parent().expect("absolute executable path has a parent"))
        .args(["+login", &account]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0000_0010); // CREATE_NEW_CONSOLE
    }
    command.spawn().map_err(|_| AppError::msg("Couldn't start SteamCMD."))?;
    Ok(())
}

/// The SteamCMD to use: the one chosen in Settings, else the copy Claws installed.
fn steamcmd_path(state: &AppState) -> Result<PathBuf> {
    crate::tools::find_steamcmd(state).ok_or_else(|| AppError::msg("Install SteamCMD first, or choose your own copy."))
}

// ---------------------------------------------------------------------------------------
// The Steam account name, found on this PC

/// One value of a Valve KeyValues (VDF) file: text, or a block of named values.
#[derive(Debug, PartialEq)]
enum Vdf {
    Text(String),
    Block(Vec<(String, Vdf)>),
}

/// Parses VDF text: quoted strings (with backslash escapes), braces and `//` comments.
/// Malformed input yields whatever parsed before the problem.
fn parse_vdf(text: &str) -> Vec<(String, Vdf)> {
    enum Token {
        Text(String),
        Open,
        Close,
    }
    let mut tokens = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' => tokens.push(Token::Open),
            '}' => tokens.push(Token::Close),
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            '"' => {
                let mut token = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => match chars.next() {
                            Some('n') => token.push('\n'),
                            Some('t') => token.push('\t'),
                            Some(other) => token.push(other),
                            None => break,
                        },
                        '"' => break,
                        other => token.push(other),
                    }
                }
                tokens.push(Token::Text(token));
            }
            _ => {}
        }
    }
    fn block(tokens: &mut std::iter::Peekable<std::vec::IntoIter<Token>>, depth: usize) -> Vec<(String, Vdf)> {
        let mut out = Vec::new();
        while let Some(token) = tokens.next() {
            let key = match token {
                Token::Text(key) => key,
                Token::Close => break,
                Token::Open => continue,
            };
            match tokens.next() {
                Some(Token::Text(value)) => out.push((key, Vdf::Text(value))),
                Some(Token::Open) if depth < 32 => out.push((key, Vdf::Block(block(tokens, depth + 1)))),
                _ => break,
            }
        }
        out
    }
    block(&mut tokens.into_iter().peekable(), 0)
}

/// Every block named `name` (case-insensitive), at any depth.
fn find_blocks<'a>(entries: &'a [(String, Vdf)], name: &str, out: &mut Vec<&'a [(String, Vdf)]>) {
    for (key, value) in entries {
        if let Vdf::Block(children) = value {
            if key.eq_ignore_ascii_case(name) {
                out.push(children);
            }
            find_blocks(children, name, out);
        }
    }
}

/// Accounts SteamCMD itself has signed in with (`config/config.vdf`, `Accounts` block).
fn steamcmd_accounts(config_vdf: &str) -> Vec<String> {
    let entries = parse_vdf(config_vdf);
    let mut blocks = Vec::new();
    find_blocks(&entries, "Accounts", &mut blocks);
    blocks.into_iter().flat_map(|accounts| accounts.iter().map(|(name, _)| name.clone())).collect()
}

/// Accounts the Steam client remembers (`config/loginusers.vdf`), the most recent first.
fn steam_client_accounts(loginusers_vdf: &str) -> Vec<String> {
    let entries = parse_vdf(loginusers_vdf);
    let mut blocks = Vec::new();
    find_blocks(&entries, "users", &mut blocks);
    let mut users: Vec<(bool, String)> = blocks
        .into_iter()
        .flat_map(|users| users.iter())
        .filter_map(|(_, user)| match user {
            Vdf::Block(fields) => {
                let field = |name: &str| {
                    fields.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).and_then(|(_, value)| match value {
                        Vdf::Text(text) => Some(text.as_str()),
                        Vdf::Block(_) => None,
                    })
                };
                Some((field("MostRecent") == Some("1"), field("AccountName")?.to_string()))
            }
            Vdf::Text(_) => None,
        })
        .collect();
    users.sort_by_key(|(recent, _)| !*recent);
    users.into_iter().map(|(_, name)| name).collect()
}

/// The Steam client's remembered automatic sign-in.
#[cfg(windows)]
fn steam_auto_login_user() -> Option<String> {
    use windows::core::w;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
    let mut buf = [0u16; 256];
    let mut size = (buf.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Valve\\Steam"),
            w!("AutoLoginUser"),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut size),
        )
    };
    if status.is_err() {
        return None;
    }
    let len = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

#[cfg(not(windows))]
fn steam_auto_login_user() -> Option<String> {
    None
}

/// Usable account names in order, each once: those SteamCMD already signed in with (they
/// download without a new sign-in), then the Steam client's most recent, its automatic
/// sign-in, and the rest it remembers.
fn merge_accounts(groups: impl IntoIterator<Item = Vec<String>>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for name in groups.into_iter().flatten() {
        let Ok(name) = check_account(&name) else { continue };
        if !out.iter().any(|known| known.eq_ignore_ascii_case(&name)) {
            out.push(name);
        }
    }
    out
}

/// Steam account names found on this PC, best first. Nothing leaves the PC.
#[tauri::command]
pub async fn detect_steam_accounts(state: State<'_, AppState>) -> Result<Vec<String>> {
    let steamcmd = crate::tools::find_steamcmd(&state);
    tauri::async_runtime::spawn_blocking(move || {
        let read = |path: PathBuf| fs::read_to_string(path).unwrap_or_default();
        let from_steamcmd = steamcmd
            .and_then(|exe| exe.parent().map(|dir| dir.join("config").join("config.vdf")))
            .map(|path| steamcmd_accounts(&read(path)))
            .unwrap_or_default();
        let client = steam_root().map(|root| steam_client_accounts(&read(root.join("config").join("loginusers.vdf")))).unwrap_or_default();
        let (recent, rest) = client.split_first().map(|(first, rest)| (vec![first.clone()], rest.to_vec())).unwrap_or_default();
        merge_accounts([from_steamcmd, recent, steam_auto_login_user().into_iter().collect(), rest])
    })
    .await
    .map_err(|_| AppError::msg("Couldn't look for Steam accounts."))
}

// ---------------------------------------------------------------------------------------
// The user's Steam library

/// Library folder paths listed in Steam's `libraryfolders.vdf`.
pub fn parse_library_paths(text: &str) -> Vec<PathBuf> {
    let mut tokens = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '"' {
            continue;
        }
        let mut token = String::new();
        while let Some(c) = chars.next() {
            match c {
                '\\' => match chars.next() {
                    Some('n') => token.push('\n'),
                    Some('t') => token.push('\t'),
                    Some(other) => token.push(other),
                    None => break,
                },
                '"' => break,
                other => token.push(other),
            }
        }
        tokens.push(token);
    }
    let mut paths = Vec::new();
    let mut i = 0;
    while i + 1 < tokens.len() {
        if tokens[i].eq_ignore_ascii_case("path") && !tokens[i + 1].is_empty() {
            paths.push(PathBuf::from(&tokens[i + 1]));
            i += 2;
        } else {
            i += 1;
        }
    }
    paths
}

/// Steam's install folder from the registry.
#[cfg(windows)]
fn steam_root() -> Option<PathBuf> {
    use windows::core::w;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
    let mut buf = [0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Valve\\Steam"),
            w!("SteamPath"),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut size),
        )
    };
    if status.is_err() {
        return None;
    }
    let len = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    let path = PathBuf::from(String::from_utf16_lossy(&buf[..len]));
    path.is_absolute().then_some(path)
}

#[cfg(not(windows))]
fn steam_root() -> Option<PathBuf> {
    None
}

/// Every Steam library folder: Steam's own folder first, then the ones it lists.
pub fn library_folders(root: &Path) -> Vec<PathBuf> {
    let mut folders = vec![root.to_path_buf()];
    if let Ok(text) = fs::read_to_string(root.join("steamapps").join("libraryfolders.vdf")) {
        for path in parse_library_paths(&text) {
            let key = |p: &Path| p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase();
            if path.is_absolute() && !folders.iter().any(|known| key(known) == key(&path)) {
                folders.push(path);
            }
        }
    }
    folders
}

/// A subscribed item's project folder in one of the libraries.
pub fn library_project(libraries: &[PathBuf], id: u64) -> Option<PathBuf> {
    libraries
        .iter()
        .filter_map(|library| workshop_folder(library, id).ok())
        .find(|folder| fs::symlink_metadata(folder.join("project.json")).is_ok_and(|m| m.is_file() && !is_link(&m)))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryStatus {
    /// Steam is installed for this Windows user.
    pub steam: bool,
    /// The item's project is in a Steam library.
    pub found: bool,
}

#[tauri::command]
pub async fn find_wallpaper_in_library(url: String) -> Result<LibraryStatus> {
    let id = item_id(&url)?;
    tauri::async_runtime::spawn_blocking(move || {
        let Some(root) = steam_root() else { return LibraryStatus { steam: false, found: false } };
        LibraryStatus { steam: true, found: library_project(&library_folders(&root), id).is_some() }
    })
    .await
    .map_err(|_| AppError::msg("Couldn't check the Steam library."))
}

// ---------------------------------------------------------------------------------------
// Download

#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Stage {
    Starting,
    Updating,
    SigningIn,
    Downloading,
    Saving,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Progress {
    url: String,
    stage: Stage,
    percent: Option<f64>,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(tag = "status", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum WallpaperOutcome {
    Saved { filename: String, path: String },
    Failed { message: String, detail: Option<String> },
    Cancelled,
}

/// Marks the one export as running until dropped.
struct RunGuard<'a>(&'a AppState);

impl<'a> RunGuard<'a> {
    fn begin(state: &'a AppState) -> Result<Self> {
        let mut run = state.workshop.lock().unwrap();
        if run.active {
            return Err(AppError::msg(RUNNING));
        }
        if state.shutting_down.load(Ordering::SeqCst) {
            return Err(AppError::msg("Claws is closing."));
        }
        *run = Run { active: true, cancelled: false, pid: None };
        Ok(Self(state))
    }

    fn cancelled(&self) -> bool {
        self.0.workshop.lock().unwrap().cancelled || self.0.shutting_down.load(Ordering::SeqCst)
    }
}

impl Drop for RunGuard<'_> {
    fn drop(&mut self) {
        *self.0.workshop.lock().unwrap() = Run::default();
    }
}

#[tauri::command]
pub fn cancel_wallpaper_download(state: State<'_, AppState>) {
    let pid = {
        let mut run = state.workshop.lock().unwrap();
        if !run.active {
            return;
        }
        run.cancelled = true;
        run.pid
    };
    if let Some(pid) = pid {
        kill_tree(pid, false);
    }
}

/// For shutdown: the SteamCMD process to stop, if one runs.
pub fn stop_for_shutdown(state: &AppState) -> Option<u32> {
    let mut run = state.workshop.lock().unwrap();
    run.cancelled = true;
    run.pid
}

#[tauri::command]
pub async fn download_wallpaper(
    app: AppHandle,
    state: State<'_, AppState>,
    url: String,
    source: String,
    account: Option<String>,
) -> Result<WallpaperOutcome> {
    let id = item_id(&url)?;
    let from_library = match source.as_str() {
        "library" => true,
        "steamcmd" => false,
        _ => return Err(AppError::msg("Unknown wallpaper source.")),
    };
    let account = if from_library { None } else { Some(check_account(account.as_deref().unwrap_or(""))?) };
    let run = RunGuard::begin(&state)?;
    let progress = |stage: Stage, percent: Option<f64>| {
        let _ = app.emit("workshop-progress", Progress { url: url.clone(), stage, percent });
    };
    progress(Stage::Starting, None);

    let found = lookup(&state.steam_http, id).await.map_err(|_| AppError::msg(STEAM_UNREACHABLE))?;
    let WorkshopLookup::Wallpaper { item } = found else { return Err(not_a_wallpaper(&found)) };

    let output = PathBuf::from(state.settings.lock().unwrap().output_dir.clone());
    if !output.is_absolute() {
        return Err(AppError::msg("Choose a download folder in Settings first."));
    }
    let output = tauri::async_runtime::spawn_blocking(move || -> Result<PathBuf> {
        fs::create_dir_all(&output)?;
        Ok(output.canonicalize()?)
    })
    .await??;

    let (project, steamcmd_copy) = if from_library {
        let project = tauri::async_runtime::spawn_blocking(move || steam_root().and_then(|root| library_project(&library_folders(&root), id)))
            .await
            .map_err(|_| AppError::msg("Couldn't check the Steam library."))?;
        let Some(project) = project else {
            return Ok(WallpaperOutcome::Failed {
                message: "This wallpaper isn't in your Steam library yet. Subscribe to it in Steam, wait for Steam to download it, then try again.".into(),
                detail: None,
            });
        };
        (project, false)
    } else {
        let steamcmd = steamcmd_path(&state)?;
        if state.tool_install.try_lock().is_err() {
            return Err(AppError::msg("A tool install is running. Try again when it finishes."));
        }
        let exe = steamcmd.clone();
        let steam_dir = tauri::async_runtime::spawn_blocking(move || -> Result<PathBuf> {
            validate_steamcmd(&exe)?;
            Ok(exe.parent().expect("absolute executable path has a parent").canonicalize()?)
        })
        .await??;
        if let Some(size) = item.file_size {
            if let Some(message) = space_shortfall(&state, &steam_dir, size, "the drive that holds SteamCMD") {
                return Ok(WallpaperOutcome::Failed { message, detail: None });
            }
        }
        let account = account.expect("checked above");
        let reported = match run_steamcmd(&state, &run, &steamcmd, &steam_dir, id, &account, &progress).await? {
            SteamRun::Done(reported) => reported,
            SteamRun::Cancelled => return Ok(WallpaperOutcome::Cancelled),
            SteamRun::Failed { message, detail } => {
                let detail = redact(&detail, &account);
                return Ok(WallpaperOutcome::Failed { message, detail: (!detail.trim().is_empty()).then_some(detail) });
            }
        };
        let root = steam_dir.clone();
        let project = tauri::async_runtime::spawn_blocking(move || checked_project(&root, &reported, id)).await??;
        (project, true)
    };

    let measured = project.clone();
    let needed = tauri::async_runtime::spawn_blocking(move || disk_budget::bytes_in(&measured)).await.unwrap_or(0);
    if let Some(message) = space_shortfall(&state, &output, needed, "the download folder's drive") {
        return Ok(WallpaperOutcome::Failed { message, detail: None });
    }

    progress(Stage::Saving, Some(0.0));
    let title = item.title.clone();
    let export_project = project.clone();
    let export_output = output.clone();
    let cancel_state = app.clone();
    let progress_app = app.clone();
    let progress_url = url.clone();
    let saved = tauri::async_runtime::spawn_blocking(move || {
        use tauri::Manager;
        let state = cancel_state.state::<AppState>();
        let cancelled = || state.workshop.lock().unwrap().cancelled || state.shutting_down.load(Ordering::SeqCst);
        let mut last = -1.0;
        let report = |fraction: f64| {
            let percent = (fraction * 100.0).floor();
            if percent > last {
                last = percent;
                let _ = progress_app.emit("workshop-progress", Progress { url: progress_url.clone(), stage: Stage::Saving, percent: Some(percent) });
            }
        };
        save_project(&export_project, &export_output, id, &title, report, cancelled)
    })
    .await?;
    let filename = match saved {
        Ok(Some(filename)) => filename,
        Ok(None) => return Ok(WallpaperOutcome::Cancelled),
        Err(error) => return Err(error),
    };
    if steamcmd_copy && state.settings.lock().unwrap().remove_steamcmd_copy {
        let _ = tauri::async_runtime::spawn_blocking(move || fs::remove_dir_all(project)).await;
    }
    drop(run);
    Ok(WallpaperOutcome::Saved { path: output.join(&filename).to_string_lossy().into_owned(), filename })
}

fn space_shortfall(state: &AppState, folder: &Path, bytes: u64, place: &str) -> Option<String> {
    let free = (state.free_space)(folder).ok()?;
    disk_budget::check(bytes, free, 0).err().map(|short| {
        format!(
            "There isn't enough free space on {place}. It needs {} more. Free up space, then try again.",
            format_bytes(short.required.saturating_sub(short.available))
        )
    })
}

fn format_bytes(bytes: u64) -> String {
    let units = ["bytes", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < units.len() {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} bytes")
    } else {
        format!("{value:.1} {}", units[unit])
    }
}

enum SteamRun {
    Done(Vec<u8>),
    Failed { message: String, detail: String },
    Cancelled,
}

fn reported_project<'a>(line: &'a [u8], marker: &[u8]) -> Option<&'a [u8]> {
    let start = line.windows(marker.len()).position(|part| part == marker)? + marker.len();
    let after = line.get(start..)?;
    let path = &after[..after.iter().position(|byte| *byte == b'"')?];
    (!path.is_empty()).then_some(path)
}

/// SteamCMD's own update progress ("[ 45%] Downloading update ..."), for the installer.
pub(crate) fn update_percent(line: &str) -> Option<f64> {
    match progress_of(line) {
        Some((Stage::Updating, percent)) => percent,
        _ => None,
    }
}

/// What a SteamCMD output line says about progress.
fn progress_of(line: &str) -> Option<(Stage, Option<f64>)> {
    let percent = line
        .trim_start()
        .strip_prefix('[')
        .and_then(|rest| rest.split_once("%]"))
        .and_then(|(number, _)| number.trim().parse::<f64>().ok());
    let lower = line.to_ascii_lowercase();
    if ["checking for available update", "downloading update", "verifying installation", "extracting package", "installing update"]
        .iter()
        .any(|text| lower.contains(text))
    {
        Some((Stage::Updating, percent))
    } else if lower.contains("logging in") || lower.contains("waiting for user info") {
        Some((Stage::SigningIn, None))
    } else if lower.contains("downloading item") {
        Some((Stage::Downloading, None))
    } else {
        None
    }
}

const SIGNED_OUT: &str = "SteamCMD isn't signed in with this account. Use Sign in to Steam, then try again.";

/// The message for SteamCMD's output when no project was reported.
fn classify(output: &str) -> &'static str {
    let text = output.to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|needle| text.contains(needle));
    if has(&["invalid password"]) {
        "Steam rejected the sign-in for this account. Use Sign in to Steam to enter the password again, then try again."
    } else if has(&["account logon denied", "two-factor", "steam guard", "steamguard"]) {
        "Steam needs a Steam Guard code for this account. Use Sign in to Steam to enter it, then try again."
    } else if has(&["rate limit"]) {
        "Steam is limiting sign-ins for now. Wait a few minutes, then try again."
    } else if has(&["cached credentials not found", "no cached credentials", "login failure", "failed to log"]) {
        SIGNED_OUT
    } else if has(&["no subscription", "access denied"]) {
        "This Steam account can't download this item. Check that it owns Wallpaper Engine and that the item is still public."
    } else if has(&["file not found"]) {
        "Steam couldn't find the files for this item. It may have been removed."
    } else if has(&["disk write failure", "not enough space", "disk full"]) {
        "There isn't enough free space for SteamCMD to save this wallpaper. Free up space on its drive, then try again."
    } else if has(&["(timeout)", "timed out"]) {
        "Steam took too long to respond. Try again."
    } else {
        "SteamCMD couldn't download this wallpaper. Check that the account owns Wallpaper Engine and is signed in, then try again."
    }
}

/// Replaces `needle` in `text` ignoring ASCII case (byte lengths stay the same).
fn replace_ignore_case(text: &str, needle: &str, with: &str) -> String {
    if needle.is_empty() {
        return text.to_string();
    }
    let lower = text.to_ascii_lowercase();
    let needle = needle.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while let Some(found) = lower[at..].find(&needle) {
        out.push_str(&text[at..at + found]);
        out.push_str(with);
        at += found + needle.len();
    }
    out.push_str(&text[at..]);
    out
}

/// SteamCMD's output without the account name or the user's home folder.
fn redact_with(text: &str, account: &str, home: Option<&str>) -> String {
    let mut out = text.to_string();
    if let Some(home) = home.filter(|home| home.len() > 3) {
        out = replace_ignore_case(&out, home, "%USERPROFILE%");
        out = replace_ignore_case(&out, &home.replace('\\', "/"), "%USERPROFILE%");
    }
    if account.len() >= 3 {
        out = replace_ignore_case(&out, account, "<account>");
    }
    out
}

fn redact(text: &str, account: &str) -> String {
    let home = std::env::var("USERPROFILE").ok();
    redact_with(text, account, home.as_deref())
}

async fn run_steamcmd(
    state: &AppState,
    run: &RunGuard<'_>,
    exe: &Path,
    directory: &Path,
    id: u64,
    account: &str,
    progress: &(dyn Fn(Stage, Option<f64>) + Sync),
) -> Result<SteamRun> {
    let mut command = Command::new(exe);
    command
        .current_dir(directory)
        .args(["+@ShutdownOnFailedCommand", "1", "+@NoPromptForPassword", "1"])
        .args(["+login", account])
        .args(["+workshop_download_item", &APP_ID.to_string(), &id.to_string(), "+quit"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let mut child = command.spawn().map_err(|_| AppError::msg("Couldn't start SteamCMD."))?;
    let pid = child.id();
    {
        let mut current = state.workshop.lock().unwrap();
        if current.cancelled || state.shutting_down.load(Ordering::SeqCst) {
            drop(current);
            if let Some(pid) = pid {
                kill_tree(pid, true);
            }
            return Ok(SteamRun::Cancelled);
        }
        current.pid = pid;
    }
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let marker = format!("Success. Downloaded item {id} to \"");
    let errors = tokio::spawn(read_tail(stderr, 16 * 1024));
    let result = timeout(STEAMCMD_LIMIT, read_output(stdout, marker.as_bytes(), progress)).await;
    let (reported, mut tail) = match result {
        Ok(read) => read,
        Err(_) => {
            if let Some(pid) = pid {
                kill_tree(pid, true);
            }
            let _ = child.kill().await;
            state.workshop.lock().unwrap().pid = None;
            if run.cancelled() {
                return Ok(SteamRun::Cancelled);
            }
            return Ok(SteamRun::Failed { message: "Steam took too long to download this wallpaper. Try again.".into(), detail: String::new() });
        }
    };
    let _ = child.wait().await;
    state.workshop.lock().unwrap().pid = None;
    if let Ok(errors) = errors.await {
        let errors = String::from_utf8_lossy(&errors);
        tail.extend(errors.lines().map(str::to_string));
    }
    if run.cancelled() {
        return Ok(SteamRun::Cancelled);
    }
    // The success line is the answer. SteamCMD's exit code is not reliable after it updates
    // itself, so it is not used.
    if let Some(reported) = reported {
        return Ok(SteamRun::Done(reported));
    }
    let detail = tail.into_iter().collect::<Vec<_>>().join("\n");
    Ok(SteamRun::Failed { message: classify(&detail).to_string(), detail })
}

async fn read_output<R: AsyncRead + Unpin>(
    stdout: R,
    marker: &[u8],
    progress: &(dyn Fn(Stage, Option<f64>) + Sync),
) -> (Option<Vec<u8>>, VecDeque<String>) {
    let mut reader = BufReader::new(stdout);
    let mut line = Vec::new();
    let mut reported = None;
    let mut tail = VecDeque::new();
    while let Some(truncated) = read_line_capped(&mut reader, &mut line, 8192).await {
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        if !truncated {
            if let Some(path) = reported_project(&line, marker) {
                reported = Some(path.to_vec());
            }
        }
        let text = String::from_utf8_lossy(&line).into_owned();
        if let Some((stage, percent)) = progress_of(&text) {
            progress(stage, percent);
        }
        if !text.trim().is_empty() {
            if tail.len() == TAIL_LINES {
                tail.pop_front();
            }
            tail.push_back(text);
        }
    }
    (reported, tail)
}

/// `root\steamapps\workshop\content\431960\<id>`, every part a real folder (no links).
fn workshop_folder(root: &Path, id: u64) -> Result<PathBuf> {
    let item = id.to_string();
    let app_id = APP_ID.to_string();
    let mut expected = root.canonicalize()?;
    for part in ["steamapps", "workshop", "content", &app_id, &item] {
        expected.push(part);
        let meta = fs::symlink_metadata(&expected)?;
        if !meta.is_dir() || is_link(&meta) {
            return Err(AppError::msg("SteamCMD reported an unsafe Workshop folder."));
        }
    }
    Ok(expected)
}

fn checked_project(steam_dir: &Path, reported: &[u8], id: u64) -> Result<PathBuf> {
    let expected = workshop_folder(steam_dir, id)?;
    if !matches_reported_path(reported, &expected.canonicalize()?) {
        return Err(AppError::msg("SteamCMD saved the item outside its expected Workshop folder."));
    }
    Ok(expected)
}

fn matches_reported_path(reported: &[u8], expected: &Path) -> bool {
    let same = |path: &Path| path.is_absolute() && path.canonicalize().is_ok_and(|path| path == expected);
    if std::str::from_utf8(reported).is_ok_and(|path| same(Path::new(path))) {
        return true;
    }
    #[cfg(windows)]
    {
        use std::{ffi::OsString, os::windows::ffi::OsStringExt};
        use windows::Win32::Globalization::{MultiByteToWideChar, CP_ACP, CP_OEMCP, MB_ERR_INVALID_CHARS};

        // SteamCMD may print a legacy code page instead of UTF-8. Try the
        // Windows encodings, comparing the entire canonical path in either case.
        let mut wide = vec![0u16; reported.len()];
        for code_page in [CP_ACP, CP_OEMCP] {
            let count = unsafe { MultiByteToWideChar(code_page, MB_ERR_INVALID_CHARS, reported, Some(&mut wide)) };
            if count > 0 && same(Path::new(&OsString::from_wide(&wide[..count as usize]))) {
                return true;
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------------------
// ZIP export

/// A file name made from the item title: `<title> (<id>).zip`.
fn export_name(title: &str, id: u64, copy: u32) -> String {
    let mut clean = String::new();
    for c in title.chars() {
        let c = if c.is_control() || r#"<>:"/\|?*"#.contains(c) { ' ' } else { c };
        if c == ' ' && clean.ends_with(' ') {
            continue;
        }
        clean.push(c);
    }
    let mut clean: String = clean.trim().trim_end_matches(['.', ' ']).chars().take(80).collect();
    clean = clean.trim_end_matches(['.', ' ']).to_string();
    let stem = clean.split('.').next().unwrap_or("").trim().to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || ((stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4 && stem.as_bytes()[3].is_ascii_digit());
    let base = if clean.is_empty() || reserved { format!("Wallpaper Engine {id}") } else { format!("{clean} ({id})") };
    if copy == 0 {
        format!("{base}.zip")
    } else {
        format!("{base} ({copy}).zip")
    }
}

fn compresses(name: &str) -> bool {
    let extension = name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()).unwrap_or_default();
    ["json", "html", "htm", "js", "css", "txt", "frag", "vert", "xml", "svg", "md", "glsl"].contains(&extension.as_str())
}

/// Opens a file without following a link that replaced it after it was checked.
fn open_plain(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() || is_link(&meta) {
        return Err(io::Error::other("The wallpaper contains a linked file."));
    }
    Ok(file)
}

/// Zips the project into the output folder. `Ok(None)` when cancelled.
fn save_project(
    project: &Path,
    output: &Path,
    id: u64,
    title: &str,
    mut report: impl FnMut(f64),
    cancelled: impl Fn() -> bool,
) -> Result<Option<String>> {
    let meta = fs::symlink_metadata(project.join("project.json"))
        .map_err(|_| AppError::msg("The download doesn't contain a Wallpaper Engine project."))?;
    if !meta.is_file() || meta.len() == 0 || is_link(&meta) {
        return Err(AppError::msg("The download doesn't contain a Wallpaper Engine project."));
    }
    let mut files = Vec::new();
    collect(project, "", 0, &mut files)?;
    let total: u64 = files.iter().map(|entry| entry.size).sum();
    if total > MAX_EXPORT_BYTES {
        return Err(AppError::msg("This wallpaper is too large to export."));
    }

    let partial = output.join(format!("{PARTIAL_PREFIX}{}{PARTIAL_SUFFIX}", uuid::Uuid::new_v4()));
    let file = OpenOptions::new().write(true).create_new(true).open(&partial)?;
    let result = (|| -> Result<Option<String>> {
        let mut zip = zip::ZipWriter::new(BufWriter::with_capacity(256 * 1024, file));
        let mut buffer = vec![0u8; 256 * 1024];
        let mut done = 0u64;
        for entry in &files {
            if cancelled() {
                return Ok(None);
            }
            let options = zip::write::SimpleFileOptions::default().compression_method(if entry.is_dir {
                zip::CompressionMethod::Stored
            } else if compresses(&entry.name) {
                zip::CompressionMethod::Deflated
            } else {
                zip::CompressionMethod::Stored
            });
            if entry.is_dir {
                zip.add_directory(&entry.name, options)?;
                continue;
            }
            zip.start_file(&entry.name, options.large_file(entry.size >= u32::MAX as u64))?;
            let mut source = open_plain(&entry.path)?;
            loop {
                let read = source.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                zip.write_all(&buffer[..read])?;
                done += read as u64;
                if total > 0 {
                    report(done as f64 / total as f64);
                }
                if cancelled() {
                    return Ok(None);
                }
            }
        }
        let writer = zip.finish()?;
        writer.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        for copy in 0..1000 {
            let name = export_name(title, id, copy);
            match move_no_replace(&partial, &output.join(&name)) {
                Ok(()) => return Ok(Some(name)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Err(AppError::msg("Too many copies of this wallpaper in the download folder."))
    })();
    if !matches!(result, Ok(Some(_))) {
        let _ = fs::remove_file(&partial);
    }
    result
}

struct Entry {
    name: String,
    path: PathBuf,
    is_dir: bool,
    size: u64,
}

fn collect(folder: &Path, prefix: &str, depth: u8, out: &mut Vec<Entry>) -> Result<()> {
    if depth > 48 {
        return Err(AppError::msg("This wallpaper has too many nested folders."));
    }
    let mut children = fs::read_dir(folder)?.collect::<io::Result<Vec<_>>>()?;
    children.sort_by_key(|entry| entry.file_name());
    for entry in children {
        if out.len() >= MAX_ENTRIES {
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
            return Err(AppError::msg("The wallpaper contains a linked file or folder."));
        }
        if meta.is_dir() {
            let prefix = format!("{name}/");
            out.push(Entry { name: prefix.clone(), path: path.clone(), is_dir: true, size: 0 });
            collect(&path, &prefix, depth + 1, out)?;
        } else if meta.is_file() {
            out.push(Entry { name, path, is_dir: false, size: meta.len() });
        } else {
            return Err(AppError::msg("The wallpaper contains an unsupported file."));
        }
    }
    Ok(())
}

/// Removes exports a crash left half-written in the download folder.
pub fn sweep_partials(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(middle) = name.strip_prefix(PARTIAL_PREFIX).and_then(|rest| rest.strip_suffix(PARTIAL_SUFFIX)) else {
            continue;
        };
        if uuid::Uuid::parse_str(middle).is_ok() && entry.file_type().is_ok_and(|t| t.is_file()) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::time::Instant;

    #[test]
    fn classifies_wallpapers_collections_and_other_items() {
        let body = br#"{"response":{"publishedfiledetails":[
            {"publishedfileid":"123","consumer_app_id":431960,"creator_app_id":431960,"result":1,"title":" Ocean dusk ",
             "file_size":"31224037","time_updated":1602087909,
             "preview_url":"https://images.steamusercontent.com/ugc/1/ABC/",
             "tags":[{"tag":"Wallpaper"},{"tag":"Scene"},{"tag":"Approved"}]},
            {"publishedfileid":"456","consumer_app_id":480,"result":1},
            {"publishedfileid":"789","consumer_app_id":431960,"result":9},
            {"publishedfileid":"321","consumer_app_id":431960,"creator_app_id":766,"result":1,"title":"Set"},
            {"publishedfileid":"654","consumer_app_id":431960,"creator_app_id":431960,"result":1,"title":"x",
             "preview_url":"https://evil.example/preview.png"}
        ]}}"#;
        let WorkshopLookup::Wallpaper { item } = parse_details(body, 123).unwrap() else { panic!("wallpaper") };
        assert_eq!(item.title, "Ocean dusk");
        assert_eq!(item.file_size, Some(31224037));
        assert_eq!(item.wallpaper_type.as_deref(), Some("Scene"));
        assert_eq!(item.preview_url.as_deref(), Some("https://images.steamusercontent.com/ugc/1/ABC/"));
        assert_eq!(parse_details(body, 456).unwrap(), WorkshopLookup::OtherApp { app_id: 480 });
        assert_eq!(parse_details(body, 789).unwrap(), WorkshopLookup::NotFound);
        assert_eq!(parse_details(body, 999).unwrap(), WorkshopLookup::NotFound);
        assert_eq!(parse_details(body, 321).unwrap(), WorkshopLookup::Collection { title: "Set".into(), items: 0 });
        let WorkshopLookup::Wallpaper { item } = parse_details(body, 654).unwrap() else { panic!("wallpaper") };
        assert_eq!(item.preview_url, None, "previews only from Valve's hosts");
    }

    #[test]
    fn reads_children_and_required_items() {
        let body = br#"{"response":{"collectiondetails":[{"publishedfileid":"321","result":1,
            "children":[{"publishedfileid":"1","filetype":0},{"publishedfileid":"2","filetype":0}]}]}}"#;
        assert_eq!(parse_children(body, 321), ["1", "2"]);
        let none = br#"{"response":{"collectiondetails":[{"publishedfileid":"123","result":9}]}}"#;
        assert!(parse_children(none, 123).is_empty());
        assert!(parse_children(b"not json", 1).is_empty());
    }

    #[test]
    fn lookup_serializes_for_the_interface() {
        let json = serde_json::to_value(WorkshopLookup::OtherApp { app_id: 480 }).unwrap();
        assert_eq!(json, serde_json::json!({"kind":"otherApp","appId":480}));
        let json = serde_json::to_value(WallpaperOutcome::Failed { message: "m".into(), detail: None }).unwrap();
        assert_eq!(json, serde_json::json!({"status":"failed","message":"m","detail":null}));
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
    fn accepts_only_plain_steam_account_names() {
        assert_eq!(check_account("  some_user1 ").unwrap(), "some_user1");
        for bad in ["", "anonymous", "ANONYMOUS", "a b", "+quit", "user@example.com", &"x".repeat(65)] {
            assert!(check_account(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn reads_progress_lines() {
        assert_eq!(progress_of("[ 45%] Downloading update (1,234 of 2,000 KB)..."), Some((Stage::Updating, Some(45.0))));
        assert_eq!(progress_of("[----] Verifying installation..."), Some((Stage::Updating, None)));
        assert_eq!(progress_of("Logging in user 'someone' to Steam Public...OK"), Some((Stage::SigningIn, None)));
        assert_eq!(progress_of("Downloading item 123 ..."), Some((Stage::Downloading, None)));
        assert_eq!(progress_of("Loading Steam API...OK"), None);
    }

    #[test]
    fn classifies_steamcmd_failures() {
        let cases = [
            ("Logging in user 'x' to Steam Public...FAILED (Invalid Password)", "rejected the sign-in"),
            ("FAILED (Account Logon Denied)", "Steam Guard"),
            ("FAILED (Two-factor code mismatch)", "Steam Guard"),
            ("FAILED (Rate Limit Exceeded)", "limiting sign-ins"),
            ("Cached credentials not found.", "isn't signed in"),
            ("ERROR! Download item 1 failed (Access Denied).", "can't download this item"),
            ("ERROR! Download item 1 failed (File Not Found).", "couldn't find the files"),
            ("ERROR! Download item 1 failed (Disk Write Failure).", "free space"),
            ("ERROR! Download item 1 failed (Timeout).", "too long"),
            // What an anonymous sign-in gets: generic, not a claim about ownership.
            ("ERROR! Download item 2250845956 failed (Failure).", "couldn't download this wallpaper"),
        ];
        for (line, expected) in cases {
            assert!(classify(line).contains(expected), "{line} -> {}", classify(line));
        }
    }

    #[test]
    fn redacts_account_and_home_folder() {
        let text = "Logging in user 'Some_User' ... C:\\Users\\Ana\\SteamCMD and c:/users/ana/x some_user";
        let out = redact_with(text, "some_user", Some("C:\\Users\\Ana"));
        assert_eq!(out, "Logging in user '<account>' ... %USERPROFILE%\\SteamCMD and %USERPROFILE%/x <account>");
        assert_eq!(redact_with("short", "ab", None), "short");
    }

    #[test]
    fn names_exports_after_the_title() {
        assert_eq!(export_name("Ocean: dusk / 4K", 7, 0), "Ocean dusk 4K (7).zip");
        assert_eq!(export_name("Ocean", 7, 2), "Ocean (7) (2).zip");
        assert_eq!(export_name("  ...  ", 7, 0), "Wallpaper Engine 7.zip");
        assert_eq!(export_name("con", 7, 0), "Wallpaper Engine 7.zip");
        assert_eq!(export_name("COM3.scene", 7, 0), "Wallpaper Engine 7.zip");
        assert_eq!(export_name("Trailing dots...", 7, 0), "Trailing dots (7).zip");
        let long = "é".repeat(200);
        assert_eq!(export_name(&long, 7, 0).chars().count(), 80 + " (7).zip".len());
    }

    #[test]
    fn finds_steam_account_names_on_this_pc() {
        let steamcmd = r#""InstallConfigStore"
        {
            "Software" { "Valve" { "Steam" {
                // comment
                "Accounts" { "cmd_user" { "SteamID" "7656" } "Second_One" { } }
                "ConnectCache" { "abc" "def" }
            } } }
        }"#;
        assert_eq!(steamcmd_accounts(steamcmd), ["cmd_user", "Second_One"]);
        let loginusers = r#""users"
        {
            "765611" { "AccountName" "older" "PersonaName" "Old" "MostRecent" "0" }
            "765612" { "AccountName" "newest" "PersonaName" "New \"one\"" "MostRecent" "1" }
        }"#;
        assert_eq!(steam_client_accounts(loginusers), ["newest", "older"]);
        assert!(steamcmd_accounts("").is_empty() && steam_client_accounts("{{{").is_empty());
        let merged = merge_accounts([
            vec!["cmd_user".to_string()],
            vec!["newest".to_string()],
            vec!["CMD_USER".to_string(), "bad name".to_string(), "anonymous".to_string()],
            vec!["older".to_string()],
        ]);
        assert_eq!(merged, ["cmd_user", "newest", "older"]);
    }

    #[test]
    fn parses_library_folders() {
        let vdf = r#""libraryfolders"
        {
            "0"
            {
                "path"		"C:\\Program Files (x86)\\Steam"
                "label"		""
                "apps" { "431960"		"123" }
            }
            "1"
            {
                "path"		"D:\\Jogos\\Bibliothèque \"Steam\""
            }
        }"#;
        assert_eq!(
            parse_library_paths(vdf),
            [PathBuf::from(r"C:\Program Files (x86)\Steam"), PathBuf::from(r#"D:\Jogos\Bibliothèque "Steam""#)]
        );
        assert!(parse_library_paths("").is_empty());
    }

    #[test]
    fn finds_a_subscribed_item_in_any_library() {
        let dir = crate::download_tests::temp_dir("workshop-library");
        let steam = dir.join("Steam");
        let second = dir.join("Second Library");
        fs::create_dir_all(steam.join("steamapps")).unwrap();
        let project = second.join("steamapps/workshop/content/431960/123");
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join("project.json"), "{}").unwrap();
        // An item folder without a project does not count.
        fs::create_dir_all(steam.join("steamapps/workshop/content/431960/456")).unwrap();
        let vdf = format!("\"libraryfolders\" {{ \"1\" {{ \"path\" \"{}\" }} }}", second.display().to_string().replace('\\', "\\\\"));
        fs::write(steam.join("steamapps/libraryfolders.vdf"), vdf).unwrap();

        let libraries = library_folders(&steam);
        assert_eq!(libraries.len(), 2);
        assert_eq!(library_project(&libraries, 123).unwrap(), project.canonicalize().unwrap());
        assert!(library_project(&libraries, 456).is_none());
        assert!(library_project(&libraries, 789).is_none());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn exports_the_project_tree_without_replacing_an_existing_backup() {
        let dir = crate::download_tests::temp_dir("workshop-export");
        let project = dir.join("steamapps/workshop/content/431960/123");
        let output = dir.join("output");
        fs::create_dir_all(project.join("assets")).unwrap();
        fs::create_dir(&output).unwrap();
        fs::write(project.join("project.json"), r#"{"type":"web","file":"assets/index.html"}"#).unwrap();
        fs::write(project.join("assets/index.html"), b"<title>Fixture</title>").unwrap();
        fs::write(project.join("assets/video.mp4"), vec![7u8; 300_000]).unwrap();
        fs::write(output.join("Fixture (123).zip"), b"existing").unwrap();

        let project = checked_project(&dir, project.to_str().unwrap().as_bytes(), 123).unwrap();
        assert!(checked_project(&dir, output.to_str().unwrap().as_bytes(), 123).is_err());
        let mut fractions = Vec::new();
        let name = save_project(&project, &output, 123, "Fixture", |f| fractions.push(f), || false).unwrap().unwrap();
        assert_eq!(name, "Fixture (123) (1).zip");
        assert_eq!(fractions.last().copied(), Some(1.0));
        assert_eq!(fs::read(output.join("Fixture (123).zip")).unwrap(), b"existing");
        {
            let mut zip = zip::ZipArchive::new(File::open(output.join(&name)).unwrap()).unwrap();
            let names: Vec<String> = zip.file_names().map(|name| name.unwrap().into_owned()).collect();
            assert_eq!(names, ["assets/", "assets/index.html", "assets/video.mp4", "project.json"]);
            assert_eq!(zip.by_name("project.json").unwrap().compression(), zip::CompressionMethod::Deflated);
            assert_eq!(zip.by_name("assets/video.mp4").unwrap().compression(), zip::CompressionMethod::Stored);
            let mut html = String::new();
            zip.by_name("assets/index.html").unwrap().read_to_string(&mut html).unwrap();
            assert_eq!(html, "<title>Fixture</title>");
        }
        assert_eq!(fs::read_dir(&output).unwrap().count(), 2);

        // Cancelled part-way: nothing new is left in the folder.
        assert!(save_project(&project, &output, 123, "Fixture", |_| {}, || true).unwrap().is_none());
        assert_eq!(fs::read_dir(&output).unwrap().count(), 2);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn sweeps_only_its_own_partial_files() {
        let dir = crate::download_tests::temp_dir("workshop-sweep");
        let ours = dir.join(format!("{PARTIAL_PREFIX}{}{PARTIAL_SUFFIX}", uuid::Uuid::new_v4()));
        let other = dir.join(".claws-wallpaper-notes.partial");
        let video = dir.join("video.mp4");
        for file in [&ours, &other, &video] {
            fs::write(file, b"x").unwrap();
        }
        sweep_partials(&dir);
        assert!(!ours.exists());
        assert!(other.exists() && video.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    /// Live: set CLAWS_TEST_STEAMCMD to a steamcmd.exe downloaded from Valve.
    #[cfg(windows)]
    #[test]
    fn accepts_valves_signed_steamcmd_when_given() {
        let Some(path) = std::env::var_os("CLAWS_TEST_STEAMCMD") else { return };
        assert!(signed_by_valve(Path::new(&path)));
        assert!(validate_steamcmd(Path::new(&path)).is_ok());
    }

    /// Live: set CLAWS_TEST_NETWORK=1 to ask Valve's API about a known wallpaper and collection.
    #[tokio::test]
    async fn reads_real_workshop_details_when_online() {
        if std::env::var_os("CLAWS_TEST_NETWORK").is_none() {
            return;
        }
        let client = http_client();
        let WorkshopLookup::Wallpaper { item } = lookup(&client, 2250845956).await.unwrap() else { panic!("wallpaper") };
        assert!(!item.title.is_empty() && item.file_size.is_some() && item.preview_url.is_some());
        assert!(matches!(lookup(&client, 3844862544).await.unwrap(), WorkshopLookup::Collection { items, .. } if items > 0));
    }

    #[cfg(windows)]
    #[test]
    fn rejects_unsigned_executables() {
        // The test binary itself is not signed by Valve.
        let me = std::env::current_exe().unwrap();
        assert!(!signed_by_valve(&me));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn stops_a_long_running_process_tree() {
        let mut child = Command::new("cmd")
            .args(["/c", "ping", "-n", "60", "127.0.0.1"])
            .stdout(Stdio::null())
            .creation_flags(0x0800_0000)
            .spawn()
            .unwrap();
        let pid = child.id().unwrap();
        let started = Instant::now();
        kill_tree(pid, true);
        let status = timeout(Duration::from_secs(10), child.wait()).await.expect("stopped").unwrap();
        assert!(!status.success());
        assert!(started.elapsed() < Duration::from_secs(10));
    }
}
