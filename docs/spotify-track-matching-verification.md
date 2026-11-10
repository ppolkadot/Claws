# Spotify track matching verification

Implementation branch: `codex/spotify-track-matching`. Date: 2026-11-09.

This feature reads Spotify identity metadata and downloads a user-confirmed YouTube recording. It does not download Spotify audio. This report distinguishes fixture checks, actual mounted frontend checks, live network evidence and checks still pending.

## Implementation boundaries

- Single track and strictly bounded short-link resolution; localized paths; no batch matching, collections or Spotify authentication.
- Public embed JSON with identity validation, documented oEmbed fallback and editable manual search when fields are missing.
- Five YouTube candidates maximum; deterministic ranking and version/duration warnings. No initial selection and no media download during search.
- Fresh same-recording audio evidence required for confirmation, followed by a separate Download action using ordinary Claws audio options/estimates/queue.
- Optional backwards-compatible provenance on jobs and Library entries; actual media URL remains YouTube; retries never re-match.
- Settled matching state survives route navigation; in-flight enqueue ownership is retained separately so acceptance clears it and failure permits retry. Replacement inputs invalidate earlier UI ownership.

## Automated and mounted checks

Raw local run logs and screenshots are in the ignored `.superpowers/sdd/2026-11-09-spotify-track-matching/` and `.test-build/spotify-bench/` directories.

| Check | Observed result |
| --- | --- |
| `npm run test:spotify` | 27 passed (24 controller, 3 DOM) |
| `npm run test:logic` | 152 passed |
| `npm run test:queue` | 19 passed |
| `npm run test:motion` | 63 passed |
| `npm run test:fonts` | 7 passed |
| `npm run build` | TypeScript, Vite and bundled font checks passed |
| Fixture-tool build | Passed; native lifecycle tests use the built tool |
| Full locked Rust tests | 434 passed, 3 opt-in live tests ignored by the default suite |
| Warning-denying all-target Clippy with test-tools | Passed |
| Mounted headless Edge harness | 22/22 checks passed, zero browser exceptions |
| `git diff --check` | Passed |

Repository-wide `cargo fmt --check` is not clean: it starts by flagging unchanged `src-tauri/build.rs`, and existing files have broad formatting differences. No repository-wide reformat was made. New Rust modules and new lifecycle-test sections were formatted separately.

The mounted harness covers no default selection, separate selection/probe/download actions, audio-only options, actual enqueue URL/origin, interrupted probe navigation, restoration of choices, enqueue-pending disabling, failure after navigation, success clearing only the owning session, replacement ordinary and matching inputs during older enqueue success/failure, freshness reset and mixed-batch/cache routing. Desktop dark/light and narrow screenshots were inspected. An independent code review reported no actionable correctness or security findings.

The mounted UI harness is `node scripts/dev/spotify-bench.mjs`. It uses a temporary headless Edge profile, Vite and clearly labelled synthetic dev IPC; it exercises the actual Download view, controls, store, navigation and enqueue arguments. It does not establish native backend or live-site compatibility. Unit tests use `npm run test:spotify`; backend fixtures are under `src-tauri/tests/fixtures/spotify`.

## Live and native acceptance

The actual Rust resolver with the normal `Claws/1.0.7` user agent returned public metadata for:

- `4cOdK2wGLETKBW3PvgPWqT`: Never Gonna Give You Up, Rick Astley, 213.573 seconds and approved-host artwork; localized `/intl-pt/track/` input canonicalized to the same track.
- `2VxeLyX666F8uXCJ0dZF8B`: Shallow, Lady Gaga and Bradley Cooper, 215.733 seconds; artwork unavailable under the current optional-field/host policy.
- `7ovUcF5uHTBRzUpB6ZOmvt`: アイドル, YOASOBI, 213.233 seconds; artwork unavailable under the current optional-field/host policy.
- All-zero unavailable track: missing metadata returned a stable track identity and editable manual-search warning; no invented fields. Album/playlist inputs returned the unsupported-type explanation.

The opt-in `spotify_match::tests::live_search_metadata_only` test used installed yt-dlp through the actual bounded runner and normal argument builder. It returned five distinct YouTube candidates for “Cher Believe audio”; this is real search metadata evidence, not automatic selection.

The opt-in `lifecycle_tests::spotify_live_audio_save_keeps_selected_source` test copied installed yt-dlp/FFmpeg tools into an isolated temporary native app and explicitly chose `https://www.youtube.com/watch?v=n7wvAEDOxAs` from that search. All 12 variants saved successfully: MP3, M4A, Opus, FLAC, WAV and Original, each with Fast Save off/on and metadata settings on/off respectively. ffprobe identified MP3, AAC, Opus, FLAC, PCM and original Opus audio as expected. Every file was nonempty; all 12 Library records retained the exact selected URL and Spotify provenance after restart. The test sets a unique output subfolder per variant to respect Claws' existing refusal to overwrite a prior file. No personal queue, library, cookies or output directory was used or changed.

Reproduce the live checks explicitly (they contact external services; default tests remain offline with respect to Spotify/YouTube):

```powershell
cargo test --locked --manifest-path src-tauri/Cargo.toml spotify::tests::live_metadata_smoke -- --ignored --nocapture
$env:CLAWS_SPOTIFY_LIVE_YTDLP = Join-Path $env:APPDATA 'dev.claws.downloader/bin/yt-dlp/yt-dlp.exe'
cargo test --locked --manifest-path src-tauri/Cargo.toml spotify_match::tests::live_search_metadata_only -- --ignored --nocapture
$env:CLAWS_SPOTIFY_LIVE_TOOLS = Join-Path $env:APPDATA 'dev.claws.downloader/bin'
cargo test --locked --manifest-path src-tauri/Cargo.toml spotify_live_audio_save_keeps_selected_source -- --ignored --nocapture
```

Remaining acceptance limits: no real `spotify.link` sample was supplied/verified (allowlist, relative hops, redirect counts and rejection behavior are covered by local HTTP tests). No listening-based playback check or manual real-WebView2 review was performed; real native saves/ffprobe and the actual frontend in headless Edge were tested separately. Live pause/resume across a network outage and removal of a real chosen recording were not exercised; existing native fixture lifecycle tests cover persistence/retry, and controller/probe tests cover unavailable candidates. Live-site availability and recording-version correctness are not guarantees.

No installer is built or installed as part of this change. The installed Claws application is not updated by changing this branch.

## Review follow-up (2026-11-09)

Implemented from `docs/superpowers/plans/2026-11-09-spotify-review-followup.md`:

- Typed `resolve_spotify_track` failures `{ code, message, retryable }` (`unsupportedCollection`, `unsupportedUri`, `unsupportedEmbed`, `invalidTrackUrl`, `insecureUrl`, `shortLinkUnresolved`, `lookupFailed`); Retry is shown only when `retryable`.
- Identical warning/error text renders once; the positive "Similar title" warning is gone; results and empty outcomes are announced in one persistent polite status region.
- Automatic queries are bounded to 300 code points (title and ` audio` reserved, whole artists fill the rest); Library search also matches the recorded Spotify title and artists (`src/library-search.ts`).
- The query field is a single node per matching session; Enter during IME composition never searches; an identical in-flight search is not repeated.
- Ranking: artist overlap is `0.70 * max + 0.30 * mean`; ` - Topic`/matching `VEVO` uploaders and trailing presentation annotations are normalized for scoring only; "Artist/channel differs" caution added. Evidence: the adversarial cases in `spotify_match::tests` (covers, live, remaster, collaborations, non-Latin names, ties). These show ordering on a small hand-built corpus, not measured match accuracy.

Automated results at this change: Rust `cargo test --locked` 441 passed / 3 ignored; `clippy --all-targets -D warnings` clean; Node suites fonts 7, spotify 36, logic 156, queue 19, motion 63; `npm run build` passes.

At the original follow-up commits, live metadata/search/audio reruns, a real `spotify.link` redirect smoke, the mounted headless-Edge harness, and a real WebView2/OS-IME session were not rerun. The recovery pass below supersedes the mounted-harness gap. Synthetic composition events do not establish native IME behavior.

## Recovery and layout review fixes (2026-11-09)

Confirmed and fixed the supplied follow-up findings:

- Interrupted full/short-link resolution snapshots now expose Retry after navigation. Interrupted search and audio-check snapshots point to the corresponding existing action instead of a stale resolver Retry.
- Editing a known track's query and starting/succeeding at manual search clear resolver retry state. A short link with no known track keeps its retry path if its query field is edited.
- The persistent review host uses the full column before confirmation and two columns only for a confirmed recording on a wide viewport; the query node stays mounted throughout.
- Nested action hosts right-align Retry/Use this match. Empty notice/action/candidate hosts are hidden so they contribute no grid gaps. An empty live status remains mounted and accessible using the existing visually-hidden treatment.
- Empty search results have one polite status explanation, not a second error alert.
- Removed native `maxlength=300` because it counts UTF-16 units; the controller and backend still validate 300 Unicode code points. A real browser insertion of 200 astral characters reaches the search intact.
- Focus leaving a removed recovery control moves to the query, or to the stable busy status until the query is enabled. It returns to the query after resolution without overriding a user's intervening focus move.
- Short links reaching a known allowed Spotify album, playlist, embed or malformed track destination now preserve the typed terminal reason and `retryable: false`. Transient request failures remain retryable. Destination allowlisting still runs before classification; no host was added.

Regression evidence: five new Node/DOM regressions failed before the implementation; a separate busy-Retry focus regression failed before its final fix. The Rust short-to-album regression failed with `shortLinkUnresolved` before typed terminal fetch errors were added. Final runs: **444 Rust tests passed / 3 live tests ignored; 42 Spotify frontend, 156 logic, 19 queue, 63 motion and 7 font tests passed; production build and warning-denying all-target Clippy passed.**

The expanded actual mounted Edge harness passes **36/36** checks with zero browser exceptions, including all original 22 lifecycle checks plus desktop widths, right alignment, empty-host spacing, interrupted full/short resolution, stale Retry removal, recovery focus, persistent composition events, astral query entry and single no-results messaging. Command: `node scripts/dev/spotify-bench.mjs --out .test-build/spotify-bench/recovery-green.json`. Local run logs are in the ignored plan workspace. An independent review found no actionable correctness/security regressions.

These runs use local fixtures and synthetic dev IPC. They do not newly establish live short-link, live-site, native WebView2 or OS-level IME compatibility. The earlier dated native audio-format evidence remains unchanged; no installer, release or installed-app update was produced.
