import assert from "node:assert/strict";
import { test } from "node:test";

import { sameTaskbarState, summarize, taskbarState } from "../../.test-build/queue-summary.js";

const job = (status, extra = {}) => ({ status, speed: null, eta: null, progressKnown: false, progressEstimated: false, downloadedBytes: null, totalBytes: null, ...extra });
const transfer = (downloaded, total, extra = {}) => job("downloading", { progressKnown: true, downloadedBytes: downloaded, totalBytes: total, ...extra });

test("the summary counts each status and adds only the speeds that were reported", () => {
  const summary = summarize([
    transfer(10, 100, { speed: 2_000_000, eta: 40 }),
    transfer(10, 100, { speed: 1_000_000, eta: 90 }),
    job("downloading"),
    job("processing"),
    job("queued"),
    job("paused"),
    job("failed"),
    job("completed"),
  ]);
  assert.deepEqual(
    [summary.downloading, summary.processing, summary.queued, summary.paused, summary.failed],
    [3, 1, 1, 1, 1],
  );
  assert.equal(summary.speed, 3_000_000);
});

test("no transfer reported a speed: the speed is unknown, never zero", () => {
  assert.equal(summarize([job("downloading"), job("queued")]).speed, null);
  assert.equal(summarize([]).speed, null);
});

test("time left is the longest transfer's, only when every transfer has one and nothing waits", () => {
  const both = [transfer(1, 2, { eta: 40 }), transfer(1, 2, { eta: 90 })];
  assert.equal(summarize(both).eta, 90);
  assert.equal(summarize([...both, job("downloading")]).eta, null, "one transfer has no time left yet");
  assert.equal(summarize([...both, job("queued")]).eta, null, "a waiting download has no size to go by");
  assert.equal(summarize([...both, job("processing"), job("paused")]).eta, 90, "processing and paused jobs do not hide it");
  assert.equal(summarize([job("processing")]).eta, null, "nothing is transferring");
});

test("a time left that rests on an estimated size is marked approximate", () => {
  assert.equal(summarize([transfer(1, 2, { eta: 40, progressEstimated: true })]).etaApproximate, true);
  assert.equal(summarize([transfer(1, 2, { eta: 40 })]).etaApproximate, false);
  assert.equal(summarize([transfer(1, 2, { eta: 40, progressEstimated: true }), job("queued")]).etaApproximate, false, "no time left is shown at all");
});

test("the taskbar shows nothing when nothing runs", () => {
  assert.deepEqual(taskbarState([job("queued"), job("paused"), job("completed"), job("failed")]), { status: "none" });
  assert.deepEqual(taskbarState([]), { status: "none" });
});

test("the taskbar progress is weighted by bytes across the running downloads", () => {
  // 100 of 1000 bytes and 400 of 1000: half of the bytes, not the mean of 10% and 40%.
  assert.deepEqual(taskbarState([transfer(100, 1000), transfer(900, 1000)]), { status: "normal", progress: 50 });
  assert.deepEqual(taskbarState([transfer(10, 1000), transfer(990, 9000)]), { status: "normal", progress: 10 });
});

test("a download being processed counts as fully received", () => {
  const processing = job("processing", { progressKnown: true, downloadedBytes: 400, totalBytes: 1000 });
  assert.deepEqual(taskbarState([processing, transfer(0, 1000)]), { status: "normal", progress: 50 });
});

test("an unknown size makes the taskbar indeterminate instead of guessing", () => {
  assert.deepEqual(taskbarState([transfer(100, 1000), job("downloading", { downloadedBytes: 5 })]), { status: "indeterminate" });
  assert.deepEqual(taskbarState([job("processing")]), { status: "indeterminate" });
  assert.deepEqual(taskbarState([transfer(1, 0)]), { status: "indeterminate" });
});

test("the window is told again only when what it shows changes", () => {
  assert.equal(sameTaskbarState({ status: "normal", progress: 40 }, { status: "normal", progress: 40 }), true);
  assert.equal(sameTaskbarState({ status: "normal", progress: 40 }, { status: "normal", progress: 41 }), false);
  assert.equal(sameTaskbarState({ status: "none" }, { status: "none" }), true);
  assert.equal(sameTaskbarState({ status: "none" }, { status: "indeterminate" }), false);
});
