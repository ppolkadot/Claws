# Current development status

## Version and delivery

**1.0.9e** is the Windows build on `main` and [GitHub Releases](https://github.com/ppolkadot/Claws/releases/tag/v1.0.9e). The npm, Cargo and Tauri version files stay `1.0.9`, which is also the version Windows reports. See the [1.0.9e build notes](releases/1.0.9e.md) for the installer, checksum and verification evidence.

The earlier [1.0.9 GitHub release](https://github.com/ppolkadot/Claws/releases/tag/v1.0.9) has its own [release notes](releases/1.0.9.md) and installer.

## In development (after 1.0.9e)

Follows [the 2026-11-26 plan](superpowers/plans/2026-11-26-wallpaper-recording-progress-steamcmd-removal.md); measurements and the places the work differs from the plan are in the [Wallpaper Engine study](wallpaper-engine-local-library-study.md#implementation-results-2026-11-26). The version files stay `1.0.9`. No installer has been built from this work.

- **SteamCMD is gone.** The Steam library is the only Wallpaper Engine source: no install, sign-in, account name or Steam account detection. Settings files from 1.0.9e that still hold the old fields load, and the fields are dropped on the next save. A leftover SteamCMD folder in Claws' tools folder is offered for removal in Settings → Wallpaper Engine, never deleted without confirmation.
- **Steam's state is shown.** Installed, still downloading (a real percentage from the bytes on disk against Valve's size) or an update pending, from `appworkshop_431960.acf` and the downloads folder.
- **MP4 (original) copies.** An 8-bit H.264 MP4 video wallpaper is copied byte for byte with a byte count (the 5.6 GB 4K sample, minutes to re-encode before, copies in under a minute even from a cold disk). Everything else converts (**MP4 (converted)**); above 1440p with a hardware encoder when one works, at the source's resolution. GIF is cut to 30 seconds (60 on request) and capped at 720p.
- **The export tracker.** Every export reports its stage, a count and a time left only when they can be backed, ends with "took m:ss", feeds the taskbar and sends a notification for exports over 30 seconds while Claws is hidden. A pre-start estimate sits under the format choice; conversion speeds are remembered.
- **Recording.** Scene and web wallpapers can be recorded to MP4 or saved as a picture through Wallpaper Engine's documented command line (signature verified, window hidden, focus returned, closed on every exit). Sizes follow the authored size, up to 4K, never stretched.

Open items: see the study's list of what remains unverified (fullscreen games, a locked screen, scene sound, a heavy GPU load, AMD and Intel encoders).

## What 1.0.9e adds

- SteamCMD acceptance recognises both exact publisher names Valve uses: `Valve` (the official bootstrap ZIP) and `Valve Corp.` (an updated copy). Both still need a signature Windows verifies with `WinVerifyTrust`, and lookalike names are refused.
- A video Wallpaper Engine item can be saved as **MP4** (H.264 with AAC audio when present) or **GIF** (silent, 15 fps, a palette per frame so FFmpeg's memory stays bounded for long clips) beside the default project **ZIP**. Claws offers the video formats only when Valve's Workshop type is Video, then checks the downloaded `project.json` and reads only the video file it declares inside the item. Scene, web and application projects stay ZIP and are never run.
- Conversion uses the managed FFmpeg under the existing tool guard, with one local input, no remote protocols, bounded diagnostics, cancellation and free-space checks. Odd frame sides are rounded down to even numbers instead of scaling up; source resolution and aspect ratio are otherwise kept. A failed or cancelled conversion leaves no partial file, an existing export is never replaced, and SteamCMD's copy is removed only after a successful save.
- The opt-in live SteamCMD test (`CLAWS_TEST_STEAMCMD`) accepts several `;`-separated disposable executable paths.

## What 1.0.9a added

- Wallpaper Engine Steam Workshop downloads via SteamCMD. Claws saves the project and its assets as a ZIP. The [user guide](../README.md#wallpaper-engine) covers setup and sign-in.
- [UI cleanup](UNSLOP_UI_REVIEW.md) for copy, icons and spacing, plus a visible filename label and simpler Library hover behavior.

## What 1.0.9c adds

- SteamCMD installs from Claws, like yt-dlp and ffmpeg: Valve's download, accepted only with Valve's signature, then one run so it updates itself. A copy you already have can still be chosen.
- The Steam account name fills in from this PC (SteamCMD's own sign-ins first, then the Steam client's), with the others offered as suggestions.
- Windows' own minimize, maximize and close buttons no longer exist behind the window's top-right corner (the system menu style is removed; Alt+Space goes with it).
- The sidebar shows the installed Claws build (for example `Claws 1.0.9c`) beside the tool dot.

## What 1.0.9b adds

Follows [the 2026-11-25 plan](superpowers/plans/2026-11-25-unslop-and-wallpaper-engine-plan.md).

- Wallpaper Engine: a preview card, the Steam library source (no sign-in in Claws), SteamCMD with a saved, Valve-signature-checked location and account, Sign in to Steam, progress and Cancel, classified SteamCMD errors with the full output, title-based ZIP, MP4 and GIF names, free-space checks, collections refused, required items flagged, a Settings section, and a full dev mock. See the [no-sign-in spike](wallpaper-engine-no-login-spike.md).
- Second unslop pass ([review log](UNSLOP_UI_REVIEW.md#2026-11-25-second-pass)): site names, English dates, audio counts, shorter copy, and edge alignment. Before and after screenshots of the dev mock are in [performance/2026-11-25-unslop](performance/2026-11-25-unslop/).
- Toolbar: the buttons now centre on the traffic lights' line (`y = 34px`, toolbar 68px tall), and only the window controls take clicks in their corner.
- Queue summary: at most three facts (downloading with its speed and time left, then failures). With nothing transferring it shows only the paused and failed counts.
- Drag file is an icon button in finished Queue rows and Library tiles, so every Queue row ends on the same line.
- Download page: a Back control and the Esc key return to the empty Download page from link details, wallpaper items, Spotify matches, lists and errors. A kept list stays one **Back to links** away, and a running wallpaper export keeps running.

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
| Wallpaper Engine | Live Valve metadata and collection detection are covered by the opt-in live test (`CLAWS_TEST_NETWORK`). Recording, copy and conversion are covered by opt-in live tests on the owner's PC (see [HANDOFF](HANDOFF.md#development-tools)); the installed native app, a fullscreen game, a locked screen, scene sound and AMD or Intel encoders remain unverified. MP4/GIF conversion is covered against generated FFmpeg fixtures with ffprobe. Still to check live: a real video wallpaper's conversion, a SteamCMD download with an owning account, the Steam library source on a PC with Steam, and both in the native app. |
| Spotify | Real share-link redirect smoke, OS-level IME composition, listening-based playback and live outage/removal behavior remain unverified. Historical native saves and synthetic UI checks are recorded separately. |
| Disk behavior | Real peak space across all postprocessors and constrained-volume behavior need controlled measurements; no strict pre-FFmpeg launch gate is claimed. |
| Optional permission cleanup | `core:window:allow-destroy` is still used by `onCloseRequested`. Replace that close path before considering removal. |

Clip sections remain deferred. Speech recognition, lyrics and replay metadata are not implemented. macOS support, vocal removal and ProRes presets are outside the approved release scope. Native drag result refinements, line-ending policy and artwork preferences remain optional; dated review suggestions are not an active implementation queue.

## Verification

See the [1.0.9a build notes](releases/1.0.9a.md) for the checks run against the packaged source and installer.

Historical test counts and release chronology belong in release records and git history, not additional status sections. This page does not claim that fixture or headless-browser success proves native or live-site acceptance.
