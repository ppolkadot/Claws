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
| Native outgoing drag | New working implementation | `drag` 2.1.1, Windows copy-only OLE operation on UI thread. Saved Library IDs resolve authoritative regular files; frontend paths are not accepted. Queue and Library drag controls; keyboard activation reveals the file. Native/editor interoperability remains unverified. |
| Completion feedback | New working implementation | Static completion check plus short cancellable animation on status transition only. Reduced motion and hidden windows skip it; progress ticks do not restart it. |
| Drag feedback | New working implementation | Active outline/cursor only during native operation; no optimistic dropped/success notification. Browser mock reports native dragging unavailable. |
| Speed diagnosis | Pending affected-source evidence | No new acceleration or throughput claim. |
| Replay metadata / speech recognition / lyrics discovery | Deferred by owner's core-only selection | No model downloads or source integrations added. |
| macOS / vocal removal / ProRes / clip sections | Excluded by owner | Do not implement. |

## Remaining core acceptance work

- Test real outgoing drag into Explorer and a target editor on Windows. Record editor version and codec compatibility separately from successful native file dropping. A build or fixture test does not prove timeline import.
- Exercise a cancelled drag (Escape), missing published files, repeated drags, Unicode and long paths; original media must stay intact.
- Re-run the existing 500-row / 8-active queue benchmark and visually verify completion/drag feedback with Reduce motion on and off. Existing large-batch animation limits remain in place.
- Finish the existing transcript work separately (review round 1 fixes are in the working tree, uncommitted).
- Package a fresh version only after integration acceptance; the published 1.0.5 installer does not contain the current working features.

## Checks

`npm run build`, `npm run test:logic`, `npm run test:queue`, `npm run test:fonts`, `npm run test:motion`, `python scripts/test_font_metrics.py`, fixture-tool build, backend tests, feature-enabled Clippy and `git diff --check`.

Native integration uses https://github.com/crabnebula-dev/drag-rs, pinned to 2.1.1. It calls the Rust API behind an application command, rather than exposing arbitrary file paths through a frontend plugin permission.
