# Claws feature expansion and animation: remaining work

> For implementers: execute one deliverable at a time and recheck HEAD first. Completed work has been removed from this plan, as the release checklist requires; the git history and `docs/CURRENT_DEVELOPMENT_STATUS.md` record it.

**Date:** 2026-11-08. Scope revised by the owner; the feature work below was implemented on branch `feature-expansion-and-motion`.
**Baseline:** version 1.1.0 source (release candidate), after commits `99e2235`, `d9cc1bb`, `88f5f02`, `6794e41`, `9501fa6`, `4b07877`, `c8603b9` and `a686590`.
**Goal:** a Windows downloader with useful batch, editorial and text workflows and restrained animation, without weakening reliability, and with no speed claim the evidence does not support.
**Architecture:** Tauri, TypeScript, yt-dlp, owned workspaces, publication journals, revisioned events and the queue writer. Focused modules (`estimate.rs`, `transcript.rs`, `file_drag.rs`) rather than a larger `downloader.rs`.

Scope: macOS support, vocal removal, ProRes presets and clip sections are excluded for this release at the owner's request. **Clip sections are deferred, not abandoned: they are planned for a future version**, and nothing here should be built in a way that blocks them. Claws remains Windows-only.

## Delivered in 1.1.0 (source)

Font safeguards; independent-link batches; size and temporary-space previews; source-driven resolutions up to 8K with Compatible and Original output; transcript export; native outgoing file drag; completion feedback and the motion pass. Their behaviour, tests and limits are described in `docs/releases/1.1.0.md` and `docs/CURRENT_DEVELOPMENT_STATUS.md`.

## 1. Remaining before the owner-requested 1.0.0 can be published

These need a person at a Windows desktop or the owner's authorization; none can be established by the automated checks.

- [ ] **Native drag acceptance.** Run the manual checklist in `docs/CURRENT_DEVELOPMENT_STATUS.md` with disposable data: Explorer copy, one installed editor (record name, version, container and codec separately from whether the drop happened), Escape, a drop onto Claws, a missing file, Unicode and long paths, and that the Library file is intact afterwards. A passing build or fixture test does not prove timeline import.
- [ ] **Windows 10 and receiver stalls.** Test the direct Shell/OLE worker on Windows 10 and against a receiver that stalls; verify Claws stays responsive, later drags are bounded and downloads continue.
- [ ] **Fresh install and upgrade.** Install the current 1.0.0 installer on a clean profile, then upgrade from an installed 1.0.5 (the queue file moves to schema 3; 1.0.5 sets a schema-3 queue aside rather than reading it). Neither has been tested.
- [ ] **A real source.** Run one download of each kind (video, Original video, audio, transcript, a pasted batch) against a real site with disposable settings. The automated tests use fixtures and made-up metadata only, so site-specific behaviour (for example YouTube caption shapes, 6K/8K and HDR formats) is unverified.
- [ ] **Reduce motion and visual check** in the real WebView2 window. The motion measurements were taken in headless Edge against the dev mock.
- [ ] **Publish only with authorization.** Verify the uploaded installer's digest against the local file and the release commit.

## 2. P0: Source integrity and measured speed diagnosis

**Files:** `scripts/dev/download-bench.mjs`, `scripts/dev/download-fixtures.py`, `docs/performance/speed-diagnosis.md`; the earlier speed plan `2026-11-04-download-reliability-and-performance.md` remains authoritative for acceleration details.

The font safeguards are committed and verified; preserve them. The affected slow source has still not been identified, so the speed question remains open.

- [ ] Capture an affected source, selected formats, tool versions and options; redact signed URLs and credentials from stored results.
- [ ] Compare Claws with standalone yt-dlp using identical source formats. Separate lookup, transfer, merge, encoding and publication time, including child-process CPU.
- [ ] Repeat comparable measurements with one warm-up and three timed runs; verify hashes or streams/duration.
- [ ] Implement acceleration only for the demonstrated cause. An external connection or source cap must be reported as such.

**Exit:** a diagnosed bottleneck or an explicit missing-evidence record; no unsupported speed claim. Preserve publication, cancel/retry and Library-failure regressions.

## 3. Deferred and conditional

- **Clip sections** (future version). The earlier design remains a sound starting point: validated ranges with a finite, ordered start and end checked again in Rust; yt-dlp `--download-sections` (note it accepts reversed, empty and chapter ranges that match nothing without error, and its size fields ignore sections); approximate (keyframe-aligned) and precise (re-encode) cuts disclosed separately; the range part of the job options and of the duplicate key; a clipped-subtitle policy; and the `RunPlan` and estimate extension points already documented in `disk_budget.rs` and `estimate.rs`.
- **Editing presets and ProRes.** Excluded. If revived: ProRes needs the MOV container and `prores_ks`; a bounded processing stage with cancellation, original streams kept until validation, and larger disk estimates. The backend currently rejects `videoPreset: "editingProRes"` and refuses to run a saved job that asks for it.
- **Most replayed.** Inspect whether selected sources expose usable heatmap data; show a shortcut only when present. It must not reintroduce clip-export controls.
- **Speech recognition.** Optional local model, language selection, separate download size, timestamp output and cancellation; do not bundle a large model to claim transcript support.
- **Lyrics.** Only text from a legitimate supported source; distinguish lyrics from speech transcripts.
- **Multiple-file drag**, after single-file reliability is proven.
- **macOS.** Separate platform plan; no macOS-native claim for this release.

## 4. Verification for every increment

- [ ] Write a failing regression, implement the smallest change, retain the lifecycle tests.
- [ ] Run `npm ci`, `npm run test:fonts`, `npm run test:queue`, `npm run test:logic`, `npm run test:motion`, `python scripts/test_font_metrics.py` and `npm run build`.
- [ ] Build the fixture tool (`cargo build --locked --manifest-path src-tauri/Cargo.toml --features test-tools --bin claws-fixture-tool`), then `cargo test --locked --manifest-path src-tauri/Cargo.toml` and `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets --features test-tools -- -D warnings`.
- [ ] Exercise affected native flows with disposable data, including cancellation, restart, blocked persistence and output conflicts. Never use the owner's real queue as a fixture.
- [ ] Measure installer bytes, first-run tool bytes, installed footprint, startup and relevant job-stage timing separately. Keep the base installer below 100 MB and optional models and tools outside it.
- [ ] Validate new media paths with ffprobe and, where promised, an actual editor import.
- [ ] Update README, HANDOFF and the status document; remove completed tasks from this plan.
- [ ] Synchronize all five version files (`package.json`, `package-lock.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `src-tauri/tauri.conf.json`), build the installer after the final source change, test upgrade, record SHA-256, and publish only with authorization.

## Sources and limits

- Feature inspiration: https://polyviral.org/id4/ (page advertising reviewed; the ID4 app and its animation implementation were not tested; all motion here is original code).
- yt-dlp behaviour: https://github.com/yt-dlp/yt-dlp#download-options and its format, subtitle and post-processing documentation, checked against the installed 2026.10.26 with offline `--simulate --load-info-json` runs.
- Speed evidence and deferred acceleration: `docs/performance/speed-diagnosis.md` and the 2026-11-04 plan.

A successful download from one site does not establish all-site support. A small installer does not establish a small tool or model footprint. Smooth animation does not establish higher network throughput.
