// Tests for src/ui/progress-motion.ts, src/ui/row-order.ts and setProgress's snap in
// src/ui/layout.ts (compiled by "test:motion" in package.json).
import assert from "node:assert/strict";
import { test } from "node:test";

import { progressMotion } from "../../.test-build/ui/progress-motion.js";
import { orderMoves } from "../../.test-build/ui/row-order.js";

const at = (status, progress, progressKnown = true) => ({ status, progress, progressKnown });

test("only forward progress of the same known download glides", () => {
  assert.equal(progressMotion(at("downloading", 0.4), at("downloading", 0.5), false), "smooth");
  assert.equal(progressMotion(at("downloading", 0.5), at("downloading", 0.5), false), "smooth");
});

test("a row's first paint snaps: the bar never grows from zero to where the job already is", () => {
  assert.equal(progressMotion(undefined, at("downloading", 0.62), false), "snap");
});

test("a lower value (a size estimate replaced by the real size) snaps, never slides backwards", () => {
  assert.equal(progressMotion(at("downloading", 0.4), at("downloading", 0.25), false), "snap");
});

test("every status change snaps: completion is never delayed, retry and pause never glide", () => {
  for (const [from, to] of [
    [at("downloading", 0.97), at("completed", 1)],
    [at("processing", 0.99, false), at("completed", 1)],
    [at("downloading", 0.5), at("paused", 0.5)],
    [at("downloading", 0.5), at("failed", 0.5)],
    [at("downloading", 0.5), at("cancelled", 0.5)],
    [at("failed", 0.8), at("queued", 0)],
    [at("queued", 0), at("downloading", 0.1)],
    [at("paused", 0.34), at("downloading", 0.35)],
  ]) {
    assert.equal(progressMotion(from, to, false), "snap", `${from.status} -> ${to.status}`);
  }
});

test("a size becoming known or unknown snaps, so the indeterminate sheen never morphs into the fill", () => {
  assert.equal(progressMotion(at("downloading", 0, false), at("downloading", 0.3)), "snap");
  assert.equal(progressMotion(at("downloading", 0.3), at("downloading", 0.3, false)), "snap");
});

test("a hidden window snaps: nothing is animated that nobody can see", () => {
  assert.equal(progressMotion(at("downloading", 0.4), at("downloading", 0.5), true), "snap");
});

/** Applies moves the way insertBefore would, on a plain array of keys. */
function apply(children, moves) {
  const list = [...children];
  for (const { key, before } of moves) {
    list.splice(list.indexOf(key), 1);
    list.splice(before === null ? list.length : list.indexOf(before), 0, key);
  }
  return list;
}

test("rows already in order do not move", () => {
  assert.deepEqual(orderMoves(["a", "b", "c"], ["a", "b", "c"], new Set()), []);
});

test("a row collapsing out stays where it was; the rows around it are not moved", () => {
  // "a" was removed and is still collapsing: it keeps its place until the animation ends.
  assert.deepEqual(orderMoves(["a", "b", "c"], ["b", "c"], new Set(["a"])), []);
  assert.deepEqual(orderMoves(["a", "b", "c", "d"], ["a", "c", "d"], new Set(["b"])), []);
  assert.deepEqual(orderMoves(["a", "b", "c"], [], new Set(["a", "b", "c"])), []);
});

test("an out-of-place row moves once, relative to rows that stay", () => {
  const children = ["a", "c", "b"];
  const moves = orderMoves(children, ["a", "b", "c"], new Set());
  assert.equal(moves.length, 1);
  assert.deepEqual(apply(children, moves), ["a", "b", "c"]);
});

test("a row out of place behind a leaving row lands among the staying rows", () => {
  const children = ["a", "x", "c", "b", "y"];
  const moves = orderMoves(children, ["a", "b", "c"], new Set(["x", "y"]));
  const after = apply(children, moves);
  assert.deepEqual(after.filter((k) => k !== "x" && k !== "y"), ["a", "b", "c"]);
  assert.equal(after.indexOf("x"), 1, "x did not move");
  assert.equal(after.at(-1), "y", "y did not move");
});

test("ordering skips leaving rows and keeps them in place", () => {
  const children = ["x", "a", "c", "b"];
  const moves = orderMoves(children, ["a", "b", "c"], new Set(["x"]));
  const after = apply(children, moves);
  assert.deepEqual(after.filter((k) => k !== "x"), ["a", "b", "c"]);
  assert.equal(after[0], "x", "the leaving row was not moved");
});

/** Just enough of a progress bar for setProgress. */
function fakeBar() {
  const classes = new Set();
  const props = new Map();
  const attrs = new Map();
  return {
    classes,
    props,
    attrs,
    classList: { add: (c) => classes.add(c), remove: (c) => classes.delete(c), contains: (c) => classes.has(c) },
    style: { setProperty: (k, v) => props.set(k, v), getPropertyValue: (k) => props.get(k) ?? "" },
    setAttribute: (k, v) => attrs.set(k, v),
    removeAttribute: (k) => attrs.delete(k),
  };
}

// These run first on purpose: setProgress keeps one module-wide batch of snapped bars, and a
// test that never runs its frames would leave later ones looking snapped for good.
// ----- the Wallpaper Engine export tracker's bar -----

/** Draws one tracker state the way views/workshop.ts does, and returns what the bar looks like. */
async function drawTracker(bar, previous, next) {
  const { setProgress } = await import("../../.test-build/ui/layout.js");
  const { progressMotion } = await import("../../.test-build/ui/progress-motion.js");
  setProgress(bar, next.progressKnown ? next.progress : null, progressMotion(previous, next, false));
  return { snapped: bar.classes.has("bar-snap"), sheen: bar.classes.has("indeterminate"), shown: Number(bar.props.get("--p") ?? 0) };
}

test("a recording's bar sheens while it opens and loads, snaps into the count, then glides forward only", async () => {
  const frames = [];
  globalThis.requestAnimationFrame = (run) => frames.push(run);
  const flush = () => { while (frames.length) frames.shift()(); };
  const bar = fakeBar();
  const state = (status, progress, known = true) => ({ status, progress, progressKnown: known });
  let previous;
  const run = [
    state("checking", 0, false),
    state("opening", 0, false),
    state("loading", 0, false),
    state("recording", 0, true),
    state("recording", 0.2, true),
    state("recording", 0.5, true),
    state("recording", 0.98, true),
    state("finishing", 0.98, true),
    state("saved", 1, true),
  ];
  const seen = [];
  for (const next of run) {
    const drawn = await drawTracker(bar, previous, next);
    seen.push({ ...drawn, status: next.status });
    previous = next;
    flush();
  }
  const by = (status, index = 0) => seen.filter((s) => s.status === status)[index];
  assert.ok(by("checking").sheen && by("opening").sheen && by("loading").sheen, "no number, no fill");
  assert.ok(by("recording", 0).snapped && !by("recording", 0).sheen, "leaving the sheen snaps");
  assert.ok(!by("recording", 1).snapped && !by("recording", 2).snapped && !by("recording", 3).snapped, "a continuing count glides");
  assert.ok(by("finishing").snapped, "a stage change snaps, so nothing lags behind the real state");
  assert.equal(by("finishing").shown, 0.98, "finishing holds the last value");
  assert.ok(by("saved").snapped && by("saved").shown === 1, "100% only when saved");
  const shown = seen.filter((s) => !s.sheen).map((s) => s.shown);
  assert.deepEqual(shown, [...shown].sort((a, b) => a - b), "the bar never slides back");
});

test("a lower value inside one stage snaps; the bar is never drawn sliding back", async () => {
  const frames = [];
  globalThis.requestAnimationFrame = (run) => frames.push(run);
  const bar = fakeBar();
  const copying = (progress) => ({ status: "copying", progress, progressKnown: true });
  await drawTracker(bar, undefined, copying(0.6));
  while (frames.length) frames.shift()();
  const back = await drawTracker(bar, copying(0.6), copying(0.3));
  assert.ok(back.snapped, "an impossible regression is shown at once, not as a slide");
  while (frames.length) frames.shift()();
});

test("a hidden window snaps the tracker's bar", async () => {
  const frames = [];
  globalThis.requestAnimationFrame = (run) => frames.push(run);
  const { setProgress } = await import("../../.test-build/ui/layout.js");
  const { progressMotion } = await import("../../.test-build/ui/progress-motion.js");
  const bar = fakeBar();
  const a = { status: "converting", progress: 0.2, progressKnown: true };
  const b = { status: "converting", progress: 0.3, progressKnown: true };
  setProgress(bar, 0.2, "snap");
  while (frames.length) frames.shift()();
  setProgress(bar, 0.3, progressMotion(a, b, true));
  assert.ok(bar.classes.has("bar-snap"));
  while (frames.length) frames.shift()();
});

test("a snap turns the bar's transition off for exactly the frame that shows the new value", async () => {
  const frames = [];
  globalThis.requestAnimationFrame = (run) => frames.push(run);
  const { setProgress } = await import("../../.test-build/ui/layout.js");
  const bar = fakeBar();
  setProgress(bar, 0.5, "snap");
  assert.equal(bar.props.get("--p"), "0.5");
  assert.ok(bar.classes.has("bar-snap"));
  // The first frame renders the value with the transition off; the class goes only after it.
  frames.shift()();
  assert.ok(bar.classes.has("bar-snap"), "still off while the snapped value is being drawn");
  frames.shift()();
  assert.ok(!bar.classes.has("bar-snap"));
  setProgress(bar, 0.6);
  assert.ok(!bar.classes.has("bar-snap"), "smooth updates leave the transition on");
  assert.equal(frames.length, 0, "and schedule no frames");
});

test("many snaps in one frame share one pair of frames", async () => {
  const frames = [];
  globalThis.requestAnimationFrame = (run) => frames.push(run);
  const { setProgress } = await import("../../.test-build/ui/layout.js");
  const bars = Array.from({ length: 50 }, fakeBar);
  for (const bar of bars) setProgress(bar, 1, "snap");
  assert.equal(frames.length, 1);
  frames.shift()();
  frames.shift()();
  assert.equal(frames.length, 0);
  assert.ok(bars.every((bar) => !bar.classes.has("bar-snap")));
});

test("a bar snapped again between the two frames stays off until its own frame has drawn", async () => {
  const frames = [];
  globalThis.requestAnimationFrame = (run) => frames.push(run);
  const { setProgress } = await import("../../.test-build/ui/layout.js");
  const bar = fakeBar();
  setProgress(bar, 0.8, "snap"); // failed at 80%
  frames.shift()(); // first frame of that snap
  setProgress(bar, 0, "snap"); // retried: back to 0 before the class came off
  frames.shift()(); // the first snap's second frame must not re-enable the transition
  assert.ok(bar.classes.has("bar-snap"), "0 is still drawn without sliding back");
  while (frames.length) frames.shift()();
  assert.ok(!bar.classes.has("bar-snap"));
});

test("leaving the indeterminate sheen always snaps, whoever calls", async () => {
  const frames = [];
  globalThis.requestAnimationFrame = (run) => frames.push(run);
  const { setProgress } = await import("../../.test-build/ui/layout.js");
  const bar = fakeBar();
  setProgress(bar, null);
  assert.ok(bar.classes.has("indeterminate"));
  setProgress(bar, 0.3); // a caller that did not ask for a snap
  assert.ok(!bar.classes.has("indeterminate"));
  assert.ok(bar.classes.has("bar-snap"), "the sheen never morphs into the fill");
  while (frames.length) frames.shift()();
});

test("a lower value snaps whoever calls: a tool bar going from one download to the next never slides back", async () => {
  const frames = [];
  globalThis.requestAnimationFrame = (run) => frames.push(run);
  const { setProgress } = await import("../../.test-build/ui/layout.js");
  const bar = fakeBar();
  setProgress(bar, 0.4, "snap");
  while (frames.length) frames.shift()();
  setProgress(bar, 1); // yt-dlp finished
  assert.ok(!bar.classes.has("bar-snap"), "forward glides");
  setProgress(bar, 0.02); // ffmpeg starts on the same bar, a caller that did not ask for a snap
  assert.ok(bar.classes.has("bar-snap"), "a lower value is drawn at once");
  assert.equal(bar.props.get("--p"), "0.02");
  while (frames.length) frames.shift()();
});

test("a hidden document snaps whoever calls", async () => {
  const frames = [];
  globalThis.requestAnimationFrame = (run) => frames.push(run);
  const { setProgress } = await import("../../.test-build/ui/layout.js");
  const bar = fakeBar();
  setProgress(bar, 0.2, "snap");
  while (frames.length) frames.shift()();
  globalThis.document = { hidden: true };
  try {
    setProgress(bar, 0.6);
    assert.ok(bar.classes.has("bar-snap"));
  } finally {
    delete globalThis.document;
  }
  while (frames.length) frames.shift()();
});

test("progress never shows past 100% or below 0%", async () => {
  globalThis.requestAnimationFrame = () => {};
  const { setProgress } = await import("../../.test-build/ui/layout.js");
  const bar = fakeBar();
  setProgress(bar, 1.4);
  assert.equal(bar.props.get("--p"), "1");
  setProgress(bar, -0.2, "snap");
  assert.equal(bar.props.get("--p"), "0");
});
