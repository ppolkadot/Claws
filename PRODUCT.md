# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Stack

Confirmed by the user: Tauri v2 (Rust backend) with a web frontend (plain TypeScript + CSS, Vite). Windows only for the first version. The glass window uses native Windows 11 backdrop materials (Acrylic/Mica) plus CSS `backdrop-filter` inside the page.

## Users

Inferred: the person who asked for it, a single desktop user on Windows 11 who pastes links and wants the video saved locally. No accounts, no sharing, no multi-user roles.

## Product Purpose

A local desktop app where the user pastes a link from any site and downloads the video (or audio) to disk. Success is "paste, pick quality, done" with no per-site knowledge needed.

## Positioning

Native Rust shell around yt-dlp and ffmpeg, so site coverage tracks yt-dlp's extractor list (1,800+ sites) rather than hand-written extractors, wrapped in an Apple-style glass interface instead of the usual utilitarian downloader UI.

## Operating Context

- Everything runs locally; the only network traffic is to the sites being downloaded from, to GitHub for the tools Claws installs when asked (yt-dlp, ffmpeg, Deno), to Spotify's public pages for a pasted track link, and, only for a download where the user turns SponsorBlock on, to the SponsorBlock service.
- yt-dlp is downloaded and updated by the app into its own data directory. ffmpeg is taken from PATH when present (it is on this machine) and otherwise fetched by the app.
- Some sites need a login, so cookies can come from a browser profile or a cookies.txt file.

## Capabilities and Constraints

Confirmed first-version scope:

- Paste a link, probe it, show a preview (title, thumbnail, duration, uploader).
- Format and quality picker: resolution, audio-only, subtitles.
- Queue with several simultaneous downloads, playlist and channel support, progress, pause/resume, cancel.
- Download history/library with thumbnails, search, open-in-folder.
- Settings: output folder, filename template, cookie source, theme, glass intensity.

Constraints:

- Site support is bounded by yt-dlp; the app must not promise sites yt-dlp cannot extract.
- URLs are untrusted input: only http(s) links, always passed to yt-dlp as arguments (never through a shell), after a `--` separator.
- Windows only; other platforms are explicitly out of scope for now.

## Brand Commitments

Binding visual constraints volunteered by the user: interface as close to Apple's as possible, a strong focus on blur effects, and a glass appearance. The working name from the repository is "Claws".

## Evidence on Hand

No existing UI, assets, or copy. Nothing about users, usage, or sites has been measured. Any sample data shown in the UI during development must be labeled synthetic.

## Product Principles

1. Paste-and-go: the shortest path from a link to a file on disk is the main path, everything else is optional.
2. Honest status: show real progress, real errors from yt-dlp, and what was actually saved and where.
3. Local and private: no telemetry, no accounts, no cloud.
4. Stay current: extractors rot, so updating yt-dlp is a first-class, one-click action.

## Accessibility & Inclusion

No product-specific standard established. Translucent surfaces must still keep text legible, so the glass intensity setting and a reduced-transparency fallback are required rather than optional.
