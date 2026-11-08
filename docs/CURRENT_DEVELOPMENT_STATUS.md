# Current development status

Reviewed 2026-11-08 against branch `feature-expansion-and-motion` (version 1.1.0 source). "Implemented" describes source behaviour verified by automated tests and headless-browser measurements, not a published installer. The published release is still 1.0.5.

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

## Native drag review (2026-11-08)

Read from the `drag` 2.1.1 source (CrabNebula, Apache-2.0 OR MIT; its dev-dependencies target Tauri 2, tao 0.34 and wry 0.54, while its crate docs still say "tested for wry 0.24 and tauri v1"; used here with Tauri 2.12.5 on Windows 11 only — Windows 10 is untested):

- Maintenance, checked 2026-11-08 on GitHub and crates.io: 2.1.1 (published 2026-05-01, local source at commit `8b48f98`) is the latest release; the repository is not archived and its last commit is 2026-11-02 (dependency updates). Open upstream items that matter here: issue #76 (paths over 260 characters crash), issue #72 (network paths crash), PR #101 (Windows network shares, unmerged), issue #94 / PR #95 (a `DoDragDrop` that never returns wedges the app; optional timeout, unmerged) and PR #79 (error handling, unmerged). So the long-path and network refusals below stay until a fixed release, and a drag a receiver never finishes would keep Claws' window frozen (downloads keep running).

- `start_drag` runs OLE `DoDragDrop` synchronously on the calling thread; Claws calls it from an async command through `run_on_main_thread`, so it runs on the window's OLE thread in a fresh event-loop turn. The window stays unresponsive (downloads continue) until the drag ends.
- The data object is the Shell's own (CF_HDROP plus the Shell ID list); only DROPEFFECT_COPY is offered, so receivers cannot move the Library file. The crate's own drop source cancels on Escape and drops when the left button is up.
- The result callback runs before `start_drag` returns; "dropped" means the button was released, not that the receiver copied anything. The crate reads the cursor position after the drop and can return an error then, so a `start_drag` error is worded as "Windows reported a problem with the drag … check there before trying again", never "did not start".
- Self-drop: wry's drop target on Claws' webview posts `WindowEvent::DragDrop(Drop)` during the drop; `lib.rs` only records it (Claws never opens or enqueues a dropped file, and the page does not listen for drop events). After the drag returns, a second main-thread task reads the record; the event loop handles the posted drop first. A receiver that shows a dialog before accepting therefore no longer gets a wrong "drop it outside Claws" note. A release on the few pixels of Claws' window not covered by the webview is reported as an ordinary drop (no note).
- File checks: only "not found" is reported as moved or deleted; other errors (offline drive, no permission, a drive without a final path) name what Windows reported. `\\server\share` paths are refused before any file-system access, so an offline share cannot stall the request.
- Thumbnails: `h()` now writes `draggable="false"` (it used to drop the attribute, leaving only the CSS `-webkit-user-drag: none` rule).
- The crate re-canonicalizes paths with dunce and `unwrap`s the Shell parse. dunce keeps the verbatim `\\?\` form past 260 characters, for reserved names and for every network share (`\\?\UNC\`); the Shell cannot parse that form (checked on this machine, including `\\localhost\C$`), so the crate would panic and abort a release build. `file_drag.rs` refuses these, and a vanished file, with a specific message first. Dragging long-path and network-stored Library files is therefore not supported while Claws uses this crate; Show in folder remains available. Fixing it needs an upstream change or Claws' own `DoDragDrop` call (the non-verbatim path parses).
- Footprint: the crate adds 12 packages to Cargo.lock (10 compiled on Windows), including a third `windows` version (0.52, alongside 0.61 and 0.62). Measured release `claws.exe`: 6,119,424 bytes with the drag code, 6,107,648 bytes with it stubbed out (about 12 KB).
- A Library delete while Explorer is still copying (Shell copies can finish after the drag returns) either fails with the existing "close any app using it" message or, with delete-on-close semantics, removes the name while the copy completes from the open handle. It cannot leave a partial Library file.

### Manual drag checklist (Windows, disposable data)

Use a test output folder, not the owner's Library. After each step the Library file must still exist with an unchanged size.

1. Explorer: drag a Library item and a completed queue item into a folder; the copy opens and plays. Repeat onto the same folder (Explorer's replace/skip prompt appears; the source is untouched).
2. Editor: drag one video into an installed editor's timeline or media bin. Record editor name, version, container and codec, and whether it imported, separately from whether the drop happened.
3. Escape: start a drag, press Escape; nothing is copied, no message appears and the handle returns to normal.
4. Self-drop: release over Claws itself; nothing is copied, nothing is enqueued or opened, and the "drop it outside Claws" note appears.
5. Quick release: press the handle, move a few pixels and release immediately; no file is copied anywhere.
6. Missing file: delete or rename the file in Explorer, then drag; a clear "moved or deleted" message appears and nothing crashes.
7. Unicode, spaces and long paths: a title with non-Latin characters and spaces drags normally; a file whose full path exceeds 260 characters is refused with the "too long" message and Claws keeps running.
8. Network destination: drag into a mapped network folder; the copy completes there. Network source: with the output folder on a share, dragging shows the "network location" message and Claws keeps running.
9. Keyboard: Tab to "Drag file", press Enter and Space; the file is shown in its folder. Open and Show in folder still work from the keyboard.
10. Left-handed mouse (buttons swapped): drag with the primary button. The crate checks the left button state, so record whether the drag holds or drops at once.
11. Receiver dialog: drop into an app that asks something before accepting (Explorer's replace prompt, an editor's import dialog), answer it with the cursor over Claws; no "drop it outside Claws" note appears.
12. Page state after a drag: after each of an Explorer drop, Escape and a self-drop, single-click another button (Open, a navigation tab) and drag the same handle again. Each responds to one click, nothing stays pressed or hovered, and the second drag starts normally.
13. Thumbnails: drag a Library thumbnail and a queue thumbnail into Explorer and an editor; nothing is exported or dropped.
14. Touch or pen (if available): a tap on "Drag file" shows the file in its folder; swiping on it does not start a drag.

## Remaining acceptance work

See section 1 of `docs/superpowers/plans/2026-11-07-feature-expansion-and-motion.md`: the manual drag checklist above, Windows 10 and upstream re-checks for the `drag` crate, a fresh install and an upgrade from 1.0.5, one real-site run of each download kind, a visual check of Reduce motion in the real window, and publication only with the owner's authorization.

## Checks

`npm ci`, `npm run test:fonts`, `npm run test:queue`, `npm run test:logic`, `npm run test:motion`, `python scripts/test_font_metrics.py`, `npm run build`, the fixture-tool build, `cargo test --locked`, feature-enabled Clippy with warnings denied, and `git diff --check`. At 1.1.0: 7, 11, 69, 37 and 2 tests in the five script suites and 227 backend tests, all passing.

Native integration uses https://github.com/crabnebula-dev/drag-rs, pinned to 2.1.1. It calls the Rust API behind an application command, rather than exposing arbitrary file paths through a frontend plugin permission.
