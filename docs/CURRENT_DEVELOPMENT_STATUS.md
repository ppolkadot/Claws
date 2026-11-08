# Current development status

Reviewed 2026-11-08 against commit `6794e41` and the working tree. “Implemented” describes source behavior, not a newly published installer.

| Feature | Status | Evidence / next action |
| --- | --- | --- |
| Font safeguards | Committed: `99e2235` | Source and packaged integrity checks; preserve them. |
| Independent-link batches | Committed: `d9cc1bb` | Do not reimplement. |
| Download/temporary-space previews | Committed: `88f5f02` | Estimates remain distinct from admission checks. |
| Higher resolutions / Original format | Committed: `6794e41` | Source-driven choices; no upscaling. |
| Transcript export | Existing work in progress, uncommitted | Parser, output modes, publication, captions UI and tests already being developed. Leave this implementation intact. Review round 1 fixed repeated speech in human captions (roll-up removal only for rolling tracks), per-voice speaker attribution, literal `<` text, distinct file names per track/format, and SRT captions without FFmpeg. |
| Preview transitions / segmented selection / queue entry and exit / smooth progress | Already implemented | Reuse existing spring and reduced-motion paths. No replacement motion system needed. |
| Native outgoing drag | Working implementation, hardened 2026-11-08 | `drag` 2.1.1, Windows copy-only OLE drag on the UI thread. Library ids (equal to job ids) resolve the recorded regular file in Rust; frontend paths are never accepted. Locations the crate would crash on (paths over 260 characters, reserved names, files on a network share, a file that vanished) are refused with a message; links are refused, cloud-file placeholders are not. Drops on Claws itself (detected from Claws' own drop target, not the cursor position) and drags whose button was already released are reported as such. Mouse only: touch and pen taps show the file in its folder. Native/editor interoperability remains unverified (manual checklist below). |
| Completion feedback | New working implementation | Static completion check plus short cancellable animation on status transition only. Reduced motion and hidden windows skip it; progress ticks do not restart it. |
| Drag feedback | Working implementation | Grab cursor on the handle; active outline only while the native operation runs. No dropped/success notification: drag 2.1.1 discards the receiver's drop effect, so Claws cannot know whether a copy or import happened. A drop on Claws itself explains where to drop instead. Browser mock reports native dragging unavailable. |
| Speed diagnosis | Pending affected-source evidence | No new acceleration or throughput claim. |
| Replay metadata / speech recognition / lyrics discovery | Deferred by owner's core-only selection | No model downloads or source integrations added. |
| macOS / vocal removal / ProRes / clip sections | Excluded by owner | Do not implement. |

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

## Remaining core acceptance work

- Test real outgoing drag into Explorer and a target editor on Windows. Record editor version and codec compatibility separately from successful native file dropping. A build or fixture test does not prove timeline import.
- Run the manual drag checklist above; original media must stay intact.
- Before release, re-check `drag` upstream (issues #72, #76, #94) and test the drag on Windows 10; neither is covered by the review above.
- Re-run the existing 500-row / 8-active queue benchmark and visually verify completion/drag feedback with Reduce motion on and off. Existing large-batch animation limits remain in place.
- Finish the existing transcript work separately (review round 1 fixes are in the working tree, uncommitted).
- Package a fresh version only after integration acceptance; the published 1.0.5 installer does not contain the current working features.

## Checks

`npm run build`, `npm run test:logic`, `npm run test:queue`, `npm run test:fonts`, `npm run test:motion`, `python scripts/test_font_metrics.py`, fixture-tool build, backend tests, feature-enabled Clippy and `git diff --check`.

Native integration uses https://github.com/crabnebula-dev/drag-rs, pinned to 2.1.1. It calls the Rust API behind an application command, rather than exposing arbitrary file paths through a frontend plugin permission.
