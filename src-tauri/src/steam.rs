//! Where Steam keeps things on this PC: its install folder, the libraries it lists, and the
//! Wallpaper Engine Workshop items inside them. Everything here only reads; Claws never writes
//! into a Steam folder.
use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{
    download_files::is_link,
    error::{AppError, Result},
};

pub const APP_ID: u64 = 431960;

// ---------------------------------------------------------------------------------------
// Valve's text format (VDF / ACF)

/// One value of a Valve KeyValues (VDF) file: text, or a block of named values.
#[derive(Debug, PartialEq)]
#[allow(dead_code)] // read by the Workshop item state and Wallpaper Engine lookup
enum Vdf {
    Text(String),
    Block(Vec<(String, Vdf)>),
}

/// Parses VDF text: quoted strings (with backslash escapes), braces and `//` comments.
/// Malformed input yields whatever parsed before the problem.
#[allow(dead_code)]
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
#[allow(dead_code)]
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
}
