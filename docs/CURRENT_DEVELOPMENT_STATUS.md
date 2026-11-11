# Current development status

## Version and delivery

Source version on branch `improve/phase-0-safety-net`: **1.0.9**, not yet merged or published; see the [1.0.9 release notes](releases/1.0.9.md). `main` is 1.0.8, whose [GitHub release](https://github.com/ppolkadot/Claws/releases/tag/v1.0.8) carries the verified installer.

The [1.0.8 release notes](releases/1.0.8.md) and installer build record identify the packaged source (`6794ea9`), checksums and verification. The installer is unsigned; installation/upgrade remains unverified. Build records describe packaging-time state, while the GitHub release records publication. Older release records and binaries remain historical evidence.

## Unreleased work

Branch `improve/phase-0-safety-net` ([pull request 1](https://github.com/ppolkadot/Claws/pull/1)) carries the improvement plan on top of 1.0.8 and is packaged as the unpublished 1.0.9 installer. None of it has been exercised in the native app.

- **Safety net:** Windows CI for every push and pull request; new installers are no longer committed.
- **Download speed:** the benchmark gate did not justify `formats=dashy` or an aria2c transport ([real-source benchmark](performance/real-source-benchmark.md)); processing no longer holds a transfer slot (`max_processing`); a per-download speed limit; the fragment default stays at 8 ([fragment count](performance/fragment-count.md)).
- **Setup:** Claws installs Deno itself and a first-run panel installs yt-dlp, ffmpeg and Deno from one button. yt-dlp is no longer fetched without asking.
- **Queue and Library:** bulk actions, Move to Top, a summary line, the processing step, error details with Copy, taskbar progress, finish notifications, Library sorting and filtering, remembered choices. Dropping a link onto the window is deferred (see the [handoff](HANDOFF.md)).
- **Motion:** page-only navigation, a transform-based sidebar, gliding Library cards, a lookup skeleton, one easing for leaving rows ([motion polish](performance/motion-polish.md)).
- **Features:** chapters named with the metadata pass, optional SponsorBlock for YouTube, playlist folders ([specification](superpowers/specs/2026-11-11-chapters-sponsorblock-playlist-folders.md)). Clip sections and a `claws://` link handler stay deferred.

Settings schema stays 2 and queue schema stays 3; every new field is defaulted.

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
