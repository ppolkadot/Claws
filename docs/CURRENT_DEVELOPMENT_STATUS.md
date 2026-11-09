# Current development status

Current release target: **1.0.0**, requested by the owner on 2026-11-09. It includes the expanded feature set and consolidated review fixes, despite its lower version number. The latest published GitHub release remains 1.0.5. The current repository installer and fresh validation are recorded in [1.0.0 release notes](releases/1.0.0.md); the old 1.1.0 installer is a historical artifact.

Startup recovery's suppressed kept-file save failure is fixed and has a red/green lifecycle regression. New code and tests belong in the final source commit. Application identifier and queue schema remain unchanged; installer downgrades are explicitly allowed. Live native drag/editor, real-site speed and clean-install/upgrade acceptance remain pending.

Verified for the current build: 409 Rust tests, 241 Node tests, 2 Python tests, clean-export dependency install/build, Clippy, production NSIS build, archive integrity, embedded executable version and exact decompressed font/CSS/JS/license bytes. Native editor drag and live installation/upgrade are still unverified. No publication or commit was performed.

| Feature | Status | Evidence / limits |
| --- | --- | --- |
| Font safeguards | Committed `99e2235` | Source and packaged integrity checks, now also rejecting orphaned or legacy packaged fonts and running correctly through junctions. |
| Independent-link batches | Committed `d9cc1bb` | Up to 500 links, three lookups at a time, per-row retry, bounded rendering. The probe cache has a size budget with leases so a prune never deletes a file a download is using. |
| Size and temporary-space previews | Committed `88f5f02` | Computed by re-running the download's own selection through `yt-dlp --simulate`; peak sized by the formula admission uses. Display only. |
| Higher resolutions, Compatible / Original | Committed `6794e41` | Source-driven choices up to 8K, exact labels, shorter-side handling for portrait video, no upscaling. Original verified with ffprobe and packet hashes on synthetic media (codec, HDR tags, dimensions, audio). Queue schema is now 3. `editingProRes` is rejected. |
| Transcript export | Committed `9501fa6` | Caption-derived TXT, timestamped TXT, SRT, VTT; no speech recognition. Verified with a real yt-dlp against local caption files; not against real sites. |
| Native outgoing drag | Committed `4b07877`, hardened `c8603b9` | See the review below. Real Explorer and editor drags are unverified. |
| Completion feedback, motion pass | Committed `4b07877`, `c8603b9`, `a686590` | Progress glides forward only and snaps on any state change; the indeterminate sheen uses transform; rows ease in and out in place; large additions and Clear Finished do not animate; focus survives status changes. Measured in headless Edge against the dev mock (500 rows, 8 active: 0 long tasks). |
| Speed diagnosis | Pending affected-source evidence | No acceleration or throughput claim. |
| Replay metadata, speech recognition, lyrics | Deferred | No model downloads or source integrations. |
| Clip sections | Deferred to a future version | Not implemented; the plan keeps the design notes. |
| ProRes / editing presets, vocal removal, macOS | Excluded by the owner | Do not implement for this release. |

## Consolidated review fixes (2026-11-09, uncommitted source)

Implemented packages W1-W4, W5-W9, W10 and W11/R07 of the 2026-11-09 consolidated plan; see `docs/HANDOFF.md` for the per-package summary. Not implemented on purpose: C01, C02, C03, M4 and F6; **R20 deferred** because `core:window:allow-destroy` is still used by the close handler. This paragraph records the pre-rebuild implementation pass; the current 1.0.0 artifact includes these fixes.

Checks run 2026-11-09: **408 Rust tests**; Node tests fonts 7, queue 19, logic 152, motion 63; 2 Python font tests; production build; feature-enabled Clippy (warnings denied); `git diff --check`. Speed work (W0) is evidence-blocked: no affected source URL, so no speed improvement is claimed. Native drag, installed-app, real-site and upgrade acceptance remain manual and unexercised.

## Current source fixes (2026-11-08, after the candidate installer)

The historical 1.1.0 installer predates these fixes. The current 1.0.0 installer includes them; native acceptance remains outstanding.

- **Drag isolation.** Removed `drag` 2.1.1 and its additional Windows dependency stack. Claws creates the Shell data object and calls copy-only `DoDragDrop` on a dedicated OLE STA thread. A receiver stalled inside a COM call cannot block Claws' window thread. Only one drag worker is allowed; a receiver that never returns prevents further drags until it finishes or Claws restarts, rather than creating unlimited threads. Cooperative cancellation runs after 60 seconds; the command reports a stalled receiver after 65 seconds. An in-flight receiver call cannot safely be forcibly terminated.
- **Paths and results.** Fallible Shell parsing accepts normal long and UNC paths after removing filesystem-only prefixes. Missing files, symbolic links, reserved names and trailing dots/spaces are refused. OneDrive placeholders remain supported. A drop is reported only when OLE returns the copy effect; this still does not prove that an editor imported or finished copying a file. The physical primary mouse button respects swapped buttons.
- **Queue downgrade return.** Schema 2 is backed up before migration. After an old Claws sets a schema-3 queue aside, returning to this source recovers one unambiguous supported backup and retains jobs added during the downgrade. The backup is consumed only after durable publication, preventing resurrection after Clear. Multiple possible backups are preserved for manual selection. This does not teach 1.0.5 to execute schema-3 jobs.
- **Persistence errors.** An unreadable queue, failed migration backup, or failed set-aside now stops startup instead of silently proceeding with an empty writable queue. Backup names include a UUID to avoid same-second collisions. A native startup error dialog explains the problem and queue location even in a release build without a console.

Windows Shell tests exercise Unicode files, long-path parsing, fallible COM data-object creation, vanished files and path normalization. A reachable-loopback-share case runs only when that share is available. An isolated debug app was built and launched with generated media and its own profile; no owner data was used. Cross-window Explorer copying could not be exercised because the automation tool rejects drag coordinates outside the source window. Explorer/editor import and a deliberately hung receiver remain manual acceptance work.

### Manual drag checklist (Windows, disposable data)

Use a test output folder, not the owner's Library. After each step the Library file must still exist with an unchanged size.

1. Explorer: drag a Library item and a completed queue item into a folder; the copy opens and plays. Repeat onto the same folder (Explorer's replace/skip prompt appears; the source is untouched).
2. Editor: drag one video into an installed editor's timeline or media bin. Record editor name, version, container and codec, and whether it imported, separately from whether the drop happened.
3. Escape: start a drag, press Escape; nothing is copied, no message appears and the handle returns to normal.
4. Self-drop: release over Claws itself; nothing is copied, nothing is enqueued or opened, and the "drop it outside Claws" note appears.
5. Quick release: press the handle, move a few pixels and release immediately; no file is copied anywhere.
6. Missing file: delete or rename the file in Explorer, then drag; a clear "moved or deleted" message appears and nothing crashes.
7. Unicode, spaces and long paths: a title with non-Latin characters and spaces drags normally; a file whose full path exceeds 260 characters prepares safely and copies when the receiver supports the path.
8. Network destination: drag into a mapped network folder; the copy completes there. Network source: with the output folder on a reachable share, verify the file copies; an unavailable share reports an access error without freezing the window.
9. Keyboard: Tab to "Drag file", press Enter and Space; the file is shown in its folder. Open and Show in folder still work from the keyboard.
10. Left-handed mouse (buttons swapped): drag with the primary button. The native source checks the physical primary button; verify that the drag holds until release.
11. Receiver dialog: drop into an app that asks something before accepting (Explorer's replace prompt, an editor's import dialog), answer it with the cursor over Claws; no "drop it outside Claws" note appears.
12. Page state after a drag: after each of an Explorer drop, Escape and a self-drop, single-click another button (Open, a navigation tab) and drag the same handle again. Each responds to one click, nothing stays pressed or hovered, and the second drag starts normally.
13. Thumbnails: drag a Library thumbnail and a queue thumbnail into Explorer and an editor; nothing is exported or dropped.
14. Touch or pen (if available): a tap on "Drag file" shows the file in its folder; swiping on it does not start a drag.

## Remaining acceptance work

See section 1 of `docs/superpowers/plans/2026-11-07-feature-expansion-and-motion.md`: the manual drag checklist above, Windows 10 and a stalled-receiver check, a fresh install and an upgrade from 1.0.5, one real-site run of each download kind, a visual check of Reduce motion in the real window, and publication only with the owner's authorization.

## Checks

`npm ci`, `npm run test:fonts`, `npm run test:queue`, `npm run test:logic`, `npm run test:motion`, `python scripts/test_font_metrics.py`, `npm run build`, the fixture-tool build, `cargo test --locked`, feature-enabled Clippy with warnings denied, and `git diff --check`. The script suites pass 7, 11, 69, 37 and 2 tests respectively. The original candidate passed 227 backend tests; the subsequent source safeguards add queue preservation/recovery and native Shell regressions. Current verification results are recorded in the latest handoff section.

Native integration uses Windows Shell/OLE APIs behind an application command. The frontend supplies a Library ID, never an arbitrary path.
