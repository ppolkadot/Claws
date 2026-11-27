# Wallpaper Engine export tracker: screenshots

Every state of the export tracker and the recording options, photographed from the dev mock (`npm run dev`, synthetic data, headless Chrome over the DevTools port) with `node scripts/dev/workshop-shots.mjs <outDir> [state]`. The desktop is never captured. The numbers in the pictures are labelled samples made up by `src/mock.ts`, not measurements; measurements are in the [study](../../wallpaper-engine-local-library-study.md#implementation-results-2026-11-26).

File names are `<state>--<look>.png`. Every state is shot in `light` and `dark`. Copying, converting with a time left, saved and failed are also shot at Transparency 0 and 100 (`light-t0`, `light-t100`, `dark-t0`, `dark-t100`) and with Reduce motion on (`light-reduce-motion`, `dark-reduce-motion`).

| State | What it shows |
| --- | --- |
| `ready-copy-estimate` | An H.264 video: **MP4 (original)** and the estimate under the choice ("About 0:29 · about 5.45 GB") |
| `ready-convert-no-estimate` | A video that must be converted: **MP4 (converted)** and "Estimate after a few seconds" |
| `checking` | "Checking the wallpaper…" with the indeterminate fill |
| `copying` | "Copying 1.9 of 5.5 GB · about 5s left" |
| `finishing` | "Finishing the file…" holding the last value |
| `converting-no-time-left`, `converting-time-left` | "Converting 38%" without, then with, "about 3:10 left" |
| `zipping` | "Saving the ZIP 64% · about 12s left" |
| `saved` | The green bar and check: "Saved to Downloads\Claws · took 0:06", with the file's name and **Show in folder** |
| `failed` | The bar turns red where it stopped; "Didn't finish" and the message |
| `cancelled` | "Cancelled" with the standard fill |
| `scene-recording-options` | A scene: **MP4 (recorded)**, **Picture (PNG)**, **ZIP**, and the Length, Size and Frame rate rows |
| `scene-4k-may-not-fit` | 4K chosen: the note about a display that may not reach it |
| `scene-still-picture` | A scene that does not move: **Picture (PNG)** by default and "This wallpaper doesn't move." |
| `scene-sound-and-engine-starts` | The notes "This scene has sound. The recording is silent." and "Wallpaper Engine will start." |
| `scene-no-wallpaper-engine` | Recording is not offered; the card says why in one line |
| `recording-opening`, `recording-loading`, `recording-time-left`, `recording-finishing`, `recording-saved` | The stages of a recording: "Opening in Wallpaper Engine…", "Loading the scene…", "Recording 0:14 of 0:30 · 33.5 MB · 17s left", "Finishing the file…" and the result |
