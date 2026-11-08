//! Transcripts made from a site's own captions: yt-dlp fetches one caption track (no
//! media), and this module reads it (WebVTT or SubRip), cleans it and writes the text the
//! user asked for. Nothing here recognises speech; a transcript is only as good as the
//! captions it comes from, and automatic captions can contain errors.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::downloader::DownloadOptions;

/// What a transcript download saves. Part of the job's options, so a paused or retried job
/// never changes it. Jobs saved before transcripts existed read as `Txt` (ignored for
/// video and audio).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum TranscriptFormat {
    /// Plain text, one caption per line, a blank line at each long pause.
    #[default]
    Txt,
    /// Plain text with `[hh:mm:ss]` before each caption.
    TxtTimestamps,
    /// Clean SubRip cues with the original timings.
    Srt,
    /// Clean WebVTT cues with the original timings.
    Vtt,
}

impl TranscriptFormat {
    pub fn extension(self) -> &'static str {
        match self {
            TranscriptFormat::Txt | TranscriptFormat::TxtTimestamps => "txt",
            TranscriptFormat::Srt => "srt",
            TranscriptFormat::Vtt => "vtt",
        }
    }

    /// What sets this format's file apart from another format with the same extension.
    pub fn name_suffix(self) -> &'static str {
        match self {
            TranscriptFormat::TxtTimestamps => ", timestamps",
            _ => "",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TranscriptFormat::Txt => "TXT",
            TranscriptFormat::TxtTimestamps => "TXT with timestamps",
            TranscriptFormat::Srt => "SRT",
            TranscriptFormat::Vtt => "VTT",
        }
    }
}

pub const EMPTY: &str = "These captions are empty.";

/// A caption file larger than this is not a caption file.
const MAX_CAPTION_BYTES: u64 = 64 << 20;
/// Cues closer than this count as adjacent (rolling captions follow each other exactly).
const ADJACENT_MS: u64 = 50;
/// A cue this short only repeats what is on screen while captions roll up.
const SNAPSHOT_MS: u64 = 50;
/// A pause at least this long starts a new paragraph in plain text.
const PARAGRAPH_GAP_MS: u64 = 3000;
/// Written files use Windows line endings, so every editor shows the lines.
const EOL: &str = "\r\n";

/// One caption after cleaning.
#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    pub start_ms: u64,
    pub end_ms: u64,
    pub lines: Vec<Line>,
    /// Bytes at the start of the first line that only repeat the previous cue's last line
    /// (roll-up captions growing word by word). Plain text continues the previous line with
    /// the rest; SRT and VTT keep the cue whole, so every cue still reads on its own.
    pub repeated: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub text: String,
    /// Starts a new speaker's turn (a voice tag or a dialogue dash): kept on its own line.
    pub turn: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Transcript {
    pub cues: Vec<Cue>,
    /// Cues that could not be read (no or broken timing) and were left out.
    pub skipped: usize,
}

/// A caption file read into clean cues: markup removed and entities decoded. Never fails;
/// an empty `cues` means there is nothing to save.
///
/// The repeats of rolling captions are removed from automatic captions (`automatic`). In
/// human captions a repeat is removed only where that pair of cues shows it rolls itself
/// (see `collapse_rolling`), never because of something elsewhere in the file: a speaker who
/// says the same words again at the start of the next caption keeps them.
pub fn parse(text: &str, automatic: bool) -> Transcript {
    let (raw, skipped) = read_cues(text);
    Transcript { cues: collapse_rolling(raw, automatic), skipped }
}

/// The finished text for `format`, or `EMPTY` when no caption has any text.
pub fn convert(text: &str, format: TranscriptFormat, automatic: bool) -> Result<(String, Transcript), String> {
    let transcript = parse(text, automatic);
    if transcript.cues.is_empty() {
        return Err(EMPTY.into());
    }
    Ok((render(&transcript, format), transcript))
}

// ---------------------------------------------------------------------------------------
// Reading

struct RawCue {
    start_ms: u64,
    end_ms: u64,
    lines: Vec<Line>,
    /// Some payload line holds a word-timing tag (`<00:00:01.234>`).
    timed: bool,
}

/// `[hh:]mm:ss(.|,)fff`, as WebVTT and SubRip write it.
fn timestamp(text: &str) -> Option<u64> {
    let text = text.trim();
    let (clock, fraction) = match text.find(['.', ',']) {
        Some(i) => (&text[..i], &text[i + 1..]),
        None => (text, ""),
    };
    let parts: Vec<&str> = clock.split(':').collect();
    let number = |s: &str| (!s.is_empty() && s.len() <= 9 && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse::<u64>().ok()).flatten();
    let (h, m, s) = match parts.as_slice() {
        [h, m, s] => (number(h)?, number(m)?, number(s)?),
        [m, s] => (0, number(m)?, number(s)?),
        _ => return None,
    };
    if m >= 60 || s >= 60 || fraction.len() > 3 || !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let ms = if fraction.is_empty() { 0 } else { fraction.parse::<u64>().ok()? * 10u64.pow(3 - fraction.len() as u32) };
    Some(((h * 60 + m) * 60 + s) * 1000 + ms)
}

/// A timing line: `start --> end [settings]`. `None` when it has no arrow; `Some(None)`
/// when it has one but cannot be read.
fn timing(line: &str) -> Option<Option<(u64, u64)>> {
    let (left, right) = line.split_once("-->")?;
    let end = right.split_whitespace().next().unwrap_or("");
    Some(match (timestamp(left), timestamp(end)) {
        (Some(start), Some(end)) if end >= start => Some((start, end)),
        _ => None,
    })
}

/// A line that is meant as a timing line: it reads as one, or it has an arrow after
/// something shaped like a timestamp (`00:00:0x,000 --> ...`). Inside a cue, any other
/// line with an arrow is caption text (`Press A --> B`).
fn timing_shaped(line: &str) -> bool {
    match timing(line) {
        Some(Some(_)) => true,
        Some(None) => {
            let left = line.split_once("-->").map_or("", |(left, _)| left.trim());
            left.starts_with(|c: char| c.is_ascii_digit()) && left.contains(':')
        }
        None => false,
    }
}

fn is_index(line: &str) -> bool {
    !line.is_empty() && line.bytes().all(|b| b.is_ascii_digit())
}

/// Whether a payload line holds a word-timing tag (`<00:00:01.234>`), as rolling
/// automatic captions do.
fn has_word_timing(line: &str) -> bool {
    let mut rest = line;
    while let Some(at) = rest.find('<') {
        let tail = &rest[at + 1..];
        if tail.find('>').is_some_and(|end| timestamp(&tail[..end]).is_some()) {
            return true;
        }
        rest = tail;
    }
    false
}

/// Reads every cue in file order. Returns the cues and how many were unreadable.
fn read_cues(text: &str) -> (Vec<RawCue>, usize) {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text).replace("\r\n", "\n").replace('\r', "\n");
    let mut cues = Vec::new();
    let mut skipped = 0;
    let mut first = true;
    let mut webvtt = false;
    let mut seen_timing = false;
    // Only an empty line ends a cue: YouTube's captions hold lines of a single space.
    for block in text.split('\n').collect::<Vec<_>>().split(|l| l.is_empty()) {
        // A block of nothing but spaces separates cues like an empty line; it is no cue.
        if block.iter().all(|l| l.trim().is_empty()) {
            continue;
        }
        let mut block = block;
        let is_first = std::mem::replace(&mut first, false);
        if is_first && block[0].trim().starts_with("WEBVTT") {
            // The WebVTT header. Cues may follow it without a blank line.
            webvtt = true;
            match block.iter().position(|l| timing(l).is_some()) {
                Some(at) => block = &block[at..],
                None => continue,
            }
        }
        let head = block[0].trim();
        // Comments, style sheets and regions are not captions.
        if head == "NOTE" || head.starts_with("NOTE ") || head.starts_with("NOTE\t") {
            continue;
        }
        if !head.contains("-->") && matches!(head, "STYLE" | "REGION") {
            continue;
        }
        let has_timing = block.iter().any(|l| timing(l).is_some());
        // In WebVTT, style sheets and regions come before the first cue. Text there with no
        // timing (such as the rest of a style sheet with a blank line in it) is not a caption.
        if webvtt && !seen_timing && !has_timing {
            continue;
        }
        seen_timing |= has_timing;
        // Several cues may run together without a blank line: a timing line (with an
        // optional identifier or SubRip index right before it) always starts a new cue.
        let mut current: Option<RawCue> = None;
        // After an unreadable timing line (or text with no cue), the lines that follow
        // belong to that unreadable cue and are left out with it.
        let mut in_broken = false;
        for (i, line) in block.iter().enumerate() {
            let mut arrow = timing(line);
            // Inside a cue, an arrow in text that is not shaped like a timing (and does not
            // follow an index) is caption text.
            if arrow == Some(None) && current.is_some() && !timing_shaped(line) && !(i > 0 && is_index(block[i - 1].trim())) {
                arrow = None;
            }
            match arrow {
                Some(Some((start_ms, end_ms))) => {
                    cues.extend(current.take());
                    current = Some(RawCue { start_ms, end_ms, lines: Vec::new(), timed: false });
                    in_broken = false;
                }
                Some(None) => {
                    cues.extend(current.take());
                    skipped += 1;
                    in_broken = true;
                }
                None => {
                    // An index followed by any arrow line starts a cue, readable or not.
                    let before_timing = block.get(i + 1).is_some_and(|next| timing(next).is_some());
                    if before_timing && (i == 0 || is_index(line.trim())) {
                        continue; // the cue's identifier or index
                    }
                    if let Some(cue) = current.as_mut() {
                        cue.timed |= has_word_timing(line);
                        cue.lines.extend(clean_line(line));
                    } else if !in_broken && !line.trim().is_empty() {
                        skipped += 1;
                        in_broken = true;
                    }
                }
            }
        }
        cues.extend(current);
    }
    (cues, skipped)
}

// ---------------------------------------------------------------------------------------
// Cleaning

fn entity(name: &str) -> Option<String> {
    let named = match name {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "nbsp" => " ",
        "lrm" => "\u{200e}",
        "rlm" => "\u{200f}",
        _ => {
            let code = if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
                u32::from_str_radix(hex, 16).ok()?
            } else {
                name.strip_prefix('#')?.parse::<u32>().ok()?
            };
            return char::from_u32(code).filter(|c| *c != '\0').map(String::from);
        }
    };
    Some(named.to_string())
}

fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at + 1..];
        let decoded = tail.find(';').filter(|end| *end > 0 && *end <= 10).and_then(|end| entity(&tail[..end]).map(|d| (d, end)));
        match decoded {
            Some((value, end)) => {
                out.push_str(&value);
                rest = &tail[end + 1..];
            }
            None => {
                out.push('&');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Whether `tag` (the text between `<` and `>`) is caption markup: a WebVTT timestamp, or
/// one of the tags WebVTT and SubRip use. Anything else (`I <3 you > them`, `a<b and c>d`)
/// is caption text and is kept. Returns the tag's lower-case name and whether it closes.
fn markup_tag(tag: &str) -> Option<(String, bool)> {
    if timestamp(tag).is_some() {
        return Some((String::new(), false));
    }
    let (closing, body) = match tag.strip_prefix('/') {
        Some(body) => (true, body),
        None => (false, tag),
    };
    let name_len = body.find(|c: char| !c.is_ascii_alphanumeric()).unwrap_or(body.len());
    let name = body[..name_len].to_ascii_lowercase();
    let rest = &body[name_len..];
    // Classes (`<c.yellow>`) carry no spaces; only some tags take an annotation or
    // attributes after a space (`<v Speaker>`, `<lang en>`, `<font color="…">`).
    let classes = rest.starts_with('.') && !rest.contains(char::is_whitespace);
    let fits = if closing {
        rest.trim_end().is_empty()
    } else {
        match name.as_str() {
            "c" | "i" | "b" | "u" | "ruby" | "rt" | "rp" => rest.is_empty() || classes,
            // `<v.loud Esme>`: classes, then the annotation.
            "v" | "lang" | "span" => rest.is_empty() || rest.starts_with('.') || rest.starts_with(char::is_whitespace),
            "font" => rest.is_empty() || rest.starts_with(char::is_whitespace),
            _ => false,
        }
    };
    let known = matches!(name.as_str(), "c" | "i" | "b" | "u" | "ruby" | "rt" | "rp" | "v" | "lang" | "font" | "span");
    (known && fits).then_some((name, closing))
}

/// One stretch of a line and who says it.
struct Segment {
    speaker: Option<String>,
    text: String,
}

/// Removes WebVTT/HTML tags, ruby annotations and ASS override blocks. Each voice tag
/// (`<v Speaker>`) starts a new segment credited to its speaker, and its closing tag ends
/// it, so text before, between and after voices is never credited to the wrong person.
fn strip_markup(line: &str) -> Vec<Segment> {
    let mut segments = vec![Segment { speaker: None, text: String::with_capacity(line.len()) }];
    let mut rest = line;
    let mut in_annotation = false;
    loop {
        let out = &mut segments.last_mut().expect("never empty").text;
        let Some(at) = rest.find(['<', '{']) else {
            if !in_annotation {
                out.push_str(rest);
            }
            break;
        };
        if !in_annotation {
            out.push_str(&rest[..at]);
        }
        let tail = &rest[at..];
        if let Some(after_brace) = tail.strip_prefix('{') {
            // `{\an8}` and similar SubRip/ASS positioning codes.
            match after_brace.strip_prefix('\\').and_then(|t| t.find('}').map(|end| &t[end + 1..])) {
                Some(after_code) => rest = after_code,
                None => {
                    if !in_annotation {
                        out.push('{');
                    }
                    rest = after_brace;
                }
            }
            continue;
        }
        let found = tail.find('>').and_then(|end| markup_tag(&tail[1..end]).map(|tag| (end, tag)));
        match found {
            Some((end, (name, closing))) => {
                match (name.as_str(), closing) {
                    ("rt" | "rp", false) => in_annotation = true,
                    ("rt" | "rp", true) => in_annotation = false,
                    ("v", false) => {
                        // `<v Speaker>` or `<v.class Speaker>`.
                        let after = tail[2..end].trim_start_matches(|c: char| !c.is_whitespace());
                        let name = decode_entities(after.trim());
                        segments.push(Segment { speaker: (!name.is_empty()).then_some(name), text: String::new() });
                    }
                    ("v", true) => segments.push(Segment { speaker: None, text: String::new() }),
                    _ => {}
                }
                rest = &tail[end + 1..];
            }
            None => {
                if !in_annotation {
                    out.push('<');
                }
                rest = &tail[1..];
            }
        }
    }
    segments
}

/// Runs of spaces and tabs as one space.
fn collapse_spaces(text: &str) -> String {
    let mut collapsed = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars() {
        if c == ' ' || c == '\t' {
            space = true;
            continue;
        }
        if space && !collapsed.is_empty() {
            collapsed.push(' ');
        }
        space = false;
        collapsed.push(c);
    }
    collapsed
}

/// One payload line as plain text: markup removed, entities decoded, runs of spaces
/// collapsed. Characters are otherwise kept exactly (no Unicode normalisation), so
/// combining marks, right-to-left marks and emoji survive. Each voice on the line becomes a
/// line of its own, `Speaker: text`; text outside any voice stays unattributed. Empty for
/// an empty line.
fn clean_line(raw: &str) -> Vec<Line> {
    strip_markup(raw)
        .into_iter()
        .filter_map(|segment| {
            let text = collapse_spaces(&decode_entities(&segment.text));
            let text = text.trim();
            if text.is_empty() {
                return None;
            }
            let dash = text.starts_with(['-', '\u{2010}', '\u{2013}', '\u{2014}']);
            Some(match segment.speaker {
                Some(name) => Line { text: format!("{name}: {text}"), turn: true },
                None => Line { text: text.to_string(), turn: dash },
            })
        })
        .collect()
}

/// Rolling captions (YouTube's automatic captions, roll-up broadcast captions) show the
/// previous line again at the top of the next cue, sometimes as a separate cue a few
/// milliseconds long. Only that carried-over text is removed, and only from a cue that
/// starts where the previous one ends: the repeated line must be the previous cue's tail,
/// followed by new text, or the cue must be a momentary snapshot. The same words said
/// again in a cue of their own are kept. A line that grows by a few words keeps its whole
/// text in the cue; `repeated` marks the part plain text leaves out.
///
/// Human captions (`automatic` false) are judged pair by pair, and only YouTube's own
/// rolling shapes count: a momentary snapshot that does nothing but repeat the previous
/// cue's last lines, or a cue with timed words whose first lines repeat the previous cue's
/// last lines before new text. One odd cue elsewhere in the file changes nothing, and a
/// line that only grows is kept whole in human captions.
fn collapse_rolling(raw: Vec<RawCue>, automatic: bool) -> Vec<Cue> {
    let mut out: Vec<Cue> = Vec::with_capacity(raw.len());
    let mut previous: Option<RawCue> = None;
    for cue in raw {
        if cue.lines.is_empty() {
            continue;
        }
        let mut lines = cue.lines.clone();
        let mut repeated = 0;
        if let Some(p) = previous.as_ref() {
            let adjacent = cue.start_ms >= p.start_ms && cue.start_ms <= p.end_ms + ADJACENT_MS;
            if adjacent {
                let carried = (1..=lines.len().min(p.lines.len()))
                    .rev()
                    .find(|&k| lines[..k].iter().zip(&p.lines[p.lines.len() - k..]).all(|(a, b)| a.text == b.text))
                    .unwrap_or(0);
                let snapshot = cue.end_ms.saturating_sub(cue.start_ms) <= SNAPSHOT_MS;
                let rolls = if automatic {
                    carried > 0 && (carried < lines.len() || snapshot)
                } else {
                    carried > 0 && ((snapshot && carried == lines.len()) || (cue.timed && carried < lines.len()))
                };
                if rolls {
                    lines.drain(..carried);
                } else if carried == 0 && automatic {
                    // Roll-up within one line: the line grows by a few words.
                    let last = &p.lines[p.lines.len() - 1].text;
                    if lines[0].text.strip_prefix(last.as_str()).is_some_and(|added| added.starts_with(' ')) {
                        repeated = last.len();
                    }
                }
            }
        }
        if !lines.is_empty() {
            out.push(Cue { start_ms: cue.start_ms, end_ms: cue.end_ms, lines, repeated });
        }
        previous = Some(cue);
    }
    out
}

// ---------------------------------------------------------------------------------------
// Writing

fn clock(ms: u64, separator: char) -> String {
    let (h, m, s, f) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000);
    format!("{h:02}:{m:02}:{s:02}{separator}{f:03}")
}

fn stamp(ms: u64) -> String {
    let (h, m, s) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60);
    format!("[{h:02}:{m:02}:{s:02}]")
}

/// Scripts written without spaces between words (Chinese, Japanese, Thai, Lao, Khmer,
/// Myanmar), with their punctuation and full-width forms. Korean uses spaces.
fn unspaced_script(c: char) -> bool {
    matches!(c as u32,
        0x0E00..=0x0EFF | 0x1000..=0x109F | 0x1780..=0x17FF | 0x2E80..=0x2FFF | 0x3000..=0x30FF
        | 0x31C0..=0x31FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0xFE30..=0xFE4F
        | 0xFF00..=0xFF60 | 0x20000..=0x3FFFF)
}

/// Joins a wrapped caption line to the text before it: with a space, except between two
/// characters of a script that uses none (`これは` + `字幕です` is `これは字幕です`).
fn push_joined(text: &mut String, next: &str) {
    let tight = text.chars().next_back().is_some_and(unspaced_script) && next.chars().next().is_some_and(unspaced_script);
    if !tight && !text.is_empty() {
        text.push(' ');
    }
    text.push_str(next);
}

/// A cue's lines as plain-text lines: lines of one turn joined, each new turn on its own.
/// The first `skip` bytes of the first line are left out.
fn joined(cue: &Cue, skip: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (i, line) in cue.lines.iter().enumerate() {
        let text = if i == 0 { line.text[skip..].trim_start() } else { line.text.as_str() };
        match out.last_mut() {
            Some(last) if !line.turn => push_joined(last, text),
            _ => out.push(text.to_string()),
        }
    }
    out
}

fn escape_vtt(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

pub fn render(transcript: &Transcript, format: TranscriptFormat) -> String {
    let mut out = String::new();
    match format {
        TranscriptFormat::Txt | TranscriptFormat::TxtTimestamps => {
            let timestamps = format == TranscriptFormat::TxtTimestamps;
            let mut lines: Vec<String> = Vec::new();
            let mut last_end: Option<u64> = None;
            for cue in &transcript.cues {
                let continues = cue.repeated > 0 && !lines.is_empty();
                let mut text = joined(cue, if continues { cue.repeated } else { 0 });
                if continues && !text.is_empty() {
                    let first = text.remove(0);
                    push_joined(lines.last_mut().expect("not empty"), &first);
                } else if !timestamps && last_end.is_some_and(|end| cue.start_ms >= end + PARAGRAPH_GAP_MS) {
                    lines.push(String::new());
                }
                for line in text {
                    lines.push(if timestamps { format!("{} {line}", stamp(cue.start_ms)) } else { line });
                }
                last_end = Some(cue.end_ms);
            }
            for line in lines {
                out.push_str(&line);
                out.push_str(EOL);
            }
        }
        TranscriptFormat::Srt => {
            for (i, cue) in transcript.cues.iter().enumerate() {
                out.push_str(&format!("{}{EOL}{} --> {}{EOL}", i + 1, clock(cue.start_ms, ','), clock(cue.end_ms, ',')));
                for line in &cue.lines {
                    out.push_str(&line.text);
                    out.push_str(EOL);
                }
                out.push_str(EOL);
            }
        }
        TranscriptFormat::Vtt => {
            out.push_str("WEBVTT");
            out.push_str(EOL);
            out.push_str(EOL);
            for cue in &transcript.cues {
                out.push_str(&format!("{} --> {}{EOL}", clock(cue.start_ms, '.'), clock(cue.end_ms, '.')));
                for line in &cue.lines {
                    out.push_str(&escape_vtt(&line.text));
                    out.push_str(EOL);
                }
                out.push_str(EOL);
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------
// The artifact in the work folder

/// The text file made for publication.
#[derive(Debug)]
pub struct Artifact {
    /// Relative to the work folder, beside the caption file.
    pub relative: PathBuf,
    pub skipped: usize,
}

/// The language as it appears in the file name: the code as the site names the track
/// (YouTube's `en-orig`, the original-language track, stays apart from its `en`), and
/// "auto" for automatic captions.
pub fn file_label(code: &str, auto: bool) -> String {
    if auto {
        format!("{code} auto")
    } else {
        code.to_string()
    }
}

/// The transcript's name: the caption file's (the frozen filename template's) stem, then
/// the language in brackets. `Talk [id].en.vtt` becomes `Talk [id] [en].txt`; text with
/// timestamps is `Talk [id] [en, timestamps].txt`. Every track and format a video offers
/// has its own name, so two different choices never compete for one file.
pub fn artifact_name(caption: &Path, code: &str, auto: bool, format: TranscriptFormat) -> Option<PathBuf> {
    let name = caption.file_name()?.to_str()?;
    let without_ext = Path::new(name).file_stem()?.to_str()?;
    let stem = without_ext.strip_suffix(&format!(".{code}")).unwrap_or(without_ext);
    let file = format!("{stem} [{}{}].{}", file_label(code, auto), format.name_suffix(), format.extension());
    Some(caption.parent().map(|p| p.join(&file)).unwrap_or_else(|| PathBuf::from(&file)))
}

/// Reads the caption file yt-dlp wrote (`caption`, relative to the work folder), writes the
/// transcript beside it and checks the result before anything is published.
pub fn write_artifact(workspace: &Path, caption: &Path, options: &DownloadOptions) -> Result<Artifact, String> {
    let code = options.subtitle_langs.first().map(String::as_str).unwrap_or("");
    let source = workspace.join(caption);
    let size = std::fs::metadata(&source).map_err(|e| format!("The captions file could not be read: {e}"))?.len();
    if size > MAX_CAPTION_BYTES {
        return Err("The captions file is too large to be captions.".into());
    }
    // Claws reads WebVTT and SubRip. Other site formats (srv3, json3, TTML) are turned into
    // WebVTT by yt-dlp, which needs FFmpeg; without it they arrive as they are.
    let ext = caption.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if !matches!(ext.as_str(), "vtt" | "srt") {
        let kind = if ext.is_empty() { "an unknown format".to_string() } else { ext.to_ascii_uppercase() };
        return Err(format!(
            "The site offers these captions only as {kind}, and turning that into a transcript needs FFmpeg. Install FFmpeg in Settings › Tools, then press Try Again."
        ));
    }
    let bytes = std::fs::read(&source).map_err(|e| format!("The captions file could not be read: {e}"))?;
    let (text, transcript) = convert(&String::from_utf8_lossy(&bytes), options.transcript_format, options.auto_subs)?;
    let relative = artifact_name(caption, code, options.auto_subs, options.transcript_format)
        .ok_or("The captions file has an unexpected name.")?;
    let target = workspace.join(&relative);
    std::fs::write(&target, text.as_bytes()).map_err(|e| format!("The transcript could not be written: {e}"))?;
    validate_artifact(&target)?;
    Ok(Artifact { relative, skipped: transcript.skipped })
}

/// The transcript for a finished caption run: `reported` is what yt-dlp named on its
/// CLAWSCAPTIONS line (`None` when it named nothing readable). The captions file must be
/// inside the work folder; the result is written beside it.
pub fn from_reported(workspace: &Path, reported: Option<&[String]>, options: &DownloadOptions) -> Result<Artifact, String> {
    let files = reported.ok_or("yt-dlp did not report a captions file.")?;
    let code = options.subtitle_langs.first().map(String::as_str).unwrap_or_default();
    let file = match files {
        [] => {
            let kind = if options.auto_subs { "automatic captions" } else { "captions" };
            return Err(format!("The site has no {kind} in “{code}” for this video now. Look up the link again to see which captions it offers."));
        }
        [one] => one,
        _ => return Err("yt-dlp saved more than one captions file for one language.".into()),
    };
    let caption = crate::download_files::owned_relative(workspace, Path::new(file))
        .ok_or("yt-dlp reported a captions file outside the download's work folder.")?;
    write_artifact(workspace, &caption, options)
}

/// A transcript is published only as a non-empty UTF-8 text file.
pub fn validate_artifact(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| format!("The transcript is missing: {e}"))?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err("The transcript is empty.".into());
    }
    let bytes = std::fs::read(path).map_err(|e| format!("The transcript could not be read back: {e}"))?;
    let text = std::str::from_utf8(&bytes).map_err(|_| "The transcript is not valid UTF-8 text.".to_string())?;
    if text.trim().is_empty() {
        return Err("The transcript is empty.".into());
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------
// Names

/// `--sub-langs` takes regular expressions; a language code is matched literally.
pub fn sub_langs(code: &str) -> String {
    let mut out = String::with_capacity(code.len() * 2);
    for c in code.chars() {
        if !c.is_ascii_alphanumeric() && c != '_' {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The English name of a caption language, for the Library; the code when unknown.
pub fn language_name(code: &str) -> String {
    let code = code.strip_suffix("-orig").unwrap_or(code);
    let known = |c: &str| -> Option<&'static str> {
        Some(match c {
            "en" => "English",
            "en-US" => "English (US)",
            "en-GB" => "English (UK)",
            "es" => "Spanish",
            "es-419" => "Spanish (Latin America)",
            "pt" => "Portuguese",
            "pt-BR" => "Portuguese (Brazil)",
            "pt-PT" => "Portuguese (Portugal)",
            "fr" => "French",
            "de" => "German",
            "it" => "Italian",
            "nl" => "Dutch",
            "ja" => "Japanese",
            "ko" => "Korean",
            "zh" => "Chinese",
            "zh-Hans" => "Chinese (Simplified)",
            "zh-Hant" => "Chinese (Traditional)",
            "zh-CN" => "Chinese (China)",
            "zh-TW" => "Chinese (Taiwan)",
            "ru" => "Russian",
            "uk" => "Ukrainian",
            "pl" => "Polish",
            "cs" => "Czech",
            "sv" => "Swedish",
            "da" => "Danish",
            "no" | "nb" => "Norwegian",
            "fi" => "Finnish",
            "el" => "Greek",
            "tr" => "Turkish",
            "ar" => "Arabic",
            "he" | "iw" => "Hebrew",
            "fa" => "Persian",
            "hi" => "Hindi",
            "bn" => "Bengali",
            "ur" => "Urdu",
            "th" => "Thai",
            "vi" => "Vietnamese",
            "id" => "Indonesian",
            "ms" => "Malay",
            "ro" => "Romanian",
            "hu" => "Hungarian",
            _ => return None,
        })
    };
    known(code)
        .or_else(|| code.split(['-', '_']).next().and_then(known))
        .map(String::from)
        .unwrap_or_else(|| code.to_string())
}

/// The Library's description of a transcript: `Transcript · English · TXT`, with
/// "(auto-generated)" for automatic captions and "original" for YouTube's original-language
/// track (`en-orig`), which is a different track from its `en`. src/transcript-options.ts
/// `transcriptQuality` writes the same words for the queue.
pub fn quality(options: &DownloadOptions) -> String {
    let code = options.subtitle_langs.first().map(String::as_str).unwrap_or("");
    let language = language_name(code);
    let language = match (code.ends_with("-orig"), options.auto_subs) {
        (true, true) => format!("{language} (original, auto-generated)"),
        (true, false) => format!("{language} (original)"),
        (false, true) => format!("{language} (auto-generated)"),
        (false, false) => language,
    };
    format!("Transcript · {language} · {}", options.transcript_format.label())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Human captions as plain text.
    fn txt(input: &str) -> String {
        convert(input, TranscriptFormat::Txt, false).unwrap().0
    }

    fn texts(input: &str) -> Vec<String> {
        lines_of(&parse(input, false))
    }

    fn lines_of(transcript: &Transcript) -> Vec<String> {
        transcript.cues.iter().flat_map(|c| c.lines.iter().map(|l| l.text.clone())).collect()
    }

    /// YouTube's automatic captions: each cue repeats the line before it, timed words are
    /// tagged, and a 10 ms cue shows the finished line again.
    const YOUTUBE_AUTO: &str = "WEBVTT\nKind: captions\nLanguage: en\n\n\
00:00:00.320 --> 00:00:02.389 align:start position:0%\n \nhello<00:00:00.640><c> everyone</c>\n\n\
00:00:02.389 --> 00:00:02.399 align:start position:0%\nhello everyone\n \n\n\
00:00:02.399 --> 00:00:05.030 align:start position:0%\nhello everyone\nand<00:00:02.800><c> welcome</c><00:00:03.100><c> back</c>\n\n\
00:00:05.030 --> 00:00:05.040 align:start position:0%\nand welcome back\n \n\n\
00:00:05.040 --> 00:00:07.000 align:start position:0%\nand welcome back\nwelcome<00:00:05.500><c> back</c>\n\n\
00:00:07.000 --> 00:00:07.010 align:start position:0%\nwelcome back\n \n";

    #[test]
    fn rolling_automatic_captions_lose_only_the_carried_over_lines() {
        // Timed words and snapshot cues show the track rolls, whichever kind it was chosen as.
        assert_eq!(parse(YOUTUBE_AUTO, false), parse(YOUTUBE_AUTO, true));
        let transcript = parse(YOUTUBE_AUTO, false);
        let lines: Vec<&str> = transcript.cues.iter().flat_map(|c| c.lines.iter().map(|l| l.text.as_str())).collect();
        // "welcome back" is said again in its own words after "and welcome back": kept.
        assert_eq!(lines, ["hello everyone", "and welcome back", "welcome back"]);
        assert_eq!(transcript.skipped, 0);
        let timings: Vec<(u64, u64)> = transcript.cues.iter().map(|c| (c.start_ms, c.end_ms)).collect();
        assert_eq!(timings, [(320, 2389), (2399, 5030), (5040, 7000)], "the original timings of the cues that carry new words");
        assert_eq!(txt(YOUTUBE_AUTO), "hello everyone\r\nand welcome back\r\nwelcome back\r\n");
    }

    #[test]
    fn the_same_words_said_again_later_or_for_a_while_are_kept() {
        let input = "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nYes.\n\n00:00:05.000 --> 00:00:06.000\nYes.\n\n\
00:00:06.000 --> 00:00:07.500\nYes.\n\n00:00:09.000 --> 00:00:10.000\nI said yes.\nI said yes.\n";
        assert_eq!(texts(input), ["Yes.", "Yes.", "Yes.", "I said yes.", "I said yes."]);
    }

    #[test]
    fn roll_up_lines_that_grow_keep_each_word_once() {
        let input = "WEBVTT\n\n00:00:00.000 --> 00:00:01.000\nwe are\n\n00:00:01.000 --> 00:00:02.000\nwe are going\n\n\
00:00:02.000 --> 00:00:03.000\nwe are going to the shop\n\n00:00:09.000 --> 00:00:10.000\nwe are going\n";
        let transcript = parse(input, true);
        let repeated: Vec<usize> = transcript.cues.iter().map(|c| c.repeated).collect();
        assert_eq!(repeated, [0, "we are".len(), "we are going".len(), 0], "the later, separate cue is kept whole");
        let auto_txt = convert(input, TranscriptFormat::Txt, true).unwrap().0;
        assert_eq!(auto_txt, "we are going to the shop\r\n\r\nwe are going\r\n");
        // SRT and VTT keep each cue's whole text with its timing: no cue reads "going" alone.
        let srt = convert(input, TranscriptFormat::Srt, true).unwrap().0;
        assert!(srt.starts_with("1\r\n00:00:00,000 --> 00:00:01,000\r\nwe are\r\n\r\n2\r\n00:00:01,000 --> 00:00:02,000\r\nwe are going\r\n\r\n3\r\n"), "{srt}");
        let vtt = convert(input, TranscriptFormat::Vtt, true).unwrap().0;
        assert!(vtt.contains("00:00:02.000 --> 00:00:03.000\r\nwe are going to the shop\r\n"), "{vtt}");
    }

    #[test]
    fn human_captions_keep_words_repeated_at_the_start_of_the_next_caption() {
        // Back-to-back human captions: the second "No." and "- Go!" were said again.
        let srt = "1\n00:00:01,000 --> 00:00:02,000\nNo.\n\n2\n00:00:02,000 --> 00:00:04,000\nNo. Absolutely not.\n\n\
3\n00:00:04,000 --> 00:00:05,000\nNo.\n\n4\n00:00:05,000 --> 00:00:06,000\nNo.\nI said no.\n\n\
5\n00:00:06,000 --> 00:00:07,000\n- Ready?\n- Go!\n\n6\n00:00:07,000 --> 00:00:08,000\n- Go!\n- Now!\n\n\
7\n00:00:08,000 --> 00:00:09,000\nGo\n\n8\n00:00:09,000 --> 00:00:10,000\nGo away\n";
        assert_eq!(texts(srt), ["No.", "No. Absolutely not.", "No.", "No.", "I said no.", "- Ready?", "- Go!", "- Go!", "- Now!", "Go", "Go away"]);
        assert_eq!(
            txt(srt),
            "No.\r\nNo. Absolutely not.\r\nNo.\r\nNo. I said no.\r\n- Ready?\r\n- Go!\r\n- Go!\r\n- Now!\r\nGo\r\nGo away\r\n"
        );
        let out = convert(srt, TranscriptFormat::Srt, false).unwrap().0;
        assert!(out.contains("8\r\n00:00:09,000 --> 00:00:10,000\r\nGo away\r\n"), "{out}");
    }

    #[test]
    fn one_odd_cue_elsewhere_does_not_make_human_captions_lose_repeated_words() {
        // A zero-length title cue somewhere in the file.
        let srt = "1\n00:00:01,000 --> 00:00:02,000\nNo.\n\n2\n00:00:02,000 --> 00:00:04,000\nNo. Absolutely not.\n\n\
3\n00:00:04,000 --> 00:00:05,000\nNo.\n\n4\n00:00:05,000 --> 00:00:06,000\nNo.\nI said no.\n\n\
5\n00:00:20,000 --> 00:00:20,000\nApplause\n";
        assert_eq!(txt(srt), "No.\r\nNo. Absolutely not.\r\nNo.\r\nNo. I said no.\r\n\r\nApplause\r\n");
        let title = "1\n00:00:01,000 --> 00:00:02,000\nNo.\n\n2\n00:00:02,000 --> 00:00:04,000\nNo. Absolutely not.\n\n3\n00:00:10,000 --> 00:00:10,000\nTitle\n";
        assert_eq!(txt(title), "No.\r\nNo. Absolutely not.\r\n\r\nTitle\r\n");
        // Word-timed lyrics: the second cue sings the words again and more.
        let karaoke = "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\n<00:00:01.000>la <00:00:01.500>la\n\n00:00:02.000 --> 00:00:03.000\nla la la la\n\n\
00:00:03.000 --> 00:00:04.000\nla la la la\n";
        assert_eq!(txt(karaoke), "la la\r\nla la la la\r\nla la la la\r\n");
        // A zero-length cue that only repeats the line before it is still left out.
        let snapshot = "1\n00:00:01,000 --> 00:00:02,000\nHello there\n\n2\n00:00:02,000 --> 00:00:02,000\nHello there\n\n3\n00:00:02,000 --> 00:00:03,000\nBye\n";
        assert_eq!(texts(snapshot), ["Hello there", "Bye"]);
    }

    #[test]
    fn an_arrow_in_caption_text_is_text() {
        let srt = "1\n00:00:01,000 --> 00:00:02,000\nPress A --> B to go on\nthen continue\n\n2\n00:00:02,000 --> 00:00:03,000\nNext\n\n\
3\n00:00:03,000 --> 00:00:04,000\nStep 5 --> done\n";
        let transcript = parse(srt, false);
        assert_eq!((lines_of(&transcript), transcript.skipped), (
            vec!["Press A --> B to go on".to_string(), "then continue".into(), "Next".into(), "Step 5 --> done".into()],
            0
        ));
        // Cues written back read back the same.
        for format in [TranscriptFormat::Srt, TranscriptFormat::Vtt] {
            let written = convert(srt, format, false).unwrap().0;
            assert_eq!(parse(&written, false).cues, transcript.cues, "{written}");
        }
        // A garbled timing line is still a broken cue, inside a block or after an index.
        let garbled = "1\n00:00:01,000 --> 00:00:02,000\nA\n00:00:0x,000 --> 00:00:03,000\nLost\n\n2\n00:00:04,000 --> 00:00:05,000\nB\n3\nnot a time --> 00:00:06,000\nLost too\n";
        let transcript = parse(garbled, false);
        assert_eq!((lines_of(&transcript), transcript.skipped), (vec!["A".to_string(), "B".into()], 2));
    }

    #[test]
    fn empty_captions_are_an_error() {
        for input in ["", "\u{feff}", "WEBVTT\n\n", "WEBVTT\n\nNOTE nothing here\n", "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\n \n\n00:00:02.000 --> 00:00:03.000\n<c> </c>\n", "1\n00:00:01,000 --> 00:00:02,000\n\n"] {
            assert_eq!(convert(input, TranscriptFormat::Txt, false).unwrap_err(), EMPTY, "{input:?}");
        }
        // Only unreadable cues: empty too, and counted.
        let broken = "1\n00:00:0x,000 --> 00:00:02,000\nText\n";
        assert_eq!(convert(broken, TranscriptFormat::Txt, false).unwrap_err(), EMPTY);
        assert_eq!(parse(broken, false).skipped, 1);
    }

    #[test]
    fn malformed_cues_are_skipped_and_counted_while_the_rest_survive() {
        let input = "1\n00:00:01,000 --> 00:00:02,000\nFirst\n\n\
2\n00:00:xx,000 --> 00:00:03,000\nBad timing\n\n\
Orphan text without a cue\n\n\
4\n00:00:05,000 --> 00:00:04,000\nEnds before it starts\n\n\
00:00:06,000 --> 00:00:07,000\nNo index\n\
7\n00:00:08,000 --> 00:00:09,000\nNo blank line before me\n";
        let transcript = parse(input, false);
        assert_eq!(texts(input), ["First", "No index", "No blank line before me"]);
        assert_eq!(transcript.skipped, 3);
    }

    #[test]
    fn blocks_of_spaces_and_style_sheets_are_not_counted_as_broken_captions() {
        let trailing = "1\n00:00:01,000 --> 00:00:02,000\nHi\n\n \n\n\t \n";
        assert_eq!((texts(trailing), parse(trailing, false).skipped), (vec!["Hi".to_string()], 0));
        let style = "WEBVTT\n\nSTYLE\n::cue { color: yellow }\n\n::cue(b) { color: red }\n\n00:00:01.000 --> 00:00:02.000\nHi\n";
        assert_eq!((texts(style), parse(style, false).skipped), (vec!["Hi".to_string()], 0));
        // Text with no timing after the first cue is still a broken caption.
        let orphan = "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nHi\n\nOrphan\n";
        assert_eq!(parse(orphan, false).skipped, 1);
    }

    #[test]
    fn cues_right_after_the_webvtt_header_are_read() {
        let input = "WEBVTT\nKind: captions\n00:00:01.000 --> 00:00:02.000\nHello\n\n00:00:02.000 --> 00:00:03.000\nThere\n";
        let transcript = parse(input, false);
        assert_eq!((lines_of(&transcript), transcript.skipped), (vec!["Hello".to_string(), "There".to_string()], 0));
        assert_eq!(txt("\u{feff}WEBVTT\r\n00:00:01.000 --> 00:00:02.000\r\nOnly\r\n"), "Only\r\n");
    }

    #[test]
    fn subrip_with_bom_crlf_and_formatting() {
        let input = "\u{feff}1\r\n00:00:01,500 --> 00:00:03,000\r\n<i>Hello</i>, <font color=\"#ffff00\">world</font>\r\n{\\an8}Up here\r\n\r\n2\r\n00:00:03,000 --> 00:00:04,000 X1:10 X2:20 Y1:5 Y2:6\r\n<b>Bold</b> &amp; <u>under</u>\r\n";
        let transcript = parse(input, false);
        assert_eq!(texts(input), ["Hello, world", "Up here", "Bold & under"]);
        assert_eq!((transcript.cues[0].start_ms, transcript.cues[0].end_ms), (1500, 3000));
        assert_eq!(txt(input), "Hello, world Up here\r\nBold & under\r\n");
        // Separator lines holding only spaces still separate cues.
        let sloppy = "1\n00:00:01,000 --> 00:00:02,000\nA\n  \n2\n00:00:02,000 --> 00:00:03,000\nB\n";
        assert_eq!(texts(sloppy), ["A", "B"]);
        assert_eq!(parse(sloppy, false).skipped, 0);
    }

    #[test]
    fn webvtt_blocks_settings_and_markup() {
        let input = "WEBVTT - A title\r\n\r\nSTYLE\r\n::cue { color: yellow }\r\n\r\nREGION\r\nid:fred width:40%\r\n\r\nNOTE a comment\r\nthat spans lines\r\n\r\n\
intro\r\n00:01.000 --> 00:02.000 line:0 position:20% align:start\r\n<v Roger Bingham>We are in New York City\r\n\r\n\
00:02.000 --> 00:03.000\r\n<v.loud Esme>It&apos;s a <c.yellow>blue</c> apple tree!</v>\r\n\r\n\
00:03.000 --> 00:04.000\r\n<ruby>漢<rt>かん</rt>字<rt>じ</rt></ruby> <lang en>and</lang> &lt;3 &#x1F600; caf&#233; I <3 you\r\n\r\n\
00:04.000 --> 00:05.000\r\n- Who?\r\n- Me.\r\n";
        let transcript = parse(input, false);
        assert_eq!(transcript.skipped, 0);
        assert_eq!(texts(input), ["Roger Bingham: We are in New York City", "Esme: It's a blue apple tree!", "漢字 and <3 😀 café I <3 you", "- Who?", "- Me."]);
        assert_eq!(transcript.cues[0].start_ms, 1000);
        assert_eq!(txt(input), "Roger Bingham: We are in New York City\r\nEsme: It's a blue apple tree!\r\n漢字 and <3 😀 café I <3 you\r\n- Who?\r\n- Me.\r\n");
    }

    #[test]
    fn text_that_only_looks_like_a_tag_is_kept() {
        let input = "1\n00:00:01,000 --> 00:00:02,000\nI <3 you > them\n\n2\n00:00:02,000 --> 00:00:03,000\nif a<b and c>d then x<5 and y>2\n\n\
3\n00:00:03,000 --> 00:00:04,000\n<I>Loud</I> <image> <Font Color=\"red\">red</font> <b.x>b</b>\n";
        assert_eq!(texts(input), ["I <3 you > them", "if a<b and c>d then x<5 and y>2", "Loud <image> red b"]);
        assert_eq!(parse(input, false).skipped, 0);
    }

    #[test]
    fn each_voice_on_a_line_is_credited_to_its_own_speaker() {
        let input = "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\n<v Ana>Hi.</v> <v Bob>Hello.</v>\n\n\
00:00:02.000 --> 00:00:03.000\nWell, <v Bob>I disagree\n\n\
00:00:03.000 --> 00:00:04.000\n<v.loud Ana Lima>Fine</v> then.\n";
        assert_eq!(texts(input), ["Ana: Hi.", "Bob: Hello.", "Well,", "Bob: I disagree", "Ana Lima: Fine", "then."]);
        assert_eq!(txt(input), "Ana: Hi.\r\nBob: Hello.\r\nWell,\r\nBob: I disagree\r\nAna Lima: Fine then.\r\n");
    }

    #[test]
    fn multilingual_text_is_kept_exactly_and_in_order() {
        let lines = [
            "これは日本語の字幕です。",
            "\u{200f}مرحبا بالعالم\u{200f}",
            "שלום עולם",
            "Ж\u{301}ёлтый — e\u{301} combining",
            "👩\u{200d}👩\u{200d}👧 family 🇺🇦",
            "中文　全角空格",
        ];
        let mut input = String::from("WEBVTT\n\n");
        for (i, line) in lines.iter().enumerate() {
            input.push_str(&format!("00:00:{:02}.000 --> 00:00:{:02}.500\n{line}\n\n", i * 2, i * 2));
        }
        assert_eq!(texts(&input), lines);
        let out = txt(&input);
        assert_eq!(out, lines.iter().map(|l| format!("{l}\r\n")).collect::<String>());
        // A sentence wrapped over two lines joins without a space where the script has none.
        let wrapped = "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nこれは\n字幕です\n\n00:00:02.000 --> 00:00:03.000\n我们\n今天\n\n\
00:00:03.000 --> 00:00:04.000\n日本語\nand English\n\n00:00:04.000 --> 00:00:05.000\n안녕하세요\n여러분\n";
        assert_eq!(txt(wrapped), "これは字幕です\r\n我们今天\r\n日本語 and English\r\n안녕하세요 여러분\r\n");
        // Every format keeps every character.
        for format in [TranscriptFormat::TxtTimestamps, TranscriptFormat::Srt, TranscriptFormat::Vtt] {
            let rendered = convert(&input, format, false).unwrap().0;
            for line in lines {
                assert!(rendered.contains(line), "{format:?} lost {line:?}");
            }
        }
    }

    #[test]
    fn each_output_format() {
        let input = "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nFirst line\nsecond half\n\n00:01:02.250 --> 00:01:04.000\nTom & Jerry <3\n\n01:00:00.000 --> 01:00:01.000\nLate\n";
        assert_eq!(txt(input), "First line second half\r\n\r\nTom & Jerry <3\r\n\r\nLate\r\n");
        assert_eq!(
            convert(input, TranscriptFormat::TxtTimestamps, false).unwrap().0,
            "[00:00:01] First line second half\r\n[00:01:02] Tom & Jerry <3\r\n[01:00:00] Late\r\n"
        );
        assert_eq!(
            convert(input, TranscriptFormat::Srt, false).unwrap().0,
            "1\r\n00:00:01,000 --> 00:00:02,000\r\nFirst line\r\nsecond half\r\n\r\n2\r\n00:01:02,250 --> 00:01:04,000\r\nTom & Jerry <3\r\n\r\n3\r\n01:00:00,000 --> 01:00:01,000\r\nLate\r\n\r\n"
        );
        let vtt = convert(input, TranscriptFormat::Vtt, false).unwrap().0;
        assert_eq!(
            vtt,
            "WEBVTT\r\n\r\n00:00:01.000 --> 00:00:02.000\r\nFirst line\r\nsecond half\r\n\r\n00:01:02.250 --> 00:01:04.000\r\nTom &amp; Jerry &lt;3\r\n\r\n01:00:00.000 --> 01:00:01.000\r\nLate\r\n\r\n"
        );
        assert!(!vtt.starts_with('\u{feff}'), "UTF-8 without a byte-order mark");
        // A clean file reads back to the same cues.
        assert_eq!(parse(&vtt, false).cues, parse(input, false).cues);
        let srt = convert(input, TranscriptFormat::Srt, false).unwrap().0;
        assert_eq!(parse(&srt, false).cues, parse(input, false).cues);
    }

    #[test]
    fn the_artifact_is_named_after_the_caption_file_and_checked() {
        let dir = crate::download_tests::temp_dir("transcript-artifact");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let caption = Path::new("sub").join("Talk： part 1 [abc].en.vtt");
        std::fs::write(dir.join(&caption), "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nHi\n").unwrap();
        let options = DownloadOptions { mode: "transcript".into(), subtitle_langs: vec!["en".into()], ..DownloadOptions::default() };
        let made = write_artifact(&dir, &caption, &options).unwrap();
        assert_eq!(made.relative, Path::new("sub").join("Talk： part 1 [abc] [en].txt"));
        assert_eq!(std::fs::read_to_string(dir.join(&made.relative)).unwrap(), "Hi\r\n");
        assert_eq!(made.skipped, 0);

        let auto = DownloadOptions { auto_subs: true, transcript_format: TranscriptFormat::Srt, subtitle_langs: vec!["en-orig".into()], ..options.clone() };
        assert_eq!(artifact_name(Path::new("T [x].en-orig.vtt"), "en-orig", true, TranscriptFormat::Srt).unwrap(), Path::new("T [x] [en-orig auto].srt"));
        std::fs::write(dir.join("T [x].en-orig.vtt"), "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nHi\n").unwrap();
        assert_eq!(write_artifact(&dir, Path::new("T [x].en-orig.vtt"), &auto).unwrap().relative, Path::new("T [x] [en-orig auto].srt"));
        // SubRip captions (a site's own, kept as they are when FFmpeg is missing) are read too.
        std::fs::write(dir.join("S.de.srt"), "1\n00:00:01,000 --> 00:00:02,000\nGuten Tag\n").unwrap();
        let german = DownloadOptions { subtitle_langs: vec!["de".into()], ..options.clone() };
        let made = write_artifact(&dir, Path::new("S.de.srt"), &german).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join(made.relative)).unwrap(), "Guten Tag\r\n");
        // Any other site format needs FFmpeg's conversion: a clear message, nothing written.
        std::fs::write(dir.join("J.en.srv3"), "<timedtext><p t=\"0\">hi</p></timedtext>").unwrap();
        let err = write_artifact(&dir, Path::new("J.en.srv3"), &options).unwrap_err();
        assert!(err.contains("SRV3") && err.contains("FFmpeg") && err.contains("Settings"), "{err}");
        assert!(!dir.join("J [en].txt").exists());
        // The original captions keep their own name: a VTT transcript never replaces them.
        assert_eq!(artifact_name(Path::new("T.de.vtt"), "de", false, TranscriptFormat::Vtt).unwrap(), Path::new("T [de].vtt"));

        std::fs::write(dir.join("empty.en.vtt"), "WEBVTT\n\n").unwrap();
        assert_eq!(write_artifact(&dir, Path::new("empty.en.vtt"), &options).unwrap_err(), EMPTY);
        assert!(!dir.join("empty [en].txt").exists(), "nothing is written for empty captions");

        std::fs::write(dir.join("zero.txt"), b"").unwrap();
        assert!(validate_artifact(&dir.join("zero.txt")).is_err());
        std::fs::write(dir.join("bad.txt"), [0xff, 0xfe, 0x41]).unwrap();
        assert!(validate_artifact(&dir.join("bad.txt")).unwrap_err().contains("UTF-8"));
        assert!(validate_artifact(&dir.join("missing.txt")).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn language_codes_are_matched_literally_and_named_for_the_library() {
        assert_eq!(sub_langs("en"), "en");
        assert_eq!(sub_langs("zh-Hans"), "zh\\-Hans");
        assert_eq!(sub_langs("en.x"), "en\\.x");
        assert_eq!(sub_langs("pt_BR"), "pt_BR");
        assert_eq!(language_name("en-orig"), "English");
        assert_eq!(language_name("zh-Hans"), "Chinese (Simplified)");
        assert_eq!(language_name("fr-CA"), "French");
        assert_eq!(language_name("xx-yy"), "xx-yy");
        let options = |code: &str, auto, transcript_format| DownloadOptions {
            mode: "transcript".into(),
            subtitle_langs: vec![code.into()],
            auto_subs: auto,
            transcript_format,
            ..DownloadOptions::default()
        };
        assert_eq!(quality(&options("en", false, TranscriptFormat::Txt)), "Transcript · English · TXT");
        assert_eq!(quality(&options("en", true, TranscriptFormat::TxtTimestamps)), "Transcript · English (auto-generated) · TXT with timestamps");
        assert_eq!(quality(&options("ja", false, TranscriptFormat::Vtt)), "Transcript · Japanese · VTT");
        assert_eq!(quality(&options("en-orig", true, TranscriptFormat::Txt)), "Transcript · English (original, auto-generated) · TXT");
    }

    #[test]
    fn every_track_and_format_of_a_video_has_its_own_file_name() {
        let tracks = [("en", false), ("en", true), ("en-orig", true), ("de", false), ("de-orig", true)];
        let formats = [TranscriptFormat::Txt, TranscriptFormat::TxtTimestamps, TranscriptFormat::Srt, TranscriptFormat::Vtt];
        let mut names = std::collections::HashSet::new();
        for (code, auto) in tracks {
            for format in formats {
                let name = artifact_name(Path::new(&format!("Talk [id].{code}.vtt")), code, auto, format).unwrap();
                assert!(names.insert(name.to_string_lossy().to_lowercase()), "{name:?} is taken twice");
            }
        }
        assert_eq!(artifact_name(Path::new("Talk.en.vtt"), "en", true, TranscriptFormat::TxtTimestamps).unwrap(), Path::new("Talk [en auto, timestamps].txt"));
    }

    #[test]
    fn timestamps_are_read_strictly() {
        assert_eq!(timestamp("01:02:03.004"), Some(3_723_004));
        assert_eq!(timestamp("02:03.4"), Some(123_400));
        assert_eq!(timestamp("00:00:01,250"), Some(1250));
        assert_eq!(timestamp("100:00:00.000"), Some(360_000_000));
        for bad in ["", "1", "00:60:00.000", "00:00:61.000", "aa:00.000", "00:00:01.0000", "-1:00.000"] {
            assert_eq!(timestamp(bad), None, "{bad:?}");
        }
    }
}
