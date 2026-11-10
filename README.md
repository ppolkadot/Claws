<div align="center">

# Claws

**A local video downloader for Windows, designed like an Apple app.**

Paste a link, choose a quality, and the file lands on your disk. Claws sits on clear desktop glass, moves on real spring physics, and does all its work on your own PC.

![Platform](https://img.shields.io/badge/platform-Windows%2011-0078D4?logo=windows11&logoColor=white)
![Tauri](https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white)
![Rust](https://img.shields.io/badge/Rust-1.85%2B-B7410E?logo=rust&logoColor=white)
![TypeScript](https://img.shields.io/badge/TypeScript-5-3178C6?logo=typescript&logoColor=white)
![License](https://img.shields.io/badge/license-MIT-34C759)

</div>

---

## Contents

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

## Features

**Downloading**
- **Paste and go.** Paste a link, or press <kbd>Ctrl</kbd>+<kbd>V</kbd> anywhere in the window, and Claws reads the title, artwork, duration and the qualities on offer.
- **Many sites.** Claws runs on [yt-dlp](https://github.com/yt-dlp/yt-dlp), so it works with any site yt-dlp supports, which is well over a thousand. It has been tested against YouTube; other sites rely on yt-dlp's support for them.
- **Format picker.** Best quality or a cap at any resolution the source actually offers, from 144p up to 8K when a site provides it. Choices come from the source's own formats, are labelled with their exact size (for example `4320p · 7680×4320 · 8K`), and portrait video is handled by its shorter side. Claws never upscales. Audio only in MP3, M4A, Opus, FLAC, WAV or the original stream. Subtitles as separate `.srt` files or embedded.
- **Compatible or Original.** *Compatible* prefers H.264 video and AAC audio in MP4 when the site offers them, and otherwise uses another codec. *Original* keeps the site's best streams without re-encoding, as MP4 when the container can hold them and MKV otherwise. Some editors cannot open every codec, and Claws does not claim otherwise.
- **Size preview.** Before you download, Claws shows the expected download size and, separately, the free space it will need while working. It sizes the same streams that will be downloaded. An unknown size is shown as unknown, never as zero, and approximate sizes say "about". The check made when a download starts is unchanged; a preview never replaces it.
- **Paste many links.** Put one link per line (up to 500) and Claws looks them up three at a time, with a status for each row. A link that fails can be retried on its own without losing the others. Nothing starts until you press Download, and links already in the queue are reported rather than added twice.
- **Transcripts.** Choose *Transcript* to save a video's own captions as text (plain, or with timestamps), SRT or VTT, without downloading the video. Human and automatic captions are listed separately. Claws does no speech recognition: automatic captions are the site's and can contain errors, and a video with no captions simply has no transcript.
- **Drag into an editor.** A finished download in the Library or the Queue has a *Drag file* handle that starts a real Windows file drag (copy only), so you can drop the file into Explorer or an editor's media bin. The file stays in your Library. Press Enter on the handle to show the file in its folder instead.
- **Fast Save.** An optional per-download switch. It skips writing metadata into the file, skips embedding the cover art (MP3 and M4A audio) and keeps subtitles as separate `.srt` files, so a large file is not rewritten after it downloads. Conversion or merging that the chosen format needs still happens. The video and audio are the same either way.
- **Audio without needless conversion.** With *Prefer matching source audio* on, an audio stream already in the chosen format is saved by copying it instead of converting it.
- **Spotify track matching.** Paste a single `https://open.spotify.com/track/...` song link (localized track links also work), or a `https://spotify.link/...` short link that resolves through supported Spotify hosts. Claws reads public song details and searches YouTube. Select a recording, press **Use this match** to check its audio, then choose an audio format and press **Download**. Nothing is selected or downloaded automatically. The Queue and Library label these files **Source: YouTube match**. The file is YouTube audio; the recording/version may differ, and FLAC or WAV conversion cannot improve its original quality. No Spotify account or developer credentials are needed. If public metadata fails, enter the artist/title to search manually. Albums, playlists, Spotify URIs and batch matching are not supported; open each track individually. Short links that redirect outside the supported hosts require the full track URL.
- **Playlists and channels.** Pick exactly which videos to download.

**Managing**
- **Queue.** Several downloads at once, with live progress, speed and time left.
  - Progress is weighted by bytes across the video and audio streams.
  - When a size is only an estimate it says "about"; when it is unknown, the row shows bytes downloaded instead of a guessed percentage.
  - Pause, resume, cancel and retry.
  - Unfinished downloads survive a restart as paused.
  - A paused download keeps the folder and file name it started with, even if you change them in Settings.
- **Safe with your files.** Each download works in its own hidden folder until it is finished. Cancelling deletes that folder and nothing else. A finished file never replaces a file that already exists.
- **Disk space check.** Before a large download starts, Claws estimates the space it will need, including the extra copy while merging, and monitors space when processing starts. Processing checks are best effort; if stopped, owned files are kept for retry.
- **Library.** Finished downloads with artwork and search. Open them or show them in their folder.
- **Sign-in.** Use cookies from your browser, or a `cookies.txt` file, for sites that need you to be logged in.
- **Instant repeat lookups.** Links you have downloaded keep their details, so looking one up again shows it at once while Claws checks the site for changes. Pick any item under **Recent** to see it again.

**Quick access**
- **Quick Open.** Press <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>D</kbd> in any app to bring Claws forward; a link on the clipboard is looked up straight away. Press it again to put Claws away. The shortcut can be changed or turned off.
- **Keeps running.** Like a Mac app, closing the window leaves Claws in the notification area, so downloads continue. <kbd>Ctrl</kbd>+<kbd>Q</kbd> or the icon's menu quits.
- **Open at login.** Optional: Claws starts quietly in the notification area when you sign in.

**Interface**
- **Desktop glass.** The desktop shows through the window, blurred by Windows itself. Apple-style materials sit on top, with no painted background.
- **Apple conventions.** A floating sidebar you can collapse, a toolbar that turns to frosted glass as content scrolls under it, macOS window controls, and inset grouped settings.
- **Spring motion.** Controls compress and spring back, switches stretch while held, the sidebar selection glides between items, and sheets and toasts arrive on a spring. A looked-up link settles into its preview, the Video / Audio / Transcript control slides, new and removed queue rows ease in and out, progress glides forward and snaps whenever the state changes, and a finished download gets a short check. None of it stands in for real state, large batches are not animated, and all of it respects *Reduce motion*.
- **Light and dark.** Follows Windows or set either one, with adjustable transparency and an opaque Solid mode.

## Installation

### From a release

1. Download `Claws_<version>_x64-setup.exe` from the [Releases](https://github.com/ppolkadot/Claws/releases) page.
2. Run it. Releases are not code-signed yet, so Windows SmartScreen may warn on first run; choose **More info → Run anyway**.
3. On first launch Claws downloads yt-dlp into its own folder and verifies it against the checksum yt-dlp publishes. This happens once and takes a few seconds.

**Claws 1.0.8**: [GitHub release](https://github.com/ppolkadot/Claws/releases/tag/v1.0.8), [Windows installer](https://github.com/ppolkadot/Claws/releases/download/v1.0.8/Claws_1.0.8_x64-setup.exe), [SHA-256 checksum](installers/Claws_1.0.8_x64-setup.exe.sha256) and [release notes](docs/releases/1.0.8.md). It adds Spotify track matching through explicitly confirmed YouTube recordings and includes the Unicode-path correction from 1.0.7.

The application identifier and data location remain unchanged. Queue schema remains 3: old 1.0.5 cannot execute these jobs, but this build can recover an unambiguous queue it preserved. Live clean-install/upgrade and editor-drag acceptance remain outstanding. Earlier installers and the 1.0.0 Beta release are retained as historical artifacts.

### Requirements

| | |
| --- | --- |
| OS | Windows 11. Windows 10 has not been tested. |
| Runtime | Microsoft Edge WebView2, included with Windows 11. |
| ffmpeg | Needed to merge HD video with audio and to convert audio. Claws uses the copy on your `PATH`, or installs one from **Settings → Tools**. |
| JavaScript runtime | Recent YouTube extraction needs Deno or Node.js. Claws finds either on your `PATH`; **Settings → Tools** shows which one it uses. |

## Using Claws

1. **Paste a link** into the field on the Download page and press **Get Info**, or just press <kbd>Ctrl</kbd>+<kbd>V</kbd> anywhere. To download several unrelated links, paste them one per line and press <kbd>Ctrl</kbd>+<kbd>Enter</kbd> (or **Look Up**).
2. **Choose** Video, Audio Only or Transcript; a quality and Compatible or Original; and optionally subtitles. For a playlist or a batch, tick the items you want. **Estimate Sizes** adds up the selected items on request.
3. **Press Download.** The job joins the queue and starts immediately. Finished files appear in the Library, where the *Drag file* handle drops them into Explorer or an editor.

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
| **Downloads** | Output folder (default `Downloads\Claws`), filename template (a yt-dlp output template), simultaneous downloads (1–8), processing alongside (1–8, default 2: downloads that may be merging or converting without holding up the next transfer), speed limit (per download, off by default; applies from the next download that starts or resumes), embed metadata, prefer matching source audio |
| **Quick Access** | Quick Open shortcut (Ctrl+Alt+D, Ctrl+Alt+Space, Ctrl+Shift+Space or off), keep running when closed, open at login |
| **Appearance** | Theme (System, Light, Dark), window material (Glass, Acrylic, Mica, Solid), transparency, font status |
| **Sign-in** | Cookies from none, a browser (Firefox, Chrome, Edge, Brave, Opera, Vivaldi), or a `cookies.txt` file |
| **Tools** | yt-dlp version and one-click update, ffmpeg status and install, and whether a JavaScript runtime (Deno or Node.js) was found |

The window materials:
- **Glass** is the default. It shows your desktop through the window, blurred.
- **Acrylic** and **Mica** are the Windows system materials; Windows flattens them while the window is in the background.
- **Solid** turns the material off.

Settings, the library, the unfinished queue, the downloaded tools and the caches are stored in `%APPDATA%\dev.claws.downloader\`.

Changes to the output folder, the filename template, *Embed metadata* and *Prefer matching source audio* apply to downloads that have not started yet. A download that has started, even one that is paused, keeps the choices it started with.

## How it works

Claws is a [Tauri 2](https://tauri.app) app. The backend is written in Rust and drives two well-known tools:

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
- **Transcripts** run yt-dlp with `--skip-download` and exactly one caption track. Claws then parses the VTT or SRT file itself (`transcript.rs`), removes markup and the repeats that rolling automatic captions carry from line to line while keeping genuinely repeated speech, and publishes the text through the same work folder and journal as a video.
- **Downloading** runs yt-dlp with progress markers that the backend parses live. Download progress arrives on stdout and post-processing progress on stderr. Each job runs in its own process tree, so pause and cancel stop yt-dlp and its ffmpeg children together.
- **Work folders.** yt-dlp writes only inside the job's own folder, `<output folder>\.claws-work\<id>\`.
  - When the download succeeds, Claws moves the finished file, and any separate subtitles, into the output folder. The move never replaces an existing file.
  - If a file with that name already exists, the download is kept and **Try Again** only finishes the move.
  - Cancelling deletes the job's work folder, which Claws identifies by a marker inside it.
- **Network trouble.** yt-dlp retries up to 10 times, with waits that grow from 1 to 30 seconds. A stream that is missing a piece fails instead of saving a file with a gap in it.
- **The glass** uses the Windows composition API to blur the desktop behind the transparent window. The interface adds only tint layers, a bright liquid edge and depth on top of that blur.

## Performance

Measured on the development machine (SATA SSD, a connection of about 500 Mbit/s):

| Step | Before tuning | Now |
| --- | --- | --- |
| Press Download → first progress | ~4.7 s | **~0.8 s** |
| yt-dlp start-up | ~1.4 s | **~0.4 s** |
| Work before the first frame | ~500 ms | **~2 ms** |

What makes the difference:
- **No second read.** Reading a link keeps its result for 20 minutes, and the download starts straight from it instead of reading the page again. If the site's links have expired, Claws retries once with a fresh read.
- **Faster start-up.** yt-dlp is installed as its folder build, which starts about three times faster than the single-file build.
- **Parallel fragments.** Streams served in fragments (HLS and DASH) download 8 fragments at a time, which helps on high-latency links.
- **No waiting at launch.** The window draws immediately, and tool checks run in the background.
- **Embed metadata** costs one extra rewrite of each file after merging. Turn it off, or use **Fast Save** for a single download, for the fastest finish on long videos.
- **A responsive queue.** Progress updates repaint only their own row, and a whole playlist is added in one step. With 500 queued jobs and 8 downloading, layout and style work over the same stretch of progress fell from about 730 ms to 63 ms. See [`docs/performance/queue-ui.md`](docs/performance/queue-ui.md).

## Privacy and security

- **Local only.** Claws makes no telemetry, analytics or account calls. The only network traffic is to the sites you download from (including the servers their artwork comes from), and to GitHub when yt-dlp or ffmpeg is installed, updated or checked for updates.
- **Verified tools.** yt-dlp and ffmpeg downloads are checked against the SHA-256 checksums their projects publish and discarded on a mismatch. Archives are then unpacked defensively: names that could land outside the tool folder are skipped, encrypted entries and links are refused, and the number of entries and the bytes actually written are capped, whatever size an entry claims. The `zip` library is kept on a patched release with only the one compression method these archives use.
- **Untrusted input stays contained.**
  - Only `http` and `https` links are accepted.
  - They reach yt-dlp as plain arguments after `--`, never through a shell.
  - Settings that end up on the command line, such as browser names, cookie files and filename templates, are validated first.
  - Titles and other text from sites are always rendered as text, never as HTML.
- **Least privilege.** The interface can only open files and folders the app itself recorded, and the window runs under a strict Content Security Policy.
- **Cookies** are used only while a cookie source is set in **Settings → Sign-in**: yt-dlp then reads them each time it looks up or downloads a link, and sends each cookie only to the site it belongs to.

## Fonts

**Salted Flutters Polaris** is the interface font and is bundled with Claws, so users do not need to install it separately. The four web fonts are committed in `src/assets/fonts/`; the unmodified license is in `public/fonts/LICENSE.txt`. Vite gives each font a content fingerprint and bundles it into `dist/assets/`. The private CSS alias `Claws Polaris` prevents substitution by an older installed font. Normal builds verify approved font hashes before building and verify the actual packaged fonts afterward, without requiring Python.

Python is only needed to regenerate them from the four source files and license committed in `assets/fonts/polaris/`:

```powershell
pip install "fonttools[woff]"
python scripts/build-fonts.py
```

The build script creates the four Polaris weights used by the interface (Regular, Medium, Semibold and Bold), subset to Latin, Greek, Cyrillic and common punctuation while preserving tabular figures, kerning and ligatures. It synchronizes outline advance widths with the layout metrics, verifies every serialized glyph, then updates `scripts/font-integrity.json`. Run `npm run test:fonts` for packaging regressions and `python scripts/test_font_metrics.py` for glyph metrics and preserved outlines.

## Building from source

**Prerequisites:** [Rust](https://rustup.rs) 1.85 or newer, [Node.js](https://nodejs.org) 20 or newer, and WebView2.

```powershell
git clone https://github.com/ppolkadot/Claws.git
cd Claws
npm install

npm run tauri dev       # run with hot reload
npm run tauri build     # produce the installer in src-tauri/target/release/bundle/nsis/
```

| Command | What it does |
| --- | --- |
| `npm run dev` | Runs the interface in a browser with synthetic sample data. Useful for styling work without the backend. |
| `npm run build` | Verifies font integrity, type-checks and builds the frontend, then checks the packaged font files. |
| `npm run test:fonts` | Checks bundled-only font selection and rejection of corrupt or stale font assets. |
| `cargo clippy` (in `src-tauri/`) | Lints the backend. |
| `cargo test` (in `src-tauri/`) | Runs the backend's tests. Tests that drive the real yt-dlp against local fixtures need Python and yt-dlp (`CLAWS_TEST_YTDLP`, or the copy Claws installed), and skip without them. |
| `npm run test:queue` | Runs the queue event tests with Node's test runner. |
| `npm run test:logic` | Runs the pure frontend logic tests: link parsing and lookup scheduling, size estimates, resolution choices and caption options. |
| `npm run test:motion` | Runs the motion tests: progress gliding and snapping, row ordering, completion feedback and the file-drag gesture. |
| `python scripts/build-fonts.py` | Regenerates the committed Polaris web fonts (optional, see [Fonts](#fonts)). |

`scripts/dev/` has the tools used to test the real app:
- `download-fixtures.py`, a localhost server with controlled faults: a missing HLS segment, servers with and without Range support, 503s, a dropped connection, an expired link, and a slow link;
- `download-bench.mjs`, `queue-bench.mjs`, `batch-bench.mjs` and `motion-bench.mjs`, which measure transfers, the Queue view, a 500-link batch and each animation (the last three drive headless Edge against the dev mock);
- a DevTools driver for the WebView;
- a capture script that verifies the desktop glass against a test pattern, and refuses to capture if another window overlaps Claws;
- a download-phase timer.

See the [developer handoff](docs/HANDOFF.md) for tool usage and the [documentation index](docs/README.md) for current guidance and historical references.

## Project structure

```
claws/
├── src/                      Frontend (TypeScript + CSS, no framework)
│   ├── views/                Download, Queue, Library, Settings, and the window shell
│   ├── ui/                   Controls, layout helpers, icons, spring motion, progress and
│   │                         row-order logic, the file-drag gesture
│   ├── download-input.ts     Link parsing and the three-at-a-time lookup scheduler
│   ├── download-estimates.ts Size and free-space wording, estimate runs
│   ├── resolution.ts         Resolution choices and size classes
│   ├── transcript-options.ts Caption language and format choices
│   └── styles/               Materials and tokens, components, screens, fonts
├── src-tauri/                Backend (Rust)
│   └── src/
│       ├── downloader.rs     Queue, concurrency, pause/resume/cancel, publication
│       ├── download_policy.rs  Frozen job settings, the yt-dlp command line, failure kinds
│       ├── download_files.rs Work folders, safe publication and cleanup
│       ├── transfer_progress.rs  Byte-weighted progress and time left
│       ├── disk_budget.rs    Free-space estimates (one formula for admission and previews)
│       ├── estimate.rs       The size preview command
│       ├── transcript.rs     Caption parsing and text/SRT/VTT output
│       ├── file_drag.rs      Resolving a Library file for a native drag
│       ├── queue_writer.rs   Ordered background saves of the queue
│       ├── probe.rs          Reading links, and the 20-minute probe cache
│       ├── links.rs          Saved details of downloaded links, for instant lookups
│       ├── history.rs        The library and its artwork
│       ├── tools.rs          Installing and updating yt-dlp and ffmpeg
│       ├── desktop.rs        Quick Open shortcut, notification-area icon, open at login
│       ├── window.rs         Desktop glass and the Windows materials
│       └── commands.rs       Everything the interface can call
├── scripts/                  Font build, icon source, dev tools
├── docs/                     Project handoff and context
├── PRODUCT.md                Product brief
└── DESIGN.md                 Design system
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

Sites change often. Open **Settings → Tools** and press **Check for Updates** to get the newest yt-dlp.
</details>

<details>
<summary><b>Quick Open does nothing when I press the shortcut</b></summary>

Another app may already use that key combination; Claws says so when it starts or when you change the shortcut. Choose a different one in **Settings → Quick Access**. The shortcut only works while Claws is running, so keep **Keep running when closed** on, and turn on **Open at login** if you want it available from the moment you sign in.
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

Large videos need room for the downloaded streams and for the merged file while both exist, plus a safety margin. Space other downloads have already used is not counted twice: each running download holds only what it still needs. The check runs at transfer admission and on processing notifications (merging, embedding). The latter are best effort: the tool may already have begun processing. A late low-space notification does not reject an otherwise successfully finished result. Free up space on the drive named in the message, or choose another output folder for new downloads. Then press **Try Again**: what was already downloaded is kept.
</details>

<details>
<summary><b>"Claws could not save its download queue" or "…could not add it to your Library"</b></summary>

Claws keeps its own records in `%APPDATA%\dev.claws.downloader\`. When it cannot write there (a full drive, or a folder it may not change), it does not start a download, delete files or forget a finished one without a saved record. Nothing is lost:
- A download that could not start stays in the queue with its folder; press **Try Again** once the problem is fixed.
- A finished file that could not be added to the Library is already in the output folder. **Try Again** only adds it to the Library; nothing is downloaded again. If Claws is closed first, it finishes this the next time it starts.
</details>

<details>
<summary><b>What is the hidden <code>.claws-work</code> folder?</b></summary>

Unfinished downloads live there, one folder per download, until they are finished or cancelled. Paused and failed downloads keep theirs so they can resume. Trying a cancelled download again starts it over in a new folder, so the cancelled one's cleanup can never touch it. Leave it alone while Claws is running. A folder that could not be deleted because a file was in use is removed the next time Claws starts.
</details>

## Contributing

Issues and pull requests are welcome. Before opening a PR:

- run `npm run build`, `cargo clippy` and `cargo test`, and keep all three clean;
- follow the existing style: plain TypeScript and CSS, with comments that explain *why*;
- keep visual work consistent with [`DESIGN.md`](DESIGN.md).

## Acknowledgements

Claws exists because of [yt-dlp](https://github.com/yt-dlp/yt-dlp), [FFmpeg](https://ffmpeg.org) and the [yt-dlp FFmpeg builds](https://github.com/yt-dlp/FFmpeg-Builds), and is built with [Tauri](https://tauri.app).

## License

Released under the [MIT License](LICENSE).

Claws is a tool. Only download content you have the right to save, and respect the terms of the sites you use.
