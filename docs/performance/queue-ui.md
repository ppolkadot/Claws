# Queue view: main-thread cost

Measured with `node scripts/dev/queue-bench.mjs` (raw results in `queue-baseline-legacy.json` and `queue-incremental-v1.json`).

## Setup

- The real frontend (`npm run dev`, dev mock, no backend) ran in headless Microsoft Edge on the development PC (Windows 11, NVIDIA GeForce GT 340), at a 1160 × 760 window.
- For each case the page was reloaded, and the Queue view opened with the mock's 6 sample jobs.
- `N` jobs were then added. The legacy protocol sent one `job-update` per job; v1 sends one `jobs-added` batch.
- After that came 40 rounds of progress, each with one event per active transfer, a frame apart.
- Milliseconds are Chrome DevTools Protocol layout plus style-recalculation time. A long task is a main-thread task over 50 ms.
- These are single runs on one machine. They are evidence of the change's direction and size, not a promise for other hardware.

## Results

| Added jobs | Active | Add, before | Add, after | Progress (40 rounds), before | Progress, after | Long tasks during progress, before → after |
| ---: | ---: | ---: | ---: | ---: | ---: | --- |
| 1 | 1 | 4 ms | 2 ms | 42 ms | 34 ms | 0 → 0 |
| 100 | 8 | 233 ms | 18 ms | 193 ms | 48 ms | 0 → 0 |
| 500 | 1 | 4,418 ms | 74 ms | 760 ms | 47 ms | 6 → 0 |
| 500 | 8 | 4,370 ms | 73 ms | 729 ms | 63 ms | 39 → 0 |

After the change (v1, from the dev counters):

- Adding 500 jobs is one structural sync and 500 row paints. Before, every job triggered a full re-sort and repaint of the list.
- Each progress event repaints only its own row: 320 row paints for 8 × 40 events, with no structural syncs.
- Adding 500 jobs still produced one long task: building 500 rows in a single frame.

## Batched row creation (2026-11-05)

New rows are now built at most 40 per frame, also when the view opens. Measured with the updated `queue-bench.mjs`, which waits until every row exists and records the longest task; the "before" run used the previous `queue.ts` with the same benchmark (raw results: `queue-batched-v2.json`).

| Added jobs | Active | Long tasks while adding, before → after | Longest task, before → after | Rows |
| ---: | ---: | --- | --- | ---: |
| 100 | 1, 3, 8 | 0 → 0 | — | 106 |
| 500 | 1 | 1 → 0 | 109 ms → none | 506 |
| 500 | 3 | 1 → 0 | 109 ms → none | 506 |
| 500 | 8 | 1 → 0 | 117 ms → none | 506 |

Script time while adding 500 rises slightly (about 28 → 33–42 ms in total) because the work is split over more frames; progress updates are unchanged (≤ 0.016 ms per event, no long tasks).

## Pasted link batches (2026-11-07)

Measured with `node scripts/dev/batch-bench.mjs` on the same PC and headless Edge, against the dev mock with its lookup delays scaled to 2% (6 to 46 ms each). Every 25th link fails and every 50th is an 8-video playlist. Paste and Ctrl+Enter are separate tasks, as when typed. Three runs; raw results of the second are in `batch-links-v1.json`.

| Links | Long tasks: paste / lookups / add | Lookups: script, layout, style | Most lookups at once | Enqueue calls (items) |
| ---: | --- | --- | ---: | --- |
| 100 | 0 / 0 / 0 | 26–41, 54–60, 48–60 ms | 3 | 1 (110) |
| 500 | 0 / 0 / 0 | 140–169, 147–178, 74–83 ms | 3 | 2 (500 + 50) |

- Rows are built 40 per frame. Each answered lookup repaints its own row; the counts repaint at most once per frame.
- Two clicks on Download in one task sent each item once. A third press after everything was added sent nothing. The Queue view then showed 6 + 550 rows.
- Stop after 400 ms (30 links, unscaled mock delays): no lookup started after Stop, and the replies still running were ignored. All 30 rows showed as stopped.
- An earlier version animated the check's colours on every row as it became ready. Style recalculation for 500 links was then 2,730 ms in total, and rose as more rows were ready. Transform-only check feedback reduced this to 183–227 ms, and `content-visibility: auto` on the rows reduced it to 74–83 ms. Before these changes, adding 550 items at once caused one 55–115 ms long task in 4 of 5 runs; there were none in the 3 runs after.
- The existing `queue-bench.mjs` was re-run after this change: no long tasks while adding 1, 100 or 500 jobs, and at most 0.012 ms per progress event.
- These numbers come from the mock, not from real yt-dlp lookups, which take seconds each and are limited to three at a time.
- Disk: each video lookup saves yt-dlp's full answer (it can be megabytes, up to the 32 MB read limit) so the download can skip a second extraction. A batch writes one such file per video, so the cache is now capped at 256 MB in total as well as 20 minutes per file; the oldest files go first, after every write, one prune at a time. A download that starts after its file was evicted reads the site again; a file a running download already uses is held until that run ends, so the prune skips it. Not measured with real lookups.

## Final motion pass (2026-11-08)

Measured with `node scripts/dev/motion-bench.mjs` (each effect, with motion and with Reduce motion emulated over CDP) and `node scripts/dev/queue-bench.mjs` (500 rows, 8 active transfers). Both used headless Edge, the dev mock and the same PC as above. `--progress known` is new: the earlier runs never set `progressKnown`, so their rows showed the indeterminate sheen and never exercised the progress fill. "Before" is the `c8603b9` source run with the same benchmark scripts. These are single runs. Raw results are in `motion-effects-v1.json` (with per-frame samples) and `queue-motion-v3.json`.

**Does the queue reorder?** No. Jobs are ordered by creation time, then id. The reducer has no move event, and a status change, retry or Clear Finished never changes a job's place. There is therefore no reorder (FLIP) animation. The only movement was a defect. While a removed row collapsed, the ordering loop counted it as a list slot, so every row below it was re-inserted and the collapsing row jumped to the bottom of the list (index 67 of 68 in the bench; rapid removals put three rows at indexes 64–66). Leaving rows are now skipped (`src/ui/row-order.ts`): the row collapses in place, and the row below rises 0 → 62 px over the first 100 ms instead of jumping 72 px in the first frame.

### Effects: layout + style milliseconds over the measured window (script ms in brackets)

| Effect (window) | Motion, before | Motion, after | Reduced, before | Reduced, after |
| --- | ---: | ---: | ---: | ---: |
| Lookup settles in (700 ms) | 16.3 (1) | 15.6 (0.9) | 1.8 | 2.0 |
| Type indicator, one arrow key (600 ms) | 14.6 (0.8) | 14.3 (0.9) | 2.0 | 2.2 |
| Idle Queue, 1 s, two sheens on screen | 37.3 (0.8) | 14.1 (0.8) | 0.0 | 0.0 |
| 12 rows arrive (700 ms) | 59.8 (2.4) | 52.0 (2.3) | 2.8 | 2.9 |
| 50 rows arrive (700 ms) | 61.1 (6) | 17.6 (4.5) | 8.0 | 9.7 |
| One row removed (450 ms) | 36.7 (2) | 20.4 (0.8) | 2.7 | 2.4 |
| 20 rows removed at once (500 ms) | 47.8 (2.5) | 11.6 (1) | 1.5 | 1.6 |
| One progress step, 40 → 50% (800 ms) | 32.9 (0.5) | 16.0 (0.8) | 0.4 | 0.5 |
| Progress stream, 8 rows × 20 rounds (≈3.3 s) | 240.5 (0.9) | 90.0 (1.1) | 23.2 | 21.4 |
| Retry, failed 80% → 2% (700 ms) | 50.4 (0.8) | 17.5 (0.8) | 0.5 | 0.9 |
| Completion, 97% → saved (500 ms) | 28.1 (0.4) | 6.9 (0.2) | 1.3 | 0.9 |
| File drag pickup, 400 ms native call (600 ms) | 28.3 (0.5) | 7.2 (0.4) | 0.9 | 0.9 |

"After" is the run after the review fixes (focus guard, removal focus, detached-row cleanup, the global snap on a lower value). Every window had 0 long tasks and no frame slower than 25 ms, in both modes, before and after. The lookup settle and the Type indicator did not change; their differences are run-to-run variation (an earlier after-run measured 10.1 and 19.5 ms).

Most of the drop in the Queue rows comes from the indeterminate sheen. It used to animate `margin-left`, which re-laid out the page on every frame while any size was unknown (28.1 ms of layout per idle second before, 0 after). It now moves by `transform` along the same path: in the bench, the segment's left edge still travels from −34% to 100% of the track. With Reduce motion it is a static 34% segment, as before. Style recalculation per idle second was 9.2 ms before and 9.3–14.1 ms after (8–15 ms across runs).

### Examples (per-frame samples in the JSON)

- **Progress step 40 → 50%.** Unchanged, a forward glide: 0.40 at 0 ms, 0.44 at 100 ms, 0.49 at 300 ms, 0.50 at about 540 ms. With Reduce motion it is 0.50 in the next frame.
- **Retry (failed at 80%, then 2%).** Before, the fill slid backwards: 0.80, then 0.48 at 100 ms, 0.09 at 300 ms, 0.02 at 700 ms. After, it shows 0.02 in the first frame.
- **Completion at 97%.** Before, the "Saved" text and check appeared in the same task as the event, but the fill took about 440 ms to reach 1. After, the fill is at 1 in the first frame. The status line still makes its one 220 ms rise, and only on the change to completed.
- **50-row addition.** Before, the second frame's 10 rows animated (10 scripted animations after three frames). After, none do. A 12-row addition still animates all 12.
- **20 rows removed at once.** Before, 20 height animations ran together. After, the rows go at once; 12 or fewer still collapse.
- **First row into an empty Queue.** The list is hidden while empty, and the row used to be measured inside it at 0 px. Its arrival then ran from 0 to 0 px, holding it at 22 px (its padding) for the whole animation. It is now measured once the list is shown and grows 22 → 71 px over about 400 ms. With Reduce motion it is 72 px at once.
- **Hidden window** (`document.hidden` forced true). Before, 3 arriving rows animated and progress glided. After, 0 animate, progress snaps, and the check still appears.
- **Keyboard focus.** A focused Pause button's row changing to paused used to drop focus to `<body>`. Focus now moves to Resume, or keeps the same action if the new status still offers it. It never jumps on its own to Cancel or Remove.
- **Rapid presses** (trusted Enter key events over CDP; the commands stubbed to answer at once with the new status). Keeping focus on the row made a second Enter land on the opposite action. With the 600 ms press guard and the held-Enter filter turned off, Enter, Enter 60 ms apart on Pause sent `pause_job, resume_job` and the job ended downloading; on Cancel download it sent `cancel_job, resume_job` (Try again). A held Enter on Pause sent 13 alternating pause and resume commands. With the guard, each case sends exactly one command, the job stays paused or cancelled, and focus stays on Resume or Try again. Once the guard is over (700 ms later), Enter on Resume resumes as normal.
- **A late status after an earlier press** (`rapidPress.latePause`, added to `motion-bench.mjs` after the JSON above was recorded; results quoted from a later run of the same script). Enter on Resume (answered at once), then 1 s later Enter on Pause, whose "paused" arrives 1.2 s after its command answers, as when a running download's process exits slowly. Each press used to start a 1.5 s timer that cleared the row's focus target and guard even if a newer press had set them since. The Resume press's timer cleared the Pause press's, so when "paused" arrived focus fell to `<body>` (with and without Reduce motion). Each press now has its own number, and a timer only clears its own press's state: focus lands on Resume, a second Enter 60 ms later does nothing (`resume_job, pause_job` only, the job stays paused), in both runs.
- **Removing the focused row** ("Remove from list" by Enter). Without the fix, focus fell to `<body>`. Now it moves to the neighbouring row's first button that cannot cancel or remove ("Try again" in the bench), and a second Enter 60 ms later does nothing there. The collapsing row is `inert` and `aria-hidden`, so its buttons cannot be focused or pressed while it leaves.
- **Leaving the Queue mid-animation.** The bench counts every `Element.animate` call, because `document.getAnimations()` cannot see an animation whose element has left the document. Its earlier check used `getAnimations()` and so could not fail. Removing a row and switching pages within two frames, with the collapse's cancellation turned off, left 1 animation running on the detached row. With the fix, 0 run on detached rows and 0 anywhere 800 ms later, with no page errors.
- **Unchanged and verified:**
  - After four rapid lookups only the last one's details remain, with no stage animation left running, and focus stays in the link field.
  - Seven rapid arrow keys leave the segment thumb on the checked item, with focus on it.
  - The drag outline is on for exactly the native call and off after it.

### 500 queued rows, 8 active transfers (`queue-bench.mjs`)

| Progress | Add 500: long tasks, before → after | 40 progress rounds, layout + style ms, before → after | Long tasks during progress |
| --- | --- | --- | --- |
| Unknown size (as the earlier baseline) | 0 → 0 | 27.8 → 27.8 | 0 → 0 |
| Known size (`--progress known`) | 0 → 0 | 42.6 → 41.4, 27.6, 31.9 (three runs) | 0 → 0 |

"After" is the run after the review fixes; an earlier after-run measured 26.5 and 32.1 ms. The known-size case varies by about ±7 ms from run to run, so its before and after are not meaningfully different. Script time per progress event stayed at 0.001 ms; each event still repaints only its own row (320 row paints, no structural syncs). The earlier recorded baseline for this case was 63 ms of layout + style with 0 long tasks, so every run here is under it. The other sizes (1 and 100 rows; 1 and 3 active transfers) are in the JSON and also had no long tasks.
