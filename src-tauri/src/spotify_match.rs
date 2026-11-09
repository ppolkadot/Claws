use crate::error::{AppError, Result};
use crate::spotify::SpotifyTrack;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MatchOrigin {
    pub kind: String,
    pub spotify_url: String,
    pub spotify_title: Option<String>,
    pub spotify_artists: Vec<String>,
}

fn search_args(query: String) -> Result<Vec<String>> {
    let query = query.trim();
    if query.is_empty() || query.chars().count() > 300 || query.chars().any(char::is_control) {
        return Err(AppError::msg(
            "Enter an artist and song title using at most 300 characters.",
        ));
    }
    let mut args = [
        "--dump-single-json",
        "--flat-playlist",
        "--skip-download",
        "--no-warnings",
        "--socket-timeout",
        "20",
        "--",
    ]
    .map(String::from)
    .to_vec();
    args.push(format!("ytsearch5:{query}"));
    Ok(args)
}

fn bounded_text(text: &str, max: usize) -> bool {
    !text.trim().is_empty()
        && text == text.trim()
        && text.chars().count() <= max
        && !text.chars().any(char::is_control)
}

fn valid_spotify_identity(url: &str, id: Option<&str>) -> bool {
    let Some(crate::spotify::SpotifyInput::Track { id: parsed }) =
        crate::spotify::classify_spotify_url(url)
    else {
        return false;
    };
    url == format!("https://open.spotify.com/track/{parsed}") && id.is_none_or(|id| id == parsed)
}

fn validate_track(track: &SpotifyTrack) -> Result<()> {
    if !valid_spotify_identity(&track.url, Some(&track.id))
        || track.title.as_ref().is_some_and(|s| !bounded_text(s, 512))
        || track.artists.len() > 20
        || track.artists.iter().any(|s| !bounded_text(s, 256))
        || track
            .duration
            .is_some_and(|d| !d.is_finite() || d <= 0.0 || d > 86400.0)
        || track.artwork.as_ref().is_some_and(|s| {
            url::Url::parse(s).ok().is_none_or(|u| {
                u.scheme() != "https"
                    || !u.username().is_empty()
                    || u.password().is_some()
                    || u.port().is_some()
                    || !matches!(
                        u.host_str(),
                        Some("i.scdn.co" | "image-cdn-ak.spotifycdn.com")
                    )
            })
        })
    {
        return Err(AppError::msg(
            "Invalid Spotify track metadata. Open the track again.",
        ));
    }
    Ok(())
}

fn valid_video_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
}

fn youtube_id(raw: &str) -> Option<String> {
    if valid_video_id(raw) {
        return Some(raw.into());
    }
    let u = url::Url::parse(raw).ok()?;
    if u.scheme() != "https"
        || !u.username().is_empty()
        || u.password().is_some()
        || u.port().is_some()
    {
        return None;
    }
    let id = match u.host_str()? {
        "youtu.be" => u.path().strip_prefix('/')?.to_string(),
        "www.youtube.com" | "youtube.com" | "m.youtube.com" if u.path() == "/watch" => {
            u.query_pairs().find(|(k, _)| k == "v")?.1.into_owned()
        }
        _ => return None,
    };
    valid_video_id(&id).then_some(id)
}

pub fn validate_match_origin(
    origin: &MatchOrigin,
    url: &str,
    playlist_index: Option<u32>,
    options: &crate::downloader::DownloadOptions,
) -> Result<()> {
    let canonical_youtube =
        youtube_id(url).is_some_and(|id| url == format!("https://www.youtube.com/watch?v={id}"));
    if origin.kind != "spotifyYouTube"
        || !valid_spotify_identity(&origin.spotify_url, None)
        || origin
            .spotify_title
            .as_ref()
            .is_some_and(|s| !bounded_text(s, 512))
        || origin.spotify_artists.len() > 20
        || origin.spotify_artists.iter().any(|s| !bounded_text(s, 256))
        || options.mode != "audio"
        || playlist_index.is_some()
        || !canonical_youtube
    {
        return Err(AppError::msg(
            "A Spotify match needs one confirmed YouTube recording in audio mode.",
        ));
    }
    Ok(())
}

fn tokens(raw: &str) -> std::collections::BTreeSet<String> {
    raw.chars()
        .flat_map(char::to_lowercase)
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}

fn dice(a: &std::collections::BTreeSet<String>, b: &std::collections::BTreeSet<String>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    2.0 * a.intersection(b).count() as f64 / (a.len() + b.len()) as f64
}

fn has_category(title: &str, category: &str) -> bool {
    let normalized = title
        .chars()
        .flat_map(char::to_lowercase)
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>();
    let words: Vec<_> = normalized.split_whitespace().collect();
    if category == "sped up" {
        words.windows(2).any(|w| w == ["sped", "up"])
    } else {
        words.contains(&category)
    }
}

const ANNOTATIONS: [&str; 14] = [
    "official audio",
    "official music video",
    "official video",
    "official lyric video",
    "official visualizer",
    "lyric video",
    "music video",
    "audio",
    "lyrics",
    "visualizer",
    "hd",
    "hq",
    "4k",
    "full song",
];

fn normalized_words(raw: &str) -> String {
    tokens_in_order(raw).join(" ")
}

fn tokens_in_order(raw: &str) -> Vec<String> {
    raw.chars()
        .flat_map(char::to_lowercase)
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}

/// The title as compared with the requested one: explicit trailing presentation
/// annotations (`(Official Audio)`, `[Official Music Video]`, ` - Lyrics`) are dropped
/// unless the requested title itself contains that annotation. Everything that names a
/// version (live, remix, cover, remaster, a year) is kept.
fn comparable_title(candidate: &str, requested: &str) -> String {
    let wanted = normalized_words(requested);
    let own = |annotation: &str| format!(" {wanted} ").contains(&format!(" {annotation} "));
    let mut title = candidate.trim().to_string();
    loop {
        let trimmed = title.trim_end();
        let stripped = [('(', ')'), ('[', ']')].iter().find_map(|(open, close)| {
            let inner_end = trimmed.strip_suffix(*close)?;
            let start = inner_end.rfind(*open)?;
            let inner = normalized_words(&inner_end[start + 1..]);
            (ANNOTATIONS.contains(&inner.as_str()) && !own(&inner))
                .then(|| trimmed[..start].to_string())
        });
        let stripped = stripped.or_else(|| {
            let start = trimmed.rfind(" - ")?;
            let tail = normalized_words(&trimmed[start + 3..]);
            (ANNOTATIONS.contains(&tail.as_str()) && !own(&tail) && start > 0)
                .then(|| trimmed[..start].to_string())
        });
        match stripped {
            Some(next) if !next.trim().is_empty() => title = next,
            _ => return title.trim().to_string(),
        }
    }
}

/// The uploader as compared with the requested artists: a trailing ` - Topic` is
/// dropped, and a trailing `VEVO` only when what remains is a requested artist.
fn comparable_uploader(uploader: &str, artists: &[String]) -> String {
    let lower = uploader.trim().to_lowercase();
    let base = lower.strip_suffix(" - topic").unwrap_or(&lower).trim();
    if let Some(rest) = base.strip_suffix("vevo") {
        let rest = rest.trim();
        if !rest.is_empty() && artists.iter().any(|a| tokens(a) == tokens(rest)) {
            return rest.into();
        }
    }
    base.into()
}

fn rank(track: &SpotifyTrack, candidate: &mut YouTubeCandidate) {
    let requested_title = track.title.as_deref().unwrap_or_default();
    let title = tokens(&comparable_title(&candidate.title, requested_title));
    let mut weighted = 0.0;
    let mut weight = 0.0;
    if let Some(requested) = track.title.as_ref() {
        weighted += 0.55 * dice(&tokens(requested), &title);
        weight += 0.55;
    } else {
        candidate
            .warnings
            .push("Spotify title is unavailable".into());
    }
    if !track.artists.is_empty() {
        let uploader = tokens(&comparable_uploader(
            candidate.uploader.as_deref().unwrap_or_default(),
            &track.artists,
        ));
        let overlaps: Vec<f64> = track
            .artists
            .iter()
            .map(|a| {
                let a = tokens(a);
                dice(&a, &title).max(dice(&a, &uploader))
            })
            .collect();
        let best = overlaps.iter().cloned().fold(0.0, f64::max);
        let mean = overlaps.iter().sum::<f64>() / overlaps.len() as f64;
        weighted += 0.30 * (0.70 * best + 0.30 * mean);
        weight += 0.30;
        if best == 0.0 {
            candidate.warnings.push("Artist/channel differs".into());
        }
    } else {
        candidate
            .warnings
            .push("Spotify artist is unavailable".into());
    }
    if let Some((target, actual)) = track.duration.zip(candidate.duration) {
        let difference = (actual - target).abs();
        let tolerance = (0.10 * target).max(10.0);
        weighted += 0.15 * (1.0 - difference / tolerance).max(0.0);
        weight += 0.15;
        if difference > tolerance {
            candidate
                .warnings
                .push(format!("Duration differs by {:.0} s", difference));
        }
    } else {
        candidate
            .warnings
            .push("Duration comparison is unavailable".into());
    }
    let mut penalty: f64 = 0.0;
    for category in [
        "live",
        "cover",
        "remix",
        "karaoke",
        "instrumental",
        "sped up",
        "slowed",
    ] {
        if has_category(&candidate.title, category)
            && !has_category(track.title.as_deref().unwrap_or_default(), category)
        {
            penalty += 0.25;
            candidate
                .warnings
                .push(format!("May be a {category} recording"));
        }
    }
    candidate.score =
        (if weight > 0.0 { weighted / weight } else { 0.0 } - penalty.min(0.75)).clamp(0.0, 1.0);
}

fn parse_candidates(track: &SpotifyTrack, value: &Value) -> Vec<YouTubeCandidate> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for entry in value["entries"].as_array().into_iter().flatten() {
        if entry["_type"].as_str() == Some("playlist")
            || entry["ie_key"].as_str().is_some_and(|key| key != "Youtube")
            || matches!(
                entry["title"].as_str(),
                Some("[Private video]" | "[Deleted video]" | "[Unavailable video]")
            )
            || entry["is_live"].as_bool() == Some(true)
            || matches!(
                entry["live_status"].as_str(),
                Some("is_live" | "is_upcoming" | "post_live")
            )
            || entry["availability"]
                .as_str()
                .is_some_and(|s| !matches!(s, "public" | "unlisted"))
        {
            continue;
        }
        let mut explicit_id = None;
        let mut invalid_url = false;
        for key in ["webpage_url", "url"] {
            if let Some(raw) = entry[key].as_str() {
                match youtube_id(raw) {
                    Some(id) if explicit_id.as_ref().is_none_or(|prior| prior == &id) => {
                        explicit_id = Some(id)
                    }
                    _ => {
                        invalid_url = true;
                        break;
                    }
                }
            }
        }
        if invalid_url {
            continue;
        }
        let id = entry["id"]
            .as_str()
            .filter(|s| valid_video_id(s))
            .map(str::to_string)
            .or_else(|| explicit_id.clone());
        let Some(id) = id else { continue };
        if explicit_id.as_ref().is_some_and(|other| other != &id) {
            continue;
        }
        let Some(title) = entry["title"]
            .as_str()
            .map(str::trim)
            .filter(|s| bounded_text(s, 512))
        else {
            continue;
        };
        if !seen.insert(id.clone()) {
            continue;
        }
        let display = |key: &str| {
            entry[key]
                .as_str()
                .map(str::trim)
                .filter(|s| bounded_text(s, 512))
                .map(str::to_string)
        };
        let mut candidate = YouTubeCandidate {
            url: format!("https://www.youtube.com/watch?v={id}"),
            video_id: id,
            title: title.into(),
            uploader: display("uploader").or_else(|| display("channel")),
            duration: entry["duration"]
                .as_f64()
                .filter(|d| d.is_finite() && *d > 0.0 && *d <= 86400.0),
            thumbnail: entry["thumbnail"]
                .as_str()
                .filter(|s| s.len() <= 2048)
                .and_then(|s| url::Url::parse(s).ok())
                .filter(|u| {
                    u.scheme() == "https"
                        && u.username().is_empty()
                        && u.password().is_none()
                        && u.port().is_none()
                        && matches!(u.host_str(), Some("i.ytimg.com" | "img.youtube.com"))
                })
                .map(|u| u.to_string()),
            score: 0.0,
            warnings: Vec::new(),
        };
        rank(track, &mut candidate);
        out.push(candidate);
        if out.len() == 5 {
            break;
        }
    }
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    out
}

pub async fn search(
    app: &crate::AppHandle,
    track: SpotifyTrack,
    query: &str,
) -> Result<Vec<YouTubeCandidate>> {
    use tauri::Manager;
    validate_track(&track)?;
    let search = search_args(query.into())?;
    let state = app.state::<crate::state::AppState>();
    let settings = state.settings.lock().unwrap().clone();
    let _engine = state.engine.read().await;
    let ytdlp = crate::tools::find_ytdlp(app)
        .ok_or_else(|| AppError::msg("yt-dlp is not installed yet."))?;
    let mut args = crate::tools::common_args(app, &settings);
    args.extend(search);
    let stdout = crate::probe::run_lookup(app, &ytdlp, &args).await?;
    let value = serde_json::from_slice(&stdout)
        .map_err(|_| AppError::msg("YouTube search returned something that could not be read."))?;
    Ok(parse_candidates(&track, &value))
}
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct YouTubeCandidate {
    pub url: String,
    pub video_id: String,
    pub title: String,
    pub uploader: Option<String>,
    pub duration: Option<f64>,
    pub thumbnail: Option<String>,
    pub score: f64,
    pub warnings: Vec<String>,
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::spotify::SpotifyTrack;
    use serde_json::json;

    #[test]
    #[ignore = "live read-only YouTube search; set CLAWS_SPOTIFY_LIVE_YTDLP to an installed executable"]
    fn live_search_metadata_only() {
        use tauri::Manager;
        let executable = std::env::var_os("CLAWS_SPOTIFY_LIVE_YTDLP")
            .expect("provide an installed yt-dlp executable");
        let dir = crate::download_tests::temp_dir("spotify-live-search");
        let app =
            crate::lifecycle_tests::TestApp::with_data(dir.clone(), dir.join("out"), Vec::new());
        let results = tauri::async_runtime::block_on(async {
            let state = app.handle().state::<crate::state::AppState>();
            let settings = state.settings.lock().unwrap().clone();
            let _engine = state.engine.read().await;
            let mut args = crate::tools::common_args(app.handle(), &settings);
            args.extend(search_args("Cher Believe audio".into()).unwrap());
            let bytes =
                crate::probe::run_lookup(app.handle(), std::path::Path::new(&executable), &args)
                    .await
                    .unwrap();
            parse_candidates(&track(), &serde_json::from_slice(&bytes).unwrap())
        });
        assert!(!results.is_empty());
        for result in results {
            eprintln!(
                "live candidate {} {:?} {:?}",
                result.video_id, result.title, result.duration
            );
        }
        assert!(
            !app.out.exists(),
            "search must not create a media output directory"
        );
    }

    fn track() -> SpotifyTrack {
        SpotifyTrack {
            url: "https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT".into(),
            id: "4cOdK2wGLETKBW3PvgPWqT".into(),
            title: Some("Believe".into()),
            artists: vec!["Cher".into()],
            duration: Some(200.0),
            artwork: None,
        }
    }

    #[test]
    fn literal_query_is_one_bounded_operand() {
        assert_eq!(
            search_args("--exec ytsearch99:evil https://evil.test".into()).unwrap(),
            [
                "--dump-single-json",
                "--flat-playlist",
                "--skip-download",
                "--no-warnings",
                "--socket-timeout",
                "20",
                "--",
                "ytsearch5:--exec ytsearch99:evil https://evil.test"
            ]
        );
        assert!(search_args(" ".into()).is_err());
        assert!(search_args("x".repeat(301)).is_err());
    }

    #[test]
    fn ranking_arithmetic_and_whole_token_versions() {
        let value = serde_json::from_str(include_str!(
            "../tests/fixtures/spotify/youtube-search.json"
        ))
        .unwrap();
        let results = parse_candidates(&track(), &value);
        assert_eq!(results[0].score, 1.0);
        assert!(results[0].warnings.iter().all(|w| !w.contains("live")));
        assert_eq!(results[1].video_id, "cdefghijklm");
        assert!(results[1].warnings.iter().any(|w| w.contains("45 s")));
        assert!(results[2].score < results[1].score);
        let mut partial = track();
        partial.artists.clear();
        partial.duration = None;
        assert_eq!(parse_candidates(&partial, &value)[0].score, 1.0);
        partial.title = Some("夜に駆ける".into());
        assert_eq!(
            parse_candidates(
                &partial,
                &json!({"entries":[{"id":"abcdefghijk","title":"夜に駆ける"}]})
            )[0]
            .score,
            1.0
        );
    }

    fn scored(track: &SpotifyTrack, entries: serde_json::Value) -> Vec<YouTubeCandidate> {
        parse_candidates(track, &json!({ "entries": entries }))
    }
    fn entry(id: &str, title: &str, uploader: &str, duration: f64) -> serde_json::Value {
        json!({"id": id, "title": title, "uploader": uploader, "duration": duration})
    }
    fn by_id<'a>(found: &'a [YouTubeCandidate], id: &str) -> &'a YouTubeCandidate {
        found.iter().find(|c| c.video_id == id).unwrap()
    }
    fn collab() -> SpotifyTrack {
        SpotifyTrack {
            artists: vec!["Cher".into(), "Björk".into()],
            ..track()
        }
    }

    #[test]
    fn collaboration_credit_does_not_lift_covers_or_wrong_versions() {
        let found = scored(
            &collab(),
            json!([
                entry("aaaaaaaaaaa", "Believe", "Cher", 200.0),
                entry("bbbbbbbbbbb", "Cher & Björk - Believe", "Some Label", 200.0),
                entry("ccccccccccc", "Believe (Cher cover)", "Random Guy", 200.0),
                entry("ddddddddddd", "Believe (Live)", "Cher", 200.0),
                entry("eeeeeeeeeee", "Believe", "Unrelated Channel", 200.0),
            ]),
        );
        let clean = by_id(&found, "aaaaaaaaaaa").score;
        let named = by_id(&found, "bbbbbbbbbbb").score;
        let cover = by_id(&found, "ccccccccccc");
        let live = by_id(&found, "ddddddddddd");
        let stranger = by_id(&found, "eeeeeeeeeee");
        assert!(clean > cover.score && clean > live.score && clean > stranger.score);
        assert!(named > cover.score && named > live.score);
        assert!(cover.warnings.iter().any(|w| w.contains("cover")));
        assert!(live.warnings.iter().any(|w| w.contains("live")));
        assert!(stranger.warnings.iter().any(|w| w == "Artist/channel differs"));
        assert!(!by_id(&found, "aaaaaaaaaaa").warnings.iter().any(|w| w.contains("differs")));
        // One participating artist alone earns most of the artist credit (a plain mean of
        // the two artists would cap this exact recording at 0.85).
        assert!(clean > 0.9, "{clean}");
        assert!(clean > stranger.score);
    }

    #[test]
    fn topic_and_vevo_channels_count_only_for_a_requested_artist() {
        let found = scored(
            &track(),
            json!([
                entry("aaaaaaaaaaa", "Believe", "Cher - Topic", 200.0),
                entry("bbbbbbbbbbb", "Believe", "CherVEVO", 200.0),
                entry("ccccccccccc", "Believe", "RandomVEVO", 200.0),
                entry("ddddddddddd", "Believe", "Cher VEVO", 200.0),
            ]),
        );
        assert_eq!(by_id(&found, "aaaaaaaaaaa").score, 1.0);
        assert_eq!(by_id(&found, "bbbbbbbbbbb").score, 1.0);
        assert_eq!(by_id(&found, "ddddddddddd").score, 1.0);
        assert!(by_id(&found, "ccccccccccc").score < 1.0);
        assert!(by_id(&found, "ccccccccccc").warnings.iter().any(|w| w.contains("differs")));
        assert_eq!(by_id(&found, "bbbbbbbbbbb").uploader.as_deref(), Some("CherVEVO"));
    }

    #[test]
    fn presentation_annotations_are_ignored_but_identity_words_are_not() {
        let found = scored(
            &track(),
            json!([
                entry("aaaaaaaaaaa", "Believe (Official Audio)", "Cher", 200.0),
                entry("bbbbbbbbbbb", "Believe [Official Music Video]", "Cher", 200.0),
                entry("ccccccccccc", "Believe - Lyrics", "Cher", 200.0),
                entry("ddddddddddd", "Believe (2020 Remaster)", "Cher", 200.0),
                entry("eeeeeeeeeee", "Believe (Official Audio) (HD)", "Cher", 200.0),
            ]),
        );
        for id in ["aaaaaaaaaaa", "bbbbbbbbbbb", "ccccccccccc", "eeeeeeeeeee"] {
            assert_eq!(by_id(&found, id).score, 1.0, "{id}");
        }
        assert!(by_id(&found, "ddddddddddd").score < 1.0);
        // A requested title that is itself an annotation word is never stripped.
        let lyrics = SpotifyTrack { title: Some("Lyrics".into()), ..track() };
        assert_eq!(scored(&lyrics, json!([entry("aaaaaaaaaaa", "Lyrics", "Cher", 200.0)]))[0].score, 1.0);
        let audio = SpotifyTrack { title: Some("Believe (Audio)".into()), ..track() };
        assert_eq!(scored(&audio, json!([entry("aaaaaaaaaaa", "Believe (Audio)", "Cher", 200.0)]))[0].score, 1.0);
        let wrong = scored(&lyrics, json!([entry("aaaaaaaaaaa", "Other Song (Lyrics)", "Cher", 200.0)]));
        assert!(wrong[0].score < 1.0);
    }

    #[test]
    fn missing_metadata_and_non_latin_names_keep_separate_warnings() {
        let none = SpotifyTrack { artists: vec![], duration: None, ..track() };
        let found = scored(&none, json!([{"id": "aaaaaaaaaaa", "title": "Believe"}]));
        assert!(found[0].warnings.iter().any(|w| w.contains("artist is unavailable")));
        assert!(!found[0].warnings.iter().any(|w| w.contains("differs")));
        let jp = SpotifyTrack {
            title: Some("夜に駆ける".into()),
            artists: vec!["YOASOBI".into()],
            duration: None,
            ..track()
        };
        let found = scored(
            &jp,
            json!([
                entry("aaaaaaaaaaa", "夜に駆ける (Official Audio)", "YOASOBI", 261.0),
                entry("bbbbbbbbbbb", "夜に駆ける", "別のチャンネル", 261.0),
            ]),
        );
        assert_eq!(found[0].video_id, "aaaaaaaaaaa");
        assert_eq!(found[0].score, 1.0);
        assert!(found[1].warnings.iter().any(|w| w.contains("differs")));
        // Equal scores keep the search order.
        let ties = scored(&track(), json!([entry("aaaaaaaaaaa", "Believe", "Cher", 200.0), entry("bbbbbbbbbbb", "Believe", "Cher", 200.0)]));
        assert_eq!(ties[0].video_id, "aaaaaaaaaaa");
    }

    #[test]
    fn title_match_no_longer_adds_a_positive_warning() {
        let found = scored(&track(), json!([entry("aaaaaaaaaaa", "Believe", "Cher", 200.0)]));
        assert!(found[0].warnings.is_empty(), "{:?}", found[0].warnings);
    }

    #[test]
    fn candidate_filtering_cap_and_stable_ties() {
        let mut entries = vec![
            json!({"id":"abcdefghijk","title":"Believe"}),
            json!({"id":"abcdefghijk","title":"duplicate"}),
            json!({"id":"bcdefghijkl","title":"external","url":"https://evil.test/watch?v=bcdefghijkl"}),
            json!({"id":"cdefghijklm","title":"live","is_live":true}),
            json!({"id":"defghijklmn","title":"upcoming","live_status":"is_upcoming"}),
            json!({"id":"efghijklmno","title":"unavailable","availability":"private"}),
            json!({"id":"short","title":"bad"}),
        ];
        for id in [
            "fghijklmnop",
            "ghijklmnopq",
            "hijklmnopqr",
            "ijklmnopqrs",
            "jklmnopqrst",
            "klmnopqrstu",
        ] {
            entries.push(json!({"url":format!("https://youtu.be/{id}"),"title":"Believe"}));
        }
        let found = parse_candidates(&track(), &json!({"entries":entries}));
        assert_eq!(found.len(), 5);
        assert_eq!(found[0].url, "https://www.youtube.com/watch?v=abcdefghijk");
        assert_eq!(found[1].video_id, "fghijklmnop");
    }

    #[test]
    fn candidates_reject_unavailable_and_foreign_extractor_entries() {
        let entries = json!({"entries":[
            {"id":"abcdefghijk","title":"[Private video]"},
            {"id":"bcdefghijkl","title":"Believe","ie_key":"Vimeo"},
            {"id":"cdefghijklm","title":"Believe","_type":"playlist"},
            {"id":"defghijklmn","title":"Believe","url":"https://www.youtube.com/watch?v=efghijklmno"},
            {"id":"efghijklmno","title":"Believe","thumbnail":"https://evil.test/track"}
        ]});
        let result = parse_candidates(&track(), &entries);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].video_id, "efghijklmno");
        assert!(result[0].thumbnail.is_none());
    }

    #[test]
    fn ipc_metadata_is_validated_independently() {
        assert!(validate_track(&track()).is_ok());
        let mut bad = track();
        bad.id = "abcdefghijklmnopqrstuv".into();
        assert!(validate_track(&bad).is_err());
        let mut bad = track();
        bad.title = Some("x".repeat(513));
        assert!(validate_track(&bad).is_err());
        let mut bad = track();
        bad.artists = vec!["x".repeat(257)];
        assert!(validate_track(&bad).is_err());
        let mut bad = track();
        bad.duration = Some(f64::NAN);
        assert!(validate_track(&bad).is_err());
        let mut bad = track();
        bad.artwork = Some("https://evil.test/track".into());
        assert!(validate_track(&bad).is_err());
    }

    fn origin() -> MatchOrigin {
        MatchOrigin {
            kind: "spotifyYouTube".into(),
            spotify_url: track().url,
            spotify_title: Some("Believe".into()),
            spotify_artists: vec!["Cher".into()],
        }
    }

    #[test]
    fn origin_validates_identity_and_audio_constraints() {
        let audio = crate::downloader::DownloadOptions {
            mode: "audio".into(),
            ..Default::default()
        };
        let yt = "https://www.youtube.com/watch?v=abcdefghijk";
        assert!(validate_match_origin(&origin(), yt, None, &audio).is_ok());
        assert!(validate_match_origin(&origin(), yt, Some(0), &audio).is_err());
        assert!(validate_match_origin(&origin(), yt, None, &Default::default()).is_err());
        for url in [
            "https://youtu.be/abcdefghijk",
            "https://www.youtube.com/watch?v=abcdefghijk&list=123",
            "https://evil.test/watch?v=abcdefghijk",
        ] {
            assert!(validate_match_origin(&origin(), url, None, &audio).is_err());
        }
        let mut bad = origin();
        bad.spotify_url.push_str("?tracking=1");
        assert!(validate_match_origin(&bad, yt, None, &audio).is_err());
        let mut bad = origin();
        bad.kind = "other".into();
        assert!(validate_match_origin(&bad, yt, None, &audio).is_err());
        let mut bad = origin();
        bad.spotify_title = Some("x".repeat(513));
        assert!(validate_match_origin(&bad, yt, None, &audio).is_err());
        let mut bad = origin();
        bad.spotify_artists = vec!["x".repeat(257)];
        assert!(validate_match_origin(&bad, yt, None, &audio).is_err());
        let mut bad = origin();
        bad.spotify_artists = vec!["artist".into(); 21];
        assert!(validate_match_origin(&bad, yt, None, &audio).is_err());
    }
}
