# Spotify Track Matching Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Accept a single Spotify track link, require the user to choose a YouTube recording, and save that recording using Claws' existing audio pipeline with durable source attribution.

**Architecture:** Add isolated Rust metadata/search modules and typed IPC commands, then a separate frontend matching controller and confirmation view. Reuse ordinary probing, estimates, enqueue, execution and Library publication only after a YouTube result is chosen. Keep Spotify identity out of `ProbeResult` and out of media URLs/cache keys.

**Tech Stack:** Existing Tauri 2, Rust, reqwest 0.13, serde_json, Tokio, managed yt-dlp, TypeScript, Vite and Node test runner. Do not bundle spotDL or Python. Prefer existing dependencies; any small HTML parser dependency must support the declared Rust floor and be justified with the parser fixtures.

**Spec:** [Spotify track matching design](../specs/2026-11-09-spotify-track-matching-design.md). Read both documents. Scope and numerical constants below are proposed implementation decisions for review.

**Execution status:** Implemented and packaged in Claws 1.0.8. The tasks below are the historical implementation specification, not unfinished work. Current checks and remaining real-short-link/listening/WebView2 limits are in [Spotify verification](../../spotify-track-matching-verification.md) and [release notes](../../releases/1.0.8.md).

## Global Constraints

- Phase 1: individual track URLs and bounded short links only; Spotify batch rows are actionable failures, never auto-matched.
- No Spotify login/OAuth/credentials, stream extraction, audio-preview use or automatic match selection.
- Every Spotify HTTP hop: HTTPS, exact allowlisted host, default port, no credentials. Five redirects maximum, 5-second connect, 20-second whole resolution, 2 MiB decoded body limit.
- Search: 300-character maximum literal query, five candidates maximum, no full probe until user selects one; reuse bounded process execution and engine lock.
- No confirmed YouTube probe means no Download. Any query/candidate/input replacement invalidates confirmation and dependent estimates.
- Source URL/duration/options/estimate refer to YouTube. Spotify provenance stays separate and survives restart without re-matching.
- Retain existing queue/disk/publication behavior, format choices and backwards compatibility. Do not change user settings or personal files for development tests.
- The user subsequently authorized source implementation. Version changes, installer builds and publication remain separate work.

## Review Focus

1. Redirect chains that leave the allowlist, loop or change entity type must fail before contacting an unapproved destination (Task 1).
2. Missing, relinked or structurally changed metadata must never identify a different track or invent artist/duration (Task 1).
3. Two overlapping searches/probes or navigation during enqueue must not confirm/download the wrong recording (Tasks 4–5).
4. Restart/retry and ordinary/matched duplicate enqueue must retain the actual chosen source and existing ownership records (Task 3).
5. Mixed batches and cached previews must not create a path that bypasses confirmation or puts Spotify URLs into yt-dlp (Tasks 2 and 5).

## File map and dependency order

| File | Responsibility |
| --- | --- |
| New `src-tauri/src/spotify.rs` | Input classification, canonicalization, dedicated HTTP client, metadata DTOs, bounded resolution and pure parsers |
| New `src-tauri/src/spotify_match.rs` | YouTube search argv, candidate parsing, ranking and match-origin validation |
| New `src-tauri/tests/fixtures/spotify/*` | Minimal public embed/oEmbed fixtures, malformed/partial variants and search results; README with capture date/schema |
| `src-tauri/src/state.rs`, `lib.rs`, `commands.rs` | Reusable dedicated client, module declarations, command registration and IPC |
| `src-tauri/src/probe.rs` | Spotify guard before starting generic yt-dlp probing |
| `src-tauri/src/downloader.rs`, `history.rs` | Optional match provenance validation, persistence and record copying |
| `src-tauri/src/download_tests.rs`, `lifecycle_tests.rs`, `history_mutation.rs` | Existing construction sites and meaningful persistence/lifecycle regressions |
| `src/api.ts` | Shared frontend DTOs and invoke wrappers |
| New `src/spotify-match.ts` | Framework-free state/controller, injectable async dependencies and request-generation protection |
| New `src/views/spotify-match.ts` | Matching presentation and audio-confirmation callbacks, using existing UI primitives |
| `src/views/download.ts`, `store.ts`, `styles/views.css` | Input routing, matching session ownership, ordinary-preview integration and minimal styling |
| `src/views/queue.ts`, `library.ts` | Durable source labels |
| New `scripts/tests/spotify-match.test.mjs`, `spotify-match-dom.test.mjs` | State and actual presentation/callback regressions |
| `package.json`, `README.md` | Test compilation/scripts and user-facing support boundaries |

Implement Task 1 → Task 2 → Task 3 → Task 4 → Task 5 → Task 6. Keep new responsibilities out of the already large download view where possible. Do not restructure unrelated code. At execution start, inspect `git status`, applicable instructions and current release/review work; this plan was written against the 2026-11-09 source and must not overwrite concurrent work.

### Task 1: Resolve Spotify identity and partial metadata safely

**Files:** create `spotify.rs` and fixtures; modify `state.rs`, `lib.rs`; conditionally update `Cargo.toml`/`Cargo.lock` only if a focused HTML parser is necessary.

**Interfaces:**

```rust
// serde camelCase DTOs; ordinary Result errors reserved for invalid input/internal failure.
SpotifyTrack { url: String, id: String, title: Option<String>, artists: Vec<String>,
               duration: Option<f64>, artwork: Option<String> }
SpotifyResolution { track: SpotifyTrack, warnings: Vec<String> }
SpotifyInput = Track { id: String } | Short { url: String } | Unsupported
pub fn classify_spotify_url(raw: &str) -> Option<SpotifyInput>;
pub fn parse_embed(raw: &[u8], expected_id: &str) -> Result<SpotifyTrack>;
pub fn parse_oembed(raw: &[u8], expected_id: &str) -> Result<SpotifyTrack>;
pub async fn resolve(app: &AppHandle, raw: &str) -> Result<SpotifyResolution>;
```

Recoverable metadata failure returns a track stub with validated canonical URL/ID, absent fields and warnings, allowing manual query entry. Short-link failure without a canonical track is an error with “paste the full Spotify track link” guidance. `AppState.spotify_http` is reusable and has redirects disabled; never change `AppState.http` behavior.

- [ ] Write failing parser tests `spotify::tests`: canonical query/fragment removal, international path prefix, exact 22-character IDs; reject deceptive hosts, userinfo, ports, HTTP, unsupported types and unexpected returned entity ID/type. Parse the observed duration as `213.573`, preserve Unicode/multiple artists, and assert missing duration is `None`.
- [ ] Add network tests using an injected internal transport/test endpoint boundary, with production URL policy independently enforced: 5 versus 6 redirects, loop, external/private/literal-IP destination rejected before request, downgrade, relative Location, album destination, 404/429/500, chunked body cap, stalled response and total-budget exhaustion across hops. Test both supported image hosts and rejected artwork. Test an outsize required title and malformed/absent script as recoverable metadata failure; a different returned ID must not contribute any metadata.
- [ ] Run `cargo test --locked --manifest-path src-tauri/Cargo.toml spotify::tests`; expect failures for the new contracts before implementation.
- [ ] Implement validation and streaming fetch limits first, then narrowly targeted script JSON parsing and oEmbed fallback. Keep `html`, preview URLs and tokens unused; never copy a whole hydration document into persistent state. Use safe URL query construction for oEmbed.
- [ ] Rerun the same filter; expect all tests passing without external network. Document fixture provenance and the exact supported script path. Add sanitized variants for multiple artists, missing optional values and changed markup.
- [ ] Implementation checkpoint commit: `feat: resolve public Spotify track metadata safely`.

### Task 2: Search YouTube candidates and expose distinct IPC

**Files:** create `spotify_match.rs`; modify `commands.rs`, `lib.rs`, `probe.rs`, `api.ts`; add search fixtures.

**Interfaces:**

```rust
YouTubeCandidate { url: String, video_id: String, title: String,
                   uploader: Option<String>, duration: Option<f64>,
                   thumbnail: Option<String>, score: f64, warnings: Vec<String> }
pub async fn search(app: &AppHandle, track: SpotifyTrack,
                    query: &str) -> Result<Vec<YouTubeCandidate>>;
// Tauri commands resolve_spotify_track(url) -> SpotifyResolution
// and search_spotify_matches(track, query) -> Vec<YouTubeCandidate>.
```

Validate all IPC inputs again; metadata is untrusted even when originating from the resolver. Frontend wrappers: `api.resolveSpotify(url)` and `api.searchSpotify(track, query)` with mirrored camelCase types. No changes to `ProbeResult.kind`.

Add `ProbeResult.audio_available: Option<bool>` with serde default and mirrored `audioAvailable?: boolean | null`. Full probes derive it from formats using the spec's codec/media-URL/DRM predicate; old caches and missing formats remain unknown. This adds evidence for matched-audio confirmation without changing ordinary download behavior. Update existing Rust `ProbeResult` literals as compilation identifies them.

- [ ] Write failing tests `spotify_match::tests` for exact search argv (one literal query operand), candidate ID normalization, five-result cap, duplicates/external results, unavailable/live entries, and stable ties. Pin score arithmetic from the spec: identical title/artist/duration = 1; unknown artist/duration renormalizes title; a +45-second duration difference on a 200-second target warns; cover/live versions rank below a clean recording. Pin non-Latin titles and `Believe` versus the whole-token `live` category. Never assert a score is an accuracy probability.
- [ ] Write `probe::tests` coverage proving any recognized Spotify input returns friendly guidance before engine discovery/spawn, including album/playlist and batch callers.
- [ ] Add `probe::tests` audio-availability fixtures: audio-only and muxed audio true; video-only, empty formats and DRM-only false; absent formats/playlist/old serialized record unknown. A successful metadata parse without usable audio must not count as match confirmation.
- [ ] Run the two new test filters separately; observe the expected failures. Implement search using `tools::common_args`, engine read lock, `--dump-single-json --flat-playlist --skip-download`, existing `run_lookup` and fixed `ytsearch5:` prefix after `--`. Do not full-probe five results or save search JSON as media cache data.
- [ ] Mirror DTOs and register commands; rerun both filters and `npm run build`. Expect no external requests in unit tests and no new TypeScript errors.
- [ ] Implementation checkpoint commit: `feat: search and rank YouTube matches for Spotify tracks`.

### Task 3: Persist the selected recording's provenance

**Files:** `spotify_match.rs`, `downloader.rs`, `history.rs`, `api.ts`, `download_tests.rs`, `lifecycle_tests.rs`, relevant construction sites in `history_mutation.rs` and other test modules discovered by compilation.

**Interfaces:** `MatchOrigin` as specified in the design; `Option<MatchOrigin>` with `#[serde(default)]` on `EnqueueItem`, `Job` and `HistoryItem`; TypeScript `matchOrigin?: MatchOrigin | null`. Validator: `validate_match_origin(origin: &MatchOrigin, url: &str, playlist_index: Option<u32>, options: &DownloadOptions) -> Result<()>`.

- [ ] Write failing tests `spotify_match::tests::origin_*` for canonical Spotify/YouTube identities, bounded text/artists, audio-only/no-playlist requirements and malformed fields. No origin must leave ordinary job validation unchanged.
- [ ] Write persistence regressions named `spotify_origin_*`: deserialize old records with absent field; enqueue/serialize/reload a matched job; copy provenance through `history::record`; preserve it in history updates/repairs; retry/resume retains URL; duplicate ordinary/matched items preserve the first accepted job and its origin. Assert `Job.url` and `HistoryItem.url` are YouTube URLs and ownership evidence/kept files survive.
- [ ] Run the new filtered tests, confirm failures, add validation and record copying with serde defaults; update existing Rust struct literals explicitly. Do not put provenance in execution options or dedup keys. Do not override yt-dlp metadata tags.
- [ ] Run `cargo test --locked --manifest-path src-tauri/Cargo.toml spotify_origin`, `cargo test --locked --manifest-path src-tauri/Cargo.toml spotify_match::tests`, then full Rust tests. Expect old fixtures and lifecycle behavior to remain valid.
- [ ] Implementation checkpoint commit: `feat: retain Spotify match provenance in queue and library`.

### Task 4: Implement a testable matching state controller

**Files:** new `src/spotify-match.ts`, `scripts/tests/spotify-match.test.mjs`; modify `store.ts` and `package.json`.

**Interfaces:** export `SpotifyMatchController`, `SpotifyMatchSnapshot`, and `SpotifyMatchDependencies` (resolve, search, probe). Controller methods: `open(url)`, `setQuery(query)`, `search()`, `select(videoId)`, `confirm()`, `snapshot()`, `restore(snapshot)`, `dispose()`. Async methods return promises; dependencies use Task 2 DTOs and ordinary `ProbeResult`. Publish typed immutable state to a supplied `onChange` callback. State stages: resolving, editable, searching, choosing, probing, confirmed, failed; warnings/issues are separate from stage. `confirmed` includes the exact selected canonical URL and successful fresh single-video probe with `audioAvailable === true`. Backend verifies URL normalization again at enqueue.

- [ ] Write tests against deferred promises: no default selection; initial search once after successful metadata; manual search after metadata failure; blank/oversized query rejection; edit/select/rerun invalidates confirmation; late resolve/search/probe ignored; selected probe resolving to a different video or playlist rejected; unavailable chosen result remains unconfirmed.
- [ ] Test successful metadata probes with `audioAvailable` false, null or absent: all remain unconfirmed and Download stays unavailable; a fresh same-identity probe with true can confirm.
- [ ] Add navigation tests: snapshot contains plain settled data only; restored pending work is retryable, never loading forever; dispose suppresses callbacks; successful enqueue clears the owning session only; opening a new ordinary input clears matching state; another route cannot restore stale ordinary and Spotify previews together.
- [ ] Add `test:spotify` to compile `src/spotify-match.ts` into `.test-build` with the existing NodeNext/ES2022 convention, then run the new Node test files. Run `npm run test:spotify` and observe failing contracts.
- [ ] Implement the controller and a separate nullable store-owned matching snapshot. Use per-session and per-request generations, not DOM state or raw mutable promises. Keep `PreviewMemory<ProbeResult,...>` unchanged; store matching audio choices separately in its snapshot after UI integration.
- [ ] Run `npm run test:spotify` and `npm run test:logic`; expect pass. Implementation checkpoint commit: `feat: model Spotify matching confirmation and navigation state`.

### Task 5: Connect review UI, audio preview and enqueue

**Files:** new `src/views/spotify-match.ts`, `scripts/tests/spotify-match-dom.test.mjs`; modify `download.ts`, `store.ts`, `views.css`, `queue.ts`, `library.ts`, `package.json`.

**Interfaces:** export `renderSpotifyMatch(state, actions): HTMLElement` from the new view; callbacks dispatch controller actions, while the parent owns ordinary audio preview/options/estimates/enqueue. `startDownload` attaches `matchOrigin` only for the currently confirmed matching session. Parent integration must keep the source summary visible while rendering the chosen `ProbeResult`, force audio mode, and pass its YouTube URL as the existing enqueue `source` argument.

- [ ] Add failing DOM/controller integration tests using the repository's existing fake-DOM pattern: no download controls before confirmation; keyboard-operable radio selection with no preselected match; “Use this match” probes but does not enqueue; Download sends exactly one chosen YouTube URL and origin; query edits/selection changes disable Download and clear estimates; double clicks produce one enqueue; a late success cannot clear a replacement input's session. Confirm returned probe identity before accepting it.
- [ ] Test input routing before cached lookup, settled navigation restore, friendly metadata/search/probe errors, and mixed batches with Spotify plus an ordinary video. Spotify rows must contain individual-track instructions, remain unselected, never reach a generic yt-dlp probe, and leave ordinary rows usable. Test ordinary cached previews still work; no Spotify cache key may hold a YouTube `ProbeResult`.
- [ ] Run `npm run test:spotify`; observe failures. Implement the new presentation with existing `h`, `button`, layout and motion helpers. Render all remote titles/artists as text; never inject returned HTML. Use desktop side-by-side identity/recording summaries with stacked layout for narrow windows, placeholders for failed artwork, labelled query/search actions, live status announcements and visible keyboard focus.
- [ ] Integrate through `lookUp`, render, `saveSession`/restore and enqueue boundaries. Reuse the existing `renderOptions`, estimate and `startDownload` path for the confirmed YouTube result with Spotify context; do not build another download engine. Persist audio-format/Fast Save choices across route changes. A failed enqueue retains the confirmed choice for retry. Clear on accepted or already-queued outcome according to existing semantics.
- [ ] Show “Source: YouTube match” in confirmation, matched Queue rows and Library details; retain separate Spotify identity. Explain existing YouTube source quality for FLAC/WAV without implying Spotify lossless audio. Use the exact failure copy from the design and explicit edit/retry actions.
- [ ] Rerun `npm run test:spotify`, `npm run test:logic`, `npm run test:queue`, `npm run test:motion`, `npm run build`; expect all pass. Extend `test:spotify` compilation to include the new presentation module and dependencies, following existing DOM-test setup.
- [ ] Implementation checkpoint commit: `feat: review Spotify matches before audio downloads`.

### Task 6: Verify live boundaries and document support

**Files:** `README.md`; new `docs/spotify-track-matching-verification.md`. Fix any issues in their owning task before proceeding.

- [ ] Complete offline gates, recording counts/results and any ignored tests:

```powershell
npm run test:fonts
npm run test:spotify
npm run test:logic
npm run test:queue
npm run test:motion
npm run build
cargo build --locked --manifest-path src-tauri/Cargo.toml --features test-tools --bin claws-fixture-tool
cargo test --locked --manifest-path src-tauri/Cargo.toml
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets --features test-tools -- -D warnings
git diff --check
```

Use the repository's established Rust formatting check if present; otherwise run `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` and report pre-existing failures separately without reformatting unrelated files.

- [ ] Run read-only live metadata checks with normal Claws user agent, not just a browser-like agent: single/multiple artists, non-Latin title, localized URL, unavailable track, actual short link, album/playlist rejection. Record date, canonical IDs, response statuses and which fields were present. Short links using an unapproved intermediate host must fail safely with a full-link fallback; they are not a passing compatibility check.
- [ ] In an isolated Windows development app/data directory, verify search returns candidates, user can edit query, choose a deliberately different version, change their mind, confirm and save an authorized test recording. Exercise MP3, M4A, Opus, FLAC, WAV and original format where the source/tools permit, and Fast Save with metadata settings on/off; record any unsupported case rather than assuming it passed. Verify the actual saved file is playable and source attribution remains after restart. Do not use the user's queue/library/settings as test fixtures.
- [ ] Verify pause/resume/retry retains the selected source, a stale/removed YouTube result does not trigger a replacement search, duplicate handling is clear, navigating away during lookup/probe/enqueue is safe, and matched audio retains ordinary estimates/disk checks. Smoke an ordinary video, audio, transcript, playlist and mixed batch for regressions. Separate live-site blocks from fixture/test results.
- [ ] Document track-only support, matching limitations, no authentication, YouTube source/quality, supported link forms and fallback behavior in README. Verification report must distinguish tests, actual live metadata/search/audio evidence and remaining unverified checks.
- [ ] Implementation checkpoint commit: `docs: document and verify Spotify track matching`.

## Completion criteria and deferred work

Complete only when all offline gates pass and the Windows/live checks required above have evidence, or clearly report remaining blockers as incomplete. No input can enqueue an unconfirmed search result; generic Spotify lookup never emits the raw DRM error; persisted files accurately retain the chosen YouTube source. No release or installer is part of this plan.

Albums/playlists require a new design and plan: complete track discovery, pagination/truncation reporting, per-track confirmation, resumable review, partial enqueue failures and rate limiting. Do not extend Phase 1 by treating a Spotify collection as a normal yt-dlp playlist.
