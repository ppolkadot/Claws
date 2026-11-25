# Workshop media exports and SteamCMD trust

**Goal:** Fix valid SteamCMD rejection and let users save a video wallpaper as MP4, GIF, or project ZIP.

**Evidence:** Windows validates the user's updated SteamCMD signature with subject organization `Valve Corp.`. Valve's official bootstrap ZIP contains a valid executable with organization `Valve`. Claws currently accepts only the latter. The error occurs before sign-in or item transfer. Scope stays on Wallpaper Engine Workshop.

## 1. Fix publisher validation

- [ ] In `src-tauri/src/workshop.rs`, accept exact Valve publisher organizations demonstrated by both executables after successful WinVerifyTrust. Keep path, link and signature checks. Add a regression test for both names and reject lookalikes and invalid trust results.
- [ ] Exercise the existing real-binary test against both files using disposable fixtures. Read only the user's executable. Do not read credentials, run their SteamCMD, or modify their library.

## 2. Export Workshop media

- [ ] Add a typed format choice to the existing Workshop view, session, mock, IPC and Rust command. Keep ZIP available and default for compatibility. Expose MP4 and GIF for confirmed video types, with an actionable ZIP fallback when the project lacks suitable video. Resolve availability from available metadata and verify it from the downloaded manifest. Scene, web and application projects remain ZIP. Never execute their content.
- [ ] Read a size-bounded `project.json`. For video project types use its declared file only. Require a regular local path inside the checked project, reject traversal, drive prefixes, ADS and symlinks or junctions at every depth. Restrict to supported media containers. Reuse managed FFmpeg and the tool guard. Disable remote protocols and additional inputs. Spawn with argument arrays, bound diagnostics, and support current cancellation and shutdown behavior.
- [ ] MP4 uses widely playable H.264 with compatible audio when present. GIF has no audio and uses a palette and sensible frame rate. Preserve aspect ratio and source resolution by default. If exposing a resolution cap, handle portrait and prevent upscaling. Handle encoder dimension constraints explicitly. Publish only a complete nonempty file with the chosen extension. Preserve no-overwrite behavior, free-space checks and partial cleanup. Remove the SteamCMD copy only after successful publication and only when requested.
- [ ] Update the Workshop UI copy, README and developer status to explain formats, dependencies and scope. Test MP4/GIF from a generated fixture and probe the real output for codec, dimensions and audio. Test unsafe path rejection, no-overwrite, cancellation/failed conversion, and unchanged ZIP behavior. Check both local Steam library and SteamCMD routing.

## 3. Review and release

- [ ] Implementation worker reports code changes and test evidence. Claude Opus 5.5 High reviews the diff independently. Fix material findings and run the frontend build, relevant frontend suites, Rust tests and Clippy.
- [ ] Parent builds Windows x64 NSIS, checks archive, version, payload and SHA-256. Use `1.0.9d` as the nickname and keep embedded Windows version `1.0.9` per existing convention.
- [ ] Parent checks docs and the staged commit, including attribution. Push `main`, then publish a GitHub Release at `v1.0.9d` with the installer and checksum. Verify target and downloaded assets.
