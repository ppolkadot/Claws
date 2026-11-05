// Tests for src/queue-events.ts, compiled first into .test-build/ (see package.json
// "test:queue"):
//   npm exec tsc -- --target ES2022 --module NodeNext --moduleResolution NodeNext --outDir .test-build src/queue-events.ts
//   node --test scripts/tests/queue-events.test.mjs
import assert from "node:assert/strict";
import { test } from "node:test";

import { JobTable, StartupEvents } from "../../.test-build/queue-events.js";

const job = (id, createdAt, status = "queued", extra = {}) => ({ id, createdAt, status, progress: 0, ...extra });
const event = (j, revision, schemaVersion = 1) => ({ schemaVersion, jobId: j.id, revision, job: j });
const batch = (events) => ({ schemaVersion: 1, jobs: events });

test("startup replays recovery and removal after bootstrap, then accepts live events", () => {
  const table = new JobTable();
  const startup = new StartupEvents();
  startup.accept(() => table.apply(event(job("a", 1, "completed"), 12)));
  startup.accept(() => table.remove({ schemaVersion: 1, jobId: "b", revision: 13 }));
  assert.equal(table.size, 0);
  table.load([job("a", 1, "processing"), job("b", 2)], () => 10);
  startup.flush();
  assert.equal(table.get("a").status, "completed");
  assert.equal(table.get("b"), undefined);
  startup.accept(() => table.apply(event(job("b", 2), 11)));
  assert.equal(table.get("b"), undefined);
  startup.accept(() => table.apply(event(job("c", 3), 14)));
  assert.equal(table.get("c").status, "queued");
  startup.flush();
  assert.equal(table.size, 2);
});

test("a 500-job batch is added once, in creation order", () => {
  const table = new JobTable();
  const jobs = Array.from({ length: 500 }, (_, i) => job(`j${String(i).padStart(3, "0")}`, 1000 - i));
  const result = table.applyBatch(batch(jobs.map((j, i) => event(j, i + 1))));
  assert.equal(result.added.length, 500);
  assert.equal(table.size, 500);
  const order = table.list().map((j) => j.createdAt);
  assert.deepEqual(order, [...order].sort((a, b) => a - b));
});

test("the enqueue reply after the jobs-added event changes nothing", () => {
  const table = new JobTable();
  const added = batch([event(job("a", 1), 10), event(job("b", 2), 11)]);
  table.applyBatch(added);
  const again = table.applyBatch(added);
  assert.deepEqual(again, { added: [], updated: [], statusChanged: false });
});

test("a delayed Queued never reverts a newer Downloading or Completed", () => {
  const table = new JobTable();
  table.apply(event(job("a", 1, "downloading", { progress: 0.4 }), 20));
  assert.deepEqual(table.apply(event(job("a", 1, "queued"), 10)), { kind: "ignored" });
  assert.equal(table.get("a").status, "downloading");
  table.apply(event(job("a", 1, "completed", { progress: 1 }), 30));
  table.applyBatch(batch([event(job("a", 1, "queued"), 10)])); // the slow enqueue reply
  assert.equal(table.get("a").status, "completed");
});

test("revision ties are treated as duplicates", () => {
  const table = new JobTable();
  table.apply(event(job("a", 1, "downloading", { progress: 0.1 }), 5));
  assert.deepEqual(table.apply(event(job("a", 1, "downloading", { progress: 0.9 }), 5)), { kind: "ignored" });
  assert.equal(table.get("a").progress, 0.1);
});

test("a delayed update after Remove does not resurrect the job", () => {
  const table = new JobTable();
  table.apply(event(job("a", 1), 1));
  assert.equal(table.remove({ schemaVersion: 1, jobId: "a", revision: 5 }), true);
  assert.deepEqual(table.apply(event(job("a", 1, "downloading"), 4)), { kind: "ignored" });
  assert.deepEqual(table.apply(event(job("a", 1, "downloading"), 9)), { kind: "ignored" });
  assert.equal(table.size, 0);
  assert.equal(table.list().length, 0);
});

test("updates report status changes and keep the row position", () => {
  const table = new JobTable();
  table.applyBatch(batch([event(job("a", 1), 1), event(job("b", 2), 2), event(job("c", 3), 3)]));
  const list = table.list();
  assert.deepEqual(table.apply(event(job("b", 2, "queued", { progress: 0 }), 4)), { kind: "updated", statusChanged: false });
  assert.deepEqual(table.apply(event(job("b", 2, "downloading", { progress: 0.5 }), 5)), { kind: "updated", statusChanged: true });
  assert.equal(table.list(), list, "an update must not rebuild the ordered list");
  assert.equal(table.list()[1].progress, 0.5);
});

test("sorted insertion places late arrivals by creation time", () => {
  const table = new JobTable();
  table.apply(event(job("late", 30), 1));
  table.apply(event(job("early", 10), 2));
  table.apply(event(job("middle", 20), 3));
  table.apply(event(job("same-b", 20), 4));
  assert.deepEqual(table.list().map((j) => j.id), ["early", "middle", "same-b", "late"]);
});

test("eight streams of progress keep only each job's newest state", () => {
  const table = new JobTable();
  const ids = Array.from({ length: 8 }, (_, i) => `s${i}`);
  table.applyBatch(batch(ids.map((id, i) => event(job(id, i), i + 1))));
  let revision = 100;
  const deliveries = [];
  for (let round = 1; round <= 50; round++) {
    for (const id of ids) deliveries.push(event(job(id, Number(id.slice(1)), "downloading", { progress: round / 50 }), ++revision));
  }
  // Delivered out of order: shuffle deterministically.
  deliveries.sort((a, b) => ((a.revision * 7919) % 1009) - ((b.revision * 7919) % 1009));
  for (const d of deliveries) table.apply(d);
  for (const id of ids) assert.equal(table.get(id).progress, 1);
});

test("events of an unknown schema are refused, not partly applied", () => {
  const table = new JobTable();
  assert.deepEqual(table.apply(event(job("a", 1), 1, 2)), { kind: "ignored" });
  assert.deepEqual(table.applyBatch({ schemaVersion: 2, jobs: [event(job("b", 1), 1)] }), { added: [], updated: [], statusChanged: false });
  assert.equal(table.remove({ schemaVersion: 0, jobId: "a", revision: 1 }), false);
  assert.equal(table.size, 0);
  assert.equal(table.rejected, 3);
});

test("load replaces the table with a sorted snapshot", () => {
  const table = new JobTable();
  table.apply(event(job("old", 1), 1));
  table.load([job("b", 2), job("a", 1)], () => 7);
  assert.deepEqual(table.list().map((j) => j.id), ["a", "b"]);
  assert.deepEqual(table.apply(event(job("a", 1, "downloading"), 7)), { kind: "ignored" });
  assert.deepEqual(table.apply(event(job("a", 1, "downloading"), 8)), { kind: "updated", statusChanged: true });
});
