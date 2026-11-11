# Motion polish (improvement plan phase 4)

**Date:** 2026-11-11 · **Raw results:** [`motion-2026-11-11/`](motion-2026-11-11/) · **Scripts:** `scripts/dev/shell-bench.mjs` (new), `motion-bench.mjs`, `queue-bench.mjs`

## What this is

Before and after numbers for five targeted motion changes. Everything ran in headless Chrome 1160×760 against the dev mock on one PC (Windows 11, Intel Core i5-12400F), one run each. Headless rendering has no GPU compositor like the real WebView2 window, so frame counts here are a comparison between before and after, not a prediction of the app. The glass materials and the native window were not measured, and nothing was looked at in the native app.

A frame is "slow" above 25 ms. A long task is above 50 ms. Each window is 700–900 ms after the trigger.

## 1. Navigation animates only the page

Which view-transition parts are animating two frames into a change of section:

| | Parts animating |
| --- | --- |
| Before | `group(root)`, `old(root)`, `new(root)`: the whole window crossfades |
| After | `group(page)`, `old(page)`, `new(page)`: only the page |

The sidebar and its indicator are now shown in their new state at once. Frame pacing did not improve, and was not expected to: with 506 queue rows the cost is building the rows.

| Navigation, 506 queue rows | Before | After |
| --- | --- | --- |
| To Library | 46 frames, 2 slow, 1 long task (69 ms) | 44 frames, 2 slow, 1 long task |
| To Queue | 58 frames, 5 slow, 1 long task (65 ms) | 51 frames, 9 slow, 1 long task |

The Queue figure is worse in this run (5 to 9 slow frames). One run each cannot say whether that is the change or noise; it is recorded as not improved. With the 6 sample rows there were no long tasks either way.

The plan named `.page` for the view-transition part. The scroller around it (`.scroll`) carries the name instead, so the captured image is bounded by the window however long the list in it.

## 2. Sidebar toggle

The plan asked for a benchmark first. It showed the cost: animating the grid column laid out the whole content pane on every frame.

| Sidebar over the Queue | Before | After |
| --- | --- | --- |
| Close, 6 rows | 61 frames, 0 slow, 24.2 ms layout | 61 frames, 0 slow, 13.1 ms layout |
| Open, 6 rows | 61 frames, 0 slow, 31.1 ms layout | 61 frames, 0 slow, 13.0 ms layout |
| Close, 506 rows | 38 frames, 6 slow, 54.2 ms layout | 52 frames, 1 slow, 16.0 ms layout, 1 long task (58 ms) |
| Open, 506 rows | 30 frames, 14 slow, 190.6 ms layout | 56 frames, 1 slow, 20.8 ms layout |

The column now changes at once and the page glides by transform. The one long task when closing is the single layout that replaces the per-frame ones.

**Tried and dropped:** `content-visibility: auto` on queue rows. It cut the sidebar's slow frames (14 to 6 when opening) and removed the navigation long tasks, but the queue bench then showed one long task of 58–68 ms during progress with 500 rows, in four runs out of four, against none in four runs without it. It was removed.

## 3. Library cards glide when the list changes

Typing in the search with 8 sample cards, 3 of which stay:

| | Cards moving | Frames | Slow |
| --- | ---: | ---: | ---: |
| Before | 0 (they jump) | 54 | 0 |
| After | 3 | 54 | 0 |

With Reduce motion: 0 moving, before and after. The 40-card cap was not exercised by the sample data.

## 4. Skeleton while a link is read

The box of what is shown while reading against the box of the details that replace it:

| | While reading | Top | Height | Shift when details arrive |
| --- | --- | ---: | ---: | --- |
| Before | spinner and text | 163 px | 178 px | 14 px down, 7 px taller |
| After | skeleton card | 177 px | 185 px | none |

## 5. One easing for a row leaving

`collapseOut` used a fixed `cubic-bezier` capped at 320 ms. It now uses the sampled smooth spring that `expandIn` uses (a unit test asserts the two easings and durations are equal). The collapse therefore takes about 0.7 s to come fully to rest, though most of the movement is over in the first third.

| Removing one row (motion bench) | Before | After |
| --- | --- | --- |
| Slow frames | 1 (longest 46 ms) | 1 (longest 34 ms); an earlier after-run had 0, so no change is claimed |
| Largest jump of the row below between two frames | not recorded in the before run | 5 px |
| Order of the rows afterwards, focus, rapid presses, disposal | pass | identical |

The motion bench's removal window was lengthened from 450 to 800 ms, and its lookup step now waits for the skeleton, to match these changes.

## Queue bench, unchanged

500 jobs with 1, 3 and 8 active transfers: 0 long tasks while adding and during progress, before and after.

## Not measured

- The native app: real WebView2 compositing, the glass materials, a real GPU.
- More than one run per configuration; small differences are within what a second run might reverse.
- Libraries of more than 40 entries, and what the cap looks like when it is crossed.
- How the sidebar's glide looks. It was checked for its end state and for frame pacing, not by eye.
