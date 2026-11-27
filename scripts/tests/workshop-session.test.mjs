import assert from "node:assert/strict";
import test from "node:test";

import {
  WorkshopSession,
  downloadFraction,
  formatChoices,
  BAR_CAP,
  barFraction,
  estimateLine,
  etaText,
  libraryLine,
  longRunMessage,
  lookupProblem,
  missingForStart,
  offersVideo,
  recordFps,
  recordNotes,
  recordSize,
  savedLine,
  stageFraction,
  stageLine,
  tookText,
} from "../../.test-build/workshop-session.js";
import { isWorkshopLink } from "../../.test-build/workshop-link.js";

const URL_A = "https://steamcommunity.com/sharedfiles/filedetails/?id=123";
const item = { id: "123", title: "Ocean", previewUrl: null, fileSize: 10, timeUpdated: null, wallpaperType: "Scene", requiredItems: [] };

/** A library status as the backend sends it. */
function library(overrides = {}) {
  return { steam: true, wallpaperEngine: true, found: false, video: false, kind: "unknown", state: "notInstalled", ...overrides };
}

function deps(overrides = {}) {
  const calls = [];
  return {
    calls,
    deps: {
      download: async (...args) => {
        calls.push(["download", ...args]);
        return { status: "saved", filename: "Ocean (123).zip", path: "C:\\out\\Ocean (123).zip", elapsedMs: 1000, stages: [] };
      },
      cancel: async () => void calls.push(["cancel"]),
      findInLibrary: async () => library(),
      videoInfo: async () => null,
      estimate: async () => ({ seconds: null, approximate: true, bytes: null }),
      recordInfo: async () => null,
      describeError: (error) => String(error?.message ?? error),
      ...overrides,
    },
  };
}

test("only single Steam Workshop item links count", () => {
  assert.ok(isWorkshopLink(URL_A));
  assert.ok(isWorkshopLink("https://www.steamcommunity.com/workshop/filedetails?id=9&searchtext=x"));
  for (const bad of [
    "https://steamcommunity.com.evil.test/sharedfiles/filedetails/?id=1",
    "https://steamcommunity.com/sharedfiles/filedetails/?id=1&id=2",
    "https://steamcommunity.com/workshop/browse/?appid=431960",
    "https://steamcommunity.com/sharedfiles/filedetails/?id=0",
    "https://steamcommunity.com/sharedfiles/filedetails/?id=18446744073709551616",
    "https://user@steamcommunity.com/sharedfiles/filedetails/?id=1",
    "not a link",
  ]) {
    assert.equal(isWorkshopLink(bad), false, bad);
  }
});

test("lookups that are not wallpapers explain why", () => {
  assert.equal(lookupProblem({ kind: "wallpaper", item }), null);
  assert.match(lookupProblem({ kind: "collection", title: "x", items: 3 }), /collection/);
  assert.match(lookupProblem({ kind: "otherApp", appId: 480 }), /Wallpaper Engine only/);
  assert.match(lookupProblem({ kind: "notFound" }), /couldn't find/);
});

test("the export says what is missing before it can start", () => {
  assert.equal(missingForStart(library({ steam: false })), "Steam isn't installed for this Windows user.");
  assert.equal(missingForStart(library()), "Subscribe in Steam first.");
  assert.equal(missingForStart(null), "Subscribe in Steam first.");
  assert.equal(missingForStart(library({ found: true, state: "installed" })), null);
  assert.equal(missingForStart(library({ found: true, state: "updatePending" })), null, "an update waiting in Steam does not stop an export");
  assert.equal(missingForStart(library({ found: true, state: "downloading", bytes: 10 })), "Steam is still downloading it.");
});

test("an export sends the link and the chosen format, from the Steam library only", async () => {
  const { calls, deps: d } = deps({ findInLibrary: async () => library({ found: true, video: true, state: "installed" }) });
  const session = new WorkshopSession(URL_A, item, d);
  await session.checkLibrary();
  assert.equal(session.format, "mp4");
  session.setFormat("mp4");
  await session.start();
  assert.deepEqual(calls[0], ["download", URL_A, "mp4", {}]);
  assert.deepEqual(session.state, { phase: "saved", filename: "Ocean (123).zip", path: "C:\\out\\Ocean (123).zip", elapsedMs: 1000, stages: [], note: null });
  assert.equal("source" in session, false, "there is no other source");
});

test("only video wallpapers offer a video format", () => {
  assert.equal(offersVideo(item), false);
  assert.equal(offersVideo({ ...item, wallpaperType: "Video" }), true);
  assert.equal(offersVideo({ ...item, wallpaperType: "VIDEO" }), true);
  assert.equal(offersVideo({ ...item, wallpaperType: "Scene" }), false);
  assert.equal(offersVideo({ ...item, wallpaperType: null }), false);
});

test("the installed project controls library export even when Workshop tags are wrong", async () => {
  let video = true;
  const { calls, deps: d } = deps({
    findInLibrary: async () => library({ found: true, video, state: "installed" }),
  });
  const session = new WorkshopSession(URL_A, item, d);
  await session.checkLibrary();
  assert.equal(session.videoAvailable, true);
  assert.equal(session.format, "mp4");
  session.setFormat("gif");
  await session.start();
  assert.deepEqual(calls[0], ["download", URL_A, "gif", { lengthSeconds: 30 }]);

  video = false;
  await session.checkLibrary();
  assert.equal(session.videoAvailable, false);
  assert.equal(session.format, "zip");
});

test("an unsupported video export offers the ZIP fallback and clears it on a format change", async () => {
  const { calls, deps: d } = deps({
    findInLibrary: async () => library({ found: true, video: true, state: "installed" }),
    download: async (...args) => {
      calls.push(["download", ...args]);
      return { status: "unsupported", message: "No video here." };
    },
  });
  const session = new WorkshopSession(URL_A, { ...item, wallpaperType: "Video" }, d);
  await session.checkLibrary();
  session.setFormat("gif");
  await session.start();
  assert.deepEqual(session.state, { phase: "unsupported", message: "No video here." });
  assert.deepEqual(calls[0], ["download", URL_A, "gif", { lengthSeconds: 30 }]);
  session.setFormat("zip");
  assert.deepEqual(session.state, { phase: "ready" });
  assert.equal(session.format, "zip");
});

/** A progress event as the backend sends it. */
function event(overrides = {}) {
  return { url: URL_A, stage: "checking", done: null, total: null, unit: null, etaMs: null, approximate: false, elapsedMs: 0, ...overrides };
}

test("progress applies only to its own running export", async () => {
  let finish;
  const { deps: d } = deps({ download: () => new Promise((resolve) => (finish = resolve)) });
  const session = new WorkshopSession(URL_A, item, d);
  session.progress(event({ stage: "converting" }));
  assert.equal(session.state.phase, "ready", "nothing is running yet");
  const done = session.start();
  assert.ok(session.running);
  assert.equal(session.state.progress.stage, "checking", "an export starts by checking");
  session.progress(event({ url: "https://steamcommunity.com/sharedfiles/filedetails/?id=999", stage: "zipping", done: 5, total: 10, unit: "bytes" }));
  assert.equal(session.state.progress.stage, "checking");
  session.progress(event({ stage: "zipping", done: 5, total: 10, unit: "bytes", etaMs: 4000, approximate: true }));
  assert.equal(session.state.progress.stage, "zipping");
  assert.equal(session.state.fraction, 0.5);
  finish({ status: "failed", message: "Steam went away.", detail: "tail" });
  await done;
  assert.deepEqual(session.state, { phase: "failed", message: "Steam went away.", detail: "tail" });
  assert.equal(session.lastFraction, 0.5, "the bar turns red where it stopped");
});

test("a saved export keeps its total time and the time in each stage", async () => {
  const stages = [{ name: "checking", ms: 400 }, { name: "copying", ms: 9600 }, { name: "finishing", ms: 2300 }];
  const { deps: d } = deps({ download: async () => ({ status: "saved", filename: "A (1).mp4", path: "C:\\out\\A (1).mp4", elapsedMs: 12300, stages }) });
  const session = new WorkshopSession(URL_A, item, d);
  await session.start();
  assert.deepEqual(session.state, { phase: "saved", filename: "A (1).mp4", path: "C:\\out\\A (1).mp4", elapsedMs: 12300, stages, note: null });
});

test("the taskbar is told what a running export counts", async () => {
  let finish;
  const { deps: d } = deps({ download: () => new Promise((resolve) => (finish = resolve)) });
  const session = new WorkshopSession(URL_A, item, d);
  assert.deepEqual(session.activity, { running: false, fraction: null });
  const done = session.start();
  assert.deepEqual(session.activity, { running: true, fraction: null }, "checking counts nothing");
  session.progress(event({ stage: "copying", done: 25, total: 100, unit: "bytes" }));
  assert.deepEqual(session.activity, { running: true, fraction: 0.25 });
  finish({ status: "cancelled" });
  await done;
  assert.deepEqual(session.activity, { running: false, fraction: null });
});

test("the bar never goes backwards, never passes its cap and holds while finishing", () => {
  const at = (previous, overrides) => barFraction(previous, event(overrides));
  assert.equal(at(null, { stage: "checking" }), null);
  assert.equal(at(null, { stage: "opening" }), null);
  assert.equal(at(null, { stage: "loading" }), null);
  assert.equal(at(null, { stage: "recording", done: 0, total: 60, unit: "seconds" }), 0);
  assert.equal(at(0, { stage: "recording", done: 30, total: 60, unit: "seconds" }), 0.5);
  assert.equal(at(0.5, { stage: "recording", done: 10, total: 60, unit: "seconds" }), 0.5, "a lower reading is ignored");
  assert.equal(at(0.5, { stage: "recording", done: 60, total: 60, unit: "seconds" }), BAR_CAP, "100% is for the saved file");
  assert.equal(at(0.5, { stage: "copying", done: 200, total: 100, unit: "bytes" }), BAR_CAP);
  assert.equal(at(0.9, { stage: "finishing" }), 0.9, "finishing holds the last value");
  assert.equal(at(null, { stage: "finishing" }), null, "and the sheen if there was none");
  assert.equal(at(0.4, { stage: "converting", done: null, total: null }), null, "a stage with nothing to count shows the sheen");
  assert.equal(at(0, { stage: "converting", done: 5, total: 0, unit: "seconds" }), null, "no length, no fraction");
  assert.equal(stageFraction(event({ stage: "finishing", done: 1, total: 1 })), null);
  assert.equal(stageFraction(event({ stage: "copying", done: -5, total: 10 })), 0);
});

test("the status line says what is happening, in the Queue's style", () => {
  const line = (overrides) => stageLine(event(overrides));
  assert.equal(line({ stage: "checking" }), "Checking the wallpaper…");
  assert.equal(line({ stage: "opening" }), "Opening in Wallpaper Engine…");
  assert.equal(line({ stage: "loading" }), "Loading the scene…");
  assert.equal(line({ stage: "finishing" }), "Finishing the file…");
  assert.equal(
    line({ stage: "recording", done: 42, total: 60, unit: "seconds", etaMs: 18_000, bytes: 163_577_856 }),
    "Recording 0:42 of 1:00  ·  156 MB  ·  18s left",
  );
  assert.equal(line({ stage: "recording", done: 0, total: 60, unit: "seconds", etaMs: 60_000 }), "Recording 0:00 of 1:00  ·  1:00 left", "an exact time left has no 'about'");
  assert.equal(line({ stage: "recording" }), "Recording…");
  assert.equal(
    line({ stage: "copying", done: 2_791_728_742, total: 5_856_547_590, unit: "bytes", etaMs: 4_000, approximate: true }),
    "Copying 2.6 of 5.5 GB  ·  about 4s left",
  );
  assert.equal(line({ stage: "copying", done: 0, total: 5_856_547_590, unit: "bytes" }), "Copying 0.0 of 5.5 GB");
  assert.equal(line({ stage: "converting", done: 111.9, total: 294.35, unit: "seconds", etaMs: 190_000, approximate: true }), "Converting 38%  ·  about 3:10 left");
  assert.equal(line({ stage: "converting", done: 1, total: 294.35, unit: "seconds" }), "Converting 0%", "no time left until it can be backed");
  assert.equal(line({ stage: "converting" }), "Converting…");
  assert.equal(line({ stage: "zipping", done: 16, total: 25, unit: "bytes", etaMs: 12_000, approximate: true }), "Saving the ZIP 64%  ·  about 12s left");
  assert.equal(line({ stage: "zipping" }), "Saving the ZIP…");
  // Parts are joined by two spaces, a middle dot and two spaces, as in Queue rows.
  assert.ok(line({ stage: "converting", done: 50, total: 100, etaMs: 5000, approximate: true }).includes("  ·  "));
});

test("the time left is 'about' only when it rests on a measured rate", () => {
  assert.equal(etaText(null, true), "");
  assert.equal(etaText(4_000, true), "about 4s left");
  assert.equal(etaText(4_000, false), "4s left");
  assert.equal(etaText(190_000, true), "about 3:10 left");
  assert.equal(etaText(0, false), "1s left", "never '0s left'");
  assert.equal(etaText(-5, false), "");
  assert.equal(etaText(Number.NaN, false), "");
});

test("a finished export says where it went and how long it took", () => {
  assert.equal(savedLine("C:\\Users\\Sam\\Videos\\Ocean (1).mp4", 63_000), "Saved to Sam\\Videos  ·  took 1:03");
  assert.equal(savedLine("D:\\Wallpapers\\Ocean (1).zip", 400), "Saved to D:\\Wallpapers  ·  took 0:01", "never less than a second");
  assert.equal(savedLine("C:\\Videos\\x.mp4", 3_723_000), "Saved to C:\\Videos  ·  took 1:02:03");
  assert.equal(tookText(62_500), "took 1:03");
});

test("the estimate before starting is a time and a size, or an honest 'after a few seconds'", () => {
  assert.equal(estimateLine(null), "");
  assert.equal(estimateLine({ seconds: null, approximate: true, bytes: null }), "Estimate after a few seconds");
  assert.equal(estimateLine({ seconds: 64, approximate: false, bytes: 241_172_480 }), "About 1:04  ·  about 230 MB");
  assert.equal(estimateLine({ seconds: 0.2, approximate: true, bytes: null }), "About 0:01");
  assert.equal(estimateLine({ seconds: 29.3, approximate: true, bytes: 5_856_547_590 }), "About 0:29  ·  about 5.45 GB");
});

test("only a long estimate asks for confirmation first", () => {
  assert.equal(longRunMessage({ seconds: null, approximate: true, bytes: null }), null);
  assert.equal(longRunMessage({ seconds: 600, approximate: true, bytes: null }), null, "ten minutes exactly is not over ten minutes");
  assert.match(longRunMessage({ seconds: 4_680, approximate: true, bytes: null }), /about 1:18:00/);
});

test("the estimate follows the choice, and a stale answer is dropped", async () => {
  const asked = [];
  const resolvers = [];
  const { deps: d } = deps({
    findInLibrary: async () => library({ found: true, video: true, state: "installed" }),
    estimate: (url, format, options) => new Promise((resolve) => {
      asked.push([format, options]);
      resolvers.push(resolve);
    }),
  });
  const session = new WorkshopSession(URL_A, item, d);
  await session.checkLibrary();
  assert.deepEqual(asked, [["mp4", {}]], "asked once the library has the video");
  session.setFormat("gif");
  assert.deepEqual(asked[1], ["gif", { lengthSeconds: 30 }]);
  // The answer for MP4 arrives after the user chose GIF: it is not shown.
  resolvers[0]({ seconds: 1, approximate: true, bytes: null });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(session.estimate, null);
  resolvers[1]({ seconds: 99, approximate: true, bytes: null });
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(session.estimate, { seconds: 99, approximate: true, bytes: null });
  session.setGifSeconds(60);
  assert.equal(session.estimate, null, "a new choice clears the old answer");
  assert.deepEqual(asked[2], ["gif", { lengthSeconds: 60 }]);
});

test("cancel asks the backend only while running; errors become failures", async () => {
  const { calls, deps: d } = deps({ download: async () => { throw new Error("A wallpaper download is already running."); } });
  const session = new WorkshopSession(URL_A, item, d);
  session.cancel();
  assert.equal(calls.length, 0);
  await session.start();
  assert.deepEqual(session.state, { phase: "failed", message: "A wallpaper download is already running.", detail: null });
});

test("the library is checked until Steam has the subscribed item", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout", "Date"] });
  let found = false;
  const { deps: d } = deps({ findInLibrary: async () => library({ found, state: found ? "installed" : "notInstalled" }) });
  const session = new WorkshopSession(URL_A, item, d);
  await session.checkLibrary();
  session.watchLibrary(Date.now());
  assert.ok(session.watching);
  t.mock.timers.tick(3000);
  await new Promise((resolve) => setImmediate(resolve));
  assert.ok(session.watching, "not there yet");
  found = true;
  t.mock.timers.tick(3000);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(session.watching, false);
  assert.deepEqual(session.library, library({ found: true, state: "installed" }));
  session.dispose();
});

test("what Steam says about the item becomes one honest line", () => {
  const size = 31_000_000;
  assert.equal(libraryLine(null, size, false), "Checking…");
  assert.match(libraryLine(library({ steam: false }), size, false), /Steam with Wallpaper Engine installed on this PC, and nothing else/);
  assert.match(libraryLine(library({ wallpaperEngine: false }), size, false), /Wallpaper Engine isn't installed in your Steam library/);
  assert.equal(libraryLine(library(), size, false), "Not in your library yet.");
  assert.equal(libraryLine(library(), size, true), "Waiting for Steam to download it…");
  assert.equal(libraryLine(library({ found: true, state: "installed", video: true }), size, false), "Video found in your Steam library.");
  assert.match(libraryLine(library({ found: true, state: "installed" }), size, false), /^Project found\./);
  assert.match(libraryLine(library({ found: true, state: "updatePending" }), size, false), /update for this wallpaper.*version you have/);
  assert.equal(libraryLine(library({ state: "downloading", bytes: 15_500_000 }), size, true), "Steam is downloading it… 50%");
  assert.equal(libraryLine(library({ state: "downloading", bytes: 1 }), null, true), "Steam is downloading it…", "no size to go by, no percentage");
  assert.equal(libraryLine(library({ wallpaperEngine: false, state: "downloading", bytes: 1 }), size, true).startsWith("Steam is downloading"), true);
});

test("the download percentage never reaches 100 before Steam is done and never goes below 0", () => {
  const d = (bytes) => library({ state: "downloading", bytes });
  assert.equal(downloadFraction(d(0), 100), 0);
  assert.equal(downloadFraction(d(50), 100), 0.5);
  assert.equal(downloadFraction(d(400), 100), 0.99, "Valve's size is not exact");
  assert.equal(downloadFraction(d(5), 0), null);
  assert.equal(downloadFraction(d(5), null), null);
  assert.equal(downloadFraction(library({ found: true, state: "installed" }), 100), null);
  assert.equal(downloadFraction(null, 100), null);
});

test("MP4 says whether it saves the original or converts, and only video offers more than a ZIP", () => {
  const copyable = { copyable: true, codec: "h264", width: 3840, height: 2160, fps: 60, durationSeconds: 294, bytes: 5, hasAudio: false };
  const offer = (video, videoInfo, record = null) => ({ video, videoInfo, record });
  assert.deepEqual(formatChoices(offer(false, null)).map((c) => c.label), ["ZIP"]);
  assert.deepEqual(formatChoices(offer(true, copyable)).map((c) => c.label), ["ZIP", "MP4 (original)", "GIF"]);
  assert.deepEqual(formatChoices(offer(true, { ...copyable, copyable: false, codec: "hevc" })).map((c) => c.label), ["ZIP", "MP4 (converted)", "GIF"]);
  assert.deepEqual(formatChoices(offer(true, null)).map((c) => c.label), ["ZIP", "MP4 (converted)", "GIF"], "unknown means it will convert");
  assert.deepEqual(formatChoices(offer(true, copyable)).map((c) => c.value), ["zip", "mp4", "gif"]);
});

test("a GIF asks for 30 seconds unless the user chose 60", async () => {
  const { calls, deps: d } = deps({ findInLibrary: async () => library({ found: true, video: true, state: "installed" }) });
  const session = new WorkshopSession(URL_A, item, d);
  await session.checkLibrary();
  session.setFormat("gif");
  session.setGifSeconds(45);
  assert.equal(session.gifSeconds, 30, "only the offered lengths");
  session.setGifSeconds(60);
  await session.start();
  assert.deepEqual(calls[0], ["download", URL_A, "gif", { lengthSeconds: 60 }]);
  session.setFormat("mp4");
  await session.start();
  assert.deepEqual(calls[1], ["download", URL_A, "mp4", {}], "no length for an MP4");
});

test("the video's details are read once, when the library has a video", async () => {
  let reads = 0;
  const info = { copyable: true, codec: "h264", width: 1920, height: 1080, fps: 30, durationSeconds: 60, bytes: 10, hasAudio: true };
  const { deps: d } = deps({
    findInLibrary: async () => library({ found: true, video: true, state: "installed" }),
    videoInfo: async () => (reads++, info),
  });
  const session = new WorkshopSession(URL_A, item, d);
  await session.checkLibrary();
  await session.checkLibrary();
  assert.equal(reads, 1);
  assert.deepEqual(session.video, info);
});

test("waiting for Steam goes on while it is still writing the item", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout", "Date"] });
  let status = library({ found: true, state: "downloading", bytes: 10 });
  const { deps: d } = deps({ findInLibrary: async () => status });
  const session = new WorkshopSession(URL_A, item, d);
  await session.checkLibrary();
  session.watchLibrary(Date.now());
  t.mock.timers.tick(3000);
  await new Promise((resolve) => setImmediate(resolve));
  assert.ok(session.watching, "the folder exists, but Steam has not finished");
  status = library({ found: true, state: "installed" });
  t.mock.timers.tick(3000);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(session.watching, false);
  session.dispose();
});

// ----- recording a scene or web wallpaper -----

const size = (height, extra = {}) => ({ width: Math.round((height * 16) / 9 / 2) * 2, height, label: height === 2160 ? "4K" : `${height}p`, available: true, reason: null, beyondDisplay: false, fps: [30, 60], ...extra });
const recordInfo = (extra = {}) => ({
  kind: "scene", available: true, reason: null, moves: true, hasAudio: false,
  sizes: [size(720), size(1080), size(1440, { beyondDisplay: true }), size(2160, { beyondDisplay: true })],
  defaultHeight: 1080, preRollSeconds: 2, encoder: "h264_nvenc", engineWillStart: false, displayLimit: [3860, 1106], ...extra,
});

test("a scene offers a recording, a picture and a ZIP; one that cannot be recorded offers only the ZIP", () => {
  const offer = (record) => ({ video: false, videoInfo: null, record });
  assert.deepEqual(formatChoices(offer(recordInfo())).map((c) => c.label), ["MP4 (recorded)", "Picture (PNG)", "ZIP"]);
  assert.deepEqual(formatChoices(offer(recordInfo())).map((c) => c.value), ["mp4", "png", "zip"]);
  assert.deepEqual(formatChoices(offer(recordInfo({ available: false, reason: "Recording needs Wallpaper Engine." }))).map((c) => c.label), ["ZIP"]);
  assert.deepEqual(formatChoices(offer(null)).map((c) => c.label), ["ZIP"]);
});

test("a moving scene defaults to a recording and a still one to a picture", async () => {
  const sceneLibrary = library({ found: true, kind: "scene", state: "installed" });
  const moving = new WorkshopSession(URL_A, item, deps({ findInLibrary: async () => sceneLibrary, recordInfo: async () => recordInfo() }).deps);
  await moving.checkLibrary();
  assert.equal(moving.format, "mp4");
  assert.ok(moving.recording);
  const still = new WorkshopSession(URL_A, item, deps({ findInLibrary: async () => sceneLibrary, recordInfo: async () => recordInfo({ moves: false }) }).deps);
  await still.checkLibrary();
  assert.equal(still.format, "png", "This wallpaper does not move");
  still.setFormat("mp4");
  assert.equal(still.format, "mp4", "the user can still ask for a recording");
  const web = new WorkshopSession(URL_A, item, deps({ findInLibrary: async () => library({ found: true, kind: "web", state: "installed" }), recordInfo: async () => recordInfo({ kind: "web", moves: null }) }).deps);
  await web.checkLibrary();
  assert.equal(web.format, "mp4", "a web page is assumed to move");
  const none = new WorkshopSession(URL_A, item, deps({ findInLibrary: async () => sceneLibrary }).deps);
  await none.checkLibrary();
  assert.equal(none.format, "zip");
  assert.equal(none.recording, false);
});

test("the size is the default until a size that is available is chosen, and the rate follows the size", async () => {
  const sizes = [size(720), size(1080, { fps: [30] }), size(1440, { available: false, reason: "no encoder", fps: [] })];
  const { calls, deps: d } = deps({ findInLibrary: async () => library({ found: true, kind: "scene", state: "installed" }), recordInfo: async () => recordInfo({ sizes }) });
  const session = new WorkshopSession(URL_A, item, d);
  await session.checkLibrary();
  assert.equal(session.size.height, 1080);
  assert.equal(session.fps, 30);
  session.setRecordHeight(1440);
  assert.equal(session.size.height, 1080, "a size that is not available is refused");
  session.setRecordHeight(999);
  assert.equal(session.size.height, 1080, "so is one that does not exist");
  session.setRecordFps(60);
  assert.equal(session.fps, 30, "1080p allows only 30 here");
  session.setRecordHeight(720);
  session.setRecordFps(60);
  assert.deepEqual([session.size.height, session.fps], [720, 60]);
  session.setRecordHeight(1080);
  assert.equal(session.fps, 30, "back to a size that cannot do 60: 30 again");
  session.setRecordHeight(720);
  session.setRecordLength(90, true);
  await session.start();
  assert.deepEqual(calls.at(-1), ["download", URL_A, "mp4", { lengthSeconds: 90, height: 720, fps: 60 }]);
  session.setFormat("png");
  await session.start();
  assert.deepEqual(calls.at(-1), ["download", URL_A, "png", { height: 720 }], "a picture has a size and nothing else");
});

test("a recording's length is a whole number of seconds from one to ten minutes", async () => {
  const { deps: d } = deps({ findInLibrary: async () => library({ found: true, kind: "scene", state: "installed" }), recordInfo: async () => recordInfo() });
  const session = new WorkshopSession(URL_A, item, d);
  await session.checkLibrary();
  assert.equal(session.recordLength, 30, "the default");
  session.setRecordLength(10);
  assert.deepEqual([session.recordLength, session.lengthCustom], [10, false]);
  session.setRecordLength(0, true);
  assert.equal(session.recordLength, 1);
  session.setRecordLength(100000, true);
  assert.equal(session.recordLength, 600, "ten minutes at most");
  session.setRecordLength(42.6, true);
  assert.equal(session.recordLength, 43);
  session.setRecordLength(Number.NaN, true);
  assert.equal(session.recordLength, 43, "not a number: unchanged");
  session.setLengthCustom(false);
  assert.equal(session.lengthCustom, false);
});

test("the notes say what a recording will and will not do", () => {
  const big = size(2160, { beyondDisplay: true });
  assert.deepEqual(recordNotes(recordInfo(), "mp4", size(1080)), []);
  assert.deepEqual(recordNotes(recordInfo({ moves: false }), "png", size(1080)), ["This wallpaper doesn't move."]);
  assert.match(recordNotes(recordInfo({ moves: false }), "mp4", size(1080))[0], /would be a still video/);
  assert.deepEqual(recordNotes(recordInfo({ hasAudio: true }), "mp4", size(1080)), ["This scene has sound. The recording is silent."]);
  assert.deepEqual(recordNotes(recordInfo({ engineWillStart: true }), "mp4", size(1080)), ["Wallpaper Engine will start."]);
  const beyond = recordNotes(recordInfo(), "mp4", big);
  assert.equal(beyond.length, 1);
  assert.ok(beyond[0].startsWith("4K may need a display set to 3840×2160 or larger, such as a 4K monitor or NVIDIA DSR / AMD VSR"), beyond[0]);
  assert.equal(recordSize(null, 1080), null);
  assert.equal(recordSize(recordInfo(), 720).height, 720);
  assert.equal(recordSize(recordInfo(), null).height, 1080);
  assert.equal(recordFps(size(720), 60), 60);
  assert.equal(recordFps(size(720, { fps: [30] }), 60), 30);
  assert.equal(recordFps(null, 60), 30);
});

test("a scene that cannot be recorded says why in the library line", () => {
  assert.equal(libraryLine(library({ found: true, state: "installed", kind: "scene" }), 1, false), "Scene found in your Steam library.");
  assert.equal(libraryLine(library({ found: true, state: "installed", kind: "web" }), 1, false), "Web wallpaper found in your Steam library.");
  assert.equal(libraryLine(library({ found: true, state: "installed", kind: "application" }), 1, false), "Project found. Claws can save a ZIP.");
});
