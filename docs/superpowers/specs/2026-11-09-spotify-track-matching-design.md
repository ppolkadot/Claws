# Spotify track matching design

Status: implemented in source following the user's execution request. See `docs/spotify-track-matching-verification.md` for evidence and remaining acceptance limits.

## Intent and scope

Let a person paste one Spotify song link, review recordings found on YouTube, choose one explicitly, and download it through Claws' existing audio pipeline. The saved file comes from YouTube. Spotify supplies identification metadata only.

Initial scope is individual `open.spotify.com/track/<id>` links and `spotify.link/<token>` links that resolve to an individual track. Support an optional `/intl-<language>/` path prefix. Album, playlist, artist, episode, show, embed URLs and `spotify:` URIs get a specific unsupported-input explanation. Spotify links in multiline batches get a per-row instruction to open the track individually; other rows continue normally. No automatic batch matching.

No Spotify authentication, developer credentials, audio-preview download, DRM handling, automatic first-result selection, external matching service, Python runtime, or spotDL executable dependency. No release, installation, version bump or publication in this work. Existing YouTube video/audio/transcript workflows stay available.

## Evidence and current seams

Repository inspected on 2026-11-09:

- `src/views/download.ts`: single-link lookup, cached previews, ticket-based stale-response suppression, options, estimates and enqueue; currently has only idle/loading/result/error/batch phases.
- `src/store.ts`: `PreviewMemory` persists ordinary previews across navigation; `LinkBatch<ProbeResult>` invokes `api.probe` for each batch row.
- `src/api.ts`, `src-tauri/src/commands.rs`, `src-tauri/src/lib.rs`: typed IPC and command registration.
- `src-tauri/src/probe.rs`: URL validation, normal media probing, full-info caching and `run_lookup`; callers hold the engine read lock. Processes have bounded output, a 90-second timeout and shutdown registration.
- `src-tauri/src/downloader.rs`: `EnqueueItem`, `Job`, enqueue validation and deduplication by resolved URL, playlist index and options.
- `src-tauri/src/history.rs`: copies job fields into persistent Library records. `enqueue` command's existing `source` argument only updates the link cache; it is not job provenance.
- `src-tauri/src/state.rs`: shared reqwest client has a connection timeout and default redirect handling. Spotify needs a separately configured client; changing the shared client could affect unrelated downloads.

Public read-only check on 2026-11-09 for `4cOdK2wGLETKBW3PvgPWqT`: oEmbed and embed returned HTTP 200. oEmbed exposed title and thumbnail but no separate artist/duration. Embed `__NEXT_DATA__.props.pageProps.state.data.entity` exposed the track identity, artists and duration `213573` milliseconds. This proves one working example, not broad compatibility or a stable schema. No YouTube matching or audio download was tested during planning.

Sources: [Spotify's documented oEmbed API](https://developer.spotify.com/documentation/embeds/reference/oembed), [Spotify embed tutorial](https://developer.spotify.com/documentation/embeds/tutorials/creating-an-embed), [spotDL's documented matching approach](https://github.com/spotDL/spotify-downloader), [yt-dlp source and usage](https://github.com/yt-dlp/yt-dlp). oEmbed is documented; the embed page's internal JSON layout is not a guaranteed metadata contract.

## Approach and decisions

Recommended approach: implement a small native Rust metadata resolver and YouTube candidate search, with a separate frontend confirmation state. This follows Claws' current Rust/TypeScript stack and reuses its managed yt-dlp installation.

Alternatives considered: bundling spotDL adds Python/tool installation and configuration obligations; manual search only is simpler but does not deliver the Spotify-link workflow. Keep editable manual search as the fallback within the native approach. These are planning recommendations, not decisions previously approved by the user.

## Metadata and request contract

- Canonical track IDs: exactly 22 ASCII alphanumeric characters. Canonical URL: `https://open.spotify.com/track/<id>`; discard tracking query and fragments. Reject credentials, non-HTTPS schemes, explicit nondefault ports and lookalike hosts.
- Resolve short links with automatic redirects disabled. Permit only exact `spotify.link` and `open.spotify.com` hosts, HTTPS and default port at each hop; maximum 5 redirects. Reject all other destinations and provide a full-link fallback. Do not silently widen the allowlist if a real short link uses another host.
- Dedicated reusable `spotify_http` client: 5-second connection timeout; one 20-second budget for the whole resolve operation including short-link hops and metadata requests; 2 MiB maximum decoded bytes per body, enforced while streaming. No cookie store, browser cookies or authorization headers. A failed metadata source may fall back within the remaining budget; no automatic retries on rate limiting.
- Fetch the canonical embed page for structured identity, title, artists, duration and optional artwork. Extract a specifically identified JSON script as inert data, not executable HTML. Validate returned track ID/type. Never search nested objects recursively for any convenient title/duration.
- If embed metadata is unavailable or changes layout, try documented oEmbed for title/artwork. Never execute or render its `html` field. Missing artist or duration stays missing. If both fail, return a recoverable issue with an editable blank query.
- Store duration as seconds (`213.573` for the observed example). Accept only finite positive duration <= 24 hours; invalid optional fields become absent. Trim and bound title (512 characters), artist names (256 each, maximum 20), user query (300), candidate display fields (512). Treat oversized required fields as an explicit parser issue rather than silently misidentifying a song.
- Artwork is optional; allow exact known Spotify image hosts observed/documented during implementation (`i.scdn.co`, `image-cdn-ak.spotifycdn.com`), HTTPS/default port only. Missing or rejected artwork shows a placeholder. Never fetch a URL taken from returned HTML.
- No persistent raw pages or tokens. Fixture files contain only public metadata and necessary schema fragments. No Spotify metadata disk cache in Phase 1; one in-memory UI session is sufficient.

## Search and ranking

Use a separate `search_spotify_matches` command. Construct exactly one argv search operand `ytsearch5:<query>` after `--`; query defaults to `<artists joined by comma> - <title> audio`, or title alone when artist is unknown. User edits are literal search text, never URLs/options/extractor prefixes. Reuse `tools::common_args`, the engine read lock and `probe::run_lookup`, with flat search results; no media downloads or five full candidate probes. Existing cookie settings apply to YouTube only, never Spotify HTTP requests.

Accept at most 5 distinct potentially playable YouTube video identities from the returned entries. Normalize to `https://www.youtube.com/watch?v=<id>` with a valid 11-character ID. Ignore malformed, unavailable, live/upcoming entries and external URLs; preserve stable ordering on ties. Search results have their own DTO, not `ProbeResult` or a downloadable playlist. Probe only the user-selected candidate with the existing `api.probe` to verify audio formats and obtain actual preview/estimate data.

Extend `ProbeResult` with backward-compatible `audioAvailable: boolean|null` (`#[serde(default)]` Rust `Option<bool>`; optional/nullable in TypeScript). Full single-video probing sets true only if a format has a nonempty audio codec other than `none`, an HTTP(S) media URL, and `has_drm` absent or false; audio-only and muxed formats both qualify. Explicit empty formats or formats without a qualifying audio stream give false; absent format data, playlists and older cached records give null/unknown. A Spotify match must have a fresh selected-candidate probe with `audioAvailable === true`; false or unknown stays unconfirmed with a retry/change-match explanation. This is availability evidence at lookup time, not a guarantee that later source access will succeed. Ordinary preview/download behavior remains unchanged by the new field.

Ranking is a deterministic ordering aid, never an accuracy probability. Normalize Unicode letters/numbers by lowercasing and punctuation-to-space; preserve non-Latin text and diacritics initially. Title similarity uses token-set Dice similarity. Artist similarity uses the maximum token Dice overlap against candidate title or uploader for each requested artist, then the mean. Duration closeness is `max(0, 1 - abs(candidate-target)/max(10, 0.10*target))`. Weighted score: title 0.55, artist 0.30, duration 0.15; renormalize weights over available signals. Missing artist/duration and partial metadata produce visible warnings, never fabricated values.

Subtract 0.25 for each distinct unwanted version category (live, cover, remix, karaoke, instrumental, sped-up, slowed), capped at 0.75, when that category occurs in the candidate title but not the requested title. Use explicit whole-token/phrase categories; avoid substrings such as `live` inside `Believe`. Clamp final score to [0,1]. Display reasons such as “Similar title”, “Duration differs by 45 s” and “May be a live recording”; do not label any match exact or verified. Mark duration differences above `max(10 seconds, 10% of target)` as a warning. No threshold hides otherwise valid candidates.

## Interaction and lifecycle

1. Spotify input is routed before the ordinary cached-link/yt-dlp probe path. Backend `probe_url` also guards Spotify inputs with readable guidance so batches/other callers cannot emit the raw DRM error.
2. Show Spotify metadata and an editable query, with loading/failure status. Once resolved, perform one initial search. No metadata/network failure enables Download.
3. Show at most five radio-selectable results with title, channel, duration, thumbnail and warning reasons. Nothing starts selected. Use “Use this match” to probe the selected result; then show the Spotify identity and selected YouTube preview together.
4. The confirmed preview is audio-only and exposes the existing audio formats, Fast Save and matching-source-audio behavior. Its estimate and download use the YouTube URL and duration. Copy: “Source: YouTube match” and “Audio is downloaded from YouTube. The recording may differ from the Spotify version.” Existing source quality still limits FLAC/WAV output.
5. A separate Download action enqueues once. Repeated clicks while probing/enqueuing are disabled. Search edits, reruns, candidate changes and a replacement input clear confirmation and estimates. Cancellation/navigation invalidates pending replies using session and request generations. A late probe must never confirm a different candidate.
6. Store only settled matching state across navigation (metadata, query, results, selected ID and confirmed preview). Pending operations return as interrupted/retryable, not permanently loading. Clear on successful enqueue or replacement input; no automatic matching on restore. Keep this separate from ordinary `PreviewMemory` and batch sessions, and explicitly define which single-link workflow owns the screen.
7. Metadata failure: “Couldn't read this Spotify track. Enter the artist and song title to search YouTube.” Action “Find on YouTube” starts an in-app edited-query search. Empty results and search/probe failures have distinct retry/change-query actions. Unsupported album/playlist types say “Spotify albums and playlists aren't supported yet. Paste a single track link.”

## Provenance and persistence

Add optional `matchOrigin` to enqueue items, jobs and Library records, with serde defaults for old files. Schema: `{ kind: "spotifyYouTube", spotifyUrl: string, spotifyTitle: string|null, spotifyArtists: string[] }`. Backend validates canonical Spotify identity, bounded fields, audio mode, no playlist index and a canonical YouTube job URL when this field is present. UI confirmation is an interaction requirement; IPC does not claim to cryptographically prove a user's click.

`Job.url` and `HistoryItem.url` remain the actual YouTube URL. Job title, uploader, duration and artwork remain those of the selected YouTube recording; Spotify identity is separate. Phase 1 does not override embedded tags/artwork with Spotify data. Library and Queue show the source label for matched items; a duplicate uses the existing YouTube+options identity and must not overwrite another job's origin. Resumes and retries retain the exact chosen URL and provenance; never re-search automatically. New fields must survive history mutation/repair paths and restart serialization.

The ordinary link cache contains the YouTube result only. Pass the YouTube URL as enqueue's existing `source`; never store a YouTube `ProbeResult` under a Spotify URL. Revisiting a Spotify link requires review again.

## Acceptance and later scope

Ship only after offline resolver/network/search/interaction/persistence tests pass and a Windows manual smoke verifies public metadata, edited searches, explicit confirmation, actual audio saves and restart behavior. Tests against saved fixtures do not establish live Spotify or YouTube compatibility. Record blocked live checks honestly.

Albums/playlists are a separate future design requiring track-list completeness, pagination/truncation visibility, per-track approval, partial failures and rate-limit behavior. Do not infer completeness from a limited embed list.
