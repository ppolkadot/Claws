# Current development status

## Version and delivery

**1.0.9a** is the Windows build on `main`. The npm, Cargo and Tauri version remains `1.0.9`, which is also the version Windows reports. See the [1.0.9a build notes](releases/1.0.9a.md) for the installer, checksum and verification evidence.

The earlier [1.0.9 GitHub release](https://github.com/ppolkadot/Claws/releases/tag/v1.0.9) has its own [release notes](releases/1.0.9.md) and installer.

## What 1.0.9a added

- Wallpaper Engine Steam Workshop downloads via SteamCMD. Claws saves the project and its assets as a ZIP. The [user guide](../README.md#wallpaper-engine) covers setup and sign-in.
- [UI cleanup](UNSLOP_UI_REVIEW.md) for copy, icons and spacing, plus a visible filename label and simpler Library hover behavior.

## What 1.0.9 added

The improvement plan on top of 1.0.8. The installer was installed and launched by the owner; the features below have not been exercised against real downloads.

- **Safety net:** Windows CI for every push and pull request.
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
| Wallpaper Engine | Live Valve metadata and the refusal of an anonymous request were verified. A successful download with an owning account still needs a live check. |
| Spotify | Real share-link redirect smoke, OS-level IME composition, listening-based playback and live outage/removal behavior remain unverified. Historical native saves and synthetic UI checks are recorded separately. |
| Disk behavior | Real peak space across all postprocessors and constrained-volume behavior need controlled measurements; no strict pre-FFmpeg launch gate is claimed. |
| Optional permission cleanup | `core:window:allow-destroy` is still used by `onCloseRequested`. Replace that close path before considering removal. |

Clip sections remain deferred. Speech recognition, lyrics and replay metadata are not implemented. macOS support, vocal removal and ProRes presets are outside the approved release scope. Native drag result refinements, line-ending policy and artwork preferences remain optional; dated review suggestions are not an active implementation queue.

## Verification

See the [1.0.9a build notes](releases/1.0.9a.md) for the checks run against the packaged source and installer.

Historical test counts and release chronology belong in release records and git history, not additional status sections. This page does not claim that fixture or headless-browser success proves native or live-site acceptance.
