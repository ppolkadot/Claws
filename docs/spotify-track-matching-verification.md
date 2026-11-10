# Spotify track matching verification

Packaged implementation: **Claws 1.0.8**, merged into `main` from `codex/release-1.0.8`; [GitHub release](https://github.com/ppolkadot/Claws/releases/tag/v1.0.8). [Release notes](releases/1.0.8.md) and the installer build record identify source commit `6794ea9`, artifact hashes and payload checks. Subsequent comment/documentation cleanup does not rebuild that artifact.

## Behavior covered

- Public single-track Spotify metadata, localized URLs and bounded short links; no login, DRM removal, Spotify stream extraction or collection/batch matching.
- Up to five YouTube candidates with deterministic title/artist/duration ranking and version warnings. Topic/VEVO and presentation annotations are normalized only for scoring; the fixture corpus is not a measured accuracy guarantee.
- No default selection. A fresh same-video probe with usable audio is required before a separate Download action. Audio options, estimates and execution use the YouTube source.
- Persistent Spotify provenance remains separate from the actual media URL/title/duration. Retry/resume retains the selected source without searching again. Library search includes Spotify identity.
- Request epochs and settled snapshots preserve safe navigation/recovery. Typed input errors govern Retry, while search/probe interruption points to the appropriate action.
- A stable query node preserves composition; Unicode queries use a 300-code-point limit. Confirmation/recovery focus, full-width matching, responsive review columns and empty-state layout have regression coverage.

## Packaged-source automated checks

These results were recorded for the 1.0.8 build on 2026-11-09, not inferred for every subsequent checkout.

| Check | Result |
| --- | --- |
| Spotify frontend/controller/DOM | 43 passed |
| Frontend logic, queue, motion, fonts | 156 / 19 / 63 / 7 passed |
| Full locked Rust suite | 444 passed; 3 opt-in live tests ignored |
| Mounted Edge harness | 37/37 passed; no browser exceptions |
| Production build and warning-denying Clippy | Passed |
| Installer archive, version and embedded assets | Passed; details in the release/build records |

The mounted harness uses actual frontend code with labelled synthetic IPC and a temporary Edge profile. It covers explicit selection, audio-only enqueue/source/origin, cache/batch routing, navigation and late replies, pending/failed/successful enqueue ownership, recovery, focus, layout, composition events and astral text entry. It is not a native WebView2 or OS-IME test. Desktop dark/light and narrow screenshots were inspected. Independent source reviews found no unresolved consequential issues after the fixes.

Run `npm run test:spotify` and `node scripts/dev/spotify-bench.mjs --out .test-build/spotify-bench/report.json` for current local results. Rust fixtures are under `src-tauri/tests/fixtures/spotify/`; broader commands are in the [handoff](HANDOFF.md#build-and-checks). Local logs/screenshots under ignored `.superpowers/` and `.test-build/` are optional working evidence, not required project files.

## Dated live/native evidence

On 2026-11-09, before the 1.0.8 packaging pass, the real Rust resolver with `Claws/1.0.7` user agent read:

| Spotify ID | Observed metadata |
| --- | --- |
| `4cOdK2wGLETKBW3PvgPWqT` | Never Gonna Give You Up; Rick Astley; 213.573 s; approved-host artwork. The localized `/intl-pt/track/` URL canonicalized to this identity. |
| `2VxeLyX666F8uXCJ0dZF8B` | Shallow; Lady Gaga and Bradley Cooper; 215.733 s. Artwork absent under the optional-field/host policy. |
| `7ovUcF5uHTBRzUpB6ZOmvt` | アイドル; YOASOBI; 213.233 s. Artwork absent under the optional-field/host policy. |

An all-zero unavailable track returned a stable identity and manual-search guidance with absent fields; album/playlist inputs were rejected. Real managed yt-dlp search returned five candidates for “Cher Believe audio”.

An isolated native test then explicitly selected `https://www.youtube.com/watch?v=n7wvAEDOxAs` and saved 12 variants: MP3, M4A, Opus, FLAC, WAV and Original, each with Fast Save off/on and metadata settings on/off respectively. ffprobe reported the expected MP3/AAC/Opus/FLAC/PCM/original-Opus codecs, files were nonempty, and all 12 Library records retained source/provenance after restart. Each variant used its own output folder to preserve overwrite protection. No personal queue, cookies, settings or downloads were changed. These runs were not repeated during packaging or comment cleanup.

## Opt-in live checks

These contact external services; inspect their source before running:

```powershell
cargo test --locked --manifest-path src-tauri/Cargo.toml spotify::tests::live_metadata_smoke -- --ignored --nocapture
$env:CLAWS_SPOTIFY_LIVE_YTDLP = Join-Path $env:APPDATA 'dev.claws.downloader/bin/yt-dlp/yt-dlp.exe'
cargo test --locked --manifest-path src-tauri/Cargo.toml spotify_match::tests::live_search_metadata_only -- --ignored --nocapture
$env:CLAWS_SPOTIFY_LIVE_TOOLS = Join-Path $env:APPDATA 'dev.claws.downloader/bin'
cargo test --locked --manifest-path src-tauri/Cargo.toml spotify_live_audio_save_keeps_selected_source -- --ignored --nocapture
```

## Remaining limits

No real `spotify.link` share was tested; redirect policy and typed terminal/transport failures are covered by local HTTP fixtures. No manual WebView2/OS-IME, listening-based playback, real-source outage/removal, installation or upgrade acceptance was performed. Native saves/ffprobe, synthetic browser tests and historical live metadata/search are separate evidence. They do not guarantee future site availability or the correct recording version. The verified local installer is attached to the GitHub release; no installation was performed.
