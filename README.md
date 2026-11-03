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
- **Format picker.** Best quality or a height cap from 144p to 4K, saved as MP4 when possible. Audio only in MP3, M4A, Opus, FLAC, WAV or the original stream. Subtitles as separate `.srt` files or embedded.
- **Playlists and channels.** Pick exactly which videos to download.

**Managing**
- **Queue.** Several downloads at once, with live progress, speed and time left. Pause, resume, cancel and retry. Unfinished downloads survive a restart as paused, and cancelling cleans up the partial files.
- **Library.** Finished downloads with artwork and search. Open them or show them in their folder.
- **Sign-in.** Use cookies from your browser, or a `cookies.txt` file, for sites that need you to be logged in.

**Interface**
- **Desktop glass.** The desktop shows through the window, blurred by Windows itself. Apple-style materials sit on top, with no painted background.
- **Apple conventions.** A floating sidebar you can collapse, a toolbar that turns to frosted glass as content scrolls under it, macOS window controls, and inset grouped settings.
- **Spring motion.** Controls compress and spring back, switches stretch while held, the sidebar selection glides between items, and sheets and toasts arrive on a spring. All of it respects *Reduce motion*.
- **Light and dark.** Follows Windows or set either one, with adjustable transparency and an opaque Solid mode.

## Installation

### From a release

1. Download `Claws_<version>_x64-setup.exe` from the [Releases](https://github.com/ppolkadot/Claws/releases) page.
2. Run it. Test builds are not code-signed yet, so Windows SmartScreen may warn on first run; choose **More info → Run anyway**.
3. On first launch Claws downloads yt-dlp into its own folder and verifies it against the checksum yt-dlp publishes. This happens once and takes a few seconds.

### Requirements

| | |
| --- | --- |
| OS | Windows 11. Windows 10 works, but the glass effects depend on Windows 11. |
| Runtime | Microsoft Edge WebView2, included with Windows 11. |
| ffmpeg | Needed to merge HD video with audio and to convert audio. Claws uses the copy on your `PATH`, or installs one from **Settings → Tools**. |
| JavaScript runtime | Recent YouTube extraction needs Deno or Node.js. Claws finds either automatically. |

## Using Claws

1. **Paste a link** into the field on the Download page and press **Get Info**, or just press <kbd>Ctrl</kbd>+<kbd>V</kbd> anywhere.
2. **Choose** Video or Audio Only, a quality or format, and optionally subtitles. For a playlist, tick the videos you want.
3. **Press Download.** The job joins the queue and starts immediately. Finished files appear in the Library.

### Keyboard shortcuts

| Shortcut | Action |
| --- | --- |
| <kbd>Ctrl</kbd>+<kbd>V</kbd> | Paste a link and look it up, from anywhere in the window |
| <kbd>Ctrl</kbd>+<kbd>1</kbd> … <kbd>4</kbd> | Download, Queue, Library, Settings |
| <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>S</kbd> | Show or hide the sidebar |
| <kbd>Enter</kbd> | Look up the link in the URL field |
| <kbd>Esc</kbd> | Close a confirmation sheet |

## Settings

| Section | Options |
| --- | --- |
| **Downloads** | Output folder (default `Downloads\Claws`), filename template (a yt-dlp output template), simultaneous downloads (1–8), embed metadata |
| **Appearance** | Theme (System, Light, Dark), window material (Glass, Acrylic, Mica, Solid), transparency, font status |
| **Sign-in** | Cookies from none, a browser (Firefox, Chrome, Edge, Brave, Opera, Vivaldi), or a `cookies.txt` file |
| **Tools** | yt-dlp version and one-click update, ffmpeg status and install |

The window materials:
- **Glass** is the default. It shows your desktop through the window, blurred.
- **Acrylic** and **Mica** are the Windows system materials; Windows flattens them while the window is in the background.
- **Solid** turns the material off.

Settings and history are stored in `%APPDATA%\dev.claws.downloader\`.

## How it works

Claws is a [Tauri 2](https://tauri.app) app. The backend is written in Rust and drives two well-known tools:

- [**yt-dlp**](https://github.com/yt-dlp/yt-dlp) reads each site and fetches the media streams.
- [**ffmpeg**](https://ffmpeg.org) merges video with audio, converts audio and embeds metadata.

```
┌──────────────── Claws window (WebView2, TypeScript + CSS) ───────────────┐
│  Download · Queue · Library · Settings                                  │
└───────────────┬──────────────────────────────────────────▲─────────────┘
          commands │                                  events │ progress, state
┌───────────────▼──────────────────────────────────────────┴─────────────┐
│  Rust backend: probe · queue & concurrency · progress parsing · history  │
│                tool install & update · window material (DWM)            │
└───────────────┬────────────────────────────────────────────────────────┘
          spawns │  argument arrays, never a shell
        ┌─────────▼─────────┐        ┌──────────────┐
        │      yt-dlp       │──────▶ │    ffmpeg    │
        └───────────────────┘        └──────────────┘
```

- **Reading a link** runs `yt-dlp --dump-single-json --flat-playlist` and turns the result into a preview, the available qualities, subtitles and playlist entries.
- **Downloading** runs yt-dlp with progress markers that the backend parses live. Download progress arrives on stdout and post-processing progress on stderr. Each job runs in its own process tree, so pause and cancel stop yt-dlp and its ffmpeg children together.
- **The glass** uses the Windows composition API to blur the desktop behind the transparent window. The interface adds only tint layers, a bright liquid edge and depth on top of that blur.

## Performance

Measured on the development machine (SATA SSD, a connection of about 500 Mbit/s):

| Step | Before tuning | Now |
| --- | --- | --- |
| Press Download → first progress | ~4.7 s | **~0.8 s** |
| yt-dlp start-up | ~1.4 s | **~0.4 s** |
| Window ready to draw | ~500 ms | **~2 ms** |

What makes the difference:
- **No second read.** Reading a link keeps its result for 20 minutes, and the download starts straight from it instead of reading the page again. If the site's links have expired, Claws retries once with a fresh read.
- **Faster start-up.** yt-dlp is installed as its folder build, which starts about three times faster than the single-file build.
- **Parallel connections.** Each stream uses 8 connections, which helps on high-latency links and on sites that serve many small fragments.
- **No waiting at launch.** The window draws immediately, and tool checks run in the background.
- **Embed metadata** costs one extra rewrite of each file after merging. Turn it off for the fastest finish on long videos.

## Privacy and security

- **Local only.** Claws makes no telemetry, analytics or account calls. The only network traffic is to the sites you download from, and to GitHub when yt-dlp or ffmpeg is installed or updated.
- **Verified tools.** yt-dlp and ffmpeg downloads are checked against the SHA-256 checksums their projects publish and discarded on a mismatch. Archives are extracted with path checks, so nothing can be written outside the tool folder.
- **Untrusted input stays contained.**
  - Only `http` and `https` links are accepted.
  - They reach yt-dlp as plain arguments after `--`, never through a shell.
  - Settings that end up on the command line, such as browser names, cookie files and filename templates, are validated first.
  - Titles and other text from sites are always rendered as text, never as HTML.
- **Least privilege.** The interface can only open files and folders the app itself recorded, and the window runs under a strict Content Security Policy.
- **Cookies** are read only when a download needs them and go only to the site they belong to.

## Fonts

The interface is set in **Legacy UI Display**. Claws builds it into the app from the source files in `assets/fonts/Legacy UI Display/`:

```powershell
pip install "fonttools[woff]"
python scripts/build-fonts.py   # writes public/fonts/Legacy UI Display-{400,500,600,700}.woff2
```

The script subsets the four weights the interface uses (Regular, Medium, Semibold and Bold) to Latin, Greek, Cyrillic and common punctuation. That takes each from about 2.2 MB to about 120 KB, while keeping tabular figures, kerning and ligatures.

> [!IMPORTANT]
> Apple's licence for Legacy UI only covers creating mock-ups for Apple platforms. The font files and the fonts built from them are therefore **not committed** (see `.gitignore`), and an installer that contains them should not be distributed publicly. Without the files, Claws builds and runs normally and falls back to **Segoe UI Variable**. **Settings → Appearance → Font** shows which one is in use.

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
| `npm run build` | Type-checks and builds the frontend. |
| `cargo clippy` (in `src-tauri/`) | Lints the backend. |
| `python scripts/build-fonts.py` | Builds the bundled fonts (optional, see [Fonts](#fonts)). |

`scripts/dev/` has the tools used to test the real app:
- a DevTools driver for the WebView;
- a capture script that verifies the desktop glass against a test pattern, and refuses to capture if another window overlaps Claws;
- a download-phase timer.

See [`docs/HANDOFF.md`](docs/HANDOFF.md) for how to use them.

## Project structure

```
claws/
├── src/                      Frontend (TypeScript + CSS, no framework)
│   ├── views/                Download, Queue, Library, Settings, and the window shell
│   ├── ui/                   Controls, layout helpers, icons, spring motion
│   └── styles/               Materials and tokens, components, screens, fonts
├── src-tauri/                Backend (Rust)
│   └── src/
│       ├── downloader.rs     Queue, concurrency, progress, pause/resume/cancel
│       ├── probe.rs          Reading links, and the probe cache
│       ├── tools.rs          Installing and updating yt-dlp and ffmpeg
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
<summary><b>The same video in another quality is skipped</b></summary>

yt-dlp skips files that already exist under the same name. Add the quality to the filename template, for example `%(title).150B [%(id)s] %(height)sp.%(ext)s`, or move the earlier file.
</details>

## Contributing

Issues and pull requests are welcome. Before opening a PR:

- run `npm run build` and `cargo clippy`, and keep both clean;
- follow the existing style: plain TypeScript and CSS, with comments that explain *why*;
- keep visual work consistent with [`DESIGN.md`](DESIGN.md).

## Acknowledgements

Claws exists because of [yt-dlp](https://github.com/yt-dlp/yt-dlp), [FFmpeg](https://ffmpeg.org) and the [yt-dlp FFmpeg builds](https://github.com/yt-dlp/FFmpeg-Builds), and is built with [Tauri](https://tauri.app). Legacy UI is a typeface by Apple Inc.

## License

Released under the [MIT License](LICENSE).

Claws is a tool. Only download content you have the right to save, and respect the terms of the sites you use.
