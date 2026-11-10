# Current development status

## Version and delivery

Source version: **1.0.8** on `codex/release-1.0.8`. The branch was pushed at `b96eccd`; later comment/documentation cleanup does not rebuild or replace its installer.

The [1.0.8 release notes](releases/1.0.8.md) and installer build record identify the packaged source (`6794ea9`), checksums and verification. The installer is unsigned and was not installed or published as a GitHub release during this work. Older release records and binaries remain historical evidence.

## Implemented behavior

- Windows Tauri app using yt-dlp/FFmpeg, bundled Polaris fonts, desktop glass and the existing system/solid material options.
- Video/audio downloads, independent-link batches, playlist selection, source-driven resolution caps, Compatible/Original output and caption-derived transcripts.
- Revisioned queue events, frozen execution settings, owned work folders, durable queue/history recovery and verified file publication/deletion. Settings schema is 2; queue schema is 3.
- Size/free-space previews, Fast Save, matching-source audio selection and background disk monitoring. Preview estimates do not replace admission checks, and processing-space checks remain best effort.
- Library actions, kept subtitles, keyboard navigation, preview restoration and outgoing copy-only Windows file drag.
- Spotify single-track metadata and user-confirmed YouTube matching, with explicit source attribution, safe short-link handling, Unicode query limits and provenance search. See [Spotify verification](spotify-track-matching-verification.md).
- UTF-8 yt-dlp output paths, preserving accented/non-Latin finished filenames without relaxing ownership checks.

## Remaining work and boundaries

| Area | Remaining evidence or decision |
| --- | --- |
| Real slow source | No affected source/format was supplied. Controlled benchmarks do not establish an Internet throughput improvement. Follow [speed diagnosis](performance/speed-diagnosis.md). |
| Native drag | Explorer/editor import, a stalled receiver, swapped buttons, long/UNC paths and Windows 10 acceptance need disposable-environment checks. See [manual acceptance](HANDOFF.md#manual-acceptance). |
| Installed application | Fresh installation/upgrade and manual real-WebView2 checks remain unverified. |
| Spotify | Real share-link redirect smoke, OS-level IME composition, listening-based playback and live outage/removal behavior remain unverified. Historical native saves and synthetic UI checks are recorded separately. |
| Disk behavior | Real peak space across all postprocessors and constrained-volume behavior need controlled measurements; no strict pre-FFmpeg launch gate is claimed. |
| Optional permission cleanup | `core:window:allow-destroy` is still used by `onCloseRequested`. Replace that close path before considering removal. |

Clip sections remain deferred. Speech recognition, lyrics and replay metadata are not implemented. macOS support, vocal removal and ProRes presets are outside the approved release scope. Native drag result refinements, line-ending policy and artwork preferences remain optional; dated review suggestions are not an active implementation queue.

## Verification

The packaged 1.0.8 source passed 444 Rust tests (3 opt-in live tests ignored), 288 frontend tests, 37 mounted Edge checks, warning-denying Clippy and the production build. Installer archive, executable version and embedded font/CSS/JS/license checks passed. These are release-specific observations; use the [handoff commands](HANDOFF.md#build-and-checks) to validate subsequent source changes.

Historical test counts and release chronology belong in release records and git history, not additional status sections. This page does not claim that fixture or headless-browser success proves native or live-site acceptance.
