# Wallpaper Engine local library study

2026-11-26. Study for the version after 1.0.9e. The study itself changed no code; the [implementation results](#implementation-results-2026-11-26) at the end record what was built from it.

## Questions

1. Does the Steam library source give the same result as SteamCMD? If yes, can SteamCMD go?
2. Can Claws take a pasted Workshop link, find the wallpaper in the local Steam folder, and hand the user the MP4 (or the project when there is no video)?

## Evidence from a real library (this PC, read only)

Steam: `HKCU\Software\Valve\Steam\SteamPath` = `c:/program files (x86)/steam`; `libraryfolders.vdf` lists six libraries (C:, F:–J:). Only the C: library has app 431960. `steamapps\workshop\appworkshop_431960.acf` lists each installed item with `size`, `timeupdated` and `manifest`.

`steamapps\workshop\content\431960` holds **216** items:

| `project.json` type | Items | Disk | Payload |
|---|---:|---:|---|
| video | 84 | 22.2 GB | 84/84 declare an `.mp4` that exists next to `project.json` |
| scene | 122 | 5.6 GB | 122/122 are `scene.pkg` + `project.json` + preview |
| web | 6 | 0.3 GB | HTML project |
| (none) | 4 | 24 MB | Presets: only `project.json` with `preset` and `dependency` keys, no payload |

ffprobe on the 84 videos:

| Property | Result |
|---|---|
| Video codec | H.264 82, HEVC 1, VP9 1 |
| Pixel format | yuv420p 83, yuv420p10le 1 |
| Audio | AAC 64, none 20 |
| Height | 1080p 45, 2160p 23, others 720–3840 |

Filenames include `[ ]`, CJK characters and a space before the extension (`Mountains In The Clouds [4K] .mp4`). Claws' Rust paths handle these; PowerShell's `Test-Path` without `-LiteralPath` does not, which matters only for scripts and tests.

## 1. Steam library vs SteamCMD

Both sources end with the same bytes. SteamCMD writes the same depot into its own `steamapps\workshop\content\431960\<id>` that the Steam client writes into the library, and Claws runs the same `save_project` / `convert_media` on either folder. Both need an account that owns Wallpaper Engine: anonymous SteamCMD fails (see the [no-sign-in spike](wallpaper-engine-no-login-spike.md)), and Valve's public details give no `file_url` for any of 20 sampled items.

| | Steam library | SteamCMD |
|---|---|---|
| Setup | None if Steam is installed | Install tool, separate sign-in, Steam Guard |
| Already-subscribed item | Instant (216 here) | Downloads again |
| New item | Subscribe in Steam, wait | Downloads without subscribing |
| Needs Steam client on this PC | Yes | No |
| Disk | Steam's copy + export | SteamCMD's copy + export (removed if the option is on) |
| Failure modes seen | Not subscribed yet | `Invalid Password`, generic `Failure`, signature/publisher checks |

What SteamCMD adds is narrow: downloading without a subscription, and working on a PC without the Steam client while the user still owns Wallpaper Engine on that account. Anyone who owns Wallpaper Engine almost always runs it through the Steam client, so the library source already covers the normal case with no Claws-side sign-in.

**Recommendation:** remove SteamCMD in two steps.

1. **Next version:** the library is the only visible source. SteamCMD moves to **Settings → Wallpaper Engine → Legacy SteamCMD** (off by default) and is offered on the Download page only when Steam is not installed. Release notes say it will be removed.
2. **The version after:** delete SteamCMD install, signature checks, account detection, sign-in, run/redact/classify code and their tests, unless users report needing it.

## 2. Local-folder export

Claws already implements most of the proposed workflow as the Steam library source (`find_wallpaper_in_library`, `library_project`, `workshop_media::video_file`). The study found three gaps.

1. **MP4 is always re-encoded.** `convert_media` runs libx264 `-crf 20 -preset veryfast` on every video. 82/84 local videos are already H.264 yuv420p in MP4, 64 with AAC. Re-encoding a 2160p wallpaper takes minutes, loses quality and can grow the file. The user's idea, a plain copy, is better for nearly all of them.
2. **Presets are exported as a ZIP of one JSON file.** 4/216 items are presets that only work with the item named in `dependency`.
3. **No way to browse what is installed.** The user must paste a link even for one of 216 local wallpapers.

### Research summary

- The local path is `<library>\steamapps\workshop\content\431960\<id>`, on whichever library holds Wallpaper Engine; libraries on other drives are common.
- Wallpaper Engine has no video export for scenes. Its developer says scenes behave like game levels, with no defined start or end, and suggests screen recording via **Play in Window**. Video wallpapers already contain their MP4.
- `scene.pkg` can be unpacked by open-source tools such as RePKG, which also converts `.tex` textures to images. That yields assets, not a video.
- The Steamworks `ISteamUGC` API can subscribe and download items, but a third-party app would have to run as app 431960 (Wallpaper Engine's ID) with the Steam client running. That impersonates another product and gains little over the library source. Rejected.
- Steam content, including Workshop items, is licensed for personal, non-commercial use. Exports stay personal copies; Claws should say so once next to the export button.

### Proposed workflow

```text
Paste link
  └─ parse item ID (existing) ─ GetPublishedFileDetails (existing, no key)
       └─ preview card: title, type, size, author, updated
Locate (automatic, read only)
  └─ SteamPath → libraryfolders.vdf → each library's workshop\content\431960\<id>
       ├─ found and complete (in appworkshop_431960.acf, not in workshop\downloads)
       │    └─ Classify
       ├─ found but older than Valve's time_updated → "Steam has an update pending"
       ├─ downloading (workshop\downloads\431960\<id> exists) → progress from bytes on disk
       └─ not found → [Subscribe in Steam] → watch library (existing) → Classify
Classify (from project.json, not tags)
  ├─ video  → ffprobe the declared file
  │    ├─ H.264/HEVC in MP4 → default "MP4 (original)": copy, verify size, title-named
  │    ├─ other codec/container → default "MP4 (converted)": current libx264 path
  │    └─ also: GIF, ZIP (project)
  ├─ scene  → ZIP (project), or MP4/GIF recorded through Wallpaper Engine (section 3)
  ├─ web / application → ZIP (project), never run
  └─ preset → "Preset for <parent title>"; offer the parent item, and ZIP of both
Save to Claws' download folder (existing free-space, no-overwrite, cancel rules)
  └─ Show in folder · Drag file · "Unsubscribe in Steam" link (Claws' copy stays)
```

Add an **Installed wallpapers** view as a second entry point: a grid of every local item with its `preview.jpg`/`preview.gif`, title (`project.json`), type and size, filterable by type, with multi-select export through the same Classify step. No link and no network are needed.

## 3. Scene to MP4 (2026-11-26 follow-up)

### `scene.pkg` holds no video

A read-only header survey of all 122 local `scene.pkg` files (format `PKGV0001`–`PKGV0024`, all readable) found **0** video files. Scenes contain `.json` scene graphs (122), `.tex` textures (121), `.vert`/`.frag` shaders (98), particle systems (89), audio (`.mp3`/`.ogg`/`.flac`/`.wav`, 43), fonts, models (8) and scripts (3). A scene is drawn live by Wallpaper Engine's renderer, and its developer describes scenes as game levels with no defined start or end. Unpacking alone (RePKG and similar) gives textures and sounds, not a video. Open-source re-renderers such as linux-wallpaperengine are Linux-only and incomplete for scenes.

### Rendering with Wallpaper Engine and recording works

Wallpaper Engine documents a command line: `wallpaper64.exe -control openWallpaper -file <project.json> -playInWindow <name> -width W -height H`, and `-control closeWallpaper`. Claws' managed FFmpeg (`N-129306`, yt-dlp build) includes `gfxcapture` (Windows.Graphics.Capture) and `ddagrab`.

Proof of concept on this PC, with Wallpaper Engine already running (script kept outside the repo):

| Scene | Window appeared | Recording | Mean frame-to-frame change | Result |
|---|---|---|---|---|
| 1173229707 "San Francisco" | 410 ms | 6.0 s, 1280×720, 30 fps, H.264 | 0.000 | Correct picture; the scene is a still image (see section 4) |
| 1272964419 "Persona 5 Metro Station" | 339 ms | 6.0 s, 1280×720, 30 fps, H.264, 2.4 MB | 1.455 | Correct picture with animated rain |
| Same scene, from a ZIP that was unpacked to a temporary folder | yes | 3.0 s | 1.295 | Correct picture |

`closeWallpaper` closed the window each time, and the desktop wallpaper was untouched. The filter was `gfxcapture=window_title=<name>:capture_cursor=0:max_framerate=30,hwdownload,format=bgra,fps=30,format=yuv420p`.

### Going through the ZIP is unnecessary

Wallpaper Engine opens a project from any folder, so the ZIP route works. But Claws already has the project folder in the Steam library, so zipping, unzipping and then recording only costs time and disk. Unpacking is useful only for an old Claws ZIP the user still has.

### Limits

- Needs Wallpaper Engine installed. Anyone using the library source owns it.
- Recording is real time: a 30-second MP4 takes 30 seconds, plus about 2 seconds to load.
- A Wallpaper Engine window is visible while recording. Windows.Graphics.Capture should keep working when the window is covered, but not when it is minimised. Not yet tested.
- Scenes have no end, so the user picks a length and the loop seam is not seamless. Random particles differ on every run. Mouse-parallax and audio-reactive scenes record in their idle state.
- Sound: gfxcapture records picture only. 43/122 scenes bundle audio inside `scene.pkg`. It can be read from the package and added to the MP4, or the MP4 stays silent. System loopback recording would pick up every other sound on the PC, so it is rejected.
- Output depends on the user's Wallpaper Engine quality and FPS settings. 4K recording needs a hardware encoder or a faster x264 preset; not measured.
- If Wallpaper Engine is not running, the command starts it, which also restores the user's desktop wallpapers. Acceptable, but say so.
- Application wallpapers run their own programs inside Wallpaper Engine. Keep them ZIP-only.

### Proposed scene flow

```text
Scene (or web) found in library
  └─ Formats: MP4 (recorded) · GIF (recorded) · ZIP (project)
       └─ MP4 (recorded): length (10 / 30 / 60 s), size (1080p default, source monitor max)
            ├─ locate <library>\steamapps\common\wallpaper_engine\wallpaper64.exe
            ├─ openWallpaper -playInWindow "Claws <id>" -width -height
            ├─ wait for the window (hwnd) and a short load delay
            ├─ ffmpeg gfxcapture hwnd=<hwnd> → H.264, progress "Recording 12 / 30 s", Cancel
            ├─ closeWallpaper (also on cancel, error and app exit)
            ├─ optional: add the scene's bundled sound track from scene.pkg
            └─ publish like other exports (no partial file, never overwrite)
```

Estimated work: one new Rust module (Wallpaper Engine location and command line, window wait, FFmpeg capture with progress and cancel, cleanup), the format choice in the view, and fixture plus opt-in live tests.

## 4. Recording reliability, progress and timing (2026-11-26)

### The frozen San Francisco recording is the wallpaper, not a bug

`1173229707` ("San Francisco", `PKGV0001`) contains one 7 MB texture, one image object and a 1.2 KB `scene.json` with no effects, particles, sprite textures, camera shake or parallax. Wallpaper Engine draws one still picture.

To rule out a recurring capture bug, every local scene was classified from its package, then 20 were recorded for 10 s each, starting as soon as the window appeared. The sample was the 5 predicted still, plus 15 predicted moving spread from 0.3 MB to 1.2 GB.

| Predicted from the package | Recorded | Result |
|---|---|---|
| Moving: effects, particles, puppets, scripts or camera shake (15 tested) | 15 moving | No moving scene recorded frozen or black |
| Still by `scene.json` alone (5) | 3 still, 2 moving | The 2 moving ones animate through sprite-sheet textures (`TEXS` frame data inside `.tex`) |
| Still once sprite sheets are also counted (3) | 3 still | Exact match: San Francisco, Kyoto Japan Sakura Pond, 将軍塚 |

In the still recordings, frames change only during Wallpaper Engine's fade-in (first second) and at about 8.3 s. That later change is the x264 keyframe at frame 250, not motion. Across all 122 scenes, 14 use sprite sheets and 3 are fully still.

Claws can say "This wallpaper doesn't move" before recording by reading `scene.json` and the `.tex` headers in `scene.pkg`, then offer a PNG or a short MP4 instead.

### Loading

| Measure (20 scenes) | Result |
|---|---|
| Window appears after the command | 0.13–0.22 s |
| FFmpeg's first captured frame | 1.2–1.8 s after the command |
| Picture shown when capture starts | 18 of 20 immediately; 1.2 GB scene `3488591998` black for 1.6 s, animating after about 3.5 s |

Claws should capture and discard a pre-roll while the scene loads (`-ss <pre-roll>` as an output option, frame-accurate, one FFmpeg process). Suggested: 2 s, and 4 s for packages over 500 MB, refined with more large scenes.

### Long recording

60 s at 1920×1080 of the busiest scene (`3170804765`), 2 s pre-roll: 1,800 frames at a constant 30 fps, no freeze (`freezedetect`), 226 MB. Total time **62.9 s**: 0.3 s window, 2 s pre-roll, about 0.6 s capture start, 60 s recording, 0.05 s finishing the file. Total ≈ length + pre-roll + 1 s, so the estimate before starting is reliable.

Recording size is limited by the display. On this PC's 1920×1080 screens, 1280×720 and 1600×900 came out exact, but a framed 1920×1080 window was clamped to a 1920×1067 client (1066 after even rounding).

- **Borderless fixes 1080p.** Removing the caption and frame (`WS_POPUP`, `SetWindowPos` with `SWP_FRAMECHANGED`) gave an exact 1920×1080 capture, still animating, and `closeWallpaper` still closed it.
- **The limit is the virtual screen.** `SM_CXMAXTRACK × SM_CYMAXTRACK` = 3860×1106 here (virtual screen 3840×1086: two 1080p screens side by side). Asking for 3840×2160, by command line or `SetWindowPos`, gave a 3840×1067 client. True 4K needs a display area at least 2160 px tall: a 4K monitor, or NVIDIA DSR / AMD VSR. No upscaled "4K".
- **Scenes are often authored at 4K.** 35 of 121 fixed-size scenes are authored at 3840×2160 and about 15 more at larger sizes; 34 at 1920×1080. Aspect ratios vary (3440×1440, 2048×1363, 1872×926). Sizes follow the scene's aspect ratio, up to the authored size.
- **4K needs a hardware encoder.** The PC has an RTX 3060 Ti; `h264_nvenc`, `hevc_nvenc` and `h264_mf` work, AMF and QSV don't. libx264 veryfast re-encoded real 4K at 0.84× real time, too slow for a live 4K capture. (A synthetic 4K60 test pattern is limited by generating the pattern, so it does not rank the encoders.)
- **Wallpaper Engine identity.** `wallpaper64.exe` 2.8.0.868 is Authenticode-signed by `Skutta Software GmbH`. `steamapps\appmanifest_431960.acf` gives `installdir` `wallpaper_engine`.

### Recording in the background

Scene `1272964419`, 1280×720, 5–6 s per run. The motion column is the share of frames that change.

| Wallpaper Engine window | Motion | Brightness |
|---|---|---|
| Visible | 96% | 112.1 |
| Behind every window | 92% | 112.1 |
| Off-screen (x = 5000) | 100% | 112.1 |
| Transparent and click-through | 95% | 112.0 |
| Off-screen, focus handed back to the user's window | 100% | — |
| Off-screen, another app maximized in front | 99% | — |

The owner's Wallpaper Engine settings are `playbackfocus: run`, `playbackmaximized: pause` and `playbackfullscreen: pause`; the maximized rule did not pause the pop-out window. Opening the window takes keyboard focus for a moment. Giving it back with `SetForegroundWindow` took about 0.3 s. Not yet tested: a fullscreen game, a locked screen or sleep, and whether a scene's sound plays aloud.

The implementation plan is [2026-11-26-wallpaper-recording-progress-steamcmd-removal](superpowers/plans/2026-11-26-wallpaper-recording-progress-steamcmd-removal.md).

### Video wallpapers are long and large

The 84 local videos run from 2 s to **64 min 27 s** (median 59 s); 39 are over a minute and 7 over five minutes. The largest is 5.6 GB (4K, 60 fps). Times on this PC (NVMe, same drive):

| Wallpaper | Plain copy | Current MP4 re-encode | Current GIF |
|---|---|---|---|
| `1975374442`, 782 MB, 720p, 64 min | 1.1 s | ~4.5 min (14.2× real time) | ~2.9 h (0.37×) |
| `3283673017`, 5.6 GB, 2160p, 4 min 54 s | 7.1 s | ~5.8 min (0.84×) | ~78 min (0.06×) |

Conversion times are projected from 20-second samples. Today Claws reports `Converting` with no percentage, so these runs show an indeterminate bar for minutes to hours. GIF must be limited: for example a 30-second maximum and 720p by default, with the estimate shown before starting.

### Progress tracker

Every stage reports `done` and `total` in seconds or bytes, plus elapsed time and time left. The bar is determinate wherever a total is known.

| Stage | Source of progress | Example text |
|---|---|---|
| Checking | quick (<1 s); indeterminate | Checking the wallpaper… |
| Opening Wallpaper Engine | ~0.3 s; indeterminate | Opening in Wallpaper Engine… |
| Loading scene | pre-roll countdown | Loading the scene… 1 s |
| Recording | wall clock since the first captured frame, capped at 99% until FFmpeg exits. FFmpeg's `out_time` trails by about 1.5 s because of x264 look-ahead. | Recording 0:42 / 1:00 · 156 MB · 18 s left |
| Finishing | FFmpeg exit; 0.02–0.05 s per minute recorded | Finishing the file… |
| Copying (MP4 original) | bytes copied / file size; rate from the last seconds | Saving 2.6 / 5.6 GB · 4 s left |
| Converting (MP4/GIF) | FFmpeg `-progress`: `out_time_us` / source duration; time left = remaining / `speed` | Converting 38% · 1.1× · about 3 min left |
| ZIP | bytes written (exists today) | Saving the ZIP… 64% |
| Done | total elapsed per stage | Saved in 1:03 (recorded 1:00) |

Before starting, the format row shows an estimate:
- **Recording:** length + pre-roll + 1 s.
- **Copy:** size ÷ a conservative rate, refined after the first second.
- **Conversion:** source length ÷ the speed Claws measured on its last conversion of that format at a similar resolution, saved in settings. Before any conversion has run: "estimate after a few seconds".

Short jobs use the same tracker; a one-second copy shows its bar briefly and then "Saved in 1 s".

Backend changes: add `done`, `total`, `unit`, `elapsedMs` and `etaMs` to the `workshop-progress` event, add `elapsedMs` to `WallpaperOutcome::Saved`, and run FFmpeg with `-progress pipe:1` in both conversion and recording. Cancel closes the Wallpaper Engine window, stops FFmpeg and deletes the partial file.

### Next-version work, in order

0. Progress tracker with elapsed time, time left and total time (section 4) for copy, conversion and ZIP.
1. Copy instead of re-encode for H.264/HEVC MP4 sources (ffprobe check; keep the conversion path as fallback). Fixtures: brackets, CJK, space before the extension, silent video, 10-bit.
2. Hide SteamCMD behind the legacy setting; library becomes the only default source.
3. Read `appworkshop_431960.acf` and `workshop\downloads` for "complete", "updating" and "downloading" states.
4. Preset detection and parent resolution.
5. Installed wallpapers browser.
6. Scene and web recording to MP4/GIF (sections 3 and 4): pre-roll, still-scene detection, sizes that fit the monitor, GIF limits.

### Still unverified

- The library source and the copy path in the installed native app.
- HEVC copies play in Windows' default player only with the HEVC extension; the copy default may need to be H.264-only.
- Whether `NeedsUpdate` in the ACF reliably marks a partially updated item.

## Sources

- [Wallpaper Engine: Exporting wallpapers as GIF / video](https://help.wallpaperengine.io/en/functionality/export.html)
- [Workshop folder location (Steam discussion)](https://steamcommunity.com/app/431960/discussions/2/135511913387918001)
- [Developer on exporting wallpapers as video](https://steamcommunity.com/app/431960/discussions/2/1738841319805775704)
- [RePKG fork (PKG/TEX extraction)](https://github.com/IsAkaling/repkg)
- [Steamworks ISteamUGC](https://partner.steamgames.com/doc/api/ISteamUGC)
- [Steamworks Workshop implementation](https://partner.steamgames.com/doc/features/workshop/implementation)
- [Steam Subscriber Agreement](https://store.steampowered.com/subscriber_agreement/)
- [Wallpaper Engine: Command Line Controls](https://help.wallpaperengine.io/en/functionality/cli.html)
- [FFmpeg gfxcapture filter](https://ayosec.github.io/ffmpeg-filters-docs/8.1/Sources/Video/gfxcapture.html)
- [linux-wallpaperengine README](https://raw.githubusercontent.com/Almamu/linux-wallpaperengine/main/README.md)
- [Steam Workshop Downloader notes on app 431960](https://steamdb.com/en/tools/steam-workshop-downloader)

## Implementation results (2026-11-26)

Measured on the owner's PC while implementing [the plan](superpowers/plans/2026-11-26-wallpaper-recording-progress-steamcmd-removal.md): RTX 3060 Ti, two 1920×1080 monitors side by side, Windows 11, Wallpaper Engine 2.8.0.868 running, Claws' managed FFmpeg. Every test wrote to a temporary folder and opened and closed only its own pop-out window. Claws was built in debug mode for the tests, so file speeds are a floor. All are opt-in tests (see HANDOFF → Development tools).

### Phase 0: what the spikes settled

| Question | Result |
|---|---|
| Hidden window | Settled before implementation (see section 4): borderless, off-screen, toolwindow, bottom, no-activate, focus handed back. Implemented as designed. |
| **Can a borderless window reach a size larger than the display?** | **Yes, which the plan assumed it could not.** Display limit from `GetSystemMetrics`: `SM_CXMAXTRACK`×`SM_CYMAXTRACK` = 3860×1106 (virtual screen 3840×1086). A *framed* window asked for 2560×1440 or 3840×2160 came out 2560×1067 and 3840×1067. The same window made borderless came out exactly **2560×1440** and **3840×2160**, and `gfxcapture` captured exactly that, with motion. No display setting was changed. |
| 4K recording | Verified without DSR: a real 3840×2160 render, 90 frames for 3 s at 30 fps, through the full export (a 10 s 4K recording of the 1.2 GB scene `3488591998`: 13.3 MB, 21.0 s in all). |
| Encoder bench, real 4K scene, 6 s plus 1 s pre-roll | `h264_nvenc`, `h264_mf` and `libx264 veryfast` all kept up with 4K at 30 and at 60 fps on this scene: 180 and 360 frames, **0 repeated, 0 dropped**, 7.5–7.8 s wall each. Sizes for 6 s: nvenc 32 MB (30 fps) and 57 MB (60 fps), mf 32 and 35 MB, x264 20 and 19 MB. This scene is simple; libx264's 0.84× on a real 4K video still rules it out for 4K and for 60 fps. |
| Encoder probe order and settings | `h264_nvenc` → `h264_amf` → `h264_qsv` → `h264_mf` → `libx264`, each tried with a 0.4 s test encode; on this PC `h264_nvenc`, `h264_mf` and `libx264` work, AMF and QSV do not. Settings: libx264 `-preset veryfast -crf 20`; nvenc `-preset p5 -tune hq -rc vbr -cq 21 -b:v 0`; mf `-rate_control pc_vbr -b:v <typical>k` (**`-profile:v high` makes `h264_mf` fail**: it rejects the name, so it is left out); AMF `-rc cqp -qp_i 20 -qp_p 22`; QSV `-global_quality 21`. All `yuv420p` (QSV `nv12`) with the H.264 level that fits the size and rate (3840×2160 is 5.1 at 30 fps and 5.2 at 60). AMF and QSV settings are unverified: no such hardware here. Software is limited to 1440p at 30 fps. |
| Fullscreen (exclusive) game | **Unverified.** No game was run. |
| Locked screen or sleep | **Unverified.** Locking the owner's session was not safe to test. Wallpaper Engine's `playbacksleep` is `stop`. |
| Scene sound while recording | **Unverified.** Playing a scene's audio on the owner's speakers at an unknown volume was not safe to test. Claws reports `has audio` from the package and notes that the recording is silent. |
| GPU contention | Implemented: FFmpeg's `dup_frames` are counted and a recording with over 5% repeats is saved with a note. Not stress-tested under a heavy GPU load. |

### What differs from the plan

1. **Sizes beyond the display's reported limit are flagged, not removed.** The plan disabled 4K unless `SM_*MAXTRACK` allowed it. Because borderless windows are not held to that limit, Claws offers every size up to the authored height and encoder capability, shows "4K may need a display set to 3840×2160 or larger, such as a 4K monitor or NVIDIA DSR / AMD VSR" under the choice when the size is beyond what the display reports, and the existing check (`GetClientRect` after sizing, never scaling) fails with that same advice before anything is recorded if a PC does refuse. This keeps the plan's rules (no upscaling, exact size, a clear message) while not denying a size this PC can do.
2. **Recording seconds are counted from the clock with a measured offset.** FFmpeg's first output past the pre-roll arrives about 0.52 s after the first kept frame was captured (measured with `h264_nvenc`), so the count starts that much earlier than the first progress block. The time left was then within 0.1 s of the real remaining time at 10, 25, 50, 75 and 90% (30 s recording at 1080p: predicted 26.9, 22.3, 14.8, 7.4 and 2.9 s against 27.0, 22.3, 14.9, 7.4 and 3.0 s).
3. **The copy time left holds the recent rate between half and twice the stage's average.** A cold copy of the 5.6 GB video stalled during the write: the plain 3 s rate once promised 7,462 s left at 25%. On the recorded series (95 points) holding the rate between 0.5× and 2× of the stage average cut the median error from 0.37 to 0.24 and the worst overestimate from ×3.8 to ×2.1. It is one run's data; the shape of the rule (a bounded rate) is what matters.
4. **Two small additions to the contracts:** a `bytes` field on `workshop-progress` (the video's size so far while recording; the plan's example line shows it but the contract did not carry it), and a `note` on `Saved` (the repeated-frames message). Both are optional.
5. **A confirmation for any export estimated at over ten minutes**, not only GIF.
6. **Sprite-sheet scenes:** 13 of 122 local scenes carry the animation flag in their texture headers (the study's 14 came from a text search; the header flag and a search for `TEXS` agree on all 122 scenes). Moving 119, still 3 (the same three: San Francisco `1173229707`, `2267802482`, `2284727786`), with sound 45 (43 had sound files; 2 more list sounds in `scene.json`).

### Live results

| Check | Result |
|---|---|
| Copy of `3283673017` (5.86 GB, 4K 60 fps H.264 + AAC) | Warm file: **9.97 s** (587 MB/s). From a cold disk, through the whole export: **51.8 s** and **66.7 s** in two runs (checking 1.4 s, copying 41.7 s and 57.1 s, finishing the file 8.6 s and 8.2 s: the flush to disk is a step of its own, shown as "Finishing the file…"). The old re-encode took about 5.8 min. |
| Copy time left | See difference 3. With the plain 3 s rate the time left at 25/50/75/90% was off by up to 7,462 s early on and by a factor of three late; with the held rate the median error is 24%. It is marked "about". |
| Copy of `1975374442` (782 MB, 720p, 64 min) | 3.4 s. |
| Conversion `3376465584` (VP9, 2560×1440, 8 min 20 s, 98 MB) | Converted with libx264 (1440p is at the software limit): **184 s**, 2.7× real time, to a 138 MB H.264 MP4. Time left at 10, 25, 50, 75 and 90%: predicted 140, 120, 82, 46 and 18 s against actual 167, 142, 100, 42 and 17 s (early readings optimistic by 15–20%, FFmpeg's speed starts high; marked "about"). The speed (2.7×) is remembered for the next estimate under `mp4-1440`. |
| 30 s recording, 1080p30, `1272964419` | **36.4 s** in all (checking 2.2 s, opening 0.4 s, loading 3.2 s, recording 29.3 s, finishing 1.3 s), 48.2 MB (12.8 Mbit/s). Time left within 0.1 s at every check. |
| 10 min recording, 1080p30, `1272964419` | **611.1 s** in all (checking 0.8 s, opening 0.3 s, loading 3.2 s, recording 599.7 s, finishing 7.2 s: flushing 966 MB, probing and the three-window motion check), 966 MB (12.9 Mbit/s). Time left within 0.4 s of the real remaining time at 10, 25, 50, 75 and 90%. A second export of the same wallpaper into the same folder was named `(1)`: nothing was overwritten. |
| Still scene to PNG, `1173229707`, 1080p | 4.0 s, 2.07 MB, a valid PNG at 1920×1080 (checking 0.8 s, opening 0.3 s, loading 2.8 s, finishing 0.2 s). |
| 4K recording of a 4K scene | See Phase 0: exactly 3840×2160, 90 frames for 3 s; 10 s through the full export in 21.0 s. |
| Web wallpaper `3136351729`, 5 s at 720p30, full export | 9.8 s, 433 KB, stages checking, opening, loading, recording, finishing; time left within 0.1 s. (No frozen-capture check for web pages: Claws cannot tell whether a page moves.) |
| Stopping a recording that cannot go on | A recording on a drive that reports itself full stopped within seconds (the supervisor kills FFmpeg itself, then the window is closed); a conversion whose output stops moving (stall limit forced to zero) was stopped and left no file. An early review of the code found that these two paths waited for FFmpeg to end by itself; fixed, with both tests added. |
| 3 s recordings, 720p30 | 90 frames at 1280×720 every time (88–92 allowed), 6.4–6.5 s in all; the 1.2 GB scene `3488591998` took 9.9 s (4 s pre-roll). |
| Cancel | While opening, while loading, while recording, and once the way Claws' shutdown ends a recording (FFmpeg's tree killed and the window closed from outside, mid-recording): each ended `Cancelled` with no Wallpaper Engine window, no FFmpeg process and no partial file. A copy, a conversion and a ZIP stopped by Cancel leave no partial file (unit and live tests). |
| Closing Claws during a recording | The shutdown steps (`kill_tree` on FFmpeg, `close_recording_window`) were run against a live recording as above; the full application was not closed, because that would have touched the owner's profile. |
| Signature | The real `wallpaper64.exe` verifies and its signer is exactly `Skutta Software GmbH`; an unsigned executable (the test binary) is refused; lookalikes (`Skutta Software GmbH.`, case changes, a Cyrillic letter, a suffix) are refused by the exact comparison. |

### Still unverified

- A fullscreen exclusive game running, a locked screen or sleep, a scene's sound playing aloud, and a heavy GPU load during a recording.
- AMD (`h264_amf`) and Intel (`h264_qsv`) encoders, and a display that does refuse a borderless window of the full size (the message exists and is unit-tested; this PC never produced it).
- The installed native application (the installer was not built), including closing it during a recording.
- Whether `NeedsUpdate` in the ACF marks a partly updated item reliably (Claws does not use it).
- HEVC copies play in Windows' default player only with the HEVC extension: Claws copies H.264 only and converts the rest.
- A web wallpaper's page runs inside Wallpaper Engine's own browser while it is recorded, exactly as when it is applied. Only one of the library's six web wallpapers was recorded.

### Window cleanup recheck (2026-11-27)

`CLAWS_TEST_WALLPAPER_ENGINE_CANCEL=1272964419 cargo test a_cancelled_recording_leaves_nothing_behind_when_given -- --nocapture` passed on this PC. Cancel during opening, loading, recording and shutdown-like cancellation during recording each reported success; the test checked that its window, FFmpeg process and temporary partial were gone. This does not cover a launch that appears at the deadline or a refused close; those paths have deterministic tests.
