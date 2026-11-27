//! Where Steam keeps things on this PC: its install folder, the libraries it lists, and the
//! Wallpaper Engine Workshop items inside them. Everything here only reads; Claws never writes
//! into a Steam folder.
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use serde::Serialize;

use crate::{
    download_files::is_link,
    error::{AppError, Result},
};

pub const APP_ID: u64 = 431960;
/// Largest VDF or ACF file Claws reads. A library of a few hundred items is under 100 KB.
const MAX_VDF_BYTES: u64 = 16 * 1024 * 1024;

/// A Valve text file, read through a size cap. Lossy, so odd bytes never stop the read.
pub(crate) fn read_text_capped(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(MAX_VDF_BYTES + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= MAX_VDF_BYTES).then(|| String::from_utf8_lossy(&bytes).into_owned())
}

// ---------------------------------------------------------------------------------------
// Valve's text format (VDF / ACF)

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

// ---------------------------------------------------------------------------------------
// Libraries

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
pub fn steam_root() -> Option<PathBuf> {
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
pub fn steam_root() -> Option<PathBuf> {
    None
}

/// Every Steam library folder: Steam's own folder first, then the ones it lists.
pub fn library_folders(root: &Path) -> Vec<PathBuf> {
    let mut folders = vec![root.to_path_buf()];
    if let Some(text) = read_text_capped(&root.join("steamapps").join("libraryfolders.vdf")) {
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

/// `root\steamapps\workshop\content\431960\<id>`, every part a real folder (no links).
pub(crate) fn workshop_folder(root: &Path, id: u64) -> Result<PathBuf> {
    let item = id.to_string();
    let app_id = APP_ID.to_string();
    let mut expected = root.canonicalize()?;
    for part in ["steamapps", "workshop", "content", &app_id, &item] {
        expected.push(part);
        let meta = fs::symlink_metadata(&expected)?;
        if !meta.is_dir() || is_link(&meta) {
            return Err(AppError::msg("The Workshop folder looks unsafe, so Claws did not use it."));
        }
    }
    Ok(expected)
}

// ---------------------------------------------------------------------------------------
// Workshop item state

/// What Steam's own bookkeeping says about one subscribed item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ItemState {
    /// Not in any library, and nothing is downloading.
    NotInstalled,
    /// Listed by Steam as installed, with nothing newer known.
    Installed,
    /// Steam is still writing it. `bytes` is what is on disk so far.
    Downloading { bytes: u64 },
    /// Installed, and Valve's copy was updated after the one Steam last downloaded.
    UpdatePending,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledItem {
    pub size: u64,
    pub time_updated: i64,
}

/// What Claws reads from `appworkshop_431960.acf`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct WorkshopManifest {
    pub installed: HashMap<u64, InstalledItem>,
    /// The newest update time Steam has seen for an item, from `WorkshopItemDetails`.
    pub latest: HashMap<u64, i64>,
    pub needs_download: HashSet<u64>,
    pub needs_update: HashSet<u64>,
}

fn text<'a>(fields: &'a [(String, Vdf)], name: &str) -> Option<&'a str> {
    fields.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).and_then(|(_, value)| match value {
        Vdf::Text(text) => Some(text.as_str()),
        Vdf::Block(_) => None,
    })
}

/// Reads the installed items (and the newest update times) out of the app's workshop manifest.
pub fn parse_workshop_manifest(source: &str) -> WorkshopManifest {
    let entries = parse_vdf(source);
    let mut manifest = WorkshopManifest::default();
    let mut blocks = Vec::new();
    find_blocks(&entries, "WorkshopItemsInstalled", &mut blocks);
    for items in blocks {
        for (key, value) in items {
            let (Ok(id), Vdf::Block(fields)) = (key.parse::<u64>(), value) else { continue };
            let size = text(fields, "size").and_then(|v| v.parse().ok()).unwrap_or(0);
            let Some(time_updated) = text(fields, "timeupdated").and_then(|v| v.parse().ok()) else { continue };
            manifest.installed.insert(id, InstalledItem { size, time_updated });
        }
    }
    let mut blocks = Vec::new();
    find_blocks(&entries, "WorkshopItemDetails", &mut blocks);
    for items in blocks {
        for (key, value) in items {
            let (Ok(id), Vdf::Block(fields)) = (key.parse::<u64>(), value) else { continue };
            if text(fields, "NeedsDownload").is_some_and(|v| v != "0" && !v.is_empty()) { manifest.needs_download.insert(id); }
            if text(fields, "NeedsUpdate").is_some_and(|v| v != "0" && !v.is_empty()) { manifest.needs_update.insert(id); }
            if let Some(latest) = text(fields, "latest_timeupdated").and_then(|v| v.parse().ok()) {
                manifest.latest.insert(id, latest);
            }
        }
    }
    for (key, target) in [("NeedsDownload", &mut manifest.needs_download), ("NeedsUpdate", &mut manifest.needs_update)] {
        let mut blocks = Vec::new();
        find_blocks(&entries, key, &mut blocks);
        for items in blocks {
            for (id, value) in items {
                if let Ok(id) = id.parse::<u64>() {
                    if !matches!(value, Vdf::Text(value) if value == "0" || value.is_empty()) { target.insert(id); }
                }
            }
        }
    }
    manifest
}

/// The state of an item that has a project folder.
fn state_with_project(downloading: Option<u64>, installed: Option<&InstalledItem>, latest: Option<i64>, valve: Option<i64>, needs_download: bool, needs_update: bool, on_disk: u64) -> ItemState {
    if let Some(bytes) = downloading { return ItemState::Downloading { bytes }; }
    if needs_download { return ItemState::Downloading { bytes: on_disk }; }
    let newest = valve.into_iter().chain(latest).max();
    if needs_update || installed.is_some_and(|entry| newest.is_some_and(|newest| newest > entry.time_updated)) {
        ItemState::UpdatePending
    } else { ItemState::Installed }
}

/// Bytes under a folder: files only, no link followed, bounded in depth and entries.
pub(crate) fn bytes_below(folder: &Path) -> u64 {
    fn walk(folder: &Path, depth: usize, seen: &mut usize) -> u64 {
        if depth > 16 {
            return 0;
        }
        let Ok(entries) = fs::read_dir(folder) else { return 0 };
        let mut total = 0u64;
        for entry in entries.flatten() {
            *seen += 1;
            if *seen > 200_000 {
                break;
            }
            let Ok(kind) = fs::symlink_metadata(entry.path()) else { continue };
            if is_link(&kind) { continue; }
            if kind.is_dir() {
                total = total.saturating_add(walk(&entry.path(), depth + 1, seen));
            } else if kind.is_file() {
                total = total.saturating_add(kind.len());
            }
        }
        total
    }
    walk(folder, 0, &mut 0)
}

/// An item found in the libraries and what Steam says about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocatedItem {
    /// The item's project folder, when it has a readable `project.json`.
    pub project: Option<PathBuf>,
    pub state: ItemState,
}

fn plain_dir(path: &Path) -> bool {
    let mut current = PathBuf::new();
    for part in path.components() {
        current.push(part.as_os_str());
        if matches!(part, std::path::Component::Prefix(_)) { continue; }
        if !fs::symlink_metadata(&current).is_ok_and(|meta| meta.is_dir() && !is_link(&meta)) { return false; }
    }
    true
}

/// Looks the item up in every library: its project folder, Steam's manifest and the folder
/// Steam downloads into. `valve_updated` is the time Valve's public details give.
pub fn locate_item(libraries: &[PathBuf], id: u64, valve_updated: Option<i64>) -> LocatedItem {
    let mut downloading_only = None;
    for library in libraries {
        let workshop = library.join("steamapps").join("workshop");
        let downloads = workshop.join("downloads").join(APP_ID.to_string()).join(id.to_string());
        let downloading = plain_dir(&downloads).then(|| bytes_below(&downloads)).filter(|bytes| *bytes > 0);
        let manifest = read_text_capped(&workshop.join(format!("appworkshop_{APP_ID}.acf")))
            .map(|text| parse_workshop_manifest(&text)).unwrap_or_default();
        let project = workshop_folder(library, id)
            .ok()
            .filter(|folder| fs::symlink_metadata(folder.join("project.json")).is_ok_and(|m| m.is_file() && !is_link(&m)));
        let Some(project) = project else {
            if downloading_only.is_none() {
                downloading_only = downloading.or_else(|| manifest.needs_download.contains(&id).then_some(0));
            }
            continue;
        };
        let on_disk = if manifest.needs_download.contains(&id) { bytes_below(&project) } else { 0 };
        let state = state_with_project(downloading, manifest.installed.get(&id), manifest.latest.get(&id).copied(), valve_updated, manifest.needs_download.contains(&id), manifest.needs_update.contains(&id), on_disk);
        return LocatedItem { project: Some(project), state };
    }
    LocatedItem {
        project: None,
        state: downloading_only.map_or(ItemState::NotInstalled, |bytes| ItemState::Downloading { bytes }),
    }
}

// ---------------------------------------------------------------------------------------
// Wallpaper Engine itself

/// The library that `libraryfolders.vdf` lists app `app` under, when it says so.
pub fn library_holding_app(libraryfolders: &str, app: u64) -> Option<PathBuf> {
    let entries = parse_vdf(libraryfolders);
    let mut folders = Vec::new();
    find_blocks(&entries, "libraryfolders", &mut folders);
    for folder in folders {
        for (_, value) in folder {
            let Vdf::Block(fields) = value else { continue };
            let Some(path) = text(fields, "path").filter(|path| !path.is_empty()) else { continue };
            let lists_app = fields.iter().any(|(key, value)| {
                key.eq_ignore_ascii_case("apps") && matches!(value, Vdf::Block(apps) if apps.iter().any(|(id, _)| id.parse::<u64>() == Ok(app)))
            });
            if lists_app {
                return Some(PathBuf::from(path));
            }
        }
    }
    None
}

/// `installdir` from an app manifest, only when it is one plain folder name.
pub fn install_dir_name(app_manifest: &str) -> Option<String> {
    let entries = parse_vdf(app_manifest);
    let mut states = Vec::new();
    find_blocks(&entries, "AppState", &mut states);
    let name = states.into_iter().find_map(|fields| text(fields, "installdir"))?;
    let plain = !name.is_empty()
        && name.len() <= 255
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', ':'])
        && !name.chars().any(char::is_control)
        && !name.ends_with(['.', ' ']);
    plain.then(|| name.to_string())
}

/// Wallpaper Engine's `wallpaper64.exe`, found the way Steam records it: the library that
/// lists the app, its app manifest's `installdir`, then `common\<installdir>\wallpaper64.exe`.
/// It must be a regular file, not a link. Whether it is genuine is checked separately, by its
/// signature, before Claws runs it.
pub fn wallpaper_engine_exe(libraries: &[PathBuf], root: &Path) -> Option<PathBuf> {
    let listed = read_text_capped(&root.join("steamapps").join("libraryfolders.vdf")).and_then(|text| library_holding_app(&text, APP_ID));
    // The library the listing names first; the others only if the listing is silent or wrong.
    let candidates = listed.into_iter().chain(libraries.iter().cloned());
    for library in candidates {
        let steamapps = library.join("steamapps");
        let Some(name) = read_text_capped(&steamapps.join(format!("appmanifest_{APP_ID}.acf"))).and_then(|text| install_dir_name(&text)) else {
            continue;
        };
        let exe = steamapps.join("common").join(name).join("wallpaper64.exe");
        #[cfg(windows)]
        let safe = crate::workshop_record::lock_engine_path(&exe).is_ok();
        #[cfg(not(windows))]
        let safe = fs::symlink_metadata(&exe).is_ok_and(|meta| meta.is_file() && !is_link(&meta));
        if safe { return Some(exe); }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

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

    const ACF: &str = r#""AppWorkshop"
{
	"appid"		"431960"
	"NeedsUpdate"		"0"
	"NeedsDownload"		"0"
	"WorkshopItemsInstalled"
	{
		"123"
		{
			"size"		"7701235"
			"timeupdated"		"1508349730"
			"manifest"		"751406270997385512"
		}
		"456"
		{
			"size"		"100"
			"timeupdated"		"2000"
			"manifest"		"1"
		}
	}
	"WorkshopItemDetails"
	{
		"123"
		{
			"manifest"		"751406270997385512"
			"timeupdated"		"1508349730"
			"latest_timeupdated"		"1508349730"
		}
		"456"
		{
			"manifest"		"1"
			"timeupdated"		"2000"
			"latest_timeupdated"		"2500"
		}
	}
}"#;

    #[test]
    fn reads_installed_items_and_latest_times_from_the_workshop_manifest() {
        let manifest = parse_workshop_manifest(ACF);
        assert_eq!(manifest.installed.get(&123), Some(&InstalledItem { size: 7_701_235, time_updated: 1_508_349_730 }));
        assert_eq!(manifest.installed.len(), 2);
        assert_eq!(manifest.latest.get(&456), Some(&2500));
        assert_eq!(parse_workshop_manifest("").installed.len(), 0);
        assert_eq!(parse_workshop_manifest("{{{ nonsense \"x\"").installed.len(), 0);
        // An id that is not a number, or an entry without a time, is skipped.
        let odd = r#""AppWorkshop" { "WorkshopItemsInstalled" { "abc" { "timeupdated" "1" } "9" { "size" "1" } } }"#;
        assert!(parse_workshop_manifest(odd).installed.is_empty());
    }

    fn library(tag: &str) -> PathBuf {
        let dir = crate::download_tests::temp_dir(tag);
        fs::create_dir_all(dir.join("steamapps/workshop/content/431960")).unwrap();
        dir
    }

    fn add_project(library: &Path, id: u64, bytes: usize) {
        let folder = library.join(format!("steamapps/workshop/content/431960/{id}"));
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("project.json"), "{}").unwrap();
        fs::write(folder.join("data.bin"), vec![0u8; bytes]).unwrap();
    }

    #[test]
    fn an_installed_item_is_installed_until_valve_has_a_newer_update() {
        let dir = library("steam-state-installed");
        fs::write(dir.join("steamapps/workshop/appworkshop_431960.acf"), ACF).unwrap();
        add_project(&dir, 123, 10);
        add_project(&dir, 456, 10);
        let libs = [dir.clone()];
        assert_eq!(locate_item(&libs, 123, Some(1_508_349_730)).state, ItemState::Installed);
        assert_eq!(locate_item(&libs, 123, None).state, ItemState::Installed, "no Valve time: nothing to compare");
        assert_eq!(locate_item(&libs, 123, Some(1_508_349_731)).state, ItemState::UpdatePending, "Valve's copy is newer");
        // Steam's own manifest knows of a newer one for 456.
        assert_eq!(locate_item(&libs, 456, Some(2000)).state, ItemState::UpdatePending);
        assert!(locate_item(&libs, 123, None).project.is_some());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_item_being_written_reports_the_bytes_on_disk() {
        let dir = library("steam-state-downloading");
        fs::write(dir.join("steamapps/workshop/appworkshop_431960.acf"), ACF).unwrap();
        // Steam is downloading 789: nothing is installed yet, only the downloads folder.
        let downloads = dir.join("steamapps/workshop/downloads/431960/789/nested");
        fs::create_dir_all(&downloads).unwrap();
        fs::write(downloads.join("part"), vec![0u8; 1500]).unwrap();
        fs::write(dir.join("steamapps/workshop/downloads/431960/789/other"), vec![0u8; 500]).unwrap();
        let libs = [dir.clone()];
        let found = locate_item(&libs, 789, None);
        assert_eq!(found, LocatedItem { project: None, state: ItemState::Downloading { bytes: 2000 } });

        // An update in progress for an installed item is still downloading.
        add_project(&dir, 123, 10);
        let update = dir.join("steamapps/workshop/downloads/431960/123");
        fs::create_dir_all(&update).unwrap();
        fs::write(update.join("part"), vec![0u8; 300]).unwrap();
        assert_eq!(locate_item(&libs, 123, None).state, ItemState::Downloading { bytes: 300 });

        // Missing item metadata alone does not prove an active download.
        add_project(&dir, 555, 64);
        assert_eq!(locate_item(&libs, 555, None).state, ItemState::Installed);
        assert_eq!(locate_item(&libs, 999, None), LocatedItem { project: None, state: ItemState::NotInstalled });
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_missing_or_unreadable_manifest_still_finds_the_item() {
        let dir = library("steam-state-no-acf");
        add_project(&dir, 123, 10);
        let found = locate_item(std::slice::from_ref(&dir), 123, None);
        assert!(found.project.is_some());
        assert_eq!(found.state, ItemState::Installed, "unknown metadata does not prove downloading");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn item_specific_download_and_update_hints_affect_the_state() {
        let dir = library("steam-state-hints");
        add_project(&dir, 123, 10);
        let manifest = dir.join("steamapps/workshop/appworkshop_431960.acf");
        fs::write(&manifest, r#""AppWorkshop" { "WorkshopItemDetails" { "123" { "NeedsDownload" "1" } } }"#).unwrap();
        assert!(matches!(locate_item(std::slice::from_ref(&dir), 123, None).state, ItemState::Downloading { .. }));
        fs::write(&manifest, r#""AppWorkshop" { "WorkshopItemDetails" { "123" { "NeedsUpdate" "1" } } }"#).unwrap();
        assert_eq!(locate_item(std::slice::from_ref(&dir), 123, None).state, ItemState::UpdatePending);
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(windows)]
    #[test]
    fn linked_download_ancestor_is_not_followed() {
        use std::process::{Command, Stdio};
        let dir = library("linked-download-path");
        let target = library("linked-download-target");
        let child = target.join("431960/123");
        fs::create_dir_all(&child).unwrap();
        fs::write(child.join("part"), b"data").unwrap();
        let link = dir.join("steamapps").join("workshop").join("downloads");
        let made = Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .unwrap();
        assert!(made.status.success(), "creating a junction must succeed on a standard account: {}", String::from_utf8_lossy(&made.stderr));
        assert_eq!(locate_item(std::slice::from_ref(&dir), 123, None).state, ItemState::NotInstalled);
        fs::remove_dir(link).unwrap();
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all(target);
    }

    #[test]
    fn the_item_state_serializes_for_the_interface() {
        assert_eq!(serde_json::to_value(ItemState::NotInstalled).unwrap(), serde_json::json!({"state":"notInstalled"}));
        assert_eq!(serde_json::to_value(ItemState::UpdatePending).unwrap(), serde_json::json!({"state":"updatePending"}));
        assert_eq!(serde_json::to_value(ItemState::Downloading { bytes: 5 }).unwrap(), serde_json::json!({"state":"downloading","bytes":5}));
    }

    #[test]
    fn bytes_below_counts_files_only_and_stops_at_the_bounds() {
        let dir = library("steam-bytes");
        fs::create_dir_all(dir.join("a/b")).unwrap();
        fs::write(dir.join("a/one"), vec![0u8; 10]).unwrap();
        fs::write(dir.join("a/b/two"), vec![0u8; 20]).unwrap();
        assert_eq!(bytes_below(&dir.join("a")), 30);
        assert_eq!(bytes_below(&dir.join("missing")), 0);
        fs::remove_dir_all(dir).unwrap();
    }


    #[test]
    fn finds_the_library_that_lists_wallpaper_engine() {
        let vdf = r#""libraryfolders"
        {
            "0" { "path" "C:\\Program Files (x86)\\Steam" "apps" { "431960" "1248432452" "228980" "1" } }
            "1" { "path" "D:\\Games" "apps" { "570" "5" } }
        }"#;
        assert_eq!(library_holding_app(vdf, 431960), Some(PathBuf::from(r"C:\Program Files (x86)\Steam")));
        assert_eq!(library_holding_app(vdf, 570), Some(PathBuf::from(r"D:\Games")));
        assert_eq!(library_holding_app(vdf, 1), None);
        assert_eq!(library_holding_app("", 431960), None);
    }

    #[test]
    fn the_install_folder_must_be_one_plain_name() {
        let manifest = |dir: &str| format!("\"AppState\" {{ \"appid\" \"431960\" \"installdir\" \"{dir}\" }}");
        assert_eq!(install_dir_name(&manifest("wallpaper_engine")).as_deref(), Some("wallpaper_engine"));
        assert_eq!(install_dir_name(&manifest("Wallpaper Engine 2")).as_deref(), Some("Wallpaper Engine 2"));
        for bad in ["", ".", "..", "../x", "a/b", "a\\\\b", "C:x", "x.", "x "] {
            assert_eq!(install_dir_name(&manifest(bad)), None, "{bad:?}");
        }
        assert_eq!(install_dir_name("\"AppState\" { \"appid\" \"1\" }"), None);
    }

    #[test]
    fn locates_wallpaper_engine_through_its_manifest() {
        let dir = crate::download_tests::temp_dir("steam-engine-exe");
        let steam = dir.join("Steam");
        let other = dir.join("Other Library");
        fs::create_dir_all(steam.join("steamapps")).unwrap();
        fs::create_dir_all(other.join("steamapps/common/wallpaper_engine")).unwrap();
        fs::write(other.join("steamapps/common/wallpaper_engine/wallpaper64.exe"), b"MZ").unwrap();
        fs::write(other.join("steamapps/appmanifest_431960.acf"), "\"AppState\" { \"installdir\" \"wallpaper_engine\" }").unwrap();
        let vdf = format!("\"libraryfolders\" {{ \"1\" {{ \"path\" \"{}\" \"apps\" {{ \"431960\" \"1\" }} }} }}", other.display().to_string().replace('\\', "\\\\"));
        fs::write(steam.join("steamapps/libraryfolders.vdf"), vdf).unwrap();
        let libraries = library_folders(&steam);
        let exe = wallpaper_engine_exe(&libraries, &steam).unwrap();
        assert_eq!(exe, other.join("steamapps/common/wallpaper_engine/wallpaper64.exe"));

        // Not installed: no manifest anywhere.
        fs::remove_file(other.join("steamapps/appmanifest_431960.acf")).unwrap();
        assert_eq!(wallpaper_engine_exe(&libraries, &steam), None);
        // A manifest whose executable is missing is not Wallpaper Engine either.
        fs::write(other.join("steamapps/appmanifest_431960.acf"), "\"AppState\" { \"installdir\" \"gone\" }").unwrap();
        assert_eq!(wallpaper_engine_exe(&libraries, &steam), None);
        fs::remove_dir_all(dir).unwrap();
    }

}
