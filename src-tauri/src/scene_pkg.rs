//! Reading a Wallpaper Engine scene package (`scene.pkg`) to tell, before anything is recorded,
//! whether the scene moves, how large it was authored and whether it has sound.
//!
//! A package is untrusted input. Every count, length and offset is bounded and checked against
//! the file's real size before it is used: a version string of at most 32 bytes, at most 100,000
//! entries, names of at most 1 KB, every entry inside the file, a `scene.json` of at most 4 MB,
//! and only the first bytes of each texture. Nothing in a package is ever run.
#![allow(dead_code)] // the recorder that reads these lands in the next commit
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

/// Bounds on what a package may claim.
pub const MAX_VERSION_BYTES: u32 = 32;
pub const MAX_ENTRIES: u32 = 100_000;
pub const MAX_NAME_BYTES: u32 = 1024;
pub const MAX_SCENE_JSON: u64 = 4 * 1024 * 1024;
/// A texture's header is read, not the picture behind it.
const TEX_HEADER_BYTES: usize = 64;
const JSON_DEPTH: usize = 64;

#[derive(Debug, PartialEq, Eq)]
pub enum PkgError {
    /// Not a package, or cut short.
    Malformed(&'static str),
    /// A count, length or offset beyond what Claws accepts or the file holds.
    OutOfBounds(&'static str),
    Io(String),
}

impl From<std::io::Error> for PkgError {
    fn from(error: std::io::Error) -> Self {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            PkgError::Malformed("the package ends too soon")
        } else {
            PkgError::Io(error.to_string())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    /// Where the entry's bytes begin in the file.
    pub start: u64,
    pub len: u64,
}

fn read_u32(reader: &mut impl Read) -> Result<u32, PkgError> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

/// Reads the package's header and entry table. `file_len` is the real size of the file.
pub fn read_index(reader: &mut (impl Read + Seek), file_len: u64) -> Result<(String, Vec<Entry>), PkgError> {
    reader.seek(SeekFrom::Start(0))?;
    let version_len = read_u32(reader)?;
    if version_len == 0 || version_len > MAX_VERSION_BYTES {
        return Err(PkgError::OutOfBounds("the version name is too long"));
    }
    let mut version = vec![0u8; version_len as usize];
    reader.read_exact(&mut version)?;
    let version = String::from_utf8(version).map_err(|_| PkgError::Malformed("the version name is not text"))?;
    if !version.starts_with("PKGV") {
        return Err(PkgError::Malformed("not a Wallpaper Engine package"));
    }
    let count = read_u32(reader)?;
    if count > MAX_ENTRIES {
        return Err(PkgError::OutOfBounds("too many entries"));
    }
    // Each entry needs at least 12 bytes of table, so a count the file cannot hold is refused
    // before a single allocation is sized from it.
    let table_floor = u64::from(count) * 12;
    let here = reader.stream_position()?;
    if here.saturating_add(table_floor) > file_len {
        return Err(PkgError::OutOfBounds("more entries than the file can hold"));
    }
    let mut raw = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let name_len = read_u32(reader)?;
        if name_len > MAX_NAME_BYTES {
            return Err(PkgError::OutOfBounds("an entry name is too long"));
        }
        let mut name = vec![0u8; name_len as usize];
        reader.read_exact(&mut name)?;
        let name = String::from_utf8(name).map_err(|_| PkgError::Malformed("an entry name is not text"))?;
        let offset = read_u32(reader)?;
        let len = read_u32(reader)?;
        raw.push((name, u64::from(offset), u64::from(len)));
    }
    let data_start = reader.stream_position()?;
    let mut entries = Vec::with_capacity(raw.len());
    for (name, offset, len) in raw {
        let start = data_start.checked_add(offset).ok_or(PkgError::OutOfBounds("an entry starts beyond the file"))?;
        let end = start.checked_add(len).ok_or(PkgError::OutOfBounds("an entry ends beyond the file"))?;
        if end > file_len {
            return Err(PkgError::OutOfBounds("an entry reaches past the end of the file"));
        }
        entries.push(Entry { name, start, len });
    }
    Ok((version, entries))
}

/// The first `limit` bytes of an entry (or all of it, when shorter).
pub fn read_entry(reader: &mut (impl Read + Seek), entry: &Entry, limit: u64) -> Result<Vec<u8>, PkgError> {
    reader.seek(SeekFrom::Start(entry.start))?;
    let want = entry.len.min(limit);
    let mut bytes = Vec::with_capacity(want as usize);
    reader.take(want).read_to_end(&mut bytes)?;
    if (bytes.len() as u64) < want {
        return Err(PkgError::Malformed("an entry is cut short"));
    }
    Ok(bytes)
}

/// How a scene was authored: the picture's size, or no fixed size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Authored {
    Fixed { width: u32, height: u32 },
    /// Wallpaper Engine fits the scene to the display.
    Auto,
    /// A perspective scene has no flat size.
    Perspective,
}

/// What `scene.json` says, without the textures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneJson {
    pub effects: bool,
    pub particles: bool,
    pub animation: bool,
    pub scripts: bool,
    pub camera_shake: bool,
    pub sound: bool,
    pub authored: Authored,
}

/// An object's value for a key, matching the key without regard to case (a hand-edited scene may
/// repeat a key in another case; both are visited).
fn visit(value: &serde_json::Value, depth: usize, found: &mut SceneJson) {
    if depth > JSON_DEPTH {
        return;
    }
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                match key.to_ascii_lowercase().as_str() {
                    "file" => {
                        if value.as_str().is_some_and(|file| file.to_ascii_lowercase().starts_with("effects/")) {
                            found.effects = true;
                        }
                    }
                    "particle" => found.particles = true,
                    "puppet" => found.animation = true,
                    "animationlayers" => {
                        if value.as_array().is_none_or(|layers| !layers.is_empty()) {
                            found.animation = true;
                        }
                    }
                    "script" => {
                        if value.as_str().is_none_or(|script| !script.trim().is_empty()) {
                            found.scripts = true;
                        }
                    }
                    "camerashake" => {
                        if value.as_bool() == Some(true) || value.as_object().is_some_and(|shake| !shake.is_empty()) {
                            found.camera_shake = true;
                        }
                    }
                    "sound" if value.as_array().is_some_and(|sounds| !sounds.is_empty()) || value.as_str().is_some_and(|sound| !sound.is_empty()) => {
                        found.sound = true;
                    }
                    _ => {}
                }
                visit(value, depth + 1, found);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                visit(item, depth + 1, found);
            }
        }
        _ => {}
    }
}

/// A number from JSON that may be written as an integer or a float, within a sane size.
fn extent(value: &serde_json::Value) -> Option<u32> {
    let number = value.as_f64()?;
    (number.is_finite() && (1.0..=65_536.0).contains(&number.round())).then(|| number.round() as u32)
}

fn authored_of(root: &serde_json::Value) -> Authored {
    let general = root.as_object().and_then(|map| map.iter().find(|(key, _)| key.eq_ignore_ascii_case("general")).map(|(_, v)| v));
    let projection = general
        .and_then(|general| general.as_object())
        .and_then(|map| map.iter().find(|(key, _)| key.eq_ignore_ascii_case("orthogonalprojection")).map(|(_, v)| v))
        .and_then(|projection| projection.as_object());
    let Some(projection) = projection else { return Authored::Perspective };
    let field = |name: &str| projection.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, v)| v);
    if field("auto").and_then(|v| v.as_bool()) == Some(true) {
        return Authored::Auto;
    }
    match (field("width").and_then(extent), field("height").and_then(extent)) {
        (Some(width), Some(height)) => Authored::Fixed { width, height },
        _ => Authored::Auto,
    }
}

/// Reads a `scene.json`. Fails only on text that is not JSON.
pub fn analyze_scene_json(json: &[u8]) -> Result<SceneJson, PkgError> {
    let text = std::str::from_utf8(json).map_err(|_| PkgError::Malformed("scene.json is not text"))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let root: serde_json::Value = serde_json::from_str(text).map_err(|_| PkgError::Malformed("scene.json is not valid JSON"))?;
    let mut found = SceneJson {
        effects: false,
        particles: false,
        animation: false,
        scripts: false,
        camera_shake: false,
        sound: false,
        authored: authored_of(&root),
    };
    visit(&root, 0, &mut found);
    Ok(found)
}

/// Whether a texture's header says it holds animation frames (a sprite sheet): the `TEXV` file
/// carries a flag bit for it after the two version tags and the format word.
pub fn tex_is_sprite_sheet(header: &[u8]) -> bool {
    const FLAGS_AT: usize = 22;
    const IS_GIF: u32 = 4;
    if header.len() < FLAGS_AT + 4 || !header.starts_with(b"TEXV") {
        return false;
    }
    let flags = u32::from_le_bytes([header[FLAGS_AT], header[FLAGS_AT + 1], header[FLAGS_AT + 2], header[FLAGS_AT + 3]]);
    flags & IS_GIF != 0
}

fn is_audio(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [".mp3", ".ogg", ".flac", ".wav"].iter().any(|extension| lower.ends_with(extension))
}

/// What a scene package shows about the scene.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneInfo {
    /// Effects, particles, puppets or animation layers, scripts, camera shake or sprite
    /// textures: it changes without anyone touching it. A scene with none is a still picture.
    pub moves: bool,
    pub authored: Authored,
    /// The package holds sound files, or the scene lists sounds.
    pub has_audio: bool,
    pub package_bytes: u64,
}

/// Reads a package from an open file of `file_len` bytes.
pub fn analyze(reader: &mut (impl Read + Seek), file_len: u64) -> Result<SceneInfo, PkgError> {
    let (_, entries) = read_index(reader, file_len)?;
    let scene = entries.iter().find(|entry| entry.name == "scene.json").ok_or(PkgError::Malformed("the package has no scene.json"))?;
    if scene.len > MAX_SCENE_JSON {
        return Err(PkgError::OutOfBounds("scene.json is too large"));
    }
    let json = read_entry(reader, scene, MAX_SCENE_JSON)?;
    let scene_json = analyze_scene_json(&json)?;
    let mut sprites = false;
    for entry in entries.iter().filter(|entry| entry.name.to_ascii_lowercase().ends_with(".tex")) {
        let header = read_entry(reader, entry, TEX_HEADER_BYTES as u64)?;
        if tex_is_sprite_sheet(&header) {
            sprites = true;
            break;
        }
    }
    let audio_files = entries.iter().any(|entry| is_audio(&entry.name));
    Ok(SceneInfo {
        moves: scene_json.effects || scene_json.particles || scene_json.animation || scene_json.scripts || scene_json.camera_shake || sprites,
        authored: scene_json.authored,
        has_audio: audio_files || scene_json.sound,
        package_bytes: file_len,
    })
}

/// Reads the package at `path`: opened as a plain file and measured by what the file holds.
pub fn analyze_file(path: &Path) -> Result<SceneInfo, PkgError> {
    let mut file = crate::workshop::open_plain(path).map_err(|e| PkgError::Io(e.to_string()))?;
    let len = file.metadata().map_err(PkgError::from)?.len();
    analyze(&mut file, len)
}

/// Reads a loose `scene.json` (a project that is not packed), with no textures to look at.
pub fn analyze_loose_scene(path: &Path) -> Result<SceneInfo, PkgError> {
    let file = crate::workshop::open_plain(path).map_err(|e| PkgError::Io(e.to_string()))?;
    let len = file.metadata().map_err(PkgError::from)?.len();
    if len > MAX_SCENE_JSON {
        return Err(PkgError::OutOfBounds("scene.json is too large"));
    }
    let mut bytes = Vec::new();
    File::take(file, MAX_SCENE_JSON).read_to_end(&mut bytes)?;
    let scene = analyze_scene_json(&bytes)?;
    Ok(SceneInfo {
        moves: scene.effects || scene.particles || scene.animation || scene.scripts || scene.camera_shake,
        authored: scene.authored,
        has_audio: scene.sound,
        package_bytes: len,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Builds a package the way Wallpaper Engine lays it out: version, entry table, data.
    pub(crate) fn package(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        let version = b"PKGV0001";
        out.extend((version.len() as u32).to_le_bytes());
        out.extend(version);
        out.extend((entries.len() as u32).to_le_bytes());
        let mut offset = 0u32;
        for (name, data) in entries {
            out.extend((name.len() as u32).to_le_bytes());
            out.extend(name.as_bytes());
            out.extend(offset.to_le_bytes());
            out.extend((data.len() as u32).to_le_bytes());
            offset += data.len() as u32;
        }
        for (_, data) in entries {
            out.extend(*data);
        }
        out
    }

    /// A texture header: two version tags, the format, the flags, then the sizes.
    pub(crate) fn tex(flags: u32) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(b"TEXV0005\0");
        out.extend(b"TEXI0001\0");
        out.extend(0u32.to_le_bytes()); // format
        out.extend(flags.to_le_bytes());
        out.extend([0u8; 40]);
        out
    }

    fn analyze_bytes(bytes: &[u8]) -> Result<SceneInfo, PkgError> {
        analyze(&mut Cursor::new(bytes), bytes.len() as u64)
    }

    const STILL: &str = r#"{"general":{"orthogonalprojection":{"width":1920,"height":1080}},"objects":[{"image":"models/a.json","origin":"0 0 0"}]}"#;

    #[test]
    fn reads_the_index_and_every_entry_inside_the_file() {
        let bytes = package(&[("scene.json", STILL.as_bytes()), ("materials/a.tex", &tex(0))]);
        let (version, entries) = read_index(&mut Cursor::new(&bytes), bytes.len() as u64).unwrap();
        assert_eq!(version, "PKGV0001");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "scene.json");
        assert_eq!(entries[0].len, STILL.len() as u64);
        assert!(entries.iter().all(|entry| entry.start + entry.len <= bytes.len() as u64));
        let json = read_entry(&mut Cursor::new(&bytes), &entries[0], MAX_SCENE_JSON).unwrap();
        assert_eq!(json, STILL.as_bytes());
        assert_eq!(read_entry(&mut Cursor::new(&bytes), &entries[0], 5).unwrap(), &STILL.as_bytes()[..5], "a limit reads only the start");
    }

    #[test]
    fn a_picture_with_nothing_that_changes_is_still() {
        let bytes = package(&[("scene.json", STILL.as_bytes()), ("materials/a.tex", &tex(0))]);
        let info = analyze_bytes(&bytes).unwrap();
        assert!(!info.moves);
        assert_eq!(info.authored, Authored::Fixed { width: 1920, height: 1080 });
        assert!(!info.has_audio);
    }

    #[test]
    fn anything_that_changes_by_itself_makes_the_scene_move() {
        let cases = [
            (r#"{"objects":[{"effects":[{"file":"effects/waterripple/effect.json"}]}]}"#, "an effect"),
            (r#"{"objects":[{"particle":"particles/rain.json"}]}"#, "particles"),
            (r#"{"objects":[{"animationlayers":[{"animation":1}]}]}"#, "animation layers"),
            (r#"{"objects":[{"puppet":"models/x.mdl"}]}"#, "a puppet"),
            (r#"{"objects":[{"visible":{"script":"'use strict'; export function update(){}"}}]}"#, "a script"),
            (r#"{"general":{"camerashake":true},"objects":[]}"#, "camera shake"),
        ];
        for (json, what) in cases {
            let bytes = package(&[("scene.json", json.as_bytes())]);
            assert!(analyze_bytes(&bytes).unwrap().moves, "{what}");
        }
        // The quiet forms of the same keys do not move anything.
        for json in [
            r#"{"objects":[{"animationlayers":[]}]}"#,
            r#"{"general":{"camerashake":false},"objects":[]}"#,
            r#"{"objects":[{"visible":{"script":"  "}}]}"#,
            r#"{"objects":[{"image":"effects-are-nice.json"}]}"#,
        ] {
            let bytes = package(&[("scene.json", json.as_bytes())]);
            assert!(!analyze_bytes(&bytes).unwrap().moves, "{json}");
        }
    }

    #[test]
    fn a_sprite_sheet_texture_makes_an_otherwise_still_scene_move() {
        let still = package(&[("scene.json", STILL.as_bytes()), ("materials/a.tex", &tex(0)), ("materials/b.tex", &tex(1))]);
        assert!(!analyze_bytes(&still).unwrap().moves);
        let sheet = package(&[("scene.json", STILL.as_bytes()), ("materials/a.tex", &tex(0)), ("materials/b.tex", &tex(1 | 4))]);
        assert!(analyze_bytes(&sheet).unwrap().moves, "the flag for animation frames");
        assert!(tex_is_sprite_sheet(&tex(4)) && !tex_is_sprite_sheet(&tex(3)));
        assert!(!tex_is_sprite_sheet(b"TEXV0005"), "a header too short to carry flags");
        assert!(!tex_is_sprite_sheet(&[0u8; 64]), "not a texture at all");
    }

    #[test]
    fn keys_that_differ_only_in_case_are_both_read() {
        // A hand-edited scene can hold "Particle" and "particle" in one object.
        let json = r#"{"General":{"OrthogonalProjection":{"Width":3840,"Height":2160}},"objects":[{"Particle":"a","particle":"b"}]}"#;
        let bytes = package(&[("scene.json", json.as_bytes())]);
        let info = analyze_bytes(&bytes).unwrap();
        assert!(info.moves);
        assert_eq!(info.authored, Authored::Fixed { width: 3840, height: 2160 });
    }

    #[test]
    fn the_authored_size_is_fixed_automatic_or_perspective() {
        let authored = |json: &str| analyze_scene_json(json.as_bytes()).unwrap().authored;
        assert_eq!(authored(r#"{"general":{"orthogonalprojection":{"width":3440,"height":1440}}}"#), Authored::Fixed { width: 3440, height: 1440 });
        assert_eq!(authored(r#"{"general":{"orthogonalprojection":{"width":1872.0,"height":925.6}}}"#), Authored::Fixed { width: 1872, height: 926 });
        assert_eq!(authored(r#"{"general":{"orthogonalprojection":{"auto":true,"width":10,"height":10}}}"#), Authored::Auto);
        assert_eq!(authored(r#"{"general":{"orthogonalprojection":{"width":0,"height":1080}}}"#), Authored::Auto, "a size that is not one");
        assert_eq!(authored(r#"{"general":{"orthogonalprojection":{"width":1e9,"height":1e9}}}"#), Authored::Auto);
        assert_eq!(authored(r#"{"general":{"fov":50}}"#), Authored::Perspective);
        assert_eq!(authored(r#"{"objects":[]}"#), Authored::Perspective);
    }

    #[test]
    fn sound_is_found_in_the_files_and_in_the_scene() {
        let with_file = package(&[("scene.json", STILL.as_bytes()), ("sounds/rain.MP3", b"x")]);
        assert!(analyze_bytes(&with_file).unwrap().has_audio);
        let with_listed = package(&[("scene.json", br#"{"objects":[{"sound":["sounds/a.ogg"]}]}"#)]);
        assert!(analyze_bytes(&with_listed).unwrap().has_audio);
        let silent = package(&[("scene.json", br#"{"objects":[{"sound":[]}]}"#), ("materials/a.tex", &tex(0))]);
        assert!(!analyze_bytes(&silent).unwrap().has_audio);
    }

    // ----- malformed and hostile packages -----

    fn assert_bounded(bytes: &[u8], what: &str) {
        match analyze_bytes(bytes) {
            Err(PkgError::OutOfBounds(_)) | Err(PkgError::Malformed(_)) => {}
            other => panic!("{what}: expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn an_entry_count_beyond_the_limit_or_the_file_is_refused_before_allocating() {
        let mut huge = Vec::new();
        huge.extend(8u32.to_le_bytes());
        huge.extend(b"PKGV0001");
        huge.extend(u32::MAX.to_le_bytes());
        assert_bounded(&huge, "u32::MAX entries");
        let mut over = huge.clone();
        over.truncate(12);
        over.extend((MAX_ENTRIES + 1).to_le_bytes());
        let mut over_limit = Vec::new();
        over_limit.extend(8u32.to_le_bytes());
        over_limit.extend(b"PKGV0001");
        over_limit.extend((MAX_ENTRIES + 1).to_le_bytes());
        assert_bounded(&over_limit, "one more than the limit");
        // Within the limit but more than the file can hold.
        let mut lying = Vec::new();
        lying.extend(8u32.to_le_bytes());
        lying.extend(b"PKGV0001");
        lying.extend(MAX_ENTRIES.to_le_bytes());
        lying.extend([0u8; 64]);
        assert_bounded(&lying, "a count the file cannot hold");
    }

    #[test]
    fn an_offset_or_length_past_the_end_is_refused() {
        let mut bytes = package(&[("scene.json", STILL.as_bytes())]);
        // The table is: name length, name, offset, length. Point the length far past the end.
        let at = 4 + 8 + 4 + 4 + "scene.json".len() + 4;
        let past = bytes.len() as u32 + 10;
        bytes[at..at + 4].copy_from_slice(&past.to_le_bytes());
        assert_bounded(&bytes, "a length past the end");
        let mut bytes = package(&[("scene.json", STILL.as_bytes())]);
        let at = 4 + 8 + 4 + 4 + "scene.json".len();
        bytes[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_bounded(&bytes, "an offset past the end");
    }

    #[test]
    fn a_long_name_a_long_version_and_a_truncated_table_are_refused() {
        let mut long_name = Vec::new();
        long_name.extend(8u32.to_le_bytes());
        long_name.extend(b"PKGV0001");
        long_name.extend(1u32.to_le_bytes());
        long_name.extend((MAX_NAME_BYTES + 1).to_le_bytes());
        long_name.extend(vec![b'a'; 2048]);
        long_name.extend([0u8; 8]);
        assert_bounded(&long_name, "a name over 1 KB");

        let mut long_version = Vec::new();
        long_version.extend((MAX_VERSION_BYTES + 1).to_le_bytes());
        long_version.extend(vec![b'P'; 64]);
        assert_bounded(&long_version, "a long version");
        assert_bounded(&[0, 0, 0, 0], "an empty version");

        let full = package(&[("scene.json", STILL.as_bytes())]);
        for cut in [0usize, 3, 7, 11, 15, 20, 27, 33] {
            assert_bounded(&full[..cut], &format!("truncated at {cut}"));
        }
        let name_cut = 4 + 8 + 4 + 4 + 5; // inside the first entry's name
        assert_bounded(&full[..name_cut], "a truncated name");
        assert_bounded(b"not a package at all, just text that is long enough", "not a package");
    }

    #[test]
    fn a_package_without_a_scene_or_with_a_huge_or_broken_one_is_refused() {
        assert_bounded(&package(&[("materials/a.tex", &tex(0))]), "no scene.json");
        assert_bounded(&package(&[("scene.json", b"{ not json")]), "broken JSON");
        assert_bounded(&package(&[("scene.json", &[0xff, 0xfe, 0xfd])]), "not text");
        // An entry that claims more than the 4 MB scene limit is refused without being read.
        let mut bytes = package(&[("scene.json", STILL.as_bytes())]);
        let at = 4 + 8 + 4 + 4 + "scene.json".len() + 4;
        bytes[at..at + 4].copy_from_slice(&(MAX_SCENE_JSON as u32 + 1).to_le_bytes());
        bytes.resize(bytes.len() + MAX_SCENE_JSON as usize + 1, 0);
        assert_bounded(&bytes, "a scene.json over 4 MB");
    }

    #[test]
    fn deeply_nested_json_is_read_to_a_limit_not_to_the_end_of_the_stack() {
        let depth = 5_000;
        let mut json = String::new();
        for _ in 0..depth {
            json.push('[');
        }
        for _ in 0..depth {
            json.push(']');
        }
        // serde_json refuses nesting this deep itself; either way the package is refused or read
        // without a crash, never run away.
        let bytes = package(&[("scene.json", json.as_bytes())]);
        let _ = analyze_bytes(&bytes);
    }

    #[test]
    fn a_loose_scene_file_is_read_the_same_way() {
        let dir = crate::download_tests::temp_dir("scene-loose");
        std::fs::write(dir.join("scene.json"), r#"{"general":{"orthogonalprojection":{"width":1280,"height":720}},"objects":[{"particle":"p"}]}"#).unwrap();
        let info = analyze_loose_scene(&dir.join("scene.json")).unwrap();
        assert!(info.moves);
        assert_eq!(info.authored, Authored::Fixed { width: 1280, height: 720 });
        std::fs::write(dir.join("broken.json"), b"{").unwrap();
        assert!(analyze_loose_scene(&dir.join("broken.json")).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Live, read only: set `CLAWS_TEST_SCENES` to Workshop item ids separated by `;` (or to `all`).
    /// Reads each item's real `scene.pkg` and prints what Claws would say about it.
    #[test]
    fn reads_real_scene_packages_when_given() {
        let Some(ids) = std::env::var_os("CLAWS_TEST_SCENES") else { return };
        let Some(root) = crate::steam::steam_root() else { panic!("Steam is not installed") };
        let libraries = crate::steam::library_folders(&root);
        let ids = ids.to_string_lossy().into_owned();
        let all: Vec<String> = if ids == "all" {
            let mut found = Vec::new();
            for library in &libraries {
                if let Ok(entries) = std::fs::read_dir(library.join("steamapps/workshop/content/431960")) {
                    found.extend(entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()));
                }
            }
            found
        } else {
            ids.split(';').map(str::to_string).collect()
        };
        let (mut scenes, mut moving, mut still, mut sound, mut failed) = (0, 0, 0, 0, 0);
        let mut by_size: std::collections::BTreeMap<String, u32> = Default::default();
        for id in all.iter().filter(|id| !id.is_empty()) {
            let Ok(id) = id.parse::<u64>() else { continue };
            let Some(project) = crate::steam::locate_item(&libraries, id, None).project else { continue };
            let pkg = project.join("scene.pkg");
            if !pkg.is_file() {
                continue;
            }
            scenes += 1;
            match analyze_file(&pkg) {
                Ok(info) => {
                    if info.moves {
                        moving += 1;
                    } else {
                        still += 1;
                        eprintln!("still: {id} {:?}", info.authored);
                    }
                    if info.has_audio {
                        sound += 1;
                    }
                    let key = match info.authored {
                        Authored::Fixed { width, height } => format!("{width}x{height}"),
                        Authored::Auto => "auto".into(),
                        Authored::Perspective => "perspective".into(),
                    };
                    *by_size.entry(key).or_default() += 1;
                    if ids != "all" {
                        eprintln!("{id}: {info:?}");
                    }
                }
                Err(error) => {
                    failed += 1;
                    eprintln!("{id}: {error:?}");
                }
            }
        }
        eprintln!("scenes {scenes}: moving {moving}, still {still}, with sound {sound}, unreadable {failed}");
        let mut sizes: Vec<_> = by_size.into_iter().collect();
        sizes.sort_by_key(|entry| std::cmp::Reverse(entry.1));
        eprintln!("authored sizes: {:?}", &sizes[..sizes.len().min(12)]);
        assert_eq!(failed, 0, "every real package reads");
    }
}
