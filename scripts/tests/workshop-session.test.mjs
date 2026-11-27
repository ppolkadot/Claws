import assert from "node:assert/strict";
import test from "node:test";

import {
  WorkshopSession,
  downloadFraction,
  formatChoices,
  libraryLine,
  lookupProblem,
  missingForStart,
  offersVideo,
  stageText,
} from "../../.test-build/workshop-session.js";
import { isWorkshopLink } from "../../.test-build/workshop-link.js";

const URL_A = "https://steamcommunity.com/sharedfiles/filedetails/?id=123";
const item = { id: "123", title: "Ocean", previewUrl: null, fileSize: 10, timeUpdated: null, wallpaperType: "Scene", requiredItems: [] };

/** A library status as the backend sends it. */
function library(overrides = {}) {
  return { steam: true, wallpaperEngine: true, found: false, video: false, state: "notInstalled", ...overrides };
}

function deps(overrides = {}) {
  const calls = [];
  return {
    calls,
    deps: {
      download: async (...args) => {
        calls.push(["download", ...args]);
        return { status: "saved", filename: "Ocean (123).zip", path: "C:\\out\\Ocean (123).zip" };
      },
      cancel: async () => void calls.push(["cancel"]),
      findInLibrary: async () => library(),
      videoInfo: async () => null,
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

test("stage text names the step, the format and its percentage", () => {
  assert.equal(stageText("starting", null), "Starting…");
  assert.equal(stageText("converting", null, "mp4"), "Converting the video with FFmpeg…");
  assert.equal(stageText("copying", 38.2), "Copying the video… 38%");
  assert.equal(stageText("saving", 100), "Saving the ZIP… 100%");
  assert.equal(stageText("saving", 42.6, "mp4"), "Saving the MP4… 43%");
  assert.equal(stageText("saving", 5, "gif"), "Saving the GIF… 5%");
});

test("an export sends the link and the chosen format, from the Steam library only", async () => {
  const { calls, deps: d } = deps({ findInLibrary: async () => library({ found: true, video: true, state: "installed" }) });
  const session = new WorkshopSession(URL_A, item, d);
  await session.checkLibrary();
  assert.equal(session.format, "mp4");
  session.setFormat("mp4");
  await session.start();
  assert.deepEqual(calls[0], ["download", URL_A, "mp4", {}]);
  assert.deepEqual(session.state, { phase: "saved", filename: "Ocean (123).zip", path: "C:\\out\\Ocean (123).zip" });
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

test("progress applies only to its own running export", async () => {
  let finish;
  const { deps: d } = deps({ download: () => new Promise((resolve) => (finish = resolve)) });
  const session = new WorkshopSession(URL_A, item, d);
  session.progress({ url: URL_A, stage: "converting", percent: null });
  assert.equal(session.state.phase, "ready", "nothing is running yet");
  const done = session.start();
  assert.ok(session.running);
  session.progress({ url: "https://steamcommunity.com/sharedfiles/filedetails/?id=999", stage: "saving", percent: 50 });
  assert.equal(session.state.stage, "starting");
  session.progress({ url: URL_A, stage: "saving", percent: 50 });
  assert.deepEqual(session.state, { phase: "running", stage: "saving", percent: 50 });
  finish({ status: "failed", message: "Steam went away.", detail: "tail" });
  await done;
  assert.deepEqual(session.state, { phase: "failed", message: "Steam went away.", detail: "tail" });
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
  assert.deepEqual(formatChoices(false, null).map((c) => c.label), ["ZIP"]);
  assert.deepEqual(formatChoices(true, copyable).map((c) => c.label), ["ZIP", "MP4 (original)", "GIF"]);
  assert.deepEqual(formatChoices(true, { ...copyable, copyable: false, codec: "hevc" }).map((c) => c.label), ["ZIP", "MP4 (converted)", "GIF"]);
  assert.deepEqual(formatChoices(true, null).map((c) => c.label), ["ZIP", "MP4 (converted)", "GIF"], "unknown means it will convert");
  assert.deepEqual(formatChoices(true, copyable).map((c) => c.value), ["zip", "mp4", "gif"]);
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
