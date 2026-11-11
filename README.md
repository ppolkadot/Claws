<div align="center">

# Claws

**A local video downloader for Windows.**

Paste a link, choose a quality, and the file lands on your disk. Built with native Windows desktop blur, spring-driven UI motion and no cloud dependencies: everything runs locally on your PC.

[![Release](https://img.shields.io/github/v/release/ppolkadot/Claws?logo=github&logoColor=white&color=34C759)](https://github.com/ppolkadot/Claws/releases/latest)
[![CI](https://github.com/ppolkadot/Claws/actions/workflows/ci.yml/badge.svg)](https://github.com/ppolkadot/Claws/actions/workflows/ci.yml)
![Platform](https://img.shields.io/badge/platform-Windows%2011-0078D4?logo=data:image/svg%2Bxml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAyNCAyNCIgZmlsbD0id2hpdGUiPjxwYXRoIGQ9Ik0wIDBoMTF2MTFIMHptMTMgMGgxMXYxMUgxM3pNMCAxM2gxMXYxMUgwem0xMyAwaDExdjExSDEzeiIvPjwvc3ZnPg==)
![Tauri](https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white)
![Rust](https://img.shields.io/badge/Rust-1.85%2B-B7410E?logo=rust&logoColor=white)
![TypeScript](https://img.shields.io/badge/TypeScript-5-3178C6?logo=typescript&logoColor=white)
![License](https://img.shields.io/badge/license-MIT-34C759?logo=opensourceinitiative&logoColor=white)

[**Download the latest release**](https://github.com/ppolkadot/Claws/releases/latest) · [Documentation](docs/README.md) · [Release notes](docs/releases/)

<!--
Screenshot slot. Add high-DPI captures at docs/assets/screenshot-dark.png and
docs/assets/screenshot-light.png, then replace this comment with:

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/screenshot-dark.png">
  <img alt="Claws: the Download page on the desktop glass" src="docs/assets/screenshot-light.png" width="900">
</picture>
-->

</div>

---

## Contents

- [Quick start](#quick-start)
- [Features](#features)
- [Installation](#installation)
- [Using Claws](#using-claws)
- [Settings](#settings)
- [Troubleshooting](#troubleshooting)
- [How it works](#how-it-works)
- [Performance](#performance)
- [Privacy and security](#privacy-and-security)
- [Fonts](#fonts)
- [Building from source](#building-from-source)
- [Project structure](#project-structure)
- [Contributing](#contributing)
- [Acknowledgements and license](#acknowledgements-and-license)

## Quick start

1. Download the installer from the [latest release](https://github.com/ppolkadot/Claws/releases/latest) and run it.
2. Press **Set Up Everything** on first launch to install yt-dlp, ffmpeg and Deno.
3. Paste a link with <kbd>Ctrl</kbd>+<kbd>V</kbd>, pick a quality, press **Download**.

## Features

### Downloading

Claws runs on [yt-dlp](https://github.com/yt-dlp/yt-dlp), so it handles any site yt-dlp supports. It has been tested against YouTube; every other site depends on yt-dlp's own support for it. Paste a link, or press <kbd>Ctrl</kbd>+<kbd>V</kbd> anywhere in the window, and Claws reads the title, artwork, duration and the qualities on offer.

- **Formats.** Best quality, or a cap at any resolution the source actually offers, from 144p to 8K. Choices come from the source's own formats with exact dimensions (`4320p · 7680×4320 · 8K`), portrait video is judged by its shorter side, and nothing is upscaled. Audio can be saved as MP3, M4A, Opus, FLAC, WAV or the original stream; with *Prefer matching source audio* on, a stream already in the chosen format is copied rather than converted. Subtitles come as separate `.srt` files or embedded.
- **Compatible or Original.** *Compatible* prefers H.264 and AAC in MP4 when the site offers them. *Original* keeps the site's best streams without re-encoding, as MP4 when the container can hold them and MKV otherwise. Some editors cannot open every codec, and remuxing does not change that.
- **Previews.** Before you download, Claws shows the expected size and the free disk space it will need while working, computed from the same streams that will be fetched. Where a stream's size is not known it says so instead of inventing a number. A preview never replaces the check made when the download starts.
- **Many links at once.** Paste up to 500 links, one per line. They are looked up three at a time, a failed link can be retried alone, and links already in the queue are reported rather than added twice. Nothing starts until you press Download.
- **Playlists and channels.** Tick exactly the videos you want. *Playlist folder* saves them into a folder named after the playlist, numbered by position (`My Playlist\03 - Title [id].mp4`).
- **Transcripts.** Save a video's own captions as text, timestamped text, SRT or VTT without downloading the video. Human and automatic captions are listed separately. There is no speech recognition, and automatic captions can contain errors.
- **Spotify links.** Paste a single track link (`open.spotify.com/track/...` or a `spotify.link` short link) and Claws searches YouTube for matching audio. You preview the candidates and pick the right recording before anything downloads; the file is labelled `Source: YouTube match`. It is YouTube audio, so the recording may differ from the Spotify version. No Spotify account is needed, and albums, playlists and batches are not supported.
- **Chapters, SponsorBlock and Fast Save.** With *Embed metadata* on, a video's chapters are written into the file. For YouTube, an optional per-download SponsorBlock setting (off by default) either marks sponsor segments as chapters or cuts them out; it asks `sponsor.ajay.app` about the video only when enabled, and removing segments makes the file smaller than the preview and can leave a brief glitch at a cut. *Fast Save* skips metadata, chapters, cover art, SponsorBlock and embedded subtitles so a large file is not rewritten after it downloads. Merging and conversion the format requires still happen.
- **Into an editor.** A finished download has a *Drag file* handle that starts a real Windows file drag (copy only) into Explorer or an editor's media bin. The file stays in your Library. Press Enter on the handle to show it in its folder instead.

### Managing

The queue runs several downloads at once with live progress, speed and time left. Progress is weighted by bytes across the video and audio streams, and a download whose total is unknown shows bytes so far rather than a guessed percentage. You can pause, resume, cancel and retry jobs one at a time or all at once (**Pause All**, **Resume All**, **Retry Failed**), and **Move to top** puts a waiting job first in line; the order survives a restart. A summary line counts what is downloading, processing, waiting, paused and failed. While a file is processed, its row names the step (merging, converting audio, applying SponsorBlock) when yt-dlp reports it, and a failed job shows its full message with a **Copy** button. The taskbar button follows overall progress, and a Windows notification says when a download finishes or fails while Claws is hidden or behind another window. Unfinished jobs come back paused after a restart, with the folder and file name they started with.

A job that has finished transferring and is merging or converting no longer holds up the next transfer. Up to two such jobs run beside the transfer limit, so with the defaults up to five are active at once. An optional per-download **speed limit** is off by default.

Each download works in its own hidden folder until it is finished, and cancelling deletes that folder and nothing else. A finished file never replaces one that already exists. Before a large download starts, Claws estimates the space it needs, including the extra copy while merging, and keeps watching space when processing begins; those later checks are best effort, and a stopped job keeps its files for a retry.

- **Library.** Finished downloads with artwork and search, sortable by date, title or size and filterable by video, audio or transcript. The type, audio format, quality cap, preset and Fast Save of your last download are offered again for the next link.
- **Sign-in.** Use cookies from your browser, or a `cookies.txt` file, for sites that need a login.
- **Repeat lookups.** Links you have downloaded keep their details, so looking one up again shows it at once while Claws checks the site for changes.

### Quick access

Press <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>D</kbd> in any app to bring Claws forward; a link on the clipboard is looked up straight away, and pressing it again puts Claws away. The shortcut can be changed or turned off. Closing the window leaves Claws running in the notification area so downloads continue; <kbd>Ctrl</kbd>+<kbd>Q</kbd> or the icon's menu quits. Claws can also start quietly in the notification area when you sign in.

### Interface

The desktop shows through the window, blurred by the Windows composition API; the interface adds tint layers and an edge highlight on top and paints no background of its own. Layout follows familiar conventions: a floating sidebar you can collapse, a toolbar that frosts as content scrolls under it, and inset grouped settings. Motion is spring-driven (controls compress and release, the sidebar selection glides, Library cards move to their new places, a looked-up link settles into its preview without shifting the layout), but it never stands in for real state, large lists are not animated, and all of it respects *Reduce motion*. Light, dark or Windows' own setting are supported, with adjustable transparency and an opaque Solid mode.

## Installation

### From a release

1. Download `Claws_<version>_x64-setup.exe` from the [Releases](https://github.com/ppolkadot/Claws/releases) page. Current release: 1.0.9, SHA-256 `3ce6d049d578120fb0b156a5dd1eef9a0a48757d7dfd11002f581941e63b2283`.
2. Run it. Releases are not code-signed yet, so Windows SmartScreen may warn on first run; choose **More info → Run anyway**.
3. On first launch the Download page lists the tools that are missing: yt-dlp, ffmpeg and a JavaScript runtime (Deno). **Set Up Everything** downloads each into its own folder from its project's GitHub releases and verifies it against the published SHA-256 before use. Nothing is fetched until you press the button, a tool already on your `PATH` counts as ready, and ffmpeg and Deno can be skipped with **Not Now**.

Upgrading keeps your settings, Library and unfinished queue.

### Requirements

| | |
| --- | --- |
| OS | Windows 11. Windows 10 has not been tested. |
| Runtime | Microsoft Edge WebView2, included with Windows 11. |
| ffmpeg | Merges HD video with audio and converts audio. Claws uses the copy on your `PATH`, or installs one (first-run setup or **Settings → Tools**). |
| JavaScript runtime | Recent YouTube extraction needs Deno or Node.js. Claws installs Deno for you, or uses either one found on your `PATH`; **Settings → Tools** shows which. |

## Using Claws

1. **Paste a link** into the field on the Download page and press **Get Info**, or press <kbd>Ctrl</kbd>+<kbd>V</kbd> anywhere. For several unrelated links, paste one per line and press <kbd>Ctrl</kbd>+<kbd>Enter</kbd>.
2. **Choose** Video, Audio Only or Transcript, a quality, Compatible or Original, and optionally subtitles. For a playlist or batch, tick the items you want; **Estimate Sizes** totals the selection on request.
3. **Press Download.** The job joins the queue. Finished files appear in the Library, where the *Drag file* handle drops them into Explorer or an editor.

### Keyboard shortcuts

| Shortcut | Action |
| --- | --- |
| <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>D</kbd> | Quick Open, from any app (configurable) |
| <kbd>Ctrl</kbd>+<kbd>V</kbd> | Paste a link and look it up, from anywhere in the window |
| <kbd>Ctrl</kbd>+<kbd>Q</kbd> | Quit Claws (closing the window keeps it running) |
| <kbd>Ctrl</kbd>+<kbd>1</kbd> … <kbd>4</kbd> | Download, Queue, Library, Settings |
| <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>S</kbd> | Show or hide the sidebar |
| <kbd>Enter</kbd> | Look up the link in the URL field |
| <kbd>Ctrl</kbd>+<kbd>Enter</kbd> | Look up every link in a multi-line field |
| <kbd>Esc</kbd> | Close a confirmation sheet |

## Settings

| Section | Options |
| --- | --- |
| **Downloads** | Output folder (default `Downloads\Claws`), filename template (a yt-dlp output template), simultaneous downloads (1–8), processing alongside (1–8, default 2), speed limit (per download, off by default), embed metadata, prefer matching source audio |
| **Quick Access** | Quick Open shortcut (Ctrl+Alt+D, Ctrl+Alt+Space, Ctrl+Shift+Space or off), notify when a download finishes, keep running when closed, open at login |
| **Appearance** | Theme (System, Light, Dark), window material (Glass, Acrylic, Mica, Solid), transparency, font status |
| **Sign-in** | Cookies from none, a browser (Firefox, Chrome, Edge, Brave, Opera, Vivaldi), or a `cookies.txt` file |
| **Tools** | yt-dlp version and one-click update, ffmpeg status and install, and the JavaScript runtime (Deno or Node.js) with a Deno install when none is found |

Glass, the default, shows your desktop through the window. Acrylic and Mica are the Windows system materials, which Windows flattens while the window is in the background. Solid turns the material off.

Changes to the output folder, the filename template, *Embed metadata* and *Prefer matching source audio* apply to downloads that have not started yet; a download that has started, even a paused one, keeps the choices it started with. The speed limit is read each time a download starts or resumes, and a limited download fetches one fragment at a time because yt-dlp applies the limit to every fragment connection.

Settings, the Library, the unfinished queue, downloaded tools and caches live in `%APPDATA%\dev.claws.downloader\`.

## Troubleshooting

<details>
<summary><b>The window isn't blurring my desktop</b></summary>

- Make sure **Settings → Appearance → Window material** is set to **Glass**.
- Make sure *Transparency effects* is on in **Windows Settings → Personalization → Colors**.
- **Acrylic** and **Mica** turn flat whenever the window is in the background; that's Windows behaviour.
- Very old GPUs, Remote Desktop sessions and battery saver can disable blur entirely.
</details>

<details>
<summary><b>A site needs me to sign in</b></summary>

Choose **Settings → Sign-in → Browser**. Firefox is the most reliable source. Chrome, Edge and other Chromium browsers lock their cookie store while they are open, so close them first, or export a `cookies.txt` file and choose **File**.
</details>

<details>
<summary><b>A download that used to work fails now</b></summary>

Sites change often. Open **Settings → Tools** and press **Check for Updates** to get the newest yt-dlp. For YouTube, also check that **Settings → Tools** shows a JavaScript runtime (Deno or Node.js).
</details>

<details>
<summary><b>Quick Open does nothing when I press the shortcut</b></summary>

Another app may already use that key combination; Claws says so when it starts or when you change the shortcut. Choose a different one in **Settings → Quick Access**. The shortcut only works while Claws is running, so keep **Keep running when closed** on, and turn on **Open at login** to have it from the moment you sign in.
</details>

<details>
<summary><b>Claws is still running after I closed its window</b></summary>

That is on purpose, so downloads continue and Quick Open keeps working. Press <kbd>Ctrl</kbd>+<kbd>Q</kbd>, or choose **Quit Claws** from its icon in the notification area. To make closing the window quit, turn off **Settings → Quick Access → Keep running when closed**.
</details>

<details>
<summary><b>A download says the file already exists</b></summary>

Claws never replaces a file that is already in the output folder. Getting the same video again, even in another quality, produces the same name. Move or rename the earlier file, then press **Try Again**: the download was kept, so only the move is left. To keep several qualities side by side, add the quality to the filename template, for example `%(title).150B [%(id)s] %(height)sp.%(ext)s`.
</details>

<details>
<summary><b>"Not enough free space"</b></summary>

Large videos need room for the downloaded streams and for the merged file while both exist, plus a safety margin. Space other downloads have already used is not counted twice. The check runs when a transfer starts and on processing notifications (merging, embedding); the latter are best effort, and a late low-space notification does not reject an otherwise finished result. Free up space on the drive named in the message, or choose another output folder for new downloads, then press **Try Again**: what was already downloaded is kept.
</details>

<details>
<summary><b>"Claws could not save its download queue" or "…could not add it to your Library"</b></summary>

Claws keeps its records in `%APPDATA%\dev.claws.downloader\`. When it cannot write there (a full drive, or a folder it may not change), it does not start a download, delete files or forget a finished one without a saved record. Nothing is lost:
- A download that could not start stays in the queue with its folder; press **Try Again** once the problem is fixed.
- A finished file that could not be added to the Library is already in the output folder. **Try Again** only adds it to the Library; nothing is downloaded again. If Claws is closed first, it finishes this the next time it starts.
</details>

<details>
<summary><b>What is the hidden <code>.claws-work</code> folder?</b></summary>

Unfinished downloads live there, one folder per download, until they are finished or cancelled. Paused and failed downloads keep theirs so they can resume. Trying a cancelled download again starts it over in a new folder. Leave it alone while Claws is running; a folder that could not be deleted because a file was in use is removed the next time Claws starts.
</details>

## How it works

Claws is a [Tauri 2](https://tauri.app) app. The Rust backend drives two external tools and installs a third:

![yt-dlp](https://img.shields.io/badge/yt--dlp-FF0000?logo=youtube&logoColor=white)
![FFmpeg](https://img.shields.io/badge/FFmpeg-007808?logo=ffmpeg&logoColor=white)
![Deno](https://img.shields.io/badge/Deno-000000?logo=deno&logoColor=white)

[yt-dlp](https://github.com/yt-dlp/yt-dlp) reads each site and fetches the streams, [ffmpeg](https://ffmpeg.org) merges, converts and embeds, and [Deno](https://deno.com) is the JavaScript runtime yt-dlp needs for recent YouTube extraction.

```
┌─────────────────── Claws window (WebView2, TypeScript + CSS) ────────────────────┐
│  Download · Queue · Library · Settings                                           │
└─────────────┬────────────────────────────────────────────────▲───────────────────┘
     commands │                                         events │ progress, state
┌─────────────▼────────────────────────────────────────────────┴───────────────────┐
│  Rust backend: link lookup & caches · queue & concurrency · history              │
│                tool install & update · window material · tray & Quick Open       │
└─────────────┬────────────────────────────────────────────────────────────────────┘
       spawns │  argument arrays, never a shell
    ┌─────────▼─────────┐        ┌──────────────┐
    │       yt-dlp      │──────▶ │    ffmpeg    │
    └───────────────────┘        └──────────────┘
```

**Reading a link** runs `yt-dlp --dump-single-json --flat-playlist` and turns the result into a preview, the available resolutions (kept per shorter side, because that is what yt-dlp's resolution cap limits), subtitle and caption tracks, and playlist entries. A pasted list is looked up three links at a time, and the lookup cache has a size budget. **Size previews** re-run Claws' own format selection against the cached lookup with `yt-dlp --simulate` and size the peak with the same formula the disk-space check uses; they are display only.

**Downloading** runs yt-dlp with progress markers the backend parses live (transfer progress on stdout, post-processing on stderr). Each job gets its own process tree, so pause and cancel stop yt-dlp and its ffmpeg children together. `max_concurrent` counts transferring jobs; up to `max_processing` jobs in final processing hold no transfer slot, and each one beyond that does, because a merge already running inside yt-dlp cannot be made to wait. If a download fails on a network error, yt-dlp retries up to 10 times with waits growing from 1 to 30 seconds, and a stream missing a piece fails rather than saving a file with a gap.

**Transcripts** run yt-dlp with `--skip-download` and exactly one caption track. Claws parses the VTT or SRT file itself, strips markup and the repeats rolling automatic captions carry while keeping genuinely repeated speech, and publishes the text through the same work folder and journal as a video.

**Work folders.** yt-dlp writes only inside the job's own `<output folder>\.claws-work\<id>\`. On success Claws moves the finished file and any separate subtitles into the output folder without replacing anything; if the name is taken, the download is kept and **Try Again** only finishes the move. Cancelling deletes the work folder, which Claws identifies by a marker inside it.

## Performance

Measured on the development machine (SATA SSD, a connection of about 500 Mbit/s) during the original tuning pass:

| Step | Before tuning | After |
| --- | --- | --- |
| Press Download → first progress | ~4.7 s | **~0.8 s** |
| yt-dlp start-up | ~1.4 s | **~0.4 s** |
| Work before the first frame | ~500 ms | **~2 ms** |

Most of that came from not doing things twice. A lookup's result is kept for 20 minutes and the download starts from it instead of reading the page again, retrying once with a fresh read if the site's links have expired. yt-dlp is installed as its folder build, which starts about three times faster than the single-file build. The window draws immediately and tool checks run in the background; nothing checks for updates at launch.

Fragmented streams (HLS and DASH) download 8 fragments at a time, which helps on high-latency links. Counts of 1, 4, 8 and 16 were compared and 8 stayed ([fragment count](docs/performance/fragment-count.md)). The queue repaints only the row whose progress changed and adds a whole playlist in one step; with 500 queued jobs and 8 downloading, layout and style work over the same stretch of progress fell from about 730 ms to 63 ms ([queue UI](docs/performance/queue-ui.md)). *Embed metadata* costs one extra rewrite of each file after merging, so turn it off, or use *Fast Save*, for the quickest finish on long videos.

Two ideas were measured and dropped because they did not reach the 1.5× gate on the links tested: yt-dlp's `formats=dashy` and an aria2c transport. A throttled source has not been tested. See the [real-source benchmark](docs/performance/real-source-benchmark.md) and [speed diagnosis](docs/performance/speed-diagnosis.md).

## Privacy and security

Claws sends no telemetry and makes no account calls. Network traffic goes to the sites you download from (including the servers their artwork comes from), to GitHub when yt-dlp, ffmpeg or Deno is installed or updated, to `sponsor.ajay.app` through yt-dlp when SponsorBlock is on, and to public Spotify and YouTube pages for track matching. Notifications are local Windows notifications.

- **Verified tools.** Every tool archive is checked against the SHA-256 its project publishes and discarded on a mismatch. Unpacking is defensive: names that could land outside the tool folder are skipped, encrypted entries and links are refused, and entry counts and bytes written are capped whatever size an entry claims.
- **Untrusted input.** Only `http` and `https` links are accepted, and they reach yt-dlp as plain arguments after `--`, never through a shell. Anything that ends up on the command line (browser names, cookie files, filename templates, SponsorBlock categories) is validated first, and text from sites is rendered as text, never HTML.
- **Least privilege.** The interface can only open files and folders the app itself recorded, and the window runs under a strict Content Security Policy.
- Cookies are used only while a source is set in **Settings → Sign-in**. yt-dlp reads them on each lookup or download and sends each cookie only to the site it belongs to.

## Fonts

**Salted Flutters Polaris** is the interface font and is bundled with Claws. The four web fonts are committed in `src/assets/fonts/` and the unmodified license is in `public/fonts/LICENSE.txt`. Vite fingerprints each font into `dist/assets/`, and the private CSS alias `Claws Polaris` prevents substitution by an older installed font. Builds verify approved font hashes before building and the packaged fonts afterward, without needing Python.

Python is only needed to regenerate the web fonts from the four source files and license in `assets/fonts/polaris/`:

```powershell
pip install "fonttools[woff]"
python scripts/build-fonts.py
```

The script builds the four weights the interface uses (Regular, Medium, Semibold, Bold), subset to Latin, Greek, Cyrillic and common punctuation while keeping tabular figures, kerning and ligatures, then updates `scripts/font-integrity.json`. Run `npm run test:fonts` for packaging checks and `python scripts/test_font_metrics.py` for glyph metrics (it needs the `brotli` module).

## Building from source

**Prerequisites:** [Rust](https://rustup.rs) 1.85 or newer, [Node.js](https://nodejs.org) 20 or newer, and WebView2.

```powershell
git clone https://github.com/ppolkadot/Claws.git
cd Claws
npm ci

npm run tauri dev                          # run with hot reload
npm run tauri build -- --bundles nsis      # installer in src-tauri/target/release/bundle/nsis/
```

CI runs the checks below on Windows for every push and pull request. To run them yourself:

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
```

| Command | What it does |
| --- | --- |
| `npm run dev` | Runs the interface in a browser with labelled synthetic data. Open it with `?setup=fresh` to see first-run setup. |
| `npm run build` | Verifies font integrity, type-checks and builds the frontend, then checks the packaged fonts. |
| `npm run test:logic` | Pure frontend logic: link parsing, size estimates, resolutions, captions, setup and remembered choices. |
| `npm run test:queue` | Queue event ordering, status transitions and the summary line. |
| `npm run test:motion` | Progress gliding, row ordering, completion feedback and the file-drag gesture. |
| `npm run test:spotify` | Spotify matching and its mounted view. |
| `cargo test` | The backend tests. Those that drive a real yt-dlp or FFmpeg against local fixtures need Python and the tool (`CLAWS_TEST_YTDLP`, `CLAWS_TEST_FFMPEG_DIR`) and skip without them. |

`scripts/dev/` holds the tools used to test the real app: a localhost fixture server with controlled faults (`download-fixtures.py`); benchmarks for transfers, processing, the queue, a 500-link batch, motion, the shell and Spotify (the browser-driven ones run headless Edge against the dev mock); a DevTools driver; and a glass capture script that refuses to capture when another window overlaps Claws. See the [developer handoff](docs/HANDOFF.md) for usage and the [documentation index](docs/README.md) for current guidance and historical records.

## Project structure

```
claws/
├── src/                        Frontend (TypeScript + CSS, no framework)
│   ├── views/                  Download (options, playlist, batch), Queue, Library, Settings,
│   │                           setup panel, Spotify match, window shell
│   ├── ui/                     Controls, layout, icons, spring motion, progress and row-order
│   │                           logic, the file-drag gesture
│   ├── store.ts, api.ts        App state and the typed backend calls
│   ├── queue-events.ts         Revisioned job table; queue-summary.ts the summary line
│   ├── download-input.ts       Link parsing and the three-at-a-time lookup scheduler
│   ├── download-estimates.ts   Size and free-space wording
│   ├── setup.ts, last-choices.ts, library-search.ts   Setup steps, remembered choices, Library sorting
│   └── styles/                 Materials and tokens, components, screens, fonts
├── src-tauri/src/              Backend (Rust)
│   ├── downloader.rs           Queue, slots, pause/resume/cancel, bulk actions, publication
│   ├── download_policy.rs      Frozen job settings, the yt-dlp command line, failure kinds
│   ├── download_files.rs       Work folders, safe publication and cleanup
│   ├── queue_writer.rs, store.rs, history.rs, history_mutation.rs   Durable queue, library, recovery
│   ├── transfer_progress.rs    Byte-weighted progress and time left
│   ├── disk_budget.rs, estimate.rs   Free-space estimates and the size preview
│   ├── probe.rs, links.rs      Reading links; saved details for instant lookups
│   ├── transcript.rs           Caption parsing and text/SRT/VTT output
│   ├── spotify.rs, spotify_match.rs   Spotify identity and YouTube ranking
│   ├── tools.rs                Installing and updating yt-dlp, ffmpeg and Deno
│   ├── desktop.rs, window.rs   Quick Open, tray, notifications; glass and materials
│   ├── file_drag.rs            Native outgoing file drag
│   └── commands.rs             Everything the interface can call
├── scripts/                    Font build, tests (tests/), dev tools (dev/)
├── docs/                       Handoff, status, release records, performance evidence
├── .github/workflows/ci.yml    Windows CI
├── PRODUCT.md                  Product brief
└── DESIGN.md                   Design system
```

## Contributing

Issues and pull requests are welcome. Before opening a PR:

- run the checks in [Building from source](#building-from-source) and keep them clean; CI runs the same ones;
- follow the existing style: plain TypeScript and CSS, with comments only where they explain *why*;
- keep visual work consistent with [`DESIGN.md`](DESIGN.md), and keep the contracts in the [handoff](docs/HANDOFF.md) intact.

## Acknowledgements and license

Claws exists because of [yt-dlp](https://github.com/yt-dlp/yt-dlp), [FFmpeg](https://ffmpeg.org), the [yt-dlp FFmpeg builds](https://github.com/yt-dlp/FFmpeg-Builds) and [Deno](https://deno.com), and is built with [Tauri](https://tauri.app). SponsorBlock data comes from the [SponsorBlock](https://sponsor.ajay.app) community.

Released under the [MIT License](LICENSE).

Claws is a tool. Only download content you have the right to save, and respect the terms of the sites you use.
