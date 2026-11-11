<div align="center">

# Claws

**A local video downloader for Windows, designed like an Apple app.**

Paste a link, choose a quality, and the file lands on your disk. Claws sits on clear desktop glass, moves on real spring physics, and does all its work on your own PC.

[![Release](https://img.shields.io/github/v/release/ppolkadot/Claws?color=34C759)](https://github.com/ppolkadot/Claws/releases/latest)
[![CI](https://github.com/ppolkadot/Claws/actions/workflows/ci.yml/badge.svg)](https://github.com/ppolkadot/Claws/actions/workflows/ci.yml)
![Platform](https://img.shields.io/badge/platform-Windows%2011-0078D4?logo=windows11&logoColor=white)
![Tauri](https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white)
![Rust](https://img.shields.io/badge/Rust-1.85%2B-B7410E?logo=rust&logoColor=white)
![TypeScript](https://img.shields.io/badge/TypeScript-5-3178C6?logo=typescript&logoColor=white)
![License](https://img.shields.io/badge/license-MIT-34C759)

[**Download Claws 1.0.9**](https://github.com/ppolkadot/Claws/releases/tag/v1.0.9) · [Documentation](docs/README.md) · [Release notes](docs/releases/1.0.9.md)

</div>

---

## Contents

- [Quick start](#quick-start)
- [Features](#features)
- [Installation](#installation)
- [Using Claws](#using-claws)
- [Settings](#settings)
- [How it works](#how-it-works)
- [Performance](#performance)
- [Privacy and security](#privacy-and-security)
- [Fonts](#fonts)
- [Building from source](#building-from-source)
- [Project structure](#project-structure)
- [Troubleshooting](#troubleshooting)
- [Contributing](#contributing)
- [Acknowledgements](#acknowledgements)
- [License](#license)

## Quick start

1. Download `Claws_1.0.9_x64-setup.exe` from the [latest release](https://github.com/ppolkadot/Claws/releases/latest) and run it.
2. Press **Set Up Everything** on first launch to install yt-dlp, ffmpeg and Deno.
3. Paste a link with <kbd>Ctrl</kbd>+<kbd>V</kbd>, pick a quality, press **Download**.

## Features

### Downloading

- **Paste and go.** Paste a link, or press <kbd>Ctrl</kbd>+<kbd>V</kbd> anywhere in the window, and Claws reads the title, artwork, duration and the qualities on offer.
- **Many sites.** Claws runs on [yt-dlp](https://github.com/yt-dlp/yt-dlp), so it works with any site yt-dlp supports. It has been tested against YouTube; other sites rely on yt-dlp's own support for them.
- **Format picker.** Best quality, or a cap at any resolution the source actually offers, from 144p up to 8K. Choices come from the source's own formats and carry exact dimensions (for example `4320p · 7680×4320 · 8K`); portrait video is handled by its shorter side. Claws never upscales. Audio only in MP3, M4A, Opus, FLAC, WAV or the original stream. Subtitles as separate `.srt` files or embedded.
- **Compatible or Original.** *Compatible* prefers H.264 and AAC in MP4 when the site offers them. *Original* keeps the site's best streams without re-encoding, as MP4 when the container can hold them and MKV otherwise. Some editors cannot open every codec, and Claws does not claim otherwise.
- **Size preview.** Before you download, Claws shows the expected download size and, separately, the free space it needs while working, computed from the same streams that will be downloaded. An unknown size is shown as unknown, never as zero, and estimates say "about". A preview never replaces the check made when a download starts.
- **Paste many links.** One link per line, up to 500, looked up three at a time with a status per row. A failed link can be retried alone. Nothing starts until you press Download, and links already queued are reported rather than added twice.
- **Transcripts.** Save a video's own captions as text (plain or timestamped), SRT or VTT without downloading the video. Human and automatic captions are listed separately. There is no speech recognition: automatic captions are the site's and can contain errors.
- **Playlists and channels.** Pick exactly which videos to download. **Playlist folder** saves them into a folder named after the playlist, each numbered by its place in it (`My Playlist\03 - Title [id].mp4`).
- **Chapters and SponsorBlock.** With *Embed metadata* on, a video's chapters are written into the file. For YouTube, an optional per-download SponsorBlock choice (off by default) either **marks** segments as chapters or **removes** them. It asks `sponsor.ajay.app` about the video, only when turned on. Removing rewrites the file, can leave a brief glitch at a cut, and makes the file smaller than the size preview. Fast Save skips it.
- **Fast Save.** A per-download switch that skips metadata, chapters, cover art, SponsorBlock and embedded subtitles, so a large file is not rewritten after it downloads. Conversion or merging the format needs still happens.
- **Audio without needless conversion.** With *Prefer matching source audio* on, a stream already in the chosen format is copied instead of converted.
- **Spotify track matching.** Paste one `open.spotify.com/track/...` link (or a `spotify.link` short link that resolves through supported Spotify hosts). Claws reads the public song details and searches YouTube; you pick a recording, press **Use this match** to check its audio, then choose a format and **Download**. Nothing is selected or downloaded automatically. The file is YouTube audio, the recording may differ from the Spotify version, and Queue and Library label it **Source: YouTube match**. No Spotify account is needed; albums, playlists and batches are not supported.
- **Drag into an editor.** A finished download has a *Drag file* handle that starts a real Windows file drag (copy only) into Explorer or an editor's media bin. The file stays in your Library. Press Enter on the handle to show it in its folder instead.

### Managing

- **Queue.** Several downloads at once with live progress, speed and time left.
  - Progress is weighted by bytes across the video and audio streams. An estimated size says "about"; an unknown size shows bytes downloaded instead of a guessed percentage.
  - Pause, resume, cancel and retry, one at a time or all at once (**Pause All**, **Resume All**, **Retry Failed**). **Move to top** puts a waiting download first in line, and the order survives a restart.
  - A summary line counts what is downloading, processing, waiting, paused and failed, with the combined speed and, only when every running download reports one, the time left.
  - While a file is processed the row names the step (merging, converting audio, applying SponsorBlock) when yt-dlp reports it. A failed download shows its full message with a **Copy** button.
  - The taskbar button shows overall progress, and a Windows notification says when a download finishes or fails while Claws is hidden or behind another window.
  - Unfinished downloads survive a restart as paused, and keep the folder and file name they started with.
- **Processing alongside.** A download that has finished transferring and is merging or converting no longer holds up the next transfer. Up to two such jobs run beside the transfer limit, so with the defaults up to five jobs can be active at once. An optional per-download **speed limit** is off by default.
- **Safe with your files.** Each download works in its own hidden folder until it is finished. Cancelling deletes that folder and nothing else. A finished file never replaces one that already exists.
- **Disk space check.** Before a large download starts, Claws estimates the space it needs, including the extra copy while merging, and watches space when processing starts. Processing checks are best effort; if stopped, owned files are kept for retry.
- **Library.** Finished downloads with artwork and search. Sort by date, title or size, and show only video, audio or transcripts. The type, audio format, quality cap, preset and Fast Save of your last download are offered again for the next link.
- **Sign-in.** Use cookies from your browser, or a `cookies.txt` file, for sites that need you to be logged in.
- **Instant repeat lookups.** Links you have downloaded keep their details, so looking one up again shows it at once while Claws checks the site for changes.

### Quick access

- **Quick Open.** Press <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>D</kbd> in any app to bring Claws forward; a link on the clipboard is looked up straight away. Press it again to put Claws away. The shortcut can be changed or turned off.
- **Keeps running.** Closing the window leaves Claws in the notification area, so downloads continue. <kbd>Ctrl</kbd>+<kbd>Q</kbd> or the icon's menu quits.
- **Open at login.** Optional: Claws starts quietly in the notification area when you sign in.

### Interface

- **Desktop glass.** The desktop shows through the window, blurred by Windows itself, with Apple-style materials on top and no painted background.
- **Apple conventions.** A collapsible floating sidebar, a toolbar that turns to frosted glass as content scrolls under it, macOS-style window controls and inset grouped settings.
- **Spring motion.** Controls compress and spring back, the sidebar selection glides, sheets and toasts arrive on a spring, Library cards glide to their new places, and a looked-up link settles into a preview with no layout shift. Motion never stands in for real state, large lists are not animated, and everything respects *Reduce motion*.
- **Light and dark.** Follows Windows or set either one, with adjustable transparency and an opaque Solid mode.

## Installation

### From a release

1. Download `Claws_<version>_x64-setup.exe` from the [Releases](https://github.com/ppolkadot/Claws/releases) page. The 1.0.9 installer's SHA-256 is `3ce6d049d578120fb0b156a5dd1eef9a0a48757d7dfd11002f581941e63b2283`.
2. Run it. Releases are not code-signed yet, so Windows SmartScreen may warn on first run; choose **More info → Run anyway**.
3. On first launch the Download page shows **Set up Claws** with the tools that are missing: yt-dlp, ffmpeg and a JavaScript runtime (Deno). Press **Set Up Everything** and Claws downloads each into its own folder from its project's GitHub releases and verifies it against the published SHA-256 before using it. Nothing is downloaded until you press the button. A tool already on your `PATH` counts as ready, and ffmpeg and Deno can be skipped with **Not Now**.

The application identifier and data location are unchanged from earlier versions, so an upgrade keeps your settings, Library and queue. Queue schema 3 is unchanged: an older 1.0.5 cannot run these jobs, but this build can recover an unambiguous queue it preserved. Earlier installers and the 1.0.0 Beta release are kept as historical artifacts.

### Requirements

| | |
| --- | --- |
| OS | Windows 11. Windows 10 has not been tested. |
| Runtime | Microsoft Edge WebView2, included with Windows 11. |
| ffmpeg | Merges HD video with audio and converts audio. Claws uses the copy on your `PATH`, or installs one (first-run setup or **Settings → Tools**). |
| JavaScript runtime | Recent YouTube extraction needs Deno or Node.js. Claws installs Deno for you, or uses either one found on your `PATH`; **Settings → Tools** shows which one it uses. |

## Using Claws

1. **Paste a link** into the field on the Download page and press **Get Info**, or press <kbd>Ctrl</kbd>+<kbd>V</kbd> anywhere. For several unrelated links, paste one per line and press <kbd>Ctrl</kbd>+<kbd>Enter</kbd>.
2. **Choose** Video, Audio Only or Transcript; a quality and Compatible or Original; and optionally subtitles. For a playlist or batch, tick the items you want. **Estimate Sizes** adds up the selected items on request.
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

Window materials: **Glass** (default) shows your desktop through the window, blurred. **Acrylic** and **Mica** are the Windows system materials, which Windows flattens while the window is in the background. **Solid** turns the material off.

Changes to the output folder, the filename template, *Embed metadata* and *Prefer matching source audio* apply to downloads that have not started yet. A download that has started, even a paused one, keeps the choices it started with. The speed limit is read each time a download starts or resumes; a limited download fetches one fragment at a time because yt-dlp applies the limit to every fragment connection.

Settings, the Library, the unfinished queue, the downloaded tools and the caches live in `%APPDATA%\dev.claws.downloader\`.

## How it works

Claws is a [Tauri 2](https://tauri.app) app. The backend is Rust and drives two well-known tools:

- [**yt-dlp**](https://github.com/yt-dlp/yt-dlp) reads each site and fetches the media streams.
- [**ffmpeg**](https://ffmpeg.org) merges video with audio, converts audio and embeds metadata.

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

- **Reading a link** runs `yt-dlp --dump-single-json --flat-playlist` and turns the result into a preview, the available resolutions (kept per shorter side, because that is what yt-dlp's resolution cap limits), subtitle and caption tracks, and playlist entries. A pasted list is looked up three links at a time, and the lookup cache has a size budget.
- **Size previews** re-run Claws' own format selection (the one the download uses) against the cached lookup with `yt-dlp --simulate`, and size the peak with the same formula the disk-space check uses. They are display only.
- **Transcripts** run yt-dlp with `--skip-download` and exactly one caption track. Claws then parses the VTT or SRT file itself, removes markup and the repeats that rolling automatic captions carry, keeps genuinely repeated speech, and publishes the text through the same work folder and journal as a video.
- **Downloading** runs yt-dlp with progress markers the backend parses live (transfer progress on stdout, post-processing on stderr). Each job runs in its own process tree, so pause and cancel stop yt-dlp and its ffmpeg children together.
- **Slots.** `max_concurrent` counts transferring jobs. Up to `max_processing` jobs in final processing hold no transfer slot; each one beyond that does, because a merge already running inside yt-dlp cannot be made to wait.
- **Work folders.** yt-dlp writes only inside the job's own folder, `<output folder>\.claws-work\<id>\`. When the download succeeds, Claws moves the finished file, and any separate subtitles, into the output folder without replacing an existing file. If the name is taken, the download is kept and **Try Again** only finishes the move. Cancelling deletes the work folder, which Claws identifies by a marker inside it.
- **Network trouble.** yt-dlp retries up to 10 times, with waits that grow from 1 to 30 seconds. A stream that is missing a piece fails instead of saving a file with a gap in it.
- **The glass** uses the Windows composition API to blur the desktop behind the transparent window. The interface adds only tint layers, a bright edge and depth on top of that blur.

## Performance

Measured on the development machine (SATA SSD, a connection of about 500 Mbit/s) during the original tuning pass:

| Step | Before tuning | After |
| --- | --- | --- |
| Press Download → first progress | ~4.7 s | **~0.8 s** |
| yt-dlp start-up | ~1.4 s | **~0.4 s** |
| Work before the first frame | ~500 ms | **~2 ms** |

What makes the difference:
- **No second read.** Reading a link keeps its result for 20 minutes, and the download starts from it instead of reading the page again. If the site's links have expired, Claws retries once with a fresh read.
- **Faster start-up.** yt-dlp is installed as its folder build, which starts about three times faster than the single-file build.
- **Parallel fragments.** Fragmented streams (HLS and DASH) download 8 fragments at a time, which helps on high-latency links. Counts of 1, 4, 8 and 16 were compared and 8 stayed ([fragment count](docs/performance/fragment-count.md)).
- **No waiting at launch.** The window draws immediately, and tool checks run in the background. Nothing checks for updates at launch.
- **A responsive queue.** Progress updates repaint only their own row, and a whole playlist is added in one step. With 500 queued jobs and 8 downloading, layout and style work over the same stretch of progress fell from about 730 ms to 63 ms ([queue UI](docs/performance/queue-ui.md)).
- **Embed metadata** costs one extra rewrite of each file after merging. Turn it off, or use **Fast Save**, for the fastest finish on long videos.

Two ideas were measured and not built, because they did not reach the 1.5× gate on the links tested: yt-dlp's `formats=dashy` and an aria2c transport. A throttled source is still untested; see the [real-source benchmark](docs/performance/real-source-benchmark.md) and [speed diagnosis](docs/performance/speed-diagnosis.md).

## Privacy and security

- **Local only.** Claws makes no telemetry, analytics or account calls. Network traffic goes to the sites you download from (including the servers their artwork comes from), to GitHub when yt-dlp, ffmpeg or Deno is installed or updated, to `sponsor.ajay.app` through yt-dlp when you turn SponsorBlock on, and to public Spotify and YouTube pages when you use track matching. Notifications are local Windows notifications.
- **Verified tools.** Every tool archive is checked against the SHA-256 its project publishes and discarded on a mismatch. Archives are unpacked defensively: names that could land outside the tool folder are skipped, encrypted entries and links are refused, and entry counts and bytes written are capped whatever size an entry claims.
- **Untrusted input stays contained.** Only `http` and `https` links are accepted. They reach yt-dlp as plain arguments after `--`, never through a shell. Settings that end up on the command line (browser names, cookie files, filename templates, SponsorBlock categories) are validated first, and text from sites is rendered as text, never as HTML.
- **Least privilege.** The interface can only open files and folders the app itself recorded, and the window runs under a strict Content Security Policy.
- **Cookies** are used only while a cookie source is set in **Settings → Sign-in**: yt-dlp reads them each time it looks up or downloads a link and sends each cookie only to the site it belongs to.

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

## Contributing

Issues and pull requests are welcome. Before opening a PR:

- run the checks in [Building from source](#building-from-source) and keep them clean; CI runs the same ones;
- follow the existing style: plain TypeScript and CSS, with comments only where they explain *why*;
- keep visual work consistent with [`DESIGN.md`](DESIGN.md), and keep the contracts in the [handoff](docs/HANDOFF.md) intact.

## Acknowledgements

Claws exists because of [yt-dlp](https://github.com/yt-dlp/yt-dlp), [FFmpeg](https://ffmpeg.org), the [yt-dlp FFmpeg builds](https://github.com/yt-dlp/FFmpeg-Builds) and [Deno](https://deno.com), and is built with [Tauri](https://tauri.app). SponsorBlock data comes from the [SponsorBlock](https://sponsor.ajay.app) community.

## License

Released under the [MIT License](LICENSE).

Claws is a tool. Only download content you have the right to save, and respect the terms of the sites you use.
