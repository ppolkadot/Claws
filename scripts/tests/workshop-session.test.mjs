import assert from "node:assert/strict";
import test from "node:test";

import { WorkshopSession, lookupProblem, missingForStart, offersVideo, stageText } from "../../.test-build/workshop-session.js";
import { isWorkshopLink } from "../../.test-build/workshop-link.js";

const URL_A = "https://steamcommunity.com/sharedfiles/filedetails/?id=123";
const item = { id: "123", title: "Ocean", previewUrl: null, fileSize: 10, timeUpdated: null, wallpaperType: "Scene", requiredItems: [] };

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
      findInLibrary: async () => ({ steam: true, found: false, video: false }),
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
  assert.equal(missingForStart("library", { steam: false, found: false }, "", ""), "Steam isn't installed for this Windows user.");
  assert.equal(missingForStart("library", { steam: true, found: false }, "", ""), "Subscribe in Steam first.");
  assert.equal(missingForStart("library", { steam: true, found: true }, "", ""), null);
  assert.equal(missingForStart("steamcmd", null, "", "owner"), "Install SteamCMD first.");
  assert.equal(missingForStart("steamcmd", null, "C:\\SteamCMD\\steamcmd.exe", "anonymous"), "Enter your Steam account name.");
  assert.equal(missingForStart("steamcmd", null, "C:\\SteamCMD\\steamcmd.exe", "a b"), "Enter your Steam account name.");
  assert.equal(missingForStart("steamcmd", null, "C:\\SteamCMD\\steamcmd.exe", " owner_1 "), null);
});

test("stage text names the step, the format and its percentage", () => {
  assert.equal(stageText("updating", 45.4), "Updating SteamCMD… 45%");
  assert.equal(stageText("downloading", null), "Downloading from Steam…");
  assert.equal(stageText("converting", null, "mp4"), "Converting the video with FFmpeg…");
  assert.equal(stageText("saving", 100), "Saving the ZIP… 100%");
  assert.equal(stageText("saving", 42.6, "mp4"), "Saving the MP4… 43%");
  assert.equal(stageText("saving", 5, "gif"), "Saving the GIF… 5%");
});

test("a SteamCMD export sends the account and format; a library export sends no account", async () => {
  const { calls, deps: d } = deps();
  const steam = new WorkshopSession(URL_A, item, d, true);
  assert.equal(steam.source, "steamcmd");
  await steam.start("  owner_1 ");
  assert.deepEqual(calls[0], ["download", URL_A, "steamcmd", "owner_1", "zip"]);
  assert.deepEqual(steam.state, { phase: "saved", filename: "Ocean (123).zip", path: "C:\\out\\Ocean (123).zip" });

  const library = new WorkshopSession(URL_A, item, {
    ...d, findInLibrary: async () => ({ steam: true, found: true, video: true }),
  }, false);
  await library.checkLibrary();
  assert.equal(library.source, "library");
  library.setFormat("mp4");
  assert.equal(library.format, "mp4");
  await library.start("owner_1");
  assert.deepEqual(calls[1], ["download", URL_A, "library", null, "mp4"]);
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
    findInLibrary: async () => ({ steam: true, found: true, video }),
  });
  const session = new WorkshopSession(URL_A, item, d, true);
  await session.checkLibrary(true);
  assert.equal(session.source, "library");
  assert.equal(session.videoAvailable, true);
  assert.equal(session.format, "mp4");
  session.setFormat("gif");
  await session.start("");
  assert.deepEqual(calls[0], ["download", URL_A, "library", null, "gif"]);

  video = false;
  await session.checkLibrary();
  assert.equal(session.videoAvailable, false);
  assert.equal(session.format, "zip");
});

test("an unsupported video export offers the ZIP fallback and clears it on a format change", async () => {
  const { calls, deps: d } = deps({
    download: async (...args) => {
      calls.push(["download", ...args]);
      return { status: "unsupported", message: "No video here." };
    },
  });
  const session = new WorkshopSession(URL_A, { ...item, wallpaperType: "Video" }, d, true);
  session.setFormat("gif");
  await session.start("owner");
  assert.deepEqual(session.state, { phase: "unsupported", message: "No video here." });
  assert.deepEqual(calls[0], ["download", URL_A, "steamcmd", "owner", "gif"]);
  session.setFormat("zip");
  assert.deepEqual(session.state, { phase: "ready" });
  assert.equal(session.format, "zip");
});

test("progress applies only to its own running export", async () => {
  let finish;
  const { deps: d } = deps({ download: () => new Promise((resolve) => (finish = resolve)) });
  const session = new WorkshopSession(URL_A, item, d, true);
  session.progress({ url: URL_A, stage: "downloading", percent: null });
  assert.equal(session.state.phase, "ready", "nothing is running yet");
  const done = session.start("owner");
  assert.ok(session.running);
  session.progress({ url: "https://steamcommunity.com/sharedfiles/filedetails/?id=999", stage: "saving", percent: 50 });
  assert.equal(session.state.stage, "starting");
  session.progress({ url: URL_A, stage: "saving", percent: 50 });
  assert.deepEqual(session.state, { phase: "running", stage: "saving", percent: 50 });
  session.setSource("library");
  assert.equal(session.source, "steamcmd", "the source is fixed while running");
  finish({ status: "failed", message: "Steam needs a Steam Guard code.", detail: "tail" });
  await done;
  assert.deepEqual(session.state, { phase: "failed", message: "Steam needs a Steam Guard code.", detail: "tail" });
});

test("cancel asks the backend only while running; errors become failures", async () => {
  const { calls, deps: d } = deps({ download: async () => { throw new Error("A wallpaper download is already running."); } });
  const session = new WorkshopSession(URL_A, item, d, true);
  session.cancel();
  assert.equal(calls.length, 0);
  await session.start("owner");
  assert.deepEqual(session.state, { phase: "failed", message: "A wallpaper download is already running.", detail: null });
});

test("the library is checked until Steam has the subscribed item", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout", "Date"] });
  let found = false;
  const { deps: d } = deps({ findInLibrary: async () => ({ steam: true, found, video: false }) });
  const session = new WorkshopSession(URL_A, item, d, false);
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
  assert.deepEqual(session.library, { steam: true, found: true, video: false });
  session.dispose();
});
