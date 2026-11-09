use crate::error::{AppError, Result};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpotifyTrack {
    pub url: String,
    pub id: String,
    pub title: Option<String>,
    pub artists: Vec<String>,
    pub duration: Option<f64>,
    pub artwork: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpotifyResolution {
    pub track: SpotifyTrack,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum SpotifyInput {
    Track { id: String },
    Short { url: String },
    Unsupported,
}
use url::Url;
const BODY_LIMIT: usize = 2 * 1024 * 1024;
const FALLBACK: &str =
    "Couldn't read this Spotify track. Enter the artist and song title to search YouTube.";
fn valid_id(id: &str) -> bool {
    id.len() == 22 && id.bytes().all(|b| b.is_ascii_alphanumeric())
}
fn secure_url(url: &Url, hosts: &[&str]) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.host_str().is_some_and(|host| hosts.contains(&host))
}
pub fn classify_spotify_url(raw: &str) -> Option<SpotifyInput> {
    let raw = raw.trim();
    if raw.to_ascii_lowercase().starts_with("spotify:") {
        return Some(SpotifyInput::Unsupported);
    }
    let url = Url::parse(raw).ok()?;
    let host = url.host_str()?;
    if !["open.spotify.com", "spotify.link"].contains(&host) {
        return None;
    }
    if !secure_url(&url, &["open.spotify.com", "spotify.link"]) {
        return Some(SpotifyInput::Unsupported);
    }
    if host == "spotify.link" {
        let token = url.path().trim_start_matches('/');
        return Some(
            if !token.is_empty()
                && token.len() <= 256
                && token
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            {
                SpotifyInput::Short {
                    url: url.to_string(),
                }
            } else {
                SpotifyInput::Unsupported
            },
        );
    }
    let parts: Vec<_> = url.path().trim_matches('/').split('/').collect();
    let parts = if parts.first().is_some_and(|part| {
        part.starts_with("intl-")
            && part.len() > 5
            && part[5..]
                .bytes()
                .all(|b| b.is_ascii_alphabetic() || b == b'-')
    }) {
        &parts[1..]
    } else {
        &parts[..]
    };
    Some(
        if parts.len() == 2 && parts[0] == "track" && valid_id(parts[1]) {
            SpotifyInput::Track {
                id: parts[1].into(),
            }
        } else {
            SpotifyInput::Unsupported
        },
    )
}
fn stub(id: &str) -> Result<SpotifyTrack> {
    if !valid_id(id) {
        return Err(AppError::msg("Invalid Spotify track ID."));
    }
    Ok(SpotifyTrack {
        url: format!("https://open.spotify.com/track/{id}"),
        id: id.into(),
        title: None,
        artists: vec![],
        duration: None,
        artwork: None,
    })
}
fn bounded_text(value: &serde_json::Value, max: usize) -> Option<String> {
    let text = value.as_str()?.trim();
    (!text.is_empty() && text.chars().count() <= max).then(|| text.to_string())
}
fn artwork(value: &serde_json::Value) -> Option<String> {
    let raw = value.as_str()?;
    if raw.len() > 4096 {
        return None;
    }
    let url = Url::parse(raw).ok()?;
    secure_url(&url, &["i.scdn.co", "image-cdn-ak.spotifycdn.com"]).then(|| url.to_string())
}
// A small inert HTML tokenizer: quoted `>` cannot close a tag, comments and raw
// script content cannot introduce elements. We only read one specifically named
// JSON script (and oEmbed's identity-bearing iframe src), never execute HTML.
fn attribute<'a>(tag: &'a str, wanted: &str) -> Option<&'a str> {
    let bytes = tag.as_bytes();
    let mut i = 0;
    while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/') {
            i += 1;
        }
        let start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && !b"=>".contains(&bytes[i]) {
            i += 1;
        }
        if i == start {
            break;
        }
        let name = &tag[start..i];
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if bytes.get(i) != Some(&b'=') {
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let quote = bytes.get(i).copied().filter(|b| *b == b'\'' || *b == b'"');
        if quote.is_some() {
            i += 1;
        }
        let value_start = i;
        while i < bytes.len()
            && match quote {
                Some(q) => bytes[i] != q,
                None => !bytes[i].is_ascii_whitespace() && bytes[i] != b'>',
            }
        {
            i += 1;
        }
        let value = &tag[value_start..i];
        if name.eq_ignore_ascii_case(wanted) {
            return Some(value);
        }
        if quote.is_some() {
            i += 1;
        }
    }
    None
}
fn elements<'a>(html: &'a str, wanted: &str) -> Result<Vec<(&'a str, &'a str)>> {
    let lower = html.to_ascii_lowercase();
    let bytes = html.as_bytes();
    let mut offset = 0;
    let mut found = vec![];
    while let Some(next) = lower[offset..].find('<') {
        let start = offset + next + 1;
        if lower[start..].starts_with("!--") {
            let end = lower[start + 3..]
                .find("-->")
                .ok_or_else(|| AppError::msg("Malformed Spotify markup."))?;
            offset = start + 3 + end + 3;
            continue;
        }
        let mut end = start;
        let mut quote = None;
        while end < bytes.len() {
            match (quote, bytes[end]) {
                (Some(q), b) if q == b => quote = None,
                (None, b'\'' | b'"') => quote = Some(bytes[end]),
                (None, b'>') => break,
                _ => {}
            }
            end += 1;
        }
        if end == bytes.len() {
            return Err(AppError::msg("Malformed Spotify markup."));
        }
        let tag = &html[start..end];
        let name = tag
            .split_ascii_whitespace()
            .next()
            .unwrap_or("")
            .trim_end_matches('/');
        offset = end + 1;
        let mut content = "";
        if name.eq_ignore_ascii_case("script") || name.eq_ignore_ascii_case("style") {
            let closing = format!("</{}", name.to_ascii_lowercase());
            let content_end = lower[offset..]
                .find(&closing)
                .ok_or_else(|| AppError::msg("Missing Spotify script closing tag."))?
                + offset;
            content = &html[offset..content_end];
            offset = content_end;
        }
        if name.eq_ignore_ascii_case(wanted) {
            found.push((tag, content));
        }
    }
    Ok(found)
}
pub fn parse_embed(raw: &[u8], expected_id: &str) -> Result<SpotifyTrack> {
    let mut track = stub(expected_id)?;
    if raw.len() > BODY_LIMIT {
        return Err(AppError::msg("Spotify metadata is too large."));
    }
    let html =
        std::str::from_utf8(raw).map_err(|_| AppError::msg("Invalid Spotify markup encoding."))?;
    let scripts: Vec<_> = elements(html, "script")?
        .into_iter()
        .filter(|(tag, _)| attribute(tag, "id") == Some("__NEXT_DATA__"))
        .collect();
    if scripts.len() != 1 {
        return Err(AppError::msg(
            "Spotify metadata script is missing or ambiguous.",
        ));
    }
    let document: serde_json::Value = serde_json::from_str(scripts[0].1)?;
    let entity = &document["props"]["pageProps"]["state"]["data"]["entity"];
    let uri = format!("spotify:track:{expected_id}");
    if entity["type"] != "track"
        || (entity["uri"].as_str() != Some(&uri) && entity["id"].as_str() != Some(expected_id))
        || entity["uri"].as_str().is_some_and(|v| v != uri)
        || entity["id"].as_str().is_some_and(|v| v != expected_id)
    {
        return Err(AppError::msg(
            "Spotify returned a different track identity.",
        ));
    }
    track.title = Some(
        bounded_text(&entity["name"], 512)
            .ok_or_else(|| AppError::msg("Spotify track title is missing or too large."))?,
    );
    if let Some(artists) = entity["artists"].as_array().filter(|a| a.len() <= 20) {
        track.artists = artists
            .iter()
            .filter_map(|a| bounded_text(&a["name"], 256))
            .collect();
    }
    track.duration = entity["duration"]
        .as_f64()
        .map(|ms| ms / 1000.0)
        .filter(|s| s.is_finite() && *s > 0.0 && *s <= 86400.0);
    track.artwork = entity["visualIdentity"]["image"]
        .as_array()
        .and_then(|images| images.iter().find_map(|image| artwork(&image["url"])))
        .or_else(|| {
            entity["coverArt"]["sources"]
                .as_array()
                .and_then(|images| images.iter().find_map(|image| artwork(&image["url"])))
        });
    Ok(track)
}
pub fn parse_oembed(raw: &[u8], expected_id: &str) -> Result<SpotifyTrack> {
    let mut track = stub(expected_id)?;
    if raw.len() > BODY_LIMIT {
        return Err(AppError::msg("Spotify metadata is too large."));
    }
    let value: serde_json::Value = serde_json::from_slice(raw)?;
    let html = value["html"]
        .as_str()
        .ok_or_else(|| AppError::msg("Spotify oEmbed identity is missing."))?;
    let frames = elements(html, "iframe")?;
    if value["type"] != "rich" || value["provider_name"] != "Spotify" || frames.len() != 1 {
        return Err(AppError::msg("Invalid Spotify oEmbed identity."));
    }
    let src = attribute(frames[0].0, "src")
        .and_then(|raw| Url::parse(raw).ok())
        .ok_or_else(|| AppError::msg("Invalid Spotify oEmbed identity."))?;
    if !secure_url(&src, &["open.spotify.com"])
        || src.path() != format!("/embed/track/{expected_id}")
    {
        return Err(AppError::msg(
            "Spotify returned a different track identity.",
        ));
    }
    track.title = Some(
        bounded_text(&value["title"], 512)
            .ok_or_else(|| AppError::msg("Spotify track title is missing or too large."))?,
    );
    track.artwork = artwork(&value["thumbnail_url"]);
    Ok(track)
}
async fn fetch_bounded(
    client: &reqwest::Client,
    mut url: Url,
    deadline: tokio::time::Instant,
    redirects: &mut usize,
    metadata_path: Option<&str>,
    endpoint: &(dyn Fn(&Url) -> Url + Sync),
) -> Result<(Url, Vec<u8>)> {
    use futures_util::StreamExt;
    let fetch = async {
        let mut visited = std::collections::HashSet::new();
        loop {
            // Validate the logical destination before the injectable endpoint boundary.
            // Production uses the identity mapping; tests map only approved logical
            // destinations to a loopback server without weakening this policy.
            if !secure_url(&url, &["open.spotify.com", "spotify.link"]) {
                return Err(AppError::msg(
                    "Spotify redirect destination is not allowed.",
                ));
            }
            match metadata_path {
                Some(path) if url.host_str() != Some("open.spotify.com") || url.path() != path => {
                    return Err(AppError::msg("Spotify redirected to another entity."))
                }
                None => match classify_spotify_url(url.as_str()) {
                    Some(SpotifyInput::Track { .. }) => return Ok((url, vec![])),
                    Some(SpotifyInput::Short { .. }) => {}
                    _ => {
                        return Err(AppError::msg(
                            "Spotify short link did not resolve to an individual track.",
                        ))
                    }
                },
                _ => {}
            }
            if !visited.insert(url.to_string()) {
                return Err(AppError::msg("Spotify redirect loop."));
            }
            let response = client
                .get(endpoint(&url))
                .header(reqwest::header::ACCEPT_ENCODING, "identity")
                .send()
                .await?;
            let status = response.status();
            #[cfg(test)]
            eprintln!(
                "Spotify HTTP host={} status={}",
                url.host_str().unwrap_or_default(),
                status.as_u16()
            );
            if [301, 302, 303, 307, 308].contains(&status.as_u16()) {
                if *redirects >= 5 {
                    return Err(AppError::msg("Spotify exceeded five redirects."));
                }
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| AppError::msg("Spotify redirect has no valid destination."))?;
                let next = url
                    .join(location)
                    .map_err(|_| AppError::msg("Invalid Spotify redirect destination."))?;
                // oEmbed query defines the requested identity as well as its path.
                if metadata_path == Some("/oembed") && next.query() != url.query() {
                    return Err(AppError::msg("Spotify redirected to another entity."));
                }
                *redirects += 1;
                url = next;
                continue;
            }
            if !status.is_success() {
                return Err(AppError::msg(format!(
                    "Spotify metadata request returned HTTP {}.",
                    status.as_u16()
                )));
            }
            if response
                .content_length()
                .is_some_and(|len| len > BODY_LIMIT as u64)
            {
                return Err(AppError::msg("Spotify metadata exceeds 2 MiB."));
            }
            // Compression is deliberately not enabled on the dedicated client. An
            // unsolicited encoded response is rejected, rather than decompressed
            // outside the streaming limit.
            if response
                .headers()
                .get(reqwest::header::CONTENT_ENCODING)
                .is_some_and(|v| v != "identity")
            {
                return Err(AppError::msg("Unsupported Spotify metadata encoding."));
            }
            let mut stream = response.bytes_stream();
            let mut body = Vec::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk?;
                if chunk.len() > BODY_LIMIT.saturating_sub(body.len()) {
                    return Err(AppError::msg("Spotify metadata exceeds 2 MiB."));
                }
                body.extend_from_slice(&chunk);
            }
            return Ok((url, body));
        }
    };
    tokio::time::timeout_at(deadline, fetch)
        .await
        .map_err(|_| AppError::msg("Spotify resolution timed out."))?
}
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("Claws/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("Spotify HTTP client configuration is valid")
}
pub async fn resolve(app: &crate::AppHandle, raw: &str) -> Result<SpotifyResolution> {
    use tauri::Manager;
    let state = app.state::<crate::state::AppState>();
    resolve_with(
        &state.spotify_http,
        raw,
        tokio::time::Instant::now() + std::time::Duration::from_secs(20),
        &|url| url.clone(),
    )
    .await
}
async fn resolve_with(
    client: &reqwest::Client,
    raw: &str,
    deadline: tokio::time::Instant,
    endpoint: &(dyn Fn(&Url) -> Url + Sync),
) -> Result<SpotifyResolution> {
    let mut redirects = 0;
    let id = match classify_spotify_url(raw) {
        Some(SpotifyInput::Track { id }) => id,
        Some(SpotifyInput::Short { url }) => {
            let (final_url, _) = fetch_bounded(client, Url::parse(&url).map_err(|_| AppError::msg("Invalid Spotify short link."))?, deadline, &mut redirects, None, endpoint).await
                .map_err(|_| AppError::msg("Couldn't resolve this Spotify short link. Please paste the full Spotify track link."))?;
            match classify_spotify_url(final_url.as_str()) {
                Some(SpotifyInput::Track { id }) => id,
                _ => return Err(AppError::msg("Couldn't resolve this Spotify short link. Please paste the full Spotify track link.")),
            }
        }
        _ => {
            return Err(AppError::msg(
                "Spotify albums and playlists aren't supported yet. Paste a single track link.",
            ))
        }
    };
    let mut warnings = Vec::new();
    let embed_url = Url::parse(&format!("https://open.spotify.com/embed/track/{id}"))
        .map_err(|_| AppError::msg("Invalid Spotify track URL."))?;
    let embed_path = embed_url.path().to_string();
    let embed = fetch_bounded(
        client,
        embed_url,
        deadline,
        &mut redirects,
        Some(&embed_path),
        endpoint,
    )
    .await;
    if let Ok(track) = embed.and_then(|(_, body)| parse_embed(&body, &id)) {
        if track.artists.is_empty() {
            warnings.push("Spotify artist metadata is unavailable.".into());
        }
        if track.duration.is_none() {
            warnings.push("Spotify duration is unavailable.".into());
        }
        return Ok(SpotifyResolution { track, warnings });
    }
    warnings.push("Spotify embed metadata was unavailable; trying public oEmbed metadata.".into());
    if tokio::time::Instant::now() < deadline {
        let mut url = Url::parse("https://open.spotify.com/oembed")
            .map_err(|_| AppError::msg("Invalid Spotify metadata URL."))?;
        url.query_pairs_mut()
            .append_pair("url", &format!("https://open.spotify.com/track/{id}"));
        let response = fetch_bounded(
            client,
            url,
            deadline,
            &mut redirects,
            Some("/oembed"),
            endpoint,
        )
        .await;
        if let Ok(track) = response.and_then(|(_, body)| parse_oembed(&body, &id)) {
            warnings.push("Spotify artist and duration metadata are unavailable.".into());
            return Ok(SpotifyResolution { track, warnings });
        }
    }
    warnings.push(FALLBACK.into());
    Ok(SpotifyResolution {
        track: stub(&id)?,
        warnings,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    const ID: &str = "4cOdK2wGLETKBW3PvgPWqT";
    #[test]
    fn classifies_and_canonicalizes_tracks() {
        for suffix in [
            format!("track/{ID}?si=abc#foo"),
            format!("intl-pt/track/{ID}"),
        ] {
            assert_eq!(
                classify_spotify_url(&format!("https://open.spotify.com/{suffix}")),
                Some(SpotifyInput::Track { id: ID.into() })
            );
        }
        for raw in [
            "http://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT",
            "https://user@open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT",
            "https://open.spotify.com:444/track/4cOdK2wGLETKBW3PvgPWqT",
            "https://open.spotify.com.evil/track/4cOdK2wGLETKBW3PvgPWqT",
            "https://open.spotify.com/track/short",
            "spotify:track:4cOdK2wGLETKBW3PvgPWqT",
            "https://open.spotify.com/album/4cOdK2wGLETKBW3PvgPWqT",
        ] {
            assert!(
                !matches!(classify_spotify_url(raw), Some(SpotifyInput::Track { .. })),
                "{raw}"
            );
        }
    }
    #[test]
    fn embed_preserves_identity_unicode_artists_and_milliseconds() {
        let track =
            parse_embed(include_bytes!("../tests/fixtures/spotify/embed.html"), ID).unwrap();
        assert_eq!(track.title.as_deref(), Some("Ça va 世界"));
        assert_eq!(track.artists, ["Artist One", "Björk"]);
        assert_eq!(track.duration, Some(213.573));
        assert_eq!(track.url, format!("https://open.spotify.com/track/{ID}"));
    }
    #[test]
    fn live_visual_identity_artwork_and_trailing_slash_are_supported() {
        let track = parse_embed(
            include_bytes!("../tests/fixtures/spotify/embed-visual-identity.html"),
            ID,
        )
        .unwrap();
        assert_eq!(
            track.artwork.as_deref(),
            Some("https://image-cdn-ak.spotifycdn.com/image/public-fixture")
        );
        assert_eq!(
            classify_spotify_url(&format!("https://open.spotify.com/track/{ID}/?si=discard")),
            Some(SpotifyInput::Track { id: ID.into() })
        );
    }
    #[test]
    fn oembed_is_partial_and_validates_identity() {
        let raw = include_bytes!("../tests/fixtures/spotify/oembed.json");
        let track = parse_oembed(raw, ID).unwrap();
        assert!(track.duration.is_none() && track.artists.is_empty());
        assert!(parse_oembed(raw, "0000000000000000000000").is_err());
    }
    #[test]
    fn malformed_or_different_entity_never_yields_metadata() {
        let raw =
            std::str::from_utf8(include_bytes!("../tests/fixtures/spotify/embed.html")).unwrap();
        for broken in [
            raw.replace(ID, "0000000000000000000000"),
            raw.replace("\"track\"", "\"album\""),
            raw.replace("__NEXT_DATA__", "changed"),
            raw.replace("Ça va 世界", &"x".repeat(513)),
        ] {
            assert!(parse_embed(broken.as_bytes(), ID).is_err());
        }
        let no_duration = raw.replace("\"duration\":213573,", "");
        assert_eq!(
            parse_embed(no_duration.as_bytes(), ID).unwrap().duration,
            None
        );
    }
    #[test]
    fn artwork_is_limited_to_exact_public_spotify_image_hosts() {
        for host in ["i.scdn.co", "image-cdn-ak.spotifycdn.com"] {
            assert!(artwork(&serde_json::json!(format!("https://{host}/image/abc"))).is_some());
        }
        for raw in [
            "http://i.scdn.co/image/abc",
            "https://i.scdn.co.evil/image/abc",
            "https://user@i.scdn.co/image/abc",
            "https://i.scdn.co:444/image/abc",
            "https://127.0.0.1/image/abc",
            "data:image/png;base64,abc",
        ] {
            assert!(artwork(&serde_json::json!(raw)).is_none(), "{raw}");
        }
    }
    #[test]
    fn script_is_inert_unambiguous_and_attributes_allow_normal_markup_variants() {
        let raw =
            std::str::from_utf8(include_bytes!("../tests/fixtures/spotify/embed.html")).unwrap();
        let valid = raw.replace("id=\"__NEXT_DATA__\"", "data-other='>' ID='__NEXT_DATA__'");
        assert!(parse_embed(valid.as_bytes(), ID).is_ok());
        assert!(parse_embed(format!("<!--{raw}-->").as_bytes(), ID).is_err());
        assert!(parse_embed(format!("{raw}{raw}").as_bytes(), ID).is_err());
        assert!(parse_embed(
            b"<script id=__NEXT_DATA__>alert('no execution')</script>",
            ID
        )
        .is_err());
        for duration in ["-1", "0", "86400001", "null", "\"213573\""] {
            assert_eq!(
                parse_embed(raw.replace("213573", duration).as_bytes(), ID)
                    .unwrap()
                    .duration,
                None
            );
        }
    }
    type TestReply = (u16, Option<String>, Vec<u8>, u64, bool);
    fn server(replies: Vec<TestReply>) -> (Url, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let handle = std::thread::spawn(move || {
            for (status, location, body, delay, chunked) in replies {
                let wait_until = std::time::Instant::now() + std::time::Duration::from_secs(2);
                let (mut stream, _) = loop {
                    match listener.accept() {
                        Ok(stream) => break stream,
                        Err(error)
                            if error.kind() == std::io::ErrorKind::WouldBlock
                                && std::time::Instant::now() < wait_until =>
                        {
                            std::thread::sleep(std::time::Duration::from_millis(2))
                        }
                        _ => return,
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                    .unwrap();
                let mut request = [0; 8192];
                let _ = stream.read(&mut request);
                std::thread::sleep(std::time::Duration::from_millis(delay));
                let extra = location
                    .map(|loc| format!("Location: {loc}\r\n"))
                    .unwrap_or_default();
                let encoding = if chunked {
                    "Transfer-Encoding: chunked".into()
                } else {
                    format!("Content-Length: {}", body.len())
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} Test\r\n{extra}{encoding}\r\nConnection: close\r\n\r\n"
                );
                if chunked {
                    for part in body.chunks(32768) {
                        if write!(stream, "{:x}\r\n", part.len())
                            .and_then(|_| stream.write_all(part))
                            .and_then(|_| stream.write_all(b"\r\n"))
                            .is_err()
                        {
                            break;
                        }
                    }
                    let _ = stream.write_all(b"0\r\n\r\n");
                } else {
                    let _ = stream.write_all(&body);
                }
            }
        });
        (base, handle)
    }
    async fn fetch_test(
        replies: Vec<TestReply>,
        start: &str,
        budget_ms: u64,
    ) -> Result<(Url, Vec<u8>)> {
        let expected_requests = replies.len();
        let requests = std::sync::atomic::AtomicUsize::new(0);
        let (base, server) = server(replies);
        let endpoint = |logical: &Url| {
            requests.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mut url = base.clone();
            url.set_path(logical.path());
            url.set_query(logical.query());
            url
        };
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let result = fetch_bounded(
            &client,
            Url::parse(start).unwrap(),
            tokio::time::Instant::now() + std::time::Duration::from_millis(budget_ms),
            &mut 0,
            None,
            &endpoint,
        )
        .await;
        server.join().unwrap();
        assert_eq!(
            requests.load(std::sync::atomic::Ordering::SeqCst),
            expected_requests,
            "an unapproved destination must never reach the transport"
        );
        result
    }
    #[tokio::test]
    async fn redirect_limit_and_relative_location() {
        let redirects = |count: usize| {
            (0..count)
                .map(|i| {
                    (
                        302,
                        Some(if i + 1 == count {
                            format!("https://open.spotify.com/track/{ID}")
                        } else {
                            format!("/hop{i}")
                        }),
                        vec![],
                        0,
                        false,
                    )
                })
                .collect()
        };
        let five = fetch_test(redirects(5), "https://spotify.link/start", 5000).await;
        assert!(five.is_ok(), "{five:?}");
        assert!(fetch_test(redirects(6), "https://spotify.link/start", 2000)
            .await
            .is_err());
    }
    #[tokio::test]
    async fn unsafe_redirect_destinations_are_rejected_before_request() {
        for destination in [
            "https://example.com/track/x",
            "http://open.spotify.com/track/x",
            "https://127.0.0.1/track/x",
            "https://192.168.1.1/track/x",
            "https://user@open.spotify.com/track/x",
            "https://open.spotify.com:8443/track/x",
            "https://open.spotify.com/album/4cOdK2wGLETKBW3PvgPWqT",
            "https://spotify.link/start",
        ] {
            assert!(
                fetch_test(
                    vec![(302, Some(destination.into()), vec![], 0, false)],
                    "https://spotify.link/start",
                    2000
                )
                .await
                .is_err(),
                "{destination}"
            );
        }
    }
    #[tokio::test]
    async fn status_body_and_whole_operation_budget_are_bounded() {
        for status in [404, 429, 500] {
            assert!(fetch_test(
                vec![(status, None, vec![], 0, false)],
                "https://spotify.link/start",
                2000
            )
            .await
            .is_err());
        }
        assert!(fetch_test(
            vec![(200, None, vec![b'x'; BODY_LIMIT + 1], 0, true)],
            "https://spotify.link/start",
            2000
        )
        .await
        .is_err());
        assert!(fetch_test(
            vec![(200, None, vec![], 100, false)],
            "https://spotify.link/start",
            30
        )
        .await
        .is_err());
        assert!(fetch_test(
            vec![
                (302, Some("/next".into()), vec![], 40, false),
                (
                    302,
                    Some(format!("https://open.spotify.com/track/{ID}")),
                    vec![],
                    40,
                    false
                )
            ],
            "https://spotify.link/start",
            65
        )
        .await
        .is_err());
    }
    #[tokio::test]
    async fn recoverable_metadata_failure_and_partial_oembed_never_change_identity() {
        for oembed in [
            include_bytes!("../tests/fixtures/spotify/oembed.json").to_vec(),
            b"changed schema".to_vec(),
        ] {
            let (base, server) = server(vec![
                (200, None, b"<html>changed</html>".to_vec(), 0, false),
                (200, None, oembed, 0, false),
            ]);
            let endpoint = |logical: &Url| {
                let mut url = base.clone();
                url.set_path(logical.path());
                url.set_query(logical.query());
                url
            };
            let client = reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap();
            let resolution = resolve_with(
                &client,
                &format!("https://open.spotify.com/track/{ID}"),
                tokio::time::Instant::now() + std::time::Duration::from_secs(2),
                &endpoint,
            )
            .await
            .unwrap();
            server.join().unwrap();
            assert_eq!(resolution.track.id, ID);
            assert!(resolution.track.duration.is_none() && resolution.track.artists.is_empty());
            assert!(!resolution.warnings.is_empty());
        }
    }
    #[tokio::test]
    async fn metadata_redirect_to_a_different_track_is_rejected_before_request() {
        let (base, server) = server(vec![(
            302,
            Some("https://open.spotify.com/embed/track/0000000000000000000000".into()),
            vec![],
            0,
            false,
        )]);
        let path = format!("/embed/track/{ID}");
        let result = fetch_bounded(
            &http_client(),
            Url::parse(&format!("https://open.spotify.com{path}")).unwrap(),
            tokio::time::Instant::now() + std::time::Duration::from_secs(2),
            &mut 0,
            Some(&path),
            &|_| base.clone(),
        )
        .await;
        server.join().unwrap();
        assert!(result.is_err());
    }
    #[tokio::test]
    async fn response_body_stall_is_inside_the_same_deadline() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = stream.read(&mut [0; 8192]);
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\na");
            let _ = stream.flush();
            std::thread::sleep(std::time::Duration::from_millis(150));
            let _ = stream.write_all(b"bcdefghi");
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let result = fetch_bounded(
            &client,
            Url::parse("https://spotify.link/start").unwrap(),
            tokio::time::Instant::now() + std::time::Duration::from_millis(40),
            &mut 0,
            None,
            &|_| base.clone(),
        )
        .await;
        server.join().unwrap();
        assert!(result.unwrap_err().to_string().contains("timed out"));
    }
    #[test]
    #[ignore = "Opt-in read-only Spotify requests; CLAWS_SPOTIFY_SMOKE_URLS overrides the public sample URLs"]
    fn live_metadata_smoke() {
        let defaults = format!("https://open.spotify.com/track/{ID}\nhttps://open.spotify.com/track/2VxeLyX666F8uXCJ0dZF8B\nhttps://open.spotify.com/track/7ovUcF5uHTBRzUpB6ZOmvt\nhttps://open.spotify.com/intl-pt/track/{ID}\nhttps://open.spotify.com/track/0000000000000000000000\nhttps://open.spotify.com/album/{ID}\nhttps://open.spotify.com/playlist/{ID}");
        let urls = std::env::var("CLAWS_SPOTIFY_SMOKE_URLS").unwrap_or(defaults);
        let dir = crate::download_tests::temp_dir("spotify-live-metadata");
        let app = crate::lifecycle_tests::TestApp::with_data(dir.clone(), dir.join("out"), vec![]);
        for raw in urls.split_whitespace().take(12) {
            let start = std::time::Instant::now();
            let result = tauri::async_runtime::block_on(resolve(app.handle(), raw));
            match result {
                Ok(resolution) => eprintln!(
                    "Spotify live {} elapsed={:?}: {}",
                    resolution.track.url,
                    start.elapsed(),
                    serde_json::to_string(&resolution).unwrap()
                ),
                Err(error) => eprintln!(
                    "Spotify live input={:?} elapsed={:?}: error={error}",
                    classify_spotify_url(raw),
                    start.elapsed()
                ),
            }
        }
    }
}
