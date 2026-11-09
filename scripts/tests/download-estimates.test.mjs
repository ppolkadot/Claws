// Tests for src/download-estimates.ts, compiled first into .test-build/ (see package.json
// "test:logic"):
//   npm exec tsc -- --target ES2022 --module NodeNext --moduleResolution NodeNext --outDir .test-build src/download-estimates.ts
//   node --test scripts/tests/download-estimates.test.mjs
import assert from "node:assert/strict";
import { test } from "node:test";

import {
  CallLimit,
  ESTIMATE_CAP,
  ESTIMATE_CONCURRENCY,
  EstimateRun,
  PreviewEstimator,
  aggregate,
  describeFailure,
  describeRun,
  describeSingle,
  estimateState,
  failedEstimate,
  formatSize,
} from "../../.test-build/download-estimates.js";
import { itemKey } from "../../.test-build/download-input.js";

const MB = 1024 * 1024;
const GB = 1024 * MB;

/** disk_budget.rs `margin`: at least 512 MiB, or a tenth of the demand. */
const margin = (bytes) => Math.max(512 * 1024 * 1024, Math.floor(bytes / 10));
/** disk_budget.rs `required`: what admission asks to be free for a peak. */
const required = (peak) => (peak == null ? null : peak + margin(peak));

const est = (bytes, confidence = "approximate", peak = bytes == null ? null : bytes * 2, over = {}) => ({
  downloadBytes: bytes,
  confidence: bytes == null ? "unknown" : confidence,
  parts: [],
  container: "mp4",
  peakTempBytes: peak,
  requiredFreeBytes: required(peak),
  outputBytes: bytes,
  converted: false,
  unknownReason: bytes == null ? "The site does not say how large this download is." : null,
  ...over,
});

/** Audio converted to `format`: the downloaded stream and the kept file differ. */
const audio = (bytes, output, format = "wav", over = {}) =>
  est(bytes, "approximate", bytes == null || output == null ? null : bytes + output, {
    container: format,
    converted: true,
    outputBytes: output,
    parts: [{ formatId: "a251", vcodec: null, acodec: "opus", ext: "webm", width: null, height: null, bytes, exact: false }],
    ...over,
  });

/** A promise and the functions that settle it, so tests decide when replies arrive. */
function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const tick = () => new Promise((resolve) => setImmediate(resolve));

// ---- Formatting --------------------------------------------------------------------

test("sizes read like the backend's, one decimal below 100", () => {
  assert.equal(formatSize(812 * MB), "812 MB");
  assert.equal(formatSize(2.1 * GB), "2.1 GB");
  assert.equal(formatSize(4.3 * GB), "4.3 GB");
  assert.equal(formatSize(512), "512 B");
  assert.equal(formatSize(1536), "1.5 KB");
  // Rounding up to the next unit moves to that unit.
  assert.equal(formatSize(1024 * MB - 1), "1.0 GB");
});

test("large totals stay readable and are not cut to a smaller unit", () => {
  assert.equal(formatSize(3 * 1024 * GB), "3.0 TB");
  assert.equal(formatSize(2.5e15), "2,274 TB");
  assert.equal(formatSize(Number.MAX_SAFE_INTEGER).endsWith(" TB"), true);
});

// ---- Aggregation -------------------------------------------------------------------

test("one exact item says Exactly, one approximate says About", () => {
  assert.equal(aggregate([est(812 * MB, "exact")]).text, "Exactly 812 MB");
  assert.equal(aggregate([est(812 * MB, "approximate")]).text, "About 812 MB");
});

test("an unknown size is counted, never added as zero", () => {
  const sum = aggregate([est(1 * GB), est(null), est(1.1 * GB), est(null), est(null)]);
  assert.equal(sum.knownBytes, 2.1 * GB);
  assert.equal(sum.knownCount, 2);
  assert.equal(sum.unknownCount, 3);
  assert.equal(sum.approximate, true);
  assert.equal(sum.text, "About 2.1 GB; 3 items have unknown sizes");
  assert.equal(aggregate([est(GB), est(null)]).text, "About 1.0 GB; 1 item has an unknown size");
});

test("an older no-answer (null) is an unknown item", () => {
  const sum = aggregate([est(GB, "exact"), null]);
  assert.equal(sum.unknownCount, 1);
  assert.equal(sum.knownBytes, GB);
});

test("exact sizes beside unknown ones name how many the exact total covers", () => {
  const sum = aggregate([est(400 * MB, "exact"), est(412 * MB, "exact"), est(null)]);
  assert.equal(sum.approximate, false);
  assert.equal(sum.text, "Exactly 812 MB for 2 items; 1 item has an unknown size");
});

test("several items: exact only when every known one is exact", () => {
  assert.equal(aggregate([est(GB, "exact"), est(GB, "exact")]).text, "Exactly 2.0 GB");
  assert.equal(aggregate([est(GB, "exact"), est(GB, "approximate")]).text, "About 2.0 GB");
});

test("all unknown is Size unknown, never 0 B", () => {
  const one = aggregate([est(null)]);
  assert.equal(one.text, "Size unknown");
  assert.equal(one.knownBytes, null);
  const many = aggregate([est(null), null, est(null)]);
  assert.equal(many.text, "Sizes unknown for all 3 items");
  assert.equal(many.knownBytes, null);
  for (const text of [one.text, many.text, one.tempText ?? "", many.tempText ?? ""]) assert.doesNotMatch(text, /\b0 B\b/);
});

test("a zero or negative size from anywhere is unknown, not zero", () => {
  const sum = aggregate([est(0, "exact", 0), est(-5, "approximate", 4)]);
  assert.equal(sum.unknownCount, 2);
  assert.equal(sum.text, "Sizes unknown for all 2 items");
});

test("the free space is shown apart from the download size, with admission's margin", () => {
  // A 4.3 GB peak: admission asks for it plus at least 512 MB, so that is what is shown, not
  // the bare peak a download could then be refused against.
  const single = aggregate([est(2 * GB, "approximate", 4.3 * GB)]);
  assert.equal(single.text, "About 2.0 GB");
  assert.equal(single.requiredFreeBytes, required(4.3 * GB));
  assert.equal(single.tempText, "Needs about 4.8 GB free while downloading, including a safety margin");
  // A small download still needs the 512 MB minimum margin.
  assert.equal(
    aggregate([est(10 * MB, "exact", 10 * MB)]).tempText,
    "Needs about 522 MB free while downloading, including a safety margin",
  );
  // A known download whose conversion cannot be sized.
  const noPeak = aggregate([est(GB, "approximate", null)]);
  assert.equal(noPeak.tempText, "Free space needed while downloading is unknown");
  // Several: the largest single need, since each download checks its own.
  const several = aggregate([est(GB, "exact", 2 * GB), est(3 * GB, "exact", 4.3 * GB), est(null)]);
  assert.equal(several.requiredFreeBytes, required(4.3 * GB));
  assert.equal(
    several.tempText,
    "The largest item needs about 4.8 GB free while downloading, including a safety margin; unknown for 1 item",
  );
  assert.equal(aggregate([est(null)]).tempText, null);
});

test("a conversion names the downloaded size and the kept file apart", () => {
  // A 10-minute 12 MB Opus stream saved as WAV: about 110 MB is kept, not 11 MB.
  const wav = audio(12_000_000, 600 * 192_000);
  const sum = aggregate([wav]);
  assert.equal(sum.text, "About 11.4 MB", "the download is the stream's size");
  assert.equal(sum.converted, true);
  assert.equal(sum.outputBytes, 600 * 192_000);
  assert.equal(sum.outputText, "Saved as WAV: about 110 MB");
  assert.deepEqual(describeSingle({ status: "done", estimate: wav }), {
    size: "Downloads about 11.4 MB  ·  saved as WAV, about 110 MB",
    detail: `Needs about ${formatSize(required(12_000_000 + 600 * 192_000))} free while downloading, including a safety margin`,
  });
  // Without a duration the WAV cannot be sized: unknown, never the stream's size or zero.
  const noLength = audio(12_000_000, null);
  assert.equal(describeSingle({ status: "done", estimate: noLength }).size, "Downloads about 11.4 MB  ·  saved as WAV, size unknown");
  assert.equal(aggregate([noLength]).outputText, "Saved as WAV; saved size unknown");
  // Exact stream sizes still say exactly for the download only.
  assert.equal(
    describeSingle({ status: "done", estimate: audio(9_600_000, 9_600_000, "mp3", { confidence: "exact" }) }).size,
    "Downloads exactly 9.2 MB  ·  saved as MP3, about 9.2 MB",
  );
  // An unconverted file names its own container, as before.
  assert.equal(describeSingle({ status: "done", estimate: est(GB, "exact") }).size, "Exactly 1.0 GB  ·  MP4");
});

test("list totals of a conversion sum the kept files, unknowns counted apart", () => {
  const sum = aggregate([audio(10 * MB, 100 * MB, "flac"), audio(20 * MB, 200 * MB, "flac"), audio(5 * MB, null, "flac"), null]);
  assert.equal(sum.knownBytes, 35 * MB);
  assert.equal(sum.outputBytes, 300 * MB);
  assert.equal(sum.outputUnknownCount, 2);
  assert.equal(sum.outputText, "Saved as FLAC: about 300 MB for 2 items; 2 items have unknown saved sizes");
  assert.equal(aggregate([audio(null, null), null]).outputText, "Saved as WAV; saved sizes unknown");
  assert.equal(aggregate([est(GB)]).outputText, null, "nothing converted, nothing more to say");
  const run = new EstimateRun(() => Promise.resolve(audio(10 * MB, 100 * MB)));
  run.start(["a"], "k");
  return tick().then(() => assert.equal(describeRun(run), "About 10.0 MB. Saved as WAV: about 100 MB."));
});

test("a single preview describes loading, failure and the selected streams", () => {
  assert.deepEqual(describeSingle({ status: "loading" }), { size: "Estimating size…", detail: "" });
  // A failed estimate is no longer passed off as an unknown size (see the failed-estimate tests below).
  assert.deepEqual(describeSingle({ status: "failed" }), { size: "Size estimate failed", detail: "" });
  const video = est(310 * MB, "approximate", 620 * MB, {
    parts: [
      { formatId: "v1080", vcodec: "avc1.640028", acodec: null, ext: "mp4", width: 1920, height: 1080, bytes: 300 * MB, exact: true },
      { formatId: "a140", vcodec: null, acodec: "mp4a.40.2", ext: "m4a", width: null, height: null, bytes: 10 * MB, exact: false },
    ],
  });
  assert.deepEqual(describeSingle({ status: "done", estimate: video }), {
    size: "About 310 MB  ·  MP4, 1920×1080",
    detail: "Needs about 1.1 GB free while downloading, including a safety margin",
  });
  const unknown = est(null);
  assert.deepEqual(describeSingle({ status: "done", estimate: unknown }), {
    size: "Size unknown",
    detail: "The site does not say how large this download is.",
  });
});

// ---- One link: debounced, latest only ---------------------------------------------

function fakeTimers() {
  const timers = new Map();
  let next = 1;
  return {
    set: (fn, ms) => {
      const id = next++;
      timers.set(id, { fn, ms });
      return id;
    },
    clear: (id) => timers.delete(id),
    fire() {
      const all = [...timers.values()];
      timers.clear();
      all.forEach((t) => t.fn());
    },
    get pending() {
      return [...timers.values()].map((t) => t.ms);
    },
  };
}

test("a preview waits for the choices to settle, then asks once", async () => {
  const timers = fakeTimers();
  const calls = [];
  const replies = [];
  const estimator = new PreviewEstimator(
    (key) => {
      calls.push(key);
      const d = deferred();
      replies.push(d);
      return d.promise;
    },
    { timers, onChange: () => undefined },
  );
  estimator.request("720p");
  estimator.request("1080p");
  assert.equal(estimator.state.status, "loading", "loading shows at once");
  assert.deepEqual(timers.pending, [250]);
  timers.fire();
  assert.deepEqual(calls, ["1080p"], "only the settled choice is estimated");
  // Asking again for what is already being estimated does nothing.
  estimator.request("1080p");
  assert.deepEqual(timers.pending, []);
  replies[0].resolve(est(GB));
  await tick();
  assert.equal(estimator.state.status, "done");
  assert.equal(estimator.state.estimate.downloadBytes, GB);
});

test("a superseded reply is ignored, and a failure is unknown, not an error", async () => {
  const timers = fakeTimers();
  const replies = [];
  const estimator = new PreviewEstimator(() => {
    const d = deferred();
    replies.push(d);
    return d.promise;
  }, { timers });
  estimator.request("video");
  timers.fire();
  estimator.request("audio");
  timers.fire();
  replies[0].resolve(est(GB)); // the older answer
  await tick();
  assert.equal(estimator.state.status, "loading", "the older answer is not shown");
  replies[1].resolve(est(10 * MB));
  await tick();
  assert.equal(estimator.state.estimate.downloadBytes, 10 * MB);
  estimator.request("video");
  timers.fire();
  replies[2].reject("yt-dlp is not installed yet.");
  await tick();
  assert.equal(estimator.state.status, "failed");
  estimator.cancel();
  assert.equal(estimator.state.status, "idle");
});

test("a preview runs one backend call at a time; only the latest waiting choice follows", async () => {
  const timers = fakeTimers();
  const calls = [];
  const replies = [];
  const estimator = new PreviewEstimator((key) => {
    calls.push(key);
    const d = deferred();
    replies.push(d);
    return d.promise;
  }, { timers });
  // Choices clicked through slower than the debounce: each settles while the first runs.
  for (const key of ["144p", "360p", "720p", "1080p"]) {
    estimator.request(key);
    timers.fire();
  }
  assert.deepEqual(calls, ["144p"], "no second process while one runs");
  replies[0].resolve(est(MB));
  await tick();
  assert.deepEqual(calls, ["144p", "1080p"], "the superseded 360p and 720p are never asked");
  assert.equal(estimator.state.status, "loading");
  replies[1].resolve(est(GB));
  await tick();
  assert.equal(estimator.state.estimate.downloadBytes, GB);
  // A failed call frees the slot too; a key cancelled while waiting is never asked.
  estimator.request("audio");
  timers.fire();
  estimator.request("video");
  timers.fire();
  estimator.cancel();
  replies[2].reject("offline");
  await tick();
  assert.deepEqual(calls, ["144p", "1080p", "audio"]);
  assert.equal(estimator.state.status, "idle");
  estimator.request("best");
  timers.fire();
  assert.deepEqual(calls, ["144p", "1080p", "audio", "best"]);
});

// ---- Many items: explicit, bounded, cancellable -----------------------------------

test("estimates at most three at once, capped, and says how many were not estimated", async () => {
  assert.equal(ESTIMATE_CONCURRENCY, 3);
  assert.equal(ESTIMATE_CAP, 200);
  const pending = [];
  let running = 0;
  let peak = 0;
  const run = new EstimateRun(
    (item) => {
      running += 1;
      peak = Math.max(peak, running);
      const d = deferred();
      pending.push({ item, settle: (value) => { running -= 1; value === "fail" ? d.reject("offline") : d.resolve(value); } });
      return d.promise;
    },
    { cap: 5 },
  );
  run.start(["a", "b", "c", "d", "e", "f", "g"], "k1");
  assert.equal(pending.length, 3, "three at once");
  assert.match(describeRun(run), /^Estimating sizes… 0 of 5$/);
  while (pending.length > 0) {
    const next = pending.shift();
    next.settle(next.item === "b" ? est(null) : next.item === "d" ? "fail" : est(100 * MB, "exact"));
    await tick();
  }
  assert.equal(peak, 3);
  assert.equal(run.state.done, 5);
  assert.equal(run.state.running, false);
  assert.equal(run.state.notEstimated, 2);
  assert.equal(
    describeRun(run),
    // "b" has no size; "d" could not be estimated at all: a failure, counted apart.
    "Exactly 300 MB for 3 items; 1 item has an unknown size; the estimate failed for 1 item. Only the first 5 selected items were estimated; 2 were not.",
  );
});

test("Stop ends a run; late replies and a replaced run's replies are ignored", async () => {
  const pending = [];
  const run = new EstimateRun((item) => {
    const d = deferred();
    pending.push({ item, d });
    return d.promise;
  });
  run.start(["a", "b", "c", "d"], "k1");
  pending[0].d.resolve(est(GB));
  await tick();
  run.cancel();
  assert.equal(run.state.cancelled, true);
  pending[1].d.resolve(est(GB)); // answered after Stop
  await tick();
  assert.equal(run.state.done, 1);
  assert.equal(describeRun(run), "About 1.0 GB. Stopped after 1 of 4; 3 were not estimated.");
  // Replies still running count towards the limit, so a new run waits for a free slot.
  run.start(["x", "y"], "k2");
  assert.equal(run.key, "k2");
  assert.equal(pending.length, 5, "only one slot is free while two old calls run");
  pending[2].d.resolve(est(GB)); // from the replaced run
  await tick();
  assert.equal(run.state.done, 0, "an older run's reply never lands in this one");
  assert.equal(pending.length, 6, "its slot went to the next item");
  pending[4].d.resolve(est(MB, "exact"));
  pending[5].d.resolve(est(MB, "exact"));
  await tick();
  assert.equal(describeRun(run), "Exactly 2.0 MB");
  run.reset();
  assert.equal(run.key, null);
  assert.equal(describeRun(run), "");
});

test("nothing is estimated until asked", () => {
  let calls = 0;
  const run = new EstimateRun(() => {
    calls += 1;
    return Promise.resolve(est(GB));
  });
  assert.equal(calls, 0);
  assert.equal(describeRun(run), "");
  run.start([], "empty");
  assert.equal(calls, 0);
  assert.equal(describeRun(run), "");
});

// ---- Runs follow the selection; one limit is shared ---------------------------------

/** An estimator whose replies the test settles; `calls` lists what was asked, in order. */
function controlled() {
  const calls = [];
  const replies = [];
  const fn = (item) => {
    calls.push(item);
    const d = deferred();
    replies.push(d);
    return d.promise;
  };
  return { calls, replies, fn };
}

test("an item selected after a run starts does not end it; it is named as not included", async () => {
  const { calls, replies, fn } = controlled();
  let changes = 0;
  const run = new EstimateRun(fn, { onChange: () => (changes += 1) });
  run.start(["a", "b"], "best");
  // A link finishes reading and is selected on its own while the run goes.
  assert.equal(run.follow("best", ["a", "b", "c"]), true, "the run stands");
  assert.equal(run.key, "best");
  assert.equal(run.state.running, true);
  assert.equal(run.state.notCovered, 1);
  assert.equal(describeRun(run), "Estimating sizes… 0 of 2; 1 item selected after estimating began is not included");
  replies[0].resolve(est(GB, "exact"));
  replies[1].resolve(est(GB, "exact"));
  await tick();
  assert.deepEqual(calls, ["a", "b"], "the later item is not estimated behind the user's back");
  assert.equal(run.state.done, 2, "results so far are kept");
  assert.equal(describeRun(run), "Exactly 2.0 GB. 1 item selected after estimating began is not included.");
  // Two more later; the order of the selection does not matter.
  assert.equal(run.follow("best", ["d", "c", "b", "a"]), true);
  assert.match(describeRun(run), /2 items selected after estimating began are not included\.$/);
  // Unselected again: the run covers the whole selection once more.
  assert.equal(run.follow("best", ["a", "b"]), true);
  assert.equal(describeRun(run), "Exactly 2.0 GB");
  assert.ok(changes > 0);
});

test("other choices, or an estimated item unselected, end the run", () => {
  const { fn } = controlled();
  const run = new EstimateRun(fn);
  run.start(["a", "b"], "best");
  assert.equal(run.follow("720p", ["a", "b"]), false, "other choices");
  assert.equal(run.key, null);
  assert.equal(describeRun(run), "");
  run.start(["a", "b"], "best");
  assert.equal(run.follow("best", ["a", "c"]), false, "b was unselected (or sent to the queue)");
  assert.equal(run.key, null);
  // No run: following does nothing.
  assert.equal(run.follow("best", ["a"]), true);
  assert.equal(run.key, null);
});

test("repeats are estimated once, as the queue adds each item once", async () => {
  const { calls, replies, fn } = controlled();
  // The queue's identity, as the download view passes it: link and playlist position.
  const run = new EstimateRun(fn, { id: itemKey });
  const v = { url: "https://example.com/v", playlistIndex: null };
  const w = { url: "https://example.com/w", playlistIndex: null };
  // The same video as a playlist entry with its own link and as a single link, plus a
  // playlist that lists one video twice.
  run.start([v, w, { ...v }, w], "best");
  assert.deepEqual(calls, [v, w]);
  assert.equal(run.state.total, 2);
  replies[0].resolve(est(2.1 * GB));
  replies[1].resolve(est(GB));
  await tick();
  assert.equal(describeRun(run), "About 3.1 GB", "not 5.2 GB");
  assert.equal(run.follow("best", [v, w, v]), true);
  assert.equal(run.state.notCovered, 0, "a repeat is not a newly selected item");
  // Same link at another playlist position is another queue item.
  run.start([{ url: v.url, playlistIndex: 1 }, { url: v.url, playlistIndex: 2 }], "best");
  assert.equal(run.state.total, 2);
});

test("one limit covers every run and the preview, including calls left by a reset run", async () => {
  const calls = new CallLimit(3);
  const a = controlled();
  const b = controlled();
  const runA = new EstimateRun(a.fn, { calls });
  const runB = new EstimateRun(b.fn, { calls });
  runA.start(["a1", "a2", "a3", "a4", "a5"], "k");
  assert.equal(a.calls.length, 3);
  runA.reset(); // e.g. the batch view was left; its three yt-dlp calls keep running
  assert.equal(calls.active, 3);
  runB.start(["b1", "b2"], "k");
  assert.deepEqual(b.calls, [], "B waits while A's orphaned calls fill every slot");
  const timers = fakeTimers();
  const p = controlled();
  const preview = new PreviewEstimator(p.fn, { timers, calls });
  preview.request("single");
  timers.fire();
  assert.deepEqual(p.calls, [], "the preview waits for a slot too");
  assert.equal(preview.state.status, "loading");
  a.replies[0].resolve(est(GB));
  await tick();
  assert.equal(calls.active, 3);
  assert.equal(b.calls.length + p.calls.length, 1, "a freed slot goes to exactly one waiter");
  assert.equal(runA.state.done, 0, "the reset run's reply is ignored");
  a.replies[1].resolve(est(GB));
  a.replies[2].resolve(est(GB));
  await tick();
  assert.deepEqual(b.calls, ["b1", "b2"]);
  assert.deepEqual(p.calls, ["single"]);
  assert.equal(calls.active, 3, "never more than three at once");
  assert.equal(a.calls.length, 3, "the reset run asks nothing more");
  // A cancelled run that was waiting for a slot never takes one later.
  const c = controlled();
  const runC = new EstimateRun(c.fn, { calls });
  runC.start(["c1"], "k");
  runC.cancel();
  b.replies[0].resolve(est(MB));
  await tick();
  assert.deepEqual(c.calls, []);
  assert.equal(calls.active, 2);
  b.replies[1].resolve(est(MB));
  p.replies[0].resolve(est(MB));
  await tick();
  assert.equal(calls.active, 0);
  assert.equal(describeRun(runB), "About 2.0 MB");
  assert.equal(preview.state.status, "done");
});

// ---- Failed estimates are not unknown sizes ------------------------------------------

/** An estimate that could not be made: no size, state failed, a safe reason. */
const failedEst = (why = "Timed out while reading that link.") => ({
  ...est(null),
  state: "failed",
  diagnostic: why,
  unknownReason: why,
});
/** The backend's shape for a size: state available. */
const avail = (bytes, confidence = "approximate") => ({ ...est(bytes, confidence), state: "available", diagnostic: null });
/** The backend's shape for a site that gives no size. */
const unknownEst = () => ({ ...est(null), state: "unknown", diagnostic: null });

test("the state reads from newer responses and is derived for older-shaped ones", () => {
  assert.equal(estimateState(avail(GB)), "available");
  assert.equal(estimateState(unknownEst()), "unknown");
  assert.equal(estimateState(failedEst()), "failed");
  // Older responses have no state: a size is available, none is unknown (never failed).
  assert.equal(estimateState(est(GB)), "available");
  assert.equal(estimateState(est(null)), "unknown");
  assert.equal(estimateState(null), "unknown");
  // A failure carries no usable size, whatever else is in the object.
  assert.equal(estimateState({ ...est(GB), state: "failed" }), "failed");
  // "available" without a size is not a size.
  assert.equal(estimateState({ ...est(null), state: "available" }), "unknown");
  assert.equal(failedEstimate("offline").downloadBytes, null);
  assert.equal(estimateState(failedEstimate("offline")), "failed");
  assert.equal(failedEstimate("offline").diagnostic, "offline");
  assert.equal(failedEstimate("offline").unknownReason, "offline");
});

test("a rejection becomes a failure with a single short line, without local paths", () => {
  assert.equal(describeFailure("yt-dlp is not installed yet."), "yt-dlp is not installed yet.");
  assert.equal(describeFailure(new Error("socket\n hang  up")), "socket hang up");
  assert.equal(describeFailure("cannot open C:\\Users\\Someone\\AppData\\x.json now"), "cannot open <path> now");
  // Other spellings of a local path are left out too; links and plain slashes stay.
  for (const raw of [
    "cannot read file:///C:/Users/Name/x.json today",
    "cannot read file:/home/name/x.json today",
    "cannot read bin/yt-dlp/yt-dlp.exe today",
    "bad --cookies=C:/Users/Name/c.txt given",
    String.raw`bad --cookies=C:\Users\Name\c.txt given`,
    "bad --config-location=/home/name/c.conf given",
  ]) {
    const safe = describeFailure(raw);
    assert.ok(!/Name|name|yt-dlp/.test(safe) && safe.includes("<path>"), `${raw} -> ${safe}`);
  }
  assert.equal(describeFailure("use audio and/or video, 1/2 done"), "use audio and/or video, 1/2 done");
  assert.equal(describeFailure("see https://a.example/b/c?x=1 and http://h/p"), "see https://a.example/b/c?x=1 and http://h/p");
  assert.equal(describeFailure({ weird: true }), "The estimate could not be made.");
  assert.equal(describeFailure("   "), "The estimate could not be made.");
  const long = describeFailure("x ".repeat(500));
  assert.ok(long.length <= 241 && long.endsWith("\u2026"));
});

test("totals count failed apart from missing-size items and never add either as zero", () => {
  const mixed = aggregate([avail(GB, "exact"), avail(2 * GB, "approximate"), unknownEst(), failedEst()]);
  assert.equal(mixed.knownCount, 2);
  assert.equal(mixed.unknownCount, 1);
  assert.equal(mixed.failedCount, 1);
  assert.equal(mixed.knownBytes, 3 * GB, "only the sizes that exist");
  assert.equal(mixed.approximate, true);
  assert.equal(mixed.text, "About 3.0 GB; 1 item has an unknown size; the estimate failed for 1 item");
  // Exact sizes beside missing ones still say how many the exact total covers.
  assert.equal(
    aggregate([avail(GB, "exact"), avail(GB, "exact"), failedEst(), failedEst("x"), unknownEst()]).text,
    "Exactly 2.0 GB for 2 items; 1 item has an unknown size; the estimates failed for 2 items",
  );
  // Only a total with nothing missing is stated as a whole.
  assert.equal(aggregate([avail(GB, "exact"), avail(GB, "exact")]).text, "Exactly 2.0 GB");
  assert.equal(aggregate([avail(GB, "exact"), failedEst()]).text, "Exactly 1.0 GB for 1 item; the estimate failed for 1 item");
  // A failure carrying a size is still no size.
  const odd = aggregate([{ ...est(GB), state: "failed" }]);
  assert.equal(odd.knownBytes, null);
  assert.equal(odd.failedCount, 1);
});

test("all failed, or only failed and unknown, never read as sizes", () => {
  const one = aggregate([failedEst()]);
  assert.equal(one.text, "Size estimate failed");
  assert.equal(one.knownBytes, null);
  assert.equal(one.tempText, null);
  const all = aggregate([failedEst(), failedEst(), failedEst()]);
  assert.equal(all.text, "Size estimates failed for all 3 items");
  assert.equal(all.failedCount, 3);
  assert.equal(all.unknownCount, 0);
  const some = aggregate([failedEst(), unknownEst(), unknownEst()]);
  assert.equal(some.text, "2 items have unknown sizes; the estimate failed for 1 item");
  assert.equal(some.knownBytes, null);
  for (const text of [one.text, all.text, some.text]) assert.doesNotMatch(text, /\b0 B\b/);
  // Unknown stays what it was: no failure counted.
  const unknownOnly = aggregate([unknownEst(), unknownEst()]);
  assert.equal(unknownOnly.failedCount, 0);
  assert.equal(unknownOnly.text, "Sizes unknown for all 2 items");
  // Failed items count towards the free-space and saved-size gaps like unknown ones.
  const wav = aggregate([audio(10 * MB, 100 * MB), failedEst()]);
  assert.equal(wav.outputUnknownCount, 1);
  assert.equal(wav.peakUnknownCount, 1);
});

test("a single preview tells a failure from an unknown size", () => {
  assert.deepEqual(describeSingle({ status: "failed", diagnostic: "Timed out while reading that link." }), {
    size: "Size estimate failed",
    detail: "Timed out while reading that link.",
  });
  assert.deepEqual(describeSingle({ status: "failed" }), { size: "Size estimate failed", detail: "" });
  // The same when the backend answered that it failed.
  assert.deepEqual(describeSingle({ status: "done", estimate: failedEst("yt-dlp is not installed yet.") }), {
    size: "Size estimate failed",
    detail: "yt-dlp is not installed yet.",
  });
  // An unknown size keeps its reason, as before, and offers no failure.
  assert.deepEqual(describeSingle({ status: "done", estimate: unknownEst() }), {
    size: "Size unknown",
    detail: "The site does not say how large this download is.",
  });
});

test("a preview that failed is retried once per request, with the same key and limit", async () => {
  const timers = fakeTimers();
  const calls = new CallLimit(3);
  const asked = [];
  const replies = [];
  const estimator = new PreviewEstimator((key) => {
    asked.push(key);
    const d = deferred();
    replies.push(d);
    return d.promise;
  }, { timers, calls });
  estimator.retry();
  timers.fire();
  assert.deepEqual(asked, [], "nothing to retry before a request");
  estimator.request("video");
  timers.fire();
  estimator.retry();
  timers.fire();
  assert.deepEqual(asked, ["video"], "not while it is loading");
  replies[0].reject("Timed out while reading that link.");
  await tick();
  assert.equal(estimator.state.status, "failed");
  assert.equal(estimator.state.diagnostic, "Timed out while reading that link.");
  assert.equal(estimator.failed, true);
  assert.equal(calls.active, 0);
  // No automatic retry, however long it is left.
  timers.fire();
  await tick();
  assert.deepEqual(asked, ["video"]);
  estimator.request("video"); // the same choices again do not re-ask by themselves
  timers.fire();
  assert.deepEqual(asked, ["video"]);
  estimator.retry();
  assert.equal(estimator.state.status, "loading");
  timers.fire();
  assert.deepEqual(asked, ["video", "video"]);
  assert.equal(calls.active, 1, "the shared limit counts it");
  replies[1].resolve(avail(GB, "exact"));
  await tick();
  assert.equal(estimator.state.status, "done");
  assert.equal(estimator.state.estimate.downloadBytes, GB);
  assert.equal(estimator.failed, false);
  estimator.retry(); // nothing failed: no new call
  timers.fire();
  assert.deepEqual(asked, ["video", "video"]);
});

test("a failure the backend reports is a failed preview, and a retry can fail again", async () => {
  const timers = fakeTimers();
  const replies = [];
  const estimator = new PreviewEstimator(() => {
    const d = deferred();
    replies.push(d);
    return d.promise;
  }, { timers });
  estimator.request("audio");
  timers.fire();
  replies[0].resolve(failedEst("yt-dlp is not installed yet."));
  await tick();
  assert.equal(estimator.state.status, "failed");
  assert.equal(estimator.state.diagnostic, "yt-dlp is not installed yet.");
  estimator.retry();
  timers.fire();
  replies[1].resolve(failedEst("Timed out while reading that link."));
  await tick();
  assert.equal(estimator.state.status, "failed");
  assert.equal(estimator.state.diagnostic, "Timed out while reading that link.");
  assert.equal(replies.length, 2, "one retry per request");
  // A size that simply is not given is done, not failed, and offers no retry.
  estimator.request("video");
  timers.fire();
  replies[2].resolve(unknownEst());
  await tick();
  assert.equal(estimator.state.status, "done");
  assert.equal(estimator.failed, false);
});

test("a late reply to a retry is ignored once the choices changed", async () => {
  const timers = fakeTimers();
  const replies = [];
  const estimator = new PreviewEstimator(() => {
    const d = deferred();
    replies.push(d);
    return d.promise;
  }, { timers });
  estimator.request("720p");
  timers.fire();
  replies[0].reject("offline");
  await tick();
  estimator.retry();
  timers.fire();
  estimator.request("1080p"); // options changed while the retry was running
  timers.fire();
  replies[1].resolve(avail(MB)); // the retry for 720p answers late
  await tick();
  assert.equal(estimator.state.status, "loading", "the 720p retry does not land on 1080p");
  replies[2].resolve(avail(GB));
  await tick();
  assert.equal(estimator.state.estimate.downloadBytes, GB);
});

/** An EstimateRun over replies the test settles one by one; `settle(item, value)` answers. */
function runWith(options = {}) {
  const asked = [];
  const waiting = new Map();
  const run = new EstimateRun((item) => {
    asked.push(item);
    const d = deferred();
    const list = waiting.get(item) ?? [];
    list.push(d);
    waiting.set(item, list);
    return d.promise;
  }, options);
  const settle = (item, value) => {
    const d = waiting.get(item).shift();
    if (value instanceof Error) d.reject(value);
    else d.resolve(value);
  };
  return { run, asked, settle };
}

test("a run's failures are told from unknown sizes and retried alone, keeping valid estimates", async () => {
  const { run, asked, settle } = runWith();
  run.start(["a", "b", "c", "d"], "k");
  settle("a", avail(GB, "exact"));
  settle("b", unknownEst());
  settle("c", new Error("Timed out while reading that link."));
  await tick(); // three calls at a time: d is asked once a slot is free
  settle("d", failedEst("yt-dlp is not installed yet."));
  await tick();
  assert.equal(run.state.running, false);
  assert.equal(run.canRetry, true);
  assert.equal(describeRun(run), "Exactly 1.0 GB for 1 item; 1 item has an unknown size; the estimates failed for 2 items");
  assert.equal(run.retryFailed("k"), true);
  assert.deepEqual(asked, ["a", "b", "c", "d", "c", "d"], "only the two failures are asked again");
  assert.equal(run.state.running, true);
  assert.equal(describeRun(run), "Retrying 2 failed estimates… 0 of 2");
  assert.equal(run.canRetry, false, "not while a retry runs");
  assert.equal(run.retryFailed("k"), false, "no second retry on top");
  settle("c", avail(MB, "exact"));
  settle("d", failedEst("Timed out while reading that link."));
  await tick();
  assert.equal(run.state.running, false);
  assert.equal(describeRun(run), "Exactly 1.0 GB for 2 items; 1 item has an unknown size; the estimate failed for 1 item");
  assert.equal(aggregate(run.state.results).knownCount, 2);
  assert.equal(asked.length, 6, "no automatic retry of what failed again");
  // The user asks once more; it works, and only the unknown size is still missing.
  assert.equal(run.retryFailed("k"), true);
  settle("d", avail(MB, "exact"));
  await tick();
  assert.equal(run.canRetry, false);
  assert.equal(describeRun(run), "Exactly 1.0 GB for 3 items; 1 item has an unknown size");
  assert.deepEqual(asked.slice(6), ["d"]);
  assert.equal(run.retryFailed("k"), false, "nothing failed any more");
});

test("a retry that fixes everything leaves the whole total, and valid estimates are never reset", async () => {
  const { run, asked, settle } = runWith();
  run.start(["a", "b"], "k");
  settle("a", avail(2 * MB, "exact"));
  settle("b", new Error("offline"));
  await tick();
  const before = run.state.results.find((r) => r.downloadBytes === 2 * MB);
  assert.equal(describeRun(run), "Exactly 2.0 MB for 1 item; the estimate failed for 1 item");
  run.retryFailed("k");
  settle("b", avail(3 * MB, "exact"));
  await tick();
  assert.equal(describeRun(run), "Exactly 5.0 MB");
  assert.ok(run.state.results.includes(before), "the estimate that was fine is the same object");
  assert.equal(run.state.done, 2);
  assert.deepEqual(asked, ["a", "b", "b"]);
});

test("a retry uses the shared limit and waits for a free slot", async () => {
  const calls = new CallLimit(2);
  const { run, asked, settle } = runWith({ calls });
  run.start(["a", "b"], "k");
  settle("a", new Error("x"));
  settle("b", new Error("y"));
  await tick();
  const other = runWith({ calls });
  other.run.start(["o1", "o2"], "k");
  assert.equal(calls.active, 2, "another run holds both slots");
  run.retryFailed("k");
  assert.deepEqual(asked, ["a", "b"], "the retry waits for a slot");
  other.settle("o1", avail(MB));
  await tick();
  assert.equal(asked.length, 3, "a freed slot goes to the retry");
  assert.ok(calls.active <= 2);
  other.settle("o2", avail(MB));
  await tick();
  settle("a", avail(MB));
  settle("b", avail(MB));
  await tick();
  assert.equal(calls.active, 0);
});

test("changed choices end the run before a retry asks anything", async () => {
  const { run, asked, settle } = runWith();
  run.start(["a", "b"], "k");
  settle("a", new Error("x"));
  settle("b", avail(MB));
  await tick();
  assert.equal(run.retryFailed("other choices"), false);
  assert.equal(run.key, null, "the old estimates no longer apply");
  assert.deepEqual(asked, ["a", "b"]);
  assert.equal(run.retryFailed("k"), false, "and there is nothing left to retry");
});

test("a late reply to a retry is ignored after the options changed or the run was reset", async () => {
  const { run, asked, settle } = runWith();
  run.start(["a", "b"], "k");
  settle("a", new Error("x"));
  settle("b", avail(GB, "exact"));
  await tick();
  run.retryFailed("k");
  // The options change while the retry is on its way: the view's follow() resets the run.
  assert.equal(run.follow("k2", ["a", "b"]), false);
  assert.equal(run.key, null);
  run.start(["a", "b"], "k2");
  settle("a", avail(MB, "exact")); // the retry's reply, from before the change
  await tick();
  assert.equal(run.state.done, 0, "it did not land in the new run");
  assert.equal(asked.length, 5, "the retry, then the new run's two");
  settle("a", avail(2 * MB, "exact")); // the new run's own first call for "a"
  settle("b", avail(2 * MB, "exact"));
  await tick();
  assert.equal(describeRun(run), "Exactly 4.0 MB");
});

test("Stop ends a retry without marking the run stopped; failed items stay failed and retryable", async () => {
  const { run, settle } = runWith();
  run.start(["a", "b"], "k");
  settle("a", new Error("x"));
  settle("b", new Error("y"));
  await tick();
  run.retryFailed("k");
  run.cancel();
  assert.equal(run.state.running, false);
  assert.equal(run.state.cancelled, false);
  settle("a", avail(MB)); // answered after Stop
  await tick();
  assert.equal(run.canRetry, true);
  assert.equal(describeRun(run), "Size estimates failed for all 2 items");
});

test("a retry in a run capped below its list covers only the items that were asked", async () => {
  const { run, asked, settle } = runWith({ cap: 2 });
  run.start(["a", "b", "c"], "k");
  settle("a", new Error("x"));
  settle("b", avail(MB, "exact"));
  await tick();
  run.retryFailed("k");
  settle("a", avail(MB, "exact"));
  await tick();
  assert.deepEqual(asked, ["a", "b", "a"], "c was beyond the cap and stays so");
  assert.equal(describeRun(run), "Exactly 2.0 MB. Only the first 2 selected items were estimated; 1 was not.");
});

test("a call that throws synchronously is a failure that can be retried", async () => {
  let attempts = 0;
  const run = new EstimateRun(() => {
    attempts += 1;
    if (attempts === 1) throw new Error("bridge closed");
    return Promise.resolve(avail(MB, "exact"));
  });
  run.start(["a"], "k");
  await tick();
  assert.equal(describeRun(run), "Size estimate failed");
  run.retryFailed("k");
  await tick();
  assert.equal(describeRun(run), "Exactly 1.0 MB");
});

// ---- Coming back to a run after another view (session state, see preview-session.ts) --

test("a finished run is saved as plain data and comes back as it was, without asking again", async () => {
  const { run, settle } = runWith();
  run.start(["a", "b", "c"], "k");
  settle("a", avail(GB, "exact"));
  settle("b", unknownEst());
  settle("c", new Error("offline"));
  await tick();
  const saved = run.snapshot();
  assert.ok(saved, "a finished run has a snapshot");
  assert.deepEqual(JSON.parse(JSON.stringify(saved)), saved, "plain data: nothing but JSON values");
  assert.equal(structuredClone(saved).key, "k");

  const { run: back, asked } = runWith();
  back.restore(saved);
  assert.equal(asked.length, 0, "restoring asks nothing");
  assert.equal(back.key, "k");
  assert.equal(back.state.running, false);
  assert.equal(describeRun(back), describeRun(run));
  assert.equal(back.canRetry, true, "a failure that was retryable still is, by the user's hand only");
  assert.equal(back.follow("k", ["a", "b", "c"]), true);
});

test("a run still going is never saved as running: Stop it and what was read stays", async () => {
  const { run, settle } = runWith();
  run.start(["a", "b", "c", "d"], "k");
  settle("a", avail(GB, "exact"));
  await tick();
  assert.equal(run.snapshot(), null, "a running run has no snapshot: its calls are not data");
  run.cancel();
  const saved = run.snapshot();
  assert.ok(saved);
  assert.equal(saved.settled.length, 1);
  const { run: back } = runWith();
  back.restore(saved);
  assert.equal(back.state.running, false);
  assert.equal(back.state.cancelled, true);
  assert.equal(describeRun(back), "Exactly 1.0 GB. Stopped after 1 of 4; 3 were not estimated.");
});

test("a reply from the run that was saved does not land in the restored run", async () => {
  const { run, settle } = runWith();
  run.start(["a", "b"], "k");
  settle("a", avail(GB, "exact"));
  await tick();
  run.cancel();
  const saved = run.snapshot();
  const { run: back } = runWith();
  back.restore(saved);
  settle("b", avail(GB, "exact")); // the first view's call answers after the save
  await tick();
  assert.equal(back.state.results.length, 1);
  assert.equal(run.state.results.length, 1, "and the old run ignores it too (it was stopped)");
});

test("a restored run is for its choices and items only: other choices or an unticked item end it", async () => {
  const { run, settle } = runWith();
  run.start(["a", "b"], "k");
  settle("a", avail(GB, "exact"));
  settle("b", avail(GB, "exact"));
  await tick();
  const saved = run.snapshot();
  const first = runWith().run;
  first.restore(saved);
  assert.equal(first.follow("other", ["a", "b"]), false);
  assert.equal(first.key, null);
  const second = runWith().run;
  second.restore(saved);
  assert.equal(second.follow("k", ["a"]), false, "b is no longer selected");
  assert.equal(second.key, null);
});

test("a restored run keeps the 200-item cap", () => {
  const settled = Array.from({ length: ESTIMATE_CAP + 50 }, (_, i) => ({ item: `i${i}`, result: avail(MB, "exact") }));
  const { run } = runWith();
  run.restore({ key: "k", asked: settled.map((s) => s.item), total: settled.length, requested: settled.length, done: settled.length, cancelled: false, notEstimated: 0, settled });
  assert.equal(run.state.results.length, ESTIMATE_CAP);
});

test("a preview can show an estimate made earlier without asking, and only for its key", async () => {
  const timers = fakeTimers();
  let asked = 0;
  const estimator = new PreviewEstimator(() => {
    asked += 1;
    return Promise.resolve(avail(MB));
  }, { timers });
  estimator.restore("720p", avail(GB));
  assert.equal(estimator.state.status, "done");
  assert.equal(estimator.state.estimate.downloadBytes, GB);
  assert.equal(estimator.currentKey, "720p");
  assert.equal(asked, 0);
  estimator.request("720p"); // the same choices: nothing to ask
  timers.fire();
  assert.equal(asked, 0);
  estimator.request("1080p");
  timers.fire();
  await tick();
  assert.equal(asked, 1, "other choices are asked as usual");
  assert.equal(estimator.currentKey, "1080p");
});
